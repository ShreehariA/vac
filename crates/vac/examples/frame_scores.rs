//! Dev probe: per-frame PSNR of a .vac against a video.
//! Usage: frame_scores <video> <file.vac>
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let frames = vac::load_video_frames(&a[1]).unwrap();
    let doc = vac::read(&a[2]).unwrap();
    let f = vac::measure(&frames, &doc).unwrap();
    let mut line = String::new();
    for (i, s) in f.frames.iter().enumerate() {
        line.push_str(&format!("{:3}:{:5.1} ", i, s.psnr_db.min(99.0)));
        if i % 10 == 9 { println!("{line}"); line.clear(); }
    }
    if !line.is_empty() { println!("{line}"); }
}
