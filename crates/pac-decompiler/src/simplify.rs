//! Ramer-Douglas-Peucker simplification for closed polygon contours.
//!
//! Classic 2D recursive RDP, adapted for closed loops by treating
//! the polygon as two half-paths from the farthest two anchor points.
//! Tolerance `epsilon` is in pixels: typical values are 1.0 (preserve
//! every visible corner) to 4.0 (aggressive simplification, blocky
//! silhouettes).

/// Simplify a closed polygon outline.  The output is *not* explicitly
/// closed — the last vertex is not repeated as the first — so the
/// PAC writer's implicit-close convention applies.
pub fn rdp(points: &[(f32, f32)], epsilon: f32) -> Vec<(f32, f32)> {
    if points.len() < 4 {
        return points.to_vec();
    }
    let eps = epsilon.max(0.01);

    // Pick the two anchor points as the pair maximally apart along
    // the polygon's bounding box diagonal.  Splitting at these gives
    // a stable two-half segmentation across regenerations.
    let (mut min_x, mut min_y, mut max_x, mut max_y) =
        (points[0].0, points[0].1, points[0].0, points[0].1);
    let mut idx_min_x = 0usize;
    let mut idx_max_x = 0usize;
    for (i, p) in points.iter().enumerate() {
        if p.0 < min_x { min_x = p.0; idx_min_x = i; }
        if p.0 > max_x { max_x = p.0; idx_max_x = i; }
        if p.1 < min_y { min_y = p.1; }
        if p.1 > max_y { max_y = p.1; }
    }
    let _ = (min_y, max_y);
    let a = idx_min_x.min(idx_max_x);
    let b = idx_min_x.max(idx_max_x);
    if a == b {
        // Degenerate — single-column region.  Fall back to bbox corners.
        return vec![
            (min_x, points[0].1),
            (max_x, points[0].1),
        ];
    }

    let half1 = &points[a..=b];
    let mut half2: Vec<(f32, f32)> = points[b..].to_vec();
    half2.extend_from_slice(&points[..=a]);

    let s1 = rdp_line(half1, eps);
    let s2 = rdp_line(&half2, eps);

    // Merge — drop the duplicated junction vertices.
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(s1.len() + s2.len());
    out.extend(s1.iter().copied());
    if s2.len() > 1 {
        out.extend(s2[1..s2.len() - 1].iter().copied());
    }
    out
}

/// Standard 2D RDP on an open polyline.  Both endpoints are kept.
fn rdp_line(points: &[(f32, f32)], epsilon: f32) -> Vec<(f32, f32)> {
    let n = points.len();
    if n < 3 { return points.to_vec(); }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    rdp_recursive(points, 0, n - 1, epsilon, &mut keep);
    let mut out = Vec::with_capacity(keep.iter().filter(|&&b| b).count());
    for (i, &k) in keep.iter().enumerate() {
        if k { out.push(points[i]); }
    }
    out
}

fn rdp_recursive(points: &[(f32, f32)], start: usize, end: usize, eps: f32, keep: &mut [bool]) {
    if end <= start + 1 { return; }
    let (ax, ay) = points[start];
    let (bx, by) = points[end];
    let dx = bx - ax;
    let dy = by - ay;
    let denom = (dx * dx + dy * dy).sqrt().max(1e-6);

    let mut best_idx = start;
    let mut best_d = 0.0f32;
    for i in (start + 1)..end {
        let (px, py) = points[i];
        let d = ((dy * px - dx * py) + (bx * ay - by * ax)).abs() / denom;
        if d > best_d {
            best_d = d;
            best_idx = i;
        }
    }
    if best_d > eps {
        keep[best_idx] = true;
        rdp_recursive(points, start, best_idx, eps, keep);
        rdp_recursive(points, best_idx, end, eps, keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_corners_of_a_rectangle() {
        // 4-corner rect, with intermediate samples on each edge.
        let pts = vec![
            (0.0, 0.0), (5.0, 0.0), (10.0, 0.0),
            (10.0, 5.0), (10.0, 10.0),
            (5.0, 10.0), (0.0, 10.0),
            (0.0, 5.0),
        ];
        let simp = rdp(&pts, 0.5);
        assert!(simp.len() <= 5 && simp.len() >= 4, "got {simp:?}");
    }

    #[test]
    fn drops_collinear_midpoints() {
        let pts: Vec<(f32, f32)> = (0..20).map(|i| (i as f32, 0.0)).collect();
        let simp = rdp_line(&pts, 0.5);
        assert_eq!(simp.len(), 2);
    }
}
