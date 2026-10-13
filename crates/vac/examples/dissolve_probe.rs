//! Dev probe: the blend test behind dissolve detection, frame by frame.
fn main() {
    let path = std::env::args().nth(1).expect("video");
    let frames = vac::load_video_frames(&path).unwrap();
    let n = frames.len();
    let w = frames[0].width;
    let f = (w / 160).max(1) as usize;
    let t: Vec<Vec<f64>> = frames.iter().map(|fr| {
        let (fw, fh) = (fr.width as usize / f, fr.height as usize / f);
        let mut v = Vec::with_capacity(fw * fh * 3);
        for y in 0..fh { for x in 0..fw { let p = ((y * f) * fr.width as usize + x * f) * 4; v.extend_from_slice(&[fr.pixels[p] as f64, fr.pixels[p+1] as f64, fr.pixels[p+2] as f64]); } }
        v
    }).collect();
    let mse = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>() / a.len() as f64;
    for k in [8usize, 16, 32, 64] {
        if 3 * k > n { continue; }
        let across: Vec<f64> = (0..n - k).map(|i| mse(&t[i], &t[i + k])).collect();
        let mut s = across.clone(); s.sort_by(|a, b| a.total_cmp(b));
        let med = s[s.len() / 2];
        let (bi, bv) = across.iter().enumerate().fold((0, 0.0), |m, (i, &v)| if v > m.1 { (i, v) } else { m });
        // blend residual at the best window
        let (p, q) = (&t[bi], &t[bi + k]);
        let d: Vec<f64> = p.iter().zip(q).map(|(x, y)| y - x).collect();
        let dd: f64 = d.iter().map(|v| v * v).sum::<f64>();
        let mut resid = 0.0; let mut betas = vec![];
        for u in bi + 1..bi + k { let fu = &t[u]; let beta = fu.iter().zip(p).zip(&d).map(|((a, b), c)| (a - b) * c).sum::<f64>() / dd; let r: f64 = fu.iter().zip(p).zip(&d).map(|((a, b), c)| (a - b - beta * c).powi(2)).sum::<f64>(); resid += r / dd; betas.push(beta); }
        let maxstep = (bi..bi + k).map(|i| mse(&t[i], &t[i + 1])).fold(0.0, f64::max);
        println!("k {k:2}: median {med:8.1}  best at {bi:3} = {bv:8.1} ({:.1}x)  maxstep {maxstep:8.1}  resid {:.3}  beta {:.2}..{:.2}", bv / med.max(1e-9), resid / (k - 1) as f64, betas.first().unwrap(), betas.last().unwrap());
    }
}
