//! Global fitting with a reloadable tile source. Decoded input and correction
//! scratch are tile-sized; recipes and solve observations are compact metadata.
use super::*;
use crate::{Ctl, Result};
pub(crate) struct Prepared {
    gain: Vec<f32>,
    offset: Vec<f32>,
    joint: Option<joint::Prepared>,
    legacy: Option<(background::BackgroundParams, Vec<bool>, f32)>,
    pub fill: f32,
}
impl Prepared {
    pub fn apply(&self, i: usize, raw: &Gray) -> Gray {
        if let Some(j) = &self.joint {
            return j.apply(i, raw);
        }
        let mut t = Gray {
            w: raw.w,
            h: raw.h,
            px: raw
                .px
                .iter()
                .map(|&v| v * self.gain[i] + self.offset[i])
                .collect(),
        };
        if let Some((p, skip, median)) = &self.legacy
            && !skip[i]
        {
            if p.edge_steps {
                background::detect_edge_steps(&t, p.threshold.unwrap(), p.dilate).apply(&mut t);
            }
            if let Some(s) = background::surface(&t, p.threshold.unwrap(), p) {
                for (v, b) in t.px.iter_mut().zip(s.px) {
                    *v = *v - b + median;
                }
            }
        }
        t
    }
}
pub(crate) fn prepare(
    pl: &Placement,
    load: &impl Fn(usize) -> Result<Gray>,
    p: &PhotometricParams,
    saved: Option<&PhotometricResult>,
    ctl: &Ctl,
) -> Result<Prepared> {
    let n = pl.tiles.len();
    let observations = samples(pl, load, 3)?;
    let (ag, ao) = match p.model {
        GainModel::Affine => fit_affine(n, &observations, p.regularisation),
        GainModel::None => (vec![1.; n], vec![0.; n]),
        GainModel::Gain => {
            if let Some(s) = saved.filter(|s| s.auto_gain.len() == n) {
                (s.auto_gain.clone(), vec![0.; n])
            } else {
                gain_loaded(pl, load)?
            }
        }
    };
    drop(observations);
    let mut g = ag.clone();
    let mut o = ao.clone();
    for v in &p.overrides {
        if v.index < n {
            if let Some(x) = v.gain {
                g[v.index] = x;
            }
            if let Some(x) = v.offset {
                o[v.index] = x;
            }
        }
    }
    let skip: Vec<_> = (0..n)
        .map(|i| p.overrides.iter().any(|v| v.index == i && v.no_background))
        .collect();
    let mut result = Prepared {
        gain: g.clone(),
        offset: o.clone(),
        joint: None,
        legacy: None,
        fill: 0.,
    };
    if p.background && p.background_params.method == background::LevelMethod::Joint {
        let fixed = joint::Fixed {
            gain: if p.model == GainModel::Affine {
                None
            } else {
                Some(&ag)
            },
            skip: &skip,
            regularisation: p.regularisation,
        };
        let overrides: Vec<_> = p
            .overrides
            .iter()
            .filter(|v| v.index < n)
            .map(|v| (v.index, v.gain, v.offset))
            .collect();
        let j = joint::prepare_loaded(pl, load, &p.background_params, &fixed, &overrides, ctl)?;
        let [joint, affine] = comparison(
            pl,
            load,
            &|i| Ok(j.apply(i, &load(i)?)),
            &j.sol.raw_stats,
            &g,
            &o,
        )?;
        if !joint_is_worse(joint, affine) {
            result.fill = j.sol.level;
            result.joint = Some(j);
        }
    } else if p.background {
        let mut bp = p.background_params.clone();
        if bp.threshold.is_none() {
            let mut hist = [0u64; 256];
            for i in 0..n {
                ctl.check()?;
                let t = result.apply(i, &load(i)?);
                for &v in t.px.iter().step_by(7) {
                    hist[(v.clamp(0., 1.) * 255.) as usize] += 1;
                }
            }
            bp.threshold = Some(background::otsu_hist(&hist));
        }
        let median = select_stream(|visit| {
            for (i, &skipped) in skip.iter().enumerate().take(n) {
                ctl.check()?;
                if skipped {
                    continue;
                }
                let mut t = result.apply(i, &load(i)?);
                if bp.edge_steps {
                    background::detect_edge_steps(&t, bp.threshold.unwrap(), bp.dilate)
                        .apply(&mut t);
                }
                if let Some(s) = background::surface(&t, bp.threshold.unwrap(), &bp) {
                    for &v in s.px.iter().step_by(31) {
                        visit(v);
                    }
                }
            }
            Ok(())
        })?;
        result.fill = median;
        result.legacy = Some((bp, skip, median));
    }
    if !p.background {
        result.fill = select_stream(|visit| {
            for i in 0..n {
                ctl.check()?;
                let t = result.apply(i, &load(i)?);
                for &v in t.px.iter().step_by(97) {
                    visit(v);
                }
            }
            Ok(())
        })?;
    }
    Ok(result)
}
/// Exact f32 median by four radix passes; no sample vector grows with tile count.
fn select_stream(mut scan: impl FnMut(&mut dyn FnMut(f32)) -> Result<()>) -> Result<f32> {
    let mut prefix = 0u32;
    let mut mask = 0u32;
    let mut rank = None;
    for shift in [24, 16, 8, 0] {
        let mut bins = [0u64; 256];
        scan(&mut |v| {
            let bits = v.to_bits();
            let key = if bits >> 31 != 0 {
                !bits
            } else {
                bits ^ 0x80000000
            };
            if key & mask == prefix {
                bins[((key >> shift) & 255) as usize] += 1;
            }
        })?;
        let total = bins.iter().sum::<u64>();
        if total == 0 {
            return Ok(0.);
        }
        let mut r = rank.unwrap_or(total / 2);
        let mut bucket = 0;
        while r >= bins[bucket] {
            r -= bins[bucket];
            bucket += 1;
        }
        rank = Some(r);
        prefix |= (bucket as u32) << shift;
        mask |= 255 << shift;
    }
    Ok(f32::from_bits(if prefix >> 31 != 0 {
        prefix ^ 0x80000000
    } else {
        !prefix
    }))
}
fn comparison(
    pl: &Placement,
    raw: &impl Fn(usize) -> crate::Result<Gray>,
    corrected: &impl Fn(usize) -> crate::Result<Gray>,
    stats: &[joint::TileStats],
    gain: &[f32],
    offset: &[f32],
) -> crate::Result<[(f32, f32); 2]> {
    let n = pl.tiles.len();
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
    let levels: Vec<Option<[f32; 2]>> = (0..n)
        .map(|i| {
            let image = raw(i)?;
            let joint = corrected(i)?;
            let (t, p, st) = (&image, &pl.tiles[i], &stats[i]);
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
                        let covered = wx >= 2 * q.x as i128
                            && wx < 2 * (q.x as i128 + q.w as i128)
                            && wy >= 2 * q.y as i128
                            && wy < 2 * (q.y as i128 + q.h as i128);
                        covered && (distance(k), k) < (own, i)
                    }) {
                        continue;
                    }
                    region += 1;
                    let k = y * t.w + x;
                    if (t.px[k] - st.mode).abs() <= st.band {
                        jv.push(joint.px[k]);
                        av.push(t.px[k] * gain[i] + offset[i]);
                    }
                }
            }
            if jv.is_empty() || jv.len() * 100 < region * 3 {
                return Ok(None);
            }
            let k = jv.len() / 2;
            jv.select_nth_unstable_by(k, f32::total_cmp);
            av.select_nth_unstable_by(k, f32::total_cmp);
            Ok(Some([jv[k], av[k]]))
        })
        .collect::<crate::Result<_>>()?;
    Ok(std::array::from_fn(|r| {
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
    }))
}

fn samples(
    pl: &Placement,
    load: &impl Fn(usize) -> crate::Result<Gray>,
    step: usize,
) -> crate::Result<Vec<(usize, usize, f32, f32)>> {
    let mut out = Vec::new();
    for p in &pl.pairs {
        let ga = load(p.a)?;
        let gb = load(p.b)?;
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
                let va = ga.px[(y - ta.y) as usize * ta.w + (x - ta.x) as usize];
                let vb = gb.px[(y - tb.y) as usize * tb.w + (x - tb.x) as usize];
                if (0.002..0.998).contains(&va) && (0.002..0.998).contains(&vb) {
                    out.push((p.a, p.b, va, vb));
                }
                x += step as i64;
            }
            y += step as i64;
        }
    }
    Ok(out)
}

// The gain-only log-space fit uses the same 40,000-cell sampling lattice and
// row order as photocraft (MIT OR Apache-2.0; LICENSE-photocraft-MIT).
fn gain_loaded(
    pl: &Placement,
    load: &impl Fn(usize) -> Result<Gray>,
) -> Result<(Vec<f32>, Vec<f32>)> {
    let n = pl.tiles.len();
    let x0 = pl.tiles.iter().map(|t| t.x as usize).min().unwrap_or(0);
    let y0 = pl.tiles.iter().map(|t| t.y as usize).min().unwrap_or(0);
    let x1 = pl
        .tiles
        .iter()
        .map(|t| t.x as usize + t.w)
        .max()
        .unwrap_or(x0);
    let y1 = pl
        .tiles
        .iter()
        .map(|t| t.y as usize + t.h)
        .max()
        .unwrap_or(y0);
    let step = ((((x1 - x0) as i64 * (y1 - y0) as i64 / 40000).max(1) as f64)
        .sqrt()
        .ceil() as usize)
        .max(1);
    let mut cells = std::collections::BTreeMap::<(usize, usize), Vec<(usize, f32)>>::new();
    for (i, p) in pl.tiles.iter().enumerate() {
        let t = load(i)?;
        for y in (y0 + (p.y as usize - y0).div_ceil(step) * step..p.y as usize + t.h).step_by(step)
        {
            for x in
                (x0 + (p.x as usize - x0).div_ceil(step) * step..p.x as usize + t.w).step_by(step)
            {
                cells
                    .entry((y, x))
                    .or_default()
                    .push((i, t.px[(y - p.y as usize) * t.w + x - p.x as usize]));
            }
        }
    }
    let mut a = vec![0f64; n * n];
    let mut b = vec![0f64; n];
    let mut count = 0;
    for v in cells.values() {
        for p in 0..v.len() {
            for q in p + 1..v.len() {
                let (i, x) = v[p];
                let (j, y) = v[q];
                if !(0.02..=0.98).contains(&x)
                    || !(0.02..=0.98).contains(&y)
                    || (x / y).ln().abs() > 1.
                {
                    continue;
                }
                count += 1;
                let rhs = (x as f64).ln() - (y as f64).ln();
                let w = 20f64 * 20.;
                a[i * n + i] += w;
                a[i * n + j] -= w;
                a[j * n + i] -= w;
                a[j * n + j] += w;
                b[i] += w * rhs;
                b[j] -= w * rhs;
            }
        }
    }
    if count < 20 {
        return Ok((vec![1.; n], vec![0.; n]));
    }
    for i in 0..n {
        a[i * n + i] += (1f64 / 0.15).powi(2);
    }
    let g = solve_dense(a, b, n).unwrap_or(vec![0.; n]);
    Ok((
        g.into_iter()
            .map(|v| 1. / (v.exp().clamp(0.2, 5.) as f32))
            .collect(),
        vec![0.; n],
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reloadable_correction_equals_canvas_for_all_models_and_backgrounds() {
        let w = 96;
        let h = 96;
        let mut pl = Placement {
            tiles: vec![],
            pairs: vec![],
            width: 168,
            height: 168,
            rms_residual: 0.,
            max_residual: 0.,
        };
        let mut images = vec![];
        for i in 0..4 {
            let (x, y) = ((i % 2) * 72, (i / 2) * 72);
            pl.tiles.push(crate::mosaic::TilePlacement {
                index: i,
                x: x as i64,
                y: y as i64,
                w,
                h,
                prior: [x as f64, y as f64],
                pos: [x as f64, y as f64],
                drift: [0.; 2],
                failed: false,
            });
            images.push(Gray {
                w,
                h,
                px: (0..w * h)
                    .map(|j| {
                        let (xx, yy) = (j % w + x, j / w + y);
                        0.25 + ((xx * 13 + yy * 31) % 71) as f32 / 900.
                    })
                    .collect(),
            });
        }
        // Explicit neighbour edges exercise the affine prefit too.
        for (a, b) in [(0, 1), (0, 2), (1, 3), (2, 3)] {
            pl.pairs.push(crate::mosaic::PairResult {
                a,
                b,
                dir: "h".into(),
                prior: [0.; 2],
                measured: None,
                psr: 0.,
                used: false,
                residual: 0.,
            });
        }
        for model in [GainModel::Affine, GainModel::None, GainModel::Gain] {
            for background in [false, true] {
                for method in [
                    background::LevelMethod::Joint,
                    background::LevelMethod::Tile,
                ] {
                    let mut params = PhotometricParams {
                        model,
                        background,
                        ..Default::default()
                    };
                    params.background_params.method = method;
                    params.background_params.cell = 32;
                    params.overrides.push(TileOverride {
                        index: 1,
                        gain: Some(1.02),
                        offset: Some(0.01),
                        no_background: false,
                    });
                    let (expected, _) = correct_ctl(&pl, &images, &params, &Ctl::NONE).unwrap();
                    let prepared =
                        prepare(&pl, &|i| Ok(images[i].clone()), &params, None, &Ctl::NONE)
                            .unwrap();
                    for (i, g) in images.iter().enumerate() {
                        let actual = prepared.apply(i, g);
                        for (j, (a, b)) in actual.px.iter().zip(&expected[i].px).enumerate() {
                            assert!(
                                (a - b).abs() < 1e-6,
                                "{model:?} {background} {method:?} tile{i} pixel{j}: {a} != {b}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn radix_median_matches_sort() {
        let v = [-0.4, 0., 0.2, 0.7, 0.8, 0.02, 0.1];
        let mut sorted = v;
        sorted.sort_by(f32::total_cmp);
        let m = select_stream(|visit| {
            for x in v {
                visit(x);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(m, sorted[v.len() / 2]);
    }
    #[test]
    fn gain_sampling_uses_active_bounds_and_their_grid_origin() {
        let mut pl = Placement {
            tiles: vec![],
            pairs: vec![],
            width: 50000,
            height: 100000,
            rms_residual: 0.,
            max_residual: 0.,
        };
        let mut images = vec![];
        for i in 0..2 {
            let x = 101 + i * 225;
            let y = 203;
            let (w, h) = (300, 180);
            pl.tiles.push(crate::mosaic::TilePlacement {
                index: i,
                x: x as i64,
                y: y as i64,
                w,
                h,
                prior: [x as f64, y as f64],
                pos: [x as f64, y as f64],
                drift: [0.; 2],
                failed: false,
            });
            images.push(Gray {
                w,
                h,
                px: (0..w * h)
                    .map(|j| {
                        (0.25 + (((x + j % w) * 13 + (y + j / w) * 31) % 71) as f32 / 900.)
                            * if i == 0 { 1. } else { 1.4 }
                    })
                    .collect(),
            });
        }
        let p = PhotometricParams {
            model: GainModel::Gain,
            background: false,
            ..Default::default()
        };
        let (expected, _) = correct_ctl(&pl, &images, &p, &Ctl::NONE).unwrap();
        let prepared = prepare(&pl, &|i| Ok(images[i].clone()), &p, None, &Ctl::NONE).unwrap();
        for (i, g) in images.iter().enumerate() {
            assert_eq!(prepared.apply(i, g).px, expected[i].px);
        }
    }
}
