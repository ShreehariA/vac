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
    Animation, Canvas, Color, Document, Easing, Gradient, Keyframe, ParseError, PathSeg, Point,
    Property, Scene, Shape, Statement, Transform, Value,
};

// Re-export the underlying error types so callers can pattern-match if they
// want to, without taking direct dependencies on the sub-crates.
pub use vac_compiler::CompileError;
pub use vac_decompiler::{Background, DecompileError, Options as DecompileOptions, Plate, Segmentation};

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
    if let Some(long) = decompile_long(path.as_ref(), opts, false, segment_budget())? {
        return Ok(long.document);
    }
    decompile_frames(load_video_frames(path)?, opts)
}

/// MP4s with more pixels than this, over all their frames, are decompiled
/// a segment at a time (v1.16). 150M pixels is 600 MB of RGBA: 3 s of
/// 1080p, 5.4 s of 720p, every corpus clip several times over.
const SEGMENT_MAX_PIXELS: u64 = 150_000_000;

/// The segment budget: [`SEGMENT_MAX_PIXELS`], or `VAC_SEGMENT_PIXELS`
/// when set (to try the segmented path on a short clip).
fn segment_budget() -> u64 {
    std::env::var("VAC_SEGMENT_PIXELS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(SEGMENT_MAX_PIXELS)
}

/// A long clip, decompiled.
struct Long {
    document: Document,
    /// Pooled from every segment's frames, when asked for.
    fidelity: Option<Fidelity>,
    frames: usize,
    first: Chosen,
    audited_out: usize,
    refined: usize,
    moved: usize,
    sprites: usize,
}

/// Decompile a long MP4 a segment at a time (v1.16), or `None` when the clip
/// is short enough to hold whole. Through v1.15 every frame was decoded
/// and held at once, and the search held copies of them: a minute of 720p
/// would not fit. A first pass decodes the clip at thumbnail size and
/// finds its cuts; shots too long for the budget are split into equal
/// segments; then the full-size frames are streamed, and each segment is
/// decompiled by the whole pipeline as soon as its last frame arrives and
/// dropped. The segments' scenes are concatenated, each exactly as long as
/// its frames. Movement across a forced segment boundary restarts there,
/// which is the price of a bounded memory.
fn decompile_long(path: &Path, opts: &DecompileOptions, measured: bool, budget: u64) -> Result<Option<Long>> {
    let is_mp4 = matches!(video_format(path), Ok(VideoFormat::Mp4));
    if !is_mp4 {
        return Ok(None);
    }
    let info = vac_decompiler::frames_mp4::probe_mp4(path)?;
    let per_frame = (info.width as u64 * info.height as u64).max(1);
    match info.frames {
        Some(n) if n * per_frame <= budget => return Ok(None),
        _ => {}
    }
    // The first look, one frame at a time: the clip's frame count, each
    // frame's step from the one before (where the cuts are), and the
    // metric's ink reference — the whole clip's modal colour, which every
    // segment is then measured against, as a clip held whole would be.
    let mut steps: Vec<f64> = Vec::new();
    let mut modal = vac_metrics::ModalBackground::default();
    let mut prev: Option<VideoFrame> = None;
    let mut n = 0usize;
    for f in vac_decompiler::frames_mp4::stream_mp4(path, None)? {
        let f = f?;
        modal.add(&vac_metrics::FrameRef::new(f.width, f.height, &f.pixels));
        if let Some(p) = &prev {
            steps.push(frame_step(p, &f));
        }
        prev = Some(f);
        n += 1;
    }
    drop(prev);
    if n as u64 * per_frame <= budget {
        return Ok(None);
    }
    let ink = modal.finish();
    let max_frames = ((budget / per_frame) as usize).max(MIN_SHOT_FRAMES * 2);
    let mut edges = vec![0usize];
    edges.extend(if opts.shots { cuts_from_steps(&steps) } else { Vec::new() });
    edges.push(n);
    let mut segments: Vec<(usize, usize)> = Vec::new();
    for w in edges.windows(2) {
        let len = w[1] - w[0];
        let parts = len.div_ceil(max_frames);
        for p in 0..parts {
            segments.push((w[0] + len * p / parts, w[0] + len * (p + 1) / parts));
        }
    }
    let fps = ((info.fps_numer as f64 / info.fps_denom.max(1) as f64).round() as u32).max(1);
    let mut scenes: Vec<Scene> = Vec::new();
    let mut scores: Vec<vac_metrics::FrameScore> = Vec::new();
    let (mut first, mut audited_out, mut refined, mut moved, mut sprites) = (None, 0, 0, 0, 0);
    let mut stream = vac_decompiler::frames_mp4::stream_mp4(path, None)?;
    for &(a, b) in &segments {
        let mut shot: Vec<VideoFrame> = Vec::with_capacity(b - a);
        for _ in a..b {
            match stream.next() {
                Some(f) => shot.push(f?),
                None => break,
            }
        }
        if shot.is_empty() {
            break;
        }
        let chosen = decompile_chosen(&shot, opts)?;
        let mut doc = chosen.document.clone();
        if doc.scenes.len() == 1 {
            // ceil(frames * 1000 / fps): exactly this segment's frames.
            doc.scenes[0].duration_ms = ((shot.len() as u64 * 1000).div_ceil(fps as u64)) as u32;
        }
        if measured {
            let fid = measure_against(&shot, &doc, ink, &CompareOptions::default())?;
            scores.extend(fid.frames);
        }
        audited_out += chosen.audited_out;
        refined += chosen.refined;
        moved += chosen.moved;
        sprites += chosen.sprites;
        scenes.extend(doc.scenes);
        if first.is_none() {
            first = Some(chosen);
        }
    }
    for (k, scene) in scenes.iter_mut().enumerate() {
        scene.name = format!("shot_{}", k + 1);
    }
    let Some(first) = first else {
        return Err(Error::Decompile(DecompileError::Other("no frames decoded from MP4".into())));
    };
    let document = Document {
        canvas: Canvas {
            width: info.width,
            height: info.height,
            fps,
        },
        scenes,
    };
    let frames = scores.len().max(n);
    Ok(Some(Long {
        document,
        fidelity: if measured { Some(vac_metrics::aggregate(scores)?) } else { None },
        frames,
        first,
        audited_out,
        refined,
        moved,
        sprites,
    }))
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
pub use vac_metrics::{is_ink, CompareOptions, Fidelity, FrameScore, MetricError, Rgb};

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
    Ok(decompile_chosen(&frames, opts)?.document)
}

/// The outcome of a decompile, with what measurement decided.
struct Chosen {
    document: Document,
    /// The segmentation actually used.
    segmentation: Segmentation,
    /// The background rule actually used.
    background: Background,
    /// How the static scene was written.
    plate: Plate,
    /// Whether the static layer (v0.8) was used — measured, not assumed,
    /// since v1.20.
    static_layer: bool,
    /// The chosen candidate's fidelity, when `Auto` had to measure it.
    fidelity: Option<Fidelity>,
    /// Shapes the audit (v1.7) measured and took out.
    audited_out: usize,
    /// Shapes whose geometry the refinement (v1.9) moved.
    refined: usize,
    /// Shapes whose motion was re-measured from their pixels (v1.11).
    moved: usize,
    /// Shapes drawn as sprites of their own pixels (v1.8).
    sprites: usize,
}

/// Every `Auto` knob is resolved here, because this is the layer that
/// has a renderer. Each combination of the undecided knobs is decompiled,
/// rendered, and scored against the source, and the candidate that
/// reconstructs best is kept — the way a video encoder makes a
/// rate–distortion mode decision against the frames it is encoding. No
/// content classifier decides; the reconstruction does.
fn decompile_chosen(frames: &[VideoFrame], opts: &DecompileOptions) -> Result<Chosen> {
    let fades = if opts.fades { fade_track(frames) } else { Vec::new() };
    if fades.is_empty() {
        return decompile_cuts(frames, opts);
    }
    // v1.13: decompile the clip as it is and with each reading of its fade
    // taken out, side by side, and keep whichever reconstructs the frames
    // as they are best.
    let (plain, faded): (Result<Chosen>, Vec<Result<Chosen>>) = std::thread::scope(|scope| {
        let plain = scope.spawn(|| decompile_cuts(frames, opts));
        let faded: Vec<_> = fades
            .iter()
            .map(|fade| {
                scope.spawn(move || -> Result<Chosen> {
                    let unfaded = unfade(frames, fade);
                    let mut c = decompile_cuts(&unfaded, opts)?;
                    let fitted = fit_fade(frames, &c.document, fade)?;
                    add_fade(&mut c.document, &fitted, opts.quality);
                    c.fidelity = Some(measure(frames, &c.document)?);
                    Ok(c)
                })
            })
            .collect();
        (
            plain.join().expect("decompile panicked"),
            faded.into_iter().map(|h| h.join().expect("faded decompile panicked")).collect(),
        )
    });
    let plain = plain?;
    let plain_fid = match &plain.fidelity {
        Some(f) => f.clone(),
        None => measure(frames, &plain.document)?,
    };
    let mut best = Chosen {
        fidelity: Some(plain_fid),
        ..plain
    };
    for candidate in faded {
        let candidate = candidate?;
        if reconstructs_better(
            candidate.fidelity.as_ref().expect("measured"),
            best.fidelity.as_ref().expect("measured"),
        ) {
            best = candidate;
        }
    }
    Ok(best)
}

/// A fade (v1.13): the picture blended toward one colour,
/// `frame = alpha · picture + (1 − alpha) · colour`, alpha per frame.
struct Fade {
    colour: [f32; 3],
    alpha: Vec<f32>,
}

/// A clip fades when some frame keeps at most this much of its picture.
const FADE_MAX_ALPHA: f32 = 0.85;
/// A frame keeping less than this of its picture is too faded to recover
/// it from: it borrows its nearest recoverable neighbour's.
const FADE_MIN_ALPHA: f32 = 0.08;
/// A frame within this of its full picture is taken as unfaded.
const FADE_FULL_ALPHA: f32 = 0.97;
/// A frame whose pixels vary less than this about their mean (RMS levels)
/// is one flat colour: the fade at its deepest.
const FADE_FLAT_RMS: f32 = 2.5;

/// Measure a fade, two ways. Each frame's mean colour, projected onto the
/// line from the fade colour to the frame farthest from it, says how much
/// of the picture the frame keeps; black and white are both tried, and the
/// one whose line explains the means better is the fade colour. That
/// reading is fooled by anything that moves, so a second one chains each
/// frame's robust ratio to the frame before. Neither always wins — the
/// chain is exact over a still background and drifts under a moving
/// camera — so both are returned, to be decided by measurement. Empty when
/// no frame fades far enough to matter.
fn fade_track(frames: &[VideoFrame]) -> Vec<Fade> {
    if frames.len() < 6 {
        return Vec::new();
    }
    let means: Vec<[f32; 3]> = frames
        .iter()
        .map(|f| {
            let mut m = [0f64; 3];
            let mut n = 0f64;
            for p in f.pixels.chunks_exact(4).step_by(3) {
                for c in 0..3 {
                    m[c] += p[c] as f64;
                }
                n += 1.0;
            }
            [(m[0] / n) as f32, (m[1] / n) as f32, (m[2] / n) as f32]
        })
        .collect();
    let mut best: Option<(f32, Fade)> = None;
    for colour in [[0f32, 0.0, 0.0], [255f32, 255.0, 255.0]] {
        let off = |m: &[f32; 3]| [m[0] - colour[0], m[1] - colour[1], m[2] - colour[2]];
        let len2 = |d: [f32; 3]| d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        let r = (0..means.len()).max_by(|&a, &b| len2(off(&means[a])).total_cmp(&len2(off(&means[b])))).expect("frames");
        let dir = off(&means[r]);
        let full = len2(dir);
        if full < 3.0 * 15.0 * 15.0 {
            continue;
        }
        let mut resid = 0f32;
        let alpha: Vec<f32> = means
            .iter()
            .map(|m| {
                let d = off(m);
                let a = ((d[0] * dir[0] + d[1] * dir[1] + d[2] * dir[2]) / full).clamp(0.0, 1.0);
                resid += len2([d[0] - a * dir[0], d[1] - a * dir[1], d[2] - a * dir[2]]);
                if a >= FADE_FULL_ALPHA {
                    1.0
                } else {
                    a
                }
            })
            .collect();
        if best.as_ref().is_none_or(|b| resid < b.0) {
            best = Some((resid, Fade { colour, alpha }));
        }
    }
    // The means gate the pass: a fade moves them, and a moving camera
    // does not — while the chained reading below drifts under a camera
    // (a slow pan read as a 15% fade, and cost a second decompile).
    if best.as_ref().is_none_or(|(_, f)| f.alpha.iter().cloned().fold(1.0, f32::min) > FADE_MAX_ALPHA) {
        return Vec::new();
    }
    // Frames that are all one colour are the fade itself, at its deepest:
    // they name the fade colour outright, and they divide the clip into
    // stretches that each fade toward or away from their own picture — a
    // fade through black out of one shot and into the next. Without any,
    // the means' choice of colour stands.
    let contrast: Vec<f32> = frames
        .iter()
        .zip(&means)
        .map(|(f, m)| {
            let (mut v, mut n) = (0f64, 0f64);
            for p in f.pixels.chunks_exact(4).step_by(3) {
                for c in 0..3 {
                    v += (p[c] as f64 - m[c] as f64).powi(2);
                }
                n += 3.0;
            }
            (v / n.max(1.0)).sqrt() as f32
        })
        .collect();
    // Only black or white: a plain dark-blue opening frame is a title
    // card, not a fade.
    let near_extreme = |m: &[f32; 3]| m.iter().all(|&v| v <= 32.0) || m.iter().all(|&v| v >= 223.0);
    let flat: Vec<bool> = contrast
        .iter()
        .zip(&means)
        .map(|(&c, m)| c < FADE_FLAT_RMS && near_extreme(m))
        .collect();
    let colour = match best.as_ref() {
        _ if flat.iter().any(|&f| f) => {
            let mut picked: Vec<[f32; 3]> = (0..frames.len()).filter(|&i| flat[i]).map(|i| means[i]).collect();
            picked.sort_by(|a, b| (a[0] + a[1] + a[2]).total_cmp(&(b[0] + b[1] + b[2])));
            picked[picked.len() / 2]
        }
        Some((_, f)) => f.colour,
        None => return Vec::new(),
    };
    let dist = |m: &[f32; 3]| ((0..3).map(|c| (m[c] - colour[c]).powi(2)).sum::<f32>()).sqrt();
    // How much the picture faded from frame a to frame b: the median of the
    // pixels' ratios of distance from the fade colour, over pixels far
    // enough from it to measure; between frames too dark for that, the
    // ratio of their mean colours' distances.
    let step = |i: usize| -> f32 {
        let (a, b) = (&frames[i - 1], &frames[i]);
        let mut ratios: Vec<f32> = Vec::new();
        for (pa, pb) in a.pixels.chunks_exact(4).zip(b.pixels.chunks_exact(4)).step_by(2) {
            let da: f32 = (0..3).map(|c| pa[c] as f32 - colour[c]).map(|v| v * v).sum();
            if da < 3.0 * 25.0 * 25.0 {
                continue;
            }
            let dot: f32 = (0..3).map(|c| (pb[c] as f32 - colour[c]) * (pa[c] as f32 - colour[c])).sum();
            ratios.push(dot / da);
        }
        if ratios.len() >= 64 {
            let mid = ratios.len() / 2;
            ratios.select_nth_unstable_by(mid, |x, y| x.total_cmp(y));
            ratios[mid].max(0.0)
        } else {
            let da = dist(&means[i - 1]).max(1e-3);
            dist(&means[i]) / da
        }
    };
    // Chain the steps through each stretch between flat frames, and scale
    // each stretch so its fullest frame keeps all of its picture.
    let mut alpha = vec![0f32; frames.len()];
    let mut start = 0usize;
    while start < frames.len() {
        if flat[start] {
            start += 1;
            continue;
        }
        let mut end = start;
        while end + 1 < frames.len() && !flat[end + 1] {
            end += 1;
        }
        alpha[start] = 1.0;
        for i in start + 1..=end {
            alpha[i] = alpha[i - 1] * step(i);
        }
        let peak = alpha[start..=end].iter().cloned().fold(0.0f32, f32::max).max(1e-6);
        for a in alpha[start..=end].iter_mut() {
            *a = (*a / peak).clamp(0.0, 1.0);
            if *a >= FADE_FULL_ALPHA {
                *a = 1.0;
            }
        }
        start = end + 1;
    }
    let mut out: Vec<Fade> = Vec::new();
    if let Some((_, mean_fade)) = best {
        if mean_fade.alpha.iter().cloned().fold(1.0, f32::min) <= FADE_MAX_ALPHA {
            out.push(mean_fade);
        }
    }
    if alpha.iter().cloned().fold(1.0, f32::min) <= FADE_MAX_ALPHA {
        out.push(Fade { colour, alpha });
    }
    out
}

/// The frames with the fade taken out: each pixel's picture recovered from
/// `(frame − (1 − alpha) · colour) / alpha`; a frame too faded to recover
/// borrows its nearest recoverable neighbour's picture.
fn unfade(frames: &[VideoFrame], fade: &Fade) -> Vec<VideoFrame> {
    let mut out: Vec<VideoFrame> = frames.to_vec();
    for (f, &a) in out.iter_mut().zip(&fade.alpha) {
        if a >= 1.0 || a < FADE_MIN_ALPHA {
            continue;
        }
        for p in f.pixels.chunks_exact_mut(4) {
            for c in 0..3 {
                let v = (p[c] as f32 - (1.0 - a) * fade.colour[c]) / a;
                p[c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    let ok: Vec<usize> = (0..frames.len()).filter(|&i| fade.alpha[i] >= FADE_MIN_ALPHA).collect();
    if ok.is_empty() {
        return frames.to_vec();
    }
    for i in 0..frames.len() {
        if fade.alpha[i] < FADE_MIN_ALPHA {
            let near = *ok.iter().min_by_key(|&&j| (j as i64 - i as i64).abs()).expect("one recoverable frame");
            let pixels = out[near].pixels.clone();
            out[i].pixels = pixels;
        }
    }
    out
}

/// The fade, measured against the picture as reconstructed: with the
/// reconstruction `P`, the source `F` and the fade colour `C`, the opacity
/// the fade needs in each frame has a closed form, the least-squares
/// `o = Σ (F − P)·(C − P) / Σ |C − P|²`. The reading that took the fade out
/// only has to be good enough to decompile the picture; the fade drawn back
/// on top is fitted to what was actually decompiled. A frame whose
/// reconstruction is too close to the fade colour to measure keeps the
/// reading's value.
fn fit_fade(frames: &[VideoFrame], doc: &Document, fade: &Fade) -> Result<Fade> {
    let count = vac_compiler::render::frame_count(doc);
    let idx = aligned_indices(frames, count, doc.canvas.fps);
    let mut wanted: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (j, &i) in idx.iter().enumerate() {
        wanted[i].push(j);
    }
    let mut alpha = fade.alpha.clone();
    let c = fade.colour;
    vac_compiler::render::render_document_each(doc, |i, r| {
        for &j in &wanted[i] {
            let f = &frames[j];
            let (mut num, mut den) = (0f64, 0f64);
            for (pf, pr) in f.pixels.chunks_exact(4).zip(r.rgba.chunks_exact(4)).step_by(2) {
                for k in 0..3 {
                    let (fv, pv) = (pf[k] as f64, pr[k] as f64);
                    num += (fv - pv) * (c[k] as f64 - pv);
                    den += (c[k] as f64 - pv).powi(2);
                }
            }
            let n = (f.pixels.len() / 8).max(1) as f64;
            if den / n > 3.0 * 20.0 * 20.0 {
                let o = (num / den).clamp(0.0, 1.0) as f32;
                alpha[j] = if 1.0 - o >= FADE_FULL_ALPHA { 1.0 } else { 1.0 - o };
            }
        }
        Ok(())
    })?;
    Ok(Fade {
        colour: fade.colour,
        alpha,
    })
}

/// Draw the fade on top of every scene: a canvas-sized rect in the fade
/// colour, its opacity keyframed to `1 − alpha` over the scene's frames,
/// simplified until no frame's opacity is off by more than the dial's
/// opacity tolerance.
fn add_fade(doc: &mut Document, fade: &Fade, quality: f32) {
    let fps = doc.canvas.fps.max(1) as u64;
    let tol = vac_decompiler::Tolerances::from_quality(quality).opacity_levels / 255.0;
    let (w, h) = (doc.canvas.width as f32, doc.canvas.height as f32);
    let colour = Color::rgb(fade.colour[0] as u8, fade.colour[1] as u8, fade.colour[2] as u8);
    let mut first = 0usize;
    for scene in &mut doc.scenes {
        let n = ((scene.duration_ms as u64 * fps) / 1000).max(1) as usize;
        let alpha: Vec<f32> = (first..first + n).map(|i| fade.alpha.get(i).copied().unwrap_or(1.0)).collect();
        first += n;
        if alpha.iter().all(|&a| a >= 1.0) {
            continue;
        }
        let time = |i: usize| ((i as u64 * 1000) / fps) as u32;
        let mut keep = vec![false; n];
        keep[0] = true;
        keep[n - 1] = true;
        let mut stack = vec![(0usize, n - 1)];
        while let Some((a, b)) = stack.pop() {
            if b <= a + 1 {
                continue;
            }
            let (worst, at) = (a + 1..b)
                .map(|t| {
                    let u = (time(t) - time(a)) as f32 / (time(b) - time(a)).max(1) as f32;
                    ((alpha[a] + (alpha[b] - alpha[a]) * u - alpha[t]).abs(), t)
                })
                .fold((0.0f32, a), |m, x| if x.0 > m.0 { x } else { m });
            if worst > tol {
                keep[at] = true;
                stack.push((a, at));
                stack.push((at, b));
            }
        }
        let round = |v: f32| (v * 1000.0).round() / 1000.0;
        scene.statements.push(Statement::Let {
            name: "fade".into(),
            shape: Shape::Rect { x: 0.0, y: 0.0, w, h },
        });
        scene.statements.push(Statement::Assign {
            target: "fade".into(),
            property: Property::Fill,
            value: Value::Color(colour),
        });
        scene.statements.push(Statement::Assign {
            target: "fade".into(),
            property: Property::Stroke,
            value: Value::None,
        });
        scene.statements.push(Statement::Animate(Animation {
            target: "fade".into(),
            keyframes: (0..n)
                .filter(|&i| keep[i])
                .map(|i| Keyframe {
                    time_ms: time(i),
                    transforms: vec![Transform::Opacity(round(1.0 - alpha[i]))],
                })
                .collect(),
            easing: Easing::Linear,
        }));
    }
}

/// Decompile a clip that may contain cuts: the shots on their own, and the
/// clip as one, keeping whichever reconstructs better.
fn decompile_cuts(frames: &[VideoFrame], opts: &DecompileOptions) -> Result<Chosen> {
    let mut cuts = if opts.shots { cut_candidates(frames) } else { Vec::new() };
    // v1.22: every frame of a wipe or a push steps like a cut. The cut
    // candidates inside one are the sweep, not cuts; its edges are.
    let sweeps: Vec<Sweep> = if opts.shots { sweep_candidates(frames) } else { Vec::new() };
    cuts.retain(|&c| !sweeps.iter().any(|s| c > s.a && c <= s.b));
    for s in &sweeps {
        cuts.push(s.a);
        cuts.push(s.b + 1);
    }
    // v1.17: a dissolve is a cut spread over frames. Its frames become a
    // scene of their own between the two shots, and their edges are cuts.
    let dissolves: Vec<(usize, usize)> = if opts.shots {
        dissolve_candidates(frames)
            .into_iter()
            .filter(|&(a, b)| !cuts.iter().any(|&c| c >= a && c <= b + 1))
            .filter(|&(a, b)| !sweeps.iter().any(|s| a <= s.b + 1 && s.a <= b + 1))
            .collect()
    } else {
        Vec::new()
    };
    for &(a, b) in &dissolves {
        cuts.push(a);
        cuts.push(b + 1);
    }
    cuts.sort_unstable();
    cuts.dedup();
    if cuts.is_empty() {
        return decompile_shot(frames, opts);
    }
    let is_dissolve = |w0: usize| dissolves.iter().any(|&(a, _)| a == w0);
    let sweep_at = |w0: usize| sweeps.iter().copied().find(|s| s.a == w0);
    // v1.12: decompile each shot on its own and the clip as one, side by
    // side, and keep whichever reconstructs better. The clip as one is a
    // safety net against a cut that is not one, and on a long clip with
    // many cuts it is by far the most expensive candidate there is — a
    // 12 s, three-shot 720p clip had not finished it after 25 minutes — so
    // past a few cuts or a few hundred frames the cuts are trusted.
    let mut edges = vec![0usize];
    edges.extend(&cuts);
    edges.push(frames.len());
    let check_whole = cuts.len() <= WHOLE_CHECK_MAX_CUTS && frames.len() <= WHOLE_CHECK_MAX_FRAMES;
    // The shots first, side by side; each dissolve after them, because it
    // carries its neighbours' cameras through itself (v1.18).
    let (whole, shots): (Option<Result<Chosen>>, Vec<Option<Result<Chosen>>>) = std::thread::scope(|scope| {
        let whole = check_whole.then(|| scope.spawn(|| decompile_shot(frames, opts)));
        let shots: Vec<_> = edges
            .windows(2)
            .map(|w| {
                let (a, b) = (w[0], w[1]);
                (!is_dissolve(a) && sweep_at(a).is_none()).then(|| scope.spawn(move || decompile_shot(&frames[a..b], opts)))
            })
            .collect();
        (
            whole.map(|h| h.join().expect("whole-clip decompile panicked")),
            shots.into_iter().map(|h| h.map(|h| h.join().expect("shot decompile panicked"))).collect(),
        )
    });
    let mut slots: Vec<Option<Chosen>> = shots.into_iter().map(|s| s.transpose()).collect::<Result<_>>()?;
    for k in 0..slots.len() {
        if slots[k].is_some() {
            continue;
        }
        let (a, b) = (edges[k], edges[k + 1]);
        let before = k.checked_sub(1).and_then(|j| slots[j].as_ref()).map(|c| &c.document);
        let after = slots.get(k + 1).and_then(|s| s.as_ref()).map(|c| &c.document);
        let made = match sweep_at(a) {
            Some(sweep) => sweep_shot(frames, sweep, opts, before, after)?,
            None => dissolve_shot(frames, a, b, opts, before, after)?,
        };
        slots[k] = Some(made);
    }
    let parts: Vec<Chosen> = slots.into_iter().map(|s| s.expect("every slot filled")).collect();
    let merged = merge_shots(frames, &edges, parts.iter().map(|c| &c.document));
    let fid = measure(frames, &merged)?;
    if let Some(whole) = whole {
        let whole = whole?;
        let whole_fid = match &whole.fidelity {
            Some(f) => f.clone(),
            None => measure(frames, &whole.document)?,
        };
        if !reconstructs_better(&fid, &whole_fid) {
            return Ok(Chosen {
                fidelity: Some(whole_fid),
                ..whole
            });
        }
    }
    let first = &parts[0];
    Ok(Chosen {
        document: merged,
        segmentation: first.segmentation,
        background: first.background,
        plate: first.plate,
        static_layer: first.static_layer,
        fidelity: Some(fid),
        audited_out: parts.iter().map(|c| c.audited_out).sum(),
        refined: parts.iter().map(|c| c.refined).sum(),
        moved: parts.iter().map(|c| c.moved).sum(),
        sprites: parts.iter().map(|c| c.sprites).sum(),
    })
}

/// How a transition brings the incoming picture on (v1.22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SweepKind {
    /// Uncovered by an edge moving across; neither picture moves.
    Wipe,
    /// Pushed on, the outgoing picture pushed off ahead of it.
    Push,
}

/// The edge a sweep's incoming picture comes on from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    fn horizontal(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
    /// The incoming picture enters at the high end of its axis.
    fn from_high(self) -> bool {
        matches!(self, Edge::Right | Edge::Bottom)
    }
}

/// A wipe or a push over frames `[a, b]`, both included: the frames that
/// show both pictures.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sweep {
    a: usize,
    b: usize,
    kind: SweepKind,
    from: Edge,
}

/// The lengths a sweep is looked for at, frames, as for a dissolve.
const SWEEP_SPANS: [usize; 7] = [8, 12, 16, 24, 32, 48, 64];
/// The most of the two pictures' difference a sweep may leave unexplained
/// in any frame between them. A dissolve's midpoint leaves a quarter, so a
/// blend is never taken for a sweep.
const SWEEP_MAX_RESIDUAL: f64 = 0.12;
/// A sweep's window is bounded by still frames: each end steps from its own
/// neighbour by at most this share of how far the window's ends differ...
const SWEEP_CALM: f64 = 0.1;
/// ...and by at most this many times the clip's typical step, plus
/// [`SWEEP_CALM_NOISE`] of codec noise. Relative to the window alone, a
/// frame in the middle of a wipe was calm — a 16 px strip is little beside
/// two whole different pictures — and bounded a false push over it.
const SWEEP_CALM_STEPS: f64 = 3.0;
/// Codec noise in a still frame's step, as mean squared error.
const SWEEP_CALM_NOISE: f64 = 25.0;
/// A sweep replaces the picture: its two ends must differ decisively (by
/// 30 levels a channel, as a dissolve's changing pixels) over at least
/// this share of the frame. On a flat background a lone shape sliding is,
/// geometrically, a push — the background does not care that it moved —
/// and `sliding_rect.vac` was taken for one.
const SWEEP_MIN_COVER: f64 = 0.5;

/// RGB as floats, row-major, for the sweep fits.
fn rgb_f32(f: &VideoFrame) -> Vec<f32> {
    f.pixels.chunks_exact(4).flat_map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect()
}

/// Frame `f` as a wipe of `q` over `p` coming on from `from`: how far it
/// has got, in lines of the sweep's axis, and the squared error left.
/// Each line's cost against either picture, then the best boundary by
/// prefix sums: exact for an axis-aligned edge.
fn wipe_fit(f: &[f32], p: &[f32], q: &[f32], w: usize, h: usize, from: Edge) -> (usize, f64) {
    let lines = if from.horizontal() { w } else { h };
    let (mut cp, mut cq) = (vec![0f64; lines], vec![0f64; lines]);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 3;
            let l = if from.horizontal() { x } else { y };
            for c in 0..3 {
                cp[l] += ((f[i + c] - p[i + c]) as f64).powi(2);
                cq[l] += ((f[i + c] - q[i + c]) as f64).powi(2);
            }
        }
    }
    let (mut pp, mut pq) = (vec![0f64; lines + 1], vec![0f64; lines + 1]);
    for l in 0..lines {
        pp[l + 1] = pp[l] + cp[l];
        pq[l + 1] = pq[l] + cq[l];
    }
    let (tp, tq) = (pp[lines], pq[lines]);
    let mut best = (f64::INFINITY, 0usize);
    for b in 0..=lines {
        // The incoming picture holds lines [b, lines) entering high, [0, b) entering low.
        let err = if from.from_high() { pp[b] + (tq - pq[b]) } else { pq[b] + (tp - pp[b]) };
        if err < best.0 {
            best = (err, b);
        }
    }
    let covered = if from.from_high() { lines - best.1 } else { best.1 };
    (covered, best.0)
}

/// Frame `f` as a push of `q` onto `p` from `from`: how far it has got, in
/// lines, and the squared error left. A push shifts whole lines, so the
/// offset is found on the lines' mean colours and the frame checked at it.
fn push_fit(f: &[f32], p: &[f32], q: &[f32], w: usize, h: usize, from: Edge) -> (usize, f64) {
    let (lines, span) = if from.horizontal() { (w, h) } else { (h, w) };
    let at = |x: usize, y: usize| (y * w + x) * 3;
    let profile = |img: &[f32]| -> Vec<[f64; 3]> {
        let mut out = vec![[0f64; 3]; lines];
        for y in 0..h {
            for x in 0..w {
                let l = if from.horizontal() { x } else { y };
                for c in 0..3 {
                    out[l][c] += img[at(x, y) + c] as f64;
                }
            }
        }
        out.iter_mut().for_each(|v| v.iter_mut().for_each(|c| *c /= span as f64));
        out
    };
    let (pf, pp, pq) = (profile(f), profile(p), profile(q));
    // Which line of which picture shows at line `l` when the push has got `d` lines in.
    let source = |l: usize, d: usize| -> (bool, usize) {
        if from.from_high() {
            if l + d < lines { (false, l + d) } else { (true, l + d - lines) }
        } else if l < d {
            (true, l + lines - d)
        } else {
            (false, l - d)
        }
    };
    let mut best = (f64::INFINITY, 0usize);
    for d in 0..=lines {
        let err: f64 = (0..lines)
            .map(|l| {
                let (incoming, sl) = source(l, d);
                let v = if incoming { &pq[sl] } else { &pp[sl] };
                (0..3).map(|c| (pf[l][c] - v[c]).powi(2)).sum::<f64>()
            })
            .sum();
        if err < best.0 {
            best = (err, d);
        }
    }
    let d = best.1;
    let mut err = 0f64;
    for y in 0..h {
        for x in 0..w {
            let l = if from.horizontal() { x } else { y };
            let (incoming, sl) = source(l, d);
            let (sx, sy) = if from.horizontal() { (sl, y) } else { (x, sl) };
            let src = if incoming { q } else { p };
            for c in 0..3 {
                err += ((f[at(x, y) + c] - src[at(sx, sy) + c]) as f64).powi(2);
            }
        }
    }
    (d, err)
}

/// Wipes and pushes (v1.22): a window whose ends are two still pictures and
/// whose every frame between is the second coming on over the first along
/// one axis — uncovered by a moving edge, or pushing it off. Each of its
/// frames steps like a cut, so the cut test fires all through one; found
/// here, its frames become one scene. Searched on thumbnails, as dissolves
/// are; the longest window that fits wins.
fn sweep_candidates(frames: &[VideoFrame]) -> Vec<Sweep> {
    let n = frames.len();
    if n < 3 * MIN_SHOT_FRAMES {
        return Vec::new();
    }
    let (w, h) = (frames[0].width, frames[0].height);
    if frames.iter().any(|f| f.width != w || f.height != h) {
        return Vec::new();
    }
    let thumbs = downscale(frames, (w / 160).max(1));
    let (tw, th) = (thumbs[0].width as usize, thumbs[0].height as usize);
    let t: Vec<Vec<f32>> = thumbs.iter().map(rgb_f32).collect();
    let mse = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| ((x - y) as f64).powi(2)).sum::<f64>() / a.len() as f64;
    let steps: Vec<f64> = (1..n).map(|i| mse(&t[i - 1], &t[i])).collect();
    let typical = {
        let mut s = steps.clone();
        s.sort_by(f64::total_cmp);
        s[s.len() / 2]
    };
    let still = SWEEP_CALM_STEPS * typical + SWEEP_CALM_NOISE;
    let models = [
        (SweepKind::Wipe, Edge::Left),
        (SweepKind::Wipe, Edge::Right),
        (SweepKind::Wipe, Edge::Top),
        (SweepKind::Wipe, Edge::Bottom),
        (SweepKind::Push, Edge::Left),
        (SweepKind::Push, Edge::Right),
        (SweepKind::Push, Edge::Top),
        (SweepKind::Push, Edge::Bottom),
    ];
    let mut found: Vec<(Sweep, f64)> = Vec::new();
    for &k in &SWEEP_SPANS {
        if k + 2 * MIN_SHOT_FRAMES > n {
            continue;
        }
        for i in 1..n - k {
            let (p, q) = (&t[i], &t[i + k]);
            let span = mse(p, q);
            if span < CUT_FLOOR_MSE {
                continue;
            }
            let calm = SWEEP_CALM * span.min(still / SWEEP_CALM);
            if steps[i - 1] > calm || steps.get(i + k).is_some_and(|&s| s > calm) {
                continue;
            }
            let decisive = p
                .chunks_exact(3)
                .zip(q.chunks_exact(3))
                .filter(|(a, b)| a.iter().zip(b.iter()).map(|(x, y)| ((x - y) as f64).powi(2)).sum::<f64>() >= 3.0 * 30.0 * 30.0)
                .count();
            if (decisive as f64) < SWEEP_MIN_COVER * (p.len() / 3) as f64 {
                continue;
            }
            let total = span * p.len() as f64;
            for &(kind, from) in &models {
                let lines = if from.horizontal() { tw } else { th };
                let fit = |u: usize| match kind {
                    SweepKind::Wipe => wipe_fit(&t[u], p, q, tw, th, from),
                    SweepKind::Push => push_fit(&t[u], p, q, tw, th, from),
                };
                // The middle frame first: most windows fail there.
                let (gm, em) = fit(i + k / 2);
                if em > SWEEP_MAX_RESIDUAL * total || gm == 0 || gm == lines {
                    continue;
                }
                let mut got = Vec::with_capacity(k - 1);
                let mut worst = 0f64;
                let fits = (i + 1..i + k).all(|u| {
                    let (g, e) = fit(u);
                    got.push(g);
                    worst = worst.max(e / total);
                    e <= SWEEP_MAX_RESIDUAL * total
                });
                // Onward only, give or take a line, from little to most, and
                // steadily: no frame takes more than a third of the frame. A
                // pan creeping a line a frame and then a cut, or a dissolve's
                // end, jumps most of it at once (`cuts_three_shots`: 15 of 160
                // lines, then 160).
                let onward = got.windows(2).all(|g| g[1] + 1 >= g[0]);
                let steady = got.windows(2).all(|g| g[1].saturating_sub(g[0]) * 3 <= lines);
                let from_little = got.first().is_some_and(|&g| g * 3 < lines);
                let to_most = got.last().is_some_and(|&g| g * 3 > 2 * lines);
                if !(fits && onward && steady && from_little && to_most) {
                    continue;
                }
                let first = got.iter().position(|&g| g > 0).map(|x| i + 1 + x).unwrap_or(i + 1);
                let last = got.iter().rposition(|&g| g < lines).map(|x| i + 1 + x).unwrap_or(i + k - 1);
                if last < first || first < MIN_SHOT_FRAMES || n - (last + 1) < MIN_SHOT_FRAMES {
                    continue;
                }
                found.push((Sweep { a: first, b: last, kind, from }, worst));
                break;
            }
        }
    }
    found.sort_by(|x, y| (y.0.b - y.0.a).cmp(&(x.0.b - x.0.a)).then(x.1.total_cmp(&y.1)));
    let mut kept: Vec<Sweep> = Vec::new();
    for (s, _) in found {
        if kept.iter().all(|k| s.b + MIN_SHOT_FRAMES < k.a || s.a > k.b + MIN_SHOT_FRAMES) {
            kept.push(s);
        }
    }
    kept.sort_by_key(|s| s.a);
    kept
}

/// A sweep's frames as a scene of their own (v1.22): the frame before it
/// and the frame after it as pictures — a wipe the incoming one under a
/// `clip()` window growing from its edge, a push the two side by side
/// under `position()` keyframes. How far it has got is re-measured in every
/// frame at full size, to the pixel, and simplified like any keyframes.
fn sweep_shot(
    frames: &[VideoFrame],
    sweep: Sweep,
    opts: &DecompileOptions,
    before: Option<&Document>,
    after: Option<&Document>,
) -> Result<Chosen> {
    // v1.23: the shots either side, when they are still pictures, already
    // write the pictures a sweep draws; drawing theirs adds nothing to the
    // file. Both are built, and the one that buys more is kept.
    let (w, h) = (frames[0].width, frames[0].height);
    let shared = (
        before.and_then(|d| d.scenes.last()).and_then(|s| still_plate(s, w, h)),
        after.and_then(|d| d.scenes.first()).and_then(|s| still_plate(s, w, h)),
    );
    let own = sweep_variant(frames, sweep, opts, (None, None))?;
    if shared.0.is_none() && shared.1.is_none() {
        return Ok(own);
    }
    let theirs = sweep_variant(frames, sweep, opts, shared)?;
    let span = &frames[sweep.a..=sweep.b];
    let (fo, ft) = (measure(span, &own.document)?, measure(span, &theirs.document)?);
    let neighbours: Vec<&Document> = [before, after].into_iter().flatten().collect();
    let gain = improvement(&ft, &fo)
        + byte_price(opts.quality) * (new_picture_bytes(&own.document, &neighbours) as f64 - new_picture_bytes(&theirs.document, &neighbours) as f64);
    Ok(if gain > 0.0 { theirs } else { own })
}

/// [`sweep_shot`] with the pictures it draws given or, where `None`, its
/// edge frames.
fn sweep_variant(
    frames: &[VideoFrame],
    sweep: Sweep,
    opts: &DecompileOptions,
    pictures: (Option<Shape>, Option<Shape>),
) -> Result<Chosen> {
    let (a, b) = (sweep.a, sweep.b);
    let (p, q) = (&frames[a - 1], &frames[(b + 1).min(frames.len() - 1)]);
    let (w, h) = (p.width as usize, p.height as usize);
    let tol = vac_decompiler::Tolerances::from_quality(opts.quality);
    let fps = vac_decompiler::frames::estimate_fps(frames).max(1);
    let n = b + 1 - a;
    let time = |i: usize| ((i as u64 * 1000) / fps as u64) as u32;
    let duration_ms = ((n as u64 * 1000).div_ceil(fps as u64)) as u32;
    let (pf, qf) = (rgb_f32(p), rgb_f32(q));
    let lines = if sweep.from.horizontal() { w } else { h };
    let got: Vec<f32> = (a..=b)
        .map(|u| {
            let f = rgb_f32(&frames[u]);
            (match sweep.kind {
                SweepKind::Wipe => wipe_fit(&f, &pf, &qf, w, h, sweep.from),
                SweepKind::Push => push_fit(&f, &pf, &qf, w, h, sweep.from),
            })
            .0 as f32
        })
        .collect();
    // Keyframes: the frames where the sweep's progress is not straight,
    // pinned to an eighth of the keyframe tolerance as a camera plate's
    // are — a whole picture a pixel off costs far more than one shape (a
    // push stepping in whole pixels, simplified to a straight line, lost
    // 12 dB on every other frame).
    let pin = tol.keyframe_px * 0.125;
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((x, y)) = stack.pop() {
        if y <= x + 1 {
            continue;
        }
        let (worst, at) = (x + 1..y)
            .map(|u| {
                let r = (time(u) - time(x)) as f32 / (time(y) - time(x)).max(1) as f32;
                ((got[x] + (got[y] - got[x]) * r - got[u]).abs(), u)
            })
            .fold((0.0f32, x), |m, v| if v.0 > m.0 { v } else { m });
        if worst > pin {
            keep[at] = true;
            stack.push((x, at));
            stack.push((at, y));
        }
    }
    let (wf, hf, lf) = (w as f32, h as f32, lines as f32);
    let jpeg = |f: &VideoFrame| {
        vac_decompiler::jpeg_data_uri(f, tol.raster_quality)
            .ok_or_else(|| Error::Decompile(DecompileError::Other("could not encode a sweep's picture".into())))
    };
    let image = |x: f32, y: f32, src: String| Shape::Image { x, y, w: wf, h: hf, src };
    let src_of = |given: &Option<Shape>, f: &VideoFrame| -> Result<String> {
        match given {
            Some(Shape::Image { src, .. }) => Ok(src.clone()),
            _ => jpeg(f),
        }
    };
    let (p_src, q_src) = (src_of(&pictures.0, p)?, src_of(&pictures.1, q)?);
    let keyed = |f: &dyn Fn(f32) -> Vec<Transform>| -> Vec<Keyframe> {
        (0..n).filter(|&u| keep[u]).map(|u| Keyframe { time_ms: time(u), transforms: f(got[u]) }).collect()
    };
    let animate = |target: &str, keyframes: Vec<Keyframe>| {
        Statement::Animate(Animation {
            target: target.into(),
            keyframes,
            easing: Easing::Linear,
        })
    };
    let mut statements = Vec::new();
    match sweep.kind {
        SweepKind::Wipe => {
            statements.push(Statement::Let { name: "plate".into(), shape: image(0.0, 0.0, p_src.clone()) });
            statements.push(Statement::Let { name: "incoming".into(), shape: image(0.0, 0.0, q_src.clone()) });
            let window = move |g: f32| -> Vec<Transform> {
                vec![match sweep.from {
                    Edge::Left => Transform::Clip(0.0, 0.0, g, hf),
                    Edge::Right => Transform::Clip(wf - g, 0.0, g, hf),
                    Edge::Top => Transform::Clip(0.0, 0.0, wf, g),
                    Edge::Bottom => Transform::Clip(0.0, hf - g, wf, g),
                }]
            };
            statements.push(animate("incoming", keyed(&window)));
        }
        SweepKind::Push => {
            // Where each picture's top-left is once the push has got `g` lines in.
            let outgoing = move |g: f32| -> (f32, f32) {
                match sweep.from {
                    Edge::Right => (-g, 0.0),
                    Edge::Left => (g, 0.0),
                    Edge::Bottom => (0.0, -g),
                    Edge::Top => (0.0, g),
                }
            };
            let incoming = move |g: f32| -> (f32, f32) {
                match sweep.from {
                    Edge::Right => (lf - g, 0.0),
                    Edge::Left => (g - lf, 0.0),
                    Edge::Bottom => (0.0, lf - g),
                    Edge::Top => (0.0, g - lf),
                }
            };
            let (x0, y0) = incoming(got[0]);
            statements.push(Statement::Let { name: "plate".into(), shape: image(0.0, 0.0, p_src.clone()) });
            statements.push(animate("plate", keyed(&|g| {
                let (x, y) = outgoing(g);
                vec![Transform::Position(x, y)]
            })));
            statements.push(Statement::Let { name: "incoming".into(), shape: image(x0, y0, q_src.clone()) });
            statements.push(animate("incoming", keyed(&|g| {
                let (x, y) = incoming(g);
                vec![Transform::Position(x, y)]
            })));
        }
    }
    let name = match sweep.kind {
        SweepKind::Wipe => "wipe",
        SweepKind::Push => "push",
    };
    Ok(Chosen {
        document: Document {
            canvas: Canvas {
                width: p.width,
                height: p.height,
                fps,
            },
            scenes: vec![Scene {
                name: name.into(),
                duration_ms,
                statements,
            }],
        },
        segmentation: Segmentation::Components,
        background: Background::Corners,
        plate: Plate::Raster,
        static_layer: false,
        fidelity: None,
        audited_out: 0,
        refined: 0,
        moved: 0,
        sprites: 0,
    })
}

/// The lengths a dissolve is looked for at, frames: about a quarter of a
/// second to two seconds at 30 fps.
const DISSOLVE_SPANS: [usize; 7] = [8, 12, 16, 24, 32, 48, 64];
/// Frames a dissolve apart must differ this many times more than frames
/// that far apart usually do. Lower than a cut's [`CUT_RATIO`]: under a
/// camera move, frames 32 apart already differ a lot (a Ken Burns
/// slideshow's dissolve stood out 6×, not 10×), and the blend test below,
/// not this, is what tells a dissolve from motion — a moving picture's
/// in-between frames are shifted copies of its ends, not mixtures of them.
const DISSOLVE_RATIO: f64 = 3.0;
/// In the frames midway through a dissolve, at least this fraction of the
/// changing pixels must have changed by the frame's own share.
const DISSOLVE_MIN_TOGETHER: f64 = 0.6;
/// The frames inside a dissolve must be this close to blends of the frames
/// either side of it: the mean of their residual's energy, as a fraction of
/// the two pictures' difference.
const DISSOLVE_MAX_RESIDUAL: f64 = 0.15;

/// Where the clip dissolves from one shot to the next (v1.17): `(a, b)`,
/// the first and last frames that show both. A dissolve is a cut spread over
/// frames, so no one step is a cut; but frames `k` apart, across it, differ
/// as much as a cut does, and every frame inside it is a blend
/// `(1 − β)·before + β·after` with `β` rising from 0 to 1. Looked for on
/// thumbnails, verified on them, at each of [`DISSOLVE_SPANS`].
fn dissolve_candidates(frames: &[VideoFrame]) -> Vec<(usize, usize)> {
    let n = frames.len();
    if n < 3 * MIN_SHOT_FRAMES {
        return Vec::new();
    }
    let (w, h) = (frames[0].width, frames[0].height);
    if frames.iter().any(|f| f.width != w || f.height != h) {
        return Vec::new();
    }
    let thumbs = downscale(frames, (w / 160).max(1));
    let px = |f: &VideoFrame| -> Vec<f64> { f.pixels.chunks_exact(4).flat_map(|p| [p[0] as f64, p[1] as f64, p[2] as f64]).collect() };
    let t: Vec<Vec<f64>> = thumbs.iter().map(px).collect();
    let mse = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>() / a.len() as f64;
    let steps: Vec<f64> = (1..n).map(|i| mse(&t[i - 1], &t[i])).collect();
    let mut found: Vec<(usize, usize, f64)> = Vec::new();
    for &k in &DISSOLVE_SPANS {
        // Room for a shot on either side.
        if k + 2 * MIN_SHOT_FRAMES > n {
            continue;
        }
        let across: Vec<f64> = (0..n - k).map(|i| mse(&t[i], &t[i + k])).collect();
        let mut sorted = across.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let bar = (DISSOLVE_RATIO * sorted[sorted.len() / 2]).max(CUT_FLOOR_MSE);
        for i in 0..n - k {
            if across[i] < bar || i == 0 || i + k >= n {
                continue;
            }
            // No single step inside may be a cut: that is a cut, not a
            // dissolve.
            if steps[i..i + k].iter().any(|&s| s >= across[i] * 0.5) {
                continue;
            }
            let (p, q) = (&t[i], &t[i + k]);
            let d: Vec<f64> = p.iter().zip(q).map(|(x, y)| y - x).collect();
            let dd: f64 = d.iter().map(|v| v * v).sum::<f64>().max(1e-9);
            let mut betas = Vec::new();
            let mut resid = 0f64;
            for u in i + 1..i + k {
                let f = &t[u];
                let beta = f.iter().zip(p).zip(&d).map(|((fv, pv), dv)| (fv - pv) * dv).sum::<f64>() / dd;
                let r: f64 = f.iter().zip(p).zip(&d).map(|((fv, pv), dv)| (fv - pv - beta * dv).powi(2)).sum::<f64>();
                resid += r / dd;
                betas.push(beta);
            }
            let resid = resid / (k - 1) as f64;
            let rising = betas.first().is_some_and(|&b| b < 0.35) && betas.last().is_some_and(|&b| b > 0.65);
            if resid > DISSOLVE_MAX_RESIDUAL || !rising {
                continue;
            }
            // Simultaneity: a dissolve moves every changing pixel at once,
            // in proportion; an edge sweeping across moves each pixel all at
            // once at its own moment. In the frames midway through, most
            // changing pixels must sit near the frame's β, not at 0 or 1 —
            // a flat card sliding over a flat background fits the blend in
            // the mean and fails this.
            let (mut agree, mut counted) = (0usize, 0usize);
            for (j, u) in (i + 1..i + k).enumerate() {
                let beta = betas[j];
                if !(0.25..=0.75).contains(&beta) {
                    continue;
                }
                let f = &t[u];
                for px in 0..p.len() / 3 {
                    let dv: f64 = (0..3).map(|c| d[px * 3 + c] * d[px * 3 + c]).sum();
                    if dv < 3.0 * 30.0 * 30.0 {
                        continue;
                    }
                    let r = (0..3).map(|c| (f[px * 3 + c] - p[px * 3 + c]) * d[px * 3 + c]).sum::<f64>() / dv;
                    counted += 1;
                    agree += ((r - beta).abs() <= 0.25) as usize;
                }
            }
            if counted == 0 || (agree as f64) < DISSOLVE_MIN_TOGETHER * counted as f64 {
                continue;
            }
            // The frames that show both: from the first that has left the
            // shot before to the last that has not reached the shot after.
            let first = betas.iter().position(|&b| b > 0.03).map(|x| i + 1 + x).unwrap_or(i + 1);
            let last = betas.iter().rposition(|&b| b < 0.97).map(|x| i + 1 + x).unwrap_or(i + k - 1);
            if last < first || first < MIN_SHOT_FRAMES || n - (last + 1) < MIN_SHOT_FRAMES {
                continue;
            }
            found.push((first, last, resid));
        }
    }
    // Of any that overlap, the longest that fits: a dissolve's extent is
    // its longest window, and shorter windows inside it fit too.
    found.sort_by(|a, b| (b.1 - b.0).cmp(&(a.1 - a.0)).then(a.2.total_cmp(&b.2)));
    let mut kept: Vec<(usize, usize)> = Vec::new();
    for (a, b, _) in found {
        if kept.iter().all(|&(c, d)| b + MIN_SHOT_FRAMES < c || a > d + MIN_SHOT_FRAMES) {
            kept.push((a, b));
        }
    }
    kept.sort_unstable();
    kept
}

/// The value each of a camera's channels — position, scale, rotation —
/// has at time `t` (ms, possibly outside the animation): linear between
/// the keyframes that carry it, and carried on linearly past either end
/// from the two nearest.
fn camera_at(anim: &Animation, t: f64) -> Vec<Transform> {
    let mut out = Vec::new();
    let pick = |f: &dyn Fn(&Transform) -> Option<Vec<f32>>| -> Option<Vec<f32>> {
        let pts: Vec<(f64, Vec<f32>)> = anim
            .keyframes
            .iter()
            .filter_map(|k| k.transforms.iter().find_map(|tr| f(tr)).map(|v| (k.time_ms as f64, v)))
            .collect();
        match pts.len() {
            0 => None,
            1 => Some(pts[0].1.clone()),
            n => {
                let (i, j) = if t <= pts[0].0 {
                    (0, 1)
                } else if t >= pts[n - 1].0 {
                    (n - 2, n - 1)
                } else {
                    let j = pts.iter().position(|p| p.0 >= t).unwrap_or(n - 1).max(1);
                    (j - 1, j)
                };
                let span = (pts[j].0 - pts[i].0).max(1e-9);
                let u = ((t - pts[i].0) / span) as f32;
                Some(pts[i].1.iter().zip(&pts[j].1).map(|(a, b)| a + (b - a) * u).collect())
            }
        }
    };
    if let Some(v) = pick(&|tr| match tr {
        Transform::Position(x, y) => Some(vec![*x, *y]),
        _ => None,
    }) {
        out.push(Transform::Position((v[0] * 100.0).round() / 100.0, (v[1] * 100.0).round() / 100.0));
    }
    if let Some(v) = pick(&|tr| match tr {
        Transform::Scale(sx, sy) => Some(vec![*sx, *sy]),
        _ => None,
    }) {
        out.push(Transform::Scale((v[0] * 10000.0).round() / 10000.0, (v[1] * 10000.0).round() / 10000.0));
    }
    if let Some(v) = pick(&|tr| match tr {
        Transform::Rotate(d) => Some(vec![*d]),
        _ => None,
    }) {
        out.push(Transform::Rotate((v[0] * 100.0).round() / 100.0));
    }
    out
}

/// One side of a dissolve: the neighbouring shot's camera plate, carried
/// on through the dissolve — the same mosaic, its camera extrapolated —
/// when that shot is one; else the frame at the dissolve's edge, frozen.
/// `offset_ms` is where the dissolve's first frame falls in the
/// neighbour's own time.
fn dissolve_side(
    neighbour: Option<&Document>,
    edge: &VideoFrame,
    offset_ms: f64,
    end_ms: u32,
    raster_quality: u8,
) -> Result<(Shape, Option<Animation>)> {
    let carried = neighbour.and_then(|d| {
        let scene = d.scenes.last()?;
        let plate = scene.statements.iter().find_map(|st| match st {
            Statement::Let { name, shape: shape @ Shape::Image { .. } } if name == "plate" => Some(shape.clone()),
            _ => None,
        })?;
        let camera = scene.statements.iter().find_map(|st| match st {
            Statement::Animate(a) if a.target == "plate" => Some(a.clone()),
            _ => None,
        })?;
        Some((plate, camera))
    });
    // v1.23: a neighbour's still plate is shared as it stands — it is
    // written once, and the dissolve draws it for nothing.
    if carried.is_none() {
        if let Some(plate) = neighbour.and_then(|d| d.scenes.last()).and_then(|sc| still_plate(sc, edge.width, edge.height)) {
            return Ok((plate, None));
        }
    }
    if let Some((plate, camera)) = carried {
        let keyframes = [0u32, end_ms]
            .iter()
            .map(|&t| Keyframe {
                time_ms: t,
                transforms: camera_at(&camera, offset_ms + t as f64),
            })
            .collect();
        return Ok((
            plate,
            Some(Animation {
                target: String::new(),
                keyframes,
                easing: Easing::Linear,
            }),
        ));
    }
    let src = vac_decompiler::jpeg_data_uri(edge, raster_quality)
        .ok_or_else(|| Error::Decompile(DecompileError::Other("could not encode a dissolve's picture".into())))?;
    Ok((
        Shape::Image {
            x: 0.0,
            y: 0.0,
            w: edge.width as f32,
            h: edge.height as f32,
            src,
        },
        None,
    ))
}

/// A dissolve's frames `[a, b)` as a scene of their own (v1.17, v1.18):
/// the shot before it under the shot after it, crossfading. Each side is
/// that shot's camera plate carried on through the dissolve when it has
/// one — a slideshow's photos keep zooming and panning while they dissolve
/// — or the frame at the dissolve's edge, frozen. The incoming side's
/// opacity is fitted to every frame against the two sides as rendered, by
/// least squares — `β = Σ (F − P)·(Q − P) / Σ |Q − P|²` — and simplified
/// like any opacity channel.
fn dissolve_shot(
    frames: &[VideoFrame],
    a: usize,
    b: usize,
    opts: &DecompileOptions,
    before: Option<&Document>,
    after: Option<&Document>,
) -> Result<Chosen> {
    // Carried cameras follow the photos' motion but know only what each
    // shot saw — a pan's mosaic has nothing for the strip the dissolve
    // still shows — while frozen pictures hold everything at the edge and
    // nothing of the motion. Both are built; the one that reconstructs the
    // dissolve's frames better is kept.
    let carried = dissolve_variant(frames, a, b, opts, before, after)?;
    if before.is_none() && after.is_none() {
        return Ok(carried);
    }
    let frozen = dissolve_variant(frames, a, b, opts, None, None)?;
    let shot = &frames[a..b];
    let (fc, ff) = (measure(shot, &carried.document)?, measure(shot, &frozen.document)?);
    // v1.23: a carried or shared picture is one the shot beside it already
    // writes, so it adds nothing to the file; that is worth its bytes, at
    // the price a sprite pays.
    let neighbours: Vec<&Document> = [before, after].into_iter().flatten().collect();
    let gain = improvement(&fc, &ff)
        + byte_price(opts.quality) * (new_picture_bytes(&frozen.document, &neighbours) as f64 - new_picture_bytes(&carried.document, &neighbours) as f64);
    Ok(if gain > 0.0 { carried } else { frozen })
}

/// The price per byte a picture, a sprite or a keyframe pays, in the
/// selection rule's currency, at this quality: [`SPRITE_MIN_GAIN_PER_BYTE`]
/// scaled by the dial.
fn byte_price(quality: f32) -> f64 {
    let m = 2f64.powf((50.0 - quality.clamp(0.0, 100.0) as f64) / 25.0);
    SPRITE_MIN_GAIN_PER_BYTE * m * m
}

/// The bytes of the pictures `doc` draws that none of `neighbours` already
/// draws (v1.23): what it adds to the file, since a picture is written
/// once however many scenes draw it.
fn new_picture_bytes(doc: &Document, neighbours: &[&Document]) -> usize {
    let srcs = |d: &Document| -> Vec<String> {
        d.scenes
            .iter()
            .flat_map(|s| &s.statements)
            .filter_map(|st| match st {
                Statement::Let { shape: Shape::Image { src, .. }, .. } => Some(src.clone()),
                _ => None,
            })
            .collect()
    };
    let known: std::collections::HashSet<String> = neighbours.iter().flat_map(|d| srcs(d)).collect();
    let mut mine = srcs(doc);
    mine.sort();
    mine.dedup();
    mine.iter().filter(|s| !known.contains(*s)).map(|s| s.len()).sum()
}

/// A scene's still plate: `let plate = image(…)` covering the whole canvas,
/// with no camera moving it — what a shot of a still picture is written as.
fn still_plate(scene: &Scene, w: u32, h: u32) -> Option<Shape> {
    if scene.statements.iter().any(|st| matches!(st, Statement::Animate(a) if a.target == "plate")) {
        return None;
    }
    scene.statements.iter().find_map(|st| match st {
        Statement::Let { name, shape: shape @ Shape::Image { x, y, w: pw, h: ph, .. } }
            if name == "plate" && *x <= 0.0 && *y <= 0.0 && x + pw >= w as f32 && y + ph >= h as f32 =>
        {
            Some(shape.clone())
        }
        _ => None,
    })
}

fn dissolve_variant(
    frames: &[VideoFrame],
    a: usize,
    b: usize,
    opts: &DecompileOptions,
    before: Option<&Document>,
    after: Option<&Document>,
) -> Result<Chosen> {
    let (p, q) = (&frames[a - 1], &frames[b.min(frames.len() - 1)]);
    let tol = vac_decompiler::Tolerances::from_quality(opts.quality);
    let fps = vac_decompiler::frames::estimate_fps(frames).max(1);
    let n = b - a;
    let time = |i: usize| ((i as u64 * 1000) / fps as u64) as u32;
    let end = time(n - 1);
    let duration_ms = ((n as u64 * 1000).div_ceil(fps as u64)) as u32;
    let before_ms = before.and_then(|d| d.scenes.last()).map(|s| s.duration_ms as f64).unwrap_or(0.0);
    let (from, from_cam) = dissolve_side(before, p, before_ms, end, tol.raster_quality)?;
    let (to, to_cam) = dissolve_side(after, q, -(duration_ms as f64), end, tol.raster_quality)?;
    let canvas = Canvas {
        width: p.width,
        height: p.height,
        fps,
    };
    // Each side alone, rendered over the dissolve's frames.
    let layer = |shape: &Shape, cam: &Option<Animation>, name: &str| -> Result<Vec<RenderedFrame>> {
        let mut statements = vec![Statement::Let {
            name: name.into(),
            shape: shape.clone(),
        }];
        if let Some(c) = cam {
            statements.push(Statement::Animate(Animation {
                target: name.into(),
                ..c.clone()
            }));
        }
        render(&Document {
            canvas: canvas.clone(),
            scenes: vec![Scene {
                name: "dissolve".into(),
                duration_ms,
                statements,
            }],
        })
    };
    let (pr, qr) = (layer(&from, &from_cam, "plate")?, layer(&to, &to_cam, "incoming")?);
    let beta: Vec<f32> = (0..n)
        .map(|i| {
            let f = &frames[a + i];
            let (pp, qq) = (&pr[i.min(pr.len() - 1)].rgba, &qr[i.min(qr.len() - 1)].rgba);
            let (mut num, mut den) = (0f64, 0f64);
            for ((fp, pp), qp) in f.pixels.chunks_exact(4).zip(pp.chunks_exact(4)).zip(qq.chunks_exact(4)).step_by(2) {
                for c in 0..3 {
                    let dv = qp[c] as f64 - pp[c] as f64;
                    num += (fp[c] as f64 - pp[c] as f64) * dv;
                    den += dv * dv;
                }
            }
            if den > 0.0 { (num / den).clamp(0.0, 1.0) as f32 } else { 0.5 }
        })
        .collect();
    let opacity_tol = tol.opacity_levels / 255.0;
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((x, y)) = stack.pop() {
        if y <= x + 1 {
            continue;
        }
        let (worst, at) = (x + 1..y)
            .map(|t| {
                let u = (time(t) - time(x)) as f32 / (time(y) - time(x)).max(1) as f32;
                ((beta[x] + (beta[y] - beta[x]) * u - beta[t]).abs(), t)
            })
            .fold((0.0f32, x), |m, v| if v.0 > m.0 { v } else { m });
        if worst > opacity_tol {
            keep[at] = true;
            stack.push((x, at));
            stack.push((at, y));
        }
    }
    let round = |v: f32| (v * 1000.0).round() / 1000.0;
    let mut incoming: Vec<Keyframe> = (0..n)
        .filter(|&i| keep[i])
        .map(|i| Keyframe {
            time_ms: time(i),
            transforms: vec![Transform::Opacity(round(beta[i]))],
        })
        .collect();
    if let Some(cam) = &to_cam {
        for k in &cam.keyframes {
            match incoming.iter_mut().find(|x| x.time_ms == k.time_ms) {
                Some(x) => x.transforms.extend(k.transforms.clone()),
                None => incoming.push(k.clone()),
            }
        }
        incoming.sort_by_key(|k| k.time_ms);
    }
    let mut statements = vec![Statement::Let {
        name: "plate".into(),
        shape: from,
    }];
    if let Some(c) = from_cam {
        statements.push(Statement::Animate(Animation {
            target: "plate".into(),
            ..c
        }));
    }
    statements.push(Statement::Let {
        name: "incoming".into(),
        shape: to,
    });
    statements.push(Statement::Animate(Animation {
        target: "incoming".into(),
        keyframes: incoming,
        easing: Easing::Linear,
    }));
    Ok(Chosen {
        document: Document {
            canvas,
            scenes: vec![Scene {
                name: "dissolve".into(),
                duration_ms,
                statements,
            }],
        },
        segmentation: Segmentation::Components,
        background: Background::Corners,
        plate: Plate::Raster,
        static_layer: false,
        fidelity: None,
        audited_out: 0,
        refined: 0,
        moved: 0,
        sprites: 0,
    })
}

/// A cut differs from the frame before it at least this many times more
/// than the clip's typical step from one frame to the next...
const CUT_RATIO: f64 = 10.0;
/// ...and by at least this much (per-channel MSE: 20 levels RMS), so a
/// still clip's sensor noise is not a cut.
const CUT_FLOOR_MSE: f64 = 400.0;
/// No shot shorter than this: a flash is two cuts a frame apart, not a
/// shot.
const MIN_SHOT_FRAMES: usize = 6;
/// The clip-as-one-shot check runs for at most this many cut candidates...
const WHOLE_CHECK_MAX_CUTS: usize = 3;
/// ...in a clip of at most this many frames.
const WHOLE_CHECK_MAX_FRAMES: usize = 300;

/// Where the clip may cut to a new shot (v1.12): frames that differ from
/// the frame before them far more than the clip's frames typically differ
/// from each other. These are candidates only — `decompile_chosen` keeps a
/// split only when it measures better than the clip as one shot.
fn cut_candidates(frames: &[VideoFrame]) -> Vec<usize> {
    if frames.len() < 2 * MIN_SHOT_FRAMES {
        return Vec::new();
    }
    let (w, h) = (frames[0].width, frames[0].height);
    if frames.iter().any(|f| f.width != w || f.height != h) {
        return Vec::new();
    }
    let diffs: Vec<f64> = (1..frames.len()).map(|i| frame_step(&frames[i - 1], &frames[i])).collect();
    cuts_from_steps(&diffs)
}

/// How far a frame moved from the one before it: per-channel mean squared
/// difference over every third pixel.
fn frame_step(a: &VideoFrame, b: &VideoFrame) -> f64 {
    let (mut s, mut n) = (0f64, 0f64);
    for i in (0..(a.width * a.height) as usize).step_by(3) {
        for c in 0..3 {
            let d = a.pixels[i * 4 + c] as f64 - b.pixels[i * 4 + c] as f64;
            s += d * d;
        }
        n += 3.0;
    }
    s / n.max(1.0)
}

/// [`cut_candidates`] from the steps between consecutive frames
/// (`diffs[k]` is frame `k + 1` against frame `k`).
fn cuts_from_steps(diffs: &[f64]) -> Vec<usize> {
    let len = diffs.len() + 1;
    if len < 2 * MIN_SHOT_FRAMES || diffs.is_empty() {
        return Vec::new();
    }
    let mut sorted = diffs.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let typical = sorted[sorted.len() / 2];
    let bar = (CUT_RATIO * typical).max(CUT_FLOOR_MSE);
    let mut cuts: Vec<usize> = Vec::new();
    for (k, &d) in diffs.iter().enumerate() {
        let i = k + 1;
        let since = cuts.last().copied().unwrap_or(0);
        if d >= bar && i - since >= MIN_SHOT_FRAMES && len - i >= MIN_SHOT_FRAMES {
            cuts.push(i);
        }
    }
    cuts
}

/// The shots' documents as the scenes of one: the canvas and frame rate
/// of the whole clip, each scene exactly as long as its shot's frames.
fn merge_shots<'a>(frames: &[VideoFrame], edges: &[usize], parts: impl Iterator<Item = &'a Document>) -> Document {
    let fps = vac_decompiler::frames::estimate_fps(frames).max(1);
    let mut scenes = Vec::new();
    for (n, (part, w)) in parts.zip(edges.windows(2)).enumerate() {
        let count = (w[1] - w[0]) as u64;
        let mut scene = part.scenes[0].clone();
        scene.name = format!("shot_{}", n + 1);
        // ceil(count * 1000 / fps): the renderer draws floor(duration *
        // fps / 1000) frames, so this is exactly `count` of them.
        scene.duration_ms = ((count * 1000 + fps as u64 - 1) / fps as u64) as u32;
        scenes.push(scene);
    }
    Document {
        canvas: Canvas {
            width: frames[0].width,
            height: frames[0].height,
            fps,
        },
        scenes,
    }
}

/// Decompile one shot: the measured search, then every pass.
fn decompile_shot(frames: &[VideoFrame], opts: &DecompileOptions) -> Result<Chosen> {
    let chosen = choose(frames, opts)?;
    if !opts.audit && !opts.refine && !opts.motion && !opts.sprites {
        return Ok(chosen);
    }
    let fidelity = match chosen.fidelity {
        Some(f) => f,
        None => measure(frames, &chosen.document)?,
    };
    let (document, fidelity) = if opts.audit {
        audit_background(frames, chosen.document, fidelity, opts.quality)?
    } else {
        (chosen.document, fidelity)
    };
    let a = if opts.audit {
        audit_measured(frames, document, fidelity)?
    } else {
        Audit {
            document,
            fidelity,
            removed: Vec::new(),
        }
    };
    let (document, fidelity, refined) = if opts.refine {
        let r = refine_measured(frames, a.document, a.fidelity)?;
        (r.document, r.fidelity, r.refined.len())
    } else {
        (a.document, a.fidelity, 0)
    };
    let (document, fidelity, moved) = if opts.motion {
        let m = motion_measured(frames, document, fidelity, opts.quality)?;
        (m.document, m.fidelity, m.moved.len())
    } else {
        (document, fidelity, 0)
    };
    let (document, fidelity, sprites) = if opts.sprites {
        let sp = sprites_measured(frames, document, fidelity, opts.quality)?;
        (sp.document, sp.fidelity, sp.sprites.len())
    } else {
        (document, fidelity, 0)
    };
    Ok(Chosen {
        document,
        fidelity: Some(fidelity),
        audited_out: a.removed.len(),
        refined,
        moved,
        sprites,
        ..chosen
    })
}

/// The background's colour keyframes (v1.24) earn their place like any
/// shape: the clip is measured without them, the background held at its
/// declared fill, and they stay only when the clip is better with them by
/// more than their bytes cost at the sprite price.
fn audit_background(frames: &[VideoFrame], doc: Document, fid: Fidelity, quality: f32) -> Result<(Document, Fidelity)> {
    let keyed = |st: &Statement| matches!(st, Statement::Animate(a) if a.target == "bg");
    if !doc.scenes.iter().any(|s| s.statements.iter().any(keyed)) {
        return Ok((doc, fid));
    }
    let mut without = doc.clone();
    for s in &mut without.scenes {
        s.statements.retain(|st| !keyed(st));
    }
    let fw = measure(frames, &without)?;
    let bytes = vac_format::write(&doc).len().saturating_sub(vac_format::write(&without).len()) as f64;
    Ok(if improvement(&fid, &fw) > byte_price(quality) * bytes { (doc, fid) } else { (without, fw) })
}

/// Frames with more pixels than this are searched at a reduced size (v1.14).
/// Every corpus clip is below it (the largest, `real_pendulum`, is 459k).
const SEARCH_MAX_PIXELS: u64 = 500_000;

/// The measured search, made affordable for large frames (v1.14). Every
/// candidate of the search decompiles and measures the whole clip, and at
/// 720p each holds its own copy of it: 3 s of a camera pan took ~100 s and
/// 2.5 GB. Above `SEARCH_MAX_PIXELS` the search runs on the clip shrunk by
/// a whole factor (box-filtered, same timing), and only its decision — the
/// segmentation, the background rule, the plate — is replayed at full
/// size, as a single candidate. The decision is still measured, on the
/// smaller clip; everything after it (audit, refinement, motion, sprites)
/// runs at full size as before.
fn choose(frames: &[VideoFrame], opts: &DecompileOptions) -> Result<Chosen> {
    let (w, h) = (frames[0].width as u64, frames[0].height as u64);
    let undecided = opts.segmentation == Segmentation::Auto
        || opts.background == Background::Auto
        || opts.plate == Plate::Auto;
    if w * h <= SEARCH_MAX_PIXELS || !undecided || frames.iter().any(|f| f.width as u64 != w || f.height as u64 != h) {
        return choose_full(frames, opts);
    }
    let factor = ((w * h) as f64 / SEARCH_MAX_PIXELS as f64).sqrt().ceil() as u32;
    let small = downscale(frames, factor);
    let picked = choose_full(&small, opts)?;
    let replay = DecompileOptions {
        segmentation: picked.segmentation,
        background: picked.background,
        plate: picked.plate,
        static_layer: picked.static_layer,
        ..*opts
    };
    let mut chosen = choose_full(frames, &replay)?;
    chosen.fidelity = None;
    Ok(chosen)
}

/// `frames` shrunk by a whole `factor` in each direction, each pixel the
/// mean of the block it covers; timing unchanged.
fn downscale(frames: &[VideoFrame], factor: u32) -> Vec<VideoFrame> {
    let f = factor.max(1);
    frames
        .iter()
        .map(|fr| {
            let (w, h) = ((fr.width / f).max(1), (fr.height / f).max(1));
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            for y in 0..h {
                for x in 0..w {
                    let mut acc = [0u32; 4];
                    for dy in 0..f {
                        for dx in 0..f {
                            let (sx, sy) = ((x * f + dx).min(fr.width - 1), (y * f + dy).min(fr.height - 1));
                            let p = ((sy * fr.width + sx) * 4) as usize;
                            for c in 0..4 {
                                acc[c] += fr.pixels[p + c] as u32;
                            }
                        }
                    }
                    let q = ((y * w + x) * 4) as usize;
                    for c in 0..4 {
                        pixels[q + c] = (acc[c] / (f * f)) as u8;
                    }
                }
            }
            VideoFrame {
                width: w,
                height: h,
                pixels,
                delay_ms: fr.delay_ms,
            }
        })
        .collect()
}

/// How much better a candidate without the static layer must measure, in
/// the selection rule's currency, to replace the one with it (v1.20): twice
/// as good, the raster plate's bar. Candidates are compared before the
/// passes that improve the winner, and a near tie there says nothing about
/// the result: `real_loader` took the layer's absence on a 0.058 margin and
/// finished worse (ink IoU 0.864 → 0.806, 43% more bytes); the slow card in
/// `cuts_three_shots` wins by 2.8.
const STATIC_LAYER_MIN_GAIN: f64 = std::f64::consts::LN_2;

fn choose_full(frames: &[VideoFrame], opts: &DecompileOptions) -> Result<Chosen> {
    let segs: &[Segmentation] = match opts.segmentation {
        Segmentation::Auto => &[Segmentation::Components, Segmentation::Regions, Segmentation::Edges],
        Segmentation::Components => &[Segmentation::Components],
        Segmentation::Regions => &[Segmentation::Regions],
        Segmentation::Edges => &[Segmentation::Edges],
    };
    let bgs: &[Background] = match opts.background {
        Background::Auto => &[Background::Corners, Background::Border],
        Background::Corners => &[Background::Corners],
        Background::Border => &[Background::Border],
    };
    if segs.len() * bgs.len() == 1 && opts.plate != Plate::Auto {
        let cand_opts = DecompileOptions {
            segmentation: segs[0],
            background: bgs[0],
            ..*opts
        };
        return Ok(Chosen {
            document: vac_decompiler::decompile_frames(frames.to_vec(), &cand_opts)?,
            segmentation: segs[0],
            background: bgs[0],
            plate: opts.plate,
            static_layer: opts.static_layer,
            fidelity: None,
            audited_out: 0,
            refined: 0,
            moved: 0,
            sprites: 0,
        });
    }
    // The static layer and its stripping depend only on the background
    // rule, so each background is prepared once and its segmentation
    // candidates share it read-only. The backgrounds are prepared side by
    // side and every candidate runs concurrently, at most
    // `MAX_CONCURRENT_CANDIDATES` at once. The winner is then picked in a
    // fixed candidate order, so the result is the same as a sequential
    // search.
    let width = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, MAX_CONCURRENT_CANDIDATES);
    let per_background = (width / bgs.len()).max(1);
    type Run = ((Background, Segmentation), Result<(Document, Fidelity)>);
    let runs: Vec<Result<Vec<Run>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = bgs
            .iter()
            .map(|&background| {
                scope.spawn(move || -> Result<Vec<Run>> {
                    let prepared = vac_decompiler::prepare(frames.to_vec(), opts.static_layer, background)?;
                    let mut out: Vec<Run> = Vec::new();
                    for chunk in segs.chunks(per_background) {
                        let chunk_results: Vec<Result<(Document, Fidelity)>> = std::thread::scope(|inner| {
                            let handles: Vec<_> = chunk
                                .iter()
                                .map(|&segmentation| {
                                    let prepared = &prepared;
                                    inner.spawn(move || -> Result<(Document, Fidelity)> {
                                        let doc =
                                            vac_decompiler::finish(prepared, segmentation, opts.quality, Plate::Vector);
                                        let fid = measure(frames, &doc)?;
                                        Ok((doc, fid))
                                    })
                                })
                                .collect();
                            handles
                                .into_iter()
                                .map(|h| h.join().expect("candidate thread panicked"))
                                .collect()
                        });
                        out.extend(chunk.iter().map(|&s| (background, s)).zip(chunk_results));
                    }
                    Ok(out)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("background thread panicked"))
            .collect()
    });
    let mut combos: Vec<(Background, Segmentation)> = Vec::new();
    let mut results: Vec<Result<(Document, Fidelity)>> = Vec::new();
    for run in runs {
        for (combo, result) in run? {
            combos.push(combo);
            results.push(result);
        }
    }
    let mut best: Option<Chosen> = None;
    for ((background, segmentation), result) in combos.into_iter().zip(results) {
        let (doc, fid) = result?;
        let wins = match &best {
            None => true,
            Some(b) => reconstructs_better(&fid, b.fidelity.as_ref().expect("measured")),
        };
        if wins {
            best = Some(Chosen {
                document: doc,
                segmentation,
                background,
                plate: Plate::Vector,
                static_layer: opts.static_layer,
                fidelity: Some(fid),
                audited_out: 0,
                refined: 0,
                moved: 0,
                sprites: 0,
            });
        }
    }
    let mut best = best.expect("at least one candidate");
    // v1.20: the static layer is measured, not assumed. A mover slow enough
    // to cover the same pixels for most of a shot is, to the temporal
    // median, scenery: its edges are stripped from every frame and come
    // back as static bars while what is left of it fragments (the first
    // 45 frames of `outlined_ui`: 24.5 dB with the layer, 34.1 without).
    // When the winner has static shapes, its segmentation and background
    // are decompiled once more without the layer, and that is kept when it
    // measures clearly better ([`STATIC_LAYER_MIN_GAIN`]).
    let has_static = |doc: &Document| {
        doc.scenes
            .iter()
            .flat_map(|s| &s.statements)
            .any(|st| matches!(st, Statement::Let { name, .. } if name.starts_with("static_")))
    };
    if opts.static_layer && has_static(&best.document) {
        let (segmentation, background) = (best.segmentation, best.background);
        let prepared = vac_decompiler::prepare(frames.to_vec(), false, background)?;
        let doc = vac_decompiler::finish(&prepared, segmentation, opts.quality, Plate::Vector);
        let fid = measure(frames, &doc)?;
        if improvement(&fid, best.fidelity.as_ref().expect("measured")) >= STATIC_LAYER_MIN_GAIN {
            // Without the layer the segmentations are a different contest:
            // which one won with it says nothing about which wins without
            // (v1.21: `outlined_ui`'s first 45 frames went to edges with
            // the layer, 32.7 dB without it, where regions gave 34.1). Each
            // is decompiled on the same layerless frames, the best kept.
            let others: Vec<Segmentation> = segs.iter().copied().filter(|&s| s != segmentation).collect();
            let tried: Vec<Result<(Segmentation, Document, Fidelity)>> = std::thread::scope(|scope| {
                let handles: Vec<_> = others
                    .iter()
                    .map(|&seg| {
                        let prepared = &prepared;
                        scope.spawn(move || -> Result<(Segmentation, Document, Fidelity)> {
                            let doc = vac_decompiler::finish(prepared, seg, opts.quality, Plate::Vector);
                            let fid = measure(frames, &doc)?;
                            Ok((seg, doc, fid))
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().expect("candidate thread panicked"))
                    .collect()
            });
            let (mut seg_best, mut doc_best, mut fid_best) = (segmentation, doc, fid);
            for t in tried {
                let (seg, doc, fid) = t?;
                if reconstructs_better(&fid, &fid_best) {
                    (seg_best, doc_best, fid_best) = (seg, doc, fid);
                }
            }
            best = Chosen {
                document: doc_best,
                segmentation: seg_best,
                static_layer: false,
                fidelity: Some(fid_best),
                ..best
            };
        }
    }
    // v1.6: the raster plate is an escape hatch, not a peer. It is tried on
    // the winning vector candidate's segmentation and background, and kept
    // only when it reconstructs *clearly* better — the source stays vector
    // unless measurement says vectors cannot describe the scenery.
    if opts.plate == Plate::Vector {
        return Ok(best);
    }
    // Vectors that already describe the scene well keep it: a JPEG would
    // always shave a little more error off (it halved `shaded.vac`'s
    // residual at 39.5 dB) at the cost of readable, editable source.
    if opts.plate == Plate::Auto
        && best.fidelity.as_ref().is_some_and(|f| f.psnr_db >= RASTER_ONLY_BELOW_PSNR_DB)
    {
        return Ok(best);
    }
    // Each plate is tried on the winning vector candidate's segmentation
    // and background: the still raster plate (v1.6) and, when the whole
    // picture moves, the camera's mosaic (v1.10). The better of them is
    // kept only if it beats the vectors clearly — or when asked for.
    let mut plated: Vec<Chosen> = Vec::new();
    let has_plate = |doc: &Document| {
        doc.scenes
            .iter()
            .flat_map(|s| &s.statements)
            .any(|st| matches!(st, Statement::Let { name, shape: Shape::Image { .. } } if name == "plate"))
    };
    let tries: &[Plate] = match opts.plate {
        Plate::Raster => &[Plate::Raster],
        Plate::Camera => &[Plate::Camera],
        _ => &[Plate::Raster, Plate::Camera],
    };
    // The plates are independent of each other, so they are tried side by
    // side.
    let (segmentation, background) = (best.segmentation, best.background);
    let tried: Vec<Result<Option<Chosen>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = tries
            .iter()
            .map(|&plate| {
                scope.spawn(move || -> Result<Option<Chosen>> {
                    let prepared = match plate {
                        Plate::Camera => vac_decompiler::prepare_camera(frames.to_vec(), background)?,
                        _ => vac_decompiler::prepare_raster(frames.to_vec(), background)?,
                    };
                    let Some(prepared) = prepared else { return Ok(None) };
                    let doc = vac_decompiler::finish(&prepared, segmentation, opts.quality, plate);
                    if !has_plate(&doc) {
                        return Ok(None);
                    }
                    let fid = measure(frames, &doc)?;
                    Ok(Some(Chosen {
                        document: doc,
                        segmentation,
                        background,
                        plate,
                        static_layer: opts.static_layer,
                        fidelity: Some(fid),
                        audited_out: 0,
                        refined: 0,
                        moved: 0,
                        sprites: 0,
                    }))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("plate thread panicked"))
            .collect()
    });
    for t in tried {
        if let Some(c) = t? {
            plated.push(c);
        }
    }
    let top = plated.into_iter().reduce(|a, b| {
        if reconstructs_better(b.fidelity.as_ref().expect("measured"), a.fidelity.as_ref().expect("measured")) {
            b
        } else {
            a
        }
    });
    let Some(top) = top else {
        return Ok(best);
    };
    let vector = best.fidelity.as_ref().expect("measured");
    let keep = opts.plate != Plate::Auto
        || improvement(top.fidelity.as_ref().expect("measured"), vector) >= RASTER_MIN_GAIN;
    Ok(if keep { top } else { best })
}

/// What the shape audit (v1.7) did to a document.
#[derive(Debug, Clone)]
pub struct Audit {
    /// The document with every shape the reconstruction is no better with
    /// taken out, and the remaining `shape_<n>`, `static_<n>` and
    /// `ground_<n>` renumbered without gaps.
    pub document: Document,
    /// Its measured fidelity.
    pub fidelity: Fidelity,
    /// The names, as they were in the input, of the shapes taken out.
    pub removed: Vec<String>,
}

/// Every shape earns its place, measured (v1.7).
///
/// The tracker emits a shape for every track that clears its gates, and
/// v1.0's pruning only *estimated* what a shape is worth — its area times
/// its contrast against one flat background, which says nothing over a
/// raster plate or a gradient. Here each shape is taken out of the
/// reconstruction in turn, topmost first, and only its own footprint is
/// re-rendered without it ([`vac_compiler::render::Rig`]). The per-frame
/// error and ink counts the full measurement already holds are updated by
/// what changed there, so the clip's score without the shape is known
/// exactly from a few small renders. The shape stays only if the clip
/// reconstructs better *with* it, in the selection rule's own currency
/// ([`improvement`]); a shape that changes nothing, or makes things worse,
/// goes. The result is re-measured in full and kept only if it measures at
/// least as well as what went in.
///
/// Works on any single-scene document — VAC's own output, an older
/// version's, or a hand-edited source — and `decompile` runs it on every
/// result unless [`DecompileOptions::audit`] is off.
pub fn audit(original: &[VideoFrame], doc: Document) -> Result<Audit> {
    let fid = measure(original, &doc)?;
    audit_measured(original, doc, fid)
}

/// [`audit`] on a document whose fidelity is already measured.
fn audit_measured(frames: &[VideoFrame], doc: Document, fid: Fidelity) -> Result<Audit> {
    let unchanged = |document: Document, fidelity: Fidelity| Audit {
        document,
        fidelity,
        removed: Vec::new(),
    };
    if doc.scenes.len() != 1 || fid.frames.len() != frames.len() {
        return Ok(unchanged(doc, fid));
    }
    let rig = vac_compiler::render::Rig::new(&doc)?;
    let count = rig.frame_count();
    let idx = aligned_indices(frames, count, doc.canvas.fps);
    let mut wanted: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (j, &i) in idx.iter().enumerate() {
        wanted[i].push(j);
    }
    let bg = ink_background(frames);
    let opts = CompareOptions::default();
    let names: Vec<String> = rig.shapes(0).to_vec();
    let bounds: Vec<Vec<Option<[u32; 4]>>> =
        (0..count).map(|i| (0..names.len()).map(|k| rig.bounds(i, k)).collect()).collect();
    let overlaps = |a: [u32; 4], b: [u32; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];

    // Each source frame's raw tallies, exactly as `compare_frame` counted
    // them, and the clip score they pool to.
    let compared: Vec<f64> = fid.frames.iter().map(|f| f.compared_pixels as f64).collect();
    let mut sse: Vec<f64> = fid.frames.iter().map(|f| f.residual * 3.0 * f.compared_pixels as f64).collect();
    let mut inter: Vec<i64> = fid.frames.iter().map(|f| (f.source_ink_px - f.missing_px) as i64).collect();
    let mut union: Vec<i64> = fid.frames.iter().map(|f| (f.source_ink_px + f.spurious_px) as i64).collect();
    let frame_res = |sse: f64, c: f64| if c > 0.0 { sse / (3.0 * c) } else { 0.0 };
    let frame_iou = |i: i64, u: i64| if u > 0 { i as f64 / u as f64 } else { 1.0 };
    let mut res_sum: f64 = (0..frames.len()).map(|j| frame_res(sse[j], compared[j])).sum();
    let mut iou_sum: f64 = (0..frames.len()).map(|j| frame_iou(inter[j], union[j])).sum();

    // One pass, topmost first: a second pass was measured to take out
    // nothing more on any corpus clip.
    let mut removed = vec![false; names.len()];
    for k in (0..names.len()).rev() {
        if names[k] == "bg" || names[k] == "plate" {
            continue;
        }
        // What taking shape `k` out changes, per source frame.
        let mut deltas: Vec<(usize, f64, i64, i64)> = Vec::new();
        for i in 0..count {
            let Some(r) = bounds[i][k] else { continue };
            if wanted[i].is_empty() {
                continue;
            }
            let admit = |m: usize| !removed[m] && bounds[i][m].is_some_and(|b| overlaps(b, r));
            let with = rig.render_region(i, r, admit);
            let without = rig.render_region(i, r, |m| m != k && admit(m));
            let rw = (r[2] - r[0]) as usize;
            for &j in &wanted[i] {
                let src = &frames[j];
                let (mut ds, mut di, mut du) = (0i64, 0i64, 0i64);
                for y in r[1]..r[3] {
                    for x in r[0]..r[2] {
                        let q = ((y - r[1]) as usize * rw + (x - r[0]) as usize) * 4;
                        let (a, b) = (&with[q..q + 4], &without[q..q + 4]);
                        if a == b {
                            continue;
                        }
                        let p = ((y * src.width + x) * 4) as usize;
                        let o = &src.pixels[p..p + 4];
                        if o[3] < vac_metrics::MIN_OPAQUE_ALPHA {
                            continue;
                        }
                        let sq = |c: &[u8]| -> i64 {
                            (0..3).map(|n| (o[n] as i64 - c[n] as i64).pow(2)).sum()
                        };
                        ds += sq(b) - sq(a);
                        let oi = is_ink(o, bg, &opts);
                        let (ai, bi) = (is_ink(a, bg, &opts), is_ink(b, bg, &opts));
                        di += (oi && bi) as i64 - (oi && ai) as i64;
                        du += (oi || bi) as i64 - (oi || ai) as i64;
                    }
                }
                if ds != 0 || di != 0 || du != 0 {
                    deltas.push((j, ds as f64, di, du));
                }
            }
        }
        let (mut res_wo, mut iou_wo) = (res_sum, iou_sum);
        for &(j, ds, di, du) in &deltas {
            res_wo += frame_res(sse[j] + ds, compared[j]) - frame_res(sse[j], compared[j]);
            iou_wo += frame_iou(inter[j] + di, union[j] + du) - frame_iou(inter[j], union[j]);
        }
        // How much better the clip is *with* shape `k` than without it.
        let gain = (res_wo.max(1e-9) / res_sum.max(1e-9)).ln() + (iou_sum.max(1e-9) / iou_wo.max(1e-9)).ln();
        if gain <= 0.0 {
            removed[k] = true;
            res_sum = res_wo;
            iou_sum = iou_wo;
            for (j, ds, di, du) in deltas {
                sse[j] += ds;
                inter[j] += di;
                union[j] += du;
            }
        }
    }

    let out: std::collections::HashSet<&str> =
        names.iter().zip(&removed).filter(|(_, &r)| r).map(|(n, _)| n.as_str()).collect();
    if out.is_empty() {
        return Ok(unchanged(doc, fid));
    }
    let mut pruned = doc.clone();
    pruned.scenes[0].statements.retain(|st| match st {
        Statement::Let { name, .. } => !out.contains(name.as_str()),
        Statement::Assign { target, .. } => !out.contains(target.as_str()),
        Statement::Animate(a) => !out.contains(a.target.as_str()),
    });
    renumber(&mut pruned);
    let pfid = measure(frames, &pruned)?;
    if reconstructs_better(&fid, &pfid) {
        return Ok(unchanged(doc, fid));
    }
    let removed = names.iter().zip(&removed).filter(|(_, &r)| r).map(|(n, _)| n.clone()).collect();
    Ok(Audit {
        document: pruned,
        fidelity: pfid,
        removed,
    })
}

/// What the refinement pass (v1.9) did to a document.
#[derive(Debug, Clone)]
pub struct Refined {
    pub document: Document,
    pub fidelity: Fidelity,
    /// The shapes whose geometry moved.
    pub refined: Vec<String>,
}

/// The fit is whatever reconstructs best (v1.9).
///
/// Every shape's geometry is *estimated* from its detected blob — a radius
/// from coverage, a position from a centroid — and through v1.8 the
/// estimate was final. Nothing checked it against the reconstruction, and
/// on a thin, high-contrast stroke a fifth of a pixel is decibels:
/// `outlined.vac`'s ring came back with radius 49.8 for 50 and gave up
/// 5.7 dB. Here each shape's size, position and stroke width are nudged,
/// by coordinate descent in steps of 0.5, 0.2 and 0.1 px, and a nudge is
/// kept when the clip reconstructs better — scored like the audit, from
/// small renders of the shape's footprint against the full measurement's
/// per-frame tallies, on a sample of frames while searching and on every
/// frame before it is accepted. Nothing is classified; the numbers move
/// to wherever the pixels agree best, and stay readable numbers.
pub fn refine(original: &[VideoFrame], doc: Document) -> Result<Refined> {
    let fid = measure(original, &doc)?;
    refine_measured(original, doc, fid)
}

/// The steps the refinement searches with, coarse to fine, pixels. The
/// finest is the precision positions are written at.
const REFINE_STEPS_PX: [f32; 3] = [0.5, 0.2, 0.1];
/// How far refinement may move a shape or change its size, pixels: a
/// correction to an estimate, not a refit.
const REFINE_MAX_PX: f32 = 3.0;
/// Frames the search scores each trial on (evenly spaced over the shape's
/// lifetime); the winner is then scored on every frame.
const REFINE_SAMPLE_FRAMES: usize = 12;
/// A footprint already reconstructed this well has nothing to gain.
const REFINE_SKIP_ABOVE_PSNR_DB: f64 = 45.0;

/// [`refine`] on a document whose fidelity is already measured.
fn refine_measured(frames: &[VideoFrame], doc: Document, fid: Fidelity) -> Result<Refined> {
    use vac_compiler::render::{Rig, Tweak};
    let unchanged = |document: Document, fidelity: Fidelity| Refined {
        document,
        fidelity,
        refined: Vec::new(),
    };
    if doc.scenes.len() != 1 || fid.frames.len() != frames.len() {
        return Ok(unchanged(doc, fid));
    }
    let mut rig = Rig::new(&doc)?;
    let count = rig.frame_count();
    let idx = aligned_indices(frames, count, doc.canvas.fps);
    let mut wanted: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (j, &i) in idx.iter().enumerate() {
        wanted[i].push(j);
    }
    let bg = ink_background(frames);
    let opts = CompareOptions::default();
    let compared: Vec<f64> = fid.frames.iter().map(|f| f.compared_pixels as f64).collect();
    let mut sse: Vec<f64> = fid.frames.iter().map(|f| f.residual * 3.0 * f.compared_pixels as f64).collect();
    let mut inter: Vec<i64> = fid.frames.iter().map(|f| (f.source_ink_px - f.missing_px) as i64).collect();
    let mut union: Vec<i64> = fid.frames.iter().map(|f| (f.source_ink_px + f.spurious_px) as i64).collect();
    let frame_res = |sse: f64, c: f64| if c > 0.0 { sse / (3.0 * c) } else { 0.0 };
    let frame_iou = |i: i64, u: i64| if u > 0 { i as f64 / u as f64 } else { 1.0 };
    let mut res_sum: f64 = (0..frames.len()).map(|j| frame_res(sse[j], compared[j])).sum();
    let mut iou_sum: f64 = (0..frames.len()).map(|j| frame_iou(inter[j], union[j])).sum();

    let names: Vec<String> = rig.shapes(0).to_vec();
    let mut bounds: Vec<Vec<Option<[u32; 4]>>> =
        (0..count).map(|i| (0..names.len()).map(|m| rig.bounds(i, m)).collect()).collect();
    let overlaps = |a: Option<[u32; 4]>, b: [u32; 4]| {
        a.is_some_and(|a| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3])
    };
    let (cw, ch) = (doc.canvas.width, doc.canvas.height);
    // Every pixel any trial can touch: the footprint, grown by the most a
    // trial may move or grow the shape, and its stroke.
    let pad = |b: [u32; 4]| -> [u32; 4] {
        let p = (REFINE_MAX_PX * 2.0).ceil() as u32 + 2;
        [b[0].saturating_sub(p), b[1].saturating_sub(p), (b[2] + p).min(cw), (b[3] + p).min(ch)]
    };

    let mut current = doc.clone();
    let mut refined: Vec<String> = Vec::new();
    for (k, name) in names.iter().enumerate() {
        if name == "bg" || name == "plate" {
            continue;
        }
        let Some(Statement::Let { shape: base, .. }) = current.scenes[0]
            .statements
            .iter()
            .find(|st| matches!(st, Statement::Let { name: n, .. } if n == name))
            .cloned()
        else {
            continue;
        };
        if matches!(base, Shape::Image { .. }) {
            continue;
        }
        let stroked = current.scenes[0].statements.iter().any(|st| {
            matches!(st, Statement::Assign { target, property: Property::Stroke, value: Value::Color(_) } if target == name)
        });
        let stroke_width = current.scenes[0]
            .statements
            .iter()
            .find_map(|st| match st {
                Statement::Assign { target, property: Property::StrokeWidth, value: Value::Number(w) }
                    if target == name =>
                {
                    Some(*w)
                }
                _ => None,
            })
            .unwrap_or(2.0);
        let alive: Vec<usize> = (0..count)
            .filter(|&i| !wanted[i].is_empty() && bounds[i][k].is_some())
            .collect();
        if alive.is_empty() {
            continue;
        }
        // Score the shape's footprint as it stands; skip it if it is
        // already reconstructed about as well as pixels allow.
        let regions: Vec<[u32; 4]> = alive.iter().map(|&i| pad(bounds[i][k].expect("alive"))).collect();
        let before_tallies: Vec<Vec<(f64, usize, i64, i64)>> = alive
            .iter()
            .zip(&regions)
            .map(|(&i, &r)| {
                let img = rig.render_region(i, r, |m| overlaps(bounds[i][m], r));
                wanted[i].iter().map(|&j| region_tallies(&frames[j], &img, r, bg, &opts)).collect()
            })
            .collect();
        let (fs, fp) = before_tallies.iter().flatten().fold((0.0f64, 0usize), |a, t| (a.0 + t.0, a.1 + t.1));
        if fp == 0 || vac_metrics::psnr_from_mse(fs / (3.0 * fp as f64)) >= REFINE_SKIP_ABOVE_PSNR_DB {
            continue;
        }
        // Parameters: offset x, offset y, the two sizes (not for paths),
        // and the stroke width when stroked.
        let mut params: Vec<usize> = vec![0, 1];
        if !matches!(base, Shape::Path(_)) {
            params.extend([2, 3]);
        }
        if stroked {
            params.push(4);
        }
        let make = |p: &[f32; 5]| -> Tweak {
            let resized = match &base {
                Shape::Rect { x, y, w, h } => Some(Shape::Rect {
                    x: *x,
                    y: *y,
                    w: (w + p[2]).max(0.5),
                    h: (h + p[3]).max(0.5),
                }),
                Shape::Ellipse { cx, cy, rx, ry } => Some(Shape::Ellipse {
                    cx: *cx,
                    cy: *cy,
                    rx: (rx + p[2]).max(0.25),
                    ry: (ry + p[3]).max(0.25),
                }),
                _ => None,
            };
            Tweak {
                shape: k,
                base: resized.filter(|_| p[2] != 0.0 || p[3] != 0.0),
                offset: (p[0], p[1]),
                stroke_width: (p[4] != 0.0).then(|| (stroke_width + p[4]).max(0.5)),
                animation: None,
            }
        };
        // The clip score with tweak `p`, from the frames in `which`.
        let score = |p: &[f32; 5], which: &[usize]| -> (f64, Vec<(usize, f64, i64, i64)>) {
            let tw = make(p);
            let mut deltas = Vec::new();
            for &a in which {
                let (i, r) = (alive[a], regions[a]);
                let after = rig.render_region_with(i, r, |m| m == k || overlaps(bounds[i][m], r), Some(&tw));
                for (n, &j) in wanted[i].iter().enumerate() {
                    let (bs, _, bi, bu) = before_tallies[a][n];
                    let (s2, _, i2, u2) = region_tallies(&frames[j], &after, r, bg, &opts);
                    if s2 != bs || i2 != bi || u2 != bu {
                        deltas.push((j, s2 - bs, i2 - bi, u2 - bu));
                    }
                }
            }
            let (mut res_c, mut iou_c) = (res_sum, iou_sum);
            for &(j, ds, di, du) in &deltas {
                res_c += frame_res(sse[j] + ds, compared[j]) - frame_res(sse[j], compared[j]);
                iou_c += frame_iou(inter[j] + di, union[j] + du) - frame_iou(inter[j], union[j]);
            }
            let gain = (res_sum.max(1e-9) / res_c.max(1e-9)).ln() + (iou_c.max(1e-9) / iou_sum.max(1e-9)).ln();
            (gain, deltas)
        };
        let sample: Vec<usize> = if alive.len() <= REFINE_SAMPLE_FRAMES {
            (0..alive.len()).collect()
        } else {
            (0..REFINE_SAMPLE_FRAMES).map(|n| n * (alive.len() - 1) / (REFINE_SAMPLE_FRAMES - 1)).collect()
        };
        let mut p = [0f32; 5];
        let mut best = 0f64;
        for step in REFINE_STEPS_PX {
            loop {
                let mut improved = false;
                for &q in &params {
                    for dir in [1.0f32, -1.0] {
                        let mut t = p;
                        t[q] = ((t[q] + dir * step) * 10.0).round() / 10.0;
                        if t[q].abs() > REFINE_MAX_PX {
                            continue;
                        }
                        let (g, _) = score(&t, &sample);
                        if g > best + 1e-12 {
                            p = t;
                            best = g;
                            improved = true;
                            break;
                        }
                    }
                }
                if !improved {
                    break;
                }
            }
        }
        if p == [0.0; 5] {
            continue;
        }
        let all: Vec<usize> = (0..alive.len()).collect();
        let (gain, deltas) = score(&p, &all);
        if gain <= 0.0 {
            continue;
        }
        let tw = make(&p);
        apply_tweak(&mut current, name, &tw, stroke_width);
        rig = Rig::new(&current)?;
        for (i, row) in bounds.iter_mut().enumerate() {
            row[k] = rig.bounds(i, k);
        }
        for (j, ds, di, du) in deltas {
            sse[j] += ds;
            inter[j] += di;
            union[j] += du;
        }
        res_sum = (0..frames.len()).map(|j| frame_res(sse[j], compared[j])).sum();
        iou_sum = (0..frames.len()).map(|j| frame_iou(inter[j], union[j])).sum();
        refined.push(name.clone());
    }
    if refined.is_empty() {
        return Ok(unchanged(doc, fid));
    }
    let rfid = measure(frames, &current)?;
    if reconstructs_better(&fid, &rfid) {
        return Ok(unchanged(doc, fid));
    }
    Ok(Refined {
        document: current,
        fidelity: rfid,
        refined,
    })
}

/// Write an accepted [`vac_compiler::render::Tweak`] into the document:
/// the new size into the `let`, the offset into every `position()`
/// keyframe (or, for a shape that never moves, into its declared place),
/// and the stroke width into its assignment.
fn apply_tweak(doc: &mut Document, name: &str, tw: &vac_compiler::render::Tweak, stroke_width: f32) {
    let round = |v: f32| (v * 10.0).round() / 10.0;
    let stmts = &mut doc.scenes[0].statements;
    let keyframed = stmts.iter().any(|st| {
        matches!(st, Statement::Animate(a) if a.target == name
            && a.keyframes.iter().flat_map(|k| &k.transforms).any(|t| matches!(t, Transform::Position(..))))
    });
    let (dx, dy) = tw.offset;
    for st in stmts.iter_mut() {
        match st {
            Statement::Let { name: n, shape } if n == name => {
                if let Some(b) = &tw.base {
                    *shape = b.clone();
                }
                if !keyframed && (dx != 0.0 || dy != 0.0) {
                    match shape {
                        Shape::Rect { x, y, .. } | Shape::Image { x, y, .. } => {
                            *x = round(*x + dx);
                            *y = round(*y + dy);
                        }
                        Shape::Ellipse { cx, cy, .. } => {
                            *cx = round(*cx + dx);
                            *cy = round(*cy + dy);
                        }
                        Shape::Path(segs) => {
                            let mv = |p: &mut Point| {
                                p.x = round(p.x + dx);
                                p.y = round(p.y + dy);
                            };
                            for seg in segs.iter_mut() {
                                match seg {
                                    PathSeg::MoveTo(p) | PathSeg::LineTo(p) => mv(p),
                                    PathSeg::CurveTo { c1, c2, p } => {
                                        mv(c1);
                                        mv(c2);
                                        mv(p);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Statement::Animate(a) if a.target == name && keyframed => {
                for kf in &mut a.keyframes {
                    for t in &mut kf.transforms {
                        if let Transform::Position(x, y) = t {
                            *x = round(*x + dx);
                            *y = round(*y + dy);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(w) = tw.stroke_width.filter(|w| (*w - stroke_width).abs() > 1e-6) {
        let w = round(w);
        let mut set = false;
        for st in stmts.iter_mut() {
            if let Statement::Assign { target, property: Property::StrokeWidth, value } = st {
                if target == name {
                    *value = Value::Number(w);
                    set = true;
                }
            }
        }
        if !set {
            let at = stmts
                .iter()
                .rposition(|st| matches!(st, Statement::Assign { target, .. } if target == name))
                .or_else(|| stmts.iter().position(|st| matches!(st, Statement::Let { name: n, .. } if n == name)))
                .expect("the shape is declared");
            stmts.insert(
                at + 1,
                Statement::Assign {
                    target: name.to_string(),
                    property: Property::StrokeWidth,
                    value: Value::Number(w),
                },
            );
        }
    }
}

/// What the motion pass (v1.11) did to a document.
#[derive(Debug, Clone)]
pub struct Moved {
    pub document: Document,
    pub fidelity: Fidelity,
    /// The shapes whose motion was re-keyframed.
    pub moved: Vec<String>,
}

/// Motion, measured from the pixels (v1.11).
///
/// The tracker keyframes a shape from its blob's centroid and, for a
/// turning shape, its second moments — estimates that drift whenever the
/// blob is lit unevenly, crossed, or merged for a frame. Here each moving
/// shape's own pixels, at the frame its vector shape matches best, are
/// registered against every other frame it is on screen: the rigid motion
/// (a turn and a translation, the turn only for elongated or already
/// turning shapes) that best maps them there, by a small grid about the
/// shape's keyframed placement and its registered neighbours' heading,
/// then a pattern search to a fiftieth of a pixel, under the camera's
/// robust error. The registered track is simplified into keyframes again,
/// and two candidates are scored like the audit — the registered motion at
/// the shape's own keyframe times (no new bytes), and at the frames the
/// simplification keeps. One is kept only if the clip reconstructs better
/// and the gain pays for any keyframes it adds, at the price per byte the
/// sprite pass pays.
pub fn motion(original: &[VideoFrame], doc: Document, quality: f32) -> Result<Moved> {
    let fid = measure(original, &doc)?;
    motion_measured(original, doc, fid, quality)
}

/// [`motion`] on a document whose fidelity is already measured.
fn motion_measured(frames: &[VideoFrame], doc: Document, fid: Fidelity, quality: f32) -> Result<Moved> {
    use vac_compiler::render::{Rig, Tweak};
    let unchanged = |document: Document, fidelity: Fidelity| Moved {
        document,
        fidelity,
        moved: Vec::new(),
    };
    if doc.scenes.len() != 1 || fid.frames.len() != frames.len() {
        return Ok(unchanged(doc, fid));
    }
    let tol = vac_decompiler::Tolerances::from_quality(quality);
    let mut rig = Rig::new(&doc)?;
    let count = rig.frame_count();
    let idx = aligned_indices(frames, count, doc.canvas.fps);
    let mut wanted: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (j, &i) in idx.iter().enumerate() {
        wanted[i].push(j);
    }
    let bg = ink_background(frames);
    let opts = CompareOptions::default();
    let thr = vac_metrics::DEFAULT_INK_THRESHOLD_SQ as i64;
    let compared: Vec<f64> = fid.frames.iter().map(|f| f.compared_pixels as f64).collect();
    let mut sse: Vec<f64> = fid.frames.iter().map(|f| f.residual * 3.0 * f.compared_pixels as f64).collect();
    let mut inter: Vec<i64> = fid.frames.iter().map(|f| (f.source_ink_px - f.missing_px) as i64).collect();
    let mut union: Vec<i64> = fid.frames.iter().map(|f| (f.source_ink_px + f.spurious_px) as i64).collect();
    let frame_res = |sse: f64, c: f64| if c > 0.0 { sse / (3.0 * c) } else { 0.0 };
    let frame_iou = |i: i64, u: i64| if u > 0 { i as f64 / u as f64 } else { 1.0 };
    let mut res_sum: f64 = (0..frames.len()).map(|j| frame_res(sse[j], compared[j])).sum();
    let mut iou_sum: f64 = (0..frames.len()).map(|j| frame_iou(inter[j], union[j])).sum();
    let names: Vec<String> = rig.shapes(0).to_vec();
    let mut bounds: Vec<Vec<Option<[u32; 4]>>> =
        (0..count).map(|i| (0..names.len()).map(|m| rig.bounds(i, m)).collect()).collect();
    let overlaps = |a: Option<[u32; 4]>, b: [u32; 4]| {
        a.is_some_and(|a| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3])
    };
    let (cw, ch) = (doc.canvas.width, doc.canvas.height);
    let mut current = doc.clone();
    let mut moved = Vec::new();
    // Bilinear sample of a source frame at continuous point p.
    let sample = |f: &VideoFrame, p: (f32, f32)| -> Option<[f32; 3]> {
        let (x, y) = (p.0 - 0.5, p.1 - 0.5);
        if x < 0.0 || y < 0.0 || x > (f.width - 1) as f32 || y > (f.height - 1) as f32 {
            return None;
        }
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(f.width as usize - 1), (y0 + 1).min(f.height as usize - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let w = f.width as usize;
        let mut out = [0f32; 3];
        for c in 0..3 {
            let at = |xx: usize, yy: usize| f.pixels[(yy * w + xx) * 4 + c] as f32;
            out[c] = (at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx) * (1.0 - fy) + (at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx) * fy;
        }
        Some(out)
    };
    for (k, name) in names.iter().enumerate() {
        if name == "bg" || name == "plate" {
            continue;
        }
        let Some(Statement::Let { shape: base, .. }) = current.scenes[0]
            .statements
            .iter()
            .find(|st| matches!(st, Statement::Let { name: n, .. } if n == name))
            .cloned()
        else {
            continue;
        };
        if matches!(base, Shape::Image { .. }) {
            continue;
        }
        let Some(anim) = current.scenes[0].statements.iter().find_map(|st| match st {
            Statement::Animate(a) if a.target == *name => Some(a.clone()),
            _ => None,
        }) else {
            continue;
        };
        if !anim.keyframes.iter().flat_map(|kf| &kf.transforms).any(|t| matches!(t, Transform::Position(..))) {
            continue;
        }
        let alive: Vec<usize> = (0..count).filter(|&i| !wanted[i].is_empty() && bounds[i][k].is_some()).collect();
        if alive.len() < 3 || alive.last().unwrap() - alive[0] + 1 != alive.len() {
            continue;
        }
        // 1. The frame where the shape best matches its own pixels.
        let mut best_ref = (0.0f64, alive[0]);
        let mut presence_of: Vec<Vec<bool>> = Vec::new();
        let mut cover_of: Vec<Vec<bool>> = Vec::new();
        for &i in &alive {
            let r = bounds[i][k].unwrap();
            let rw = (r[2] - r[0]) as usize;
            let without = rig.render_region(i, r, |m| m != k && overlaps(bounds[i][m], r));
            let alone = rig.render_region(i, r, |m| m == k);
            let src = &frames[wanted[i][0]];
            let (mut it, mut un) = (0usize, 0usize);
            let mut pres = vec![false; rw * (r[3] - r[1]) as usize];
            let mut cov = vec![false; rw * (r[3] - r[1]) as usize];
            for y in r[1]..r[3] {
                for x in r[0]..r[2] {
                    let q = ((y - r[1]) as usize * rw + (x - r[0]) as usize) * 4;
                    let sp = ((y * src.width + x) * 4) as usize;
                    let o = &src.pixels[sp..sp + 4];
                    let d: i64 = (0..3).map(|c| (o[c] as i64 - without[q + c] as i64).pow(2)).sum();
                    let p = d > thr;
                    let c = alone[q + 3] >= 128;
                    pres[q / 4] = p;
                    cov[q / 4] = c;
                    it += (p && c) as usize;
                    un += (p || c) as usize;
                }
            }
            let iou = if un > 0 { it as f64 / un as f64 } else { 0.0 };
            if iou > best_ref.0 {
                best_ref = (iou, i);
            }
            presence_of.push(pres);
            cover_of.push(cov);
        }
        let (ref_iou, iref) = best_ref;
        if ref_iou < 0.3 {
            continue;
        }
        let a_ref = alive.iter().position(|&i| i == iref).unwrap();
        let r = bounds[iref][k].unwrap();
        let rw = (r[2] - r[0]) as usize;
        let src_ref = &frames[wanted[iref][0]];
        // 2. The template: the shape's own pixels at the reference frame.
        // The shape's own pixels: where the source differs from the scene
        // without it *and* the shape itself paints, give or take 2 px —
        // not every pixel some other shape got wrong.
        let rh = (r[3] - r[1]) as usize;
        let near_cover = |x: usize, y: usize| {
            let (x0, x1) = (x.saturating_sub(2), (x + 2).min(rw - 1));
            let (y0, y1) = (y.saturating_sub(2), (y + 2).min(rh - 1));
            (y0..=y1).any(|yy| (x0..=x1).any(|xx| cover_of[a_ref][yy * rw + xx]))
        };
        let mut template: Vec<((f32, f32), [f32; 3])> = Vec::new();
        for y in r[1]..r[3] {
            for x in r[0]..r[2] {
                let (lx, ly) = ((x - r[0]) as usize, (y - r[1]) as usize);
                if presence_of[a_ref][ly * rw + lx] && near_cover(lx, ly) {
                    let sp = ((y * src_ref.width + x) * 4) as usize;
                    let o = &src_ref.pixels[sp..sp + 4];
                    template.push(((x as f32 + 0.5, y as f32 + 0.5), [o[0] as f32, o[1] as f32, o[2] as f32]));
                }
            }
        }
        if template.len() < 12 {
            continue;
        }
        if template.len() > 1500 {
            let step = template.len() as f32 / 1500.0;
            template = (0..1500).map(|n| template[(n as f32 * step) as usize]).collect();
        }
        // Rotation centre: where the shape turns about, at the reference.
        let pl = rig.placement(iref, k);
        let centre_off = match &base {
            Shape::Rect { w, h, .. } => (w * 0.5 * pl.scale.0, h * 0.5 * pl.scale.1),
            _ => (0.0, 0.0),
        };
        let c_ref = (pl.position.0 + centre_off.0, pl.position.1 + centre_off.1);
        let elongated = match &base {
            Shape::Rect { w, h, .. } => w.max(*h) / w.min(*h).max(0.5) >= 1.5,
            Shape::Ellipse { rx, ry, .. } => rx.max(*ry) / rx.min(*ry).max(0.25) >= 1.5,
            _ => {
                let bw = (r[2] - r[0]) as f32;
                let bh = (r[3] - r[1]) as f32;
                bw.max(bh) / bw.min(bh).max(1.0) >= 1.5
            }
        };
        let turns = elongated
            || anim.keyframes.iter().flat_map(|kf| &kf.transforms).any(|t| matches!(t, Transform::Rotate(_)));
        const CEIL: f32 = 3.0 * 40.0 * 40.0;
        let cost = |f: &VideoFrame, ang: f32, m: (f32, f32)| -> f64 {
            let (sn, cs) = ang.sin_cos();
            let mut sum = 0f64;
            let mut n = 0usize;
            for &(q, col) in &template {
                let (dx, dy) = (q.0 - c_ref.0, q.1 - c_ref.1);
                let p = (cs * dx - sn * dy + m.0, sn * dx + cs * dy + m.1);
                if let Some(v) = sample(f, p) {
                    let d: f32 = (0..3).map(|c| (v[c] - col[c]).powi(2)).sum();
                    sum += d.min(CEIL) as f64;
                    n += 1;
                }
            }
            if n * 2 < template.len() {
                f64::INFINITY
            } else {
                sum / n as f64 + CEIL as f64 * (template.len() - n) as f64 / template.len() as f64
            }
        };
        let register = |f: &VideoFrame, guesses: &[(f32, (f32, f32))]| -> (f32, (f32, f32)) {
            let mut best = (f64::INFINITY, guesses[0].0, guesses[0].1);
            // The basin: a small grid about each guess — where the shape's
            // own keyframes put it, and where its registered neighbours
            // say it is heading.
            let angs: Vec<f32> = if turns { vec![-0.03, 0.0, 0.03] } else { vec![0.0] };
            for &(ga, gm) in guesses {
                for &da in &angs {
                    for dy in [-3.0f32, -1.5, 0.0, 1.5, 3.0] {
                        for dx in [-3.0f32, -1.5, 0.0, 1.5, 3.0] {
                            let (a, m) = (ga + da, (gm.0 + dx, gm.1 + dy));
                            let c = cost(f, a, m);
                            if c < best.0 {
                                best = (c, a, m);
                            }
                        }
                    }
                }
            }
            // Then a pattern search down to a fiftieth of a pixel.
            let (mut sa, mut st) = (0.02f32, 1.0f32);
            for _ in 0..300 {
                let (_, a, m) = best;
                let mut trials = vec![(a, (m.0 + st, m.1)), (a, (m.0 - st, m.1)), (a, (m.0, m.1 + st)), (a, (m.0, m.1 - st))];
                if turns {
                    trials.push((a + sa, m));
                    trials.push((a - sa, m));
                }
                let mut improved = false;
                for (ta, tm) in trials {
                    let c = cost(f, ta, tm);
                    if c < best.0 {
                        best = (c, ta, tm);
                        improved = true;
                    }
                }
                if !improved {
                    sa *= 0.5;
                    st *= 0.5;
                    if st < 0.02 && sa < 0.0005 {
                        break;
                    }
                }
            }
            (best.1, best.2)
        };
        // 3. Register outward from the reference frame.
        let keyed = |i: usize| -> (f32, (f32, f32)) {
            let p = rig.placement(i, k);
            let off = match &base {
                Shape::Rect { w, h, .. } => (w * 0.5 * p.scale.0, h * 0.5 * p.scale.1),
                _ => (0.0, 0.0),
            };
            ((p.rotation - pl.rotation).to_radians(), (p.position.0 + off.0, p.position.1 + off.1))
        };
        let mut reg: Vec<(f32, (f32, f32))> = vec![(0.0, c_ref); alive.len()];
        for dir in [1i64, -1] {
            let mut prev = (0.0f32, c_ref);
            let mut prev2: Option<(f32, (f32, f32))> = None;
            let mut a = a_ref as i64 + dir;
            while a >= 0 && (a as usize) < alive.len() {
                let heading = match prev2 {
                    Some(p2) => (2.0 * prev.0 - p2.0, (2.0 * prev.1 .0 - p2.1 .0, 2.0 * prev.1 .1 - p2.1 .1)),
                    None => prev,
                };
                let got = register(&frames[wanted[alive[a as usize]][0]], &[keyed(alive[a as usize]), heading]);
                reg[a as usize] = got;
                prev2 = Some(prev);
                prev = got;
                a += dir;
            }
        }
        // 4. Keyframes: RDP on the transform, measured at the template's
        // corners.
        let (tx0, ty0, tx1, ty1) = template.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |b, (q, _)| {
            (b.0.min(q.0), b.1.min(q.1), b.2.max(q.0), b.3.max(q.1))
        });
        let corners = [(tx0, ty0), (tx1, ty0), (tx0, ty1), (tx1, ty1)];
        let place = |t: (f32, (f32, f32)), q: (f32, f32)| -> (f32, f32) {
            let (sn, cs) = t.0.sin_cos();
            let (dx, dy) = (q.0 - c_ref.0, q.1 - c_ref.1);
            (cs * dx - sn * dy + t.1 .0, sn * dx + cs * dy + t.1 .1)
        };
        let times: Vec<u32> = alive.iter().map(|&i| rig.time_ms(i)).collect();
        let err = |a: usize, b: usize, t: usize| -> f32 {
            let span = (times[b] - times[a]).max(1) as f32;
            let u = (times[t] - times[a]) as f32 / span;
            let lerp = |x: f32, y: f32| x + (y - x) * u;
            let v = (lerp(reg[a].0, reg[b].0), (lerp(reg[a].1 .0, reg[b].1 .0), lerp(reg[a].1 .1, reg[b].1 .1)));
            corners
                .iter()
                .map(|&q| {
                    let (p1, p2) = (place(v, q), place(reg[t], q));
                    ((p1.0 - p2.0).powi(2) + (p1.1 - p2.1).powi(2)).sqrt()
                })
                .fold(0.0, f32::max)
        };
        let n = alive.len();
        let mut keep = vec![false; n];
        keep[0] = true;
        keep[n - 1] = true;
        let mut stack = vec![(0usize, n - 1)];
        let ktol = tol.keyframe_px * 0.5;
        while let Some((a, b)) = stack.pop() {
            if b <= a + 1 {
                continue;
            }
            let (worst, at) = (a + 1..b).map(|t| (err(a, b, t), t)).fold((0.0f32, a), |m, x| if x.0 > m.0 { x } else { m });
            if worst > ktol {
                keep[at] = true;
                stack.push((a, at));
                stack.push((at, b));
            }
        }
        // 5. Candidate animations: the registered motion sampled at the
        // shape's own keyframe times (no new bytes), and at the frames the
        // simplification kept. Every other channel stays as it was.
        let theta_ref = pl.rotation;
        let round = |v: f32| (v * 10.0).round() / 10.0;
        let at_time = |t: u32| -> (f32, (f32, f32)) {
            let t = t.clamp(times[0], times[n - 1]);
            let b = times.iter().position(|&x| x >= t).unwrap_or(n - 1);
            if b == 0 || times[b] == t {
                return reg[b];
            }
            let a = b - 1;
            let u = (t - times[a]) as f32 / (times[b] - times[a]).max(1) as f32;
            let l = |x: f32, y: f32| x + (y - x) * u;
            (l(reg[a].0, reg[b].0), (l(reg[a].1 .0, reg[b].1 .0), l(reg[a].1 .1, reg[b].1 .1)))
        };
        let build = |at: &[u32]| -> vac_format::Animation {
            let mut keyframes: Vec<vac_format::Keyframe> = anim
                .keyframes
                .iter()
                .filter_map(|kf| {
                    let rest: Vec<Transform> = kf
                        .transforms
                        .iter()
                        .filter(|t| !matches!(t, Transform::Position(..)) && !(turns && matches!(t, Transform::Rotate(_))))
                        .cloned()
                        .collect();
                    (!rest.is_empty()).then(|| vac_format::Keyframe { time_ms: kf.time_ms, transforms: rest })
                })
                .collect();
            for &t in at {
                let (ang, m) = at_time(t);
                let i = alive[times.iter().position(|&x| x >= t.min(times[n - 1])).unwrap_or(n - 1)];
                let scale = rig.placement(i, k).scale;
                let off = match &base {
                    Shape::Rect { w, h, .. } => (w * 0.5 * scale.0, h * 0.5 * scale.1),
                    _ => (0.0, 0.0),
                };
                let mut tr = vec![Transform::Position(round(m.0 - off.0), round(m.1 - off.1))];
                if turns {
                    tr.push(Transform::Rotate(round(theta_ref + ang.to_degrees())));
                }
                match keyframes.iter_mut().find(|kf| kf.time_ms == t) {
                    Some(kf) => kf.transforms.extend(tr),
                    None => keyframes.push(vac_format::Keyframe { time_ms: t, transforms: tr }),
                }
            }
            keyframes.sort_by_key(|kf| kf.time_ms);
            vac_format::Animation { target: name.clone(), keyframes, easing: anim.easing }
        };
        let own_times: Vec<u32> = anim
            .keyframes
            .iter()
            .filter(|kf| kf.transforms.iter().any(|t| matches!(t, Transform::Position(..))))
            .map(|kf| kf.time_ms)
            .collect();
        let kept_times: Vec<u32> = (0..n).filter(|&a| keep[a]).map(|a| times[a]).collect();
        let anim_bytes = |a: &vac_format::Animation| -> i64 {
            let mut d = current.clone();
            d.scenes[0].statements = vec![Statement::Animate(a.clone())];
            vac_format::write(&d).len() as i64
        };
        let old_bytes = anim_bytes(&anim);
        let m = 2f64.powf((50.0 - quality.clamp(0.0, 100.0) as f64) / 25.0);
        let price = SPRITE_MIN_GAIN_PER_BYTE * m * m;
        // 6. Score each on every frame either version is visible in; keep
        // the one that buys the most, if it pays for its bytes.
        let mut chosen: Option<(f64, vac_format::Animation, Vec<(usize, f64, i64, i64)>)> = None;
        for cand in [build(&own_times), build(&kept_times)] {
        let tw = Tweak { shape: k, base: None, offset: (0.0, 0.0), stroke_width: None, animation: Some(cand.clone()) };
        let mut deltas: Vec<(usize, f64, i64, i64)> = Vec::new();
        for i in 0..count {
            if wanted[i].is_empty() {
                continue;
            }
            let rr = match (bounds[i][k], rig.bounds_with(i, k, Some(&tw))) {
                (Some(a), Some(b)) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => continue,
            };
            let rr = [rr[0], rr[1], rr[2].min(cw), rr[3].min(ch)];
            let before = rig.render_region(i, rr, |m| overlaps(bounds[i][m], rr));
            let after = rig.render_region_with(i, rr, |m| m == k || overlaps(bounds[i][m], rr), Some(&tw));
            for &j in &wanted[i] {
                let (s0, _, i0, u0) = region_tallies(&frames[j], &before, rr, bg, &opts);
                let (s1, _, i1, u1) = region_tallies(&frames[j], &after, rr, bg, &opts);
                if s0 != s1 || i0 != i1 || u0 != u1 {
                    deltas.push((j, s1 - s0, i1 - i0, u1 - u0));
                }
            }
        }
        let (mut res_c, mut iou_c) = (res_sum, iou_sum);
        for &(j, ds, di, du) in &deltas {
            res_c += frame_res(sse[j] + ds, compared[j]) - frame_res(sse[j], compared[j]);
            iou_c += frame_iou(inter[j] + di, union[j] + du) - frame_iou(inter[j], union[j]);
        }
        let gain = (res_sum.max(1e-9) / res_c.max(1e-9)).ln() + (iou_c.max(1e-9) / iou_sum.max(1e-9)).ln();
        let added = (anim_bytes(&cand) - old_bytes) as f64;
        // Below the default quality the dial only ever takes keyframes
        // away; the motion pass may correct them there, not add to them
        // (on `spin.vac` it made quality 0 larger than quality 25).
        if added > 0.0 && quality < vac_decompiler::track::DEFAULT_QUALITY {
            continue;
        }
        let net = gain - price * added.max(0.0);
        if gain > 0.0 && net > 0.0 && chosen.as_ref().is_none_or(|c| net > c.0) {
            chosen = Some((net, cand, deltas));
        }
        }
        let Some((_, new_anim, deltas)) = chosen else {
            continue;
        };
        for st in &mut current.scenes[0].statements {
            if let Statement::Animate(a) = st {
                if a.target == *name {
                    *a = new_anim.clone();
                }
            }
        }
        rig = Rig::new(&current)?;
        for (i, row) in bounds.iter_mut().enumerate() {
            row[k] = rig.bounds(i, k);
        }
        for (j, ds, di, du) in deltas {
            sse[j] += ds;
            inter[j] += di;
            union[j] += du;
        }
        res_sum = (0..frames.len()).map(|j| frame_res(sse[j], compared[j])).sum();
        iou_sum = (0..frames.len()).map(|j| frame_iou(inter[j], union[j])).sum();
        moved.push(name.clone());
    }
    if moved.is_empty() {
        return Ok(unchanged(doc, fid));
    }
    let mfid = measure(frames, &current)?;
    if reconstructs_better(&fid, &mfid) {
        return Ok(unchanged(doc, fid));
    }
    Ok(Moved {
        document: current,
        fidelity: mfid,
        moved,
    })
}

/// What the sprite pass (v1.8) did to a document.
#[derive(Debug, Clone)]
pub struct Sprited {
    pub document: Document,
    pub fidelity: Fidelity,
    /// The shapes now drawn as sprites of their own pixels.
    pub sprites: Vec<String>,
}

/// A shape vectors cannot describe keeps its own pixels (v1.8).
///
/// A flat fill is the right description of a flat shape and the wrong one
/// of a shaded ball, a chrome sphere or a hand-drawn bob. For each shape
/// that moves only by translation (or not at all), the source's own pixels
/// are captured in a window that follows it, as a per-pixel median over
/// every frame it is on screen: transient occluders and noise drop out,
/// and a pixel belongs to the sprite when the source differs there from
/// the scene without the shape in most of those frames. The sprite — a PNG
/// `image()` with that alpha — replaces the shape, keeping its keyframes,
/// only if it reconstructs the shape's own footprint at least **twice as
/// well** (the v1.6 raster plate's bar, applied locally) and the clip as a
/// whole better. The motion stays code; only the appearance is pixels.
///
/// Like the audit, each candidate is scored from small renders of the
/// footprint ([`vac_compiler::render::Rig`]) against the full
/// measurement's per-frame tallies, and the result is re-measured in full
/// and kept only if it measures at least as well as what went in.
pub fn sprites(original: &[VideoFrame], doc: Document, quality: f32) -> Result<Sprited> {
    let fid = measure(original, &doc)?;
    sprites_measured(original, doc, fid, quality)
}

/// A sprite must reconstruct the shape's footprint this much better than
/// the vector shape did — `ln 2`, twice as good, the raster plate's bar.
const SPRITE_MIN_LOCAL_GAIN: f64 = std::f64::consts::LN_2;
/// Only shapes whose interior the flat fill reconstructs below this PSNR
/// are tried — the raster plate's 30 dB, applied to appearance: rasters are
/// for where vectors measurably fail, not for softening the edges of a flat
/// rectangle keyframed a pixel off (judged on the whole footprint,
/// `motion_graphics` traded a drawbox square for a PNG).
const SPRITE_ONLY_BELOW_PSNR_DB: f64 = RASTER_ONLY_BELOW_PSNR_DB;
/// What a sprite must buy per byte it adds, in the selection rule's
/// currency, at quality 50 — scaled like every other tolerance by the
/// dial (×4 per 25 points down, ÷4 per 25 up). `1e-5` is ~0.04 dB of clip
/// PSNR per KB. Without it the Mandelbrot zoom took 162 KB of PNG for
/// 0.7 dB; the bouncing ball's sprite buys 0.74 for 1.1 KB, 60× the bar.
const SPRITE_MIN_GAIN_PER_BYTE: f64 = 1e-5;

/// [`sprites`] on a document whose fidelity is already measured.
fn sprites_measured(frames: &[VideoFrame], doc: Document, fid: Fidelity, quality: f32) -> Result<Sprited> {
    let m = 2f64.powf((50.0 - quality.clamp(0.0, 100.0) as f64) / 25.0);
    let min_gain_per_byte = SPRITE_MIN_GAIN_PER_BYTE * m * m;
    let unchanged = |document: Document, fidelity: Fidelity| Sprited {
        document,
        fidelity,
        sprites: Vec::new(),
    };
    if doc.scenes.len() != 1 || fid.frames.len() != frames.len() {
        return Ok(unchanged(doc, fid));
    }
    let mut rig = vac_compiler::render::Rig::new(&doc)?;
    let count = rig.frame_count();
    let idx = aligned_indices(frames, count, doc.canvas.fps);
    let mut wanted: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (j, &i) in idx.iter().enumerate() {
        wanted[i].push(j);
    }
    let bg = ink_background(frames);
    let opts = CompareOptions::default();
    let compared: Vec<f64> = fid.frames.iter().map(|f| f.compared_pixels as f64).collect();
    let mut sse: Vec<f64> = fid.frames.iter().map(|f| f.residual * 3.0 * f.compared_pixels as f64).collect();
    let mut inter: Vec<i64> = fid.frames.iter().map(|f| (f.source_ink_px - f.missing_px) as i64).collect();
    let mut union: Vec<i64> = fid.frames.iter().map(|f| (f.source_ink_px + f.spurious_px) as i64).collect();
    let frame_res = |sse: f64, c: f64| if c > 0.0 { sse / (3.0 * c) } else { 0.0 };
    let frame_iou = |i: i64, u: i64| if u > 0 { i as f64 / u as f64 } else { 1.0 };
    let mut res_sum: f64 = (0..frames.len()).map(|j| frame_res(sse[j], compared[j])).sum();
    let mut iou_sum: f64 = (0..frames.len()).map(|j| frame_iou(inter[j], union[j])).sum();

    let mut current = doc.clone();
    let mut sprited: Vec<String> = Vec::new();
    let names: Vec<String> = rig.shapes(0).to_vec();
    // Where every shape paints in every frame, so a region render draws
    // only the shapes that reach it. A sprite takes its shape's place in
    // the draw order, so only its own row changes when one is accepted.
    let mut bounds: Vec<Vec<Option<[u32; 4]>>> =
        (0..count).map(|i| (0..names.len()).map(|m| rig.bounds(i, m)).collect()).collect();
    let overlaps = |a: Option<[u32; 4]>, b: [u32; 4]| {
        a.is_some_and(|a| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3])
    };
    for (k, name) in names.iter().enumerate() {
        if name == "bg" || name == "plate" {
            continue;
        }
        let Some(Statement::Let { shape, .. }) = current.scenes[0]
            .statements
            .iter()
            .find(|st| matches!(st, Statement::Let { name: n, .. } if n == name))
        else {
            continue;
        };
        if matches!(shape, Shape::Image { .. }) {
            continue;
        }
        let anim = current.scenes[0].statements.iter().find_map(|st| match st {
            Statement::Animate(a) if a.target == *name => Some(a.clone()),
            _ => None,
        });
        // Translation (and fades) only: a sprite captured in a window that
        // follows the shape cannot follow it turning, growing or changing
        // colour.
        if anim.as_ref().is_some_and(|a| {
            a.keyframes
                .iter()
                .flat_map(|kf| &kf.transforms)
                .any(|t| matches!(t, Transform::Scale(..) | Transform::Rotate(_) | Transform::Fill(_)))
        }) {
            continue;
        }
        let alive: Vec<usize> = (0..count)
            .filter(|&i| !wanted[i].is_empty() && bounds[i][k].is_some())
            .collect();
        if alive.is_empty() {
            continue;
        }
        // Does the flat fill describe the shape's *appearance*? Judged on
        // its interior — fully covered by the shape, two pixels in from
        // every edge, and visible — where a position or size slightly off
        // (a motion keyframed a pixel wide) cannot reach. A shape too thin
        // to have an interior offers no evidence that its fill fails: what
        // goes wrong with a stroke is its geometry (`outlined.vac`'s ring,
        // radius 49.8 for 50, gave up 5.7 dB to a PNG copy of itself), and
        // that is the vector fit's to fix, not a raster's to hide.
        let (mut in_sse, mut in_px) = (0f64, 0f64);
        for &i in &alive {
            let r = bounds[i][k].expect("alive");
            let rw = (r[2] - r[0]) as i64;
            let rh = (r[3] - r[1]) as i64;
            let admit = |m: usize| overlaps(bounds[i][m], r);
            let with = rig.render_region(i, r, admit);
            let without = rig.render_region(i, r, |m| m != k && admit(m));
            let alone = rig.render_region(i, r, |m| m == k);
            let full = |x: i64, y: i64| {
                x >= 0 && y >= 0 && x < rw && y < rh && alone[((y * rw + x) * 4 + 3) as usize] == 255
            };
            for y in 0..rh {
                for x in 0..rw {
                    let q = ((y * rw + x) * 4) as usize;
                    if with[q..q + 4] == without[q..q + 4] {
                        continue;
                    }
                    if !(-2..=2).all(|dy| (-2..=2).all(|dx| full(x + dx, y + dy))) {
                        continue;
                    }
                    for &j in &wanted[i] {
                        let src = &frames[j];
                        let sp = (((r[1] as i64 + y) * src.width as i64 + r[0] as i64 + x) * 4) as usize;
                        let o = &src.pixels[sp..sp + 4];
                        if o[3] < vac_metrics::MIN_OPAQUE_ALPHA {
                            continue;
                        }
                        in_sse += (0..3).map(|c| (o[c] as f64 - with[q + c] as f64).powi(2)).sum::<f64>();
                        in_px += 1.0;
                    }
                }
            }
        }
        if in_px == 0.0 || vac_metrics::psnr_from_mse(in_sse / (3.0 * in_px)) >= SPRITE_ONLY_BELOW_PSNR_DB {
            continue;
        }
        let Some(sprite) = capture_sprite(frames, &rig, &wanted, &bounds, k, &alive) else {
            continue;
        };
        let cand = with_sprite(&current, name, &sprite, anim.as_ref());
        let crig = vac_compiler::render::Rig::new(&cand)?;
        debug_assert_eq!(crig.shapes(0)[k], *name, "a sprite keeps its shape's place");
        let cbounds: Vec<Option<[u32; 4]>> = (0..count).map(|i| crig.bounds(i, k)).collect();
        // Score both versions over the union of their footprints.
        let mut deltas: Vec<(usize, f64, i64, i64)> = Vec::new();
        let (mut s0, mut s1, mut i0, mut u0, mut i1, mut u1) = (0f64, 0f64, 0i64, 0i64, 0i64, 0i64);
        for i in 0..count {
            if wanted[i].is_empty() {
                continue;
            }
            let r = match (bounds[i][k], cbounds[i]) {
                (Some(a), Some(b)) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => continue,
            };
            let before = rig.render_region(i, r, |m| overlaps(bounds[i][m], r));
            let after = crig.render_region(i, r, |m| {
                if m == k {
                    overlaps(cbounds[i], r)
                } else {
                    overlaps(bounds[i][m], r)
                }
            });
            for &j in &wanted[i] {
                let (a_sse, _, a_i, a_u) = region_tallies(&frames[j], &before, r, bg, &opts);
                let (b_sse, _, b_i, b_u) = region_tallies(&frames[j], &after, r, bg, &opts);
                s0 += a_sse;
                s1 += b_sse;
                i0 += a_i;
                u0 += a_u;
                i1 += b_i;
                u1 += b_u;
                if a_sse != b_sse || a_i != b_i || a_u != b_u {
                    deltas.push((j, b_sse - a_sse, b_i - a_i, b_u - a_u));
                }
            }
        }
        let local_iou = |i: i64, u: i64| if u > 0 { i as f64 / u as f64 } else { 1.0 };
        let local_gain =
            (s0.max(1.0) / s1.max(1.0)).ln() + (local_iou(i1, u1).max(1e-9) / local_iou(i0, u0).max(1e-9)).ln();
        let (mut res_c, mut iou_c) = (res_sum, iou_sum);
        for &(j, ds, di, du) in &deltas {
            res_c += frame_res(sse[j] + ds, compared[j]) - frame_res(sse[j], compared[j]);
            iou_c += frame_iou(inter[j] + di, union[j] + du) - frame_iou(inter[j], union[j]);
        }
        let clip_gain = (res_sum.max(1e-9) / res_c.max(1e-9)).ln() + (iou_c.max(1e-9) / iou_sum.max(1e-9)).ln();
        let added_bytes = (statement_bytes(&cand, name) - statement_bytes(&current, name)).max(1) as f64;
        if local_gain >= SPRITE_MIN_LOCAL_GAIN && clip_gain >= min_gain_per_byte * added_bytes {
            current = cand;
            rig = crig;
            for (i, b) in cbounds.into_iter().enumerate() {
                bounds[i][k] = b;
            }
            res_sum = res_c;
            iou_sum = iou_c;
            for (j, ds, di, du) in deltas {
                sse[j] += ds;
                inter[j] += di;
                union[j] += du;
            }
            sprited.push(name.clone());
        }
    }
    if sprited.is_empty() {
        return Ok(unchanged(doc, fid));
    }
    let sfid = measure(frames, &current)?;
    if reconstructs_better(&fid, &sfid) {
        return Ok(unchanged(doc, fid));
    }
    Ok(Sprited {
        document: current,
        fidelity: sfid,
        sprites: sprited,
    })
}

/// Source bytes of one shape's statements — its `let`, assignments and
/// `animate` block — as written.
fn statement_bytes(doc: &Document, name: &str) -> i64 {
    let mut only = doc.clone();
    only.scenes[0].statements.retain(|st| match st {
        Statement::Let { name: n, .. } => n == name,
        Statement::Assign { target, .. } => target == name,
        Statement::Animate(a) => a.target == name,
    });
    vac_format::write(&only).len() as i64
}

/// Squared error, compared pixels, ink intersection and ink union of a
/// rendered region against one source frame — `compare_frame`'s tallies,
/// over a rectangle.
fn region_tallies(src: &VideoFrame, img: &[u8], r: [u32; 4], bg: Rgb, opts: &CompareOptions) -> (f64, usize, i64, i64) {
    let rw = (r[2] - r[0]) as usize;
    let (mut sse, mut n, mut it, mut un) = (0i64, 0usize, 0i64, 0i64);
    for y in r[1]..r[3] {
        for x in r[0]..r[2] {
            let sp = ((y * src.width + x) * 4) as usize;
            let o = &src.pixels[sp..sp + 4];
            if o[3] < vac_metrics::MIN_OPAQUE_ALPHA {
                continue;
            }
            let q = ((y - r[1]) as usize * rw + (x - r[0]) as usize) * 4;
            let a = &img[q..q + 4];
            sse += (0..3).map(|c| (o[c] as i64 - a[c] as i64).pow(2)).sum::<i64>();
            n += 1;
            let (oi, ai) = (is_ink(o, bg, opts), is_ink(a, bg, opts));
            it += (oi && ai) as i64;
            un += (oi || ai) as i64;
        }
    }
    (sse as f64, n, it, un)
}

/// A captured sprite: its box at the reference frame, the offset of that
/// box from the shape's anchor there, and the PNG data URI.
struct Sprite {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    offset: (f32, f32),
    uri: String,
}

/// Capture shape `k`'s own pixels: a window the size of its largest
/// footprint, following its anchor through every frame it is alive in;
/// per pixel, the median of the source colours where the source differs
/// from the scene without the shape, opaque where it does so in most
/// frames.
fn capture_sprite(
    frames: &[VideoFrame],
    rig: &vac_compiler::render::Rig,
    wanted: &[Vec<usize>],
    bounds: &[Vec<Option<[u32; 4]>>],
    k: usize,
    alive: &[usize],
) -> Option<Sprite> {
    use image::ImageEncoder;
    let area = |b: [u32; 4]| (b[2] - b[0]) as u64 * (b[3] - b[1]) as u64;
    let iref = *alive.iter().max_by_key(|&&i| area(bounds[i][k].expect("alive")))?;
    let b = bounds[iref][k]?;
    let aref = rig.placement(iref, k).position;
    let (x0, y0) = (b[0] as i64, b[1] as i64);
    let (w, h) = ((b[2] - b[0]) as i64, (b[3] - b[1]) as i64);
    let (cw, ch) = (frames[0].width as i64, frames[0].height as i64);
    let thr = vac_metrics::DEFAULT_INK_THRESHOLD_SQ as i64;
    let n = (w * h) as usize;
    let mut samples: Vec<Vec<[u8; 3]>> = vec![Vec::new(); n];
    let mut seen = vec![0u32; n];
    for &i in alive {
        let p = rig.placement(i, k).position;
        let (dx, dy) = ((p.0 - aref.0).round() as i64, (p.1 - aref.1).round() as i64);
        let r = [
            (x0 + dx).clamp(0, cw) as u32,
            (y0 + dy).clamp(0, ch) as u32,
            (x0 + dx + w).clamp(0, cw) as u32,
            (y0 + dy + h).clamp(0, ch) as u32,
        ];
        if r[0] >= r[2] || r[1] >= r[3] {
            continue;
        }
        let without = rig.render_region(i, r, |m| {
            m != k && bounds[i][m].is_some_and(|a| a[0] < r[2] && r[0] < a[2] && a[1] < r[3] && r[1] < a[3])
        });
        let rw = (r[2] - r[0]) as i64;
        for &j in &wanted[i] {
            let src = &frames[j];
            for y in r[1] as i64..r[3] as i64 {
                for x in r[0] as i64..r[2] as i64 {
                    let sp = ((y * cw + x) * 4) as usize;
                    let o = &src.pixels[sp..sp + 4];
                    if o[3] < vac_metrics::MIN_OPAQUE_ALPHA {
                        continue;
                    }
                    let q = ((y - y0 - dy) * w + (x - x0 - dx)) as usize;
                    let wp = (((y - r[1] as i64) * rw + (x - r[0] as i64)) * 4) as usize;
                    let s = &without[wp..wp + 4];
                    let d: i64 = (0..3).map(|c| (o[c] as i64 - s[c] as i64).pow(2)).sum();
                    seen[q] += 1;
                    if d > thr {
                        samples[q].push([o[0], o[1], o[2]]);
                    }
                }
            }
        }
    }
    let mut rgba = vec![0u8; n * 4];
    let mut opaque = 0usize;
    for q in 0..n {
        let v = &samples[q];
        if v.is_empty() || (v.len() as u32) * 2 <= seen[q] {
            continue;
        }
        for c in 0..3 {
            let mut ch: Vec<u8> = v.iter().map(|s| s[c]).collect();
            ch.sort_unstable();
            rgba[q * 4 + c] = ch[ch.len() / 2];
        }
        rgba[q * 4 + 3] = 255;
        opaque += 1;
    }
    if opaque == 0 {
        return None;
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(Sprite {
        x: x0 as f32,
        y: y0 as f32,
        w: w as f32,
        h: h as f32,
        offset: (x0 as f32 - aref.0, y0 as f32 - aref.1),
        uri: format!("data:image/png;base64,{}", vac_format::base64::encode(&png)),
    })
}

/// `doc` with shape `name` drawn as `sprite`: the same place in the draw
/// order, its fill and stroke dropped, its keyframes kept with positions
/// moved from the shape's anchor to the sprite's top-left corner.
fn with_sprite(doc: &Document, name: &str, sprite: &Sprite, anim: Option<&vac_format::Animation>) -> Document {
    let mut out = doc.clone();
    let stmts = &mut out.scenes[0].statements;
    stmts.retain(|st| match st {
        Statement::Assign { target, .. } => target != name,
        Statement::Animate(a) => a.target != name,
        _ => true,
    });
    let pos = stmts
        .iter()
        .position(|st| matches!(st, Statement::Let { name: n, .. } if n == name))
        .expect("the shape is declared");
    stmts[pos] = Statement::Let {
        name: name.to_string(),
        shape: Shape::Image {
            x: sprite.x,
            y: sprite.y,
            w: sprite.w,
            h: sprite.h,
            src: sprite.uri.clone(),
        },
    };
    if let Some(a) = anim {
        let mut a = a.clone();
        let round = |v: f32| (v * 10.0).round() / 10.0;
        for kf in &mut a.keyframes {
            for t in &mut kf.transforms {
                if let Transform::Position(px, py) = t {
                    *px = round(*px + sprite.offset.0);
                    *py = round(*py + sprite.offset.1);
                }
            }
        }
        stmts.insert(pos + 1, Statement::Animate(a));
    }
    out
}

/// Close the gaps the audit leaves in `shape_<n>`, `static_<n>` and
/// `ground_<n>`, keeping each family in declaration order.
fn renumber(doc: &mut Document) {
    for scene in &mut doc.scenes {
        let mut map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut next: std::collections::HashMap<&'static str, usize> = std::collections::HashMap::new();
        for st in &scene.statements {
            if let Statement::Let { name, .. } = st {
                for family in ["shape_", "static_", "ground_"] {
                    if name.strip_prefix(family).is_some_and(|n| n.parse::<usize>().is_ok()) {
                        let n = next.entry(family).or_insert(0);
                        map.insert(name.clone(), format!("{family}{n}"));
                        *n += 1;
                    }
                }
            }
        }
        let rename = |s: &mut String| {
            if let Some(n) = map.get(s.as_str()) {
                *s = n.clone();
            }
        };
        for st in &mut scene.statements {
            match st {
                Statement::Let { name, .. } => rename(name),
                Statement::Assign { target, .. } => rename(target),
                Statement::Animate(a) => rename(&mut a.target),
            }
        }
    }
}

/// How much better a raster plate must reconstruct than the best vector
/// candidate to be kept (v1.6), in the selection rule's own currency —
/// `ln(residual ratio) + ln(IoU ratio)`. `ln 2`: twice as good, which with
/// IoU unchanged is a 3 dB PSNR gain. Measured as PSNR alone, the bar threw
/// out `real_pendulum`'s plate, which lifts its ink IoU 0.435 → 0.647 and
/// its PSNR 2.4 dB.
const RASTER_MIN_GAIN: f64 = std::f64::consts::LN_2;
/// A raster plate is only considered when the best vector reconstruction
/// is below this PSNR — when vectors measurably fail to describe the scene.
const RASTER_ONLY_BELOW_PSNR_DB: f64 = 30.0;

/// The outcome of rate control: the smallest source found that meets a
/// fidelity target, or the most faithful one if none does.
#[derive(Debug, Clone)]
pub struct RateControlled {
    pub document: Document,
    pub fidelity: Fidelity,
    /// The quality the search settled on.
    pub quality: f32,
    /// Size of the emitted source, bytes.
    pub vac_bytes: usize,
    /// Whether `fidelity.psnr_db` reaches the target.
    pub met: bool,
    /// Every quality tried, as `(quality, bytes, psnr_db)` — the slice of
    /// the rate–distortion curve the search actually measured.
    pub probes: Vec<(f32, usize, f64)>,
}

/// Rate control (v1.0): the smallest `.vac` whose reconstruction reaches
/// `target_psnr_db`.
///
/// The quality dial is probed at 0, 25, 50, 75 and 100, then refined by
/// bisection between the cheapest probe that meets the target and its
/// next-lower neighbour. Among all probes that meet the target the one
/// with the fewest source bytes wins — measured, not assumed, because the
/// curve is not guaranteed monotone (a different segmentation can win the
/// measured search at a different quality). If nothing meets the target,
/// the most faithful probe is returned with `met: false`.
pub fn decompile_to_target(
    frames: Vec<VideoFrame>,
    opts: &DecompileOptions,
    target_psnr_db: f64,
) -> Result<RateControlled> {
    let mut probes: Vec<(f32, Document, Fidelity, usize)> = Vec::new();
    let probe = |q: f32, probes: &mut Vec<(f32, Document, Fidelity, usize)>| -> Result<()> {
        if probes.iter().any(|p| (p.0 - q).abs() < 0.01) {
            return Ok(());
        }
        let o = DecompileOptions { quality: q, ..*opts };
        let chosen = decompile_chosen(&frames, &o)?;
        let fid = match chosen.fidelity {
            Some(f) => f,
            None => measure(&frames, &chosen.document)?,
        };
        let bytes = vac_format::write(&chosen.document).len();
        probes.push((q, chosen.document, fid, bytes));
        Ok(())
    };
    for q in [0.0, 25.0, 50.0, 75.0, 100.0] {
        probe(q, &mut probes)?;
    }
    let meets = |p: &(f32, Document, Fidelity, usize)| p.2.psnr_db >= target_psnr_db;
    // Refine: bisect between the cheapest passing quality and the highest
    // failing quality below it.
    for _ in 0..3 {
        let Some(pass_q) = probes
            .iter()
            .filter(|p| meets(p))
            .min_by(|a, b| a.3.cmp(&b.3).then(a.0.total_cmp(&b.0)))
            .map(|p| p.0)
        else {
            break;
        };
        let Some(fail_q) = probes
            .iter()
            .filter(|p| !meets(p) && p.0 < pass_q)
            .map(|p| p.0)
            .max_by(|a, b| a.total_cmp(b))
        else {
            break;
        };
        if pass_q - fail_q < 3.0 {
            break;
        }
        probe((pass_q + fail_q) * 0.5, &mut probes)?;
    }
    let summary: Vec<(f32, usize, f64)> = {
        let mut v: Vec<_> = probes.iter().map(|p| (p.0, p.3, p.2.psnr_db)).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v
    };
    let best = match probes
        .iter()
        .enumerate()
        .filter(|(_, p)| meets(p))
        .min_by(|a, b| a.1 .3.cmp(&b.1 .3).then(a.1 .0.total_cmp(&b.1 .0)))
    {
        Some((i, _)) => i,
        None => probes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1 .2.psnr_db.total_cmp(&b.1 .2.psnr_db))
            .map(|(i, _)| i)
            .expect("probed at least once"),
    };
    let (quality, document, fidelity, vac_bytes) = probes.swap_remove(best);
    Ok(RateControlled {
        met: fidelity.psnr_db >= target_psnr_db,
        document,
        fidelity,
        quality,
        vac_bytes,
        probes: summary,
    })
}

/// How many measured-search candidates may decompile at once.
const MAX_CONCURRENT_CANDIDATES: usize = 6;

/// How much better `b` reconstructs than `a`: `ln(residual_a / residual_b)
/// + ln(iou_b / iou_a)` — positive when better. The currency
/// [`reconstructs_better`] weighs trade-offs in.
pub fn improvement(b: &Fidelity, a: &Fidelity) -> f64 {
    let (ra, rb) = (a.residual.max(1e-9), b.residual.max(1e-9));
    let (ia, ib) = (a.ink_iou.max(1e-9), b.ink_iou.max(1e-9));
    (ra / rb).ln() + (ib / ia).ln()
}

/// Whether reconstruction `b` beats `a`.
///
/// A candidate at least as good on both residual and ink IoU (and
/// strictly better on one) wins outright. When they trade off, the two
/// relative changes are weighed equally — `ln(residual_a / residual_b) +
/// ln(iou_b / iou_a)` — so a 10% loss of ink overlap is never bought with
/// a 1% gain in colour error, or the reverse. Ties keep `a`.
pub fn reconstructs_better(b: &Fidelity, a: &Fidelity) -> bool {
    let (ra, rb) = (a.residual.max(1e-9), b.residual.max(1e-9));
    let (ia, ib) = (a.ink_iou.max(1e-9), b.ink_iou.max(1e-9));
    if rb <= ra && ib >= ia {
        return rb < ra || ib > ia;
    }
    if ra <= rb && ia >= ib {
        return false;
    }
    improvement(b, a) > 0.0
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
    if original.is_empty() {
        return Err(MetricError::Empty.into());
    }
    // The ink reference is measured from the source frames themselves,
    // by `vac-metrics`, and deliberately *not* by calling the
    // decompiler's own `estimate_background`. If the metric shared the
    // pipeline's background rule, tuning that rule would silently move
    // every committed baseline and "the decompiler improved" would be
    // indistinguishable from "the ruler moved".
    let bg = ink_background(original);
    measure_against(original, doc, bg, opts)
}

/// [`measure_with`] against a given ink reference — the whole clip's,
/// when the clip is measured a segment at a time (v1.16).
fn measure_against(original: &[VideoFrame], doc: &Document, bg: Rgb, opts: &CompareOptions) -> Result<Fidelity> {
    let count = vac_compiler::render::frame_count(doc);
    let idx = aligned_indices(original, count, doc.canvas.fps);
    let src: Vec<vac_metrics::FrameRef> = original
        .iter()
        .map(|f| vac_metrics::FrameRef::new(f.width, f.height, &f.pixels))
        .collect();
    // Which source frames each rendered frame stands for. Rendering
    // streams frame by frame and each rendered frame is scored against
    // its source frames and dropped — the reconstruction is never held
    // whole (on the 220-frame double pendulum that was ~400 MB per
    // candidate of the measured search).
    let mut wanted: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (j, &i) in idx.iter().enumerate() {
        wanted[i].push(j);
    }
    let mut scores: Vec<Option<vac_metrics::FrameScore>> = vec![None; original.len()];
    let mut mismatch: Option<MetricError> = None;
    vac_compiler::render::render_document_each(doc, |i, r| {
        for &j in &wanted[i] {
            let o = &src[j];
            if o.width != r.width || o.height != r.height {
                mismatch.get_or_insert(MetricError::DimensionMismatch {
                    frame: j,
                    original: (o.width, o.height),
                    reconstructed: (r.width, r.height),
                });
                continue;
            }
            let rf = vac_metrics::FrameRef::new(r.width, r.height, &r.rgba);
            scores[j] = Some(vac_metrics::compare_frame(o, &rf, bg, opts));
        }
        Ok(())
    })?;
    if let Some(e) = mismatch {
        return Err(e.into());
    }
    let frames: Vec<vac_metrics::FrameScore> =
        scores.into_iter().map(|s| s.expect("every source frame is aligned")).collect();
    Ok(vac_metrics::aggregate(frames)?)
}

fn aligned_indices(original: &[VideoFrame], rendered: usize, fps: u32) -> Vec<usize> {
    let delays: Vec<u32> = original.iter().map(|f| f.delay_ms).collect();
    let times = vac_metrics::timeline_from_delays(&delays);
    vac_metrics::align(&times, rendered, fps)
}

/// The reconstruction, re-rendered and aligned to the source's timeline:
/// element `i` is what the `.vac` shows while source frame `i` is on
/// screen — exactly the frame `measure` scores it against.
pub fn render_aligned(original: &[VideoFrame], doc: &Document) -> Result<Vec<RenderedFrame>> {
    let rendered = render(doc)?;
    let idx = aligned_indices(original, rendered.len(), doc.canvas.fps);
    Ok(idx
        .into_iter()
        .map(|i| {
            let r = &rendered[i];
            RenderedFrame {
                width: r.width,
                height: r.height,
                rgba: r.rgba.clone(),
            }
        })
        .collect())
}

/// The background the fidelity metric measures ink against for this
/// clip (the modal colour of the source, computed independently of the
/// decompiler's own background rule).
pub fn ink_background(original: &[VideoFrame]) -> Rgb {
    let src: Vec<vac_metrics::FrameRef> = original
        .iter()
        .map(|f| vac_metrics::FrameRef::new(f.width, f.height, &f.pixels))
        .collect();
    vac_metrics::modal_background(&src)
}

/// Rate *and* distortion for one clip — everything `vac verify` reports.
#[derive(Debug, Clone)]
pub struct Verification {
    pub fidelity: Fidelity,
    pub document: Document,
    /// The segmentation the decompile used — under `Auto`, the one
    /// measurement picked.
    pub segmentation: Segmentation,
    /// The background rule the decompile used.
    pub background: Background,
    /// How the static scene was written — under `Auto`, what measurement
    /// picked.
    pub plate: Plate,
    /// Frames decoded from the source.
    pub source_frames: usize,
    /// Size of the emitted `.vac` source, in bytes. The "rate" axis.
    pub vac_bytes: usize,
    pub shapes: usize,
    pub animate_blocks: usize,
    pub keyframes: usize,
    /// Shapes the decompiler emitted that the audit (v1.7) measured as not
    /// improving the reconstruction, and took out. 0 for a given document.
    pub audited_out: usize,
    /// Shapes whose size, position or stroke width the refinement (v1.9)
    /// moved. 0 for a given document.
    pub refined: usize,
    /// Shapes whose keyframes the motion pass (v1.11) re-measured from
    /// their own pixels. 0 for a given document.
    pub moved: usize,
    /// Shapes drawn as sprites of their own pixels (v1.8). For a given
    /// document, the `image()` shapes other than the plate.
    pub sprites: usize,
}

/// Decompile a video, re-render the result, and score it against the
/// source. The full round trip, measured.
pub fn verify<P: AsRef<Path>>(path: P, opts: &DecompileOptions) -> Result<Verification> {
    verify_with_budget(path, opts, segment_budget())
}

/// [`verify`] with an explicit segment budget (v1.16): an MP4 with more
/// pixels than `segment_pixels` over all its frames is decompiled and
/// measured a segment at a time, holding one segment's frames at once.
pub fn verify_with_budget<P: AsRef<Path>>(path: P, opts: &DecompileOptions, segment_pixels: u64) -> Result<Verification> {
    if let Some(long) = decompile_long(path.as_ref(), opts, true, segment_pixels)? {
        return Ok(summarise(
            long.document,
            long.fidelity.expect("measured"),
            long.frames,
            long.first.segmentation,
            long.first.background,
            long.first.plate,
            long.audited_out,
            long.refined,
            long.moved,
            long.sprites,
        ));
    }
    let frames = load_video_frames(path)?;
    let source_frames = frames.len();
    let chosen = decompile_chosen(&frames, opts)?;
    let fidelity = match chosen.fidelity {
        Some(f) => f,
        None => measure(&frames, &chosen.document)?,
    };
    Ok(summarise(
        chosen.document,
        fidelity,
        source_frames,
        chosen.segmentation,
        chosen.background,
        chosen.plate,
        chosen.audited_out,
        chosen.refined,
        chosen.moved,
        chosen.sprites,
    ))
}

/// Score a given document — hand-edited, or produced by another version —
/// against a video, exactly as [`verify`] scores its own decompile. This
/// is what makes the source *editable and measurable* at once: change a
/// `.vac` by hand, and see what it did to the reconstruction.
pub fn verify_document<P: AsRef<Path>>(path: P, doc: Document) -> Result<Verification> {
    let frames = load_video_frames(path)?;
    let source_frames = frames.len();
    let fidelity = measure(&frames, &doc)?;
    let stmts = || doc.scenes.iter().flat_map(|s| &s.statements);
    let plate = if !stmts().any(|st| matches!(st, Statement::Let { name, shape: Shape::Image { .. } } if name == "plate")) {
        Plate::Vector
    } else if stmts().any(|st| matches!(st, Statement::Animate(a) if a.target == "plate")) {
        Plate::Camera
    } else {
        Plate::Raster
    };
    let sprite_count = doc
        .scenes
        .iter()
        .flat_map(|s| &s.statements)
        .filter(|st| matches!(st, Statement::Let { name, shape: Shape::Image { .. } } if name != "plate"))
        .count();
    Ok(summarise(
        doc,
        fidelity,
        source_frames,
        Segmentation::Auto,
        Background::Auto,
        plate,
        0,
        0,
        0,
        sprite_count,
    ))
}

fn summarise(
    doc: Document,
    fidelity: Fidelity,
    source_frames: usize,
    segmentation: Segmentation,
    background: Background,
    plate: Plate,
    audited_out: usize,
    refined: usize,
    moved: usize,
    sprites: usize,
) -> Verification {
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
        segmentation,
        background,
        plate,
        source_frames,
        vac_bytes: text.len(),
        shapes,
        animate_blocks,
        keyframes,
        audited_out,
        refined,
        moved,
        sprites,
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn fid(residual: f64, ink_iou: f64) -> Fidelity {
        Fidelity {
            frames_compared: 1,
            residual,
            psnr_db: vac_metrics::psnr_from_mse(residual),
            ink_iou,
            missing_ink: 0.0,
            spurious_ink: 0.0,
            worst_psnr_db: 0.0,
            worst_psnr_frame: 0,
            worst_iou: ink_iou,
            worst_iou_frame: 0,
            frames: Vec::new(),
        }
    }

    #[test]
    fn a_candidate_better_on_both_axes_wins() {
        assert!(reconstructs_better(&fid(100.0, 0.9), &fid(200.0, 0.8)));
        assert!(!reconstructs_better(&fid(200.0, 0.8), &fid(100.0, 0.9)));
    }

    #[test]
    fn a_tie_keeps_the_incumbent() {
        assert!(!reconstructs_better(&fid(100.0, 0.9), &fid(100.0, 0.9)));
    }

    /// The trade-off rule weighs relative changes equally, so a small gain
    /// on one axis never buys a large loss on the other — in either
    /// direction.
    #[test]
    fn trade_offs_are_weighed_by_relative_change() {
        // 10% IoU loss for a 1% residual gain: no.
        assert!(!reconstructs_better(&fid(99.0, 0.81), &fid(100.0, 0.90)));
        // 40% IoU gain for a 14% residual loss: yes.
        assert!(reconstructs_better(&fid(114.0, 0.70), &fid(100.0, 0.50)));
        // Residual halved for a 5% IoU loss: yes.
        assert!(reconstructs_better(&fid(50.0, 0.855), &fid(100.0, 0.90)));
    }
}
