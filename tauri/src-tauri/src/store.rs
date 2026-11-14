//! Private app-data store. Callers supply handled keys, never filesystem paths.
//! Filenames are SHA-256 encodings: fixed length even for long project paths.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize)]
struct Stored { key: String, contents: String }
#[derive(Serialize)]
pub struct Listing { path: String, keys: Vec<String> }

fn handled(key: &str) -> bool {
    ["grainatlas.project.", "grainatlas.copy.", "grainatlas.draft."].iter().any(|p| key.starts_with(p))
        || ["grainatlas.drafts", "grainatlas.discarded-drafts", "grainatlas.recent"].contains(&key)
}
fn encoded(key: &str) -> io::Result<String> {
    if !handled(key) { return Err(io::Error::other("Invalid storage key")); }
    Ok(format!("{:x}.json", Sha256::digest(key.as_bytes())))
}
fn regular(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() => Err(io::Error::other("Storage entry is not a regular file")),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
/// The normal app keeps its files in `<app data>/store`; a named, test or debug profile keeps
/// them inside that profile's own folder, so a test copy never reads or writes the real store.
fn folder(app: &AppHandle) -> Result<PathBuf, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = match crate::launch_profile(&args) {
        Some(name) => app.path().app_local_data_dir().map_err(|e| e.to_string())?
            .join("profiles").join(crate::profile::directory_name(&name)).join("store"),
        None => app.path().app_data_dir().map_err(|e| e.to_string())?.join("store"),
    };
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let meta = fs::symlink_metadata(&root).map_err(|e| e.to_string())?;
    if !meta.is_dir() || meta.is_symlink() { return Err("Invalid storage folder".into()); }
    Ok(root)
}
/// Persist the directory entry too. Windows cannot open a folder as a file;
/// its rename (MoveFileEx) is already durable enough there.
fn sync_folder(root: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(root)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = root;
    Ok(())
}
fn read(root: &Path, key: &str) -> io::Result<Option<String>> {
    let path = root.join(encoded(key)?);
    regular(&path)?;
    match fs::read(path) {
        Ok(bytes) => {
            let entry: Stored = serde_json::from_slice(&bytes)?;
            if entry.key != key { return Err(io::Error::other("Storage key differs")); }
            Ok(Some(entry.contents))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
fn list(root: &Path) -> io::Result<Listing> {
    let mut keys = Vec::new();
    for file in fs::read_dir(root)? {
        let file = file?;
        if file.path().extension().and_then(|e| e.to_str()) != Some("json") { continue; }
        regular(&file.path())?;
        let entry: Stored = serde_json::from_slice(&fs::read(file.path())?)?;
        if file.file_name() != std::ffi::OsStr::new(&encoded(&entry.key)?) {
            return Err(io::Error::other("Storage filename differs"));
        }
        keys.push(entry.key);
    }
    keys.sort();
    Ok(Listing { path: root.to_string_lossy().into_owned(), keys })
}

/// Only rename after the full file has been written and synced. The hook tests
/// failures before rename without depending on disk quota or OS permissions.
fn atomic_write(root: &Path, key: &str, contents: &str, before_rename: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    let destination = root.join(encoded(key)?);
    regular(&destination)?;
    let temporary = root.join(format!(".{}-{}.tmp", std::process::id(), NEXT_TEMP.fetch_add(1, Ordering::Relaxed)));
    let file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result = (|| {
        let mut file = file;
        serde_json::to_writer(&mut file, &Stored { key: key.into(), contents: contents.into() })?;
        file.flush()?;
        file.sync_all()?;
        drop(file); // Closed before rename, which Windows needs.
        before_rename()?;
        fs::rename(&temporary, &destination)?;
        sync_folder(root)
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}
fn remove(root: &Path, key: &str) -> io::Result<()> {
    let path = root.join(encoded(key)?);
    regular(&path)?;
    match fs::remove_file(path) {
        Ok(()) => sync_folder(root),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub fn store_list(app: AppHandle) -> Result<Listing, String> {
    list(&folder(&app)?).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn store_read(app: AppHandle, key: String) -> Result<Option<String>, String> {
    read(&folder(&app)?, &key).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn store_write(app: AppHandle, key: String, contents: String) -> Result<(), String> {
    atomic_write(&folder(&app)?, &key, &contents, || Ok(())).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn store_remove(app: AppHandle, key: String) -> Result<(), String> {
    remove(&folder(&app)?, &key).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("grainatlas-store-{}-{}", std::process::id(), NEXT_TEMP.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    #[test]
    fn encodes_long_unicode_and_path_keys_without_traversal() {
        for key in ["grainatlas.copy.uuid|/../../a\\b\0é".to_string(), format!("grainatlas.copy.{}", "path/".repeat(1000))] {
            let name = encoded(&key).unwrap();
            assert_eq!(name.len(), 69);
            assert!(name[..64].bytes().all(|b| b.is_ascii_hexdigit()));
            let temp = Temp::new();
            atomic_write(&temp.0, &key, " \n\0é𐐀 ", || Ok(())).unwrap();
            assert_eq!(read(&temp.0, &key).unwrap().as_deref(), Some(" \n\0é𐐀 "));
            assert_eq!(list(&temp.0).unwrap().keys, vec![key.clone()]);
            remove(&temp.0, &key).unwrap();
            assert_eq!(read(&temp.0, &key).unwrap(), None);
        }
        let temp = Temp::new();
        for key in ["../outside", "/tmp/outside", "grainatlas.settings", ""] {
            assert!(encoded(key).is_err());
            assert!(atomic_write(&temp.0, key, "bad", || Ok(())).is_err());
            assert!(read(&temp.0, key).is_err());
            assert!(remove(&temp.0, key).is_err());
        }
        assert!(list(&temp.0).unwrap().keys.is_empty());
    }
    #[test]
    fn failure_before_rename_keeps_old_file_and_cleans_temporary() {
        let temp = Temp::new();
        let key = "grainatlas.recent";
        atomic_write(&temp.0, key, "old", || Ok(())).unwrap();
        let original = fs::read(temp.0.join(encoded(key).unwrap())).unwrap();
        let result = atomic_write(&temp.0, key, "new", || {
            assert_eq!(read(&temp.0, key)?.as_deref(), Some("old"));
            let staged = fs::read_dir(&temp.0)?.filter_map(Result::ok).find(|f| f.path().extension().is_some_and(|e| e == "tmp")).unwrap();
            let value: Stored = serde_json::from_slice(&fs::read(staged.path())?)?;
            assert_eq!(value.contents, "new");
            Err(io::Error::other("Injected failure after sync"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(temp.0.join(encoded(key).unwrap())).unwrap(), original);
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
        atomic_write(&temp.0, key, "new", || Ok(())).unwrap();
        assert_eq!(read(&temp.0, key).unwrap().as_deref(), Some("new"));
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlink_entries() {
        let temp = Temp::new();
        let outside = Temp::new();
        let target = outside.0.join("outside.json");
        fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, temp.0.join(encoded("grainatlas.recent").unwrap())).unwrap();
        assert!(read(&temp.0, "grainatlas.recent").is_err());
        assert!(atomic_write(&temp.0, "grainatlas.recent", "bad", || Ok(())).is_err());
        assert!(remove(&temp.0, "grainatlas.recent").is_err());
        assert!(list(&temp.0).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "keep");
    }
}
