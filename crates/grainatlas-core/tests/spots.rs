mod common;

use std::collections::BTreeMap;

use grainatlas_core::project::Project;
use grainatlas_core::segment::Grain;
use grainatlas_core::spots::{self, Settings, Spot, SpotKind};

fn grain(n: u32, cx: f64, cy: f64) -> Grain {
    Grain {
        id: grainatlas_core::project::new_id(),
        number: n,
        polygon_um: vec![[cx - 20.0, cy - 30.0], [cx + 20.0, cy - 30.0], [cx + 20.0, cy + 30.0], [cx - 20.0, cy + 30.0]],
        bbox_um: [cx - 20.0, cy - 30.0, cx + 20.0, cy + 30.0],
        centroid_um: [cx, cy],
        area_um2: 2400.0,
        eq_diameter_um: 55.3,
        major_um: 60.0,
        minor_um: 40.0,
        orientation_deg: 90.0,
        mean_intensity: 0.5,
        manual: false,
        scale_bar_um: 10.0,
    }
}

#[test]
fn naming_patterns() {
    assert_eq!(spots::format_name("{sample}-{grain}-{n}", "AH1", Some(12), 3), "AH1-12-3");
    assert_eq!(spots::format_name("{sample}_{n:03}", "WS2", None, 7), "WS2_007");
    assert_eq!(spots::format_name("{sample}-g{grain:02}-{n}", "KD", Some(4), 10), "KD-g04-10");
    assert_eq!(spots::format_name("{sample}-{grain}-{n}", "X", None, 1), "X-0-1");
    assert_eq!(spots::format_name("plain {unknown} {n", "X", None, 1), "plain {unknown} {n");
}

#[test]
fn spots_continue_numbering_and_link_grains() {
    let gs = vec![grain(1, 100.0, 100.0), grain(2, 300.0, 100.0)];
    let st = Settings::default();
    let mut list: Vec<Spot> = Vec::new();
    for (x, y) in [(100.0, 100.0), (300.0, 110.0), (500.0, 500.0)] {
        let s = spots::new_spot("a1", "AH1", &gs, &list, &st, x, y, 25.0, "2026-11-03T00:00:00Z");
        list.push(s);
    }
    assert_eq!(list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["AH1-1-1", "AH1-2-2", "AH1-0-3"]);
    assert_eq!(list[1].grain_id, Some(gs[1].id));
    assert_eq!(list[2].grain_id, None);
    // Deleting a spot does not reuse its number: continuation from the highest.
    list.remove(1);
    let s = spots::new_spot("a1", "AH1", &gs, &list, &st, 110.0, 90.0, 25.0, "");
    assert_eq!(s.n, 4);
    // Ids are UUIDv7.
    assert_eq!(s.id.get_version_num(), 7);
}

#[test]
fn csv_and_geojson_exports() {
    let pkg = common::fixture();
    let mut project = Project::new(pkg.info(), "fixture", "2026-11-03T00:00:00Z");
    let gs = vec![grain(1, 100.0, 100.0)];
    let st = Settings::default();
    let mut s = spots::new_spot("001_HJ-GC1", "HJ-GC1", &gs, &[], &st, 100.0, 100.0, 25.0, "2026-11-03T00:00:00Z");
    s.comment = "rim, \"bright\"".into();
    let mut line = spots::new_spot("001_HJ-GC1", "HJ-GC1", &gs, &[s.clone()], &st, 90.0, 80.0, 10.0, "");
    line.kind = SpotKind::Line;
    line.x2_um = Some(110.0);
    line.y2_um = Some(120.0);
    project.areas[0].placement = None;
    let csv = spots::spots_csv(&project, &[s.clone(), line.clone()]);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("id,name,sample,area,grain,grain_id,x_um,y_um,x_px,y_px,diameter_um"));
    assert!(lines[1].contains("HJ-GC1-1-1"));
    assert!(lines[1].contains("\"rim, \"\"bright\"\"\""));
    // 100 um at 661.4583 nm/px.
    assert!(lines[1].contains(",151.2,151.2,"), "{}", lines[1]);
    let mut grains = BTreeMap::new();
    grains.insert("001_HJ-GC1".to_string(), gs);
    let gj = spots::geojson(&project, &grains, &[s, line]);
    assert_eq!(gj["type"], "FeatureCollection");
    let f = gj["features"].as_array().unwrap();
    assert_eq!(f.len(), 3);
    assert_eq!(f[0]["geometry"]["type"], "Polygon");
    assert_eq!(f[0]["geometry"]["coordinates"][0].as_array().unwrap().len(), 5, "ring closed");
    assert_eq!(f[1]["geometry"]["type"], "Point");
    assert_eq!(f[2]["geometry"]["type"], "LineString");
    assert_eq!(f[1]["properties"]["name"], "HJ-GC1-1-1");
}
