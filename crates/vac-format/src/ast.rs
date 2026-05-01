//! AST types for the `.vac` source format.
//!
//! Designed to round-trip cleanly between the writer and parser.
//! v0.1 covers the "moving ball" subset: rect/ellipse primitives,
//! solid-color fill/stroke, and translate-only animation keyframes.

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub canvas: Canvas,
    pub scenes: Vec<Scene>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub name: String,
    /// Total scene duration, in milliseconds.
    pub duration_ms: u32,
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// `let <name> = <shape>`
    Let { name: String, shape: Shape },
    /// `<target>.<property> = <value>`
    Assign {
        target: String,
        property: Property,
        value: Value,
    },
    Animate(Animation),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Rect { x: f32, y: f32, w: f32, h: f32 },
    Ellipse { cx: f32, cy: f32, rx: f32, ry: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Property {
    Fill,
    Stroke,
}

impl Property {
    pub fn as_str(&self) -> &'static str {
        match self {
            Property::Fill => "fill",
            Property::Stroke => "stroke",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "fill" => Some(Property::Fill),
            "stroke" => Some(Property::Stroke),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Color(Color),
    /// Explicit "no value", e.g. `stroke = none`.
    None,
}

/// 32-bit RGBA color. Alpha defaults to 0xFF when written from RGB hex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 0xFF }
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color { r, g, b, a }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub target: String,
    pub keyframes: Vec<Keyframe>,
    pub easing: Easing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe {
    pub time_ms: u32,
    pub transforms: Vec<Transform>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transform {
    Position(f32, f32),
    Scale(f32),
    Opacity(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Easing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl Easing {
    pub fn as_str(&self) -> &'static str {
        match self {
            Easing::Linear => "linear",
            Easing::EaseIn => "ease-in",
            Easing::EaseOut => "ease-out",
            Easing::EaseInOut => "ease-in-out",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "linear" => Some(Easing::Linear),
            "ease-in" => Some(Easing::EaseIn),
            "ease-out" => Some(Easing::EaseOut),
            "ease-in-out" => Some(Easing::EaseInOut),
            _ => None,
        }
    }

    /// Maps a normalized progress `t` in `[0, 1]` through the easing curve.
    /// Standard CSS-equivalent cubic-bezier approximations.
    pub fn apply(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
        }
    }
}
