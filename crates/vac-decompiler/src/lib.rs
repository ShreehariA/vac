//! VAC decompiler — video frames → `.vac` AST.
//!
//! Two front-ends feed the same algorithmic pipeline:
//!
//! - [`decompile_gif`] — pure-Rust via the `image` crate. Centisecond
//!   delay quantisation drifts the recovered FPS / duration.
//! - [`decompile_mp4`] — H.264/MP4 via the system `ffmpeg`/`ffprobe`
//!   binaries (subprocess). Exact FPS, exact accumulated `t_ms`.
//!
//! The pipeline itself is intentionally algorithmic, not classifier-based:
//!
//! 0. **Static-scene layering** (v0.8): a per-pixel temporal median
//!    plate plus a per-pixel agreement count separate the persistent
//!    scene from the moving content, and strip it out of every frame
//!    where it is actually visible. Scene furniture is emitted once as
//!    `static_<n>`; the tracker below never sees it blink.
//! 1. **Background estimate**: sample frame corners → median colour.
//! 2. **Foreground mask**: pixels whose colour distance from the
//!    background exceeds a threshold.
//! 3. **Connected-component labeling** (4-connectivity, two-pass
//!    union-find) → all blobs above the minimum-area threshold.
//! 4. **Per-blob measurements**: centroid, bounding box, interior-only
//!    mean RGB, area, and the seven Hu image-moment invariants.
//! 5. **Cross-frame tracking** (v0.3): lowest-cost-first greedy
//!    bipartite matching between active tracks and the current
//!    frame's detections, scored on position + colour + size + Hu.
//!    Unmatched detections start new tracks; a 3-frame gap tolerance
//!    survives brief occlusion or detection drop-outs.
//! 6. **Per-track simplification**: temporal-aware Ramer-Douglas-Peucker
//!    on `x(t)` and `y(t)` independently → keyframes (v0.2).
//! 7. **Easing classification** (v0.2): SSE comparison against
//!    canonical easing templates between consecutive keyframes.
//! 8. **AST emission**: one `let shape_<n>` + fill + stroke + animate
//!    block per recovered track.
//!
//! Scale/opacity estimation and proper contour tracing build on this
//! same skeleton in later versions.

pub mod camera;
pub mod detect;
pub mod frames;
pub mod frames_mp4;
pub mod gradient;
pub mod layer;
pub mod track;

use std::path::Path;

use vac_format::Document;

/// Knobs for the decompiler. `Default` is what the CLI and the `vac`
/// facade use; the flags exist so a regression can be measured against
/// the previous version's behaviour with the same binary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// Measure and strip the persistent scene before tracking (v0.8).
    /// Off reproduces the v0.7 single-layer pipeline exactly.
    pub static_layer: bool,
    /// How foreground pixels are grouped into blobs (v0.13).
    pub segmentation: Segmentation,
    /// Which rule picks each frame's background colour (v0.13).
    pub background: Background,
    /// The rate–distortion dial, 0–100 (v1.0). 50 reproduces v0.14
    /// exactly; higher keeps more keyframes and finer outlines, lower
    /// fewer and coarser. See [`track::Tolerances`].
    pub quality: f32,
    /// How the static scene is written (v1.6): as vector shapes, or as one
    /// raster `image()` with the moving shapes drawn on top of it.
    pub plate: Plate,
    /// Measure every emitted shape against its own absence and drop the
    /// ones the reconstruction is no better with (v1.7). Like
    /// [`Plate::Auto`], it needs a renderer, so the `vac` facade runs it;
    /// the decompiler on its own ignores it.
    pub audit: bool,
    /// Nudge every shape's size, position and stroke width to wherever the
    /// reconstruction measures best (v1.9). Resolved by the `vac` facade.
    pub refine: bool,
    /// Re-measure every mover's keyframes by registering its own pixels
    /// from frame to frame (v1.11). Resolved by the `vac` facade.
    pub motion: bool,
    /// Find the clip's cuts and decompile each shot as its own scene, when
    /// that measures better (v1.12). Resolved by the `vac` facade.
    pub shots: bool,
    /// Measure a fade to or from black or white, decompile the picture
    /// with it taken out, and draw the fade on top, when that measures
    /// better (v1.13). Resolved by the `vac` facade.
    pub fades: bool,
    /// Draw a shape vectors cannot describe as a sprite of its own pixels
    /// when that measures clearly better (v1.8). Resolved by the `vac`
    /// facade, like [`Options::audit`].
    pub sprites: bool,
}

/// How the static scene is written (v1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Plate {
    /// Background, ground and scenery as vector shapes — VAC's default.
    #[default]
    Vector,
    /// The clip's temporal-median frame as one JPEG `image()`: the escape
    /// hatch for scenery no vector primitive can describe.
    Raster,
    /// The scene under a moving camera (v1.10): every frame warped back by
    /// its measured view into one mosaic, a JPEG `image()` whose
    /// `position()` and `scale()` keyframes are the camera.
    Camera,
    /// Decide by measurement — resolved by the `vac` facade, which keeps
    /// the raster plate only when it reconstructs clearly better. On its
    /// own the decompiler treats it as `Vector`.
    Auto,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            static_layer: true,
            segmentation: Segmentation::Auto,
            background: Background::Auto,
            quality: track::DEFAULT_QUALITY,
            plate: Plate::Auto,
            audit: true,
            refine: true,
            motion: true,
            shots: true,
            fades: true,
            sprites: true,
        }
    }
}

pub use detect::{Background, Segmentation};
pub use track::Tolerances;

#[derive(Debug, thiserror::Error)]
pub enum DecompileError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
    #[error("decompile: {0}")]
    Other(String),
}

/// Decompile an animated GIF into a [`Document`].
pub fn decompile_gif(input: &Path) -> Result<Document, DecompileError> {
    decompile_gif_with(input, &Options::default())
}

/// Decompile an MP4 (any codec ffmpeg can decode) into a [`Document`].
///
/// Requires the system `ffmpeg` and `ffprobe` binaries on PATH.
/// Use [`frames_mp4::ffmpeg_available`] for a runtime probe.
pub fn decompile_mp4(input: &Path) -> Result<Document, DecompileError> {
    decompile_mp4_with(input, &Options::default())
}

/// [`decompile_gif`] with explicit [`Options`].
pub fn decompile_gif_with(input: &Path, opts: &Options) -> Result<Document, DecompileError> {
    decompile_frames(frames::load_gif(input)?, opts)
}

/// [`decompile_mp4`] with explicit [`Options`].
pub fn decompile_mp4_with(input: &Path, opts: &Options) -> Result<Document, DecompileError> {
    decompile_frames(frames_mp4::load_mp4(input)?, opts)
}

/// Run the pipeline on frames already in memory.
///
/// Frames are taken by value because the static-scene pass edits them:
/// stripping the persistent scene in place is what keeps peak memory at
/// one copy of the clip rather than two.
///
/// Public since v0.9 so the pipeline can be measured without a codec in
/// the loop — rendering a `.vac` straight to frames and feeding them back
/// here isolates *the decompiler's own error* from GIF palette
/// quantisation and H.264 lossiness.  See `vac::measure`.
pub fn decompile_frames(
    frames: Vec<frames::VideoFrame>,
    opts: &Options,
) -> Result<Document, DecompileError> {
    if opts.plate == Plate::Raster {
        if let Some(prepared) = prepare_raster(frames.clone(), opts.background)? {
            return Ok(finish(&prepared, opts.segmentation, opts.quality, Plate::Raster));
        }
    }
    if opts.plate == Plate::Camera {
        if let Some(prepared) = prepare_camera(frames.clone(), opts.background)? {
            return Ok(finish(&prepared, opts.segmentation, opts.quality, Plate::Camera));
        }
    }
    let prepared = prepare(frames, opts.static_layer, opts.background)?;
    Ok(finish(&prepared, opts.segmentation, opts.quality, opts.plate))
}

/// The background-dependent half of a decompile: the static layer, the
/// frames with it stripped out, and the raw temporal median the ground
/// pass reads. Every segmentation candidate of the measured search that
/// shares a background rule can share one `Prepared` — read-only — instead
/// of each cloning and stripping the whole clip (on the 220-frame double
/// pendulum, ~400 MB per candidate).
pub struct Prepared {
    frames: Vec<frames::VideoFrame>,
    scene: Option<layer::StaticLayer>,
    ground: Option<(frames::VideoFrame, vac_format::Color)>,
    background: Background,
    /// The camera and the mosaic it sees, for [`Plate::Camera`].
    camera: Option<CameraPlate>,
}

/// What [`Plate::Camera`] draws from: the views, the median mosaic, the
/// same mosaic fitted through the renderer (v1.19), and a few of the shot's
/// frames as they were before the plate was stripped from them, to choose
/// between the two mosaics on.
struct CameraPlate {
    mosaic: camera::Mosaic,
    sharp: camera::Mosaic,
    views: Vec<camera::View>,
    probes: Vec<(usize, frames::VideoFrame)>,
}

/// Frames [`CameraPlate`] keeps to choose between its mosaics on.
const CAMERA_PROBES: usize = 8;
/// What the sharpened mosaic must buy per byte it adds over the median
/// one, in the selection rule's currency (`ln` of the residual ratio), at
/// quality 50, scaled by the dial like every other price — the price a
/// sprite pays (`1e-5`, ~0.04 dB per KB). The Ken Burns clip's sharpened
/// mosaic buys ~0.28 dB per KB; the Mandelbrot zoom's buys nothing.
const MOSAIC_MIN_GAIN_PER_BYTE: f64 = 1e-5;

/// Measure and strip the static layer, and take the ground median.
pub fn prepare(
    mut frames: Vec<frames::VideoFrame>,
    static_layer: bool,
    background: Background,
) -> Result<Prepared, DecompileError> {
    if frames.is_empty() {
        return Err(DecompileError::Other("no frames in input".into()));
    }
    let scene = if static_layer {
        layer::StaticLayer::measure(&frames, background)
    } else {
        None
    };
    // The ground pass reads the raw temporal median, which the static
    // layer already holds when it formed; otherwise it is measured here,
    // before any stripping.
    let ground = if static_layer {
        match &scene {
            Some(l) => Some((l.median_frame(), l.bg)),
            None => layer::median_frame(&frames).map(|m| {
                let bg = detect::background_of(&m, background);
                (m, bg)
            }),
        }
    } else {
        None
    };
    if let Some(l) = &scene {
        l.strip(&mut frames);
    }
    Ok(Prepared {
        frames,
        scene,
        ground,
        background,
        camera: None,
    })
}

/// [`prepare`] for a raster plate (v1.6): the median frame is the plate,
/// and every pixel matching it is stripped from every frame, so detection
/// sees only what moves. `None` when the clip has no median (too short,
/// or frames of different sizes).
pub fn prepare_raster(
    mut frames: Vec<frames::VideoFrame>,
    background: Background,
) -> Result<Option<Prepared>, DecompileError> {
    if frames.is_empty() {
        return Err(DecompileError::Other("no frames in input".into()));
    }
    let Some(median) = layer::median_frame(&frames) else {
        return Ok(None);
    };
    let bg = detect::background_of(&median, background);
    layer::strip_plate(&mut frames, &median, bg);
    Ok(Some(Prepared {
        frames,
        scene: None,
        ground: Some((median, bg)),
        background,
        camera: None,
    }))
}

/// Frames the camera's first look tracks before deciding the camera moves.
/// 12 through v1.24; a credits roll scrolls 35 to 47 px between twelve
/// samples of three seconds, too far for its first step to find, and a
/// look that loses the scroll measures no spread (v1.25: 0.11 at 12, 0.44
/// at 30).
const CAMERA_PROBE_FRAMES: usize = 30;

/// [`prepare`] for a moving camera (v1.10): track every frame's view of
/// the first, build the mosaic they see, and strip from each frame every
/// pixel that shows the mosaic there, so detection sees only what moves
/// against the scene. `None` when the camera does not move — measured
/// first on a dozen frames, so a still clip pays little — or the scene it
/// sees is unreasonably large.
pub fn prepare_camera(
    mut frames: Vec<frames::VideoFrame>,
    background: Background,
) -> Result<Option<Prepared>, DecompileError> {
    if frames.is_empty() {
        return Err(DecompileError::Other("no frames in input".into()));
    }
    let (w, h) = (frames[0].width, frames[0].height);
    if frames.len() < 8 || frames.iter().any(|f| f.width != w || f.height != h) {
        return Ok(None);
    }
    let stride = (frames.len() / CAMERA_PROBE_FRAMES).max(1);
    let probe = camera::track_every(&frames, 0, stride);
    if camera::motion_px(&probe, w, h) < camera::MIN_CAMERA_MOTION_PX {
        return Ok(None);
    }
    // A camera moves the whole picture, not one object in it.
    let sampled: Vec<usize> = (0..frames.len()).step_by(stride).collect();
    if camera::support(&frames, 0, &probe, &sampled) < camera::MIN_CAMERA_SUPPORT
        && camera::spread_support(&frames, &probe, &sampled) < camera::MIN_CAMERA_SPREAD
    {
        return Ok(None);
    }
    let views = camera::track(&frames, 0);
    if camera::motion_px(&views, w, h) < camera::MIN_CAMERA_MOTION_PX {
        return Ok(None);
    }
    let bg = detect::background_of(&frames[0], background);
    // Two passes: frame 0 carries its own movers into the first estimate,
    // the median mosaic does not.
    let Some(first) = camera::mosaic(&frames, &views, [bg.r, bg.g, bg.b], 1.0) else {
        return Ok(None);
    };
    // A camera that never turns by a quarter degree is a zoom or a pan
    // with noise in its angle: re-measured without turns, it is written
    // without `rotate()`.
    let turns = camera::rolls(&views);
    let views: Vec<camera::View> = if turns {
        views
    } else {
        views.into_iter().map(|v| camera::View { angle: 0.0, ..v }).collect()
    };
    let views = camera::refine_views(&frames, &first, &views, turns);
    // One mosaic pixel per reference pixel. A denser mosaic keeps more of
    // a zoom's late detail but was measured a poor trade at the default
    // quality: the Ken Burns clip +0.8 dB for +54% bytes, the Mandelbrot
    // zoom +0.2 dB for twice the bytes.
    let Some(mosaic) = camera::mosaic(&frames, &views, [bg.r, bg.g, bg.b], 1.0) else {
        return Ok(None);
    };
    let sharp = camera::sharpen(&mosaic, &frames, &views);
    // The probes, where the clip has them, are frames the fit never read.
    let n = frames.len();
    let fitted_every = (n / camera::MOSAIC_MAX_SAMPLES).max(1);
    let probes: Vec<(usize, frames::VideoFrame)> = (0..CAMERA_PROBES.min(n))
        .map(|k| {
            let i = k * n / CAMERA_PROBES.min(n);
            let i = if fitted_every > 1 && i % fitted_every == 0 { (i + 1).min(n - 1) } else { i };
            (i, frames[i].clone())
        })
        .collect();
    for (f, v) in frames.iter_mut().zip(&views) {
        let seen = camera::warp(&mosaic, *v, w, h);
        layer::strip_against(f, &seen, bg);
    }
    Ok(Some(Prepared {
        frames,
        scene: None,
        ground: None,
        background,
        camera: Some(CameraPlate {
            mosaic,
            sharp,
            views,
            probes,
        }),
    }))
}

/// Detect, track and emit, on frames already prepared.
pub fn finish(prepared: &Prepared, segmentation: Segmentation, quality: f32, plate: Plate) -> Document {
    let tol = track::Tolerances::from_quality(quality);
    let (w, h) = prepared
        .frames
        .first()
        .map(|f| (f.width as f32, f.height as f32))
        .unwrap_or((0.0, 0.0));
    let raster = match plate {
        // A raster plate is the raw temporal median — the scene with its
        // movers removed — as a JPEG, when the clip has one.
        Plate::Raster => prepared
            .ground
            .as_ref()
            .and_then(|(m, _)| raster_data_uri(m, tol.raster_quality))
            .map(|uri| track::RasterPlate {
                uri,
                rect: (0.0, 0.0, w, h),
                camera: Vec::new(),
            }),
        // Under a moving camera, the mosaic, and the views as keyframes —
        // pinned to a quarter of the keyframe tolerance, because a whole
        // picture misregistered by a pixel costs far more than one shape.
        Plate::Camera => prepared.camera.as_ref().and_then(|cam| {
            let (m, uri) = chosen_mosaic(cam, tol.raster_quality, quality)?;
            let views = &cam.views;
            let mut times = Vec::with_capacity(prepared.frames.len());
            let mut t = 0u32;
            for f in &prepared.frames {
                times.push(t);
                t = t.saturating_add(f.delay_ms);
            }
            let rect = (m.x, m.y, m.w, m.h);
            let keys = camera::keyframes(views, &times, w as u32, h as u32, tol.keyframe_px * 0.125, rect);
            Some(track::RasterPlate {
                uri,
                rect,
                camera: keys
                    .into_iter()
                    .map(|i| {
                        let (pos, scale, angle) = camera::plate_params(views[i], rect);
                        (times[i], pos, scale, angle.to_degrees())
                    })
                    .collect(),
            })
        }),
        Plate::Vector | Plate::Auto => None,
    };
    let detections = detect::detect_all(
        &prepared.frames,
        segmentation,
        prepared.background,
        tol.min_blob_area,
    );
    track::build_document(
        &prepared.frames,
        &detections,
        prepared.scene.as_ref(),
        prepared.ground.as_ref().map(|(m, bg)| (m, *bg)),
        &tol,
        raster,
    )
}

/// The median mosaic or the sharpened one, whichever pays at this quality,
/// and its URI: each is encoded as it will be written, decoded, and read
/// through the camera onto the probe frames. The sharpened one is kept
/// when the residual it removes (`ln` of the ratio) beats
/// [`MOSAIC_MIN_GAIN_PER_BYTE`] for every byte it adds.
fn chosen_mosaic(cam: &CameraPlate, raster_quality: u8, quality: f32) -> Option<(&camera::Mosaic, String)> {
    let residual = |m: &camera::Mosaic, jpeg: &[u8]| -> Option<f64> {
        let decoded = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()?.to_rgba8();
        let seen_from = camera::Mosaic {
            x: m.x,
            y: m.y,
            w: m.w,
            h: m.h,
            image: frames::VideoFrame {
                width: decoded.width(),
                height: decoded.height(),
                pixels: decoded.into_raw(),
                delay_ms: m.image.delay_ms,
            },
            density: m.density,
        };
        let (mut sq, mut count) = (0f64, 0usize);
        for (i, f) in &cam.probes {
            let seen = camera::warp(&seen_from, cam.views[*i], f.width, f.height);
            for (a, b) in seen.chunks_exact(4).zip(f.pixels.chunks_exact(4)) {
                if a[3] == 0 {
                    continue;
                }
                for c in 0..3 {
                    sq += (a[c] as f64 - b[c] as f64).powi(2);
                }
                count += 3;
            }
        }
        Some(sq / count.max(1) as f64)
    };
    let plain = jpeg_bytes(&cam.mosaic.image, raster_quality)?;
    let sharp = jpeg_bytes(&cam.sharp.image, raster_quality)?;
    let uri = |b: &[u8]| format!("data:image/jpeg;base64,{}", vac_format::base64::encode(b));
    let (plain_uri, sharp_uri) = (uri(&plain), uri(&sharp));
    let gain = (residual(&cam.mosaic, &plain)? / residual(&cam.sharp, &sharp)?.max(1e-9)).ln();
    let m = 2f64.powf((50.0 - quality.clamp(0.0, 100.0) as f64) / 25.0);
    let price = MOSAIC_MIN_GAIN_PER_BYTE * m * m * (sharp_uri.len() as f64 - plain_uri.len() as f64).max(0.0);
    Some(if gain > price { (&cam.sharp, sharp_uri) } else { (&cam.mosaic, plain_uri) })
}

/// A frame as a `data:image/jpeg;base64,` URI at the given JPEG quality.
pub fn jpeg_data_uri(f: &frames::VideoFrame, quality: u8) -> Option<String> {
    raster_data_uri(f, quality)
}

fn raster_data_uri(f: &frames::VideoFrame, quality: u8) -> Option<String> {
    let buf = jpeg_bytes(f, quality)?;
    Some(format!("data:image/jpeg;base64,{}", vac_format::base64::encode(&buf)))
}

fn jpeg_bytes(f: &frames::VideoFrame, quality: u8) -> Option<Vec<u8>> {
    let rgb: Vec<u8> = f.pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality)
        .encode(&rgb, f.width, f.height, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(buf)
}
