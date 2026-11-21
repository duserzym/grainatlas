//! Session calls for cross-area grain ownership and print sheets (Boise SEM
//! run, 2026-11-03). Kept apart from `session.rs`; same JSON-in, JSON-out style.

use std::collections::BTreeMap;

use super::{Session, invalid};
use crate::layout::sheets::{self, SheetArea, SheetMeta, SheetParams};
use crate::ownership::{self, OwnershipParams};
use crate::photometric::ToneParams;
use crate::photometric::enhance::{self, EnhanceParams};
use crate::segment::Grain;
use crate::project::image_edits::{printable_grains, oriented_grains};
use crate::{Error, Result};

fn parse<T: serde::de::DeserializeOwned + Default>(s: &str) -> Result<T> {
    if s.trim().is_empty() { Ok(T::default()) } else { serde_json::from_str(s).map_err(invalid) }
}

impl Session {
    /// Whole area on one sheet. WholeParams JSON and `{area_id: ToneParams}` JSON;
    /// returns `{pages, registration, stats}`. Only borrows the base mosaic, even
    /// after heavy-buffer eviction. Hosts turn pages into PDF with pdf_from_svgs.
    pub fn print_whole(&self, params_json: &str, tone_json: &str) -> Result<String> {
        self.print_whole_mode(params_json, tone_json, false)
    }
    pub fn print_whole_preview(&self, params: &str, tone: &str) -> Result<String> {
        self.print_whole_mode(params, tone, true)
    }
    fn print_whole_mode(&self, params_json: &str, tone_json: &str, preview: bool) -> Result<String> {
        let p: crate::layout::whole::WholeParams = parse(params_json)?;
        let tone: BTreeMap<String, ToneParams> = parse(tone_json)?;
        let project = self
            .project
            .as_ref()
            .ok_or_else(|| Error::Invalid("no project".into()))?;
        let rec = p.select_area(&project.areas)?;
        let o = self
            .outputs
            .get(&rec.id)
            .ok_or_else(|| Error::Invalid(format!("area {} has no stitched mosaic", rec.id)))?;
        let area = SheetArea {
            rec,
            img: print_image(project, rec, o, tone.get(&rec.id).unwrap_or(&rec.params.tone)),
        };
        let mut meta = self.meta()?;
        if let Ok(pkg) = self.pkg() {
            let mut info = pkg.info().clone();
            info.areas.retain(|a| a.id == rec.id);
            meta = sheets::sheet_meta(&info);
        }
        meta.areas = 1;
        meta.tiles = rec.tiles.len();
        meta.um_per_px = rec.um_per_px();
        let mut r = if preview {
            crate::layout::whole::whole_sheet_with_image(&p, &area, &meta, &|img, w, h| {
                let target = (img.w as f64 / w.min(1600) as f64).max(img.h as f64 / h.min(1600) as f64);
                let level = o.pyramid.levels.iter().find(|(i, _)| i.scale as f64 >= target).unwrap_or_else(|| o.pyramid.levels.last().unwrap());
                let img = crate::layout::AreaImage { w: level.0.w, h: level.0.h, px: &level.1, um_per_px: img.um_per_px * level.0.scale as f64, ..*img };
                sheets::band_png(&img, [0.0,0.0,img.display_size()[0] as f64*img.um_per_px,img.display_size()[1] as f64*img.um_per_px], &[], &EnhanceParams::default()).0
            })?
        } else { crate::layout::whole::whole_sheet(&p, &area, &meta)? };
        if project.image_edits.is_some() { r.registration.mapping="mount_um = band.paper_to_mount × [sheet_x_mm, sheet_y_mm, 1]; SVG affine order [a,b,c,d,e,f]. mosaic_rect_um is in the oriented display frame; image_crop is in mount µm".into(); }
        r.registration.project_uuid = Some(project.uuid);
        r.registration.project_path = self.project_path.clone();
        if preview { self.fallback_job(FullRequest::Whole { params: params_json.into(), tone: tone_json.into() }, serde_json::to_value(r).map_err(invalid)?) }
        else { serde_json::to_string(&r).map_err(invalid) }
    }

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
        self.print_sheets_with_edits(params, grains, tone, "")
    }

    /// Optional stored edit replay, preserving the existing print_sheets signature.
    pub fn print_sheets_with_edits(&self, params: &str, grains: &str, tone: &str, edits: &str) -> Result<String> {
        let p: SheetParams = parse(params)?;
        self.render_sheets(Some(p), None, edits, grains, tone, false)
    }

    /// Apply a fresh ordered edit list to the automatic plan, then render it.
    /// A PrintEdits object is also accepted for parameter-checked replay.
    pub fn print_plan_render(&self, plan_json: &str, edits_json: &str, grains_json: &str, tone_json: &str) -> Result<String> {
        let plan: sheets::SheetPlan = serde_json::from_str(plan_json).map_err(invalid)?;
        if plan.edit_summary.is_some() { return Err(Error::Invalid("expected an automatic plan".into())); }
        self.render_sheets(None, Some(plan), edits_json, grains_json, tone_json, false)
    }

    fn render_sheets(&self, params: Option<SheetParams>, plan: Option<sheets::SheetPlan>, edits: &str, grains_json: &str, tone_json: &str, preview: bool) -> Result<String> {
        let grains: BTreeMap<String, Vec<Grain>> = parse(grains_json)?;
        let tone: BTreeMap<String, ToneParams> = parse(tone_json)?;
        let project = self.project.as_ref().ok_or_else(|| Error::Invalid("no project".into()))?;
        let own = ownership::assign(&project.areas, &grains, &OwnershipParams::default());
        let grains: BTreeMap<String, Vec<Grain>> = project.areas.iter().map(|rec| {
            let gs=printable_grains(rec,grains.get(&rec.id).map_or(&[],Vec::as_slice),project.image_edits.as_ref().filter(|e| e.version==1).and_then(|e| e.areas.get(&rec.id)));
            let gs=if let Some(o)=self.outputs.get(&rec.id) { oriented_grains(&gs,print_image(project,rec,o,tone.get(&rec.id).unwrap_or(&rec.params.tone)).image_transform()) } else { gs };
            (rec.id.clone(),gs)
        }).collect();
        let areas: Vec<SheetArea> = project.areas.iter().filter_map(|rec| {
            let o = self.outputs.get(&rec.id)?;
            Some(SheetArea { rec, img: print_image(project, rec, o, tone.get(&rec.id).unwrap_or(&rec.params.tone)) })
        }).collect();
        if areas.is_empty() { return Err(Error::Invalid("no stitched area to print".into())); }
        let plan = match plan { Some(plan) => plan, None => sheets::plan_sheets(&params.expect("parameters or plan"), &areas, &grains, &own, &self.meta()?) };
        // Reject stale or malformed host plans before the renderer dereferences their ids.
        validate_plan(&plan, &areas, &grains)?;
        let edited = if edits.trim().is_empty() { sheets::apply_edits(&plan, &[], &areas, &grains) }
            else if edits.trim_start().starts_with('[') { sheets::apply_edits(&plan, &serde_json::from_str::<Vec<sheets::StripEdit>>(edits).map_err(invalid)?, &areas, &grains) }
            else { sheets::replay(&plan, &serde_json::from_str::<sheets::PrintEdits>(edits).map_err(invalid)?, &areas, &grains) };
        *self.sheet_images.last.lock().unwrap() = Some(LastSheet { params: edited.plan.params.clone(), grains: grains.clone() });
        let styles: Vec<_> = areas.iter().map(|a| image_style(&edited.plan.params.enhance, &a.img, grains.get(&a.rec.id).map_or(&[], Vec::as_slice))).collect();
        let mut r = sheets::render_plan_with_images(&edited.plan, &areas, &grains, &own, &|ai, rect| {
            let a = &areas[ai];
            if preview { return preview_band(a, rect, &self.outputs[&a.rec.id], edited.plan.um_per_mm, edited.plan.paper_mm); }
            self.sheet_images.image(&a.img, rect, &styles[ai], &self.outputs[&a.rec.id].base, grains.get(&a.rec.id).map_or(&[], Vec::as_slice), &edited.plan.params.enhance)
        });
        add_image_left_off(&mut r, project, grains_json)?;
        if project.image_edits.is_some() { r.registration.mapping="mount_um = band.paper_to_mount × [sheet_x_mm, sheet_y_mm, 1]; SVG affine order [a,b,c,d,e,f]. mosaic_rect_um is in the oriented display frame; image_crop is in mount µm".into(); }
        r.registration.project_uuid = Some(project.uuid);
        r.registration.project_path = self.project_path.clone();
        r.dropped = edited.dropped;
        let mut value = serde_json::to_value(&r).map_err(invalid)?;
        value["ownership"] = serde_json::to_value(&own).map_err(invalid)?;
        if preview {
            let id = self.sheet_images.sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            value["job_id"] = id.into();
            *self.sheet_images.job.lock().unwrap() = Some(PrintJob { id, next: 0, ready: false, sources: self.outputs.iter().map(|(id,o)| (id.clone(),std::sync::Arc::downgrade(&o.base))).collect(), styles: areas.iter().zip(styles).map(|(a,style)| (a.rec.id.clone(),style)).collect(), plan: Some(edited.plan), grains, tone, full: BTreeMap::new(), result: value.clone(), fallback: None, pdf: None });
        }
        serde_json::to_string(&value).map_err(invalid)
    }

    fn fallback_job(&self, request: FullRequest, mut result: serde_json::Value) -> Result<String> {
        let id = self.sheet_images.sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        result["job_id"] = id.into();
        *self.sheet_images.job.lock().unwrap() = Some(PrintJob { id, next: 0, ready: false, sources: self.outputs.iter().map(|(id,o)| (id.clone(),std::sync::Arc::downgrade(&o.base))).collect(), styles: BTreeMap::new(), plan: None, grains: BTreeMap::new(), tone: BTreeMap::new(), full: BTreeMap::new(), result: result.clone(), fallback: Some(request), pdf: None });
        serde_json::to_string(&result).map_err(invalid)
    }
    pub fn layout_preview(&self, params: &str, grains: &str, spots: &str, labels: &str, tone: &str) -> Result<String> {
        let p: crate::layout::LayoutParams = serde_json::from_str(params).map_err(invalid)?;
        let gs: BTreeMap<String,Vec<Grain>> = parse(grains)?; let ss: Vec<crate::spots::Spot> = parse(spots)?; let ls: Vec<crate::spots::Label> = parse(labels)?;
        let tones: BTreeMap<String,ToneParams> = parse(tone)?;
        let project = self.project.as_ref().ok_or_else(|| invalid("no project"))?;
        let areas: Vec<_> = project.areas.iter().filter_map(|rec| Some(SheetArea { rec, img: print_image(project, rec, self.outputs.get(&rec.id)?, tones.get(&rec.id).unwrap_or(&rec.params.tone)) })).collect();
        let imgs = areas.iter().map(|a| crate::layout::AreaImage { ..a.img }).collect::<Vec<_>>();
        let gs=areas.iter().map(|a| (a.rec.id.clone(),oriented_grains(&printable_grains(a.rec,gs.get(&a.rec.id).map_or(&[],Vec::as_slice),a.img.edits),a.img.image_transform()))).collect();
        let paper = p.page();
        let r = crate::layout::layout_with_images(&p, &imgs, &gs, &ss, &ls, &|a, rect, k| {
            let area = areas.iter().find(|v| v.rec.id == a.id).unwrap();
            preview_band(area, rect, &self.outputs[a.id], k, [paper.w_mm, paper.h_mm]).0
        });
        self.fallback_job(FullRequest::Layout { params: params.into(), tone: tone.into(), grains: grains.into(), spots: spots.into(), labels: labels.into() }, serde_json::to_value(r).map_err(invalid)?)
    }
    /// Interactive preview: identical planning, pyramid pixels, no export enhancement.
    pub fn print_sheets_preview(&self, params: &str, grains: &str, tone: &str, edits: &str) -> Result<String> {
        self.render_sheets(Some(parse(params)?), None, edits, grains, tone, true)
    }
    pub fn print_plan_preview(&self, plan: &str, edits: &str, grains: &str, tone: &str) -> Result<String> {
        let plan: sheets::SheetPlan = serde_json::from_str(plan).map_err(invalid)?;
        if plan.edit_summary.is_some() { return Err(invalid("expected an automatic plan")); }
        self.render_sheets(None, Some(plan), edits, grains, tone, true)
    }
    /// A bounded unit of background work. Hosts yield/check cancellation between pages.
    pub fn print_task(&self, id: usize, action: &str, page: usize) -> Result<Vec<u8>> {
        let mut guard = self.sheet_images.job.lock().unwrap();
        let job = guard.as_mut().filter(|j| j.id == id).ok_or_else(|| invalid("print job replaced; make sheets again"))?;
        if action == "cancel" { *guard = None; return Ok(b"{}".to_vec()); }
        if action == "pdf_abort" { job.pdf = None; return Ok(b"{}".to_vec()); }
        if !job.sources.iter().all(|(id, source)| self.outputs.get(id).is_some_and(|o| source.ptr_eq(&std::sync::Arc::downgrade(&o.base)))) { return Err(invalid("print source changed; make sheets again")); }
        if action == "quality" {
            let total = job.result["pages"].as_array().map_or(0, Vec::len);
            if page != job.next || page >= total { return Err(invalid("invalid print page order")); }
            if let Some(plan) = &job.plan {
                let project = self.project.as_ref().ok_or_else(|| invalid("no project"))?;
                let areas: Vec<_> = project.areas.iter().filter_map(|rec| Some(SheetArea { rec, img: print_image(project, rec, self.outputs.get(&rec.id)?, job.tone.get(&rec.id).unwrap_or(&rec.params.tone)) })).collect();
                let bands: Vec<_> = plan.strips.iter().filter(|s| !s.hidden && s.sheet == page).flat_map(|s| s.bands.iter().map(move |b| (&s.area, b.mosaic_rect_um))).collect();
                let images = crate::util::par_map(bands.len(), |i| {
                    let (id, rect) = bands[i]; let a = areas.iter().find(|a| &a.rec.id == id).unwrap();
                    let gs: &[Grain] = job.grains.get(id).map_or(&[], Vec::as_slice);
                    let image = self.sheet_images.image(&a.img, rect, &job.styles[id], &self.outputs[id].base, gs, &plan.params.enhance);
                    (band_key(id, rect), image)
                });
                job.full.extend(images);
                if page + 1 == plan.sheet_bodies.len() {
                    // Ownership was measured in the original mount frame, before display orientation.
                    let own: ownership::Ownership = serde_json::from_value(job.result["ownership"].clone()).map_err(invalid)?;
                    let mut r = sheets::render_plan_with_images(plan, &areas, &job.grains, &own, &|ai, rect| job.full[&band_key(&areas[ai].rec.id, rect)].clone());
                    if project.image_edits.is_some() { r.registration.mapping="mount_um = band.paper_to_mount × [sheet_x_mm, sheet_y_mm, 1]; SVG affine order [a,b,c,d,e,f]. mosaic_rect_um is in the oriented display frame; image_crop is in mount µm".into(); }
        r.registration.project_uuid = Some(project.uuid); r.registration.project_path = self.project_path.clone();
                    if let Some(off)=job.result.get("left_off").and_then(|v| v.as_array()) {
                        for value in off { let item: sheets::LeftOff=serde_json::from_value(value.clone()).map_err(invalid)?; if !r.left_off.iter().any(|o| o.grain.id==item.grain.id) { r.registration.left_off.push(item.clone()); r.left_off.push(item); } }
                        r.left_off_summary.cropped=r.left_off.iter().filter(|o| o.reason=="cropped").count();
                    }
                    r.dropped = job.result.get("dropped").map(|v| serde_json::from_value(v.clone()).map_err(invalid)).transpose()?.unwrap_or_default();
                    job.result = serde_json::to_value(r).map_err(invalid)?;
                    job.result["ownership"] = serde_json::to_value(own).map_err(invalid)?;
                    job.full.clear();
                }
            } else if let Some(request) = &job.fallback {
                let full = match request {
                    FullRequest::Whole { params, tone } => self.print_whole(params, tone)?,
                    FullRequest::Layout { params, tone, grains, spots, labels } => self.layout_page(params, grains, spots, labels, tone, Some(page))?,
                };
                let full: serde_json::Value = serde_json::from_str(&full).map_err(invalid)?;
                if matches!(request, FullRequest::Layout { .. }) { job.result["pages"][page] = full["pages"][0].clone(); }
                else { job.result = full; }
            }
            job.next += 1;
            job.ready = job.next == total;
            if job.ready { job.result.as_object_mut().unwrap().remove("job_id"); }
            let resident_bytes: usize = job.result["pages"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).map(str::len).sum::<usize>() + job.full.values().map(|v| v.0.len()).sum::<usize>();
            return serde_json::to_vec(&serde_json::json!({"ready":job.ready,"resident_bytes":resident_bytes})).map_err(invalid);
        }
        if !job.ready { return Err(invalid("print quality is not ready")); }
        if action == "summary" {
            return serde_json::to_vec(&serde_json::json!({"registration":job.result["registration"],"stats":job.result["stats"]})).map_err(invalid);
        }
        if action == "full" { return serde_json::to_vec(&job.result).map_err(invalid); }
        if action == "pdf_begin" {
            let mut pdf = crate::layout::pdf::PdfBuilder::new();
            if !job.result["registration"].is_null() { pdf.attach(job.result["registration"]["registration_file"].as_str().filter(|s| !s.is_empty()).unwrap_or("print_registration.json"), serde_json::to_vec_pretty(&job.result["registration"]).map_err(invalid)?); }
            job.pdf = Some(pdf);
        }
        else if action == "pdf_prepare" {
            let svg = job.result["pages"][page].as_str().ok_or_else(|| invalid("invalid PDF page"))?;
            let pdf = job.pdf.as_mut().ok_or_else(|| invalid("PDF not started"))?;
            if pdf.prepared() != page || !pdf.is_empty() { return Err(invalid("invalid PDF preparation order")); }
            pdf.prepare(svg)?;
        }
        else if action == "pdf_page" {
            let svg = job.result["pages"][page].as_str().ok_or_else(|| invalid("invalid PDF page"))?;
            let pdf = job.pdf.as_mut().ok_or_else(|| invalid("PDF not started"))?;
            if Some(pdf.prepared()) != job.result["pages"].as_array().map(Vec::len) { return Err(invalid("PDF font preparation incomplete")); }
            if pdf.len() != page { return Err(invalid("invalid PDF page order")); }
            pdf.push(svg)?;
        } else if action == "pdf_finish" {
            if job.pdf.as_ref().is_none_or(|p| Some(p.len()) != job.result["pages"].as_array().map(Vec::len)) { return Err(invalid("PDF pages incomplete")); }
            return Ok(job.pdf.take().unwrap().finish());
        }
        else { return Err(invalid("unknown print task")); }
        Ok(b"{}".to_vec())
    }

    /// Strip dragging uses the same reduced-resolution, unenhanced preview as pages.
    pub fn print_band_preview(&self, area_id: &str, rect: [f64;4], tone_json: &str) -> Result<Vec<u8>> {
        let tone: BTreeMap<String,ToneParams> = parse(tone_json)?;
        let project = self.project.as_ref().ok_or_else(|| invalid("no project"))?;
        let rec = self.rec(area_id)?; let output = self.out(area_id)?;
        let area = SheetArea { rec, img: print_image(project, rec, output, tone.get(area_id).unwrap_or(&rec.params.tone)) };
        let guard = self.sheet_images.job.lock().unwrap();
        let plan = guard.as_ref().and_then(|j| j.plan.as_ref()).ok_or_else(|| invalid("make sheets before editing strips"))?;
        Ok(preview_band(&area, rect, output, plan.um_per_mm, plan.paper_mm).0)
    }

    /// Pixels identical to the last sheet render for this rectangle and display tone.
    pub fn print_band_image(&self, area_id: &str, rect: [f64; 4], tone_json: &str) -> Result<Vec<u8>> {
        let tone: BTreeMap<String, ToneParams> = parse(tone_json)?;
        let project = self.project.as_ref().ok_or_else(|| invalid("no project"))?;
        let rec = self.rec(area_id)?;
        let o = self.out(area_id)?;
        let img = print_image(project, rec, o, tone.get(area_id).unwrap_or(&rec.params.tone));
        if rect.iter().any(|v| !v.is_finite()) || rect[0] < 0.0 || rect[1] < 0.0 || rect[2] > img.display_size()[0] as f64 * img.um_per_px || rect[3] > img.display_size()[1] as f64 * img.um_per_px || rect[0] >= rect[2] || rect[1] >= rect[3] { return Err(Error::Invalid("invalid band rectangle".into())); }
        let last = self.sheet_images.last.lock().unwrap();
        let defaults = LastSheet::default();
        let last = last.as_ref().unwrap_or(&defaults);
        let grains: &[Grain] = last.grains.get(area_id).map_or(&[], Vec::as_slice);
        Ok(self.sheet_images.image(&img, rect, &image_style(&last.params.enhance, &img, grains), &o.base, grains, &last.params.enhance).0)
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

// A Weak retains source allocation identity without keeping replaced mosaics alive.
// Its control block prevents pointer reuse until the cached entry is removed.
struct CachedBand { used: usize, area: String, source: std::sync::Weak<Vec<u8>>, image: (Vec<u8>, String) }
#[derive(Default)]
struct LastSheet { params: SheetParams, grains: BTreeMap<String, Vec<Grain>> }
pub(super) struct BandImageCache {
    images: std::sync::Mutex<BTreeMap<String, CachedBand>>,
    budget: usize,
    last: std::sync::Mutex<Option<LastSheet>>,
    sequence: std::sync::atomic::AtomicUsize,
    clock: std::sync::atomic::AtomicUsize,
    job: std::sync::Mutex<Option<PrintJob>>,
    #[cfg(test)]
    misses: std::sync::atomic::AtomicUsize,
}
impl Default for BandImageCache {
    fn default() -> Self {
        Self { images: Default::default(), budget: if cfg!(target_arch="wasm32") {256} else {512} * 1024 * 1024,
            last: Default::default(), sequence: Default::default(), clock: Default::default(), job: Default::default(),
            #[cfg(test)] misses: Default::default() }
    }
}
impl BandImageCache {
    pub(super) fn clear(&self) { self.images.lock().unwrap().clear(); *self.last.lock().unwrap() = None; *self.job.lock().unwrap() = None; }
    fn image(&self, img: &crate::layout::AreaImage, rect: [f64; 4], style: &str, source: &std::sync::Arc<Vec<u8>>, grains: &[Grain], enhance: &EnhanceParams) -> (Vec<u8>, String) {
        let key = format!("{}:{:p}:{}:{}:{}:{:?}:{}", img.id, std::sync::Arc::as_ptr(source), img.w, img.h, img.um_per_px.to_bits(), rect.map(f64::to_bits), style);
        let used = self.clock.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(entry) = self.images.lock().unwrap().get_mut(&key) { entry.used = used; return entry.image.clone(); }
        #[cfg(test)]
        self.misses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let image = sheets::band_png(img, rect, grains, enhance);
        let mut cache = self.images.lock().unwrap();
        cache.retain(|_, v| v.source.strong_count() > 0 && (v.area != img.id || v.source.ptr_eq(&std::sync::Arc::downgrade(source))));
        let budget = self.budget;
        if image.0.len() > budget { return image; }
        while cache.values().map(|v| v.image.0.len()).sum::<usize>() + image.0.len() > budget {
            let oldest = cache.iter().min_by_key(|(_, v)| v.used).map(|(k, _)| k.clone()).unwrap(); cache.remove(&oldest);
        }
        cache.insert(key, CachedBand { used, area: img.id.into(), source: std::sync::Arc::downgrade(source), image: image.clone() }); image
    }
}
fn image_style(enhance: &EnhanceParams, img: &crate::layout::AreaImage, grains: &[Grain]) -> String {
    let mut bytes = serde_json::to_vec(enhance).expect("enhancement serializes");
    bytes.extend_from_slice(&img.lut);
    bytes.extend(serde_json::to_vec(&img.edits).expect("edits serialize"));
    if enhance.strip { bytes.extend(serde_json::to_vec(grains).expect("grains serialize")); }
    crate::util::sha256_hex(&bytes)
}
fn validate_plan(plan: &sheets::SheetPlan, areas: &[SheetArea], grains: &BTreeMap<String, Vec<Grain>>) -> Result<()> {
    let valid = plan.um_per_mm.is_finite() && plan.um_per_mm > 0.0 && !plan.sheet_bodies.is_empty()
        && plan.targets.iter().all(|(id, ids)| areas.iter().any(|a| &a.rec.id == id) && ids.iter().all(|idg| grains.get(id).is_some_and(|gs| gs.iter().any(|g| &g.id == idg))))
        && areas.iter().all(|a| plan.targets.contains_key(&a.rec.id))
        && plan.sheet_bodies.iter().enumerate().all(|(i, b)| b.sheet == i + 1 && b.body_rect_mm.iter().all(|v| v.is_finite()))
        && plan.lines.as_ref().is_none_or(|lines| lines.iter().all(|ln| ln.sheet < plan.sheet_bodies.len() && ln.strips.iter().all(|i| *i < plan.strips.len())))
        && plan.strips.iter().all(|s| !s.hidden && s.key.area == s.area && s.sheet < plan.sheet_bodies.len() && s.origin_mm.iter().chain(&s.size_mm).all(|v| v.is_finite()) && plan.targets.get(&s.area).is_some_and(|ids| s.bands.iter().all(|b| b.key.area == s.area && b.mosaic_rect_um.iter().all(|v| v.is_finite()) && b.mosaic_rect_um[0] < b.mosaic_rect_um[2] && b.mosaic_rect_um[1] < b.mosaic_rect_um[3] && b.grains.iter().all(|id| ids.contains(id)))));
    if valid { Ok(()) } else { Err(Error::Invalid("invalid plan or source grains changed; make sheets again".into())) }
}

pub(super) fn print_image<'a>(project: &'a crate::project::Project, rec: &'a crate::project::AreaRecord, out: &'a super::pipeline::AreaOutput, tone: &ToneParams) -> crate::layout::AreaImage<'a> {
    let mut img=super::pipeline::area_image(rec,out,tone);
    img.source_size_um=Some([img.w as f64*img.um_per_px,img.h as f64*img.um_per_px]);
    img.edits=project.image_edits.as_ref().filter(|e| e.version==1).and_then(|e| e.areas.get(&rec.id)); img
}
fn add_image_left_off(r: &mut sheets::SheetResult, project: &crate::project::Project, source: &str) -> Result<()> {
    let grains: BTreeMap<String,Vec<Grain>>=parse(source)?;
    for area in &project.areas {
        let e=project.image_edits.as_ref().filter(|e| e.version==1).and_then(|e| e.areas.get(&area.id));
        for g in grains.get(&area.id).into_iter().flatten().filter(|g| e.is_some_and(|e| !e.visible(g.centroid_um))) {
            let off=sheets::LeftOff { grain: sheets::StrayGrain { area:area.id.clone(),id:g.id,number:g.number,label:g.label.clone(),eq_diameter_um:g.eq_diameter_um,centroid_um:g.centroid_um,bbox_um:g.bbox_um },reason:"cropped".into() };
            r.left_off.push(off.clone()); r.registration.left_off.push(off);
        }
    }
    r.left_off_summary.cropped=r.left_off.iter().filter(|o| o.reason=="cropped").count(); Ok(())
}

fn band_key(area: &str, rect: [f64; 4]) -> String { format!("{area}:{:?}", rect.map(f64::to_bits)) }
enum FullRequest {
    Whole { params: String, tone: String },
    Layout { params: String, tone: String, grains: String, spots: String, labels: String },
}
struct PrintJob {
    id: usize,
    next: usize,
    ready: bool,
    sources: Vec<(String, std::sync::Weak<Vec<u8>>)>,
    styles: BTreeMap<String,String>,
    plan: Option<sheets::SheetPlan>,
    grains: BTreeMap<String, Vec<Grain>>,
    tone: BTreeMap<String, ToneParams>,
    full: BTreeMap<String, (Vec<u8>, String)>,
    result: serde_json::Value,
    fallback: Option<FullRequest>,
    pdf: Option<crate::layout::pdf::PdfBuilder>,
}
fn preview_band(a: &SheetArea, rect: [f64; 4], output: &super::pipeline::AreaOutput, k: f64, paper: [f64; 2]) -> (Vec<u8>, String) {
    // At most 1600 pixels on a page's long side; choose the nearest coarser
    // resident level. No full-resolution crop or enhancement is performed.
    let target_u = k * paper[0].max(paper[1]) / 1600.0;
    let level = output.pyramid.levels.iter().find(|(i, _)| a.img.um_per_px * i.scale as f64 >= target_u).unwrap_or_else(|| output.pyramid.levels.last().unwrap());
    let img = crate::layout::AreaImage { w: level.0.w, h: level.0.h, px: &level.1, um_per_px: a.img.um_per_px * level.0.scale as f64, ..a.img };
    sheets::band_png(&img, rect, &[], &EnhanceParams::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn band_cache_evicts_the_least_recently_used_and_refuses_oversized_images() {
        let source = std::sync::Arc::new(vec![128; 64*64]);
        let img = crate::layout::AreaImage { source_size_um: None, edits: None, id:"a",name:"a",sample:"a",w:64,h:64,px:&source,lut:std::array::from_fn(|i| i as u8),um_per_px:1.0 };
        let e = EnhanceParams::default(); let style = image_style(&e,&img,&[]);
        let a = [0.0,0.0,16.0,16.0]; let b = [16.0,0.0,32.0,16.0]; let c = [32.0,0.0,48.0,16.0];
        let bytes = sheets::band_png(&img,a,&[],&e).0.len();
        let cache = BandImageCache { budget: bytes*2, ..Default::default() };
        cache.image(&img,a,&style,&source,&[],&e); cache.image(&img,b,&style,&source,&[],&e);
        cache.image(&img,a,&style,&source,&[],&e); cache.image(&img,c,&style,&source,&[],&e);
        assert_eq!(cache.images.lock().unwrap().len(),2);
        cache.image(&img,a,&style,&source,&[],&e);
        assert_eq!(cache.misses.load(std::sync::atomic::Ordering::Relaxed),3);
        cache.image(&img,b,&style,&source,&[],&e);
        assert_eq!(cache.misses.load(std::sync::atomic::Ordering::Relaxed),4);
        assert!(cache.images.lock().unwrap().values().map(|v| v.image.0.len()).sum::<usize>() <= bytes*2);
        let small = BandImageCache { budget: bytes-1, ..Default::default() };
        small.image(&img,a,&style,&source,&[],&e); assert!(small.images.lock().unwrap().is_empty());
    }
    #[test]
    fn band_images_reuse_enhancement_and_invalidate_source_tone_and_rectangle() {
        let cache = BandImageCache::default();
        let source = std::sync::Arc::new((0..64*64).map(|i| (i % 255) as u8).collect::<Vec<_>>());
        let mut img = crate::layout::AreaImage { source_size_um: None, edits: None, id: "A1", name: "zr-1", sample: "zr-1", w: 64, h: 64, px: &source, lut: std::array::from_fn(|i| i as u8), um_per_px: 1.0 };
        let e = EnhanceParams { denoise: true, sharpen: true, ..Default::default() };
        let rect = [0.0, 0.0, 50.0, 50.0];
        let style = image_style(&e, &img, &[]);
        let first = cache.image(&img, rect, &style, &source, &[], &e);
        assert_eq!(first, cache.image(&img, rect, &style, &source, &[], &e));
        assert_eq!(cache.misses.load(std::sync::atomic::Ordering::Relaxed), 1);
        let cropped = cache.image(&img, [0.0, 0.0, 40.0, 50.0], &style, &source, &[], &e); assert_ne!(first.0, cropped.0);
        img.lut = std::array::from_fn(|i| (255 - i) as u8);
        let retone = cache.image(&img, rect, &image_style(&e, &img, &[]), &source, &[], &e); assert_ne!(first.0, retone.0);
        let next = std::sync::Arc::new(vec![1; 64*64]); img.px = &next;
        let replaced = cache.image(&img, rect, &image_style(&e, &img, &[]), &next, &[], &e); assert_ne!(retone.0, replaced.0);
        assert_eq!(cache.misses.load(std::sync::atomic::Ordering::Relaxed), 4);
        assert_eq!(cache.images.lock().unwrap().len(), 1);
    }
}
