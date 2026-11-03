//! `grainmap` command line: batch work and tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use grainmap_core::Ctl;
use grainmap_core::project::pipeline::{self, Timings};
use grainmap_core::project::{AreaParams, Project};
use grainmap_core::readers::{self, Package};
use grainmap_core::source::DirSource;

#[derive(Parser)]
#[command(name = "grainmap", version, about = "Stitch, correct, segment and lay out SEM grain-mount images")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the areas and tiles of a package without stitching.
    Info { folder: PathBuf },
    /// Print `<area> <sha256>` of the base mosaic of every area with default parameters
    /// (the native half of the wasm/native parity check).
    Hash { folder: PathBuf },
    /// Stitch one area folder (or every area of a package folder) into a project folder.
    Stitch {
        /// An area folder (e.g. Sweden_XS_150X/005_AH1-GC1) or a package folder.
        folder: PathBuf,
        /// Project folder to write (`<name>.grainmap`).
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
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Info { folder } => {
            let (pkg, _) = open(&folder)?;
            println!("{}", serde_json::to_string_pretty(pkg.info())?);
        }
        Cmd::Hash { folder } => {
            let (pkg, only) = open(&folder)?;
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
        }
        Cmd::Stitch { folder, out, params, fast, no_pyramid, png } => {
            let (pkg, only) = open(&folder)?;
            let mut params: AreaParams = match params {
                Some(p) => serde_json::from_str(&std::fs::read_to_string(&p)?)?,
                None => AreaParams::default(),
            };
            if fast {
                params.blend.mode = grainmap_core::blend::BlendMode::Feather;
            }
            let pj = out.join("project.json");
            let mut project = if pj.exists() {
                Project::from_json(&std::fs::read_to_string(&pj)?)?
            } else {
                Project::new(pkg.info(), &abs(&folder), &now_rfc3339())
            };
            let start = Instant::now();
            let clock = || start.elapsed().as_secs_f64() * 1000.0;
            let mut summary = Vec::new();
            for (ai, area) in pkg.info().areas.iter().enumerate() {
                if only.as_ref().is_some_and(|o| o != &area.id) {
                    continue;
                }
                if project.areas.iter().all(|a| a.id != area.id) {
                    project.areas.push(grainmap_core::project::AreaRecord::from_meta(area));
                }
                let rec = project.area_mut(&area.id).expect("area");
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
                let rec = project.areas.iter().find(|a| a.id == area.id).expect("area").clone();
                if !no_pyramid {
                    for (rel, bytes) in pipeline::area_files(&rec, &o) {
                        write(&out.join(rel), &bytes)?;
                    }
                }
                if png {
                    let b = grainmap_core::image::encode_png_gray(o.linear.w, o.linear.h, &o.base);
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
/// the parent and returns the area id to restrict to.
fn open(folder: &Path) -> Result<(Box<dyn Package>, Option<String>)> {
    let folder = folder.canonicalize().with_context(|| format!("{}", folder.display()))?;
    if let Some(parent) = folder.parent() {
        let has_zag = std::fs::read_dir(parent)?.flatten().any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("zag")));
        if has_zag {
            let src = DirSource::new(parent)?;
            let pkg = readers::open(Arc::new(src))?;
            let id = folder.file_name().unwrap_or_default().to_string_lossy().to_string();
            if pkg.info().areas.iter().any(|a| a.id == id) {
                return Ok((pkg, Some(id)));
            }
        }
    }
    let pkg = readers::open(Arc::new(DirSource::new(&folder)?))?;
    Ok((pkg, None))
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
