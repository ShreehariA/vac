//! PAC decompiler: PNG/JPG/WebP → `.pac` AST.
//!
//! Pipeline (each stage deterministic, no AI):
//!   1. Load + decode to RGBA8.
//!   2. K-means over the pixel histogram → K palette colours.
//!   3. Per-pixel: nearest palette index → labelled image.
//!   4. Per cluster: 4-connected component labelling → regions.
//!   5. Drop regions below MIN_REGION_AREA (small noise).
//!   6. For each region: Moore-neighbour contour trace → polygon.
//!   7. Ramer-Douglas-Peucker simplify each polygon.
//!   8. Sort regions descending by area and emit shapes back-to-front.

pub mod kmeans;
pub mod regions;
pub mod contour;
pub mod simplify;
pub mod curves;
pub mod gradient;
pub mod decompile;
pub mod image_io;

pub use decompile::{
    decompile, decompile_image, decompile_image_with_stats, DecompileError, DecompileOptions,
    DecompileStats,
};
