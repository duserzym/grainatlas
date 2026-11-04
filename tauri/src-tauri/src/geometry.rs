//! The main window's size and position, remembered between runs (docs/ux-conventions.md rule 7)
//! in `<app config dir>/window.json`, in logical pixels. A saved place is used only when it still
//! lies on a connected monitor, and the size is clamped to that monitor's work area, so a
//! window left on a large external monitor reopens whole on the laptop screen.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, WebviewWindow, Window};

/// Default size: fits a 13- and 14-inch laptop screen (1280 x 800 logical and up).
pub const DEFAULT: (f64, f64) = (1280.0, 800.0);

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
struct Geometry {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    maximized: bool,
}

fn file<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("window.json"))
}

/// Puts the window where it was, if that place is still on a monitor.
pub fn restore<R: Runtime>(w: &WebviewWindow<R>) {
    let Some(path) = file(w.app_handle()) else { return };
    let Ok(bytes) = std::fs::read(&path) else { return };
    let Ok(g) = serde_json::from_slice::<Geometry>(&bytes) else { return };
    let monitors = w.available_monitors().unwrap_or_default();
    // The title bar (top 40 px, 100 px in from the left) must land inside a work area.
    let area = monitors.iter().find_map(|m| {
        let s = m.scale_factor();
        let a = m.work_area();
        let (ax, ay, aw, ah) = (a.position.x as f64 / s, a.position.y as f64 / s, a.size.width as f64 / s, a.size.height as f64 / s);
        (g.x + 100.0 >= ax && g.x + 100.0 <= ax + aw && g.y + 1.0 >= ay && g.y + 40.0 <= ay + ah).then_some((ax, ay, aw, ah))
    });
    let Some((ax, ay, aw, ah)) = area else { return };
    let (width, height) = (g.w.clamp(640.0, aw), g.h.clamp(480.0, ah));
    let _ = w.set_size(LogicalSize::new(width, height));
    let _ = w.set_position(LogicalPosition::new(g.x.clamp(ax, (ax + aw - width).max(ax)), g.y.clamp(ay, (ay + ah - height).max(ay))));
    if g.maximized {
        let _ = w.maximize();
    }
}

/// Records the window's place (not while minimised; a maximised window keeps its normal size).
pub fn save<R: Runtime>(w: &Window<R>) {
    if w.is_minimized().unwrap_or(false) {
        return;
    }
    let Some(path) = file(w.app_handle()) else { return };
    let s = w.scale_factor().unwrap_or(1.0);
    let maximized = w.is_maximized().unwrap_or(false);
    let prev: Option<Geometry> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let g = match (maximized, prev) {
        (true, Some(p)) => Geometry { maximized: true, ..p },
        _ => {
            let (Ok(pos), Ok(size)) = (w.outer_position(), w.inner_size()) else { return };
            Geometry { x: pos.x as f64 / s, y: pos.y as f64 / s, w: size.width as f64 / s, h: size.height as f64 / s, maximized }
        }
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, serde_json::to_vec(&g).unwrap_or_default());
}

/// Forgets the saved place: default size, centred (Settings → Reset layout).
pub fn reset<R: Runtime>(w: &WebviewWindow<R>) {
    if let Some(path) = file(w.app_handle()) {
        let _ = std::fs::remove_file(path);
    }
    let _ = w.unmaximize();
    let _ = w.set_size(LogicalSize::new(DEFAULT.0, DEFAULT.1));
    let _ = w.center();
}
