//! Checks behind the Open screen's Recent projects list. Packages often live on a
//! Google Drive (File Provider) volume, where a stat can take seconds or hang while
//! offline, so every disk touch here runs on its own thread with a deadline and the
//! answer is "unreachable" when the deadline passes.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use serde::Serialize;

/// How long a recent-list check waits for the disk.
const DEADLINE: Duration = Duration::from_secs(4);

/// Runs `f` on a new thread and waits at most `deadline`; `None` on timeout (the
/// thread is left to finish on its own and its answer is dropped).
fn with_deadline<T: Send + 'static>(deadline: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(deadline).ok()
}

fn status_of(path: &Path) -> &'static str {
    match std::fs::metadata(path) {
        Ok(_) => "present",
        Err(x) if x.kind() == std::io::ErrorKind::NotFound => "missing",
        Err(_) => "unreachable",
    }
}

/// `present`, `missing` (nothing at that path) or `unreachable` (no answer within
/// the deadline, or the volume refused: permissions, I/O error).
pub fn path_status_within(path: PathBuf, deadline: Duration) -> &'static str {
    with_deadline(deadline, move || status_of(&path)).unwrap_or("unreachable")
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Counts {
    pub areas: usize,
    pub spots: usize,
}

fn counts_of(path: &Path) -> Result<Counts, String> {
    let file = if path.is_dir() { path.join("project.json") } else { path.to_path_buf() };
    let text = std::fs::read_to_string(&file).map_err(|x| format!("{}: {x}", file.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|x| x.to_string())?;
    let areas = v["areas"].as_array().ok_or("project.json has no areas")?.len();
    let spots = v["grainatlas_ui"]["spots"].as_array().map_or(0, |s| s.len());
    Ok(Counts { areas, spots })
}

pub fn project_counts_within(path: PathBuf, deadline: Duration) -> Result<Counts, String> {
    let shown = path.display().to_string();
    with_deadline(deadline, move || counts_of(&path)).unwrap_or_else(|| Err(format!("{shown} did not answer")))
}

/// Whether a recent project's folder or file is still where the list says.
#[tauri::command]
pub async fn path_status(path: String) -> Result<String, String> {
    Ok(path_status_within(PathBuf::from(path), DEADLINE).to_string())
}

/// Areas and spots of a saved project (`.grainatlas/` folder or `.gaproj` file),
/// so the recent list shows the file's numbers rather than the ones cached when it
/// was last opened.
#[tauri::command]
pub async fn project_counts(path: String) -> Result<Counts, String> {
    project_counts_within(PathBuf::from(path), DEADLINE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grainatlas-recent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn present_and_missing() {
        let d = scratch("status");
        assert_eq!(path_status_within(d.clone(), DEADLINE), "present");
        assert_eq!(path_status_within(d.join("gone"), DEADLINE), "missing");
    }

    #[test]
    fn a_slow_disk_is_unreachable_not_a_hang() {
        let start = std::time::Instant::now();
        let r = with_deadline(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_secs(2));
            "present"
        });
        assert_eq!(r.unwrap_or("unreachable"), "unreachable");
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn counts_from_folder_and_file() {
        let d = scratch("counts");
        let json = r#"{"areas":[{"id":"a"},{"id":"b"}],"grainatlas_ui":{"spots":[{"id":"s"}]}}"#;
        std::fs::create_dir_all(d.join("x.grainatlas")).unwrap();
        std::fs::write(d.join("x.grainatlas/project.json"), json).unwrap();
        std::fs::write(d.join("y.gaproj"), r#"{"areas":[]}"#).unwrap();
        assert_eq!(project_counts_within(d.join("x.grainatlas"), DEADLINE).unwrap(), Counts { areas: 2, spots: 1 });
        assert_eq!(project_counts_within(d.join("y.gaproj"), DEADLINE).unwrap(), Counts { areas: 0, spots: 0 });
        assert!(project_counts_within(d.join("none.gaproj"), DEADLINE).is_err());
    }
}
