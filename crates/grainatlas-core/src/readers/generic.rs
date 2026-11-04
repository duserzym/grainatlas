//! A plain folder of images plus `positions.csv` with columns
//! `file,x_um,y_um,pixel_size_nm` (header required; extra columns are kept as metadata).
//! Each folder holding a `positions.csv` is one area.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{AreaMeta, Package, PackageInfo, TileMeta, normalise_positions};
use crate::source::{Source, file_name, parent};
use crate::util::decode_text;
use crate::{Error, Result};

pub struct GenericPackage {
    src: Arc<dyn Source>,
    info: PackageInfo,
}

impl GenericPackage {
    pub fn detect(src: &dyn Source) -> bool {
        src.list().iter().any(|f| file_name(f).eq_ignore_ascii_case("positions.csv"))
    }

    pub fn open(src: Arc<dyn Source>) -> Result<Self> {
        let mut areas = Vec::new();
        for csv in src.list().into_iter().filter(|f| file_name(f).eq_ignore_ascii_case("positions.csv")) {
            let dir = parent(&csv).to_string();
            let text = decode_text(&src.read(&csv)?);
            let mut lines = text.lines().filter(|l| !l.trim().is_empty());
            let header: Vec<String> = lines.next().ok_or_else(|| Error::Invalid(format!("{csv}: empty")))?.split(',').map(|s| s.trim().to_ascii_lowercase()).collect();
            let col = |n: &str| header.iter().position(|h| h == n);
            let (Some(cf), Some(cx), Some(cy)) = (col("file"), col("x_um"), col("y_um")) else {
                return Err(Error::Invalid(format!("{csv}: need columns file,x_um,y_um[,pixel_size_nm]")));
            };
            let cp = col("pixel_size_nm");
            let mut tiles = Vec::new();
            for (i, line) in lines.enumerate() {
                let cells: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
                let get = |c: usize| cells.get(c).copied().unwrap_or("");
                let file = if dir.is_empty() { get(cf).to_string() } else { format!("{dir}/{}", get(cf)) };
                let x: f64 = get(cx).parse().map_err(|_| Error::Invalid(format!("{csv} line {}: x_um", i + 2)))?;
                let y: f64 = get(cy).parse().map_err(|_| Error::Invalid(format!("{csv} line {}: y_um", i + 2)))?;
                let p: f64 = cp.and_then(|c| get(c).parse().ok()).unwrap_or(1000.0);
                let mut meta = BTreeMap::new();
                for (k, h) in header.iter().enumerate() {
                    if ![Some(cf), Some(cx), Some(cy), cp].contains(&Some(k)) {
                        meta.insert(h.clone(), get(k).to_string());
                    }
                }
                let (w, h) = src.read(&file).ok().and_then(|b| crate::image::probe_size(&b)).unwrap_or((0, 0));
                tiles.push(TileMeta { index: i, file, w, h, pos_um: [x, y], stage_nm: None, pixel_size_nm: p, grid: None, meta });
            }
            normalise_positions(&mut tiles);
            let pixel_size_nm = tiles.first().map_or(1000.0, |t| t.pixel_size_nm);
            let id = if dir.is_empty() { src.root_name() } else { file_name(&dir).to_string() };
            areas.push(AreaMeta { id: id.clone(), name: id, grid: None, pixel_size_nm, field_um: None, pitch_um: None, stage_sign: [1.0, 1.0], tiles, meta: BTreeMap::new() });
        }
        let info = PackageInfo { reader: "generic".into(), kind: super::kind::PACKAGE.into(), name: src.root_name(), areas, meta: BTreeMap::new(), navigation: None };
        Ok(Self { src, info })
    }
}

impl Package for GenericPackage {
    fn info(&self) -> &PackageInfo {
        &self.info
    }
    fn source(&self) -> &dyn Source {
        self.src.as_ref()
    }
}
