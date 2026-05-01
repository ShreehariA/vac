//! Frame extraction from a GIF.

use std::path::Path;

use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;
use image::RgbaImage;

use crate::DecompileError;

pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    /// Non-premultiplied RGBA8.
    pub pixels: Vec<u8>,
    /// Frame delay in milliseconds (1/fps approximation, GIF granularity is 10ms).
    pub delay_ms: u32,
}

/// Decode every frame of a GIF.
pub fn load_gif(path: &Path) -> Result<Vec<VideoFrame>, DecompileError> {
    let file = std::fs::File::open(path)?;
    let decoder = GifDecoder::new(std::io::BufReader::new(file))?;
    let frames = decoder.into_frames().collect_frames()?;

    let mut out = Vec::with_capacity(frames.len());
    for f in frames {
        let delay = f.delay();
        let (numer, denom) = delay.numer_denom_ms();
        let delay_ms = if denom == 0 {
            33 // sane fallback
        } else {
            (numer / denom).max(1)
        };
        let img: RgbaImage = f.into_buffer();
        let (w, h) = img.dimensions();
        out.push(VideoFrame {
            width: w,
            height: h,
            pixels: img.into_raw(),
            delay_ms,
        });
    }
    Ok(out)
}

/// Estimate frame rate from the average inter-frame delay.
pub fn estimate_fps(frames: &[VideoFrame]) -> u32 {
    if frames.is_empty() {
        return 30;
    }
    let total_ms: u64 = frames.iter().map(|f| f.delay_ms as u64).sum();
    let avg = (total_ms / frames.len() as u64).max(1);
    let fps = (1000.0 / avg as f32).round() as i64;
    fps.clamp(1, 240) as u32
}
