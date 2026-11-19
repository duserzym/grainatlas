//! Manual paper transforms. Source rectangles and registered data never change.
use super::*;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct PieceTransform {
    /// Uniform multiplier relative to the automatic layout.
    pub scale: f64,
    pub rotation_deg: f64,
}
impl Default for PieceTransform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            rotation_deg: 0.0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PiecePlacement {
    pub strip: StripKey,
    pub sheet: usize,
    pub origin_mm: [f64; 2],
    pub scale: f64,
    pub rotation_deg: f64,
}
/// SVG affine convention: x'=a*x+c*y+e, y'=b*x+d*y+f.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BandTransform {
    pub um_per_mm: f64,
    pub rotation_deg: f64,
    pub crop_um: [f64; 4],
    pub mount_to_paper: [f64; 6],
    pub paper_to_mount: [f64; 6],
}
pub fn point(m: [f64; 6], p: [f64; 2]) -> [f64; 2] {
    [
        m[0] * p[0] + m[2] * p[1] + m[4],
        m[1] * p[0] + m[3] * p[1] + m[5],
    ]
}
pub fn inverse(m: [f64; 6]) -> [f64; 6] {
    let det = m[0] * m[3] - m[1] * m[2];
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    [a, b, c, d, -a * m[4] - c * m[5], -b * m[4] - d * m[5]]
}
pub fn bounds(m: [f64; 6], r: [f64; 4]) -> [f64; 4] {
    let ps = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]].map(|p| point(m, p));
    [
        ps.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
        ps.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
        ps.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max),
        ps.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max),
    ]
}
/// Transform local, unscaled strip coordinates to paper, anchored at its AABB top-left.
pub fn matrix(s: &SheetStrip) -> [f64; 6] {
    let t = s.transform.unwrap_or_default();
    let (sin, cos) = t.rotation_deg.to_radians().sin_cos();
    let mut m = [
        cos * t.scale,
        sin * t.scale,
        -sin * t.scale,
        cos * t.scale,
        0.0,
        0.0,
    ];
    let r = bounds(m, [0.0, 0.0, s.size_mm[0], s.size_mm[1]]);
    m[4] = s.origin_mm[0] - r[0];
    m[5] = s.origin_mm[1] - r[1];
    m
}
pub fn size(s: &SheetStrip) -> [f64; 2] {
    let r = bounds(matrix(s), [0.0, 0.0, s.size_mm[0], s.size_mm[1]]);
    [r[2] - r[0], r[3] - r[1]]
}
pub fn clamp(s: &mut SheetStrip, body: [f64; 4]) -> Result<(), String> {
    let size = size(s);
    if size[0] > body[2] - body[0] + 1e-7 || size[1] > body[3] - body[1] + 1e-7 {
        return Err("piece does not fit below the header".into());
    }
    for d in 0..2 {
        s.origin_mm[d] = s.origin_mm[d].clamp(body[d], body[d + 2] - size[d]);
    }
    Ok(())
}
pub fn band_transform(s: &SheetStrip, b: &SheetBand, k: f64) -> BandTransform {
    let m = matrix(s);
    let o = point(
        m,
        [
            b.offset_mm - b.mosaic_rect_um[0] / k,
            LABEL_MM - b.mosaic_rect_um[1] / k,
        ],
    );
    let forward = [m[0] / k, m[1] / k, m[2] / k, m[3] / k, o[0], o[1]];
    BandTransform {
        um_per_mm: k / s.transform.unwrap_or_default().scale,
        rotation_deg: s.transform.unwrap_or_default().rotation_deg,
        crop_um: b.mosaic_rect_um,
        mount_to_paper: forward,
        paper_to_mount: inverse(forward),
    }
}
/// Largest 1-2-5 value no longer than the requested physical span.
pub fn scale_bar(um_per_mm: f64, width_mm: f64) -> f64 {
    let max = (um_per_mm * width_mm).max(f64::MIN_POSITIVE);
    let power = 10.0_f64.powf(max.log10().floor());
    [5.0, 2.0, 1.0]
        .into_iter()
        .map(|v| v * power)
        .find(|v| *v <= max * (1.0 + 1e-12))
        .unwrap_or(power)
}

/// A reference row is part of the piece, so packing and collision checks reserve
/// its circles and labels even for a short or narrow source image.
pub(super) fn references(s: &mut SheetStrip, k: f64, sizes: &[f64]) {
    let shown = key_spots(sizes, k).0;
    let width = shown.iter().sum::<f64>() / k + shown.len() as f64 * 4.0 + 4.0;
    let height = shown.iter().copied().fold(0.0, f64::max) / k + 7.0;
    s.size_mm[0] = s.size_mm[0].max(width);
    s.size_mm[1] += height;
}
