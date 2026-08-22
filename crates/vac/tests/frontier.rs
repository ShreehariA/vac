//! Tier 5 — the frontier: fixtures that VAC does *not* handle correctly.
//!
//! These tests pass. That is not the same as saying the behaviour is
//! right — each one asserts against a **measured defect**, so the suite
//! stays green while the failure stays visible and cannot quietly get
//! worse.
//!
//! The alternative — deleting the fixture, or asserting the wrong
//! behaviour as though it were correct — is how a project ends up
//! believing its own proxy metrics. Every number here was measured on
//! 2026-08-22 and is reproduced in `docs/frontier.md`.
//!
//! When one of these is fixed, the assertion flips from "no worse than
//! the defect" to a real floor, and the row leaves `docs/frontier.md`.

use std::path::{Path, PathBuf};

use vac::{DecompileOptions, Document, Statement, Transform};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

fn hermetic(sample: &str) -> (Document, Document, vac::Fidelity) {
    let source = vac::read(examples_dir().join(sample)).expect("read fixture");
    let rendered = vac::render(&source).expect("render");
    let frames = vac::rendered_to_video_frames(&rendered, source.canvas.fps);
    let recovered =
        vac::decompile_frames(frames.clone(), &DecompileOptions::default()).expect("decompile");
    let fidelity = vac::measure(&frames, &recovered).expect("measure");
    (source, recovered, fidelity)
}

fn first_keyframe_times(doc: &Document) -> Vec<(String, u32, u32)> {
    doc.scenes
        .iter()
        .flat_map(|s| &s.statements)
        .filter_map(|st| match st {
            Statement::Animate(a) if !a.keyframes.is_empty() => Some((
                a.target.clone(),
                a.keyframes.first().unwrap().time_ms,
                a.keyframes.last().unwrap().time_ms,
            )),
            _ => None,
        })
        .collect()
}

/// **DEFECT — objects that appear after t=0 have their whole trajectory
/// shifted earlier.**
///
/// `emit_shape` normalises keyframe times to the track's first sighting
/// (`t_norm = t.saturating_sub(first_t)`) and the emitted `animate`
/// block carries no start offset, so a track that begins at 1200 ms
/// emits its first keyframe at 0 ms. On re-render the object is already
/// moving while the source frame is still empty.
///
/// Measured on `examples/late_entry.vac`, whose card holds off-canvas
/// until 1000 ms and parks at x=150 at 2000 ms. The recovered source
/// parks it at **840 ms** — over a second early — and scores ink IoU
/// 0.394 with a worst frame of 0.000.
///
/// The reason this survived eight versions: **the recovered AST looks
/// completely reasonable.** Two shapes, one animate block, four
/// keyframes, correct geometry, correct colour. Every count matches. No
/// count-based assertion can see it; only pixel comparison can.
#[test]
fn late_entering_objects_are_time_shifted() {
    let (source, recovered, fidelity) = hermetic("late_entry.vac");

    let src_times = first_keyframe_times(&source);
    let got_times = first_keyframe_times(&recovered);
    assert_eq!(src_times.len(), 1, "fixture should have one animate block");
    assert_eq!(got_times.len(), 1, "recovered should have one animate block");

    // Every structural count is correct. This is the point.
    let count_shapes = |d: &Document| {
        d.scenes
            .iter()
            .flat_map(|s| &s.statements)
            .filter(|st| matches!(st, Statement::Let { name, .. } if name != "bg"))
            .count()
    };
    assert_eq!(count_shapes(&source), count_shapes(&recovered));

    // And yet the picture is wrong.
    assert!(
        fidelity.ink_iou < 0.60,
        "late-entry defect appears fixed (ink IoU {:.4}) — if so, tighten this \
         assertion into a real floor and drop the row from docs/frontier.md",
        fidelity.ink_iou
    );
    assert_eq!(
        fidelity.worst_iou_frame, 0,
        "the failure should be worst at frame 0, where the source is empty \
         and the reconstruction has already painted the card"
    );

    // The recovered animation ends far earlier than the source's.
    let src_end = src_times[0].2;
    let got_end = got_times[0].2;
    assert!(
        got_end + 500 < src_end,
        "expected the recovered trajectory to end early (defect); \
         source ends {src_end}ms, recovered ends {got_end}ms"
    );
}

/// **DEFECT — `scale()` and `opacity()` are never recovered.**
///
/// The language has supported both since v0.1 and `vac-compiler`
/// renders both, but the decompiler only ever emits
/// `Transform::Position`. Across all 26 `.vac` files in this repository
/// there are 1654 `position(` and zero of either other kind.
///
/// So a shape that grows, shrinks, or fades round-trips as a
/// constant-size, fully-opaque shape. Real footage is full of both.
#[test]
fn scale_and_opacity_are_never_emitted() {
    for sample in ["ball.vac", "two_balls.vac", "occlusion.vac", "late_entry.vac"] {
        let (_, recovered, _) = hermetic(sample);
        for st in recovered.scenes.iter().flat_map(|s| &s.statements) {
            if let Statement::Animate(a) = st {
                for kf in &a.keyframes {
                    for t in &kf.transforms {
                        assert!(
                            matches!(t, Transform::Position(_, _)),
                            "{sample}: the decompiler emitted {t:?} — if scale/opacity \
                             recovery has landed, remove this frontier test.\n\
                             NOTE: `apply_animation` in vac-compiler consults only the two \
                             bracketing keyframes and pulls each transform kind \
                             independently, so a kind absent from both reverts to base \
                             instead of holding. Fix that before emitting a second kind."
                        );
                    }
                }
            }
        }
    }
}

/// **DEFECT — strokes are never recovered.**
///
/// `emit_shape` writes `stroke = none` unconditionally, and the renderer
/// hard-codes stroke width at 2.0 with no way to express it in source.
/// A stroked shape comes back as a bare fill.
#[test]
fn strokes_are_never_recovered() {
    for sample in ["ball.vac", "triangle.vac"] {
        let (_, recovered, _) = hermetic(sample);
        let strokes: Vec<_> = recovered
            .scenes
            .iter()
            .flat_map(|s| &s.statements)
            .filter_map(|st| match st {
                Statement::Assign {
                    property: vac::Property::Stroke,
                    value,
                    ..
                } => Some(value.clone()),
                _ => None,
            })
            .collect();
        assert!(
            !strokes.is_empty(),
            "{sample}: expected stroke assignments to exist at all"
        );
        assert!(
            strokes.iter().all(|v| matches!(v, vac::Value::None)),
            "{sample}: a stroke was recovered — remove this frontier test"
        );
    }
}
