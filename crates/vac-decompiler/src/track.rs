//! Cross-frame tracking, keyframe extraction, easing classification,
//! AST emission.
//!
//! v0.3 promotes the pipeline from single-blob to multi-blob:
//!
//!   per-frame Vec<Blob>  →  build_tracks  →  Vec<Track>
//!                                              │
//!                                              ▼
//!                       per-track simplify + easing → animate
//!
//! The matching is a lowest-cost-first greedy bipartite assignment
//! between active tracks and the current frame's detections, with a
//! gate threshold that opens new tracks for unmatched blobs and a
//! short look-back tolerance that survives 1-3 frames of detection
//! noise.  Cost combines position, RGB colour, radius, and Hu
//! image-moment shape similarity — Hu is most useful when v0.5
//! introduces non-elliptical primitives, but it costs nothing to
//! compute now and adds a small bonus for ball-radius differences.
//!
//! Trajectory simplification and easing classification are
//! per-track and reuse the v0.2 implementation verbatim.

use vac_format::{
    Animation, Canvas, Color, Document, Easing, Keyframe, Property, Scene, Shape, Statement,
    Transform, Value,
};

use crate::detect::{Blob, Detection};
use crate::frames::{estimate_fps, VideoFrame};

/// 1D RDP tolerance, in pixels.  Below this, deviations from the
/// straight-line interpolation between segment endpoints are noise.
const RDP_TOLERANCE_PX: f32 = 2.0;

/// Easing classifier only switches off `Linear` when an alternative
/// fits at least this much better (lower SSE).
const EASING_IMPROVEMENT_RATIO: f64 = 0.90;

// --- Tracking weights & gates ----------------------------------------
// Cost = W_POS·|Δpos| + W_COL·|Δrgb| + W_RAD·|Δr| + W_HU·|Δhu|.
// The weights are picked so that for two distinct-coloured balls a
// frame apart the same-track cost is ~10 and the cross-track cost is
// > GATE.  Values within ±2× still match correctly in practice.
const W_POS: f32 = 1.0;
const W_COL: f32 = 0.4;
const W_RAD: f32 = 2.0;
const W_HU: f32 = 30.0;
const COST_GATE: f32 = 80.0;

/// Maximum number of consecutive frames a track may go unseen
/// before it is closed out.  Bumps a bit of robustness against
/// brief occlusion / detection drop-outs.
const MAX_GAP_FRAMES: usize = 3;

/// A blob fewer than this many samples long is treated as a
/// noise track and dropped entirely from the output.
const MIN_TRACK_SAMPLES: usize = 3;

#[derive(Debug, Clone)]
pub struct Track {
    pub id: usize,
    /// (t_ms, blob) ordered by time.
    pub samples: Vec<(u32, Blob)>,
    last_seen_frame: usize,
    active: bool,
}

pub fn build_document(frames: &[VideoFrame], detections: &[Detection]) -> Document {
    let canvas = Canvas {
        width: frames[0].width,
        height: frames[0].height,
        fps: estimate_fps(frames),
    };

    let bg_color = median_color(detections.iter().map(|d| d.bg));

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

    let mut tracks = build_tracks(detections);
    // Filter out tracks that are too short to be meaningful.
    tracks.retain(|t| t.samples.len() >= MIN_TRACK_SAMPLES);
    // Stable order: longest tracks first → lowest indices to the most
    // prominent objects.  Tie-break by first-seen time.
    tracks.sort_by(|a, b| {
        b.samples
            .len()
            .cmp(&a.samples.len())
            .then_with(|| a.samples[0].0.cmp(&b.samples[0].0))
    });

    for (idx, track) in tracks.iter().enumerate() {
        emit_track(&mut statements, idx, track);
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

fn emit_track(statements: &mut Vec<Statement>, idx: usize, track: &Track) {
    let name = format!("shape_{idx}");
    let radii: Vec<f32> = track.samples.iter().map(|(_, b)| b.radius).collect();
    let radius = round_to_int(median_f32(&radii));
    let color = median_color(track.samples.iter().map(|(_, b)| b.color));
    let (first_t, first_b) = track.samples[0];

    statements.push(Statement::Let {
        name: name.clone(),
        shape: Shape::Ellipse {
            cx: round_to_int(first_b.cx),
            cy: round_to_int(first_b.cy),
            rx: radius,
            ry: radius,
        },
    });
    statements.push(Statement::Assign {
        target: name.clone(),
        property: Property::Fill,
        value: Value::Color(color),
    });
    statements.push(Statement::Assign {
        target: name.clone(),
        property: Property::Stroke,
        value: Value::None,
    });

    let blobs = &track.samples;

    let mut keep = simplify_trajectory(blobs, RDP_TOLERANCE_PX);
    if !keep.contains(&0) {
        keep.insert(0, 0);
    }
    let last_idx = blobs.len() - 1;
    if !keep.contains(&last_idx) {
        keep.push(last_idx);
    }
    keep.sort();
    keep.dedup();

    if keep.len() >= 2 && trajectory_has_motion(blobs) {
        let easing = classify_easing(blobs, &keep);

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
            target: name,
            keyframes,
            easing,
        }));
    }
}

// --------------------------------------------------------------------
// Cross-frame tracking
// --------------------------------------------------------------------

/// Build tracks from per-frame detections via lowest-cost-first
/// greedy bipartite matching with gating + a short gap-tolerance.
pub fn build_tracks(detections: &[Detection]) -> Vec<Track> {
    let mut tracks: Vec<Track> = Vec::new();
    let mut next_id = 0usize;

    for (frame_idx, det) in detections.iter().enumerate() {
        // Snapshot of currently active track indices.
        let active: Vec<usize> = tracks
            .iter()
            .enumerate()
            .filter_map(|(i, t)| if t.active { Some(i) } else { None })
            .collect();

        // Build all viable (cost, blob, track) candidates.
        let mut candidates: Vec<(f32, usize, usize)> =
            Vec::with_capacity(det.blobs.len() * active.len());
        for (b_idx, blob) in det.blobs.iter().enumerate() {
            for &t_idx in &active {
                let last_blob = tracks[t_idx].samples.last().unwrap().1;
                let cost = matching_cost(&last_blob, blob);
                if cost <= COST_GATE {
                    candidates.push((cost, b_idx, t_idx));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut blob_assigned = vec![false; det.blobs.len()];
        let mut track_assigned: std::collections::HashSet<usize> = std::collections::HashSet::new();

        for (_cost, b_idx, t_idx) in candidates {
            if blob_assigned[b_idx] || track_assigned.contains(&t_idx) {
                continue;
            }
            tracks[t_idx].samples.push((det.t_ms, det.blobs[b_idx]));
            tracks[t_idx].last_seen_frame = frame_idx;
            blob_assigned[b_idx] = true;
            track_assigned.insert(t_idx);
        }

        // Unassigned blobs → start new tracks.
        for (b_idx, blob) in det.blobs.iter().enumerate() {
            if !blob_assigned[b_idx] {
                tracks.push(Track {
                    id: next_id,
                    samples: vec![(det.t_ms, *blob)],
                    last_seen_frame: frame_idx,
                    active: true,
                });
                next_id += 1;
            }
        }

        // Expire stale tracks.
        for t in tracks.iter_mut() {
            if t.active && frame_idx.saturating_sub(t.last_seen_frame) > MAX_GAP_FRAMES {
                t.active = false;
            }
        }
    }

    tracks
}

fn matching_cost(prev: &Blob, curr: &Blob) -> f32 {
    let pos = ((curr.cx - prev.cx).powi(2) + (curr.cy - prev.cy).powi(2)).sqrt();
    let dr = curr.color.r as f32 - prev.color.r as f32;
    let dg = curr.color.g as f32 - prev.color.g as f32;
    let db = curr.color.b as f32 - prev.color.b as f32;
    let col = (dr * dr + dg * dg + db * db).sqrt();
    let rad = (curr.radius - prev.radius).abs();
    let hu = (0..7)
        .map(|i| (curr.hu[i] - prev.hu[i]).abs())
        .sum::<f32>();
    W_POS * pos + W_COL * col + W_RAD * rad + W_HU * hu
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
// Temporal-aware trajectory simplification (v0.2 — unchanged).
// --------------------------------------------------------------------

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
// Easing classification (v0.2 — unchanged).
// --------------------------------------------------------------------

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

    let (mut best, mut best_err) = (0usize, errors[0]);
    for k in 1..CANDIDATES.len() {
        if errors[k] < best_err {
            best = k;
            best_err = errors[k];
        }
    }

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
// 2D RDP (kept for future contour simplification work).
// --------------------------------------------------------------------

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
            area: 1800,
            hu: [0.0; 7],
        }
    }

    fn blob_full(cx: f32, cy: f32, color: Color) -> Blob {
        Blob {
            cx,
            cy,
            radius: 24.0,
            color,
            area: 1800,
            hu: [0.0; 7],
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

    /// Two distinct-coloured balls moving in opposite directions
    /// across the same y must produce two stable tracks.
    #[test]
    fn tracker_separates_two_balls_by_colour() {
        let red = Color::rgb(0xe9, 0x45, 0x60);
        let yellow = Color::rgb(0xff, 0xd1, 0x66);

        let mut detections: Vec<Detection> = Vec::new();
        for i in 0..=60u32 {
            let t = i * 33;
            let frac = i as f32 / 60.0;
            let xr = 60.0 + frac * 480.0;
            let xy = 540.0 - frac * 480.0;
            detections.push(Detection {
                frame_index: i as usize,
                t_ms: t,
                bg: Color::rgb(0x1a, 0x1a, 0x2e),
                blobs: vec![blob_full(xr, 100.0, red), blob_full(xy, 260.0, yellow)],
            });
        }

        let tracks = build_tracks(&detections);
        let long: Vec<&Track> = tracks.iter().filter(|t| t.samples.len() >= 30).collect();
        assert_eq!(
            long.len(),
            2,
            "expected exactly 2 long tracks, got {} ({tracks:#?})",
            long.len()
        );

        let red_track = long
            .iter()
            .find(|t| t.samples[0].1.color == red)
            .expect("no red track");
        let yellow_track = long
            .iter()
            .find(|t| t.samples[0].1.color == yellow)
            .expect("no yellow track");

        // Red moves 60 → 540, yellow moves 540 → 60. End positions confirm.
        let (_, red_last) = red_track.samples.last().unwrap();
        let (_, yellow_last) = yellow_track.samples.last().unwrap();
        assert!(red_last.cx > 400.0, "red did not advance: cx={}", red_last.cx);
        assert!(
            yellow_last.cx < 200.0,
            "yellow did not advance: cx={}",
            yellow_last.cx
        );
    }
}
