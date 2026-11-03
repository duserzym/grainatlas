//! Phase correlation of the overlap band of two tiles.

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use super::StitchParams;
use crate::image::Gray;

/// Measured offset of `b` relative to `a` (mosaic px) and the peak-to-sidelobe ratio.
///
/// `prior` is the expected offset. The overlap implied by the prior is cut from
/// both tiles, correlated at `coarse_factor` and then (optionally) at full
/// resolution around the coarse estimate.
pub fn measure_pair(a: &Gray, b: &Gray, prior: [f64; 2], p: &StitchParams) -> Option<([f64; 2], f64)> {
    let f = p.coarse_factor.max(1);
    let (ca, cb) = (a.downsample(f), b.downsample(f));
    let pr = [prior[0] / f as f64, prior[1] / f as f64];
    let (d, psr) = correlate_overlap(&ca, &cb, pr)?;
    let mut est = [pr[0] + d[0], pr[1] + d[1]];
    est = [est[0] * f as f64, est[1] * f as f64];
    if p.refine_full && f > 1 {
        let r = [est[0].round(), est[1].round()];
        if let Some((d2, psr2)) = correlate_overlap(a, b, r) {
            // The fine pass only corrects a small residual; a large one means it locked on noise.
            if d2[0].abs() <= f as f64 * 1.5 && d2[1].abs() <= f as f64 * 1.5 && psr2 >= p.min_psr * 0.5 {
                est = [r[0] + d2[0], r[1] + d2[1]];
            }
        }
    }
    if p.refine_full {
        est = lk_refine(a, b, est).unwrap_or(est);
    }
    Some((est, psr))
}

/// Gauss-Newton (Lucas-Kanade) refinement of a translation on the overlap, with each side
/// normalised to zero mean and unit variance. Returns `None` if it diverges.
pub fn lk_refine(a: &Gray, b: &Gray, d0: [f64; 2]) -> Option<[f64; 2]> {
    let mut d = d0;
    for _ in 0..8 {
        let (dx, dy) = (d[0].floor() as i64, d[1].floor() as i64);
        // Overlap in a's coordinates with a 2 px safety margin.
        let x0 = dx.max(0) + 2;
        let y0 = dy.max(0) + 2;
        let x1 = (a.w as i64).min(dx + b.w as i64) - 3;
        let y1 = (a.h as i64).min(dy + b.h as i64) - 3;
        if x1 - x0 < 16 || y1 - y0 < 16 {
            return None;
        }
        let step = (((x1 - x0) * (y1 - y0)) as f64 / 120_000.0).sqrt().max(1.0) as usize;
        let mut sa = Vec::new();
        let mut sb = Vec::new();
        let mut gx = Vec::new();
        let mut gy = Vec::new();
        for y in (y0..y1).step_by(step) {
            for x in (x0..x1).step_by(step) {
                let (xu, yu) = (x as usize, y as usize);
                sa.push(a.px[yu * a.w + xu] as f64);
                gx.push((a.px[yu * a.w + xu + 1] as f64 - a.px[yu * a.w + xu - 1] as f64) * 0.5);
                gy.push((a.px[(yu + 1) * a.w + xu] as f64 - a.px[(yu - 1) * a.w + xu] as f64) * 0.5);
                sb.push(b.bilinear((x as f64 - d[0]) as f32, (y as f64 - d[1]) as f32) as f64);
            }
        }
        let norm = |v: &mut Vec<f64>| -> f64 {
            let n = v.len() as f64;
            let m = v.iter().sum::<f64>() / n;
            let sd = (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / n).sqrt().max(1e-9);
            v.iter_mut().for_each(|x| *x = (*x - m) / sd);
            sd
        };
        let sda = norm(&mut sa);
        norm(&mut sb);
        // b(x - d) ≈ a(x)  ⇒  a(x + δ) ≈ a(x) + ∇a·δ where δ = d_true - d... solve for the shift
        // that moves a onto b: r = b - a ≈ -∇a·δ.
        let (mut h11, mut h12, mut h22, mut g1, mut g2) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for i in 0..sa.len() {
            let (ix, iy) = (gx[i] / sda, gy[i] / sda);
            let r = sb[i] - sa[i];
            h11 += ix * ix;
            h12 += ix * iy;
            h22 += iy * iy;
            g1 += ix * r;
            g2 += iy * r;
        }
        let det = h11 * h22 - h12 * h12;
        if det.abs() < 1e-9 * (h11 * h22).max(1e-30) || h11 < 1e-6 || h22 < 1e-6 {
            return None;
        }
        // b(x - d) = a(x + e) with e the error; minimise Σ(a + ∇a·e - b)² ⇒ H e = Σ∇a (b - a).
        let ex = (h22 * g1 - h12 * g2) / det;
        let ey = (h11 * g2 - h12 * g1) / det;
        // b(x - d) = a(x + e) ⇒ b(x) = a(x + d + e) ⇒ true offset D satisfies b(x) = a(x + D): D = d + e.
        d = [d[0] + ex, d[1] + ey];
        if (d[0] - d0[0]).abs() > 3.0 || (d[1] - d0[1]).abs() > 3.0 {
            return None;
        }
        if ex.abs() < 1e-3 && ey.abs() < 1e-3 {
            break;
        }
    }
    Some(d)
}

/// Residual shift (b relative to a, beyond `prior`) from the overlap rectangle implied by `prior`.
pub fn correlate_overlap(a: &Gray, b: &Gray, prior: [f64; 2]) -> Option<([f64; 2], f64)> {
    let (dx, dy) = (prior[0].round() as i64, prior[1].round() as i64);
    // Overlap in a's coordinates.
    let x0 = dx.max(0);
    let y0 = dy.max(0);
    let x1 = (a.w as i64).min(dx + b.w as i64);
    let y1 = (a.h as i64).min(dy + b.h as i64);
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if x1 <= x0 || y1 <= y0 || w < 16 || h < 16 {
        return None;
    }
    let ra = a.crop(x0 as usize, y0 as usize, w, h);
    let rb = b.crop((x0 - dx) as usize, (y0 - dy) as usize, w, h);
    let (sx, sy, psr) = phase_correlate(&ra, &rb)?;
    // With b's true offset D = round(prior) + e, rb(x) = ra(x + e), i.e. a content shift
    // s = -e. Return the offset beyond the prior: D - prior.
    Some(([dx as f64 - sx - prior[0], dy as f64 - sy - prior[1]], psr))
}

/// Phase correlation of two same-size images. Returns the shift `(sx, sy)` such
/// that `b(x) ≈ a(x - s)` and the peak-to-sidelobe ratio.
pub fn phase_correlate(a: &Gray, b: &Gray) -> Option<(f64, f64, f64)> {
    let (w, h) = (a.w, a.h);
    let mut planner = FftPlanner::<f64>::new();
    let win = |n: usize, i: usize| 0.5 - 0.5 * libm::cos(2.0 * std::f64::consts::PI * (i as f64 + 0.5) / n as f64);
    let prep = |g: &Gray| {
        let mean = g.px.iter().map(|&v| v as f64).sum::<f64>() / g.px.len() as f64;
        let mut v: Vec<Complex<f64>> = Vec::with_capacity(w * h);
        for y in 0..h {
            let wy = win(h, y);
            for x in 0..w {
                v.push(Complex::new((g.px[y * w + x] as f64 - mean) * wy * win(w, x), 0.0));
            }
        }
        v
    };
    let mut fa = prep(a);
    let mut fb = prep(b);
    let energy: f64 = fa.iter().map(|c| c.re * c.re).sum::<f64>().min(fb.iter().map(|c| c.re * c.re).sum::<f64>());
    if energy / ((w * h) as f64) < 1e-8 {
        return None; // flat (empty epoxy)
    }
    fft2(&mut planner, &mut fa, w, h, false);
    fft2(&mut planner, &mut fb, w, h, false);
    let mut r: Vec<Complex<f64>> = fa
        .iter()
        .zip(&fb)
        .map(|(x, y)| {
            let c = y * x.conj();
            let m = c.norm();
            if m > 1e-12 { c / m } else { Complex::new(0.0, 0.0) }
        })
        .collect();
    fft2(&mut planner, &mut r, w, h, true);
    let c: Vec<f64> = r.iter().map(|z| z.re).collect();
    let (mut bi, mut bv) = (0usize, f64::NEG_INFINITY);
    for (i, &v) in c.iter().enumerate() {
        if v > bv {
            bv = v;
            bi = i;
        }
    }
    let (px, py) = (bi % w, bi / w);
    let at = |x: i64, y: i64| c[(y.rem_euclid(h as i64) as usize) * w + x.rem_euclid(w as i64) as usize];
    let sub = |l: f64, m: f64, r: f64| -> f64 {
        // Foroosh et al. estimator on the larger neighbour, parabola as a fallback.
        if r >= l && r > 0.0 && m + r > 0.0 {
            r / (r + m)
        } else if l > r && l > 0.0 && m + l > 0.0 {
            -l / (l + m)
        } else {
            let den = l - 2.0 * m + r;
            if den.abs() > 1e-12 { 0.5 * (l - r) / den } else { 0.0 }
        }
    };
    let ox = sub(at(px as i64 - 1, py as i64), bv, at(px as i64 + 1, py as i64)).clamp(-1.0, 1.0);
    let oy = sub(at(px as i64, py as i64 - 1), bv, at(px as i64, py as i64 + 1)).clamp(-1.0, 1.0);
    let wrap = |p: usize, n: usize| if p > n / 2 { p as f64 - n as f64 } else { p as f64 };
    let sx = wrap(px, w) + ox;
    let sy = wrap(py, h) + oy;
    // Peak-to-sidelobe ratio: peak against mean and sd outside an 11x11 window.
    let (mut s, mut s2, mut n) = (0.0f64, 0.0f64, 0usize);
    for y in 0..h {
        let ddy = (y as i64 - py as i64).rem_euclid(h as i64).min((py as i64 - y as i64).rem_euclid(h as i64));
        for x in 0..w {
            let ddx = (x as i64 - px as i64).rem_euclid(w as i64).min((px as i64 - x as i64).rem_euclid(w as i64));
            if ddx <= 5 && ddy <= 5 {
                continue;
            }
            let v = c[y * w + x];
            s += v;
            s2 += v * v;
            n += 1;
        }
    }
    let mean = s / n.max(1) as f64;
    let sd = (s2 / n.max(1) as f64 - mean * mean).max(1e-30).sqrt();
    Some((sx, sy, (bv - mean) / sd))
}

fn fft2(planner: &mut FftPlanner<f64>, v: &mut [Complex<f64>], w: usize, h: usize, inverse: bool) {
    let fw = if inverse { planner.plan_fft_inverse(w) } else { planner.plan_fft_forward(w) };
    for row in v.chunks_mut(w) {
        fw.process(row);
    }
    let fh = if inverse { planner.plan_fft_inverse(h) } else { planner.plan_fft_forward(h) };
    let mut col = vec![Complex::new(0.0, 0.0); h];
    for x in 0..w {
        for y in 0..h {
            col[y] = v[y * w + x];
        }
        fh.process(&mut col);
        for y in 0..h {
            v[y * w + x] = col[y];
        }
    }
    if inverse {
        let k = 1.0 / (w * h) as f64;
        for z in v.iter_mut() {
            *z *= k;
        }
    }
}
