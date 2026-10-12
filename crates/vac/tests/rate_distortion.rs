//! v1.0 — VAC as a codec: one quality dial, a measured rate–distortion
//! curve, and rate control against a fidelity target.
//!
//! The fast tests run on the hermetic fixtures and gate CI. The recorder
//! is `#[ignore]`d — it decompiles every corpus clip at five qualities,
//! each through the four-way measured search — and writes
//! `benches/rate-distortion.tsv`:
//!
//! ```text
//! cargo test --release -p vac --test rate_distortion -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};

use vac::DecompileOptions;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fixture_frames(name: &str) -> Vec<vac::VideoFrame> {
    let doc = vac::read(repo_root().join("examples").join(name)).unwrap();
    vac::rendered_to_video_frames(&vac::render(&doc).unwrap(), doc.canvas.fps)
}

/// (source bytes, residual) for one decompile at `quality`.
fn point(frames: &[vac::VideoFrame], quality: f32) -> (usize, f64) {
    let opts = DecompileOptions {
        quality,
        ..Default::default()
    };
    let doc = vac::decompile_frames(frames.to_vec(), &opts).unwrap();
    let fid = vac::measure(frames, &doc).unwrap();
    (vac::to_string(&doc).len(), fid.residual)
}

/// The dial's two ends must be a real trade: the low end never costs more
/// bytes than the default, the high end never fewer, and the high end is
/// never less faithful than the low end. Measured on fixtures whose motion
/// is curved (keyframe-hungry) and which carry outlines.
#[test]
fn the_quality_dial_trades_bytes_for_fidelity() {
    for name in ["spin.vac", "two_balls.vac", "triangle.vac", "grow.vac"] {
        let frames = fixture_frames(name);
        let (b0, r0) = point(&frames, 0.0);
        let (b50, r50) = point(&frames, 50.0);
        let (b100, r100) = point(&frames, 100.0);
        println!("{name}: q0 {b0} B {r0:.2} | q50 {b50} B {r50:.2} | q100 {b100} B {r100:.2}");
        assert!(b0 <= b50 && b50 <= b100, "{name}: bytes not monotone: {b0} {b50} {b100}");
        assert!(r100 <= r0 + 1e-6, "{name}: q100 residual {r100} worse than q0 {r0}");
    }
}

/// Asking for less fidelity than the default delivers must cost fewer
/// bytes than the default, and still meet the target.
#[test]
fn rate_control_meets_a_reachable_target_with_fewer_bytes() {
    let frames = fixture_frames("spin.vac");
    let (b50, _) = point(&frames, 50.0);
    let default_psnr = {
        let doc = vac::decompile_frames(frames.clone(), &DecompileOptions::default()).unwrap();
        vac::measure(&frames, &doc).unwrap().psnr_db
    };
    let target = default_psnr - 1.5;
    let rc = vac::decompile_to_target(frames.clone(), &DecompileOptions::default(), target).unwrap();
    println!("{:?}", rc.probes);
    assert!(rc.met, "target {target:.2} dB should be reachable");
    assert!(rc.fidelity.psnr_db >= target);
    assert!(
        rc.vac_bytes < b50,
        "a looser target must cost fewer bytes than the default: {} vs {b50}",
        rc.vac_bytes
    );
}

/// An unreachable target returns the most faithful probe, flagged.
#[test]
fn rate_control_reports_an_unreachable_target() {
    let frames = fixture_frames("triangle.vac");
    let rc = vac::decompile_to_target(frames, &DecompileOptions::default(), 99.0).unwrap();
    assert!(!rc.met);
    let best = rc.probes.iter().map(|p| p.2).fold(f64::MIN, f64::max);
    assert!((rc.fidelity.psnr_db - best).abs() < 1e-9);
}

/// The first version of the dial scaled the minimum blob *area* with
/// quality, and at quality 0 (144 px) deleted every tick of the 32 px
/// `real_loader` spinner: IoU 0.69 → 0.00. Lowering quality must trade
/// detail for bytes, never delete the subject. Small, high-contrast
/// shapes — this fixture's six 5×8 ticks on a 64 px canvas — must all
/// survive quality 0.
#[test]
fn low_quality_never_deletes_small_high_contrast_shapes() {
    let mut src = String::from("canvas 64x64 @20fps\nscene s {\n  duration 1s\n  let bg = rect(0, 0, 64, 64)\n  bg.fill = #000000\n");
    for i in 0..6 {
        let x = 4 + i * 10;
        src.push_str(&format!("  let t{i} = rect({x}, 20, 5, 8)\n  t{i}.fill = #f0f0f0\n"));
        if i % 2 == 0 {
            src.push_str(&format!(
                "  animate t{i} {{\n    0ms -> position({x}, 20)\n    1s -> position({x}, 40)\n    easing: linear\n  }}\n"
            ));
        }
    }
    src.push_str("}\n");
    let doc = vac::parse(&src).unwrap();
    let frames = vac::rendered_to_video_frames(&vac::render(&doc).unwrap(), doc.canvas.fps);
    let count = |q: f32| {
        let opts = DecompileOptions { quality: q, ..Default::default() };
        let d = vac::decompile_frames(frames.clone(), &opts).unwrap();
        let fid = vac::measure(&frames, &d).unwrap();
        let n = d
            .scenes
            .iter()
            .flat_map(|s| &s.statements)
            .filter(|st| matches!(st, vac::Statement::Let { name, .. } if name != "bg"))
            .count();
        (n, fid.ink_iou)
    };
    let (n50, iou50) = count(50.0);
    let (n0, iou0) = count(0.0);
    assert_eq!(n50, 6, "the default should recover all six ticks");
    assert_eq!(n0, 6, "quality 0 deleted ticks: {n0} of 6 left");
    assert!(iou0 > 0.8 && iou50 > 0.8, "IoU q0 {iou0:.3}, q50 {iou50:.3}");
}

const QUALITIES: [f32; 5] = [0.0, 25.0, 50.0, 75.0, 100.0];

#[test]
#[ignore = "slow: decompiles every corpus clip at five qualities; records benches/rate-distortion.tsv"]
fn record_rate_distortion_curves() {
    let inputs_dir = repo_root().join("examples/wild/inputs");
    let mut inputs: Vec<PathBuf> = std::fs::read_dir(&inputs_dir)
        .map(|rd| {
            rd.filter_map(|e| {
                let p = e.ok()?.path();
                let name = p.file_name()?.to_string_lossy().into_owned();
                let ext = p.extension()?.to_string_lossy().to_ascii_lowercase();
                (!name.starts_with("ai_") && (ext == "gif" || ext == "mp4")).then_some(p)
            })
            .collect()
        })
        .unwrap_or_default();
    inputs.sort();
    if inputs.is_empty() {
        eprintln!("SKIPPED: no corpus inputs under {}", inputs_dir.display());
        return;
    }

    let mut out = String::from(
        "# VAC rate–distortion curves (v1.0).\n\
         #\n\
         # Regenerate: cargo test --release -p vac --test rate_distortion -- --ignored\n\
         #\n\
         # Each row is one clip decompiled at one quality (0–100; 50 is the default).\n\
         # vac_bytes is the RATE: the size of the human-readable .vac source.\n\
         # residual / psnr_db / ink_iou are the DISTORTION of its reconstruction.\n\
         # source_bytes is the input video file, for scale.\n\
         input\tquality\tvac_bytes\tsource_bytes\tresidual\tpsnr_db\tink_iou\tshapes\tkeyframes\n",
    );
    for path in &inputs {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let source_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let frames = match vac::load_video_frames(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("SKIPPED {name}: {e}");
                continue;
            }
        };
        for q in QUALITIES {
            let opts = DecompileOptions {
                quality: q,
                ..Default::default()
            };
            let doc = vac::decompile_frames(frames.clone(), &opts).unwrap();
            let fid = vac::measure(&frames, &doc).unwrap();
            let text = vac::to_string(&doc);
            let keyframes: usize = doc
                .scenes
                .iter()
                .flat_map(|s| &s.statements)
                .map(|st| match st {
                    vac::Statement::Animate(a) => a.keyframes.len(),
                    _ => 0,
                })
                .sum();
            let shapes = doc
                .scenes
                .iter()
                .flat_map(|s| &s.statements)
                .filter(|st| matches!(st, vac::Statement::Let { .. }))
                .count();
            let psnr = if fid.psnr_db.is_finite() { fid.psnr_db } else { 99.0 };
            let row = format!(
                "{name}\t{q}\t{}\t{source_bytes}\t{:.2}\t{:.2}\t{:.4}\t{shapes}\t{keyframes}\n",
                text.len(),
                fid.residual,
                psnr,
                fid.ink_iou
            );
            print!("{row}");
            out.push_str(&row);
        }
    }
    std::fs::write(repo_root().join("benches/rate-distortion.tsv"), out).unwrap();
}
