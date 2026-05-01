# `.vac` Format Specification — v0.1 (Moving Ball Subset)

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
| `opacity(o)` | `[0, 1]` | multiplies fill/stroke alpha |

v0.1 easings: `linear`, `ease-in`, `ease-out`, `ease-in-out`.

Between two consecutive keyframes `(t0, V0)` and `(t1, V1)` for transform `V`, the runtime computes `t = (now − t0) / (t1 − t0)`, applies the easing curve `e = easing(t)`, and produces `V0 + (V1 − V0) * e`. Outside the keyframe range the value clamps.

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
