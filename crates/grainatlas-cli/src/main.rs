//! `grainatlas` command line: batch work and tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

mod legacy;
mod import;
mod segment;
mod rerender;
mod project_atlas;
mod whole;
use grainatlas_core::Ctl;
use grainatlas_core::layout::sheets::{self, SheetArea, SheetMeta, SheetParams, Packing};
use grainatlas_core::project::pipeline::{self, Timings};
use grainatlas_core::project::session::Session;
use grainatlas_core::project::{AreaParams, Project};
use grainatlas_core::readers::{self, Package};
use grainatlas_core::source::DirSource;

#[derive(Parser)]
#[command(
    name = "grainatlas",
    version,
    about = "Stitch, correct, segment and lay out SEM grain-mount images"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Re-render print sheets from a saved project's mosaics and grains.
    Sheets {
        project: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// sheets (default) or whole (one continuous area, no segmentation required).
        #[arg(long, default_value = "sheets", value_parser = ["sheets", "grains", "whole"])]
        layout: String,
        #[arg(long)]
        area: Option<String>,
        #[arg(long, value_parser = ["auto", "landscape", "portrait"])]
        orientation: Option<String>,
        #[arg(long)]
        dpi: Option<f64>,
        #[arg(long)]
        no_grid: bool,
        /// Whole-area output stem (without directory or extension).
        #[arg(long)]
        name: Option<String>,
        /// SheetParams or WholeParams JSON; missing fields use defaults.
        #[arg(long)]
        params: Option<PathBuf>,
        /// Replay a PrintEdits JSON file.
        #[arg(long)]
        edits: Option<PathBuf>,
        /// tabloid (default), letter, a4 or a3.
        #[arg(long)]
        paper: Option<String>,
        /// Micrometres of sample per millimetre of paper, or "auto" (the default: from the
        /// mount's grain sizes, see docs/print_sheets.md "Layout rule: print scale").
        #[arg(long, value_parser = parse_scale)]
        scale: Option<f64>,
        /// Grain sheets: lines (default, mount order) or packed (fewer sheets).
        #[arg(long, value_name = "lines|packed", value_parser = parse_packing)]
        packing: Option<Packing>,
        /// Draw the project's saved spots with automatic paper-space labels.
        #[arg(long)]
        spots: bool,
        /// Label em size in paper millimetres; overrides the saved preference.
        #[arg(long, requires = "spots", value_parser = parse_label_size)]
        label_size: Option<f64>,
        /// Also write each sheet as SVG.
        #[arg(long)]
        svg: bool,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        lab: Option<String>,
    },
    /// Stitch every direct package folder, continuing after failures.
    Batch(legacy::BatchOptions),
    /// List a printout photo and initialise its area overlay.
    Overlay {
        project: PathBuf,
        #[arg(long)]
        area: String,
        #[arg(long)]
        image: PathBuf,
        #[arg(long, default_value = "")]
        note: String,
    },
    /// Stack saved mosaics with grain outlines for visual checking.
    Render {
        project: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u32).range(1..))]
        max_width: u32,
    },
    /// Check a saved project; warnings always exit successfully.
    Check {
        project: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Copy saved grains from another copy of this project (e.g. the one a print was made from)
    /// into areas without grains, when the mosaics are the same; spots are re-linked.
    ImportGrains {
        project: PathBuf,
        #[arg(long)]
        from: PathBuf,
        /// Areas to fill (each must have no grains yet).
        #[arg(long = "area")]
        areas: Vec<String>,
        /// Every area without grains.
        #[arg(long)]
        missing: bool,
        /// Check and report without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Segment saved areas with their saved placement, parameters and manual decisions.
    Segment {
        project: PathBuf,
        #[arg(long)]
        area: Vec<String>,
        /// Only areas with no saved grains entry (an empty entry counts as segmented).
        #[arg(long)]
        missing: bool,
        /// Append uncovered detections while preserving existing grains and spot links.
        #[arg(long, conflicts_with = "missing")]
        add_missing: bool,
        /// Compute and report without writing.
        #[arg(long)]
        dry_run: bool,
        #[arg(long, value_parser = ["auto", "cl", "bse"])]
        signal: Option<String>,
    },
    /// Re-run correction, blend and tone from saved placements without stitching or segmenting.
    Rerender {
        project: PathBuf,
        /// Areas to render; all saved areas when omitted.
        #[arg(long = "area")]
        areas: Vec<String>,
        #[arg(long, default_value = "keep", value_parser = ["keep", "on", "off"])]
        background: String,
        /// Compute and report without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Write the atlas bundle of a saved project (`<name>.grainatlas`) as a zip: areas rendered
    /// from their saved placements and tone, with the project's grains, spots and labels, as the
    /// app's Publish builds it. The project folder is only read.
    AtlasProject {
        project: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// UPbDB mount to record in atlas.json instead of the project's link.
        #[arg(long)]
        mount_id: Option<String>,
        /// 100 = lossless WebP tiles, 90 to 99 = near-lossless.
        #[arg(long, default_value_t = 100)]
        quality: u8,
        /// `exported` time in atlas.json (default now).
        #[arg(long)]
        exported: Option<String>,
    },
    /// Show the areas and tiles of a package without stitching.
    Info { folder: PathBuf },
    /// Print `<area> <sha256>` of the base mosaic of every area with default parameters, then
    /// `<area>+auto+grains <sha256>` after Auto tone and per-grain normalisation on the default
    /// segmentation (the native half of the wasm/native parity check).
    Hash { folder: PathBuf },
    /// Write the atlas bundle of a package (every area stitched with defaults, grains found,
    /// Auto tone): a `<project_id>.atlas/` folder in OUT, or a zip when OUT ends in `.zip`.
    Atlas(AtlasOptions),
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
    Stitch(StitchOptions),
}

#[derive(Args, Default)]
struct AtlasOptions {
    folder: PathBuf,
    #[arg(long)]
    out: PathBuf,
    /// Fixed project id (areas get the following ids); a new UUIDv7 otherwise.
    #[arg(long)]
    project_id: Option<String>,
    /// UPbDB mount the project is linked to.
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
}

#[derive(Args, Default)]
struct StitchOptions {
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
    /// Also print (implies --segment except with --print-style whole). Paper: tabloid (default), letter, a4, a3.
    /// Sheets (default): every owned grain of the package on landscape sheets at a fixed scale,
    /// exports/<package>_grain_sheets_<paper>.pdf with <package>_print_registration.json
    /// (frame print_sheet). With --print-style grains: one packed grain map per area,
    /// exports/<area>_grains.pdf.
    #[arg(long, value_name = "PAPER", num_args = 0..=1, default_missing_value = "tabloid")]
    print: Option<String>,
    /// sheets (default), grains, or whole (one area, no segmentation implied).
    #[arg(long, default_value = "sheets")]
    print_style: String,
    /// Print scale in um of sample per mm of paper (sheets: 0 = auto from the grain sizes; grains: 0 = fit).
    #[arg(long, default_value_t = 0.0)]
    scale: f64,
    /// Sheets: lines (default, mount order) or packed (fewer sheets).
    #[arg(long, value_name = "lines|packed", default_value = "lines", value_parser = parse_packing)]
    packing: Packing,
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
    /// Signal profile; auto reads each area's instrument metadata.
    #[arg(long, default_value = "auto", value_parser = ["auto", "cl", "bse"])]
    signal: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Sheets {
            project,
            out,
            params,
            edits,
            layout,
            area,
            orientation,
            dpi,
            no_grid,
            name,
            paper,
            scale,
            packing,
            spots,
            label_size,
            svg,
            title,
            lab,
        } => {
            if layout == "whole" {
                if edits.is_some() { bail!("--edits requires grain sheets"); }
                let mut p: grainatlas_core::layout::whole::WholeParams = match params {
                    Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
                    None => Default::default(),
                };
                if let Some(v) = paper {
                    p.paper = v;
                }
                if let Some(v) = scale {
                    p.um_per_mm = v;
                }
                if let Some(v) = area {
                    p.area_id = v;
                }
                if let Some(v) = orientation {
                    p.orientation = v;
                }
                if let Some(v) = dpi {
                    p.dpi = v;
                }
                if let Some(v) = name {
                    p.name = v;
                }
                if let Some(v) = title {
                    p.title = v;
                }
                if let Some(v) = lab {
                    p.lab = v;
                }
                if no_grid {
                    p.grid = false;
                }
                whole::render(&project, &out, p, svg, spots, label_size)?;
                return Ok(());
            }
            let mut p: SheetParams = match params {
                Some(path) => serde_json::from_str(&std::fs::read_to_string(&path)?)?,
                None => match &edits {
                    Some(path) => serde_json::from_slice::<sheets::PrintEdits>(&std::fs::read(path)?)?.params,
                    None => SheetParams::default(),
                },
            };
            if let Some(paper) = paper {
                p.paper = paper;
            }
            if let Some(scale) = scale {
                p.um_per_mm = scale;
            }
            if let Some(packing) = packing {
                p.packing = packing;
            }
            if let Some(title) = title {
                p.title = title;
            }
            if let Some(lab) = lab {
                p.lab = lab;
            }
            render_sheets(&project, &out, p, svg, edits.as_deref(), spots, label_size)?;
        }
        Cmd::Batch(options) => {
            let (report, failed) = legacy::batch(options)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if failed {
                bail!("one or more packages failed; see JSON report");
            }
        }
        Cmd::Overlay {
            project,
            area,
            image,
            note,
        } => legacy::overlay(&project, &area, &image, &note)?,
        Cmd::Render {
            project,
            out,
            max_width,
        } => legacy::render(&project, &out, max_width)?,
        Cmd::ImportGrains { project, from, areas, missing, dry_run } => import::run(&project, &from, &areas, missing, dry_run)?,
        Cmd::Segment { project, area, missing, add_missing, dry_run, signal } => segment::run(&project, &area, missing, add_missing, dry_run, signal.as_deref())?,
        Cmd::Rerender { project, areas, background, dry_run } => rerender::run(&project, &areas, &background, dry_run)?,
        Cmd::AtlasProject { project, out, mount_id, quality, exported } => {
            println!("{}", serde_json::to_string_pretty(&project_atlas::run(&project, &out, mount_id.as_deref(), quality, exported)?)?);
        }
        Cmd::Check { project, json } => {
            let path = if project.is_dir() {
                project.join("project.json")
            } else {
                project
            };
            let input = std::fs::read_to_string(&path)?;
            let findings = grainatlas_core::checks::check_json(&input, "null")?;
            if json {
                println!("{findings}");
            } else {
                let rows: Vec<grainatlas_core::checks::Finding> = serde_json::from_str(&findings)?;
                for f in &rows {
                    println!("warning [{}] {}", f.code, f.message);
                }
                println!("{} warnings", rows.len());
            }
        }
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
                let o = pipeline::run(
                    rec,
                    &tiles,
                    &AreaParams::default(),
                    &Ctl::NONE,
                    &|| 0.0,
                    &mut Timings::default(),
                )?;
                println!("{} {}", area.id, o.mosaic_json.sha256);
            }
            // The same steps as ui/scripts/wasm-hash.mjs, through the Session both front ends use.
            let mut s = Session::new("hash");
            s.open_source(
                Arc::new(DirSource::new(&folder)?),
                "hash",
                "1970-01-01T00:00:00Z",
            )?;
            s.pin_identity(PARITY_ID, EPOCH)?;
            let mut grains = serde_json::Map::new();
            for area in &pkg.info().areas {
                if only.as_ref().is_some_and(|o| o != &area.id) {
                    continue;
                }
                let (hash, mut g) = auto_grains(&mut s, &area.id)?;
                println!("{}+auto+grains {hash}", area.id);
                pin_grain_ids(
                    &mut g,
                    PARITY_ID.parse()?,
                    grains
                        .values()
                        .map(|v| v.as_array().map_or(0, |a| a.len()))
                        .sum(),
                );
                grains.insert(area.id.clone(), g);
            }
            // The atlas bundle of that state (ui/scripts/wasm-hash.mjs does the same).
            let full = full_project(&s, serde_json::Value::Object(grains), serde_json::json!([]))?;
            let opts = serde_json::json!({ "quality": 100, "exported": EPOCH }).to_string();
            let files = s.atlas_files(&full, &opts)?;
            println!("bundle {}", grainatlas_core::util::sha256_hex(&files[0].1));
            println!(
                "bundle.zip {}",
                grainatlas_core::util::sha256_hex(&grainatlas_core::atlas::zip(PARITY_ID, &files)?)
            );
        }
        Cmd::Atlas(options) => {
            println!("{}", atlas(options)?);
        }
        Cmd::Photometry {
            folder,
            out,
            variants,
            scale,
            area,
        } => {
            let (pkg, only, _) = open(&folder)?;
            let only = area.or(only);
            let mut sets: Vec<(String, AreaParams)> = Vec::new();
            for v in if variants.is_empty() {
                vec!["default".to_string()]
            } else {
                variants
            } {
                let (name, file) = v.split_once('=').unwrap_or((v.as_str(), ""));
                let p = if file.is_empty() {
                    AreaParams::default()
                } else {
                    serde_json::from_str(&std::fs::read_to_string(file)?)?
                };
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
                let placement = grainatlas_core::mosaic::stitch_with_failed(
                    &inputs.meta,
                    &inputs.imgs,
                    &inputs.failed,
                    &sets[0].1.stitch,
                    &Ctl::NONE,
                )?;
                write(
                    &out.join(format!("{}_placement.json", a.id)),
                    serde_json::to_string(&placement)?.as_bytes(),
                )?;
                let (raw_px, raw_blk) =
                    grainatlas_core::photometric::overlap_metrics(&placement, &inputs.imgs, 32);
                let mut row = serde_json::json!({ "area": a.id, "tiles": inputs.imgs.len(), "raw_px": raw_px * 255.0, "raw_block": raw_blk * 255.0 });
                for (name, p) in &sets {
                    let t = Instant::now();
                    let (corrected, photo) = grainatlas_core::photometric::correct(
                        &placement,
                        &inputs.imgs,
                        &p.photometric,
                    );
                    let (px, blk) =
                        grainatlas_core::photometric::overlap_metrics(&placement, &corrected, 32);
                    let ms = t.elapsed().as_secs_f64() * 1000.0;
                    let fill = if p.photometric.background {
                        photo.background_median
                    } else {
                        0.0
                    };
                    let (linear, cov) = grainatlas_core::blend::blend(
                        &placement,
                        corrected,
                        &p.blend,
                        fill,
                        &Ctl::NONE,
                    )?;
                    let tone =
                        grainatlas_core::photometric::tone::auto(&linear, Some(&cov), 1.0, &p.tone);
                    let flatness =
                        grainatlas_core::photometric::joint::background_flatness(&linear, &cov, 32);
                    let small = linear.downsample(scale.max(1));
                    let scov: Vec<f32> = grainatlas_core::image::Gray {
                        w: linear.w,
                        h: linear.h,
                        px: cov,
                    }
                    .downsample(scale.max(1))
                    .px;
                    let base = pipeline::tone_base(&small, &scov, &tone);
                    write(
                        &out.join(format!("{}_{name}.png", a.id)),
                        &grainatlas_core::image::encode_png_gray(small.w, small.h, &base),
                    )?;
                    // The same stretch without CLAHE: what the correction did, before local contrast.
                    let flat = grainatlas_core::photometric::ToneParams {
                        clahe: false,
                        ..tone.clone()
                    };
                    let base = pipeline::tone_base(&small, &scov, &flat);
                    write(
                        &out.join(format!("{}_{name}_stretch.png", a.id)),
                        &grainatlas_core::image::encode_png_gray(small.w, small.h, &base),
                    )?;
                    row[name] = serde_json::json!({ "px": px * 255.0, "block": blk * 255.0, "app_before": photo.overlap_rms_before * 255.0, "app_after": photo.overlap_rms_after * 255.0, "flat": flatness * 255.0, "ms": ms });
                }
                eprintln!("{row}");
                rows.push(row);
            }
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        Cmd::Stitch(options) => {
            println!("{}", serde_json::to_string_pretty(&stitch(options)?)?);
        }
    }
    Ok(())
}

fn atlas(options: AtlasOptions) -> Result<String> {
    let AtlasOptions {
        folder,
        out,
        project_id,
        mount_id,
        quality,
        demo_spots,
        exported,
    } = options;
    let (pkg, only, root) = open(&folder)?;
    let mut s = Session::new("atlas");
    s.open_source(
        Arc::new(DirSource::new(&root)?),
        &abs(&root),
        &now_rfc3339(),
    )?;
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
            pin_grain_ids(
                &mut g,
                base,
                grains
                    .values()
                    .map(|v| v.as_array().map_or(0, |a| a.len()))
                    .sum(),
            );
        }
        let gs: Vec<grainatlas_core::segment::Grain> = serde_json::from_value(g.clone())?;
        let project = s.project.as_ref().expect("project");
        let rec = project
            .areas
            .iter()
            .find(|a| a.id == area.id)
            .expect("area");
        for g in gs.iter().take(demo_spots.saturating_sub(spots.len())) {
            let mut sp = grainatlas_core::spots::new_spot(
                &rec.id,
                &rec.sample,
                &gs,
                &spots,
                &project.settings,
                g.centroid_um[0],
                g.centroid_um[1],
                25.0,
                EPOCH,
            );
            sp.id = uuid_after(project.uuid, 0x100 + spots.len() as u128);
            spots.push(sp);
        }
        grains.insert(area.id.clone(), g);
    }
    let full = full_project(
        &s,
        serde_json::Value::Object(grains),
        serde_json::to_value(&spots)?,
    )?;
    let opts =
        serde_json::json!({ "quality": quality, "exported": exported.unwrap_or_else(now_rfc3339) })
            .to_string();
    let files = s.atlas_files(&full, &opts)?;
    let id = s.project.as_ref().expect("project").uuid.to_string();
    if out
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("zip"))
    {
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
    Ok(id)
}

fn stitch(options: StitchOptions) -> Result<serde_json::Value> {
    let StitchOptions {
        folder,
        out,
        params,
        fast,
        no_pyramid,
        png,
        segment,
        print,
        print_style,
        scale,
        packing,
        no_denoise,
        no_sharpen,
        strip_bg,
        title,
        lab,
        sample,
        auto,
        polarity,
        signal,
    } = options;
    let sheets = print.is_some() && print_style == "sheets";
    let whole_print = print.is_some() && print_style == "whole";
    if !matches!(print_style.as_str(), "sheets" | "grains" | "whole") {
        bail!("--print-style {print_style}: sheets, grains or whole");
    }
    let (pkg, only, root) = open(&folder)?;
    if whole_print && only.is_none() && pkg.info().areas.len() != 1 {
        bail!(
            "whole-area printing requires one area; pass an area folder or use sheets --layout whole --area after stitching"
        );
    }
    let mut params: AreaParams = match params {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(&p)?)?,
        None => AreaParams::default(),
    };
    params.segment.signal = segment::signal_override(&signal);
    if fast {
        params.blend.mode = grainatlas_core::blend::BlendMode::Feather;
    }
    if let Some(pol) = &polarity {
        params.segment.polarity = serde_json::from_value(serde_json::json!(pol))
            .with_context(|| format!("--polarity {pol}: bright, dark or both"))?;
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
    let mut all_grains: std::collections::BTreeMap<String, Vec<grainatlas_core::segment::Grain>> =
        Default::default();
    for (ai, area) in pkg.info().areas.iter().enumerate() {
        if only.as_ref().is_some_and(|o| o != &area.id) {
            continue;
        }
        if project.areas.iter().all(|a| a.id != area.id) {
            project
                .areas
                .push(grainatlas_core::project::AreaRecord::from_meta(area));
        }
        let rec = project.area_mut(&area.id).expect("area");
        if let Some(s) = &sample {
            rec.sample = s.clone();
        }
        let mut timings = Timings::default();
        let t0 = clock();
        let tiles = pipeline::read_tiles(pkg.as_ref(), ai, rec)?;
        timings.steps.push(("read".into(), clock() - t0));
        let tile_count = tiles.len();
        let mut o = pipeline::run_owned(rec, tiles, &params, &[], &Ctl::NONE, &clock, &mut timings)?;
        if auto {
            let tone = grainatlas_core::photometric::tone::auto(
                &o.linear,
                Some(&o.coverage),
                rec.um_per_px(),
                &params.tone,
            );
            pipeline::retone(rec, &mut o, &tone, &[]);
        }
        let total: f64 = timings.steps.iter().map(|s| s.1).sum();
        let pairs_used = o.placement.pairs.iter().filter(|p| p.used).count();
        eprintln!(
            "{}: {} tiles, {}x{} px, pairs {}/{} measured, rms {:.3} px, max {:.3} px, {:.0} ms ({})",
            area.id,
            tile_count,
            o.linear.w,
            o.linear.h,
            pairs_used,
            o.placement.pairs.len(),
            o.placement.rms_residual,
            o.placement.max_residual,
            total,
            timings
                .steps
                .iter()
                .map(|(n, t)| format!("{n} {t:.0}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        summary.push(serde_json::json!({
            "area": area.id, "failed_tiles": rec.failed_tiles, "tiles": tile_count, "w": o.linear.w, "h": o.linear.h,
            "pairs": o.placement.pairs.len(), "pairs_measured": pairs_used,
            "rms_px": o.placement.rms_residual, "max_px": o.placement.max_residual,
            "ms": total, "steps": timings.steps, "sha256": o.mosaic_json.sha256,
        }));
        let mut grains_n = None;
        if segment || (print.is_some() && !whole_print) {
            let t = clock();
            let outline = (&project.settings).into();
            let rec = project.area_mut(&area.id).expect("area");
            let prev: Vec<grainatlas_core::segment::Grain> =
                std::fs::read(out.join(format!("masks/{}/grains.json", area.id)))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|v| serde_json::from_value(v["grains"].clone()).ok())
                    .unwrap_or_default();
            let grains = pipeline::segment_area(rec, &o, &params.segment, &prev, outline);
            let work = pipeline::work_image(&o, rec, params.segment.work_scale);
            let resolved = params.segment.resolved_signal(&work);
            let report = segment::report(
                &area.id,
                &grains,
                resolved,
                if signal == "auto" { "metadata" } else { "flag" },
                grainatlas_core::segment::grain_mask(&work, &params.segment).1,
            );
            let row = summary.last_mut().expect("summary");
            for key in ["artifacts", "signal", "signal_source", "threshold", "warning"] {
                row[key] = report[key].clone();
            }
            eprintln!("  segmentation in {:.0} ms", clock() - t);
            for (rel, bytes) in pipeline::mask_files(rec, &o, &grains) {
                write(&out.join(rel), &bytes)?;
            }
            let csv =
                grainatlas_core::segment::grains_csv(&grains, |x, y| rec.stage_of(x, y), |_| 0);
            write(
                &out.join(format!("exports/{}_grains.csv", area.id)),
                csv.as_bytes(),
            )?;
            grains_n = Some(grains.iter().filter(|g| g.is_grain()).count());
            // The UI keeps grains under `grainatlas_ui` in project.json.
            let ui = project
                .extra
                .entry("grainatlas_ui".into())
                .or_insert_with(|| serde_json::json!({}));
            ui["grains"][&area.id] = serde_json::to_value(&grains)?;
            if sheets {
                kept.push((
                    area.id.clone(),
                    o.linear.w,
                    o.linear.h,
                    o.base.as_ref().clone(),
                ));
                all_grains.insert(area.id.clone(), grains.clone());
            } else if let Some(paper) = print.as_ref().filter(|_| !whole_print) {
                let t = clock();
                let rec = project
                    .areas
                    .iter()
                    .find(|a| a.id == area.id)
                    .expect("area");
                let img = pipeline::area_image(rec, &o, &rec.params.tone);
                let mut gmap = std::collections::BTreeMap::new();
                gmap.insert(rec.id.clone(), grains.clone());
                let lp = grainatlas_core::layout::LayoutParams {
                    paper: paper.clone(),
                    items: grainatlas_core::layout::Items::Grains,
                    um_per_mm: scale,
                    title: format!("{} · {}", pkg.info().name, rec.name),
                    ..Default::default()
                };
                let r = grainatlas_core::layout::layout(&lp, &[img], &gmap, &[], &[]);
                for (i, svg) in r.pages.iter().enumerate() {
                    write(
                        &out.join(format!("exports/{}_page{}.svg", area.id, i + 1)),
                        svg.as_bytes(),
                    )?;
                }
                let pdf = grainatlas_core::layout::pdf::pdf_from_svgs(&r.pages)?;
                write(&out.join(format!("exports/{}_grains.pdf", area.id)), &pdf)?;
                eprintln!(
                    "  {} pages at {} um/mm in {:.0} ms",
                    r.pages.len(),
                    r.um_per_mm,
                    clock() - t
                );
            }
        }
        if whole_print {
            let rec = project
                .areas
                .iter()
                .find(|a| a.id == area.id)
                .expect("area");
            let p = grainatlas_core::layout::whole::WholeParams {
                paper: print.clone().expect("paper"),
                um_per_mm: scale,
                title: title.clone().unwrap_or_default(),
                lab: lab.clone().unwrap_or_default(),
                printed: now_rfc3339(),
                project: out.file_name().unwrap_or_default().to_string_lossy().into(),
                ..Default::default()
            };
            let mut info = pkg.info().clone();
            info.areas.retain(|a| a.id == rec.id);
            whole::export(
                &out.join("exports"),
                &out,
                &project,
                &p,
                &SheetArea {
                    rec,
                    img: pipeline::area_image(rec, &o, &rec.params.tone),
                },
                &sheets::sheet_meta(&info),
                false,
            )?;
        }
        summary.last_mut().expect("summary")["grains"] = serde_json::json!(grains_n);
        let rec = project
            .areas
            .iter()
            .find(|a| a.id == area.id)
            .expect("area")
            .clone();
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
        let own =
            grainatlas_core::ownership::assign(&project.areas, &all_grains, &Default::default());
        let name = pkg.info().name.clone();
        let reg_file = format!("{name}_print_registration.json");
        let sp = grainatlas_core::layout::sheets::SheetParams {
            paper: paper.clone(),
            packing,
            um_per_mm: if scale > 0.0 { scale } else { 0.0 },
            enhance: grainatlas_core::photometric::enhance::EnhanceParams {
                denoise: !no_denoise,
                sharpen: !no_sharpen,
                strip: strip_bg.is_some(),
                strip_grey: strip_bg.unwrap_or(22),
                ..Default::default()
            },
            title: title.clone().unwrap_or_default(),
            lab: lab.clone().unwrap_or_default(),
            project: out
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default(),
            registration_file: reg_file.clone(),
            printed: now_rfc3339()
                .replace('T', " ")
                .trim_end_matches('Z')
                .chars()
                .take(16)
                .collect::<String>()
                + " UTC",
            ..Default::default()
        };
        let areas: Vec<grainatlas_core::layout::sheets::SheetArea> = kept
            .iter()
            .filter_map(|(id, w, h, base)| {
                let rec = project.areas.iter().find(|a| &a.id == id)?;
                Some(grainatlas_core::layout::sheets::SheetArea {
                    rec,
                    img: grainatlas_core::layout::AreaImage { source_size_um: None, edits: None,
                        id: &rec.id,
                        name: &rec.name,
                        sample: if rec.sample.is_empty() {
                            &rec.name
                        } else {
                            &rec.sample
                        },
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
        let mut r = grainatlas_core::layout::sheets::sheets(&sp, &areas, &all_grains, &own, &meta);
        r.registration.project_uuid = Some(project.uuid);
        r.registration.project_path = abs(&out);
        let pdf = grainatlas_core::layout::pdf::pdf_from_svgs(&r.pages)?;
        write(
            &out.join(format!("exports/{name}_grain_sheets_{paper}.pdf")),
            &pdf,
        )?;
        write(
            &out.join(format!("exports/{reg_file}")),
            &serde_json::to_vec_pretty(&r.registration)?,
        )?;
        write(
            &out.join("exports/ownership.json"),
            &serde_json::to_vec_pretty(&own)?,
        )?;
        let s = &r.stats;
        let line_fill = if sp.packing == Packing::Lines { format!(", {:.1}% line width", s.fill.line_width * 100.0) } else { String::new() };
        eprintln!(
            "sheets: {} sheets, {} bands, {} grains printed ({} duplicates, {} artifacts, {} outside the size limits, {} strays left off; fill: {:.1}% band area{line_fill}; background stripped in {} bands, kept in {} for safety; check: {} grains cut by a band edge, {} printed twice, {} unmarked, {} bands outside body, {} overlapping image/block pairs) in {:.0} ms",
            s.sheets,
            s.bands,
            s.grains,
            s.duplicates,
            s.artifacts,
            s.filtered,
            s.strays,
            s.fill.band_area * 100.0,
            s.stripped,
            s.strip_refused,
            s.grains_cut,
            s.grains_twice,
            s.grains_unmarked,
            s.bands_outside_body,
            s.bands_overlapping,
            clock() - t
        );
        let ui = project
            .extra
            .entry("grainatlas_ui".into())
            .or_insert_with(|| serde_json::json!({}));
        ui["ownership"] = serde_json::to_value(&own)?;
    }
    write(&pj, project.to_json().as_bytes())?;
    Ok(serde_json::to_value(summary)?)
}

/// Only reads the saved project; exports always go outside its folder.
fn render_sheets(folder: &Path, out: &Path, mut p: SheetParams, svg: bool, edits: Option<&Path>, spots: bool, label_size: Option<f64>) -> Result<()> {
    let folder = folder.canonicalize()?;
    let out = resolved_path(out)?;
    if out.starts_with(&folder) {
        bail!("--out must be outside the project folder");
    }
    if !matches!(p.paper.as_str(), "tabloid" | "letter" | "a4" | "a3") {
        bail!("paper {}: tabloid, letter, a4 or a3", p.paper);
    }
    if !p.um_per_mm.is_finite() || p.um_per_mm < 0.0 {
        bail!("scale must be positive and finite, or auto");
    }
    let project = Project::from_json(&std::fs::read_to_string(folder.join("project.json"))?)?;
    let start = Instant::now();
    let mut grains =
        std::collections::BTreeMap::<String, Vec<grainatlas_core::segment::Grain>>::new();
    let mut kept = Vec::new();
    for rec in &project.areas {
        let Some(saved) = project
            .extra
            .get("grainatlas_ui")
            .and_then(|ui| ui["grains"].get(&rec.id))
        else {
            eprintln!("{}: skipped (no saved grains)", rec.id);
            continue;
        };
        let gs: Vec<grainatlas_core::segment::Grain> = serde_json::from_value(saved.clone())
            .with_context(|| format!("{}: saved grains", rec.id))?;
        if gs.is_empty() {
            eprintln!("{}: skipped (no saved grains)", rec.id);
            continue;
        }
        let dir = folder.join("mosaic").join(&rec.id);
        if !dir.join("mosaic.json").exists() {
            eprintln!("{}: skipped (no saved mosaic)", rec.id);
            continue;
        }
        let (m, px) = saved_mosaic(&dir).with_context(|| format!("{}: saved mosaic", rec.id))?;
        kept.push((rec, m, px));
        grains.insert(rec.id.clone(), gs);
    }
    if kept.is_empty() {
        bail!("no saved area with mosaics and grains to print");
    }
    let own = grainatlas_core::ownership::assign(&project.areas, &grains, &Default::default());
    let meta = match open(Path::new(&project.package.path)) {
        Ok((pkg, _, _)) => sheets::sheet_meta(pkg.info()),
        Err(_) => SheetMeta {
            package: project.package.name.clone(),
            areas: project.areas.len(),
            tiles: project.areas.iter().map(|a| a.tiles.len()).sum(),
            um_per_px: project.areas.first().map_or(0.0, |a| a.um_per_px()),
            ..Default::default()
        },
    };
    // A project may preserve the intended filename case while the native ZAG
    // uses lowercase (Sandbox_S.grainatlas versus sandbox_s.zag). Do not
    // otherwise rename the package or its exported files.
    let name = folder.file_stem().and_then(|s| s.to_str())
        .filter(|s| s.eq_ignore_ascii_case(&project.package.name))
        .unwrap_or(&project.package.name);
    p.project = folder
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    if p.registration_file.is_empty() {
        p.registration_file = format!("{name}_print_registration.json");
    }
    p.printed = now_rfc3339()
        .replace('T', " ")
        .trim_end_matches('Z')
        .chars()
        .take(16)
        .collect::<String>()
        + " UTC";
    let areas: Vec<SheetArea> = kept
        .iter()
        .map(|(rec, m, px)| SheetArea {
            rec,
            img: grainatlas_core::layout::AreaImage { source_size_um: Some([m.w as f64*m.um_per_px,m.h as f64*m.um_per_px]), edits: project.image_edits.as_ref().filter(|e| e.version==1).and_then(|e| e.areas.get(&rec.id)),
                id: &rec.id,
                name: &rec.name,
                sample: if rec.sample.is_empty() {
                    &rec.name
                } else {
                    &rec.sample
                },
                w: m.w,
                h: m.h,
                px,
                lut: std::array::from_fn(|i| i as u8),
                um_per_px: m.um_per_px,
            },
        })
        .collect();
    let source_grains = &grains;
    let grains = areas.iter().map(|a| {
        let gs=grainatlas_core::project::printable_grains(a.rec,grains.get(&a.rec.id).map_or(&[],Vec::as_slice),a.img.edits);
        (a.rec.id.clone(),grainatlas_core::project::image_edits::oriented_grains(&gs,a.img.image_transform()))
    }).collect();
    let plan = sheets::plan_sheets(&p, &areas, &grains, &own, &meta);
    let mut edited = if let Some(file) = edits {
        let stored: sheets::PrintEdits = serde_json::from_slice(&std::fs::read(file).with_context(|| file.display().to_string())?)?;
        sheets::replay(&plan, &stored, &areas, &grains)
    } else { sheets::apply_edits(&plan, &[], &areas, &grains) };
    for dropped in &edited.dropped { eprintln!("edit {} dropped: {}", dropped.index, dropped.reason); }
    if !edited.problems.is_empty() { bail!("cannot export sheets: {}", edited.problems.iter().map(|p| p.message.as_str()).collect::<Vec<_>>().join("; ")); }
    if spots {
        let (spots, labels) = print_spots(&project, label_size)?;
        edited.plan.print_spots = Some(spots);
        edited.plan.print_labels = Some(labels);
    }
    let mut r = sheets::render_plan(&edited.plan, &areas, &grains, &own);
    for area in &areas {
        for g in source_grains.get(&area.rec.id).into_iter().flatten().filter(|g| area.img.edits.is_some_and(|e| !e.visible(g.centroid_um))) {
            let off = sheets::LeftOff { grain: sheets::StrayGrain { area: area.rec.id.clone(), id: g.id, number: g.number, label: g.label.clone(), eq_diameter_um: g.eq_diameter_um, centroid_um: g.centroid_um, bbox_um: g.bbox_um }, reason: "cropped".into() };
            r.left_off.push(off.clone()); r.registration.left_off.push(off);
            r.left_off_summary.cropped += 1;
        }
    }
    r.registration.project_uuid = Some(project.uuid);
    r.registration.project_path = abs(&folder);
    if areas.iter().any(|a| a.img.edits.is_some()) { r.registration.mapping = "mount_um = band.paper_to_mount × [sheet_x_mm, sheet_y_mm, 1]; SVG affine order [a,b,c,d,e,f]".into(); }
    let mut files = vec![
        (
            format!("{name}_grain_sheets_{}.pdf", p.paper),
            whole::registered_pdf(&r.pages, &r.registration)?,
        ),
        (
            p.registration_file.clone(),
            serde_json::to_vec_pretty(&r.registration)?,
        ),
        (
            format!("{name}_sheet_stats.json"),
            serde_json::to_vec_pretty(&r.stats)?,
        ),
    ];
    if svg {
        files.extend(r.pages.iter().enumerate().map(|(i, page)| {
            (
                format!("{name}_sheet{}.svg", i + 1),
                page.as_bytes().to_vec(),
            )
        }));
    }
    // Check all destinations first, including existing symlinks and names from project.json.
    for (file, _) in &files {
        if Path::new(file)
            .file_name()
            .is_none_or(|n| Path::new(n) != Path::new(file))
            || resolved_path(&out.join(file))?.parent() != Some(out.as_path())
        {
            bail!("export must be a file outside the project folder: {file}");
        }
    }
    for (file, bytes) in files {
        write(&out.join(file), &bytes)?;
    }
    let s = &r.stats;
    let line_fill = if edited.plan.lines.is_some() { format!(", {:.1}% line width", s.fill.line_width * 100.0) } else { String::new() };
    eprintln!(
        "sheets: {} sheets, {} bands, {} grains printed ({} duplicates, {} artifacts, {} outside the size limits, {} strays left off; fill: {:.1}% band area{line_fill}; background stripped in {} bands, kept in {} for safety; check: {} grains cut by a band edge, {} printed twice, {} unmarked, {} bands outside body, {} overlapping image/block pairs) in {:.0} ms",
        s.sheets,
        s.bands,
        s.grains,
        s.duplicates,
        s.artifacts,
        s.filtered,
        s.strays,
        s.fill.band_area * 100.0,
        s.stripped,
        s.strip_refused,
        s.grains_cut,
        s.grains_twice,
        s.grains_unmarked,
        s.bands_outside_body,
        s.bands_overlapping,
        start.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}

/// Read preferences and exact stored spot records; no project mutation or sync.
fn print_spots(project: &Project, size: Option<f64>) -> Result<(Vec<grainatlas_core::spots::Spot>, grainatlas_core::layout::labels::Options)> {
    let ui = project.extra.get("grainatlas_ui");
    let spots = ui.and_then(|u| u.get("spots")).map(|v| serde_json::from_value(v.clone())).transpose().context("saved spots")?.unwrap_or_default();
    let pins = ui.and_then(|u| u.get("print_label_pins")).map(|v| serde_json::from_value(v.clone())).transpose().context("saved print label pins")?.unwrap_or_default();
    let size_mm = size.or_else(|| ui.and_then(|u| u.get("print_label_size_mm")).and_then(|v| v.as_f64())).unwrap_or_default();
    Ok((spots, grainatlas_core::layout::labels::Options { size_mm, pins }))
}
fn parse_label_size(s: &str) -> std::result::Result<f64, String> {
    match s.parse::<f64>() {
        Ok(v) if v.is_finite() && (1.6..=12.0).contains(&v) => Ok(v),
        _ => Err("label size must be between 1.6 and 12 mm".into()),
    }
}

/// Assemble level 0, including the smaller tiles at the right and bottom edges.
fn saved_mosaic(dir: &Path) -> Result<(grainatlas_core::mosaic::pyramid::MosaicJson, Vec<u8>)> {
    let m: grainatlas_core::mosaic::pyramid::MosaicJson =
        serde_json::from_slice(&std::fs::read(dir.join("mosaic.json"))?)?;
    if m.w == 0 || m.h == 0 || m.tile_size == 0 || m.format != "png" {
        bail!("invalid mosaic dimensions, tile size or format");
    }
    let mut px = vec![0; m.w.checked_mul(m.h).context("mosaic dimensions overflow")?];
    for row in 0..m.h.div_ceil(m.tile_size) {
        for col in 0..m.w.div_ceil(m.tile_size) {
            let path = dir.join(format!("0/{col}_{row}.png"));
            let (tile, _) = grainatlas_core::image::decode(
                &std::fs::read(&path).with_context(|| path.display().to_string())?,
            )?;
            let (x, y) = (col * m.tile_size, row * m.tile_size);
            if tile.w != m.tile_size.min(m.w - x) || tile.h != m.tile_size.min(m.h - y) {
                bail!("{}: unexpected tile dimensions", path.display());
            }
            let bytes = tile.to_u8();
            for r in 0..tile.h {
                px[(y + r) * m.w + x..(y + r) * m.w + x + tile.w]
                    .copy_from_slice(&bytes[r * tile.w..(r + 1) * tile.w]);
            }
        }
    }
    Ok((m, px))
}

/// Resolve symlinks in existing components even when the output does not exist yet.
fn resolved_path(path: &Path) -> Result<PathBuf> {
    let mut out = std::env::current_dir()?;
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            _ => {
                out.push(part);
                match out.canonicalize() {
                    Ok(path) => out = path,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::NotFound
                            && std::fs::symlink_metadata(&out).is_err() => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    Ok(out)
}

/// Opens a package; if `folder` has no `.zag` and is one area of a package, opens
/// the parent and returns the area id to restrict to. Also returns the package root.
fn open(folder: &Path) -> Result<(Box<dyn Package>, Option<String>, PathBuf)> {
    let folder = folder
        .canonicalize()
        .with_context(|| format!("{}", folder.display()))?;
    if std::fs::read_dir(&folder)?.flatten().any(|e| {
        e.path()
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("zag"))
    }) {
        let pkg = readers::open(Arc::new(DirSource::new(&folder)?))?;
        return Ok((pkg, None, folder));
    }
    if let Some(parent) = folder.parent() {
        let has_zag = std::fs::read_dir(parent)?.flatten().any(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("zag"))
        });
        if has_zag {
            let src = DirSource::new(parent)?;
            let pkg = readers::open(Arc::new(src))?;
            let id = folder
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
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
fn full_project(
    s: &Session,
    grains: serde_json::Value,
    spots: serde_json::Value,
) -> Result<String> {
    let mut v: serde_json::Value = serde_json::from_str(&s.project_json()?)?;
    v["grainatlas_ui"] = serde_json::json!({ "grains": grains, "spots": spots, "labels": [] });
    Ok(v.to_string())
}

/// Stitch with defaults, find grains, apply Auto and per-grain normalisation (strength 0.8, area
/// reference); returns the base mosaic's sha256 and the grains (JSON).
fn auto_grains(s: &mut Session, id: &str) -> Result<(String, serde_json::Value)> {
    let defaults = serde_json::to_string(&AreaParams::default())?;
    s.stitch(id, &defaults, &Ctl::NONE, &|| 0.0)?;
    let outline = serde_json::to_string(&grainatlas_core::segment::OutlinePad::from(
        &s.project.as_ref().expect("project").settings,
    ))?;
    let grains = s.segment(
        id,
        &serde_json::to_string(&grainatlas_core::segment::SegmentParams::default())?,
        "",
        &outline,
    )?;
    s.set_grains(id, &grains)?;
    let mut p: AreaParams = serde_json::from_str(&s.auto_params(id, &defaults)?)?;
    p.tone.grain_norm.enabled = true;
    let r: serde_json::Value = serde_json::from_str(&s.recompute(
        id,
        &serde_json::to_string(&p)?,
        &Ctl::NONE,
        &|| 0.0,
    )?)?;
    Ok((
        r["mosaic"]["sha256"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        serde_json::from_str(&grains)?,
    ))
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, bytes).with_context(|| path.display().to_string())
}

fn abs(p: &Path) -> String {
    p.canonicalize()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| p.display().to_string())
}

/// UTC timestamp, RFC 3339, from the system clock (civil-from-days, H. Hinnant).
fn now_rfc3339() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
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
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// `--scale`: micrometres per millimetre, or "auto" (0).
fn parse_scale(s: &str) -> std::result::Result<f64, String> {
    if s.trim().eq_ignore_ascii_case("auto") {
        return Ok(0.0);
    }
    match s.trim().parse::<f64>() {
        Ok(v) if v.is_finite() && v > 0.0 => Ok(v),
        _ => Err(format!("{s:?}: a positive number of um per mm, or auto")),
    }
}

fn parse_packing(s: &str) -> std::result::Result<Packing, String> {
    match s {
        "lines" => Ok(Packing::Lines),
        "packed" => Ok(Packing::Packed),
        _ => Err(format!("{s:?}: expected lines or packed")),
    }
}

#[cfg(test)]
mod packing_tests {
    use super::*;

    #[test]
    fn sheets_parse_spots_and_size() {
        for layout in ["grains", "whole"] {
            let cli=Cli::try_parse_from(["grainatlas","sheets","mount-a.grainatlas","--out","print-a","--spots","--label-size","3","--svg","--layout",layout]).unwrap();
            assert!(matches!(cli.cmd,Cmd::Sheets{spots:true,label_size:Some(3.0),svg:true,..}));
        }
        assert!(Cli::try_parse_from(["grainatlas","sheets","mount-a.grainatlas","--out","print-a","--label-size","3"]).is_err());
        for size in ["NaN","0","13"] {
            assert!(Cli::try_parse_from(["grainatlas","sheets","mount-a.grainatlas","--out","print-a","--spots","--label-size",size]).is_err());
        }
    }
    #[test]
    fn sheets_and_stitch_parse_packing_and_reject_unknown_modes() {
        for mode in ["lines", "packed"] {
            let expected = parse_packing(mode).unwrap();
            let cli = Cli::try_parse_from(["grainatlas", "sheets", "mount-a.grainatlas", "--out", "print-a", "--packing", mode]).unwrap();
            let Cmd::Sheets { packing, .. } = cli.cmd else { panic!("sheets command") };
            let mut params: SheetParams = serde_json::from_str(r#"{"packing":"lines"}"#).unwrap();
            if let Some(packing) = packing { params.packing = packing; }
            assert_eq!(params.packing, expected, "flags override JSON");
            let cli = Cli::try_parse_from(["grainatlas", "stitch", "mount-a", "--out", "mount-a.grainatlas", "--packing", mode]).unwrap();
            let Cmd::Stitch(options) = cli.cmd else { panic!("stitch command") };
            assert_eq!(options.packing, expected);
        }
        assert!(Cli::try_parse_from(["grainatlas", "sheets", "mount-a.grainatlas", "--out", "print-a", "--packing", "other"]).err().unwrap().to_string().contains("expected lines or packed"));
        let cli = Cli::try_parse_from(["grainatlas", "sheets", "mount-a.grainatlas", "--out", "print-a"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Sheets { packing: None, .. }));
    }
}
