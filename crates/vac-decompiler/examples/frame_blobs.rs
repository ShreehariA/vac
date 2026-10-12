//! Print every blob the detector finds in one frame of a clip.
//!
//! ```text
//! cargo run --release -p vac-decompiler --example frame_blobs -- clip.mp4 45 [regions|components]
//! ```
use vac_decompiler::detect::{detect_all, Background, Segmentation};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: frame_blobs <clip> <frame> [regions|components]");
    let fi: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(0);
    let seg = match args.next().as_deref() {
        Some("components") => Segmentation::Components,
        _ => Segmentation::Regions,
    };
    let p = std::path::Path::new(&path);
    let frames = if p.extension().is_some_and(|e| e == "gif") {
        vac_decompiler::frames::load_gif(p).expect("load")
    } else {
        vac_decompiler::frames_mp4::load_mp4(p).expect("load")
    };
    let d = &detect_all(&frames[fi..=fi], seg, Background::Corners, 9)[0];
    println!("bg {:?}", d.bg);
    for b in &d.blobs {
        println!(
            "c=({:.1},{:.1}) bbox {:.0}x{:.0} area {} hole {} soft {:.2}x{:.2} soft_area {:.1} fit {:.3} filled r{:.3}/e{:.3} {:?} {:?}",
            b.cx, b.cy, 2.0 * b.rx, 2.0 * b.ry, b.area, b.hole_area, b.soft_w, b.soft_h, b.soft_area,
            b.fit_iou, b.filled_iou_rect, b.filled_iou_ellipse, b.kind, b.color
        );
    }
}
