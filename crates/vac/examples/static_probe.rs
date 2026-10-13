//! Dev probe: decompile frames [a, b) of a video with and without the
//! static layer, and print each result's PSNR, IoU and shapes.
//! Usage: static_probe <video> <a> <b>
use vac::{DecompileOptions, Statement};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let all = vac::load_video_frames(&a[1]).unwrap();
    let (x, y): (usize, usize) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let frames = all[x..y.min(all.len())].to_vec();
    for static_layer in [true, false] {
        let opts = DecompileOptions { static_layer, ..Default::default() };
        let t = std::time::Instant::now();
        let doc = vac::decompile_frames(frames.clone(), &opts).unwrap();
        let fid = vac::measure(&frames, &doc).unwrap();
        let names: Vec<String> = doc
            .scenes
            .iter()
            .flat_map(|s| &s.statements)
            .filter_map(|st| match st {
                Statement::Let { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        println!(
            "static_layer={static_layer}: {:.2} dB, IoU {:.4}, {} bytes, {:.1}s, shapes {:?}",
            fid.psnr_db,
            fid.ink_iou,
            vac::to_string(&doc).len(),
            t.elapsed().as_secs_f32(),
            names
        );
    }
}
