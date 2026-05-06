//! Per-frame multi-blob detection.
//!
//! All deterministic, all measurement: sample → mask → connected
//! components → for each component, centroid + bbox + interior-only
//! mean colour + Hu image-moment invariants. No models, no training.
//!
//! v0.3 generalises v0.2's "find the single largest blob" to "find
//! every blob above the minimum-area threshold," so the cross-frame
//! tracker in `track.rs` can stitch tracks for an arbitrary number
//! of objects.

use vac_format::Color;

use crate::frames::VideoFrame;

#[derive(Debug, Clone)]
pub struct Detection {
    pub frame_index: usize,
    /// Time offset of this frame in milliseconds (cumulative).
    pub t_ms: u32,
    /// Estimated background colour for this frame.
    pub bg: Color,
    /// All foreground blobs above the minimum area, sorted descending
    /// by area (largest first — useful when the tracker wants a stable
    /// processing order).
    pub blobs: Vec<Blob>,
}

#[derive(Debug, Clone, Copy)]
pub struct Blob {
    pub cx: f32,
    pub cy: f32,
    /// Mean of bbox half-width and half-height. For circles ≈ true radius.
    pub radius: f32,
    /// Mean colour over interior pixels (4-neighbours all share label).
    /// Falls back to whole-blob mean for very thin shapes.
    pub color: Color,
    /// Pixel count of the blob.
    pub area: u32,
    /// Hu image-moment invariants (h₁..h₇). Translation, scale, rotation
    /// invariant — useful for shape-based matching across frames.
    pub hu: [f32; 7],
}

/// Squared Euclidean colour distance threshold for the
/// foreground/background mask. ~24 per channel ≈ visually distinct
/// without flagging dithering noise.
const FG_THRESHOLD_SQ: u32 = 24 * 24 * 3;

/// A blob with fewer than this many pixels is treated as detection noise
/// and discarded (smaller than a 3x3 cluster).
const MIN_BLOB_AREA: u32 = 9;

/// Run detection across all frames.
pub fn detect_all(frames: &[VideoFrame]) -> Vec<Detection> {
    let mut out = Vec::with_capacity(frames.len());
    let mut t_ms: u32 = 0;
    for (i, f) in frames.iter().enumerate() {
        let bg = estimate_background(f);
        let mut blobs = detect_blobs(f, bg);
        blobs.sort_by(|a, b| b.area.cmp(&a.area));
        out.push(Detection {
            frame_index: i,
            t_ms,
            bg,
            blobs,
        });
        t_ms = t_ms.saturating_add(f.delay_ms);
    }
    out
}

/// Sample a small block at each corner and return the median RGB.
fn estimate_background(f: &VideoFrame) -> Color {
    let w = f.width as i32;
    let h = f.height as i32;
    let block = 5i32;
    let corners = [
        (0, 0),
        (w - block, 0),
        (0, h - block),
        (w - block, h - block),
    ];
    let mut samples: Vec<(u8, u8, u8)> = Vec::with_capacity(4);
    for (cx, cy) in corners {
        if let Some(c) = sample_block_avg(f, cx.max(0), cy.max(0), block, block) {
            samples.push(c);
        }
    }
    if samples.is_empty() {
        return Color::rgb(0, 0, 0);
    }
    let mut rs: Vec<u8> = samples.iter().map(|c| c.0).collect();
    let mut gs: Vec<u8> = samples.iter().map(|c| c.1).collect();
    let mut bs: Vec<u8> = samples.iter().map(|c| c.2).collect();
    rs.sort();
    gs.sort();
    bs.sort();
    Color::rgb(rs[rs.len() / 2], gs[gs.len() / 2], bs[bs.len() / 2])
}

fn sample_block_avg(f: &VideoFrame, x0: i32, y0: i32, w: i32, h: i32) -> Option<(u8, u8, u8)> {
    let fw = f.width as i32;
    let fh = f.height as i32;
    let x1 = (x0 + w).min(fw);
    let y1 = (y0 + h).min(fh);
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    let mut sr: u32 = 0;
    let mut sg: u32 = 0;
    let mut sb: u32 = 0;
    let mut n: u32 = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * fw + x) * 4) as usize;
            sr += f.pixels[i] as u32;
            sg += f.pixels[i + 1] as u32;
            sb += f.pixels[i + 2] as u32;
            n += 1;
        }
    }
    if n == 0 {
        return None;
    }
    Some(((sr / n) as u8, (sg / n) as u8, (sb / n) as u8))
}

// --------------------------------------------------------------------
// Multi-blob detection
// --------------------------------------------------------------------

fn detect_blobs(f: &VideoFrame, bg: Color) -> Vec<Blob> {
    let w = f.width as usize;
    let h = f.height as usize;

    // Foreground mask.
    let mut mask = vec![false; w * h];
    for i in 0..(w * h) {
        let r = f.pixels[i * 4];
        let g = f.pixels[i * 4 + 1];
        let b = f.pixels[i * 4 + 2];
        let dr = r as i32 - bg.r as i32;
        let dg = g as i32 - bg.g as i32;
        let db = b as i32 - bg.b as i32;
        let dist_sq = (dr * dr + dg * dg + db * db) as u32;
        mask[i] = dist_sq > FG_THRESHOLD_SQ;
    }

    let labels = label_components_4(&mask, w, h);

    // Per-label area count.
    let mut counts = std::collections::HashMap::<u32, u32>::new();
    for &l in &labels {
        if l != 0 {
            *counts.entry(l).or_insert(0) += 1;
        }
    }

    // Keep labels above min area; build a stable ordering for the per-blob
    // accumulators below.
    let kept_labels: Vec<u32> = counts
        .iter()
        .filter_map(|(&l, &c)| if c >= MIN_BLOB_AREA { Some(l) } else { None })
        .collect();
    if kept_labels.is_empty() {
        return Vec::new();
    }
    let label_to_idx: std::collections::HashMap<u32, usize> = kept_labels
        .iter()
        .enumerate()
        .map(|(i, &l)| (l, i))
        .collect();

    let n = kept_labels.len();
    let mut acc: Vec<BlobAcc> = (0..n).map(|_| BlobAcc::new()).collect();

    // Single pass over the frame: accumulate per-blob stats simultaneously.
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let l = labels[idx];
            if l == 0 {
                continue;
            }
            let Some(&bi) = label_to_idx.get(&l) else {
                continue; // sub-min-area noise blob
            };
            let a = &mut acc[bi];

            a.area += 1;
            let pi = idx * 4;
            let r = f.pixels[pi] as u64;
            let g = f.pixels[pi + 1] as u64;
            let b = f.pixels[pi + 2] as u64;
            a.all_r += r;
            a.all_g += g;
            a.all_b += b;

            // Raw moments for centroid + Hu invariants.
            let xf = x as f64;
            let yf = y as f64;
            a.m00 += 1.0;
            a.m10 += xf;
            a.m01 += yf;

            // Bounding box.
            if (x as u32) < a.min_x {
                a.min_x = x as u32;
            }
            if (x as u32) > a.max_x {
                a.max_x = x as u32;
            }
            if (y as u32) < a.min_y {
                a.min_y = y as u32;
            }
            if (y as u32) > a.max_y {
                a.max_y = y as u32;
            }

            // Interior-pixel colour (4-neighbours share the label).
            let interior = x > 0
                && y > 0
                && x + 1 < w
                && y + 1 < h
                && labels[idx - 1] == l
                && labels[idx + 1] == l
                && labels[idx - w] == l
                && labels[idx + w] == l;
            if interior {
                a.int_r += r;
                a.int_g += g;
                a.int_b += b;
                a.int_count += 1;
            }
        }
    }

    // Second pass: central moments (require centroids from pass 1).
    // Bound the moments by the bbox so we only revisit the relevant region.
    for (bi, a) in acc.iter_mut().enumerate() {
        if a.area == 0 {
            continue;
        }
        let cx = a.m10 / a.m00;
        let cy = a.m01 / a.m00;
        let l = kept_labels[bi];

        let (mut mu20, mut mu02, mut mu11) = (0.0f64, 0.0f64, 0.0f64);
        let (mut mu30, mut mu03, mut mu21, mut mu12) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);

        let x0 = a.min_x as usize;
        let x1 = a.max_x as usize;
        let y0 = a.min_y as usize;
        let y1 = a.max_y as usize;
        for y in y0..=y1 {
            for x in x0..=x1 {
                if labels[y * w + x] != l {
                    continue;
                }
                let dx = x as f64 - cx;
                let dy = y as f64 - cy;
                let dx2 = dx * dx;
                let dy2 = dy * dy;
                mu20 += dx2;
                mu02 += dy2;
                mu11 += dx * dy;
                mu30 += dx2 * dx;
                mu03 += dy2 * dy;
                mu21 += dx2 * dy;
                mu12 += dx * dy2;
            }
        }
        a.cx = cx as f32;
        a.cy = cy as f32;
        a.hu = hu_moments(a.m00, mu20, mu02, mu11, mu30, mu03, mu21, mu12);
    }

    acc.into_iter()
        .filter(|a| a.area > 0)
        .map(|a| {
            let bbox_w = (a.max_x - a.min_x + 1) as f32;
            let bbox_h = (a.max_y - a.min_y + 1) as f32;
            let radius = (bbox_w + bbox_h) * 0.25;
            let color = if a.int_count > 0 {
                Color::rgb(
                    (a.int_r / a.int_count) as u8,
                    (a.int_g / a.int_count) as u8,
                    (a.int_b / a.int_count) as u8,
                )
            } else {
                let n = a.area as u64;
                Color::rgb(
                    (a.all_r / n) as u8,
                    (a.all_g / n) as u8,
                    (a.all_b / n) as u8,
                )
            };
            Blob {
                cx: a.cx,
                cy: a.cy,
                radius,
                color,
                area: a.area,
                hu: a.hu,
            }
        })
        .collect()
}

/// Compute the 7 Hu moment invariants from a blob's central moments.
/// Given `m00` (= area) and central moments μ₂₀ μ₀₂ μ₁₁ μ₃₀ μ₀₃ μ₂₁ μ₁₂.
fn hu_moments(
    m00: f64,
    mu20: f64,
    mu02: f64,
    mu11: f64,
    mu30: f64,
    mu03: f64,
    mu21: f64,
    mu12: f64,
) -> [f32; 7] {
    if m00 <= 0.0 {
        return [0.0; 7];
    }
    // Normalised central moments η_pq = μ_pq / m00^((p+q)/2 + 1).
    let n2 = m00 * m00; // m00^2 (used for p+q=2)
    let n_pq2 = n2;
    // For p+q = 3 → m00^((3/2)+1) = m00^2.5
    let n_pq3 = m00.powf(2.5);

    let n20 = mu20 / n_pq2;
    let n02 = mu02 / n_pq2;
    let n11 = mu11 / n_pq2;
    let n30 = mu30 / n_pq3;
    let n03 = mu03 / n_pq3;
    let n21 = mu21 / n_pq3;
    let n12 = mu12 / n_pq3;

    let h1 = n20 + n02;
    let h2 = (n20 - n02).powi(2) + 4.0 * n11.powi(2);
    let h3 = (n30 - 3.0 * n12).powi(2) + (3.0 * n21 - n03).powi(2);
    let h4 = (n30 + n12).powi(2) + (n21 + n03).powi(2);
    let h5 = (n30 - 3.0 * n12) * (n30 + n12) * ((n30 + n12).powi(2) - 3.0 * (n21 + n03).powi(2))
        + (3.0 * n21 - n03)
            * (n21 + n03)
            * (3.0 * (n30 + n12).powi(2) - (n21 + n03).powi(2));
    let h6 = (n20 - n02) * ((n30 + n12).powi(2) - (n21 + n03).powi(2))
        + 4.0 * n11 * (n30 + n12) * (n21 + n03);
    let h7 = (3.0 * n21 - n03) * (n30 + n12) * ((n30 + n12).powi(2) - 3.0 * (n21 + n03).powi(2))
        - (n30 - 3.0 * n12)
            * (n21 + n03)
            * (3.0 * (n30 + n12).powi(2) - (n21 + n03).powi(2));

    [
        h1 as f32, h2 as f32, h3 as f32, h4 as f32, h5 as f32, h6 as f32, h7 as f32,
    ]
}

struct BlobAcc {
    area: u32,
    cx: f32,
    cy: f32,
    hu: [f32; 7],
    m00: f64,
    m10: f64,
    m01: f64,
    min_x: u32,
    max_x: u32,
    min_y: u32,
    max_y: u32,
    all_r: u64,
    all_g: u64,
    all_b: u64,
    int_r: u64,
    int_g: u64,
    int_b: u64,
    int_count: u64,
}

impl BlobAcc {
    fn new() -> Self {
        Self {
            area: 0,
            cx: 0.0,
            cy: 0.0,
            hu: [0.0; 7],
            m00: 0.0,
            m10: 0.0,
            m01: 0.0,
            min_x: u32::MAX,
            max_x: 0,
            min_y: u32::MAX,
            max_y: 0,
            all_r: 0,
            all_g: 0,
            all_b: 0,
            int_r: 0,
            int_g: 0,
            int_b: 0,
            int_count: 0,
        }
    }
}

// --------------------------------------------------------------------
// Connected components (4-neighbourhood, two-pass union-find)
// --------------------------------------------------------------------

fn label_components_4(mask: &[bool], w: usize, h: usize) -> Vec<u32> {
    let mut labels = vec![0u32; w * h];
    let mut uf = UnionFind::new();
    let mut next: u32 = 1;

    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !mask[i] {
                continue;
            }
            let left = if x > 0 && mask[i - 1] { labels[i - 1] } else { 0 };
            let up = if y > 0 && mask[i - w] { labels[i - w] } else { 0 };
            match (left, up) {
                (0, 0) => {
                    labels[i] = next;
                    uf.make_set(next);
                    next += 1;
                }
                (a, 0) => labels[i] = a,
                (0, b) => labels[i] = b,
                (a, b) => {
                    let m = a.min(b);
                    labels[i] = m;
                    if a != b {
                        uf.union(a, b);
                    }
                }
            }
        }
    }
    for l in labels.iter_mut() {
        if *l != 0 {
            *l = uf.find(*l);
        }
    }
    labels
}

struct UnionFind {
    parent: Vec<u32>,
}

impl UnionFind {
    fn new() -> Self {
        UnionFind { parent: vec![0] }
    }
    fn make_set(&mut self, x: u32) {
        while self.parent.len() <= x as usize {
            self.parent.push(self.parent.len() as u32);
        }
        self.parent[x as usize] = x;
    }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let p = self.parent[x as usize];
            let gp = self.parent[p as usize];
            self.parent[x as usize] = gp;
            x = gp;
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            if ra < rb {
                self.parent[rb as usize] = ra;
            } else {
                self.parent[ra as usize] = rb;
            }
        }
    }
}
