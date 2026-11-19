//! Reloadable source tiles and cropped global pyramid accumulators. The grids
//! keep their full-mosaic phase and boundary conditions; only required rows exist.
use super::*;
use crate::Result;
pub(crate) struct Prepared {
    labels: Labels,
    sw: usize,
    sh: usize,
    k: usize,
    levels: usize,
    mode: BlendMode,
}
impl Prepared {
    pub fn new(
        pl: &Placement,
        load: &impl Fn(usize) -> Result<Gray>,
        p: &BlendParams,
        ctl: &Ctl,
    ) -> Result<Self> {
        let k = p.seam_factor.max(1);
        let sw = pl.width.div_ceil(k).max(1);
        let sh = pl.height.div_ceil(k).max(1);
        let mut labels = Labels::default();
        if p.mode == BlendMode::Multiband {
            for i in 0..pl.tiles.len() {
                ctl.check()?;
                let grid = coarse(pl, i, &load(i)?, k, sw, sh);
                let (gw, gh) = (grid.w, grid.h);
                if gw == 0 || gh == 0 {
                    continue;
                }
                let mut local = vec![-1i32; gw * gh];
                for yy in 0..gh {
                    for x in 0..gw {
                        local[yy * gw + x] = labels.at(grid.x + x, grid.y + yy);
                    }
                }
                let used: std::collections::BTreeSet<_> =
                    local.iter().copied().filter(|v| *v >= 0).collect();
                let mut old = vec![0.; gw * gh];
                for prior in used {
                    let g = coarse(pl, prior as usize, &load(prior as usize)?, k, sw, sh);
                    for yy in 0..gh {
                        for x in 0..gw {
                            let j = yy * gw + x;
                            if local[j] == prior {
                                old[j] = g[(grid.y + yy) * sw + grid.x + x];
                            }
                        }
                    }
                }
                let centre = labels.centre();
                let cov = seams::Grid {
                    x: 0,
                    y: 0,
                    w: gw,
                    h: gh,
                    stride: gw,
                    px: Vec::new(),
                };
                seams::step(
                    gw,
                    gh,
                    &cov,
                    i,
                    &mut local,
                    |_, j| 0.002 + (grid.px[j] - old[j]).abs(),
                    centre,
                    [grid.x, grid.y],
                    ctl,
                )?;
                for yy in 0..gh {
                    labels.replace(grid.y + yy, grid.x, &local[yy * gw..(yy + 1) * gw]);
                }
            }
        }
        let levels = if p.levels == 0 {
            blend_levels(pl.width, pl.height)
        } else {
            p.levels
        }
        .clamp(1, 16);
        Ok(Self {
            labels,
            sw,
            sh,
            k,
            levels,
            mode: p.mode,
        })
    }
    pub fn strip(
        &self,
        pl: &Placement,
        load: &impl Fn(usize) -> Result<Gray>,
        y: usize,
        rows: usize,
        fill: f32,
        ctl: &Ctl,
    ) -> Result<(Gray, Vec<f32>, usize)> {
        let (w, h) = (pl.width, pl.height);
        let rows = rows.min(h - y);
        let mut cov = vec![0f32; w * rows];
        for t in &pl.tiles {
            for yy in y.max(t.y as usize)..(y + rows).min(t.y as usize + t.h) {
                for x in t.x as usize..(t.x as usize + t.w).min(w) {
                    cov[(yy - y) * w + x] = 1.;
                }
            }
        }
        if self.mode == BlendMode::Feather {
            let mut out = Gray::new(w, rows);
            let mut ws = vec![0f32; w * rows];
            let mut peak = (out.px.len() + ws.len() + cov.len()) * 4;
            for (i, t) in pl.tiles.iter().enumerate() {
                if t.y as usize >= y + rows || t.y as usize + t.h <= y {
                    continue;
                }
                ctl.check()?;
                let g = load(i)?;
                peak = peak.max((out.px.len() + ws.len() + cov.len() + g.px.len()) * 4);
                for yy in y.max(t.y as usize)..(y + rows).min(t.y as usize + t.h) {
                    let ly = yy - t.y as usize;
                    let dy = (ly.min(t.h - 1 - ly) + 1) as f32 / (t.h as f32 / 2.);
                    for x in 0..t.w {
                        let mx = t.x as usize + x;
                        if mx >= w {
                            break;
                        }
                        let dx = (x.min(t.w - 1 - x) + 1) as f32 / (t.w as f32 / 2.);
                        let k = dx.min(1.) * dy.min(1.);
                        let j = (yy - y) * w + mx;
                        out.px[j] += k * g.px[ly * t.w + x];
                        ws[j] += k;
                    }
                }
            }
            for ((v, k), c) in out.px.iter_mut().zip(ws).zip(&cov) {
                *v = if *c < 0.5 {
                    fill
                } else {
                    (*v / k).clamp(0., 1.)
                };
            }
            return Ok((out, cov, peak));
        }
        let levels = self.levels;
        let mut sizes = vec![(w, h)];
        let mut ranges = vec![(y, y + rows)];
        for l in 1..levels {
            let (pw, ph) = sizes[l - 1];
            let (nw, nh) = (pw.div_ceil(2), ph.div_ceil(2));
            sizes.push((nw, nh));
            let (lo, hi) = ranges[l - 1];
            let at = |v: usize| ((v as f32 + 0.5) / 2. - 0.5).clamp(0., (nh - 1) as f32) as usize;
            ranges.push((at(lo), (at(hi - 1) + 2).min(nh)));
        }
        let mut acc: Vec<Vec<f32>> = sizes
            .iter()
            .zip(&ranges)
            .map(|(&(w, _), &(a, b))| vec![0.; w * (b - a)])
            .collect();
        let mut ws = acc.clone();
        let base_bytes = (acc.iter().map(Vec::len).sum::<usize>() * 2 + cov.len()) * 4;
        let mut peak = base_bytes;
        let align = 1usize << (levels - 1);
        let margin = align * 4;
        for (i, t) in pl.tiles.iter().enumerate() {
            let gy0 = (t.y as usize).saturating_sub(margin) / align * align;
            let gy1 = (t.y as usize + t.h + margin).min(h);
            if !(0..levels).any(|l| {
                let lo = gy0 >> l;
                let hi = lo + (gy1 - gy0).div_ceil(1 << l);
                lo < ranges[l].1 && hi > ranges[l].0
            }) {
                continue;
            }
            ctl.check()?;
            let g = load(i)?;
            let im = RoiImage {
                x0: t.x as i32,
                y0: t.y as i32,
                w: g.w,
                h: g.h,
                ch: 1,
                px: g.px,
                alpha: Vec::new(),
            };
            let weights = |j: usize| {
                let mx = (t.x as usize + j % t.w) / self.k;
                let my = (t.y as usize + j / t.w) / self.k;
                let label = self.labels.at(mx.min(self.sw - 1), my.min(self.sh - 1));
                if label == i as i32 || label < 0 {
                    1.
                } else {
                    0.
                }
            };
            let b = bounded::tile_band(&im, weights, w, h, levels, ctl)?;
            // tile_band's simultaneous scratch is bounded by 16 grid planes.
            let gw = (t.w + margin * 2 + align).min(w);
            let gh = (t.h + margin * 2 + align).min(h);
            peak = peak.max(base_bytes + gw * gh * 4 * 16 + im.px.len() * 4);
            for l in 0..levels {
                let pw = sizes[l].0;
                let (lo, hi) = ranges[l];
                let ox = (b.0 as usize) >> l;
                let oy = (b.1 as usize) >> l;
                let (bw, bh, px) = &b.2[l];
                for yy in lo.max(oy)..hi.min(oy + bh) {
                    for x in 0..*bw {
                        let xx = ox + x;
                        if xx >= pw {
                            break;
                        }
                        let j = (yy - oy) * bw + x;
                        let k = b.3[l][j];
                        if k <= 0. {
                            continue;
                        }
                        let dst = (yy - lo) * pw + xx;
                        ws[l][dst] += k;
                        acc[l][dst] += k * px[j];
                    }
                }
            }
        }
        for l in 0..levels {
            for (v, k) in acc[l].iter_mut().zip(&ws[l]) {
                *v = if *k > 1e-8 { *v / *k } else { 0. };
            }
        }
        drop(ws);
        for l in (0..levels - 1).rev() {
            let (sw, sh) = sizes[l + 1];
            let pw = sizes[l].0;
            let (lo, hi) = ranges[l];
            let sy0 = ranges[l + 1].0;
            let (coarse, fine) = {
                let (a, b) = acc.split_at_mut(l + 1);
                (&b[0], &mut a[l])
            };
            for yy in lo..hi {
                let fy = ((yy as f32 + 0.5) / 2. - 0.5).clamp(0., (sh - 1) as f32);
                let y0 = fy as usize;
                let ty = fy - y0 as f32;
                let y1 = (y0 + 1).min(sh - 1);
                for x in 0..pw {
                    let fx = ((x as f32 + 0.5) / 2. - 0.5).clamp(0., (sw - 1) as f32);
                    let x0 = fx as usize;
                    let tx = fx - x0 as f32;
                    let x1 = (x0 + 1).min(sw - 1);
                    let a = coarse[(y0 - sy0) * sw + x0];
                    let b = coarse[(y0 - sy0) * sw + x1];
                    let c = coarse[(y1 - sy0) * sw + x0];
                    let d = coarse[(y1 - sy0) * sw + x1];
                    let top = a + (b - a) * tx;
                    let bot = c + (d - c) * tx;
                    fine[(yy - lo) * pw + x] += top + (bot - top) * ty;
                }
            }
        }
        let mut px = std::mem::take(&mut acc[0]);
        for (v, c) in px.iter_mut().zip(&cov) {
            *v = if *c < 0.5 { fill } else { v.clamp(0., 1.) };
        }
        Ok((Gray { w, h: rows, px }, cov, peak))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(w: usize, h: usize) -> (Placement, Vec<Gray>) {
        let positions = [
            (0, 0),
            (w * 3 / 4, 0),
            (0, h * 3 / 4),
            (w * 3 / 4, h * 3 / 4),
        ];
        let tiles = positions
            .iter()
            .enumerate()
            .map(|(index, &(x, y))| crate::mosaic::TilePlacement {
                index,
                x: x as i64,
                y: y as i64,
                w,
                h,
                prior: [x as f64, y as f64],
                pos: [x as f64, y as f64],
                drift: [0.; 2],
                failed: false,
            })
            .collect();
        let pl = Placement {
            tiles,
            pairs: vec![],
            width: w + w * 3 / 4,
            height: h + h * 3 / 4,
            rms_residual: 0.,
            max_residual: 0.,
        };
        let imgs = (0..4)
            .map(|i| Gray {
                w,
                h,
                px: (0..w * h)
                    .map(|j| ((j * 37 + i * 71) % 1001) as f32 / 1000.)
                    .collect(),
            })
            .collect();
        (pl, imgs)
    }
    #[test]
    fn strips_equal_canvas_blend_bit_for_bit() {
        let (pl, imgs) = fixture(91, 103);
        for mode in [BlendMode::Feather, BlendMode::Multiband] {
            for levels in [1, 3, 5] {
                let p = BlendParams {
                    mode,
                    levels,
                    seam_factor: 4,
                };
                let expected = blend(&pl, imgs.clone(), &p, 0.15, &Ctl::NONE).unwrap();
                let loader = |i: usize| Ok(imgs[i].clone());
                let prep = Prepared::new(&pl, &loader, &p, &Ctl::NONE).unwrap();
                for y in (0..pl.height).step_by(17) {
                    let (g, c, _) = prep.strip(&pl, &loader, y, 17, 0.15, &Ctl::NONE).unwrap();
                    assert_eq!(
                        g.px,
                        expected.0.px[y * pl.width..(y + g.h) * pl.width],
                        "{mode:?} level {levels} y{y}"
                    );
                    assert_eq!(c, expected.1[y * pl.width..(y + g.h) * pl.width]);
                }
            }
        }
    }
    #[test]
    fn sparse_seam_spans_match_canvas_for_contained_tiles_and_cost_ties() {
        let (mut pl, mut imgs) = fixture(91, 103);
        pl.tiles.truncate(3);
        imgs.truncate(3);
        pl.tiles[1].x = 61;
        pl.tiles[1].y = 9;
        pl.tiles[2].x = 23;
        pl.tiles[2].y = 17;
        pl.tiles[2].w = 40;
        pl.tiles[2].h = 50;
        imgs[2] = Gray::filled(40, 50, 0.4);
        for constant in [false, true] {
            if constant {
                for g in &mut imgs {
                    g.px.fill(0.4);
                }
            }
            let p = BlendParams {
                seam_factor: 4,
                ..Default::default()
            };
            let load = |i: usize| Ok(imgs[i].clone());
            let prep = Prepared::new(&pl, &load, &p, &Ctl::NONE).unwrap();
            let (expected, sw, sh) = seam_grid(&pl, &imgs, 4, &Ctl::NONE).unwrap();
            for y in 0..sh {
                for x in 0..sw {
                    assert_eq!(
                        prep.labels.at(x, y),
                        expected[y * sw + x],
                        "{constant} {x},{y}"
                    );
                }
            }
        }
    }
    #[test]
    fn tall_mosaic_pixel_working_set_is_one_strip_plus_one_tile() {
        let (mut pl, imgs) = fixture(91, 103);
        let p = BlendParams {
            mode: BlendMode::Multiband,
            levels: 3,
            seam_factor: 8,
        };
        let loader = |i: usize| Ok(imgs[i].clone());
        let mut peaks = vec![];
        for height in [10000, 100000] {
            pl.height = height;
            let prep = Prepared::new(&pl, &loader, &p, &Ctl::NONE).unwrap();
            let (g, _, peak) = prep.strip(&pl, &loader, 0, 64, 0., &Ctl::NONE).unwrap();
            assert_eq!(g.px.len(), pl.width * 64);
            assert!(peak < 2_000_000);
            peaks.push(peak);
        }
        assert_eq!(peaks[0], peaks[1]);
    }
}

fn coarse(pl: &Placement, i: usize, g: &Gray, k: usize, sw: usize, sh: usize) -> seams::Grid {
    let t = &pl.tiles[i];
    let small = g.downsample(k);
    let (cx0, cy0) = ((t.x as usize).div_ceil(k), (t.y as usize).div_ceil(k));
    let (gw, gh) = (
        ((t.x as usize + g.w) / k).min(sw).saturating_sub(cx0),
        ((t.y as usize + g.h) / k).min(sh).saturating_sub(cy0),
    );
    let mut grid = seams::Grid {
        x: cx0,
        y: cy0,
        w: gw,
        h: gh,
        stride: sw,
        px: vec![0.; gw * gh],
    };
    for cy in cy0..cy0 + gh {
        for cx in cx0..cx0 + gw {
            let lx = (((cx * k) as i64 - t.x).max(0) as usize / k).min(small.w - 1);
            let ly = (((cy * k) as i64 - t.y).max(0) as usize / k).min(small.h - 1);
            grid.px[(cy - cy0) * gw + cx - cx0] = small.px[ly * small.w + lx];
        }
    }
    grid
}
/// Run-length ownership rows. Holes and tile interiors are spans, never an
/// image-sized label plane. DP scratch is confined to one incoming coarse tile.
#[derive(Default)]
struct Labels {
    rows: std::collections::BTreeMap<usize, Vec<(usize, usize, i32)>>,
}
impl Labels {
    fn at(&self, x: usize, y: usize) -> i32 {
        self.rows
            .get(&y)
            .and_then(|r| {
                let i = r.partition_point(|v| v.0 <= x);
                i.checked_sub(1).and_then(|i| {
                    let v = r[i];
                    (x < v.1).then_some(v.2)
                })
            })
            .unwrap_or(-1)
    }
    fn centre(&self) -> Option<[f64; 2]> {
        let (mut sx, mut sy, mut n) = (0., 0., 0.);
        for (&y, r) in &self.rows {
            for &(a, b, _) in r {
                let k = (b - a) as f64;
                n += k;
                sx += (a as f64 + (b - 1) as f64) * 0.5 * k;
                sy += y as f64 * k;
            }
        }
        (n > 0.).then_some([sx / n, sy / n])
    }
    fn replace(&mut self, y: usize, x: usize, v: &[i32]) {
        let end = x + v.len();
        let row = self.rows.entry(y).or_default();
        let mut next = Vec::new();
        for &(a, b, id) in row.iter() {
            if a < x {
                next.push((a, b.min(x), id));
            }
            if b > end {
                next.push((a.max(end), b, id));
            }
        }
        let mut j = 0;
        while j < v.len() {
            let start = j;
            let id = v[j];
            j += 1;
            while j < v.len() && v[j] == id {
                j += 1;
            }
            if id >= 0 {
                next.push((x + start, x + j, id));
            }
        }
        next.sort_by_key(|v| v.0);
        row.clear();
        for span in next {
            if let Some(last) = row.last_mut()
                && last.1 == span.0
                && last.2 == span.2
            {
                last.1 = span.1;
            } else {
                row.push(span);
            }
        }
    }
}
