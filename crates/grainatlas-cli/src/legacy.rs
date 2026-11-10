//! Reusable steps for importing a visit. Image operations read saved project outputs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use grainatlas_core::mosaic::pyramid::MosaicJson;
use image::{ImageDecoder, ImageReader, Rgb, RgbImage};
use serde_json::{Value, json};

use super::{AtlasOptions, StitchOptions, atlas, open, stitch};

#[derive(Args)]
pub(super) struct BatchOptions {
    sem_dir: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    segment: bool,
    /// Sample names by direct package folder, repeatable.
    #[arg(long = "sample", value_name = "PACKAGE=SAMPLE")]
    samples: Vec<String>,
    #[arg(long, value_parser = ["bright", "dark", "both"])]
    polarity: Option<String>,
    #[arg(long)]
    atlas_dir: Option<PathBuf>,
    /// Leave existing project folders untouched (including their atlas bundle).
    #[arg(long)]
    skip_existing: bool,
}

pub(super) fn batch(options: BatchOptions) -> Result<(Value, bool)> {
    let mut samples = BTreeMap::new();
    for sample in &options.samples {
        let (package, sample) = sample
            .split_once('=')
            .context("--sample needs PACKAGE=SAMPLE")?;
        ensure!(
            !package.is_empty() && !sample.is_empty(),
            "--sample needs PACKAGE=SAMPLE"
        );
        samples.insert(package.to_string(), sample.to_string());
    }
    let mut folders = Vec::new();
    for entry in std::fs::read_dir(&options.sem_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            folders.push(entry.path());
        }
    }
    folders.sort();
    let mut report = Vec::new();
    let mut failed = false;
    for folder in folders {
        let name = folder
            .file_name()
            .context("package folder name")?
            .to_string_lossy()
            .to_string();
        let out = options.out.join(format!("{name}.grainatlas"));
        // Keep the same package/area selection rules as `stitch` and `info`.
        if let Err(error) = open(&folder) {
            report.push(
                json!({ "package": name, "status": "skipped", "error": format!("{error:#}") }),
            );
            continue;
        }
        if options.skip_existing && out.exists() {
            report.push(json!({ "package": name, "status": "existing", "project": out }));
            continue;
        }
        let mut row = json!({ "package": name, "project": out });
        let result = (|| -> Result<()> {
            row["areas"] = stitch(StitchOptions {
                folder: folder.clone(),
                out: out.clone(),
                segment: options.segment,
                sample: samples.get(&name).cloned(),
                polarity: options.polarity.clone(),
                print_style: "sheets".into(),
                ..Default::default()
            })?;
            let project: Value = serde_json::from_slice(&std::fs::read(out.join("project.json"))?)?;
            let id = project["uuid"].as_str().context("project uuid")?;
            row["uuid"] = json!(id);
            if let Some(dir) = &options.atlas_dir {
                let bundle = dir.join(format!("{name}.zip"));
                atlas(AtlasOptions {
                    folder: folder.clone(),
                    out: bundle.clone(),
                    project_id: Some(id.into()),
                    quality: 100,
                    ..Default::default()
                })?;
                row["atlas"] = json!(bundle);
            }
            Ok(())
        })();
        match result {
            Ok(()) => row["status"] = json!("ok"),
            Err(error) => {
                row["status"] = json!("failed");
                row["error"] = json!(format!("{error:#}"));
                failed = true;
            }
        }
        report.push(row);
    }
    Ok((json!(report), failed))
}

fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&std::fs::read(path).with_context(|| path.display().to_string())?)
        .with_context(|| path.display().to_string())
}

/// Tempfile in the same directory, then rename: readers see the old or new JSON.
fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    use std::io::Write;
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().context("JSON parent directory")?)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}

fn object_entry<'a>(value: &'a mut Value, key: &str) -> Result<&'a mut Value> {
    let object = value.as_object_mut().context("expected a JSON object")?;
    let entry = object.entry(key).or_insert_with(|| json!({}));
    if entry.is_null() {
        *entry = json!({});
    }
    ensure!(entry.is_object(), "{key} must be an object");
    Ok(entry)
}

pub(super) fn overlay(project: &Path, area: &str, image: &Path, note: &str) -> Result<()> {
    let path = project.join("project.json");
    let mut value = read_json(&path)?;
    ensure!(
        value["areas"]
            .as_array()
            .context("project areas")?
            .iter()
            .any(|a| a["id"] == area),
        "unknown area id: {area}"
    );
    let mosaic: MosaicJson = serde_json::from_value(read_json(
        &project.join(format!("mosaic/{area}/mosaic.json")),
    )?)?;
    let mut decoder = ImageReader::open(image)?
        .with_guessed_format()?
        .into_decoder()?;
    let (mut w, mut h) = decoder.dimensions();
    // Only dimensions are needed; avoid decoding a full-resolution printout photo.
    if matches!(decoder.orientation()?.to_exif(), 5..=8) {
        std::mem::swap(&mut w, &mut h);
    }
    ensure!(w > 0 && h > 0, "empty overlay image");
    let name = image
        .file_name()
        .context("image file name")?
        .to_string_lossy()
        .to_string();
    let k0 = (mosaic.w as f64 * mosaic.um_per_px / w as f64)
        .min(mosaic.h as f64 * mosaic.um_per_px / h as f64);
    let overlays = object_entry(object_entry(&mut value, "grainatlas_ui")?, "overlays")?;
    let previous = &overlays[area];
    if previous.is_null()
        || (previous["name"] == name && (previous["w"] != w || previous["h"] != h))
    {
        overlays[area] = json!({ "name": name, "url": null, "w": w, "h": h,
            "pairs": [], "matrix": [k0, 0, 0, k0, 0, 0], "rms_um": 0,
            "opacity": 0.6, "visible": true });
    } else if previous["name"] == name {
        // Like Load image: reloading the same image keeps its registration.
        overlays[area]["url"] = Value::Null;
    }
    let listing_path = project.join("annotated_photos.json");
    let mut listing = if listing_path.exists() {
        read_json(&listing_path)?
    } else {
        json!({ "photos": [] })
    };
    // Also accept a plain list, preserving older/imported listing metadata.
    let photos = if listing.is_array() {
        &mut listing
    } else {
        listing
            .as_object_mut()
            .context("annotated_photos.json must be an object or list")?
            .entry("photos")
            .or_insert_with(|| json!([]))
    };
    photos
        .as_array_mut()
        .context("annotated_photos.json photos must be a list")?
        .push(json!({
            "photo": name, "file": image, "area": area, "note": note
        }));
    atomic_json(&path, &value)?;
    atomic_json(&listing_path, &listing)?;
    Ok(())
}

pub(super) fn render(project: &Path, out: &Path, max_width: u32) -> Result<()> {
    ensure!(max_width > 0, "--max-width must be positive");
    let format = match out
        .extension()
        .and_then(|x| x.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => image::ImageFormat::Png,
        Some("jpg" | "jpeg") => image::ImageFormat::Jpeg,
        _ => bail!("--out must end in .png or .jpg"),
    };
    let value = read_json(&project.join("project.json"))?;
    let mut panels = Vec::new();
    for area in value["areas"].as_array().context("project areas")? {
        let id = area["id"].as_str().context("area id")?;
        let dir = project.join("mosaic").join(id);
        let m: MosaicJson = serde_json::from_value(read_json(&dir.join("mosaic.json"))?)?;
        let level = m
            .levels
            .iter()
            .filter(|l| l.w <= max_width as usize)
            .max_by_key(|l| l.w)
            .or_else(|| m.levels.iter().min_by_key(|l| l.w))
            .context("empty pyramid")?;
        let mut rgb = RgbImage::new(level.w.try_into()?, level.h.try_into()?);
        for row in 0..level.rows {
            for col in 0..level.cols {
                let tile_path = dir.join(format!("{}/{}_{}.{}", level.level, col, row, m.format));
                let tile = image::open(&tile_path)
                    .with_context(|| tile_path.display().to_string())?
                    .to_rgb8();
                image::imageops::replace(
                    &mut rgb,
                    &tile,
                    (col * m.tile_size) as i64,
                    (row * m.tile_size) as i64,
                );
            }
        }
        // Very small width bounds can be below even the smallest saved pyramid level.
        if rgb.width() > max_width {
            let height = (rgb.height() as f64 * max_width as f64 / rgb.width() as f64)
                .round()
                .max(1.0) as u32;
            rgb = image::imageops::resize(
                &rgb,
                max_width,
                height,
                image::imageops::FilterType::Triangle,
            );
        }
        let mask_path = project.join(format!("masks/{id}/grains.png"));
        let mut grains = "?".to_string();
        if mask_path.exists() {
            let mask = image::open(&mask_path)?.to_luma16();
            grains = mask
                .pixels()
                .map(|p| p.0[0])
                .filter(|p| *p != 0)
                .collect::<BTreeSet<_>>()
                .len()
                .to_string();
            let mask = image::imageops::resize(
                &mask,
                rgb.width(),
                rgb.height(),
                image::imageops::FilterType::Nearest,
            );
            for y in 0..rgb.height() {
                for x in 0..rgb.width() {
                    let label = mask.get_pixel(x, y);
                    let edge = (x > 0 && label != mask.get_pixel(x - 1, y))
                        || (y > 0 && label != mask.get_pixel(x, y - 1))
                        || (label.0[0] > 0
                            && (x == 0 || y == 0 || x + 1 == rgb.width() || y + 1 == rgb.height()));
                    if edge {
                        rgb.put_pixel(x, y, Rgb([255, 60, 0]));
                    }
                }
            }
        }
        let tiles = area["tiles"].as_array().map_or(0, Vec::len);
        let text = format!(
            "{id}  tiles {tiles}  grains {grains}  {}x{} px  {:.3} um/px",
            m.w, m.h, m.um_per_px
        );
        panels.push((rgb, text));
    }
    ensure!(!panels.is_empty(), "project has no areas");
    let width = panels.iter().map(|(im, _)| im.width()).max().unwrap();
    let height = panels
        .iter()
        .try_fold(0u32, |sum, (im, _)| {
            sum.checked_add(im.height())?.checked_add(34)
        })
        .context("overview too tall")?;
    let mut output = RgbImage::from_pixel(width, height, Rgb([220, 220, 220]));
    let mut y = 0;
    for (rgb, text) in panels {
        let header = header(&text, width)?;
        image::imageops::replace(&mut output, &header, 0, y);
        image::imageops::replace(&mut output, &rgb, 0, y + 28);
        y += rgb.height() as i64 + 34;
    }
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    output.save_with_format(out, format)?;
    Ok(())
}

fn header(text: &str, width: u32) -> Result<RgbImage> {
    let mut db = resvg::usvg::fontdb::Database::new();
    db.load_font_data(include_bytes!("../../grainatlas-core/assets/fonts/DejaVuSans.ttf").to_vec());
    let options = resvg::usvg::Options {
        font_family: "DejaVu Sans".into(),
        fontdb: Arc::new(db),
        ..Default::default()
    };
    let text = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    // Keep the entire header within the requested bound, including small previews.
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="28"><rect width="100%" height="100%" fill="white"/><text x="4" y="19" font-size="14">{text}</text></svg>"#
    );
    let tree = resvg::usvg::Tree::from_str(&svg, &options)?;
    let text_width = tree.root().abs_bounding_box().width().max(width as f32);
    let scale = (width as f32 / text_width).min(1.0);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, 28).context("header dimensions")?;
    pixmap.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let rgba =
        image::RgbaImage::from_raw(width, 28, pixmap.data().to_vec()).context("header pixels")?;
    Ok(image::DynamicImage::ImageRgba8(rgba).to_rgb8())
}
