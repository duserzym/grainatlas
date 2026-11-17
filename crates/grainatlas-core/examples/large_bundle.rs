//! Invented performance fixture, written only beneath ignored target/large-bundle.
//! Run: cargo run -p grainatlas-core --example large_bundle
//! For a debug run exceeding two minutes, use:
//! cargo run --release -p grainatlas-core --example large_bundle
#[allow(dead_code)]
mod support;

use grainatlas_core::{
    project::{Project, session::Session},
    readers::{AreaMeta, PackageInfo},
};
use serde_json::{Map, Value, json};
use std::{f64::consts::TAU, time::Instant};

const SIZES: [(usize, usize); 13] = [
    (7374, 1790),
    (6358, 1777),
    (3309, 1743),
    (4320, 1752),
    (6337, 1767),
    (7338, 1777),
    (7322, 1786),
    (7319, 2530),
    (5292, 1004),
    (3288, 983),
    (6291, 1014),
    (7298, 1780),
    (8276, 1789),
];
const GRAINS_PER_AREA: usize = 98;

fn grain(area: usize, index: usize, w: usize, h: usize) -> Value {
    let serial = area * GRAINS_PER_AREA + index + 1;
    let cx = (index % 14 + 1) as f64 * (w as f64 * 2.0 / 15.0);
    let cy = (index / 14 + 1) as f64 * (h as f64 * 2.0 / 8.0);
    let rx = 25.0 + (serial * 17 % 31) as f64;
    let ry = 22.0 + (serial * 11 % 24) as f64;
    let vertices = 16 + index % 3;
    let polygon: Vec<[f64; 2]> = (0..vertices)
        .map(|v| {
            let angle = v as f64 * TAU / vertices as f64;
            let wobble = 1.0 + 0.06 * (3.0 * angle + serial as f64).sin();
            [
                cx + rx * wobble * angle.cos(),
                cy + ry * wobble * angle.sin(),
            ]
        })
        .collect();
    let mut cross_sum = 0.0;
    let mut centroid = [0.0, 0.0];
    let mut bbox = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (i, p) in polygon.iter().enumerate() {
        let q = polygon[(i + 1) % vertices];
        let cross = p[0] * q[1] - q[0] * p[1];
        cross_sum += cross;
        for axis in 0..2 {
            centroid[axis] += (p[axis] + q[axis]) * cross;
            bbox[axis] = bbox[axis].min(p[axis]);
            bbox[axis + 2] = bbox[axis + 2].max(p[axis]);
        }
    }
    centroid.iter_mut().for_each(|v| *v /= 3.0 * cross_sum);
    let area_um2 = cross_sum / 2.0;
    let artifact = serial.is_multiple_of(20);
    let mut g = json!({
        "id": format!("40000000-0000-4000-8000-{serial:012}"),
        "number": if artifact { 0 } else { index + 1 },
        "label": format!("S{:02}/{}", area + 1, index + 1), "sample": format!("S{:02}", area + 1),
        "polygon_um": polygon, "bbox_um": bbox, "centroid_um": centroid,
        "area_um2": area_um2, "eq_diameter_um": (4.0 * area_um2 / std::f64::consts::PI).sqrt(),
        "major_um": 2.0 * rx.max(ry), "minor_um": 2.0 * rx.min(ry),
        "orientation_deg": if rx >= ry { 0.0 } else { 90.0 },
        "mean_intensity": 0.75, "manual": true, "scale_bar_um": 20.0
    });
    if artifact {
        g["artifact"] = json!("manual");
        g["kept"] = json!(false);
    }
    g
}

fn pixels(area: usize, w: usize, h: usize, grains: &[Value]) -> Vec<u8> {
    // A smooth, cheap texture; the tile index changes both phase and grey level.
    // No repeated tile payloads, and much cheaper to encode than random noise.
    let mut pixels: Vec<u8> = (0..w * h)
        .map(|i| {
            let x = i % w;
            let y = i / w;
            let tile = area * 4096 + (y / 256) * w.div_ceil(256) + x / 256;
            (28 + (tile * 13 % 45) + ((x / 8 + y / 11 + tile * 7) % 31)) as u8
        })
        .collect();
    for g in grains {
        let b = g["bbox_um"].as_array().unwrap();
        let cx = g["centroid_um"][0].as_f64().unwrap() / 2.0;
        let cy = g["centroid_um"][1].as_f64().unwrap() / 2.0;
        let rx = (b[2].as_f64().unwrap() - b[0].as_f64().unwrap()) / 4.0;
        let ry = (b[3].as_f64().unwrap() - b[1].as_f64().unwrap()) / 4.0;
        for y in ((cy - ry).max(0.0) as usize)..=((cy + ry) as usize).min(h - 1) {
            for x in ((cx - rx).max(0.0) as usize)..=((cx + rx) as usize).min(w - 1) {
                let radius = ((x as f64 - cx) / rx).powi(2) + ((y as f64 - cy) / ry).powi(2);
                if radius <= 1.0 {
                    pixels[y * w + x] = (170.0 + 60.0 * (1.0 - radius)) as u8;
                }
            }
        }
    }
    // A 16-bit marker makes each level-0 tile unique even when texture phases
    // repeat across areas. Draw it last so a bright blob cannot erase it.
    for y in (0..h).step_by(256) {
        for x in (0..w).step_by(256) {
            let tile = area * 4096 + (y / 256) * w.div_ceil(256) + x / 256;
            for bit in 0..16.min(w - x) {
                pixels[y * w + x + bit] = 105 + (((tile >> bit) & 1) * 15) as u8;
            }
        }
    }
    pixels
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let info = PackageInfo {
        reader: "synthetic".into(),
        kind: "package".into(),
        name: "Synthetic mount".into(),
        areas: (0..SIZES.len())
            .map(|i| AreaMeta {
                id: format!("a{:02}", i + 1),
                name: format!("Area {:02}", i + 1),
                grid: None,
                pixel_size_nm: 2000.0,
                field_um: None,
                pitch_um: None,
                stage_sign: [1.0, 1.0],
                tiles: vec![],
                meta: Default::default(),
            })
            .collect(),
        meta: Default::default(),
        navigation: None,
    };
    let mut project = Project::new(&info, "", "2026-01-01T00:00:00Z");
    project.uuid = support::PROJECT_ID.parse()?;
    project.upbdb.mount_id = Some(support::MOUNT_ID.parse()?);
    let mut grains = Map::new();
    let mut tone = Map::new();
    for (i, &(w, h)) in SIZES.iter().enumerate() {
        let a = &mut project.areas[i];
        a.uuid = format!("30000000-0000-4000-8000-{:012}", i + 1).parse()?;
        a.sample = format!("S{:02}", i + 1);
        grains.insert(
            a.id.clone(),
            json!(
                (0..GRAINS_PER_AREA)
                    .map(|n| grain(i, n, w, h))
                    .collect::<Vec<_>>()
            ),
        );
        tone.insert(a.id.clone(), serde_json::to_value(&a.params.tone)?);
    }
    let spots: Vec<_> = grains["a01"].as_array().unwrap().iter().take(10).enumerate().map(|(i, g)| json!({
        "id": format!("50000000-0000-4000-8000-{:012}", i + 1), "area_id": "a01",
        "name": format!("S01_{}", i + 1), "sample": "S01", "grain_id": g["id"], "grain_number": g["number"],
        "n": i + 1, "x_um": g["centroid_um"][0], "y_um": g["centroid_um"][1],
        "x2_um": null, "y2_um": null, "diameter_um": 15.0, "kind": "spot", "comment": "",
        "created": "2026-01-01T00:00:00Z", "created_by": ""
    })).collect();
    project.extra.insert(
        "grainatlas_ui".into(),
        json!({ "grains": grains, "spots": spots, "labels": [], "tone": tone }),
    );
    let mut session = Session::new("synthetic");
    session.load_project(&project.to_json())?;
    for (i, &(w, h)) in SIZES.iter().enumerate() {
        let id = &project.areas[i].id;
        let px = pixels(
            i,
            w,
            h,
            project.extra["grainatlas_ui"]["grains"][id]
                .as_array()
                .unwrap(),
        );
        project.areas[i].mosaic = Some(serde_json::from_str(
            &session.load_bundle_area(id, w, h, px)?,
        )?);
        eprintln!("Area {:02}: {w} x {h} px", i + 1);
    }
    session.load_project(&project.to_json())?;
    session.set_package_digest(&"f".repeat(64), 1);
    let files = session.atlas_files(&project.to_json(), "")?;
    let tiles = files
        .iter()
        .filter(|(p, _)| p.contains("/0/") && p.ends_with(".webp"))
        .count();
    assert_eq!(tiles, 2129);
    let atlas: Value =
        serde_json::from_slice(&files.iter().find(|(p, _)| p == "atlas.json").unwrap().1)?;
    let hashes: std::collections::HashSet<_> = atlas["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["path"].as_str().unwrap().contains("/0/"))
        .map(|f| f["sha256"].as_str().unwrap())
        .collect();
    assert_eq!(hashes.len(), tiles, "level-0 tile hashes must be distinct");
    let count = files.len();
    let bytes: usize = files.iter().map(|(_, b)| b.len()).sum();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/large-bundle");
    for (relative, bytes) in files {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, bytes)?;
    }
    println!("{}", root.display());
    println!(
        "{} areas, {tiles} level-0 tiles, {} grains, {count} files, {:.2} MB; {:.2} s",
        SIZES.len(),
        SIZES.len() * GRAINS_PER_AREA,
        bytes as f64 / 1_000_000.0,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
