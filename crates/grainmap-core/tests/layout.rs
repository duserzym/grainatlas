mod common;

use std::collections::BTreeMap;

use common::fixture;
use grainmap_core::Ctl;
use grainmap_core::layout::{self, Items, LayoutParams};
use grainmap_core::project::pipeline::{self, Timings};
use grainmap_core::project::{AreaParams, Project};
use grainmap_core::spots::{self, Settings};

/// Prints the fixture on Tabloid: pages carry the 10 mm paper bar and a um bar at image scale,
/// and the PDF has Tabloid pages (792 x 1224 pt).
#[test]
fn prints_fixture_grains_on_tabloid() {
    let pkg = fixture();
    let mut project = Project::new(pkg.info(), "fixture", "2026-11-03T00:00:00Z");
    let rec = &mut project.areas[0];
    let tiles = pipeline::read_tiles(pkg.as_ref(), 0, rec).unwrap();
    let out = pipeline::run(rec, &tiles, &AreaParams::default(), &Ctl::NONE, &|| 0.0, &mut Timings::default()).unwrap();
    let grains = pipeline::segment_area(rec, &out, &Default::default(), &[]);
    assert!(grains.len() > 10);
    let sp = spots::new_spot(&rec.id, "HJ-GC1", &grains, &[], &Settings::default(), grains[0].centroid_um[0], grains[0].centroid_um[1], 25.0, "");
    let rec = &project.areas[0];
    let img = pipeline::area_image(rec, &out, &Default::default());
    let mut gmap = BTreeMap::new();
    gmap.insert(rec.id.clone(), grains.clone());

    let p = LayoutParams { um_per_mm: 10.0, title: "HJ-GC1 test".into(), ..Default::default() };
    let r = layout::layout(&p, &[img], &gmap, &[sp.clone()], &[]);
    assert_eq!(r.paper_mm, [279.4, 431.8]);
    assert_eq!(r.um_per_mm, 10.0);
    assert_eq!(r.items, grains.len());
    assert!(!r.pages.is_empty());
    let first = &r.pages[0];
    assert!(first.starts_with("<svg") && first.contains(r#"width="279.4mm""#));
    // 10 mm paper bar and a 200 um image bar (round length under 40 mm x 10 um/mm = 400 um → 200).
    assert!(first.contains(r#"h10 "#) && first.contains("10 mm (paper)"));
    assert!(first.contains("200 µm (sample)"));
    assert!(first.contains(&format!("{}", sp.name)), "spot name printed");
    assert!(first.contains(&format!("{} / {}", 1, r.pages.len())));

    let pdf = layout::pdf::pdf_from_svgs(&r.pages).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    let text = String::from_utf8_lossy(&pdf);
    let mb: Vec<&str> = text.match_indices("/MediaBox").map(|(i, _)| &text[i..(i + 40).min(text.len())]).collect();
    eprintln!("{mb:?}");
    assert_eq!(mb.len(), r.pages.len(), "one page per SVG");
    assert!(mb[0].starts_with("/MediaBox [0 0 792 1224") || mb[0].starts_with("/MediaBox [0 0 792.0 1224.0"), "tabloid media box: {}", mb[0]);

    // Fit scale on Letter with whole areas: one page, nice scale.
    let img = pipeline::area_image(rec, &out, &Default::default());
    let p2 = LayoutParams { paper: "letter".into(), items: Items::Areas, ..Default::default() };
    let r2 = layout::layout(&p2, &[img], &gmap, &[], &[]);
    assert_eq!(r2.pages.len(), 1);
    assert!([10.0, 20.0, 25.0, 50.0].contains(&r2.um_per_mm), "{}", r2.um_per_mm);
}

#[test]
fn oversized_items_are_tiled_with_registration_marks() {
    let px = vec![128u8; 400 * 300];
    let img = layout::AreaImage { id: "a", name: "A", sample: "A", w: 400, h: 300, px: &px, lut: std::array::from_fn(|i| i as u8), um_per_px: 10.0 };
    // 4000 x 3000 um at 10 um/mm = 400 x 300 mm: larger than A4.
    let p = LayoutParams { paper: "a4".into(), items: Items::Areas, um_per_mm: 10.0, ..Default::default() };
    let r = layout::layout(&p, &[img], &BTreeMap::new(), &[], &[]);
    assert!(r.items >= 4, "{}", r.items);
    assert!(r.pages.iter().any(|s| s.contains("#e3b341")), "registration marks");
    assert!(r.pages.iter().any(|s| s.contains("(1/")));
}

#[test]
fn nice_scales() {
    assert_eq!(layout::nice_scale(3.2), 5.0);
    assert_eq!(layout::nice_scale(11.0), 20.0);
    assert_eq!(layout::nice_scale(21.0), 25.0);
    assert_eq!(layout::nice_scale(100.0), 100.0);
    assert_eq!(layout::paper("A3").unwrap().h_mm, 420.0);
}
