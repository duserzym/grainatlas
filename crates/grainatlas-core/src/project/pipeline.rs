//! The area pipeline: read → stitch → photometric → blend → tone → pyramid.
//! Everything after `read` is a pure function of the tiles and `AreaParams`.

use serde::{Deserialize, Serialize};

use super::{AreaParams, AreaRecord};
use crate::Ctl;
use crate::image::Gray;
use crate::mosaic::pyramid::{MosaicJson, Pyramid, TILE};
use crate::mosaic::{self, Placement};
use crate::readers::{Package, Tile};
use crate::segment::Grain;

/// Wall-clock milliseconds per step (filled by the caller's clock; the core has none on wasm).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Timings {
    pub steps: Vec<(String, f64)>,
}

/// Everything the pipeline produced for one area.
pub struct AreaOutput {
    pub placement: Placement,
    /// Blended mosaic before the tone curve (f32, 0..1). Session eviction clears
    /// pixels but retains dimensions; Session restores them before heavy operations.
    pub linear: Gray,
    /// Coverage per pixel (0..1), empty alongside evicted linear pixels.
    pub coverage: Vec<f32>,
    /// 8-bit base mosaic after the tone curve, shared with pyramid level 0.
    pub base: std::sync::Arc<Vec<u8>>,
    pub pyramid: Pyramid,
    pub mosaic_json: MosaicJson,
}

/// A monotonic clock supplied by the host (native `Instant`, `performance.now()` in the browser).
pub type Clock<'a> = &'a dyn Fn() -> f64;

/// Reads the tiles of `area` and records their hashes in `rec`.
pub fn read_tiles(pkg: &dyn Package, area: usize, rec: &mut AreaRecord) -> crate::Result<Vec<Tile>> {
    let tiles = pkg.tiles(area)?;
    if let Some(meta) = pkg.info().areas.get(area) { rec.merge_metadata(meta); }
    for (t, r) in tiles.iter().zip(rec.tiles.iter_mut()) {
        r.sha256 = t.sha256.clone();
        r.w = t.image.w;
        r.h = t.image.h;
    }
    Ok(tiles)
}

/// The images the stitcher and the photometric steps see: one per field (exposures of the same
/// field HDR-merged), with their metadata.
pub struct Inputs {
    pub imgs: Vec<Gray>,
    pub meta: crate::readers::AreaMeta,
}

/// Groups the decoded tiles into fields (HDR merge of repeated exposures).
pub fn inputs(rec: &AreaRecord, tiles: &[Tile]) -> Inputs {
    let meta = crate::readers::AreaMeta {
        id: rec.id.clone(),
        name: rec.name.clone(),
        grid: None,
        pixel_size_nm: rec.pixel_size_nm,
        field_um: None,
        pitch_um: None,
        stage_sign: rec.stage_sign,
        tiles: tiles.iter().map(|t| t.meta.clone()).collect(),
        meta: Default::default(),
    };
    // Fields captured more than once (same stage position) are HDR-merged first.
    let groups = hdr_groups(&meta.tiles);
    let (imgs, meta) = if groups.iter().any(|g| g.len() > 1) {
        let imgs: Vec<Gray> = groups
            .iter()
            .map(|g| {
                if g.len() == 1 {
                    tiles[g[0]].image.clone()
                } else {
                    let stack: Vec<Gray> = g.iter().map(|&i| tiles[i].image.clone()).collect();
                    crate::photometric::hdr::merge_exposures(&stack).map(|m| m.image).unwrap_or_else(|| stack[0].clone())
                }
            })
            .collect();
        let mut m = meta;
        m.tiles = groups.iter().enumerate().map(|(k, g)| crate::readers::TileMeta { index: k, ..m.tiles[g[0]].clone() }).collect();
        (imgs, m)
    } else {
        (tiles.iter().map(|t| t.image.clone()).collect(), meta)
    };
    Inputs { imgs, meta }
}

/// Runs the whole pipeline on decoded tiles.
pub fn run(rec: &mut AreaRecord, tiles: &[Tile], params: &AreaParams, ctl: &Ctl, clock: Clock, timings: &mut Timings) -> crate::Result<AreaOutput> {
    run_with(rec, tiles, params, &[], ctl, clock, timings)
}

/// The whole pipeline, with the area's grains for per-grain tone normalisation.
pub fn run_with(rec: &mut AreaRecord, tiles: &[Tile], params: &AreaParams, grains: &[Grain], ctl: &Ctl, clock: Clock, timings: &mut Timings) -> crate::Result<AreaOutput> {
    let Inputs { imgs, meta } = inputs(rec, tiles);
    let t0 = clock();
    ctl.report("stitch", 0.0);
    let placement = mosaic::stitch(&meta, &imgs, &params.stitch, ctl)?;
    timings.steps.push(("stitch".into(), clock() - t0));
    ctl.check()?;
    finish(rec, imgs, placement, params, grains, ctl, clock, timings)
}

/// Everything after the stitch, on a solved placement: photometric correction, blend, tone,
/// pyramid. A change of correction, blending or tone re-runs only this (no re-stitch).
#[allow(clippy::too_many_arguments)]
pub fn finish(rec: &mut AreaRecord, imgs: Vec<Gray>, placement: Placement, params: &AreaParams, grains: &[Grain], ctl: &Ctl, clock: Clock, timings: &mut Timings) -> crate::Result<AreaOutput> {
    let mut t0 = clock();
    let mut lap = |name: &str, timings: &mut Timings| {
        let t = clock();
        timings.steps.push((name.to_string(), t - t0));
        t0 = t;
    };
    ctl.report("photometric", 0.0);
    let (corrected, photo) = crate::photometric::correct(&placement, &imgs, &params.photometric);
    drop(imgs);
    lap("photometric", timings);
    ctl.check()?;
    let fill = if params.photometric.background { photo.background_median } else { median_of(&corrected) };

    let (linear, coverage) = crate::blend::blend(&placement, corrected, &params.blend, fill, ctl)?;
    lap("blend", timings);
    ctl.check()?;

    ctl.report("tone", 0.0);
    let base = tone_base_with(&linear, &coverage, &params.tone, grains, rec.um_per_px());
    lap("tone", timings);
    let mosaic_json_sha = crate::util::sha256_hex(&base);
    let pyramid = Pyramid::build(linear.w, linear.h, base);
    lap("pyramid", timings);
    let base = pyramid.levels[0].1.clone();
    let mosaic_json = MosaicJson {
        w: linear.w,
        h: linear.h,
        um_per_px: rec.um_per_px(),
        origin_um: [0.0, 0.0],
        tile_size: TILE,
        format: "png".into(),
        levels: pyramid.infos(),
        sha256: mosaic_json_sha,
    };
    rec.params = params.clone();
    rec.placement = Some(placement.clone());
    rec.photometric = Some(photo);
    rec.mosaic = Some(mosaic_json.clone());
    Ok(AreaOutput { placement, linear, coverage, base, pyramid, mosaic_json })
}

/// Deterministic rehydration from the original package and the stored solved placement.
pub fn rebuild_heavy(
    rec: &AreaRecord,
    tiles: Vec<Tile>,
    placement: &Placement,
    ctl: &Ctl,
) -> crate::Result<(Gray, Vec<f32>)> {
    let Inputs { imgs, .. } = inputs(rec, &tiles);
    // inputs owns its pixels; release the decoded source copies before correction/blending.
    drop(tiles);
    let (corrected, photo) = crate::photometric::correct(placement, &imgs, &rec.params.photometric);
    drop(imgs);
    let fill = if rec.params.photometric.background {
        photo.background_median
    } else {
        median_of(&corrected)
    };
    crate::blend::blend(placement, corrected, &rec.params.blend, fill, ctl)
}

/// The 8-bit base mosaic: percentile stretch over covered pixels, then CLAHE if enabled.
pub fn tone_base(linear: &Gray, coverage: &[f32], p: &crate::photometric::ToneParams) -> Vec<u8> {
    tone_base_with(linear, coverage, p, &[], 0.0)
}

/// `tone_base`, then per-grain normalisation inside the grain masks when enabled
/// (`um_per_px` of `linear`).
pub fn tone_base_with(linear: &Gray, coverage: &[f32], p: &crate::photometric::ToneParams, grains: &[Grain], um_per_px: f64) -> Vec<u8> {
    let mut g = linear.clone();
    crate::photometric::tone::stretch(&mut g, Some(coverage), p);
    if p.clahe {
        crate::photometric::tone::clahe(&mut g, p.clahe_tile, p.clahe_clip);
    }
    crate::photometric::grain_norm::normalize(&mut g, grains, um_per_px, &p.grain_norm);
    g.to_u8()
}

/// A low-resolution render of the whole area (downsampled `factor` times) with `params`, for a
/// live preview while a control moves: correction, blend and tone run on downsampled inputs and
/// a scaled placement, with pixel-sized parameters scaled to match. Returns (w, h, 8-bit pixels).
pub fn preview(small: &[Gray], placement: &Placement, params: &AreaParams, factor: usize, grains: &[Grain], um_per_px: f64) -> (usize, usize, Vec<u8>) {
    let k = factor.max(1);
    let pl = scale_placement(placement, small, k);
    let mut p = params.clone();
    let bp = &mut p.photometric.background_params;
    bp.cell = (bp.cell / k).max(8);
    bp.dilate = bp.dilate.div_ceil(k);
    p.blend.seam_factor = (p.blend.seam_factor / k).max(1);
    p.tone.clahe_tile = (p.tone.clahe_tile / k).max(8);
    let (corrected, photo) = crate::photometric::correct(&pl, small, &p.photometric);
    let fill = if p.photometric.background { photo.background_median } else { median_of(&corrected) };
    let Ok((linear, coverage)) = crate::blend::blend(&pl, corrected, &p.blend, fill, &Ctl::NONE) else {
        return (0, 0, Vec::new());
    };
    let base = tone_base_with(&linear, &coverage, &p.tone, grains, um_per_px * k as f64);
    (linear.w, linear.h, base)
}

/// Tone-only preview from an existing output: the linear mosaic downsampled, then tone.
pub fn preview_tone(out: &AreaOutput, tone: &crate::photometric::ToneParams, factor: usize, grains: &[Grain], um_per_px: f64) -> (usize, usize, Vec<u8>) {
    let k = factor.max(1);
    let lin = out.linear.downsample(k);
    let cov = Gray { w: out.linear.w, h: out.linear.h, px: out.coverage.clone() }.downsample(k).px;
    let mut t = tone.clone();
    t.clahe_tile = (t.clahe_tile / k).max(8);
    (lin.w, lin.h, tone_base_with(&lin, &cov, &t, grains, um_per_px * k as f64))
}

/// The placement of `pl` for inputs downsampled `k` times (sizes from the small images).
pub fn scale_placement(pl: &Placement, small: &[Gray], k: usize) -> Placement {
    let kf = k as f64;
    let mut out = pl.clone();
    for (t, g) in out.tiles.iter_mut().zip(small) {
        t.x = (t.x as f64 / kf).round() as i64;
        t.y = (t.y as f64 / kf).round() as i64;
        t.w = g.w;
        t.h = g.h;
        t.pos = [t.pos[0] / kf, t.pos[1] / kf];
        t.prior = [t.prior[0] / kf, t.prior[1] / kf];
    }
    out.width = out.tiles.iter().map(|t| (t.x.max(0) as usize) + t.w).max().unwrap_or(1);
    out.height = out.tiles.iter().map(|t| (t.y.max(0) as usize) + t.h).max().unwrap_or(1);
    out
}

/// Rebuilds base, pyramid and mosaic.json after a tone change (no re-stitch).
pub fn retone(rec: &mut AreaRecord, out: &mut AreaOutput, tone: &crate::photometric::ToneParams, grains: &[Grain]) {
    let base = tone_base_with(&out.linear, &out.coverage, tone, grains, rec.um_per_px());
    out.pyramid = Pyramid::build(out.linear.w, out.linear.h, base);
    out.base = out.pyramid.levels[0].1.clone();
    out.mosaic_json.sha256 = crate::util::sha256_hex(&out.base);
    out.mosaic_json.levels = out.pyramid.infos();
    rec.params.tone = tone.clone();
    rec.mosaic = Some(out.mosaic_json.clone());
}

/// Groups of tile indices that share a position (within 1 um): one field, several exposures.
pub fn hdr_groups(tiles: &[crate::readers::TileMeta]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, t) in tiles.iter().enumerate() {
        match groups.iter_mut().find(|g| {
            let r = &tiles[g[0]];
            (r.pos_um[0] - t.pos_um[0]).abs() < 1.0 && (r.pos_um[1] - t.pos_um[1]).abs() < 1.0 && r.w == t.w && r.h == t.h
        }) {
            Some(g) => g.push(i),
            None => groups.push(vec![i]),
        }
    }
    groups
}

/// Median of a sample of all tile pixels (hole fill value before photometric correction).
fn median_of(tiles: &[Gray]) -> f32 {
    let mut v: Vec<f32> = tiles.iter().flat_map(|t| t.px.iter().step_by(97).copied()).collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// Files of one area inside the project folder (relative path, bytes): the pyramid tiles under
/// `mosaic/<area>/<level>/<col>_<row>.png` and `mosaic/<area>/mosaic.json`.
pub fn area_files(rec: &AreaRecord, out: &AreaOutput) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = out
        .pyramid
        .png_tiles()
        .into_iter()
        .map(|(l, c, r, b)| (format!("mosaic/{}/{l}/{c}_{r}.png", rec.id), b))
        .collect();
    files.push((format!("mosaic/{}/mosaic.json", rec.id), serde_json::to_vec_pretty(&out.mosaic_json).expect("json")));
    files
}

/// Work image for segmentation from a stitched area.
pub fn work_image(out: &AreaOutput, rec: &AreaRecord, scale: usize) -> crate::segment::WorkImage {
    let mut w = crate::segment::WorkImage::new(&out.linear, &out.coverage, rec.um_per_px(), scale);
    w.signal = rec.signal();
    let k = rec.um_per_px();
    // Use the integer blend positions, not stage priors or the sub-pixel stitch solution.
    // Placement also has the merged field sizes when inputs were HDR-grouped.
    w.tiles_um = out.placement.tiles.iter().map(|t| {
        [t.x as f64 * k, t.y as f64 * k, (t.x as f64 + t.w as f64) * k, (t.y as f64 + t.h as f64) * k]
    }).collect();
    w
}

/// Segments a stitched area, keeping ids and numbers of `previous` grains that still match.
pub fn segment_area(rec: &mut AreaRecord, out: &AreaOutput, p: &crate::segment::SegmentParams, previous: &[crate::segment::Grain], outline: crate::segment::OutlinePad) -> Vec<crate::segment::Grain> {
    let w = work_image(out, rec, p.work_scale);
    rec.params.segment = p.clone();
    crate::segment::segment_padded(&w, p, previous, outline)
}

/// `masks/<area>/grains.png` (u16 labels by grain number at work resolution) and `grains.json`.
pub fn mask_files(rec: &AreaRecord, out: &AreaOutput, grains: &[crate::segment::Grain]) -> Vec<(String, Vec<u8>)> {
    let w = work_image(out, rec, rec.params.segment.work_scale);
    let lab = crate::segment::label_image(&w, grains);
    let json = serde_json::json!({ "work_scale": rec.params.segment.work_scale, "um_per_px": w.um_per_px, "w": w.img.w, "h": w.img.h, "grains": grains });
    vec![
        (format!("masks/{}/grains.png", rec.id), crate::image::encode_png_gray16(w.img.w, w.img.h, &lab)),
        (format!("masks/{}/grains.json", rec.id), serde_json::to_vec_pretty(&json).expect("json")),
    ]
}

/// An area's pixels for the print layout, with the display LUT of `tone` applied on print.
pub fn area_image<'a>(rec: &'a AreaRecord, out: &'a AreaOutput, tone: &crate::photometric::ToneParams) -> crate::layout::AreaImage<'a> {
    crate::layout::AreaImage {
        id: &rec.id,
        name: &rec.name,
        sample: if rec.sample.is_empty() { &rec.name } else { &rec.sample },
        w: out.linear.w,
        h: out.linear.h,
        px: &out.base,
        lut: crate::photometric::tone::lut(tone),
        um_per_px: rec.um_per_px(),
    }
}
