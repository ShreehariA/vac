//! Feed the pipeline its own output, repeatedly, and print each
//! generation's fidelity against the original plus its shape geometry.
//!
//! ```text
//! cargo run --release -p vac --example generations -- examples/two_balls.vac 6
//! ```
fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: generations <file.vac> [n]");
    let n: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(5);
    let original = vac::read(&path).expect("read");
    let gen1 = vac::rendered_to_video_frames(&vac::render(&original).unwrap(), original.canvas.fps);
    let mut current = original;
    for g in 1..=n {
        let frames = vac::rendered_to_video_frames(&vac::render(&current).unwrap(), current.canvas.fps);
        current = vac::decompile_frames(frames, &vac::DecompileOptions::default()).unwrap();
        let fid = vac::measure(&gen1, &current).unwrap();
        let shapes: Vec<String> = current
            .scenes
            .iter()
            .flat_map(|s| &s.statements)
            .filter_map(|st| match st {
                vac::Statement::Let { name, shape } if name != "bg" => Some(match shape {
                    vac::Shape::Rect { w, h, .. } => format!("rect {w}x{h}"),
                    vac::Shape::Ellipse { rx, ry, .. } => format!("ellipse {rx}x{ry}"),
                    vac::Shape::Path(p) => format!("path {}", p.len()),
                    vac::Shape::Image { w, h, .. } => format!("image {w}x{h}"),
                }),
                _ => None,
            })
            .collect();
        println!("gen {g}: IoU {:.4} residual {:7.2}  {}", fid.ink_iou, fid.residual, shapes.join(", "));
    }
}
