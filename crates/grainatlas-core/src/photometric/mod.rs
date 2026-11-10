//! Photometric correction (plan section 6.3): per-tile gain and offset from
//! the overlaps, per-tile background leveling, global tone (percentile stretch,
//! black / white / gamma, CLAHE) and HDR merges. Manual per-tile overrides are
//! parameters, never baked pixels.

pub mod background;
pub mod enhance;
pub mod grain_norm;
pub mod hdr;
pub mod joint;
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
    /// Why the joint background correction was replaced by gain and offset only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// Spread of owned-region background medians before choosing the correction (0..1 units).
    #[serde(default)]
    pub level_spread_joint: f32,
    #[serde(default)]
    pub level_spread_affine: f32,
}

/// Compare the same raw-background population in both results. A tile owns the pixels nearest
/// its centre among all tiles covering them (ties go to the lower index). Sample at most about
/// 64 x 64 pixels per tile; this is a panel-level guard, not a second full-image analysis.
/// Returns (global spread, largest overlapping-neighbour difference) for joint and affine.
fn background_level_comparison(pl: &Placement, raw: &[Gray], joint: &[Gray], stats: &[joint::TileStats], gain: &[f32], offset: &[f32]) -> [(f32, f32); 2] {
    let n = raw.len();
    let mut neighbours = vec![Vec::new(); n];
    for a in 0..n {
        for b in a + 1..n {
            let (ta, tb) = (&pl.tiles[a], &pl.tiles[b]);
            if ta.x.max(tb.x) < (ta.x + ta.w as i64).min(tb.x + tb.w as i64)
                && ta.y.max(tb.y) < (ta.y + ta.h as i64).min(tb.y + tb.h as i64)
            {
                neighbours[a].push(b);
                neighbours[b].push(a);
            }
        }
    }
    let levels: Vec<Option<[f32; 2]>> = crate::util::par_map(n, |i| {
        let (t, p, st) = (&raw[i], &pl.tiles[i], &stats[i]);
        let stride = t.w.max(t.h).div_ceil(64).max(4);
        let (mut jv, mut av, mut region) = (Vec::new(), Vec::new(), 0usize);
        for y in (0..t.h).step_by(stride) {
            for x in (0..t.w).step_by(stride) {
                // Doubled pixel/centre coordinates keep ownership and ties exact, on wasm too.
                let wx = 2 * (p.x as i128 + x as i128) + 1;
                let wy = 2 * (p.y as i128 + y as i128) + 1;
                let distance = |k: usize| {
                    let q = &pl.tiles[k];
                    let dx = wx - (2 * q.x as i128 + q.w as i128);
                    let dy = wy - (2 * q.y as i128 + q.h as i128);
                    dx * dx + dy * dy
                };
                let own = distance(i);
                if neighbours[i].iter().any(|&k| {
                    let q = &pl.tiles[k];
                    let covered = wx >= 2 * q.x as i128 && wx < 2 * (q.x as i128 + q.w as i128)
                        && wy >= 2 * q.y as i128 && wy < 2 * (q.y as i128 + q.h as i128);
                    covered && (distance(k), k) < (own, i)
                }) {
                    continue;
                }
                region += 1;
                let k = y * t.w + x;
                if (t.px[k] - st.mode).abs() <= st.band {
                    jv.push(joint[i].px[k]);
                    av.push(t.px[k] * gain[i] + offset[i]);
                }
            }
        }
        if jv.is_empty() || jv.len() * 100 < region * 3 {
            return None;
        }
        let k = jv.len() / 2;
        jv.select_nth_unstable_by(k, f32::total_cmp);
        av.select_nth_unstable_by(k, f32::total_cmp);
        Some([jv[k], av[k]])
    });
    std::array::from_fn(|r| {
        let (mut lo, mut hi, mut step) = (f32::INFINITY, f32::NEG_INFINITY, 0.0f32);
        for (i, level) in levels.iter().enumerate() {
            let Some(v) = level else { continue };
            lo = lo.min(v[r]);
            hi = hi.max(v[r]);
            for &k in &neighbours[i] {
                if let Some(w) = levels[k] {
                    step = step.max((v[r] - w[r]).abs());
                }
            }
        }
        (if lo <= hi { hi - lo } else { 0.0 }, step)
    })
}

fn joint_is_worse(joint: (f32, f32), affine: (f32, f32)) -> bool {
    joint.1 > affine.1 + 0.5 / 255.0 && joint.0 >= affine.0
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

/// Overlap disagreement of placed tiles, two ways (0..1 units): the pixel RMS the app reports, and
/// the RMS of the difference of `block` x `block` px means, which leaves out noise and small
/// misregistration and measures what shows as tile blocks and exposure steps.
pub fn overlap_metrics(pl: &Placement, tiles: &[Gray], block: usize) -> (f32, f32) {
    let px = rms(&overlap_samples(pl, tiles, 3), &vec![1.0; tiles.len()], &vec![0.0; tiles.len()]);
    let b = block.max(4) as i64;
    let (mut s, mut n) = (0.0f64, 0usize);
    for p in &pl.pairs {
        let (ta, tb) = (&pl.tiles[p.a], &pl.tiles[p.b]);
        let x0 = ta.x.max(tb.x) + 4;
        let y0 = ta.y.max(tb.y) + 4;
        let x1 = (ta.x + ta.w as i64).min(tb.x + tb.w as i64) - 4;
        let y1 = (ta.y + ta.h as i64).min(tb.y + tb.h as i64) - 4;
        let mut by = y0;
        while by + b <= y1 {
            let mut bx = x0;
            while bx + b <= x1 {
                let (mut sa, mut sb, mut k) = (0.0f64, 0.0f64, 0usize);
                for y in by..by + b {
                    for x in bx..bx + b {
                        let va = tiles[p.a].px[(y - ta.y) as usize * ta.w + (x - ta.x) as usize];
                        let vb = tiles[p.b].px[(y - tb.y) as usize * tb.w + (x - tb.x) as usize];
                        if (0.002..0.998).contains(&va) && (0.002..0.998).contains(&vb) {
                            sa += va as f64;
                            sb += vb as f64;
                            k += 1;
                        }
                    }
                }
                if k * 2 >= (b * b) as usize {
                    let d = (sa - sb) / k as f64;
                    s += d * d;
                    n += 1;
                }
                bx += b;
            }
            by += b;
        }
    }
    (px, if n == 0 { 0.0 } else { (s / n as f64).sqrt() as f32 })
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
    if p.background && p.background_params.method == background::LevelMethod::Joint {
        let skip: Vec<bool> = (0..n).map(|i| p.overrides.iter().any(|ov| ov.index == i && ov.no_background)).collect();
        let fixed_gain = match p.model {
            GainModel::Affine => None,
            _ => Some(ag.as_slice()),
        };
        let fixed = joint::Fixed { gain: fixed_gain, skip: &skip, regularisation: p.regularisation };
        let overrides: Vec<(usize, Option<f32>, Option<f32>)> = p.overrides.iter().filter(|ov| ov.index < n).map(|ov| (ov.index, ov.gain, ov.offset)).collect();
        let (out, sol, auto_gain, auto_offset) = joint::correct(pl, tiles, &p.background_params, &fixed, &overrides);
        // Compare the background-disabled path with the same model and overrides, reusing
        // fitted gains/offsets and raw statistics. Compute affine pixels only on the stride;
        // materialise the full affine result only if the guard selects it.
        let [(level_spread_joint, joint_step), (level_spread_affine, affine_step)] = background_level_comparison(pl, tiles, &out, &sol.raw_stats, &g, &o);
        let mut res = PhotometricResult {
            auto_gain,
            auto_offset,
            gain: sol.gain.clone(),
            offset: sol.offset.clone(),
            background_median: sol.level,
            background_range: sol.range.clone(),
            overlap_rms_before: before,
            overlap_rms_after: 0.0,
            fallback: None,
            level_spread_joint,
            level_spread_affine,
        };
        // A single neighbour can worsen while the area becomes flatter overall. Reject
        // joint only when its largest step worsens meaningfully and its spread does not improve.
        let out = if joint_is_worse((level_spread_joint, joint_step), (level_spread_affine, affine_step)) {
            res.fallback = Some(format!("background leveling left larger tile steps than none ({:.1} vs {:.1} grey); used gain and offset only", joint_step * 255.0, affine_step * 255.0));
            let affine = crate::util::par_map(n, |i| Gray {
                w: tiles[i].w, h: tiles[i].h, px: tiles[i].px.iter().map(|&v| v * g[i] + o[i]).collect(),
            });
            res.auto_gain = ag;
            res.auto_offset = ao;
            res.gain = g;
            res.offset = o;
            res.background_median = 0.0;
            res.background_range.clear();
            affine
        } else {
            out
        };
        let after = overlap_samples(pl, &out, 3);
        res.overlap_rms_after = rms(&after, &vec![1.0; n], &vec![0.0; n]);
        return (out, res);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mosaic::TilePlacement;

    fn placement(rows: usize, cols: usize, w: usize, h: usize) -> Placement {
        let mut tiles = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                let (x, y) = ((c * w * 3 / 4) as i64, (r * h * 3 / 4) as i64);
                let pos = [x as f64, y as f64];
                tiles.push(TilePlacement { index: tiles.len(), prior: pos, pos, x, y, w, h, drift: [0.0; 2] });
            }
        }
        Placement { tiles, pairs: Vec::new(), width: (cols - 1) * w * 3 / 4 + w, height: (rows - 1) * h * 3 / 4 + h, rms_residual: 0.0, max_residual: 0.0 }
    }

    #[test]
    fn level_guard_uses_raw_background_in_owned_regions_and_geometric_neighbours() {
        let pl = placement(1, 3, 64, 64);
        let mut raw = vec![Gray { w: 64, h: 64, px: vec![0.41; 64 * 64] }; 3];
        let mut out = raw.clone();
        // The last tile has under 3 % raw epoxy in its own region, so its extreme output is
        // excluded. The other two have grains in over half their region; exclude those using
        // the raw mask even though their corrected values are inside the epoxy band.
        for i in 0..3 {
            for y in 0..64 {
                for x in 0..64 {
                    let k = y * 64 + x;
                    if (i == 2 && y >= 1) || (i < 2 && y >= 24) {
                        raw[i].px[k] = 0.8;
                        out[i].px[k] = 0.41;
                    } else {
                        out[i].px[k] = [0.50, 0.52, 0.9][i];
                    }
                    // Unowned overlap pixels must not affect the medians.
                    if (i == 0 && x >= 56) || (i == 1 && !(8..56).contains(&x)) {
                        out[i].px[k] = 0.1;
                    }
                }
            }
        }
        let stats = vec![joint::TileStats { mode: 0.41, band: 0.04 }; 3];
        // One sampled background pixel is below 3 % of tile 2's owned region.
        raw[2].px.fill(0.8);
        raw[2].px[16] = 0.41;
        let [j, a] = background_level_comparison(&pl, &raw, &out, &stats, &[1.0; 3], &[0.0; 3]);
        assert!((j.0 - 0.02).abs() < 1e-6, "{j:?}");
        assert!((j.1 - 0.02).abs() < 1e-6, "{j:?}");
        assert_eq!(a, (0.0, 0.0));
        // Tile 1's corrected epoxy is deliberately outside the raw band's bounds.
        assert!(out[1].px[16] > stats[1].mode + stats[1].band);
        // No registration pair is needed: ownership and adjacency come from tile rectangles.
        assert!(pl.pairs.is_empty());
    }

    #[test]
    fn level_guard_comparison_small_area_finishes_within_generous_bound() {
        let pl = placement(2, 3, 128, 96);
        let raw = vec![Gray { w: 128, h: 96, px: vec![0.41; 128 * 96] }; 6];
        let out = raw.clone();
        let stats = vec![joint::TileStats { mode: 0.41, band: 0.04 }; 6];
        let start = std::time::Instant::now();
        let metrics = background_level_comparison(&pl, &raw, &out, &stats, &[1.0; 6], &[0.0; 6]);
        assert_eq!(metrics, [(0.0, 0.0); 2]);
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
    }

    #[test]
    fn level_guard_keeps_joint_when_area_spread_improves_despite_a_larger_step() {
        let pl = placement(1, 4, 64, 64);
        let raw: Vec<Gray> = [0.30, 0.34, 0.38, 0.42].iter().map(|&v| Gray { w: 64, h: 64, px: vec![v; 64 * 64] }).collect();
        let out: Vec<Gray> = [0.32, 0.32, 0.39, 0.39].iter().map(|&v| Gray { w: 64, h: 64, px: vec![v; 64 * 64] }).collect();
        let stats: Vec<_> = raw.iter().map(|t| joint::TileStats { mode: t.px[0], band: 0.04 }).collect();
        let [j, a] = background_level_comparison(&pl, &raw, &out, &stats, &[1.0; 4], &[0.0; 4]);
        assert!(j.1 > a.1 + 0.5 / 255.0 && j.0 < a.0, "{j:?} {a:?}");
        assert!(!joint_is_worse(j, a));
        assert!(joint_is_worse((a.0, j.1), a), "equal spread still falls back");
        assert!(!joint_is_worse((a.0 + 0.1, a.1 + 0.4 / 255.0), a), "sub-threshold step keeps joint");
    }
}
