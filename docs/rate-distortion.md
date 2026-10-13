# Rate–distortion: VAC as a codec

VAC v1.0 turns the decompiler into a **lossy codec whose compressed form is
human-readable, editable source code**, with a measured trade-off between
the two things a codec trades:

- **rate**: the size in bytes of the emitted `.vac` source;
- **distortion**: how far its re-rendered reconstruction is from the source
  video, measured by `vac verify` (per-channel residual, PSNR, ink IoU).

One dial, `--quality 0..100`, moves along that curve. 50 is the default and
reproduces v0.14's output byte for byte. `--target-psnr DB` runs rate
control instead: it finds the smallest source that still reconstructs at
that PSNR.

Every number below comes from `benches/rate-distortion.tsv`:

```text
cargo test --release -p vac --test rate_distortion -- --ignored --nocapture
```

## What the dial moves, and why

Each tolerance is scaled only in the direction it was **measured** to trade
bytes for fidelity across the corpus. One knob was swept at a time, and the
totals below cover all ten clips:

| knob | finer | coarser | in the dial |
|---|---|---|---|
| keyframe simplification (2.0 px) | 0.5 px: +16% bytes, −10% residual | 4.0 px: −11% bytes, +2.5% residual | **both directions**: this is where the bytes go |
| outline simplification (2.0 px) | 1.0 px: +27 KB *and* worse fidelity | 4.0 px: −14 KB, worse | coarser only, below 50 |
| Bézier fit (was 1.5 px) | 0.75 px: +47 bytes, better on every row it moved | 3.0 px: worse | **new default 0.75**; coarser only, below 50 |
| minimum blob area (9 px) | admits noise specks as fragment tracks | a cliff (see below) | **never scaled** |
| shape pruning | none | drop shapes worth < λ MSE/byte | below 50 only |

Why finer outlines are *worse*: an outline is traced from one
representative frame. On a non-rigid track, such as a pendulum bob that
merges with the force arrows drawn across it, more outline detail means
more of that one frame's accidents, painted into every other frame.

**The cliff, and the fix.** The first version of the dial raised the
minimum blob area at low quality (144 px at quality 0). Measured, it was a
cliff rather than a trade: it deleted every tick of the 32 px `real_loader`
spinner and its IoU went from 0.69 to 0.00. The replacement is a
rate–distortion decision per shape. Below quality 50, a shape is dropped
when the clip error it removes, per byte of source it costs, falls under a
threshold the dial sets. A loader tick is small but high-contrast, worth
about 2 MSE per byte. A Newton's-cradle fragment is worth about 0.0002. The
first survives every quality and the second does not. The test
`low_quality_never_deletes_small_high_contrast_shapes` pins this.

## Rate control

```text
$ vac decompile motion_graphics.mp4 --target-psnr 42
  probe quality   0.0       781 bytes   39.46 dB
  ...
  probe quality  84.4      3036 bytes   42.19 dB
  probe quality 100.0      4255 bytes   43.18 dB
  ✓ met at quality 84.4: 3036 bytes at 42.19 dB (target 42.00 dB)
```

The search probes the dial at five points, then bisects toward the cheapest
probe that meets the target. Among all probes that meet it, the one with
the fewest bytes wins. That is measured rather than assumed, because the
curve is not guaranteed to be monotone: the measured segmentation search
can pick a different candidate at a different quality (see
`real_bouncing_ball`, where quality 25 is both smaller and more faithful
than quality 0).

## The curves

"vs source" compares against the input video file's size. For flat-shaded
motion graphics the readable source is far smaller than the H.264 it came
from. For a test card full of noise it is barely smaller at the default
quality, and the dial shows a 7× cheaper point that costs half a decibel.

### `cuts_three_shots.mp4` — source file 56 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 16,896 | 3.4× smaller | 12.0 | 37.34 | 0.9861 | 7 | 10 |
| 25 | 20,356 | 2.8× smaller | 11.6 | 37.50 | 0.9870 | 7 | 17 |
| 50 | 28,495 | 2.0× smaller | 11.1 | 37.70 | 0.9880 | 7 | 38 |
| 75 | 39,258 | 1.5× smaller | 7.4 | 39.43 | 0.9922 | 7 | 73 |
| 100 | 64,157 | 0.9× smaller | 6.8 | 39.83 | 0.9934 | 7 | 93 |

### `fade_through_black.mp4` — source file 78 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 17,355 | 4.6× smaller | 5.4 | 40.80 | 0.9818 | 5 | 17 |
| 25 | 20,801 | 3.8× smaller | 4.6 | 41.50 | 0.9905 | 5 | 27 |
| 50 | 29,032 | 2.8× smaller | 3.9 | 42.22 | 0.9918 | 5 | 53 |
| 75 | 39,358 | 2.0× smaller | 3.7 | 42.48 | 0.9942 | 5 | 84 |
| 100 | 63,180 | 1.3× smaller | 3.4 | 42.81 | 0.9949 | 5 | 96 |

### `kenburns_watercolor.mp4` — source file 396 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 35,197 | 11.5× smaller | 135.6 | 26.81 | 0.9888 | 1 | 14 |
| 25 | 51,479 | 7.9× smaller | 56.8 | 30.59 | 0.9914 | 1 | 66 |
| 50 | 70,484 | 5.8× smaller | 37.5 | 32.39 | 0.9932 | 1 | 83 |
| 75 | 91,201 | 4.4× smaller | 31.2 | 33.19 | 0.9940 | 1 | 89 |
| 100 | 135,641 | 3.0× smaller | 27.2 | 33.79 | 0.9946 | 1 | 89 |

### `mandelbrot.mp4` — source file 704 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 20,394 | 35.4× smaller | 174.1 | 25.72 | 0.9859 | 1 | 5 |
| 25 | 24,780 | 29.1× smaller | 161.0 | 26.06 | 0.9886 | 1 | 9 |
| 50 | 34,448 | 20.9× smaller | 149.2 | 26.39 | 0.9898 | 2 | 20 |
| 75 | 45,504 | 15.8× smaller | 143.2 | 26.57 | 0.9912 | 1 | 24 |
| 100 | 72,242 | 10.0× smaller | 139.4 | 26.69 | 0.9916 | 1 | 50 |

### `motion_graphics.mp4` — source file 12 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 801 | 15.8× smaller | 4.3 | 41.76 | 0.9773 | 4 | 9 |
| 25 | 801 | 15.8× smaller | 4.3 | 41.76 | 0.9773 | 4 | 9 |
| 50 | 801 | 15.8× smaller | 4.3 | 41.76 | 0.9773 | 4 | 9 |
| 75 | 3,120 | 4.1× smaller | 3.7 | 42.42 | 0.9779 | 4 | 77 |
| 100 | 6,664 | 1.9× smaller | 2.3 | 44.48 | 0.9819 | 4 | 188 |

### `outlined_ui.mp4` — source file 14 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 688 | 21.0× smaller | 26.8 | 33.85 | 0.9847 | 4 | 4 |
| 25 | 688 | 21.0× smaller | 26.8 | 33.85 | 0.9847 | 4 | 4 |
| 50 | 688 | 21.0× smaller | 26.8 | 33.85 | 0.9847 | 4 | 4 |
| 75 | 2,682 | 5.4× smaller | 17.2 | 35.77 | 0.9886 | 4 | 48 |
| 100 | 4,037 | 3.6× smaller | 14.1 | 36.64 | 0.9914 | 4 | 80 |

### `pan_portrait.mp4` — source file 76 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 17,844 | 4.4× smaller | 6.6 | 39.93 | 0.9600 | 1 | 2 |
| 25 | 22,378 | 3.5× smaller | 5.3 | 40.87 | 0.9637 | 1 | 22 |
| 50 | 32,187 | 2.4× smaller | 3.6 | 42.54 | 0.9708 | 1 | 60 |
| 75 | 42,770 | 1.8× smaller | 2.8 | 43.62 | 0.9743 | 1 | 65 |
| 100 | 69,794 | 1.1× smaller | 2.1 | 44.82 | 0.9783 | 1 | 72 |

### `real_bouncing_ball.gif` — source file 85 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 10,804 | 8.0× smaller | 13.5 | 36.82 | 0.9832 | 2 | 51 |
| 25 | 11,714 | 7.4× smaller | 3.5 | 42.65 | 0.9889 | 2 | 27 |
| 50 | 12,997 | 6.7× smaller | 1.5 | 46.37 | 0.9905 | 2 | 37 |
| 75 | 14,539 | 6.0× smaller | 1.0 | 48.19 | 0.9922 | 2 | 53 |
| 100 | 17,874 | 4.8× smaller | 0.8 | 49.13 | 0.9949 | 2 | 76 |

### `real_double_pendulum.gif` — source file 3374 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 29,744 | 116.2× smaller | 544.8 | 20.77 | 0.5204 | 106 | 399 |
| 25 | 60,617 | 57.0× smaller | 505.1 | 21.10 | 0.5588 | 154 | 950 |
| 50 | 174,285 | 19.8× smaller | 406.3 | 22.04 | 0.6211 | 419 | 2905 |
| 75 | 197,204 | 17.5× smaller | 383.3 | 22.30 | 0.6349 | 420 | 3463 |
| 100 | 208,895 | 16.5× smaller | 380.8 | 22.32 | 0.6359 | 420 | 3857 |

### `real_loader.gif` — source file 3 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 2,874 | 1.1× smaller | 194.9 | 25.23 | 0.8384 | 16 | 21 |
| 25 | 3,158 | 1.0× smaller | 142.9 | 26.58 | 0.8406 | 16 | 26 |
| 50 | 3,523 | 0.9× smaller | 95.8 | 28.32 | 0.8641 | 16 | 33 |
| 75 | 3,937 | 0.8× smaller | 87.0 | 28.74 | 0.8584 | 16 | 40 |
| 100 | 3,937 | 0.8× smaller | 87.0 | 28.74 | 0.8584 | 16 | 40 |

### `real_newtons_cradle.gif` — source file 290 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 27,275 | 10.9× smaller | 88.3 | 28.67 | 0.9050 | 8 | 51 |
| 25 | 33,993 | 8.7× smaller | 78.7 | 29.17 | 0.9058 | 12 | 90 |
| 50 | 56,051 | 5.3× smaller | 65.8 | 29.95 | 0.9204 | 47 | 315 |
| 75 | 67,157 | 4.4× smaller | 63.1 | 30.13 | 0.9243 | 46 | 339 |
| 100 | 99,221 | 3.0× smaller | 60.9 | 30.28 | 0.9336 | 46 | 357 |

### `real_pendulum.gif` — source file 247 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 35,891 | 7.1× smaller | 419.6 | 21.90 | 0.5486 | 19 | 151 |
| 25 | 42,956 | 5.9× smaller | 391.9 | 22.20 | 0.5749 | 23 | 226 |
| 50 | 49,578 | 5.1× smaller | 331.5 | 22.93 | 0.6430 | 27 | 265 |
| 75 | 54,246 | 4.7× smaller | 329.2 | 22.96 | 0.6498 | 27 | 275 |
| 100 | 62,776 | 4.0× smaller | 329.3 | 22.96 | 0.6510 | 27 | 277 |

### `roll_watercolor.mp4` — source file 341 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 45,013 | 7.7× smaller | 43.5 | 31.75 | 0.9940 | 1 | 2 |
| 25 | 60,033 | 5.8× smaller | 23.4 | 34.43 | 0.9951 | 1 | 2 |
| 50 | 81,833 | 4.3× smaller | 16.9 | 35.85 | 0.9958 | 1 | 2 |
| 75 | 106,303 | 3.3× smaller | 13.5 | 36.83 | 0.9963 | 1 | 3 |
| 100 | 160,608 | 2.2× smaller | 11.2 | 37.66 | 0.9966 | 1 | 4 |

### `slide_in_card.mp4` — source file 7 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 395 | 19.3× smaller | 2.2 | 44.63 | 0.9940 | 2 | 5 |
| 25 | 395 | 19.3× smaller | 2.2 | 44.63 | 0.9940 | 2 | 5 |
| 50 | 395 | 19.3× smaller | 2.2 | 44.63 | 0.9940 | 2 | 5 |
| 75 | 883 | 8.6× smaller | 1.9 | 45.35 | 0.9964 | 2 | 20 |
| 100 | 1,339 | 5.7× smaller | 1.5 | 46.46 | 0.9975 | 2 | 32 |

### `slideshow_dissolve.mp4` — source file 435 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 96,915 | 4.6× smaller | 85.7 | 28.80 | 0.9924 | 4 | 8 |
| 25 | 130,307 | 3.4× smaller | 72.9 | 29.50 | 0.9934 | 4 | 12 |
| 50 | 174,569 | 2.6× smaller | 69.4 | 29.72 | 0.9962 | 4 | 71 |
| 75 | 231,150 | 1.9× smaller | 65.2 | 29.99 | 0.9967 | 4 | 103 |
| 100 | 359,970 | 1.2× smaller | 63.3 | 30.12 | 0.9972 | 4 | 113 |

### `testsrc2.mp4` — source file 305 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 28,382 | 11.0× smaller | 397.7 | 22.14 | 0.9951 | 42 | 265 |
| 25 | 44,391 | 7.0× smaller | 380.4 | 22.33 | 0.9951 | 58 | 541 |
| 50 | 54,981 | 5.7× smaller | 372.7 | 22.42 | 0.9951 | 64 | 705 |
| 75 | 72,216 | 4.3× smaller | 366.4 | 22.49 | 0.9952 | 64 | 1069 |
| 100 | 90,892 | 3.4× smaller | 361.7 | 22.55 | 0.9952 | 64 | 1399 |

### `vac_control.mp4` — source file 18 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 25 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 50 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 75 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 100 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |

