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
    /// Photometric report: stitch every area once, then correct and blend it with each parameter
    /// set, print the overlap RMS (pixel and block-mean, grey levels) and write Auto-toned mosaics
    /// reduced by SCALE to OUT/<area>_<variant>.png.
    Photometry {
        folder: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Parameter sets as NAME=FILE.json (AreaParams); `default` = the defaults.
        #[arg(long = "variant", value_name = "NAME=FILE")]
        variants: Vec<String>,
        #[arg(long, default_value_t = 4)]
        scale: usize,
        /// Only this area id.
        #[arg(long)]
        area: Option<String>,
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
        /// Also print (implies --segment). Paper: tabloid (default), letter, a4, a3.
        /// Sheets (default): every owned grain of the package on landscape sheets at a fixed scale,
        /// exports/<package>_grain_sheets_<paper>.pdf with <package>_print_registration.json
        /// (frame print_sheet). With --print-style grains: one packed grain map per area,
        /// exports/<area>_grains.pdf.
        #[arg(long, value_name = "PAPER", num_args = 0..=1, default_missing_value = "tabloid")]
        print: Option<String>,
        /// sheets (default) or grains.
        #[arg(long, default_value = "sheets")]
        print_style: String,
        /// Print scale in um of sample per mm of paper (sheets: default 8; grains: 0 = fit).
        #[arg(long, default_value_t = 0.0)]
        scale: f64,
        /// Sheets: no non-local means denoising of the bands.
        #[arg(long)]
        no_denoise: bool,
        /// Sheets: no unsharp mask.
        #[arg(long)]
        no_sharpen: bool,
        /// Sheets: make everything outside the dilated grain outlines this grey (0 to 255),
        /// where every grain in a band has a closed outline and nothing else stands out.
        #[arg(long, value_name = "GREY", num_args = 0..=1, default_missing_value = "22")]
        strip_bg: Option<u8>,
        /// Sheets: title (default: the package name).
        #[arg(long)]
        title: Option<String>,
        /// Sheets: lab and operator line of the header.
        #[arg(long)]
        lab: Option<String>,
        /// Sample name for spot names (`{sample}`) of the stitched area(s).
        #[arg(long)]
        sample: Option<String>,
        /// One-click Auto tone (background leveling, stretch, CLAHE) on every area, as in the app.
        #[arg(long)]
        auto: bool,
        /// Grain polarity: bright (default), dark (transmitted light) or both (bright grains plus
        /// CL-dark grains relative to the local epoxy level).
        #[arg(long)]
        polarity: Option<String>,
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
        Cmd::Photometry { folder, out, variants, scale, area } => {
            let (pkg, only, _) = open(&folder)?;
            let only = area.or(only);
            let mut sets: Vec<(String, AreaParams)> = Vec::new();
            for v in if variants.is_empty() { vec!["default".to_string()] } else { variants } {
                let (name, file) = v.split_once('=').unwrap_or((v.as_str(), ""));
                let p = if file.is_empty() { AreaParams::default() } else { serde_json::from_str(&std::fs::read_to_string(file)?)? };
                sets.push((name.to_string(), p));
            }
            let mut project = Project::new(pkg.info(), "photometry", EPOCH);
            std::fs::create_dir_all(&out)?;
            let mut rows = Vec::new();
            for (ai, a) in pkg.info().areas.iter().enumerate() {
                if only.as_ref().is_some_and(|o| o != &a.id) {
                    continue;
                }
                let rec = project.area_mut(&a.id).expect("area");
                let tiles = pipeline::read_tiles(pkg.as_ref(), ai, rec)?;
                let inputs = pipeline::inputs(rec, &tiles);
                let placement = grainatlas_core::mosaic::stitch(&inputs.meta, &inputs.imgs, &sets[0].1.stitch, &Ctl::NONE)?;
                write(&out.join(format!("{}_placement.json", a.id)), serde_json::to_string(&placement)?.as_bytes())?;
                let (raw_px, raw_blk) = grainatlas_core::photometric::overlap_metrics(&placement, &inputs.imgs, 32);
                let mut row = serde_json::json!({ "area": a.id, "tiles": inputs.imgs.len(), "raw_px": raw_px * 255.0, "raw_block": raw_blk * 255.0 });
                for (name, p) in &sets {
                    let t = Instant::now();
                    let (corrected, photo) = grainatlas_core::photometric::correct(&placement, &inputs.imgs, &p.photometric);
                    let (px, blk) = grainatlas_core::photometric::overlap_metrics(&placement, &corrected, 32);
                    let ms = t.elapsed().as_secs_f64() * 1000.0;
                    let fill = if p.photometric.background { photo.background_median } else { 0.0 };
                    let (linear, cov) = grainatlas_core::blend::blend(&placement, corrected, &p.blend, fill, &Ctl::NONE)?;
                    let tone = grainatlas_core::photometric::tone::auto(&linear, Some(&cov), 1.0, &p.tone);
                    let flatness = grainatlas_core::photometric::joint::background_flatness(&linear, &cov, 32);
                    let small = linear.downsample(scale.max(1));
                    let scov: Vec<f32> = grainatlas_core::image::Gray { w: linear.w, h: linear.h, px: cov }.downsample(scale.max(1)).px;
                    let base = pipeline::tone_base(&small, &scov, &tone);
                    write(&out.join(format!("{}_{name}.png", a.id)), &grainatlas_core::image::encode_png_gray(small.w, small.h, &base))?;
                    // The same stretch without CLAHE: what the correction did, before local contrast.
                    let flat = grainatlas_core::photometric::ToneParams { clahe: false, ..tone.clone() };
                    let base = pipeline::tone_base(&small, &scov, &flat);
                    write(&out.join(format!("{}_{name}_stretch.png", a.id)), &grainatlas_core::image::encode_png_gray(small.w, small.h, &base))?;
                    row[name] = serde_json::json!({ "px": px * 255.0, "block": blk * 255.0, "app_before": photo.overlap_rms_before * 255.0, "app_after": photo.overlap_rms_after * 255.0, "flat": flatness * 255.0, "ms": ms });
                }
                eprintln!("{row}");
                rows.push(row);
            }
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        Cmd::Stitch { folder, out, params, fast, no_pyramid, png, segment, print, print_style, scale, no_denoise, no_sharpen, strip_bg, title, lab, sample, auto, polarity } => {
            let sheets = print.is_some() && print_style != "grains";
            if !matches!(print_style.as_str(), "sheets" | "grains") {
                bail!("--print-style {print_style}: sheets or grains");
            }
            let (pkg, only, root) = open(&folder)?;
            let mut params: AreaParams = match params {
                Some(p) => serde_json::from_str(&std::fs::read_to_string(&p)?)?,
                None => AreaParams::default(),
            };
            if fast {
                params.blend.mode = grainatlas_core::blend::BlendMode::Feather;
            }
            if let Some(pol) = &polarity {
                params.segment.polarity = serde_json::from_value(serde_json::json!(pol)).with_context(|| format!("--polarity {pol}: bright, dark or both"))?;
            }
            if auto {
                params.photometric.background = true;
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
            // Kept for the sheets: each area's toned mosaic and grains.
            let mut kept: Vec<(String, usize, usize, Vec<u8>)> = Vec::new();
            let mut all_grains: std::collections::BTreeMap<String, Vec<grainatlas_core::segment::Grain>> = Default::default();
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
                let mut o = pipeline::run(rec, &tiles, &params, &Ctl::NONE, &clock, &mut timings)?;
                if auto {
                    let tone = grainatlas_core::photometric::tone::auto(&o.linear, Some(&o.coverage), rec.um_per_px(), &params.tone);
                    pipeline::retone(rec, &mut o, &tone, &[]);
                }
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
                    let mut why: std::collections::BTreeMap<&str, usize> = Default::default();
                    for g in grains.iter().filter_map(|g| g.artifact) {
                        *why.entry(g.label()).or_default() += 1;
                    }
                    let flagged: usize = why.values().sum();
                    eprintln!(
                        "  {} grains, {flagged} artifacts{} in {:.0} ms",
                        grains.len() - flagged,
                        if flagged > 0 { format!(" ({})", why.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ")) } else { String::new() },
                        clock() - t
                    );
                    for (rel, bytes) in pipeline::mask_files(rec, &o, &grains) {
                        write(&out.join(rel), &bytes)?;
                    }
                    let csv = grainatlas_core::segment::grains_csv(&grains, |x, y| rec.stage_of(x, y), |_| 0);
                    write(&out.join(format!("exports/{}_grains.csv", area.id)), csv.as_bytes())?;
                    grains_n = Some(grains.iter().filter(|g| g.is_grain()).count());
                    // The UI keeps grains under `grainatlas_ui` in project.json.
                    let ui = project.extra.entry("grainatlas_ui".into()).or_insert_with(|| serde_json::json!({}));
                    ui["grains"][&area.id] = serde_json::to_value(&grains)?;
                    if sheets {
                        kept.push((area.id.clone(), o.linear.w, o.linear.h, o.base.clone()));
                        all_grains.insert(area.id.clone(), grains.clone());
                    } else if let Some(paper) = &print {
                        let t = clock();
                        let rec = project.areas.iter().find(|a| a.id == area.id).expect("area");
                        let img = pipeline::area_image(rec, &o, &rec.params.tone);
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
            if sheets {
                let t = clock();
                let paper = print.clone().unwrap_or_else(|| "tabloid".into());
                let own = grainatlas_core::ownership::assign(&project.areas, &all_grains, &Default::default());
                let name = pkg.info().name.clone();
                let reg_file = format!("{name}_print_registration.json");
                let sp = grainatlas_core::layout::sheets::SheetParams {
                    paper: paper.clone(),
                    um_per_mm: if scale > 0.0 { scale } else { 8.0 },
                    enhance: grainatlas_core::photometric::enhance::EnhanceParams {
                        denoise: !no_denoise,
                        sharpen: !no_sharpen,
                        strip: strip_bg.is_some(),
                        strip_grey: strip_bg.unwrap_or(22),
                        ..Default::default()
                    },
                    title: title.clone().unwrap_or_default(),
                    lab: lab.clone().unwrap_or_default(),
                    project: out.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
                    registration_file: reg_file.clone(),
                    printed: now_rfc3339().replace('T', " ").trim_end_matches('Z').chars().take(16).collect::<String>() + " UTC",
                    ..Default::default()
                };
                let areas: Vec<grainatlas_core::layout::sheets::SheetArea> = kept
                    .iter()
                    .filter_map(|(id, w, h, base)| {
                        let rec = project.areas.iter().find(|a| &a.id == id)?;
                        Some(grainatlas_core::layout::sheets::SheetArea {
                            rec,
                            img: grainatlas_core::layout::AreaImage {
                                id: &rec.id,
                                name: &rec.name,
                                sample: if rec.sample.is_empty() { &rec.name } else { &rec.sample },
                                w: *w,
                                h: *h,
                                px: base,
                                lut: grainatlas_core::photometric::tone::lut(&rec.params.tone),
                                um_per_px: rec.um_per_px(),
                            },
                        })
                    })
                    .collect();
                let meta = grainatlas_core::layout::sheets::sheet_meta(pkg.info());
                let r = grainatlas_core::layout::sheets::sheets(&sp, &areas, &all_grains, &own, &meta);
                let pdf = grainatlas_core::layout::pdf::pdf_from_svgs(&r.pages)?;
                write(&out.join(format!("exports/{name}_grain_sheets_{paper}.pdf")), &pdf)?;
                write(&out.join(format!("exports/{reg_file}")), &serde_json::to_vec_pretty(&r.registration)?)?;
                write(&out.join("exports/ownership.json"), &serde_json::to_vec_pretty(&own)?)?;
                let s = &r.stats;
                eprintln!(
                    "sheets: {} sheets, {} bands, {} grains printed ({} duplicates, {} artifacts, {} outside the size limits, background stripped in {} bands, kept in {} for safety; check: {} grains cut by a band edge, {} printed twice) in {:.0} ms",
                    s.sheets, s.bands, s.grains, s.duplicates, s.artifacts, s.filtered, s.stripped, s.strip_refused, s.grains_cut, s.grains_twice, clock() - t
                );
                let ui = project.extra.entry("grainatlas_ui".into()).or_insert_with(|| serde_json::json!({}));
                ui["ownership"] = serde_json::to_value(&own)?;
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
