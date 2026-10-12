//! Print the blobs the static layer's plate yields for a clip — the
//! measurements behind every `static_<n>` shape.
//!
//! ```text
//! cargo run --release -p vac-decompiler --example plate_blobs -- clip.gif
//! ```
use vac_decompiler::detect::{detect_all, Background, Segmentation};
use vac_decompiler::layer::StaticLayer;

fn main() {
    let path = std::env::args().nth(1).expect("usage: plate_blobs <clip>");
    let p = std::path::Path::new(&path);
    let frames = if p.extension().is_some_and(|e| e == "gif") {
        vac_decompiler::frames::load_gif(p).expect("load")
    } else {
        vac_decompiler::frames_mp4::load_mp4(p).expect("load")
    };
    let layer = StaticLayer::measure(&frames, Background::Corners).expect("no static layer");
    let plate = layer.plate();
    println!("plate bg {:?}", layer.bg);
    for d in detect_all(&[plate], Segmentation::Components, Background::Corners, 9) {
        for b in d.blobs {
            println!(
                "blob c=({:.1},{:.1}) bbox {:.0}x{:.0} soft {:.2}x{:.2} fit {:.3} {:?} colour {:?}",
                b.cx, b.cy, 2.0 * b.rx, 2.0 * b.ry, b.soft_w, b.soft_h, b.fit_iou, b.kind, b.color
            );
        }
    }
}
