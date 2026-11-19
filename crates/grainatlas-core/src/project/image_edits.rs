//! Brief A integration boundary. Coordinates remain in mount micrometres.
use super::AreaRecord;
use crate::segment::Grain;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AreaImageEdits {
    pub crop: Option<ImageCrop>,
    pub rotate_deg: f64,
    pub flip_h: bool,
    pub flip_v: bool,
    pub levels: Option<ImageLevels>,
    pub masks: Vec<ImageMask>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageCrop {
    pub kind: String,
    pub points_um: Vec<[f64; 2]>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageLevels {
    pub black: u8,
    pub white: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageMask {
    pub points_um: Vec<[f64; 2]>,
    pub radius_um: f64,
}
/// TODO(Brief A): replace the identity implementation with centroid crop/mask
/// filtering and supply the area's saved edits in every print entry point.
pub fn printable_grains(
    _area: &AreaRecord,
    grains: &[Grain],
    _edits: Option<&AreaImageEdits>,
) -> Vec<Grain> {
    grains.to_vec()
}
