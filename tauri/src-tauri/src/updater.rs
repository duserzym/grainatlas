//! Signed updates, downloaded in the background and installed on request.
use serde::Serialize;
#[cfg(any(target_os = "windows", test))]
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct UpdateState(Mutex<Pending>);

#[derive(Default)]
struct Pending {
    update: Option<Update>,
    bytes: Option<Vec<u8>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
    pub ready: bool,
}

#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

fn info(update: &Update, ready: bool) -> UpdateInfo {
    UpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        notes: update.body.clone(),
        date: update.date.map(|date| date.to_string()),
        ready,
    }
}

/// Only release-note links enter the system browser through this command.
#[tauri::command]
pub fn update_open_release_notes(url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || !parsed.path().starts_with("/duserzym/grainatlas/releases/tag/v")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
    {
        return Err("Invalid release notes URL".into());
    }
    crate::signin::open_browser(&url)
}

#[tauri::command]
pub async fn update_check(
    app: AppHandle,
    state: State<'_, UpdateState>,
    endpoint: String,
) -> Result<Option<UpdateInfo>, String> {
    // An empty key is valid at startup, but cannot verify a download.
    let config = app.config().plugins.0.get("updater");
    if config
        .and_then(|c| c.get("pubkey"))
        .and_then(|k| k.as_str())
        .is_none_or(|k| k.trim().is_empty())
    {
        return Err("Updater public key is not configured".into());
    }
    let url = Url::parse(&endpoint).map_err(|e| e.to_string())?;
    let update = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let mut pending = state.0.lock().map_err(|e| e.to_string())?;
    // A manual check still reaches the server; keep verified bytes only for the same release.
    let same = pending
        .update
        .as_ref()
        .zip(update.as_ref())
        .is_some_and(|(a, b)| {
            a.version == b.version && a.signature == b.signature && a.download_url == b.download_url
        });
    if !same {
        pending.bytes = None;
    }
    let out = update.as_ref().map(|u| info(u, pending.bytes.is_some()));
    pending.update = update;
    Ok(out)
}

#[tauri::command]
pub async fn update_download(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<UpdateInfo, String> {
    if !update_supported() {
        return Err("Install grainatlas before downloading updates".into());
    }
    let update = state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .update
        .clone()
        .ok_or("No update available")?;
    let mut downloaded = 0;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = app.emit("update-progress", Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut pending = state.0.lock().map_err(|e| e.to_string())?;
    if pending.update.as_ref().is_none_or(|u| {
        u.version != update.version
            || u.signature != update.signature
            || u.download_url != update.download_url
    }) {
        return Err("Update changed during download; check again".into());
    }
    pending.bytes = Some(bytes);
    Ok(info(&update, true))
}

#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String> {
    if !update_supported() {
        return Err("Install grainatlas before updating".into());
    }
    let pending = state.0.lock().map_err(|e| e.to_string())?;
    let update = pending.update.as_ref().ok_or("No update downloaded")?;
    // Keep the verified bytes on failure so installation can be retried.
    let bytes = pending.bytes.as_ref().ok_or("No update downloaded")?;
    update.install(bytes).map_err(|e| e.to_string())?;
    drop(pending);
    app.restart();
}

#[tauri::command]
pub fn update_status(state: State<'_, UpdateState>) -> Option<UpdateInfo> {
    let pending = state.0.lock().ok()?;
    pending
        .update
        .as_ref()
        .map(|u| info(u, pending.bytes.is_some()))
}

#[tauri::command]
pub fn update_supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(target_os = "windows")]
    {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(installed_copy))
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

#[cfg(any(target_os = "windows", test))]
fn installed_copy(exe_dir: &Path) -> bool {
    // Tauri's NSIS installer.nsi writes "$INSTDIR\uninstall.exe".
    // Its MSI main.wxs creates "Uninstall grainatlas.lnk" in INSTALLDIR.
    // Check files beside the running exe, never just a conventional install path.
    exe_dir.join("uninstall.exe").is_file()
        || exe_dir.join("Uninstall grainatlas.lnk").is_file()
}

#[cfg(test)]
mod tests {
    use super::installed_copy;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "grainatlas-updater-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn release_notes_reject_urls_outside_this_repositorys_release_tags() {
        for url in [
            "https://example.com/duserzym/grainatlas/releases/tag/v0.1.8",
            "http://github.com/duserzym/grainatlas/releases/tag/v0.1.8",
            "https://github.com/duserzym/other/releases/tag/v0.1.8",
            "https://github.com/duserzym/grainatlas/releases/latest",
            "https://user@github.com/duserzym/grainatlas/releases/tag/v0.1.8",
            "https://github.com:8443/duserzym/grainatlas/releases/tag/v0.1.8",
            "file:///tmp/release-notes",
        ] {
            assert!(super::update_open_release_notes(url.into()).is_err());
        }
    }

    #[test]
    fn portable_copy_and_missing_directory_are_unsupported() {
        let dir = Scratch::new();
        assert!(!installed_copy(&dir.0));
        fs::write(dir.0.join("grainatlas.exe"), b"portable").unwrap();
        fs::write(dir.0.join("grainatlas.msi"), b"installer download").unwrap();
        assert!(!installed_copy(&dir.0));
        assert!(!installed_copy(&dir.0.join("missing")));
    }

    #[test]
    fn installed_markers_must_be_files_beside_the_exe() {
        for marker in ["uninstall.exe", "Uninstall grainatlas.lnk"] {
            let dir = Scratch::new();
            fs::create_dir(dir.0.join(marker)).unwrap();
            assert!(!installed_copy(&dir.0));
            fs::remove_dir(dir.0.join(marker)).unwrap();
            fs::write(dir.0.join(marker), b"installer marker").unwrap();
            assert!(installed_copy(&dir.0));
            let portable = dir.0.join("portable");
            fs::create_dir(&portable).unwrap();
            assert!(!installed_copy(&portable));
            fs::remove_file(dir.0.join(marker)).unwrap();
            assert!(!installed_copy(&dir.0));
        }
    }
}
