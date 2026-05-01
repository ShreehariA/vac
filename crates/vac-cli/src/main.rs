//! `vac` CLI — `unzip`-style entry point for the VAC toolchain.
//!
//! ```text
//! vac compile   ball.vac              # → ball.gif (next to source)
//! vac compile   ball.vac out.gif      # → out.gif  (positional, name what you like)
//!
//! vac decompile ball.gif              # → ball.vac (next to source)
//! vac decompile ball.gif out.vac      # → out.vac
//! ```

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
    /// Compile a `.vac` source file into a video (currently `.gif`).
    Compile {
        /// Path to a `.vac` source file.
        input: PathBuf,
        /// Output video path. Defaults to `<input>.gif` next to the source.
        /// You can use any name you want; the extension picks the format.
        output: Option<PathBuf>,
    },
    /// Decompile a video file back into `.vac` source.
    Decompile {
        /// Path to a video file (currently `.gif`).
        input: PathBuf,
        /// Output `.vac` path. Defaults to `<input>.vac` next to the video.
        /// You can use any name you want.
        output: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Compile { input, output } => run_compile(&input, output.as_deref()),
        Cmd::Decompile { input, output } => run_decompile(&input, output.as_deref()),
    }
}

fn run_compile(input: &Path, output: Option<&Path>) -> Result<()> {
    let out_path: PathBuf = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| input.with_extension("gif"));

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

fn run_decompile(input: &Path, output: Option<&Path>) -> Result<()> {
    let out_path: PathBuf = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| input.with_extension("vac"));

    eprintln!("decompiling {} ...", input.display());
    let doc =
        vac::read_video(input).with_context(|| format!("decompiling {}", input.display()))?;

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
