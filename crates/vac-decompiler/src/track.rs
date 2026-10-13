//! Cross-frame tracking, keyframe extraction, easing classification,
//! AST emission.
//!
//! v0.3 promotes the pipeline from single-blob to multi-blob:
//!
//!   per-frame Vec<Blob>  →  build_tracks  →  Vec<Track>
//!                                              │
//!                                              ▼
//!                       per-track simplify + easing → animate
//!
//! The matching is a lowest-cost-first greedy bipartite assignment
//! between active tracks and the current frame's detections, with a
//! gate threshold that opens new tracks for unmatched blobs and a
//! short look-back tolerance that survives 1-3 frames of detection
//! noise.  Cost combines position, RGB colour, radius, and Hu
//! image-moment shape similarity — Hu is most useful when v0.5
//! introduces non-elliptical primitives, but it costs nothing to
//! compute now and adds a small bonus for ball-radius differences.
//!
//! v0.7 makes the position term velocity-aware: each track predicts
//! its next position by constant-velocity extrapolation from its last
//! two samples, and candidates are scored against that prediction.
//! Two same-coloured balls crossing paths keep their identities
//! because each ball's direction is part of the model, and fast
//! movers whose per-frame displacement exceeds the gate stay matched
//! because prediction error measures acceleration, not speed.
//!
//! v0.8 changes what reaches the tracker rather than the tracker
//! itself.  `layer::StaticLayer` measures the persistent scene first
//! and strips it out of every frame, so the blobs arriving here are
//! moving content only; the scenery is emitted ahead of the tracks as
//! `static_<n>`, using this module's own classifier and outline tracer
//! via one-sample tracks.
//!
//! Trajectory simplification and easing classification are
//! per-track and reuse the v0.2 implementation verbatim.

use vac_format::{
    Animation, Canvas, Color, Document, Easing, Keyframe, PathSeg, Property, Scene, Shape,
    Statement, Transform, Value,
};

use crate::detect::{Blob, Detection, Segmentation, ShapeKind};
use crate::frames::{estimate_fps, VideoFrame};
use crate::layer::StaticLayer;

/// 1D RDP tolerance, in pixels.  Below this, deviations from the
/// straight-line interpolation between segment endpoints are noise.
const RDP_TOLERANCE_PX: f32 = 2.0;

/// Every lossy simplification the decompiler makes, scaled together by one
/// quality dial (v1.0). Quality 50 reproduces the constants VAC used
/// through v0.14 exactly; each 25 points above halves every tolerance,
/// each 25 below doubles it. That single scalar is what turns the
/// decompiler into a codec with a rate–distortion curve: fewer keyframes
/// and coarser outlines cost fidelity and save source bytes, and
/// `vac verify` measures both.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerances {
    /// Keyframe simplification (temporal RDP), pixels.
    pub keyframe_px: f32,
    /// Outline polygon simplification (closed-loop RDP), pixels.
    pub outline_px: f32,
    /// Bézier fitting error, pixels.
    pub curve_px: f32,
    /// Opacity keyframe simplification, 0–255 levels.
    pub opacity_levels: f32,
    /// Smallest blob (pixels) emitted as a shape at all — the detector's
    /// noise floor, deliberately *not* scaled by quality (see below).
    pub min_blob_area: u32,
    /// Shape-level rate–distortion pruning (v1.0): below quality 50, a
    /// shape is dropped when the clip error it removes, per byte of source
    /// it costs, falls under this (per-channel MSE per byte). 0 keeps every
    /// shape, which is what quality ≥ 50 does.
    pub min_mse_per_byte: f64,
    /// JPEG quality of a raster plate (v1.6): 80 at quality 50, rising to
    /// 95 at 100 and falling to 50 at 0.
    pub raster_quality: u8,
}

impl Tolerances {
    /// Tolerances for a quality in `[0, 100]` (clamped).
    ///
    /// Each knob is scaled only in the direction it was *measured* to trade
    /// rate for distortion across the corpus (`docs/rate-distortion.md`):
    ///
    /// - keyframe and opacity tolerances scale both ways — keyframes are
    ///   where the bytes go, and finer keyframing buys real fidelity;
    /// - outline simplification and the minimum blob area only coarsen
    ///   below 50: finer outlines cost bytes *and* fidelity (an outline
    ///   traced at one frame overfits it), and blobs under 9 px are noise;
    /// - curve fitting is held at 0.75 px from 50 up, which measured
    ///   nearly free (+47 bytes over the whole corpus) and strictly better
    ///   than v0.14's 1.5.
    pub fn from_quality(quality: f32) -> Self {
        let m = 2f32.powf((50.0 - quality.clamp(0.0, 100.0)) / 25.0);
        let coarser = m.max(1.0);
        Tolerances {
            keyframe_px: RDP_TOLERANCE_PX * m,
            outline_px: PATH_RDP_EPSILON_PX * coarser,
            curve_px: PATH_CURVE_EPSILON_PX * coarser,
            opacity_levels: OPACITY_RDP_TOLERANCE * m,
            // The minimum blob area is the detector's noise floor, not a
            // fidelity knob: lowering it at high quality only admitted
            // dashed-line specks as dozens of extra fragment tracks, and
            // `real_pendulum` got *worse* (residual 657 → 756 at q75 with
            // 48 → 116 shapes). It may only rise, to save bytes.
            min_blob_area: crate::detect::MIN_BLOB_AREA,
            // Scaling the *area* floor was tried first and was a cliff, not
            // a trade: at quality 0 it rose to 144 px and deleted every tick
            // of the 32 px `real_loader` spinner (IoU 0.69 → 0.00). What a
            // shape is worth is the error it removes per byte it costs: a
            // loader tick is small but high-contrast (~2 MSE/byte), a
            // Newton's-cradle fragment is ~0.0002.
            min_mse_per_byte: MSE_PER_BYTE_SCALE * (m * m - 1.0).max(0.0) as f64,
            raster_quality: {
                let q = quality.clamp(0.0, 100.0);
                if q >= 50.0 {
                    (80.0 + (q - 50.0) * 0.3).round() as u8
                } else {
                    (50.0 + q * 0.6).round() as u8
                }
            },
        }
    }
}

impl Default for Tolerances {
    fn default() -> Self {
        Tolerances::from_quality(DEFAULT_QUALITY)
    }
}

/// The quality the CLI and the facade use unless told otherwise.
pub const DEFAULT_QUALITY: f32 = 50.0;

/// Pruning threshold scale: quality 25 drops shapes worth under 9e-4 MSE
/// per byte, quality 0 under 4.5e-3. Swept 1e-4 / 3e-4 / 1e-3 on the
/// corpus; 3e-4 is the strongest setting under which no clip's curve
/// turns pathological (at 1e-3 `real_pendulum` at quality 0 jumps from
/// residual 926 to 1113 while saving only 4 KB).
const MSE_PER_BYTE_SCALE: f64 = 3e-4;

/// Easing classifier only switches off `Linear` when an alternative
/// fits at least this much better (lower SSE).
const EASING_IMPROVEMENT_RATIO: f64 = 0.90;

// --- Tracking weights & gates ----------------------------------------
// Cost = W_POS·|pos − predicted| + W_COL·|Δrgb| + W_RAD·|Δr| + W_HU·|Δhu|,
// where `predicted` is the track's constant-velocity extrapolation
// (v0.7).  For smooth motion the position term measures acceleration
// rather than speed, so fast movers stay matched and same-colour
// crossings resolve by direction.  The weights are picked so that for
// two distinct-coloured balls a frame apart the same-track cost is ~10
// and the cross-track cost is > GATE.  Values within ±2× still match
// correctly in practice.
const W_POS: f32 = 1.0;
const W_COL: f32 = 0.4;
const W_RAD: f32 = 2.0;
const W_HU: f32 = 30.0;
const COST_GATE: f32 = 80.0;

/// Gate for tracks with a single sample (v0.7).  A 1-sample track has
/// no velocity model yet, so a fast mover whose per-frame displacement
/// exceeds `COST_GATE` could never acquire its second sample — every
/// frame would spawn a fresh singleton.  The wider bootstrap gate lets
/// it, after which prediction takes over and the gate tightens.  Wrong
/// bootstraps self-prune: a 2-sample track with a bogus velocity
/// predicts into empty space, starves, and falls under
/// `MIN_TRACK_SAMPLES`.
const BOOTSTRAP_COST_GATE: f32 = 160.0;

/// Maximum number of consecutive frames a track may go unseen
/// before it is closed out.  Sized (v0.7) to survive a full
/// merge-occlusion of two same-sized blobs crossing: while they
/// overlap, detection sees one blob and the losing bidder starves
/// for the whole merge window (~5 frames for ball-sized shapes at
/// crossing speeds).  Velocity prediction keeps stale re-acquisition
/// safe — a dead-object track predicts further along its exit path,
/// away from unrelated blobs.
const MAX_GAP_FRAMES: usize = 5;

/// A blob fewer than this many samples long is treated as a
/// noise track and dropped entirely from the output.
const MIN_TRACK_SAMPLES: usize = 3;

/// Stationarity gate (v0.5.1): a track whose every sample stays within
/// this distance (px) of its mean position is emitted as a static
/// shape with no `animate` block.  Deviation is measured from the mean
/// rather than the first sample so a noisy first frame can't disguise
/// a static shape as a mover.  The threshold sits just above the
/// 1-2 px of centroid jitter that GIF palette flips and H.264 chroma
/// subsampling inflict on genuinely static content.
const STATIONARY_MAX_DEV_PX: f32 = 2.5;

/// Closed-loop RDP tolerance for path outlines, in pixels (v0.6 —
/// same default as PAC's `--epsilon`).
const PATH_RDP_EPSILON_PX: f32 = 2.0;

/// Schneider Bezier fitting max-deviation tolerance, in pixels. 1.5 from
/// v0.6 (PAC's `--curve-eps` default) until v1.0's quality sweep measured
/// 0.75 as nearly free — +47 bytes over the whole corpus — and better on
/// every row it moved.
const PATH_CURVE_EPSILON_PX: f32 = 0.75;

/// Cap on how much more area a filled path outline may claim than the
/// blob's measured pixel count (v0.7).  Filling the outer contour is
/// right for compact shapes (ratio ≈ 1) and for ring-shaped masks like
/// a shaded ball with a near-background specular centre (ratio ≈ 2-4,
/// where filling the hole is exactly what we want) — but a sprawling
/// swept-trace web encloses huge empty bays (ratio 10-20×) and would
/// render as a giant solid slab.  Past this cap the track falls back
/// to an ellipse shrunk to the measured pixel area, so the render
/// never claims ink that was not measured.
const PATH_MAX_FILL_OVERCLAIM: f32 = 6.0;

#[derive(Debug, Clone)]
pub struct Track {
    pub id: usize,
    /// (t_ms, blob) ordered by time.
    pub samples: Vec<(u32, Blob)>,
    /// Source frame index of each sample, parallel to `samples`.
    /// v0.6's path emission re-visits the frame to trace the blob's
    /// outline.
    pub frame_indices: Vec<usize>,
    last_seen_frame: usize,
    active: bool,
}

/// A raster plate (v1.6): the scenery as one embedded image, and — under a
/// moving camera (v1.10) — the keyframes that move it.
#[derive(Debug, Clone, PartialEq)]
pub struct RasterPlate {
    /// The image, as a `data:` URI.
    pub uri: String,
    /// Its box `(x, y, w, h)`.
    pub rect: (f32, f32, f32, f32),
    /// The camera: `(time ms, position, scale, rotation in degrees)` per
    /// keyframe — the plate's own values, rotation about its box's centre.
    /// Empty for a still camera.
    pub camera: Vec<(u32, (f32, f32), f32, f32)>,
}

pub fn build_document(
    frames: &[VideoFrame],
    detections: &[Detection],
    layer: Option<&StaticLayer>,
    ground: Option<(&VideoFrame, Color)>,
    tol: &Tolerances,
    raster: Option<RasterPlate>,
) -> Document {
    let canvas = Canvas {
        width: frames[0].width,
        height: frames[0].height,
        fps: estimate_fps(frames),
    };

    let bg_color = median_color(detections.iter().map(|d| d.bg));

    let total_ms = detections
        .last()
        .map(|d| {
            d.t_ms
                .saturating_add(frames.last().map(|f| f.delay_ms).unwrap_or(0))
        })
        .unwrap_or(0);

    let mut statements = Vec::new();

    // v1.0 shape pruning: what one shape's statements cost in source
    // bytes, and whether the error it removes pays for them.
    let pixels_frames = (canvas.width as f64) * (canvas.height as f64) * frames.len().max(1) as f64;
    let empty_len = vac_format::write(&Document {
        canvas: canvas.clone(),
        scenes: vec![Scene {
            name: "main".into(),
            duration_ms: 0,
            statements: Vec::new(),
        }],
    })
    .len();
    let cost_bytes = |stmts: &[Statement]| -> f64 {
        let len = vac_format::write(&Document {
            canvas: canvas.clone(),
            scenes: vec![Scene {
                name: "main".into(),
                duration_ms: 0,
                statements: stmts.to_vec(),
            }],
        })
        .len();
        len.saturating_sub(empty_len).max(1) as f64
    };
    let worth_it = |stmts: &[Statement], removed_sq_err: f64| -> bool {
        tol.min_mse_per_byte <= 0.0
            || removed_sq_err / (3.0 * pixels_frames) >= tol.min_mse_per_byte * cost_bytes(stmts)
    };
    let contrast_sq = |c: Color, bg: Color| -> f64 {
        let d = |a: u8, b: u8| a as f64 - b as f64;
        d(c.r, bg.r).powi(2) + d(c.g, bg.g).powi(2) + d(c.b, bg.b).powi(2)
    };

    // v1.6: a raster plate stands in for the whole static scene — the
    // background, ground and scenery — and the moving shapes are drawn on
    // top of it as usual.
    let rastered = raster.is_some();
    if let Some(plate) = raster {
        let (x, y, w, h) = plate.rect;
        statements.push(Statement::Let {
            name: "plate".into(),
            shape: Shape::Image { x, y, w, h, src: plate.uri },
        });
        if !plate.camera.is_empty() {
            let round = |v: f32, p: f32| (v * p).round() / p;
            // A camera that never rolls is written without `rotate()`.
            let rolls = plate.camera.iter().any(|k| k.3.abs() >= 0.05);
            statements.push(Statement::Animate(Animation {
                target: "plate".into(),
                keyframes: plate
                    .camera
                    .iter()
                    .map(|&(t, (px, py), sc, deg)| {
                        let mut transforms = vec![
                            Transform::Position(round(px, 100.0), round(py, 100.0)),
                            Transform::Scale(round(sc, 10000.0), round(sc, 10000.0)),
                        ];
                        if rolls {
                            transforms.push(Transform::Rotate(round(deg, 100.0)));
                        }
                        Keyframe { time_ms: t, transforms }
                    })
                    .collect(),
                easing: Easing::Linear,
            }));
        }
    } else {
        statements.push(Statement::Let {
            name: "bg".into(),
            shape: Shape::Rect {
                x: 0.0,
                y: 0.0,
                w: canvas.width as f32,
                h: canvas.height as f32,
            },
        });
        statements.push(Statement::Assign {
            target: "bg".into(),
            property: Property::Fill,
            value: Value::Color(bg_color),
        });
    }

    // v0.8: the persistent scene is emitted first, so it renders behind
    // everything that moves — which is where it was measured.  Each
    // static shape is a one-sample track, which routes it through
    // exactly the same classifier, outline tracer and honesty checks a
    // moving object gets, and drops out of the animate path for free.
    // v1.1: ground. Large regions a few shades off the background — a
    // plot area inside its margin — never reach the foreground threshold,
    // so through v1.0 they were painted the background colour: on
    // `real_double_pendulum` a 13-level error over 420 000 pixels. They are
    // found in the raw temporal median, kept only when large and
    // rect-shaped, and drawn first, behind everything else.
    if let Some((raw, ground_bg)) = ground.filter(|_| !rastered) {
        let gmask = crate::detect::ground_mask(raw, ground_bg);
        let canvas_px = (canvas.width * canvas.height) as f32;
        let mut ground: Vec<Blob> = crate::detect::detect_blobs_masked(
            raw,
            &gmask,
            ground_bg,
            Segmentation::Regions,
            (canvas_px * GROUND_MIN_FRACTION) as u32,
        )
        .into_iter()
        .filter(|b| b.fill_ratio >= GROUND_MIN_FILL)
        .collect();
        ground.sort_by(|a, b| b.area.cmp(&a.area).then_with(|| crate::detect::cmp_position(a, b)));
        for (n, b) in ground.iter().enumerate() {
            let name = format!("ground_{n}");
            // The bbox is the rect: ground regions are flat panels whose
            // holes are content drawn on top of them.
            let (x0, y0) = ((b.cx - b.rx).round(), (b.cy - b.ry).round());
            statements.push(Statement::Let {
                name: name.clone(),
                shape: Shape::Rect {
                    x: x0,
                    y: y0,
                    w: (2.0 * b.rx).round(),
                    h: (2.0 * b.ry).round(),
                },
            });
            statements.push(Statement::Assign {
                target: name.clone(),
                property: Property::Fill,
                value: Value::Color(b.color),
            });
            statements.push(Statement::Assign {
                target: name,
                property: Property::Stroke,
                value: Value::None,
            });
        }
    }

    if let Some(l) = layer.filter(|_| !rastered) {
        let plate = l.plate();
        let seg = detections.first().map(|d| d.segmentation).unwrap_or_default();
        let mut blobs = crate::detect::detect_blobs_with(&plate, l.bg, seg, tol.min_blob_area);
        blobs.sort_by(|a, b| {
            b.area
                .cmp(&a.area)
                .then_with(|| crate::detect::cmp_position(a, b))
        });
        let plate_dets = [Detection {
            frame_index: 0,
            t_ms: 0,
            bg: l.bg,
            blobs: blobs.clone(),
            segmentation: seg,
        }];
        let plate_frames = [plate];
        let as_track = |idx: usize, blob: &Blob| Track {
            id: idx,
            samples: vec![(0, *blob)],
            frame_indices: vec![0],
            last_seen_frame: 0,
            active: false,
        };
        let scenery: Vec<Track> = blobs.iter().enumerate().map(|(i, b)| as_track(i, b)).collect();
        let (inner_fill, absorbed) = pair_outlines(&scenery);
        let mut n = 0usize;
        for (idx, blob) in blobs.iter().enumerate() {
            if absorbed[idx] {
                continue;
            }
            let mut stmts = Vec::new();
            emit_shape(
                &mut stmts,
                &format!("static_{n}"),
                &scenery[idx],
                &plate_frames,
                &plate_dets,
                tol,
                inner_fill[idx],
            );
            // Scenery is on screen for the whole clip.
            let removed = blob.area as f64
                * contrast_sq(blob.color, l.bg)
                * frames.len() as f64;
            if worth_it(&stmts, removed) {
                statements.extend(stmts);
                n += 1;
            }
        }
    }

    let mut tracks = build_tracks(detections);
    // Filter out tracks that are too short to be meaningful.
    tracks.retain(|t| t.samples.len() >= MIN_TRACK_SAMPLES);
    // Stable order: longest tracks first → lowest indices to the most
    // prominent objects.  Tie-break by first-seen time.
    tracks.sort_by(|a, b| {
        b.samples
            .len()
            .cmp(&a.samples.len())
            .then_with(|| a.samples[0].0.cmp(&b.samples[0].0))
    });

    let (inner_fill, absorbed) = pair_outlines(&tracks);
    let mut n = 0usize;
    for (ti, track) in tracks.iter().enumerate() {
        if absorbed[ti] {
            continue;
        }
        let mut stmts = Vec::new();
        emit_shape(
            &mut stmts,
            &format!("shape_{n}"),
            track,
            frames,
            detections,
            tol,
            inner_fill[ti],
        );
        let removed: f64 = track
            .samples
            .iter()
            .zip(&track.frame_indices)
            .map(|((_, b), &fi)| {
                let bg = detections.get(fi).map(|d| d.bg).unwrap_or(bg_color);
                b.area as f64 * contrast_sq(b.color, bg)
            })
            .sum();
        if worth_it(&stmts, removed) {
            statements.extend(stmts);
            n += 1;
        }
    }

    Document {
        canvas,
        scenes: vec![Scene {
            name: "main".into(),
            duration_ms: total_ms,
            statements,
        }],
    }
}

/// Emit one recovered object: a `let`, its fill and stroke, and — when
/// it actually moved — an `animate` block.  Used for both moving tracks
/// and (v0.8) the static scene, whose shapes arrive as one-sample
/// tracks measured on the median plate.
/// Smallest max/min scale ratio across a track that is claimed as a real
/// size change rather than anti-aliasing jitter.
const SCALE_MIN_RATIO: f32 = 1.12;
/// How far a sample's rx/ry may drift from the track's median aspect and
/// still count as a measurement of its uniform scale.
const SCALE_MAX_ASPECT_DRIFT: f32 = 0.15;
/// How many consecutive samples (after the 5-sample median) a track's
/// axes must part by more than [`SCALE_MAX_ASPECT_DRIFT`] for it to be
/// written as stretching, `scale(sx, sy)` (v1.21), rather than as a
/// uniform scale with its merges held out.
const STRETCH_MIN_RUN: usize = 5;
/// The least a stretching axis must change, in pixels of half-extent
/// (4: the shape's width or height changes by 8 px). testsrc2's 8 px
/// squares wobble a pixel either side as they touch, a 25% aspect change
/// that is the detector's noise, not the shape growing.
const STRETCH_MIN_PX: f32 = 4.0;
/// A fill closer than this (RGB distance) to the background cannot carry a
/// measurable opacity.
const OPACITY_MIN_CONTRAST: f32 = 60.0;
/// How far (RGB distance) a sample may sit off the background→fill segment
/// before the colour change is read as a hue change instead of a fade.
const OPACITY_MAX_OFF_AXIS: f32 = 12.0;
/// Opacity is emitted only if some sample drops to or below this.
const OPACITY_MAX_EMITTED: f32 = 0.88;
/// Keyframe tolerance for the opacity channel, in 0–255 levels.
const OPACITY_RDP_TOLERANCE: f32 = 6.0;

/// A rotated rect/ellipse track emits `rotate()` when its angle sweeps at
/// least this many degrees, or sits at least `ROT_MIN_STATIC_DEG` away
/// from axis-aligned throughout.
const ROT_MIN_RANGE_DEG: f32 = 3.0;
const ROT_MIN_STATIC_DEG: f32 = 1.5;
/// A path track only carries rotation when it is clearly elongated (so its
/// principal axis is meaningful) and turns through at least this much.
const PATH_ROT_MIN_ELONGATION: f32 = 1.6;
const PATH_ROT_MIN_RANGE_DEG: f32 = 8.0;

/// Soft (coverage) sizing applies only to samples whose primitive fits at
/// least this well — the same bar as being called a rect or ellipse at all.
const SOFT_MIN_FIT_IOU: f32 = 0.88;

/// A ground region must cover at least this fraction of the canvas...
const GROUND_MIN_FRACTION: f32 = 0.02;
/// ...and fill at least this much of its bounding box.
const GROUND_MIN_FILL: f32 = 0.75;

/// A track's interior colour must stray this far (RGB distance) from its
/// median before it carries a `fill()` colour channel (v1.3).
const COLOUR_MIN_DEV: f32 = 24.0;
/// A sample-to-sample colour step larger than this (RGB distance)...
const COLOUR_MAX_JUMP: f32 = 40.0;
/// ...is only believed if the area changes by at most this factor.
const COLOUR_JUMP_MAX_AREA_RATIO: f32 = 1.25;

/// A blob is an outline when its enclosed hole is at least this fraction of
/// its filled area (v1.4).
const RING_MIN_HOLE_FRACTION: f32 = 0.3;
/// An outline and the fill it frames must be centred within this many
/// pixels of each other...
const PAIR_MAX_OFFSET_PX: f32 = 2.5;
/// ...and the fill must cover at least this fraction of the outline's hole.
const PAIR_MIN_HOLE_COVER: f32 = 0.6;

/// Offset from a pixel index to that pixel's centre in continuous
/// coordinates.
const PIXEL_CENTRE: f32 = 0.5;

/// Clearance beyond the canvas edge for a shape held off-canvas.
const OFF_CANVAS_MARGIN_PX: f32 = 1.0;

/// How close a bbox edge must sit to the canvas border to count as
/// clipped by it.
const CLIP_EDGE_PX: f32 = 1.5;

fn emit_shape(
    statements: &mut Vec<Statement>,
    name: &str,
    track: &Track,
    frames: &[VideoFrame],
    detections: &[Detection],
    tol: &Tolerances,
    inner_fill: Option<Color>,
) {
    let name = name.to_string();
    // Majority-vote the shape kind across all samples of the track.
    // A track that classifies inconsistently usually means antialiased
    // edges flipping the bbox fill-ratio across the threshold; majority
    // is more robust than per-frame.
    let mut kind = majority_kind(&track.samples);
    // Aggregate rx and ry independently so a wide rect stays wide.
    let rxs: Vec<f32> = track.samples.iter().map(|(_, b)| b.rx).collect();
    let rys: Vec<f32> = track.samples.iter().map(|(_, b)| b.ry).collect();
    let mut rx = round_to_int(median_f32(&rxs));
    let mut ry = round_to_int(median_f32(&rys));
    // v1.4: outlines. A blob enclosing a large hole whose filled outline is
    // a genuine rect or ellipse is that primitive *stroked*, not filled: an
    // outlined box used to come back as a small filled ellipse. Its stroke
    // width follows from the ring's area and outer size.
    let ring_samples: Vec<&Blob> = track.samples.iter().map(|(_, b)| b).filter(|b| is_outline(b)).collect();
    let ring_width: Option<f32> = (ring_samples.len() * 2 > track.samples.len()).then(|| {
        let rect_votes = ring_samples
            .iter()
            .filter(|b| b.filled_iou_rect >= b.filled_iou_ellipse)
            .count();
        kind = if rect_votes * 2 >= ring_samples.len() {
            ShapeKind::Rect
        } else {
            ShapeKind::Ellipse
        };
        let ts: Vec<f32> = ring_samples
            .iter()
            .map(|b| {
                // Coverage-weighted area when the detector could read it.
                let area = if b.soft_area > 0.0 { b.soft_area } else { b.area as f32 };
                ring_thickness(b.rx, b.ry, area, kind)
            })
            .collect();
        round_half(median_f32(&ts)).max(1.0)
    });
    // v0.14: a rect/ellipse track is *rotated* when most of its samples
    // were fitted best by an oriented primitive. Its size is then measured
    // along its own axes from the second moments — the bbox of a turning
    // bar says nothing about the bar — and its angle becomes a channel.
    let rotated = ring_width.is_none()
        && matches!(kind, ShapeKind::Rect | ShapeKind::Ellipse)
        && track.samples.iter().filter(|(_, b)| b.angle_deg != 0.0).count() * 2
            > track.samples.len();
    if rotated {
        let half = |v: f32| match kind {
            ShapeKind::Rect => crate::detect::rect_half(v),
            _ => crate::detect::ellipse_semi(v),
        };
        let majors: Vec<f32> = track.samples.iter().map(|(_, b)| half(b.var_major)).collect();
        let minors: Vec<f32> = track.samples.iter().map(|(_, b)| half(b.var_minor)).collect();
        rx = round_half(median_f32(&majors)).max(0.5);
        ry = round_half(median_f32(&minors)).max(0.5);
    }
    // The bbox medians stay the reference for edge-clipping logic, which
    // compares them with each sample's own bbox.
    let (brx, bry) = (rx, ry);
    // v1.1: an axis-aligned rect or ellipse is sized from coverage-weighted
    // moments rather than the bbox of thresholded pixels, which reads a
    // moving shape ~1 px wide and grew `two_balls`' radius by a pixel per
    // round trip. Samples clipped by the canvas edge under-measure, so
    // they are left out when any unclipped sample exists.
    if ring_width.is_none() && !rotated && matches!(kind, ShapeKind::Rect | ShapeKind::Ellipse) {
        let frame_wh = frames.first().map(|f| (f.width as f32, f.height as f32));
        let inside = |b: &Blob| {
            frame_wh.is_none_or(|(w, h)| {
                b.cx - b.rx > CLIP_EDGE_PX
                    && b.cy - b.ry > CLIP_EDGE_PX
                    && b.cx + b.rx < w - CLIP_EDGE_PX
                    && b.cy + b.ry < h - CLIP_EDGE_PX
            })
        };
        let mut use_samples: Vec<&Blob> =
            track.samples.iter().map(|(_, b)| b).filter(|b| inside(b)).collect();
        if use_samples.is_empty() {
            use_samples = track.samples.iter().map(|(_, b)| b).collect();
        }
        let half = |v: f32| match kind {
            ShapeKind::Rect => crate::detect::soft_rect_half(v),
            _ => crate::detect::soft_ellipse_semi(v),
        };
        // Only flat samples that genuinely are their primitive carry a
        // soft size: the detector declines on shaded blobs, and a blob that
        // fell back to an ellipse because nothing fitted (a diagonal loader
        // tick) is no ellipse to size. Use it when most samples qualify.
        // Rects use the coverage-corrected bbox, which a notch does not
        // disturb; ellipses use the coverage-weighted variance, which is
        // exact for an ellipse.
        let sized: Vec<&&Blob> = use_samples
            .iter()
            .filter(|b| b.soft_w > 0.0 && b.soft_h > 0.0 && b.fit_iou >= SOFT_MIN_FIT_IOU)
            .collect();
        if sized.len() * 2 > use_samples.len() {
            let (xs, ys): (Vec<f32>, Vec<f32>) = match kind {
                ShapeKind::Rect => (
                    sized.iter().map(|b| b.soft_w * 0.5).collect(),
                    sized.iter().map(|b| b.soft_h * 0.5).collect(),
                ),
                _ => (
                    sized.iter().map(|b| half(b.soft_var_x)).collect(),
                    sized.iter().map(|b| half(b.soft_var_y)).collect(),
                ),
            };
            rx = round_tenth(median_f32(&xs)).max(0.5);
            ry = round_tenth(median_f32(&ys)).max(0.5);
        }
    }
    // An outline is drawn along its centreline: half the stroke inside,
    // half outside, as the renderer strokes.
    // The outer edge is read by coverage too, when the detector could: the
    // thresholded bbox sits ~½ px outside a codec-blurred edge, which
    // pushed `outlined_ui`'s card border half a pixel out.
    if let Some(t) = ring_width {
        let outer_x: Vec<f32> = ring_samples
            .iter()
            .map(|b| if b.soft_w > 0.0 { b.soft_w * 0.5 } else { b.rx })
            .collect();
        let outer_y: Vec<f32> = ring_samples
            .iter()
            .map(|b| if b.soft_h > 0.0 { b.soft_h * 0.5 } else { b.ry })
            .collect();
        rx = round_tenth(median_f32(&outer_x) - t * 0.5).max(0.5);
        ry = round_tenth(median_f32(&outer_y) - t * 0.5).max(0.5);
    }
    let color = median_color(track.samples.iter().map(|(_, b)| b.color));
    let (first_t, first_b) = track.samples[0];

    // A moving track is declared at its first-frame position (the
    // animate block re-positions it from there).  A stationary track is
    // declared at its median position, which averages out the per-frame
    // centroid jitter that made it look like it was moving at all.
    let stationary = is_stationary(&track.samples);
    let (base_cx, base_cy) = if stationary {
        let cxs: Vec<f32> = track.samples.iter().map(|(_, b)| b.cx).collect();
        let cys: Vec<f32> = track.samples.iter().map(|(_, b)| b.cy).collect();
        (median_f32(&cxs), median_f32(&cys))
    } else {
        (first_b.cx, first_b.cy)
    };

    // v0.6: a Path-kind track gets its outline from PAC's pipeline —
    // Moore contour tracing at a representative frame, closed-loop
    // RDP, Schneider Bezier fitting — translated so the outline's
    // centroid sits at the declaration anchor.  When tracing fails
    // (blob vanished from its frame, degenerate contour) the track
    // falls back to the ellipse primitive rather than being dropped.
    let mut path_segs: Option<Vec<PathSeg>> = None;
    if kind == ShapeKind::Path {
        path_segs = trace_track_outline(track, frames, detections, (base_cx, base_cy), tol);
        if path_segs.is_none() {
            kind = ShapeKind::Ellipse;
            // The component was too sprawling for a filled outline (or
            // tracing failed outright).  A bbox-sized ellipse would
            // overclaim just as badly, so shrink it to the measured
            // pixel area — the render never paints more ink than was
            // measured.  Compact blobs are unaffected (scale ≈ 1).
            let areas: Vec<f32> = track.samples.iter().map(|(_, b)| b.area as f32).collect();
            let median_area = median_f32(&areas);
            let ellipse_area = std::f32::consts::PI * rx * ry;
            if ellipse_area > 1.0 {
                let scale = (median_area / ellipse_area).sqrt().min(1.0);
                rx = round_to_int(rx * scale).max(1.0);
                ry = round_to_int(ry * scale).max(1.0);
            }
        }
    }

    // All primitives keep keyframe positions in the same coordinate the
    // compiler expects as `position(...)` anchor:
    //   Ellipse anchor = (cx, cy)
    //   Rect    anchor = top-left (x, y) = (cx - rx, cy - ry)
    //   Path    anchor = outline centroid = blob centroid (cx, cy)
    // A blob clipped by the canvas edge has a bbox centre biased toward
    // the canvas interior: a card 3 px on screen reads as centred 1.5 px
    // in.  Recover the true centre from the *visible* opposite edge and
    // the track's full measured extent.
    let canvas = frames.first().map(|f| (f.width as f32, f.height as f32));
    let unclip = |b: &Blob| -> Blob {
        let Some((w, h)) = canvas else { return *b };
        if rotated {
            // rx/ry are axis lengths here, not bbox extents.
            return *b;
        }
        let mut u = *b;
        if b.rx < brx {
            if b.cx - b.rx <= CLIP_EDGE_PX {
                u.cx = b.cx + b.rx - brx;
            } else if b.cx + b.rx >= w - CLIP_EDGE_PX {
                u.cx = b.cx - b.rx + brx;
            }
        }
        if b.ry < bry {
            if b.cy - b.ry <= CLIP_EDGE_PX {
                u.cy = b.cy + b.ry - bry;
            } else if b.cy + b.ry >= h - CLIP_EDGE_PX {
                u.cy = b.cy - b.ry + bry;
            }
        }
        u.rx = u.rx.max(brx);
        u.ry = u.ry.max(bry);
        u
    };
    // Rect and ellipse anchors live in continuous coordinates: a blob's
    // centroid is a mean of pixel *indices*, half a pixel short of the
    // pixel centres it stands for (`PIXEL_CENTRE`). Whole-pixel rounding
    // used to hide that by luck; at tenth-pixel precision it put every
    // shape ½ px up and left. Paths are traced in index coordinates and
    // stay consistent with themselves.
    let anchor_xy = |b: &Blob| -> (f32, f32) {
        let b = &unclip(b);
        match kind {
            ShapeKind::Path => (b.cx, b.cy),
            ShapeKind::Ellipse => (b.cx + PIXEL_CENTRE, b.cy + PIXEL_CENTRE),
            ShapeKind::Rect => (b.cx + PIXEL_CENTRE - rx, b.cy + PIXEL_CENTRE - ry),
        }
    };

    // v0.11: size and opacity channels.  Each is measured per sample
    // and emitted only when it varies by more than detector noise, so a
    // track that neither grows nor fades produces exactly the source it
    // did before.
    let touches_edge = |b: &Blob| {
        canvas.is_some_and(|(w, h)| {
            b.cx - b.rx <= CLIP_EDGE_PX
                || b.cy - b.ry <= CLIP_EDGE_PX
                || b.cx + b.rx >= w - CLIP_EDGE_PX
                || b.cy + b.ry >= h - CLIP_EDGE_PX
        })
    };
    // Scale = sqrt(area / median area).  Area rather than bbox extent:
    // it integrates every pixel, so anti-aliasing jitter averages out.
    // Skipped for paths (their outline is traced at one frame, so no
    // single reference size exists) and for any track clipped by the
    // canvas edge, whose visible area shrinks without the shape doing so.
    let scales: Option<Vec<(f32, f32)>> = (|| {
        if kind == ShapeKind::Path
            || rotated
            || ring_width.is_some()
            || track.samples.len() < 3
            || track.samples.iter().any(|(_, b)| touches_edge(b))
        {
            return None;
        }
        // v1.21: each axis measured on its own.
        // When the two part for a sustained run the shape stretches —
        // a progress bar filling, a bar rising — and is written
        // `scale(sx, sy)`. A short excursion is a merge (two same-colour
        // blobs side by side are twice as wide, not as tall), and the
        // temporal median takes it out.
        let smooth = |v: &[f32]| -> Vec<f32> {
            (0..v.len())
                .map(|i| median_f32(&v[i.saturating_sub(2)..(i + 3).min(v.len())]))
                .collect()
        };
        let half_w: Vec<f32> = track
            .samples
            .iter()
            .map(|(_, b)| if b.soft_w > 0.0 { b.soft_w * 0.5 } else { b.rx })
            .collect();
        let half_h: Vec<f32> = track
            .samples
            .iter()
            .map(|(_, b)| if b.soft_h > 0.0 { b.soft_h * 0.5 } else { b.ry })
            .collect();
        // Each axis against its own median, as the uniform scale is against
        // the median area: the declared size comes from a stricter
        // estimator, and measured against it a shape of constant size read
        // as a constant `scale(0.8, 1)` (testsrc2's squares).
        let (mw, mh) = (median_f32(&half_w).max(0.5), median_f32(&half_h).max(0.5));
        let sx: Vec<f32> = smooth(&half_w).iter().map(|v| v / mw).collect();
        let sy: Vec<f32> = smooth(&half_h).iter().map(|v| v / mh).collect();
        let drift = (1.0 + SCALE_MAX_ASPECT_DRIFT).ln();
        let (mut run, mut longest) = (0usize, 0usize);
        for (a, b) in sx.iter().zip(&sy) {
            if (a / b.max(1e-3)).ln().abs() > drift {
                run += 1;
                longest = longest.max(run);
            } else {
                run = 0;
            }
        }
        if longest >= STRETCH_MIN_RUN {
            // Grows by enough to be the shape and not its outline's noise:
            // a pixel either side is a quarter of an 8 px square.
            let grows = |v: &[f32], m: f32| {
                let (lo, hi) = (
                    v.iter().cloned().fold(f32::INFINITY, f32::min),
                    v.iter().cloned().fold(0.0f32, f32::max),
                );
                hi / lo.max(1e-3) >= SCALE_MIN_RATIO && (hi - lo) * m >= STRETCH_MIN_PX
            };
            return (grows(&sx, mw) || grows(&sy, mh)).then(|| sx.into_iter().zip(sy).collect());
        }
        let areas: Vec<f32> = track.samples.iter().map(|(_, b)| b.area as f32).collect();
        let med = median_f32(&areas).max(1.0);
        // `scale()` is uniform, so a real size change keeps the aspect
        // ratio.  A sample whose aspect departs from the track's median
        // (two blobs merged side by side double the width, not the
        // height) is not a measurement of this shape's size: it holds
        // the previous valid value instead.
        let aspects: Vec<f32> = track
            .samples
            .iter()
            .map(|(_, b)| b.rx / b.ry.max(0.5))
            .collect();
        let med_aspect = median_f32(&aspects);
        let mut raw: Vec<f32> = Vec::with_capacity(areas.len());
        let mut held: Option<f32> = None;
        for (a, asp) in areas.iter().zip(&aspects) {
            if (asp / med_aspect - 1.0).abs() <= SCALE_MAX_ASPECT_DRIFT {
                held = Some((a / med).sqrt());
            }
            raw.push(held.unwrap_or(f32::NAN));
        }
        let first_valid = raw.iter().copied().find(|v| !v.is_nan())?;
        for v in raw.iter_mut() {
            if v.is_nan() {
                *v = first_valid;
            }
        }
        // Temporal median over 5 samples: a real size change is smooth,
        // while the area spikes of a merge (two same-colour blobs
        // crossing) last a frame or two and must not read as growth.
        let s: Vec<f32> = (0..raw.len())
            .map(|i| {
                let lo = i.saturating_sub(2);
                let hi = (i + 3).min(raw.len());
                median_f32(&raw[lo..hi])
            })
            .collect();
        let lo = s.iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = s.iter().cloned().fold(0.0, f32::max);
        (hi / lo.max(1e-3) >= SCALE_MIN_RATIO).then(|| s.into_iter().map(|v| (v, v)).collect())
    })();
    // Opacity: the renderer composites `bg + a·(fill − bg)`, so every
    // sample's colour should lie on the segment from its frame's
    // background to the full-strength fill.  The full-strength fill is
    // the sample furthest from the background; `a` is the projection
    // onto that segment.  If any sample sits off the segment the colour
    // is changing hue, not fading, and no opacity is claimed.
    let opacity: Option<(Vec<f32>, Color)> = (|| {
        if ring_width.is_some()
            || track.samples.len() < 3
            || track.frame_indices.len() != track.samples.len()
        {
            return None;
        }
        let rgb = |c: Color| [c.r as f32, c.g as f32, c.b as f32];
        let bgs: Vec<[f32; 3]> = track
            .frame_indices
            .iter()
            .map(|&fi| detections.get(fi).map(|d| rgb(d.bg)))
            .collect::<Option<_>>()?;
        let diff = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let (k_full, _) = track
            .samples
            .iter()
            .enumerate()
            .map(|(k, (_, b))| {
                let u = diff(rgb(b.color), bgs[k]);
                (k, dot(u, u))
            })
            .fold((0, -1.0), |acc, x| if x.1 > acc.1 { x } else { acc });
        let full = track.samples[k_full].1.color;
        let mut alphas = Vec::with_capacity(track.samples.len());
        for (k, (_, b)) in track.samples.iter().enumerate() {
            let v = diff(rgb(full), bgs[k]);
            let vv = dot(v, v);
            if vv < OPACITY_MIN_CONTRAST * OPACITY_MIN_CONTRAST {
                return None;
            }
            let u = diff(rgb(b.color), bgs[k]);
            let a = (dot(u, v) / vv).clamp(0.0, 1.0);
            let off = diff(u, [v[0] * a, v[1] * a, v[2] * a]);
            if dot(off, off).sqrt() > OPACITY_MAX_OFF_AXIS {
                return None;
            }
            alphas.push(a);
        }
        let lo = alphas.iter().cloned().fold(1.0, f32::min);
        (lo <= OPACITY_MAX_EMITTED).then_some((alphas, full))
    })();
    let color = opacity.as_ref().map_or(color, |(_, full)| *full);

    // v1.3: the colour channel. A track whose interior colour moves away
    // from its median by more than detector noise — and not along the
    // background→fill axis, which opacity already explains — carries a
    // `fill()` keyframe channel. A 3-sample median filter keeps a one-frame
    // merge from reading as a colour change.
    let colours: Option<Vec<Color>> = (|| {
        if opacity.is_some() || ring_width.is_some() || track.samples.len() < 3 {
            return None;
        }
        let raw: Vec<[f32; 3]> = track
            .samples
            .iter()
            .map(|(_, b)| [b.color.r as f32, b.color.g as f32, b.color.b as f32])
            .collect();
        let filt: Vec<[f32; 3]> = (0..raw.len())
            .map(|i| {
                let lo = i.saturating_sub(1);
                let hi = (i + 2).min(raw.len());
                let mut out = [0.0f32; 3];
                for (k, o) in out.iter_mut().enumerate() {
                    let v: Vec<f32> = raw[lo..hi].iter().map(|c| c[k]).collect();
                    *o = median_f32(&v);
                }
                out
            })
            .collect();
        // A colour animation changes colour, not shape. A jump between
        // consecutive samples that coincides with a jump in area is the
        // tracker latching onto a different region — on the Mandelbrot
        // zoom a mauve region's track jumped to the pink ring around it,
        // and painting that "colour change" covered the set (worst-frame
        // IoU 0.40 → 0.09). Such a track keeps its median colour.
        for i in 1..filt.len() {
            let step = ((filt[i][0] - filt[i - 1][0]).powi(2)
                + (filt[i][1] - filt[i - 1][1]).powi(2)
                + (filt[i][2] - filt[i - 1][2]).powi(2))
            .sqrt();
            let (a0, a1) = (track.samples[i - 1].1.area as f32, track.samples[i].1.area as f32);
            let ratio = a0.max(a1) / a0.min(a1).max(1.0);
            if step > COLOUR_MAX_JUMP && ratio > COLOUR_JUMP_MAX_AREA_RATIO {
                return None;
            }
        }
        let med: [f32; 3] = [0, 1, 2].map(|k| median_f32(&filt.iter().map(|c| c[k]).collect::<Vec<_>>()));
        let dev = filt
            .iter()
            .map(|c| ((c[0] - med[0]).powi(2) + (c[1] - med[1]).powi(2) + (c[2] - med[2]).powi(2)).sqrt())
            .fold(0.0f32, f32::max);
        (dev >= COLOUR_MIN_DEV).then(|| {
            filt.iter()
                .map(|c| Color::rgb(c[0].round() as u8, c[1].round() as u8, c[2].round() as u8))
                .collect()
        })
    })();

    // v0.14: the rotation channel, in degrees. Principal-axis angles are
    // only defined modulo 180°, so the series is unwrapped to the nearest
    // turn of its predecessor before anything reads it. Rect and ellipse
    // tracks carry their absolute angle; a path carries its angle relative
    // to the sample its outline was traced from.
    let unwrapped: Vec<f32> = {
        let mut out: Vec<f32> = Vec::with_capacity(track.samples.len());
        for (_, b) in &track.samples {
            let mut a = b.theta_deg;
            if let Some(&p) = out.last() {
                while a - p > 90.0 {
                    a -= 180.0;
                }
                while a - p < -90.0 {
                    a += 180.0;
                }
            }
            out.push(a);
        }
        out
    };
    let rotation: Option<Vec<f32>> = (|| {
        let lo = unwrapped.iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = unwrapped.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        if rotated {
            let med = median_f32(&unwrapped);
            return (hi - lo >= ROT_MIN_RANGE_DEG || med.abs() >= ROT_MIN_STATIC_DEG)
                .then(|| unwrapped.clone());
        }
        if kind == ShapeKind::Path && track.samples.len() >= 3 {
            let el: Vec<f32> = track.samples.iter().map(|(_, b)| b.elongation()).collect();
            if median_f32(&el) < PATH_ROT_MIN_ELONGATION || hi - lo < PATH_ROT_MIN_RANGE_DEG {
                return None;
            }
            let rep = unwrapped[representative_sample(track)];
            return Some(unwrapped.iter().map(|a| a - rep).collect());
        }
        None
    })();

    // v1.21: a stretching shape is declared at its fullest, so its scale
    // reads as how full it is — a progress bar `scale(0.02, 1)` to
    // `scale(1, 1)` — with its first frame's top-left.
    // The 90th percentile, so a merge's brief spike does not set it.
    let fullest: Option<(f32, f32)> = scales.as_ref().filter(|sc| sc.iter().any(|s| s.0 != s.1)).map(|sc| {
        let p90 = |mut v: Vec<f32>| {
            v.sort_by(f32::total_cmp);
            v[(v.len() * 9 / 10).min(v.len() - 1)].max(1e-3)
        };
        (p90(sc.iter().map(|s| s.0).collect()), p90(sc.iter().map(|s| s.1).collect()))
    });
    let shape = match kind {
        ShapeKind::Ellipse => {
            let (mx, my) = fullest.unwrap_or((1.0, 1.0));
            Shape::Ellipse {
                cx: round_pos(base_cx + PIXEL_CENTRE),
                cy: round_pos(base_cy + PIXEL_CENTRE),
                rx: round_tenth(rx * mx),
                ry: round_tenth(ry * my),
            }
        }
        ShapeKind::Rect => match (fullest, &scales) {
            (Some((mx, my)), Some(sc)) => Shape::Rect {
                x: round_pos(first_b.cx + PIXEL_CENTRE - rx * sc[0].0),
                y: round_pos(first_b.cy + PIXEL_CENTRE - ry * sc[0].1),
                w: round_tenth(2.0 * rx * mx),
                h: round_tenth(2.0 * ry * my),
            },
            _ => Shape::Rect {
                x: round_pos(base_cx + PIXEL_CENTRE - rx),
                y: round_pos(base_cy + PIXEL_CENTRE - ry),
                w: 2.0 * rx,
                h: 2.0 * ry,
            },
        },
        ShapeKind::Path => Shape::Path(path_segs.take().expect("checked above")),
    };

    // v1.2: a gradient fill, fitted to the interior pixels of the
    // representative sample and shifted into declared coordinates. Only
    // for shapes whose look one frame can stand for: a rotating, scaling
    // or fading track would need its shading re-expressed per channel.
    let gradient: Option<vac_format::Gradient> = (|| {
        if rotated || scales.is_some() || opacity.is_some() || colours.is_some() || ring_width.is_some() {
            return None;
        }
        let rep = representative_sample(track);
        let (_, rb) = track.samples[rep];
        let fi = *track.frame_indices.get(rep)?;
        let frame = frames.get(fi)?;
        let det = detections.get(fi)?;
        let px = crate::detect::interior_pixels(
            frame,
            det.bg,
            (rb.cx, rb.cy),
            rb.rx.max(rb.ry).max(4.0),
            (rb.area / 4).max(1),
            det.segmentation,
        )?;
        let fit = crate::gradient::fit(&px)?;
        let (dx, dy) = ((base_cx - rb.cx).round(), (base_cy - rb.cy).round());
        let mv = |p: vac_format::Point| vac_format::Point::new(p.x + dx, p.y + dy);
        Some(match fit.gradient {
            vac_format::Gradient::Linear { c0, p0, c1, p1 } => vac_format::Gradient::Linear {
                c0,
                p0: mv(p0),
                c1,
                p1: mv(p1),
            },
            vac_format::Gradient::Radial { c0, center, c1, radius } => vac_format::Gradient::Radial {
                c0,
                center: mv(center),
                c1,
                radius,
            },
        })
    })();

    statements.push(Statement::Let {
        name: name.clone(),
        shape,
    });
    statements.push(Statement::Assign {
        target: name.clone(),
        property: Property::Fill,
        value: match (ring_width, gradient) {
            // An outline paired with the fill it frames (v1.4) is one
            // shape: the fill runs to the stroke's centreline, under it.
            (Some(_), _) => inner_fill.map_or(Value::None, Value::Color),
            (None, Some(g)) => Value::Gradient(g),
            (None, None) => Value::Color(color),
        },
    });
    statements.push(Statement::Assign {
        target: name.clone(),
        property: Property::Stroke,
        value: match ring_width {
            Some(_) => Value::Color(color),
            None => Value::None,
        },
    });
    if let Some(t) = ring_width {
        statements.push(Statement::Assign {
            target: name.clone(),
            property: Property::StrokeWidth,
            value: Value::Number(t),
        });
    }

    // Simplify the *unclipped* trajectory, so an edge-clipped entry is
    // not mistaken for a slow start.
    let unclipped: Vec<(u32, Blob)> = track.samples.iter().map(|(t, b)| (*t, unclip(b))).collect();
    let blobs = &unclipped;

    let mut keep = simplify_trajectory(blobs, tol.keyframe_px);
    let times: Vec<f32> = blobs.iter().map(|(t, _)| *t as f32).collect();
    if let Some(sc) = &scales {
        // Tolerance in pixels of radius, like position; a stretch on each
        // axis's own extent.
        if sc.iter().all(|s| s.0 == s.1) {
            let extent: Vec<f32> = sc.iter().map(|s| s.0 * (rx + ry) / 2.0).collect();
            keep.extend(rdp_1d(&times, &extent, tol.keyframe_px));
        } else {
            let ex: Vec<f32> = sc.iter().map(|s| s.0 * rx).collect();
            let ey: Vec<f32> = sc.iter().map(|s| s.1 * ry).collect();
            keep.extend(rdp_1d(&times, &ex, tol.keyframe_px));
            keep.extend(rdp_1d(&times, &ey, tol.keyframe_px));
        }
    }
    if let Some((al, _)) = &opacity {
        let levels: Vec<f32> = al.iter().map(|a| a * 255.0).collect();
        keep.extend(rdp_1d(&times, &levels, tol.opacity_levels));
    }
    if let Some(cols) = &colours {
        for k in 0..3 {
            let ch: Vec<f32> = cols
                .iter()
                .map(|c| [c.r, c.g, c.b][k] as f32)
                .collect();
            keep.extend(rdp_1d(&times, &ch, tol.opacity_levels));
        }
    }
    if let Some(rot) = &rotation {
        // Tolerance in pixels at the shape's tip, like position.
        let reach = rx.max(ry).max(1.0);
        let tol_deg = (tol.keyframe_px / reach).to_degrees();
        keep.extend(rdp_1d(&times, rot, tol_deg));
    }
    if !keep.contains(&0) {
        keep.insert(0, 0);
    }
    let last_idx = blobs.len() - 1;
    if !keep.contains(&last_idx) {
        keep.push(last_idx);
    }
    keep.sort();
    keep.dedup();

    // A stationary shape still animates if it grows or fades.  A rect
    // scales about its top-left anchor, so a scaling rect needs position
    // keyframes to stay centred.
    let emit_pos = !stationary || (scales.is_some() && kind == ShapeKind::Rect);
    let has_channels = (keep.len() >= 2
        && (emit_pos
            || scales.is_some()
            || opacity.is_some()
            || rotation.is_some()
            || colours.is_some()))
        || (keep.len() == 1 && rotation.is_some());

    // Lifetime.  A track first seen after the clip starts, or last seen
    // before it ends, was not on screen the whole time.
    let timeline: Vec<u32> = frames
        .iter()
        .scan(0u32, |acc, f| {
            let t = *acc;
            *acc += f.delay_ms;
            Some(t)
        })
        .collect();
    let clip_end = frames.iter().map(|f| f.delay_ms).sum::<u32>();
    let (last_t, last_b) = blobs[last_idx];
    let prev_frame = timeline.iter().copied().take_while(|&t| t < first_t).last();
    let next_frame = timeline
        .iter()
        .copied()
        .find(|&t| t > last_t)
        .filter(|&t| t < clip_end);

    // v0.10: if it was clipped by the canvas edge at that moment it came
    // or went *through* the edge, and is held just off-canvas.
    let off = |b: &Blob| -> Option<(f32, f32)> {
        let (w, h) = canvas?;
        let (mut cx, mut cy) = (b.cx, b.cy);
        // A rotated shape can reach as far as its half-diagonal.
        let (ox, oy) = if rotated {
            let d = (rx * rx + ry * ry).sqrt();
            (d, d)
        } else {
            (rx, ry)
        };
        // A full pixel of clearance: at sub-pixel precision a shape parked
        // exactly at the edge leaves a half-covered column on screen, which
        // the renderer anti-aliases into visible ink on frames the source
        // shows empty (`late_entry.vac` frame 0: IoU 0).
        if b.cx - b.rx <= CLIP_EDGE_PX {
            cx = -ox - OFF_CANVAS_MARGIN_PX;
        } else if b.cx + b.rx >= w - CLIP_EDGE_PX {
            cx = w + ox + OFF_CANVAS_MARGIN_PX;
        } else if b.cy - b.ry <= CLIP_EDGE_PX {
            cy = -oy - OFF_CANVAS_MARGIN_PX;
        } else if b.cy + b.ry >= h - CLIP_EDGE_PX {
            cy = h + oy + OFF_CANVAS_MARGIN_PX;
        } else {
            return None;
        }
        let (ax, ay) = anchor_xy(&Blob {
            cx,
            cy,
            rx,
            ry,
            ..*b
        });
        Some((round_pos(ax), round_pos(ay)))
    };
    let enter_via_edge = prev_frame.and(if emit_pos { off(&first_b) } else { None });
    let exit_via_edge = next_frame.and(if emit_pos { off(&last_b) } else { None });
    // v0.12: otherwise it popped in or out mid-canvas, and is hidden with
    // `opacity(0)` outside its lifetime.  Before v0.12 it was drawn from
    // t=0 and held on screen to the end (`examples/pop_in.vac`).
    let pop_in = prev_frame.is_some() && enter_via_edge.is_none();
    let pop_out = next_frame.is_some() && exit_via_edge.is_none();

    if !(has_channels || pop_in || pop_out) {
        return;
    }
    let easing = if has_channels {
        classify_easing(blobs, &keep)
    } else {
        Easing::Linear
    };

    // v0.10: keyframe times are *absolute* clip time.  Through v0.9 they
    // were re-based to the track's first sighting, so an object entering
    // at 1000 ms replayed its whole trajectory a second early.
    let mut keyframes: Vec<Keyframe> = if has_channels {
        keep.iter()
            .map(|&i| {
                let (t, b) = blobs[i];
                let s = scales.as_ref().map(|sc| sc[i]);
                let mut transforms = Vec::with_capacity(3);
                if emit_pos {
                    let (ax, ay) = match (kind, s) {
                        (ShapeKind::Rect, Some((sx, sy))) => {
                            (b.cx + PIXEL_CENTRE - rx * sx, b.cy + PIXEL_CENTRE - ry * sy)
                        }
                        (ShapeKind::Path, _) if stationary => (base_cx, base_cy),
                        (_, _) if stationary => (base_cx + PIXEL_CENTRE, base_cy + PIXEL_CENTRE),
                        _ => anchor_xy(&b),
                    };
                    transforms.push(Transform::Position(round_pos(ax), round_pos(ay)));
                }
                if let Some((sx, sy)) = s {
                    let r = |v: f32| (v * 1000.0).round() / 1000.0;
                    let (mx, my) = fullest.unwrap_or((1.0, 1.0));
                    transforms.push(Transform::Scale(r(sx / mx), r(sy / my)));
                }
                if let Some((al, _)) = &opacity {
                    transforms.push(Transform::Opacity((al[i] * 100.0).round() / 100.0));
                }
                if let Some(rot) = &rotation {
                    transforms.push(Transform::Rotate((rot[i] * 10.0).round() / 10.0));
                }
                if let Some(cols) = &colours {
                    transforms.push(Transform::Fill(cols[i]));
                }
                Keyframe {
                    time_ms: t,
                    transforms,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    // A scale channel that is `scale(1)` throughout says nothing (v1.21).
    let no_op_scale = |t: &Transform| matches!(t, Transform::Scale(sx, sy) if *sx == 1.0 && *sy == 1.0);
    let has_scale = |t: &Transform| matches!(t, Transform::Scale(..));
    if keyframes.iter().any(|k| k.transforms.iter().any(has_scale))
        && keyframes.iter().all(|k| k.transforms.iter().filter(|t| has_scale(t)).all(no_op_scale))
    {
        for k in keyframes.iter_mut() {
            k.transforms.retain(|t| !has_scale(t));
        }
        keyframes.retain(|k| !k.transforms.is_empty());
    }

    let at_pos = |t: u32, (x, y): (f32, f32)| Keyframe {
        time_ms: t,
        transforms: vec![Transform::Position(x, y)],
    };
    let at_opacity = |t: u32, o: f32| Keyframe {
        time_ms: t,
        transforms: vec![Transform::Opacity(o)],
    };
    // The opacity the shape has while visible at sample `i`: the
    // channel value if one was measured, else fully opaque.
    let visible_opacity = |i: usize| {
        opacity
            .as_ref()
            .map_or(1.0, |(al, _)| (al[i] * 100.0).round() / 100.0)
    };
    // Make sure the keyframe at time `t` carries an opacity, so the
    // per-channel renderer does not hold the hidden value past it.
    let ensure_opacity = |kfs: &mut Vec<Keyframe>, t: u32, o: f32| {
        match kfs.iter_mut().find(|k| k.time_ms == t) {
            Some(k) => {
                if !k.transforms.iter().any(|x| matches!(x, Transform::Opacity(_))) {
                    k.transforms.push(Transform::Opacity(o));
                }
            }
            None => {
                kfs.push(at_opacity(t, o));
                kfs.sort_by_key(|k| k.time_ms);
            }
        }
    };

    if let (Some(pt), Some(p)) = (prev_frame, enter_via_edge) {
        // Off-canvas on the last frame it was unseen, so the entry
        // interpolates across one real frame gap.
        keyframes.insert(0, at_pos(pt, p));
        if pt > 0 {
            keyframes.insert(0, at_pos(0, p));
        }
    }
    if let (Some(nt), Some(p)) = (next_frame, exit_via_edge) {
        keyframes.push(at_pos(nt, p));
    }
    if let (true, Some(pt)) = (pop_in, prev_frame) {
        ensure_opacity(&mut keyframes, first_t, visible_opacity(0));
        keyframes.insert(0, at_opacity(pt, 0.0));
        if pt > 0 {
            keyframes.insert(0, at_opacity(0, 0.0));
        }
    }
    if let (true, Some(nt)) = (pop_out, next_frame) {
        ensure_opacity(&mut keyframes, last_t, visible_opacity(last_idx));
        keyframes.push(at_opacity(nt, 0.0));
    }

    statements.push(Statement::Animate(Animation {
        target: name,
        keyframes,
        easing,
    }));
}

fn majority_kind(samples: &[(u32, Blob)]) -> ShapeKind {
    let rects = samples
        .iter()
        .filter(|(_, b)| b.kind == ShapeKind::Rect)
        .count();
    let paths = samples
        .iter()
        .filter(|(_, b)| b.kind == ShapeKind::Path)
        .count();
    if rects * 2 > samples.len() {
        ShapeKind::Rect
    } else if paths * 2 > samples.len() {
        ShapeKind::Path
    } else {
        ShapeKind::Ellipse
    }
}

/// The sample a path's outline is traced from: median area, the frame
/// where the blob is most typical. A path's rotation channel is measured
/// relative to this same sample.
fn representative_sample(track: &Track) -> usize {
    let mut order: Vec<usize> = (0..track.samples.len()).collect();
    order.sort_by_key(|&i| track.samples[i].1.area);
    order[order.len() / 2]
}

/// A blob enclosing a large hole whose filled outline is a genuine rect or
/// ellipse: an outline (v1.4).
fn is_outline(b: &Blob) -> bool {
    b.hole_area as f32 >= RING_MIN_HOLE_FRACTION * (b.area + b.hole_area) as f32
        && b.filled_iou_rect.max(b.filled_iou_ellipse) >= crate::detect::PRIMITIVE_MIN_IOU
}

fn is_outline_track(t: &Track) -> bool {
    t.samples.iter().filter(|(_, b)| is_outline(b)).count() * 2 > t.samples.len()
}

/// Pair each outline track with the fill track it frames — one that sits
/// in its hole, centred on it, through most of both lifetimes — so the two
/// are emitted as one shape with a fill and a stroke. Returns, per track,
/// the fill colour an outline takes on, and whether a fill was absorbed.
///
/// A bordered card is drawn as a fill with a stroke over its edge, so the
/// visible fill stops short of the stroke's inner edge. Emitted apart, an
/// accurate (thinner) stroke left a dark gap between border and card on
/// `outlined_ui.mp4`; emitted as one shape, the fill runs under the stroke.
fn pair_outlines(tracks: &[Track]) -> (Vec<Option<Color>>, Vec<bool>) {
    let mut inner: Vec<Option<Color>> = vec![None; tracks.len()];
    let mut absorbed = vec![false; tracks.len()];
    for (ri, r) in tracks.iter().enumerate() {
        if absorbed[ri] || !is_outline_track(r) {
            continue;
        }
        let by_frame: std::collections::HashMap<usize, &Blob> = r
            .frame_indices
            .iter()
            .zip(&r.samples)
            .map(|(&fi, (_, b))| (fi, b))
            .collect();
        for (fi_idx, f) in tracks.iter().enumerate() {
            if fi_idx == ri || absorbed[fi_idx] || inner[fi_idx].is_some() || is_outline_track(f) {
                continue;
            }
            let matched = f
                .frame_indices
                .iter()
                .zip(&f.samples)
                .filter(|(fi, (_, fb))| {
                    by_frame.get(fi).is_some_and(|rb| {
                        (rb.cx - fb.cx).abs() <= PAIR_MAX_OFFSET_PX
                            && (rb.cy - fb.cy).abs() <= PAIR_MAX_OFFSET_PX
                            && fb.rx <= rb.rx
                            && fb.ry <= rb.ry
                            && fb.area as f32 >= PAIR_MIN_HOLE_COVER * rb.hole_area as f32
                    })
                })
                .count();
            if matched * 5 >= f.samples.len() * 4 && matched * 2 >= r.samples.len() {
                inner[ri] = Some(median_color(f.samples.iter().map(|(_, b)| b.color)));
                absorbed[fi_idx] = true;
                break;
            }
        }
    }
    (inner, absorbed)
}

/// Stroke thickness of an outline from its outer half-extents and ring
/// area: a rect ring `W×H` of thickness `t` has area `2t(W+H) − 4t²`, an
/// ellipse ring with semi-axes `a, b` has `π(t(a+b) − t²)`; solve for `t`.
fn ring_thickness(rx: f32, ry: f32, area: f32, kind: ShapeKind) -> f32 {
    match kind {
        ShapeKind::Rect => {
            let s = 2.0 * rx + 2.0 * ry;
            ((s - (s * s - 4.0 * area).max(0.0).sqrt()) / 4.0).max(0.5)
        }
        _ => {
            let s = rx + ry;
            ((s - (s * s - 4.0 * area / std::f32::consts::PI).max(0.0).sqrt()) / 2.0).max(0.5)
        }
    }
}

/// Round to a tenth of a pixel: coverage sizing measures sub-pixel extents,
/// and a 1.3 px line declared 1 px (half-pixel rounding) lost the partial
/// pixel its anti-aliasing really covers.
fn round_tenth(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// Round to the nearest half pixel, so a rect's width `2·rx` can be odd.
fn round_half(v: f32) -> f32 {
    (v * 2.0).round() / 2.0
}

/// Trace a Path-kind track's outline at its representative sample
/// (median area — the frame where the blob is most typical), simplify
/// with PAC's closed-loop RDP, fit Schneider Beziers, and translate
/// the outline so its centroid sits at `declare_at` (the track's
/// declaration anchor).  Coordinates are rounded to 2 decimals so the
/// emitted `.vac` stays compact and write→parse→write is stable.
fn trace_track_outline(
    track: &Track,
    frames: &[VideoFrame],
    detections: &[Detection],
    declare_at: (f32, f32),
    tol: &Tolerances,
) -> Option<Vec<PathSeg>> {
    let rep = representative_sample(track);
    let (_, rep_b) = track.samples[rep];
    let fidx = *track.frame_indices.get(rep)?;
    let frame = frames.get(fidx)?;
    let det = detections.get(fidx)?;

    let contour = crate::detect::extract_contour(
        frame,
        det.bg,
        (rep_b.cx, rep_b.cy),
        rep_b.rx.max(rep_b.ry).max(4.0),
        (rep_b.area / 4).max(1),
        det.segmentation,
    )?;
    let simplified = pac_decompiler::simplify::rdp(&contour, tol.outline_px);
    if simplified.len() < 3 {
        return None;
    }
    // Degenerate outline (collapsed to a point / sliver): the traced
    // component clearly wasn't the blob — fall back to the ellipse.
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in &simplified {
        min_x = min_x.min(p.0);
        max_x = max_x.max(p.0);
        min_y = min_y.min(p.1);
        max_y = max_y.max(p.1);
    }
    if max_x - min_x < 2.0 || max_y - min_y < 2.0 {
        return None;
    }

    // Filled-outline honesty check: a spidery component (swept traces,
    // scribbles) has an outer contour enclosing far more area than its
    // pixels — filling it would paint a slab nothing measured.
    let outline_area = shoelace_area(&simplified);
    if outline_area > rep_b.area as f32 * PATH_MAX_FILL_OVERCLAIM {
        return None;
    }

    let dx = declare_at.0 - rep_b.cx;
    let dy = declare_at.1 - rep_b.cy;
    let moved: Vec<(f32, f32)> = simplified.iter().map(|p| (p.0 + dx, p.1 + dy)).collect();

    let segs = pac_decompiler::curves::fit_closed(&moved, tol.curve_px);
    if segs.len() < 2 {
        return None;
    }
    let round2 = |v: f32| (v * 100.0).round() / 100.0;
    let round_pt = |p: vac_format::Point| vac_format::Point::new(round2(p.x), round2(p.y));
    Some(
        segs.into_iter()
            .map(|seg| match seg {
                PathSeg::MoveTo(p) => PathSeg::MoveTo(round_pt(p)),
                PathSeg::LineTo(p) => PathSeg::LineTo(round_pt(p)),
                PathSeg::CurveTo { c1, c2, p } => PathSeg::CurveTo {
                    c1: round_pt(c1),
                    c2: round_pt(c2),
                    p: round_pt(p),
                },
            })
            .collect(),
    )
}

// --------------------------------------------------------------------
// Cross-frame tracking
// --------------------------------------------------------------------

/// Build tracks from per-frame detections via lowest-cost-first
/// greedy bipartite matching with gating + a short gap-tolerance.
pub fn build_tracks(detections: &[Detection]) -> Vec<Track> {
    let mut tracks: Vec<Track> = Vec::new();
    let mut next_id = 0usize;

    for (frame_idx, det) in detections.iter().enumerate() {
        // Snapshot of currently active track indices.
        let active: Vec<usize> = tracks
            .iter()
            .enumerate()
            .filter_map(|(i, t)| if t.active { Some(i) } else { None })
            .collect();

        // Build all viable (cost, blob, track) candidates, scoring
        // position against each track's constant-velocity prediction.
        let mut candidates: Vec<(f32, usize, usize)> =
            Vec::with_capacity(det.blobs.len() * active.len());
        for &t_idx in &active {
            let last_blob = tracks[t_idx].samples.last().unwrap().1;
            let predicted = predicted_position(&tracks[t_idx], det.t_ms);
            let gate = if tracks[t_idx].samples.len() < 2 {
                BOOTSTRAP_COST_GATE
            } else {
                COST_GATE
            };
            for (b_idx, blob) in det.blobs.iter().enumerate() {
                let cost = matching_cost(&last_blob, predicted, blob);
                if cost <= gate {
                    candidates.push((cost, b_idx, t_idx));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut blob_assigned = vec![false; det.blobs.len()];
        let mut track_assigned: std::collections::HashSet<usize> = std::collections::HashSet::new();

        for (_cost, b_idx, t_idx) in candidates {
            if blob_assigned[b_idx] || track_assigned.contains(&t_idx) {
                continue;
            }
            tracks[t_idx].samples.push((det.t_ms, det.blobs[b_idx]));
            tracks[t_idx].frame_indices.push(frame_idx);
            tracks[t_idx].last_seen_frame = frame_idx;
            blob_assigned[b_idx] = true;
            track_assigned.insert(t_idx);
        }

        // Unassigned blobs → start new tracks.
        for (b_idx, blob) in det.blobs.iter().enumerate() {
            if !blob_assigned[b_idx] {
                tracks.push(Track {
                    id: next_id,
                    samples: vec![(det.t_ms, *blob)],
                    frame_indices: vec![frame_idx],
                    last_seen_frame: frame_idx,
                    active: true,
                });
                next_id += 1;
            }
        }

        // Expire stale tracks.
        for t in tracks.iter_mut() {
            if t.active && frame_idx.saturating_sub(t.last_seen_frame) > MAX_GAP_FRAMES {
                t.active = false;
            }
        }
    }

    tracks
}

/// How far back in time the velocity estimate reaches.  Time-bounded
/// rather than sample-bounded so it is fps-independent: at 30 fps it
/// spans ~8 samples — long enough to reach *behind* the ~130 ms
/// plateau a track suffers while riding the merged blob of a
/// same-size crossing (whose samples all sit at the merge midpoint
/// and would otherwise flatten the velocity to zero) — while at a
/// 5 fps GIF it degrades to the freshest 2-sample estimate, which is
/// what strongly curved motion like a pendulum bob needs.
const VELOCITY_WINDOW_MS: u32 = 250;

/// Constant-velocity prediction of where a track's blob should be at
/// `t_now`, extrapolated from its trailing samples (v0.7).  A track
/// with a single sample predicts its last observed position.  The
/// timestamps make the extrapolation gap-aware: a track unseen for
/// two frames predicts two frames ahead.  The cost of this model is
/// paid at direction reversals, where the overshoot is ~2× the
/// per-frame motion — still far under the gate at any fps this
/// pipeline meets, and the old measure was off by 1× there anyway.
fn predicted_position(track: &Track, t_now: u32) -> (f32, f32) {
    let n = track.samples.len();
    let (t1, b1) = track.samples[n - 1];
    if n < 2 {
        return (b1.cx, b1.cy);
    }
    // Oldest sample still inside the velocity window.
    let mut j = n - 2;
    while j > 0 && t1.saturating_sub(track.samples[j - 1].0) <= VELOCITY_WINDOW_MS {
        j -= 1;
    }
    let (t0, b0) = track.samples[j];
    let dt = t1.saturating_sub(t0) as f32;
    if dt < 1.0 {
        return (b1.cx, b1.cy);
    }
    let dt_now = t_now.saturating_sub(t1) as f32;
    (
        b1.cx + (b1.cx - b0.cx) / dt * dt_now,
        b1.cy + (b1.cy - b0.cy) / dt * dt_now,
    )
}

fn matching_cost(prev: &Blob, predicted: (f32, f32), curr: &Blob) -> f32 {
    let pos = ((curr.cx - predicted.0).powi(2) + (curr.cy - predicted.1).powi(2)).sqrt();
    let dr = curr.color.r as f32 - prev.color.r as f32;
    let dg = curr.color.g as f32 - prev.color.g as f32;
    let db = curr.color.b as f32 - prev.color.b as f32;
    let col = (dr * dr + dg * dg + db * db).sqrt();
    let rad = (curr.radius - prev.radius).abs();
    let hu = (0..7).map(|i| (curr.hu[i] - prev.hu[i]).abs()).sum::<f32>();
    W_POS * pos + W_COL * col + W_RAD * rad + W_HU * hu
}

/// v0.5.1 stationarity filter: true when every sample sits within
/// `STATIONARY_MAX_DEV_PX` of the track's mean position.
fn is_stationary(blobs: &[(u32, Blob)]) -> bool {
    if blobs.len() < 2 {
        return true;
    }
    let n = blobs.len() as f32;
    let mean_cx = blobs.iter().map(|(_, b)| b.cx).sum::<f32>() / n;
    let mean_cy = blobs.iter().map(|(_, b)| b.cy).sum::<f32>() / n;
    blobs.iter().all(|(_, b)| {
        ((b.cx - mean_cx).powi(2) + (b.cy - mean_cy).powi(2)).sqrt() <= STATIONARY_MAX_DEV_PX
    })
}

// --------------------------------------------------------------------
// Temporal-aware trajectory simplification (v0.2 — unchanged).
// --------------------------------------------------------------------

pub fn simplify_trajectory(blobs: &[(u32, Blob)], tol: f32) -> Vec<usize> {
    let n = blobs.len();
    if n < 3 {
        return (0..n).collect();
    }
    let times: Vec<f32> = blobs.iter().map(|(t, _)| *t as f32).collect();
    let xs: Vec<f32> = blobs.iter().map(|(_, b)| b.cx).collect();
    let ys: Vec<f32> = blobs.iter().map(|(_, b)| b.cy).collect();

    let kx = rdp_1d(&times, &xs, tol);
    let ky = rdp_1d(&times, &ys, tol);

    let mut combined: Vec<usize> = kx.into_iter().chain(ky.into_iter()).collect();
    combined.sort();
    combined.dedup();
    combined
}

fn rdp_1d(times: &[f32], values: &[f32], tol: f32) -> Vec<usize> {
    debug_assert_eq!(times.len(), values.len());
    let n = values.len();
    if n < 3 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    rdp_1d_recurse(times, values, 0, n - 1, tol, &mut keep);
    (0..n).filter(|&i| keep[i]).collect()
}

fn rdp_1d_recurse(
    times: &[f32],
    values: &[f32],
    start: usize,
    end: usize,
    tol: f32,
    keep: &mut [bool],
) {
    if end <= start + 1 {
        return;
    }
    let t0 = times[start];
    let v0 = values[start];
    let t1 = times[end];
    let v1 = values[end];
    let dt = t1 - t0;

    let mut max_dev = 0.0_f32;
    let mut max_idx = start;
    for i in (start + 1)..end {
        let dev = if dt.abs() < f32::EPSILON {
            (values[i] - v0).abs()
        } else {
            let expected = v0 + (times[i] - t0) / dt * (v1 - v0);
            (values[i] - expected).abs()
        };
        if dev > max_dev {
            max_dev = dev;
            max_idx = i;
        }
    }

    if max_dev > tol {
        keep[max_idx] = true;
        rdp_1d_recurse(times, values, start, max_idx, tol, keep);
        rdp_1d_recurse(times, values, max_idx, end, tol, keep);
    }
}

// --------------------------------------------------------------------
// Easing classification (v0.2 — unchanged).
// --------------------------------------------------------------------

fn classify_easing(blobs: &[(u32, Blob)], keep: &[usize]) -> Easing {
    const CANDIDATES: [Easing; 4] = [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
    ];
    let mut errors = [0.0_f64; 4];
    let mut samples: usize = 0;

    for w in keep.windows(2) {
        let i0 = w[0];
        let i1 = w[1];
        if i1 <= i0 + 1 {
            continue;
        }
        let (t0, b0) = blobs[i0];
        let (t1, b1) = blobs[i1];
        let dt = t1.saturating_sub(t0) as f32;
        if dt < 1.0 {
            continue;
        }
        let dx = b1.cx - b0.cx;
        let dy = b1.cy - b0.cy;
        if dx.abs() < 1.0 && dy.abs() < 1.0 {
            continue;
        }

        for j in (i0 + 1)..i1 {
            let (t, b) = blobs[j];
            let t_norm = ((t.saturating_sub(t0)) as f32) / dt;
            for (k, e) in CANDIDATES.iter().enumerate() {
                let p = e.apply(t_norm);
                let ex = b0.cx + p * dx;
                let ey = b0.cy + p * dy;
                let err = ((b.cx - ex).powi(2) + (b.cy - ey).powi(2)) as f64;
                errors[k] += err;
            }
            samples += 1;
        }
    }

    if samples == 0 {
        return Easing::Linear;
    }

    let (mut best, mut best_err) = (0usize, errors[0]);
    for k in 1..CANDIDATES.len() {
        if errors[k] < best_err {
            best = k;
            best_err = errors[k];
        }
    }

    if best == 0 {
        return Easing::Linear;
    }
    let linear_err = errors[0];
    if linear_err > 0.0 && best_err <= linear_err * EASING_IMPROVEMENT_RATIO {
        CANDIDATES[best]
    } else {
        Easing::Linear
    }
}

/// Unsigned shoelace area of a closed polygon.
fn shoelace_area(pts: &[(f32, f32)]) -> f32 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut acc = 0.0f64;
    for i in 0..n {
        let (x0, y0) = (pts[i].0 as f64, pts[i].1 as f64);
        let (x1, y1) = (pts[(i + 1) % n].0 as f64, pts[(i + 1) % n].1 as f64);
        acc += x0 * y1 - x1 * y0;
    }
    (acc.abs() * 0.5) as f32
}

// --------------------------------------------------------------------
// Stats helpers
// --------------------------------------------------------------------

fn median_f32(v: &[f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f32> = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s[s.len() / 2]
}

fn median_color(it: impl Iterator<Item = Color>) -> Color {
    let cs: Vec<Color> = it.collect();
    if cs.is_empty() {
        return Color::rgb(0, 0, 0);
    }
    let mut rs: Vec<u8> = cs.iter().map(|c| c.r).collect();
    let mut gs: Vec<u8> = cs.iter().map(|c| c.g).collect();
    let mut bs: Vec<u8> = cs.iter().map(|c| c.b).collect();
    rs.sort();
    gs.sort();
    bs.sort();
    Color::rgb(rs[rs.len() / 2], gs[gs.len() / 2], bs[bs.len() / 2])
}

/// Positions are written to a tenth of a pixel (v1.4). Whole pixels
/// misplaced a 3 px border's half-pixel centreline by ½ px on every frame,
/// a large squared error along a high-contrast edge.
fn round_pos(x: f32) -> f32 {
    (x * 10.0).round() / 10.0
}

fn round_to_int(x: f32) -> f32 {
    x.round()
}

// --------------------------------------------------------------------
// 2D RDP (kept for future contour simplification work).
// --------------------------------------------------------------------

pub fn rdp(points: &[(f32, f32)], tolerance: f32) -> Vec<usize> {
    let n = points.len();
    if n < 3 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    rdp_recurse(points, 0, n - 1, tolerance, &mut keep);
    (0..n).filter(|&i| keep[i]).collect()
}

fn rdp_recurse(points: &[(f32, f32)], start: usize, end: usize, tol: f32, keep: &mut [bool]) {
    if end <= start + 1 {
        return;
    }
    let mut max_dist = 0.0f32;
    let mut max_idx = start;
    for i in (start + 1)..end {
        let d = perpendicular_distance(points[i], points[start], points[end]);
        if d > max_dist {
            max_dist = d;
            max_idx = i;
        }
    }
    if max_dist > tol {
        keep[max_idx] = true;
        rdp_recurse(points, start, max_idx, tol, keep);
        rdp_recurse(points, max_idx, end, tol, keep);
    }
}

fn perpendicular_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len_sq = dx * dx + dy * dy;
    if len_sq < f32::EPSILON {
        let ex = p.0 - a.0;
        let ey = p.1 - a.1;
        return (ex * ex + ey * ey).sqrt();
    }
    let num = (dy * p.0 - dx * p.1 + b.0 * a.1 - b.1 * a.0).abs();
    num / len_sq.sqrt()
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn blob_at(cx: f32, cy: f32) -> Blob {
        blob_full(cx, cy, Color::rgb(0, 0, 0))
    }

    fn blob_full(cx: f32, cy: f32, color: Color) -> Blob {
        Blob {
            cx,
            cy,
            rx: 24.0,
            ry: 24.0,
            radius: 24.0,
            fill_ratio: 0.785,
            kind: ShapeKind::Ellipse,
            color,
            area: 1800,
            hu: [0.0; 7],
            theta_deg: 0.0,
            var_major: 144.0,
            var_minor: 144.0,
            angle_deg: 0.0,
            soft_var_x: 144.0,
            soft_var_y: 144.0,
            soft_w: 48.0,
            soft_h: 48.0,
            soft_area: 1800.0,
            fit_iou: 0.97,
            hole_area: 0,
            filled_iou_rect: 0.785,
            filled_iou_ellipse: 0.97,
        }
    }

    #[test]
    fn rdp_keeps_corners_only() {
        let mut pts = Vec::new();
        for x in 0..=10 {
            pts.push((x as f32, 0.0));
        }
        for y in 1..=10 {
            pts.push((10.0, y as f32));
        }
        let kept = rdp(&pts, 0.5);
        assert_eq!(kept.first().copied(), Some(0));
        assert_eq!(kept.last().copied(), Some(pts.len() - 1));
        assert!(kept.contains(&10), "kept indices: {kept:?}");
        assert!(kept.len() <= 4, "expected ~3 keyframes, got {kept:?}");
    }

    #[test]
    fn temporal_rdp_recovers_collinear_apex() {
        let mut blobs = Vec::new();
        for i in 0..=90u32 {
            let t = i * 33;
            let frac = if i <= 45 {
                i as f32 / 45.0
            } else {
                (90 - i) as f32 / 45.0
            };
            let x = 60.0 + frac * 360.0;
            blobs.push((t, blob_at(x, 160.0)));
        }
        let keep = simplify_trajectory(&blobs, 2.0);
        let last = blobs.len() - 1;
        assert!(keep.contains(&0), "missing start: {keep:?}");
        assert!(keep.contains(&last), "missing end: {keep:?}");
        assert!(
            keep.iter().any(|&i| (43..=47).contains(&i)),
            "missing apex near i=45: {keep:?}"
        );
    }

    #[test]
    fn easing_classifier_returns_linear_for_linear_motion() {
        let mut blobs = Vec::new();
        for i in 0..=90u32 {
            let t = i * 33;
            let x = 60.0 + (i as f32 / 90.0) * 360.0;
            blobs.push((t, blob_at(x, 160.0)));
        }
        let keep = vec![0usize, 90];
        assert_eq!(classify_easing(&blobs, &keep), Easing::Linear);
    }

    #[test]
    fn easing_classifier_picks_ease_in_out_when_data_demands_it() {
        let mut blobs = Vec::new();
        for i in 0..=90u32 {
            let t = i * 33;
            let t_norm = i as f32 / 90.0;
            let p = Easing::EaseInOut.apply(t_norm);
            let x = 60.0 + p * 360.0;
            blobs.push((t, blob_at(x, 160.0)));
        }
        let keep = vec![0usize, 90];
        let easing = classify_easing(&blobs, &keep);
        assert!(
            matches!(easing, Easing::EaseInOut | Easing::EaseIn | Easing::EaseOut),
            "expected an eased classification, got {easing:?}"
        );
    }

    /// Two distinct-coloured balls moving in opposite directions
    /// across the same y must produce two stable tracks.
    #[test]
    fn tracker_separates_two_balls_by_colour() {
        let red = Color::rgb(0xe9, 0x45, 0x60);
        let yellow = Color::rgb(0xff, 0xd1, 0x66);

        let mut detections: Vec<Detection> = Vec::new();
        for i in 0..=60u32 {
            let t = i * 33;
            let frac = i as f32 / 60.0;
            let xr = 60.0 + frac * 480.0;
            let xy = 540.0 - frac * 480.0;
            detections.push(Detection {
                frame_index: i as usize,
                t_ms: t,
                bg: Color::rgb(0x1a, 0x1a, 0x2e),
                blobs: vec![blob_full(xr, 100.0, red), blob_full(xy, 260.0, yellow)],
                segmentation: crate::detect::Segmentation::Components,
            });
        }

        let tracks = build_tracks(&detections);
        let long: Vec<&Track> = tracks.iter().filter(|t| t.samples.len() >= 30).collect();
        assert_eq!(
            long.len(),
            2,
            "expected exactly 2 long tracks, got {} ({tracks:#?})",
            long.len()
        );

        let red_track = long
            .iter()
            .find(|t| t.samples[0].1.color == red)
            .expect("no red track");
        let yellow_track = long
            .iter()
            .find(|t| t.samples[0].1.color == yellow)
            .expect("no yellow track");

        // Red moves 60 → 540, yellow moves 540 → 60. End positions confirm.
        let (_, red_last) = red_track.samples.last().unwrap();
        let (_, yellow_last) = yellow_track.samples.last().unwrap();
        assert!(
            red_last.cx > 400.0,
            "red did not advance: cx={}",
            red_last.cx
        );
        assert!(
            yellow_last.cx < 200.0,
            "yellow did not advance: cx={}",
            yellow_last.cx
        );
    }

    /// v0.7 — two SAME-coloured, same-sized balls crossing on the same
    /// row, with a single merged detection at the crossing frame.
    /// Colour, radius and Hu are all identical, so only the
    /// constant-velocity prediction can keep the identities straight:
    /// the track moving right must exit right, the one moving left
    /// must exit left — not bounce.
    #[test]
    fn tracker_keeps_identities_through_same_colour_crossing() {
        let red = Color::rgb(0xe9, 0x45, 0x60);
        let mut detections: Vec<Detection> = Vec::new();
        for i in 0..=60u32 {
            let t = i * 33;
            let xa = 60.0 + 8.0 * i as f32; // left ball → right
            let xb = 540.0 - 8.0 * i as f32; // right ball → left
            let blobs = if (xa - xb).abs() < 16.0 {
                // Balls overlap: detection sees one merged blob.
                vec![blob_full((xa + xb) / 2.0, 180.0, red)]
            } else {
                vec![blob_full(xa, 180.0, red), blob_full(xb, 180.0, red)]
            };
            detections.push(Detection {
                frame_index: i as usize,
                t_ms: t,
                bg: Color::rgb(0x1a, 0x1a, 0x2e),
                blobs,
                segmentation: crate::detect::Segmentation::Components,
            });
        }

        let tracks = build_tracks(&detections);
        let long: Vec<&Track> = tracks.iter().filter(|t| t.samples.len() >= 40).collect();
        assert_eq!(
            long.len(),
            2,
            "expected 2 long tracks through the crossing, got {} ({tracks:#?})",
            long.len()
        );

        for t in long {
            let x0 = t.samples.first().unwrap().1.cx;
            let x1 = t.samples.last().unwrap().1.cx;
            let travelled = x1 - x0;
            if x0 < 300.0 {
                assert!(
                    travelled > 300.0,
                    "left-starting track bounced instead of crossing: {x0} → {x1}"
                );
            } else {
                assert!(
                    travelled < -300.0,
                    "right-starting track bounced instead of crossing: {x0} → {x1}"
                );
            }
        }
    }

    /// v0.7 — a fast mover whose per-frame displacement exceeds the
    /// cost gate must stay a single track: the prediction lands on the
    /// blob even across a multi-frame detection drop-out.
    #[test]
    fn velocity_prediction_survives_fast_motion_and_gaps() {
        let red = Color::rgb(0xe9, 0x45, 0x60);
        let mut detections: Vec<Detection> = Vec::new();
        for i in 0..=40u32 {
            let t = i * 33;
            // 100 px/frame — far beyond COST_GATE for a static matcher.
            let blobs = if (18..=19).contains(&i) {
                Vec::new() // two-frame detection drop-out mid-flight
            } else {
                vec![blob_full(60.0 + 100.0 * i as f32, 180.0, red)]
            };
            detections.push(Detection {
                frame_index: i as usize,
                t_ms: t,
                bg: Color::rgb(0x1a, 0x1a, 0x2e),
                blobs,
                segmentation: crate::detect::Segmentation::Components,
            });
        }

        let tracks = build_tracks(&detections);
        let long: Vec<&Track> = tracks.iter().filter(|t| t.samples.len() >= 10).collect();
        assert_eq!(
            long.len(),
            1,
            "fast mover fragmented into {} tracks ({tracks:#?})",
            long.len()
        );
        assert!(
            long[0].samples.len() >= 39,
            "track lost samples: {}",
            long[0].samples.len()
        );
    }

    fn track_from(samples: Vec<(u32, Blob)>) -> Track {
        let last = samples.len().saturating_sub(1);
        let frame_indices = (0..samples.len()).collect();
        Track {
            id: 0,
            samples,
            frame_indices,
            last_seen_frame: last,
            active: true,
        }
    }

    /// v0.5.1 — a static label whose centroid flickers ±2 px from
    /// frame to frame (GIF palette flips, H.264 chroma noise) must be
    /// emitted as a static shape anchored at its median position, not
    /// as a keyframe salad.  The old gate measured deviation from the
    /// *first* sample, so this exact input produced an animate block.
    #[test]
    fn stationary_jittering_track_emits_no_animate_block() {
        let jitter = [0.0f32, 1.5, -1.5, 2.0, -2.0, 1.0, -1.0, 0.5];
        let samples: Vec<(u32, Blob)> = (0..40u32)
            .map(|i| {
                let j = jitter[(i as usize) % jitter.len()];
                (i * 33, blob_at(100.0 + j, 80.0 + 0.5 * j))
            })
            .collect();

        let mut statements = Vec::new();
        emit_shape(&mut statements, "shape_0", &track_from(samples), &[], &[], &Tolerances::default(), None);

        assert!(
            !statements
                .iter()
                .any(|s| matches!(s, Statement::Animate(_))),
            "stationary track produced an animate block: {statements:#?}"
        );

        let shape = statements
            .iter()
            .find_map(|s| match s {
                Statement::Let { shape, .. } => Some(shape.clone()),
                _ => None,
            })
            .expect("no let statement emitted");
        match shape {
            Shape::Ellipse { cx, cy, .. } => {
                assert!(
                    (cx - 100.0).abs() <= 1.0 && (cy - 80.0).abs() <= 1.0,
                    "static anchor should be the median position, got ({cx}, {cy})"
                );
            }
            other => panic!("expected an ellipse, got {other:?}"),
        }
    }

    /// Real motion must keep its animate block — the stationarity
    /// filter only suppresses sub-jitter wobble.
    #[test]
    fn moving_track_keeps_its_animate_block() {
        let samples: Vec<(u32, Blob)> = (0..=60u32)
            .map(|i| (i * 33, blob_at(60.0 + (i as f32 / 60.0) * 480.0, 100.0)))
            .collect();

        let mut statements = Vec::new();
        emit_shape(&mut statements, "shape_0", &track_from(samples), &[], &[], &Tolerances::default(), None);

        assert!(
            statements
                .iter()
                .any(|s| matches!(s, Statement::Animate(_))),
            "moving track lost its animate block"
        );
    }
}
