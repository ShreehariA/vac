//! Read a `.vac` source file and render it to a video.
//!
//! Demonstrates the pandas-style library API:
//!
//! ```text
//! cargo run -p vac --example create_video -- examples/ball.vac ball.gif
//! ```
//!
//! Output filename can be anything you want — VAC chooses the encoder
//! by extension (`.gif` today, `.mp4` once Phase 2 lands).

use std::path::PathBuf;
use std::process::ExitCode;

use vac::DocumentExt;

fn main() -> ExitCode {
    match run() {
        Ok(out) => {
            eprintln!("✓ wrote {}", out.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("create_video: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> vac::Result<PathBuf> {
    let mut args = std::env::args().skip(1);
    let input: PathBuf = match args.next() {
        Some(s) => s.into(),
        None => {
            return Err(vac::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "usage: create_video <input.vac> [output.gif]",
            )))
        }
    };
    let output: PathBuf = match args.next() {
        Some(s) => s.into(),
        None => input.with_extension("gif"),
    };

    // The whole flow is just two lines — pandas-style.
    let doc = vac::read(&input)?;
    doc.to_video(&output)?;

    Ok(output)
}
