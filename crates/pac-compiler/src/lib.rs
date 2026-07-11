//! PAC compiler: `.pac` AST → PNG via tiny-skia.
//!
//! Pipeline:
//!   1. Allocate a `Pixmap(canvas.width, canvas.height)`.
//!   2. If `background` is set, fill the whole pixmap.
//!   3. Walk `shapes` in declaration order (back-to-front painter's
//!      algorithm) and rasterise each polygon with its fill colour.
//!   4. Un-premultiply → RGBA8 → PNG.

use pac_format::{Color, Document, Fill, PathSeg, Shape};
use thiserror::Error;
use tiny_skia::{
    FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point as SkPoint,
    Shader, SpreadMode, Transform,
};

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("render: {0}")]
    Render(String),
    #[error("encode PNG: {0}")]
    Encode(#[from] png::EncodingError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

mod png {
    pub use ::image::ImageError as EncodingError;
}

pub fn render(doc: &Document) -> Result<image::RgbaImage, CompileError> {
    let w = doc.canvas.width;
    let h = doc.canvas.height;
    let mut pixmap = Pixmap::new(w, h)
        .ok_or_else(|| CompileError::Render(format!("invalid canvas {w}x{h}")))?;

    if let Some(bg) = doc.background {
        fill_canvas(&mut pixmap, bg);
    }

    for decl in &doc.shapes {
        match &decl.shape {
            Shape::Polygon(pts) => draw_polygon(&mut pixmap, pts, &decl.fill),
            Shape::Path(segs) => draw_path(&mut pixmap, segs, &decl.fill),
        }
    }

    let rgba = unpremultiply(pixmap.data(), w, h);
    image::RgbaImage::from_raw(w, h, rgba)
        .ok_or_else(|| CompileError::Render("failed to wrap RGBA buffer".to_string()))
}

pub fn render_to_png(doc: &Document, path: &str) -> Result<(), CompileError> {
    let img = render(doc)?;
    img.save_with_format(path, image::ImageFormat::Png)?;
    Ok(())
}

fn fill_canvas(pixmap: &mut Pixmap, c: Color) {
    let mut paint = Paint::default();
    paint.anti_alias = false;
    paint.set_color_rgba8(c.r, c.g, c.b, c.a);
    let rect = tiny_skia::Rect::from_xywh(
        0.0,
        0.0,
        pixmap.width() as f32,
        pixmap.height() as f32,
    ).expect("non-zero canvas");
    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
}

fn draw_polygon(pixmap: &mut Pixmap, pts: &[pac_format::Point], fill: &Fill) {
    if pts.len() < 3 {
        return;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].x, pts[0].y);
    for p in &pts[1..] {
        pb.line_to(p.x, p.y);
    }
    pb.close();
    let path = match pb.finish() {
        Some(p) => p,
        None => return,
    };
    let paint = paint_from_fill(fill);
    pixmap.fill_path(&path, &paint, FillRule::EvenOdd, Transform::identity(), None);
}

fn draw_path(pixmap: &mut Pixmap, segs: &[PathSeg], fill: &Fill) {
    if segs.is_empty() {
        return;
    }
    let mut pb = PathBuilder::new();
    for seg in segs {
        match *seg {
            PathSeg::MoveTo(p) => pb.move_to(p.x, p.y),
            PathSeg::LineTo(p) => pb.line_to(p.x, p.y),
            PathSeg::CurveTo { c1, c2, p } => pb.cubic_to(c1.x, c1.y, c2.x, c2.y, p.x, p.y),
        }
    }
    pb.close();
    let path = match pb.finish() {
        Some(p) => p,
        None => return,
    };
    let paint = paint_from_fill(fill);
    pixmap.fill_path(&path, &paint, FillRule::EvenOdd, Transform::identity(), None);
}

fn paint_from_fill<'a>(fill: &Fill) -> Paint<'a> {
    let mut paint = Paint::default();
    paint.anti_alias = true;
    match fill {
        Fill::Solid(c) => {
            paint.set_color_rgba8(c.r, c.g, c.b, c.a);
        }
        Fill::LinearGradient { c0, c1, p0, p1 } => {
            let stops = vec![
                GradientStop::new(0.0, sk_color(c0)),
                GradientStop::new(1.0, sk_color(c1)),
            ];
            if let Some(shader) = LinearGradient::new(
                SkPoint::from_xy(p0.x, p0.y),
                SkPoint::from_xy(p1.x, p1.y),
                stops,
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                paint.shader = shader;
            } else {
                // Degenerate gradient (p0 == p1): fall back to a solid
                // colour halfway between the two stops.
                let r = ((c0.r as u16 + c1.r as u16) / 2) as u8;
                let g = ((c0.g as u16 + c1.g as u16) / 2) as u8;
                let b = ((c0.b as u16 + c1.b as u16) / 2) as u8;
                let a = ((c0.a as u16 + c1.a as u16) / 2) as u8;
                paint.set_color_rgba8(r, g, b, a);
                paint.shader = Shader::SolidColor(tiny_skia::Color::from_rgba8(r, g, b, a));
            }
        }
    }
    paint
}

fn sk_color(c: &Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn unpremultiply(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let n = (w as usize) * (h as usize) * 4;
    let mut out = Vec::with_capacity(n);
    for px in src.chunks_exact(4) {
        let (pr, pg, pb, pa) = (px[0], px[1], px[2], px[3]);
        match pa {
            0 => out.extend_from_slice(&[0, 0, 0, 0]),
            255 => out.extend_from_slice(&[pr, pg, pb, pa]),
            _ => {
                let inv = 255.0 / pa as f32;
                let r = ((pr as f32 * inv).round() as u32).min(255) as u8;
                let g = ((pg as f32 * inv).round() as u32).min(255) as u8;
                let b = ((pb as f32 * inv).round() as u32).min(255) as u8;
                out.extend_from_slice(&[r, g, b, pa]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pac_format::*;

    fn rect_doc() -> Document {
        Document {
            canvas: Canvas { width: 40, height: 30 },
            background: Some(Color::rgb(0, 0, 0)),
            shapes: vec![ShapeDecl {
                name: "r".into(),
                shape: Shape::Polygon(vec![
                    Point::new(10.0, 10.0),
                    Point::new(30.0, 10.0),
                    Point::new(30.0, 20.0),
                    Point::new(10.0, 20.0),
                ]),
                fill: Fill::Solid(Color::rgb(255, 0, 0)),
            }],
        }
    }

    #[test]
    fn paints_background_and_polygon() {
        let img = render(&rect_doc()).expect("render");
        // pixel (0,0): background black
        let p = img.get_pixel(0, 0);
        assert_eq!(p.0[0..3], [0, 0, 0]);
        // pixel (20,15): inside the red rect
        let p = img.get_pixel(20, 15);
        assert!(p.0[0] > 250 && p.0[1] < 5 && p.0[2] < 5, "got {:?}", p.0);
    }
}
