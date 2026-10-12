//! How faithful would a raster of the static scene alone be? Scores the
//! clip's temporal-median frame against every source frame (no movers).
//!
//! ```text
//! cargo run --release -p vac-decompiler --example plate_psnr -- clip.gif
//! ```
fn main() {
    let path = std::env::args().nth(1).expect("usage: plate_psnr <clip>");
    let p = std::path::Path::new(&path);
    let frames = if p.extension().is_some_and(|e| e == "gif") {
        vac_decompiler::frames::load_gif(p).expect("load")
    } else {
        vac_decompiler::frames_mp4::load_mp4(p).expect("load")
    };
    let plate = vac_decompiler::layer::median_frame(&frames).expect("median");
    let (mut sum, mut n) = (0f64, 0u64);
    for f in &frames {
        for i in 0..(f.width * f.height) as usize {
            if f.pixels[i * 4 + 3] < 128 {
                continue;
            }
            for c in 0..3 {
                let d = f.pixels[i * 4 + c] as f64 - plate.pixels[i * 4 + c] as f64;
                sum += d * d;
            }
            n += 3;
        }
    }
    let mse = sum / n as f64;
    println!("{path}: median-plate residual {mse:.1}, PSNR {:.2} dB", 10.0 * (255.0f64 * 255.0 / mse).log10());
}
