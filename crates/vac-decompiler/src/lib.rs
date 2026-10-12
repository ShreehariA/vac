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
}

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
    }))
}

/// Detect, track and emit, on frames already prepared.
pub fn finish(prepared: &Prepared, segmentation: Segmentation, quality: f32, plate: Plate) -> Document {
    let tol = track::Tolerances::from_quality(quality);
    // A raster plate is the raw temporal median — the scene with its
    // movers removed — as a JPEG, when the clip has one.
    let raster = match plate {
        Plate::Raster => prepared
            .ground
            .as_ref()
            .and_then(|(m, _)| raster_data_uri(m, tol.raster_quality)),
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

/// A frame as a `data:image/jpeg;base64,` URI at the given JPEG quality.
fn raster_data_uri(f: &frames::VideoFrame, quality: u8) -> Option<String> {
    let rgb: Vec<u8> = f.pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality)
        .encode(&rgb, f.width, f.height, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(format!("data:image/jpeg;base64,{}", vac_format::base64::encode(&buf)))
}
