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

/// **FIXED in v0.13 — touching regions of different colour.**
///
/// Through v0.12 a blob was a connected component of "not background":
/// colour-blind. `two_tone.vac`'s four touching bars fused into one blob
/// and its two-tone card into one shape of their averaged colour, and the
/// background was a per-channel median of four corners that landed on a
/// bar.
///
/// v0.13 adds colour-aware regions and a border-mode background, and the
/// facade keeps them only when they measurably reconstruct better. This
/// test pins both halves: the measured choice is right, *and* it matters
/// — forcing the old pipeline is an order of magnitude worse.
#[test]
fn touching_colours_are_separated_by_measured_choice() {
    let source = vac::read(examples_dir().join("two_tone.vac")).unwrap();
    let frames = vac::rendered_to_video_frames(&vac::render(&source).unwrap(), source.canvas.fps);

    let auto = vac::decompile_frames(frames.clone(), &DecompileOptions::default()).unwrap();
    let auto_fid = vac::measure(&frames, &auto).unwrap();
    let legacy_opts = DecompileOptions {
        segmentation: vac::Segmentation::Components,
        background: vac::Background::Corners,
        ..Default::default()
    };
    let legacy = vac::decompile_frames(frames.clone(), &legacy_opts).unwrap();
    let legacy_fid = vac::measure(&frames, &legacy).unwrap();
    println!("auto:   {auto_fid}\nlegacy: {legacy_fid}");

    assert_eq!(shapes_of(&auto), shapes_of(&source), "{}", vac::to_string(&auto));
    assert!(auto_fid.residual < 150.0, "auto residual {:.1}", auto_fid.residual);
    assert!(auto_fid.ink_iou > 0.98, "auto ink IoU {:.4}", auto_fid.ink_iou);
    assert!(
        legacy_fid.residual > 10.0 * auto_fid.residual,
        "the legacy pipeline should be far worse here: {:.1} vs {:.1}",
        legacy_fid.residual,
        auto_fid.residual
    );
}

/// **FIXED in v0.14 — rotation.**
///
/// Through v0.13 VAC could translate, scale and fade a shape but not turn
/// it. `spin.vac`'s swinging rod and spinning bar came back as eight
/// fragments frozen at fixed angles (ink IoU 0.353). Each recovered
/// keyframe's angle is now checked against the source's own rotation
/// curve. A rect's long axis is what is measurable, and the decompiler
/// declares rects long side horizontal, so angles are compared as
/// principal-axis directions modulo 180°.
#[test]
fn rotation_is_recovered_along_the_source_curve() {
    let (source, recovered, fid) = hermetic("spin.vac");
    println!("{}\n{fid}", vac::to_string(&recovered));
    assert_eq!(shapes_of(&recovered), 2, "one rod and one bar");
    assert!(fid.ink_iou >= 0.90, "spin.vac ink IoU {:.4}", fid.ink_iou);

    // (name, long axis offset, rotation curve) for each animated rect.
    let curves = |d: &Document| -> Vec<(f32, Vec<(u32, f32)>)> {
        let mut out = Vec::new();
        for st in d.scenes.iter().flat_map(|s| &s.statements) {
            if let Statement::Animate(a) = st {
                let offset = d
                    .scenes
                    .iter()
                    .flat_map(|s| &s.statements)
                    .find_map(|s| match s {
                        Statement::Let { name, shape: vac::Shape::Rect { w, h, .. } } if *name == a.target => {
                            Some(if w >= h { 0.0 } else { 90.0 })
                        }
                        _ => None,
                    })
                    .unwrap_or(0.0);
                let rot: Vec<(u32, f32)> = a
                    .keyframes
                    .iter()
                    .filter_map(|k| {
                        k.transforms.iter().find_map(|t| match t {
                            Transform::Rotate(r) => Some((k.time_ms, *r)),
                            _ => None,
                        })
                    })
                    .collect();
                if !rot.is_empty() {
                    out.push((offset, rot));
                }
            }
        }
        out
    };
    let at = |c: &[(u32, f32)], t: u32| -> f32 {
        let i = c.iter().position(|&(kt, _)| kt >= t).unwrap_or(c.len() - 1);
        if i == 0 || c[i].0 == t || c[i].0 < t {
            return c[i].1;
        }
        let ((t0, v0), (t1, v1)) = (c[i - 1], c[i]);
        v0 + (v1 - v0) * (t - t0) as f32 / (t1 - t0) as f32
    };
    let axis_diff = |a: f32, b: f32| {
        let d = (a - b).rem_euclid(180.0);
        d.min(180.0 - d)
    };
    let src = curves(&source);
    let got = curves(&recovered);
    assert_eq!(got.len(), 2, "both shapes should carry rotation");
    for (g_off, g) in &got {
        // Match each recovered curve to the source curve it tracks best.
        let err = src
            .iter()
            .map(|(s_off, s)| {
                g.iter()
                    .map(|&(t, r)| axis_diff(r + g_off, at(s, t) + s_off))
                    .fold(0.0f32, f32::max)
            })
            .fold(f32::INFINITY, f32::min);
        assert!(err <= 4.0, "recovered rotation strays {err:.1}° from the source: {g:?}");
    }
}

/// **FIXED in v1.1 — ground: regions a few shades off the background.**
///
/// A region whose colour differs from the background by less than the
/// foreground threshold was painted the background colour. On
/// `real_double_pendulum` that was its whole plot area (a near-white panel
/// in an off-white margin, 13 levels apart over 420 000 pixels); here it
/// is `panel.vac`'s panel, 21 units off.
#[test]
fn a_panel_near_the_background_is_recovered() {
    let (_, recovered, fid) = hermetic("panel.vac");
    println!("{}\n{fid}", vac::to_string(&recovered));
    let ground = recovered
        .scenes
        .iter()
        .flat_map(|s| &s.statements)
        .find_map(|st| match st {
            Statement::Let { name, shape: vac::Shape::Rect { x, y, w, h } } if name.starts_with("ground_") => {
                Some((*x, *y, *w, *h))
            }
            _ => None,
        })
        .expect("the panel should come back as a ground rect");
    let near = |a: f32, b: f32| (a - b).abs() <= 1.0;
    assert!(
        near(ground.0, 40.0) && near(ground.1, 40.0) && near(ground.2, 240.0) && near(ground.3, 180.0),
        "ground rect {ground:?}, source (40, 40, 240, 180)"
    );
    assert!(fid.residual < 2.0, "panel.vac residual {:.2}", fid.residual);
}

/// **FIXED in v1.2 — shaded fills.** Through v1.1 every fill was one flat
/// colour, so `shaded.vac`'s lit ball and fading panel came back as their
/// means. A gradient is now fitted to each shape's interior and kept only
/// when it describes the region (removes ≥60% of the flat error, leaves
/// ≤25 levels RMS). The recovered stops are checked against the source's.
#[test]
fn shaded_fills_are_recovered_as_gradients() {
    let (_, recovered, fid) = hermetic("shaded.vac");
    println!("{}\n{fid}", vac::to_string(&recovered));
    let fills: Vec<vac::Gradient> = recovered
        .scenes
        .iter()
        .flat_map(|s| &s.statements)
        .filter_map(|st| match st {
            Statement::Assign { property: vac::Property::Fill, value: vac::Value::Gradient(g), .. } => Some(*g),
            _ => None,
        })
        .collect();
    let close = |a: vac::Color, b: (u8, u8, u8)| {
        (a.r as i32 - b.0 as i32).abs() <= 8 && (a.g as i32 - b.1 as i32).abs() <= 8 && (a.b as i32 - b.2 as i32).abs() <= 8
    };
    let linear = fills.iter().find_map(|g| match g {
        vac::Gradient::Linear { c0, p0, c1, p1 } => Some((*c0, *p0, *c1, *p1)),
        _ => None,
    });
    let (c0, p0, c1, p1) = linear.expect("the panel should come back as a linear gradient");
    assert!(close(c0, (0x20, 0x3a, 0x6a)) && close(c1, (0x7f, 0xb0, 0xf0)), "{c0:?} → {c1:?}");
    assert!((p0.x - 40.0).abs() <= 3.0 && (p1.x - 440.0).abs() <= 3.0, "{p0:?} → {p1:?}");
    let radial = fills.iter().find_map(|g| match g {
        vac::Gradient::Radial { c0, center, .. } => Some((*c0, *center)),
        _ => None,
    });
    let (rc0, centre) = radial.expect("the ball should come back as a radial gradient");
    assert!(close(rc0, (0xff, 0xf2, 0xc0)), "{rc0:?}");
    // The ball is declared at its first position, (90, 230); the
    // highlight sits 14 px up and left of its centre.
    assert!((centre.x - 76.0).abs() <= 3.0 && (centre.y - 216.0).abs() <= 3.0, "{centre:?}");
    assert!(fid.residual < 40.0, "shaded.vac residual {:.2}", fid.residual);
}

/// **FIXED in v1.3 — colour animation.** VAC could move, scale, turn and
/// fade a shape but not recolour it, so `color_shift.vac`'s ball (red →
/// blue → green) and pulsing square came back as one average colour each
/// (residual 101.7). Each recovered `fill()` keyframe is checked against
/// the source's own colour curve at that time.
#[test]
fn colour_animation_is_recovered_along_the_source_curve() {
    let (source, recovered, fid) = hermetic("color_shift.vac");
    println!("{}\n{fid}", vac::to_string(&recovered));
    fn curves(d: &Document) -> Vec<Vec<(u32, vac::Color)>> {
        d.scenes
            .iter()
            .flat_map(|s| &s.statements)
            .filter_map(|st| match st {
                Statement::Animate(a) => Some(
                    a.keyframes
                        .iter()
                        .filter_map(|k| {
                            k.transforms.iter().find_map(|t| match t {
                                Transform::Fill(c) => Some((k.time_ms, *c)),
                                _ => None,
                            })
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .filter(|c: &Vec<(u32, vac::Color)>| !c.is_empty())
            .collect()
    }
    fn at(c: &[(u32, vac::Color)], t: u32) -> [f32; 3] {
        let rgb = |c: vac::Color| [c.r as f32, c.g as f32, c.b as f32];
        let i = c.iter().position(|&(kt, _)| kt >= t).unwrap_or(c.len() - 1);
        if i == 0 || c[i].0 <= t {
            return rgb(c[i].1);
        }
        let ((t0, a), (t1, b)) = (c[i - 1], c[i]);
        let p = (t - t0) as f32 / (t1 - t0) as f32;
        let (a, b) = (rgb(a), rgb(b));
        [a[0] + (b[0] - a[0]) * p, a[1] + (b[1] - a[1]) * p, a[2] + (b[2] - a[2]) * p]
    }
    let src = curves(&source);
    let got = curves(&recovered);
    assert_eq!(got.len(), 2, "both shapes should carry a colour channel");
    for g in &got {
        let err = src
            .iter()
            .map(|s| {
                g.iter()
                    .map(|&(t, c)| {
                        let w = at(s, t);
                        ((c.r as f32 - w[0]).powi(2) + (c.g as f32 - w[1]).powi(2) + (c.b as f32 - w[2]).powi(2)).sqrt()
                    })
                    .fold(0.0f32, f32::max)
            })
            .fold(f32::INFINITY, f32::min);
        assert!(err <= 12.0, "recovered colours stray {err:.1} from the source curve: {g:?}");
    }
    assert!(fid.residual < 5.0, "color_shift.vac residual {:.2}", fid.residual);
}

/// **FIXED in v1.6 — scenery no vector primitive can describe.**
///
/// `textured.vac`'s background is a smooth, detailed texture. Vectors
/// approximate it with one flat rect (15.3 dB). The measured search now
/// also tries a raster plate — the median frame as one JPEG `image()` —
/// and keeps it when it reconstructs at least twice as well, with the
/// moving shapes still written as code on top. Flat scenery must never
/// take a plate.
#[test]
fn textured_scenery_takes_a_raster_plate_and_flat_scenery_does_not() {
    let images = |d: &Document| {
        d.scenes
            .iter()
            .flat_map(|s| &s.statements)
            .filter(|st| matches!(st, Statement::Let { shape: vac::Shape::Image { .. }, .. }))
            .count()
    };
    let (_, recovered, fid) = hermetic("textured.vac");
    assert_eq!(images(&recovered), 1, "textured.vac should take one raster plate");
    let vector_shapes: Vec<&vac::Shape> = recovered
        .scenes
        .iter()
        .flat_map(|s| &s.statements)
        .filter_map(|st| match st {
            Statement::Let { shape, .. } if !matches!(shape, vac::Shape::Image { .. }) => Some(shape),
            _ => None,
        })
        .collect();
    assert_eq!(vector_shapes.len(), 1, "only the ball should remain as code: {vector_shapes:?}");
    assert!(matches!(vector_shapes[0], vac::Shape::Ellipse { rx, .. } if (rx - 26.0).abs() <= 1.0));
    assert!(fid.psnr_db > 40.0, "textured.vac PSNR {:.2}", fid.psnr_db);
    for flat in ["two_balls.vac", "two_tone.vac", "panel.vac", "shaded.vac", "outlined.vac"] {
        let (_, recovered, _) = hermetic(flat);
        assert_eq!(images(&recovered), 0, "{flat}: flat scenery must stay vector");
    }
}

/// **FIXED in v1.4 — outlines come back as strokes.**
///
/// Through v1.3 the decompiler wrote `stroke = none` for everything: an
/// outlined box came back as a small filled ellipse, and a card's border
/// was lost. A blob enclosing a large hole whose filled outline is a
/// genuine rect or ellipse is now that primitive *stroked*, at its
/// centreline, with its width solved from its coverage-weighted area; and
/// an outline that frames a fill moving with it becomes one shape with
/// both.
#[test]
fn outlines_are_recovered_as_strokes() {
    let (_, recovered, fid) = hermetic("outlined.vac");
    println!("{}\n{fid}", vac::to_string(&recovered));
    // (fill, stroke, stroke_width, shape) per declared shape.
    let mut shapes: Vec<(Option<vac::Value>, Option<vac::Value>, Option<f32>, vac::Shape)> = Vec::new();
    let stmts: Vec<&Statement> = recovered.scenes.iter().flat_map(|s| &s.statements).collect();
    for st in &stmts {
        if let Statement::Let { name, shape } = st {
            if name == "bg" {
                continue;
            }
            let prop = |p: vac::Property| {
                stmts.iter().find_map(|s| match s {
                    Statement::Assign { target, property, value } if target == name && *property == p => {
                        Some(value.clone())
                    }
                    _ => None,
                })
            };
            let width = match prop(vac::Property::StrokeWidth) {
                Some(vac::Value::Number(n)) => Some(n),
                _ => None,
            };
            shapes.push((prop(vac::Property::Fill), prop(vac::Property::Stroke), width, shape.clone()));
        }
    }
    assert_eq!(shapes.len(), 3, "frame, ring, and the card fused with its border");
    let colour = |v: &Option<vac::Value>| match v {
        Some(vac::Value::Color(c)) => Some((c.r, c.g, c.b)),
        _ => None,
    };
    let near = |a: u8, b: u8| (a as i32 - b as i32).abs() <= 6;
    let rgb_near = |c: Option<(u8, u8, u8)>, t: (u8, u8, u8)| c.is_some_and(|c| near(c.0, t.0) && near(c.1, t.1) && near(c.2, t.2));
    // The moving frame: an unfilled 160x90 rect stroked 4 px #50d0ff.
    let frame = shapes.iter().find(|s| rgb_near(colour(&s.1), (0x50, 0xd0, 0xff))).expect("the frame");
    assert!(matches!(frame.0, Some(vac::Value::None)), "frame should be unfilled");
    assert!((frame.2.unwrap() - 4.0).abs() <= 0.5, "frame stroke width {:?}", frame.2);
    if let vac::Shape::Rect { w, h, .. } = frame.3 {
        assert!((w - 160.0).abs() <= 1.0 && (h - 90.0).abs() <= 1.0, "frame {w}x{h}");
    } else {
        panic!("frame should be a rect: {:?}", frame.3);
    }
    // The ring: an unfilled r=50 ellipse stroked 3 px.
    let ring = shapes.iter().find(|s| rgb_near(colour(&s.1), (0xf0, 0xc0, 0x40))).expect("the ring");
    assert!((ring.2.unwrap() - 3.0).abs() <= 0.5, "ring stroke width {:?}", ring.2);
    assert!(matches!(ring.3, vac::Shape::Ellipse { rx, ry, .. } if (rx - 50.0).abs() <= 1.0 && (ry - 50.0).abs() <= 1.0));
    // The card and its border: ONE shape, filled #e04848, stroked #f8f8f8.
    let card = shapes.iter().find(|s| rgb_near(colour(&s.1), (0xf8, 0xf8, 0xf8))).expect("the bordered card");
    assert!(rgb_near(colour(&card.0), (0xe0, 0x48, 0x48)), "card fill {:?}", card.0);
    assert!((card.2.unwrap() - 4.0).abs() <= 0.5, "border width {:?}", card.2);
    assert!(fid.ink_iou > 0.97, "outlined.vac ink IoU {:.4}", fid.ink_iou);
}
