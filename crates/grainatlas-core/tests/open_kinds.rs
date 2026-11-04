//! The three ways of opening (ADR-0006): a whole package, one area folder on its
//! own, and loose images with a scale the user gives.

use std::path::PathBuf;
use std::sync::Arc;

use grainatlas_core::Ctl;
use grainatlas_core::project::Project;
use grainatlas_core::project::session::Session;
use grainatlas_core::source::{DirSource, MemSource};

fn package() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tm4000/Sweden_XS_150X")
}

fn stitch(s: &mut Session, id: &str) -> serde_json::Value {
    serde_json::from_str(&s.stitch(id, "", &Ctl::NONE, &|| 0.0).unwrap()).unwrap()
}

fn opened(json: &str) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(v["status"], "opened", "{v}");
    v["info"].clone()
}

#[test]
fn package_folder_is_kind_package() {
    let mut s = Session::new("t");
    let info = opened(&s.open_source(Arc::new(DirSource::new(package()).unwrap()), "pkg", "now").unwrap());
    assert_eq!(info["kind"], "package");
    let p = Project::from_json(&s.project_json().unwrap()).unwrap();
    assert_eq!(p.package.kind, "package");
    assert!(p.package.loose.is_none());
}

#[test]
fn area_folder_alone_is_a_one_area_package_with_stage_calibration() {
    let mut whole = Session::new("t");
    opened(&whole.open_source(Arc::new(DirSource::new(package()).unwrap()), "pkg", "now").unwrap());
    let full = stitch(&mut whole, "001_HJ-GC1");

    let mut s = Session::new("t");
    let info = opened(&s.open_source(Arc::new(DirSource::new(package().join("001_HJ-GC1")).unwrap()), "area", "now").unwrap());
    assert_eq!(info["kind"], "area");
    assert_eq!(info["name"], "001_HJ-GC1");
    let a = &info["areas"][0];
    assert_eq!(info["areas"].as_array().unwrap().len(), 1);
    assert_eq!(a["id"], "001_HJ-GC1");
    assert_eq!(a["name"], "HJ-GC1", "the name spot names use, as in the package");
    assert_eq!(a["grid"], serde_json::json!([3, 2]));
    assert!((a["pixel_size_nm"].as_f64().unwrap() - 661.4583).abs() < 1e-6);
    assert_eq!(a["tiles"][0]["stage_nm"], serde_json::json!([14954450.0, 19902350.0]));
    // Same tiles, same calibration: the same mosaic as from the package.
    let r = stitch(&mut s, "001_HJ-GC1");
    assert_eq!(r["mosaic"]["sha256"], full["mosaic"]["sha256"]);
    assert_eq!(Project::from_json(&s.project_json().unwrap()).unwrap().package.kind, "area");
}

/// Tiles 0, 1 and 2 (the top row) as loose JPEGs, no sidecars.
fn loose_three() -> MemSource {
    let mut m = MemSource::new("images");
    for i in 0..3 {
        let f = format!("001_HJ-GC1_000{i}.jpg");
        m.insert(&f, std::fs::read(package().join("001_HJ-GC1").join(&f)).unwrap());
    }
    m
}

#[test]
fn loose_images_ask_for_a_scale_then_open_as_single_tile_areas() {
    let mut s = Session::new("images");
    let v: serde_json::Value = serde_json::from_str(&s.open_source(Arc::new(loose_three()), "images", "now").unwrap()).unwrap();
    assert_eq!(v["status"], "needs_scale");
    assert_eq!(v["name"], "001_HJ-GC1");
    assert_eq!(v["images"].as_array().unwrap().len(), 3);
    assert_eq!(v["images"][0]["w"], 1280);
    assert!(s.project_json().is_err(), "no project before the scale is known");

    let info: serde_json::Value = serde_json::from_str(&s.open_loose(r#"{"um_per_px":0.6614583}"#, "now").unwrap()).unwrap();
    assert_eq!(info["kind"], "loose");
    let areas = info["areas"].as_array().unwrap();
    assert_eq!(areas.len(), 3);
    assert_eq!(areas[1]["id"], "001_HJ-GC1_0001");
    assert!((areas[1]["pixel_size_nm"].as_f64().unwrap() - 661.4583).abs() < 1e-6);
    for a in areas {
        let r = stitch(&mut s, a["id"].as_str().unwrap());
        assert_eq!(r["mosaic"]["w"], 1280);
        assert_eq!(r["mosaic"]["h"], 960);
        assert!((r["mosaic"]["um_per_px"].as_f64().unwrap() - 0.6614583).abs() < 1e-9);
    }
    // Downstream: grains on a single-tile area.
    let seg = s.segment("001_HJ-GC1_0000", &serde_json::to_string(&grainatlas_core::segment::SegmentParams::default()).unwrap(), "").unwrap();
    assert!(serde_json::from_str::<Vec<serde_json::Value>>(&seg).unwrap().len() > 3);
    let p = Project::from_json(&s.project_json().unwrap()).unwrap();
    assert_eq!(p.package.kind, "loose");
    assert_eq!(p.package.loose.as_ref().unwrap().um_per_px, 0.6614583);
}

#[test]
fn loose_field_width_sets_the_scale() {
    let mut s = Session::new("images");
    s.open_source(Arc::new(loose_three()), "images", "now").unwrap();
    let info: serde_json::Value = serde_json::from_str(&s.open_loose(r#"{"um_per_px":0,"field_width_um":846.666}"#, "now").unwrap()).unwrap();
    assert!((info["areas"][0]["pixel_size_nm"].as_f64().unwrap() - 846_666.0 / 1280.0).abs() < 1e-6);
    assert!(s.open_loose(r#"{"um_per_px":0}"#, "now").is_err(), "a scale is required");
}

#[test]
fn loose_images_as_one_grid_stitch() {
    let mut s = Session::new("images");
    s.open_source(Arc::new(loose_three()), "images", "now").unwrap();
    let spec = r#"{"um_per_px":0.6614583,"grid":{"cols":3,"rows":1,"overlap_pct":20,"order":"rows","name":"row0"}}"#;
    let info: serde_json::Value = serde_json::from_str(&s.open_loose(spec, "now").unwrap()).unwrap();
    assert_eq!(info["areas"].as_array().unwrap().len(), 1);
    assert_eq!(info["areas"][0]["id"], "row0");
    let r = stitch(&mut s, "row0");
    let pl = &r["placement"];
    assert!(pl["pairs"].as_array().unwrap().iter().all(|p| p["used"] == true), "both overlaps measured: {pl}");
    assert!(pl["rms_residual"].as_f64().unwrap() < 1.0);
    // Three 1280-px tiles at ~20 % overlap (the real pitch is 676 of 846.7 um).
    let w = r["mosaic"]["w"].as_u64().unwrap();
    assert!((3300..3400).contains(&w), "mosaic width {w}");
    let p = Project::from_json(&s.project_json().unwrap()).unwrap();
    assert_eq!(p.package.loose.unwrap().grid.unwrap().cols, 3);

    // Too many images for the stated grid.
    assert!(s.open_loose(r#"{"um_per_px":1,"grid":{"cols":1,"rows":2,"overlap_pct":10}}"#, "now").is_err());
}

#[test]
fn reopening_with_the_saved_project_keeps_it() {
    let mut s = Session::new("images");
    s.open_source(Arc::new(loose_three()), "images", "now").unwrap();
    s.open_loose(r#"{"um_per_px":0.66}"#, "now").unwrap();
    let saved = s.project_json().unwrap();
    let uuid = Project::from_json(&saved).unwrap().uuid;

    let mut t = Session::new("images");
    t.load_project(&saved).unwrap();
    t.open_source(Arc::new(loose_three()), "images", "now").unwrap();
    let spec = serde_json::to_string(&Project::from_json(&saved).unwrap().package.loose.unwrap()).unwrap();
    t.open_loose(&spec, "now").unwrap();
    assert_eq!(Project::from_json(&t.project_json().unwrap()).unwrap().uuid, uuid);

    // A different arrangement is a different project.
    t.load_project(&saved).unwrap();
    t.open_loose(r#"{"um_per_px":0.66,"grid":{"cols":3,"rows":1,"overlap_pct":20}}"#, "now").unwrap();
    assert_ne!(Project::from_json(&t.project_json().unwrap()).unwrap().uuid, uuid);
}
