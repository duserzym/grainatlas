//! Atlas bundles (`upblab/docs/WORKFLOW.md` section 3.1): what grainatlas
//! publishes to UPbDB, or hands over as files.
//!
//! ```text
//! <project_id>.atlas/
//!   atlas.json                         format, version, ids, scale, areas and levels, package hash,
//!                                      sha256 and size of every other file
//!   pyramid/<area>/<z>/<x>_<y>.webp    the corrected mosaic, 256 px tiles, z = 0 full resolution
//!   grains.geojson                     grain outlines in mosaic micrometres, with labels and ids
//!   spots.json                         the spot document of POST /v1/mounts/{id}/spots (one per sample)
//!   scalebars.json                     per-grain scale bar lengths
//!   project.json                       the grainatlas project (parameters, never pixels)
//! ```
//!
//! Everything is a pure function of the project, the stitched mosaics and the
//! options, so the native and wasm builds write identical bytes (the parity
//! check hashes `atlas.json`, which carries the hash of every other file).

pub mod vp8l;
pub mod webp;
pub mod zip;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::mosaic::pyramid::Pyramid;
use crate::photometric::ToneParams;
use crate::project::{AreaRecord, Project};
use crate::segment::Grain;
use crate::spots::{Spot, SpotKind};
use crate::{Error, Result};

pub const FORMAT: &str = "grainatlas-atlas";
pub const VERSION: u32 = 1;
pub const TILE: usize = 256;

/// How to write a bundle (the UI's Settings, plus the export time).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BundleOptions {
    /// 100 = lossless WebP, 90 to 99 = near-lossless (see [`webp`]).
    pub quality: u8,
    /// RFC 3339 time written into `atlas.json` (the caller's clock; fixed in tests).
    pub exported: String,
    /// Where the raw SEM package lives on the lab storage (NAS layout), when known.
    pub nas_path: Option<String>,
}

impl Default for BundleOptions {
    fn default() -> Self {
        BundleOptions { quality: 100, exported: String::new(), nas_path: None }
    }
}

/// Content hash of the raw package: sha256 of the lines `<sha256>  <path>\n` (the
/// `sha256sum` format) of every image tile of every area, sorted by path.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct PackageDigest {
    pub sha256: String,
    pub files: usize,
}

impl PackageDigest {
    pub fn of(mut entries: Vec<(String, String)>) -> PackageDigest {
        entries.sort();
        entries.dedup();
        let text: String = entries.iter().map(|(path, sha)| format!("{sha}  {path}\n")).collect();
        PackageDigest { sha256: crate::util::sha256_hex(text.as_bytes()), files: entries.len() }
    }
}

/// One stitched area: its record and its 8-bit base mosaic (before the display LUT).
pub struct AreaPixels<'a> {
    pub rec: &'a AreaRecord,
    pub w: usize,
    pub h: usize,
    pub base: &'a [u8],
    /// True when base comes from a published image rather than raw package tiles.
    pub bundle_backed: bool,
}

/// A path segment from an area id (folder names may hold spaces or other characters).
pub fn safe_segment(id: &str) -> String {
    let s: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
    let s = s.trim_matches('.').to_string();
    if s.is_empty() { "area".into() } else { s }
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn kind_name(k: SpotKind) -> &'static str {
    match k {
        SpotKind::Spot => "spot",
        SpotKind::Line => "line",
        SpotKind::Raster => "raster",
    }
}

/// The UI-held parts of a saved project (`grainatlas_ui`): grains per area, spots, tone per area.
pub struct UiParts {
    pub grains: BTreeMap<String, Vec<Grain>>,
    pub spots: Vec<Spot>,
    pub tone: BTreeMap<String, ToneParams>,
}

impl UiParts {
    pub fn of(project: &Project) -> UiParts {
        let ui = project.extra.get("grainatlas_ui").cloned().unwrap_or(serde_json::Value::Null);
        UiParts {
            // Artifacts (bubbles, seams, tile blocks the user did not keep) are not published.
            grains: serde_json::from_value::<BTreeMap<String, Vec<Grain>>>(ui["grains"].clone())
                .unwrap_or_default()
                .into_iter()
                .map(|(a, gs)| (a, gs.into_iter().filter(|g| g.is_grain()).collect()))
                .collect(),
            spots: serde_json::from_value(ui["spots"].clone()).unwrap_or_default(),
            tone: serde_json::from_value(ui["tone"].clone()).unwrap_or_default(),
        }
    }
}

/// `spots.json`: the body of `POST /v1/mounts/{mount_id}/spots`. One sample: that body (each
/// spot also names its `area`). Several samples: `{ project_id, documents: [body per sample] }`,
/// the form UPbDB also accepts in bundles (its docs/workflow_api.md).
pub fn spots_document(project: &Project, spots: &[Spot], areas: &[&str]) -> serde_json::Value {
    let grains = UiParts::of(project).grains;
    let spots: Vec<&Spot> = spots.iter().filter(|s| areas.contains(&s.area_id.as_str())).collect();
    let sample_of = |s: &Spot| -> String {
        if !s.sample.is_empty() {
            return s.sample.clone();
        }
        project.areas.iter().find(|a| a.id == s.area_id).map(|a| if a.sample.is_empty() { a.name.clone() } else { a.sample.clone() }).unwrap_or_default()
    };
    let mut by_sample: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for s in &spots {
        let mut v = json!({
            "id": s.id,
            "name": s.name,
            "grain": s.grain_id.and_then(|id| grains.get(&s.area_id)?.iter().find(|g| g.id == id))
                .map(|g| g.label.clone().unwrap_or_else(|| format!("{}/{}", s.area_id, g.number)))
                .unwrap_or_else(|| s.grain_number.map_or(String::new(), |n| format!("{}/{n}", s.area_id))),
            "x_um": r3(s.x_um),
            "y_um": r3(s.y_um),
            "diameter_um": r3(s.diameter_um),
            "kind": kind_name(s.kind),
            "area": s.area_id,
        });
        if let (Some(x2), Some(y2)) = (s.x2_um, s.y2_um) {
            v["x2_um"] = json!(r3(x2));
            v["y2_um"] = json!(r3(y2));
        }
        by_sample.entry(sample_of(s)).or_default().push(v);
    }
    if by_sample.len() <= 1 {
        let (sample, list) = by_sample.into_iter().next().unwrap_or_default();
        return json!({ "project_id": project.uuid, "sample": sample, "spots": list });
    }
    let documents: Vec<serde_json::Value> = by_sample.into_iter().map(|(sample, list)| json!({ "project_id": project.uuid, "sample": sample, "spots": list })).collect();
    json!({ "project_id": project.uuid, "documents": documents })
}

/// `grains.geojson`: one Polygon feature per grain (mosaic micrometres, x right, y down).
pub fn grains_geojson(project: &Project, grains: &BTreeMap<String, Vec<Grain>>, areas: &[&str]) -> serde_json::Value {
    let mut features = Vec::new();
    for (area_id, gs) in grains {
        if !areas.contains(&area_id.as_str()) {
            continue;
        }
        let rec = project.areas.iter().find(|a| &a.id == area_id);
        let sample = rec.map(|a| if a.sample.is_empty() { a.name.clone() } else { a.sample.clone() }).unwrap_or_default();
        for g in gs {
            let mut ring: Vec<[f64; 2]> = g.polygon_um.iter().map(|p| [r3(p[0]), r3(p[1])]).collect();
            if let Some(f) = ring.first().copied() {
                ring.push(f);
            }
            features.push(json!({
                "type": "Feature",
                "id": g.id,
                "geometry": { "type": "Polygon", "coordinates": [ring] },
                "properties": {
                    "kind": "grain", "id": g.id, "label": g.display_number(), "number": g.number,
                    "area": area_id, "sample": sample,
                    "centroid_um": [r3(g.centroid_um[0]), r3(g.centroid_um[1])],
                    "area_um2": r3(g.area_um2), "eq_diameter_um": r3(g.eq_diameter_um),
                    "major_um": r3(g.major_um), "minor_um": r3(g.minor_um), "orientation_deg": r3(g.orientation_deg),
                    "scale_bar_um": g.scale_bar_um, "manual": g.manual,
                }
            }));
        }
    }
    json!({
        "type": "FeatureCollection",
        "name": project.name,
        "grainatlas": { "project_id": project.uuid, "units": "micrometre", "frame": "mosaic, origin top-left, y down" },
        "features": features,
    })
}

/// `scalebars.json`: the scale bar of each grain (plan section 6.6) and a whole-area bar.
pub fn scalebars(grains: &BTreeMap<String, Vec<Grain>>, areas: &[AreaPixels]) -> serde_json::Value {
    let list: Vec<serde_json::Value> = areas
        .iter()
        .map(|a| {
            let k = a.rec.um_per_px();
            let gs: Vec<serde_json::Value> = grains
                .get(&a.rec.id)
                .map(|gs| {
                    gs.iter()
                        .map(|g| {
                            let len = if g.scale_bar_um > 0.0 { g.scale_bar_um } else { crate::scale::grain_bar_um(g.bbox_um[2] - g.bbox_um[0]) };
                            json!({ "grain_id": g.id, "number": g.number, "length_um": len, "anchor_um": [r3(g.bbox_um[0]), r3(g.bbox_um[3])] })
                        })
                        .collect()
                })
                .unwrap_or_default();
            json!({ "area": a.rec.id, "um_per_px": k, "area_bar_um": crate::scale::round_bar_um(a.w as f64 * k / 5.0), "grains": gs })
        })
        .collect();
    json!({ "units": "micrometre", "anchor": "bottom-left corner of the grain's bounding box; the bar is drawn below it", "areas": list })
}

/// Every file of the bundle, `atlas.json` first, then by path.
pub fn build(project: &Project, areas: &[AreaPixels], package: &PackageDigest, opts: &BundleOptions) -> Result<Vec<(String, Vec<u8>)>> {
    if areas.is_empty() {
        return Err(Error::Invalid("stitch at least one area before exporting an atlas bundle".into()));
    }
    let ui = UiParts::of(project);
    let ids: Vec<&str> = areas.iter().map(|a| a.rec.id.as_str()).collect();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut area_json = Vec::new();
    let quality = opts.quality.clamp(90, 100);
    for a in areas {
        let overlay: Option<ToneParams> = project
            .extra
            .get("grainatlas_ui")
            .and_then(|ui| ui.get("bundle_tone"))
            .and_then(|v| v.get(&a.rec.id))
            .and_then(|v| v.get("display"))
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let identity = ToneParams::neutral();
        let tone = if a.bundle_backed {
            overlay.as_ref().unwrap_or(&identity)
        } else {
            ui.tone.get(&a.rec.id).unwrap_or(&a.rec.params.tone)
        };
        let lut = crate::photometric::tone::lut(tone);
        let px: Vec<u8> = a.base.iter().map(|&v| lut[v as usize]).collect();
        let pyr = Pyramid::build_tiled(a.w, a.h, px, TILE);
        let dir = format!("pyramid/{}", safe_segment(&a.rec.id));
        let mut jobs = Vec::new();
        for (info, _) in &pyr.levels {
            for r in 0..info.rows {
                for c in 0..info.cols {
                    jobs.push((info.level, c, r));
                }
            }
        }
        let tiles = crate::util::par_map(jobs.len(), |k| {
            let (l, c, r) = jobs[k];
            let (w, h, px) = pyr.tile(l, c, r).expect("tile in range");
            webp::encode_gray(w, h, &px, quality).map(|b| (format!("{dir}/{l}/{c}_{r}.webp"), b))
        });
        for t in tiles {
            files.push(t?);
        }
        let levels: Vec<serde_json::Value> = pyr.infos().iter().map(|i| json!({ "z": i.level, "w": i.w, "h": i.h, "cols": i.cols, "rows": i.rows, "scale": i.scale })).collect();
        area_json.push(json!({
            "id": a.rec.id, "uuid": a.rec.uuid, "name": a.rec.name,
            "sample": if a.rec.sample.is_empty() { &a.rec.name } else { &a.rec.sample },
            "path": dir, "w": a.w, "h": a.h, "um_per_px": a.rec.um_per_px(), "origin_um": [0.0, 0.0],
            "levels": levels,
            "grains": ui.grains.get(&a.rec.id).map_or(0, |g| g.len()),
            "spots": ui.spots.iter().filter(|s| s.area_id == a.rec.id).count(),
            "mosaic_sha256": a.rec.mosaic.as_ref().map(|m| m.sha256.clone()),
        }));
    }
    let pretty = |v: &serde_json::Value| {
        let mut v = v.clone();
        canonical(&mut v);
        serde_json::to_vec_pretty(&v).map_err(|e| Error::Invalid(e.to_string()))
    };
    files.push(("grains.geojson".into(), pretty(&grains_geojson(project, &ui.grains, &ids))?));
    files.push(("spots.json".into(), pretty(&spots_document(project, &ui.spots, &ids))?));
    files.push(("scalebars.json".into(), pretty(&scalebars(&ui.grains, areas))?));
    let mut saved = serde_json::to_value(project).map_err(|e| Error::Invalid(e.to_string()))?;
    if let Some(ui) = saved
        .get_mut("grainatlas_ui")
        .and_then(|v| v.as_object_mut())
    {
        ui.remove("bundle_tone");
    }
    canonical(&mut saved);
    // Unknown strip edits are opaque project data, including their numeric precision.
    if let Some(crate::project::StoredPrintEdits::Raw(raw)) = &project.print_sheets { saved["print_sheets"] = raw.clone(); }
    files.push(("project.json".into(), serde_json::to_vec_pretty(&saved).map_err(|e| Error::Invalid(e.to_string()))?));
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let listed: Vec<serde_json::Value> = files.iter().map(|(p, b)| json!({ "path": p, "size": b.len(), "sha256": crate::util::sha256_hex(b) })).collect();
    let atlas = json!({
        "format": FORMAT,
        "version": VERSION,
        "project_id": project.uuid,
        "name": project.name,
        "mount_id": project.upbdb.mount_id,
        "opened_from": project.package.kind,
        "scale_um_per_px": areas[0].rec.um_per_px(),
        "tile_size": TILE,
        "tile_format": "webp",
        "tile_encoding": webp::encoding_name(quality),
        "tile_quality": quality,
        "levels": areas.iter().map(|a| level_count(a.w, a.h)).max().unwrap_or(1),
        "areas": area_json,
        "package": {
            "name": project.package.name,
            "reader": project.package.reader,
            "kind": project.package.kind,
            "path": project.package.path,
            "nas_path": opts.nas_path,
            "content_sha256": package.sha256,
            "content_files": package.files,
            "content_hash": "sha256 of the lines '<sha256>  <path>\\n' of every image tile, sorted by path",
        },
        "grainatlas_version": env!("CARGO_PKG_VERSION"),
        "exported": opts.exported,
        "files": listed,
    });
    files.insert(0, ("atlas.json".into(), pretty(&atlas)?));
    Ok(files)
}

/// Rounds every non-integer number to 9 significant digits and writes integral ones as integers. The native and wasm builds agree on
/// every pixel but not always on the last bits of derived floats (placement residuals, grain
/// statistics: platform `libm` and summation order), so the bundle's JSON is written at a
/// precision both reach.
pub fn canonical(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Number(n) if n.is_f64() => {
            let x = n.as_f64().unwrap_or(0.0);
            let r: f64 = format!("{x:.8e}").parse().unwrap_or(x);
            // Integral values are written as integers (JavaScript cannot tell 10.0 from 10).
            if r.fract() == 0.0 && r.abs() < 9.0e15 {
                *n = serde_json::Number::from(r as i64);
                return;
            }
            if let Some(m) = serde_json::Number::from_f64(if r == 0.0 { 0.0 } else { r }) {
                *n = m;
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(canonical),
        serde_json::Value::Object(o) => o.values_mut().for_each(canonical),
        _ => {}
    }
}

/// Number of pyramid levels of a `w` x `h` mosaic in [`TILE`] tiles.
pub fn level_count(w: usize, h: usize) -> usize {
    let (mut w, mut h, mut n) = (w.max(1), h.max(1), 1);
    while w > TILE || h > TILE {
        w = w.div_ceil(2);
        h = h.div_ceil(2);
        n += 1;
    }
    n
}

/// The zip of a bundle: entries under `<project_id>.atlas/`, `atlas.json` first.
pub fn zip(project_id: &str, files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    zip::write(&format!("{project_id}.atlas/"), files)
}

/// Checks a bundle's files against its `atlas.json` (paths, sizes and hashes). Returns the
/// parsed `atlas.json`.
pub fn verify(files: &[(String, Vec<u8>)]) -> Result<serde_json::Value> {
    let strip = |p: &str| -> String { p.split_once(".atlas/").map_or(p.to_string(), |(_, rest)| rest.to_string()) };
    let map: BTreeMap<String, &Vec<u8>> = files.iter().map(|(p, b)| (strip(p), b)).collect();
    let atlas: serde_json::Value = serde_json::from_slice(map.get("atlas.json").ok_or_else(|| Error::Invalid("no atlas.json".into()))?).map_err(|e| Error::Invalid(format!("atlas.json: {e}")))?;
    if atlas["format"] != FORMAT {
        return Err(Error::Invalid("atlas.json: format is not grainatlas-atlas".into()));
    }
    for f in atlas["files"].as_array().ok_or_else(|| Error::Invalid("atlas.json: no files".into()))? {
        let p = f["path"].as_str().unwrap_or_default();
        let b = map.get(p).ok_or_else(|| Error::Invalid(format!("{p} is listed but missing")))?;
        if f["size"].as_u64() != Some(b.len() as u64) || f["sha256"].as_str() != Some(&crate::util::sha256_hex(b)) {
            return Err(Error::Invalid(format!("{p} does not match its hash")));
        }
    }
    Ok(atlas)
}
