//! Segment an existing project using the same saved-placement rendering path as the app.
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use grainatlas_core::Ctl;
use grainatlas_core::project::{Project, pipeline, session::Session};
use grainatlas_core::segment::{self, Grain, SegmentParams, Signal};
use grainatlas_core::source::{DirSource, FileListSource, Source};
use serde_json::{Value, json};

pub fn signal_override(value: &str) -> Option<Signal> {
    match value {
        "cl" => Some(Signal::Cl),
        "bse" => Some(Signal::Bse),
        _ => None,
    }
}

/// A machine-readable report and the same information on stderr for batch users.
pub fn report(area: &str, grains: &[Grain], signal: Signal, source: &str, threshold: f32) -> Value {
    let mut why = BTreeMap::new();
    for reason in grains.iter().filter_map(|g| g.artifact) {
        *why.entry(reason.label()).or_insert(0usize) += 1;
    }
    let flagged: usize = why.values().sum();
    let warning = segment::summary_warning(grains, signal, area, threshold);
    eprintln!(
        "{area}: {} grains, {flagged} artifacts ({}), {} from {source}",
        grains.len() - flagged,
        why.iter()
            .map(|(rule, n)| format!("{rule} {n}"))
            .collect::<Vec<_>>()
            .join(", "),
        signal.label()
    );
    if let Some(message) = &warning {
        eprintln!("warning: {message}");
    }
    json!({"area": area, "grains": grains.len() - flagged, "artifacts": why,
        "signal": signal, "signal_source": source, "threshold": threshold, "warning": warning})
}

/// Links every spot of `area` in a full project JSON to the grain under it (artifacts excluded),
/// as the app does; names, ids and positions are never touched.
pub fn relink_spots(full: &mut Value, area: &str, grains: &[Grain]) -> Result<()> {
    if let Some(spots) = full.get_mut("grainatlas_ui").and_then(|ui| ui.get_mut("spots")).and_then(Value::as_array_mut) {
        for spot in spots.iter_mut().filter(|s| s["area_id"] == area) {
            let x = spot["x_um"].as_f64().context("spot has no x_um")?;
            let y = spot["y_um"].as_f64().context("spot has no y_um")?;
            let grain = grainatlas_core::spots::grain_at(grains, x, y);
            spot["grain_id"] = json!(grain.map(|g| g.id));
            spot["grain_number"] = json!(grain.map(|g| g.number));
        }
    }
    Ok(())
}

/// Only previously unlinked spots inside a new real grain receive a link.
fn link_unlinked_spots(full: &mut Value, area: &str, grains: &[Grain]) -> Result<()> {
    if let Some(spots) = full.get_mut("grainatlas_ui").and_then(|ui| ui.get_mut("spots")).and_then(Value::as_array_mut) {
        for spot in spots.iter_mut().filter(|s| s["area_id"] == area && s["grain_id"].is_null()) {
            let x = spot["x_um"].as_f64().context("spot has no x_um")?;
            let y = spot["y_um"].as_f64().context("spot has no y_um")?;
            if let Some(g) = grainatlas_core::spots::grain_at(grains, x, y) {
                spot["grain_id"] = json!(g.id);
                spot["grain_number"] = json!(g.number);
            }
        }
    }
    Ok(())
}

pub fn run(path: &Path, areas: &[String], missing: bool, add_missing: bool, dry_run: bool, flag: Option<&str>) -> Result<()> {
    let file = if path.is_dir() {
        path.join("project.json")
    } else {
        path.to_path_buf()
    };
    let out = file.parent().context("project has no parent folder")?;
    let input = std::fs::read_to_string(&file)?;
    // Keep the untyped value: serialization of Project would fill defaults and drop unknown
    // nested fields. Only the selected grain entries and their spots' two links may change.
    let mut full: Value = serde_json::from_str(&input)?;
    let project = Project::from_json(&input)?;
    for id in areas {
        if !project.areas.iter().any(|a| &a.id == id) {
            bail!("unknown area {id}");
        }
    }
    let ids: Vec<_> = project
        .areas
        .iter()
        .filter(|a| areas.is_empty() || areas.contains(&a.id))
        .filter(|a| !missing || full["grainatlas_ui"]["grains"].get(&a.id).is_none())
        .filter(|a| {
            if add_missing && full["grainatlas_ui"]["grains"].get(&a.id).is_none() {
                eprintln!("{}: never segmented, use segment --missing", a.id);
                false
            } else {
                true
            }
        })
        .map(|a| a.id.clone())
        .collect();
    if ids.is_empty() {
        println!("[]");
        return Ok(());
    }
    let mut session = Session::new(&project.name);
    session.load_project(&input)?;
    let loose_files: Vec<_> = project
        .package
        .files
        .iter()
        .map(std::path::PathBuf::from)
        .collect();
    let is_loose = project.package.kind == "loose";
    let src: Arc<dyn Source> = if is_loose && !loose_files.is_empty() {
        Arc::new(FileListSource::new(&loose_files)?)
    } else {
        Arc::new(
            DirSource::new(&project.package.path)
                .with_context(|| format!("package {}", project.package.path))?,
        )
    };
    session.origin_files = project.package.files.clone();
    let probe: Value = serde_json::from_str(&session.open_source(
        src,
        &project.package.path,
        &project.created,
    )?)?;
    if is_loose {
        session.open_loose(
            &serde_json::to_string(&project.package.loose)?,
            &project.created,
        )?;
    } else if probe["status"] != "opened" {
        bail!("{} is not a package any more", project.package.path);
    }
    if session
        .project
        .as_ref()
        .is_none_or(|loaded| loaded.uuid != project.uuid)
    {
        bail!(
            "package {} does not match the saved project",
            project.package.path
        );
    }
    // Rehydrate exactly one area at a time; keep its heavy pixels through mask export.
    let outline = serde_json::to_string(&segment::OutlinePad::from(&project.settings))?;
    let mut summary = Vec::new();
    for id in ids {
        session.set_memory_budget(usize::MAX);
        let saved = project
            .areas
            .iter()
            .find(|a| a.id == id)
            .expect("selected area");
        let mut p: SegmentParams = saved.params.segment.clone();
        if let Some(flag) = flag {
            p.signal = signal_override(flag);
        }
        let source = if flag.is_some_and(|f| f != "auto") {
            "flag"
        } else if flag.is_none() && saved.params.segment.signal.is_some() {
            "project"
        } else {
            "metadata"
        };
        let previous: Vec<Grain> = full["grainatlas_ui"]["grains"]
            .get(&id)
            .cloned()
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_default();
        session.set_grains(&id, &serde_json::to_string(&previous)?)?;
        session.recompute(
            &id,
            &serde_json::to_string(&saved.params)?,
            &Ctl::NONE,
            &|| 0.0,
        )?;
        let segment = if add_missing { Session::segment_missing } else { Session::segment };
        let grains: Vec<Grain> = serde_json::from_str(&segment(&mut session,
            &id,
            &serde_json::to_string(&p)?,
            &serde_json::to_string(&previous)?,
            &outline,
        )?)?;
        let rec = session
            .project
            .as_ref()
            .and_then(|p| p.areas.iter().find(|a| a.id == id))
            .context("area missing after reopen")?;
        let output = &session.outputs[&id];
        let work = pipeline::work_image(output, rec, p.work_scale);
        let new = &grains[if add_missing { previous.len() } else { 0 }..];
        if add_missing {
            let mut why = BTreeMap::new();
            for reason in new.iter().filter_map(|g| g.artifact) {
                *why.entry(reason.label()).or_insert(0usize) += 1;
            }
            let existing_grains = previous.iter().filter(|g| g.is_grain()).count();
            let added_grains = new.iter().filter(|g| g.is_grain()).count();
            eprintln!("{id}: {existing_grains} existing grains, {added_grains} added grains, added artifacts: {}", json!(why));
            summary.push(json!({"area": id, "existing_grains": existing_grains,
                "added_grains": added_grains, "added_artifacts": why}));
        } else {
            summary.push(report(
                &id, &grains, p.resolved_signal(&work), source, segment::grain_mask(&work, &p).1,
            ));
        }
        if dry_run {
            session.outputs.remove(&id);
            session.set_memory_budget(0);
            continue;
        }
        for (rel, bytes) in pipeline::mask_files(rec, output, &grains) {
            super::write(&out.join(rel), &bytes)?;
        }
        let csv = segment::grains_csv(
            &grains,
            |x, y| rec.stage_of(x, y),
            |id| {
                full["grainatlas_ui"]["spots"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|s| {
                        s["area_id"] == rec.id
                            && s["x_um"]
                                .as_f64()
                                .zip(s["y_um"].as_f64())
                                .and_then(|(x, y)| grainatlas_core::spots::grain_at(&grains, x, y))
                                .is_some_and(|g| &g.id == id)
                    })
                    .count()
            },
        );
        super::write(
            &out.join(format!("exports/{id}_grains.csv")),
            csv.as_bytes(),
        )?;
        if add_missing {
            link_unlinked_spots(&mut full, &id, new)?;
            let entries = full["grainatlas_ui"]["grains"][&id].as_array_mut().context("grains must be an array")?;
            entries.extend(new.iter().map(serde_json::to_value).collect::<Result<Vec<_>, _>>()?);
        } else {
            relink_spots(&mut full, &id, &grains)?;
            full["grainatlas_ui"]["grains"][&id] = serde_json::to_value(grains)?;
        }
        session.outputs.remove(&id);
        session.set_memory_budget(0);
    }
    // Write the project last, atomically, after all areas and exports succeeded.
    if !dry_run {
        let tmp = file.with_extension("json.tmp");
        super::write(&tmp, &serde_json::to_vec_pretty(&full)?)?;
        std::fs::rename(&tmp, &file)?;
    }
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
