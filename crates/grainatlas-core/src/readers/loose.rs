//! Loose images: files chosen without any sidecar, `.ini` or `positions.csv`.
//! The user states one scale for the whole set (micrometres per pixel, or the
//! width of the field of view), once; each image becomes its own single-tile
//! area, or, when the user says so, the images are tiles of one grid with a
//! stated layout and overlap. The spec is recorded in `project.json`
//! (`package.kind = "loose"`, `package.loose`), so a reopened project is
//! rebuilt the same way.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{AreaMeta, Package, PackageInfo, TileMeta, is_image};
use crate::source::{Source, file_name};
use crate::{Error, Result};

/// How tiles fill a grid.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GridOrder {
    /// Left to right, then the next row down.
    #[default]
    Rows,
    /// Left to right, then right to left on the next row (serpentine; TM4000 Zigzag).
    Snake,
    /// Top to bottom, then the next column to the right.
    Columns,
}

/// The images are the tiles of one grid.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LooseGrid {
    pub cols: usize,
    pub rows: usize,
    /// Overlap between neighbouring tiles, percent of the tile size (same on both axes).
    pub overlap_pct: f64,
    #[serde(default)]
    pub order: GridOrder,
    /// Area name ("" = the set's name).
    #[serde(default)]
    pub name: String,
}

/// The scale and arrangement the user gave for a set of loose images.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LooseSpec {
    /// Micrometres per pixel for every image (used when `field_width_um` is not set).
    pub um_per_px: f64,
    /// Width of each image's field of view in micrometres (scale = width / image width).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_width_um: Option<f64>,
    /// None: each image is its own area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<LooseGrid>,
    /// Image paths in the source, in tile order ("" list = every image, sorted).
    #[serde(default)]
    pub files: Vec<String>,
}

/// One image found in a source (Open screen, before the scale is known).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageEntry {
    pub file: String,
    pub w: usize,
    pub h: usize,
}

/// Every decodable image in a source, sorted, with its size (0 × 0 when the header is unreadable).
pub fn list_images(src: &dyn Source) -> Vec<ImageEntry> {
    src.list()
        .into_iter()
        .filter(|f| is_image(f) && !file_name(f).starts_with('.'))
        .map(|f| {
            let (w, h) = src.read(&f).ok().and_then(|b| crate::image::probe_size(&b)).unwrap_or((0, 0));
            ImageEntry { file: f, w, h }
        })
        .collect()
}

/// A name for a set of files: their common stem prefix (without trailing `_- .`), else `fallback`.
pub fn set_name(files: &[String], fallback: &str) -> String {
    let stems: Vec<&str> = files.iter().map(|f| stem(file_name(f))).collect();
    let Some(first) = stems.first() else { return fallback.to_string() };
    let mut n = first.len();
    for s in &stems[1..] {
        n = n.min(first.bytes().zip(s.bytes()).take_while(|(a, b)| a == b).count());
    }
    while n > 0 && !first.is_char_boundary(n) {
        n -= 1;
    }
    let mut p = &first[..n];
    // Do not cut a running number in half ("img_000" of img_0001, img_0002).
    if first[n..].starts_with(|c: char| c.is_ascii_digit()) {
        p = p.trim_end_matches(|c: char| c.is_ascii_digit());
    }
    let p = p.trim_end_matches(['_', '-', ' ', '.']);
    if stems.len() > 1 && p.len() >= 3 { p.to_string() } else if stems.len() == 1 { first.to_string() } else { fallback.to_string() }
}

fn stem(name: &str) -> &str {
    name.rfind('.').map_or(name, |i| &name[..i])
}

pub struct LoosePackage {
    src: Arc<dyn Source>,
    info: PackageInfo,
}

impl LoosePackage {
    pub fn open(src: Arc<dyn Source>, spec: &LooseSpec) -> Result<Self> {
        if spec.um_per_px.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) && !spec.field_width_um.is_some_and(|w| w > 0.0) {
            return Err(Error::Invalid("loose images need a scale: micrometres per pixel or the field width".into()));
        }
        let all = list_images(src.as_ref());
        let images: Vec<ImageEntry> = if spec.files.is_empty() {
            all
        } else {
            spec.files
                .iter()
                .map(|f| all.iter().find(|i| &i.file == f).cloned().ok_or_else(|| Error::NotFound(f.clone())))
                .collect::<Result<_>>()?
        };
        if images.is_empty() {
            return Err(Error::Invalid("no images".into()));
        }
        let px_nm = |w: usize| match spec.field_width_um {
            Some(fw) if fw > 0.0 && w > 0 => fw * 1000.0 / w as f64,
            _ => spec.um_per_px * 1000.0,
        };
        let name = set_name(&images.iter().map(|i| i.file.clone()).collect::<Vec<_>>(), &src.root_name());
        let mut meta = BTreeMap::new();
        meta.insert("loose.um_per_px".to_string(), format!("{}", spec.um_per_px));
        if let Some(fw) = spec.field_width_um {
            meta.insert("loose.field_width_um".to_string(), format!("{fw}"));
        }
        let mut areas = Vec::new();
        match &spec.grid {
            None => {
                let mut used: BTreeMap<String, usize> = BTreeMap::new();
                for img in &images {
                    let base = stem(file_name(&img.file)).to_string();
                    let k = used.entry(base.clone()).or_insert(0);
                    *k += 1;
                    let id = if *k == 1 { base.clone() } else { format!("{base}_{k}") };
                    let p = px_nm(img.w);
                    let tile = TileMeta { index: 0, file: img.file.clone(), w: img.w, h: img.h, pos_um: [0.0, 0.0], stage_nm: None, pixel_size_nm: p, grid: None, meta: BTreeMap::new() };
                    areas.push(AreaMeta {
                        id: id.clone(),
                        name: id,
                        grid: Some([1, 1]),
                        pixel_size_nm: p,
                        field_um: Some([img.w as f64 * p / 1000.0, img.h as f64 * p / 1000.0]),
                        pitch_um: None,
                        stage_sign: [1.0, 1.0],
                        tiles: vec![tile],
                        meta: BTreeMap::new(),
                    });
                }
            }
            Some(g) => {
                let (cols, rows) = (g.cols.max(1), g.rows.max(1));
                if images.len() > cols * rows {
                    return Err(Error::Invalid(format!("{} images do not fit a {cols} × {rows} grid", images.len())));
                }
                if !(0.0..90.0).contains(&g.overlap_pct) {
                    return Err(Error::Invalid("overlap must be between 0 and 90 %".into()));
                }
                let w = images.iter().map(|i| i.w).max().unwrap_or(0);
                let h = images.iter().map(|i| i.h).max().unwrap_or(0);
                let p = px_nm(w);
                let k = 1.0 - g.overlap_pct / 100.0;
                let (pitch_x, pitch_y) = (w as f64 * k * p / 1000.0, h as f64 * k * p / 1000.0);
                let tiles = images
                    .iter()
                    .enumerate()
                    .map(|(n, img)| {
                        let (c, r) = match g.order {
                            GridOrder::Rows => (n % cols, n / cols),
                            GridOrder::Snake => {
                                let r = n / cols;
                                (if r % 2 == 0 { n % cols } else { cols - 1 - n % cols }, r)
                            }
                            GridOrder::Columns => (n / rows, n % rows),
                        };
                        TileMeta {
                            index: n,
                            file: img.file.clone(),
                            w: img.w,
                            h: img.h,
                            pos_um: [c as f64 * pitch_x, r as f64 * pitch_y],
                            stage_nm: None,
                            pixel_size_nm: p,
                            grid: Some([c, r]),
                            meta: BTreeMap::new(),
                        }
                    })
                    .collect();
                let id = if g.name.trim().is_empty() { name.clone() } else { g.name.trim().to_string() };
                let mut ameta = BTreeMap::new();
                ameta.insert("loose.overlap_pct".to_string(), format!("{}", g.overlap_pct));
                areas.push(AreaMeta {
                    id: id.clone(),
                    name: id,
                    grid: Some([cols, rows]),
                    pixel_size_nm: p,
                    field_um: Some([w as f64 * p / 1000.0, h as f64 * p / 1000.0]),
                    pitch_um: Some([pitch_x, pitch_y]),
                    stage_sign: [1.0, 1.0],
                    tiles,
                    meta: ameta,
                });
            }
        }
        let info = PackageInfo { reader: "loose".into(), kind: super::kind::LOOSE.into(), name, areas, meta, navigation: None };
        Ok(Self { src, info })
    }
}

impl Package for LoosePackage {
    fn info(&self) -> &PackageInfo {
        &self.info
    }
    fn source(&self) -> &dyn Source {
        self.src.as_ref()
    }
}
