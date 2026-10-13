//! Tier 2 — properties that must hold for *any* input.
//!
//! Unlike the fixture tests, nothing here asserts a specific number. These
//! are invariants: if one breaks, the pipeline is wrong in a way that no
//! amount of threshold-tuning fixes.
//!
//! Kept deliberately small and fast — one or two representative fixtures
//! each — because their value is in what they assert, not in coverage
//! breadth.

use std::path::{Path, PathBuf};

use vac::{DecompileOptions, Document, Shape, Statement, Transform};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

fn frames_of(sample: &str) -> (Document, Vec<vac::VideoFrame>) {
    let doc = vac::read(examples_dir().join(sample)).expect("read fixture");
    let rendered = vac::render(&doc).expect("render");
    let frames = vac::rendered_to_video_frames(&rendered, doc.canvas.fps);
    (doc, frames)
}

// ====================================================================
// Determinism
// ====================================================================

/// v0.8's headline claim was "the decompiler is now reproducible run to
/// run" — it fixed `detect_blobs` ordering its output by `HashMap`
/// iteration. That claim has never actually been asserted anywhere.
/// Without it, every baseline in this repo measures noise.
#[test]
fn decompiling_the_same_frames_twice_gives_byte_identical_source() {
    for sample in ["two_balls.vac", "occlusion.vac"] {
        let (_, frames) = frames_of(sample);
        let opts = DecompileOptions::default();

        let first = vac::to_string(&vac::decompile_frames(frames.clone(), &opts).unwrap());
        for run in 1..4 {
            let again = vac::to_string(&vac::decompile_frames(frames.clone(), &opts).unwrap());
            assert_eq!(
                first, again,
                "{sample}: run {run} differs from run 0 — the decompiler is not reproducible"
            );
        }
    }
}

#[test]
fn rendering_the_same_document_twice_gives_identical_pixels() {
    // The metric compares rendered pixels, so a non-deterministic
    // rasteriser would make every fidelity number noise.
    let doc = vac::read(examples_dir().join("two_balls.vac")).unwrap();
    let a = vac::render(&doc).unwrap();
    let b = vac::render(&doc).unwrap();
    assert_eq!(a.len(), b.len());
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(x.rgba, y.rgba, "frame {i} differs between renders");
    }
}

// ====================================================================
// Format stability
// ====================================================================

/// The documented invariant is `parse(write(d)) == d`, but that is not
/// quite true and never was: `fmt_num` rounds to 4 decimal places, so an
/// AST holding arbitrary floats cannot survive exactly. The honest
/// property is *idempotence after one normalisation pass* — writing is
/// stable once the values have been through it.
#[test]
fn writing_is_idempotent_after_one_normalisation_pass() {
    for sample in ["triangle.vac", "occlusion.vac", "two_balls.vac"] {
        let (doc, frames) = frames_of(sample);
        let recovered = vac::decompile_frames(frames, &DecompileOptions::default()).unwrap();

        for (label, d) in [("source", &doc), ("recovered", &recovered)] {
            let once = vac::to_string(d);
            let reparsed = vac::parse(&once).unwrap_or_else(|e| {
                panic!("{sample} ({label}): emitted source does not parse: {e}\n{once}")
            });
            let twice = vac::to_string(&reparsed);
            assert_eq!(
                once, twice,
                "{sample} ({label}): write→parse→write is not stable"
            );
        }
    }
}

/// Whatever the decompiler emits must be readable by the parser. This is
/// the one thing that would make the output worthless, and emitted paths
/// carry long fractional coordinate lists that no hand-written fixture
/// exercises.
#[test]
fn everything_the_decompiler_emits_parses_back() {
    for sample in ["triangle.vac", "crossing.vac", "static_and_moving.vac"] {
        let (_, frames) = frames_of(sample);
        let recovered = vac::decompile_frames(frames, &DecompileOptions::default()).unwrap();
        let text = vac::to_string(&recovered);
        vac::parse(&text)
            .unwrap_or_else(|e| panic!("{sample}: decompiler emitted unparseable source: {e}"));
    }
}

// ====================================================================
// Well-formedness of emitted documents
// ====================================================================

fn assert_well_formed(sample: &str, doc: &Document) {
    let canvas = &doc.canvas;
    assert!(canvas.width > 0 && canvas.height > 0, "{sample}: empty canvas");
    assert!(canvas.fps >= 1, "{sample}: fps must be at least 1");

    for scene in &doc.scenes {
        let mut declared: Vec<&str> = Vec::new();

        for st in &scene.statements {
            match st {
                Statement::Let { name, shape } => {
                    assert!(
                        !declared.contains(&name.as_str()),
                        "{sample}: `{name}` declared twice — the renderer would draw it twice"
                    );
                    declared.push(name);
                    assert_shape_finite(sample, name, shape);
                }
                Statement::Assign { target, .. } => {
                    assert!(
                        declared.contains(&target.as_str()),
                        "{sample}: assignment to undeclared `{target}` — this is a hard \
                         render error, not a warning"
                    );
                }
                Statement::Animate(a) => {
                    assert!(
                        declared.contains(&a.target.as_str()),
                        "{sample}: animate targets undeclared `{}`",
                        a.target
                    );
                    let mut last = None::<u32>;
                    for kf in &a.keyframes {
                        if let Some(prev) = last {
                            assert!(
                                kf.time_ms > prev,
                                "{sample}: keyframe times must strictly increase, got \
                                 {prev} then {}",
                                kf.time_ms
                            );
                        }
                        last = Some(kf.time_ms);
                        for t in &kf.transforms {
                            match t {
                                Transform::Position(x, y) => {
                                    assert!(
                                        x.is_finite() && y.is_finite(),
                                        "{sample}: non-finite position"
                                    );
                                }
                                Transform::Scale(sx, sy) => {
                                    assert!(sx.is_finite() && sy.is_finite(), "{sample}: non-finite scale")
                                }
                                Transform::Rotate(d) => {
                                    assert!(d.is_finite(), "{sample}: non-finite rotation")
                                }
                                Transform::Fill(_) => {}
                                Transform::Clip(x, y, w, h) => assert!(
                                    [x, y, w, h].iter().all(|v| v.is_finite()) && *w >= 0.0 && *h >= 0.0,
                                    "{sample}: bad clip window"
                                ),
                                Transform::Opacity(o) => assert!(
                                    o.is_finite() && (0.0..=1.0).contains(o),
                                    "{sample}: opacity {o} outside [0,1]"
                                ),
                            }
                        }
                    }
                }
            }
        }
    }
}

fn assert_shape_finite(sample: &str, name: &str, shape: &Shape) {
    match shape {
        Shape::Rect { x, y, w, h } | Shape::Image { x, y, w, h, .. } => {
            assert!(
                x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite(),
                "{sample}/{name}: non-finite rect"
            );
            assert!(*w > 0.0 && *h > 0.0, "{sample}/{name}: rect has no area");
        }
        Shape::Ellipse { cx, cy, rx, ry } => {
            assert!(
                cx.is_finite() && cy.is_finite() && rx.is_finite() && ry.is_finite(),
                "{sample}/{name}: non-finite ellipse"
            );
            assert!(*rx > 0.0 && *ry > 0.0, "{sample}/{name}: ellipse has no area");
        }
        Shape::Path(segs) => {
            assert!(
                segs.len() >= 2,
                "{sample}/{name}: path with {} segments cannot be drawn",
                segs.len()
            );
            for seg in segs {
                let pts: Vec<vac::Point> = match seg {
                    vac::PathSeg::MoveTo(p) | vac::PathSeg::LineTo(p) => vec![*p],
                    vac::PathSeg::CurveTo { c1, c2, p } => vec![*c1, *c2, *p],
                };
                for p in pts {
                    assert!(
                        p.x.is_finite() && p.y.is_finite(),
                        "{sample}/{name}: non-finite path point"
                    );
                }
            }
        }
    }
}

#[test]
fn decompiled_documents_are_well_formed() {
    for sample in [
        "ball.vac",
        "two_balls.vac",
        "triangle.vac",
        "occlusion.vac",
        "static_and_moving.vac",
    ] {
        let (_, frames) = frames_of(sample);
        let recovered = vac::decompile_frames(frames, &DecompileOptions::default()).unwrap();
        assert_well_formed(sample, &recovered);
    }
}

// ====================================================================
// Ink honesty
// ====================================================================

/// The decompiler must never paint substantially more ink than it
/// measured. `PATH_MAX_FILL_OVERCLAIM` exists to protect exactly this,
/// but it is enforced at a single call site and has never been asserted
/// as a property of the output.
#[test]
fn the_decompiler_does_not_invent_ink() {
    for sample in ["ball.vac", "triangle.vac", "occlusion.vac", "sliding_rect.vac"] {
        let (_, frames) = frames_of(sample);
        let recovered = vac::decompile_frames(frames.clone(), &DecompileOptions::default()).unwrap();
        let fid = vac::measure(&frames, &recovered).unwrap();
        assert!(
            fid.spurious_ink < 0.25,
            "{sample}: painted {:.1}% more ink than the source had\n{fid}",
            fid.spurious_ink * 100.0
        );
    }
}

// ====================================================================
// Generational drift
// ====================================================================

/// Feed the pipeline its own output, repeatedly. A stable compiler
/// reaches a fixed point; an unstable one drifts a little further every
/// generation until the result is unrecognisable.
///
/// **Through v1.0 this drifted.** Each round trip grew `two_balls`'
/// ellipses by a pixel of radius (22 → 27 over five generations) and
/// `sliding_rect`'s width by two (120 → 130): a shape moving at sub-pixel
/// speed straddles a pixel on both edges in most frames, the bbox of
/// thresholded pixels reads it a pixel wide, and the median rounded up.
/// v1.1 sizes axis-aligned primitives from coverage — each pixel counts by
/// how much of it the shape covers — and the loop now reaches a fixed
/// point at generation one, at the source's exact size.
#[test]
fn repeated_round_trips_reach_a_fixed_point() {
    fn dims(d: &Document) -> Vec<(f32, f32)> {
        d.scenes
            .iter()
            .flat_map(|s| &s.statements)
            .filter_map(|st| match st {
                Statement::Let { name, shape } if name != "bg" => Some(match shape {
                    Shape::Rect { w, h, .. } | Shape::Image { w, h, .. } => (*w, *h),
                    Shape::Ellipse { rx, ry, .. } => (*rx, *ry),
                    Shape::Path(p) => (p.len() as f32, 0.0),
                }),
                _ => None,
            })
            .collect()
    }
    for (sample, truth) in [("two_balls.vac", (22.0, 22.0)), ("sliding_rect.vac", (120.0, 60.0))] {
        let (doc, gen1_frames) = frames_of(sample);
        let opts = DecompileOptions::default();
        let mut current = doc;
        let mut seen: Vec<(Vec<(f32, f32)>, f64)> = Vec::new();
        for _ in 1..=4 {
            let rendered = vac::render(&current).unwrap();
            let frames = vac::rendered_to_video_frames(&rendered, current.canvas.fps);
            current = vac::decompile_frames(frames, &opts).unwrap();
            // Always score against generation 1 — the original truth — so
            // slow drift cannot hide behind small steps.
            let fid = vac::measure(&gen1_frames, &current).unwrap();
            seen.push((dims(&current), fid.ink_iou));
        }
        println!("{sample}: {seen:?}");
        for (g, (d, _)) in seen.iter().enumerate() {
            assert!(
                d.iter().all(|&s| s == truth),
                "{sample} generation {}: sizes {d:?}, source {truth:?}",
                g + 1
            );
        }
        let (first, last) = (seen[0].1, seen[3].1);
        assert!(
            last >= first - 0.005,
            "{sample}: fidelity drifts across generations ({first:.4} → {last:.4})"
        );
    }
}
