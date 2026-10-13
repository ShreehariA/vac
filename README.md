# VAC — Video As Code

**A purely algorithmic decompiler that reverse-engineers video into human-readable, structured code.**

> What Potrace did for images (bitmap → SVG), VAC does for video (pixels → structured code).

---

## Where VAC is today (v1.23)

VAC decompiles a video into `.vac` source: shapes, fills, strokes and
keyframed motion. It compiles that source back to video and measures, in
pixels, how much of the original survived. Everything below runs today.
Every number comes from `vac verify` or `benches/`.

```sh
cargo build --release
vac compile   examples/color_shift.vac  out.mp4   # source → video
vac decompile out.mp4  recovered.vac              # video → source
vac verify    out.mp4                             # how much survived, in numbers
vac verify    out.mp4  --dump frames/             # source | reconstruction | ink map PNGs
vac verify    out.mp4  --vac edited.vac           # score any .vac, e.g. one you edited
vac decompile out.mp4  small.vac --quality 0      # the rate–distortion dial, 0–100
vac decompile out.mp4  good.vac  --target-psnr 40 # smallest source that reaches 40 dB
```

This is what VAC writes for `examples/color_shift.vac` after a round trip
through video. A ball moving while it changes colour, and a square pulsing
between two colours:

```vac
let shape_1 = ellipse(80, 100, 30, 30)
shape_1.fill = #446099

animate shape_1 {
  0ms    -> position(80, 100), fill(#e04144)
  1500ms -> position(240, 100), fill(#4060dd)
  2966ms -> position(396.3, 100), fill(#40be66)
  easing: linear
}
```

**What it recovers:** shots, each as its own scene; fades to and from
black or white; dissolves, wipes and pushes between shots; rects, ellipses and traced paths, by measured fit;
position, scale (uniform, or stretching along one axis), rotation, opacity
and colour over time; linear and radial
gradient fills; strokes with their width; objects entering, leaving,
appearing and disappearing. Where vectors measurably fail, the static
scenery becomes one embedded `image()` with the moving shapes still written
as code on top. Each picture is written once, as a named asset after the
scenes, and the scenes name it. When the whole picture moves, that image is moved by a
measured camera that zooms, pans and rolls, written as `position()`,
`scale()` and `rotate()` keyframes; the image is fitted to the frames
through the renderer's own reading of it, so what is drawn back is as sharp
as what was filmed. A mover no
flat fill can describe, like a shaded ball, keeps its keyframes and is drawn
as its own pixels.

**How it decides:** by measurement, not classification. Every ambiguous
choice is tried several ways: segmentation, background rule, static scenery
or none, raster versus vector plate. Each candidate is re-rendered and scored against the source,
and the one that reconstructs best wins. Then every shape in the winner is
measured against its own absence, and a shape the reconstruction is no
better with is taken out. Each remaining shape's size, position and stroke
width are nudged to wherever the pixels agree best, and every mover's
keyframes are re-measured by registering its own pixels across frames.

**How well**, against the corpus in `examples/wild/` (default quality):

| clip | what it is | PSNR | ink IoU |
|---|---|---|---|
| `vac_control.mp4` | VAC's own output, two moving balls | 46.6 dB | 0.971 |
| `real_bouncing_ball.gif` | Wikimedia, shaded sphere and ball | 46.4 dB | 0.991 |
| `slide_in_card.mp4` | ffmpeg card sliding in and out | 44.6 dB | 0.994 |
| `pan_portrait.mp4` | slow camera pan over an AI-generated portrait | 42.5 dB | 0.971 |
| `fade_through_black.mp4` | a card fading out, then a photo pan fading in | 42.2 dB | 0.992 |
| `motion_graphics.mp4` | ffmpeg rectangles ping-ponging | 41.8 dB | 0.977 |
| `progress_bars.mp4` | ffmpeg progress bar and a bar chart rising | 39.9 dB | 0.995 |
| `cuts_three_shots.mp4` | three corpus clips cut together | 37.7 dB | 0.988 |
| `slideshow_wipe_push.mp4` | three photos: a wipe, then a push | 37.2 dB | 0.998 |
| `roll_watercolor.mp4` | a camera rolling 26° over an AI-generated watercolour | 35.9 dB | 0.996 |
| `outlined_ui.mp4` | ffmpeg outlines and a bordered card | 33.9 dB | 0.985 |
| `kenburns_watercolor.mp4` | Ken Burns zoom into an AI-generated watercolour | 32.4 dB | 0.993 |
| `real_newtons_cradle.gif` | Wikimedia photograph | 30.0 dB | 0.921 |
| `slideshow_dissolve.mp4` | two photos, a Ken Burns zoom dissolving into a pan | 29.6 dB | 0.995 |
| `real_loader.gif` | Wikimedia 32 px spinner | 28.3 dB | 0.864 |
| `mandelbrot.mp4` | ffmpeg fractal zoom | 26.4 dB | 0.990 |
| `real_pendulum.gif` | Wikimedia diagram | 22.9 dB | 0.643 |
| `testsrc2.mp4` | ffmpeg test card with noise | 22.6 dB | 0.995 |
| `real_double_pendulum.gif` | Wikimedia diagram with trails | 22.1 dB | 0.624 |

What it still gets wrong is listed, measured, in `docs/frontier.md`. The
rate–distortion curves are in `docs/rate-distortion.md`. The sections after
this one are the project's original vision and its version-by-version log.
The example in "The MVP" below was the target, written before the code.
The output above is what VAC actually produces.

---

## The Insight

Code-to-video already exists. Remotion, Manim, Motion Canvas — they all let you write code that outputs video. That space is crowded.

**Nobody has built the reverse: video → code.**

No tool takes an MP4 and produces a human-readable, editable, recompilable source file describing what's *in* the video — geometrically, structurally, algorithmically. Without AI. Without ML. Without classification.

That's VAC.

---

## The MVP: Video → Code (Decompiler)

The MVP is the **decompiler** — a CLI tool that takes a video file and outputs `.vac` source code describing its contents as geometric primitives, colors, motion curves, and scene structure.

```
$ vac decompile input.mp4 -o output.vac

Extracting frames...        ████████████████ 240 frames
Detecting scenes...         Found 3 scenes (histogram analysis)
Tracing contours...         ████████████████ 847 shapes
Fitting curves...           ████████████████ 847 shapes → 312 primitives
Estimating motion...        ████████████████ optical flow complete
Generating code...          Done.

Written to output.vac (12.4 KB)
```

The output is human-readable `.vac` code:

```
canvas 1920x1080 @30fps

scene intro {
  duration 3s

  let bg = rect(0, 0, 1920, 1080)
  bg.fill = #1a1a2e

  let circle = ellipse(960, 540, 200, 200)
  circle.fill = #e94560
  circle.stroke = none

  animate circle {
    0ms   -> position(960, 540), scale(1.0)
    1500ms -> position(960, 300), scale(1.4)
    3000ms -> position(960, 540), scale(1.0)
    easing: ease-in-out
  }
}

scene main {
  duration 5s

  let title = text("Hello World", 960, 500)
  title.font = "Inter"
  title.size = 72
  title.fill = #ffffff

  animate title.opacity {
    0ms   -> 0.0
    800ms -> 1.0
    easing: ease-out
  }
}
```

**This is what gets generated from the video.** Not written by hand — extracted algorithmically.

---

## Why No AI / No ML

```
Measurement is deterministic.  →  "This contour has 4 vertices at these coordinates"
Classification is probabilistic. →  "I think this is probably a button (87% confidence)"
```

AI classifications can lie. They're probabilistic, inconsistent across runs, and lose geometric precision. If you classify an object as "a circle," you've lost the *exact* contour, the *exact* color gradients, the *exact* motion path.

VAC measures. It doesn't classify. The decompiler uses:

| Technique | What It Does | Library/Algorithm |
|-----------|-------------|-------------------|
| Histogram analysis | Detect scene cuts | Frame-to-frame color histogram comparison |
| Canny / Sobel | Find edges | Gradient-based edge detection |
| Suzuki contour tracing | Extract boundaries | Border following algorithm |
| Ramer-Douglas-Peucker | Simplify contours | Polygon approximation |
| Least-squares Bezier fitting | Smooth curves | Fit Bezier curves to point sequences |
| Color quantization | Extract palette | Median cut / k-means on pixel colors |
| Connected component analysis | Group regions | Flood-fill based region detection |
| Lucas-Kanade / Farnebäck | Track motion | Optical flow between frames |
| FFT analysis | Detect periodicity | Find repeating animation patterns |

Every technique is deterministic. Same input → same output. Every time.

---

## The Format Spectrum

```
Raw Pixels ◄──────────────── VAC ────────────────► AI Classification

MP4/H.264                   .vac                    "a red circle moves up"
├─ Lossy compressed          ├─ Geometric primitives   ├─ Probabilistic
├─ Not editable              ├─ Animation curves        ├─ Loses precision  
├─ Not readable              ├─ Human-readable          ├─ Inconsistent
└─ Frame-by-frame            ├─ Editable & recompilable └─ Version-dependent
                             └─ Deterministic
```

VAC sits **between** raw pixels and high-level AI descriptions. It's a structured, geometric representation — like what SVG is for images, but for video.

---

## Decompiler Pipeline (MVP Architecture)

```
┌─────────────┐
│  Input MP4  │
└──────┬──────┘
       ▼
┌──────────────────┐
│ Frame Extraction  │  ffmpeg-next: decode video → raw RGBA frames
└──────┬───────────┘
       ▼
┌──────────────────┐
│ Scene Detection   │  Compare frame histograms → find cut points
└──────┬───────────┘  (frames with >threshold histogram difference = new scene)
       ▼
┌──────────────────┐
│ Per-Scene Analysis│
│                  │
│  ┌─────────────┐ │
│  │Edge Detection│ │  Canny/Sobel → edge maps
│  └──────┬──────┘ │
│         ▼        │
│  ┌─────────────┐ │
│  │Contour Trace│ │  Suzuki → closed contour paths
│  └──────┬──────┘ │
│         ▼        │
│  ┌─────────────┐ │
│  │ Curve Fit   │ │  RDP simplification → Bezier fitting
│  └──────┬──────┘ │
│         ▼        │
│  ┌─────────────┐ │
│  │Color Extract│ │  Quantization → fill/stroke colors
│  └──────┬──────┘ │
│         ▼        │
│  ┌─────────────┐ │
│  │ Motion Est. │ │  Optical flow → animation curves
│  └─────────────┘ │
└──────┬───────────┘
       ▼
┌──────────────────┐
│ Shape Matching    │  Track shapes across frames (temporal coherence)
└──────┬───────────┘  Match contour in frame N to contour in frame N+1
       ▼
┌──────────────────┐
│ Animation Fitting │  Convert motion vectors → easing curves
└──────┬───────────┘  Fit keyframes (detect linear/ease/bezier motion)
       ▼
┌──────────────────┐
│ Code Generation   │  Emit .vac source code
└──────┬───────────┘
       ▼
┌─────────────┐
│  output.vac │
└─────────────┘
```

---

## Build Plan (MVP-First)

### Phase 1: Static Frame Decompiler ← START HERE
**Input:** A single image (PNG/JPG)  
**Output:** `.vac` code describing the shapes in that image

This is "Potrace but outputs VAC code instead of SVG."

**Steps:**
1. Set up Rust project with `clap` CLI
2. Load image with `image` crate
3. Edge detection (implement Canny or use `imageproc`)
4. Contour tracing (implement Suzuki-85 algorithm)
5. Polygon simplification (implement Ramer-Douglas-Peucker)
6. Bezier curve fitting (least-squares fit)
7. Color extraction (quantize colors inside each contour)
8. Code generation (output `.vac` text format)

**Success metric:** Feed in a simple graphic (shapes on a background), get readable `.vac` code that describes those shapes.

### Phase 2: Scene-Aware Video Decompiler
**Input:** A video file (MP4)  
**Output:** `.vac` code with scenes and static shapes per scene

**Steps:**
1. Frame extraction via `ffmpeg-next`
2. Scene detection (histogram comparison between consecutive frames)
3. Run Phase 1's image decompiler on keyframes from each scene
4. Generate multi-scene `.vac` code

**Success metric:** Feed in a video with 3 distinct scenes, get `.vac` code with 3 `scene` blocks, each containing the shapes from that scene.

### Phase 3: Motion Decompiler
**Input:** A video with moving objects  
**Output:** `.vac` code with `animate` blocks

**Steps:**
1. Shape tracking across frames (match contours between consecutive frames)
2. Optical flow for motion estimation (`opencv-rust` or custom Lucas-Kanade)
3. Motion path simplification (fit Bezier curves to position-over-time data)
4. Easing detection (classify motion as linear, ease-in, ease-out, etc.)
5. Keyframe generation (find the minimal set of keyframes)
6. Generate `animate` blocks in `.vac` code

**Success metric:** Feed in a video of a circle moving from left to right, get `.vac` code with an `animate` block describing that motion.

### Phase 4: Compiler (Prove the Round-Trip)
**Input:** `.vac` code (including decompiler output)  
**Output:** Video file (MP4)

**Steps:**
1. Define `.vac` grammar formally (PEG grammar with `pest`)
2. Build lexer → parser → AST
3. Scene graph builder (AST → renderable scene graph)
4. Frame renderer (`tiny-skia` for 2D rasterization)
5. Video encoder (`ffmpeg-next` for MP4 output)

**Success metric:** `vac decompile video.mp4 | vac compile -o roundtrip.mp4` produces a recognizable reconstruction of the original.

### Phase 5+: Ecosystem
- `.vacb` binary format for efficient storage/playback
- VAC Player (native, WASM)
- VSCode extension (syntax highlighting, preview)
- VAC Studio (visual editor)
- Package manager for reusable components

---

## What I Need to Learn

Since the MVP is the **decompiler**, the learning priorities are:

### Critical (Phase 1)
| Topic | Why | Resources |
|-------|-----|-----------|
| **Edge Detection** | Foundation of shape extraction | Canny edge detector paper, OpenCV tutorials |
| **Contour Tracing** | Extract shape boundaries from edges | Suzuki-85 paper, `imageproc` crate source |
| **Curve Fitting** | Convert point sequences to Bezier curves | Least-squares Bezier fitting, Potrace source code |
| **Color Quantization** | Extract dominant colors from regions | Median cut algorithm, `color_quant` crate |
| **Rust fundamentals** | Implementation language | The Rust Book, `image`/`imageproc` crate docs |

### Important (Phase 2-3)
| Topic | Why | Resources |
|-------|-----|-----------|
| **Optical Flow** | Track motion between frames | Lucas-Kanade paper, Farnebäck method |
| **Video Codecs** | Understand frame extraction, I/P/B frames | FFmpeg docs, H.264 overview |
| **Histogram Analysis** | Scene cut detection | Color histogram comparison algorithms |
| **Shape Matching** | Track shapes across frames | Hu moments, contour matching algorithms |

### Later (Phase 4)
| Topic | Why | Resources |
|-------|-----|-----------|
| **PEG Grammars** | Define `.vac` language formally | `pest` documentation, PEG papers |
| **Compiler Design** | Lexer → Parser → AST → Codegen | Crafting Interpreters, Engineering a Compiler |

---

## Tech Stack

| Component | Choice | Reason |
|-----------|--------|--------|
| **Language** | Rust | Performance, safety, great image/video ecosystem |
| **Image Processing** | `image` + `imageproc` | Pure Rust, no system deps for basic ops |
| **Edge/Contour** | Custom implementation + study Potrace | Core algorithm, need full control |
| **Video I/O** | `ffmpeg-next` | Battle-tested FFmpeg bindings |
| **Optical Flow** | `opencv-rust` (algorithmic modules only) | Proven implementations of Lucas-Kanade/Farnebäck |
| **CLI** | `clap` | Standard Rust CLI framework |
| **Later: Rendering** | `tiny-skia` | For compiler (Phase 4) |
| **Later: GPU** | `wgpu` | For VAC Player |

---

## Project Structure

```
vac/
├── README.md                # This file
├── RESEARCH.md              # Landscape research & competitive analysis
├── Cargo.toml               # Rust workspace
├── crates/
│   ├── vac-cli/             # CLI tool (`vac decompile`, later `vac compile`)
│   │   └── src/main.rs
│   ├── vac-decompiler/      # Core decompiler library (THE MVP)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── frame.rs        # Frame extraction
│   │       ├── scene.rs        # Scene detection
│   │       ├── edge.rs         # Edge detection (Canny/Sobel)
│   │       ├── contour.rs      # Contour tracing (Suzuki)
│   │       ├── curve.rs        # Curve fitting (Bezier)
│   │       ├── color.rs        # Color quantization
│   │       ├── motion.rs       # Optical flow & motion estimation
│   │       ├── shape.rs        # Shape matching across frames
│   │       └── codegen.rs      # .vac code generation
│   ├── vac-format/          # .vac format definition & types
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── ast.rs          # AST types (shared between decompiler & compiler)
│   │       └── writer.rs       # .vac text writer
│   └── vac-compiler/        # Compiler (Phase 4, later)
│       └── src/
│           ├── lib.rs
│           ├── parser.rs       # .vac text parser
│           ├── scene_graph.rs
│           ├── renderer.rs
│           └── encoder.rs
├── tests/
│   ├── fixtures/            # Test images and videos
│   └── integration/         # End-to-end tests
└── docs/
    └── format-spec.md       # .vac format specification
```

---

## The Analogy

| Domain | Raw Format | Structured Format | The Reverse Tool |
|--------|-----------|-------------------|------------------|
| Images | Bitmap (PNG/JPG) | **SVG** | **Potrace** (bitmap → SVG) |
| 3D | Point clouds | **USD / glTF** | Various reconstruction tools |
| Video | **MP4 / H.264** | **VAC** | **`vac decompile`** (MP4 → VAC) |

Potrace took years to get right. VAC is harder — it adds the time dimension.  
But the approach is the same: **measure, don't classify.**

---

## Why This Is Hard (And Why That's Good)

1. **Temporal coherence** — Matching shapes across frames is fundamentally harder than single-image tracing. Shapes deform, occlude, appear, disappear.

2. **Motion decomposition** — Converting pixel-level optical flow into clean animation curves (keyframes + easing) requires sophisticated curve fitting.

3. **Lossy input** — Video compression (H.264) introduces artifacts that make edge detection and contour tracing noisier than working with clean images.

4. **Combinatorial complexity** — A 30fps, 10-second video has 300 frames. Each frame might have 50+ shapes. That's 15,000 shape instances to track and match.

**But this is exactly why no one has done it.** The difficulty is the moat.

The approach: start simple (Phase 1 = single image), prove each algorithm works, then layer complexity incrementally.

---

*"The best time to start was yesterday. The second best time is now."*

*Start with Phase 1. One image in, `.vac` code out. Everything builds from there.*

---

## v0.1: Bidirectional MVP — Moving Ball Round-Trip

The very first slice now in this repo proves the **round-trip** end-to-end on a constrained problem: a single solid-coloured ball moving on a solid background.

```
ball.vac  ──► vac compile  ──►  ball.gif  ──► vac decompile  ──►  ball.roundtrip.vac
```

If both halves work for the ball case, the architecture is sound and every later capability (multiple shapes, scale/opacity, MP4, contour tracing, easing classification) is just a wider domain on the same skeleton.

### Why GIF as the v0.1 video container?

Because it sidesteps the FFmpeg-bindings friction entirely (no system libs, no clang, no `ffmpeg-next` build dance) — pure-Rust I/O via the `image` crate. The round-trip principle is independent of the container; in Phase 2/3 we swap GIF for MP4 with `ffmpeg-next` and the rendering/measurement code stays untouched.

### What's implemented

| Crate | Job |
|------|------|
| `vac` | **Top-level facade** — pandas-style API (`vac::read`, `vac::read_video`, `Document::to_video`, `Document::to_vac`). This is what most callers should depend on. |
| `vac-format` | AST + `.vac` writer + hand-rolled recursive-descent parser |
| `vac-compiler` | AST → frames (tiny-skia) → animated GIF |
| `vac-decompiler` | GIF → per-frame connected-component centroid → RDP-simplified keyframes → AST |
| `vac-cli` | `vac compile` and `vac decompile` subcommands (unzip-style positional args) |

The decompiler is **measurement-only**: per-frame background sampling → foreground mask → connected-component labeling (4-neighbourhood union-find) → centroid + bounding box + mean colour → Ramer-Douglas-Peucker on the trajectory. No models, no training data, deterministic.

### Two ways to use it

**1. Bash CLI — `unzip`-style.** Output filename is optional; if omitted, the result lands next to the source.

```bash
vac compile   ball.vac                 # → ball.gif
vac compile   ball.vac fancy_name.gif  # → fancy_name.gif (any name you want)

vac decompile ball.gif                 # → ball.vac
vac decompile ball.gif my_decoded.vac  # → my_decoded.vac
```

**2. Library API — pandas-style.** `vac::read(...)` is `pd.read_csv`. `doc.to_video(...)` is `df.to_csv`. The output filename is whatever you pass in.

```rust
use vac::DocumentExt;

// .vac file → video file (any name)
let doc = vac::read("ball.vac")?;
doc.to_video("anything_you_want.gif")?;

// video file → .vac file (any name)
let doc = vac::read_video("anything_you_want.gif")?;
doc.to_vac("decompiled.vac")?;
```

Two ready-made example programs in `crates/vac/examples/` show this:

- [`create_video.rs`](crates/vac/examples/create_video.rs) — read a `.vac`, write a video.
- [`convert_to_code.rs`](crates/vac/examples/convert_to_code.rs) — read a video, write a `.vac`.

### Quickstart

```bash
# Once: install the Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Build everything
cargo build --release

# --- Bash CLI ---
cargo run --release -p vac-cli -- compile   examples/ball.vac
#   → writes examples/ball.gif next to the source
cargo run --release -p vac-cli -- decompile examples/ball.gif decompiled.vac
#   → writes decompiled.vac with the measured ball animation

diff examples/ball.vac decompiled.vac

# --- Library API (via the example programs) ---
cargo run --release -p vac --example create_video    -- examples/ball.vac ball.gif
cargo run --release -p vac --example convert_to_code -- ball.gif round_trip.vac

# End-to-end round-trip test
cargo test --release -p vac-cli
```

After installing the binary system-wide (`cargo install --path crates/vac-cli`), the CLI feels truly unzip-flavoured:

```bash
vac compile   ball.vac
vac decompile ball.gif
```

The format itself is documented in `docs/format-spec.md`. Every later version is a strict superset of v0.1.

### v0.1 known limits (each is a focused next slice)

- ~~One ball, one scene, linear easing only — easing classification (RMS-fit against `ease-in/out/in-out` templates) is the next thing.~~ **Shipped in v0.2.**
- ~~Single-shape decompilation — multi-blob tracking via Hu-moment shape matching across frames is the slice after.~~ **Shipped in v0.3.**
- GIF intermediate at ~33fps effective (centisecond granularity) — Phase 2 swaps to MP4 via `ffmpeg-next`.
- No `path()` primitive yet — the Suzuki contour tracing → RDP → Bezier fitting pipeline (the real Phase-1-of-the-original-plan work) plugs in via the same `vac-decompiler` skeleton; only `detect.rs` and `track.rs` widen.

---

## v0.2: Faithful Round-Trip

The v0.1 round-trip had one embarrassing bug: the apex of a ball going *left → right → left* vanished, because 2D Ramer-Douglas-Peucker on the trajectory measures perpendicular distance from the line through the endpoints — and for collinear back-and-forth motion, every interior point lies *on* that line. The decompiler said *"the ball moved from (60, 160) to (68, 160) over 2.67s,"* i.e. *"the ball didn't move."*

v0.2 fixes that and tightens the round-trip in three places:

| Change | Where | Why |
|---|---|---|
| **Temporal-aware RDP** — split the trajectory into two 1D RDPs on `x(t)` and `y(t)`, union the kept indices | `vac-decompiler::track::simplify_trajectory` | Recovers the apex of any back-and-forth motion. Univariate RDP has clean px-only semantics, no mixed-units perpendicular-distance kludge. |
| **Interior-only colour sampling** — pixels along the contour are anti-aliased blends with the background; v0.2 samples only pixels whose 4-neighbours are also in the blob | `vac-decompiler::detect::detect_blob` | Cuts the colour drift from `#e94560 → #e4445e` (L1 = 21) down to within ~6 per channel. |
| **Easing classification** — between each pair of kept keyframes, RMS-fit the measured curve against `linear`, `ease-in`, `ease-out`, `ease-in-out` templates; pick the lowest, default to `linear` on near-ties | `vac-decompiler::track::classify_easing` | Decompiler stops emitting `linear` for everything. Still pure measurement: RMS distance to a template is deterministic. |

Plus a new fixture `examples/diagonal.vac` and a CI smoke test (`.github/workflows/ci.yml`) that runs the full round-trip on every push.

After v0.2, the diff between `examples/ball.vac` and the round-tripped `.vac` is purely cosmetic (whitespace / leading comments) and the GIF-fps quirk (30 → 33fps from centisecond delay quantisation, which goes away when we move to MP4 in v0.4).

---

## v0.3: Multi-shape Tracking

v0.1 and v0.2 hard-coded *the largest blob.* The pipeline could only ever describe one moving object. v0.3 generalises it: detect every blob above the noise floor, stitch tracks across frames, and emit one `let shape_<n>` + `animate shape_<n>` block per recovered object.

The new fixture is `examples/two_balls.vac` — a red ball and a yellow ball moving in opposite directions on different rows. After round-trip, both are recovered with their own keyframes:

```
$ vac compile   examples/two_balls.vac
$ vac decompile examples/two_balls.gif round_trip.vac
$ cat round_trip.vac
canvas 640x360 @33fps
scene main {
  duration 2700ms
  let bg = rect(0, 0, 640, 360)
  bg.fill = #1a1a2e
  let shape_0 = ellipse(60, 100, 22, 22)
  shape_0.fill = #e94560
  shape_0.stroke = none
  animate shape_0 {
    0ms    -> position(60, 100)
    1350ms -> position(560, 100)
    2670ms -> position(71, 100)
    easing: linear
  }
  let shape_1 = ellipse(560, 260, 22, 22)
  shape_1.fill = #fed065
  shape_1.stroke = none
  animate shape_1 {
    0ms    -> position(560, 260)
    1350ms -> position(60, 260)
    2670ms -> position(548, 259)
    easing: linear
  }
}
```

| Change | Where | Why |
|---|---|---|
| **Multi-blob detection** — connected-component labelling now keeps every component above a minimum-area threshold; per-blob area + Hu image-moment invariants are computed in the same pass | `vac-decompiler::detect::detect_blobs` | The renderer was already keyed by shape *name*, not by "the one ball" — generalising detection unlocks the rest of the pipeline for free. |
| **Cross-frame tracker** — lowest-cost-first greedy bipartite matching scored on position + RGB colour + radius + Hu, with a gate threshold and a 3-frame gap tolerance | `vac-decompiler::track::build_tracks` | Stable identities across frames without solving the full Hungarian assignment problem. The gap tolerance survives single-frame detection drop-outs that show up as soon as you have multiple blobs. |
| **Per-track AST emission** — each track gets its own `let shape_<n>` + fill + stroke + `animate` block; v0.2's temporal RDP and easing classification run independently per track | `vac-decompiler::track::emit_track` | Re-uses every algorithm that already worked, just keyed per object. |

The cost weights and gate are tuned for distinct-coloured balls a frame apart. **Two balls of the same colour crossing paths is still an open ID-swap problem** — the right fix is velocity prediction, which arrives with v0.4 once we drop the GIF-fps drift and have stable inter-frame timing.

Hu moments contribute little for purely circular blobs (every circle has roughly the same Hu signature) — they're computed now so v0.5's `path()` primitive can lean on them when shapes actually vary.

---

## v0.4: MP4 Round-Trip — Exact FPS

GIF charged for its zero-system-deps simplicity in centiseconds: a 30fps source round-tripped as 33fps, a 3-second animation became 2.7 seconds, and the apex of a ping-pong ball drifted 15ms. v0.4 fixes all of that by replacing the GIF intermediate with H.264/MP4.

The implementation does **not** use the `ffmpeg-next` Rust bindings. Their latest releases target FFmpeg 6/7, and modern toolchains (FFmpeg 8+) ship without a compatible binding crate yet. Instead, VAC shells out to the system `ffmpeg` binary — same FFmpeg under the hood, no FFI, no `pkg-config`, no `clang`, no version-pinning headaches, and works against any FFmpeg 4+ release.

```
$ vac compile   examples/ball.vac          # → examples/ball.mp4 (default from v0.4)
$ vac decompile examples/ball.mp4 round_trip.vac
$ cat round_trip.vac
canvas 480x320 @30fps                       ← exact, not 33fps
scene main {
  duration 3s                               ← exact, not 2700ms
  let bg = rect(0, 0, 480, 320)
  bg.fill = #1a182e
  let shape_0 = ellipse(60, 160, 24, 24)
  shape_0.fill = #e64561
  shape_0.stroke = none
  animate shape_0 {
    0ms    -> position(60, 160)
    1500ms -> position(420, 160)            ← apex on the millisecond
    2966ms -> position(68, 160)
    easing: linear
  }
}
```

| Change | Where | Why |
|---|---|---|
| **MP4 encoder** — RGBA frames piped to `ffmpeg -f rawvideo -pix_fmt rgba ... -c:v libx264 -crf 18 -pix_fmt yuv420p -movflags +faststart` | `vac-compiler::mp4::encode_mp4` | One subprocess per encode. Stderr is captured and surfaced in the `CompileError`. CRF 18 keeps colour drift ≤ ~6 per channel. |
| **MP4 decoder** — `ffprobe` for `(width, height, r_frame_rate)` + `ffmpeg -i ... -f rawvideo -pix_fmt rgba` for the bytes | `vac-decompiler::frames_mp4::load_mp4` | Reads the rational frame rate, packs the raw bytes into `VideoFrame`s with per-frame `delay_ms` derived from the rational fps so accumulated `t_ms` lands on exact wall-clock instants. |
| **Extension dispatch** — `.mp4` is the new default for `vac compile foo.vac`; `.gif` still works when explicitly requested | `vac::video_format`, `vac-cli::main` | MP4 wins on quality, GIF wins on portability. Pick by extension; both backends share the same renderer + decompiler. |
| **Per-frame timing** — `delay_ms` accumulates the rational fps without rounding error | `frames_mp4::load_mp4` | For 30fps the per-frame delay alternates 33 / 34 / 33 / 33 / 34 … averaging exactly 33.33. The apex of `ball.vac` now lands on `1500ms` instead of v0.3's `1485ms`. |
| **CI MP4 smoke test** | `.github/workflows/ci.yml` | Installs `ffmpeg` on the runner and asserts the round-tripped header reads `@30fps` exactly for both single- and multi-shape fixtures. |

### Sizes

| Fixture | GIF (v0.3) | MP4 (v0.4) |
|---|---|---|
| `ball.vac` (480×320 @30fps, 3s) | ~50 KB | ~7.5 KB |
| `two_balls.vac` (640×360 @30fps, 3s) | 139 KB | 18 KB |

7-8× smaller and exact-FPS in one swap.

### How to test it

You need `ffmpeg` on PATH. On macOS: `brew install ffmpeg`. On Debian/Ubuntu: `apt install ffmpeg`. The MP4 integration tests skip cleanly with a log message if `ffmpeg` isn't available, so the rest of the suite still runs in minimal environments.

```bash
# Sanity-check ffmpeg is reachable
ffmpeg -version | head -1
ffprobe -version | head -1

# Full suite (3 GIF + 2 MP4 round-trip integration tests + 5 unit + 7 format)
cargo test --release --workspace

# CLI round-trip — the v0.4 happy path
cargo run --release -p vac-cli -- compile   examples/ball.vac
cargo run --release -p vac-cli -- decompile examples/ball.mp4 round_trip.vac
cat round_trip.vac
# Expect:
#   canvas 480x320 @30fps          (NOT @33fps)
#   duration 3s                    (NOT 2700ms)
#   1500ms -> position(420, 160)   (apex exact, was 1350ms on GIF)

# Same for the multi-shape fixture
cargo run --release -p vac-cli -- compile   examples/two_balls.vac
cargo run --release -p vac-cli -- decompile examples/two_balls.mp4 round_trip_two.vac
grep -c '^  let shape_' round_trip_two.vac    # expect: 2

# Compare file sizes
ls -lh examples/ball.gif examples/ball.mp4 examples/two_balls.gif examples/two_balls.mp4

# GIF still works when you ask for it
cargo run --release -p vac-cli -- compile examples/ball.vac out.gif
```

### Known limits (queued for v0.5+)

- **libx264 + yuv420p** subsamples chroma at 2×2, which can shift detected centroids by ±1px when a blob's edge happens to cross a chroma block. Tests still pass within the existing ±4px tolerance; switching to `yuv444p` or a lossless codec is a one-line tweak when we need it.
- **No GIF fallback in `vac::DocumentExt`** — extension dispatch handles this, but if you rename a `.gif` to `.mp4` the decoder will fail with a confusing FFmpeg error. Realistically a non-issue.

### What's still on the list

- ~~**v0.5** — `path()` primitive: Suzuki contour tracing → RDP → least-squares Bezier fitting. The original Phase 1 work.~~ **Shipped in v0.6** (via PAC's Moore tracer + Schneider fitter — see the v0.6 section).
- **v0.7** — velocity-aware tracking (handles same-colour ID-swap), scene-cut detection (histogram comparison).
- **v0.8+** — real-world video (codec-artifact-tolerant background estimation, scale/opacity inference), per-keyframe outline morphing for deforming shapes.

### v0.4 — Capability matrix on external inputs

[`docs/v0.4-capability-matrix.md`](docs/v0.4-capability-matrix.md) runs VAC
end-to-end on five inputs spanning the structural→photographic spectrum, four
of which were produced by `ffmpeg` (a tool with zero knowledge of VAC). The
matrix shows VAC is honest for content with discrete shapes (16–33× compression
vs MP4, shape count exact, colour drift ≤ 5/765, apex within one frame) and
fails predictably on photographic / continuous-gradient content — the same
shape of limitation that SVG has against photographs, with the same eventual
fix (a `raster(...)` escape hatch). The doc also enumerates the v0.5+ backlog
each failure row implies. All fixtures regenerate in under a minute with no
new dependencies — see the embedded manual test plan.

### v0.4.1 — Rect / ellipse classifier

The largest single gap the capability matrix surfaced is now closed.
`vac-decompiler::detect` measures each blob's bbox fill ratio
(`area / (bbox_w × bbox_h)`) and classifies it as `ShapeKind::Rect` if
≥ `RECT_FILL_THRESHOLD` (0.90) — a filled rect rasterises at ~1.0, a filled
ellipse at π/4 ≈ 0.785, so a single threshold separates them cleanly even
after H.264 chroma subsampling. `track.rs::emit_track` majority-votes the
kind across each track's samples (robust to per-frame antialiasing flips),
aggregates `rx` and `ry` independently, and emits `Shape::Rect { x, y, w, h }`
or `Shape::Ellipse { cx, cy, rx, ry }` accordingly. The parser also learned
to lex negative numbers in coordinate slots so shapes that start partially
off-canvas (`rect(-86, 120, 180, 60)`) round-trip cleanly. New tests:

- `vac-decompiler::detect::tests::classifier_recognises_filled_rectangle_as_rect` (and friends, 4 total)
- `vac-format::parser::tests::accepts_negative_numbers_in_rect_and_position`
- `vac-format::parser::tests::rejects_negative_dimensions_and_times`
- `vac-cli::tests::sliding_rect_round_trip_preserves_rect_kind_and_aspect` (new fixture: `examples/sliding_rect.vac`)

Visible effect on the capability matrix: `motion_graphics` round-trips as
three coloured *squares* instead of three circles, and `slide_in_card` round-
trips as a 180×60 rectangle instead of a 60×60 disk. Diff before/after in
`examples/wild/outputs/*_compare.png`.

---

## PAC — Picture As Code (v0.2, first-class primitive)

VAC's "measure, don't classify" philosophy lifts cleanly into a still-image
sibling. PAC takes any PNG / JPG / WebP (including AI-generated content) and
emits an editable `.pac` source file where every visible colour region is a
polygon, path, or gradient you can grep, diff, and re-render.

PAC was prompted by an observation: SVG handles geometric shapes beautifully
but **silently degrades to "embed a giant PNG"** for any photographic input.
PAC is built around the principle that the output must always tell you,
honestly, what was vector-decomposable and what wasn't.

**v0.2 graduates PAC from "sibling experiment" to a first-class primitive
the rest of the project depends on.** The thesis: a video is a sequence of
stills with cross-frame constraints. If PAC decomposes a single frame well,
VAC's job collapses to "PAC each frame + match tracks across frames" — so
*every PAC quality improvement flows straight into realistic VAC*. The five
v0.2 improvements (per-region tracing fix, Schneider Bézier curves, auto-K
palette sizing, linear gradient fills, semantic naming + residual report) are
each designed with this map-into-VAC follow-on in mind.

### When PAC is useful

| Input class | PAC output | Use case |
|---|---|---|
| Logos / icons / flat illustrations | ~10–50 polygons; round-trips faithfully | Vectorise without losing source-level editability |
| AI-generated illustrations (Midjourney "flat", DALL-E "vector style") | ~30–100 polygons; recognisable | Edit AI-generated art per region |
| Watercolour / painterly AI portraits | ~70–100 polygons; clearly the subject, stylised | Stylise to editable vector art |
| Photorealistic AI portraits (Mona Lisa class) | ~50–80 polygons; posterised reconstruction | Region isolation; not photo-grade |
| Real photos (faces, foliage) | Same as photoreal AI — informative posterisation | Use the same gap-fill tools you'd use for SVG photos |

PAC does **not** magically un-melt an AI face or fix a 6-fingered hand. What
it gives you is a **structural handle** on top of pixels: every region of the
image becomes a named, recolourable, repositionable polygon. The editing
surface area is closer to Figma than to Photoshop.

### Architecture

| Crate | Job |
|---|---|
| `pac-format` | `.pac` AST + writer + hand-rolled parser (mirrors `vac-format`) |
| `pac-decompiler` | image → AST via K-means colour clustering → per-cluster CCL → Moore-neighbour contour tracing → Ramer-Douglas-Peucker simplification |
| `pac-compiler` | AST → PNG via tiny-skia (same renderer family as `vac-compiler`) |
| `pac-cli` | `pac compile` / `pac decompile`, same unzip-style positional CLI as `vac` |

Decompile pipeline (all deterministic, no ML):

1. Load + decode to RGBA8 (PNG / JPG / WebP / GIF first-frame).
2. Optional downscale so the longest edge ≤ `--max-edge` (default 768).
3. **K-means** over a 6-bit-per-channel weighted histogram → K palette
   colours. Deterministic k-means++ seeding so the same input always
   produces the same `.pac` source.
4. Per-pixel nearest-palette assignment → labelled image.
5. Per-cluster 4-connected component labelling → regions, with
   sub-`--min-area` regions dropped as noise.
6. Pick the largest cluster as the `background` and emit it as the
   canvas fill (skips emitting a shape for it).
7. For every surviving region, Moore-neighbour contour tracing →
   raw boundary → RDP simplification at `--epsilon` pixels → polygon.
8. Sort emitted shapes descending by area so the bottom of the paint
   stack is the largest regions.

### `.pac` grammar (v0.2)

```text
canvas WIDTHxHEIGHT
background #rrggbb         # optional

# Closed polygon (v0.1).
let NAME = polygon(x1 y1, x2 y2, x3 y3, ...)
NAME.fill = #rrggbb

# Closed cubic-Bezier path (v0.2; default decompile output).  SVG-mini.
let NAME = path(M x,y C cx1,cy1 cx2,cy2 x,y L x,y C ... )
NAME.fill = #rrggbb

# Two-stop linear gradient fill (v0.2).  Replaces #rrggbb anywhere.
NAME.fill = gradient(linear, #c0 at x0,y0, #c1 at x1,y1)
```

Declaration order = z-order (back-to-front painter's algorithm), matching SVG.

| Version | Primitive | Status |
|---|---|---|
| v0.1 | `polygon(...)` + `#rrggbb` | ✓ shipped |
| v0.2 | `path(M..L..C..)` Bezier curves (Schneider fitter) | ✓ shipped |
| v0.2 | `gradient(linear, c0 at p0, c1 at p1)` | ✓ shipped |
| v0.2 | auto-K palette + per-region residual report | ✓ shipped |
| v0.2 | semantic region names (`top_left_blue`) | ✓ shipped |
| v0.3 | `gradient(radial, ...)` + multi-stop gradients | planned |
| v0.3 | region merging by residual-change | planned |
| v0.4 | `raster(x, y, w, h, "tile.webp")` bounded escape hatch | planned |

### Try it

```bash
# Render the committed example (uses gradient + path).
cargo run --release -p pac-cli -- compile examples/hello.pac hello.png

# Decompile a still image into editable .pac source (auto-K + Bezier + gradients on by default).
cargo run --release -p pac-cli -- decompile some_picture.png decoded.pac
# → ✓ wrote decoded.pac (768x768, K=24, 148 shapes [15 gradient], residual=84.8)
#     weakest-fit regions: red_8=751, red_9=555, red_4=430

# Round-trip: re-render the .pac to a new PNG.
cargo run --release -p pac-cli -- compile decoded.pac roundtrip.png

# Tuning knobs:
cargo run --release -p pac-cli -- decompile some_picture.png decoded.pac \
    --k 6                  # palette floor for auto-K (default 6)
    --k-max 32             # palette ceiling for auto-K (default 24)
    --target-residual 50   # tighter colour fidelity (default 80; smaller = more shapes)
    --min-area 250         # keep finer details (default 400)
    --epsilon 1.5          # smoother polygon outlines (default 2.0)
    --curve-eps 1.0        # tighter Bezier fit (default 1.5; smaller = more segments)
    --max-edge 1024        # process at higher resolution (default 768)
    --no-smooth            # opt-out of Bezier curves; emit polygons
    --no-gradients         # opt-out of gradient detection; emit solid fills
```

### Capability matrix — AI-generated stress test

[`docs/pac-v0.2-ai-matrix.md`](docs/pac-v0.2-ai-matrix.md) is the **v0.2
update** to the original [`docs/pac-v0.1-ai-matrix.md`](docs/pac-v0.1-ai-matrix.md):
same four CC-licensed AI inputs, but now showing the v0.1 → v0.2 deltas
(smooth Bézier edges, auto-grown palette, gradient-shaded sky/smoke regions,
no magenta artefacts). The 4×3 visual side-by-side lives at
`examples/wild/outputs/pac_v02_results.png`.

Headline numbers (auto-K, default thresholds):

| Image | v0.1 shapes | v0.2 shapes | v0.2 gradients | Auto-K chose | residual |
|---|---|---|---|---|---|
| Dream Machine | 89 | 107 | 5 | K=12 | 64 |
| Night Pool | 57 | 127 | 6 | K=24 | 56 |
| Watercolour Van Gogh | 75 | 148 | 15 | K=24 | 85 |
| Mona Lisa | 67 | 76 | 4 | K=12 | 68 |

---

## v0.5.1 — Stationarity filter

First slice of the capability-matrix backlog: tracks that don't actually
move stop pretending they do.

The decompiler used to decide "does this track animate?" by checking
whether any sample strayed more than 1.5 px from the *first* sample — so
a static label whose centroid flickers ±2 px under GIF palette flips or
H.264 chroma noise got a full `animate` block with a keyframe for every
wobble. v0.5.1 replaces that gate with a proper stationarity test:

| Change | Where | Why |
|---|---|---|
| **`is_stationary`** — a track whose every sample sits within 2.5 px of its **mean** position is static | `vac-decompiler::track` | Deviation from the mean, not the first sample, so a noisy first frame can't disguise a static shape as a mover. |
| **Median anchoring** — static shapes are declared at their median position | `track::emit_track` | Averages out per-frame centroid jitter instead of trusting frame 0. |
| **No `animate` block for static tracks** | `track::emit_track` | A static card is `let card = rect(...)`, full stop — no keyframe salad. |
| New fixture + tests | `examples/static_and_moving.vac`, unit tests in `track.rs`, round-trip test in `vac-cli` | A static card sharing the canvas with a moving ball: exactly one animate block must survive the round-trip. |

Honest measurement on the wild corpus (animate blocks, before → after):

| input | before | after |
|---|---:|---:|
| `real_newtons_cradle` | 60 | **53** |
| `mandelbrot` | 27 | **22** |
| `real_pendulum` | 59 | 58 |
| `real_double_pendulum` | 36 | 36 |
| `real_loader` | 12 | 12 |
| every moving fixture (`ball`, `two_balls`, `sliding_rect`, …) | — | unchanged ✓ |

The interesting result is what *didn't* change. The pendulum-class rows
were predicted to clean up dramatically; measuring the surviving blocks
shows their keyframe amplitudes are 17–300 px — the greedy matcher is
chaining different dash / label / tick fragments into single track
identities. That noise is **track churn, not stationary jitter**, so the
real pendulum fix is velocity-aware tracking (now scheduled v0.7), which
has hard numbers behind it. What the filter reliably buys is the UI /
motion-graphics content class: static cards, labels and logos that
jitter 1–2 px under codec noise now emit as bare `let` statements.

---

## v0.5.2 — Alpha-aware background estimation

Second capability-matrix slice: transparent GIF canvases stop reading
as black.

The GIF decoder fills fully-transparent pixels with `rgba(0, 0, 0, 0)`,
and the corner-sampling background estimator happily averaged that
black into its estimate — which is how `real_loader.gif` (a spinner on
a transparent canvas) round-tripped with `bg.fill = #000000` and an
inverted look. v0.5.2 makes `detect.rs` alpha-aware:

- **Background estimation** skips sub-opaque pixels; if *no* corner
  pixel is opaque the canvas has no measurable background colour, and
  the estimator falls back to white — the colour viewers composite
  transparent GIFs onto.
- **The foreground mask** treats transparent pixels as background by
  definition, so a transparent canvas can't become one giant
  foreground blob the moment the estimated background isn't black.

Measured effect: the loader's background flips `#000000 → #ffffff`
(all 12 tick tracks intact), and opaque inputs are geometrically
untouched — all 117 recovered geometries of `real_pendulum` are
byte-identical before/after; only the tie-break ordering of
equal-length noise tracks shuffles. H.264/MP4 frames decode fully
opaque, so the MP4 path is unaffected by construction.

---

## v0.6 — The `path()` primitive: PAC merges into VAC

The strategic slice. The PAC section above states the thesis: *a video
is a sequence of stills with cross-frame constraints, so if PAC
decomposes a single frame well, VAC's job collapses to "PAC each frame
+ match tracks across frames."* v0.6 is the first realisation of that
map: blobs whose silhouette is neither a rect nor an ellipse now
round-trip as an editable `path(...)` traced by PAC's pipeline, instead
of being flattened into a lying ellipse.

```
let tri = path(M 80,220 L 140,100 L 200,220)       ← .vac source
   │ vac compile → H.264 → vac decompile
let shape_0 = path(M 198.8,216.94 L 79.8,218.94 L 139.8,99.94 L 198.8,216.94)
```

Every vertex back within ~1.2 px, apex keyframe at exactly `1500ms`,
colour drift L1 = 3 — through a lossy video codec.

| Change | Where | Why |
|---|---|---|
| **Shared geometry types** — `Point` + `PathSeg` move into `vac-format`; `pac-format` re-exports them (as it already did `Color`) | `vac-format::ast`, `pac-format::ast` | A `.pac` path segment *is* a `.vac` path segment — PAC's Schneider fitter output feeds the VAC AST with no conversion layer. |
| **`path(M … L … C …)` grammar** — SVG-mini segments, implicitly closed | `vac-format` parser + writer | Mirrors the `.pac` grammar exactly. First segment must be `M`; negative and fractional coordinates lex fine. |
| **Path rendering** — tiny-skia `PathBuilder`, anchored at the outline's shoelace centroid | `vac-compiler::render` | `position(x, y)` keyframes move the area centroid, which coincides with the measured blob centroid to within a pixel — declaration and animation agree. |
| **Three-band classifier** — fill ratio ≥ 0.90 → `rect`; ≥ 0.70 → `ellipse`; below (and area ≥ 64 px) → `path` | `vac-decompiler::detect` | A triangle fills ~0.5 of its bbox, a star ~0.35, a thin diagonal ~0.1 — all far outside the ellipse band. Small blobs stay ellipses; a contour around a dozen pixels is noise, not measurement. |
| **Per-track outline tracing** — Moore contour trace at the track's median-area frame → closed-loop RDP (2.0 px) → Schneider Bézier fit (1.5 px), translated to the declaration anchor | `vac-decompiler::track::trace_track_outline` + `detect::extract_contour`, calling `pac-decompiler::{contour, simplify, curves}` | Literal code reuse — the decompiler crate now depends on `pac-decompiler`. The seed search skips components smaller than ¼ of the blob's area, so a ring-shaped mask (shaded ball with a near-background specular centre) traces the ring, not a noise speck in the hole. |
| New fixture + tests | `examples/triangle.vac`, unit tests in `detect.rs`/`parser.rs`, round-trip test in `vac-cli` | A ping-ponging triangle must come back as `Shape::Path` with ~3 corners, exact-width bbox, and its apex on the millisecond. |

Effect on the wild corpus (primitive counts after v0.6):

| input | paths | ellipses | rects | note |
|---|---:|---:|---:|---|
| `vac_control`, `motion_graphics`, `slide_in_card`, `testsrc2` | **0** | unchanged | unchanged | Geometric content stays on the cheap primitives — no regression. |
| `real_pendulum` | 13 | 80 | 24 | The hatched mount, dashed arc and force arrows now render as recognisable outlines — the round-trip finally *looks like a pendulum diagram* instead of two giant swept ellipses. |
| `real_double_pendulum` | 24 | 11 | 5 | Same story: arms and title fragments become outlines. |
| `real_newtons_cradle` | 34 | 105 | 4 | Still an honest pointillist failure (photographic content needs `raster(...)`), but the fragments are at least shaped like what they measure. |
| `real_bouncing_ball` | 1 | 1 | 1 | The shaded big ball's mask is a *ring* (near-white specular centre); it now traces as a circular outline. This row is why the seed search has the ¼-area guard. |

The remaining honest gap in this slice: paths are traced once per track
(at the median-area frame) and only translated — a shape that *deforms*
over time keeps its representative outline. Per-keyframe outline
morphing is a v0.7+ candidate, behind velocity-aware tracking.

---

## v0.7 — Velocity-aware tracking

Closes the oldest open problem in the tracker (on the books since
v0.3): **two same-coloured balls crossing paths swapped identities.**
Colour, radius and Hu moments are identical for the pair, so the only
signal that can tell them apart is motion — and the matcher ignored it.

The new fixture `examples/crossing.vac` (two identical red balls,
same row, opposite directions, full occlusion merge at the centre)
decompiled like this before:

```
shape_0: 60 → 320 → back to 65     ← bounced (identity swap)
shape_1: 580 → 320, dies           ← track starved in the merge
shape_2: born at 337 → 574         ← phantom appears mid-canvas
```

and like this now:

```
shape_0: 60 → 320 (merge plateau) → 574   ← passes through ✓
shape_1: 580 → 320 (merge plateau) → 65   ← passes through ✓
```

| Change | Where | Why |
|---|---|---|
| **Constant-velocity prediction** — each track extrapolates its next position from its trailing samples; the matcher scores candidates against the *prediction*, not the last seen position | `track::predicted_position`, `matching_cost` | For smooth motion the position term now measures acceleration, not speed: fast movers stay matched, and crossings resolve by direction. Timestamps make it gap-aware — a track unseen for two frames predicts two frames ahead. |
| **Time-bounded velocity window** (250 ms, not a sample count) | `VELOCITY_WINDOW_MS` | At 30 fps the window reaches *behind* the ~130 ms merge plateau (whose samples sit at the merge midpoint and would flatten the velocity to zero); at a 5 fps GIF it degrades to the freshest 2-sample estimate, which strongly curved motion needs. |
| **Bootstrap gate** (160 vs the usual 80) for 1-sample tracks | `BOOTSTRAP_COST_GATE` | A 1-sample track has no velocity model, so a 100 px/frame mover could never acquire its second sample. Wrong bootstraps self-prune: a 2-sample track with bogus velocity predicts into empty space, starves, and falls under `MIN_TRACK_SAMPLES`. |
| **Gap tolerance 3 → 5 frames** | `MAX_GAP_FRAMES` | The losing bidder starves for the whole merge window of a same-size crossing (~5 frames). Velocity prediction keeps stale re-acquisition safe — a dead track predicts *away* from unrelated blobs. |
| **Filled-outline honesty cap** — a path whose outline area exceeds 6× the measured pixel count falls back to an ellipse *shrunk to the measured area* | `PATH_MAX_FILL_OVERCLAIM`, `trace_track_outline` | Surfaced by this slice: chase-tracking rode the double-pendulum's swept-trace web and filled its outline into a canvas-sized slab. Rings (shaded balls, ratio ≈ 4) still fill — sweeps (ratio 10-20×) never render more ink than was measured. |

Honest measurement:

- `crossing.vac` — **fixed** (2 tracks, both traverse, apex plateau is
  the honest record of the occlusion frames). Locked in CI.
- A 100 px/frame mover with a 2-frame detection drop-out — one track,
  no fragments (unit-tested; impossible under the old gate).
- `real_newtons_cradle` — track churn collapsed: animate blocks with
  > 100 px amplitude went from dozens to **one**.
- Every geometric fixture (`ball`, `two_balls`, `sliding_rect`,
  `triangle`, `vac_control`, …) — byte-for-byte the same behaviour.
- `real_pendulum` — **worse on phantom amplitude** (40 blocks over
  100 px, max 421 px vs ~300 before): prediction happily *chases*
  static dashes that blink as the swinging arm occludes them,
  stringing them into longer phantom sweeps. No motion model fixes
  blinking fragments — that row's real cure is occlusion reasoning /
  static-scene layering (v0.8+ candidate), and the capability matrix
  now says so.

---

## v0.8 — Static-scene layering and occlusion reasoning

Every version up to here treated a video as **one layer**: mask each
frame against its background, and whatever survives is a moving object.
That is wrong for the most ordinary kind of footage there is — a scene
with furniture in it. A pendulum's scale, a dial's tick marks, the
wires and end-posts of a Newton's cradle: they never move, but the
mover *passes in front of them*, so from the tracker's point of view
they blink out and back. A blinking blob is indistinguishable from a
fast-moving one.

v0.8 measures the scene before tracking anything:

| Step | What it measures | Where |
|---|---|---|
| **Temporal median plate** | Per pixel, the median colour over the clip. A pixel showing furniture 80% of the time medians to the furniture; a pixel a ball crosses for six frames medians to the background. No thresholds — just the middle value. | `layer::temporal_median` |
| **Agreement** | For every plate pixel that reads as foreground, the fraction of frames whose actual colour matches the plate. *This is the occlusion measurement*: furniture briefly hidden scores 0.75-1.0; an object that merely rests in one spot for half the clip scores ~0.5 and is rejected. | `STATIC_MIN_AGREEMENT` |
| **Conditional strip** | Blank the persistent pixels back to background — **only in the frames where they agree**. Where a static pixel disagrees, something is standing in front of it, and those pixels are left untouched so the occluder keeps its silhouette instead of being punched full of holes. | `StaticLayer::strip` |
| **Emit as scenery** | The plate's shapes are emitted ahead of the tracks as `static_<n>`, through the same classifier, outline tracer and honesty checks a moving object gets (they arrive as one-sample tracks, so they drop out of the animate path for free). | `track::build_document` |

`--no-static-layer` on `vac decompile` turns the whole pass off and
reproduces the v0.7 single-layer pipeline, so every number below is an
A/B against the same binary.

### The fixture

`examples/occlusion.vac` — a ball crossing in front of a static post.
The two touch, so connected components merge them, and under one layer
the damage is total:

```
shape_0: post   + a 10-keyframe animate block wobbling ±6 px
shape_1: ball fragment  276 → 420 → 276
shape_2: ball fragment   60 → 204
shape_3: ball fragment  203 → 68        ← one ball, three tracks
```

With the scene measured and stripped first:

```
static_0: rect(230, 40, 20, 240)        ← exactly the source post, no animate
shape_0:  ball  60 → 420 → 68           ← one unbroken track
```

Locked in CI.

### Honest measurement (whole wild corpus, same binary, `--no-static-layer` vs default)

| Input | shapes | animate blocks | keyframes | `.vac` size |
|---|---|---|---|---|
| `real_double_pendulum` | 49 → 91 | **45 → 19** | **1159 → 186** | **54.0 KB → 25.5 KB** |
| `real_loader` | 12 → 15 | **12 → 3** | **75 → 28** | **3.9 KB → 2.4 KB** |
| `real_newtons_cradle` | 137 → 185 | **50 → 38** | **354 → 267** | 33.2 KB → 36.5 KB |
| `real_pendulum` | 119 → 110 | 63 → 60 | 496 → 480 | 35.0 KB → 33.3 KB |
| `real_bouncing_ball` | 2 → 2 | 1 → 1 | 33 → 36 | 1.6 KB → 1.7 KB |
| `mandelbrot` | 26 → 45 | 21 → 27 | 77 → 128 | 8.0 KB → 15.9 KB |
| `testsrc2` | 1 → 75 | 0 → 47 | 0 → 458 | 0.2 KB → 25.0 KB |
| `motion_graphics`, `slide_in_card`, `vac_control` | byte-for-byte identical (no persistent scenery to measure) | | | |

Reading the rows that got *bigger*:

- **`real_newtons_cradle`** — the +48 shapes are the words *PRINCIPIA
  MATHEMATICA* on the book under the cradle. They were always in the
  frame; one layer swallowed them, and they now come out as scenery
  legible in the render. Meanwhile the moving pass got quieter: 12
  fewer animate blocks, 87 fewer keyframes.
- **`testsrc2`** — the whole test pattern used to collapse into a
  single canvas-sized blob (194 bytes of output, a flat grey
  rectangle). Stripping the persistent slab exposes what was
  underneath, and the round-trip now shows the checkerboard, the
  diagonal sweep and the coloured blocks. Still nothing like the
  source, and the colour bars still merge into one grey component —
  VAC has no colour segmentation, which is a separate gap.
- **`mandelbrot`** — the extra shapes buy a visibly truer cardioid
  silhouette (see `mandelbrot_compare.png`); the fractal remains
  out of scope by design.

### The counter-finding: v0.7's diagnosis of the pendulum was wrong

v0.7 recorded that `real_pendulum`'s phantom tracks were static dashes
blinking as the arm occluded them, and routed the fix here. Building
the fix disproved it. Layering cleans up the pendulum's actual
scenery — the hatched ceiling and dotted plumb line now render crisply
instead of as smeared artefacts — but the phantom amplitude is
unchanged (16 blocks over 100 px, max 191.8 px, identical with and
without the pass).

Reading the offenders shows why: every one of them is a `#007f00`,
2 px-radius blob. They are the dots of the **dotted green force
vectors**, which are not scenery at all — they *move*, and a dotted
stroke rasterises into dozens of interchangeable 4-pixel fragments.
The tracker matches dot *k* in one frame to dot *k+3* in the next
because nothing distinguishes them. That is a fragment-grouping
problem (do these dots share one rigid transform?), not an occlusion
problem, and no amount of scene measurement will touch it.

### Also fixed: the decompiler was not reproducible

Found while A/B-ing the corpus: **the same binary on the same file
produced different output between runs.** `detect_blobs` collected its
kept labels straight out of a randomly-seeded `HashMap`, so the blob
order — and with it the tie-break between equal-area blobs and the
emitted `shape_<n>` numbering — was whatever the hasher felt like that
process. `motion_graphics` (three identical 44×44 squares) flipped
between two orderings roughly half the time.

Labels come out of the raster-scan union-find in first-appearance
order, so sorting them restores determinism, and equal-area blobs now
tie-break in raster order. Ten runs across the corpus, byte-identical.
A decompiler whose output isn't reproducible can't be measured, and
every before/after table in this README depends on it.

### Still open after v0.8

- **Dashed and dotted strokes fragment.** The pendulum row above.
  Wants co-moving fragment grouping: cluster blobs that share a rigid
  transform between frames and emit them as one object.
- **No colour segmentation.** Adjacent regions of different colours
  merge into one component (the `testsrc2` bars, the Newton's cradle
  frame). PAC already does k-means palette extraction; VAC's mask is
  still binary foreground/background.
- **Scene cuts.** A single median plate assumes one continuous shot.
- **Per-keyframe outline morphing** for shapes that deform as they
  move — the outline is traced once, at a representative frame.

---

## v0.9 — Closing the loop: reconstruction becomes measurable

`RESEARCH.md` states VAC's claim in four words: **Deterministic ·
Measurable · Editable · Human-Readable**. Eight versions in, three of
those were true.

Nothing in this repository had ever compared reconstructed pixels
against original pixels. Zero occurrences of PSNR, SSIM, MAE, RMSE or
any per-pixel diff, in Rust, shell or Python. The only "compare"
operation in the project was an ffmpeg `hstack` producing a side-by-side
PNG, followed literally by `# 6. Open the .png pairs in your image
viewer of choice.`

Every measured claim in the eight sections above is a **proxy**: shape
counts, animate-block counts, keyframe counts, `.vac` byte size. Those
describe how *small* the output is. None of them describes how *right*
it is, and a proxy can improve while the picture gets worse.

v0.9 adds the missing axis.

```
vac verify clip.gif          # decompile → re-render → score against the source
vac verify clip.gif --json   # machine-readable, feeds the baseline
vac verify clip.gif --ab     # the v0.8 static-layer switch, finally measured
```

### What is measured

Three numbers, because one cannot express what goes wrong here.

| Metric | Answers | Catches |
|---|---|---|
| **ink IoU** | Is the ink in the right place? | A pendulum reconstructing as three superimposed bodies |
| **residual** | Are the colours right? (per-channel MSE; 100 ⇒ each R/G/B within ~10) | Colour bars reconstructing as a flat grey field |
| **worst frame** | Did it collapse *anywhere*? | A tracking failure a mean would hide entirely |

PSNR is reported alongside, derived from the same MSE for free, because
it is the number everyone reads. It is not the gate: squared error
concentrates at edges, so a recovery whose edges sit 1–2 px off scores
worse than a blurry aligned one.

The ink reference is a **modal colour measured by `vac-metrics` itself**,
deliberately not `detect::estimate_background`. A metric that shared the
pipeline's own background rule would silently rescale every baseline the
moment that rule was tuned, and "the decompiler improved" would become
indistinguishable from "the ruler moved."

### The corpus, measured

First real fidelity numbers in the project's history.

| input | ink IoU | worst | residual | psnr dB | spurious ink | shapes |
|---|---|---|---|---|---|---|
| `motion_graphics.mp4` | 0.9762 | 0.9562 | 7.4 | 39.46 | 0.8% | 4 |
| `vac_control.mp4` | 0.9596 | 0.9449 | 9.1 | 38.54 | 2.9% | 3 |
| `real_bouncing_ball.gif` | 0.8758 | 0.8736 | 413.8 | 21.96 | 13.0% | 3 |
| `testsrc2.mp4` | 0.8398 | 0.8387 | **15127.9** | 6.33 | 19.1% | 76 |
| `slide_in_card.mp4` | 0.8182 | **0.0000** | 157.4 | 26.16 | 14.2% | 2 |
| `mandelbrot.mp4` | 0.7162 | 0.6362 | 4644.6 | 11.46 | 27.3% | 46 |
| `real_newtons_cradle.gif` | 0.5996 | 0.5948 | 732.4 | 19.48 | 57.0% | 186 |
| `real_loader.gif` | 0.5012 | 0.4766 | 258.9 | 24.00 | 76.2% | 16 |
| `real_pendulum.gif` | 0.1579 | 0.1006 | 2122.7 | 14.86 | **230.1%** | 111 |
| `real_double_pendulum.gif` | 0.0706 | 0.0386 | 8425.9 | 8.87 | **616.6%** | 92 |

The top two rows are VAC's own output and ffmpeg `drawbox` rectangles —
flat-shaded content of exactly the kind the detector was built for. The
spread between those and the bottom of the table is the honest statement
of where VAC stands.

### Correcting the record

**v0.8's headline win was a compression win, not a fidelity win.** It
recorded the double pendulum improving 45 → 19 animate blocks, 1159 →
186 keyframes, 54 KB → 25 KB. All of that is true and none of it was
wrong. Measured on the other axis, the same clip reconstructs at an ink
IoU of **0.0706**, painting **6.2× more ink than the source contained**.
Both facts were always true; only one of them was measurable.

The same applies, more mildly, to the Newton's cradle row (v0.8: "+48
shapes, the PRINCIPIA MATHEMATICA text now legible"). The text is
partially legible. The swinging balls — the subject — are gone. IoU
0.5996, 57% spurious ink.

None of this means v0.8 was a mistake. It means v0.8 could not tell.

### What the first run found

Three defects that eight versions of count-based testing walked past.
Full detail with fixtures and mechanisms in [`docs/frontier.md`](docs/frontier.md).

**1. Objects that appear after t=0 are time-shifted.** `emit_shape`
normalises keyframe times to the track's first sighting and the emitted
`animate` block carries no start offset, so a track beginning at 1200 ms
emits its first keyframe at 0 ms. On `examples/late_entry.vac` — a card
that holds off-canvas until 1000 ms and parks at 2000 ms — the recovered
source parks it at **840 ms**.

The recovered AST looks perfect: two shapes, one animate block, four
keyframes, correct geometry, correct colour. **Every structural count
matches.** No count-based assertion can see this defect. Only pixel
comparison can. That single fixture is the argument for this whole
version.

**2. Repeated round-trips inflate shapes without converging.** Feeding
the pipeline its own output, ink IoU falls 0.9523 → 0.9149 → 0.8788 →
0.8459 over four generations, because each recovered ellipse comes back
one pixel wider than the one that drew it (22 → 23 → 24 → 25 → 26 → 27),
forever. `sliding_rect` gains 2 px of width per generation. Anyone using
VAC iteratively watches their shapes swell.

The *trigger* is not yet established — a static shape never drifts, but
neither does the moving ellipse in `static_and_moving.vac`, which
travels horizontally exactly like the two that do. It is recorded
undiagnosed rather than explained away. v0.7 shipped a confident
diagnosis that v0.8 disproved; once was enough.

**3. A test that could never fail.** `fitted_curve_stays_within_tolerance`
in `pac-decompiler` discarded its fitted curve and asserted
`dist(p, p) == 0`. It was the only assertion-free test in the workspace,
and it is now repaired to measure real point-to-segment deviation. (My
first repair reported the fitter missing its tolerance by 3.5×; that was
my flattening being too coarse, not a fitter bug. The fitter is fine.)

### Testing, rebuilt

71 tests → **118**. Five tiers, and the two new ones cover complementary
ground — verified by deliberately breaking the pipeline and seeing which
tier notices:

| Perturbation | Hermetic tier | Corpus tier |
|---|---|---|
| `FG_THRESHOLD_SQ` 24² → 60² | **caught** (IoU 0.9997 → 0.9839) | caught |
| `RDP_TOLERANCE_PX` 2.0 → 20.0 | missed | **caught** (IoU 0.5012 → 0.2258) |

The **hermetic tier** runs `.vac → render → decompile → render → compare`
with no codec, no ffmpeg, no network and no temp files, so it isolates
the decompiler's own error. It is a sensitive regression detector and
*not* evidence about real video — it feeds the detector exactly the
flat-shaded content its thresholds were tuned on. The **corpus tier** is
the one that measures progress.

That distinction has teeth. An earlier draft of the hermetic tier used a
single global IoU floor of 0.80 against measured values of 0.91–0.9997,
and under it the `FG_THRESHOLD_SQ` break passed all twelve tests. Floors
are now per-fixture.

Corpus baselines live in `benches/corpus-baseline.tsv` — the repo's first
machine-readable measurement record. Inputs are gitignored and
externally fetched, so each row carries an FNV-1a hash of its input: a
changed file is reported as *changed*, loudly skipped, never as a
fidelity regression. Recording is separate from gating; a baseline that
silently self-updates measures nothing.

```
cargo test --release -p vac --test corpus -- --nocapture
VAC_UPDATE_BASELINES=1 cargo test --release -p vac --test corpus
```

### A bug found in the measuring apparatus itself

Worth recording because it nearly poisoned everything above. The first
version of the frame bridge attached a flat `delay_ms = 1000/fps = 33`,
which looks equivalent to what the renderer does and is not: the
renderer stamps frame *i* at `i*1000/fps` with integer division, giving
0, 33, 66, **100**, 133. Flat delays drift 0.33 ms per frame, reaching a
full frame by the end of a 3 s clip.

Under that bug `ball.vac` scored 35.7 dB and IoU 0.893. With rational
stamping it scores **61.6 dB and 0.9986** — the harness had been
measuring a defect it introduced itself. Validating the metric against
known-answer cases *before* measuring anything with it is not ceremony.

### Still open after v0.9

Ranked by measured fidelity cost, which inverts the previous ranking —
the v0.8 list put fragment grouping first, chosen by proxy counts.

- **Colour: the single-flat-background model.** The whole background
  model is four 5×5 corner blocks reduced to a *per-channel* median, so
  it can name a colour appearing nowhere in the frame. `testsrc2`
  residual 15127.9. PAC — already a dependency — has
  `kmeans::quantize_auto` and `regions::label_regions_per_cluster`. v0.6
  took PAC's pen and left its eyes behind.
- **Dotted strokes fragment.** Now quantified: 230% and 617% spurious
  ink on the two pendulums.
- **Late-entry time shift**, **generational inflation**, **`scale()` /
  `opacity()` never recovered**, **strokes never recovered** — see
  `docs/frontier.md`.
- **`apply_animation` drops held transforms.** Latent only because a
  single transform kind is ever emitted; it becomes live the moment
  scale or opacity recovery lands, so it must be fixed first.

## v0.10 — Late entries keep their timing

The first slice chosen by measurement rather than by proxy. v0.9's frontier
list opened with a defect no count-based test could see: `emit_shape`
re-based every track's keyframes to its first sighting, so anything that
entered after t=0 replayed its whole trajectory early.

Three changes in `crates/vac-decompiler/src/track.rs`:

1. **Absolute keyframe times.** No re-basing.
2. **Edge-clipped centroids are corrected.** A card 3 px on screen has a bbox
   centre 1.5 px in; the true centre is recovered from the visible opposite
   edge and the track's full extent, before trajectory simplification.
3. **Entry/exit through an edge is held off-canvas** until the last unseen
   frame (and from the first unseen frame after an exit).

| | ink IoU | worst frame | residual |
|---|---|---|---|
| `late_entry.vac` (hermetic) | 0.394 → **0.9928** | 0.000 → 0.667 | 906.3 → 1.70 |
| `slide_in_card.mp4` | 0.8182 → **0.9918** | 0.0000 → **0.8792** | 157.4 → 3.0 |
| `real_pendulum.gif` | 0.1579 → 0.1906 | 0.1006 → 0.1045 | 2122.7 → 2059.5 |
| `real_double_pendulum.gif` | 0.0706 → 0.0792 | 0.0386 → 0.0476 | 8425.9 → 7627.0 |
| `mandelbrot.mp4` | 0.7162 → 0.7235 | 0.6362 → 0.6469 | 4644.6 → 4497.5 |

No corpus row or hermetic fixture regressed. The frontier test became a real
floor (`late_entering_objects_keep_their_timing`), `late_entry.vac` joined the
hermetic baselines, and the row left `docs/frontier.md`.

**Still open:** a late-appearing object that does *not* enter through an edge
(pops or fades in mid-canvas) is drawn from t=0 — hiding it needs `opacity()`,
which is blocked on the `apply_animation` hold bug.

## v0.11 — Things that grow and fade

v0.10 ended blocked: a late object that appears mid-canvas needs `opacity()`,
and emitting any second transform kind was unsafe because of a renderer bug.
This slice does it in the order the frontier doc demanded.

1. **Renderer: per-channel transforms.** `apply_animation` used only the two
   keyframes bracketing `t`, so a transform kind missing from both reverted
   to its base value. Each kind now interpolates between the nearest
   keyframes that *carry* it and holds outside them. Two unit tests pin the
   exact case that used to break.
2. **Decompiler: `scale()`.** Per sample, `sqrt(area / median area)`,
   median-filtered over 5 samples. Samples whose aspect ratio departs from
   the track's (two balls merging side by side double the width, not the
   height) hold the previous value. Without that guard `crossing.vac`
   emitted a phantom 1.25× pulse at the merge.
3. **Decompiler: `opacity()`.** The renderer composites
   `bg + a·(fill − bg)`, so each sample's colour is projected onto that
   segment. A sample off the segment means a hue change, and nothing is
   claimed.
4. **Static layer: growing cores are not scenery.** A growing disc's centre
   is covered in every frame, so it passed the agreement test and the motion
   pass got a ring that shattered into 10 tracks. A compact component whose
   own colour keeps spilling past its outline, and whose surround is plain
   background in other frames, goes back to the motion pass. The first
   version of this rule cost `mandelbrot.mp4` 0.72 → 0.65 IoU. The corpus
   gate caught that, and compactness plus the background condition fixed it.

| fixture | ink IoU | residual | shapes |
|---|---|---|---|
| `grow.vac` | 0.644 → **0.970** | 824 → 18.6 | 10 → 1 |
| `fade.vac` | 0.9995 → 1.0000 | 59.8 → **0.01** | 1 → 1 |

**No corpus row moved.** Nothing in the wild corpus scales or fades cleanly,
so this is a hermetic win, not evidence about real footage. The new tests
check recovered values against the source curve (radius within 3 px, opacity
within 0.05), and the pre-v0.11 fixtures must stay position-only. Each of the
three mechanisms was deliberately disabled and a test failed every time.

## v0.12 — Only drawn while alive

With `opacity()` available, a track that first appears or last appears
mid-canvas is hidden outside its lifetime. v0.10's edge rule still handles
objects that enter or leave through the canvas border.

| | ink IoU | spurious ink | residual |
|---|---|---|---|
| `pop_in.vac` (hermetic) | 0.662 → **0.988** | 50.5% → 0.5% | 323.9 → 2.4 |
| `real_pendulum.gif` | 0.191 → **0.376** | 228% → **45%** | 2059 → 872 |
| `real_double_pendulum.gif` | 0.079 → **0.174** | 559% → **136%** | 7627 → 2456 |
| `real_newtons_cradle.gif` | 0.600 → 0.605 | 57.0% → 57.1% | 728 → 706 |

The fixture was written for a small case, and the corpus gain was not
predicted. The pendulums' dotted strokes break into many short tracks. Each
one used to stay on screen for the whole clip at its first or last position,
and that was most of their 6× ink overclaim. The fragmentation itself is still
open (`docs/frontier.md`). v0.12 stopped the fragments from outliving
themselves, not from existing. `mandelbrot.mp4` moved 0.7235 → 0.7227, inside
tolerance.

## v0.13 — Colour, decided by measurement

v0.9's first fidelity run showed `testsrc2`, a test card of six vivid colour
bars, coming back as a flat grey field (residual 15063, about 123 of 255 off
per channel). `vac verify --dump`, new in this release, writes source |
reconstruction | ink-map triptychs, and the triptych showed two causes. Blob
detection was colour-blind: a blob was a connected component of "not
background", so touching bars fused. And the background guess was a
per-channel median of four corners, which can name a colour that appears
nowhere in the frame.

Four changes, each kept only if it measured better:

1. **Region labelling.** Two neighbouring pixels share a region only if their
   colours are close. Neighbour-to-neighbour joining chains through smooth
   gradients and splits at edges. Anti-aliased and codec-blurred edges would
   otherwise turn into thin fringe regions, so fringes are absorbed into the
   solid region they border most, repeating until nothing moves.
   - *Tried and rejected:* absorbing only fringes whose colour is a blend of
     their two sides. H.264 chroma subsampling shifts fringe colours off the
     blend line, and `vac_control` regrew 16 fringe shapes.
2. **Border-mode background.** The colour covering most of the frame's
   border. A mostly transparent border means a white background: voting
   among the few opaque pixels crowned the `real_loader` spinner's tick
   colour, and 9 of its 12 ticks vanished.
3. **Primitive choice by measured fit.** Each blob's rect and inscribed
   ellipse are scored by IoU against its actual pixels. The better fit wins,
   and below 0.88 the outline is traced as a path. The fill-ratio bands this
   replaces called a bar notched by a caption box an *ellipse*.
4. **Measured selection.** Regions and the border rule help some clips and
   hurt others: on the Mandelbrot zoom, regions shatter the picture into 289
   shapes and 261 KB of source while scoring *worse*. So no rule is
   hard-coded. The facade decompiles every combination, re-renders each,
   scores it against the source, and keeps the best reconstruction, the way
   a video encoder makes a rate–distortion mode decision. When two
   candidates trade off, relative changes in residual and ink IoU are
   weighed equally, so a 10% IoU loss is never bought with a 1% residual
   gain.

| clip | chosen | ink IoU | residual |
|---|---|---|---|
| `testsrc2.mp4` | regions + corners | 0.840 → **0.984** | 15063 → **1711** |
| `real_double_pendulum.gif` | regions + border | 0.174 → **0.410** | 2456 → 1199 |
| `real_newtons_cradle.gif` | regions + corners | 0.605 → 0.641 | 706 → 543 |
| `mandelbrot.mp4` | components + border | 0.723 → 0.765 | 4508 → 4186 |
| `two_tone.vac` (new fixture) | regions + border | 0.49 → **0.991** | 6676 → **83** |

No corpus row regressed. `vac verify` reports which candidate won, and
`--segmentation` and `--background` pin either knob.

**Found along the way** (`docs/frontier.md`):
- VAC has no rotation, so a pendulum rod is drawn frozen at one angle.
- Gradients have no primitive.
- The ink metric is blind on transparent canvases: `real_loader`'s IoU
  measures only its three highlighted ticks. That fix changes the ruler, so
  it gets its own commit.

## v0.14 — Rotation

`vac verify --dump` on the pendulum showed its rod drawn frozen at one angle,
sliding. VAC could move, scale and fade a shape, but not turn it. The same gap
explained the rainbow band that sweeps across `testsrc2` and the loader's
diagonal ticks.

- **Language:** `rotate(deg)`, clockwise, about the shape's centre, the way
  CSS `transform-origin: center` works. It is a per-channel transform like
  the others.
- **Detector:** a blob's principal axis and its extent along and across it
  come from the second central moments it already computed for the Hu
  invariants: `θ = ½·atan2(2μ₁₁, μ₂₀ − μ₀₂)`. A solid W×H rect has variances
  W²/12 and H²/12, so its true side lengths are recoverable at any angle. An
  oriented rect and an oriented ellipse of that size are scored by IoU
  against the pixels, next to the axis-aligned fits, and must win by a
  margin so near-axis shapes don't flicker.
- **Tracker:** a track whose samples mostly fit oriented primitives is
  sized along its own axes. Its angle series is unwrapped (principal axes
  are only defined mod 180°) and simplified with a tolerance measured in
  pixels at the shape's tip. Elongated paths carry rotation relative to the
  frame their outline was traced from.
- **Static layer:** the centre of a spinning bar is covered in every frame,
  so it looked like scenery, and stripping it cut the bar in half. The
  bleeding-core test now needs 20% of the surrounding ring, not 50%, to show
  the shape's colour. That was measured on the corpus as the loosest
  setting that leaves the loader unchanged.

| | ink IoU | residual |
|---|---|---|
| `spin.vac` (new fixture) | 0.353 → **0.917** | 673 → **40** |
| `testsrc2.mp4` | 0.984 → 0.987 | 1711 → **1349** |
| `real_pendulum.gif` | 0.376 → **0.420** | 872 → **657** |
| `real_double_pendulum.gif` | 0.410 → **0.457** | 1199 → 1056 |
| `real_loader.gif` | 0.698 → 0.691 | 259 → **228** |

No corpus row regressed. `testsrc2`'s residual is now **11.2× lower than
v0.12's 15063**: the colour slice's 10× target, met across v0.13 and v0.14.

## v1.0 — A codec, with a curve

Since v0.9 VAC has been able to measure how good a reconstruction is. v1.0
spends that measurement on the claim the project was built toward: **VAC is
a lossy video codec whose compressed form is human-readable source code**,
with a published, measured rate–distortion trade-off.

- **`--quality 0..100`.** One dial scales every lossy tolerance in the
  decompiler. 50 is the default and reproduces v0.14 byte for byte. Each
  knob moves only in the direction it was *measured* to trade bytes for
  fidelity. Keyframe simplification scales both ways, because that's where
  the bytes go. Outline detail only coarsens, because finer outlines
  measured worse: an outline traced from one frame overfits it.
- **Shape-level rate–distortion pruning.** Below quality 50, a shape is
  dropped when the clip error it removes, per byte of source it costs,
  falls under a threshold the dial sets. The first version scaled the
  minimum blob area instead, and at quality 0 it deleted every tick of the
  `real_loader` spinner (IoU 0.69 → 0.00). The test that pins this fails on
  that code.
- **`--target-psnr DB`.** Rate control: probe the dial, bisect, and keep the
  fewest-bytes reconstruction that meets the target. The curve isn't
  assumed monotone; it's measured.
- **A free improvement found by the sweep.** The Bézier fit tolerance drops
  1.5 → 0.75 px. Across the whole corpus that costs 47 bytes and improved
  every row it moved.

Selected points (full curves in `docs/rate-distortion.md`):

| clip | point | `.vac` bytes | vs source video | PSNR |
|---|---|---|---|---|
| `slide_in_card.mp4` | default | 391 | 19.5× smaller | 43.4 dB |
| `vac_control.mp4` | default | 564 | 33.5× smaller | 38.5 dB |
| `motion_graphics.mp4` | default / q100 | 781 / 4,255 | 16× / 3× smaller | 39.5 / 43.2 dB |
| `testsrc2.mp4` | q0 / default | 29,494 / 216,127 | 10.6× / 1.4× smaller | 16.5 / 16.8 dB |
| `real_newtons_cradle.gif` | q0 / default | 11,103 / 51,139 | 27× / 5.8× smaller | 19.5 / 20.9 dB |

Two things that table says honestly:

- On flat-shaded motion graphics, the readable source is an order of
  magnitude smaller than the H.264 it came from, at 38–44 dB.
- On a noisy test card or a photograph, the default spends a lot of bytes
  for little fidelity, and the dial shows that. On `testsrc2`, quality 0
  is 7× smaller than the default for 0.4 dB.

The distortion axis also still has the gaps listed in `docs/frontier.md`:
gradients, strokes, and dotted strokes.

## v1.1 — A fixed point, and the ground

**Repeated round trips now reach a fixed point.** v0.9 found that feeding
VAC its own output grew every moving shape by a pixel per generation:
`two_balls` radius 22 → 27 over five generations. v0.9 recorded the cause as
undiagnosed. The mechanism, now measured:

- a shape moving at sub-pixel speed straddles a pixel on *both* edges in
  most frames;
- any pixel more than about 17% covered passes the foreground threshold;
- so the thresholded bbox reads the shape about a pixel wide, and the
  median rounds up.

Axis-aligned rects and ellipses are now sized from **coverage**. Each pixel
counts by the fraction of it the shape covers, read from where its colour
sits between the background and the local fill.

- **Rects** use the bbox corrected by the median coverage along each edge.
  That's exact for an anti-aliased edge, and a notch or a rounded pixel-art
  cap doesn't disturb it.
- **Ellipses** use coverage-weighted variance.
- **Blobs the model doesn't describe keep the bbox:** shaded ones (measured
  by their own interior spread), transparent edges, low contrast, and poor
  primitive fits. Each of these exclusions was added after the corpus
  measured it failing: a shaded ball shrank, a pixel-art loader tick lost a
  row, and a diagonal tick in the ellipse fallback lost width.

Generation 1 now lands at the source's exact size and stays there. The
property test that documented the drift is now
`repeated_round_trips_reach_a_fixed_point`.

**Ground.** A region a few shades off the background never clears the
foreground threshold, so it was painted the background colour.
`real_double_pendulum`'s plot area is near-white inside an off-white margin:
a 13-level error over 420 000 pixels. Large rect-shaped regions in that band
of the static median are now emitted first, as `ground_<n>`.

| | ink IoU | residual |
|---|---|---|
| `two_balls.vac` | 0.952 → **0.989** | 8.75 → **1.79** |
| `diagonal.vac` | 0.916 → **0.991** | 18.89 → **0.76** |
| `sliding_rect.vac` | 0.983 → **0.999** | 7.05 → **0.45** |
| `panel.vac` (new) | 0.980 | 40.9 → **0.44** |
| `vac_control.mp4` | 0.960 → 0.962 | 9.1 → **3.0** (38.5 → 43.4 dB) |
| `real_double_pendulum.gif` | 0.463 → 0.435 | 1034 → **766** |

The double pendulum's IoU drop is a recorded trade-off. Its 1.3 px
anti-aliased plot lines are now declared at their measured width instead of
painting their light fringe solid dark. That fringe counted as ink, but in
the wrong colour, and residual, which sees colour, improved 26%.

## v1.2 — Gradients

Through v1.1 every fill was one flat colour, so a lit ball came back as its
average grey. v1.2 adds two-stop gradient fills, using PAC's syntax for the
linear form:

```vac
ball.fill  = gradient(radial, #fff2c0 at 76,216, #b05010 radius 52)
panel.fill = gradient(linear, #203a6a at 40,85, #7fb0f0 at 440,85)
```

Gradient points live in the shape's declared coordinates. The renderer
applies the shape's transform to the shader too, so the shading moves,
scales and turns with the shape.

For each shape, the decompiler fits its interior pixels (anti-aliased edges
excluded) three ways:

- **flat:** one colour, as before;
- **linear:** a least-squares colour plane per channel, reduced to the
  dominant common direction and two stops;
- **radial:** colour against distance from a centre found by
  coarse-to-fine grid search, because a lit ball's highlight is rarely at
  its centre.

A gradient is kept only if it *describes* the region: at least 60% of the
flat error removed and at most 25 levels RMS left. The first version kept
any gain over 35%. On the Mandelbrot clip it laid a green-to-pink radial
smear over the fractal, which improved residual but cost 0.05 ink IoU. A
fractal or a photograph isn't a two-stop gradient, and the fitter now says
so.

| | PSNR | ink IoU | residual |
|---|---|---|---|
| `shaded.vac` (new) | 22.2 → **34.9 dB** | 0.995 → 0.995 | 390 → **21** |
| `real_bouncing_ball.gif` | 21.9 → **25.0 dB** | 0.869 → **0.885** | 420 → **204** |

The bouncing ball had drifted slightly below v0.12 over the earlier
releases (0.876 → 0.869). It's now well above it. No corpus row regressed.

## v1.3 — Colour animation

VAC could move, scale, turn and fade a shape, but not recolour it, and
motion graphics recolour constantly. v1.3 adds a `fill()` keyframe
transform, interpolated in RGB per channel like everything else:

```vac
animate ball {
  0ms    -> position(80, 100), fill(#e04040)
  1500ms -> position(240, 100), fill(#4060e0)
  3s     -> position(400, 100), fill(#40c060)
}
```

The decompiler gives a track a colour channel when its interior colour strays
more than 24 RGB units from its median, and an opacity fade doesn't already
explain the change. One guard came straight from measurement. On the
Mandelbrot zoom, a region's track jumped to the pink ring around it, and the
first draft faithfully "animated" that jump: one frame's IoU fell from 0.40
to 0.09 while the clip's mean moved 0.005, so the gate passed. A real colour
animation changes colour, not shape, so a colour jump that coincides with an
area jump is now treated as a tracking change. The corpus gate also checks
worst-frame IoU now, so a collapse like that one fails the build.

| | PSNR | residual |
|---|---|---|
| `color_shift.vac` (new) | 28.1 → **47.1 dB** | 101.7 → **1.27** |
| `testsrc2.mp4` | 16.84 → 16.86 dB | 1347 → 1341 |

## v1.4 — Strokes, and exact positions

**Strokes.** Until v1.4 the decompiler wrote `stroke = none` for everything,
and the renderer always stroked at 2 px. Now:

```vac
let card = rect(301.5, 221.5, 147, 67)
card.fill = #ff5050
card.stroke = #ffffff
card.stroke_width = 3
```

- A blob that encloses a large hole, and whose *filled* outline is a genuine
  rect or ellipse, is that primitive stroked: unfilled, drawn along the
  ring's centreline. Its width is solved from the ring's coverage-weighted
  area and outer size. The thresholded area counted every edge pixel above
  about 17% coverage as whole, and read a 4 px stroke as 4.5.
- An outline that frames a fill moving with it is emitted as **one** shape
  with both. Emitted apart, an accurate stroke left a dark gap where the
  card's fill ran under its border.
- New corpus clip `outlined_ui.mp4` has outlines and a bordered card drawn
  by ffmpeg `drawbox`, not by VAC's own renderer. IoU 0.696 → **0.984**,
  residual 854 → **28.7**.

**Exact positions.** Chasing that clip's last error turned up a bug older
than v0.2. A blob's centroid is the mean of its pixel *indices*, half a
pixel short of the pixel centres they stand for. Whole-pixel rounding had
hidden that by luck: a rect edge at 39.5 rounded to the correct 40. Anchors
now use continuous coordinates, and positions are written to a tenth of a
pixel.

| | ink IoU | residual |
|---|---|---|
| `two_balls.vac` | 0.989 → **0.9997** | 1.79 → **0.02** |
| `late_entry.vac` | 0.994 → **1.0000** | 0.85 → **0.00** |
| `two_tone.vac` | 0.992 → **0.9999** | 68 → **0.58** |
| `vac_control.mp4` | 0.962 → 0.971 | 3.0 → **1.43** (46.6 dB) |
| `real_loader.gif` | 0.700 → **0.732** | 241 → **173** |
| `testsrc2.mp4` | 0.987 → 0.986 | 1341 → **1215** |

The first cut of sub-pixel positions parked shapes waiting off-canvas
exactly at the edge, which left half a pixel showing. The worst-frame gate
added in v1.3 caught it at once: `slide_in_card`'s worst frame fell to 0.00.
Shapes now park a full pixel clear, and that worst frame is the best it has
been (0.88 → 0.93).

## v1.5 — A third way to see, and a leaner search

`Segmentation::Regions` grows a region by colour similarity between
neighbours. That chains across any transition gradual enough. On the
pendulum, the grey bob and the green force arrow drawn across it fused into
one tall blob of mixed colour.

`Segmentation::Edges` measures edge strength as the colour change across a
two-pixel span. A pixel on a strong edge may join only other edge pixels,
never a flat region, so a thin line keeps its own identity. A thin region is
absorbed into a solid one only if it *wraps* it, meaning their shared border
is at least 40% of the thin region's area. An anti-aliased fringe wraps its
shape; a line drawn across a shape doesn't.

It isn't a replacement. It's the third candidate in the measured search, and
it wins where it should:

| clip | chosen | ink IoU | residual |
|---|---|---|---|
| `real_double_pendulum.gif` | edges + border | 0.437 → **0.492** | 760 → **697** |
| `real_newtons_cradle.gif` | edges + corners | 0.642 → 0.640 | 520 → **455** |

It loses on the clip it was written for. The pendulum's bob is still cut in
two by the rod and arrow drawn over it, which is an occlusion problem rather
than a segmentation one, and the search correctly keeps `regions` there.

**The search got cheaper as it got wider.** The static layer depends only on
the background rule, so decompiling now splits into `prepare` (layer, strip,
ground median), done once per background, and `finish` (detect, track,
emit), which each segmentation candidate runs concurrently on the shared,
read-only frames. Measurement renders frame by frame instead of holding the
whole reconstruction. On the 220-frame double pendulum, peak memory went
2.2 GB → **1.0 GB**, and the corpus run went 90 s → 26 s.

## v1.6 — A raster plate, when vectors fail

Some scenery is not shapes: a photographed desk, a lit sphere's soft
falloff, a test card's noise. A cheap experiment showed how much that cost.
A raw image of a clip's static scene alone, with nothing moving on it, beat
VAC's entire reconstruction on most real clips: 36 dB against 26 dB on the
bouncing ball.

v1.6 adds `image(x, y, w, h, "data:image/jpeg;base64,...")` and uses it as a
**measured escape hatch, not a default**:

- It is only considered when the best all-vector reconstruction is under
  30 dB, meaning vectors measurably fail.
- The plate is the clip's median frame, JPEG-encoded at a quality the dial
  sets.
- Every pixel that matches the plate is stripped from the frames, so only
  true movers remain, and they're still written as code on top. The first
  version reused the static layer's mask instead. Its bleeding-core test
  read a texture as a growing shape, and a full-canvas rect got tracked over
  the plate.
- It is kept only if it reconstructs at least **twice as well** in the
  selection rule's currency (`ln` residual ratio + `ln` IoU ratio, which is
  3 dB at unchanged IoU). A PSNR-only bar threw out the pendulum's plate,
  which lifts its IoU by half.

| clip | vector | raster plate + vector movers |
|---|---|---|
| `real_bouncing_ball.gif` | 26.0 dB, IoU 0.898 | **40.9 dB**, IoU **0.988**, 2 shapes |
| `real_newtons_cradle.gif` | 21.5 dB, IoU 0.640, 60 KB | **29.4 dB**, IoU **0.919**, **56 KB** |
| `testsrc2.mp4` | 17.3 dB, 255 KB | **22.2 dB**, **94 KB** |
| `real_pendulum.gif` | 20.1 dB, IoU 0.435 | **22.2 dB**, IoU **0.626** |
| `textured.vac` (new fixture) | 15.3 dB | **45.7 dB**, plate + the ball |

Two clips get *better and smaller* at once: describing a photograph as flat
patches had cost more bytes than the JPEG does. Flat content never takes a
plate (motion graphics, the outlines clip, every flat fixture), and neither
does `shaded.vac`, whose gradients are already 39.5 dB as code. The
Mandelbrot zoom has no static scene to keep, so it stays vector.

## v1.7 — Every shape earns its place

Through v1.6 a shape stayed in the output once its track cleared the
tracker's gates. Nothing checked that drawing it actually helped. v1.0's
pruning only *estimated* a shape's worth (its area times its contrast against
one flat background), which says nothing over a raster plate or a gradient.

v1.7 measures it:

1. Each shape is taken out of the reconstruction in turn, topmost first.
2. Only the rectangle it touches is re-rendered without it, using a new
   renderer entry point, `Rig`, that renders a region and draws only the
   shapes that reach it.
3. The full measurement already holds every frame's raw tallies (squared
   error, ink intersection, ink union), so the clip's score without the
   shape follows *exactly* from those few small renders.
4. The shape stays only if the clip reconstructs better with it, in the same
   currency the measured search picks candidates by.
5. The result is re-measured in full and kept only if it measures at least
   as well.

The shapes it found were mostly worse than nothing: dashed-line fragments
painted where the source had gaps, GIF palette flicker tracked over a raster
plate, test-card noise squares in the wrong colour.

| clip | v1.6 | v1.7 |
|---|---|---|
| `real_double_pendulum.gif` | 19.70 dB, 454 shapes, 192 KB, 24.6% spurious ink | **20.64 dB**, 419 shapes, **166 KB**, **9.7%** |
| `real_pendulum.gif` | 22.18 dB, 41 shapes, 58 KB, 12.2% spurious | **22.60 dB**, 27 shapes, **49 KB**, **6.6%** |
| `testsrc2.mp4` | 22.17 dB, 110 shapes, 94 KB | **22.24 dB**, 64 shapes, **55 KB** |
| `mandelbrot.mp4` | 11.94 dB, 43 shapes, 21 KB | 11.95 dB, 33 shapes, **17 KB** |

Nothing on the corpus got worse in the selection rule's currency. One clip
trades: `real_pendulum`'s ink IoU dips 0.626 → 0.622 while its residual falls
9%. A second pass was measured to remove nothing more, so there is one.

`vac::audit(frames, doc)` runs the audit on any document, including an older
version's output or a hand-edited source. `--no-audit` turns it off, to
measure what it changed. The test injects a stray blob and an exact duplicate
of a real ball into a perfect reconstruction. The audit must remove exactly
those two and leave the rest untouched.

## v1.8 — A mover no flat fill describes keeps its own pixels

A flat fill is the right description of a flat shape and the wrong one of a
shaded ball, a chrome sphere or a textured tile. v1.6 gave static scenery an
escape hatch, the raster plate. v1.8 gives one to things that move.

For a shape that only moves (translation, fades), VAC captures the source's
own pixels in a window that follows the shape's keyframes, as a per-pixel
median over every frame it is on screen. Transient occluders and noise drop
out. A pixel belongs to the sprite where the source differs from the scene
*without* the shape in most of those frames. The sprite is a PNG `image()`
with that alpha. It takes the shape's place and keeps its keyframes, so the
motion stays code and only the appearance becomes pixels.

Three measured rules decide, and each one came from a failure:

1. **The flat fill must fail on the shape's interior**: under 30 dB, two
   pixels in from every edge. That's the plate's bar, applied to appearance.
   Judged on the whole footprint instead, `motion_graphics` traded a flat
   drawbox square, keyframed a pixel off, for a PNG of its own blurred
   edges. With thin shapes allowed, `outlined.vac`'s ring (radius 49.8 for 50)
   gave up 5.7 dB to a PNG copy of itself. Both are geometry errors, which
   are the vector fit's to fix, recorded as frontier item 3 and fixed in v1.9.
2. **It must reconstruct the shape's own footprint at least twice as well.**
3. **It must pay for its bytes**: about 0.04 dB of clip PSNR per KB at
   quality 50, scaled by the dial. Without this rule the Mandelbrot zoom
   took 162 KB of PNG for 0.7 dB.

| clip | v1.7 | v1.8 |
|---|---|---|
| `real_bouncing_ball.gif` | 40.86 dB | **44.05 dB**, one sprite (the ball), +1.1 KB |
| `sprite.vac` (new fixture: a textured tile), hermetic | 26.3 dB as a flat rect | **36.5 dB** |
| `sprite.vac` through MP4 | 26.0 dB | **37.2 dB** |
| `real_double_pendulum.gif` | 20.64 dB | 20.87 dB, 6 sprites |
| `real_newtons_cradle.gif` | 29.36 dB | 29.39 dB, 1 sprite |

Every other clip and fixture is unchanged, and flat content never takes a
sprite. `--no-sprites` turns the pass off, and `vac::sprites(frames, doc,
quality)` runs it on any document.

The same slice closed a test gap. Twelve of the hermetic tier's twenty
recorded floors were never asserted by any test, so those fixtures could
regress silently. One test now walks every fixture in `examples/` and
requires a recorded row for each. With the keyframe tolerance broken tenfold
it fails on `grow.vac`, which no test checked before.

## v1.9 — The fit is whatever reconstructs best

Every shape's geometry is *estimated* from its detected blob: a radius from
its coverage, a position from its centroid, a stroke width from a ring's
thickness. Through v1.8 that estimate was final. On a thin, high-contrast
stroke a fifth of a pixel is worth decibels. `outlined.vac`'s 3 px ring came
back with radius 49.8 instead of 50 and gave up 5.7 dB, which v1.8 had to
refuse to paper over with a sprite.

v1.9 measures the numbers themselves. Each shape's size, position offset and
stroke width are nudged by coordinate descent, in steps of 0.5, then 0.2,
then 0.1 px (the precision positions are written at). Each trial is scored
the way the audit scores a shape: only the shape's footprint is re-rendered,
using a `Tweak` the renderer applies without rebuilding anything, against
the full measurement's per-frame tallies. The search scores trials on a
sample of 12 frames. A winner must also win on every frame before it is
written back, and the result is re-measured in full. Nothing is classified;
the numbers simply move to where the pixels agree best, and they stay
readable numbers.

| | v1.8 | v1.9 |
|---|---|---|
| `outlined.vac` (hermetic) | 40.57 dB, IoU 0.988, ring r = 49.8 | **47.80 dB**, IoU **1.000**, r = **50.0** |
| `spin.vac` (hermetic) | 34.4 dB, IoU 0.927 | **47.6 dB**, IoU **0.993** |
| `shaded.vac` (hermetic) | 39.6 dB | **51.4 dB** |
| `triangle.vac` (hermetic) | 35.8 dB, IoU 0.975 | **39.6 dB**, IoU 0.986 |
| `real_loader.gif` | 25.75 dB, IoU 0.732 | **28.32 dB**, IoU **0.864** |
| `real_double_pendulum.gif` | 20.87 dB, IoU 0.505 | **21.46 dB**, IoU **0.594** |
| `motion_graphics.mp4` | 39.46 dB | **41.76 dB** |
| `real_newtons_cradle.gif` | 29.39 dB | 29.95 dB |
| `real_pendulum.gif` | 22.60 dB, IoU 0.622 | 22.88 dB, IoU 0.640 |

Every corpus clip with room improved; the rest were already above the 45 dB
footprint at which refinement doesn't bother. The test knocks three numbers
off a perfect reconstruction (the ring's radius, the moving frame's
keyframes, the card border's width), and refinement must put each back to
within 0.1 px from the pixels alone. `vac::refine(frames, doc)` runs it on
any document, and `--no-refine` turns it off.

## v1.10 — A camera

A clip whose whole picture moves together is one image seen through a
moving camera. Through v1.9 VAC had no way to say so. The static layer
found nothing static, and the Mandelbrot zoom came back as 33 flat shapes
at 12 dB, although frame 0, magnified, explains every later frame at
23–29 dB.

v1.10 measures the camera instead of classifying the clip:

1. **Track.** Each frame's *view* of the scene (where each scene point
   appears in it, `p = scale · q + offset`) is found by a coarse-to-fine
   pattern search over a three-level pyramid. The error is robust, capped
   per pixel, so a mover can't drag the camera with it.
2. **Mosaic.** Every frame is warped back by its view, and the per-pixel
   median of all of them is the scene: a mosaic, larger than the frame when
   the camera pans.
3. **Re-track.** Frame 0 carries its own movers into the first estimate. On
   the new `camera.vac` fixture a crossing ball dragged the zoom 1.5 px off
   by the end. The median mosaic has no ball, so every view is re-measured
   against it, which leaves 0.18 px.
4. **Write.** The mosaic becomes one `image()`, and the views become
   `position()` + `scale()` keyframes, simplified until no canvas corner is
   off by more than a quarter pixel. Movers are found against the warped
   mosaic and stay code on top.

```vac
let plate = image(0, 0, 480, 320, "data:image/jpeg;base64,...")
animate plate {
  0ms    -> position(-0.85, -0.92), scale(0.9995)
  1966ms -> position(-129.47, -75.28), scale(1.492)
  easing: linear
}
let shape_0 = ellipse(80, 250, 20, 20)
```

That is `camera.vac` decompiled: its straight 1.5× zoom comes back as two
keyframes, and the ball as the shape it is.

Two measured gates come before the usual "twice as good as the vectors"
bar. The camera has to move the canvas by 2 px or more; a dozen-frame probe
settles that, so a still clip pays well under a second. And it has to
explain at least 20% of the picture better than a still camera would. On
`sprite.vac` the tracker happily follows the lone textured tile over its
flat background, but only 3% of the picture agrees, and the tile stays a
sprite.

| clip | v1.9 | v1.10 |
|---|---|---|
| `mandelbrot.mp4` | 12.02 dB, ink IoU 0.770, 33 shapes | **26.37 dB**, IoU **0.990**, one image + 16 camera keyframes |
| `camera.vac` (new fixture), hermetic | 12.9 dB as vectors | **48.32 dB**, IoU **1.000** |
| `camera.vac` through MP4 | 12.9 dB vectors, 22.4 dB still plate | **43.19 dB** |

Every other clip is unchanged. `--plate camera` forces a camera, and
`--plate raster` a still plate.

### v1.10.1 — On photographs

Two new corpus clips put the camera on photographic content. They're made
with ffmpeg's `zoompan` from two of the AI-generated stills in the PAC
matrices; the commands are in `docs/v0.4-capability-matrix.md`.

| clip | camera | still plate | vectors |
|---|---|---|---|
| `kenburns_watercolor.mp4` (zoom 1 → 1.4) | **30.29 dB**, 63 KB, 2 shapes | 16.76 dB, 712 KB | 13.45 dB, 845 KB, 710 shapes |
| `pan_portrait.mp4` (slow pan) | **42.47 dB**, 32 KB, 1 shape | 20.39 dB | 25.60 dB, 147 KB |

A Ken Burns slideshow comes back as one JPEG and a couple of keyframes, at
less than a tenth of the bytes the vectors spent failing to describe it.

The Ken Burns zoom also showed where the mosaic was losing detail. A median
over every frame let the wide, blurry first frames outvote the sharp
zoomed-in ones. Each mosaic point is now read only from the frames that
see it most magnified (scale within 10% of the largest), which is
+0.3 dB for free. A *denser* mosaic would keep more of a zoom's late detail,
but it measured a poor trade at the default quality: +0.8 dB for +54% bytes
on the Ken Burns clip, +0.2 dB for twice the bytes on the Mandelbrot. So it
stays at one mosaic pixel per frame pixel.

## v1.11 — Motion, measured from the pixels

The tracker keyframes a shape from its blob's centroid and, for a turning
shape, its second moments. Both estimates drift whenever the blob is lit
unevenly, crossed by something, or merged with a neighbour for a frame.
v1.9's refinement can move a whole shape, but it can't fix one keyframe.

v1.11 registers each mover's own pixels across its lifetime:

1. **Template.** Take the shape's pixels at the frame its vector shape
   matches best: where the source differs from the scene without it, and the
   shape itself paints (give or take 2 px).
2. **Register.** Against every other frame it's on screen, find the rigid
   motion that best maps those pixels there, under the camera's robust
   error. That's a turn and a translation, with the turn only for elongated
   or already-turning shapes. The search is a small grid about the shape's
   keyframed placement and its registered neighbours' heading, then a pattern
   search to a fiftieth of a pixel.
3. **Keyframe.** Simplify the registered track into keyframes again.
4. **Choose.** Score two candidates the way the audit scores shapes: the
   registered motion sampled at the shape's own keyframe times (no new
   bytes), and at the frames the simplification keeps. A candidate is kept
   only if the clip reconstructs better and the gain pays for any keyframes
   it adds, at the per-byte price sprites pay. Below the default quality the
   dial only ever takes keyframes away, so there motion may correct them but
   not add any. Without that rule, `spin.vac` came out larger at quality 0
   than at quality 25.

| | v1.10 | v1.11 |
|---|---|---|
| `real_bouncing_ball.gif` | 44.36 dB | **46.37 dB**, +83 bytes |
| `real_double_pendulum.gif` | 21.46 dB, IoU 0.594 | **22.04 dB**, IoU **0.621** |
| `crossing.vac` (hermetic) | IoU 0.9794, residual 1.03 | IoU **0.9999**, residual **0.00** |
| `real_pendulum.gif` | 22.88 dB, IoU 0.640 | 22.93 dB, IoU 0.643 |

On the ball, the gain comes from *where* the keyframes fall on the precise
trajectory, not from sampling the old times more precisely. Those two
candidates measured +0.18 and +0.005. The test knocks one keyframe off a
perfect reconstruction (a ball's position in `two_balls.vac`, a swinging
rod's turn and position in `spin.vac`), and the motion pass has to
re-measure it from the pixels. `vac::motion(frames, doc, quality)` runs it on
any document, and `--no-motion` turns it off.

The measured search also got faster with identical output. The two
background rules are now prepared side by side, all six candidates run
concurrently, and the still and camera plates are tried in parallel. The
corpus run went from about 145 s to 104 s (the Ken Burns clip 42 → 26 s),
and every clip's output is byte-for-byte what the baseline recorded.

## v1.12 — Shots

Real video is cut together from shots, and through v1.11 VAC decompiled
every clip as one. The static layer looked for a scene that held across the
cut, and tracks smeared from one shot into the next. Three corpus clips
spliced together (`cuts_three_shots.mp4`, the commands are in
`docs/v0.4-capability-matrix.md`) came back at 22.3 dB as 104 shapes, with
one frame scoring ink IoU 0.000.

v1.12 finds the cuts and lets each shot be its own picture:

1. **Candidates.** A frame that differs from the one before it at least ten
   times more than the clip's frames typically do (and by at least 20
   levels RMS) is a cut candidate. No shot is shorter than 6 frames, so a
   flash isn't a shot.
2. **Shots.** Each shot is decompiled on its own by the whole pipeline: its
   own measured search, plate or camera, audit, refinement, motion and
   sprites. The shots run in parallel with each other and with a
   single-scene decompile of the whole clip.
3. **Measure.** The shots become the scenes of one document. Each scene is
   exactly as long as its shot's frames: `ceil(frames × 1000 / fps)` ms,
   because the renderer draws `floor(duration × fps / 1000)` frames. The
   split is kept only if it reconstructs better than the clip as one shot.

```vac
scene shot_1 {                      scene shot_2 {
  duration 1500ms                     duration 1500ms
  let bg = rect(0, 0, 480, 320)       let plate = image(-1, -1, 552, 322, "…")
  let static_0 = rect(211, …)         animate plate { … }   // the camera pans
  …                                 }
}                                   scene shot_3 { … the sliding card … }
```

| clip | one scene | shots |
|---|---|---|
| `cuts_three_shots.mp4` | 22.30 dB, IoU 0.746, 104 shapes, 56 KB | **29.17 dB**, IoU **0.956**, 8 shapes, **28 KB** |
| `shots.vac` (new fixture, hermetic) | — | **51.55 dB**, three scenes, exactly their shapes |

The pan shot's mosaic is 552 px wide because its camera pans. The middle
shot became a camera plate while the other two stayed vector shapes, each
shot choosing for itself. No other corpus clip has a cut candidate, so none
changed or slowed down. The test cuts between three unrelated pictures and
requires three scenes, each as long as its shot and holding exactly its own
shapes; clips without a cut must stay one scene. `--no-shots` decompiles
the clip as one scene.

## v1.13 — Fades

A fade blends the whole picture toward black or white, and through v1.12 it
broke everything downstream of it. The sliding card, faded out over its
last second, scored 24.4 dB instead of 44.6. The photo pan, faded in, lost
its camera, because the camera aligns to frame 0 and frame 0 was black, and
came back as 115 shapes.

A fade is `frame = α · picture + (1 − α) · colour`, so v1.13 measures α,
takes the fade out, decompiles the picture, and draws the fade back:

1. **Read the fade, two ways.** The first is each frame's mean colour,
   projected onto the line toward black or white. The second chains each
   frame's median per-pixel ratio to the frame before. The mean reading is
   fooled by a card sliding in (an 11% "fade"). The chain is exact over a
   still background but drifts under a moving camera. Frames that are one
   flat black or white are the fade at its deepest: they split the chain into
   stretches that each fade toward their own picture. Both readings are
   tried.
2. **Take it out.** `picture = (frame − (1 − α) · colour) / α`. A frame too
   dark to recover borrows its nearest neighbour's picture.
3. **Decompile the picture** with everything: shots, so a fade through black
   reveals the cut it was hiding, and the camera, plates and sprites.
4. **Fit the fade to what was decompiled.** With the reconstruction `P`,
   the source `F` and the colour `C`, each frame's best opacity has a
   closed form: `o = Σ (F − P)·(C − P) / Σ |C − P|²`. The reading only has
   to be good enough to decompile the picture; the opacity is then measured
   against the actual reconstruction. Fitting took the pan from 41.1 dB to
   42.6 dB, and the fade through black from 41.9 to 42.2.
5. **Draw it on top.** Each scene gets a canvas-sized rect in the fade
   colour, its `opacity()` keyframed and simplified like any other channel.
   The result is kept only if it beats decompiling the clip as it is.

| clip | v1.12 | v1.13 |
|---|---|---|
| sliding card, fading out | 24.36 dB | **45.37 dB** (the unfaded clip: 44.63) |
| photo pan, fading in | 26.23 dB, 115 shapes | **42.62 dB**, a camera and a fade (the unfaded clip: 42.47) |
| `fade_through_black.mp4` (new corpus clip) | 25.88 dB, 50 shapes | **42.22 dB**, two scenes, 5 shapes |
| `fade_through.vac` (new fixture), hermetic | — | **49.38 dB**, two scenes, each faded |

```vac
scene shot_1 {
  …
  let fade = rect(0, 0, 480, 320)
  fade.fill = #000000
  animate fade {
    0ms -> opacity(0)
    1s -> opacity(0)
    1466ms -> opacity(0.996)
    1500ms -> opacity(1)
  }
}
```

The faded clips even come back slightly better than their unfaded originals
did (45.37 against 44.63, and 42.62 against 42.47). A single shape fading on
its own (`fade.vac`) is not mistaken for a fade of the picture. `--no-fades`
turns the pass off.

## v1.14 — HD, searched small

Every candidate of the measured search decompiles and measures the whole
clip, and each holds its own copy of it. That's fine at the corpus's sizes
(the largest is 459k pixels), but HD footage paid for it. Three seconds of a
720p camera pan took 64 s and 2.4 GB.

v1.14 decides small and decompiles full:

- **Above 500k pixels the search runs on a smaller copy of the clip**,
  shrunk by a whole factor (2 for 720p, 3 for 1080p), box-filtered, with the
  same timing. Only its decision (segmentation, background rule, plate) is
  replayed at full size, as a single candidate. The decision is still
  measured, just on the smaller clip; audit, refinement, motion and sprites
  then run at full size as before.
- **Camera tracking stops at a working size of about 700 px.** A view
  doesn't depend on resolution, and the pattern search reaches a fiftieth of
  a level pixel.
- **The mosaic samples the 8-bit frames directly** instead of holding float
  copies of 45 of them (11 MB each at 720p).
- **The fade pass is gated on the frame means.** The chained reading drifts
  under a moving camera and had read the pan as a 15% fade, which cost a
  whole second decompile before measurement threw it out.

| 3 s 720p camera pan | v1.13 | v1.14 |
|---|---|---|
| time | 64 s | **33 s** |
| peak memory | 2.44 GB | **1.66 GB** |
| reconstruction | 44.28 dB | **44.74 dB** |

The new `hd_camera.vac` fixture (the camera fixture at 960×540) comes back
from the half-size decision with the camera and the ball exactly placed, at
48.1 dB. Forcing the replayed decision to vectors fails its test, so the
test really exercises the small search. Every corpus clip is below the
threshold and byte-for-byte unchanged. Long HD clips are still open: they
need decompiling shot by shot without holding every frame at once.

## v1.15 — A camera that rolls

The v1.10 camera could zoom and pan but not turn. A photo rolling 26° over
three seconds (`roll_watercolor.mp4`, made with ffmpeg's `rotate`) fell back
to a still plate and 588 shapes at 18.5 dB.

A view is now `scale · R(angle) · q + offset`:

- **Tracking** tries turns about the frame centre alongside zooms, stepping
  the angle the way it steps the scale.
- **Writing** has a subtlety. The renderer turns an image about its box's
  centre and then anchors it at its top-left corner, so the plate's
  `position()` depends on the angle. And because the renderer interpolates
  position, scale and rotation independently, the keyframe simplifier now
  measures its error on the plate's own `position`/`scale`/`rotate` values,
  converted back into the view the renderer will actually draw, not on the
  camera's internal parameters.
- **A camera that doesn't roll stays rotation-free.** With turns searched,
  a pure zoom picks up a few hundredths of a degree of noise. If no view
  turns by a quarter degree, the angles are zeroed and the views
  re-measured without turns, so `camera.vac` is still written without
  `rotate()`.

| | v1.14 | v1.15 |
|---|---|---|
| `roll_watercolor.mp4` (new corpus clip) | 18.49 dB, still plate, 588 shapes, 615 KB | **33.25 dB**, one camera plate, **73 KB** |
| `camera_roll.vac` (new fixture), hermetic | — | **47.31 dB**, IoU 1.000, every keyframe within 0.5° of the source |

The other camera clips moved by hundredths of a dB, since every camera is
now tracked with turns available before the angle is judged.

## v1.16 — Long clips, a segment at a time

Through v1.15 every frame of a clip was decoded and held at once, and the
measured search held copies of them. A minute of 720p is about 6.6 GB of
raw frames before VAC does anything.

v1.16 streams any MP4 over a budget of 150M pixels in total (5.4 s of 720p,
3 s of 1080p):

1. **One pass to look.** Frames are read off ffmpeg's pipe one at a time
   and dropped. The pass keeps only the frame count, each frame's step from
   the one before (where the cuts are, by exactly the numbers the in-memory
   cut detector uses), and the metric's ink reference, i.e. the whole clip's
   modal colour, accumulated frame by frame.
2. **Segments.** The cuts split the clip into shots, and any shot too long
   for the budget is split into equal slices.
3. **One pass to decompile.** The frames are streamed again. Each segment
   is decompiled by the whole pipeline (fades, camera, audit, everything)
   as soon as its last frame arrives, measured against the clip's ink
   reference, and dropped.

| 12 s of 720p: a Ken Burns zoom, a pan and a roll, cut together | |
|---|---|
| held whole, as v1.15 would | had not finished after **25 minutes** |
| v1.16, a segment at a time | **188 s**, peak **1.75 GB**, 38.5 dB, one camera plate per shot |

Two fixes came with it. `load_mp4` buffered the whole decoded clip and then
copied every frame out of it, so loading alone needed twice the clip's
memory; frames now come straight off the pipe. And the single-scene check
that guards a split against false cuts was the slow part of the in-memory
attempt, because it decompiles a multi-picture clip as one shot. It now
runs only up to 3 cuts and 300 frames, which is all of the corpus; past
that, the deliberately conservative cut test (ten times the clip's
typical frame step) is trusted.

The test compiles `shots.vac` to MP4 and decompiles it twice: held whole,
and with a budget that streams it but fits each shot. The two must agree
exactly (same scenes, same source bytes, same score). Measuring segments
against their own ink reference instead of the clip's fails it. One cost
remains: a forced split inside a long shot restarts any movement across the
boundary. In a deliberately tiny budget test (1-second segments) the
sliding card lost 9 dB, because each slice's static layer read the slow
card as scenery. At the real budget, forced splits only start past 5.4 s
of continuous 720p shot.

## v1.17 — Dissolves

Cuts and fades were handled; dissolves, the third standard transition, were
not. A dissolve is a cut spread over frames. No single step from one frame to
the next looks like a cut, so through v1.16 a two-photo slideshow with a
one-second dissolve came back as one shot: a single camera trying to follow
two photographs, at 19.1 dB and 371 shapes.

v1.17 looks for what a dissolve is, at window lengths from 8 to 64 frames:

1. **A cut across it.** Frames a window apart differ at least three times
   more than frames that far apart usually do. That's lower than a cut's
   10×, because under a camera move frames 32 apart already differ a lot;
   the slideshow's dissolve stood out 6×. No single step inside may be a
   cut.
2. **A blend inside it.** Every frame between is `(1 − β)·before + β·after`
   to within 15% of the two pictures' difference, with `β` rising from near
   0 to near 1.
3. **Everything at once.** In the frames midway through, at least 60% of
   the changing pixels must have changed by the frame's own share `β`. A
   dissolve moves every pixel together; an edge sweeping across moves each
   pixel all at once at its own moment. Without this test, a flat card
   sliding over a flat background (`late_entry.vac`) passed the blend test
   in the mean and was cut into three dissolves.

Of overlapping candidates, the longest that fits wins, since a dissolve's
extent is its longest window and shorter windows inside it fit too. The
dissolve becomes a scene of its own between the two shots: the frame before
it and the frame after it as pictures, the second drawn over the first with
its opacity fitted to every frame by least squares and simplified like any
opacity channel. The split is still kept only if it measures better than the
clip as one.

| clip | v1.16 | v1.17 |
|---|---|---|
| `slideshow_dissolve.mp4` (new corpus clip) | 19.06 dB, 371 shapes, 438 KB | **29.09 dB**, three scenes, 6 shapes, **166 KB** |
| `dissolve.vac` (new fixture), hermetic | — | **47.78 dB**, three scenes, opacity 0.03 → 0.97 |

Every other corpus clip is unchanged. One thing is left: the dissolve's
pictures are frozen at its ends, so motion during the dissolve isn't
followed. The slideshow's worst frame (21.1 dB) is mid-dissolve, where the
first photo is still zooming.

## v1.18 — Cameras carried through dissolves

v1.17 wrote a dissolve as its two edge frames crossfading, frozen, so a
slideshow's photos stopped moving for the length of the dissolve. Now each
side can be its neighbouring shot's camera plate, the same mosaic with the
camera extrapolated linearly through the dissolve. The first photo keeps
zooming while the second, already panning, fades in over it. The crossfade
is fitted against the two sides as rendered.

```vac
scene shot_2 {                        // the dissolve
  duration 934ms
  let plate = image(0, 0, 480, 320, "…")          // the first photo's mosaic
  animate plate {
    0ms   -> position(-33.27, -22.42), scale(1.1407)
    900ms -> position(-50.46, -37.6), scale(1.2119)   // still zooming
  }
  let incoming = image(0, -1, 560, 322, "…")      // the second's mosaic
  animate incoming {
    0ms   -> opacity(0.063), position(32.94, -1), scale(1)
    900ms -> opacity(0.969), position(1.17, -1), scale(1)  // already panning
  }
}
```

The gain is small, 29.09 → **29.16 dB**, because a carried mosaic knows only
what its own shot saw. The second photo's pan had not yet shown the 33 px
strip the dissolve still displays, so that strip comes from nothing. Since
neither representation always wins, the decompiler builds both, carried and
frozen, and keeps whichever reconstructs the dissolve's frames better. The
remaining gap (the dissolve frames sit near 22 dB between shots at 30 and
42) needs each shot's mosaic to learn from the dissolve's frames too, by
taking the other picture out of them. That's recorded as frontier item 4.

### A still frame is a scene, not nothing

A robustness pass fed VAC inputs nobody had tried: one frame, two frames,
odd sizes (334×212), an 8×8 clip, three seconds of one flat colour. None of
them crashed. But a single frame of ffmpeg's test card came back as a lone
background at **3.9 dB**. Every track needs three frames and the static layer
needed eight, so a clip of a frame or two had nothing to describe it with.
In a clip that short nothing can be a mover, and its frames are the scene,
so it's now layered like any other. The test card frame comes back at
**37.8 dB**, and one frame of `outlined.vac` comes back as its shapes.

## v1.19 — A mosaic fitted through the renderer

The camera clips were the corpus's middle tier. The Ken Burns zoom sat at
30.3 dB and the rolling watercolour at 33.3. Raising the quality dial barely
moved them: at quality 100 the Ken Burns clip reached 31.2 dB for twice the
bytes. So the limit was the plate, not its JPEG. A probe of the worst frame
said where:

| test on Ken Burns frame 84 | PSNR |
|---|---|
| reconstruction as written | 29.3 dB |
| both averaged over 4×4 blocks (colour only) | **40.0 dB** |
| best global sub-pixel shift of the reconstruction | 29.3 dB (no shift helps) |
| reconstruction sharpened | 30.0 dB |
| reconstruction blurred | 27.5 dB |

The colours were right and the registration was right. The detail was
missing. The mosaic was the per-pixel median of every frame warped back,
so each sample was blurred once by the warp on the way in and again by the
renderer's bilinear reading on the way out. Drawing images bicubically
instead was tried and bought 0.3 dB.

v1.19 fits the mosaic to the frames *through* the renderer's reading of
it, by iterative back-projection. Each round draws every sampled frame from
the mosaic, then pushes each pixel's error back onto the mosaic pixels it
was read from, in proportion to how much each contributed. Six rounds. Two
things keep it honest:

- **The fit decides the detail; the median keeps the colour.** A fade's
  restored frames are a few percent off in brightness. A median ignores
  that and a fit absorbs it. Plain back-projection cost the fade clip
  0.45 dB. Keeping only the fit's change at scales finer than a 2-pixel
  blur turned that into a 0.15 dB gain. Frames that disagree with the rest
  are also weighted down by the square of how much more they disagree.
- **It must pay for its bytes.** A sharper image costs more JPEG. Both
  mosaics are encoded as they would be written, decoded, and read through
  the camera onto eight probe frames, chosen where possible from frames the
  fit never saw. The fitted one is kept only if it buys at least the
  sprite price, about 0.04 dB per KB at the default quality, scaled by the
  dial.

| clip | v1.18 | v1.19 |
|---|---|---|
| `roll_watercolor.mp4` | 33.25 dB, 73 KB | **35.85 dB**, 82 KB |
| `kenburns_watercolor.mp4` | 30.29 dB, 63 KB | **32.39 dB**, 70 KB |
| `slideshow_dissolve.mp4` | 29.16 dB, 163 KB | **29.72 dB**, 175 KB |
| `pan_portrait.mp4` | 42.51 dB | 42.54 dB |
| `mandelbrot.mp4`, `fade_through_black.mp4` | — | unchanged: the median mosaic kept |

At the default quality the Ken Burns clip now beats what quality 100 gave
before (32.4 dB at 70 KB, against 31.2 dB at 121 KB). The Mandelbrot zoom
measured no gain and kept its median mosaic. Its error is content changing
from frame to frame, not blur. A new frontier test zooms a fine procedural
texture 1.0 → 1.4 and requires the camera to come back above 33.5 dB:
fitted, 34.5; the median alone, 32.0.

One attempt failed and is recorded here. An "oracle" plate, the source
photograph itself under the exact `zoompan` camera, scored *lower* than
VAC's own decompile (26.3 against 30.3 dB). The clip's frames went through
4:2:0 chroma subsampling, and the photograph had not. A mosaic built from
the frames inherits what the frames are; that is what makes it the right
thing to fit.

## v1.20 — A slow mover is not scenery

`cuts_three_shots` is three corpus clips cut together. Each of them scores
33.9–44.6 dB alone, but together they scored 29.2. The first shot,
`outlined_ui` cut to its first 45 frames, came back at 24 dB.

The static layer (v0.8) separates scenery from movers with a per-pixel
temporal median. To a median, anything that covers a pixel for more than
half the shot is scenery. The outlined card slides at 100 px/s. In 1.5 s
the middle of its top and bottom edges is covered 85% of the time; in the
full 3 s clip, 43%. So in the short shot those edges were stripped from
every frame and written as two static bars. With its edges stripped, what
was left of the card came apart into fragments.

No per-pixel statistic can tell those pixels from a painted line: they hold
the edge's colour in most of the frames. What tells them apart is what the
reconstruction makes of them. When the winning candidate has static
shapes, its segmentation and background are now decompiled once more
without the layer. That version is kept when it measures twice as good,
the bar a raster plate has to clear:

| | with the layer | without |
|---|---|---|
| `outlined_ui`, first 45 frames, at the search | 22.15 dB, ink IoU 0.811 | **33.54 dB**, ink IoU 0.985 |
| `real_loader`, at the search | 25.75 dB, ink IoU 0.732 | 26.14 dB, ink IoU 0.707 |

The bar matters. Candidates are compared before the passes that improve
the winner, and a near tie there says nothing about the result. Taken on a
0.058 margin, the loader's layerless version finished worse: ink IoU 0.864
→ 0.806, with 43% more bytes. The card wins by 2.8.

| clip | v1.19 | v1.20 |
|---|---|---|
| `cuts_three_shots.mp4` | 29.17 dB, ink IoU 0.956 | **37.70 dB**, ink IoU **0.988** |

Every other corpus clip is unchanged. A new frontier test slides an
outlined card past a static square over 45 frames. With the layer assumed,
it came back as three static bars and eight fragments of the card's sides.
Measured against the layer's absence, it comes back as the card and the
square, exactly.

The v1.16 note on forced slices blamed the same thing: each 1-second
slice's static layer reading the slow card as scenery. Re-measured now,
that's wrong. With 1-second slices `slide_in_card` still loses 9.4 dB
(35.24 against 44.63), and v1.20 doesn't change it. The slice with the card
at rest is exact (59 dB). The loss is in the first slice, where the card
enters from off-screen: its first keyframe puts it at x = −145.6, partly
visible, where the source has it at −180, hidden (24.9 dB at frame 0).

## v1.21 — Shapes that grow one way

`scale()` was uniform, so a shape that grows along one axis had no
description. That covers a progress bar filling, a bar chart rising, an
underline drawing out. A progress bar drawn by ffmpeg came back as three
wrong things at 29.6 dB: a static stub for the part covered longest, a
uniformly scaled rect sliding right, and a sprite for its leading edge.

The format's `scale` now takes a value per axis. `scale(sx, sy)` stretches
about the shape's anchor, which for a rect is its top-left, so a bar keeps
its left edge as it fills. `scale(s)` is still written whenever the two
are equal, and every older file parses as before. A chart bar keeps its
bottom edge by moving its top up as it grows. Both channels are linear, so
two keyframes describe it exactly:

```vac
let shape_0 = rect(100, 273, 60, 140)          // the red bar, at its fullest
animate shape_0 {
  33ms -> position(100, 273), scale(1, 0.071)  // a sliver on the baseline
  1s   -> position(100, 140), scale(1)         // risen to 140 px
}
let shape_1 = rect(37.5, 40, 399.9, 16)        // the progress bar
animate shape_1 {
  33ms   -> position(37.6, 40), scale(0.025, 1)
  2500ms -> position(40.1, 40), scale(1)       // full at 2.5 s
}
```

The tracker measures a track's width and height on their own, each against
its own median, and writes a stretch when the two part for a sustained run.
Getting that rule honest took three corrections, each found on the corpus:

- **Against its own median, not the declared size.** The declared size
  comes from a stricter estimator, so measured against it, shapes of
  constant size read as a constant `scale(0.8, 1)` (testsrc2's squares).
- **By enough to be the shape.** The stretched axis must change by 8 px or
  more. testsrc2's 8 px squares wobble a pixel either side as they touch:
  a 25% aspect change that is the detector's noise, not growth.
- **Declared at its fullest.** A stretching shape is declared at the 90th
  percentile of its extent, so its scale reads as how full it is.

The corpus check also caught a regression in v1.20's static-layer
variant. Without the layer, the segmentations compete afresh: which one won
*with* the layer says nothing about which wins without it. v1.21's tracker
change tipped `cuts_three_shots`'s first shot to edges, which finishes at
32.7 dB without the layer where regions gives 34.1. When the layer is
dropped, every segmentation is now decompiled on the same layerless frames
and the best is kept. The clip is back to 37.70 dB.

| clip | v1.20 | v1.21 |
|---|---|---|
| `progress_bars.mp4` (new corpus clip) | 21.72 dB, ink IoU 0.839 | **39.90 dB**, ink IoU **0.995**, one rect per bar |
| `progress.vac` (new fixture), hermetic | — | **41.30 dB**, ink IoU 0.997 |
| `testsrc2.mp4` | 22.42 dB | 22.60 dB: its grey bar's arms extend and retract |
| `real_newtons_cradle.gif` | 29.95 dB | 30.03 dB |
| `real_double_pendulum.gif` | 22.04 dB, 174 KB | 22.06 dB, 185 KB |

Every other corpus clip is unchanged. One thing `progress_bars` shows that
the format can't say: the grey track the bar fills comes back as a rect
shrinking from the left, which is what stays visible, not a track under a
bar. Writing it as the source has it needs depth order, which the tracker
does not yet infer.

## v1.22 — Wipes and pushes

With cuts, fades and dissolves handled, I made a small matrix of the other
standard transitions with ffmpeg's `xfade`: the same two photos, a second
each side of a one-second transition.

| transition | v1.21 | v1.22 |
|---|---|---|
| crossfade | 36.88 dB, three scenes | unchanged |
| wipe (the second photo uncovered from the right) | **18.90 dB**, six scenes, 256 shapes | **36.97 dB**, three scenes |
| push (the second photo pushing the first off) | **19.48 dB**, six scenes, 469 shapes | **36.79 dB**, three scenes |
| circle opening | 29.89 dB | unchanged (not handled) |

Between two still pictures, every frame of a wipe or a push steps like a
cut, so the cut test fired all through one and chopped it into fragments.
v1.22 looks for a window whose ends are still and whose every frame between
is the second picture coming on along one axis. That's two models, in four
directions each:

- **A wipe.** Each pixel is one picture or the other, split at an edge.
  Each line's cost against either picture, summed by prefix sums, gives
  the best edge in every frame exactly.
- **A push.** Both pictures shift together. A push moves whole lines, so
  its offset is found by matching the lines' mean colours in one dimension,
  and the frame is checked at that offset.

Both are searched on thumbnails and re-measured to the pixel at full size.
A wipe is written as the incoming picture under a new transform,
`clip(x, y, w, h)`: a window in canvas coordinates outside which a shape
isn't drawn. A push is written as the two pictures side by side under
`position()` keyframes:

```vac
let incoming = image(0, 0, 480, 320, "…")    // the wipe
animate incoming {
  0ms   -> clip(464, 0, 16, 320)
  466ms -> clip(240, 0, 240, 320)
  933ms -> clip(16, 0, 464, 320)
}
```

Three gates came out of false positives the tests and the corpus caught:

- **It replaces the picture.** The two ends must differ decisively over
  half the frame. On a flat background a lone rect sliding *is*,
  geometrically, a push: the background doesn't care that it moved.
  `sliding_rect.vac` was taken for one.
- **Steadily.** No frame may take more than a third of the frame. A pan
  creeping a line a frame and then a cut jumps most of it at once
  (`cuts_three_shots`: 15 of 160 lines, then all 160).
- **Between still frames, in absolute terms.** The window's ends must step
  no more than three times the clip's typical step. Measured only against
  the window, a frame in the middle of a wipe was "still" (a 16 px strip is
  little beside two different pictures) and bounded a false push over it.

Keyframes are pinned at an eighth of the keyframe tolerance, as a camera
plate's are. ffmpeg's push steps in whole pixels, and simplified to a
straight line it lost 12 dB on every other frame.

| clip | v1.21 | v1.22 |
|---|---|---|
| `slideshow_wipe_push.mp4` (new corpus clip) | 20.28 dB, 477 shapes | **37.23 dB**, five scenes, 7 shapes |
| `sweep.vac` (new fixture), hermetic | — | **46.16 dB**, five scenes as written |

Every other corpus clip is unchanged. Two things are left. The pictures are
frozen at the sweep's ends, as v1.17's dissolve pictures were, so motion
during a sweep isn't followed. And each scene embeds its own copy of a
picture, so the new clip's three photos are written seven times (283 KB).

## v1.23 — A picture is written once

Through v1.22 every scene embedded its own copy of every picture it drew,
as a base64 blob in the middle of the code. The wipe-and-push slideshow
shows three photos and wrote seven: each still shot's plate, and both ends
of each transition, 283 KB. The open list on the results page put it
plainly: rasters are blobs in the source.

The writer now names each distinct picture and writes it once, after all
the scenes, and the scenes refer to it by name:

```vac
scene shot_2 {                                  // the wipe
  duration 967ms

  let plate = image(0, 0, 480, 320, picture_1)
  let incoming = image(0, 0, 480, 320, picture_2)

  animate incoming {
    0ms   -> clip(464, 0, 16, 320)
    933ms -> clip(16, 0, 464, 320)
  }
}

// The pictures the scenes draw, each written once.
asset picture_1 = "data:image/jpeg;base64,/9j/4AAQ…"
asset picture_2 = "data:image/jpeg;base64,/9j/4AAQ…"
```

The AST is unchanged. The writer factors identical pictures out, and the
parser resolves the names, wherever in the file the asset is declared, so
`parse(write(D)) == D` still holds and every older file reads as before.

Identical bytes are only half of it. In a real video a still shot's plate,
the median of its frames, and the frame beside a transition differ by the
codec's noise. So a transition between still shots now also tries drawing
the plates its neighbours already write, and keeps whichever buys more:
the fidelity on its own frames, plus every byte of picture it doesn't add,
at the price a sprite pays. A dissolve's carried camera plates get the same
credit.

| clip | v1.22 | v1.23 |
|---|---|---|
| `slideshow_wipe_push.mp4` | 37.23 dB, 283 KB | 37.23 dB, **132 KB** |
| `slideshow_dissolve.mp4` | 29.72 dB, 175 KB | 29.59 dB, **93 KB** |
| `sweep.vac` hermetic, and with ±3 levels of noise | 3 pictures written 3 and 7 times | 3 pictures written **once** each |

The dissolve trades 0.13 dB for 81 KB, 0.002 dB per KB against a price of
0.04. Clips with one picture grow by its `asset` line (81 bytes), and
`real_double_pendulum` moved 0.03 dB: its sprites price themselves by the
written file, which now writes pictures differently.
