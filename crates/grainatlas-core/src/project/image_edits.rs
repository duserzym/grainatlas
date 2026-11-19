//! Non-destructive display recipes. Geometry and source pixels always stay in mount µm.
use super::AreaRecord;
use crate::segment::Grain;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// Keep the SVG preview's maskAppearance in ui/src/lib/image-edits.ts in step.
pub const MASK_BACKGROUND: u8 = 224;
pub const MASK_INK: u8 = 144;
pub const MASK_EDGE_UM: f64 = 6.0;
pub const MASK_PITCH_UM: f64 = 18.0;
pub const MASK_LINE_UM: f64 = 2.0;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageEdits {
    pub version: u32,
    pub areas: BTreeMap<String, AreaImageEdits>,
}
impl Default for ImageEdits {
    fn default() -> Self {
        Self {
            version: 1,
            areas: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AreaImageEdits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<ImageCrop>,
    pub rotate_deg: f64,
    pub flip_h: bool,
    pub flip_v: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<ImageLevels>,
    pub masks: Vec<MaskStroke>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CropKind {
    Rect,
    Polygon,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageCrop {
    pub kind: CropKind,
    pub points_um: Vec<[f64; 2]>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageLevels {
    pub black: u8,
    pub white: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MaskStroke {
    pub points_um: Vec<[f64; 2]>,
    pub radius_um: f64,
}

pub fn bounds(points: &[[f64; 2]]) -> [f64; 4] {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |r, p| {
            [
                r[0].min(p[0]),
                r[1].min(p[1]),
                r[2].max(p[0]),
                r[3].max(p[1]),
            ]
        },
    )
}
fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let n = d[0] * d[0] + d[1] * d[1];
    let t = if n == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / n).clamp(0.0, 1.0)
    };
    (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
}
impl AreaImageEdits {
    pub fn in_crop(&self, p: [f64; 2]) -> bool {
        if let Some(c) = &self.crop {
            match c.kind {
                CropKind::Rect if c.points_um.len() >= 2 => {
                    let b = bounds(&c.points_um);
                    if p[0] < b[0] || p[1] < b[1] || p[0] > b[2] || p[1] > b[3] {
                        return false;
                    }
                }
                CropKind::Polygon if c.points_um.len() >= 3 => {
                    let mut inside = false;
                    let mut edge = false;
                    for (a, b) in c
                        .points_um
                        .iter()
                        .zip(c.points_um.iter().cycle().skip(1))
                        .take(c.points_um.len())
                    {
                        edge |= segment_distance(p, *a, *b) <= 1e-8;
                        if (a[1] > p[1]) != (b[1] > p[1])
                            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
                        {
                            inside = !inside;
                        }
                    }
                    if !inside && !edge {
                        return false;
                    }
                }
                _ => (),
            }
        }
        true
    }
    /// Depth inside the union of round-capped strokes, in original mount µm.
    fn mask_depth(&self, p: [f64; 2]) -> Option<f64> {
        self.masks
            .iter()
            .filter_map(|m| {
                let first = m.points_um.first()?;
                if m.radius_um <= 0.0 {
                    return None;
                }
                let distance = m
                    .points_um
                    .windows(2)
                    .map(|w| segment_distance(p, w[0], w[1]))
                    .fold((p[0] - first[0]).hypot(p[1] - first[1]), f64::min);
                Some(m.radius_um - distance)
            })
            .reduce(f64::max)
            .filter(|depth| *depth >= 0.0)
    }
    pub fn visible(&self, p: [f64; 2]) -> bool {
        self.in_crop(p) && self.mask_depth(p).is_none()
    }
    /// Grey fill and a hatched inner border, anchored in mount µm in every view.
    pub fn mask_pixel(&self, p: [f64; 2]) -> Option<u8> {
        self.mask_depth(p).map(|depth| {
            let hatch = (p[0] + p[1] + MASK_LINE_UM / 2.0).rem_euclid(MASK_PITCH_UM) < MASK_LINE_UM;
            if depth <= MASK_EDGE_UM && hatch {
                MASK_INK
            } else {
                MASK_BACKGROUND
            }
        })
    }
    pub fn level(&self, v: u8) -> u8 {
        match &self.levels {
            Some(l) if l.white > l.black => {
                (((v as f64 - l.black as f64) * 255.0 / (l.white - l.black) as f64)
                    .round()
                    .clamp(0.0, 255.0)) as u8
            }
            _ => v,
        }
    }
    /// SVG affine order [a,b,c,d,e,f]. Normalise oriented image bounds to (0,0).
    pub fn transform(&self, size: [f64; 2]) -> [f64; 6] {
        let r = self.rotate_deg.to_radians();
        let (s, c) = r.sin_cos();
        let h = if self.flip_h { -1.0 } else { 1.0 };
        let v = if self.flip_v { -1.0 } else { 1.0 };
        let b = self
            .crop
            .as_ref()
            .filter(|c| c.points_um.len() >= 2)
            .map_or([0.0, 0.0, size[0], size[1]], |c| bounds(&c.points_um));
        let centre = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
        let mut m = [c * h, s * h, -s * v, c * v, 0.0, 0.0];
        m[4] = centre[0] - m[0] * centre[0] - m[2] * centre[1];
        m[5] = centre[1] - m[1] * centre[0] - m[3] * centre[1];
        let corners = [[0.0, 0.0], [size[0], 0.0], [0.0, size[1]], size].map(|p| apply(m, p));
        let extent = bounds(&corners);
        m[4] -= extent[0];
        m[5] -= extent[1];
        m
    }
}
pub fn apply(m: [f64; 6], p: [f64; 2]) -> [f64; 2] {
    [
        m[0] * p[0] + m[2] * p[1] + m[4],
        m[1] * p[0] + m[3] * p[1] + m[5],
    ]
}
/// Apply `inner`, then `outer`, using SVG affine order.
pub fn compose(outer: [f64; 6], inner: [f64; 6]) -> [f64; 6] {
    let p = apply(outer, [inner[4], inner[5]]);
    [
        outer[0] * inner[0] + outer[2] * inner[1],
        outer[1] * inner[0] + outer[3] * inner[1],
        outer[0] * inner[2] + outer[2] * inner[3],
        outer[1] * inner[2] + outer[3] * inner[3],
        p[0],
        p[1],
    ]
}
pub fn inverse(m: [f64; 6]) -> [f64; 6] {
    let d = m[0] * m[3] - m[1] * m[2];
    [
        m[3] / d,
        -m[1] / d,
        -m[2] / d,
        m[0] / d,
        (m[2] * m[5] - m[3] * m[4]) / d,
        (m[1] * m[4] - m[0] * m[5]) / d,
    ]
}
pub fn printable_grains(
    _area: &AreaRecord,
    grains: &[Grain],
    edits: Option<&AreaImageEdits>,
) -> Vec<Grain> {
    grains
        .iter()
        .filter(|g| edits.is_none_or(|e| e.visible(g.centroid_um)))
        .cloned()
        .collect()
}
pub fn oriented_grains(grains: &[Grain], m: [f64; 6]) -> Vec<Grain> {
    grains
        .iter()
        .cloned()
        .map(|mut g| {
            g.centroid_um = apply(m, g.centroid_um);
            g.polygon_um = g.polygon_um.iter().map(|p| apply(m, *p)).collect();
            let b = g.bbox_um;
            g.bbox_um = bounds(
                &[[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|p| apply(m, p)),
            );
            g
        })
        .collect()
}
