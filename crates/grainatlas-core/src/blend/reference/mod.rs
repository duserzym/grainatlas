//! Frozen pre-export reference from origin/main bfa87ee9c621.
//! Test-only; do not share helpers with the production blend.
//! Seams and blending (plan section 6.4): seam labels on the intensity
//! difference with `photocraft_algo::panorama::seam_labels` (computed on a
//! reduced grid and upsampled), then multiband blending with
//! `photocraft_algo::panorama::multiband`. A fast mode feathers linearly.
//! Holes are filled with the median background, never inpainted.

use photocraft_algo::panorama::{RoiImage, blend_levels};

mod bounded;
mod seams;

use crate::Ctl;
use crate::image::Gray;
use crate::mosaic::Placement;

use super::{BlendMode, BlendParams};

/// Blends corrected tiles at their integer placements. Returns the mosaic and its coverage
/// (0..1). `fill` is written where no tile covers the mosaic.
/// Takes the tiles by value so their pixels move into the blend instead of being copied.
pub fn blend(
    pl: &Placement,
    tiles: Vec<Gray>,
    p: &BlendParams,
    fill: f32,
    ctl: &Ctl,
) -> crate::Result<(Gray, Vec<f32>)> {
    ctl.check()?;
    if pl.tiles.iter().any(|t| t.failed) {
        let (active, _) = pl.active();
        let tiles = tiles.into_iter().zip(&pl.tiles).filter(|(_, t)| !t.failed).map(|(g, _)| g).collect();
        if active.tiles.is_empty() {
            return Ok((Gray::filled(pl.width, pl.height, fill), vec![0.0; pl.width * pl.height]));
        }
        return blend(&active, tiles, p, fill, ctl);
    }
    let (w, h) = (pl.width, pl.height);
    ctl.report("seams", 0.0);
    let (px, cov) = match p.mode {
        BlendMode::Multiband => {
            let k = p.seam_factor.max(1);
            let (labels, sw, sh) = seam_grid(pl, &tiles, k, ctl)?;
            ctl.check()?;
            ctl.report("blend", 0.2);
            let weights = |i: usize, j: usize| {
                let t = &pl.tiles[i];
                let mx = (t.x as usize + j % t.w) / k;
                let my = (t.y as usize + j / t.w) / k;
                let l = labels[my.min(sh - 1) * sw + mx.min(sw - 1)];
                if l == i as i32 || l < 0 { 1.0 } else { 0.0 }
            };
            let imgs: Vec<RoiImage> = pl
                .tiles
                .iter()
                .zip(tiles)
                .map(|(t, g)| RoiImage {
                    x0: t.x as i32,
                    y0: t.y as i32,
                    w: g.w,
                    h: g.h,
                    ch: 1,
                    alpha: Vec::new(),
                    px: g.px,
                })
                .collect();
            let levels = if p.levels == 0 {
                blend_levels(w, h)
            } else {
                p.levels
            };
            bounded::multiband(w, h, imgs, weights, levels, ctl)?
        }
        BlendMode::Feather => {
            let weights = feather_weights(&tiles);
            ctl.report("seams", 1.0);
            ctl.report("blend", 0.2);
            feather(w, h, pl, &tiles, &weights, ctl)?
        }
    };
    ctl.report("blend", 1.0);
    let mut out = Gray { w, h, px };
    for (v, c) in out.px.iter_mut().zip(&cov) {
        if *c < 0.5 {
            *v = fill;
        } else {
            *v = v.clamp(0.0, 1.0);
        }
    }
    ctl.check()?;
    Ok((out, cov))
}

/// Binary weights from seam labels found on a grid reduced by `k`.
fn seam_grid(
    pl: &Placement,
    tiles: &[Gray],
    k: usize,
    ctl: &Ctl,
) -> crate::Result<(Vec<i32>, usize, usize)> {
    let (sw, sh) = (pl.width.div_ceil(k).max(1), pl.height.div_ceil(k).max(1));
    let n = tiles.len();
    let mut luma = Vec::with_capacity(n);
    for (t, g) in pl.tiles.iter().zip(tiles) {
        ctl.check()?;
        let small = g.downsample(k);
        // A small cell is covered if its whole k x k block lies inside the tile.
        let cx0 = (t.x as usize).div_ceil(k);
        let cy0 = (t.y as usize).div_ceil(k);
        let cx1 = (t.x as usize + g.w) / k;
        let cy1 = (t.y as usize + g.h) / k;
        let (gw, gh) = (
            cx1.min(sw).saturating_sub(cx0),
            cy1.min(sh).saturating_sub(cy0),
        );
        let mut grid = seams::Grid {
            x: cx0,
            y: cy0,
            w: gw,
            h: gh,
            stride: sw,
            px: vec![0.0; gw * gh],
        };
        for cy in cy0..cy0 + gh {
            for cx in cx0..cx0 + gw {
                let lx = (((cx * k) as i64 - t.x).max(0) as usize / k).min(small.w - 1);
                let ly = (((cy * k) as i64 - t.y).max(0) as usize / k).min(small.h - 1);
                grid.px[(cy - cy0) * gw + cx - cx0] = small.px[ly * small.w + lx];
            }
        }
        luma.push(grid);
    }
    let order: Vec<usize> = (0..n).collect();
    let labels = seams::seam_labels(sw, sh, &luma, &order, ctl)?;
    drop(luma);
    Ok((labels, sw, sh))
}

/// Linear feather: weight falls to zero at the tile border.
fn feather_weights(tiles: &[Gray]) -> Vec<Vec<f32>> {
    tiles
        .iter()
        .map(|g| {
            let mut wv = vec![0.0f32; g.w * g.h];
            for y in 0..g.h {
                let dy = (y.min(g.h - 1 - y) + 1) as f32 / (g.h as f32 / 2.0);
                for x in 0..g.w {
                    let dx = (x.min(g.w - 1 - x) + 1) as f32 / (g.w as f32 / 2.0);
                    wv[y * g.w + x] = dx.min(1.0) * dy.min(1.0);
                }
            }
            wv
        })
        .collect()
}

fn feather(
    w: usize,
    h: usize,
    pl: &Placement,
    tiles: &[Gray],
    weights: &[Vec<f32>],
    ctl: &Ctl,
) -> crate::Result<(Vec<f32>, Vec<f32>)> {
    let mut acc = vec![0.0f32; w * h];
    let mut ws = vec![0.0f32; w * h];
    for ((t, g), wv) in pl.tiles.iter().zip(tiles).zip(weights) {
        ctl.check()?;
        for y in 0..g.h {
            let my = t.y as usize + y;
            if my >= h {
                break;
            }
            for x in 0..g.w {
                let mx = t.x as usize + x;
                if mx >= w {
                    break;
                }
                let k = wv[y * g.w + x];
                acc[my * w + mx] += k * g.px[y * g.w + x];
                ws[my * w + mx] += k;
            }
        }
    }
    let cov: Vec<f32> = ws
        .iter()
        .map(|&k| if k > 0.0 { 1.0 } else { 0.0 })
        .collect();
    for (a, k) in acc.iter_mut().zip(&ws) {
        if *k > 0.0 {
            *a /= *k;
        }
    }
    Ok((acc, cov))
}

