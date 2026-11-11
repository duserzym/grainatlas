//! Print sheets for hand spot registration (first used on the Boise State SEM
//! run of 2026-11-03): every owned grain of a mount, at one print scale per mount (by default from its grain sizes),
//! as bands cut straight out of the mosaic.
//!
//! Per sample (top of the mount first), the owned grains of each area are
//! linked into 2-D context clusters. Clusters stay whole whenever they fit a
//! line, and small orphans join the nearest cluster that fits. Oversized clusters
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SheetParams {
    /// "tabloid" (default), "letter", "a4", "a3".
    pub paper: String,
    pub landscape: bool,
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
    /// Merge smaller clusters into the nearest cluster that fits a line or can absorb another cut.
    pub min_group: usize,
    /// Leave post-merge clusters with fewer owners off, except those tied for largest in their area; 0 disables.
    pub min_cluster_grains: usize,
    /// Repeated strip beyond a cut (um); 0 = max(reference grain size, context distance / 3).
    pub overlap_um: f64,
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
            margin_mm: 8.0,
            um_per_mm: 0.0,
            target_grain_mm: 0.0,
            pad_um: 22.0,
            gap_break_um: 420.0,
            context_um: 0.0,
            min_group: 3,
            min_cluster_grains: 5,
            overlap_um: 0.0,
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
mod scale_serde {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_scale: Option<AutoScale>,
    /// Spot sizes (um) left off the key because they print too small to draw or tell apart.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spot_sizes_left_off_um: Vec<f64>,
    pub sheet_origin: String,
    pub mapping: String,
    /// Body available to band images and line labels, in sheet mm.
    #[serde(default)]
    pub body_rect_mm: [f64; 4],
    /// Detections left in the project but omitted as dating targets.
    #[serde(default)]
    pub strays: Vec<StrayGrain>,
    pub bands: Vec<BandRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BandRecord {
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
    /// x0, y0, x1, y1 on the sheet (mm, top-left origin, y down).
    pub sheet_rect_mm: [f64; 4],
    /// x0, y0, x1, y1 in the area's mosaic (um).
    pub mosaic_rect_um: [f64; 4],
    /// "kept", "stripped", or why it was kept.
    pub background: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StrayGrain {
    pub area: String,
    pub id: uuid::Uuid,
    pub number: u32,
    pub eq_diameter_um: f64,
    pub centroid_um: [f64; 2],
    pub bbox_um: [f64; 4],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BandGrain {
    pub id: uuid::Uuid,
    pub number: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct SheetStats {
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
    pub clusters: usize,
    pub strays: usize,
    /// Band image area / available body area, and mean occupied line width / body width.
    pub fill: SheetFill,
    /// Bands extending beyond the registered sheet body.
    pub bands_outside_body: usize,
    /// Printed grains missing an owner band or not whole in their owner band.
    pub grains_cut: usize,
    /// Printed grains listed as owned by more than one band.
    pub grains_twice: usize,
    /// Printed grains whole in a band but absent from both its ownership and context.
    pub grains_unmarked: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct SheetFill {
    pub band_area: f64,
    pub line_width: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SheetResult {
    pub pages: Vec<String>,
    pub registration: PrintRegistration,
    pub stats: SheetStats,
}

// Sheet geometry (mm).
const HEADER_MM: f64 = 66.0;
const FOOTER_MM: f64 = 5.0;
const LABEL_MM: f64 = 4.6;
const NOTE_MM: f64 = 4.2;
const RUN_GAP_MM: f64 = 7.0;
const PIECE_GAP_MM: f64 = 5.0;
const LABEL_MIN_MM: f64 = 48.0;
const INK: &str = "#1a1a1a";
const GREY: &str = "#6b6b6b";
const LIGHT: &str = "#bdbdbd";

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
    /// False when it follows a run of the same area on the line (a divider, no label).
    labelled: bool,
    x: f64,
    end: f64,
    pieces: Vec<Piece<'g>>,
}

struct Piece<'g> {
    grains: Vec<&'g Grain>,
    x: f64,
    /// Mosaic rectangle (um), snapped to pixels.
    rect: [f64; 4],
    /// Partition before extending across cuts. Its internal edges are cut positions.
    core: [f64; 4],
    cuts: Vec<(usize, f64)>,
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
    let (px1, py1) = ((x1 / u).ceil().min(img.w as f64), (y1 / u).ceil().min(img.h as f64));
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
    merge_intersections(&mut clusters, p.pad_um, img);
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
        merge_intersections(&mut clusters, p.pad_um, img);
    }
    let d = usize::from(img.w <= img.h);
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
    Some((Piece { grains: left, x: 0.0, rect: lr, core: lc, cuts: cuts.clone(), gap_after_um: None }, Piece { grains: right, x: 0.0, rect: rr, core: rc, cuts, gap_after_um: None }))
}

/// Prefer the final 10 % of the line. On a full line, widen the search before
/// relaxing minimum ownership: a small slice still must fit the paper.
/// Whole boxes, repeated context and outward pixel snapping are never relaxed.
fn cut_piece<'g>(q: &Piece<'g>, d: usize, capacity: f64, full: f64, cut: &CutContext, img: &AreaImage) -> Option<(Piece<'g>, Piece<'g>)> {
    if let Some(parts) = cut_in_window(q, d, capacity, full, cut, img, 0.1) {
        return Some(parts);
    }
    if capacity < full - 1e-7 {
        return None;
    }
    if let Some(parts) = cut_in_window(q, d, capacity, full, cut, img, 1.0) {
        return Some(parts);
    }
    let relaxed = CutContext { min_group: 0, ..*cut };
    cut_in_window(q, d, capacity, full, &relaxed, img, 1.0)
}

fn cut_in_window<'g>(q: &Piece<'g>, d: usize, capacity: f64, full: f64, cut: &CutContext, img: &AreaImage, window: f64) -> Option<(Piece<'g>, Piece<'g>)> {
    let limit = q.rect[d] + capacity;
    let mut hi = limit.min(q.core[d + 2]);
    let mut window_end = limit;
    let mut feasible_hi = ((limit / img.um_per_px).floor() * img.um_per_px - cut.overlap).min(hi);
    if cut.min_group > 0 {
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
        (!a.grains.is_empty() && !b.grains.is_empty() && a.rect[d + 2] - a.rect[d] <= capacity + 1e-7).then_some((a, b))
    };
    let grouped = |(a, b): &(Piece, Piece)| a.grains.len() >= cut.min_group && b.grains.len() >= cut.min_group;
    for (a, b) in gaps {
        let at = (a + b) / 2.0;
        if at <= feasible_hi
            && let Some(parts) = fits(at)
        {
            // A small head spends a new line rather than falling back to a
            // different cut just to fill the remaining line width. At full
            // width, continue searching for a cut that keeps both groups.
            if capacity < full && parts.0.grains.len() < cut.min_group {
                return None;
            }
            if grouped(&parts) {
                return Some(parts);
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
            if grouped(&parts) {
                return Some(parts);
            }
        }
    }
    None
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
    let base = paper(&p.paper).unwrap_or(super::Paper { w_mm: 279.4, h_mm: 431.8 });
    let page = if p.landscape { super::Paper { w_mm: base.w_mm.max(base.h_mm), h_mm: base.w_mm.min(base.h_mm) } } else { base };
    let m = p.margin_mm;
    let body_w = page.w_mm - 2.0 * m;
    let body_h = page.h_mm - 2.0 * m - HEADER_MM - 1.0 - FOOTER_MM;
    let mut stats = SheetStats::default();

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
    let (spots_shown, spots_off) = key_spots(&p.spot_sizes_um, k);

    let band_h = body_h - LABEL_MM - NOTE_MM;
    let mut strays = Vec::new();
    let mut area_clusters = Vec::new();
    let mut contexts = Vec::new();
    for (ai, gs) in printed.iter_mut().enumerate() {
        let img = &areas[ai].img;
        let r = reference_size(gs);
        let c = if p.context_um > 0.0 { p.context_um } else { (3.0 * r).max(p.gap_break_um) };
        let mut clusters = clusters_of(gs, c, p, img, [body_w * k, band_h * k]);
        let largest = clusters.iter().map(|q| q.grains.len()).max().unwrap_or(0);
        clusters.retain(|q| {
            if q.grains.len() >= p.min_cluster_grains || q.grains.len() == largest {
                return true;
            }
            strays.extend(q.grains.iter().map(|g| StrayGrain { area: areas[ai].rec.id.clone(), id: g.id, number: g.number, eq_diameter_um: g.eq_diameter_um, centroid_um: g.centroid_um, bbox_um: g.bbox_um }));
            false
        });
        gs.retain(|g| !strays.iter().any(|s| s.area == areas[ai].rec.id && s.id == g.id));
        area_clusters.push(clusters);
        contexts.push((r, c));
    }
    stats.strays = strays.len();
    stats.grains = printed.iter().map(Vec::len).sum();

    // Samples, top of the mount first (mean "SEM view" y of their printed grains).
    let geo: Vec<Option<crate::ownership::AreaGeometry>> = areas.iter().map(|a| crate::ownership::AreaGeometry::new(a.rec)).collect();
    let view = |ai: usize, g: &Grain| -> Option<[f64; 2]> {
        let s = geo[ai].as_ref()?.to_stage(g.centroid_um);
        let sg = areas[ai].rec.stage_sign;
        Some([sg[0] * s[0] / 1000.0, sg[1] * s[1] / 1000.0])
    };
    let sample_of = |ai: usize| ownership.sample_of(&areas[ai].rec.id).unwrap_or(areas[ai].img.sample).to_string();
    let mut samples: Vec<(String, Vec<usize>)> = Vec::new();
    for ai in 0..areas.len() {
        let s = sample_of(ai);
        match samples.iter_mut().find(|x| x.0 == s) {
            Some(x) => x.1.push(ai),
            None => samples.push((s, vec![ai])),
        }
    }
    let top = |ais: &Vec<usize>| {
        let ys: Vec<f64> = ais.iter().flat_map(|&ai| printed[ai].iter().filter_map(move |g| view(ai, g)).map(|v| v[1])).collect();
        if ys.is_empty() { f64::INFINITY } else { ys.iter().sum::<f64>() / ys.len() as f64 }
    };
    samples.sort_by(|a, b| top(&a.1).total_cmp(&top(&b.1)));

    // Context clusters → translated bands → lines. Spend a new line before
    // cutting any cluster which could have fitted whole on an empty line.
    let mut lines: Vec<Vec<Run>> = Vec::new();
    let mut cur: Vec<Run> = Vec::new();
    let mut lx = 0.0;
    for (sample, ais) in &samples {
        for &ai in ais {
            if printed[ai].is_empty() {
                continue;
            }
            let img = &areas[ai].img;
            let (r, c) = contexts[ai];
            let overlap = if p.overlap_um > 0.0 { p.overlap_um } else { r.max(c / 3.0) };
            let clusters = std::mem::take(&mut area_clusters[ai]);
            stats.clusters += clusters.len();
            let label_w = text_w(sample, 3.2, true) + 6.0 + text_w(&format!("area {} · continued", area_short(areas[ai].rec)), 2.5, false);
            for (ri, cluster) in clusters.into_iter().enumerate() {
                // Prefer this ownership bound through both axes; the full-line
                // fallback relaxes it only if no fitting grouped cut exists.
                let min_group = if cluster.grains.len() >= 2 * p.min_group { p.min_group } else { 0 };
                let cut = CutContext { r, c, overlap, pad: p.pad_um, min_group, grains: &printed[ai] };
                let initial = Piece { grains: cluster.grains, x: 0.0, rect: cluster.rect, core: cluster.rect, cuts: Vec::new(), gap_after_um: None };
                // Cut wide clusters first: each width piece can then keep its
                // own height, without making a full-width sparse bottom slice.
                let mut cont = false;
                let mut rest = initial;
                loop {
                    let same = cur.last().is_some_and(|r| r.area == ai);
                    let mut start = if cur.is_empty() { 0.0 } else { lx + if same { PIECE_GAP_MM } else { RUN_GAP_MM } };
                    let whole = rest.w(k) <= body_w;
                    let need = if same { rest.w(k) } else { rest.w(k).max(LABEL_MIN_MM.min(label_w)) };
                    if !cur.is_empty() && ((whole && start + need > body_w) || (!whole && body_w - start < 0.4 * body_w)) {
                        lines.push(std::mem::take(&mut cur));
                        start = 0.0;
                    }
                    let parts = if rest.w(k) > body_w - start { cut_piece(&rest, 0, (body_w - start) * k, body_w * k, &cut, img) } else { None };
                    if parts.is_none() && start > 0.0 && rest.w(k) > body_w - start {
                        lines.push(std::mem::take(&mut cur));
                        lx = 0.0;
                        continue;
                    }
                    let (mut q, next) = match parts {
                        Some((left, right)) => {
                            stats.cuts += 1;
                            (left, Some(right))
                        }
                        // An individual grain can exceed the fixed-scale
                        // body. Preserve it and report the overflow in the audit.
                        None => (rest, None),
                    };
                    let mut vertical = Vec::new();
                    while q.h(k) > band_h {
                        let Some((top, bottom)) = cut_piece(&q, 1, band_h * k, band_h * k, &cut, img) else { break };
                        vertical.push(top);
                        q = bottom;
                        stats.cuts += 1;
                    }
                    vertical.push(q);
                    let count = vertical.len();
                    for (vi, mut q) in vertical.into_iter().enumerate() {
                        let start = if vi == 0 { start } else { 0.0 };
                        q.x = start;
                        let labelled = !cur.last().is_some_and(|r| r.area == ai);
                        // A removed inter-cluster stretch retains the existing
                        // // mark and distance. Cuts instead carry repeated strips.
                        if let Some(prev) = cur.last_mut().filter(|r| r.area == ai && r.row != ri + 1) {
                            let prev = prev.pieces.last_mut().expect("band");
                            prev.gap_after_um = Some(box_gap(prev.rect, q.rect));
                        }
                        let end = start + q.w(k);
                        lx = if labelled { end.max(start + label_w) } else { end };
                        cur.push(Run { sample: sample.clone(), area: ai, row: ri + 1, cont, labelled, x: start, end, pieces: vec![q] });
                        cont = true;
                        if vi + 1 < count {
                            lines.push(std::mem::take(&mut cur));
                        }
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
    if !cur.is_empty() {
        lines.push(cur);
    }
    stats.isolated = lines
        .iter()
        .flatten()
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
    let line_h = |ln: &Vec<Run>| LABEL_MM + ln.iter().flat_map(|r| &r.pieces).map(|q| q.h(k)).fold(0.0, f64::max) + NOTE_MM;
    let mut pages: Vec<Vec<Vec<Run>>> = Vec::new();
    let mut pg: Vec<Vec<Run>> = Vec::new();
    let mut ph = 0.0;
    for ln in lines {
        let h = line_h(&ln);
        if !pg.is_empty() && ph + h > body_h {
            pages.push(std::mem::take(&mut pg));
            ph = 0.0;
        }
        ph += h;
        pg.push(ln);
    }
    if !pg.is_empty() {
        pages.push(pg);
    }

    // Band images (in parallel).
    let jobs: Vec<(usize, usize, usize, usize)> = pages
        .iter()
        .enumerate()
        .flat_map(|(pi, pg)| pg.iter().enumerate().flat_map(move |(li, ln)| ln.iter().enumerate().flat_map(move |(ri, r)| (0..r.pieces.len()).map(move |qi| (pi, li, ri, qi)))))
        .collect();
    let imgs: Vec<(Vec<u8>, String)> = crate::util::par_map(jobs.len(), |j| {
        let (pi, li, ri, qi) = jobs[j];
        let run = &pages[pi][li][ri];
        let a = &areas[run.area];
        band_png(&a.img, run.pieces[qi].rect, grains.get(&a.rec.id).map_or(&[][..], |v| v.as_slice()), &p.enhance)
    });
    stats.bands = imgs.len();
    stats.stripped = imgs.iter().filter(|i| i.1 == "stripped").count();
    stats.strip_refused = imgs.iter().filter(|i| i.1.starts_with("kept:")).count();
    stats.sheets = pages.len();
    let line_count = pages.iter().map(Vec::len).sum::<usize>();
    let image_area: f64 = pages.iter().flatten().flatten().flat_map(|r| &r.pieces).map(|q| q.w(k) * q.h(k)).sum();
    let line_width: f64 = pages.iter().flatten().flatten().flat_map(|r| &r.pieces).map(|q| q.w(k)).sum();
    if stats.sheets > 0 {
        stats.fill.band_area = image_area / (stats.sheets as f64 * body_w * body_h);
    }
    if line_count > 0 {
        stats.fill.line_width = line_width / (line_count as f64 * body_w);
    }

    let mut reg = PrintRegistration {
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
        auto_scale: auto.clone(),
        spot_sizes_left_off_um: spots_off.clone(),
        sheet_origin: "sheet top-left corner, mm, y down".into(),
        mapping: "mosaic_um = mosaic_rect_um[0:2] + (sheet_mm - sheet_rect_mm[0:2]) * um_per_mm".into(),
        body_rect_mm: [m, m + HEADER_MM + 1.0, page.w_mm - m, page.h_mm - m - FOOTER_MM],
        strays,
        bands: Vec::new(),
    };
    // The header draws the key that fits this scale and says how the scale was chosen.
    let hp = SheetParams { spot_sizes_um: spots_shown.clone(), ..p.clone() };
    let scale_note = match &auto {
        Some(a) => {
            let why = match a.limit.as_str() {
                "target" => String::new(),
                "small grains" => format!(", limited by small grains ({} µm)", fmt(a.p10_um, 0)),
                other => format!(", limited by {other}"),
            };
            format!("auto: median grain {} µm prints {} mm{why}", fmt(a.median_um, 0), fmt(a.median_um / k, 1))
        }
        None => String::new(),
    };
    let mut svgs = Vec::new();
    let mut j = 0;
    let n = pages.len();
    for (pi, pg) in pages.iter().enumerate() {
        let mut here: Vec<String> = Vec::new();
        for r in pg.iter().flatten() {
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
        let mut band_rects: Vec<(usize, [f64; 4])> = Vec::new();
        let mut y = m + HEADER_MM + 1.0;
        for ln in pg {
            let bh = ln.iter().flat_map(|r| &r.pieces).map(|q| q.h(k)).fold(0.0, f64::max);
            let itop = y + LABEL_MM;
            for run in ln {
                let a = &areas[run.area];
                let (rx0, rx1) = (m + run.x, m + if run.labelled { run.end.max(run.x + 20.0) } else { run.end });
                let ly = y + LABEL_MM - 1.5;
                if run.labelled {
                    let _ = write!(s, r#"<rect x="{rx0:.2}" y="{:.2}" width="2.6" height="2.6" rx="0.4" fill="{INK}"/>"#, ly - 2.4);
                    let _ = write!(s, r#"<text x="{:.2}" y="{ly:.2}" font-size="3.2" font-weight="bold" fill="{INK}">{}</text>"#, rx0 + 3.6, esc(&run.sample));
                    let note = if run.cont { format!("area {} · continued", area_short(a.rec)) } else { format!("area {}", area_short(a.rec)) };
                    let _ = write!(s, r#"<text x="{:.2}" y="{ly:.2}" font-size="2.5" fill="{GREY}">{}</text>"#, rx0 + 3.6 + text_w(&run.sample, 3.2, true) + 2.4, esc(&note));
                } else {
                    // Next row of the same area: a short divider.
                    let dx = rx0 - PIECE_GAP_MM / 2.0;
                    let _ = write!(s, r#"<path d="M{dx:.2} {:.2} V{:.2}" stroke="{INK}" stroke-width="0.35"/>"#, ly - 2.0, ly + 1.1);
                }
                let _ = write!(s, r#"<path d="M{:.2} {:.2} H{rx1:.2}" stroke="{INK}" stroke-width="0.35"/>"#, if run.labelled { rx0 } else { rx0 - PIECE_GAP_MM / 2.0 }, ly + 1.1);
                for q in &run.pieces {
                    let (png, bg) = &imgs[j];
                    j += 1;
                    let (px, pw, ph) = (m + q.x, q.w(k), q.h(k));
                    let iy = itop;
                    let _ = write!(s, r#"<image x="{px:.3}" y="{iy:.3}" width="{pw:.3}" height="{ph:.3}" preserveAspectRatio="none" xlink:href="data:image/png;base64,{}"/>"#, base64(png));
                    let repeated = q.repeated(&printed[run.area], p.pad_um);
                    draw_repeated(&mut s, q, &repeated, [px, iy], k, p.pad_um, j);
                    let owned_w = (q.core[2].min(q.rect[2]) - q.core[0].max(q.rect[0])) / k;
                    if owned_w > 13.0 {
                        let bar = if owned_w > 28.0 { 100.0 } else { 50.0 };
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
                    band_rects.push((run.area, q.rect));
                    let context = printed[run.area]
                        .iter()
                        .filter(|g| !q.grains.iter().any(|own| own.id == g.id) && repeated.iter().any(|r| intersects(*r, g.bbox_um)))
                        .map(|g| BandGrain { id: g.id, number: g.number })
                        .collect();
                    reg.bands.push(BandRecord {
                        sheet: pi + 1,
                        sample: run.sample.clone(),
                        area: a.rec.id.clone(),
                        row: run.row,
                        grains: q.grains.iter().map(|g| BandGrain { id: g.id, number: g.number }).collect(),
                        sheet_rect_mm: [px, iy, px + pw, iy + ph],
                        mosaic_rect_um: q.rect,
                        repeated_um: repeated,
                        context,
                        background: bg.clone(),
                    });
                }
            }
            let ny = itop + bh + NOTE_MM - 0.8;
            let _ = write!(s, r##"<path d="M{m} {ny:.2} H{:.2}" stroke="#e3e3e3" stroke-width="0.2" stroke-dasharray="0.6 1.2"/>"##, w - m);
            y = itop + bh + NOTE_MM;
        }
        header(&mut s, &hp, meta, page, pi + 1, n, &here, areas, &printed, ownership, &stats, k, &scale_note, &spots_off);
        mount_map(&mut s, p, page, areas, &printed, &here, &band_rects, &|ai| sample_of(ai), &view);
        // Footer and corner registration marks.
        let fy = h - m + 0.5;
        let reg_file = if p.registration_file.is_empty() { format!("{}_print_registration.json", meta.package) } else { p.registration_file.clone() };
        let proj = if p.project.is_empty() { String::new() } else { format!("grainatlas project {} · ", p.project) };
        let _ = write!(
            s,
            r#"<text x="{:.2}" y="{fy:.2}" font-size="2.1" fill="{GREY}">{}</text><text x="{:.2}" y="{fy:.2}" font-size="2.1" fill="{GREY}" text-anchor="end">{} · sheet {}/{n}</text>"#,
            m + 7.0,
            esc(&format!(
                "{proj}registration {reg_file} (frame print_sheet; bands are translated mosaic crops, corner ticks mark their edges){}",
                if meta.source.is_empty() { String::new() } else { format!(" · source {}", meta.source) }
            )),
            w - m - 7.0,
            esc(&meta.package),
            pi + 1
        );
        for (cx, cy) in [(m - 3.0, m - 3.0), (w - m + 3.0, m - 3.0), (m - 3.0, h - m + 3.0), (w - m + 3.0, h - m + 3.0)] {
            let _ = write!(
                s,
                r#"<g stroke="{INK}" stroke-width="0.15" fill="none"><circle cx="{cx:.2}" cy="{cy:.2}" r="1.2"/><path d="M{:.2} {cy:.2} h5.2 M{cx:.2} {:.2} v5.2"/></g>"#,
                cx - 2.6,
                cy - 2.6
            );
        }
        s.push_str("</svg>");
        svgs.push(s);
    }
    let check = audit(&reg, &printed, areas);
    stats.grains_cut = check.cut;
    stats.grains_twice = check.twice;
    stats.grains_unmarked = check.unmarked;
    stats.bands_outside_body = check.outside_body;
    SheetResult { pages: svgs, registration: reg, stats }
}

/// Exact ownership audit, independent of how many repeated images show a grain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Audit {
    pub cut: usize,
    pub twice: usize,
    pub unmarked: usize,
    pub outside_body: usize,
}

pub fn audit(reg: &PrintRegistration, printed: &[Vec<&Grain>], areas: &[SheetArea]) -> Audit {
    // Older registrations have no body rectangle; at least check the paper.
    let body = if reg.body_rect_mm[2] > reg.body_rect_mm[0] { reg.body_rect_mm } else { [0.0, 0.0, reg.paper_mm[0], reg.paper_mm[1]] };
    let mut result = Audit { outside_body: reg.bands.iter().filter(|b| !contains(body, b.sheet_rect_mm) || b.sheet_rect_mm.iter().any(|v| !v.is_finite())).count(), ..Audit::default() };
    for (ai, gs) in printed.iter().enumerate() {
        let bands: Vec<&BandRecord> = reg.bands.iter().filter(|b| b.area == areas[ai].rec.id).collect();
        for g in gs {
            let owners: Vec<_> = bands.iter().filter(|b| b.grains.iter().any(|v| v.id == g.id)).collect();
            if owners.is_empty() || owners.iter().any(|b| !contains(b.mosaic_rect_um, g.bbox_um)) {
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

/// One band: the mosaic crop with the display LUT, enhanced as asked, as PNG; and what happened
/// to its background.
fn band_png(img: &AreaImage, rect: [f64; 4], grains: &[Grain], e: &EnhanceParams) -> (Vec<u8>, String) {
    let u = img.um_per_px;
    let (x0, y0) = ((rect[0] / u).round() as usize, (rect[1] / u).round() as usize);
    let (x1, y1) = (((rect[2] / u).round() as usize).min(img.w), ((rect[3] / u).round() as usize).min(img.h));
    let (cw, ch) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
    let mut g = Gray::new(cw, ch);
    for y in 0..ch {
        for x in 0..cw {
            let (sx, sy) = ((x0 + x).min(img.w - 1), (y0 + y).min(img.h - 1));
            g.px[y * cw + x] = img.lut[img.px[sy * img.w + sx] as usize] as f32 / 255.0;
        }
    }
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
    (crate::image::encode_png_gray(cw, ch, &g.to_u8()), bg)
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
) {
    let m = p.margin_mm;
    let map_w = map_width(page, areas);
    let text_r = page.w_mm - m - map_w - 6.0;
    let title = if p.title.is_empty() { meta.package.clone() } else { p.title.clone() };
    let _ = write!(s, r#"<text x="{m}" y="{:.2}" font-size="6" font-weight="bold" fill="{INK}">{}</text>"#, m + 6.0, esc(&title));
    let _ = write!(s, r#"<text x="{:.2}" y="{:.2}" font-size="4.2" font-weight="bold" fill="{INK}" text-anchor="end">Sheet {sheet} / {sheets}</text>"#, text_r, m + 5.6);
    let _ = write!(
        s,
        r#"<text x="{:.2}" y="{:.2}" font-size="2.3" fill="{GREY}" text-anchor="end">{} grains · {} bands · {} duplicates left to their owner area{}{}{}</text>"#,
        text_r,
        m + 10.0,
        stats.grains,
        stats.bands,
        stats.duplicates,
        if stats.strays > 0 { format!(" · {} strays left off", stats.strays) } else { String::new() },
        if stats.isolated > 0 { format!(" · {} isolated", stats.isolated) } else { String::new() },
        if p.printed.is_empty() { String::new() } else { format!(" · printed {}", esc(&p.printed)) }
    );
    // Key / value grid.
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
    let mut proc = vec![format!("1 mm = {} µm ({}×)", super::fmt_num(k), fmt(1000.0 / k, 0))];
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
    kv.push(("Print", proc.join(" · ")));
    let mut pkg = meta.package.clone();
    if !meta.comment.is_empty() {
        pkg += &format!(" · comment: {}", meta.comment);
    }
    if !p.lab.is_empty() {
        pkg += &format!(" · {}", p.lab);
    }
    kv.push(("Package", pkg));
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
    // On this sheet.
    let mut x = m;
    let mut y = m + 13.5 + 5.0 * 4.2 + 2.8;
    let _ = write!(s, r#"<text x="{x:.2}" y="{y:.2}" font-size="2.1" fill="{GREY}">ON THIS SHEET</text>"#);
    x += 25.0;
    for sn in here {
        let ais: Vec<usize> = (0..areas.len()).filter(|&ai| ownership.sample_of(&areas[ai].rec.id).unwrap_or(areas[ai].img.sample) == sn).collect();
        let ng: usize = ais.iter().map(|&ai| printed[ai].len()).sum();
        let ar: Vec<String> = ais.iter().filter(|&&ai| !printed[ai].is_empty()).map(|&ai| area_short(areas[ai].rec)).collect();
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
    let sy = m + HEADER_MM - 9.0;
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
    let _ = write!(s, r#"<path d="M{m} {:.2} H{text_r:.2}" stroke="{LIGHT}" stroke-width="0.3"/>"#, m + HEADER_MM - 0.5);
}

/// Area boxes in the SEM view (stage axes signed like the mosaic), um.
fn area_boxes(areas: &[SheetArea]) -> Vec<Option<[f64; 4]>> {
    areas
        .iter()
        .map(|a| {
            let r = a.rec;
            let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
            for t in &r.tiles {
                let st = t.stage_nm?;
                let (u, v) = (r.stage_sign[0] * st[0] / 1000.0, r.stage_sign[1] * st[1] / 1000.0);
                let (hw, hh) = (t.w as f64 * t.pixel_size_nm / 2000.0, t.h as f64 * t.pixel_size_nm / 2000.0);
                b = [b[0].min(u - hw), b[1].min(v - hh), b[2].max(u + hw), b[3].max(v + hh)];
            }
            b[0].is_finite().then_some(b)
        })
        .collect()
}

const MAP_LABEL_MM: f64 = 26.0;

/// Width of the map box: the mount at the full header height plus its labels, within limits.
fn map_width(page: super::Paper, areas: &[SheetArea]) -> f64 {
    let all: Vec<[f64; 4]> = area_boxes(areas).into_iter().flatten().collect();
    let max = (page.w_mm * 0.42).min(180.0);
    if all.is_empty() {
        return 80.0f64.min(max);
    }
    let w = all.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max) - all.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min);
    let h = all.iter().map(|b| b[3]).fold(f64::NEG_INFINITY, f64::max) - all.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min);
    ((HEADER_MM - 7.5) * w / h.max(1.0) + MAP_LABEL_MM + 6.0).clamp(90.0, max)
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
    bands: &[(usize, [f64; 4])],
    sample_of: &dyn Fn(usize) -> String,
    view: &dyn Fn(usize, &Grain) -> Option<[f64; 2]>,
) {
    let m = p.margin_mm;
    let (bw, bh) = (map_width(page, areas), HEADER_MM - 1.5);
    let (bx, by) = (page.w_mm - m - bw, m);
    let boxes = area_boxes(areas);
    let _ = write!(s, r#"<rect x="{bx:.2}" y="{by:.2}" width="{bw:.2}" height="{bh:.2}" rx="1.5" fill="none" stroke="{LIGHT}" stroke-width="0.3"/>"#);
    let all: Vec<[f64; 4]> = boxes.iter().flatten().copied().collect();
    if all.is_empty() {
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
        let on = here.contains(&sample_of(ai));
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
    // Grains as dots.
    for (ai, gs) in printed.iter().enumerate() {
        let on = here.contains(&sample_of(ai));
        let mut d = String::new();
        for g in gs {
            if let Some(v) = view(ai, g) {
                let _ = write!(d, "M{:.2} {:.2}h0.01", tx(v[0]), ty(v[1]));
            }
        }
        if !d.is_empty() {
            let _ = write!(s, r#"<path d="{d}" stroke="{}" stroke-width="0.7" stroke-linecap="round"/>"#, if on { "white" } else { "#9a9a9a" });
        }
    }
    // This sheet's bands as white outlines (mosaic corners through the stage).
    for (ai, r) in bands {
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
    for ai in 0..areas.len() {
        let sn = sample_of(ai);
        if labels.iter().any(|l| l.0 == sn) {
            continue;
        }
        let bs: Vec<[f64; 4]> = (0..areas.len()).filter(|&j| sample_of(j) == sn).filter_map(|j| boxes[j]).collect();
        if bs.is_empty() {
            continue;
        }
        let right = bs.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max);
        let mid = bs.iter().map(|b| (b[1] + b[3]) / 2.0).sum::<f64>() / bs.len() as f64;
        let on = here.contains(&sn);
        labels.push((sn, tx(right), ty(mid), on));
    }
    labels.sort_by(|a, b| a.2.total_cmp(&b.2));
    let gap = 2.7;
    let mut ys: Vec<f64> = labels.iter().map(|l| l.2).collect();
    for i in 1..ys.len() {
        ys[i] = ys[i].max(ys[i - 1] + gap);
    }
    // Shift the column up when it runs off the bottom of the map.
    let over = ys.last().map_or(0.0, |&y| y - (by + bh - 7.0)).max(0.0);
    for y in ys.iter_mut() {
        *y -= over;
    }
    let col = tx(x1) + 3.0;
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
            if *on { 2.5 } else { 2.2 },
            if *on { r#"font-weight="bold" "# } else { "" },
            if *on { INK } else { GREY },
            esc(sn)
        );
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
