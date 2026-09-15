# The frontier — what VAC gets wrong, measured

Every number here was produced by `vac verify` on 2026-08-22 at v0.9 (corpus
table re-measured 2026-09-15 at v0.10) and is
locked by a test in `crates/vac/tests/frontier.rs` or
`benches/corpus-baseline.tsv`. Rows leave this file when they are fixed, not
when they become inconvenient.

**Why this file exists.** Through v0.8, VAC measured itself with proxies —
shape counts, animate-block counts, keyframe counts, `.vac` byte size. Those
describe how *small* the output is. None of them describes how *right* it is,
and a proxy can improve while the picture gets worse. v0.9 added the missing
axis, and the first thing it did was find defects that eight versions of
count-based testing had walked straight past.

## How to read the numbers

| Metric | Meaning | Good |
|---|---|---|
| `ink IoU` | Overlap between the source's ink and the reconstruction's, as masks | → 1.0 |
| `worst IoU` | The single worst frame. A tracking collapse is invisible in a mean | → 1.0 |
| `residual` | Per-channel mean squared error. 100 ⇒ each R/G/B within ~10 | → 0 |
| `missing ink` | Fraction of the source's ink never painted | → 0% |
| `spurious ink` | Ink painted where the source had none, as a fraction of source ink | → 0% |

`ink IoU` and `residual` answer different questions and it takes both. A shape
drawn in exactly the right place in entirely the wrong colour scores IoU ≈ 1.0
and a terrible residual. A shape drawn in the right colour in the wrong place
scores the reverse.

---

## 1. Repeated round-trips inflate shapes without converging

**Status: open. Test:
`repeated_round_trips_do_not_drift_worse_than_the_recorded_defect`.**

Feeding the pipeline its own output does not reach a fixed point. Measured on
`two_balls.vac`, ink IoU against the original falls every generation:

| generation | 1 | 2 | 3 | 4 |
|---|---|---|---|---|
| ink IoU vs original | 0.9523 | 0.9149 | 0.8788 | 0.8459 |

Because each recovered ellipse comes back one pixel wider than the one that
drew it, forever:

```
two_balls.vac    rx: 22 → 23 → 24 → 25 → 26 → 27     (ry pinned at 22)
sliding_rect.vac  w: 120 → 122 → 124 → 126 → 128 → 130  (h pinned at 60)
```

Anyone using VAC iteratively — decompile, edit, recompile, decompile — watches
their shapes swell.

**What is not yet established is the trigger.** A fully static shape never
drifts (the card in `static_and_moving.vac` is stable to the pixel across five
generations). But neither does the *moving* ellipse in that same fixture, even
though it travels horizontally exactly like the two that do inflate. So "moves
along an axis" is not the rule, and the real condition is still open. It is
recorded here undiagnosed rather than explained away — v0.7 shipped a confident
diagnosis of the pendulum that v0.8 proved wrong, and once was enough.

---

## 2. `scale()` and `opacity()` are never recovered

**Status: open. Test: `scale_and_opacity_are_never_emitted`.**

The language has had both since v0.1 and `vac-compiler` renders both. The
decompiler emits only `Transform::Position`. Across all 26 `.vac` files in this
repository there are **1654 `position(`, zero `scale(`, zero `opacity(`**.

A shape that grows, shrinks, or fades round-trips as a constant-size, fully
opaque shape. Real footage is full of both.

**Blocked on a renderer bug that must be fixed first.** `apply_animation` in
`crates/vac-compiler/src/render.rs` consults only the two bracketing keyframes
and pulls each transform kind independently. If neither bracketing keyframe
carries a `Scale`, the shape reverts to base scale 1.0 rather than holding the
last value set. This is latent today precisely *because* only one transform
kind is ever emitted; it becomes a live bug the moment a second one is.

---

## 3. Strokes are never recovered

**Status: open. Test: `strokes_are_never_recovered`.**

`emit_shape` writes `stroke = none` unconditionally, and the renderer
hard-codes stroke width at 2.0 with no way to express it in source. A stroked
shape comes back as a bare fill.

---

## 4. Colour: the single-flat-background model

**Status: open. Best-evidenced fidelity gap. Corpus rows: `testsrc2.mp4`,
`mandelbrot.mp4`.**

VAC's entire background model is four 5×5 corner blocks reduced to a
**per-channel** median (`detect::estimate_background`) — one flat `Color` per
frame — with everything else classified foreground by distance from it.
Because the channels are medianed independently, the result can name a colour
that appears nowhere in the frame.

| | ink IoU | residual | psnr |
|---|---|---|---|
| `testsrc2.mp4` (SMPTE colour bars) | 0.840 | **15127.9** | 6.33 dB |
| `mandelbrot.mp4` | 0.716 | 4644.6 | 11.46 dB |

A residual of 15127 means each channel is off by roughly 123 out of 255. The
IoU of 0.84 looks respectable and is measuring the right thing: the *ink* is
broadly in the right places. The colour is what is destroyed — the bars
reconstruct as a flat grey field. This is the clearest case in the corpus for
reporting both numbers rather than one.

**The fix is mostly reuse.** PAC — which `vac-decompiler` already depends on —
has `kmeans::quantize_auto` and `regions::label_regions_per_cluster`. v0.6
merged PAC into VAC and took its contour tracer, RDP and Bézier fitter — its
pen — while leaving its eyes behind.

---

## 5. Dotted and dashed strokes fragment

**Status: open. Corpus rows: `real_pendulum.gif`, `real_double_pendulum.gif`.**

A dotted stroke rasterises into dozens of interchangeable ~4 px blobs and the
matcher links dot *k* to dot *k+3*. Diagnosed in v0.8 (correcting v0.7, which
had blamed static dashes blinking under a sweeping arm).

| | ink IoU | worst | residual | spurious |
|---|---|---|---|---|
| `real_pendulum.gif` | 0.158 | 0.101 | 2122.7 | **230.1%** |
| `real_double_pendulum.gif` | 0.071 | 0.039 | 8425.9 | **616.6%** |

The double pendulum paints **more than six times** the ink its source had.

This row is worth dwelling on, because v0.8 recorded the double pendulum as its
headline win: 45 → 19 animate blocks, 1159 → 186 keyframes, 54 KB → 25 KB. All
of that is true. It is also entirely a *compression* result. Measured on the
other axis, the same clip reconstructs at an ink IoU of 0.07. Both facts were
always true; only one of them was measurable.

---

## Fixed

- **v0.10 — objects that appear after t=0 were time-shifted.** Keyframes were
  re-based to each track's first sighting, so `late_entry.vac` parked its card
  at 840 ms instead of 2000 ms (IoU 0.394, worst frame 0.000). Now absolute,
  with edge-clipped entries/exits held off-canvas and clipped centroids
  corrected. `late_entry.vac` 0.394 → **0.9928**; `slide_in_card.mp4` 0.818 →
  **0.9918**, worst frame 0.000 → 0.879, residual 157.4 → 3.0.

## Corpus, in full

From `benches/corpus-baseline.tsv`. Regenerate with
`VAC_UPDATE_BASELINES=1 cargo test --release -p vac --test corpus`.

| input | ink IoU | worst | residual | psnr dB | spurious | shapes |
|---|---|---|---|---|---|---|
| `slide_in_card.mp4` | 0.9918 | 0.8792 | 3.0 | 43.42 | 0.3% | 2 |
| `motion_graphics.mp4` | 0.9762 | 0.9562 | 7.4 | 39.46 | 0.8% | 4 |
| `vac_control.mp4` | 0.9596 | 0.9449 | 9.1 | 38.54 | 2.9% | 3 |
| `real_bouncing_ball.gif` | 0.8758 | 0.8736 | 413.8 | 21.96 | 13.0% | 3 |
| `testsrc2.mp4` | 0.8398 | 0.8387 | 15004.6 | 6.37 | 19.1% | 76 |
| `mandelbrot.mp4` | 0.7235 | 0.6469 | 4497.5 | 11.60 | 32.4% | 46 |
| `real_newtons_cradle.gif` | 0.5996 | 0.5947 | 731.3 | 19.49 | 57.0% | 186 |
| `real_loader.gif` | 0.5012 | 0.4766 | 258.9 | 24.00 | 76.2% | 16 |
| `real_pendulum.gif` | 0.1906 | 0.1045 | 2059.5 | 14.99 | 228.3% | 111 |
| `real_double_pendulum.gif` | 0.0792 | 0.0476 | 7627.0 | 9.31 | 559.1% | 92 |

The two clips at the top are VAC's own output and ffmpeg `drawbox` rectangles —
flat-shaded content of exactly the kind the detector was built for. The spread
between those and the bottom of the table is the honest statement of where VAC
currently stands.

---

## What the two test tiers can and cannot see

Verified by deliberately breaking the pipeline and checking which tier notices:

| Perturbation | Hermetic tier | Corpus tier |
|---|---|---|
| `FG_THRESHOLD_SQ` 24² → 60² | **caught** (`occlusion.vac` IoU 0.9997 → 0.9839) | caught |
| `RDP_TOLERANCE_PX` 2.0 → 20.0 | missed | **caught** (`real_loader` IoU 0.5012 → 0.2258) |

The hermetic fixtures have simple linear trajectories, so ten-fold coarser
keyframe simplification still reproduces them; `real_loader`'s motion does not
survive it. Neither tier subsumes the other, which is the argument for keeping
both.

This table is also the reason the hermetic floors are **per-fixture**. An
earlier draft used one global floor of 0.80 against measured values of
0.91–0.9997, and under it the `FG_THRESHOLD_SQ` break passed all twelve tests.
A test with that much slack does not fail when the pipeline breaks — which is
the same defect as `fitted_curve_stays_within_tolerance`, whose body computed
`dist(p, p)` and could never fail at all until v0.9 repaired it.
