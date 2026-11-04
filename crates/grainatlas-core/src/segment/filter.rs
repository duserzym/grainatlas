//! Second-pass segmentation (Boise SEM run, 2026-11-03): the deviation from the
//! local epoxy level, CL-dark (metamict) grains that `Polarity::Bright` misses,
//! epoxy bubbles, tile-seam slivers and bright exposure-step tile blocks.
//!
//! The primary segmenter stays as it is. [`analyse`] measures the mosaic once;
//! [`reject`] removes primary grains that are bubbles, bubble halos, slivers or
//! tile blocks; [`supplement`] finds grains of either polarity relative to the
//! local epoxy level, and the caller adds those that overlap no primary grain.

use serde::{Deserialize, Serialize};

use super::{Grain, SegmentParams, WorkImage, morph, shape, watershed};
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
    /// Robust noise of the deviation from the local epoxy level (normalised units).
    pub sigma: f32,
    /// Bright and dark thresholds on the deviation.
    pub t_bright: f32,
    pub t_dark: f32,
    pub bubbles: Vec<Bubble>,
    #[serde(skip)]
    pub dev: Option<Gray>,
    #[serde(skip)]
    pub dark: Vec<bool>,
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

/// Measures the deviation from the local epoxy level and finds bubbles. Values are normalised
/// first (covered p1 → 0, p99.5 → 1) so the thresholds do not depend on the tone.
pub fn analyse(w: &WorkImage, p: &SegmentParams) -> Analysis {
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
    if p.reject_bubbles {
        let (lab, counts) = morph::components(&dark, iw, ih);
        let gs = grains_of_labels(&lab, &counts, iw, ih, w, eqd_px(12.0, k).powi(2) * std::f64::consts::PI / 4.0, 0.8);
        for (_, g) in gs {
            let s = shape_stats(&g, k);
            if (12.0..=75.0).contains(&g.eq_diameter_um) && s.circularity > 0.72 && s.solidity > 0.88 && s.aspect < 1.35 {
                bubbles.push(Bubble { c: g.centroid_um, r: g.eq_diameter_um / 2.0 });
            }
        }
    }
    Analysis { sigma, t_bright, t_dark: td, bubbles, dev: Some(dev), dark }
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
/// (aspect > 5), under 30 um wide and straight along a tile axis (its outline hugs its bounding
/// box): a zircon needle, even one lying along an axis, has tapering ends and stays.
pub fn is_sliver(g: &Grain) -> bool {
    g.minor_um < 15.0 || (g.major_um / g.minor_um.max(1e-6) > 5.0 && g.minor_um < 30.0 && box_fraction(g, 3.0) > 0.5)
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

/// A bright exposure-step tile block (or the step region next to one): larger than
/// `max_size_um`; or with almost no pixels that stand out from the local epoxy level (it was
/// only above the global threshold because its tile was exposed brighter); or weakly contrasted
/// and either larger than any grain of the run (260 um) or bounded by its box (tile edges).
pub fn is_block(g: &Grain, contrast: f64, max_size_um: f64) -> bool {
    (max_size_um > 0.0 && g.eq_diameter_um > max_size_um) || contrast < 0.15 || (contrast < 0.5 && (g.eq_diameter_um > 260.0 || box_fraction(g, 3.0) > 0.45))
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

/// The bright rim of a bubble (12 to 120 um): an arc or a ring on a circle, or a round blob
/// whose middle is no brighter than the epoxy.
pub fn is_bubble_rim(g: &Grain, s: &ShapeStats, dev: Option<&Gray>, k: f64, tb: f32) -> bool {
    let Some((c, r, rms)) = fit_circle(&g.polygon_um) else { return false };
    if !(6.0..=60.0).contains(&r) || rms > 0.15 * r + k {
        return false;
    }
    // An arc: on a circle but thin and hollow.
    if s.solidity < 0.75 && thickness(g) < 0.36 * g.eq_diameter_um {
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

/// A bubble's halo (or the bubble itself) mistaken for a grain.
pub fn is_bubble_halo(g: &Grain, bubbles: &[Bubble]) -> bool {
    bubbles.iter().any(|b| (g.centroid_um[0] - b.c[0]).hypot(g.centroid_um[1] - b.c[1]) < b.r + 12.0 && g.eq_diameter_um < 2.2 * b.r + 25.0)
}

/// Removes bubbles and their rims, slivers and tile blocks from `grains` (as `p` enables).
/// Returns how many of each were dropped.
pub fn reject(grains: &mut Vec<Grain>, p: &SegmentParams, an: &Analysis, k: f64) -> [usize; 3] {
    let debug = std::env::var_os("GRAINATLAS_DEBUG_SEG").is_some();
    let dev = an.dev.as_ref();
    let verdict: Vec<usize> = crate::util::par_map(grains.len(), |i| {
        let g = &grains[i];
        let s = shape_stats(g, k);
        let contrast = dev.map_or(1.0, |d| contrast_fraction(d, g, k, an.t_bright, an.t_dark));
        let bright = dev.is_none_or(|d| mean_inside(d, g, k) > 0.0);
        let v = if p.reject_bubbles && ((bright && is_bubble_halo(g, &an.bubbles)) || is_bubble_rim(g, &s, dev, k, an.t_bright)) {
            0
        } else if p.reject_slivers && is_sliver(g) {
            1
        } else if p.reject_blocks && is_block(g, contrast, p.max_size_um) {
            2
        } else {
            3
        };
        if debug {
            let circle = fit_circle(&g.polygon_um).map_or((0.0, 0.0), |(_, r, e)| (r, e / r));
            eprintln!(
                "seg {} c=({:.0},{:.0}) eqd={:.0} min={:.0} or={:.0} sol={:.2} circ={:.2} asp={:.2} thick={:.2} contrast={:.2} box={:.2} r={:.0} rms/r={:.2}",
                ["bubble", "sliver", "block", "keep"][v], g.centroid_um[0], g.centroid_um[1], g.eq_diameter_um, g.minor_um, g.orientation_deg, s.solidity, s.circularity, s.aspect, thickness(g) / g.eq_diameter_um, contrast, box_fraction(g, 3.0), circle.0, circle.1
            );
        }
        v
    });
    let mut n = [0usize; 3];
    let mut i = 0;
    grains.retain(|_| {
        let v = verdict[i];
        i += 1;
        if v < 3 {
            n[v] += 1;
        }
        v == 3
    });
    n
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

/// Grains of either polarity relative to the local epoxy level (mostly the CL-dark grains the
/// primary segmenter misses), without bubbles, slivers and tile blocks.
pub fn supplement(w: &WorkImage, p: &SegmentParams, an: &Analysis) -> Vec<Grain> {
    let Some(dev) = &an.dev else { return Vec::new() };
    let k = w.um_per_px;
    let (iw, ih) = (dev.w, dev.h);
    let mut bub = vec![false; iw * ih];
    for b in &an.bubbles {
        let (cx, cy, r) = (b.c[0] / k, b.c[1] / k, b.r / k + 0.5);
        let (x0, x1) = ((cx - r).floor().max(0.0) as usize, ((cx + r).ceil() as usize).min(iw.saturating_sub(1)));
        let (y0, y1) = ((cy - r).floor().max(0.0) as usize, ((cy + r).ceil() as usize).min(ih.saturating_sub(1)));
        for y in y0..=y1 {
            for x in x0..=x1 {
                if (x as f64 + 0.5 - cx).hypot(y as f64 + 0.5 - cy) <= r {
                    bub[y * iw + x] = true;
                }
            }
        }
    }
    let zone = morph::dilate(&bub, iw, ih, ((9.0 / k).round() as usize).max(3));
    let mut m: Vec<bool> = (0..iw * ih).map(|i| w.cov[i] && ((dev.px[i] > an.t_bright && !zone[i]) || (an.dark[i] && !bub[i]))).collect();
    m = morph::close(&m, iw, ih, 3);
    m = morph::fill_holes(&m, iw, ih);
    m = morph::open(&m, iw, ih, 2);
    m = morph::fill_holes(&m, iw, ih);
    let lab = watershed::split(&m, iw, ih, p.split_h, p.neck_ratio);
    let n = lab.iter().copied().max().unwrap_or(0) as usize;
    let counts = vec![0usize; n + 1];
    let min_px = std::f64::consts::PI / 4.0 * (28.0f64.max(p.min_size_um) / k).powi(2);
    let mut out = Vec::new();
    for (_, mut g) in grains_of_labels(&lab, &counts, iw, ih, w, min_px, p.simplify_px) {
        let s = shape_stats(&g, k);
        if is_sliver(&g) || s.solidity < 0.45 || g.area_um2 > 0.5e6 || (p.max_size_um > 0.0 && g.eq_diameter_um > p.max_size_um) {
            continue;
        }
        let contrast = contrast_fraction(dev, &g, k, an.t_bright, an.t_dark);
        if p.reject_blocks && is_block(&g, contrast, 0.0) {
            continue;
        }
        if p.reject_bubbles && is_bubble_rim(&g, &s, Some(dev), k, an.t_bright) {
            continue;
        }
        let md = mean_inside(dev, &g, k);
        if g.eq_diameter_um < 60.0 && s.circularity > 0.78 && s.aspect < 1.3 && s.solidity > 0.92 && (md < 0.0 || g.eq_diameter_um < 45.0) {
            continue; // a bubble with its halo
        }
        if md > 0.0 && is_bubble_halo(&g, &an.bubbles) {
            continue;
        }
        g.mean_intensity = mean_inside(&w.img, &g, k);
        out.push(g);
    }
    out
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

/// Adds supplement grains to the primary ones: a supplement grain that overlaps no primary grain
/// is added; one that covers primary grains and is at least 1.5 times their area replaces them
/// (the primary pass found only the bright part of a mostly CL-dark grain) and keeps the id of
/// the largest; otherwise the primary grains stay.
pub fn merge_supplement(grains: &mut Vec<Grain>, sup: Vec<Grain>) {
    for s in sup {
        let hit: Vec<usize> = (0..grains.len()).filter(|&i| overlaps(&grains[i], &s)).collect();
        if hit.is_empty() {
            grains.push(s);
            continue;
        }
        let area: f64 = hit.iter().map(|&i| grains[i].area_um2).sum();
        let inside = hit.iter().all(|&i| shape::inside(&s.polygon_um, grains[i].centroid_um[0], grains[i].centroid_um[1]));
        if s.area_um2 >= 1.5 * area && inside {
            let keep = *hit.iter().max_by(|&&a, &&b| grains[a].area_um2.total_cmp(&grains[b].area_um2)).expect("hit");
            let mut s = s;
            s.id = grains[keep].id;
            s.manual = grains[keep].manual;
            grains[keep] = s;
            let mut drop: Vec<usize> = hit.into_iter().filter(|&i| i != keep).collect();
            drop.sort_unstable_by(|a, b| b.cmp(a));
            for i in drop {
                grains.remove(i);
            }
        }
    }
}
