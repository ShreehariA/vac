//! Frame rendering.  Walks the scene graph at each frame's time
//! offset and rasterises shapes with tiny-skia.

use std::collections::HashMap;

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
use vac_format::{
    Animation, Color, Document, Gradient, Keyframe, Property, Scene, Shape, Statement,
    Transform as VTransform, Value,
};

use crate::CompileError;

pub struct RenderedFrame {
    pub width: u32,
    pub height: u32,
    /// Non-premultiplied RGBA8.
    pub rgba: Vec<u8>,
}

#[derive(Clone)]
struct ShapeState {
    base: Shape,
    fill: Option<Color>,
    /// A gradient fill (v1.2); when set it takes the place of `fill`.
    fill_gradient: Option<Gradient>,
    stroke: Option<Color>,
    /// Stroke width, pixels (v1.4; 2.0 when unset, as before).
    stroke_width: f32,
    /// Absolute world position (anchor point) — overrides the shape's
    /// declared position when set by an `animate` block.
    position: Option<(f32, f32)>,
    scale: f32,
    opacity: f32,
    /// Clockwise degrees about the shape's centre (v0.14).
    rotation: f32,
}

impl ShapeState {
    fn new(base: Shape) -> Self {
        ShapeState {
            base,
            fill: Some(Color::rgb(0, 0, 0)),
            fill_gradient: None,
            stroke_width: 2.0,
            stroke: None,
            position: None,
            scale: 1.0,
            opacity: 1.0,
            rotation: 0.0,
        }
    }
}

pub fn render_document(doc: &Document) -> Result<Vec<RenderedFrame>, CompileError> {
    let mut all = Vec::new();
    for scene in &doc.scenes {
        all.extend(render_scene(doc, scene)?);
    }
    Ok(all)
}

fn render_scene(doc: &Document, scene: &Scene) -> Result<Vec<RenderedFrame>, CompileError> {
    let w = doc.canvas.width;
    let h = doc.canvas.height;
    let fps = doc.canvas.fps.max(1);

    // Pre-collect declarations so an `animate` block can reference a shape
    // declared either before or after it (declaration order is preserved
    // for draw order, but animate lookup is name-based).
    let mut declared: Vec<String> = Vec::new();
    let mut shapes: HashMap<String, ShapeState> = HashMap::new();
    let mut animations: HashMap<String, Animation> = HashMap::new();

    for stmt in &scene.statements {
        match stmt {
            Statement::Let { name, shape } => {
                declared.push(name.clone());
                shapes.insert(name.clone(), ShapeState::new(shape.clone()));
            }
            Statement::Assign {
                target,
                property,
                value,
            } => {
                let st = shapes.get_mut(target).ok_or_else(|| {
                    CompileError::Render(format!("assign to undeclared shape `{target}`"))
                })?;
                match (property, value) {
                    (Property::Fill, Value::Color(c)) => {
                        st.fill = Some(*c);
                        st.fill_gradient = None;
                    }
                    (Property::Fill, Value::Gradient(g)) => {
                        st.fill = None;
                        st.fill_gradient = Some(*g);
                    }
                    (Property::Fill, Value::None) => {
                        st.fill = None;
                        st.fill_gradient = None;
                    }
                    (Property::Stroke, Value::Color(c)) => st.stroke = Some(*c),
                    (Property::Stroke, Value::Gradient(_)) => {
                        return Err(CompileError::Render(format!(
                            "`{target}.stroke`: gradient strokes are not supported"
                        )))
                    }
                    (Property::Stroke, Value::None) => st.stroke = None,
                    (Property::StrokeWidth, Value::Number(n)) => st.stroke_width = n.max(0.0),
                    (Property::StrokeWidth, other) => {
                        return Err(CompileError::Render(format!(
                            "`{target}.stroke_width` takes a number, got {other:?}"
                        )))
                    }
                    (prop @ (Property::Fill | Property::Stroke), Value::Number(n)) => {
                        return Err(CompileError::Render(format!(
                            "`{target}.{}` takes a colour, got the number {n}",
                            prop.as_str()
                        )))
                    }
                }
            }
            Statement::Animate(a) => {
                animations.insert(a.target.clone(), a.clone());
            }
        }
    }

    // Number of frames — one for the first instant and one for each
    // tick after that, ending at `duration_ms`.  An animation that runs
    // 3000ms at 30fps therefore renders 90 frames (t=0..3000, step=33.3ms).
    let frame_count = ((scene.duration_ms as u64 * fps as u64) / 1000).max(1) as u32;

    let mut out = Vec::with_capacity(frame_count as usize);
    for i in 0..frame_count {
        let t_ms = ((i as u64 * 1000) / fps as u64) as u32;
        let frame = render_frame_at(w, h, &declared, &shapes, &animations, t_ms)?;
        out.push(frame);
    }
    Ok(out)
}

fn render_frame_at(
    w: u32,
    h: u32,
    declared: &[String],
    shapes: &HashMap<String, ShapeState>,
    animations: &HashMap<String, Animation>,
    t_ms: u32,
) -> Result<RenderedFrame, CompileError> {
    let mut pixmap = Pixmap::new(w, h)
        .ok_or_else(|| CompileError::Render(format!("invalid canvas size {w}x{h}")))?;

    for name in declared {
        let state = shapes.get(name).expect("declared shape exists");
        let live = apply_animation(state, animations.get(name), t_ms);
        draw_shape(&mut pixmap, &live);
    }

    Ok(RenderedFrame {
        width: w,
        height: h,
        rgba: unpremultiply(pixmap.data(), w, h),
    })
}

fn apply_animation(base: &ShapeState, anim: Option<&Animation>, t_ms: u32) -> ShapeState {
    let mut st = base.clone();
    let Some(anim) = anim else {
        return st;
    };
    if anim.keyframes.is_empty() {
        return st;
    }

    // Each transform kind is its own channel.  A channel interpolates
    // between the nearest keyframes that *carry that kind*, skipping
    // keyframes that only set other kinds, and holds its last (or first)
    // value outside them.  Through v0.10 the renderer looked only at the
    // two keyframes bracketing `t`, so a kind absent from both reverted to
    // its base value — harmless while the decompiler emitted only
    // `position`, wrong the moment it emits a second kind.
    if let Some((x, y)) = sample_channel(anim, t_ms, first_position, |a, b, p| {
        (a.0 + (b.0 - a.0) * p, a.1 + (b.1 - a.1) * p)
    }) {
        st.position = Some((x, y));
    }
    if let Some(s) = sample_channel(anim, t_ms, first_scale, |a, b, p| a + (b - a) * p) {
        st.scale = s;
    }
    if let Some(o) = sample_channel(anim, t_ms, first_opacity, |a, b, p| a + (b - a) * p) {
        st.opacity = o;
    }
    if let Some(r) = sample_channel(anim, t_ms, first_rotation, |a, b, p| a + (b - a) * p) {
        st.rotation = r;
    }
    if let Some(c) = sample_channel(anim, t_ms, first_fill, |a: Color, b: Color, p| {
        let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * p).round().clamp(0.0, 255.0) as u8;
        Color { r: l(a.r, b.r), g: l(a.g, b.g), b: l(a.b, b.b), a: l(a.a, b.a) }
    }) {
        // A colour channel sets a flat fill, replacing any gradient.
        st.fill = Some(c);
        st.fill_gradient = None;
    }
    st
}

fn first_fill(k: &Keyframe) -> Option<Color> {
    k.transforms.iter().find_map(|t| match t {
        VTransform::Fill(c) => Some(*c),
        _ => None,
    })
}

fn sample_channel<T: Copy>(
    anim: &Animation,
    t: u32,
    extract: fn(&Keyframe) -> Option<T>,
    lerp: impl Fn(T, T, f32) -> T,
) -> Option<T> {
    let mut before: Option<(u32, T)> = None;
    let mut after: Option<(u32, T)> = None;
    for k in &anim.keyframes {
        let Some(v) = extract(k) else { continue };
        if k.time_ms <= t {
            before = Some((k.time_ms, v));
        } else if after.is_none() {
            after = Some((k.time_ms, v));
        }
    }
    match (before, after) {
        (Some((t0, a)), Some((t1, b))) => {
            let span = (t1 - t0).max(1) as f32;
            let p = anim.easing.apply((t - t0) as f32 / span);
            Some(lerp(a, b, p))
        }
        (Some((_, v)), None) | (None, Some((_, v))) => Some(v),
        (None, None) => None,
    }
}

fn first_position(k: &Keyframe) -> Option<(f32, f32)> {
    k.transforms.iter().find_map(|t| match t {
        VTransform::Position(x, y) => Some((*x, *y)),
        _ => None,
    })
}
fn first_scale(k: &Keyframe) -> Option<f32> {
    k.transforms.iter().find_map(|t| match t {
        VTransform::Scale(s) => Some(*s),
        _ => None,
    })
}
fn first_rotation(k: &Keyframe) -> Option<f32> {
    k.transforms.iter().find_map(|t| match t {
        VTransform::Rotate(d) => Some(*d),
        _ => None,
    })
}
fn first_opacity(k: &Keyframe) -> Option<f32> {
    k.transforms.iter().find_map(|t| match t {
        VTransform::Opacity(o) => Some(*o),
        _ => None,
    })
}

fn draw_shape(pixmap: &mut Pixmap, state: &ShapeState) {
    let (path, anchor, centre) = match &state.base {
        Shape::Rect { x, y, w, h } => {
            let pb = PathBuilder::from_rect(
                tiny_skia::Rect::from_xywh(*x, *y, *w, *h)
                    .unwrap_or_else(|| tiny_skia::Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap()),
            );
            (pb, (*x, *y), (*x + *w * 0.5, *y + *h * 0.5))
        }
        Shape::Ellipse { cx, cy, rx, ry } => {
            (build_ellipse_path(*cx, *cy, *rx, *ry), (*cx, *cy), (*cx, *cy))
        }
        Shape::Path(segs) => {
            let Some(path) = build_path(segs) else {
                return;
            };
            // Anchor at the outline's shoelace centroid — this is the
            // point `position(x, y)` keyframes move, matching the
            // measured blob centroid the decompiler emits.
            let a = vac_format::path_anchor(segs);
            (path, a, a)
        }
    };

    // World transform: (rotate about the shape's centre) (translate the
    // anchor to the origin) (scale) (translate to position).
    let scale = state.scale.max(0.0);
    let pos = state.position.unwrap_or(anchor);
    let transform = Transform::from_rotate_at(state.rotation, centre.0, centre.1)
        .post_translate(-anchor.0, -anchor.1)
        .post_scale(scale, scale)
        .post_translate(pos.0, pos.1);

    let opacity = state.opacity.clamp(0.0, 1.0);
    if let Some(g) = &state.fill_gradient {
        // The shader lives in the shape's declared coordinates; fill_path
        // applies `transform` to path and shader alike, so the shading
        // moves, scales and turns with the shape.
        let stop = |pos: f32, c: &Color| {
            tiny_skia::GradientStop::new(
                pos,
                tiny_skia::Color::from_rgba8(c.r, c.g, c.b, (c.a as f32 * opacity) as u8),
            )
        };
        let shader = match g {
            Gradient::Linear { c0, p0, c1, p1 } => tiny_skia::LinearGradient::new(
                tiny_skia::Point::from_xy(p0.x, p0.y),
                tiny_skia::Point::from_xy(p1.x, p1.y),
                vec![stop(0.0, c0), stop(1.0, c1)],
                tiny_skia::SpreadMode::Pad,
                Transform::identity(),
            ),
            Gradient::Radial { c0, center, c1, radius } => tiny_skia::RadialGradient::new(
                tiny_skia::Point::from_xy(center.x, center.y),
                tiny_skia::Point::from_xy(center.x, center.y),
                *radius,
                vec![stop(0.0, c0), stop(1.0, c1)],
                tiny_skia::SpreadMode::Pad,
                Transform::identity(),
            ),
        };
        let mut paint = Paint::default();
        paint.anti_alias = true;
        match shader {
            Some(sh) => paint.shader = sh,
            // Degenerate (coincident points, zero radius): the first stop.
            None => {
                let c = match g {
                    Gradient::Linear { c0, .. } | Gradient::Radial { c0, .. } => c0,
                };
                paint.set_color_rgba8(c.r, c.g, c.b, (c.a as f32 * opacity) as u8);
            }
        }
        pixmap.fill_path(&path, &paint, FillRule::EvenOdd, transform, None);
    } else if let Some(fill) = state.fill {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        let a = (fill.a as f32 * opacity) as u8;
        paint.set_color_rgba8(fill.r, fill.g, fill.b, a);
        pixmap.fill_path(&path, &paint, FillRule::EvenOdd, transform, None);
    }

    if let Some(stroke) = state.stroke {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        let a = (stroke.a as f32 * state.opacity.clamp(0.0, 1.0)) as u8;
        paint.set_color_rgba8(stroke.r, stroke.g, stroke.b, a);
        let stroke_settings = tiny_skia::Stroke {
            width: state.stroke_width,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke_settings, transform, None);
    }
}

/// Build a tiny-skia path from `path(...)` segments.  The path is
/// implicitly closed, matching the `.pac` renderer's convention.
fn build_path(segs: &[vac_format::PathSeg]) -> Option<tiny_skia::Path> {
    use vac_format::PathSeg;
    if segs.is_empty() {
        return None;
    }
    let mut pb = PathBuilder::new();
    for seg in segs {
        match *seg {
            PathSeg::MoveTo(p) => pb.move_to(p.x, p.y),
            PathSeg::LineTo(p) => pb.line_to(p.x, p.y),
            PathSeg::CurveTo { c1, c2, p } => pb.cubic_to(c1.x, c1.y, c2.x, c2.y, p.x, p.y),
        }
    }
    pb.close();
    pb.finish()
}

/// Build a closed elliptical path from four cubic-Bezier quadrants.
/// Constant `k = (4/3) * tan(π/8)` gives the standard "Bezier circle"
/// approximation with max radial error < 0.0003.
fn build_ellipse_path(cx: f32, cy: f32, rx: f32, ry: f32) -> tiny_skia::Path {
    let k = 0.5522847498_f32;
    let mut pb = PathBuilder::new();
    pb.move_to(cx + rx, cy);
    pb.cubic_to(cx + rx, cy + ry * k, cx + rx * k, cy + ry, cx, cy + ry);
    pb.cubic_to(cx - rx * k, cy + ry, cx - rx, cy + ry * k, cx - rx, cy);
    pb.cubic_to(cx - rx, cy - ry * k, cx - rx * k, cy - ry, cx, cy - ry);
    pb.cubic_to(cx + rx * k, cy - ry, cx + rx, cy - ry * k, cx + rx, cy);
    pb.close();
    pb.finish()
        .unwrap_or_else(|| PathBuilder::from_circle(cx, cy, rx.max(0.5)).unwrap())
}

/// Convert tiny-skia's premultiplied output to plain RGBA8.
fn unpremultiply(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let n = (w as usize) * (h as usize) * 4;
    let mut out = Vec::with_capacity(n);
    for px in src.chunks_exact(4) {
        let (pr, pg, pb, pa) = (px[0], px[1], px[2], px[3]);
        match pa {
            0 => out.extend_from_slice(&[0, 0, 0, 0]),
            255 => out.extend_from_slice(&[pr, pg, pb, pa]),
            _ => {
                let inv = 255.0 / pa as f32;
                let r = ((pr as f32 * inv).round() as u32).min(255) as u8;
                let g = ((pg as f32 * inv).round() as u32).min(255) as u8;
                let b = ((pb as f32 * inv).round() as u32).min(255) as u8;
                out.extend_from_slice(&[r, g, b, pa]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vac_format::Easing;

    fn kf(t: u32, tr: Vec<VTransform>) -> Keyframe {
        Keyframe { time_ms: t, transforms: tr }
    }

    fn state_at(anim: &Animation, t: u32) -> ShapeState {
        let base = ShapeState {
            base: Shape::Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
            fill: None,
            fill_gradient: None,
            stroke_width: 2.0,
            stroke: None,
            position: None,
            scale: 1.0,
            opacity: 1.0,
            rotation: 0.0,
        };
        apply_animation(&base, Some(anim), t)
    }

    /// Scale set at 0 ms and 1000 ms, position-only keyframe between.
    /// At 750 ms scale must be interpolating 2.0 → 4.0 (3.5), not
    /// reverting to base 1.0 because the bracketing pair lacks it.
    #[test]
    fn a_transform_kind_absent_from_the_bracketing_keyframes_still_interpolates() {
        let anim = Animation {
            target: "s".into(),
            easing: Easing::Linear,
            keyframes: vec![
                kf(0, vec![VTransform::Scale(2.0), VTransform::Position(0.0, 0.0)]),
                kf(500, vec![VTransform::Position(10.0, 0.0)]),
                kf(1000, vec![VTransform::Scale(4.0), VTransform::Position(20.0, 0.0)]),
            ],
        };
        let st = state_at(&anim, 750);
        assert!((st.scale - 3.5).abs() < 1e-4, "scale {}", st.scale);
        assert_eq!(st.position, Some((15.0, 0.0)));
    }

    #[test]
    fn rotation_interpolates_like_any_other_channel() {
        let anim = Animation {
            target: "s".into(),
            easing: Easing::Linear,
            keyframes: vec![
                kf(0, vec![VTransform::Rotate(0.0)]),
                kf(500, vec![VTransform::Position(10.0, 0.0)]),
                kf(1000, vec![VTransform::Rotate(90.0)]),
            ],
        };
        assert!((state_at(&anim, 250).rotation - 22.5).abs() < 1e-4);
        assert_eq!(state_at(&anim, 2000).rotation, 90.0);
    }

    /// A 40×10 bar rotated 90° about its centre must cover a 10×40
    /// vertical strip around the same centre.
    #[test]
    fn a_rotated_rect_turns_about_its_centre() {
        let doc = vac_format::parse(
            "canvas 100x100 @10fps\nscene s {\n  duration 100ms\n  let bg = rect(0, 0, 100, 100)\n  bg.fill = #000000\n  let bar = rect(30, 45, 40, 10)\n  bar.fill = #ffffff\n  animate bar {\n    0ms -> rotate(90)\n  }\n}\n",
        )
        .unwrap();
        let frames = render_document(&doc).unwrap();
        let f = &frames[0];
        let lit = |x: u32, y: u32| f.rgba[((y * 100 + x) * 4) as usize] > 128;
        assert!(lit(50, 32) && lit(50, 68), "vertical extent after rotation");
        assert!(!lit(32, 50) && !lit(68, 50), "horizontal extent gone after rotation");
    }

    /// A horizontal linear gradient renders dark at its start, light at
    /// its end, and in between at the midpoint — and moves with its shape.
    #[test]
    fn a_linear_gradient_fill_is_rendered_in_shape_coordinates() {
        let doc = vac_format::parse(
            "canvas 100x20 @10fps\nscene s {\n  duration 100ms\n  let bg = rect(0, 0, 100, 20)\n  bg.fill = gradient(linear, #000000 at 0,0, #ffffff at 100,0)\n}\n",
        )
        .unwrap();
        let f = &render_document(&doc).unwrap()[0];
        let r = |x: u32| f.rgba[((10 * 100 + x) * 4) as usize];
        assert!(r(1) < 10, "start {}", r(1));
        assert!(r(98) > 245, "end {}", r(98));
        assert!((r(50) as i32 - 128).abs() < 8, "middle {}", r(50));
    }

    #[test]
    fn a_kind_set_once_holds_after_its_last_keyframe() {
        let anim = Animation {
            target: "s".into(),
            easing: Easing::Linear,
            keyframes: vec![
                kf(0, vec![VTransform::Opacity(0.25)]),
                kf(500, vec![VTransform::Position(10.0, 0.0)]),
                kf(1000, vec![VTransform::Position(20.0, 0.0)]),
            ],
        };
        for t in [0, 400, 800, 2000] {
            assert_eq!(state_at(&anim, t).opacity, 0.25, "t={t}");
        }
        assert_eq!(state_at(&anim, 0).position, Some((10.0, 0.0)));
    }
}
