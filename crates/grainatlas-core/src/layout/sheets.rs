//! Print sheets for hand spot registration (first used on the Boise State SEM
//! run of 2026-11-03): every owned grain of a mount, at one fixed print scale,
//! as bands cut straight out of the mosaic.
//!
//! Per sample (top of the mount first), the owned grains of each area are
//! chained into rows along x. Rows flow across the lines of a sheet and break
//! only between grains; an empty stretch longer than `gap_break_um` becomes a
//! short break marked with its length. Every band is an exact translation of
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
    /// Micrometres of sample per millimetre of paper (8 = 125x).
    pub um_per_mm: f64,
    /// Margin around the grains of a band (um).
    pub pad_um: f64,
    /// An empty stretch of a row longer than this becomes a marked break (um).
    pub gap_break_um: f64,
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
            um_per_mm: 8.0,
            pad_um: 22.0,
            gap_break_um: 420.0,
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

/// One area to print: its record (stage geometry) and pixels.
pub struct SheetArea<'a> {
    pub rec: &'a AreaRecord,
    pub img: AreaImage<'a>,
}

/// The `print_sheet` frame: paper sheet ↔ grainatlas mosaic.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrintRegistration {
    pub frame: String,
    pub note: String,
    pub package: String,
    pub project: String,
    pub paper: String,
    pub paper_mm: [f64; 2],
    pub um_per_mm: f64,
    pub sheet_origin: String,
    pub mapping: String,
    pub bands: Vec<BandRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BandRecord {
    pub sheet: usize,
    pub sample: String,
    pub area: String,
    pub row: usize,
    pub grains: Vec<BandGrain>,
    /// x0, y0, x1, y1 on the sheet (mm, top-left origin, y down).
    pub sheet_rect_mm: [f64; 4],
    /// x0, y0, x1, y1 in the area's mosaic (um).
    pub mosaic_rect_um: [f64; 4],
    /// "kept", "stripped", or why it was kept.
    pub background: String,
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
    pub stripped: usize,
    pub strip_refused: usize,
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
/// Between bands of one row cut where its grains step up or down.
const STEP_GAP_MM: f64 = 2.0;
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
        let best = rows
            .iter()
            .enumerate()
            .map(|(i, r)| (i, (r.last().expect("row").centroid_um[1] - y).abs()))
            .filter(|(_, d)| *d < tol)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((i, _)) => rows[i].push(g),
            None => rows.push(vec![g]),
        }
    }
    let mean_y = |r: &Vec<&Grain>| r.iter().map(|g| g.centroid_um[1]).sum::<f64>() / r.len() as f64;
    rows.sort_by(|a, b| mean_y(a).total_cmp(&mean_y(b)));
    rows
}

/// A run of grains of one row on one line, cut into pieces at long gaps.
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
    let k = p.um_per_mm.max(0.5);
    let m = p.margin_mm;
    let body_w = page.w_mm - 2.0 * m;
    let body_h = page.h_mm - 2.0 * m - HEADER_MM - FOOTER_MM;
    let mut stats = SheetStats::default();

    // Printed grains per area: owned, within the size limits.
    let mut printed: Vec<Vec<&Grain>> = Vec::new();
    for a in areas {
        let all = grains.get(&a.rec.id).map_or(&[][..], |v| v.as_slice());
        let mut v = Vec::new();
        for g in all {
            if !ownership.is_owned(&a.rec.id, &g.id) {
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

    // Rows → runs → lines.
    let mut lines: Vec<Vec<Run>> = Vec::new();
    let mut cur: Vec<Run> = Vec::new();
    let mut lx = 0.0;
    for (sample, ais) in &samples {
        for &ai in ais {
            if printed[ai].is_empty() {
                continue;
            }
            let img = &areas[ai].img;
            let (aw, ah) = (img.w as f64, img.h as f64);
            let rows = rows_of(&printed[ai], if aw > 2.5 * ah { 260.0 } else { 120.0 });
            for (ri, row) in rows.iter().enumerate() {
                let pw = |gs: &[&Grain]| {
                    let r = piece_rect(gs, p.pad_um, img);
                    (r[2] - r[0]) / k
                };
                let label_w = text_w(sample, 3.2, true) + 6.0 + text_w(&format!("area {} · continued", area_short(areas[ai].rec)), 2.5, false);
                let mut i = 0;
                let mut cont = false;
                while i < row.len() {
                    // A row that follows another row of the same area on the line gets a divider.
                    let same = cur.last().is_some_and(|r: &Run| r.area == ai);
                    let mut start = if cur.is_empty() { 0.0 } else if same { lx + PIECE_GAP_MM } else { lx + RUN_GAP_MM };
                    let need = if same { pw(&row[i..=i]) } else { pw(&row[i..=i]).max(LABEL_MIN_MM.min(label_w)) };
                    if !cur.is_empty() && start + need > body_w {
                        lines.push(std::mem::take(&mut cur));
                        start = 0.0;
                    }
                    let labelled = !cur.last().is_some_and(|r: &Run| r.area == ai);
                    let mut run = Run { sample: sample.clone(), area: ai, row: ri + 1, cont, labelled, x: start, end: start, pieces: Vec::new() };
                    let mut x = start;
                    let mut piece: Option<Vec<&Grain>> = Some(vec![row[i]]);
                    i += 1;
                    while i < row.len() {
                        let g = row[i];
                        let cur_piece = piece.as_mut().expect("piece");
                        let gap = g.bbox_um[0] - cur_piece.iter().map(|q| q.bbox_um[2]).fold(f64::NEG_INFINITY, f64::max);
                        if gap > p.gap_break_um {
                            let gs = piece.take().expect("piece");
                            let rect = piece_rect(&gs, p.pad_um, img);
                            let w = (rect[2] - rect[0]) / k;
                            run.pieces.push(Piece { grains: gs, x, rect, gap_after_um: Some(gap) });
                            x += w + PIECE_GAP_MM;
                            if x + pw(&row[i..=i]) > body_w {
                                break;
                            }
                            piece = Some(vec![g]);
                            i += 1;
                            continue;
                        }
                        let mut trial = cur_piece.clone();
                        trial.push(g);
                        // A row whose grains step up or down is cut into bands of their own height,
                        // so one stray grain does not make the whole band (and line) tall.
                        let tallest = trial.iter().map(|q| q.bbox_um[3] - q.bbox_um[1]).fold(0.0, f64::max);
                        let span = trial.iter().map(|q| q.bbox_um[3]).fold(f64::NEG_INFINITY, f64::max) - trial.iter().map(|q| q.bbox_um[1]).fold(f64::INFINITY, f64::min);
                        if span > 1.35 * tallest + 40.0 && !cur_piece.is_empty() {
                            let gs = piece.take().expect("piece");
                            let rect = piece_rect(&gs, p.pad_um, img);
                            run.pieces.push(Piece { grains: gs, x, rect, gap_after_um: None });
                            x += (rect[2] - rect[0]) / k + STEP_GAP_MM;
                            if x + pw(&row[i..=i]) > body_w {
                                break;
                            }
                            piece = Some(vec![g]);
                            i += 1;
                            continue;
                        }
                        if x + pw(&trial) > body_w {
                            break;
                        }
                        cur_piece.push(g);
                        i += 1;
                    }
                    if let Some(gs) = piece {
                        let rect = piece_rect(&gs, p.pad_um, img);
                        let w = (rect[2] - rect[0]) / k;
                        run.pieces.push(Piece { grains: gs, x, rect, gap_after_um: None });
                        x += w;
                    } else if let Some(last) = run.pieces.last() {
                        x = last.x + last.w(k);
                    }
                    if let Some(last) = run.pieces.last_mut() {
                        last.gap_after_um = None;
                    }
                    run.end = x;
                    lx = if labelled { x.max(start + label_w) } else { x };
                    cur.push(run);
                    if i < row.len() {
                        lines.push(std::mem::take(&mut cur));
                        lx = 0.0;
                        cont = true;
                    }
                }
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
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

    let mut reg = PrintRegistration {
        frame: "print_sheet".into(),
        note: "paper sheet <-> grainatlas mosaic; not the laser stage registration".into(),
        package: meta.package.clone(),
        project: p.project.clone(),
        paper: format!("{} {} {} x {} mm", p.paper, if p.landscape { "landscape" } else { "portrait" }, page.w_mm, page.h_mm),
        paper_mm: [page.w_mm, page.h_mm],
        um_per_mm: k,
        sheet_origin: "sheet top-left corner, mm, y down".into(),
        mapping: "mosaic_um = mosaic_rect_um[0:2] + (sheet_mm - sheet_rect_mm[0:2]) * um_per_mm".into(),
        bands: Vec::new(),
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
                    if pw > 13.0 {
                        let bar = if pw > 28.0 { 100.0 } else { 50.0 };
                        let (bx1, by) = (px + pw - 2.2, iy + ph - 2.0);
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
                        let _ = write!(s, r#"<text x="{gx:.2}" y="{:.2}" font-size="2.4" fill="{GREY}" text-anchor="middle">//</text><text x="{gx:.2}" y="{:.2}" font-size="1.7" fill="{GREY}" text-anchor="middle">{:.1} mm</text>"#, iy + ph / 2.0, iy + ph / 2.0 + 2.6, gap / 1000.0);
                    }
                    band_rects.push((run.area, q.rect));
                    reg.bands.push(BandRecord {
                        sheet: pi + 1,
                        sample: run.sample.clone(),
                        area: a.rec.id.clone(),
                        row: run.row,
                        grains: q.grains.iter().map(|g| BandGrain { id: g.id, number: g.number }).collect(),
                        sheet_rect_mm: [r3(px), r3(iy), r3(px + pw), r3(iy + ph)],
                        mosaic_rect_um: [r3(q.rect[0]), r3(q.rect[1]), r3(q.rect[2]), r3(q.rect[3])],
                        background: bg.clone(),
                    });
                }
            }
            let ny = itop + bh + NOTE_MM - 0.8;
            let _ = write!(s, r##"<path d="M{m} {ny:.2} H{:.2}" stroke="#e3e3e3" stroke-width="0.2" stroke-dasharray="0.6 1.2"/>"##, w - m);
            y = itop + bh + NOTE_MM;
        }
        header(&mut s, p, meta, page, pi + 1, n, &here, areas, &printed, ownership, &stats, k);
        mount_map(&mut s, p, page, areas, &printed, &here, &band_rects, &|ai| sample_of(ai), &view);
        // Footer and corner registration marks.
        let fy = h - m + 0.5;
        let reg_file = if p.registration_file.is_empty() { format!("{}_print_registration.json", meta.package) } else { p.registration_file.clone() };
        let proj = if p.project.is_empty() { String::new() } else { format!("grainatlas project {} · ", p.project) };
        let _ = write!(
            s,
            r#"<text x="{:.2}" y="{fy:.2}" font-size="2.1" fill="{GREY}">{}</text><text x="{:.2}" y="{fy:.2}" font-size="2.1" fill="{GREY}" text-anchor="end">{} · sheet {}/{n}</text>"#,
            m + 7.0,
            esc(&format!("{proj}registration {reg_file} (frame print_sheet; bands are translated mosaic crops, corner ticks mark their edges){}", if meta.source.is_empty() { String::new() } else { format!(" · source {}", meta.source) })),
            w - m - 7.0,
            esc(&meta.package),
            pi + 1
        );
        for (cx, cy) in [(m - 3.0, m - 3.0), (w - m + 3.0, m - 3.0), (m - 3.0, h - m + 3.0), (w - m + 3.0, h - m + 3.0)] {
            let _ = write!(s, r#"<g stroke="{INK}" stroke-width="0.15" fill="none"><circle cx="{cx:.2}" cy="{cy:.2}" r="1.2"/><path d="M{:.2} {cy:.2} h5.2 M{cx:.2} {:.2} v5.2"/></g>"#, cx - 2.6, cy - 2.6);
        }
        s.push_str("</svg>");
        svgs.push(s);
    }
    SheetResult { pages: svgs, registration: reg, stats }
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
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

fn fmt(v: f64, d: usize) -> String {
    format!("{v:.d$}")
}

#[allow(clippy::too_many_arguments)]
fn header(s: &mut String, p: &SheetParams, meta: &SheetMeta, page: super::Paper, sheet: usize, sheets: usize, here: &[String], areas: &[SheetArea], printed: &[Vec<&Grain>], ownership: &Ownership, stats: &SheetStats, k: f64) {
    let m = p.margin_mm;
    let map_w = map_width(page, areas);
    let text_r = page.w_mm - m - map_w - 6.0;
    let title = if p.title.is_empty() { meta.package.clone() } else { p.title.clone() };
    let _ = write!(s, r#"<text x="{m}" y="{:.2}" font-size="6" font-weight="bold" fill="{INK}">{}</text>"#, m + 6.0, esc(&title));
    let _ = write!(s, r#"<text x="{:.2}" y="{:.2}" font-size="4.2" font-weight="bold" fill="{INK}" text-anchor="end">Sheet {sheet} / {sheets}</text>"#, text_r, m + 5.6);
    let _ = write!(
        s,
        r#"<text x="{:.2}" y="{:.2}" font-size="2.3" fill="{GREY}" text-anchor="end">{} grains · {} bands · {} duplicates left to their owner area{}</text>"#,
        text_r,
        m + 10.0,
        stats.grains,
        stats.bands,
        stats.duplicates,
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
    let mut proc = vec![format!("1 mm = {} µm ({}×)", super::fmt_num(k), fmt(1000.0 / k, 0)), "Auto tone".to_string()];
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
        let _ = write!(s, r#"<text x="{cx:.2}" y="{cy:.2}" font-size="2.1" fill="{GREY}">{}</text><text x="{:.2}" y="{cy:.2}" font-size="2.5" fill="{INK}">{}</text>"#, key.to_uppercase(), cx + 16.0, esc(val));
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
        let _ = write!(s, r#"<rect x="{x:.2}" y="{:.2}" width="2.4" height="2.4" rx="0.4" fill="{INK}"/><text x="{:.2}" y="{y:.2}" font-size="2.7" font-weight="bold" fill="{INK}">{}</text><text x="{:.2}" y="{y:.2}" font-size="2.5" fill="{INK}">{}</text>"#, y - 2.1, x + 3.4, esc(sn), x + 4.6 + text_w(sn, 2.7, true), esc(&txt));
        x += tw;
    }
    // Laser spot sizes at print scale.
    let sy = m + HEADER_MM - 9.0;
    let _ = write!(s, r#"<text x="{m}" y="{:.2}" font-size="2.1" fill="{GREY}">LASER SPOT Ø AT PRINT SCALE (µm)</text>"#, sy - 1.0);
    let mut cx = m + 52.0;
    for &d in &p.spot_sizes_um {
        let r = d / 2.0 / k;
        let _ = write!(s, r#"<circle cx="{:.3}" cy="{:.3}" r="{r:.3}" fill="none" stroke="{INK}" stroke-width="0.2"/><text x="{:.3}" y="{:.3}" font-size="2" fill="{GREY}" text-anchor="middle">{}</text>"#, cx + r, sy - 1.8, cx + r, sy + r.max(1.0) + 1.0, super::fmt_num(d));
        cx += 2.0 * r + 4.0;
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
fn mount_map(s: &mut String, p: &SheetParams, page: super::Paper, areas: &[SheetArea], printed: &[Vec<&Grain>], here: &[String], bands: &[(usize, [f64; 4])], sample_of: &dyn Fn(usize) -> String, view: &dyn Fn(usize, &Grain) -> Option<[f64; 2]>) {
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
        let _ = write!(s, r#"<rect x="{:.3}" y="{:.3}" width="{:.3}" height="{:.3}" fill="none" stroke="white" stroke-width="0.35"/><rect x="{:.3}" y="{:.3}" width="{:.3}" height="{:.3}" fill="none" stroke="black" stroke-width="0.12"/>"#, tx(u0), ty(v0), (u1 - u0) * sc, (v1 - v0) * sc, tx(u0), ty(v0), (u1 - u0) * sc, (v1 - v0) * sc);
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
    let _ = write!(s, r#"<path d="M{:.2} {:.2} h{l:.3}" stroke="{GREY}" stroke-width="0.4"/><text x="{:.2}" y="{:.2}" font-size="2" fill="{GREY}">1 mm · SEM view · dark: this sheet · boxes: its bands</text>"#, bx + 3.0, by + bh - 2.2, bx + 4.5 + l, by + bh - 1.5);
}
