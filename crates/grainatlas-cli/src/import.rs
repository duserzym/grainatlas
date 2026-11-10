//! `grainatlas import-grains`: copy saved grains from another copy of the same project (for
//! example the copy a print was made from) into areas of this project, so the grain numbers in
//! the app equal the printed ones.
//!
//! Only areas whose mosaic is provably the same are touched: same project uuid, same area uuid,
//! same mosaic hash and the same tile placement. Grains are copied as they are (ids, numbers,
//! outlines, artifact decisions); spots in those areas are linked to the grain under them. Every
//! other part of project.json is left as it was.

use std::path::Path;

use anyhow::{Context, Result, bail};
use grainatlas_core::segment::Grain;
use serde_json::{Value, json};

fn load(path: &Path) -> Result<(std::path::PathBuf, Value)> {
    let file = if path.is_dir() { path.join("project.json") } else { path.to_path_buf() };
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).with_context(|| file.display().to_string())?)?;
    Ok((file, v))
}

fn area<'a>(project: &'a Value, id: &str) -> Option<&'a Value> {
    project["areas"].as_array()?.iter().find(|a| a["id"] == id)
}

fn has_grains(project: &Value, id: &str) -> bool {
    project.get("grainatlas_ui").and_then(|u| u.get("grains")).and_then(|g| g.get(id)).is_some()
}

/// Why the source area's grains cannot be placed on the target area, if they cannot.
fn mismatch(src: &Value, dst: &Value) -> Option<String> {
    let checks = [("uuid", &src["uuid"], &dst["uuid"]), ("mosaic hash", &src["mosaic"]["sha256"], &dst["mosaic"]["sha256"]), ("mosaic size", &json!([src["mosaic"]["w"], src["mosaic"]["h"]]), &json!([dst["mosaic"]["w"], dst["mosaic"]["h"]])), ("pixel size", &src["pixel_size_nm"], &dst["pixel_size_nm"]), ("placement", &src["placement"], &dst["placement"])];
    for (what, a, b) in checks {
        if a.is_null() || a != b {
            return Some(format!("{what} differs"));
        }
    }
    None
}

pub fn run(path: &Path, from: &Path, areas: &[String], missing: bool, dry_run: bool) -> Result<()> {
    if areas.is_empty() && !missing {
        bail!("name the areas with --area, or pass --missing for every area without grains");
    }
    let (file, mut full) = load(path)?;
    let (_, source) = load(from)?;
    if full["uuid"] != source["uuid"] {
        bail!("{} is not a copy of this project (project uuid {} vs {})", from.display(), source["uuid"], full["uuid"]);
    }
    let ids: Vec<String> = full["areas"].as_array().context("project has no areas")?.iter().filter_map(|a| a["id"].as_str().map(String::from)).collect();
    for a in areas {
        if !ids.contains(a) {
            bail!("unknown area {a}");
        }
    }
    let todo: Vec<String> = ids.into_iter().filter(|id| if areas.is_empty() { !has_grains(&full, id) } else { areas.contains(id) }).collect();
    let mut summary = Vec::new();
    for id in &todo {
        if has_grains(&full, id) {
            bail!("{id} already has grains; this command never replaces grains");
        }
        let (Some(dst), Some(src)) = (area(&full, id), area(&source, id)) else {
            bail!("{id} is not in {}", from.display());
        };
        if let Some(why) = mismatch(src, dst) {
            bail!("{id}: the source mosaic is not this area's mosaic ({why}); nothing was written");
        }
        let Some(raw) = source.get("grainatlas_ui").and_then(|u| u.get("grains")).and_then(|g| g.get(id)) else {
            bail!("{id} has no grains in {}", from.display());
        };
        let grains: Vec<Grain> = serde_json::from_value(raw.clone()).with_context(|| format!("grains of {id}"))?;
        let n = grains.iter().filter(|g| g.is_grain()).count();
        full["grainatlas_ui"]["grains"][id] = raw.clone();
        super::segment::relink_spots(&mut full, id, &grains)?;
        let linked = full["grainatlas_ui"]["spots"].as_array().map_or(0, |s| s.iter().filter(|s| s["area_id"] == id.as_str() && !s["grain_id"].is_null()).count());
        let spots = full["grainatlas_ui"]["spots"].as_array().map_or(0, |s| s.iter().filter(|s| s["area_id"] == id.as_str()).count());
        eprintln!("{id}: {n} grains, {} artifacts, {linked} of {spots} spots on a grain", grains.len() - n);
        summary.push(json!({"area": id, "grains": n, "artifacts": grains.len() - n, "spots": spots, "spots_on_grains": linked}));
    }
    if !dry_run && !todo.is_empty() {
        let tmp = file.with_extension("json.tmp");
        super::write(&tmp, &serde_json::to_vec_pretty(&full)?)?;
        std::fs::rename(&tmp, &file)?;
    }
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
