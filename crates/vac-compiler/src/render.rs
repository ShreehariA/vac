//! Frame rendering.  Walks the scene graph at each frame's time
//! offset and rasterises shapes with tiny-skia.

use std::collections::HashMap;

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
use vac_format::{
    Animation, Color, Document, Keyframe, Property, Scene, Shape, Statement,
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
    stroke: Option<Color>,
    /// Absolute world position (anchor point) — overrides the shape's
    /// declared position when set by an `animate` block.
    position: Option<(f32, f32)>,
    scale: f32,
    opacity: f32,
}

impl ShapeState {
    fn new(base: Shape) -> Self {
        ShapeState {
            base,
            fill: Some(Color::rgb(0, 0, 0)),
            stroke: None,
            position: None,
            scale: 1.0,
            opacity: 1.0,
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
                shapes.insert(name.clone(), ShapeState::new(*shape));
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
                    (Property::Fill, Value::Color(c)) => st.fill = Some(*c),
                    (Property::Fill, Value::None) => st.fill = None,
                    (Property::Stroke, Value::Color(c)) => st.stroke = Some(*c),
                    (Property::Stroke, Value::None) => st.stroke = None,
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

    // Find bracketing keyframes for time t_ms; clamp at edges.
    let (k0, k1, p) = bracket(&anim.keyframes, t_ms);
    let eased = anim.easing.apply(p);

    // Aggregate transforms: pull each transform-kind from k0 and k1
    // independently, since k0 may set position+scale while k1 only sets
    // position. If a transform is absent from one side, fall back to the
    // other (so it acts as a "hold").
    let pos0 = first_position(k0);
    let pos1 = first_position(k1);
    if let Some((x, y)) = lerp_pair(pos0, pos1, eased) {
        st.position = Some((x, y));
    }

    let s0 = first_scale(k0);
    let s1 = first_scale(k1);
    if let Some(s) = lerp_scalar(s0, s1, eased) {
        st.scale = s;
    }

    let o0 = first_opacity(k0);
    let o1 = first_opacity(k1);
    if let Some(o) = lerp_scalar(o0, o1, eased) {
        st.opacity = o;
    }

    st
}

fn bracket(kfs: &[Keyframe], t: u32) -> (&Keyframe, &Keyframe, f32) {
    if t <= kfs[0].time_ms {
        return (&kfs[0], &kfs[0], 0.0);
    }
    let last = kfs.last().unwrap();
    if t >= last.time_ms {
        return (last, last, 0.0);
    }
    for pair in kfs.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        if t >= a.time_ms && t <= b.time_ms {
            let span = (b.time_ms - a.time_ms).max(1) as f32;
            let p = (t - a.time_ms) as f32 / span;
            return (a, b, p);
        }
    }
    (last, last, 0.0)
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
fn first_opacity(k: &Keyframe) -> Option<f32> {
    k.transforms.iter().find_map(|t| match t {
        VTransform::Opacity(o) => Some(*o),
        _ => None,
    })
}

fn lerp_pair(
    a: Option<(f32, f32)>,
    b: Option<(f32, f32)>,
    t: f32,
) -> Option<(f32, f32)> {
    match (a, b) {
        (Some(a), Some(b)) => Some((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)),
        (Some(v), None) | (None, Some(v)) => Some(v),
        (None, None) => None,
    }
}
fn lerp_scalar(a: Option<f32>, b: Option<f32>, t: f32) -> Option<f32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a + (b - a) * t),
        (Some(v), None) | (None, Some(v)) => Some(v),
        (None, None) => None,
    }
}

fn draw_shape(pixmap: &mut Pixmap, state: &ShapeState) {
    let (path, anchor) = match state.base {
        Shape::Rect { x, y, w, h } => {
            let pb = PathBuilder::from_rect(
                tiny_skia::Rect::from_xywh(x, y, w, h)
                    .unwrap_or_else(|| tiny_skia::Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap()),
            );
            (pb, (x, y))
        }
        Shape::Ellipse { cx, cy, rx, ry } => (build_ellipse_path(cx, cy, rx, ry), (cx, cy)),
    };

    // World transform: (translate to origin) (scale) (translate to position)
    let scale = state.scale.max(0.0);
    let pos = state.position.unwrap_or(anchor);
    let transform = Transform::from_translate(-anchor.0, -anchor.1)
        .post_scale(scale, scale)
        .post_translate(pos.0, pos.1);

    if let Some(fill) = state.fill {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        let a = (fill.a as f32 * state.opacity.clamp(0.0, 1.0)) as u8;
        paint.set_color_rgba8(fill.r, fill.g, fill.b, a);
        pixmap.fill_path(&path, &paint, FillRule::EvenOdd, transform, None);
    }

    if let Some(stroke) = state.stroke {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        let a = (stroke.a as f32 * state.opacity.clamp(0.0, 1.0)) as u8;
        paint.set_color_rgba8(stroke.r, stroke.g, stroke.b, a);
        let stroke_settings = tiny_skia::Stroke {
            width: 2.0,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke_settings, transform, None);
    }
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
