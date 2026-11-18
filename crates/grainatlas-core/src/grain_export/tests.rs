use super::*;
use stats::{Reference, describe, quantile};
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-10, "{a} != {b}");
}
fn run(s: &mut Session, p: &Project, o: &Options) -> BTreeMap<String, Vec<u8>> {
    let mut job = Export::begin(s, &p.to_json(), &serde_json::to_string(o).unwrap()).unwrap();
    let mut files = BTreeMap::new();
    let mut batches = 0;
    loop {
        let b = job.batch(s, &Ctl::NONE).unwrap();
        assert!(b.files.len() <= 2);
        for (name, data) in b.files {
            assert!(files.insert(name, data).is_none(), "filename collision");
        }
        batches += 1;
        assert!(batches < 1000);
        if b.done {
            break;
        }
    }
    files.extend(job.finish(&Ctl::NONE).unwrap());
    files
}
fn csv(files: &BTreeMap<String, Vec<u8>>) -> Vec<BTreeMap<String, String>> {
    let text = String::from_utf8(files["grains.csv"].clone()).unwrap();
    let mut lines = text.lines();
    let header: Vec<_> = lines.next().unwrap().split(',').collect();
    lines
        .map(|line| {
            let cells: Vec<_> = line.split(',').collect();
            assert_eq!(cells.len(), header.len());
            header
                .iter()
                .zip(cells)
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .collect()
}
#[test]
fn statistics_hand_computed_and_small_populations() {
    let mut v = [1.0, 2.0, 3.0, 4.0];
    let s = describe(&mut v);
    assert_eq!(s.n_px, 4);
    close(s.mean.unwrap(), 2.5);
    close(s.median.unwrap(), 2.5);
    close(s.sd.unwrap(), (5.0f64 / 3.0).sqrt());
    close(s.min.unwrap(), 1.0);
    close(s.max.unwrap(), 4.0);
    close(s.p5.unwrap(), 1.15);
    close(s.p25.unwrap(), 1.75);
    close(s.p75.unwrap(), 3.25);
    close(s.p95.unwrap(), 3.85);
    close(s.iqr.unwrap(), 1.5);
    close(s.skewness.unwrap(), 0.0);
    close(describe(&mut [0.0, 0.0, 0.0, 1.0]).skewness.unwrap(), 2.0);
    let singleton = describe(&mut [4.0]);
    assert_eq!(singleton.sd, None);
    assert_eq!(singleton.skewness, None);
    close(singleton.mean.unwrap(), 4.0);
    assert_eq!(describe(&mut []).mean, None);
    assert_eq!(describe(&mut [2.0, 2.0, 2.0]).skewness, None);
    assert_eq!(quantile(&[], 0.5), None);
}
#[test]
fn relative_ties_mad_and_reference_thresholds() {
    let r = Reference::new(vec![1.0, 2.0, 2.0, 4.0]);
    let v = r.of(Some(2.0), 2);
    assert_eq!(v.n, 4);
    close(v.percentile.unwrap(), 50.0);
    close(v.z.unwrap(), -0.25 / (4.75f64 / 3.0).sqrt());
    close(r.of(Some(4.0), 2).robust_z.unwrap(), 2.0 / (1.4826 * 0.5));
    assert_eq!(Reference::new(vec![1.0]).of(Some(1.0), 2).percentile, None);
    assert_eq!(Reference::new(vec![1.0, 2.0]).of(Some(1.0), 3).z, None);
    assert_eq!(Reference::new(vec![1.0, 2.0]).of(Some(1.0), 2).n, 2);
    let tied = Reference::new(vec![2.0; 4]).of(Some(2.0), 2);
    assert_eq!(tied.z, None);
    assert_eq!(tied.robust_z, None);
    close(tied.percentile.unwrap(), 50.0);
    assert_eq!(Reference::new(vec![]).of(None, 2).n, 0);
}
#[test]
fn directional_glcm_constant_checkerboard_and_no_pairs() {
    let ctl = &Ctl::NONE;
    assert_eq!(
        stats::texture(&[0.5; 4], &[255; 4], 2, 2, &[0.5; 4], ctl).unwrap(),
        (Some(0.0), Some(1.0))
    );
    let (c, h) = stats::texture(
        &[0.0, 1.0, 1.0, 0.0],
        &[255; 4],
        2,
        2,
        &[0.0, 0.0, 1.0, 1.0],
        ctl,
    )
    .unwrap();
    close(c.unwrap(), 480.5);
    close(h.unwrap(), (1.0 + 1.0 / 962.0) / 2.0);
    assert_eq!(
        stats::texture(&[0.5], &[255], 1, 1, &[0.5], ctl).unwrap(),
        (None, None)
    );
    assert_eq!(
        stats::texture(&[0.0, 1.0], &[255, 0], 2, 1, &[0.0], ctl).unwrap(),
        (None, None)
    );
}
#[test]
fn raster_centres_margin_and_mosaic_edge() {
    let (_, p) = synthetic::mount();
    let g: Grain =
        serde_json::from_value(p.extra["grainatlas_ui"]["grains"]["a"][0].clone()).unwrap();
    let px = vec![0.4f32; 120 * 96];
    let r = pixels::raster(&g, 1.0, 120, 96, 10.0, &Pixels::Float(&px), &Ctl::NONE).unwrap();
    assert_eq!((r.x, r.y, r.w, r.h), (0, 0, 24, 22));
    assert_eq!(r.mask.iter().filter(|&&m| m == 255).count(), 168);
    assert!(pixels::inside(
        &[[0.0, 0.0], [2.0, 0.0], [0.0, 2.0]],
        0.5,
        0.5
    ));
    assert!(!pixels::inside(
        &[[0.0, 0.0], [2.0, 0.0], [0.0, 2.0]],
        1.5,
        1.5
    ));
}
#[test]
fn synthetic_full_export_provenance_coordinates_images_and_order() {
    let (mut s, p) = synthetic::mount();
    let files = run(&mut s, &p, &Options::default());
    let rows = csv(&files);
    assert_eq!(rows.len(), 144);
    assert_eq!(
        rows.iter()
            .filter(|r| r["source_area_id"] == "a" && r["area_id"] == "a")
            .count(),
        36
    );
    let manifest: Value = serde_json::from_slice(&files["manifest.json"]).unwrap();
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["areas"].as_array().unwrap().len(), 2);
    assert!(
        manifest["areas"][0]["sampled_buffer_sha256"]
            .as_str()
            .unwrap()
            .len()
            == 64
    );
    assert_eq!(
        manifest["areas"][0]["sampled_buffer_sha256"],
        pixels::Pixels::Float(&s.outputs["a"].linear.px)
            .hash(&Ctl::NONE)
            .unwrap()
    );
    assert_eq!(manifest["areas"][0]["pixel_source"], "original-f32");
    assert!(!manifest["package_digest"].is_null());
    assert!(!manifest["areas"][0]["segmentation_fingerprint"]["parameters"].is_null());
    let header = String::from_utf8(files["grains.csv"].clone())
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert_eq!(
        header,
        table::columns()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(header.starts_with("grain_id,grain_label,sample,source_area_id,area_id,area_name,tile_index,layer,pixel_source,spot_names"));
    let geo: Value = serde_json::from_slice(&files["outlines.geojson"]).unwrap();
    assert_eq!(geo["coordinate_frame"]["units"], "micrometres");
    assert_eq!(
        geo["features"][0]["geometry"]["coordinates"][0][0],
        json!([1000.0, 2000.0])
    );
    assert_eq!(geo["features"].as_array().unwrap().len(), 72);
    let own = rows
        .iter()
        .find(|r| r["source_area_id"] == "a" && r["area_id"] == "a")
        .unwrap();
    assert_eq!(own["stage_x_um"], "993");
    assert_eq!(own["tile_index"], "0");
    assert_eq!(own["n_px"], "168");
    assert_eq!(own["area_mean_n"], "36");
    let crop = image::load_from_memory(&files[&own["crop_file"]]).unwrap();
    assert_eq!(crop.color(), image::ColorType::L16);
    let mask = image::load_from_memory(&files[&own["mask_file"]])
        .unwrap()
        .to_luma8();
    assert_eq!(mask.pixels().filter(|p| p.0[0] == 255).count(), 168);
    assert_eq!(crop.width(), mask.width());
    assert!(
        !String::from_utf8(files["grains.csv"].clone())
            .unwrap()
            .contains("NaN")
    );
}
#[test]
fn linear_transform_invariance_and_selection_independence() {
    let (mut s, p) = synthetic::mount();
    let opts = Options {
        include_crops: false,
        include_masks: false,
        ..Options::default()
    };
    let full = run(&mut s, &p, &opts);
    let rows = csv(&full);
    for i in 0..36 {
        let label = format!("grain/{:02}", i + 1);
        let a = rows
            .iter()
            .find(|r| {
                r["grain_label"] == label && r["source_area_id"] == "a" && r["area_id"] == "a"
            })
            .unwrap();
        let b = rows
            .iter()
            .find(|r| {
                r["grain_label"] == label && r["source_area_id"] == "b" && r["area_id"] == "b"
            })
            .unwrap();
        for measure in ["mean", "median"] {
            let key = format!("area_{measure}_z");
            assert!(
                (a[&key].parse::<f64>().unwrap() - b[&key].parse::<f64>().unwrap()).abs() < 1e-6
            );
        }
    }
    let one = Options {
        selected: Some(vec![uuid::Uuid::from_u128(100)]),
        ..opts
    };
    let selected = csv(&run(&mut s, &p, &one));
    assert_eq!(selected.len(), 2);
    for row in selected {
        let orig = rows
            .iter()
            .find(|r| r["grain_id"] == row["grain_id"] && r["area_id"] == row["area_id"])
            .unwrap();
        for scope in ["area", "tile", "mount"] {
            for metric in ["z", "robust_z", "percentile", "n"] {
                for value in ["mean", "median"] {
                    let k = format!("{scope}_{value}_{metric}");
                    assert_eq!(row[&k], orig[&k]);
                }
            }
        }
    }
    assert_eq!(full.len(), 4);
}
#[test]
fn cancelled_batch_and_finish_cannot_produce_a_complete_export() {
    let (mut s, p) = synthetic::mount();
    let mut job = Export::begin(&s, &p.to_json(), "{}").unwrap();
    job.batch(&mut s, &Ctl::NONE).unwrap();
    let ctl = Ctl {
        cancel: &|| true,
        progress: &|_, _| {},
    };
    assert!(matches!(job.batch(&mut s, &ctl), Err(Error::Cancelled)));
    assert!(job.finish(&Ctl::NONE).is_err());
    let ctl = Ctl {
        cancel: &|| true,
        progress: &|_, _| {},
    };
    assert!(pixels::Pixels::Float(&[0.0; 4]).hash(&ctl).is_err());
}
#[test]
fn ambiguous_stage_map_does_not_export_extra_layers() {
    let (mut s, mut p) = synthetic::mount();
    p.areas[1].tiles[1].stage_nm.as_mut().unwrap()[0] += 2000.0;
    s.project = Some(p.clone());
    let files = run(
        &mut s,
        &p,
        &Options {
            include_crops: false,
            include_masks: false,
            ..Options::default()
        },
    );
    assert_eq!(csv(&files).len(), 72);
    let m: Value = serde_json::from_slice(&files["manifest.json"]).unwrap();
    assert!(!m["skipped_layers"].as_array().unwrap().is_empty());
}
#[test]
fn displayed_lut_and_published_source_are_exact_and_tile_is_unknown() {
    let (mut s, p) = synthetic::mount();
    let mut opts = Options {
        pixel_values: PixelSource::Displayed,
        include_crops: false,
        include_masks: false,
        ..Options::default()
    };
    let mut tone = crate::photometric::ToneParams::neutral();
    tone.gamma = 2.0;
    opts.display_tone.insert("a".into(), tone.clone());
    let files = run(&mut s, &p, &opts);
    let rows = csv(&files);
    let r = rows
        .iter()
        .find(|r| r["area_id"] == "a" && r["grain_id"] == uuid::Uuid::from_u128(100).to_string())
        .unwrap();
    let expected =
        crate::photometric::tone::lut(&tone)[s.outputs["a"].base[0] as usize] as f64 / 255.0;
    close(r["mean"].parse().unwrap(), expected);
    assert_eq!(r["pixel_source"], "displayed-8bit");
    let published = vec![79u8; 120 * 96];
    s.load_bundle_area("a", 120, 96, published).unwrap();
    let files = run(
        &mut s,
        &p,
        &Options {
            include_crops: false,
            include_masks: false,
            ..Options::default()
        },
    );
    let rows = csv(&files);
    let r = rows.iter().find(|r| r["area_id"] == "a").unwrap();
    assert_eq!(r["pixel_source"], "published-8bit");
    assert_eq!(r["tile_index"], "");
    close(r["mean"].parse().unwrap(), 79.0 / 255.0);
}
#[test]
fn png_and_tiff_quantisation_match_and_incremental_zip_roundtrips() {
    let ras = pixels::Raster {
        x: 0,
        y: 0,
        w: 3,
        h: 1,
        mask: vec![255; 3],
        values: vec![0.0, 0.5, 1.0],
    };
    for format in [ImageFormat::Png, ImageFormat::Tiff] {
        let b = pixels::encode(&ras, false, format).unwrap();
        let decoded = image::load_from_memory(&b).unwrap().to_luma16();
        assert_eq!(decoded.into_raw(), vec![0, 32768, 65535]);
    }
    let files: Vec<File> = vec![
        ("a.png".into(), vec![1, 2, 3]),
        ("grains.csv".into(), b"a,b\r\n".to_vec()),
    ];
    let mut stream = crate::atlas::zip::Stream::default();
    let mut bytes = Vec::new();
    for (n, b) in &files {
        bytes.extend(stream.entry(n, b).unwrap());
    }
    bytes.extend(stream.finish());
    assert_eq!(crate::atlas::zip::read(&bytes).unwrap(), files);
    assert_eq!(bytes, crate::atlas::zip::write("", &files).unwrap());
}

#[test]
fn csv_quoting_spot_links_and_polygon_fingerprints() {
    assert_eq!(
        table::field("sample,\"name\"\r\nline"),
        "\"sample,\"\"name\"\"\r\nline\""
    );
    let (mut s, mut p) = synthetic::mount();
    let ui = p.extra.get_mut("grainatlas_ui").unwrap();
    ui["spots"] = json!([{"id":uuid::Uuid::from_u128(999),"area_id":"a","name":"spot1","grain_id":uuid::Uuid::from_u128(100),"grain_number":1,"x_um":7.0,"y_um":6.0,"diameter_um":5.0},{"id":uuid::Uuid::from_u128(998),"area_id":"a","name":"spot2","grain_id":uuid::Uuid::from_u128(100),"grain_number":1,"x_um":7.0,"y_um":6.0,"diameter_um":5.0}]);
    // A mapped layer fails coverage, and must be absent from its fingerprint/count.
    s.outputs.get_mut("b").unwrap().coverage[0] = 0.0;
    let opts = Options {
        selected: Some(vec![uuid::Uuid::from_u128(100)]),
        include_crops: false,
        include_masks: false,
        ..Options::default()
    };
    let files = run(&mut s, &p, &opts);
    let rows = csv(&files);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["spot_names"], "spot1;spot2");
    let m: Value = serde_json::from_slice(&files["manifest.json"]).unwrap();
    let b = m["areas"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["area_id"] == "b")
        .unwrap();
    assert_eq!(b["export_grain_count"], 0);
    assert_eq!(
        b["segmentation_fingerprint"]["polygons_sha256"],
        crate::util::sha256_hex(b"[]")
    );
}
#[test]
fn thousands_of_references_and_evicted_published_areas_are_bounded() {
    let (mut s, mut p) = synthetic::mount();
    for id in ["a", "b"] {
        let original = p.extra["grainatlas_ui"]["grains"][id]
            .as_array()
            .unwrap()
            .clone();
        let grains: Vec<_> = (0..1200)
            .map(|i| {
                let mut g = original[i % 36].clone();
                g["id"] = json!(uuid::Uuid::from_u128(
                    10000 + if id == "a" { 0 } else { 2000 } + i as u128
                ));
                g
            })
            .collect();
        p.extra.get_mut("grainatlas_ui").unwrap()["grains"][id] = json!(grains);
        let pixels = s.outputs[id].base.as_ref().clone();
        s.load_bundle_area(id, 120, 96, pixels).unwrap();
    }
    s.set_memory_budget(0);
    let opts = Options {
        selected: Some(vec![uuid::Uuid::from_u128(10000)]),
        include_crops: false,
        include_masks: false,
        ..Options::default()
    };
    let files = run(&mut s, &p, &opts);
    let rows = csv(&files);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["area_mean_n"], "1200");
    assert_eq!(s.rebuild_heavy_calls, 2);
    assert!(
        s.outputs
            .values()
            .all(|o| o.linear.px.is_empty() && o.coverage.is_empty())
    );
}
#[test]
fn cancel_during_raster_and_signal_partition_policy() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (mut s, mut p) = synthetic::mount();
    let mut g: Grain =
        serde_json::from_value(p.extra["grainatlas_ui"]["grains"]["a"][0].clone()).unwrap();
    g.polygon_um = vec![[0.0, 0.0], [120.0, 0.0], [120.0, 96.0], [0.0, 96.0]];
    let n = AtomicUsize::new(0);
    let cancel = || n.fetch_add(1, Ordering::SeqCst) >= 1;
    let ctl = Ctl {
        cancel: &cancel,
        progress: &|_, _| {},
    };
    assert!(matches!(
        pixels::raster(
            &g,
            1.0,
            120,
            96,
            0.0,
            &Pixels::Float(&s.outputs["a"].linear.px),
            &ctl
        ),
        Err(Error::Cancelled)
    ));
    p.areas[1]
        .area_meta
        .insert("zag.ABCCHistory.SignalType1".into(), "BSE".into());
    s.project = Some(p.clone());
    let opts = Options {
        include_crops: false,
        include_masks: false,
        ..Options::default()
    };
    let rows = csv(&run(&mut s, &p, &opts));
    assert!(rows.iter().all(|r| r["mount_mean_n"] == "36"));
    p.areas[1].area_meta.remove("zag.ABCCHistory.SignalType1");
    s.project = Some(p.clone());
    let rows = csv(&run(&mut s, &p, &opts));
    assert!(rows.iter().all(|r| r["mount_mean_n"] == "72"));
}

#[test]
fn artifacts_never_enter_output_or_reference_sets() {
    let (mut s, mut p) = synthetic::mount();
    p.extra.get_mut("grainatlas_ui").unwrap()["grains"]["a"][0]["artifact"] = json!("manual");
    let options = Options {
        selected: Some(vec![uuid::Uuid::from_u128(100)]),
        include_crops: false,
        include_masks: false,
        ..Options::default()
    };
    assert!(Export::begin(&s, &p.to_json(), &serde_json::to_string(&options).unwrap()).is_err());
    let rows = csv(&run(
        &mut s,
        &p,
        &Options {
            selected: Some(vec![uuid::Uuid::from_u128(101)]),
            ..options
        },
    ));
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter().find(|r| r["area_id"] == "a").unwrap()["area_mean_n"],
        "35"
    );
    assert!(rows.iter().all(|r| r["mount_mean_n"] == "71"));
}

#[test]
fn changing_pixels_between_batches_aborts_instead_of_mixing_provenance() {
    let (mut s, p) = synthetic::mount();
    let mut job = Export::begin(&s, &p.to_json(), "{}").unwrap();
    job.batch(&mut s, &Ctl::NONE).unwrap();
    s.outputs.get_mut("a").unwrap().mosaic_json.sha256 = "changed".into();
    assert!(
        job.batch(&mut s, &Ctl::NONE)
            .err()
            .unwrap()
            .to_string()
            .contains("changed during export")
    );
}

#[test]
fn large_synthetic_outlines_are_deterministic_irregular_and_disjoint() {
    let a = synthetic::large_grains(42, synthetic::DEFAULT_GRAINS_PER_AREA);
    let b = synthetic::large_grains(42, synthetic::DEFAULT_GRAINS_PER_AREA);
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap()
    );
    assert_ne!(
        a[0].polygon_um,
        synthetic::large_grains(43, 1)[0].polygon_um
    );
    assert!(a.iter().any(|g| g.major_um > 140.0));
    assert!(a.iter().any(|g| g.major_um < 35.0));
    let mut ids = BTreeSet::new();
    for (i, g) in a.iter().enumerate() {
        assert!(ids.insert(g.id));
        assert!((30..=60).contains(&g.polygon_um.len()));
        assert!((30.0..=150.0).contains(&g.major_um));
        assert!((-90.0..=90.0).contains(&g.orientation_deg));
        assert!(g.area_um2 > 0.0);
        assert!(pixels::inside(
            &g.polygon_um,
            g.centroid_um[0],
            g.centroid_um[1]
        ));
        assert!(
            g.polygon_um
                .iter()
                .flatten()
                .all(|&x| (0.0..3000.0).contains(&x))
        );
        for other in &a[..i] {
            let b = g.bbox_um;
            let q = other.bbox_um;
            assert!(b[2] <= q[0] || b[0] >= q[2] || b[3] <= q[1] || b[1] >= q[3]);
        }
    }
}

#[test]
#[ignore = "allocates two 6000 × 6000 mosaics and exports 3000 synthetic grains"]
fn large_synthetic_mount_export() {
    let (mut session, project) = synthetic::large_mount(42, synthetic::DEFAULT_GRAINS_PER_AREA);
    assert_eq!(project.areas.len(), 2);
    assert!(
        project
            .areas
            .iter()
            .all(|a| a.tiles.len() == 16 && a.um_per_px() == 0.5)
    );
    let a = &session.outputs["a"].linear;
    let b = &session.outputs["b"].linear;
    assert_eq!((a.w, a.h, b.w, b.h), (6000, 6000, 6000, 6000));
    assert!(a.px.iter().zip(&b.px).all(|(&x, &y)| y == 0.55 * x + 0.06));
    let mut export = Export::begin(&session, &project.to_json(), "{}").unwrap();
    let mut names = BTreeSet::new();
    loop {
        let batch = export.batch(&mut session, &Ctl::NONE).unwrap();
        assert!(batch.files.len() <= 2);
        for (name, _) in batch.files {
            assert!(names.insert(name));
        }
        if batch.done {
            break;
        }
    }
    assert_eq!(names.len(), 12000);
    let metadata: BTreeMap<_, _> = export.finish(&Ctl::NONE).unwrap().into_iter().collect();
    assert_eq!(csv(&metadata).len(), 6000);
}

#[test]
fn short_image_names_sanitise_labels_preserve_ids_and_resolve_collisions() {
    let (mut session, mut project) = synthetic::mount();
    project.areas[0].name = "Mount / A".into();
    for i in 0..2 {
        project.extra.get_mut("grainatlas_ui").unwrap()["grains"]["a"][i]["label"] =
            json!("zr / 01");
    }
    let options = Options {
        selected: Some(vec![uuid::Uuid::from_u128(100), uuid::Uuid::from_u128(101)]),
        ..Options::default()
    };
    let files = run(&mut session, &project, &options);
    let rows = csv(&files);
    assert_eq!(rows.len(), 4);
    for row in &rows {
        let i = usize::from(row["grain_id"] == uuid::Uuid::from_u128(101).to_string());
        let layer = if row["area_id"] == "a" {
            ""
        } else {
            "-Synthetic_b"
        };
        let short = if i == 0 { "00000064" } else { "00000065" };
        let stem = format!("Mount___A-zr___01-{short}{layer}");
        assert_eq!(row["crop_file"], format!("crops/{stem}.png"));
        assert_eq!(row["mask_file"], format!("masks/{stem}.png"));
        assert!(files.contains_key(&row["crop_file"]));
        assert!(files.contains_key(&row["mask_file"]));
        assert_eq!(row["source_area_uuid"], project.areas[0].uuid.to_string());
        let area = project
            .areas
            .iter()
            .find(|a| a.id == row["area_id"])
            .unwrap();
        assert_eq!(row["area_uuid"], area.uuid.to_string());
    }
    let tiff = run(
        &mut session,
        &project,
        &Options {
            format: ImageFormat::Tiff,
            ..options
        },
    );
    assert!(
        csv(&tiff)
            .iter()
            .all(|r| r["crop_file"].ends_with(".tiff") && r["mask_file"].ends_with(".png"))
    );
    let mut used = BTreeSet::new();
    let mut counters = BTreeMap::new();
    for (base, expected) in [
        ("name", "name"),
        ("name-2", "name-2"),
        ("name", "name-3"),
        ("NAME", "NAME-4"),
    ] {
        assert_eq!(unique_stem(base.into(), &mut used, &mut counters), expected);
    }
    assert_eq!(safe(""), "grain");
    assert_eq!(safe(&"x".repeat(100)).len(), 48);
}

#[test]
fn scanline_mask_matches_even_odd_at_pixel_centres_on_random_polygons() {
    let (_, project) = synthetic::mount();
    let template: Grain =
        serde_json::from_value(project.extra["grainatlas_ui"]["grains"]["a"][0].clone()).unwrap();
    let mut rng = synthetic::Rng(0x706f6c79676f6e);
    let pixels: Vec<f32> = (0..64 * 64).map(|i| i as f32 / 4096.0).collect();
    let fixtures = [
        vec![[0.5, 0.5], [10.5, 0.5], [10.5, 10.5], [0.5, 10.5]],
        vec![[0.5, 0.5], [10.5, 10.5], [0.5, 10.5], [10.5, 0.5]],
        vec![[1.5, 1.5], [1.5, 1.5], [1.5, 10.5]],
        vec![[0.5, 0.5], [10.5, 0.5], [0.5, 10.5]],
        vec![
            [1.5, 1.5],
            [15.5, 1.5],
            [15.5, 15.5],
            [10.5, 15.5],
            [10.5, 5.5],
            [5.5, 5.5],
            [5.5, 15.5],
            [1.5, 15.5],
        ],
    ];
    for case in 0..512 {
        let scale = [0.1, 0.3, 0.5, 1.0, 1.7][case % 5];
        let mut g = template.clone();
        let mut poly = if case < fixtures.len() {
            fixtures[case].clone()
        } else {
            let n = 3 + (rng.next() % 58) as usize;
            (0..n)
                .map(|i| {
                    let angle = std::f64::consts::TAU * i as f64 / n as f64;
                    let radius = 5.0 + 40.0 * rng.unit();
                    [32.0 + radius * angle.cos(), 32.0 + radius * angle.sin()]
                })
                .collect::<Vec<_>>()
        };
        if case % 3 == 0 {
            poly.reverse();
        }
        if case % 7 == 0 && case >= fixtures.len() {
            poly.swap(0, 2);
        }
        // Half-pixel coordinates and adjacent representable values stress
        // vertices, horizontal edges and floating point boundary rounding.
        if case % 4 == 0 {
            for p in &mut poly {
                for v in p {
                    *v = v.floor() + 0.5;
                }
            }
        }
        for p in &mut poly {
            for v in p {
                *v *= scale;
            }
        }
        if case % 11 == 0 {
            for p in &mut poly {
                p[0] = p[0].next_up();
            }
        }
        if case % 13 == 0 {
            for p in &mut poly {
                p[1] = p[1].next_down();
            }
        }
        g.polygon_um = poly;
        let r = pixels::raster(
            &g,
            scale,
            64,
            64,
            (case % 6) as f64,
            &Pixels::Float(&pixels),
            &Ctl::NONE,
        )
        .unwrap();
        for row in 0..r.h {
            for col in 0..r.w {
                let expected = pixels::inside(
                    &g.polygon_um,
                    (col + r.x) as f64 * scale + scale / 2.0,
                    (row + r.y) as f64 * scale + scale / 2.0,
                );
                assert_eq!(
                    r.mask[row * r.w + col],
                    if expected { 255 } else { 0 },
                    "case {case}, pixel ({col},{row}), scale {scale}"
                );
                assert_eq!(
                    r.values[row * r.w + col],
                    pixels[(row + r.y) * 64 + col + r.x] as f64
                );
            }
        }
    }
}

#[test]
#[ignore = "manual mask timing comparison against the per-pixel ray test"]
fn scanline_raster_timing() {
    use std::time::{Duration, Instant};
    let grains = synthetic::large_grains(42, 300);
    let pixels = vec![0.4; 400 * 400];
    let (mut scanline, mut ray) = (Duration::ZERO, Duration::ZERO);
    for mut g in grains {
        let [left, top, _, _] = g.bbox_um;
        for p in &mut g.polygon_um {
            p[0] += 10.0 - left;
            p[1] += 10.0 - top;
        }
        let start = Instant::now();
        let r =
            pixels::raster(&g, 0.5, 400, 400, 10.0, &Pixels::Float(&pixels), &Ctl::NONE).unwrap();
        scanline += start.elapsed();
        let start = Instant::now();
        let mut expected = vec![0; r.w * r.h];
        for row in 0..r.h {
            for col in 0..r.w {
                expected[row * r.w + col] = if pixels::inside(
                    &g.polygon_um,
                    (r.x + col) as f64 * 0.5 + 0.25,
                    (r.y + row) as f64 * 0.5 + 0.25,
                ) {
                    255
                } else {
                    0
                };
            }
        }
        ray += start.elapsed();
        assert_eq!(r.mask, expected);
    }
    println!(
        "300 grains: scanline raster (including value copy) {:.3} s; even-odd mask alone {:.3} s",
        scanline.as_secs_f64(),
        ray.as_secs_f64()
    );
}
