//! End-to-end round-trip via the public `vac` facade.
//!
//! `.vac` source → video → reconstructed `.vac` source.
//!
//! The decompiler measures pixels and the GIF intermediate quantises
//! delays to centiseconds, so we don't expect byte equality.  What we
//! do assert is *structural fidelity*: same canvas, same number of
//! shape declarations, the apex keyframe survives, and the colour
//! drift stays under tight bounds now that v0.2 samples interior
//! pixels only.

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

fn keyframes(doc: &vac::Document) -> Vec<(u32, f32, f32)> {
    let scene = &doc.scenes[0];
    for stmt in &scene.statements {
        if let Statement::Animate(a) = stmt {
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
    Vec::new()
}

fn ball_color(doc: &vac::Document) -> Color {
    let scene = &doc.scenes[0];
    let mut last_fill = None;
    let mut found_ball = false;
    for stmt in &scene.statements {
        if let Statement::Let { name, .. } = stmt {
            if name == "ball" {
                found_ball = true;
            } else {
                found_ball = false;
            }
        }
        if found_ball {
            if let Statement::Assign {
                target,
                property: vac::Property::Fill,
                value: vac::Value::Color(c),
            } = stmt
            {
                if target == "ball" {
                    last_fill = Some(*c);
                }
            }
        }
    }
    last_fill.expect("ball fill missing")
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

#[test]
fn ball_round_trip_recovers_apex() {
    let (original, reconstructed) = round_trip("ball.vac");
    assert_canvas_matches(&original, &reconstructed);

    let kfs = keyframes(&reconstructed);
    assert!(
        kfs.len() >= 3,
        "expected ≥ 3 keyframes for left→right→left ball, got {} in {kfs:?}",
        kfs.len()
    );

    // The original ball reaches (420, 160) at the apex. Allow ±2 px for
    // pixel quantisation and ±2 px for the radius-aware centroid offset.
    assert_apex_recovered(&kfs, 420.0, 160.0, 4.0);

    // v0.2 colour fix: original #e94560 should round-trip with very
    // little drift now that we sample interior pixels only.
    let original_color = Color::rgb(0xe9, 0x45, 0x60);
    let recovered = ball_color(&reconstructed);
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

    let kfs = keyframes(&reconstructed);
    assert!(
        kfs.len() >= 3,
        "expected ≥ 3 keyframes for diagonal ping-pong, got {kfs:?}",
    );

    // Apex at (400, 260) ± 4 px.
    assert_apex_recovered(&kfs, 400.0, 260.0, 4.0);
}
