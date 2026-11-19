//! The project folder format (plan section 5.4, ADR-0003) and the area pipeline.
//!
//! `project.json` is versioned; every automatic step stores its parameters and
//! manual edits are stored as parameters, never as baked pixels.

pub mod pipeline;
pub mod image_edits;
pub use image_edits::{AreaImageEdits, printable_grains};
pub mod session;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::blend::BlendParams;
use crate::mosaic::{Placement, StitchParams};
use crate::photometric::{PhotometricParams, PhotometricResult, ToneParams};
use crate::readers::{AreaMeta, PackageInfo};
use crate::segment::SegmentParams;
use crate::spots::Settings;

pub const SCHEMA_VERSION: u32 = 1;

/// A new UUIDv7 (the scheme UPbDB uses).
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub schema_version: u32,
    pub uuid: Uuid,
    pub name: String,
    pub created: String,
    pub package: PackageRef,
    #[serde(default)]
    pub upbdb: UpbdbLink,
    pub areas: Vec<AreaRecord>,
    #[serde(default)]
    pub settings: Settings,
    /// Laser stage registration per area and the spot-list adapter (phase 11).
    #[serde(default, skip_serializing_if = "crate::laser::LaserRegistration::is_empty")]
    pub laser: crate::laser::LaserRegistration,
    #[serde(default, deserialize_with = "stored_print_edits", skip_serializing_if = "Option::is_none")]
    pub print_sheets: Option<StoredPrintEdits>,
    /// Keys this version does not know are kept verbatim.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Unreadable strip edits must never prevent registered data from loading or saving.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StoredPrintEdits {
    Typed(Box<crate::layout::sheets::PrintEdits>),
    Raw(serde_json::Value),
}
impl From<crate::layout::sheets::PrintEdits> for StoredPrintEdits {
    fn from(edits: crate::layout::sheets::PrintEdits) -> Self { Self::Typed(Box::new(edits)) }
}
// Preserve an explicit JSON null too; only an absent key means no stored edits.
fn stored_print_edits<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<StoredPrintEdits>, D::Error> {
    StoredPrintEdits::deserialize(d).map(Some)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct UpbdbLink {
    pub mount_id: Option<Uuid>,
    /// Atlas bundles published to UPbDB, oldest first (append-only on the server).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub published: Vec<Published>,
}

/// One publish of the atlas bundle (`POST /v1/mounts/{mount_id}/atlas`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Published {
    pub server: String,
    pub mount_id: Option<Uuid>,
    /// The version number the server gave this bundle, when it gave one.
    pub version: Option<u64>,
    /// sha256 of `atlas.json` (which carries the hash of every other file).
    pub atlas_sha256: String,
    /// sha256 of the zip that was posted.
    pub zip_sha256: String,
    pub at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PackageRef {
    /// Path of the package as opened (absolute on desktop, the folder name in the browser).
    pub path: String,
    pub reader: String,
    pub name: String,
    /// How it was opened: "package" (instrument folder), "area" (one area folder on its own) or
    /// "loose" (images without sidecars, scale from `loose`).
    #[serde(default = "crate::readers::default_kind")]
    pub kind: String,
    /// Scale and arrangement the user gave for loose images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loose: Option<crate::readers::loose::LooseSpec>,
    /// Loose images opened by path (desktop): the absolute path of each, so the project reopens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default)]
    pub meta: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TileRef {
    pub file: String,
    pub sha256: String,
    pub pos_um: [f64; 2],
    pub stage_nm: Option<[f64; 2]>,
    pub pixel_size_nm: f64,
    pub w: usize,
    pub h: usize,
}

/// Everything about one area that is needed to reproduce its outputs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AreaRecord {
    pub uuid: Uuid,
    pub id: String,
    pub name: String,
    /// Sample name used in spot names (defaults to the area name).
    #[serde(default)]
    pub sample: String,
    pub pixel_size_nm: f64,
    pub stage_sign: [f64; 2],
    pub tiles: Vec<TileRef>,
    /// Original package indices of blank captures, before any HDR grouping.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_tiles: Vec<usize>,
    #[serde(default)]
    pub params: AreaParams,
    /// Solved placement (drift per tile, pair residuals).
    pub placement: Option<Placement>,
    pub photometric: Option<PhotometricResult>,
    pub mosaic: Option<crate::mosaic::pyramid::MosaicJson>,
    #[serde(default)]
    pub area_meta: BTreeMap<String, String>,
}

/// Parameters of every step for one area.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct AreaParams {
    pub stitch: StitchParams,
    pub photometric: PhotometricParams,
    pub blend: BlendParams,
    pub tone: ToneParams,
    pub segment: SegmentParams,
}

impl Project {
    pub fn new(info: &PackageInfo, path: &str, created: &str) -> Project {
        Project {
            schema_version: SCHEMA_VERSION,
            uuid: new_id(),
            name: info.name.clone(),
            created: created.to_string(),
            package: PackageRef {
                path: path.to_string(),
                reader: info.reader.clone(),
                name: info.name.clone(),
                kind: info.kind.clone(),
                loose: None,
                files: Vec::new(),
                meta: info.meta.clone(),
            },
            upbdb: UpbdbLink::default(),
            areas: info.areas.iter().map(AreaRecord::from_meta).collect(),
            settings: Settings::default(),
            laser: Default::default(),
            print_sheets: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("serialisable")
    }

    pub fn from_json(s: &str) -> crate::Result<Project> {
        let mut p: Project = serde_json::from_str(s).map_err(|e| crate::Error::Invalid(format!("project.json: {e}")))?;
        if p.schema_version > SCHEMA_VERSION {
            return Err(crate::Error::Unsupported(format!("project schema {} is newer than this grainatlas ({SCHEMA_VERSION})", p.schema_version)));
        }
        for area in &mut p.areas {
            if area.area_meta.contains_key("zag.CenterX") {
                for key in crate::ownership::NAVIGATION_KEYS {
                    if !area.area_meta.contains_key(key)
                        && let Some(value) = p.package.meta.get(key)
                    {
                        area.area_meta.insert(key.to_string(), value.clone());
                    }
                }
            }
        }
        Ok(p)
    }

    pub fn area_mut(&mut self, id: &str) -> Option<&mut AreaRecord> {
        self.areas.iter_mut().find(|a| a.id == id)
    }
}

impl AreaRecord {
    pub fn from_meta(a: &AreaMeta) -> AreaRecord {
        AreaRecord {
            uuid: new_id(),
            id: a.id.clone(),
            name: a.name.clone(),
            sample: a.name.clone(),
            pixel_size_nm: a.pixel_size_nm,
            stage_sign: a.stage_sign,
            tiles: a
                .tiles
                .iter()
                .map(|t| TileRef { file: t.file.clone(), sha256: String::new(), pos_um: t.pos_um, stage_nm: t.stage_nm, pixel_size_nm: t.pixel_size_nm, w: t.w, h: t.h })
                .collect(),
            failed_tiles: Vec::new(),
            params: AreaParams::default(),
            placement: None,
            photometric: None,
            mosaic: None,
            area_meta: Self::metadata(a),
        }
    }

    fn metadata(a: &AreaMeta) -> BTreeMap<String, String> {
        let mut meta = a.meta.clone();
        if !meta.contains_key("zag.ABCCHistory.SignalType1")
            && a.tiles.iter().any(|t| crate::segment::signal_from_meta(&t.meta) == Some(crate::segment::Signal::Bse))
        {
            meta.insert("tile.SignalName".into(), "BSE".into());
        }
        meta
    }

    /// Older project records may predate saved sidecar metadata. Fill missing keys from the
    /// opened package so Auto works identically for fresh and reopened areas.
    pub(crate) fn merge_metadata(&mut self, a: &AreaMeta) {
        for (key, value) in Self::metadata(a) { self.area_meta.entry(key).or_insert(value); }
    }

    /// Signal from the area's instrument metadata, with CL as the compatibility fallback.
    pub fn signal(&self) -> crate::segment::Signal {
        crate::segment::signal_from_meta(&self.area_meta).unwrap_or_default()
    }

    /// Mosaic micrometres per pixel.
    pub fn um_per_px(&self) -> f64 {
        self.pixel_size_nm / 1000.0
    }

    /// Stage position (nm) of a mosaic point (um), through the nearest tile's stage position.
    pub fn stage_of(&self, x_um: f64, y_um: f64) -> Option<[f64; 2]> {
        let pl = self.placement.as_ref()?;
        let k = self.um_per_px();
        let mut best: Option<(f64, [f64; 2])> = None;
        for (t, tr) in pl.tiles.iter().zip(&self.tiles) {
            let Some(st) = tr.stage_nm else { continue };
            // Stage position is the tile centre.
            let cx = (t.pos[0] + t.w as f64 / 2.0) * k;
            let cy = (t.pos[1] + t.h as f64 / 2.0) * k;
            let d = (x_um - cx).hypot(y_um - cy);
            let s = [st[0] + self.stage_sign[0] * (x_um - cx) * 1000.0, st[1] + self.stage_sign[1] * (y_um - cy) * 1000.0];
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, s));
            }
        }
        best.map(|b| b.1)
    }
}
