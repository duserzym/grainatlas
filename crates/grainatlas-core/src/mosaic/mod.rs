//! Stitching by translation (plan section 6.2): stage prior, phase correlation
//! per neighbour pair with a sub-pixel peak and a peak-to-sidelobe gate, and a
//! global least-squares solve with one round of outlier rejection.
//!
//! Design after Preibisch, Saalfeld and Tomancak (2009); phase correlation after
//! Kuglin and Hines (1975), sub-pixel estimator after Foroosh, Zerubia and
//! Berthod (2002).

pub mod phasecorr;
pub mod pyramid;
pub mod solve;

use serde::{Deserialize, Serialize};

use crate::Ctl;
use crate::image::Gray;
use crate::readers::AreaMeta;

/// Parameters of the stitch (stored in `project.json`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StitchParams {
    /// Downsampling factor for the coarse correlation (2 = half resolution).
    pub coarse_factor: usize,
    /// Minimum peak-to-sidelobe ratio to trust a measured pair.
    pub min_psr: f64,
    /// Largest accepted deviation of a pair from the stage prior, in micrometres.
    pub max_drift_um: f64,
    /// Residual above which a pair is dropped and the solve repeated (pixels).
    pub outlier_px: f64,
    /// Refine at full resolution after the coarse estimate.
    pub refine_full: bool,
}

impl Default for StitchParams {
    fn default() -> Self {
        Self { coarse_factor: 2, min_psr: 7.0, max_drift_um: 60.0, outlier_px: 3.0, refine_full: true }
    }
}

/// One neighbour pair and what the correlation found.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PairResult {
    pub a: usize,
    pub b: usize,
    /// "h" (left-right) or "v" (up-down).
    pub dir: String,
    /// Offset of `b` relative to `a` from the stage prior (mosaic px).
    pub prior: [f64; 2],
    /// Measured offset, if the correlation passed the gate.
    pub measured: Option<[f64; 2]>,
    pub psr: f64,
    /// Used in the final solve.
    pub used: bool,
    /// |solved - measured| after the solve (px); 0 for unused pairs.
    pub residual: f64,
}

/// Solved position of one tile.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TilePlacement {
    pub index: usize,
    /// Prior top-left position in mosaic pixels.
    pub prior: [f64; 2],
    /// Solved sub-pixel top-left position in mosaic pixels.
    pub pos: [f64; 2],
    /// Integer placement used for blending.
    pub x: i64,
    pub y: i64,
    pub w: usize,
    pub h: usize,
    /// Solved minus prior (px).
    pub drift: [f64; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Placement {
    pub tiles: Vec<TilePlacement>,
    pub pairs: Vec<PairResult>,
    pub width: usize,
    pub height: usize,
    pub rms_residual: f64,
    pub max_residual: f64,
}

/// Prior placement from positions (um) and pixel size, in mosaic pixels.
pub fn prior_px(area: &AreaMeta) -> Vec<[f64; 2]> {
    let k = 1000.0 / area.pixel_size_nm.max(1e-9);
    area.tiles.iter().map(|t| [t.pos_um[0] * k, t.pos_um[1] * k]).collect()
}

/// Neighbour pairs from prior rectangles: left-right and up-down overlaps only.
pub fn neighbour_pairs(prior: &[[f64; 2]], sizes: &[(usize, usize)]) -> Vec<(usize, usize, &'static str)> {
    let mut out = Vec::new();
    for a in 0..prior.len() {
        for b in a + 1..prior.len() {
            let (wa, ha) = (sizes[a].0 as f64, sizes[a].1 as f64);
            let (wb, hb) = (sizes[b].0 as f64, sizes[b].1 as f64);
            let ox = (prior[a][0] + wa).min(prior[b][0] + wb) - prior[a][0].max(prior[b][0]);
            let oy = (prior[a][1] + ha).min(prior[b][1] + hb) - prior[a][1].max(prior[b][1]);
            if ox <= 0.0 || oy <= 0.0 {
                continue;
            }
            let fx = ox / wa.min(wb);
            let fy = oy / ha.min(hb);
            if fy > 0.6 && fx < 0.6 && fx > 0.02 {
                out.push((a, b, "h"));
            } else if fx > 0.6 && fy < 0.6 && fy > 0.02 {
                out.push((a, b, "v"));
            }
        }
    }
    out
}

/// Full stitch of one area's tiles.
pub fn stitch(area: &AreaMeta, tiles: &[Gray], p: &StitchParams, ctl: &Ctl) -> crate::Result<Placement> {
    let prior = prior_px(area);
    let sizes: Vec<(usize, usize)> = tiles.iter().map(|t| (t.w, t.h)).collect();
    let pairs = neighbour_pairs(&prior, &sizes);
    let max_drift_px = p.max_drift_um * 1000.0 / area.pixel_size_nm.max(1e-9);
    let n = pairs.len().max(1);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<PairResult> = crate::util::par_map(pairs.len(), |k| {
        let (a, b, dir) = pairs[k];
        let pd = [prior[b][0] - prior[a][0], prior[b][1] - prior[a][1]];
        let m = if ctl.check().is_ok() { phasecorr::measure_pair(&tiles[a], &tiles[b], pd, p) } else { None };
        let (measured, psr) = match m {
            Some((d, psr)) if psr >= p.min_psr && (d[0] - pd[0]).hypot(d[1] - pd[1]) <= max_drift_px => (Some(d), psr),
            Some((_, psr)) => (None, psr),
            None => (None, 0.0),
        };
        let c = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        ctl.report("correlate", c as f32 / n as f32);
        PairResult { a, b, dir: dir.to_string(), prior: pd, measured, psr, used: measured.is_some(), residual: 0.0 }
    });
    ctl.check()?;
    let (pos, results) = solve::solve(&prior, results, p.outlier_px);
    let mut tp = Vec::new();
    let (mut minx, mut miny) = (f64::INFINITY, f64::INFINITY);
    for q in &pos {
        minx = minx.min(q[0]);
        miny = miny.min(q[1]);
    }
    let (mut w, mut h) = (0i64, 0i64);
    for (i, q) in pos.iter().enumerate() {
        let sx = q[0] - minx;
        let sy = q[1] - miny;
        let x = sx.round() as i64;
        let y = sy.round() as i64;
        w = w.max(x + sizes[i].0 as i64);
        h = h.max(y + sizes[i].1 as i64);
        tp.push(TilePlacement {
            index: i,
            prior: prior[i],
            pos: [sx, sy],
            x,
            y,
            w: sizes[i].0,
            h: sizes[i].1,
            drift: [q[0] - prior[i][0] - (minx - prior.iter().map(|v| v[0]).fold(f64::INFINITY, f64::min)), q[1] - prior[i][1] - (miny - prior.iter().map(|v| v[1]).fold(f64::INFINITY, f64::min))],
        });
    }
    let used: Vec<f64> = results.iter().filter(|r| r.used).map(|r| r.residual).collect();
    let rms = if used.is_empty() { 0.0 } else { (used.iter().map(|r| r * r).sum::<f64>() / used.len() as f64).sqrt() };
    let max = used.iter().cloned().fold(0.0, f64::max);
    Ok(Placement { tiles: tp, pairs: results, width: w.max(0) as usize, height: h.max(0) as usize, rms_residual: rms, max_residual: max })
}
