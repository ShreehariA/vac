# `.vac` Format Specification — v0.1 (Moving Ball Subset)

> **Stale as of v0.9.** This document still describes only the v0.1
> subset and does not cover `path(...)`, shipped in v0.6 and now the most
> common primitive in emitted output (137 `path(` declarations across the
> corpus). The grammar as actually parsed is in
> `crates/vac-format/src/parser.rs`, whose header doc-comment carries an
> accurate EBNF. Two facts worth knowing that contradict the text below:
> `duration` is documented as required but the parser defaults it to 0,
> and the transform table below is current: `scale()` and `opacity()` are
> recovered by the decompiler since v0.11 and `rotate()` exists since v0.14.

This is the **v0.1 subset** of the VAC source format — deliberately tiny, scoped to what a single moving ball needs. Every later version of this spec is a strict superset.

> **Round-trip invariant**: for any `.vac` document `D` that uses only v0.1 features, `parse(write(D)) == D`. The `vac-format` crate has tests asserting this.

---

## File structure

```
document    := canvas-decl scene*
canvas-decl := "canvas" <Dimension> "@" <Fps>
scene       := "scene" <ident> "{" scene-stmt* "}"
```

A document is a single canvas declaration followed by one or more scenes.

### Example

```vac
canvas 480x320 @30fps

scene main {
  duration 3s

  let bg = rect(0, 0, 480, 320)
  bg.fill = #1a1a2e

  let ball = ellipse(60, 160, 24, 24)
  ball.fill = #e94560
  ball.stroke = none

  animate ball {
    0ms    -> position(60, 160)
    1500ms -> position(420, 160)
    3s     -> position(60, 160)
    easing: linear
  }
}
```

---

## Lexical grammar

| Element | Pattern | Example |
|---|---|---|
| Identifier | `[a-zA-Z_][a-zA-Z0-9_-]*` | `ball`, `ease-in-out` |
| Number | `[0-9]+(\.[0-9]+)?` | `60`, `1.4` |
| Dimension | `<digits>x<digits>` | `480x320` |
| Time literal | `<number>(ms\|s)` | `0ms`, `1500ms`, `3s` |
| FPS literal | `<number>fps` | `30fps` |
| Hex color | `#[0-9a-fA-F]{6,8}` | `#e94560`, `#e9456080` |
| Comment | `// … <newline>` | `// header` |
| Punctuation | `{ } ( ) , = . @ : ->` | |

Whitespace separates tokens; line comments start with `//`.

> Hyphens are valid identifier characters (so easing names like `ease-in-out` are a single ident). v0.1 has no arithmetic, so this is unambiguous.

---

## Scene statements

```
scene-stmt := duration-stmt | let-stmt | assign-stmt | animate-stmt
```

### `duration <Time>`

Total scene length. Required exactly once per scene.

### `let <name> = <shape>`

Declare a named shape. v0.1 shapes:

| Form | Args | Anchor |
|---|---|---|
| `rect(x, y, w, h)` | top-left + size | `(x, y)` |
| `ellipse(cx, cy, rx, ry)` | centre + radii | `(cx, cy)` |

### `<name>.<property> = <value>`

Assign to a shape property. v0.1 properties:

| Property | Valid values |
|---|---|
| `fill` | `#rrggbb`, `#rrggbbaa`, `none` |
| `stroke` | `#rrggbb`, `#rrggbbaa`, `none` |

If a shape never gets a `fill` assignment, it defaults to opaque black. `stroke` defaults to `none`.

### `animate <target> { … }`

Drive a named shape with a list of timed keyframes plus an easing.

```
animate <target> {
  <Time> -> <transform>(",")<transform>...
  <Time> -> ...
  easing: <easing-name>
}
```

v0.1 transforms:

| Transform | Args | Effect |
|---|---|---|
| `position(x, y)` | absolute world point | overrides the shape's declared anchor |
| `scale(s)` | uniform | multiplies extent about the anchor |
| `scale(sx, sy)` | per axis | stretches each axis about the anchor (a rect's top-left), so a bar fills as `scale(0, 1)` → `scale(1, 1)`; `scale(s)` is `scale(s, s)` — added v1.21 |
| `opacity(o)` | `[0, 1]` | multiplies fill/stroke alpha |
| `rotate(deg)` | degrees, clockwise | turns the shape about its **centre** (rect/ellipse centre, path centroid), independent of the `position` anchor — added v0.14 |
| `clip(x, y, w, h)` | canvas window | the shape is drawn only inside this rectangle, in canvas coordinates; interpolated per component; an empty window shows nothing — added v1.22 |
| `fill(#rrggbb)` | colour | sets a flat fill, interpolated in RGB between keyframes that carry one; replaces a gradient fill while it applies — added v1.3 |

**Gradient fills (v1.2).** `fill` takes a colour, `none`, or a two-stop
gradient in the shape's declared coordinates, so it moves, scales and turns
with the shape:

```
ball.fill = gradient(radial, #fff2c0 at 76,216, #b05010 radius 52)
panel.fill = gradient(linear, #203a6a at 40,85, #7fb0f0 at 440,85)
```

Colours pad beyond both stops. Strokes take a colour or `none` only.

**Raster images (v1.6).** `image(x, y, w, h, "data:image/jpeg;base64,...")`
stretches an embedded image over a box. It is positioned like a rect (anchor
top-left, rotation about its centre) and takes `position`, `scale`,
`rotate` and `opacity` keyframes like any shape. The decompiler uses it only
as a measured escape hatch: when the best all-vector reconstruction of a clip
is under 30 dB and a raster plate of the static scene reconstructs at least
twice as well, the scenery becomes one `plate = image(...)` with the moving
shapes still written as code on top. The source stays one self-contained
file.

Since v1.10 a plate can also move: under a moving camera the scenery is one
mosaic `image()`, possibly larger than the canvas, and the camera is an
ordinary `animate plate` block of `position()` and `scale()` keyframes,
interpolated linearly. A zoom about a fixed point is a pair of keyframes:

```vac
let plate = image(0, 0, 480, 320, "data:image/jpeg;base64,...")
animate plate {
  0ms -> position(0, 0), scale(1)
  2s  -> position(-130, -75), scale(1.5)
  easing: linear
}
```

Since v1.8 the same primitive also carries **sprites**: a PNG with an alpha
channel, standing in for one moving shape whose flat fill measurably fails
to describe its interior (a shaded ball, a textured tile). The sprite keeps
the shape's `animate` block, with `position()` keyframes moved to the
image's top-left corner, so the motion is still code. Any data URI the
renderer can decode works (`image/png`, `image/jpeg`); an image that fails
to decode draws nothing rather than failing the render.

**Stroke width (v1.4).** `shape.stroke_width = 3` sets the stroke width in
pixels, in declared coordinates (it scales with the shape). The stroke is
centred on the outline, half inside and half outside. Unset, strokes are
2 px, as in every earlier version.

v0.1 easings: `linear`, `ease-in`, `ease-out`, `ease-in-out`.

**Each transform kind is its own channel (v0.11).** For transform kind `V`,
the runtime finds the nearest keyframes *that carry `V`* on either side of
`now` — `(t0, V0)` and `(t1, V1)` — computes `t = (now − t0) / (t1 − t0)`,
applies the easing curve `e = easing(t)`, and produces `V0 + (V1 − V0) * e`.
Keyframes that set only other kinds are skipped, not treated as resets.
Before its first keyframe and after its last, a kind holds that keyframe's
value. (Through v0.10 the runtime used only the two keyframes bracketing
`now`, so a kind absent from both reverted to its base value.)

**Assets (v1.23).** A picture may be declared once at the top level,
`asset name = "data:…"`, before or after the scenes, and an `image()` may
name it in place of its source string: `image(0, 0, 480, 320, name)`. The
writer always does this, so each distinct picture is written once, after
the scenes.

Composition order when rendering a shape: rotate about its centre, then
scale about its anchor, then move the anchor to `position`. Stretching a
rotated shape (`scale(sx, sy)` with `sx ≠ sy`) therefore skews it.

---

## Decompiler invariants

When `vac-decompiler` emits a `.vac` document for the v0.1 subset, the output:

- Always starts with a `canvas` line,
- Declares exactly one `scene` named `main`,
- Always emits a `bg` rectangle covering the canvas with the measured background colour,
- Emits a single `ball` ellipse if a moving foreground blob is detected,
- Uses median statistics (radius, colour) across all detected frames,
- Uses Ramer–Douglas–Peucker simplification on the trajectory to produce keyframes,
- Always emits `easing: linear` (easing classification arrives in v0.2).

---

## Out of scope for v0.1 (planned)

- Multiple shapes per animation
- `path(...)` primitive (raw contour output from Suzuki tracing)
- `text(...)` primitive
- `rotate(angle)` transform
- Stroke width, stroke caps/joins
- Custom cubic-bezier easings
- Camera / viewport
- Layers and z-ordering beyond declaration order
- `import "other.vac"`

Each of these is an additive change — the v0.1 subset is forward-compatible.
