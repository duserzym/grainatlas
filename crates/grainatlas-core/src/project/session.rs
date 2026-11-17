//! A working session over one opened package: the project record, the
//! stitched areas in memory and the segmentation work images. The browser
//! (grainatlas-wasm) and the desktop app (Tauri commands) are thin wrappers
//! around this type, so both run the same code. JSON in, JSON or bytes out.
//!
//! Image buffers start with an 8-byte header: width and height as
//! little-endian u32.

mod sheets;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// Resident image allocations, in bytes. `pyramid` excludes its shared base level.
/// Package source bytes, metadata and temporary operation allocations are not included.
#[derive(Default, Debug, serde::Serialize)]
pub struct AreaMemory {
    pub base: usize,
    pub source: usize,
    pub pyramid: usize,
    pub linear: usize,
    pub coverage: usize,
    pub work: usize,
    pub small: usize,
    pub total: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct MemoryReport {
    pub areas: BTreeMap<String, AreaMemory>,
    pub total: usize,
    pub budget: usize,
}

pub const DEFAULT_HEAVY_BUDGET: usize = if cfg!(target_arch = "wasm32") {
    192 * 1024 * 1024
} else {
    512 * 1024 * 1024
};

// Shared by sessions, including sessions used on different native threads.
static HEAVY_USE: AtomicUsize = AtomicUsize::new(1);

pub struct Session {
    sheet_images: sheets::BandImageCache,
    last_heavy_use: usize,
    /// Number of attempted heavy rehydrations, for diagnostics and regression tests.
    pub rebuild_heavy_calls: usize,
    heavy_budget: usize,
    recent: VecDeque<String>,
    /// Files handed over one by one (browser) before `open`.
    pub mem: MemSource,
    src: Option<Arc<dyn Source>>,
    pkg: Option<Box<dyn Package>>,
    pub project: Option<Project>,
    /// Saved project location supplied by the native host for print provenance.
    pub project_path: String,
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
    bundle_sources: BTreeMap<String, Arc<Vec<u8>>>,
    bundle_digest: Option<crate::atlas::PackageDigest>,
}

impl Session {
    pub fn new(name: &str) -> Session {
        Session {
            sheet_images: Default::default(),
            last_heavy_use: 0,
            rebuild_heavy_calls: 0,
            heavy_budget: DEFAULT_HEAVY_BUDGET,
            recent: VecDeque::new(),
            mem: MemSource::new(name),
            src: None,
            pkg: None,
            project: None,
            project_path: String::new(),
            outputs: BTreeMap::new(),
            work: BTreeMap::new(),
            origin_files: Vec::new(),
            path: String::new(),
            small: BTreeMap::new(),
            grains: BTreeMap::new(),
            bundle_sources: BTreeMap::new(),
            bundle_digest: None,
        }
    }

    /// Budget shared by linear/coverage, segmentation work and preview inputs.
    /// An active operation may exceed it; finished heavy mosaics can always be evicted.
    pub fn set_memory_budget(&mut self, bytes: usize) {
        self.heavy_budget = bytes;
        self.trim(None);
    }

    pub fn memory_report(&self) -> MemoryReport {
        let mut areas: BTreeMap<String, AreaMemory> = self
            .project
            .as_ref()
            .map(|p| {
                p.areas
                    .iter()
                    .map(|a| (a.id.clone(), AreaMemory::default()))
                    .collect()
            })
            .unwrap_or_default();
        for (id, o) in &self.outputs {
            let a = areas.entry(id.clone()).or_default();
            a.base = o.base.capacity();
            a.source = self
                .bundle_sources
                .get(id)
                .filter(|px| !Arc::ptr_eq(px, &o.base))
                .map_or(0, |px| px.capacity());
            a.pyramid = o
                .pyramid
                .levels
                .iter()
                .skip(1)
                .map(|(_, px)| px.capacity())
                .sum();
            a.linear = o.linear.px.capacity() * size_of::<f32>();
            a.coverage = o.coverage.capacity() * size_of::<f32>();
        }
        for (id, (_, w)) in &self.work {
            let a = areas.entry(id.clone()).or_default();
            a.work = w.img.px.capacity() * size_of::<f32>() + w.cov.capacity() + w.tiles_um.capacity() * size_of::<[f64; 4]>();
        }
        for ((id, _), imgs) in &self.small {
            areas.entry(id.clone()).or_default().small += imgs
                .iter()
                .map(|g| g.px.capacity() * size_of::<f32>())
                .sum::<usize>();
        }
        for a in areas.values_mut() {
            a.total = a.source + a.base + a.pyramid + a.linear + a.coverage + a.work + a.small;
        }
        MemoryReport {
            total: areas.values().map(|a| a.total).sum(),
            areas,
            budget: self.heavy_budget,
        }
    }

    /// Resident linear, coverage, work-image and preview-input allocations, in bytes.
    pub fn evictable_bytes(&self) -> usize {
        self.memory_report()
            .areas
            .values()
            .map(|a| a.linear + a.coverage + a.work + a.small)
            .sum()
    }

    /// Evict LRU areas down to `bytes` (zero releases all evictable buffers).
    /// Keeps base/pyramid pixels, dimensions, placement and project metadata, and
    /// leaves the session's configured budget and last-use stamp unchanged.
    pub fn evict_to(&mut self, bytes: usize) {
        self.trim_to(bytes, None);
    }

    /// Monotonic heavy/work/preview-use stamp, comparable across sessions.
    /// Zero means no recorded area use yet. Eviction and pyramid/PNG reads do not touch it.
    pub fn last_heavy_use(&self) -> usize {
        self.last_heavy_use
    }

    fn touch(&mut self, id: &str) {
        self.last_heavy_use = HEAVY_USE.fetch_add(1, Ordering::Relaxed);
        self.recent.retain(|a| a != id);
        self.recent.push_back(id.to_string());
    }

    fn trim(&mut self, keep: Option<&str>) {
        self.trim_to(self.heavy_budget, keep);
    }

    fn trim_to(&mut self, bytes: usize, keep: Option<&str>) {
        let report = self.memory_report();
        let mut used: usize = report
            .areas
            .values()
            .map(|a| a.linear + a.coverage + a.work + a.small)
            .sum();
        for id in &self.recent {
            if used <= bytes {
                break;
            }
            if keep == Some(id.as_str()) {
                continue;
            }
            if let Some(a) = report.areas.get(id) {
                used -= a.linear + a.coverage + a.work + a.small;
            }
            if let Some(o) = self.outputs.get_mut(id) {
                o.linear.px = Vec::new();
                o.coverage = Vec::new();
            }
            self.work.remove(id);
            self.small.retain(|(a, _), _| a != id);
        }
    }

    /// Restore only the pre-tone mosaic; resident pixels and project metadata never change.
    fn ensure_heavy(&mut self, id: &str, ctl: &Ctl) -> Result<()> {
        ctl.check()?;
        self.touch(id);
        if self.out(id)?.linear.px.is_empty() {
            // Make room before allocating the replacement, including the oversized-area case.
            let needed = self.out(id)?.linear.w * self.out(id)?.linear.h * 8;
            let budget = self.heavy_budget;
            self.heavy_budget = budget.saturating_sub(needed);
            self.trim(Some(id));
            self.heavy_budget = budget;
            self.rebuild_heavy_calls += 1;
            ctl.report("rebuilding mosaic", 0.0);
            ctl.check()?;
            let rec = self.rec(id)?;
            let (linear, coverage) = if let Some(px) = self.bundle_sources.get(id) {
                let o = self.out(id)?;
                (
                    crate::image::Gray {
                        w: o.linear.w,
                        h: o.linear.h,
                        px: px.iter().map(|&v| v as f32 / 255.0).collect(),
                    },
                    vec![1.0; px.len()],
                )
            } else {
                let pkg = self.pkg()?;
            let ai = pkg
                .info()
                .areas
                .iter()
                .position(|a| a.id == id)
                .ok_or_else(|| Error::NotFound(id.into()))?;
                pipeline::rebuild_heavy(rec, pkg.tiles_ctl(ai, ctl)?, &self.out(id)?.placement,
                ctl,
            )?
            };
            let auto_tone = crate::photometric::tone::auto(
                &linear,
                Some(&coverage),
                rec.um_per_px(),
                &rec.params.tone,
            );
            let auto_photometric = rec.params.photometric.clone();
            let auto_blend = rec.params.blend.clone();
            let o = self.outputs.get_mut(id).expect("stitched");
            o.linear = linear;
            o.coverage = coverage;
            o.auto_tone = auto_tone;
            o.auto_photometric = auto_photometric;
            o.auto_blend = auto_blend;
            ctl.report("rebuilding mosaic", 1.0);
        }
        self.trim(Some(id));
        Ok(())
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
    pub fn open_source(&mut self, src: Arc<dyn Source>, path: &str, created: &str,
    ) -> Result<String> {
        let (probe, pkg) = readers::probe(src.clone())?;
        self.bundle_sources.clear();
        self.bundle_digest = None;
        self.outputs.clear();
        self.sheet_images.clear();
        self.recent.clear();
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
        self.bundle_sources.clear();
        self.bundle_digest = None;
        self.outputs.clear();
        self.sheet_images.clear();
        self.recent.clear();
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
            self.project_path.clear();
            self.project = Some(Project::new(info, &self.path, created));
        }
        if let Some(p) = self.project.as_mut() {
            for area in &info.areas {
                if let Some(rec) = p.area_mut(&area.id) {
                    rec.merge_metadata(area);
                }
            }
            p.package.kind = info.kind.clone();
            p.package.loose = loose;
            p.package.files = if info.kind == readers::kind::LOOSE { self.origin_files.clone() } else { Vec::new() };
        }
        self.pkg = Some(pkg);
    }

    pub fn load_project(&mut self, json: &str) -> Result<()> {
        let project = Project::from_json(json)?;
        if self.project.as_ref().is_none_or(|old| old.uuid != project.uuid) {
            self.project_path.clear();
        }
        self.project = Some(project);
        Ok(())
    }

    /// Load an already displayed mosaic without changing any raw package metadata.
    pub fn load_bundle_area(
        &mut self,
        area_id: &str,
        w: usize,
        h: usize,
        pixels: Vec<u8>,
    ) -> Result<String> {
        if w == 0 || h == 0 || w.checked_mul(h) != Some(pixels.len()) {
            return Err(invalid("published image dimensions do not match pixels"));
        }
        self.rec(area_id)?;
        // Release old heavy buffers before allocating this area's full-resolution f32 data.
        self.evict_to(0);
        let pixels = Arc::new(pixels);
        let out = pipeline::bundle_output(self.rec(area_id)?, w, h, pixels.clone());
        let json = serde_json::to_string(&out.mosaic_json).map_err(invalid)?;
        self.bundle_sources.insert(area_id.into(), pixels);
        self.outputs.insert(area_id.into(), out);
        self.work.remove(area_id);
        self.small.retain(|(a, _), _| a != area_id);
        self.touch(area_id);
        self.trim(None);
        Ok(json)
    }

    pub fn set_package_digest(&mut self, sha256: &str, files: usize) {
        self.bundle_digest = Some(crate::atlas::PackageDigest {
            sha256: sha256.into(),
            files,
        });
    }

    /// Rebase further tone edits on the image just published.
    pub fn bake_bundle_area(&mut self, area_id: &str, display_tone: &str) -> Result<String> {
        if !self.bundle_sources.contains_key(area_id) {
            return Err(invalid("not a bundle-backed area"));
        }
        let tone: ToneParams = serde_json::from_str(display_tone).map_err(invalid)?;
        let o = self.out(area_id)?;
        let lut = crate::photometric::tone::lut(&tone);
        let pixels = o.base.iter().map(|&v| lut[v as usize]).collect();
        self.load_bundle_area(area_id, o.linear.w, o.linear.h, pixels)
    }

    fn require_raw(&self, id: &str) -> Result<()> {
        if self.bundle_sources.contains_key(id) {
            return Err(invalid(
                "Stitching and shading correction need the raw package.",
            ));
        }
        Ok(())
    }

    /// Metadata signal for Auto, regardless of a saved segmentation override.
    pub fn area_signal(&self, area_id: &str) -> Result<String> {
        serde_json::to_string(&self.rec(area_id)?.signal()).map_err(invalid)
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
    pub fn stitch(&mut self, area_id: &str, params: &str, ctl: &Ctl, clock: Clock,
    ) -> Result<String> {
        self.require_raw(area_id)?;
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
        let tiles = pipeline::read_tiles_ctl(pkg, ai, rec, ctl)?;
        ctl.report("read", 1.0);
        ctl.check()?;
        timings.steps.push(("read".into(), clock() - t0));
        let grains = self.grains.get(area_id).map_or(&[][..], |g| g.as_slice());
        let out = pipeline::run_owned(rec, tiles, &params, grains, ctl, clock, &mut timings)?;
        let summary = serde_json::json!({
            "mosaic": out.mosaic_json,
            "placement": out.placement,
            "photometric": rec.photometric, "failed_tiles": rec.failed_tiles,
            "timings": timings.steps,
        });
        self.outputs.insert(area_id.to_string(), out);
        self.work.remove(area_id);
        self.small.retain(|(a, _), _| a != area_id);
        self.touch(area_id);
        self.trim(None);
        Ok(summary.to_string())
    }

    /// Applies new parameters to a stitched area with as little work as possible: tone only →
    /// re-tone; correction or blending → re-run from the solved placement (no re-stitch); stitch
    /// parameters → full stitch. Returns the same summary as `stitch`.
    pub fn recompute(&mut self, area_id: &str, params: &str, ctl: &Ctl, clock: Clock,
    ) -> Result<String> {
        self.require_raw(area_id)?;
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
            self.retone_with_ctl(area_id, &serde_json::to_string(&p.tone).map_err(invalid)?,
                ctl,
            )?;
            timings.steps.push(("tone".into(), clock() - t0));
        } else {
            let pkg = self.pkg.as_deref().ok_or_else(|| Error::Invalid("no package".into()))?;
            let ai = pkg.info().areas.iter().position(|a| a.id == area_id).ok_or_else(|| Error::NotFound(area_id.into()))?;
            let project = self.project.as_mut().ok_or_else(|| Error::Invalid("no project".into()))?;
            let rec = project.area_mut(area_id).expect("area");
            let t0 = clock();
            ctl.report("read", 0.0);
            let tiles = pipeline::read_tiles_ctl(pkg, ai, rec, ctl)?;
            let inputs = pipeline::inputs_owned(rec, tiles);
            ctl.report("read", 1.0);
            ctl.check()?;
            timings.steps.push(("read".into(), clock() - t0));
            let mut placement = rec.placement.clone().expect("placement");
            for (t, failed) in placement.tiles.iter_mut().zip(inputs.failed) { t.failed = failed; }
            let grains = self.grains.get(area_id).map_or(&[][..], |g| g.as_slice());
            let out = pipeline::finish(rec, inputs.imgs, placement, &p, grains, ctl, clock, &mut timings,
            )?;
            self.outputs.insert(area_id.to_string(), out);
            self.work.remove(area_id);
        }
        self.touch(area_id);
        self.trim(None);
        let rec = self.rec(area_id)?;
        let o = self.out(area_id)?;
        Ok(serde_json::json!({ "mosaic": o.mosaic_json, "placement": o.placement, "photometric": rec.photometric, "failed_tiles": rec.failed_tiles, "timings": timings.steps }).to_string())
    }

    /// A live preview of `params` on a stitched area, downsampled `2^level` times (an 8-byte
    /// w/h header, then 8-bit pixels): the whole mosaic at that pyramid level.
    pub fn preview(&mut self, area_id: &str, params: &str, level: usize) -> Result<Vec<u8>> {
        self.preview_with_ctl(area_id, params, level, &Ctl::NONE)
    }

    pub fn preview_with_ctl(
        &mut self,
        area_id: &str,
        params: &str,
        level: usize,
        ctl: &Ctl,
    ) -> Result<Vec<u8>> {
        let result = self.preview_controlled(area_id, params, level, ctl);
        self.trim(None);
        result
    }

    fn preview_controlled(
        &mut self,
        area_id: &str,
        params: &str,
        level: usize,
        ctl: &Ctl,
    ) -> Result<Vec<u8>> {
        let p: AreaParams = serde_json::from_str(params).map_err(invalid)?;
        let k = 1usize << level.min(12);
        let rec = self.rec(area_id)?.clone();
        if let Some(px) = self.bundle_sources.get(area_id) {
            // Downsample bytes first: previews never rehydrate the full f32 mosaic.
            let o = self.out(area_id)?;
            let w = o.linear.w.div_ceil(k);
            let h = o.linear.h.div_ceil(k);
            let mut values = Vec::with_capacity(w * h);
            for y in 0..h {
                for x in 0..w {
                    let mut sum = 0u64;
                    let mut n = 0u64;
                    for yy in y * k..((y + 1) * k).min(o.linear.h) {
                        for xx in x * k..((x + 1) * k).min(o.linear.w) {
                            sum += px[yy * o.linear.w + xx] as u64;
                            n += 1;
                        }
                    }
                    values.push(sum as f32 / n as f32 / 255.0);
                }
            }
            let lin = crate::image::Gray { w, h, px: values };
            let neutral = p.tone == ToneParams::neutral();
            let mut tone = p.tone;
            tone.clahe_tile = (tone.clahe_tile / k).max(8);
            let base = if neutral {
                lin.to_u8()
            } else {
                pipeline::tone_base_with(
                    &lin,
                    &vec![1.0; w * h],
                    &tone,
                    self.grains_of(area_id),
                    rec.um_per_px() * k as f64,
                )
            };
            return Ok(with_header(w, h, &base));
        }
        let placement = rec.placement.clone().ok_or_else(|| Error::Invalid(format!("{area_id} is not stitched")))?;
        self.touch(area_id);
        if rec.params.photometric == p.photometric && rec.params.blend == p.blend {
            self.ensure_heavy(area_id, ctl)?;
            let (w, h, px) = pipeline::preview_tone(
                self.out(area_id)?, &p.tone, k, self.grains_of(area_id), rec.um_per_px(),
            );
            self.trim(None);
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
        let (w, h, px) = pipeline::preview(&self.small[&key], &placement, &p, k, self.grains_of(area_id), rec.um_per_px(),
        );
        self.trim(None);
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
    pub fn auto_params(&mut self, area_id: &str, params: &str) -> Result<String> {
        self.auto_params_with_ctl(area_id, params, &Ctl::NONE)
    }

    pub fn auto_params_with_ctl(
        &mut self,
        area_id: &str,
        params: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let result = self.auto_params_controlled(area_id, params, ctl);
        self.trim(None);
        result
    }

    fn auto_params_controlled(&mut self, area_id: &str, params: &str, ctl: &Ctl) -> Result<String> {
        let mut p: AreaParams = serde_json::from_str(params).map_err(invalid)?;
        if self.bundle_sources.contains_key(area_id) {
            self.ensure_heavy(area_id, ctl)?;
            let o = self.out(area_id)?;
            p.tone = crate::photometric::tone::auto(
                &o.linear,
                Some(&o.coverage),
                self.rec(area_id)?.um_per_px(),
                &p.tone,
            );
            return serde_json::to_string(&p).map_err(invalid);
        }
        p.photometric.background = true;
        self.out(area_id)?;
        self.touch(area_id);
        let rec = self.rec(area_id)?.clone();
        let o = self.out(area_id)?;
        let mut tone = if o.auto_photometric == p.photometric &&o.auto_blend == p.blend {
            o.auto_tone.clone()
        } else {
            ctl.check()?;
            self.rebuild_heavy_calls += 1;
            let pkg = self.pkg()?;
            let ai = pkg.info().areas.iter().position(|a| a.id == area_id).ok_or_else(|| Error::NotFound(area_id.into()))?;
            let mut r = rec.clone();
            r.params = p.clone();
            let placement = r.placement.clone().ok_or_else(|| Error::Invalid(format!("{area_id} is not stitched")))?;
            ctl.report("rebuilding mosaic", 0.0);
            let rebuild_ctl = ctl;
            let (linear, coverage) = pipeline::rebuild_heavy(&r, pkg.tiles_ctl(ai, ctl)?, &placement, rebuild_ctl)?;
            let tone =
                crate::photometric::tone::auto(&linear, Some(&coverage), rec.um_per_px(), &p.tone);
            let o = self.outputs.get_mut(area_id).expect("stitched");
            o.auto_tone = tone.clone();
            o.auto_photometric = p.photometric.clone();
            o.auto_blend = p.blend.clone();
            tone
        };
        tone.grain_norm = p.tone.grain_norm.clone();
        p.tone = tone;
        self.trim(None);
        serde_json::to_string(&p).map_err(invalid)
    }

    fn out(&self, area_id: &str) -> Result<&AreaOutput> {
        self.outputs.get(area_id).ok_or_else(|| Error::Invalid(format!("{area_id} is not stitched")))
    }

    pub fn pyramid_tile(&self, area_id: &str, level: usize, col: usize, row: usize) -> Result<Vec<u8>> {
        let (w, h, px) = self.out(area_id)?.pyramid.tile(level, col, row).ok_or_else(|| Error::NotFound("tile".into()))?;
        Ok(with_header(w, h, &px))
    }

    pub fn histogram(&mut self, area_id: &str) -> Result<Vec<u32>> {
        // This read touches the LRU stamp for backwards-compatible usage ordering,
        // but never allocates heavy pixels.
        self.touch(area_id);
        Ok(self.out(area_id)?.histogram.clone())
    }

    pub fn retone(&mut self, area_id: &str, tone: &str) -> Result<String> {
        self.retone_with_ctl(area_id, tone, &Ctl::NONE)
    }

    pub fn retone_with_ctl(&mut self, area_id: &str, tone: &str, ctl: &Ctl) -> Result<String> {
        let result = self.retone_controlled(area_id, tone, ctl);
        self.trim(None);
        result
    }

    fn retone_controlled(&mut self, area_id: &str, tone: &str, ctl: &Ctl) -> Result<String> {
        let tone: ToneParams = serde_json::from_str(tone).map_err(invalid)?;
        if !self.bundle_sources.contains_key(area_id) || tone != ToneParams::neutral() {
            self.ensure_heavy(area_id, ctl)?;
        }
        let project = self.project.as_mut().ok_or_else(|| Error::Invalid("no project".into()))?;
        let rec = project.area_mut(area_id).ok_or_else(|| Error::NotFound(area_id.into()))?;
        let o = self.outputs.get_mut(area_id).ok_or_else(|| Error::Invalid("not stitched".into()))?;
        if let Some(source) = self.bundle_sources.get(area_id) {
            let base = if tone == ToneParams::neutral() {
                source.clone()
            } else {
                Arc::new(pipeline::tone_base_with(
                    &o.linear,
                    &o.coverage,
                    &tone,
                    self.grains.get(area_id).map_or(&[][..], |g| g.as_slice()),
                    rec.um_per_px(),
                ))
            };
            o.pyramid =
                crate::mosaic::pyramid::Pyramid::build_shared(o.linear.w, o.linear.h, base.clone());
            o.base = base;
            o.histogram = vec![0; 256];
            for &value in o.base.iter() { o.histogram[value as usize] += 1; }
            o.mosaic_json.sha256 = crate::util::sha256_hex(&o.base);
            o.mosaic_json.levels = o.pyramid.infos();
        } else {
            pipeline::retone(rec, o, &tone, self.grains.get(area_id).map_or(&[][..], |g| g.as_slice()),
        );
        }
        let json = serde_json::to_string(&o.mosaic_json).map_err(invalid);
        self.trim(None);
        json
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

    fn work_for(&mut self, area_id: &str, scale: usize, ctl: &Ctl) -> Result<&WorkImage> {
        self.touch(area_id);
        let fresh = !matches!(self.work.get(area_id), Some((s, _)) if *s == scale);
        if fresh {
            self.ensure_heavy(area_id, ctl)?;
            let w = pipeline::work_image(self.out(area_id)?, self.rec(area_id)?, scale);
            self.work.insert(area_id.to_string(), (scale, w));
        }
        self.trim(Some(area_id));
        // Work images suffice for segmentation: release the current linear buffers if needed.
        let report = self.memory_report();
        let used: usize = report
            .areas
            .values()
            .map(|a| a.linear + a.coverage + a.work + a.small)
            .sum();
        if used > self.heavy_budget {
            if let Some(o) = self.outputs.get_mut(area_id) {
                o.linear.px = Vec::new();
                o.coverage = Vec::new();
            }
            self.small.retain(|(a, _), _| a != area_id);
        }
        Ok(&self.work.get(area_id).expect("inserted").1)
    }

    fn seg_scale(&self, area_id: &str) -> usize {
        self.rec(area_id).map_or(2, |a| a.params.segment.work_scale)
    }

    /// The shared profile warning for the UI after segmentation, including the actual threshold.
    pub fn segment_warning(&mut self, area_id: &str, params: &str, grains: &str) -> Result<String> {
        self.segment_warning_with_ctl(area_id, params, grains, &Ctl::NONE)
    }

    pub fn segment_warning_with_ctl(
        &mut self,
        area_id: &str,
        params: &str,
        grains: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let result = self.segment_warning_controlled(area_id, params, grains, ctl);
        self.trim(None);
        result
    }

    fn segment_warning_controlled(
        &mut self,
        area_id: &str,
        params: &str,
        grains: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let p: SegmentParams = serde_json::from_str(params).map_err(invalid)?;
        let grains: Vec<Grain> = serde_json::from_str(grains).map_err(invalid)?;
        let w = self.work_for(area_id, p.work_scale, ctl)?;
        let warning = crate::segment::summary_warning(&grains, p.resolved_signal(w), area_id, crate::segment::grain_mask(w, &p).1,
        );
        serde_json::to_string(&warning).map_err(invalid)
    }

    pub fn segment(&mut self, area_id: &str, params: &str, previous: &str, outline: &str,
    ) -> Result<String> {
        self.segment_with_ctl(area_id, params, previous, outline, &Ctl::NONE)
    }

    pub fn segment_with_ctl(
        &mut self,
        area_id: &str,
        params: &str,
        previous: &str,
        outline: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let result = self.segment_controlled(area_id, params, previous, outline, ctl);
        self.trim(None);
        result
    }

    fn segment_controlled(
        &mut self,
        area_id: &str,
        params: &str,
        previous: &str,
        outline: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let p: SegmentParams = serde_json::from_str(params).map_err(invalid)?;
        let prev: Vec<Grain> = if previous.trim().is_empty() { Vec::new() } else { serde_json::from_str(previous).map_err(invalid)? };
        let outline = Self::outline_args(outline)?;
        let grains = crate::segment::segment_padded(self.work_for(area_id, p.work_scale, ctl)?, &p, &prev, outline,
        );
        if let Some(rec) = self.project.as_mut().and_then(|pr| pr.area_mut(area_id)) {
            rec.params.segment = p;
        }
        serde_json::to_string(&grains).map_err(invalid)
    }

    pub fn segment_missing(&mut self, area_id: &str, params: &str, existing: &str, outline: &str,
    ) -> Result<String> {
        self.segment_missing_with_ctl(area_id, params, existing, outline, &Ctl::NONE)
    }

    pub fn segment_missing_with_ctl(
        &mut self,
        area_id: &str,
        params: &str,
        existing: &str,
        outline: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let result = self.segment_missing_controlled(area_id, params, existing, outline, ctl);
        self.trim(None);
        result
    }

    fn segment_missing_controlled(
        &mut self,
        area_id: &str,
        params: &str,
        existing: &str,
        outline: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let p: SegmentParams = serde_json::from_str(params).map_err(invalid)?;
        let prev: Vec<Grain> = if existing.trim().is_empty() { Vec::new() } else { serde_json::from_str(existing).map_err(invalid)? };
        let outline = Self::outline_args(outline)?;
        let grains = crate::segment::add_missing(self.work_for(area_id, p.work_scale, ctl)?, &p, &prev, outline,
        );
        if let Some(rec) = self.project.as_mut().and_then(|pr| pr.area_mut(area_id)) {
            rec.params.segment = p;
        }
        serde_json::to_string(&grains).map_err(invalid)
    }

    pub fn grain_at(&mut self, area_id: &str, x_um: f64, y_um: f64, params: &str, outline: &str,
    ) -> Result<String> {
        self.grain_at_with_ctl(area_id, x_um, y_um, params, outline, &Ctl::NONE)
    }

    pub fn grain_at_with_ctl(
        &mut self,
        area_id: &str,
        x_um: f64,
        y_um: f64,
        params: &str,
        outline: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let result = self.grain_at_controlled(area_id, x_um, y_um, params, outline, ctl);
        self.trim(None);
        result
    }

    fn grain_at_controlled(
        &mut self,
        area_id: &str,
        x_um: f64,
        y_um: f64,
        params: &str,
        outline: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let p: SegmentParams = serde_json::from_str(params).map_err(invalid)?;
        let w = self.work_for(area_id, p.work_scale, ctl)?;
        let outline = Self::outline_args(outline)?;
        // Click-grow has no neighbouring labels in its API; clip plain growth to coverage.
        let grain = crate::segment::grain_at(w, &p, x_um, y_um).map(|g| {
            crate::segment::pad_outlines(w, &[g], outline, p.simplify_px, |_| true).remove(0)
        });
        serde_json::to_string(&grain).map_err(invalid)
    }

    pub fn split_grain(&mut self, area_id: &str, grain: &str, line: [f64; 4]) -> Result<String> {
        self.split_grain_with_ctl(area_id, grain, line, &Ctl::NONE)
    }

    pub fn split_grain_with_ctl(
        &mut self,
        area_id: &str,
        grain: &str,
        line: [f64; 4],
        ctl: &Ctl,
    ) -> Result<String> {
        let result = self.split_grain_controlled(area_id, grain, line, ctl);
        self.trim(None);
        result
    }

    fn split_grain_controlled(
        &mut self,
        area_id: &str,
        grain: &str,
        line: [f64; 4],
        ctl: &Ctl,
    ) -> Result<String> {
        let g: Grain = serde_json::from_str(grain).map_err(invalid)?;
        let params = self.rec(area_id)?.params.segment.clone();
        let w = self.work_for(area_id, params.work_scale, ctl)?;
        let signal = params.resolved_signal(w);
        serde_json::to_string(&crate::segment::split_grain_for_signal(w, &g, line, 0.8, signal,
        )).map_err(invalid)
    }

    pub fn merge_grains(&mut self, area_id: &str, grains: &str) -> Result<String> {
        self.merge_grains_with_ctl(area_id, grains, &Ctl::NONE)
    }

    pub fn merge_grains_with_ctl(
        &mut self,
        area_id: &str,
        grains: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let result = self.merge_grains_controlled(area_id, grains, ctl);
        self.trim(None);
        result
    }

    fn merge_grains_controlled(
        &mut self,
        area_id: &str,
        grains: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let gs: Vec<Grain> = serde_json::from_str(grains).map_err(invalid)?;
        let scale = self.seg_scale(area_id);
        let w = self.work_for(area_id, scale, ctl)?;
        serde_json::to_string(&crate::segment::merge_grains(w, &gs, 0.8)).map_err(invalid)
    }

    pub fn refine_grain(&mut self, area_id: &str, grain: &str) -> Result<String> {
        self.refine_grain_with_ctl(area_id, grain, &Ctl::NONE)
    }

    pub fn refine_grain_with_ctl(
        &mut self,
        area_id: &str,
        grain: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let result = self.refine_grain_controlled(area_id, grain, ctl);
        self.trim(None);
        result
    }

    fn refine_grain_controlled(&mut self, area_id: &str, grain: &str, ctl: &Ctl) -> Result<String> {
        let g: Grain = serde_json::from_str(grain).map_err(invalid)?;
        let scale = self.seg_scale(area_id);
        let w = self.work_for(area_id, scale, ctl)?;
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
    pub fn outline_grain(&mut self, area_id: &str, polygon: &str, base: &str, op: &str, outline: &str,
    ) -> Result<String> {
        self.outline_grain_with_ctl(area_id, polygon, base, op, outline, &Ctl::NONE)
    }

    pub fn outline_grain_with_ctl(
        &mut self,
        area_id: &str,
        polygon: &str,
        base: &str,
        op: &str,
        outline: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let result = self.outline_grain_controlled(area_id, polygon, base, op, outline, ctl);
        self.trim(None);
        result
    }

    fn outline_grain_controlled(
        &mut self,
        area_id: &str,
        polygon: &str,
        base: &str,
        op: &str,
        outline: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let poly: Vec<[f64; 2]> = serde_json::from_str(polygon).map_err(invalid)?;
        let (base, op) = Self::edit_args(base, op)?;
        let scale = self.seg_scale(area_id);
        let w = self.work_for(area_id, scale, ctl)?;
        let outline = Self::outline_args(outline)?;
        serde_json::to_string(&crate::segment::edit::outline_grain_padded(w, &poly, base.as_ref(), op, 0.8, outline,
        )).map_err(invalid)
    }

    /// A grain from a brush stroke (um, diameter `size_um`) grown to the nearby edges.
    pub fn brush_grain(&mut self, area_id: &str, stroke: &str, size_um: f64, base: &str, op: &str,
    ) -> Result<String> {
        self.brush_grain_with_ctl(area_id, stroke, size_um, base, op, &Ctl::NONE)
    }

    pub fn brush_grain_with_ctl(
        &mut self,
        area_id: &str,
        stroke: &str,
        size_um: f64,
        base: &str,
        op: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let result = self.brush_grain_controlled(area_id, stroke, size_um, base, op, ctl);
        self.trim(None);
        result
    }

    fn brush_grain_controlled(
        &mut self,
        area_id: &str,
        stroke: &str,
        size_um: f64,
        base: &str,
        op: &str,
        ctl: &Ctl,
    ) -> Result<String> {
        let stroke: Vec<[f64; 2]> = serde_json::from_str(stroke).map_err(invalid)?;
        let (base, op) = Self::edit_args(base, op)?;
        let scale = self.seg_scale(area_id);
        self.work_for(area_id, scale, ctl)?;
        let w = &self.work.get(area_id).expect("made above").1;
        let o = self.out(area_id)?;
        let v = crate::segment::edit::View { w: o.linear.w, h: o.linear.h, px: &o.base, um_per_px: self.rec(area_id)?.um_per_px(),
        };
        serde_json::to_string(&crate::segment::edit::brush_grain(w, &v, &stroke, size_um, base.as_ref(), op, 0.8,
        )).map_err(invalid)
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
        project.upbdb.mount_id = if mount_id.trim().is_empty() { None } else { Some(mount_id.trim().parse().map_err(invalid)?) };
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
    pub fn project_files(&mut self, full_project_json: &str) -> Result<Vec<(String, Vec<u8>)>> {
        self.project_files_with_ctl(full_project_json, &Ctl::NONE)
    }

    pub fn project_files_with_ctl(
        &mut self,
        full_project_json: &str,
        ctl: &Ctl,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let result = self.project_files_controlled(full_project_json, ctl);
        self.trim(None);
        result
    }

    fn project_files_controlled(
        &mut self,
        full_project_json: &str,
        ctl: &Ctl,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let v: serde_json::Value = serde_json::from_str(full_project_json).map_err(invalid)?;
        let mut files = vec![("project.json".to_string(), serde_json::to_vec_pretty(&v).map_err(invalid)?,
        )];
        let ui = &v["grainatlas_ui"];
        let spots = serde_json::json!({ "spots": ui["spots"], "labels": ui["labels"] });
        files.push(("spots.json".into(), serde_json::to_vec_pretty(&spots).map_err(invalid)?,
        ));
        let project = self.project.as_ref().ok_or_else(|| Error::Invalid("no project".into()))?;
        for rec in project.areas.clone() {
            let Some(o) = self.outputs.get(&rec.id) else { continue;
            };
            files.extend(pipeline::area_files(&rec, o));
            let grains: Vec<Grain> = serde_json::from_value(ui["grains"][&rec.id].clone()).unwrap_or_default();
            if !grains.is_empty() {
                self.ensure_heavy(&rec.id, ctl)?;
                files.extend(pipeline::mask_files(&rec, self.out(&rec.id)?, &grains));
                self.trim(None);
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
        if self.src.is_none() && let Some(digest) = &self.bundle_digest {
            return Ok(digest.clone());
        }
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
    pub fn atlas_files(&self, full_project_json: &str, options: &str,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let project = Project::from_json(full_project_json)?;
        let opts: crate::atlas::BundleOptions = if options.trim().is_empty() { Default::default() } else { serde_json::from_str(options).map_err(invalid)? };
        let digest = self.package_digest()?;
        let areas: Vec<crate::atlas::AreaPixels> = project
            .areas
            .iter()
            .filter_map(|rec| {
                let o = self.outputs.get(&rec.id)?;
                Some(crate::atlas::AreaPixels { rec, w: o.linear.w, h: o.linear.h, base: &o.base,
                    bundle_backed: self.bundle_sources.contains_key(&rec.id),
                })
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

    /// Records a publish (`Published` JSON) in the project's UPbDB link.
    pub fn record_published(&mut self, published: &str) -> Result<()> {
        let rec: super::Published = serde_json::from_str(published).map_err(invalid)?;
        let project = self.project.as_mut().ok_or_else(|| Error::Invalid("no project".into()))?;
        project.upbdb.published.push(rec);
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

#[cfg(test)]
mod memory_tests {
    use super::*;
    use crate::image::Gray;

    #[test]
    fn work_and_preview_inputs_share_the_budget_and_evict_lru_areas() {
        let mut s = Session::new("synthetic");
        for id in ["a", "b", "c"] {
            s.work.insert(
                id.into(),
                (
                    1,
                    WorkImage {
                        tiles_um: Vec::new(),
                        signal: crate::segment::Signal::Cl,
                        img: Gray {
                            w: 2,
                            h: 2,
                            px: vec![0.5; 4],
                        },
                        cov: vec![true; 4],
                        um_per_px: 1.0,
                    },
                ),
            );
            s.small.insert(
                (id.into(), 2),
                vec![Gray {
                    w: 2,
                    h: 2,
                    px: vec![0.5; 4],
                }],
            );
            s.touch(id);
        }
        s.touch("a"); // b is now oldest, then c, then a.
        let per_area = 4 * 4 + 4 + 4 * 4;
        assert_eq!(s.memory_report().total, 3 * per_area);
        s.set_memory_budget(2 * per_area);
        assert!(!s.work.contains_key("b"));
        assert!(!s.small.contains_key(&("b".into(), 2)));
        assert!(s.work.contains_key("a"));
        assert!(s.work.contains_key("c"));
        s.heavy_budget = 0;
        s.trim(Some("c")); // never evict the current area's work mid-operation.
        assert!(!s.work.contains_key("a"));
        assert!(s.work.contains_key("c"));
        s.trim(None);
        assert_eq!(s.memory_report().total, 0);
    }
}
