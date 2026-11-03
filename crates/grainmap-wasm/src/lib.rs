//! grainmap-core for the browser. One `Session` per opened package lives in a
//! Web Worker; pixels stay in wasm memory and leave as transferable buffers.
//!
//! Buffers that carry an image start with an 8-byte header: width and height
//! as little-endian u32.

use std::collections::BTreeMap;
use std::sync::Arc;

use grainmap_core::Ctl;
use grainmap_core::photometric::ToneParams;
use grainmap_core::project::pipeline::{self, AreaOutput, Timings};
use grainmap_core::project::{AreaParams, Project};
use grainmap_core::readers::{self, Package};
use grainmap_core::segment::{Grain, SegmentParams};
use grainmap_core::source::MemSource;
use wasm_bindgen::prelude::*;

fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn with_header(w: usize, h: usize, px: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + px.len());
    out.extend_from_slice(&(w as u32).to_le_bytes());
    out.extend_from_slice(&(h as u32).to_le_bytes());
    out.extend_from_slice(px);
    out
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn iso_now() -> String {
    js_sys::Date::new_0().to_iso_string().as_string().unwrap_or_default()
}

#[wasm_bindgen]
pub struct Session {
    src: MemSource,
    pkg: Option<Box<dyn Package>>,
    project: Option<Project>,
    outputs: BTreeMap<String, AreaOutput>,
    /// Segmentation work images per area (and work scale).
    work: BTreeMap<String, (usize, grainmap_core::segment::WorkImage)>,
}

#[wasm_bindgen]
impl Session {
    #[wasm_bindgen(constructor)]
    pub fn new(name: &str) -> Session {
        Session { src: MemSource::new(name), pkg: None, project: None, outputs: BTreeMap::new(), work: BTreeMap::new() }
    }

    /// Adds one file of the package (path relative to the package root).
    pub fn add_file(&mut self, path: &str, bytes: Vec<u8>) {
        self.src.insert(path, bytes);
    }

    /// Paths of all files added so far.
    pub fn file_count(&self) -> usize {
        self.src.files.len()
    }

    /// Opens the package with the first reader that recognises it; returns `PackageInfo` JSON and
    /// starts a new project (unless one was loaded with `load_project`).
    pub fn open_package(&mut self) -> Result<String, JsError> {
        let pkg = readers::open(Arc::new(self.src.clone())).map_err(err)?;
        let info = serde_json::to_string(pkg.info()).map_err(err)?;
        if self.project.is_none() {
            self.project = Some(Project::new(pkg.info(), &self.src.name, &iso_now()));
        }
        self.pkg = Some(pkg);
        Ok(info)
    }

    /// Replaces the project (re-opening a saved `project.json`).
    pub fn load_project(&mut self, json: &str) -> Result<(), JsError> {
        self.project = Some(Project::from_json(json).map_err(err)?);
        Ok(())
    }

    pub fn project_json(&self) -> Result<String, JsError> {
        self.project.as_ref().map(|p| p.to_json()).ok_or_else(|| JsError::new("no project"))
    }

    /// Navigation image of the package as RGBA-free grey with header, or empty.
    pub fn navigation(&self) -> Vec<u8> {
        let Some(pkg) = &self.pkg else { return Vec::new() };
        let Some(nav) = &pkg.info().navigation else { return Vec::new() };
        let Ok(bytes) = self.src.read_file(&nav.file) else { return Vec::new() };
        match grainmap_core::image::decode(&bytes) {
            Ok((g, _)) => with_header(g.w, g.h, &g.to_u8()),
            Err(_) => Vec::new(),
        }
    }

    /// A tile reduced so its longer side is at most `max` px (Area screen previews).
    pub fn tile_thumb(&self, area: usize, tile: usize, max: usize) -> Result<Vec<u8>, JsError> {
        let pkg = self.pkg.as_ref().ok_or_else(|| JsError::new("no package"))?;
        let t = pkg.tile(area, tile).map_err(err)?;
        let k = t.image.w.max(t.image.h).div_ceil(max.max(1)).max(1);
        let g = t.image.downsample(k);
        Ok(with_header(g.w, g.h, &g.to_u8()))
    }

    /// Runs the whole pipeline for one area. `params` is `AreaParams` JSON ("" for defaults).
    /// `progress(stage, fraction)` is called along the way; `cancel()` returning true stops it.
    pub fn stitch(&mut self, area_id: &str, params: &str, progress: &js_sys::Function, cancel: &js_sys::Function) -> Result<String, JsError> {
        let pkg = self.pkg.as_ref().ok_or_else(|| JsError::new("no package"))?;
        let project = self.project.as_mut().ok_or_else(|| JsError::new("no project"))?;
        let ai = pkg.info().areas.iter().position(|a| a.id == area_id).ok_or_else(|| JsError::new("no such area"))?;
        let params: AreaParams = if params.trim().is_empty() {
            project.areas.iter().find(|a| a.id == area_id).map(|a| a.params.clone()).unwrap_or_default()
        } else {
            serde_json::from_str(params).map_err(err)?
        };
        let rec = project.area_mut(area_id).ok_or_else(|| JsError::new("area not in project"))?;
        let prog = |stage: &str, f: f32| {
            let _ = progress.call2(&JsValue::NULL, &JsValue::from_str(stage), &JsValue::from_f64(f as f64));
        };
        let canc = || cancel.call0(&JsValue::NULL).ok().and_then(|v| v.as_bool()).unwrap_or(false);
        let ctl = Ctl { cancel: &canc, progress: &prog };
        let mut timings = Timings::default();
        let t0 = now_ms();
        prog("read", 0.0);
        let tiles = pipeline::read_tiles(pkg.as_ref(), ai, rec).map_err(err)?;
        timings.steps.push(("read".into(), now_ms() - t0));
        let out = pipeline::run(rec, &tiles, &params, &ctl, &now_ms, &mut timings).map_err(err)?;
        let summary = serde_json::json!({
            "mosaic": out.mosaic_json,
            "placement": out.placement,
            "photometric": rec.photometric,
            "timings": timings.steps,
        });
        self.outputs.insert(area_id.to_string(), out);
        self.work.remove(area_id);
        Ok(summary.to_string())
    }

    pub fn has_mosaic(&self, area_id: &str) -> bool {
        self.outputs.contains_key(area_id)
    }

    /// One pyramid tile (raw grey with header).
    pub fn pyramid_tile(&self, area_id: &str, level: usize, col: usize, row: usize) -> Result<Vec<u8>, JsError> {
        let o = self.outputs.get(area_id).ok_or_else(|| JsError::new("not stitched"))?;
        let (w, h, px) = o.pyramid.tile(level, col, row).ok_or_else(|| JsError::new("tile out of range"))?;
        Ok(with_header(w, h, &px))
    }

    /// 256-bin histogram of the base mosaic (covered pixels).
    pub fn histogram(&self, area_id: &str) -> Result<Vec<u32>, JsError> {
        let o = self.outputs.get(area_id).ok_or_else(|| JsError::new("not stitched"))?;
        let mut h = vec![0u32; 256];
        for (v, c) in o.base.iter().zip(&o.coverage) {
            if *c >= 0.5 {
                h[*v as usize] += 1;
            }
        }
        Ok(h)
    }

    /// Re-tones the base mosaic and pyramid (stretch percentiles, CLAHE) without re-stitching.
    pub fn retone(&mut self, area_id: &str, tone: &str) -> Result<String, JsError> {
        let tone: ToneParams = serde_json::from_str(tone).map_err(err)?;
        let project = self.project.as_mut().ok_or_else(|| JsError::new("no project"))?;
        let rec = project.area_mut(area_id).ok_or_else(|| JsError::new("no such area"))?;
        let o = self.outputs.get_mut(area_id).ok_or_else(|| JsError::new("not stitched"))?;
        pipeline::retone(rec, o, &tone);
        serde_json::to_string(&o.mosaic_json).map_err(err)
    }

    /// Level-0 mosaic as an 8-bit grey PNG with the black / white / gamma LUT applied.
    pub fn export_png(&self, area_id: &str, tone: &str) -> Result<Vec<u8>, JsError> {
        let tone: ToneParams = serde_json::from_str(tone).map_err(err)?;
        let o = self.outputs.get(area_id).ok_or_else(|| JsError::new("not stitched"))?;
        let lut = grainmap_core::photometric::tone::lut(&tone);
        let px: Vec<u8> = o.base.iter().map(|&v| lut[v as usize]).collect();
        Ok(grainmap_core::image::encode_png_gray(o.linear.w, o.linear.h, &px))
    }

    /// Project folder files for one area (`mosaic/<area>/...`), as JSON `[[path, base64?]]` is
    /// avoided: returns the list of paths; fetch each with `area_file`.
    pub fn area_file_list(&self, area_id: &str) -> Result<String, JsError> {
        let project = self.project.as_ref().ok_or_else(|| JsError::new("no project"))?;
        let rec = project.areas.iter().find(|a| a.id == area_id).ok_or_else(|| JsError::new("no such area"))?;
        let o = self.outputs.get(area_id).ok_or_else(|| JsError::new("not stitched"))?;
        let names: Vec<String> = pipeline::area_files(rec, o).into_iter().map(|(p, _)| p).collect();
        serde_json::to_string(&names).map_err(err)
    }
}

#[wasm_bindgen]
impl Session {
    fn work_for(&mut self, area_id: &str, scale: usize) -> Result<&grainmap_core::segment::WorkImage, JsError> {
        let fresh = !matches!(self.work.get(area_id), Some((s, _)) if *s == scale);
        if fresh {
            let o = self.outputs.get(area_id).ok_or_else(|| JsError::new("not stitched"))?;
            let rec = self.project.as_ref().and_then(|p| p.areas.iter().find(|a| a.id == area_id)).ok_or_else(|| JsError::new("no such area"))?;
            self.work.insert(area_id.to_string(), (scale, pipeline::work_image(o, rec, scale)));
        }
        Ok(&self.work.get(area_id).expect("inserted").1)
    }

    /// Automatic segmentation; `previous` (JSON array of grains) keeps ids and numbers.
    pub fn segment(&mut self, area_id: &str, params: &str, previous: &str) -> Result<String, JsError> {
        let p: SegmentParams = serde_json::from_str(params).map_err(err)?;
        let prev: Vec<Grain> = if previous.trim().is_empty() { Vec::new() } else { serde_json::from_str(previous).map_err(err)? };
        let grains = {
            let w = self.work_for(area_id, p.work_scale)?;
            grainmap_core::segment::segment(w, &p, &prev)
        };
        if let Some(rec) = self.project.as_mut().and_then(|pr| pr.area_mut(area_id)) {
            rec.params.segment = p;
        }
        serde_json::to_string(&grains).map_err(err)
    }

    /// Click-grow a grain at a point (um); "null" on background.
    pub fn grain_at(&mut self, area_id: &str, x_um: f64, y_um: f64, params: &str) -> Result<String, JsError> {
        let p: SegmentParams = serde_json::from_str(params).map_err(err)?;
        let w = self.work_for(area_id, p.work_scale)?;
        serde_json::to_string(&grainmap_core::segment::grain_at(w, &p, x_um, y_um)).map_err(err)
    }

    /// Splits a grain along a line [x0, y0, x1, y1] in um.
    pub fn split_grain(&mut self, area_id: &str, grain: &str, line: Vec<f64>) -> Result<String, JsError> {
        let g: Grain = serde_json::from_str(grain).map_err(err)?;
        let scale = self.seg_scale(area_id);
        let w = self.work_for(area_id, scale)?;
        let l = [line[0], line[1], line[2], line[3]];
        serde_json::to_string(&grainmap_core::segment::split_grain(w, &g, l, 0.8)).map_err(err)
    }

    pub fn merge_grains(&mut self, area_id: &str, grains: &str) -> Result<String, JsError> {
        let gs: Vec<Grain> = serde_json::from_str(grains).map_err(err)?;
        let scale = self.seg_scale(area_id);
        let w = self.work_for(area_id, scale)?;
        serde_json::to_string(&grainmap_core::segment::merge_grains(w, &gs, 0.8)).map_err(err)
    }

    pub fn refine_grain(&mut self, area_id: &str, grain: &str) -> Result<String, JsError> {
        let g: Grain = serde_json::from_str(grain).map_err(err)?;
        let scale = self.seg_scale(area_id);
        let w = self.work_for(area_id, scale)?;
        let out = grainmap_core::segment::refine_grain(w, &g, 0.8).unwrap_or(g);
        serde_json::to_string(&out).map_err(err)
    }

    /// Grain table CSV with stage positions.
    pub fn export_grains(&self, area_id: &str, grains: &str) -> Result<String, JsError> {
        let gs: Vec<Grain> = serde_json::from_str(grains).map_err(err)?;
        let rec = self.project.as_ref().and_then(|p| p.areas.iter().find(|a| a.id == area_id)).ok_or_else(|| JsError::new("no such area"))?;
        Ok(grainmap_core::segment::grains_csv(&gs, |x, y| rec.stage_of(x, y), |_| 0))
    }

    /// Spot export: `format` is "csv" or "geojson"; `grains` maps area id → grains.
    pub fn export_spots(&self, spots: &str, grains: &str, format: &str) -> Result<String, JsError> {
        let spots: Vec<grainmap_core::spots::Spot> = serde_json::from_str(spots).map_err(err)?;
        let project = self.project.as_ref().ok_or_else(|| JsError::new("no project"))?;
        match format {
            "csv" => Ok(grainmap_core::spots::spots_csv(project, &spots)),
            _ => {
                let grains: BTreeMap<String, Vec<Grain>> = serde_json::from_str(grains).map_err(err)?;
                serde_json::to_string_pretty(&grainmap_core::spots::geojson(project, &grains, &spots)).map_err(err)
            }
        }
    }

    /// Sets the project's laicpmsdb mount id ("" clears it) and area sample names.
    pub fn set_link(&mut self, mount_id: &str, samples: &str) -> Result<(), JsError> {
        let project = self.project.as_mut().ok_or_else(|| JsError::new("no project"))?;
        project.laicpmsdb.mount_id = if mount_id.trim().is_empty() { None } else { Some(mount_id.trim().parse().map_err(err)?) };
        if !samples.trim().is_empty() {
            let m: BTreeMap<String, String> = serde_json::from_str(samples).map_err(err)?;
            for a in project.areas.iter_mut() {
                if let Some(s) = m.get(&a.id) {
                    a.sample = s.clone();
                }
            }
        }
        Ok(())
    }

    /// Print layout: SVG pages as JSON `{pages, um_per_mm, items, paper_mm}`.
    pub fn layout(&self, params: &str, grains: &str, spots: &str, labels: &str, tone: &str) -> Result<String, JsError> {
        let p: grainmap_core::layout::LayoutParams = serde_json::from_str(params).map_err(err)?;
        let grains: BTreeMap<String, Vec<Grain>> = serde_json::from_str(grains).map_err(err)?;
        let spots: Vec<grainmap_core::spots::Spot> = serde_json::from_str(spots).map_err(err)?;
        let labels: Vec<grainmap_core::spots::Label> = serde_json::from_str(labels).map_err(err)?;
        let tone: BTreeMap<String, ToneParams> = serde_json::from_str(tone).map_err(err)?;
        let project = self.project.as_ref().ok_or_else(|| JsError::new("no project"))?;
        let imgs: Vec<grainmap_core::layout::AreaImage> = project
            .areas
            .iter()
            .filter_map(|rec| {
                let o = self.outputs.get(&rec.id)?;
                Some(pipeline::area_image(rec, o, tone.get(&rec.id).unwrap_or(&rec.params.tone)))
            })
            .collect();
        serde_json::to_string(&grainmap_core::layout::layout(&p, &imgs, &grains, &spots, &labels)).map_err(err)
    }

    fn seg_scale(&self, area_id: &str) -> usize {
        self.project.as_ref().and_then(|p| p.areas.iter().find(|a| a.id == area_id)).map_or(2, |a| a.params.segment.work_scale)
    }
}

/// Renumbers grains in reading order (from = {"grainId": uuid, "number": n} for "from here").
#[wasm_bindgen]
pub fn renumber(grains: &str, tolerance: f64, from: &str) -> Result<String, JsError> {
    let mut gs: Vec<Grain> = serde_json::from_str(grains).map_err(err)?;
    let f = if from.trim().is_empty() {
        None
    } else {
        let v: serde_json::Value = serde_json::from_str(from).map_err(err)?;
        let id = v["grainId"].as_str().and_then(|s| s.parse().ok()).ok_or_else(|| JsError::new("grainId"))?;
        Some((id, v["number"].as_u64().unwrap_or(1) as u32))
    };
    grainmap_core::segment::renumber(&mut gs, tolerance, f);
    serde_json::to_string(&gs).map_err(err)
}

/// SVG pages (JSON array of strings) → PDF bytes.
#[wasm_bindgen]
pub fn pdf_from_svgs(pages: &str) -> Result<Vec<u8>, JsError> {
    let pages: Vec<String> = serde_json::from_str(pages).map_err(err)?;
    grainmap_core::layout::pdf::pdf_from_svgs(&pages).map_err(err)
}

/// Black / white / gamma lookup table (256 bytes) for display.
#[wasm_bindgen]
pub fn tone_lut(tone: &str) -> Result<Vec<u8>, JsError> {
    let tone: ToneParams = serde_json::from_str(tone).map_err(err)?;
    Ok(grainmap_core::photometric::tone::lut(&tone).to_vec())
}

/// Default `AreaParams` as JSON (the UI's starting point).
#[wasm_bindgen]
pub fn default_params() -> String {
    serde_json::to_string(&AreaParams::default()).unwrap_or_default()
}

/// A new UUIDv7.
#[wasm_bindgen]
pub fn new_id() -> String {
    grainmap_core::project::new_id().to_string()
}

trait ReadFile {
    fn read_file(&self, p: &str) -> grainmap_core::Result<Vec<u8>>;
}
impl ReadFile for MemSource {
    fn read_file(&self, p: &str) -> grainmap_core::Result<Vec<u8>> {
        use grainmap_core::source::Source;
        self.read(p)
    }
}
