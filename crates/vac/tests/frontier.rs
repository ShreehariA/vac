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

/// **FIXED in v1.20 — a slow mover is not scenery.**
///
/// The static layer (v0.8) is a per-pixel temporal median, and to a median
/// anything covering a pixel for more than half a shot is scenery. An
/// outlined card sliding at 100 px/s covers the middle of its own top and
/// bottom edges for 85% of a 1.5 s shot. Those edges were stripped from
/// every frame and written as two static bars, and what was left of the
/// card fragmented: the first shot of `cuts_three_shots`, which is
/// `outlined_ui` cut to 45 frames, came back at 24 dB against 33.9 for the
/// whole clip. The static layer is now measured against its absence: when
/// the winning candidate has static shapes, its segmentation and background
/// are decompiled once more without the layer, and the better is kept. Here
/// the card must come back as one rect that slides, at better than 40 dB.
/// With the layer assumed it came back as two static bars and eight
/// fragments of its sides; measured against its absence, as the card and
/// the square, exactly.
#[test]
fn a_slow_mover_is_not_scenery() {
    let source = vac::parse(
        "canvas 480x320 @30fps

scene main {
  duration 1500ms

  let bg = rect(0, 0, 480, 320)
  bg.fill = #1a182e

  let square = rect(211, 131, 58, 58)
  square.fill = none
  square.stroke = #f1bd3f
  square.stroke_width = 2

  let card = rect(22, 32, 136, 76)
  card.fill = none
  card.stroke = #3fbffe
  card.stroke_width = 4

  animate card {
    0ms -> position(22, 32)
    1500ms -> position(172, 32)
    easing: linear
  }
}
",
    )
    .expect("parse");
    let rendered = vac::render(&source).expect("render");
    let frames = vac::rendered_to_video_frames(&rendered, source.canvas.fps);
    let recovered = vac::decompile_frames(frames.clone(), &DecompileOptions::default()).expect("decompile");
    let fid = vac::measure(&frames, &recovered).expect("measure");
    let stmts: Vec<&Statement> = recovered.scenes.iter().flat_map(|s| &s.statements).collect();
    let slides = stmts.iter().any(|st| match st {
        Statement::Animate(a) => {
            let xs: Vec<f32> = a
                .keyframes
                .iter()
                .filter_map(|k| k.transforms.iter().find_map(|t| match t {
                    Transform::Position(x, _) => Some(*x),
                    _ => None,
                }))
                .collect();
            xs.len() >= 2 && xs.last().unwrap() - xs.first().unwrap() > 100.0
        }
        _ => false,
    });
    assert!(slides, "the card comes back sliding: {}", vac::to_string(&recovered));
    assert!(fid.psnr_db > 40.0, "slow card: {:.2} dB", fid.psnr_db);
}

/// **FIXED in v1.19 — a mosaic fitted through the renderer.**
///
/// A camera plate's mosaic was the per-pixel median of every frame warped
/// back: each sample blurred once by the warp in and again by the
/// renderer's reading out. On fine texture that blur was most of the error
/// (the Ken Burns clip's worst frame: 40 dB at a 4×4 average, 29 dB at full
/// resolution). The mosaic is now also fitted to the frames through the
/// renderer's own bilinear reading of it, by back-projection, and kept when
/// it pays for its bytes. Here a fine procedural texture zoomed 1.0 → 1.4
/// must come back as a camera at better than 33.5 dB: fitted, 34.5; the
/// median mosaic alone, 32.0.
#[test]
fn a_detailed_zoom_comes_back_sharp() {
    let (w, h) = (240u32, 160u32);
    let mut texture = image::RgbImage::new(w, h);
    for (x, y, p) in texture.enumerate_pixels_mut() {
        let (x, y) = (x as f32, y as f32);
        let r = 128.0 + 70.0 * (x * 0.7).sin() * (y * 0.5).cos() + 30.0 * (x * 0.13 + y * 0.21).sin();
        let g = 128.0 + 70.0 * (x * 0.45 + y * 0.8).sin() + 30.0 * (y * 0.09).cos();
        let b = 128.0 + 70.0 * ((x - y) * 0.6).cos() + 30.0 * (x * 0.05).sin();
        *p = image::Rgb([r as u8, g as u8, b as u8]);
    }
    let mut png = Vec::new();
    texture.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).expect("encode texture");
    let src = format!(
        "canvas 240x160 @30fps\n\nscene main {{\n  duration 800ms\n\n  let photo = image(0, 0, 240, 160, \"data:image/png;base64,{}\")\n\n  animate photo {{\n    0ms -> position(0, 0), scale(1)\n    800ms -> position(-48, -32), scale(1.4)\n    easing: linear\n  }}\n}}\n",
        vac_format::base64::encode(&png)
    );
    let source = vac::parse(&src).expect("parse the zoom");
    let rendered = vac::render(&source).expect("render");
    let frames = vac::rendered_to_video_frames(&rendered, source.canvas.fps);
    let recovered = vac::decompile_frames(frames.clone(), &DecompileOptions::default()).expect("decompile");
    let fid = vac::measure(&frames, &recovered).expect("measure");
    let zoomed = recovered.scenes.iter().flat_map(|s| &s.statements).any(|st| {
        matches!(st, Statement::Animate(a) if a.keyframes.iter().any(|k| k.transforms.iter().any(|t| matches!(t, Transform::Scale(s) if *s > 1.3))))
    });
    assert!(zoomed, "the zoom comes back as a camera");
    assert!(fid.psnr_db > 33.5, "detailed zoom: {:.2} dB", fid.psnr_db);
}

/// **FIXED in v1.18 — a still frame is a scene, not nothing.**
///
/// Every track needs three frames and the static layer needed eight, so a
/// clip of a frame or two had nothing to describe it with: a single frame
/// of ffmpeg's test card came back as a lone background at 3.9 dB. In a
/// clip that short nothing can be a mover, and its frames are the scene, so
/// it is layered like any other; the test card frame now comes back at
/// 37.8 dB. Here `outlined.vac`'s first frame alone must come back as its
/// shapes.
#[test]
fn a_single_frame_comes_back_as_its_shapes() {
    let mut source = vac::read(examples_dir().join("outlined.vac")).expect("read fixture");
    // One frame at 30 fps.
    source.scenes[0].duration_ms = 34;
    let rendered = vac::render(&source).expect("render");
    assert_eq!(rendered.len(), 1);
    let frames = vac::rendered_to_video_frames(&rendered, source.canvas.fps);
    let recovered = vac::decompile_frames(frames.clone(), &DecompileOptions::default()).expect("decompile");
    let fid = vac::measure(&frames, &recovered).expect("measure");
    let shapes = recovered
        .scenes
        .iter()
        .flat_map(|s| &s.statements)
        .filter(|st| matches!(st, Statement::Let { name, .. } if name != "bg"))
        .count();
    assert!(shapes >= 3, "the frame's shapes, not a lone background: {shapes}");
    assert!(fid.psnr_db > 35.0, "one frame of outlined.vac: {:.2} dB", fid.psnr_db);
}

/// **FIXED in v1.17 — dissolves.**
///
/// A dissolve is a cut spread over frames: no single step looks like a cut,
/// so through v1.16 a two-photo slideshow with a one-second dissolve came
/// back as one shot, one camera trying to follow two photographs, at 19.1 dB
/// and 371 shapes. Frames a dissolve apart now differ like a cut while every
/// frame between them is a blend `(1 − β)·before + β·after` with `β`
/// rising; the dissolve becomes a scene of its own between the two shots —
/// the frame before it and the frame after it as pictures, the second's
/// opacity fitted to every frame by least squares. The slideshow: three
/// scenes at 29.1 dB.
///
/// `dissolve.vac` holds one picture, dissolves into another over a second,
/// and holds that: three scenes, the middle one dissolving with an opacity
/// that rises from 0 to 1, and no shot on either side mistaken for a fade.
#[test]
fn a_dissolve_comes_back_as_a_scene_between_its_two_shots() {
    let (_, recovered, fid) = hermetic("dissolve.vac");
    assert_eq!(recovered.scenes.len(), 3, "a picture, the dissolve, the next picture");
    let middle = &recovered.scenes[1];
    let rising = middle.statements.iter().find_map(|st| match st {
        Statement::Animate(a) if a.target == "incoming" => Some(a),
        _ => None,
    });
    let opacity = |k: &vac::Keyframe| {
        k.transforms.iter().find_map(|t| match t {
            Transform::Opacity(o) => Some(*o),
            _ => None,
        })
    };
    let rising = rising.expect("the dissolve's incoming picture fades in");
    let (first, last) = (rising.keyframes.first().unwrap(), rising.keyframes.last().unwrap());
    assert!(opacity(first).is_some_and(|o| o < 0.1) && opacity(last).is_some_and(|o| o > 0.9), "0 -> 1: {rising:?}");
    assert!((900..=1100).contains(&middle.duration_ms), "the dissolve lasts its second: {} ms", middle.duration_ms);
    for scene in &recovered.scenes {
        assert!(
            !scene.statements.iter().any(|st| matches!(st, Statement::Let { name, .. } if name == "fade")),
            "{}: a dissolve is not a fade",
            scene.name
        );
    }
    assert!(fid.psnr_db > 44.0, "dissolve.vac PSNR {:.2}", fid.psnr_db);
}

/// **FIXED in v1.15 — a camera that rolls.**
///
/// The camera of v1.10 could zoom and pan but not turn: a photo rolled 26°
/// over three seconds (`roll_watercolor.mp4`) fell back to a still plate
/// and 588 shapes at 18.5 dB. A view is now `scale · R(angle) · q + offset`,
/// tracked with turns about the frame centre alongside zooms, and written
/// as the plate's own `rotate()` keyframes (the renderer turns an image
/// about its box's centre, so position follows the angle). The clip: one
/// camera plate at 33.3 dB.
///
/// `camera_roll.vac` turns a detailed scene 12° about the frame centre while
/// a ball crosses it: the turn must come back as `rotate()` keyframes on
/// track with the source, the ball as the shape it is; a camera that never
/// rolls (`camera.vac`) must not be written with `rotate()`.
#[test]
fn a_rolling_camera_comes_back_as_rotate_keyframes() {
    let (_, recovered, fid) = hermetic("camera_roll.vac");
    let stmts: Vec<&Statement> = recovered.scenes.iter().flat_map(|s| &s.statements).collect();
    let camera = stmts
        .iter()
        .find_map(|st| match st {
            Statement::Animate(a) if a.target == "plate" => Some(a),
            _ => None,
        })
        .expect("the plate is keyframed: the camera");
    for k in &camera.keyframes {
        let turn = k.transforms.iter().find_map(|t| match t {
            Transform::Rotate(d) => Some(*d),
            _ => None,
        });
        let expected = 12.0 * k.time_ms as f32 / 2000.0;
        assert!(
            turn.is_some_and(|d| (d - expected).abs() < 0.5),
            "at {} ms the camera has turned {turn:?}, the source {expected:.2}",
            k.time_ms
        );
    }
    let ball = stmts.iter().find_map(|st| match st {
        Statement::Let { shape: vac::Shape::Ellipse { rx, .. }, .. } => Some(*rx),
        _ => None,
    });
    assert!(ball.is_some_and(|r| (r - 20.0).abs() <= 0.5), "the ball, on top: {ball:?}");
    assert!(fid.psnr_db > 44.0, "camera_roll.vac PSNR {:.2}", fid.psnr_db);

    let (_, zoom, _) = hermetic("camera.vac");
    let rolls = zoom.scenes.iter().flat_map(|s| &s.statements).any(|st| {
        matches!(st, Statement::Animate(a) if a.target == "plate"
            && a.keyframes.iter().flat_map(|k| &k.transforms).any(|t| matches!(t, Transform::Rotate(_))))
    });
    assert!(!rolls, "camera.vac zooms without rolling");
}

/// **PARTLY FIXED in v1.14 — HD frames are searched small.**
///
/// Every candidate of the measured search decompiles and measures the whole
/// clip, and at 720p each held its own copy: 3 s of a camera pan took ~64 s
/// and 2.4 GB. Above 500k pixels the search now runs on the clip shrunk by
/// a whole factor and only its decision is replayed at full size; camera
/// tracking stops at a working size of ~700 px; the mosaic samples frames
/// directly instead of float copies of them. The same pan: 33 s, 1.7 GB,
/// and a better result (44.3 → 44.7 dB). Long HD clips are still open.
///
/// `hd_camera.vac` is the camera fixture at 960×540, over the threshold: the
/// decision made on the half-size copy must still be the camera, and the
/// full-size decompile must place it and the ball as exactly as the small
/// fixture does.
#[test]
fn hd_frames_are_decided_small_and_decompiled_full() {
    let (_, recovered, fid) = hermetic("hd_camera.vac");
    assert_eq!(recovered.canvas.width, 960, "the output is full size");
    let stmts: Vec<&Statement> = recovered.scenes.iter().flat_map(|s| &s.statements).collect();
    assert!(
        stmts.iter().any(|st| matches!(st, Statement::Animate(a) if a.target == "plate")),
        "the decision replayed at full size is the camera"
    );
    let ball = stmts.iter().find_map(|st| match st {
        Statement::Let { shape: vac::Shape::Ellipse { rx, .. }, .. } => Some(*rx),
        _ => None,
    });
    assert!(ball.is_some_and(|r| (r - 36.0).abs() <= 0.5), "the ball, at full size: {ball:?}");
    assert!(fid.psnr_db > 44.0, "hd_camera.vac PSNR {:.2}", fid.psnr_db);
}

/// **FIXED in v1.13 — fades.**
///
/// A fade blends the whole picture toward black or white, and through v1.12
/// it broke everything downstream of it: the sliding card faded out scored
/// 24.4 dB instead of 44.6, and a photo pan faded in lost its camera (frame
/// 0 was black) and came back as 115 shapes. The fade is now measured — a
/// mean reading and a chained per-pixel one, decided by measurement — taken
/// out of the frames, the picture decompiled (shots, camera and all), and
/// drawn back on top as a canvas-sized rect whose opacity is fitted, frame
/// by frame, to the reconstruction itself.
///
/// `fade_through.vac` fades one shot out to black and the next in from it.
/// Both shots must come back as scenes, each with its fade drawn on top and
/// its own content intact; a shape fading on its own (`fade.vac`) is not a
/// fade of the picture.
#[test]
fn a_fade_through_black_comes_back_as_two_faded_scenes() {
    let (_, recovered, fid) = hermetic("fade_through.vac");
    assert_eq!(recovered.scenes.len(), 2, "the fade hid a cut between two shots");
    for scene in &recovered.scenes {
        let names: Vec<&str> = scene
            .statements
            .iter()
            .filter_map(|st| match st {
                Statement::Let { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names.last(), Some(&"fade"), "{}: the fade is drawn on top of everything: {names:?}", scene.name);
        assert_eq!(names.len(), 3, "{}: background, its one shape, the fade: {names:?}", scene.name);
    }
    assert!(fid.psnr_db > 45.0, "fade_through.vac PSNR {:.2}", fid.psnr_db);
    let (_, one, _) = hermetic("fade.vac");
    assert!(
        !one.scenes.iter().flat_map(|s| &s.statements).any(|st| matches!(st, Statement::Let { name, .. } if name == "fade")),
        "fade.vac fades one shape, not the picture"
    );
}

/// **FIXED in v1.12 — shots.**
///
/// Real video is cut together from shots, and through v1.11 VAC decompiled
/// every clip as one: a static layer and tracks smeared across the cuts.
/// Three corpus clips spliced together (`cuts_three_shots.mp4`) came back
/// at 22.3 dB with a frame scoring ink IoU 0.000. Frames that differ from
/// the one before far more than the clip's frames usually do are now cut
/// candidates; each shot is decompiled on its own — every pass, its own
/// plate or camera — and the shots become scenes, kept when that measures
/// better than the clip as one shot.
///
/// `shots.vac` cuts between three unrelated pictures. Each must come back as
/// its own scene, as long as its shot, holding exactly its own shapes; a
/// clip with no cut must stay one scene.
#[test]
fn each_shot_comes_back_as_its_own_scene() {
    let (_, recovered, fid) = hermetic("shots.vac");
    assert_eq!(recovered.scenes.len(), 3, "three shots, three scenes");
    for (scene, want) in recovered.scenes.iter().zip([1usize, 1, 2]) {
        assert_eq!(scene.duration_ms, 1000, "{} lasts its shot", scene.name);
        let shapes = scene
            .statements
            .iter()
            .filter(|st| matches!(st, Statement::Let { name, .. } if name != "bg"))
            .count();
        assert_eq!(shapes, want, "{} holds exactly its own shapes", scene.name);
    }
    assert!(fid.psnr_db > 45.0, "shots.vac PSNR {:.2}", fid.psnr_db);
    for single in ["two_balls.vac", "spin.vac", "camera.vac"] {
        let (_, one, _) = hermetic(single);
        assert_eq!(one.scenes.len(), 1, "{single} has no cut and must stay one scene");
    }
}

/// **FIXED in v1.11 — motion, measured from the pixels.**
///
/// The tracker keyframes a shape from its blob's centroid and second
/// moments, estimates that drift when a blob is lit unevenly or crossed;
/// on `real_bouncing_ball` re-measuring the ball's motion from its own
/// pixels took the clip from 44.4 to 46.4 dB. Refinement (v1.9) moves a
/// whole shape; it cannot fix one keyframe. The motion pass registers each
/// mover's pixels against every frame and keyframes what it measures.
///
/// Mechanism: knock one keyframe of a perfect reconstruction off — a ball's
/// position in `two_balls.vac`, a swinging rod's turn and position in
/// `spin.vac` — and the motion pass must re-measure it from the pixels.
#[test]
fn a_keyframe_knocked_off_is_re_measured_from_the_shapes_own_pixels() {
    let run = |fixture: &str, target: &str, at_ms: u32, nudge: &dyn Fn(&mut Transform)| {
        let source = vac::read(examples_dir().join(fixture)).expect("read fixture");
        let frames = vac::rendered_to_video_frames(&vac::render(&source).expect("render"), source.canvas.fps);
        let mut off = source.clone();
        for st in &mut off.scenes[0].statements {
            if let Statement::Animate(a) = st {
                if a.target == target {
                    for k in a.keyframes.iter_mut().filter(|k| k.time_ms == at_ms) {
                        k.transforms.iter_mut().for_each(|t| nudge(t));
                    }
                }
            }
        }
        let before = vac::measure(&frames, &off).unwrap();
        let m = vac::motion(&frames, off, 50.0).expect("motion");
        assert!(m.moved.iter().any(|n| n == target), "{fixture}: `{target}` must be re-measured, moved {:?}", m.moved);
        assert!(
            m.fidelity.psnr_db > before.psnr_db + 6.0,
            "{fixture}: the knocked-off keyframe must come back: {:.2} -> {:.2} dB",
            before.psnr_db,
            m.fidelity.psnr_db
        );
        m.fidelity
    };
    let balls = run("two_balls.vac", "red", 1500, &|t| {
        if let Transform::Position(x, y) = t {
            *x += 6.0;
            *y -= 4.0;
        }
    });
    assert!(balls.psnr_db > 40.0, "two_balls.vac after the motion pass: {:.2} dB", balls.psnr_db);
    let rod = run("spin.vac", "rod", 1000, &|t| match t {
        Transform::Position(x, y) => {
            *x += 4.0;
            *y -= 3.0;
        }
        Transform::Rotate(d) => *d += 9.0,
        _ => {}
    });
    assert!(rod.psnr_db > 30.0, "spin.vac after the motion pass: {:.2} dB", rod.psnr_db);
}

/// **FIXED in v1.10 — a camera.**
///
/// A clip whose whole picture moves together is one image seen through a
/// moving camera. Through v1.9 VAC could not say so: the Mandelbrot zoom
/// came back as 33 flat shapes at 12 dB. The camera is now measured — each
/// frame's view of the scene, by a robust coarse-to-fine search, refined
/// against the median mosaic so movers in the first frame cannot bias it —
/// and written as `position()` and `scale()` keyframes on one plate.
///
/// `camera.vac` zooms a detailed scene 1.5x about (260, 150) while a ball
/// crosses it. The camera must come back as the straight zoom it is, and
/// the ball as the shape it is, on top.
#[test]
fn a_moving_camera_comes_back_as_keyframes_on_one_image() {
    let (_, recovered, fid) = hermetic("camera.vac");
    let stmts: Vec<&Statement> = recovered.scenes.iter().flat_map(|s| &s.statements).collect();
    assert!(
        stmts.iter().any(|st| matches!(st, Statement::Let { name, shape: vac::Shape::Image { .. } } if name == "plate")),
        "the scene becomes one plate"
    );
    let camera = stmts
        .iter()
        .find_map(|st| match st {
            Statement::Animate(a) if a.target == "plate" => Some(a),
            _ => None,
        })
        .expect("the plate is keyframed: the camera");
    assert!(camera.keyframes.len() <= 4, "a straight zoom needs few keyframes, got {}", camera.keyframes.len());
    let last = camera.keyframes.last().unwrap();
    let scale = last.transforms.iter().find_map(|t| match t {
        Transform::Scale(s) => Some(*s),
        _ => None,
    });
    // At the last frame (1966 ms) the source camera is at 1 + 0.5 * 1966 / 2000.
    let expected = 1.0 + 0.5 * last.time_ms as f32 / 2000.0;
    assert!(scale.is_some_and(|s| (s - expected).abs() < 0.005), "final zoom {scale:?}, expected {expected:.4}");
    let ball = stmts.iter().find_map(|st| match st {
        Statement::Let { shape: vac::Shape::Ellipse { rx, ry, .. }, .. } => Some((*rx, *ry)),
        _ => None,
    });
    assert!(
        ball.is_some_and(|(rx, ry)| (rx - 20.0).abs() <= 0.5 && (ry - 20.0).abs() <= 0.5),
        "the ball, on top: {ball:?}"
    );
    assert!(fid.psnr_db > 40.0, "camera.vac PSNR {:.2}", fid.psnr_db);

    // A still scene keeps a still plate.
    let (_, still, _) = hermetic("textured.vac");
    assert!(
        !still.scenes.iter().flat_map(|s| &s.statements).any(|st| matches!(st, Statement::Animate(a) if a.target == "plate")),
        "textured.vac's camera does not move"
    );
}

/// **FIXED in v1.9 — the fit is whatever reconstructs best.**
///
/// Through v1.8 every shape's geometry was estimated from its blob and
/// then final: `outlined.vac`'s 3 px ring came back with radius 49.8 for
/// 50 and gave up 5.7 dB. Refinement nudges each shape's size, position
/// and stroke width to wherever the reconstruction measures best.
///
/// Mechanism: `outlined.vac`'s own source is a perfect reconstruction of
/// its render. Knock three of its numbers off — the ring's radius, the
/// moving frame's keyframes, the card border's width — and refinement must
/// put each back to within a tenth of a pixel, from the pixels alone.
#[test]
fn refinement_moves_each_number_to_where_the_pixels_agree() {
    let source = vac::read(examples_dir().join("outlined.vac")).expect("read fixture");
    let frames = vac::rendered_to_video_frames(&vac::render(&source).expect("render"), source.canvas.fps);
    let mut off = source.clone();
    for st in &mut off.scenes[0].statements {
        match st {
            Statement::Let { name, shape: vac::Shape::Ellipse { rx, ry, .. } } if name == "ring" => {
                *rx -= 0.4;
                *ry -= 0.4;
            }
            Statement::Assign { target, property: vac::Property::StrokeWidth, value: vac::Value::Number(w) }
                if target == "border" =>
            {
                *w -= 0.6;
            }
            Statement::Animate(a) if a.target == "frame" => {
                for k in &mut a.keyframes {
                    for t in &mut k.transforms {
                        if let Transform::Position(x, y) = t {
                            *x += 0.7;
                            *y -= 0.3;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let before = vac::measure(&frames, &off).unwrap();
    let r = vac::refine(&frames, off).expect("refine");
    let mut moved = r.refined.clone();
    moved.sort();
    assert_eq!(moved, ["border", "frame", "ring"], "exactly the three knocked-off shapes move");
    let near = |a: f32, b: f32| (a - b).abs() <= 0.1;
    for st in &r.document.scenes[0].statements {
        match st {
            Statement::Let { name, shape: vac::Shape::Ellipse { rx, ry, .. } } if name == "ring" => {
                assert!(near(*rx, 50.0) && near(*ry, 50.0), "ring radius back to 50: {rx} x {ry}");
            }
            Statement::Assign { target, property: vac::Property::StrokeWidth, value: vac::Value::Number(w) }
                if target == "border" =>
            {
                assert!(near(*w, 4.0), "border width back to 4: {w}");
            }
            Statement::Animate(a) if a.target == "frame" => {
                let pos: Vec<(f32, f32)> = a
                    .keyframes
                    .iter()
                    .flat_map(|k| &k.transforms)
                    .filter_map(|t| match t {
                        Transform::Position(x, y) => Some((*x, *y)),
                        _ => None,
                    })
                    .collect();
                assert!(near(pos[0].0, 40.0) && near(pos[0].1, 40.0), "frame keyframes back to (40, 40): {pos:?}");
            }
            _ => {}
        }
    }
    assert!(
        r.fidelity.psnr_db > before.psnr_db + 10.0,
        "refinement must recover the reconstruction: {:.2} -> {:.2} dB",
        before.psnr_db,
        r.fidelity.psnr_db
    );

    // And the decompiler's own estimate of the ring, through a real
    // round trip, now lands on 50.
    let (_, recovered, fid) = hermetic("outlined.vac");
    let ring = recovered.scenes[0].statements.iter().find_map(|st| match st {
        Statement::Let { shape: vac::Shape::Ellipse { rx, .. }, .. } => Some(*rx),
        _ => None,
    });
    assert!(ring.is_some_and(|r| near(r, 50.0)), "recovered ring radius {ring:?}");
    assert!(fid.psnr_db > 46.0, "outlined.vac PSNR {:.2} (v1.8: 40.57)", fid.psnr_db);
}

/// **FIXED in v1.8 — a mover no flat fill describes keeps its own pixels.**
///
/// `sprite.vac` slides a smooth, many-coloured tile across a flat
/// background. Vectors find it exactly (a 72×72 rect, every keyframe right)
/// and can still only paint it one colour: 26.3 dB. The sprite pass
/// captures the tile's own pixels as a median over every frame it is on
/// screen, in a window that follows its keyframes, and keeps the sprite
/// because it reconstructs the tile's footprint far better than twice as
/// well. The motion stays code. Flat fixtures, whose fills describe their
/// interiors exactly, never take one (asserted with the raster plate's
/// test below).
#[test]
fn a_mover_no_flat_fill_describes_becomes_a_sprite_of_its_own_pixels() {
    let (_, recovered, fid) = hermetic("sprite.vac");
    let lets: Vec<(&String, &vac::Shape)> = recovered
        .scenes
        .iter()
        .flat_map(|s| &s.statements)
        .filter_map(|st| match st {
            Statement::Let { name, shape } => Some((name, shape)),
            _ => None,
        })
        .collect();
    let sprites: Vec<&&String> = lets
        .iter()
        .filter(|(_, sh)| matches!(sh, vac::Shape::Image { .. }))
        .map(|(n, _)| n)
        .collect();
    assert_eq!(sprites.len(), 1, "exactly the tile becomes a sprite: {lets:?}");
    assert_ne!(sprites[0].as_str(), "plate", "a sprite, not a raster plate");
    let animated = recovered
        .scenes
        .iter()
        .flat_map(|s| &s.statements)
        .any(|st| matches!(st, Statement::Animate(a) if &a.target == *sprites[0]));
    assert!(animated, "the sprite keeps the tile's keyframes");
    assert!(fid.psnr_db > 34.0, "sprite.vac PSNR {:.2} (one flat rect: 26.3)", fid.psnr_db);

    let frames = {
        let source = vac::read(examples_dir().join("sprite.vac")).unwrap();
        vac::rendered_to_video_frames(&vac::render(&source).unwrap(), source.canvas.fps)
    };
    let flat = vac::decompile_frames(frames.clone(), &DecompileOptions { sprites: false, ..Default::default() })
        .expect("decompile without sprites");
    let flat_fid = vac::measure(&frames, &flat).unwrap();
    assert!(
        fid.psnr_db > flat_fid.psnr_db + 6.0,
        "the sprite must beat the flat fill by a wide margin: {:.2} vs {:.2} dB",
        fid.psnr_db,
        flat_fid.psnr_db
    );
}

/// **FIXED in v1.7 — every shape earns its place.**
///
/// Through v1.6 a shape stayed in the output once its track cleared the
/// tracker's gates; nothing checked that drawing it helped. On the corpus,
/// 35 of the double pendulum's 454 shapes and 46 of `testsrc2`'s 110 made
/// the reconstruction *worse*. The audit takes each shape out in turn and
/// keeps it only if the clip reconstructs better with it.
///
/// Here the source document is its own perfect reconstruction, with two
/// shapes injected: a wrong one (a blob where the source has nothing) and
/// an exact duplicate of a real ball drawn over it (which changes no
/// pixel). Both must go, and nothing else.
#[test]
fn a_shape_the_reconstruction_is_no_better_with_is_taken_out() {
    let source = vac::read(examples_dir().join("two_balls.vac")).expect("read fixture");
    let frames = vac::rendered_to_video_frames(&vac::render(&source).expect("render"), source.canvas.fps);
    let mut doc = source.clone();
    let extra = vac::parse(
        "canvas 640x360 @30fps\nscene main {\n  duration 3s\n  \
         let stray = ellipse(320, 180, 15, 9)\n  stray.fill = #3a8f4c\n  stray.stroke = none\n  \
         let red_again = ellipse(60, 100, 22, 22)\n  red_again.fill = #e94560\n  red_again.stroke = none\n  \
         animate red_again {\n    0ms -> position(60, 100)\n    1500ms -> position(560, 100)\n    3s -> position(60, 100)\n    easing: linear\n  }\n}\n",
    )
    .expect("parse the injected shapes");
    doc.scenes[0].statements.extend(extra.scenes[0].statements.clone());

    let audited = vac::audit(&frames, doc).expect("audit");
    let mut removed = audited.removed.clone();
    removed.sort();
    assert_eq!(removed, ["red_again", "stray"], "exactly the two injected shapes must go");
    assert_eq!(audited.document, source, "everything else must be left as it was");
    assert!(audited.fidelity.residual == 0.0, "the audited document is the perfect reconstruction again");

    // And on a real decompile, nothing the audit keeps is worse than before.
    let (_, recovered, fid) = hermetic("two_balls.vac");
    let again = vac::audit(&vac::rendered_to_video_frames(&vac::render(&source).unwrap(), source.canvas.fps), recovered.clone())
        .expect("audit a decompile");
    assert!(again.removed.is_empty(), "an audited decompile has nothing left to take out: {:?}", again.removed);
    assert!(again.fidelity.residual <= fid.residual);
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
    for flat in ["two_balls.vac", "two_tone.vac", "panel.vac", "shaded.vac", "outlined.vac", "color_shift.vac", "spin.vac"] {
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
