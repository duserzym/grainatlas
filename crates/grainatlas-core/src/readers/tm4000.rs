//! Hitachi TM4000 with Zigzag multi-capture (Tsunagi): `<mount>.zag` (UTF-8 XML),
//! `<area>/<area>_.ini` (UTF-16LE), `<area>/<area>_NNNN.txt` (UTF-16LE `[SemImageFile]`)
//! next to `<area>_NNNN.jpg`.
//!
//! Positions come from each tile's `StagePositionX/Y` (nm), never from the index
//! order. On the TM4000 stage x and y both decrease towards the right and the
//! bottom of the image, so the micrometre frame is `-stage / 1000`.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{AreaMeta, Navigation, Package, PackageInfo, TileMeta, normalise_positions};
use crate::source::{Source, file_name, parent};
use crate::util::{decode_text, leading_f64, parse_ini};
use crate::{Error, Result};

pub struct TM4000Package {
    src: Arc<dyn Source>,
    info: PackageInfo,
}

/// `<anything>_NNNN.<ext>` → (stem, NNNN).
fn tile_number(name: &str) -> Option<(String, usize)> {
    let dot = name.rfind('.')?;
    let stem = &name[..dot];
    let us = stem.rfind('_')?;
    let num = &stem[us + 1..];
    if num.len() != 4 || !num.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((stem[..us].to_string(), num.parse().ok()?))
}

impl TM4000Package {
    pub fn detect(src: &dyn Source) -> bool {
        let files = src.list();
        files.iter().any(|f| f.to_ascii_lowercase().ends_with(".zag"))
            || files.iter().any(|f| f.ends_with(".txt") && tile_number(file_name(f)).is_some() && Self::is_sem_txt(src, f))
    }

    fn is_sem_txt(src: &dyn Source, f: &str) -> bool {
        src.read(f).map(|b| decode_text(&b).contains("[SemImageFile]")).unwrap_or(false)
    }

    pub fn open(src: Arc<dyn Source>) -> Result<Self> {
        let files = src.list();
        let mut meta = BTreeMap::new();
        let mut zag_areas: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut navigation = None;
        let mut name = src.root_name();
        if let Some(zag) = files.iter().find(|f| f.to_ascii_lowercase().ends_with(".zag") && parent(f).is_empty()).or_else(|| files.iter().find(|f| f.to_ascii_lowercase().ends_with(".zag"))) {
            let fname = file_name(zag);
            name = fname[..fname.len() - 4].to_string();
            let text = decode_text(&src.read(zag)?);
            let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|e| Error::Read(zag.clone(), e.to_string()))?;
            let root = doc.root_element();
            for a in root.attributes() {
                meta.insert(format!("zag.{}", a.name()), a.value().to_string());
            }
            for def in root.children().filter(|n| n.has_tag_name("AreaDefinition")) {
                let mut m = BTreeMap::new();
                collect_xml(def, "", &mut m);
                if let Some(folder) = m.get("FolderName").cloned() {
                    zag_areas.insert(folder, m);
                }
            }
            let g = |k: &str| meta.get(&format!("zag.ImageInfo{k}")).and_then(|v| v.parse::<f64>().ok());
            let nav_file = files.iter().find(|f| parent(f).is_empty() && (file_name(f) == format!("{name}.bmp") || file_name(f).contains("navigation")) && is_image(f));
            if let (Some(f), Some(tlx), Some(tly), Some(brx), Some(bry)) =
                (nav_file, g("TopLeftStagePositionX"), g("TopLeftStagePositionY"), g("BottomRightStagePositionX"), g("BottomRightStagePositionY"))
            {
                navigation = Some(Navigation { file: f.clone(), top_left_nm: [tlx, tly], bottom_right_nm: [brx, bry] });
            }
        }

        // Group tile sidecars by folder.
        let mut folders: BTreeMap<String, Vec<(usize, String, String)>> = BTreeMap::new(); // folder -> (n, txt, image)
        for f in &files {
            let fname = file_name(f);
            if !fname.to_ascii_lowercase().ends_with(".txt") {
                continue;
            }
            let Some((stem, n)) = tile_number(fname) else { continue };
            let dir = parent(f).to_string();
            let img = files
                .iter()
                .find(|g| parent(g) == dir && is_image(g) && tile_number(file_name(g)).is_some_and(|(s, m)| s == stem && m == n))
                .cloned();
            if let Some(img) = img {
                folders.entry(dir).or_default().push((n, f.clone(), img));
            }
        }
        if folders.is_empty() {
            return Err(Error::Invalid("no TM4000 tiles (<area>_NNNN.jpg with .txt sidecar) found".into()));
        }

        // No .zag and the tiles at the top: one area folder opened on its own.
        let kind = if zag_areas.is_empty() && meta.is_empty() && folders.len() == 1 && folders.contains_key("") { super::kind::AREA } else { super::kind::PACKAGE };
        let mut areas = Vec::new();
        for (dir, mut list) in folders {
            list.sort();
            let folder = if dir.is_empty() { src.root_name() } else { file_name(&dir).to_string() };
            let mut ameta: BTreeMap<String, String> = BTreeMap::new();
            if let Some(z) = zag_areas.get(&folder) {
                for (k, v) in z {
                    ameta.insert(format!("zag.{k}"), v.clone());
                }
            }
            // The area's .ini (UTF-16LE).
            let ini = files.iter().find(|f| parent(f) == dir && f.to_ascii_lowercase().ends_with("_.ini"));
            if let Some(ini) = ini {
                for (sec, k, v) in parse_ini(&decode_text(&src.read(ini)?)) {
                    let key = if sec.is_empty() { format!("ini.{k}") } else { format!("ini.{sec}.{k}") };
                    ameta.insert(key, v);
                }
            }
            let ini_f = |k: &str| ameta.iter().find(|(kk, _)| kk.starts_with("ini.") && kk.ends_with(&format!(".{k}"))).and_then(|(_, v)| v.parse::<f64>().ok());
            let ini_u = |k: &str| ini_f(k).map(|v| v as usize);
            let grid = match (ini_u("wXCount"), ini_u("wYCount")) {
                (Some(x), Some(y)) => Some([x, y]),
                _ => match (ameta.get("zag.CaptureNumX").and_then(|v| v.parse().ok()), ameta.get("zag.CaptureNumY").and_then(|v| v.parse().ok())) {
                    (Some(x), Some(y)) => Some([x, y]),
                    _ => None,
                },
            };
            let field_um = ini_f("dXSize").zip(ini_f("dYSize")).map(|(a, b)| [a, b]);
            let pitch_um = ini_f("dXPitch").zip(ini_f("dYPitch")).map(|(a, b)| [a, b]);

            let mut tiles = Vec::new();
            for (i, (n, txt, img)) in list.iter().enumerate() {
                let mut tm = BTreeMap::new();
                for (sec, k, v) in parse_ini(&decode_text(&src.read(txt)?)) {
                    let key = if sec.is_empty() || sec == "SemImageFile" { k } else { format!("{sec}.{k}") };
                    tm.insert(key, v);
                }
                let px_nm = tm.get("PixelSize").and_then(|v| leading_f64(v)).unwrap_or(0.0);
                let sx = tm.get("StagePositionX").and_then(|v| leading_f64(v));
                let sy = tm.get("StagePositionY").and_then(|v| leading_f64(v));
                let (w, h) = tm
                    .get("DataSize")
                    .and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?))))
                    .unwrap_or((0, 0));
                let stage = sx.zip(sy).map(|(a, b)| [a, b]);
                let grid_cell = grid.map(|[gx, _]| [n % gx.max(1), n / gx.max(1)]);
                let pos_um = match (stage, grid_cell, pitch_um) {
                    // Stage nm -> um, sign flipped on both axes (TM4000).
                    (Some([a, b]), _, _) => [-a / 1000.0, -b / 1000.0],
                    (None, Some([c, r]), Some([px, py])) => [c as f64 * px, r as f64 * py],
                    _ => [0.0, 0.0],
                };
                tiles.push(TileMeta { index: i, file: img.clone(), w, h, pos_um, stage_nm: stage, pixel_size_nm: px_nm, grid: grid_cell, meta: tm });
            }
            normalise_positions(&mut tiles);
            let pixel_size_nm = median(tiles.iter().map(|t| t.pixel_size_nm).filter(|v| *v > 0.0).collect()).unwrap_or(1000.0);
            // The .zag names the area; without it (an area folder alone) the folder `NNN_<name>` does.
            let area_name = ameta.get("zag.AreaName").cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| strip_index(&folder).to_string());
            areas.push(AreaMeta { id: folder.clone(), name: area_name, grid, pixel_size_nm, field_um, pitch_um, stage_sign: [-1.0, -1.0], tiles, meta: ameta });
        }
        let info = PackageInfo { reader: "tm4000".into(), kind: kind.to_string(), name, areas, meta, navigation };
        Ok(Self { src, info })
    }
}

use super::is_image;

/// `001_HJ-GC1` → `HJ-GC1` (Zigzag area folders are `<NNN>_<area name>`).
fn strip_index(folder: &str) -> &str {
    match folder.split_once('_') {
        Some((n, rest)) if !rest.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => rest,
        _ => folder,
    }
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
}

fn collect_xml(n: roxmltree::Node, prefix: &str, out: &mut BTreeMap<String, String>) {
    for c in n.children().filter(|c| c.is_element()) {
        let key = if prefix.is_empty() { c.tag_name().name().to_string() } else { format!("{prefix}.{}", c.tag_name().name()) };
        if c.children().any(|g| g.is_element()) {
            collect_xml(c, &key, out);
        } else {
            out.insert(key, c.text().unwrap_or("").trim().to_string());
        }
    }
}

impl Package for TM4000Package {
    fn info(&self) -> &PackageInfo {
        &self.info
    }
    fn source(&self) -> &dyn Source {
        self.src.as_ref()
    }
}
