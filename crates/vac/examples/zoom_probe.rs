//! Dev probe: how much of a clip is one frame under a camera zoom?
//! For each frame t, search the zoom z and centre c that best map frame
//! `ref` onto frame t (bilinear), and print the PSNR of that warp.
//! Usage: zoom_probe <video> [ref_frame]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let frames = vac::load_video_frames(&args[1]).expect("load");
    let r = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(0usize);
    let (w, h) = (frames[0].width as usize, frames[0].height as usize);
    let src = &frames[r].pixels;
    let sample = |x: f32, y: f32, c: usize| -> f32 {
        let (x, y) = (x.clamp(0.0, (w - 1) as f32), y.clamp(0.0, (h - 1) as f32));
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let p = |xx: usize, yy: usize| src[(yy * w + xx) * 4 + c] as f32;
        (p(x0, y0) * (1.0 - fx) + p(x1, y0) * fx) * (1.0 - fy) + (p(x0, y1) * (1.0 - fx) + p(x1, y1) * fx) * fy
    };
    // Score on a 4x-subsampled grid for the search, full grid for the report.
    let mse = |f: &[u8], z: f32, cx: f32, cy: f32, stride: usize| -> f64 {
        let mut s = 0f64;
        let mut n = 0f64;
        for y in (0..h).step_by(stride) {
            for x in (0..w).step_by(stride) {
                let (sx, sy) = (cx + (x as f32 - cx) / z, cy + (y as f32 - cy) / z);
                for c in 0..3 {
                    let d = f[(y * w + x) * 4 + c] as f32 - sample(sx, sy, c);
                    s += (d * d) as f64;
                }
                n += 3.0;
            }
        }
        s / n
    };
    let psnr = |m: f64| 10.0 * (255.0f64 * 255.0 / m.max(1e-9)).log10();
    let mut prev_z = 1.0f32;
    for t in (0..frames.len()).step_by(6) {
        let f = &frames[t].pixels;
        let mut best = (f64::MAX, 1.0f32, w as f32 / 2.0, h as f32 / 2.0);
        // zoom search around the previous estimate, centre on a small grid
        for zi in -40..=40 {
            let z = (prev_z * (1.0 + zi as f32 * 0.004)).max(0.2);
            for (dx, dy) in [(0.0f32, 0.0f32), (-4.0, 0.0), (4.0, 0.0), (0.0, -4.0), (0.0, 4.0)] {
                let (cx, cy) = (w as f32 / 2.0 + dx, h as f32 / 2.0 + dy);
                let m = mse(f, z, cx, cy, 4);
                if m < best.0 {
                    best = (m, z, cx, cy);
                }
            }
        }
        prev_z = best.1;
        let full = mse(f, best.1, best.2, best.3, 1);
        let ident = mse(f, 1.0, w as f32 / 2.0, h as f32 / 2.0, 1);
        println!(
            "frame {t:3}: zoom {:.4} centre ({:.0},{:.0})  warp {:.2} dB   (unwarped {:.2} dB)",
            best.1, best.2, best.3, psnr(full), psnr(ident)
        );
    }
}
