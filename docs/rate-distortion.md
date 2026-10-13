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
| 0 | 3,626 | 198.8× smaller | 4143.3 | 11.96 | 0.7642 | 11 | 42 |
| 25 | 7,897 | 91.3× smaller | 4074.0 | 12.03 | 0.7721 | 18 | 101 |
| 50 | 17,048 | 42.3× smaller | 4087.2 | 12.02 | 0.7695 | 33 | 228 |
| 75 | 19,225 | 37.5× smaller | 4088.7 | 12.01 | 0.7694 | 33 | 277 |
| 100 | 183,819 | 3.9× smaller | 3511.3 | 12.68 | 0.8090 | 33 | 358 |

### `motion_graphics.mp4` — source file 12 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 801 | 15.8× smaller | 4.3 | 41.76 | 0.9773 | 4 | 9 |
| 25 | 801 | 15.8× smaller | 4.3 | 41.76 | 0.9773 | 4 | 9 |
| 50 | 801 | 15.8× smaller | 4.3 | 41.76 | 0.9773 | 4 | 9 |
| 75 | 801 | 15.8× smaller | 4.3 | 41.76 | 0.9773 | 4 | 9 |
| 100 | 4,329 | 2.9× smaller | 2.8 | 43.60 | 0.9797 | 4 | 117 |

### `outlined_ui.mp4` — source file 14 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 688 | 21.0× smaller | 26.8 | 33.85 | 0.9847 | 4 | 4 |
| 25 | 688 | 21.0× smaller | 26.8 | 33.85 | 0.9847 | 4 | 4 |
| 50 | 688 | 21.0× smaller | 26.8 | 33.85 | 0.9847 | 4 | 4 |
| 75 | 1,379 | 10.5× smaller | 28.8 | 33.53 | 0.9842 | 4 | 23 |
| 100 | 3,905 | 3.7× smaller | 12.0 | 37.33 | 0.9889 | 4 | 72 |

### `real_bouncing_ball.gif` — source file 85 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 11,200 | 7.7× smaller | 13.7 | 36.77 | 0.9834 | 2 | 41 |
| 25 | 11,707 | 7.4× smaller | 3.9 | 42.21 | 0.9887 | 2 | 28 |
| 50 | 12,913 | 6.7× smaller | 2.4 | 44.36 | 0.9900 | 2 | 35 |
| 75 | 14,532 | 6.0× smaller | 1.3 | 47.07 | 0.9919 | 2 | 53 |
| 100 | 17,492 | 5.0× smaller | 0.8 | 48.81 | 0.9947 | 2 | 65 |

### `real_double_pendulum.gif` — source file 3374 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 29,805 | 115.9× smaller | 567.2 | 20.59 | 0.5103 | 106 | 399 |
| 25 | 60,806 | 56.8× smaller | 522.0 | 20.95 | 0.5520 | 154 | 950 |
| 50 | 172,803 | 20.0× smaller | 464.2 | 21.46 | 0.5943 | 419 | 2876 |
| 75 | 189,224 | 18.3× smaller | 459.2 | 21.51 | 0.5963 | 420 | 3221 |
| 100 | 201,107 | 17.2× smaller | 458.5 | 21.52 | 0.5966 | 420 | 3483 |

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
| 0 | 27,281 | 10.9× smaller | 89.2 | 28.62 | 0.9052 | 8 | 51 |
| 25 | 34,000 | 8.7× smaller | 78.7 | 29.17 | 0.9058 | 12 | 90 |
| 50 | 56,049 | 5.3× smaller | 65.8 | 29.95 | 0.9203 | 47 | 315 |
| 75 | 67,157 | 4.4× smaller | 63.1 | 30.13 | 0.9243 | 46 | 339 |
| 100 | 99,221 | 3.0× smaller | 61.0 | 30.28 | 0.9336 | 46 | 357 |

### `real_pendulum.gif` — source file 247 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 35,930 | 7.0× smaller | 418.8 | 21.91 | 0.5446 | 19 | 152 |
| 25 | 42,967 | 5.9× smaller | 394.3 | 22.17 | 0.5733 | 23 | 226 |
| 50 | 49,534 | 5.1× smaller | 334.8 | 22.88 | 0.6398 | 27 | 266 |
| 75 | 53,634 | 4.7× smaller | 333.3 | 22.90 | 0.6459 | 27 | 274 |
| 100 | 62,162 | 4.1× smaller | 333.1 | 22.90 | 0.6475 | 27 | 275 |

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
| 0 | 28,410 | 11.0× smaller | 398.4 | 22.13 | 0.9951 | 42 | 266 |
| 25 | 44,419 | 7.0× smaller | 381.3 | 22.32 | 0.9951 | 58 | 542 |
| 50 | 55,063 | 5.7× smaller | 374.0 | 22.40 | 0.9951 | 64 | 700 |
| 75 | 71,566 | 4.4× smaller | 368.3 | 22.47 | 0.9951 | 64 | 1046 |
| 100 | 90,456 | 3.4× smaller | 363.2 | 22.53 | 0.9952 | 64 | 1387 |

### `vac_control.mp4` — source file 18 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 25 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 50 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 75 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |
| 100 | 569 | 33.2× smaller | 1.4 | 46.58 | 0.9707 | 3 | 6 |

