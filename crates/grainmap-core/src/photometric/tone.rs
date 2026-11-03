//! Global tone: percentile stretch, black / white / gamma handles, CLAHE, histogram.
//!
//! The base mosaic (pyramid, hash) carries the percentile stretch and, when
//! enabled, CLAHE. Black, white and gamma are a 256-entry lookup table applied
//! on top, identically by the UI (display) and by the core (exports, print).

use serde::{Deserialize, Serialize};

use crate::image::Gray;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ToneParams {
    /// Low and high percentiles of the stretch (percent).
    pub low_pct: f32,
    pub high_pct: f32,
    /// Handles on the stretched 0..1 scale.
    pub black: f32,
    pub white: f32,
    pub gamma: f32,
    pub clahe: bool,
    /// CLAHE tile size (px) and clip limit (multiple of the mean bin count).
    pub clahe_tile: usize,
    pub clahe_clip: f32,
}

impl Default for ToneParams {
    fn default() -> Self {
        Self { low_pct: 0.1, high_pct: 99.9, black: 0.0, white: 1.0, gamma: 1.0, clahe: false, clahe_tile: 64, clahe_clip: 2.5 }
    }
}

/// 1024-bin histogram of covered pixels in 0..1.
pub fn histogram(g: &Gray, cov: Option<&[f32]>, bins: usize) -> Vec<u64> {
    let mut h = vec![0u64; bins];
    for (i, &v) in g.px.iter().enumerate() {
        if cov.is_some_and(|c| c[i] < 0.5) {
            continue;
        }
        h[((v.clamp(0.0, 1.0) * (bins - 1) as f32) + 0.5) as usize] += 1;
    }
    h
}

/// Value at a percentile of a histogram over 0..1.
pub fn percentile(h: &[u64], pct: f32) -> f32 {
    let n: u64 = h.iter().sum();
    if n == 0 {
        return 0.0;
    }
    let target = (pct.clamp(0.0, 100.0) as f64 / 100.0 * n as f64) as u64;
    let mut acc = 0u64;
    for (i, &c) in h.iter().enumerate() {
        acc += c;
        if acc > target {
            return i as f32 / (h.len() - 1) as f32;
        }
    }
    1.0
}

/// Percentile stretch to 0..1 (values clamp). Returns the (low, high) levels used.
pub fn stretch(g: &mut Gray, cov: Option<&[f32]>, p: &ToneParams) -> (f32, f32) {
    let h = histogram(g, cov, 4096);
    let lo = percentile(&h, p.low_pct);
    let mut hi = percentile(&h, p.high_pct);
    if hi <= lo + 1e-4 {
        hi = lo + 1e-4;
    }
    let k = 1.0 / (hi - lo);
    for v in g.px.iter_mut() {
        *v = ((*v - lo) * k).clamp(0.0, 1.0);
    }
    (lo, hi)
}

/// Black / white / gamma lookup table on 8-bit values.
pub fn lut(p: &ToneParams) -> [u8; 256] {
    let mut t = [0u8; 256];
    let span = (p.white - p.black).max(1e-4);
    let inv_g = 1.0 / p.gamma.max(0.01);
    for (i, o) in t.iter_mut().enumerate() {
        let v = ((i as f32 / 255.0 - p.black) / span).clamp(0.0, 1.0);
        *o = crate::util::to_u8(libm::powf(v, inv_g));
    }
    t
}

/// Contrast-limited adaptive histogram equalisation on a 0..1 image (256 bins per tile,
/// bilinear interpolation of the tile mappings; Zuiderveld, Graphics Gems IV, 1994).
pub fn clahe(g: &mut Gray, tile: usize, clip: f32) {
    let tile = tile.max(8);
    let (tw, th) = (g.w.div_ceil(tile), g.h.div_ceil(tile));
    let maps: Vec<[f32; 256]> = crate::util::par_map(tw * th, |k| {
        let (cx, cy) = (k % tw, k / tw);
        let mut hist = [0f32; 256];
        let mut n = 0f32;
        for y in cy * tile..((cy + 1) * tile).min(g.h) {
            for x in cx * tile..((cx + 1) * tile).min(g.w) {
                hist[(g.px[y * g.w + x].clamp(0.0, 1.0) * 255.0 + 0.5) as usize] += 1.0;
                n += 1.0;
            }
        }
        let limit = (clip * n / 256.0).max(1.0);
        let mut excess = 0.0;
        for h in hist.iter_mut() {
            if *h > limit {
                excess += *h - limit;
                *h = limit;
            }
        }
        let add = excess / 256.0;
        let mut m = [0f32; 256];
        let mut acc = 0.0;
        for (i, h) in hist.iter().enumerate() {
            acc += h + add;
            m[i] = acc / n.max(1.0);
        }
        m
    });
    let src = g.px.clone();
    let w = g.w;
    crate::util::par_rows(&mut g.px, w, |y, row| {
        let fy = (y as f32 + 0.5) / tile as f32 - 0.5;
        let y0 = fy.floor().clamp(0.0, (th - 1) as f32) as usize;
        let y1 = (y0 + 1).min(th - 1);
        let ay = (fy - y0 as f32).clamp(0.0, 1.0);
        for (x, o) in row.iter_mut().enumerate() {
            let fx = (x as f32 + 0.5) / tile as f32 - 0.5;
            let x0 = fx.floor().clamp(0.0, (tw - 1) as f32) as usize;
            let x1 = (x0 + 1).min(tw - 1);
            let ax = (fx - x0 as f32).clamp(0.0, 1.0);
            let b = (src[y * w + x].clamp(0.0, 1.0) * 255.0 + 0.5) as usize;
            let v00 = maps[y0 * tw + x0][b];
            let v10 = maps[y0 * tw + x1][b];
            let v01 = maps[y1 * tw + x0][b];
            let v11 = maps[y1 * tw + x1][b];
            *o = (v00 * (1.0 - ax) + v10 * ax) * (1.0 - ay) + (v01 * (1.0 - ax) + v11 * ax) * ay;
        }
    });
}
