//! Dev probe: track camera.vac's camera and compare it with the truth.
use vac_decompiler::camera::{self, View};
fn main() {
    let mut doc = vac::read("examples/camera.vac").unwrap();
    if std::env::var_os("NO_BALL").is_some() {
        doc.scenes[0].statements.retain(|st| !matches!(st, vac::Statement::Let { name, .. } | vac::Statement::Assign { target: name, .. } if name == "ball") && !matches!(st, vac::Statement::Animate(a) if a.target == "ball"));
    }
    let rendered = vac::render(&doc).unwrap();
    let frames = vac::rendered_to_video_frames(&rendered, doc.canvas.fps);
    let mut views = camera::track(&frames, 0);
    if std::env::var_os("TWO_PASS").is_some() {
        let m = camera::mosaic(&frames, &views, [0, 0, 0]).unwrap();
        views = camera::refine_views(&frames, &m, &views);
    }
    let mut worst = 0f32;
    for (t, v) in views.iter().enumerate() {
        let ms = (t as f32 * 1000.0 / 30.0).floor();
        let s = 1.0 + 0.5 * ms / 2000.0;
        let truth = View { scale: s, offset: (260.0 * (1.0 - s), 150.0 * (1.0 - s)) };
        let err = [(0.0f32, 0.0f32), (480.0, 0.0), (0.0, 320.0), (480.0, 320.0)]
            .iter()
            .map(|&c| {
                let q = truth.to_reference(c);
                let p = v.to_frame(q);
                ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt()
            })
            .fold(0.0, f32::max);
        worst = worst.max(err);
        if t % 6 == 0 {
            println!("frame {t:2}: tracked s {:.4} o ({:.2},{:.2})  truth s {:.4} o ({:.2},{:.2})  corner err {err:.2} px", v.scale, v.offset.0, v.offset.1, truth.scale, truth.offset.0, truth.offset.1);
        }
    }
    println!("worst corner error {worst:.2} px");
}
