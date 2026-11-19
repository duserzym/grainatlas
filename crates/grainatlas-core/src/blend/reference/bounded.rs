//! Frozen pre-export reference from origin/main bfa87ee9c621.
//! Test-only; do not share helpers with the production blend.
//! See LICENSE-photocraft-MIT for the upstream copyright and permission notice.
//! Bounded-memory multiband for opaque grey tiles. Arithmetic and accumulation order
//! follow photocraft-algo 81bd973 panorama/blend.rs (MIT OR Apache-2.0), by its contributors.
//! Native band batches are bounded to 256 MiB (one tile on wasm); consumed tiles are
//! released, and collapse adds
//! the expansion directly into each destination band. Tests compare upstream bit for bit.
use crate::Ctl;
use crate::util::par_map;
use photocraft_algo::panorama::RoiImage;
fn par_rows(buf: &mut [f32], w: usize, ch: usize, f: impl Fn(usize, &mut [f32]) + Sync + Send) {
    crate::util::par_rows(buf, w * ch, f);
}
fn par_zip(a: &[f32], b: &[f32], f: impl Fn(f32, f32) -> f32 + Sync + Send) -> Vec<f32> {
    const CHUNK: usize = 1 << 14;
    let mut out = vec![0.0f32; a.len().min(b.len())];
    let fill = |o: &mut [f32], a: &[f32], b: &[f32]| {
        for ((o, &a), &b) in o.iter_mut().zip(a).zip(b) {
            *o = f(a, b);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.par_chunks_mut(CHUNK)
            .zip(a.par_chunks(CHUNK))
            .zip(b.par_chunks(CHUNK))
            .for_each(|((o, a), b)| fill(o, a, b));
    }
    #[cfg(target_arch = "wasm32")]
    out.chunks_mut(CHUNK)
        .zip(a.chunks(CHUNK))
        .zip(b.chunks(CHUNK))
        .for_each(|((o, a), b)| fill(o, a, b));
    out
}

/// `f(a[i])` for every `i`, in parallel on native targets (elementwise, so bit for bit serial).
fn par_map1(a: &[f32], f: impl Fn(f32) -> f32 + Sync + Send) -> Vec<f32> {
    par_zip(a, a, |x, _| f(x))
}

// `reduce`, `expand` and `push_pull` run one pyramid level for one image at a time, and the
// images themselves only fill part of a many-core machine, so each of them also runs its rows
// in parallel. Every output element is computed by the same expression, in the same order, as
// the one-thread loops they replace (rows never read each other's output), so the result is
// bit for bit unchanged.

pub(super) fn reduce(w: usize, h: usize, ch: usize, src: &[f32]) -> (usize, usize, Vec<f32>) {
    let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
    let k = [1.0f32, 4.0, 6.0, 4.0, 1.0];
    // Separable 5-tap binomial, then decimate.
    let mut tmp = vec![0.0f32; nw * h * ch];
    par_rows(&mut tmp, nw, ch, |y, row| {
        let line = &src[y * w * ch..(y + 1) * w * ch];
        for x in 0..nw {
            // The clamped source column of each tap is the same for every channel.
            let xs: [usize; 5] = std::array::from_fn(|t| {
                (2 * x as i64 + t as i64 - 2).clamp(0, w as i64 - 1) as usize * ch
            });
            for c in 0..ch {
                let mut s = 0.0;
                for (kv, xx) in k.iter().zip(xs) {
                    s += kv * line[xx + c];
                }
                row[x * ch + c] = s / 16.0;
            }
        }
    });
    let mut out = vec![0.0f32; nw * nh * ch];
    par_rows(&mut out, nw, ch, |y, row| {
        // Each output row is five source rows added in tap order: the same sum per element as
        // before (`0 + k0·a + k1·b + …`), but over whole rows so it vectorises.
        for (t, kv) in k.iter().enumerate() {
            let yy = (2 * y as i64 + t as i64 - 2).clamp(0, h as i64 - 1) as usize;
            for (o, &v) in row.iter_mut().zip(&tmp[yy * nw * ch..(yy + 1) * nw * ch]) {
                *o += kv * v;
            }
        }
        for o in row.iter_mut() {
            *o /= 16.0;
        }
    });
    (nw, nh, out)
}

pub(super) fn expand(sw: usize, sh: usize, ch: usize, src: &[f32], w: usize, h: usize) -> Vec<f32> {
    // Bilinear upsampling at half-pixel-aligned positions.
    let mut out = vec![0.0f32; w * h * ch];
    // Source columns and weights depend on x only: worked out once instead of for every row.
    // The clamped position is never negative, so truncating is `floor` (and spares the libm call).
    let cols: Vec<(usize, usize, f32)> = (0..w)
        .map(|x| {
            let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = fx as usize;
            (x0 * ch, (x0 + 1).min(sw - 1) * ch, fx - x0 as f32)
        })
        .collect();
    par_rows(&mut out, w, ch, |y, row| {
        let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = fy as usize;
        let ty = fy - y0 as f32;
        let y1 = (y0 + 1).min(sh - 1);
        let (r0, r1) = (
            &src[y0 * sw * ch..(y0 + 1) * sw * ch],
            &src[y1 * sw * ch..(y1 + 1) * sw * ch],
        );
        for (px, &(x0, x1, tx)) in row.chunks_exact_mut(ch.max(1)).zip(&cols) {
            for (c, o) in px.iter_mut().enumerate() {
                let top = r0[x0 + c] + (r0[x1 + c] - r0[x0 + c]) * tx;
                let bot = r1[x0 + c] + (r1[x1 + c] - r1[x0 + c]) * tx;
                *o = top + (bot - top) * ty;
            }
        }
    });
    out
}

/// Fills pixels with zero `wt` from their surroundings (push-pull), so pyramid levels don't
/// pull in black from outside an image.
pub(super) fn push_pull(w: usize, h: usize, ch: usize, px: &mut [f32], wt: &[f32]) {
    if w <= 1 && h <= 1 {
        return;
    }
    let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
    // Coarse sums. A coarse row is fed by two fine rows; walking them in raster order adds each
    // coarse cell's (up to) four fine pixels in the order the serial scatter-add did, from zero.
    let mut cpx = vec![0.0f32; nw * nh * ch];
    let mut cw = vec![0.0f32; nw * nh];
    par_rows(&mut cw, nw, 1, |cy, row| {
        for y in 2 * cy..(2 * cy + 2).min(h) {
            for (x, &a) in wt[y * w..(y + 1) * w].iter().enumerate() {
                row[x / 2] += a;
            }
        }
    });
    let px_in: &[f32] = px;
    par_rows(&mut cpx, nw, ch, |cy, row| {
        for y in 2 * cy..(2 * cy + 2).min(h) {
            for x in 0..w {
                let (i, j) = (y * w + x, x / 2);
                let a = wt[i];
                for c in 0..ch {
                    row[j * ch + c] += a * px_in[i * ch + c];
                }
            }
        }
    });
    // Normalise by the weight (this reads `cw` before it is reset to 0 / 1 below).
    par_rows(&mut cpx, nw, ch, |cy, row| {
        for cx in 0..nw {
            let wj = cw[cy * nw + cx];
            if wj > 0.0 {
                for c in 0..ch {
                    row[cx * ch + c] /= wj;
                }
            }
        }
    });
    for v in cw.iter_mut() {
        if *v > 0.0 {
            *v = 1.0;
        }
    }
    if cw.contains(&0.0) && cw.iter().any(|v| *v > 0.0) {
        push_pull(nw, nh, ch, &mut cpx, &cw);
    }
    par_rows(px, w, ch, |y, row| {
        for x in 0..w {
            let i = y * w + x;
            if wt[i] < 1.0 {
                let j = (y / 2) * nw + x / 2;
                let a = wt[i];
                for c in 0..ch {
                    row[x * ch + c] = a * row[x * ch + c] + (1.0 - a) * cpx[j * ch + c];
                }
            }
        }
    });
}

/// One image's pyramid in [`multiband`]: the grid origin `(gx0, gy0)` at level 0, the Laplacian
/// levels `(w, h, colours)` and the Gaussian weight levels.
type PyramidBand = (i32, i32, Vec<(usize, usize, Vec<f32>)>, Vec<Vec<f32>>);

/// The part of `band` on panorama row `py` of level `l`: the panorama column its first pixel
/// lands on, its width, its weight row and its colour row. `None` when it misses the row.
fn band_row(
    band: &PyramidBand,
    l: usize,
    ch: usize,
    py: usize,
) -> Option<(usize, usize, &[f32], &[f32])> {
    let (ox, oy) = ((band.0 >> l) as usize, (band.1 >> l) as usize);
    let (lw, lh, colours) = &band.2[l];
    (py >= oy && py - oy < *lh).then(|| {
        let y = py - oy;
        (
            ox,
            *lw,
            &band.3[l][y * lw..(y + 1) * lw],
            &colours[y * lw * ch..(y + 1) * lw * ch],
        )
    })
}

fn expand_add(sw: usize, sh: usize, ch: usize, src: &[f32], w: usize, _h: usize, out: &mut [f32]) {
    // Bilinear upsampling at half-pixel-aligned positions.
    // Source columns and weights depend on x only: worked out once instead of for every row.
    // The clamped position is never negative, so truncating is `floor` (and spares the libm call).
    let cols: Vec<(usize, usize, f32)> = (0..w)
        .map(|x| {
            let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = fx as usize;
            (x0 * ch, (x0 + 1).min(sw - 1) * ch, fx - x0 as f32)
        })
        .collect();
    par_rows(out, w, ch, |y, row| {
        let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = fy as usize;
        let ty = fy - y0 as f32;
        let y1 = (y0 + 1).min(sh - 1);
        let (r0, r1) = (
            &src[y0 * sw * ch..(y0 + 1) * sw * ch],
            &src[y1 * sw * ch..(y1 + 1) * sw * ch],
        );
        for (px, &(x0, x1, tx)) in row.chunks_exact_mut(ch.max(1)).zip(&cols) {
            for (c, o) in px.iter_mut().enumerate() {
                let top = r0[x0 + c] + (r0[x1 + c] - r0[x0 + c]) * tx;
                let bot = r1[x0 + c] + (r1[x1 + c] - r1[x0 + c]) * tx;
                *o += top + (bot - top) * ty;
            }
        }
    });
}

pub(super) fn multiband(
    w: usize,
    h: usize,
    mut imgs: Vec<RoiImage>,
    weights: impl Fn(usize, usize) -> f32 + Sync,
    levels: usize,
    ctl: &Ctl,
) -> crate::Result<(Vec<f32>, Vec<f32>)> {
    let budget = if cfg!(target_arch = "wasm32") { 0 } else { 256 * 1024 * 1024 };
    let ch = imgs.first().map_or(1, |i| i.ch);
    let levels = levels.max(1);
    let align = 1i32 << (levels - 1);
    // Level sizes of the panorama.
    let mut sizes = vec![(w, h)];
    for _ in 1..levels {
        let Some(&(pw, ph)) = sizes.last() else { break };
        sizes.push((pw.div_ceil(2), ph.div_ceil(2)));
    }
    let mut acc: Vec<Vec<f32>> = sizes
        .iter()
        .map(|&(lw, lh)| vec![0.0; lw * lh * ch])
        .collect();
    let mut wsum: Vec<Vec<f32>> = sizes.iter().map(|&(lw, lh)| vec![0.0; lw * lh]).collect();
    let margin = align * 4;
    // Each image on its ROI grown by a margin and aligned to the coarsest level's grid.
    let grid = |im: &RoiImage| {
        let gx0 = ((im.x0 - margin).max(0) / align) * align;
        let gy0 = ((im.y0 - margin).max(0) / align) * align;
        let gx1 = (im.x0 + im.w as i32 + margin).min(w as i32);
        let gy1 = (im.y0 + im.h as i32 + margin).min(h as i32);
        (
            gx0,
            gy0,
            (gx1 - gx0).max(1) as usize,
            (gy1 - gy0).max(1) as usize,
        )
    };
    // Building one band holds its grid's colour and weight planes about three times over: the
    // pyramids are 4/3 of a plane, and the grid copy, coverage and `expand` scratch come on top.
    let cost = |im: &RoiImage| {
        let (_, _, gw, gh) = grid(im);
        gw.saturating_mul(gh)
            .saturating_mul(ch + 1)
            .saturating_mul(4 * 3)
    };
    let band = |k: usize, imgs: &[RoiImage]| -> crate::Result<PyramidBand> {
        ctl.check()?;
        let im = &imgs[k];
        let (gx0, gy0, gw, gh) = grid(im);
        // Each grid pixel takes the image pixel that lands on it (every image pixel lands on at
        // most one), so rows of the grid are independent and fill in parallel.
        let src_of = |gx: usize, gy: usize| -> Option<usize> {
            let (x, y) = (
                gx as i64 + gx0 as i64 - im.x0 as i64,
                gy as i64 + gy0 as i64 - im.y0 as i64,
            );
            (x >= 0 && y >= 0 && (x as usize) < im.w && (y as usize) < im.h)
                .then(|| y as usize * im.w + x as usize)
        };
        let mut px = vec![0.0f32; gw * gh * ch];
        par_rows(&mut px, gw, ch, |gy, row| {
            for gx in 0..gw {
                if let Some(i) = src_of(gx, gy) {
                    row[gx * ch..(gx + 1) * ch].copy_from_slice(&im.px[i * ch..(i + 1) * ch]);
                }
            }
        });
        let mut cov = vec![0.0f32; gw * gh];
        par_rows(&mut cov, gw, 1, |gy, row| {
            for (gx, o) in row.iter_mut().enumerate() {
                if let Some(i) = src_of(gx, gy) {
                    let _ = i;
                    *o = 1.0;
                }
            }
        });
        let mut wt = vec![0.0f32; gw * gh];
        par_rows(&mut wt, gw, 1, |gy, row| {
            for (gx, o) in row.iter_mut().enumerate() {
                if let Some(i) = src_of(gx, gy) {
                    *o = weights(k, i);
                }
            }
        });
        let bin: Vec<f32> = par_map1(&cov, |a| if a > 0.5 { 1.0 } else { 0.0 });
        drop(cov);
        push_pull(gw, gh, ch, &mut px, &bin);
        drop(bin);
        // Laplacian of the colours, Gaussian of the weights. Planes move into the pyramids rather
        // than being copied: with one image built at a time, a copy is a serial pass over it.
        let mut lap: Vec<(usize, usize, Vec<f32>)> = Vec::new();
        let mut gau: Vec<Vec<f32>> = Vec::new();
        let (mut cw, mut chh, mut cur, mut cwt) = (gw, gh, px, wt);
        for l in 0..levels {
            ctl.check()?;
            if l + 1 == levels {
                lap.push((cw, chh, cur));
                gau.push(cwt);
                break;
            }
            let (nw, nh, down) = reduce(cw, chh, ch, &cur);
            let up = expand(nw, nh, ch, &down, cw, chh);
            lap.push((cw, chh, par_zip(&cur, &up, |a, b| a - b)));
            let (_, _, wdown) = reduce(cw, chh, 1, &cwt);
            gau.push(cwt);
            cw = nw;
            chh = nh;
            cur = down;
            cwt = wdown;
        }
        Ok((gx0, gy0, lap, gau))
    };
    // Build the bands a batch at a time and add each batch before building the next. Each pixel
    // adds the bands in the same order as the serial loop (per level, bands one after another;
    // batches follow each other in image order), but rows of a level are independent, so they
    // run in parallel: every row walks the batch's bands itself.
    let mut start = 0;
    while start < imgs.len() {
        let mut end = start + 1;
        let mut bytes = cost(&imgs[start]);
        while let Some(im) = imgs.get(end)
            && bytes.saturating_add(cost(im)) <= budget
        {
            bytes = bytes.saturating_add(cost(im));
            end += 1;
        }
        ctl.check()?;
        let bands = par_map(end - start, |j| band(start + j, &imgs))
            .into_iter()
            .collect::<crate::Result<Vec<_>>>()?;
        for l in 0..levels {
            let pw = sizes[l].0;
            par_rows(&mut wsum[l], pw, 1, |py, row| {
                for b in &bands {
                    let Some((ox, lw, g, _)) = band_row(b, l, ch, py) else {
                        continue;
                    };
                    for (x, &k) in g.iter().enumerate().take(lw) {
                        let px = ox + x;
                        if px >= pw {
                            break;
                        }
                        if k <= 0.0 {
                            continue;
                        }
                        row[px] += k;
                    }
                }
            });
            par_rows(&mut acc[l], pw, ch, |py, row| {
                for b in &bands {
                    let Some((ox, lw, g, band)) = band_row(b, l, ch, py) else {
                        continue;
                    };
                    for (x, &k) in g.iter().enumerate().take(lw) {
                        let px = ox + x;
                        if px >= pw {
                            break;
                        }
                        if k <= 0.0 {
                            continue;
                        }
                        for c in 0..ch {
                            row[px * ch + c] += k * band[x * ch + c];
                        }
                    }
                }
            });
        }
        for im in &mut imgs[start..end] {
            im.px = Vec::new();
        }
        start = end;
        ctl.report("blend", 0.2 + 0.7 * start as f32 / imgs.len().max(1) as f32);
    }
    for l in 0..levels {
        let pw = sizes[l].0;
        let ws = &wsum[l];
        par_rows(&mut acc[l], pw, ch, |py, row| {
            for x in 0..pw {
                let k = ws[py * pw + x];
                for c in 0..ch {
                    row[x * ch + c] = if k > 1e-8 { row[x * ch + c] / k } else { 0.0 };
                }
            }
        });
    }
    drop(wsum);
    // Collapse.
    // `levels ≥ 1`, so there is always a coarsest level.
    let Some(mut cur) = acc.pop() else {
        return Ok((vec![0.0; w * h * ch], vec![0.0; w * h]));
    };
    for l in (0..levels - 1).rev() {
        let (pw, ph) = sizes[l];
        let (sw, sh) = sizes[l + 1];
        ctl.check()?;
        let mut next = acc.pop().expect("level");
        expand_add(sw, sh, ch, &cur, pw, ph, &mut next);
        cur = next;
    }
    // Coverage: union of the images. Rows are independent; each walks the images in order, so a
    // pixel sees the same sequence of `max` calls as in the serial loop.
    let mut cov = vec![0.0f32; w * h];
    par_rows(&mut cov, w, 1, |py, row| {
        for im in &imgs {
            let y = py as i64 - im.y0 as i64;
            if y < 0 || y as usize >= im.h {
                continue;
            }
            for x in 0..im.w {
                let px = im.x0 + x as i32;
                if px < 0 || px as usize >= w {
                    continue;
                }
                let p = px as usize;
                row[p] = row[p].max(1.0);
            }
        }
    });
    Ok((cur, cov))
}
