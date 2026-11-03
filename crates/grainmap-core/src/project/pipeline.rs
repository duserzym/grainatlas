//! The area pipeline: read → stitch → photometric → blend → tone → pyramid.
//! Everything after `read` is a pure function of the tiles and `AreaParams`.

use serde::{Deserialize, Serialize};

use super::{AreaParams, AreaRecord};
use crate::Ctl;
use crate::image::Gray;
use crate::mosaic::pyramid::{MosaicJson, Pyramid, TILE};
use crate::mosaic::{self, Placement};
use crate::readers::{Package, Tile};

/// Wall-clock milliseconds per step (filled by the caller's clock; the core has none on wasm).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Timings {
    pub steps: Vec<(String, f64)>,
}

/// Everything the pipeline produced for one area.
pub struct AreaOutput {
    pub placement: Placement,
    /// Blended mosaic before the tone curve (f32, 0..1).
    pub linear: Gray,
    /// Coverage per pixel (0..1).
    pub coverage: Vec<f32>,
    /// 8-bit base mosaic after the tone curve (what the pyramid and the hash are made of).
    pub base: Vec<u8>,
    pub pyramid: Pyramid,
    pub mosaic_json: MosaicJson,
}

/// A monotonic clock supplied by the host (native `Instant`, `performance.now()` in the browser).
pub type Clock<'a> = &'a dyn Fn() -> f64;

/// Reads the tiles of `area` and records their hashes in `rec`.
pub fn read_tiles(pkg: &dyn Package, area: usize, rec: &mut AreaRecord) -> crate::Result<Vec<Tile>> {
    let tiles = pkg.tiles(area)?;
    for (t, r) in tiles.iter().zip(rec.tiles.iter_mut()) {
        r.sha256 = t.sha256.clone();
        r.w = t.image.w;
        r.h = t.image.h;
    }
    Ok(tiles)
}

/// Runs the whole pipeline on decoded tiles.
pub fn run(rec: &mut AreaRecord, tiles: &[Tile], params: &AreaParams, ctl: &Ctl, clock: Clock, timings: &mut Timings) -> crate::Result<AreaOutput> {
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
    let mut t0 = clock();
    let mut lap = |name: &str, timings: &mut Timings| {
        let t = clock();
        timings.steps.push((name.to_string(), t - t0));
        t0 = t;
    };
    ctl.report("stitch", 0.0);
    let placement = mosaic::stitch(&meta, &imgs, &params.stitch, ctl)?;
    lap("stitch", timings);
    ctl.check()?;

    ctl.report("photometric", 0.0);
    let (mut corrected, photo) = crate::photometric::correct(&placement, &imgs, &params.photometric);
    lap("photometric", timings);
    ctl.check()?;
    let fill = if params.photometric.background { photo.background_median } else { median_of(&corrected) };

    let (linear, coverage) = crate::blend::blend(&placement, &corrected, &params.blend, fill, ctl)?;
    corrected.clear();
    lap("blend", timings);
    ctl.check()?;

    ctl.report("tone", 0.0);
    let base = tone_base(&linear, &coverage, &params.tone);
    lap("tone", timings);
    let mosaic_json_sha = crate::util::sha256_hex(&base);
    let pyramid = Pyramid::build(linear.w, linear.h, base.clone());
    lap("pyramid", timings);
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

/// The 8-bit base mosaic: percentile stretch over covered pixels, then CLAHE if enabled.
pub fn tone_base(linear: &Gray, coverage: &[f32], p: &crate::photometric::ToneParams) -> Vec<u8> {
    let mut g = linear.clone();
    crate::photometric::tone::stretch(&mut g, Some(coverage), p);
    if p.clahe {
        crate::photometric::tone::clahe(&mut g, p.clahe_tile, p.clahe_clip);
    }
    g.to_u8()
}

/// Rebuilds base, pyramid and mosaic.json after a tone change (no re-stitch).
pub fn retone(rec: &mut AreaRecord, out: &mut AreaOutput, tone: &crate::photometric::ToneParams) {
    out.base = tone_base(&out.linear, &out.coverage, tone);
    out.pyramid = Pyramid::build(out.linear.w, out.linear.h, out.base.clone());
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
