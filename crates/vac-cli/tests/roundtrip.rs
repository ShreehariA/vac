//! End-to-end round-trip via the public `vac` facade.
//!
//! `.vac` source → video → reconstructed `.vac` source.
//!
//! Tolerances are deliberate: the decompiler measures pixels, GIF
//! quantises delays to centiseconds, and that's expected — what we
//! assert is that the round-trip is *structurally* faithful.

use std::path::PathBuf;

use vac::DocumentExt;

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is `<root>/crates/vac-cli`; pop two segments.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(&manifest)
        .to_path_buf()
}

#[test]
fn ball_round_trip() {
    let ws = workspace_root();
    let sample_path = ws.join("examples/ball.vac");

    let original = vac::read(&sample_path).expect("read ball.vac");

    let out_dir = ws.join("target/test-roundtrip");
    std::fs::create_dir_all(&out_dir).unwrap();
    let gif_path = out_dir.join("ball.gif");
    let recon_path = out_dir.join("ball.roundtrip.vac");

    // .vac → video
    original.to_video(&gif_path).expect("compile to gif");
    assert!(gif_path.exists(), "compiled gif should exist");

    // video → .vac
    let reconstructed = vac::read_video(&gif_path).expect("decompile");
    reconstructed
        .to_vac(&recon_path)
        .expect("write reconstructed .vac");

    // The reconstructed file must be parseable again.
    let _reparsed = vac::read(&recon_path).expect("re-parse round-tripped vac");

    // Structural assertions (deliberately tolerant — we measure pixels).
    assert_eq!(reconstructed.canvas.width, original.canvas.width);
    assert_eq!(reconstructed.canvas.height, original.canvas.height);
    let fps_diff = (reconstructed.canvas.fps as i32 - original.canvas.fps as i32).abs();
    assert!(
        fps_diff <= 5,
        "fps mismatch: original {} vs reconstructed {}",
        original.canvas.fps,
        reconstructed.canvas.fps,
    );

    let scene = &reconstructed.scenes[0];
    let has_animate = scene
        .statements
        .iter()
        .any(|s| matches!(s, vac::Statement::Animate(_)));
    assert!(has_animate, "reconstructed scene should have an animate block");

    eprintln!("--- reconstructed ball.vac ---\n{}", reconstructed.to_vac_string());
}
