//! Camera motion (v1.10).
//!
//! A clip whose whole picture moves together — a zoom, a pan — is one image
//! seen through a moving camera. Through v1.9 VAC had no way to say so:
//! the static layer found nothing static, and the Mandelbrot zoom came back
//! as 33 flat shapes at 12 dB, although frame 0, magnified, explains every
//! later frame at 23–29 dB.
//!
//! This module measures the camera instead of classifying the clip. Each
//! frame's *view* of a reference frame — where each reference point appears
//! in it, `p = scale · q + offset` — is found by a coarse-to-fine pattern
//! search that minimises a robust pixel error, so a moving object on top
//! cannot drag the camera with it. Every frame is then warped back into
//! reference coordinates and the per-pixel median of all of them is the
//! scene: a *mosaic*, larger than any one frame when the camera pans. The
//! mosaic becomes one `image()` whose `position()` and `scale()` keyframes
//! are the camera. Whether any of this beats the alternatives is decided by
//! the `vac` facade, by measurement.

use crate::frames::VideoFrame;

/// How one frame sees the reference: the reference point `q` appears at
/// `scale · q + offset` in the frame. Continuous coordinates, in which
/// pixel `(i, j)` covers `[i, i + 1) × [j, j + 1)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub scale: f32,
    pub offset: (f32, f32),
}

impl View {
    pub const IDENTITY: View = View {
        scale: 1.0,
        offset: (0.0, 0.0),
    };

    /// Where reference point `q` appears in the frame.
    pub fn to_frame(&self, q: (f32, f32)) -> (f32, f32) {
        (self.scale * q.0 + self.offset.0, self.scale * q.1 + self.offset.1)
    }

    /// The reference point shown at frame point `p`.
    pub fn to_reference(&self, p: (f32, f32)) -> (f32, f32) {
        ((p.0 - self.offset.0) / self.scale, (p.1 - self.offset.1) / self.scale)
    }
}

/// Frames whose views move no corner of the canvas this far (pixels) are
/// a still camera: the static plate already covers them.
pub const MIN_CAMERA_MOTION_PX: f32 = 2.0;

/// The robust error's ceiling, per pixel (squared RGB distance). A pixel
/// that disagrees by more than this — a mover, new detail — costs the same
/// however far off it is, so it cannot pull the estimate toward itself.
const ROBUST_CEILING_SQ: f32 = 3.0 * 40.0 * 40.0;

/// A frame as floating-point RGB, for sampling between pixels. `origin`
/// is where its top-left corner sits in the coordinates it is sampled in
/// — zero for a frame, the mosaic's corner for a mosaic.
struct Level {
    w: usize,
    h: usize,
    px: Vec<f32>,
    origin: (f32, f32),
}

impl Level {
    fn from_frame(f: &VideoFrame) -> Level {
        let (w, h) = (f.width as usize, f.height as usize);
        let mut px = Vec::with_capacity(w * h * 3);
        for p in f.pixels.chunks_exact(4) {
            px.extend_from_slice(&[p[0] as f32, p[1] as f32, p[2] as f32]);
        }
        Level {
            w,
            h,
            px,
            origin: (0.0, 0.0),
        }
    }

    /// Half the size, each pixel the mean of the 2×2 block it covers.
    fn half(&self) -> Level {
        let (w, h) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let mut px = vec![0f32; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                for c in 0..3 {
                    let at = |xx: usize, yy: usize| {
                        self.px[((yy.min(self.h - 1)) * self.w + xx.min(self.w - 1)) * 3 + c]
                    };
                    px[(y * w + x) * 3 + c] =
                        0.25 * (at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1));
                }
            }
        }
        Level {
            w,
            h,
            px,
            origin: (self.origin.0 * 0.5, self.origin.1 * 0.5),
        }
    }

    /// Bilinear sample at continuous point `p`; `None` outside the frame.
    fn sample(&self, p: (f32, f32)) -> Option<[f32; 3]> {
        let (x, y) = (p.0 - self.origin.0 - 0.5, p.1 - self.origin.1 - 0.5);
        if x < -0.5 || y < -0.5 || x > self.w as f32 - 0.5 || y > self.h as f32 - 0.5 {
            return None;
        }
        let (x, y) = (x.clamp(0.0, (self.w - 1) as f32), y.clamp(0.0, (self.h - 1) as f32));
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let mut out = [0f32; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let at = |xx: usize, yy: usize| self.px[(yy * self.w + xx) * 3 + c];
            *o = (at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx) * (1.0 - fy)
                + (at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx) * fy;
        }
        Some(out)
    }
}

/// A frame at successively halved sizes.
fn pyramid(f: &VideoFrame, levels: usize) -> Vec<Level> {
    pyramid_from(Level::from_frame(f), levels)
}

fn pyramid_from(base: Level, levels: usize) -> Vec<Level> {
    let mut out = vec![base];
    for _ in 1..levels {
        let next = out.last().expect("one level").half();
        out.push(next);
    }
    out
}

/// The mean robust error of `frame` against `reference` under view `v`
/// (in this level's coordinates), over every `stride`-th pixel the
/// reference covers. Infinite when less than a third is covered.
fn cost(frame: &Level, reference: &Level, v: View, stride: usize) -> f64 {
    let (mut sum, mut n, mut total) = (0f64, 0usize, 0usize);
    for y in (0..frame.h).step_by(stride) {
        for x in (0..frame.w).step_by(stride) {
            total += 1;
            let q = v.to_reference((x as f32 + 0.5, y as f32 + 0.5));
            let Some(r) = reference.sample(q) else { continue };
            let i = (y * frame.w + x) * 3;
            let d: f32 = (0..3).map(|c| (frame.px[i + c] - r[c]).powi(2)).sum();
            sum += d.min(ROBUST_CEILING_SQ) as f64;
            n += 1;
        }
    }
    if n * 3 < total || n == 0 {
        f64::INFINITY
    } else {
        sum / n as f64
    }
}

/// Pattern search for the view minimising [`cost`], from `start`, at one
/// pyramid level. Offsets step from `offset_step` level pixels and scale
/// from `scale_step`, halving whenever no neighbour improves.
fn search(frame: &Level, reference: &Level, start: View, offset_step: f32, scale_step: f32, stride: usize) -> View {
    let mut v = start;
    let mut best = cost(frame, reference, v, stride);
    let (mut os, mut ss) = (offset_step, scale_step);
    for _ in 0..400 {
        let mut moved = false;
        let trials = [
            View { offset: (v.offset.0 + os, v.offset.1), ..v },
            View { offset: (v.offset.0 - os, v.offset.1), ..v },
            View { offset: (v.offset.0, v.offset.1 + os), ..v },
            View { offset: (v.offset.0, v.offset.1 - os), ..v },
            // Scale about the frame centre, so a zoom does not also drift.
            zoom_about(v, 1.0 + ss, (frame.w as f32 * 0.5, frame.h as f32 * 0.5)),
            zoom_about(v, 1.0 - ss, (frame.w as f32 * 0.5, frame.h as f32 * 0.5)),
        ];
        for t in trials {
            let c = cost(frame, reference, t, stride);
            if c < best {
                best = c;
                v = t;
                moved = true;
            }
        }
        if !moved {
            os *= 0.5;
            ss *= 0.5;
            if os < 0.02 && ss < 0.0002 {
                break;
            }
        }
    }
    v
}

/// `v`, zoomed by `factor` about frame point `c`.
fn zoom_about(v: View, factor: f32, c: (f32, f32)) -> View {
    View {
        scale: v.scale * factor,
        offset: (c.0 + (v.offset.0 - c.0) * factor, c.1 + (v.offset.1 - c.1) * factor),
    }
}

/// `v` in the coordinates of a level `2^k` times smaller.
fn at_level(v: View, k: usize) -> View {
    let f = (1u32 << k) as f32;
    View {
        scale: v.scale,
        offset: (v.offset.0 / f, v.offset.1 / f),
    }
}

/// `v` from a level `2^k` times smaller, in full-size coordinates.
fn from_level(v: View, k: usize) -> View {
    let f = (1u32 << k) as f32;
    View {
        scale: v.scale,
        offset: (v.offset.0 * f, v.offset.1 * f),
    }
}

/// Each frame's view of frame `reference`, tracked frame to frame: every
/// estimate starts from the one before it and is refined from a quarter
/// size up to full size against the reference itself, so errors do not
/// accumulate along the clip.
pub fn track(frames: &[VideoFrame], reference: usize) -> Vec<View> {
    track_every(frames, reference, 1)
}

/// [`track`] on every `stride`-th frame only — the cheap first look that
/// tells a still camera from a moving one. Untracked frames keep the
/// identity view.
pub fn track_every(frames: &[VideoFrame], reference: usize, stride: usize) -> Vec<View> {
    const LEVELS: usize = 3;
    let stride = stride.max(1);
    let rp = pyramid(&frames[reference], LEVELS);
    let mut views = vec![View::IDENTITY; frames.len()];
    let walk = |order: &mut dyn Iterator<Item = usize>, views: &mut Vec<View>| {
        let mut prev = View::IDENTITY;
        for t in order {
            let fp = pyramid(&frames[t], LEVELS);
            let mut v = at_level(prev, LEVELS - 1);
            for k in (0..LEVELS).rev() {
                if k != LEVELS - 1 {
                    v = at_level(from_level(v, k + 1), k);
                }
                let stride = if k == 0 { 2 } else { 1 };
                v = search(&fp[k], &rp[k], v, 1.0, 0.01, stride);
            }
            prev = from_level(v, 0);
            views[t] = prev;
        }
    };
    walk(&mut (reference..frames.len()).step_by(stride), &mut views);
    walk(&mut (0..reference).rev().step_by(stride), &mut views);
    views
}

/// Re-estimate every view against the mosaic instead of a single frame.
///
/// The first pass aligns each frame to frame 0, and anything moving in
/// frame 0 is baked into that reference: on `camera.vac` a ball crossing
/// the scene dragged the zoom estimate 1.5 px off by the end of the clip
/// (0.06 px without the ball). The mosaic is the median of all frames, so
/// the ball is gone from it; aligning to it, from the first pass's views,
/// takes the bias out.
pub fn refine_views(frames: &[VideoFrame], m: &Mosaic, first: &[View]) -> Vec<View> {
    const LEVELS: usize = 2;
    let mut base = Level::from_frame(&m.image);
    base.origin = (m.x, m.y);
    let mp = pyramid_from(base, LEVELS);
    frames
        .iter()
        .zip(first)
        .map(|(f, &v0)| {
            let fp = pyramid(f, LEVELS);
            let mut v = at_level(v0, LEVELS - 1);
            for k in (0..LEVELS).rev() {
                if k != LEVELS - 1 {
                    v = at_level(from_level(v, k + 1), k);
                }
                let stride = if k == 0 { 2 } else { 1 };
                v = search(&fp[k], &mp[k], v, 0.5, 0.004, stride);
            }
            from_level(v, 0)
        })
        .collect()
}

/// The frames at which the camera must be pinned so that interpolating
/// linearly in time between them (what the renderer does with `position()`
/// and `scale()` keyframes) keeps every canvas corner within `tol_px` of
/// where its own view puts it. Always includes the first and last frame.
pub fn keyframes(views: &[View], times: &[u32], w: u32, h: u32, tol_px: f32) -> Vec<usize> {
    let n = views.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let corners = [(0.0, 0.0), (w as f32, 0.0), (0.0, h as f32), (w as f32, h as f32)];
    let err = |a: usize, b: usize, t: usize| -> f32 {
        let span = times[b].saturating_sub(times[a]).max(1) as f32;
        let u = (times[t].saturating_sub(times[a]) as f32 / span).clamp(0.0, 1.0);
        let (va, vb) = (views[a], views[b]);
        let v = View {
            scale: va.scale + (vb.scale - va.scale) * u,
            offset: (
                va.offset.0 + (vb.offset.0 - va.offset.0) * u,
                va.offset.1 + (vb.offset.1 - va.offset.1) * u,
            ),
        };
        corners
            .iter()
            .map(|&p| {
                let q = views[t].to_reference(p);
                let r = v.to_frame(q);
                ((r.0 - p.0).powi(2) + (r.1 - p.1).powi(2)).sqrt()
            })
            .fold(0.0, f32::max)
    };
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (worst, at) = (a + 1..b).map(|t| (err(a, b, t), t)).fold((0.0f32, a), |m, x| if x.0 > m.0 { x } else { m });
        if worst > tol_px {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    (0..n).filter(|&i| keep[i]).collect()
}

/// A camera moves the whole picture: at least this fraction of a frame's
/// pixels must be explained better by its view than by a still camera.
/// On `sprite.vac` — one textured tile sliding over a flat background —
/// the tracker follows the tile, because nothing else has texture to hold
/// on to, but only the ~7% of pixels the tile covers at either end of its
/// path agree; on a real zoom or pan nearly all of them do.
pub const MIN_CAMERA_SUPPORT: f32 = 0.2;

/// How much better a pixel must match under the camera than under a still
/// camera to count as evidence for it (squared RGB distance).
const SUPPORT_MARGIN_SQ: f32 = 3.0 * 10.0 * 10.0;

/// The median, over the frames in `sample` whose view moves the canvas at
/// all, of the fraction of each frame's pixels that frame `reference`
/// explains better through that view than through a still camera.
pub fn support(frames: &[VideoFrame], reference: usize, views: &[View], sample: &[usize]) -> f32 {
    let r = Level::from_frame(&frames[reference]);
    let (w, h) = (frames[0].width, frames[0].height);
    let mut fractions: Vec<f32> = Vec::new();
    for &t in sample {
        if motion_px(&views[t..=t], w, h) < MIN_CAMERA_MOTION_PX {
            continue;
        }
        let f = Level::from_frame(&frames[t]);
        let (mut agree, mut total) = (0usize, 0usize);
        for y in (0..f.h).step_by(2) {
            for x in (0..f.w).step_by(2) {
                total += 1;
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let (Some(cam), Some(still)) = (r.sample(views[t].to_reference(p)), r.sample(p)) else {
                    continue;
                };
                let i = (y * f.w + x) * 3;
                let d = |c: [f32; 3]| (0..3).map(|k| (f.px[i + k] - c[k]).powi(2)).sum::<f32>();
                if d(cam) + SUPPORT_MARGIN_SQ < d(still) {
                    agree += 1;
                }
            }
        }
        fractions.push(agree as f32 / total.max(1) as f32);
    }
    if fractions.is_empty() {
        return 0.0;
    }
    fractions.sort_by(|a, b| a.total_cmp(b));
    fractions[fractions.len() / 2]
}

/// How far the views move the canvas: the largest displacement of any of
/// its corners from where the reference has it, pixels.
pub fn motion_px(views: &[View], w: u32, h: u32) -> f32 {
    let corners = [(0.0, 0.0), (w as f32, 0.0), (0.0, h as f32), (w as f32, h as f32)];
    views
        .iter()
        .flat_map(|v| corners.iter().map(move |&c| (*v, c)))
        .map(|(v, c)| {
            let p = v.to_frame(c);
            ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt()
        })
        .fold(0.0, f32::max)
}

/// The scene in reference coordinates: every frame warped back by its view,
/// the per-pixel median of all that see each point.
pub struct Mosaic {
    /// The mosaic's box in reference coordinates.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Its pixels, one per reference pixel.
    pub image: VideoFrame,
}

/// Most frames the mosaic's median reads, evenly spaced over the clip.
const MOSAIC_MAX_SAMPLES: usize = 45;
/// The largest mosaic, as a multiple of the canvas area: a camera that
/// wanders further is not one scene.
const MOSAIC_MAX_AREA: f32 = 4.0;

/// Build the [`Mosaic`] the views see; `None` when it would be
/// unreasonably large.
pub fn mosaic(frames: &[VideoFrame], views: &[View], bg: [u8; 3]) -> Option<Mosaic> {
    let (fw, fh) = (frames[0].width as f32, frames[0].height as f32);
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for v in views {
        for c in [(0.0, 0.0), (fw, 0.0), (0.0, fh), (fw, fh)] {
            let q = v.to_reference(c);
            x0 = x0.min(q.0);
            y0 = y0.min(q.1);
            x1 = x1.max(q.0);
            y1 = y1.max(q.1);
        }
    }
    let (x0, y0, x1, y1) = (x0.floor(), y0.floor(), x1.ceil(), y1.ceil());
    let (mw, mh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if mw == 0 || mh == 0 || (mw * mh) as f32 > MOSAIC_MAX_AREA * fw * fh {
        return None;
    }
    let step = (frames.len() / MOSAIC_MAX_SAMPLES).max(1);
    let picked: Vec<usize> = (0..frames.len()).step_by(step).collect();
    let levels: Vec<Level> = picked.iter().map(|&t| Level::from_frame(&frames[t])).collect();
    let mut pixels = vec![0u8; mw * mh * 4];
    let mut samples: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for my in 0..mh {
        for mx in 0..mw {
            let q = (x0 + mx as f32 + 0.5, y0 + my as f32 + 0.5);
            for s in samples.iter_mut() {
                s.clear();
            }
            for (n, &t) in picked.iter().enumerate() {
                if let Some(rgb) = levels[n].sample(views[t].to_frame(q)) {
                    for c in 0..3 {
                        samples[c].push(rgb[c]);
                    }
                }
            }
            let i = (my * mw + mx) * 4;
            for c in 0..3 {
                pixels[i + c] = if samples[c].is_empty() {
                    bg[c]
                } else {
                    let s = &mut samples[c];
                    let mid = s.len() / 2;
                    s.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
                    s[mid].round().clamp(0.0, 255.0) as u8
                };
            }
            pixels[i + 3] = 255;
        }
    }
    Some(Mosaic {
        x: x0,
        y: y0,
        w: mw as f32,
        h: mh as f32,
        image: VideoFrame {
            width: mw as u32,
            height: mh as u32,
            pixels,
            delay_ms: frames[0].delay_ms,
        },
    })
}

/// The mosaic as frame `view` shows it, `w × h` RGBA — what the renderer
/// draws for the plate in that frame.
pub fn warp(m: &Mosaic, view: View, w: u32, h: u32) -> Vec<u8> {
    let mut level = Level::from_frame(&m.image);
    level.origin = (m.x, m.y);
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let q = view.to_reference((x as f32 + 0.5, y as f32 + 0.5));
            let i = ((y * w + x) * 4) as usize;
            if let Some(rgb) = level.sample(q) {
                for c in 0..3 {
                    out[i + c] = rgb[c].round().clamp(0.0, 255.0) as u8;
                }
                out[i + 3] = 255;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A smooth synthetic texture, zoomed by a known camera, comes back
    /// with the camera that made it.
    #[test]
    fn a_known_zoom_and_pan_is_recovered() {
        let (w, h) = (160u32, 120u32);
        let texture = |x: f32, y: f32| -> [u8; 3] {
            let r = 128.0 + 100.0 * (x * 0.11).sin() * (y * 0.07).cos();
            let g = 128.0 + 100.0 * (x * 0.05 + y * 0.09).sin();
            let b = 128.0 + 100.0 * ((x - y) * 0.06).cos();
            [r as u8, g as u8, b as u8]
        };
        let truth: Vec<View> = (0..12)
            .map(|t| {
                let s = 1.0 + 0.015 * t as f32;
                View { scale: s, offset: (80.0 * (1.0 - s) + 0.8 * t as f32, 60.0 * (1.0 - s) - 0.5 * t as f32) }
            })
            .collect();
        let frames: Vec<VideoFrame> = truth
            .iter()
            .map(|v| {
                let mut pixels = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h {
                    for x in 0..w {
                        let q = v.to_reference((x as f32 + 0.5, y as f32 + 0.5));
                        let c = texture(q.0, q.1);
                        pixels.extend_from_slice(&[c[0], c[1], c[2], 255]);
                    }
                }
                VideoFrame { width: w, height: h, pixels, delay_ms: 33 }
            })
            .collect();
        let got = track(&frames, 0);
        for (t, (g, v)) in got.iter().zip(&truth).enumerate() {
            for c in [(0.0, 0.0), (w as f32, 0.0), (0.0, h as f32), (w as f32, h as f32)] {
                let (a, b) = (g.to_frame(c), v.to_frame(c));
                let err = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
                assert!(err < 0.35, "frame {t}: corner {c:?} off by {err:.2} px ({g:?} vs {v:?})");
            }
        }
        assert!(motion_px(&got, w, h) > MIN_CAMERA_MOTION_PX);
    }
}
