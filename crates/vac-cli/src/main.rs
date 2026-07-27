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
