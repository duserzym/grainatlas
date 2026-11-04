//! Export-only enhancement of a mosaic crop: non-local means denoising (the
//! fast integral-image form of Darbon et al. 2008, with the weights of
//! Buades et al.), unsharp masking, and background stripping (everything
//! outside the dilated grain outlines becomes one grey). None of this touches
//! the stored mosaic.

use serde::{Deserialize, Serialize};

use crate::image::Gray;
use crate::photometric::background::gaussian_blur;
use crate::segment::Grain;
use crate::segment::filter::convex_hull;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EnhanceParams {
    /// Non-local means denoising.
    pub denoise: bool,
    /// Filter strength as a multiple of the estimated noise.
    pub denoise_h: f32,
    /// Unsharp mask.
    pub sharpen: bool,
    /// Gaussian radius (px) and amount of the unsharp mask.
    pub sharpen_radius: f32,
    pub sharpen_amount: f32,
    /// Everything outside the dilated grain outlines becomes `strip_grey`.
    pub strip: bool,
    /// Grey of the stripped background (0 black to 255 white).
    pub strip_grey: u8,
    /// Margin kept around each grain's outline (um).
    pub strip_margin_um: f64,
}

impl Default for EnhanceParams {
    fn default() -> Self {
        EnhanceParams { denoise: false, denoise_h: 0.7, sharpen: false, sharpen_radius: 1.6, sharpen_amount: 0.55, strip: false, strip_grey: 22, strip_margin_um: 6.0 }
    }
}

impl EnhanceParams {
    pub fn any(&self) -> bool {
        self.denoise || self.sharpen || self.strip
    }
}

/// Noise standard deviation of an image (Immerkær 1996: the Laplacian-difference operator).
pub fn estimate_noise(g: &Gray) -> f32 {
    if g.w < 3 || g.h < 3 {
        return 0.0;
    }
    let mut s = 0.0f64;
    for y in 1..g.h - 1 {
        for x in 1..g.w - 1 {
            let a = |dx: i64, dy: i64| g.px[(y as i64 + dy) as usize * g.w + (x as i64 + dx) as usize] as f64;
            let v = a(-1, -1) - 2.0 * a(0, -1) + a(1, -1) - 2.0 * a(-1, 0) + 4.0 * a(0, 0) - 2.0 * a(1, 0) + a(-1, 1) - 2.0 * a(0, 1) + a(1, 1);
            s += v.abs();
        }
    }
    ((std::f64::consts::PI / 2.0).sqrt() * s / (6.0 * (g.w - 2) as f64 * (g.h - 2) as f64)) as f32
}

/// Non-local means with `patch` x `patch` patches searched over a `2·search+1` window. `h` and
/// `sigma` are in image units; the weight of a patch at mean squared distance d² is
/// exp(-max(d² - 2σ², 0) / h²).
pub fn nl_means(g: &Gray, h: f32, sigma: f32, patch: usize, search: usize) -> Gray {
    let (w, ht) = (g.w, g.h);
    if w == 0 || ht == 0 || h <= 0.0 {
        return g.clone();
    }
    let pr = (patch / 2) as i64;
    let sr = search as i64;
    let pad = pr + sr;
    let area = ((2 * pr + 1) * (2 * pr + 1)) as f32;
    let (h2, s2) = (h * h, 2.0 * sigma * sigma);
    const STRIP: usize = 32;
    let strips = ht.div_ceil(STRIP);
    let rows: Vec<Vec<f32>> = crate::util::par_map(strips, |si| {
        let (y0, y1) = (si * STRIP, ((si + 1) * STRIP).min(ht));
        // Padded region: rows y0-pad .. y1+pad, cols -pad .. w+pad (clamped samples).
        let (pw, ph) = (w + 2 * pad as usize, (y1 - y0) + 2 * pad as usize);
        let at = |x: i64, y: i64| g.atc(x, y);
        let mut acc = vec![0f32; (y1 - y0) * w];
        let mut wsum = vec![0f32; (y1 - y0) * w];
        let mut wmax = vec![0f32; (y1 - y0) * w];
        let mut integ = vec![0f64; (pw + 1) * (ph + 1)];
        for dy in -sr..=sr {
            for dx in -sr..=sr {
                if dx == 0 && dy == 0 {
                    continue;
                }
                // Integral image of (I(p) - I(p+d))² over the region needed for the patches.
                for yy in 0..ph {
                    let y = y0 as i64 - pad + yy as i64;
                    let mut row = 0f64;
                    for xx in 0..pw {
                        let x = xx as i64 - pad;
                        let d = at(x, y) - at(x + dx, y + dy);
                        row += (d * d) as f64;
                        integ[(yy + 1) * (pw + 1) + xx + 1] = integ[yy * (pw + 1) + xx + 1] + row;
                    }
                }
                for y in y0..y1 {
                    let yy = (y as i64 - y0 as i64 + pad) as usize;
                    for x in 0..w {
                        let xx = x + pad as usize;
                        let (a0, a1) = (xx - pr as usize, xx + pr as usize + 1);
                        let (b0, b1) = (yy - pr as usize, yy + pr as usize + 1);
                        let s = integ[b1 * (pw + 1) + a1] - integ[b0 * (pw + 1) + a1] - integ[b1 * (pw + 1) + a0] + integ[b0 * (pw + 1) + a0];
                        let d2 = s as f32 / area;
                        let wt = (-(d2 - s2).max(0.0) / h2).exp();
                        let i = (y - y0) * w + x;
                        acc[i] += wt * at(x as i64 + dx, y as i64 + dy);
                        wsum[i] += wt;
                        wmax[i] = wmax[i].max(wt);
                    }
                }
            }
        }
        // The pixel itself gets the largest weight any other patch got.
        (0..(y1 - y0) * w)
            .map(|i| {
                let (x, y) = (i % w, y0 + i / w);
                let own = wmax[i].max(1e-6);
                (acc[i] + own * g.px[y * w + x]) / (wsum[i] + own)
            })
            .collect()
    });
    Gray { w, h: ht, px: rows.into_iter().flatten().collect() }
}

/// Unsharp mask: `g + amount · (g − blur(g, radius))`.
pub fn unsharp(g: &Gray, radius: f32, amount: f32) -> Gray {
    let b = gaussian_blur(g, radius);
    Gray { w: g.w, h: g.h, px: g.px.iter().zip(&b.px).map(|(a, b)| a + amount * (a - b)).collect() }
}

/// Denoise and sharpen a 0..1 crop as `p` asks.
pub fn denoise_sharpen(g: &Gray, p: &EnhanceParams) -> Gray {
    let mut out = g.clone();
    if p.denoise {
        let sigma = estimate_noise(&out);
        if sigma > 0.0 {
            out = nl_means(&out, p.denoise_h * sigma, sigma, 5, 6);
        }
    }
    if p.sharpen {
        out = unsharp(&out, p.sharpen_radius, p.sharpen_amount);
    }
    for v in out.px.iter_mut() {
        *v = v.clamp(0.0, 1.0);
    }
    out
}

/// Why a crop's background was not stripped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StripRefusal {
    /// A grain in the crop has no closed outline.
    OpenOutline,
    /// Something grain-like outside every outline (an undetected grain would vanish).
    UnmaskedObject,
}

/// Mask (crop pixels) of the grains' filled convex hulls, grown by `margin_um`. The hull covers a
/// grain whose outline caught only its bright rim or part of it. `None` when a grain that reaches
/// into the crop has no closed outline.
pub fn grain_mask(w: usize, h: usize, origin_um: [f64; 2], um_per_px: f64, grains: &[Grain], margin_um: f64) -> Option<Vec<bool>> {
    let mut m = vec![false; w * h];
    let (x1, y1) = (origin_um[0] + w as f64 * um_per_px, origin_um[1] + h as f64 * um_per_px);
    for g in grains {
        let b = g.bbox_um;
        if b[2] < origin_um[0] - margin_um || b[0] > x1 + margin_um || b[3] < origin_um[1] - margin_um || b[1] > y1 + margin_um {
            continue;
        }
        if g.polygon_um.len() < 3 || crate::segment::filter::polygon_area(&g.polygon_um) <= 0.0 || g.polygon_um.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
            return None;
        }
        let hull: Vec<[f64; 2]> = convex_hull(&g.polygon_um).iter().map(|p| [(p[0] - origin_um[0]) / um_per_px, (p[1] - origin_um[1]) / um_per_px]).collect();
        let gx0 = ((b[0] - origin_um[0]) / um_per_px).floor().max(0.0) as usize;
        let gy0 = ((b[1] - origin_um[1]) / um_per_px).floor().max(0.0) as usize;
        let gx1 = (((b[2] - origin_um[0]) / um_per_px).ceil().max(0.0) as usize).min(w);
        let gy1 = (((b[3] - origin_um[1]) / um_per_px).ceil().max(0.0) as usize).min(h);
        for y in gy0..gy1 {
            for x in gx0..gx1 {
                if crate::segment::shape::inside(&hull, x as f64 + 0.5, y as f64 + 0.5) {
                    m[y * w + x] = true;
                }
            }
        }
    }
    let r = (margin_um / um_per_px).max(1.0);
    let d = crate::segment::morph::distance(&m.iter().map(|v| !v).collect::<Vec<bool>>(), w, h);
    Some(d.iter().map(|&v| v <= r as f32).collect())
}

/// Strips the background of a 0..1 crop: outside `mask` becomes `grey` (0..1) with a soft edge.
/// Refuses when something grain-like lies outside the mask: a smoothed blob at least 30 um across
/// that departs from the background level by 6 robust σ.
pub fn strip_background(g: &mut Gray, mask: &[bool], grey: f32, um_per_px: f64) -> Result<(), StripRefusal> {
    let (w, h) = (g.w, g.h);
    let sm = gaussian_blur(g, (2.0 / um_per_px).max(1.0) as f32);
    let mut out: Vec<f32> = sm.px.iter().zip(mask).filter(|(_, m)| !**m).map(|(v, _)| *v).collect();
    if out.len() > 16 {
        let mid = out.len() / 2;
        let med = *out.select_nth_unstable_by(mid, f32::total_cmp).1;
        let mut dev: Vec<f32> = out.iter().map(|v| (v - med).abs()).collect();
        let mad = 1.4826 * *dev.select_nth_unstable_by(mid, f32::total_cmp).1;
        let thr = 6.0 * mad.max(1.0 / 255.0);
        let odd: Vec<bool> = (0..w * h).map(|i| !mask[i] && (sm.px[i] - med).abs() > thr).collect();
        let odd = crate::segment::morph::open(&odd, w, h, 1);
        let (_, counts) = crate::segment::morph::components(&odd, w, h);
        let min_px = std::f64::consts::PI / 4.0 * (30.0 / um_per_px).powi(2);
        if counts.iter().skip(1).any(|&c| c as f64 >= min_px) {
            return Err(StripRefusal::UnmaskedObject);
        }
    }
    let soft = gaussian_blur(&Gray { w, h, px: mask.iter().map(|&m| if m { 1.0 } else { 0.0 }).collect() }, 1.6);
    for (v, s) in g.px.iter_mut().zip(&soft.px) {
        *v = *v * s + grey * (1.0 - s);
    }
    Ok(())
}
