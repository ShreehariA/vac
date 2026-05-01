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
//! // .vac → video
//! let doc = vac::read("ball.vac")?;
//! doc.to_video("any_name.gif")?;
//!
//! // video → .vac
//! let doc = vac::read_video("any_name.gif")?;
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
//! The output format is selected by the destination's file extension —
//! `.gif` today, `.mp4` once Phase 2 lands. Choose any filename you like;
//! VAC just looks at the extension.

use std::path::Path;

// Re-export the AST and friends so callers only need `use vac::*`.
pub use vac_format::{
    Animation, Canvas, Color, Document, Easing, Keyframe, ParseError, Property, Scene, Shape,
    Statement, Transform, Value,
};

// Re-export the underlying error types so callers can pattern-match if they
// want to, without taking direct dependencies on the sub-crates.
pub use vac_compiler::CompileError;
pub use vac_decompiler::DecompileError;

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
    #[error("unsupported video format `.{ext}` (supported: gif)")]
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
/// Format is dispatched by extension. Currently supported: `.gif`.
pub fn read_video<P: AsRef<Path>>(path: P) -> Result<Document> {
    let path = path.as_ref();
    match video_format(path)? {
        VideoFormat::Gif => Ok(vac_decompiler::decompile_gif(path)?),
    }
}

/// Render a [`Document`] to a video file on disk.
///
/// Format is dispatched by extension. Currently supported: `.gif`.
/// Future: `.mp4` (Phase 2 swaps in `ffmpeg-next`).
pub fn write_video<P: AsRef<Path>>(doc: &Document, path: P) -> Result<()> {
    let path = path.as_ref();
    match video_format(path)? {
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
        "gif" => Ok(VideoFormat::Gif),
        other => Err(Error::UnsupportedVideoFormat {
            ext: other.to_string(),
        }),
    }
}
