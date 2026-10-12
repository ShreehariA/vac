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
| 0 | 5,151 | 140.0× smaller | 4226.6 | 11.87 | 0.7581 | 21 | 48 |
| 25 | 10,011 | 72.0× smaller | 4149.5 | 11.95 | 0.7673 | 28 | 108 |
| 50 | 19,417 | 37.1× smaller | 4166.1 | 11.93 | 0.7640 | 43 | 230 |
| 75 | 21,155 | 34.1× smaller | 4170.3 | 11.93 | 0.7637 | 43 | 267 |
| 100 | 22,978 | 31.4× smaller | 4168.9 | 11.93 | 0.7641 | 43 | 309 |

### `motion_graphics.mp4` — source file 12 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 781 | 16.2× smaller | 7.4 | 39.46 | 0.9762 | 4 | 9 |
| 25 | 781 | 16.2× smaller | 7.4 | 39.46 | 0.9762 | 4 | 9 |
| 50 | 781 | 16.2× smaller | 7.4 | 39.46 | 0.9762 | 4 | 9 |
| 75 | 781 | 16.2× smaller | 7.4 | 39.46 | 0.9762 | 4 | 9 |
| 100 | 4,255 | 3.0× smaller | 3.1 | 43.18 | 0.9802 | 4 | 117 |

### `real_bouncing_ball.gif` — source file 85 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 2,890 | 30.0× smaller | 425.1 | 21.85 | 0.8654 | 3 | 53 |
| 25 | 1,269 | 68.3× smaller | 425.3 | 21.84 | 0.8654 | 3 | 27 |
| 50 | 1,616 | 53.6× smaller | 419.9 | 21.90 | 0.8686 | 3 | 36 |
| 75 | 2,263 | 38.3× smaller | 419.6 | 21.90 | 0.8688 | 3 | 56 |
| 100 | 2,682 | 32.3× smaller | 419.6 | 21.90 | 0.8688 | 3 | 69 |

### `real_double_pendulum.gif` — source file 3374 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 42,768 | 80.8× smaller | 1105.3 | 17.70 | 0.3928 | 100 | 716 |
| 25 | 82,077 | 42.1× smaller | 1113.3 | 17.66 | 0.4230 | 146 | 1448 |
| 50 | 169,318 | 20.4× smaller | 1033.8 | 17.99 | 0.4630 | 378 | 3029 |
| 75 | 183,505 | 18.8× smaller | 1032.2 | 17.99 | 0.4637 | 378 | 3381 |
| 100 | 193,359 | 17.9× smaller | 1032.2 | 17.99 | 0.4637 | 378 | 3622 |

### `real_loader.gif` — source file 3 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 2,745 | 1.2× smaller | 348.5 | 22.71 | 0.6615 | 16 | 21 |
| 25 | 3,007 | 1.1× smaller | 279.2 | 23.67 | 0.6750 | 16 | 26 |
| 50 | 3,366 | 1.0× smaller | 228.3 | 24.55 | 0.6908 | 16 | 33 |
| 75 | 3,740 | 0.9× smaller | 218.2 | 24.74 | 0.6930 | 16 | 40 |
| 100 | 3,740 | 0.9× smaller | 218.2 | 24.74 | 0.6930 | 16 | 40 |

### `real_newtons_cradle.gif` — source file 290 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 11,103 | 26.8× smaller | 734.5 | 19.47 | 0.6149 | 81 | 34 |
| 25 | 24,432 | 12.2× smaller | 595.1 | 20.39 | 0.6345 | 137 | 129 |
| 50 | 51,139 | 5.8× smaller | 525.4 | 20.93 | 0.6419 | 202 | 466 |
| 75 | 52,695 | 5.6× smaller | 525.0 | 20.93 | 0.6419 | 202 | 502 |
| 100 | 53,731 | 5.5× smaller | 524.8 | 20.93 | 0.6419 | 202 | 526 |

### `real_pendulum.gif` — source file 247 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 15,629 | 16.2× smaller | 925.5 | 18.47 | 0.2945 | 33 | 252 |
| 25 | 24,291 | 10.4× smaller | 725.5 | 19.52 | 0.3722 | 45 | 393 |
| 50 | 27,411 | 9.2× smaller | 646.1 | 20.03 | 0.4309 | 48 | 436 |
| 75 | 27,964 | 9.1× smaller | 645.1 | 20.03 | 0.4313 | 48 | 451 |
| 100 | 27,997 | 9.0× smaller | 645.1 | 20.03 | 0.4313 | 48 | 452 |

### `slide_in_card.mp4` — source file 7 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 391 | 19.5× smaller | 3.0 | 43.42 | 0.9918 | 2 | 5 |
| 25 | 391 | 19.5× smaller | 3.0 | 43.42 | 0.9918 | 2 | 5 |
| 50 | 391 | 19.5× smaller | 3.0 | 43.42 | 0.9918 | 2 | 5 |
| 75 | 879 | 8.7× smaller | 2.9 | 43.49 | 0.9916 | 2 | 20 |
| 100 | 1,273 | 6.0× smaller | 2.5 | 44.12 | 0.9922 | 2 | 32 |

### `testsrc2.mp4` — source file 305 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 29,494 | 10.6× smaller | 1473.9 | 16.45 | 0.9869 | 96 | 469 |
| 25 | 162,702 | 1.9× smaller | 1369.0 | 16.77 | 0.9870 | 174 | 3227 |
| 50 | 216,127 | 1.4× smaller | 1349.7 | 16.83 | 0.9871 | 202 | 4374 |
| 75 | 242,133 | 1.3× smaller | 1341.2 | 16.86 | 0.9871 | 202 | 5058 |
| 100 | 261,545 | 1.2× smaller | 1340.3 | 16.86 | 0.9871 | 202 | 5593 |

### `vac_control.mp4` — source file 18 KB

| quality | `.vac` bytes | vs source | residual | PSNR dB | ink IoU | shapes | keyframes |
|---|---|---|---|---|---|---|---|
| 0 | 564 | 33.5× smaller | 9.1 | 38.54 | 0.9596 | 3 | 6 |
| 25 | 564 | 33.5× smaller | 9.1 | 38.54 | 0.9596 | 3 | 6 |
| 50 | 564 | 33.5× smaller | 9.1 | 38.54 | 0.9596 | 3 | 6 |
| 75 | 564 | 33.5× smaller | 9.1 | 38.54 | 0.9596 | 3 | 6 |
| 100 | 564 | 33.5× smaller | 9.1 | 38.54 | 0.9596 | 3 | 6 |

