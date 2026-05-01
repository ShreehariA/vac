//! Read a video file and decompile it back to `.vac` source.
//!
//! Demonstrates the pandas-style library API:
//!
//! ```text
//! cargo run -p vac --example convert_to_code -- ball.gif decompiled.vac
//! ```
//!
//! Both filenames can be anything you want.

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
            eprintln!("convert_to_code: {e}");
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
                "usage: convert_to_code <input.gif> [output.vac]",
            )))
        }
    };
    let output: PathBuf = match args.next() {
        Some(s) => s.into(),
        None => input.with_extension("vac"),
    };

    let doc = vac::read_video(&input)?;
    doc.to_vac(&output)?;

    Ok(output)
}
