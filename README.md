# VAC — Video As Code

**A purely algorithmic decompiler that reverse-engineers video into human-readable, structured code.**

> What Potrace did for images (bitmap → SVG), VAC does for video (pixels → structured code).

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

### What's still on the list

- **v0.4** — MP4 I/O via `ffmpeg-next` (removes the centisecond fps drift) + velocity-aware tracking (handles same-colour ID-swap).
- **v0.5** — `path()` primitive: Suzuki contour tracing → RDP → least-squares Bezier fitting. The original Phase 1 work. Hu moments earn their keep when shapes stop being circles.
- **v0.6+** — real-world video (codec-artifact-tolerant background estimation, scale/opacity inference, scene cuts).
