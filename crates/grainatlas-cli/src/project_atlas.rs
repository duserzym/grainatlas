//! The atlas bundle of a saved project, as the app's Save atlas / Publish writes it: every area
//! rendered from its saved placement, parameters and tone, with the project's own grains, spots
//! and labels. The project folder is only read.
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use grainatlas_core::Ctl;
use grainatlas_core::project::Project;
use grainatlas_core::segment::Grain;
use serde_json::{Value, json};

use super::rerender::{open_saved, saved_params};

pub fn run(path: &Path, out: &Path, mount_id: Option<&str>, quality: u8, exported: Option<String>) -> Result<Value> {
    let file = if path.is_dir() { path.join("project.json") } else { path.to_path_buf() };
    let input = std::fs::read_to_string(&file).with_context(|| file.display().to_string())?;
    let mut full: Value = serde_json::from_str(&input)?;
    if let Some(m) = mount_id {
        full["laicpmsdb"]["mount_id"] = json!(m);
    }
    let input = full.to_string();
    let project = Project::from_json(&input)?;
    let mut session = open_saved(&project, &input)?;
    session.set_memory_budget(usize::MAX);
    let mut areas = Vec::new();
    for saved in &project.areas {
        if saved.placement.is_none() {
            eprintln!("{}: skipped (no saved placement)", saved.id);
            areas.push(json!({"id": saved.id, "skipped": "no saved placement"}));
            continue;
        }
        let start = Instant::now();
        let params = saved_params(&full, saved)?;
        let grains: Vec<Grain> = full["grainatlas_ui"]["grains"].get(&saved.id).cloned().map(serde_json::from_value).transpose()?.unwrap_or_default();
        session.set_grains(&saved.id, &serde_json::to_string(&grains)?)?;
        session.recompute(&saved.id, &serde_json::to_string(&params)?, &Ctl::NONE, &|| start.elapsed().as_secs_f64() * 1000.)?;
        let o = &session.outputs[&saved.id];
        let geometry_kept = saved.mosaic.as_ref().is_none_or(|m| m.w == o.mosaic_json.w && m.h == o.mosaic_json.h && m.um_per_px == o.mosaic_json.um_per_px);
        let spots = full["grainatlas_ui"]["spots"].as_array().map_or(0, |s| s.iter().filter(|s| s["area_id"] == saved.id.as_str()).count());
        eprintln!("{}: rendered in {:.1} s", saved.id, start.elapsed().as_secs_f64());
        areas.push(json!({"id": saved.id, "grains": grains.iter().filter(|g| g.is_grain()).count(), "spots": spots,
                          "geometry_kept": geometry_kept, "seconds": start.elapsed().as_secs_f64()}));
    }
    let opts = json!({"quality": quality, "exported": exported.unwrap_or_else(super::now_rfc3339)}).to_string();
    let files = session.atlas_files(&input, &opts)?;
    let id = project.uuid.to_string();
    let zip = grainatlas_core::atlas::zip(&id, &files)?;
    super::write(out, &zip)?;
    Ok(json!({"project_id": id, "name": project.name, "out": out, "files": files.len(), "bytes": zip.len(),
              "zip_sha256": grainatlas_core::util::sha256_hex(&zip), "mount_id": full["laicpmsdb"]["mount_id"], "areas": areas}))
}
