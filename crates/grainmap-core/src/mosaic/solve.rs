//! Global least squares over pairwise offsets (conjugate gradient on the sparse
//! normal equations, first tile fixed), with one round of outlier rejection.

use super::PairResult;

/// Weight of a prior (fallback) edge relative to a measured one.
const PRIOR_WEIGHT: f64 = 1e-3;

/// Returns solved positions (same frame as `prior`) and the pairs with residuals.
pub fn solve(prior: &[[f64; 2]], mut pairs: Vec<PairResult>, outlier_px: f64) -> (Vec<[f64; 2]>, Vec<PairResult>) {
    let mut pos = solve_once(prior, &pairs);
    residuals(&pos, &mut pairs);
    // Drop the worst pair above the threshold and re-solve, until none is left above it: one bad
    // pair spreads its error over its loop, so dropping everything above the threshold at once
    // would also discard the good pairs around it.
    for _ in 0..pairs.len() {
        let worst = pairs.iter().enumerate().filter(|(_, p)| p.used && p.residual > outlier_px).max_by(|a, b| a.1.residual.total_cmp(&b.1.residual)).map(|(i, _)| i);
        let Some(i) = worst else { break };
        pairs[i].used = false;
        pos = solve_once(prior, &pairs);
        residuals(&pos, &mut pairs);
    }
    (pos, pairs)
}

fn residuals(pos: &[[f64; 2]], pairs: &mut [PairResult]) {
    for p in pairs.iter_mut() {
        p.residual = match p.measured {
            Some(m) => {
                let d = [pos[p.b][0] - pos[p.a][0], pos[p.b][1] - pos[p.a][1]];
                (d[0] - m[0]).hypot(d[1] - m[1])
            }
            None => 0.0,
        };
        if !p.used {
            // Unused pairs keep their residual for the report but do not count.
        }
    }
}

fn solve_once(prior: &[[f64; 2]], pairs: &[PairResult]) -> Vec<[f64; 2]> {
    let n = prior.len();
    if n == 0 {
        return Vec::new();
    }
    // Edges: (a, b, d, weight). Every neighbour pair contributes its prior offset with a small
    // weight, so tiles without texture follow the stage; measured pairs dominate where present.
    let mut edges: Vec<(usize, usize, [f64; 2], f64)> = Vec::new();
    for p in pairs {
        edges.push((p.a, p.b, p.prior, PRIOR_WEIGHT));
        if p.used
            && let Some(m) = p.measured
        {
            // Confidence from the peak-to-sidelobe ratio: weak peaks give way to strong ones.
            edges.push((p.a, p.b, m, (p.psr / 40.0).clamp(0.1, 1.0)));
        }
    }
    // Tiles that appear in no pair are anchored to the prior.
    let mut deg = vec![0usize; n];
    for e in &edges {
        deg[e.0] += 1;
        deg[e.1] += 1;
    }
    let mut out = vec![[0.0; 2]; n];
    for axis in 0..2 {
        // Unknowns: positions of tiles 1..n relative to tile 0 (fixed at its prior).
        let apply = |x: &[f64], y: &mut [f64]| {
            y.iter_mut().for_each(|v| *v = 0.0);
            for &(a, b, _, w) in &edges {
                let xa = if a == 0 { 0.0 } else { x[a] };
                let xb = if b == 0 { 0.0 } else { x[b] };
                let r = w * (xb - xa);
                if b != 0 {
                    y[b] += r;
                }
                if a != 0 {
                    y[a] -= r;
                }
            }
            for i in 1..n {
                if deg[i] == 0 {
                    y[i] += x[i];
                }
            }
        };
        let mut rhs = vec![0.0; n];
        let x0 = prior[0][axis];
        for &(a, b, d, w) in &edges {
            // w * ((x_b - x_a) - d), with x_0 = x0 fixed.
            let c = d[axis] + if a == 0 { x0 } else { 0.0 } - if b == 0 { x0 } else { 0.0 };
            if b != 0 {
                rhs[b] += w * c;
            }
            if a != 0 {
                rhs[a] -= w * c;
            }
        }
        for i in 1..n {
            if deg[i] == 0 {
                rhs[i] += prior[i][axis];
            }
        }
        // Start from the prior.
        let mut x: Vec<f64> = (0..n).map(|i| if i == 0 { 0.0 } else { prior[i][axis] }).collect();
        let mut ax = vec![0.0; n];
        apply(&x, &mut ax);
        let mut r: Vec<f64> = (0..n).map(|i| if i == 0 { 0.0 } else { rhs[i] - ax[i] }).collect();
        let mut pdir = r.clone();
        let mut rr: f64 = r.iter().map(|v| v * v).sum();
        let tol = 1e-20 * rhs.iter().map(|v| v * v).sum::<f64>().max(1.0);
        for _ in 0..(10 * n + 100) {
            if rr <= tol {
                break;
            }
            apply(&pdir, &mut ax);
            ax[0] = 0.0;
            let pap: f64 = pdir.iter().zip(&ax).map(|(a, b)| a * b).sum();
            if pap.abs() < 1e-300 {
                break;
            }
            let alpha = rr / pap;
            for i in 1..n {
                x[i] += alpha * pdir[i];
                r[i] -= alpha * ax[i];
            }
            let rr2: f64 = r.iter().map(|v| v * v).sum();
            let beta = rr2 / rr;
            rr = rr2;
            for i in 1..n {
                pdir[i] = r[i] + beta * pdir[i];
            }
        }
        out[0][axis] = x0;
        for i in 1..n {
            out[i][axis] = x[i];
        }
    }
    out
}
