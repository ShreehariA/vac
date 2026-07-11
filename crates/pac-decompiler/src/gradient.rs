//! Linear-gradient detection per region.
//!
//! Algorithm (deterministic, no AI/ML):
//!
//! 1. Collect every source-image pixel `(x, y, r, g, b)` belonging to
//!    the region (via `region_labels[i] == r.label`).
//! 2. For each of R, G, B independently, solve the 3-parameter least
//!    squares `c = a*x + b*y + c0`.
//! 3. Convert the three planes into a single (dr/dx, dg/dx, db/dx) and
//!    (dr/dy, dg/dy, db/dy) — the chrominance gradient vectors.
//! 4. Pick the dominant gradient direction `d = (a, b)` whose
//!    magnitude is the larger of |dC/dx| and |dC/dy| (Euclidean norm
//!    across channels).
//! 5. Project every region pixel onto `d` and take the min/max
//!    projection — those are the gradient endpoints `p0` and `p1`.
//!    Evaluate the colour at each endpoint by plugging back into the
//!    fitted planes.
//! 6. Compute the residual sum-of-squares against (a) the gradient
//!    model and (b) the solid-mean model.  Only emit a gradient if
//!    `gradient_residual < (1 - improvement_ratio) * solid_residual`
//!    AND the gradient magnitude exceeds `min_delta` per channel
//!    (otherwise the gradient is too flat to be visible / is noise).
//!
//! Output: an `Option<LinearGradient { c0, c1, p0, p1 }>`, ready to
//! drop into `Fill::LinearGradient`.

use crate::image_io::RgbaImage;
use crate::regions::LabelledRegion;
use pac_format::{Color, Point};

#[derive(Debug, Clone, Copy)]
pub struct DetectedGradient {
    pub c0: Color,
    pub c1: Color,
    pub p0: Point,
    pub p1: Point,
}

/// Try to fit a linear gradient to the pixels of `region`.  Returns
/// `Some` if the gradient model explains substantially more variance
/// than a flat fill.
pub fn detect_linear_gradient(
    img: &RgbaImage,
    region_labels: &[u32],
    region: &LabelledRegion,
    improvement_ratio: f32,
    min_delta: f32,
) -> Option<DetectedGradient> {
    let w = img.width as i32;
    let target = region.label;

    // Single-pass accumulation of sums for the 3-parameter LS plane
    // fit per channel.  Normal equations:
    //   [Sxx Sxy Sx ] [a ]   [Sxr ]
    //   [Sxy Syy Sy ] [b ] = [Syr ]
    //   [Sx  Sy  n  ] [c0]   [Sr  ]
    let mut n: u64 = 0;
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    let (mut sxx, mut syy, mut sxy) = (0.0f64, 0.0f64, 0.0f64);
    let (mut sr, mut sg, mut sb) = (0.0f64, 0.0f64, 0.0f64);
    let (mut sxr, mut sxg, mut sxb) = (0.0f64, 0.0f64, 0.0f64);
    let (mut syr, mut syg, mut syb) = (0.0f64, 0.0f64, 0.0f64);

    // Bounded scan: only iterate the region's bounding box.
    for y in (region.min_y as i32)..=(region.max_y as i32) {
        for x in (region.min_x as i32)..=(region.max_x as i32) {
            let idx = (y as usize) * (w as usize) + x as usize;
            if region_labels[idx] != target {
                continue;
            }
            let p = &img.pixels[idx * 4..idx * 4 + 3];
            let fx = x as f64;
            let fy = y as f64;
            let fr = p[0] as f64;
            let fg = p[1] as f64;
            let fb = p[2] as f64;

            n += 1;
            sx += fx; sy += fy;
            sxx += fx * fx; syy += fy * fy; sxy += fx * fy;
            sr += fr; sg += fg; sb += fb;
            sxr += fx * fr; sxg += fx * fg; sxb += fx * fb;
            syr += fy * fr; syg += fy * fg; syb += fy * fb;
        }
    }
    if n < 16 {
        return None;
    }

    // Solve the 3×3 normal equations once and reuse the inverse for
    // each channel.
    let nf = n as f64;
    let mat = [
        [sxx, sxy, sx],
        [sxy, syy, sy],
        [sx,  sy,  nf],
    ];
    let inv = match invert_3x3(mat) {
        Some(m) => m,
        None => return None,
    };

    let solve = |rhs: [f64; 3]| -> [f64; 3] {
        [
            inv[0][0] * rhs[0] + inv[0][1] * rhs[1] + inv[0][2] * rhs[2],
            inv[1][0] * rhs[0] + inv[1][1] * rhs[1] + inv[1][2] * rhs[2],
            inv[2][0] * rhs[0] + inv[2][1] * rhs[1] + inv[2][2] * rhs[2],
        ]
    };

    let pr = solve([sxr, syr, sr]); // a_r, b_r, c0_r
    let pg = solve([sxg, syg, sg]);
    let pb_ = solve([sxb, syb, sb]);

    // Channel means for solid-fit baseline.
    let mr = sr / nf;
    let mg = sg / nf;
    let mb = sb / nf;

    // Average plane direction.  We weight by per-channel gradient
    // magnitude so dim channels don't pull a bright region's direction.
    let ax = pr[0]; let ay = pr[1];
    let bx = pg[0]; let by = pg[1];
    let cx = pb_[0]; let cy = pb_[1];
    // Pick the channel with the largest 2D gradient magnitude as the
    // direction; using "average" can cancel out when channels point
    // in opposite directions.
    let mag = |x: f64, y: f64| (x*x + y*y).sqrt();
    let mr_m = mag(ax, ay);
    let mg_m = mag(bx, by);
    let mb_m = mag(cx, cy);
    let (gx, gy) = if mr_m >= mg_m && mr_m >= mb_m {
        (ax, ay)
    } else if mg_m >= mb_m {
        (bx, by)
    } else {
        (cx, cy)
    };
    let glen = (gx*gx + gy*gy).sqrt();
    if glen < 1e-9 {
        return None;
    }
    let ux = gx / glen;
    let uy = gy / glen;

    // Project every pixel onto the direction (ux, uy) to find min/max.
    // Track the actual pixel positions at the extremes so the gradient
    // endpoints land *inside* the region's spatial bounding box, not
    // out at infinity along a line through origin (which would force
    // the plane fit to extrapolate and clip to saturated colours —
    // observed as magenta artefacts in v0.2.0).
    let mut t_min = f64::INFINITY;
    let mut t_max = f64::NEG_INFINITY;
    let mut pmin = (0i32, 0i32);
    let mut pmax = (0i32, 0i32);
    for y in (region.min_y as i32)..=(region.max_y as i32) {
        for x in (region.min_x as i32)..=(region.max_x as i32) {
            let idx = (y as usize) * (w as usize) + x as usize;
            if region_labels[idx] != target {
                continue;
            }
            let t = (x as f64) * ux + (y as f64) * uy;
            if t < t_min { t_min = t; pmin = (x, y); }
            if t > t_max { t_max = t; pmax = (x, y); }
        }
    }
    if !t_min.is_finite() || !t_max.is_finite() || (t_max - t_min) < 1.0 {
        return None;
    }

    // Endpoint colours by evaluating the fitted planes at the actual
    // extreme-pixel coordinates.  No extrapolation, no out-of-gamut
    // saturation.
    let x0 = pmin.0 as f64;
    let y0 = pmin.1 as f64;
    let x1 = pmax.0 as f64;
    let y1 = pmax.1 as f64;
    let eval = |p: [f64; 3], x: f64, y: f64| -> f64 { p[0] * x + p[1] * y + p[2] };
    let c0 = Color::rgb(
        clamp_u8(eval(pr, x0, y0)),
        clamp_u8(eval(pg, x0, y0)),
        clamp_u8(eval(pb_, x0, y0)),
    );
    let c1 = Color::rgb(
        clamp_u8(eval(pr, x1, y1)),
        clamp_u8(eval(pg, x1, y1)),
        clamp_u8(eval(pb_, x1, y1)),
    );

    // Variance reduction check.  Compute SSE for both models.
    let mut gradient_sse: f64 = 0.0;
    let mut solid_sse: f64 = 0.0;
    for y in (region.min_y as i32)..=(region.max_y as i32) {
        for x in (region.min_x as i32)..=(region.max_x as i32) {
            let idx = (y as usize) * (w as usize) + x as usize;
            if region_labels[idx] != target {
                continue;
            }
            let p = &img.pixels[idx * 4..idx * 4 + 3];
            let fx = x as f64; let fy = y as f64;
            let gr = eval(pr, fx, fy);
            let gg = eval(pg, fx, fy);
            let gb = eval(pb_, fx, fy);
            let dr = p[0] as f64 - gr;
            let dg = p[1] as f64 - gg;
            let db = p[2] as f64 - gb;
            gradient_sse += dr*dr + dg*dg + db*db;
            let sdr = p[0] as f64 - mr;
            let sdg = p[1] as f64 - mg;
            let sdb = p[2] as f64 - mb;
            solid_sse += sdr*sdr + sdg*sdg + sdb*sdb;
        }
    }

    // Per-channel colour delta along the gradient.  Reject very flat
    // gradients (visually they'd look like a solid fill).
    let delta = ((c1.r as f32 - c0.r as f32).abs()
        + (c1.g as f32 - c0.g as f32).abs()
        + (c1.b as f32 - c0.b as f32).abs()) / 3.0;
    if delta < min_delta {
        return None;
    }

    let improvement = (solid_sse - gradient_sse).max(0.0) / solid_sse.max(1e-9);
    if improvement < improvement_ratio as f64 {
        return None;
    }

    Some(DetectedGradient {
        c0,
        c1,
        p0: Point::new(x0 as f32, y0 as f32),
        p1: Point::new(x1 as f32, y1 as f32),
    })
}

fn clamp_u8(v: f64) -> u8 { v.round().clamp(0.0, 255.0) as u8 }

fn invert_3x3(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv_det = 1.0 / det;
    let cof = |a: f64, b: f64, c: f64, d: f64| (a * d - b * c) * inv_det;
    Some([
        [
             cof(m[1][1], m[1][2], m[2][1], m[2][2]),
            -cof(m[0][1], m[0][2], m[2][1], m[2][2]),
             cof(m[0][1], m[0][2], m[1][1], m[1][2]),
        ],
        [
            -cof(m[1][0], m[1][2], m[2][0], m[2][2]),
             cof(m[0][0], m[0][2], m[2][0], m[2][2]),
            -cof(m[0][0], m[0][2], m[1][0], m[1][2]),
        ],
        [
             cof(m[1][0], m[1][1], m[2][0], m[2][1]),
            -cof(m[0][0], m[0][1], m[2][0], m[2][1]),
             cof(m[0][0], m[0][1], m[1][0], m[1][1]),
        ],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_io::RgbaImage;
    use crate::regions::label_regions_per_cluster_full;

    fn img_with_horizontal_gradient(w: u32, h: u32) -> RgbaImage {
        // R goes 0..255 from x=0 to x=w-1, G=128 const, B=255-R.
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for _y in 0..h {
            for x in 0..w {
                let r = ((x * 255) / (w - 1).max(1)) as u8;
                let b = 255 - r;
                px.extend_from_slice(&[r, 128, b, 255]);
            }
        }
        RgbaImage { width: w, height: h, pixels: px }
    }

    #[test]
    fn detects_strong_horizontal_gradient() {
        let img = img_with_horizontal_gradient(64, 16);
        // One region covering the entire image.
        let labels = vec![1u32; (img.width * img.height) as usize];
        let res = label_regions_per_cluster_full(&labels, img.width, img.height, 1);
        let r = res.regions.iter().find(|r| r.cluster == 1).unwrap();
        let g = detect_linear_gradient(&img, &res.region_labels, r, 0.5, 5.0)
            .expect("gradient should be detected");
        // Direction should be roughly horizontal.
        let dx = (g.p1.x - g.p0.x).abs();
        let dy = (g.p1.y - g.p0.y).abs();
        assert!(dx > 10.0 && dy < dx, "expected horizontal gradient, got dx={dx} dy={dy}");
        // Colours should span red→blue (high-R/low-B → low-R/high-B or reverse).
        assert!((g.c0.r as i32 - g.c1.r as i32).abs() > 100,
            "gradient r-span too small: {} → {}", g.c0.r, g.c1.r);
    }

    #[test]
    fn rejects_flat_region() {
        let mut px = Vec::with_capacity(32 * 32 * 4);
        for _ in 0..32*32 { px.extend_from_slice(&[100, 150, 200, 255]); }
        let img = RgbaImage { width: 32, height: 32, pixels: px };
        let labels = vec![1u32; 32 * 32];
        let res = label_regions_per_cluster_full(&labels, 32, 32, 1);
        let r = res.regions.iter().find(|r| r.cluster == 1).unwrap();
        assert!(detect_linear_gradient(&img, &res.region_labels, r, 0.5, 5.0).is_none());
    }
}
