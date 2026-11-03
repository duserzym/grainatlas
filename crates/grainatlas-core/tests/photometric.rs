mod common;

use std::collections::BTreeMap;

use common::{Rng, Scene};
use grainatlas_core::Ctl;
use grainatlas_core::image::Gray;
use grainatlas_core::mosaic::{self, StitchParams};
use grainatlas_core::photometric::{self, GainModel, PhotometricParams, TileOverride, background, hdr, tone};
use grainatlas_core::readers::{AreaMeta, TileMeta};

fn grid(gains: &[(f64, f64)]) -> (mosaic::Placement, Vec<Gray>) {
    let (tw, th) = (300usize, 240usize);
    let scene = Scene::new(900.0, 700.0, 500, 3);
    let mut tiles = Vec::new();
    let mut metas = Vec::new();
    for r in 0..2 {
        for c in 0..3 {
            let i = r * 3 + c;
            let (x, y) = (10.0 + c as f64 * 240.0, 10.0 + r as f64 * 190.0);
            tiles.push(scene.tile(x, y, tw, th, gains[i].0, gains[i].1));
            metas.push(TileMeta { index: i, file: String::new(), w: tw, h: th, pos_um: [c as f64 * 240.0, r as f64 * 190.0], stage_nm: None, pixel_size_nm: 1000.0, grid: None, meta: BTreeMap::new() });
        }
    }
    let area = AreaMeta { id: "s".into(), name: "s".into(), grid: None, pixel_size_nm: 1000.0, field_um: None, pitch_um: None, stage_sign: [1.0, 1.0], tiles: metas, meta: BTreeMap::new() };
    let pl = mosaic::stitch(&area, &tiles, &StitchParams::default(), &Ctl::NONE).unwrap();
    (pl, tiles)
}

/// Known per-tile gains and offsets are recovered within 1 percent (relative to tile 0).
#[test]
fn affine_gain_offset_recovered() {
    let truth = [(1.0, 0.0), (1.1, 0.02), (0.92, -0.01), (1.05, 0.03), (0.97, 0.0), (1.08, -0.02)];
    let (pl, tiles) = grid(&truth);
    let p = PhotometricParams { background: false, regularisation: 1e-4, ..Default::default() };
    let (_, res) = photometric::correct(&pl, &tiles, &p);
    // Corrected = g_i (t_i x + o_i) + c_i must be the same affine map of x for all tiles.
    let (g0, o0) = (res.gain[0] as f64 * truth[0].0, res.gain[0] as f64 * truth[0].1 + res.offset[0] as f64);
    for i in 1..6 {
        let gi = res.gain[i] as f64 * truth[i].0;
        let oi = res.gain[i] as f64 * truth[i].1 + res.offset[i] as f64;
        assert!((gi / g0 - 1.0).abs() < 0.01, "tile {i}: gain ratio {}", gi / g0);
        assert!((oi - o0).abs() < 0.01, "tile {i}: offset diff {}", oi - o0);
    }
    assert!(res.overlap_rms_after < res.overlap_rms_before * 0.3, "{} -> {}", res.overlap_rms_before, res.overlap_rms_after);
}

#[test]
fn gain_model_uses_photocraft() {
    let truth = [(1.0, 0.0), (1.2, 0.0), (1.0, 0.0), (1.0, 0.0), (0.9, 0.0), (1.0, 0.0)];
    let (pl, tiles) = grid(&truth);
    let p = PhotometricParams { model: GainModel::Gain, background: false, ..Default::default() };
    let (_, res) = photometric::correct(&pl, &tiles, &p);
    let r = (res.gain[1] as f64 * 1.2) / (res.gain[0] as f64);
    assert!((r - 1.0).abs() < 0.05, "{r}");
}

#[test]
fn manual_overrides_replace_auto_values() {
    let truth = [(1.0, 0.0); 6];
    let (pl, tiles) = grid(&truth);
    let p = PhotometricParams { background: false, overrides: vec![TileOverride { index: 2, gain: Some(1.5), offset: Some(0.1), no_background: false }], ..Default::default() };
    let (out, res) = photometric::correct(&pl, &tiles, &p);
    assert_eq!(res.gain[2], 1.5);
    assert_eq!(res.offset[2], 0.1);
    assert!((out[2].px[0] - (tiles[2].px[0] * 1.5 + 0.1)).abs() < 1e-6);
    // Overrides survive a round trip through project.json parameters.
    let j = serde_json::to_string(&p).unwrap();
    assert_eq!(serde_json::from_str::<PhotometricParams>(&j).unwrap(), p);
}

#[test]
fn background_leveling_removes_ramp() {
    let mut r = Rng(5);
    let (w, h) = (400usize, 300usize);
    let mut t = Gray::new(w, h);
    for y in 0..h {
        for x in 0..w {
            t.px[y * w + x] = 0.1 + 0.05 * x as f32 / w as f32 + 0.01 * (r.next() as f32 - 0.5);
        }
    }
    // A bright "grain" that must not bias the surface.
    for y in 100..180 {
        for x in 150..230 {
            t.px[y * w + x] = 0.8;
        }
    }
    let mut tiles = vec![t];
    let (median, ranges) = background::level(&mut tiles, &[false], &Default::default());
    assert!(ranges[0] > 0.03, "ramp seen: {}", ranges[0]);
    let left: f32 = (0..h).map(|y| tiles[0].px[y * w + 10]).sum::<f32>() / h as f32;
    let right: f32 = (0..h).map(|y| tiles[0].px[y * w + w - 10]).sum::<f32>() / h as f32;
    assert!((left - right).abs() < 0.006, "left {left} right {right}");
    assert!((left - median).abs() < 0.01);
    assert!(tiles[0].px[140 * w + 190] > 0.7, "grain kept");
}

#[test]
fn stretch_lut_and_clahe() {
    let mut g = Gray::new(256, 256);
    for (i, v) in g.px.iter_mut().enumerate() {
        *v = 0.2 + 0.3 * ((i % 256) as f32 / 255.0);
    }
    let p = tone::ToneParams::default();
    let (lo, hi) = tone::stretch(&mut g, None, &p);
    assert!((lo - 0.2).abs() < 0.01 && (hi - 0.5).abs() < 0.01, "{lo} {hi}");
    assert!(g.px.iter().all(|v| (0.0..=1.0).contains(v)));
    let lut = tone::lut(&tone::ToneParams { black: 0.25, white: 0.75, gamma: 2.0, ..Default::default() });
    assert_eq!(lut[0], 0);
    assert_eq!(lut[255], 255);
    assert!(lut[128] > 128, "gamma 2 brightens mid-tones: {}", lut[128]);
    let identity = tone::lut(&tone::ToneParams::default());
    assert!(identity.iter().enumerate().all(|(i, &v)| v as usize == i));
    let before = g.clone();
    tone::clahe(&mut g, 64, 2.0);
    assert_ne!(before, g);
    assert!(g.px.iter().all(|v| (0.0..=1.0).contains(v)));
}

/// HDR merge of synthetic exposures recovers a monotonic tone map spanning dim and bright parts.
#[test]
fn hdr_merge_synthetic_exposures() {
    let (w, h) = (128usize, 96usize);
    // Radiance spanning 3 decades: a dim core and a bright rim.
    let rad: Vec<f32> = (0..w * h).map(|i| {
        let (x, y) = ((i % w) as f32, (i / w) as f32);
        let d = ((x - 64.0).powi(2) + (y - 48.0).powi(2)).sqrt();
        if d < 20.0 { 0.002 + 0.0005 * (x % 7.0) } else if d < 30.0 { 0.5 + 0.05 * (y % 5.0) } else { 0.05 + 0.01 * ((x + y) % 9.0) }
    }).collect();
    let expose = |k: f32| Gray { w, h, px: rad.iter().map(|r| (r * k).powf(1.0 / 2.2).clamp(0.0, 1.0)).collect() };
    let shots = vec![expose(1.0), expose(8.0), expose(64.0)];
    let m = hdr::merge_exposures(&shots).expect("merged");
    assert_eq!(m.exposures.len(), 3);
    // Exposures are estimated in increasing order with ratios near 8.
    assert!(m.exposures[1] > m.exposures[0] * 3.0 && m.exposures[2] > m.exposures[1] * 3.0, "{:?}", m.exposures);
    assert!(m.offsets.iter().all(|&o| o == (0, 0)));
    // Core, background and rim keep their order and the core is no longer black.
    let at = |x: usize, y: usize| m.image.px[y * w + x];
    let (core, bg, rim) = (at(64, 48), at(5, 5), at(64, 22));
    assert!(core < bg && bg < rim, "{core} {bg} {rim}");
    assert!(core > 0.01, "dim core recovered: {core}");
    assert!(rim < 0.999, "rim not clipped: {rim}");
}

#[test]
fn duplicate_fields_group_for_hdr() {
    let t = |i: usize, x: f64| TileMeta { index: i, file: String::new(), w: 10, h: 10, pos_um: [x, 0.0], stage_nm: None, pixel_size_nm: 1000.0, grid: None, meta: BTreeMap::new() };
    let g = grainatlas_core::project::pipeline::hdr_groups(&[t(0, 0.0), t(1, 100.0), t(2, 0.3), t(3, 200.0)]);
    assert_eq!(g, vec![vec![0, 2], vec![1], vec![3]]);
}

/// A darker band along the top edge (beam already scanned it for the tile above) is found and removed.
#[test]
fn beam_exposure_step_removed() {
    let mut r = Rng(11);
    let (w, h) = (640usize, 480usize);
    let mut t = Gray::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let step = if y < 150 { -0.012 } else { 0.0 };
            t.px[y * w + x] = 0.08 + step + 0.006 * (r.next() as f32 - 0.5);
        }
    }
    let st = background::detect_edge_steps(&t, 0.5, 0);
    let top = st.top.expect("top step");
    assert!((top.at as i64 - 150).abs() <= 8, "at {}", top.at);
    assert!((top.height - 0.012).abs() < 0.002, "height {}", top.height);
    assert!(st.left.is_none() && st.right.is_none() && st.bottom.is_none(), "{st:?}");
    st.apply(&mut t);
    let band: f32 = (20..130).map(|y| t.px[y * w + 300]).sum::<f32>() / 110.0;
    let rest: f32 = (200..400).map(|y| t.px[y * w + 300]).sum::<f32>() / 200.0;
    assert!((band - rest).abs() < 0.002, "{band} {rest}");
}
