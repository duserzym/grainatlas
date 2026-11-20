//! Start-screen mirror. Read only the requested dataset/account; there is no account index.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Manager};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
const MAX_BYTES: usize = 20 * 1024 * 1024;

fn segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn relative(key: &str) -> io::Result<PathBuf> {
    let parts: Vec<_> = key.split('/').collect();
    let valid = match parts.as_slice() {
        ["mirror", dataset, account, "start-screen.json"] => segment(dataset) && segment(account),
        ["mirror", dataset, account, "tims", server, file] => {
            segment(dataset)
                && segment(account)
                && segment(server)
                && file.strip_suffix(".json").is_some_and(segment)
        }
        ["mirror-bindings", server, file] => {
            segment(server) && file.strip_suffix(".json").is_some_and(segment)
        }
        _ => false,
    };
    if !valid {
        return Err(io::Error::other("Invalid mirror key"));
    }
    Ok(PathBuf::from(key))
}
fn root(app: &AppHandle) -> Result<PathBuf, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match crate::launch_profile(&args) {
        Some(name) => Ok(app
            .path()
            .app_local_data_dir()
            .map_err(|e| e.to_string())?
            .join("profiles")
            .join(crate::profile::directory_name(&name))),
        None => app.path().app_data_dir().map_err(|e| e.to_string()),
    }
}
fn regular(path: &Path, directory: bool) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_symlink() || (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
        return Err(io::Error::other("Invalid mirror entry"));
    }
    Ok(())
}
fn private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    regular(path, true)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn read(root: &Path, key: &str) -> io::Result<Option<String>> {
    let relative = relative(key)?;
    match regular(root, true) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        result => result?,
    }
    let mut path = root.to_path_buf();
    // Check every component; a symlink cannot redirect a keyed read to another account.
    for part in relative.components() {
        path.push(part);
        if !path.exists() {
            return Ok(None);
        }
        regular(&path, path != root.join(&relative))?;
    }
    if fs::metadata(&path)?.len() > MAX_BYTES as u64 {
        return Err(io::Error::other("Mirror entry too large"));
    }
    fs::read_to_string(path).map(Some)
}
fn write(root: &Path, key: &str, contents: &str) -> io::Result<()> {
    let relative = relative(key)?;
    if contents.len() > MAX_BYTES {
        return Err(io::Error::other("Mirror entry too large"));
    }
    private_dir(root)?;
    let mut parent = root.to_path_buf();
    for part in relative.parent().unwrap().components() {
        parent.push(part);
        private_dir(&parent)?;
    }
    let destination = root.join(relative);
    if destination.exists() {
        regular(&destination, false)?;
    }
    let temporary = parent.join(format!(
        ".{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, destination)?;
        #[cfg(unix)]
        File::open(&parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
#[tauri::command]
pub fn mirror_read(app: AppHandle, key: String) -> Result<Option<String>, String> {
    read(&root(&app)?, &key).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn mirror_write(app: AppHandle, key: String, contents: String) -> Result<(), String> {
    write(&root(&app)?, &key, &contents).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyed_private_mirrors_survive_reopen_and_isolate_accounts_and_datasets() {
        let root = std::env::temp_dir().join(format!(
            "grainatlas-mirror-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let key = "mirror/dataset-a/account-a/start-screen.json";
        write(&root, key, "{\"pages\":{}}").unwrap();
        assert_eq!(read(&root, key).unwrap().as_deref(), Some("{\"pages\":{}}"));
        assert_eq!(
            read(&root, "mirror/dataset-a/account-b/start-screen.json").unwrap(),
            None
        );
        assert_eq!(
            read(&root, "mirror/dataset-b/account-a/start-screen.json").unwrap(),
            None
        );
        let tims_key = "mirror/dataset-a/account-a/tims/server-a/mount-a.json";
        write(
            &root,
            tims_key,
            "{\"outbox\":[{\"key\":\"pending-key\"}]}",
        )
        .unwrap();
        assert_eq!(
            read(&root, tims_key).unwrap().as_deref(),
            Some("{\"outbox\":[{\"key\":\"pending-key\"}]}")
        );
        assert_eq!(
            read(&root, "mirror/dataset-a/account-b/tims/server-a/mount-a.json").unwrap(),
            None
        );
        assert_eq!(
            read(&root, "mirror/dataset-a/account-a/tims/server-b/mount-a.json").unwrap(),
            None
        );
        for key in [
            "mirror/dataset-a/account-a/tims/../mount-a.json",
            "mirror/dataset-a/account-a/tims/server-a/../../mount-a.json",
            "mirror/a/../start-screen.json",
            "mirror/../a/start-screen.json",
            "/mirror/a/b/start-screen.json",
        ] {
            assert!(read(&root, key).is_err());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            assert_eq!(
                fs::metadata(root.join("mirror/dataset-a/account-a"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(root.join(key)).unwrap().permissions().mode() & 0o777,
                0o600
            );
            symlink(
                root.join("mirror/dataset-a/account-a"),
                root.join("mirror/dataset-a/account-c"),
            )
            .unwrap();
            assert!(read(&root, "mirror/dataset-a/account-c/start-screen.json").is_err());
            assert!(write(&root, "mirror/dataset-a/account-c/start-screen.json", "{}").is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
