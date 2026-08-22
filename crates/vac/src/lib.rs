//! # VAC — Video As Code
//!
//! Top-level facade. The whole project is exposed through this one crate so
//! callers don't have to reach into `vac-format` / `vac-compiler` /
//! `vac-decompiler` individually.
//!
//! The API is shaped like `pandas` on purpose:
//!
//! ```no_run
//! use vac::DocumentExt;
//!
//! // .vac → video (extension picks the format: .mp4 default, .gif still works)
//! let doc = vac::read("ball.vac")?;
//! doc.to_video("any_name.mp4")?;
//!
//! // video → .vac
//! let doc = vac::read_video("any_name.mp4")?;
//! doc.to_vac("decompiled.vac")?;
//! # Ok::<_, vac::Error>(())
//! ```
//!
//! Module-level functions (`vac::read`, `vac::read_video`, `vac::write_video`,
//! `vac::write_vac`) mirror `pd.read_csv` / `pd.read_parquet` style.
//! Method-style is provided by [`DocumentExt`]:
//!
//! - [`Document::to_video`](DocumentExt::to_video)
//! - [`Document::to_vac`](DocumentExt::to_vac)
//!
//! The output format is selected by the destination's file extension.
//! Two backends are available:
//!
//! - `.mp4` — H.264/MP4 via the system `ffmpeg` binary (subprocess).
//!   Exact FPS, smaller files. **Default from v0.4 on.**
//! - `.gif` — pure-Rust via `image::codecs::gif`. No system deps but
//!   centisecond delay quantisation drifts the round-trip FPS.
//!
//! Choose any filename you like; VAC just looks at the extension.

use std::path::Path;

// Re-export the AST and friends so callers only need `use vac::*`.
pub use vac_format::{
    Animation, Canvas, Color, Document, Easing, Keyframe, ParseError, PathSeg, Point, Property,
    Scene, Shape, Statement, Transform, Value,
};

// Re-export the underlying error types so callers can pattern-match if they
// want to, without taking direct dependencies on the sub-crates.
pub use vac_compiler::CompileError;
pub use vac_decompiler::{DecompileError, Options as DecompileOptions};

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(#[from] ParseError),
    #[error("compile: {0}")]
    Compile(#[from] CompileError),
    #[error("decompile: {0}")]
    Decompile(#[from] DecompileError),
    #[error("measure: {0}")]
    Measure(#[from] MetricError),
    #[error("unsupported video format `.{ext}` (supported: mp4, gif)")]
    UnsupportedVideoFormat { ext: String },
    #[error("path has no extension; can't decide format: {path}")]
    MissingExtension { path: String },
}

pub type Result<T> = std::result::Result<T, Error>;

// --------------------------------------------------------------------
// Module-level (pandas pd.* style)
// --------------------------------------------------------------------

/// Read and parse a `.vac` source file.
///
/// Mirrors `pandas.read_csv`. Path can be anything you like; the
/// content must be valid VAC source.
pub fn read<P: AsRef<Path>>(path: P) -> Result<Document> {
    let text = std::fs::read_to_string(path.as_ref())?;
    Ok(vac_format::parse(&text)?)
}

/// Parse a `.vac` source string in memory.
pub fn parse(src: &str) -> Result<Document> {
    Ok(vac_format::parse(src)?)
}

/// Decompile a video file back into a [`Document`].
///
/// Format is dispatched by extension. Supported: `.mp4`, `.gif`.
pub fn read_video<P: AsRef<Path>>(path: P) -> Result<Document> {
    read_video_with(path, &DecompileOptions::default())
}

/// [`read_video`] with explicit decompiler [`DecompileOptions`].
pub fn read_video_with<P: AsRef<Path>>(path: P, opts: &DecompileOptions) -> Result<Document> {
    let path = path.as_ref();
    match video_format(path)? {
        VideoFormat::Mp4 => Ok(vac_decompiler::decompile_mp4_with(path, opts)?),
        VideoFormat::Gif => Ok(vac_decompiler::decompile_gif_with(path, opts)?),
    }
}

/// Render a [`Document`] to a video file on disk.
///
/// Format is dispatched by extension. Supported: `.mp4`, `.gif`.
/// MP4 needs the system `ffmpeg` binary on PATH at runtime.
pub fn write_video<P: AsRef<Path>>(doc: &Document, path: P) -> Result<()> {
    let path = path.as_ref();
    match video_format(path)? {
        VideoFormat::Mp4 => Ok(vac_compiler::compile_to_mp4(doc, path)?),
        VideoFormat::Gif => Ok(vac_compiler::compile_to_gif(doc, path)?),
    }
}

/// Serialise a [`Document`] back to `.vac` source on disk.
pub fn write_vac<P: AsRef<Path>>(doc: &Document, path: P) -> Result<()> {
    let text = vac_format::write(doc);
    std::fs::write(path.as_ref(), text)?;
    Ok(())
}

/// Serialise a [`Document`] to a `.vac` source string in memory.
pub fn to_string(doc: &Document) -> String {
    vac_format::write(doc)
}

// --------------------------------------------------------------------
// Method-style (pandas df.to_csv style)
// --------------------------------------------------------------------

/// Extension trait that adds method-style writers to [`Document`],
/// matching `pandas`' `df.to_csv("name.csv")` ergonomics.
///
/// Bring it into scope with `use vac::DocumentExt;` to use them.
pub trait DocumentExt {
    /// Render this document to a video file. Format chosen by extension.
    fn to_video<P: AsRef<Path>>(&self, path: P) -> Result<()>;

    /// Write this document as `.vac` source to disk.
    fn to_vac<P: AsRef<Path>>(&self, path: P) -> Result<()>;

    /// Serialise this document to a `.vac` source string.
    fn to_vac_string(&self) -> String;
}

impl DocumentExt for Document {
    fn to_video<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        write_video(self, path)
    }
    fn to_vac<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        write_vac(self, path)
    }
    fn to_vac_string(&self) -> String {
        vac_format::write(self)
    }
}

// --------------------------------------------------------------------
// Format dispatch
// --------------------------------------------------------------------

enum VideoFormat {
    Mp4,
    Gif,
}

fn video_format(path: &Path) -> Result<VideoFormat> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::MissingExtension {
            path: path.display().to_string(),
        })?;
    match ext.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => Ok(VideoFormat::Mp4),
        "gif" => Ok(VideoFormat::Gif),
        other => Err(Error::UnsupportedVideoFormat {
            ext: other.to_string(),
        }),
    }
}

// --------------------------------------------------------------------
// Measurement (v0.9)
// --------------------------------------------------------------------
//
// Every VAC version through v0.8 measured itself with proxies — shape
// counts, keyframe counts, `.vac` byte size.  Those describe the *rate*
// side of the tradeoff and say nothing about the *distortion* side.
// These entry points supply the missing axis.

pub use vac_compiler::render::RenderedFrame;
pub use vac_decompiler::frames::VideoFrame;
pub use vac_metrics::{CompareOptions, Fidelity, FrameScore, MetricError, Rgb};

/// Rasterise a document to frames in memory, with no encoder involved.
pub fn render(doc: &Document) -> Result<Vec<RenderedFrame>> {
    Ok(vac_compiler::render::render_document(doc)?)
}

/// Decode a video's frames without running the decompiler on them.
pub fn load_video_frames<P: AsRef<Path>>(path: P) -> Result<Vec<VideoFrame>> {
    let path = path.as_ref();
    Ok(match video_format(path)? {
        VideoFormat::Mp4 => vac_decompiler::frames_mp4::load_mp4(path)?,
        VideoFormat::Gif => vac_decompiler::frames::load_gif(path)?,
    })
}

/// Run the decompiler on frames already in memory.
///
/// This is the codec-free path: rendering a `.vac` straight to frames and
/// feeding them back through here isolates the decompiler's own error
/// from GIF palette quantisation and H.264 lossiness.
pub fn decompile_frames(
    frames: Vec<VideoFrame>,
    opts: &DecompileOptions,
) -> Result<Document> {
    Ok(vac_decompiler::decompile_frames(frames, opts)?)
}

/// Reinterpret rendered frames as decoder frames. Both sides are
/// non-premultiplied RGBA8, so this only attaches timing.
///
/// Timing uses **rational stamping**, matching the renderer exactly:
/// `vac_compiler` places frame `i` at `t = i * 1000 / fps` with integer
/// division, giving 0, 33, 66, **100**, 133 … for 30 fps. A flat
/// `delay = 1000/fps = 33` would look equivalent and is not: it drifts
/// 0.33 ms per frame, reaching 29 ms — a whole frame — by the end of a
/// 3 s clip, so the recovered duration comes back 2970 ms instead of
/// 3000 ms and the re-render emits 89 frames where the source had 90.
/// The harness would then be measuring a defect it introduced itself.
pub fn rendered_to_video_frames(frames: &[RenderedFrame], fps: u32) -> Vec<VideoFrame> {
    let fps = fps.max(1) as u64;
    let stamp = |i: u64| (i * 1000) / fps;
    frames
        .iter()
        .enumerate()
        .map(|(i, f)| VideoFrame {
            width: f.width,
            height: f.height,
            pixels: f.rgba.clone(),
            delay_ms: (stamp(i as u64 + 1) - stamp(i as u64)) as u32,
        })
        .collect()
}

/// Measure how much of `original` survived into `doc`.
///
/// Renders `doc`, aligns it onto the source timeline, and scores each
/// source frame against the reconstruction that should be showing at
/// that instant. The ink mask is measured against the **source** frame's
/// background: judging the reconstruction on its own estimated
/// background would be circular precisely when background estimation is
/// the thing under suspicion.
pub fn measure(original: &[VideoFrame], doc: &Document) -> Result<Fidelity> {
    measure_with(original, doc, &CompareOptions::default())
}

/// [`measure`] with explicit [`CompareOptions`].
pub fn measure_with(
    original: &[VideoFrame],
    doc: &Document,
    opts: &CompareOptions,
) -> Result<Fidelity> {
    let rendered = render(doc)?;
    let delays: Vec<u32> = original.iter().map(|f| f.delay_ms).collect();
    let times = vac_metrics::timeline_from_delays(&delays);
    let idx = vac_metrics::align(&times, rendered.len(), doc.canvas.fps);

    let src: Vec<vac_metrics::FrameRef> = original
        .iter()
        .map(|f| vac_metrics::FrameRef::new(f.width, f.height, &f.pixels))
        .collect();
    let recon: Vec<vac_metrics::FrameRef> = idx
        .iter()
        .map(|&i| {
            let r = &rendered[i];
            vac_metrics::FrameRef::new(r.width, r.height, &r.rgba)
        })
        .collect();
    // The ink reference is measured from the source frames themselves,
    // by `vac-metrics`, and deliberately *not* by calling the
    // decompiler's own `estimate_background`. If the metric shared the
    // pipeline's background rule, tuning that rule would silently move
    // every committed baseline and "the decompiler improved" would be
    // indistinguishable from "the ruler moved".
    let bg = vac_metrics::modal_background(&src);
    let backgrounds: Vec<Rgb> = vec![bg; src.len()];

    Ok(vac_metrics::compare(&src, &recon, &backgrounds, opts)?)
}

/// Rate *and* distortion for one clip — everything `vac verify` reports.
#[derive(Debug, Clone)]
pub struct Verification {
    pub fidelity: Fidelity,
    pub document: Document,
    /// Frames decoded from the source.
    pub source_frames: usize,
    /// Size of the emitted `.vac` source, in bytes. The "rate" axis.
    pub vac_bytes: usize,
    pub shapes: usize,
    pub animate_blocks: usize,
    pub keyframes: usize,
}

/// Decompile a video, re-render the result, and score it against the
/// source. The full round trip, measured.
pub fn verify<P: AsRef<Path>>(path: P, opts: &DecompileOptions) -> Result<Verification> {
    let frames = load_video_frames(path)?;
    let source_frames = frames.len();
    let doc = decompile_frames(frames.clone(), opts)?;
    let fidelity = measure(&frames, &doc)?;
    Ok(summarise(doc, fidelity, source_frames))
}

fn summarise(doc: Document, fidelity: Fidelity, source_frames: usize) -> Verification {
    let text = vac_format::write(&doc);
    let mut shapes = 0usize;
    let mut animate_blocks = 0usize;
    let mut keyframes = 0usize;
    for scene in &doc.scenes {
        for st in &scene.statements {
            match st {
                Statement::Let { .. } => shapes += 1,
                Statement::Animate(a) => {
                    animate_blocks += 1;
                    keyframes += a.keyframes.len();
                }
                Statement::Assign { .. } => {}
            }
        }
    }
    Verification {
        fidelity,
        document: doc,
        source_frames,
        vac_bytes: text.len(),
        shapes,
        animate_blocks,
        keyframes,
    }
}
