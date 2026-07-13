//! End-to-end round-trip via the public `vac` facade.
//!
//! `.vac` source → video → reconstructed `.vac` source.
//!
//! Two backends are exercised:
//!
//! - **GIF** (pure-Rust, no system deps). Centisecond-quantised
//!   delays mean a 30fps source round-trips as 33fps and durations
//!   drift by ~10%.
//! - **MP4** (`ffmpeg` subprocess, v0.4+). FPS and duration are
//!   exact.  Skipped at runtime if `ffmpeg` isn't on PATH.
//!
//! What we assert in both cases is *structural fidelity*: same
//! canvas, the right number of recovered shapes, apex keyframes
//! survive, and colour drift stays under tight bounds.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use vac::{Color, DocumentExt, Shape, Statement, Transform};

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(&manifest)
        .to_path_buf()
}

/// Round-trip a `.vac` fixture through the named container.  `ext`
/// is `"gif"` or `"mp4"`.
fn round_trip_via(sample: &str, ext: &str) -> (vac::Document, vac::Document) {
    let ws = workspace_root();
    let sample_path = ws.join("examples").join(sample);

    let original = vac::read(&sample_path).expect("read sample");

    let out_dir = ws.join("target/test-roundtrip");
    std::fs::create_dir_all(&out_dir).unwrap();
    let stem = Path::new(sample).file_stem().unwrap().to_string_lossy();
    let video_path = out_dir.join(format!("{stem}.{ext}"));
    let recon_path = out_dir.join(format!("{stem}.{ext}.roundtrip.vac"));

    original
        .to_video(&video_path)
        .expect("compile to video");
    let reconstructed = vac::read_video(&video_path).expect("decompile");
    reconstructed.to_vac(&recon_path).expect("write recon vac");
    let _reparsed = vac::read(&recon_path).expect("re-parse round-tripped vac");

    eprintln!(
        "--- {sample} reconstructed via .{ext} ---\n{}",
        reconstructed.to_vac_string()
    );
    (original, reconstructed)
}

/// Convenience wrapper for the legacy GIF backend.
fn round_trip(sample: &str) -> (vac::Document, vac::Document) {
    round_trip_via(sample, "gif")
}

fn ffmpeg_on_path() -> bool {
    fn ok(bin: &str) -> bool {
        Command::new(bin)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    ok("ffmpeg") && ok("ffprobe")
}

/// Names of all non-`bg` shapes in declaration order.
/// v0.3 emits `shape_0`, `shape_1`, … so we look up by index.
fn shape_names(doc: &vac::Document) -> Vec<String> {
    doc.scenes[0]
        .statements
        .iter()
        .filter_map(|s| match s {
            Statement::Let { name, .. } if name != "bg" => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn shape_color(doc: &vac::Document, idx: usize) -> Color {
    let names = shape_names(doc);
    let target = names
        .get(idx)
        .unwrap_or_else(|| panic!("no shape at index {idx} (have {names:?})"));
    let mut found = None;
    for stmt in &doc.scenes[0].statements {
        if let Statement::Assign {
            target: t,
            property: vac::Property::Fill,
            value: vac::Value::Color(c),
        } = stmt
        {
            if t == target {
                found = Some(*c);
            }
        }
    }
    found.unwrap_or_else(|| panic!("fill missing for {target}"))
}

fn keyframes_for(doc: &vac::Document, idx: usize) -> Vec<(u32, f32, f32)> {
    let names = shape_names(doc);
    let target = names
        .get(idx)
        .unwrap_or_else(|| panic!("no shape at index {idx} (have {names:?})"));
    for stmt in &doc.scenes[0].statements {
        if let Statement::Animate(a) = stmt {
            if &a.target == target {
                return a
                    .keyframes
                    .iter()
                    .filter_map(|kf| {
                        kf.transforms.iter().find_map(|t| match t {
                            Transform::Position(x, y) => Some((kf.time_ms, *x, *y)),
                            _ => None,
                        })
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}

fn color_l1(a: Color, b: Color) -> u32 {
    (a.r as i32 - b.r as i32).unsigned_abs()
        + (a.g as i32 - b.g as i32).unsigned_abs()
        + (a.b as i32 - b.b as i32).unsigned_abs()
}

fn assert_canvas_matches(o: &vac::Document, r: &vac::Document) {
    assert_eq!(r.canvas.width, o.canvas.width);
    assert_eq!(r.canvas.height, o.canvas.height);
    let fps_diff = (r.canvas.fps as i32 - o.canvas.fps as i32).abs();
    assert!(
        fps_diff <= 5,
        "fps mismatch: original {} vs reconstructed {}",
        o.canvas.fps,
        r.canvas.fps,
    );
}

/// MP4 round-trips have no centisecond rounding — FPS must match
/// exactly. Duration may be one frame short of the source because the
/// renderer doesn't render the trailing instant (a 3s @30fps animation
/// produces 90 frames at t = 0..2966ms).
fn assert_canvas_exact(o: &vac::Document, r: &vac::Document) {
    assert_eq!(r.canvas.width, o.canvas.width);
    assert_eq!(r.canvas.height, o.canvas.height);
    assert_eq!(
        r.canvas.fps, o.canvas.fps,
        "v0.4 promise: MP4 round-trip must preserve fps exactly",
    );
}

fn assert_apex_recovered(kfs: &[(u32, f32, f32)], expected_x: f32, expected_y: f32, tol: f32) {
    assert!(
        kfs.iter().any(|&(_, x, y)| (x - expected_x).abs() <= tol
            && (y - expected_y).abs() <= tol),
        "no keyframe near ({expected_x}, {expected_y}) — got {kfs:?}",
    );
}

/// Find the index of the recovered shape whose mean colour is closest
/// to `target`.  Lets multi-ball tests assert "the red one's apex is X"
/// without depending on the tracker's internal ordering.
fn shape_index_by_color(doc: &vac::Document, target: Color) -> usize {
    let names = shape_names(doc);
    let (mut best, mut best_d) = (0usize, u32::MAX);
    for (i, _) in names.iter().enumerate() {
        let c = shape_color(doc, i);
        let d = color_l1(target, c);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

#[test]
fn ball_round_trip_recovers_apex() {
    let (original, reconstructed) = round_trip("ball.vac");
    assert_canvas_matches(&original, &reconstructed);
    assert_eq!(shape_names(&reconstructed).len(), 1, "expected 1 shape");

    let kfs = keyframes_for(&reconstructed, 0);
    assert!(
        kfs.len() >= 3,
        "expected ≥ 3 keyframes for left→right→left ball, got {} in {kfs:?}",
        kfs.len()
    );

    // The original ball reaches (420, 160) at the apex. Allow ±2 px for
    // pixel quantisation and ±2 px for the radius-aware centroid offset.
    assert_apex_recovered(&kfs, 420.0, 160.0, 4.0);

    let original_color = Color::rgb(0xe9, 0x45, 0x60);
    let recovered = shape_color(&reconstructed, 0);
    let drift = color_l1(original_color, recovered);
    assert!(
        drift <= 12,
        "ball colour drifted too far: {original_color:?} → {recovered:?} (L1 = {drift})",
    );
}

#[test]
fn diagonal_round_trip_recovers_apex() {
    let (original, reconstructed) = round_trip("diagonal.vac");
    assert_canvas_matches(&original, &reconstructed);
    assert_eq!(shape_names(&reconstructed).len(), 1, "expected 1 shape");

    let kfs = keyframes_for(&reconstructed, 0);
    assert!(
        kfs.len() >= 3,
        "expected ≥ 3 keyframes for diagonal ping-pong, got {kfs:?}",
    );

    // Apex at (400, 260) ± 4 px.
    assert_apex_recovered(&kfs, 400.0, 260.0, 4.0);
}

/// v0.3: two distinct-coloured balls moving in opposite directions
/// must produce two independent tracks → two `shape_<n>` declarations,
/// each with its own animate block, each apex recovered, each colour
/// recovered with the same tight drift bound as the single-ball case.
#[test]
fn two_balls_round_trip_recovers_both_apexes() {
    let (original, reconstructed) = round_trip("two_balls.vac");
    assert_canvas_matches(&original, &reconstructed);

    let names = shape_names(&reconstructed);
    assert_eq!(
        names.len(),
        2,
        "expected exactly 2 recovered shapes, got {names:?}",
    );

    let red = Color::rgb(0xe9, 0x45, 0x60);
    let yellow = Color::rgb(0xff, 0xd1, 0x66);
    let red_idx = shape_index_by_color(&reconstructed, red);
    let yellow_idx = shape_index_by_color(&reconstructed, yellow);
    assert_ne!(red_idx, yellow_idx, "tracker collapsed the two balls");

    let red_drift = color_l1(red, shape_color(&reconstructed, red_idx));
    let yellow_drift = color_l1(yellow, shape_color(&reconstructed, yellow_idx));
    assert!(red_drift <= 12, "red drifted too far: {red_drift}");
    assert!(yellow_drift <= 12, "yellow drifted too far: {yellow_drift}");

    // Red travels (60, 100) → (560, 100) → (60, 100) — apex at (560, 100).
    let red_kfs = keyframes_for(&reconstructed, red_idx);
    assert!(
        red_kfs.len() >= 3,
        "red track lost its apex: kfs={red_kfs:?}",
    );
    assert_apex_recovered(&red_kfs, 560.0, 100.0, 4.0);

    // Yellow travels (560, 260) → (60, 260) → (560, 260) — apex at (60, 260).
    let yellow_kfs = keyframes_for(&reconstructed, yellow_idx);
    assert!(
        yellow_kfs.len() >= 3,
        "yellow track lost its apex: kfs={yellow_kfs:?}",
    );
    assert_apex_recovered(&yellow_kfs, 60.0, 260.0, 4.0);
}

/// Look up the `Shape` of a named `let` statement.
fn shape_of(doc: &vac::Document, name: &str) -> Shape {
    for stmt in &doc.scenes[0].statements {
        if let Statement::Let {
            name: n,
            shape,
        } = stmt
        {
            if n == name {
                return *shape;
            }
        }
    }
    panic!("no `let {name} = ...` in document");
}

/// v0.4.1 — the rect/ellipse classifier.  A `rect(...)` source must
/// round-trip back to `rect(...)` (not collapse into an `ellipse`).
/// Width and height must survive — a 120×60 card stays 120×60, not
/// averaged into a 90×90 circle.
#[test]
fn sliding_rect_round_trip_preserves_rect_kind_and_aspect() {
    if !ffmpeg_on_path() {
        eprintln!("skipping rect round-trip: ffmpeg/ffprobe not on PATH");
        return;
    }
    let (_, reconstructed) = round_trip_via("sliding_rect.vac", "mp4");

    let names = shape_names(&reconstructed);
    assert_eq!(names.len(), 1, "expected exactly one non-bg shape");

    let shape = shape_of(&reconstructed, &names[0]);
    match shape {
        Shape::Rect { w, h, .. } => {
            // Source rect is 120×60.  Allow ±3 px for chroma-subsampling
            // / antialiasing on the edges.
            assert!(
                (w - 120.0).abs() <= 3.0,
                "rect width drifted: got {w}, expected 120",
            );
            assert!(
                (h - 60.0).abs() <= 3.0,
                "rect height drifted: got {h}, expected 60",
            );
        }
        Shape::Ellipse { .. } => panic!(
            "regression: sliding_rect.vac round-tripped as Ellipse instead of Rect ({shape:?})",
        ),
    }

    // Apex still recovered.
    let kfs = keyframes_for(&reconstructed, 0);
    assert!(kfs.len() >= 3, "rect lost its apex keyframe: {kfs:?}");
    assert_apex_recovered(&kfs, 300.0, 130.0, 4.0);
}

/// v0.5.1 — the stationarity filter.  A static card sharing the canvas
/// with a moving ball must round-trip as a bare `let` (no `animate`
/// block, no keyframe salad from centroid jitter) while the ball keeps
/// its keyframes.
#[test]
fn static_shape_round_trips_without_animate_block() {
    let (original, reconstructed) = round_trip("static_and_moving.vac");
    assert_canvas_matches(&original, &reconstructed);

    let names = shape_names(&reconstructed);
    assert_eq!(names.len(), 2, "expected card + ball, got {names:?}");

    let animated: Vec<&str> = reconstructed.scenes[0]
        .statements
        .iter()
        .filter_map(|s| match s {
            Statement::Animate(a) => Some(a.target.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        animated.len(),
        1,
        "exactly one shape should carry an animate block, got {animated:?}",
    );

    // The animated one must be the red ball, not the static blue card.
    let ball_idx = shape_index_by_color(&reconstructed, Color::rgb(0xe9, 0x45, 0x60));
    assert_eq!(
        animated[0], names[ball_idx],
        "the animate block should target the moving red ball",
    );
    let kfs = keyframes_for(&reconstructed, ball_idx);
    assert!(kfs.len() >= 3, "ball lost its apex keyframe: {kfs:?}");
    assert_apex_recovered(&kfs, 420.0, 240.0, 4.0);

    // The static card survives as a rect at its source geometry.
    let card_idx = 1 - ball_idx;
    match shape_of(&reconstructed, &names[card_idx]) {
        Shape::Rect { x, y, w, h } => {
            assert!(
                (x - 40.0).abs() <= 3.0 && (y - 40.0).abs() <= 3.0,
                "card anchor drifted: got ({x}, {y}), expected (40, 40)",
            );
            assert!(
                (w - 120.0).abs() <= 3.0 && (h - 60.0).abs() <= 3.0,
                "card size drifted: got {w}x{h}, expected 120x60",
            );
        }
        other => panic!("static card should stay a rect, got {other:?}"),
    }
}

// --------------------------------------------------------------------
// v0.4: MP4 round-trips
//
// These exercise the `ffmpeg` subprocess path. If `ffmpeg` / `ffprobe`
// aren't on PATH we skip cleanly so the test suite still runs in
// minimal environments — the GIF tests above keep the algorithmic
// pipeline covered there.
// --------------------------------------------------------------------

#[test]
fn ball_round_trip_mp4_preserves_fps_and_apex() {
    if !ffmpeg_on_path() {
        eprintln!("skipping MP4 round-trip: ffmpeg/ffprobe not on PATH");
        return;
    }

    let (original, reconstructed) = round_trip_via("ball.vac", "mp4");
    assert_canvas_exact(&original, &reconstructed);

    let kfs = keyframes_for(&reconstructed, 0);
    assert!(
        kfs.len() >= 3,
        "expected ≥ 3 keyframes for ping-pong ball, got {kfs:?}",
    );
    // With MP4's exact FPS the apex now lands on 1500ms ± 1 frame
    // (33ms) instead of 1350ms-on-GIF.
    let apex = kfs
        .iter()
        .find(|&&(_, x, y)| (x - 420.0).abs() <= 4.0 && (y - 160.0).abs() <= 4.0)
        .unwrap_or_else(|| panic!("no apex keyframe in {kfs:?}"));
    let drift_ms = (apex.0 as i32 - 1500).abs();
    assert!(
        drift_ms <= 33,
        "apex time drifted by {drift_ms}ms (expected ~0ms with MP4); kfs={kfs:?}",
    );

    let original_color = Color::rgb(0xe9, 0x45, 0x60);
    let recovered = shape_color(&reconstructed, 0);
    let drift = color_l1(original_color, recovered);
    // libx264 CRF 18 plus yuv420p chroma subsampling can drift colour
    // a touch more than the GIF path's interior-pixel sampling, but
    // it should still stay tightly bounded.
    assert!(
        drift <= 18,
        "ball colour drifted too far via MP4: {original_color:?} → {recovered:?} (L1 = {drift})",
    );
}

#[test]
fn two_balls_round_trip_mp4_preserves_both_tracks() {
    if !ffmpeg_on_path() {
        eprintln!("skipping MP4 round-trip: ffmpeg/ffprobe not on PATH");
        return;
    }

    let (original, reconstructed) = round_trip_via("two_balls.vac", "mp4");
    assert_canvas_exact(&original, &reconstructed);

    let names = shape_names(&reconstructed);
    assert_eq!(
        names.len(),
        2,
        "expected 2 recovered shapes via MP4, got {names:?}",
    );

    let red = Color::rgb(0xe9, 0x45, 0x60);
    let yellow = Color::rgb(0xff, 0xd1, 0x66);
    let red_idx = shape_index_by_color(&reconstructed, red);
    let yellow_idx = shape_index_by_color(&reconstructed, yellow);
    assert_ne!(red_idx, yellow_idx);

    let red_drift = color_l1(red, shape_color(&reconstructed, red_idx));
    let yellow_drift = color_l1(yellow, shape_color(&reconstructed, yellow_idx));
    assert!(red_drift <= 18, "red MP4 drift too high: {red_drift}");
    assert!(yellow_drift <= 18, "yellow MP4 drift too high: {yellow_drift}");

    let red_kfs = keyframes_for(&reconstructed, red_idx);
    let yellow_kfs = keyframes_for(&reconstructed, yellow_idx);
    assert!(red_kfs.len() >= 3 && yellow_kfs.len() >= 3);
    assert_apex_recovered(&red_kfs, 560.0, 100.0, 4.0);
    assert_apex_recovered(&yellow_kfs, 60.0, 260.0, 4.0);

    // v0.4 promise: with MP4, both apex *times* land on 1500ms exactly
    // (within one frame) — not 1350ms like the GIF path's centisecond
    // rounding produced.
    for kfs in [&red_kfs, &yellow_kfs] {
        let mid = kfs
            .iter()
            .find(|&&(t, _, _)| (t as i32 - 1500).abs() <= 33)
            .unwrap_or_else(|| panic!("no keyframe near 1500ms in {kfs:?}"));
        let _ = mid;
    }
}
