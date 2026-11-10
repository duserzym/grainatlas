//! Second pass and artifact classifier (Boise SEM runs, 2026-11-03 and -04): the
//! deviation from the local epoxy level, CL-dark (metamict) grains that
//! `Polarity::Bright` misses, and the artifacts of TM4000 CL mounts.
//!
//! The primary segmenter stays as it is. [`analyse`] measures the mosaic once and
//! finds bubbles; [`clean_mask`] keeps bubble halos out of the bright mask and cuts
//! seam lines that bridge grains; [`merge_footprints`] adds the grains of either
//! polarity and joins the fragments of one grain into its whole footprint;
//! [`classify`] flags artifacts with a reason (it never deletes them), splitting
//! oversized blobs back into their parts first. Sizes scale with the area's median
//! grain size ([`reference_size`], module [`rel`]): the rules were tuned at about
//! 110 um (S mount) and 175 um (L mount).

use serde::{Deserialize, Serialize};

use super::{Artifact, Grain, Polarity, SegmentParams, Signal, WorkImage, morph, shape, watershed};
use crate::image::Gray;
use crate::photometric::background::gaussian_blur;

/// A round dark blob in the epoxy (an air bubble or a pit), mosaic micrometres.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Bubble {
    pub c: [f64; 2],
    pub r: f64,
}

/// Shape measures of an outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeStats {
    /// 4πA/P² (1 for a disc).
    pub circularity: f64,
    /// Area over the convex hull's area.
    pub solidity: f64,
    /// Major over minor axis.
    pub aspect: f64,
    /// Area over the bounding box's area.
    pub extent: f64,
}

/// Measures of the deviation image, kept for the report.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Analysis {
    /// Raw-intensity span used to normalize the deviation thresholds.
    #[serde(skip)]
    pub intensity_span: f32,
    /// Robust noise of the deviation from the local epoxy level (normalised units).
    pub sigma: f32,
    /// Bright and dark thresholds on the deviation.
    pub t_bright: f32,
    pub t_dark: f32,
    pub bubbles: Vec<Bubble>,
    /// The bubbles as artifact detections (shown, never printed).
    #[serde(skip)]
    pub bubble_grains: Vec<Grain>,
    #[serde(skip)]
    pub dev: Option<Gray>,
    #[serde(skip)]
    pub dark: Vec<bool>,
    /// Bubble cores (work pixels).
    #[serde(skip)]
    pub bubble_mask: Vec<bool>,
}

/// Size rules relative to the reference grain size `r` (um), as tuned on the Boise mounts.
pub mod rel {
    /// Largest bubble: 75 um at r = 110 (S mount), 115 um at r = 175 (L mount).
    pub fn bubble_max(r: f64) -> f64 {
        (0.68 * r).clamp(40.0, 160.0)
    }
    /// Seam opening radius: 4 um on S, 6.6 um on L.
    pub fn seam_open(r: f64) -> f64 {
        (0.037 * r).clamp(2.5, 9.0)
    }
    /// Oversized blob: 330 um on S, 500 um on L.
    pub fn block_split(r: f64) -> f64 {
        (3.0 * r).max(260.0)
    }
    /// A weakly contrasted region larger than this is a tile block.
    pub fn block_weak(r: f64) -> f64 {
        (2.4 * r).max(200.0)
    }
    /// Specks: under 22 um on S, 35 um on L (at least the 20 um minimum).
    pub fn speck(r: f64) -> f64 {
        (0.21 * r).max(22.0)
    }
    /// A speck this far from every grain is off the row.
    pub fn speck_far(r: f64) -> f64 {
        (1.8 * r).max(200.0)
    }
    /// Halo crescents: under 50 um on S.
    pub fn crescent(r: f64) -> f64 {
        (0.45 * r).max(40.0)
    }
    /// Fragments of one grain: under 35 um on S.
    pub fn fragment(r: f64) -> f64 {
        (0.32 * r).max(30.0)
    }
    /// Largest radius of a bubble rim arc.
    pub fn rim_max(r: f64) -> f64 {
        (0.8 * r).max(60.0)
    }
}

/// Reference grain size of an area: the median eq. diameter of its grains over 35 um that are
/// not seam slivers, each weighted by its eq. diameter so fragments of zoned grains count for
/// less, leaving out blobs over three times the upper quartile (tile blocks). 110 um with fewer
/// than five grains.
pub fn reference_size(grains: &[Grain]) -> f64 {
    let mut e: Vec<f64> = grains.iter().filter(|g| g.is_grain() && g.eq_diameter_um >= 35.0 && g.minor_um >= 15.0).map(|g| g.eq_diameter_um).collect();
    if e.len() < 5 {
        return 110.0;
    }
    e.sort_by(f64::total_cmp);
    // Tile blocks: over three times the upper quartile.
    let cap = 3.0 * e[(e.len() * 3) / 4];
    e.retain(|&d| d <= cap);
    let total: f64 = e.iter().sum();
    let mut acc = 0.0;
    for d in &e {
        acc += d;
        if acc >= total / 2.0 {
            return *d;
        }
    }
    e[e.len() / 2]
}

/// Convex hull (monotone chain), counter-clockwise, without the closing point.
pub fn convex_hull(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut p: Vec<[f64; 2]> = pts.to_vec();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

pub fn polygon_area(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0;
    for i in 0..n {
        let (q, r) = (p[i], p[(i + 1) % n]);
        a += q[0] * r[1] - r[0] * q[1];
    }
    a.abs() / 2.0
}

pub fn polygon_perimeter(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n).map(|i| (p[i][0] - p[(i + 1) % n][0]).hypot(p[i][1] - p[(i + 1) % n][1])).sum()
}

/// Shape measures of a grain. Its outline runs through pixel centres, so the hull is grown by
/// half a work pixel to compare with the pixel-count area.
pub fn shape_stats(g: &Grain, um_per_px: f64) -> ShapeStats {
    let hull = convex_hull(&g.polygon_um);
    let hull_area = polygon_area(&hull) + polygon_perimeter(&hull) * um_per_px / 2.0 + um_per_px * um_per_px;
    let perim = polygon_perimeter(&g.polygon_um) + std::f64::consts::PI * um_per_px / 2.0;
    let bw = (g.bbox_um[2] - g.bbox_um[0]).max(um_per_px);
    let bh = (g.bbox_um[3] - g.bbox_um[1]).max(um_per_px);
    ShapeStats {
        circularity: 4.0 * std::f64::consts::PI * g.area_um2 / perim.max(1e-9).powi(2),
        solidity: (g.area_um2 / hull_area.max(1e-9)).min(1.0),
        aspect: g.major_um / g.minor_um.max(um_per_px),
        extent: g.area_um2 / (bw * bh),
    }
}

/// Median of the covered values (sorted copy).
fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let m = v.len() / 2;
    *v.select_nth_unstable_by(m, f32::total_cmp).1
}

/// Percentile (0..100) of the values.
fn percentile(v: &[f32], pct: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    let k = ((pct / 100.0) * (s.len() - 1) as f32).round() as usize;
    *s.select_nth_unstable_by(k.min(v.len() - 1), f32::total_cmp).1
}

/// Local epoxy level: medians of 8 x 8 cells of covered pixels, a running median over
/// `window_um`, bilinear back to full size.
pub fn local_background(img: &Gray, cov: &[bool], um_per_px: f64, window_um: f64) -> Gray {
    const K: usize = 8;
    let (w, h) = (img.w, img.h);
    let (gw, gh) = (w.div_ceil(K), h.div_ceil(K));
    let global = median(img.px.iter().zip(cov).filter(|(_, c)| **c).map(|(v, _)| *v).collect());
    let mut cell = vec![f32::NAN; gw * gh];
    crate::util::par_rows(&mut cell, gw, |gy, row| {
        let mut buf = Vec::with_capacity(K * K);
        for (gx, o) in row.iter_mut().enumerate() {
            buf.clear();
            for y in gy * K..((gy + 1) * K).min(h) {
                for x in gx * K..((gx + 1) * K).min(w) {
                    if cov[y * w + x] {
                        buf.push(img.px[y * w + x]);
                    }
                }
            }
            if buf.len() * 4 >= K * K {
                *o = median(std::mem::take(&mut buf));
            }
        }
    });
    let win = ((window_um / (um_per_px * K as f64)).round() as usize).max(5) | 1;
    let r = (win / 2) as i64;
    let mut filt = vec![f32::NAN; gw * gh];
    crate::util::par_rows(&mut filt, gw, |gy, row| {
        let mut buf = Vec::with_capacity(win * win);
        for (gx, o) in row.iter_mut().enumerate() {
            buf.clear();
            for yy in (gy as i64 - r).max(0)..(gy as i64 + r + 1).min(gh as i64) {
                for xx in (gx as i64 - r).max(0)..(gx as i64 + r + 1).min(gw as i64) {
                    let v = cell[yy as usize * gw + xx as usize];
                    if !v.is_nan() {
                        buf.push(v);
                    }
                }
            }
            if buf.len() >= 3 {
                *o = median(std::mem::take(&mut buf));
            }
        }
    });
    for v in filt.iter_mut().filter(|v| v.is_nan()) {
        *v = global;
    }
    let g = Gray { w: gw, h: gh, px: filt };
    let mut out = Gray::new(w, h);
    crate::util::par_rows(&mut out.px, w, |y, row| {
        let fy = (y as f32 + 0.5) / K as f32 - 0.5;
        for (x, o) in row.iter_mut().enumerate() {
            *o = g.bilinear((x as f32 + 0.5) / K as f32 - 0.5, fy);
        }
    });
    out
}

fn eqd_px(um: f64, k: f64) -> f64 {
    um / k
}

/// Measures the deviation from the local epoxy level and finds bubbles (round dark blobs from
/// 12 um to [`rel::bubble_max`] of the reference size `r`; not in transmitted light, where the
/// grains are the dark blobs). Values are normalised first (covered p1 → 0, p99.5 → 1) so the
/// thresholds do not depend on the tone.
pub fn analyse(w: &WorkImage, p: &SegmentParams, r: f64) -> Analysis {
    let k = w.um_per_px;
    let covered: Vec<f32> = w.img.px.iter().zip(&w.cov).filter(|(_, c)| **c).map(|(v, _)| *v).step_by(3).collect();
    if covered.len() < 64 {
        return Analysis::default();
    }
    let (lo, hi) = (percentile(&covered, 1.0), percentile(&covered, 99.5));
    let span = (hi - lo).max(1e-4);
    let norm = Gray { w: w.img.w, h: w.img.h, px: w.img.px.iter().map(|&v| ((v - lo) / span).clamp(-0.5, 1.5)).collect() };
    let bg = local_background(&norm, &w.cov, k, p.background_window_um);
    let sm = gaussian_blur(&norm, 1.2);
    let dev = Gray { w: sm.w, h: sm.h, px: sm.px.iter().zip(&bg.px).map(|(a, b)| a - b).collect() };
    let dc: Vec<f32> = dev.px.iter().zip(&w.cov).filter(|(_, c)| **c).map(|(v, _)| *v).step_by(3).collect();
    let med = median(dc.clone());
    let sigma = 1.4826 * median(dc.iter().map(|v| (v - med).abs()).collect());
    let t_bright = (5.5 * sigma).max(0.10 * percentile(&dc, 99.7));
    // Dark: the noise margin, but never more than a fraction of the local epoxy level (a grain
    // darker than half the epoxy is dark however noisy the image is).
    let td = (5.5 * sigma).max(18.0 / 255.0);
    let (iw, ih) = (dev.w, dev.h);
    let mut dark: Vec<bool> = (0..iw * ih).map(|i| w.cov[i] && dev.px[i] < -td.min((p.dark_rel * bg.px[i]).max(14.0 / 255.0))).collect();
    dark = morph::open(&dark, iw, ih, 2);
    let mut bubbles = Vec::new();
    let mut bubble_grains = Vec::new();
    let mut bubble_mask = vec![false; iw * ih];
    if p.bubbles_enabled(w) && p.polarity != Polarity::Dark {
        // A bubble with a grey, textured middle breaks up in the dark mask: close and fill it.
        let solid = morph::fill_holes(&morph::close(&dark, iw, ih, ((4.0 / k).round() as usize).max(2)), iw, ih);
        let (lab, counts) = morph::components(&solid, iw, ih);
        let gs = grains_of_labels(&lab, &counts, iw, ih, w, eqd_px(12.0, k).powi(2) * std::f64::consts::PI / 4.0, 0.8);
        let max = rel::bubble_max(r);
        let mut is_bubble = vec![false; counts.len().max(1)];
        for (l, mut g) in gs {
            let s = shape_stats(&g, k);
            let round = s.circularity > 0.72 && s.solidity > 0.88 && s.aspect < 1.35;
            // Larger round dark blobs (up to 1.2 x the reference size) are bubbles when a bright
            // halo rings them; a CL-dark zircon sits in plain epoxy.
            let haloed = || g.eq_diameter_um <= 1.2 * r && ring_mean(&dev, &g, k, 6.0) > 0.5 * t_bright as f64;
            if round && g.eq_diameter_um >= 12.0 && (g.eq_diameter_um <= max || haloed()) {
                bubbles.push(Bubble { c: g.centroid_um, r: g.eq_diameter_um / 2.0 });
                if (l as usize) < is_bubble.len() {
                    is_bubble[l as usize] = true;
                }
                g.artifact = Some(Artifact::Bubble);
                bubble_grains.push(g);
            }
        }
        for (m, &l) in bubble_mask.iter_mut().zip(&lab) {
            *m = l > 0 && is_bubble[l as usize];
        }
        // Large bubbles whose middle is only somewhat below the epoxy (grey, textured): anything
        // clearly below the epoxy level, closed and filled, that is very round and has at least a
        // faint bright halo, from the bubble size limit up to 1.2 x the reference size.
        let lowt = (2.5 * sigma).max(0.25 * td);
        let low: Vec<bool> = (0..iw * ih).map(|i| w.cov[i] && dev.px[i] < -lowt).collect();
        let low = morph::open(&low, iw, ih, 2);
        let low = morph::fill_holes(&morph::close(&low, iw, ih, ((5.0 / k).round() as usize).max(2)), iw, ih);
        let (lab2, counts2) = morph::components(&low, iw, ih);
        let min2 = std::f64::consts::PI / 4.0 * (max / k).powi(2);
        let mut extra = vec![false; counts2.len().max(1)];
        for (l, mut g) in grains_of_labels(&lab2, &counts2, iw, ih, w, min2, 0.8) {
            let s = shape_stats(&g, k);
            let fresh = !bubbles.iter().any(|b| (b.c[0] - g.centroid_um[0]).hypot(b.c[1] - g.centroid_um[1]) < b.r + g.eq_diameter_um / 2.0);
            // Rounder than the first set asks, since the halo may glow on one side only.
            if fresh && g.eq_diameter_um <= 1.2 * r && s.circularity > 0.75 && s.solidity > 0.92 && s.aspect < 1.3 && ring_mean(&dev, &g, k, 6.0) > 0.25 * t_bright as f64 {
                bubbles.push(Bubble { c: g.centroid_um, r: g.eq_diameter_um / 2.0 });
                if (l as usize) < extra.len() {
                    extra[l as usize] = true;
                }
                g.artifact = Some(Artifact::Bubble);
                bubble_grains.push(g);
            }
        }
        for (m, &l) in bubble_mask.iter_mut().zip(&lab2) {
            if l > 0 && extra[l as usize] {
                *m = true;
            }
        }
    }
    Analysis { intensity_span: span, sigma, t_bright, t_dark: td, bubbles, bubble_grains, dev: Some(dev), dark, bubble_mask }
}

/// Mean of `img` in a ring `width_um` wide just outside a grain's outline.
fn ring_mean(img: &Gray, g: &Grain, k: f64, width_um: f64) -> f64 {
    let d = ((width_um / k).round() as usize).max(1);
    let (mask, bw, bh, o) = shape::rasterize(g, k, d + 1);
    let grown = morph::dilate(&mask, bw, bh, d);
    let (mut sum, mut n) = (0.0f64, 0usize);
    for y in 0..bh {
        for x in 0..bw {
            let i = y * bw + x;
            let (gx, gy) = (o.0 + x, o.1 + y);
            if grown[i] && !mask[i] && gx < img.w && gy < img.h {
                sum += img.px[gy * img.w + gx] as f64;
                n += 1;
            }
        }
    }
    if n == 0 { 0.0 } else { sum / n as f64 }
}

/// Bubble cores grown by 9 um: their halo zone.
fn bubble_zone(an: &Analysis, iw: usize, ih: usize, k: f64) -> Vec<bool> {
    if an.bubble_mask.len() != iw * ih {
        return vec![false; iw * ih];
    }
    // Round, not square: a square grows into the corners of grains next to the bubble.
    let r = (9.0 / k).max(3.0) as f32;
    let d = morph::distance(&an.bubble_mask.iter().map(|b| !b).collect::<Vec<bool>>(), iw, ih);
    d.iter().map(|&v| v <= r).collect()
}

/// The primary mask cleaned of artifacts before labelling, or `None` when nothing changes:
/// bubble cores and their halo zone removed (else the halo becomes a crescent "grain" or fuses
/// with a zircon next to it), and seam lines that bridge grains cut by an opening of
/// [`rel::seam_open`].
pub fn clean_mask(mask: &[bool], w: &WorkImage, p: &SegmentParams, an: &Analysis, r: f64) -> Option<Vec<bool>> {
    let (iw, ih) = (w.img.w, w.img.h);
    let k = w.um_per_px;
    let bubbles = p.bubbles_enabled(w) && !an.bubbles.is_empty() && p.polarity != Polarity::Dark;
    let seam = if p.flag_slivers { (rel::seam_open(r) / k).round() as usize } else { 0 };
    let seam = if seam > p.open_px { seam } else { 0 };
    if !bubbles && seam == 0 {
        return None;
    }
    let mut m = mask.to_vec();
    let zone = if bubbles { bubble_zone(an, iw, ih, k) } else { Vec::new() };
    let clear = |m: &mut Vec<bool>| {
        for (v, z) in m.iter_mut().zip(&zone) {
            if *z {
                *v = false;
            }
        }
    };
    clear(&mut m);
    // The opening also takes the thin remnants of a halo ring outside the zone.
    let r_open = seam.max(if bubbles { 2 } else { 0 });
    if r_open > 0 {
        m = morph::open(&m, iw, ih, r_open);
    }
    m = morph::fill_holes(&m, iw, ih);
    // Filling must not bring a bubble back inside a halo remnant.
    clear(&mut m);
    Some(m)
}

/// Grains of every label of `lab` with at least `min_px` pixels, with their label.
pub fn grains_of_labels(lab: &[u32], counts: &[usize], iw: usize, ih: usize, w: &WorkImage, min_px: f64, simplify_px: f64) -> Vec<(u32, Grain)> {
    let n = counts.len().saturating_sub(1).max(lab.iter().copied().max().unwrap_or(0) as usize);
    let mut bb = vec![[usize::MAX, usize::MAX, 0usize, 0usize]; n + 1];
    let mut cnt = vec![0usize; n + 1];
    for (i, &l) in lab.iter().enumerate() {
        if l == 0 {
            continue;
        }
        let (x, y) = (i % iw, i / iw);
        let b = &mut bb[l as usize];
        b[0] = b[0].min(x);
        b[1] = b[1].min(y);
        b[2] = b[2].max(x);
        b[3] = b[3].max(y);
        cnt[l as usize] += 1;
    }
    let _ = ih;
    let keep: Vec<u32> = (1..=n as u32).filter(|&l| cnt[l as usize] as f64 >= min_px.max(3.0)).collect();
    crate::util::par_map(keep.len(), |i| {
        let l = keep[i];
        let b = bb[l as usize];
        let (bw, bh) = (b[2] - b[0] + 1, b[3] - b[1] + 1);
        let mut local = vec![false; bw * bh];
        for y in 0..bh {
            for x in 0..bw {
                local[y * bw + x] = lab[(b[1] + y) * iw + b[0] + x] == l;
            }
        }
        shape::grain_from_mask(&local, bw, bh, (b[0], b[1]), w, simplify_px, false).map(|g| (l, g))
    })
    .into_iter()
    .flatten()
    .collect()
}

/// Mean of `img` inside a grain's outline.
pub fn mean_inside(img: &Gray, g: &Grain, k: f64) -> f64 {
    let (mask, bw, bh, origin) = shape::rasterize(g, k, 0);
    let (mut s, mut n) = (0.0f64, 0usize);
    for y in 0..bh {
        for x in 0..bw {
            let (gx, gy) = (origin.0 + x, origin.1 + y);
            if mask[y * bw + x] && gx < img.w && gy < img.h {
                s += img.px[gy * img.w + gx] as f64;
                n += 1;
            }
        }
    }
    if n == 0 { 0.0 } else { s / n as f64 }
}

/// A tile-seam sliver: a thin line along a seam, not a grain. Narrower than 15 um, or long
/// (aspect > 5), under 30 um wide, straight along a tile axis (its outline hugs its bounding
/// box) and either weak or very long (aspect > 8): a bright zircon needle lying along an axis
/// stays.
pub fn is_sliver(g: &Grain, contrast: f64) -> bool {
    let aspect = g.major_um / g.minor_um.max(1e-6);
    g.minor_um < 15.0 || (aspect > 5.0 && g.minor_um < 30.0 && box_fraction(g, 3.0) > 0.5 && (contrast < 0.6 || aspect > 8.0))
}

/// Fraction of a grain's pixels that stand out from the local epoxy level (above the bright
/// threshold or below the dark one).
pub fn contrast_fraction(dev: &Gray, g: &Grain, k: f64, tb: f32, td: f32) -> f64 {
    let (mask, bw, bh, origin) = shape::rasterize(g, k, 0);
    let (mut on, mut n) = (0usize, 0usize);
    for y in 0..bh {
        for x in 0..bw {
            let (gx, gy) = (origin.0 + x, origin.1 + y);
            if mask[y * bw + x] && gx < dev.w && gy < dev.h {
                let v = dev.px[gy * dev.w + gx];
                n += 1;
                if v > tb || v < -td {
                    on += 1;
                }
            }
        }
    }
    if n == 0 { 0.0 } else { on as f64 / n as f64 }
}

/// Fraction of the closed outline following an interior tile boundary. Opposing edges of
/// overlapping neighbours bound a band: blending can put the exposure step anywhere in it.
/// Only the common edge extent counts; unshared outer coverage edges never make a band.
/// Segments must stay in the band and within about 14 degrees of its axis. Lengths, rather
/// than vertex counts, keep the result independent of outline simplification.
pub fn tile_edge_fraction(g: &Grain, tiles: &[[f64; 4]], tol_um: f64) -> f64 {
    let tol = tol_um.max(0.0);
    // (normal axis, normal low/high, along-edge low/high).
    let mut bands = Vec::new();
    for (i, a) in tiles.iter().enumerate() {
        for b in &tiles[i + 1..] {
            for axis in 0..2 {
                let along = 1 - axis;
                let lo = a[along].max(b[along]);
                let hi = a[along + 2].min(b[along + 2]);
                if lo >= hi {
                    continue;
                }
                let (left, right) = if a[axis] < b[axis] { (a, b) } else { (b, a) };
                // Both tiles extend past the other on one side. Identical outer edges,
                // gaps and corner-only contacts give no interior boundary.
                if left[axis] < right[axis] && left[axis + 2] < right[axis + 2] && right[axis] <= left[axis + 2] {
                    bands.push((axis, right[axis] - tol, left[axis + 2] + tol, lo - tol, hi + tol));
                }
                // Unequal/contained footprints can have an interior edge without a
                // facing neighbour edge. Count that line, without widening to the
                // whole containing tile. Equal outer edges remain excluded.
                for (tile, other) in [(a, b), (b, a)] {
                    for edge in [tile[axis], tile[axis + 2]] {
                        if other[axis] < edge && edge < other[axis + 2] {
                            bands.push((axis, edge - tol, edge + tol, lo - tol, hi + tol));
                        }
                    }
                }
            }
        }
    }
    let (mut aligned, mut perimeter) = (0.0, 0.0);
    for (a, b) in g.polygon_um.iter().zip(g.polygon_um.iter().cycle().skip(1)).take(g.polygon_um.len()) {
        let length = (a[0] - b[0]).hypot(a[1] - b[1]);
        perimeter += length;
        if bands.iter().any(|&(axis, lo, hi, start, end)| {
            let along = 1 - axis;
            (a[axis] - b[axis]).abs() <= 0.25 * (a[along] - b[along]).abs() && [a, b].iter().all(|q| q[axis] >= lo && q[axis] <= hi && q[along] >= start && q[along] <= end)
        }) {
            aligned += length;
        }
    }
    if perimeter > 0.0 { aligned / perimeter } else { 0.0 }
}

// Measured at max(6 um, 2 work px): the 775/279 um BSE grains give 0.236/0.245;
// the HJ17/HH12 exposure blocks give 0.535, 0.497 and 0.306. Keep a gap on both sides.
const BLOCK_TILE_EDGE_FRACTION: f64 = 0.28;

fn away_from_tile_edges(g: &Grain, w: &WorkImage) -> bool {
    !w.tiles_um.is_empty() && tile_edge_fraction(g, &w.tiles_um, 6.0_f64.max(2.0 * w.um_per_px)) < BLOCK_TILE_EDGE_FRACTION
}

/// A large uniform BSE grain can fill the running background window. In that case its
/// interior falsely measures as epoxy. Compare non-boxy candidates with the exterior epoxy
/// too, using the same normalized bright/dark thresholds as `contrast_fraction`. Boxy steps
/// retain their local measurement, and CL retains every original measurement.
fn block_contrast(g: &Grain, w: &WorkImage, p: &SegmentParams, an: &Analysis, contrast: f64) -> f64 {
    if p.resolved_signal(w) != Signal::Bse || contrast >= 0.5 || box_fraction(g, 3.0) > 0.45 {
        return contrast;
    }
    let k = w.um_per_px;
    let level = ring_mean(&w.img, g, k, 12.0) as f32;
    let span = an.intensity_span.max(1e-4);
    let (mask, bw, bh, o) = shape::rasterize(g, k, 0);
    let (mut on, mut n) = (0usize, 0usize);
    for y in 0..bh {
        for x in 0..bw {
            let (gx, gy) = (o.0 + x, o.1 + y);
            if mask[y * bw + x] && gx < w.img.w && gy < w.img.h && w.cov[gy * w.img.w + gx] {
                let v = (w.img.px[gy * w.img.w + gx] - level) / span;
                n += 1;
                if v > an.t_bright || v < -an.t_dark { on += 1; }
            }
        }
    }
    contrast.max(if n == 0 { 0.0 } else { on as f64 / n as f64 })
}

fn should_split_block(g: &Grain, w: &WorkImage, p: &SegmentParams, an: &Analysis, r: f64, contrast: f64) -> bool {
    g.eq_diameter_um > rel::block_split(r)
        && (p.resolved_signal(w) == Signal::Cl || !away_from_tile_edges(g, w) || block_contrast(g, w, p, an, contrast) < 0.5 || box_fraction(g, 3.0) > 0.45)
}

/// A bright exposure-step tile block (or the step region next to one): larger than
/// `max_size_um`; or with almost no pixels that stand out from the local epoxy level (it was
/// only above the global threshold because its tile was exposed brighter); or weakly contrasted
/// and either larger than [`rel::block_weak`] of the reference size or bounded by its box (tile
/// edges); or a dark straight-edged rectangle filling its bounding box.
pub fn is_block(g: &Grain, contrast: f64, dark: bool, max_size_um: f64, r: f64) -> bool {
    let bx = box_fraction(g, 3.0);
    let extent = g.area_um2 / ((g.bbox_um[2] - g.bbox_um[0]) * (g.bbox_um[3] - g.bbox_um[1])).max(1e-9);
    (max_size_um > 0.0 && g.eq_diameter_um > max_size_um)
        || contrast < 0.15
        || (contrast < 0.5 && (g.eq_diameter_um > rel::block_weak(r) || bx > 0.45))
        // A dark straight-edged rectangle filling its box: a tile exposed darker. (Bright
        // prisms lying along an axis, or grains cut by the mosaic edge, are boxy too.)
        || (dark && bx > 0.65 && extent > 0.85 && g.eq_diameter_um > 40.0)
}

/// Least-squares circle through points (Kåsa): centre, radius and RMS distance off the circle.
pub fn fit_circle(p: &[[f64; 2]]) -> Option<([f64; 2], f64, f64)> {
    let n = p.len() as f64;
    if p.len() < 5 {
        return None;
    }
    let (mx, my) = (p.iter().map(|q| q[0]).sum::<f64>() / n, p.iter().map(|q| q[1]).sum::<f64>() / n);
    let (mut suu, mut svv, mut suv, mut suuu, mut svvv, mut suvv, mut svuu) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for q in p {
        let (u, v) = (q[0] - mx, q[1] - my);
        suu += u * u;
        svv += v * v;
        suv += u * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suvv += u * v * v;
        svuu += v * u * u;
    }
    let det = suu * svv - suv * suv;
    if det.abs() < 1e-12 {
        return None;
    }
    let (b1, b2) = (0.5 * (suuu + suvv), 0.5 * (svvv + svuu));
    let uc = (b1 * svv - b2 * suv) / det;
    let vc = (suu * b2 - suv * b1) / det;
    let r = (uc * uc + vc * vc + (suu + svv) / n).sqrt();
    let c = [mx + uc, my + vc];
    let rms = (p.iter().map(|q| ((q[0] - c[0]).hypot(q[1] - c[1]) - r).powi(2)).sum::<f64>() / n).sqrt();
    Some((c, r, rms))
}

/// The bright rim of a bubble (radius 6 um to `rim_max`): an arc or a ring on a circle, or a
/// round blob whose middle is no brighter than the epoxy.
pub fn is_bubble_rim(g: &Grain, s: &ShapeStats, dev: Option<&Gray>, k: f64, tb: f32, rim_max: f64) -> bool {
    let Some((c, r, rms)) = fit_circle(&g.polygon_um) else { return false };
    if !(6.0..=rim_max).contains(&r) || rms > 0.15 * r + k {
        return false;
    }
    // An arc: on a circle but thin and hollow.
    if s.solidity < 0.7 && thickness(g) < 0.32 * g.eq_diameter_um {
        return true;
    }
    // A filled ring: round, its middle at the epoxy level.
    let Some(dev) = dev else { return false };
    if s.circularity < 0.7 || s.aspect > 1.35 {
        return false;
    }
    let ri = 0.5 * r / k;
    let (cx, cy) = (c[0] / k, c[1] / k);
    let (mut sum, mut n) = (0.0f64, 0usize);
    for y in (cy - ri).floor().max(0.0) as usize..((cy + ri).ceil() as usize).min(dev.h) {
        for x in (cx - ri).floor().max(0.0) as usize..((cx + ri).ceil() as usize).min(dev.w) {
            if (x as f64 + 0.5 - cx).hypot(y as f64 + 0.5 - cy) <= ri {
                sum += dev.px[y * dev.w + x] as f64;
                n += 1;
            }
        }
    }
    // A dark round grain is not a ring: its middle is far below the epoxy, a bubble's is at it.
    n > 0 && (sum / n as f64).abs() < 0.5 * tb as f64
}

/// A bubble's halo mistaken for a grain: centred on a bubble and not much larger than it, or a
/// small crescent (under [`rel::crescent`]) touching a bubble's edge.
pub fn is_bubble_halo(g: &Grain, bubbles: &[Bubble], r: f64) -> bool {
    bubbles.iter().any(|b| {
        let d = (g.centroid_um[0] - b.c[0]).hypot(g.centroid_um[1] - b.c[1]);
        (d < b.r + 12.0 && g.eq_diameter_um < 2.2 * b.r + 25.0) || (g.eq_diameter_um < rel::crescent(r) && d - g.eq_diameter_um / 2.0 < b.r + 15.0)
    })
}

/// A crescent halo of a bubble that was not found (a large bubble with a grey middle): a small
/// bright arc (under [`rel::crescent`], solidity < 0.8, aspect > 1.8) whose concave side, the
/// rest of its convex hull, is a sizeable region at the epoxy level.
pub fn is_crescent(g: &Grain, s: &ShapeStats, dev: &Gray, k: f64, tb: f32, r: f64) -> bool {
    if g.eq_diameter_um >= rel::crescent(r) || s.solidity >= 0.8 || s.aspect <= 1.8 {
        return false;
    }
    let hull = Grain { polygon_um: convex_hull(&g.polygon_um), ..g.clone() };
    let (hm, bw, bh, o) = shape::rasterize(&hull, k, 0);
    let (mut sum, mut n) = (0.0f64, 0usize);
    for y in 0..bh {
        for x in 0..bw {
            let (gx, gy) = (o.0 + x, o.1 + y);
            if hm[y * bw + x] && gx < dev.w && gy < dev.h && !shape::inside_polygon_px(g, k, (0, 0), gx, gy) {
                sum += dev.px[gy * dev.w + gx] as f64;
                n += 1;
            }
        }
    }
    let deficit = n as f64 * k * k;
    n > 0 && deficit > 0.25 * g.area_um2 && sum / (n as f64) < 0.6 * tb as f64
}

/// Why `g` is an artifact, if it is (as the `flag_*` switches allow). Specks are decided
/// later, against the grains that remain.
fn verdict(g: &Grain, w: &WorkImage, p: &SegmentParams, an: &Analysis, r: f64) -> Option<Artifact> {
    let k = w.um_per_px;
    let signal = p.resolved_signal(w);
    let dev = an.dev.as_ref();
    let s = shape_stats(g, k);
    let bright = dev.is_none_or(|d| mean_inside(d, g, k) > 0.0);
    if p.flag_bubbles && signal == Signal::Cl
        && p.polarity != Polarity::Dark
        && ((bright && (is_bubble_halo(g, &an.bubbles, r) || dev.is_some_and(|d| is_crescent(g, &s, d, k, an.t_bright, r)))) || is_bubble_rim(g, &s, dev, k, an.t_bright, rel::rim_max(r)))
    {
        return Some(Artifact::Halo);
    }
    // A large bubble on its own: round, dark, and ringed by a bright halo in the epoxy (a CL-dark
    // zircon sits in plain epoxy).
    if p.flag_bubbles && signal == Signal::Cl
        && p.polarity != Polarity::Dark
        && !bright
        && g.eq_diameter_um <= 1.5 * r
        && s.circularity > 0.72
        && s.solidity > 0.88
        && s.aspect < 1.35
        && dev.is_some_and(|d| ring_mean(d, g, k, 8.0) > 0.5 * an.t_bright as f64)
    {
        return Some(Artifact::Bubble);
    }
    let contrast = dev.map_or(1.0, |d| contrast_fraction(d, g, k, an.t_bright, an.t_dark));
    if p.flag_slivers && is_sliver(g, contrast) {
        return Some(Artifact::Sliver);
    }
    if p.flag_blocks {
        if signal == Signal::Bse && should_split_block(g, w, p, an, r, contrast) {
            return Some(Artifact::Block);
        }
        let contrast = if signal == Signal::Bse && away_from_tile_edges(g, w) {
            block_contrast(g, w, p, an, contrast)
        } else { contrast };
        if is_block(g, contrast, !bright, p.max_size_um, r) {
            return Some(Artifact::Block);
        }
    }
    None
}

/// The raw parts of an oversized blob (grains joined through seams, or an exposure-step block
/// with real grains inside): its pixels that stand out from the local epoxy level, opened
/// harder than the seam opening, holes filled, at least the speck size.
fn split_blob(g: &Grain, w: &WorkImage, p: &SegmentParams, an: &Analysis, r: f64) -> Vec<Grain> {
    let Some(dev) = &an.dev else { return Vec::new() };
    let k = w.um_per_px;
    let (mask, bw, bh, o) = shape::rasterize(g, k, 0);
    let mut m: Vec<bool> = (0..bw * bh)
        .map(|i| {
            let (x, y) = (o.0 + i % bw, o.1 + i / bw);
            mask[i] && x < dev.w && y < dev.h && {
                let v = dev.px[y * dev.w + x];
                v > an.t_bright || v < -an.t_dark
            }
        })
        .collect();
    let rr = ((2.0 * rel::seam_open(r) / k).round() as usize).max(3);
    m = morph::open(&m, bw, bh, rr);
    m = morph::fill_holes(&m, bw, bh);
    let (lab, counts) = morph::components(&m, bw, bh);
    let min_px = std::f64::consts::PI / 4.0 * (p.min_size_um.max(rel::speck(r)) / k).powi(2);
    let mut out = Vec::new();
    for (l, &n) in counts.iter().enumerate().skip(1) {
        if (n as f64) < min_px {
            continue;
        }
        let part: Vec<bool> = lab.iter().map(|&v| v as usize == l).collect();
        if let Some(mut q) = shape::grain_from_mask(&part, bw, bh, o, w, p.simplify_px, false) {
            q.mean_intensity = mean_inside(&w.img, &q, k);
            out.push(q);
        }
    }
    out
}

/// Blobs over [`rel::block_split`] of the reference size (grains joined through seams, or an
/// exposure-step block with real grains inside) are flagged as blocks, and their raw parts are
/// added to be joined into whole grains and classified on their own.
pub fn split_blocks(grains: &mut Vec<Grain>, w: &WorkImage, p: &SegmentParams, an: &Analysis, r: f64) {
    let big: Vec<usize> = (0..grains.len())
        .filter(|&i| {
            let g = &grains[i];
            if !g.is_grain() || g.eq_diameter_um <= rel::block_split(r) {
                return false;
            }
            let contrast = an.dev.as_ref().map_or(1.0, |d| contrast_fraction(d, g, w.um_per_px, an.t_bright, an.t_dark));
            if p.resolved_signal(w) == Signal::Bse && std::env::var_os("GRAINATLAS_DEBUG_SEG").is_some() {
                eprintln!(
                    "seg oversized BSE eqd_um={:.2} bbox_um={:?} contrast={:.4} block_contrast={:.4} tile_edge_fraction={:.4} tol_um={:.2} split={}",
                    g.eq_diameter_um,
                    g.bbox_um,
                    contrast,
                    block_contrast(g, w, p, an, contrast),
                    tile_edge_fraction(g, &w.tiles_um, 6.0_f64.max(2.0 * w.um_per_px)),
                    6.0_f64.max(2.0 * w.um_per_px),
                    should_split_block(g, w, p, an, r, contrast)
                );
            }
            should_split_block(g, w, p, an, r, contrast)
        })
        .collect();
    let mut parts = Vec::new();
    for i in big {
        grains[i].artifact = Some(Artifact::Block);
        parts.extend(split_blob(&grains[i], w, p, an, r));
    }
    grains.extend(parts);
}

/// Drops detections lying inside a larger grain's outline (fragments of a zoned grain found
/// on their own); the larger grain stands for them.
pub fn drop_nested(grains: &mut Vec<Grain>) {
    let nested: Vec<bool> = (0..grains.len())
        .map(|i| {
            let g = &grains[i];
            g.is_grain()
                && grains.iter().enumerate().any(|(j, q)| {
                    j != i && q.is_grain() && q.area_um2 > 2.0 * g.area_um2 && g.bbox_um[0] >= q.bbox_um[0] && g.bbox_um[2] <= q.bbox_um[2] && g.bbox_um[1] >= q.bbox_um[1] && g.bbox_um[3] <= q.bbox_um[3] && shape::inside(&q.polygon_um, g.centroid_um[0], g.centroid_um[1])
                })
        })
        .collect();
    let mut i = 0;
    grains.retain(|_| {
        i += 1;
        !nested[i - 1]
    });
}

/// Flags the artifacts among `grains` with their reason (never removes anything). Specks:
/// smaller than [`rel::speck`] and weakly contrasted or farther than [`rel::speck_far`] from
/// every grain.
pub fn classify(grains: &mut [Grain], w: &WorkImage, p: &SegmentParams, an: &Analysis, r: f64) {
    let k = w.um_per_px;
    let verdicts: Vec<Option<Artifact>> = crate::util::par_map(grains.len(), |i| if grains[i].is_grain() { verdict(&grains[i], w, p, an, r) } else { grains[i].artifact });
    for (g, v) in grains.iter_mut().zip(verdicts) {
        g.artifact = v;
    }
    if p.flag_specks {
        let dev = an.dev.as_ref();
        let real: Vec<([f64; 2], f64)> = grains.iter().filter(|g| g.is_grain() && g.eq_diameter_um >= rel::speck(r)).map(|g| (g.centroid_um, g.eq_diameter_um / 2.0)).collect();
        for g in grains.iter_mut().filter(|g| g.is_grain() && g.eq_diameter_um < rel::speck(r)) {
            let contrast = dev.map_or(1.0, |d| contrast_fraction(d, g, k, an.t_bright, an.t_dark));
            let gap = real.iter().map(|(c, rad)| (c[0] - g.centroid_um[0]).hypot(c[1] - g.centroid_um[1]) - rad).fold(f64::INFINITY, f64::min);
            if contrast < 0.5 || gap > rel::speck_far(r) {
                g.artifact = Some(Artifact::Speck);
            }
        }
    }
    if std::env::var_os("GRAINATLAS_DEBUG_SEG").is_some() {
        for g in grains.iter() {
            let s = shape_stats(g, k);
            let contrast = an.dev.as_ref().map_or(1.0, |d| contrast_fraction(d, g, k, an.t_bright, an.t_dark));
            eprintln!(
                "seg {} c=({:.0},{:.0}) eqd={:.0} min={:.0} or={:.0} sol={:.2} circ={:.2} asp={:.2} contrast={:.2} box={:.2} ref={r:.0}",
                g.artifact.map_or("grain", |a| a.label()),
                g.centroid_um[0],
                g.centroid_um[1],
                g.eq_diameter_um,
                g.minor_um,
                g.orientation_deg,
                s.solidity,
                s.circularity,
                s.aspect,
                contrast,
                box_fraction(g, 3.0)
            );
        }
    }
}

fn bbox_inter(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    ((a[2].min(b[2]) - a[0].max(b[0])).max(0.0)) * ((a[3].min(b[3]) - a[1].max(b[1])).max(0.0))
}

fn bbox_area(a: &[f64; 4]) -> f64 {
    (a[2] - a[0]).max(0.0) * (a[3] - a[1]).max(0.0)
}

/// True when two grains overlap by more than a quarter of the smaller bounding box.
pub fn overlaps(a: &Grain, b: &Grain) -> bool {
    bbox_inter(&a.bbox_um, &b.bbox_um) > 0.25 * bbox_area(&a.bbox_um).min(bbox_area(&b.bbox_um))
}

/// Pixels of either polarity relative to the local epoxy level (mostly the CL-dark grains the
/// primary pass misses), without bubbles and their halo zone, closed, filled and opened.
pub fn supplement_mask(w: &WorkImage, an: &Analysis) -> Option<Vec<bool>> {
    let dev = an.dev.as_ref()?;
    let (iw, ih) = (dev.w, dev.h);
    let zone = bubble_zone(an, iw, ih, w.um_per_px);
    let core = |i: usize| an.bubble_mask.get(i).copied().unwrap_or(false);
    let mut m: Vec<bool> = (0..iw * ih).map(|i| w.cov[i] && ((dev.px[i] > an.t_bright && !zone[i]) || (an.dark[i] && !core(i)))).collect();
    m = morph::close(&m, iw, ih, 3);
    m = morph::fill_holes(&m, iw, ih);
    m = morph::open(&m, iw, ih, 2);
    Some(morph::fill_holes(&m, iw, ih))
}

/// Adds the grains of either polarity and makes one footprint per grain (`Polarity::Both`):
/// the supplement mask joined with the grain outlines, bridged by a 5 um closing, holes filled
/// and opened like the seams. A footprint holding fragments of one grain (at most one piece
/// over [`rel::fragment`] of the reference size), or one grain whose detection covers under two
/// thirds of it (the bright part of a mostly CL-dark grain), becomes that grain, keeping the
/// largest piece's id. Footprints with no grain in them are added.
pub fn merge_footprints(grains: &mut Vec<Grain>, w: &WorkImage, p: &SegmentParams, an: &Analysis, r: f64) {
    let Some(sup) = supplement_mask(w, an) else { return };
    let (iw, ih) = (w.img.w, w.img.h);
    let k = w.um_per_px;
    let mut fp = sup;
    for g in grains.iter().filter(|g| g.is_grain()) {
        let (mask, bw, bh, o) = shape::rasterize(g, k, 0);
        for y in 0..bh {
            for x in 0..bw {
                let (gx, gy) = (o.0 + x, o.1 + y);
                if mask[y * bw + x] && gx < iw && gy < ih {
                    fp[gy * iw + gx] = true;
                }
            }
        }
    }
    fp = morph::close(&fp, iw, ih, ((5.0 / k).round() as usize).max(2));
    fp = morph::fill_holes(&fp, iw, ih);
    let seam = if p.flag_slivers { ((rel::seam_open(r) / k).round() as usize).max(2) } else { 2 };
    fp = morph::open(&fp, iw, ih, seam);
    // Bubbles and their halo zone stay out (filling would otherwise close the gap between a
    // grain and a bubble's halo remnant).
    let zone = bubble_zone(an, iw, ih, k);
    for (v, z) in fp.iter_mut().zip(&zone) {
        if *z {
            *v = false;
        }
    }
    // Footprints that touch each other stay apart where the watershed splits them.
    let lab = watershed::split(&fp, iw, ih, p.split_h, p.neck_ratio);
    let n = lab.iter().copied().max().unwrap_or(0) as usize;
    let min_px = std::f64::consts::PI / 4.0 * (28.0f64.max(p.min_size_um) / k).powi(2);
    let shapes: Vec<(u32, Grain)> = grains_of_labels(&lab, &vec![0; n + 1], iw, ih, w, min_px, p.simplify_px);
    let label_of = |g: &Grain| {
        let (x, y) = ((g.centroid_um[0] / k) as usize, (g.centroid_um[1] / k) as usize);
        if x < iw && y < ih { lab[y * iw + x] } else { 0 }
    };
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    for (i, g) in grains.iter().enumerate() {
        if g.is_grain() {
            members[label_of(g) as usize].push(i);
        }
    }
    let mut drop = vec![false; grains.len()];
    let mut add = Vec::new();
    for (l, shape) in &shapes {
        let ms = &members[*l as usize];
        let large = ms.iter().filter(|&&i| grains[i].eq_diameter_um >= rel::fragment(r)).count();
        let area: f64 = ms.iter().map(|&i| grains[i].area_um2).sum();
        // Pieces in one footprint are one grain (the watershed keeps touching grains apart),
        // unless two large ones each fill a good part of it.
        let big = ms.iter().filter(|&&i| grains[i].eq_diameter_um >= rel::fragment(r) && grains[i].area_um2 > 0.25 * shape.area_um2).count();
        let whole = match ms.len() {
            0 => !grains.iter().any(|g| g.is_grain() && overlaps(g, shape)),
            1 => area < shape.area_um2 / 1.5,
            _ => large <= 1 || big <= 1,
        };
        if !whole {
            continue;
        }
        let mut q = shape.clone();
        q.mean_intensity = mean_inside(&w.img, &q, k);
        if let Some(&keep) = ms.iter().max_by(|&&a, &&b| grains[a].area_um2.total_cmp(&grains[b].area_um2)) {
            q.id = grains[keep].id;
            q.manual = grains[keep].manual;
        }
        for &i in ms {
            drop[i] = true;
        }
        add.push(q);
    }
    let mut i = 0;
    grains.retain(|_| {
        i += 1;
        !drop[i - 1]
    });
    grains.extend(add);
}

/// Mean width of an outline: twice its area over its perimeter (a disc gives its radius).
pub fn thickness(g: &Grain) -> f64 {
    2.0 * g.area_um2 / polygon_perimeter(&g.polygon_um).max(1e-9)
}

/// Fraction of the outline within `tol_um` of its bounding box: about 1 for a tile-shaped
/// rectangle, low for a grain, which touches its box at a few points.
pub fn box_fraction(g: &Grain, tol_um: f64) -> f64 {
    let p = &g.polygon_um;
    let n = p.len();
    if n < 3 {
        return 0.0;
    }
    let b = g.bbox_um;
    let near = |q: [f64; 2]| (q[0] - b[0]).abs().min((b[2] - q[0]).abs()).min((q[1] - b[1]).abs()).min((b[3] - q[1]).abs()) <= tol_um;
    let mut on = 0.0;
    for i in 0..n {
        let (a, c) = (p[i], p[(i + 1) % n]);
        if near(a) && near(c) {
            on += (c[0] - a[0]).hypot(c[1] - a[1]);
        }
    }
    on / polygon_perimeter(p).max(1e-9)
}

/// Fraction of the outline on straight axis-aligned runs at least `min_um` long (tile edges).
pub fn straight_fraction(g: &Grain, min_um: f64) -> f64 {
    let p = &g.polygon_um;
    let n = p.len();
    if n < 3 {
        return 0.0;
    }
    let total = polygon_perimeter(p).max(1e-9);
    let mut on = 0.0;
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        let (dx, dy) = ((b[0] - a[0]).abs(), (b[1] - a[1]).abs());
        let len = dx.hypot(dy);
        if len >= min_um && (dx.min(dy) / len) < 0.05 {
            on += len;
        }
    }
    on / total
}

/// Grain mask over its bbox grown by `pad` pixels, as (mask, w, h, origin).
fn grain_raster(g: &Grain, k: f64, pad: usize) -> (Vec<bool>, usize, usize, (usize, usize)) {
    shape::rasterize(g, k, pad)
}

/// Fraction of a circle (centre and radius in local pixels) that lies outside `mask`.
fn circle_outside(mask: &[bool], bw: usize, bh: usize, c: [f64; 2], rad: f64) -> f64 {
    let n = 72;
    let mut out = 0;
    for i in 0..n {
        let a = i as f64 / n as f64 * std::f64::consts::TAU;
        let (x, y) = (c[0] + rad * a.cos(), c[1] + rad * a.sin());
        let inside = x >= 0.0 && y >= 0.0 && (x as usize) < bw && (y as usize) < bh && mask[y as usize * bw + x as usize];
        if !inside {
            out += 1;
        }
    }
    out as f64 / n as f64
}

/// Cuts epoxy bubbles off the grains they fused with (Boise Sweden S Skin_felsic, AH1-gc1, many
/// on Grenville L). A round dark region in a grain (12 um to 1.2 x the reference size,
/// circularity > 0.7, solidity > 0.85, aspect < 1.4) is a bubble when a circle beyond its halo
/// (radius + 20 um) runs mostly through the epoxy: the bubble sticks out of the grain. A
/// CL-dark core is ringed by its own grain and stays. The bubble and its 9 um halo come off the
/// grain (which keeps its id), the bubble is added as an artifact, and the analysis learns it
/// so its halo pieces are flagged too.
pub fn detach_bubbles(grains: &mut Vec<Grain>, w: &WorkImage, p: &SegmentParams, an: &mut Analysis, r: f64) {
    if !p.bubbles_enabled(w) || p.polarity == Polarity::Dark {
        return;
    }
    let Some(dev) = an.dev.clone() else { return };
    let k = w.um_per_px;
    let halo = ((9.0 / k).round() as usize).max(3);
    let mut found: Vec<Grain> = Vec::new();
    for g in grains.iter_mut().filter(|g| g.is_grain() && g.eq_diameter_um > 30.0) {
        let pad = ((16.0 / k).ceil() as usize).max(4);
        let (mask, bw, bh, o) = grain_raster(g, k, pad);
        let dark: Vec<bool> = (0..bw * bh)
            .map(|i| {
                let (x, y) = (o.0 + i % bw, o.1 + i / bw);
                mask[i] && x < dev.w && y < dev.h && dev.px[y * dev.w + x] < -an.t_dark
            })
            .collect();
        let dark = morph::open(&dark, bw, bh, 2);
        let (lab, counts) = morph::components(&dark, bw, bh);
        let mut cut = vec![false; bw * bh];
        let mut bubbles_here = Vec::new();
        for (l, &n) in counts.iter().enumerate().skip(1) {
            let eqd = 2.0 * (n as f64 / std::f64::consts::PI).sqrt() * k;
            if !(12.0..=1.2 * r).contains(&eqd) {
                continue;
            }
            let part: Vec<bool> = lab.iter().map(|&v| v as usize == l).collect();
            let Some(mut b) = shape::grain_from_mask(&part, bw, bh, o, w, 0.8, false) else { continue };
            let s = shape_stats(&b, k);
            if s.circularity <= 0.7 || s.solidity <= 0.85 || s.aspect >= 1.4 || b.area_um2 > 0.7 * g.area_um2 {
                continue;
            }
            let c = [b.centroid_um[0] / k - o.0 as f64, b.centroid_um[1] / k - o.1 as f64];
            let rad = b.eq_diameter_um / 2.0 / k + 20.0 / k;
            if circle_outside(&mask, bw, bh, c, rad) < 0.45 {
                continue; // ringed by the grain: a CL-dark core
            }
            let grown = morph::dilate(&part, bw, bh, halo);
            // The halo is round too: cut the disc of the bubble plus its halo.
            let rr = b.eq_diameter_um / 2.0 / k + halo as f64;
            for y in 0..bh {
                for x in 0..bw {
                    let i = y * bw + x;
                    if grown[i] || (x as f64 + 0.5 - c[0]).hypot(y as f64 + 0.5 - c[1]) <= rr {
                        cut[i] = true;
                    }
                }
            }
            b.artifact = Some(Artifact::Bubble);
            bubbles_here.push(b);
        }
        if bubbles_here.is_empty() {
            continue;
        }
        let rest: Vec<bool> = mask.iter().zip(&cut).map(|(m, c)| *m && !*c).collect();
        let rest = morph::open(&rest, bw, bh, 1);
        let (rl, rc) = morph::components(&rest, bw, bh);
        let Some((best, bn)) = rc.iter().copied().enumerate().skip(1).max_by_key(|&(_, n)| n) else { continue };
        if (bn as f64) * k * k < 0.25 * g.area_um2 {
            continue; // little left: let the classifier judge the whole
        }
        let keep: Vec<bool> = rl.iter().map(|&v| v as usize == best).collect();
        if let Some(mut q) = shape::grain_from_mask(&keep, bw, bh, o, w, p.simplify_px, false) {
            q.id = g.id;
            q.manual = g.manual;
            q.mean_intensity = mean_inside(&w.img, &q, k);
            *g = q;
            for b in &bubbles_here {
                an.bubbles.push(Bubble { c: b.centroid_um, r: b.eq_diameter_um / 2.0 });
            }
            found.extend(bubbles_here);
        }
    }
    grains.extend(found);
}

/// Splits grains that are several grains bridged by a seam line or by the footprint closing:
/// the grain's own outline opened at 1.65 x the seam radius (6.6 um on the S mount) falls apart
/// into two or more compact parts (solidity >= 0.75, over [`rel::fragment`] of the reference
/// size); each part grows back over the outline by up to 8 um and becomes a grain, the largest
/// keeping the id. The test is on the outline, so it works whether the parts are bright or
/// CL-dark.
pub fn split_bridged(grains: &mut Vec<Grain>, w: &WorkImage, p: &SegmentParams, r: f64) {
    if !p.flag_slivers {
        return;
    }
    let k = w.um_per_px;
    let rr = ((1.65 * rel::seam_open(r) / k).round() as usize).max(2);
    let grow = ((8.0 / k).round() as usize).max(2);
    let mut out: Vec<Grain> = Vec::with_capacity(grains.len());
    for g in grains.drain(..) {
        if !g.is_grain() || g.eq_diameter_um < 2.0 * rel::fragment(r) {
            out.push(g);
            continue;
        }
        let (mask, bw, bh, o) = grain_raster(&g, k, rr + 2);
        let opened = morph::open(&mask, bw, bh, rr);
        let (lab, counts) = morph::components(&opened, bw, bh);
        let min_px = std::f64::consts::PI / 4.0 * (rel::fragment(r) / k).powi(2);
        let parts: Vec<usize> = (1..counts.len()).filter(|&l| counts[l] as f64 >= min_px).collect();
        if parts.len() < 2 {
            out.push(g);
            continue;
        }
        // Each pixel of the outline goes to the nearest part, within `grow`.
        let mut owner = vec![0usize; bw * bh];
        for (i, &l) in lab.iter().enumerate() {
            if parts.contains(&(l as usize)) {
                owner[i] = l as usize;
            }
        }
        for _ in 0..grow {
            let prev = owner.clone();
            for y in 0..bh {
                for x in 0..bw {
                    let i = y * bw + x;
                    if prev[i] != 0 || !mask[i] {
                        continue;
                    }
                    let nb = [(x > 0).then(|| i - 1), (x + 1 < bw).then(|| i + 1), (y > 0).then(|| i - bw), (y + 1 < bh).then(|| i + bw)];
                    if let Some(j) = nb.into_iter().flatten().find(|&j| prev[j] != 0) {
                        owner[i] = prev[j];
                    }
                }
            }
        }
        let mut pieces: Vec<Grain> = parts
            .iter()
            .filter_map(|&l| {
                let m: Vec<bool> = owner.iter().map(|&v| v == l).collect();
                let m = morph::fill_holes(&m, bw, bh);
                shape::grain_from_mask(&m, bw, bh, o, w, p.simplify_px, false)
            })
            .filter(|q| shape_stats(q, k).solidity >= 0.75)
            .collect();
        if pieces.len() < 2 {
            out.push(g);
            continue;
        }
        pieces.sort_by(|a, b| b.area_um2.total_cmp(&a.area_um2));
        pieces[0].id = g.id;
        pieces[0].manual = g.manual;
        for q in pieces.iter_mut() {
            q.mean_intensity = mean_inside(&w.img, q, k);
        }
        out.extend(pieces);
    }
    *grains = out;
}

/// A part split off a grain by hand that is an epoxy bubble: round and well below the epoxy
/// level around the grain (the Split tool flags it, so one cut detaches a bubble).
pub fn looks_like_bubble(part: &Grain, grain: &Grain, w: &WorkImage) -> bool {
    let k = w.um_per_px;
    let s = shape_stats(part, k);
    if s.circularity <= 0.7 || s.solidity <= 0.85 || s.aspect >= 1.4 || !(12.0..=300.0).contains(&part.eq_diameter_um) {
        return false;
    }
    // The epoxy: a ring 6 to 14 um outside the original grain.
    let (inner, bw, bh, o) = shape::rasterize(grain, k, ((16.0 / k).ceil() as usize).max(4));
    let a = morph::dilate(&inner, bw, bh, ((14.0 / k).round() as usize).max(2));
    let b = morph::dilate(&inner, bw, bh, ((6.0 / k).round() as usize).max(1));
    let (mut sum, mut n) = (0.0f64, 0usize);
    for y in 0..bh {
        for x in 0..bw {
            let i = y * bw + x;
            let (gx, gy) = (o.0 + x, o.1 + y);
            if a[i] && !b[i] && gx < w.img.w && gy < w.img.h && w.cov[gy * w.img.w + gx] {
                sum += w.img.px[gy * w.img.w + gx] as f64;
                n += 1;
            }
        }
    }
    n > 0 && mean_inside(&w.img, part, k) < 0.7 * (sum / n as f64)
}
