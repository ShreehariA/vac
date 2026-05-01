# VAC Research: Existing Projects, Formats, Tools & Landscape

> A comprehensive survey of projects, formats, standards, and tools related to the VAC (Video As Code) vision — a purely algorithmic programming language that compiles into video and decompiles video back into code.

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Landscape Map](#landscape-map)
3. [Code-to-Video Tools](#1-code-to-video-tools)
   - [Remotion](#remotion)
   - [Motion Canvas](#motion-canvas)
   - [Manim](#manim)
   - [Editly](#editly)
4. [Animation Formats & Standards](#2-animation-formats--standards)
   - [Lottie](#lottie)
   - [Rive](#rive)
   - [SVG + SMIL](#svg--smil)
5. [Scene Description Formats](#3-scene-description-formats)
   - [Universal Scene Description (USD)](#universal-scene-description-usd)
   - [glTF](#gltf)
6. [Algorithmic Tracing Tools](#4-algorithmic-tracing-tools)
   - [Potrace](#potrace)
7. [Creative Coding Frameworks](#5-creative-coding-frameworks)
   - [Processing / p5.js](#processing--p5js)
8. [Declarative Layout Libraries](#6-declarative-layout-libraries)
   - [Clay](#clay)
9. [Comparison Matrix](#comparison-matrix)
10. [Key Takeaways for VAC](#key-takeaways-for-vac)
11. [What VAC Can Learn From Each](#what-vac-can-learn-from-each)
12. [The Gap VAC Fills](#the-gap-vac-fills)
13. [Recommended Libraries & Tools to Build VAC](#recommended-libraries--tools-to-build-vac)

---

## Executive Summary

After researching the landscape, **no existing project does what VAC aims to do** — create a human-readable programming language that compiles to video AND decompiles video back to code using purely algorithmic methods.

The closest projects fall into distinct categories:
- **Code-to-video tools** (Remotion, Manim, Motion Canvas) — let you write code that outputs video, but have no decompiler and no custom format
- **Animation formats** (Lottie, Rive) — structured vector animation formats, but not human-readable languages and not designed for full video
- **Scene description formats** (USD, glTF) — interchange formats for 3D scenes, not video languages
- **Algorithmic tracing** (Potrace) — bitmap→vector conversion, directly relevant to VAC's decompiler

VAC sits in a genuinely unexplored space: **a bidirectional, human-readable video format with a compiler and decompiler, using purely algorithmic methods.**

---

## Landscape Map

```
                        Human-Readable Language
                               ▲
                               │
                    VAC ★      │     Processing/p5.js
                   (goal)      │     (creative coding,
                               │      no video output)
                               │
  Full Video ◄─────────────────┼──────────────────► Vector/Animation Only
                               │
         Editly                │     Lottie / Rive
    (JSON specs,               │     (structured format,
     FFmpeg render)            │      not human-readable)
                               │
         Remotion              │     SVG + SMIL
    (React→MP4,                │     (XML markup,
     no decompiler)            │      images not video)
                               │
                               ▼
                        Machine-Readable Format
```

---

## 1. Code-to-Video Tools

### Remotion

| Property | Detail |
|----------|--------|
| **What** | React-based framework for creating videos programmatically |
| **Language** | TypeScript/React |
| **GitHub** | github.com/remotion-dev/remotion |
| **Stars** | ~41,000 |
| **License** | Commercial ($25-500+/month for companies) |
| **Output** | MP4, WebM, GIF |

**How it works:** You write React components with a `useCurrentFrame()` hook. Each frame is a React render. Remotion renders every frame using headless Chromium, then stitches them into a video via FFmpeg.

```jsx
// Remotion example
export const MyVideo = () => {
  const frame = useCurrentFrame();
  const opacity = interpolate(frame, [0, 30], [0, 1]);
  return <div style={{ opacity }}>Hello World</div>;
};
```

**Relevance to VAC:**
- ✅ Proves "code → video" is a viable product (41k stars!)
- ✅ Shows the market wants programmatic video creation
- ❌ Not a custom language — uses React/TypeScript
- ❌ No decompiler (one-way only)
- ❌ No custom binary format — renders via browser + FFmpeg
- ❌ Browser rendering is slow (headless Chromium per frame)
- ❌ Commercial license limits adoption

**What VAC does differently:** Native language, bidirectional (compile + decompile), no browser dependency, purely algorithmic.

---

### Motion Canvas

| Property | Detail |
|----------|--------|
| **What** | TypeScript library for programmatic 2D animations with a real-time editor |
| **Language** | TypeScript |
| **GitHub** | github.com/motion-canvas/motion-canvas |
| **Stars** | ~18,400 |
| **License** | MIT |
| **Output** | Video frames (rendered in browser) |

**How it works:** Uses TypeScript generators to define animation sequences. Ships with a real-time preview editor that shows animations as you code. Specialized for informative/educational vector animations.

**Key Architecture:** Monorepo with packages: `2d`, `core`, `create`, `ui`, `vite-plugin`, `player`. Created by @aarthificial.

**Relevance to VAC:**
- ✅ Generator-based animation sequencing is elegant (similar to VAC's `animate` blocks)
- ✅ Real-time preview editor — VAC Studio should learn from this
- ✅ MIT license, open ecosystem
- ✅ Vector-first approach aligns with VAC's geometric primitives
- ❌ Still TypeScript, not a custom language
- ❌ No decompiler
- ❌ Browser-based rendering (canvas)

**What VAC does differently:** Custom language with grammar, native rendering (not browser), bidirectional decompilation.

---

### Manim

| Property | Detail |
|----------|--------|
| **What** | Python animation engine for explanatory math videos (3Blue1Brown community) |
| **Language** | Python |
| **GitHub** | github.com/ManimCommunity/manim |
| **Stars** | ~28,000+ (community edition) |
| **License** | MIT |
| **Output** | MP4 (via FFmpeg) |

**How it works:** Scene-based API where you define `construct()` methods with animations. Each `Scene` is rendered frame-by-frame using Cairo (2D) or OpenGL (3D).

```python
# Manim example
class Example(Scene):
    def construct(self):
        circle = Circle()
        self.play(Create(circle))
        self.play(circle.animate.shift(RIGHT * 2))
```

**Relevance to VAC:**
- ✅ Scene-based animation model is proven and intuitive
- ✅ Uses Cairo for rendering — VAC could use similar (tiny-skia, Cairo)
- ✅ Outputs via FFmpeg — same pipeline VAC plans
- ✅ Shows that "code → educational video" has massive demand
- ❌ Python library, not a standalone language
- ❌ No decompiler
- ❌ Focused on math/education, not general video

**What VAC does differently:** Standalone language (not embedded in Python), general-purpose video (not just math), bidirectional decompilation.

---

### Editly

| Property | Detail |
|----------|--------|
| **What** | Declarative, JSON-based non-linear video editor |
| **Language** | Node.js + FFmpeg |
| **GitHub** | github.com/mifi/editly |
| **Stars** | ~5,400 |
| **License** | MIT |
| **Output** | MP4 (via FFmpeg) |

**How it works:** You write a JSON/JSON5 spec describing clips, layers, transitions, and audio. Editly renders it via headless browser (for canvas/HTML layers) and FFmpeg.

**Layer types:** `video`, `audio`, `image`, `title`, `subtitle`, `canvas` (custom JS), `fabric` (Fabric.js), `gl` (GLSL shaders), `fill-color`, gradients.

**Key features:** Streaming editing (fast, low storage), gl-transitions, Ken Burns effect, audio normalization/ducking.

```json5
// Editly example
{
  outPath: './output.mp4',
  clips: [
    { layers: [{ type: 'title', text: 'Hello' }] },
    { layers: [{ type: 'image', path: './photo.jpg' }] }
  ]
}
```

**Relevance to VAC:**
- ✅ JSON spec → video is the closest to "declarative video description"
- ✅ Layer-based composition model
- ✅ Streaming editing concept (fast rendering without full video in memory)
- ❌ JSON is not human-friendly for complex animations
- ❌ No decompiler
- ❌ Limited to what FFmpeg filters can do

**What VAC does differently:** Purpose-built language (not JSON), fine-grained animation control, bidirectional decompilation, custom rendering engine.

---

## 2. Animation Formats & Standards

### Lottie

| Property | Detail |
|----------|--------|
| **What** | JSON-based vector animation format |
| **Created** | 2015 by Hernan Torrisi (Bodymovin plugin for After Effects) |
| **Developed by** | Airbnb (original renderers), LottieFiles (ecosystem), LAC (standardization since 2024) |
| **Format** | `.json` (Lottie), `.lottie` (dotLottie — zip container since 2020) |
| **Renderers** | lottie-web, lottie-ios, lottie-android, rLottie (Samsung, C++), ThorVG |
| **Adoption** | 280,000+ companies (Google, Disney, Duolingo, Uber, Netflix) |

**How it works:** Animations are authored in After Effects and exported via the Bodymovin plugin to JSON. The JSON uses 1-2 character keys for compact representation (not human-readable). Renderers parse the JSON and draw vector animations at runtime.

**Format structure:**
```json
{
  "v": "5.5.2",          // version
  "fr": 30,              // frame rate
  "ip": 0,               // in-point
  "op": 60,              // out-point
  "w": 512, "h": 512,    // dimensions
  "layers": [{
    "ty": 4,              // shape layer
    "shapes": [{
      "ty": "el",         // ellipse
      "p": { "a": 0, "k": [256, 256] },  // position
      "s": { "a": 0, "k": [200, 200] }   // size
    }]
  }]
}
```

**Relevance to VAC:**
- ✅ **The closest existing "structured video format"** — proves the concept works
- ✅ Massive ecosystem adoption shows demand for structured animation
- ✅ Vector-based, scalable, small file size — same goals as VAC
- ✅ Multi-platform renderers exist (web, iOS, Android, C++)
- ✅ rLottie (Samsung) and ThorVG are C/C++ renderers VAC could study
- ❌ **Not human-readable** (1-2 char JSON keys, machine-generated)
- ❌ Designed for UI animations, not full video content
- ❌ No "language" — you can't write Lottie by hand
- ❌ Authored in After Effects, not code
- ❌ No decompiler concept (one-way: AE → JSON)

**What VAC does differently:** Human-readable language you write by hand, full video support (not just UI animations), algorithmic decompiler to go from video → code.

---

### Rive

| Property | Detail |
|----------|--------|
| **What** | Interactive experience engine with GPU-accelerated vector renderer |
| **Rendering** | Custom GPU renderer at 120fps |
| **Platforms** | Web, iOS, Android, Flutter, React, Unity, Unreal, C++ |
| **Adoption** | 2B+ users reached (Spotify, Duolingo, Disney, ESPN, LinkedIn, Google) |
| **Business model** | Free tier + paid plans |

**How it works:** Design, animate, and add interactivity in Rive's visual editor. Exports a binary `.riv` format. Open-source runtimes render it natively on each platform. Key innovation: State Machine-driven animations (not just timeline-based).

**Key claims:** 4x faster production than After Effects/Lottie workflows, 90% smaller files.

**Relevance to VAC:**
- ✅ **Custom binary format (`.riv`)** — proves custom formats can outperform generic ones
- ✅ GPU-accelerated rendering at 120fps — VAC should target similar performance
- ✅ State machine concept for interactive animations is powerful
- ✅ Open-source runtimes on multiple platforms
- ❌ Visual editor only — no code-first approach
- ❌ Proprietary format (not human-readable)
- ❌ Focused on interactive UI, not full video content
- ❌ No decompiler

**What VAC does differently:** Code-first (not visual-editor-first), human-readable source format, full video output, algorithmic decompilation.

---

### SVG + SMIL

| Property | Detail |
|----------|--------|
| **SVG** | W3C standard for 2D vector graphics (since 2001). XML-based. |
| **SMIL** | W3C standard for synchronized multimedia (since 1998). XML-based. |
| **SVG 2** | Candidate Recommendation since 2016 (latest draft Sept 2025) |
| **SMIL 3.0** | Latest version (2008) |

**SVG** defines 14 functional areas: paths, basic shapes, text, painting, color, gradients/patterns, clipping/masking/compositing, filter effects, interactivity, linking, scripting, animation, fonts, metadata.

**SVG Path Commands** (Bezier curves):
```
M x,y          - Move to
L x,y          - Line to
C x1,y1 x2,y2 x,y  - Cubic Bezier
S x2,y2 x,y   - Smooth cubic Bezier
Q x1,y1 x,y   - Quadratic Bezier
T x,y          - Smooth quadratic Bezier
A rx,ry ...    - Arc
Z              - Close path
```

**SMIL** defines timing and synchronization for multimedia:
- `<seq>` — sequential playback
- `<par>` — parallel playback
- `<excl>` — exclusive (switch between children)

**Relevance to VAC:**
- ✅ **SVG is the direct analogy: "What SVG did for images, VAC does for video"**
- ✅ Path commands (Bezier curves) are the basis for VAC's geometric primitives
- ✅ SMIL's `<seq>`, `<par>`, `<excl>` timing model maps to VAC scene composition
- ✅ SVG + SMIL together already define "animated vector graphics" — but only for web
- ✅ Potrace (bitmap → SVG) is the proof that algorithmic "decompilation" works for images
- ❌ XML is verbose and not ergonomic for hand-authoring video
- ❌ SVG animation is limited (no real video concepts like scenes, cuts, camera)
- ❌ SMIL is largely abandoned by browsers (Chrome dropped SMIL support)
- ❌ No binary format for efficient playback

**What VAC does differently:** Purpose-built language syntax (not XML), video-native concepts (scenes, cameras, timelines, cuts), binary format for playback, algorithmic decompiler.

---

## 3. Scene Description Formats

### Universal Scene Description (USD)

| Property | Detail |
|----------|--------|
| **What** | Framework for 3D scene interchange |
| **Developed by** | Pixar (open-sourced 2016), now Alliance for OpenUSD (AOUSD) |
| **Members** | Pixar, Adobe, Apple, Autodesk, NVIDIA + Linux Foundation |
| **Formats** | `.usda` (ASCII), `.usdc` (binary), `.usdz` (package/zip) |
| **License** | Modified Apache |
| **Used by** | Blender, Maya, Houdini, Cinema 4D, Unreal, Apple (AR), NVIDIA Omniverse |

**How it works:** USD focuses on non-destructive editing, collaboration, and multiple views/opinions about scene data. It uses a layering system where multiple USD files can be composed together.

**Key concept:** ASCII (`.usda`) and binary (`.usdc`) representations of the same data — exactly what VAC plans with `.vac` (text) and `.vacb` (binary).

**Relevance to VAC:**
- ✅ **Dual format model** (ASCII `.usda` + binary `.usdc`) is exactly VAC's `.vac` + `.vacb` plan
- ✅ Scene graph hierarchy is the standard approach for structured graphics
- ✅ Non-destructive composition/layering is a powerful concept
- ✅ Backed by industry giants — validates the approach of structured scene descriptions
- ❌ Focused on 3D scenes, not 2D video
- ❌ Not a programming language — it's a data interchange format
- ❌ Very complex (designed for film production pipelines)

**What VAC does differently:** 2D video focus, actual programming language with control flow, much simpler scope, algorithmic decompiler.

---

### glTF

| Property | Detail |
|----------|--------|
| **What** | Standard file format for 3D scenes and models |
| **Developed by** | Khronos Group |
| **Versions** | glTF 1.0 (2015), glTF 2.0 (2017) — ISO/IEC 12113:2022 |
| **Formats** | `.gltf` (JSON/ASCII), `.glb` (binary) |
| **Nickname** | "The JPEG of 3D" |
| **Adoption** | Massive — Godot, Unity, Unreal, Blender, Facebook, Microsoft, Google |

**How it works:** JSON-based format with optional binary blobs. Nodes are organized in hierarchies. Supports geometry, materials (PBR), animations, and scene graphs. Extensible via arbitrary JSON properties.

**Relevance to VAC:**
- ✅ **"JPEG of 3D"** — VAC aims to be the "JPEG of structured video" (or rather, "SVG of video")
- ✅ JSON + binary model is similar to VAC's text + binary format plan
- ✅ Extension mechanism for adding custom data is a good pattern
- ✅ ISO standardization shows the path for format adoption
- ❌ 3D-focused, not 2D video
- ❌ Not a language — data interchange format
- ❌ Relies on external renderers

**What VAC does differently:** 2D video domain, actual programming language, built-in rendering pipeline, decompiler.

---

## 4. Algorithmic Tracing Tools

### Potrace

| Property | Detail |
|----------|--------|
| **What** | Bitmap-to-vector tracing algorithm and tool |
| **Author** | Peter Selinger |
| **Language** | C |
| **License** | GPL |
| **Used by** | Inkscape, Boxy SVG, many others |
| **Ports** | JavaScript, Python, Swift, Ruby, Dart, C# |

**How it works (purely algorithmic):**
1. **Bitmap → Path decomposition**: Finds boundaries between black and white regions
2. **Path optimization**: Fits optimal polygons to the paths
3. **Curve fitting**: Converts polygon segments to Bezier curves using least-squares fitting
4. **Output**: SVG, EPS, PDF, or other vector formats

**This is the most directly relevant project to VAC's decompiler.** Potrace proves that purely algorithmic bitmap→vector conversion works and produces high-quality results. No ML/AI involved.

**Algorithm pipeline:**
```
Bitmap pixels
  → Edge detection (boundary tracing)
    → Path decomposition (connected components)
      → Polygon approximation (optimal polygon fitting)
        → Bezier curve fitting (least squares)
          → SVG output
```

**Relevance to VAC:**
- ✅ **Direct proof that algorithmic "decompilation" works** (raster → structured vector)
- ✅ The curve-fitting algorithm is exactly what VAC's decompiler needs
- ✅ GPL C code is available to study and learn from
- ✅ Widely used and battle-tested (Inkscape relies on it)
- ✅ Ports exist in many languages
- ⚠️ Works on single images, not video (VAC needs to add temporal analysis)
- ⚠️ Works on black/white bitmaps (VAC needs color support)

**How VAC extends this:**
- Frame-by-frame Potrace-style tracing → geometric primitives
- Add temporal coherence (match shapes across frames)
- Add color quantization and gradient detection
- Add motion estimation (optical flow) between frames
- Generate VAC code with `animate` blocks from the motion data

---

## 5. Creative Coding Frameworks

### Processing / p5.js

| Property | Detail |
|----------|--------|
| **What** | Creative coding language/framework for visual arts |
| **Created** | 2001 by Casey Reas & Ben Fry (MIT Media Lab) |
| **Language** | Java (Processing), JavaScript (p5.js) |
| **License** | GPL/LGPL |
| **p5.js users** | 1.5M+ |
| **Related** | Arduino, Wiring (hardware), py5 (Python) |

**How it works:** Simplified programming environment for visual/interactive art. Core loop: `setup()` runs once, `draw()` runs every frame. Built-in functions for shapes, colors, transforms.

```java
// Processing example
void setup() {
  size(400, 400);
}
void draw() {
  ellipse(mouseX, mouseY, 50, 50);
}
```

**Relevance to VAC:**
- ✅ **Pioneered "code → visual output" for non-programmers** — VAC's target is similar
- ✅ The `setup()` / `draw()` frame loop maps to VAC's scene/frame model
- ✅ Built-in shape primitives (rect, ellipse, line, bezier) — same as VAC
- ✅ 20+ years of community proves demand for visual coding
- ✅ Spawned an entire ecosystem (Arduino, p5.js, Processing.py, etc.)
- ❌ Produces interactive programs, not video files
- ❌ Not a standalone language (Java/JS under the hood)
- ❌ No decompiler concept
- ❌ No structured file format

**What VAC does differently:** Outputs actual video files, standalone language, structured format with binary representation, bidirectional decompilation.

---

## 6. Declarative Layout Libraries

### Clay

| Property | Detail |
|----------|--------|
| **What** | High-performance 2D UI layout library |
| **Author** | Nic Barker |
| **Language** | C (single 4.8k LOC header file) |
| **GitHub** | github.com/nicbarker/clay |
| **Stars** | ~17,000 |
| **License** | Zlib |

**How it works:** Declarative C macros define a UI hierarchy. Clay calculates layout (flexbox-like model) and outputs a sorted array of `RenderCommand` structs. Renderer-agnostic — you implement the actual drawing.

```c
CLAY(CLAY_ID("Box"), {
    .layout = { .padding = CLAY_PADDING_ALL(16) },
    .backgroundColor = { 200, 100, 50, 255 },
    .cornerRadius = CLAY_CORNER_RADIUS(10)
}) {
    CLAY_TEXT(CLAY_STRING("Hello"), { .fontSize = 24 });
}
```

**Key innovations:**
- Microsecond layout performance
- Zero-dependency single header file
- Static arena memory (no malloc/free)
- Renderer-agnostic render command output
- Compiles to 15kb WASM

**Relevance to VAC:**
- ✅ **Render command architecture** — VAC's scene graph → render pipeline should follow this pattern
- ✅ Declarative C macros prove you can have nice DSL-like syntax in C
- ✅ Renderer-agnostic design — VAC should separate layout/scene from rendering
- ✅ Arena-based memory model is excellent for frame-by-frame rendering
- ✅ Transition/animation API with easing functions
- ❌ UI library, not a video tool
- ❌ No file format

**What VAC does differently:** Video output, custom language (not C macros), file format, decompiler.

---

## Comparison Matrix

| Project | Type | Language | Custom Format | Human-Readable | Decompiler | Full Video | Open Source | Rendering |
|---------|------|----------|---------------|----------------|------------|------------|-------------|-----------|
| **VAC** (goal) | Language + Format | VAC (custom) | ✅ .vac/.vacb | ✅ | ✅ Algorithmic | ✅ | ✅ | Native (Skia/Cairo) |
| **Remotion** | Library | TypeScript/React | ❌ | ✅ (React) | ❌ | ✅ | Partial* | Chromium + FFmpeg |
| **Motion Canvas** | Library | TypeScript | ❌ | ✅ (TS) | ❌ | ❌ (animation) | ✅ MIT | Browser Canvas |
| **Manim** | Library | Python | ❌ | ✅ (Python) | ❌ | ✅ | ✅ MIT | Cairo + FFmpeg |
| **Editly** | Tool | JSON/Node.js | ❌ | ⚠️ (JSON) | ❌ | ✅ | ✅ MIT | FFmpeg |
| **Lottie** | Format | JSON | ✅ .json/.lottie | ❌ (1-2 char keys) | ❌ | ❌ (animation) | ✅ | Web/Native renderers |
| **Rive** | Engine | Visual editor | ✅ .riv | ❌ (binary) | ❌ | ❌ (interactive) | Partial* | Custom GPU |
| **SVG+SMIL** | Standard | XML | ✅ .svg | ⚠️ (verbose XML) | ❌ | ❌ (image) | ✅ W3C | Browsers/librsvg |
| **USD** | Format | Text/Binary | ✅ .usda/.usdc | ✅ (.usda) | ❌ | ❌ (3D scenes) | ✅ Apache | Various |
| **glTF** | Format | JSON/Binary | ✅ .gltf/.glb | ⚠️ (JSON) | ❌ | ❌ (3D scenes) | ✅ Khronos | Various |
| **Potrace** | Algorithm | C | ❌ | N/A | ✅ (image→vector) | ❌ | ✅ GPL | N/A |
| **Processing** | Framework | Java/JS | ❌ | ✅ | ❌ | ❌ (interactive) | ✅ GPL | Java2D/OpenGL |
| **Clay** | Library | C | ❌ | ✅ (C macros) | ❌ | ❌ (UI) | ✅ Zlib | Renderer-agnostic |

*Remotion has commercial licensing; Rive has open-source runtimes but proprietary editor.

---

## Key Takeaways for VAC

### 1. The Market Exists
- Remotion (41k ⭐), Manim (28k ⭐), Motion Canvas (18.4k ⭐) prove massive demand for "code → video"
- Lottie (280k+ companies) proves structured animation formats are widely adopted
- Processing (20+ years, 1.5M p5.js users) proves visual coding has lasting appeal

### 2. Nobody Has a Decompiler
- Every existing tool is one-way (code/design → video/animation)
- No project attempts video → structured code (without ML/AI)
- Potrace is the only algorithmic "reverse" tool, but only for single images
- **This is VAC's biggest differentiator**

### 3. No Existing Human-Readable Video Language
- Remotion/Manim use host languages (TypeScript/Python)
- Lottie JSON is machine-generated, not human-readable
- SVG/XML is verbose
- **A purpose-built, ergonomic video language doesn't exist yet**

### 4. Dual Format (Text + Binary) is Proven
- USD does `.usda` (ASCII) + `.usdc` (binary) — same concept as VAC's `.vac` + `.vacb`
- glTF does `.gltf` (JSON) + `.glb` (binary)
- This dual-format approach is industry-proven and correct

### 5. The Rendering Pipeline is Solved
- Cairo (Manim uses it), tiny-skia (Rust native), Skia (industry standard)
- FFmpeg for final video encoding
- No need to reinvent rendering — focus on the language and format

---

## What VAC Can Learn From Each

| Project | Lesson for VAC |
|---------|---------------|
| **Remotion** | The "every frame is a function" model. Parametric video generation. Server-side rendering architecture. |
| **Motion Canvas** | Generator-based animation sequencing. Real-time preview editor (for VAC Studio). The monorepo architecture for related packages. |
| **Manim** | Scene-based API design. The `animate` transform pattern. Cairo rendering pipeline. LaTeX/text integration. |
| **Editly** | JSON spec → video pipeline. Layer composition model. Streaming editing for performance. Transition library (gl-transitions). |
| **Lottie** | Shape/layer data model. Multi-platform renderer approach. Keyframe animation representation. The dotLottie container format (zip bundle). Community-driven standardization (LAC). |
| **Rive** | GPU-accelerated rendering at 120fps (target for VAC player). State machine for interactive animations. Binary format efficiency (90% smaller). |
| **SVG+SMIL** | Bezier curve path commands (reuse for VAC path primitives). SMIL timing model (`seq`/`par`/`excl`). Filter effects pipeline. |
| **USD** | Dual text/binary format design. Non-destructive composition layers. Industry alliance for standardization. |
| **glTF** | Extension mechanism for format evolution. ISO standardization path. "The JPEG of 3D" narrative (VAC = "The SVG of video"). |
| **Potrace** | Core algorithm for VAC's decompiler: boundary tracing → polygon fitting → Bezier curve fitting. Pure algorithmic, no ML. |
| **Processing** | Simple API for visual creation (`setup`/`draw` paradigm). Built-in shape primitives. Teaching visual coding to non-programmers. Community-driven ecosystem growth. |
| **Clay** | Renderer-agnostic render command architecture. Arena-based memory for frame rendering. Transition API with easing. Single-header simplicity philosophy. |

---

## The Gap VAC Fills

```
Existing landscape:

  Pixels (MP4/H.264)          AI Classifications (YOLO/SAM)
  ├── Raw video frames         ├── "person at (x,y)"
  ├── Lossy compression        ├── Probabilistic
  ├── Not editable             ├── Unreliable for recreation
  └── Not human-readable       └── Loses geometric detail

                    ┌──────────────┐
                    │     VAC      │
                    │              │
                    │  Geometric   │
                    │  Primitives  │
                    │  + Animation │
                    │  Curves      │
                    │              │
                    │  Deterministic│
                    │  Measurable  │
                    │  Editable    │
                    │  Human-      │
                    │  Readable    │
                    └──────────────┘

  Structured but not video:     Code-to-video but not a format:
  ├── Lottie (UI animations)    ├── Remotion (React → MP4)
  ├── SVG (static/simple anim)  ├── Manim (Python → MP4)
  ├── Rive (interactive)        ├── Motion Canvas (TS → frames)
  └── USD/glTF (3D scenes)      └── Editly (JSON → MP4)
```

**VAC is the first project to combine:**
1. ✅ A purpose-built, human-readable language
2. ✅ Compilation to video (like Remotion/Manim but with a real language)
3. ✅ A structured format (like Lottie/SVG but for video)
4. ✅ Dual text/binary formats (like USD/glTF)
5. ✅ An algorithmic decompiler (extending Potrace to video)
6. ✅ Purely algorithmic — no ML/AI dependency

---

## Recommended Libraries & Tools to Build VAC

Based on this research, here are the specific tools and libraries recommended for building VAC:

### Language & Compiler (Phase 1)
| Need | Tool | Why |
|------|------|-----|
| Parser | **pest** (Rust) or **nom** (Rust) | PEG grammar for `.vac` language parsing |
| AST | Custom Rust structs | Typed scene graph representation |
| 2D Rendering | **tiny-skia** (Rust) | Pure Rust, no system deps, Skia subset |
| SVG Rendering | **resvg** (Rust) | Reference SVG renderer, uses tiny-skia |
| Video Encoding | **ffmpeg-next** (Rust) | FFmpeg bindings for MP4/H.264 output |
| CLI | **clap** (Rust) | Argument parsing for `vac` CLI tool |

### Decompiler (Phase 2)
| Need | Tool | Why |
|------|------|-----|
| Frame Extraction | **ffmpeg-next** | Decode video → raw frames |
| Edge Detection | **imageproc** (Rust) or OpenCV | Canny/Sobel edge detection |
| Contour Tracing | Study **Potrace** (C, GPL) | Boundary tracing, curve fitting algorithms |
| Vector Tracing | **visioncortex/vtracer** (Rust) | Raster-to-vector, similar to Potrace but Rust |
| Color Quantization | **color_quant** (Rust) | Reduce colors to palette |
| Optical Flow | **opencv-rust** | Lucas-Kanade / Farnebäck for motion estimation |
| Scene Detection | Custom (histogram comparison) | Detect cuts between scenes |

### Format & Playback (Phase 3)
| Need | Tool | Why |
|------|------|-----|
| Binary Serialization | **bincode** or **flatbuffers** | Efficient `.vacb` binary format |
| Compression | **zstd** (Rust) | Fast compression for binary format |
| GPU Rendering | **wgpu** (Rust) | GPU-accelerated rendering for VAC Player |
| WASM | **wasm-pack** | Compile VAC to WASM for browser playback |

### Ecosystem (Phase 4)
| Need | Tool | Why |
|------|------|-----|
| LSP Server | **tower-lsp** (Rust) | VSCode extension language support |
| Web Playground | **wasm-bindgen** | VAC compiler running in the browser |
| Package Registry | **crates.io** model | Inspiration for VAC package manager |

---

## Further Research Needed

- [ ] **Video codec internals** — Study H.264/H.265 I-frame/P-frame structure for VAC binary format inspiration
- [ ] **VTracer** (visioncortex/vtracer) — Rust alternative to Potrace, may be better fit
- [ ] **Cavalry** — 2D motion design tool, may have relevant UX patterns
- [ ] **Houdini** — Procedural animation system, node-based approach
- [ ] **GSAP/Anime.js** — Web animation libraries, easing function libraries
- [ ] **OpenTimelineIO** — Pixar's editorial timeline interchange format
- [ ] **Academic papers** — "Video as structured data" research, video grammars
- [ ] **FFmpeg filtergraph DSL** — Existing text-based video manipulation language

---

*Last updated: Research conducted for VAC project planning*
*This document will be updated as new tools and projects are discovered*
