//! One-click auto tone and per-grain tone normalisation.

use std::path::PathBuf;
use std::sync::Arc;

use grainatlas_core::Ctl;
use grainatlas_core::image::Gray;
use grainatlas_core::photometric::grain_norm::{GrainNorm, normalize};
use grainatlas_core::project::AreaParams;
use grainatlas_core::project::session::Session;
use grainatlas_core::segment::{Grain, SegmentParams};
use grainatlas_core::source::DirSource;

const AREA: &str = "001_HJ-GC1";

fn stitched() -> Session {
    let mut s = Session::new("t");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tm4000/Sweden_XS_150X");
    s.open_source(Arc::new(DirSource::new(root).unwrap()), "pkg", "now").unwrap();
    s.stitch(AREA, "", &Ctl::NONE, &|| 0.0).unwrap();
    s
}

fn sha(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json).unwrap()["mosaic"]["sha256"].as_str().unwrap().to_string()
}

#[test]
fn auto_turns_on_leveling_and_clahe_with_safe_parameters() {
    let mut s = stitched();
    let mut p0 = AreaParams::default();
    p0.photometric.background = false;
    let h0 = sha(&s.recompute(AREA, &serde_json::to_string(&p0).unwrap(), &Ctl::NONE, &|| 0.0).unwrap());
    let auto: AreaParams = serde_json::from_str(&s.auto_params(AREA, &serde_json::to_string(&p0).unwrap()).unwrap()).unwrap();
    assert!(auto.photometric.background);
    let t = &auto.tone;
    assert!(t.clahe);
    assert!((1.5..=3.0).contains(&t.clahe_clip), "clip {}", t.clahe_clip);
    assert!((64..=512).contains(&t.clahe_tile) && t.clahe_tile % 16 == 0, "tile {}", t.clahe_tile);
    assert!(t.low_pct <= 1.0 && t.high_pct >= 99.0);
    assert_eq!((t.black, t.white, t.gamma), (0.0, 1.0, 1.0));
    // Stitch parameters untouched: applying it never re-stitches.
    assert_eq!(auto.stitch, p0.stitch);
    let h1 = sha(&s.recompute(AREA, &serde_json::to_string(&auto).unwrap(), &Ctl::NONE, &|| 0.0).unwrap());
    assert_ne!(h0, h1);
    // Deterministic, and a second click changes nothing (Auto of the Auto result is the same).
    let again: AreaParams = serde_json::from_str(&s.auto_params(AREA, &serde_json::to_string(&p0).unwrap()).unwrap()).unwrap();
    assert_eq!(again, auto);
    let twice: AreaParams = serde_json::from_str(&s.auto_params(AREA, &serde_json::to_string(&auto).unwrap()).unwrap()).unwrap();
    assert_eq!(twice, auto);
}

fn square(id: u128, x0: f64, y0: f64, side: f64) -> Grain {
    Grain {
        id: uuid::Uuid::from_u128(id),
        number: id as u32,
        polygon_um: vec![[x0, y0], [x0 + side, y0], [x0 + side, y0 + side], [x0, y0 + side]],
        bbox_um: [x0, y0, x0 + side, y0 + side],
        centroid_um: [x0 + side / 2.0, y0 + side / 2.0],
        area_um2: side * side,
        eq_diameter_um: 0.0,
        major_um: side,
        minor_um: side,
        orientation_deg: 0.0,
        mean_intensity: 0.0,
        manual: false,
        scale_bar_um: 0.0,
        artifact: None,
        kept: false,
    }
}

/// Two textured grains, one dim and flat, one bright and contrasty, on a dark background.
fn two_grains() -> (Gray, Vec<Grain>) {
    let (w, h) = (100, 50);
    let mut g = Gray { w, h, px: vec![0.05; w * h] };
    for y in 5..45 {
        for x in 5..45 {
            g.px[y * w + x] = 0.30 + 0.05 * (((x * 7 + y * 3) % 10) as f32 / 10.0);
        }
        for x in 55..95 {
            g.px[y * w + x] = 0.60 + 0.30 * (((x * 7 + y * 3) % 10) as f32 / 10.0);
        }
    }
    (g, vec![square(1, 5.0, 5.0, 40.0), square(2, 55.0, 5.0, 40.0)])
}

fn median_in(g: &Gray, x0: usize, x1: usize) -> f32 {
    let mut v: Vec<f32> = (15..35).flat_map(|y| (x0..x1).map(move |x| (y, x))).map(|(y, x)| g.px[y * g.w + x]).collect();
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

#[test]
fn grain_normalisation_matches_grains_inside_masks_only() {
    let (g0, grains) = two_grains();
    let gap0 = (median_in(&g0, 65, 85) - median_in(&g0, 15, 35)).abs();

    let mut g = g0.clone();
    let p = GrainNorm { enabled: true, strength: 1.0, reference: None, feather_um: 2.0 };
    assert_eq!(normalize(&mut g, &grains, 1.0, &p), 2);
    let gap = (median_in(&g, 65, 85) - median_in(&g, 15, 35)).abs();
    assert!(gap < gap0 * 0.2, "medians pulled together: {gap0} -> {gap}");
    // Outside the grains nothing changes; at the very edge the soft edge keeps it close.
    for y in 0..50 {
        for x in [0usize, 2, 48, 50, 52, 97, 99] {
            assert_eq!(g.px[y * 100 + x], g0.px[y * 100 + x]);
        }
    }
    assert!((g.px[25 * 100 + 5] - g0.px[25 * 100 + 5]).abs() < (g.px[25 * 100 + 25] - g0.px[25 * 100 + 25]).abs());

    // Reference grain: the other grain takes its median.
    let mut r = g0.clone();
    normalize(&mut r, &grains, 1.0, &GrainNorm { reference: Some(grains[1].id), ..p.clone() });
    assert!((median_in(&r, 15, 35) - median_in(&g0, 65, 85)).abs() < 0.03);
    assert!((median_in(&r, 65, 85) - median_in(&g0, 65, 85)).abs() < 1e-6);

    // Strength 0 and disabled are no-ops; strength 0.5 lands in between.
    for q in [GrainNorm { strength: 0.0, ..p.clone() }, GrainNorm { enabled: false, ..p.clone() }] {
        let mut z = g0.clone();
        normalize(&mut z, &grains, 1.0, &q);
        assert_eq!(z.px, g0.px);
    }
    let mut half = g0.clone();
    normalize(&mut half, &grains, 1.0, &GrainNorm { strength: 0.5, ..p });
    let gh = (median_in(&half, 65, 85) - median_in(&half, 15, 35)).abs();
    assert!(gh < gap0 && gh > gap, "{gap} < {gh} < {gap0}");
}

#[test]
fn grain_normalisation_in_the_session_changes_only_grain_pixels() {
    let mut s = stitched();
    let grains = s.segment(AREA, &serde_json::to_string(&SegmentParams::default()).unwrap(), "").unwrap();
    s.set_grains(AREA, &grains).unwrap();
    let off = s.outputs[AREA].base.clone();
    let mut p = AreaParams::default();
    p.tone.grain_norm = GrainNorm { enabled: true, strength: 1.0, reference: None, feather_um: 4.0 };
    let r = s.recompute(AREA, &serde_json::to_string(&p).unwrap(), &Ctl::NONE, &|| 0.0).unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&r).unwrap()["timings"].to_string().contains("tone"));
    let on = s.outputs[AREA].base.clone();
    let changed = off.iter().zip(&on).filter(|(a, b)| a != b).count();
    assert!(changed > 1000, "{changed} pixels changed");
    // Pixels outside every grain's bounding box are untouched.
    let gs: Vec<Grain> = serde_json::from_str(&grains).unwrap();
    let k = s.project.as_ref().unwrap().areas[0].um_per_px();
    let w = s.outputs[AREA].linear.w;
    for (i, (a, b)) in off.iter().zip(&on).enumerate() {
        if a != b {
            let (x, y) = ((i % w) as f64 * k, (i / w) as f64 * k);
            assert!(gs.iter().any(|g| x >= g.bbox_um[0] - k && x <= g.bbox_um[2] + k && y >= g.bbox_um[1] - k && y <= g.bbox_um[3] + k), "pixel {i} outside grains changed");
        }
    }
}

#[test]
fn auto_then_back_to_before_gives_the_first_mosaic() {
    let mut s = stitched();
    let p0 = serde_json::to_string(&AreaParams::default()).unwrap();
    let h0 = s.outputs[AREA].mosaic_json.sha256.clone();
    let auto = s.auto_params(AREA, &p0).unwrap();
    assert_ne!(sha(&s.recompute(AREA, &auto, &Ctl::NONE, &|| 0.0).unwrap()), h0);
    assert_eq!(sha(&s.recompute(AREA, &p0, &Ctl::NONE, &|| 0.0).unwrap()), h0);
}
