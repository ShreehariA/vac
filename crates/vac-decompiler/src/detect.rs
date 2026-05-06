//! Per-frame ball detection.
//!
//! All deterministic, all measurement: sample → mask → connected
//! components → centroid + bbox + mean colour. No models, no training.

use vac_format::Color;

use crate::frames::VideoFrame;

#[derive(Debug, Clone, Copy)]
pub struct Detection {
    pub frame_index: usize,
    /// Time offset of this frame in milliseconds (cumulative).
    pub t_ms: u32,
    /// Estimated background colour for this frame.
    pub bg: Color,
    /// `None` means no foreground blob was found in this frame.
    pub blob: Option<Blob>,
}

#[derive(Debug, Clone, Copy)]
pub struct Blob {
    pub cx: f32,
    pub cy: f32,
    /// Avg of bbox half-width and half-height — good for circles.
    pub radius: f32,
    pub color: Color,
}

/// Squared Euclidean colour distance threshold for the
/// foreground/background mask. ~24 per channel ≈ visually distinct
/// without flagging dithering noise.
const FG_THRESHOLD_SQ: u32 = 24 * 24 * 3;

/// Run detection across all frames.
pub fn detect_all(frames: &[VideoFrame]) -> Vec<Detection> {
    let mut out = Vec::with_capacity(frames.len());
    let mut t_ms: u32 = 0;
    for (i, f) in frames.iter().enumerate() {
        let bg = estimate_background(f);
        let blob = detect_blob(f, bg);
        out.push(Detection {
            frame_index: i,
            t_ms,
            bg,
            blob,
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
    // Median per channel — robust to one outlier corner.
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

fn detect_blob(f: &VideoFrame, bg: Color) -> Option<Blob> {
    let w = f.width as usize;
    let h = f.height as usize;

    // Build foreground mask.
    let mut mask = vec![false; w * h];
    for i in 0..(w * h) {
        let r = f.pixels[i * 4];
        let g = f.pixels[i * 4 + 1];
        let b = f.pixels[i * 4 + 2];
        let dr = (r as i32 - bg.r as i32) as i32;
        let dg = (g as i32 - bg.g as i32) as i32;
        let db = (b as i32 - bg.b as i32) as i32;
        let dist_sq = (dr * dr + dg * dg + db * db) as u32;
        mask[i] = dist_sq > FG_THRESHOLD_SQ;
    }

    // Connected-component labeling (4-connectivity, two-pass union-find).
    let labels = label_components_4(&mask, w, h);

    // Find largest non-zero label.
    let mut counts = std::collections::HashMap::<u32, u32>::new();
    for &l in &labels {
        if l != 0 {
            *counts.entry(l).or_insert(0) += 1;
        }
    }
    let (best_label, best_count) = counts.into_iter().max_by_key(|(_, c)| *c)?;
    // Tiny noise blobs (< ~9 px) are not balls.
    if best_count < 9 {
        return None;
    }

    // Single pass over the blob: centroid + bbox use *all* pixels, but
    // colour is sampled only from "interior" pixels (those whose four
    // 4-neighbours are also in the blob). Edge pixels along the contour
    // are anti-aliased blends of fill+background and would otherwise pull
    // the mean colour towards the background, producing the visible drift
    // (`#e94560` → `#e4445e`) we saw in the v0.1 round-trip.
    let (mut sx, mut sy) = (0u64, 0u64);
    let (mut all_r, mut all_g, mut all_b) = (0u64, 0u64, 0u64);
    let (mut int_r, mut int_g, mut int_b) = (0u64, 0u64, 0u64);
    let mut int_count: u64 = 0;
    let mut min_x = u32::MAX;
    let mut max_x = 0u32;
    let mut min_y = u32::MAX;
    let mut max_y = 0u32;

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if labels[idx] != best_label {
                continue;
            }

            sx += x as u64;
            sy += y as u64;

            let pi = idx * 4;
            let r = f.pixels[pi] as u64;
            let g = f.pixels[pi + 1] as u64;
            let b = f.pixels[pi + 2] as u64;
            all_r += r;
            all_g += g;
            all_b += b;

            // Interior = strict 4-neighbours all share our label.
            // Frame-border pixels are never interior.
            let interior = x > 0
                && y > 0
                && x + 1 < w
                && y + 1 < h
                && labels[idx - 1] == best_label
                && labels[idx + 1] == best_label
                && labels[idx - w] == best_label
                && labels[idx + w] == best_label;
            if interior {
                int_r += r;
                int_g += g;
                int_b += b;
                int_count += 1;
            }

            if (x as u32) < min_x {
                min_x = x as u32;
            }
            if (x as u32) > max_x {
                max_x = x as u32;
            }
            if (y as u32) < min_y {
                min_y = y as u32;
            }
            if (y as u32) > max_y {
                max_y = y as u32;
            }
        }
    }

    let n = best_count as u64;
    let cx = (sx as f32) / (n as f32);
    let cy = (sy as f32) / (n as f32);
    let bbox_w = (max_x - min_x + 1) as f32;
    let bbox_h = (max_y - min_y + 1) as f32;
    let radius = (bbox_w + bbox_h) * 0.25; // (w/2 + h/2)/2

    let color = if int_count > 0 {
        Color::rgb(
            (int_r / int_count) as u8,
            (int_g / int_count) as u8,
            (int_b / int_count) as u8,
        )
    } else {
        // Tiny / 1-pixel-wide blobs fall back to the unfiltered mean.
        Color::rgb((all_r / n) as u8, (all_g / n) as u8, (all_b / n) as u8)
    };

    Some(Blob {
        cx,
        cy,
        radius,
        color,
    })
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
        UnionFind { parent: vec![0] } // index 0 reserved for "no label"
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
            self.parent[x as usize] = gp; // path compression
            x = gp;
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            // Attach the larger label under the smaller; keeps roots low.
            if ra < rb {
                self.parent[rb as usize] = ra;
            } else {
                self.parent[ra as usize] = rb;
            }
        }
    }
}
