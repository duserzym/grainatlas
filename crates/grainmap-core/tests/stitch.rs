mod common;

use std::collections::BTreeMap;

use common::{Rng, Scene, fixture};
use grainmap_core::Ctl;
use grainmap_core::mosaic::{self, StitchParams};
use grainmap_core::project::pipeline::{self, Timings};
use grainmap_core::project::{AreaParams, Project};
use grainmap_core::readers::{AreaMeta, TileMeta};

#[test]
fn tm4000_reader_parses_fixture() {
    let pkg = fixture();
    let info = pkg.info();
    assert_eq!(info.reader, "tm4000");
    assert_eq!(info.name, "Sweden_XS_150X");
    assert_eq!(info.areas.len(), 1, "only area 001 has tiles in the fixture");
    let a = &info.areas[0];
    assert_eq!(a.id, "001_HJ-GC1");
    assert_eq!(a.name, "HJ-GC1");
    assert_eq!(a.grid, Some([3, 2]));
    assert_eq!(a.tiles.len(), 6);
    assert!((a.pixel_size_nm - 661.4583).abs() < 1e-6);
    assert_eq!(a.field_um, Some([846.666, 635.0]));
    assert_eq!(a.pitch_um, Some([676.0, 508.0]));
    // Stage x decreases to the right: tile 1 is 676 um right of tile 0, tile 3 508 um below.
    let p = |i: usize| a.tiles[i].pos_um;
    assert!((p(1)[0] - p(0)[0] - 676.0).abs() < 1e-6);
    assert!((p(3)[1] - p(0)[1] - 508.0).abs() < 1e-6);
    assert_eq!(a.tiles[0].stage_nm, Some([14954450.0, 19902350.0]));
    // Unknown keys are kept verbatim.
    assert_eq!(a.tiles[0].meta.get("SignalName").map(String::as_str), Some("SE"));
    assert_eq!(a.tiles[0].meta.get("UserName").map(String::as_str), Some("***"));
    assert!(a.meta.contains_key("ini.TsunagiInfo::CTsunagiInfo.wScanSpeed"));
    assert_eq!(a.meta.get("zag.Comment").map(String::as_str), Some("slab"));
    let t = pkg.tile(0, 0).unwrap();
    assert_eq!((t.image.w, t.image.h), (1280, 960));
    assert_eq!(t.sha256.len(), 64);
    // Navigation image with stage calibration.
    let nav = info.navigation.as_ref().expect("navigation");
    assert_eq!(nav.top_left_nm, [41000000.0, 38125000.0]);
}

#[test]
fn generic_reader_reads_positions_csv() {
    use grainmap_core::source::MemSource;
    let mut src = MemSource::new("plain");
    let g = grainmap_core::image::Gray::filled(8, 6, 0.5);
    let png = grainmap_core::image::encode_png_gray(8, 6, &g.to_u8());
    src.insert("a/t0.png", png.clone());
    src.insert("a/t1.png", png);
    src.insert("a/positions.csv", b"file,x_um,y_um,pixel_size_nm,note\nt0.png,100,50,500,first\nt1.png,103,50,500,\n".to_vec());
    let pkg = grainmap_core::readers::open(std::sync::Arc::new(src)).unwrap();
    let a = &pkg.info().areas[0];
    assert_eq!(pkg.info().reader, "generic");
    assert_eq!(a.tiles.len(), 2);
    assert_eq!(a.tiles[1].pos_um, [3.0, 0.0]);
    assert_eq!((a.tiles[0].w, a.tiles[0].h), (8, 6));
    assert_eq!(a.tiles[0].meta.get("note").map(String::as_str), Some("first"));
    assert_eq!(pkg.tile(0, 1).unwrap().image.w, 8);
}

/// Synthetic 3 x 3 grid with known sub-pixel drifts: recovered within 0.25 px.
#[test]
fn synthetic_offsets_recovered() {
    let (tw, th) = (360usize, 280usize);
    let (pitch_x, pitch_y) = (290.0, 225.0);
    let scene = Scene::new(1200.0, 900.0, 700, 7);
    let mut rng = Rng(99);
    let mut truth = Vec::new();
    let mut tiles = Vec::new();
    let mut metas = Vec::new();
    for r in 0..3 {
        for c in 0..3 {
            let i = r * 3 + c;
            let dx = if i == 0 { 0.0 } else { (rng.next() - 0.5) * 16.0 };
            let dy = if i == 0 { 0.0 } else { (rng.next() - 0.5) * 16.0 };
            let x = 20.0 + c as f64 * pitch_x + dx;
            let y = 20.0 + r as f64 * pitch_y + dy;
            truth.push([x, y]);
            tiles.push(scene.tile(x, y, tw, th, 1.0, 0.0));
            metas.push(TileMeta {
                index: i,
                file: format!("t{i}"),
                w: tw,
                h: th,
                pos_um: [c as f64 * pitch_x, r as f64 * pitch_y], // pixel size 1000 nm: um == px
                stage_nm: None,
                pixel_size_nm: 1000.0,
                grid: Some([c, r]),
                meta: BTreeMap::new(),
            });
        }
    }
    let area = AreaMeta { id: "syn".into(), name: "syn".into(), grid: Some([3, 3]), pixel_size_nm: 1000.0, field_um: None, pitch_um: None, stage_sign: [1.0, 1.0], tiles: metas, meta: BTreeMap::new() };
    let pl = mosaic::stitch(&area, &tiles, &StitchParams::default(), &Ctl::NONE).unwrap();
    assert_eq!(pl.pairs.len(), 12);
    assert!(pl.pairs.iter().all(|p| p.used), "all synthetic pairs measured: {:?}", pl.pairs.iter().map(|p| p.psr).collect::<Vec<_>>());
    // Compare relative to tile 0 (the mosaic frame is shifted to the bounding box).
    let mut worst: f64 = 0.0;
    for (t, tr) in pl.tiles.iter().zip(&truth) {
        let ex = (t.pos[0] - pl.tiles[0].pos[0]) - (tr[0] - truth[0][0]);
        let ey = (t.pos[1] - pl.tiles[0].pos[1]) - (tr[1] - truth[0][1]);
        worst = worst.max(ex.abs()).max(ey.abs());
    }
    assert!(worst < 0.25, "worst error {worst:.3} px");
}

/// A pair with no texture falls back to the prior.
#[test]
fn flat_pair_falls_back_to_prior() {
    let a = grainmap_core::image::Gray::filled(200, 150, 0.1);
    let b = a.clone();
    assert!(mosaic::phasecorr::measure_pair(&a, &b, [160.0, 0.0], &StitchParams::default()).is_none());
}

#[test]
fn fixture_stitches_with_small_residuals() {
    let pkg = fixture();
    let mut project = Project::new(pkg.info(), "fixture", "2026-11-03T00:00:00Z");
    let rec = &mut project.areas[0];
    let tiles = pipeline::read_tiles(pkg.as_ref(), 0, rec).unwrap();
    let mut t = Timings::default();
    let out = pipeline::run(rec, &tiles, &AreaParams::default(), &Ctl::NONE, &|| 0.0, &mut t).unwrap();
    let pl = &out.placement;
    assert_eq!(pl.pairs.len(), 7);
    assert!(pl.pairs.iter().filter(|p| p.used).count() >= 5);
    assert!(pl.rms_residual < 0.5, "rms {}", pl.rms_residual);
    assert!(pl.max_residual < 1.0, "max {}", pl.max_residual);
    // The mosaic is roughly 3 x 2 pitches plus one field.
    assert!((3300..3360).contains(&out.linear.w) && (1700..1850).contains(&out.linear.h), "{}x{}", out.linear.w, out.linear.h);
    // Pyramid down to one tile.
    let last = out.pyramid.levels.last().unwrap().0.clone();
    assert_eq!((last.cols, last.rows), (1, 1));
    assert_eq!(out.pyramid.levels[0].0.cols, out.linear.w.div_ceil(512));
    // Deterministic: a second run gives the same hash.
    let mut rec2 = project.areas[0].clone();
    let out2 = pipeline::run(&mut rec2, &tiles, &AreaParams::default(), &Ctl::NONE, &|| 0.0, &mut t).unwrap();
    assert_eq!(out.mosaic_json.sha256, out2.mosaic_json.sha256);
    // project.json round-trips.
    let j = project.to_json();
    assert_eq!(Project::from_json(&j).unwrap(), project);
}

/// Golden hash (plan section 9): the fixture mosaic with default parameters. A change in output
/// must be deliberate and update tests/golden/fixture_mosaic.sha256 (the wasm build is checked
/// against the same value by `make parity`).
#[test]
fn fixture_mosaic_matches_golden_hash() {
    let golden = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/fixture_mosaic.sha256")).unwrap();
    let pkg = fixture();
    let mut project = Project::new(pkg.info(), "hash", "1970-01-01T00:00:00Z");
    let rec = &mut project.areas[0];
    let tiles = pipeline::read_tiles(pkg.as_ref(), 0, rec).unwrap();
    let out = pipeline::run(rec, &tiles, &AreaParams::default(), &Ctl::NONE, &|| 0.0, &mut Timings::default()).unwrap();
    assert_eq!(format!("{} {}", rec.id, out.mosaic_json.sha256), golden.trim());
}
