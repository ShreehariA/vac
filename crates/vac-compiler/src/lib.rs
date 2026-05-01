//! VAC compiler — `.vac` AST → video frames → GIF.
//!
//! v0.1 produces an animated GIF (no system FFmpeg dependency).
//! When we widen the slice in Phase 2/3, the encoder swaps to MP4
//! via `ffmpeg-next` while the rendering pipeline stays untouched.

pub mod render;

use std::path::Path;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, RgbaImage};
use vac_format::Document;

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("rendering failed: {0}")]
    Render(String),
    #[error("encoding failed: {0}")]
    Encode(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Compile a [`Document`] to an animated GIF on disk.
pub fn compile_to_gif(doc: &Document, out: &Path) -> Result<(), CompileError> {
    let frames = render::render_document(doc)?;
    encode_gif(&frames, doc.canvas.fps, out)
}

fn encode_gif(
    frames: &[render::RenderedFrame],
    fps: u32,
    out: &Path,
) -> Result<(), CompileError> {
    let file = std::fs::File::create(out)?;
    let mut encoder = GifEncoder::new(file);
    encoder
        .set_repeat(Repeat::Infinite)
        .map_err(|e| CompileError::Encode(format!("set_repeat: {e}")))?;

    // GIF stores delay in centiseconds (1/100s). At 30fps that's ~3.33,
    // which `image::Delay` represents as a numer/denom pair (ms units).
    let numer = 1000u32;
    let denom = fps.max(1);

    for f in frames {
        let img = RgbaImage::from_raw(f.width, f.height, f.rgba.clone()).ok_or_else(|| {
            CompileError::Encode(format!(
                "frame has wrong byte count: expected {}, got {}",
                (f.width * f.height * 4) as usize,
                f.rgba.len()
            ))
        })?;
        let delay = Delay::from_numer_denom_ms(numer, denom);
        let frame = Frame::from_parts(img, 0, 0, delay);
        encoder
            .encode_frame(frame)
            .map_err(|e| CompileError::Encode(format!("encode_frame: {e}")))?;
    }
    Ok(())
}
