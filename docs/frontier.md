# The frontier — what VAC gets wrong, measured

Every number here was produced by `vac verify` on 2026-08-22 at v0.9 (corpus
table re-measured 2026-10-12 at v1.7, default quality 50) and is
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

## 1. Fractals and full-frame motion

**Status: open. Corpus row: `mandelbrot.mp4`.**

v1.2 gradients describe lit and fading surfaces, and v1.6's raster plate
covers static scenery no vector can describe (photographs, textures). What
remains is content that is neither vector-shaped nor static. The Mandelbrot
zoom changes every pixel every frame, so there is no static plate to keep
(a plate scores 12.7 dB against the vector 11.9 dB, short of the bar) and no
shape to track. A codec for it would need per-frame raster or a description
of the fractal itself.

---

## 2. Dotted and dashed strokes fragment

**Status: open, much reduced. Corpus rows: `real_pendulum.gif`,
`real_double_pendulum.gif`.**

A dotted stroke rasterises into dozens of interchangeable ~4 px blobs and the
matcher links dot *k* to dot *k+3*. Diagnosed in v0.8 (correcting v0.7, which
had blamed static dashes blinking under a sweeping arm).

| | ink IoU | worst | residual | spurious |
|---|---|---|---|---|
| `real_pendulum.gif`, v0.9 | 0.158 | 0.101 | 2122.7 | **230.1%** |
| `real_pendulum.gif`, v1.7 | 0.622 | 0.518 | 357.5 | 6.6% |
| `real_double_pendulum.gif`, v0.9 | 0.071 | 0.039 | 8425.9 | **616.6%** |
| `real_double_pendulum.gif`, v1.7 | 0.498 | 0.355 | 560.5 | 9.7% |

At v0.9 the double pendulum painted **more than six times** the ink its
source had. The v1.7 audit takes out every fragment that made the picture
worse, so what is left of this row is a *description* problem: the dashed
traces still come back as hundreds of short-lived fragments (419 shapes on
the double pendulum), each of which measurably helps, and none of which is
the dashed line the source drew.

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

- **v1.2 — gradient fills.** `gradient(linear, #c0 at x,y, #c1 at x,y)` (the
  same syntax as PAC) and `gradient(radial, #c0 at x,y, #c1 radius r)`, in
  the shape's declared coordinates, so the shading moves, scales and turns
  with it. Each shape's interior pixels are fitted with a flat fill, a
  least-squares linear model, and a radial model whose centre is found by
  coarse-to-fine grid search. A gradient is kept only if it removes at least
  60% of the flat error and leaves at most 25 levels RMS. The first version
  accepted any gain over 35%, and the Mandelbrot clip lost 0.05 ink IoU to
  gradients smeared over its fractal. `real_bouncing_ball` 21.9 → **25.0 dB**,
  IoU 0.869 → **0.885** (now above v0.12's 0.876). `shaded.vac` (new)
  residual 390 → **21**. No row regressed.

- **v1.3 — colour animation.** A `fill(#rrggbb)` keyframe transform,
  interpolated in RGB like any other channel. A track whose interior colour
  strays more than 24 units from its median carries the channel, unless an
  opacity fade already explains it. A colour jump that coincides with an area
  jump is the tracker changing regions, not a colour animation. The first
  draft painted one such jump on the Mandelbrot zoom and collapsed its worst
  frame from 0.40 to 0.09 IoU, while the mean moved 0.005 and passed. The
  corpus gate now checks worst-frame IoU too. `color_shift.vac` (new)
  residual 101.7 → **1.27** (28.1 → 47.1 dB).

- **v1.4 — outlines become strokes, and positions became exact.**
  `stroke_width` joins the language. A blob enclosing a large hole whose
  filled outline is a genuine rect or ellipse is emitted as that primitive,
  unfilled and stroked along its centreline. Its width is solved from
  coverage-weighted ring area and outer size, and an outline framing a fill
  that moves with it becomes one shape with both. On a new corpus clip with
  outlines drawn by ffmpeg, `outlined_ui.mp4`, IoU went 0.696 → **0.984**
  and residual 854 → **28.7**.
  - Chasing that clip's last error turned up a bug older than any of this:
    blob centroids are means of pixel *indices*, half a pixel short of pixel
    centres. Whole-pixel rounding had been hiding it by luck. Anchors now use
    continuous coordinates and positions are written to 0.1 px. Six fixtures
    are now within 0.001 of perfect (`sliding_rect` and `late_entry`
    1.0000 / 0.00), `vac_control` went 38.5 → **46.6 dB**, and `testsrc2`
    1341 → **1215**.
  - The first cut at sub-pixel positions parked off-canvas shapes exactly at
    the edge, leaving a half-covered column on screen. The worst-frame gate
    added in v1.3 caught it on `slide_in_card` (0.88 → 0.00), and shapes now
    park a full pixel clear.

- **v1.5 — a third segmentation, and a leaner search.** Colour-similarity
  region growing chains across a gradual anti-aliased transition. On
  `real_pendulum` the grey bob and the green arrow drawn over it fused into
  one mixed-colour blob. `Segmentation::Edges` lets a pixel on a strong edge
  join only other edge pixels, and absorbs a thin region only when it wraps
  a solid one (shared border ≥ 40% of its area), not when it merely crosses
  it. It is a third candidate in the measured search, not a fixed rule. It
  wins on `real_double_pendulum` (IoU 0.437 → **0.492**, invented ink 39% →
  25%) and `real_newtons_cradle` (residual 520 → **455**). It loses on the
  pendulum it was written for: the bob is still cut in two by the rod and
  arrow drawn across it, which is an occlusion problem rather than a
  segmentation one. The search now prepares the static layer once per
  background rule and runs candidates concurrently on shared frames. Peak
  memory on the double pendulum went 2.2 GB → **1.0 GB**, and the corpus run
  90 s → 26 s.

- **v1.6 — a raster plate, chosen by measurement.** A new `image()`
  primitive embeds a JPEG data URI. When the best all-vector reconstruction
  is under 30 dB, the search also builds a raster plate: the clip's median
  frame as one `image()`, with every pixel matching it stripped from the
  frames, so only true movers remain as code on top. It is kept when it
  reconstructs at least twice as well by the selection rule's own measure.
  Four clips take it: `real_bouncing_ball` 26.0 → **40.9 dB**,
  `real_newtons_cradle` 21.5 → **29.4 dB** (IoU 0.64 → **0.92**, and *fewer*
  bytes), `testsrc2` 17.3 → **22.2 dB** (255 KB → **94 KB**), `real_pendulum`
  20.1 → **22.2 dB** (IoU 0.44 → **0.63**). Flat content never takes a plate.
  `shaded.vac`'s gradients, already 39.5 dB as code, stay code.

- **v1.7 — every shape earns its place, measured.** Through v1.6 a shape
  stayed in the output once its track cleared the tracker's gates; nothing
  checked that drawing it helped. Each shape is now taken out of the
  reconstruction in turn, topmost first, and only the rectangle it touches is
  re-rendered without it. The full measurement already holds every frame's
  raw tallies, so the clip's score without the shape follows exactly. It
  stays only if the clip reconstructs better with it, in the selection rule's
  currency. On the corpus the shapes it removed were worse than nothing:
  `real_double_pendulum` 19.70 → **20.64 dB** with spurious ink 24.6% →
  **9.7%** and 192 → **166 KB**; `real_pendulum` 22.18 → **22.60 dB**,
  58 → **49 KB**; `testsrc2` 94 → **55 KB** (46 of 110 shapes out) at
  22.17 → 22.24 dB. One trade: `real_pendulum`'s ink IoU dips 0.626 → 0.622
  while its residual falls 9%.

## Corpus, in full

From `benches/corpus-baseline.tsv`. Regenerate with
`VAC_UPDATE_BASELINES=1 cargo test --release -p vac --test corpus`.

| input | ink IoU | worst | residual | psnr dB | spurious | shapes |
|---|---|---|---|---|---|---|
| `testsrc2.mp4` | 0.9951 | 0.9858 | 387.9 | 22.24 | 0.4% | 64 |
| `slide_in_card.mp4` | 0.9940 | 0.9286 | 2.2 | 44.63 | 0.2% | 2 |
| `real_bouncing_ball.gif` | 0.9883 | 0.9845 | 5.3 | 40.86 | 0.8% | 2 |
| `outlined_ui.mp4` | 0.9839 | 0.9689 | 28.8 | 33.54 | 1.1% | 4 |
| `motion_graphics.mp4` | 0.9762 | 0.9562 | 7.4 | 39.46 | 0.8% | 4 |
| `vac_control.mp4` | 0.9707 | 0.9607 | 1.4 | 46.58 | 0.0% | 3 |
| `real_newtons_cradle.gif` | 0.9190 | 0.9017 | 75.3 | 29.36 | 5.7% | 47 |
| `mandelbrot.mp4` | 0.7658 | 0.3961 | 4148.6 | 11.95 | 12.8% | 33 |
| `real_loader.gif` | 0.7318 | 0.7115 | 173.2 | 25.75 | 9.8% | 16 |
| `real_pendulum.gif` | 0.6215 | 0.5180 | 357.5 | 22.60 | 6.6% | 27 |
| `real_double_pendulum.gif` | 0.4979 | 0.3552 | 560.5 | 20.64 | 9.7% | 419 |

Flat-shaded content of exactly the kind the detector was built for, meaning
VAC's own output and ffmpeg `drawbox` rectangles, reconstructs at 33–47 dB. The
spread between that and the bottom of the table is the honest statement of
where VAC currently stands.

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
