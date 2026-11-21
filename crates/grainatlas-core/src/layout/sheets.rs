//! Print sheets for hand spot registration (first used on the Boise State SEM
//! run of 2026-11-03): every owned grain of a mount, at one print scale per mount (by default from its grain sizes),
//! as bands cut straight out of the mosaic.
//!
//! Per sample (top of the mount first), the owned grains of each area are
//! linked into 2-D context clusters. Clusters stay whole whenever they fit a
//! line, and small orphans join a neighbour only when the join is cheap. Oversized clusters
//! are cut at empty gaps where possible, with marked repeated context strips.
//! See "Layout rule: context first" in `docs/print_sheets.md`.
//! Every band is an exact translation of
//! the mosaic, and the registration sidecar ([`PrintRegistration`], frame
//! `print_sheet`) maps each band's sheet rectangle (mm) to its mosaic rectangle
//! (um). That frame is the paper's, not the laser stage's.
//!
//! Each sheet carries a structured header from the instrument metadata, a
//! vector mount map from stage coordinates with this sheet's samples and bands
//! highlighted, laser spot-size circles, a scale bar per band and corner
//! registration marks. Everything is black, white and grey.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::{AreaImage, base64, esc, paper};
use crate::image::Gray;
use crate::ownership::Ownership;
use crate::photometric::enhance::{self, EnhanceParams};
use crate::project::AreaRecord;
use crate::readers::PackageInfo;
use crate::segment::Grain;

pub mod packed;
pub mod arrange;
mod compact;
pub mod edit;
pub use edit::{StripEdit, PrintEdits, EditedPlan, DroppedEdit, Problem, ProblemKind, LeftOff, LeftOffSummary, EditSummary, apply_edits, replay};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Packing {
    #[default]
    Lines,
    Packed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SheetParams {
    /// "tabloid" (default), "letter", "a4", "a3".
    pub paper: String,
    pub landscape: bool,
    /// Placement only; missing JSON uses lines and unknown modes are rejected.
    pub packing: Packing,
    pub margin_mm: f64,
    /// Micrometres of sample per millimetre of paper (8 = 125x). 0, written "auto" in JSON,
    /// picks the scale per mount from its grain sizes ([`auto_scale`]).
    #[serde(with = "scale_serde")]
    pub um_per_mm: f64,
    /// Auto scale: the median printed grain prints at least this large (mm); 0 = by signal
    /// ([`AUTO_TARGET_CL_MM`], or [`AUTO_TARGET_BSE_MM`] when every printed area is BSE).
    pub target_grain_mm: f64,
    /// Margin around the grains of a band (um).
    pub pad_um: f64,
    /// Floor of the automatic context distance (um).
    pub gap_break_um: f64,
    /// Neighbour box gap (um); 0 = max(3 * reference grain size, gap_break_um).
    pub context_um: f64,
    /// Merge smaller components only across a cheap gap (see property 7 in docs/print_sheets.md).
    pub min_group: usize,
    /// Leave post-merge clusters with fewer owners off, except those tied for largest in their area; 0 disables.
    pub min_cluster_grains: usize,
    /// Repeated strip beyond a cut (um); 0 = max(reference grain size, context distance / 3).
    pub overlap_um: f64,
    /// Minimum owners on either side of a cut; 0 uses min_cluster_grains.
    pub min_cut_grains: usize,
    /// Maximum union of repeated strips / band area on either side of a cut.
    pub max_repeated_fraction: f64,
    /// Grains smaller than this (eq. diameter, um) are left off the sheets (debris).
    pub min_grain_um: f64,
    /// Grains larger than this are left off; 0 = no limit.
    pub max_grain_um: f64,
    /// Laser spot diameters drawn at print scale (um).
    pub spot_sizes_um: Vec<f64>,
    pub enhance: EnhanceParams,
    /// Sheet title; the package name when empty.
    pub title: String,
    /// Lab and operator line of the header.
    pub lab: String,
    /// Project folder named in the footer and the registration.
    pub project: String,
    /// File name of the registration sidecar named in the footer.
    pub registration_file: String,
    /// Print time shown in the header.
    pub printed: String,
}

impl Default for SheetParams {
    fn default() -> Self {
        SheetParams {
            paper: "tabloid".into(),
            landscape: true,
            packing: Packing::Lines,
            margin_mm: 8.0,
            um_per_mm: 0.0,
            target_grain_mm: 0.0,
            pad_um: 22.0,
            gap_break_um: 420.0,
            context_um: 0.0,
            min_group: 3,
            min_cluster_grains: 5,
            overlap_um: 0.0,
            min_cut_grains: 0,
            max_repeated_fraction: 1.0 / 3.0,
            min_grain_um: 35.0,
            max_grain_um: 0.0,
            spot_sizes_um: vec![15.0, 20.0, 25.0, 30.0, 35.0, 50.0],
            enhance: EnhanceParams { denoise: true, sharpen: true, ..Default::default() },
            title: String::new(),
            lab: String::new(),
            project: String::new(),
            registration_file: String::new(),
            printed: String::new(),
        }
    }
}

/// Acquisition metadata for the header, gathered from the instrument sidecars.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct SheetMeta {
    pub package: String,
    pub instrument: String,
    pub serial: String,
    /// Detector, e.g. "CL (UVD)".
    pub detector: String,
    pub kv: Option<f64>,
    pub magnification: String,
    pub wd_mm: Option<f64>,
    pub scan: String,
    pub emission_ua: Option<f64>,
    pub tile_px: Option<[usize; 2]>,
    pub um_per_px: f64,
    /// ISO date and first / last capture time (HH:MM).
    pub date: String,
    pub t_start: String,
    pub t_end: String,
    pub areas: usize,
    pub tiles: usize,
    pub comment: String,
    /// Folder the instrument wrote to.
    pub source: String,
}

fn num(s: Option<&String>) -> Option<f64> {
    s.and_then(|v| crate::util::leading_f64(v))
}

/// Header metadata of a package (TM4000 keys; other readers fill what they have).
pub fn sheet_meta(info: &PackageInfo) -> SheetMeta {
    let mut m = SheetMeta { package: info.name.clone(), areas: info.areas.len(), ..Default::default() };
    let tiles: Vec<&crate::readers::TileMeta> = info.areas.iter().flat_map(|a| &a.tiles).collect();
    m.tiles = tiles.len();
    m.um_per_px = info.areas.first().map_or(0.0, |a| a.pixel_size_nm / 1000.0);
    let Some(t0) = tiles.first() else { return m };
    let g = |k: &str| t0.meta.get(k);
    m.instrument = ["Hitachi", g("InstructName").map_or("", |s| s.as_str())].join(" ").trim().to_string();
    if g("InstructName").is_none() {
        m.instrument = info.reader.clone();
    }
    m.serial = g("SerialNumber").cloned().unwrap_or_default();
    m.kv = num(g("AcceleratingVoltage")).map(|v| v / 1000.0);
    m.magnification = g("Magnification").cloned().unwrap_or_default();
    m.wd_mm = num(g("WorkingDistance")).map(|v| v / 1000.0);
    m.scan = g("ScanSpeed").cloned().unwrap_or_default();
    m.emission_ua = num(g("EmissionCurrent")).map(|v| v / 1000.0);
    m.tile_px = Some([t0.w, t0.h]).filter(|s| s[0] > 0);
    m.comment = g("Comment").cloned().unwrap_or_default();
    if let Some(d) = g("Directory") {
        let d = d.trim_end_matches(['\\', '/']);
        // The package folder: two levels above the tile (…\<package>\<area>\).
        let parts: Vec<&str> = d.split(['\\', '/']).collect();
        m.source = parts[..parts.len().saturating_sub(1)].join("\\");
    }
    let signal = info.areas.first().and_then(|a| a.meta.get("zag.ABCCHistory.SignalType1")).cloned().or_else(|| g("SignalName").cloned()).unwrap_or_default();
    m.detector = match signal.as_str() {
        "UVD" => "CL (UVD)".to_string(),
        "" => String::new(),
        s => s.to_string(),
    };
    // Capture times: (date, time) over every tile, compared as yyyy-mm-dd hh:mm:ss.
    let mut stamps: Vec<(String, String)> = tiles
        .iter()
        .filter_map(|t| {
            let (d, tm) = (t.meta.get("Date")?, t.meta.get("Time")?);
            let iso = match d.split('/').collect::<Vec<_>>().as_slice() {
                [mo, da, yr] => format!("{yr}-{mo:0>2}-{da:0>2}"),
                _ => d.clone(),
            };
            Some((iso, tm.clone()))
        })
        .collect();
    stamps.sort();
    if let (Some(a), Some(b)) = (stamps.first(), stamps.last()) {
        m.date = a.0.clone();
        m.t_start = a.1.chars().take(5).collect();
        m.t_end = if b.0 == a.0 { b.1.chars().take(5).collect() } else { format!("{} {}", b.0, b.1.chars().take(5).collect::<String>()) };
    }
    m
}

/// `um_per_mm` in JSON: a number, or "auto" (stored as 0).
pub(super) mod scale_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
        if *v > 0.0 && v.is_finite() { s.serialize_f64(*v) } else { s.serialize_str("auto") }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum V {
            N(f64),
            S(String),
        }
        match V::deserialize(d)? {
            V::N(n) => Ok(n),
            V::S(t) if t.trim().eq_ignore_ascii_case("auto") || t.trim().is_empty() => Ok(0.0),
            V::S(t) => t.trim().parse::<f64>().map_err(|_| serde::de::Error::custom(format!("um_per_mm: a number or \"auto\", not {t:?}"))),
        }
    }
}

/// Round print scales (um per mm, the R10 series): a grain keeps a simple ratio on paper.
pub const ROUND_SCALES: [f64; 17] = [2.0, 2.5, 3.2, 4.0, 5.0, 6.3, 8.0, 10.0, 12.5, 16.0, 20.0, 25.0, 32.0, 40.0, 50.0, 63.0, 80.0];
/// Auto scale target for CL: the median printed grain prints at least this large (mm), so its
/// zoning can be read. The Boise zircon S and L sheets used at the laser printed their median
/// grains at 15 and 24 mm.
pub const AUTO_TARGET_CL_MM: f64 = 15.0;
/// Auto scale target for BSE (apatite, titanite, rutile: no zoning to place spots in). The
/// Boise BSE sheets chosen by hand printed their median grains at 7 to 12 mm.
pub const AUTO_TARGET_BSE_MM: f64 = 10.0;
/// The small grains (10th percentile of the printed ones) still print this large (mm), room
/// for a hand-drawn spot and its number.
pub const MIN_MARK_MM: f64 = 3.0;
/// CL zones about 10 um wide still print 0.4 mm wide: at most 25 um per mm.
pub const ZONE_UM: f64 = 10.0;
pub const MIN_ZONE_MM: f64 = 0.4;
/// Spot-key circles smaller than this are left off the key (mm).
pub const MIN_SPOT_MM: f64 = 1.0;
/// Two key circles closer in diameter than this cannot be told apart; the smaller is left off.
pub const MIN_SPOT_STEP_MM: f64 = 0.3;

/// The scale [`auto_scale`] chose and why.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct AutoScale {
    pub um_per_mm: f64,
    pub median_um: f64,
    pub p10_um: f64,
    /// The median size aimed for (mm).
    #[serde(default)]
    pub target_mm: f64,
    /// What set the scale: "target", "small grains", "zoning" or "spot key".
    pub limit: String,
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let x = q * (sorted.len() - 1) as f64;
    let (i, f) = (x.floor() as usize, x - x.floor());
    sorted[i] + f * (sorted[(i + 1).min(sorted.len() - 1)] - sorted[i])
}

fn round_down(v: f64) -> f64 {
    ROUND_SCALES.iter().rev().copied().find(|&r| r <= v + 1e-9).unwrap_or(ROUND_SCALES[0])
}

/// The print scale of a mount from the eq. diameters of its printed grains: the coarsest round
/// scale at which the median grain prints at least `target_mm`, no coarser than the limits that
/// keep small grains markable (10th percentile at [`MIN_MARK_MM`]), CL zoning readable
/// ([`ZONE_UM`] at [`MIN_ZONE_MM`]) and three spot-key circles drawable ([`MIN_SPOT_MM`]).
/// Paper is spent before any of these is given up.
pub fn auto_scale(diameters_um: &[f64], spot_sizes_um: &[f64], target_mm: f64) -> AutoScale {
    let mut d: Vec<f64> = diameters_um.iter().copied().filter(|v| v.is_finite() && *v > 0.0).collect();
    d.sort_by(f64::total_cmp);
    let (median, p10) = if d.is_empty() { (100.0, 100.0) } else { (quantile(&d, 0.5), quantile(&d, 0.1)) };
    let target = if target_mm > 0.0 { target_mm } else { AUTO_TARGET_CL_MM };
    let mut spots: Vec<f64> = spot_sizes_um.iter().copied().filter(|v| *v > 0.0).collect();
    spots.sort_by(|a, b| b.total_cmp(a));
    let spot_cap = spots.get(2.min(spots.len().saturating_sub(1))).map_or(f64::INFINITY, |s| s / MIN_SPOT_MM);
    let mut k = round_down(median / target);
    let mut limit = "target";
    for (cap, why) in [(p10 / MIN_MARK_MM, "small grains"), (ZONE_UM / MIN_ZONE_MM, "zoning"), (spot_cap, "spot key")] {
        if round_down(cap) < k {
            k = round_down(cap);
            limit = why;
        }
    }
    AutoScale { um_per_mm: k, median_um: median, p10_um: p10, target_mm: target, limit: limit.into() }
}

/// Spot-key sizes drawable at `um_per_mm`: (shown, left off). A circle under [`MIN_SPOT_MM`]
/// or within [`MIN_SPOT_STEP_MM`] of a larger shown circle is left off.
pub fn key_spots(sizes_um: &[f64], um_per_mm: f64) -> (Vec<f64>, Vec<f64>) {
    let mut s: Vec<f64> = sizes_um.iter().copied().filter(|v| *v > 0.0).collect();
    s.sort_by(|a, b| b.total_cmp(a));
    s.dedup();
    let (mut shown, mut off): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    for d in s {
        let mm = d / um_per_mm;
        if mm < MIN_SPOT_MM || shown.last().is_some_and(|&l: &f64| (l - d) / um_per_mm < MIN_SPOT_STEP_MM) {
            off.push(d);
        } else {
            shown.push(d);
        }
    }
    shown.reverse();
    off.reverse();
    (shown, off)
}

/// One area to print: its record (stage geometry) and pixels.
pub struct SheetArea<'a> {
    pub rec: &'a AreaRecord,
    pub img: AreaImage<'a>,
}

/// The `print_sheet` frame: paper sheet ↔ grainatlas mosaic.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrintRegistration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot_labels: Option<Vec<super::labels::Report>>,
    #[serde(default)]
    pub project_uuid: Option<uuid::Uuid>,
    /// Absolute saved-project path on native hosts; empty for an unsaved/browser project.
    #[serde(default)]
    pub project_path: String,
    pub frame: String,
    pub note: String,
    pub package: String,
    pub project: String,
    pub paper: String,
    pub paper_mm: [f64; 2],
    pub um_per_mm: f64,
    /// "auto" or "fixed"; with auto, how the scale was chosen.
    #[serde(default)]
    pub scale: String,
    /// Absent or empty means lines for grain sheets; packed writes "packed".
    /// "whole_area" identifies an uncut area (edited plans will use "edited").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub layout: String,
    /// Whole-area grid spacing and top-left origin, in mosaic um.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_step_um: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_origin_um: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_scale: Option<AutoScale>,
    /// Spot sizes (um) left off the key because they print too small to draw or tell apart.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spot_sizes_left_off_um: Vec<f64>,
    pub sheet_origin: String,
    pub mapping: String,
    /// Exact filename used by the footer and export, including case.
    #[serde(default)]
    pub registration_file: String,
    /// Envelope of all sheet bodies, for older readers. Use sheet_bodies for bounds audits.
    #[serde(default)]
    pub body_rect_mm: [f64; 4],
    #[serde(default)]
    pub sheet_bodies: Vec<SheetBody>,
    /// Detections left in the project but omitted as dating targets.
    #[serde(default)]
    pub strays: Vec<StrayGrain>,
    /// Packed strip blocks, including labels and gap marks, in sheet mm. Omitted for lines
    /// to preserve legacy bytes; legacy line blocks are inferred from standard labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strip_blocks: Vec<SheetBlock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left_off: Vec<LeftOff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<EditSummary>,
    pub bands: Vec<BandRecord>,
}

/// A taken or registered strip rectangle. Sheet numbers are 1-based.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SheetBlock {
    pub sheet: usize,
    pub rect_mm: [f64; 4],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SheetBody {
    pub sheet: usize,
    pub header_mm: f64,
    pub body_rect_mm: [f64; 4],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BandRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<arrange::BandTransform>,
    pub sheet: usize,
    pub sample: String,
    pub area: String,
    /// Area cluster, 1-based in reading order; cuts keep the same number.
    pub row: usize,
    pub grains: Vec<BandGrain>,
    /// Other targets visible in repeated strips. Strays are listed separately in the registration.
    #[serde(default)]
    pub context: Vec<BandGrain>,
    /// Repeated strip rectangles in mosaic um.
    #[serde(default)]
    pub repeated_um: Vec<[f64; 4]>,
    /// Cuts inherited by this band, in order (width, height_line, height_sheet).
    #[serde(default)]
    pub cut_sources: Vec<String>,
    /// x0, y0, x1, y1 on the sheet (mm, top-left origin, y down).
    pub sheet_rect_mm: [f64; 4],
    /// x0, y0, x1, y1 in the area's mosaic (um).
    pub mosaic_rect_um: [f64; 4],
    /// "kept", "stripped", or why it was kept.
    pub background: String,
    /// SVG affine [a,b,c,d,e,f]: sheet mm to original mount µm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_to_mount: Option<[f64; 6]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_crop: Option<crate::project::image_edits::ImageCrop>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StrayGrain {
    pub area: String,
    pub id: uuid::Uuid,
    pub number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub eq_diameter_um: f64,
    pub centroid_um: [f64; 2],
    pub bbox_um: [f64; 4],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BandGrain {
    pub id: uuid::Uuid,
    pub number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct SheetStats {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spot_labels: Option<Vec<super::labels::Report>>,
    pub sheets: usize,
    pub bands: usize,
    pub grains: usize,
    /// Detections left off as duplicates of another area's grain.
    pub duplicates: usize,
    /// Owned grains left off by the size limits.
    pub filtered: usize,
    /// Detections flagged as artifacts (left off).
    pub artifacts: usize,
    pub stripped: usize,
    pub strip_refused: usize,
    pub isolated: usize,
    pub cuts: usize,
    /// Necessary cuts for which neither direction could keep both panels worthwhile.
    pub forced_cuts: usize,
    /// Largest union of repeated strip area / band area.
    pub worst_repeated_fraction: f64,
    pub clusters: usize,
    pub strays: usize,
    /// Band image area / available body area, and mean occupied line width / body width.
    pub fill: SheetFill,
    /// First and later placement header heights, in mm (compact in packed/edited plans).
    pub header_mm: [f64; 2],
    /// Mutually exclusive paper-area budget per sheet; empty epoxy is a subset of band area.
    pub height_budget: Vec<HeightBudget>,
    /// Band images or explicit strip blocks extending beyond the registered sheet body.
    pub bands_outside_body: usize,
    /// Intersecting band-image pairs plus intersecting strip-block pairs (epsilon 1e-7 mm).
    #[serde(skip_serializing_if = "is_zero")]
    pub bands_overlapping: usize,
    /// Printed grains missing an owner band or not whole in their owner band.
    pub grains_cut: usize,
    /// Printed grains listed as owned by more than one band.
    pub grains_twice: usize,
    /// Printed grains whole in a band but absent from both its ownership and context.
    pub grains_unmarked: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct SheetFill {
    pub band_area: f64,
    /// Mean occupied line width; kept as 0 in packed JSON.
    pub line_width: f64,
    /// Union of repeated strip area / band image area, over all bands.
    pub repeated: f64,
    /// Fraction of band image area in rows farther than pad_um from any visible printed grain box.
    pub empty_epoxy: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct HeightBudget {
    pub sheet: usize,
    pub header_mm2: f64,
    pub footer_margins_mm2: f64,
    pub labels_notes_mm2: f64,
    pub band_mm2: f64,
    pub empty_epoxy_mm2: f64,
    pub blank_below_mm: f64,
    pub blank_right_mm2: f64,
    /// Inter-band gaps and space below shorter images on the same line.
    pub other_line_mm2: f64,
    /// Packed body area minus strip blocks; absent/0 for legacy lines budgets.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub blank_mm2: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SheetResult {
    pub pages: Vec<String>,
    pub registration: PrintRegistration,
    pub stats: SheetStats,
    pub audit: Audit,
    pub left_off_summary: LeftOffSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<Problem>,
    pub plan: SheetPlan,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<DroppedEdit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left_off: Vec<LeftOff>,
}

/// Pixel-free cut and placement result. Coordinates are sheet mm, top-left, y down.
/// Render with the same source areas and grains via [`render_plan`]. Placement can move
/// whole strips without changing their bands; optional lines hold only legacy line data.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SheetPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub print_labels: Option<super::labels::Options>,
    pub params: SheetParams,
    /// Read-only annotation snapshot. It never changes registered positions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub print_spots: Option<Vec<crate::spots::Spot>>,
    pub meta: SheetMeta,
    pub paper_mm: [f64; 2],
    pub um_per_mm: f64,
    pub auto_scale: Option<AutoScale>,
    pub sheet_bodies: Vec<SheetBody>,
    pub strips: Vec<SheetStrip>,
    /// None for arrangements without lines. Indices refer to `strips`.
    pub lines: Option<Vec<SheetLine>>,
    /// Retained target ids, in source area/grain order; excludes strays and filters.
    pub targets: BTreeMap<String, Vec<uuid::Uuid>>,
    pub strays: Vec<StrayGrain>,
    /// Counts from filtering and cutting; rendering derives placement/image counts anew.
    pub cut_stats: SheetStats,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_summary: Option<EditSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left_off: Vec<LeftOff>,
}

/// Stable replay identity, independent of page, position and grain iteration order.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct StripKey {
    pub area: String,
    /// SHA-256 of sorted owned grain UUID bytes.
    pub grains_hash: String,
    /// Automatic strips start at 0; later manual splits allocate child counters.
    pub split: u32,
}

/// One movable run: label, translated mosaic bands, gap marks and note allowance.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SheetStrip {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<arrange::PieceTransform>,
    pub key: StripKey,
    /// Zero-based sheet index; registrations remain 1-based.
    pub sheet: usize,
    /// Top-left of the label, in sheet mm.
    pub origin_mm: [f64; 2],
    /// Packed width includes the label, underline and trailing gap mark.
    pub size_mm: [f64; 2],
    pub sample: String,
    pub area: String,
    pub row: usize,
    pub cont: bool,
    pub labelled: bool,
    /// Area/continuation text, separate from the sample name.
    pub label: String,
    pub bands: Vec<SheetBand>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub neighbours: Vec<StripKey>,
    /// A lines-mode trailing gap mark shares its centre with this run's divider.
    /// Cleared when either strip is touched, even if its position is later restored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shares_gap_with: Option<StripKey>,
}

/// A band's source geometry and ownership, enough to recompute repeated context.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SheetBand {
    pub key: StripKey,
    /// Horizontal offset from the strip origin; images start LABEL_MM below it.
    pub offset_mm: f64,
    pub grains: Vec<uuid::Uuid>,
    pub mosaic_rect_um: [f64; 4],
    /// Partition before extending across cuts; internal edges define repeated strips.
    pub core_um: [f64; 4],
    /// Axis (0 = x, 1 = y) and mosaic cut coordinate, in inherited order.
    pub cuts: Vec<(usize, f64)>,
    pub cut_sources: Vec<String>,
    pub gap_after_um: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeated_um: Option<Vec<[f64; 4]>>,
}

/// Lines-only separators and paper budgets, independent of strip origins.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SheetLine {
    pub sheet: usize,
    pub origin_mm: [f64; 2],
    pub band_height_mm: f64,
    /// Rightmost image edge relative to the body left edge.
    pub right_mm: f64,
    pub strips: Vec<usize>,
}

fn strip_key(area: &str, ids: impl Iterator<Item = uuid::Uuid>) -> StripKey {
    use sha2::{Digest, Sha256};
    let mut ids: Vec<_> = ids.collect();
    ids.sort();
    let mut hash = Sha256::new();
    for id in ids { hash.update(id.as_bytes()); }
    StripKey { area: area.into(), grains_hash: format!("{:x}", hash.finalize()), split: 0 }
}

struct CutSheets<'g> {
    pages: Vec<Vec<Vec<Run<'g>>>>,
    printed: Vec<Vec<&'g Grain>>,
    stats: SheetStats,
    strays: Vec<StrayGrain>,
    auto: Option<AutoScale>,
    k: f64,
    page: super::Paper,
}

// Sheet geometry (mm).
const FULL_HEADER_MM: f64 = 50.0;
const COMPACT_HEADER_MM: f64 = 36.0;
/// Placement only. Cutting always uses the legacy header heights above.
pub const PACKED_FIRST_HEADER_MM: f64 = 40.0;
pub const PACKED_LATER_HEADER_MM: f64 = 25.0;
/// Cheap orphan joins bridge at most another quarter of the context distance,
/// and add at most 15% to the host rectangle. Neither bound scales with paper.
const ORPHAN_GAP_FACTOR: f64 = 1.25;
const ORPHAN_GROWTH: f64 = 0.15;
pub(super) const FOOTER_MM: f64 = 5.0;
const LABEL_MM: f64 = 4.6;
const NOTE_MM: f64 = 4.2;
const RUN_GAP_MM: f64 = 7.0;
const PIECE_GAP_MM: f64 = 5.0;
const LABEL_MIN_MM: f64 = 48.0;
const INK: &str = "#1a1a1a";
const GREY: &str = "#6b6b6b";
const LIGHT: &str = "#bdbdbd";

/// A non-empty grain sample wins over the owning area's resolved sample.
fn grain_sample<'a>(g: &'a Grain, area_sample: &'a str) -> &'a str {
    g.sample.as_deref().filter(|s| !s.is_empty()).unwrap_or(area_sample)
}

/// Allow a taller map for area and grain sample labels; cap at the former 66 mm.
fn header_heights(areas: &[SheetArea], grains: &BTreeMap<String, Vec<Grain>>) -> [f64; 2] {
    let labels = areas.iter().flat_map(|a| std::iter::once(a.img.sample).chain(
        grains.get(&a.rec.id).into_iter().flatten().filter_map(|g| g.sample.as_deref().filter(|s| !s.is_empty()))
    )).collect::<std::collections::BTreeSet<_>>().len();
    let map = (19.1 + labels as f64 * 2.7).min(66.0);
    [FULL_HEADER_MM.max(map), COMPACT_HEADER_MM.max(map)]
}

/// Exact union of visible target-box row intervals, expanded by the band pad.
/// A stray or an artifact never makes an empty row count as occupied.
fn empty_rows_um(rect: [f64; 4], printed: &[&Grain], pad: f64) -> f64 {
    let mut rows: Vec<_> = printed.iter().filter(|g| intersects(rect, g.bbox_um)).map(|g| ((g.bbox_um[1] - pad).max(rect[1]), (g.bbox_um[3] + pad).min(rect[3]))).collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut end = rect[1];
    let mut occupied = 0.0;
    for (a, b) in rows {
        occupied += (b - a.max(end)).max(0.0);
        end = end.max(b);
    }
    (rect[3] - rect[1] - occupied).max(0.0)
}

/// Approximate width (mm) of DejaVu Sans text.
fn text_w(s: &str, size: f64, bold: bool) -> f64 {
    s.chars().count() as f64 * size * if bold { 0.66 } else { 0.58 }
}

/// Rows of grains chained along x: a grain joins the row whose last grain is nearest in y
/// within `tol` um. Rows come top to bottom, grains left to right.
pub fn rows_of<'g>(grains: &[&'g Grain], tol: f64) -> Vec<Vec<&'g Grain>> {
    let mut gs: Vec<&Grain> = grains.to_vec();
    gs.sort_by(|a, b| a.centroid_um[0].total_cmp(&b.centroid_um[0]));
    let mut rows: Vec<Vec<&Grain>> = Vec::new();
    for g in gs {
        let y = g.centroid_um[1];
        let best = rows.iter().enumerate().map(|(i, r)| (i, (r.last().expect("row").centroid_um[1] - y).abs())).filter(|(_, d)| *d < tol).min_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((i, _)) => rows[i].push(g),
            None => rows.push(vec![g]),
        }
    }
    let mean_y = |r: &Vec<&Grain>| r.iter().map(|g| g.centroid_um[1]).sum::<f64>() / r.len() as f64;
    rows.sort_by(|a, b| mean_y(a).total_cmp(&mean_y(b)));
    rows
}

/// One cluster band on a sheet line (the drawing keeps the existing run structure).
struct Run<'g> {
    sample: String,
    area: usize,
    row: usize,
    cont: bool,
    /// False when it follows a run of the same area and sample (a divider, no label).
    labelled: bool,
    x: f64,
    end: f64,
    pieces: Vec<Piece<'g>>,
}

fn finish_line<'g>(cur: &mut Vec<Run<'g>>, pg: &mut Vec<Vec<Run<'g>>>, height: &mut f64, k: f64) {
    if !cur.is_empty() {
        *height += LABEL_MM + cur.iter().flat_map(|r| &r.pieces).map(|q| q.h(k)).fold(0.0, f64::max) + NOTE_MM;
        pg.push(std::mem::take(cur));
    }
}

struct Piece<'g> {
    grains: Vec<&'g Grain>,
    x: f64,
    /// Mosaic rectangle (um), snapped to pixels.
    rect: [f64; 4],
    /// Partition before extending across cuts. Its internal edges are cut positions.
    core: [f64; 4],
    cuts: Vec<(usize, f64)>,
    cut_sources: Vec<String>,
    gap_after_um: Option<f64>,
}

impl Piece<'_> {
    fn w(&self, k: f64) -> f64 {
        (self.rect[2] - self.rect[0]) / k
    }
    fn h(&self, k: f64) -> f64 {
        (self.rect[3] - self.rect[1]) / k
    }
}

/// Mosaic rectangle of grains plus `pad`, snapped outwards to whole pixels and clamped.
fn piece_rect(gs: &[&Grain], pad: f64, img: &AreaImage) -> [f64; 4] {
    let u = img.um_per_px;
    let x0 = gs.iter().map(|g| g.bbox_um[0]).fold(f64::INFINITY, f64::min) - pad;
    let y0 = gs.iter().map(|g| g.bbox_um[1]).fold(f64::INFINITY, f64::min) - pad;
    let x1 = gs.iter().map(|g| g.bbox_um[2]).fold(f64::NEG_INFINITY, f64::max) + pad;
    let y1 = gs.iter().map(|g| g.bbox_um[3]).fold(f64::NEG_INFINITY, f64::max) + pad;
    let (px0, py0) = ((x0 / u).floor().max(0.0), (y0 / u).floor().max(0.0));
    let (px1, py1) = ((x1 / u).ceil().min(img.display_size()[0] as f64), (y1 / u).ceil().min(img.display_size()[1] as f64));
    [px0 * u, py0 * u, px1.max(px0 + 1.0) * u, py1.max(py0 + 1.0) * u]
}

fn intersects(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn contains(a: [f64; 4], b: [f64; 4]) -> bool {
    (0..2).all(|d| a[d] <= b[d] + 1e-7 && a[d + 2] >= b[d + 2] - 1e-7)
}

fn box_gap(a: [f64; 4], b: [f64; 4]) -> f64 {
    (a[0] - b[2]).max(b[0] - a[2]).max(a[1] - b[3]).max(b[1] - a[3]).max(0.0)
}

fn reference_size(gs: &[&Grain]) -> f64 {
    let owned: Vec<Grain> = gs.iter().map(|g| (*g).clone()).collect();
    let mut r = crate::segment::filter::reference_size(&owned);
    if !r.is_finite() || r <= 0.0 {
        let mut diameters: Vec<f64> = gs.iter().map(|g| g.eq_diameter_um).filter(|d| d.is_finite() && *d > 0.0).collect();
        diameters.sort_by(f64::total_cmp);
        r = if diameters.is_empty() { 100.0 } else { (diameters[(diameters.len() - 1) / 2] + diameters[diameters.len() / 2]) / 2.0 };
    }
    r
}

struct Cluster<'g> {
    grains: Vec<&'g Grain>,
    rect: [f64; 4],
}

/// Rectangle unions can reach another cluster, so restart after every merge.
fn merge_intersections(clusters: &mut Vec<Cluster>, pad: f64, img: &AreaImage) {
    loop {
        let pair = (0..clusters.len()).find_map(|i| (i + 1..clusters.len()).find(|&j| intersects(clusters[i].rect, clusters[j].rect)).map(|j| (i, j)));
        let Some((i, j)) = pair else { break };
        let other = clusters.remove(j);
        clusters[i].grains.extend(other.grains);
        clusters[i].rect = piece_rect(&clusters[i].grains, pad, img);
    }
}

fn clusters_of<'g>(gs: &[&'g Grain], c: f64, p: &SheetParams, img: &AreaImage, size: [f64; 2]) -> Vec<Cluster<'g>> {
    // Components of the grain-box neighbour graph, independent of input order.
    let mut remaining = gs.to_vec();
    let mut clusters = Vec::new();
    while let Some(g) = remaining.pop() {
        let mut group = vec![g];
        let mut i = 0;
        while i < group.len() {
            let mut j = 0;
            while j < remaining.len() {
                if box_gap(group[i].bbox_um, remaining[j].bbox_um) <= c {
                    group.push(remaining.remove(j));
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        let rect = piece_rect(&group, p.pad_um, img);
        clusters.push(Cluster { grains: group, rect });
    }
    loop {
        let pair = (0..clusters.len()).find_map(|i| {
            if clusters[i].grains.len() >= p.min_group {
                return None;
            }
            (0..clusters.len())
                .filter(|&j| j != i)
                .filter(|&j| {
                    let mut gs = clusters[i].grains.clone();
                    gs.extend(&clusters[j].grains);
                    let r = piece_rect(&gs, p.pad_um, img);
                    let host = clusters[j].rect;
                    let area = |b: [f64; 4]| (b[2] - b[0]) * (b[3] - b[1]);
                    let gap = clusters[i].grains.iter().flat_map(|a| clusters[j].grains.iter().map(move |b| box_gap(a.bbox_um, b.bbox_um))).fold(f64::INFINITY, f64::min);
                    if gap > ORPHAN_GAP_FACTOR * c || area(r) > (1.0 + ORPHAN_GROWTH) * area(host) {
                        return false;
                    }
                    (0..2).any(|d| {
                        let extent = r[d + 2] - r[d];
                        let target = clusters[j].rect[d + 2] - clusters[j].rect[d];
                        r[3 - d] - r[1 - d] <= size[1 - d] && (extent <= size[d] || (target > size[d] && extent - target <= size[d]))
                    })
                })
                .min_by(|&a, &b| box_gap(clusters[i].rect, clusters[a].rect).total_cmp(&box_gap(clusters[i].rect, clusters[b].rect)))
                .map(|j| (i, j))
        });
        let Some((i, j)) = pair else { break };
        let other = clusters.remove(j);
        let i = if j < i { i - 1 } else { i };
        clusters[i].grains.extend(other.grains);
        clusters[i].rect = piece_rect(&clusters[i].grains, p.pad_um, img);
    }
    let d = usize::from(img.display_size()[0] <= img.display_size()[1]);
    clusters.sort_by(|a, b| a.rect[d].total_cmp(&b.rect[d]).then(a.rect[1 - d].total_cmp(&b.rect[1 - d])));
    clusters
}

impl Piece<'_> {
    fn repeated(&self, grains: &[&Grain], pad: f64) -> Vec<[f64; 4]> {
        let mut strips: Vec<_> = self.cuts
            .iter()
            .filter_map(|&(d, at)| {
                let mut r = self.rect;
                if (at - self.core[d]).abs() < 1e-7 {
                    r[d + 2] = at.min(r[d + 2]);
                } else if (at - self.core[d + 2]).abs() < 1e-7 {
                    r[d] = at.max(r[d]);
                } else {
                    return None;
                }
                (r[d + 2] > r[d]).then_some(r)
            })
            .collect();
        // Fitting a slice to its owners can trim a context grain across the
        // other axis, beyond the original seam. Mark that fragment as repeated
        // too; drawing still punches out every owned box plus padding.
        for g in grains.iter().filter(|g| !self.grains.iter().any(|own| own.id == g.id) && intersects(self.rect, g.bbox_um)) {
            let b = g.bbox_um;
            for d in 0..2 {
                for high in [false, true] {
                    let edge = self.rect[if high { d + 2 } else { d }];
                    if b[d] >= edge || b[d + 2] <= edge {
                        continue;
                    }
                    let mut border = self.rect;
                    let other = 1 - d;
                    border[other] = border[other].max(b[other]);
                    border[other + 2] = border[other + 2].min(b[other + 2]);
                    border[d] = edge;
                    border[d + 2] = edge;
                    if strips.iter().any(|&s| contains(s, border)) {
                        continue;
                    }
                    let mut strip = self.rect;
                    strip[other] = strip[other].max(b[other] - pad);
                    strip[other + 2] = strip[other + 2].min(b[other + 2] + pad);
                    if high {
                        strip[d] = strip[d].max(b[d] - pad);
                    } else {
                        strip[d + 2] = strip[d + 2].min(b[d + 2] + pad);
                    }
                    strips.push(strip);
                }
            }
        }
        strips
    }
}

struct CutContext<'a, 'g> {
    r: f64,
    c: f64,
    overlap: f64,
    pad: f64,
    min_group: usize,
    optional_min_group: usize,
    source: &'static str,
    max_repeated_fraction: f64,
    grains: &'a [&'g Grain],
}

/// An empty default strip reaches the nearest box if it lies within half the
/// context distance. Test the actual other-axis crop, including inherited seams.
fn visible_overlap(rect: [f64; 4], d: usize, at: f64, forward: bool, cut: &CutContext) -> f64 {
    let mut strip = rect;
    if forward {
        strip[d] = at;
        strip[d + 2] = (at + cut.overlap).min(rect[d + 2]);
    } else {
        strip[d] = (at - cut.overlap).max(rect[d]);
        strip[d + 2] = at;
    }
    if cut.grains.iter().any(|g| intersects(strip, g.bbox_um)) {
        return cut.overlap;
    }
    let nearest = cut
        .grains
        .iter()
        .filter_map(|g| {
            let b = g.bbox_um;
            if !intersects(rect, b) {
                return None;
            }
            if forward && b[d + 2] > at {
                Some((b[d] - at).max(0.0))
            } else if !forward && b[d] < at {
                Some((at - b[d + 2]).max(0.0))
            } else {
                None
            }
        })
        .min_by(f64::total_cmp);
    match nearest {
        Some(distance) if distance < cut.c / 2.0 => cut.overlap.max((distance + cut.r / 2.0).min(cut.c / 2.0)),
        _ => cut.overlap,
    }
}

/// Try a cut with centroid ownership and outward pixel snapping. Internal edges
/// repeat visible context; crossing owners are extended to include their boxes.
fn split_at<'g>(q: &Piece<'g>, d: usize, at: f64, cut: &CutContext, img: &AreaImage) -> Option<(Piece<'g>, Piece<'g>)> {
    let (left, right): (Vec<_>, Vec<_>) = q.grains.iter().copied().partition(|g| g.centroid_um[d] < at);
    // A vertical slice can have owners only on one side of a later horizontal
    // cut. Small clusters retain this context-only rectangle; the cut search
    // enforces minimum ownership for clusters large enough to form two groups.
    if at <= q.core[d] || at >= q.core[d + 2] {
        return None;
    }
    let u = img.um_per_px;
    let mut lr = q.rect;
    let mut rr = q.rect;
    // Each slice follows its own owners across the other axis. Intersect with
    // the parent so inherited seams and their marked strips stay exact subsets.
    let other = 1 - d;
    for (gs, rect) in [(&left, &mut lr), (&right, &mut rr)] {
        if !gs.is_empty() {
            let owned = piece_rect(gs, cut.pad, img);
            rect[other] = owned[other].max(q.rect[other]);
            rect[other + 2] = owned[other + 2].min(q.rect[other + 2]);
        }
    }
    let left_overlap = visible_overlap(lr, d, at, true, cut);
    let right_overlap = visible_overlap(rr, d, at, false, cut);
    let whole_left = left.iter().map(|g| g.bbox_um[d + 2] + cut.pad).fold(at + left_overlap, f64::max);
    let whole_right = right.iter().map(|g| g.bbox_um[d] - cut.pad).fold(at - right_overlap, f64::min);
    lr[d + 2] = ((whole_left / u).ceil() * u).min(q.rect[d + 2]);
    rr[d] = ((whole_right / u).floor() * u).max(q.rect[d]);
    let mut lc = q.core;
    let mut rc = q.core;
    lc[d + 2] = at;
    rc[d] = at;
    let mut cuts = q.cuts.clone();
    cuts.push((d, at));
    let mut cut_sources = q.cut_sources.clone();
    cut_sources.push(cut.source.into());
    Some((Piece { grains: left, x: 0.0, rect: lr, core: lc, cuts: cuts.clone(), cut_sources: cut_sources.clone(), gap_after_um: None }, Piece { grains: right, x: 0.0, rect: rr, core: rc, cuts, cut_sources, gap_after_um: None }))
}

/// Prefer a gap near the line end, then search the full line. A cut to use
/// spare paper is optional. A necessary cut tries the other axis before
/// minimising the worst panel's ownership deficit and excess repeated fraction.
/// Whole boxes, marked context and outward pixel snapping are never relaxed.
fn cut_piece<'g>(q: &Piece<'g>, d: usize, capacity: f64, full: f64, other_full: f64, cut: &CutContext, img: &AreaImage) -> Option<(Piece<'g>, Piece<'g>, bool)> {
    // Propose the usual gap cut before testing panel worth. A rejected spare-
    // paper cut moves the whole piece, rather than carving a different slice.
    if capacity < full - 1e-7 {
        let proposal = CutContext { min_group: cut.optional_min_group, max_repeated_fraction: f64::INFINITY, ..*cut };
        let parts = cut_in_window(q, d, capacity, full, &proposal, img, CutSearch { window: 0.1, forced: false, other_limit: f64::INFINITY })?;
        return (cut_penalty(&parts, cut) <= 1e-12).then_some((parts.0, parts.1, false));
    }
    for window in [0.1, 1.0] {
        if let Some((a, b)) = cut_in_window(q, d, capacity, full, cut, img, CutSearch { window, forced: false, other_limit: f64::INFINITY }) {
            return Some((a, b, false));
        }
    }
    let other = CutContext { source: if d == 0 { "height_line" } else { "width" }, ..*cut };
    for window in [0.1, 1.0] {
        if let Some((a, b)) = cut_in_window(q, 1 - d, other_full, other_full, &other, img, CutSearch { window, forced: false, other_limit: full }) {
            return Some((a, b, false));
        }
    }
    let primary = cut_in_window(q, d, capacity, full, cut, img, CutSearch { window: 1.0, forced: true, other_limit: f64::INFINITY });
    let alternate = cut_in_window(q, 1 - d, other_full, other_full, &other, img, CutSearch { window: 1.0, forced: true, other_limit: full });
    [primary, alternate].into_iter().flatten()
        .min_by(|a, b| cut_penalty(a, cut).total_cmp(&cut_penalty(b, cut)))
        .map(|(a, b)| (a, b, true))
}

fn repeated_fraction(q: &Piece, cut: &CutContext) -> f64 {
    rect_union_area(&q.repeated(cut.grains, cut.pad)) / rect_area(q.rect)
}

fn cut_penalty(parts: &(Piece, Piece), cut: &CutContext) -> f64 {
    [&parts.0, &parts.1].into_iter().map(|q| {
        let missing = cut.min_group.saturating_sub(q.grains.len()) as f64 / cut.min_group.max(1) as f64;
        missing + (repeated_fraction(q, cut) - cut.max_repeated_fraction).max(0.0)
    }).fold(0.0, f64::max)
}

fn rect_area(r: [f64; 4]) -> f64 {
    (r[2] - r[0]) * (r[3] - r[1])
}

/// Exact rectangle union: intersections of inherited x/y strips count once.
fn rect_union_area(rects: &[[f64; 4]]) -> f64 {
    let mut xs: Vec<_> = rects.iter().flat_map(|r| [r[0], r[2]]).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    xs.windows(2).map(|x| {
        let mut ys: Vec<_> = rects.iter().filter(|r| r[0] < x[1] && r[2] > x[0]).map(|r| (r[1], r[3])).collect();
        ys.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut end = f64::NEG_INFINITY;
        let mut height = 0.0;
        for (a, b) in ys {
            height += (b - a.max(end)).max(0.0);
            end = end.max(b);
        }
        (x[1] - x[0]) * height
    }).sum()
}

struct CutSearch {
    window: f64,
    forced: bool,
    other_limit: f64,
}

fn cut_in_window<'g>(q: &Piece<'g>, d: usize, capacity: f64, full: f64, cut: &CutContext, img: &AreaImage, search: CutSearch) -> Option<(Piece<'g>, Piece<'g>)> {
    let CutSearch { window, forced, other_limit } = search;
    let limit = q.rect[d] + capacity;
    let mut hi = limit.min(q.core[d + 2]);
    let mut window_end = limit;
    let mut feasible_hi = ((limit / img.um_per_px).floor() * img.um_per_px - cut.overlap).min(hi);
    if !forced && cut.min_group > 0 {
        if q.grains.len() < 2 * cut.min_group {
            return None;
        }
        let mut centres: Vec<_> = q.grains.iter().map(|g| g.centroid_um[d]).collect();
        centres.sort_by(f64::total_cmp);
        // Equality belongs to the remainder. Move the whole search window back
        // when the usual line-end cut would leave fewer than min_group owners.
        let tail_limit = centres[centres.len() - cut.min_group];
        if tail_limit < feasible_hi {
            hi = tail_limit;
            feasible_hi = tail_limit;
            window_end = tail_limit;
        }
    }
    let lo = (window_end - window * full).max(q.core[d]);
    if feasible_hi < lo {
        return None;
    }
    let mut intervals: Vec<(f64, f64)> = q.grains.iter().map(|g| (g.bbox_um[d], g.bbox_um[d + 2])).collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut end = q.core[d];
    let mut gaps = Vec::new();
    for (a, b) in intervals {
        let (a0, b0) = (end.max(lo), a.min(hi));
        if b0 > a0 {
            gaps.push((a0, b0));
        }
        end = end.max(b);
    }
    if end.max(lo) < hi {
        gaps.push((end.max(lo), hi));
    }
    let min_gap = cut.r.min(gaps.iter().map(|&(a, b)| b - a).fold(0.0, f64::max));
    gaps.retain(|&(a, b)| b - a >= min_gap);
    gaps.sort_by(|a, b| (b.0 + b.1).total_cmp(&(a.0 + a.1)));
    let fits = |at| {
        let (a, b) = split_at(q, d, at, cut, img)?;
        (!a.grains.is_empty() && !b.grains.is_empty() && a.rect[d + 2] - a.rect[d] <= capacity + 1e-7 && a.rect[3 - d] - a.rect[1 - d] <= other_limit + 1e-7).then_some((a, b))
    };
    let mut best = None;
    let grouped = |parts: &(Piece, Piece)| cut_penalty(parts, cut) <= 1e-12;
    for (a, b) in gaps {
        let at = (a + b) / 2.0;
        if at <= feasible_hi
            && let Some(parts) = fits(at)
        {
            if capacity < full && parts.0.grains.len() < cut.min_group {
                return None;
            }
            if !forced && grouped(&parts) {
                return Some(parts);
            }
            if forced && best.as_ref().is_none_or(|b| cut_penalty(&parts, cut) < cut_penalty(b, cut)) {
                best = Some(parts);
            }
        }
    }
    // Box endpoints are the only places the crossing count changes; centroid
    // endpoints also cover ownership changes when the window crosses a grain.
    let mut candidates = vec![lo, feasible_hi];
    for g in &q.grains {
        candidates.extend([g.bbox_um[d], g.bbox_um[d + 2], g.centroid_um[d]]);
    }
    candidates.retain(|&at| at >= lo && at <= feasible_hi);
    let crossings = |at: f64| q.grains.iter().filter(|g| g.bbox_um[d] < at && g.bbox_um[d + 2] > at).count();
    candidates.sort_by(|a, b| crossings(*a).cmp(&crossings(*b)).then(b.total_cmp(a)));
    for at in candidates {
        if let Some(parts) = fits(at) {
            if capacity < full && parts.0.grains.len() < cut.min_group {
                return None;
            }
            if !forced && grouped(&parts) {
                return Some(parts);
            }
            if forced && best.as_ref().is_none_or(|b| cut_penalty(&parts, cut) < cut_penalty(b, cut)) {
                best = Some(parts);
            }
        }
    }
    best
}

fn area_short(rec: &AreaRecord) -> String {
    match rec.id.split_once('_') {
        Some((n, _)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => n.to_string(),
        _ => rec.name.clone(),
    }
}

/// Lays out the print sheets of a mount. `grains` holds every detection per area id;
/// `ownership` says which are printed.
pub fn sheets(p: &SheetParams, areas: &[SheetArea], grains: &BTreeMap<String, Vec<Grain>>, ownership: &Ownership, meta: &SheetMeta) -> SheetResult {
    let plan = plan_sheets(p, areas, grains, ownership, meta);
    render_plan(&plan, areas, grains, ownership)
}

/// Cut with the existing line/page limits, then place whole strips in the requested mode.
pub fn plan_sheets(p: &SheetParams, areas: &[SheetArea], grains: &BTreeMap<String, Vec<Grain>>, ownership: &Ownership, meta: &SheetMeta) -> SheetPlan {
    let mut plan = place_lines(p, areas, meta, cut_sheets(p, areas, grains, ownership));
    if p.packing == Packing::Packed { place_packed(&mut plan, &[]); }
    plan
}

struct SampleUnit<'g> {
    area: usize,
    grains: Vec<&'g Grain>,
    clusters: Vec<Cluster<'g>>,
    context: (f64, f64),
}

/// Cutting deliberately retains the line-filling loop: later cuts depend on its spare space.
fn cut_sheets<'g>(p: &SheetParams, areas: &[SheetArea], grains: &'g BTreeMap<String, Vec<Grain>>, ownership: &Ownership) -> CutSheets<'g> {
    let base = paper(&p.paper).unwrap_or(super::Paper { w_mm: 279.4, h_mm: 431.8 });
    let page = if p.landscape { super::Paper { w_mm: base.w_mm.max(base.h_mm), h_mm: base.w_mm.min(base.h_mm) } } else { base };
    let m = p.margin_mm;
    let body_w = page.w_mm - 2.0 * m;
    let headers = header_heights(areas, grains);
    let body_h = |pi: usize| page.h_mm - 2.0 * m - headers[usize::from(pi > 0)] - 1.0 - FOOTER_MM;
    let mut stats = SheetStats { header_mm: headers, ..Default::default() };

    // Printed grains per area: owned, within the size limits.
    let mut printed: Vec<Vec<&Grain>> = Vec::new();
    for a in areas {
        let all = grains.get(&a.rec.id).map_or(&[][..], |v| v.as_slice());
        let mut v = Vec::new();
        for g in all {
            if !g.is_grain() {
                stats.artifacts += 1;
            } else if !ownership.is_owned(&a.rec.id, &g.id) {
                stats.duplicates += 1;
            } else if g.eq_diameter_um < p.min_grain_um || (p.max_grain_um > 0.0 && g.eq_diameter_um > p.max_grain_um) {
                stats.filtered += 1;
            } else {
                v.push(g);
            }
        }
        printed.push(v);
    }
    stats.grains = printed.iter().map(|v| v.len()).sum();
    // CL zoning needs a larger grain on paper than a uniform BSE grain does.
    let all_bse = areas.iter().zip(&printed).filter(|(_, v)| !v.is_empty()).all(|(a, _)| {
        a.rec.params.segment.signal.or_else(|| crate::segment::signal_from_meta(&a.rec.area_meta)) == Some(crate::segment::Signal::Bse)
    }) && printed.iter().any(|v| !v.is_empty());
    let target = if p.target_grain_mm > 0.0 { p.target_grain_mm } else if all_bse { AUTO_TARGET_BSE_MM } else { AUTO_TARGET_CL_MM };
    let auto = (!(p.um_per_mm > 0.0 && p.um_per_mm.is_finite()))
        .then(|| auto_scale(&printed.iter().flatten().map(|g| g.eq_diameter_um).collect::<Vec<_>>(), &p.spot_sizes_um, target));
    let k = auto.as_ref().map_or(p.um_per_mm, |a| a.um_per_mm).max(0.5);

    let band_h = body_h(1) - LABEL_MM - NOTE_MM;
    let mut strays = Vec::new();
    // Each unit owns only one area's grains of one effective sample. Context,
    // orphan joins and the largest-cluster exception apply within that unit.
    let mut samples: Vec<(String, Vec<SampleUnit>)> = Vec::new();
    // Preserve area order when mount heights tie, even for areas with no printed grains.
    for a in areas {
        let sample = ownership.sample_of(&a.rec.id).unwrap_or(a.img.sample);
        if !samples.iter().any(|s| s.0 == sample) {
            samples.push((sample.to_string(), Vec::new()));
        }
    }
    for (ai, gs) in printed.iter_mut().enumerate() {
        let fallback = ownership.sample_of(&areas[ai].rec.id).unwrap_or(areas[ai].img.sample);
        let mut units: Vec<(String, Vec<&Grain>)> = Vec::new();
        for &g in gs.iter() {
            let sample = grain_sample(g, fallback);
            match units.iter_mut().find(|u| u.0 == sample) {
                Some(u) => u.1.push(g),
                None => units.push((sample.to_string(), vec![g])),
            }
        }
        for (sample, mut unit) in units {
            let img = &areas[ai].img;
            let r = reference_size(&unit);
            let c = if p.context_um > 0.0 { p.context_um } else { (3.0 * r).max(p.gap_break_um) };
            let mut clusters = clusters_of(&unit, c, p, img, [body_w * k, band_h * k]);
            // Classify disconnected components before rectangle unions can rescue
            // an off-row singleton inside a long, wandering row's bounding box.
            let largest = clusters.iter().map(|q| q.grains.len()).max().unwrap_or(0);
            clusters.retain(|q| {
                if q.grains.len() >= p.min_cluster_grains || q.grains.len() == largest {
                    return true;
                }
                strays.extend(q.grains.iter().map(|g| StrayGrain { area: areas[ai].rec.id.clone(), id: g.id, number: g.number, label: g.label.clone(), eq_diameter_um: g.eq_diameter_um, centroid_um: g.centroid_um, bbox_um: g.bbox_um }));
                false
            });
            merge_intersections(&mut clusters, p.pad_um, img);
            let d = usize::from(img.display_size()[0] <= img.display_size()[1]);
            clusters.sort_by(|a, b| a.rect[d].total_cmp(&b.rect[d]).then(a.rect[1 - d].total_cmp(&b.rect[1 - d])));
            unit.retain(|g| !strays.iter().any(|s| s.area == areas[ai].rec.id && s.id == g.id));
            let work = SampleUnit { area: ai, grains: unit, clusters, context: (r, c) };
            match samples.iter_mut().find(|s| s.0 == sample) {
                Some(s) => s.1.push(work),
                None => samples.push((sample, vec![work])),
            }
        }
        gs.retain(|g| !strays.iter().any(|s| s.area == areas[ai].rec.id && s.id == g.id));
    }
    stats.strays = strays.len();
    stats.grains = printed.iter().map(Vec::len).sum();

    // Samples, top of the mount first (mean "SEM view" y of their printed grains).
    let geo: Vec<Option<crate::ownership::AreaGeometry>> = areas.iter().map(|a| crate::ownership::AreaGeometry::new(a.rec)).collect();
    let view = |ai: usize, g: &Grain| -> Option<[f64; 2]> {
        let s = geo[ai].as_ref()?.to_stage(crate::project::image_edits::apply(crate::project::image_edits::inverse(areas[ai].img.image_transform()),g.centroid_um));
        let sg = areas[ai].rec.stage_sign;
        Some([sg[0] * s[0] / 1000.0, sg[1] * s[1] / 1000.0])
    };
    let top = |units: &Vec<SampleUnit>| {
        let ys: Vec<f64> = units.iter().flat_map(|u| u.grains.iter().filter_map(|g| view(u.area, g)).map(|v| v[1])).collect();
        if ys.is_empty() { f64::INFINITY } else { ys.iter().sum::<f64>() / ys.len() as f64 }
    };
    samples.sort_by(|a, b| top(&a.1).total_cmp(&top(&b.1)));

    // Context clusters → translated bands → lines. Spend a new line before
    // cutting any cluster which could have fitted whole on an empty line.
    let mut pages: Vec<Vec<Vec<Run>>> = Vec::new();
    let mut pg: Vec<Vec<Run>> = Vec::new();
    let mut ph = 0.0;
    let mut cur: Vec<Run> = Vec::new();
    let mut lx = 0.0;
    let mut rows = vec![0; areas.len()];
    for (sample, units) in samples {
        for SampleUnit { area: ai, grains: unit, clusters, context: (r, c) } in units {
            let img = &areas[ai].img;
            let overlap = if p.overlap_um > 0.0 { p.overlap_um } else { r.max(c / 3.0) };
            stats.clusters += clusters.len();
            let label_w = text_w(&sample, 3.2, true) + 6.0 + text_w(&format!("area {} · continued", area_short(areas[ai].rec)), 2.5, false);
            for cluster in clusters {
                rows[ai] += 1;
                let row = rows[ai];
                // Prefer this ownership bound through both axes; the full-line
                // fallback relaxes it only if no fitting grouped cut exists.
                let optional_min_group = if cluster.grains.len() >= 2 * p.min_group { p.min_group } else { 0 };
                let min_group = optional_min_group.max(if p.min_cut_grains == 0 { p.min_cluster_grains } else { p.min_cut_grains });
                let cut = CutContext { r, c, overlap, pad: p.pad_um, min_group, optional_min_group, source: "width", max_repeated_fraction: p.max_repeated_fraction, grains: &unit };
                let initial = Piece { grains: cluster.grains, x: 0.0, rect: cluster.rect, core: cluster.rect, cuts: Vec::new(), cut_sources: Vec::new(), gap_after_um: None };
                // Cut wide clusters first: each width piece can then keep its
                // own height, without making a full-width sparse bottom slice.
                let mut cont = false;
                let mut rest = initial;
                loop {
                    let same = cur.last().is_some_and(|r| r.area == ai && r.sample == sample);
                    let mut start = if cur.is_empty() { 0.0 } else { lx + if same { PIECE_GAP_MM } else { RUN_GAP_MM } };
                    let whole = rest.w(k) <= body_w;
                    let need = if same { rest.w(k) } else { rest.w(k).max(LABEL_MIN_MM.min(label_w)) };
                    if !cur.is_empty() && ((whole && start + need > body_w) || (!whole && body_w - start < 0.4 * body_w)) {
                        finish_line(&mut cur, &mut pg, &mut ph, k);
                        start = 0.0;
                    }
                    let parts = if rest.w(k) > body_w - start { cut_piece(&rest, 0, (body_w - start) * k, body_w * k, (body_h(pages.len()) - LABEL_MM - NOTE_MM) * k, &cut, img) } else { None };
                    if parts.is_none() && start > 0.0 && rest.w(k) > body_w - start {
                        finish_line(&mut cur, &mut pg, &mut ph, k);
                        lx = 0.0;
                        continue;
                    }
                    let (mut q, next) = match parts {
                        Some((left, right, forced)) => {
                            stats.cuts += 1;
                            stats.forced_cuts += usize::from(forced);
                            (left, Some(right))
                        }
                        // An individual grain can exceed the fixed-scale
                        // body. Preserve it and report the overflow in the audit.
                        None => (rest, None),
                    };
                    loop {
                        let available = body_h(pages.len()) - ph - LABEL_MM - NOTE_MM;
                        let full = body_h(pages.len()) - LABEL_MM - NOTE_MM;
                        let mut tail = None;
                        if q.h(k) > available + 1e-7 {
                            if !cur.is_empty() {
                                finish_line(&mut cur, &mut pg, &mut ph, k);
                                start = 0.0;
                                continue;
                            }
                            // Whole clusters which fit an empty body spend a
                            // sheet. Already cut clusters may use the remainder.
                            let parts = if q.h(k) > full || !q.cuts.is_empty() {
                                cut_piece(&q, 1, available * k, full * k, body_w * k, &CutContext { source: if available < full { "height_sheet" } else { "height_line" }, ..cut }, img)
                            } else { None };
                            if let Some((top, bottom, forced)) = parts {
                                stats.forced_cuts += usize::from(forced);
                                q = top;
                                tail = Some(bottom);
                                stats.cuts += 1;
                            } else if !pg.is_empty() {
                                pages.push(std::mem::take(&mut pg));
                                ph = 0.0;
                                start = 0.0;
                                continue;
                            }
                        }
                        q.x = start;
                        let labelled = !cur.last().is_some_and(|r| r.area == ai && r.sample == sample);
                        // A removed inter-cluster stretch retains the existing
                        // // mark and distance. Cuts instead carry repeated strips.
                        if let Some(prev) = cur.last_mut().filter(|r| r.area == ai && r.sample == sample && r.row != row) {
                            let prev = prev.pieces.last_mut().expect("band");
                            prev.gap_after_um = Some(box_gap(prev.rect, q.rect));
                        }
                        let end = start + q.w(k);
                        lx = if labelled { end.max(start + label_w) } else { end };
                        cur.push(Run { sample: sample.clone(), area: ai, row, cont, labelled, x: start, end, pieces: vec![q] });
                        cont = true;
                        if let Some(bottom) = tail {
                            finish_line(&mut cur, &mut pg, &mut ph, k);
                            start = 0.0;
                            q = bottom;
                        } else { break; }
                    }
                    match next {
                        // A height slice can be much narrower than its
                        // original width piece. Let the next cut use the
                        // remaining line instead of forcing a line break.
                        Some(next) => rest = next,
                        None => break,
                    }
                }
            }
        }
    }
    finish_line(&mut cur, &mut pg, &mut ph, k);
    if !pg.is_empty() {
        pages.push(pg);
    }
    CutSheets { pages, printed, stats, strays, auto, k, page }
}

/// Convert line-relative cut coordinates to independently placed strips.
fn place_lines(p: &SheetParams, areas: &[SheetArea], meta: &SheetMeta, cut: CutSheets) -> SheetPlan {
    let CutSheets { pages, printed, stats, strays, auto, k, page } = cut;
    let m = p.margin_mm;
    let mut strips = Vec::new();
    let mut lines = Vec::new();
    let mut bodies = Vec::new();
    for (pi, pg) in pages.into_iter().enumerate() {
        let header_mm = stats.header_mm[usize::from(pi > 0)];
        bodies.push(SheetBody { sheet: pi + 1, header_mm, body_rect_mm: [m, m + header_mm + 1.0, page.w_mm - m, page.h_mm - m - FOOTER_MM] });
        let mut y = m + header_mm + 1.0;
        for ln in pg {
            let bh = ln.iter().flat_map(|r| &r.pieces).map(|q| q.h(k)).fold(0.0, f64::max);
            let right = ln.iter().map(|r| r.end).fold(0.0, f64::max);
            let mut indices = Vec::new();
            for run in ln {
                let area = &areas[run.area].rec.id;
                let key = strip_key(area, run.pieces.iter().flat_map(|q| q.grains.iter().map(|g| g.id)));
                let height = LABEL_MM + run.pieces.iter().map(|q| q.h(k)).fold(0.0, f64::max) + NOTE_MM;
                let label = if run.cont { format!("area {} · continued", area_short(areas[run.area].rec)) } else { format!("area {}", area_short(areas[run.area].rec)) };
                let bands = run.pieces.into_iter().map(|q| SheetBand {
                    key: strip_key(area, q.grains.iter().map(|g| g.id)), offset_mm: q.x - run.x,
                    grains: q.grains.iter().map(|g| g.id).collect(), mosaic_rect_um: q.rect, core_um: q.core,
                    cuts: q.cuts, cut_sources: q.cut_sources, gap_after_um: q.gap_after_um, repeated_um: None,
                }).collect();
                indices.push(strips.len());
                strips.push(SheetStrip { transform: None, key, sheet: pi, origin_mm: [m + run.x, y], size_mm: [run.end - run.x, height], sample: run.sample, area: area.clone(), row: run.row, cont: run.cont, labelled: run.labelled, label, bands, hidden: false, neighbours: Vec::new(), shares_gap_with: None });
            }
            for pair in indices.windows(2) {
                let (a, b) = (&strips[pair[0]], &strips[pair[1]]);
                if a.area == b.area && a.sample == b.sample && !b.labelled && a.bands.last().is_some_and(|b| b.gap_after_um.is_some()) && (a.origin_mm[0] + a.size_mm[0] + PIECE_GAP_MM - b.origin_mm[0]).abs() < 1e-7 {
                    strips[pair[0]].shares_gap_with = Some(b.key.clone());
                }
            }
            lines.push(SheetLine { sheet: pi, origin_mm: [m, y], band_height_mm: bh, right_mm: right, strips: indices });
            y = y + LABEL_MM + bh + NOTE_MM;
        }
    }
    let targets = areas.iter().zip(printed).map(|(a, gs)| (a.rec.id.clone(), gs.iter().map(|g| g.id).collect())).collect();
    SheetPlan { print_labels: None, print_spots: None, params: p.clone(), meta: meta.clone(), paper_mm: [page.w_mm, page.h_mm], um_per_mm: k, auto_scale: auto, sheet_bodies: bodies, strips, lines: Some(lines), targets, strays, cut_stats: stats, edit_summary: None, left_off: Vec::new() }
}

/// Place the plan's strips around separate taken rectangles. The public packer also
/// accepts sizes without a complete plan; its caller omits pinned strips from those sizes.
fn place_packed(plan: &mut SheetPlan, taken: &[SheetBlock]) {
    let p = &plan.params;
    let [w, h] = plan.paper_mm;
    let m = p.margin_mm;
    plan.cut_stats.header_mm = compact::heights(plan);
    let bodies = plan.cut_stats.header_mm.map(|header| [m, m + header + 1.0, w - m, h - m - FOOTER_MM]);
    for strip in &mut plan.strips {
        strip.shares_gap_with = None;
        strip.labelled = true;
        edit::measure(strip, plan.um_per_mm);
    }
    let sizes: Vec<_> = plan.strips.iter().map(|s| s.size_mm).collect();
    let placed = packed::pack_rectangles(&sizes, bodies, taken);
    let count = placed.iter().chain(taken).map(|r| r.sheet).max().unwrap_or(0);
    plan.sheet_bodies = (0..count).map(|i| SheetBody { sheet: i + 1, header_mm: plan.cut_stats.header_mm[usize::from(i > 0)], body_rect_mm: bodies[usize::from(i > 0)] }).collect();
    for (strip, block) in plan.strips.iter_mut().zip(placed) {
        strip.sheet = block.sheet - 1;
        strip.origin_mm = [block.rect_mm[0], block.rect_mm[1]];
    }
    plan.lines = None;
}

fn is_zero(v: &usize) -> bool { *v == 0 }
fn is_zero_f64(v: &f64) -> bool { *v == 0.0 }

/// Render a plan without cutting or placing again. Source areas and target UUIDs must
/// match those used by [`plan_sheets`]; edited-plan validation is a separate later stage.
pub fn render_plan(plan: &SheetPlan, areas: &[SheetArea], grains: &BTreeMap<String, Vec<Grain>>, ownership: &Ownership) -> SheetResult {
    render_plan_with_images(plan, areas, grains, ownership, &|ai, rect| band_png(&areas[ai].img, rect, grains.get(&areas[ai].rec.id).map_or(&[], Vec::as_slice), &plan.params.enhance))
}

/// The provider returns the renderer's PNG and background status; sessions cache it.
pub fn render_plan_with_images(plan: &SheetPlan, areas: &[SheetArea], grains: &BTreeMap<String, Vec<Grain>>, ownership: &Ownership, image: &(impl Fn(usize, [f64; 4]) -> (Vec<u8>, String) + Sync)) -> SheetResult {
    let visible;
    let source_plan = plan;
    let plan = if plan.strips.iter().any(|s| s.hidden) {
        visible = SheetPlan { strips: plan.strips.iter().filter(|s| !s.hidden).cloned().collect(), ..plan.clone() };
        &visible
    } else { plan };
    let (p, meta, k) = (&plan.params, &plan.meta, plan.um_per_mm);
    let page = super::Paper { w_mm: plan.paper_mm[0], h_mm: plan.paper_mm[1] };
    let m = p.margin_mm;
    let body_w = page.w_mm - 2.0 * m;
    let headers = plan.cut_stats.header_mm;
    let body_h = |pi: usize| page.h_mm - 2.0 * m - plan.sheet_bodies[pi].header_mm - 1.0 - FOOTER_MM;
    let auto = plan.auto_scale.clone();
    let (_, spots_off) = key_spots(&p.spot_sizes_um, k);
    let mut stats = plan.cut_stats.clone();
    let printed: Vec<Vec<&Grain>> = areas.iter().map(|a| plan.targets[&a.rec.id].iter().map(|id| grains[&a.rec.id].iter().find(|g| g.id == *id).expect("plan target in source grains")).collect()).collect();
    // Display-only x index, built once rather than scanning every project spot
    // for every band. Records and registration stay in their original frame.
    let mut spot_index:BTreeMap<&str,Vec<(&crate::spots::Spot,[f64;2])>>=BTreeMap::new();
    if let Some(spots)=&plan.print_spots {
        let images:BTreeMap<_,_>=areas.iter().map(|a|(a.rec.id.as_str(),&a.img)).collect();
        for spot in spots {if let Some(image)=images.get(spot.area_id.as_str()) {spot_index.entry(&spot.area_id).or_default().push((spot,arrange::point(image.image_transform(),[spot.x_um,spot.y_um])));}}
        for spots in spot_index.values_mut(){spots.sort_by(|a,b|a.1[0].total_cmp(&b.1[0]));}
    }
    let runs: Vec<Run> = plan.strips.iter().map(|r| {
        let ai = areas.iter().position(|a| a.rec.id == r.area).expect("plan area in source areas");
        let pieces = r.bands.iter().map(|q| Piece {
            grains: q.grains.iter().map(|id| printed[ai].iter().copied().find(|g| g.id == *id).expect("band owner in plan targets")).collect(),
            x: q.offset_mm, rect: q.mosaic_rect_um, core: q.core_um, cuts: q.cuts.clone(), cut_sources: q.cut_sources.clone(), gap_after_um: q.gap_after_um,
        }).collect();
        Run { sample: r.sample.clone(), area: ai, row: r.row, cont: r.cont, labelled: r.labelled, x: 0.0, end: r.size_mm[0], pieces }
    }).collect();
    let shown: Vec<Vec<&Grain>> = areas.iter().enumerate().map(|(ai, a)| printed[ai].iter().copied().filter(|g| plan.strips.iter().filter(|s| s.area == a.rec.id).flat_map(|s| &s.bands).any(|b| b.grains.contains(&g.id))).collect()).collect();
    let geo: Vec<Option<crate::ownership::AreaGeometry>> = areas.iter().map(|a| crate::ownership::AreaGeometry::new(a.rec)).collect();
    let view = |ai: usize, g: &Grain| -> Option<[f64; 2]> {
        let s = geo[ai].as_ref()?.to_stage(crate::project::image_edits::apply(crate::project::image_edits::inverse(areas[ai].img.image_transform()),g.centroid_um));
        let sg = areas[ai].rec.stage_sign;
        Some([sg[0] * s[0] / 1000.0, sg[1] * s[1] / 1000.0])
    };
    stats.isolated = runs
        .iter()
        .map(|r| {
            r.pieces
                .iter()
                .map(|q| {
                    let visible = printed[r.area]
                        .iter()
                        .filter(|g| {
                            let b = g.bbox_um;
                            let centre = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
                            (0..2).all(|d| centre[d] >= q.rect[d] && centre[d] <= q.rect[d + 2])
                        })
                        .count();
                    if visible < 3 { q.grains.len() } else { 0 }
                })
                .sum::<usize>()
        })
        .sum();
    stats.grains = runs.iter().flat_map(|r| &r.pieces).map(|q| q.grains.len()).sum();
    // Band images (in parallel).
    let jobs: Vec<(usize, usize)> = runs.iter().enumerate().flat_map(|(ri, r)| (0..r.pieces.len()).map(move |qi| (ri, qi))).collect();
    let mut image_offsets = vec![0];
    for run in &runs { image_offsets.push(image_offsets.last().unwrap() + run.pieces.len()); }
    let imgs: Vec<(Vec<u8>, String)> = crate::util::par_map(jobs.len(), |j| {
        let (ri, qi) = jobs[j];
        let run = &runs[ri];
        image(run.area, run.pieces[qi].rect)
    });
    stats.bands = imgs.len();
    stats.stripped = imgs.iter().filter(|i| i.1 == "stripped").count();
    stats.strip_refused = imgs.iter().filter(|i| i.1.starts_with("kept:")).count();
    stats.sheets = plan.sheet_bodies.len();
    let line_count = plan.lines.as_ref().map_or(0, Vec::len);
    let image_area: f64 = runs.iter().zip(&plan.strips).map(|(r,s)|r.pieces.iter().map(|q|q.w(k)*q.h(k)).sum::<f64>()*s.transform.unwrap_or_default().scale.powi(2)).sum();
    let line_width: f64 = runs.iter().flat_map(|r| &r.pieces).map(|q| q.w(k)).sum();
    if stats.sheets > 0 {
        stats.fill.band_area = image_area / ((0..stats.sheets).map(body_h).sum::<f64>() * body_w);
    }
    if line_count > 0 {
        stats.fill.line_width = line_width / (line_count as f64 * body_w);
    }

    let mut reg = PrintRegistration {
        spot_labels: None,
        project_uuid: None,
        project_path: String::new(),
        frame: "print_sheet".into(),
        note: "paper sheet <-> grainatlas mosaic; not the laser stage registration".into(),
        package: meta.package.clone(),
        project: p.project.clone(),
        paper: format!("{} {} {} x {} mm", p.paper, if p.landscape { "landscape" } else { "portrait" }, page.w_mm, page.h_mm),
        paper_mm: [page.w_mm, page.h_mm],
        um_per_mm: k,
        scale: if auto.is_some() { "auto" } else { "fixed" }.into(),
        layout: if plan.edit_summary.is_some() { "edited".into() } else if p.packing == Packing::Packed { "packed".into() } else { String::new() },
        grid_step_um: None,
        grid_origin_um: None,
        auto_scale: auto.clone(),
        spot_sizes_left_off_um: spots_off.clone(),
        sheet_origin: "sheet top-left corner, mm, y down".into(),
        mapping: "mosaic_um = mosaic_rect_um[0:2] + (sheet_mm - sheet_rect_mm[0:2]) * um_per_mm".into(),
        registration_file: if p.registration_file.is_empty() { format!("{}_print_registration.json", meta.package) } else { p.registration_file.clone() },
        body_rect_mm: [m, m + headers[usize::from(stats.sheets > 1)] + 1.0, page.w_mm - m, page.h_mm - m - FOOTER_MM],
        sheet_bodies: plan.sheet_bodies.clone(),
        strays: plan.strays.clone(),
        left_off: plan.left_off.clone(),
        edits: plan.edit_summary.clone(),
        strip_blocks: if plan.lines.is_none() { plan.strips.iter().map(edit::strip_block).collect() } else { Vec::new() },
        bands: Vec::new(),
    };
    // The header draws the key that fits this scale and says how the scale was chosen.
    let piece_scales: Vec<_> = plan.strips.iter().map(|s| k/s.transform.unwrap_or_default().scale).collect();
    let mixed = piece_scales.first().is_some_and(|first| piece_scales.iter().any(|v| (v-first).abs()>1e-7));
    let header_k = if mixed { k } else { piece_scales.first().copied().unwrap_or(k) };
    let hp = SheetParams { spot_sizes_um: if mixed { Vec::new() } else { key_spots(&p.spot_sizes_um,header_k).0 }, ..p.clone() };
    let scale_note = if mixed { "Per-piece scale; see each scale bar".into() } else { scale_note(if plan.strips.iter().any(|s|s.transform.is_some()) {None} else {auto.as_ref()}, header_k) };
    let mut svgs = Vec::new();
    let mut j = 0;
    let n = plan.sheet_bodies.len();
    for pi in 0..n {
        let mut pg: Vec<_> = plan.strips.iter().enumerate().filter(|(_, r)| r.sheet == pi).collect();
        if plan.lines.is_none() { pg.sort_by(|a, b| a.1.origin_mm[1].total_cmp(&b.1.origin_mm[1]).then(a.1.origin_mm[0].total_cmp(&b.1.origin_mm[0])).then(a.0.cmp(&b.0))); }
        let mut here: Vec<String> = Vec::new();
        for &(_, r) in &pg {
            if !here.contains(&r.sample) {
                here.push(r.sample.clone());
            }
        }
        let mut s = String::new();
        let (w, h) = (page.w_mm, page.h_mm);
        let _ = write!(
            s,
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}" font-family="DejaVu Sans, sans-serif"><rect width="{w}" height="{h}" fill="white"/>"#
        );
        // Bands on this sheet, for the map.
        let mut band_rects: Vec<(usize, String, [f64; 4])> = Vec::new();
        let header_mm = plan.sheet_bodies[pi].header_mm;
        let mut budget = HeightBudget { sheet: pi + 1, header_mm2: header_mm * body_w, footer_margins_mm2: w * h - body_w * (h - 2.0 * m) + (FOOTER_MM + 1.0) * body_w, ..Default::default() };
        if let Some(lines) = &plan.lines {
            for ln in lines.iter().filter(|ln| ln.sheet == pi) {
                let bh = ln.band_height_mm;
                let band_area: f64 = ln.strips.iter().flat_map(|&ri| &runs[ri].pieces).map(|q| q.w(k) * q.h(k)).sum();
                budget.labels_notes_mm2 += (LABEL_MM + NOTE_MM) * body_w;
                budget.band_mm2 += band_area;
                budget.blank_right_mm2 += (body_w - ln.right_mm).max(0.0) * bh;
                budget.other_line_mm2 += ln.right_mm * bh - band_area;
            }
        }
        if plan.lines.is_none() {
            let mut blocks = 0.0;
            for &(ri, strip) in &pg {
                let scale = strip.transform.unwrap_or_default().scale;
                let images: f64 = runs[ri].pieces.iter().map(|q| q.w(k) * q.h(k)).sum::<f64>()*scale.powi(2);
                let block = if strip.transform.is_some() { let size=arrange::size(strip);size[0]*size[1] } else {strip.size_mm[0] * strip.size_mm[1]};
                blocks += block;
                budget.band_mm2 += images;
                budget.labels_notes_mm2 += strip.size_mm[0] * (LABEL_MM + edit::note_height(strip))*scale.powi(2);
                budget.other_line_mm2 += block - strip.size_mm[0] * (LABEL_MM + edit::note_height(strip))*scale.powi(2) - images;
            }
            budget.blank_mm2 = body_w * body_h(pi) - blocks;
        }
        for &(ri, strip) in &pg {
            let run = &runs[ri];
            if strip.transform.is_some() {
                let mut mat = arrange::matrix(strip);
                mat[4] -= mat[0]*strip.origin_mm[0]+mat[2]*strip.origin_mm[1];
                mat[5] -= mat[1]*strip.origin_mm[0]+mat[3]*strip.origin_mm[1];
                let _ = write!(s, r#"<g data-piece="{}" transform="matrix({} {} {} {} {} {})">"#, esc(&serde_json::to_string(&(&strip.key.area,&strip.key.grains_hash,strip.key.split)).expect("piece key serializes")),mat[0],mat[1],mat[2],mat[3],mat[4],mat[5]);
            }
            let y = strip.origin_mm[1];
            let itop = y + LABEL_MM;
            let a = &areas[run.area];
            for q in &run.pieces {
                budget.empty_epoxy_mm2 += empty_rows_um(q.rect, &printed[run.area], p.pad_um) / k * q.w(k);
            }
            let rx0 = strip.origin_mm[0];
            let rx1 = strip.origin_mm[0] + if run.labelled { run.end.max(20.0) } else { run.end };
            let ly = y + LABEL_MM - 1.5;
            if run.labelled {
                let _ = write!(s, r#"<rect x="{rx0:.2}" y="{:.2}" width="2.6" height="2.6" rx="0.4" fill="{INK}"/>"#, ly - 2.4);
                let _ = write!(s, r#"<text x="{:.2}" y="{ly:.2}" font-size="3.2" font-weight="bold" fill="{INK}">{}</text>"#, rx0 + 3.6, esc(&run.sample));
                let note = &strip.label;
                let _ = write!(s, r#"<text x="{:.2}" y="{ly:.2}" font-size="2.5" fill="{GREY}">{}</text>"#, rx0 + 3.6 + text_w(&run.sample, 3.2, true) + 2.4, esc(note));
            } else {
                // Next row of the same area: a short divider.
                let dx = rx0 - PIECE_GAP_MM / 2.0;
                let _ = write!(s, r#"<path d="M{dx:.2} {:.2} V{:.2}" stroke="{INK}" stroke-width="0.35"/>"#, ly - 2.0, ly + 1.1);
            }
            let _ = write!(s, r#"<path d="M{:.2} {:.2} H{rx1:.2}" stroke="{INK}" stroke-width="0.35"/>"#, if run.labelled { rx0 } else { rx0 - PIECE_GAP_MM / 2.0 }, ly + 1.1);
            for (qi, q) in run.pieces.iter().enumerate() {
                let (png, bg) = &imgs[image_offsets[ri] + qi];
                j += 1;
                let (px, pw, ph) = (strip.origin_mm[0] + q.x, q.w(k), q.h(k));
                let iy = itop;
                if strip.transform.is_some() {
                    let _ = write!(s, r#"<image x="{px}" y="{iy}" width="{pw}" height="{ph}" preserveAspectRatio="none" xlink:href="data:image/png;base64,{}"/>"#, base64(png));
                } else { let _ = write!(s, r#"<image x="{px:.3}" y="{iy:.3}" width="{pw:.3}" height="{ph:.3}" preserveAspectRatio="none" xlink:href="data:image/png;base64,{}"/>"#, base64(png)); }
                if strip.transform.is_none() { image_axes(&mut s, &a.img, [px,iy,px+pw,iy+ph]); }
                let repeated = strip.bands[qi].repeated_um.clone().unwrap_or_else(|| q.repeated(&printed[run.area], p.pad_um));
                draw_repeated(&mut s, q, &repeated, [px, iy], k, p.pad_um, j);
                let owned_w = (q.core[2].min(q.rect[2]) - q.core[0].max(q.rect[0])) / k;
                if owned_w > 13.0 || strip.transform.is_some() {
                    let bar = if let Some(t) = strip.transform { arrange::scale_bar(k/t.scale, (owned_w*t.scale*0.25).clamp(0.01,20.0)) } else if owned_w > 28.0 { 100.0 } else { 50.0 };
                    let (bx1, by) = (px + (q.core[2].min(q.rect[2]) - q.rect[0]) / k - 2.2, iy + (q.core[3].min(q.rect[3]) - q.rect[1]) / k - 2.0);
                    let bl = bar / k;
                    let _ = write!(
                        s,
                        r#"<g stroke-linecap="butt"><path d="M{:.3} {by:.3} H{bx1:.3} M{:.3} {:.3} v2 M{bx1:.3} {:.3} v2" stroke="black" stroke-width="0.9"/><path d="M{:.3} {by:.3} H{bx1:.3} M{:.3} {:.3} v2 M{bx1:.3} {:.3} v2" stroke="white" stroke-width="0.45"/><text x="{:.3}" y="{:.3}" font-size="1.9" font-weight="bold" fill="white" stroke="black" stroke-width="0.35" paint-order="stroke" text-anchor="middle">{} µm</text></g>"#,
                        bx1 - bl,
                        bx1 - bl,
                        by - 1.0,
                        by - 1.0,
                        bx1 - bl,
                        bx1 - bl,
                        by - 1.0,
                        by - 1.0,
                        bx1 - bl / 2.0,
                        by - 1.3,
                        bar
                    );
                }
                // Corner ticks at the band's edges.
                for (cx, cy, dx, dy) in [(px, iy, -1.0, -1.0), (px + pw, iy, 1.0, -1.0), (px, iy + ph, -1.0, 1.0), (px + pw, iy + ph, 1.0, 1.0)] {
                    let _ = write!(s, r#"<path d="M{cx:.3} {cy:.3} h{:.2} M{cx:.3} {cy:.3} v{:.2}" stroke="{GREY}" stroke-width="0.15"/>"#, dx * 1.6, dy * 1.6);
                }
                if let Some(t) = strip.transform {
                    arrange_annotations(&mut s, q, [px, iy], k, t, &a.img);
                }
                if let Some(gap) = q.gap_after_um {
                    let gx = px + pw + PIECE_GAP_MM / 2.0;
                    let _ = write!(
                        s,
                        r#"<text x="{gx:.2}" y="{:.2}" font-size="2.4" fill="{GREY}" text-anchor="middle">//</text><text x="{gx:.2}" y="{:.2}" font-size="1.7" fill="{GREY}" text-anchor="middle">{:.1} mm</text>"#,
                        iy + ph / 2.0,
                        iy + ph / 2.0 + 2.6,
                        gap / 1000.0
                    );
                }
                band_rects.push((run.area, run.sample.clone(), q.rect));
                let context = printed[run.area]
                    .iter()
                    .filter(|g| !q.grains.iter().any(|own| own.id == g.id) && (repeated.iter().any(|r| intersects(*r, g.bbox_um)) || (plan.edit_summary.is_some() && contains(q.rect, g.bbox_um) && plan.strips.iter().flat_map(|s| &s.bands).any(|b| b.grains.contains(&g.id)))))
                    .map(|g| BandGrain { id: g.id, number: g.number, label: g.label.clone() })
                    .collect();
                let transform = strip.transform.map(|_| {
                    let mut t=arrange::band_transform(strip,&strip.bands[qi],k);
                    t.mount_to_paper=crate::project::image_edits::compose(t.mount_to_paper,a.img.image_transform());
                    t.paper_to_mount=arrange::inverse(t.mount_to_paper);
                    t
                });
                let mapping=transform.as_ref().map(|t|t.paper_to_mount).or_else(||a.img.edits.map(|_|paper_to_mount(&a.img,q.rect,[px,iy],k)));
                reg.bands.push(BandRecord {
                    transform,
                    sheet: pi + 1,
                    sample: run.sample.clone(),
                    area: a.rec.id.clone(),
                    row: run.row,
                    grains: q.grains.iter().map(|g| BandGrain { id: g.id, number: g.number, label: g.label.clone() }).collect(),
                    sheet_rect_mm: if strip.transform.is_some() { arrange::bounds(arrange::matrix(strip), [q.x, LABEL_MM, q.x + pw, LABEL_MM + ph]) } else { [px, iy, px + pw, iy + ph] },
                    mosaic_rect_um: q.rect,
                    repeated_um: repeated,
                    cut_sources: q.cut_sources.clone(),
                    context,
                    background: bg.clone(),
                    paper_to_mount: mapping,
                    image_crop: a.img.edits.and_then(|e| e.crop.clone()),
                });
            }
            if !strip.neighbours.is_empty() {
                let note = edit::continuation_note(source_plan, strip);
                let _ = write!(s, r#"<text x="{rx0:.2}" y="{:.2}" font-size="1.9" fill="{GREY}">{}</text>"#, y + strip.size_mm[1] - 0.8, esc(&note));
            }
            if let Some(t)=strip.transform {
                let max_h=run.pieces.iter().map(|q|q.h(k)).fold(0.0,f64::max);
                let mut cx=rx0+2.0;
                let font=2.116667/t.scale;
                let refs=key_spots(&p.spot_sizes_um,k).0;
                for (i,d) in refs.iter().enumerate() {
                    let r=d/k/2.0;let cy=itop+max_h+3.0+r;
                    let label=if i+1==refs.len() {format!("{d} µm")} else {d.to_string()};
                    let _ = write!(s,r#"<circle data-spot-um="{d}" cx="{}" cy="{cy}" r="{r}" fill="none" stroke="black" stroke-width="0.2"/><text x="{}" y="{}" font-size="{font}" fill="black" text-anchor="middle">{label}</text>"#,cx+r,cx+r,cy+r+font);
                    cx+=2.0*r+4.0;
                }
                let _ = write!(s, "</g>");
            }
            if let Some(ln) = plan.lines.as_ref().and_then(|lines| lines.iter().find(|ln| ln.sheet == pi && ln.strips.last() == Some(&ri))) {
                let ny = ln.origin_mm[1] + LABEL_MM + ln.band_height_mm + NOTE_MM - 0.8;
                let _ = write!(s, r##"<path d="M{m} {ny:.2} H{:.2}" stroke="#e3e3e3" stroke-width="0.2" stroke-dasharray="0.6 1.2"/>"##, w - m);
            }
        }
        let bottom = if let Some(lines) = &plan.lines {
            lines.iter().filter(|ln| ln.sheet == pi).map(|ln| ln.origin_mm[1] + LABEL_MM + ln.band_height_mm + NOTE_MM).fold(m + header_mm + 1.0, f64::max)
        } else {
            pg.iter().map(|(_, r)| r.origin_mm[1] + r.size_mm[1]).fold(m + header_mm + 1.0, f64::max)
        };
        if plan.lines.is_some() { budget.blank_below_mm = (page.h_mm - m - FOOTER_MM - bottom).max(0.0); }
        stats.height_budget.push(budget);
        header(&mut s, if plan.lines.is_none() && !plan.strips.iter().any(|s|s.transform.is_some()) { p } else { &hp }, meta, page, pi + 1, n, &here, areas, &shown, ownership, &stats, header_k, &scale_note, &spots_off, header_mm, plan.lines.is_none(), &band_rects);
        mount_map(&mut s, p, page, areas, &shown, &here, &band_rects, ownership, &view, header_mm, if plan.lines.is_none() { Some(if pi == 0 { compact::map_top(p, meta, page, &stats) } else { 6.5 }) } else { None });
        // Footer and corner registration marks.
        let fy = h - m - 2.0;
        let reg_file = &reg.registration_file;
        let proj = if p.project.is_empty() { String::new() } else { format!("grainatlas project {} · ", p.project) };
        // Two baselines fit the existing footer. Keep the filename whole and
        // put the long acquisition path on its own line, also on letter paper.
        let footer = format!("{proj}registration {reg_file} · frame print_sheet");
        let _ = write!(s, r#"<text x="{:.2}" y="{fy:.2}" font-size="2.1" fill="{GREY}">{}</text><text x="{:.2}" y="{fy:.2}" font-size="2.1" fill="{GREY}" text-anchor="end">{} · sheet {}/{n}</text>"#, m + 7.0, esc(&footer), w - m - 7.0, esc(&meta.package), pi + 1);
        if !meta.source.is_empty() {
            let _ = write!(s, r#"<text x="{:.2}" y="{:.2}" font-size="2.1" fill="{GREY}">source {}</text>"#, m + 7.0, fy + 2.4, esc(&meta.source));
        }
        for (cx, cy) in [(m - 3.0, m - 3.0), (w - m + 3.0, m - 3.0), (m - 3.0, h - m + 3.0), (w - m + 3.0, h - m + 3.0)] {
            let _ = write!(
                s,
                r#"<g stroke="{INK}" stroke-width="0.15" fill="none"><circle cx="{cx:.2}" cy="{cy:.2}" r="1.2"/><path d="M{:.2} {cy:.2} h5.2 M{cx:.2} {:.2} v5.2"/></g>"#,
                cx - 2.6,
                cy - 2.6
            );
        }
        s.push_str("</svg>");
        if plan.print_spots.is_some() {
            let mut labels = super::labels::Page::new(plan.sheet_bodies[pi].body_rect_mm);
            for (_,strip) in &pg {
                let area = areas.iter().find(|a| a.rec.id == strip.area).expect("plan area");
                for band in &strip.bands {
                    let tr = arrange::band_transform(strip,band,k).mount_to_paper;
                    let occurrence = serde_json::to_string(&(&band.key.area,&band.key.grains_hash,band.key.split)).expect("band key");
                    let image = super::labels::Image { area:&strip.area, occurrence, crop:band.mosaic_rect_um, display_to_paper:tr, mount_to_display:area.img.image_transform(), edits:area.img.edits, repeated:band.repeated_um.clone().unwrap_or_else(|| {
                        reg.bands.iter().find(|b| b.sheet==pi+1 && b.area==strip.area && b.mosaic_rect_um==band.mosaic_rect_um).map(|b|b.repeated_um.clone()).unwrap_or_default()
                    }) };
                    let candidates=spot_index.get(strip.area.as_str()).map_or(&[][..],Vec::as_slice);
                    let first=candidates.partition_point(|(_,p)|p[0]<band.mosaic_rect_um[0]);
                    let last=candidates.partition_point(|(_,p)|p[0]<=band.mosaic_rect_um[2]);
                    labels.image(&image,candidates[first..last].iter().map(|(spot,_)|*spot));
                    for grain in grains.get(&strip.area).into_iter().flatten() {
                        if intersects(grain.bbox_um,band.mosaic_rect_um) {
                            labels.grain(&image,&grain.polygon_um);
                        }
                    }
                }
            }
            super::labels::decorations(&s,&mut labels);
            // Legacy callers passed text sizes in piece coordinates. Retain their
            // minimum while explicit display options use the upright paper size.
            let options=plan.print_labels.clone().unwrap_or_else(||super::labels::Options { size_mm:plan.strips.iter().filter_map(|s|s.transform).map(|t|2.116667/t.scale).fold(2.4,f64::max),..Default::default() });
            let report=super::labels::render(&mut s,labels,&options,pi+1);
            reg.spot_labels.get_or_insert_with(Vec::new).push(report.clone());
            stats.spot_labels.get_or_insert_with(Vec::new).push(report);
        }
        svgs.push(s);
    }
    if image_area > 0.0 {
        stats.fill.empty_epoxy = stats.height_budget.iter().map(|b| b.empty_epoxy_mm2).sum::<f64>() / image_area;
    }
    let total_band_area = reg.bands.iter().map(|b| rect_area(b.mosaic_rect_um)).sum::<f64>();
    let repeated: Vec<_> = reg.bands.iter().map(|b| rect_union_area(&b.repeated_um)).collect();
    if total_band_area > 0.0 {
        stats.fill.repeated = repeated.iter().sum::<f64>() / total_band_area;
    }
    stats.worst_repeated_fraction = reg.bands.iter().zip(repeated).map(|(b, area)| area / rect_area(b.mosaic_rect_um)).fold(0.0, f64::max);
    let check = audit(&reg, &printed, areas);
    stats.grains_cut = check.cut;
    stats.grains_twice = check.twice;
    stats.grains_unmarked = check.unmarked;
    stats.bands_outside_body = check.outside_body;
    stats.bands_overlapping = check.bands_overlapping;
    if plan.strips.iter().any(|s|s.transform.is_some()) { reg.mapping="Use bands[].transform.paper_to_mount or bands[].paper_to_mount (SVG affine [a,b,c,d,e,f]) for paper mm to original mount µm. crop_um and mosaic_rect_um are in the image display frame; image_crop is in mount µm.".into(); }
    let problems = edit::problems(plan, &reg, &printed, areas);
    SheetResult { pages: svgs, registration: reg, stats, audit: check, left_off_summary: LeftOffSummary { cropped: plan.left_off.iter().filter(|g| g.reason == "cropped").count(), hidden: plan.left_off.iter().filter(|g| g.reason == "hidden").count() }, problems, plan: source_plan.clone(), dropped: Vec::new(), left_off: plan.left_off.clone() }
}

/// Exact ownership audit, independent of how many repeated images show a grain.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Audit {
    pub cut: usize,
    pub twice: usize,
    pub unmarked: usize,
    pub outside_body: usize,
    pub bands_overlapping: usize,
}

/// Legacy same-area line groups share a label and inter-band gap marks/dividers.
/// Audit their envelope as one block: internal gap marks and dividers deliberately
/// share horizontal space, at different heights. Explicit packed blocks stay separate.
fn legacy_blocks(reg: &PrintRegistration, areas: &[SheetArea]) -> Vec<SheetBlock> {
    if !reg.layout.is_empty() && reg.layout != "lines" { return Vec::new(); }
    let mut blocks: Vec<SheetBlock> = Vec::new();
    for (i, b) in reg.bands.iter().enumerate() {
        let r = b.sheet_rect_mm;
        if i.checked_sub(1).is_some_and(|j| reg.bands[j].sheet == b.sheet && reg.bands[j].area == b.area && (reg.bands[j].sheet_rect_mm[1] - r[1]).abs() < 1e-7) {
            let block = blocks.last_mut().expect("previous band block");
            block.rect_mm[2] = block.rect_mm[2].max(r[2]);
            block.rect_mm[3] = block.rect_mm[3].max(r[3] + NOTE_MM);
            continue;
        }
        let cont = reg.bands[..i].iter().any(|q| q.area == b.area && q.row == b.row);
        let area = areas.iter().find(|a| a.rec.id == b.area).map_or_else(|| b.area.split_once('_').map_or(b.area.clone(), |(n, _)| n.into()), |a| area_short(a.rec));
        let label = format!("area {area}{}", if cont { " · continued" } else { "" });
        let right = r[2].max(r[0] + 20.0).max(r[0] + text_w(&b.sample, 3.2, true) + 6.0 + text_w(&label, 2.5, false));
        blocks.push(SheetBlock { sheet: b.sheet, rect_mm: [r[0], r[1] - LABEL_MM, right, r[3] + NOTE_MM] });
    }
    blocks
}

pub fn audit(reg: &PrintRegistration, printed: &[Vec<&Grain>], areas: &[SheetArea]) -> Audit {
    // Older registrations have no body rectangle; at least check the paper.
    let body = if reg.body_rect_mm[2] > reg.body_rect_mm[0] { reg.body_rect_mm } else { [0.0, 0.0, reg.paper_mm[0], reg.paper_mm[1]] };
    let mut result = Audit { outside_body: reg.bands.iter().filter(|b| !contains(reg.sheet_bodies.iter().find(|s| s.sheet == b.sheet).map_or(body, |s| s.body_rect_mm), b.sheet_rect_mm) || b.sheet_rect_mm.iter().any(|v| !v.is_finite())).count(), ..Audit::default() };
    result.outside_body += reg.strip_blocks.iter().filter(|b| !contains(reg.sheet_bodies.iter().find(|s| s.sheet == b.sheet).map_or(body, |s| s.body_rect_mm), b.rect_mm) || b.rect_mm.iter().any(|v| !v.is_finite())).count();
    result.bands_overlapping += reg.strip_blocks.iter().filter(|b| edit::covers_header(reg, b)).count();
    for (i, a) in reg.bands.iter().enumerate() {
        result.bands_overlapping += reg.bands[i + 1..].iter().filter(|b| a.sheet == b.sheet && packed::overlaps(a.sheet_rect_mm, b.sheet_rect_mm)).count();
    }
    let legacy;
    let blocks = if reg.strip_blocks.is_empty() { legacy = legacy_blocks(reg, areas); &legacy } else { &reg.strip_blocks };
    for (i, a) in blocks.iter().enumerate() {
        result.bands_overlapping += blocks[i + 1..].iter().filter(|b| a.sheet == b.sheet && packed::overlaps(a.rect_mm, b.rect_mm)).count();
    }
    for (ai, gs) in printed.iter().enumerate() {
        let bands: Vec<&BandRecord> = reg.bands.iter().filter(|b| b.area == areas[ai].rec.id).collect();
        for g in gs {
            let owners: Vec<_> = bands.iter().filter(|b| b.grains.iter().any(|v| v.id == g.id)).collect();
            if (owners.is_empty() && !reg.left_off.iter().any(|off| off.grain.id == g.id && off.grain.area == areas[ai].rec.id)) || owners.iter().any(|b| !contains(b.mosaic_rect_um, g.bbox_um)) {
                result.cut += 1;
            }
            if owners.len() > 1 {
                result.twice += 1;
            }
            if bands.iter().any(|b| contains(b.mosaic_rect_um, g.bbox_um) && !b.grains.iter().chain(&b.context).any(|v| v.id == g.id)) {
                result.unmarked += 1;
            }
        }
    }
    result
}

/// A mount-axis key for image orientation; scale and paper layout remain independent.
pub(crate) fn image_axes(s: &mut String, img: &AreaImage, rect: [f64;4]) {
    let Some(e)=img.edits else {return;};
    if e.rotate_deg.abs()<1e-8 && !e.flip_h && !e.flip_v {return;}
    let m=img.image_transform();
    let x=rect[0]+4.0; let y=rect[1]+5.0;
    let ex=x+m[2]*2.5; let ey=y+m[3]*2.5;
    let label=format!("mount +y · {:.1}°{}{}",e.rotate_deg,if e.flip_h {" H"} else {""},if e.flip_v {" V"} else {""});
    let _=write!(s,r#"<g class="mount-axis" fill="black" stroke="white" stroke-width="0.6" paint-order="stroke"><path d="M{x:.3} {y:.3} L{ex:.3} {ey:.3}" stroke="black" stroke-width="0.3"/><circle cx="{ex:.3}" cy="{ey:.3}" r="0.35"/><text x="{:.3}" y="{:.3}" font-size="1.6">{label}</text></g>"#,x+3.5,y);
}

/// Compose the inverse image orientation with the band's scale and translation.
pub fn paper_to_mount(img: &AreaImage, rect: [f64;4], paper: [f64;2], k: f64) -> [f64;6] {
    let m=crate::project::image_edits::inverse(img.image_transform());
    let p=crate::project::image_edits::apply(m,[rect[0]-paper[0]*k,rect[1]-paper[1]*k]);
    [m[0]*k,m[1]*k,m[2]*k,m[3]*k,p[0],p[1]]
}

/// One band: the mosaic crop with the display LUT, enhanced as asked, as PNG; and what happened
/// to its background.
pub fn band_png(img: &AreaImage, rect: [f64; 4], grains: &[Grain], e: &EnhanceParams) -> (Vec<u8>, String) {
    let (w, h, px, bg) = band_pixels(img, rect, grains, e);
    (crate::image::encode_png_gray(w, h, &px), bg)
}

/// Unencoded band pixels, also used to separate enhancement and encoding in benchmarks.
pub fn band_pixels(img: &AreaImage, rect: [f64; 4], grains: &[Grain], e: &EnhanceParams) -> (usize, usize, Vec<u8>, String) {
    let u = img.um_per_px;
    let size = img.display_size();
    let (x0,y0)=((rect[0]/u).round() as usize,(rect[1]/u).round() as usize);
    let (x1,y1)=(((rect[2]/u).round() as usize).min(size[0]),((rect[3]/u).round() as usize).min(size[1]));
    let (cw,ch)=(x1.saturating_sub(x0).max(1),y1.saturating_sub(y0).max(1));
    let inv=crate::project::image_edits::inverse(img.image_transform());
    let background=if e.strip { e.strip_grey } else { 255 };
    let mut g=Gray::new(cw,ch);
    for y in 0..ch { for x in 0..cw {
        let p=crate::project::image_edits::apply(inv,[(x0+x) as f64*u+u/2.0,(y0+y) as f64*u+u/2.0]);
        let value=img.sample_mount(p,background);
        g.px[y*cw+x]=value as f32/255.0;
    } }
    let mut g = if e.denoise || e.sharpen { enhance::denoise_sharpen(&g, e) } else { g };
    let mut bg = "kept".to_string();
    if e.strip {
        bg = match enhance::grain_mask(cw, ch, [x0 as f64 * u, y0 as f64 * u], u, grains, e.strip_margin_um) {
            None => "kept: a grain has no closed outline".into(),
            Some(mask) => match enhance::strip_background(&mut g, &mask, e.strip_grey as f32 / 255.0, u) {
                Ok(()) => "stripped".into(),
                Err(_) => "kept: something outside every grain outline".into(),
            },
        };
    }
    if let Some(edits)=img.edits { for y in 0..ch { for x in 0..cw {
        let p=crate::project::image_edits::apply(inv,[(x0+x) as f64*u+u/2.0,(y0+y) as f64*u+u/2.0]);
        if !edits.in_crop(p) || p[0]<0.0 || p[1]<0.0 || p[0]>=img.w as f64*u || p[1]>=img.h as f64*u { g.px[y*cw+x]=background as f32/255.0; }
        else if let Some(value)=edits.mask_pixel(p) { g.px[y*cw+x]=value as f32/255.0; }
    } } }
    (cw, ch, g.to_u8(), bg)
}

/// A single hatch mask unions the strips and punches out all owned boxes plus
/// padding. Overlapping owner boxes stay holes (unlike an even-odd path).
fn draw_repeated(s: &mut String, q: &Piece, strips: &[[f64; 4]], xy: [f64; 2], k: f64, pad: f64, id: usize) {
    let [x, y] = xy;
    if strips.is_empty() {
        return;
    }
    let (w, h) = (q.w(k), q.h(k));
    let sheet = |r: [f64; 4]| [x + (r[0] - q.rect[0]) / k, y + (r[1] - q.rect[1]) / k, (r[2] - r[0]) / k, (r[3] - r[1]) / k];
    let _ = write!(s, r#"<defs><mask id="repeat{id}" maskUnits="userSpaceOnUse" x="{x}" y="{y}" width="{w}" height="{h}"><rect x="{x}" y="{y}" width="{w}" height="{h}" fill="black"/>"#);
    for &r in strips {
        let [a, b, w, h] = sheet(r);
        let _ = write!(s, r#"<rect x="{a}" y="{b}" width="{w}" height="{h}" fill="white"/>"#);
    }
    for g in &q.grains {
        let b = g.bbox_um;
        let [a, b, w, h] = sheet([b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad]);
        let _ = write!(s, r#"<rect x="{a}" y="{b}" width="{w}" height="{h}" fill="black"/>"#);
    }
    // A thin black and white diagonal hatch instead of a veil: the strip keeps its own tone (a
    // white veil read as an odd bright panel), and the hatch shows on dark and bright ground.
    let _ = write!(
        s,
        r##"</mask><pattern id="hatch{id}" patternUnits="userSpaceOnUse" width="1.8" height="1.8" patternTransform="rotate(45)"><path d="M0 0.45 H1.8" stroke="black" stroke-width="0.14" opacity="0.7"/><path d="M0 0.6 H1.8" stroke="white" stroke-width="0.14" opacity="0.8"/></pattern></defs><rect class="repeated-hatch" x="{x}" y="{y}" width="{w}" height="{h}" fill="url(#hatch{id})" mask="url(#repeat{id})"/>"##
    );
    for &(d, at) in &q.cuts {
        if at < q.rect[d] || at > q.rect[d + 2] || ((at - q.core[d]).abs() > 1e-7 && (at - q.core[d + 2]).abs() > 1e-7) {
            continue;
        }
        let pos = (at - q.rect[d]) / k;
        let (a, b, c, e) = if d == 0 { (x + pos, y, x + pos, y + h) } else { (x, y + pos, x + w, y + pos) };
        let _ = write!(s, r#"<path d="M{a} {b} L{c} {e}" fill="none" stroke="{GREY}" stroke-width="0.15" stroke-dasharray="1 1"/>"#);
        let direction = if (at - q.core[d]).abs() < 1e-7 { -1.0 } else { 1.0 };
        let (tx, ty) = if d == 0 { (a + direction, y + 2.0) } else { (x + 2.0, b + direction) };
        let _ = write!(s, r#"<text x="{tx}" y="{ty}" font-size="2" font-weight="bold" fill="white" stroke="black" stroke-width="0.3" paint-order="stroke" transform="rotate(90 {tx} {ty})">repeated</text>"#);
    }
}

fn fmt(v: f64, d: usize) -> String {
    format!("{v:.d$}")
}

fn scale_note(auto: Option<&AutoScale>, k: f64) -> String {
    match auto {
        Some(a) => {
            let why = match a.limit.as_str() {
                "target" => String::new(),
                "small grains" => format!(", limited by small grains ({} µm)", fmt(a.p10_um, 0)),
                other => format!(", limited by {other}"),
            };
            format!("auto: median grain {} µm prints {} mm{why}", fmt(a.median_um, 0), fmt(a.median_um / k, 1))
        }
        None => String::new(),
    }
}

fn header_values(p: &SheetParams, meta: &SheetMeta, k: f64, scale_note: &str) -> (Vec<(&'static str, String)>, String) {
    let mut kv: Vec<(&str, String)> = Vec::new();
    let mut inst = meta.instrument.clone();
    if !meta.serial.is_empty() {
        inst += &format!(" · S/N {}", meta.serial);
    }
    kv.push(("Instrument", inst));
    kv.push(("Detector", if meta.detector.is_empty() { "—".into() } else { meta.detector.clone() }));
    let mut beam = Vec::new();
    if let Some(v) = meta.kv {
        beam.push(format!("{} kV", fmt(v, if v.fract() == 0.0 { 0 } else { 1 })));
    }
    if let Some(v) = meta.emission_ua {
        beam.push(format!("emission {} µA", fmt(v, 1)));
    }
    if let Some(v) = meta.wd_mm {
        beam.push(format!("WD {} mm", fmt(v, 1)));
    }
    kv.push(("Beam", if beam.is_empty() { "—".into() } else { beam.join(" · ") }));
    let mut scan = Vec::new();
    if !meta.magnification.is_empty() {
        scan.push(format!("×{}", meta.magnification));
    }
    if !meta.scan.is_empty() {
        scan.push(meta.scan.clone());
    }
    kv.push(("Scan", if scan.is_empty() { "—".into() } else { scan.join(" · ") }));
    let tiles = match meta.tile_px {
        Some([tw, th]) => format!("{tw}×{th} px at {} µm/px · {} areas, {} tiles", fmt(meta.um_per_px, 3), meta.areas, meta.tiles),
        None => format!("{} areas, {} tiles", meta.areas, meta.tiles),
    };
    kv.push(("Tiles", tiles));
    let imaged = if meta.date.is_empty() { "—".into() } else { format!("{} {}–{}", meta.date, meta.t_start, meta.t_end) };
    kv.push(("Imaged", imaged));
    let mut proc = if scale_note.starts_with("Per-piece") { Vec::new() } else { vec![format!("1 mm = {} µm ({}×)", super::fmt_num(k), fmt(1000.0 / k, 0))] };
    if !scale_note.is_empty() {
        proc.push(scale_note.to_string());
    }
    proc.push("Auto tone".to_string());
    if p.enhance.denoise {
        proc.push("NLM denoise".into());
    }
    if p.enhance.sharpen {
        proc.push("unsharp mask".into());
    }
    proc.push(if p.enhance.strip { format!("background stripped to grey {} where safe", p.enhance.strip_grey) } else { "background kept".into() });
    let processing = proc.join(" · ");
    kv.push(("Print", processing.clone()));
    let mut pkg = meta.package.clone();
    if !meta.comment.is_empty() {
        pkg += &format!(" · comment: {}", meta.comment);
    }
    if !p.lab.is_empty() {
        pkg += &format!(" · {}", p.lab);
    }
    kv.push(("Package", pkg));
    (kv, processing)
}

#[allow(clippy::too_many_arguments)]
fn header(
    s: &mut String,
    p: &SheetParams,
    meta: &SheetMeta,
    page: super::Paper,
    sheet: usize,
    sheets: usize,
    here: &[String],
    areas: &[SheetArea],
    printed: &[Vec<&Grain>],
    ownership: &Ownership,
    stats: &SheetStats,
    k: f64,
    scale_note: &str,
    spots_off: &[f64],
    header_mm: f64,
    compact: bool,
    bands: &[(usize, String, [f64; 4])],
) {
    let m = p.margin_mm;
    let map_w = map_width(page, areas, header_mm);
    let text_r = page.w_mm - m - map_w - 6.0;
    let (mut kv, processing) = header_values(p, meta, k, scale_note);
    if sheet > 1 { kv.clear(); }
    if compact {
        compact::draw(s, p, meta, page, sheet, sheets, here, areas, printed, ownership, stats, k, header_mm, &kv, &processing, bands);
        return;
    }
    let title = if p.title.is_empty() { meta.package.clone() } else { p.title.clone() };
    let _ = write!(s, r#"<text x="{m}" y="{:.2}" font-size="6" font-weight="bold" fill="{INK}">{}</text>"#, m + 6.0, esc(&title));
    let _ = write!(s, r#"<text x="{:.2}" y="{:.2}" font-size="4.2" font-weight="bold" fill="{INK}" text-anchor="end">Sheet {sheet} / {sheets}</text>"#, page.w_mm - m, m + 5.6);
    let _ = write!(
        s,
        r#"<text x="{:.2}" y="{:.2}" font-size="2.3" fill="{GREY}" text-anchor="end">{} grains · {} bands · {} duplicates left to their owner area{}{}{}</text>"#,
        page.w_mm - m,
        m + 10.0,
        stats.grains,
        stats.bands,
        stats.duplicates,
        if stats.strays > 0 { format!(" · {} strays left off", stats.strays) } else { String::new() },
        if stats.isolated > 0 { format!(" · {} isolated", stats.isolated) } else { String::new() },
        if p.printed.is_empty() { String::new() } else { format!(" · printed {}", esc(&p.printed)) }
    );
    let col_w = (text_r - m) / 2.0;
    for (i, (key, val)) in kv.iter().enumerate() {
        // Six short values in two columns, then Print and Package on lines of their own.
        let (col, line) = if i < 6 { (i % 2, i / 2) } else { (0, i - 3) };
        let (cx, cy) = (m + col as f64 * col_w, m + 13.5 + line as f64 * 4.2);
        let _ = write!(
            s,
            r#"<text x="{cx:.2}" y="{cy:.2}" font-size="2.1" fill="{GREY}">{}</text><text x="{:.2}" y="{cy:.2}" font-size="2.5" fill="{INK}">{}</text>"#,
            key.to_uppercase(),
            cx + 16.0,
            esc(val)
        );
    }
    if sheet > 1 {
        let _ = write!(s, r#"<text x="{m}" y="{:.2}" font-size="2.5" fill="{INK}">{}</text><text x="{m}" y="{:.2}" font-size="2.1" fill="{GREY}">acquisition details on sheet 1</text>"#, m + 14.0, esc(&processing), m + 18.0);
    }
    // On this sheet.
    let mut x = m;
    let mut y = m + if sheet == 1 { 36.0 } else { 22.0 };
    let _ = write!(s, r#"<text x="{x:.2}" y="{y:.2}" font-size="2.1" fill="{GREY}">ON THIS SHEET</text>"#);
    x += 25.0;
    for sn in here {
        let count = |ai: usize| printed[ai].iter().filter(|g| grain_sample(g, ownership.sample_of(&areas[ai].rec.id).unwrap_or(areas[ai].img.sample)) == sn).count();
        let ais: Vec<usize> = (0..areas.len()).filter(|&ai| count(ai) > 0).collect();
        let ng: usize = ais.iter().map(|&ai| count(ai)).sum();
        let ar: Vec<String> = ais.iter().map(|&ai| area_short(areas[ai].rec)).collect();
        let txt = format!("{ng} grains · area {}", ar.join(" + "));
        let tw = text_w(sn, 2.7, true) + text_w(&txt, 2.5, false) + 9.0;
        if x + tw > text_r && x > m + 25.0 {
            x = m + 25.0;
            y += 4.0;
        }
        let _ = write!(
            s,
            r#"<rect x="{x:.2}" y="{:.2}" width="2.4" height="2.4" rx="0.4" fill="{INK}"/><text x="{:.2}" y="{y:.2}" font-size="2.7" font-weight="bold" fill="{INK}">{}</text><text x="{:.2}" y="{y:.2}" font-size="2.5" fill="{INK}">{}</text>"#,
            y - 2.1,
            x + 3.4,
            esc(sn),
            x + 4.6 + text_w(sn, 2.7, true),
            esc(&txt)
        );
        x += tw;
    }
    // Laser spot sizes at print scale.
    let sy = m + header_mm - 7.0;
    let _ = write!(s, r#"<text x="{m}" y="{:.2}" font-size="2.1" fill="{GREY}">LASER SPOT Ø AT PRINT SCALE (µm)</text>"#, sy - 1.0);
    let mut cx = m + 52.0;
    for &d in &p.spot_sizes_um {
        let r = d / 2.0 / k;
        let _ = write!(
            s,
            r#"<circle cx="{:.3}" cy="{:.3}" r="{r:.3}" fill="none" stroke="{INK}" stroke-width="0.2"/><text x="{:.3}" y="{:.3}" font-size="2" fill="{GREY}" text-anchor="middle">{}</text>"#,
            cx + r,
            sy - 1.8,
            cx + r,
            sy + r.max(1.0) + 1.0,
            super::fmt_num(d)
        );
        cx += 2.0 * r + 4.0;
    }
    if !spots_off.is_empty() {
        let list = spots_off.iter().map(|d| super::fmt_num(*d)).collect::<Vec<_>>().join(", ");
        let _ = write!(s, r#"<text x="{:.2}" y="{:.2}" font-size="2" fill="{GREY}">{list} µm left off: too small to draw or tell apart at this scale</text>"#, cx + 2.0, sy - 1.0);
    }
    let _ = write!(s, r#"<path d="M{m} {:.2} H{text_r:.2}" stroke="{LIGHT}" stroke-width="0.3"/>"#, m + header_mm - 0.5);
}

/// Area boxes in the SEM view (stage axes signed like the mosaic), um.
fn area_boxes(areas: &[SheetArea]) -> Vec<Option<[f64; 4]>> {
    areas.iter().map(|a| stage_box(a.rec)).collect()
}

/// One area's box on the mount in the SEM view (stage µm, axes signed like the mosaic):
/// `[x0, y0, x1, y1]`, or None when the area has no tiles or a tile has no stage position.
/// The viewer's mount inset ports this (`stageBox` in ui/src/lib/mount-inset.ts); both are
/// checked against a hand-made mount-inset project.
pub fn stage_box(r: &AreaRecord) -> Option<[f64; 4]> {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for t in &r.tiles {
        let st = t.stage_nm?;
        let (u, v) = (r.stage_sign[0] * st[0] / 1000.0, r.stage_sign[1] * st[1] / 1000.0);
        let (hw, hh) = (t.w as f64 * t.pixel_size_nm / 2000.0, t.h as f64 * t.pixel_size_nm / 2000.0);
        b = [b[0].min(u - hw), b[1].min(v - hh), b[2].max(u + hw), b[3].max(v + hh)];
    }
    b[0].is_finite().then_some(b)
}

const MAP_LABEL_MM: f64 = 26.0;

/// Width of the map box: the mount at the full header height plus its labels, within limits.
fn map_width(page: super::Paper, areas: &[SheetArea], header_mm: f64) -> f64 {
    let all: Vec<[f64; 4]> = area_boxes(areas).into_iter().flatten().collect();
    let max = (page.w_mm * 0.42).min(180.0);
    if all.is_empty() {
        return 80.0f64.min(max);
    }
    let w = all.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max) - all.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min);
    let h = all.iter().map(|b| b[3]).fold(f64::NEG_INFINITY, f64::max) - all.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min);
    ((header_mm - 19.5) * w / h.max(1.0) + MAP_LABEL_MM + 6.0).clamp(75.0, max)
}

/// Vector map of the mount's areas in the SEM view (stage axes signed like the mosaic), this
/// sheet's samples dark with their grains as white dots, this sheet's bands outlined.
#[allow(clippy::too_many_arguments)]
fn mount_map(
    s: &mut String,
    p: &SheetParams,
    page: super::Paper,
    areas: &[SheetArea],
    printed: &[Vec<&Grain>],
    here: &[String],
    bands: &[(usize, String, [f64; 4])],
    ownership: &Ownership,
    view: &dyn Fn(usize, &Grain) -> Option<[f64; 2]>,
    header_mm: f64,
    compact_top: Option<f64>,
) {
    let sample_of = |ai: usize| ownership.sample_of(&areas[ai].rec.id).unwrap_or(areas[ai].img.sample);
    let m = p.margin_mm;
    let compact = compact_top.is_some();
    let top = compact_top.unwrap_or(12.0);
    let (bw, bh) = (if compact { compact::map_width(page) } else { map_width(page, areas, header_mm) }, if compact { header_mm - top - 1.5 } else { header_mm - 13.5 });
    let (bx, by) = (page.w_mm - m - bw, m + top);
    if compact { s.push_str(r#"<g data-header="map">"#); }
    let boxes = area_boxes(areas);
    let _ = write!(s, r#"<rect x="{bx:.2}" y="{by:.2}" width="{bw:.2}" height="{bh:.2}" rx="1.5" fill="none" stroke="{LIGHT}" stroke-width="0.3"/>"#);
    let all: Vec<[f64; 4]> = boxes.iter().flatten().copied().collect();
    if all.is_empty() {
        if compact { s.push_str("</g>"); }
        return;
    }
    let (x0, y0) = (all.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min), all.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min));
    let (x1, y1) = (all.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max), all.iter().map(|b| b[3]).fold(f64::NEG_INFINITY, f64::max));
    let label_w = MAP_LABEL_MM;
    let sc = ((bw - label_w - 6.0) / (x1 - x0)).min((bh - 6.0) / (y1 - y0));
    let ox = bx + 3.0 + (bw - label_w - 6.0 - (x1 - x0) * sc) / 2.0;
    let oy = by + 2.0 + (bh - 6.0 - (y1 - y0) * sc) / 2.0;
    let tx = |u: f64| ox + (u - x0) * sc;
    let ty = |v: f64| oy + (v - y0) * sc;
    for (ai, b) in boxes.iter().enumerate() {
        let Some(b) = b else { continue };
        let on = if compact { bands.iter().any(|(j, _, _)| *j == ai) } else if printed[ai].is_empty() { here.iter().any(|sn| sn == sample_of(ai)) } else { printed[ai].iter().any(|g| here.iter().any(|sn| grain_sample(g, sample_of(ai)) == sn)) };
        let _ = write!(
            s,
            r#"<rect x="{:.3}" y="{:.3}" width="{:.3}" height="{:.3}" fill="{}" stroke="{}" stroke-width="0.15"/>"#,
            tx(b[0]),
            ty(b[1]),
            (b[2] - b[0]) * sc,
            (b[3] - b[1]) * sc,
            if on { "#3a3a3a" } else { "#ededed" },
            if on { INK } else { LIGHT }
        );
    }
    // Split a dot path only when the area's grains have different states.
    for (ai, gs) in printed.iter().enumerate() {
        let fallback = sample_of(ai);
        let mut paths: Vec<(bool, String)> = Vec::new();
        for g in gs {
            let on = here.iter().any(|sn| grain_sample(g, fallback) == sn)
                && (!compact || bands.iter().any(|(j, _, _)| *j == ai));
            if let Some(v) = view(ai, g) {
                let i = paths.iter().position(|(state, _)| *state == on).unwrap_or_else(|| { paths.push((on, String::new())); paths.len() - 1 });
                let _ = write!(paths[i].1, "M{:.2} {:.2}h0.01", tx(v[0]), ty(v[1]));
            }
        }
        for (on, d) in paths {
            let _ = write!(s, r#"<path d="{d}" stroke="{}" stroke-width="0.7" stroke-linecap="round"/>"#, if on { "white" } else { "#9a9a9a" });
        }
    }
    // This sheet's bands as white outlines (mosaic corners through the stage).
    for (ai, _, r) in bands {
        let Some(ge) = crate::ownership::AreaGeometry::new(areas[*ai].rec) else { continue };
        let sg = areas[*ai].rec.stage_sign;
        let c: Vec<[f64; 2]> = [[r[0], r[1]], [r[2], r[3]]].iter().map(|&q| ge.to_stage(q)).map(|st| [sg[0] * st[0] / 1000.0, sg[1] * st[1] / 1000.0]).collect();
        let (u0, u1) = (c[0][0].min(c[1][0]), c[0][0].max(c[1][0]));
        let (v0, v1) = (c[0][1].min(c[1][1]), c[0][1].max(c[1][1]));
        let _ = write!(
            s,
            r#"<rect x="{:.3}" y="{:.3}" width="{:.3}" height="{:.3}" fill="none" stroke="white" stroke-width="0.35"/><rect x="{:.3}" y="{:.3}" width="{:.3}" height="{:.3}" fill="none" stroke="black" stroke-width="0.12"/>"#,
            tx(u0),
            ty(v0),
            (u1 - u0) * sc,
            (v1 - v0) * sc,
            tx(u0),
            ty(v0),
            (u1 - u0) * sc,
            (v1 - v0) * sc
        );
    }
    // One label per sample in a column right of the mount, at the sample's mean height (pushed
    // apart where they would touch), with a leader from the sample's right edge.
    let mut labels: Vec<(String, f64, f64, bool)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for (ai, _) in areas.iter().enumerate() {
        let fallback = sample_of(ai);
        for sn in std::iter::once(fallback).chain(printed[ai].iter().map(|g| grain_sample(g, fallback))) {
            if !names.iter().any(|s| s == sn) { names.push(sn.to_string()); }
        }
    }
    for sn in names {
        let bs: Vec<[f64; 4]> = (0..areas.len()).filter(|&j| sample_of(j) == sn).filter_map(|j| boxes[j]).collect();
        let unchanged = (0..areas.len()).all(|ai| printed[ai].iter().all(|g| (grain_sample(g, sample_of(ai)) == sn) == (sample_of(ai) == sn)));
        let (right, mid) = if unchanged {
            if bs.is_empty() { continue; }
            (bs.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max), bs.iter().map(|b| (b[1] + b[3]) / 2.0).sum::<f64>() / bs.len() as f64)
        } else {
            let positions: Vec<_> = (0..areas.len()).flat_map(|ai| printed[ai].iter().filter(|g| grain_sample(g, sample_of(ai)) == sn).filter_map(|g| view(ai, g)).collect::<Vec<_>>()).collect();
            if positions.is_empty() { continue; }
            (positions.iter().map(|v| v[0]).fold(f64::NEG_INFINITY, f64::max), positions.iter().map(|v| v[1]).sum::<f64>() / positions.len() as f64)
        };
        let on = here.contains(&sn);
        if !compact || on { labels.push((sn, tx(right), ty(mid), on)); }
    }
    labels.sort_by(|a, b| a.2.total_cmp(&b.2));
    let gap = if compact { 2.4 } else { 2.7 };
    if compact {
        let capacity = ((bh - 7.0) / gap).floor().max(1.0) as usize;
        if labels.len() > capacity {
            let more = labels.len() - capacity + 1;
            labels.truncate(capacity.saturating_sub(1));
            labels.push((format!("+ {more} more"), tx(x1), by + bh - 6.0, true));
        }
    }
    let mut ys: Vec<f64> = labels.iter().map(|l| l.2.clamp(by + 2.3, by + bh - 6.0)).collect();
    for i in 1..ys.len() {
        ys[i] = ys[i].max(ys[i - 1] + gap);
    }
    // Compress unused gaps from the bottom rather than translating the whole
    // column: a far last area must not push the first label above the map.
    if let Some(last) = ys.last_mut() { *last = last.min(by + bh - 6.0); }
    for i in (0..ys.len().saturating_sub(1)).rev() {
        ys[i] = ys[i].min(ys[i + 1] - gap);
    }
    let col = if compact { bx + bw - MAP_LABEL_MM } else { tx(x1) + 3.0 };
    for ((sn, rx, ry, on), y) in labels.iter().zip(&ys) {
        let _ = write!(
            s,
            r#"<path d="M{:.2} {ry:.2} L{:.2} {y:.2} H{:.2}" fill="none" stroke="{}" stroke-width="0.12"/><text x="{:.2}" y="{:.2}" font-size="{}" {}fill="{}">{}</text>"#,
            rx + 0.3,
            col - 1.2,
            col - 0.4,
            if *on { INK } else { LIGHT },
            col,
            y + 0.8,
            if compact { 1.8 } else if *on { 2.5 } else { 2.2 },
            if *on { r#"font-weight="bold" "# } else { "" },
            if *on { INK } else { GREY },
            esc(&if compact { compact::ellipsis(sn, MAP_LABEL_MM - 2.0, 1.8, *on) } else { sn.clone() })
        );
    }
    if compact {
        // Bound the scale bar even for a mount smaller than 1 mm.
        let length = (1000.0 * sc).min(12.0);
        let note = format!("{} mm · SEM view", fmt(length / sc / 1000.0, 2));
        let _ = write!(s, r#"<path d="M{:.2} {:.2} h{length:.3}" stroke="{GREY}" stroke-width="0.3"/><text x="{:.2}" y="{:.2}" font-size="1.8" fill="{GREY}">{}</text></g>"#, bx + 3.0, by + bh - 2.0, bx + 4.5 + length, by + bh - 1.5, esc(&compact::ellipsis(&note, bw - length - 6.0, 1.8, false)));
        return;
    }
    // 1 mm bar and orientation note.
    let l = 1000.0 * sc;
    let _ = write!(
        s,
        r#"<path d="M{:.2} {:.2} h{l:.3}" stroke="{GREY}" stroke-width="0.4"/><text x="{:.2}" y="{:.2}" font-size="2" fill="{GREY}">1 mm · SEM view · dark: this sheet · boxes: its bands</text>"#,
        bx + 3.0,
        by + bh - 2.2,
        bx + 4.5 + l,
        by + bh - 1.5
    );
}

fn arrange_annotations(s: &mut String, q: &Piece, origin: [f64;2], k: f64, t: arrange::PieceTransform, img: &AreaImage) {
    let [x,y] = origin;
    let font = 2.5_f64.max(2.116667/t.scale);
    for g in &q.grains {
        let gx = x+(g.centroid_um[0]-q.rect[0])/k;
        let gy = y+(g.centroid_um[1]-q.rect[1])/k;
        let label = g.label.clone().unwrap_or_else(|| g.number.to_string());
        let _ = write!(s,r#"<text x="{gx}" y="{gy}" font-size="{font}" fill="white" stroke="black" stroke-width="0.3" paint-order="stroke" text-anchor="middle">{}</text>"#,esc(&label));
    }
    if t.rotation_deg.abs()>1e-7 || img.edits.is_some_and(|e|e.rotate_deg.abs()>1e-7||e.flip_h||e.flip_v) {
        let gx = x+q.w(k)/2.0;
        let m=img.image_transform();
        let (tx,ty)=(gx+6.0*m[2],y+3.0+6.0*m[3]);
        let label=img.edits.map_or_else(||format!("mount +y · {}°",t.rotation_deg),|e|format!("mount +y · paper {}° · image {}°{}{}",t.rotation_deg,e.rotate_deg,if e.flip_h {" H"} else {""},if e.flip_v {" V"} else {""}));
        let _ = write!(s,r#"<g data-orientation="{}" stroke="white" fill="white"><path d="M{gx} {} L{tx} {ty} M{} {} L{tx} {ty} L{} {}" fill="none" stroke-width="0.4"/><text x="{}" y="{}" font-size="{font}" stroke="black" stroke-width="0.25" paint-order="stroke">{}</text></g>"#,t.rotation_deg,y+3.0,tx-2.0*m[2]-1.5*m[0],ty-2.0*m[3]-1.5*m[1],tx-2.0*m[2]+1.5*m[0],ty-2.0*m[3]+1.5*m[1],gx+2.0,y+5.0,esc(&label));
    }
}
