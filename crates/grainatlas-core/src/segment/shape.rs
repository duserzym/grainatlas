//! Outlines and statistics of one grain mask, rasterisation of polygons, and
//! GrabCut refinement through `photocraft_algo::segment::grabcut`.

use photocraft_algo::segment::RgbImage;
use photocraft_algo::segment::grabcut::{BG, FG, PR_BG, PR_FG, grabcut};

use super::{Grain, WorkImage, morph};
use crate::image::Gray;

const DIRS: [(i64, i64); 8] = [(-1, 0), (-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1)];

/// Outer boundary of the largest component (Moore-neighbour tracing, Jacob's stopping
/// criterion), as pixel centres.
pub fn trace(mask: &[bool], w: usize, h: usize) -> Vec<[f64; 2]> {
    let fg = |x: i64, y: i64| x >= 0 && y >= 0 && x < w as i64 && y < h as i64 && mask[y as usize * w + x as usize];
    let Some(s) = (0..w * h).find(|&i| mask[i]) else { return Vec::new() };
    let s = ((s % w) as i64, (s / w) as i64);
    let mut p = s;
    let mut b = (s.0 - 1, s.1);
    let b0 = b;
    let mut out = vec![[s.0 as f64 + 0.5, s.1 as f64 + 0.5]];
    let limit = 4 * mask.len() + 16;
    for _ in 0..limit {
        let bd = DIRS.iter().position(|&(dx, dy)| (p.0 + dx, p.1 + dy) == b).unwrap_or(0);
        let mut found = None;
        for k in 1..=8 {
            let d = (bd + k) % 8;
            let q = (p.0 + DIRS[d].0, p.1 + DIRS[d].1);
            if fg(q.0, q.1) {
                let pd = (d + 7) % 8;
                found = Some((q, (p.0 + DIRS[pd].0, p.1 + DIRS[pd].1)));
                break;
            }
        }
        let Some((q, nb)) = found else { break };
        p = q;
        b = nb;
        if p == s && b == b0 {
            break;
        }
        if p == s && out.len() > 2 {
            // Back at the start from another side: closed.
            break;
        }
        out.push([p.0 as f64 + 0.5, p.1 as f64 + 0.5]);
    }
    out
}

/// Douglas-Peucker simplification of a closed polygon.
pub fn simplify(poly: &[[f64; 2]], eps: f64) -> Vec<[f64; 2]> {
    if poly.len() < 5 || eps <= 0.0 {
        return poly.to_vec();
    }
    let a = poly[0];
    let m = (0..poly.len()).max_by(|&i, &j| dist2(poly[i], a).total_cmp(&dist2(poly[j], a))).unwrap_or(0);
    let mut first = dp(&poly[..=m], eps);
    let mut second: Vec<[f64; 2]> = poly[m..].to_vec();
    second.push(a);
    let second = dp(&second, eps);
    first.pop();
    first.extend_from_slice(&second[..second.len() - 1]);
    first
}

fn dist2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

fn dp(pts: &[[f64; 2]], eps: f64) -> Vec<[f64; 2]> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dy);
    let (mut idx, mut dmax) = (0, 0.0);
    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let d = if len < 1e-12 { dist2(*p, a).sqrt() } else { ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len };
        if d > dmax {
            dmax = d;
            idx = i;
        }
    }
    if dmax > eps {
        let mut l = dp(&pts[..=idx], eps);
        let r = dp(&pts[idx..], eps);
        l.pop();
        l.extend(r);
        l
    } else {
        vec![a, b]
    }
}

/// Builds a grain from a local mask placed at `origin` (work pixels). Keeps the largest component.
pub fn grain_from_mask(mask: &[bool], bw: usize, bh: usize, origin: (usize, usize), w: &WorkImage, simplify_px: f64, manual: bool) -> Option<Grain> {
    let (lab, counts) = morph::components(mask, bw, bh);
    let best = (1..counts.len()).max_by_key(|&l| counts[l])?;
    let m: Vec<bool> = lab.iter().map(|&l| l as usize == best).collect();
    let k = w.um_per_px;
    let (mut n, mut sx, mut sy, mut sxx, mut syy, mut sxy, mut si) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    for y in 0..bh {
        for x in 0..bw {
            if !m[y * bw + x] {
                continue;
            }
            let (gx, gy) = ((origin.0 + x) as f64 + 0.5, (origin.1 + y) as f64 + 0.5);
            n += 1.0;
            sx += gx;
            sy += gy;
            sxx += gx * gx;
            syy += gy * gy;
            sxy += gx * gy;
            let (ix, iy) = (origin.0 + x, origin.1 + y);
            if ix < w.img.w && iy < w.img.h {
                si += w.img.px[iy * w.img.w + ix] as f64;
            }
        }
    }
    if n < 3.0 {
        return None;
    }
    let (cx, cy) = (sx / n, sy / n);
    let (vxx, vyy, vxy) = (sxx / n - cx * cx, syy / n - cy * cy, sxy / n - cx * cy);
    let tr = vxx + vyy;
    let disc = ((vxx - vyy).powi(2) / 4.0 + vxy * vxy).sqrt();
    let (l1, l2) = (tr / 2.0 + disc, (tr / 2.0 - disc).max(0.0));
    let orientation = 0.5 * (2.0 * vxy).atan2(vxx - vyy);
    let poly_px = simplify(&trace(&m, bw, bh), simplify_px);
    let polygon_um: Vec<[f64; 2]> = poly_px.iter().map(|p| [(origin.0 as f64 + p[0]) * k, (origin.1 as f64 + p[1]) * k]).collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in &polygon_um {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    let area = n * k * k;
    Some(Grain {
        id: crate::project::new_id(),
        number: 0,
        polygon_um,
        bbox_um: [x0 - 0.5 * k, y0 - 0.5 * k, x1 + 0.5 * k, y1 + 0.5 * k],
        centroid_um: [cx * k, cy * k],
        area_um2: area,
        eq_diameter_um: 2.0 * (area / std::f64::consts::PI).sqrt(),
        major_um: 4.0 * l1.sqrt() * k,
        minor_um: 4.0 * l2.sqrt() * k,
        orientation_deg: orientation.to_degrees(),
        mean_intensity: si / n,
        manual,
        scale_bar_um: crate::scale::grain_bar_um(x1 - x0),
        artifact: None,
        kept: false,
        drawn: false,
    })
}

/// Point-in-polygon test of a work pixel centre.
pub fn inside_polygon_px(g: &Grain, k: f64, origin: (usize, usize), x: usize, y: usize) -> bool {
    let (px, py) = (((origin.0 + x) as f64 + 0.5) * k, ((origin.1 + y) as f64 + 0.5) * k);
    inside(&g.polygon_um, px, py)
}

pub fn inside(poly: &[[f64; 2]], px: f64, py: f64) -> bool {
    let mut c = false;
    let n = poly.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > py) != (b[1] > py) && px < (b[0] - a[0]) * (py - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
        j = i;
    }
    c
}

/// Mask of a grain polygon at work resolution over its bbox grown by `pad` pixels.
pub fn rasterize(g: &Grain, k: f64, pad: usize) -> (Vec<bool>, usize, usize, (usize, usize)) {
    let x0 = ((g.bbox_um[0] / k).floor() as i64 - pad as i64).max(0) as usize;
    let y0 = ((g.bbox_um[1] / k).floor() as i64 - pad as i64).max(0) as usize;
    let x1 = (g.bbox_um[2] / k).ceil() as usize + pad;
    let y1 = (g.bbox_um[3] / k).ceil() as usize + pad;
    let (bw, bh) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
    let mut m = vec![false; bw * bh];
    for y in 0..bh {
        for x in 0..bw {
            m[y * bw + x] = inside_polygon_px(g, k, (x0, y0), x, y);
        }
    }
    (m, bw, bh, (x0, y0))
}

/// GrabCut on a crop around a grain mask (dilated band = probable background, eroded core =
/// definite foreground). Returns the refined mask, its origin and size.
pub fn grabcut_refine(img: &Gray, mask: &[bool], origin: (usize, usize), bw: usize, bh: usize) -> Option<(Vec<bool>, (usize, usize), usize, usize)> {
    let margin = ((bw.max(bh) as f64 * 0.25) as usize).clamp(6, 40);
    let ox = origin.0.saturating_sub(margin);
    let oy = origin.1.saturating_sub(margin);
    let ex = (origin.0 + bw + margin).min(img.w);
    let ey = (origin.1 + bh + margin).min(img.h);
    let (cw, ch) = (ex - ox, ey - oy);
    if cw < 8 || ch < 8 {
        return None;
    }
    let mut m = vec![false; cw * ch];
    for y in 0..bh {
        for x in 0..bw {
            let (gx, gy) = (origin.0 + x - ox, origin.1 + y - oy);
            if gx < cw && gy < ch {
                m[gy * cw + gx] = mask[y * bw + x];
            }
        }
    }
    let core = morph::erode(&m, cw, ch, 2);
    let band = morph::dilate(&m, cw, ch, (margin / 2).max(3));
    let mut trimap: Vec<u8> = (0..cw * ch).map(|i| if core[i] { FG } else if m[i] { PR_FG } else if band[i] { PR_BG } else { BG }).collect();
    let crop = img.crop(ox, oy, cw, ch);
    let rgb = RgbImage { w: cw, h: ch, px: crop.px.iter().map(|&v| [v, v, v]).collect() };
    grabcut(&rgb, &mut trimap, 4)?;
    let out: Vec<bool> = trimap.iter().map(|&t| t == FG || t == PR_FG).collect();
    let out = morph::fill_holes(&out, cw, ch);
    Some((out, (ox, oy), cw, ch))
}
