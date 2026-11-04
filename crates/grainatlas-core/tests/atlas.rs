//! Atlas bundles (WORKFLOW.md section 3.1) on the fixture: layout, hashes, tiles and documents.
mod common;

use std::sync::Arc;

use grainatlas_core::Ctl;
use grainatlas_core::atlas;
use grainatlas_core::project::AreaParams;
use grainatlas_core::project::session::Session;
use grainatlas_core::source::DirSource;

const ID: &str = "01972c4e-0000-7000-8000-00000000a7a5";

fn bundle(quality: u8) -> (Session, Vec<(String, Vec<u8>)>) {
    let mut s = Session::new("t");
    s.open_source(Arc::new(DirSource::new(common::fixture_dir()).unwrap()), "t", "2026-11-03T00:00:00Z").unwrap();
    s.pin_identity(ID, "2026-11-03T00:00:00Z").unwrap();
    s.set_link("01972c4e-0000-7000-8000-0000000000b1", "").unwrap();
    let defaults = serde_json::to_string(&AreaParams::default()).unwrap();
    s.stitch("001_HJ-GC1", &defaults, &Ctl::NONE, &|| 0.0).unwrap();
    let grains = s.segment("001_HJ-GC1", &serde_json::to_string(&grainatlas_core::segment::SegmentParams::default()).unwrap(), "").unwrap();
    let gs: Vec<grainatlas_core::segment::Grain> = serde_json::from_str(&grains).unwrap();
    let g = &gs[0];
    let spot = serde_json::json!({
        "id": "01972c4e-0000-7000-8000-000000000101", "area_id": "001_HJ-GC1", "name": "HJ-GC1-1-1", "sample": "HJ-GC1",
        "grain_id": g.id, "grain_number": g.number, "n": 1, "x_um": g.centroid_um[0], "y_um": g.centroid_um[1],
        "diameter_um": 25.0, "kind": "spot", "acquisition_id": null
    });
    let mut full: serde_json::Value = serde_json::from_str(&s.project_json().unwrap()).unwrap();
    full["grainatlas_ui"] = serde_json::json!({ "grains": { "001_HJ-GC1": gs }, "spots": [spot], "labels": [] });
    let files = s.atlas_files(&full.to_string(), &format!("{{\"quality\":{quality},\"exported\":\"2026-11-03T00:00:00Z\",\"nas_path\":\"/laicpmsdb/sem/Sweden_XS_150X\"}}")).unwrap();
    (s, files)
}

#[test]
fn bundle_layout_hashes_and_documents() {
    let (_s, files) = bundle(100);
    assert_eq!(files[0].0, "atlas.json");
    let a = atlas::verify(&files).unwrap();
    assert_eq!(a["format"], "grainatlas-atlas");
    assert_eq!(a["version"], 1);
    assert_eq!(a["project_id"], ID);
    assert_eq!(a["mount_id"], "01972c4e-0000-7000-8000-0000000000b1");
    assert_eq!(a["opened_from"], "package");
    assert_eq!(a["tile_size"], 256);
    assert_eq!(a["tile_encoding"], "lossless");
    assert_eq!(a["package"]["nas_path"], "/laicpmsdb/sem/Sweden_XS_150X");
    assert_eq!(a["package"]["content_files"], 6);
    assert_eq!(a["package"]["content_sha256"].as_str().unwrap().len(), 64);
    let area = &a["areas"][0];
    assert_eq!(area["path"], "pyramid/001_HJ-GC1");
    let levels = area["levels"].as_array().unwrap();
    assert_eq!(levels.last().unwrap()["cols"], 1);
    // Every tile of every level is in the bundle and listed.
    let tiles: usize = levels.iter().map(|l| (l["cols"].as_u64().unwrap() * l["rows"].as_u64().unwrap()) as usize).sum();
    assert_eq!(files.iter().filter(|(p, _)| p.ends_with(".webp")).count(), tiles);
    assert_eq!(a["files"].as_array().unwrap().len(), files.len() - 1);
    let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
    for f in ["grains.geojson", "spots.json", "scalebars.json", "project.json", "pyramid/001_HJ-GC1/0/0_0.webp"] {
        assert!(names.contains(&f), "{f}");
    }

    let doc = |name: &str| -> serde_json::Value { serde_json::from_slice(&files.iter().find(|(p, _)| p == name).unwrap().1).unwrap() };
    let spots = doc("spots.json");
    assert_eq!(spots["project_id"], ID);
    assert_eq!(spots["sample"], "HJ-GC1");
    let s0 = &spots["spots"][0];
    assert_eq!(s0["name"], "HJ-GC1-1-1");
    assert_eq!(s0["kind"], "spot");
    assert!(s0["grain"].is_string());
    let grains = doc("grains.geojson");
    assert_eq!(grains["type"], "FeatureCollection");
    let f0 = &grains["features"][0];
    assert_eq!(f0["geometry"]["type"], "Polygon");
    assert!(f0["properties"]["label"].is_string());
    let bars = doc("scalebars.json");
    assert!(bars["areas"][0]["grains"].as_array().unwrap().len() > 10);
    let project = doc("project.json");
    assert_eq!(project["uuid"], ID);
    assert_eq!(project["grainatlas_ui"]["spots"][0]["id"], "01972c4e-0000-7000-8000-000000000101");

    // A tampered file fails verification.
    let mut bad = files.clone();
    bad[1].1.push(0);
    assert!(atlas::verify(&bad).is_err());
}

#[test]
fn tiles_decode_to_the_toned_mosaic_and_zip_round_trips() {
    let (s, files) = bundle(100);
    let tile = &files.iter().find(|(p, _)| p == "pyramid/001_HJ-GC1/0/1_0.webp").unwrap().1;
    let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(tile)).unwrap();
    let (w, h) = d.dimensions();
    assert_eq!((w, h), (256, 256));
    let mut buf = vec![0u8; d.output_buffer_size().unwrap()];
    d.read_image(&mut buf).unwrap();
    let stride = buf.len() / (256 * 256);
    // Level 0 tile (1, 0) is mosaic pixels x 256..512, y 0..256 through the display LUT.
    let o = &s.outputs["001_HJ-GC1"];
    let tone = &s.project.as_ref().unwrap().areas[0].params.tone;
    let lut = grainatlas_core::photometric::tone::lut(tone);
    for y in [0usize, 100, 255] {
        for x in [0usize, 17, 255] {
            assert_eq!(buf[(y * 256 + x) * stride], lut[o.base[y * o.linear.w + 256 + x] as usize]);
        }
    }
    let z = atlas::zip(ID, &files).unwrap();
    let back = atlas::zip::read(&z).unwrap();
    assert_eq!(back[0].0, format!("{ID}.atlas/atlas.json"));
    assert_eq!(back.len(), files.len());
    atlas::verify(&back).unwrap();
    // Same files, same bytes.
    assert!(atlas::zip(ID, &files).unwrap() == z);
}

#[test]
fn near_lossless_is_smaller_and_named() {
    let (_, lossless) = bundle(100);
    let (_, near) = bundle(90);
    let size = |f: &[(String, Vec<u8>)]| f.iter().filter(|(p, _)| p.ends_with(".webp")).map(|(_, b)| b.len()).sum::<usize>();
    assert!(size(&near) < size(&lossless));
    let a: serde_json::Value = serde_json::from_slice(&near[0].1).unwrap();
    assert_eq!(a["tile_encoding"], "near-lossless");
    assert_eq!(a["tile_quality"], 90);
}

#[test]
fn package_digest_is_sha256sum_of_tiles() {
    let d = atlas::PackageDigest::of(vec![("b.tif".into(), "22".into()), ("a.tif".into(), "11".into())]);
    assert_eq!(d.files, 2);
    assert_eq!(d.sha256, grainatlas_core::util::sha256_hex(b"11  a.tif\n22  b.tif\n"));
}

#[test]
fn spots_json_is_one_document_per_sample() {
    let mut p: grainatlas_core::project::Project = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "uuid": ID, "name": "p", "created": "", "package": { "path": "", "reader": "tm4000", "name": "p" },
        "areas": [], "settings": {}
    }))
    .unwrap();
    p.name = "p".into();
    let spot = |id: &str, area: &str, sample: &str| -> grainatlas_core::spots::Spot {
        serde_json::from_value(serde_json::json!({ "id": id, "area_id": area, "name": format!("{sample}-{id}"), "sample": sample, "grain_id": null, "grain_number": null, "x_um": 1.0, "y_um": 2.0, "diameter_um": 25.0 })).unwrap()
    };
    let one = atlas::spots_document(&p, &[spot("01972c4e-0000-7000-8000-000000000201", "a", "S1")], &["a", "b"]);
    assert_eq!(one["sample"], "S1");
    assert_eq!(one["spots"][0]["area"], "a");
    let two = atlas::spots_document(&p, &[spot("01972c4e-0000-7000-8000-000000000201", "a", "S1"), spot("01972c4e-0000-7000-8000-000000000202", "b", "S2")], &["a", "b"]);
    assert_eq!(two["project_id"], ID);
    assert_eq!(two["documents"].as_array().unwrap().len(), 2);
    assert_eq!(two["documents"][1]["sample"], "S2");
    assert_eq!(two["documents"][1]["spots"][0]["name"], "S2-01972c4e-0000-7000-8000-000000000202");
}
