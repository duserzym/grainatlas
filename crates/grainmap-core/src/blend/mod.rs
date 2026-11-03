//! Seams and blending (plan section 6.4): seam labels on the intensity
//! difference with `photocraft_algo::panorama::seam_labels` (computed on a
//! reduced grid and upsampled), then multiband blending with
//! `photocraft_algo::panorama::multiband`. A fast mode feathers linearly.
//! Holes are filled with the median background, never inpainted.

use photocraft_algo::panorama::{RoiImage, blend_levels, multiband, seam_labels};
use serde::{Deserialize, Serialize};

use crate::Ctl;
use crate::image::Gray;
use crate::mosaic::Placement;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BlendMode {
    #[default]
    Multiband,
    Feather,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BlendParams {
    pub mode: BlendMode,
    /// Seam grid reduction (the seam search runs on a grid this many times coarser).
    pub seam_factor: usize,
    /// Pyramid levels for multiband (0 = automatic via `blend_levels`).
    pub levels: usize,
}

impl Default for BlendParams {
    fn default() -> Self {
        Self { mode: BlendMode::Multiband, seam_factor: 8, levels: 0 }
    }
}

/// Blends corrected tiles at their integer placements. Returns the mosaic and its coverage
/// (0..1). `fill` is written where no tile covers the mosaic.
/// Takes the tiles by value so their pixels move into the blend instead of being copied.
pub fn blend(pl: &Placement, tiles: Vec<Gray>, p: &BlendParams, fill: f32, ctl: &Ctl) -> crate::Result<(Gray, Vec<f32>)> {
    let (w, h) = (pl.width, pl.height);
    ctl.report("seams", 0.0);
    let weights: Vec<Vec<f32>> = match p.mode {
        BlendMode::Multiband => seam_weights(pl, &tiles, p.seam_factor.max(1)),
        BlendMode::Feather => feather_weights(&tiles),
    };
    ctl.check()?;
    ctl.report("blend", 0.2);
    let (px, cov) = match p.mode {
        BlendMode::Multiband => {
            let imgs: Vec<RoiImage> = pl
                .tiles
                .iter()
                .zip(tiles)
                .map(|(t, g)| RoiImage { x0: t.x as i32, y0: t.y as i32, w: g.w, h: g.h, ch: 1, alpha: vec![1.0; g.w * g.h], px: g.px })
                .collect();
            let levels = if p.levels == 0 { blend_levels(w, h) } else { p.levels };
            multiband(w, h, &imgs, &weights, levels)
        }
        BlendMode::Feather => feather(w, h, pl, &tiles, &weights),
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
    Ok((out, cov))
}

/// Binary weights from seam labels found on a grid reduced by `k`.
fn seam_weights(pl: &Placement, tiles: &[Gray], k: usize) -> Vec<Vec<f32>> {
    let (sw, sh) = (pl.width.div_ceil(k).max(1), pl.height.div_ceil(k).max(1));
    let n = tiles.len();
    let mut luma = vec![vec![0.0f32; sw * sh]; n];
    let mut cover = vec![vec![false; sw * sh]; n];
    for (i, (t, g)) in pl.tiles.iter().zip(tiles).enumerate() {
        let small = g.downsample(k);
        // A small cell is covered if its whole k x k block lies inside the tile.
        let cx0 = (t.x as usize).div_ceil(k);
        let cy0 = (t.y as usize).div_ceil(k);
        let cx1 = (t.x as usize + g.w) / k;
        let cy1 = (t.y as usize + g.h) / k;
        for cy in cy0..cy1.min(sh) {
            for cx in cx0..cx1.min(sw) {
                let lx = ((cx * k) as i64 - t.x).max(0) as usize / k;
                let ly = ((cy * k) as i64 - t.y).max(0) as usize / k;
                let lx = lx.min(small.w - 1);
                let ly = ly.min(small.h - 1);
                luma[i][cy * sw + cx] = small.px[ly * small.w + lx];
                cover[i][cy * sw + cx] = true;
            }
        }
    }
    let order: Vec<usize> = (0..n).collect();
    let labels = seam_labels(sw, sh, &luma, &cover, &order);
    drop(luma);
    drop(cover);
    crate::util::par_map(n, |i| {
        let (t, g) = (&pl.tiles[i], &tiles[i]);
        let mut wv = vec![0.0f32; g.w * g.h];
        for y in 0..g.h {
            let my = (t.y as usize + y) / k;
            for x in 0..g.w {
                let mx = (t.x as usize + x) / k;
                let l = labels[my.min(sh - 1) * sw + mx.min(sw - 1)];
                // Cells on the tile border may be unlabelled (partly covered): the tile keeps them
                // unless another tile owns them.
                wv[y * g.w + x] = if l == i as i32 || l < 0 { 1.0 } else { 0.0 };
            }
        }
        wv
    })
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

fn feather(w: usize, h: usize, pl: &Placement, tiles: &[Gray], weights: &[Vec<f32>]) -> (Vec<f32>, Vec<f32>) {
    let mut acc = vec![0.0f32; w * h];
    let mut ws = vec![0.0f32; w * h];
    for ((t, g), wv) in pl.tiles.iter().zip(tiles).zip(weights) {
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
    let cov: Vec<f32> = ws.iter().map(|&k| if k > 0.0 { 1.0 } else { 0.0 }).collect();
    for (a, k) in acc.iter_mut().zip(&ws) {
        if *k > 0.0 {
            *a /= *k;
        }
    }
    (acc, cov)
}
