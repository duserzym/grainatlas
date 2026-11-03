//! Marker-controlled watershed on the distance transform with h-maxima markers
//! (grayscale reconstruction by dilation, L. Vincent, IEEE TIP 1993).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

use super::morph;

const NB8: [(i64, i64); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// Labels (0 = background) after splitting touching objects of `mask`. Neighbouring regions
/// are merged back unless the distance at their shared boundary (the neck) is below
/// `neck_ratio` times the smaller region's largest distance: touching grains meet at a neck,
/// a long prism with a slightly uneven width does not.
pub fn split(mask: &[bool], w: usize, h: usize, hval: f32, neck_ratio: f32) -> Vec<u32> {
    let d = morph::distance(mask, w, h);
    let marker: Vec<f32> = d.iter().map(|&v| (v - hval).max(0.0)).collect();
    let r = reconstruct(&marker, &d, w, h);
    let maxima: Vec<bool> = d.iter().zip(&r).map(|(&a, &b)| a > 0.0 && a - b >= hval - 1e-3).collect();
    let (mut lab, _) = morph::components(&maxima, w, h);
    flood(&mut lab, &d, mask, w, h);
    merge_without_neck(&mut lab, &d, w, h, neck_ratio);
    // Components that got no marker (smaller than h) keep their own label.
    let (cl, cc) = morph::components(mask, w, h);
    let mut next = lab.iter().copied().max().unwrap_or(0);
    let mut fresh = vec![0u32; cc.len()];
    for i in 0..w * h {
        if mask[i] && lab[i] == 0 {
            let c = cl[i] as usize;
            if fresh[c] == 0 {
                next += 1;
                fresh[c] = next;
            }
            lab[i] = fresh[c];
        }
    }
    lab
}

#[derive(PartialEq)]
struct Item(f32, u64, usize);
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    // Highest distance first; earlier insertion first on ties (deterministic).
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0).then(o.1.cmp(&self.1))
    }
}

/// Floods labels from the markers over `mask`, highest distance first.
fn flood(lab: &mut [u32], d: &[f32], mask: &[bool], w: usize, h: usize) {
    let mut heap = BinaryHeap::new();
    let mut seq = 0u64;
    for i in 0..w * h {
        if lab[i] != 0 {
            heap.push(Item(d[i], seq, i));
            seq += 1;
        }
    }
    while let Some(Item(_, _, i)) = heap.pop() {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                continue;
            }
            let j = ny as usize * w + nx as usize;
            if mask[j] && lab[j] == 0 {
                lab[j] = lab[i];
                heap.push(Item(d[j], seq, j));
                seq += 1;
            }
        }
    }
}

/// Grayscale reconstruction by dilation of `marker` under `mask` (8-connected, hybrid algorithm).
pub fn reconstruct(marker: &[f32], mask: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut j: Vec<f32> = marker.iter().zip(mask).map(|(a, b)| a.min(*b)).collect();
    let at = |x: i64, y: i64| -> Option<usize> { (x >= 0 && y >= 0 && x < w as i64 && y < h as i64).then(|| y as usize * w + x as usize) };
    // Forward raster scan.
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let p = y as usize * w + x as usize;
            let mut m = j[p];
            for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0)] {
                if let Some(q) = at(x + dx, y + dy) {
                    m = m.max(j[q]);
                }
            }
            j[p] = m.min(mask[p]);
        }
    }
    // Backward scan, queueing pixels that can still propagate.
    let mut fifo = VecDeque::new();
    for y in (0..h as i64).rev() {
        for x in (0..w as i64).rev() {
            let p = y as usize * w + x as usize;
            let mut m = j[p];
            let back = [(1, 1), (0, 1), (-1, 1), (1, 0)];
            for (dx, dy) in back {
                if let Some(q) = at(x + dx, y + dy) {
                    m = m.max(j[q]);
                }
            }
            j[p] = m.min(mask[p]);
            for (dx, dy) in back {
                if let Some(q) = at(x + dx, y + dy)
                    && j[q] < j[p]
                    && j[q] < mask[q]
                {
                    fifo.push_back(p);
                    break;
                }
            }
        }
    }
    while let Some(p) = fifo.pop_front() {
        let (x, y) = ((p % w) as i64, (p / w) as i64);
        for (dx, dy) in NB8 {
            if let Some(q) = at(x + dx, y + dy)
                && j[q] < j[p]
                && mask[q] != j[q]
            {
                j[q] = j[p].min(mask[q]);
                fifo.push_back(q);
            }
        }
    }
    j
}

fn merge_without_neck(lab: &mut [u32], d: &[f32], w: usize, h: usize, ratio: f32) {
    let n = lab.iter().copied().max().unwrap_or(0) as usize;
    if n < 2 {
        return;
    }
    let mut peak = vec![0f32; n + 1];
    for (i, &l) in lab.iter().enumerate() {
        if l != 0 {
            peak[l as usize] = peak[l as usize].max(d[i]);
        }
    }
    let mut saddle: std::collections::BTreeMap<(u32, u32), f32> = std::collections::BTreeMap::new();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let a = lab[i];
            if a == 0 {
                continue;
            }
            for j in [(x + 1 < w).then(|| i + 1), (y + 1 < h).then(|| i + w)].into_iter().flatten() {
                let b = lab[j];
                if b != 0 && b != a {
                    let k = (a.min(b), a.max(b));
                    let s = d[i].min(d[j]);
                    let e = saddle.entry(k).or_insert(0.0);
                    *e = e.max(s);
                }
            }
        }
    }
    let mut parent: Vec<u32> = (0..=n as u32).collect();
    fn find(p: &mut [u32], x: u32) -> u32 {
        let mut r = x;
        while p[r as usize] != r {
            r = p[r as usize];
        }
        let mut c = x;
        while p[c as usize] != r {
            let nx = p[c as usize];
            p[c as usize] = r;
            c = nx;
        }
        r
    }
    let mut pairs: Vec<((u32, u32), f32)> = saddle.into_iter().collect();
    pairs.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    for ((a, b), s) in pairs {
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra == rb {
            continue;
        }
        if s >= ratio * peak[ra as usize].min(peak[rb as usize]) {
            let (lo, hi) = (ra.min(rb), ra.max(rb));
            parent[hi as usize] = lo;
            peak[lo as usize] = peak[lo as usize].max(peak[hi as usize]);
        }
    }
    for l in lab.iter_mut() {
        if *l != 0 {
            *l = find(&mut parent, *l);
        }
    }
}
