//! grainatlas desktop: the same Svelte UI, with the core running natively
//! (rayon, all cores) behind Tauri commands that mirror the wasm worker API.
//! Packages are read from disk by path; results are saved with native dialogs.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use grainatlas_core::Ctl;
use grainatlas_core::project::session::{self, Session};
use grainatlas_core::source::{DirSource, FileListSource, Source};
use serde::Serialize;
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    session: Mutex<Session>,
    cancel: Arc<AtomicBool>,
    package_path: Mutex<Option<PathBuf>>,
}

type Res<T> = Result<T, String>;

fn e(x: impl std::fmt::Display) -> String {
    x.to_string()
}

fn lock<'a>(s: &'a State<'_, AppState>) -> std::sync::MutexGuard<'a, Session> {
    s.session.lock().unwrap_or_else(|p| p.into_inner())
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
    stage: String,
    fraction: f32,
}

/// Opens one folder (a package or an area folder) or a set of files (loose images, or the
/// files of an area). Returns the core's probe JSON: opened, or needs a scale.
#[tauri::command]
async fn open_paths(state: State<'_, AppState>, paths: Vec<String>) -> Res<String> {
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
        let mut s = lock(&state);
        s.origin_files = files;
        s.open_source(src, &path.display().to_string(), &now_iso()).map_err(e)?
    };
    *state.package_path.lock().unwrap_or_else(|x| x.into_inner()) = Some(path);
    Ok(out)
}

#[tauri::command]
async fn open_path(state: State<'_, AppState>, path: String) -> Res<String> {
    open_paths(state, vec![path]).await
}

/// Loose images of the last open, with the scale the user gave (LooseSpec JSON).
#[tauri::command]
async fn open_loose(state: State<'_, AppState>, spec: String) -> Res<String> {
    lock(&state).open_loose(&spec, &now_iso()).map_err(e)
}

/// Opens a `<name>.grainatlas/` project folder: its package (by the recorded path) and its
/// project.json. Returns `{ "info": PackageInfo, "project": full project JSON }`.
#[tauri::command]
async fn open_project(state: State<'_, AppState>, path: String) -> Res<String> {
    let pj = std::fs::read_to_string(PathBuf::from(&path).join("project.json")).map_err(|x| format!("{path}: {x}"))?;
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
        let mut s = lock(&state);
        s.load_project(&pj).map_err(e)?;
        s.origin_files = if is_loose { loose_files.iter().map(|p| p.display().to_string()).collect() } else { Vec::new() };
        let probe = s.open_source(src, &pkg, &now_iso()).map_err(e)?;
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
    *state.package_path.lock().unwrap_or_else(|x| x.into_inner()) = Some(PathBuf::from(&pkg));
    Ok(serde_json::json!({ "info": serde_json::from_str::<serde_json::Value>(&info).map_err(e)?, "project": v }).to_string())
}

#[tauri::command]
fn load_project(state: State<'_, AppState>, json: String) -> Res<()> {
    lock(&state).load_project(&json).map_err(e)
}

#[tauri::command]
fn project_json(state: State<'_, AppState>) -> Res<String> {
    lock(&state).project_json().map_err(e)
}

#[tauri::command]
fn navigation(state: State<'_, AppState>) -> Response {
    Response::new(lock(&state).navigation())
}

#[tauri::command]
async fn tile_thumb(state: State<'_, AppState>, area: usize, tile: usize, max: usize) -> Res<Response> {
    lock(&state).tile_thumb(area, tile, max).map(Response::new).map_err(e)
}

#[tauri::command]
async fn stitch(app: AppHandle, state: State<'_, AppState>, area_id: String, params: String) -> Res<String> {
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();
    let start = Instant::now();
    let prog = |stage: &str, fraction: f32| {
        let _ = app.emit("grainatlas://progress", Progress { stage: stage.to_string(), fraction });
    };
    let canc = || cancel.load(Ordering::SeqCst);
    let clock = || start.elapsed().as_secs_f64() * 1000.0;
    let ctl = Ctl { cancel: &canc, progress: &prog };
    lock(&state).stitch(&area_id, &params, &ctl, &clock).map_err(e)
}

#[tauri::command]
async fn recompute(app: AppHandle, state: State<'_, AppState>, area_id: String, params: String) -> Res<String> {
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();
    let start = Instant::now();
    let prog = |stage: &str, fraction: f32| {
        let _ = app.emit("grainatlas://progress", Progress { stage: stage.to_string(), fraction });
    };
    let canc = || cancel.load(Ordering::SeqCst);
    let clock = || start.elapsed().as_secs_f64() * 1000.0;
    let ctl = Ctl { cancel: &canc, progress: &prog };
    lock(&state).recompute(&area_id, &params, &ctl, &clock).map_err(e)
}

#[tauri::command]
async fn preview(state: State<'_, AppState>, area_id: String, params: String, level: usize) -> Res<Response> {
    lock(&state).preview(&area_id, &params, level).map(Response::new).map_err(e)
}

#[tauri::command]
fn auto_params(state: State<'_, AppState>, area_id: String, params: String) -> Res<String> {
    lock(&state).auto_params(&area_id, &params).map_err(e)
}

#[tauri::command]
fn set_grains(state: State<'_, AppState>, area_id: String, grains: String) -> Res<()> {
    lock(&state).set_grains(&area_id, &grains).map_err(e)
}

#[tauri::command]
fn default_params() -> String {
    serde_json::to_string(&grainatlas_core::project::AreaParams::default()).unwrap_or_default()
}

#[tauri::command]
fn cancel(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
async fn pyramid_tile(state: State<'_, AppState>, area_id: String, level: usize, col: usize, row: usize) -> Res<Response> {
    lock(&state).pyramid_tile(&area_id, level, col, row).map(Response::new).map_err(e)
}

#[tauri::command]
fn histogram(state: State<'_, AppState>, area_id: String) -> Res<Vec<u32>> {
    lock(&state).histogram(&area_id).map_err(e)
}

#[tauri::command]
async fn retone(state: State<'_, AppState>, area_id: String, tone: String) -> Res<String> {
    lock(&state).retone(&area_id, &tone).map_err(e)
}

#[tauri::command]
fn tone_lut(tone: String) -> Res<Response> {
    session::tone_lut(&tone).map(Response::new).map_err(e)
}

#[tauri::command]
async fn export_png(state: State<'_, AppState>, area_id: String, tone: String) -> Res<Response> {
    lock(&state).export_png(&area_id, &tone).map(Response::new).map_err(e)
}

#[tauri::command]
async fn segment(state: State<'_, AppState>, area_id: String, params: String, previous: String) -> Res<String> {
    lock(&state).segment(&area_id, &params, &previous).map_err(e)
}

#[tauri::command]
async fn grain_at(state: State<'_, AppState>, area_id: String, x: f64, y: f64, params: String) -> Res<String> {
    lock(&state).grain_at(&area_id, x, y, &params).map_err(e)
}

#[tauri::command]
async fn split_grain(state: State<'_, AppState>, area_id: String, grain: String, line: [f64; 4]) -> Res<String> {
    lock(&state).split_grain(&area_id, &grain, line).map_err(e)
}

#[tauri::command]
async fn merge_grains(state: State<'_, AppState>, area_id: String, grains: String) -> Res<String> {
    lock(&state).merge_grains(&area_id, &grains).map_err(e)
}

#[tauri::command]
async fn refine_grain(state: State<'_, AppState>, area_id: String, grain: String) -> Res<String> {
    lock(&state).refine_grain(&area_id, &grain).map_err(e)
}

#[tauri::command]
fn renumber(grains: String, tolerance: f64, from: String) -> Res<String> {
    session::renumber(&grains, tolerance, &from).map_err(e)
}

#[tauri::command]
fn export_grains(state: State<'_, AppState>, area_id: String, grains: String) -> Res<String> {
    lock(&state).export_grains(&area_id, &grains).map_err(e)
}

#[tauri::command]
fn export_spots(state: State<'_, AppState>, spots: String, grains: String, format: String) -> Res<String> {
    lock(&state).export_spots(&spots, &grains, &format).map_err(e)
}

#[tauri::command]
fn set_link(state: State<'_, AppState>, mount_id: String, samples: String) -> Res<()> {
    lock(&state).set_link(&mount_id, &samples).map_err(e)
}

#[tauri::command]
async fn layout(state: State<'_, AppState>, params: String, grains: String, spots: String, labels: String, tone: String) -> Res<String> {
    lock(&state).layout(&params, &grains, &spots, &labels, &tone).map_err(e)
}

#[tauri::command]
fn register(pairs: String) -> Res<String> {
    session::register(&pairs).map_err(e)
}

#[tauri::command]
async fn pdf(pages: String) -> Res<Response> {
    session::pdf_from_svgs(&pages).map(Response::new).map_err(e)
}

/// Saves bytes (raw request body) under a name chosen in a native save dialog.
/// Header `x-name` carries the suggested file name. Returns the path, or null if cancelled.
#[tauri::command]
async fn save_bytes(app: AppHandle, state: State<'_, AppState>, request: Request<'_>) -> Res<Option<String>> {
    let raw = request.headers().get("x-name").and_then(|v| v.to_str().ok()).unwrap_or("grainatlas-export");
    let name = percent_decode(raw);
    let bytes = match request.body() {
        InvokeBody::Raw(b) => b.clone(),
        InvokeBody::Json(v) => v.as_str().map(|s| s.as_bytes().to_vec()).unwrap_or_default(),
    };
    let mut d = app.dialog().file().set_file_name(&name);
    if let Some(dir) = state.package_path.lock().unwrap_or_else(|x| x.into_inner()).as_ref().and_then(|p| p.parent().map(|q| q.to_path_buf())) {
        d = d.set_directory(dir);
    }
    let Some(path) = d.blocking_save_file() else { return Ok(None) };
    let path = path.into_path().map_err(e)?;
    std::fs::write(&path, bytes).map_err(e)?;
    Ok(Some(path.display().to_string()))
}

/// Writes the project folder `<name>.grainatlas/` (plan section 5.4) next to the package, or in a
/// folder chosen in a dialog. `project` is the full project JSON (with the UI's grains and spots).
#[tauri::command]
async fn save_project(app: AppHandle, state: State<'_, AppState>, project: String, ask: bool) -> Res<Option<String>> {
    let v: serde_json::Value = serde_json::from_str(&project).map_err(e)?;
    let name = v["name"].as_str().unwrap_or("project").to_string();
    let pkg_parent = state.package_path.lock().unwrap_or_else(|x| x.into_inner()).as_ref().and_then(|p| p.parent().map(|q| q.to_path_buf()));
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
    let files = lock(&state).project_files(&project).map_err(e)?;
    for (rel, bytes) in files {
        let p = dir.join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(e)?;
        }
        std::fs::write(&p, bytes).map_err(e)?;
    }
    Ok(Some(dir.display().to_string()))
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
    path: Option<String>,
    stitch: bool,
    segment: bool,
}

/// Command line: `grainatlas [package-folder] [--stitch] [--segment]` opens the folder and, on
/// request, stitches every area and finds grains (batch preparation before a session).
#[tauri::command]
fn startup() -> Startup {
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("-psn")).collect();
    let path = args.iter().find(|a| !a.starts_with("--") && std::path::Path::new(a).is_dir()).cloned();
    let segment = args.iter().any(|a| a == "--segment");
    Startup { path, stitch: segment || args.iter().any(|a| a == "--stitch"), segment }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { session: Mutex::new(Session::new("package")), cancel: Arc::new(AtomicBool::new(false)), package_path: Mutex::new(None) })
        .invoke_handler(tauri::generate_handler![
            startup,
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
            retone,
            tone_lut,
            export_png,
            segment,
            grain_at,
            split_grain,
            merge_grains,
            refine_grain,
            renumber,
            export_grains,
            export_spots,
            set_link,
            layout,
            pdf,
            register,
            save_bytes,
            save_project,
        ])
        .run(tauri::generate_context!())
        .expect("error while running grainatlas");
}
