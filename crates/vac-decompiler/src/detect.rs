//! Per-frame multi-blob detection.
//!
//! All deterministic, all measurement: sample → mask → connected
//! components → for each component, centroid + bbox + interior-only
//! mean colour + Hu image-moment invariants. No models, no training.
//!
//! v0.3 generalises v0.2's "find the single largest blob" to "find
//! every blob above the minimum-area threshold," so the cross-frame
//! tracker in `track.rs` can stitch tracks for an arbitrary number
//! of objects.

use vac_format::Color;

use crate::frames::VideoFrame;

#[derive(Debug, Clone)]
pub struct Detection {
    pub frame_index: usize,
    /// Time offset of this frame in milliseconds (cumulative).
    pub t_ms: u32,
    /// Estimated background colour for this frame.
    pub bg: Color,
    /// All foreground blobs above the minimum area, sorted descending
    /// by area (largest first — useful when the tracker wants a stable
    /// processing order).
    pub blobs: Vec<Blob>,
    /// How this frame's pixels were grouped into blobs. Carried so that
    /// re-tracing a blob's outline later groups pixels the same way.
    pub segmentation: Segmentation,
}

/// How foreground pixels are grouped into blobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Segmentation {
    /// v0.3–v0.12: connected components of the foreground mask.
    /// Colour-blind — touching regions of different colours fuse.
    #[default]
    Components,
    /// v0.13: connected regions of similar colour, with edge fringes
    /// absorbed. Separates the bars of a test card; on smooth-gradient
    /// content (a Mandelbrot zoom) it shatters the picture instead.
    Regions,
    /// v1.5: regions whose growth stops at edges. A pixel on a strong edge
    /// (colour change across a two-pixel span) joins only other edge
    /// pixels, so an anti-aliased transition can no longer chain two
    /// different objects together — on `real_pendulum` the grey bob and
    /// the green arrow drawn across it fused into one mixed-colour blob.
    /// Edge fringes are then absorbed only where they wrap a region.
    Edges,
    /// Decide by measurement. The decompiler has no renderer, so on its
    /// own this behaves as `Components`; the `vac` facade, which does,
    /// decompiles each way, re-renders each, and keeps the one that
    /// reconstructs the source best.
    Auto,
}

fn label(f: &VideoFrame, mask: &[bool], w: usize, h: usize, seg: Segmentation) -> Vec<u32> {
    match seg {
        Segmentation::Regions => label_regions(f, mask, w, h),
        Segmentation::Edges => label_edge_regions(f, mask, w, h),
        Segmentation::Components | Segmentation::Auto => label_components_4(mask, w, h),
    }
}

/// Edge-gated region labelling (`Segmentation::Edges`).
fn label_edge_regions(f: &VideoFrame, mask: &[bool], w: usize, h: usize) -> Vec<u32> {
    let rgb = |i: usize| [f.pixels[i * 4] as i32, f.pixels[i * 4 + 1] as i32, f.pixels[i * 4 + 2] as i32];
    let dist_sq = |a: [i32; 3], b: [i32; 3]| {
        ((a[0] - b[0]).pow(2) + (a[1] - b[1]).pow(2) + (a[2] - b[2]).pow(2)) as u32
    };
    // Edge strength: the larger central difference across the pixel.
    let mut edge = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !mask[i] {
                continue;
            }
            let mut e = 0u32;
            if x > 0 && x + 1 < w {
                e = e.max(dist_sq(rgb(i - 1), rgb(i + 1)));
            }
            if y > 0 && y + 1 < h {
                e = e.max(dist_sq(rgb(i - w), rgb(i + w)));
            }
            edge[i] = e > EDGE_STRENGTH_SQ;
        }
    }
    let close = |a: usize, b: usize| dist_sq(rgb(a), rgb(b)) <= REGION_JOIN_SQ;
    // Flat pixels join flat pixels, edge pixels join edge pixels.
    let joins = |a: usize, b: usize| mask[a] && mask[b] && edge[a] == edge[b] && close(a, b);
    let mut labels = vec![0u32; w * h];
    let mut uf = UnionFind::new();
    let mut next: u32 = 1;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !mask[i] {
                continue;
            }
            let left = if x > 0 && joins(i, i - 1) { labels[i - 1] } else { 0 };
            let up = if y > 0 && joins(i, i - w) { labels[i - w] } else { 0 };
            match (left, up) {
                (0, 0) => {
                    labels[i] = next;
                    uf.make_set(next);
                    next += 1;
                }
                (a, 0) => labels[i] = a,
                (0, b) => labels[i] = b,
                (a, b) => {
                    labels[i] = a.min(b);
                    if a != b {
                        uf.union(a, b);
                    }
                }
            }
        }
    }
    for l in labels.iter_mut() {
        if *l != 0 {
            *l = uf.find(*l);
        }
    }
    for _ in 0..ABSORB_MAX_PASSES {
        if !absorb_wrapping_fringes(&mut labels, w, h) {
            break;
        }
    }
    labels
}

/// One absorption pass for `Edges`: a thin region joins the solid region it
/// shares the most border with — but only when that border is a large
/// share of the thin region itself. A fringe wraps the region it fringes;
/// a line drawn across a region touches it only where it crosses.
fn absorb_wrapping_fringes(labels: &mut [u32], w: usize, h: usize) -> bool {
    use std::collections::HashMap;
    let same_all_4 = |labels: &[u32], i: usize| {
        let (x, y) = (i % w, i / w);
        let l = labels[i];
        (x == 0 || labels[i - 1] == l)
            && (x + 1 == w || labels[i + 1] == l)
            && (y == 0 || labels[i - w] == l)
            && (y + 1 == h || labels[i + w] == l)
    };
    let mut solid: HashMap<u32, bool> = HashMap::new();
    let mut area: HashMap<u32, u32> = HashMap::new();
    for i in 0..w * h {
        let l = labels[i];
        if l == 0 {
            continue;
        }
        *area.entry(l).or_insert(0) += 1;
        let e = solid.entry(l).or_insert(false);
        if !*e && same_all_4(labels, i) {
            *e = true;
        }
    }
    let mut border: HashMap<(u32, u32), u32> = HashMap::new();
    for i in 0..w * h {
        let l = labels[i];
        if l == 0 || solid[&l] {
            continue;
        }
        let (x, y) = (i % w, i / w);
        let mut visit = |j: usize| {
            let m = labels[j];
            if m != 0 && m != l && solid[&m] {
                *border.entry((l, m)).or_insert(0) += 1;
            }
        };
        if x > 0 {
            visit(i - 1);
        }
        if x + 1 < w {
            visit(i + 1);
        }
        if y > 0 {
            visit(i - w);
        }
        if y + 1 < h {
            visit(i + w);
        }
    }
    let mut target: HashMap<u32, (u32, u32)> = HashMap::new();
    for (&(l, m), &n) in &border {
        if (n as f32) < FRINGE_MIN_WRAP * area[&l] as f32 {
            continue;
        }
        let e = target.entry(l).or_insert((m, n));
        if n > e.1 || (n == e.1 && m < e.0) {
            *e = (m, n);
        }
    }
    if target.is_empty() {
        return false;
    }
    for l in labels.iter_mut() {
        if let Some(&(m, _)) = target.get(l) {
            *l = m;
        }
    }
    true
}

/// Primitive class the decompiler thinks best fits this blob's silhouette.
/// Decided by the bbox fill ratio (`area / (bbox_w * bbox_h)`):
/// a perfectly filled rect is 1.0, a perfectly filled ellipse is π/4 ≈ 0.785,
/// and anything well below the ellipse band (a triangle is ~0.5, a star
/// ~0.35, a thin diagonal ~0.1) is a `Path` — the v0.6 primitive that
/// gets its outline from PAC's contour-tracing pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    Rect,
    Ellipse,
    Path,
}

#[derive(Debug, Clone, Copy)]
pub struct Blob {
    pub cx: f32,
    pub cy: f32,
    /// bbox half-width  (= (max_x - min_x + 1) / 2).
    pub rx: f32,
    /// bbox half-height (= (max_y - min_y + 1) / 2).
    pub ry: f32,
    /// Mean of `rx` and `ry`. Kept for v0.3-era tracker code paths that
    /// only care about a scalar size; new code should prefer `rx`/`ry`.
    pub radius: f32,
    /// `area / (bbox_w * bbox_h)`. Drives `kind`; also useful as a
    /// per-frame confidence score for the classification.
    pub fill_ratio: f32,
    /// Classification result of `fill_ratio` against `RECT_FILL_THRESHOLD`.
    pub kind: ShapeKind,
    /// Mean colour over interior pixels (4-neighbours all share label).
    /// Falls back to whole-blob mean for very thin shapes.
    pub color: Color,
    /// Pixel count of the blob.
    pub area: u32,
    /// Hu image-moment invariants (h₁..h₇). Translation, scale, rotation
    /// invariant — useful for shape-based matching across frames.
    pub hu: [f32; 7],
    /// Orientation of the principal (long) axis in degrees, in (-90, 90],
    /// clockwise from +x in screen coordinates — from the second central
    /// moments, `½·atan2(2μ₁₁, μ₂₀ − μ₀₂)` (v0.14). Meaningless when
    /// `var_major ≈ var_minor`.
    pub theta_deg: f32,
    /// Pixel-position variance along the principal axis and across it.
    /// Rotation-invariant size: a solid W×H rect has W²/12 and H²/12, a
    /// solid ellipse with semi-axes a, b has a²/4 and b²/4.
    pub var_major: f32,
    pub var_minor: f32,
    /// Rotation of the fitted primitive in degrees: 0 when an axis-aligned
    /// rect or ellipse fitted best, `theta_deg` when an oriented one did.
    pub angle_deg: f32,
    /// Coverage-weighted position variance along x and y (v1.1). Each
    /// pixel counts by the fraction of it the shape covers — read back from
    /// where its colour sits between the background and the fill — so an
    /// anti-aliased edge contributes its true partial area. The bbox of
    /// thresholded pixels reads a moving shape about a pixel too wide (both
    /// edges straddle a pixel at most sub-pixel phases), which is what made
    /// repeated round-trips grow every shape by a pixel per generation.
    pub soft_var_x: f32,
    pub soft_var_y: f32,
    /// Coverage-corrected bbox width and height (v1.1): the thresholded
    /// bbox, minus the uncovered part of its outermost columns and rows,
    /// plus the covered part of the sub-threshold pixels just outside. Exact
    /// for an anti-aliased rect at any sub-pixel phase, and — unlike a
    /// variance — unaffected by notches in it. 0 when sizing declined.
    pub soft_w: f32,
    pub soft_h: f32,
    /// Coverage-weighted area — `Σ α` over the blob and the background
    /// pixels touching it (v1.4). What an outline's stroke width is
    /// solved from: thresholded area counts every edge pixel above ~17%
    /// coverage as whole, and read a 4 px stroke as 4.5. 0 when declined.
    pub soft_area: f32,
    /// IoU of the fitted primitive against the blob's pixels.
    pub fit_iou: f32,
    /// Pixels enclosed by the blob but not part of it (v1.4): background
    /// that cannot reach the bbox border without crossing the blob. A
    /// large hole makes the blob an *outline*.
    pub hole_area: u32,
    /// IoU of the bbox rect and of the inscribed ellipse against the blob
    /// with its holes filled in — the shape an outline traces.
    pub filled_iou_rect: f32,
    pub filled_iou_ellipse: f32,
}

impl Blob {
    /// `sqrt(var_major / var_minor)`: 1 for a disc or a square, where no
    /// orientation exists; large for a rod.
    pub fn elongation(&self) -> f32 {
        (self.var_major / self.var_minor.max(1e-3)).sqrt()
    }
}

/// Bbox fill-ratio threshold above which a blob is classified as
/// `ShapeKind::Rect`. Picked so that a filled rect with H.264 chroma
/// subsampling artefacts (~0.94 typical) still classifies as rect, while
/// a filled circle (~0.78) classifies as ellipse and a rounded rect
/// (~0.85) lands in the ellipse bucket too (safer false-negative).
pub const RECT_FILL_THRESHOLD: f32 = 0.90;

/// Bbox fill-ratio floor for `ShapeKind::Ellipse` (v0.6).  A rasterised
/// ellipse fills π/4 ≈ 0.785 of its bbox and codec noise pulls that to
/// ~0.72 at worst; anything below is shaped like neither primitive and
/// classifies as `Path`.
pub const ELLIPSE_MIN_FILL: f32 = 0.70;

/// Blobs smaller than this stay `Ellipse` even when their fill ratio
/// says `Path`: a contour traced around a couple dozen pixels is
/// quantisation noise, not a measurable outline.
pub const MIN_PATH_AREA: u32 = 64;

/// Squared Euclidean colour distance threshold for the
/// foreground/background mask. ~24 per channel ≈ visually distinct
/// without flagging dithering noise.
const FG_THRESHOLD_SQ: u32 = 24 * 24 * 3;

/// A blob with fewer than this many pixels is treated as detection noise
/// and discarded (smaller than a 3x3 cluster).
pub(crate) const MIN_BLOB_AREA: u32 = 9;

/// A rect or ellipse must overlap a blob's pixels at least this well
/// (IoU) to stand for it; below, the outline is traced as a path.
pub(crate) const PRIMITIVE_MIN_IOU: f32 = 0.88;

/// Below this elongation (`sqrt(var_major / var_minor)`) a blob has no
/// meaningful orientation and is never fitted with a rotated primitive.
const ORIENTED_MIN_ELONGATION: f32 = 1.25;

/// How much better (IoU) an oriented fit must be than the best
/// axis-aligned one to be preferred.
const ORIENTED_MIN_GAIN: f32 = 0.03;

/// Squared RGB distance within which two adjacent foreground pixels are
/// the same region (v0.13). Well above H.264 noise on a flat fill, well
/// below the contrast of any two colours a viewer would call different.
const REGION_JOIN_SQ: u32 = 20 * 20 * 3;

/// Squared RGB distance around the modal border bin that still counts
/// toward the background mean.
const BG_REFINE_SQ: u32 = 12 * 12 * 3;

/// Soft sizing only applies to blobs whose interior colour spread (RMS per
/// channel) is under this — flat fills. Shaded blobs keep the bbox.
const SOFT_MAX_INTERIOR_STD: f64 = 10.0;
/// ...measured over at least this many interior pixels.
const SOFT_MIN_INTERIOR: u64 = 16;

/// Below this squared fill-to-background contrast, coverage cannot be read
/// from a pixel's colour and soft sizing falls back to whole pixels.
const SOFT_MIN_CONTRAST_SQ: f64 = 60.0 * 60.0;

/// Squared RGB distance from the background above which a pixel belongs
/// to the ground band (v1.1): about 7 levels per channel, well above
/// codec noise on a flat field.
const GROUND_MIN_SQ: u32 = 7 * 7 * 3;

/// Squared colour change across a two-pixel span above which a pixel is
/// on an edge (`Segmentation::Edges`): about 46 levels per channel.
const EDGE_STRENGTH_SQ: u32 = 80 * 80;
/// A thin region is a fringe of a solid one when their shared border is
/// at least this share of the thin region's area.
const FRINGE_MIN_WRAP: f32 = 0.4;

/// Upper bound on fringe-absorption passes in `label_regions`.
const ABSORB_MAX_PASSES: usize = 4;

/// Alpha below this is treated as transparent and excluded from both
/// background estimation and the foreground mask (v0.5.2). GIF alpha
/// is binary (0 or 255) and H.264 frames decode with alpha = 255
/// everywhere, so this only bites on transparent GIF canvases — where
/// the decoder's rgba(0,0,0,0) fill used to masquerade as a black
/// background.
pub(crate) const MIN_OPAQUE_ALPHA: u8 = 128;

/// Run detection across all frames.
pub fn detect_all(
    frames: &[VideoFrame],
    seg: Segmentation,
    model: Background,
    min_area: u32,
) -> Vec<Detection> {
    let mut out = Vec::with_capacity(frames.len());
    let mut t_ms: u32 = 0;
    for (i, f) in frames.iter().enumerate() {
        let bg = background_of(f, model);
        let mut blobs = detect_blobs_with(f, bg, seg, min_area);
        blobs.sort_by(|a, b| b.area.cmp(&a.area).then_with(|| cmp_position(a, b)));
        out.push(Detection {
            frame_index: i,
            t_ms,
            bg,
            blobs,
            segmentation: seg,
        });
        t_ms = t_ms.saturating_add(f.delay_ms);
    }
    out
}

/// The frame's background colour: the colour that occupies the most of
/// its border (v0.13).
///
/// Through v0.12 this was the per-channel median of four 5×5 corner
/// blocks. When the corners agree that is the same answer; when they do
/// not — a test card's bars, a scene whose lower half is floor — the
/// channels are medianed independently and the result can name a colour
/// that appears nowhere in the frame (`two_tone.vac` came back with a
/// red-bar "background" and its real dark ground drawn as an ellipse).
///
/// Border pixels are binned at 5 bits per channel; the fullest bin wins,
/// and the answer is the mean of every border pixel within
/// `BG_REFINE_SQ` of that bin's mean, so codec noise straddling a bin
/// edge does not shift it. Transparent pixels are excluded; a canvas with
/// no opaque border pixel falls back to white, the colour viewers
/// composite transparent GIFs onto.
pub fn estimate_background_border(f: &VideoFrame) -> Color {
    let (w, h) = (f.width as usize, f.height as usize);
    if w == 0 || h == 0 {
        return Color::rgb(255, 255, 255);
    }
    let mut border: Vec<[u8; 3]> = Vec::with_capacity(2 * (w + h));
    let mut transparent = 0usize;
    let mut take = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        if f.pixels[i + 3] >= MIN_OPAQUE_ALPHA {
            border.push([f.pixels[i], f.pixels[i + 1], f.pixels[i + 2]]);
        } else {
            transparent += 1;
        }
    };
    for x in 0..w {
        take(x, 0);
        if h > 1 {
            take(x, h - 1);
        }
    }
    for y in 1..h.saturating_sub(1) {
        take(0, y);
        if w > 1 {
            take(w - 1, y);
        }
    }
    // A mostly transparent border means the background *is*
    // transparency. Voting among the few opaque border pixels would
    // instead crown whatever foreground touches the edge — on the
    // `real_loader` spinner, the tick colour, after which 9 of its 12
    // ticks matched "background" and vanished.
    if border.len() <= transparent {
        return Color::rgb(255, 255, 255);
    }
    let bin = |p: &[u8; 3]| ((p[0] as u32 >> 3) << 10) | ((p[1] as u32 >> 3) << 5) | (p[2] as u32 >> 3);
    let mut counts: std::collections::HashMap<u32, (u32, [u64; 3])> = std::collections::HashMap::new();
    for p in &border {
        let e = counts.entry(bin(p)).or_insert((0, [0; 3]));
        e.0 += 1;
        for c in 0..3 {
            e.1[c] += p[c] as u64;
        }
    }
    // Fullest bin; ties to the lower bin index, so the result never
    // depends on HashMap iteration order.
    let (_, (n, sum)) = counts
        .iter()
        .max_by(|a, b| a.1 .0.cmp(&b.1 .0).then(b.0.cmp(a.0)))
        .expect("non-empty");
    let seed = [
        (sum[0] / *n as u64) as i32,
        (sum[1] / *n as u64) as i32,
        (sum[2] / *n as u64) as i32,
    ];
    let (mut acc, mut k) = ([0u64; 3], 0u64);
    for p in &border {
        let d = |c: usize| p[c] as i32 - seed[c];
        if ((d(0) * d(0) + d(1) * d(1) + d(2) * d(2)) as u32) <= BG_REFINE_SQ {
            for c in 0..3 {
                acc[c] += p[c] as u64;
            }
            k += 1;
        }
    }
    let k = k.max(1);
    Color::rgb((acc[0] / k) as u8, (acc[1] / k) as u8, (acc[2] / k) as u8)
}

/// The frame's background colour under `model` (v0.13).
pub fn background_of(f: &VideoFrame, model: Background) -> Color {
    match model {
        Background::Border => estimate_background_border(f),
        Background::Corners | Background::Auto => estimate_background(f),
    }
}

/// Which rule picks a frame's single background colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Background {
    /// v0.1–v0.12: per-channel median of four 5×5 corner blocks.
    #[default]
    Corners,
    /// v0.13: the modal colour of the frame's border.
    Border,
    /// Decide by measurement — resolved by the `vac` facade, which can
    /// render; on its own the decompiler treats it as `Corners`.
    Auto,
}

/// Sample a small block at each corner and return the median RGB — the
/// v0.1–v0.12 background rule, `Background::Corners`.
///
/// Fully-transparent pixels are excluded from the samples. If *no*
/// corner pixel is opaque, the canvas has no measurable background
/// colour at all — fall back to white, the colour viewers composite
/// transparent GIFs onto.
pub fn estimate_background(f: &VideoFrame) -> Color {
    let w = f.width as i32;
    let h = f.height as i32;
    let block = 5i32;
    let corners = [
        (0, 0),
        (w - block, 0),
        (0, h - block),
        (w - block, h - block),
    ];
    let mut samples: Vec<(u8, u8, u8)> = Vec::with_capacity(4);
    for (cx, cy) in corners {
        if let Some(c) = sample_block_avg(f, cx.max(0), cy.max(0), block, block) {
            samples.push(c);
        }
    }
    if samples.is_empty() {
        return Color::rgb(255, 255, 255);
    }
    let mut rs: Vec<u8> = samples.iter().map(|c| c.0).collect();
    let mut gs: Vec<u8> = samples.iter().map(|c| c.1).collect();
    let mut bs: Vec<u8> = samples.iter().map(|c| c.2).collect();
    rs.sort();
    gs.sort();
    bs.sort();
    Color::rgb(rs[rs.len() / 2], gs[gs.len() / 2], bs[bs.len() / 2])
}

fn sample_block_avg(f: &VideoFrame, x0: i32, y0: i32, w: i32, h: i32) -> Option<(u8, u8, u8)> {
    let fw = f.width as i32;
    let fh = f.height as i32;
    let x1 = (x0 + w).min(fw);
    let y1 = (y0 + h).min(fh);
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    let mut sr: u32 = 0;
    let mut sg: u32 = 0;
    let mut sb: u32 = 0;
    let mut n: u32 = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * fw + x) * 4) as usize;
            if f.pixels[i + 3] < MIN_OPAQUE_ALPHA {
                continue;
            }
            sr += f.pixels[i] as u32;
            sg += f.pixels[i + 1] as u32;
            sb += f.pixels[i + 2] as u32;
            n += 1;
        }
    }
    if n == 0 {
        return None;
    }
    Some(((sr / n) as u8, (sg / n) as u8, (sb / n) as u8))
}

// --------------------------------------------------------------------
// Multi-blob detection
// --------------------------------------------------------------------

/// Foreground mask against the estimated background.  Transparent
/// pixels are background by definition — without the alpha guard, a
/// transparent canvas (decoded as rgba(0,0,0,0)) turns into one giant
/// foreground blob the moment the estimated background isn't black.
pub(crate) fn foreground_mask(f: &VideoFrame, bg: Color) -> Vec<bool> {
    let w = f.width as usize;
    let h = f.height as usize;
    let mut mask = vec![false; w * h];
    for i in 0..(w * h) {
        if f.pixels[i * 4 + 3] < MIN_OPAQUE_ALPHA {
            continue;
        }
        let r = f.pixels[i * 4];
        let g = f.pixels[i * 4 + 1];
        let b = f.pixels[i * 4 + 2];
        let dr = r as i32 - bg.r as i32;
        let dg = g as i32 - bg.g as i32;
        let db = b as i32 - bg.b as i32;
        let dist_sq = (dr * dr + dg * dg + db * db) as u32;
        mask[i] = dist_sq > FG_THRESHOLD_SQ;
    }
    mask
}

#[cfg(test)]
pub(crate) fn detect_blobs(f: &VideoFrame, bg: Color, seg: Segmentation) -> Vec<Blob> {
    detect_blobs_with(f, bg, seg, MIN_BLOB_AREA)
}

/// [`detect_blobs`] with an explicit minimum blob area — the one detector
/// tolerance the quality dial (v1.0) scales.
pub(crate) fn detect_blobs_with(f: &VideoFrame, bg: Color, seg: Segmentation, min_area: u32) -> Vec<Blob> {
    let mask = foreground_mask(f, bg);
    detect_blobs_masked(f, &mask, bg, seg, min_area)
}

/// Pixels whose colour is measurably off the background but too close to
/// it to count as foreground (v1.1): the band between `GROUND_MIN_SQ` and
/// the foreground threshold. A plot area a few shades lighter than its
/// margin lives here — and was painted the margin's colour, a flat error
/// across half the frame.
pub(crate) fn ground_mask(f: &VideoFrame, bg: Color) -> Vec<bool> {
    let n = (f.width * f.height) as usize;
    (0..n)
        .map(|i| {
            let p = &f.pixels[i * 4..i * 4 + 4];
            if p[3] < MIN_OPAQUE_ALPHA {
                return false;
            }
            let d = |c: usize, b: u8| p[c] as i32 - b as i32;
            let sq = (d(0, bg.r).pow(2) + d(1, bg.g).pow(2) + d(2, bg.b).pow(2)) as u32;
            sq > GROUND_MIN_SQ && sq <= FG_THRESHOLD_SQ
        })
        .collect()
}

/// Blobs of a precomputed mask.
pub(crate) fn detect_blobs_masked(
    f: &VideoFrame,
    mask: &[bool],
    bg: Color,
    seg: Segmentation,
    min_area: u32,
) -> Vec<Blob> {
    let w = f.width as usize;
    let h = f.height as usize;
    let labels = label(f, mask, w, h, seg);

    // Per-label area count.
    let mut counts = std::collections::HashMap::<u32, u32>::new();
    for &l in &labels {
        if l != 0 {
            *counts.entry(l).or_insert(0) += 1;
        }
    }

    // Keep labels above min area; build a stable ordering for the per-blob
    // accumulators below.
    //
    // The sort is load-bearing, not cosmetic: `counts` is a HashMap with
    // a randomly-seeded hasher, so iterating it directly makes the blob
    // order — and therefore the tie-break between equal-area blobs, and
    // therefore the emitted `shape_<n>` numbering — differ between runs
    // of the same binary on the same file. Labels come out of the
    // raster-scan union-find in first-appearance order, so sorting them
    // makes the output reproducible.
    let mut kept_labels: Vec<u32> = counts
        .iter()
        .filter_map(|(&l, &c)| if c >= min_area { Some(l) } else { None })
        .collect();
    kept_labels.sort_unstable();
    if kept_labels.is_empty() {
        return Vec::new();
    }
    let label_to_idx: std::collections::HashMap<u32, usize> = kept_labels
        .iter()
        .enumerate()
        .map(|(i, &l)| (l, i))
        .collect();

    let n = kept_labels.len();
    let mut acc: Vec<BlobAcc> = (0..n).map(|_| BlobAcc::new()).collect();

    // Single pass over the frame: accumulate per-blob stats simultaneously.
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let l = labels[idx];
            if l == 0 {
                continue;
            }
            let Some(&bi) = label_to_idx.get(&l) else {
                continue; // sub-min-area noise blob
            };
            let a = &mut acc[bi];

            a.area += 1;
            let pi = idx * 4;
            let r = f.pixels[pi] as u64;
            let g = f.pixels[pi + 1] as u64;
            let b = f.pixels[pi + 2] as u64;
            a.all_r += r;
            a.all_g += g;
            a.all_b += b;

            // Raw moments for centroid + Hu invariants.
            let xf = x as f64;
            let yf = y as f64;
            a.m00 += 1.0;
            a.m10 += xf;
            a.m01 += yf;

            // Bounding box.
            if (x as u32) < a.min_x {
                a.min_x = x as u32;
            }
            if (x as u32) > a.max_x {
                a.max_x = x as u32;
            }
            if (y as u32) < a.min_y {
                a.min_y = y as u32;
            }
            if (y as u32) > a.max_y {
                a.max_y = y as u32;
            }

            // Interior-pixel colour (4-neighbours share the label).
            let interior = x > 0
                && y > 0
                && x + 1 < w
                && y + 1 < h
                && labels[idx - 1] == l
                && labels[idx + 1] == l
                && labels[idx - w] == l
                && labels[idx + w] == l;
            if interior {
                a.int_r += r;
                a.int_g += g;
                a.int_b += b;
                a.int_count += 1;
            }
        }
    }

    // Second pass: central moments (require centroids from pass 1).
    // Bound the moments by the bbox so we only revisit the relevant region.
    for (bi, a) in acc.iter_mut().enumerate() {
        if a.area == 0 {
            continue;
        }
        let cx = a.m10 / a.m00;
        let cy = a.m01 / a.m00;
        let l = kept_labels[bi];

        let (mut mu20, mut mu02, mut mu11) = (0.0f64, 0.0f64, 0.0f64);
        let (mut mu30, mut mu03, mut mu21, mut mu12) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);

        let x0 = a.min_x as usize;
        let x1 = a.max_x as usize;
        let y0 = a.min_y as usize;
        let y1 = a.max_y as usize;
        let ecx = (x0 + x1 + 1) as f64 * 0.5;
        let ecy = (y0 + y1 + 1) as f64 * 0.5;
        let erx = (x1 - x0 + 1) as f64 * 0.5;
        let ery = (y1 - y0 + 1) as f64 * 0.5;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let ex = (x as f64 + 0.5 - ecx) / erx;
                let ey = (y as f64 + 0.5 - ecy) / ery;
                let in_ellipse = ex * ex + ey * ey <= 1.0;
                if in_ellipse {
                    a.ell_total += 1;
                }
                if labels[y * w + x] != l {
                    continue;
                }
                if in_ellipse {
                    a.ell_hit += 1;
                }
                let dx = x as f64 - cx;
                let dy = y as f64 - cy;
                let dx2 = dx * dx;
                let dy2 = dy * dy;
                mu20 += dx2;
                mu02 += dy2;
                mu11 += dx * dy;
                mu30 += dx2 * dx;
                mu03 += dy2 * dy;
                mu21 += dx2 * dy;
                mu12 += dx * dy2;
            }
        }
        a.cx = cx as f32;
        a.cy = cy as f32;
        a.hu = hu_moments(a.m00, mu20, mu02, mu11, mu30, mu03, mu21, mu12);
        a.mu20 = mu20;
        a.mu02 = mu02;
        a.mu11 = mu11;
    }

    // Third pass (v0.14): oriented fits. Second moments give each blob's
    // principal axis; an oriented rect and an oriented ellipse of the
    // moment-implied size are scored against the pixels exactly as the
    // axis-aligned ones are, and compete with them below.
    let mut oriented: Vec<Option<OrientedFit>> = Vec::with_capacity(acc.len());
    for (bi, a) in acc.iter().enumerate() {
        oriented.push(if a.area == 0 {
            None
        } else {
            oriented_fit(&labels, w, h, kept_labels[bi], a)
        });
    }

    // Fifth pass (v1.4): holes, and how the filled outline fits.
    let rings: Vec<(u32, f32, f32)> = acc
        .iter()
        .enumerate()
        .map(|(bi, a)| if a.area == 0 { (0, 0.0, 0.0) } else { ring_stats(&labels, w, kept_labels[bi], a) })
        .collect();

    // Fourth pass (v1.1): coverage-weighted size.
    let soft: Vec<SoftSize> = acc
        .iter()
        .enumerate()
        .map(|(bi, a)| {
            if a.area == 0 {
                return SoftSize::default();
            }
            soft_size(f, &labels, w, h, kept_labels[bi], a, bg)
        })
        .collect();

    acc.into_iter()
        .zip(oriented)
        .zip(soft)
        .zip(rings)
        .filter(|(((a, _), _), _)| a.area > 0)
        .map(|(((a, of), soft), (hole_area, filled_iou_rect, filled_iou_ellipse))| {
            let bbox_w = (a.max_x - a.min_x + 1) as f32;
            let bbox_h = (a.max_y - a.min_y + 1) as f32;
            let rx = bbox_w * 0.5;
            let ry = bbox_h * 0.5;
            let radius = (rx + ry) * 0.5;
            let bbox_area = bbox_w * bbox_h;
            let fill_ratio = if bbox_area > 0.0 {
                (a.area as f32) / bbox_area
            } else {
                0.0
            };
            // v0.13: measure, don't threshold. Each primitive's fit is its
            // IoU with the blob's actual pixels — a bbox rect's IoU is the
            // fill ratio, an inscribed ellipse's is counted on the
            // lattice. The better fit wins; if neither reaches
            // `PRIMITIVE_MIN_IOU` the outline is traced as a path.
            //
            // The fill-ratio bands this replaces called a bar notched by
            // a caption box (fill 0.85) an *ellipse*, whose real overlap
            // with it is ~0.7: `testsrc2`'s red bar came back oval.
            let iou_rect = fill_ratio;
            let ell_union = a.area + a.ell_total - a.ell_hit;
            let iou_ellipse = if ell_union > 0 {
                a.ell_hit as f32 / ell_union as f32
            } else {
                0.0
            };
            let axis_best = iou_rect.max(iou_ellipse);
            let (principal, var_major, var_minor) = principal_axes(a.area, a.mu20, a.mu02, a.mu11);
            // An oriented fit has to beat the axis-aligned ones by a clear
            // margin, so a shape sitting near 0° does not flicker between
            // the two descriptions from frame to frame.
            let (mut angle_deg, mut fit_rect, mut fit_iou) = (0.0f32, iou_rect >= iou_ellipse, axis_best);
            if let Some(of) = &of {
                let (o_rect, o_iou) = if of.iou_rect >= of.iou_ellipse {
                    (true, of.iou_rect)
                } else {
                    (false, of.iou_ellipse)
                };
                if o_iou > axis_best + ORIENTED_MIN_GAIN {
                    angle_deg = principal;
                    fit_rect = o_rect;
                    fit_iou = o_iou;
                }
            }
            let best = fit_iou;
            let kind = if best >= PRIMITIVE_MIN_IOU {
                if fit_rect {
                    ShapeKind::Rect
                } else {
                    ShapeKind::Ellipse
                }
            } else if a.area >= MIN_PATH_AREA {
                ShapeKind::Path
            } else {
                // Too small to trace and fitting neither primitive (a
                // diagonal loader tick): the ellipse, which overclaims
                // the bbox least — what v0.4–v0.12 did here too.
                angle_deg = 0.0;
                ShapeKind::Ellipse
            };
            if kind == ShapeKind::Path {
                angle_deg = 0.0;
            }
            let color = if a.int_count > 0 {
                Color::rgb(
                    (a.int_r / a.int_count) as u8,
                    (a.int_g / a.int_count) as u8,
                    (a.int_b / a.int_count) as u8,
                )
            } else {
                let n = a.area as u64;
                Color::rgb(
                    (a.all_r / n) as u8,
                    (a.all_g / n) as u8,
                    (a.all_b / n) as u8,
                )
            };
            Blob {
                cx: a.cx,
                cy: a.cy,
                rx,
                ry,
                radius,
                fill_ratio,
                kind,
                color,
                area: a.area,
                hu: a.hu,
                theta_deg: principal,
                var_major,
                var_minor,
                angle_deg,
                soft_var_x: soft.var_x,
                soft_var_y: soft.var_y,
                soft_w: soft.w,
                soft_h: soft.h,
                soft_area: soft.area,
                fit_iou: best,
                hole_area,
                filled_iou_rect,
                filled_iou_ellipse,
            }
        })
        .collect()
}

/// Raster-order tie-break (top-to-bottom, then left-to-right) for
/// blobs the area sort can't separate — two identically-sized boxes on
/// the same frame must always come out in the same order.
pub(crate) fn cmp_position(a: &Blob, b: &Blob) -> std::cmp::Ordering {
    a.cy
        .partial_cmp(&b.cy)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a.cx.partial_cmp(&b.cx).unwrap_or(std::cmp::Ordering::Equal))
}

/// Re-trace the outer contour of the blob nearest `near` in `frame`
/// (v0.6).  Re-runs the same deterministic mask + CCL as
/// `detect_blobs`, picks the foreground component closest to `near`
/// (within `max_seed_dist` pixels), and hands it to PAC's
/// Moore-neighbour tracer.  Returns clockwise boundary pixels, or
/// `None` when no component lies close enough — callers fall back to
/// the ellipse primitive in that case.
///
/// `min_component_area` guards against seeding on noise: the labels
/// array still contains sub-`MIN_BLOB_AREA` specks that `detect_blobs`
/// filtered from its blob list, and a blob whose mask is a ring (a
/// shaded ball with a near-background specular centre) has its
/// centroid *inside the hole*, right next to any such speck.  Callers
/// pass a fraction of the blob's measured area so the search can only
/// land on the component that actually is the blob.
pub fn extract_contour(
    frame: &VideoFrame,
    bg: Color,
    near: (f32, f32),
    max_seed_dist: f32,
    min_component_area: u32,
    seg: Segmentation,
) -> Option<Vec<(f32, f32)>> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let mask = foreground_mask(frame, bg);
    let labels = label(frame, &mask, w, h, seg);

    // Component sizes, so the seed search can skip noise specks.
    let mut label_area = std::collections::HashMap::<u32, u32>::new();
    for &l in &labels {
        if l != 0 {
            *label_area.entry(l).or_insert(0) += 1;
        }
    }

    // Nearest sufficiently-large labelled pixel to `near` within the
    // search radius.
    let r = max_seed_dist.ceil().max(1.0) as i32;
    let cx = near.0.round() as i32;
    let cy = near.1.round() as i32;
    let mut best: Option<(f32, u32, usize, usize)> = None; // (dist_sq, label, x, y)
    for y in (cy - r).max(0)..=(cy + r).min(h as i32 - 1) {
        for x in (cx - r).max(0)..=(cx + r).min(w as i32 - 1) {
            let l = labels[y as usize * w + x as usize];
            if l == 0 || label_area.get(&l).copied().unwrap_or(0) < min_component_area {
                continue;
            }
            let d = (x as f32 - near.0).powi(2) + (y as f32 - near.1).powi(2);
            if best.map(|(bd, ..)| d < bd).unwrap_or(true) {
                best = Some((d, l, x as usize, y as usize));
            }
        }
    }
    let (_, target, seed_x, seed_y) = best?;
    trace_component(frame, &labels, w, h, target, seed_x, seed_y)
}

/// The labelled component nearest `near` in `frame`, found exactly as
/// [`extract_contour`] finds it: `(labels, target label)`.
fn find_component(
    frame: &VideoFrame,
    bg: Color,
    near: (f32, f32),
    max_seed_dist: f32,
    min_component_area: u32,
    seg: Segmentation,
) -> Option<(Vec<u32>, u32)> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let mask = foreground_mask(frame, bg);
    let labels = label(frame, &mask, w, h, seg);
    let mut label_area = std::collections::HashMap::<u32, u32>::new();
    for &l in &labels {
        if l != 0 {
            *label_area.entry(l).or_insert(0) += 1;
        }
    }
    let r = max_seed_dist.ceil().max(1.0) as i32;
    let cx = near.0.round() as i32;
    let cy = near.1.round() as i32;
    let mut best: Option<(f32, u32)> = None;
    for y in (cy - r).max(0)..=(cy + r).min(h as i32 - 1) {
        for x in (cx - r).max(0)..=(cx + r).min(w as i32 - 1) {
            let l = labels[y as usize * w + x as usize];
            if l == 0 || label_area.get(&l).copied().unwrap_or(0) < min_component_area {
                continue;
            }
            let d = (x as f32 - near.0).powi(2) + (y as f32 - near.1).powi(2);
            if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, l));
            }
        }
    }
    let (_, target) = best?;
    Some((labels, target))
}

/// The *interior* pixels — all four neighbours in the same component, so
/// no anti-aliased edge — of the component nearest `near`, as
/// `(x, y, rgb)`. What a gradient is fitted to (v1.2).
pub fn interior_pixels(
    frame: &VideoFrame,
    bg: Color,
    near: (f32, f32),
    max_seed_dist: f32,
    min_component_area: u32,
    seg: Segmentation,
) -> Option<Vec<(f32, f32, [f32; 3])>> {
    let (labels, target) = find_component(frame, bg, near, max_seed_dist, min_component_area, seg)?;
    let w = frame.width as usize;
    let h = frame.height as usize;
    let mut out = Vec::new();
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let i = y * w + x;
            if labels[i] != target
                || labels[i - 1] != target
                || labels[i + 1] != target
                || labels[i - w] != target
                || labels[i + w] != target
            {
                continue;
            }
            let p = i * 4;
            // Continuous coordinates: the centre of pixel (x, y).
            out.push((
                x as f32 + 0.5,
                y as f32 + 0.5,
                [frame.pixels[p] as f32, frame.pixels[p + 1] as f32, frame.pixels[p + 2] as f32],
            ));
        }
    }
    Some(out)
}

fn trace_component(
    frame: &VideoFrame,
    labels: &[u32],
    w: usize,
    h: usize,
    target: u32,
    seed_x: usize,
    seed_y: usize,
) -> Option<Vec<(f32, f32)>> {
    // Full bbox + area of the chosen component (the tracer's start
    // pixel must be the component's true topmost-leftmost pixel).
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut area = 0u32;
    for y in 0..h {
        for x in 0..w {
            if labels[y * w + x] != target {
                continue;
            }
            area += 1;
            min_x = min_x.min(x as u32);
            max_x = max_x.max(x as u32);
            min_y = min_y.min(y as u32);
            max_y = max_y.max(y as u32);
        }
    }
    if area == 0 {
        return None;
    }

    let region = pac_decompiler::regions::LabelledRegion {
        cluster: 0,
        area,
        min_x,
        max_x,
        min_y,
        max_y,
        seed_x: seed_x as u32,
        seed_y: seed_y as u32,
        label: target,
    };
    pac_decompiler::contour::trace_outer_contour(labels, frame.width, frame.height, &region)
}

/// Holes of one blob and the fit of its filled outline: `(hole area,
/// bbox-rect IoU, inscribed-ellipse IoU)` of the blob with holes filled.
fn ring_stats(labels: &[u32], w: usize, l: u32, a: &BlobAcc) -> (u32, f32, f32) {
    let (x0, y0) = (a.min_x as usize, a.min_y as usize);
    let (bw, bh) = ((a.max_x - a.min_x + 1) as usize, (a.max_y - a.min_y + 1) as usize);
    let at = |x: usize, y: usize| labels[(y0 + y) * w + x0 + x] == l;
    // Flood the non-blob pixels reachable from the bbox border.
    let mut outside = vec![false; bw * bh];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for x in 0..bw {
        for y in [0, bh - 1] {
            if !at(x, y) && !outside[y * bw + x] {
                outside[y * bw + x] = true;
                stack.push((x, y));
            }
        }
    }
    for y in 0..bh {
        for x in [0, bw - 1] {
            if !at(x, y) && !outside[y * bw + x] {
                outside[y * bw + x] = true;
                stack.push((x, y));
            }
        }
    }
    while let Some((x, y)) = stack.pop() {
        let mut go = |nx: usize, ny: usize| {
            if !at(nx, ny) && !outside[ny * bw + nx] {
                outside[ny * bw + nx] = true;
                stack.push((nx, ny));
            }
        };
        if x > 0 {
            go(x - 1, y);
        }
        if x + 1 < bw {
            go(x + 1, y);
        }
        if y > 0 {
            go(x, y - 1);
        }
        if y + 1 < bh {
            go(x, y + 1);
        }
    }
    let (ecx, ecy, erx, ery) = (bw as f64 * 0.5, bh as f64 * 0.5, bw as f64 * 0.5, bh as f64 * 0.5);
    let (mut holes, mut filled, mut ell_total, mut ell_hit) = (0u32, 0u32, 0u32, 0u32);
    for y in 0..bh {
        for x in 0..bw {
            let is_filled = at(x, y) || !outside[y * bw + x];
            if !at(x, y) && !outside[y * bw + x] {
                holes += 1;
            }
            let ex = (x as f64 + 0.5 - ecx) / erx;
            let ey = (y as f64 + 0.5 - ecy) / ery;
            let inside = ex * ex + ey * ey <= 1.0;
            if inside {
                ell_total += 1;
            }
            if is_filled {
                filled += 1;
                if inside {
                    ell_hit += 1;
                }
            }
        }
    }
    let iou_rect = filled as f32 / (bw * bh) as f32;
    let union = filled + ell_total - ell_hit;
    let iou_ell = if union == 0 { 0.0 } else { ell_hit as f32 / union as f32 };
    (holes, iou_rect, iou_ell)
}

/// Coverage-weighted position variances `(var_x, var_y)` of one blob.
///
/// Coverage α of a pixel is the projection of `pixel − bg` onto
/// `fill − bg`, clamped to [0, 1], where `fill` is the blob's interior
/// colour. Pixels of the blob count with their α, and so do background
/// pixels touching it: an edge pixel covered 15% falls under the
/// foreground threshold but is still 15% shape. With too little contrast
/// to read coverage, every blob pixel counts fully.
#[derive(Debug, Clone, Copy, Default)]
struct SoftSize {
    var_x: f32,
    var_y: f32,
    w: f32,
    h: f32,
    area: f32,
}

fn soft_size(
    f: &VideoFrame,
    labels: &[u32],
    w: usize,
    h: usize,
    l: u32,
    a: &BlobAcc,
    bg: Color,
) -> SoftSize {
    let fill = if a.int_count > 0 {
        [
            (a.int_r / a.int_count) as f64,
            (a.int_g / a.int_count) as f64,
            (a.int_b / a.int_count) as f64,
        ]
    } else {
        let n = a.area.max(1) as f64;
        [a.all_r as f64 / n, a.all_g as f64 / n, a.all_b as f64 / n]
    };
    let bgc = [bg.r as f64, bg.g as f64, bg.b as f64];
    let v = [fill[0] - bgc[0], fill[1] - bgc[1], fill[2] - bgc[2]];
    let vv = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    let readable = vv >= SOFT_MIN_CONTRAST_SQ;

    // The coverage model assumes a flat fill with anti-aliased edges. A
    // shaded or textured blob (a lit ball, a photograph) breaks it: its
    // colour variation reads as partial coverage. Measure the blob's own
    // interior spread and decline — returning zeros sends the caller back
    // to the bbox — when it is not flat.
    if a.int_count >= SOFT_MIN_INTERIOR {
        let (mut sq, mut n) = (0.0f64, 0u64);
        for y in a.min_y as usize..=a.max_y as usize {
            for x in a.min_x as usize..=a.max_x as usize {
                let i = y * w + x;
                if labels[i] != l
                    || x == 0
                    || y == 0
                    || x + 1 >= w
                    || y + 1 >= h
                    || labels[i - 1] != l
                    || labels[i + 1] != l
                    || labels[i - w] != l
                    || labels[i + w] != l
                {
                    continue;
                }
                let p = i * 4;
                for c in 0..3 {
                    let d = f.pixels[p + c] as f64 - fill[c];
                    sq += d * d;
                }
                n += 1;
            }
        }
        if n > 0 && (sq / (3.0 * n as f64)).sqrt() > SOFT_MAX_INTERIOR_STD {
            return SoftSize::default();
        }
    }

    let x0 = (a.min_x as usize).saturating_sub(1);
    let y0 = (a.min_y as usize).saturating_sub(1);
    let x1 = (a.max_x as usize + 1).min(w - 1);
    let y1 = (a.max_y as usize + 1).min(h - 1);
    let (mut sw, mut sx, mut sy, mut sxx, mut syy) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
    // Per-column / per-row coverage sums for the edge-profile size.
    let (bx0, bx1, by0, by1) = (a.min_x as usize, a.max_x as usize, a.min_y as usize, a.max_y as usize);
    // Coverage samples of the four lines that matter on each axis: the
    // outermost column/row of the bbox and the sub-threshold one outside it.
    let mut col_cov: Vec<Vec<f64>> = vec![Vec::new(); x1 - x0 + 1];
    let mut row_cov: Vec<Vec<f64>> = vec![Vec::new(); y1 - y0 + 1];
    for y in y0..=y1 {
        for x in x0..=x1 {
            let i = y * w + x;
            let lab = labels[i];
            let touching = lab == 0
                && ((x > 0 && labels[i - 1] == l)
                    || (x + 1 < w && labels[i + 1] == l)
                    || (y > 0 && labels[i - w] == l)
                    || (y + 1 < h && labels[i + w] == l));
            if lab != l && !touching {
                continue;
            }
            // An edge that blends into *transparency* is no blend of fill
            // and background at all (the `real_loader` spinner): decline.
            if f.pixels[i * 4 + 3] < MIN_OPAQUE_ALPHA {
                return SoftSize::default();
            }
            let alpha = if !readable {
                if lab == l { 1.0 } else { 0.0 }
            } else {
                // Coverage is read against the *local* fill — the most
                // saturated blob pixel in the 3×3 neighbourhood — not the
                // blob's mean. On a flat fill they agree; on a shaded ball
                // the rim is darker than the mean, and reading it against
                // the mean called solid rim pixels "partly covered" and
                // shrank `real_bouncing_ball` (IoU 0.87 → 0.71).
                let px = |j: usize| {
                    [
                        f.pixels[j * 4] as f64 - bgc[0],
                        f.pixels[j * 4 + 1] as f64 - bgc[1],
                        f.pixels[j * 4 + 2] as f64 - bgc[2],
                    ]
                };
                let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
                let u = px(i);
                let mut local = v;
                let mut best = -1.0f64;
                for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                    for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                        let j = yy * w + xx;
                        if labels[j] != l {
                            continue;
                        }
                        let q = px(j);
                        let m = dot(q, q);
                        if m > best {
                            best = m;
                            local = q;
                        }
                    }
                }
                let ll = dot(local, local);
                if ll < SOFT_MIN_CONTRAST_SQ {
                    if lab == l { 1.0 } else { 0.0 }
                } else {
                    (dot(u, local) / ll).clamp(0.0, 1.0)
                }
            };
            // Edge profile: a pixel counts toward its column's coverage
            // when the shape continues horizontally next to it (so the
            // column's mean reads the edge, not the shape's curvature),
            // and likewise for rows.
            let horiz = (x > 0 && labels[i - 1] == l) || (x + 1 < w && labels[i + 1] == l);
            let vert = (y > 0 && labels[i - w] == l) || (y + 1 < h && labels[i + w] == l);
            let edge_col = x + 1 == bx0 || x == bx0 || x == bx1 || x == bx1 + 1;
            let edge_row = y + 1 == by0 || y == by0 || y == by1 || y == by1 + 1;
            if horiz && edge_col && (y >= by0 && y <= by1) {
                col_cov[x - x0].push(alpha);
            }
            if vert && edge_row && (x >= bx0 && x <= bx1) {
                row_cov[y - y0].push(alpha);
            }
            if alpha <= 0.0 {
                continue;
            }
            let (xf, yf) = (x as f64, y as f64);
            sw += alpha;
            sx += alpha * xf;
            sy += alpha * yf;
            sxx += alpha * xf * xf;
            syy += alpha * yf * yf;
        }
    }
    if sw <= 0.0 {
        return SoftSize::default();
    }
    let (mx, my) = (sx / sw, sy / sw);
    // A straight anti-aliased edge has the same coverage all along it, so
    // the median along the line reads it exactly — and, unlike a mean,
    // ignores a rounded corner's missing pixels (a 3 px loader tick with
    // clipped corners measured 2.71 px wide by its mean).
    let median = |v: &[f64]| -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        let mut s = v.to_vec();
        s.sort_by(|a, b| a.total_cmp(b));
        s[s.len() / 2]
    };
    // bbox span, minus what the outermost columns/rows leave uncovered,
    // plus what the sub-threshold pixels just outside cover.
    // The outermost line of an anti-aliased edge always reads above ~0.17
    // (fainter pixels are not in the mask at all). A line whose median
    // reads ~0 is a hard, pixel-art end — a one-pixel cap — and there the
    // bbox already is the extent.
    let inner = |v: &[f64]| {
        let m = median(v);
        if m < 0.1 {
            1.0
        } else {
            m
        }
    };
    let edge = |cov: &[Vec<f64>], lo: usize, hi: usize, origin: usize| -> f64 {
        let span = (hi - lo + 1) as f64;
        let inner_lo = inner(&cov[lo - origin]);
        let inner_hi = inner(&cov[hi - origin]);
        let outer_lo = if lo > origin { median(&cov[lo - origin - 1]) } else { 0.0 };
        let outer_hi = if hi - origin + 1 < cov.len() { median(&cov[hi - origin + 1]) } else { 0.0 };
        span - (1.0 - inner_lo) - (1.0 - inner_hi) + outer_lo + outer_hi
    };
    SoftSize {
        var_x: ((sxx / sw) - mx * mx).max(0.0) as f32,
        var_y: ((syy / sw) - my * my).max(0.0) as f32,
        w: edge(&col_cov, bx0, bx1, x0).max(0.0) as f32,
        h: edge(&row_cov, by0, by1, y0).max(0.0) as f32,
        area: sw as f32,
    }
}

/// Half-width of the solid rect whose coverage-weighted pixel-centre
/// variance is `var`: sampling at centres loses the 1/12 px² of spread
/// inside each pixel, so `W² / 12 = var + 1/12`.
pub fn soft_rect_half(var: f32) -> f32 {
    (3.0 * var + 0.25).sqrt()
}

/// Semi-axis of the solid ellipse with coverage-weighted variance `var`
/// (`a² / 4 = var + 1/12`).
pub fn soft_ellipse_semi(var: f32) -> f32 {
    2.0 * (var + 1.0 / 12.0).sqrt()
}

/// Principal-axis orientation (degrees, (-90, 90]) and the pixel-position
/// variances along and across it, from raw central-moment sums.
fn principal_axes(area: u32, mu20: f64, mu02: f64, mu11: f64) -> (f32, f32, f32) {
    let n = (area as f64).max(1.0);
    let (a, b, c) = (mu20 / n, mu02 / n, mu11 / n);
    let theta = 0.5 * (2.0 * c).atan2(a - b);
    let mid = 0.5 * (a + b);
    let rad = (0.25 * (a - b) * (a - b) + c * c).sqrt();
    let mut deg = theta.to_degrees() as f32;
    if deg <= -90.0 {
        deg += 180.0;
    }
    (deg, (mid + rad) as f32, (mid - rad).max(0.0) as f32)
}

/// An oriented rect and an oriented ellipse sized from a blob's second
/// moments, each scored by IoU against its pixels.
struct OrientedFit {
    iou_rect: f32,
    iou_ellipse: f32,
}

/// Half-lengths of the solid rect with the given pixel-position variance
/// along an axis: a run of W pixel centres has variance (W² − 1)/12.
pub fn rect_half(var: f32) -> f32 {
    (3.0 * var + 0.25).sqrt()
}

/// Semi-axis of the solid ellipse with the given variance along an axis
/// (a²/4 for a continuous ellipse).
pub fn ellipse_semi(var: f32) -> f32 {
    2.0 * var.max(0.0).sqrt()
}

fn oriented_fit(labels: &[u32], w: usize, h: usize, l: u32, a: &BlobAcc) -> Option<OrientedFit> {
    let (deg, v1, v2) = principal_axes(a.area, a.mu20, a.mu02, a.mu11);
    // Orientation only exists for elongated shapes, and a fit within a
    // degree of the axes is the axis-aligned fit already scored.
    if (v1 / v2.max(1e-3)).sqrt() < ORIENTED_MIN_ELONGATION || deg.abs() < 1.0 || deg.abs() > 89.0 {
        return None;
    }
    let th = (deg as f64).to_radians();
    let (cs, sn) = (th.cos(), th.sin());
    // Pixel (x, y) covers [x, x+1); its centre is x + 0.5, and the
    // centroid of index positions sits half a pixel left of that.
    let (cx, cy) = (a.cx as f64 + 0.5, a.cy as f64 + 0.5);
    let score = |h0: f64, h1: f64, ellipse: bool| -> f32 {
        let ex = (h0 * cs).abs() + (h1 * sn).abs();
        let ey = (h0 * sn).abs() + (h1 * cs).abs();
        let x0 = (cx - ex).floor().max(0.0) as usize;
        let x1 = ((cx + ex).ceil() as usize).min(w.saturating_sub(1));
        let y0 = (cy - ey).floor().max(0.0) as usize;
        let y1 = ((cy + ey).ceil() as usize).min(h.saturating_sub(1));
        let (mut total, mut hit) = (0u32, 0u32);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f64 + 0.5 - cx;
                let dy = y as f64 + 0.5 - cy;
                let u = dx * cs + dy * sn;
                let v = -dx * sn + dy * cs;
                let inside = if ellipse {
                    (u / h0).powi(2) + (v / h1).powi(2) <= 1.0
                } else {
                    u.abs() <= h0 && v.abs() <= h1
                };
                if inside {
                    total += 1;
                    if labels[y * w + x] == l {
                        hit += 1;
                    }
                }
            }
        }
        let union = a.area + total - hit;
        if union == 0 {
            0.0
        } else {
            hit as f32 / union as f32
        }
    };
    Some(OrientedFit {
        iou_rect: score(rect_half(v1) as f64, rect_half(v2) as f64, false),
        iou_ellipse: score(ellipse_semi(v1) as f64, ellipse_semi(v2) as f64, true),
    })
}

/// Compute the 7 Hu moment invariants from a blob's central moments.
/// Given `m00` (= area) and central moments μ₂₀ μ₀₂ μ₁₁ μ₃₀ μ₀₃ μ₂₁ μ₁₂.
fn hu_moments(
    m00: f64,
    mu20: f64,
    mu02: f64,
    mu11: f64,
    mu30: f64,
    mu03: f64,
    mu21: f64,
    mu12: f64,
) -> [f32; 7] {
    if m00 <= 0.0 {
        return [0.0; 7];
    }
    // Normalised central moments η_pq = μ_pq / m00^((p+q)/2 + 1).
    let n2 = m00 * m00; // m00^2 (used for p+q=2)
    let n_pq2 = n2;
    // For p+q = 3 → m00^((3/2)+1) = m00^2.5
    let n_pq3 = m00.powf(2.5);

    let n20 = mu20 / n_pq2;
    let n02 = mu02 / n_pq2;
    let n11 = mu11 / n_pq2;
    let n30 = mu30 / n_pq3;
    let n03 = mu03 / n_pq3;
    let n21 = mu21 / n_pq3;
    let n12 = mu12 / n_pq3;

    let h1 = n20 + n02;
    let h2 = (n20 - n02).powi(2) + 4.0 * n11.powi(2);
    let h3 = (n30 - 3.0 * n12).powi(2) + (3.0 * n21 - n03).powi(2);
    let h4 = (n30 + n12).powi(2) + (n21 + n03).powi(2);
    let h5 = (n30 - 3.0 * n12) * (n30 + n12) * ((n30 + n12).powi(2) - 3.0 * (n21 + n03).powi(2))
        + (3.0 * n21 - n03)
            * (n21 + n03)
            * (3.0 * (n30 + n12).powi(2) - (n21 + n03).powi(2));
    let h6 = (n20 - n02) * ((n30 + n12).powi(2) - (n21 + n03).powi(2))
        + 4.0 * n11 * (n30 + n12) * (n21 + n03);
    let h7 = (3.0 * n21 - n03) * (n30 + n12) * ((n30 + n12).powi(2) - 3.0 * (n21 + n03).powi(2))
        - (n30 - 3.0 * n12)
            * (n21 + n03)
            * (3.0 * (n30 + n12).powi(2) - (n21 + n03).powi(2));

    [
        h1 as f32, h2 as f32, h3 as f32, h4 as f32, h5 as f32, h6 as f32, h7 as f32,
    ]
}

struct BlobAcc {
    area: u32,
    cx: f32,
    cy: f32,
    hu: [f32; 7],
    m00: f64,
    m10: f64,
    m01: f64,
    min_x: u32,
    max_x: u32,
    min_y: u32,
    max_y: u32,
    all_r: u64,
    all_g: u64,
    all_b: u64,
    int_r: u64,
    int_g: u64,
    int_b: u64,
    int_count: u64,
    /// Lattice points of the bbox-inscribed ellipse, and how many of them
    /// belong to the blob — the ellipse's overlap with the actual pixels.
    ell_total: u32,
    ell_hit: u32,
    mu20: f64,
    mu02: f64,
    mu11: f64,
}

impl BlobAcc {
    fn new() -> Self {
        Self {
            area: 0,
            cx: 0.0,
            cy: 0.0,
            hu: [0.0; 7],
            m00: 0.0,
            m10: 0.0,
            m01: 0.0,
            min_x: u32::MAX,
            max_x: 0,
            min_y: u32::MAX,
            max_y: 0,
            all_r: 0,
            all_g: 0,
            all_b: 0,
            int_r: 0,
            int_g: 0,
            int_b: 0,
            int_count: 0,
            ell_total: 0,
            ell_hit: 0,
            mu20: 0.0,
            mu02: 0.0,
            mu11: 0.0,
        }
    }
}

// --------------------------------------------------------------------
// Connected components (4-neighbourhood, two-pass union-find)
// --------------------------------------------------------------------

/// v0.13: label *regions*, not components.
///
/// Through v0.12 a blob was a connected component of the foreground
/// mask, which is colour-blind: touching regions of different colours —
/// the six bars of `testsrc2`, a red card on a green panel — fused into
/// one blob with one averaged colour. Now two 4-adjacent foreground
/// pixels share a label only when their colours are within
/// `REGION_JOIN_SQ` of each other.
///
/// Neighbour-to-neighbour joining chains through smooth gradients (a
/// shaded ball stays one region), and splits at edges. What it would
/// also do is turn every anti-aliased edge into a fringe of thin
/// in-between regions, so a second pass hands each thin region — one
/// with no pixel whose four neighbours all share its label — to the
/// solid region it shares the most border with. A thin region touching
/// no solid region (a hairline on background) keeps its own label.
fn label_regions(f: &VideoFrame, mask: &[bool], w: usize, h: usize) -> Vec<u32> {
    let close = |a: usize, b: usize| {
        let d = |c: usize| f.pixels[a * 4 + c] as i32 - f.pixels[b * 4 + c] as i32;
        (d(0) * d(0) + d(1) * d(1) + d(2) * d(2)) as u32 <= REGION_JOIN_SQ
    };
    let mut labels = vec![0u32; w * h];
    let mut uf = UnionFind::new();
    let mut next: u32 = 1;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !mask[i] {
                continue;
            }
            let left = if x > 0 && mask[i - 1] && close(i, i - 1) {
                labels[i - 1]
            } else {
                0
            };
            let up = if y > 0 && mask[i - w] && close(i, i - w) {
                labels[i - w]
            } else {
                0
            };
            match (left, up) {
                (0, 0) => {
                    labels[i] = next;
                    uf.make_set(next);
                    next += 1;
                }
                (a, 0) => labels[i] = a,
                (0, b) => labels[i] = b,
                (a, b) => {
                    labels[i] = a.min(b);
                    if a != b {
                        uf.union(a, b);
                    }
                }
            }
        }
    }
    for l in labels.iter_mut() {
        if *l != 0 {
            *l = uf.find(*l);
        }
    }
    // A codec-blurred edge is a fringe several regions deep: the outer
    // band touches only the inner band, which is thin too. Repeat until
    // nothing moves (bounded — each pass only ever reduces the number of
    // thin regions).
    // A codec-blurred edge is a fringe several regions deep: the outer
    // band touches only the inner band, which is thin too. Repeat until
    // nothing moves (bounded — each pass only ever reduces the number of
    // thin regions).
    for _ in 0..ABSORB_MAX_PASSES {
        if !absorb_thin_regions(&mut labels, w, h) {
            break;
        }
    }
    labels
}

/// One fringe-absorption pass: every thin region (no pixel whose four
/// neighbours all share its label) joins the solid region it shares the
/// most border with. Returns whether anything changed.
///
/// A stricter rule — absorb only when the thin region's colour is a
/// blend of its two sides — was tried and measured worse: H.264 chroma
/// subsampling shifts fringe colours off the blend line, so codec edges
/// stopped being absorbed and `vac_control` regrew 16 fringe shapes.
fn absorb_thin_regions(labels: &mut [u32], w: usize, h: usize) -> bool {
    use std::collections::HashMap;
    let same_all_4 = |labels: &[u32], i: usize| {
        let (x, y) = (i % w, i / w);
        let l = labels[i];
        (x == 0 || labels[i - 1] == l)
            && (x + 1 == w || labels[i + 1] == l)
            && (y == 0 || labels[i - w] == l)
            && (y + 1 == h || labels[i + w] == l)
    };
    let mut solid: HashMap<u32, bool> = HashMap::new();
    for i in 0..w * h {
        let l = labels[i];
        if l == 0 {
            continue;
        }
        let e = solid.entry(l).or_insert(false);
        if !*e && same_all_4(labels, i) {
            *e = true;
        }
    }
    let mut border: HashMap<(u32, u32), u32> = HashMap::new();
    for i in 0..w * h {
        let l = labels[i];
        if l == 0 || solid[&l] {
            continue;
        }
        let (x, y) = (i % w, i / w);
        let mut visit = |j: usize| {
            let m = labels[j];
            if m != 0 && m != l && solid[&m] {
                *border.entry((l, m)).or_insert(0) += 1;
            }
        };
        if x > 0 { visit(i - 1); }
        if x + 1 < w { visit(i + 1); }
        if y > 0 { visit(i - w); }
        if y + 1 < h { visit(i + w); }
    }
    let mut target: HashMap<u32, (u32, u32)> = HashMap::new();
    for (&(l, m), &n) in &border {
        let e = target.entry(l).or_insert((m, n));
        if n > e.1 || (n == e.1 && m < e.0) {
            *e = (m, n);
        }
    }
    if target.is_empty() {
        return false;
    }
    for l in labels.iter_mut() {
        if let Some(&(m, _)) = target.get(l) {
            *l = m;
        }
    }
    true
}

fn label_components_4(mask: &[bool], w: usize, h: usize) -> Vec<u32> {
    let mut labels = vec![0u32; w * h];
    let mut uf = UnionFind::new();
    let mut next: u32 = 1;

    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !mask[i] {
                continue;
            }
            let left = if x > 0 && mask[i - 1] { labels[i - 1] } else { 0 };
            let up = if y > 0 && mask[i - w] { labels[i - w] } else { 0 };
            match (left, up) {
                (0, 0) => {
                    labels[i] = next;
                    uf.make_set(next);
                    next += 1;
                }
                (a, 0) => labels[i] = a,
                (0, b) => labels[i] = b,
                (a, b) => {
                    let m = a.min(b);
                    labels[i] = m;
                    if a != b {
                        uf.union(a, b);
                    }
                }
            }
        }
    }
    for l in labels.iter_mut() {
        if *l != 0 {
            *l = uf.find(*l);
        }
    }
    labels
}

struct UnionFind {
    parent: Vec<u32>,
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::VideoFrame;

    /// Build a synthetic RGBA frame painted entirely in `bg`, then fill a
    /// region selected by `mask(x, y)` with `fg`.
    fn synth_frame(w: u32, h: u32, bg: (u8, u8, u8), fg: (u8, u8, u8), mask: impl Fn(u32, u32) -> bool) -> VideoFrame {
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let (r, g, b) = if mask(x, y) { fg } else { bg };
                pixels[i] = r;
                pixels[i + 1] = g;
                pixels[i + 2] = b;
                pixels[i + 3] = 255;
            }
        }
        VideoFrame {
            width: w,
            height: h,
            pixels,
            delay_ms: 33,
        }
    }

    #[test]
    fn classifier_recognises_filled_rectangle_as_rect() {
        // 50×30 rect at (60, 40) on a uniform dark background.
        let f = synth_frame(160, 120, (20, 20, 20), (255, 0, 0), |x, y| {
            (60..110).contains(&x) && (40..70).contains(&y)
        });
        let blobs = detect_blobs(&f, Color::rgb(20, 20, 20), Segmentation::Components);
        assert_eq!(blobs.len(), 1, "expected exactly one blob, got {blobs:?}");
        let b = blobs[0];
        assert_eq!(b.kind, ShapeKind::Rect, "fill_ratio={}", b.fill_ratio);
        assert!(b.fill_ratio > 0.99, "fill_ratio={}", b.fill_ratio);
        // bbox is 50×30 so rx=25, ry=15. Aspect ratio recovered.
        assert!((b.rx - 25.0).abs() < 1.0, "rx={}", b.rx);
        assert!((b.ry - 15.0).abs() < 1.0, "ry={}", b.ry);
    }

    #[test]
    fn classifier_recognises_filled_disk_as_ellipse() {
        // Disk of radius 30 centered at (80, 60).
        let f = synth_frame(160, 120, (20, 20, 20), (255, 0, 0), |x, y| {
            let dx = x as i32 - 80;
            let dy = y as i32 - 60;
            dx * dx + dy * dy <= 30 * 30
        });
        let blobs = detect_blobs(&f, Color::rgb(20, 20, 20), Segmentation::Components);
        assert_eq!(blobs.len(), 1, "expected exactly one blob");
        let b = blobs[0];
        assert_eq!(b.kind, ShapeKind::Ellipse, "fill_ratio={}", b.fill_ratio);
        // π/4 ≈ 0.785; pixel rasterisation drifts but stays well below 0.90.
        assert!(b.fill_ratio < 0.85, "fill_ratio={}", b.fill_ratio);
        assert!(b.fill_ratio > 0.70, "fill_ratio={}", b.fill_ratio);
    }

    #[test]
    fn classifier_recognises_wide_ellipse_with_independent_rx_ry() {
        // Ellipse rx=40, ry=15 centered at (80, 60).
        let f = synth_frame(160, 120, (20, 20, 20), (0, 255, 0), |x, y| {
            let dx = (x as f32 - 80.0) / 40.0;
            let dy = (y as f32 - 60.0) / 15.0;
            dx * dx + dy * dy <= 1.0
        });
        let blobs = detect_blobs(&f, Color::rgb(20, 20, 20), Segmentation::Components);
        assert_eq!(blobs.len(), 1);
        let b = blobs[0];
        assert_eq!(b.kind, ShapeKind::Ellipse);
        assert!((b.rx - 40.0).abs() <= 1.0, "rx={}", b.rx);
        assert!((b.ry - 15.0).abs() <= 1.0, "ry={}", b.ry);
    }

    #[test]
    fn classifier_keeps_two_rects_separate() {
        // Two disjoint rectangles → two Rect blobs.
        let f = synth_frame(200, 100, (10, 10, 10), (255, 200, 0), |x, y| {
            ((20..60).contains(&x) && (20..70).contains(&y))
                || ((130..170).contains(&x) && (20..70).contains(&y))
        });
        let blobs = detect_blobs(&f, Color::rgb(10, 10, 10), Segmentation::Components);
        assert_eq!(blobs.len(), 2);
        for b in &blobs {
            assert_eq!(b.kind, ShapeKind::Rect, "fill_ratio={}", b.fill_ratio);
        }
    }

    /// v0.6 — a filled triangle fills ~0.5 of its bbox: far below the
    /// ellipse band, so it must classify as `Path`, and its traced +
    /// RDP-simplified contour must come back as ~3 vertices.
    #[test]
    fn classifier_recognises_triangle_as_path_and_traces_contour() {
        // Apex (80, 20), base y=90 spanning x=30..130.
        let inside = |x: u32, y: u32| -> bool {
            if !(20..=90).contains(&y) {
                return false;
            }
            let spread = (y as f32 - 20.0) * 50.0 / 70.0;
            (x as f32) >= 80.0 - spread && (x as f32) <= 80.0 + spread
        };
        let f = synth_frame(160, 120, (20, 20, 20), (0, 200, 255), inside);
        let bg = Color::rgb(20, 20, 20);

        let blobs = detect_blobs(&f, bg, Segmentation::Components);
        assert_eq!(blobs.len(), 1, "expected one blob, got {blobs:?}");
        let b = blobs[0];
        assert_eq!(b.kind, ShapeKind::Path, "fill_ratio={}", b.fill_ratio);
        assert!(
            b.fill_ratio > 0.40 && b.fill_ratio < ELLIPSE_MIN_FILL,
            "triangle fill_ratio should sit near 0.5, got {}",
            b.fill_ratio
        );

        let contour = extract_contour(&f, bg, (b.cx, b.cy), b.rx.max(b.ry), (b.area / 4).max(1), Segmentation::Components)
            .expect("triangle contour");
        let simplified = pac_decompiler::simplify::rdp(&contour, 2.0);
        assert!(
            (3..=6).contains(&simplified.len()),
            "expected ~3 corners after RDP, got {}: {simplified:?}",
            simplified.len()
        );
    }

    /// Like `synth_frame`, but pixels where `opaque(x, y)` is false are
    /// written as rgba(0, 0, 0, 0) — exactly what the GIF decoder
    /// leaves in transparent regions.
    fn synth_frame_transparent(
        w: u32,
        h: u32,
        fg: (u8, u8, u8),
        opaque: impl Fn(u32, u32) -> bool,
    ) -> VideoFrame {
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                if opaque(x, y) {
                    let i = ((y * w + x) * 4) as usize;
                    pixels[i] = fg.0;
                    pixels[i + 1] = fg.1;
                    pixels[i + 2] = fg.2;
                    pixels[i + 3] = 255;
                }
            }
        }
        VideoFrame {
            width: w,
            height: h,
            pixels,
            delay_ms: 33,
        }
    }

    /// v0.5.2 — a fully-transparent canvas has no measurable background;
    /// the estimator must fall back to white (the viewer compositing
    /// default), not report the decoder's rgba(0,0,0,0) fill as black.
    #[test]
    fn transparent_corners_fall_back_to_white_background() {
        let f = synth_frame_transparent(32, 32, (60, 60, 60), |x, y| {
            // Opaque dark disk in the middle; corners fully transparent.
            let dx = x as i32 - 16;
            let dy = y as i32 - 16;
            dx * dx + dy * dy <= 6 * 6
        });
        let bg = estimate_background(&f);
        assert_eq!(
            (bg.r, bg.g, bg.b),
            (255, 255, 255),
            "transparent corners should fall back to white, got {bg:?}"
        );
    }

    /// v0.5.2 — opaque corner pixels must win over transparent ones in
    /// the same corner block instead of being averaged with black.
    #[test]
    fn background_estimation_ignores_transparent_pixels_in_corner_blocks() {
        // Every even pixel is opaque mid-grey, every odd pixel is
        // transparent — a 50% dither. The estimate must be the grey,
        // not grey averaged toward rgba(0,0,0,0).
        let f = synth_frame_transparent(32, 32, (200, 200, 200), |x, y| (x + y) % 2 == 0);
        let bg = estimate_background(&f);
        assert_eq!(
            (bg.r, bg.g, bg.b),
            (200, 200, 200),
            "transparent pixels leaked into the estimate: {bg:?}"
        );
    }

    /// v0.5.2 — transparent pixels are background by definition. With a
    /// white estimated background, the transparent canvas (decoded as
    /// black) must NOT become one giant foreground blob; only the
    /// opaque tick is a blob.
    #[test]
    fn transparent_canvas_does_not_become_a_foreground_blob() {
        let f = synth_frame_transparent(32, 32, (60, 60, 60), |x, y| {
            (14..18).contains(&x) && (4..12).contains(&y)
        });
        let blobs = detect_blobs(&f, Color::rgb(255, 255, 255), Segmentation::Components);
        assert_eq!(
            blobs.len(),
            1,
            "expected only the opaque tick as a blob, got {blobs:?}"
        );
        let b = blobs[0];
        assert!((b.cx - 15.5).abs() <= 1.0, "cx={}", b.cx);
        assert!((b.cy - 7.5).abs() <= 1.0, "cy={}", b.cy);
        assert_eq!((b.color.r, b.color.g, b.color.b), (60, 60, 60));
    }
}

impl UnionFind {
    fn new() -> Self {
        UnionFind { parent: vec![0] }
    }
    fn make_set(&mut self, x: u32) {
        while self.parent.len() <= x as usize {
            self.parent.push(self.parent.len() as u32);
        }
        self.parent[x as usize] = x;
    }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let p = self.parent[x as usize];
            let gp = self.parent[p as usize];
            self.parent[x as usize] = gp;
            x = gp;
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            if ra < rb {
                self.parent[rb as usize] = ra;
            } else {
                self.parent[ra as usize] = rb;
            }
        }
    }
}

#[cfg(test)]
mod region_tests {
    //! v0.13: colour-aware segmentation and the border background rule.
    use super::*;

    /// A frame painted by `paint(x, y) -> Option<rgb>`, background `bg`.
    fn frame(w: u32, h: u32, bg: [u8; 3], paint: impl Fn(u32, u32) -> Option<[u8; 3]>) -> VideoFrame {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let c = paint(x, y).unwrap_or(bg);
                pixels.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        VideoFrame {
            width: w,
            height: h,
            pixels,
            delay_ms: 33,
        }
    }

    const DARK: [u8; 3] = [20, 20, 28];
    const RED: [u8; 3] = [224, 72, 72];
    const GREEN: [u8; 3] = [72, 192, 96];

    #[test]
    fn regions_split_touching_colours_where_components_fuse_them() {
        // A red block and a green block sharing an edge.
        let f = frame(80, 50, DARK, |x, y| match (x, y) {
            (10..=39, 10..=39) => Some(RED),
            (40..=69, 10..=39) => Some(GREEN),
            _ => None,
        });
        let bg = Color::rgb(DARK[0], DARK[1], DARK[2]);
        let fused = detect_blobs(&f, bg, Segmentation::Components);
        assert_eq!(fused.len(), 1, "components are colour-blind");

        let mut split = detect_blobs(&f, bg, Segmentation::Regions);
        split.sort_by(|a, b| a.cx.partial_cmp(&b.cx).unwrap());
        assert_eq!(split.len(), 2, "regions separate the two colours");
        assert_eq!((split[0].color.r, split[0].color.g), (RED[0], RED[1]));
        assert_eq!((split[1].color.r, split[1].color.g), (GREEN[0], GREEN[1]));
        assert_eq!(split[0].area, 900);
        assert_eq!(split[1].area, 900);
    }

    #[test]
    fn an_antialiased_fringe_is_absorbed_not_emitted() {
        // A red disc with a one-pixel ring of half-blended colour — what a
        // rasteriser leaves at the edge.
        let blend = [
            ((RED[0] as u16 + DARK[0] as u16) / 2) as u8,
            ((RED[1] as u16 + DARK[1] as u16) / 2) as u8,
            ((RED[2] as u16 + DARK[2] as u16) / 2) as u8,
        ];
        let f = frame(64, 64, DARK, |x, y| {
            let d = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
            if d <= 15.0 {
                Some(RED)
            } else if d <= 16.0 {
                Some(blend)
            } else {
                None
            }
        });
        let blobs = detect_blobs(&f, Color::rgb(DARK[0], DARK[1], DARK[2]), Segmentation::Regions);
        assert_eq!(blobs.len(), 1, "the fringe must join the disc: {blobs:?}");
    }

    #[test]
    fn a_smooth_gradient_stays_one_region() {
        // 120 px from red to blue: each step is far below the join
        // threshold even though the ends are completely different.
        let f = frame(140, 30, DARK, |x, y| {
            (10..130).contains(&x).then(|| {
                let t = (x - 10) as f32 / 119.0;
                let _ = y;
                [
                    (220.0 * (1.0 - t) + 40.0 * t) as u8,
                    60,
                    (40.0 * (1.0 - t) + 220.0 * t) as u8,
                ]
            })
        });
        let blobs = detect_blobs(&f, Color::rgb(DARK[0], DARK[1], DARK[2]), Segmentation::Regions);
        assert_eq!(blobs.len(), 1, "{blobs:?}");
    }

    #[test]
    fn the_border_rule_picks_the_dominant_border_colour() {
        // Top band red, the rest dark. Two corners are red and two dark,
        // so the per-channel median lands on red: a colour that is in the
        // frame, but covers a third of it and none of its lower border.
        let f = frame(100, 60, DARK, |_, y| (y < 20).then_some(RED));
        let corners = estimate_background(&f);
        let border = estimate_background_border(&f);
        assert_eq!((corners.r, corners.g, corners.b), (RED[0], RED[1], RED[2]));
        assert_eq!((border.r, border.g, border.b), (DARK[0], DARK[1], DARK[2]));
    }

    #[test]
    fn the_corner_median_can_name_a_colour_absent_from_the_frame() {
        // Top-left red, top-right green, rest dark: the channels median
        // independently to (72, 72, 72), which appears nowhere.
        let f = frame(100, 60, DARK, |x, y| match (x < 50, y < 20) {
            (true, true) => Some(RED),
            (false, true) => Some(GREEN),
            _ => None,
        });
        let present = |c: Color| {
            f.pixels
                .chunks(4)
                .any(|p| p[0] == c.r && p[1] == c.g && p[2] == c.b)
        };
        let corners = estimate_background(&f);
        let border = estimate_background_border(&f);
        assert!(!present(corners), "corner median {corners:?} is not in the frame");
        assert!(present(border));
        assert_eq!((border.r, border.g, border.b), (DARK[0], DARK[1], DARK[2]));
    }

    #[test]
    fn a_transparent_border_means_a_white_background() {
        // Light ticks touching the edge of an otherwise transparent canvas
        // (the `real_loader` spinner) must not be voted the background.
        let mut f = frame(40, 40, [0, 0, 0], |x, y| {
            ((18..22).contains(&x) && y < 12).then_some([230, 230, 230])
        });
        for p in f.pixels.chunks_mut(4) {
            if p[0] == 0 {
                p[3] = 0;
            }
        }
        let bg = estimate_background_border(&f);
        assert_eq!((bg.r, bg.g, bg.b), (255, 255, 255));
    }
}

#[cfg(test)]
mod orientation_tests {
    //! v0.14: principal axes and oriented primitive fits.
    use super::*;

    /// A 60×8 bar centred at (50, 50) rotated by `deg` (clockwise, screen
    /// coordinates), sampled at pixel centres.
    fn bar(deg: f32) -> VideoFrame {
        let (w, h) = (100u32, 100u32);
        let th = deg.to_radians();
        let (cs, sn) = (th.cos(), th.sin());
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f32 + 0.5 - 50.0, y as f32 + 0.5 - 50.0);
                let u = dx * cs + dy * sn;
                let v = -dx * sn + dy * cs;
                let inside = u.abs() <= 30.0 && v.abs() <= 4.0;
                let c = if inside { [240, 192, 64] } else { [16, 20, 31] };
                pixels.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        VideoFrame {
            width: w,
            height: h,
            pixels,
            delay_ms: 33,
        }
    }

    #[test]
    fn a_rotated_bar_is_fitted_as_a_rotated_rect() {
        for deg in [30.0f32, -50.0, 70.0] {
            let blobs = detect_blobs(&bar(deg), Color::rgb(16, 20, 31), Segmentation::Components);
            assert_eq!(blobs.len(), 1);
            let b = blobs[0];
            assert_eq!(b.kind, ShapeKind::Rect, "{deg}°: {b:?}");
            assert!((b.angle_deg - deg).abs() < 2.0, "{deg}°: fitted {}", b.angle_deg);
            let len = 2.0 * rect_half(b.var_major);
            let thick = 2.0 * rect_half(b.var_minor);
            assert!((len - 60.0).abs() < 3.0, "{deg}°: length {len}");
            assert!((thick - 8.0).abs() < 2.0, "{deg}°: thickness {thick}");
        }
    }

    #[test]
    fn an_axis_aligned_bar_keeps_an_axis_aligned_fit() {
        let blobs = detect_blobs(&bar(0.0), Color::rgb(16, 20, 31), Segmentation::Components);
        assert_eq!(blobs[0].kind, ShapeKind::Rect);
        assert_eq!(blobs[0].angle_deg, 0.0);
    }

    #[test]
    fn a_disc_has_no_orientation_to_fit() {
        let (w, h) = (60u32, 60u32);
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let d = ((x as f32 + 0.5 - 30.0).powi(2) + (y as f32 + 0.5 - 30.0).powi(2)).sqrt();
                let c = if d <= 20.0 { [200, 60, 60] } else { [10, 10, 10] };
                pixels.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        let f = VideoFrame { width: w, height: h, pixels, delay_ms: 33 };
        let b = detect_blobs(&f, Color::rgb(10, 10, 10), Segmentation::Components)[0];
        assert_eq!(b.kind, ShapeKind::Ellipse);
        assert_eq!(b.angle_deg, 0.0);
        assert!(b.elongation() < 1.05);
    }
}

#[cfg(test)]
mod soft_size_tests {
    //! v1.1: coverage-weighted sizing.
    use super::*;

    const BG: [f32; 3] = [20.0, 20.0, 28.0];
    const FG: [f32; 3] = [230.0, 80.0, 80.0];

    /// A frame where each pixel is `bg + cov(x, y)·(fg − bg)`.
    fn covered(w: u32, h: u32, cov: impl Fn(f32, f32) -> f32) -> VideoFrame {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let c = cov(x as f32, y as f32).clamp(0.0, 1.0);
                for k in 0..3 {
                    pixels.push((BG[k] + c * (FG[k] - BG[k])).round() as u8);
                }
                pixels.push(255);
            }
        }
        VideoFrame { width: w, height: h, pixels, delay_ms: 33 }
    }

    /// Exact area coverage of pixel [x, x+1) by the interval [a, b).
    fn overlap(x: f32, a: f32, b: f32) -> f32 {
        ((x + 1.0).min(b) - x.max(a)).max(0.0)
    }

    #[test]
    fn an_antialiased_rect_at_a_subpixel_offset_is_sized_exactly() {
        for off in [0.0f32, 0.3, 0.5, 0.8] {
            let (x0, x1, y0, y1) = (10.0 + off, 30.0 + off, 8.0, 22.0 + off);
            let f = covered(48, 32, |x, y| overlap(x, x0, x1) * overlap(y, y0, y1));
            let b = detect_blobs(&f, Color::rgb(20, 20, 28), Segmentation::Components)[0];
            assert!((b.soft_w - 20.0).abs() < 0.1, "offset {off}: soft width {}", b.soft_w);
            assert!((b.soft_h - (14.0 + off)).abs() < 0.1, "offset {off}: soft height {}", b.soft_h);
            // The thresholded bbox is what used to be declared.
            let bbox_w = 2.0 * b.rx;
            assert!(bbox_w >= 20.0, "offset {off}: bbox {bbox_w}");
        }
    }

    #[test]
    fn a_shaded_blob_declines_coverage_sizing() {
        // A radially shaded disc: its interior is not one flat colour, so
        // colour cannot be read as coverage.
        let f = {
            let (w, h) = (64u32, 64u32);
            let mut pixels = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    let d = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
                    let c = if d <= 20.0 {
                        let t = d / 20.0;
                        [(240.0 - 140.0 * t) as u8, (240.0 - 140.0 * t) as u8, (240.0 - 140.0 * t) as u8]
                    } else {
                        [10, 10, 10]
                    };
                    pixels.extend_from_slice(&[c[0], c[1], c[2], 255]);
                }
            }
            VideoFrame { width: w, height: h, pixels, delay_ms: 33 }
        };
        let b = detect_blobs(&f, Color::rgb(10, 10, 10), Segmentation::Components)[0];
        assert_eq!((b.soft_w, b.soft_h), (0.0, 0.0), "shaded disc must decline: {b:?}");
    }

    #[test]
    fn a_hard_edged_cap_keeps_its_bbox_extent() {
        // A 3×7 pixel-art tick with clipped corners, no anti-aliasing.
        let f = covered(16, 16, |x, y| {
            let (x, y) = (x as i32, y as i32);
            let inside = (5..8).contains(&x) && (4..11).contains(&y);
            let corner = (x == 5 || x == 7) && (y == 4 || y == 10);
            if inside && !corner { 1.0 } else { 0.0 }
        });
        let b = detect_blobs(&f, Color::rgb(20, 20, 28), Segmentation::Components)[0];
        assert!((b.soft_w - 3.0).abs() < 1e-3 && (b.soft_h - 7.0).abs() < 1e-3, "{} x {}", b.soft_w, b.soft_h);
    }
}

#[cfg(test)]
mod edge_region_tests {
    //! v1.5: edge-gated regions.
    use super::*;

    /// A grey disc with a long green line drawn across it, on white.
    fn disc_and_line() -> VideoFrame {
        let (w, h) = (100u32, 200u32);
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let in_line = (49..51).contains(&x);
                let d = ((x as f32 - 50.0).powi(2) + (y as f32 - 100.0).powi(2)).sqrt();
                let c = if in_line {
                    [0, 128, 0]
                } else if d <= 25.0 {
                    [128, 128, 128]
                } else {
                    [255, 255, 255]
                };
                pixels.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        VideoFrame { width: w, height: h, pixels, delay_ms: 33 }
    }

    #[test]
    fn a_line_crossing_a_shape_stays_its_own_region() {
        let f = disc_and_line();
        let white = Color::rgb(255, 255, 255);
        let pure_green = |b: &Blob| b.color.r < 20 && b.color.g > 110 && b.color.b < 20;
        let tall = |b: &Blob| b.ry >= 95.0;
        // Plain regions absorb any thin region touching a solid one: the
        // line (it cuts the disc in two) is swallowed by one half, which
        // becomes a tall blob of mixed grey-green — no blob is the line.
        let regions = detect_blobs(&f, white, Segmentation::Regions);
        assert!(!regions.iter().any(pure_green), "regions kept the line: {regions:?}");
        assert!(regions.iter().any(|b| tall(b) && !pure_green(b)), "{regions:?}");
        // Edges keep the line: it touches each half only where it
        // crosses, a small share of its own area, so it is no fringe.
        let edges = detect_blobs(&f, white, Segmentation::Edges);
        assert!(edges.iter().any(|b| pure_green(b) && tall(b)), "edges lost the line: {edges:?}");
        assert!(
            edges.iter().filter(|b| !pure_green(b)).all(|b| b.color.r > 110 && b.color.r == b.color.g),
            "disc pieces should stay pure grey: {edges:?}"
        );
    }

    #[test]
    fn edges_still_absorb_a_fringe_that_wraps_its_shape() {
        // A red disc with a one-pixel half-blended ring: the ring wraps the
        // disc, so it is absorbed exactly as under plain regions.
        let (w, h) = (64u32, 64u32);
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let d = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
                let c = if d <= 15.0 {
                    [224, 72, 72]
                } else if d <= 16.0 {
                    [122, 46, 50]
                } else {
                    [20, 20, 28]
                };
                pixels.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        let f = VideoFrame { width: w, height: h, pixels, delay_ms: 33 };
        let blobs = detect_blobs(&f, Color::rgb(20, 20, 28), Segmentation::Edges);
        assert_eq!(blobs.len(), 1, "{blobs:?}");
    }
}
