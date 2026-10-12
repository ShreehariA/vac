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

### `mandelbrot.mp4` — source file 704 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 5,746 | 125.5× smaller | 4226.2 | 11.87 | 0.7583 | 21 | 54 |
| 25 | 10,940 | 65.9× smaller | 4146.5 | 11.95 | 0.7671 | 28 | 118 |
| 50 | 21,100 | 34.2× smaller | 4163.2 | 11.94 | 0.7639 | 43 | 247 |
| 75 | 23,466 | 30.7× smaller | 4165.1 | 11.93 | 0.7639 | 43 | 300 |
| 100 | 26,680 | 27.0× smaller | 4163.9 | 11.94 | 0.7641 | 43 | 384 |

### `motion_graphics.mp4` — source file 12 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 781 | 16.2× smaller | 7.4 | 39.46 | 0.9762 | 4 | 9 |
| 25 | 781 | 16.2× smaller | 7.4 | 39.46 | 0.9762 | 4 | 9 |
| 50 | 781 | 16.2× smaller | 7.4 | 39.46 | 0.9762 | 4 | 9 |
| 75 | 781 | 16.2× smaller | 7.4 | 39.46 | 0.9762 | 4 | 9 |
| 100 | 4,255 | 3.0× smaller | 3.1 | 43.18 | 0.9802 | 4 | 117 |

### `outlined_ui.mp4` — source file 14 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 688 | 21.0× smaller | 28.8 | 33.54 | 0.9839 | 4 | 4 |
| 25 | 688 | 21.0× smaller | 28.8 | 33.54 | 0.9839 | 4 | 4 |
| 50 | 688 | 21.0× smaller | 28.8 | 33.54 | 0.9839 | 4 | 4 |
| 75 | 1,379 | 10.5× smaller | 32.2 | 33.06 | 0.9840 | 4 | 23 |
| 100 | 2,969 | 4.9× smaller | 24.3 | 34.28 | 0.9882 | 4 | 72 |

### `real_bouncing_ball.gif` — source file 85 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 3,132 | 27.7× smaller | 171.6 | 25.78 | 0.8941 | 3 | 53 |
| 25 | 1,466 | 59.1× smaller | 169.4 | 25.84 | 0.8945 | 3 | 27 |
| 50 | 1,845 | 47.0× smaller | 164.3 | 25.97 | 0.8978 | 3 | 36 |
| 75 | 2,566 | 33.8× smaller | 163.8 | 25.99 | 0.8980 | 3 | 56 |
| 100 | 3,037 | 28.5× smaller | 163.7 | 25.99 | 0.8981 | 3 | 69 |

### `real_double_pendulum.gif` — source file 3374 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 40,671 | 85.0× smaller | 750.3 | 19.38 | 0.4283 | 131 | 550 |
| 25 | 75,753 | 45.6× smaller | 737.3 | 19.45 | 0.4633 | 183 | 1164 |
| 50 | 192,240 | 18.0× smaller | 697.2 | 19.70 | 0.4918 | 454 | 3261 |
| 75 | 208,683 | 16.6× smaller | 694.8 | 19.71 | 0.4928 | 454 | 3626 |
| 100 | 221,796 | 15.6× smaller | 693.8 | 19.72 | 0.4933 | 454 | 3913 |

### `real_loader.gif` — source file 3 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 2,838 | 1.1× smaller | 309.8 | 23.22 | 0.6950 | 16 | 21 |
| 25 | 3,118 | 1.0× smaller | 233.2 | 24.45 | 0.7129 | 16 | 26 |
| 50 | 3,503 | 0.9× smaller | 173.2 | 25.75 | 0.7318 | 16 | 33 |
| 75 | 3,905 | 0.8× smaller | 160.8 | 26.07 | 0.7386 | 16 | 40 |
| 100 | 3,905 | 0.8× smaller | 160.8 | 26.07 | 0.7386 | 16 | 40 |

### `real_newtons_cradle.gif` — source file 290 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 10,544 | 28.2× smaller | 652.8 | 19.98 | 0.6104 | 64 | 44 |
| 25 | 29,493 | 10.1× smaller | 562.1 | 20.63 | 0.6281 | 148 | 183 |
| 50 | 59,677 | 5.0× smaller | 455.3 | 21.55 | 0.6400 | 232 | 515 |
| 75 | 61,326 | 4.8× smaller | 455.2 | 21.55 | 0.6401 | 232 | 549 |
| 100 | 62,301 | 4.8× smaller | 455.2 | 21.55 | 0.6401 | 232 | 569 |

### `real_pendulum.gif` — source file 247 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 12,262 | 20.7× smaller | 775.6 | 19.23 | 0.3308 | 32 | 174 |
| 25 | 25,937 | 9.8× smaller | 718.6 | 19.57 | 0.3748 | 45 | 393 |
| 50 | 29,152 | 8.7× smaller | 634.6 | 20.11 | 0.4353 | 48 | 436 |
| 75 | 29,772 | 8.5× smaller | 633.6 | 20.11 | 0.4356 | 48 | 451 |
| 100 | 29,809 | 8.5× smaller | 633.5 | 20.11 | 0.4356 | 48 | 452 |

### `slide_in_card.mp4` — source file 7 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 395 | 19.3× smaller | 2.2 | 44.63 | 0.9940 | 2 | 5 |
| 25 | 395 | 19.3× smaller | 2.2 | 44.63 | 0.9940 | 2 | 5 |
| 50 | 395 | 19.3× smaller | 2.2 | 44.63 | 0.9940 | 2 | 5 |
| 75 | 883 | 8.6× smaller | 1.9 | 45.35 | 0.9964 | 2 | 20 |
| 100 | 1,277 | 6.0× smaller | 1.5 | 46.36 | 0.9972 | 2 | 32 |

### `testsrc2.mp4` — source file 305 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 30,976 | 10.1× smaller | 1338.4 | 16.86 | 0.9855 | 95 | 459 |
| 25 | 167,591 | 1.9× smaller | 1243.8 | 17.18 | 0.9856 | 160 | 2975 |
| 50 | 255,463 | 1.2× smaller | 1214.8 | 17.29 | 0.9857 | 202 | 4503 |
| 75 | 283,367 | 1.1× smaller | 1207.4 | 17.31 | 0.9857 | 202 | 5144 |
| 100 | 303,881 | 1.0× smaller | 1204.9 | 17.32 | 0.9858 | 202 | 5659 |

### `vac_control.mp4` — source file 18 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 25 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 50 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 75 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 100 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |

