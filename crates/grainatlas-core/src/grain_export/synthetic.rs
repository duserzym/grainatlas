//! Code-generated, deterministic intensity distributions; no instrument data.
use super::*;
use crate::{
    image::Gray,
    mosaic::{Placement, TilePlacement},
    project::pipeline,
    readers::{AreaMeta, PackageInfo, TileMeta},
};
/// Two 120 x 96 mosaics, four SEM tiles and 36 rectangular grains each.
/// Area b = 0.6 * area a + 0.05. Stage positions overlap for layer validation.
pub fn mount() -> (Session, Project) {
    let (w, h) = (120, 96);
    let mut areas = Vec::new();
    for id in ["a", "b"] {
        let tiles = (0..4)
            .map(|i| {
                let x = (i % 2) * 60;
                let y = (i / 2) * 48;
                TileMeta {
                    index: i,
                    file: format!("synthetic-{id}-{i}"),
                    w: 60,
                    h: 48,
                    pos_um: [x as f64, y as f64],
                    stage_nm: Some([
                        (1000.0 - x as f64 - 30.0) * 1000.0,
                        (2000.0 - y as f64 - 24.0) * 1000.0,
                    ]),
                    pixel_size_nm: 1000.0,
                    grid: Some([i % 2, i / 2]),
                    meta: BTreeMap::new(),
                }
            })
            .collect();
        areas.push(AreaMeta {
            id: id.into(),
            name: format!("Synthetic {id}"),
            grid: Some([2, 2]),
            pixel_size_nm: 1000.0,
            field_um: None,
            pitch_um: None,
            stage_sign: [-1.0, -1.0],
            tiles,
            meta: BTreeMap::from([
                ("zag.ABCCHistory.SignalType1".into(), "UVD".into()),
                ("AcquisitionDate".into(), "2026-01-01".into()),
                ("Session".into(), "synthetic".into()),
            ]),
        });
    }
    let info = PackageInfo {
        reader: "synthetic".into(),
        kind: "package".into(),
        name: "Synthetic mount".into(),
        areas,
        meta: BTreeMap::new(),
        navigation: None,
    };
    let mut p = Project::new(&info, "", "2026-01-01T00:00:00Z");
    p.uuid = uuid::Uuid::from_u128(1);
    let mut all = BTreeMap::<String, Vec<Grain>>::new();
    let mut base = vec![0.02f32; w * h];
    let mut state = 42u32;
    for i in 0..36 {
        let x = (i % 6) * 20;
        let y = (i / 6) * 16;
        for dy in 0..12 {
            for dx in 0..14 {
                let centre = 0.15 + i as f32 * 0.012;
                let value = match i % 4 {
                    0 => centre,
                    1 => centre + 0.10 * dx as f32 / 13.0,
                    2 => centre + if (dx + dy) % 2 == 0 { 0.0 } else { 0.15 },
                    _ => {
                        let mut uniform = || {
                            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                            (state as f64 + 1.0) / (u32::MAX as f64 + 2.0)
                        };
                        let n = (-2.0 * uniform().ln()).sqrt()
                            * (std::f64::consts::TAU * uniform()).cos();
                        centre + 0.025 * n as f32
                    }
                };
                base[(y + dy) * w + x + dx] = value;
            }
        }
    }
    let mut session = Session::new("synthetic");
    for (ai, a) in p.areas.iter_mut().enumerate() {
        a.uuid = uuid::Uuid::from_u128(10 + ai as u128);
        let placement = Placement {
            width: w,
            height: h,
            tiles: (0..4)
                .map(|i| {
                    let x = ((i % 2) * 60) as i64;
                    let y = ((i / 2) * 48) as i64;
                    TilePlacement {
                        index: i,
                        x,
                        y,
                        w: 60,
                        h: 48,
                        pos: [x as f64, y as f64],
                        prior: [x as f64, y as f64],
                        drift: [0.0; 2],
                        failed: false,
                    }
                })
                .collect(),
            pairs: vec![],
            rms_residual: 0.0,
            max_residual: 0.0,
        };
        a.placement = Some(placement.clone());
        for t in &mut a.tiles {
            t.sha256 = crate::util::sha256_hex(t.file.as_bytes());
        }
        let values: Vec<f32> = base
            .iter()
            .map(|&v| if ai == 0 { v } else { 0.6 * v + 0.05 })
            .collect();
        let display: Vec<u8> = values.iter().map(|v| (v * 255.0).round() as u8).collect();
        let mut o = pipeline::bundle_output(a, w, h, std::sync::Arc::new(display));
        o.linear = Gray { w, h, px: values };
        o.placement = placement;
        a.mosaic = Some(o.mosaic_json.clone());
        session.outputs.insert(a.id.clone(), o);
        let grains=(0..36).map(|i|{let x=((i%6)*20) as f64;let y=((i/6)*16) as f64;serde_json::from_value(json!({
            "id":uuid::Uuid::from_u128(100+ai as u128*100+i as u128),"number":i+1,"label":format!("grain/{:02}",i+1),"sample":"Synthetic","polygon_um":[[x,y],[x+14.0,y],[x+14.0,y+12.0],[x,y+12.0]],"bbox_um":[x,y,x+14.0,y+12.0],"centroid_um":[x+7.0,y+6.0],"area_um2":168.0,"eq_diameter_um":(4.0*168.0/std::f64::consts::PI).sqrt(),"major_um":14.0,"minor_um":12.0,"orientation_deg":0.0,"mean_intensity":0.0,"manual":true,"scale_bar_um":5.0
        })).unwrap()}).collect();
        all.insert(a.id.clone(), grains);
    }
    p.extra
        .insert("grainatlas_ui".into(), json!({"grains":all,"spots":[]}));
    session.project = Some(p.clone());
    (session, p)
}

/// Default scale check: 3,000 native grains (1,500 per area).
pub const DEFAULT_GRAINS_PER_AREA: usize = 1500;
const LARGE_PX: usize = 6000;
const LARGE_SCALE: f64 = 0.5;

/// Two 6,000 × 6,000 px areas at 0.5 µm/px, with 4 × 4 SEM tiles.
/// Both image the same invented mount; b = 0.55 * a + 0.06. Native grain
/// UUIDs differ, so a whole-mount export has four rows per generated outline.
/// This intentionally allocates large mosaics; it is not a normal test fixture.
/// Panics if the requested density cannot be placed after bounded retries.
pub fn large_mount(seed: u64, grains_per_area: usize) -> (Session, Project) {
    let grains = large_grains(seed, grains_per_area);
    let mut rng = Rng(seed ^ 0x74696c6573);
    let tile_adjustments: Vec<_> = (0..16)
        .map(|_| (0.97 + 0.06 * rng.unit(), -0.002 + 0.004 * rng.unit()))
        .collect();
    let areas = ["a", "b"]
        .into_iter()
        .map(|id| AreaMeta {
            id: id.into(),
            name: format!("Synthetic {id}"),
            grid: Some([4, 4]),
            pixel_size_nm: 500.0,
            field_um: None,
            pitch_um: None,
            stage_sign: [-1.0, -1.0],
            tiles: (0..16)
                .map(|i| {
                    let x = (i % 4) * 1500;
                    let y = (i / 4) * 1500;
                    TileMeta {
                        index: i,
                        file: format!("synthetic-large-{id}-{i}"),
                        w: 1500,
                        h: 1500,
                        pos_um: [x as f64 * LARGE_SCALE, y as f64 * LARGE_SCALE],
                        stage_nm: Some([
                            (5000.0 - (x as f64 + 750.0) * LARGE_SCALE) * 1000.0,
                            (6000.0 - (y as f64 + 750.0) * LARGE_SCALE) * 1000.0,
                        ]),
                        pixel_size_nm: 500.0,
                        grid: Some([i % 4, i / 4]),
                        meta: BTreeMap::new(),
                    }
                })
                .collect(),
            meta: BTreeMap::from([
                ("zag.ABCCHistory.SignalType1".into(), "UVD".into()),
                ("Session".into(), "synthetic-large".into()),
                (
                    "synthetic.tile_gain_offset".into(),
                    serde_json::to_string(&tile_adjustments).unwrap(),
                ),
            ]),
        })
        .collect();
    let info = PackageInfo {
        reader: "synthetic".into(),
        kind: "package".into(),
        name: "Synthetic large mount".into(),
        areas,
        meta: BTreeMap::new(),
        navigation: None,
    };
    let mut project = Project::new(&info, "", "2026-01-01T00:00:00Z");
    project.uuid = uuid::Uuid::from_u128(seed as u128 + 1);
    let mut base = vec![0.0f32; LARGE_PX * LARGE_PX];
    for v in &mut base {
        *v = 0.05 + 0.004 * (rng.unit() as f32 - 0.5);
    }
    for g in &grains {
        let bright = rng.unit() < 0.7;
        let mean = if bright { 0.55 } else { 0.35 } + 0.03 * (rng.unit() - 0.5);
        let phase = rng.unit() * std::f64::consts::TAU;
        let (sin, cos) = g.orientation_deg.to_radians().sin_cos();
        let left = (g.bbox_um[0] / LARGE_SCALE).floor() as usize;
        let top = (g.bbox_um[1] / LARGE_SCALE).floor() as usize;
        let right = (g.bbox_um[2] / LARGE_SCALE).ceil() as usize;
        let bottom = (g.bbox_um[3] / LARGE_SCALE).ceil() as usize;
        for y in top..bottom {
            for x in left..right {
                let xx = x as f64 * LARGE_SCALE + LARGE_SCALE / 2.0;
                let yy = y as f64 * LARGE_SCALE + LARGE_SCALE / 2.0;
                if !pixels::inside(&g.polygon_um, xx, yy) {
                    continue;
                }
                let dx = xx - g.centroid_um[0];
                let dy = yy - g.centroid_um[1];
                let u = 2.0 * (cos * dx + sin * dy) / g.major_um;
                let v = 2.0 * (-sin * dx + cos * dy) / g.minor_um;
                let rings = (u.hypot(v) * 24.0 + phase).sin();
                let zoning = 0.045 * rings + 0.025 * u;
                // Sum of uniforms gives smooth, bounded detector-like noise.
                let noise = 0.012 * (rng.unit() + rng.unit() + rng.unit() - 1.5);
                base[y * LARGE_PX + x] = (mean + zoning + noise) as f32;
            }
        }
    }
    for (i, value) in base.iter_mut().enumerate() {
        let tile = (i / LARGE_PX / 1500) * 4 + (i % LARGE_PX / 1500);
        let (gain, offset) = tile_adjustments[tile];
        *value = (*value as f64 * gain + offset) as f32;
    }
    let mut session = Session::new("synthetic-large");
    // Invented f32 mosaics have no source package to rebuild after eviction.
    // Keep their linear and coverage buffers resident for the scale check.
    session.set_memory_budget(2 * LARGE_PX * LARGE_PX * 2 * size_of::<f32>());
    let mut all = BTreeMap::new();
    for (ai, area) in project.areas.iter_mut().enumerate() {
        area.uuid = uuid::Uuid::from_u128(10 + ai as u128);
        let placement = Placement {
            width: LARGE_PX,
            height: LARGE_PX,
            tiles: (0..16)
                .map(|i| {
                    let x = ((i % 4) * 1500) as i64;
                    let y = ((i / 4) * 1500) as i64;
                    TilePlacement {
                        index: i,
                        x,
                        y,
                        w: 1500,
                        h: 1500,
                        pos: [x as f64, y as f64],
                        prior: [x as f64, y as f64],
                        drift: [0.0; 2],
                        failed: false,
                    }
                })
                .collect(),
            pairs: vec![],
            rms_residual: 0.0,
            max_residual: 0.0,
        };
        area.placement = Some(placement.clone());
        for tile in &mut area.tiles {
            tile.sha256 = crate::util::sha256_hex(tile.file.as_bytes());
        }
        let values: Vec<f32> = base
            .iter()
            .map(|&v| if ai == 0 { v } else { 0.55 * v + 0.06 })
            .collect();
        let display = values.iter().map(|v| (v * 255.0).round() as u8).collect();
        let mut output =
            pipeline::bundle_output(area, LARGE_PX, LARGE_PX, std::sync::Arc::new(display));
        output.linear = Gray {
            w: LARGE_PX,
            h: LARGE_PX,
            px: values,
        };
        output.placement = placement;
        area.mosaic = Some(output.mosaic_json.clone());
        session.outputs.insert(area.id.clone(), output);
        let mut native = grains.clone();
        for g in &mut native {
            // Preserve random high bits, reserve the low bit for the area.
            g.id = uuid::Uuid::from_u128((g.id.as_u128() & !1) | ai as u128);
        }
        all.insert(area.id.clone(), native);
    }
    project
        .extra
        .insert("grainatlas_ui".into(), json!({"grains":all,"spots":[]}));
    session.project = Some(project.clone());
    (session, project)
}

/// Small deterministic PRNG, independent of platform or external random crates.
pub(super) struct Rng(pub u64);
impl Rng {
    pub(super) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    pub(super) fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub(super) fn large_grains(seed: u64, count: usize) -> Vec<Grain> {
    const CELL: f64 = 160.0;
    let mut rng = Rng(seed);
    let mut grains: Vec<Grain> = Vec::new();
    let mut cells: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    let cells_of = |b: [f64; 4]| {
        (b[0] as usize / CELL as usize..=b[2] as usize / CELL as usize).flat_map(move |x| {
            (b[1] as usize / CELL as usize..=b[3] as usize / CELL as usize).map(move |y| (x, y))
        })
    };
    for _ in 0..count.saturating_mul(2000) {
        if grains.len() == count {
            break;
        }
        // Many small grains and a tail of long grains, like a sorted mount.
        let length = 30.0 + 120.0 * rng.unit().powi(2);
        let width = length * (0.35 + 0.3 * rng.unit());
        let theta = (rng.unit() - 0.5) * std::f64::consts::PI;
        let centre = [80.0 + 2840.0 * rng.unit(), 80.0 + 2840.0 * rng.unit()];
        let n = 30 + (rng.next() % 31) as usize;
        let phase = rng.unit() * std::f64::consts::TAU;
        let mut local: Vec<_> = (0..n)
            .map(|j| {
                let angle = j as f64 * std::f64::consts::TAU / n as f64;
                let r =
                    1.0 + 0.08 * (3.0 * angle + phase).sin() + 0.04 * (5.0 * angle - phase).cos();
                [r * angle.cos(), r * angle.sin()]
            })
            .collect();
        let bounds = |poly: &[[f64; 2]]| {
            poly.iter().fold(
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
            )
        };
        let b = bounds(&local);
        let (sin, cos) = theta.sin_cos();
        for p in &mut local {
            let x = (p[0] - (b[0] + b[2]) / 2.0) * length / (b[2] - b[0]);
            let y = (p[1] - (b[1] + b[3]) / 2.0) * width / (b[3] - b[1]);
            *p = [centre[0] + cos * x - sin * y, centre[1] + sin * x + cos * y];
        }
        let b = bounds(&local);
        // Disjoint bounding boxes guarantee disjoint polygons, with at least
        // 1 µm of epoxy between them. The grid bounds neighbour searches.
        if cells_of(b).any(|cell| {
            cells.get(&cell).is_some_and(|indices| {
                indices.iter().any(|&i| {
                    let q = grains[i].bbox_um;
                    b[0] < q[2] + 1.0 && b[2] + 1.0 > q[0] && b[1] < q[3] + 1.0 && b[3] + 1.0 > q[1]
                })
            })
        }) {
            continue;
        }
        let mut twice_area = 0.0;
        let mut centroid = [0.0; 2];
        for (a, b) in local.iter().zip(local.iter().cycle().skip(1)).take(n) {
            let cross = a[0] * b[1] - b[0] * a[1];
            twice_area += cross;
            for axis in 0..2 {
                centroid[axis] += (a[axis] + b[axis]) * cross;
            }
        }
        for c in &mut centroid {
            *c /= 3.0 * twice_area;
        }
        let area = twice_area.abs() / 2.0;
        let i = grains.len();
        grains.push(Grain {
            id: uuid::Uuid::from_u128(((rng.next() as u128) << 64) | ((i as u128 + 1) << 1)),
            number: (i + 1) as u32,
            label: Some(format!("grain/{:04}", i + 1)),
            sample: Some("Synthetic".into()),
            polygon_um: local,
            bbox_um: b,
            centroid_um: centroid,
            area_um2: area,
            eq_diameter_um: (4.0 * area / std::f64::consts::PI).sqrt(),
            major_um: length,
            minor_um: width,
            orientation_deg: theta.to_degrees(),
            mean_intensity: 0.0,
            manual: true,
            scale_bar_um: 20.0,
            artifact: None,
            kept: false,
            drawn: false,
            added: false,
        });
        // Expand the lookup footprint too, so the epoxy gap spans cell seams.
        for cell in cells_of([b[0] - 1.0, b[1] - 1.0, b[2] + 1.0, b[3] + 1.0]) {
            cells.entry(cell).or_default().push(i);
        }
    }
    assert_eq!(
        grains.len(),
        count,
        "synthetic mount density exceeds bounded placement retries"
    );
    grains
}
