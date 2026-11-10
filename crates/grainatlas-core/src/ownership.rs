//! Cross-area grain ownership within one mount.
//!
//! In a zigzag package neighbouring areas overlap (100 to 850 um on the Boise
//! TM4000 runs), so one grain is detected in several areas. Every detection is
//! mapped mosaic → stage (as [`AreaRecord::stage_of`]) and from the stage into
//! every other area; detections of the same grain are linked. The grain is owned
//! by the area whose operator-drawn rectangle ([`drawn_area_nm`], the smallest one
//! when several hold it) contains it: the tile grid that covers a rectangle is
//! larger than it, so a neighbour's grid can hold a row more completely than the
//! row's own area does. A grain in no drawn rectangle (or without that metadata) is
//! owned by the area where its bounding box lies farthest inside the covered
//! mosaic; a grain cut by an area's edge then always loses to one that is whole.
//! The others are duplicates of the owner's grain.
//!
//! Areas whose names differ only by a `_N` suffix (`KH2-30.7` and
//! `KH2-30.7_2`) are one sample.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::project::AreaRecord;
use crate::segment::Grain;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OwnershipParams {
    /// A grain whose bounding box comes within this distance of the covered mosaic's edge is
    /// cut there (um).
    pub edge_um: f64,
    /// Stage disagreement allowed between areas when linking detections, on top of half the
    /// grain's size (um).
    pub match_um: f64,
    /// Areas named `<sample>_<N>` join the area named `<sample>`.
    pub merge_suffix: bool,
}

impl Default for OwnershipParams {
    fn default() -> Self {
        OwnershipParams { edge_um: 3.0, match_um: 20.0, merge_suffix: true }
    }
}

/// One detection and who owns it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GrainOwner {
    pub area_id: String,
    pub grain_id: Uuid,
    /// The area that owns this grain (its own area unless it is a duplicate).
    pub owner_area: String,
    /// The owner's detection of the same grain, when this one is a duplicate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<Uuid>,
    /// The bounding box touches the covered mosaic's edge.
    pub cut: bool,
    /// Distance from the bounding box to the covered mosaic's edge (um; negative outside).
    pub interior_um: f64,
    /// Stage position of the centroid (nm), when the area has stage positions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_nm: Option<[f64; 2]>,
    /// The grain lies in none of its areas' operator-drawn rectangles, so the owner comes from
    /// the coverage rule alone: worth a look.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outside_drawn: bool,
}

/// The rectangle the operator drew for an area on the TM4000 navigation image, on the stage
/// ([x0, y0, x1, y1] nm, x0 < x1, y0 < y1). The instrument covers it with whole tiles, so the
/// tile grid is larger and neighbouring grids overlap by grain rows that belong to the
/// neighbour; the drawn rectangle is the sample row the operator meant. `CenterX/Y` and
/// `AreaWidth/Height` are display pixels of the navigation image: its pixel size times
/// `ImageInfoWidth / ImageDispInfoWidth`, counted from its top-left stage corner towards the
/// bottom-right one. `None` without those keys (other instruments, older saved metadata).
pub fn drawn_area_nm(a: &AreaRecord) -> Option<[f64; 4]> {
    let g = |k: &str| a.area_meta.get(&format!("zag.{k}")).and_then(|v| v.trim().parse::<f64>().ok());
    let px = g("ImageInfoPixelSize")? * g("ImageInfoWidth")? / g("ImageDispInfoWidth")?;
    let (tl, br) = ([g("ImageInfoTopLeftStagePositionX")?, g("ImageInfoTopLeftStagePositionY")?], [g("ImageInfoBottomRightStagePositionX")?, g("ImageInfoBottomRightStagePositionY")?]);
    let (cx, cy, w, h) = (g("CenterX")?, g("CenterY")?, g("AreaWidth")?, g("AreaHeight")?);
    if !(px > 0.0 && w > 0.0 && h > 0.0) {
        return None;
    }
    let sx = (br[0] - tl[0]).signum();
    let sy = (br[1] - tl[1]).signum();
    let (x, y) = (tl[0] + sx * cx * px, tl[1] + sy * cy * px);
    let (hw, hh) = (w * px / 2.0, h * px / 2.0);
    Some([x - hw, y - hh, x + hw, y + hh])
}

fn rect_contains(r: &[f64; 4], s: [f64; 2]) -> bool {
    s[0] >= r[0] && s[0] <= r[2] && s[1] >= r[1] && s[1] <= r[3]
}

/// Areas that make up one sample.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SampleGroup {
    pub sample: String,
    pub areas: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Ownership {
    pub grains: Vec<GrainOwner>,
    pub samples: Vec<SampleGroup>,
}

impl Ownership {
    /// The record of one detection.
    pub fn of(&self, area_id: &str, grain_id: &Uuid) -> Option<&GrainOwner> {
        self.grains.iter().find(|g| g.area_id == area_id && &g.grain_id == grain_id)
    }

    /// True when the detection is its own (not a duplicate). Unknown detections are owned.
    pub fn is_owned(&self, area_id: &str, grain_id: &Uuid) -> bool {
        self.of(area_id, grain_id).is_none_or(|g| g.duplicate_of.is_none())
    }

    /// The owned grains of every area.
    pub fn owned(&self, grains: &BTreeMap<String, Vec<Grain>>) -> BTreeMap<String, Vec<Grain>> {
        grains.iter().map(|(a, gs)| (a.clone(), gs.iter().filter(|g| self.is_owned(a, &g.id)).cloned().collect())).collect()
    }

    /// The sample an area belongs to.
    pub fn sample_of(&self, area_id: &str) -> Option<&str> {
        self.samples.iter().find(|s| s.areas.iter().any(|a| a == area_id)).map(|s| s.sample.as_str())
    }
}

/// `KH2-30.7_2` → `KH2-30.7`; names without a numeric `_N` suffix are returned as they are.
pub fn strip_suffix(name: &str) -> &str {
    match name.rsplit_once('_') {
        Some((base, n)) if !base.is_empty() && !n.is_empty() && n.len() <= 2 && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => name,
    }
}

fn area_sample(a: &AreaRecord) -> &str {
    if a.sample.is_empty() { &a.name } else { &a.sample }
}

/// Samples of a mount in area order: an area named `<name>_<N>` joins the sample `<name>`
/// when another area of the mount is named `<name>` or `<name>_<M>`.
pub fn sample_groups(areas: &[AreaRecord], merge_suffix: bool) -> Vec<SampleGroup> {
    let names: Vec<&str> = areas.iter().map(area_sample).collect();
    let mut out: Vec<SampleGroup> = Vec::new();
    for (a, &n) in areas.iter().zip(&names) {
        let base = strip_suffix(n);
        let joined = merge_suffix && names.iter().filter(|m| **m == base || strip_suffix(m) == base).count() > 1;
        let key = if joined { base } else { n };
        match out.iter_mut().find(|g| g.sample == key) {
            Some(g) => g.areas.push(a.id.clone()),
            None => out.push(SampleGroup { sample: key.to_string(), areas: vec![a.id.clone()] }),
        }
    }
    out
}

/// Where one area's tiles are, in its mosaic and on the stage, and the covered region as a
/// signed distance field.
pub struct AreaGeometry {
    /// (mosaic centre um, stage centre nm) per tile with a stage position.
    tiles: Vec<([f64; 2], [f64; 2])>,
    sign: [f64; 2],
    /// Distance field cell (um) and size.
    cell: f64,
    gw: usize,
    gh: usize,
    /// Signed distance to the coverage edge (um; positive inside).
    sdf: Vec<f32>,
}

impl AreaGeometry {
    /// `None` for an area without placement or stage positions.
    pub fn new(a: &AreaRecord) -> Option<AreaGeometry> {
        let pl = a.placement.as_ref()?;
        let k = a.um_per_px();
        let tiles: Vec<([f64; 2], [f64; 2])> = pl
            .tiles
            .iter()
            .zip(&a.tiles)
            .filter_map(|(t, r)| Some(([(t.pos[0] + t.w as f64 / 2.0) * k, (t.pos[1] + t.h as f64 / 2.0) * k], r.stage_nm?)))
            .collect();
        if tiles.is_empty() {
            return None;
        }
        let cell = 4.0;
        let (mw, mh) = (pl.width as f64 * k, pl.height as f64 * k);
        // One empty cell around the mosaic so its outer edge is an edge.
        let (gw, gh) = ((mw / cell).ceil() as usize + 2, (mh / cell).ceil() as usize + 2);
        let mut cov = vec![false; gw * gh];
        for t in &pl.tiles {
            let (x0, y0) = (t.pos[0] * k / cell, t.pos[1] * k / cell);
            let (x1, y1) = ((t.pos[0] + t.w as f64) * k / cell, (t.pos[1] + t.h as f64) * k / cell);
            for gy in (y0.ceil().max(0.0) as usize)..(y1.floor().max(0.0) as usize).min(gh - 2) {
                for gx in (x0.ceil().max(0.0) as usize)..(x1.floor().max(0.0) as usize).min(gw - 2) {
                    cov[(gy + 1) * gw + gx + 1] = true;
                }
            }
        }
        let inside = crate::segment::morph::distance(&cov, gw, gh);
        let outside = crate::segment::morph::distance(&cov.iter().map(|c| !c).collect::<Vec<bool>>(), gw, gh);
        let sdf = (0..gw * gh).map(|i| if cov[i] { (inside[i] - 0.5) * cell as f32 } else { -(outside[i] - 0.5) * cell as f32 }).collect();
        Some(AreaGeometry { tiles, sign: a.stage_sign, cell, gw, gh, sdf })
    }

    /// Stage position (nm) of a mosaic point (um), through the nearest tile.
    pub fn to_stage(&self, p: [f64; 2]) -> [f64; 2] {
        let (m, s) = self.tiles.iter().min_by(|a, b| d2(a.0, p).total_cmp(&d2(b.0, p))).expect("tiles");
        [s[0] + self.sign[0] * (p[0] - m[0]) * 1000.0, s[1] + self.sign[1] * (p[1] - m[1]) * 1000.0]
    }

    /// Mosaic position (um) of a stage point (nm), through the nearest tile.
    pub fn to_mosaic(&self, s: [f64; 2]) -> [f64; 2] {
        let (m, st) = self.tiles.iter().min_by(|a, b| d2(a.1, s).total_cmp(&d2(b.1, s))).expect("tiles");
        [m[0] + (s[0] - st[0]) / (1000.0 * self.sign[0]), m[1] + (s[1] - st[1]) / (1000.0 * self.sign[1])]
    }

    fn sdf_at(&self, x: f64, y: f64) -> f64 {
        let gx = (x / self.cell).floor() as i64 + 1;
        let gy = (y / self.cell).floor() as i64 + 1;
        if gx < 0 || gy < 0 || gx >= self.gw as i64 || gy >= self.gh as i64 {
            let dx = (-x).max(x - (self.gw - 2) as f64 * self.cell).max(0.0);
            let dy = (-y).max(y - (self.gh - 2) as f64 * self.cell).max(0.0);
            return -dx.hypot(dy) - self.cell;
        }
        self.sdf[gy as usize * self.gw + gx as usize] as f64
    }

    /// Distance from a box (um) to the covered region's edge: the smallest signed distance
    /// along its outline (negative when part of it is outside).
    pub fn interior(&self, b: [f64; 4]) -> f64 {
        let n = (((b[2] - b[0]).max(b[3] - b[1]) / self.cell).ceil() as usize).max(1);
        let mut m = f64::INFINITY;
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let (x, y) = (b[0] + t * (b[2] - b[0]), b[1] + t * (b[3] - b[1]));
            for (px, py) in [(x, b[1]), (x, b[3]), (b[0], y), (b[2], y)] {
                m = m.min(self.sdf_at(px, py));
            }
        }
        m
    }
}

fn d2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

fn find(p: &mut [usize], mut a: usize) -> usize {
    while p[a] != a {
        p[a] = p[p[a]];
        a = p[a];
    }
    a
}

/// Assigns every detection of `grains` (per area id) an owner area.
pub fn assign(areas: &[AreaRecord], grains: &BTreeMap<String, Vec<Grain>>, p: &OwnershipParams) -> Ownership {
    let samples = sample_groups(areas, p.merge_suffix);
    let geo: Vec<Option<AreaGeometry>> = areas.iter().map(AreaGeometry::new).collect();
    // Every detection: (area index, grain), with its stage position, interior and cut flag.
    struct Det<'a> {
        area: usize,
        g: &'a Grain,
        stage: Option<[f64; 2]>,
        interior: f64,
        cut: bool,
    }
    let mut dets: Vec<Det> = Vec::new();
    for (ai, a) in areas.iter().enumerate() {
        for g in grains.get(&a.id).map_or(&[][..], |v| v.as_slice()).iter().filter(|g| g.is_grain()) {
            let (stage, interior) = match &geo[ai] {
                Some(ge) => (Some(ge.to_stage(g.centroid_um)), ge.interior(g.bbox_um)),
                None => (None, f64::INFINITY),
            };
            dets.push(Det { area: ai, g, stage, interior, cut: interior < p.edge_um });
        }
    }
    // Link detections of the same grain in different areas.
    let mut parent: Vec<usize> = (0..dets.len()).collect();
    let mut by_area: Vec<Vec<usize>> = vec![Vec::new(); areas.len()];
    for (i, d) in dets.iter().enumerate() {
        by_area[d.area].push(i);
    }
    for i in 0..dets.len() {
        let Some(s) = dets[i].stage else { continue };
        for (bi, gb) in geo.iter().enumerate() {
            let Some(gb) = gb else { continue };
            if bi == dets[i].area {
                continue;
            }
            let m = gb.to_mosaic(s);
            let r = dets[i].g.eq_diameter_um / 2.0;
            if gb.interior([m[0] - r, m[1] - r, m[0] + r, m[1] + r]) < -r {
                continue; // not in that area
            }
            // Same grain: centres close on the stage, or this detection's centre inside the
            // other area's outline. The second catches a piece of a grain (a CL-dark half, or a
            // grain cut by this mosaic's edge) that the other area found whole: its centre is
            // too far from the whole grain's centre, and printing it unlinked put the grain on
            // paper twice, under two samples (Grenville_XL_80X, HH3 and HH5).
            let best = by_area[bi]
                .iter()
                .map(|&j| (j, d2(dets[j].g.centroid_um, m).sqrt()))
                .filter(|&(j, d)| {
                    d < 0.5 * dets[i].g.eq_diameter_um.max(dets[j].g.eq_diameter_um) + p.match_um
                        || crate::segment::shape::inside(&dets[j].g.polygon_um, m[0], m[1])
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((j, _)) = best {
                let (ra, rb) = (find(&mut parent, i), find(&mut parent, j));
                if ra != rb {
                    parent[ra.max(rb)] = ra.min(rb);
                }
            }
        }
    }
    let score = |d: &Det| d.interior - if d.cut { 1e6 } else { 0.0 };
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..dets.len() {
        let r = find(&mut parent, i);
        clusters.entry(r).or_default().push(i);
    }
    let drawn: Vec<Option<[f64; 4]>> = areas.iter().map(drawn_area_nm).collect();
    let mut out: Vec<Option<GrainOwner>> = vec![None; dets.len()];
    for members in clusters.values() {
        // The operator's drawn rectangles decide first: the grain belongs to the area whose
        // rectangle holds its centre. Where rectangles nest or overlap (HH11 drawn inside HH5 on
        // the Grenville 50X mount), the smallest one is the narrower, deliberate selection.
        let stages: Vec<[f64; 2]> = members.iter().filter_map(|&i| dets[i].stage).collect();
        let centre = (!stages.is_empty()).then(|| {
            let n = stages.len() as f64;
            [stages.iter().map(|s| s[0]).sum::<f64>() / n, stages.iter().map(|s| s[1]).sum::<f64>() / n]
        });
        let rect_area = |r: &[f64; 4]| (r[2] - r[0]) * (r[3] - r[1]);
        // A detection cut by its mosaic edge never owns a grain another area has whole.
        let whole = members.iter().any(|&i| !dets[i].cut);
        let by_drawn = centre.and_then(|c| {
            members
                .iter()
                .filter(|&&i| !(whole && dets[i].cut))
                .filter_map(|&i| drawn[dets[i].area].filter(|r| rect_contains(r, c)).map(|r| (dets[i].area, rect_area(&r))))
                .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
                .map(|(area, _)| area)
        });
        let outside_drawn = by_drawn.is_none() && members.iter().any(|&i| drawn[dets[i].area].is_some());
        // Otherwise (and within the chosen area): the detection farthest inside its covered
        // mosaic; ties to the earlier area, then the earlier grain.
        let best = *members
            .iter()
            .filter(|&&i| by_drawn.is_none_or(|a| dets[i].area == a))
            .max_by(|&&a, &&b| score(&dets[a]).total_cmp(&score(&dets[b])).then(b.cmp(&a)))
            .expect("member");
        let owner_area = dets[best].area;
        for &i in members {
            let d = &dets[i];
            let duplicate_of = if d.area == owner_area {
                None
            } else {
                // The owner area's detection nearest on the stage.
                members
                    .iter()
                    .filter(|&&j| dets[j].area == owner_area)
                    .min_by(|&&a, &&b| {
                        let da = d.stage.zip(dets[a].stage).map_or(f64::INFINITY, |(x, y)| d2(x, y));
                        let db = d.stage.zip(dets[b].stage).map_or(f64::INFINITY, |(x, y)| d2(x, y));
                        da.total_cmp(&db)
                    })
                    .map(|&j| dets[j].g.id)
            };
            out[i] = Some(GrainOwner {
                area_id: areas[d.area].id.clone(),
                grain_id: d.g.id,
                owner_area: areas[owner_area].id.clone(),
                duplicate_of,
                cut: d.cut,
                interior_um: if d.interior.is_finite() { (d.interior * 10.0).round() / 10.0 } else { 0.0 },
                stage_nm: d.stage.map(|s| [s[0].round(), s[1].round()]),
                outside_drawn,
            });
        }
    }
    Ownership { grains: out.into_iter().flatten().collect(), samples }
}
