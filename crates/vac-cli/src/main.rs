//! `vac` CLI — `unzip`-style entry point for the VAC toolchain.
//!
//! ```text
//! vac compile   ball.vac              # → ball.mp4 (next to source, default from v0.4)
//! vac compile   ball.vac out.gif      # → out.gif  (extension picks format)
//!
//! vac decompile ball.mp4              # → ball.vac (next to source)
//! vac decompile ball.gif out.vac      # → out.vac
//!
//! vac decompile clip.mp4 --no-static-layer   # skip v0.8 scene layering
//!
//! vac verify     ball.mp4              # measure the round trip
//! vac verify     ball.mp4 --json       # machine-readable record
//! vac verify     ball.mp4 --ab         # A/B the v0.8 static-layer pass
//! ```
//!
//! Format dispatch is by extension: `.mp4` (preferred) and `.gif` (legacy
//! pure-Rust path). MP4 requires the system `ffmpeg` binary on PATH.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use vac::DocumentExt;

#[derive(Parser, Debug)]
#[command(
    name = "vac",
    version,
    about = "Video As Code — bidirectional compiler / decompiler.",
    long_about = "VAC compiles a .vac source file to a video and decompiles a \
                  video back to .vac source. Output filenames are optional and \
                  default to placing the result next to the input."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// The measured-search knobs shared by `verify` and `--dump`.
#[derive(Clone, Copy, Debug)]
struct Knobs {
    segmentation: vac::Segmentation,
    background: vac::Background,
    quality: f32,
    plate: vac::Plate,
    audit: bool,
    sprites: bool,
}

fn parse_quality(s: &str) -> std::result::Result<f32, String> {
    let q: f32 = s.parse().map_err(|_| format!("`{s}` is not a number"))?;
    if (0.0..=100.0).contains(&q) {
        Ok(q)
    } else {
        Err(format!("quality must be between 0 and 100, got {q}"))
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum SegArg {
    Auto,
    Components,
    Regions,
    Edges,
}

impl From<SegArg> for vac::Segmentation {
    fn from(a: SegArg) -> Self {
        match a {
            SegArg::Auto => vac::Segmentation::Auto,
            SegArg::Components => vac::Segmentation::Components,
            SegArg::Regions => vac::Segmentation::Regions,
            SegArg::Edges => vac::Segmentation::Edges,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum PlateArg {
    Auto,
    Vector,
    Raster,
}

impl From<PlateArg> for vac::Plate {
    fn from(a: PlateArg) -> Self {
        match a {
            PlateArg::Auto => vac::Plate::Auto,
            PlateArg::Vector => vac::Plate::Vector,
            PlateArg::Raster => vac::Plate::Raster,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum BgArg {
    Auto,
    Corners,
    Border,
}

impl From<BgArg> for vac::Background {
    fn from(a: BgArg) -> Self {
        match a {
            BgArg::Auto => vac::Background::Auto,
            BgArg::Corners => vac::Background::Corners,
            BgArg::Border => vac::Background::Border,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Compile a `.vac` source file into a video. Default container is MP4.
    Compile {
        /// Path to a `.vac` source file.
        input: PathBuf,
        /// Output video path. Defaults to `<input>.mp4` next to the source.
        /// Extension picks the container: `.mp4` (default) or `.gif`.
        output: Option<PathBuf>,
    },
    /// Decompile a video file back into `.vac` source.
    Decompile {
        /// Path to a video file (`.mp4` or `.gif`).
        input: PathBuf,
        /// Output `.vac` path. Defaults to `<input>.vac` next to the video.
        output: Option<PathBuf>,
        /// Skip static-scene layering (v0.8) and treat every frame as a
        /// single layer, the way v0.7 did. Useful for measuring what the
        /// layering pass actually changed.
        #[arg(long)]
        no_static_layer: bool,
        /// How foreground pixels are grouped into shapes. `auto` (the
        /// default) decompiles each way, re-renders each, and keeps the
        /// one that reconstructs the video best.
        #[arg(long, value_enum, default_value_t = SegArg::Auto)]
        segmentation: SegArg,
        /// Which rule picks the background colour. `auto` (the default)
        /// tries both and keeps the better reconstruction.
        #[arg(long, value_enum, default_value_t = BgArg::Auto)]
        background: BgArg,
        /// The rate–distortion dial, 0–100. 50 (the default) is VAC's
        /// standard output; higher keeps more keyframes and finer outlines
        /// (larger, more faithful source), lower fewer and coarser.
        #[arg(long, default_value_t = 50.0, value_parser = parse_quality)]
        quality: f32,
        /// How the static scene is written. `auto` (the default) keeps
        /// vector shapes unless they reconstruct under 30 dB and a raster
        /// plate — one JPEG `image()` of the scenery, movers still drawn as
        /// code on top — reconstructs at least twice as well.
        #[arg(long, value_enum, default_value_t = PlateArg::Auto)]
        plate: PlateArg,
        /// Skip the shape audit (v1.7), which measures every emitted shape
        /// against its own absence and drops the ones the reconstruction
        /// is no better with. Useful for measuring what the audit changed.
        #[arg(long)]
        no_audit: bool,
        /// Never draw a shape as a sprite of its own pixels (v1.8). By
        /// default a shape that only moves is replaced by one when that
        /// reconstructs its footprint at least twice as well.
        #[arg(long)]
        no_sprites: bool,
        /// Rate control: instead of a fixed quality, find the smallest
        /// source whose reconstruction reaches this PSNR (dB).
        #[arg(long, value_name = "DB", conflicts_with = "quality")]
        target_psnr: Option<f64>,
    },
    /// Measure how much of a video survives the round trip through
    /// `.vac` source: decompile it, re-render the result, and compare
    /// the pixels against the original.
    ///
    /// Every VAC version through v0.8 reported shape counts, keyframe
    /// counts and `.vac` byte size — all of which describe how *small*
    /// the output is and none of which describe how *right* it is.
    /// This is the other axis.
    Verify {
        /// Path to a video file (`.mp4` or `.gif`).
        input: PathBuf,
        /// Emit a machine-readable record instead of a human report.
        #[arg(long)]
        json: bool,
        /// Run the pipeline with and without static-scene layering and
        /// print the difference. The `--no-static-layer` flag has
        /// existed since v0.8 "for measuring what the layering pass
        /// actually changed"; this is what actually measures it.
        #[arg(long)]
        ab: bool,
        /// Skip static-scene layering. Ignored when `--ab` is set.
        #[arg(long)]
        no_static_layer: bool,
        /// How foreground pixels are grouped into shapes. `auto` (the
        /// default) decompiles each way, re-renders each, and keeps the
        /// one that reconstructs the video best.
        #[arg(long, value_enum, default_value_t = SegArg::Auto)]
        segmentation: SegArg,
        /// Which rule picks the background colour. `auto` (the default)
        /// tries both and keeps the better reconstruction.
        #[arg(long, value_enum, default_value_t = BgArg::Auto)]
        background: BgArg,
        /// The rate–distortion dial, 0–100. 50 (the default) is VAC's
        /// standard output; higher keeps more keyframes and finer outlines
        /// (larger, more faithful source), lower fewer and coarser.
        #[arg(long, default_value_t = 50.0, value_parser = parse_quality)]
        quality: f32,
        /// How the static scene is written. `auto` (the default) keeps
        /// vector shapes unless they reconstruct under 30 dB and a raster
        /// plate — one JPEG `image()` of the scenery, movers still drawn as
        /// code on top — reconstructs at least twice as well.
        #[arg(long, value_enum, default_value_t = PlateArg::Auto)]
        plate: PlateArg,
        /// Skip the shape audit (v1.7), which measures every emitted shape
        /// against its own absence and drops the ones the reconstruction
        /// is no better with. Useful for measuring what the audit changed.
        #[arg(long)]
        no_audit: bool,
        /// Never draw a shape as a sprite of its own pixels (v1.8). By
        /// default a shape that only moves is replaced by one when that
        /// reconstructs its footprint at least twice as well.
        #[arg(long)]
        no_sprites: bool,
        /// Write PNG triptychs — source | reconstruction | ink map — of
        /// the worst-IoU frame, the worst-PSNR frame and the middle frame
        /// into this directory. In the ink map, white is ink both agree
        /// on, red is source ink the reconstruction missed, blue is ink
        /// the reconstruction invented, and grey marks pixels the source
        /// left transparent, which are not scored.
        #[arg(long, value_name = "DIR")]
        dump: Option<PathBuf>,
        /// Measure this `.vac` file against the video instead of
        /// decompiling it — a hand-edited source, or one written by an
        /// older version.
        #[arg(long, value_name = "FILE", conflicts_with = "ab")]
        vac: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Compile { input, output } => run_compile(&input, output.as_deref()),
        Cmd::Decompile {
            input,
            output,
            no_static_layer,
            segmentation,
            background,
            quality,
            plate,
            no_audit,
            no_sprites,
            target_psnr,
        } => run_decompile(
            &input,
            output.as_deref(),
            &vac::DecompileOptions {
                static_layer: !no_static_layer,
                segmentation: segmentation.into(),
                background: background.into(),
                quality,
                plate: plate.into(),
                audit: !no_audit,
                sprites: !no_sprites,
            },
            target_psnr,
        ),
        Cmd::Verify {
            input,
            json,
            ab,
            no_static_layer,
            segmentation,
            background,
            quality,
            plate,
            no_audit,
            no_sprites,
            dump,
            vac: given,
        } => {
            let knobs = Knobs {
                segmentation: segmentation.into(),
                background: background.into(),
                quality,
                plate: plate.into(),
                audit: !no_audit,
                sprites: !no_sprites,
            };
            if let Some(file) = &given {
                let doc = vac::read(file).with_context(|| format!("reading {}", file.display()))?;
                let v = vac::verify_document(&input, doc.clone())
                    .with_context(|| format!("verifying {} against {}", file.display(), input.display()))?;
                if json {
                    println!("{{");
                    println!("  \"input\": {:?},", input.display().to_string());
                    println!("  \"vac\": {:?},", file.display().to_string());
                    println!("  \"result\": {}", json_body(&v));
                    println!("}}");
                } else {
                    print_report(&input, &v);
                }
                return match dump {
                    Some(dir) => dump_document(&input, &dir, &doc),
                    None => Ok(()),
                };
            }
            run_verify(&input, json, ab, !no_static_layer, knobs)?;
            match dump {
                Some(dir) => run_dump(&input, &dir, !no_static_layer, knobs),
                None => Ok(()),
            }
        }
    }
}

fn run_verify(
    input: &Path,
    json: bool,
    ab: bool,
    static_layer: bool,
    knobs: Knobs,
) -> Result<()> {
    if ab {
        let on = vac::verify(input, &vac::DecompileOptions { static_layer: true, segmentation: knobs.segmentation, background: knobs.background, quality: knobs.quality, plate: knobs.plate, audit: knobs.audit, sprites: knobs.sprites })
            .with_context(|| format!("verifying {} with layering", input.display()))?;
        let off = vac::verify(input, &vac::DecompileOptions { static_layer: false, segmentation: knobs.segmentation, background: knobs.background, quality: knobs.quality, plate: knobs.plate, audit: knobs.audit, sprites: knobs.sprites })
            .with_context(|| format!("verifying {} without layering", input.display()))?;
        if json {
            println!("{{");
            println!("  \"input\": {:?},", input.display().to_string());
            println!("  \"with_static_layer\": {},", json_body(&on));
            println!("  \"without_static_layer\": {}", json_body(&off));
            println!("}}");
        } else {
            print_ab(input, &on, &off);
        }
        return Ok(());
    }

    let v = vac::verify(input, &vac::DecompileOptions { static_layer, segmentation: knobs.segmentation, background: knobs.background, quality: knobs.quality, plate: knobs.plate, audit: knobs.audit, sprites: knobs.sprites })
        .with_context(|| format!("verifying {}", input.display()))?;
    if json {
        println!("{{");
        println!("  \"input\": {:?},", input.display().to_string());
        println!("  \"static_layer\": {static_layer},");
        println!("  \"result\": {}", json_body(&v));
        println!("}}");
    } else {
        print_report(input, &v);
    }
    Ok(())
}

fn print_report(input: &Path, v: &vac::Verification) {
    let f = &v.fidelity;
    let d = &v.document;
    println!(
        "{}  {}x{} @{}fps  {} frames",
        input.display(),
        d.canvas.width,
        d.canvas.height,
        d.canvas.fps,
        v.source_frames
    );
    println!();
    println!("  fidelity — how much of the picture survived");
    println!(
        "    ink IoU       {:.4}   (worst frame {:.4} @ #{})",
        f.ink_iou, f.worst_iou, f.worst_iou_frame
    );
    println!(
        "    residual      {:.2}     (per-channel MSE; 100 ⇒ each R/G/B within ~10)",
        f.residual
    );
    println!(
        "    psnr          {:.2} dB  (worst frame {:.2} dB @ #{})",
        f.psnr_db, f.worst_psnr_db, f.worst_psnr_frame
    );
    println!(
        "    missing ink   {:.1}%     spurious ink {:.1}%",
        f.missing_ink * 100.0,
        f.spurious_ink * 100.0
    );
    println!();
    println!("  structure — not fidelity, but what it cost");
    println!("    segmentation  {:?}", v.segmentation);
    println!("    background    {:?}", v.background);
    println!("    plate         {:?}", v.plate);
    if v.audited_out > 0 {
        println!("    audit         took out {} shape(s) the reconstruction was no better with", v.audited_out);
    }
    if v.sprites > 0 {
        println!("    sprites       {} shape(s) drawn as their own pixels", v.sprites);
    }
    println!("    shapes        {}", v.shapes);
    println!("    animate       {}", v.animate_blocks);
    println!("    keyframes     {}", v.keyframes);
    println!("    .vac size     {} bytes", v.vac_bytes);
}

fn print_ab(input: &Path, on: &vac::Verification, off: &vac::Verification) {
    println!("{}  — static-scene layering A/B", input.display());
    println!();
    println!(
        "  {:<16} {:>12} {:>12} {:>12}",
        "", "layered", "single-layer", "delta"
    );
    let row = |label: &str, a: f64, b: f64, prec: usize| {
        println!(
            "  {:<16} {:>12.*} {:>12.*} {:>+12.*}",
            label,
            prec,
            a,
            prec,
            b,
            prec,
            a - b
        );
    };
    row("ink IoU", on.fidelity.ink_iou, off.fidelity.ink_iou, 4);
    row("worst IoU", on.fidelity.worst_iou, off.fidelity.worst_iou, 4);
    row("residual", on.fidelity.residual, off.fidelity.residual, 2);
    row("psnr dB", on.fidelity.psnr_db, off.fidelity.psnr_db, 2);
    row(
        "missing ink %",
        on.fidelity.missing_ink * 100.0,
        off.fidelity.missing_ink * 100.0,
        1,
    );
    row(
        "spurious ink %",
        on.fidelity.spurious_ink * 100.0,
        off.fidelity.spurious_ink * 100.0,
        1,
    );
    println!();
    let irow = |label: &str, a: usize, b: usize| {
        println!(
            "  {:<16} {:>12} {:>12} {:>+12}",
            label,
            a,
            b,
            a as i64 - b as i64
        );
    };
    irow("shapes", on.shapes, off.shapes);
    irow("animate", on.animate_blocks, off.animate_blocks);
    irow("keyframes", on.keyframes, off.keyframes);
    irow(".vac bytes", on.vac_bytes, off.vac_bytes);
}

/// Hand-rolled so the workspace takes no serialisation dependency.
/// Every field is a number or a bool, so no escaping is needed.
fn json_body(v: &vac::Verification) -> String {
    let f = &v.fidelity;
    format!(
        "{{ \"frames\": {}, \"ink_iou\": {:.6}, \"worst_iou\": {:.6}, \"worst_iou_frame\": {}, \
         \"residual\": {:.6}, \"psnr_db\": {:.4}, \"worst_psnr_db\": {:.4}, \
         \"missing_ink\": {:.6}, \"spurious_ink\": {:.6}, \
         \"shapes\": {}, \"animate_blocks\": {}, \"keyframes\": {}, \"vac_bytes\": {}, \
         \"segmentation\": \"{:?}\", \"background\": \"{:?}\", \"plate\": \"{:?}\", \"audited_out\": {}, \"sprites\": {} }}",
        f.frames_compared,
        f.ink_iou,
        f.worst_iou,
        f.worst_iou_frame,
        f.residual,
        clamp_db(f.psnr_db),
        clamp_db(f.worst_psnr_db),
        f.missing_ink,
        f.spurious_ink,
        v.shapes,
        v.animate_blocks,
        v.keyframes,
        v.vac_bytes,
        v.segmentation,
        v.background,
        v.plate,
        v.audited_out,
        v.sprites,
    )
}

/// JSON has no representation for infinity, which is what a pixel-exact
/// frame scores. Clamp rather than emit something no parser accepts.
fn clamp_db(db: f64) -> f64 {
    if db.is_finite() {
        db
    } else {
        99.0
    }
}

fn run_compile(input: &Path, output: Option<&Path>) -> Result<()> {
    let out_path: PathBuf = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| input.with_extension("mp4"));

    let doc = vac::read(input).with_context(|| format!("reading {}", input.display()))?;

    eprintln!(
        "compiling {} ({}x{} @ {}fps, {} scene{}) → {}",
        input.display(),
        doc.canvas.width,
        doc.canvas.height,
        doc.canvas.fps,
        doc.scenes.len(),
        if doc.scenes.len() == 1 { "" } else { "s" },
        out_path.display(),
    );

    doc.to_video(&out_path)
        .with_context(|| format!("compiling to {}", out_path.display()))?;

    eprintln!("✓ wrote {}", out_path.display());
    Ok(())
}

fn run_decompile(
    input: &Path,
    output: Option<&Path>,
    opts: &vac::DecompileOptions,
    target_psnr: Option<f64>,
) -> Result<()> {
    let out_path: PathBuf = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| input.with_extension("vac"));

    eprintln!("decompiling {} ...", input.display());
    let doc = match target_psnr {
        None => vac::read_video_with(input, opts)
            .with_context(|| format!("decompiling {}", input.display()))?,
        Some(target) => {
            let frames = vac::load_video_frames(input)
                .with_context(|| format!("loading {}", input.display()))?;
            let rc = vac::decompile_to_target(frames, opts, target)?;
            for (q, bytes, psnr) in &rc.probes {
                eprintln!("  probe quality {q:5.1}  {bytes:8} bytes  {psnr:6.2} dB");
            }
            eprintln!(
                "  {} quality {:.1}: {} bytes at {:.2} dB (target {target:.2} dB)",
                if rc.met { "✓ met at" } else { "✗ not reachable; best is" },
                rc.quality,
                rc.vac_bytes,
                rc.fidelity.psnr_db
            );
            rc.document
        }
    };

    doc.to_vac(&out_path)
        .with_context(|| format!("writing {}", out_path.display()))?;

    eprintln!(
        "✓ wrote {} ({}x{} @ {}fps, {} scene{})",
        out_path.display(),
        doc.canvas.width,
        doc.canvas.height,
        doc.canvas.fps,
        doc.scenes.len(),
        if doc.scenes.len() == 1 { "" } else { "s" },
    );
    Ok(())
}

fn run_dump(input: &Path, dir: &Path, static_layer: bool, knobs: Knobs) -> Result<()> {
    let frames = vac::load_video_frames(input)
        .with_context(|| format!("loading {}", input.display()))?;
    let doc = vac::decompile_frames(frames.clone(), &vac::DecompileOptions { static_layer, segmentation: knobs.segmentation, background: knobs.background, quality: knobs.quality, plate: knobs.plate, audit: knobs.audit, sprites: knobs.sprites })?;
    dump_frames(input, dir, &frames, &doc)
}

fn dump_document(input: &Path, dir: &Path, doc: &vac::Document) -> Result<()> {
    let frames = vac::load_video_frames(input)
        .with_context(|| format!("loading {}", input.display()))?;
    dump_frames(input, dir, &frames, doc)
}

fn dump_frames(input: &Path, dir: &Path, frames: &[vac::VideoFrame], doc: &vac::Document) -> Result<()> {
    let frames = frames.to_vec();
    let doc = doc.clone();
    let fid = vac::measure(&frames, &doc)?;
    let recon = vac::render_aligned(&frames, &doc)?;
    let bg = vac::ink_background(&frames);
    let opts = vac::CompareOptions::default();
    std::fs::create_dir_all(dir)?;
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clip".into());

    let mut picks = vec![
        ("worst_iou", fid.worst_iou_frame),
        ("worst_psnr", fid.worst_psnr_frame),
        ("mid", frames.len() / 2),
    ];
    picks.dedup_by_key(|p| p.1);
    for (label, i) in picks {
        let (src, rec) = (&frames[i], &recon[i]);
        let (w, h) = (src.width, src.height);
        let mut out = image::RgbImage::new(w * 3 + 8, h);
        for y in 0..h {
            for x in 0..w {
                let k = ((y * w + x) * 4) as usize;
                let s = &src.pixels[k..k + 4];
                let r = &rec.rgba[k..k + 4];
                out.put_pixel(x, y, image::Rgb([s[0], s[1], s[2]]));
                out.put_pixel(w + 4 + x, y, image::Rgb([r[0], r[1], r[2]]));
                // Pixels the source never drew are not scored at all;
                // show them as neutral grey rather than as ink verdicts.
                let ink = if s[3] < 128 {
                    [70, 70, 76]
                } else {
                    match (vac::is_ink(s, bg, &opts), vac::is_ink(r, bg, &opts)) {
                    (true, true) => [235, 235, 235],
                    (true, false) => [230, 50, 50],
                    (false, true) => [60, 110, 240],
                    (false, false) => [24, 24, 28],
                    }
                };
                out.put_pixel(2 * w + 8 + x, y, image::Rgb(ink));
            }
        }
        let score = &fid.frames[i];
        let path = dir.join(format!("{stem}_{label}_f{i:04}.png"));
        // Small clips (a 32 px loader) are unreadable at 1:1; scale up
        // with nearest-neighbour so every source pixel stays a crisp block.
        let k = (480 / w.max(1)).clamp(1, 12);
        let out = if k > 1 {
            image::imageops::resize(&out, out.width() * k, out.height() * k, image::imageops::FilterType::Nearest)
        } else {
            out
        };
        out.save(&path)
            .with_context(|| format!("writing {}", path.display()))?;
        eprintln!(
            "  wrote {}  (IoU {:.4}, psnr {:.2} dB)",
            path.display(),
            score.ink_iou,
            score.psnr_db
        );
    }
    Ok(())
}
