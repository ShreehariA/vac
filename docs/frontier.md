# The frontier — what VAC gets wrong, measured

Every number here was produced by `vac verify` on 2026-08-22 at v0.9 (corpus
table re-measured 2026-10-12 at v1.1, default quality 50) and is
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

## 1. Strokes are never recovered

**Status: open. Test: `strokes_are_never_recovered`.**

`emit_shape` writes `stroke = none` unconditionally, and the renderer
hard-codes stroke width at 2.0 with no way to express it in source. A stroked
shape comes back as a bare fill.

---

## 2. Smooth gradients have no primitive

**Status: open. Corpus rows: `mandelbrot.mp4`, `real_newtons_cradle.gif`.**

Every fill in VAC is one flat colour. The Mandelbrot zoom is radial colour
ramps on a pink radial background; the Newton's cradle is a greyscale
photograph. Each is approximated by flat patches, and v0.13's measured
search shows how little the choice of patches matters: colour-aware regions
shatter the Mandelbrot into 289 shapes and 261 KB of source while making it
*worse* (IoU 0.67), so the search keeps the coarse decomposition.

| | ink IoU | worst | residual |
|---|---|---|---|
| `mandelbrot.mp4` | 0.765 | 0.396 | 4186.4 |
| `real_newtons_cradle.gif` | 0.641 | 0.635 | 542.7 |

PAC already fits linear gradients (`pac_decompiler::gradient`). A `gradient`
fill in the language is the obvious next primitive; the planned `raster()`
escape hatch is the fallback for what no vector primitive can say.

---

## 3. Dotted and dashed strokes fragment

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

- **v0.11 — `scale()` and `opacity()` are recovered, and the renderer holds
  transforms.** `apply_animation` now interpolates each transform kind
  between the keyframes that carry it. The decompiler emits a scale channel
  (sqrt of area vs median, aspect-guarded against merges) and an opacity
  channel (projection onto the background→fill segment). The static layer no
  longer claims the core of a compact growing shape. `grow.vac` 0.644 →
  **0.970** (10 shapes → 1); `fade.vac` residual 59.8 → **0.01**. No corpus
  row moved: nothing in the corpus scales or fades cleanly, so this is a
  hermetic win only, stated as such.

- **v0.12 — objects are only drawn during their lifetime.** A track seen late
  or lost early without touching the edge is hidden with `opacity(0)` outside
  its lifetime. `pop_in.vac` 0.662 → **0.988**. In the corpus this cut the
  pendulums' spurious ink by stopping short fragment tracks from persisting
  all clip: `real_pendulum.gif` 0.191 → **0.376** (spurious 228% → 45%),
  `real_double_pendulum.gif` 0.079 → **0.174** (559% → 136%). The dotted-stroke
  fragmentation itself (row 4) is still open. This only stopped the fragments
  from outliving themselves.

- **v0.13 — colour: regions, a real background, and a measured choice.**
  Blobs were connected components of "not background", which is colour-blind,
  and the background was a per-channel median of four corners. v0.13 adds
  colour-aware region labelling (edge fringes absorbed), a border-mode
  background, and primitive choice by measured IoU instead of fill-ratio
  bands. Because each of these helps some clips and hurts others, the facade
  decompiles every combination, re-renders each, and keeps the one that
  reconstructs the source best. `testsrc2.mp4` residual 15063 → **1711**
  (8.8×), `real_double_pendulum.gif` IoU 0.174 → **0.410**,
  `real_newtons_cradle.gif` residual 706 → 543, `two_tone.vac` residual
  6676 → **83**. No row regressed.

- **v0.13 (metric) — the ink metric on transparent canvases.** A *ruler*
  change, recorded separately from every decompiler change.
  `modal_background` now lets transparency vote as a colour; when it wins,
  ink is measured against white, the colour a viewer composites onto. Before,
  the vote went to the most common *opaque* colour, the `real_loader`
  spinner's tick grey, so its IoU measured three highlighted ticks out of
  twelve. Exactly one corpus row moved, `real_loader.gif`: IoU 0.501 → 0.698,
  spurious 76% → 23%, residual unchanged at 258.7 (residual never uses the
  reference colour). The reconstruction did not change at all. Only the
  ruler did, and it now measures what it claims to.

- **v0.14 — rotation.** The language gains `rotate(deg)` (clockwise, about
  the shape's centre). The detector reads each blob's principal axis and
  extent from its second moments, scores an oriented rect and ellipse
  against the pixels alongside the axis-aligned ones, and the tracker
  unwraps the angle series into a channel. The static layer's
  bleeding-core test was loosened (ring fraction 0.5 → 0.2): a spinning
  bar's always-covered centre had been stripped as scenery, cutting the bar
  in half. `spin.vac` 0.353 → **0.917**; `testsrc2.mp4` residual 1711 →
  **1349** (its colour band rotates); `real_pendulum.gif` 0.376 → **0.420**
  (residual 872 → 657); `real_double_pendulum.gif` 0.410 → **0.457**. No
  row regressed.

- **v1.1 — sizes measured from coverage; repeated round trips reach a fixed
  point; ground.** A shape moving at sub-pixel speed straddles a pixel on
  both edges in most frames, so the bbox of thresholded pixels read it about
  a pixel wide. The median rounded up, and every round trip grew it
  (`two_balls` radius 22 → 27 over five generations, `sliding_rect` 120 →
  130). Axis-aligned rects and ellipses are now sized from coverage, with
  each pixel counted by how much of it the shape covers, read from its
  colour. Rects use a coverage-corrected bbox, unaffected by notches.
  Ellipses use coverage-weighted variance. Shaded blobs, transparent edges,
  low contrast, and blobs that fit their primitive poorly all keep the bbox.
  Generation 1 lands at the source's exact size and stays there.
  Separately, regions a few shades off the background are found in the
  static plate's "ground" band and drawn first. Through v1.0 they were
  painted the background colour.
  - Fixtures: `two_balls` 0.952 → **0.989**, `diagonal` 0.916 → **0.991**,
    `crossing` 0.911 → 0.946, `sliding_rect` 0.983 → 0.999, `panel.vac`
    (new) residual 40.9 → **0.44**.
  - Corpus: `vac_control` residual 9.1 → **3.0**, `real_double_pendulum`
    residual 1034 → **766**.
  - **One recorded trade-off:** `real_double_pendulum` ink IoU 0.463 →
    0.435. Its 1.3 px anti-aliased plot-frame lines are now declared at
    their measured width, so they no longer paint their light fringe solid
    dark. That fringe was counted as "ink" but painted the wrong colour.
    Residual is the measure that sees colour, and it improved 26%.

## Corpus, in full

From `benches/corpus-baseline.tsv`. Regenerate with
`VAC_UPDATE_BASELINES=1 cargo test --release -p vac --test corpus`.

| input | ink IoU | worst | residual | psnr dB | spurious | shapes |
|---|---|---|---|---|---|---|
| `slide_in_card.mp4` | 0.9918 | 0.8792 | 3.0 | 43.42 | 0.3% | 2 |
| `testsrc2.mp4` | 0.9871 | 0.9771 | 1347.6 | 16.84 | 1.2% | 202 |
| `motion_graphics.mp4` | 0.9762 | 0.9562 | 7.4 | 39.46 | 0.8% | 4 |
| `vac_control.mp4` | 0.9620 | 0.9493 | 3.0 | 43.35 | 0.5% | 3 |
| `real_bouncing_ball.gif` | 0.8686 | 0.8662 | 419.9 | 21.90 | 14.0% | 3 |
| `mandelbrot.mp4` | 0.7640 | 0.3963 | 4166.1 | 11.93 | 13.7% | 43 |
| `real_loader.gif` | 0.6998 | 0.6706 | 240.7 | 24.32 | 10.3% | 16 |
| `real_newtons_cradle.gif` | 0.6419 | 0.6369 | 525.3 | 20.93 | 46.9% | 202 |
| `real_double_pendulum.gif` | 0.4347 | 0.2947 | 766.5 | 19.29 | 39.4% | 379 |
| `real_pendulum.gif` | 0.4298 | 0.3268 | 646.4 | 20.03 | 11.8% | 48 |

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
