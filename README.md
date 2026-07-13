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

- **v0.5** — `path()` primitive: Suzuki contour tracing → RDP → least-squares Bezier fitting. The original Phase 1 work. Hu moments earn their keep when shapes stop being circles.
- **v0.6** — velocity-aware tracking (handles same-colour ID-swap), scene-cut detection (histogram comparison).
- **v0.7+** — real-world video (codec-artifact-tolerant background estimation, scale/opacity inference).

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
real pendulum fix is velocity-aware tracking (v0.6), which now has hard
numbers behind it. What the filter reliably buys is the UI /
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
