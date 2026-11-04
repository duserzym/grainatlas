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
use grainatlas_core::segment::{self, Artifact, Grain, Polarity, SegmentParams, WorkImage};

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
        artifact: None,
        kept: false,
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
fn both_polarities_find_dark_grains_and_flag_bubbles_slivers_and_blocks() {
    let w = cl_scene();
    let near = |gs: &[Grain], x: f64, y: f64| gs.iter().any(|g| g.is_grain() && (g.centroid_um[0] - x * 2.0).hypot(g.centroid_um[1] - y * 2.0) < 20.0);
    let flagged = |gs: &[Grain], x: f64, y: f64, why: Artifact| gs.iter().any(|g| g.artifact == Some(why) && (g.centroid_um[0] - x * 2.0).hypot(g.centroid_um[1] - y * 2.0) < 30.0);
    let bright = segment::segment(&w, &SegmentParams { work_scale: 1, ..Default::default() }, &[]);
    assert!(near(&bright, 60.0, 80.0) && near(&bright, 160.0, 80.0));
    assert!(!near(&bright, 60.0, 250.0), "Bright does not see CL-dark grains");
    let both = segment::segment(&w, &SegmentParams { work_scale: 1, polarity: Polarity::Both, ..Default::default() }, &[]);
    assert!(near(&both, 60.0, 80.0) && near(&both, 160.0, 80.0));
    assert!(near(&both, 60.0, 250.0) && near(&both, 170.0, 260.0), "{:?}", both.iter().map(|g| (g.centroid_um, g.artifact)).collect::<Vec<_>>());
    for gs in [&bright, &both] {
        // Flagged with a reason, never deleted, never a grain.
        assert!(!near(gs, 300.0, 150.0), "bubble and halo are not grains");
        assert!(flagged(gs, 300.0, 150.0, Artifact::Bubble), "the bubble is flagged: {:?}", gs.iter().map(|g| (g.centroid_um, g.artifact)).collect::<Vec<_>>());
        assert!(!near(gs, 310.0, 332.0), "the seam sliver is not a grain");
        assert!(gs.iter().filter(|g| g.is_grain()).all(|g| g.eq_diameter_um < 200.0), "the tile block is not a grain");
        // Artifacts have no number; grains are 1..n.
        assert!(gs.iter().filter(|g| !g.is_grain()).all(|g| g.number == 0));
        let mut ns: Vec<u32> = gs.iter().filter(|g| g.is_grain()).map(|g| g.number).collect();
        ns.sort_unstable();
        assert_eq!(ns, (1..=ns.len() as u32).collect::<Vec<_>>());
    }
    assert_eq!(both.iter().filter(|g| g.is_grain() && g.centroid_um[0] < 800.0).count(), 4, "{:?}", both.iter().map(|g| (g.centroid_um, g.eq_diameter_um, g.artifact)).collect::<Vec<_>>());
    // With the classifier off, the halo and the block come back as grains.
    let raw = segment::segment(&w, &SegmentParams { work_scale: 1, flag_bubbles: false, flag_slivers: false, flag_blocks: false, flag_specks: false, ..Default::default() }, &[]);
    assert!(raw.iter().all(|g| g.is_grain()));
    assert!(raw.len() > bright.iter().filter(|g| g.is_grain()).count());
}

#[test]
fn user_decisions_survive_a_rerun() {
    let w = cl_scene();
    let p = SegmentParams { work_scale: 1, polarity: Polarity::Both, ..Default::default() };
    let mut first = segment::segment(&w, &p, &[]);
    // Keep the flagged bubble as a grain, mark the first grain as not a grain.
    let b = first.iter().position(|g| g.artifact == Some(Artifact::Bubble)).expect("a bubble");
    first[b].artifact = None;
    first[b].kept = true;
    first[b].number = 99;
    let m = first.iter().position(|g| g.is_grain() && !g.kept).expect("a grain");
    first[m].artifact = Some(Artifact::Manual);
    let (bid, mid) = (first[b].id, first[m].id);
    let again = segment::segment(&w, &p, &first);
    let kept = again.iter().find(|g| g.id == bid).expect("kept grain keeps its id");
    assert!(kept.is_grain() && kept.kept && kept.number == 99);
    let marked = again.iter().find(|g| g.id == mid).expect("marked detection keeps its id");
    assert_eq!(marked.artifact, Some(Artifact::Manual));
    assert_eq!(marked.number, 0);
}

#[test]
fn reference_size_ignores_fragments_and_blocks() {
    let mut gs: Vec<Grain> = (0..8).map(|i| grain(100.0 + 300.0 * i as f64, 100.0, 150.0)).collect();
    gs.extend((0..20).map(|i| grain(100.0 + 50.0 * i as f64, 400.0, 36.0)));
    gs.push(grain(5000.0, 5000.0, 1500.0));
    let r = segment::filter::reference_size(&gs);
    let side = 2.0 * (150.0f64 * 150.0 / std::f64::consts::PI).sqrt();
    assert!((r - side).abs() < 1.0, "{r}");
    assert_eq!(segment::filter::reference_size(&gs[..3]), 110.0);
    // The bubble and block limits scale with it: 75 um and 330 um at 110 um, 115 um and 500 um at 170 um.
    assert!((segment::filter::rel::bubble_max(110.0) - 75.0).abs() < 1.0 && (segment::filter::rel::bubble_max(170.0) - 115.6).abs() < 1.0);
    assert!((segment::filter::rel::block_split(110.0) - 330.0).abs() < 1.0 && (segment::filter::rel::block_split(170.0) - 510.0).abs() < 1.0);
}

/// A CL-dark grain whose bright rim the primary pass finds in pieces.
#[test]
fn a_dark_grain_with_a_bright_rim_is_one_footprint() {
    let (w, h) = (300usize, 200usize);
    let mut g = Gray::filled(w, h, 0.3);
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f64 - 150.0).powi(2) + (y as f64 - 100.0).powi(2)).sqrt();
            let a = (y as f64 - 100.0).atan2(x as f64 - 150.0);
            if d <= 40.0 {
                g.px[y * w + x] = 0.04; // dark core
            } else if d <= 46.0 && (a.sin() * 3.0).fract().abs() < 0.6 {
                g.px[y * w + x] = 0.85; // broken bright rim
            }
        }
    }
    let mut r = Rng(11);
    for v in g.px.iter_mut() {
        *v += ((r.next() - 0.5) * 0.02) as f32;
    }
    let wi = WorkImage { img: g, cov: vec![true; w * h], um_per_px: 2.0 };
    let both = segment::segment(&wi, &SegmentParams { work_scale: 1, polarity: Polarity::Both, ..Default::default() }, &[]);
    let real: Vec<&Grain> = both.iter().filter(|g| g.is_grain()).collect();
    assert_eq!(real.len(), 1, "{:?}", both.iter().map(|g| (g.centroid_um, g.eq_diameter_um, g.artifact)).collect::<Vec<_>>());
    assert!(real[0].eq_diameter_um > 160.0, "{}", real[0].eq_diameter_um);
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

/// Epoxy (0.3, noisy) at 2 um/px with discs drawn on it.
fn scene_with(w: usize, h: usize, draw: impl Fn(&mut Gray)) -> WorkImage {
    let mut g = Gray::filled(w, h, 0.3);
    draw(&mut g);
    let mut r = Rng(5);
    for v in g.px.iter_mut() {
        *v += ((r.next() - 0.5) * 0.02) as f32;
    }
    WorkImage { img: g, cov: vec![true; w * h], um_per_px: 2.0 }
}

fn disc(g: &mut Gray, cx: f64, cy: f64, r0: f64, r1: f64, v: f32) {
    let w = g.w;
    for y in 0..g.h {
        for x in 0..w {
            let d = (x as f64 - cx).hypot(y as f64 - cy);
            if d >= r0 && d <= r1 {
                g.px[y * w + x] = v;
            }
        }
    }
}

#[test]
fn a_fused_bubble_comes_off_and_a_dark_core_stays() {
    // Left: a bright grain with an epoxy bubble (dark, bright halo) fused to its top edge.
    // Right: a bright grain with a CL-dark core inside.
    let w = scene_with(500, 260, |g| {
        disc(g, 110.0, 150.0, 0.0, 45.0, 0.8);
        disc(g, 110.0, 88.0, 0.0, 26.0, 0.85); // halo
        disc(g, 110.0, 88.0, 0.0, 20.0, 0.03); // bubble
        disc(g, 360.0, 130.0, 0.0, 50.0, 0.8);
        disc(g, 362.0, 128.0, 0.0, 18.0, 0.04); // dark core
    });
    let gs = segment::segment(&w, &SegmentParams { work_scale: 1, polarity: Polarity::Both, ..Default::default() }, &[]);
    let real: Vec<&Grain> = gs.iter().filter(|g| g.is_grain()).collect();
    assert_eq!(real.len(), 2, "{:?}", gs.iter().map(|g| (g.centroid_um, g.eq_diameter_um, g.artifact)).collect::<Vec<_>>());
    let left = real.iter().find(|g| g.centroid_um[0] < 400.0).expect("left grain");
    assert!(!segment::shape::inside(&left.polygon_um, 220.0, 176.0), "the bubble is off the grain");
    assert!(gs.iter().any(|g| g.artifact == Some(Artifact::Bubble) && (g.centroid_um[0] - 220.0).hypot(g.centroid_um[1] - 176.0) < 20.0), "and flagged");
    let right = real.iter().find(|g| g.centroid_um[0] > 400.0).expect("right grain");
    assert!(segment::shape::inside(&right.polygon_um, 724.0, 256.0), "the dark core stays in its grain");
    assert!(right.eq_diameter_um > 180.0);
}

#[test]
fn grains_bridged_by_a_seam_line_are_split() {
    let w = scene_with(400, 200, |_| {});
    // Two squares joined by a 6 um bar, as one detection.
    let mut g = grain(0.0, 0.0, 1.0);
    g.polygon_um = vec![[100.0, 100.0], [260.0, 100.0], [260.0, 170.0], [400.0, 170.0], [400.0, 100.0], [560.0, 100.0], [560.0, 260.0], [400.0, 260.0], [400.0, 176.0], [260.0, 176.0], [260.0, 260.0], [100.0, 260.0]];
    g.bbox_um = [100.0, 100.0, 560.0, 260.0];
    g.centroid_um = [330.0, 180.0];
    g.area_um2 = 2.0 * 160.0 * 160.0;
    g.eq_diameter_um = 255.0;
    let id = g.id;
    let mut gs = vec![g];
    segment::filter::split_bridged(&mut gs, &w, &SegmentParams::default(), 110.0);
    assert_eq!(gs.len(), 2, "{:?}", gs.iter().map(|g| (g.centroid_um, g.eq_diameter_um)).collect::<Vec<_>>());
    assert!(gs.iter().any(|q| q.id == id), "a part keeps the id");
    assert!(gs.iter().all(|q| (q.eq_diameter_um - 180.0).abs() < 25.0));
}

#[test]
fn the_split_tool_flags_a_bubble_part() {
    let w = scene_with(300, 200, |g| {
        disc(g, 100.0, 100.0, 0.0, 40.0, 0.8);
        disc(g, 160.0, 100.0, 0.0, 22.0, 0.03); // bubble against the grain
    });
    // The grain outline covering both, cut between them.
    let mut g = grain(0.0, 0.0, 1.0);
    g.polygon_um = (0..64)
        .map(|i| {
            let a = i as f64 / 64.0 * std::f64::consts::TAU;
            [2.0 * (100.0 + 40.0 * a.cos()), 2.0 * (100.0 + 40.0 * a.sin())]
        })
        .collect();
    let bubble: Vec<[f64; 2]> = (0..64)
        .map(|i| {
            let a = i as f64 / 64.0 * std::f64::consts::TAU;
            [2.0 * (160.0 + 22.0 * a.cos()), 2.0 * (100.0 + 22.0 * a.sin())]
        })
        .collect();
    // Union as a hull-like outline: both circles' points (convex hull).
    let mut pts = g.polygon_um.clone();
    pts.extend(bubble);
    g.polygon_um = segment::filter::convex_hull(&pts);
    g.bbox_um = [120.0, 120.0, 364.0, 280.0];
    g.centroid_um = [230.0, 200.0];
    let parts = segment::split_grain(&w, &g, [276.0, 100.0, 276.0, 300.0], 0.8);
    assert_eq!(parts.len(), 2, "{:?}", parts.iter().map(|p| (p.centroid_um, p.artifact)).collect::<Vec<_>>());
    let b = parts.iter().find(|p| p.centroid_um[0] > 276.0).expect("bubble part");
    assert_eq!(b.artifact, Some(Artifact::Bubble));
    assert!(parts.iter().find(|p| p.centroid_um[0] < 276.0).expect("grain part").is_grain());
}
