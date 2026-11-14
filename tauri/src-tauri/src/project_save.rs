use std::fs;
use std::io;
use std::path::Path;

/// Replace a folder member only after all of its bytes have been written beside it.
/// Each file is atomic; the folder as a whole is not a transaction.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing file name"))?.to_os_string();
    name.push(".tmp");
    let temporary = path.with_file_name(name);
    fs::write(&temporary, bytes)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!("grainatlas-save-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn replaces_complete_files_and_creates_nested_members() {
        let dir = Scratch::new();
        for name in ["project.json", "areas/a/grains.json", "areas/a/mosaic.png"] {
            let path = dir.0.join(name);
            write(&path, b"old content").unwrap();
            write(&path, b"new").unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"new");
            assert!(!path.with_file_name(format!("{}.tmp", path.file_name().unwrap().to_str().unwrap())).exists());
        }
    }

    #[test]
    fn failed_temp_write_keeps_the_original_bytes() {
        let dir = Scratch::new();
        let path = dir.0.join("spots.json");
        fs::write(&path, b"original").unwrap();
        fs::create_dir(dir.0.join("spots.json.tmp")).unwrap();
        assert!(write(&path, b"replacement").is_err());
        assert_eq!(fs::read(path).unwrap(), b"original");
    }

    #[test]
    fn failed_rename_keeps_the_target_and_cleans_up_temp() {
        let dir = Scratch::new();
        let path = dir.0.join("project.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), b"original").unwrap();
        assert!(write(&path, b"replacement").is_err());
        assert_eq!(fs::read(path.join("keep")).unwrap(), b"original");
        assert!(!dir.0.join("project.json.tmp").exists());
    }
}
