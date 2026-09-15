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

/// **FIXED in v0.10 — objects that appear after t=0 were time-shifted.**
///
/// Through v0.9, `emit_shape` re-based keyframe times to the track's
/// first sighting, so the card in `examples/late_entry.vac` (off-canvas
/// until 1000 ms, parked at 2000 ms) was recovered parking at 840 ms:
/// ink IoU 0.394, worst frame 0.000. Every structural count was correct,
/// which is why no count-based test ever saw it.
///
/// Keyframes are now absolute, and a track clipped by the canvas edge at
/// its first sighting is held just off-canvas until then. This is now a
/// floor, not a defect guard.
#[test]
fn late_entering_objects_keep_their_timing() {
    let (source, recovered, fidelity) = hermetic("late_entry.vac");
    println!("{}", vac::to_string(&recovered));
    println!("{fidelity}");

    let src = first_keyframe_times(&source);
    let got = first_keyframe_times(&recovered);
    assert_eq!(got.len(), 1, "recovered should have one animate block");

    // Measured 0.9318 after the fix (0.394 before).
    assert!(
        fidelity.ink_iou >= 0.92,
        "late-entry timing regressed: ink IoU {:.4}",
        fidelity.ink_iou
    );
    // The recovered trajectory must end within a frame of the source's
    // last sighting, not a second early.
    assert!(
        got[0].2 + 100 >= src[0].2 - 1000,
        "recovered trajectory ends at {}ms, source at {}ms",
        got[0].2,
        src[0].2
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
    for sample in [
        "ball.vac",
        "two_balls.vac",
        "occlusion.vac",
        "late_entry.vac",
    ] {
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
