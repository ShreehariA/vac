//! Image loading wrapper.  Decodes any format `image` supports
//! (PNG / JPEG / WebP / GIF first frame), normalises to RGBA8, and
//! optionally downscales so the longest edge is no greater than
//! `max_edge`.

use image::imageops::FilterType;
use image::GenericImageView;

#[derive(Debug, Clone)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn load_rgba(path: &str, max_edge: u32) -> Result<RgbaImage, image::ImageError> {
    let img = image::open(path)?;
    let (mut w, mut h) = img.dimensions();
    let img = if max_edge > 0 && (w > max_edge || h > max_edge) {
        let scale = max_edge as f32 / w.max(h) as f32;
        w = (w as f32 * scale).round() as u32;
        h = (h as f32 * scale).round() as u32;
        img.resize_exact(w, h, FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.into_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Ok(RgbaImage {
        width: w,
        height: h,
        pixels: rgba.into_raw(),
    })
}
