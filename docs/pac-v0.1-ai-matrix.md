# PAC v0.1 — AI-generated image stress test

## TL;DR

We ran PAC's first cut on four CC-licensed AI-generated images from Wikimedia
Commons that span the structural→photographic spectrum. The same images were
also fed to **VAC's existing blob detector** (designed for "ball on solid
background" video) as a baseline. The comparison shows why PAC needed a
fundamentally different decomposition primitive, and what the v0.1 pipeline
buys us today.

| Image | Source | VAC blob detector | PAC v0.1 |
|---|---|---|---|
| `ai_dream_machine` (steampunk, garbled AI text) | Midjourney | Unrecognisable: 1 background blob + scattered noise | Subject clearly visible; light bar + smoke + frame readable |
| `ai_flat_illustration` (photoreal night ice rink) | Midjourney | One brown bg + cyan oval | Outdoor scene with sky, trees, illuminated pool, ground texture |
| `ai_watercolor` (Van Gogh-style portrait) | DALL-E watercolor | White bg + paint splatters; **portrait gone** | Recognisable face, hat, beard, eyes |
| `ai_photorealistic_portrait` (Mona Lisa 2024) | Stable Diffusion | Body silhouette + sky | Face + body + landscape + dress; clearly the subject |

Side-by-side renders live in `examples/wild/outputs/ai_*_pac_compare.png` —
three columns each: **original | VAC blob output | PAC polygon output**.

PAC isn't a magic AI-fixer. It cannot un-melt a face, recover the AI's "source
code" (the AI doesn't have one), or restore lost detail. What it provides is
a **structural handle** on top of the pixels, so editing a region (recolour,
reposition, replace) is a text edit on a few-vertex polygon instead of a
lasso-select-and-pray operation in Photoshop.

## Why a baseline against VAC

The natural first question is *"why not just point VAC's existing blob
detector at the still frame?"* — we tested that path explicitly and the
answer is in the table above. The detector was designed for solid-background
video and collapses every non-trivial input to ~1–3 ellipses around the
dominant background colour. PAC needed:

1. **Multi-modal background** — K-means over the pixel histogram replaces
   VAC's corner-sample bg estimator. No more single dominant colour.
2. **Per-cluster CCL** — connected components are tracked *within* each
   palette colour rather than against a single foreground mask, so regions
   of similar colour don't bleed into each other.
3. **Polygon output** — Moore-neighbour contour tracing + RDP simplification
   produces region outlines, not just rect/ellipse approximations.

Each of these is a 1980s-1990s computer vision algorithm. No models, no
training data, fully deterministic.

## Per-input results

### 1. `ai_dream_machine` — steampunk + AI-garbled text

Source: [Wikimedia Commons — A machine that reads dreams and translates them into images, by Midjourney](https://commons.wikimedia.org/wiki/File:A_machine_that_reads_dreams_and_translates_them_into_images_-_by_Midjourney.png)
{{PD-algorithm}} on Commons (no human authorship → US public domain).

| Metric | Value |
|---|---|
| Input size | 1196 KB (1024×1024 PNG, downscaled to 768×768 for processing) |
| `.pac` size | 64 KB (≈19× smaller) |
| Polygons | 89 |
| Decompile wall clock | ~0.5s |

Visible recovery: the rounded brass body, the central glowing light bar, the
smoke wisp on the left, the gear silhouettes, the wooden floor. Recovered
palette is muted gold / brass / navy / cream — matching the source mood.

Visible loss: the garbled AI text ("SNDARTIA DMIIIAMNST") is gone. That's
expected: text is a high-frequency feature each glyph would need to be its
own polygon, but at K=12 the text fades into the bronze background cluster.
Boosting `--k 24 --min-area 100` recovers crude letterforms — at the cost of
~6× more polygons.

### 2. `ai_flat_illustration` — night ice rink (misnamed but useful)

Source: [Wikimedia Commons — Midjourney_artificial_intelligence_art.png](https://commons.wikimedia.org/wiki/File:Midjourney_artificial_intelligence_art.png)

| Metric | Value |
|---|---|
| Input size | 1176 KB |
| `.pac` size | 32 KB (≈37× smaller) |
| Polygons | 57 |

This is a fully photorealistic night-time scene despite the filename. PAC
recovers the sky, the lit pool surface, the surrounding ground, the moon, and
several individual light blobs. The output is recognisable as the same
location and mood.

Visible loss: individual stars, fence detail, fine reflections — all merge
into the nearest palette colour. This is the standard "vector art of a
photo" trade-off, made honest.

### 3. `ai_watercolor` — Van Gogh-style portrait

Source: [Wikimedia Commons — Vincent van Gogh in watercolour.png](https://commons.wikimedia.org/wiki/File:Vincent_van_Gogh_in_watercolour.png)

| Metric | Value |
|---|---|
| Input size | 1416 KB |
| `.pac` size | 52 KB (≈27× smaller) |
| Polygons | 75 |

**This is the headline result.** VAC's blob detector returned a single grey
circle and a handful of paint splatter dots — the portrait was completely
gone. PAC produces a stylised but unmistakably recognisable face: the cap is
visible, the beard, the eyes, the collar, the watercolour cloud behind the
subject. You could edit any one of those regions individually.

The output looks like deliberate vector portrait art. That's the right
aesthetic for the input — a stylised AI portrait stays stylised, just now
with editable structure underneath.

### 4. `ai_photorealistic_portrait` — Mona Lisa 2024

Source: [Wikimedia Commons — Mona Lisa 2024.jpg](https://commons.wikimedia.org/wiki/File:Mona_Lisa_2024.jpg)

| Metric | Value |
|---|---|
| Input size | 772 KB |
| `.pac` size | 44 KB (≈18× smaller) |
| Polygons | 67 |

Even on hard photorealistic input, the subject is recognisable. Face shape,
hair, dress, hands, landscape behind — all readable. The aesthetic is
"posterised art-nouveau Mona Lisa" rather than a photographic copy, which is
the honest output for this kind of input.

For the user's "I want to edit AI slop" use case: the face is now a
polygon you can recolour, the hands are individual regions you can
reposition or delete, the dress is recolourable in one edit. Pixel-level
repainting still needs another tool, but the *structural* edits that used
to require lasso-and-paint are now text edits.

## How to reproduce

```bash
# 1. Fetch the four CC-licensed AI inputs (~4.6 MB total)
bash examples/wild/scripts/fetch_ai_images.sh

# 2. Decompile each with PAC v0.1 defaults
for img in examples/wild/inputs/ai_*.{png,jpg}; do
  [ -f "$img" ] || continue
  name=$(basename "$img" | sed 's/\.[^.]*$//')
  cargo run --release -p pac-cli --quiet -- \
    decompile "$img" "examples/wild/outputs/${name}.pac"
done

# 3. Round-trip render
for pac in examples/wild/outputs/ai_*.pac; do
  name=$(basename "$pac" .pac)
  cargo run --release -p pac-cli --quiet -- \
    compile "$pac" "examples/wild/outputs/${name}_pac_render.png"
done

# 4. Build the 3-up compare image (input | VAC | PAC)
for img in examples/wild/inputs/ai_*.{png,jpg}; do
  [ -f "$img" ] || continue
  name=$(basename "$img" | sed 's/\.[^.]*$//')
  vac_rt="examples/wild/outputs/${name}_roundtrip.mp4"
  pac_re="examples/wild/outputs/${name}_pac_render.png"
  cmp="examples/wild/outputs/${name}_pac_compare.png"
  ffmpeg -y -loglevel error -i "$img" -i "$vac_rt" -i "$pac_re" \
    -filter_complex "[0:v]scale=-1:400[a];[1:v]select=eq(n\,1),scale=-1:400[b];[2:v]scale=-1:400[c];[a][b][c]hstack=inputs=3[v]" \
    -map "[v]" -frames:v 1 -update 1 "$cmp"
done
```

The Wikimedia images carry `{{PD-algorithm}}` tags (AI-generated content
without sufficient human authorship for US copyright). They are read-only
test fixtures and are not redistributed by the repo (downloaded on demand
by the script above).

## v0.2 backlog (what this matrix surfaced)

- **Sub-region resolution at high K** — the dream machine's text only
  recovers with K ≥ 24 + a smaller `--min-area`. Need an `--auto-k` mode
  that grows K until residual energy per pixel drops below a threshold.
- **Smooth-curve primitive** — water/watercolour edges currently land as
  jagged polygons. Cubic Bézier fitting (Schneider's algorithm) on the RDP
  output would smooth them at minimal polygon-count cost.
- **Gradient fills** — many AI illustrations (sky, smoke, drop shadows) are
  smooth colour ramps that we currently approximate with several adjacent
  flat-colour polygons. A `gradient(linear, ...)` primitive in v0.4 collapses
  that cost.
- **Region naming heuristics** — `region_0042` is hard to edit. A v0.3
  improvement could derive semantic-ish names from bbox position
  (`bottom_left_blob`, `top_centre`) or dominant colour (`red_region`).
- **Layer ordering** — currently shapes are emitted strictly by area
  (largest first → bottom). Some inputs benefit from explicit z-order
  (foreground object on top regardless of area).
- **Raster escape hatch** — `raster(x, y, w, h, "tile.webp")` for regions
  whose residual energy proves they can't be decomposed honestly. Same
  primitive proposed for VAC v0.5+.
