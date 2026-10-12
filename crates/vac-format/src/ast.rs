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

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Rect { x: f32, y: f32, w: f32, h: f32 },
    Ellipse { cx: f32, cy: f32, rx: f32, ry: f32 },
    /// Closed path of straight-line and cubic-Bezier segments (v0.6).
    /// The path is implicitly closed: the renderer connects the final
    /// point back to the first `MoveTo`.  Shared with the `.pac`
    /// format, whose decompiler pipeline (Moore contour tracing → RDP
    /// → Schneider Bezier fitting) produces these segments.
    Path(Vec<PathSeg>),
}

/// Point in canvas pixels.  Shared by `.vac` paths and the `.pac`
/// still-image format (which re-exports it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathSeg {
    /// Move pen to (x, y).  Must be the first segment of every path.
    /// Mirrors SVG's `M x,y`.
    MoveTo(Point),
    /// Straight line from current point to (x, y).  Mirrors SVG's `L x,y`.
    LineTo(Point),
    /// Cubic Bezier from current point through control points c1 and
    /// c2 to endpoint p.  Mirrors SVG's `C cx1,cy1 cx2,cy2 x,y`.
    CurveTo { c1: Point, c2: Point, p: Point },
}

/// Anchor point of a closed path: the area (shoelace) centroid of the
/// flattened outline.  This is what `position(x, y)` keyframes move —
/// for decompiled paths it coincides with the measured blob centroid
/// to within a pixel, so declaration and animation agree.  Degenerate
/// (near-zero-area) paths fall back to the vertex mean.
pub fn path_anchor(segs: &[PathSeg]) -> (f32, f32) {
    let pts = flatten_path(segs);
    if pts.is_empty() {
        return (0.0, 0.0);
    }
    let n = pts.len();
    let mut area2 = 0.0f64;
    let mut cx = 0.0f64;
    let mut cy = 0.0f64;
    for i in 0..n {
        let (x0, y0) = (pts[i].0 as f64, pts[i].1 as f64);
        let (x1, y1) = (pts[(i + 1) % n].0 as f64, pts[(i + 1) % n].1 as f64);
        let cross = x0 * y1 - x1 * y0;
        area2 += cross;
        cx += (x0 + x1) * cross;
        cy += (y0 + y1) * cross;
    }
    if area2.abs() < 1e-6 {
        let sx: f32 = pts.iter().map(|p| p.0).sum();
        let sy: f32 = pts.iter().map(|p| p.1).sum();
        return (sx / n as f32, sy / n as f32);
    }
    ((cx / (3.0 * area2)) as f32, (cy / (3.0 * area2)) as f32)
}

/// Flatten a path to a polygon: on-curve points, with each cubic
/// sampled at a fixed number of interior steps.
fn flatten_path(segs: &[PathSeg]) -> Vec<(f32, f32)> {
    const CURVE_STEPS: usize = 8;
    let mut out: Vec<(f32, f32)> = Vec::new();
    let mut cur = (0.0f32, 0.0f32);
    for seg in segs {
        match *seg {
            PathSeg::MoveTo(p) | PathSeg::LineTo(p) => {
                out.push((p.x, p.y));
                cur = (p.x, p.y);
            }
            PathSeg::CurveTo { c1, c2, p } => {
                for k in 1..=CURVE_STEPS {
                    let t = k as f32 / CURVE_STEPS as f32;
                    let v = 1.0 - t;
                    let b0 = v * v * v;
                    let b1 = 3.0 * v * v * t;
                    let b2 = 3.0 * v * t * t;
                    let b3 = t * t * t;
                    out.push((
                        b0 * cur.0 + b1 * c1.x + b2 * c2.x + b3 * p.x,
                        b0 * cur.1 + b1 * c1.y + b2 * c2.y + b3 * p.y,
                    ));
                }
                cur = (p.x, p.y);
            }
        }
    }
    out
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
    /// A two-stop gradient fill (v1.2), in the shape's declared
    /// coordinates — it moves, scales and turns with the shape.
    Gradient(Gradient),
    /// Explicit "no value", e.g. `stroke = none`.
    None,
}

/// Two-stop gradients. The linear form is PAC's, so the two siblings
/// read the same:
///
/// ```text
/// gradient(linear, #c0 at x0,y0, #c1 at x1,y1)
/// gradient(radial, #c0 at cx,cy, #c1 radius r)
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gradient {
    /// `c0` at `p0` blending to `c1` at `p1`, padded beyond both.
    Linear { c0: Color, p0: Point, c1: Color, p1: Point },
    /// `c0` at `center` blending to `c1` at distance `radius`, padded
    /// beyond it.
    Radial { c0: Color, center: Point, c1: Color, radius: f32 },
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
    /// Clockwise rotation in degrees (v0.14), about the shape's centre —
    /// a rect's or ellipse's centre, a path's centroid — independent of
    /// the `position` anchor, the way CSS `transform-origin: center` is.
    Rotate(f32),
    /// A flat fill colour (v1.3), interpolated in RGB between keyframes
    /// that carry one — colour animation.
    Fill(Color),
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
