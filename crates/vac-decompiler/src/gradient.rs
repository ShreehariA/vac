//! Gradient fills (v1.2).
//!
//! Through v1.1 every fill was one flat colour, so a lit sphere, a
//! vignette or a sky came back as its mean. This module fits two two-stop
//! models to a shape's *interior* pixels (anti-aliased edges excluded) and
//! keeps one only when the measurement says it is worth it:
//!
//! - **linear**: a least-squares colour plane per channel gives each
//!   channel's gradient; their dominant common direction (the principal
//!   eigenvector of `Σ gₖgₖᵀ`) is the axis, and each channel is refitted
//!   along it, `c = α + β·t`, with the stops at the extreme projections;
//! - **radial**: colour against distance from a centre, `c = α + β·r`, the
//!   centre found by a coarse-to-fine grid search over the shape's extent
//!   (a lit ball's highlight is rarely at its centre).
//!
//! The winner must remove at least `MIN_GAIN` of the flat fill's squared
//! error, leave at most `MAX_FIT_RMS` per channel, and have two visibly
//! different stops; otherwise the shape stays flat. Deterministic: fixed grids, no randomness, no learned model.

use vac_format::{Color, Gradient, Point};

/// Fraction of the flat fill's squared error a gradient must remove...
const MIN_GAIN: f64 = 0.6;
/// ...and the RMS error per channel it may leave. Both together mean the
/// gradient *describes* the region. Measured on the corpus: a lit ball
/// fits with gain 0.94 and 13 levels left; the coarse silhouette over the
/// Mandelbrot set "fits" with gain 0.41 and 55 levels left, and accepting
/// that smear cost the clip 0.05 ink IoU for its residual gain.
const MAX_FIT_RMS: f64 = 25.0;
/// RGB distance the two stops must differ by to be worth writing.
const MIN_STOP_DELTA: f32 = 24.0;
/// Fewer interior pixels than this cannot support a gradient fit.
const MIN_PIXELS: usize = 48;

/// A fitted gradient and how much better than flat it is.
#[derive(Debug, Clone, Copy)]
pub struct Fit {
    pub gradient: Gradient,
    pub flat_sse: f64,
    pub fit_sse: f64,
}

/// Fit a gradient to `(x, y, rgb)` pixels, or `None` if flat is as good.
pub fn fit(px: &[(f32, f32, [f32; 3])]) -> Option<Fit> {
    if px.len() < MIN_PIXELS {
        return None;
    }
    let n = px.len() as f64;
    let mut mean = [0.0f64; 3];
    let (mut mx, mut my) = (0.0f64, 0.0f64);
    for (x, y, c) in px {
        mx += *x as f64;
        my += *y as f64;
        for k in 0..3 {
            mean[k] += c[k] as f64;
        }
    }
    mx /= n;
    my /= n;
    for m in &mut mean {
        *m /= n;
    }
    let flat_sse: f64 = px
        .iter()
        .map(|(_, _, c)| (0..3).map(|k| (c[k] as f64 - mean[k]).powi(2)).sum::<f64>())
        .sum();
    if flat_sse <= 0.0 {
        return None;
    }

    let mut best: Option<(f64, Gradient)> = None;
    let mut consider = |sse: f64, g: Gradient| {
        if best.as_ref().is_none_or(|(b, _)| sse < *b) {
            best = Some((sse, g));
        }
    };
    if let Some((sse, g)) = fit_linear(px, mx, my) {
        consider(sse, g);
    }
    if let Some((sse, g)) = fit_radial(px) {
        consider(sse, g);
    }
    let (fit_sse, gradient) = best?;
    let (c0, c1) = match gradient {
        Gradient::Linear { c0, c1, .. } | Gradient::Radial { c0, c1, .. } => (c0, c1),
    };
    let delta = (((c0.r as f32 - c1.r as f32).powi(2)
        + (c0.g as f32 - c1.g as f32).powi(2)
        + (c0.b as f32 - c1.b as f32).powi(2)) as f32)
        .sqrt();
    let fit_rms = (fit_sse / (3.0 * n)).sqrt();
    (fit_sse <= (1.0 - MIN_GAIN) * flat_sse && fit_rms <= MAX_FIT_RMS && delta >= MIN_STOP_DELTA).then_some(Fit {
        gradient,
        flat_sse,
        fit_sse,
    })
}

fn to_color(c: [f64; 3]) -> Color {
    let q = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    Color::rgb(q(c[0]), q(c[1]), q(c[2]))
}

/// Per-channel 1-D least squares `c = α + β·u` over a per-pixel
/// coordinate `u`; returns `(α, β, sse)` with the model clamped to the
/// stop range, which is how the renderer pads it.
fn fit_along(px: &[(f32, f32, [f32; 3])], u: &[f64]) -> Option<([f64; 3], [f64; 3], f64)> {
    let n = u.len() as f64;
    let mu = u.iter().sum::<f64>() / n;
    let suu: f64 = u.iter().map(|v| (v - mu).powi(2)).sum();
    if suu < 1e-6 {
        return None;
    }
    let (umin, umax) = u.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
    let mut alpha = [0.0; 3];
    let mut beta = [0.0; 3];
    for k in 0..3 {
        let mc = px.iter().map(|p| p.2[k] as f64).sum::<f64>() / n;
        let suc: f64 = px.iter().zip(u).map(|(p, v)| (v - mu) * (p.2[k] as f64 - mc)).sum();
        beta[k] = suc / suu;
        alpha[k] = mc - beta[k] * mu;
    }
    let sse = px
        .iter()
        .zip(u)
        .map(|(p, &v)| {
            let v = v.clamp(umin, umax);
            (0..3)
                .map(|k| {
                    let m = (alpha[k] + beta[k] * v).clamp(0.0, 255.0);
                    (p.2[k] as f64 - m).powi(2)
                })
                .sum::<f64>()
        })
        .sum();
    Some((alpha, beta, sse))
}

fn fit_linear(px: &[(f32, f32, [f32; 3])], mx: f64, my: f64) -> Option<(f64, Gradient)> {
    // Per-channel plane gradients from the centred normal equations.
    let (mut sxx, mut syy, mut sxy) = (0.0f64, 0.0f64, 0.0f64);
    let mut sxc = [0.0f64; 3];
    let mut syc = [0.0f64; 3];
    let n = px.len() as f64;
    let mut mc = [0.0f64; 3];
    for p in px {
        for k in 0..3 {
            mc[k] += p.2[k] as f64;
        }
    }
    for m in &mut mc {
        *m /= n;
    }
    for p in px {
        let (dx, dy) = (p.0 as f64 - mx, p.1 as f64 - my);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
        for k in 0..3 {
            let dc = p.2[k] as f64 - mc[k];
            sxc[k] += dx * dc;
            syc[k] += dy * dc;
        }
    }
    let det = sxx * syy - sxy * sxy;
    if det.abs() < 1e-9 {
        return None;
    }
    let mut m = [[0.0f64; 2]; 2];
    for k in 0..3 {
        let a = (syy * sxc[k] - sxy * syc[k]) / det;
        let b = (sxx * syc[k] - sxy * sxc[k]) / det;
        m[0][0] += a * a;
        m[0][1] += a * b;
        m[1][1] += b * b;
    }
    // Principal eigenvector of the symmetric 2×2 matrix.
    let theta = 0.5 * (2.0 * m[0][1]).atan2(m[0][0] - m[1][1]);
    let (dx, dy) = (theta.cos(), theta.sin());
    let t: Vec<f64> = px.iter().map(|p| (p.0 as f64 - mx) * dx + (p.1 as f64 - my) * dy).collect();
    let (alpha, beta, sse) = fit_along(px, &t)?;
    let (tmin, tmax) = t.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
    if tmax - tmin < 2.0 {
        return None;
    }
    let at = |tv: f64| [alpha[0] + beta[0] * tv, alpha[1] + beta[1] * tv, alpha[2] + beta[2] * tv];
    let pt = |tv: f64| Point::new((mx + tv * dx).round() as f32, (my + tv * dy).round() as f32);
    Some((
        sse,
        Gradient::Linear {
            c0: to_color(at(tmin)),
            p0: pt(tmin),
            c1: to_color(at(tmax)),
            p1: pt(tmax),
        },
    ))
}

fn fit_radial(px: &[(f32, f32, [f32; 3])]) -> Option<(f64, Gradient)> {
    let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in px {
        x0 = x0.min(p.0);
        x1 = x1.max(p.0);
        y0 = y0.min(p.1);
        y1 = y1.max(p.1);
    }
    let (w, h) = ((x1 - x0).max(1.0) as f64, (y1 - y0).max(1.0) as f64);
    let score = |cx: f64, cy: f64| -> Option<(f64, [f64; 3], [f64; 3], f64)> {
        let r: Vec<f64> = px
            .iter()
            .map(|p| ((p.0 as f64 - cx).powi(2) + (p.1 as f64 - cy).powi(2)).sqrt())
            .collect();
        let rmax = r.iter().cloned().fold(0.0, f64::max);
        let (alpha, beta, sse) = fit_along(px, &r)?;
        Some((sse, alpha, beta, rmax))
    };
    // Coarse 9×9 grid over the extent plus a quarter margin, then two
    // 5×5 refinements around the best centre at shrinking spacing.
    let mut best: Option<(f64, f64, f64)> = None; // (sse, cx, cy)
    let (gx0, gy0) = (x0 as f64 - 0.25 * w, y0 as f64 - 0.25 * h);
    let (sx, sy) = (1.5 * w / 8.0, 1.5 * h / 8.0);
    for i in 0..9 {
        for j in 0..9 {
            let (cx, cy) = (gx0 + i as f64 * sx, gy0 + j as f64 * sy);
            if let Some((sse, ..)) = score(cx, cy) {
                if best.is_none_or(|b| sse < b.0) {
                    best = Some((sse, cx, cy));
                }
            }
        }
    }
    let (mut step_x, mut step_y) = (sx / 2.0, sy / 2.0);
    for _ in 0..2 {
        let (_, bx, by) = best?;
        for i in -2..=2 {
            for j in -2..=2 {
                let (cx, cy) = (bx + i as f64 * step_x, by + j as f64 * step_y);
                if let Some((sse, ..)) = score(cx, cy) {
                    if best.is_none_or(|b| sse < b.0) {
                        best = Some((sse, cx, cy));
                    }
                }
            }
        }
        step_x /= 2.0;
        step_y /= 2.0;
    }
    let (_, cx, cy) = best?;
    let (sse, alpha, beta, rmax) = score(cx, cy)?;
    if rmax < 2.0 {
        return None;
    }
    // The fit only ever saw distances in [rmin, rmax]; the stops sit at
    // the centre and at the farthest pixel, and the renderer pads beyond.
    let at = |rv: f64| [alpha[0] + beta[0] * rv, alpha[1] + beta[1] * rv, alpha[2] + beta[2] * rv];
    Some((
        sse,
        Gradient::Radial {
            c0: to_color(at(0.0)),
            center: Point::new(cx.round() as f32, cy.round() as f32),
            c1: to_color(at(rmax)),
            radius: rmax.round() as f32,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(cx: f32, cy: f32, r: f32, colour: impl Fn(f32, f32) -> [f32; 3]) -> Vec<(f32, f32, [f32; 3])> {
        let mut out = Vec::new();
        for y in 0..100 {
            for x in 0..100 {
                let (xf, yf) = (x as f32, y as f32);
                if ((xf - cx).powi(2) + (yf - cy).powi(2)).sqrt() <= r {
                    out.push((xf, yf, colour(xf, yf)));
                }
            }
        }
        out
    }

    #[test]
    fn a_flat_fill_stays_flat() {
        let px = disc(50.0, 50.0, 30.0, |_, _| [200.0, 60.0, 60.0]);
        assert!(fit(&px).is_none());
    }

    #[test]
    fn a_horizontal_ramp_is_a_linear_gradient() {
        let px = disc(50.0, 50.0, 30.0, |x, _| [x * 2.0, 40.0, 255.0 - x * 2.0]);
        let f = fit(&px).expect("ramp should fit");
        match f.gradient {
            Gradient::Linear { c0, p0, c1, p1 } => {
                assert!((p0.y - p1.y).abs() <= 1.0, "axis should be horizontal: {p0:?} {p1:?}");
                let (lo, hi) = if p0.x < p1.x { (c0, c1) } else { (c1, c0) };
                assert!((lo.r as i32 - 40).abs() <= 3 && (hi.r as i32 - 160).abs() <= 3, "{lo:?} {hi:?}");
            }
            g => panic!("expected linear, got {g:?}"),
        }
        assert!(f.fit_sse < 0.01 * f.flat_sse);
    }

    #[test]
    fn an_off_centre_highlight_is_a_radial_gradient() {
        // A lit ball: bright at (40, 40), darkening with distance from it.
        let px = disc(50.0, 50.0, 30.0, |x, y| {
            let d = ((x - 40.0).powi(2) + (y - 40.0).powi(2)).sqrt();
            let v = (240.0 - 4.0 * d).max(0.0);
            [v, v, v]
        });
        let f = fit(&px).expect("lit ball should fit");
        match f.gradient {
            Gradient::Radial { c0, center, .. } => {
                assert!((center.x - 40.0).abs() <= 2.0 && (center.y - 40.0).abs() <= 2.0, "{center:?}");
                assert!((c0.r as i32 - 240).abs() <= 6, "{c0:?}");
            }
            g => panic!("expected radial, got {g:?}"),
        }
    }
}
