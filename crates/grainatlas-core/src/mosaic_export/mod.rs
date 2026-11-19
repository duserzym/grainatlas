//! Read-only, sequential export of stitched pixels. TIFF drains one output tile;
//! PNG and JPEG drain scanline strips. Hosts await each write before the next chunk.
mod raster;
#[cfg(test)]
mod tests;
pub mod tiff;
use crate::{
    Ctl, Error, Result,
    photometric::ToneParams,
    project::{AreaRecord, Project, session::Session},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
fn invalid(s: impl ToString) -> Error {
    Error::Invalid(s.to_string())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Values {
    Original,
    Displayed,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Tiff,
    Png,
    Jpg,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Tiff => "tiff",
            Self::Png => "png",
            Self::Jpg => "jpg",
        }
    }
    fn world(self) -> &'static str {
        match self {
            Self::Tiff => "tfw",
            Self::Png => "pgw",
            Self::Jpg => "jgw",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SizeChoice {
    #[default]
    Full,
    Downsample,
    Split,
}
fn sixteen() -> u16 {
    16
}
fn ninety() -> u8 {
    90
}
pub const RASTER_LIMIT: u32 = 65535;
pub fn fit_factor(w: u32, h: u32) -> u32 {
    let mut f = 1;
    while w.div_ceil(f) > RASTER_LIMIT || h.div_ceil(f) > RASTER_LIMIT {
        f *= 2;
    }
    f
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Options {
    pub area_ids: Vec<String>,
    pub whole_mount: bool,
    pub values: Values,
    #[serde(default)]
    pub display_tone: BTreeMap<String, ToneParams>,
    #[serde(default)]
    pub format: Format,
    #[serde(default = "sixteen")]
    pub png_bits: u16,
    #[serde(default = "ninety")]
    pub quality: u8,
    #[serde(default)]
    pub size_choice: SizeChoice,
    #[serde(default)]
    pub downsample: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImagePlan {
    pub name: String,
    #[serde(default)]
    pub format: Format,
    #[serde(default)]
    pub downsample: u32,
    #[serde(default)]
    pub source_origin: [u32; 2],
    #[serde(default)]
    pub limit_reason: Option<String>,
    pub area_ids: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u16,
    pub estimated_bytes: u64,
    pub big_tiff: bool,
    /// [a,b,c,d,e,f]: stage x=a*col+b*row+c, y=d*col+e*row+f.
    pub pixel_to_stage_um: Option<[f64; 6]>,
    pub pixel_size_um: [f64; 2],
    pub coordinate_reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub images: Vec<ImagePlan>,
    pub whole_mount_reason: Option<String>,
}
/// Filesystem-safe names on Windows and macOS. Identity is retained in JSON.
pub fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .collect();
    let s = s.trim_matches([' ', '.']);
    let s = if s.is_empty() { "mosaic" } else { s };
    let base = s.split('.').next().unwrap_or(s).to_ascii_uppercase();
    if [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&base.as_str())
    {
        format!("_{s}")
    } else {
        s.into()
    }
}
/// Keep compound filenames below filesystem component limits, including suffixes.
fn stem_part(name: &str) -> String {
    let s = safe_name(name);
    let mut n = 0;
    s.chars()
        .take_while(|c| {
            n += c.len_utf8();
            n <= 80
        })
        .collect()
}
pub fn affine(area: &AreaRecord) -> Option<[f64; 6]> {
    let o = crate::grain_export::affine(area)?;
    if o.iter().any(|v| !v.is_finite()) {
        return None;
    }
    // GeoJSON's nearest-tile stage map also sees failed captures. Reject a
    // single affine when one of their recorded centres disagrees with it.
    for (t, r) in area.placement.as_ref()?.tiles.iter().zip(&area.tiles) {
        if let Some(stage) = r.stage_nm {
            let expected = [
                o[0] + area.stage_sign[0] * (t.pos[0] + t.w as f64 / 2.0) * area.um_per_px(),
                o[1] + area.stage_sign[1] * (t.pos[1] + t.h as f64 / 2.0) * area.um_per_px(),
            ];
            if stage.iter().any(|v| !v.is_finite())
                || (stage[0] / 1000.0 - expected[0]).hypot(stage[1] / 1000.0 - expected[1])
                    > area.um_per_px() * 0.1
            {
                return None;
            }
        }
    }
    Some([
        area.stage_sign[0] * area.um_per_px(),
        0.0,
        o[0],
        0.0,
        area.stage_sign[1] * area.um_per_px(),
        o[1],
    ])
}
fn area_plan(session: &Session, a: &AreaRecord, values: Values, name: String) -> Result<ImagePlan> {
    let dims = if let Some(o) = session.outputs.get(&a.id) {
        (o.linear.w, o.linear.h)
    } else {
        let pl = a
            .placement
            .as_ref()
            .ok_or_else(|| invalid("Missing mosaic placement"))?;
        if values == Values::Displayed {
            return Err(invalid(format!(
                "Stitch {} to export As displayed.",
                a.name
            )));
        }
        (pl.width, pl.height)
    };
    if !a.um_per_px().is_finite() || a.um_per_px() <= 0.0 {
        return Err(invalid("Pixel size must be positive and finite"));
    }
    let bits = if values == Values::Displayed {
        8
    } else {
        session.mosaic_source_bits(&a.id)?
    };
    let l = tiff::Layout::new(dims.0 as u64, dims.1 as u64, bits, 1)?;
    let transform = affine(a);
    Ok(ImagePlan {
        name,
        format: Format::Tiff,
        downsample: 1,
        source_origin: [0, 0],
        limit_reason: None,
        area_ids: vec![a.id.clone()],
        width: l.width,
        height: l.height,
        bit_depth: bits,
        estimated_bytes: l.estimate(),
        big_tiff: l.big,
        pixel_to_stage_um: transform,
        pixel_size_um: [a.um_per_px(); 2],
        coordinate_reason: transform.is_none().then(|| {
            "Stage transforms are missing or inconsistent; coordinates are area-local µm.".into()
        }),
    })
}
fn whole(areas: &[ImagePlan], name: String) -> Result<ImagePlan> {
    let first = areas
        .first()
        .ok_or_else(|| invalid("Choose at least one area."))?;
    let k = first.pixel_size_um[0];
    if areas.iter().any(|a| a.pixel_to_stage_um.is_none()) {
        return Err(invalid(
            "Stage transforms are missing or inconsistent; export one file per area.",
        ));
    }
    if areas
        .iter()
        .any(|a| (a.pixel_size_um[0] - k).abs() > k * 1e-9)
    {
        return Err(invalid("Pixel sizes differ; export one file per area."));
    }
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for a in areas {
        let t = a.pixel_to_stage_um.unwrap();
        for (x, y) in [(0.0, 0.0), (a.width as f64, a.height as f64)] {
            let xx = t[0] * x + t[2];
            let yy = t[4] * y + t[5];
            b[0] = b[0].min(xx);
            b[1] = b[1].min(yy);
            b[2] = b[2].max(xx);
            b[3] = b[3].max(yy);
        }
    }
    let l = tiff::Layout::new(
        ((b[2] - b[0]) / k - 1e-9).ceil().max(1.0) as u64,
        ((b[3] - b[1]) / k - 1e-9).ceil().max(1.0) as u64,
        areas.iter().map(|a| a.bit_depth).max().unwrap_or(8),
        1,
    )?;
    Ok(ImagePlan {
        name,
        format: Format::Tiff,
        downsample: 1,
        source_origin: [0, 0],
        limit_reason: None,
        area_ids: areas.iter().flat_map(|a| a.area_ids.clone()).collect(),
        width: l.width,
        height: l.height,
        bit_depth: l.bits,
        estimated_bytes: l.estimate(),
        big_tiff: l.big,
        pixel_to_stage_um: Some([k, 0.0, b[0], 0.0, k, b[1]]),
        pixel_size_um: [k; 2],
        coordinate_reason: None,
    })
}
pub(crate) struct Source {
    pub placement: crate::mosaic::Placement,
    pub ids: Vec<usize>,
    pub groups: Vec<Vec<usize>>,
    pub area: usize,
    pub photo: crate::photometric::streaming::Prepared,
    pub blend: crate::blend::streaming::Prepared,
    pub strip: Option<(usize, crate::image::Gray)>,
    pub peak: usize,
}
pub struct Export {
    pub plan: Plan,
    project: Project,
    options: Options,
    sources: BTreeMap<String, serde_json::Value>,
    image: usize,
    tile: u64,
    header: bool,
    sidecars: u8,
    done: u64,
    total: u64,
    pub peak_output_bytes: usize,
    png: Option<raster::Png>,
    raw: BTreeMap<String, Source>,
    pub peak_source_working_bytes: usize,
}
impl Export {
    pub fn begin(session: &Session, project: &str, options: &str) -> Result<Self> {
        Self::begin_ctl(session, project, options, &Ctl::NONE)
    }
    pub fn begin_ctl(session: &Session, project: &str, options: &str, ctl: &Ctl) -> Result<Self> {
        ctl.check()?;
        let mut p = Project::from_json(project)?;
        let live = session
            .project
            .as_ref()
            .ok_or_else(|| invalid("No project is open."))?;
        if p.uuid != live.uuid {
            return Err(invalid("Export project differs from the open session."));
        }
        // Pixel provenance always uses the live core's placement and correction.
        for a in &mut p.areas {
            if let Some(actual) = live.areas.iter().find(|b| b.id == a.id) {
                let name = a.name.clone();
                *a = actual.clone();
                a.name = name;
            }
        }
        let opts: Options = serde_json::from_str(options).map_err(invalid)?;
        if opts.downsample > 131072
            || ![8, 16].contains(&opts.png_bits)
            || !(1..=100).contains(&opts.quality)
        {
            return Err(invalid("Invalid PNG depth or JPEG quality."));
        }
        if opts.format == Format::Jpg && opts.values != Values::Displayed {
            return Err(invalid("JPEG uses As displayed values."));
        }
        if opts.format == Format::Png && opts.png_bits == 16 && opts.values != Values::Original {
            return Err(invalid("16-bit PNG uses Original values."));
        }
        for a in &mut p.areas {
            if opts.area_ids.contains(&a.id)
                && a.placement.is_none()
                && !session.outputs.contains_key(&a.id)
            {
                session.mosaic_export_placement(a, ctl)?;
            }
        }
        let ids: BTreeSet<_> = opts.area_ids.iter().collect();
        if ids.is_empty() {
            return Err(invalid("Choose at least one area."));
        }
        if opts.whole_mount
            && (ids.len() != p.areas.len() || p.areas.iter().any(|a| !ids.contains(&a.id)))
        {
            return Err(invalid("Whole mount needs every area."));
        }
        let mut used = BTreeSet::new();
        let mut images = Vec::new();
        for id in &opts.area_ids {
            if images.iter().any(|a: &ImagePlan| a.area_ids.contains(id)) {
                continue;
            }
            let a = p
                .areas
                .iter()
                .find(|a| &a.id == id)
                .ok_or_else(|| invalid("Unknown export area."))?;
            let stem = format!("{}_{}_mosaic", stem_part(&p.name), stem_part(&a.name));
            let mut name = stem.clone();
            let mut n = 2;
            while !used.insert(name.to_lowercase()) {
                name = format!("{stem}-{n}");
                n += 1;
            }
            images.push(area_plan(session, a, opts.values, name)?);
        }
        let reason = if opts.whole_mount {
            match whole(&images, format!("{}_mount_mosaic", stem_part(&p.name))) {
                Ok(m) => {
                    images = vec![m];
                    None
                }
                Err(Error::Invalid(reason)) => Some(reason),
                Err(e) => Some(e.to_string()),
            }
        } else {
            None
        };
        let mut output = Vec::new();
        for mut a in images {
            a.format = opts.format;
            if opts.format == Format::Png {
                a.bit_depth = if opts.values == Values::Displayed {
                    8
                } else {
                    opts.png_bits
                };
            }
            let factor = if opts.size_choice == SizeChoice::Downsample {
                fit_factor(a.width, a.height).max(opts.downsample.max(1).next_power_of_two())
            } else {
                1
            };
            if factor > 1 {
                a.width = a.width.div_ceil(factor);
                a.height = a.height.div_ceil(factor);
                a.downsample = factor;
                for v in &mut a.pixel_size_um {
                    *v *= factor as f64;
                }
                if let Some(t) = &mut a.pixel_to_stage_um {
                    t[0] *= factor as f64;
                    t[1] *= factor as f64;
                    t[3] *= factor as f64;
                    t[4] *= factor as f64;
                }
            }
            let limit =
                opts.format != Format::Tiff && (a.width > RASTER_LIMIT || a.height > RASTER_LIMIT);
            if limit && opts.size_choice == SizeChoice::Split {
                for row in 0..a.height.div_ceil(RASTER_LIMIT) {
                    for col in 0..a.width.div_ceil(RASTER_LIMIT) {
                        let mut tile = a.clone();
                        tile.name = format!("{}_r{}_c{}", a.name, row + 1, col + 1);
                        tile.source_origin = [col * RASTER_LIMIT, row * RASTER_LIMIT];
                        tile.width = (a.width - col * RASTER_LIMIT).min(RASTER_LIMIT);
                        tile.height = (a.height - row * RASTER_LIMIT).min(RASTER_LIMIT);
                        if let Some(t) = &mut tile.pixel_to_stage_um {
                            t[2] += t[0] * tile.source_origin[0] as f64
                                + t[1] * tile.source_origin[1] as f64;
                            t[5] += t[3] * tile.source_origin[0] as f64
                                + t[4] * tile.source_origin[1] as f64;
                        }
                        output.push(tile);
                    }
                }
            } else {
                a.limit_reason = limit.then(|| {
                    format!(
                        "{} exceeds 65,535 px per side. Choose downsampling or split files.",
                        opts.format.extension().to_uppercase()
                    )
                });
                output.push(a);
            }
        }
        let mut images = output;
        for a in &mut images {
            if a.format == Format::Tiff {
                let l = tiff::Layout::new(a.width as u64, a.height as u64, a.bit_depth, 1)?;
                a.estimated_bytes = l.estimate();
                a.big_tiff = l.big;
            } else {
                a.big_tiff = false;
                a.estimated_bytes = (a.width as u64)
                    .saturating_mul(a.height as u64)
                    .saturating_mul((a.bit_depth / 8) as u64)
                    .saturating_mul(if a.format == Format::Jpg { 8 } else { 2 })
                    .saturating_add(1048576);
            }
        }
        let sources = opts
            .area_ids
            .iter()
            .map(|id| (id.clone(), session.mosaic_export_provenance(id)))
            .collect();
        let total = images
            .iter()
            .map(|a| {
                if a.format == Format::Tiff {
                    (a.width as u64).div_ceil(tiff::TILE) * (a.height as u64).div_ceil(tiff::TILE)
                } else {
                    (a.height as u64).div_ceil(if a.format == Format::Jpg { 8 } else { 64 })
                }
            })
            .sum();
        Ok(Self {
            plan: Plan {
                images,
                whole_mount_reason: reason,
            },
            project: p,
            options: opts,
            sources,
            image: 0,
            tile: 0,
            header: false,
            sidecars: 0,
            done: 0,
            total,
            peak_output_bytes: 0,
            png: None,
            raw: BTreeMap::new(),
            peak_source_working_bytes: 0,
        })
    }
    pub fn done(&self) -> bool {
        self.image >= self.plan.images.len()
    }
    pub fn progress(&self) -> (u64, u64) {
        (self.done, self.total)
    }
    pub fn metadata(&self, i: usize) -> serde_json::Value {
        let a = &self.plan.images[i];
        let areas:Vec<_>=a.area_ids.iter().map(|id| {let r=self.project.areas.iter().find(|r| &r.id==id).unwrap();json!({"id":r.id,"name":r.name,"pixel_to_stage_um":affine(r),"pixel_source":self.sources.get(id),"stage_sign":r.stage_sign,"tiles":r.tiles,"placement":r.placement,"photometric":r.photometric,"base_tone":if self.options.values==Values::Displayed {Some(&r.params.tone)} else {None},"display_tone":if self.options.values==Values::Displayed {Some(self.options.display_tone.get(id).unwrap_or(&r.params.tone))} else {None}})}).collect();
        json!({"schema":"grainatlas-mosaic-1","area_id":if a.area_ids.len()==1 {a.area_ids.first()} else {None},"area_name":if areas.len()==1 {areas[0].get("name")} else {None},"areas":areas,"pixel_size_um":a.pixel_size_um,"image_size":[a.width,a.height],"pixel_to_stage_um":a.pixel_to_stage_um,"pixel_to_local_um":[a.pixel_size_um[0],0.0,a.source_origin[0] as f64*a.pixel_size_um[0]/a.downsample as f64,0.0,a.pixel_size_um[1],a.source_origin[1] as f64*a.pixel_size_um[1]/a.downsample as f64],"coordinate_frame":if a.pixel_to_stage_um.is_some(){"instrument stage / mount µm"} else {"area-local µm"},"coordinate_reason":a.coordinate_reason,"pixel_convention":"Integer (col,row) denotes the upper-left pixel corner; sample centres are (col+0.5,row+0.5). Affine order [a,b,c,d,e,f]: X=a*col+b*row+c; Y=d*col+e*row+f.","bit_depth":a.bit_depth,"channels":1,"values_mode":self.options.values,"app_version":env!("CARGO_PKG_VERSION"),"source_package":self.project.package,"project_id":self.project.uuid,"format":a.format,"downsample_factor":a.downsample,"downsample_sampling":"Nearest source pixel centre; partial per-area edges clamp to source bounds.","source_origin":a.source_origin,"quality":if a.format==Format::Jpg {Some(self.options.quality)} else {None},"original_8bit_scaling":"Clamp blended values to [0,1], multiply by 255 and round; no tone curve.","compression":if a.format==Format::Tiff {"none"} else {"format default"},"tile_size":if a.format==Format::Tiff {Some([512,512])} else {None},"whole_mount_sampling":"Nearest source pixel centre; first selected area wins overlaps; gaps are zero."})
    }
    fn render(
        &mut self,
        session: &mut Session,
        tx: u32,
        ty: u32,
        rw: u32,
        rh: u32,
        ctl: &Ctl,
    ) -> Result<Vec<u8>> {
        let a = &self.plan.images[self.image];
        let whole_mount = self.options.whole_mount && self.plan.whole_mount_reason.is_none();
        let (tx, ty) = (tx as u64, ty as u64);
        let mut data = vec![0; rw as usize * rh as usize * (a.bit_depth / 8) as usize];
        // Areas are visited one at a time; no mount-sized composite is allocated.
        let mut filled = vec![false; (rw as u64 * rh as u64) as usize];
        for id in &a.area_ids {
            ctl.check()?;
            let rec = self.project.areas.iter().find(|r| &r.id == id).unwrap();
            let dims = if let Some(o) = session.outputs.get(id) {
                (o.linear.w, o.linear.h)
            } else {
                let pl = rec.placement.as_ref().unwrap();
                (pl.width, pl.height)
            };
            let transform = affine(rec);
            let rect = |x: f64, y: f64| -> [f64; 2] {
                if !whole_mount {
                    return [
                        (a.source_origin[0] as f64 + x * a.downsample as f64)
                            .min(dims.0 as f64 - 0.5),
                        (a.source_origin[1] as f64 + y * a.downsample as f64)
                            .min(dims.1 as f64 - 0.5),
                    ];
                }
                let dst = a.pixel_to_stage_um.unwrap();
                let src = transform.unwrap();
                [
                    (dst[0] * x + dst[2] - src[2]) / src[0],
                    (dst[4] * y + dst[5] - src[5]) / src[4],
                ]
            };
            let corners = [
                rect(tx as f64 + 0.5, ty as f64 + 0.5),
                rect((tx + rw as u64) as f64 - 0.5, (ty + rh as u64) as f64 - 0.5),
            ];
            if corners[0][0].max(corners[1][0]) < 0.0
                || corners[0][1].max(corners[1][1]) < 0.0
                || corners[0][0].min(corners[1][0]) >= dims.0 as f64
                || corners[0][1].min(corners[1][1]) >= dims.1 as f64
            {
                continue;
            }
            let tone = self
                .options
                .display_tone
                .get(id)
                .unwrap_or(&rec.params.tone);
            let min_y = corners[0][1].min(corners[1][1]).floor().max(0.) as usize;
            let max_y = (corners[0][1].max(corners[1][1]).floor().max(0.) as usize + 1).min(dims.1);
            // Match the output strip's phase. For flipped areas, cache the
            // preceding 512 rows so descending PNG/JPEG scanlines can reuse it.
            let descending = whole_mount && transform.is_some_and(|t| t[4] < 0.);
            let span = max_y.saturating_sub(min_y);
            let start = if descending {
                min_y.saturating_sub((512 - span % 512) % 512)
            } else {
                min_y
            };
            for source_y in (start..max_y).step_by(512) {
                for (other, s) in &mut self.raw {
                    if other != id {
                        s.strip = None;
                    }
                }
                let cache = self.raw.entry(id.clone());
                let source = if session.mosaic_export_needs_source(id, self.options.values) {
                    Some(match cache {
                        std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                        std::collections::btree_map::Entry::Vacant(e) => {
                            e.insert(session.mosaic_export_source(rec, ctl)?)
                        }
                    })
                } else {
                    None
                };
                if let Some(s) = source.as_ref() {
                    self.peak_source_working_bytes = self.peak_source_working_bytes.max(s.peak);
                }
                session.with_mosaic_export_pixels(
                    id,
                    self.options.values,
                    tone,
                    ctl,
                    source,
                    source_y,
                    max_y.min(source_y + 512),
                    |w, h, origin_y, pixels| {
                        for y in 0..rh as u64 {
                            ctl.check()?;
                            for x in 0..rw as u64 {
                                if tx + x >= a.width as u64 || ty + y >= a.height as u64 {
                                    continue;
                                }
                                let j = (y * rw as u64 + x) as usize;
                                if filled[j] {
                                    continue;
                                }
                                let src = rect((tx + x) as f64 + 0.5, (ty + y) as f64 + 0.5);
                                let (sx, sy) = (src[0].floor() as i64, src[1].floor() as i64);
                                if sx < 0
                                    || sy < origin_y as i64
                                    || sx >= w as i64
                                    || sy >= (origin_y + h) as i64
                                {
                                    continue;
                                }
                                let v = pixels.at((sy as usize - origin_y) * w + sx as usize);
                                if !v.is_finite() {
                                    return Err(invalid("Non-finite mosaic pixel."));
                                }
                                if a.bit_depth == 8 {
                                    data[j] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                                } else {
                                    data[j * 2..j * 2 + 2].copy_from_slice(
                                        &((v * 65535.0).round().clamp(0.0, 65535.0) as u16)
                                            .to_le_bytes(),
                                    );
                                }
                                filled[j] = true;
                            }
                        }
                        Ok(())
                    },
                )?;
                if let Some(s) = self.raw.get(id) {
                    self.peak_source_working_bytes = self.peak_source_working_bytes.max(s.peak);
                }
            }
        }
        self.peak_output_bytes = self.peak_output_bytes.max(data.len() + filled.len());
        Ok(data)
    }
    /// One chunk of one file. `last` lets hosts close the stream immediately.
    pub fn next(&mut self, session: &mut Session, ctl: &Ctl) -> Result<Option<Chunk>> {
        ctl.check()?;
        if self.done() {
            return Ok(None);
        }
        let a = &self.plan.images[self.image];
        let l = tiff::Layout::new(a.width as u64, a.height as u64, a.bit_depth, 1)?;
        let name = format!("{}.{}", a.name, a.format.extension());
        if let Some(reason) = &a.limit_reason {
            return Err(invalid(reason));
        }
        if a.format != Format::Tiff && self.sidecars == 0 && !self.header {
            let rows = if a.format == Format::Jpg { 8 } else { 64 };
            let y = self.tile as u32 * rows;
            if y < a.height {
                let count = rows.min(a.height - y);
                let w = a.width;
                let h = a.height;
                let bits = a.bit_depth;
                let format = a.format;
                let mut data = self.render(session, 0, y, w, count, ctl)?;
                let last = y + count == h;
                self.peak_output_bytes = self.peak_output_bytes.max(data.len());
                let encoded = if format == Format::Png {
                    if bits == 16 {
                        for px in data.as_chunks_mut::<2>().0 {
                            px.swap(0, 1);
                        }
                    }
                    if self.png.is_none() {
                        self.png = Some(raster::Png::new(w, h, bits)?);
                    }
                    let result = self.png.as_mut().unwrap().rows(&data, last)?;
                    if last {
                        self.png = None;
                    }
                    result
                } else {
                    raster::jpeg_strip(w, h, y, &data, self.options.quality)?
                };
                self.tile += 1;
                self.done += 1;
                ctl.report(
                    &format!("Exporting mosaic: {} / {} strips", self.done, self.total),
                    self.done as f32 / self.total as f32,
                );
                if last {
                    self.header = true;
                }
                return Ok(Some(Chunk {
                    name,
                    data: encoded,
                    last,
                }));
            }
        }
        let a = &self.plan.images[self.image];
        if !self.header && a.format == Format::Tiff {
            self.header = true;
            let description=json!({"schema":"grainatlas-mosaic-1","project_id":self.project.uuid,"areas":a.area_ids,"values_mode":self.options.values,"app_version":env!("CARGO_PKG_VERSION"),"sidecar":format!("{}.json",a.name)}).to_string();
            return Ok(Some(Chunk {
                name,
                data: l.header(a.pixel_size_um, &description)?,
                last: false,
            }));
        }
        if a.format == Format::Tiff && self.tile < l.tiles {
            let tx = self.tile % l.cols() * tiff::TILE;
            let ty = self.tile / l.cols() * tiff::TILE;
            let data = self.render(
                session,
                tx as u32,
                ty as u32,
                tiff::TILE as u32,
                tiff::TILE as u32,
                ctl,
            )?;
            self.tile += 1;
            self.done += 1;
            ctl.report(
                &format!("Exporting mosaic: {} / {} tiles", self.done, self.total),
                self.done as f32 / self.total as f32,
            );
            return Ok(Some(Chunk {
                name,
                data,
                last: self.tile == l.tiles,
            }));
        }
        let a = &self.plan.images[self.image];
        let chunk = if self.sidecars == 0 {
            Chunk {
                name: format!("{}.json", a.name),
                data: serde_json::to_vec_pretty(&self.metadata(self.image)).map_err(invalid)?,
                last: true,
            }
        } else {
            let t = a.pixel_to_stage_um.unwrap_or([
                a.pixel_size_um[0],
                0.0,
                a.source_origin[0] as f64 * a.pixel_size_um[0] / a.downsample as f64,
                0.0,
                a.pixel_size_um[1],
                a.source_origin[1] as f64 * a.pixel_size_um[1] / a.downsample as f64,
            ]);
            Chunk {
                name: format!("{}.{}", a.name, a.format.world()),
                data: format!(
                    "{:.15}\n{:.15}\n{:.15}\n{:.15}\n{:.15}\n{:.15}\n",
                    t[0],
                    t[3],
                    t[1],
                    t[4],
                    t[2] + (t[0] + t[1]) / 2.0,
                    t[5] + (t[3] + t[4]) / 2.0
                )
                .into_bytes(),
                last: true,
            }
        };
        self.sidecars += 1;
        if self.sidecars == 2 {
            self.raw.clear();
            self.image += 1;
            self.tile = 0;
            self.header = false;
            self.sidecars = 0;
        }
        Ok(Some(chunk))
    }
}
pub struct Chunk {
    pub name: String,
    pub data: Vec<u8>,
    pub last: bool,
}

/// Native hosts pass a newly created, private export directory. On any error
/// (including cancellation and flush failure) all files created here are removed.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_directory(
    job: &mut Export,
    session: &mut Session,
    dir: &std::path::Path,
    ctl: &Ctl,
) -> Result<()> {
    use std::io::Write;
    let mut paths = Vec::new();
    let result = (|| {
        let mut current: Option<(String, std::io::BufWriter<std::fs::File>)> = None;
        while let Some(chunk) = job.next(session, ctl)? {
            if current.as_ref().is_none_or(|(n, _)| n != &chunk.name) {
                if current.is_some() {
                    return Err(invalid("Unfinished mosaic stream."));
                }
                let path = dir.join(&chunk.name);
                let f = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(invalid)?;
                paths.push(path);
                current = Some((chunk.name.clone(), std::io::BufWriter::new(f)));
            }
            current
                .as_mut()
                .unwrap()
                .1
                .write_all(&chunk.data)
                .map_err(invalid)?;
            if chunk.last {
                current.as_mut().unwrap().1.flush().map_err(invalid)?;
                current = None;
            }
        }
        ctl.check()
    })();
    session.finish_mosaic_export();
    if let Err(error) = result {
        for path in paths {
            if let Err(cleanup) = std::fs::remove_file(&path) {
                return Err(invalid(format!(
                    "{error}; could not remove partial file {}: {cleanup}",
                    path.display()
                )));
            }
        }
        return Err(error);
    }
    Ok(())
}
