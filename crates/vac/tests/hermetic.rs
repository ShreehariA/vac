//! Tier 3 — ground-truth round-trip, with no codec in the loop.
//!
//! Every other end-to-end test in this repo goes through GIF or MP4,
//! which means every measurement it takes is contaminated by palette
//! quantisation or H.264 lossiness. This one is not:
//!
//! ```text
//! examples/foo.vac → parse → render → frames
//!                                       ↓ decompile_frames
//!                                    Document'
//!                                       ↓ render
//!                                    frames'
//!   pixels: frames vs frames'   → fidelity
//!   AST:    Document vs Document'  → recovery error
//! ```
//!
//! Pure Rust, deterministic, no ffmpeg, no network, no temp files. Each
//! `examples/*.vac` is free ground truth — we know exactly what the
//! answer should be, because we wrote it.
//!
//! **What this tier cannot tell you.** It feeds the decompiler exactly
//! the clean, flat-shaded, anti-aliased synthetic content its thresholds
//! were tuned on, so the numbers here are flattering by construction and
//! are *not* evidence about real footage. This tier exists to catch
//! regressions, which it does very sensitively. Progress on real video is
//! Tier 4's job (`benches/baseline.json`).

use std::path::{Path, PathBuf};

use vac::{DecompileOptions, Document, Fidelity, Shape, Statement};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

struct RoundTrip {
    source: Document,
    recovered: Document,
    fidelity: Fidelity,
}

fn hermetic(sample: &str) -> RoundTrip {
    hermetic_with(sample, &DecompileOptions::default())
}

fn hermetic_with(sample: &str, opts: &DecompileOptions) -> RoundTrip {
    let path = examples_dir().join(sample);
    let source = vac::read(&path).unwrap_or_else(|e| panic!("reading {sample}: {e}"));

    let rendered = vac::render(&source).expect("render source");
    assert!(!rendered.is_empty(), "{sample} rendered zero frames");
    let frames = vac::rendered_to_video_frames(&rendered, source.canvas.fps);

    let recovered = vac::decompile_frames(frames.clone(), opts).expect("decompile");
    let fidelity = vac::measure(&frames, &recovered).expect("measure");

    RoundTrip {
        source,
        recovered,
        fidelity,
    }
}

/// Non-background `let` bindings, in declaration order.
fn shapes(doc: &Document) -> Vec<(&str, &Shape)> {
    doc.scenes
        .iter()
        .flat_map(|s| &s.statements)
        .filter_map(|st| match st {
            Statement::Let { name, shape } if name != "bg" => Some((name.as_str(), shape)),
            _ => None,
        })
        .collect()
}

fn animate_count(doc: &Document) -> usize {
    doc.scenes
        .iter()
        .flat_map(|s| &s.statements)
        .filter(|st| matches!(st, Statement::Animate(_)))
        .count()
}

/// Anchor point of a shape, in the coordinate `position()` drives.
fn anchor(shape: &Shape) -> (f32, f32) {
    match shape {
        Shape::Rect { x, y, .. } | Shape::Image { x, y, .. } => (*x, *y),
        Shape::Ellipse { cx, cy, .. } => (*cx, *cy),
        Shape::Path(segs) => {
            vac_format::path_anchor(segs)
        }
    }
}

// ====================================================================
// Per-fixture fidelity floors
// ====================================================================
//
// A single global floor does not work here. Measured hermetic IoU ranges
// from 0.91 to 0.9997 across these fixtures, so any floor loose enough
// for the worst one leaves the best ones with ~0.09 of dead slack — and
// a test with that much slack does not fail when the pipeline breaks.
//
// Verified: with a global floor of 0.80, multiplying `RDP_TOLERANCE_PX`
// by ten (a gross degradation that the corpus tier caught immediately,
// halving `real_loader`'s IoU) left all twelve hermetic tests green.
// That is the "test that cannot fail" failure mode this whole tier
// exists to eliminate, so the floors are per-fixture and tight.
//
// Values re-measured 2026-10-13 at v1.11. The tolerance below absorbs platform
// rasteriser differences — `tiny-skia` picks SIMD backends by target
// feature, so f32 rasterisation is not guaranteed bit-identical between
// an aarch64 laptop and an x86_64 CI runner — and nothing more.

/// How far below its recorded value a fixture may score before it counts
/// as a regression.
const IOU_TOL: f64 = 0.01;

/// Recorded hermetic scores: (fixture, ink_iou, residual).
const BASELINE: &[(&str, f64, f64)] = &[
    ("ball.vac", 0.9993, 0.01),
    ("camera.vac", 1.0000, 0.95),
    ("color_shift.vac", 0.9998, 0.14),
    ("crossing.vac", 0.9999, 0.00),
    ("diagonal.vac", 0.9995, 0.03),
    ("fade.vac", 1.0000, 0.01),
    ("grow.vac", 0.9769, 13.11),
    ("late_entry.vac", 1.0000, 0.00),
    ("occlusion.vac", 0.9999, 0.01),
    ("outlined.vac", 1.0000, 1.08),
    ("panel.vac", 0.9987, 0.01),
    ("pop_in.vac", 0.9997, 0.01),
    ("round_trip_two.vac", 0.9941, 2.05),
    ("shaded.vac", 0.9999, 0.47),
    ("sliding_rect.vac", 1.0000, 0.00),
    ("spin.vac", 0.9932, 1.12),
    ("sprite.vac", 0.9746, 13.39),
    ("static_and_moving.vac", 0.9999, 0.01),
    ("textured.vac", 0.9999, 1.51),
    ("triangle.vac", 0.9855, 7.18),
    ("two_balls.vac", 0.9997, 0.02),
    ("two_tone.vac", 1.0000, 0.26),
];

fn baseline_for(sample: &str) -> (f64, f64) {
    BASELINE
        .iter()
        .find(|(n, _, _)| *n == sample)
        .map(|(_, iou, res)| (*iou, *res))
        .unwrap_or_else(|| {
            panic!("{sample} has no recorded hermetic baseline — add one to BASELINE")
        })
}

fn assert_fidelity(sample: &str, f: &Fidelity) {
    let (base_iou, base_residual) = baseline_for(sample);
    assert!(
        f.ink_iou >= base_iou - IOU_TOL,
        "{sample}: ink IoU regressed {base_iou:.4} → {:.4} (tolerance {IOU_TOL})\n{f}",
        f.ink_iou
    );
    // Residual is scale-free noise-prone at very small values, so allow a
    // relative band plus a small absolute floor.
    let ceiling = base_residual * 1.25 + 1.0;
    assert!(
        f.residual <= ceiling,
        "{sample}: residual regressed {base_residual:.2} → {:.2} (ceiling {ceiling:.2})\n{f}",
        f.residual
    );
    // Scoring better than recorded is good news, but it means the record
    // is stale and every comparison against it is understated.
    assert!(
        f.ink_iou <= base_iou + 0.05,
        "{sample}: ink IoU improved {base_iou:.4} → {:.4} — update BASELINE so the \
         floor keeps its bite",
        f.ink_iou
    );
}

/// Every fixture in `examples/` has a recorded floor, and meets it.
///
/// The tests below check *structure* for the fixtures they were written
/// for. This is what makes a `BASELINE` row mean anything: through v1.7,
/// twelve of its twenty rows were recorded and never asserted, so those
/// fixtures could regress without a test noticing.
#[test]
fn every_fixture_meets_its_recorded_floor() {
    let mut names: Vec<String> = std::fs::read_dir(examples_dir())
        .expect("examples/ is readable")
        .filter_map(|e| {
            let name = e.ok()?.file_name().to_string_lossy().into_owned();
            name.ends_with(".vac").then_some(name)
        })
        .collect();
    names.sort();
    for (row, _, _) in BASELINE {
        assert!(
            names.iter().any(|n| n == row),
            "BASELINE records {row}, which is no longer a fixture"
        );
    }
    for name in &names {
        let rt = hermetic(name);
        assert_fidelity(name, &rt.fidelity);
    }
}

// ====================================================================
// Per-fixture ground truth
// ====================================================================

#[test]
fn ball_recovers_its_trajectory_not_just_its_apex() {
    let rt = hermetic("ball.vac");
    assert_fidelity("ball.vac", &rt.fidelity);

    let got = shapes(&rt.recovered);
    assert_eq!(got.len(), 1, "expected one recovered object, got {got:?}");
    assert!(
        matches!(got[0].1, Shape::Ellipse { .. }),
        "ball should stay an ellipse, got {:?}",
        got[0].1
    );
    assert_eq!(animate_count(&rt.recovered), 1);
}

#[test]
fn two_balls_stay_two_objects() {
    let rt = hermetic("two_balls.vac");
    assert_fidelity("two_balls.vac", &rt.fidelity);
    assert_eq!(shapes(&rt.recovered).len(), 2);
}

#[test]
fn crossing_keeps_identities() {
    let rt = hermetic("crossing.vac");
    assert_fidelity("crossing.vac", &rt.fidelity);
    assert_eq!(
        shapes(&rt.recovered).len(),
        2,
        "same-coloured crossing must not spawn phantom tracks"
    );
}

#[test]
fn sliding_rect_stays_a_rect() {
    let rt = hermetic("sliding_rect.vac");
    assert_fidelity("sliding_rect.vac", &rt.fidelity);
    let got = shapes(&rt.recovered);
    assert_eq!(got.len(), 1);
    assert!(
        matches!(got[0].1, Shape::Rect { .. }),
        "got {:?}",
        got[0].1
    );
}

#[test]
fn static_and_moving_separates_the_two() {
    let rt = hermetic("static_and_moving.vac");
    assert_fidelity("static_and_moving.vac", &rt.fidelity);
    assert_eq!(
        animate_count(&rt.recovered),
        1,
        "only the ball moves; the card must not get an animate block"
    );
}

#[test]
fn occlusion_keeps_the_ball_whole_behind_a_static_post() {
    let rt = hermetic("occlusion.vac");
    assert_fidelity("occlusion.vac", &rt.fidelity);
    assert_eq!(shapes(&rt.recovered).len(), 2);
    assert_eq!(animate_count(&rt.recovered), 1);
}

#[test]
fn diagonal_recovers_two_axis_motion() {
    let rt = hermetic("diagonal.vac");
    assert_fidelity("diagonal.vac", &rt.fidelity);
    assert_eq!(shapes(&rt.recovered).len(), 1);
}

#[test]
fn triangle_round_trips_as_a_path() {
    let rt = hermetic("triangle.vac");
    assert_fidelity("triangle.vac", &rt.fidelity);
    let got = shapes(&rt.recovered);
    assert_eq!(got.len(), 1);
    assert!(matches!(got[0].1, Shape::Path(_)), "got {:?}", got[0].1);
}

// ====================================================================
// Recovery error against known geometry
// ====================================================================

#[test]
fn recovered_geometry_lands_within_a_few_pixels_of_the_source() {
    // sliding_rect is the cleanest geometric ground truth: one rect, one
    // axis, known size. Assert the *error*, not a count.
    let rt = hermetic("sliding_rect.vac");
    let src = shapes(&rt.source);
    let got = shapes(&rt.recovered);
    assert_eq!(src.len(), 1);
    assert_eq!(got.len(), 1);

    if let (Shape::Rect { w: sw, h: sh, .. }, Shape::Rect { w: gw, h: gh, .. }) =
        (src[0].1, got[0].1)
    {
        assert!(
            (sw - gw).abs() <= 3.0 && (sh - gh).abs() <= 3.0,
            "size drift: source {sw}x{sh}, recovered {gw}x{gh}"
        );
    } else {
        panic!("expected rects on both sides");
    }

    let (sx, sy) = anchor(src[0].1);
    let (gx, gy) = anchor(got[0].1);
    assert!(
        (sx - gx).abs() <= 4.0 && (sy - gy).abs() <= 4.0,
        "anchor drift: source ({sx},{sy}), recovered ({gx},{gy})"
    );
}

#[test]
fn canvas_survives_exactly() {
    // No codec in the loop, so there is no excuse for fps or dimension
    // drift here — unlike the GIF path, where centisecond quantisation
    // turns 30fps into 33fps.
    for sample in ["ball.vac", "two_balls.vac", "sliding_rect.vac"] {
        let rt = hermetic(sample);
        assert_eq!(
            rt.source.canvas.width, rt.recovered.canvas.width,
            "{sample} width"
        );
        assert_eq!(
            rt.source.canvas.height, rt.recovered.canvas.height,
            "{sample} height"
        );
        assert_eq!(
            rt.source.canvas.fps, rt.recovered.canvas.fps,
            "{sample} fps"
        );
    }
}

// ====================================================================
// The layering A/B, as a fidelity number
// ====================================================================

#[test]
fn static_layering_does_not_hurt_fidelity_on_the_occlusion_fixture() {
    // v0.8 justified static-scene layering with shape and keyframe
    // counts. This is the first time the claim is checked against
    // reconstruction quality on either side of the switch.
    let with = hermetic_with("occlusion.vac", &DecompileOptions { static_layer: true, ..Default::default() });
    let without = hermetic_with(
        "occlusion.vac",
        &DecompileOptions {
            static_layer: false,
            ..Default::default()
        },
    );

    assert!(
        with.fidelity.ink_iou >= without.fidelity.ink_iou - 0.02,
        "layering cost ink IoU: {:.4} with vs {:.4} without",
        with.fidelity.ink_iou,
        without.fidelity.ink_iou
    );
}

/// Not an assertion — a report. `cargo test -p vac --test hermetic -- \
/// --nocapture fidelity_report` prints the hermetic score of every
/// fixture, which is how the floors above were calibrated and how you
/// check there is still margin under them.
#[test]
fn fidelity_report() {
    let mut names: Vec<String> = std::fs::read_dir(examples_dir())
        .unwrap()
        .filter_map(|e| {
            let n = e.ok()?.file_name().to_string_lossy().into_owned();
            n.ends_with(".vac").then_some(n)
        })
        .collect();
    names.sort();

    println!(
        "\n{:<24} {:>8} {:>8} {:>7} {:>7} {:>8} {:>8} {:>6}",
        "fixture", "residual", "psnr dB", "ink IoU", "worst", "missing", "spurious", "shapes"
    );
    println!("{}", "-".repeat(84));
    for name in &names {
        let rt = hermetic(name);
        let f = &rt.fidelity;
        println!(
            "{:<24} {:>8.2} {:>8.2} {:>7.4} {:>7.4} {:>7.1}% {:>7.1}% {:>6}",
            name,
            f.residual,
            f.psnr_db,
            f.ink_iou,
            f.worst_iou,
            f.missing_ink * 100.0,
            f.spurious_ink * 100.0,
            shapes(&rt.recovered).len()
        );
    }
    println!();
}
