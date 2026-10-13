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
| 0 | 16,977 | 3.4× smaller | 12.0 | 37.34 | 0.9861 | 7 | 10 |
| 25 | 20,437 | 2.8× smaller | 11.6 | 37.50 | 0.9870 | 7 | 17 |
| 50 | 28,576 | 2.0× smaller | 11.1 | 37.70 | 0.9880 | 7 | 38 |
| 75 | 39,339 | 1.5× smaller | 7.4 | 39.43 | 0.9922 | 7 | 73 |
| 100 | 64,238 | 0.9× smaller | 6.8 | 39.83 | 0.9934 | 7 | 93 |

### `fade_through_black.mp4` — source file 78 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 17,436 | 4.6× smaller | 5.3 | 40.84 | 0.9818 | 5 | 17 |
| 25 | 20,997 | 3.8× smaller | 4.9 | 41.26 | 0.9905 | 5 | 30 |
| 50 | 29,312 | 2.7× smaller | 3.9 | 42.18 | 0.9918 | 5 | 59 |
| 75 | 39,751 | 2.0× smaller | 3.7 | 42.49 | 0.9942 | 5 | 94 |
| 100 | 63,603 | 1.3× smaller | 3.2 | 43.03 | 0.9949 | 5 | 107 |

### `hue_cycle.mp4` — source file 21 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 540 | 39.1× smaller | 6.9 | 39.77 | 1.0000 | 2 | 11 |
| 25 | 595 | 35.5× smaller | 5.2 | 40.95 | 1.0000 | 2 | 13 |
| 50 | 791 | 26.7× smaller | 1.7 | 45.88 | 1.0000 | 2 | 20 |
| 75 | 985 | 21.4× smaller | 0.7 | 49.50 | 1.0000 | 2 | 27 |
| 100 | 1,370 | 15.4× smaller | 0.4 | 52.53 | 1.0000 | 2 | 41 |

### `kenburns_watercolor.mp4` — source file 396 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 35,278 | 11.5× smaller | 135.6 | 26.81 | 0.9888 | 1 | 14 |
| 25 | 51,560 | 7.9× smaller | 56.8 | 30.59 | 0.9914 | 1 | 66 |
| 50 | 70,565 | 5.7× smaller | 37.5 | 32.39 | 0.9932 | 1 | 83 |
| 75 | 91,282 | 4.4× smaller | 31.2 | 33.19 | 0.9940 | 1 | 89 |
| 100 | 135,722 | 3.0× smaller | 27.2 | 33.79 | 0.9946 | 1 | 89 |

### `mandelbrot.mp4` — source file 704 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 20,475 | 35.2× smaller | 174.1 | 25.72 | 0.9859 | 1 | 5 |
| 25 | 24,861 | 29.0× smaller | 161.0 | 26.06 | 0.9886 | 1 | 9 |
| 50 | 34,529 | 20.9× smaller | 149.2 | 26.39 | 0.9898 | 2 | 20 |
| 75 | 45,585 | 15.8× smaller | 143.2 | 26.57 | 0.9912 | 1 | 24 |
| 100 | 72,323 | 10.0× smaller | 139.4 | 26.69 | 0.9916 | 1 | 50 |

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
| 0 | 17,925 | 4.3× smaller | 6.6 | 39.93 | 0.9600 | 1 | 2 |
| 25 | 22,459 | 3.5× smaller | 5.3 | 40.87 | 0.9637 | 1 | 22 |
| 50 | 32,268 | 2.4× smaller | 3.6 | 42.54 | 0.9708 | 1 | 60 |
| 75 | 42,851 | 1.8× smaller | 2.8 | 43.62 | 0.9743 | 1 | 65 |
| 100 | 69,875 | 1.1× smaller | 2.1 | 44.82 | 0.9783 | 1 | 72 |

### `progress_bars.mp4` — source file 8 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 1,660 | 5.1× smaller | 18.4 | 35.48 | 0.9862 | 6 | 22 |
| 25 | 1,660 | 5.1× smaller | 18.4 | 35.48 | 0.9862 | 6 | 22 |
| 50 | 1,872 | 4.5× smaller | 6.7 | 39.90 | 0.9947 | 6 | 26 |
| 75 | 2,214 | 3.8× smaller | 3.8 | 42.36 | 0.9953 | 6 | 33 |
| 100 | 3,868 | 2.2× smaller | 4.4 | 41.72 | 0.9965 | 6 | 66 |

### `real_bouncing_ball.gif` — source file 85 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 10,885 | 8.0× smaller | 13.5 | 36.82 | 0.9832 | 2 | 51 |
| 25 | 11,823 | 7.3× smaller | 3.5 | 42.65 | 0.9889 | 2 | 27 |
| 50 | 13,106 | 6.6× smaller | 1.5 | 46.37 | 0.9905 | 2 | 37 |
| 75 | 14,648 | 5.9× smaller | 1.0 | 48.19 | 0.9922 | 2 | 53 |
| 100 | 17,983 | 4.8× smaller | 0.8 | 49.13 | 0.9949 | 2 | 76 |

### `real_double_pendulum.gif` — source file 3374 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 26,901 | 128.4× smaller | 554.6 | 20.69 | 0.5132 | 106 | 340 |
| 25 | 60,893 | 56.7× smaller | 503.8 | 21.11 | 0.5594 | 154 | 956 |
| 50 | 184,870 | 18.7× smaller | 407.4 | 22.03 | 0.6233 | 421 | 3038 |
| 75 | 209,213 | 16.5× smaller | 379.9 | 22.33 | 0.6377 | 421 | 3601 |
| 100 | 220,212 | 15.7× smaller | 379.3 | 22.34 | 0.6377 | 422 | 3870 |

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
| 0 | 27,347 | 10.9× smaller | 87.5 | 28.71 | 0.9055 | 8 | 50 |
| 25 | 34,091 | 8.7× smaller | 77.7 | 29.23 | 0.9064 | 12 | 89 |
| 50 | 56,235 | 5.3× smaller | 64.5 | 30.03 | 0.9210 | 46 | 308 |
| 75 | 66,672 | 4.5× smaller | 62.0 | 30.21 | 0.9250 | 45 | 327 |
| 100 | 99,467 | 3.0× smaller | 59.8 | 30.36 | 0.9341 | 45 | 349 |

### `real_pendulum.gif` — source file 247 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 36,169 | 7.0× smaller | 418.1 | 21.92 | 0.5448 | 19 | 151 |
| 25 | 43,037 | 5.9× smaller | 391.9 | 22.20 | 0.5749 | 23 | 226 |
| 50 | 49,659 | 5.1× smaller | 331.5 | 22.93 | 0.6430 | 27 | 265 |
| 75 | 54,327 | 4.7× smaller | 329.2 | 22.96 | 0.6498 | 27 | 275 |
| 100 | 62,885 | 4.0× smaller | 329.3 | 22.96 | 0.6510 | 27 | 277 |

### `roll_watercolor.mp4` — source file 341 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 45,094 | 7.7× smaller | 43.5 | 31.75 | 0.9940 | 1 | 2 |
| 25 | 60,114 | 5.8× smaller | 23.4 | 34.43 | 0.9951 | 1 | 2 |
| 50 | 81,914 | 4.3× smaller | 16.9 | 35.85 | 0.9958 | 1 | 2 |
| 75 | 106,384 | 3.3× smaller | 13.5 | 36.83 | 0.9963 | 1 | 3 |
| 100 | 160,689 | 2.2× smaller | 11.2 | 37.66 | 0.9966 | 1 | 4 |

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
| 0 | 49,016 | 9.1× smaller | 85.7 | 28.80 | 0.9924 | 4 | 8 |
| 25 | 65,816 | 6.8× smaller | 72.9 | 29.50 | 0.9934 | 4 | 12 |
| 50 | 93,246 | 4.8× smaller | 71.5 | 29.59 | 0.9945 | 4 | 73 |
| 75 | 231,315 | 1.9× smaller | 65.2 | 29.99 | 0.9967 | 4 | 103 |
| 100 | 360,135 | 1.2× smaller | 63.3 | 30.12 | 0.9972 | 4 | 113 |

### `slideshow_wipe_push.mp4` — source file 173 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 76,390 | 2.3× smaller | 29.6 | 33.41 | 0.9967 | 7 | 30 |
| 25 | 93,769 | 1.9× smaller | 20.2 | 35.07 | 0.9970 | 7 | 63 |
| 50 | 131,758 | 1.3× smaller | 12.3 | 37.23 | 0.9976 | 7 | 69 |
| 75 | 175,365 | 1.0× smaller | 7.5 | 39.38 | 0.9983 | 7 | 78 |
| 100 | 278,293 | 0.6× smaller | 3.1 | 43.24 | 0.9989 | 7 | 78 |

### `testsrc2.mp4` — source file 305 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 29,709 | 10.5× smaller | 384.6 | 22.28 | 0.9953 | 43 | 278 |
| 25 | 44,992 | 6.9× smaller | 365.9 | 22.50 | 0.9954 | 58 | 549 |
| 50 | 56,033 | 5.6× smaller | 357.2 | 22.60 | 0.9954 | 63 | 713 |
| 75 | 75,484 | 4.1× smaller | 352.0 | 22.67 | 0.9954 | 65 | 1118 |
| 100 | 93,281 | 3.3× smaller | 346.5 | 22.73 | 0.9954 | 64 | 1412 |

### `vac_control.mp4` — source file 18 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 25 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 50 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 75 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 100 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |

