//! VAC decompiler — video frames → `.vac` AST.
//!
//! Two front-ends feed the same algorithmic pipeline:
//!
//! - [`decompile_gif`] — pure-Rust via the `image` crate. Centisecond
//!   delay quantisation drifts the recovered FPS / duration.
//! - [`decompile_mp4`] — H.264/MP4 via the system `ffmpeg`/`ffprobe`
//!   binaries (subprocess). Exact FPS, exact accumulated `t_ms`.
//!
//! The pipeline itself is intentionally algorithmic, not classifier-based:
//!
//! 1. **Background estimate**: sample frame corners → median colour.
//! 2. **Foreground mask**: pixels whose colour distance from the
//!    background exceeds a threshold.
//! 3. **Connected-component labeling** (4-connectivity, two-pass
//!    union-find) → all blobs above the minimum-area threshold.
//! 4. **Per-blob measurements**: centroid, bounding box, interior-only
//!    mean RGB, area, and the seven Hu image-moment invariants.
//! 5. **Cross-frame tracking** (v0.3): lowest-cost-first greedy
//!    bipartite matching between active tracks and the current
//!    frame's detections, scored on position + colour + size + Hu.
//!    Unmatched detections start new tracks; a 3-frame gap tolerance
//!    survives brief occlusion or detection drop-outs.
//! 6. **Per-track simplification**: temporal-aware Ramer-Douglas-Peucker
//!    on `x(t)` and `y(t)` independently → keyframes (v0.2).
//! 7. **Easing classification** (v0.2): SSE comparison against
//!    canonical easing templates between consecutive keyframes.
//! 8. **AST emission**: one `let shape_<n>` + fill + stroke + animate
//!    block per recovered track.
//!
//! Scale/opacity estimation and proper contour tracing build on this
//! same skeleton in later versions.

pub mod detect;
pub mod frames;
pub mod frames_mp4;
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
    decompile_frames(&frames)
}

/// Decompile an MP4 (any codec ffmpeg can decode) into a [`Document`].
///
/// Requires the system `ffmpeg` and `ffprobe` binaries on PATH.
/// Use [`frames_mp4::ffmpeg_available`] for a runtime probe.
pub fn decompile_mp4(input: &Path) -> Result<Document, DecompileError> {
    let frames = frames_mp4::load_mp4(input)?;
    decompile_frames(&frames)
}

fn decompile_frames(frames: &[frames::VideoFrame]) -> Result<Document, DecompileError> {
    if frames.is_empty() {
        return Err(DecompileError::Other("no frames in input".into()));
    }
    let detections = detect::detect_all(frames);
    Ok(track::build_document(frames, &detections))
}
