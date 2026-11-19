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
pub struct Session(
    session::Session,
    Option<grainatlas_core::grain_export::Export>,
    grainatlas_core::atlas::zip::Stream,
    Option<grainatlas_core::mosaic_export::Export>,
);

#[wasm_bindgen]
impl Session {
    #[wasm_bindgen(constructor)]
    pub fn new(name: &str) -> Session {
        Session(session::Session::new(name), None, Default::default(), None)
    }
    pub fn add_file(&mut self, path: &str, bytes: Vec<u8>) {
        self.0.add_file(path, bytes);
    }
    pub fn memory_report(&self) -> String {
        serde_json::to_string(&self.0.memory_report()).expect("memory report")
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
    pub fn load_bundle_area(
        &mut self,
        area_id: &str,
        w: usize,
        h: usize,
        pixels: Vec<u8>,
    ) -> Result<String, JsError> {
        self.0.load_bundle_area(area_id, w, h, pixels).map_err(err)
    }
    pub fn set_package_digest(&mut self, sha256: &str, files: usize) {
        self.0.set_package_digest(sha256, files);
    }
    pub fn bake_bundle_area(
        &mut self,
        area_id: &str,
        display_tone: &str,
    ) -> Result<String, JsError> {
        self.0.bake_bundle_area(area_id, display_tone).map_err(err)
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
    pub fn auto_params(&mut self, area_id: &str, params: &str) -> Result<String, JsError> {
        self.0.auto_params(area_id, params).map_err(err)
    }
    pub fn has_mosaic(&self, area_id: &str) -> bool {
        self.0.outputs.contains_key(area_id)
    }
    pub fn pyramid_tile(&self, area_id: &str, level: usize, col: usize, row: usize) -> Result<Vec<u8>, JsError> {
        self.0.pyramid_tile(area_id, level, col, row).map_err(err)
    }
    pub fn histogram(&mut self, area_id: &str) -> Result<Vec<u32>, JsError> {
        self.0.histogram(area_id).map_err(err)
    }
    pub fn retone(&mut self, area_id: &str, tone: &str) -> Result<String, JsError> {
        self.0.retone(area_id, tone).map_err(err)
    }
    pub fn export_png(&self, area_id: &str, tone: &str) -> Result<Vec<u8>, JsError> {
        self.0.export_png(area_id, tone).map_err(err)
    }
    pub fn area_signal(&self, area_id: &str) -> Result<String, JsError> {
        self.0.area_signal(area_id).map_err(err)
    }
    pub fn segment_warning(&mut self, area_id: &str, params: &str, grains: &str) -> Result<String, JsError> {
        self.0.segment_warning(area_id, params, grains).map_err(err)
    }
    pub fn segment(&mut self, area_id: &str, params: &str, previous: &str, outline: &str) -> Result<String, JsError> {
        self.0.segment(area_id, params, previous, outline).map_err(err)
    }
    pub fn segment_missing(&mut self, area_id: &str, params: &str, existing: &str, outline: &str) -> Result<String, JsError> {
        self.0.segment_missing(area_id, params, existing, outline).map_err(err)
    }
    pub fn grain_at(&mut self, area_id: &str, x_um: f64, y_um: f64, params: &str, outline: &str) -> Result<String, JsError> {
        self.0.grain_at(area_id, x_um, y_um, params, outline).map_err(err)
    }
    pub fn split_grain(&mut self, area_id: &str, grain: &str, path_um: Vec<f64>) -> Result<String, JsError> {
        if path_um.len() < 4 || !path_um.len().is_multiple_of(2) || path_um.iter().any(|v| !v.is_finite()) {
            return Err(JsError::new("A cut needs at least two finite points (an even number of coordinates)."));
        }
        let path_um = path_um.as_chunks::<2>().0.to_vec();
        self.0.split_grain(area_id, grain, path_um).map_err(err)
    }
    pub fn merge_grains(&mut self, area_id: &str, grains: &str) -> Result<String, JsError> {
        self.0.merge_grains(area_id, grains).map_err(err)
    }
    pub fn refine_grain(&mut self, area_id: &str, grain: &str) -> Result<String, JsError> {
        self.0.refine_grain(area_id, grain).map_err(err)
    }
    pub fn snap_outline(&mut self, area_id: &str, path: &str, width_um: f64, contrast: f64) -> Result<String, JsError> {
        self.0.snap_outline(area_id, path, width_um, contrast).map_err(err)
    }
    pub fn outline_grain(&mut self, area_id: &str, polygon: &str, base: &str, op: &str, outline: &str) -> Result<String, JsError> {
        self.0.outline_grain(area_id, polygon, base, op, outline).map_err(err)
    }
    pub fn brush_grain(&mut self, area_id: &str, stroke: &str, size_um: f64, base: &str, op: &str) -> Result<String, JsError> {
        self.0.brush_grain(area_id, stroke, size_um, base, op).map_err(err)
    }
    pub fn mosaic_export_plan(&self, project: &str, options: &str) -> Result<String, JsError> {
        let job = grainatlas_core::mosaic_export::Export::begin(&self.0, project, options)
            .map_err(err)?;
        serde_json::to_string(&job.plan).map_err(err)
    }
    pub fn mosaic_export_begin(&mut self, project: &str, options: &str) -> Result<String, JsError> {
        self.3 = None;
        let job = grainatlas_core::mosaic_export::Export::begin(&self.0, project, options)
            .map_err(err)?;
        let plan = serde_json::to_string(&job.plan).map_err(err)?;
        self.3 = Some(job);
        Ok(plan)
    }
    pub fn mosaic_export_abort(&mut self) {
        self.3 = None;
        self.0.finish_mosaic_export();
    }
    pub fn mosaic_export_next(&mut self) -> Result<JsValue, JsError> {
        let job = self.3.as_mut().ok_or_else(|| err("No mosaic export"))?;
        let chunk = job.next(&mut self.0, &Ctl::NONE).map_err(err)?;
        let (done, total) = job.progress();
        let out = js_sys::Object::new();
        let set = |key: &str, value: JsValue| {
            js_sys::Reflect::set(&out, &JsValue::from_str(key), &value).map_err(|_| err("Could not construct mosaic export chunk"))
        };
        if let Some(chunk) = chunk {
            set("name", JsValue::from_str(&chunk.name))?;
            set(
                "data",
                js_sys::Uint8Array::from(chunk.data.as_slice()).into(),
            )?;
            set("last", JsValue::from_bool(chunk.last))?;
        }
        set("done", JsValue::from_bool(job.done()))?;
        set("tiles_done", JsValue::from_f64(done as f64))?;
        set("tiles_total", JsValue::from_f64(total as f64))?;
        Ok(out.into())
    }
    pub fn grain_export_begin(&mut self, project: &str, options: &str) -> Result<(), JsError> {
        self.grain_export_abort();
        self.1 = Some(
            grainatlas_core::grain_export::Export::begin(&self.0, project, options).map_err(err)?,
        );
        Ok(())
    }
    pub fn grain_export_abort(&mut self) {
        self.2 = Default::default();
        if let Some(job) = self.1.take() {
            job.abort(&mut self.0);
        }
    }
    pub fn grain_export_done(&self) -> bool {
        self.1.is_none()
    }
    pub fn grain_export_batch(
        &mut self,
        progress: &js_sys::Function,
        zip_stream: bool,
    ) -> Result<Vec<u8>, JsError> {
        let prog = |stage: &str, f: f32| {
            let _ = progress.call2(
                &JsValue::NULL,
                &JsValue::from_str(stage),
                &JsValue::from_f64(f as f64),
            );
        };
        let ctl = Ctl {
            cancel: &|| false,
            progress: &prog,
        };
        let job = self.1.as_mut().ok_or_else(|| err("no grain export"))?;
        let files = if job.is_ready() {
            self.1.take().unwrap().finish(&ctl).map_err(err)?
        } else {
            job.batch(&mut self.0, &ctl).map_err(err)?.files
        };
        if zip_stream {
            let mut out = Vec::new();
            for (name, bytes) in files {
                out.extend(self.2.entry(&name, &bytes).map_err(err)?);
            }
            if self.1.is_none() {
                out.extend(std::mem::take(&mut self.2).finish());
            }
            Ok(out)
        } else {
            grainatlas_core::atlas::zip::write("", &files).map_err(err)
        }
    }
    pub fn export_grains(&self, area_id: &str, grains: &str) -> Result<String, JsError> {
        self.0.export_grains(area_id, grains).map_err(err)
    }
    pub fn check_project(&self, project: &str, labels: &str) -> Result<String, JsError> {
        self.0.check_project(project, labels).map_err(err)
    }
    pub fn export_spots(&self, spots: &str, grains: &str, format: &str) -> Result<String, JsError> {
        self.0.export_spots(spots, grains, format).map_err(err)
    }
    pub fn set_link(&mut self, mount_id: &str, samples: &str) -> Result<(), JsError> {
        self.0.set_link(mount_id, samples).map_err(err)
    }
    /// The atlas bundle as a zip (`<project_id>.atlas/…`) for a full project JSON and
    /// BundleOptions JSON.
    pub fn atlas_zip(&self, project: &str, options: &str) -> Result<Vec<u8>, JsError> {
        self.0.atlas_zip(project, options).map_err(err)
    }
    /// Stores an area's laser registration (AreaRegistration JSON; "" removes it).
    pub fn set_laser(&mut self, area_id: &str, registration: &str) -> Result<(), JsError> {
        self.0.set_laser(area_id, registration).map_err(err)
    }
    /// The area's spot list for the laser software.
    pub fn laser_export(&self, spots: &str, area_id: &str, adapter: &str) -> Result<String, JsError> {
        self.0.laser_export(spots, area_id, adapter).map_err(err)
    }
    /// Which area owns each detection (`{area_id: [Grain]}` JSON; OwnershipParams JSON or "").
    pub fn ownership(&self, grains: &str, params: &str) -> Result<String, JsError> {
        self.0.ownership(grains, params).map_err(err)
    }
    /// Print sheets of the mount (SheetParams, grains and tone JSON): pages, the print_sheet
    /// registration, stats and ownership.
    pub fn print_sheets(&self, params: &str, grains: &str, tone: &str) -> Result<String, JsError> {
        self.0.print_sheets(params, grains, tone).map_err(err)
    }
    pub fn print_sheets_with_edits(&self, params: &str, grains: &str, tone: &str, edits: &str) -> Result<String, JsError> {
        self.0.print_sheets_preview(params, grains, tone, edits).map_err(err)
    }
    pub fn print_plan_render(&self, plan: &str, edits: &str, grains: &str, tone: &str) -> Result<String, JsError> {
        self.0.print_plan_preview(plan, edits, grains, tone).map_err(err)
    }
    pub fn print_band_image(&self, area_id: &str, mosaic_rect_um: &str, tone: &str) -> Result<Vec<u8>, JsError> {
        let rect = serde_json::from_str(mosaic_rect_um).map_err(err)?;
        self.0.print_band_preview(area_id, rect, tone).map_err(err)
    }
    /// Whole area on one sheet (WholeParams and tone JSON); no grains required.
    pub fn print_whole(&self, params: &str, tone: &str) -> Result<String, JsError> {
        self.0.print_whole_preview(params, tone).map_err(err)
    }
    pub fn print_task(&self, job: usize, action: &str, page: usize) -> Result<Vec<u8>, JsError> {
        self.0.print_task(job, action, page).map_err(err)
    }
    /// Header metadata of the open package for print sheets.
    pub fn sheet_meta(&self) -> Result<String, JsError> {
        self.0.sheet_meta().map_err(err)
    }
    /// Records a publish (Published JSON) in the project.
    pub fn record_published(&mut self, published: &str) -> Result<(), JsError> {
        self.0.record_published(published).map_err(err)
    }
    /// Fixed project and area ids, `created` and package path (parity checks).
    pub fn pin_identity(&mut self, project_id: &str, created: &str) -> Result<(), JsError> {
        self.0.pin_identity(project_id, created).map_err(err)
    }
    pub fn layout(&self, params: &str, grains: &str, spots: &str, labels: &str, tone: &str) -> Result<String, JsError> {
        self.0.layout_preview(params, grains, spots, labels, tone).map_err(err)
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
pub fn laser_fit(fiducials: &str, units: &str) -> Result<String, JsError> {
    session::laser_fit(fiducials, units).map_err(err)
}

#[wasm_bindgen]
pub fn laser_adapters() -> String {
    session::laser_adapters()
}

#[wasm_bindgen]
pub fn default_params() -> String {
    serde_json::to_string(&AreaParams::default()).unwrap_or_default()
}

#[wasm_bindgen]
pub fn new_id() -> String {
    grainatlas_core::project::new_id().to_string()
}
