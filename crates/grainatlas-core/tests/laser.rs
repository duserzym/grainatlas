//! Laser registration through the Session both front ends use: fit, store in project.json, export.
mod common;

use std::sync::Arc;

use grainatlas_core::project::Project;
use grainatlas_core::project::session::{self, Session};
use grainatlas_core::source::DirSource;

#[test]
fn registration_is_stored_in_the_project_and_exports_the_generic_csv() {
    let mut s = Session::new("t");
    s.open_source(Arc::new(DirSource::new(common::fixture_dir()).unwrap()), "t", "2026-11-03T00:00:00Z").unwrap();
    // Stage = 1 µm → 0.001 mm, y flipped, offset (12.5, 40.25) mm.
    let stage = |x: f64, y: f64| (12.5 + x / 1000.0, 40.25 - y / 1000.0);
    let pts = [(100.0, 80.0), (2000.0, 150.0), (900.0, 1100.0)];
    let fids: Vec<serde_json::Value> = pts.iter().map(|&(x, y)| serde_json::json!({ "x_um": x, "y_um": y, "x_stage": stage(x, y).0, "y_stage": stage(x, y).1 })).collect();
    let fit: serde_json::Value = serde_json::from_str(&session::laser_fit(&serde_json::to_string(&fids).unwrap(), "mm").unwrap()).unwrap();
    assert_eq!(fit["exact"], true);
    assert_eq!(fit["mirrored"], true);
    let reg = serde_json::json!({ "units": "mm", "adapter": "generic", "fiducials": fids, "fit": fit });
    s.set_laser("001_HJ-GC1", &reg.to_string()).unwrap();

    let p = Project::from_json(&s.project_json().unwrap()).unwrap();
    assert_eq!(p.laser.areas["001_HJ-GC1"].fiducials.len(), 3);
    let back: serde_json::Value = serde_json::from_str(&s.project_json().unwrap()).unwrap();
    assert_eq!(back["laser"]["areas"]["001_HJ-GC1"]["units"], "mm");

    let spots = serde_json::json!([
        { "id": "01972c4e-0000-7000-8000-000000000101", "area_id": "001_HJ-GC1", "name": "HJ-GC1-1-1", "sample": "HJ-GC1", "grain_id": null, "grain_number": 1, "x_um": 500.0, "y_um": 250.0, "x2_um": null, "y2_um": null, "diameter_um": 25.0, "kind": "spot" },
        { "id": "01972c4e-0000-7000-8000-000000000102", "area_id": "001_HJ-GC1", "name": "HJ-GC1-2-2", "sample": "HJ-GC1", "grain_id": null, "grain_number": null, "x_um": 700.0, "y_um": 300.0, "x2_um": 760.0, "y2_um": 300.0, "diameter_um": 15.0, "kind": "line" },
        { "id": "01972c4e-0000-7000-8000-000000000103", "area_id": "other", "name": "X", "sample": "", "grain_id": null, "grain_number": null, "x_um": 1.0, "y_um": 1.0, "x2_um": null, "y2_um": null, "diameter_um": 15.0, "kind": "spot" }
    ]);
    let csv = s.laser_export(&spots.to_string(), "001_HJ-GC1", "").unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], "name,x_stage,y_stage,x_um,y_um,diameter_um,kind,grain,sample,x2_stage,y2_stage");
    assert_eq!(lines[1], "HJ-GC1-1-1,13.0000,40.0000,500.00,250.00,25,spot,1,HJ-GC1,,");
    assert_eq!(lines[2], "HJ-GC1-2-2,13.2000,39.9500,700.00,300.00,15,line,,HJ-GC1,13.2600,39.9500");
    assert_eq!(lines.len(), 3);
    assert!(s.laser_export(&spots.to_string(), "002_none", "").is_err());
    assert!(session::laser_adapters().contains("\"generic\""));
}
