//! Top-level decompile pipeline.  Stitches every other module together.

use pac_format::{Canvas, Document, ShapeDecl, Shape};
use thiserror::Error;

use crate::contour::trace_outer_contour;
use crate::curves::fit_closed;
use crate::gradient::detect_linear_gradient;
use crate::image_io::{load_rgba, RgbaImage};
use crate::kmeans::{quantize, quantize_auto, KMeansResult};
use crate::regions::{label_regions_per_cluster_full, LabelledRegion, RegionsResult};
use crate::simplify::rdp;
use pac_format::Fill;

#[derive(Debug, Error)]
pub enum DecompileError {
    #[error("image load: {0}")]
    Io(#[from] std::io::Error),
    #[error("image decode: {0}")]
    Decode(#[from] image::ImageError),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone, Copy)]
pub struct DecompileOptions {
    /// v0.2: floor for adaptive palette sizing.  Auto-K starts here
    /// and grows until residual drops below `target_residual` or it
    /// hits `palette_size_max`.  Set `palette_size_min == palette_size_max`
    /// to lock K.
    pub palette_size_min: usize,
    /// Ceiling for adaptive palette sizing.
    pub palette_size_max: usize,
    /// Per-channel mean squared error target for auto-K.  Smaller =
    /// finer palette (more shapes), larger = coarser.  Values of 50..200
    /// give visually reasonable defaults; 100 ≈ "within 10 RGB per
    /// channel on average".
    pub target_residual: f32,
    /// Regions with fewer pixels than this are dropped as noise.
    pub min_region_area: u32,
    /// RDP simplification tolerance in pixels.  Higher = fewer
    /// vertices per polygon, blockier silhouettes.
    pub rdp_epsilon: f32,
    /// Cap the longest input edge before quantizing.  Faster, and the
    /// loss is rarely visible because we re-rasterise through
    /// polygons anyway.  0 = no downscale.
    pub max_edge: u32,
    /// Maximum K-means iterations (10..30 is plenty for image colour
    /// histograms, which converge fast).
    pub kmeans_max_iters: usize,
    /// v0.2: emit `path(M..C..C..)` cubic-Bezier-fitted shapes
    /// instead of raw `polygon(...)` line segments.  Same vertex
    /// data, smoother visible curves.
    pub smooth_curves: bool,
    /// Schneider curve-fitting error tolerance in pixels.  Lower =
    /// more Bezier segments per region, closer fit; higher = fewer
    /// segments, softer approximation.
    pub curve_eps: f32,
    /// v0.2: attempt linear-gradient detection per region.  Falls
    /// back to solid fill if the gradient model isn't a clear win.
    pub detect_gradients: bool,
    /// Minimum SSE improvement (1.0 = perfect, 0.0 = no improvement)
    /// for a gradient to be preferred over the solid mean fill.
    pub gradient_improvement: f32,
    /// Minimum per-channel colour delta along the gradient axis.
    /// Drops "flat" gradients (visible noise) and keeps emit honest.
    pub gradient_min_delta: f32,
}

impl Default for DecompileOptions {
    fn default() -> Self {
        DecompileOptions {
            palette_size_min: 6,
            palette_size_max: 24,
            target_residual: 80.0,
            min_region_area: 400,
            rdp_epsilon: 2.0,
            max_edge: 768,
            kmeans_max_iters: 16,
            smooth_curves: true,
            curve_eps: 1.5,
            detect_gradients: true,
            gradient_improvement: 0.30,
            gradient_min_delta: 20.0,
        }
    }
}

/// Metrics emitted by the decompiler — useful for the CLI to print
/// a one-line "what got recovered" summary and for tooling that
/// wants to know how confident the output is.
#[derive(Debug, Clone)]
pub struct DecompileStats {
    pub palette_size: usize,
    pub residual: f32,
    pub region_count: usize,
    pub gradient_count: usize,
    /// Per-region residual energy (mean squared error of the fitted
    /// fill — gradient OR solid — against the source pixels).  Index
    /// matches the order of `Document::shapes` so the CLI can name
    /// the worst-fit shapes by their PAC identifiers.
    pub region_residuals: Vec<f32>,
}

/// Build a semantic-ish identifier from a region's centroid and colour.
/// Goal: editable, not unique — uniqueness is handled by the caller
/// appending a counter when the same name appears more than once.
///
/// Format: `{vertical}_{horizontal}_{colour_name}` (e.g.
/// `top_left_blue`, `center_skin`, `bottom_dark`).  Colour names
/// come from a small 14-bucket HSL palette good enough for editing
/// hand-off; users can rename anything they like.
fn semantic_name(r: &LabelledRegion, fill: &pac_format::Color, w: u32, h: u32) -> String {
    let cx = (r.min_x + r.max_x) as f32 / 2.0;
    let cy = (r.min_y + r.max_y) as f32 / 2.0;
    let wf = w as f32;
    let hf = h as f32;

    let v = if cy < hf / 3.0 { "top" }
        else if cy < 2.0 * hf / 3.0 { "mid" }
        else { "bottom" };
    let hh = if cx < wf / 3.0 { "left" }
        else if cx < 2.0 * wf / 3.0 { "center" }
        else { "right" };

    let colour = colour_bucket(fill);
    // Drop the "mid_center" prefix on solidly-central regions —
    // shorter names read better in the source.
    if v == "mid" && hh == "center" {
        colour.to_string()
    } else {
        format!("{v}_{hh}_{colour}")
    }
}

fn colour_bucket(c: &pac_format::Color) -> &'static str {
    let r = c.r as i32;
    let g = c.g as i32;
    let b = c.b as i32;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lum = (max + min) / 2;
    let sat = if max == 0 { 0 } else { 255 * (max - min) / max };

    if lum < 40 { return "black"; }
    if lum > 215 && sat < 30 { return "white"; }
    if sat < 35 {
        return if lum < 110 { "dark_gray" }
            else if lum > 175 { "light_gray" }
            else { "gray" };
    }
    // Hue bucket — 6 way split.  We compute it from RGB directly
    // (cheap; we don't care about perceptual uniformity here).
    let hue6 = if r >= g && r >= b {
        if g >= b { 0 } else { 5 } // red-orange or red-magenta
    } else if g >= r && g >= b {
        if r >= b { 1 } else { 2 } // yellow/green vs green/cyan
    } else {
        if r >= g { 4 } else { 3 } // blue/magenta vs cyan/blue
    };
    match (hue6, lum > 170, lum < 90) {
        (0, true, _)  => "salmon",
        (0, _, true)  => "maroon",
        (0, _, _)     => "red",
        (1, true, _)  => "olive",
        (1, _, _)     => "yellow",
        (2, true, _)  => "mint",
        (2, _, _)     => "green",
        (3, true, _)  => "sky",
        (3, _, _)     => "teal",
        (4, true, _)  => "lavender",
        (4, _, true)  => "navy",
        (4, _, _)     => "blue",
        (5, true, _)  => "pink",
        (5, _, _)     => "purple",
        _             => "color",
    }
}

pub fn decompile(path: &str, opts: &DecompileOptions) -> Result<Document, DecompileError> {
    let img = load_rgba(path, opts.max_edge)?;
    decompile_image(&img, opts)
}

pub fn decompile_image(img: &RgbaImage, opts: &DecompileOptions) -> Result<Document, DecompileError> {
    let (doc, _stats) = decompile_image_with_stats(img, opts)?;
    Ok(doc)
}

pub fn decompile_image_with_stats(
    img: &RgbaImage,
    opts: &DecompileOptions,
) -> Result<(Document, DecompileStats), DecompileError> {
    // v0.2: auto-K — grow palette until the per-pixel residual drops
    // below `target_residual` or we hit `palette_size_max`.  Locking
    // K is just `min == max`.
    let (chosen_k, res) = if opts.palette_size_min >= opts.palette_size_max {
        let k = opts.palette_size_max.max(opts.palette_size_min);
        (k, quantize(img, k, opts.kmeans_max_iters))
    } else {
        quantize_auto(
            img,
            opts.palette_size_min,
            opts.palette_size_max,
            opts.target_residual,
            opts.kmeans_max_iters,
        )
    };
    let KMeansResult { palette, labels } = res;
    let residual = crate::kmeans::mean_squared_residual(img, &KMeansResult {
        palette: palette.clone(),
        labels: labels.clone(),
    });

    let RegionsResult { regions, region_labels } =
        label_regions_per_cluster_full(&labels, img.width, img.height, opts.min_region_area);

    // Pick the background cluster as the one with the largest *total*
    // pixel count.  That's the canvas-fill colour we write as
    // `background ...`, and we skip emitting a shape for it (it'd
    // cover everything else).
    let mut cluster_pixel_counts = vec![0u32; palette.len()];
    for &l in &labels {
        cluster_pixel_counts[l as usize] += 1;
    }
    let bg_cluster = cluster_pixel_counts
        .iter()
        .enumerate()
        .max_by_key(|(_, c)| *c)
        .map(|(i, _)| i as u32)
        .unwrap_or(0);
    let background = Some(palette[bg_cluster as usize]);

    // Build shape declarations: largest regions first so they live at
    // the bottom of the paint stack and smaller details land on top.
    let mut regs: Vec<LabelledRegion> = regions
        .into_iter()
        .filter(|r| r.cluster != bg_cluster)
        .collect();
    regs.sort_by_key(|r| std::cmp::Reverse(r.area));

    let mut shapes: Vec<ShapeDecl> = Vec::with_capacity(regs.len());
    let mut name_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut gradient_count: usize = 0;
    let mut region_residuals: Vec<f32> = Vec::with_capacity(regs.len());
    for r in regs.iter() {
        // v0.2 correctness: trace by *region* label (unique per blob)
        // rather than cluster id, so two disjoint regions of the same
        // colour can never bleed into one contour.
        let Some(contour) = trace_outer_contour(&region_labels, img.width, img.height, r) else {
            continue;
        };
        let simplified = rdp(&contour, opts.rdp_epsilon);
        if simplified.len() < 3 {
            continue;
        }
        let shape = if opts.smooth_curves {
            // v0.2: fit cubic Beziers to the simplified outline.
            // Falls back to polygon if the path degenerates.
            let segs = fit_closed(&simplified, opts.curve_eps);
            if segs.len() >= 2 {
                Shape::Path(segs)
            } else {
                Shape::Polygon(
                    simplified.into_iter()
                        .map(|(x, y)| pac_format::Point::new(x, y))
                        .collect()
                )
            }
        } else {
            Shape::Polygon(
                simplified.into_iter()
                    .map(|(x, y)| pac_format::Point::new(x, y))
                    .collect()
            )
        };

        let palette_color = palette[r.cluster as usize];
        let fill = if opts.detect_gradients {
            match detect_linear_gradient(
                img,
                &region_labels,
                r,
                opts.gradient_improvement,
                opts.gradient_min_delta,
            ) {
                Some(g) => {
                    gradient_count += 1;
                    Fill::LinearGradient { c0: g.c0, c1: g.c1, p0: g.p0, p1: g.p1 }
                }
                None => Fill::Solid(palette_color),
            }
        } else {
            Fill::Solid(palette_color)
        };

        // Naming uses the dominant colour for solid fills, or the
        // midpoint of a gradient.
        let name_color = match &fill {
            Fill::Solid(c) => *c,
            Fill::LinearGradient { c0, c1, .. } => pac_format::Color::rgb(
                ((c0.r as u16 + c1.r as u16) / 2) as u8,
                ((c0.g as u16 + c1.g as u16) / 2) as u8,
                ((c0.b as u16 + c1.b as u16) / 2) as u8,
            ),
        };
        let base = semantic_name(r, &name_color, img.width, img.height);
        let counter = name_counts.entry(base.clone()).or_insert(0);
        let name = if *counter == 0 { base.clone() } else { format!("{base}_{counter}") };
        *counter += 1;
        region_residuals.push(per_region_residual(img, &region_labels, r, &fill));
        shapes.push(ShapeDecl { name, shape, fill });
    }

    let doc = Document {
        canvas: Canvas { width: img.width, height: img.height },
        background,
        shapes,
    };
    let stats = DecompileStats {
        palette_size: chosen_k,
        residual,
        region_count: doc.shapes.len(),
        gradient_count,
        region_residuals,
    };
    Ok((doc, stats))
}

/// Mean squared per-channel error between the chosen fill and the
/// region's actual source pixels.  Useful for diagnostics and as the
/// trigger heuristic for a future `raster(...)` escape-hatch primitive
/// (v0.3).
fn per_region_residual(
    img: &RgbaImage,
    region_labels: &[u32],
    r: &LabelledRegion,
    fill: &Fill,
) -> f32 {
    let w = img.width as usize;
    let target = r.label;
    let mut sse: f64 = 0.0;
    let mut n: u64 = 0;
    for y in (r.min_y as i32)..=(r.max_y as i32) {
        for x in (r.min_x as i32)..=(r.max_x as i32) {
            let idx = (y as usize) * w + x as usize;
            if region_labels[idx] != target { continue; }
            let p = &img.pixels[idx * 4..idx * 4 + 3];

            let (fr, fg, fb) = match fill {
                Fill::Solid(c) => (c.r as f64, c.g as f64, c.b as f64),
                Fill::LinearGradient { c0, c1, p0, p1 } => {
                    // Linear interpolation along the gradient line.
                    let dx = p1.x - p0.x;
                    let dy = p1.y - p0.y;
                    let len2 = dx * dx + dy * dy;
                    if len2 < 1e-6 {
                        (c0.r as f64, c0.g as f64, c0.b as f64)
                    } else {
                        let t = (((x as f32 - p0.x) * dx + (y as f32 - p0.y) * dy) / len2).clamp(0.0, 1.0);
                        let lerp = |a: u8, b: u8| (a as f64) * (1.0 - t as f64) + (b as f64) * (t as f64);
                        (lerp(c0.r, c1.r), lerp(c0.g, c1.g), lerp(c0.b, c1.b))
                    }
                }
            };

            let dr = p[0] as f64 - fr;
            let dg = p[1] as f64 - fg;
            let db = p[2] as f64 - fb;
            sse += dr * dr + dg * dg + db * db;
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { (sse / (3.0 * n as f64)) as f32 }
}
