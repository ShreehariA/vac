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
use clap::{Parser, Subcommand};
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
        } => run_decompile(
            &input,
            output.as_deref(),
            &vac::DecompileOptions {
                static_layer: !no_static_layer,
            },
        ),
        Cmd::Verify {
            input,
            json,
            ab,
            no_static_layer,
        } => run_verify(&input, json, ab, !no_static_layer),
    }
}

fn run_verify(input: &Path, json: bool, ab: bool, static_layer: bool) -> Result<()> {
    if ab {
        let on = vac::verify(input, &vac::DecompileOptions { static_layer: true })
            .with_context(|| format!("verifying {} with layering", input.display()))?;
        let off = vac::verify(input, &vac::DecompileOptions { static_layer: false })
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

    let v = vac::verify(input, &vac::DecompileOptions { static_layer })
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
         \"shapes\": {}, \"animate_blocks\": {}, \"keyframes\": {}, \"vac_bytes\": {} }}",
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
) -> Result<()> {
    let out_path: PathBuf = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| input.with_extension("vac"));

    eprintln!("decompiling {} ...", input.display());
    let doc = vac::read_video_with(input, opts)
        .with_context(|| format!("decompiling {}", input.display()))?;

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
