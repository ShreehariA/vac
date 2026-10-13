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
    decompile_frames(load_video_frames(path)?, opts)
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
    /// The chosen candidate's fidelity, when `Auto` had to measure it.
    fidelity: Option<Fidelity>,
    /// Shapes the audit (v1.7) measured and took out.
    audited_out: usize,
    /// Shapes whose geometry the refinement (v1.9) moved.
    refined: usize,
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
    let chosen = choose(frames, opts)?;
    if !opts.audit && !opts.refine && !opts.sprites {
        return Ok(chosen);
    }
    let fidelity = match chosen.fidelity {
        Some(f) => f,
        None => measure(frames, &chosen.document)?,
    };
    let a = if opts.audit {
        audit_measured(frames, chosen.document, fidelity)?
    } else {
        Audit {
            document: chosen.document,
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
        sprites,
        ..chosen
    })
}

fn choose(frames: &[VideoFrame], opts: &DecompileOptions) -> Result<Chosen> {
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
            fidelity: None,
            audited_out: 0,
            refined: 0,
            sprites: 0,
        });
    }
    // The static layer and its stripping depend only on the background
    // rule, so each background is prepared once and its segmentation
    // candidates share it read-only, running concurrently (at most
    // `MAX_CONCURRENT_CANDIDATES` at once). The winner is then picked in a
    // fixed candidate order, so the result is the same as a sequential
    // search.
    let width = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, MAX_CONCURRENT_CANDIDATES);
    let mut combos: Vec<(Background, Segmentation)> = Vec::new();
    let mut results: Vec<Result<(Document, Fidelity)>> = Vec::new();
    for &background in bgs {
        let prepared = vac_decompiler::prepare(frames.to_vec(), opts.static_layer, background)?;
        for chunk in segs.chunks(width) {
            let chunk_results: Vec<Result<(Document, Fidelity)>> = std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|&segmentation| {
                        let prepared = &prepared;
                        scope.spawn(move || -> Result<(Document, Fidelity)> {
                            let doc = vac_decompiler::finish(prepared, segmentation, opts.quality, Plate::Vector);
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
            combos.extend(chunk.iter().map(|&s| (background, s)));
            results.extend(chunk_results);
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
                fidelity: Some(fid),
                audited_out: 0,
                refined: 0,
                sprites: 0,
            });
        }
    }
    let best = best.expect("at least one candidate");
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
    for &plate in tries {
        let prepared = match plate {
            Plate::Camera => vac_decompiler::prepare_camera(frames.to_vec(), best.background)?,
            _ => vac_decompiler::prepare_raster(frames.to_vec(), best.background)?,
        };
        let Some(prepared) = prepared else { continue };
        let doc = vac_decompiler::finish(&prepared, best.segmentation, opts.quality, plate);
        if !has_plate(&doc) {
            continue;
        }
        let fid = measure(frames, &doc)?;
        plated.push(Chosen {
            document: doc,
            segmentation: best.segmentation,
            background: best.background,
            plate,
            fidelity: Some(fid),
            audited_out: 0,
            refined: 0,
            sprites: 0,
        });
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
                .any(|t| matches!(t, Transform::Scale(_) | Transform::Rotate(_) | Transform::Fill(_)))
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
const MAX_CONCURRENT_CANDIDATES: usize = 3;

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
    let count = vac_compiler::render::frame_count(doc);
    let idx = aligned_indices(original, count, doc.canvas.fps);
    // The ink reference is measured from the source frames themselves,
    // by `vac-metrics`, and deliberately *not* by calling the
    // decompiler's own `estimate_background`. If the metric shared the
    // pipeline's background rule, tuning that rule would silently move
    // every committed baseline and "the decompiler improved" would be
    // indistinguishable from "the ruler moved".
    let src: Vec<vac_metrics::FrameRef> = original
        .iter()
        .map(|f| vac_metrics::FrameRef::new(f.width, f.height, &f.pixels))
        .collect();
    let bg = vac_metrics::modal_background(&src);
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
    /// Shapes drawn as sprites of their own pixels (v1.8). For a given
    /// document, the `image()` shapes other than the plate.
    pub sprites: usize,
}

/// Decompile a video, re-render the result, and score it against the
/// source. The full round trip, measured.
pub fn verify<P: AsRef<Path>>(path: P, opts: &DecompileOptions) -> Result<Verification> {
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
