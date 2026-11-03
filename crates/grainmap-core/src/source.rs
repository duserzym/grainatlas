//! Byte sources for packages: the only way the core sees files.

use std::collections::BTreeMap;

use crate::{Error, Result};

/// A tree of files addressed by relative paths with `/` separators.
pub trait Source: Send + Sync {
    /// All file paths, sorted.
    fn list(&self) -> Vec<String>;
    fn read(&self, path: &str) -> Result<Vec<u8>>;
    /// A label for the root (folder name), used as the mount name fallback.
    fn root_name(&self) -> String;
}

/// Files held in memory (the browser hands the selected folder over file by file).
#[derive(Default, Clone)]
pub struct MemSource {
    pub name: String,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl MemSource {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), files: BTreeMap::new() }
    }
    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) {
        self.files.insert(path.replace('\\', "/").trim_start_matches('/').to_string(), bytes);
    }
}

impl Source for MemSource {
    fn list(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }
    fn read(&self, path: &str) -> Result<Vec<u8>> {
        self.files.get(path).cloned().ok_or_else(|| Error::NotFound(path.to_string()))
    }
    fn root_name(&self) -> String {
        self.name.clone()
    }
}

/// A directory on disk (native targets only).
#[cfg(not(target_arch = "wasm32"))]
pub struct DirSource {
    pub root: std::path::PathBuf,
    files: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl DirSource {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Result<Self> {
        let root: std::path::PathBuf = root.into();
        if !root.is_dir() {
            return Err(Error::NotFound(root.display().to_string()));
        }
        let mut files = Vec::new();
        fn walk(base: &std::path::Path, dir: &std::path::Path, depth: usize, out: &mut Vec<String>) {
            if depth > 3 {
                return;
            }
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name.ends_with(".grainmap") {
                    continue;
                }
                if p.is_dir() {
                    walk(base, &p, depth + 1, out);
                } else if let Ok(rel) = p.strip_prefix(base) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        walk(&root, &root, 0, &mut files);
        files.sort();
        Ok(Self { root, files })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Source for DirSource {
    fn list(&self) -> Vec<String> {
        self.files.clone()
    }
    fn read(&self, path: &str) -> Result<Vec<u8>> {
        std::fs::read(self.root.join(path)).map_err(|e| Error::Read(path.to_string(), e.to_string()))
    }
    fn root_name(&self) -> String {
        self.root.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }
}

/// Last path component.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Parent directory ("" for top level).
pub fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}
