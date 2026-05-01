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
pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    write_canvas(&mut out, &doc.canvas);
    for scene in &doc.scenes {
        out.push('\n');
        write_scene(&mut out, scene);
    }
    out
}

fn write_canvas(out: &mut String, c: &Canvas) {
    let _ = writeln!(out, "canvas {}x{} @{}fps", c.width, c.height, c.fps);
}

fn write_scene(out: &mut String, s: &Scene) {
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
        write_statement(out, stmt, 1);
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

fn write_statement(out: &mut String, stmt: &Statement, indent: usize) {
    let pad = "  ".repeat(indent);
    match stmt {
        Statement::Let { name, shape } => {
            let _ = writeln!(out, "{pad}let {name} = {}", fmt_shape(shape));
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

fn fmt_shape(s: &Shape) -> String {
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
    }
}

fn fmt_value(v: &Value) -> String {
    match v {
        Value::Color(c) => fmt_color(c),
        Value::None => "none".to_string(),
    }
}

fn fmt_transform(t: &Transform) -> String {
    match t {
        Transform::Position(x, y) => format!("position({}, {})", fmt_num(*x), fmt_num(*y)),
        Transform::Scale(s) => format!("scale({})", fmt_num(*s)),
        Transform::Opacity(o) => format!("opacity({})", fmt_num(*o)),
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
}
