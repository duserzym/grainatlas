//! Padding of new outlines on the work grid, with neighbouring labels as hard limits.

use serde::{Deserialize, Serialize};

use super::{Grain, SegmentParams, WorkImage, morph, shape};
use crate::image::Gray;
use crate::photometric::background::gaussian_blur;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OutlinePad {
    pub margin_um: f64,
    pub smooth: bool,
}

impl Default for OutlinePad {
    fn default() -> Self {
        Self { margin_um: 4.0, smooth: false }
    }
}

impl From<&crate::spots::Settings> for OutlinePad {
    fn from(s: &crate::spots::Settings) -> Self {
        Self { margin_um: s.outline_margin_um, smooth: s.outline_smooth }
    }
}

/// Automatic detections get padding; artifacts and carried-over hand edits keep their outlines.
pub fn segment_padded(w: &WorkImage, p: &SegmentParams, previous: &[Grain], outline: OutlinePad) -> Vec<Grain> {
    let grains = super::segment(w, p, previous);
    pad_outlines(w, &grains, outline, p.simplify_px, |g| g.is_grain() && !g.drawn)
}

pub(super) struct Crop {
    pub mask: Vec<bool>,
    distance: Vec<f32>,
    pub w: usize,
    pub h: usize,
    pub origin: (usize, usize),
}

/// Grows only selected grains, assigning empty pixels to their strictly nearest original label.
/// Unselected grains (including artifacts and unnumbered grains) still reserve their pixels.
pub fn pad_outlines(w: &WorkImage, grains: &[Grain], outline: OutlinePad, simplify_px: f64, selected: impl Fn(&Grain) -> bool) -> Vec<Grain> {
    if outline.margin_um <= 0.0 && !outline.smooth || grains.is_empty() || w.img.w == 0 || w.img.h == 0 {
        return grains.to_vec();
    }
    let radius = (outline.margin_um.max(0.0) / w.um_per_px) as f32;
    let reach = radius + if outline.smooth { 1.0 } else { 0.0 };
    let mut labels = vec![0usize; w.img.w * w.img.h];
    let mut nearest = vec![0usize; labels.len()];
    let mut best = vec![f32::INFINITY; labels.len()];
    let mut crops = Vec::with_capacity(grains.len());
    for (n, g) in grains.iter().enumerate() {
        let (mask, bw, bh, origin) = shape::rasterize(g, w.um_per_px, reach.ceil() as usize + 2);
        let cw = bw.min(w.img.w.saturating_sub(origin.0));
        let ch = bh.min(w.img.h.saturating_sub(origin.1));
        if cw == 0 || ch == 0 {
            crops.push(None);
            continue;
        }
        let mask: Vec<bool> = (0..ch).flat_map(|y| mask[y * bw..y * bw + cw].iter().copied()).collect();
        let distance = morph::distance(&mask.iter().map(|v| !v).collect::<Vec<_>>(), cw, ch);
        for y in 0..ch {
            for x in 0..cw {
                let i = y * cw + x;
                let j = (origin.1 + y) * w.img.w + origin.0 + x;
                if mask[i] {
                    labels[j] = n + 1;
                }
                if distance[i] < best[j] {
                    best[j] = distance[i];
                    nearest[j] = n + 1;
                } else if distance[i] == best[j] {
                    nearest[j] = 0; // Ties stay empty, so neighbouring grains cannot merge.
                }
            }
        }
        crops.push(Some(Crop { mask, distance, w: cw, h: ch, origin }));
    }
    grains.iter().enumerate().map(|(n, g)| {
        if !selected(g) {
            return g.clone();
        }
        let Some(c) = &crops[n] else { return g.clone() };
        let allowed: Vec<bool> = (0..c.mask.len()).map(|i| {
            let j = (c.origin.1 + i / c.w) * w.img.w + c.origin.0 + i % c.w;
            w.cov[j] && (labels[j] == 0 || labels[j] == n + 1) && (c.mask[i] || nearest[j] == n + 1 && c.distance[i] <= reach)
        }).collect();
        let mask = grow(c, &allowed, radius, outline.smooth);
        if !outline.smooth && mask == c.mask {
            return g.clone(); // No new work pixels: avoid shrinking the rim by tracing it again.
        }
        let trace = |eps| shape::grain_from_mask(&mask, c.w, c.h, c.origin, w, eps, g.manual);
        let safe = |ng: &Grain| (0..c.h).all(|y| (0..c.w).all(|x| !shape::inside_polygon_px(ng, w.um_per_px, c.origin, x, y) || allowed[y * c.w + x]));
        let Some(mut ng) = trace(simplify_px).filter(safe).or_else(|| {
            // Simplification may bridge a concavity into a neighbour; retain the pixel boundary.
            trace(0.0).filter(safe)
        }) else { return g.clone() };
        ng.id = g.id;
        ng.number = g.number;
        ng.manual = g.manual;
        ng.artifact = g.artifact;
        ng.kept = g.kept;
        ng.drawn = g.drawn;
        ng
    }).collect()
}

/// Pads an editing mask before its first trace, avoiding repeated rasterisation of the rim.
pub(super) fn pad_mask(w: &WorkImage, mask: &[bool], bw: usize, bh: usize, origin: (usize, usize), outline: OutlinePad) -> Crop {
    let radius = (outline.margin_um.max(0.0) / w.um_per_px) as f32;
    let reach = radius + if outline.smooth { 1.0 } else { 0.0 };
    let pad = reach.ceil() as usize + 2;
    let ox = origin.0.saturating_sub(pad);
    let oy = origin.1.saturating_sub(pad);
    let cw = (origin.0 + bw + pad).min(w.img.w) - ox;
    let ch = (origin.1 + bh + pad).min(w.img.h) - oy;
    let mut m = vec![false; cw * ch];
    for y in 0..bh {
        for x in 0..bw {
            let (gx, gy) = (origin.0 + x, origin.1 + y);
            if gx < w.img.w && gy < w.img.h {
                m[(gy - oy) * cw + gx - ox] = mask[y * bw + x];
            }
        }
    }
    let distance = morph::distance(&m.iter().map(|v| !v).collect::<Vec<_>>(), cw, ch);
    let mut c = Crop { mask: m, distance, w: cw, h: ch, origin: (ox, oy) };
    let allowed: Vec<bool> = (0..cw * ch).map(|i| w.cov[(oy + i / cw) * w.img.w + ox + i % cw] && c.distance[i] <= reach).collect();
    c.mask = grow(&c, &allowed, radius, outline.smooth);
    c
}

fn grow(c: &Crop, allowed: &[bool], radius: f32, smooth: bool) -> Vec<bool> {
    let mut mask: Vec<bool> = (0..c.mask.len()).map(|i| allowed[i] && (c.mask[i] || c.distance[i] <= radius)).collect();
    if smooth {
        let blurred = gaussian_blur(&Gray { w: c.w, h: c.h, px: mask.iter().map(|&v| if v { 1.0 } else { 0.0 }).collect() }, 1.0);
        let inward = morph::distance(&mask, c.w, c.h);
        let outward = morph::distance(&mask.iter().map(|v| !v).collect::<Vec<_>>(), c.w, c.h);
        for i in 0..mask.len() {
            // Bound smoothing to one work pixel even on narrow or ragged outlines.
            mask[i] = allowed[i] && if mask[i] { blurred.px[i] >= 0.5 || inward[i] > 1.0 } else { blurred.px[i] >= 0.5 && outward[i] <= 1.0 };
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> WorkImage {
        WorkImage { img: Gray { w: 240, h: 180, px: vec![0.5; 240 * 180] }, cov: vec![true; 240 * 180], um_per_px: 2.0 }
    }

    fn block(w: &WorkImage, x: usize, y: usize, width: usize, height: usize, number: u32) -> Grain {
        let mut g = shape::grain_from_mask(&vec![true; width * height], width, height, (x, y), w, 0.8, false).unwrap();
        // Exact pixel-edge polygon, to measure padding independently of tracing's centre boundary.
        let k = w.um_per_px;
        g.polygon_um = vec![[x as f64 * k, y as f64 * k], [(x + width) as f64 * k, y as f64 * k], [(x + width) as f64 * k, (y + height) as f64 * k], [x as f64 * k, (y + height) as f64 * k]];
        g.number = number;
        g
    }

    #[test]
    fn block_grows_by_the_margin_and_zero_is_exact() {
        let w = scene();
        let g = block(&w, 30, 30, 100, 80, 7);
        for margin in [0.0, -4.0] {
            let no = pad_outlines(&w, std::slice::from_ref(&g), OutlinePad { margin_um: margin, smooth: false }, 0.8, |_| true);
            assert_eq!(serde_json::to_vec(&no).unwrap(), serde_json::to_vec(std::slice::from_ref(&g)).unwrap());
        }
        let subpixel = pad_outlines(&w, std::slice::from_ref(&g), OutlinePad { margin_um: 0.5, smooth: false }, 0.8, |_| true);
        assert_eq!(subpixel[0], g);
        let out = pad_outlines(&w, std::slice::from_ref(&g), OutlinePad::default(), 0.8, |_| true);
        let perimeter = 2.0 * (100.0 + 80.0) * w.um_per_px;
        assert!((out[0].area_um2 - g.area_um2 - perimeter * 4.0).abs() < perimeter * w.um_per_px);
        for (a, b) in out[0].bbox_um.iter().zip(g.bbox_um) {
            assert!(((a - b).abs() - 4.0).abs() <= w.um_per_px);
        }
    }

    #[test]
    fn touching_labels_keep_identity_and_do_not_overlap() {
        let w = scene();
        let mut a = block(&w, 30, 30, 50, 80, 7);
        a.manual = true;
        a.kept = true;
        let mut b = block(&w, 80, 30, 50, 80, 8);
        b.drawn = true;
        let grains = [a, b];
        for smooth in [false, true] {
            let out = pad_outlines(&w, &grains, OutlinePad { margin_um: 8.0, smooth }, 0.8, |_| true);
            for (a, b) in out.iter().zip(&grains) {
                assert_eq!((a.id, a.number, a.manual, a.artifact, a.kept, a.drawn), (b.id, b.number, b.manual, b.artifact, b.kept, b.drawn));
                assert!(a.area_um2 > b.area_um2);
            }
            for y in 0..w.img.h {
                for x in 0..w.img.w {
                    assert!(!out.iter().all(|g| shape::inside_polygon_px(g, w.um_per_px, (0, 0), x, y)));
                }
            }
        }
    }

    #[test]
    fn smoothing_is_light_and_unselected_grains_are_untouched() {
        let w = scene();
        let mut grains = vec![block(&w, 20, 20, 100, 100, 1), block(&w, 122, 20, 90, 100, 0)];
        grains[1].artifact = Some(super::super::Artifact::Manual);
        let out = pad_outlines(&w, &grains, OutlinePad { margin_um: 0.0, smooth: true }, 0.8, |g| g.is_grain());
        assert!((out[0].area_um2 / grains[0].area_um2 - 1.0).abs() < 0.03);
        assert_eq!(out[1], grains[1]);
    }

    #[test]
    fn padding_stays_in_coverage_and_the_nearest_label() {
        let mut w = scene();
        for y in 0..w.img.h {
            w.cov[y * w.img.w + 25] = false;
        }
        let grains = [block(&w, 30, 30, 50, 80, 1), block(&w, 86, 30, 50, 80, 2)];
        let out = pad_outlines(&w, &grains, OutlinePad { margin_um: 20.0, smooth: true }, 0.8, |_| true);
        for y in 0..w.img.h {
            assert!(!shape::inside_polygon_px(&out[0], w.um_per_px, (0, 0), 25, y));
            assert!(!shape::inside_polygon_px(&out[0], w.um_per_px, (0, 0), 84, y));
            assert!(!shape::inside_polygon_px(&out[1], w.um_per_px, (0, 0), 81, y));
        }
    }
}
