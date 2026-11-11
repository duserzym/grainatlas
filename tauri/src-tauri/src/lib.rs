//! grainatlas desktop: the same Svelte UI, with the core running natively
//! (rayon, all cores) behind Tauri commands that mirror the wasm worker API.
//! Packages are read from disk by path; results are saved with native dialogs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use grainatlas_core::project::session::{self, Session};
use grainatlas_core::source::{DirSource, FileListSource, Source};
use serde::Serialize;
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

mod geometry;
mod memory;
mod profile;
mod recent;
mod updater;
use tauri_plugin_dialog::DialogExt;

/// One open project: a window holds several, as tabs (the UI names them by number).
struct Tab {
    session: Mutex<Session>,
    /// Last completed allocation count, also readable while a command is busy.
    evictable: AtomicUsize,
    cancel: Arc<AtomicBool>,
    package_path: Mutex<Option<PathBuf>>,
}

impl Tab {
    fn new() -> Self {
        Self { session: Mutex::new(Session::new("package")), evictable: AtomicUsize::new(0), cancel: Arc::new(AtomicBool::new(false)), package_path: Mutex::new(None) }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn package_dir(&self) -> Option<PathBuf> {
        self.package_path.lock().unwrap_or_else(|x| x.into_inner()).as_ref().and_then(|p| p.parent().map(|q| q.to_path_buf()))
    }
}

/// The sessions of the window's tabs, by tab id. A tab's session is made on its first command
/// and dropped by `close_tab`; each has its own lock, so tabs stitch and edit independently.
#[derive(Clone, Default)]
struct AppState {
    tabs: Arc<Mutex<HashMap<u32, Arc<Tab>>>>,
    /// Paths from the OS's open-file events not yet taken by the page (`take_opened`).
    opened: Arc<Mutex<Vec<String>>>,
    /// Serializes short eviction passes, never running commands.
    memory_trim: Arc<Mutex<()>>,
}

impl AppState {
    fn tab(&self, id: u32) -> Arc<Tab> {
        self.tabs.lock().unwrap_or_else(|p| p.into_inner()).entry(id).or_insert_with(|| Arc::new(Tab::new())).clone()
    }
}

type Res<T> = Result<T, String>;

fn e(x: impl std::fmt::Display) -> String {
    x.to_string()
}

fn now_iso() -> String {
    // UTC RFC 3339 from the system clock (civil-from-days, H. Hinnant).
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (s.div_euclid(86400), s.rem_euclid(86400));
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[derive(Clone, Serialize)]
struct Progress {
    tab: u32,
    stage: String,
    fraction: f32,
}

/// Opens one folder (a package or an area folder) or a set of files (loose images, or the
/// files of an area). Returns the core's probe JSON: opened, or needs a scale.
#[tauri::command]
async fn open_paths(state: State<'_, AppState>, tab: u32, paths: Vec<String>) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        let ps: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let (src, path, files): (Arc<dyn Source>, PathBuf, Vec<String>) = match ps.as_slice() {
            [one] if one.is_dir() => (Arc::new(DirSource::new(one).map_err(e)?), one.clone(), Vec::new()),
            [] => return Err("nothing chosen".into()),
            _ => {
                let dir = ps[0].parent().map(|p| p.to_path_buf()).unwrap_or_default();
                (Arc::new(FileListSource::new(&ps).map_err(e)?), dir, paths.clone())
            }
        };
        let out = {
            let mut s = t.lock();
            s.origin_files = files;
            let out = s.open_source(src, &path.display().to_string(), &now_iso()).map_err(e)?;
            t.evictable.store(0, Ordering::Relaxed);
            out
        };
        *t.package_path.lock().unwrap_or_else(|x| x.into_inner()) = Some(path);
        Ok(out)
    }).await.map_err(e)?
}

#[tauri::command]
async fn open_path(state: State<'_, AppState>, tab: u32, path: String) -> Res<String> {
    open_paths(state, tab, vec![path]).await
}

/// Loose images of the last open, with the scale the user gave (LooseSpec JSON).
#[tauri::command]
async fn open_loose(state: State<'_, AppState>, tab: u32, spec: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(tab, |s| s.open_loose(&spec, &now_iso())).map_err(e)
    }).await.map_err(e)?
}

/// Opens a project: a `<name>.grainatlas/` folder (its project.json) or a `<name>.gaproj` project
/// file (the same JSON on its own), with its package by the recorded path. Returns `{ "info":
/// PackageInfo, "project": full project JSON }`; the UI then renders the stitched areas from
/// their saved placements.
#[tauri::command]
async fn open_project(state: State<'_, AppState>, tab: u32, path: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        let file = PathBuf::from(&path);
        let file = if file.is_dir() { file.join("project.json") } else { file };
        let modified_ms = std::fs::metadata(&file).and_then(|m| m.modified()).map_err(e)?
            .duration_since(std::time::UNIX_EPOCH).map_err(e)?.as_millis();
        let pj = std::fs::read_to_string(&file).map_err(|x| format!("{path}: {x}"))?;
        let v: serde_json::Value = serde_json::from_str(&pj).map_err(e)?;
        let pkg = v["package"]["path"].as_str().ok_or("project.json has no package path")?.to_string();
        // Loose images reopen from their recorded paths with the recorded scale.
        let loose_files: Vec<PathBuf> = v["package"]["files"].as_array().map(|a| a.iter().filter_map(|f| f.as_str().map(PathBuf::from)).collect()).unwrap_or_default();
        let is_loose = v["package"]["kind"] == "loose";
        let src: Arc<dyn Source> = if is_loose && !loose_files.is_empty() {
            Arc::new(FileListSource::new(&loose_files).map_err(|x| format!("images of {pkg}: {x}"))?)
        } else {
            Arc::new(DirSource::new(&pkg).map_err(|x| format!("package {pkg}: {x}"))?)
        };
        let info = {
            let mut s = t.lock();
            s.load_project(&pj).map_err(e)?;
            s.project_path = file.canonicalize().map_err(e)?.display().to_string();
            s.origin_files = if is_loose { loose_files.iter().map(|p| p.display().to_string()).collect() } else { Vec::new() };
            let probe = s.open_source(src, &pkg, &now_iso()).map_err(e)?;
            t.evictable.store(0, Ordering::Relaxed);
            if is_loose {
                let spec = serde_json::to_string(&v["package"]["loose"]).map_err(e)?;
                s.open_loose(&spec, &now_iso()).map_err(e)?
            } else {
                let p: serde_json::Value = serde_json::from_str(&probe).map_err(e)?;
                if p["status"] != "opened" {
                    return Err(format!("{pkg} is not a package any more"));
                }
                p["info"].to_string()
            }
        };
        *t.package_path.lock().unwrap_or_else(|x| x.into_inner()) = Some(PathBuf::from(&pkg));
        Ok(serde_json::json!({ "info": serde_json::from_str::<serde_json::Value>(&info).map_err(e)?, "project": v, "modified_ms": modified_ms }).to_string())
    }).await.map_err(e)?
}

#[tauri::command]
async fn load_project(state: State<'_, AppState>, tab: u32, json: String) -> Res<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().load_project(&json).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn project_json(state: State<'_, AppState>, tab: u32) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().project_json().map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn navigation(state: State<'_, AppState>, tab: u32) -> Res<Response> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        Ok(Response::new(t.lock().navigation()))
    }).await.map_err(e)?
}

#[tauri::command]
async fn tile_thumb(state: State<'_, AppState>, tab: u32, area: usize, tile: usize, max: usize) -> Res<Response> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().tile_thumb(area, tile, max).map(Response::new).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn stitch(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, params: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| {
            let start = Instant::now();
            let clock = || start.elapsed().as_secs_f64() * 1000.0;
            s.stitch(&area_id, &params, ctl, &clock)
        }).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn recompute(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, params: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| {
            let start = Instant::now();
            let clock = || start.elapsed().as_secs_f64() * 1000.0;
            s.recompute(&area_id, &params, ctl, &clock)
        }).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn preview(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, params: String, level: usize) -> Res<Response> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.preview_with_ctl(&area_id, &params, level, ctl)).map(Response::new).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn auto_params(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, params: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.auto_params_with_ctl(&area_id, &params, ctl)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn set_grains(state: State<'_, AppState>, tab: u32, area_id: String, grains: String) -> Res<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(tab, |s| s.set_grains(&area_id, &grains)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
fn default_params() -> String {
    serde_json::to_string(&grainatlas_core::project::AreaParams::default()).unwrap_or_default()
}

#[tauri::command]
fn cancel(state: State<'_, AppState>, tab: u32) {
    let t = state.tab(tab);
    t.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
async fn pyramid_tile(state: State<'_, AppState>, tab: u32, area_id: String, level: usize, col: usize, row: usize) -> Res<Response> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().pyramid_tile(&area_id, level, col, row).map(Response::new).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn memory_report(state: State<'_, AppState>, tab: u32) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        serde_json::to_string(&t.lock().memory_report()).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn histogram(state: State<'_, AppState>, tab: u32, area_id: String) -> Res<Vec<u32>> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(tab, |s| s.histogram(&area_id)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn retone(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, tone: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.retone_with_ctl(&area_id, &tone, ctl)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
fn tone_lut(tone: String) -> Res<Response> {
    session::tone_lut(&tone).map(Response::new).map_err(e)
}

#[tauri::command]
async fn export_png(state: State<'_, AppState>, tab: u32, area_id: String, tone: String) -> Res<Response> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().export_png(&area_id, &tone).map(Response::new).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn area_signal(state: State<'_, AppState>, tab: u32, area_id: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.tab(tab).lock().area_signal(&area_id).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn segment_warning(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, params: String, grains: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.segment_warning_with_ctl(&area_id, &params, &grains, ctl)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn segment(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, params: String, previous: String, outline: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.segment_with_ctl(&area_id, &params, &previous, &outline, ctl)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn segment_missing(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, params: String, existing: String, outline: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.segment_missing_with_ctl(&area_id, &params, &existing, &outline, ctl)).map_err(e)
    }).await.map_err(e)?
}

#[allow(clippy::too_many_arguments)] // AppHandle is injected; keep the existing IPC arguments.
#[tauri::command]
async fn grain_at(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, x: f64, y: f64, params: String, outline: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.grain_at_with_ctl(&area_id, x, y, &params, &outline, ctl)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn split_grain(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, grain: String, line: [f64; 4]) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.split_grain_with_ctl(&area_id, &grain, line, ctl)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn merge_grains(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, grains: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.merge_grains_with_ctl(&area_id, &grains, ctl)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn snap_outline(state: State<'_, AppState>, tab: u32, area_id: String, path: String, width_um: f64, contrast: f64) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(tab, |s| s.snap_outline(&area_id, &path, width_um, contrast)).map_err(e)
    }).await.map_err(e)?
}

#[allow(clippy::too_many_arguments)] // AppHandle is injected; keep the existing IPC arguments.
#[tauri::command]
async fn outline_grain(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, polygon: String, base: String, op: String, outline: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.outline_grain_with_ctl(&area_id, &polygon, &base, &op, &outline, ctl)).map_err(e)
    }).await.map_err(e)?
}

#[allow(clippy::too_many_arguments)] // AppHandle is injected; keep the existing IPC arguments.
#[tauri::command]
async fn brush_grain(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, stroke: String, size_um: f64, base: String, op: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.brush_grain_with_ctl(&area_id, &stroke, size_um, &base, &op, ctl)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn refine_grain(app: AppHandle, state: State<'_, AppState>, tab: u32, area_id: String, grain: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session_ctl(&app, tab, |s, ctl| s.refine_grain_with_ctl(&area_id, &grain, ctl)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
fn renumber(grains: String, tolerance: f64, from: String) -> Res<String> {
    session::renumber(&grains, tolerance, &from).map_err(e)
}

#[tauri::command]
async fn export_grains(state: State<'_, AppState>, tab: u32, area_id: String, grains: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().export_grains(&area_id, &grains).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn check_project(state: State<'_, AppState>, tab: u32, project: String, labels: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().check_project(&project, &labels).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn export_spots(state: State<'_, AppState>, tab: u32, spots: String, grains: String, format: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().export_spots(&spots, &grains, &format).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn set_link(state: State<'_, AppState>, tab: u32, mount_id: String, samples: String) -> Res<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().set_link(&mount_id, &samples).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn layout(state: State<'_, AppState>, tab: u32, params: String, grains: String, spots: String, labels: String, tone: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(tab, |s| s.layout(&params, &grains, &spots, &labels, &tone)).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
fn register(pairs: String) -> Res<String> {
    session::register(&pairs).map_err(e)
}

#[tauri::command]
async fn pdf(pages: String) -> Res<Response> {
    tauri::async_runtime::spawn_blocking(move || {
        session::pdf_from_svgs(&pages).map(Response::new).map_err(e)
    }).await.map_err(e)?
}

/// Saves bytes (raw request body) under a name chosen in a native save dialog.
/// Header `x-name` carries the suggested file name, `x-tab` the tab (its package folder is where
/// the dialog starts). Returns the path, or null if cancelled.
#[tauri::command]
async fn save_bytes(app: AppHandle, state: State<'_, AppState>, request: Request<'_>) -> Res<Option<String>> {
    let state = state.inner().clone();
    let tab = request.headers().get("x-tab").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
    let t = state.tab(tab);
    let raw = request.headers().get("x-name").and_then(|v| v.to_str().ok()).unwrap_or("grainatlas-export");
    let name = percent_decode(raw);
    let bytes = match request.body() {
        InvokeBody::Raw(b) => b.clone(),
        InvokeBody::Json(v) => v.as_str().map(|s| s.as_bytes().to_vec()).unwrap_or_default(),
    };

    tauri::async_runtime::spawn_blocking(move || {
        let mut d = app.dialog().file().set_file_name(&name);
        if let Some(dir) = t.package_dir() {
            d = d.set_directory(dir);
        }
        let Some(path) = d.blocking_save_file() else { return Ok(None) };
        let path = path.into_path().map_err(e)?;
        std::fs::write(&path, bytes).map_err(e)?;
        Ok(Some(path.display().to_string()))
    }).await.map_err(e)?
}

/// Writes the project folder `<name>.grainatlas/` (plan section 5.4) next to the package, or in a
/// folder chosen in a dialog. `project` is the full project JSON (with the UI's grains and spots).
#[tauri::command]
async fn save_project(app: AppHandle, state: State<'_, AppState>, tab: u32, project: String, ask: bool, dir: Option<String>) -> Res<Option<String>> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        // A project opened from its folder is saved back into that folder, wherever it is.
        if let Some(d) = dir.filter(|d| !ask && !d.is_empty()) {
            let dir = PathBuf::from(d);
            let files = state.with_session_ctl(&app, tab, |s, ctl| s.project_files_with_ctl(&project, ctl)).map_err(e)?;
            for (rel, bytes) in files {
                let p = dir.join(rel);
                if let Some(q) = p.parent() {
                    std::fs::create_dir_all(q).map_err(e)?;
                }
                std::fs::write(&p, bytes).map_err(e)?;
            }
            t.lock().project_path = dir.canonicalize().map_err(e)?.display().to_string();
            return Ok(Some(dir.display().to_string()));
        }
        let v: serde_json::Value = serde_json::from_str(&project).map_err(e)?;
        let name = v["name"].as_str().unwrap_or("project").to_string();
        let pkg_parent = t.package_dir();
        let parent = match (ask, pkg_parent) {
            (false, Some(p)) => p,
            (_, start) => {
                let mut d = app.dialog().file().set_title("Folder for the .grainatlas project");
                if let Some(s) = start {
                    d = d.set_directory(s);
                }
                match d.blocking_pick_folder() {
                    Some(p) => p.into_path().map_err(e)?,
                    None => return Ok(None),
                }
            }
        };
        let dir = parent.join(format!("{name}.grainatlas"));
        let files = state.with_session_ctl(&app, tab, |s, ctl| s.project_files_with_ctl(&project, ctl)).map_err(e)?;
        for (rel, bytes) in files {
            let p = dir.join(rel);
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).map_err(e)?;
            }
            std::fs::write(&p, bytes).map_err(e)?;
        }
        t.lock().project_path = dir.canonicalize().map_err(e)?.display().to_string();
        Ok(Some(dir.display().to_string()))
    }).await.map_err(e)?
}

/// Writes the project file `<name>.gaproj`: the project record alone (parameters, placements,
/// photometric results, grains, spots, labels, the laicpmsdb link), no pixels. It reopens with
/// the package's own images by their recorded path. `path` = where it was opened from or saved
/// before; otherwise next to the package, or in a save dialog when `ask`.
#[tauri::command]
async fn save_project_file(app: AppHandle, state: State<'_, AppState>, tab: u32, project: String, path: Option<String>, ask: bool) -> Res<Option<String>> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        let v: serde_json::Value = serde_json::from_str(&project).map_err(e)?;
        let name = format!("{}.gaproj", v["name"].as_str().filter(|s| !s.is_empty()).unwrap_or("project"));
        let target = match (path, ask, t.package_dir()) {
            (Some(p), false, _) => PathBuf::from(p),
            (_, false, Some(dir)) => dir.join(&name),
            (prev, _, dir) => {
                let mut d = app.dialog().file().set_file_name(&name).add_filter("grainatlas project", &["gaproj"]);
                if let Some(start) = prev.as_ref().and_then(|p| PathBuf::from(p).parent().map(|q| q.to_path_buf())).or(dir) {
                    d = d.set_directory(start);
                }
                match d.blocking_save_file() {
                    Some(p) => p.into_path().map_err(e)?,
                    None => return Ok(None),
                }
            }
        };
        let pretty = serde_json::to_vec_pretty(&v).map_err(e)?;
        // Write next to the target and rename, so a crash never leaves half a project.
        let tmp = target.with_extension("gaproj.tmp");
        std::fs::write(&tmp, pretty).map_err(e)?;
        std::fs::rename(&tmp, &target).map_err(e)?;
        t.lock().project_path = target.canonicalize().map_err(e)?.display().to_string();
        Ok(Some(target.display().to_string()))
    }).await.map_err(e)?
}

/// Projects the OS asked to open (Finder double-click, Open With, dropped on the Dock icon),
/// waiting for the page to take them; it is told by `grainatlas://opened`.
#[tauri::command]
fn take_opened(state: State<'_, AppState>) -> Vec<String> {
    std::mem::take(&mut *state.opened.lock().unwrap_or_else(|p| p.into_inner()))
}

/// The atlas bundle as zip bytes (`<project_id>.atlas/…`), for publishing to laicpmsdb.
#[tauri::command]
async fn atlas_zip(state: State<'_, AppState>, tab: u32, project: String, options: String) -> Res<Response> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        Ok(Response::new(t.lock().atlas_zip(&project, &options).map_err(e)?))
    }).await.map_err(e)?
}

/// Saves the atlas bundle: a `<project_id>.atlas/` folder in a chosen folder, or a zip through the
/// save dialog. Returns the path written, or null when cancelled.
#[tauri::command]
async fn save_atlas(app: AppHandle, state: State<'_, AppState>, tab: u32, project: String, options: String, zip: bool) -> Res<Option<String>> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        let id = serde_json::from_str::<serde_json::Value>(&project).map_err(e)?["uuid"].as_str().unwrap_or("project").to_string();
        let start = t.package_dir();
        if zip {
            let bytes = t.lock().atlas_zip(&project, &options).map_err(e)?;
            let mut d = app.dialog().file().set_file_name(format!("{id}.atlas.zip"));
            if let Some(s) = start {
                d = d.set_directory(s);
            }
            let Some(path) = d.blocking_save_file() else { return Ok(None) };
            let path = path.into_path().map_err(e)?;
            std::fs::write(&path, bytes).map_err(e)?;
            return Ok(Some(path.display().to_string()));
        }
        let files = t.lock().atlas_files(&project, &options).map_err(e)?;
        let mut d = app.dialog().file().set_title("Folder for the .atlas bundle");
        if let Some(s) = start {
            d = d.set_directory(s);
        }
        let Some(parent) = d.blocking_pick_folder() else { return Ok(None) };
        let dir = parent.into_path().map_err(e)?.join(format!("{id}.atlas"));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(e)?;
        }
        for (rel, bytes) in files {
            let p = dir.join(rel);
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).map_err(e)?;
            }
            std::fs::write(&p, bytes).map_err(e)?;
        }
        Ok(Some(dir.display().to_string()))
    }).await.map_err(e)?
}

/// Settings → Reset layout: the window back to its default size, centred.
#[tauri::command]
fn reset_window(window: tauri::WebviewWindow) {
    geometry::reset(&window);
}

#[tauri::command]
fn laser_fit(fiducials: String, units: String) -> Res<String> {
    session::laser_fit(&fiducials, &units).map_err(e)
}

#[tauri::command]
fn laser_adapters() -> String {
    session::laser_adapters()
}

#[tauri::command]
async fn set_laser(state: State<'_, AppState>, tab: u32, area_id: String, registration: String) -> Res<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().set_laser(&area_id, &registration).map_err(e)
    }).await.map_err(e)?
}

#[tauri::command]
async fn laser_export(state: State<'_, AppState>, tab: u32, spots: String, area_id: String, adapter: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().laser_export(&spots, &area_id, &adapter).map_err(e)
    }).await.map_err(e)?
}

/// Which area owns each detected grain (cross-area duplicates).
#[tauri::command]
async fn ownership(state: State<'_, AppState>, tab: u32, grains: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().ownership(&grains, "").map_err(e)
    }).await.map_err(e)?
}

/// Print sheets of the mount (pages, print_sheet registration, stats, ownership).
#[tauri::command]
async fn print_sheets(state: State<'_, AppState>, tab: u32, params: String, grains: String, tone: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(tab, |s| s.print_sheets(&params, &grains, &tone)).map_err(e)
    }).await.map_err(e)?
}

/// Whole area on one sheet (pages, print_sheet registration and stats).
#[tauri::command]
async fn print_whole(state: State<'_, AppState>, tab: u32, params: String, tone: String) -> Res<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.with_session(tab, |s| s.print_whole(&params, &tone)).map_err(e)
    }).await.map_err(e)?
}

/// Drops the open package and project (a project opened from a server becomes a local copy).
#[tauri::command]
async fn blank(state: State<'_, AppState>, tab: u32, name: String) -> Res<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        {
            let mut s = t.lock();
            *s = Session::new(&name);
            t.evictable.store(0, Ordering::Relaxed);
        }
        *t.package_path.lock().unwrap_or_else(|x| x.into_inner()) = None;

        Ok(())
    }).await.map_err(e)?
}

/// A tab closed: its session (package, mosaics) is dropped; a step still running is cancelled.
#[tauri::command]
fn close_tab(state: State<'_, AppState>, tab: u32) {
    if let Some(t) = state.tabs.lock().unwrap_or_else(|p| p.into_inner()).remove(&tab) {
        t.cancel.store(true, Ordering::SeqCst);
    }
}

#[tauri::command]
async fn record_published(state: State<'_, AppState>, tab: u32, published: String) -> Res<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = state.tab(tab);
        t.lock().record_published(&published).map_err(e)
    }).await.map_err(e)?
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Serialize)]
struct Startup {
    /// A folder (package, area folder, `.grainatlas` project) or image files.
    paths: Vec<String>,
    stitch: bool,
    segment: bool,
    /// Scale for loose images, so they open without the dialog.
    um_per_px: Option<f64>,
    field_um: Option<f64>,
    profile: Option<String>,
}

/// Command line: `grainatlas [folder | images…] [--um-per-px X | --field-um W] [--stitch]
/// [--segment]` opens a package or area folder, or loose images with the given scale, and on
/// request stitches every area and finds grains (batch preparation before a session). Several
/// folders (packages, `.grainatlas` projects) open one tab each; files open together in one.
#[tauri::command]
fn startup() -> Startup {
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("-psn")).collect();
    let profile = launch_profile(&args);
    let args = profile::without_profile(&args);
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0);
    let paths: Vec<String> = args.iter().filter(|a| !a.starts_with("--") && std::path::Path::new(a).exists()).cloned().collect();
    let segment = args.iter().any(|a| a == "--segment");
    Startup { paths, stitch: segment || args.iter().any(|a| a == "--stitch"), segment, um_per_px: value("--um-per-px"), field_um: value("--field-um"), profile }
}

fn launch_profile(args: &[String]) -> Option<String> {
    profile::name(args, std::env::var("GRAINATLAS_PROFILE").ok().as_deref(), cfg!(debug_assertions))
}

/// The menu bar: the platform's usual menus, with File → New Tab (Cmd+T) and Close Tab (Cmd+W)
/// sent to the page as `grainatlas://menu` (`new-tab`, `close-tab`); Close Window is Shift+Cmd+W.
fn menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{AboutMetadata, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
    let new_tab = MenuItemBuilder::with_id("new-tab", "New Tab").accelerator("CmdOrCtrl+T").build(app)?;
    let close_tab = MenuItemBuilder::with_id("close-tab", "Close Tab").accelerator("CmdOrCtrl+W").build(app)?;
    let close_window = MenuItemBuilder::with_id("close-window", "Close Window").accelerator("CmdOrCtrl+Shift+W").build(app)?;
    let file = SubmenuBuilder::new(app, "File").item(&new_tab).item(&close_tab).separator().item(&close_window).build()?;
    let edit = SubmenuBuilder::new(app, "Edit").undo().redo().separator().cut().copy().paste().select_all().build()?;
    let window = SubmenuBuilder::new(app, "Window").minimize().maximize().separator().fullscreen().build()?;
    let mut m = MenuBuilder::new(app);
    #[cfg(target_os = "macos")]
    {
        let about = PredefinedMenuItem::about(app, None, Some(AboutMetadata::default()))?;
        let appm = SubmenuBuilder::new(app, "grainatlas").item(&about).separator().services().separator().hide().hide_others().show_all().separator().quit().build()?;
        m = m.item(&appm);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (AboutMetadata::default(), PredefinedMenuItem::separator(app)?);
    m.item(&file).item(&edit).item(&window).build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .menu(menu)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "new-tab" | "close-tab" => {
                let _ = app.emit("grainatlas://menu", ev.id().as_ref());
            }
            "close-window" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.close();
                }
            }
            _ => {}
        })
        .setup(|app| {
            let config = app.config().app.windows.iter().find(|w| w.label == "main")
                .ok_or("main window configuration is missing")?;
            let mut builder = tauri::WebviewWindowBuilder::from_config(app, config)?;
            if let Some(name) = launch_profile(&std::env::args().skip(1).collect::<Vec<_>>()) {
                let directory = app.path().app_local_data_dir()?.join("profiles").join(profile::directory_name(&name));
                std::fs::create_dir_all(&directory)?;
                builder = builder.data_store_identifier(profile::identifier(&name))
                    .data_directory(directory).title(format!("{} ({name})", config.title));
            }
            let w = builder.build()?;
            geometry::restore(&w);
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::CloseRequested { .. }) {
                geometry::save(window);
            }
        })
        .manage(AppState::default())
        .manage(updater::UpdateState::default())
        .invoke_handler(tauri::generate_handler![
            updater::update_check,
            updater::update_download,
            updater::update_install,
            updater::update_status,
            updater::update_supported,
            startup,
            recent::path_status,
            recent::project_counts,
            open_path,
            open_paths,
            open_loose,
            open_project,
            load_project,
            project_json,
            navigation,
            tile_thumb,
            stitch,
            recompute,
            preview,
            auto_params,
            set_grains,
            default_params,
            cancel,
            pyramid_tile,
            histogram,
            memory_report,
            retone,
            tone_lut,
            export_png,
            area_signal,
            segment_warning,
            segment,
            segment_missing,
            grain_at,
            split_grain,
            merge_grains,
            refine_grain,
            snap_outline,
            outline_grain,
            brush_grain,
            renumber,
            export_grains,
            export_spots,
            check_project,
            set_link,
            layout,
            pdf,
            register,
            save_bytes,
            save_project,
            atlas_zip,
            save_atlas,
            record_published,
            blank,
            close_tab,
            save_project_file,
            take_opened,
            laser_fit,
            laser_adapters,
            set_laser,
            laser_export,
            reset_window,
            ownership,
            print_sheets,
            print_whole,
        ])
        .build(tauri::generate_context!())
        .expect("error while running grainatlas")
        .run(|app, event| {
            // macOS hands documents to the running app (or the one it launches for them).
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let tauri::RunEvent::Opened { urls } = event {
                let paths: Vec<String> = urls.iter().filter_map(|u| u.to_file_path().ok()).map(|p| p.display().to_string()).collect();
                if !paths.is_empty() {
                    app.state::<AppState>().opened.lock().unwrap_or_else(|p| p.into_inner()).extend(paths);
                    let _ = app.emit("grainatlas://opened", ());
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.set_focus();
                    }
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            let _ = (app, event);
        });
}
