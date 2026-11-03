//! A working session over one opened package: the project record, the
//! stitched areas in memory and the segmentation work images. The browser
//! (grainatlas-wasm) and the desktop app (Tauri commands) are thin wrappers
//! around this type, so both run the same code. JSON in, JSON or bytes out.
//!
//! Image buffers start with an 8-byte header: width and height as
//! little-endian u32.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::pipeline::{self, AreaOutput, Clock, Timings};
use super::{AreaParams, Project};
use crate::photometric::ToneParams;
use crate::readers::{self, Package};
use crate::segment::{Grain, SegmentParams, WorkImage};
use crate::source::{MemSource, Source};
use crate::spots::{Label, Spot};
use crate::{Ctl, Error, Result};

fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Invalid(e.to_string())
}

pub fn with_header(w: usize, h: usize, px: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + px.len());
    out.extend_from_slice(&(w as u32).to_le_bytes());
    out.extend_from_slice(&(h as u32).to_le_bytes());
    out.extend_from_slice(px);
    out
}

pub struct Session {
    /// Files handed over one by one (browser) before `open`.
    pub mem: MemSource,
    src: Option<Arc<dyn Source>>,
    pkg: Option<Box<dyn Package>>,
    pub project: Option<Project>,
    pub outputs: BTreeMap<String, AreaOutput>,
    work: BTreeMap<String, (usize, WorkImage)>,
}

impl Session {
    pub fn new(name: &str) -> Session {
        Session { mem: MemSource::new(name), src: None, pkg: None, project: None, outputs: BTreeMap::new(), work: BTreeMap::new() }
    }

    pub fn add_file(&mut self, path: &str, bytes: Vec<u8>) {
        self.mem.insert(path, bytes);
    }

    /// Opens the files added with `add_file`.
    pub fn open_mem(&mut self, created: &str) -> Result<String> {
        let src: Arc<dyn Source> = Arc::new(self.mem.clone());
        self.open_source(src, &self.mem.name.clone(), created)
    }

    /// Opens any source (a directory on desktop). `path` is recorded in the project.
    pub fn open_source(&mut self, src: Arc<dyn Source>, path: &str, created: &str) -> Result<String> {
        let pkg = readers::open(src.clone())?;
        let info = serde_json::to_string(pkg.info()).map_err(invalid)?;
        let keep = self.project.as_ref().is_some_and(|p| p.package.name == pkg.info().name);
        if !keep {
            self.project = Some(Project::new(pkg.info(), path, created));
        }
        self.outputs.clear();
        self.work.clear();
        self.src = Some(src);
        self.pkg = Some(pkg);
        Ok(info)
    }

    pub fn load_project(&mut self, json: &str) -> Result<()> {
        self.project = Some(Project::from_json(json)?);
        Ok(())
    }

    pub fn project_json(&self) -> Result<String> {
        self.project.as_ref().map(|p| p.to_json()).ok_or_else(|| Error::Invalid("no project".into()))
    }

    fn pkg(&self) -> Result<&dyn Package> {
        self.pkg.as_deref().ok_or_else(|| Error::Invalid("no package".into()))
    }

    pub fn navigation(&self) -> Vec<u8> {
        let (Ok(pkg), Some(src)) = (self.pkg(), self.src.as_ref()) else { return Vec::new() };
        let Some(nav) = &pkg.info().navigation else { return Vec::new() };
        let Ok(bytes) = src.read(&nav.file) else { return Vec::new() };
        match crate::image::decode(&bytes) {
            Ok((g, _)) => with_header(g.w, g.h, &g.to_u8()),
            Err(_) => Vec::new(),
        }
    }

    pub fn tile_thumb(&self, area: usize, tile: usize, max: usize) -> Result<Vec<u8>> {
        let t = self.pkg()?.tile(area, tile)?;
        let k = t.image.w.max(t.image.h).div_ceil(max.max(1)).max(1);
        let g = t.image.downsample(k);
        Ok(with_header(g.w, g.h, &g.to_u8()))
    }

    /// Runs the whole pipeline for one area (`params` = AreaParams JSON, "" = the project's).
    pub fn stitch(&mut self, area_id: &str, params: &str, ctl: &Ctl, clock: Clock) -> Result<String> {
        let pkg = self.pkg.as_deref().ok_or_else(|| Error::Invalid("no package".into()))?;
        let project = self.project.as_mut().ok_or_else(|| Error::Invalid("no project".into()))?;
        let ai = pkg.info().areas.iter().position(|a| a.id == area_id).ok_or_else(|| Error::NotFound(area_id.into()))?;
        let params: AreaParams = if params.trim().is_empty() {
            project.areas.iter().find(|a| a.id == area_id).map(|a| a.params.clone()).unwrap_or_default()
        } else {
            serde_json::from_str(params).map_err(invalid)?
        };
        if project.areas.iter().all(|a| a.id != area_id) {
            project.areas.push(super::AreaRecord::from_meta(&pkg.info().areas[ai]));
        }
        let rec = project.area_mut(area_id).expect("area");
        let mut timings = Timings::default();
        let t0 = clock();
        ctl.report("read", 0.0);
        let tiles = pipeline::read_tiles(pkg, ai, rec)?;
        timings.steps.push(("read".into(), clock() - t0));
        let out = pipeline::run(rec, &tiles, &params, ctl, clock, &mut timings)?;
        let summary = serde_json::json!({
            "mosaic": out.mosaic_json,
            "placement": out.placement,
            "photometric": rec.photometric,
            "timings": timings.steps,
        });
        self.outputs.insert(area_id.to_string(), out);
        self.work.remove(area_id);
        Ok(summary.to_string())
    }

    fn out(&self, area_id: &str) -> Result<&AreaOutput> {
        self.outputs.get(area_id).ok_or_else(|| Error::Invalid(format!("{area_id} is not stitched")))
    }

    pub fn pyramid_tile(&self, area_id: &str, level: usize, col: usize, row: usize) -> Result<Vec<u8>> {
        let (w, h, px) = self.out(area_id)?.pyramid.tile(level, col, row).ok_or_else(|| Error::NotFound("tile".into()))?;
        Ok(with_header(w, h, &px))
    }

    pub fn histogram(&self, area_id: &str) -> Result<Vec<u32>> {
        let o = self.out(area_id)?;
        let mut h = vec![0u32; 256];
        for (v, c) in o.base.iter().zip(&o.coverage) {
            if *c >= 0.5 {
                h[*v as usize] += 1;
            }
        }
        Ok(h)
    }

    pub fn retone(&mut self, area_id: &str, tone: &str) -> Result<String> {
        let tone: ToneParams = serde_json::from_str(tone).map_err(invalid)?;
        let project = self.project.as_mut().ok_or_else(|| Error::Invalid("no project".into()))?;
        let rec = project.area_mut(area_id).ok_or_else(|| Error::NotFound(area_id.into()))?;
        let o = self.outputs.get_mut(area_id).ok_or_else(|| Error::Invalid("not stitched".into()))?;
        pipeline::retone(rec, o, &tone);
        serde_json::to_string(&o.mosaic_json).map_err(invalid)
    }

    pub fn export_png(&self, area_id: &str, tone: &str) -> Result<Vec<u8>> {
        let tone: ToneParams = serde_json::from_str(tone).map_err(invalid)?;
        let o = self.out(area_id)?;
        let lut = crate::photometric::tone::lut(&tone);
        let px: Vec<u8> = o.base.iter().map(|&v| lut[v as usize]).collect();
        Ok(crate::image::encode_png_gray(o.linear.w, o.linear.h, &px))
    }

    fn rec(&self, area_id: &str) -> Result<&super::AreaRecord> {
        self.project.as_ref().and_then(|p| p.areas.iter().find(|a| a.id == area_id)).ok_or_else(|| Error::NotFound(area_id.into()))
    }

    fn work_for(&mut self, area_id: &str, scale: usize) -> Result<&WorkImage> {
        let fresh = !matches!(self.work.get(area_id), Some((s, _)) if *s == scale);
        if fresh {
            let w = pipeline::work_image(self.out(area_id)?, self.rec(area_id)?, scale);
            self.work.insert(area_id.to_string(), (scale, w));
        }
        Ok(&self.work.get(area_id).expect("inserted").1)
    }

    fn seg_scale(&self, area_id: &str) -> usize {
        self.rec(area_id).map_or(2, |a| a.params.segment.work_scale)
    }

    pub fn segment(&mut self, area_id: &str, params: &str, previous: &str) -> Result<String> {
        let p: SegmentParams = serde_json::from_str(params).map_err(invalid)?;
        let prev: Vec<Grain> = if previous.trim().is_empty() { Vec::new() } else { serde_json::from_str(previous).map_err(invalid)? };
        let grains = crate::segment::segment(self.work_for(area_id, p.work_scale)?, &p, &prev);
        if let Some(rec) = self.project.as_mut().and_then(|pr| pr.area_mut(area_id)) {
            rec.params.segment = p;
        }
        serde_json::to_string(&grains).map_err(invalid)
    }

    pub fn grain_at(&mut self, area_id: &str, x_um: f64, y_um: f64, params: &str) -> Result<String> {
        let p: SegmentParams = serde_json::from_str(params).map_err(invalid)?;
        let w = self.work_for(area_id, p.work_scale)?;
        serde_json::to_string(&crate::segment::grain_at(w, &p, x_um, y_um)).map_err(invalid)
    }

    pub fn split_grain(&mut self, area_id: &str, grain: &str, line: [f64; 4]) -> Result<String> {
        let g: Grain = serde_json::from_str(grain).map_err(invalid)?;
        let scale = self.seg_scale(area_id);
        let w = self.work_for(area_id, scale)?;
        serde_json::to_string(&crate::segment::split_grain(w, &g, line, 0.8)).map_err(invalid)
    }

    pub fn merge_grains(&mut self, area_id: &str, grains: &str) -> Result<String> {
        let gs: Vec<Grain> = serde_json::from_str(grains).map_err(invalid)?;
        let scale = self.seg_scale(area_id);
        let w = self.work_for(area_id, scale)?;
        serde_json::to_string(&crate::segment::merge_grains(w, &gs, 0.8)).map_err(invalid)
    }

    pub fn refine_grain(&mut self, area_id: &str, grain: &str) -> Result<String> {
        let g: Grain = serde_json::from_str(grain).map_err(invalid)?;
        let scale = self.seg_scale(area_id);
        let w = self.work_for(area_id, scale)?;
        let out = crate::segment::refine_grain(w, &g, 0.8).unwrap_or(g);
        serde_json::to_string(&out).map_err(invalid)
    }

    pub fn export_grains(&self, area_id: &str, grains: &str) -> Result<String> {
        let gs: Vec<Grain> = serde_json::from_str(grains).map_err(invalid)?;
        let rec = self.rec(area_id)?;
        Ok(crate::segment::grains_csv(&gs, |x, y| rec.stage_of(x, y), |_| 0))
    }

    pub fn export_spots(&self, spots: &str, grains: &str, format: &str) -> Result<String> {
        let spots: Vec<Spot> = serde_json::from_str(spots).map_err(invalid)?;
        let project = self.project.as_ref().ok_or_else(|| Error::Invalid("no project".into()))?;
        match format {
            "csv" => Ok(crate::spots::spots_csv(project, &spots)),
            _ => {
                let grains: BTreeMap<String, Vec<Grain>> = serde_json::from_str(grains).map_err(invalid)?;
                serde_json::to_string_pretty(&crate::spots::geojson(project, &grains, &spots)).map_err(invalid)
            }
        }
    }

    pub fn set_link(&mut self, mount_id: &str, samples: &str) -> Result<()> {
        let project = self.project.as_mut().ok_or_else(|| Error::Invalid("no project".into()))?;
        project.laicpmsdb.mount_id = if mount_id.trim().is_empty() { None } else { Some(mount_id.trim().parse().map_err(invalid)?) };
        if !samples.trim().is_empty() {
            let m: BTreeMap<String, String> = serde_json::from_str(samples).map_err(invalid)?;
            for a in project.areas.iter_mut() {
                if let Some(s) = m.get(&a.id) {
                    a.sample = s.clone();
                }
            }
        }
        Ok(())
    }

    pub fn layout(&self, params: &str, grains: &str, spots: &str, labels: &str, tone: &str) -> Result<String> {
        let p: crate::layout::LayoutParams = serde_json::from_str(params).map_err(invalid)?;
        let grains: BTreeMap<String, Vec<Grain>> = serde_json::from_str(grains).map_err(invalid)?;
        let spots: Vec<Spot> = serde_json::from_str(spots).map_err(invalid)?;
        let labels: Vec<Label> = serde_json::from_str(labels).map_err(invalid)?;
        let tone: BTreeMap<String, ToneParams> = serde_json::from_str(tone).map_err(invalid)?;
        let project = self.project.as_ref().ok_or_else(|| Error::Invalid("no project".into()))?;
        let imgs: Vec<crate::layout::AreaImage> = project
            .areas
            .iter()
            .filter_map(|rec| Some(pipeline::area_image(rec, self.outputs.get(&rec.id)?, tone.get(&rec.id).unwrap_or(&rec.params.tone))))
            .collect();
        serde_json::to_string(&crate::layout::layout(&p, &imgs, &grains, &spots, &labels)).map_err(invalid)
    }

    /// All files of the project folder (plan section 5.4) for a full project JSON that also
    /// carries the UI's grains, spots and labels under `grainatlas_ui`.
    pub fn project_files(&self, full_project_json: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let v: serde_json::Value = serde_json::from_str(full_project_json).map_err(invalid)?;
        let mut files = vec![("project.json".to_string(), serde_json::to_vec_pretty(&v).map_err(invalid)?)];
        let ui = &v["grainatlas_ui"];
        let spots = serde_json::json!({ "spots": ui["spots"], "labels": ui["labels"] });
        files.push(("spots.json".into(), serde_json::to_vec_pretty(&spots).map_err(invalid)?));
        let project = self.project.as_ref().ok_or_else(|| Error::Invalid("no project".into()))?;
        for rec in &project.areas {
            let Some(o) = self.outputs.get(&rec.id) else { continue };
            files.extend(pipeline::area_files(rec, o));
            let grains: Vec<Grain> = serde_json::from_value(ui["grains"][&rec.id].clone()).unwrap_or_default();
            if !grains.is_empty() {
                files.extend(pipeline::mask_files(rec, o, &grains));
            }
        }
        Ok(files)
    }
}

/// Renumbers grains (from = {"grainId", "number"} JSON or "").
pub fn renumber(grains: &str, tolerance: f64, from: &str) -> Result<String> {
    let mut gs: Vec<Grain> = serde_json::from_str(grains).map_err(invalid)?;
    let f = if from.trim().is_empty() {
        None
    } else {
        let v: serde_json::Value = serde_json::from_str(from).map_err(invalid)?;
        let id = v["grainId"].as_str().and_then(|s| s.parse().ok()).ok_or_else(|| Error::Invalid("grainId".into()))?;
        Some((id, v["number"].as_u64().unwrap_or(1) as u32))
    };
    crate::segment::renumber(&mut gs, tolerance, f);
    serde_json::to_string(&gs).map_err(invalid)
}

pub fn pdf_from_svgs(pages: &str) -> Result<Vec<u8>> {
    let pages: Vec<String> = serde_json::from_str(pages).map_err(invalid)?;
    crate::layout::pdf::pdf_from_svgs(&pages)
}

pub fn tone_lut(tone: &str) -> Result<Vec<u8>> {
    let tone: ToneParams = serde_json::from_str(tone).map_err(invalid)?;
    Ok(crate::photometric::tone::lut(&tone).to_vec())
}

/// Reference-image registration from point pairs: `pairs` = `[[[img_x, img_y], [um_x, um_y]], ...]`.
pub fn register(pairs: &str) -> Result<String> {
    let pairs: Vec<[[f64; 2]; 2]> = serde_json::from_str(pairs).map_err(invalid)?;
    let src: Vec<[f64; 2]> = pairs.iter().map(|p| p[0]).collect();
    let dst: Vec<[f64; 2]> = pairs.iter().map(|p| p[1]).collect();
    serde_json::to_string(&crate::scale::register::fit(&src, &dst)).map_err(invalid)
}
