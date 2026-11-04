//! Per-grain tone normalisation: after segmentation, each grain's tone is
//! matched to the area's (median and spread of all grain pixels pooled) or to
//! one reference grain, with a strength and a soft edge, and only inside the
//! grain masks. Runs on the stretched 0..1 mosaic as the last tone step, so the
//! base mosaic, exports and prints carry it; parameters live in
//! `ToneParams::grain_norm` (project.json).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::image::Gray;
use crate::segment::Grain;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GrainNorm {
    pub enabled: bool,
    /// 0 = unchanged, 1 = fully matched.
    pub strength: f32,
    /// Match this grain's median and spread; None = the area's (all grains pooled).
    pub reference: Option<Uuid>,
    /// Width of the soft edge inside each grain, micrometres.
    pub feather_um: f64,
}

impl Default for GrainNorm {
    fn default() -> Self {
        Self { enabled: false, strength: 0.8, reference: None, feather_um: 4.0 }
    }
}

/// Spread below this (0..1 units) is treated as flat: no contrast is invented.
const MIN_SPREAD: f32 = 0.02;

/// Median and spread (interquartile range) of values.
fn stats(mut v: Vec<f32>) -> Option<(f32, f32)> {
    if v.len() < 8 {
        return None;
    }
    v.sort_by(f32::total_cmp);
    let q = |p: f32| v[((v.len() - 1) as f32 * p).round() as usize];
    Some((q(0.5), q(0.75) - q(0.25)))
}

/// Label image (index + 1 into `grains`, 0 = none) at the resolution of `g`.
fn labels(w: usize, h: usize, grains: &[Grain], um_per_px: f64) -> Vec<u32> {
    let mut lab = vec![0u32; w * h];
    for (i, gr) in grains.iter().enumerate().filter(|(_, g)| g.is_grain()) {
        let (mask, bw, bh, (ox, oy)) = crate::segment::shape::rasterize(gr, um_per_px, 0);
        for y in 0..bh {
            for x in 0..bw {
                let (gx, gy) = (ox + x, oy + y);
                if mask[y * bw + x] && gx < w && gy < h {
                    lab[gy * w + gx] = i as u32 + 1;
                }
            }
        }
    }
    lab
}

/// Distance (px, chamfer 3-4 / 3) from each labelled pixel to the nearest pixel with another label.
fn inner_distance(lab: &[u32], w: usize, h: usize) -> Vec<f32> {
    const INF: u32 = u32::MAX / 4;
    let mut d: Vec<u32> = vec![INF; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let l = lab[i];
            if l == 0 {
                d[i] = 0;
                continue;
            }
            let edge = x == 0 || y == 0 || x + 1 == w || y + 1 == h || lab[i - 1] != l || lab[i + 1] != l || lab[i - w] != l || lab[i + w] != l;
            if edge {
                d[i] = 3; // one pixel
            }
        }
    }
    let at = |d: &Vec<u32>, x: isize, y: isize| if x < 0 || y < 0 || x >= w as isize || y >= h as isize { 0 } else { d[y as usize * w + x as usize] };
    for y in 0..h as isize {
        for x in 0..w as isize {
            let i = y as usize * w + x as usize;
            if lab[i] == 0 {
                continue;
            }
            let m = (at(&d, x - 1, y) + 3).min(at(&d, x, y - 1) + 3).min(at(&d, x - 1, y - 1) + 4).min(at(&d, x + 1, y - 1) + 4);
            d[i] = d[i].min(m);
        }
    }
    for y in (0..h as isize).rev() {
        for x in (0..w as isize).rev() {
            let i = y as usize * w + x as usize;
            if lab[i] == 0 {
                continue;
            }
            let m = (at(&d, x + 1, y) + 3).min(at(&d, x, y + 1) + 3).min(at(&d, x + 1, y + 1) + 4).min(at(&d, x - 1, y + 1) + 4);
            d[i] = d[i].min(m);
        }
    }
    d.into_iter().map(|v| v as f32 / 3.0).collect()
}

/// Matches each grain's median and spread to the target, in place on a 0..1 image whose pixels
/// are `um_per_px` micrometres. Returns the number of grains changed.
pub fn normalize(g: &mut Gray, grains: &[Grain], um_per_px: f64, p: &GrainNorm) -> usize {
    if !p.enabled || p.strength <= 0.0 || grains.is_empty() || um_per_px <= 0.0 {
        return 0;
    }
    let (w, h) = (g.w, g.h);
    let lab = labels(w, h, grains, um_per_px);
    let mut per: Vec<Vec<f32>> = vec![Vec::new(); grains.len()];
    for (i, &l) in lab.iter().enumerate() {
        if l > 0 {
            per[l as usize - 1].push(g.px[i]);
        }
    }
    let st: Vec<Option<(f32, f32)>> = per.iter().map(|v| stats(v.clone())).collect();
    let target = match p.reference.and_then(|id| grains.iter().position(|q| q.id == id)) {
        Some(r) => st[r],
        None => stats(per.concat()),
    };
    let Some((tm, ts)) = target else { return 0 };
    let ts = ts.max(MIN_SPREAD);
    let maps: Vec<Option<(f32, f32)>> = st
        .iter()
        .map(|s| s.map(|(m, sp)| {
            let gain = (ts / sp.max(MIN_SPREAD)).clamp(0.25, 4.0);
            (gain, tm - m * gain)
        }))
        .collect();
    let feather = (p.feather_um / um_per_px).max(0.0) as f32;
    let dist = if feather > 0.0 { inner_distance(&lab, w, h) } else { Vec::new() };
    let k = p.strength.clamp(0.0, 1.0);
    for (i, &l) in lab.iter().enumerate() {
        if l == 0 {
            continue;
        }
        let Some((gain, off)) = maps[l as usize - 1] else { continue };
        let soft = if feather > 0.0 { (dist[i] / feather).min(1.0) } else { 1.0 };
        let v = g.px[i];
        g.px[i] = (v + k * soft * (v * gain + off - v)).clamp(0.0, 1.0);
    }
    maps.iter().filter(|m| m.is_some()).count()
}
