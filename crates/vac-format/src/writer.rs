//! AST → `.vac` text serializer.
//!
//! Format conventions (v0.1):
//! - 2-space indentation inside `scene` and `animate` blocks.
//! - Numbers: integers print bare (`100`), floats with up to 4 decimals,
//!   trailing zeros trimmed (`1.4`, not `1.4000`).
//! - Colors: `#rrggbb` when alpha is 0xFF, otherwise `#rrggbbaa`.
//! - Times: emitted in milliseconds with the `ms` suffix; multiples of
//!   1000 are emitted with the `s` suffix for readability (`3000ms` → `3s`).

use crate::ast::*;
use std::fmt::Write;

/// Serialize a [`Document`] to canonical `.vac` text.
///
/// v1.23: every picture an `image()` draws is written once, after the
/// scenes, as `asset picture_N = "data:…"`, and the shapes name it — the
/// scenes read as code, and a picture two scenes show is written once.
pub fn write(doc: &Document) -> String {
    let pictures = Pictures::of(doc);
    let mut out = String::new();
    write_canvas(&mut out, &doc.canvas);
    for scene in &doc.scenes {
        out.push('\n');
        write_scene(&mut out, scene, &pictures);
    }
    if !pictures.order.is_empty() {
        out.push('\n');
        let _ = writeln!(out, "// The pictures the scenes draw, each written once.");
        for (name, src) in &pictures.order {
            let _ = writeln!(out, "asset {name} = \"{src}\"");
        }
    }
    out
}

/// The distinct pictures a document's images draw, named `picture_1`,
/// `picture_2`, … in the order they are first drawn.
struct Pictures<'a> {
    names: std::collections::HashMap<&'a str, String>,
    order: Vec<(String, &'a str)>,
}

impl<'a> Pictures<'a> {
    fn of(doc: &'a Document) -> Self {
        let mut names = std::collections::HashMap::new();
        let mut order = Vec::new();
        for scene in &doc.scenes {
            for st in &scene.statements {
                if let Statement::Let { shape: Shape::Image { src, .. }, .. } = st {
                    if !names.contains_key(src.as_str()) {
                        let name = format!("picture_{}", order.len() + 1);
                        names.insert(src.as_str(), name.clone());
                        order.push((name, src.as_str()));
                    }
                }
            }
        }
        Pictures { names, order }
    }
}

fn write_canvas(out: &mut String, c: &Canvas) {
    let _ = writeln!(out, "canvas {}x{} @{}fps", c.width, c.height, c.fps);
}

fn write_scene(out: &mut String, s: &Scene, pictures: &Pictures) {
    let _ = writeln!(out, "scene {} {{", s.name);
    let _ = writeln!(out, "  duration {}", fmt_time(s.duration_ms));

    let mut prev: Option<&Statement> = None;
    for stmt in &s.statements {
        // Insert a blank line between groups of unrelated statements
        // for readability (between a `let` block and the next `let`,
        // or before/after an `animate` block).
        if needs_blank_line(prev, stmt) {
            out.push('\n');
        }
        write_statement(out, stmt, 1, pictures);
        prev = Some(stmt);
    }
    let _ = writeln!(out, "}}");
}

fn needs_blank_line(prev: Option<&Statement>, next: &Statement) -> bool {
    match (prev, next) {
        (None, _) => true,
        (Some(Statement::Assign { .. }), Statement::Let { .. }) => true,
        (Some(Statement::Assign { .. }), Statement::Animate(_)) => true,
        (Some(Statement::Let { .. }), Statement::Animate(_)) => true,
        (Some(Statement::Animate(_)), _) => true,
        _ => false,
    }
}

fn write_statement(out: &mut String, stmt: &Statement, indent: usize, pictures: &Pictures) {
    let pad = "  ".repeat(indent);
    match stmt {
        Statement::Let { name, shape } => {
            let _ = writeln!(out, "{pad}let {name} = {}", fmt_shape(shape, pictures));
        }
        Statement::Assign {
            target,
            property,
            value,
        } => {
            let _ = writeln!(
                out,
                "{pad}{target}.{} = {}",
                property.as_str(),
                fmt_value(value)
            );
        }
        Statement::Animate(a) => write_animation(out, a, indent),
    }
}

fn write_animation(out: &mut String, a: &Animation, indent: usize) {
    let pad = "  ".repeat(indent);
    let inner = "  ".repeat(indent + 1);
    let _ = writeln!(out, "{pad}animate {} {{", a.target);
    for kf in &a.keyframes {
        let transforms = kf
            .transforms
            .iter()
            .map(fmt_transform)
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "{inner}{} -> {}", fmt_time(kf.time_ms), transforms);
    }
    let _ = writeln!(out, "{inner}easing: {}", a.easing.as_str());
    let _ = writeln!(out, "{pad}}}");
}

fn fmt_shape(s: &Shape, pictures: &Pictures) -> String {
    match s {
        Shape::Rect { x, y, w, h } => {
            format!(
                "rect({}, {}, {}, {})",
                fmt_num(*x),
                fmt_num(*y),
                fmt_num(*w),
                fmt_num(*h)
            )
        }
        Shape::Ellipse { cx, cy, rx, ry } => {
            format!(
                "ellipse({}, {}, {}, {})",
                fmt_num(*cx),
                fmt_num(*cy),
                fmt_num(*rx),
                fmt_num(*ry)
            )
        }
        Shape::Image { x, y, w, h, src } => format!(
            "image({}, {}, {}, {}, {})",
            fmt_num(*x),
            fmt_num(*y),
            fmt_num(*w),
            fmt_num(*h),
            pictures.names.get(src.as_str()).cloned().unwrap_or_else(|| format!("\"{src}\""))
        ),
        Shape::Path(segs) => {
            let body = segs
                .iter()
                .map(fmt_path_seg)
                .collect::<Vec<_>>()
                .join(" ");
            format!("path({body})")
        }
    }
}

fn fmt_path_seg(seg: &PathSeg) -> String {
    match seg {
        PathSeg::MoveTo(p) => format!("M {},{}", fmt_num(p.x), fmt_num(p.y)),
        PathSeg::LineTo(p) => format!("L {},{}", fmt_num(p.x), fmt_num(p.y)),
        PathSeg::CurveTo { c1, c2, p } => format!(
            "C {},{} {},{} {},{}",
            fmt_num(c1.x),
            fmt_num(c1.y),
            fmt_num(c2.x),
            fmt_num(c2.y),
            fmt_num(p.x),
            fmt_num(p.y),
        ),
    }
}

fn fmt_value(v: &Value) -> String {
    match v {
        Value::Color(c) => fmt_color(c),
        Value::Gradient(Gradient::Linear { c0, p0, c1, p1 }) => format!(
            "gradient(linear, {} at {},{}, {} at {},{})",
            fmt_color(c0),
            fmt_num(p0.x),
            fmt_num(p0.y),
            fmt_color(c1),
            fmt_num(p1.x),
            fmt_num(p1.y)
        ),
        Value::Gradient(Gradient::Radial { c0, center, c1, radius }) => format!(
            "gradient(radial, {} at {},{}, {} radius {})",
            fmt_color(c0),
            fmt_num(center.x),
            fmt_num(center.y),
            fmt_color(c1),
            fmt_num(*radius)
        ),
        Value::Number(n) => fmt_num(*n),
        Value::None => "none".to_string(),
    }
}

fn fmt_transform(t: &Transform) -> String {
    match t {
        Transform::Position(x, y) => format!("position({}, {})", fmt_num(*x), fmt_num(*y)),
        Transform::Scale(sx, sy) if sx == sy => format!("scale({})", fmt_num(*sx)),
        Transform::Scale(sx, sy) => format!("scale({}, {})", fmt_num(*sx), fmt_num(*sy)),
        Transform::Opacity(o) => format!("opacity({})", fmt_num(*o)),
        Transform::Rotate(d) => format!("rotate({})", fmt_num(*d)),
        Transform::Fill(c) => format!("fill({})", fmt_color(c)),
        Transform::Clip(x, y, w, h) => format!("clip({}, {}, {}, {})", fmt_num(*x), fmt_num(*y), fmt_num(*w), fmt_num(*h)),
    }
}

fn fmt_color(c: &Color) -> String {
    if c.a == 0xFF {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
    }
}

fn fmt_time(ms: u32) -> String {
    if ms == 0 {
        return "0ms".to_string();
    }
    if ms % 1000 == 0 {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

fn fmt_num(n: f32) -> String {
    if n.is_finite() && (n.fract().abs() < 1e-6) {
        // Print integers without a decimal point.
        format!("{}", n.round() as i64)
    } else {
        // Up to 4 decimal places, trimmed.
        let s = format!("{:.4}", n);
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ball_doc() -> Document {
        Document {
            canvas: Canvas {
                width: 800,
                height: 600,
                fps: 30,
            },
            scenes: vec![Scene {
                name: "main".into(),
                duration_ms: 3000,
                statements: vec![
                    Statement::Let {
                        name: "bg".into(),
                        shape: Shape::Rect {
                            x: 0.0,
                            y: 0.0,
                            w: 800.0,
                            h: 600.0,
                        },
                    },
                    Statement::Assign {
                        target: "bg".into(),
                        property: Property::Fill,
                        value: Value::Color(Color::rgb(0x1a, 0x1a, 0x2e)),
                    },
                    Statement::Let {
                        name: "ball".into(),
                        shape: Shape::Ellipse {
                            cx: 100.0,
                            cy: 300.0,
                            rx: 30.0,
                            ry: 30.0,
                        },
                    },
                    Statement::Assign {
                        target: "ball".into(),
                        property: Property::Fill,
                        value: Value::Color(Color::rgb(0xe9, 0x45, 0x60)),
                    },
                    Statement::Assign {
                        target: "ball".into(),
                        property: Property::Stroke,
                        value: Value::None,
                    },
                    Statement::Animate(Animation {
                        target: "ball".into(),
                        keyframes: vec![
                            Keyframe {
                                time_ms: 0,
                                transforms: vec![Transform::Position(100.0, 300.0)],
                            },
                            Keyframe {
                                time_ms: 1500,
                                transforms: vec![Transform::Position(700.0, 300.0)],
                            },
                            Keyframe {
                                time_ms: 3000,
                                transforms: vec![Transform::Position(100.0, 300.0)],
                            },
                        ],
                        easing: Easing::Linear,
                    }),
                ],
            }],
        }
    }

    #[test]
    fn writes_canvas_header() {
        let doc = ball_doc();
        let s = write(&doc);
        assert!(s.starts_with("canvas 800x600 @30fps\n"));
    }

    #[test]
    fn writes_durations_in_seconds_when_round() {
        let doc = ball_doc();
        let s = write(&doc);
        assert!(s.contains("duration 3s"), "got:\n{s}");
    }

    #[test]
    fn writes_hex_colors() {
        let doc = ball_doc();
        let s = write(&doc);
        assert!(s.contains("#1a1a2e"), "got:\n{s}");
        assert!(s.contains("#e94560"), "got:\n{s}");
    }

    /// v1.21: `scale(s)` is uniform and still written that way; a
    /// stretch, `scale(sx, sy)`, keeps both axes.
    #[test]
    fn scale_round_trips_uniform_and_stretched() {
        let src = "canvas 10x10 @10fps\nscene s {\n  duration 1s\n  let r = rect(0, 0, 4, 2)\n  animate r {\n    0ms -> scale(1.5)\n    1s -> scale(0.25, 1)\n    easing: linear\n  }\n}\n";
        let doc = crate::parse(src).unwrap();
        let kfs = match &doc.scenes[0].statements[1] {
            Statement::Animate(a) => &a.keyframes,
            other => panic!("expected the animate block, got {other:?}"),
        };
        assert_eq!(kfs[0].transforms, vec![Transform::Scale(1.5, 1.5)]);
        assert_eq!(kfs[1].transforms, vec![Transform::Scale(0.25, 1.0)]);
        let out = write(&doc);
        assert!(out.contains("0ms -> scale(1.5)\n"), "{out}");
        assert!(out.contains("1s -> scale(0.25, 1)"), "{out}");
        assert_eq!(crate::parse(&out).unwrap(), doc);
        assert!(crate::parse(&src.replace("scale(1.5)", "scale(1, 2, 3)")).is_err());
    }

    /// v1.22: `clip(x, y, w, h)` round-trips, and a negative size is
    /// refused.
    #[test]
    fn clip_round_trips() {
        let src = "canvas 10x10 @10fps\nscene s {\n  duration 1s\n  let r = rect(0, 0, 4, 2)\n  animate r {\n    0ms -> clip(10, 0, 0, 10)\n    1s -> clip(0, 0, 10, 10)\n    easing: linear\n  }\n}\n";
        let doc = crate::parse(src).unwrap();
        let out = write(&doc);
        assert!(out.contains("0ms -> clip(10, 0, 0, 10)"), "{out}");
        assert!(out.contains("1s -> clip(0, 0, 10, 10)"), "{out}");
        assert_eq!(crate::parse(&out).unwrap(), doc);
        assert!(crate::parse(&src.replace("clip(10, 0, 0, 10)", "clip(10, 0, -1, 10)")).is_err());
        assert!(crate::parse(&src.replace("clip(10, 0, 0, 10)", "clip(10, 0, 0)")).is_err());
    }

    #[test]
    fn rotate_round_trips() {
        let src = "canvas 10x10 @10fps\nscene s {\n  duration 1s\n  let r = rect(0, 0, 4, 2)\n  animate r {\n    0ms -> rotate(-12.5), position(1, 2)\n    1s -> rotate(90)\n    easing: linear\n  }\n}\n";
        let doc = crate::parse(src).unwrap();
        let out = write(&doc);
        assert!(out.contains("rotate(-12.5), position(1, 2)"), "{out}");
        assert!(out.contains("rotate(90)"), "{out}");
        assert_eq!(crate::parse(&out).unwrap(), doc);
    }

    #[test]
    fn image_round_trips() {
        let src = "canvas 10x10 @10fps\nscene s {\n  duration 1s\n  let p = image(0, 0, 10, 10, \"data:image/png;base64,Zm9v\")\n}\n";
        let doc = crate::parse(src).unwrap();
        let out = write(&doc);
        assert!(out.contains("image(0, 0, 10, 10, picture_1)"), "{out}");
        assert!(out.contains("asset picture_1 = \"data:image/png;base64,Zm9v\""), "{out}");
        assert_eq!(crate::parse(&out).unwrap(), doc);
    }

    /// v1.23: a picture two scenes draw is written once, after the scenes,
    /// and named from both; an asset may be declared before or after its
    /// use; an unknown or twice-declared name is refused.
    #[test]
    fn a_picture_is_written_once_and_named() {
        let src = "canvas 10x10 @10fps\nscene a {\n  duration 1s\n  let p = image(0, 0, 10, 10, \"data:image/png;base64,Zm9v\")\n}\nscene b {\n  duration 1s\n  let q = image(0, 0, 10, 10, \"data:image/png;base64,Zm9v\")\n  let r = image(2, 2, 4, 4, \"data:image/png;base64,YmFy\")\n}\n";
        let doc = crate::parse(src).unwrap();
        let out = write(&doc);
        assert_eq!(out.matches("Zm9v").count(), 1, "{out}");
        assert_eq!(out.matches("picture_1").count(), 3, "two uses and the asset: {out}");
        assert!(out.contains("let r = image(2, 2, 4, 4, picture_2)"), "{out}");
        assert!(out.find("asset picture_1").unwrap() > out.find("scene b").unwrap(), "assets after the scenes: {out}");
        assert_eq!(crate::parse(&out).unwrap(), doc);
        let before = "canvas 10x10 @10fps\nasset photo = \"data:image/png;base64,Zm9v\"\nscene a {\n  duration 1s\n  let p = image(0, 0, 10, 10, photo)\n}\n";
        let parsed = crate::parse(before).unwrap();
        assert!(matches!(&parsed.scenes[0].statements[0], Statement::Let { shape: Shape::Image { src, .. }, .. } if src == "data:image/png;base64,Zm9v"));
        assert!(crate::parse(&before.replace("image(0, 0, 10, 10, photo)", "image(0, 0, 10, 10, missing)")).is_err());
        assert!(crate::parse(&before.replace("scene a", "asset photo = \"data:x\"\nscene a")).is_err());
    }
}
