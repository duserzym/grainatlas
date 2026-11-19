//! Per-tile background leveling: mask grains with a coarse threshold, take the
//! median of the background on a coarse grid, fit a 2nd-order polynomial (or fill
//! and smooth the grid) into a surface, subtract it and add the mosaic-wide median background back.

use serde::{Deserialize, Serialize};

use crate::image::Gray;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BackgroundParams {
    /// `joint` (default): gains, offsets, surfaces and exposure steps of all tiles in one solve
    /// that keeps overlaps in agreement (`photometric::joint`); `tile`: each tile leveled on its
    /// own after the gain and offset (the earlier behaviour; `model`, `smooth_cells`,
    /// `threshold`, `dilate` and `edge_steps` apply to it).
    pub method: LevelMethod,
    /// Surface model: a 2nd-order polynomial fitted to the cell medians (default), or the
    /// cell grid itself, filled and smoothed (follows local charging, risks following grains).
    pub model: SurfaceModel,
    /// Grid cell size in pixels (`joint` uses blocks of half a cell).
    pub cell: usize,
    /// Gaussian smoothing of the cell grid, in cells.
    pub smooth_cells: f32,
    /// Grain threshold (0..1); `None` = Otsu on all tiles.
    pub threshold: Option<f32>,
    /// Grain mask dilation in pixels (halos around grains are not background).
    pub dilate: usize,
    /// Detect and remove beam-exposure steps: the band of a tile that the beam already scanned
    /// while a neighbour was captured is darker, with a sharp edge a smooth surface cannot fit.
    pub edge_steps: bool,
}

impl Default for BackgroundParams {
    fn default() -> Self {
        Self { method: LevelMethod::Joint, model: SurfaceModel::Poly2, cell: 64, smooth_cells: 1.5, threshold: None, dilate: 6, edge_steps: true }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LevelMethod {
    #[default]
    Joint,
    Tile,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceModel {
    #[default]
    Poly2,
    Smooth,
}

/// Otsu threshold of values in 0..1 (256 bins).
pub fn otsu(values: impl Iterator<Item = f32>) -> f32 {
    let mut hist = [0u64; 256];
    for v in values {
        hist[(v.clamp(0.0, 1.0) * 255.0) as usize] += 1;
    }
    otsu_hist(&hist)
}
pub(crate) fn otsu_hist(hist:&[u64;256])->f32 {
    let n=hist.iter().sum::<u64>();
    if n == 0 {
        return 0.5;
    }
    let total: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let (mut sb, mut wb, mut best, mut thr) = (0.0f64, 0u64, -1.0f64, 0usize);
    for (t, &c) in hist.iter().enumerate() {
        wb += c;
        if wb == 0 {
            continue;
        }
        let wf = n - wb;
        if wf == 0 {
            break;
        }
        sb += t as f64 * c as f64;
        let mb = sb / wb as f64;
        let mf = (total - sb) / wf as f64;
        let between = wb as f64 * wf as f64 * (mb - mf) * (mb - mf);
        if between > best {
            best = between;
            thr = t;
        }
    }
    (thr as f32 + 0.5) / 255.0
}

/// Levels every tile in place. Returns the median background and each tile's surface range.
pub fn level(tiles: &mut [Gray], skip: &[bool], p: &BackgroundParams) -> (f32, Vec<f32>) {
    level_ctl(tiles, skip, p, &crate::Ctl::NONE).expect("uncancelled leveling")
}

pub fn level_ctl(tiles: &mut [Gray], skip: &[bool], p: &BackgroundParams, ctl: &crate::Ctl) -> crate::Result<(f32, Vec<f32>)> {
    ctl.check()?;
    let thr = p.threshold.unwrap_or_else(|| otsu(tiles.iter().flat_map(|t| t.px.iter().step_by(7).copied())));
    if p.edge_steps {
        let steps: Vec<EdgeSteps> = crate::util::try_par_map(tiles.len(), ctl, |i| Ok(if skip.get(i).copied().unwrap_or(false) { EdgeSteps::default() } else { detect_edge_steps(&tiles[i], thr, p.dilate) }))?;
        for (t, st) in tiles.iter_mut().zip(&steps) {
            st.apply(t);
        }
    }
    let surfaces: Vec<Option<Gray>> = crate::util::try_par_map(tiles.len(), ctl, |i| Ok(if skip.get(i).copied().unwrap_or(false) { None } else { surface(&tiles[i], thr, p) }))?;
    // Mosaic median background from the surfaces.
    let mut all: Vec<f32> = surfaces.iter().flatten().flat_map(|s| s.px.iter().step_by(31).copied()).collect();
    if all.is_empty() {
        return Ok((0.0, vec![0.0; tiles.len()]));
    }
    all.sort_by(f32::total_cmp);
    let median = all[all.len() / 2];
    let mut ranges = Vec::with_capacity(tiles.len());
    for (t, s) in tiles.iter_mut().zip(&surfaces) {
        ctl.check()?;
        match s {
            Some(s) => {
                let (mn, mx) = s.px.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
                ranges.push(mx - mn);
                for (v, b) in t.px.iter_mut().zip(&s.px) {
                    *v = *v - b + median;
                }
            }
            None => ranges.push(0.0),
        }
    }
    Ok((median, ranges))
}

/// Smooth background surface of one tile at full resolution, or `None` if it has too little
/// background.
pub fn surface(t: &Gray, thr: f32, p: &BackgroundParams) -> Option<Gray> {
    let cell = p.cell.max(8);
    // Grain mask, dilated.
    let mut grain: Vec<bool> = t.px.iter().map(|&v| v > thr).collect();
    if p.dilate > 0 {
        grain = crate::segment::morph::dilate(&grain, t.w, t.h, p.dilate);
    }
    let (gw, gh) = (t.w.div_ceil(cell), t.h.div_ceil(cell));
    let mut grid = vec![f32::NAN; gw * gh];
    let mut any = 0;
    for cy in 0..gh {
        for cx in 0..gw {
            let mut v: Vec<f32> = Vec::new();
            for y in cy * cell..((cy + 1) * cell).min(t.h) {
                for x in cx * cell..((cx + 1) * cell).min(t.w) {
                    if !grain[y * t.w + x] {
                        v.push(t.px[y * t.w + x]);
                    }
                }
            }
            if v.len() >= cell * cell / 8 {
                v.sort_by(f32::total_cmp);
                grid[cy * gw + cx] = v[v.len() / 2];
                any += 1;
            }
        }
    }
    if any < (gw * gh) / 4 {
        return None;
    }
    if p.model == SurfaceModel::Poly2 {
        // Least squares on cell centres in normalised coordinates: 1, x, y, x², xy, y².
        let basis = |x: f64, y: f64| [1.0, x, y, x * x, x * y, y * y];
        let mut ata = vec![0.0f64; 36];
        let mut atb = vec![0.0f64; 6];
        for cy in 0..gh {
            for cx in 0..gw {
                let v = grid[cy * gw + cx];
                if v.is_nan() {
                    continue;
                }
                let x = ((cx as f64 + 0.5) * cell as f64) / t.w as f64 - 0.5;
                let y = ((cy as f64 + 0.5) * cell as f64) / t.h as f64 - 0.5;
                let b = basis(x, y);
                for i in 0..6 {
                    for j in 0..6 {
                        ata[i * 6 + j] += b[i] * b[j];
                    }
                    atb[i] += b[i] * v as f64;
                }
            }
        }
        for i in 0..6 {
            ata[i * 6 + i] += 1e-9;
        }
        let c = super::solve_dense(ata, atb, 6)?;
        let mut out = Gray::new(t.w, t.h);
        crate::util::par_rows(&mut out.px, t.w, |y, row| {
            let fy = (y as f64 + 0.5) / t.h as f64 - 0.5;
            for (x, o) in row.iter_mut().enumerate() {
                let fx = (x as f64 + 0.5) / t.w as f64 - 0.5;
                let b = basis(fx, fy);
                *o = (0..6).map(|i| c[i] * b[i]).sum::<f64>() as f32;
            }
        });
        return Some(out);
    }
    // Fill empty cells from the nearest filled ones (repeated neighbour averaging).
    for _ in 0..(gw + gh) {
        let mut changed = false;
        let prev = grid.clone();
        for cy in 0..gh {
            for cx in 0..gw {
                if !prev[cy * gw + cx].is_nan() {
                    continue;
                }
                let (mut s, mut n) = (0.0, 0);
                for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (x, y) = (cx as i64 + dx, cy as i64 + dy);
                    if x >= 0 && y >= 0 && (x as usize) < gw && (y as usize) < gh {
                        let v = prev[y as usize * gw + x as usize];
                        if !v.is_nan() {
                            s += v;
                            n += 1;
                        }
                    }
                }
                if n > 0 {
                    grid[cy * gw + cx] = s / n as f32;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let g = Gray { w: gw, h: gh, px: grid };
    let g = gaussian_blur(&g, p.smooth_cells);
    // Bilinear upsample (cell centres).
    let mut out = Gray::new(t.w, t.h);
    crate::util::par_rows(&mut out.px, t.w, |y, row| {
        let fy = (y as f32 + 0.5) / cell as f32 - 0.5;
        for (x, o) in row.iter_mut().enumerate() {
            let fx = (x as f32 + 0.5) / cell as f32 - 0.5;
            *o = g.bilinear(fx, fy);
        }
    });
    Some(out)
}

/// Separable Gaussian blur with clamped edges.
pub fn gaussian_blur(g: &Gray, sigma: f32) -> Gray {
    if sigma <= 0.0 {
        return g.clone();
    }
    let r = (sigma * 3.0).ceil() as i64;
    let k: Vec<f32> = (-r..=r).map(|i| libm::expf(-(i * i) as f32 / (2.0 * sigma * sigma))).collect();
    let ks: f32 = k.iter().sum();
    let k: Vec<f32> = k.iter().map(|v| v / ks).collect();
    let mut tmp = Gray::new(g.w, g.h);
    crate::util::par_rows(&mut tmp.px, g.w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let mut s = 0.0;
            for (j, kv) in k.iter().enumerate() {
                s += kv * g.atc(x as i64 + j as i64 - r, y as i64);
            }
            *o = s;
        }
    });
    let mut out = Gray::new(g.w, g.h);
    crate::util::par_rows(&mut out.px, g.w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let mut s = 0.0;
            for (j, kv) in k.iter().enumerate() {
                s += kv * tmp.atc(x as i64, y as i64 + j as i64 - r);
            }
            *o = s;
        }
    });
    out
}

/// A step along one edge: the band `[0, at)` px from that edge is offset by `-height`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub at: usize,
    pub height: f32,
}

/// Steps found on the four edges of a tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EdgeSteps {
    pub top: Option<Step>,
    pub bottom: Option<Step>,
    pub left: Option<Step>,
    pub right: Option<Step>,
}

impl EdgeSteps {
    /// Adds each step's height back inside its band, with a 4 px ramp at the band edge.
    pub fn apply(&self, t: &mut Gray) {
        const RAMP: f32 = 4.0;
        let (w, h) = (t.w, t.h);
        let k = |d: usize, s: &Step| -> f32 {
            let x = (s.at as f32 - d as f32) / RAMP + 0.5;
            x.clamp(0.0, 1.0) * s.height
        };
        for y in 0..h {
            let mut row_add = 0.0;
            if let Some(s) = &self.top {
                row_add += k(y, s);
            }
            if let Some(s) = &self.bottom {
                row_add += k(h - 1 - y, s);
            }
            for x in 0..w {
                let mut a = row_add;
                if let Some(s) = &self.left {
                    a += k(x, s);
                }
                if let Some(s) = &self.right {
                    a += k(w - 1 - x, s);
                }
                t.px[y * w + x] += a;
            }
        }
    }
}

/// Finds steps in the background profile near each edge (within 60 % of the size).
pub fn detect_edge_steps(t: &Gray, thr: f32, dilate: usize) -> EdgeSteps {
    let mut grain: Vec<bool> = t.px.iter().map(|&v| v > thr).collect();
    if dilate > 0 {
        grain = crate::segment::morph::dilate(&grain, t.w, t.h, dilate);
    }
    // Background median per row and per column (NaN where too little background).
    let median = |mut v: Vec<f32>| -> f32 {
        if v.len() < 16 {
            return f32::NAN;
        }
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let rows: Vec<f32> = (0..t.h).map(|y| median((0..t.w).step_by(2).filter(|&x| !grain[y * t.w + x]).map(|x| t.px[y * t.w + x]).collect())).collect();
    let cols: Vec<f32> = (0..t.w).map(|x| median((0..t.h).step_by(2).filter(|&y| !grain[y * t.w + x]).map(|y| t.px[y * t.w + x]).collect())).collect();
    let (top, bottom) = steps_both_ends(&rows);
    let (left, right) = steps_both_ends(&cols);
    EdgeSteps { top, bottom, left, right }
}

/// Steps at both ends of a profile: the more significant one first, then the other end on the
/// profile with the first removed (a strong step at one end otherwise leaks into the other).
fn steps_both_ends(p: &[f32]) -> (Option<Step>, Option<Step>) {
    let rev = |v: &[f32]| v.iter().rev().copied().collect::<Vec<f32>>();
    let a = find_step(p);
    let b = find_step(&rev(p));
    let remove = |v: &[f32], st: &Step| -> Vec<f32> { v.iter().enumerate().map(|(i, &x)| if i < st.at { x + st.height } else { x }).collect() };
    match (a, b) {
        (None, None) => (None, None),
        (Some((ta, sa)), b) if b.is_none_or(|(tb, _)| ta >= tb) => {
            let q = remove(p, &sa);
            (Some(sa), find_step(&rev(&q)).map(|x| x.1))
        }
        (_, Some((_, sb))) => {
            let q = rev(&remove(&rev(p), &sb));
            (find_step(&q).map(|x| x.1), Some(sb))
        }
        (Some((_, sa)), None) => (Some(sa), None),
    }
}

/// Best step in a profile measured from index 0: for each candidate `s` fits
/// `a + b·i + h·[i < s]` by least squares over the profile and keeps the split whose `h` is most
/// significant, so a smooth gradient is not mistaken for a step.
fn find_step(p: &[f32]) -> Option<(f64, Step)> {
    let n = p.len();
    if n < 128 {
        return None;
    }
    let pts: Vec<(f64, f64)> = p.iter().enumerate().skip(8).take(n.saturating_sub(16)).filter(|(_, v)| !v.is_nan()).map(|(i, &v)| (i as f64, v as f64)).collect();
    if pts.len() < 64 {
        return None;
    }
    let mut best: Option<(f64, Step)> = None;
    let max_at = (n as f64 * 0.6) as usize;
    let mut s = 32;
    while s <= max_at {
        let mut ata = [0.0f64; 9];
        let mut atb = [0.0f64; 3];
        for &(x, v) in &pts {
            let b = [1.0, x / n as f64, if (x as usize) < s { 1.0 } else { 0.0 }];
            for i in 0..3 {
                for j in 0..3 {
                    ata[i * 3 + j] += b[i] * b[j];
                }
                atb[i] += b[i] * v;
            }
        }
        if let Some(c) = super::solve_dense(ata.to_vec(), atb.to_vec(), 3) {
            let mut rss = 0.0;
            for &(x, v) in &pts {
                let f = c[0] + c[1] * x / n as f64 + if (x as usize) < s { c[2] } else { 0.0 };
                rss += (v - f) * (v - f);
            }
            let sigma2 = rss / (pts.len() as f64 - 3.0).max(1.0);
            // Variance of h: sigma² times the (2,2) entry of (AᵀA)⁻¹.
            let inv = invert3(&ata);
            if let Some(inv) = inv {
                let t = c[2] / (sigma2 * inv[8]).max(1e-30).sqrt();
                if best.is_none_or(|(b, _)| t.abs() > b) {
                    best = Some((t.abs(), Step { at: s, height: -c[2] as f32 }));
                }
            }
        }
        s += 4;
    }
    let (t, st) = best?;
    // A real exposure step is large against the row-to-row noise and at least ~1 grey level.
    (t > 12.0 && st.height.abs() > 0.004).then_some((t, st))
}

fn invert3(m: &[f64; 9]) -> Option<[f64; 9]> {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if det.abs() < 1e-18 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        (m[4] * m[8] - m[5] * m[7]) * d,
        (m[2] * m[7] - m[1] * m[8]) * d,
        (m[1] * m[5] - m[2] * m[4]) * d,
        (m[5] * m[6] - m[3] * m[8]) * d,
        (m[0] * m[8] - m[2] * m[6]) * d,
        (m[2] * m[3] - m[0] * m[5]) * d,
        (m[3] * m[7] - m[4] * m[6]) * d,
        (m[1] * m[6] - m[0] * m[7]) * d,
        (m[0] * m[4] - m[1] * m[3]) * d,
    ])
}
