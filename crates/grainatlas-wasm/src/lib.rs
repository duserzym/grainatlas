//! grainatlas-core for the browser: a thin wasm-bindgen wrapper around
//! `grainatlas_core::project::session::Session` (the desktop app wraps the same
//! type in Tauri commands). One session lives in a Web Worker; pixels stay in
//! wasm memory and leave as transferable buffers with an 8-byte w/h header.

use grainatlas_core::Ctl;
use grainatlas_core::project::AreaParams;
use grainatlas_core::project::session;
use wasm_bindgen::prelude::*;

fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn iso_now() -> String {
    js_sys::Date::new_0().to_iso_string().as_string().unwrap_or_default()
}

#[wasm_bindgen]
pub struct Session(session::Session);

#[wasm_bindgen]
impl Session {
    #[wasm_bindgen(constructor)]
    pub fn new(name: &str) -> Session {
        Session(session::Session::new(name))
    }
    pub fn add_file(&mut self, path: &str, bytes: Vec<u8>) {
        self.0.add_file(path, bytes);
    }
    pub fn file_count(&self) -> usize {
        self.0.mem.files.len()
    }
    /// `{"status":"opened","info":…}` or `{"status":"needs_scale","name":…,"images":[…]}`.
    pub fn open_package(&mut self) -> Result<String, JsError> {
        self.0.open_mem(&iso_now()).map_err(err)
    }
    /// Opens the added images as loose files with the user's scale (LooseSpec JSON).
    pub fn open_loose(&mut self, spec: &str) -> Result<String, JsError> {
        self.0.open_loose(spec, &iso_now()).map_err(err)
    }
    pub fn load_project(&mut self, json: &str) -> Result<(), JsError> {
        self.0.load_project(json).map_err(err)
    }
    pub fn project_json(&self) -> Result<String, JsError> {
        self.0.project_json().map_err(err)
    }
    pub fn navigation(&self) -> Vec<u8> {
        self.0.navigation()
    }
    pub fn tile_thumb(&self, area: usize, tile: usize, max: usize) -> Result<Vec<u8>, JsError> {
        self.0.tile_thumb(area, tile, max).map_err(err)
    }
    /// `progress(stage, fraction)` is called along the way; `cancel()` returning true stops it.
    pub fn stitch(&mut self, area_id: &str, params: &str, progress: &js_sys::Function, cancel: &js_sys::Function) -> Result<String, JsError> {
        let prog = |stage: &str, f: f32| {
            let _ = progress.call2(&JsValue::NULL, &JsValue::from_str(stage), &JsValue::from_f64(f as f64));
        };
        let canc = || cancel.call0(&JsValue::NULL).ok().and_then(|v| v.as_bool()).unwrap_or(false);
        let ctl = Ctl { cancel: &canc, progress: &prog };
        self.0.stitch(area_id, params, &ctl, &now_ms).map_err(err)
    }
    /// New parameters on a stitched area: re-tone, or re-run from the solved placement, or
    /// re-stitch when stitch parameters changed. Same summary JSON as `stitch`.
    pub fn recompute(&mut self, area_id: &str, params: &str, progress: &js_sys::Function, cancel: &js_sys::Function) -> Result<String, JsError> {
        let prog = |stage: &str, f: f32| {
            let _ = progress.call2(&JsValue::NULL, &JsValue::from_str(stage), &JsValue::from_f64(f as f64));
        };
        let canc = || cancel.call0(&JsValue::NULL).ok().and_then(|v| v.as_bool()).unwrap_or(false);
        let ctl = Ctl { cancel: &canc, progress: &prog };
        self.0.recompute(area_id, params, &ctl, &now_ms).map_err(err)
    }
    /// The whole mosaic at pyramid `level` with `params` (live preview while a control moves).
    pub fn preview(&mut self, area_id: &str, params: &str, level: usize) -> Result<Vec<u8>, JsError> {
        self.0.preview(area_id, params, level).map_err(err)
    }
    /// The UI's grains of an area, for per-grain tone normalisation.
    pub fn set_grains(&mut self, area_id: &str, grains: &str) -> Result<(), JsError> {
        self.0.set_grains(area_id, grains).map_err(err)
    }
    /// One-click Auto: `params` with background leveling on and an automatic tone.
    pub fn auto_params(&self, area_id: &str, params: &str) -> Result<String, JsError> {
        self.0.auto_params(area_id, params).map_err(err)
    }
    pub fn has_mosaic(&self, area_id: &str) -> bool {
        self.0.outputs.contains_key(area_id)
    }
    pub fn pyramid_tile(&self, area_id: &str, level: usize, col: usize, row: usize) -> Result<Vec<u8>, JsError> {
        self.0.pyramid_tile(area_id, level, col, row).map_err(err)
    }
    pub fn histogram(&self, area_id: &str) -> Result<Vec<u32>, JsError> {
        self.0.histogram(area_id).map_err(err)
    }
    pub fn retone(&mut self, area_id: &str, tone: &str) -> Result<String, JsError> {
        self.0.retone(area_id, tone).map_err(err)
    }
    pub fn export_png(&self, area_id: &str, tone: &str) -> Result<Vec<u8>, JsError> {
        self.0.export_png(area_id, tone).map_err(err)
    }
    pub fn segment(&mut self, area_id: &str, params: &str, previous: &str) -> Result<String, JsError> {
        self.0.segment(area_id, params, previous).map_err(err)
    }
    pub fn grain_at(&mut self, area_id: &str, x_um: f64, y_um: f64, params: &str) -> Result<String, JsError> {
        self.0.grain_at(area_id, x_um, y_um, params).map_err(err)
    }
    pub fn split_grain(&mut self, area_id: &str, grain: &str, line: Vec<f64>) -> Result<String, JsError> {
        if line.len() != 4 {
            return Err(JsError::new("line needs 4 numbers"));
        }
        self.0.split_grain(area_id, grain, [line[0], line[1], line[2], line[3]]).map_err(err)
    }
    pub fn merge_grains(&mut self, area_id: &str, grains: &str) -> Result<String, JsError> {
        self.0.merge_grains(area_id, grains).map_err(err)
    }
    pub fn refine_grain(&mut self, area_id: &str, grain: &str) -> Result<String, JsError> {
        self.0.refine_grain(area_id, grain).map_err(err)
    }
    pub fn export_grains(&self, area_id: &str, grains: &str) -> Result<String, JsError> {
        self.0.export_grains(area_id, grains).map_err(err)
    }
    pub fn export_spots(&self, spots: &str, grains: &str, format: &str) -> Result<String, JsError> {
        self.0.export_spots(spots, grains, format).map_err(err)
    }
    pub fn set_link(&mut self, mount_id: &str, samples: &str) -> Result<(), JsError> {
        self.0.set_link(mount_id, samples).map_err(err)
    }
    pub fn layout(&self, params: &str, grains: &str, spots: &str, labels: &str, tone: &str) -> Result<String, JsError> {
        self.0.layout(params, grains, spots, labels, tone).map_err(err)
    }
}

#[wasm_bindgen]
pub fn renumber(grains: &str, tolerance: f64, from: &str) -> Result<String, JsError> {
    session::renumber(grains, tolerance, from).map_err(err)
}

#[wasm_bindgen]
pub fn pdf_from_svgs(pages: &str) -> Result<Vec<u8>, JsError> {
    session::pdf_from_svgs(pages).map_err(err)
}

#[wasm_bindgen]
pub fn tone_lut(tone: &str) -> Result<Vec<u8>, JsError> {
    session::tone_lut(tone).map_err(err)
}

#[wasm_bindgen]
pub fn register(pairs: &str) -> Result<String, JsError> {
    session::register(pairs).map_err(err)
}

#[wasm_bindgen]
pub fn default_params() -> String {
    serde_json::to_string(&AreaParams::default()).unwrap_or_default()
}

#[wasm_bindgen]
pub fn new_id() -> String {
    grainatlas_core::project::new_id().to_string()
}
