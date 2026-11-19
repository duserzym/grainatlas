//! Joint background correction: one least-squares solve for every tile's gain, offset, smooth
//! background surface and beam-exposure steps, from two kinds of evidence at once:
//!
//! - **overlaps agree**: in every overlap of two placed tiles, block means of the corrected
//!   tiles are equal;
//! - **background is flat**: the epoxy background of every tile, corrected, sits at one level.
//!
//! Per-tile leveling (`background::level`) fits each tile on its own, after the gain and offset:
//! nothing then keeps neighbours in agreement, and its one-sided grain mask (grains brighter than
//! the background) takes the background of CL-bright tiles for grain and skips them, so whole
//! tiles and halves of mosaics jump. Here each tile's correction is
//!
//! `g·v + o + w(v)·(Σ_k s_k·[inside the overlap with tile k] + P(x, y))`
//!
//! with `P` a 2nd-order polynomial without constant term (charging gradients) and one step `s_k`
//! per overlapping neighbour `k` (the part of a tile the beam had already scanned while `k` was
//! captured is darker, with sharp edges at the neighbour's outline; corners scanned twice add two
//! steps). Background is found by level and flatness on both sides of the tile's mode, so
//! CL-dark and CL-bright grains alike are left out. Grain contrast is kept: the correction is a
//! gain per tile and a smooth or piecewise-constant additive field. The field fades below the
//! epoxy (full at mode - 1.5 bands), with a wider ramp for strong fields to keep the mapping
//! monotonic and limit contrast changes, so it is not extrapolated off mount.

use crate::image::Gray;
use crate::mosaic::Placement;

use super::background::BackgroundParams;

/// The joint solution per tile.
#[derive(Clone, Debug, Default)]
pub struct JointSolution {
    pub gain: Vec<f32>,
    pub offset: Vec<f32>,
    /// Background level everything is brought to.
    pub level: f32,
    /// Range (max - min) of each tile's additive field (surface and steps).
    pub range: Vec<f32>,
    fields: Vec<Field>,
    pub(super) raw_stats: Vec<TileStats>,
}

#[derive(Clone, Debug, Default)]
struct Field {
    poly: [f64; 5],
    /// (rectangle in tile pixels x0, y0, x1, y1; step height).
    steps: Vec<([i64; 4], f64)>,
    /// Edge widths of the tile's step lines (`Cuts`).
    widths: Cuts,
}

/// A tile's step lines: (position, edge width in px) along x and along y.
type Cuts = (Vec<(i64, f64)>, Vec<(i64, f64)>);

/// Width of an edge at `at` among `cuts` (fitted where the edge was found in the data, else a
/// typical TM4000 edge).
fn edge_width(cuts: &[(i64, f64)], at: i64) -> f64 {
    cuts.iter().find(|c| c.0 == at).map_or(DEFAULT_EDGE, |c| c.1)
}
const DEFAULT_EDGE: f64 = 8.0;

/// What the solve may change per tile.
pub struct Fixed<'a> {
    /// Gains to keep (gain model `gain` or `none`); `None` = solve them.
    pub gain: Option<&'a [f32]>,
    /// Tiles without background correction (offset only).
    pub skip: &'a [bool],
    pub regularisation: f64,
}

/// Background statistics of one tile.
#[derive(Clone, Debug)]
pub(super) struct TileStats {
    /// Background level (mode) and pixel noise.
    pub(super) mode: f32,
    pub(super) band: f32,
}

impl TileStats {
    /// Keep the full additive field on epoxy and brighter grains, and fade it to zero below
    /// the epoxy. Gain and offset still apply everywhere. Near-black CL epoxy has a full-field
    /// threshold at or below zero, so all nonnegative pixels keep the original correction.
    fn field_weight(&self, v: f32, field: f32) -> f32 {
        let full = self.mode - 1.5 * self.band;
        // For a fixed spatial field F, d(v + w(v)*F)/dv is 1 outside the ramp and
        // 1 + F/ramp inside it: widening downwards keeps that slope in [0.5, 1.5].
        let ramp = (1.5 * self.band).max(2.0 * field.abs());
        let zero = full - ramp;
        if v >= full { 1.0 } else if v <= zero { 0.0 } else { (v - zero) / (full - zero) }
    }
}

fn valid(v: f32) -> bool {
    (0.002..0.998).contains(&v)
}

/// Smoothed 256-bin histogram of a tile's valid pixels.
fn smoothed_hist(t: &Gray) -> Vec<f64> {
    let mut hist = [0f64; 256];
    for &v in t.px.iter().step_by(3) {
        if valid(v) {
            hist[(v * 255.0) as usize] += 1.0;
        }
    }
    (0..256usize)
        .map(|i| {
            (i.saturating_sub(3)..(i + 4).min(256))
                .map(|j| hist[j])
                .sum()
        })
        .collect()
}

/// Bins of the main peaks of a smoothed histogram: local maxima with at least `MIN_PEAK` of the
/// highest one, highest first. Plateaus count once, at their centre; a smaller peak must be
/// separated from every retained higher peak by a drop of at least 20 % of its own height.
fn peaks(sm: &[f64]) -> Vec<usize> {
    const MIN_PEAK: f64 = 0.15;
    const MIN_DROP: f64 = 0.20;
    let top = sm.iter().copied().fold(0.0, f64::max);
    let mut candidates = Vec::new();
    let mut i = 0;
    while i < sm.len() {
        let mut end = i + 1;
        while end < sm.len() && sm[end] == sm[i] {
            end += 1;
        }
        if sm[i] > 0.0 && sm[i] >= MIN_PEAK * top
            && (i == 0 || sm[i] > sm[i - 1])
            && (end == sm.len() || sm[i] > sm[end])
        {
            candidates.push((i + end - 1) / 2);
        }
        i = end;
    }
    candidates.sort_by(|&a, &b| sm[b].total_cmp(&sm[a]).then(a.cmp(&b)));
    let mut out: Vec<usize> = Vec::new();
    for p in candidates {
        if out.iter().all(|&q| {
            let valley = sm[p.min(q)..=p.max(q)].iter().copied().fold(f64::INFINITY, f64::min);
            valley <= (1.0 - MIN_DROP) * sm[p]
        }) {
            out.push(p);
        }
    }
    out
}

fn bin_value(i: usize) -> f32 {
    (i as f32 + 0.5) / 255.0
}

/// One tile's statistics with the background at its largest histogram peak (a single image,
/// e.g. a blended mosaic, where the epoxy is the largest population).
fn tile_stats(t: &Gray) -> TileStats {
    let sm = smoothed_hist(t);
    let mi = (0..256)
        .filter(|&i| sm[i] > 0.0)
        .max_by(|&a, &b| sm[a].total_cmp(&sm[b]))
        .unwrap_or(128);
    tile_stats_at(t, bin_value(mi))
}

/// Statistics of every tile of a mosaic, with each tile's background at the peak of its
/// histogram near the background its overlap neighbours agree on. Each neighbour votes with
/// its own largest peak; even counts use the mean of the two middle votes. Only switch a
/// distant main peak (> 0.06) to a significant peak within 0.04 of that local consensus, so
/// noise and smooth background drift do not change the population. Isolated tiles keep their
/// main peak. Votes always use the original peaks, independent of tile traversal order.
/// A tile mostly off the mount (BSE: dark beyond the mount edge) or mostly grain (CL-bright
/// grains) has its largest peak elsewhere; taking that for background would pull it to the
/// level of the others and print the whole tile as a bright or dark panel.
fn tiles_stats(tiles: &[impl std::borrow::Borrow<Gray>], neighbours: &[Vec<usize>]) -> Vec<TileStats> {
    let tiles: Vec<&Gray> = tiles.iter().map(|g| g.borrow()).collect();
    let hists: Vec<Vec<f64>> = crate::util::par_map(tiles.len(), |i| smoothed_hist(tiles[i]));
    let pks: Vec<Vec<usize>> = hists.iter().map(|h| peaks(h)).collect();
    crate::util::par_map(tiles.len(), |i| {
        let mut mi = pks[i].first().copied().unwrap_or(128);
        let mut votes: Vec<usize> = neighbours[i].iter().filter_map(|&j| pks[j].first().copied()).collect();
        if !votes.is_empty() {
            votes.sort_unstable();
            let k = votes.len() / 2;
            let consensus = (bin_value(votes[(votes.len() - 1) / 2]) + bin_value(votes[k])) * 0.5;
            if (bin_value(mi) - consensus).abs() > 0.06
                && let Some(&p) = pks[i].iter().min_by(|&&a, &&b| {
                    (bin_value(a) - consensus).abs().total_cmp(&(bin_value(b) - consensus).abs())
                })
                && (bin_value(p) - consensus).abs() <= 0.04
            {
                mi = p;
            }
        }
        tile_stats_at(tiles[i], bin_value(mi))
    })
}

fn tile_stats_at(t: &Gray, mode: f32) -> TileStats {
    // Pixel noise from neighbour differences (independent of gradients and steps).
    let mut d: Vec<f32> = Vec::with_capacity(t.px.len() / 9);
    for y in (0..t.h).step_by(3) {
        let row = &t.px[y * t.w..(y + 1) * t.w];
        for x in (0..t.w.saturating_sub(1)).step_by(3) {
            if (row[x] - mode).abs() < 0.1 && (row[x + 1] - mode).abs() < 0.1 {
                d.push((row[x + 1] - row[x]).abs());
            }
        }
    }
    let sigma = if d.is_empty() {
        0.02
    } else {
        let k = d.len() / 2;
        d.select_nth_unstable_by(k, f32::total_cmp);
        1.4826 * d[k] / std::f32::consts::SQRT_2
    };
    TileStats { mode, band: (4.0 * sigma).max(0.04) }
}

/// Background non-uniformity of a blended mosaic (0..1 units): the RMS spread of the means of
/// its `block` px background blocks (by level, as the correction finds them) about their
/// median, the most extreme 1 % left out. Seams, exposure blocks and gradients raise it; grain
/// contrast does not enter.
pub fn background_flatness(g: &Gray, coverage: &[f32], block: usize) -> f32 {
    let st = tile_stats(g);
    let b = block.max(4);
    let mut means = Vec::new();
    for by in (0..g.h.saturating_sub(b - 1)).step_by(b) {
        for bx in (0..g.w.saturating_sub(b - 1)).step_by(b) {
            let (mut s, mut k) = (0.0f64, 0usize);
            for y in by..by + b {
                for x in bx..bx + b {
                    let i = y * g.w + x;
                    let v = g.px[i];
                    if coverage[i] >= 0.5 && valid(v) && (v - st.mode).abs() < st.band {
                        s += v as f64;
                        k += 1;
                    }
                }
            }
            if k * 100 >= b * b * 85 {
                means.push(s / k as f64);
            }
        }
    }
    if means.len() < 4 {
        return 0.0;
    }
    means.sort_by(f64::total_cmp);
    let med = means[means.len() / 2];
    let mut d: Vec<f64> = means.iter().map(|m| (m - med) * (m - med)).collect();
    d.sort_by(f64::total_cmp);
    d.truncate(d.len() * 99 / 100);
    (d.iter().sum::<f64>() / d.len().max(1) as f64).sqrt() as f32
}

/// One observation row: sparse coefficients, right-hand side, weight.
struct Row {
    c: Vec<(usize, f64)>,
    rhs: f64,
    w: f64,
    kind: u8,
}

/// Unknown layout of one tile: gain (if solved), offset, 5 polynomial terms, one height per step
/// rectangle (overlaps with neighbours, snapped to the step lines found in the tile, and the
/// half-planes of step lines no overlap explains).
struct Layout {
    base: Vec<usize>,
    gain_free: bool,
    /// Neighbours of each tile: (other tile, overlap rectangle in this tile's pixels).
    nb: Vec<Vec<(usize, [i64; 4])>>,
    steps: Vec<Vec<[i64; 4]>>,
    n: usize,
}

impl Layout {
    fn g(&self, i: usize) -> Option<usize> {
        self.gain_free.then_some(self.base[i])
    }
    fn o(&self, i: usize) -> usize {
        self.base[i] + self.gain_free as usize
    }
    fn p(&self, i: usize, k: usize) -> usize {
        self.o(i) + 1 + k
    }
    fn s(&self, i: usize, k: usize) -> usize {
        self.o(i) + 6 + k
    }
}

fn poly_terms(fx: f64, fy: f64) -> [f64; 5] {
    [fx, fy, fx * fx, fx * fy, fy * fy]
}

fn frac(block: [i64; 4], r: [i64; 4]) -> f64 {
    let w = (block[2].min(r[2]) - block[0].max(r[0])).max(0);
    let h = (block[3].min(r[3]) - block[1].max(r[1])).max(0);
    (w * h) as f64 / ((block[2] - block[0]) * (block[3] - block[1])).max(1) as f64
}

/// The whole correction: each tile's background flattened on its own over the cells its step
/// lines and overlap edges cut it into (exposure windows, wherever they fall), then the joint
/// solve over all tiles on the flattened tiles. Returns the corrected tiles and the solution
/// (gains and offsets of the joint solve, ranges of both fields together).
/// `overrides` are manual (tile, gain, offset) that replace the solved ones; the automatic values
/// are returned as well. Returns (tiles, solution after overrides, automatic gains, offsets).
#[allow(clippy::type_complexity)]
pub fn correct(pl: &Placement, tiles: &[Gray], bp: &BackgroundParams, fixed: &Fixed, overrides: &[(usize, Option<f32>, Option<f32>)],
) -> (Vec<Gray>, JointSolution, Vec<f32>, Vec<f32>) {
    correct_ctl(pl, tiles, bp, fixed, overrides, &crate::Ctl::NONE).expect("uncancelled correction")
}

type Correction = (Vec<Gray>, JointSolution, Vec<f32>, Vec<f32>);

pub fn correct_ctl(
    pl: &Placement,
    tiles: &[impl std::borrow::Borrow<Gray>],
    bp: &BackgroundParams,
    fixed: &Fixed,
    overrides: &[(usize, Option<f32>, Option<f32>)],
    ctl: &crate::Ctl,
) -> crate::Result<Correction> {
    ctl.check()?;
    let tiles: Vec<&Gray> = tiles.iter().map(|g| g.borrow()).collect();
    let n = tiles.len();
    let block = (bp.cell / 2).max(4) as i64;

    // Neighbours: every pair of placed tiles that overlap by at least a block (and margins) each way.
    let rect = |i: usize| {
        let t = &pl.tiles[i];
        [t.x, t.y, t.x + t.w as i64, t.y + t.h as i64]
    };
    let mut nb: Vec<Vec<(usize, [i64; 4])>> = vec![Vec::new(); n];
    let mut neighbours = vec![Vec::new(); n];
    for a in 0..n {
        for b in a + 1..n {
            let (ra, rb) = (rect(a), rect(b));
            let r = [ra[0].max(rb[0]), ra[1].max(rb[1]), ra[2].min(rb[2]), ra[3].min(rb[3]),
            ];
            if r[2] > r[0] && r[3] > r[1] {
                neighbours[a].push(b);
                neighbours[b].push(a);
            }
            if r[2] - r[0] >= block + 8 && r[3] - r[1] >= block + 8 {
                nb[a].push((b, [r[0] - ra[0], r[1] - ra[1], r[2] - ra[0], r[3] - ra[1]]));
                nb[b].push((a, [r[0] - rb[0], r[1] - rb[1], r[2] - rb[0], r[3] - rb[1]]));
            }
        }
    }
    let stats: Vec<TileStats> = tiles_stats(&tiles, &neighbours);
    // Step rectangles: the overlaps, with their inner edges moved onto the step lines found in
    // the tile's background, and the step lines that no overlap edge explains.
    let lines: Vec<Cuts> = crate::util::try_par_map(n, ctl, |i| {
        Ok(if fixed.skip[i] { (Vec::new(), Vec::new()) } else { step_lines(tiles[i], &stats[i]) })
    })?;
    if std::env::var_os("GRAINATLAS_DEBUG_STEPS").is_some() {
        for (i, l) in lines.iter().enumerate() {
            eprintln!("tile {i}: mode {:.4} band {:.4} x {:?} y {:?}", stats[i].mode, stats[i].band, l.0, l.1);
        }
    }
    let mut cuts: Vec<Cuts> = Vec::with_capacity(n);
    let steps: Vec<Vec<[i64; 4]>> = (0..n)
        .map(|i| {
            let (w, h) = (tiles[i].w as i64, tiles[i].h as i64);
            let (xs, ys) = lines[i].clone();
            // Every overlap edge near a step line moves onto it (several overlaps can share one
            // edge, e.g. the left and the diagonal neighbour) and takes its fitted width.
            let snap = |e: i64, n: i64, ls: &[(i64, f64)]| -> i64 {
                if e <= 0 || e >= n {
                    return e;
                }
                ls.iter().filter(|l| (l.0 - e).abs() <= SNAP).min_by_key(|l| (l.0 - e).abs()).map_or(e, |l| l.0)
            };
            let rects: Vec<[i64; 4]> = nb[i].iter().map(|(_, r)| {
                    [snap(r[0], w, &xs), snap(r[1], h, &ys), snap(r[2], w, &xs), snap(r[3], h, &ys),
                    ]
                }).collect();
            // The step lines stay cuts themselves (cell_cuts merges duplicates).
            (rects, xs, ys)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .enumerate()
        .map(|(i, (rects, wx, wy))| {
            cuts.push(cell_cuts(&rects, &wx, &wy, tiles[i].w as i64, tiles[i].h as i64,
            ));
            rects
        })
        .collect();
    // Stage 1: every tile flattened on its own.
    let flat: Vec<Option<(Gray, f32)>> = crate::util::try_par_map(n, ctl, |i| {
        Ok(if fixed.skip[i] { None } else { flatten_tile(tiles[i], &stats[i], &cuts[i], (block / 2).max(4)) })
    })?;
    let mut range1 = vec![0.0f32; n];
    let leveled: Vec<Gray> = flat
        .into_iter()
        .enumerate()
        .map(|(i, f)| match f {
            Some((g, r)) => {
                range1[i] = r;
                g
            }
            None => (*tiles[i]).clone(),
        })
        .collect();
    // Stage 2: gains, offsets, surfaces and overlap steps of all tiles together.
    let leveled_stats = tiles_stats(&leveled, &neighbours);
    let mut sol = solve_loaded(pl, &|i|Ok(std::borrow::Cow::Borrowed(&leveled[i])), block, fixed, nb, steps, &cuts, &leveled_stats, &stats)?;
    // Both application stages use the raw mode/band. Stage-two fitting still uses the
    // leveled background statistics, but its overlap mask and application must not shift
    // the fade thresholds after stage one.
    sol.raw_stats = stats;
    for (r, r1) in sol.range.iter_mut().zip(&range1) {
        *r += r1;
    }
    let (auto_gain, auto_offset) = (sol.gain.clone(), sol.offset.clone());
    for &(i, g, o) in overrides {
        if let Some(v) = g {
            sol.gain[i] = v;
        }
        if let Some(v) = o {
            sol.offset[i] = v;
        }
    }
    let out = leveled
        .into_iter()
        .enumerate()
        .map(|(i, tile)| {
            ctl.check()?;
            Ok(sol.apply(i, &tile))
        })
        .collect::<crate::Result<Vec<_>>>()?;
    ctl.check()?;
    Ok((out, sol, auto_gain, auto_offset))
}

/// A compact correction recipe. Full-resolution fields are regenerated for one
/// source tile at a time; no mosaic or corrected tile set is retained.
pub(crate) struct Prepared {
    pub sol: JointSolution,
    stats: Vec<TileStats>,
    cuts: Vec<Cuts>,
    skip: Vec<bool>,
    block: i64,
}
impl Prepared {
    pub fn apply(&self, i: usize, t: &Gray) -> Gray {
        let flat = if self.skip[i] {
            None
        } else {
            flatten_tile(t, &self.stats[i], &self.cuts[i], self.block)
        };
        self.sol.apply(i, flat.as_ref().map_or(t, |f| &f.0))
    }
}
pub(crate) fn prepare_loaded(
    pl: &Placement,
    load: &impl Fn(usize) -> crate::Result<Gray>,
    bp: &BackgroundParams,
    fixed: &Fixed,
    overrides: &[(usize, Option<f32>, Option<f32>)],
    ctl: &crate::Ctl,
) -> crate::Result<Prepared> {
    ctl.check()?;
    let n = pl.tiles.len();
    let block = (bp.cell / 2).max(4) as i64;

    // Neighbours: every pair of placed tiles that overlap by at least a block (and margins) each way.
    let rect = |i: usize| {
        let t = &pl.tiles[i];
        [t.x, t.y, t.x + t.w as i64, t.y + t.h as i64]
    };
    let mut nb: Vec<Vec<(usize, [i64; 4])>> = vec![Vec::new(); n];
    let mut neighbours = vec![Vec::new(); n];
    for a in 0..n {
        for b in a + 1..n {
            let (ra, rb) = (rect(a), rect(b));
            let r = [
                ra[0].max(rb[0]),
                ra[1].max(rb[1]),
                ra[2].min(rb[2]),
                ra[3].min(rb[3]),
            ];
            if r[2] > r[0] && r[3] > r[1] {
                neighbours[a].push(b);
                neighbours[b].push(a);
            }
            if r[2] - r[0] >= block + 8 && r[3] - r[1] >= block + 8 {
                nb[a].push((b, [r[0] - ra[0], r[1] - ra[1], r[2] - ra[0], r[3] - ra[1]]));
                nb[b].push((a, [r[0] - rb[0], r[1] - rb[1], r[2] - rb[0], r[3] - rb[1]]));
            }
        }
    }
    let stats = stats_loaded(n, load, &neighbours, ctl)?;
    // Step rectangles: the overlaps, with their inner edges moved onto the step lines found in
    // the tile's background, and the step lines that no overlap edge explains.
    let lines: Vec<Cuts> = (0..n)
        .map(|i| {
            ctl.check()?;
            Ok(if fixed.skip[i] {
                (Vec::new(), Vec::new())
            } else {
                step_lines(&load(i)?, &stats[i])
            })
        })
        .collect::<crate::Result<_>>()?;
    if std::env::var_os("GRAINATLAS_DEBUG_STEPS").is_some() {
        for (i, l) in lines.iter().enumerate() {
            eprintln!(
                "tile {i}: mode {:.4} band {:.4} x {:?} y {:?}",
                stats[i].mode, stats[i].band, l.0, l.1
            );
        }
    }
    let mut cuts: Vec<Cuts> = Vec::with_capacity(n);
    let steps: Vec<Vec<[i64; 4]>> = (0..n)
        .map(|i| {
            let (w, h) = (pl.tiles[i].w as i64, pl.tiles[i].h as i64);
            let (xs, ys) = lines[i].clone();
            // Every overlap edge near a step line moves onto it (several overlaps can share one
            // edge, e.g. the left and the diagonal neighbour) and takes its fitted width.
            let snap = |e: i64, n: i64, ls: &[(i64, f64)]| -> i64 {
                if e <= 0 || e >= n {
                    return e;
                }
                ls.iter()
                    .filter(|l| (l.0 - e).abs() <= SNAP)
                    .min_by_key(|l| (l.0 - e).abs())
                    .map_or(e, |l| l.0)
            };
            let rects: Vec<[i64; 4]> = nb[i]
                .iter()
                .map(|(_, r)| {
                    [
                        snap(r[0], w, &xs),
                        snap(r[1], h, &ys),
                        snap(r[2], w, &xs),
                        snap(r[3], h, &ys),
                    ]
                })
                .collect();
            // The step lines stay cuts themselves (cell_cuts merges duplicates).
            (rects, xs, ys)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .enumerate()
        .map(|(i, (rects, wx, wy))| {
            cuts.push(cell_cuts(
                &rects,
                &wx,
                &wy,
                pl.tiles[i].w as i64,
                pl.tiles[i].h as i64,
            ));
            rects
        })
        .collect();

    let block1 = (block / 2).max(4);
    let leveled = |i: usize| {
        ctl.check()?;
        let t = load(i)?;
        Ok(if fixed.skip[i] {
            t
        } else {
            flatten_tile(&t, &stats[i], &cuts[i], block1).map_or(t, |f| f.0)
        })
    };
    let leveled_stats = stats_loaded(n, &leveled, &neighbours, ctl)?;
    let mut sol = solve_loaded(
        pl,
        &|i|Ok(std::borrow::Cow::Owned(leveled(i)?)),
        block,
        fixed,
        nb,
        steps,
        &cuts,
        &leveled_stats,
        &stats,
    )?;
    sol.raw_stats = stats.clone();
    for &(i, g, o) in overrides {
        if let Some(v) = g {
            sol.gain[i] = v;
        }
        if let Some(v) = o {
            sol.offset[i] = v;
        }
    }
    Ok(Prepared {
        sol,
        stats,
        cuts,
        skip: fixed.skip.to_vec(),
        block: block1,
    })
}
fn stats_loaded(
    n: usize,
    load: &impl Fn(usize) -> crate::Result<Gray>,
    neighbours: &[Vec<usize>],
    ctl: &crate::Ctl,
) -> crate::Result<Vec<TileStats>> {
    let hists = (0..n)
        .map(|i| {
            ctl.check()?;
            Ok(smoothed_hist(&load(i)?))
        })
        .collect::<crate::Result<Vec<_>>>()?;
    let pks: Vec<_> = hists.iter().map(|h| peaks(h)).collect();
    (0..n)
        .map(|i| {
            ctl.check()?;
            let mut mi = pks[i].first().copied().unwrap_or(128);
            let mut votes: Vec<_> = neighbours[i]
                .iter()
                .filter_map(|&j| pks[j].first().copied())
                .collect();
            if !votes.is_empty() {
                votes.sort_unstable();
                let k = votes.len() / 2;
                let consensus =
                    (bin_value(votes[(votes.len() - 1) / 2]) + bin_value(votes[k])) * 0.5;
                if (bin_value(mi) - consensus).abs() > 0.06
                    && let Some(&p) = pks[i].iter().min_by(|&&a, &&b| {
                        (bin_value(a) - consensus)
                            .abs()
                            .total_cmp(&(bin_value(b) - consensus).abs())
                    })
                    && (bin_value(p) - consensus).abs() <= 0.04
                {
                    mi = p;
                }
            }
            Ok(tile_stats_at(&load(i)?, bin_value(mi)))
        })
        .collect()
}

/// The joint solve over all tiles (stage 2 of `correct`); `steps` are each tile's overlap
/// rectangles, snapped to its step lines.
#[allow(clippy::too_many_arguments)]
fn solve_loaded<'a>(pl: &Placement, load:&impl Fn(usize)->crate::Result<std::borrow::Cow<'a,Gray>>, block: i64, fixed: &Fixed, nb: Vec<Vec<(usize, [i64; 4])>>, steps: Vec<Vec<[i64; 4]>>, cuts: &[Cuts], stats: &[TileStats], fade_stats: &[TileStats])->crate::Result<JointSolution> {
    let n=pl.tiles.len();
    let gain_free = fixed.gain.is_none();
    let fixed_g = |i: usize| fixed.gain.map_or(1.0, |g| g[i] as f64);
    let mut base = Vec::with_capacity(n);
    let mut u = 0;
    for st in &steps {
        base.push(u);
        u += gain_free as usize + 6 + st.len();
    }
    let lay = Layout { base, gain_free, nb, steps, n: u };

    // Additive-field coefficients of tile i over a block (tile pixels).
    let field_coef = |i: usize, bl: [i64; 4], out: &mut Vec<(usize, f64)>| {
        let t=&pl.tiles[i];
        let fx = ((bl[0] + bl[2]) as f64 * 0.5) / t.w as f64 - 0.5;
        let fy = ((bl[1] + bl[3]) as f64 * 0.5) / t.h as f64 - 0.5;
        if !fixed.skip[i] {
            for (k, c) in poly_terms(fx, fy).into_iter().enumerate() {
                out.push((lay.p(i, k), c));
            }
            for (k, r) in lay.steps[i].iter().enumerate() {
                let f = frac(bl, *r);
                if f > 0.0 {
                    out.push((lay.s(i, k), f));
                }
            }
        }
    };

    // Overlap rows: block means over pixels valid in both tiles agree. Leave out pixels whose
    // field is faded: these rows model the full field, not a value-dependent fraction of it.
    let mut rows: Vec<Row> = Vec::new();
    for a in 0..n {
        let image_a=load(a)?;
        for &(b, ra) in &lay.nb[a] {
            if b < a {
                continue;
            }
            let image_b=load(b)?;
            let (ta, tb) = (&pl.tiles[a], &pl.tiles[b]);
            let (dx, dy) = (ta.x - tb.x, ta.y - tb.y);
            let mut by = ra[1] + 4;
            while by + block <= ra[3] - 4 {
                let mut bx = ra[0] + 4;
                while bx + block <= ra[2] - 4 {
                    let (mut sa, mut sb, mut k) = (0.0f64, 0.0f64, 0usize);
                    for y in by..by + block {
                        for x in bx..bx + block {
                            let va = image_a.px[y as usize * ta.w + x as usize];
                            let vb = image_b.px[(y + dy) as usize * tb.w + (x + dx) as usize];
                            if valid(va) && valid(vb) && fade_stats[a].field_weight(va, 0.0) == 1.0 && fade_stats[b].field_weight(vb, 0.0) == 1.0 {
                                sa += va as f64;
                                sb += vb as f64;
                                k += 1;
                            }
                        }
                    }
                    if k * 2 >= (block * block) as usize {
                        let (ma, mb) = (sa / k as f64, sb / k as f64);
                        let mut c = Vec::with_capacity(24);
                        let mut rhs = 0.0;
                        match lay.g(a) {
                            Some(g) => c.push((g, ma)),
                            None => rhs -= fixed_g(a) * ma,
                        }
                        c.push((lay.o(a), 1.0));
                        let bla = [bx, by, bx + block, by + block];
                        let mut fa = Vec::new();
                        field_coef(a, bla, &mut fa);
                        c.extend(fa);
                        match lay.g(b) {
                            Some(g) => c.push((g, -mb)),
                            None => rhs += fixed_g(b) * mb,
                        }
                        c.push((lay.o(b), -1.0));
                        let blb = [bx + dx, by + dy, bx + dx + block, by + dy + block];
                        let mut fb = Vec::new();
                        field_coef(b, blb, &mut fb);
                        c.extend(fb.into_iter().map(|(j, v)| (j, -v)));
                        rows.push(Row { c, rhs, w: 4.0, kind: 0 });
                    }
                    bx += block;
                }
                by += block;
            }
        }
    }

    // Flatness rows: background blocks of every tile sit at one level.
    let mut flat: Vec<(usize, [i64; 4], f64)> = Vec::new();
    for (i,st) in stats.iter().enumerate().take(n) {
        let t=load(i)?;
        if fixed.skip[i] {
            continue;
        }
        let mut by = 0;
        while by + block <= t.h as i64 {
            let mut bx = 0;
            while bx + block <= t.w as i64 {
                let (mut s, mut k) = (0.0f64, 0usize);
                for y in by..by + block {
                    for x in bx..bx + block {
                        let v = t.px[y as usize * t.w + x as usize];
                        if valid(v) && (v - st.mode).abs() < st.band {
                            s += v as f64;
                            k += 1;
                        }
                    }
                }
                if k * 100 >= (block * block) as usize * 85 {
                    flat.push((i, [bx, by, bx + block, by + block], s / k as f64));
                }
                bx += block;
            }
            by += block;
        }
    }
    let level = {
        let mut v: Vec<f64> = flat.iter().map(|f| f.2).collect();
        if v.is_empty() {
            stats.iter().map(|s| s.mode as f64).sum::<f64>() / n.max(1) as f64
        } else {
            let k = v.len() / 2;
            v.select_nth_unstable_by(k, f64::total_cmp);
            v[k]
        }
    };
    for &(i, bl, m) in &flat {
        let mut c = Vec::with_capacity(16);
        let mut rhs = level;
        match lay.g(i) {
            Some(g) => c.push((g, m)),
            None => rhs -= fixed_g(i) * m,
        }
        c.push((lay.o(i), 1.0));
        field_coef(i, bl, &mut c);
        rows.push(Row { c, rhs, w: 1.0, kind: 1 });
    }

    // Solve with residual trimming (misregistered blocks, bubbles, halos taken for background).
    let mut keep = vec![true; rows.len()];
    let mut x = vec![0.0f64; lay.n];
    for i in 0..n {
        if let Some(g) = lay.g(i) {
            x[g] = 1.0;
        }
    }
    for round in 0..3 {
        let Some(sol) = solve_rows(&rows, &keep, &lay, fixed, n) else { break };
        x = sol;
        if round == 2 {
            break;
        }
        let res: Vec<f64> = rows.iter().map(|r| (r.c.iter().map(|&(j, v)| v * x[j]).sum::<f64>() - r.rhs).abs()).collect();
        for kind in 0..2u8 {
            let mut v: Vec<f64> = rows.iter().zip(&res).filter(|(r, _)| r.kind == kind).map(|(_, e)| *e).collect();
            if v.is_empty() {
                continue;
            }
            let k = v.len() * 3 / 4;
            v.select_nth_unstable_by(k, f64::total_cmp);
            let thr = (v[k] * 3.0).max(0.004);
            for (j, r) in rows.iter().enumerate() {
                if r.kind == kind {
                    keep[j] = res[j] <= thr;
                }
            }
        }
    }

    let mut out = JointSolution { level: level as f32, ..Default::default() };
    for i in 0..n {
        let g = lay.g(i).map_or(fixed_g(i), |j| x[j].clamp(0.2, 5.0));
        out.gain.push(g as f32);
        out.offset.push(x[lay.o(i)].clamp(-0.5, 0.5) as f32);
        let mut f = Field { widths: cuts[i].clone(), ..Default::default() };
        if !fixed.skip[i] {
            for k in 0..5 {
                f.poly[k] = x[lay.p(i, k)];
            }
            f.steps = lay.steps[i].iter().enumerate().map(|(k, r)| (*r, x[lay.s(i, k)])).collect();
        }
        // Range of the field over a coarse grid.
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        let t=&pl.tiles[i];
        for gy in 0..=8 {
            for gx in 0..=8 {
                let (px, py) = ((gx * t.w / 8).min(t.w - 1) as i64, (gy * t.h / 8).min(t.h - 1) as i64);
                let v = f.at(px, py, t.w, t.h);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        out.range.push((hi - lo).max(0.0) as f32);
        out.fields.push(f);
    }
    Ok(out)
}

/// Cuts of a tile: the interior edges of its overlap rectangles (width fitted where they were
/// snapped to a step line, else the default) and its other step lines, sorted, with cuts closer
/// than 16 px merged.
fn cell_cuts(rects: &[[i64; 4]], xs: &[(i64, f64)], ys: &[(i64, f64)], w: i64, h: i64) -> Cuts {
    let norm = |mut v: Vec<(i64, f64)>, n: i64| -> Vec<(i64, f64)> {
        v.retain(|c| c.0 > 16 && c.0 < n - 16);
        // Fitted widths first, so a duplicate default-width edge is the one dropped.
        v.sort_by(|a, b| a.0.cmp(&b.0).then((a.1 == DEFAULT_EDGE).cmp(&(b.1 == DEFAULT_EDGE))));
        let mut out: Vec<(i64, f64)> = Vec::new();
        for c in v {
            if out.last().is_none_or(|l| c.0 - l.0 >= 16) {
                out.push(c);
            }
        }
        out
    };
    let cx: Vec<(i64, f64)> = rects.iter().flat_map(|r| [r[0], r[2]]).map(|e| (e, edge_width(xs, e))).chain(xs.iter().copied()).collect();
    let cy: Vec<(i64, f64)> = rects.iter().flat_map(|r| [r[1], r[3]]).map(|e| (e, edge_width(ys, e))).chain(ys.iter().copied()).collect();
    (norm(cx, w), norm(cy, h))
}

/// Soft membership of position `p` in each of the intervals cut at `cuts` (a partition of
/// unity with a linear ramp of each cut's width across it).
fn memberships(p: f64, cuts: &[(i64, f64)], out: &mut Vec<f64>) {
    out.clear();
    let n = cuts.len() + 1;
    // Cumulative "past cut k" weights.
    let past: Vec<f64> = cuts.iter().map(|&(c, w)| ((p - c as f64) / w.max(1.0) + 0.5).clamp(0.0, 1.0)).collect();
    for k in 0..n {
        let a = if k == 0 { 1.0 } else { past[k - 1] };
        let b = if k == n - 1 { 0.0 } else { past[k] };
        out.push((a - b).max(0.0));
    }
}

/// Stage 1 of `correct`: one tile's background made flat on its own, as a constant per cell of
/// the grid its cuts make plus a 2nd-order surface, fitted to its background blocks (`block`
/// px) and subtracted, so the background sits at its median everywhere in the tile. Returns
/// `None` when the tile has too little background. Also returns the range of the field.
fn flatten_tile(t: &Gray, st: &TileStats, cuts: &Cuts, block: i64) -> Option<(Gray, f32)> {
    let (cx, cy) = cuts;
    let (nx, ny) = (cx.len() + 1, cy.len() + 1);
    let nc = nx * ny;
    let m = nc + 5;
    // Background blocks.
    let mut obs: Vec<(f64, f64, f64)> = Vec::new();
    let mut by = 0;
    while by + block <= t.h as i64 {
        let mut bx = 0;
        while bx + block <= t.w as i64 {
            let (mut s, mut k) = (0.0f64, 0usize);
            for y in by..by + block {
                for x in bx..bx + block {
                    let v = t.px[y as usize * t.w + x as usize];
                    if valid(v) && (v - st.mode).abs() < st.band {
                        s += v as f64;
                        k += 1;
                    }
                }
            }
            if k * 100 >= (block * block) as usize * 85 {
                obs.push(((bx + block / 2) as f64, (by + block / 2) as f64, s / k as f64));
            }
            bx += block;
        }
        by += block;
    }
    if obs.len() < 24 {
        return None;
    }
    let level = {
        let mut v: Vec<f64> = obs.iter().map(|o| o.2).collect();
        let k = v.len() / 2;
        v.select_nth_unstable_by(k, f64::total_cmp);
        v[k]
    };
    let (mut mx, mut my) = (Vec::new(), Vec::new());
    let coef = |x: f64, y: f64, mx: &mut Vec<f64>, my: &mut Vec<f64>| -> Vec<(usize, f64)> {
        memberships(x, cx, mx);
        memberships(y, cy, my);
        let mut c = Vec::with_capacity(9);
        for (j, &wy) in my.iter().enumerate() {
            if wy == 0.0 {
                continue;
            }
            for (i, &wx) in mx.iter().enumerate() {
                if wx > 0.0 {
                    c.push((j * nx + i, wx * wy));
                }
            }
        }
        let fx = (x + 0.5) / t.w as f64 - 0.5;
        let fy = (y + 0.5) / t.h as f64 - 0.5;
        for (k, v) in poly_terms(fx, fy).into_iter().enumerate() {
            c.push((nc + k, v));
        }
        c
    };
    let rows: Vec<Vec<(usize, f64)>> = obs.iter().map(|&(x, y, _)| coef(x, y, &mut mx, &mut my)).collect();
    let mut keep = vec![true; obs.len()];
    let mut sol = vec![0.0f64; m];
    for round in 0..3 {
        let mut ata = vec![0.0f64; m * m];
        let mut atb = vec![0.0f64; m];
        let mut kept = 0;
        for ((c, o), &k) in rows.iter().zip(&obs).zip(&keep) {
            if !k {
                continue;
            }
            kept += 1;
            let rhs = level - o.2;
            for &(i, a) in c {
                atb[i] += a * rhs;
                for &(j, b) in c {
                    ata[i * m + j] += a * b;
                }
            }
        }
        // A gentle pull to 0 keeps it well posed; a cell seen in fewer than 8 background blocks
        // (a narrow strip between two cuts, a cell mostly grain and halo) is pulled to "no
        // correction" in proportion, so it cannot take a value from a few unlucky blocks.
        let d = (kept as f64 / nc as f64).max(1.0);
        for i in 0..m {
            let support = ata[i * m + i];
            let thin = if i < nc { 2.0 * (8.0 - support).max(0.0) } else { 0.0 };
            ata[i * m + i] += 1e-3 * d + thin;
        }
        sol = super::solve_dense(ata, atb, m)?;
        if round == 2 {
            break;
        }
        let res: Vec<f64> = rows.iter().zip(&obs).map(|(c, o)| (c.iter().map(|&(j, v)| v * sol[j]).sum::<f64>() - (level - o.2)).abs()).collect();
        let mut v = res.clone();
        let k = v.len() * 3 / 4;
        v.select_nth_unstable_by(k, f64::total_cmp);
        let thr = (v[k] * 3.0).max(0.002);
        for (k, r) in keep.iter_mut().zip(&res) {
            *k = *r <= thr;
        }
    }
    // Apply: separable memberships per row and column.
    let colm: Vec<Vec<f64>> = (0..t.w)
        .map(|x| {
            let mut v = Vec::new();
            memberships(x as f64, cx, &mut v);
            v
        })
        .collect();
    let mut out = Gray::new(t.w, t.h);
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    crate::util::par_rows(&mut out.px, t.w, |y, row| {
        let mut wy = Vec::new();
        memberships(y as f64, cy, &mut wy);
        let fy = (y as f64 + 0.5) / t.h as f64 - 0.5;
        // Cell values blended along y for this row.
        let line: Vec<f64> = (0..nx).map(|i| wy.iter().enumerate().map(|(j, w)| w * sol[j * nx + i]).sum()).collect();
        for (x, o) in row.iter_mut().enumerate() {
            let fx = (x as f64 + 0.5) / t.w as f64 - 0.5;
            let mut v: f64 = colm[x].iter().zip(&line).map(|(a, b)| a * b).sum();
            v += poly_terms(fx, fy).iter().zip(&sol[nc..]).map(|(a, b)| a * b).sum::<f64>();
            let raw = t.px[y * t.w + x];
            *o = raw + st.field_weight(raw, v as f32) * v as f32;
        }
    });
    for gy in 0..=8 {
        for gx in 0..=8 {
            let (x, y) = ((gx * t.w / 8).min(t.w - 1), (gy * t.h / 8).min(t.h - 1));
            let v = out.px[y * t.w + x] - t.px[y * t.w + x];
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    Some((out, (hi - lo).max(0.0)))
}

/// An overlap edge moves onto a step line found within this many pixels.
const SNAP: i64 = 20;

/// Step lines of one tile: x positions where the background's column profile steps, and y
/// positions where its row profile does (beam-exposure edges, wherever they fall).
fn step_lines(t: &Gray, st: &TileStats) -> Cuts {
    let bg = |v: f32| valid(v) && (v - st.mode).abs() < st.band;
    let mut cs = vec![(0.0f64, 0usize); t.w];
    let mut rs = vec![(0.0f64, 0usize); t.h];
    for (row, r) in t.px.chunks_exact(t.w).zip(rs.iter_mut()) {
        for (&v, c) in row.iter().zip(cs.iter_mut()) {
            if bg(v) {
                c.0 += v as f64;
                c.1 += 1;
                r.0 += v as f64;
                r.1 += 1;
            }
        }
    }
    let prof = |a: &[(f64, usize)], other: usize| -> Vec<f64> { a.iter().map(|&(s, k)| if k * 4 >= other { s / k as f64 } else { f64::NAN }).collect() };
    let (cp, rp) = (prof(&cs, t.h), prof(&rs, t.w));
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    change_points(&cp, &mut xs);
    change_points(&rp, &mut ys);
    let mut xs: Vec<(i64, f64)> = xs.into_iter().map(|s| fit_edge(&cp, s)).collect();
    let mut ys: Vec<(i64, f64)> = ys.into_iter().map(|s| fit_edge(&rp, s)).collect();
    xs.sort_by_key(|c| c.0);
    ys.sort_by_key(|c| c.0);
    (xs, ys)
}

/// Position and width of the edge near `s` in a profile: the linear ramp `a + h·ramp((i - c) /
/// w)` that fits the 64 samples around it best, over centres within ±6 px and widths 2 to 24
/// px (sharp beam edges, and edges softened by beam blur and 8 px JPEG blocks).
fn fit_edge(p: &[f64], s: i64) -> (i64, f64) {
    let n = p.len() as i64;
    let (lo, hi) = ((s - 32).max(0), (s + 32).min(n));
    let pts: Vec<(f64, f64)> = (lo..hi).filter(|&i| !p[i as usize].is_nan()).map(|i| (i as f64 + 0.5, p[i as usize])).collect();
    let mut best = (f64::MAX, s, DEFAULT_EDGE);
    for c in s - 6..=s + 6 {
        for w in [2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0] {
            // Least squares for a and h with the ramp fixed.
            let (mut s1, mut sr, mut srr, mut sv, mut srv) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for &(x, v) in &pts {
                let r = ((x - c as f64) / w + 0.5).clamp(0.0, 1.0);
                s1 += 1.0;
                sr += r;
                srr += r * r;
                sv += v;
                srv += r * v;
            }
            let det = s1 * srr - sr * sr;
            if det.abs() < 1e-12 {
                continue;
            }
            let h = (s1 * srv - sr * sv) / det;
            let a = (sv - h * sr) / s1;
            let sse: f64 = pts.iter().map(|&(x, v)| {
                let r = ((x - c as f64) / w + 0.5).clamp(0.0, 1.0);
                (v - a - h * r) * (v - a - h * r)
            }).sum();
            if sse < best.0 {
                best = (sse, c, w);
            }
        }
    }
    (best.1, best.2)
}

/// Step positions in a profile: where the mean of the `W` samples after a position differs from
/// the mean of the `W` before it by at least ~1 grey level and many times the profile noise
/// (from neighbour differences, so gradients and the steps themselves do not inflate it),
/// strongest first, one per `W` (non-maximum suppression). A smooth gradient moves the two means
/// apart by its slope times `W` only, far below a step.
fn change_points(p: &[f64], out: &mut Vec<i64>) {
    const W: usize = 40;
    const EDGE: usize = 24;
    let n = p.len();
    if n < 2 * W + 2 * EDGE {
        return;
    }
    let mut d: Vec<f64> = p.windows(2).filter(|w| !w[0].is_nan() && !w[1].is_nan()).map(|w| (w[1] - w[0]).abs()).collect();
    if d.len() < W {
        return;
    }
    let k = d.len() / 2;
    d.select_nth_unstable_by(k, f64::total_cmp);
    let sigma = (1.4826 * d[k] / std::f64::consts::SQRT_2).max(1e-5);
    // Prefix sums over valid samples.
    let mut cs = vec![0.0f64; n + 1];
    let mut cn = vec![0usize; n + 1];
    for i in 0..n {
        let ok = !p[i].is_nan();
        cs[i + 1] = cs[i] + if ok { p[i] } else { 0.0 };
        cn[i + 1] = cn[i] + ok as usize;
    }
    let mean = |a: usize, b: usize| -> Option<(f64, usize)> {
        let k = cn[b] - cn[a];
        (k * 2 >= b - a).then(|| ((cs[b] - cs[a]) / k as f64, k))
    };
    let lo = EDGE.max(W);
    let hi = n - EDGE.max(W);
    let mut cand: Vec<(f64, usize)> = Vec::new();
    for s in lo..hi {
        let (Some((l, nl)), Some((r, nr))) = (mean(s - W, s), mean(s, s + W)) else { continue };
        let diff = r - l;
        let t = diff.abs() / (sigma * (1.0 / nl as f64 + 1.0 / nr as f64).sqrt());
        if diff.abs() >= 0.004 && t >= 8.0 {
            cand.push((diff.abs(), s));
        }
    }
    cand.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, s) in cand {
        if out.len() >= 8 {
            break;
        }
        if out.iter().all(|&o| (o - s as i64).unsigned_abs() as usize >= W) {
            // The wide windows can see a neighbouring step: place the edge with short ones.
            const R: usize = 12;
            let best = (s.saturating_sub(R).max(R)..(s + R).min(n - R))
                .filter_map(|q| Some(((mean(q, q + R)?.0 - mean(q - R, q)?.0).abs(), q)))
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map_or(s, |b| b.1);
            out.push(best as i64);
        }
    }
}

impl Field {
    fn at(&self, x: i64, y: i64, w: usize, h: usize) -> f64 {
        let fx = (x as f64 + 0.5) / w as f64 - 0.5;
        let fy = (y as f64 + 0.5) / h as f64 - 0.5;
        let mut v: f64 = poly_terms(fx, fy).iter().zip(&self.poly).map(|(a, b)| a * b).sum();
        for (r, s) in &self.steps {
            if x >= r[0] && x < r[2] && y >= r[1] && y < r[3] {
                v += s;
            }
        }
        v
    }
}

impl JointSolution {
    /// Applies gain, offset and field to tile `i`; step edges get a linear ramp of the width
    /// fitted to that edge (TM4000 edges run from sharp to ~16 px: beam blur, 8 px JPEG blocks).
    pub fn apply(&self, i: usize, t: &Gray) -> Gray {
        let f = &self.fields[i];
        let (g, o) = (self.gain[i], self.offset[i]);
        let (w, h) = (t.w, t.h);
        // Separable ramps: inside(x) for a rectangle edge interior to the tile.
        let ramp = |p: i64, lo: i64, hi: i64, n: i64, cuts: &[(i64, f64)]| -> f64 {
            let a = if lo <= 0 { 1.0 } else { ((p as f64 + 0.5 - lo as f64) / edge_width(cuts, lo) + 0.5).clamp(0.0, 1.0) };
            let b = if hi >= n { 1.0 } else { ((hi as f64 - p as f64 - 0.5) / edge_width(cuts, hi) + 0.5).clamp(0.0, 1.0) };
            a.min(b)
        };
        let colw: Vec<Vec<f64>> = f.steps.iter().map(|(r, _)| (0..w as i64).map(|x| ramp(x, r[0], r[2], w as i64, &f.widths.0)).collect()).collect();
        let mut out = Gray::new(w, h);
        crate::util::par_rows(&mut out.px, w, |y, row| {
            let fy = (y as f64 + 0.5) / h as f64 - 0.5;
            let rows: Vec<f64> = f.steps.iter().map(|(r, _)| ramp(y as i64, r[1], r[3], h as i64, &f.widths.1)).collect();
            for (x, o_px) in row.iter_mut().enumerate() {
                let fx = (x as f64 + 0.5) / w as f64 - 0.5;
                let mut v: f64 = poly_terms(fx, fy).iter().zip(&f.poly).map(|(a, b)| a * b).sum();
                for (k, (_, s)) in f.steps.iter().enumerate() {
                    let m = rows[k] * colw[k][x];
                    if m > 0.0 {
                        v += s * m;
                    }
                }
                let raw = t.px[y * w + x];
                // Scale ramp width by the positive gain too: the transition slope then
                // stays between 0.5*g and 1.5*g. Composition with stage one is monotonic.
                *o_px = raw * g + o + self.raw_stats[i].field_weight(raw, v as f32 / g.abs().max(f32::MIN_POSITIVE)) * v as f32;
            }
        });
        out
    }
}

/// Normal equations of the kept rows plus the priors, solved by banded Cholesky (unknowns of
/// one tile are contiguous and rows join neighbours only, so the band is narrow).
fn solve_rows(rows: &[Row], keep: &[bool], lay: &Layout, fixed: &Fixed, n_tiles: usize) -> Option<Vec<f64>> {
    let m = lay.n;
    // Bandwidth from the rows.
    let mut bw = 0;
    for r in rows {
        let (lo, hi) = r.c.iter().fold((usize::MAX, 0), |(a, b), &(j, _)| (a.min(j), b.max(j)));
        bw = bw.max(hi.saturating_sub(lo));
    }
    let bw = bw.min(m.saturating_sub(1));
    let width = bw + 1;
    // Band storage: a[i][d] = A[i][i - d], d in 0..=bw (lower triangle).
    let mut a = vec![0.0f64; m * width];
    let mut b = vec![0.0f64; m];
    for (r, &k) in rows.iter().zip(keep) {
        if !k {
            continue;
        }
        for &(i, ci) in &r.c {
            b[i] += r.w * ci * r.rhs;
            for &(j, cj) in &r.c {
                if j <= i {
                    a[i * width + (i - j)] += r.w * ci * cj;
                }
            }
        }
    }
    // Priors relative to the data weight of each kind of unknown: gains to 1, the rest to 0.
    let mean_diag = |sel: &dyn Fn(usize) -> Vec<usize>| -> f64 {
        let v: Vec<f64> = (0..n_tiles).flat_map(sel).map(|j| a[j * width]).collect();
        (v.iter().sum::<f64>() / v.len().max(1) as f64).max(1e-9)
    };
    let dg = mean_diag(&|i| lay.g(i).into_iter().collect());
    let doff = mean_diag(&|i| vec![lay.o(i)]);
    let dp = mean_diag(&|i| (0..5).map(|k| lay.p(i, k)).collect());
    let ds = mean_diag(&|i| (0..lay.steps[i].len()).map(|k| lay.s(i, k)).collect());
    for i in 0..n_tiles {
        if let Some(g) = lay.g(i) {
            a[g * width] += fixed.regularisation * dg;
            b[g] += fixed.regularisation * dg;
        }
        a[lay.o(i) * width] += 1e-6 * doff;
        let strong = if fixed.skip[i] { 1e6 } else { 1.0 };
        for k in 0..5 {
            a[lay.p(i, k) * width] += strong * 1e-3 * dp;
        }
        for k in 0..lay.steps[i].len() {
            a[lay.s(i, k) * width] += strong * 2e-3 * ds;
        }
    }
    // Banded Cholesky: A = L Lᵀ in place.
    for j in 0..m {
        let lo = j.saturating_sub(bw);
        let mut d = a[j * width];
        for k in lo..j {
            let l = a[j * width + (j - k)];
            d -= l * l;
        }
        if d <= 1e-18 {
            return None;
        }
        let d = d.sqrt();
        a[j * width] = d;
        for i in j + 1..(j + width).min(m) {
            let lo_i = i.saturating_sub(bw).max(lo);
            let mut s = a[i * width + (i - j)];
            for k in lo_i..j {
                s -= a[i * width + (i - k)] * a[j * width + (j - k)];
            }
            a[i * width + (i - j)] = s / d;
        }
    }
    // Forward and back substitution.
    let mut y = b;
    for i in 0..m {
        let lo = i.saturating_sub(bw);
        let mut s = y[i];
        for k in lo..i {
            s -= a[i * width + (i - k)] * y[k];
        }
        y[i] = s / a[i * width];
    }
    for i in (0..m).rev() {
        let mut s = y[i];
        for k in i + 1..(i + width).min(m) {
            s -= a[k * width + (k - i)] * y[k];
        }
        y[i] = s / a[i * width];
    }
    Some(y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additive_field_fade_preserves_affine_and_near_black_cl() {
        let bse = TileStats { mode: 0.41, band: 0.04 };
        assert_eq!(bse.field_weight(0.27, 0.0), 0.0);
        assert_eq!(bse.field_weight(bse.mode - 3.0 * bse.band, 0.0), 0.0);
        assert_eq!(bse.field_weight(bse.mode - 1.5 * bse.band, 0.0), 1.0);
        assert!((bse.field_weight(bse.mode - 2.25 * bse.band, 0.0) - 0.5).abs() < 1e-6);
        for v in [0.41, 0.65, 1.0] {
            assert_eq!(bse.field_weight(v, 0.0), 1.0);
        }
        let cl = TileStats { mode: 0.04, band: 0.04 };
        assert!(cl.mode - 1.5 * cl.band <= 0.0);
        for k in 0..=255 {
            assert_eq!(cl.field_weight(k as f32 / 255.0, 0.0), 1.0);
        }
        let t = Gray { w: 3, h: 1, px: vec![0.27, 0.41, 0.8] };
        let sol = JointSolution {
            gain: vec![1.2], offset: vec![0.03], raw_stats: vec![bse],
            fields: vec![Field { poly: [0.06, 0.0, 0.0, 0.0, 0.0], ..Default::default() }],
            ..Default::default()
        };
        let out = sol.apply(0, &t);
        assert_eq!(out.px[0], t.px[0] * 1.2 + 0.03);
        assert_eq!(out.px[1], t.px[1] * 1.2 + 0.03);
        assert!((out.px[2] - (t.px[2] * 1.2 + 0.03 + 0.02)).abs() < 1e-6);
    }

    #[test]
    fn strong_fields_keep_raw_ramps_monotonic_and_gentle() {
        let st = TileStats { mode: 0.41, band: 0.04 };
        let t = Gray { w: 1001, h: 1, px: (0..=1000).map(|k| k as f32 / 1000.0).collect() };
        for field in [-0.1, 0.1] {
            // Stage one's pixel mapping, and stage two with both low and high gains.
            let first: Vec<f32> = t.px.iter().map(|&v| v + st.field_weight(v, field) * field).collect();
            let check = |px: &[f32], gain: f32| {
                for (raw, corrected) in t.px.windows(2).zip(px.windows(2)) {
                    let slope = (corrected[1] - corrected[0]) / (raw[1] - raw[0]);
                    assert!((0.5 * gain - 0.001..=1.5 * gain + 0.001).contains(&slope), "field {field}, gain {gain}, raw {raw:?}, slope {slope}");
                }
            };
            check(&first, 1.0);
            for gain in [0.2, 1.0, 2.0] {
                let sol = JointSolution {
                    gain: vec![gain], offset: vec![0.03], raw_stats: vec![st.clone()],
                    fields: vec![Field { steps: vec![([0, 0, t.w as i64, 1], field as f64)], ..Default::default() }],
                    ..Default::default()
                };
                check(&sol.apply(0, &t).px, gain);
                let composed = sol.apply(0, &Gray { w: t.w, h: 1, px: first.clone() });
                assert!(composed.px.windows(2).all(|w| w[1] >= w[0]));
            }
        }
    }

    #[test]
    fn both_stages_keep_the_raw_background_statistics_for_fading() {
        let (w, h) = (128, 96);
        let t = Gray { w, h, px: (0..w * h).map(|k| 0.38 + 0.05 * ((k % w) as f32 / w as f32).powi(2)).collect() };
        let raw = tiles_stats(std::slice::from_ref(&t), &[Vec::new()]);
        let (flat, _) = flatten_tile(&t, &raw[0], &(Vec::new(), Vec::new()), 8).unwrap();
        let leveled = tiles_stats(&[flat], &[Vec::new()]);
        assert_ne!(raw[0].mode, leveled[0].mode, "scene must change the stage-two histogram bin");
        let pl = Placement {
            tiles: vec![crate::mosaic::TilePlacement { failed: false, index: 0, prior: [0.0; 2], pos: [0.0; 2], x: 0, y: 0, w, h, drift: [0.0; 2] }],
            pairs: Vec::new(), width: w, height: h, rms_residual: 0.0, max_residual: 0.0,
        };
        let bp = BackgroundParams { cell: 32, ..Default::default() };
        let (_, sol, _, _) = correct(&pl, &[t], &bp, &Fixed { gain: None, skip: &[false], regularisation: 0.01 }, &[]);
        assert_eq!(sol.raw_stats[0].mode, raw[0].mode);
        assert_eq!(sol.raw_stats[0].band, raw[0].band);
    }

    #[test]
    fn all_black_statistics_use_the_same_default_peak() {
        let t = Gray::new(32, 32);
        let single = tile_stats(&t);
        let multiple = tiles_stats(&[t], &[Vec::new()]);
        assert_eq!(single.mode, bin_value(128));
        assert_eq!(single.mode, multiple[0].mode);
        assert_eq!(single.band, multiple[0].band);
    }

    #[test]
    fn histogram_shoulders_and_plateaus_are_one_peak() {
        // A shallow dip on the flank is a shoulder, not a second population.
        assert_eq!(peaks(&[0., 3., 8., 10., 10., 9., 9.5, 8., 3., 0.]), vec![3]);
        // A rising plateau is not a peak at either edge.
        assert_eq!(peaks(&[0., 3., 3., 6., 10., 10., 6., 0.]), vec![4]);
        assert_eq!(peaks(&[0., 10., 10., 2., 6., 6., 0.]), vec![1, 4]);
        assert!(peaks(&[0.; 256]).is_empty());
    }

    fn populations(main: f32, secondary: f32) -> Gray {
        let mut t = Gray::new(90, 30);
        for (i, v) in t.px.iter_mut().enumerate() {
            *v = if i % 90 < 60 { main } else { secondary };
        }
        t
    }

    #[test]
    fn background_switch_requires_local_nearby_evidence() {
        let tiles = vec![
            populations(0.10, 0.25), populations(0.20, 0.20),
            populations(0.10, 0.15), populations(0.14, 0.14),
            populations(0.04, 0.30), populations(0.30, 0.30),
        ];
        let neighbours = vec![vec![1], vec![], vec![3], vec![], vec![5], vec![]];
        let stats = tiles_stats(&tiles, &neighbours);
        assert!((stats[0].mode - 0.10).abs() < 0.01, "distant alternative stays out");
        assert!((stats[2].mode - 0.10).abs() < 0.01, "nearby main peak stays put");
        assert!((stats[4].mode - 0.30).abs() < 0.01, "off-mount main peak switches");
        assert!((stats[1].mode - 0.20).abs() < 0.01, "isolated main peak stays put");
        // Remote bright tiles cannot vote; an even split between populations is ambiguous.
        let neighbours = vec![vec![2, 3], vec![], vec![], vec![], vec![1, 5], vec![]];
        let stats = tiles_stats(&tiles, &neighbours);
        assert!((stats[0].mode - 0.10).abs() < 0.01);
        assert!((stats[4].mode - 0.04).abs() < 0.01);
    }
}
