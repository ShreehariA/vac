//! Static-scene layering and occlusion reasoning (v0.8).
//!
//! Everything before this module treats a video as *one* layer: every
//! frame is masked against its background and whatever survives is a
//! moving object.  That is wrong for the most ordinary kind of footage
//! there is — a scene with furniture in it.  A dial's tick marks, a
//! pendulum's scale, the wires and end-posts of a Newton's cradle: they
//! never move, but the mover *passes in front of them*, so from the
//! tracker's point of view they blink out and back.  A blinking blob is
//! indistinguishable from a fast-moving one, and v0.7's velocity model
//! made it worse, not better: prediction happily chases a dash that
//! "jumped" 400 px because the arm uncovered a different dash.
//!
//! The cure is to measure the scene before tracking anything:
//!
//! 1. **Temporal median plate.**  Per pixel, the median colour over the
//!    clip.  A pixel showing furniture 80% of the time medians to the
//!    furniture; a pixel a ball crosses for six frames medians to the
//!    background.  No thresholds, no classification — just the middle
//!    value.
//! 2. **Agreement.**  For every plate pixel that reads as foreground,
//!    count the frames whose actual colour matches the plate.  This is
//!    the occlusion measurement: furniture briefly hidden by a mover
//!    agrees 75-100% of the time; an object that merely *rests* for
//!    half the clip agrees ~50% and is rejected.
//! 3. **Strip.**  In each frame, blank the agreeing pixels back to the
//!    background — but *only where they agree*.  Where a static pixel
//!    disagrees, something is standing in front of it, and those pixels
//!    stay exactly as they are so the occluder keeps its silhouette
//!    intact instead of being punched full of holes.
//!
//! What reaches the tracker afterwards is the moving content alone, and
//! the furniture is emitted once, up front, as plain static shapes.

use vac_format::Color;

use crate::detect::{estimate_background, foreground_mask, MIN_OPAQUE_ALPHA};
use crate::frames::VideoFrame;

/// Fewer frames than this and a temporal median is not a measurement,
/// it's an accident — skip layering entirely.
const MIN_LAYER_FRAMES: usize = 8;

/// Upper bound on how many frames the median plate is estimated from.
/// The median of 96 evenly-spaced frames and the median of 900 differ
/// nowhere that matters, and the cap keeps plate estimation linear in
/// canvas size rather than in clip length.  Agreement (step 2) always
/// uses *every* frame — that one is the occlusion measurement and has
/// to see every occluded frame to count it.
const MAX_PLATE_SAMPLES: usize = 96;

/// A plate pixel only joins the static layer when the frames actually
/// show the plate's colour there this often.  The median already
/// guarantees >50% per channel; the margin above it is what separates
/// *furniture briefly hidden by a mover* (agreement 0.75-1.0) from *a
/// mover that happens to rest in one spot for half the clip* (~0.5).
///
/// Both directions were measured on the wild corpus, and both fail in
/// a way you can point at:
///
/// - At **0.60** the swept traces of the double pendulum qualify as
///   scenery.  Stripping them punches holes through the rod that is
///   drawing them, and the rod fragments: 19 animate blocks become 44,
///   186 keyframes become 461.
/// - At **0.90** anything whose colour breathes is rejected.  The
///   loader GIF's ticks — always present, but shaded differently as the
///   highlight sweeps past — score ~0.8, so the layer comes back empty
///   and the output is byte-for-byte the v0.7 result.
const STATIC_MIN_AGREEMENT: f32 = 0.75;

/// Components smaller than this are never tested for bleeding; they are
/// below the blob detector's floor anyway.
const BLEED_MIN_COMPONENT_PX: usize = 16;
/// A frame "bleeds" when at least this fraction of a component's outer
/// ring shows the component's own colour.
const BLEED_RING_FRACTION: f32 = 0.5;
/// A component that bleeds in at least this fraction of frames is the
/// core of a moving or growing shape, not scenery.
const BLEED_MIN_FRAMES: f32 = 0.25;

/// Minimum `area / bbox area` for a component to be tested at all.
const BLEED_MIN_FILL: f32 = 0.6;
/// ...and shows plain background in at least this fraction.
const BLEED_MIN_CLEAR_FRAMES: f32 = 0.10;

/// Squared RGB distance within which a frame's pixel counts as "the
/// plate is visible here".  Deliberately looser than the foreground
/// threshold: codec noise on a static element must not read as an
/// occluder, because a false occlusion leaves furniture in the motion
/// pass, which is the failure this module exists to remove.
const STATIC_MATCH_TOL_SQ: u32 = 32 * 32 * 3;

/// The persistent scene measured across a whole clip.
#[derive(Debug, Clone)]
pub struct StaticLayer {
    pub width: u32,
    pub height: u32,
    /// Per-pixel temporal median RGBA of the clip.
    median: Vec<u8>,
    /// Background colour measured on the median plate.
    pub bg: Color,
    /// True where the plate is foreground *and* persistent.
    pub mask: Vec<bool>,
    /// Number of pixels in `mask` — the layer's footprint.
    pub area: usize,
    /// Indices of `mask`, ascending. Cached so the per-frame strip is a
    /// forward scan rather than a full-canvas walk.
    indices: Vec<usize>,
    delay_ms: u32,
}

impl StaticLayer {
    /// Measure the static scene.  Returns `None` when the clip is too
    /// short to median meaningfully, when frame sizes disagree, or when
    /// nothing at all is persistent — all of which mean "carry on as if
    /// v0.7", not "fail".
    pub fn measure(frames: &[VideoFrame]) -> Option<StaticLayer> {
        if frames.len() < MIN_LAYER_FRAMES {
            return None;
        }
        let width = frames[0].width;
        let height = frames[0].height;
        if frames
            .iter()
            .any(|f| f.width != width || f.height != height)
        {
            return None;
        }
        let n = width as usize * height as usize;
        if n == 0 {
            return None;
        }

        let median = temporal_median(frames, n);
        let plate = VideoFrame {
            width,
            height,
            pixels: median.clone(),
            delay_ms: frames[0].delay_ms,
        };
        let bg = estimate_background(&plate);
        let candidate = foreground_mask(&plate, bg);

        // Occlusion measurement: how often is each candidate pixel
        // actually showing what the plate says it shows?
        let cand_idx: Vec<usize> = (0..n).filter(|&i| candidate[i]).collect();
        if cand_idx.is_empty() {
            return None;
        }
        let mut agree = vec![0u32; cand_idx.len()];
        for f in frames {
            for (slot, &i) in cand_idx.iter().enumerate() {
                if plate_visible(&f.pixels, &median, i) {
                    agree[slot] += 1;
                }
            }
        }

        let need = (frames.len() as f32 * STATIC_MIN_AGREEMENT).ceil() as u32;
        let mut mask = vec![false; n];
        for (slot, &i) in cand_idx.iter().enumerate() {
            if agree[slot] >= need {
                mask[i] = true;
            }
        }

        reject_bleeding_cores(&mut mask, &median, frames, width, height, bg);

        // Note that the mask deliberately keeps persistent pixels too
        // small to become a `static_<n>` shape of their own.  Restricting
        // it to emittable components was tried and is worse: those
        // specks are below the blob detector's noise floor in every
        // frame anyway, and leaving them in the motion pass seeds
        // phantom tracks (the pendulum grows 16 extra shapes, the
        // Newton's cradle 10 extra animate blocks).  Stripping them is
        // the same noise floor applied one stage earlier.
        let indices: Vec<usize> = (0..n).filter(|&i| mask[i]).collect();
        if indices.is_empty() {
            return None;
        }

        Some(StaticLayer {
            width,
            height,
            median,
            bg,
            area: indices.len(),
            mask,
            indices,
            delay_ms: frames[0].delay_ms,
        })
    }

    /// The static scene as a single frame: persistent pixels in their
    /// measured colour, everything else flattened to the background.
    /// Feeding this to the ordinary blob detector recovers the scene's
    /// shapes with exactly the same measurements a moving object gets.
    pub fn plate(&self) -> VideoFrame {
        let mut pixels = self.median.clone();
        for i in 0..self.mask.len() {
            if self.mask[i] {
                continue;
            }
            pixels[i * 4] = self.bg.r;
            pixels[i * 4 + 1] = self.bg.g;
            pixels[i * 4 + 2] = self.bg.b;
            pixels[i * 4 + 3] = 255;
        }
        VideoFrame {
            width: self.width,
            height: self.height,
            pixels,
            delay_ms: self.delay_ms,
        }
    }

    /// Remove the static scene from every frame, in place.
    ///
    /// A static pixel is blanked to that frame's background **only in
    /// the frames where it is actually visible**.  In frames where it
    /// disagrees with the plate, something is in front of it; those
    /// pixels are left untouched so the occluder survives as one
    /// connected component instead of being cut in half by the
    /// furniture it happens to be crossing.
    pub fn strip(&self, frames: &mut [VideoFrame]) {
        for f in frames.iter_mut() {
            if f.width != self.width || f.height != self.height {
                continue;
            }
            // Measured before the frame is edited — the corner blocks
            // this samples may themselves be static pixels.
            let bg = estimate_background(f);
            for &i in &self.indices {
                if !plate_visible(&f.pixels, &self.median, i) {
                    continue;
                }
                f.pixels[i * 4] = bg.r;
                f.pixels[i * 4 + 1] = bg.g;
                f.pixels[i * 4 + 2] = bg.b;
                f.pixels[i * 4 + 3] = 255;
            }
        }
    }
}

/// Is pixel `i` of `frame` showing what the plate says is there?
/// v0.11: a persistent region is not scenery if the same colour keeps
/// spilling past its edge.
///
/// A growing or pulsing shape has a core that is covered in every frame,
/// so it passes the agreement test exactly like furniture does — and
/// stripping it leaves the motion pass a ring, which fragments into a
/// dozen tracks (`examples/grow.vac`: 10 shapes, ink IoU 0.644).
/// Furniture is different in one measurable way: its outline stays put.
/// So look two pixels outside each mask component (one pixel out is
/// anti-aliasing) and count the frames in which most of that ring shows
/// the component's own colour.  If that happens often, the region is
/// the core of something that moves or grows, and it goes back to the
/// motion pass whole — provided the ring is also plain background in
/// some frames (see the Mandelbrot note below).
fn reject_bleeding_cores(
    mask: &mut [bool],
    median: &[u8],
    frames: &[VideoFrame],
    width: u32,
    height: u32,
    bg: Color,
) {
    let (w, h) = (width as usize, height as usize);
    let mut label = vec![usize::MAX; w * h];
    let mut comps: Vec<Vec<usize>> = Vec::new();
    for start in 0..w * h {
        if !mask[start] || label[start] != usize::MAX {
            continue;
        }
        let id = comps.len();
        let mut stack = vec![start];
        let mut pix = Vec::new();
        label[start] = id;
        while let Some(i) = stack.pop() {
            pix.push(i);
            let (x, y) = (i % w, i / w);
            let mut push = |j: usize| {
                if mask[j] && label[j] == usize::MAX {
                    label[j] = id;
                    stack.push(j);
                }
            };
            if x > 0 { push(i - 1); }
            if x + 1 < w { push(i + 1); }
            if y > 0 { push(i - w); }
            if y + 1 < h { push(i + w); }
        }
        comps.push(pix);
    }

    for (id, pix) in comps.iter().enumerate() {
        if pix.len() < BLEED_MIN_COMPONENT_PX {
            continue;
        }
        // Only compact components are returned to the motion pass: that
        // is where it can track them as one shape.  Sprawling regions
        // (the Mandelbrot zoom's filaments) do genuinely grow, but the
        // motion pass shreds them — measured, handing them back cost
        // that clip 0.72 → 0.65 ink IoU.  They stay scenery.
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        for &i in pix {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let bbox = ((x1 - x0 + 1) * (y1 - y0 + 1)) as f32;
        if (pix.len() as f32) < bbox * BLEED_MIN_FILL {
            continue;
        }
        // Chebyshev distance 1 and 2 around the component.
        let near = |i: usize, r: usize| -> Vec<usize> {
            let (x, y) = (i % w, i / w);
            let mut out = Vec::new();
            for yy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                for xx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                    out.push(yy * w + xx);
                }
            }
            out
        };
        let mut d1 = std::collections::HashSet::new();
        for &i in pix {
            d1.extend(near(i, 1));
        }
        let mut ring = std::collections::BTreeSet::new();
        for &i in &d1 {
            for j in near(i, 1) {
                if label[j] != id && !d1.contains(&j) {
                    ring.insert(j);
                }
            }
        }
        if ring.is_empty() {
            continue;
        }
        let mut sum = [0u64; 3];
        for &i in pix {
            for c in 0..3 {
                sum[c] += median[i * 4 + c] as u64;
            }
        }
        let n = pix.len() as u64;
        let colour = [sum[0] / n, sum[1] / n, sum[2] / n].map(|v| v as u8);
        let same = |px: &[u8], i: usize| {
            let d = |c: usize| px[i * 4 + c] as i32 - colour[c] as i32;
            (d(0) * d(0) + d(1) * d(1) + d(2) * d(2)) as u32 <= STATIC_MATCH_TOL_SQ
        };
        let bleeding = frames
            .iter()
            .filter(|f| {
                let hits = ring.iter().filter(|&&i| same(&f.pixels, i)).count();
                hits as f32 >= ring.len() as f32 * BLEED_RING_FRACTION
            })
            .count();
        // The surround must also show plain background in some frames:
        // a grower's edge alternates between its own colour and the
        // background, whereas scenery embedded in similar-coloured
        // texture (the Mandelbrot zoom) is simply always surrounded.
        let bg_rgb = [bg.r, bg.g, bg.b];
        let is_bg = |px: &[u8], i: usize| {
            let d = |c: usize| px[i * 4 + c] as i32 - bg_rgb[c] as i32;
            (d(0) * d(0) + d(1) * d(1) + d(2) * d(2)) as u32 <= STATIC_MATCH_TOL_SQ
        };
        let clear = frames
            .iter()
            .filter(|f| {
                let hits = ring.iter().filter(|&&i| is_bg(&f.pixels, i)).count();
                hits as f32 >= ring.len() as f32 * BLEED_RING_FRACTION
            })
            .count();
        if bleeding as f32 >= frames.len() as f32 * BLEED_MIN_FRAMES
            && clear as f32 >= frames.len() as f32 * BLEED_MIN_CLEAR_FRAMES
        {
            for &i in pix {
                mask[i] = false;
            }
        }
    }
}

fn plate_visible(frame: &[u8], median: &[u8], i: usize) -> bool {
    let p = i * 4;
    if frame[p + 3] < MIN_OPAQUE_ALPHA {
        return false;
    }
    let dr = frame[p] as i32 - median[p] as i32;
    let dg = frame[p + 1] as i32 - median[p + 1] as i32;
    let db = frame[p + 2] as i32 - median[p + 2] as i32;
    (dr * dr + dg * dg + db * db) as u32 <= STATIC_MATCH_TOL_SQ
}

/// Per-pixel, per-channel temporal median over up to
/// `MAX_PLATE_SAMPLES` evenly-spaced frames.
///
/// Done in bands: a band of pixels is transposed out of every sampled
/// frame into one scratch buffer so each pixel's time series ends up
/// contiguous.  The naive loop (gather pixel `i` from every frame in
/// turn) strides across hundreds of megabytes per pixel and spends all
/// its time in cache misses; banding keeps the working set to a few
/// megabytes.
fn temporal_median(frames: &[VideoFrame], n: usize) -> Vec<u8> {
    let sampled: Vec<&VideoFrame> = if frames.len() <= MAX_PLATE_SAMPLES {
        frames.iter().collect()
    } else {
        (0..MAX_PLATE_SAMPLES)
            .map(|k| &frames[k * frames.len() / MAX_PLATE_SAMPLES])
            .collect()
    };
    let s = sampled.len();
    let mid = s / 2;

    // ~4 MiB of scratch, whatever the clip length.
    let band = ((4 << 20) / (4 * s)).clamp(64, n);
    let mut scratch = vec![0u8; band * 4 * s];
    let mut out = vec![0u8; n * 4];

    let mut start = 0usize;
    while start < n {
        let len = band.min(n - start);
        // scratch[(j * 4 + c) * s + k] = sampled[k].pixels[(start+j)*4 + c]
        for (k, f) in sampled.iter().enumerate() {
            let src = &f.pixels[start * 4..(start + len) * 4];
            for j in 0..len {
                let base = j * 4;
                scratch[(base) * s + k] = src[base];
                scratch[(base + 1) * s + k] = src[base + 1];
                scratch[(base + 2) * s + k] = src[base + 2];
                scratch[(base + 3) * s + k] = src[base + 3];
            }
        }
        for j in 0..len {
            for c in 0..4 {
                let slot = &mut scratch[(j * 4 + c) * s..(j * 4 + c) * s + s];
                let (_, m, _) = slot.select_nth_unstable(mid);
                out[(start + j) * 4 + c] = *m;
            }
        }
        start += len;
    }
    out
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// `bar` paints a static vertical bar; `blocker` paints a mover.
    fn scene_frame(
        w: u32,
        h: u32,
        bg: (u8, u8, u8),
        parts: &[((u8, u8, u8), Box<dyn Fn(u32, u32) -> bool + '_>)],
    ) -> VideoFrame {
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let mut c = bg;
                for (col, f) in parts {
                    if f(x, y) {
                        c = *col;
                    }
                }
                pixels[i] = c.0;
                pixels[i + 1] = c.1;
                pixels[i + 2] = c.2;
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

    /// A static tick mark that a mover crosses must stay in the plate:
    /// occlusion is not motion.  And the frames the tracker sees must
    /// come back with the tick removed but the mover intact.
    #[test]
    fn occluded_static_mark_stays_in_the_layer_and_leaves_the_mover_whole() {
        let w = 120u32;
        let h = 60u32;
        // Tick: a 6-px-wide bar at x=58..64, full height.
        let tick = |x: u32, _y: u32| (58..64).contains(&x);
        let frames: Vec<VideoFrame> = (0..20u32)
            .map(|i| {
                // Mover: a 20×20 block sweeping left → right, crossing
                // the tick around i = 10.
                let mx = 5 + i * 5;
                let mover = move |x: u32, y: u32| {
                    (mx..mx + 20).contains(&x) && (20..40).contains(&y)
                };
                scene_frame(
                    w,
                    h,
                    (20, 20, 20),
                    &[
                        ((200, 200, 0), Box::new(tick)),
                        ((0, 120, 255), Box::new(mover)),
                    ],
                )
            })
            .collect();

        let layer = StaticLayer::measure(&frames).expect("tick should be a static layer");
        // The tick is 6 × 60 = 360 px; the mover is never persistent.
        assert!(
            (300..=420).contains(&layer.area),
            "layer should be the tick alone, got {} px",
            layer.area
        );
        for y in 0..h {
            assert!(layer.mask[(y * w + 60) as usize], "tick column missing at y={y}");
            assert!(!layer.mask[(y * w + 10) as usize], "mover leaked in at y={y}");
        }

        // Frame 10: the mover is on top of the tick.
        let mut stripped = frames.clone();
        layer.strip(&mut stripped);
        let f = &stripped[10];
        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            (f.pixels[i], f.pixels[i + 1], f.pixels[i + 2])
        };
        // Above the mover the tick is visible → stripped to background.
        assert_eq!(px(60, 5), (20, 20, 20), "visible tick was not stripped");
        // Where the mover covers the tick, the mover's pixels survive —
        // otherwise it would be sliced into two components.
        assert_eq!(
            px(60, 30),
            (0, 120, 255),
            "occluding mover was punched through by the strip"
        );
    }

    /// A mover that rests for *half* the clip is not furniture.  The
    /// median may well pick it up, but agreement rejects it — this is
    /// the guard that keeps a slide-in-and-settle animation from being
    /// frozen into the background.
    #[test]
    fn a_shape_at_rest_for_half_the_clip_is_not_static() {
        let w = 80u32;
        let h = 40u32;
        let frames: Vec<VideoFrame> = (0..20u32)
            .map(|i| {
                // Parked at x=30..50 for frames 0..10, then slides away.
                let bx = if i < 10 { 30 } else { 30 + (i - 9) * 6 };
                let block =
                    move |x: u32, y: u32| (bx..bx + 20).contains(&x) && (10..30).contains(&y);
                scene_frame(w, h, (20, 20, 20), &[((255, 80, 80), Box::new(block))])
            })
            .collect();

        match StaticLayer::measure(&frames) {
            None => {}
            Some(l) => assert!(
                l.area < 40,
                "a half-clip rest was frozen into the static layer ({} px)",
                l.area
            ),
        }
    }

    /// A clip with genuinely nothing persistent must not invent a layer.
    #[test]
    fn empty_scene_measures_no_layer() {
        let w = 64u32;
        let h = 64u32;
        let frames: Vec<VideoFrame> = (0..16u32)
            .map(|i| {
                let bx = i * 4;
                let block =
                    move |x: u32, y: u32| (bx..bx + 8).contains(&x) && (20..28).contains(&y);
                scene_frame(w, h, (10, 10, 10), &[((250, 250, 250), Box::new(block))])
            })
            .collect();
        assert!(
            StaticLayer::measure(&frames).is_none(),
            "invented a static layer from a clip with only a mover in it"
        );
    }
}
