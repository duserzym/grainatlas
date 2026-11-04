//! Session calls for cross-area grain ownership and print sheets (Boise SEM
//! run, 2026-11-03). Kept apart from `session.rs`; same JSON-in, JSON-out style.

use std::collections::BTreeMap;

use super::{Session, invalid};
use crate::layout::sheets::{self, SheetArea, SheetMeta, SheetParams};
use crate::ownership::{self, OwnershipParams};
use crate::photometric::ToneParams;
use crate::photometric::enhance::{self, EnhanceParams};
use crate::segment::Grain;
use crate::{Error, Result};

fn parse<T: serde::de::DeserializeOwned + Default>(s: &str) -> Result<T> {
    if s.trim().is_empty() { Ok(T::default()) } else { serde_json::from_str(s).map_err(invalid) }
}

impl Session {
    /// Cross-area ownership of the UI's grains (`{area_id: [Grain]}` JSON) with
    /// [`OwnershipParams`] JSON ("" = defaults). Returns [`ownership::Ownership`] JSON: for every
    /// detection its owner area and, for a duplicate, the owner's grain id; and the samples.
    pub fn ownership(&self, grains: &str, params: &str) -> Result<String> {
        let grains: BTreeMap<String, Vec<Grain>> = parse(grains)?;
        let p: OwnershipParams = parse(params)?;
        let project = self.project.as_ref().ok_or_else(|| Error::Invalid("no project".into()))?;
        serde_json::to_string(&ownership::assign(&project.areas, &grains, &p)).map_err(invalid)
    }

    /// Header metadata for print sheets ([`SheetMeta`] JSON), from the open package's sidecars,
    /// or the project's areas when the package is not open.
    pub fn sheet_meta(&self) -> Result<String> {
        serde_json::to_string(&self.meta()?).map_err(invalid)
    }

    fn meta(&self) -> Result<SheetMeta> {
        if let Ok(pkg) = self.pkg() {
            return Ok(sheets::sheet_meta(pkg.info()));
        }
        let project = self.project.as_ref().ok_or_else(|| Error::Invalid("no project".into()))?;
        Ok(SheetMeta {
            package: project.package.name.clone(),
            areas: project.areas.len(),
            tiles: project.areas.iter().map(|a| a.tiles.len()).sum(),
            um_per_px: project.areas.first().map_or(0.0, |a| a.um_per_px()),
            ..Default::default()
        })
    }

    /// Print sheets of every stitched area ([`SheetParams`] JSON; grains `{area_id: [Grain]}`;
    /// tone `{area_id: ToneParams}`, missing areas use their own). Returns
    /// `{"pages": [svg…], "registration": PrintRegistration, "stats": SheetStats,
    /// "ownership": Ownership}`; the PDF comes from `pdf_from_svgs(pages)`.
    pub fn print_sheets(&self, params: &str, grains: &str, tone: &str) -> Result<String> {
        let p: SheetParams = parse(params)?;
        let grains: BTreeMap<String, Vec<Grain>> = parse(grains)?;
        let tone: BTreeMap<String, ToneParams> = parse(tone)?;
        let project = self.project.as_ref().ok_or_else(|| Error::Invalid("no project".into()))?;
        let own = ownership::assign(&project.areas, &grains, &OwnershipParams::default());
        let areas: Vec<SheetArea> = project
            .areas
            .iter()
            .filter_map(|rec| {
                let o = self.outputs.get(&rec.id)?;
                Some(SheetArea { rec, img: super::pipeline::area_image(rec, o, tone.get(&rec.id).unwrap_or(&rec.params.tone)) })
            })
            .collect();
        if areas.is_empty() {
            return Err(Error::Invalid("no stitched area to print".into()));
        }
        let r = sheets::sheets(&p, &areas, &grains, &own, &self.meta()?);
        serde_json::to_string(&serde_json::json!({ "pages": r.pages, "registration": r.registration, "stats": r.stats, "ownership": own })).map_err(invalid)
    }

    /// The mosaic as PNG with the display tone and, as [`EnhanceParams`] asks, NLM denoising,
    /// unsharp masking and background stripping (`grains` = the area's grains JSON, needed to
    /// strip). The stored mosaic is not changed.
    pub fn export_png_enhanced(&self, area_id: &str, tone: &str, enhance_params: &str, grains: &str) -> Result<Vec<u8>> {
        let tone: ToneParams = serde_json::from_str(tone).map_err(invalid)?;
        let e: EnhanceParams = parse(enhance_params)?;
        let gs: Vec<Grain> = parse(grains)?;
        let o = self.out(area_id)?;
        let rec = self.rec(area_id)?;
        let lut = crate::photometric::tone::lut(&tone);
        let (w, h) = (o.linear.w, o.linear.h);
        let g = crate::image::Gray { w, h, px: o.base.iter().map(|&v| lut[v as usize] as f32 / 255.0).collect() };
        let mut g = enhance::denoise_sharpen(&g, &e);
        if e.strip {
            let mask = enhance::grain_mask(w, h, [0.0, 0.0], rec.um_per_px(), &gs, e.strip_margin_um).ok_or_else(|| Error::Invalid("a grain has no closed outline; background not stripped".into()))?;
            enhance::strip_background(&mut g, &mask, e.strip_grey as f32 / 255.0, rec.um_per_px()).map_err(|_| Error::Invalid("something outside every grain outline stands out; background not stripped".into()))?;
        }
        Ok(crate::image::encode_png_gray(w, h, &g.to_u8()))
    }
}
