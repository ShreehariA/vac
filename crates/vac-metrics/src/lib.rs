//! VAC reconstruction fidelity — how much of the video survived the
//! round-trip through `.vac` source.
//!
//! Every VAC version through v0.8 measured itself with *proxies*: shape
//! counts, animate-block counts, keyframe counts, `.vac` byte size.  A
//! proxy can improve while the reconstruction gets worse and nothing
//! notices.  This crate supplies the missing axis.
//!
//! # Three numbers, not one
//!
//! A single scalar cannot express what actually goes wrong here, because
//! the failure modes are qualitatively different:
//!
//! - [`Fidelity::residual`] — per-channel mean squared error.  Answers
//!   *"are the colours right?"*.  Framing matches PAC's
//!   `kmeans::mean_squared_residual`: a value of 100 means each R/G/B
//!   sits on average within ~10 of the truth.  This is what catches a
//!   clip of vivid colour bars reconstructing as a flat grey field.
//!
//! - [`Fidelity::ink_iou`] — intersection-over-union of the two frames'
//!   *ink masks* (pixels differing from the background).  Answers *"is
//!   the ink in the right place?"*, independently of whether its colour
//!   is right.  This is what catches a pendulum reconstructing as three
//!   superimposed bodies, or a Newton's cradle losing its balls.
//!
//! - The `worst_*` fields — the single worst frame and its index.  A
//!   tracking collapse that lasts half a second is invisible in a mean
//!   and obvious here.
//!
//! [`Fidelity::psnr_db`] is reported too, derived from the same MSE for
//! free, because it is the number everyone reads.  It should not be the
//! gate: squared error concentrates at edges, so a perceptually fine
//! recovery whose edges sit 1-2 px off scores worse than a blurry but
//! aligned one — and with anti-aliasing, even a geometrically perfect
//! recovery is never pixel-exact.
//!
//! # What is deliberately not here
//!
//! No SSIM.  It needs windowing, it is unstable on exactly the large
//! flat regions this content is full of, and the structural question
//! specific to this domain — *is the ink where it should be* — is
//! answered more directly and more interpretably by IoU.
//!
//! This crate is a leaf: it depends on nothing, takes frames as plain
//! borrowed RGBA8 slices, and can be unit-tested against hand-built
//! pixel buffers with no video machinery in scope.

#![forbid(unsafe_code)]

use std::fmt;

/// Pixels whose alpha is below this are treated as "not there" and are
/// excluded from every measurement.  Their RGB is undefined, so scoring
/// them would be measuring decoder noise.  Matches the decompiler's own
/// `MIN_OPAQUE_ALPHA`.
pub const MIN_OPAQUE_ALPHA: u8 = 128;

/// Default squared-RGB distance above which a pixel counts as ink.
/// Matches the decompiler's `FG_THRESHOLD_SQ` so the metric's notion of
/// "foreground" agrees with the pipeline's.
pub const DEFAULT_INK_THRESHOLD_SQ: u32 = 24 * 24 * 3;

/// A borrowed frame of non-premultiplied RGBA8.
///
/// Both `vac_compiler::RenderedFrame` and
/// `vac_decompiler::frames::VideoFrame` have exactly this layout, so
/// converting into one costs nothing.
#[derive(Debug, Clone, Copy)]
pub struct FrameRef<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}

impl<'a> FrameRef<'a> {
    pub fn new(width: u32, height: u32, rgba: &'a [u8]) -> Self {
        FrameRef {
            width,
            height,
            rgba,
        }
    }

    fn pixel_count(&self) -> usize {
        (self.width as usize) * (self.height as usize)
    }
}

/// Opaque RGB triple — the reference "background" a frame's ink is
/// measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb { r, g, b }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CompareOptions {
    /// Squared RGB distance from the background above which a pixel is
    /// ink.  See [`DEFAULT_INK_THRESHOLD_SQ`].
    pub ink_threshold_sq: u32,
}

impl Default for CompareOptions {
    fn default() -> Self {
        CompareOptions {
            ink_threshold_sq: DEFAULT_INK_THRESHOLD_SQ,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricError {
    /// The two sides disagree on how many frames there are.  Callers are
    /// expected to have aligned them with [`align`] first.
    FrameCountMismatch { original: usize, reconstructed: usize },
    /// One frame pair disagrees on dimensions.  This is a hard error
    /// rather than a silent rescale: it means the canvas was recovered
    /// wrong, which is a finding, not something to paper over.
    DimensionMismatch {
        frame: usize,
        original: (u32, u32),
        reconstructed: (u32, u32),
    },
    /// A backgrounds slice that does not line up with the frames.
    BackgroundCountMismatch { frames: usize, backgrounds: usize },
    /// A pixel buffer is shorter than `width * height * 4`.
    ShortBuffer {
        frame: usize,
        expected: usize,
        got: usize,
    },
    /// Nothing to compare.
    Empty,
}

impl fmt::Display for MetricError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricError::FrameCountMismatch {
                original,
                reconstructed,
            } => write!(
                f,
                "frame count mismatch: {original} original vs {reconstructed} reconstructed \
                 (align() them first)"
            ),
            MetricError::DimensionMismatch {
                frame,
                original,
                reconstructed,
            } => write!(
                f,
                "frame {frame}: dimensions differ — original {}x{}, reconstructed {}x{}",
                original.0, original.1, reconstructed.0, reconstructed.1
            ),
            MetricError::BackgroundCountMismatch { frames, backgrounds } => write!(
                f,
                "{backgrounds} backgrounds supplied for {frames} frames"
            ),
            MetricError::ShortBuffer {
                frame,
                expected,
                got,
            } => write!(
                f,
                "frame {frame}: pixel buffer is {got} bytes, expected {expected}"
            ),
            MetricError::Empty => write!(f, "nothing to compare: zero frames"),
        }
    }
}

impl std::error::Error for MetricError {}

/// One frame pair's scores.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameScore {
    /// Per-channel mean squared error over compared pixels.
    pub residual: f64,
    /// Peak signal-to-noise ratio in dB. Infinite for an exact match.
    pub psnr_db: f64,
    /// Ink-mask intersection over union, in `[0, 1]`. `1.0` when both
    /// masks are empty (two blank frames do agree).
    pub ink_iou: f64,
    /// Fraction of the source's ink the reconstruction failed to paint.
    /// Together with `spurious_ink` this says *which direction* the
    /// failure went — dropped an object, or hallucinated one — which the
    /// IoU deficit alone cannot distinguish.
    pub missing_ink: f64,
    /// Ink the reconstruction painted where the source had none, as a
    /// fraction of the source's ink. Can exceed 1.0 (painting three
    /// times the ink scores 3.0), which is informative rather than a bug.
    pub spurious_ink: f64,
    /// Pixels that were actually compared (opaque in both).
    pub compared_pixels: usize,
    /// Raw ink-pixel counts, so clip-level ratios can be pooled from
    /// sums rather than averaged from per-frame ratios. Averaging
    /// ratios lets a single frame whose source is blank — where the
    /// denominator collapses to 1 — dominate the whole clip.
    pub source_ink_px: usize,
    pub missing_px: usize,
    pub spurious_px: usize,
}

/// Whole-clip reconstruction quality.
#[derive(Debug, Clone, PartialEq)]
pub struct Fidelity {
    pub frames_compared: usize,
    /// Mean per-channel MSE across frames. Lower is better; 0 is exact.
    pub residual: f64,
    /// PSNR derived from `residual`. Higher is better.
    pub psnr_db: f64,
    /// Mean ink IoU across frames. Higher is better; 1.0 is exact.
    pub ink_iou: f64,
    /// Mean fraction of source ink left unpainted.
    pub missing_ink: f64,
    /// Mean fraction of extra ink painted where none belonged.
    pub spurious_ink: f64,
    /// Lowest single-frame PSNR, and where.
    pub worst_psnr_db: f64,
    pub worst_psnr_frame: usize,
    /// Lowest single-frame ink IoU, and where.
    pub worst_iou: f64,
    pub worst_iou_frame: usize,
    /// Per-frame detail, in source order.
    pub frames: Vec<FrameScore>,
}

impl fmt::Display for Fidelity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "frames compared : {}", self.frames_compared)?;
        writeln!(
            f,
            "residual        : {:.2}   (per-channel MSE; 100 ⇒ each R/G/B within ~10)",
            self.residual
        )?;
        writeln!(f, "psnr            : {:.2} dB", self.psnr_db)?;
        writeln!(
            f,
            "ink IoU         : {:.4}  (1.0 = ink exactly where it should be)",
            self.ink_iou
        )?;
        writeln!(
            f,
            "missing / extra : {:.1}% of source ink unpainted, {:.1}% painted spuriously",
            self.missing_ink * 100.0,
            self.spurious_ink * 100.0
        )?;
        writeln!(
            f,
            "worst frame     : psnr {:.2} dB @ #{}, IoU {:.4} @ #{}",
            self.worst_psnr_db, self.worst_psnr_frame, self.worst_iou, self.worst_iou_frame
        )
    }
}

/// PSNR for a per-channel mean squared error, 8-bit range.
pub fn psnr_from_mse(mse: f64) -> f64 {
    if mse <= 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    }
}

/// Compare one aligned frame pair.
///
/// `background` is the reference the *source* frame's ink is measured
/// against, and the reconstruction is judged on the same terms — using
/// each frame's own estimated background would be circular whenever
/// background estimation is itself the thing under suspicion.
pub fn compare_frame(
    original: &FrameRef,
    reconstructed: &FrameRef,
    background: Rgb,
    opts: &CompareOptions,
) -> FrameScore {
    let n = original.pixel_count();
    let mut sum_sq: u64 = 0;
    let mut compared: usize = 0;
    let mut inter: usize = 0;
    let mut union: usize = 0;
    let mut src_ink: usize = 0;
    let mut missing: usize = 0;
    let mut spurious: usize = 0;

    for i in 0..n {
        let o = &original.rgba[i * 4..i * 4 + 4];
        let r = &reconstructed.rgba[i * 4..i * 4 + 4];

        // A pixel the source never drew has undefined colour; scoring it
        // would measure decoder noise, not reconstruction quality.
        if o[3] < MIN_OPAQUE_ALPHA {
            continue;
        }
        compared += 1;

        let dr = o[0] as i32 - r[0] as i32;
        let dg = o[1] as i32 - r[1] as i32;
        let db = o[2] as i32 - r[2] as i32;
        sum_sq += (dr * dr + dg * dg + db * db) as u64;

        let o_ink = dist_sq(o, background) > opts.ink_threshold_sq;
        // A transparent pixel in the reconstruction is not ink: nothing
        // was painted there.
        let r_ink = r[3] >= MIN_OPAQUE_ALPHA && dist_sq(r, background) > opts.ink_threshold_sq;
        if o_ink {
            src_ink += 1;
        }
        if o_ink || r_ink {
            union += 1;
            match (o_ink, r_ink) {
                (true, true) => inter += 1,
                (true, false) => missing += 1,
                (false, true) => spurious += 1,
                (false, false) => unreachable!(),
            }
        }
    }

    let residual = if compared == 0 {
        0.0
    } else {
        sum_sq as f64 / (compared as f64 * 3.0)
    };
    let ink_iou = if union == 0 {
        1.0
    } else {
        inter as f64 / union as f64
    };

    // Per-frame ratios are diagnostics only; a frame whose source has no
    // ink has no meaningful denominator, and reports 0 rather than a
    // division artefact. Clip-level figures pool the raw counts instead.
    let denom = src_ink as f64;
    let (missing_ink, spurious_ink) = if src_ink == 0 {
        (0.0, 0.0)
    } else {
        (missing as f64 / denom, spurious as f64 / denom)
    };

    FrameScore {
        residual,
        psnr_db: psnr_from_mse(residual),
        ink_iou,
        missing_ink,
        spurious_ink,
        compared_pixels: compared,
        source_ink_px: src_ink,
        missing_px: missing,
        spurious_px: spurious,
    }
}

/// The most common exact opaque colour across a clip.
///
/// This is the reference every ink mask is measured against, and it is
/// computed here — from the frames themselves — rather than borrowed
/// from `vac_decompiler::detect::estimate_background`. That
/// independence is the point: a metric that shares the pipeline's own
/// background rule would silently rescale every committed baseline the
/// moment that rule was tuned, and "the decompiler improved" would
/// become indistinguishable from "the ruler moved".
///
/// A modal colour is also a stronger rule than corner sampling. The
/// decompiler medians four 5x5 corner blocks *per channel*, which on
/// content without a flat border can name a colour appearing nowhere in
/// the frame. The mode, by construction, always names a colour that is
/// really there — and for VAC's content the background wins the vote by
/// a wide margin.
///
/// Ties break toward the lower packed RGB value so the result is
/// deterministic regardless of hash iteration order.
pub fn modal_background(frames: &[FrameRef]) -> Rgb {
    let mut hist: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    for f in frames {
        let n = f.pixel_count().min(f.rgba.len() / 4);
        for i in 0..n {
            let px = &f.rgba[i * 4..i * 4 + 4];
            if px[3] < MIN_OPAQUE_ALPHA {
                continue;
            }
            let key = ((px[0] as u32) << 16) | ((px[1] as u32) << 8) | px[2] as u32;
            *hist.entry(key).or_insert(0) += 1;
        }
    }
    let best = hist
        .iter()
        .fold(None::<(u32, u64)>, |acc, (&k, &v)| match acc {
            Some((bk, bv)) if bv > v || (bv == v && bk <= k) => Some((bk, bv)),
            _ => Some((k, v)),
        });
    match best {
        Some((k, _)) => Rgb::new((k >> 16) as u8, (k >> 8) as u8, k as u8),
        None => Rgb::new(0, 0, 0),
    }
}

fn dist_sq(px: &[u8], bg: Rgb) -> u32 {
    let dr = px[0] as i32 - bg.r as i32;
    let dg = px[1] as i32 - bg.g as i32;
    let db = px[2] as i32 - bg.b as i32;
    (dr * dr + dg * dg + db * db) as u32
}

/// Compare two already-aligned frame sequences.
///
/// `original`, `reconstructed` and `backgrounds` must be parallel — use
/// [`align`] to build the reconstruction side when the two clips do not
/// share a frame grid.
pub fn compare(
    original: &[FrameRef],
    reconstructed: &[FrameRef],
    backgrounds: &[Rgb],
    opts: &CompareOptions,
) -> Result<Fidelity, MetricError> {
    if original.is_empty() {
        return Err(MetricError::Empty);
    }
    if original.len() != reconstructed.len() {
        return Err(MetricError::FrameCountMismatch {
            original: original.len(),
            reconstructed: reconstructed.len(),
        });
    }
    if backgrounds.len() != original.len() {
        return Err(MetricError::BackgroundCountMismatch {
            frames: original.len(),
            backgrounds: backgrounds.len(),
        });
    }

    for (i, (o, r)) in original.iter().zip(reconstructed.iter()).enumerate() {
        if o.width != r.width || o.height != r.height {
            return Err(MetricError::DimensionMismatch {
                frame: i,
                original: (o.width, o.height),
                reconstructed: (r.width, r.height),
            });
        }
        for (which, f) in [o, r].iter().enumerate() {
            let expected = f.pixel_count() * 4;
            if f.rgba.len() < expected {
                return Err(MetricError::ShortBuffer {
                    frame: if which == 0 { i } else { i },
                    expected,
                    got: f.rgba.len(),
                });
            }
        }
    }

    let mut frames = Vec::with_capacity(original.len());
    for i in 0..original.len() {
        frames.push(compare_frame(
            &original[i],
            &reconstructed[i],
            backgrounds[i],
            opts,
        ));
    }

    let n = frames.len() as f64;
    let residual = frames.iter().map(|f| f.residual).sum::<f64>() / n;
    let ink_iou = frames.iter().map(|f| f.ink_iou).sum::<f64>() / n;
    // Pooled over the whole clip, not averaged per frame — see
    // `FrameScore::source_ink_px`.
    let total_src: usize = frames.iter().map(|f| f.source_ink_px).sum();
    let total_missing: usize = frames.iter().map(|f| f.missing_px).sum();
    let total_spurious: usize = frames.iter().map(|f| f.spurious_px).sum();
    let (missing_ink, spurious_ink) = if total_src == 0 {
        (0.0, 0.0)
    } else {
        (
            total_missing as f64 / total_src as f64,
            total_spurious as f64 / total_src as f64,
        )
    };

    let (worst_psnr_frame, worst_psnr_db) = frames
        .iter()
        .enumerate()
        .map(|(i, f)| (i, f.psnr_db))
        .fold((0usize, f64::INFINITY), |acc, x| {
            if x.1 < acc.1 {
                x
            } else {
                acc
            }
        });
    let (worst_iou_frame, worst_iou) = frames
        .iter()
        .enumerate()
        .map(|(i, f)| (i, f.ink_iou))
        .fold((0usize, f64::INFINITY), |acc, x| {
            if x.1 < acc.1 {
                x
            } else {
                acc
            }
        });

    Ok(Fidelity {
        frames_compared: frames.len(),
        residual,
        psnr_db: psnr_from_mse(residual),
        ink_iou,
        missing_ink,
        spurious_ink,
        worst_psnr_db,
        worst_psnr_frame,
        worst_iou,
        worst_iou_frame,
        frames,
    })
}

// --------------------------------------------------------------------
// Temporal alignment
// --------------------------------------------------------------------

/// Running start time of each frame, from per-frame delays.
pub fn timeline_from_delays(delays_ms: &[u32]) -> Vec<u32> {
    let mut t = 0u32;
    let mut out = Vec::with_capacity(delays_ms.len());
    for &d in delays_ms {
        out.push(t);
        t = t.saturating_add(d);
    }
    out
}

/// For each source frame time, the index of the reconstruction frame
/// that should be showing at that instant.
///
/// The two sides never share a frame grid:
///
/// - Source frames carry independent per-frame delays (GIF's centisecond
///   quantisation is why a 30 fps clip round-trips as 33 fps).
/// - Rendered frames sit on an exact `t = i * 1000 / fps` grid, and the
///   **final instant is never rendered** — `vac_compiler` emits
///   `duration_ms * fps / 1000` frames, so a 3 s @ 30 fps scene spans
///   0…2966 ms, not 0…3000 ms.
///
/// So we walk the *source* timeline (it is the ground truth) and clamp
/// into the reconstruction, which is why the missing final instant is
/// harmless rather than an off-by-one.
pub fn align(source_times_ms: &[u32], recon_frame_count: usize, recon_fps: u32) -> Vec<usize> {
    if recon_frame_count == 0 {
        return Vec::new();
    }
    let fps = recon_fps.max(1) as u64;
    source_times_ms
        .iter()
        .map(|&t| {
            // Round to nearest rendered tick, then clamp.
            let idx = ((t as u64 * fps) + 500) / 1000;
            (idx as usize).min(recon_frame_count - 1)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a solid-colour frame.
    fn solid(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
        c.iter()
            .copied()
            .cycle()
            .take((w * h * 4) as usize)
            .collect()
    }

    /// Paint an axis-aligned filled rect into an RGBA buffer.
    fn paint_rect(buf: &mut [u8], w: u32, x0: u32, y0: u32, x1: u32, y1: u32, c: [u8; 4]) {
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * w + x) * 4) as usize;
                buf[i..i + 4].copy_from_slice(&c);
            }
        }
    }

    const BG: Rgb = Rgb::new(0, 0, 0);
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    // ---- known-answer cases: the metric itself must be right before -
    // ---- anything is measured with it. ------------------------------

    #[test]
    fn identical_frames_score_perfect() {
        let a = solid(16, 16, [10, 20, 30, 255]);
        let f = FrameRef::new(16, 16, &a);
        let s = compare_frame(&f, &f, BG, &CompareOptions::default());
        assert_eq!(s.residual, 0.0);
        assert!(s.psnr_db.is_infinite());
        assert_eq!(s.ink_iou, 1.0);
        assert_eq!(s.compared_pixels, 256);
    }

    #[test]
    fn a_known_uniform_offset_produces_the_arithmetic_residual() {
        // Every channel off by exactly 10 ⇒ per-channel MSE of 100.
        // This is the anchor for the doc claim "100 ⇒ within ~10".
        let a = solid(8, 8, [100, 100, 100, 255]);
        let b = solid(8, 8, [110, 110, 110, 255]);
        let s = compare_frame(
            &FrameRef::new(8, 8, &a),
            &FrameRef::new(8, 8, &b),
            BG,
            &CompareOptions::default(),
        );
        assert!(
            (s.residual - 100.0).abs() < 1e-9,
            "residual = {}",
            s.residual
        );
        // 10*log10(65025/100) = 28.13 dB
        assert!((s.psnr_db - 28.1308).abs() < 1e-3, "psnr = {}", s.psnr_db);
    }

    #[test]
    fn psnr_is_monotone_in_error() {
        let base = solid(8, 8, [128, 128, 128, 255]);
        let near = solid(8, 8, [130, 128, 128, 255]);
        let far = solid(8, 8, [200, 128, 128, 255]);
        let o = FrameRef::new(8, 8, &base);
        let opts = CompareOptions::default();
        let s_near = compare_frame(&o, &FrameRef::new(8, 8, &near), BG, &opts);
        let s_far = compare_frame(&o, &FrameRef::new(8, 8, &far), BG, &opts);
        assert!(
            s_near.psnr_db > s_far.psnr_db,
            "near {} should beat far {}",
            s_near.psnr_db,
            s_far.psnr_db
        );
        assert!(s_near.residual < s_far.residual);
    }

    #[test]
    fn transparent_source_pixels_are_excluded_not_scored() {
        // Left half opaque and matching, right half transparent in the
        // source but wildly wrong in the reconstruction.  A metric that
        // scored the transparent half would report a large error.
        let mut a = solid(8, 4, BLACK);
        let mut b = solid(8, 4, BLACK);
        for y in 0..4u32 {
            for x in 4..8u32 {
                let i = ((y * 8 + x) * 4) as usize;
                a[i + 3] = 0; // source: nothing there
                b[i..i + 4].copy_from_slice(&WHITE); // recon: painted
            }
        }
        let s = compare_frame(
            &FrameRef::new(8, 4, &a),
            &FrameRef::new(8, 4, &b),
            BG,
            &CompareOptions::default(),
        );
        assert_eq!(s.compared_pixels, 16, "only the opaque half counts");
        assert_eq!(s.residual, 0.0, "the opaque half matches exactly");
    }

    // ---- the structural term ---------------------------------------

    #[test]
    fn iou_separates_two_failures_that_psnr_conflates() {
        // This is the load-bearing justification for reporting IoU at all.
        //
        // Two reconstructions of the same source, representing genuinely
        // different failures:
        //
        //   A — ink in the wrong place (the pendulum-ghosting shape of
        //       failure: right amount of ink, disjoint placement).
        //   B — ink in the right place, wrong colour (the Newton's-cradle
        //       shape: correct silhouette flattened to one median tone).
        //
        // PSNR lands them within a few dB of each other, because it only
        // ever reports "how much squared error, spread over the frame".
        // IoU gives them opposite verdicts, which is the information PSNR
        // structurally cannot carry.
        let mut src = solid(300, 300, BLACK);
        paint_rect(&mut src, 300, 0, 0, 30, 30, WHITE);

        let mut displaced = solid(300, 300, BLACK);
        paint_rect(&mut displaced, 300, 100, 100, 130, 130, WHITE);

        let mut miscoloured = solid(300, 300, BLACK);
        paint_rect(&mut miscoloured, 300, 0, 0, 30, 30, [255, 0, 0, 255]);

        let o = FrameRef::new(300, 300, &src);
        let opts = CompareOptions::default();
        let a = compare_frame(&o, &FrameRef::new(300, 300, &displaced), BG, &opts);
        let b = compare_frame(&o, &FrameRef::new(300, 300, &miscoloured), BG, &opts);

        assert_eq!(a.ink_iou, 0.0, "displaced ink: nothing overlaps");
        assert_eq!(b.ink_iou, 1.0, "miscoloured ink: silhouette is exact");

        assert!(
            (a.psnr_db - b.psnr_db).abs() < 6.0,
            "PSNR should fail to distinguish these: displaced {:.2} dB vs \
             miscoloured {:.2} dB",
            a.psnr_db,
            b.psnr_db
        );
    }

    #[test]
    fn partial_overlap_gives_the_arithmetic_iou() {
        // Source ink = x in [0,20); recon ink = x in [10,30).
        // intersection 10 cols, union 30 cols ⇒ 1/3.
        let mut a = solid(40, 5, BLACK);
        let mut b = solid(40, 5, BLACK);
        paint_rect(&mut a, 40, 0, 0, 20, 5, WHITE);
        paint_rect(&mut b, 40, 10, 0, 30, 5, WHITE);
        let s = compare_frame(
            &FrameRef::new(40, 5, &a),
            &FrameRef::new(40, 5, &b),
            BG,
            &CompareOptions::default(),
        );
        assert!(
            (s.ink_iou - 1.0 / 3.0).abs() < 1e-9,
            "iou = {}",
            s.ink_iou
        );
    }

    #[test]
    fn a_reconstruction_that_paints_nothing_scores_zero_iou() {
        // The testsrc2 failure: source is full of content, recon is flat.
        let mut a = solid(20, 20, BLACK);
        paint_rect(&mut a, 20, 2, 2, 18, 18, WHITE);
        let b = solid(20, 20, BLACK);
        let s = compare_frame(
            &FrameRef::new(20, 20, &a),
            &FrameRef::new(20, 20, &b),
            BG,
            &CompareOptions::default(),
        );
        assert_eq!(s.ink_iou, 0.0);
    }

    #[test]
    fn two_blank_frames_agree() {
        let a = solid(8, 8, BLACK);
        let s = compare_frame(
            &FrameRef::new(8, 8, &a),
            &FrameRef::new(8, 8, &a),
            BG,
            &CompareOptions::default(),
        );
        assert_eq!(s.ink_iou, 1.0, "no ink on either side is agreement");
    }

    #[test]
    fn iou_is_colour_blind_but_residual_is_not() {
        // Same silhouette, wrong colour — the Newton's-cradle failure of
        // painting the right shape in one flat median tone.  IoU should
        // stay perfect (ink is where it belongs) while residual reports
        // the colour loss.  That separation is the point of having both.
        let mut a = solid(20, 20, BLACK);
        let mut b = solid(20, 20, BLACK);
        paint_rect(&mut a, 20, 5, 5, 15, 15, [255, 0, 0, 255]);
        paint_rect(&mut b, 20, 5, 5, 15, 15, [0, 0, 255, 255]);
        let s = compare_frame(
            &FrameRef::new(20, 20, &a),
            &FrameRef::new(20, 20, &b),
            BG,
            &CompareOptions::default(),
        );
        assert_eq!(s.ink_iou, 1.0, "silhouette is correct");
        assert!(s.residual > 1000.0, "colour is badly wrong: {}", s.residual);
    }

    // ---- clip-level aggregation ------------------------------------

    #[test]
    fn worst_frame_is_reported_not_averaged_away() {
        // 9 perfect frames and 1 catastrophic one.  The mean looks fine;
        // the worst-frame fields must expose the collapse.
        let good = solid(10, 10, BLACK);
        let bad = solid(10, 10, WHITE);
        let mut orig = Vec::new();
        let mut recon = Vec::new();
        for _ in 0..10 {
            orig.push(FrameRef::new(10, 10, &good));
        }
        for i in 0..10 {
            recon.push(FrameRef::new(10, 10, if i == 7 { &bad } else { &good }));
        }
        let bgs = vec![BG; 10];
        let fid = compare(&orig, &recon, &bgs, &CompareOptions::default()).unwrap();
        assert_eq!(fid.frames_compared, 10);
        assert_eq!(fid.worst_psnr_frame, 7);
        assert_eq!(fid.worst_iou_frame, 7);
        assert_eq!(fid.worst_iou, 0.0);
        assert!(
            fid.worst_psnr_db < 10.0,
            "worst frame psnr = {}",
            fid.worst_psnr_db
        );
    }

    #[test]
    fn dimension_mismatch_is_an_error_not_a_rescale() {
        let a = solid(8, 8, BLACK);
        let b = solid(4, 4, BLACK);
        let err = compare(
            &[FrameRef::new(8, 8, &a)],
            &[FrameRef::new(4, 4, &b)],
            &[BG],
            &CompareOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, MetricError::DimensionMismatch { .. }));
    }

    #[test]
    fn mismatched_frame_counts_are_rejected() {
        let a = solid(4, 4, BLACK);
        let err = compare(
            &[FrameRef::new(4, 4, &a), FrameRef::new(4, 4, &a)],
            &[FrameRef::new(4, 4, &a)],
            &[BG, BG],
            &CompareOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, MetricError::FrameCountMismatch { .. }));
    }

    // ---- the ink reference ------------------------------------------

    #[test]
    fn modal_background_picks_the_dominant_colour() {
        let mut buf = solid(20, 20, [26, 26, 46, 255]);
        paint_rect(&mut buf, 20, 5, 5, 12, 12, [233, 69, 96, 255]);
        let bg = modal_background(&[FrameRef::new(20, 20, &buf)]);
        assert_eq!(bg, Rgb::new(26, 26, 46));
    }

    #[test]
    fn modal_background_names_a_colour_that_is_actually_present() {
        // Four quadrants, no flat border. The decompiler's rule medians
        // the four corner blocks *per channel independently*, which here
        // yields (255,0,0)-ish blends that appear nowhere in the frame.
        // A mode cannot do that: it always returns a real pixel value.
        let mut buf = solid(20, 20, [255, 0, 0, 255]);
        paint_rect(&mut buf, 20, 10, 0, 20, 10, [0, 255, 0, 255]);
        paint_rect(&mut buf, 20, 0, 10, 10, 20, [0, 0, 255, 255]);
        paint_rect(&mut buf, 20, 10, 10, 20, 20, [255, 255, 0, 255]);
        let bg = modal_background(&[FrameRef::new(20, 20, &buf)]);
        let present = [
            Rgb::new(255, 0, 0),
            Rgb::new(0, 255, 0),
            Rgb::new(0, 0, 255),
            Rgb::new(255, 255, 0),
        ];
        assert!(present.contains(&bg), "mode returned a phantom colour: {bg:?}");
    }

    #[test]
    fn modal_background_is_deterministic_under_an_exact_tie() {
        // Two colours, equal counts. Hash iteration order must not decide.
        let mut buf = solid(10, 10, [10, 10, 10, 255]);
        paint_rect(&mut buf, 10, 0, 5, 10, 10, [200, 200, 200, 255]);
        let first = modal_background(&[FrameRef::new(10, 10, &buf)]);
        for _ in 0..20 {
            assert_eq!(modal_background(&[FrameRef::new(10, 10, &buf)]), first);
        }
    }

    // ---- directional decomposition ----------------------------------

    #[test]
    fn missing_and_spurious_name_the_direction_of_the_failure() {
        // Source has ink; reconstruction paints none. IoU says "wrong",
        // missing/spurious say "you dropped it" rather than "you invented
        // something".
        let mut a = solid(20, 20, BLACK);
        paint_rect(&mut a, 20, 5, 5, 15, 15, WHITE);
        let blank = solid(20, 20, BLACK);
        let dropped = compare_frame(
            &FrameRef::new(20, 20, &a),
            &FrameRef::new(20, 20, &blank),
            BG,
            &CompareOptions::default(),
        );
        assert!((dropped.missing_ink - 1.0).abs() < 1e-9, "{dropped:?}");
        assert_eq!(dropped.spurious_ink, 0.0);

        // And the reverse: source blank, reconstruction hallucinates.
        // With no source ink there is no meaningful denominator, so the
        // per-frame *ratio* is reported as 0 and the raw count carries
        // the signal — which is what the clip level pools.
        let invented = compare_frame(
            &FrameRef::new(20, 20, &blank),
            &FrameRef::new(20, 20, &a),
            BG,
            &CompareOptions::default(),
        );
        assert_eq!(invented.source_ink_px, 0);
        assert_eq!(invented.missing_px, 0);
        assert!(invented.spurious_px > 0, "{invented:?}");
        assert_eq!(invented.ink_iou, 0.0, "hallucinated ink is still a miss");
    }

    #[test]
    fn spurious_ink_may_exceed_one_when_ink_is_multiplied() {
        // The pendulum-ghosting signature: one body in, three out.
        let mut a = solid(90, 10, BLACK);
        paint_rect(&mut a, 90, 0, 0, 10, 10, WHITE);
        let mut b = solid(90, 10, BLACK);
        for k in 0..3 {
            paint_rect(&mut b, 90, k * 30, 0, k * 30 + 10, 10, WHITE);
        }
        let s = compare_frame(
            &FrameRef::new(90, 10, &a),
            &FrameRef::new(90, 10, &b),
            BG,
            &CompareOptions::default(),
        );
        assert!(
            (s.spurious_ink - 2.0).abs() < 1e-9,
            "two extra bodies ⇒ 2.0, got {}",
            s.spurious_ink
        );
        assert_eq!(s.missing_ink, 0.0, "the real body was still painted");
    }

    #[test]
    fn a_blank_source_frame_cannot_blow_up_the_clip_ratio() {
        // Found on `slide_in_card.mp4`: its first frames have the card
        // fully off-canvas, so source ink is zero while the
        // reconstruction paints a card. Dividing by a zero denominator
        // (clamped to 1) reported 6313% spurious ink for the clip.
        // Pooling raw counts over the clip is the fix.
        let blank = solid(40, 40, BLACK);
        let mut inked = solid(40, 40, BLACK);
        paint_rect(&mut inked, 40, 0, 0, 20, 20, WHITE);

        let orig = vec![
            FrameRef::new(40, 40, &blank),
            FrameRef::new(40, 40, &inked),
        ];
        let recon = vec![
            FrameRef::new(40, 40, &inked),
            FrameRef::new(40, 40, &inked),
        ];
        let fid = compare(&orig, &recon, &[BG, BG], &CompareOptions::default()).unwrap();

        // 400 spurious px on frame 0, 400 source ink px on frame 1.
        assert!(
            (fid.spurious_ink - 1.0).abs() < 1e-9,
            "spurious should pool to 1.0, got {}",
            fid.spurious_ink
        );
        assert_eq!(fid.missing_ink, 0.0);
        // The blank frame is still a real failure and must still show up.
        assert_eq!(fid.worst_iou, 0.0);
        assert_eq!(fid.worst_iou_frame, 0);
    }

    // ---- alignment --------------------------------------------------

    #[test]
    fn timeline_accumulates_delays() {
        assert_eq!(timeline_from_delays(&[33, 33, 33]), vec![0, 33, 66]);
        assert_eq!(timeline_from_delays(&[]), Vec::<u32>::new());
    }

    #[test]
    fn identical_grids_align_one_to_one() {
        let times = timeline_from_delays(&[33; 5]);
        // 30 fps ticks: 0, 33, 66, 100, 133
        assert_eq!(align(&times, 5, 30), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn alignment_clamps_past_the_unrendered_final_instant() {
        // vac-compiler renders duration*fps/1000 frames, so a 3 s @ 30 fps
        // scene has 90 frames covering 0..2966 ms.  A source frame at
        // 2990 ms has no exact counterpart and must clamp, not panic.
        let times = vec![0, 1500, 2990];
        let idx = align(&times, 90, 30);
        assert_eq!(idx[0], 0);
        assert_eq!(idx[1], 45);
        assert_eq!(idx[2], 89, "clamped to the last rendered frame");
    }

    #[test]
    fn alignment_handles_fps_drift_between_the_two_grids() {
        // GIF centisecond quantisation: source claims 33 fps (30 ms
        // delays) while the reconstruction renders at 30 fps.
        let times = timeline_from_delays(&[30; 10]); // 0,30,...,270
        let idx = align(&times, 10, 30);
        // At 30 fps a tick is 33.3 ms; source frame k sits at 30k ms.
        assert_eq!(idx[0], 0);
        assert_eq!(idx[9], 8, "270 ms → tick 8.1 → 8");
        assert!(
            idx.windows(2).all(|w| w[1] >= w[0]),
            "alignment must be monotone: {idx:?}"
        );
    }

    #[test]
    fn empty_reconstruction_aligns_to_nothing() {
        assert_eq!(align(&[0, 33], 0, 30), Vec::<usize>::new());
    }
}
