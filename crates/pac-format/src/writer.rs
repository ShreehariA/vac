//! AST → `.pac` text serializer.
//!
//! Layout conventions:
//! - One blank line between successive shapes.
//! - Points written `x y` (space, no comma) and grouped into the
//!   parenthesised polygon list with `, ` between successive points.
//! - Integers print bare; non-integer floats with up to 2 decimals,
//!   trailing zeros trimmed.  PAC coordinates come straight out of
//!   pixel-grid contour tracing so 2 decimals is plenty.
//! - Colours: `#rrggbb` when alpha is 0xFF, otherwise `#rrggbbaa`.

use crate::ast::*;
use std::fmt::Write;

pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "canvas {}x{}", doc.canvas.width, doc.canvas.height);
    if let Some(bg) = doc.background {
        let _ = writeln!(out, "background {}", fmt_color(&bg));
    }
    for shape in &doc.shapes {
        out.push('\n');
        write_shape(&mut out, shape);
    }
    out
}

fn write_shape(out: &mut String, decl: &ShapeDecl) {
    match &decl.shape {
        Shape::Polygon(pts) => {
            let body = pts
                .iter()
                .map(|p| format!("{} {}", fmt_num(p.x), fmt_num(p.y)))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "let {} = polygon({body})", decl.name);
            let _ = writeln!(out, "{}.fill = {}", decl.name, fmt_fill(&decl.fill));
        }
        Shape::Path(segs) => {
            let body = segs
                .iter()
                .map(fmt_path_seg)
                .collect::<Vec<_>>()
                .join(" ");
            let _ = writeln!(out, "let {} = path({body})", decl.name);
            let _ = writeln!(out, "{}.fill = {}", decl.name, fmt_fill(&decl.fill));
        }
    }
}

fn fmt_fill(fill: &Fill) -> String {
    match fill {
        Fill::Solid(c) => fmt_color(c),
        Fill::LinearGradient { c0, c1, p0, p1 } => format!(
            "gradient(linear, {} at {},{}, {} at {},{})",
            fmt_color(c0), fmt_num(p0.x), fmt_num(p0.y),
            fmt_color(c1), fmt_num(p1.x), fmt_num(p1.y),
        ),
    }
}

fn fmt_path_seg(seg: &PathSeg) -> String {
    match seg {
        PathSeg::MoveTo(p) => format!("M {},{}", fmt_num(p.x), fmt_num(p.y)),
        PathSeg::LineTo(p) => format!("L {},{}", fmt_num(p.x), fmt_num(p.y)),
        PathSeg::CurveTo { c1, c2, p } => format!(
            "C {},{} {},{} {},{}",
            fmt_num(c1.x), fmt_num(c1.y),
            fmt_num(c2.x), fmt_num(c2.y),
            fmt_num(p.x),  fmt_num(p.y),
        ),
    }
}

fn fmt_color(c: &Color) -> String {
    if c.a == 0xFF {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
    }
}

fn fmt_num(n: f32) -> String {
    if n.is_finite() && n.fract().abs() < 1e-3 {
        format!("{}", n.round() as i64)
    } else {
        let s = format!("{:.2}", n);
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_doc() -> Document {
        Document {
            canvas: Canvas { width: 200, height: 100 },
            background: Some(Color::rgb(0x1a, 0x1a, 0x2e)),
            shapes: vec![
                ShapeDecl {
                    name: "region_0".into(),
                    shape: Shape::Polygon(vec![
                        Point::new(10.0, 10.0),
                        Point::new(50.0, 10.0),
                        Point::new(50.0, 40.0),
                        Point::new(10.0, 40.0),
                    ]),
                    fill: Fill::Solid(Color::rgb(0xff, 0x00, 0x00)),
                },
            ],
        }
    }

    #[test]
    fn writes_canvas_header() {
        let s = write(&sample_doc());
        assert!(s.starts_with("canvas 200x100\n"), "got:\n{s}");
    }

    #[test]
    fn writes_background_line() {
        let s = write(&sample_doc());
        assert!(s.contains("background #1a1a2e"), "got:\n{s}");
    }

    #[test]
    fn writes_polygon_with_inline_points() {
        let s = write(&sample_doc());
        assert!(
            s.contains("let region_0 = polygon(10 10, 50 10, 50 40, 10 40)"),
            "got:\n{s}"
        );
        assert!(s.contains("region_0.fill = #ff0000"), "got:\n{s}");
    }
}
