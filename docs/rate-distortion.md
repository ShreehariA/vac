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
| 0 | 17,216 | 3.3× smaller | 69.7 | 29.70 | 0.9596 | 9 | 12 |
| 25 | 20,336 | 2.8× smaller | 69.3 | 29.73 | 0.9603 | 9 | 19 |
| 50 | 27,875 | 2.1× smaller | 78.8 | 29.17 | 0.9555 | 8 | 36 |
| 75 | 37,753 | 1.5× smaller | 75.7 | 29.34 | 0.9587 | 8 | 58 |
| 100 | 61,262 | 0.9× smaller | 75.7 | 29.34 | 0.9595 | 8 | 64 |

### `kenburns_watercolor.mp4` — source file 396 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 35,205 | 11.5× smaller | 135.6 | 26.81 | 0.9888 | 1 | 14 |
| 25 | 45,759 | 8.9× smaller | 77.7 | 29.23 | 0.9909 | 1 | 69 |
| 50 | 62,834 | 6.5× smaller | 60.9 | 30.29 | 0.9920 | 2 | 92 |
| 75 | 81,339 | 5.0× smaller | 54.6 | 30.76 | 0.9927 | 2 | 103 |
| 100 | 120,982 | 3.4× smaller | 49.9 | 31.15 | 0.9934 | 2 | 104 |

### `mandelbrot.mp4` — source file 704 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 20,388 | 35.4× smaller | 174.1 | 25.72 | 0.9860 | 1 | 5 |
| 25 | 24,781 | 29.1× smaller | 161.1 | 26.06 | 0.9885 | 1 | 9 |
| 50 | 34,353 | 21.0× smaller | 149.3 | 26.39 | 0.9896 | 2 | 19 |
| 75 | 45,929 | 15.7× smaller | 143.2 | 26.57 | 0.9912 | 2 | 30 |
| 100 | 72,973 | 9.9× smaller | 139.4 | 26.69 | 0.9915 | 2 | 62 |

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
| 0 | 17,835 | 4.4× smaller | 6.6 | 39.93 | 0.9601 | 1 | 2 |
| 25 | 21,995 | 3.5× smaller | 5.4 | 40.80 | 0.9636 | 1 | 22 |
| 50 | 31,598 | 2.5× smaller | 3.7 | 42.47 | 0.9697 | 1 | 60 |
| 75 | 41,774 | 1.9× smaller | 2.9 | 43.52 | 0.9737 | 1 | 66 |
| 100 | 67,778 | 1.1× smaller | 2.2 | 44.72 | 0.9774 | 1 | 74 |

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

### `slide_in_card.mp4` — source file 7 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 395 | 19.3× smaller | 2.2 | 44.63 | 0.9940 | 2 | 5 |
| 25 | 395 | 19.3× smaller | 2.2 | 44.63 | 0.9940 | 2 | 5 |
| 50 | 395 | 19.3× smaller | 2.2 | 44.63 | 0.9940 | 2 | 5 |
| 75 | 883 | 8.6× smaller | 1.9 | 45.35 | 0.9964 | 2 | 20 |
| 100 | 1,339 | 5.7× smaller | 1.5 | 46.46 | 0.9975 | 2 | 32 |

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

