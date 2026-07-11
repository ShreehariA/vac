//! Moore-neighbour contour tracing of a region's outer boundary.
//!
//! Given a per-pixel `region_labels` array (one label per connected
//! component, dense in 1..N) and a [`LabelledRegion`] picked out of
//! [`RegionsResult`], walk clockwise from the region's topmost-leftmost
//! pixel and collect the boundary in pixel coordinates.
//!
//! Stopping criterion: Jacob's rule — return to the same pixel *and*
//! the same incoming direction (`entry_dir`).  Just `cur == start` is
//! insufficient on figure-eight or hairpin shapes where the tracer
//! crosses its own start pixel mid-walk; the v0.2 version of this
//! module fixes that.
//!
//! The walker compares `region_labels[i] == region.label`, never
//! `cluster_labels[i] == region.cluster`, so two disjoint regions of
//! the same palette colour can never bleed into one contour.

use crate::regions::LabelledRegion;

/// Moore-neighbour offsets, clockwise starting at "left" (W-NW-N-NE-E-SE-S-SW).
const OFFSETS: [(i32, i32); 8] = [
    (-1, 0), (-1, -1), (0, -1), (1, -1),
    (1, 0),  (1, 1),   (0, 1),  (-1, 1),
];

/// Trace the outer contour of `region` in `region_labels`.  Returns
/// boundary pixels in clockwise order.  Coordinates are pixel
/// integers cast to `f32` so the simplify and curve-fitting stages
/// can run in fractional pixels later.
pub fn trace_outer_contour(
    region_labels: &[u32],
    w: u32,
    h: u32,
    region: &LabelledRegion,
) -> Option<Vec<(f32, f32)>> {
    let target = region.label;
    let stride = w as usize;
    let n = (w * h) as usize;
    debug_assert_eq!(region_labels.len(), n);

    let inside = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return false;
        }
        let i = (y as usize) * stride + x as usize;
        region_labels[i] == target
    };

    // Topmost-leftmost pixel within the bounding box that belongs to
    // this region — guaranteed boundary pixel.
    let mut start: Option<(i32, i32)> = None;
    'outer: for y in (region.min_y as i32)..=(region.max_y as i32) {
        for x in (region.min_x as i32)..=(region.max_x as i32) {
            if inside(x, y) {
                start = Some((x, y));
                break 'outer;
            }
        }
    }
    let start = start?;
    let (sx, sy) = start;

    // Single-pixel region: emit a 4-vertex micro-square.
    if !inside(sx + 1, sy) && !inside(sx, sy + 1) && !inside(sx - 1, sy) && !inside(sx, sy - 1) {
        let x = sx as f32;
        let y = sy as f32;
        return Some(vec![(x, y), (x + 1.0, y), (x + 1.0, y + 1.0), (x, y + 1.0)]);
    }

    // Pretend we arrived at `start` from the west — by construction
    // the topmost-leftmost pixel has no inside neighbour to its left,
    // so this is a valid "previous" direction.
    let mut entry_dir: usize = 0; // OFFSETS index for W
    let mut cur = start;

    let mut contour: Vec<(f32, f32)> = Vec::with_capacity(64);
    contour.push((sx as f32, sy as f32));

    // Jacob's stopping criterion: record the (pixel, entry_dir) state
    // after the very first step.  When we re-encounter the same
    // state, the loop has closed cleanly.  This is robust on
    // figure-eight / hairpin shapes that legally re-visit `start`
    // mid-walk, where naive `cur == start` would stop early.
    let mut first_state: Option<((i32, i32), usize)> = None;

    // Bound: at most 8 visits per pixel before we give up (worst case
    // for very thin / hairpin shapes).
    let max_steps = (region.area as usize).saturating_mul(8) + 64;

    for _step in 0..max_steps {
        // Scan Moore-neighbours clockwise, starting one step after
        // the entry direction.
        let mut found_next = false;
        for k in 1..=8 {
            let idx = (entry_dir + k) % 8;
            let (dx, dy) = OFFSETS[idx];
            let nx = cur.0 + dx;
            let ny = cur.1 + dy;
            if inside(nx, ny) {
                // New entry direction = direction from (nx, ny) back to cur.
                let back = (idx + 4) % 8;
                entry_dir = back;
                cur = (nx, ny);

                match first_state {
                    None => {
                        first_state = Some((cur, entry_dir));
                        contour.push((nx as f32, ny as f32));
                    }
                    Some(fs) if (cur, entry_dir) == fs => {
                        // Closed the loop — don't push the duplicate.
                        return finalize(contour);
                    }
                    Some(_) => {
                        contour.push((nx as f32, ny as f32));
                    }
                }
                found_next = true;
                break;
            }
        }
        if !found_next {
            // Truly isolated pixel — fall back to micro-square.
            return Some(vec![
                (sx as f32, sy as f32),
                (sx as f32 + 1.0, sy as f32),
                (sx as f32 + 1.0, sy as f32 + 1.0),
                (sx as f32, sy as f32 + 1.0),
            ]);
        }
    }

    // Safety fallback: ran out of step budget.  Return what we have
    // if it's at least a valid polygon; otherwise abort.
    finalize(contour)
}

fn finalize(mut contour: Vec<(f32, f32)>) -> Option<Vec<(f32, f32)>> {
    // Drop any trailing duplicate of the first vertex.
    if contour.len() >= 2 && contour.first() == contour.last() {
        contour.pop();
    }
    if contour.len() < 3 {
        return None;
    }
    Some(contour)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regions::label_regions_per_cluster_full;

    fn mk_labels(w: u32, h: u32, mask: impl Fn(u32, u32) -> bool) -> Vec<u32> {
        let mut v = vec![0u32; (w * h) as usize];
        for y in 0..h { for x in 0..w {
            v[(y * w + x) as usize] = if mask(x, y) { 1 } else { 0 };
        } }
        v
    }

    #[test]
    fn traces_simple_rectangle() {
        let w = 20u32; let h = 12u32;
        let labels = mk_labels(w, h, |x, y| (5..=14).contains(&x) && (3..=8).contains(&y));
        let res = label_regions_per_cluster_full(&labels, w, h, 1);
        let r = res.regions.iter().find(|r| r.cluster == 1).expect("region");
        let contour = trace_outer_contour(&res.region_labels, w, h, r).expect("contour");
        assert!(contour.len() >= 20 && contour.len() <= 36, "len={} contour={contour:?}", contour.len());
        assert_eq!(contour[0], (5.0, 3.0));
    }

    #[test]
    fn keeps_two_same_color_regions_separate_after_fix() {
        // Two disjoint 4x4 squares of cluster 1, separated by 2 cells of cluster 0.
        let w = 20u32; let h = 8u32;
        let labels = mk_labels(w, h, |x, y| {
            ((2..=5).contains(&x) && (2..=5).contains(&y))
                || ((10..=13).contains(&x) && (2..=5).contains(&y))
        });
        let res = label_regions_per_cluster_full(&labels, w, h, 1);
        let cluster1: Vec<_> = res.regions.iter().filter(|r| r.cluster == 1).collect();
        assert_eq!(cluster1.len(), 2);
        // Both contours must be 4×4-rect-shaped (perimeter ≈ 12), not a combined contour spanning both.
        for r in cluster1 {
            let contour = trace_outer_contour(&res.region_labels, w, h, r).expect("contour");
            // Bounding box of contour must be <=5 wide (one rect, not both).
            let max_x = contour.iter().map(|p| p.0 as i32).max().unwrap();
            let min_x = contour.iter().map(|p| p.0 as i32).min().unwrap();
            assert!(max_x - min_x <= 4, "contour spans cols {min_x}..{max_x} — leaked into the other region");
        }
    }

    #[test]
    fn handles_single_pixel_region() {
        let w = 6u32; let h = 6u32;
        let labels = mk_labels(w, h, |x, y| x == 3 && y == 3);
        let res = label_regions_per_cluster_full(&labels, w, h, 1);
        let r = res.regions.iter().find(|r| r.cluster == 1).expect("region");
        let contour = trace_outer_contour(&res.region_labels, w, h, r).expect("contour");
        // Single-pixel fallback: 4 vertices forming a micro-square.
        assert_eq!(contour.len(), 4);
    }
}
