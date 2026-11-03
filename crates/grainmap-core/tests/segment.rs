use grainmap_core::image::Gray;
use grainmap_core::scale;
use grainmap_core::segment::{self, SegmentParams, WorkImage};

/// Dark epoxy with bright discs; two of them touching (a figure eight) and one long prism.
fn scene() -> WorkImage {
    let (w, h) = (400usize, 300usize);
    let mut g = Gray::filled(w, h, 0.08);
    let disc = |g: &mut Gray, cx: f64, cy: f64, r: f64, v: f32| {
        for y in 0..h {
            for x in 0..w {
                if ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt() <= r {
                    g.px[y * w + x] = v;
                }
            }
        }
    };
    disc(&mut g, 60.0, 60.0, 25.0, 0.7);
    disc(&mut g, 200.0, 70.0, 20.0, 0.5);
    // Touching pair.
    disc(&mut g, 80.0, 200.0, 28.0, 0.6);
    disc(&mut g, 128.0, 200.0, 28.0, 0.65);
    // Long prism with a dark internal zone.
    for y in 160..280 {
        for x in 280..310 {
            g.px[y * w + x] = if (200..230).contains(&y) { 0.2 } else { 0.75 };
        }
    }
    // A speck below the size limit.
    disc(&mut g, 350.0, 40.0, 2.0, 0.9);
    let n = w * h;
    WorkImage { img: g, cov: vec![true; n], um_per_px: 2.0 }
}

#[test]
fn segments_splits_and_numbers() {
    let w = scene();
    let p = SegmentParams { work_scale: 1, ..Default::default() };
    let gs = segment::segment(&w, &p, &[]);
    assert_eq!(gs.len(), 5, "{:?}", gs.iter().map(|g| (g.number, g.centroid_um)).collect::<Vec<_>>());
    // Reading order: top row (two discs), then the pair left to right, then the prism.
    let c: Vec<[f64; 2]> = gs.iter().map(|g| g.centroid_um).collect();
    assert!(c[0][0] < c[1][0] && c[0][1] < 200.0 && c[1][1] < 200.0);
    assert!(c[2][0] < c[3][0] && c[3][0] < c[4][0]);
    // Disc of radius 25 px at 2 um/px: area ≈ π·50² um².
    let a = gs[0].area_um2;
    // Outlines sit at a low threshold on a smoothed image, so they err outwards by ~1 px.
    assert!((a / (std::f64::consts::PI * 2500.0) - 1.0).abs() < 0.12, "{a}");
    assert!((gs[0].eq_diameter_um - 100.0).abs() < 6.0);
    // The prism is long and vertical, and its dark zone did not cut it in two.
    let prism = &gs[4];
    assert!(prism.major_um > 3.0 * prism.minor_um, "{} {}", prism.major_um, prism.minor_um);
    assert!((prism.orientation_deg.abs() - 90.0).abs() < 5.0, "{}", prism.orientation_deg);
    assert_eq!(prism.scale_bar_um, 20.0, "bbox width 60 um → 20 um bar");
    // Polygons are closed rings around the centroid.
    assert!(gs.iter().all(|g| g.polygon_um.len() >= 4 && segment::shape::inside(&g.polygon_um, g.centroid_um[0], g.centroid_um[1])));
}

#[test]
fn numbering_is_stable_across_reruns() {
    let w = scene();
    let p = SegmentParams { work_scale: 1, ..Default::default() };
    let first = segment::segment(&w, &p, &[]);
    // Re-run with a slightly different threshold: same ids and numbers.
    let p2 = SegmentParams { threshold: Some(0.15), ..p.clone() };
    let second = segment::segment(&w, &p2, &first);
    assert_eq!(first.len(), second.len());
    for (a, b) in first.iter().zip(&second) {
        assert_eq!((a.id, a.number), (b.id, b.number));
    }
    // A grain removed from the previous run gets a new number after the highest.
    let partial: Vec<_> = first.iter().filter(|g| g.number != 2).cloned().collect();
    let third = segment::segment(&w, &p, &partial);
    let new = third.iter().find(|g| !partial.iter().any(|q| q.id == g.id)).unwrap();
    assert_eq!(new.number, 6);
}

#[test]
fn manual_tools() {
    let w = scene();
    let p = SegmentParams { work_scale: 1, ..Default::default() };
    // Click-grow on the first disc.
    let g = segment::grain_at(&w, &p, 120.0, 120.0).expect("grain");
    assert!((g.centroid_um[0] - 121.0).abs() < 4.0 && (g.centroid_um[1] - 121.0).abs() < 4.0, "{:?}", g.centroid_um);
    assert!(g.manual);
    assert!(segment::grain_at(&w, &p, 700.0, 500.0).is_none(), "background click");
    // Split the first disc vertically, then merge the halves back.
    let parts = segment::split_grain(&w, &g, [120.0, 0.0, 120.0, 300.0], 0.8);
    assert_eq!(parts.len(), 2);
    let merged = segment::merge_grains(&w, &parts, 0.8).expect("merged");
    assert!((merged.area_um2 / g.area_um2 - 1.0).abs() < 0.08, "{} vs {}", merged.area_um2, g.area_um2);
    // Renumber from the third grain onwards starting at 10.
    let mut gs = segment::segment(&w, &p, &[]);
    let id3 = gs[2].id;
    segment::renumber(&mut gs, 0.6, Some((id3, 10)));
    assert_eq!(gs.iter().map(|g| g.number).collect::<Vec<_>>(), vec![1, 2, 10, 11, 12]);
}

#[test]
fn grabcut_refinement_keeps_the_grain() {
    let w = scene();
    let p = SegmentParams { work_scale: 1, grabcut: true, ..Default::default() };
    let gs = segment::segment(&w, &p, &[]);
    assert_eq!(gs.len(), 5);
    let a = gs[0].area_um2;
    assert!((a / (std::f64::consts::PI * 2500.0) - 1.0).abs() < 0.12, "{a}");
}

#[test]
fn csv_and_label_image() {
    let w = scene();
    let gs = segment::segment(&w, &SegmentParams { work_scale: 1, ..Default::default() }, &[]);
    let csv = segment::grains_csv(&gs, |x, y| Some([x * 1000.0, y * 1000.0]), |_| 0);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 6);
    assert!(lines[0].starts_with("number,id,x_um,y_um,area_um2"));
    let lab = segment::label_image(&w, &gs);
    assert_eq!(lab[60 * 400 + 60], 1);
    assert_eq!(lab[5 * 400 + 5], 0);
}

#[test]
fn scale_helpers() {
    assert_eq!(scale::grain_bar_um(100.0), 20.0);
    assert_eq!(scale::grain_bar_um(10.0), 10.0);
    assert_eq!(scale::grain_bar_um(2000.0), 500.0);
    assert_eq!(scale::round_bar_um(740.0), 500.0);
    assert_eq!(scale::round_bar_um(0.3), 0.2);
    assert!((scale::from_two_points([0.0, 0.0], [300.0, 400.0], 250.0).unwrap() - 500.0).abs() < 1e-9);
    // A burned-in bar in the bottom band.
    let mut g = Gray::filled(200, 100, 0.1);
    for x in 40..140 {
        g.px[95 * 200 + x] = 1.0;
    }
    assert_eq!(scale::detect_bar(&g, 0.1), Some((40, 140, 95)));
}
