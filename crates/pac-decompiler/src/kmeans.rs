//! Tiny K-means in RGB space.  Deterministic seed → same palette
//! every run, no model files, no training, no AI.
//!
//! Optimisations specific to image colour histograms:
//! - Operates on `(R, G, B, weight)` histogram entries, not raw
//!   pixels, so a 768×768 image collapses to ≤ 4096 unique colour
//!   buckets after we 6-bit-truncate per channel.  ~100× speedup.
//! - Seeds with a deterministic k-means++ pass on the histogram so
//!   we get good cluster spread without RNG state.
//! - Stops early when no centroid moves by more than 0.5 in any
//!   channel between iterations.

use pac_format::Color;

use crate::image_io::RgbaImage;

pub struct KMeansResult {
    /// `palette[i]` is the centroid colour of cluster `i`.
    pub palette: Vec<Color>,
    /// `labels[p]` is the cluster index assigned to pixel `p` of the
    /// input image, in row-major order.
    pub labels: Vec<u32>,
}

/// Mean squared per-channel residual between every pixel and its
/// assigned palette colour.  Returned value is "average squared
/// distance per channel", so a threshold of 100 means "on average
/// each R/G/B value is within ~10 of the palette colour".
pub fn mean_squared_residual(img: &RgbaImage, res: &KMeansResult) -> f32 {
    let n = (img.width as usize) * (img.height as usize);
    debug_assert_eq!(img.pixels.len(), n * 4);
    debug_assert_eq!(res.labels.len(), n);

    let mut total: u64 = 0;
    for (i, p) in img.pixels.chunks_exact(4).enumerate() {
        let c = res.palette[res.labels[i] as usize];
        let dr = p[0] as i32 - c.r as i32;
        let dg = p[1] as i32 - c.g as i32;
        let db = p[2] as i32 - c.b as i32;
        total += (dr * dr + dg * dg + db * db) as u64;
    }
    // Divide by 3*n to get per-channel mean square error.
    total as f32 / (3.0 * n as f32)
}

/// Auto-K quantize: start at `k_min` and grow K (doubling, then +4
/// near the top) until either:
///   (a) per-channel residual drops at/below `target_residual`, or
///   (b) we hit `k_max`.
/// Returns the chosen K and the corresponding result.
///
/// Deterministic given the same input + bounds (no RNG, no AI/ML).
pub fn quantize_auto(
    img: &RgbaImage,
    k_min: usize,
    k_max: usize,
    target_residual: f32,
    max_iters: usize,
) -> (usize, KMeansResult) {
    let k_min = k_min.max(2);
    let k_max = k_max.max(k_min);

    let mut k = k_min;
    loop {
        let res = quantize(img, k, max_iters);
        let resid = mean_squared_residual(img, &res);
        if resid <= target_residual || k >= k_max {
            return (k, res);
        }
        // Doubling schedule keeps the total runtime bounded:
        // 6 → 12 → 24 → 32 is at most 4 iterations.
        k = if k < 16 { (k * 2).min(k_max) } else { (k + 8).min(k_max) };
    }
}

/// Quantize `img` to `k` colour clusters.  Returns the palette plus
/// a per-pixel cluster index.
pub fn quantize(img: &RgbaImage, k: usize, max_iters: usize) -> KMeansResult {
    assert!(k >= 1);
    let n = (img.width as usize) * (img.height as usize);
    assert_eq!(img.pixels.len(), n * 4);

    // 1. Build a 6-bit-per-channel weighted histogram.
    //    Bucket = (r >> 2) * 64 * 64 + (g >> 2) * 64 + (b >> 2)
    //    → 64^3 = 262_144 buckets max, but most images touch only a few k.
    let mut hist = std::collections::HashMap::<u32, u32>::with_capacity(8192);
    for p in img.pixels.chunks_exact(4) {
        let bucket = ((p[0] as u32 >> 2) << 12) | ((p[1] as u32 >> 2) << 6) | (p[2] as u32 >> 2);
        *hist.entry(bucket).or_insert(0) += 1;
    }
    let entries: Vec<(f32, f32, f32, u32)> = hist
        .into_iter()
        .map(|(b, w)| {
            let r = (((b >> 12) & 0x3F) << 2) as f32 + 2.0;
            let g = (((b >> 6) & 0x3F) << 2) as f32 + 2.0;
            let bl = ((b & 0x3F) << 2) as f32 + 2.0;
            (r, g, bl, w)
        })
        .collect();

    // 2. Seed with deterministic k-means++ on the histogram.
    let mut centroids = seed_kmeanspp(&entries, k);

    // 3. Lloyd iterations on histogram buckets.
    let mut assignments = vec![0u32; entries.len()];
    let mut sums = vec![(0.0f64, 0.0f64, 0.0f64, 0u64); k];

    for _iter in 0..max_iters {
        let mut moved = 0.0f32;

        for s in sums.iter_mut() {
            *s = (0.0, 0.0, 0.0, 0);
        }
        for (i, e) in entries.iter().enumerate() {
            assignments[i] = nearest_centroid(e.0, e.1, e.2, &centroids);
            let c = &mut sums[assignments[i] as usize];
            let w = e.3 as f64;
            c.0 += e.0 as f64 * w;
            c.1 += e.1 as f64 * w;
            c.2 += e.2 as f64 * w;
            c.3 += w as u64;
        }
        for (i, c) in centroids.iter_mut().enumerate() {
            let s = &sums[i];
            if s.3 > 0 {
                let nr = (s.0 / s.3 as f64) as f32;
                let ng = (s.1 / s.3 as f64) as f32;
                let nb = (s.2 / s.3 as f64) as f32;
                moved = moved.max((nr - c.0).abs()).max((ng - c.1).abs()).max((nb - c.2).abs());
                *c = (nr, ng, nb);
            }
        }
        if moved < 0.5 {
            break;
        }
    }

    // 4. Materialise the palette.
    let palette: Vec<Color> = centroids
        .iter()
        .map(|c| Color::rgb(c.0 as u8, c.1 as u8, c.2 as u8))
        .collect();

    // 5. Build per-pixel labels using the *final* centroids.
    let mut labels = vec![0u32; n];
    for (i, p) in img.pixels.chunks_exact(4).enumerate() {
        labels[i] = nearest_centroid(p[0] as f32, p[1] as f32, p[2] as f32, &centroids);
    }

    KMeansResult { palette, labels }
}

fn nearest_centroid(r: f32, g: f32, b: f32, centroids: &[(f32, f32, f32)]) -> u32 {
    let mut best = 0u32;
    let mut best_d = f32::INFINITY;
    for (i, c) in centroids.iter().enumerate() {
        let dr = r - c.0;
        let dg = g - c.1;
        let db = b - c.2;
        let d = dr * dr + dg * dg + db * db;
        if d < best_d {
            best_d = d;
            best = i as u32;
        }
    }
    best
}

/// Deterministic k-means++ seeding on weighted histogram entries.
/// First seed = highest-weight bucket; subsequent seeds = the bucket
/// with maximum (weight × distance²) from any chosen seed.
fn seed_kmeanspp(entries: &[(f32, f32, f32, u32)], k: usize) -> Vec<(f32, f32, f32)> {
    let mut chosen: Vec<(f32, f32, f32)> = Vec::with_capacity(k);
    if entries.is_empty() {
        return (0..k).map(|_| (0.0, 0.0, 0.0)).collect();
    }
    let first = entries
        .iter()
        .max_by_key(|e| e.3)
        .map(|e| (e.0, e.1, e.2))
        .unwrap();
    chosen.push(first);
    while chosen.len() < k {
        let mut best_score = -1.0f64;
        let mut best_pick = (chosen[0].0, chosen[0].1, chosen[0].2);
        for e in entries {
            let mut min_d2 = f32::INFINITY;
            for c in &chosen {
                let dr = e.0 - c.0;
                let dg = e.1 - c.1;
                let db = e.2 - c.2;
                let d2 = dr * dr + dg * dg + db * db;
                if d2 < min_d2 {
                    min_d2 = d2;
                }
            }
            let score = (min_d2 as f64) * (e.3 as f64);
            if score > best_score {
                best_score = score;
                best_pick = (e.0, e.1, e.2);
            }
        }
        chosen.push(best_pick);
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(r: u8, g: u8, b: u8) -> [u8; 4] { [r, g, b, 255] }

    fn flat(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        let mut pix = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) { pix.extend_from_slice(&c); }
        RgbaImage { width: w, height: h, pixels: pix }
    }

    #[test]
    fn finds_two_clusters_in_two_color_image() {
        // Half-and-half red/blue image; k=2 must place one centroid on red, one on blue.
        let mut pix = Vec::new();
        for y in 0..40 {
            for x in 0..40 {
                let c = if x < 20 { rgba(255, 0, 0) } else { rgba(0, 0, 255) };
                pix.extend_from_slice(&c);
                let _ = y;
            }
        }
        let img = RgbaImage { width: 40, height: 40, pixels: pix };
        let res = quantize(&img, 2, 16);
        assert_eq!(res.palette.len(), 2);

        // Either order is fine; both colours must appear.
        let has_red = res.palette.iter().any(|c| c.r > 200 && c.g < 50 && c.b < 50);
        let has_blue = res.palette.iter().any(|c| c.r < 50 && c.g < 50 && c.b > 200);
        assert!(has_red && has_blue, "palette = {:?}", res.palette);
    }

    #[test]
    fn assigns_every_pixel_to_a_label_within_range() {
        let img = flat(8, 8, rgba(120, 80, 200));
        let res = quantize(&img, 4, 8);
        assert_eq!(res.labels.len(), 64);
        for l in &res.labels {
            assert!((*l as usize) < res.palette.len());
        }
    }

    #[test]
    fn auto_k_stops_early_on_flat_image() {
        // A flat image trivially fits within the histogram bucket
        // quantization — auto-K should stay at k_min.  Residual is
        // small but nonzero due to 6-bit-per-channel bucketing.
        let img = flat(32, 32, rgba(100, 150, 200));
        let (k, res) = quantize_auto(&img, 4, 32, 50.0, 8);
        assert_eq!(k, 4);
        let r = mean_squared_residual(&img, &res);
        assert!(r < 20.0, "residual on flat image = {r}");
    }

    #[test]
    fn auto_k_grows_on_complex_image() {
        // 16-colour checkerboard — k_min=2 is too low, auto-K should grow.
        let w = 16u32; let h = 16u32;
        let palette: [[u8; 4]; 16] = [
            [255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255],
            [255, 0, 255, 255], [0, 255, 255, 255], [255, 255, 255, 255], [128, 0, 0, 255],
            [0, 128, 0, 255], [0, 0, 128, 255], [128, 128, 0, 255], [128, 0, 128, 255],
            [0, 128, 128, 255], [192, 192, 192, 255], [64, 64, 64, 255], [200, 100, 50, 255],
        ];
        let mut pix = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let idx = (((y / 4) * 4) + (x / 4)) as usize % 16;
                pix.extend_from_slice(&palette[idx]);
            }
        }
        let img = RgbaImage { width: w, height: h, pixels: pix };
        let (k, _) = quantize_auto(&img, 2, 32, 50.0, 16);
        assert!(k > 2, "auto-K stayed at k_min on a 16-color image (got {k})");
    }
}
