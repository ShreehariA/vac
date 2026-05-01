//! Across-frame tracking and AST emission.

use vac_format::{
    Animation, Canvas, Color, Document, Easing, Keyframe, Property, Scene, Shape, Statement,
    Transform, Value,
};

use crate::detect::{Blob, Detection};
use crate::frames::{estimate_fps, VideoFrame};

/// RDP tolerance, in pixels.  Below this, kinks in the trajectory are
/// considered noise and merged into a straight segment.
const RDP_TOLERANCE_PX: f32 = 2.0;

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

    // Background rectangle: always emitted.
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

        // Trajectory simplification → keyframes.
        let path: Vec<(f32, f32)> = blobs.iter().map(|(_, b)| (b.cx, b.cy)).collect();
        let mut keep = rdp(&path, RDP_TOLERANCE_PX);

        // Always include the first and last sample so timing is preserved
        // even if the ball never moves.
        if !keep.contains(&0) {
            keep.insert(0, 0);
        }
        let last_idx = blobs.len() - 1;
        if !keep.contains(&last_idx) {
            keep.push(last_idx);
        }
        keep.sort();
        keep.dedup();

        // Only bother emitting an animate block if the ball actually moves.
        if keep.len() >= 2 && trajectory_has_motion(&blobs) {
            let keyframes: Vec<Keyframe> = keep
                .iter()
                .map(|&i| {
                    let (t, b) = blobs[i];
                    // Anchor first keyframe at t=0 even if the very first
                    // detection is a few ms in. Keeps the .vac readable.
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
                easing: Easing::Linear,
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
// Ramer–Douglas–Peucker (2D)
// --------------------------------------------------------------------

/// Returns the indices of `points` to retain such that no removed
/// point lies more than `tolerance` units from the polyline through
/// the retained points.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rdp_keeps_corners_only() {
        // Right-angle path: 0,0 -> 10,0 -> 10,10
        // Densely sampled along each leg.
        let mut pts = Vec::new();
        for x in 0..=10 {
            pts.push((x as f32, 0.0));
        }
        for y in 1..=10 {
            pts.push((10.0, y as f32));
        }
        let kept = rdp(&pts, 0.5);
        // Should reduce to start, corner, end.
        assert_eq!(kept.first().copied(), Some(0));
        assert_eq!(kept.last().copied(), Some(pts.len() - 1));
        // Corner is at index 10 (10,0).
        assert!(kept.contains(&10), "kept indices: {kept:?}");
        assert!(kept.len() <= 4, "expected ~3 keyframes, got {kept:?}");
    }
}
