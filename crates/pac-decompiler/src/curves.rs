//! Schneider's cubic-Bezier curve fitting (Graphics Gems I, 1990).
//!
//! Given an open polyline `[P0, P1, ..., Pn]` and an error tolerance
//! `eps` in pixels, recursively fit cubic Beziers:
//!
//! 1. Pick endpoint tangents `T0` and `T1` from neighbouring chords.
//! 2. Solve a 2×2 least-squares system for `alpha1` and `alpha2`:
//!    `P1 = P0 + alpha1 * T0`, `P2 = P3 + alpha2 * T1`.
//! 3. Measure the max deviation between the fitted curve and the
//!    polyline (sampled at each interior point reparameterised by
//!    Newton-Raphson on the Bezier).
//! 4. If max deviation ≤ `eps`, accept the single Bezier.
//! 5. Otherwise split the polyline at the worst point and recurse.
//!
//! Output is a sequence of `(c1, c2, p)` cubic Bezier control sets
//! whose start point is the polyline's first point and whose
//! subsequent end points chain.
//!
//! For closed loops (PAC's primary use case), we cut the loop at the
//! same "topmost-leftmost" pixel the tracer started from and feed
//! both halves through `fit_curve` separately so the join lands at
//! a known stable anchor.

use pac_format::{PathSeg, Point};

#[derive(Debug, Clone, Copy)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    fn from_xy(x: f32, y: f32) -> Self { Self { x, y } }
    fn from_pt(p: (f32, f32)) -> Self { Self::from_xy(p.0, p.1) }
    fn sub(self, o: Self) -> Self { Self::from_xy(self.x - o.x, self.y - o.y) }
    fn add(self, o: Self) -> Self { Self::from_xy(self.x + o.x, self.y + o.y) }
    fn mul(self, s: f32) -> Self { Self::from_xy(self.x * s, self.y * s) }
    fn dot(self, o: Self) -> f32 { self.x * o.x + self.y * o.y }
    fn length(self) -> f32 { self.dot(self).sqrt() }
    fn normalized(self) -> Self {
        let l = self.length().max(1e-12);
        Self::from_xy(self.x / l, self.y / l)
    }
}

/// Fit a sequence of cubic Beziers to a CLOSED polyline `pts`.
///
/// The returned `PathSeg` list starts with `MoveTo(pts[0])` and
/// continues with `CurveTo` segments (and the occasional `LineTo`
/// for trivial straight segments).  The list is implicitly closed
/// by the renderer.
pub fn fit_closed(pts: &[(f32, f32)], eps: f32) -> Vec<PathSeg> {
    if pts.len() < 3 {
        return pts.iter()
            .map(|&p| PathSeg::LineTo(Point::new(p.0, p.1)))
            .collect();
    }
    // Cut the closed loop at the first vertex; treat the rest as an
    // open polyline that returns to pts[0].
    let mut open: Vec<(f32, f32)> = pts.to_vec();
    open.push(pts[0]);

    let segs = fit_open(&open, eps);

    let mut out = Vec::with_capacity(segs.len() + 1);
    out.push(PathSeg::MoveTo(Point::new(pts[0].0, pts[0].1)));
    out.extend(segs);
    out
}

/// Fit a sequence of cubic Beziers to an OPEN polyline.  Returned
/// segments are all `CurveTo` (or `LineTo` for sufficiently straight
/// runs).  The first point is implicit (caller must `MoveTo` it).
pub fn fit_open(pts: &[(f32, f32)], eps: f32) -> Vec<PathSeg> {
    if pts.len() < 2 {
        return Vec::new();
    }
    if pts.len() == 2 {
        return vec![PathSeg::LineTo(Point::new(pts[1].0, pts[1].1))];
    }
    // Endpoint tangents — chord between immediate neighbours.
    let t0 = Vec2::from_pt(pts[1]).sub(Vec2::from_pt(pts[0])).normalized();
    let n = pts.len();
    let t1 = Vec2::from_pt(pts[n - 2]).sub(Vec2::from_pt(pts[n - 1])).normalized();
    let mut out = Vec::new();
    fit_recursive(pts, 0, n - 1, t0, t1, eps, &mut out);
    out
}

fn fit_recursive(
    pts: &[(f32, f32)],
    first: usize,
    last: usize,
    t1: Vec2,
    t2: Vec2,
    eps: f32,
    out: &mut Vec<PathSeg>,
) {
    let n_pts = last - first + 1;

    // Two-point case: emit a straight line.
    if n_pts == 2 {
        out.push(PathSeg::LineTo(Point::new(pts[last].0, pts[last].1)));
        return;
    }

    // Reparameterise the chord-length parameter for each point.
    let u = chord_length_parameterize(&pts[first..=last]);

    // Solve for the Bezier.
    let (bez, _is_line) = generate_bezier(&pts[first..=last], &u, t1, t2);

    // Measure error and find the worst point.
    let (max_err, split_idx) = compute_max_error(&pts[first..=last], &u, &bez);

    if max_err < eps {
        out.push(PathSeg::CurveTo {
            c1: Point::new(bez[1].x, bez[1].y),
            c2: Point::new(bez[2].x, bez[2].y),
            p: Point::new(bez[3].x, bez[3].y),
        });
        return;
    }

    // Try one Newton-Raphson re-parameterise pass — if the new
    // parameterisation lowers the error below `eps * 4`, refit with
    // those parameters before splitting (per the Gem).
    if max_err < eps * eps * 16.0 {
        let mut u2 = u.clone();
        reparameterize(&pts[first..=last], &mut u2, &bez);
        let (bez2, _) = generate_bezier(&pts[first..=last], &u2, t1, t2);
        let (err2, split2) = compute_max_error(&pts[first..=last], &u2, &bez2);
        if err2 < eps {
            out.push(PathSeg::CurveTo {
                c1: Point::new(bez2[1].x, bez2[1].y),
                c2: Point::new(bez2[2].x, bez2[2].y),
                p: Point::new(bez2[3].x, bez2[3].y),
            });
            return;
        }
        // Fallthrough: split at the (possibly improved) worst point.
        let split_at = split2.max(1).min(n_pts - 2);
        let split_global = first + split_at;
        let tc = center_tangent(pts, split_global);
        fit_recursive(pts, first, split_global, t1, tc, eps, out);
        fit_recursive(pts, split_global, last, tc.mul(-1.0), t2, eps, out);
        return;
    }

    // Split at the worst point and recurse on both halves.
    let split_at = split_idx.max(1).min(n_pts - 2);
    let split_global = first + split_at;
    let tc = center_tangent(pts, split_global);
    fit_recursive(pts, first, split_global, t1, tc, eps, out);
    fit_recursive(pts, split_global, last, tc.mul(-1.0), t2, eps, out);
}

/// Center tangent at a split point, per Schneider's Gem.
/// Points from `pts[i+1]` toward `pts[i-1]` — i.e. "back along the
/// polyline" so the LEFT-half end-tangent and (negated) RIGHT-half
/// start-tangent are C1-continuous at the join.
fn center_tangent(pts: &[(f32, f32)], i: usize) -> Vec2 {
    Vec2::from_pt(pts[i - 1]).sub(Vec2::from_pt(pts[i + 1])).normalized()
}

/// Compute the chord-length parameterization of an open polyline,
/// normalized to [0, 1].
fn chord_length_parameterize(pts: &[(f32, f32)]) -> Vec<f32> {
    let n = pts.len();
    let mut u = vec![0.0f32; n];
    for i in 1..n {
        let d = Vec2::from_pt(pts[i]).sub(Vec2::from_pt(pts[i - 1])).length();
        u[i] = u[i - 1] + d;
    }
    let total = u[n - 1].max(1e-12);
    for v in u.iter_mut() { *v /= total; }
    u
}

/// Schneider's least-squares solver for the two control points of
/// a cubic Bezier whose endpoints and endpoint tangents are fixed.
fn generate_bezier(
    pts: &[(f32, f32)],
    u: &[f32],
    t1: Vec2,
    t2: Vec2,
) -> ([Vec2; 4], bool) {
    let n = pts.len();
    let p0 = Vec2::from_pt(pts[0]);
    let p3 = Vec2::from_pt(pts[n - 1]);

    // Build A[i][0] = T1 * B1(u_i), A[i][1] = T2 * B2(u_i)
    // (T1/T2 scaled by the cubic basis functions for the middle control points).
    let mut c00 = 0.0f32;
    let mut c01 = 0.0f32;
    let mut c11 = 0.0f32;
    let mut x0 = 0.0f32;
    let mut x1 = 0.0f32;

    for i in 0..n {
        let ui = u[i];
        let b1 = 3.0 * (1.0 - ui).powi(2) * ui;
        let b2 = 3.0 * (1.0 - ui) * ui.powi(2);

        let a0 = t1.mul(b1);
        let a1 = t2.mul(b2);

        c00 += a0.dot(a0);
        c01 += a0.dot(a1);
        c11 += a1.dot(a1);

        let tmp = Vec2::from_pt(pts[i])
            .sub(bezier_point_simple(p0, p0, p3, p3, ui)); // q(u) - B(u; P0,P0,P3,P3)
        x0 += a0.dot(tmp);
        x1 += a1.dot(tmp);
    }

    // Solve 2x2 system: [c00 c01; c01 c11] * [alpha1; alpha2] = [x0; x1]
    let det = c00 * c11 - c01 * c01;
    let (alpha1, alpha2) = if det.abs() < 1e-9 {
        // Degenerate — fall back to straight line approximation (alphas based on chord length / 3).
        let chord = p3.sub(p0).length();
        (chord / 3.0, chord / 3.0)
    } else {
        ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det)
    };

    // Clamp alphas hard.  Two failure modes if unclamped:
    //   - alpha ≤ 0  → cusp / inversion → wild looping curve
    //   - alpha very large → control point flies far off the chord
    //     → renderer draws a giant spike out of the polygon
    // The Wu-Barsky-style fix is to fall back to `chord / 3` whenever
    // alpha leaves the "physically reasonable" band of (0, chord].
    let chord = p3.sub(p0).length().max(1e-6);
    let min_alpha = chord * 1e-6;
    let max_alpha = chord;
    let alpha1 = if alpha1 < min_alpha || alpha1 > max_alpha {
        chord / 3.0
    } else {
        alpha1
    };
    let alpha2 = if alpha2 < min_alpha || alpha2 > max_alpha {
        chord / 3.0
    } else {
        alpha2
    };

    let p1 = p0.add(t1.mul(alpha1));
    let p2 = p3.add(t2.mul(alpha2));
    let was_clamped = alpha1 <= min_alpha || alpha2 <= min_alpha;
    ([p0, p1, p2, p3], was_clamped)
}

/// Evaluate a cubic Bezier at parameter `u` from its four control points.
fn bezier_point(b: &[Vec2; 4], u: f32) -> Vec2 {
    bezier_point_simple(b[0], b[1], b[2], b[3], u)
}

fn bezier_point_simple(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, u: f32) -> Vec2 {
    let v = 1.0 - u;
    let b0 = v * v * v;
    let b1 = 3.0 * v * v * u;
    let b2 = 3.0 * v * u * u;
    let b3 = u * u * u;
    Vec2::from_xy(
        b0 * p0.x + b1 * p1.x + b2 * p2.x + b3 * p3.x,
        b0 * p0.y + b1 * p1.y + b2 * p2.y + b3 * p3.y,
    )
}

/// Maximum squared distance from any sample point to the fitted curve.
/// Returns (max_error, index_of_worst_point).
fn compute_max_error(pts: &[(f32, f32)], u: &[f32], bez: &[Vec2; 4]) -> (f32, usize) {
    let mut max_err = 0.0f32;
    let mut max_i = pts.len() / 2;
    for i in 0..pts.len() {
        let on_curve = bezier_point(bez, u[i]);
        let q = Vec2::from_pt(pts[i]);
        let d = q.sub(on_curve);
        let err = d.dot(d);
        if err > max_err {
            max_err = err;
            max_i = i;
        }
    }
    (max_err.sqrt(), max_i)
}

/// One Newton-Raphson pass to improve each sample's t-parameter.
fn reparameterize(pts: &[(f32, f32)], u: &mut [f32], bez: &[Vec2; 4]) {
    for i in 0..pts.len() {
        let q = Vec2::from_pt(pts[i]);
        let p = bezier_point(bez, u[i]);

        // First derivative of the Bezier at u.
        let dp = bezier_d1(bez, u[i]);
        let ddp = bezier_d2(bez, u[i]);

        let numerator = (p.sub(q)).dot(dp);
        let denominator = dp.dot(dp) + (p.sub(q)).dot(ddp);
        if denominator.abs() > 1e-9 {
            u[i] = (u[i] - numerator / denominator).clamp(0.0, 1.0);
        }
    }
}

fn bezier_d1(b: &[Vec2; 4], u: f32) -> Vec2 {
    let v = 1.0 - u;
    // d/du B = 3(1-u)^2 (P1-P0) + 6(1-u)u (P2-P1) + 3 u^2 (P3-P2)
    let q0 = b[1].sub(b[0]).mul(3.0);
    let q1 = b[2].sub(b[1]).mul(3.0);
    let q2 = b[3].sub(b[2]).mul(3.0);
    let c0 = v * v;
    let c1 = 2.0 * v * u;
    let c2 = u * u;
    Vec2::from_xy(
        c0 * q0.x + c1 * q1.x + c2 * q2.x,
        c0 * q0.y + c1 * q1.y + c2 * q2.y,
    )
}

fn bezier_d2(b: &[Vec2; 4], u: f32) -> Vec2 {
    // 2nd derivative: 6(1-u)(P2 - 2P1 + P0) + 6u(P3 - 2P2 + P1)
    let r0 = b[2].sub(b[1].mul(2.0)).add(b[0]).mul(6.0);
    let r1 = b[3].sub(b[2].mul(2.0)).add(b[1]).mul(6.0);
    Vec2::from_xy(
        (1.0 - u) * r0.x + u * r1.x,
        (1.0 - u) * r0.y + u * r1.y,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
        ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
    }

    #[test]
    fn fits_a_circle_with_handful_of_beziers() {
        // 64 samples around a unit circle.
        let pts: Vec<(f32, f32)> = (0..64)
            .map(|i| {
                let t = (i as f32) * std::f32::consts::TAU / 64.0;
                (50.0 + 40.0 * t.cos(), 50.0 + 40.0 * t.sin())
            })
            .collect();
        let segs = fit_closed(&pts, 0.5);

        // Should be MoveTo + a small number of CurveTo (Bezier circle approx is 4).
        let curve_count = segs.iter().filter(|s| matches!(s, PathSeg::CurveTo { .. })).count();
        assert!(curve_count >= 2 && curve_count <= 12, "got {curve_count} curves; segs={segs:?}");
    }

    #[test]
    fn fits_a_straight_run_as_lines() {
        // Long horizontal run.
        let pts: Vec<(f32, f32)> = (0..30).map(|i| (i as f32 * 2.0, 5.0)).collect();
        let segs = fit_open(&pts, 0.25);
        // Should reduce to a single LineTo (or at most 2 LineTos / very gentle curves).
        assert!(segs.len() <= 2, "expected <=2 segs for straight run, got {}: {segs:?}", segs.len());
    }

    #[test]
    fn closed_path_starts_with_moveto() {
        let pts = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let segs = fit_closed(&pts, 0.5);
        assert!(matches!(segs[0], PathSeg::MoveTo(_)));
    }

    #[test]
    fn fitted_curve_stays_within_tolerance() {
        // Half-circle: 30 samples from 0 to pi.
        let pts: Vec<(f32, f32)> = (0..30)
            .map(|i| {
                let t = (i as f32) * std::f32::consts::PI / 29.0;
                (50.0 + 40.0 * t.cos(), 50.0 + 40.0 * t.sin())
            })
            .collect();
        let tolerance = 0.5;
        let segs = fit_open(&pts, tolerance);

        // Flatten the fitted curve densely, then check that every input
        // sample lies close to *some* point on it. Anything less than
        // this does not test the fit at all.
        //
        // Before v0.9 this test's body was `let _ = segs;` followed by
        // `dist(p, p)`, which is identically zero — it could never fail.
        // It was the only assertion-free test in the workspace.
        let mut curve: Vec<(f32, f32)> = vec![pts[0]];
        let mut cursor = pts[0];
        for seg in &segs {
            match seg {
                PathSeg::MoveTo(p) => {
                    cursor = (p.x, p.y);
                    curve.push(cursor);
                }
                PathSeg::LineTo(p) => {
                    let end = (p.x, p.y);
                    for i in 1..=32 {
                        let u = i as f32 / 32.0;
                        curve.push((
                            cursor.0 + (end.0 - cursor.0) * u,
                            cursor.1 + (end.1 - cursor.1) * u,
                        ));
                    }
                    cursor = end;
                }
                PathSeg::CurveTo { c1, c2, p } => {
                    let (a, b, c, d) = (cursor, (c1.x, c1.y), (c2.x, c2.y), (p.x, p.y));
                    for i in 1..=32 {
                        let u = i as f32 / 32.0;
                        let v = 1.0 - u;
                        let x = v * v * v * a.0
                            + 3.0 * v * v * u * b.0
                            + 3.0 * v * u * u * c.0
                            + u * u * u * d.0;
                        let y = v * v * v * a.1
                            + 3.0 * v * v * u * b.1
                            + 3.0 * v * u * u * c.1
                            + u * u * u * d.1;
                        curve.push((x, y));
                    }
                    cursor = d;
                }
            }
        }

        // Distance to the polyline's *segments*, not to its vertices.
        // Measuring vertex-to-vertex instead reports roughly half the
        // sampling step as error, which on a coarse flattening looks
        // exactly like a fitter that misses its tolerance.
        let point_to_segment = |p: (f32, f32), a: (f32, f32), b: (f32, f32)| -> f32 {
            let (vx, vy) = (b.0 - a.0, b.1 - a.1);
            let len_sq = vx * vx + vy * vy;
            if len_sq <= f32::EPSILON {
                return dist(p, a);
            }
            let t = (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / len_sq).clamp(0.0, 1.0);
            dist(p, (a.0 + vx * t, a.1 + vy * t))
        };

        let mut worst = 0.0f32;
        for &p in &pts {
            let nearest = curve
                .windows(2)
                .map(|w| point_to_segment(p, w[0], w[1]))
                .fold(f32::INFINITY, f32::min);
            worst = worst.max(nearest);
        }
        assert!(
            worst <= tolerance,
            "fitted curve strays {worst:.3}px from the samples (tolerance {tolerance})"
        );
    }
}
