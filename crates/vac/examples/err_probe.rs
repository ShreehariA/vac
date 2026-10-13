//! Dev probe: where a frame's error lives. For frame k of a video against a
//! .vac: PSNR at full resolution and after 4×4 box-averaging (detail vs
//! colour), the best global sub-pixel shift of the render (registration),
//! and whether sharpening or blurring the render helps (resampling).
//! Writes source | render | 4×|error| to the given PNG.
//! Usage: err_probe <video> <file.vac> <frame> <out.png>
fn psnr(a: &[f32], b: &[f32]) -> f64 {
    let mse: f64 = a.iter().zip(b).map(|(x, y)| ((x - y) as f64).powi(2)).sum::<f64>() / a.len() as f64;
    10.0 * (65025.0 / mse.max(1e-9)).log10()
}
fn rgb(px: &[u8]) -> Vec<f32> {
    px.chunks_exact(4).flat_map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect()
}
fn at(img: &[f32], w: usize, h: usize, x: f32, y: f32, c: usize) -> f32 {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let g = |xx: usize, yy: usize| img[(yy * w + xx) * 3 + c];
    (g(x0, y0) * (1.0 - fx) + g(x1, y0) * fx) * (1.0 - fy) + (g(x0, y1) * (1.0 - fx) + g(x1, y1) * fx) * fy
}
fn shifted(img: &[f32], w: usize, h: usize, dx: f32, dy: f32) -> Vec<f32> {
    let mut o = vec![0.0; img.len()];
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                o[(y * w + x) * 3 + c] = at(img, w, h, x as f32 - dx, y as f32 - dy, c);
            }
        }
    }
    o
}
fn blur3(img: &[f32], w: usize, h: usize) -> Vec<f32> {
    let k = [0.25f32, 0.5, 0.25];
    let mut t = vec![0.0; img.len()];
    let mut o = vec![0.0; img.len()];
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                t[(y * w + x) * 3 + c] = (0..3).map(|i| k[i] * img[(y * w + (x + i).saturating_sub(1).min(w - 1)) * 3 + c]).sum();
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                o[(y * w + x) * 3 + c] = (0..3).map(|i| k[i] * t[((y + i).saturating_sub(1).min(h - 1) * w + x) * 3 + c]).sum();
            }
        }
    }
    o
}
fn box4(img: &[f32], w: usize, h: usize) -> Vec<f32> {
    let (bw, bh) = (w / 4, h / 4);
    let mut o = vec![0.0; bw * bh * 3];
    for by in 0..bh {
        for bx in 0..bw {
            for c in 0..3 {
                let mut s = 0.0;
                for y in 0..4 {
                    for x in 0..4 {
                        s += img[((by * 4 + y) * w + bx * 4 + x) * 3 + c];
                    }
                }
                o[(by * bw + bx) * 3 + c] = s / 16.0;
            }
        }
    }
    o
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let frames = vac::load_video_frames(&a[1]).unwrap();
    let doc = vac::read(&a[2]).unwrap();
    let k: usize = a[3].parse().unwrap();
    let rendered = vac::render(&doc).unwrap();
    let f = &frames[k];
    let r = &rendered[k.min(rendered.len() - 1)];
    let (w, h) = (f.width as usize, f.height as usize);
    let (s, rr) = (rgb(&f.pixels), rgb(&r.rgba));
    println!("frame {k}: psnr {:.2}", psnr(&s, &rr));
    println!("  4x4-averaged psnr {:.2}  (colour/brightness only)", psnr(&box4(&s, w, h), &box4(&rr, w, h)));
    let mut best = (psnr(&s, &rr), 0.0f32, 0.0f32);
    for iy in -8..=8 {
        for ix in -8..=8 {
            let (dx, dy) = (ix as f32 * 0.125, iy as f32 * 0.125);
            let p = psnr(&s, &shifted(&rr, w, h, dx, dy));
            if p > best.0 {
                best = (p, dx, dy);
            }
        }
    }
    println!("  best shift ({:+.3}, {:+.3}) psnr {:.2}", best.1, best.2, best.0);
    let b = blur3(&rr, w, h);
    println!("  render blurred psnr {:.2}", psnr(&s, &b));
    let sharp: Vec<f32> = rr.iter().zip(&b).map(|(x, y)| (x + 0.7 * (x - y)).clamp(0.0, 255.0)).collect();
    println!("  render sharpened psnr {:.2}", psnr(&s, &sharp));
    println!("  source vs its own blur {:.2}  (how much detail there is)", psnr(&s, &blur3(&s, w, h)));
    let mut out = image::RgbImage::new((w * 3) as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 3;
            let e: Vec<u8> = (0..3).map(|c| ((s[i + c] - rr[i + c]).abs() * 4.0).min(255.0) as u8).collect();
            out.put_pixel(x as u32, y as u32, image::Rgb([s[i] as u8, s[i + 1] as u8, s[i + 2] as u8]));
            out.put_pixel((w + x) as u32, y as u32, image::Rgb([rr[i] as u8, rr[i + 1] as u8, rr[i + 2] as u8]));
            out.put_pixel((2 * w + x) as u32, y as u32, image::Rgb([e[0], e[1], e[2]]));
        }
    }
    out.save(&a[4]).unwrap();
}
