//! End-to-end round-trip via the public `vac` facade.
//!
//! `.vac` source → video → reconstructed `.vac` source.
//!
//! The decompiler measures pixels and the GIF intermediate quantises
//! delays to centiseconds, so we don't expect byte equality.  What we
//! do assert is *structural fidelity*: same canvas, the right number
//! of recovered shapes, the apex keyframe survives per shape, and the
//! colour drift stays under tight bounds now that v0.2 samples
//! interior pixels only.

use std::path::{Path, PathBuf};

use vac::{Color, DocumentExt, Statement, Transform};

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(&manifest)
        .to_path_buf()
}

fn round_trip(sample: &str) -> (vac::Document, vac::Document) {
    let ws = workspace_root();
    let sample_path = ws.join("examples").join(sample);

    let original = vac::read(&sample_path).expect("read sample");

    let out_dir = ws.join("target/test-roundtrip");
    std::fs::create_dir_all(&out_dir).unwrap();
    let stem = Path::new(sample).file_stem().unwrap().to_string_lossy();
    let gif_path = out_dir.join(format!("{stem}.gif"));
    let recon_path = out_dir.join(format!("{stem}.roundtrip.vac"));

    original.to_video(&gif_path).expect("compile to gif");
    let reconstructed = vac::read_video(&gif_path).expect("decompile");
    reconstructed.to_vac(&recon_path).expect("write recon vac");
    let _reparsed = vac::read(&recon_path).expect("re-parse round-tripped vac");

    eprintln!(
        "--- {} reconstructed ---\n{}",
        sample,
        reconstructed.to_vac_string()
    );
    (original, reconstructed)
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
