# How VAC decompiles a video

This is the pipeline as of v1.21: what runs, in what order, and what decides
each step. The README is a version-by-version log, and this is the map. One
rule runs through all of it: **measure, don't classify.** Wherever there is a
choice, every option is built, rendered back to pixels, and scored against
the source. The one that reconstructs better wins.

## The ruler

Everything below is decided by one measurement: render a `.vac`, align its
frames to the source's timeline, and compare (`vac_metrics`).

- **residual**: per-channel mean squared error. PSNR is derived from it.
- **ink IoU**: overlap between where the source has ink and where the
  reconstruction does. Ink is any pixel far from the clip's modal colour,
  measured from the source, never from the decompiler's own guesses.

Two reconstructions are compared in one currency, `ln(residual ratio) +
ln(IoU ratio)` (`vac::improvement`), so a gain in one never silently buys a
loss in the other. The audit, refinement, motion and sprite passes score
candidates from per-frame tallies and small region renders (`Rig`), so a
change to one shape costs a few small renders, not a whole clip.

## From the outside in

```
video ──► long?  ──► scan once, then a segment at a time  (v1.16)
            │
            ▼
         fade?   ──► read it, take it out, fit it back on top (v1.13)
            │
            ▼
      cuts / dissolves ──► each shot alone; each dissolve a scene (v1.12, v1.17)
            │
            ▼
         one shot:
            measured search ──► vector candidates × backgrounds  (v0.13)
                             ──► still raster plate / camera     (v1.6, v1.10, v1.19)
            then, on the winner:
              audit   ──► every shape against its own absence    (v1.7)
              refine  ──► every size, position, stroke width     (v1.9)
              motion  ──► every mover registered from its pixels (v1.11)
              sprites ──► a mover no flat fill describes          (v1.8)
```

### Long clips (v1.16)

An MP4 over 150M pixels in total is never held whole. One streaming pass
reads the frame count, each frame's step from the one before (for the cut
detector), and the clip's modal colour (for the metric). A second pass
decompiles one segment at a time: a shot, or an equal slice of a shot too
long for the budget. Large frames (over 500k pixels) are also *searched* on a
copy shrunk by a whole factor, and only the decision is replayed at full size
(v1.14).

### Fades (v1.13)

`frame = α · picture + (1 − α) · colour`. α is read two ways, from the frame
means and from chained per-pixel ratios. The means gate the pass, because a
camera move can fool the chain. The picture is restored by dividing out the
fade, decompiled, and the fade drawn back on top as a canvas-sized rect. Its
opacity is fitted to the reconstruction by least squares, frame by frame.
The result is kept only if it beats decompiling the clip as it is.

### Shots and dissolves (v1.12, v1.17, v1.18)

A **cut** is a frame that differs from the one before it ten times more than
frames usually do. A **dissolve** is a cut spread over frames. Frames a window
apart differ like a cut, every frame between is a blend of them, and in its
middle every changing pixel has moved by the same share. That last test is
what stops a card sliding over a flat background from passing as a dissolve.
Each shot is decompiled alone. Each dissolve becomes a scene between its
shots: the neighbouring shots' camera plates carried through it, or its edge
frames frozen, whichever measures better, crossfading with a least-squares
opacity. On short clips the split is kept only if it beats the clip as one
shot.

### One shot: the measured search

- **Vector candidates.** Three segmentations (connected components, colour
  regions, edge-gated regions) times two background rules, run in parallel.
  Each one finds the static layer (the per-pixel temporal median), detects
  blobs, tracks them, and fits shapes: rect, ellipse or traced path by
  overlap, flat or gradient fill, stroke, rotation, colour animation. A
  track that changes size is written as a uniform `scale(s)`, or as a
  stretch `scale(sx, sy)` when its width and height part for a sustained
  run (v1.21): a progress bar filling, a chart bar rising.
- **Static layer or none (v1.20).** When the winner has static shapes, its
  segmentation and background are decompiled once more without the static
  layer, and that is kept when it measures twice as good: a mover that
  covers the same pixels for most of a shot is scenery to a median. When
  the layer is dropped, every segmentation is re-tried without it (v1.21).
- **Plates**, tried only when the best vector candidate is under 30 dB:
  - a **still plate**: the median frame as one JPEG, movers on top;
  - a **camera**: each frame's view of the scene (scale, roll, offset) by a
    robust coarse-to-fine search, then re-measured against the median
    mosaic; the mosaic becomes one image with `position`/`scale`/`rotate`
    keyframes. The image is the median mosaic or the same mosaic fitted to
    the frames through the renderer's bilinear reading of it by
    back-projection (v1.19), with only its fine detail kept, whichever pays
    for its JPEG bytes on held-out probe frames.

  A plate must reconstruct at least twice as well as the vectors to be kept.

### One shot: the passes on the winner

- **Audit (v1.7).** Each shape is taken out in turn, and it stays only if
  the clip is better with it.
- **Refine (v1.9).** Each shape's size, position offset and stroke width are
  nudged in 0.5, 0.2 and 0.1 px steps toward where the pixels agree.
- **Motion (v1.11).** Each mover's own pixels are registered against every
  frame, as a rigid turn and translation, and re-keyframed. The result must
  pay for any keyframes it adds.
- **Sprites (v1.8).** A mover whose flat fill fails on its interior becomes a
  PNG of its own pixels that keeps its keyframes, if that's at least twice as
  good locally and pays for its bytes.

## The dial

`--quality 0–100` scales every lossy tolerance together: keyframe
simplification, outline simplification, opacity, JPEG quality, and the price
per byte that sprites and motion keyframes must pay. Each one moves only in
the direction the corpus measured as a real trade. `--target-psnr` searches
the dial for the smallest source that meets a fidelity target. The curves are
in `docs/rate-distortion.md`.

## Where to look

| | |
|---|---|
| format and renderer | `crates/vac-format`, `crates/vac-compiler/src/render.rs` (`Rig` for region renders) |
| detection, tracking, emission | `crates/vac-decompiler/src/{detect,track,layer}.rs` |
| camera | `crates/vac-decompiler/src/camera.rs` |
| the search and every pass | `crates/vac/src/lib.rs` |
| the ruler | `crates/vac-metrics` |
| what is still wrong | `docs/frontier.md` |
| tests | `crates/vac/tests/`: `hermetic` (every fixture against its floor), `frontier` (one per fixed problem), `corpus` (real clips against `benches/corpus-baseline.tsv`), `long_clips`, `rate_distortion` |
