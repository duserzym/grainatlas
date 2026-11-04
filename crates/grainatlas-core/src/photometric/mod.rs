//! Photometric correction (plan section 6.3): per-tile gain and offset from
//! the overlaps, per-tile background leveling, global tone (percentile stretch,
//! black / white / gamma, CLAHE) and HDR merges. Manual per-tile overrides are
//! parameters, never baked pixels.

pub mod background;
pub mod enhance;
pub mod grain_norm;
pub mod hdr;
pub mod tone;

use photocraft_algo::panorama::{RoiImage, photometric};
use serde::{Deserialize, Serialize};

pub use tone::ToneParams;

use crate::image::Gray;
use crate::mosaic::Placement;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GainModel {
    /// Gain and offset per tile (our least squares; SE and CL have offsets).
    #[default]
    Affine,
    /// Gain only, in log space (`photocraft_algo::panorama::photometric`, vignetting off).
    Gain,
    /// No compensation.
    None,
}

/// A manual per-tile correction; set fields replace the automatic value.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct TileOverride {
    pub index: usize,
    pub gain: Option<f32>,
    pub offset: Option<f32>,
    /// Skip background leveling for this tile.
    #[serde(default)]
    pub no_background: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PhotometricParams {
    pub model: GainModel,
    /// Weight pulling gains to 1 and offsets to 0 (keeps the solve well posed).
    pub regularisation: f64,
    pub background: bool,
    pub background_params: background::BackgroundParams,
    pub overrides: Vec<TileOverride>,
}

impl Default for PhotometricParams {
    fn default() -> Self {
        Self { model: GainModel::Affine, regularisation: 0.01, background: true, background_params: Default::default(), overrides: Vec::new() }
    }
}

/// What the correction found (stored in `project.json` next to the parameters).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct PhotometricResult {
    /// Automatic gain and offset per tile (before overrides).
    pub auto_gain: Vec<f32>,
    pub auto_offset: Vec<f32>,
    /// Applied gain and offset per tile (after overrides).
    pub gain: Vec<f32>,
    pub offset: Vec<f32>,
    /// Median background level added back after leveling.
    pub background_median: f32,
    /// Range (max - min) of each tile's background surface before leveling.
    pub background_range: Vec<f32>,
    /// RMS intensity difference over the overlaps before and after correction (0..1 units).
    pub overlap_rms_before: f32,
    pub overlap_rms_after: f32,
}

/// Overlap samples between placed tiles: (tile a, tile b, value a, value b).
fn overlap_samples(pl: &Placement, tiles: &[Gray], step: usize) -> Vec<(usize, usize, f32, f32)> {
    let mut out = Vec::new();
    for p in &pl.pairs {
        let (ta, tb) = (&pl.tiles[p.a], &pl.tiles[p.b]);
        let x0 = ta.x.max(tb.x);
        let y0 = ta.y.max(tb.y);
        let x1 = (ta.x + ta.w as i64).min(tb.x + tb.w as i64);
        let y1 = (ta.y + ta.h as i64).min(tb.y + tb.h as i64);
        // Stay 4 px clear of the tile edges (scan edge effects).
        let mut y = y0 + 4;
        while y < y1 - 4 {
            let mut x = x0 + 4;
            while x < x1 - 4 {
                let va = tiles[p.a].px[(y - ta.y) as usize * ta.w + (x - ta.x) as usize];
                let vb = tiles[p.b].px[(y - tb.y) as usize * tb.w + (x - tb.x) as usize];
                if (0.002..0.998).contains(&va) && (0.002..0.998).contains(&vb) {
                    out.push((p.a, p.b, va, vb));
                }
                x += step as i64;
            }
            y += step as i64;
        }
    }
    out
}

fn rms(samples: &[(usize, usize, f32, f32)], g: &[f32], o: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let s: f64 = samples.iter().map(|&(a, b, va, vb)| ((g[a] * va + o[a]) - (g[b] * vb + o[b])) as f64).map(|d| d * d).sum();
    (s / samples.len() as f64).sqrt() as f32
}

/// Least-squares gain and offset per tile from the overlaps, with residual trimming.
pub fn fit_affine(n: usize, samples: &[(usize, usize, f32, f32)], reg: f64) -> (Vec<f32>, Vec<f32>) {
    let mut keep: Vec<bool> = vec![true; samples.len()];
    let mut g = vec![1.0f32; n];
    let mut o = vec![0.0f32; n];
    for round in 0..3 {
        // Unknowns: g_0..g_n-1, o_0..o_n-1. Residual per sample: g_a va + o_a - g_b vb - o_b.
        let m = 2 * n;
        let mut ata = vec![0.0f64; m * m];
        let mut atb = vec![0.0f64; m];
        for (k, &(a, b, va, vb)) in samples.iter().enumerate() {
            if !keep[k] {
                continue;
            }
            let row = [(a, va as f64), (n + a, 1.0), (b, -(vb as f64)), (n + b, -1.0)];
            for &(i, ci) in &row {
                for &(j, cj) in &row {
                    ata[i * m + j] += ci * cj;
                }
            }
        }
        // Regularisation: g_i ≈ 1, o_i ≈ 0, relative to the mean data weight of each kind of
        // unknown (so it stays a gentle prior whatever the intensities and sample counts).
        let dg = (0..n).map(|i| ata[i * m + i]).sum::<f64>() / n.max(1) as f64;
        let doff = (0..n).map(|i| ata[(n + i) * m + n + i]).sum::<f64>() / n.max(1) as f64;
        for i in 0..n {
            ata[i * m + i] += reg * dg.max(1e-9);
            atb[i] += reg * dg.max(1e-9);
            ata[(n + i) * m + n + i] += reg * doff.max(1e-9);
        }
        let Some(x) = solve_dense(ata, atb, m) else { break };
        for i in 0..n {
            g[i] = x[i].clamp(0.2, 5.0) as f32;
            o[i] = x[n + i].clamp(-0.5, 0.5) as f32;
        }
        if round == 2 {
            break;
        }
        // Trim samples whose residual is far beyond the typical (misregistered content, charging).
        let mut res: Vec<f32> = samples.iter().map(|&(a, b, va, vb)| ((g[a] * va + o[a]) - (g[b] * vb + o[b])).abs()).collect();
        let mut sorted = res.clone();
        sorted.sort_by(f32::total_cmp);
        let thr = (sorted.get(sorted.len() * 3 / 4).copied().unwrap_or(0.0) * 3.0).max(0.01);
        for (k, r) in res.iter_mut().enumerate() {
            keep[k] = *r <= thr;
        }
    }
    // Normalise so the mean gain is 1 and the mean offset 0 (the solve has a free global scale).
    let mg = g.iter().sum::<f32>() / n.max(1) as f32;
    let mo = o.iter().sum::<f32>() / n.max(1) as f32;
    for i in 0..n {
        g[i] /= mg;
        o[i] = (o[i] - mo) / mg;
    }
    (g, o)
}

/// Gaussian elimination with partial pivoting.
pub fn solve_dense(mut a: Vec<f64>, mut b: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| a[i * n + c].abs().total_cmp(&a[j * n + c].abs()))?;
        if a[p * n + c].abs() < 1e-14 {
            return None;
        }
        if p != c {
            for k in 0..n {
                a.swap(p * n + k, c * n + k);
            }
            b.swap(p, c);
        }
        for r in c + 1..n {
            let f = a[r * n + c] / a[c * n + c];
            if f == 0.0 {
                continue;
            }
            for k in c..n {
                a[r * n + k] -= f * a[c * n + k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| a[r * n + k] * x[k]).sum();
        x[r] = (b[r] - s) / a[r * n + r];
    }
    Some(x)
}

/// Applies the correction: gain/offset (automatic, then overrides), then background leveling.
/// Returns corrected tiles and the result record.
pub fn correct(pl: &Placement, tiles: &[Gray], p: &PhotometricParams) -> (Vec<Gray>, PhotometricResult) {
    let n = tiles.len();
    let samples = overlap_samples(pl, tiles, 3);
    let (ag, ao) = match p.model {
        GainModel::Affine => fit_affine(n, &samples, p.regularisation),
        GainModel::Gain => {
            let imgs: Vec<RoiImage> = pl
                .tiles
                .iter()
                .zip(tiles)
                .map(|(t, g)| RoiImage { x0: t.x as i32, y0: t.y as i32, w: g.w, h: g.h, ch: 1, px: g.px.clone(), alpha: vec![1.0; g.w * g.h] })
                .collect();
            let ph = photometric(&imgs, None, true, false);
            ((0..n).map(|i| 1.0 / ph.gains[i][0]).collect(), vec![0.0; n])
        }
        GainModel::None => (vec![1.0; n], vec![0.0; n]),
    };
    let mut g = ag.clone();
    let mut o = ao.clone();
    for ov in &p.overrides {
        if ov.index < n {
            if let Some(v) = ov.gain {
                g[ov.index] = v;
            }
            if let Some(v) = ov.offset {
                o[ov.index] = v;
            }
        }
    }
    let before = rms(&samples, &vec![1.0; n], &vec![0.0; n]);
    let mut out: Vec<Gray> = tiles
        .iter()
        .enumerate()
        .map(|(i, t)| Gray { w: t.w, h: t.h, px: t.px.iter().map(|&v| v * g[i] + o[i]).collect() })
        .collect();
    let mut res = PhotometricResult { auto_gain: ag, auto_offset: ao, gain: g, offset: o, ..Default::default() };
    if p.background {
        let skip: Vec<bool> = (0..n).map(|i| p.overrides.iter().any(|ov| ov.index == i && ov.no_background)).collect();
        let (median, ranges) = background::level(&mut out, &skip, &p.background_params);
        res.background_median = median;
        res.background_range = ranges;
    }
    let after_samples = overlap_samples(pl, &out, 3);
    res.overlap_rms_before = before;
    res.overlap_rms_after = rms(&after_samples, &vec![1.0; n], &vec![0.0; n]);
    (out, res)
}
