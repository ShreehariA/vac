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
| 0 | 3,585 | 201.1× smaller | 4212.4 | 11.89 | 0.7593 | 11 | 42 |
| 25 | 7,826 | 92.1× smaller | 4132.6 | 11.97 | 0.7688 | 18 | 101 |
| 50 | 16,971 | 42.5× smaller | 4148.6 | 11.95 | 0.7658 | 33 | 228 |
| 75 | 19,142 | 37.7× smaller | 4150.5 | 11.95 | 0.7658 | 33 | 277 |
| 100 | 183,686 | 3.9× smaller | 3569.5 | 12.60 | 0.8053 | 33 | 358 |

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
| 100 | 3,841 | 3.8× smaller | 12.0 | 37.33 | 0.9889 | 4 | 72 |

### `real_bouncing_ball.gif` — source file 85 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 11,208 | 7.7× smaller | 13.8 | 36.75 | 0.9833 | 2 | 41 |
| 25 | 11,691 | 7.4× smaller | 3.9 | 42.21 | 0.9887 | 2 | 28 |
| 50 | 12,913 | 6.7× smaller | 2.6 | 44.05 | 0.9899 | 2 | 35 |
| 75 | 14,530 | 6.0× smaller | 1.4 | 46.81 | 0.9919 | 2 | 53 |
| 100 | 17,466 | 5.0× smaller | 0.9 | 48.63 | 0.9948 | 2 | 65 |

### `real_double_pendulum.gif` — source file 3374 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 29,400 | 117.5× smaller | 626.1 | 20.16 | 0.4284 | 106 | 399 |
| 25 | 60,575 | 57.0× smaller | 579.5 | 20.50 | 0.4701 | 154 | 950 |
| 50 | 169,914 | 20.3× smaller | 532.8 | 20.87 | 0.5052 | 419 | 2876 |
| 75 | 185,473 | 18.6× smaller | 532.1 | 20.87 | 0.5072 | 420 | 3221 |
| 100 | 197,345 | 17.5× smaller | 531.1 | 20.88 | 0.5077 | 420 | 3483 |

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
| 50 | 56,468 | 5.3× smaller | 74.8 | 29.39 | 0.9192 | 47 | 315 |
| 75 | 67,035 | 4.4× smaller | 72.2 | 29.55 | 0.9230 | 46 | 339 |
| 100 | 99,105 | 3.0× smaller | 70.2 | 29.67 | 0.9322 | 46 | 357 |

### `real_pendulum.gif` — source file 247 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 35,889 | 7.1× smaller | 437.8 | 21.72 | 0.5259 | 19 | 152 |
| 25 | 42,896 | 5.9× smaller | 406.6 | 22.04 | 0.5627 | 23 | 226 |
| 50 | 49,470 | 5.1× smaller | 357.5 | 22.60 | 0.6215 | 27 | 266 |
| 75 | 69,606 | 3.6× smaller | 333.4 | 22.90 | 0.6451 | 27 | 274 |
| 100 | 77,574 | 3.3× smaller | 333.2 | 22.90 | 0.6468 | 27 | 275 |

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
| 0 | 28,155 | 11.1× smaller | 417.5 | 21.92 | 0.9950 | 42 | 266 |
| 25 | 43,975 | 7.1× smaller | 398.5 | 22.13 | 0.9951 | 58 | 542 |
| 50 | 54,545 | 5.7× smaller | 387.9 | 22.24 | 0.9951 | 64 | 700 |
| 75 | 70,960 | 4.4× smaller | 380.8 | 22.32 | 0.9951 | 64 | 1046 |
| 100 | 89,797 | 3.5× smaller | 375.4 | 22.39 | 0.9952 | 64 | 1387 |

### `vac_control.mp4` — source file 18 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 25 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 50 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 75 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 100 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |

