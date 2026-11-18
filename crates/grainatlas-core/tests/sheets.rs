//! Synthetic SEM tests: cross-area ownership, two-polarity segmentation with
//! bubble / sliver / tile-block rejection, export enhancement and print sheets.

mod common;

use std::collections::BTreeMap;

use common::Rng;
use grainatlas_core::image::Gray;
use grainatlas_core::layout::AreaImage;
use grainatlas_core::layout::sheets::{self, SheetArea, SheetMeta, SheetParams, Packing};
use grainatlas_core::mosaic::{Placement, TilePlacement};
use grainatlas_core::ownership::{self, OwnershipParams};
use grainatlas_core::photometric::enhance::{self, EnhanceParams, StripRefusal};
use grainatlas_core::project::{AreaParams, AreaRecord, TileRef};
use grainatlas_core::segment::{self, Artifact, Grain, Polarity, SegmentParams, WorkImage};

/// A one-tile area of `w` x `h` um at 1 um/px, its stage centre at `stage_x` nm (TM4000 signs).
fn area(id: &str, name: &str, stage_x: f64, w: usize, h: usize) -> AreaRecord {
    AreaRecord {
        failed_tiles: Vec::new(),
        uuid: grainatlas_core::project::new_id(),
        id: id.into(),
        name: name.into(),
        sample: name.into(),
        pixel_size_nm: 1000.0,
        stage_sign: [-1.0, -1.0],
        tiles: vec![TileRef {
            file: format!("{id}/t.jpg"),
            sha256: String::new(),
            pos_um: [0.0, 0.0],
            stage_nm: Some([stage_x, 0.0]),
            pixel_size_nm: 1000.0,
            w,
            h,
        }],
        params: AreaParams::default(),
        placement: Some(Placement {
            tiles: vec![TilePlacement {
                failed: false,
                index: 0,
                prior: [0.0, 0.0],
                pos: [0.0, 0.0],
                x: 0,
                y: 0,
                w,
                h,
                drift: [0.0, 0.0],
            }],
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
        label: None,
        sample: None,
        polygon_um: vec![
            [x - h, y - h],
            [x + h, y - h],
            [x + h, y + h],
            [x - h, y + h],
        ],
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
        drawn: false,
        added: false,
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
    assert!(
        !r.cut && (r.interior_um - 70.0).abs() < 5.0,
        "{}",
        r.interior_um
    );
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
    assert!(
        (s[0] - 300_000.0).abs() < 1.0 && (s[1] - 0.0).abs() < 1.0,
        "{s:?}"
    );
    // S1 and S1_2 are one sample.
    assert_eq!(o.samples.len(), 1);
    assert_eq!(o.samples[0].sample, "S1");
    assert_eq!(o.sample_of("002_S1_2"), Some("S1"));
}

#[test]
fn a_piece_of_a_grain_found_whole_next_door_is_not_printed_twice() {
    // At 80X, area A split a grain into a CL-dark piece and a bright piece; area B found it
    // whole. The dark piece's centre is far from the whole grain's centre (more than half its
    // size plus match_um), but it lies inside B's outline: the same grain, held once.
    let (areas, mut grains) = two_areas();
    // B's whole grain at B (170, 100), 300 x 100 um: A's (770, 100).
    let mut whole = grain(170.0, 100.0, 100.0);
    whole.polygon_um = vec![[20.0, 50.0], [320.0, 50.0], [320.0, 150.0], [20.0, 150.0]];
    whole.bbox_um = [20.0, 50.0, 320.0, 150.0];
    whole.area_um2 = 30_000.0;
    whole.eq_diameter_um = 2.0 * (30_000.0 / std::f64::consts::PI).sqrt();
    grains.get_mut("002_S1_2").unwrap().push(whole.clone());
    // A's piece covers the whole grain's left end: A (640, 100), 40 um, 130 um from its centre.
    let piece = grain(640.0, 100.0, 40.0);
    grains.get_mut("001_S1").unwrap().push(piece.clone());
    let p = OwnershipParams::default();
    assert!(
        130.0 > 0.5 * whole.eq_diameter_um + p.match_um,
        "the centres alone do not link them"
    );
    let o = ownership::assign(&areas, &grains, &p);
    let (rp, rw) = (
        o.of("001_S1", &piece.id).unwrap(),
        o.of("002_S1_2", &whole.id).unwrap(),
    );
    assert_eq!(rp.owner_area, rw.owner_area, "one grain, one owner");
    assert_eq!(
        o.is_owned("001_S1", &piece.id) as u8 + o.is_owned("002_S1_2", &whole.id) as u8,
        1,
        "printed once"
    );
    if o.is_owned("002_S1_2", &whole.id) {
        assert_eq!(rp.duplicate_of, Some(whole.id));
    } else {
        assert_eq!(rw.duplicate_of, Some(piece.id));
    }
    // The grain in the overlap that B did not find stays with A.
    assert!(o.is_owned("001_S1", &grains["001_S1"][3].id));
}

#[test]
fn suffix_joins_only_existing_samples() {
    let areas = vec![
        area("001_a", "A1-30.7", 0.0, 10, 10),
        area("002_b", "A1-30.7_2", 0.0, 10, 10),
        area("003_c", "B1_2", 0.0, 10, 10),
        area("004_d", "C1_felsic", 0.0, 10, 10),
    ];
    let g = ownership::sample_groups(&areas, true);
    let names: Vec<&str> = g.iter().map(|s| s.sample.as_str()).collect();
    assert_eq!(names, ["A1-30.7", "B1_2", "C1_felsic"]);
    assert_eq!(g[0].areas, ["001_a", "002_b"]);
    assert_eq!(ownership::strip_suffix("A1-30.7_2"), "A1-30.7");
    assert_eq!(ownership::strip_suffix("C1_felsic"), "C1_felsic");
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
    WorkImage {
        tiles_um: Vec::new(),
        signal: segment::Signal::Cl,
        img: g,
        cov: vec![true; n],
        um_per_px: 2.0,
    }
}

#[test]
fn both_polarities_find_dark_grains_and_flag_bubbles_slivers_and_blocks() {
    let w = cl_scene();
    let near = |gs: &[Grain], x: f64, y: f64| {
        gs.iter().any(|g| {
            g.is_grain() && (g.centroid_um[0] - x * 2.0).hypot(g.centroid_um[1] - y * 2.0) < 20.0
        })
    };
    let flagged = |gs: &[Grain], x: f64, y: f64, why: Artifact| {
        gs.iter().any(|g| {
            g.artifact == Some(why)
                && (g.centroid_um[0] - x * 2.0).hypot(g.centroid_um[1] - y * 2.0) < 30.0
        })
    };
    let bright = segment::segment(
        &w,
        &SegmentParams {
            work_scale: 1,
            ..Default::default()
        },
        &[],
    );
    assert!(near(&bright, 60.0, 80.0) && near(&bright, 160.0, 80.0));
    assert!(
        !near(&bright, 60.0, 250.0),
        "Bright does not see CL-dark grains"
    );
    let both = segment::segment(
        &w,
        &SegmentParams {
            work_scale: 1,
            polarity: Polarity::Both,
            ..Default::default()
        },
        &[],
    );
    assert!(near(&both, 60.0, 80.0) && near(&both, 160.0, 80.0));
    assert!(
        near(&both, 60.0, 250.0) && near(&both, 170.0, 260.0),
        "{:?}",
        both.iter()
            .map(|g| (g.centroid_um, g.artifact))
            .collect::<Vec<_>>()
    );
    for gs in [&bright, &both] {
        // Flagged with a reason, never deleted, never a grain.
        assert!(!near(gs, 300.0, 150.0), "bubble and halo are not grains");
        assert!(
            flagged(gs, 300.0, 150.0, Artifact::Bubble),
            "the bubble is flagged: {:?}",
            gs.iter()
                .map(|g| (g.centroid_um, g.artifact))
                .collect::<Vec<_>>()
        );
        assert!(!near(gs, 310.0, 332.0), "the seam sliver is not a grain");
        assert!(
            gs.iter()
                .filter(|g| g.is_grain())
                .all(|g| g.eq_diameter_um < 200.0),
            "the tile block is not a grain"
        );
        // Artifacts have no number; grains are 1..n.
        assert!(gs.iter().filter(|g| !g.is_grain()).all(|g| g.number == 0));
        let mut ns: Vec<u32> = gs
            .iter()
            .filter(|g| g.is_grain())
            .map(|g| g.number)
            .collect();
        ns.sort_unstable();
        assert_eq!(ns, (1..=ns.len() as u32).collect::<Vec<_>>());
    }
    assert_eq!(
        both.iter()
            .filter(|g| g.is_grain() && g.centroid_um[0] < 800.0)
            .count(),
        4,
        "{:?}",
        both.iter()
            .map(|g| (g.centroid_um, g.eq_diameter_um, g.artifact))
            .collect::<Vec<_>>()
    );
    // With the classifier off, the halo and the block come back as grains.
    let raw = segment::segment(
        &w,
        &SegmentParams {
            work_scale: 1,
            flag_bubbles: false,
            flag_slivers: false,
            flag_blocks: false,
            flag_specks: false,
            ..Default::default()
        },
        &[],
    );
    assert!(raw.iter().all(|g| g.is_grain()));
    assert!(raw.len() > bright.iter().filter(|g| g.is_grain()).count());
}

#[test]
fn user_decisions_survive_a_rerun() {
    let w = cl_scene();
    let p = SegmentParams {
        work_scale: 1,
        polarity: Polarity::Both,
        ..Default::default()
    };
    let mut first = segment::segment(&w, &p, &[]);
    // Keep the flagged bubble as a grain, mark the first grain as not a grain.
    let b = first
        .iter()
        .position(|g| g.artifact == Some(Artifact::Bubble))
        .expect("a bubble");
    first[b].artifact = None;
    first[b].kept = true;
    first[b].number = 99;
    let m = first
        .iter()
        .position(|g| g.is_grain() && !g.kept)
        .expect("a grain");
    first[m].artifact = Some(Artifact::Manual);
    let (bid, mid) = (first[b].id, first[m].id);
    let again = segment::segment(&w, &p, &first);
    let kept = again
        .iter()
        .find(|g| g.id == bid)
        .expect("kept grain keeps its id");
    assert!(kept.is_grain() && kept.kept && kept.number == 99);
    let marked = again
        .iter()
        .find(|g| g.id == mid)
        .expect("marked detection keeps its id");
    assert_eq!(marked.artifact, Some(Artifact::Manual));
    assert_eq!(marked.number, 0);
}

#[test]
fn reference_size_ignores_fragments_and_blocks() {
    let mut gs: Vec<Grain> = (0..8)
        .map(|i| grain(100.0 + 300.0 * i as f64, 100.0, 150.0))
        .collect();
    gs.extend((0..20).map(|i| grain(100.0 + 50.0 * i as f64, 400.0, 36.0)));
    gs.push(grain(5000.0, 5000.0, 1500.0));
    let r = segment::filter::reference_size(&gs);
    let side = 2.0 * (150.0f64 * 150.0 / std::f64::consts::PI).sqrt();
    assert!((r - side).abs() < 1.0, "{r}");
    assert_eq!(segment::filter::reference_size(&gs[..3]), 110.0);
    // The bubble and block limits scale with it: 75 um and 330 um at 110 um, 115 um and 500 um at 170 um.
    assert!(
        (segment::filter::rel::bubble_max(110.0) - 75.0).abs() < 1.0
            && (segment::filter::rel::bubble_max(170.0) - 115.6).abs() < 1.0
    );
    assert!(
        (segment::filter::rel::block_split(110.0) - 330.0).abs() < 1.0
            && (segment::filter::rel::block_split(170.0) - 510.0).abs() < 1.0
    );
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
    let wi = WorkImage {
        tiles_um: Vec::new(),
        signal: segment::Signal::Cl,
        img: g,
        cov: vec![true; w * h],
        um_per_px: 2.0,
    };
    let both = segment::segment(
        &wi,
        &SegmentParams {
            work_scale: 1,
            polarity: Polarity::Both,
            ..Default::default()
        },
        &[],
    );
    let real: Vec<&Grain> = both.iter().filter(|g| g.is_grain()).collect();
    assert_eq!(
        real.len(),
        1,
        "{:?}",
        both.iter()
            .map(|g| (g.centroid_um, g.eq_diameter_um, g.artifact))
            .collect::<Vec<_>>()
    );
    assert!(real[0].eq_diameter_um > 160.0, "{}", real[0].eq_diameter_um);
}

#[test]
fn nl_means_reduces_noise_and_keeps_edges() {
    let (w, h) = (96usize, 64usize);
    let mut r = Rng(3);
    let clean: Vec<f32> = (0..w * h)
        .map(|i| if i % w < w / 2 { 0.2 } else { 0.7 })
        .collect();
    let noisy = Gray {
        w,
        h,
        px: clean
            .iter()
            .map(|v| v + ((r.next() - 0.5) * 0.2) as f32)
            .collect(),
    };
    let sigma = enhance::estimate_noise(&noisy);
    assert!((sigma - 0.0577).abs() < 0.015, "{sigma}");
    let d = enhance::nl_means(&noisy, 0.7 * sigma, sigma, 5, 6);
    let err = |g: &Gray| {
        (g.px
            .iter()
            .zip(&clean)
            .map(|(a, b)| ((a - b) as f64).powi(2))
            .sum::<f64>()
            / (w * h) as f64)
            .sqrt()
    };
    assert!(
        err(&d) < 0.5 * err(&noisy),
        "{} vs {}",
        err(&d),
        err(&noisy)
    );
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
    g.polygon_um = vec![
        [20.0, 20.0],
        [80.0, 20.0],
        [80.0, 50.0],
        [50.0, 50.0],
        [50.0, 80.0],
        [20.0, 80.0],
    ];
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
    assert_eq!(
        enhance::strip_background(&mut b, &mask, 0.1, 1.0),
        Err(StripRefusal::UnmaskedObject)
    );
}

#[test]
fn sheets_print_owned_grains_with_a_registration() {
    both_packings(|| {
    let (areas, grains) = two_areas();
    let own = ownership::assign(&areas, &grains, &OwnershipParams::default());
    let px: Vec<Vec<u8>> = areas
        .iter()
        .map(|_| (0..1000 * 600).map(|i| ((i * 7) % 200) as u8).collect())
        .collect();
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = i as u8;
    }
    let sa: Vec<SheetArea> = areas
        .iter()
        .zip(&px)
        .map(|(rec, px)| SheetArea {
            rec,
            img: AreaImage {
                id: &rec.id,
                name: &rec.name,
                sample: &rec.sample,
                w: 1000,
                h: 600,
                px,
                lut,
                um_per_px: 1.0,
            },
        })
        .collect();
    let meta = SheetMeta {
        package: "mount_A".into(),
        instrument: "Hitachi TM4000".into(),
        detector: "CL (UVD)".into(),
        kv: Some(20.0),
        ..Default::default()
    };
    let p = SheetParams {
        packing: TEST_PACKING.get(),
        um_per_mm: 8.0,
        min_grain_um: 0.0,
        enhance: EnhanceParams::default(),
        ..Default::default()
    };
    let r = sheets::sheets(&p, &sa, &grains, &own, &meta);
    assert_eq!(r.stats.grains, 5);
    assert_eq!(r.stats.duplicates, 2);
    assert!(r.stats.sheets >= 1 && !r.pages.is_empty());
    let page = &r.pages[0];
    assert!(
        page.contains(r#"width="431.8mm""#) && page.contains(r#"height="279.4mm""#),
        "tabloid landscape"
    );
    assert!(page.contains("Sheet 1 /") && page.contains("CL (UVD)") && page.contains("20 kV"));
    assert!(page.contains(">S1<"), "sample label");
    assert!(
        !page.contains("#1<") && !page.contains("#2<"),
        "no grain numbers"
    );
    let reg = &r.registration;
    assert_eq!(reg.frame, "print_sheet");
    assert_eq!(reg.um_per_mm, 8.0);
    assert_eq!(reg.scale, "fixed");
    assert!(
        reg.auto_scale.is_none() && reg.spot_sizes_left_off_um.is_empty(),
        "every key circle drawable at 8 um/mm"
    );
    // The default scale is auto: these 60 um grains print at a median of 15 mm or more.
    let auto = sheets::sheets(
        &SheetParams {
            um_per_mm: 0.0,
            ..p.clone()
        },
        &sa,
        &grains,
        &own,
        &meta,
    );
    let areg = &auto.registration;
    assert_eq!(areg.scale, "auto");
    let a = areg.auto_scale.as_ref().unwrap();
    assert_eq!(areg.um_per_mm, a.um_per_mm);
    assert!(
        a.median_um / areg.um_per_mm >= sheets::AUTO_TARGET_CL_MM,
        "{a:?}"
    );
    assert!(
        auto.pages[0].contains("auto: median grain"),
        "the header says how the scale was chosen"
    );
    assert_eq!(
        a.target_mm,
        sheets::AUTO_TARGET_CL_MM,
        "CL (or unknown signal) aims for the CL size"
    );
    // Every printed area BSE: the smaller BSE target.
    let mut bse_areas = areas.clone();
    for r in &mut bse_areas {
        r.area_meta
            .insert("zag.ABCCHistory.SignalType1".into(), "BSE-COMP".into());
    }
    let bse_sa: Vec<SheetArea> = bse_areas
        .iter()
        .zip(&sa)
        .map(|(rec, s)| SheetArea {
            rec,
            img: AreaImage { ..s.img },
        })
        .collect();
    let bse = sheets::sheets(
        &SheetParams {
            um_per_mm: 0.0,
            ..p.clone()
        },
        &bse_sa,
        &grains,
        &own,
        &meta,
    );
    assert_eq!(
        bse.registration.auto_scale.as_ref().unwrap().target_mm,
        sheets::AUTO_TARGET_BSE_MM
    );
    let printed: usize = reg.bands.iter().map(|b| b.grains.len()).sum();
    assert_eq!(printed, 5);
    for b in &reg.bands {
        // Each band is a translation at 8 um per mm.
        let (sw, sh) = (
            b.sheet_rect_mm[2] - b.sheet_rect_mm[0],
            b.sheet_rect_mm[3] - b.sheet_rect_mm[1],
        );
        let (mw, mh) = (
            b.mosaic_rect_um[2] - b.mosaic_rect_um[0],
            b.mosaic_rect_um[3] - b.mosaic_rect_um[1],
        );
        assert!(
            (sw * 8.0 - mw).abs() < 0.01 && (sh * 8.0 - mh).abs() < 0.01,
            "{b:?}"
        );
        assert!(
            b.sheet_rect_mm[0] >= 0.0 && b.sheet_rect_mm[2] <= 431.8 && b.sheet_rect_mm[3] <= 279.4
        );
        assert_eq!(b.sample, "S1");
    }
    assert_band_geometry(&r);
    assert_eq!(
        (
            r.stats.grains_cut,
            r.stats.grains_twice,
            r.stats.grains_unmarked
        ),
        (0, 0, 0)
    );
    let pdf = grainatlas_core::layout::pdf::pdf_from_svgs(&r.pages).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    });
}

#[test]
fn rows_chain_grains_along_x() {
    let gs = [
        grain(10.0, 100.0, 20.0),
        grain(60.0, 130.0, 20.0),
        grain(30.0, 400.0, 20.0),
        grain(110.0, 160.0, 20.0),
    ];
    let refs: Vec<&Grain> = gs.iter().collect();
    let rows = sheets::rows_of(&refs, 120.0);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].len(), 3);
    assert!(
        rows[0]
            .windows(2)
            .all(|w| w[0].centroid_um[0] < w[1].centroid_um[0])
    );
}

/// Epoxy (0.3, noisy) at 2 um/px with discs drawn on it.
fn scene_with(w: usize, h: usize, draw: impl Fn(&mut Gray)) -> WorkImage {
    let mut g = Gray::filled(w, h, 0.3);
    draw(&mut g);
    let mut r = Rng(5);
    for v in g.px.iter_mut() {
        *v += ((r.next() - 0.5) * 0.02) as f32;
    }
    WorkImage {
        tiles_um: Vec::new(),
        signal: segment::Signal::Cl,
        img: g,
        cov: vec![true; w * h],
        um_per_px: 2.0,
    }
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
    let gs = segment::segment(
        &w,
        &SegmentParams {
            work_scale: 1,
            polarity: Polarity::Both,
            ..Default::default()
        },
        &[],
    );
    let real: Vec<&Grain> = gs.iter().filter(|g| g.is_grain()).collect();
    assert_eq!(
        real.len(),
        2,
        "{:?}",
        gs.iter()
            .map(|g| (g.centroid_um, g.eq_diameter_um, g.artifact))
            .collect::<Vec<_>>()
    );
    let left = real
        .iter()
        .find(|g| g.centroid_um[0] < 400.0)
        .expect("left grain");
    assert!(
        !segment::shape::inside(&left.polygon_um, 220.0, 176.0),
        "the bubble is off the grain"
    );
    assert!(
        gs.iter().any(|g| g.artifact == Some(Artifact::Bubble)
            && (g.centroid_um[0] - 220.0).hypot(g.centroid_um[1] - 176.0) < 20.0),
        "and flagged"
    );
    let right = real
        .iter()
        .find(|g| g.centroid_um[0] > 400.0)
        .expect("right grain");
    assert!(
        segment::shape::inside(&right.polygon_um, 724.0, 256.0),
        "the dark core stays in its grain"
    );
    assert!(right.eq_diameter_um > 180.0);
}

#[test]
fn grains_bridged_by_a_seam_line_are_split() {
    let w = scene_with(400, 200, |_| {});
    // Two squares joined by a 6 um bar, as one detection.
    let mut g = grain(0.0, 0.0, 1.0);
    g.polygon_um = vec![
        [100.0, 100.0],
        [260.0, 100.0],
        [260.0, 170.0],
        [400.0, 170.0],
        [400.0, 100.0],
        [560.0, 100.0],
        [560.0, 260.0],
        [400.0, 260.0],
        [400.0, 176.0],
        [260.0, 176.0],
        [260.0, 260.0],
        [100.0, 260.0],
    ];
    g.bbox_um = [100.0, 100.0, 560.0, 260.0];
    g.centroid_um = [330.0, 180.0];
    g.area_um2 = 2.0 * 160.0 * 160.0;
    g.eq_diameter_um = 255.0;
    let id = g.id;
    let mut gs = vec![g];
    segment::filter::split_bridged(&mut gs, &w, &SegmentParams::default(), 110.0);
    assert_eq!(
        gs.len(),
        2,
        "{:?}",
        gs.iter()
            .map(|g| (g.centroid_um, g.eq_diameter_um))
            .collect::<Vec<_>>()
    );
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
            [
                2.0 * (100.0 + 40.0 * a.cos()),
                2.0 * (100.0 + 40.0 * a.sin()),
            ]
        })
        .collect();
    let bubble: Vec<[f64; 2]> = (0..64)
        .map(|i| {
            let a = i as f64 / 64.0 * std::f64::consts::TAU;
            [
                2.0 * (160.0 + 22.0 * a.cos()),
                2.0 * (100.0 + 22.0 * a.sin()),
            ]
        })
        .collect();
    // Union as a hull-like outline: both circles' points (convex hull).
    let mut pts = g.polygon_um.clone();
    pts.extend(bubble);
    g.polygon_um = segment::filter::convex_hull(&pts);
    g.bbox_um = [120.0, 120.0, 364.0, 280.0];
    g.centroid_um = [230.0, 200.0];
    let parts = segment::split_grain(&w, &g, [276.0, 100.0, 276.0, 300.0], 0.8);
    assert_eq!(
        parts.len(),
        2,
        "{:?}",
        parts
            .iter()
            .map(|p| (p.centroid_um, p.artifact))
            .collect::<Vec<_>>()
    );
    let b = parts
        .iter()
        .find(|p| p.centroid_um[0] > 276.0)
        .expect("bubble part");
    assert_eq!(b.artifact, Some(Artifact::Bubble));
    assert!(
        parts
            .iter()
            .find(|p| p.centroid_um[0] < 276.0)
            .expect("grain part")
            .is_grain()
    );
}

std::thread_local! { static TEST_PACKING: std::cell::Cell<Packing> = const { std::cell::Cell::new(Packing::Lines) }; }

fn both_packings(test: impl Fn()) {
    for packing in [Packing::Lines, Packing::Packed] {
        TEST_PACKING.set(packing);
        test();
    }
    TEST_PACKING.set(Packing::Lines);
}

fn fast_sheets() -> SheetParams {
    SheetParams {
        packing: TEST_PACKING.get(),
        um_per_mm: 8.0,
        min_grain_um: 0.0,
        min_cluster_grains: 0,
        enhance: EnhanceParams {
            denoise: false,
            sharpen: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn field_sheets(
    p: &SheetParams,
    recs: &[AreaRecord],
    gs: &BTreeMap<String, Vec<Grain>>,
) -> sheets::SheetResult {
    field_sheets_at_pixel(p, recs, gs, 1.0)
}

fn field_sheets_at_pixel(
    p: &SheetParams,
    recs: &[AreaRecord],
    gs: &BTreeMap<String, Vec<Grain>>,
    pixel: f64,
) -> sheets::SheetResult {
    let pixels: Vec<Vec<u8>> = recs
        .iter()
        .map(|r| {
            let m = r.placement.as_ref().unwrap();
            vec![128; (m.width as f64 / pixel) as usize * (m.height as f64 / pixel) as usize]
        })
        .collect();
    let areas: Vec<_> = recs
        .iter()
        .zip(&pixels)
        .map(|(rec, px)| {
            let m = rec.placement.as_ref().unwrap();
            SheetArea {
                rec,
                img: AreaImage {
                    id: &rec.id,
                    name: &rec.name,
                    sample: &rec.sample,
                    w: (m.width as f64 / pixel) as usize,
                    h: (m.height as f64 / pixel) as usize,
                    px,
                    lut: std::array::from_fn(|i| i as u8),
                    um_per_px: pixel,
                },
            }
        })
        .collect();
    let own = ownership::assign(recs, gs, &OwnershipParams::default());
    let result = sheets::sheets(p, &areas, gs, &own, &SheetMeta::default());
    if p.packing == Packing::Lines {
        let packed = sheets::sheets(&SheetParams { packing: Packing::Packed, ..p.clone() }, &areas, gs, &own, &SheetMeta::default());
        same_cut(&result, &packed);
        assert_packed_blocks(&packed);
    }
    let printed: Vec<Vec<&Grain>> = recs
        .iter()
        .map(|a| {
            gs[&a.id]
                .iter()
                .filter(|g| {
                    !result
                        .registration
                        .strays
                        .iter()
                        .any(|s| s.area == a.id && s.id == g.id)
                        && own.is_owned(&a.id, &g.id)
                        && g.is_grain()
                        && g.eq_diameter_um >= p.min_grain_um
                        && (p.max_grain_um == 0.0 || g.eq_diameter_um <= p.max_grain_um)
                })
                .collect()
        })
        .collect();
    assert_eq!(
        sheets::audit(&result.registration, &printed, &areas),
        sheets::Audit {
            cut: 0,
            twice: 0,
            unmarked: 0,
            outside_body: 0,
            bands_overlapping: 0
        }
    );
    assert_eq!(
        (
            result.stats.grains_cut,
            result.stats.grains_twice,
            result.stats.grains_unmarked
        ),
        (0, 0, 0)
    );
    assert_band_geometry(&result);
    let mut isolated = 0;
    for b in &result.registration.bands {
        let r = b.mosaic_rect_um;
        let ai = recs.iter().position(|a| a.id == b.area).unwrap();
        let visible = printed[ai]
            .iter()
            .filter(|g| {
                let g = g.bbox_um;
                in_rect(r, [(g[0] + g[2]) / 2.0, (g[1] + g[3]) / 2.0])
            })
            .count();
        if visible < 3 {
            isolated += b.grains.len();
        }
        for g in &printed[ai] {
            let owned = b.grains.iter().any(|v| v.id == g.id);
            let context = b.context.iter().any(|v| v.id == g.id);
            assert_eq!(
                context,
                !owned
                    && b.repeated_um
                        .iter()
                        .any(|&r| rect_overlap(r, g.bbox_um).is_some()),
                "exact context: {b:?}, {g:?}"
            );
            if owned {
                assert!(holds(r, g.bbox_um));
            } else if rect_overlap(r, g.bbox_um).is_some() {
                // A visible context grain cut at any band edge has a repeated
                // strip there, including edges trimmed across the other axis.
                for d in 0..2 {
                    for high in [false, true] {
                        let edge = r[if high { d + 2 } else { d }];
                        if g.bbox_um[d] < edge && edge < g.bbox_um[d + 2] {
                            let mut border = rect_overlap(r, g.bbox_um).unwrap();
                            border[d] = edge;
                            border[d + 2] = edge;
                            assert!(
                                b.repeated_um.iter().any(|&s| holds(s, border)),
                                "unmarked context edge: {b:?}, {g:?}"
                            );
                        }
                    }
                }
            }
        }
    }
    assert_eq!(result.stats.isolated, isolated);
    result
}

fn single_field(p: &SheetParams, w: usize, h: usize, grains: Vec<Grain>) -> sheets::SheetResult {
    let a = area("001_Field", "Field", 0.0, w, h);
    field_sheets(
        p,
        std::slice::from_ref(&a),
        &BTreeMap::from([(a.id.clone(), grains)]),
    )
}

fn in_rect(r: [f64; 4], p: [f64; 2]) -> bool {
    p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]
}

fn holds(a: [f64; 4], b: [f64; 4]) -> bool {
    in_rect(a, [b[0], b[1]]) && in_rect(a, [b[2], b[3]])
}

fn rect_overlap(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let r = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}

/// Shared checks on every synthetic band: exact translation, paper bounds,
/// and no same-area overlap except the union of their marked repeated strips.
fn assert_band_geometry(result: &sheets::SheetResult) {
    let reg = &result.registration;
    for (i, b) in reg.bands.iter().enumerate() {
        let (s, r) = (b.sheet_rect_mm, b.mosaic_rect_um);
        for d in 0..2 {
            assert!(((s[d + 2] - s[d]) * reg.um_per_mm - (r[d + 2] - r[d])).abs() < 0.01);
            assert!(s[d] >= 0.0 && s[d + 2] <= reg.paper_mm[d], "{b:?}");
        }
        for &strip in &b.repeated_um {
            assert!(holds(r, strip));
        }
        for other in reg.bands.iter().skip(i + 1).filter(|a| a.area == b.area) {
            let Some(overlap) = rect_overlap(r, other.mosaic_rect_um) else {
                continue;
            };
            let strips: Vec<_> = b
                .repeated_um
                .iter()
                .chain(&other.repeated_um)
                .copied()
                .collect();
            // Split at every strip edge, so each cell is either entirely
            // covered or entirely unmarked; sampling is exact for rectangles.
            let edges = |d: usize| {
                let mut es = vec![overlap[d], overlap[d + 2]];
                es.extend(
                    strips
                        .iter()
                        .flat_map(|r| [r[d], r[d + 2]])
                        .filter(|v| *v > overlap[d] && *v < overlap[d + 2]),
                );
                es.sort_by(f64::total_cmp);
                es.dedup();
                es
            };
            for x in edges(0).windows(2) {
                for y in edges(1).windows(2) {
                    assert!(
                        strips
                            .iter()
                            .any(|&s| in_rect(s, [(x[0] + x[1]) / 2.0, (y[0] + y[1]) / 2.0])),
                        "unmarked overlap: {b:?}, {other:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn dense_two_dimensional_context_stays_in_one_band() {
    both_packings(|| {
    let gs: Vec<_> = (0..3)
        .flat_map(|y| {
            (0..6).map(move |x| grain(100.0 + x as f64 * 80.0, 100.0 + y as f64 * 80.0, 60.0))
        })
        .collect();
    let r = single_field(&fast_sheets(), 1000, 500, gs.clone());
    assert_eq!(
        (
            r.stats.clusters,
            r.stats.bands,
            r.stats.isolated,
            r.stats.cuts
        ),
        (1, 1, 0, 0)
    );
    let b = &r.registration.bands[0];
    assert_eq!(b.grains.len(), 18);
    assert!(gs.iter().all(|g| holds(b.mosaic_rect_um, g.bbox_um)));
    assert!(b.context.is_empty() && b.repeated_um.is_empty());
    });
}

#[test]
fn sparse_orphans_stay_separate_when_a_join_is_expensive() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 200.0,
        ..fast_sheets()
    };
    let gs: Vec<_> = (0..11)
        .map(|x| grain(100.0 + x as f64 * 600.0, 100.0, 60.0))
        .collect();
    let r = single_field(&p, 6500, 300, gs);
    assert!(r.registration.bands.iter().all(|b| b.grains.len() == 1));
    assert_eq!(r.stats.isolated, 11);
    assert_eq!(r.stats.cuts, 0);
    // One small orphan cannot join its distant neighbours within a full line.
    let gs = vec![
        grain(100.0, 100.0, 60.0),
        grain(5000.0, 100.0, 60.0),
        grain(5080.0, 100.0, 60.0),
        grain(5160.0, 100.0, 60.0),
    ];
    let r = single_field(&p, 5400, 300, gs);
    assert_eq!(r.stats.isolated, 1);
    assert_eq!(r.stats.bands, 2);
    assert!(r.pages.iter().any(|s| s.contains("1 isolated")));
    });
}

#[test]
fn long_empty_stretches_are_separate_nonoverlapping_clusters() {
    both_packings(|| {
    let gs: Vec<_> = [100.0, 10_000.0]
        .into_iter()
        .flat_map(|base| (0..3).map(move |x| grain(base + x as f64 * 80.0, 100.0, 60.0)))
        .collect();
    let r = single_field(&fast_sheets(), 10_500, 300, gs);
    assert_eq!((r.stats.clusters, r.stats.bands), (2, 2));
    let bs = &r.registration.bands;
    assert!(rect_overlap(bs[0].mosaic_rect_um, bs[1].mosaic_rect_um).is_none());
    assert!(
        r.pages.iter().any(|s| s.contains(">//</text>"))
            || bs[0].sheet_rect_mm[1] != bs[1].sheet_rect_mm[1]
    );
    assert_eq!((bs[0].row, bs[1].row), (1, 2));
    });
}

#[test]
fn wide_cluster_prefers_the_final_ten_percent_and_repeats_context() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 500.0,
        overlap_um: 180.0,
        ..fast_sheets()
    };
    // Full line: 3326.4 um. A unique 340 um clear gap (2450..2790)
    // lies outside the final 10 %, among otherwise 20 um clear gaps.
    let xs: Vec<f64> = (0..149)
        .map(|i| 100.0 + i as f64 * 80.0)
        .filter(|x| *x <= 2490.0 || *x >= 2810.0)
        .collect();
    let gs: Vec<_> = [100.0, 180.0]
        .into_iter()
        .flat_map(|y| xs.iter().map(move |&x| grain(x, y, 60.0)))
        .collect();
    let r = single_field(&p, 12_200, 300, gs.clone());
    assert_eq!(r.stats.clusters, 1);
    assert!(r.stats.bands >= 4);
    assert_eq!(r.stats.cuts + 1, r.stats.bands);
    let bs = &r.registration.bands;
    let first_cut = bs[0]
        .repeated_um
        .iter()
        .map(|r| r[0])
        .fold(f64::INFINITY, f64::min);
    assert!(
        (first_cut - 3180.0).abs() < 1e-7,
        "nearest qualifying gap in the final 10 %: {first_cut}"
    );
    for b in bs.iter().take(bs.len() - 1) {
        let cut = b
            .repeated_um
            .iter()
            .find(|s| s[2] == b.mosaic_rect_um[2])
            .unwrap()[0];
        let limit = b.mosaic_rect_um[0] + (431.8 - 16.0) * p.um_per_mm;
        assert!(cut >= limit - 0.1 * (431.8 - 16.0) * p.um_per_mm && cut <= limit);
        assert!(
            gs.iter()
                .all(|g| !(g.bbox_um[0] < cut && g.bbox_um[2] > cut)),
            "empty column at every cut"
        );
        assert!(b.mosaic_rect_um[2] - cut >= p.overlap_um);
    }
    assert!(bs.iter().all(|b| b.row == 1));
    assert!(
        bs.windows(2)
            .all(|w| w[0].mosaic_rect_um[2] - w[1].mosaic_rect_um[0] >= p.overlap_um)
    );
    assert_eq!(bs.iter().map(|b| b.grains.len()).sum::<usize>(), gs.len());
    assert!(bs.iter().any(|b| !b.context.is_empty()));
    assert!(r.pages.iter().all(|s| s.contains("repeated")
        && s.contains("repeated-hatch")
        && !s.contains("fill=\"white\" opacity=")
        && s.contains("<mask")));
    let pdf = grainatlas_core::layout::pdf::pdf_from_svgs(&r.pages).unwrap();
    assert!(
        pdf.starts_with(b"%PDF"),
        "repeated-strip masks render to PDF"
    );
    });
}

#[test]
fn cuts_keep_minimum_owned_groups_at_heads_and_tails() {
    for (d, prefill) in [(0, false), (1, false), (0, true)] {
        let step = if d == 0 { 600.0 } else { 260.0 };
        let p = SheetParams {
            context_um: step,
            overlap_um: 80.0,
            ..fast_sheets()
        };
        // Previously these rows left a one-grain tail: 6, 5, 1 along x;
        // 5, 5, 1 along y. The partial line also made a two-grain head.
        let count = if d == 0 { 12 } else { 11 };
        let gs: Vec<_> = (0..count)
            .map(|i| {
                let along = 100.0 + i as f64 * step;
                let [x, y] = if d == 0 {
                    [along, 100.0]
                } else {
                    [100.0, along]
                };
                grain(x, y, 60.0)
            })
            .collect();
        let (w, h) = if d == 0 { (9500, 300) } else { (300, 4500) };
        let a = area("002_Row", "Row", 0.0, w, h);
        let mut fields = vec![a.clone()];
        let mut grains = BTreeMap::from([(a.id.clone(), gs)]);
        if prefill {
            let first = area("001_First", "First", 20_000_000.0, 2000, 300);
            grains.insert(
                first.id.clone(),
                (0..23)
                    .map(|i| grain(100.0 + i as f64 * 80.0, 100.0, 60.0))
                    .collect(),
            );
            fields.insert(0, first);
        }
        let result = field_sheets(&p, &fields, &grains);
        let bands: Vec<_> = result
            .registration
            .bands
            .iter()
            .filter(|b| b.area == a.id)
            .collect();
        assert!(result.stats.cuts > 0);
        assert!(
            bands.iter().all(|b| b.grains.len() >= p.min_group),
            "axis {d}, prefill {prefill}: {:?}",
            bands.iter().map(|b| b.grains.len()).collect::<Vec<_>>()
        );
        assert_eq!(bands.iter().map(|b| b.grains.len()).sum::<usize>(), count);
        if prefill {
            let first = &result.registration.bands[0];
            assert!(
                bands[0].sheet > first.sheet || bands[0].sheet_rect_mm[1] > first.sheet_rect_mm[1]
            );
            assert_eq!(
                bands[0].sheet_rect_mm[0], p.margin_mm,
                "a two-grain head spends a new line"
            );
        }
        // A cluster too small to form two groups still permits small pieces.
        let step = if d == 0 { 1000.0 } else { 500.0 };
        let p = SheetParams {
            context_um: step,
            ..p
        };
        let gs = (0..5)
            .map(|i| {
                let along = 100.0 + i as f64 * step;
                let [x, y] = if d == 0 {
                    [along, 100.0]
                } else {
                    [100.0, along]
                };
                grain(x, y, 60.0)
            })
            .collect();
        let result = single_field(&p, w, h, gs);
        assert!(result.stats.cuts > 0);
        assert!(
            result
                .registration
                .bands
                .iter()
                .any(|b| b.grains.len() < p.min_group)
        );
        assert_eq!(
            result
                .registration
                .bands
                .iter()
                .map(|b| b.grains.len())
                .sum::<usize>(),
            5
        );
    }
}

#[test]
fn empty_repeated_strips_widen_to_show_and_register_a_neighbour() {
    both_packings(|| {
    for (d, step, context) in [
        (0, 360.0, 400.0),
        (1, 360.0, 400.0),
        (0, 500.0, 600.0),
        (1, 500.0, 600.0),
    ] {
        // Keep the first vertical cut at the original test capacity despite the
        // shorter header, so it still exercises an initially empty strip.
        // The wider spacing needs room for three owners and both strips.
        let p = SheetParams {
            context_um: context,
            margin_mm: if d == 1 { 16.0 } else { 8.0 },
            um_per_mm: if step > 400.0 { 18.0 } else { 8.0 },
            ..fast_sheets()
        };
        let gs: Vec<_> = (0..30)
            .map(|i| {
                let along = 100.0 + i as f64 * step;
                let [x, y] = if d == 0 {
                    [along, 100.0]
                } else {
                    [100.0, along]
                };
                grain(x, y, 60.0)
            })
            .collect();
        let reference = segment::filter::reference_size(&gs);
        let default = reference.max(p.context_um / 3.0);
        let (w, h) = if d == 0 { (16_000, 300) } else { (300, 16_000) };
        let result = single_field(&p, w, h, gs.clone());
        let mut widened = 0;
        for b in &result.registration.bands {
            for &strip in &b.repeated_um {
                let mut original = strip;
                let forward = strip[d + 2] == b.mosaic_rect_um[d + 2];
                if forward {
                    original[d + 2] = original[d] + default;
                } else {
                    original[d] = original[d + 2] - default;
                }
                if gs
                    .iter()
                    .any(|g| rect_overlap(original, g.bbox_um).is_some())
                {
                    continue;
                }
                let at = if forward { strip[d] } else { strip[d + 2] };
                let nearest = gs
                    .iter()
                    .filter_map(|g| {
                        if forward && g.bbox_um[d] >= at {
                            Some(g.bbox_um[d] - at)
                        } else if !forward && g.bbox_um[d + 2] <= at {
                            Some(at - g.bbox_um[d + 2])
                        } else {
                            None
                        }
                    })
                    .min_by(f64::total_cmp)
                    .unwrap();
                if nearest >= p.context_um / 2.0 {
                    assert!(
                        ((strip[d + 2] - strip[d]) - default).abs() < 1.0,
                        "keep the default when no box is within the cap"
                    );
                    continue;
                }
                widened += 1;
                assert!(strip[d + 2] - strip[d] > default);
                assert!(
                    ((strip[d + 2] - strip[d])
                        - (nearest + reference / 2.0).min(p.context_um / 2.0))
                    .abs()
                        < 1.0,
                    "reach the nearest box plus r/2, up to the cap"
                );
                assert!(
                    strip[d + 2] - strip[d] <= p.context_um / 2.0 + 1.0,
                    "cap with outward pixel snapping"
                );
                let neighbour = gs.iter().find(|g| rect_overlap(strip, g.bbox_um).is_some()).unwrap_or_else(|| panic!("the widened strip reaches a grain box: axis {d}, strip {strip:?}, band {b:?}"));
                assert!(
                    b.context.iter().any(|g| g.id == neighbour.id),
                    "the visible neighbour is registered as context"
                );
            }
        }
        assert!(
            widened > 0,
            "axis {d}, spacing {step} must exercise an originally empty strip"
        );
    }
    });
}

#[test]
fn svg_scale_bars_stay_in_owned_space_outside_repeated_strips() {
    both_packings(|| {
    for d in 0..2 {
        let gs = (0..80)
            .map(|i| {
                let along = 100.0 + i as f64 * 80.0;
                let [x, y] = if d == 0 {
                    [along, 100.0]
                } else {
                    [100.0, along]
                };
                grain(x, y, if d == 0 { 60.0 } else { 80.0 })
            })
            .collect();
        let p = fast_sheets();
        let (w, h) = if d == 0 { (6700, 300) } else { (300, 6700) };
        let result = single_field(&p, w, h, gs);
        let mut bars = 0;
        let mut right_strips = 0;
        for (pi, svg) in result.pages.iter().enumerate() {
            let doc = roxmltree::Document::parse(svg).unwrap();
            let paths: Vec<_> = doc
                .descendants()
                .filter(|n| n.has_tag_name("g") && n.attribute("stroke-linecap") == Some("butt"))
                .map(|n| n.children().find(|c| c.has_tag_name("path")).unwrap())
                .collect();
            let bands: Vec<_> = result
                .registration
                .bands
                .iter()
                .filter(|b| b.sheet == pi + 1)
                .collect();
            assert_eq!(paths.len(), bands.len());
            for (path, b) in paths.iter().zip(bands) {
                let coords: Vec<f64> = path
                    .attribute("d")
                    .unwrap()
                    .split(|c: char| c.is_ascii_alphabetic() || c.is_whitespace())
                    .filter(|v| !v.is_empty())
                    .map(|v| v.parse().unwrap())
                    .collect();
                let (x0, y, x1) = (coords[0], coords[1], coords[2]);
                assert!(x0 >= b.sheet_rect_mm[0] && x1 <= b.sheet_rect_mm[2]);
                for strip in &b.repeated_um {
                    let s = [
                        b.sheet_rect_mm[0] + (strip[0] - b.mosaic_rect_um[0]) / p.um_per_mm,
                        b.sheet_rect_mm[1] + (strip[1] - b.mosaic_rect_um[1]) / p.um_per_mm,
                        b.sheet_rect_mm[0] + (strip[2] - b.mosaic_rect_um[0]) / p.um_per_mm,
                        b.sheet_rect_mm[1] + (strip[3] - b.mosaic_rect_um[1]) / p.um_per_mm,
                    ];
                    assert!(
                        x1 < s[0] || x0 > s[2] || y < s[1] || y > s[3],
                        "scale bar [{x0}, {y}, {x1}] overlaps strip {s:?}"
                    );
                    if strip[0] > b.mosaic_rect_um[0] && strip[2] == b.mosaic_rect_um[2] {
                        right_strips += 1;
                        assert!(
                            (x1 - (s[0] - 2.2)).abs() < 0.001,
                            "bar ends left of the right strip"
                        );
                    }
                }
                if !b
                    .repeated_um
                    .iter()
                    .any(|s| s[0] > b.mosaic_rect_um[0] && s[2] == b.mosaic_rect_um[2])
                {
                    assert!(
                        (x1 - (b.sheet_rect_mm[2] - 2.2)).abs() < 0.001,
                        "without a right strip, keep the right edge"
                    );
                }
                bars += 1;
            }
        }
        assert!(bars > 0);
        if d == 0 {
            assert!(bars > right_strips && right_strips > 0);
        }
    }
    });
}

#[test]
fn cut_uses_the_nearest_empty_column_that_is_wide_enough() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 500.0,
        overlap_um: 180.0,
        ..fast_sheets()
    };
    // The 340 um gap at 2450..2790 is widest, but the 100 um gap at
    // 3010..3110 also clears r (about 68 um) and is nearer the line end.
    // Later 20 um gaps do not qualify.
    let gs = (0..80)
        .map(|i| 100.0 + i as f64 * 80.0)
        .filter(|x| (*x <= 2490.0 || *x >= 2810.0) && *x != 3060.0)
        .map(|x| grain(x, 100.0, 60.0))
        .collect();
    let r = single_field(&p, 6700, 300, gs);
    let window_start =
        r.registration.bands[0].mosaic_rect_um[0] + 0.9 * (431.8 - 16.0) * p.um_per_mm;
    assert_eq!(
        r.registration.bands[0].repeated_um[0][0],
        (window_start + 3110.0) / 2.0
    );
    // When every gap is narrower than r, the widest width is the threshold.
    // Equal 20 um gaps qualify, so take the last midpoint whose strip fits.
    let gs = (0..80)
        .map(|i| grain(100.0 + i as f64 * 80.0, 100.0, 60.0))
        .collect();
    let r = single_field(&p, 6700, 300, gs);
    assert_eq!(r.registration.bands[0].repeated_um[0][0], 3180.0);
    });
}

#[test]
fn cut_pieces_follow_their_own_grains_on_a_wandering_row() {
    both_packings(|| {
    let p = SheetParams {
        pad_um: 22.2,
        ..fast_sheets()
    };
    for d in 0..2 {
        let gs: Vec<_> = (0..110)
            .map(|i| {
                let along = 100.3 + i as f64 * 80.0;
                let across = 45.2 + i as f64 * 8.0;
                let xy = if d == 0 {
                    [along, across]
                } else {
                    [across, along]
                };
                grain(xy[0], xy[1], 60.7)
            })
            .collect();
        let (w, h) = if d == 0 { (9200, 1300) } else { (1300, 9200) };
        let a = area("001_Field", "Field", 0.0, w, h);
        let r = field_sheets_at_pixel(
            &p,
            std::slice::from_ref(&a),
            &BTreeMap::from([(a.id.clone(), gs.clone())]),
            2.5,
        );
        assert_eq!(r.stats.clusters, 1);
        assert!(r.stats.cuts >= 2);
        let other = 1 - d;
        let mut spans = Vec::new();
        for b in &r.registration.bands {
            let owned: Vec<_> = gs
                .iter()
                .filter(|g| b.grains.iter().any(|own| own.id == g.id))
                .collect();
            assert!(!owned.is_empty());
            let lo = owned
                .iter()
                .map(|g| g.bbox_um[other])
                .fold(f64::INFINITY, f64::min)
                - p.pad_um;
            let hi = owned
                .iter()
                .map(|g| g.bbox_um[other + 2])
                .fold(f64::NEG_INFINITY, f64::max)
                + p.pad_um;
            assert_eq!(b.mosaic_rect_um[other], (lo / 2.5).floor().max(0.0) * 2.5);
            assert_eq!(b.mosaic_rect_um[other + 2], (hi / 2.5).ceil() * 2.5);
            for strip in &b.repeated_um {
                assert_eq!(
                    (strip[other], strip[other + 2]),
                    (b.mosaic_rect_um[other], b.mosaic_rect_um[other + 2])
                );
            }
            spans.push(b.mosaic_rect_um[other + 2] - b.mosaic_rect_um[other]);
        }
        assert!(
            spans.iter().all(|&span| span < 500.0),
            "each slice is shorter than the full wandering span: {spans:?}"
        );
    }
    });
}

#[test]
fn auto_overlap_is_the_larger_of_reference_size_and_a_third_of_context() {
    both_packings(|| {
    let gs: Vec<_> = (0..80)
        .map(|i| grain(100.0 + i as f64 * 80.0, 100.0, 60.0))
        .collect();
    let reference = segment::filter::reference_size(&gs);
    for (context, floor) in [(0.0, 420.0), (0.0, 1.0), (100.0, 420.0), (600.0, 420.0)] {
        let p = SheetParams {
            context_um: context,
            gap_break_um: floor,
            ..fast_sheets()
        };
        let c = if context > 0.0 {
            context
        } else {
            (3.0 * reference).max(floor)
        };
        let expected = reference.max(c / 3.0);
        let r = single_field(&p, 6700, 300, gs.clone());
        assert!(r.stats.cuts > 0);
        for b in &r.registration.bands {
            for strip in &b.repeated_um {
                assert!(
                    ((strip[2] - strip[0]) - expected).abs() < 1.0,
                    "auto overlap with outward pixel snapping: {strip:?}, expected {expected}"
                );
            }
        }
    }
    });
}

#[test]
fn orphan_joins_a_cluster_that_already_needs_cuts() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 100.0,
        ..fast_sheets()
    };
    for d in 0..2 {
        let xy = |along, across| {
            if d == 0 {
                [along, across]
            } else {
                [across, along]
            }
        };
        let cluster: Vec<_> = (0..90)
            .map(|i| {
                let [x, y] = xy(100.0 + i as f64 * 80.0, 100.0);
                grain(x, y, 60.0)
            })
            .collect();
        for (position, clusters) in [(7400.0, 1), (12_000.0, 2)] {
            let [x, y] = xy(position, 100.0);
            let orphan = grain(x, y, 60.0);
            let mut gs = cluster.clone();
            gs.push(orphan.clone());
            let (w, h) = if d == 0 { (12_300, 300) } else { (300, 12_300) };
            let r = single_field(&p, w, h, gs);
            assert_eq!(
                r.stats.clusters, clusters,
                "a cheap nearby join still must fit or add at most one line"
            );
            let band = r
                .registration
                .bands
                .iter()
                .find(|b| b.grains.iter().any(|g| g.id == orphan.id))
                .unwrap();
            if clusters == 1 {
                assert_eq!(band.row, 1);
                assert!(band.grains.len() >= p.min_group);
                assert_eq!(r.stats.isolated, 0);
            } else {
                assert_eq!(band.grains.len(), 1);
                assert_eq!(r.stats.isolated, 1);
            }
        }
    }
    });
}

#[test]
fn wide_tall_clusters_cut_width_before_local_height() {
    let p = SheetParams {
        context_um: 200.0,
        overlap_um: 80.0,
        ..fast_sheets()
    };
    let gs: Vec<_> = (0..31)
        .flat_map(|y| {
            (0..46).map(move |x| grain(100.0 + x as f64 * 80.0, 100.0 + y as f64 * 80.0, 60.0))
        })
        .collect();
    let r = single_field(&p, 3900, 2700, gs);
    assert_eq!(r.stats.clusters, 1);
    assert!(r.stats.bands >= 4);
    let bs = &r.registration.bands;
    assert_eq!(bs[0].mosaic_rect_um[0], bs[1].mosaic_rect_um[0]);
    assert!(bs[0].mosaic_rect_um[1] < bs[1].mosaic_rect_um[1]);
    assert!(bs[2].mosaic_rect_um[0] > bs[0].mosaic_rect_um[0]);
    assert_eq!(r.stats.isolated, 0);
}

#[test]
fn crossing_cuts_keep_each_centroid_owner_whole() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 100.0,
        overlap_um: 70.0,
        ..fast_sheets()
    };
    // No empty column: wide overlapping boxes, centres displaced in y.
    let gs: Vec<_> = (0..70)
        .map(|i| {
            grain(
                200.0 + i as f64 * 70.0,
                200.0 + (i % 2) as f64 * 100.0,
                140.0,
            )
        })
        .collect();
    let r = single_field(&p, 5300, 500, gs.clone());
    assert!(r.stats.cuts > 0);
    let first = &r.registration.bands[0];
    let cut = first.repeated_um[0][0];
    assert_eq!(cut, 3350.0, "fewest crossings, then nearest feasible limit");
    assert_eq!(
        gs.iter()
            .filter(|g| g.bbox_um[0] < cut && g.bbox_um[2] > cut)
            .count(),
        1
    );
    for b in &r.registration.bands {
        for owned in &b.grains {
            let g = gs.iter().find(|g| g.id == owned.id).unwrap();
            assert!(holds(b.mosaic_rect_um, g.bbox_um));
        }
    }
    });
}

#[test]
fn whole_cluster_spends_a_new_line_and_bands_never_mix_areas() {
    let p = SheetParams {
        context_um: 100.0,
        min_group: 3,
        ..fast_sheets()
    };
    let gs: Vec<_> = [100.0, 1800.0]
        .into_iter()
        .flat_map(|base| (0..17).map(move |x| grain(base + x as f64 * 80.0, 100.0, 60.0)))
        .collect();
    // Each cluster fits whole, but both plus the break cannot share one line.
    let p = SheetParams {
        um_per_mm: 6.0,
        ..p
    };
    let r = single_field(&p, 3400, 300, gs);
    assert_eq!((r.stats.clusters, r.stats.bands, r.stats.cuts), (2, 2, 0));
    assert_ne!(
        r.registration.bands[0].sheet_rect_mm[1],
        r.registration.bands[1].sheet_rect_mm[1]
    );
    let a = area("001_A", "A", 0.0, 400, 300);
    let b = area("002_B", "B", -1_000_000.0, 400, 300);
    let gs = BTreeMap::from([
        (a.id.clone(), vec![grain(100.0, 200.0, 60.0)]),
        (b.id.clone(), vec![grain(100.0, 100.0, 60.0)]),
    ]);
    let r = field_sheets(&p, &[a, b], &gs);
    assert_eq!(r.stats.bands, 2);
    assert_eq!(
        r.registration.bands[0].sheet_rect_mm[1],
        r.registration.bands[1].sheet_rect_mm[1]
    );
    assert_ne!(r.registration.bands[0].area, r.registration.bands[1].area);
    assert_eq!(
        r.registration.bands[0].sample, "B",
        "samples follow mount top, not input order"
    );
}

#[test]
fn box_neighbours_and_intersecting_rectangles_merge_to_stability() {
    both_packings(|| {
    // Diagonal box gap uses max(dx, dy), not Euclidean or centroid distance.
    let p = SheetParams {
        context_um: 50.0,
        min_group: 1,
        pad_um: 0.0,
        ..fast_sheets()
    };
    let r = single_field(
        &p,
        400,
        400,
        vec![grain(100.0, 100.0, 60.0), grain(200.0, 200.0, 60.0)],
    );
    assert_eq!(r.stats.clusters, 1);
    // Three initially separate rectangles. First union reaches the third,
    // even though none of its grains are neighbours of that third cluster.
    let p = SheetParams {
        context_um: 10.0,
        min_group: 1,
        pad_um: 80.0,
        ..fast_sheets()
    };
    let gs = vec![
        grain(100.0, 100.0, 20.0),
        grain(260.0, 260.0, 20.0),
        grain(100.0, 420.0, 20.0),
    ];
    let r = single_field(&p, 500, 600, gs);
    assert_eq!((r.stats.clusters, r.stats.bands), (1, 1));
    // A cheap orphan join can still reach another retained rectangle.
    // Intersection merging runs after omission and restores property 4.
    let p = SheetParams {
        context_um: 100.0,
        min_group: 3,
        ..fast_sheets()
    };
    let mut gs: Vec<_> = [100.0, 980.0]
        .into_iter()
        .flat_map(|y| (0..21).map(move |i| grain(100.0 + i as f64 * 80.0, y, 60.0)))
        .collect();
    gs.extend((1..11).map(|i| grain(100.0, 100.0 + i as f64 * 80.0, 60.0)));
    gs.push(grain(1880.0, 100.0, 60.0));
    gs.extend((0..3).map(|i| grain(1900.0, 420.0 + i as f64 * 80.0, 60.0)));
    let r = single_field(&p, 2100, 1100, gs);
    assert_eq!((r.stats.clusters, r.stats.bands), (1, 1));
    });
}

#[test]
fn cluster_order_follows_the_long_axis_and_auto_context_uses_reference_size() {
    let p = SheetParams {
        context_um: 10.0,
        min_group: 1,
        pad_um: 0.0,
        ..fast_sheets()
    };
    let gs = vec![grain(100.0, 400.0, 60.0), grain(400.0, 100.0, 60.0)];
    let r = single_field(&p, 800, 600, gs.clone());
    assert_eq!(r.registration.bands[0].grains[0].id, gs[0].id);
    let r = single_field(&p, 600, 800, gs.clone());
    assert_eq!(r.registration.bands[0].grains[0].id, gs[1].id);
    let gs: Vec<_> = (0..6)
        .map(|i| grain(150.0 + i as f64 * 500.0, 150.0, 140.0))
        .collect();
    let auto = SheetParams {
        gap_break_um: 1.0,
        min_group: 1,
        ..fast_sheets()
    };
    let r = single_field(&auto, 3000, 400, gs.clone());
    assert_eq!(r.stats.clusters, 1, "3r links a 360 um box gap");
    let explicit = SheetParams {
        context_um: 350.0,
        ..auto.clone()
    };
    assert_eq!(
        single_field(&explicit, 3000, 400, gs.clone())
            .stats
            .clusters,
        6
    );
    let floor = SheetParams {
        gap_break_um: 600.0,
        ..auto
    };
    assert_eq!(single_field(&floor, 3000, 400, gs).stats.clusters, 1);
}

#[test]
fn old_sheet_json_defaults_and_audit_checks_ownership_not_visibility() {
    both_packings(|| {
    let p: SheetParams = serde_json::from_str(r#"{"gap_break_um":420}"#).unwrap();
    assert_eq!(
        (
            p.context_um,
            p.min_group,
            p.overlap_um,
            p.min_cluster_grains
        ),
        (0.0, 3, 0.0, 5)
    );
    let g = grain(100.0, 100.0, 60.0);
    let r = single_field(&fast_sheets(), 400, 300, vec![g.clone()]);
    let mut json = serde_json::to_value(&r.registration.bands[0]).unwrap();
    json.as_object_mut().unwrap().remove("context");
    json.as_object_mut().unwrap().remove("repeated_um");
    let old: sheets::BandRecord = serde_json::from_value(json).unwrap();
    assert!(old.context.is_empty() && old.repeated_um.is_empty());
    let a = area("001_Field", "Field", 0.0, 400, 300);
    let pixels = vec![128; 400 * 300];
    let sa = SheetArea {
        rec: &a,
        img: AreaImage {
            id: &a.id,
            name: &a.name,
            sample: &a.sample,
            w: 400,
            h: 300,
            px: &pixels,
            lut: [128; 256],
            um_per_px: 1.0,
        },
    };
    let areas = [sa];
    let printed = vec![vec![&g]];
    let mut reg = r.registration;
    reg.bands.push(old);
    assert_eq!(
        sheets::audit(&reg, &printed, &areas),
        sheets::Audit {
            cut: 0,
            twice: 1,
            unmarked: 0,
            outside_body: 0,
            bands_overlapping: 1
        }
    );
    reg.bands[1].grains.clear();
    assert_eq!(
        sheets::audit(&reg, &printed, &areas),
        sheets::Audit {
            cut: 0,
            twice: 0,
            unmarked: 1,
            outside_body: 0,
            bands_overlapping: 1
        }
    );
    reg.bands[1].context = reg.bands[0].grains.clone();
    assert_eq!(
        sheets::audit(&reg, &printed, &areas),
        sheets::Audit { bands_overlapping: 1, ..Default::default() }
    );
    reg.bands[0].mosaic_rect_um[2] = g.bbox_um[2] - 0.1;
    assert_eq!(
        sheets::audit(&reg, &printed, &areas).cut,
        1,
        "context visibility cannot repair a cut owner"
    );
    reg.bands[0].grains.clear();
    assert_eq!(
        sheets::audit(&reg, &printed, &areas).cut,
        1,
        "context alone is not ownership"
    );
    });
}

#[test]
fn oversized_clusters_use_remaining_space_only_above_forty_percent() {
    for count in [10, 30] {
        let a = area("001_Field", "Field", 0.0, 2700, 300);
        let b = area("002_Field", "Field", -20_000_000.0, 6000, 300);
        let gs = BTreeMap::from([
            (
                a.id.clone(),
                (0..count)
                    .map(|i| grain(100.0 + i as f64 * 80.0, 100.0, 60.0))
                    .collect(),
            ),
            (
                b.id.clone(),
                (0..70)
                    .map(|i| grain(100.0 + i as f64 * 80.0, 100.0, 60.0))
                    .collect(),
            ),
        ]);
        let r = field_sheets(&fast_sheets(), &[a, b], &gs);
        let first = &r.registration.bands[0];
        let second = &r.registration.bands[1];
        if count == 10 {
            assert_eq!(
                first.sheet_rect_mm[1], second.sheet_rect_mm[1],
                "use the remaining line when at least 40 % remains"
            );
            assert!(second.sheet_rect_mm[0] > first.sheet_rect_mm[2]);
        } else {
            assert!(second.sheet > first.sheet || second.sheet_rect_mm[1] > first.sheet_rect_mm[1]);
            assert_eq!(
                second.sheet_rect_mm[0], 8.0,
                "start at the full line width below 40 % remaining"
            );
        }
    }
}

#[test]
fn fractional_scale_and_pixels_keep_the_translation_exact() {
    both_packings(|| {
    let p = SheetParams {
        um_per_mm: 5.7,
        pad_um: 22.2,
        context_um: 250.0,
        overlap_um: 91.3,
        ..fast_sheets()
    };
    let a = area("001_Field", "Field", 0.0, 6000, 500);
    let gs = BTreeMap::from([(
        a.id.clone(),
        (0..70)
            .map(|i| grain(100.3 + i as f64 * 80.1, 100.2, 60.7))
            .collect(),
    )]);
    let r = field_sheets_at_pixel(&p, &[a], &gs, 2.5);
    assert!(r.stats.cuts > 0);
    for b in &r.registration.bands {
        assert!(
            b.mosaic_rect_um
                .iter()
                .all(|v| (v / 2.5).fract().abs() < 1e-7)
        );
    }
    });
}

#[test]
fn diagonal_cluster_retains_context_when_slices_have_empty_sides() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 200.0,
        overlap_um: 100.0,
        ..fast_sheets()
    };
    let gs = (0..70)
        .map(|i| grain(100.0 + i as f64 * 80.0, 100.0 + i as f64 * 40.0, 60.0))
        .collect();
    let r = single_field(&p, 6000, 3100, gs);
    assert_eq!(r.stats.clusters, 1);
    assert!(r.stats.bands > 2);
    assert_eq!(r.stats.isolated, 0);
    });
}

/// The operator's drawn rectangle (TM4000 `.zag`), in navigation display pixels of 1 um with the
/// top-left stage corner at (0, 0) and stage x and y falling to the right and down: a rectangle
/// centred at stage (-cx, -cy) um, `w` x `h` um.
fn draw(a: &mut AreaRecord, cx: f64, cy: f64, w: f64, h: f64) {
    for (k, v) in [
        ("ImageInfoPixelSize", 500.0),
        ("ImageInfoWidth", 1280.0),
        ("ImageDispInfoWidth", 640.0),
        ("ImageInfoTopLeftStagePositionX", 0.0),
        ("ImageInfoTopLeftStagePositionY", 0.0),
        ("ImageInfoBottomRightStagePositionX", -1.0e8),
        ("ImageInfoBottomRightStagePositionY", -1.0e8),
        ("CenterX", cx),
        ("CenterY", cy),
        ("AreaWidth", w),
        ("AreaHeight", h),
    ] {
        a.area_meta.insert(format!("zag.{k}"), v.to_string());
    }
}

#[test]
fn project_restores_navigation_for_drawn_ownership() {
    let (mut areas, grains) = two_areas();
    let id = grains["001_S1"][1].id;
    let params = OwnershipParams::default();
    assert_eq!(ownership::assign(&areas, &grains, &params).of("001_S1", &id).unwrap().owner_area, "002_S1_2");
    draw(&mut areas[0], 250.0, 0.0, 500.0, 600.0);
    draw(&mut areas[1], 900.0, 0.0, 100.0, 100.0);
    let mut meta = BTreeMap::new();
    for key in ownership::NAVIGATION_KEYS {
        meta.insert(key.to_string(), areas[0].area_meta.remove(key).unwrap());
        areas[1].area_meta.remove(key);
    }
    // An existing area value wins, even when the package has a different value.
    areas[1].area_meta.insert("zag.ImageInfoWidth".into(), "2560".into());
    let mut other = area("A1", "S2", 0.0, 100, 100);
    other.area_meta.insert("zag.CenterY".into(), "0".into());
    areas.push(other.clone());
    let json = serde_json::json!({
        "schema_version": 1, "uuid": grainatlas_core::project::new_id(), "name": "S1", "created": "",
        "package": {"path": "", "reader": "tm4000", "name": "S1", "meta": meta}, "areas": areas,
    }).to_string();
    let project = grainatlas_core::project::Project::from_json(&json).unwrap();
    assert!(ownership::drawn_area_nm(&project.areas[0]).is_some());
    assert!(ownership::drawn_area_nm(&project.areas[1]).is_some());
    for key in ownership::NAVIGATION_KEYS {
        assert_eq!(project.areas[0].area_meta[key], project.package.meta[key]);
    }
    assert_eq!(project.areas[1].area_meta["zag.ImageInfoWidth"], "2560");
    assert_eq!(project.areas[2].area_meta, other.area_meta);
    let owned = ownership::assign(&project.areas, &grains, &params);
    assert_eq!(owned.of("001_S1", &id).unwrap().owner_area, "001_S1");
    assert!(!owned.of("001_S1", &id).unwrap().outside_drawn);
}

#[test]
fn ownership_follows_the_operators_drawn_areas() {
    // At 50X, B's tile grid holds A's row better than A's own grid does, and A's drawn
    // rectangle lies inside B's. A's grain at (900, 300) is stage (-400, 0) um,
    // 300 um inside B and 70 um inside A: the coverage rule gives it to B.
    let (mut areas, grains) = two_areas();
    let a = grains["001_S1"].clone();
    let b = grains["002_S1_2"].clone();
    let p = OwnershipParams::default();
    assert_eq!(
        ownership::assign(&areas, &grains, &p)
            .of("001_S1", &a[1].id)
            .unwrap()
            .owner_area,
        "002_S1_2"
    );

    // A's drawn rectangle: stage x -500..0 um (A's mosaic x 500..1000), the full height.
    draw(&mut areas[0], 250.0, 0.0, 500.0, 600.0);
    let r = ownership::drawn_area_nm(&areas[0]).unwrap();
    assert!(
        (r[0] + 500_000.0).abs() < 1.0 && r[2].abs() < 1.0 && (r[1] + 300_000.0).abs() < 1.0,
        "{r:?}"
    );
    let o = ownership::assign(&areas, &grains, &p);
    let r = o.of("002_S1_2", &b[0].id).unwrap();
    assert_eq!(r.owner_area, "001_S1", "inside A's drawn area only");
    assert_eq!(r.duplicate_of, Some(a[1].id));
    assert!(!r.outside_drawn);
    // A's (200, 300) is stage +300 um: in no drawn area, so the coverage rule keeps it in A and
    // marks it for review.
    let r = o.of("001_S1", &a[0].id).unwrap();
    assert!(o.is_owned("001_S1", &a[0].id) && r.outside_drawn);

    // B's drawn rectangle nested in A's, around the same grain: the smaller one wins.
    draw(&mut areas[1], 400.0, 0.0, 100.0, 100.0);
    let o = ownership::assign(&areas, &grains, &p);
    assert_eq!(o.of("001_S1", &a[1].id).unwrap().owner_area, "002_S1_2");
    assert_eq!(
        o.of("001_S1", &a[1].id).unwrap().duplicate_of,
        Some(b[0].id)
    );
    // B's (385, 151) = A's (985, 150), stage (-485, 149) um: inside A's rectangle, but A's
    // detection is cut by A's mosaic edge, so B (whole) keeps it and it is marked for review.
    let r = o.of("001_S1", &a[2].id).unwrap();
    assert_eq!(r.owner_area, "002_S1_2");
    assert!(r.cut && r.outside_drawn);
}

/// Eq. diameters of `n` grains, log-normal with this median and 10th percentile.
fn sizes(n: usize, median: f64, p10: f64) -> Vec<f64> {
    // Inverse normal CDF (Acklam), enough for quantiles of a test distribution.
    fn z(p: f64) -> f64 {
        let a = [
            -39.696830,
            220.946098,
            -275.928510,
            138.357751,
            -30.664798,
            2.506628,
        ];
        let b = [-54.476098, 161.585836, -155.698979, 66.801311, -13.280681];
        let c = [
            -0.007784894,
            -0.32239645,
            -2.4007582,
            -2.5497325,
            4.3746641,
            2.938164,
        ];
        let d = [0.007784695, 0.32246712, 2.445134, 3.7544086];
        if p < 0.02425 {
            let q = (-2.0 * p.ln()).sqrt();
            (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5])
                / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
        } else if p > 1.0 - 0.02425 {
            -z(1.0 - p)
        } else {
            let q = p - 0.5;
            let r = q * q;
            (((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) * q
                / (((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1.0)
        }
    }
    let s = (median / p10).ln() / 1.2816;
    (0..n)
        .map(|i| median * (s * z((i as f64 + 0.5) / n as f64)).exp())
        .collect()
}

#[test]
fn auto_scale_prints_the_median_grain_at_the_target_on_a_round_scale() {
    let spots = sheets::SheetParams::default().spot_sizes_um;
    let (cl, bse) = (sheets::AUTO_TARGET_CL_MM, sheets::AUTO_TARGET_BSE_MM);
    // (mount, median, 10th percentile of the printed grains in um, target, expected um per mm),
    // represented by synthetic distributions. Zircon mount 2 uses 8 um/mm;
    // the BSE examples cover scales from 10 to 20 um/mm.
    for (mount, median, p10, target, k) in [
        ("zircon mount 1", 81.0, 57.0, cl, 5.0),
        ("zircon mount 2", 123.0, 89.0, cl, 8.0),
        ("zircon mount 3", 191.0, 48.0, cl, 12.5),
        ("zircon mount 4", 287.0, 66.0, cl, 16.0),
        ("apatite mount 1", 166.0, 89.0, bse, 16.0),
        ("apatite mount 2", 85.0, 59.0, bse, 8.0),
    ] {
        let a = sheets::auto_scale(&sizes(300, median, p10), &spots, target);
        assert_eq!(a.um_per_mm, k, "{mount}: {a:?}");
        assert_eq!(
            (a.limit.as_str(), a.target_mm),
            ("target", target),
            "{mount}"
        );
        assert!(sheets::ROUND_SCALES.contains(&a.um_per_mm));
        let mm = a.median_um / a.um_per_mm;
        // The coarsest round scale that still reaches the target: one R10 step coarser misses it.
        assert!(
            (target..target * 1.3).contains(&mm),
            "{mount}: median prints {mm} mm"
        );
        assert!(
            (a.median_um - median).abs() < 0.02 * median && (a.p10_um - p10).abs() < 0.05 * p10,
            "{mount}: {a:?}"
        );
    }
}

#[test]
fn auto_scale_gives_paper_before_markable_grains_zoning_or_the_spot_key() {
    let spots = [15.0, 20.0, 25.0, 30.0, 35.0, 50.0];
    // Very large grains: CL zoning (10 um at 0.4 mm) caps the scale at 25 um per mm.
    let a = sheets::auto_scale(&sizes(200, 1200.0, 900.0), &spots, 15.0);
    assert_eq!((a.um_per_mm, a.limit.as_str()), (25.0, "zoning"));
    // A wide spread: the small grains (10th percentile 40 um) still print 3 mm or more.
    let a = sheets::auto_scale(&sizes(200, 450.0, 40.0), &spots, 15.0);
    assert_eq!((a.um_per_mm, a.limit.as_str()), (12.5, "small grains"));
    assert!(a.p10_um / a.um_per_mm >= sheets::MIN_MARK_MM);
    // Small laser spots: the third largest key circle stays 1 mm across.
    let a = sheets::auto_scale(&sizes(200, 450.0, 400.0), &[8.0, 10.0, 12.0], 15.0);
    assert_eq!((a.um_per_mm, a.limit.as_str()), (8.0, "spot key"));
    // No grains: a 100 um default, never a panic.
    assert_eq!(sheets::auto_scale(&[], &spots, 15.0).um_per_mm, 6.3);
}

#[test]
fn the_spot_key_leaves_off_circles_too_small_to_draw_or_tell_apart() {
    let spots = [15.0, 20.0, 25.0, 30.0, 35.0, 50.0];
    assert_eq!(sheets::key_spots(&spots, 8.0), (spots.to_vec(), vec![]));
    assert_eq!(
        sheets::key_spots(&spots, 16.0),
        (vec![20.0, 25.0, 30.0, 35.0, 50.0], vec![15.0])
    );
    // At 20 um/mm 30 um (1.5 mm) is 0.25 mm from 35 um, and 20 um 0.25 mm from 25 um.
    assert_eq!(
        sheets::key_spots(&spots, 20.0),
        (vec![25.0, 35.0, 50.0], vec![15.0, 20.0, 30.0])
    );
}

#[test]
fn sheet_scale_reads_auto_or_a_number() {
    let p: SheetParams = serde_json::from_str(r#"{"um_per_mm":"auto"}"#).unwrap();
    assert_eq!(p.um_per_mm, 0.0);
    let p: SheetParams = serde_json::from_str(r#"{"um_per_mm":8}"#).unwrap();
    assert_eq!(p.um_per_mm, 8.0, "a saved explicit scale is kept");
    let p: SheetParams = serde_json::from_str("{}").unwrap();
    assert_eq!(p.um_per_mm, 0.0, "auto by default");
    assert!(
        serde_json::to_string(&p)
            .unwrap()
            .contains(r#""um_per_mm":"auto""#)
    );
    assert!(
        serde_json::to_string(&SheetParams {
            um_per_mm: 12.5,
            ..p
        })
        .unwrap()
        .contains(r#""um_per_mm":12.5"#)
    );
    assert!(serde_json::from_str::<SheetParams>(r#"{"um_per_mm":"big"}"#).is_err());
}

#[test]
fn strays_are_reported_after_orphan_merging_and_never_become_targets() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 100.0,
        min_cluster_grains: SheetParams::default().min_cluster_grains,
        ..fast_sheets()
    };
    let mut gs: Vec<_> = (0..6)
        .map(|i| grain(100.0 + i as f64 * 80.0, 100.0, 60.0))
        .collect();
    let mut omitted = Vec::new();
    for count in 1..=4 {
        omitted.extend((0..count).map(|i| {
            grain(
                100.0 + (count - 1) as f64 * 4000.0 + i as f64 * 80.0,
                2200.0,
                60.0,
            )
        }));
    }
    gs.extend(omitted.clone());
    let original = gs.clone();
    let result = single_field(&p, 14_000, 2500, gs.clone());
    assert_eq!(
        (result.stats.grains, result.stats.strays, result.stats.bands),
        (6, 10, 1)
    );
    assert_eq!(result.registration.strays.len(), omitted.len());
    for g in &omitted {
        let stray = result
            .registration
            .strays
            .iter()
            .find(|s| s.id == g.id)
            .unwrap();
        assert_eq!(
            (
                stray.area.as_str(),
                stray.number,
                stray.eq_diameter_um,
                stray.centroid_um,
                stray.bbox_um
            ),
            (
                "001_Field",
                g.number,
                g.eq_diameter_um,
                g.centroid_um,
                g.bbox_um
            )
        );
        assert!(
            result.registration.bands.iter().all(|b| b
                .grains
                .iter()
                .chain(&b.context)
                .all(|v| v.id != g.id))
        );
    }
    assert!(result.pages[0].contains("10 strays left off"));
    assert_eq!(gs, original, "layout never changes the project's grains");
    let disabled = single_field(
        &SheetParams {
            min_cluster_grains: 0,
            ..p.clone()
        },
        14_000,
        2500,
        gs,
    );
    assert_eq!((disabled.stats.grains, disabled.stats.strays), (16, 0));
    // The orphan merge runs first: a nearby singleton is still a target.
    let mut gs: Vec<_> = (0..30)
        .map(|i| grain(100.0 + i as f64 * 80.0, 100.0, 60.0))
        .collect();
    gs.push(grain(2600.0, 100.0, 60.0)); // 120 um box gap, < 15% added area.
    let merged = single_field(&p, 2800, 300, gs);
    assert_eq!(
        (merged.stats.grains, merged.stats.strays, merged.stats.bands),
        (31, 0, 1)
    );
    });
}

#[test]
fn stray_rule_preserves_small_areas_and_all_largest_ties() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 100.0,
        min_cluster_grains: 5,
        ..fast_sheets()
    };
    for count in 1..=6 {
        let gs = (0..count)
            .map(|i| grain(100.0 + i as f64 * 80.0, 100.0, 60.0))
            .collect();
        let result = single_field(&p, 800, 300, gs);
        assert_eq!((result.stats.grains, result.stats.strays), (count, 0));
    }
    let gs = [100.0, 6000.0]
        .into_iter()
        .flat_map(|base| (0..3).map(move |i| grain(base + i as f64 * 80.0, 100.0, 60.0)))
        .collect();
    let result = single_field(&p, 6500, 300, gs);
    assert_eq!(
        (
            result.stats.grains,
            result.stats.strays,
            result.stats.clusters
        ),
        (6, 0, 2)
    );
    });
}

#[test]
fn wide_cluster_with_a_sparse_bottom_slice_stays_inside_the_body() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 1400.0,
        overlap_um: 100.0,
        ..fast_sheets()
    };
    let mut gs: Vec<_> = [100.0, 1400.0]
        .into_iter()
        .flat_map(|y| (0..80).map(move |i| grain(100.0 + i as f64 * 80.0, y, 60.0)))
        .collect();
    gs.extend([100.0, 3300.0, 6400.0].map(|x| grain(x, 1700.0, 60.0)));
    let result = single_field(&p, 6700, 1900, gs.clone());
    assert_eq!(result.stats.grains, gs.len());
    assert_eq!(result.stats.bands_outside_body, 0);
    assert!(
        result
            .registration
            .bands
            .iter()
            .all(|b| holds(result.registration.body_rect_mm, b.sheet_rect_mm)
                && !b.grains.is_empty())
    );
    });
}

#[test]
fn audit_detects_a_band_outside_paper_even_when_mosaic_owners_are_whole() {
    both_packings(|| {
    let g = grain(100.0, 100.0, 60.0);
    let mut result = single_field(&fast_sheets(), 400, 300, vec![g.clone()]);
    let a = area("001_Field", "Field", 0.0, 400, 300);
    let pixels = vec![128; 400 * 300];
    let areas = [SheetArea {
        rec: &a,
        img: AreaImage {
            id: &a.id,
            name: &a.name,
            sample: &a.sample,
            w: 400,
            h: 300,
            px: &pixels,
            lut: [128; 256],
            um_per_px: 1.0,
        },
    }];
    for d in 0..2 {
        let original = result.registration.bands[0].sheet_rect_mm;
        result.registration.bands[0].sheet_rect_mm[d + 2] =
            result.registration.body_rect_mm[d + 2] + 1.0;
        let audit = sheets::audit(&result.registration, &[vec![&g]], &areas);
        assert_eq!(
            audit,
            sheets::Audit {
                outside_body: 1,
                ..Default::default()
            }
        );
        result.registration.bands[0].sheet_rect_mm = original;
    }
    });
}

#[test]
fn fill_measures_image_area_and_mean_occupied_line_width() {
    let result = single_field(
        &fast_sheets(),
        1200,
        300,
        (0..6)
            .map(|i| grain(100.0 + i as f64 * 80.0, 100.0, 60.0))
            .collect(),
    );
    let body = result.registration.body_rect_mm;
    let band = result.registration.bands[0].sheet_rect_mm;
    let bw = body[2] - body[0];
    assert!((result.stats.fill.line_width - (band[2] - band[0]) / bw).abs() < 1e-10);
    assert!(
        (result.stats.fill.band_area
            - (band[2] - band[0]) * (band[3] - band[1]) / (bw * (body[3] - body[1])))
            .abs()
            < 1e-10
    );
}

#[test]
fn a_narrow_height_remainder_shares_its_line_with_the_next_width_piece() {
    let p = SheetParams {
        context_um: 1200.0,
        overlap_um: 100.0,
        ..fast_sheets()
    };
    let mut gs: Vec<_> = (0..120)
        .map(|i| grain(100.0 + i as f64 * 80.0, 700.0, 60.0))
        .collect();
    let bottom = grain(100.0, 1900.0, 60.0);
    gs.extend([grain(100.0, 100.0, 60.0), bottom.clone()]);
    let result = single_field(&p, 10_000, 2100, gs);
    let bands = &result.registration.bands;
    let i = bands
        .iter()
        .position(|b| b.grains.iter().any(|g| g.id == bottom.id))
        .unwrap();
    assert!(bands[i].sheet_rect_mm[2] - bands[i].sheet_rect_mm[0] < 100.0);
    assert_eq!(
        (bands[i].sheet, bands[i].sheet_rect_mm[1]),
        (bands[i + 1].sheet, bands[i + 1].sheet_rect_mm[1])
    );
    assert!(bands[i + 1].sheet_rect_mm[0] > bands[i].sheet_rect_mm[2]);
}

#[test]
fn fitting_a_height_slice_marks_context_clipped_at_its_side_edge() {
    both_packings(|| {
    // This fixture deliberately permits a heavily repeated height slice to test
    // the side-edge hatch independently of the default panel-worth limit.
    let p = SheetParams {
        context_um: 1000.0,
        overlap_um: 100.0,
        max_repeated_fraction: 1.0,
        ..fast_sheets()
    };
    let mut gs: Vec<_> = (0..100)
        .map(|i| grain(100.0 + i as f64 * 80.0, 1000.0, 60.0))
        .collect();
    let mut crossing = grain(5000.0, 1100.0, 600.0);
    crossing.bbox_um = [4600.0, 900.0, 5400.0, 1480.0];
    let bottom = grain(5200.0, 2100.0, 60.0);
    gs.extend([
        grain(6000.0, 100.0, 60.0),
        crossing.clone(),
        bottom.clone(),
        grain(5280.0, 2100.0, 60.0),
        grain(5360.0, 2100.0, 60.0),
    ]);
    let result = single_field(&p, 8300, 2300, gs);
    let band = result
        .registration
        .bands
        .iter()
        .find(|b| b.grains.iter().any(|g| g.id == bottom.id))
        .unwrap();
    assert!(!band.grains.iter().any(|g| g.id == crossing.id));
    assert!(
        crossing.bbox_um[0] < band.mosaic_rect_um[0]
            && band.mosaic_rect_um[0] < crossing.bbox_um[2]
    );
    assert!(band.context.iter().any(|g| g.id == crossing.id));
    let mut edge = rect_overlap(band.mosaic_rect_um, crossing.bbox_um).unwrap();
    edge[2] = edge[0];
    assert!(band.repeated_um.iter().any(|&s| holds(s, edge)));
    });
}

#[test]
fn orphan_joins_are_bounded_by_grain_gap_and_rectangle_growth() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 100.0,
        min_cluster_grains: 5,
        ..fast_sheets()
    };
    let host: Vec<_> = (0..90)
        .map(|i| grain(100.0 + i as f64 * 80.0, 100.0, 60.0))
        .collect();
    // Both proposed unions add little width. Only the first crosses <= 1.25c.
    for (x, stray) in [(7400.0, 0), (7500.0, 1)] {
        let mut gs = host.clone();
        gs.push(grain(x, 100.0, 60.0));
        let r = single_field(&p, 7800, 500, gs);
        assert_eq!((r.stats.grains, r.stats.strays), (91 - stray, stray));
    }
    // The gap is only 120 um, but a bottom singleton more than doubles the
    // rectangle area. A larger sheet must not make this join cheap.
    for paper in ["letter", "tabloid", "a3"] {
        let mut gs = host.clone();
        gs.push(grain(2000.0, 280.0, 60.0));
        let r = single_field(
            &SheetParams {
                paper: paper.into(),
                ..p.clone()
            },
            7800,
            500,
            gs,
        );
        assert_eq!((r.stats.grains, r.stats.strays), (90, 1));
    }
    });
}

#[test]
fn rectangle_merging_cannot_rescue_a_far_disconnected_stray() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 100.0,
        min_cluster_grains: 5,
        ..fast_sheets()
    };
    let mut host: Vec<_> = [100.0, 980.0]
        .into_iter()
        .flat_map(|y| (0..21).map(move |i| grain(100.0 + i as f64 * 80.0, y, 60.0)))
        .collect();
    host.extend((1..11).map(|i| grain(100.0, 100.0 + i as f64 * 80.0, 60.0)));
    let far = grain(1500.0, 400.0, 60.0);
    let near = grain(1500.0, 260.0, 60.0); // exactly c from the top row: a real neighbour.
    let mut gs = host.clone();
    gs.extend([far.clone(), near.clone()]);
    // With both present they are connected to each other and the host.
    let connected = single_field(&p, 1900, 1100, gs);
    assert_eq!(connected.stats.strays, 0);
    let mut gs = host.clone();
    gs.push(far.clone());
    let omitted = single_field(&p, 1900, 1100, gs.clone());
    assert_eq!(omitted.stats.grains, host.len());
    assert_eq!(omitted.registration.strays[0].id, far.id);
    assert!(
        omitted.registration.bands.iter().all(|b| b
            .grains
            .iter()
            .chain(&b.context)
            .all(|g| g.id != far.id))
    );
    let disabled = single_field(
        &SheetParams {
            min_cluster_grains: 0,
            ..p
        },
        1900,
        1100,
        gs,
    );
    assert_eq!(
        (
            disabled.stats.bands,
            disabled.stats.strays,
            disabled.stats.grains
        ),
        (1, 0, host.len() + 1)
    );
    });
}

#[test]
fn sheet_bodies_and_height_budgets_use_each_headers_actual_height() {
    let gs = (0..130)
        .flat_map(|y| {
            (0..5).map(move |x| grain(100.0 + x as f64 * 80.0, 100.0 + y as f64 * 80.0, 60.0))
        })
        .collect();
    let r = single_field(&fast_sheets(), 600, 10_600, gs);
    assert!(r.stats.sheets > 2);
    assert_eq!(r.stats.header_mm, [50.0, 36.0]);
    assert!(r.pages[0].contains("INSTRUMENT"));
    assert!(
        r.pages
            .iter()
            .skip(1)
            .all(|s| s.contains("acquisition details on sheet 1")
                && !s.contains("INSTRUMENT")
                && !s.contains("IMAGED"))
    );
    let [w, h] = r.registration.paper_mm;
    let bw = w - 16.0;
    let mut area = 0.0;
    let mut body_area = 0.0;
    for (body, budget) in r
        .registration
        .sheet_bodies
        .iter()
        .zip(&r.stats.height_budget)
    {
        assert_eq!(body.header_mm, if body.sheet == 1 { 50.0 } else { 36.0 });
        assert_eq!(body.body_rect_mm[1], 9.0 + body.header_mm);
        assert!(
            r.registration
                .bands
                .iter()
                .filter(|b| b.sheet == body.sheet)
                .all(|b| holds(body.body_rect_mm, b.sheet_rect_mm))
        );
        let total = budget.header_mm2
            + budget.footer_margins_mm2
            + budget.labels_notes_mm2
            + budget.band_mm2
            + budget.blank_below_mm * bw
            + budget.blank_right_mm2
            + budget.other_line_mm2;
        assert!((total - w * h).abs() < 1e-6);
        assert!(budget.empty_epoxy_mm2 >= 0.0 && budget.empty_epoxy_mm2 <= budget.band_mm2);
        area += budget.band_mm2;
        body_area += bw * (body.body_rect_mm[3] - body.body_rect_mm[1]);
    }
    assert!((r.stats.fill.band_area - area / body_area).abs() < 1e-10);
    // A tall cluster uses most of every sheet before the final remainder.
    assert!(
        r.stats
            .height_budget
            .iter()
            .take(r.stats.sheets - 1)
            .all(|b| b.blank_below_mm < 35.0)
    );
}

#[test]
fn audit_uses_the_sheet_body_before_its_legacy_envelope() {
    both_packings(|| {
    let g = grain(100.0, 100.0, 60.0);
    let mut r = single_field(&fast_sheets(), 400, 300, vec![g.clone()]);
    let a = area("001_Field", "Field", 0.0, 400, 300);
    let pixels = vec![128; 400 * 300];
    let areas = [SheetArea {
        rec: &a,
        img: AreaImage {
            id: &a.id,
            name: &a.name,
            sample: &a.sample,
            w: 400,
            h: 300,
            px: &pixels,
            lut: [128; 256],
            um_per_px: 1.0,
        },
    }];
    let top = r.registration.sheet_bodies[0].body_rect_mm[1];
    r.registration.body_rect_mm[1] = top - 14.0;
    r.registration.bands[0].sheet_rect_mm[1] = top - 9.0; // inside envelope, above sheet 1 body.
    assert_eq!(
        sheets::audit(&r.registration, &[vec![&g]], &areas).outside_body,
        1
    );
    r.registration.sheet_bodies.clear();
    assert_eq!(
        sheets::audit(&r.registration, &[vec![&g]], &areas).outside_body,
        0,
        "old registrations retain their body fallback"
    );
    });
}

#[test]
fn registration_filename_is_the_exact_footer_and_export_name() {
    both_packings(|| {
    let p = SheetParams {
        registration_file: "mount_A_print_registration.json".into(),
        ..fast_sheets()
    };
    let r = single_field(&p, 400, 300, vec![grain(100.0, 100.0, 60.0)]);
    assert_eq!(r.registration.registration_file, p.registration_file);
    assert!(r.pages[0].contains(&format!(
        "registration {}",
        r.registration.registration_file
    )));
    });
}

#[test]
fn empty_epoxy_counts_empty_rows_without_counting_artifacts_as_targets() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 1000.0,
        ..fast_sheets()
    };
    let mut artifact = grain(100.0, 500.0, 60.0);
    artifact.artifact = Some(Artifact::Manual);
    let r = single_field(
        &p,
        300,
        1100,
        vec![
            grain(100.0, 100.0, 60.0),
            grain(100.0, 900.0, 60.0),
            artifact,
        ],
    );
    // The 904 um tall rectangle contains two 104 um padded target intervals.
    assert!((r.stats.fill.empty_epoxy - 696.0 / 904.0).abs() < 1e-10);
    assert!((r.stats.height_budget[0].empty_epoxy_mm2 - 104.0 * 696.0 / 64.0).abs() < 1e-10);
    assert_eq!(r.stats.artifacts, 1);
    });
}

#[test]
fn compact_map_keeps_six_asymmetrically_spaced_labels_inside_its_box() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 600.0,
        ..fast_sheets()
    };
    let recs: Vec<_> = (0..6)
        .map(|i| {
            let mut a = area(&format!("{i:03}_A{i}"), &format!("A{i}"), 0.0, 4000, 1200);
            a.tiles[0].stage_nm.as_mut().unwrap()[1] = if i == 5 {
                -100_000_000.0
            } else {
                -i as f64 * 1_000_000.0
            };
            a
        })
        .collect();
    let grains: BTreeMap<_, _> = recs
        .iter()
        .map(|a| {
            (
                a.id.clone(),
                [100.0, 700.0]
                    .into_iter()
                    .flat_map(|y| (0..20).map(move |x| grain(100.0 + x as f64 * 160.0, y, 60.0)))
                    .collect(),
            )
        })
        .collect();
    let r = field_sheets(&p, &recs, &grains);
    assert!(r.stats.sheets > 1);
    let doc = roxmltree::Document::parse(&r.pages[1]).unwrap();
    let map = doc
        .descendants()
        .find(|n| n.has_tag_name("rect") && n.attribute("rx") == Some("1.5"))
        .unwrap();
    let top: f64 = map.attribute("y").unwrap().parse().unwrap();
    let height: f64 = map.attribute("height").unwrap().parse().unwrap();
    if r.registration.layout == "packed" {
        assert_compact_header_boxes(&r, p.margin_mm);
        let labels: Vec<_> = map.parent().unwrap().children().filter(|n| n.has_tag_name("text") && n.attribute("font-weight") == Some("bold")).collect();
        assert!(!labels.is_empty());
        let here: std::collections::BTreeSet<_> = r.registration.bands.iter().filter(|b| b.sheet == 2).map(|b| b.sample.as_str()).collect();
        let mut accounted = 0;
        for label in labels {
            let name = label.text().unwrap();
            if name.starts_with("+ ") {
                accounted += name.trim_start_matches("+ ").trim_end_matches(" more").parse::<usize>().unwrap();
            } else {
                assert!(here.contains(name));
                accounted += 1;
            }
            assert_eq!(label.attribute("font-size"), Some("1.8"));
            let y: f64 = label.attribute("y").unwrap().parse().unwrap();
            assert!(y - 1.8 >= top && y + 0.4 <= top + height - 3.0);
        }
        assert_eq!(accounted, here.len(), "every on-sheet sample has a label or is counted as more");
        return;
    }
    for a in &recs {
        let label = doc
            .descendants()
            .find(|n| {
                n.has_tag_name("text")
                    && n.text() == Some(&a.name)
                    && matches!(n.attribute("font-size"), Some("2.2" | "2.5"))
            })
            .unwrap();
        let y: f64 = label.attribute("y").unwrap().parse().unwrap();
        assert!(
            y - 2.5 >= top && y + 1.0 <= top + height - 3.0,
            "{} at {y}, map {top} + {height}",
            a.name
        );
    }
    });
}

#[test]
fn a_mostly_repeated_height_tail_moves_the_width_piece_whole() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 650.0,
        overlap_um: 140.0,
        min_cluster_grains: 5,
        ..fast_sheets()
    };
    let mut first = area("001_First", "First", 20_000_000.0, 3400, 1200);
    first.tiles[0].stage_nm = Some([20_000_000.0, 20_000_000.0]);
    let second = area("002_Field", "Field", 0.0, 5000, 1400);
    let prefill = (0..3)
        .flat_map(|y| {
            (0..39).map(move |x| grain(100.0 + x as f64 * 80.0, 100.0 + y as f64 * 480.0, 60.0))
        })
        .collect();
    let mut gs: Vec<_> = (0..60)
        .map(|x| grain(100.0 + x as f64 * 80.0, 500.0, 60.0))
        .collect();
    let bottom: Vec<_> = (0..6)
        .map(|x| grain(900.0 + x as f64 * 240.0, 1150.0, 60.0))
        .collect();
    gs.extend(bottom.clone());
    let grains = BTreeMap::from([(first.id.clone(), prefill), (second.id.clone(), gs)]);
    let recs = [first, second];
    let permissive = field_sheets(
        &SheetParams {
            max_repeated_fraction: 1.0,
            ..p.clone()
        },
        &recs,
        &grains,
    );
    assert!(
        permissive
            .registration
            .bands
            .iter()
            .any(|b| b.cut_sources.iter().any(|s| s == "height_sheet")),
        "fixture must reproduce the old leftover-height slice: {:?}",
        permissive
            .registration
            .bands
            .iter()
            .map(|b| (&b.area, b.sheet, b.mosaic_rect_um, &b.cut_sources))
            .collect::<Vec<_>>()
    );
    let result = field_sheets(&p, &recs, &grains);
    let band = result
        .registration
        .bands
        .iter()
        .find(|b| b.grains.iter().any(|g| g.id == bottom[0].id))
        .unwrap();
    assert_eq!(band.sheet, 2);
    assert!(
        bottom
            .iter()
            .all(|g| band.grains.iter().any(|own| own.id == g.id))
    );
    assert!(!band.cut_sources.iter().any(|s| s == "height_sheet"));
    assert!(
        band.mosaic_rect_um[1] < 500.0 && band.mosaic_rect_um[3] > 1150.0,
        "both rows stay together"
    );
    assert_eq!(result.stats.forced_cuts, 0);
    assert!(result.stats.worst_repeated_fraction <= p.max_repeated_fraction);
    });
}

#[test]
fn a_few_owner_tail_moves_the_cut_earlier_in_either_direction() {
    both_packings(|| {
    for d in 0..2 {
        let step = if d == 0 { 340.0 } else { 140.0 };
        let p = SheetParams {
            context_um: step,
            overlap_um: 80.0,
            min_cut_grains: 5,
            ..fast_sheets()
        };
        let gs = (0..12)
            .map(|i| {
                let [x, y] = if d == 0 {
                    [100.0 + i as f64 * step, 100.0]
                } else {
                    [100.0, 100.0 + i as f64 * step]
                };
                grain(x, y, 60.0)
            })
            .collect();
        let (w, h) = if d == 0 { (4500, 300) } else { (300, 2000) };
        let result = single_field(&p, w, h, gs);
        assert_eq!(result.stats.bands, 2);
        assert_eq!(result.stats.forced_cuts, 0);
        assert!(
            result
                .registration
                .bands
                .iter()
                .all(|b| b.grains.len() >= 5)
        );
        assert_eq!(result.stats.grains, 12);
    }
    });
}

#[test]
fn a_forced_cut_keeps_every_owner_once_and_marks_the_context() {
    both_packings(|| {
    for (min_cut_grains, max_repeated_fraction) in [(5, 1.0 / 3.0), (0, 0.01)] {
        let p = SheetParams {
            min_cut_grains,
            max_repeated_fraction,
            context_um: 1000.0,
            overlap_um: 140.0,
            ..fast_sheets()
        };
        let gs: Vec<_> = (0..5)
            .map(|x| grain(100.0 + x as f64 * 900.0, 100.0, 60.0))
            .collect();
        let result = single_field(&p, 4000, 300, gs.clone());
        assert_eq!(
            (
                result.stats.bands,
                result.stats.cuts,
                result.stats.forced_cuts
            ),
            (2, 1, 1)
        );
        for g in gs {
            assert_eq!(
                result
                    .registration
                    .bands
                    .iter()
                    .flat_map(|b| &b.grains)
                    .filter(|own| own.id == g.id)
                    .count(),
                1
            );
        }
        assert!(
            result
                .registration
                .bands
                .iter()
                .all(|b| !b.repeated_um.is_empty())
        );
        if min_cut_grains > 0 {
            let mut counts: Vec<_> = result
                .registration
                .bands
                .iter()
                .map(|b| b.grains.len())
                .collect();
            counts.sort_unstable();
            assert_eq!(
                counts,
                [2, 3],
                "least bad ownership deficit, rather than a one-owner tail"
            );
        }
    }
    });
}

#[test]
fn panel_worth_fields_default_for_older_json() {
    both_packings(|| {
    let p: SheetParams = serde_json::from_str("{}").unwrap();
    assert_eq!((p.min_cut_grains, p.max_repeated_fraction), (0, 1.0 / 3.0));
    let stats: sheets::SheetStats = serde_json::from_str(r#"{"fill":{"band_area":0.5}}"#).unwrap();
    assert_eq!(
        (
            stats.fill.repeated,
            stats.worst_repeated_fraction,
            stats.forced_cuts
        ),
        (0.0, 0.0, 0)
    );
    let result = single_field(&fast_sheets(), 400, 300, vec![grain(100.0, 100.0, 60.0)]);
    let mut json = serde_json::to_value(&result.registration.bands[0]).unwrap();
    json.as_object_mut().unwrap().remove("cut_sources");
    assert!(
        serde_json::from_value::<sheets::BandRecord>(json)
            .unwrap()
            .cut_sources
            .is_empty()
    );
    });
}

#[test]
fn repeated_fill_counts_intersecting_strips_once_and_weights_band_area() {
    both_packings(|| {
    let p = SheetParams {
        context_um: 150.0,
        overlap_um: 100.0,
        ..fast_sheets()
    };
    let gs = (0..20)
        .flat_map(|y| {
            (0..40).map(move |x| grain(100.0 + x as f64 * 100.0, 100.0 + y as f64 * 100.0, 60.0))
        })
        .collect();
    let result = single_field(&p, 4200, 2200, gs);
    let mut repeat = 0.0;
    let mut total = 0.0;
    let mut worst: f64 = 0.0;
    let mut saw_intersection = false;
    for b in &result.registration.bands {
        let r = b.mosaic_rect_um;
        let area = (r[2] - r[0]) * (r[3] - r[1]);
        let edges = |d: usize| {
            let mut es: Vec<_> = b
                .repeated_um
                .iter()
                .flat_map(|s| [s[d], s[d + 2]])
                .collect();
            es.sort_by(f64::total_cmp);
            es.dedup();
            es
        };
        let mut covered = 0.0;
        for x in edges(0).windows(2) {
            for y in edges(1).windows(2) {
                let count = b
                    .repeated_um
                    .iter()
                    .filter(|&&s| in_rect(s, [(x[0] + x[1]) / 2.0, (y[0] + y[1]) / 2.0]))
                    .count();
                saw_intersection |= count > 1;
                if count > 0 {
                    covered += (x[1] - x[0]) * (y[1] - y[0]);
                }
            }
        }
        total += area;
        repeat += covered;
        worst = worst.max(covered / area);
    }
    assert!(
        saw_intersection,
        "fixture must exercise inherited x and y strips"
    );
    assert!((result.stats.fill.repeated - repeat / total).abs() < 1e-10);
    assert!((result.stats.worst_repeated_fraction - worst).abs() < 1e-10);
    });
}

/// The viewer's mount inset (ui/src/lib/mount-inset.ts `stageBox`) ports `stage_box`. Both
/// must give the boxes stored in this invented fixture; ui/src/lib/mount-inset.test.mjs checks the port.
#[test]
fn stage_boxes_match_the_inset_fixture() {
    let doc: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/projects/mount-inset-areas.json"
    ))
    .unwrap();
    let areas: Vec<AreaRecord> = serde_json::from_value(doc["areas"].clone()).unwrap();
    let expected: Vec<Option<[f64; 4]>> =
        serde_json::from_value(doc["expected_boxes"].clone()).unwrap();
    assert_eq!(areas.len(), expected.len());
    for (a, e) in areas.iter().zip(&expected) {
        match (sheets::stage_box(a), e) {
            (Some(g), Some(e)) => assert!(
                g.iter().zip(e).all(|(g, e)| (g - e).abs() < 1e-9),
                "{}: {g:?} != {e:?}",
                a.id
            ),
            (g, e) => assert_eq!(g, *e, "{}", a.id),
        }
    }
}

fn golden_mount() -> (SheetParams, Vec<AreaRecord>, BTreeMap<String, Vec<Grain>>) {
    let mut recs = vec![area("A1", "zr-1", 0.0, 3200, 1000), area("A2", "zr-2", -10_000_000.0, 300, 3200), area("A3", "zr-3", -20_000_000.0, 1400, 400)];
    let mut gs = BTreeMap::from([
        ("A1".into(), (0..20).map(|i| grain(100.0 + 150.0 * i as f64, 100.0, 60.0)).chain([grain(200.0, 800.0, 60.0)]).collect::<Vec<_>>()),
        ("A2".into(), (0..20).map(|i| grain(100.0, 100.0 + 150.0 * i as f64, 60.0)).collect()),
        ("A3".into(), [100.0, 200.0, 300.0, 1000.0, 1100.0, 1200.0].into_iter().map(|x| grain(x, 200.0, 60.0)).collect()),
    ]);
    let mut id = 1;
    for (ai, rec) in recs.iter_mut().enumerate() {
        rec.uuid = uuid::Uuid::from_u128(100 + ai as u128);
        for (i, g) in gs.get_mut(&rec.id).unwrap().iter_mut().enumerate() {
            g.id = uuid::Uuid::from_u128(id);
            g.number = i as u32 + 1;
            id += 1;
        }
    }
    let p = SheetParams { paper: "letter".into(), context_um: 140.0, min_cluster_grains: 3, printed: "2000-01-01".into(), ..fast_sheets() };
    (p, recs, gs)
}

#[test]
fn sheets_golden_registration_and_svg() {
    use sha2::{Digest, Sha256};
    let (p, recs, gs) = golden_mount();
    let first = field_sheets(&p, &recs, &gs);
    let second = field_sheets(&p, &recs, &gs);
    let registration = serde_json::to_string(&first.registration).unwrap();
    let hashes: Vec<String> = first.pages.iter().map(|s| format!("{:x}", Sha256::digest(s.as_bytes()))).collect();
    assert_eq!(registration, serde_json::to_string(&second.registration).unwrap());
    assert_eq!(first.pages, second.pages);
    assert_eq!(first.stats, second.stats);
    assert!(first.pages.len() >= 2);
    assert_eq!(first.registration.strays.len(), 1);
    assert!(first.registration.bands.iter().any(|b| b.cut_sources.iter().any(|s| s == "width")));
    assert!(first.registration.bands.iter().any(|b| b.cut_sources.iter().any(|s| s.starts_with("height_"))));
    assert!(first.registration.bands.iter().any(|b| !b.repeated_um.is_empty()));
    assert!(first.pages.iter().any(|s| s.contains(">//</text>")));
    assert_eq!(serde_json::to_string(&first.stats).unwrap(), r#"{"sheets":4,"bands":8,"grains":46,"duplicates":0,"filtered":0,"artifacts":0,"stripped":0,"strip_refused":0,"isolated":0,"cuts":4,"forced_cuts":0,"worst_repeated_fraction":0.13810101010100945,"clusters":4,"strays":1,"fill":{"band_area":0.07053367605171118,"line_width":0.32539230574538097,"repeated":0.07718453476844639,"empty_epoxy":0.14133975357598075},"header_mm":[50.0,36.0],"height_budget":[{"sheet":1,"header_mm2":13169.999999999998,"footer_margins_mm2":9249.200000000003,"labels_notes_mm2":6953.76,"band_mm2":6210.75,"empty_epoxy_mm2":344.5,"blank_below_mm":2.842170943040401e-14,"blank_right_mm2":24738.75,"other_line_mm2":0.0},{"sheet":2,"header_mm2":9482.4,"footer_margins_mm2":9249.200000000003,"labels_notes_mm2":2317.92,"band_mm2":1859.0,"empty_epoxy_mm2":529.75,"blank_below_mm":6.100000000000023,"blank_right_mm2":35807.2,"other_line_mm2":0.0},{"sheet":3,"header_mm2":9482.4,"footer_margins_mm2":9249.200000000003,"labels_notes_mm2":2317.92,"band_mm2":1612.0,"empty_epoxy_mm2":523.25,"blank_below_mm":25.100000000000023,"blank_right_mm2":31049.6,"other_line_mm2":0.0},{"sheet":4,"header_mm2":9482.4,"footer_margins_mm2":9249.200000000003,"labels_notes_mm2":2317.92,"band_mm2":1792.375,"empty_epoxy_mm2":224.25,"blank_below_mm":87.22500000000001,"blank_right_mm2":7647.9974999999995,"other_line_mm2":6857.502499999999}],"bands_outside_body":0,"grains_cut":0,"grains_twice":0,"grains_unmarked":0}"#);
    // Recorded before splitting the layout stages; compare bytes, including field order.
    assert_eq!(registration, r#"{"project_uuid":null,"project_path":"","frame":"print_sheet","note":"paper sheet <-> grainatlas mosaic; not the laser stage registration","package":"","project":"","paper":"letter landscape 279.4 x 215.9 mm","paper_mm":[279.4,215.9],"um_per_mm":8.0,"scale":"fixed","sheet_origin":"sheet top-left corner, mm, y down","mapping":"mosaic_um = mosaic_rect_um[0:2] + (sheet_mm - sheet_rect_mm[0:2]) * um_per_mm","registration_file":"_print_registration.json","body_rect_mm":[8.0,45.0,271.4,202.9],"sheet_bodies":[{"sheet":1,"header_mm":50.0,"body_rect_mm":[8.0,59.0,271.4,202.9]},{"sheet":2,"header_mm":36.0,"body_rect_mm":[8.0,45.0,271.4,202.9]},{"sheet":3,"header_mm":36.0,"body_rect_mm":[8.0,45.0,271.4,202.9]},{"sheet":4,"header_mm":36.0,"body_rect_mm":[8.0,45.0,271.4,202.9]}],"strays":[{"area":"A1","id":"00000000-0000-0000-0000-000000000015","number":21,"eq_diameter_um":67.70275002573075,"centroid_um":[200.0,800.0],"bbox_um":[170.0,770.0,230.0,830.0]}],"bands":[{"sheet":1,"sample":"zr-1","area":"A1","row":1,"grains":[{"id":"00000000-0000-0000-0000-00000000000d","number":13},{"id":"00000000-0000-0000-0000-00000000000c","number":12},{"id":"00000000-0000-0000-0000-00000000000b","number":11},{"id":"00000000-0000-0000-0000-00000000000a","number":10},{"id":"00000000-0000-0000-0000-000000000009","number":9},{"id":"00000000-0000-0000-0000-000000000008","number":8},{"id":"00000000-0000-0000-0000-000000000007","number":7},{"id":"00000000-0000-0000-0000-000000000006","number":6},{"id":"00000000-0000-0000-0000-000000000005","number":5},{"id":"00000000-0000-0000-0000-000000000004","number":4},{"id":"00000000-0000-0000-0000-000000000003","number":3},{"id":"00000000-0000-0000-0000-000000000002","number":2},{"id":"00000000-0000-0000-0000-000000000001","number":1}],"context":[{"id":"00000000-0000-0000-0000-00000000000e","number":14}],"repeated_um":[[1982.2399999999998,48.0,2050.0,152.0]],"cut_sources":["width"],"sheet_rect_mm":[8.0,63.6,258.25,76.6],"mosaic_rect_um":[48.0,48.0,2050.0,152.0],"background":"kept"},{"sheet":1,"sample":"zr-1","area":"A1","row":1,"grains":[{"id":"00000000-0000-0000-0000-000000000014","number":20},{"id":"00000000-0000-0000-0000-000000000013","number":19},{"id":"00000000-0000-0000-0000-000000000012","number":18},{"id":"00000000-0000-0000-0000-000000000011","number":17},{"id":"00000000-0000-0000-0000-000000000010","number":16},{"id":"00000000-0000-0000-0000-00000000000f","number":15},{"id":"00000000-0000-0000-0000-00000000000e","number":14}],"context":[{"id":"00000000-0000-0000-0000-00000000000d","number":13}],"repeated_um":[[1914.0,48.0,1982.2399999999998,152.0]],"cut_sources":["width"],"sheet_rect_mm":[8.0,85.39999999999999,144.0,98.39999999999999],"mosaic_rect_um":[1914.0,48.0,3002.0,152.0],"background":"kept"},{"sheet":1,"sample":"zr-2","area":"A2","row":1,"grains":[{"id":"00000000-0000-0000-0000-00000000001a","number":5},{"id":"00000000-0000-0000-0000-000000000019","number":4},{"id":"00000000-0000-0000-0000-000000000018","number":3},{"id":"00000000-0000-0000-0000-000000000017","number":2},{"id":"00000000-0000-0000-0000-000000000016","number":1}],"context":[],"repeated_um":[[48.0,712.2972499742692,152.0,780.0]],"cut_sources":["height_sheet"],"sheet_rect_mm":[8.0,107.19999999999999,21.0,198.7],"mosaic_rect_um":[48.0,48.0,152.0,780.0],"background":"kept"},{"sheet":2,"sample":"zr-2","area":"A2","row":1,"grains":[{"id":"00000000-0000-0000-0000-000000000020","number":11},{"id":"00000000-0000-0000-0000-00000000001f","number":10},{"id":"00000000-0000-0000-0000-00000000001e","number":9},{"id":"00000000-0000-0000-0000-00000000001d","number":8},{"id":"00000000-0000-0000-0000-00000000001c","number":7},{"id":"00000000-0000-0000-0000-00000000001b","number":6}],"context":[{"id":"00000000-0000-0000-0000-00000000001a","number":5},{"id":"00000000-0000-0000-0000-000000000021","number":12}],"repeated_um":[[48.0,644.0,152.0,712.2972499742692],[48.0,1720.0,152.0,1788.0]],"cut_sources":["height_sheet","height_line"],"sheet_rect_mm":[8.0,49.6,21.0,192.6],"mosaic_rect_um":[48.0,644.0,152.0,1788.0],"background":"kept"},{"sheet":3,"sample":"zr-2","area":"A2","row":1,"grains":[{"id":"00000000-0000-0000-0000-000000000026","number":17},{"id":"00000000-0000-0000-0000-000000000025","number":16},{"id":"00000000-0000-0000-0000-000000000024","number":15},{"id":"00000000-0000-0000-0000-000000000023","number":14},{"id":"00000000-0000-0000-0000-000000000022","number":13},{"id":"00000000-0000-0000-0000-000000000021","number":12}],"context":[{"id":"00000000-0000-0000-0000-000000000027","number":18}],"repeated_um":[[48.0,1652.0,152.0,1720.0],[48.0,2575.3599999999997,152.0,2644.0]],"cut_sources":["height_sheet","height_line","height_line"],"sheet_rect_mm":[8.0,49.6,21.0,173.6],"mosaic_rect_um":[48.0,1652.0,152.0,2644.0],"background":"kept"},{"sheet":4,"sample":"zr-2","area":"A2","row":1,"grains":[{"id":"00000000-0000-0000-0000-000000000029","number":20},{"id":"00000000-0000-0000-0000-000000000028","number":19},{"id":"00000000-0000-0000-0000-000000000027","number":18}],"context":[{"id":"00000000-0000-0000-0000-000000000026","number":17}],"repeated_um":[[48.0,2507.0,152.0,2575.3599999999997]],"cut_sources":["height_sheet","height_line","height_line"],"sheet_rect_mm":[8.0,49.6,21.0,111.475],"mosaic_rect_um":[48.0,2507.0,152.0,3002.0],"background":"kept"},{"sheet":4,"sample":"zr-3","area":"A3","row":1,"grains":[{"id":"00000000-0000-0000-0000-00000000002c","number":3},{"id":"00000000-0000-0000-0000-00000000002a","number":1},{"id":"00000000-0000-0000-0000-00000000002b","number":2}],"context":[],"repeated_um":[],"cut_sources":[],"sheet_rect_mm":[59.897999999999996,49.6,97.898,62.6],"mosaic_rect_um":[48.0,148.0,352.0,252.0],"background":"kept"},{"sheet":4,"sample":"zr-3","area":"A3","row":2,"grains":[{"id":"00000000-0000-0000-0000-00000000002f","number":6},{"id":"00000000-0000-0000-0000-00000000002d","number":4},{"id":"00000000-0000-0000-0000-00000000002e","number":5}],"context":[],"repeated_um":[],"cut_sources":[],"sheet_rect_mm":[109.79599999999999,49.6,147.796,62.6],"mosaic_rect_um":[948.0,148.0,1252.0,252.0],"background":"kept"}]}"#);
    assert_eq!(hashes, ["4709fa5fafd2d0a0609bbc180feac08612a5f710561c4646b13a1ff3ee870feb", "27dc76bfa4814f2157fa113075b0ac97eadff8e0064a827d07ad585b765f0e48", "c567203ae53d3dd78bd9b66de11fe22fd0a687ad401bf034b92b787872455da4", "08123164e01db35a5d2f8430ca0f68f8375807b7018762a112643f838c0f9389"]);
}

#[test]
fn sheet_plan_round_trips_and_renders_at_strip_origins() {
    let (p, recs, gs) = golden_mount();
    let pixels: Vec<Vec<u8>> = recs.iter().enumerate().map(|(ai, r)| {
        let m = r.placement.as_ref().unwrap();
        (0..m.width * m.height).map(|i| (i % 173 + ai * 30) as u8).collect()
    }).collect();
    let areas: Vec<_> = recs.iter().zip(&pixels).map(|(rec, px)| {
        let m = rec.placement.as_ref().unwrap();
        SheetArea { rec, img: AreaImage { id: &rec.id, name: &rec.name, sample: &rec.sample, w: m.width, h: m.height, px, lut: std::array::from_fn(|i| i as u8), um_per_px: 1.0 } }
    }).collect();
    let own = ownership::assign(&recs, &gs, &OwnershipParams::default());
    let plan = sheets::plan_sheets(&p, &areas, &gs, &own, &SheetMeta::default());
    let encoded = serde_json::to_string(&plan).unwrap();
    let mut decoded: sheets::SheetPlan = serde_json::from_str(&encoded).unwrap();
    assert_eq!(plan, decoded);
    let original = sheets::sheets(&p, &areas, &gs, &own, &SheetMeta::default());
    assert_eq!(original, sheets::render_plan(&decoded, &areas, &gs, &own));
    let reversed = gs.iter().map(|(area, gs)| (area.clone(), gs.iter().rev().cloned().collect())).collect();
    let reordered = sheets::plan_sheets(&p, &areas, &reversed, &own, &SheetMeta::default());
    let keys = |p: &sheets::SheetPlan| p.strips.iter().map(|r| r.key.clone()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(keys(&plan), keys(&reordered));
    assert_eq!(keys(&plan).len(), plan.strips.len());
    assert!(plan.strips.iter().flat_map(|r| &r.bands).all(|b| b.key.split == 0));

    // This exercises the rendering boundary, without adding an edit operation or packer.
    decoded.lines = None;
    decoded.strips[0].sheet = 3;
    decoded.strips[0].origin_mm = [8.0, 140.0];
    let moved = sheets::render_plan(&decoded, &areas, &gs, &own);
    let id = original.registration.bands[0].grains[0].id;
    let band = moved.registration.bands.iter().find(|b| b.grains.iter().any(|g| g.id == id)).unwrap();
    assert_eq!(band.sheet, 4);
    assert_eq!(band.sheet_rect_mm, [8.0, 144.6, 258.25, 157.6]);
    assert_eq!(band.mosaic_rect_um, original.registration.bands[0].mosaic_rect_um);
    assert_eq!(band.grains, original.registration.bands[0].grains);
    assert_eq!(band.context, original.registration.bands[0].context);
    assert_eq!(band.repeated_um, original.registration.bands[0].repeated_um);
    assert_eq!((moved.stats.grains_cut, moved.stats.grains_twice, moved.stats.grains_unmarked, moved.stats.bands_outside_body), (0, 0, 0, 0));
    assert!(moved.pages.iter().all(|s| !s.contains("stroke-dasharray=\"0.6 1.2\"")));
    let images = |r: &sheets::SheetResult| {
        let mut images = BTreeMap::new();
        for (pi, svg) in r.pages.iter().enumerate() {
            let doc = roxmltree::Document::parse(svg).unwrap();
            let pngs = doc.descendants().filter(|n| n.has_tag_name("image"));
            for (b, image) in r.registration.bands.iter().filter(|b| b.sheet == pi + 1).zip(pngs) {
                images.insert(b.grains[0].id, image.attribute(("http://www.w3.org/1999/xlink", "href")).unwrap().to_string());
            }
        }
        images
    };
    assert_eq!(images(&original), images(&moved));
}

/// Ignore placement and compare every other band field as a multiset.
fn same_cut(a: &sheets::SheetResult, b: &sheets::SheetResult) {
    let bands = |r: &sheets::SheetResult| {
        let mut bands: Vec<_> = r.registration.bands.iter().cloned().map(|mut b| {
            b.sheet = 0;
            b.sheet_rect_mm = [0.0; 4];
            serde_json::to_string(&b).unwrap()
        }).collect();
        bands.sort();
        bands
    };
    assert_eq!(bands(a), bands(b));
    assert_eq!(a.registration.strays, b.registration.strays);
    assert_eq!((a.stats.grains, a.stats.cuts, a.stats.forced_cuts, a.stats.clusters, a.stats.isolated), (b.stats.grains, b.stats.cuts, b.stats.forced_cuts, b.stats.clusters, b.stats.isolated));
}

fn assert_packed_blocks(r: &sheets::SheetResult) {
    assert_eq!(r.registration.layout, "packed");
    assert_eq!((r.stats.bands_overlapping, r.stats.bands_outside_body), (0, 0));
    assert_eq!(r.stats.fill.line_width, 0.0);
    assert!(!r.registration.strip_blocks.is_empty());
    for (i, block) in r.registration.strip_blocks.iter().enumerate() {
        let body = &r.registration.sheet_bodies[block.sheet - 1];
        assert!(holds(body.body_rect_mm, block.rect_mm));
        let band_height = r.registration.bands.iter().filter(|b| b.sheet == block.sheet && (0..2).all(|d| block.rect_mm[d] <= b.sheet_rect_mm[d] + 1e-7 && block.rect_mm[d + 2] >= b.sheet_rect_mm[d + 2] - 1e-7)).map(|b| b.sheet_rect_mm[3] - b.sheet_rect_mm[1]).fold(0.0_f64, f64::max);
        assert!((block.rect_mm[3] - block.rect_mm[1] - 4.6 - band_height).abs() < 1e-7);
        for other in r.registration.strip_blocks.iter().skip(i + 1).filter(|b| b.sheet == block.sheet) {
            assert!(rect_overlap(block.rect_mm, other.rect_mm).is_none(), "blocks include labels: {block:?}, {other:?}");
            let [a, b] = [block.rect_mm, other.rect_mm];
            assert!(a[2] + 2.5 <= b[0] + 1e-7 || b[2] + 2.5 <= a[0] + 1e-7 || a[3] + 2.5 <= b[1] + 1e-7 || b[3] + 2.5 <= a[1] + 1e-7);
        }
    }
    for band in &r.registration.bands {
        let body = &r.registration.sheet_bodies[band.sheet - 1];
        assert!(holds(body.body_rect_mm, band.sheet_rect_mm));
        for d in 0..2 {
            let um = band.mosaic_rect_um[d + 2] - band.mosaic_rect_um[d];
            assert!(((band.sheet_rect_mm[d + 2] - band.sheet_rect_mm[d]) * r.registration.um_per_mm - um).abs() < 1e-6 * um);
        }
    }
    for (body, budget) in r.registration.sheet_bodies.iter().zip(&r.stats.height_budget) {
        assert_eq!(body.header_mm, r.stats.header_mm[usize::from(body.sheet > 1)]);
        let margin = body.body_rect_mm[0];
        assert_eq!(body.body_rect_mm, [margin, margin + 1.0 + body.header_mm, r.registration.paper_mm[0] - margin, r.registration.paper_mm[1] - margin - 5.0]);
        let block_area: f64 = r.registration.strip_blocks.iter().filter(|b| b.sheet == body.sheet).map(|b| (b.rect_mm[2] - b.rect_mm[0]) * (b.rect_mm[3] - b.rect_mm[1])).sum();
        let body_area = (body.body_rect_mm[2] - body.body_rect_mm[0]) * (body.body_rect_mm[3] - body.body_rect_mm[1]);
        assert!((budget.blank_mm2 - (body_area - block_area)).abs() < 1e-7);
        let [w, h] = r.registration.paper_mm;
        assert!((budget.header_mm2 + budget.footer_margins_mm2 + budget.labels_notes_mm2 + budget.band_mm2 + budget.other_line_mm2 + budget.blank_mm2 - w * h).abs() < 1e-7);
    }
    for page in &r.pages {
        assert!(!page.contains("stroke-dasharray=\"0.6 1.2\""));
    }
}

#[test]
fn packed_preserves_the_cut_mapping_and_bytes_across_runs() {
    let (p, recs, gs) = golden_mount();
    let lines = field_sheets(&p, &recs, &gs);
    assert!(lines.stats.sheets >= 3);
    let p = SheetParams { packing: Packing::Packed, ..p };
    let packed = field_sheets(&p, &recs, &gs);
    same_cut(&lines, &packed);
    assert!(packed.stats.sheets <= lines.stats.sheets);
    assert!(packed.stats.sheets <= 2, "the pre-compaction golden mount used 2 packed sheets");
    assert_eq!(packed.stats.header_mm, [40.0, 25.0]);
    assert_compact_header_boxes(&packed, p.margin_mm);
    assert_packed_blocks(&packed);
    let second = field_sheets(&p, &recs, &gs);
    assert_eq!(serde_json::to_string(&packed.registration).unwrap(), serde_json::to_string(&second.registration).unwrap());
    assert_eq!(packed.pages, second.pages);
    assert_eq!(packed.stats, second.stats);
    assert!(packed.pages.iter().any(|p| p.contains(">//</text>")));
    assert!(packed.pages.iter().any(|p| p.contains("repeated-hatch")));
}

#[test]
fn packed_fills_space_below_short_two_thirds_width_bands() {
    // Lines pair a short wide band with a tall narrow band. Packing can stack
    // short bands beside tall ones without changing a single mosaic crop.
    let p = SheetParams { paper: "letter".into(), context_um: 2200.0, printed: "2000-01-01".into(), ..fast_sheets() };
    let mut recs = Vec::new();
    let mut gs = BTreeMap::new();
    for i in 0..16 {
        let (w, h) = if i % 2 == 0 { (1400, 200) } else { (520, 800) };
        let rec = area(&format!("A{i}"), &format!("zr-{i}"), -(i as f64) * 10_000_000.0, w + 100, h + 100);
        let grains = [100.0, w as f64 - 50.0].into_iter().flat_map(|x| [100.0, h as f64 - 50.0].into_iter().map(move |y| grain(x, y, 60.0))).collect();
        gs.insert(rec.id.clone(), grains);
        recs.push(rec);
    }
    let lines = field_sheets(&p, &recs, &gs);
    assert!(lines.stats.sheets >= 3);
    let packed = field_sheets(&SheetParams { packing: Packing::Packed, ..p }, &recs, &gs);
    same_cut(&lines, &packed);
    assert!(packed.stats.sheets < lines.stats.sheets, "{} packed, {} lines", packed.stats.sheets, lines.stats.sheets);
    assert_packed_blocks(&packed);
}

#[test]
fn packed_reserves_pinned_rectangles_and_uses_body_edges_without_a_gap() {
    use sheets::{SheetBlock, packed::pack_rectangles};
    let bodies = [[8.0, 59.0, 208.0, 159.0], [8.0, 45.0, 208.0, 159.0]];
    let pinned = [SheetBlock { sheet: 1, rect_mm: [38.0, 79.0, 108.0, 119.0] }, SheetBlock { sheet: 2, rect_mm: [8.0, 45.0, 208.0, 60.0] }];
    let sizes = [[25.0, 30.0]; 30];
    let placed = pack_rectangles(&sizes, bodies, &pinned);
    assert_eq!(placed.len(), sizes.len());
    assert!(placed.iter().any(|b| b.sheet == 1));
    assert_eq!(placed, pack_rectangles(&sizes, bodies, &pinned));
    for (i, block) in placed.iter().enumerate() {
        assert!(holds(bodies[usize::from(block.sheet > 1)], block.rect_mm));
        for other in pinned.iter().chain(placed.iter().skip(i + 1)).filter(|b| b.sheet == block.sheet) {
            let [a, b] = [block.rect_mm, other.rect_mm];
            assert!(a[2] + 2.5 <= b[0] + 1e-7 || b[2] + 2.5 <= a[0] + 1e-7 || a[3] + 2.5 <= b[1] + 1e-7 || b[3] + 2.5 <= a[1] + 1e-7);
        }
    }
    let exact = pack_rectangles(&[[200.0, 100.0], [200.0, 114.0]], bodies, &[]);
    assert_eq!(exact, [SheetBlock { sheet: 1, rect_mm: bodies[0] }, SheetBlock { sheet: 2, rect_mm: bodies[1] }]);
    let oversize = pack_rectangles(&[[201.0, 101.0], [10.0, 10.0]], bodies, &[]);
    assert_ne!(oversize[0].sheet, oversize[1].sheet, "oversized strips get a sheet of their own");
}

#[test]
fn packing_json_defaults_and_unknown_modes_are_clear() {
    let old: SheetParams = serde_json::from_str(r#"{"paper":"letter","um_per_mm":8}"#).unwrap();
    assert_eq!(old.packing, Packing::Lines);
    let packed: SheetParams = serde_json::from_str(r#"{"packing":"packed"}"#).unwrap();
    assert_eq!(packed.packing, Packing::Packed);
    let error = serde_json::from_str::<SheetParams>(r#"{"packing":"other"}"#).unwrap_err().to_string();
    assert!(error.contains("expected `lines` or `packed`"), "{error}");
    let mut old_budget = serde_json::to_value(sheets::HeightBudget::default()).unwrap();
    old_budget.as_object_mut().unwrap().remove("blank_mm2");
    let budget: sheets::HeightBudget = serde_json::from_value(old_budget).unwrap();
    assert_eq!(budget.blank_mm2, 0.0);
    let stats: sheets::SheetStats = serde_json::from_str("{}").unwrap();
    assert_eq!(stats.bands_overlapping, 0);
}

#[test]
fn registration_audit_detects_image_and_label_block_overlaps() {
    let (p, recs, gs) = golden_mount();
    let r = field_sheets(&SheetParams { packing: Packing::Packed, ..p }, &recs, &gs);
    // Geometry auditing also works without source pixels, targets or area records.
    let mut reg: sheets::PrintRegistration = serde_json::from_str(&serde_json::to_string(&r.registration).unwrap()).unwrap();
    assert_eq!(sheets::audit(&reg, &[], &[]).bands_overlapping, 0);
    reg.strip_blocks.truncate(2);
    reg.strip_blocks[1] = reg.strip_blocks[0].clone();
    assert_eq!(sheets::audit(&reg, &[], &[]).bands_overlapping, 1, "block collision even though the images are disjoint");
    reg.strip_blocks.clear();
    reg.bands.truncate(2);
    reg.bands[1].sheet = reg.bands[0].sheet;
    reg.bands[1].sheet_rect_mm = reg.bands[0].sheet_rect_mm;
    assert_eq!(sheets::audit(&reg, &[], &[]).bands_overlapping, 1, "image auditing does not need strip records");
    let edge = reg.bands[0].sheet_rect_mm[2];
    reg.bands[1].sheet_rect_mm[0] = edge - 1e-8;
    reg.bands[1].sheet_rect_mm[2] = edge + 10.0;
    assert_eq!(sheets::audit(&reg, &[], &[]).bands_overlapping, 0, "ignore floating point edge contact");
}

fn field_plan(p: &SheetParams, recs: &[AreaRecord], gs: &BTreeMap<String, Vec<Grain>>) -> sheets::SheetPlan {
    // Planning is pixel-free: only the source image dimensions and resolution matter.
    let areas: Vec<_> = recs.iter().map(|rec| {
        let m = rec.placement.as_ref().unwrap();
        SheetArea { rec, img: AreaImage { id: &rec.id, name: &rec.name, sample: &rec.sample, w: m.width, h: m.height, px: &[], lut: [128; 256], um_per_px: 1.0 } }
    }).collect();
    let own = ownership::assign(recs, gs, &OwnershipParams::default());
    sheets::plan_sheets(p, &areas, gs, &own, &SheetMeta::default())
}

#[test]
fn packed_block_sizes_cover_labels_underlines_and_gap_marks() {
    let mut rec = area("A1", "zr-1", 0.0, 2400, 300);
    rec.sample = "zr-12345678901234567890".into();
    let gs = BTreeMap::from([(rec.id.clone(), vec![grain(100.0, 100.0, 60.0), grain(2000.0, 100.0, 60.0)])]);
    let p = SheetParams { context_um: 100.0, ..fast_sheets() };
    let lines = field_plan(&p, std::slice::from_ref(&rec), &gs);
    let packed = field_plan(&SheetParams { packing: Packing::Packed, ..p.clone() }, std::slice::from_ref(&rec), &gs);
    assert_eq!(lines.strips.len(), 2);
    assert!(lines.strips[0].bands.last().unwrap().gap_after_um.is_some());
    assert!(!lines.strips[1].labelled);
    assert_eq!(lines.targets, packed.targets);
    for (line, block) in lines.strips.iter().zip(&packed.strips) {
        assert_eq!(line.key, block.key);
        assert!((line.size_mm[1] - block.size_mm[1] - 4.2).abs() < 1e-7);
        assert_eq!(line.bands[0].mosaic_rect_um, block.bands[0].mosaic_rect_um);
        assert!(block.labelled);
        assert_eq!(line.bands[0].offset_mm, block.bands[0].offset_mm);
        let band = &block.bands[0];
        let image_right = band.offset_mm + (band.mosaic_rect_um[2] - band.mosaic_rect_um[0]) / packed.um_per_mm;
        assert!(block.size_mm[0] >= image_right);
        let drawn = block.sample.chars().count() as f64 * 3.2 * 0.66 + 6.0 + block.label.chars().count() as f64 * 2.5 * 0.58;
        assert!(block.size_mm[0] >= drawn.max(20.0));
        if let Some(gap) = band.gap_after_um {
            let drawn = format!("{:.1} mm", gap / 1000.0).chars().count() as f64 * 1.7 * 0.58;
            assert!(block.size_mm[0] >= image_right + 2.5 + drawn / 2.0);
        }
    }
    let json = serde_json::to_string(&packed).unwrap();
    assert_eq!(packed, serde_json::from_str::<sheets::SheetPlan>(&json).unwrap());
    assert_packed_blocks(&field_sheets(&SheetParams { packing: Packing::Packed, ..p }, &[rec], &gs));
}

#[test]
fn packed_every_bands_strip_draws_its_sample_and_area_labels() {
    let mut rec = area("A1", "zr-1", 0.0, 2400, 300);
    rec.name = "A1".into();
    let recs = [rec];
    let gs = BTreeMap::from([(recs[0].id.clone(), vec![grain(100.0, 100.0, 60.0), grain(2000.0, 100.0, 60.0)])]);
    let p = SheetParams { context_um: 100.0, ..fast_sheets() };
    let lines = field_plan(&p, &recs, &gs);
    let line = lines.lines.as_ref().unwrap().iter().find(|l| l.strips.iter().any(|&i| !lines.strips[i].labelled)).unwrap();
    assert!(line.strips.iter().any(|&i| lines.strips[i].labelled));
    let p = SheetParams { packing: Packing::Packed, ..p };
    let plan = field_plan(&p, &recs, &gs);
    let result = field_sheets(&p, &recs, &gs);
    let mut checked = 0;
    for band in &result.registration.bands {
        let strip = plan.strips.iter().find(|s| s.area == band.area && s.bands.iter().any(|b| b.grains.contains(&band.grains[0].id))).unwrap();
        assert!(strip.labelled);
        assert_eq!(strip.sheet + 1, band.sheet);
        let doc = roxmltree::Document::parse(&result.pages[band.sheet - 1]).unwrap();
        let texts: Vec<_> = doc.descendants().filter(|n| n.has_tag_name("text") && n.attribute("y").is_some_and(|y| (y.parse::<f64>().unwrap() - (strip.origin_mm[1] + 3.1)).abs() < 0.01)).collect();
        assert!(texts.iter().any(|n| n.text() == Some(strip.sample.as_str()) && n.attribute("x").is_some_and(|x| (x.parse::<f64>().unwrap() - (strip.origin_mm[0] + 3.6)).abs() < 0.01)), "sample label for band {band:?}");
        let expected = if strip.cont { "area A1 · continued" } else { "area A1" };
        assert!(texts.iter().any(|n| n.text() == Some(expected)), "area label for band {band:?}");
        checked += 1;
    }
    assert_eq!(checked, 2);
}

#[test]
fn packed_oversized_images_and_labels_keep_their_scale_and_fail_bounds_audit() {
    let px = vec![128; 64 * 64];
    for label_only in [false, true] {
        let mut rec = area("A1", "zr-1", 0.0, 6400, 6400);
        if label_only { rec.sample = format!("zr-{}", "a".repeat(220)); }
        let g = if label_only { grain(1000.0, 1000.0, 60.0) } else { grain(3200.0, 3200.0, 4000.0) };
        let gs = BTreeMap::from([(rec.id.clone(), vec![g.clone()])]);
        let own = ownership::assign(std::slice::from_ref(&rec), &gs, &OwnershipParams::default());
        let areas = [SheetArea { rec: &rec, img: AreaImage { id: &rec.id, name: &rec.name, sample: &rec.sample, w: 64, h: 64, px: &px, lut: [128; 256], um_per_px: 100.0 } }];
        let p = SheetParams { packing: Packing::Packed, ..fast_sheets() };
        let result = sheets::sheets(&p, &areas, &gs, &own, &SheetMeta::default());
        assert_eq!((result.stats.sheets, result.stats.bands, result.stats.grains), (1, 1, 1));
        assert!(result.stats.bands_outside_body > 0);
        assert_eq!((result.stats.grains_cut, result.stats.bands_overlapping), (0, 0));
        let b = &result.registration.bands[0];
        assert!(holds(b.mosaic_rect_um, g.bbox_um));
        for d in 0..2 {
            assert!(((b.sheet_rect_mm[d + 2] - b.sheet_rect_mm[d]) * p.um_per_mm - (b.mosaic_rect_um[d + 2] - b.mosaic_rect_um[d])).abs() < 1e-6);
        }
    }
}
/// Read rendered SVG, including text estimates and the circles at print scale.
fn assert_compact_header_boxes(r: &sheets::SheetResult, margin: f64) {
    for (page, body) in r.pages.iter().zip(&r.registration.sheet_bodies) {
        let header = [margin, margin, r.registration.paper_mm[0] - margin, margin + body.header_mm];
        assert_eq!(body.body_rect_mm[1], header[3] + 1.0);
        let doc = roxmltree::Document::parse(page).unwrap();
        let mut text_boxes = Vec::new();
        let mut circles = Vec::new();
        let mut map = None;
        for group in doc.descendants().filter(|n| n.has_tag_name("g") && n.attribute("data-header").is_some()) {
            let in_map = group.attribute("data-header") == Some("map");
            for node in group.children().filter(|n| n.is_element()) {
                let value = |key| node.attribute(key).unwrap().parse::<f64>().unwrap();
                let rect = match node.tag_name().name() {
                    "text" => {
                        let size = value("font-size");
                        assert!(size >= 1.8);
                        let width = node.text().unwrap_or("").chars().count() as f64 * size * if node.attribute("font-weight") == Some("bold") { 0.66 } else { 0.58 };
                        let rect = [value("x"), value("y") - size, value("x") + width, value("y") + size * 0.22];
                        text_boxes.push((rect, in_map, node.text().unwrap_or("")));
                        rect
                    }
                    "circle" => {
                        let (x, y, radius) = (value("cx"), value("cy"), value("r"));
                        let rect = [x - radius, y - radius, x + radius, y + radius];
                        circles.push(rect);
                        rect
                    }
                    "rect" => {
                        let rect = [value("x"), value("y"), value("x") + value("width"), value("y") + value("height")];
                        if node.attribute("rx") == Some("1.5") { map = Some(rect); }
                        rect
                    }
                    _ => continue,
                };
                // SVG coordinates round to at most 0.001 mm (map border to 0.01 mm).
                assert!((0..2).all(|d| rect[d] >= header[d] - 0.011 && rect[d + 2] <= header[d + 2] + 0.011), "outside header: {rect:?}, {header:?}, {node:?}");
            }
        }
        assert!(map.is_some());
        for (i, (rect, in_map, text)) in text_boxes.iter().enumerate() {
            for (other, _, other_text) in &text_boxes[i + 1..] {
                assert!(rect_overlap(*rect, *other).is_none(), "header text overlaps: {text:?} {rect:?}, {other_text:?} {other:?}");
            }
            if !in_map { assert!(rect_overlap(*rect, map.unwrap()).is_none(), "text overlaps map: {text:?}"); }
            for circle in &circles { assert!(rect_overlap(*rect, *circle).is_none(), "text overlaps key: {text:?}"); }
        }
        for circle in circles { assert!(rect_overlap(circle, map.unwrap()).is_none()); }
    }
}

#[test]
fn compact_headers_bound_many_long_labels_and_preserve_acquisition_values() {
    let mut recs = Vec::new();
    let mut gs = BTreeMap::new();
    for i in 0..24 {
        let rec = area(&format!("A{i}"), &format!("Synthetic sample {i} with a long label"), -(i as f64) * 1_000_000.0, 400, 300);
        gs.insert(rec.id.clone(), vec![grain(100.0, 100.0, 60.0)]);
        recs.push(rec);
    }
    for paper in ["letter", "tabloid"] {
        let params = SheetParams { packing: Packing::Packed, paper: paper.into(), title: "Synthetic mount title with enough words to need abbreviation".into(), ..fast_sheets() };
        let areas: Vec<_> = recs.iter().map(|rec| SheetArea { rec, img: AreaImage { id: &rec.id, name: &rec.name, sample: &rec.sample, w: 400, h: 300, px: &[], lut: [128; 256], um_per_px: 1.0 } }).collect();
        let meta = SheetMeta { instrument: "Synthetic instrument".into(), serial: "0000".into(), detector: "Synthetic detector".into(), kv: Some(15.0), emission_ua: Some(42.0), wd_mm: Some(10.0), scan: "Synthetic scan with detailed settings preserved across the compact grid column".into(), magnification: "100".into(), tile_px: Some([400, 300]), package: "Synthetic package".into(), comment: "Acquisition text stays complete even when it needs another line".into(), ..Default::default() };
        let own = ownership::assign(&recs, &gs, &OwnershipParams::default());
        let mut plan = sheets::plan_sheets(&params, &areas, &gs, &own, &meta);
        let edited = sheets::apply_edits(&plan, &[sheets::StripEdit::AddSheet { at: plan.sheet_bodies.len() }], &areas, &gs);
        assert!(edited.problems.is_empty());
        plan = edited.plan;
        let result = sheets::render_plan_with_images(&plan, &areas, &gs, &own, &|_, _| (Vec::new(), "kept".into()));
        assert_compact_header_boxes(&result, params.margin_mm);
        assert!(result.pages[0].contains("more"));
        let doc = roxmltree::Document::parse(&result.pages[0]).unwrap();
        let text = doc.descendants().filter(|n| n.has_tag_name("text")).filter_map(|n| n.text()).collect::<Vec<_>>().join(" ");
        for value in ["Synthetic instrument", "S/N 0000", "Synthetic detector", "15 kV", "emission 42.0 µA", "WD 10.0 mm", "Synthetic scan", "PRINT", "PACKAGE", "Synthetic package", &meta.comment, &params.title, &meta.scan] {
            assert!(text.contains(value), "missing {value:?} from {text}");
        }
        assert_eq!(plan.sheet_bodies.last().unwrap().header_mm, sheets::PACKED_LATER_HEADER_MM);
        assert!(plan.sheet_bodies[0].header_mm <= 42.0);
        if paper == "letter" { assert!(plan.sheet_bodies[0].header_mm > 40.0, "wrapped scan value grows the first header"); }
    }
}

fn mixed_sample_field() -> (Vec<AreaRecord>, BTreeMap<String, Vec<Grain>>) {
    let rec = area("001_S1", "S1", 0.0, 1000, 1800);
    let mut gs = Vec::new();
    for (sample, y, count) in [(Some("S2"), 300.0, 5), (None, 900.0, 6), (Some("S3"), 1500.0, 4)] {
        for i in 0..count {
            let mut g = grain(150.0 + i as f64 * 100.0, y, 60.0);
            g.sample = sample.map(str::to_string);
            g.number = gs.len() as u32 + 1;
            gs.push(g);
        }
    }
    (vec![rec], BTreeMap::from([("001_S1".into(), gs)]))
}

fn sample_field_areas(recs: &[AreaRecord]) -> Vec<SheetArea<'_>> {
    recs.iter().map(|rec| {
        let m = rec.placement.as_ref().unwrap();
        SheetArea { rec, img: AreaImage { id: &rec.id, name: &rec.name, sample: &rec.sample, w: m.width, h: m.height, px: &[], lut: [128; 256], um_per_px: 1.0 } }
    }).collect()
}

fn render_sample_plan(plan: &sheets::SheetPlan, recs: &[AreaRecord], gs: &BTreeMap<String, Vec<Grain>>) -> sheets::SheetResult {
    let own = ownership::assign(recs, gs, &OwnershipParams::default());
    sheets::render_plan_with_images(plan, &sample_field_areas(recs), gs, &own, &|_, _| (Vec::new(), "kept".into()))
}

#[test]
fn samples_without_geometry_keep_area_order_even_when_the_first_area_has_no_grains() {
    let mut recs = vec![
        area("001_X", "X", 0.0, 600, 600),
        area("002_Y", "Y", -1000000.0, 600, 600),
        area("003_X", "X", -2000000.0, 600, 600),
    ];
    for rec in &mut recs {
        rec.tiles[0].stage_nm = None;
        assert!(ownership::AreaGeometry::new(rec).is_none());
    }
    let gs = BTreeMap::from([
        (recs[1].id.clone(), vec![grain(300.0, 300.0, 60.0)]),
        (recs[2].id.clone(), vec![grain(300.0, 300.0, 60.0)]),
    ]);
    for packing in [Packing::Lines, Packing::Packed] {
        let plan = field_plan(&SheetParams { packing, ..fast_sheets() }, &recs, &gs);
        assert_eq!(plan.strips.iter().map(|s| s.sample.as_str()).collect::<Vec<_>>(), ["X", "Y"]);
        assert_eq!(plan.cut_stats.grains, 2);
    }
}

#[test]
fn unchanged_map_samples_without_stage_boxes_have_no_label() {
    let mut recs = vec![area("001_X", "X", 0.0, 600, 600), area("002_Y", "Y", -1000000.0, 600, 600)];
    let mut tile = recs[0].tiles[0].clone();
    tile.stage_nm = None;
    recs[0].tiles.push(tile);
    let placement = recs[0].placement.as_mut().unwrap();
    let mut tile = placement.tiles[0].clone();
    tile.index = 1;
    placement.tiles.push(tile);
    assert!(ownership::AreaGeometry::new(&recs[0]).is_some());
    assert!(sheets::stage_box(&recs[0]).is_none());
    let gs = recs.iter().map(|rec| (rec.id.clone(), vec![grain(300.0, 300.0, 60.0)])).collect();
    for packing in [Packing::Lines, Packing::Packed] {
        let plan = field_plan(&SheetParams { packing, ..fast_sheets() }, &recs, &gs);
        let result = render_sample_plan(&plan, &recs, &gs);
        let doc = roxmltree::Document::parse(&result.pages[0]).unwrap();
        let labels: Vec<_> = doc.descendants().filter(|n| n.has_tag_name("text") && n.prev_sibling_element().is_some_and(|p| p.has_tag_name("path") && p.attribute("stroke-width") == Some("0.12"))).filter_map(|n| n.text()).collect();
        assert!(labels.contains(&"Y"));
        assert!(!labels.contains(&"X"));
    }
}

fn assert_sample_owners(plan: &sheets::SheetPlan, gs: &BTreeMap<String, Vec<Grain>>) {
    for s in &plan.strips {
        for id in s.bands.iter().flat_map(|b| &b.grains) {
            let g = gs[&s.area].iter().find(|g| g.id == *id).unwrap();
            assert_eq!(s.sample, g.sample.as_deref().filter(|s| !s.is_empty()).unwrap_or("S1"));
        }
    }
}

#[test]
fn mixed_area_strips_follow_grain_samples_in_mount_order() {
    let (recs, gs) = mixed_sample_field();
    for packing in [Packing::Lines, Packing::Packed] {
        let p = SheetParams { packing, min_cluster_grains: 5, ..fast_sheets() };
        let plan = field_plan(&p, &recs, &gs);
        assert_eq!(plan.strips.iter().map(|s| s.sample.as_str()).collect::<Vec<_>>(), ["S2", "S1", "S3"]);
        assert_eq!(plan.strips.iter().map(|s| s.row).collect::<std::collections::BTreeSet<_>>().len(), 3);
        assert_eq!(plan.strips.iter().map(|s| &s.key).collect::<std::collections::BTreeSet<_>>().len(), 3);
        assert!(plan.strips.iter().all(|s| s.labelled && s.bands.iter().all(|b| b.gap_after_um.is_none())));
        assert_sample_owners(&plan, &gs);
        let r = render_sample_plan(&plan, &recs, &gs);
        assert_eq!((r.stats.grains, r.stats.clusters, r.stats.strays), (15, 3, 0));
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        for strip in &plan.strips {
            let doc = roxmltree::Document::parse(&r.pages[strip.sheet]).unwrap();
            assert!(doc.descendants().any(|n| n.has_tag_name("text") && n.text() == Some(&strip.sample) && n.attribute("y").is_some_and(|y| (y.parse::<f64>().unwrap() - strip.origin_mm[1] - 3.1).abs() < 0.01)));
        }
    }
}

#[test]
fn mixed_sample_headers_count_grains_in_their_holding_area() {
    let (recs, gs) = mixed_sample_field();
    for packing in [Packing::Lines, Packing::Packed] {
        let p = SheetParams { packing, ..fast_sheets() };
        let r = render_sample_plan(&field_plan(&p, &recs, &gs), &recs, &gs);
        let doc = roxmltree::Document::parse(&r.pages[0]).unwrap();
        for (sample, count) in [("S2", 5), ("S1", 6), ("S3", 4)] {
            if packing == Packing::Packed {
                assert!(doc.descendants().filter_map(|n| n.text()).any(|t| t.contains(&format!("{sample} · {count} grains · area 001"))));
            } else {
                let label = doc.descendants().find(|n| n.has_tag_name("text") && n.text() == Some(sample) && n.attribute("font-size") == Some("2.7")).unwrap();
                assert_eq!(label.next_sibling_element().unwrap().text(), Some(format!("{count} grains · area 001").as_str()));
            }
        }
    }
}

#[test]
fn explicit_area_samples_and_empty_samples_preserve_svg_bytes_and_edits() {
    let (p, recs, gs) = golden_mount();
    let own = ownership::assign(&recs, &gs, &OwnershipParams::default());
    for packing in [Packing::Lines, Packing::Packed] {
        let p = SheetParams { packing, ..p.clone() };
        let original = field_plan(&p, &recs, &gs);
        let edits = [sheets::StripEdit::Move { strip: original.strips[0].key.clone(), sheet: 0, origin_mm: Some(original.strips[0].origin_mm) }];
        let baseline = render_sample_plan(&original, &recs, &gs).pages;
        let edited = sheets::apply_edits(&original, &edits, &sample_field_areas(&recs), &gs);
        let edited_svg = render_sample_plan(&edited.plan, &recs, &gs).pages;
        for empty in [false, true] {
            let mut explicit = gs.clone();
            for (ai, grains) in &mut explicit {
                for g in grains { g.sample = Some(if empty { "" } else { own.sample_of(ai).unwrap() }.into()); }
            }
            let plan = field_plan(&p, &recs, &explicit);
            assert_eq!(original, plan);
            assert_eq!(baseline, render_sample_plan(&plan, &recs, &explicit).pages);
            let replay = sheets::replay(&plan, &sheets::PrintEdits { params: p.clone(), um_per_mm: plan.um_per_mm, edits: edits.to_vec() }, &sample_field_areas(&recs), &explicit);
            assert_eq!(replay.applied, 1);
            assert_eq!(edited_svg, render_sample_plan(&replay.plan, &recs, &explicit).pages);
        }
    }
}

#[test]
fn mixed_samples_stay_separate_after_split_move_and_replay() {
    let (recs, gs) = mixed_sample_field();
    let p = SheetParams { packing: Packing::Packed, ..fast_sheets() };
    let plan = field_plan(&p, &recs, &gs);
    let strip = plan.strips.iter().find(|s| s.sample == "S2").unwrap();
    let edits = vec![
        sheets::StripEdit::Split { band: strip.bands[0].key.clone(), axis: sheets::edit::Axis::X, at_um: 300.0 },
        sheets::StripEdit::Move { strip: strip.key.clone(), sheet: 0, origin_mm: Some(strip.origin_mm) },
        sheets::StripEdit::Repack {},
    ];
    let replay = sheets::replay(&plan, &sheets::PrintEdits { params: p, um_per_mm: plan.um_per_mm, edits }, &sample_field_areas(&recs), &gs);
    assert_eq!(replay.applied, 3, "{:?}", replay.dropped);
    assert_sample_owners(&replay.plan, &gs);
    assert!(replay.left_off.is_empty());
    assert!(replay.problems.is_empty(), "{:?}", replay.problems);
    assert_eq!(replay.plan.strips.iter().find(|s| s.sample == "S2").unwrap().bands.len(), 2);
}

#[test]
fn hiding_a_sample_cannot_adopt_its_grains_into_another_samples_crop() {
    let (recs, gs) = mixed_sample_field();
    let plan = field_plan(&fast_sheets(), &recs, &gs);
    let s1 = plan.strips.iter().find(|s| s.sample == "S1").unwrap();
    let s2 = plan.strips.iter().find(|s| s.sample == "S2").unwrap();
    let edits = [
        sheets::StripEdit::Crop { band: s1.bands[0].key.clone(), mosaic_rect_um: [100.0, 250.0, 700.0, 950.0] },
        sheets::StripEdit::Hide { strip: s2.key.clone() },
    ];
    let edited = sheets::apply_edits(&plan, &edits, &sample_field_areas(&recs), &gs);
    assert_eq!(edited.applied, 2, "{:?}", edited.dropped);
    assert_sample_owners(&edited.plan, &gs);
    assert_eq!(edited.left_off.len(), 5);
    assert!(edited.left_off.iter().all(|g| g.reason == "hidden"));
    let refused = sheets::apply_edits(&plan, &[sheets::StripEdit::Join { band_a: s1.bands[0].key.clone(), band_b: s2.bands[0].key.clone() }], &sample_field_areas(&recs), &gs);
    assert_eq!(refused.applied, 0);
    assert_eq!(refused.plan, plan);
    assert!(refused.dropped[0].reason.contains("different areas or samples"));
}

#[test]
fn mixed_sample_map_uses_grain_anchors_and_dot_states() {
    let (recs, gs) = mixed_sample_field();
    for packing in [Packing::Lines, Packing::Packed] {
        let mut plan = field_plan(&SheetParams { packing, ..fast_sheets() }, &recs, &gs);
        // Keep all targets visible in the mount, but put S1 and S3 on a later sheet.
        let mut later = plan.sheet_bodies[0].clone();
        later.sheet = 2;
        plan.sheet_bodies.push(later);
        for s in &mut plan.strips { if s.sample != "S2" { s.sheet = 1; } }
        let r = render_sample_plan(&plan, &recs, &gs);
        let doc = roxmltree::Document::parse(&r.pages[0]).unwrap();
        let dots: Vec<_> = doc.descendants().filter(|n| n.has_tag_name("path") && n.attribute("stroke-linecap") == Some("round") && n.attribute("stroke-width") == Some("0.7")).collect();
        assert_eq!(dots.len(), 2);
        let count = |color| dots.iter().find(|n| n.attribute("stroke") == Some(color)).unwrap().attribute("d").unwrap().matches('M').count();
        assert_eq!(count("white"), 5);
        assert_eq!(count("#9a9a9a"), 10);
        // The S1 box is dark because it holds printed S2 targets.
        assert!(doc.descendants().any(|n| n.has_tag_name("rect") && n.attribute("fill") == Some("#3a3a3a")));
        // S3 has no area of its own. Its leader starts at its rightmost grain
        // and mean view height, rather than at the S1 area box.
        let page = if packing == Packing::Packed { 1 } else { 0 };
        let doc = roxmltree::Document::parse(&r.pages[page]).unwrap();
        let label = doc.descendants().find(|n| n.has_tag_name("text") && n.text() == Some("S3") && n.prev_sibling_element().is_some_and(|p| p.has_tag_name("path"))).unwrap();
        let leader = label.prev_sibling_element().unwrap().attribute("d").unwrap();
        let coords: Vec<f64> = leader.split_whitespace().take(2).map(|v| v.trim_start_matches('M').parse().unwrap()).collect();
        let rect = doc.descendants().find(|n| n.has_tag_name("rect") && matches!(n.attribute("fill"), Some("#3a3a3a" | "#ededed"))).unwrap();
        let v = |name| rect.attribute(name).unwrap().parse::<f64>().unwrap();
        assert!((coords[0] - (v("x") + 450.0 / 1000.0 * v("width") + 0.3)).abs() < 0.02);
        assert!((coords[1] - (v("y") + 1500.0 / 1800.0 * v("height"))).abs() < 0.02);
    }
}

#[test]
fn compact_header_lists_only_areas_with_bands_of_that_sample_on_the_sheet() {
    let (mut recs, mut gs) = mixed_sample_field();
    let mut other = area("002_S1", "S1", -3000000.0, 1000, 1800);
    other.name = "002_S1".into();
    let mut g = grain(300.0, 300.0, 60.0);
    g.sample = Some("S2".into());
    gs.insert(other.id.clone(), vec![g]);
    recs.push(other);
    let p = SheetParams { packing: Packing::Packed, ..fast_sheets() };
    let plan = field_plan(&p, &recs, &gs);
    let other_strip = plan.strips.iter().find(|s| s.area == "002_S1").unwrap();
    let edits = [sheets::StripEdit::AddSheet { at: 1 }, sheets::StripEdit::Move { strip: other_strip.key.clone(), sheet: 1, origin_mm: None }];
    let edited = sheets::apply_edits(&plan, &edits, &sample_field_areas(&recs), &gs);
    assert_eq!(edited.applied, 2, "{:?}", edited.dropped);
    let r = render_sample_plan(&edited.plan, &recs, &gs);
    let doc = roxmltree::Document::parse(&r.pages[0]).unwrap();
    let header = doc.descendants().find(|n| n.attribute("data-header") == Some("compact")).unwrap();
    let text = header.descendants().filter(|n| n.has_tag_name("text")).filter_map(|n| n.text()).collect::<Vec<_>>().join(" ");
    assert!(text.contains("S2 · 6 grains · area 001"), "{text}");
    assert!(!text.contains("002"), "{text}");
    let doc = roxmltree::Document::parse(&r.pages[1]).unwrap();
    let header = doc.descendants().find(|n| n.attribute("data-header") == Some("compact")).unwrap();
    let text = header.descendants().filter(|n| n.has_tag_name("text")).filter_map(|n| n.text()).collect::<Vec<_>>().join(" ");
    assert!(text.contains("S2 · 6 grains · area 002"), "{text}");
    assert!(!text.contains("area 001"), "{text}");
}

#[test]
fn mixed_sample_strays_are_classified_within_each_sample_unit() {
    let (recs, mut gs) = mixed_sample_field();
    let grains = gs.get_mut("001_S1").unwrap();
    let mut stray = grain(900.0, 1100.0, 20.0);
    stray.sample = Some("S2".into());
    let stray_id = stray.id;
    grains.push(stray);
    let p = SheetParams { context_um: 150.0, min_cluster_grains: 5, ..fast_sheets() };
    let plan = field_plan(&p, &recs, &gs);
    assert_eq!(plan.strays.len(), 1);
    assert_eq!(plan.strays[0].id, stray_id);
    assert_eq!(plan.cut_stats.grains, 15);
    assert_eq!(plan.cut_stats.clusters, 3);
    assert_eq!(plan.strips.iter().find(|s| s.sample == "S3").unwrap().bands[0].grains.len(), 4, "each sample keeps its largest cluster");
}

#[test]
fn lines_header_reserves_room_for_grain_only_sample_labels() {
    let (recs, mut gs) = mixed_sample_field();
    let p = fast_sheets();
    for g in gs.values_mut().flatten() { g.sample = None; }
    let original = field_plan(&p, &recs, &gs);
    assert_eq!(original.cut_stats.header_mm, [50.0, 36.0]);
    for (i, g) in gs.values_mut().flatten().enumerate() { g.sample = Some(format!("S{}", i + 2)); }
    let plan = field_plan(&p, &recs, &gs);
    let expected = 19.1 + 16.0 * 2.7;
    for h in plan.cut_stats.header_mm { assert!((h - expected).abs() < 1e-7); }
}
