//! Point-pair registration of a reference image (a photo of an annotated
//! printout, a post-laser frame, a navigation image) to the mosaic: one pair
//! translates, two give a similarity, three or more an affine fit by least
//! squares, four or more optionally a homography (DLT).

use serde::{Deserialize, Serialize};

/// Image pixel → mosaic micrometre, as a 2D affine `[a, b, c, d, e, f]`:
/// x' = a·x + c·y + e, y' = b·x + d·y + f (the SVG / canvas matrix order).
pub type Affine = [f64; 6];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Registration {
    pub matrix: Affine,
    /// RMS of the pair residuals in micrometres (0 for fewer than 4 pairs).
    pub rms_um: f64,
    pub residuals_um: Vec<f64>,
}

pub fn apply(m: &Affine, x: f64, y: f64) -> [f64; 2] {
    [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
}

/// Registration from pairs (image px → mosaic um).
pub fn fit(src: &[[f64; 2]], dst: &[[f64; 2]]) -> Option<Registration> {
    let n = src.len().min(dst.len());
    let m = match n {
        0 => return None,
        1 => [1.0, 0.0, 0.0, 1.0, dst[0][0] - src[0][0], dst[0][1] - src[0][1]],
        2 => similarity(src, dst)?,
        _ => affine(&src[..n], &dst[..n])?,
    };
    let residuals_um: Vec<f64> = (0..n)
        .map(|i| {
            let p = apply(&m, src[i][0], src[i][1]);
            (p[0] - dst[i][0]).hypot(p[1] - dst[i][1])
        })
        .collect();
    let rms_um = if n > 3 { (residuals_um.iter().map(|r| r * r).sum::<f64>() / n as f64).sqrt() } else { 0.0 };
    Some(Registration { matrix: m, rms_um, residuals_um })
}

fn similarity(src: &[[f64; 2]], dst: &[[f64; 2]]) -> Option<Affine> {
    let (s0, s1, d0, d1) = (src[0], src[1], dst[0], dst[1]);
    let (sx, sy) = (s1[0] - s0[0], s1[1] - s0[1]);
    let (dx, dy) = (d1[0] - d0[0], d1[1] - d0[1]);
    let den = sx * sx + sy * sy;
    if den < 1e-12 {
        return None;
    }
    // Complex ratio (dx + i dy) / (sx + i sy) = a + i b.
    let a = (dx * sx + dy * sy) / den;
    let b = (dy * sx - dx * sy) / den;
    let e = d0[0] - (a * s0[0] - b * s0[1]);
    let f = d0[1] - (b * s0[0] + a * s0[1]);
    Some([a, b, -b, a, e, f])
}

fn affine(src: &[[f64; 2]], dst: &[[f64; 2]]) -> Option<Affine> {
    // Two independent 3-parameter least squares: x' = a x + c y + e, y' = b x + d y + f.
    let mut ata = [0.0f64; 9];
    let mut atx = [0.0f64; 3];
    let mut aty = [0.0f64; 3];
    for (s, d) in src.iter().zip(dst) {
        let r = [s[0], s[1], 1.0];
        for i in 0..3 {
            for j in 0..3 {
                ata[i * 3 + j] += r[i] * r[j];
            }
            atx[i] += r[i] * d[0];
            aty[i] += r[i] * d[1];
        }
    }
    let px = crate::photometric::solve_dense(ata.to_vec(), atx.to_vec(), 3)?;
    let py = crate::photometric::solve_dense(ata.to_vec(), aty.to_vec(), 3)?;
    Some([px[0], py[0], px[1], py[1], px[2], py[2]])
}

/// Inverse of an affine (mosaic um → image px).
pub fn invert(m: &Affine) -> Option<Affine> {
    let det = m[0] * m[3] - m[2] * m[1];
    if det.abs() < 1e-15 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])])
}
