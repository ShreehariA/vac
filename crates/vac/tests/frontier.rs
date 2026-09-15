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

/// **FIXED in v0.11 — `scale()` and `opacity()` are recovered.**
///
/// Through v0.10 the decompiler emitted only `position()`. `fade.vac`
/// came back fully opaque (residual 59.8) and `grow.vac` came back as ten
/// fragments (ink IoU 0.644), because the static layer mistook the
/// always-covered core of the growing disc for scenery.
///
/// The recovered channel values are checked against the source's own
/// curve, sampled at each recovered keyframe time — not merely for
/// existence, which a single spurious keyframe would satisfy.
#[test]
fn scale_and_opacity_are_recovered_along_the_source_curve() {
    fn channel(doc: &Document, pick: fn(&Transform) -> Option<f32>) -> Vec<(u32, f32)> {
        doc.scenes
            .iter()
            .flat_map(|s| &s.statements)
            .filter_map(|st| match st {
                Statement::Animate(a) => Some(a),
                _ => None,
            })
            .flat_map(|a| &a.keyframes)
            .filter_map(|k| k.transforms.iter().find_map(pick).map(|v| (k.time_ms, v)))
            .collect()
    }
    fn at(curve: &[(u32, f32)], t: u32) -> f32 {
        let i = curve.iter().position(|&(kt, _)| kt >= t).unwrap_or(curve.len() - 1);
        if i == 0 || curve[i].0 == t {
            return curve[i].1;
        }
        let ((t0, v0), (t1, v1)) = (curve[i - 1], curve[i]);
        v0 + (v1 - v0) * (t - t0) as f32 / (t1 - t0) as f32
    }
    let scale = |t: &Transform| match t {
        Transform::Scale(s) => Some(*s),
        _ => None,
    };
    let opacity = |t: &Transform| match t {
        Transform::Opacity(o) => Some(*o),
        _ => None,
    };

    // grow.vac: base radius differs between source (40) and recovered
    // (median size), so compare absolute radius = base · scale.
    let (source, recovered, fid) = hermetic("grow.vac");
    println!("{}\n{fid}", vac::to_string(&recovered));
    assert_eq!(shapes_of(&recovered), 1, "grow.vac should recover one shape");
    let base = |d: &Document| {
        d.scenes.iter().flat_map(|s| &s.statements).find_map(|st| match st {
            Statement::Let { name, shape: vac::Shape::Ellipse { rx, .. } } if name != "bg" => Some(*rx),
            _ => None,
        })
    };
    let (src_r, got_r) = (base(&source).unwrap(), base(&recovered).unwrap());
    let (src_c, got_c) = (channel(&source, scale), channel(&recovered, scale));
    assert!(got_c.len() >= 3, "expected a scale curve, got {got_c:?}");
    for &(t, s) in &got_c {
        let want = src_r * at(&src_c, t);
        let got = got_r * s;
        assert!((got - want).abs() <= 3.0, "radius at {t}ms: got {got:.1}, source {want:.1}");
    }
    assert!(fid.ink_iou >= 0.95, "grow.vac ink IoU {:.4}", fid.ink_iou);

    let (source, recovered, fid) = hermetic("fade.vac");
    let (src_c, got_c) = (channel(&source, opacity), channel(&recovered, opacity));
    assert!(got_c.len() >= 3, "expected an opacity curve, got {got_c:?}");
    for &(t, o) in &got_c {
        let want = at(&src_c, t);
        assert!((o - want).abs() <= 0.05, "opacity at {t}ms: got {o}, source {want}");
    }
    assert!(fid.residual < 5.0, "fade.vac residual {:.2}", fid.residual);
}

/// A shape that neither grows nor fades must not sprout either channel:
/// the round trip of every pre-v0.11 fixture stays position-only.
#[test]
fn constant_shapes_do_not_invent_scale_or_opacity() {
    for sample in ["ball.vac", "two_balls.vac", "crossing.vac", "occlusion.vac", "late_entry.vac", "sliding_rect.vac"] {
        let (_, recovered, _) = hermetic(sample);
        for st in recovered.scenes.iter().flat_map(|s| &s.statements) {
            if let Statement::Animate(a) = st {
                for kf in &a.keyframes {
                    for t in &kf.transforms {
                        assert!(
                            matches!(t, Transform::Position(_, _)),
                            "{sample}: invented {t:?} at {}ms",
                            kf.time_ms
                        );
                    }
                }
            }
        }
    }
}

fn shapes_of(doc: &Document) -> usize {
    doc.scenes
        .iter()
        .flat_map(|s| &s.statements)
        .filter(|st| matches!(st, Statement::Let { name, .. } if name != "bg"))
        .count()
}

/// **FIXED in v0.12 — objects that pop in or out mid-canvas.**
///
/// A track first seen late, or last seen early, without touching the
/// canvas edge was drawn for the whole clip. `pop_in.vac` scored ink IoU
/// 0.662 with 50.5% spurious ink. It is now hidden with `opacity(0)`
/// outside its lifetime.
///
/// The same mechanism is what moved the wild corpus: short fragment
/// tracks stopped persisting, and `real_pendulum.gif` went 0.19 → 0.38.
#[test]
fn objects_are_only_drawn_during_their_lifetime() {
    let (_, recovered, fid) = hermetic("pop_in.vac");
    println!("{}\n{fid}", vac::to_string(&recovered));
    assert!(fid.ink_iou >= 0.97, "pop_in.vac ink IoU {:.4}", fid.ink_iou);
    assert!(fid.spurious_ink <= 0.02, "pop_in.vac spurious {:.3}", fid.spurious_ink);
    // Frame 0: the dot is not visible yet, the tag is.
    // Frame 75 (2500 ms): the tag has gone.
    for (i, f) in [(0usize, &fid.frames[0]), (75, &fid.frames[75])] {
        assert!(f.ink_iou >= 0.9, "frame {i}: IoU {:.4}", f.ink_iou);
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
