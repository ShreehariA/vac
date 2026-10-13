//! Dev probe: the `vac verify --dump` strip (source | reconstruction | ink
//! map) for one chosen frame of a video against a .vac, with its scores.
//! Usage: strip_probe <video> <file.vac> <frame> <out.png>
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let frames = vac::load_video_frames(&a[1]).unwrap();
    let doc = vac::read(&a[2]).unwrap();
    let i: usize = a[3].parse().unwrap();
    let fid = vac::measure(&frames, &doc).unwrap();
    let recon = vac::render_aligned(&frames, &doc).unwrap();
    let bg = vac::ink_background(&frames);
    let opts = vac::CompareOptions::default();
    let (src, rec) = (&frames[i], &recon[i]);
    let (w, h) = (src.width, src.height);
    let mut out = image::RgbImage::new(w * 3 + 8, h);
    for y in 0..h {
        for x in 0..w {
            let k = ((y * w + x) * 4) as usize;
            let s = &src.pixels[k..k + 4];
            let r = &rec.rgba[k..k + 4];
            out.put_pixel(x, y, image::Rgb([s[0], s[1], s[2]]));
            out.put_pixel(w + 4 + x, y, image::Rgb([r[0], r[1], r[2]]));
            let ink = if s[3] < 128 {
                [70, 70, 76]
            } else {
                match (vac::is_ink(s, bg, &opts), vac::is_ink(r, bg, &opts)) {
                    (true, true) => [235, 235, 235],
                    (true, false) => [230, 50, 50],
                    (false, true) => [60, 110, 240],
                    (false, false) => [24, 24, 28],
                }
            };
            out.put_pixel(2 * w + 8 + x, y, image::Rgb(ink));
        }
    }
    let k = (480 / w.max(1)).clamp(1, 12);
    let out = if k > 1 {
        image::imageops::resize(&out, out.width() * k, out.height() * k, image::imageops::FilterType::Nearest)
    } else {
        out
    };
    out.save(&a[4]).unwrap();
    let s = &fid.frames[i];
    println!("frame {i}: IoU {:.4}, psnr {:.2} dB; clip {:.2} dB, IoU {:.4}", s.ink_iou, s.psnr_db, fid.psnr_db, fid.ink_iou);
}
