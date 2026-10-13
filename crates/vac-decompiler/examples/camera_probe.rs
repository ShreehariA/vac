//! Dev probe: track the camera, build the mosaic, and score the warped
//! mosaic against every frame. Usage: camera_probe <video>
use vac_decompiler::camera;

fn main() {
    let path = std::env::args().nth(1).expect("video path");
    let frames = if path.ends_with(".gif") {
        vac_decompiler::frames::load_gif(std::path::Path::new(&path)).unwrap()
    } else {
        vac_decompiler::frames_mp4::load_mp4(std::path::Path::new(&path)).unwrap()
    };
    let (w, h) = (frames[0].width, frames[0].height);
    let t0 = std::time::Instant::now();
    let views = camera::track(&frames, 0);
    println!("tracked {} frames in {:.2?}; motion {:.1} px", frames.len(), t0.elapsed(), camera::motion_px(&views, w, h));
    let sample: Vec<usize> = (0..frames.len()).step_by((frames.len() / 12).max(1)).collect();
    println!("support {:.3}", camera::support(&frames, 0, &views, &sample));
    let t1 = std::time::Instant::now();
    let m = camera::mosaic(&frames, &views, [0, 0, 0]).expect("mosaic");
    println!("mosaic {}x{} at ({}, {}) in {:.2?}", m.w, m.h, m.x, m.y, t1.elapsed());
    let mut total = 0f64;
    for (t, f) in frames.iter().enumerate() {
        let img = camera::warp(&m, views[t], w, h);
        let mut s = 0f64;
        for (a, b) in f.pixels.chunks(4).zip(img.chunks(4)) {
            for c in 0..3 {
                s += (a[c] as f64 - b[c] as f64).powi(2);
            }
        }
        let mse = s / (3.0 * (w * h) as f64);
        total += mse;
        if t % 10 == 0 {
            let v = views[t];
            println!("frame {t:3}: scale {:.4} offset ({:7.2},{:7.2})  {:.2} dB", v.scale, v.offset.0, v.offset.1, 10.0 * (65025.0 / mse.max(1e-9)).log10());
        }
    }
    let mse = total / frames.len() as f64;
    println!("mean residual {:.1} = {:.2} dB", mse, 10.0 * (65025.0 / mse).log10());
}
