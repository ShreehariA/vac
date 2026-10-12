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
| 0 | 11,208 | 7.7× smaller | 13.8 | 36.75 | 0.9833 | 2 | 41 |
| 25 | 10,588 | 8.2× smaller | 10.6 | 37.89 | 0.9844 | 2 | 28 |
| 50 | 11,774 | 7.4× smaller | 5.3 | 40.86 | 0.9883 | 2 | 35 |
| 75 | 13,375 | 6.5× smaller | 4.3 | 41.81 | 0.9900 | 2 | 53 |
| 100 | 16,310 | 5.3× smaller | 4.0 | 42.15 | 0.9927 | 2 | 65 |

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
| 0 | 27,275 | 10.9× smaller | 96.9 | 28.27 | 0.9051 | 8 | 51 |
| 25 | 33,942 | 8.8× smaller | 84.9 | 28.84 | 0.9054 | 12 | 90 |
| 50 | 56,339 | 5.3× smaller | 75.4 | 29.36 | 0.9190 | 48 | 321 |
| 75 | 67,221 | 4.4× smaller | 72.8 | 29.51 | 0.9229 | 48 | 350 |
| 100 | 99,283 | 3.0× smaller | 70.9 | 29.63 | 0.9320 | 48 | 368 |

### `real_pendulum.gif` — source file 247 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 37,586 | 6.7× smaller | 490.6 | 21.22 | 0.5266 | 23 | 178 |
| 25 | 51,815 | 4.9× smaller | 473.7 | 21.38 | 0.5580 | 38 | 383 |
| 50 | 58,290 | 4.3× smaller | 393.5 | 22.18 | 0.6256 | 41 | 427 |
| 75 | 62,655 | 4.0× smaller | 392.1 | 22.20 | 0.6309 | 41 | 442 |
| 100 | 70,666 | 3.6× smaller | 391.8 | 22.20 | 0.6325 | 41 | 445 |

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
| 0 | 28,848 | 10.8× smaller | 418.6 | 21.91 | 0.9950 | 44 | 277 |
| 25 | 49,794 | 6.3× smaller | 399.2 | 22.12 | 0.9951 | 64 | 654 |
| 50 | 94,205 | 3.3× smaller | 394.3 | 22.17 | 0.9951 | 110 | 1436 |
| 75 | 111,745 | 2.8× smaller | 387.2 | 22.25 | 0.9951 | 110 | 1804 |
| 100 | 130,837 | 2.4× smaller | 381.8 | 22.31 | 0.9952 | 110 | 2150 |

### `vac_control.mp4` — source file 18 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 25 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 50 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 75 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 100 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |

