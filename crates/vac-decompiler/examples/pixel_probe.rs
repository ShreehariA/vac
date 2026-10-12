//! Print a clip's background estimates and a few pixel values of a frame.
//!
//! ```text
//! cargo run --release -p vac-decompiler --example pixel_probe -- clip.gif [frame]
//! ```
use vac_decompiler::detect::{background_of, Background};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: pixel_probe <clip> [frame]");
    let fi: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(0);
    let p = std::path::Path::new(&path);
    let frames = if p.extension().is_some_and(|e| e == "gif") {
        vac_decompiler::frames::load_gif(p).expect("load")
    } else {
        vac_decompiler::frames_mp4::load_mp4(p).expect("load")
    };
    let f = &frames[fi];
    let (w, h) = (f.width as usize, f.height as usize);
    println!("{} frames, {}x{}", frames.len(), w, h);
    println!("corners bg {:?}", background_of(f, Background::Corners));
    println!("border  bg {:?}", background_of(f, Background::Border));
    let px = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        (f.pixels[i], f.pixels[i + 1], f.pixels[i + 2], f.pixels[i + 3])
    };
    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1), (2, 2), (20, 60), (w / 2, h / 2), (w / 2, 8), (100, 600)] {
        println!("  ({x},{y}) {:?}", px(x, y));
    }
}
