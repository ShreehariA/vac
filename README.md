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
