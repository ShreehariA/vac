//! Print the hermetic round trip of a `.vac` fixture: render it in memory,
//! decompile the frames, and print the recovered source and its fidelity.
//!
//! ```text
//! cargo run --release -p vac --example hermetic_rt -- examples/spin.vac
//! ```
fn main() {
    let path = std::env::args().nth(1).expect("usage: hermetic_rt <file.vac>");
    let doc = vac::read(&path).expect("read");
    let frames = vac::rendered_to_video_frames(&vac::render(&doc).expect("render"), doc.canvas.fps);
    let recovered = vac::decompile_frames(frames.clone(), &vac::DecompileOptions::default()).expect("decompile");
    let fid = vac::measure(&frames, &recovered).expect("measure");
    println!("{}", vac::to_string(&recovered));
    eprintln!("{fid}");
    if std::env::var("VAC_FRAMES").is_ok() {
        for (i, f) in fid.frames.iter().enumerate() {
            eprintln!("  frame {i:3}  IoU {:.4}  residual {:8.2}", f.ink_iou, f.residual);
        }
    }
}
