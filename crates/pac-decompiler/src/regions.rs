//! For each colour cluster, run 4-connected component labelling and
//! keep the regions with `area >= min_area`.  Each surviving region
//! carries its cluster id (the palette index) so the decompiler
//! knows what colour to fill it with.

#[derive(Debug, Clone)]
pub struct LabelledRegion {
    /// Palette index (0..palette.len()).
    pub cluster: u32,
    pub area: u32,
    /// Bounding box (inclusive).
    pub min_x: u32,
    pub max_x: u32,
    pub min_y: u32,
    pub max_y: u32,
    /// Any pixel guaranteed to lie inside the region.  Used to seed
    /// the contour tracer.
    pub seed_x: u32,
    pub seed_y: u32,
    /// Per-region label, dense in 1..N. Pixels with the same label
    /// belong to the same region.  Index = y * width + x.  Stored
    /// here so the contour tracer doesn't have to re-run CCL.
    pub label: u32,
}

pub struct RegionsResult {
    pub regions: Vec<LabelledRegion>,
    /// Per-pixel region label (0 = background / dropped).  Same
    /// row-major indexing as `labels` from the K-means stage.
    pub region_labels: Vec<u32>,
}

pub fn label_regions_per_cluster(
    cluster_labels: &[u32],
    w: u32,
    h: u32,
    min_area: u32,
) -> Vec<LabelledRegion> {
    let res = label_regions_per_cluster_full(cluster_labels, w, h, min_area);
    res.regions
}

pub fn label_regions_per_cluster_full(
    cluster_labels: &[u32],
    w: u32,
    h: u32,
    min_area: u32,
) -> RegionsResult {
    let n = (w * h) as usize;
    assert_eq!(cluster_labels.len(), n);

    let mut region_labels = vec![0u32; n];
    let mut uf = UnionFind::new();
    let mut next: u32 = 1;

    // Pass 1: assign provisional labels with 4-connectivity, only
    // merging neighbours that share the same *cluster* id.
    for y in 0..h as usize {
        for x in 0..w as usize {
            let i = y * w as usize + x;
            let cur_cluster = cluster_labels[i];
            let left = if x > 0 && cluster_labels[i - 1] == cur_cluster {
                region_labels[i - 1]
            } else {
                0
            };
            let up = if y > 0 && cluster_labels[i - w as usize] == cur_cluster {
                region_labels[i - w as usize]
            } else {
                0
            };
            match (left, up) {
                (0, 0) => {
                    region_labels[i] = next;
                    uf.make_set(next);
                    next += 1;
                }
                (a, 0) => region_labels[i] = a,
                (0, b) => region_labels[i] = b,
                (a, b) => {
                    let m = a.min(b);
                    region_labels[i] = m;
                    if a != b {
                        uf.union(a, b);
                    }
                }
            }
        }
    }
    // Pass 2: resolve every provisional label to its root.
    for l in region_labels.iter_mut() {
        if *l != 0 {
            *l = uf.find(*l);
        }
    }

    // Aggregate per-region stats in a single pass.
    use std::collections::HashMap;
    let mut acc: HashMap<u32, LabelledRegion> = HashMap::new();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let l = region_labels[i];
            if l == 0 {
                continue;
            }
            let entry = acc.entry(l).or_insert(LabelledRegion {
                cluster: cluster_labels[i],
                area: 0,
                min_x: x,
                max_x: x,
                min_y: y,
                max_y: y,
                seed_x: x,
                seed_y: y,
                label: l,
            });
            entry.area += 1;
            if x < entry.min_x { entry.min_x = x; }
            if x > entry.max_x { entry.max_x = x; }
            if y < entry.min_y { entry.min_y = y; }
            if y > entry.max_y { entry.max_y = y; }
        }
    }

    let regions: Vec<LabelledRegion> = acc
        .into_values()
        .filter(|r| r.area >= min_area)
        .collect();

    RegionsResult { regions, region_labels }
}

struct UnionFind {
    parent: Vec<u32>,
}
impl UnionFind {
    fn new() -> Self { UnionFind { parent: vec![0] } }
    fn make_set(&mut self, x: u32) {
        while self.parent.len() <= x as usize {
            self.parent.push(self.parent.len() as u32);
        }
        self.parent[x as usize] = x;
    }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let p = self.parent[x as usize];
            let gp = self.parent[p as usize];
            self.parent[x as usize] = gp;
            x = gp;
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            if ra < rb { self.parent[rb as usize] = ra; } else { self.parent[ra as usize] = rb; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_two_disjoint_blobs_of_same_cluster() {
        // 10x4 image, cluster 1 fills cols 1-3 AND cols 6-8, cluster 0 elsewhere.
        let w = 10u32; let h = 4u32;
        let mut labels = vec![0u32; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let l = if (1..=3).contains(&x) || (6..=8).contains(&x) { 1 } else { 0 };
                labels[(y * w + x) as usize] = l;
            }
        }
        let res = label_regions_per_cluster_full(&labels, w, h, 1);
        let cluster1_regions: Vec<_> = res.regions.iter().filter(|r| r.cluster == 1).collect();
        assert_eq!(cluster1_regions.len(), 2, "expected 2 disjoint cluster-1 regions, got {cluster1_regions:?}");
    }

    #[test]
    fn drops_regions_below_min_area() {
        // Single 2-pixel sliver in cluster 1.
        let w = 5u32; let h = 5u32;
        let mut labels = vec![0u32; (w * h) as usize];
        labels[6] = 1;
        labels[7] = 1;
        let res = label_regions_per_cluster_full(&labels, w, h, 5);
        let cluster1_regions: Vec<_> = res.regions.iter().filter(|r| r.cluster == 1).collect();
        assert_eq!(cluster1_regions.len(), 0);
    }
}
