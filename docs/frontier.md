# The frontier — what VAC gets wrong, measured

Every number here was produced by `vac verify` on 2026-08-22 at v0.9 (corpus
table re-measured 2026-10-13 at v1.17, default quality 50) and is
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

## 1. Content that changes every frame

**Status: open, much reduced. Corpus row: `mandelbrot.mp4`.**

v1.10's camera turned the Mandelbrot zoom from 33 flat shapes at 12.0 dB
into one image under a measured zoom at **26.4 dB** (ink IoU 0.770 →
0.990). What remains is what a camera cannot explain. The zoom reveals
detail the first frames never showed, and the set's interior colours cycle
from frame to frame. The single mosaic, the median of every frame warped
back, is blurred by the first and averages away the second (frame 89 is
the worst, at 24.5 dB). A finer mosaic, or plates that change over time,
would be next.

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
| `real_pendulum.gif`, v1.9 | 0.640 | 0.541 | 334.8 | 5.0% |
| `real_double_pendulum.gif`, v0.9 | 0.071 | 0.039 | 8425.9 | **616.6%** |
| `real_double_pendulum.gif`, v1.7 | 0.498 | 0.355 | 560.5 | 9.7% |
| `real_double_pendulum.gif`, v1.9 | 0.594 | 0.402 | 464.2 | 8.3% |
| `real_double_pendulum.gif`, v1.11 | 0.621 | 0.385 | 406.3 | 6.2% |

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

## 3. Scale: one long shot

**Status: open, narrowed to forced splits.**

v1.14 searches HD frames small, and v1.16 streams any MP4 over 150M pixels
a segment at a time, with segments at the clip's cuts and the clip held
whole nowhere. 12 s of 720p takes 188 s in 1.75 GB; held whole, it had not
finished after 25 minutes. What remains is a single shot longer than the
budget allows (5.4 s of 720p). It is split into equal slices that are
decompiled independently, so a movement crossing a slice boundary restarts
there. In a deliberately tiny-budget test (1 s slices) the sliding card
lost 9 dB to each slice's static layer reading the slow card as scenery.
Overlapping the slices, or carrying the static layer and tracks across them,
would close this.

---

## 4. Motion during a dissolve

**Status: open. Corpus row: `slideshow_dissolve.mp4`.**

v1.17 writes a dissolve as the frame before it and the frame after it,
crossfading. Both pictures are frozen, so a camera still moving through the
dissolve isn't followed. The slideshow's worst frame (21.1 dB, against
29.1 for the clip) is mid-dissolve, where the first photo is still zooming.
Carrying each shot's camera, or its shapes, into the dissolve under the
fitted crossfade would follow it.

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

- **v1.8 — a mover no flat fill describes keeps its own pixels.** A shape
  that only moves can be replaced by a *sprite*: the source's own pixels,
  captured as a per-pixel median over every frame it is on screen in a
  window that follows its keyframes, as a PNG `image()` that keeps those
  keyframes. Three measured rules gate it. The flat fill must fail on the
  shape's interior (under 30 dB, two pixels in from every edge). The sprite
  must reconstruct the shape's footprint at least twice as well. And it must
  buy ~0.04 dB of clip PSNR per KB at quality 50. `real_bouncing_ball` 40.86
  → **44.05 dB** for 1.1 KB; the new `sprite.vac` fixture, a textured tile,
  26.3 → **36.5 dB** (37.2 dB through MP4). Flat content never takes one.
  The same slice found that twelve of the hermetic tier's twenty recorded
  floors were never asserted; one test now walks every fixture.

- **v1.9 — the fit is whatever reconstructs best.** (Was open item 3.)
  Every shape's geometry used to be estimated from its blob and then final.
  On `outlined.vac` the 3 px ring came back with radius 49.8 for 50, which
  cost 5.7 dB. Each shape's size, position and stroke width are now nudged
  by coordinate descent in 0.5 / 0.2 / 0.1 px steps, scored like the audit,
  and a nudge is kept only when the clip reconstructs better. The ring
  lands on 50.0: `outlined.vac` 40.57 → **47.80 dB** at IoU **1.0000**.
  `spin.vac` residual 23.83 → **1.12** (IoU 0.927 → 0.993); `real_loader`
  25.75 → **28.32 dB**, IoU 0.732 → **0.864**; `real_double_pendulum` IoU
  0.505 → **0.594**; `motion_graphics` 39.46 → **41.76 dB**. The test knocks
  three numbers off a perfect reconstruction and refinement has to put each
  back to within 0.1 px from the pixels alone.

- **v1.10 — a camera.** (Was half of open item 1.) A clip whose whole
  picture moves together is one image seen through a moving camera. Each
  frame's view of the scene (`p = scale · q + offset`) is measured by a
  robust coarse-to-fine pattern search. Every frame is warped back and the
  per-pixel median is the scene, a mosaic that can be larger than the frame.
  The views are re-measured against that mosaic, because frame 0's own movers
  biased the first estimate by 1.5 px, and the camera becomes `position()` +
  `scale()` keyframes on one `image()`. Two measured gates keep it honest.
  The camera has to move the canvas by 2 px or more, which a dozen-frame
  probe settles quickly for still clips. And it has to explain at least 20%
  of the picture better than a still camera: `sprite.vac`'s lone textured
  tile drags the tracker along but scores 3%. Then, like any plate, it must
  reconstruct twice as well as the vectors. `mandelbrot` 12.0 → **26.4
  dB**; the new `camera.vac` (a zoomed scene with a ball crossing it) comes
  back as exactly two keyframes and the ball, at 48.3 dB (43.2 dB through
  MP4, against 22.4 dB for a still plate). v1.10.1 added two photographic
  camera clips to the corpus, a Ken Burns zoom (**30.3 dB**, 63 KB, against
  13.5 dB in 845 KB of vectors) and a slow pan (**42.5 dB**). It also reads
  each mosaic point from the frames that see it most magnified, rather than
  letting a zoom's blurry early frames outvote its sharp late ones.

- **v1.11 — motion, measured from the pixels.** The tracker's keyframes
  come from blob centroids and second moments, which drift when a blob is lit
  unevenly or crossed. Each mover's own pixels are now registered against
  every frame it's on screen: a rigid turn and translation, by robust pattern
  search seeded from its keyframes. The track is re-keyframed and kept only
  when the clip measures better and the gain pays for any added keyframes.
  `real_bouncing_ball` 44.36 → **46.37 dB**; `real_double_pendulum` IoU
  0.594 → **0.621**; `crossing.vac` IoU 0.979 → **0.9999**.

- **v1.12 — shots.** Every clip used to be decompiled as one scene, with a
  static layer and tracks smeared across the cuts. Frames that differ from
  the one before at least ten times more than usual are now cut candidates.
  Each shot is decompiled on its own by the whole pipeline, and the shots
  become scenes, each exactly as long as its frames. The split is kept when
  it measures better than one scene. The new corpus clip `cuts_three_shots`
  (three clips spliced) went 22.30 → **29.17 dB**, IoU 0.746 → **0.956**,
  104 → 8 shapes; the `shots.vac` fixture comes back as its three scenes at
  51.6 dB.

- **v1.13 — fades.** A fade blends the whole picture toward black or
  white, and it used to break every stage after it: a faded-in photo pan lost
  its camera (frame 0 was black). The fade is read two ways (frame means,
  and chained per-pixel ratios split at flat frames) and taken out. The
  picture is decompiled with shots, camera and all, and the fade is drawn
  back on top, its opacity fitted per frame to the reconstruction by least
  squares. The new corpus clip `fade_through_black` (a card fading out, a
  photo pan fading in) went 25.88 → **42.22 dB** as two faded scenes; the
  faded card 24.36 → 45.37 dB, the faded pan 26.23 → 42.62 dB.

- **v1.14 — HD frames are searched small.** (Half of open item 3.) Above
  500k pixels the measured search runs on a copy shrunk by a whole factor
  and only its decision is replayed at full size. Camera tracking stops at
  ~700 px, the mosaic samples frames directly, and the fade pass is gated on
  the frame means (a pan had been read as a fade, at the cost of a second
  decompile). A 3 s 720p pan: 64 → **33 s**, 2.44 → **1.66 GB**, 44.3 →
  44.7 dB. The corpus is byte-for-byte unchanged.

- **v1.15 — a camera that rolls.** A view is `scale · R(angle) · q +
  offset`, tracked with turns about the frame centre. It is written as the
  plate's `rotate()` keyframes, simplified against what the renderer will
  draw: the renderer turns an image about its box's centre, so position
  follows the angle. A camera that never turns by a quarter degree is
  re-measured without turns and stays `rotate()`-free. The new corpus clip
  `roll_watercolor` (a photo rolling 26°) went 18.49 → **33.25 dB**, 588
  shapes → one plate.

- **v1.16 — long clips, a segment at a time.** An MP4 over 150M pixels is
  scanned once (frame count, the steps between frames where the cuts are,
  the clip's modal colour for the metric) and then streamed. Each segment, a
  shot or a slice of a long one, is decompiled by the whole pipeline and
  dropped. 12 s of three-shot 720p: **188 s and 1.75 GB**, where holding it
  whole had not finished in 25 minutes. Loading no longer buffers the clip
  twice. The test requires a shot-at-a-time decompile of `shots.vac` to match
  it held whole exactly.

- **v1.17 — dissolves.** A dissolve is a cut spread over frames. Frames a
  window apart now count as one when they differ like a cut while every
  frame between is a blend of them, with every changing pixel moving by the
  same share at once (a sliding card fits the blend in the mean and fails
  that). The dissolve becomes a scene of its own, its two end pictures
  crossfading with a least-squares opacity. The new corpus clip
  `slideshow_dissolve` (a Ken Burns zoom dissolving into a pan) went 19.06 →
  **29.09 dB**, 371 shapes → 6.

## Corpus, in full

From `benches/corpus-baseline.tsv`. Regenerate with
`VAC_UPDATE_BASELINES=1 cargo test --release -p vac --test corpus`.

| input | ink IoU | worst | residual | psnr dB | spurious | shapes |
|---|---|---|---|---|---|---|
| `slideshow_dissolve.mp4` | 0.9955 | 0.9634 | 80.1 | 29.09 | 0.3% | 6 |
| `testsrc2.mp4` | 0.9951 | 0.9859 | 372.7 | 22.42 | 0.4% | 64 |
| `roll_watercolor.mp4` | 0.9951 | 0.9921 | 30.8 | 33.25 | 0.4% | 1 |
| `slide_in_card.mp4` | 0.9940 | 0.9286 | 2.2 | 44.63 | 0.2% | 2 |
| `kenburns_watercolor.mp4` | 0.9921 | 0.9899 | 60.9 | 30.29 | 0.6% | 2 |
| `fade_through_black.mp4` | 0.9918 | 0.9286 | 3.9 | 42.22 | 0.4% | 5 |
| `real_bouncing_ball.gif` | 0.9905 | 0.9885 | 1.5 | 46.37 | 0.6% | 2 |
| `mandelbrot.mp4` | 0.9898 | 0.9843 | 149.2 | 26.39 | 0.5% | 2 |
| `outlined_ui.mp4` | 0.9847 | 0.9689 | 26.8 | 33.85 | 1.0% | 4 |
| `motion_graphics.mp4` | 0.9773 | 0.9565 | 4.3 | 41.76 | 1.5% | 4 |
| `vac_control.mp4` | 0.9707 | 0.9607 | 1.4 | 46.58 | 0.0% | 3 |
| `pan_portrait.mp4` | 0.9699 | 0.9580 | 3.6 | 42.51 | 1.5% | 1 |
| `cuts_three_shots.mp4` | 0.9555 | 0.8438 | 78.8 | 29.17 | 0.7% | 8 |
| `real_newtons_cradle.gif` | 0.9204 | 0.9047 | 65.8 | 29.95 | 5.6% | 47 |
| `real_loader.gif` | 0.8641 | 0.8398 | 95.8 | 28.32 | 11.6% | 16 |
| `real_pendulum.gif` | 0.6430 | 0.5445 | 331.5 | 22.93 | 4.8% | 27 |
| `real_double_pendulum.gif` | 0.6211 | 0.3853 | 406.3 | 22.04 | 6.2% | 419 |

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
