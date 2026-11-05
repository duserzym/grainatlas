//! A working session over one opened package: the project record, the
//! stitched areas in memory and the segmentation work images. The browser
//! (grainatlas-wasm) and the desktop app (Tauri commands) are thin wrappers
//! around this type, so both run the same code. JSON in, JSON or bytes out.
//!
//! Image buffers start with an 8-byte header: width and height as
//! little-endian u32.

mod sheets;

use std::collections::BTreeMap;
use std::sync::Arc;

use super::pipeline::{self, AreaOutput, Clock, Timings};
use super::{AreaParams, Project};
use crate::photometric::ToneParams;
use crate::readers::loose::{LoosePackage, LooseSpec};
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
    /// Absolute paths of loose files opened by path (desktop), recorded in the project.
    pub origin_files: Vec<String>,
    /// Path recorded for a source opened with `open_source` (kept for `open_loose`).
    path: String,
    /// Downsampled stitch inputs per (area, factor), for live previews.
    small: BTreeMap<(String, usize), Vec<crate::image::Gray>>,
    /// The UI's current grains per area (for per-grain tone normalisation).
    grains: BTreeMap<String, Vec<Grain>>,
}

impl Session {
    pub fn new(name: &str) -> Session {
        Session { mem: MemSource::new(name), src: None, pkg: None, project: None, outputs: BTreeMap::new(), work: BTreeMap::new(), origin_files: Vec::new(), path: String::new(), small: BTreeMap::new(), grains: BTreeMap::new() }
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
    /// Returns a `readers::Probe` as JSON: `{"status":"opened","info":…}` for a package or an area
    /// folder, `{"status":"needs_scale","name":…,"images":[…]}` for images without calibration
    /// (then call [`Session::open_loose`] with the scale the user gave).
    pub fn open_source(&mut self, src: Arc<dyn Source>, path: &str, created: &str) -> Result<String> {
        let (probe, pkg) = readers::probe(src.clone())?;
        self.outputs.clear();
        self.work.clear();
        self.small.clear();
        self.src = Some(src);
        self.path = path.to_string();
        self.pkg = None;
        if let Some(pkg) = pkg {
            self.adopt(pkg, None, created);
        }
        serde_json::to_string(&probe).map_err(invalid)
    }

    /// Opens the images of the last `open_source` as loose files with the user's scale
    /// (`spec` = [`LooseSpec`] JSON). Returns the PackageInfo JSON.
    pub fn open_loose(&mut self, spec: &str, created: &str) -> Result<String> {
        let spec: LooseSpec = serde_json::from_str(spec).map_err(invalid)?;
        let src = self.src.clone().ok_or_else(|| Error::Invalid("nothing opened".into()))?;
        let pkg = LoosePackage::open(src, &spec)?;
        let info = serde_json::to_string(pkg.info()).map_err(invalid)?;
        self.outputs.clear();
        self.work.clear();
        self.small.clear();
        self.adopt(Box::new(pkg), Some(spec), created);
        Ok(info)
    }

    /// Makes `pkg` current; keeps a loaded project of the same package (same name, kind and
    /// areas), else starts a new one, and records how the package was opened.
    fn adopt(&mut self, pkg: Box<dyn Package>, loose: Option<LooseSpec>, created: &str) {
        let info = pkg.info();
        let ids: Vec<&str> = info.areas.iter().map(|a| a.id.as_str()).collect();
        let keep = self.project.as_ref().is_some_and(|p| {
            p.package.name == info.name && p.package.kind == info.kind && (info.kind != readers::kind::LOOSE || p.areas.iter().map(|a| a.id.as_str()).eq(ids.iter().copied()))
        });
        if !keep {
            self.project = Some(Project::new(info, &self.path, created));
        }
        if let Some(p) = self.project.as_mut() {
            p.package.kind = info.kind.clone();
            p.package.loose = loose;
            p.package.files = if info.kind == readers::kind::LOOSE { self.origin_files.clone() } else { Vec::new() };
        }
        self.pkg = Some(pkg);
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
        let grains = self.grains.get(area_id).map_or(&[][..], |g| g.as_slice());
        let out = pipeline::run_with(rec, &tiles, &params, grains, ctl, clock, &mut timings)?;
        let summary = serde_json::json!({
            "mosaic": out.mosaic_json,
            "placement": out.placement,
            "photometric": rec.photometric,
            "timings": timings.steps,
        });
        self.outputs.insert(area_id.to_string(), out);
        self.work.remove(area_id);
        self.small.retain(|(a, _), _| a != area_id);
        Ok(summary.to_string())
    }

    /// Applies new parameters to a stitched area with as little work as possible: tone only →
    /// re-tone; correction or blending → re-run from the solved placement (no re-stitch); stitch
    /// parameters → full stitch. Returns the same summary as `stitch`.
    pub fn recompute(&mut self, area_id: &str, params: &str, ctl: &Ctl, clock: Clock) -> Result<String> {
        let p: AreaParams = serde_json::from_str(params).map_err(invalid)?;
        let Some(rec) = self.project.as_ref().and_then(|pr| pr.areas.iter().find(|a| a.id == area_id)) else {
            return self.stitch(area_id, params, ctl, clock);
        };
        // A placement saved in the project (reopened, nothing rendered yet) is used as it is: the
        // area is rendered from it without stitching again.
        if rec.placement.is_none() || rec.params.stitch != p.stitch {
            return self.stitch(area_id, params, ctl, clock);
        }
        let rendered = self.outputs.contains_key(area_id);
        let mut timings = Timings::default();
        if rendered && rec.params.photometric == p.photometric && rec.params.blend == p.blend {
            let t0 = clock();
            self.retone(area_id, &serde_json::to_string(&p.tone).map_err(invalid)?)?;
            timings.steps.push(("tone".into(), clock() - t0));
        } else {
            let pkg = self.pkg.as_deref().ok_or_else(|| Error::Invalid("no package".into()))?;
            let ai = pkg.info().areas.iter().position(|a| a.id == area_id).ok_or_else(|| Error::NotFound(area_id.into()))?;
            let project = self.project.as_mut().ok_or_else(|| Error::Invalid("no project".into()))?;
            let rec = project.area_mut(area_id).expect("area");
            let t0 = clock();
            ctl.report("read", 0.0);
            let tiles = pipeline::read_tiles(pkg, ai, rec)?;
            let inputs = pipeline::inputs(rec, &tiles);
            drop(tiles);
            timings.steps.push(("read".into(), clock() - t0));
            let placement = rec.placement.clone().expect("placement");
            let grains = self.grains.get(area_id).map_or(&[][..], |g| g.as_slice());
            let out = pipeline::finish(rec, inputs.imgs, placement, &p, grains, ctl, clock, &mut timings)?;
            self.outputs.insert(area_id.to_string(), out);
            self.work.remove(area_id);
        }
        let rec = self.rec(area_id)?;
        let o = self.out(area_id)?;
        Ok(serde_json::json!({ "mosaic": o.mosaic_json, "placement": o.placement, "photometric": rec.photometric, "timings": timings.steps }).to_string())
    }

    /// A live preview of `params` on a stitched area, downsampled `2^level` times (an 8-byte
    /// w/h header, then 8-bit pixels): the whole mosaic at that pyramid level.
    pub fn preview(&mut self, area_id: &str, params: &str, level: usize) -> Result<Vec<u8>> {
        let p: AreaParams = serde_json::from_str(params).map_err(invalid)?;
        let k = 1usize << level.min(12);
        let rec = self.rec(area_id)?.clone();
        let placement = rec.placement.clone().ok_or_else(|| Error::Invalid(format!("{area_id} is not stitched")))?;
        let o = self.out(area_id)?;
        if rec.params.photometric == p.photometric && rec.params.blend == p.blend {
            let (w, h, px) = pipeline::preview_tone(o, &p.tone, k, self.grains_of(area_id), rec.um_per_px());
            return Ok(with_header(w, h, &px));
        }
        let key = (area_id.to_string(), k);
        if !self.small.contains_key(&key) {
            let pkg = self.pkg()?;
            let ai = pkg.info().areas.iter().position(|a| a.id == area_id).ok_or_else(|| Error::NotFound(area_id.into()))?;
            let tiles = pkg.tiles(ai)?;
            let inputs = pipeline::inputs(&rec, &tiles);
            let small: Vec<crate::image::Gray> = inputs.imgs.iter().map(|g| g.downsample(k)).collect();
            self.small.retain(|(a, _), _| a != area_id);
            self.small.insert(key.clone(), small);
        }
        let (w, h, px) = pipeline::preview(&self.small[&key], &placement, &p, k, self.grains_of(area_id), rec.um_per_px());
        Ok(with_header(w, h, &px))
    }

    fn grains_of(&self, area_id: &str) -> &[Grain] {
        self.grains.get(area_id).map_or(&[][..], |g| g.as_slice())
    }

    /// The UI's grains of an area (JSON list), used by per-grain tone normalisation. Pixels do
    /// not change until the next recompute.
    pub fn set_grains(&mut self, area_id: &str, grains: &str) -> Result<()> {
        let gs: Vec<Grain> = if grains.trim().is_empty() { Vec::new() } else { serde_json::from_str(grains).map_err(invalid)? };
        self.grains.insert(area_id.to_string(), gs);
        Ok(())
    }

    /// Suggested one-click parameters for a stitched area: `params` with background leveling on
    /// and the tone from `tone::auto` (see there), measured on the mosaic as these parameters
    /// leave it (so Auto gives the same answer whatever was set before). The caller applies them
    /// like any edit.
    pub fn auto_params(&self, area_id: &str, params: &str) -> Result<String> {
        let mut p: AreaParams = serde_json::from_str(params).map_err(invalid)?;
        p.photometric.background = true;
        let o = self.out(area_id)?;
        let rec = self.rec(area_id)?;
        let tone = if rec.params.photometric == p.photometric && rec.params.blend == p.blend {
            crate::photometric::tone::auto(&o.linear, Some(&o.coverage), rec.um_per_px(), &p.tone)
        } else {
            // The current mosaic is corrected differently: render the linear mosaic Auto will give.
            let pkg = self.pkg()?;
            let ai = pkg.info().areas.iter().position(|a| a.id == area_id).ok_or_else(|| Error::NotFound(area_id.into()))?;
            let mut r = rec.clone();
            let placement = r.placement.clone().ok_or_else(|| Error::Invalid(format!("{area_id} is not stitched")))?;
            let inputs = pipeline::inputs(&r, &pkg.tiles(ai)?);
            let out = pipeline::finish(&mut r, inputs.imgs, placement, &p, &[], &Ctl::NONE, &|| 0.0, &mut Timings::default())?;
            crate::photometric::tone::auto(&out.linear, Some(&out.coverage), rec.um_per_px(), &p.tone)
        };
        p.tone = tone;
        serde_json::to_string(&p).map_err(invalid)
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
        pipeline::retone(rec, o, &tone, self.grains.get(area_id).map_or(&[][..], |g| g.as_slice()));
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

    pub fn segment(&mut self, area_id: &str, params: &str, previous: &str, outline: &str) -> Result<String> {
        let p: SegmentParams = serde_json::from_str(params).map_err(invalid)?;
        let prev: Vec<Grain> = if previous.trim().is_empty() { Vec::new() } else { serde_json::from_str(previous).map_err(invalid)? };
        let outline = Self::outline_args(outline)?;
        let grains = crate::segment::segment_padded(self.work_for(area_id, p.work_scale)?, &p, &prev, outline);
        if let Some(rec) = self.project.as_mut().and_then(|pr| pr.area_mut(area_id)) {
            rec.params.segment = p;
        }
        serde_json::to_string(&grains).map_err(invalid)
    }

    pub fn grain_at(&mut self, area_id: &str, x_um: f64, y_um: f64, params: &str, outline: &str) -> Result<String> {
        let p: SegmentParams = serde_json::from_str(params).map_err(invalid)?;
        let w = self.work_for(area_id, p.work_scale)?;
        let outline = Self::outline_args(outline)?;
        // Click-grow has no neighbouring labels in its API; clip plain growth to coverage.
        let grain = crate::segment::grain_at(w, &p, x_um, y_um).map(|g| crate::segment::pad_outlines(w, &[g], outline, p.simplify_px, |_| true).remove(0));
        serde_json::to_string(&grain).map_err(invalid)
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

    /// A drawn outline (um, closed) snapped to the mosaic's edges within `width_um` of it.
    pub fn snap_outline(&mut self, area_id: &str, path: &str, width_um: f64, contrast: f64) -> Result<String> {
        let path: Vec<[f64; 2]> = serde_json::from_str(path).map_err(invalid)?;
        let o = self.out(area_id)?;
        let v = crate::segment::edit::View { w: o.linear.w, h: o.linear.h, px: &o.base, um_per_px: self.rec(area_id)?.um_per_px() };
        serde_json::to_string(&crate::segment::edit::snap_outline(&v, &path, width_um, contrast as f32)).map_err(invalid)
    }

    /// A grain from a drawn outline: `op` is `new`, `add` (to `base`) or `cut` (from `base`).
    /// `null` when nothing is left.
    pub fn outline_grain(&mut self, area_id: &str, polygon: &str, base: &str, op: &str, outline: &str) -> Result<String> {
        let poly: Vec<[f64; 2]> = serde_json::from_str(polygon).map_err(invalid)?;
        let (base, op) = Self::edit_args(base, op)?;
        let scale = self.seg_scale(area_id);
        let w = self.work_for(area_id, scale)?;
        let outline = Self::outline_args(outline)?;
        serde_json::to_string(&crate::segment::edit::outline_grain_padded(w, &poly, base.as_ref(), op, 0.8, outline)).map_err(invalid)
    }

    /// A grain from a brush stroke (um, diameter `size_um`) grown to the nearby edges.
    pub fn brush_grain(&mut self, area_id: &str, stroke: &str, size_um: f64, base: &str, op: &str) -> Result<String> {
        let stroke: Vec<[f64; 2]> = serde_json::from_str(stroke).map_err(invalid)?;
        let (base, op) = Self::edit_args(base, op)?;
        let scale = self.seg_scale(area_id);
        self.work_for(area_id, scale)?;
        let w = &self.work.get(area_id).expect("made above").1;
        let o = self.out(area_id)?;
        let v = crate::segment::edit::View { w: o.linear.w, h: o.linear.h, px: &o.base, um_per_px: self.rec(area_id)?.um_per_px() };
        serde_json::to_string(&crate::segment::edit::brush_grain(w, &v, &stroke, size_um, base.as_ref(), op, 0.8)).map_err(invalid)
    }

    fn outline_args(outline: &str) -> Result<crate::segment::OutlinePad> {
        if outline.trim().is_empty() { Ok(Default::default()) } else { serde_json::from_str(outline).map_err(invalid) }
    }

    fn edit_args(base: &str, op: &str) -> Result<(Option<Grain>, crate::segment::edit::Op)> {
        let base: Option<Grain> = if base.trim().is_empty() { None } else { serde_json::from_str(base).map_err(invalid)? };
        let op = serde_json::from_value(serde_json::Value::String(op.to_string())).map_err(invalid)?;
        Ok((base, op))
    }

    pub fn export_grains(&self, area_id: &str, grains: &str) -> Result<String> {
        let gs: Vec<Grain> = serde_json::from_str(grains).map_err(invalid)?;
        let rec = self.rec(area_id)?;
        Ok(crate::segment::grains_csv(&gs, |x, y| rec.stage_of(x, y), |_| 0))
    }

    /// Pure checks of a full saved record; does not access or mutate this session.
    pub fn check_project(&self, project: &str, labels: &str) -> Result<String> {
        crate::checks::check_json(project, labels)
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

impl Session {
    /// Content hash of the opened package's image tiles ([`crate::atlas::PackageDigest`]): the
    /// hashes recorded while stitching, the other tiles read again from the source.
    pub fn package_digest(&self) -> Result<crate::atlas::PackageDigest> {
        let project = self.project.as_ref().ok_or_else(|| Error::Invalid("no project".into()))?;
        let mut entries = Vec::new();
        for rec in &project.areas {
            for t in &rec.tiles {
                let sha = if t.sha256.is_empty() {
                    let src = self.src.as_ref().ok_or_else(|| Error::Invalid("the package is not open".into()))?;
                    crate::util::sha256_hex(&src.read(&t.file)?)
                } else {
                    t.sha256.clone()
                };
                entries.push((t.file.clone(), sha));
            }
        }
        Ok(crate::atlas::PackageDigest::of(entries))
    }

    /// The files of the atlas bundle for a full project JSON (with `grainatlas_ui`) and
    /// [`crate::atlas::BundleOptions`] JSON; every stitched area is included.
    pub fn atlas_files(&self, full_project_json: &str, options: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let project = Project::from_json(full_project_json)?;
        let opts: crate::atlas::BundleOptions = if options.trim().is_empty() { Default::default() } else { serde_json::from_str(options).map_err(invalid)? };
        let digest = self.package_digest()?;
        let areas: Vec<crate::atlas::AreaPixels> = project
            .areas
            .iter()
            .filter_map(|rec| {
                let o = self.outputs.get(&rec.id)?;
                Some(crate::atlas::AreaPixels { rec, w: o.linear.w, h: o.linear.h, base: &o.base })
            })
            .collect();
        crate::atlas::build(&project, &areas, &digest, &opts)
    }

    /// The atlas bundle as one zip (`<project_id>.atlas/…`).
    pub fn atlas_zip(&self, full_project_json: &str, options: &str) -> Result<Vec<u8>> {
        let files = self.atlas_files(full_project_json, options)?;
        let id = Project::from_json(full_project_json)?.uuid.to_string();
        crate::atlas::zip(&id, &files)
    }

    /// Fixed identity for reproducible bundles (parity checks, test fixtures): the project gets
    /// `project_id`, its areas `project_id + 1, + 2, …`, `created` and the package path become
    /// fixed strings. Nothing else changes.
    pub fn pin_identity(&mut self, project_id: &str, created: &str) -> Result<()> {
        let id: uuid::Uuid = project_id.trim().parse().map_err(invalid)?;
        let project = self.project.as_mut().ok_or_else(|| Error::Invalid("no project".into()))?;
        project.uuid = id;
        for (i, a) in project.areas.iter_mut().enumerate() {
            a.uuid = uuid::Uuid::from_u128(id.as_u128().wrapping_add(i as u128 + 1));
        }
        project.created = created.to_string();
        project.package.path = project.package.name.clone();
        Ok(())
    }

    /// Stores the laser registration of an area (`AreaRegistration` JSON; "" removes it).
    pub fn set_laser(&mut self, area_id: &str, registration: &str) -> Result<()> {
        let project = self.project.as_mut().ok_or_else(|| Error::Invalid("no project".into()))?;
        if registration.trim().is_empty() {
            project.laser.areas.remove(area_id);
        } else {
            let reg: crate::laser::AreaRegistration = serde_json::from_str(registration).map_err(invalid)?;
            project.laser.areas.insert(area_id.to_string(), reg);
        }
        Ok(())
    }

    /// The spot list of one area for the laser (the area's adapter, or `adapter` when not empty).
    pub fn laser_export(&self, spots: &str, area_id: &str, adapter: &str) -> Result<String> {
        let spots: Vec<Spot> = serde_json::from_str(spots).map_err(invalid)?;
        let project = self.project.as_ref().ok_or_else(|| Error::Invalid("no project".into()))?;
        crate::laser::export(project, &spots, area_id, (!adapter.trim().is_empty()).then_some(adapter.trim()))
    }

    /// Records a publish (`Published` JSON) in the project's laicpmsdb link.
    pub fn record_published(&mut self, published: &str) -> Result<()> {
        let rec: super::Published = serde_json::from_str(published).map_err(invalid)?;
        let project = self.project.as_mut().ok_or_else(|| Error::Invalid("no project".into()))?;
        project.laicpmsdb.published.push(rec);
        Ok(())
    }
}

/// Laser registration from fiducials (`[Fiducial]` JSON) in stage `units` ("mm" or "um").
pub fn laser_fit(fiducials: &str, units: &str) -> Result<String> {
    let f: Vec<crate::laser::Fiducial> = serde_json::from_str(fiducials).map_err(invalid)?;
    serde_json::to_string(&crate::laser::fit(&f, units)?).map_err(invalid)
}

/// The laser adapters: `[{"id","label","extension"}]`.
pub fn laser_adapters() -> String {
    serde_json::json!(crate::laser::adapters().iter().map(|a| serde_json::json!({ "id": a.id(), "label": a.label(), "extension": a.extension() })).collect::<Vec<_>>()).to_string()
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
