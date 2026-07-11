//! AST for the `.pac` (Picture As Code) source format.
//!
//! Re-exports [`vac_format::Color`] so the two formats share a single
//! colour type and there is no impedance mismatch if a future tool
//! wants to convert one to the other.

pub use vac_format::Color;

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub canvas: Canvas,
    /// Optional background fill rasterised before any shape.  When `None`
    /// the canvas starts transparent.
    pub background: Option<Color>,
    /// Shapes drawn back-to-front in declaration order.
    pub shapes: Vec<ShapeDecl>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShapeDecl {
    pub name: String,
    pub shape: Shape,
    pub fill: Fill,
}

/// What's painted *inside* the shape's outline.  v0.2 adds linear
/// gradients on top of the v0.1 solid-colour fill.
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    /// Single flat colour.  Equivalent to v0.1's `name.fill = #rrggbb`.
    Solid(Color),
    /// Linear gradient: colour interpolates from `c0` at `p0` to `c1`
    /// at `p1`, perpendicular bands fill the rest of the shape.
    /// Mirrors SVG's `<linearGradient>` two-stop form.
    LinearGradient {
        c0: Color,
        c1: Color,
        p0: Point,
        p1: Point,
    },
}

impl From<Color> for Fill {
    fn from(c: Color) -> Self { Fill::Solid(c) }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// Closed polygon: at least 3 points; an implicit edge connects
    /// the last vertex back to the first.
    Polygon(Vec<Point>),
    /// Closed path of cubic Bezier segments.  The path is implicitly
    /// closed: the renderer connects the final point back to the
    /// first via a straight line (use `CurveTo` for the final
    /// segment if you want it curved).  All segments share the same
    /// implicit "current point" rule as SVG paths.
    Path(Vec<PathSeg>),
}

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
