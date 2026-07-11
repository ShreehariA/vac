//! PAC source format: AST, text writer, and parser.
//!
//! PAC is the still-image sibling of VAC.  Where a `.vac` document
//! describes a video as a sequence of animated shapes over time,
//! a `.pac` document describes a still image as a flat, ordered list
//! of shapes drawn back-to-front.
//!
//! Round-trip invariant:  `parse(write(doc)) == doc` for any [`Document`]
//! the writer can emit.
//!
//! ## Grammar (v0.1, "polygon-only")
//!
//! ```text
//! canvas WIDTHxHEIGHT
//! background #rrggbb       # optional; defaults to transparent
//!
//! let NAME = polygon(x1 y1, x2 y2, ...)   # implicitly closed
//! NAME.fill = #rrggbb                     # required
//! ```
//!
//! Declaration order = draw order: the first `let` is the bottom layer,
//! the last `let` is the top layer.  This matches SVG's painter's
//! algorithm and means the decompiler can emit "biggest region first".
//!
//! Grammar grows in later versions:
//!   v0.2 — `ellipse(cx, cy, rx, ry)` and `rect(x, y, w, h)` shortcuts
//!          for trivially-round / trivially-rectangular regions.
//!   v0.3 — `path(M x,y L x,y ... Z)` SVG-style mini-path syntax.
//!          Bezier control points so smooth curves stop being
//!          piecewise-linear staircases.
//!   v0.4 — `raster(x, y, w, h, "tile.webp")` for regions VAC's
//!          philosophy proves cannot be vector-decomposed honestly.
//!          The escape hatch is always *scoped*; PAC will never embed
//!          a single raster covering the whole canvas.

pub mod ast;
pub mod parser;
pub mod writer;

pub use ast::*;
pub use parser::{parse, ParseError};
pub use writer::write;
