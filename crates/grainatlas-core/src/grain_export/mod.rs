//! Bounded grain export: begin, measurement batches, individual image batches, finish.
//! No crops survive a batch. Hosts write each returned file before requesting another.
mod measure;
pub(crate) mod pixels;
mod stats;
pub mod synthetic;
mod table;
#[cfg(test)]
mod tests;

use crate::{
    Ctl, Error, Result,
    project::{AreaRecord, Project, session::Session},
    segment::Grain,
    spots::Spot,
};
use pixels::{Pixels, raster, source};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub use stats::{Relative, Stats};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PixelSource {
    #[default]
    Original,
    Displayed,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    #[default]
    Png,
    Tiff,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub pixel_values: PixelSource,
    pub format: ImageFormat,
    pub margin_um: f64,
    pub include_masks: bool,
    pub include_crops: bool,
    /// None = whole mount; a list of UUIDs = selection (including an empty selection).
    pub selected: Option<Vec<uuid::Uuid>>,
    /// Limits output only; all mount grains still form reference sets.
    pub area_id: Option<String>,
    pub exported: String,
    pub display_tone: BTreeMap<String, crate::photometric::ToneParams>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            pixel_values: PixelSource::Original,
            format: ImageFormat::Png,
            margin_um: 10.0,
            include_masks: true,
            include_crops: true,
            selected: None,
            area_id: None,
            exported: String::new(),
            display_tone: BTreeMap::new(),
        }
    }
}
pub type File = (String, Vec<u8>);
#[derive(Serialize)]
pub struct Batch {
    pub done: bool,
    pub files: Vec<File>,
    pub stage: String,
    pub fraction: f32,
}

#[derive(Clone)]
struct Row {
    grain: Grain,
    source_area: String,
    area: String,
    selected: bool,
    reference: bool,
    tile: Option<usize>,
    stats: Stats,
    pixel_source: String,
    origin: [usize; 2],
    crop: String,
    mask: String,
    image_stem: String,
}

pub struct Export {
    project: Project,
    options: Options,
    spots: Vec<Spot>,
    rows: Vec<Row>,
    area_ids: Vec<String>,
    area_at: usize,
    row_at: usize,
    crops: bool,
    ready: bool,
    manifests: Vec<Value>,
    digest: crate::atlas::PackageDigest,
    skipped_layers: Vec<Value>,
    refs: BTreeMap<String, [stats::Reference; 2]>,
    mount_partition: BTreeMap<String, String>,
    signals_known: bool,
    buffers: BTreeMap<String, (usize, usize, String)>,
}
fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Invalid(e.to_string())
}

/// Only accept stage maps that agree across every real tile within 0.1 pixel.
/// Piecewise nearest-tile stage maps can jump at seams; those are not registrations.
pub(crate) fn affine(a: &AreaRecord) -> Option<[f64; 2]> {
    let p = a.placement.as_ref()?;
    if p.tiles.is_empty() || a.stage_sign.iter().any(|s| s.abs() != 1.0) {
        return None;
    }
    let k = a.um_per_px();
    let mut offset: Option<[f64; 2]> = None;
    for (t, r) in p.tiles.iter().zip(&a.tiles) {
        if t.failed {
            continue;
        }
        let st = r.stage_nm?;
        let o = std::array::from_fn(|i| {
            st[i] / 1000.0
                - a.stage_sign[i] * (t.pos[i] + if i == 0 { t.w } else { t.h } as f64 / 2.0) * k
        });
        if let Some(b) = offset {
            if (o[0] - b[0]).hypot(o[1] - b[1]) > k * 0.1 {
                return None;
            }
        } else {
            offset = Some(o);
        }
    }
    if p.tiles.len() != a.tiles.len() {
        return None;
    }
    offset
}
fn mapped(
    g: &Grain,
    from: &AreaRecord,
    to: &AreaRecord,
    a: [f64; 2],
    b: [f64; 2],
    size: [usize; 2],
) -> Option<Grain> {
    let map = |p: [f64; 2]| {
        std::array::from_fn(|i| (a[i] + from.stage_sign[i] * p[i] - b[i]) / to.stage_sign[i])
    };
    let corners = [
        map([g.bbox_um[0], g.bbox_um[1]]),
        map([g.bbox_um[2], g.bbox_um[3]]),
    ];
    if (0..2).any(|i| {
        corners[0][i].min(corners[1][i]) < 0.0
            || corners[0][i].max(corners[1][i]) > size[i] as f64 * to.um_per_px()
    }) {
        return None;
    }
    let mut out = g.clone();
    out.polygon_um = g.polygon_um.iter().copied().map(map).collect();
    out.centroid_um = map(g.centroid_um);
    out.bbox_um = out.polygon_um.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |r, p| {
            [
                r[0].min(p[0]),
                r[1].min(p[1]),
                r[2].max(p[0]),
                r[3].max(p[1]),
            ]
        },
    );
    out.orientation_deg = (g.orientation_deg.to_radians().sin() * from.stage_sign[1]
        / to.stage_sign[1])
        .atan2(g.orientation_deg.to_radians().cos() * from.stage_sign[0] / to.stage_sign[0])
        .to_degrees();
    if out.orientation_deg > 90.0 {
        out.orientation_deg -= 180.0;
    } else if out.orientation_deg < -90.0 {
        out.orientation_deg += 180.0;
    }
    Some(out)
}
impl Export {
    pub fn begin(session: &Session, full_project: &str, options: &str) -> Result<Self> {
        let mut project = Project::from_json(full_project)?;
        // Buffer provenance comes from the live core, while current segmentation
        // controls and the polygon edits come from the UI snapshot.
        if let Some(live) = &session.project {
            project.package.meta = live.package.meta.clone();
            for area in &mut project.areas {
                if let Some(actual) = live.areas.iter().find(|a| a.id == area.id) {
                    let segment = area.params.segment.clone();
                    area.params = actual.params.clone();
                    area.params.segment = segment;
                    area.pixel_size_nm = actual.pixel_size_nm;
                    area.stage_sign = actual.stage_sign;
                    area.tiles = actual.tiles.clone();
                    area.area_meta = actual.area_meta.clone();
                    area.placement = actual.placement.clone();
                    area.photometric = actual.photometric.clone();
                }
            }
        }
        for area in &project.areas {
            if !area.um_per_px().is_finite() || area.um_per_px() <= 0.0 {
                return Err(invalid("area pixel size must be positive and finite"));
            }
        }
        if session.project.as_ref().map(|p| p.uuid) != Some(project.uuid) {
            return Err(invalid("export project differs from the open session"));
        }
        let options: Options = serde_json::from_str(options).map_err(invalid)?;
        if !options.margin_um.is_finite() || options.margin_um < 0.0 {
            return Err(invalid(
                "margin must be a finite number of micrometres, at least zero",
            ));
        }
        let ui = project
            .extra
            .get("grainatlas_ui")
            .cloned()
            .unwrap_or(json!({}));
        let grains: BTreeMap<String, Vec<Grain>> =
            serde_json::from_value(ui.get("grains").cloned().unwrap_or(json!({})))
                .map_err(invalid)?;
        let spots = serde_json::from_value(ui.get("spots").cloned().unwrap_or(json!([])))
            .map_err(invalid)?;
        let chosen: Option<BTreeSet<_>> = options
            .selected
            .as_ref()
            .map(|v| v.iter().copied().collect());
        let mut rows = Vec::new();
        let mut skipped_layers:Vec<Value> = project.areas.iter().filter(|a|!session.outputs.contains_key(&a.id)).map(|a|json!({"area_id":a.id,"reason":"Area pixels are not loaded; load its mosaic to export it as an additional layer."})).collect();
        let mut ids = BTreeSet::new();
        let transforms: BTreeMap<_, _> = project
            .areas
            .iter()
            .map(|a| (a.id.clone(), affine(a)))
            .collect();
        let mut skipped_pairs = BTreeSet::new();
        for a in &project.areas {
            for g in grains
                .get(&a.id)
                .into_iter()
                .flatten()
                .filter(|g| g.artifact.is_none())
            {
                if !ids.insert(g.id) {
                    return Err(invalid("grain UUIDs must be unique across the mount"));
                }
                if g.polygon_um.len() < 3
                    || !g.polygon_um.iter().flatten().all(|v| v.is_finite())
                    || !g.centroid_um.iter().all(|v| v.is_finite())
                {
                    return Err(invalid("grain polygon or centroid is invalid"));
                }
                let mut g = g.clone();
                g.bbox_um = g.polygon_um.iter().fold(
                    [
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ],
                    |b, p| {
                        [
                            b[0].min(p[0]),
                            b[1].min(p[1]),
                            b[2].max(p[0]),
                            b[3].max(p[1]),
                        ]
                    },
                );
                let selected = options.area_id.as_ref().is_none_or(|id| id == &a.id)
                    && chosen.as_ref().is_none_or(|s| s.contains(&g.id));
                // A missing reference area must be reported, never silently reduce the population.
                if !session.outputs.contains_key(&a.id) {
                    return Err(invalid(format!(
                        "{} has grains but no mosaic; load its pixels before exporting",
                        a.name
                    )));
                }
                rows.push(Row {
                    grain: g.clone(),
                    source_area: a.id.clone(),
                    area: a.id.clone(),
                    selected,
                    reference: true,
                    tile: None,
                    stats: Stats::default(),
                    pixel_source: String::new(),
                    origin: [0; 2],
                    crop: String::new(),
                    image_stem: String::new(),
                    mask: String::new(),
                });
                if !selected {
                    continue;
                }
                for target in project
                    .areas
                    .iter()
                    .filter(|t| t.id != a.id && session.outputs.contains_key(&t.id))
                {
                    let (Some(from), Some(to)) = (transforms[&a.id], transforms[&target.id]) else {
                        if skipped_pairs.insert((a.id.clone(), target.id.clone())) {
                            skipped_layers.push(json!({"source_area_id":a.id,"area_id":target.id,"reason":"No consistent stage transform across real tiles (0.1 pixel tolerance)."}));
                        }
                        continue;
                    };
                    let out = &session.outputs[&target.id];
                    let Some(mg) = mapped(&g, a, target, from, to, [out.linear.w, out.linear.h])
                    else {
                        continue;
                    };
                    rows.push(Row {
                        grain: mg,
                        source_area: a.id.clone(),
                        area: target.id.clone(),
                        selected: true,
                        reference: false,
                        tile: None,
                        stats: Stats::default(),
                        pixel_source: String::new(),
                        origin: [0; 2],
                        crop: String::new(),
                        image_stem: String::new(),
                        mask: String::new(),
                    });
                }
            }
        }
        if !rows.iter().any(|r| r.selected) {
            return Err(invalid("no non-artifact grains in this export scope"));
        }
        rows.sort_by(|a, b| {
            (&a.area, !a.reference, a.grain.id).cmp(&(&b.area, !b.reference, b.grain.id))
        });
        // Reserve names once in stable row order, shared by crops and masks.
        // Case-fold reservations too: exported folders may be case-insensitive.
        let mut used = BTreeSet::new();
        let mut counters = BTreeMap::new();
        for row in rows.iter_mut().filter(|r| r.selected) {
            let own = project
                .areas
                .iter()
                .find(|a| a.id == row.source_area)
                .unwrap();
            let target = project.areas.iter().find(|a| a.id == row.area).unwrap();
            let mut stem = format!(
                "{}-{}-{}",
                safe(&own.name),
                safe(&grain_label(&row.grain)),
                short_id(row.grain.id)
            );
            if row.area != row.source_area {
                stem.push('-');
                stem.push_str(&safe(&target.name));
            }
            row.image_stem = unique_stem(stem, &mut used, &mut counters);
        }
        let area_ids = rows
            .iter()
            .map(|r| r.area.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let signal = |a: &AreaRecord| {
            crate::segment::signal_from_meta(&a.area_meta)
                .or_else(|| crate::segment::signal_from_meta(&project.package.meta))
        };
        let reference_ids: BTreeSet<_> = rows
            .iter()
            .filter(|r| r.reference)
            .map(|r| r.area.as_str())
            .collect();
        let signals_known = project
            .areas
            .iter()
            .filter(|a| reference_ids.contains(a.id.as_str()))
            .all(|a| signal(a).is_some());
        let mount_partition = project
            .areas
            .iter()
            .map(|a| {
                (
                    a.id.clone(),
                    if signals_known {
                        signal(a)
                            .map(|s| s.label())
                            .unwrap_or("unknown")
                            .to_string()
                    } else {
                        "all-areas".into()
                    },
                )
            })
            .collect();
        project.extra.remove("grainatlas_ui");
        Ok(Self {
            digest: session.package_digest()?,
            project,
            options,
            spots,
            rows,
            area_ids,
            area_at: 0,
            row_at: 0,
            crops: false,
            ready: false,
            manifests: Vec::new(),
            skipped_layers,
            refs: BTreeMap::new(),
            mount_partition,
            signals_known,
            buffers: session
                .outputs
                .iter()
                .map(|(id, o)| {
                    (
                        id.clone(),
                        (o.linear.w, o.linear.h, o.mosaic_json.sha256.clone()),
                    )
                })
                .collect(),
        })
    }
    fn area(&self, id: &str) -> &AreaRecord {
        self.project
            .areas
            .iter()
            .find(|a| a.id == id)
            .expect("validated area")
    }
    fn group(&self, r: &Row, scope: &str) -> String {
        match scope {
            "area" => format!("area:{}", r.area),
            "tile" => format!("tile:{}:{:?}", r.area, r.tile),
            _ => format!("mount:{}:{}", self.mount_partition[&r.area], r.pixel_source),
        }
    }
    fn references(&mut self) {
        let mut values: BTreeMap<String, [Vec<f64>; 2]> = BTreeMap::new();
        for r in self.rows.iter().filter(|r| r.reference) {
            for scope in ["area", "tile", "mount"] {
                if scope == "tile" && r.tile.is_none() {
                    continue;
                }
                let v = values.entry(self.group(r, scope)).or_default();
                if let Some(x) = r.stats.mean {
                    v[0].push(x);
                }
                if let Some(x) = r.stats.median {
                    v[1].push(x);
                }
            }
        }
        self.refs = values
            .into_iter()
            .map(|(k, v)| (k, v.map(stats::Reference::new)))
            .collect();
    }
    /// One bounded unit of work; the caller yields between calls and writes files immediately.
    pub fn is_ready(&self) -> bool {
        self.ready
    }
    pub fn abort(self, session: &mut Session) {
        session.trim_grain_export();
    }
    pub fn batch(&mut self, session: &mut Session, ctl: &Ctl) -> Result<Batch> {
        let result = self.batch_controlled(session, ctl);
        if result.is_err() || result.as_ref().is_ok_and(|b| b.done) {
            session.trim_grain_export();
        }
        result
    }
    fn batch_controlled(&mut self, session: &mut Session, ctl: &Ctl) -> Result<Batch> {
        ctl.check()?;
        if session.project.as_ref().map(|p| p.uuid) != Some(self.project.uuid) {
            return Err(invalid("the open project changed during export"));
        }
        if self.ready {
            return Err(invalid("export already finished; call finish once"));
        }
        let stage = if self.crops {
            "writing grain images"
        } else {
            "measuring grains"
        };
        let mut files = Vec::new();
        if self.area_at >= self.area_ids.len() {
            if self.crops {
                self.ready = true;
                return Ok(Batch {
                    done: true,
                    files,
                    stage: stage.into(),
                    fraction: 1.0,
                });
            }
            self.references();
            if !self.options.include_crops && !self.options.include_masks {
                self.ready = true;
                return Ok(Batch {
                    done: true,
                    files,
                    stage: stage.into(),
                    fraction: 1.0,
                });
            }
            self.crops = true;
            self.area_at = 0;
            self.row_at = 0;
        } else {
            let id = self.area_ids[self.area_at].clone();
            let opts = self.options.clone();
            let rec = self.area(&id).clone();
            let output = session
                .outputs
                .get(&id)
                .ok_or_else(|| invalid("an export mosaic was unloaded"))?;
            let actual = session
                .project
                .as_ref()
                .and_then(|p| p.areas.iter().find(|a| a.id == id))
                .ok_or_else(|| invalid("an export area was removed"))?;
            if self.buffers[&id]
                != (
                    output.linear.w,
                    output.linear.h,
                    output.mosaic_json.sha256.clone(),
                )
                || actual.params.stitch != rec.params.stitch
                || actual.params.photometric != rec.params.photometric
                || actual.params.blend != rec.params.blend
                || actual.params.tone != rec.params.tone
            {
                return Err(invalid(
                    "image parameters or pixels changed during export; export again after the update finishes",
                ));
            }
            let start = self.row_at;
            if !self.crops && start == 0 {
                let selected: Vec<_> = self
                    .rows
                    .iter()
                    .filter(|r| r.area == id && r.selected)
                    .map(|r| json!({"grain_id":r.grain.id,"polygon_um":r.grain.polygon_um}))
                    .collect();
                let polygons = serde_json::to_vec(&selected).map_err(invalid)?;
                let manifest=session.with_grain_export_area(&id,opts.pixel_values==PixelSource::Original,ctl,|o,published| {
                    let (px,kind)=source(o,published,&opts,opts.display_tone.get(&id).unwrap_or(&rec.params.tone));
                    Ok(json!({"area_id":id,"name":rec.name,"mount_reference_partition":self.mount_partition[&id],"pixel_size_um_per_px":rec.um_per_px(),"mosaic_size_px":[o.linear.w,o.linear.h],"pixel_source":kind,
                        "sampled_buffer_sha256":px.hash(ctl)?,"hashed_bytes":match px {Pixels::Float(_)=>"row-major IEEE-754 f32 little-endian, full mosaic, no header",Pixels::Byte(..)=>"row-major u8, full mosaic, no header"},
                        "corrections":if published.is_some() {"Only published 8-bit pixels exist. Earlier corrections and quantisation are baked in; original precision cannot be recovered. Displayed mode uses the current base."} else {"Decoded intensities, optional HDR exposure merge, solved tile placement, configured gain/offset and background leveling with manual overrides (the joint solve can include a smooth charging surface and additive beam-overlap steps; its recorded fallback may use gain/offset only), feather or multiband blending, clamped to 0..1. No additional flat-field calibration. Original mode has no tone or display LUT; displayed mode uses the toned base plus current display LUT."},
                        "source_tiles":rec.tiles,"sem_placement":if published.is_some(){None}else{Some(&o.placement)},"tile_index_basis":"Solved mosaic placement index; HDR exposure groups share one index. Published mosaics have no verified SEM placement.","parameters":rec.params,"current_display_tone":opts.display_tone.get(&id).unwrap_or(&rec.params.tone),"applied_photometric":rec.photometric,"stage_sign":rec.stage_sign,
                        "segmentation_fingerprint":{"parameters":rec.params.segment,"polygons_sha256":crate::util::sha256_hex(&polygons),"polygon_hash_bytes":"UTF-8 compact JSON array of grain_id and polygon_um, export row order","parameter_note":"Current segmentation controls; manual edits are captured by the polygon hash."},
                        "reference_grain_count":self.rows.iter().filter(|r|r.area==id&&r.reference).count(),"export_grain_count":selected.len(),"acquisition_metadata":acquisition(&rec,&self.project)}))
                })?;
                self.manifests.push(manifest);
            }
            let next = (start..self.rows.len())
                .find(|&i| self.rows[i].area == id && (!self.crops || self.rows[i].selected));
            if let Some(i) = next {
                if !self.crops {
                    let end = measure::batch_end(&self.rows, i, &rec);
                    let measured = session.with_grain_export_area(
                        &id,
                        opts.pixel_values == PixelSource::Original,
                        ctl,
                        |o, published| {
                            crate::util::par_map(end - i, |j| {
                                measure::one(
                                    self.rows[i + j].clone(),
                                    &rec,
                                    o,
                                    published,
                                    &opts,
                                    ctl,
                                )
                            })
                            .into_iter()
                            .collect::<Result<Vec<_>>>()
                        },
                    )?;
                    for (j, (row, complete)) in measured.into_iter().enumerate() {
                        if !complete {
                            self.skipped_layers.push(json!({"grain_id":row.grain.id,"area_id":id,"reason":"Target polygon has no covered pixels or crosses uncovered pixels."}));
                        }
                        self.rows[i + j] = row;
                    }
                    self.row_at = end;
                } else {
                    let row = &mut self.rows[i];
                    // One crop and its mask at a time; no crop collection in the core.
                    session.with_grain_export_area(
                        &id,
                        opts.pixel_values == PixelSource::Original,
                        ctl,
                        |o, published| {
                            let (px, _) = source(
                                o,
                                published,
                                &opts,
                                opts.display_tone.get(&id).unwrap_or(&rec.params.tone),
                            );
                            let ras = raster(
                                &row.grain,
                                rec.um_per_px(),
                                o.linear.w,
                                o.linear.h,
                                opts.margin_um,
                                &px,
                                ctl,
                            )?;
                            row.origin = [ras.x, ras.y];
                            if ras.w > 0 && ras.h > 0 {
                                let stem = &row.image_stem;
                                if opts.include_crops {
                                    row.crop = format!(
                                        "crops/{stem}.{}",
                                        if opts.format == ImageFormat::Tiff {
                                            "tiff"
                                        } else {
                                            "png"
                                        }
                                    );
                                    files.push((
                                        row.crop.clone(),
                                        pixels::encode(
                                            &ras,
                                            matches!(px, Pixels::Byte(..)),
                                            opts.format,
                                        )?,
                                    ));
                                }
                                ctl.check()?;
                                if opts.include_masks {
                                    row.mask = format!("masks/{stem}.png");
                                    files.push((
                                        row.mask.clone(),
                                        pixels::png(ras.w, ras.h, &ras.mask, false)?,
                                    ));
                                }
                            }
                            Ok(())
                        },
                    )?;
                    self.row_at = i + 1;
                }
            } else {
                self.area_at += 1;
                self.row_at = 0;
                session.trim_grain_export();
            }
        }
        let within = if self.area_at < self.area_ids.len() {
            let id = &self.area_ids[self.area_at];
            let begin = self.rows.partition_point(|r| r.area.as_str() < id.as_str());
            let end = self
                .rows
                .partition_point(|r| r.area.as_str() <= id.as_str());
            self.row_at.saturating_sub(begin) as f32 / (end - begin).max(1) as f32
        } else {
            0.0
        };
        let fraction = (if self.crops { 0.5 } else { 0.0 })
            + 0.5 * ((self.area_at as f32 + within) / self.area_ids.len().max(1) as f32);
        ctl.report(stage, fraction);
        Ok(Batch {
            done: false,
            files,
            stage: stage.into(),
            fraction,
        })
    }
    pub fn finish(mut self, ctl: &Ctl) -> Result<Vec<File>> {
        ctl.check()?;
        if !self.ready {
            return Err(invalid("grain export batches are not complete"));
        }
        for manifest in &mut self.manifests {
            let id = manifest["area_id"].as_str().unwrap();
            let polygons: Vec<_> = self
                .rows
                .iter()
                .filter(|r| r.area == id && r.selected)
                .map(|r| json!({"grain_id":r.grain.id,"polygon_um":r.grain.polygon_um}))
                .collect();
            manifest["export_grain_count"] = json!(polygons.len());
            manifest["segmentation_fingerprint"]["polygons_sha256"] = json!(
                crate::util::sha256_hex(&serde_json::to_vec(&polygons).map_err(invalid)?)
            );
        }
        ctl.report("writing export metadata", 1.0);
        let columns = table::columns();
        let table = self.csv(ctl)?;
        let outlines = self.outlines(ctl)?;
        let manifest = json!({"schema_version":1,"app_version":env!("CARGO_PKG_VERSION"),"export_time":self.options.exported,"options":self.options,"project_uuid":self.project.uuid,"package_digest":self.digest,
            "intensity_scale":"Statistics always use 0..1 values, original f32 without quantisation or u8/255. Original images: round(clamp(value,0,1)*65535), divide by 65535 to recover the scale. Published/displayed images remain 8-bit.",
            "mount_reference_grouping":if self.signals_known {"known-signal"}else{"all-areas"},
            "mount_reference_policy":"Partition by metadata signal when every area contributing native reference grains has a known signal; otherwise pool all areas. Always partition by pixel_source to avoid mixing original and published/displayed values.",
            "layer_policy":"Own area plus fully covered areas with consistent stage transforms (0.1 pixel tolerance). Relative values use native non-artifact grains of the target layer area; mapped copies never enter reference sets.",
            "skipped_layers":self.skipped_layers,"areas":self.manifests,"columns":columns,"column_order":columns.iter().map(|v|v["name"].clone()).collect::<Vec<_>>()});
        Ok(vec![
            ("grains.csv".into(), table.into_bytes()),
            (
                "outlines.geojson".into(),
                serde_json::to_vec_pretty(&outlines).map_err(invalid)?,
            ),
            (
                "manifest.json".into(),
                serde_json::to_vec_pretty(&manifest).map_err(invalid)?,
            ),
            ("README.md".into(), table::readme().into_bytes()),
        ])
    }
}
/// Last eight hex digits: grain UUIDs are v7, whose leading digits are a
/// timestamp shared by every grain found in the same minute.
fn short_id(id: uuid::Uuid) -> String {
    id.simple().to_string()[24..].to_string()
}
fn unique_stem(
    stem: String,
    used: &mut BTreeSet<String>,
    counters: &mut BTreeMap<String, usize>,
) -> String {
    let key = stem.to_ascii_lowercase();
    let counter = counters.entry(key).or_insert(1);
    loop {
        let candidate = if *counter == 1 {
            stem.clone()
        } else {
            format!("{stem}-{counter}")
        };
        *counter += 1;
        if used.insert(candidate.to_ascii_lowercase()) {
            return candidate;
        }
    }
}
fn safe(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    if s.is_empty() { "grain".into() } else { s }
}
fn acquisition(a: &AreaRecord, project: &Project) -> BTreeMap<String, String> {
    project
        .package
        .meta
        .iter()
        .chain(a.area_meta.iter())
        .filter(|(k, _)| {
            let k = k.to_ascii_lowercase();
            k.contains("date")
                || k.contains("session")
                || k.contains("acquisition")
                || k.contains("time")
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn grain_label(g: &Grain) -> String {
    g.label
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| g.display_number())
}
