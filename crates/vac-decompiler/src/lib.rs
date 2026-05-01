//! VAC decompiler — video frames → `.vac` AST.
//!
//! v0.1 covers the "moving ball" problem: a single solid-coloured
//! circle on a solid background.  The pipeline is intentionally
//! algorithmic, not classifier-based:
//!
//! 1. **Background estimate**: sample frame corners → median colour.
//! 2. **Foreground mask**: pixels whose colour distance from the
//!    background exceeds a threshold.
//! 3. **Connected-component labeling** (4-connectivity, two-pass
//!    union-find) → blobs.
//! 4. **Largest blob**: centroid, bounding box, mean RGB.
//! 5. **Trajectory simplification**: Ramer-Douglas-Peucker on the
//!    `(cx, cy)` polyline → keyframes.
//! 6. **AST emission**: shape declared at its first detected position;
//!    median radius and colour across all frames; linear easing in v0.1.
//!
//! Easing classification, multi-shape tracking, scale/opacity
//! estimation, and proper contour tracing all build on this same
//! skeleton in later versions.

pub mod detect;
pub mod frames;
pub mod track;

use std::path::Path;

use vac_format::Document;

#[derive(Debug, thiserror::Error)]
pub enum DecompileError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
    #[error("decompile: {0}")]
    Other(String),
}

/// Decompile an animated GIF into a [`Document`].
pub fn decompile_gif(input: &Path) -> Result<Document, DecompileError> {
    let frames = frames::load_gif(input)?;
    if frames.is_empty() {
        return Err(DecompileError::Other("no frames in input".into()));
    }
    let detections = detect::detect_all(&frames);
    let doc = track::build_document(&frames, &detections);
    Ok(doc)
}
