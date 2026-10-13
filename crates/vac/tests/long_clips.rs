//! v1.16: long clips are decompiled a segment at a time.
//!
//! Through v1.15 every frame of a clip was decoded and held at once, and
//! the search held copies of them. An MP4 over the segment budget is now
//! scanned once (frame count, the steps between frames where the cuts are,
//! the metric's ink reference) and then streamed, a segment — a shot, or a
//! slice of a long one — decompiled and dropped at a time.

use std::path::PathBuf;

use vac::DecompileOptions;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// A segment budget that sends `shots.vac` (90 frames of 480x320, 13.8M
/// pixels) down the segmented path while each of its 30-frame shots fits:
/// the segments are then exactly its shots, and a clip decompiled a shot at
/// a time must come back exactly as it does held whole — the same scenes,
/// the same source bytes, the same score against the same ink reference.
#[test]
fn a_clip_decompiled_a_shot_at_a_time_matches_it_held_whole() {
    if !vac_decompiler::frames_mp4::ffmpeg_available() {
        eprintln!("SKIPPED: ffmpeg not available");
        return;
    }
    let dir = std::env::temp_dir().join(format!("vac_long_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mp4 = dir.join("shots.mp4");
    let source = vac::read(examples_dir().join("shots.vac")).expect("read fixture");
    vac::write_video(&source, &mp4).expect("compile to MP4");

    let opts = DecompileOptions::default();
    let whole = vac::verify(&mp4, &opts).expect("verify, held whole");
    let streamed = vac::verify_with_budget(&mp4, &opts, 10_000_000).expect("verify, a segment at a time");
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(whole.document.scenes.len(), 3, "three shots");
    assert_eq!(streamed.document.scenes.len(), 3, "the segments are the shots");
    for (a, b) in whole.document.scenes.iter().zip(&streamed.document.scenes) {
        assert_eq!(a.duration_ms, b.duration_ms, "{} keeps its length", a.name);
    }
    assert_eq!(streamed.vac_bytes, whole.vac_bytes, "the same source");
    assert_eq!(streamed.source_frames, whole.source_frames);
    assert!(
        (streamed.fidelity.psnr_db - whole.fidelity.psnr_db).abs() < 0.01
            && (streamed.fidelity.ink_iou - whole.fidelity.ink_iou).abs() < 1e-4,
        "the same score: streamed {:.3} dB / {:.4}, whole {:.3} dB / {:.4}",
        streamed.fidelity.psnr_db,
        streamed.fidelity.ink_iou,
        whole.fidelity.psnr_db,
        whole.fidelity.ink_iou
    );
}
