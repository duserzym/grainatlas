//! `grainatlas` command line: batch work and tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use grainatlas_core::Ctl;
use grainatlas_core::project::pipeline::{self, Timings};
use grainatlas_core::project::session::Session;
use grainatlas_core::project::{AreaParams, Project};
use grainatlas_core::readers::{self, Package};
use grainatlas_core::source::DirSource;

#[derive(Parser)]
#[command(name = "grainatlas", version, about = "Stitch, correct, segment and lay out SEM grain-mount images")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the areas and tiles of a package without stitching.
    Info { folder: PathBuf },
    /// Print `<area> <sha256>` of the base mosaic of every area with default parameters, then
    /// `<area>+auto+grains <sha256>` after Auto tone and per-grain normalisation on the default
    /// segmentation (the native half of the wasm/native parity check).
    Hash { folder: PathBuf },
    /// Write the atlas bundle of a package (every area stitched with defaults, grains found,
    /// Auto tone): a `<project_id>.atlas/` folder in OUT, or a zip when OUT ends in `.zip`.
    Atlas {
        folder: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Fixed project id (areas get the following ids); a new UUIDv7 otherwise.
        #[arg(long)]
        project_id: Option<String>,
        /// laicpmsdb mount the project is linked to.
        #[arg(long)]
        mount_id: Option<String>,
        /// 100 = lossless WebP tiles, 90 to 99 = near-lossless.
        #[arg(long, default_value_t = 100)]
        quality: u8,
        /// Place this many spots at the first grains' centroids (ids follow the project id + 0x100):
        /// test fixtures.
        #[arg(long, default_value_t = 0)]
        demo_spots: usize,
        /// `exported` time in atlas.json (default now).
        #[arg(long)]
        exported: Option<String>,
    },
    /// Stitch one area folder (or every area of a package folder) into a project folder.
    Stitch {
        /// An area folder (e.g. Sweden_XS_150X/005_AH1-GC1) or a package folder.
        folder: PathBuf,
        /// Project folder to write (`<name>.grainatlas`).
        #[arg(long)]
        out: PathBuf,
        /// Parameters as a JSON file (AreaParams); defaults otherwise.
        #[arg(long)]
        params: Option<PathBuf>,
        /// Linear feathering instead of seams + multiband.
        #[arg(long)]
        fast: bool,
        /// Skip writing pyramid tiles (timing runs).
        #[arg(long)]
        no_pyramid: bool,
        /// Also write the level-0 mosaic as PNG to exports/.
        #[arg(long)]
        png: bool,
        /// Also segment grains (masks/<area>/, exports/<area>_grains.csv).
        #[arg(long)]
        segment: bool,
        /// Also print grain maps to exports/<area>_grains.pdf and .svg pages (implies --segment).
        /// Paper: letter, tabloid (default), a4, a3.
        #[arg(long, value_name = "PAPER", num_args = 0..=1, default_missing_value = "tabloid")]
        print: Option<String>,
        /// Print scale in um of sample per mm of paper (0 = fit).
        #[arg(long, default_value_t = 0.0)]
        scale: f64,
        /// Sample name for spot names (`{sample}`) of the stitched area(s).
        #[arg(long)]
        sample: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Info { folder } => {
            let (pkg, _, _) = open(&folder)?;
            println!("{}", serde_json::to_string_pretty(pkg.info())?);
        }
        Cmd::Hash { folder } => {
            let (pkg, only, _) = open(&folder)?;
            let mut project = Project::new(pkg.info(), "hash", "1970-01-01T00:00:00Z");
            for (ai, area) in pkg.info().areas.iter().enumerate() {
                if only.as_ref().is_some_and(|o| o != &area.id) {
                    continue;
                }
                let rec = project.area_mut(&area.id).expect("area");
                let tiles = pipeline::read_tiles(pkg.as_ref(), ai, rec)?;
                let o = pipeline::run(rec, &tiles, &AreaParams::default(), &Ctl::NONE, &|| 0.0, &mut Timings::default())?;
                println!("{} {}", area.id, o.mosaic_json.sha256);
            }
            // The same steps as ui/scripts/wasm-hash.mjs, through the Session both front ends use.
            let mut s = Session::new("hash");
            s.open_source(Arc::new(DirSource::new(&folder)?), "hash", "1970-01-01T00:00:00Z")?;
            s.pin_identity(PARITY_ID, EPOCH)?;
            let mut grains = serde_json::Map::new();
            for area in &pkg.info().areas {
                if only.as_ref().is_some_and(|o| o != &area.id) {
                    continue;
                }
                let (hash, mut g) = auto_grains(&mut s, &area.id)?;
                println!("{}+auto+grains {hash}", area.id);
                pin_grain_ids(&mut g, PARITY_ID.parse()?, grains.values().map(|v| v.as_array().map_or(0, |a| a.len())).sum());
                grains.insert(area.id.clone(), g);
            }
            // The atlas bundle of that state (ui/scripts/wasm-hash.mjs does the same).
            let full = full_project(&s, serde_json::Value::Object(grains), serde_json::json!([]))?;
            let opts = serde_json::json!({ "quality": 100, "exported": EPOCH }).to_string();
            let files = s.atlas_files(&full, &opts)?;
            println!("bundle {}", grainatlas_core::util::sha256_hex(&files[0].1));
            println!("bundle.zip {}", grainatlas_core::util::sha256_hex(&grainatlas_core::atlas::zip(PARITY_ID, &files)?));
        }
        Cmd::Atlas { folder, out, project_id, mount_id, quality, demo_spots, exported } => {
            let (pkg, only, root) = open(&folder)?;
            let mut s = Session::new("atlas");
            s.open_source(Arc::new(DirSource::new(&root)?), &abs(&root), &now_rfc3339())?;
            if let Some(id) = &project_id {
                s.pin_identity(id, EPOCH)?;
            }
            if let Some(m) = &mount_id {
                s.set_link(m, "")?;
            }
            let mut grains = serde_json::Map::new();
            let mut spots = Vec::new();
            for area in &pkg.info().areas {
                if only.as_ref().is_some_and(|o| o != &area.id) {
                    continue;
                }
                let (_, mut g) = auto_grains(&mut s, &area.id)?;
                if project_id.is_some() {
                    let base = s.project.as_ref().expect("project").uuid;
                    pin_grain_ids(&mut g, base, grains.values().map(|v| v.as_array().map_or(0, |a| a.len())).sum());
                }
                let gs: Vec<grainatlas_core::segment::Grain> = serde_json::from_value(g.clone())?;
                let project = s.project.as_ref().expect("project");
                let rec = project.areas.iter().find(|a| a.id == area.id).expect("area");
                for g in gs.iter().take(demo_spots.saturating_sub(spots.len())) {
                    let mut sp = grainatlas_core::spots::new_spot(&rec.id, &rec.sample, &gs, &spots, &project.settings, g.centroid_um[0], g.centroid_um[1], 25.0, EPOCH);
                    sp.id = uuid_after(project.uuid, 0x100 + spots.len() as u128);
                    spots.push(sp);
                }
                grains.insert(area.id.clone(), g);
            }
            let full = full_project(&s, serde_json::Value::Object(grains), serde_json::to_value(&spots)?)?;
            let opts = serde_json::json!({ "quality": quality, "exported": exported.unwrap_or_else(now_rfc3339) }).to_string();
            let files = s.atlas_files(&full, &opts)?;
            let id = s.project.as_ref().expect("project").uuid.to_string();
            if out.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) {
                write(&out, &grainatlas_core::atlas::zip(&id, &files)?)?;
                eprintln!("{}: {} files", out.display(), files.len());
            } else {
                let dir = out.join(format!("{id}.atlas"));
                if dir.exists() {
                    std::fs::remove_dir_all(&dir)?;
                }
                for (rel, bytes) in &files {
                    write(&dir.join(rel), bytes)?;
                }
                eprintln!("{}: {} files", dir.display(), files.len());
            }
            println!("{id}");
        }
        Cmd::Stitch { folder, out, params, fast, no_pyramid, png, segment, print, scale, sample } => {
            let (pkg, only, root) = open(&folder)?;
            let mut params: AreaParams = match params {
                Some(p) => serde_json::from_str(&std::fs::read_to_string(&p)?)?,
                None => AreaParams::default(),
            };
            if fast {
                params.blend.mode = grainatlas_core::blend::BlendMode::Feather;
            }
            let pj = out.join("project.json");
            let mut project = if pj.exists() {
                Project::from_json(&std::fs::read_to_string(&pj)?)?
            } else {
                Project::new(pkg.info(), &abs(&root), &now_rfc3339())
            };
            let start = Instant::now();
            let clock = || start.elapsed().as_secs_f64() * 1000.0;
            let mut summary = Vec::new();
            for (ai, area) in pkg.info().areas.iter().enumerate() {
                if only.as_ref().is_some_and(|o| o != &area.id) {
                    continue;
                }
                if project.areas.iter().all(|a| a.id != area.id) {
                    project.areas.push(grainatlas_core::project::AreaRecord::from_meta(area));
                }
                let rec = project.area_mut(&area.id).expect("area");
                if let Some(s) = &sample {
                    rec.sample = s.clone();
                }
                let mut timings = Timings::default();
                let t0 = clock();
                let tiles = pipeline::read_tiles(pkg.as_ref(), ai, rec)?;
                timings.steps.push(("read".into(), clock() - t0));
                let o = pipeline::run(rec, &tiles, &params, &Ctl::NONE, &clock, &mut timings)?;
                let total: f64 = timings.steps.iter().map(|s| s.1).sum();
                let pairs_used = o.placement.pairs.iter().filter(|p| p.used).count();
                eprintln!(
                    "{}: {} tiles, {}x{} px, pairs {}/{} measured, rms {:.3} px, max {:.3} px, {:.0} ms ({})",
                    area.id,
                    tiles.len(),
                    o.linear.w,
                    o.linear.h,
                    pairs_used,
                    o.placement.pairs.len(),
                    o.placement.rms_residual,
                    o.placement.max_residual,
                    total,
                    timings.steps.iter().map(|(n, t)| format!("{n} {t:.0}")).collect::<Vec<_>>().join(", ")
                );
                summary.push(serde_json::json!({
                    "area": area.id, "tiles": tiles.len(), "w": o.linear.w, "h": o.linear.h,
                    "pairs": o.placement.pairs.len(), "pairs_measured": pairs_used,
                    "rms_px": o.placement.rms_residual, "max_px": o.placement.max_residual,
                    "ms": total, "steps": timings.steps, "sha256": o.mosaic_json.sha256,
                }));
                let mut grains_n = None;
                if segment || print.is_some() {
                    let t = clock();
                    let rec = project.area_mut(&area.id).expect("area");
                    let prev: Vec<grainatlas_core::segment::Grain> = std::fs::read(out.join(format!("masks/{}/grains.json", area.id)))
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                        .and_then(|v| serde_json::from_value(v["grains"].clone()).ok())
                        .unwrap_or_default();
                    let grains = pipeline::segment_area(rec, &o, &params.segment, &prev);
                    eprintln!("  {} grains in {:.0} ms", grains.len(), clock() - t);
                    for (rel, bytes) in pipeline::mask_files(rec, &o, &grains) {
                        write(&out.join(rel), &bytes)?;
                    }
                    let csv = grainatlas_core::segment::grains_csv(&grains, |x, y| rec.stage_of(x, y), |_| 0);
                    write(&out.join(format!("exports/{}_grains.csv", area.id)), csv.as_bytes())?;
                    grains_n = Some(grains.len());
                    // The UI keeps grains under `grainatlas_ui` in project.json.
                    let ui = project.extra.entry("grainatlas_ui".into()).or_insert_with(|| serde_json::json!({}));
                    ui["grains"][&area.id] = serde_json::to_value(&grains)?;
                    if let Some(paper) = &print {
                        let t = clock();
                        let rec = project.areas.iter().find(|a| a.id == area.id).expect("area");
                        let img = pipeline::area_image(rec, &o, &params.tone);
                        let mut gmap = std::collections::BTreeMap::new();
                        gmap.insert(rec.id.clone(), grains.clone());
                        let lp = grainatlas_core::layout::LayoutParams { paper: paper.clone(), um_per_mm: scale, title: format!("{} · {}", pkg.info().name, rec.name), ..Default::default() };
                        let r = grainatlas_core::layout::layout(&lp, &[img], &gmap, &[], &[]);
                        for (i, svg) in r.pages.iter().enumerate() {
                            write(&out.join(format!("exports/{}_page{}.svg", area.id, i + 1)), svg.as_bytes())?;
                        }
                        let pdf = grainatlas_core::layout::pdf::pdf_from_svgs(&r.pages)?;
                        write(&out.join(format!("exports/{}_grains.pdf", area.id)), &pdf)?;
                        eprintln!("  {} pages at {} um/mm in {:.0} ms", r.pages.len(), r.um_per_mm, clock() - t);
                    }
                }
                summary.last_mut().expect("summary")["grains"] = serde_json::json!(grains_n);
                let rec = project.areas.iter().find(|a| a.id == area.id).expect("area").clone();
                if !no_pyramid {
                    for (rel, bytes) in pipeline::area_files(&rec, &o) {
                        write(&out.join(rel), &bytes)?;
                    }
                }
                if png {
                    let b = grainatlas_core::image::encode_png_gray(o.linear.w, o.linear.h, &o.base);
                    write(&out.join(format!("exports/{}_mosaic.png", area.id)), &b)?;
                }
            }
            if summary.is_empty() {
                bail!("no area matched {}", folder.display());
            }
            write(&pj, project.to_json().as_bytes())?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
    }
    Ok(())
}

/// Opens a package; if `folder` is one area of a package (its parent holds the `.zag`), opens
/// the parent and returns the area id to restrict to. Also returns the package root.
fn open(folder: &Path) -> Result<(Box<dyn Package>, Option<String>, PathBuf)> {
    let folder = folder.canonicalize().with_context(|| format!("{}", folder.display()))?;
    if let Some(parent) = folder.parent() {
        let has_zag = std::fs::read_dir(parent)?.flatten().any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("zag")));
        if has_zag {
            let src = DirSource::new(parent)?;
            let pkg = readers::open(Arc::new(src))?;
            let id = folder.file_name().unwrap_or_default().to_string_lossy().to_string();
            if pkg.info().areas.iter().any(|a| a.id == id) {
                return Ok((pkg, Some(id), parent.to_path_buf()));
            }
        }
    }
    let pkg = readers::open(Arc::new(DirSource::new(&folder)?))?;
    Ok((pkg, None, folder))
}

const EPOCH: &str = "1970-01-01T00:00:00Z";
/// Project id of the parity bundle (`grainatlas hash`, ui/scripts/wasm-hash.mjs).
const PARITY_ID: &str = "00000000-0000-7000-8000-000000000000";

fn uuid_after(id: uuid::Uuid, k: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(id.as_u128().wrapping_add(k))
}

/// Grain ids are new UUIDv7s on every run; reproducible bundles number them from `base + 0x1000`.
fn pin_grain_ids(grains: &mut serde_json::Value, base: uuid::Uuid, first: usize) {
    for (i, g) in grains.as_array_mut().into_iter().flatten().enumerate() {
        g["id"] = serde_json::json!(uuid_after(base, 0x1000 + (first + i) as u128));
    }
}

/// The session's project with the UI-held parts the bundle reads (`grainatlas_ui`).
fn full_project(s: &Session, grains: serde_json::Value, spots: serde_json::Value) -> Result<String> {
    let mut v: serde_json::Value = serde_json::from_str(&s.project_json()?)?;
    v["grainatlas_ui"] = serde_json::json!({ "grains": grains, "spots": spots, "labels": [] });
    Ok(v.to_string())
}

/// Stitch with defaults, find grains, apply Auto and per-grain normalisation (strength 0.8, area
/// reference); returns the base mosaic's sha256 and the grains (JSON).
fn auto_grains(s: &mut Session, id: &str) -> Result<(String, serde_json::Value)> {
    let defaults = serde_json::to_string(&AreaParams::default())?;
    s.stitch(id, &defaults, &Ctl::NONE, &|| 0.0)?;
    let grains = s.segment(id, &serde_json::to_string(&grainatlas_core::segment::SegmentParams::default())?, "")?;
    s.set_grains(id, &grains)?;
    let mut p: AreaParams = serde_json::from_str(&s.auto_params(id, &defaults)?)?;
    p.tone.grain_norm.enabled = true;
    let r: serde_json::Value = serde_json::from_str(&s.recompute(id, &serde_json::to_string(&p)?, &Ctl::NONE, &|| 0.0)?)?;
    Ok((r["mosaic"]["sha256"].as_str().unwrap_or_default().to_string(), serde_json::from_str(&grains)?))
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, bytes).with_context(|| path.display().to_string())
}

fn abs(p: &Path) -> String {
    p.canonicalize().map(|p| p.display().to_string()).unwrap_or_else(|_| p.display().to_string())
}

/// UTC timestamp, RFC 3339, from the system clock (civil-from-days, H. Hinnant).
fn now_rfc3339() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (s.div_euclid(86400), s.rem_euclid(86400));
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}
