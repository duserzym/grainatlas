//! Statistics use f64 arithmetic on the sampled f32/normalised u8 values.
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub n_px: usize,
    pub mean: Option<f64>,
    pub median: Option<f64>,
    pub sd: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub p5: Option<f64>,
    pub p25: Option<f64>,
    pub p75: Option<f64>,
    pub p95: Option<f64>,
    pub iqr: Option<f64>,
    pub skewness: Option<f64>,
    pub glcm_contrast: Option<f64>,
    pub glcm_homogeneity: Option<f64>,
}

pub fn quantile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let at = (sorted.len() - 1) as f64 * p;
    let lo = at.floor() as usize;
    Some(sorted[lo] + (sorted[at.ceil() as usize] - sorted[lo]) * at.fract())
}

pub fn describe(v: &mut [f64]) -> Stats {
    v.sort_unstable_by(f64::total_cmp);
    let n = v.len();
    if n == 0 {
        return Stats::default();
    }
    let mean = v.iter().sum::<f64>() / n as f64;
    let m2 = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
    let sd = (n > 1).then(|| (m2 / (n - 1) as f64).sqrt());
    let skewness = if n >= 3 && m2 > 0.0 {
        let m3 = v.iter().map(|x| (x - mean).powi(3)).sum::<f64>();
        Some(n as f64 * m3 / ((n - 1) * (n - 2)) as f64 / sd.unwrap().powi(3))
    } else {
        None
    };
    Stats {
        n_px: n,
        mean: Some(mean),
        median: quantile(v, 0.5),
        sd,
        min: v.first().copied(),
        max: v.last().copied(),
        p5: quantile(v, 0.05),
        p25: quantile(v, 0.25),
        p75: quantile(v, 0.75),
        p95: quantile(v, 0.95),
        iqr: Some(quantile(v, 0.75).unwrap() - quantile(v, 0.25).unwrap()),
        skewness,
        ..Stats::default()
    }
}

/// Each direction is normalised separately; the symmetric matrix gives identical
/// contrast/homogeneity to its directed pairs. Average only directions with pairs.
pub fn texture(
    values: &[f64],
    mask: &[u8],
    w: usize,
    h: usize,
    sorted: &[f64],
    ctl: &crate::Ctl,
) -> crate::Result<(Option<f64>, Option<f64>)> {
    let (Some(lo), Some(hi)) = (quantile(sorted, 0.01), quantile(sorted, 0.99)) else {
        return Ok((None, None));
    };
    let q = |v: f64| {
        if hi <= lo {
            0.0
        } else {
            (((v - lo) / (hi - lo) * 32.0).floor()).clamp(0.0, 31.0)
        }
    };
    let (mut contrast, mut homogeneity, mut dirs) = (0.0, 0.0, 0);
    for (dx, dy) in [(1isize, 0isize), (0, 1), (1, 1), (-1, 1)] {
        let (mut c, mut hm, mut n) = (0.0, 0.0, 0usize);
        for y in 0..h {
            if y % 64 == 0 {
                ctl.check()?;
            }
            for x in 0..w {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let (i, j) = (y * w + x, ny as usize * w + nx as usize);
                if mask[i] == 0 || mask[j] == 0 {
                    continue;
                }
                let d2 = (q(values[i]) - q(values[j])).powi(2);
                c += d2;
                hm += 1.0 / (1.0 + d2);
                n += 1;
            }
        }
        if n > 0 {
            contrast += c / n as f64;
            homogeneity += hm / n as f64;
            dirs += 1;
        }
    }
    Ok(if dirs == 0 {
        (None, None)
    } else {
        (
            Some(contrast / dirs as f64),
            Some(homogeneity / dirs as f64),
        )
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct Relative {
    pub n: usize,
    pub z: Option<f64>,
    pub robust_z: Option<f64>,
    pub percentile: Option<f64>,
}

/// Reference populations are precomputed once per group, avoiding O(grains²).
pub struct Reference {
    sorted: Vec<f64>,
    mean: Option<f64>,
    sd: Option<f64>,
    median: Option<f64>,
    mad: Option<f64>,
}
impl Reference {
    pub fn new(mut values: Vec<f64>) -> Self {
        let s = describe(&mut values);
        let mut dev: Vec<f64> = values
            .iter()
            .map(|x| (x - s.median.unwrap()).abs())
            .collect();
        dev.sort_unstable_by(f64::total_cmp);
        Self {
            sorted: values,
            mean: s.mean,
            sd: s.sd,
            median: s.median,
            mad: quantile(&dev, 0.5),
        }
    }
    pub fn of(&self, value: Option<f64>, min_n: usize) -> Relative {
        let n = self.sorted.len();
        let mut r = Relative {
            n,
            z: None,
            robust_z: None,
            percentile: None,
        };
        if n < min_n {
            return r;
        }
        let Some(x) = value else {
            return r;
        };
        r.z = self
            .sd
            .filter(|s| *s > 0.0)
            .map(|s| (x - self.mean.unwrap()) / s);
        r.robust_z = self
            .mad
            .filter(|s| *s > 0.0)
            .map(|s| (x - self.median.unwrap()) / (1.4826 * s));
        let less = self.sorted.partition_point(|v| *v < x);
        let equal = self.sorted.partition_point(|v| *v <= x) - less;
        r.percentile = Some(100.0 * (less as f64 + equal as f64 / 2.0) / n as f64);
        r
    }
}
