//! Across-frame tracking, keyframe extraction, easing classification,
//! and AST emission.
//!
//! The simplification step is **temporal-aware**: we run a 1D
//! Ramer-Douglas-Peucker on `x(t)` and another on `y(t)` and union
//! the kept indices.  This catches collinear back-and-forth motion
//! that a 2D RDP on `(x, y)` would miss (every interior point of a
//! left → right → left ball lies exactly on the line through start
//! and end → 2D perpendicular distance is zero → apex is dropped).
//!
//! Easing classification is template-based: between each consecutive
//! pair of kept keyframes we compare the measured trajectory against
//! `linear`, `ease-in`, `ease-out`, `ease-in-out`, sum the squared
//! pixel error per template, and pick the lowest.  Linear is preferred
//! on near-ties (within 10 %) to avoid noise-driven false positives.

use vac_format::{
    Animation, Canvas, Color, Document, Easing, Keyframe, Property, Scene, Shape, Statement,
    Transform, Value,
};

use crate::detect::{Blob, Detection};
use crate::frames::{estimate_fps, VideoFrame};

/// 1D RDP tolerance, in pixels.  Below this, deviations from the
/// straight-line interpolation between segment endpoints are considered
/// noise.
const RDP_TOLERANCE_PX: f32 = 2.0;

/// Easing classifier only switches off `Linear` when an alternative
/// fits at least this much better (lower SSE).  Stops noisy trajectories
/// from accidentally being labelled `ease-in` etc.
const EASING_IMPROVEMENT_RATIO: f64 = 0.90;

pub fn build_document(frames: &[VideoFrame], detections: &[Detection]) -> Document {
    let canvas = Canvas {
        width: frames[0].width,
        height: frames[0].height,
        fps: estimate_fps(frames),
    };

    let bg_color = median_color(detections.iter().map(|d| d.bg));
    let blobs: Vec<(u32, Blob)> = detections
        .iter()
        .filter_map(|d| d.blob.map(|b| (d.t_ms, b)))
        .collect();

    let total_ms = detections
        .last()
        .map(|d| d.t_ms.saturating_add(frames.last().map(|f| f.delay_ms).unwrap_or(0)))
        .unwrap_or(0);

    let mut statements = Vec::new();

    statements.push(Statement::Let {
        name: "bg".into(),
        shape: Shape::Rect {
            x: 0.0,
            y: 0.0,
            w: canvas.width as f32,
            h: canvas.height as f32,
        },
    });
    statements.push(Statement::Assign {
        target: "bg".into(),
        property: Property::Fill,
        value: Value::Color(bg_color),
    });

    if !blobs.is_empty() {
        let radii: Vec<f32> = blobs.iter().map(|(_, b)| b.radius).collect();
        let radius = round_to_int(median_f32(&radii));
        let ball_color = median_color(blobs.iter().map(|(_, b)| b.color));
        let (first_t, first_b) = blobs[0];

        statements.push(Statement::Let {
            name: "ball".into(),
            shape: Shape::Ellipse {
                cx: round_to_int(first_b.cx),
                cy: round_to_int(first_b.cy),
                rx: radius,
                ry: radius,
            },
        });
        statements.push(Statement::Assign {
            target: "ball".into(),
            property: Property::Fill,
            value: Value::Color(ball_color),
        });
        statements.push(Statement::Assign {
            target: "ball".into(),
            property: Property::Stroke,
            value: Value::None,
        });

        // Trajectory simplification (temporal-aware) → keyframes.
        let mut keep = simplify_trajectory(&blobs, RDP_TOLERANCE_PX);

        if !keep.contains(&0) {
            keep.insert(0, 0);
        }
        let last_idx = blobs.len() - 1;
        if !keep.contains(&last_idx) {
            keep.push(last_idx);
        }
        keep.sort();
        keep.dedup();

        if keep.len() >= 2 && trajectory_has_motion(&blobs) {
            let easing = classify_easing(&blobs, &keep);

            let keyframes: Vec<Keyframe> = keep
                .iter()
                .map(|&i| {
                    let (t, b) = blobs[i];
                    let t_norm = if i == 0 { 0 } else { t.saturating_sub(first_t) };
                    Keyframe {
                        time_ms: t_norm,
                        transforms: vec![Transform::Position(
                            round_to_int(b.cx),
                            round_to_int(b.cy),
                        )],
                    }
                })
                .collect();

            statements.push(Statement::Animate(Animation {
                target: "ball".into(),
                keyframes,
                easing,
            }));
        }
    }

    Document {
        canvas,
        scenes: vec![Scene {
            name: "main".into(),
            duration_ms: total_ms,
            statements,
        }],
    }
}

fn trajectory_has_motion(blobs: &[(u32, Blob)]) -> bool {
    if blobs.len() < 2 {
        return false;
    }
    let (cx0, cy0) = (blobs[0].1.cx, blobs[0].1.cy);
    blobs
        .iter()
        .any(|(_, b)| ((b.cx - cx0).powi(2) + (b.cy - cy0).powi(2)).sqrt() > 1.5)
}

// --------------------------------------------------------------------
// Temporal-aware trajectory simplification
// --------------------------------------------------------------------

/// Run two independent 1D RDPs — one on `x(t)`, one on `y(t)` —
/// and return the sorted union of the indices each chose to keep.
///
/// This is the core of the v0.2 fix: 2D RDP on `(x, y)` collapses
/// the apex of any back-and-forth-along-a-line trajectory because
/// every interior point lies *on* the line through the endpoints.
/// Splitting the work along the time axis recovers it cleanly and
/// also generalises to arbitrary 2D motion.
pub fn simplify_trajectory(blobs: &[(u32, Blob)], tol: f32) -> Vec<usize> {
    let n = blobs.len();
    if n < 3 {
        return (0..n).collect();
    }
    let times: Vec<f32> = blobs.iter().map(|(t, _)| *t as f32).collect();
    let xs: Vec<f32> = blobs.iter().map(|(_, b)| b.cx).collect();
    let ys: Vec<f32> = blobs.iter().map(|(_, b)| b.cy).collect();

    let kx = rdp_1d(&times, &xs, tol);
    let ky = rdp_1d(&times, &ys, tol);

    let mut combined: Vec<usize> = kx.into_iter().chain(ky.into_iter()).collect();
    combined.sort();
    combined.dedup();
    combined
}

/// 1D RDP on a univariate signal `v(t)`: deviation is `|v_actual - v_linear(t)|`.
/// Independent of the time-axis scale, unlike 2D perpendicular distance.
fn rdp_1d(times: &[f32], values: &[f32], tol: f32) -> Vec<usize> {
    debug_assert_eq!(times.len(), values.len());
    let n = values.len();
    if n < 3 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    rdp_1d_recurse(times, values, 0, n - 1, tol, &mut keep);
    (0..n).filter(|&i| keep[i]).collect()
}

fn rdp_1d_recurse(
    times: &[f32],
    values: &[f32],
    start: usize,
    end: usize,
    tol: f32,
    keep: &mut [bool],
) {
    if end <= start + 1 {
        return;
    }
    let t0 = times[start];
    let v0 = values[start];
    let t1 = times[end];
    let v1 = values[end];
    let dt = t1 - t0;

    let mut max_dev = 0.0_f32;
    let mut max_idx = start;
    for i in (start + 1)..end {
        let dev = if dt.abs() < f32::EPSILON {
            (values[i] - v0).abs()
        } else {
            let expected = v0 + (times[i] - t0) / dt * (v1 - v0);
            (values[i] - expected).abs()
        };
        if dev > max_dev {
            max_dev = dev;
            max_idx = i;
        }
    }

    if max_dev > tol {
        keep[max_idx] = true;
        rdp_1d_recurse(times, values, start, max_idx, tol, keep);
        rdp_1d_recurse(times, values, max_idx, end, tol, keep);
    }
}

// --------------------------------------------------------------------
// Easing classification
// --------------------------------------------------------------------

/// For each consecutive pair of kept keyframes, project all measured
/// in-between samples onto the segment, sum squared pixel error against
/// each canonical easing template, and return the global winner.
///
/// Linear is the default and is only displaced by an alternative whose
/// SSE is ≤ `EASING_IMPROVEMENT_RATIO × SSE(Linear)`.  This keeps
/// noise-dominated or dense-keyframe trajectories honest.
fn classify_easing(blobs: &[(u32, Blob)], keep: &[usize]) -> Easing {
    const CANDIDATES: [Easing; 4] = [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
    ];
    let mut errors = [0.0_f64; 4];
    let mut samples: usize = 0;

    for w in keep.windows(2) {
        let i0 = w[0];
        let i1 = w[1];
        if i1 <= i0 + 1 {
            continue;
        }
        let (t0, b0) = blobs[i0];
        let (t1, b1) = blobs[i1];
        let dt = t1.saturating_sub(t0) as f32;
        if dt < 1.0 {
            continue;
        }
        let dx = b1.cx - b0.cx;
        let dy = b1.cy - b0.cy;
        if dx.abs() < 1.0 && dy.abs() < 1.0 {
            continue;
        }

        for j in (i0 + 1)..i1 {
            let (t, b) = blobs[j];
            let t_norm = ((t.saturating_sub(t0)) as f32) / dt;
            for (k, e) in CANDIDATES.iter().enumerate() {
                let p = e.apply(t_norm);
                let ex = b0.cx + p * dx;
                let ey = b0.cy + p * dy;
                let err = ((b.cx - ex).powi(2) + (b.cy - ey).powi(2)) as f64;
                errors[k] += err;
            }
            samples += 1;
        }
    }

    if samples == 0 {
        return Easing::Linear;
    }

    // Find the candidate with the lowest SSE.
    let (mut best, mut best_err) = (0usize, errors[0]);
    for k in 1..CANDIDATES.len() {
        if errors[k] < best_err {
            best = k;
            best_err = errors[k];
        }
    }

    // Stay on Linear unless the alternative is meaningfully better.
    if best == 0 {
        return Easing::Linear;
    }
    let linear_err = errors[0];
    if linear_err > 0.0 && best_err <= linear_err * EASING_IMPROVEMENT_RATIO {
        CANDIDATES[best]
    } else {
        Easing::Linear
    }
}

// --------------------------------------------------------------------
// Stats helpers
// --------------------------------------------------------------------

fn median_f32(v: &[f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f32> = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s[s.len() / 2]
}

fn median_color(it: impl Iterator<Item = Color>) -> Color {
    let cs: Vec<Color> = it.collect();
    if cs.is_empty() {
        return Color::rgb(0, 0, 0);
    }
    let mut rs: Vec<u8> = cs.iter().map(|c| c.r).collect();
    let mut gs: Vec<u8> = cs.iter().map(|c| c.g).collect();
    let mut bs: Vec<u8> = cs.iter().map(|c| c.b).collect();
    rs.sort();
    gs.sort();
    bs.sort();
    Color::rgb(rs[rs.len() / 2], gs[gs.len() / 2], bs[bs.len() / 2])
}

fn round_to_int(x: f32) -> f32 {
    x.round()
}

// --------------------------------------------------------------------
// 2D RDP (kept around for future work — contour simplification in the
// path-primitive slice will reuse it).
// --------------------------------------------------------------------

/// Returns the indices of `points` to retain such that no removed
/// point lies more than `tolerance` units (perpendicular distance)
/// from the polyline through the retained points.
pub fn rdp(points: &[(f32, f32)], tolerance: f32) -> Vec<usize> {
    let n = points.len();
    if n < 3 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    rdp_recurse(points, 0, n - 1, tolerance, &mut keep);
    (0..n).filter(|&i| keep[i]).collect()
}

fn rdp_recurse(points: &[(f32, f32)], start: usize, end: usize, tol: f32, keep: &mut [bool]) {
    if end <= start + 1 {
        return;
    }
    let mut max_dist = 0.0f32;
    let mut max_idx = start;
    for i in (start + 1)..end {
        let d = perpendicular_distance(points[i], points[start], points[end]);
        if d > max_dist {
            max_dist = d;
            max_idx = i;
        }
    }
    if max_dist > tol {
        keep[max_idx] = true;
        rdp_recurse(points, start, max_idx, tol, keep);
        rdp_recurse(points, max_idx, end, tol, keep);
    }
}

fn perpendicular_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len_sq = dx * dx + dy * dy;
    if len_sq < f32::EPSILON {
        let ex = p.0 - a.0;
        let ey = p.1 - a.1;
        return (ex * ex + ey * ey).sqrt();
    }
    let num = (dy * p.0 - dx * p.1 + b.0 * a.1 - b.1 * a.0).abs();
    num / len_sq.sqrt()
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn blob_at(cx: f32, cy: f32) -> Blob {
        Blob {
            cx,
            cy,
            radius: 24.0,
            color: Color::rgb(0, 0, 0),
        }
    }

    #[test]
    fn rdp_keeps_corners_only() {
        let mut pts = Vec::new();
        for x in 0..=10 {
            pts.push((x as f32, 0.0));
        }
        for y in 1..=10 {
            pts.push((10.0, y as f32));
        }
        let kept = rdp(&pts, 0.5);
        assert_eq!(kept.first().copied(), Some(0));
        assert_eq!(kept.last().copied(), Some(pts.len() - 1));
        assert!(kept.contains(&10), "kept indices: {kept:?}");
        assert!(kept.len() <= 4, "expected ~3 keyframes, got {kept:?}");
    }

    /// The bug v0.2 fixes: a ball going right and retracing the same
    /// horizontal line back. 2D RDP on `(x, y)` would keep only the
    /// endpoints; temporal RDP must keep the apex.
    #[test]
    fn temporal_rdp_recovers_collinear_apex() {
        let mut blobs = Vec::new();
        for i in 0..=90u32 {
            let t = i * 33;
            let frac = if i <= 45 {
                i as f32 / 45.0
            } else {
                (90 - i) as f32 / 45.0
            };
            let x = 60.0 + frac * 360.0;
            blobs.push((t, blob_at(x, 160.0)));
        }
        let keep = simplify_trajectory(&blobs, 2.0);
        let last = blobs.len() - 1;
        assert!(keep.contains(&0), "missing start: {keep:?}");
        assert!(keep.contains(&last), "missing end: {keep:?}");
        assert!(
            keep.iter().any(|&i| (43..=47).contains(&i)),
            "missing apex near i=45: {keep:?}"
        );
    }

    #[test]
    fn easing_classifier_returns_linear_for_linear_motion() {
        let mut blobs = Vec::new();
        for i in 0..=90u32 {
            let t = i * 33;
            let x = 60.0 + (i as f32 / 90.0) * 360.0;
            blobs.push((t, blob_at(x, 160.0)));
        }
        let keep = vec![0usize, 90];
        assert_eq!(classify_easing(&blobs, &keep), Easing::Linear);
    }

    #[test]
    fn easing_classifier_picks_ease_in_out_when_data_demands_it() {
        // Synthesize an ease-in-out ball going 60 -> 420 over 3000ms.
        let mut blobs = Vec::new();
        for i in 0..=90u32 {
            let t = i * 33;
            let t_norm = i as f32 / 90.0;
            let p = Easing::EaseInOut.apply(t_norm);
            let x = 60.0 + p * 360.0;
            blobs.push((t, blob_at(x, 160.0)));
        }
        let keep = vec![0usize, 90];
        let easing = classify_easing(&blobs, &keep);
        assert!(
            matches!(easing, Easing::EaseInOut | Easing::EaseIn | Easing::EaseOut),
            "expected an eased classification, got {easing:?}"
        );
    }
}
