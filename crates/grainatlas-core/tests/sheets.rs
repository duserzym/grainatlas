//! Boise SEM run features: cross-area ownership, two-polarity segmentation with
//! bubble / sliver / tile-block rejection, export enhancement and print sheets.

mod common;

use std::collections::BTreeMap;

use common::Rng;
use grainatlas_core::image::Gray;
use grainatlas_core::layout::AreaImage;
use grainatlas_core::layout::sheets::{self, SheetArea, SheetMeta, SheetParams};
use grainatlas_core::mosaic::{Placement, TilePlacement};
use grainatlas_core::ownership::{self, OwnershipParams};
use grainatlas_core::photometric::enhance::{self, EnhanceParams, StripRefusal};
use grainatlas_core::project::{AreaParams, AreaRecord, TileRef};
use grainatlas_core::segment::{self, Grain, Polarity, SegmentParams, WorkImage};

/// A one-tile area of `w` x `h` um at 1 um/px, its stage centre at `stage_x` nm (TM4000 signs).
fn area(id: &str, name: &str, stage_x: f64, w: usize, h: usize) -> AreaRecord {
    AreaRecord {
        uuid: grainatlas_core::project::new_id(),
        id: id.into(),
        name: name.into(),
        sample: name.into(),
        pixel_size_nm: 1000.0,
        stage_sign: [-1.0, -1.0],
        tiles: vec![TileRef { file: format!("{id}/t.jpg"), sha256: String::new(), pos_um: [0.0, 0.0], stage_nm: Some([stage_x, 0.0]), pixel_size_nm: 1000.0, w, h }],
        params: AreaParams::default(),
        placement: Some(Placement {
            tiles: vec![TilePlacement { index: 0, prior: [0.0, 0.0], pos: [0.0, 0.0], x: 0, y: 0, w, h, drift: [0.0, 0.0] }],
            pairs: Vec::new(),
            width: w,
            height: h,
            rms_residual: 0.0,
            max_residual: 0.0,
        }),
        photometric: None,
        mosaic: None,
        area_meta: BTreeMap::new(),
    }
}

/// A square grain centred at (x, y) um.
fn grain(x: f64, y: f64, side: f64) -> Grain {
    let h = side / 2.0;
    Grain {
        id: grainatlas_core::project::new_id(),
        number: 0,
        polygon_um: vec![[x - h, y - h], [x + h, y - h], [x + h, y + h], [x - h, y + h]],
        bbox_um: [x - h, y - h, x + h, y + h],
        centroid_um: [x, y],
        area_um2: side * side,
        eq_diameter_um: 2.0 * (side * side / std::f64::consts::PI).sqrt(),
        major_um: side,
        minor_um: side,
        orientation_deg: 0.0,
        mean_intensity: 0.5,
        manual: false,
        scale_bar_um: 20.0,
    }
}

/// Areas A (stage x 0) and B (stage x -600 um): B's mosaic starts 600 um right of A's on the
/// stage, so A's x = 600..1000 um is B's x = 0..400 um.
fn two_areas() -> (Vec<AreaRecord>, BTreeMap<String, Vec<Grain>>) {
    let a = area("001_S1", "S1", 0.0, 1000, 600);
    let b = area("002_S1_2", "S1_2", -600_000.0, 1000, 600);
    let mut g = BTreeMap::new();
    g.insert(
        a.id.clone(),
        vec![
            grain(200.0, 300.0, 60.0), // only in A
            grain(900.0, 300.0, 60.0), // 70 um from A's edge, 300 um inside B
            grain(985.0, 150.0, 40.0), // cut by A's edge, whole in B
            grain(700.0, 450.0, 50.0), // in the overlap, but B did not find it
        ],
    );
    g.insert(
        b.id.clone(),
        vec![
            grain(302.0, 303.0, 62.0), // = A's (900, 300), stage error of a few um
            grain(385.0, 151.0, 40.0), // = A's (985, 150)
            grain(700.0, 300.0, 60.0), // only in B
        ],
    );
    (vec![a, b], g)
}

#[test]
fn ownership_keeps_each_grain_once_where_it_is_farthest_inside() {
    let (areas, grains) = two_areas();
    let o = ownership::assign(&areas, &grains, &OwnershipParams::default());
    assert_eq!(o.grains.len(), 7);
    let a = &grains["001_S1"];
    let b = &grains["002_S1_2"];
    // A's grain near its edge belongs to B, and points at B's detection.
    let r = o.of("001_S1", &a[1].id).unwrap();
    assert_eq!(r.owner_area, "002_S1_2");
    assert_eq!(r.duplicate_of, Some(b[0].id));
    assert!(!r.cut && (r.interior_um - 70.0).abs() < 5.0, "{}", r.interior_um);
    // The grain cut by A's edge always loses.
    let r = o.of("001_S1", &a[2].id).unwrap();
    assert!(r.cut);
    assert_eq!(r.duplicate_of, Some(b[1].id));
    // Not found in B: A keeps it.
    assert!(o.is_owned("001_S1", &a[3].id));
    assert!(o.is_owned("001_S1", &a[0].id));
    assert!(b.iter().all(|g| o.is_owned("002_S1_2", &g.id)));
    let owned = o.owned(&grains);
    assert_eq!(owned["001_S1"].len() + owned["002_S1_2"].len(), 5);
    // Stage positions are recorded (A's (200, 300) is 300 um left of its centre: stage x +300 um).
    let s = o.of("001_S1", &a[0].id).unwrap().stage_nm.unwrap();
    assert!((s[0] - 300_000.0).abs() < 1.0 && (s[1] - 0.0).abs() < 1.0, "{s:?}");
    // S1 and S1_2 are one sample.
    assert_eq!(o.samples.len(), 1);
    assert_eq!(o.samples[0].sample, "S1");
    assert_eq!(o.sample_of("002_S1_2"), Some("S1"));
}

#[test]
fn suffix_joins_only_existing_samples() {
    let areas = vec![area("001_a", "KH2-30.7", 0.0, 10, 10), area("002_b", "KH2-30.7_2", 0.0, 10, 10), area("003_c", "AH1_2", 0.0, 10, 10), area("004_d", "Skin_felsic", 0.0, 10, 10)];
    let g = ownership::sample_groups(&areas, true);
    let names: Vec<&str> = g.iter().map(|s| s.sample.as_str()).collect();
    assert_eq!(names, ["KH2-30.7", "AH1_2", "Skin_felsic"]);
    assert_eq!(g[0].areas, ["001_a", "002_b"]);
    assert_eq!(ownership::strip_suffix("KH2-30.7_2"), "KH2-30.7");
    assert_eq!(ownership::strip_suffix("Skin_felsic"), "Skin_felsic");
}

/// CL-like scene at 2 um/px: epoxy 0.3 with noise, bright grains, CL-dark grains, an epoxy bubble
/// (dark disc with a bright halo), a tile-seam sliver and a slightly brighter tile block.
fn cl_scene() -> WorkImage {
    let (w, h) = (700usize, 400usize);
    let mut g = Gray::filled(w, h, 0.3);
    // Tile block: the right part exposed a little brighter.
    for y in 0..h {
        for x in 450..w {
            g.px[y * w + x] = 0.37;
        }
    }
    let mut disc = |cx: f64, cy: f64, r: f64, v: f32| {
        for y in 0..h {
            for x in 0..w {
                if ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt() <= r {
                    g.px[y * w + x] = v;
                }
            }
        }
    };
    disc(60.0, 80.0, 25.0, 0.8); // bright
    disc(160.0, 80.0, 22.0, 0.75); // bright
    disc(60.0, 250.0, 25.0, 0.03); // CL-dark
    disc(170.0, 260.0, 20.0, 0.05); // CL-dark
    disc(300.0, 150.0, 14.0, 0.85); // bubble halo ...
    disc(300.0, 150.0, 10.0, 0.04); // ... around a 40 um dark bubble
    for y in 330..334 {
        for x in 200..420 {
            g.px[y * w + x] = 0.8; // seam sliver, 8 um wide
        }
    }
    let mut r = Rng(7);
    for v in g.px.iter_mut() {
        *v += ((r.next() - 0.5) * 0.02) as f32;
    }
    let n = w * h;
    WorkImage { img: g, cov: vec![true; n], um_per_px: 2.0 }
}

#[test]
fn both_polarities_find_dark_grains_and_reject_bubbles_slivers_and_blocks() {
    let w = cl_scene();
    let near = |gs: &[Grain], x: f64, y: f64| gs.iter().any(|g| (g.centroid_um[0] - x * 2.0).hypot(g.centroid_um[1] - y * 2.0) < 20.0);
    let bright = segment::segment(&w, &SegmentParams { work_scale: 1, ..Default::default() }, &[]);
    assert!(near(&bright, 60.0, 80.0) && near(&bright, 160.0, 80.0));
    assert!(!near(&bright, 60.0, 250.0), "Bright does not see CL-dark grains");
    let both = segment::segment(&w, &SegmentParams { work_scale: 1, polarity: Polarity::Both, ..Default::default() }, &[]);
    assert!(near(&both, 60.0, 80.0) && near(&both, 160.0, 80.0));
    assert!(near(&both, 60.0, 250.0) && near(&both, 170.0, 260.0), "{:?}", both.iter().map(|g| g.centroid_um).collect::<Vec<_>>());
    for gs in [&bright, &both] {
        assert!(!near(gs, 300.0, 150.0), "bubble and halo rejected");
        assert!(!near(gs, 310.0, 332.0), "seam sliver rejected");
        assert!(gs.iter().all(|g| g.eq_diameter_um < 200.0), "tile block rejected");
    }
    assert_eq!(both.iter().filter(|g| g.centroid_um[0] < 800.0).count(), 4, "{:?}", both.iter().map(|g| (g.centroid_um, g.eq_diameter_um)).collect::<Vec<_>>());
    // With the rejections off the block and the halo come back as grains.
    let raw = segment::segment(&w, &SegmentParams { work_scale: 1, reject_bubbles: false, reject_slivers: false, reject_blocks: false, ..Default::default() }, &[]);
    assert!(raw.len() > bright.len());
}

#[test]
fn supplement_replaces_a_bright_fragment_of_a_dark_grain() {
    let big = grain(100.0, 100.0, 80.0);
    let fragment = grain(110.0, 95.0, 20.0);
    let mut gs = vec![fragment.clone(), grain(400.0, 100.0, 60.0)];
    segment::filter::merge_supplement(&mut gs, vec![big.clone(), grain(402.0, 101.0, 58.0), grain(700.0, 100.0, 50.0)]);
    assert_eq!(gs.len(), 3);
    assert_eq!(gs[0].id, fragment.id, "keeps the primary id");
    assert_eq!(gs[0].area_um2, big.area_um2, "takes the whole grain");
    assert_eq!(gs[1].area_um2, 3600.0, "a primary grain of about the same size stays");
}

#[test]
fn nl_means_reduces_noise_and_keeps_edges() {
    let (w, h) = (96usize, 64usize);
    let mut r = Rng(3);
    let clean: Vec<f32> = (0..w * h).map(|i| if i % w < w / 2 { 0.2 } else { 0.7 }).collect();
    let noisy = Gray { w, h, px: clean.iter().map(|v| v + ((r.next() - 0.5) * 0.2) as f32).collect() };
    let sigma = enhance::estimate_noise(&noisy);
    assert!((sigma - 0.0577).abs() < 0.015, "{sigma}");
    let d = enhance::nl_means(&noisy, 0.7 * sigma, sigma, 5, 6);
    let err = |g: &Gray| (g.px.iter().zip(&clean).map(|(a, b)| ((a - b) as f64).powi(2)).sum::<f64>() / (w * h) as f64).sqrt();
    assert!(err(&d) < 0.5 * err(&noisy), "{} vs {}", err(&d), err(&noisy));
    // The step is still a step.
    let row = 32 * w;
    assert!(d.px[row + w / 2 - 3] < 0.35 && d.px[row + w / 2 + 2] > 0.55);
}

#[test]
fn strip_fills_hulls_and_refuses_when_unsafe() {
    let (w, h) = (200usize, 100usize);
    let mut img = Gray::filled(w, h, 0.3);
    // A grain whose outline caught only its bright upper half: an L-shaped polygon.
    for y in 20..80 {
        for x in 20..80 {
            img.px[y * w + x] = if y < 50 { 0.9 } else { 0.6 };
        }
    }
    let mut g = grain(50.0, 35.0, 30.0);
    g.polygon_um = vec![[20.0, 20.0], [80.0, 20.0], [80.0, 50.0], [50.0, 50.0], [50.0, 80.0], [20.0, 80.0]];
    g.bbox_um = [20.0, 20.0, 80.0, 80.0];
    let mask = enhance::grain_mask(w, h, [0.0, 0.0], 1.0, &[g.clone()], 3.0).unwrap();
    assert!(mask[60 * w + 60], "the hull covers the missed corner");
    assert!(!mask[50 * w + 150]);
    let mut a = img.clone();
    enhance::strip_background(&mut a, &mask, 0.1, 1.0).unwrap();
    assert!((a.px[50 * w + 150] - 0.1).abs() < 1e-3 && (a.px[60 * w + 60] - 0.6).abs() < 1e-3);
    // An open outline: no strip.
    let mut open = g.clone();
    open.polygon_um.truncate(2);
    assert!(enhance::grain_mask(w, h, [0.0, 0.0], 1.0, &[open], 3.0).is_none());
    // A grain nobody outlined: no strip.
    let mut b = img.clone();
    for y in 30..70 {
        for x in 120..160 {
            b.px[y * w + x] = 0.8;
        }
    }
    assert_eq!(enhance::strip_background(&mut b, &mask, 0.1, 1.0), Err(StripRefusal::UnmaskedObject));
}

#[test]
fn sheets_print_owned_grains_with_a_registration() {
    let (areas, grains) = two_areas();
    let own = ownership::assign(&areas, &grains, &OwnershipParams::default());
    let px: Vec<Vec<u8>> = areas.iter().map(|_| (0..1000 * 600).map(|i| ((i * 7) % 200) as u8).collect()).collect();
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = i as u8;
    }
    let sa: Vec<SheetArea> = areas
        .iter()
        .zip(&px)
        .map(|(rec, px)| SheetArea { rec, img: AreaImage { id: &rec.id, name: &rec.name, sample: &rec.sample, w: 1000, h: 600, px, lut, um_per_px: 1.0 } })
        .collect();
    let meta = SheetMeta { package: "Test_S_150X".into(), instrument: "Hitachi TM4000".into(), detector: "CL (UVD)".into(), kv: Some(20.0), ..Default::default() };
    let p = SheetParams { min_grain_um: 0.0, enhance: EnhanceParams::default(), ..Default::default() };
    let r = sheets::sheets(&p, &sa, &grains, &own, &meta);
    assert_eq!(r.stats.grains, 5);
    assert_eq!(r.stats.duplicates, 2);
    assert!(r.stats.sheets >= 1 && !r.pages.is_empty());
    let page = &r.pages[0];
    assert!(page.contains(r#"width="431.8mm""#) && page.contains(r#"height="279.4mm""#), "tabloid landscape");
    assert!(page.contains("Sheet 1 /") && page.contains("CL (UVD)") && page.contains("20 kV"));
    assert!(page.contains(">S1<"), "sample label");
    assert!(!page.contains("#1<") && !page.contains("#2<"), "no grain numbers");
    let reg = &r.registration;
    assert_eq!(reg.frame, "print_sheet");
    assert_eq!(reg.um_per_mm, 8.0);
    let printed: usize = reg.bands.iter().map(|b| b.grains.len()).sum();
    assert_eq!(printed, 5);
    for b in &reg.bands {
        // Each band is a translation at 8 um per mm.
        let (sw, sh) = (b.sheet_rect_mm[2] - b.sheet_rect_mm[0], b.sheet_rect_mm[3] - b.sheet_rect_mm[1]);
        let (mw, mh) = (b.mosaic_rect_um[2] - b.mosaic_rect_um[0], b.mosaic_rect_um[3] - b.mosaic_rect_um[1]);
        assert!((sw * 8.0 - mw).abs() < 0.01 && (sh * 8.0 - mh).abs() < 0.01, "{b:?}");
        assert!(b.sheet_rect_mm[0] >= 0.0 && b.sheet_rect_mm[2] <= 431.8 && b.sheet_rect_mm[3] <= 279.4);
        assert_eq!(b.sample, "S1");
    }
    // A's grain at x = 200 um and B's at x = 700 um are more than 420 um apart: a marked break.
    let pdf = grainatlas_core::layout::pdf::pdf_from_svgs(&r.pages).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn rows_chain_grains_along_x() {
    let gs = [grain(10.0, 100.0, 20.0), grain(60.0, 130.0, 20.0), grain(30.0, 400.0, 20.0), grain(110.0, 160.0, 20.0)];
    let refs: Vec<&Grain> = gs.iter().collect();
    let rows = sheets::rows_of(&refs, 120.0);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].len(), 3);
    assert!(rows[0].windows(2).all(|w| w[0].centroid_um[0] < w[1].centroid_um[0]));
}
