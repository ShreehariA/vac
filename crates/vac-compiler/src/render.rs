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
    /// Scale on each axis about the anchor (v1.21; one value before).
    scale: (f32, f32),
    opacity: f32,
    /// Clockwise degrees about the shape's centre (v0.14).
    rotation: f32,
    /// The canvas window outside which nothing of the shape is drawn
    /// (v1.22), `(x, y, w, h)`; `None` draws everywhere.
    clip: Option<(f32, f32, f32, f32)>,
    /// Decoded pixels of an `image()` shape (v1.6), premultiplied.
    image: Option<std::sync::Arc<Pixmap>>,
}

impl ShapeState {
    fn new(base: Shape) -> Self {
        let image = match &base {
            Shape::Image { src, .. } => decode_image(src).map(std::sync::Arc::new),
            _ => None,
        };
        ShapeState {
            base,
            image,
            fill: Some(Color::rgb(0, 0, 0)),
            fill_gradient: None,
            stroke_width: 2.0,
            stroke: None,
            position: None,
            scale: (1.0, 1.0),
            opacity: 1.0,
            rotation: 0.0,
            clip: None,
        }
    }
}

pub fn render_document(doc: &Document) -> Result<Vec<RenderedFrame>, CompileError> {
    let mut all = Vec::with_capacity(frame_count(doc));
    render_document_each(doc, |_, f| {
        all.push(f);
        Ok(())
    })?;
    Ok(all)
}

/// How many frames [`render_document`] produces, without rendering them.
pub fn frame_count(doc: &Document) -> usize {
    let fps = doc.canvas.fps.max(1) as u64;
    doc.scenes
        .iter()
        .map(|s| ((s.duration_ms as u64 * fps) / 1000).max(1) as usize)
        .sum()
}

/// Render frame by frame, handing each to `each(index, frame)` instead of
/// keeping them: a caller that only needs to look at each frame once (the
/// fidelity metric) never holds the whole clip in memory.
pub fn render_document_each(
    doc: &Document,
    mut each: impl FnMut(usize, RenderedFrame) -> Result<(), CompileError>,
) -> Result<(), CompileError> {
    let mut index = 0usize;
    for scene in &doc.scenes {
        render_scene(doc, scene, &mut |f| {
            each(index, f)?;
            index += 1;
            Ok(())
        })?;
    }
    Ok(())
}

fn render_scene(
    doc: &Document,
    scene: &Scene,
    each: &mut dyn FnMut(RenderedFrame) -> Result<(), CompileError>,
) -> Result<(), CompileError> {
    let w = doc.canvas.width;
    let h = doc.canvas.height;
    let fps = doc.canvas.fps.max(1);
    let rig = SceneRig::build(scene)?;

    // Number of frames — one for the first instant and one for each
    // tick after that, ending at `duration_ms`.  An animation that runs
    // 3000ms at 30fps therefore renders 90 frames (t=0..3000, step=33.3ms).
    let frame_count = ((scene.duration_ms as u64 * fps as u64) / 1000).max(1) as u32;

    for i in 0..frame_count {
        let t_ms = ((i as u64 * 1000) / fps as u64) as u32;
        let frame = render_frame_at(w, h, &rig.declared, &rig.shapes, &rig.animations, t_ms)?;
        each(frame)?;
    }
    Ok(())
}

/// One scene's declarations, resolved once: every shape's base state with
/// its assignments applied, and its animation.
struct SceneRig {
    /// Shape names in declaration (= draw) order.
    declared: Vec<String>,
    shapes: HashMap<String, ShapeState>,
    animations: HashMap<String, Animation>,
}

impl SceneRig {
    fn build(scene: &Scene) -> Result<Self, CompileError> {
        // Pre-collect declarations so an `animate` block can reference a
        // shape declared either before or after it (declaration order is
        // preserved for draw order, but animate lookup is name-based).
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
        Ok(SceneRig {
            declared,
            shapes,
            animations,
        })
    }

    /// Shape `k` as it stands at `t_ms`.
    fn live(&self, k: usize, t_ms: u32) -> ShapeState {
        let name = &self.declared[k];
        let state = self.shapes.get(name).expect("declared shape exists");
        apply_animation(state, self.animations.get(name), t_ms)
    }

    /// [`SceneRig::live`], with `tweak` applied when it names shape `k`.
    fn live_tweaked(&self, k: usize, t_ms: u32, tweak: Option<&Tweak>) -> ShapeState {
        let Some(tw) = tweak.filter(|tw| tw.shape == k) else {
            return self.live(k, t_ms);
        };
        let mut st = match &tw.animation {
            Some(a) => {
                let state = self.shapes.get(&self.declared[k]).expect("declared shape exists");
                apply_animation(state, Some(a), t_ms)
            }
            None => self.live(k, t_ms),
        };
        if let Some(b) = &tw.base {
            st.base = b.clone();
        }
        if tw.offset != (0.0, 0.0) {
            let p = st.position.unwrap_or_else(|| anchor_of(&st.base));
            st.position = Some((p.0 + tw.offset.0, p.1 + tw.offset.1));
        }
        if let Some(w) = tw.stroke_width {
            st.stroke_width = w.max(0.0);
        }
        st
    }
}

/// The point a shape's `position()` keyframes move.
fn anchor_of(base: &Shape) -> (f32, f32) {
    match base {
        Shape::Rect { x, y, .. } | Shape::Image { x, y, .. } => (*x, *y),
        Shape::Ellipse { cx, cy, .. } => (*cx, *cy),
        Shape::Path(segs) => vac_format::path_anchor(segs),
    }
}

/// One shape changed, to try an alternative without rebuilding the rig
/// (v1.9): its declared geometry replaced, its position moved in every
/// frame, its stroke width set.
#[derive(Debug, Clone, PartialEq)]
pub struct Tweak {
    /// The shape, as an index into [`Rig::shapes`].
    pub shape: usize,
    /// Replacement geometry; `None` keeps the declared one. The anchor —
    /// what `position()` moves — is the replacement's.
    pub base: Option<Shape>,
    /// Added to the shape's position in every frame.
    pub offset: (f32, f32),
    /// Replacement stroke width; `None` keeps it.
    pub stroke_width: Option<f32>,
    /// A replacement for the shape's whole `animate` block (v1.11);
    /// `None` keeps it.
    pub animation: Option<Animation>,
}

/// Where a shape stands in one frame — see [`Rig::placement`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub position: (f32, f32),
    /// Scale on each axis (v1.21).
    pub scale: (f32, f32),
    pub rotation: f32,
    pub opacity: f32,
}

/// A document resolved once for repeated, partial rendering (v1.7).
///
/// The shape audit asks, for every shape, what each frame would look like
/// without it — but only inside the shape's own footprint. Re-rendering
/// whole frames for that would cost one clip render per shape. A `Rig`
/// resolves the declarations once, reports where each shape paints in each
/// frame, and renders just a rectangle, drawing only the shapes the caller
/// admits, in the same order, offset by a whole-pixel translation.
///
/// The whole-canvas region is the frame, exactly. A smaller region matches
/// its crop of the frame except along a curve that crosses the region's
/// edge: tiny-skia chops curves at the clip and flattens the pieces
/// differently, moving a few anti-aliased pixels (by up to ~40 levels on
/// the fixtures). A shape compared against its own absence in one region
/// sees the same chopping on both sides, which is all the audit relies on.
pub struct Rig {
    width: u32,
    height: u32,
    scenes: Vec<SceneRig>,
    /// Global frame index → (scene, time within it), exactly as
    /// [`render_document_each`] numbers and times frames.
    timeline: Vec<(usize, u32)>,
}

impl Rig {
    pub fn new(doc: &Document) -> Result<Self, CompileError> {
        let fps = doc.canvas.fps.max(1) as u64;
        let mut scenes = Vec::with_capacity(doc.scenes.len());
        let mut timeline = Vec::new();
        for (si, scene) in doc.scenes.iter().enumerate() {
            scenes.push(SceneRig::build(scene)?);
            let n = ((scene.duration_ms as u64 * fps) / 1000).max(1);
            timeline.extend((0..n).map(|k| (si, ((k * 1000) / fps) as u32)));
        }
        Ok(Rig {
            width: doc.canvas.width,
            height: doc.canvas.height,
            scenes,
            timeline,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.timeline.len()
    }

    /// The time of frame `i` within its scene, ms — what keyframes are
    /// written against.
    pub fn time_ms(&self, i: usize) -> u32 {
        self.timeline[i].1
    }

    /// The shapes of frame `i`'s scene, in draw order. Shape indices in
    /// [`Rig::bounds`] and [`Rig::render_region`] index this list.
    pub fn shapes(&self, i: usize) -> &[String] {
        &self.scenes[self.timeline[i].0].declared
    }

    /// The pixels shape `k` can touch in frame `i`, as `[x0, y0, x1, y1)`
    /// clipped to the canvas — a conservative box (anti-aliasing and the
    /// stroke included). `None` when it paints nothing there: fully
    /// transparent, unpainted, or off the canvas.
    pub fn bounds(&self, i: usize, k: usize) -> Option<[u32; 4]> {
        self.bounds_with(i, k, None)
    }

    /// [`Rig::bounds`] with `tweak` applied.
    pub fn bounds_with(&self, i: usize, k: usize, tweak: Option<&Tweak>) -> Option<[u32; 4]> {
        let (si, t) = self.timeline[i];
        let st = self.scenes[si].live_tweaked(k, t, tweak);
        let r = paint_bounds(&st)?;
        let clamp = |v: f32, hi: u32| (v.max(0.0) as u32).min(hi);
        let b = [
            clamp(r.left().floor() - 1.0, self.width),
            clamp(r.top().floor() - 1.0, self.height),
            clamp(r.right().ceil() + 1.0, self.width),
            clamp(r.bottom().ceil() + 1.0, self.height),
        ];
        (b[0] < b[2] && b[1] < b[3]).then_some(b)
    }

    /// Where shape `k` stands in frame `i`: the point its `position()`
    /// keyframes move, and its scale, rotation and opacity.
    pub fn placement(&self, i: usize, k: usize) -> Placement {
        let (si, t) = self.timeline[i];
        let st = self.scenes[si].live(k, t);
        let anchor = anchor_of(&st.base);
        Placement {
            position: st.position.unwrap_or(anchor),
            scale: st.scale,
            rotation: st.rotation,
            opacity: st.opacity,
        }
    }

    /// Render the rectangle `[x0, y0, x1, y1)` of frame `i`, drawing only
    /// the shapes `draw(k)` admits, as non-premultiplied RGBA8 rows of
    /// `x1 - x0` pixels.
    pub fn render_region(&self, i: usize, rect: [u32; 4], draw: impl FnMut(usize) -> bool) -> Vec<u8> {
        self.render_region_with(i, rect, draw, None)
    }

    /// [`Rig::render_region`] with `tweak` applied.
    pub fn render_region_with(
        &self,
        i: usize,
        rect: [u32; 4],
        mut draw: impl FnMut(usize) -> bool,
        tweak: Option<&Tweak>,
    ) -> Vec<u8> {
        let (si, t) = self.timeline[i];
        let scene = &self.scenes[si];
        let (w, h) = (rect[2].saturating_sub(rect[0]), rect[3].saturating_sub(rect[1]));
        let Some(mut pixmap) = Pixmap::new(w.max(1), h.max(1)) else {
            return Vec::new();
        };
        let origin = Transform::from_translate(-(rect[0] as f32), -(rect[1] as f32));
        for k in 0..scene.declared.len() {
            if draw(k) {
                draw_shape(&mut pixmap, &scene.live_tweaked(k, t, tweak), origin);
            }
        }
        unpremultiply(pixmap.data(), w.max(1), h.max(1))
    }
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
        draw_shape(&mut pixmap, &live, Transform::identity());
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
    if let Some(s) = sample_channel(anim, t_ms, first_scale, |a, b, p| {
        (a.0 + (b.0 - a.0) * p, a.1 + (b.1 - a.1) * p)
    }) {
        st.scale = s;
    }
    if let Some(o) = sample_channel(anim, t_ms, first_opacity, |a, b, p| a + (b - a) * p) {
        st.opacity = o;
    }
    if let Some(r) = sample_channel(anim, t_ms, first_rotation, |a, b, p| a + (b - a) * p) {
        st.rotation = r;
    }
    if let Some(c) = sample_channel(anim, t_ms, first_clip, |a, b, p| {
        (a.0 + (b.0 - a.0) * p, a.1 + (b.1 - a.1) * p, a.2 + (b.2 - a.2) * p, a.3 + (b.3 - a.3) * p)
    }) {
        st.clip = Some(c);
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
fn first_scale(k: &Keyframe) -> Option<(f32, f32)> {
    k.transforms.iter().find_map(|t| match t {
        VTransform::Scale(sx, sy) => Some((*sx, *sy)),
        _ => None,
    })
}
fn first_clip(k: &Keyframe) -> Option<(f32, f32, f32, f32)> {
    k.transforms.iter().find_map(|t| match t {
        VTransform::Clip(x, y, w, h) => Some((*x, *y, *w, *h)),
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

/// Decode an `image()` source — a `data:image/...;base64,` URI — into a
/// premultiplied pixmap. `None` for anything else: an undecodable image
/// draws nothing rather than failing the render.
fn decode_image(src: &str) -> Option<Pixmap> {
    let bytes = vac_format::base64::decode_data_uri(src)?;
    let rgba = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut px = rgba.into_raw();
    for p in px.chunks_mut(4) {
        let a = p[3] as u16;
        for c in &mut p[..3] {
            *c = ((*c as u16 * a + 127) / 255) as u8;
        }
    }
    Pixmap::from_vec(px, tiny_skia::IntSize::from_wh(w, h)?)
}

/// Where an `image()` shape's pixels land: image pixels → its declared
/// box, then the chain every shape takes — rotate about the centre, anchor
/// at the top-left, scale, move to `position`.
fn image_transform(state: &ShapeState, img: &Pixmap) -> Option<Transform> {
    let Shape::Image { x, y, w, h, .. } = &state.base else {
        return None;
    };
    let (sx, sy) = (state.scale.0.max(0.0), state.scale.1.max(0.0));
    let pos = state.position.unwrap_or((*x, *y));
    let to_box = Transform::from_scale(*w / img.width() as f32, *h / img.height() as f32)
        .post_translate(*x, *y);
    let shape = Transform::from_rotate_at(state.rotation, *x + *w * 0.5, *y + *h * 0.5)
        .post_translate(-*x, -*y)
        .post_scale(sx, sy)
        .post_translate(pos.0, pos.1);
    Some(to_box.post_concat(shape))
}

/// A vector shape's outline in its declared coordinates, and the world
/// transform that places it: (rotate about the shape's centre) (translate
/// the anchor to the origin) (scale) (translate to position).
fn placed_path(state: &ShapeState) -> Option<(tiny_skia::Path, Transform)> {
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
        Shape::Image { .. } => return None,
        Shape::Path(segs) => {
            let path = build_path(segs)?;
            // Anchor at the outline's shoelace centroid — this is the
            // point `position(x, y)` keyframes move, matching the
            // measured blob centroid the decompiler emits.
            let a = vac_format::path_anchor(segs);
            (path, a, a)
        }
    };
    let (sx, sy) = (state.scale.0.max(0.0), state.scale.1.max(0.0));
    let pos = state.position.unwrap_or(anchor);
    let transform = Transform::from_rotate_at(state.rotation, centre.0, centre.1)
        .post_translate(-anchor.0, -anchor.1)
        .post_scale(sx, sy)
        .post_translate(pos.0, pos.1);
    Some((path, transform))
}

/// The world-space box a shape paints into, before anti-aliasing; `None`
/// when it paints nothing. Its clip window (v1.22) bounds it.
fn paint_bounds(state: &ShapeState) -> Option<tiny_skia::Rect> {
    let r = unclipped_bounds(state)?;
    match state.clip {
        None => Some(r),
        Some((x, y, w, h)) => {
            let (l, t) = (r.left().max(x), r.top().max(y));
            let (rt, b) = (r.right().min(x + w), r.bottom().min(y + h));
            (l < rt && t < b).then(|| tiny_skia::Rect::from_ltrb(l, t, rt, b)).flatten()
        }
    }
}

fn unclipped_bounds(state: &ShapeState) -> Option<tiny_skia::Rect> {
    if state.opacity <= 0.0 {
        return None;
    }
    if let Some(img) = &state.image {
        let ts = image_transform(state, img)?;
        let r = tiny_skia::Rect::from_xywh(0.0, 0.0, img.width() as f32, img.height() as f32)?;
        return PathBuilder::from_rect(r).transform(ts).map(|p| p.bounds());
    }
    if matches!(state.base, Shape::Image { .. }) {
        return None;
    }
    let filled = state.fill_gradient.is_some() || state.fill.is_some_and(|c| c.a > 0);
    let stroked = state.stroke.is_some_and(|c| c.a > 0) && state.stroke_width > 0.0;
    if !filled && !stroked {
        return None;
    }
    let (path, ts) = placed_path(state)?;
    let b = path.transform(ts)?.bounds();
    // A stroke is centred on the outline and, like the path, scaled.
    let out = if stroked {
        state.stroke_width * 0.5 * state.scale.0.max(state.scale.1).max(0.0)
    } else {
        0.0
    };
    tiny_skia::Rect::from_ltrb(b.left() - out, b.top() - out, b.right() + out, b.bottom() + out)
}

/// Draw one shape. `origin` is applied after the shape's own transform:
/// identity for a full frame, a whole-pixel translation for a region.
fn draw_shape(pixmap: &mut Pixmap, state: &ShapeState, origin: Transform) {
    // The clip window (v1.22), in canvas coordinates like the shape, as a
    // mask over this pixmap. An empty window shows nothing.
    let mask = match state.clip {
        None => None,
        Some((x, y, w, h)) => {
            if !(w > 0.0 && h > 0.0) {
                return;
            }
            let Some(r) = tiny_skia::Rect::from_xywh(x, y, w, h) else { return };
            let Some(mut m) = tiny_skia::Mask::new(pixmap.width(), pixmap.height()) else { return };
            m.fill_path(&PathBuilder::from_rect(r), FillRule::Winding, true, origin);
            Some(m)
        }
    };
    let mask = mask.as_ref();
    if matches!(state.base, Shape::Image { .. }) {
        let Some(img) = &state.image else { return };
        let Some(ts) = image_transform(state, img) else { return };
        let paint = tiny_skia::PixmapPaint {
            opacity: state.opacity.clamp(0.0, 1.0),
            blend_mode: tiny_skia::BlendMode::SourceOver,
            quality: tiny_skia::FilterQuality::Bilinear,
        };
        pixmap.draw_pixmap(0, 0, img.as_ref().as_ref(), &paint, ts.post_concat(origin), mask);
        return;
    }
    let Some((path, transform)) = placed_path(state) else {
        return;
    };
    let transform = transform.post_concat(origin);

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
        pixmap.fill_path(&path, &paint, FillRule::EvenOdd, transform, mask);
    } else if let Some(fill) = state.fill {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        let a = (fill.a as f32 * opacity) as u8;
        paint.set_color_rgba8(fill.r, fill.g, fill.b, a);
        pixmap.fill_path(&path, &paint, FillRule::EvenOdd, transform, mask);
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
        pixmap.stroke_path(&path, &paint, &stroke_settings, transform, mask);
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
            scale: (1.0, 1.0),
            opacity: 1.0,
            rotation: 0.0,
            clip: None,
            image: None,
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
                kf(0, vec![VTransform::Scale(2.0, 2.0), VTransform::Position(0.0, 0.0)]),
                kf(500, vec![VTransform::Position(10.0, 0.0)]),
                kf(1000, vec![VTransform::Scale(4.0, 4.0), VTransform::Position(20.0, 0.0)]),
            ],
        };
        let st = state_at(&anim, 750);
        assert!((st.scale.0 - 3.5).abs() < 1e-4 && (st.scale.1 - 3.5).abs() < 1e-4, "scale {:?}", st.scale);
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

    /// A 2×1 image (red | blue) stretched over a 100×20 box: the left
    /// half renders red, the right half blue — and moving it moves it.
    #[test]
    fn an_image_is_stretched_over_its_box_and_moves_with_it() {
        let src = "canvas 100x20 @10fps\nscene s {\n  duration 200ms\n  let bg = rect(0, 0, 100, 20)\n  bg.fill = #000000\n  let pic = image(0, 0, 100, 20, \"URI\")\n  animate pic {\n    0ms -> position(0, 0)\n    100ms -> position(50, 0)\n    easing: linear\n  }\n}\n".replace("URI", "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAADUlEQVR42mP4zwAE/wEHAAH/PX2MSQAAAABJRU5ErkJggg==");
        let doc = vac_format::parse(&src).unwrap();
        let frames = render_document(&doc).unwrap();
        let px = |f: &RenderedFrame, x: u32| {
            let i = ((10 * 100 + x) * 4) as usize;
            (f.rgba[i], f.rgba[i + 2])
        };
        let (r, b) = px(&frames[0], 10);
        assert!(r > 200 && b < 50, "left quarter should be red: {r},{b}");
        let (r, b) = px(&frames[0], 90);
        assert!(b > 200 && r < 50, "right quarter should be blue: {r},{b}");
        // At 100 ms it has moved 50 px right: x=10 is now uncovered.
        let (r, b) = px(&frames[1], 10);
        assert!(r < 20 && b < 20, "moved image should leave x=10 black: {r},{b}");
    }

    /// v1.21: `scale(sx, sy)` stretches each axis about the anchor, so a
    /// rect keeps its top-left and only the stretched axis grows — a
    /// progress bar, not a zoom.
    #[test]
    fn a_rect_stretches_from_its_top_left() {
        let doc = vac_format::parse(
            "canvas 60x30 @10fps\n\nscene main {\n  duration 100ms\n  let bar = rect(10, 10, 20, 10)\n  bar.fill = #ffffff\n  bar.stroke = none\n  animate bar {\n    0ms -> scale(2, 1)\n  }\n}\n",
        )
        .unwrap();
        let frames = render_document(&doc).unwrap();
        let red = |x: u32, y: u32| frames[0].rgba[((y * 60 + x) * 4) as usize];
        assert_eq!(red(12, 15), 255, "the top-left stays");
        assert_eq!(red(45, 15), 255, "twice as wide: 10 to 50");
        assert_eq!(red(55, 15), 0, "and no wider");
        assert_eq!(red(15, 25), 0, "the height is unstretched: 10 to 20");
    }

    /// v1.22: `clip(x, y, w, h)` keeps a shape inside a canvas window, and
    /// an empty window shows nothing — a wipe is a picture whose window
    /// grows.
    #[test]
    fn a_clip_window_hides_what_is_outside_it() {
        let doc = vac_format::parse(
            "canvas 60x30 @10fps\n\nscene main {\n  duration 200ms\n  let r = rect(0, 0, 60, 30)\n  r.fill = #ffffff\n  r.stroke = none\n  animate r {\n    0ms -> clip(40, 0, 20, 30)\n    100ms -> clip(60, 0, 0, 30)\n  }\n}\n",
        )
        .unwrap();
        let frames = render_document(&doc).unwrap();
        let red = |f: usize, x: u32, y: u32| frames[f].rgba[((y * 60 + x) * 4) as usize];
        assert_eq!(red(0, 50, 15), 255, "inside the window");
        assert_eq!(red(0, 30, 15), 0, "outside it");
        assert_eq!(red(1, 59, 15), 0, "an empty window shows nothing");
    }

    /// The two promises the shape audit (v1.7) rests on, over every
    /// fixture that exercises a different drawing path — rects, ellipses,
    /// stroked paths, gradients, rotation, scale, opacity and an image:
    /// a region renders exactly as that crop of the full frame does, and
    /// leaving a shape out changes no pixel outside its reported bounds.
    #[test]
    fn a_region_renders_as_its_crop_and_a_shape_stays_inside_its_bounds() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/");
        for name in ["spin.vac", "shaded.vac", "outlined.vac", "textured.vac", "grow.vac", "fade.vac", "color_shift.vac", "sweep.vac", "progress.vac"] {
            let src = std::fs::read_to_string(format!("{root}{name}")).unwrap();
            let doc = vac_format::parse(&src).unwrap();
            let full = render_document(&doc).unwrap();
            let rig = Rig::new(&doc).unwrap();
            assert_eq!(rig.frame_count(), full.len(), "{name}");
            let (w, h) = (doc.canvas.width, doc.canvas.height);
            let crop = |f: &RenderedFrame, r: [u32; 4]| -> Vec<u8> {
                let mut out = Vec::new();
                for y in r[1]..r[3] {
                    let a = ((y * w + r[0]) * 4) as usize;
                    out.extend_from_slice(&f.rgba[a..a + ((r[2] - r[0]) * 4) as usize]);
                }
                out
            };
            for i in (0..full.len()).step_by((full.len() / 5).max(1)) {
                for r in [[0, 0, w, h], [w / 3, h / 4, w / 3 + 57, h / 4 + 41], [1, 2, w - 3, h - 1]] {
                    let (a, b) = (rig.render_region(i, r, |_| true), crop(&full[i], r));
                    if r == [0, 0, w, h] {
                        assert!(a == b, "{name} frame {i}: the whole-canvas region must be the frame");
                    }
                    // Elsewhere a curve crossing the region's edge is chopped
                    // there and flattened differently, which moves a few
                    // anti-aliased pixels; content and offset must still agree.
                    let mad = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y) as f64).sum::<f64>() / a.len() as f64;
                    assert!(mad < 0.5, "{name} frame {i}: region {r:?} is off its crop by {mad:.2} levels on average");
                }
                let all = [0, 0, w, h];
                let with = rig.render_region(i, all, |_| true);
                for k in 0..rig.shapes(i).len() {
                    let without = rig.render_region(i, all, |m| m != k);
                    let b = rig.bounds(i, k);
                    for y in 0..h {
                        for x in 0..w {
                            let p = ((y * w + x) * 4) as usize;
                            if with[p..p + 4] != without[p..p + 4] {
                                let inside = b.is_some_and(|b| x >= b[0] && x < b[2] && y >= b[1] && y < b[3]);
                                assert!(inside, "{name} frame {i}: `{}` changed ({x},{y}) outside its bounds {b:?}", rig.shapes(i)[k]);
                            }
                        }
                    }
                }
            }
        }
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
