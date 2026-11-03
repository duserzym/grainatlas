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
    #[allow(dead_code)]
    extra: BTreeMap<String, String>,
}

#[wasm_bindgen]
impl Session {
    #[wasm_bindgen(constructor)]
    pub fn new(name: &str) -> Session {
        Session { src: MemSource::new(name), pkg: None, project: None, outputs: BTreeMap::new(), extra: BTreeMap::new() }
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
