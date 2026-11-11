//! Package readers (plan section 6.1). A reader never fails on an unknown key:
//! unknown metadata is kept as a string map.

pub mod generic;
pub mod loose;
pub mod tm4000;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::image::{Depth, Gray};
use crate::source::Source;
use crate::{Error, Result};

/// One tile as listed by a package, before pixels are read.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TileMeta {
    pub index: usize,
    /// Path relative to the package root.
    pub file: String,
    pub w: usize,
    pub h: usize,
    /// Prior position of the tile's top-left corner in the area's micrometre frame (x right, y down).
    pub pos_um: [f64; 2],
    /// Stage position in nm as recorded by the instrument (tile centre), if any.
    pub stage_nm: Option<[f64; 2]>,
    pub pixel_size_nm: f64,
    /// Grid cell (column, row) if the package says so.
    pub grid: Option<[usize; 2]>,
    /// Every key of the tile's sidecar, verbatim.
    pub meta: BTreeMap<String, String>,
}

/// One area (a grid of tiles over one region of the mount).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AreaMeta {
    /// Folder name (stable id within the package).
    pub id: String,
    pub name: String,
    pub grid: Option<[usize; 2]>,
    pub pixel_size_nm: f64,
    pub field_um: Option<[f64; 2]>,
    pub pitch_um: Option<[f64; 2]>,
    /// Sign of stage axes relative to mosaic axes (TM4000: x and y both decrease right/down).
    pub stage_sign: [f64; 2],
    pub tiles: Vec<TileMeta>,
    pub meta: BTreeMap<String, String>,
}

/// How a package was opened (recorded in `project.json` as `package.kind`).
pub mod kind {
    /// A whole instrument package (TM4000 `.zag` folder, or a folder with `positions.csv`).
    pub const PACKAGE: &str = "package";
    /// One TM4000 area folder on its own (`.ini`, tiles and `.txt` sidecars, no `.zag` above it).
    pub const AREA: &str = "area";
    /// Loose images without sidecars; the scale was given by the user ([`super::loose::LooseSpec`]).
    pub const LOOSE: &str = "loose";
}

pub fn default_kind() -> String {
    kind::PACKAGE.to_string()
}

/// A whole package as shown on the Open screen.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PackageInfo {
    pub reader: String,
    /// [`kind::PACKAGE`], [`kind::AREA`] or [`kind::LOOSE`].
    #[serde(default = "default_kind")]
    pub kind: String,
    pub name: String,
    pub areas: Vec<AreaMeta>,
    pub meta: BTreeMap<String, String>,
    /// Navigation image file and stage calibration, if present.
    pub navigation: Option<Navigation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Navigation {
    pub file: String,
    /// Stage (nm) of the navigation image's top-left and bottom-right corners.
    pub top_left_nm: [f64; 2],
    pub bottom_right_nm: [f64; 2],
}

/// Pixels of one tile.
#[derive(Clone, Debug)]
pub struct Tile {
    pub meta: TileMeta,
    pub image: Gray,
    pub depth: Depth,
    pub sha256: String,
    /// Capture status computed once from the decoded image.
    pub failed: bool,
}

/// Detects a blank capture in one pass over decoded, normalized grey pixels. A tile fails
/// when its population standard deviation is below 0.5 on the 0..255 grey scale, or
/// more than 99.5% of its pixels have the same decoded value (up to 16-bit precision).
/// No sampling: even a small non-constant remainder must satisfy the stated thresholds.
pub fn failed_capture(image: &Gray) -> bool {
    let mut hist = vec![0usize; 65536];
    let (mut sum, mut squares) = (0u128, 0u128);
    for &v in &image.px {
        let value = (v.clamp(0.0, 1.0) * 65535.0).round() as usize;
        sum += value as u128;
        squares += (value as u64 * value as u64) as u128;
        hist[value] += 1;
    }
    let n = image.px.len() as u128;
    // 16-bit / 257 is the 0..255 scale. Integer comparisons keep both strict
    // thresholds exact, including a 0.5-grey standard deviation near white.
    n == 0 || 4 * (squares * n - sum * sum) < 257 * 257 * n * n
        || hist.into_iter().max().unwrap_or(0) as u128 * 200 > 199 * n
}

#[cfg(test)]
mod failed_tests {
    use super::*;

    #[test]
    fn blank_and_low_variance_captures_fail() {
        for value in [0, 1, 127, 255] {
            assert!(failed_capture(&Gray::from_u8(100, 100, &vec![value; 10000])));
        }
        let g = Gray { w: 100, h: 1, px: (0..100).map(|i| (100.0 + (i % 2) as f32 * 0.9) / 255.0).collect() };
        assert!(failed_capture(&g));
        // Exactly 0.5 grey levels is accepted.
        assert!(!failed_capture(&Gray::from_u8(2, 1, &[0, 1])));
        assert!(!failed_capture(&Gray::from_u8(2, 1, &[254, 255])));
    }

    #[test]
    fn dominant_value_threshold_is_strict_and_independent_of_variance() {
        let mut px = vec![0; 10000];
        px[..50].fill(255);
        assert!(!failed_capture(&Gray::from_u8(100, 100, &px)));
        px[49] = 0;
        assert!(failed_capture(&Gray::from_u8(100, 100, &px)));
        let px: Vec<_> = (0..10000).map(|i| (i % 256) as u8).collect();
        assert!(!failed_capture(&Gray::from_u8(100, 100, &px)));
    }
}

/// The reader interface (plan section 6.1).
pub trait Package: Send + Sync {
    fn info(&self) -> &PackageInfo;
    fn areas(&self) -> Vec<AreaMeta> {
        self.info().areas.clone()
    }
    fn source(&self) -> &dyn Source;
    fn tile(&self, area: usize, index: usize) -> Result<Tile> {
        let a = self.info().areas.get(area).ok_or_else(|| Error::NotFound(format!("area {area}")))?;
        let m = a.tiles.get(index).ok_or_else(|| Error::NotFound(format!("tile {index}")))?;
        let bytes = self.source().read(&m.file)?;
        let sha256 = crate::util::sha256_hex(&bytes);
        let (image, depth) = crate::image::decode(&bytes)?;
        let mut meta = m.clone();
        meta.w = image.w;
        meta.h = image.h;
        let failed = failed_capture(&image);
        Ok(Tile { meta, image, depth, sha256, failed })
    }
    fn tiles_ctl(&self, area: usize, ctl: &crate::Ctl) -> Result<Vec<Tile>> {
        let n = self.info().areas.get(area).map_or(0, |a| a.tiles.len());
        crate::util::try_par_map(n, ctl, |i| self.tile(area, i))
    }
    /// All tiles of an area, in index order (decoded in parallel on native).
    fn tiles(&self, area: usize) -> Result<Vec<Tile>> {
        let n = self.info().areas.get(area).map_or(0, |a| a.tiles.len());
        crate::util::par_map(n, |i| self.tile(area, i)).into_iter().collect()
    }
}

/// Opens a package with the first reader that recognises it.
pub fn open(src: Arc<dyn Source>) -> Result<Box<dyn Package>> {
    if tm4000::TM4000Package::detect(src.as_ref()) {
        return Ok(Box::new(tm4000::TM4000Package::open(src)?));
    }
    if generic::GenericPackage::detect(src.as_ref()) {
        return Ok(Box::new(generic::GenericPackage::open(src)?));
    }
    Err(Error::Unsupported("no reader recognises this folder (expected a TM4000 Zigzag package or area folder, images with positions.csv, or plain images)".into()))
}

/// What opening a source found: a package the readers understand, or images that need a scale.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Probe {
    /// A reader recognised the source.
    Opened { info: PackageInfo },
    /// Plain images with no calibration: ask the user once for a scale ([`loose::LooseSpec`]).
    NeedsScale { name: String, images: Vec<loose::ImageEntry> },
}

/// Opens a source if a reader recognises it; otherwise lists its images (loose files).
pub fn probe(src: Arc<dyn Source>) -> Result<(Probe, Option<Box<dyn Package>>)> {
    match open(src.clone()) {
        Ok(pkg) => Ok((Probe::Opened { info: pkg.info().clone() }, Some(pkg))),
        Err(Error::Unsupported(msg)) => {
            let images = loose::list_images(src.as_ref());
            if images.is_empty() {
                return Err(Error::Unsupported(msg));
            }
            let name = loose::set_name(&images.iter().map(|i| i.file.clone()).collect::<Vec<_>>(), &src.root_name());
            Ok((Probe::NeedsScale { name, images }, None))
        }
        Err(e) => Err(e),
    }
}

/// True for the image types the readers decode.
pub fn is_image(f: &str) -> bool {
    let l = f.to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".tif", ".tiff", ".bmp"].iter().any(|e| l.ends_with(e))
}

/// Moves positions so the smallest x and y are zero.
pub(crate) fn normalise_positions(tiles: &mut [TileMeta]) {
    let mx = tiles.iter().map(|t| t.pos_um[0]).fold(f64::INFINITY, f64::min);
    let my = tiles.iter().map(|t| t.pos_um[1]).fold(f64::INFINITY, f64::min);
    if mx.is_finite() && my.is_finite() {
        for t in tiles {
            t.pos_um[0] -= mx;
            t.pos_um[1] -= my;
        }
    }
}
