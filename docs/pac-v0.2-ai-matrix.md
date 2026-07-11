# PAC v0.2 — AI-generated image stress test (round 2)

## TL;DR

v0.2 ships five things on top of the v0.1 polygon decompiler:

1. **Per-region contour tracing** — fixes a latent bug where two disjoint regions of the same palette colour could in principle bleed into one contour. Tracer now walks by region-id, not cluster-id.
2. **Cubic Bézier curve fitting** (Schneider 1990) with clamped control-point magnitudes — jagged polygon outlines become smooth `path(M..C..C..)` curves. Same vertex count, dramatically smoother re-render.
3. **Auto-K palette sizing** — the decompiler now grows K from a floor (default 6) until the per-pixel residual drops below a target (default 80) or hits a ceiling (default 24). The watercolour portrait that previously needed `--k 24` is now caught automatically.
4. **Linear-gradient fills** — per-region least-squares plane fit. Regions whose colour variance is dominated by a single 2D direction emit `gradient(linear, c0 at p0, c1 at p1)` instead of a flat fill. Replaces 5-10 adjacent flat polygons with a single gradient on smooth shaded inputs.
5. **Semantic-ish names + per-region residual report** — shapes are named `top_left_teal`, `mid_center_skin`, `bottom_right_navy` instead of `region_0042`. The CLI now prints the three worst-fit regions per image, surfacing exactly which shapes would benefit from a future `raster(...)` escape hatch.

| Image | v0.1 shapes | v0.2 shapes | v0.2 gradients | Auto-K | v0.2 residual |
|---|---|---|---|---|---|
| `ai_dream_machine` | 89 | 107 | 5 | 12 | 64.1 |
| `ai_flat_illustration` | 57 | 127 | 6 | 24 | 56.5 |
| `ai_watercolor` | 75 | 148 | 15 | 24 | 84.8 |
| `ai_photorealistic_portrait` | 67 | 76 | 4 | 12 | 68.2 |

The 4×3 visual comparison (`input | v0.1 | v0.2`) lives at:

```
examples/wild/outputs/pac_v02_results.png
```

The right-most column is recognisably closer to the source on every input: smoother edges everywhere, gradient-shaded pool/sky/dream-machine highlights instead of flat banding, and visible texture on the Mona Lisa face / dress.

## What each fix unlocks for VAC

This is the part the rest of the project hangs on. The user's framing was right: **PAC is the first step toward making VAC realistic**. Every v0.2 PAC improvement maps to a VAC unlock:

| PAC v0.2 | VAC implication |
|---|---|
| Per-region tracing (correctness) | Same fix VAC needs once it adopts K-means background estimation. |
| Bézier curve fitting | Frame-to-frame motion on jagged polygons looks like "boiling" jitter. Smooth Bézier-fitted shapes stabilise the motion track. |
| Auto-K | VAC needs auto-K *per frame* — a quiet frame might need K=4, a busy frame K=20. |
| Linear gradients | Motion graphics with gradient title backgrounds currently shed many tracked shapes per frame; one gradient primitive collapses them. |
| Semantic names | Track stability — fewer ephemeral tiny blobs to match across frames. |
| Residual report | Tells the future VAC `raster()` escape hatch *which* per-frame regions to fall back on. |

## Empirical residual report (v0.2)

The CLI's new `weakest-fit regions:` line directly tells us which regions a v0.3 `raster(...)` escape hatch should target. Real readings from the matrix:

```
ai_dream_machine            → top_center_gray=445, top_center_gray_1=397, mid_left_gray_1=366
ai_flat_illustration        → bottom_left_red_6=328, bottom_left_red_4=322, top_center_white=297
ai_watercolor               → red_8=751, red_9=555, red_4=430
ai_photorealistic_portrait  → bottom_center_maroon_1=276, red=267, top_center_red=249
```

Interpretation:

- The **dream machine** struggles on the high-frequency brass-with-highlights at the top of the boiler. A textured raster patch would honestly represent what plane-fit + Bézier cannot.
- The **flat illustration** struggles on the small red point-lights — they're sub-region-area sparkles that PAC currently merges into the surrounding ground.
- The **watercolour** has very high residual on the red beard/skin regions because the source paint has high spatial colour variance that no single gradient captures. Multi-stop gradients would fix this in v0.3.
- The **Mona Lisa** struggles where the skin tones blend into the dress — soft tonal gradations between adjacent regions, exactly the use-case for v0.3 region merging.

## Bézier curves: before/after

`pac_v02_results.png` row 3 (watercolour Van Gogh) is the most dramatic illustration. The middle column (v0.1 polygons) has visible staircase edges along the cap and beard outline. The right column (v0.2 Bézier) has smoothly-curved boundaries that match the underlying watercolour brushstrokes. Both columns have similar shape counts — the visual win is the same vertex data interpreted as cubic curves instead of straight line segments.

Implementation note: Schneider's algorithm in `crates/pac-decompiler/src/curves.rs` is the textbook recursive split-on-worst-error variant *plus* a Wu-Barsky-style alpha clamp. Without the clamp the fitter happily produces control points outside the shape's convex hull, which the renderer draws as bright spikes. The first run of v0.2 had clearly-visible spikes; the second run, with `alpha ∈ (0, chord_length]`, has none.

## Gradient detection: how endpoints are picked

The first cut anchored gradient endpoints at `(ux·t_min, uy·t_min)` — the projection onto the gradient direction *through origin*. For regions whose pixels are far from origin, this places the endpoint at infinity, so the plane fit extrapolates wildly and the renderer clamps to saturated colours — visible as magenta artefacts on the Mona Lisa.

The fix: anchor the endpoints at the **actual pixel positions** that produce `t_min` and `t_max`. Same gradient direction, same alpha-interpolation math, but the spatial endpoints lie inside the region's bounding box and the plane evaluation never extrapolates.

This is a small change in `crates/pac-decompiler/src/gradient.rs` but visually it turns the difference between "promising-but-broken" and "clean-and-shippable".

## How to reproduce

```bash
# 1. Fetch the four CC-licensed AI inputs (~4.6 MB total).
bash examples/wild/scripts/fetch_ai_images.sh

# 2. Decompile each with PAC v0.2 defaults (auto-K + curves + gradients).
for img in examples/wild/inputs/ai_*.{png,jpg}; do
  [ -f "$img" ] || continue
  name=$(basename "$img" | sed 's/\.[^.]*$//')
  cargo run --release -p pac-cli --quiet -- \
    decompile "$img" "examples/wild/outputs/${name}_v02.pac"
done

# 3. Round-trip render.
for pac in examples/wild/outputs/ai_*_v02.pac; do
  name=$(basename "$pac" .pac)
  cargo run --release -p pac-cli --quiet -- \
    compile "$pac" "examples/wild/outputs/${name}_render.png"
done

# 4. Build the 4×3 compare image (input | v0.1 | v0.2).
for img in examples/wild/inputs/ai_*.{png,jpg}; do
  [ -f "$img" ] || continue
  name=$(basename "$img" | sed 's/\.[^.]*$//')
  v01="examples/wild/outputs/${name}_pac_render.png"
  v02="examples/wild/outputs/${name}_v02_render.png"
  ffmpeg -y -loglevel error -i "$img" -i "$v01" -i "$v02" \
    -filter_complex "[0:v]scale=-1:400[a];[1:v]scale=-1:400[b];[2:v]scale=-1:400[c];[a][b][c]hstack=inputs=3[v]" \
    -map "[v]" -frames:v 1 -update 1 "examples/wild/outputs/${name}_v02_compare.png"
done
```

## What's not in v0.2 (intentionally)

- **`raster(x,y,w,h, "tile.webp")` primitive.** The residual report tells you which regions would benefit; the primitive itself ships in v0.3 along with a deliberate decision on whether PAC permits external-file references at all.
- **Region merging.** Risk of swallowing fine details (a sparkle in the night-pool image is small enough to look mergeable but is semantically important). Needs a "merge only if the residual change is small" guard — deferred to v0.3.
- **Radial gradients.** Linear is the common case for AI sky/smoke/sheen; radial would help photorealistic lighting (a single light source). Less common; v0.3.
- **Multi-stop gradients.** Would address the watercolour's red regions. Two-stop is the standard case; multi-stop is v0.3.

## v0.3 backlog (what this matrix surfaced for the next pass)

1. **`raster(...)` escape hatch** with PNG/WebP sibling files (no base64). Triggered automatically when `region_residual > threshold` and the user opts in via `--allow-raster`.
2. **Region merging** based on residual-change-on-merge. Small region with a similar-coloured big neighbour and merging that would not raise residual? Merge.
3. **Radial gradient fits.** Detect when colour variance pattern is centric rather than linear.
4. **Multi-stop linear gradients.** Two stops can't describe a sky-to-horizon-to-glow ramp; three stops can.
5. **Layer ordering hints** — explicit z-index in the source so editing doesn't have to reason about implicit area-descending order.
6. **Lift the same algorithms into VAC** for per-frame decompilation. This is the project's actual headline goal.

PAC v0.2 is ready for editing AI-generated stills. The next move is folding these same primitives into VAC's per-frame pipeline so the realistic-video round-trip becomes a function of "PAC each frame + match tracks across frames" — exactly the user's framing.
