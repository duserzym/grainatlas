//! Render saved placements again, preserving the project's untyped annotations and geometry.
use std::path::{Component, Path};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result, bail};
use grainatlas_core::Ctl;
use grainatlas_core::photometric::background::LevelMethod;
use grainatlas_core::project::{Project, pipeline, session::Session};
use grainatlas_core::segment::Grain;
use grainatlas_core::source::{DirSource, FileListSource, Source};
use serde_json::{Value, json};

/// Detect app saves during computation before touching any saved files. The app currently
/// writes no on-disk open-project lock or marker (its session locks are in-memory mutexes).
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProjectVersion {
    modified: SystemTime,
    size: u64,
}

impl ProjectVersion {
    fn read(file: &Path) -> Result<Self> {
        let meta = std::fs::metadata(file)?;
        Ok(Self { modified: meta.modified()?, size: meta.len() })
    }

    fn check(&self, file: &Path) -> Result<()> {
        if Self::read(file)? != *self {
            bail!("{} changed during rerender; nothing was written; save and close the project in the app first", file.display());
        }
        Ok(())
    }
}

fn check_backups(out: &Path, stamp: &str) -> Result<()> {
    for name in [format!("project.before-rerender.{stamp}.json"), format!("mosaic.before-rerender.{stamp}")] {
        // Include dangling symlinks: they also occupy backup names.
        match std::fs::symlink_metadata(out.join(name)) {
            Ok(_) => bail!("rerender backup for {stamp} already exists; wait until the next minute"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Attempt every rollback operation even if an earlier move fails. Keep the backup and
/// report all failures so each area still needing manual recovery can be identified.
fn rollback(out: &Path, stage: &Path, mosaics: &Path, moved: &[&String], installed: &[&String], mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>) -> Vec<String> {
    let mut failures = Vec::new();
    for id in installed.iter().rev() {
        if let Err(e) = rename(&out.join("mosaic").join(id), &stage.join("mosaic").join(id)) {
            failures.push(format!("could not remove the new mosaic for {id}: {e}"));
        }
    }
    for id in moved.iter().rev() {
        if let Err(e) = rename(&mosaics.join(id), &out.join("mosaic").join(id)) {
            failures.push(format!("could not restore the original mosaic for {id}: {e}"));
        }
    }
    failures
}

fn area_path(id: &str) -> Result<()> {
    let mut parts = Path::new(id).components();
    if !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() {
        bail!("unsafe area id {id:?}");
    }
    Ok(())
}

/// All computation and geometry checks finish before any saved files are replaced. Staging is
/// on the same filesystem as the project, so each installation is a rename. On an installation
/// error, restore the moved mosaics; project.json is installed last.
fn install(file: &Path, stage: &Path, ids: &[String], stamp: &str, version: &ProjectVersion) -> Result<()> {
    let out = file.parent().context("project has no parent folder")?;
    let backup = out.join(format!("project.before-rerender.{stamp}.json"));
    let mosaics = out.join(format!("mosaic.before-rerender.{stamp}"));
    check_backups(out, stamp)?;
    version.check(file)?;
    // Copy the original bytes, never a reserialization, and never overwrite an older backup.
    let mut original = tempfile::NamedTempFile::new_in(out)?;
    std::io::copy(&mut std::fs::File::open(file)?, &mut original)?;
    original.persist_noclobber(&backup)?;
    let mut moved = Vec::new();
    let mut installed = Vec::new();
    let result = (|| -> Result<()> {
        std::fs::create_dir_all(out.join("mosaic"))?;
        for id in ids {
            let live = out.join("mosaic").join(id);
            if live.exists() {
                std::fs::create_dir_all(&mosaics)?;
                std::fs::rename(&live, mosaics.join(id))?;
                moved.push(id);
            }
            std::fs::rename(stage.join("mosaic").join(id), &live)?;
            installed.push(id);
        }
        std::fs::rename(stage.join("project.json"), file)?;
        Ok(())
    })();
    if let Err(error) = result {
        let failures = rollback(out, stage, &mosaics, &moved, &installed, |from, to| std::fs::rename(from, to));
        if !failures.is_empty() {
            bail!("{error}; rollback failures: {}", failures.join("; "));
        }
        return Err(error);
    }
    Ok(())
}

/// A session on the saved project's package with the saved project loaded: areas render from
/// their saved placements without stitching again.
pub(crate) fn open_saved(project: &Project, input: &str) -> Result<Session> {
    let mut session = Session::new(&project.name);
    session.load_project(input)?;
    let loose_files: Vec<_> = project.package.files.iter().map(std::path::PathBuf::from).collect();
    let is_loose = project.package.kind == "loose";
    let src: Arc<dyn Source> = if is_loose && !loose_files.is_empty() {
        Arc::new(FileListSource::new(&loose_files)?)
    } else {
        Arc::new(DirSource::new(&project.package.path).with_context(|| format!("package {}", project.package.path))?)
    };
    session.origin_files = project.package.files.clone();
    let probe: Value = serde_json::from_str(&session.open_source(src, &project.package.path, &project.created)?)?;
    if is_loose {
        session.open_loose(&serde_json::to_string(&project.package.loose)?, &project.created)?;
    } else if probe["status"] != "opened" {
        bail!("{} is not a package any more", project.package.path);
    }
    if session.project.as_ref().is_none_or(|p| p.uuid != project.uuid) {
        bail!("package {} does not match the saved project", project.package.path);
    }
    Ok(session)
}

/// The saved parameters of an area with the UI's tone, as the app renders it.
pub(crate) fn saved_params(full: &Value, saved: &grainatlas_core::project::AreaRecord) -> Result<grainatlas_core::project::AreaParams> {
    let mut params = saved.params.clone();
    if let Some(tone) = full["grainatlas_ui"]["tone"].get(&saved.id).filter(|tone| !tone.is_null()) {
        params.tone = serde_json::from_value(tone.clone()).with_context(|| format!("{}: UI tone", saved.id))?;
    }
    Ok(params)
}

pub fn run(path: &Path, areas: &[String], background: &str, dry_run: bool) -> Result<()> {
    let file = if path.is_dir() { path.join("project.json") } else { path.to_path_buf() };
    let out = file.parent().context("project has no parent folder")?;
    let version = ProjectVersion::read(&file)?;
    let stamp = super::now_rfc3339();
    let stamp = format!("{}_{}{}", &stamp[..10], &stamp[11..13], &stamp[14..16]);
    check_backups(out, &stamp)?;
    let input = std::fs::read_to_string(&file).with_context(|| file.display().to_string())?;
    version.check(&file)?;
    let mut full: Value = serde_json::from_str(&input)?;
    let project = Project::from_json(&input)?;
    for id in areas {
        if !project.areas.iter().any(|a| &a.id == id) {
            bail!("unknown area {id}");
        }
    }
    let mut selected = Vec::new();
    let mut summary = Vec::new();
    for (index, saved) in project.areas.iter().enumerate().filter(|(_, a)| areas.is_empty() || areas.contains(&a.id)) {
        if saved.placement.is_none() {
            if !areas.is_empty() {
                bail!("{} has no saved placement; nothing was written", saved.id);
            }
            let note = "no saved placement";
            eprintln!("{}: skipped ({note})", saved.id);
            summary.push(json!({"id": saved.id, "skipped": true, "note": note}));
            continue;
        }
        area_path(&saved.id)?;
        if saved.mosaic.is_none() {
            bail!("{} has no saved mosaic geometry; nothing was written", saved.id);
        }
        selected.push((index, saved));
    }
    if selected.is_empty() {
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }
    let mut session = open_saved(&project, &input)?;
    // Do not follow a mosaic symlink outside the saved project when replacing folders.
    for dir in std::iter::once(out.join("mosaic")).chain(selected.iter().map(|(_, a)| out.join("mosaic").join(&a.id))) {
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if !meta.is_dir() => bail!("{} is not a mosaic directory", dir.display()),
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    // A dry run does not even create temporary files. TempDir removes all staged files if a
    // later area fails validation; the original project and mosaics remain untouched.
    let stage = if dry_run { None } else { Some(tempfile::Builder::new().prefix(".rerender-").tempdir_in(out)?) };
    let mut ids = Vec::new();
    session.set_memory_budget(usize::MAX);
    for (index, saved) in selected {
        let start = Instant::now();
        let id = &saved.id;
        let mut params = saved_params(&full, saved)?;
        match background {
            "on" => {
                params.photometric.background = true;
                params.photometric.background_params.method = LevelMethod::Joint;
            }
            "off" => params.photometric.background = false,
            "keep" => {}
            _ => bail!("unknown background setting {background}"),
        }
        let grains: Vec<Grain> = full["grainatlas_ui"]["grains"].get(id).cloned().map(serde_json::from_value).transpose()?.unwrap_or_default();
        session.set_grains(id, &serde_json::to_string(&grains)?)?;
        session.recompute(id, &serde_json::to_string(&params)?, &Ctl::NONE, &|| start.elapsed().as_secs_f64() * 1000.)?;
        let rec = session.project.as_ref().and_then(|p| p.areas.iter().find(|a| a.id == *id)).context("area missing after reopen")?;
        let output = &session.outputs[id];
        let before = saved.mosaic.as_ref().expect("validated mosaic");
        let after = &output.mosaic_json;
        let mut previous = saved.placement.clone();
        if let Some(pl) = &mut previous {
            for (t, now) in pl.tiles.iter_mut().zip(&output.placement.tiles) { t.failed = now.failed; }
        }
        if rec.placement != previous || Some(&output.placement) != previous.as_ref()
            || before.w != after.w || before.h != after.h || before.um_per_px != after.um_per_px || before.origin_um != after.origin_um
        {
            bail!("{id}: recomputed geometry differs from saved geometry; nothing was written");
        }
        let area = &mut full["areas"][index];
        // Both objects are fully generated: merging would retain omitted keys (for
        // example an old fallback reason) and misrepresent the newly rendered pixels.
        area["photometric"] = serde_json::to_value(&rec.photometric)?;
        area["mosaic"] = serde_json::to_value(after)?;
        if !rec.failed_tiles.is_empty() || area.get("failed_tiles").is_some() {
            area["failed_tiles"] = json!(rec.failed_tiles);
        }
        for (i, t) in output.placement.tiles.iter().enumerate() {
            if t.failed || area["placement"]["tiles"][i].get("failed").is_some() {
                area["placement"]["tiles"][i]["failed"] = json!(t.failed);
            }
        }
        if params.photometric != saved.params.photometric {
            area["params"]["photometric"]["background"] = json!(params.photometric.background);
            if background == "on" {
                area["params"]["photometric"]["background_params"]["method"] = json!("joint");
            }
        }
        let photo = rec.photometric.as_ref().context("missing photometric result")?;
        let spots = full["grainatlas_ui"]["spots"].as_array().map_or(0, |s| s.iter().filter(|s| s["area_id"] == *id).count());
        summary.push(json!({"id": id, "seconds": start.elapsed().as_secs_f64(),
            "overlap_rms_before": photo.overlap_rms_before, "overlap_rms_after": photo.overlap_rms_after,
            "placement_unchanged": true, "grains": grains.iter().filter(|g| g.is_grain()).count(), "spots": spots}));
        if let Some(stage) = &stage {
            for (rel, bytes) in pipeline::area_files(rec, output) {
                super::write(&stage.path().join(rel), &bytes)?;
            }
        }
        ids.push(id.clone());
        session.outputs.remove(id);
        session.set_memory_budget(0);
        session.set_memory_budget(usize::MAX);
    }
    if let Some(stage) = &stage {
        super::write(&stage.path().join("project.json"), &serde_json::to_vec_pretty(&full)?)?;
        install(&file, stage.path(), &ids, &stamp, &version)?;
    }
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_install_restores_every_moved_mosaic_and_keeps_the_original_project() {
        let dir = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir_in(dir.path()).unwrap();
        let file = dir.path().join("project.json");
        let original = b"{\"original\": true}\n";
        super::super::write(&file, original).unwrap();
        for id in ["first", "second"] {
            super::super::write(&dir.path().join("mosaic").join(id).join("tile.png"), id.as_bytes()).unwrap();
        }
        super::super::write(&stage.path().join("mosaic/first/tile.png"), b"replacement").unwrap();
        super::super::write(&stage.path().join("project.json"), b"new project").unwrap();
        let version = ProjectVersion::read(&file).unwrap();
        // The second staged folder is absent: fail after installing the first and moving both.
        let result = install(&file, stage.path(), &["first".into(), "second".into()], "2026-11-10_1200", &version);
        assert!(result.is_err());
        assert_eq!(std::fs::read(&file).unwrap(), original);
        assert_eq!(std::fs::read(dir.path().join("project.before-rerender.2026-11-10_1200.json")).unwrap(), original);
        for id in ["first", "second"] {
            assert_eq!(std::fs::read(dir.path().join("mosaic").join(id).join("tile.png")).unwrap(), id.as_bytes());
        }
        // A pre-existing backup must cause refusal before any saved files are changed.
        assert!(install(&file, stage.path(), &["first".into()], "2026-11-10_1200", &version).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), original);
        assert_eq!(std::fs::read(dir.path().join("mosaic/first/tile.png")).unwrap(), b"first");
    }

    #[test]
    fn changed_project_is_refused_before_install_writes_anything() {
        for change in ["mtime", "size"] {
            let dir = tempfile::tempdir().unwrap();
            let stage = tempfile::tempdir_in(dir.path()).unwrap();
            let file = dir.path().join("project.json");
            super::super::write(&file, b"original").unwrap();
            super::super::write(&dir.path().join("mosaic/first/tile.png"), b"old pixels").unwrap();
            super::super::write(&stage.path().join("mosaic/first/tile.png"), b"new pixels").unwrap();
            super::super::write(&stage.path().join("project.json"), b"new project").unwrap();
            let version = ProjectVersion::read(&file).unwrap();
            if change == "mtime" {
                let modified = version.modified + std::time::Duration::from_secs(5);
                std::fs::File::options().write(true).open(&file).unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(modified)).unwrap();
                assert_eq!(ProjectVersion::read(&file).unwrap().size, version.size);
            } else {
                std::fs::write(&file, b"app saved a longer project").unwrap();
                // Exercise size independently of mtime.
                std::fs::File::options().write(true).open(&file).unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(version.modified)).unwrap();
            }
            let current = std::fs::read(&file).unwrap();
            let names: std::collections::BTreeSet<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
            let error = install(&file, stage.path(), &["first".into()], "test", &version).unwrap_err();
            assert!(error.to_string().contains("changed during rerender"), "{error}");
            assert_eq!(std::fs::read(&file).unwrap(), current);
            assert_eq!(std::fs::read(dir.path().join("mosaic/first/tile.png")).unwrap(), b"old pixels");
            assert_eq!(std::fs::read(stage.path().join("mosaic/first/tile.png")).unwrap(), b"new pixels");
            assert_eq!(std::fs::read(stage.path().join("project.json")).unwrap(), b"new project");
            let after: std::collections::BTreeSet<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
            assert_eq!(after, names, "no backup or temporary file created on refusal");
        }
    }

    #[test]
    fn occupied_backup_names_are_checked_before_parsing_or_rendering() {
        for name in ["project.before-rerender", "mosaic.before-rerender"] {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("project.json");
            // Parsing this would fail: the backup error must take precedence.
            std::fs::write(&file, b"not JSON").unwrap();
            let stamp = super::super::now_rfc3339();
            let stamp = format!("{}_{}{}", &stamp[..10], &stamp[11..13], &stamp[14..16]);
            let backup = if name == "project.before-rerender" { format!("{name}.{stamp}.json") } else { format!("{name}.{stamp}") };
            let backup = dir.path().join(backup);
            if name == "project.before-rerender" {
                std::fs::write(&backup, b"keep backup").unwrap();
            } else {
                std::fs::create_dir(&backup).unwrap();
            }
            let error = run(&file, &[], "keep", false).unwrap_err();
            assert!(error.to_string().contains("backup"), "{error}");
            assert_eq!(std::fs::read(&file).unwrap(), b"not JSON");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        }
    }

    #[test]
    fn rollback_continues_after_multiple_failures_and_reports_each_one() {
        let dir = tempfile::tempdir().unwrap();
        let stage = dir.path().join("stage");
        let backups = dir.path().join("backups");
        let ids: Vec<String> = ["first", "second", "third"].iter().map(|s| (*s).into()).collect();
        for id in &ids {
            super::super::write(&dir.path().join("mosaic").join(id).join("tile"), b"new").unwrap();
            super::super::write(&backups.join(id).join("tile"), b"old").unwrap();
        }
        std::fs::create_dir_all(stage.join("mosaic")).unwrap();
        let mut attempted = Vec::new();
        let refs: Vec<_> = ids.iter().collect();
        let failures = rollback(dir.path(), &stage, &backups, &refs, &refs, |from, to| {
            attempted.push(from.to_path_buf());
            if from == dir.path().join("mosaic/third") || from == backups.join("second") {
                return Err(std::io::Error::other("injected move failure"));
            }
            std::fs::rename(from, to)
        });
        assert_eq!(attempted.len(), 6, "try both loops despite removal and restore errors");
        assert_eq!(failures.len(), 3, "includes blocked restore of the third mosaic");
        assert!(failures[0].contains("remove the new mosaic for third"));
        assert!(failures[1].contains("restore the original mosaic for third"));
        assert!(failures[2].contains("restore the original mosaic for second"));
        assert_eq!(std::fs::read(dir.path().join("mosaic/first/tile")).unwrap(), b"old");
        assert_eq!(std::fs::read(backups.join("second/tile")).unwrap(), b"old");
        assert_eq!(std::fs::read(backups.join("third/tile")).unwrap(), b"old");
    }
}
