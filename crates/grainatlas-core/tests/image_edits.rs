#[allow(dead_code)]
#[path = "../examples/support/mod.rs"]
mod support;
use grainatlas_core::{
    layout::{
        sheets::{self, SheetResult},
        AreaImage,
    },
    project::image_edits::*,
    project::{printable_grains, AreaImageEdits, ImageEdits, Project},
    segment::Grain,
};
use serde_json::json;
fn recipe() -> AreaImageEdits {
    serde_json::from_value(json!({"crop":{"kind":"rect","points_um":[[20,20],[180,250]]},"rotate_deg":31.5,"flip_h":true,"flip_v":false,"levels":{"black":18,"white":236},"masks":[{"points_um":[[30,30],[40,50]],"radius_um":6}]})).unwrap()
}
fn grains(p: &Project) -> Vec<Grain> {
    serde_json::from_value(p.extra["grainatlas_ui"]["grains"]["a"].clone()).unwrap()
}
#[test]
fn recipe_round_trip_old_projects_and_registered_geometry_are_unchanged() {
    let (mut session, mut p, _) = support::synthetic();
    let old = p.to_json();
    let before = serde_json::to_vec(&p.extra["grainatlas_ui"]).unwrap();
    assert!(!old.contains("image_edits"));
    assert_eq!(Project::from_json(&old).unwrap().to_json(), old);
    let mut precise = recipe();
    precise.rotate_deg = 31.123456789;
    precise.crop.as_mut().unwrap().points_um[0] = [20.123456789, 20.987654321];
    precise.masks[0].radius_um = 6.123456789;
    let stored = ImageEdits {
        areas: std::collections::BTreeMap::from([("a".into(), precise)]),
        ..Default::default()
    };
    for edits in [
        Some(stored.clone()),
        None,
        Some(stored),
        Some(ImageEdits::default()),
    ] {
        p.image_edits = edits;
        let saved = p.to_json();
        let reopened = Project::from_json(&saved).unwrap();
        assert_eq!(reopened.to_json().as_bytes(), saved.as_bytes());
        assert_eq!(
            serde_json::to_vec(&reopened.extra["grainatlas_ui"]).unwrap(),
            before,
            "positions, polygons, names, ids and links are byte-identical through apply, undo and reset"
        );
        session.load_project(&saved).unwrap();
        let files = session.atlas_files(&saved, "").unwrap();
        let record = files
            .iter()
            .find(|(name, _)| name == "project.json")
            .unwrap();
        let published = Project::from_json(std::str::from_utf8(&record.1).unwrap()).unwrap();
        assert_eq!(
            published.image_edits, p.image_edits,
            "published project round trip"
        );
    }
}
#[test]
fn rect_polygon_mask_and_rotated_crop_use_original_centroids() {
    let (_, p, _) = support::synthetic();
    let gs = grains(&p);
    let area = &p.areas[0];
    let mut e = recipe();
    assert_eq!(printable_grains(area, &gs, Some(&e)).len(), 1);
    e.rotate_deg = -121.0;
    e.flip_v = true;
    assert_eq!(printable_grains(area, &gs, Some(&e))[0].id, gs[0].id);
    e.crop = Some(ImageCrop {
        kind: CropKind::Polygon,
        points_um: vec![[0.0, 0.0], [160.0, 0.0], [160.0, 200.0], [0.0, 200.0]],
    });
    assert_eq!(printable_grains(area, &gs, Some(&e)).len(), 1);
    e.masks = vec![MaskStroke {
        points_um: vec![[20.0, 120.0], [150.0, 120.0]],
        radius_um: 5.0,
    }];
    assert!(printable_grains(area, &gs, Some(&e)).is_empty());
    assert_eq!(printable_grains(area, &gs, None), gs);
    assert!(e.visible([0.0, 0.0]), "polygon boundary is included");
}
#[test]
fn band_pixels_apply_levels_crop_and_masks_without_changing_source() {
    let source = vec![20, 60, 100, 140, 180, 220, 240, 250];
    let original = source.clone();
    let mut e = AreaImageEdits {
        crop: Some(ImageCrop {
            kind: CropKind::Rect,
            points_um: vec![[1.0, 0.0], [4.0, 2.0]],
        }),
        levels: Some(ImageLevels {
            black: 20,
            white: 220,
        }),
        masks: vec![MaskStroke {
            points_um: vec![[2.5, 1.5]],
            radius_um: 0.4,
        }],
        ..Default::default()
    };
    let render = |e: &AreaImageEdits| {
        let img = AreaImage {
            source_size_um: None,
            edits: Some(e),
            id: "a",
            name: "a",
            sample: "a",
            w: 4,
            h: 2,
            px: &source,
            lut: std::array::from_fn(|i| i as u8),
            um_per_px: 1.0,
        };
        let s = img.display_size();
        sheets::band_pixels(
            &img,
            [0.0, 0.0, s[0] as f64, s[1] as f64],
            &[],
            &Default::default(),
        )
    };
    let (w, h, px, _) = render(&e);
    assert_eq!((w, h), (4, 2));
    assert_eq!(px, vec![255, 51, 102, 153, 255, 255, MASK_BACKGROUND, 255]);
    e.rotate_deg = 90.0;
    let (w, h, rot, _) = render(&e);
    assert_eq!((w, h), (2, 4));
    assert_eq!(rot, vec![255, 255, 255, 51, MASK_BACKGROUND, 102, 255, 153]);
    assert_eq!(source, original);
}
#[test]
fn mask_fill_and_hatched_border_follow_the_entire_path_in_print_bands() {
    let source = vec![100; 200 * 100];
    let original = source.clone();
    let mut edits = AreaImageEdits {
        crop: Some(ImageCrop {
            kind: CropKind::Rect,
            points_um: vec![[10.0, 10.0], [190.0, 90.0]],
        }),
        levels: Some(ImageLevels {
            black: 20,
            white: 220,
        }),
        masks: vec![MaskStroke {
            points_um: vec![[20.0, 50.0], [180.0, 50.0]],
            radius_um: 12.0,
        }],
        ..Default::default()
    };
    assert_eq!(
        edits.mask_pixel([100.5, 50.5]),
        Some(MASK_BACKGROUND),
        "middle of the path, far from its endpoints"
    );
    assert_eq!(
        edits.mask_pixel([31.5, 39.5]),
        Some(MASK_INK),
        "hatched edge"
    );
    assert_eq!(
        edits.mask_pixel([36.5, 39.5]),
        Some(MASK_BACKGROUND),
        "gap between hatch marks"
    );
    assert_eq!(
        edits.mask_pixel([10.5, 50.5]),
        Some(MASK_BACKGROUND),
        "round cap"
    );
    assert_eq!(edits.mask_pixel([5.5, 50.5]), None, "outside the stroke");
    for angle in [0.0, 90.0] {
        edits.rotate_deg = angle;
        let img = AreaImage {
            source_size_um: None,
            edits: Some(&edits),
            id: "a",
            name: "a",
            sample: "a",
            w: 200,
            h: 100,
            px: &source,
            lut: std::array::from_fn(|i| i as u8),
            um_per_px: 1.0,
        };
        let size = img.display_size();
        for enhanced in [false, true] {
            let params = grainatlas_core::photometric::enhance::EnhanceParams {
                denoise: enhanced,
                sharpen: enhanced,
                ..Default::default()
            };
            let (w, _, px, _) = sheets::band_pixels(
                &img,
                [0.0, 0.0, size[0] as f64, size[1] as f64],
                &[],
                &params,
            );
            for (mount, value) in [
                ([100.5, 50.5], MASK_BACKGROUND),
                ([31.5, 39.5], MASK_INK),
                ([36.5, 39.5], MASK_BACKGROUND),
                ([2.5, 2.5], 255),
            ] {
                let p = apply(img.image_transform(), mount);
                assert_eq!(
                    px[p[1].floor() as usize * w + p[0].floor() as usize],
                    value,
                    "angle {angle}, enhanced {enhanced}, mount {mount:?}"
                );
                assert_eq!(
                    img.sample_mount(mount, 255),
                    value,
                    "whole-area and band samplers agree"
                );
            }
        }
    }
    assert_eq!(source, original);
    // Intersections use the union boundary, not an extra hatch down the middle.
    edits.masks.push(MaskStroke {
        points_um: vec![[100.0, 20.0], [100.0, 80.0]],
        radius_um: 12.0,
    });
    assert_eq!(edits.mask_pixel([100.5, 39.5]), Some(MASK_BACKGROUND));
}
#[test]
fn sheets_and_whole_print_recipe_round_trip_and_registration() {
    let (mut session, mut p, _) = support::synthetic();
    let gs = grains(&p);
    let before = serde_json::to_vec(&p.extra["grainatlas_ui"]).unwrap();
    for angle in [0.0, 90.0, 31.5, -90.0] {
        for k in [2.0, 4.0] {
            let mut e = recipe();
            e.rotate_deg = angle;
            p.image_edits = Some(ImageEdits {
                areas: std::collections::BTreeMap::from([("a".into(), e.clone())]),
                ..Default::default()
            });
            session.load_project(&p.to_json()).unwrap();
            let params = json!({"um_per_mm":k,"packing":"packed","min_cluster_grains":0,"min_cut_grains":1,"min_grain_um":0,"enhance":{"denoise":false,"sharpen":false}});
            let r: SheetResult = serde_json::from_str(
                &session
                    .print_sheets(
                        &params.to_string(),
                        &p.extra["grainatlas_ui"]["grains"].to_string(),
                        "",
                    )
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(r.stats.grains, 1);
            assert_eq!(r.left_off_summary.cropped, 1);
            let b = r
                .registration
                .bands
                .iter()
                .find(|b| b.grains.iter().any(|g| g.id == gs[0].id))
                .unwrap();
            let m = e.transform([support::W as f64, support::H as f64]);
            let displayed = apply(m, gs[0].centroid_um);
            let paper = [
                b.sheet_rect_mm[0] + (displayed[0] - b.mosaic_rect_um[0]) / k,
                b.sheet_rect_mm[1] + (displayed[1] - b.mosaic_rect_um[1]) / k,
            ];
            let mount = apply(b.paper_to_mount.unwrap(), paper);
            assert!(
                (mount[0] - gs[0].centroid_um[0]).hypot(mount[1] - gs[0].centroid_um[1]) < 0.01
            );
            assert_eq!(
                serde_json::to_vec(&p.extra["grainatlas_ui"]).unwrap(),
                before
            );
            let whole: serde_json::Value = serde_json::from_str(
                &session
                    .print_whole("{\"area_id\":\"a\",\"grid\":false}", "")
                    .unwrap(),
            )
            .unwrap();
            assert!(whole["registration"]["bands"][0]["paper_to_mount"].is_array());
            if angle == 31.5 && k == 4.0 {
                assert!(r.pages.iter().any(|s| s.contains("mount +y")));
                let preview: serde_json::Value = serde_json::from_str(
                    &session
                        .print_sheets_preview(
                            &params.to_string(),
                            &p.extra["grainatlas_ui"]["grains"].to_string(),
                            "",
                            "",
                        )
                        .unwrap(),
                )
                .unwrap();
                let id = preview["job_id"].as_u64().unwrap() as usize;
                for page in 0..preview["pages"].as_array().unwrap().len() {
                    session.print_task(id, "quality", page).unwrap();
                }
                let full: serde_json::Value =
                    serde_json::from_slice(&session.print_task(id, "full", 0).unwrap()).unwrap();
                assert_eq!(full["ownership"], preview["ownership"]);
                assert_eq!(full["registration"], preview["registration"]);
                assert_eq!(full["left_off_summary"]["cropped"], 1);
            }
        }
    }
    p.image_edits = None;
    session.load_project(&p.to_json()).unwrap();
    let r: SheetResult = serde_json::from_str(
        &session
            .print_sheets(
                "{\"min_cluster_grains\":0,\"min_grain_um\":0}",
                &p.extra["grainatlas_ui"]["grains"].to_string(),
                "",
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(r.stats.grains, 2);
}

#[test]
fn arranged_cropped_piece_rescales_rotates_and_maps_paper_back_to_mount() {
    use sheets::{arrange, PrintEdits, SheetPlan, StripEdit};
    let (mut session, mut project, _) = support::synthetic();
    let gs = grains(&project);
    let grain_json = project.extra["grainatlas_ui"]["grains"].to_string();
    let registered = serde_json::to_vec(&project.extra["grainatlas_ui"]).unwrap();
    for image_angle in [0.0, 31.5] {
        let mut image = recipe();
        image.rotate_deg = image_angle;
        image.crop.as_mut().unwrap().points_um = vec![[60.0, 90.0], [180.0, 250.0]];
        image.masks = vec![MaskStroke {
            points_um: vec![[95.0, 105.0], [95.0, 150.0]],
            radius_um: 6.0,
        }];
        project.image_edits = Some(ImageEdits {
            areas: std::collections::BTreeMap::from([("a".into(), image.clone())]),
            ..Default::default()
        });
        session.load_project(&project.to_json()).unwrap();
        let params=json!({"um_per_mm":4,"packing":"packed","min_cluster_grains":0,"min_cut_grains":1,"min_grain_um":0,"enhance":{"denoise":false,"sharpen":false}}).to_string();
        let automatic: SheetResult =
            serde_json::from_str(&session.print_sheets(&params, &grain_json, "").unwrap()).unwrap();
        let mut base: SheetPlan = automatic.plan;
        base.print_spots = Some(serde_json::from_value(project.extra["grainatlas_ui"]["spots"].clone()).unwrap());
        base.print_labels = Some(Default::default());
        let arranging = vec![StripEdit::Arrange {}];
        let arranged: SheetResult = serde_json::from_str(
            &session
                .print_plan_render(
                    &serde_json::to_string(&base).unwrap(),
                    &serde_json::to_string(&arranging).unwrap(),
                    &grain_json,
                    "",
                )
                .unwrap(),
        )
        .unwrap();
        let key = arranged.plan.strips[0].key.clone();
        for paper_angle in [90.0, 23.125] {
            for scale in [0.73, 1.25] {
                let edits = PrintEdits {
                    version: 2,
                    params: base.params.clone(),
                    um_per_mm: base.um_per_mm,
                    edits: vec![
                        StripEdit::Arrange {},
                        StripEdit::Transform {
                            pieces: vec![arrange::PiecePlacement {
                                strip: key.clone(),
                                sheet: 0,
                                origin_mm: [60.123456, 80.654321],
                                scale,
                                rotation_deg: paper_angle,
                            }],
                        },
                    ],
                };
                project.print_sheets = Some(edits.clone().into());
                let saved = project.to_json();
                let reopened = Project::from_json(&saved).unwrap();
                assert_eq!(reopened.image_edits, project.image_edits);
                assert_eq!(
                    reopened.print_sheets, project.print_sheets,
                    "v2 arrangement and v1 image recipe coexist"
                );
                session.load_project(&saved).unwrap();
                let r: SheetResult = serde_json::from_str(
                    &session
                        .print_plan_render(
                            &serde_json::to_string(&base).unwrap(),
                            &serde_json::to_string(&edits).unwrap(),
                            &grain_json,
                            "",
                        )
                        .unwrap(),
                )
                .unwrap();
                assert!(r.dropped.is_empty(), "{:?}", r.dropped);
                assert!(r.problems.is_empty(), "{:?}", r.problems);
                assert_eq!(r.stats.grains, 1);
                assert_eq!(r.left_off_summary.cropped, 1);
                assert!(r
                    .left_off
                    .iter()
                    .any(|off| off.grain.id == gs[1].id && off.reason == "cropped"));
                assert!(r.registration.bands.iter().all(|b| b
                    .grains
                    .iter()
                    .chain(&b.context)
                    .all(|g| g.id != gs[1].id)));
                let b = r
                    .registration
                    .bands
                    .iter()
                    .find(|b| b.grains.iter().any(|g| g.id == gs[0].id))
                    .unwrap();
                let strip = r
                    .plan
                    .strips
                    .iter()
                    .find(|s| {
                        s.bands
                            .iter()
                            .any(|part| part.mosaic_rect_um == b.mosaic_rect_um)
                    })
                    .unwrap();
                let part = &strip.bands[0];
                let displayed = apply(
                    image.transform([support::W as f64, support::H as f64]),
                    gs[0].centroid_um,
                );
                // Derive the physical paper point from the SVG piece placement, independently
                // of the registration's forward/inverse pair (which could cancel the same bug).
                let placement = arrange::band_transform(strip, part, base.um_per_mm);
                let paper = arrange::point(placement.mount_to_paper, displayed);
                let t = b.transform.as_ref().unwrap();
                assert_eq!(b.paper_to_mount, Some(t.paper_to_mount));
                assert_eq!(b.image_crop, image.crop);
                assert!((t.um_per_mm - base.um_per_mm / scale).abs() < 1e-10);
                for recovered in [
                    apply(t.paper_to_mount, paper),
                    apply(b.paper_to_mount.unwrap(), paper),
                ] {
                    for d in 0..2 {
                        assert!((recovered[d]-gs[0].centroid_um[d]).abs()<0.01,"image {image_angle}, paper {paper_angle}, scale {scale}: {recovered:?}");
                    }
                }
                let forward = apply(t.mount_to_paper, gs[0].centroid_um);
                for d in 0..2 {
                    assert!((forward[d] - paper[d]).abs() * t.um_per_mm < 0.01);
                }
                // Arranged pieces fetch the same crop/levels/mask-aware band sampler.
                let (band, _) = grainatlas_core::image::decode(
                    &session.print_band_image("a", b.mosaic_rect_um, "").unwrap(),
                )
                .unwrap();
                let pixels = band.to_u8();
                assert!(pixels.contains(&255), "crop background remains in the band");
                assert!(
                    pixels.contains(&MASK_BACKGROUND),
                    "mask fill survives arrangement"
                );
                assert!(
                    pixels.contains(&MASK_INK),
                    "hatched mask survives arrangement"
                );
                let doc = roxmltree::Document::parse(&r.pages[b.sheet - 1]).unwrap();
                let spot = doc
                    .descendants()
                    .find(|n| {
                        n.attribute("data-spot-label")
                            == Some("50000000-0000-4000-8000-000000000001")
                    })
                    .unwrap();
                let marker = doc.descendants().find(|n| n.attribute("data-spot-marker") == Some("50000000-0000-4000-8000-000000000001")).unwrap();
                let circle = marker.descendants().find(|n| n.has_tag_name("circle")).unwrap();
                let paper_spot = [circle.attribute("cx").unwrap().parse::<f64>().unwrap(), circle.attribute("cy").unwrap().parse::<f64>().unwrap()];
                assert!((circle.attribute("r").unwrap().parse::<f64>().unwrap() - 15.0 / t.um_per_mm / 2.0).abs() < 1e-9);
                let report = &r.registration.spot_labels.as_ref().unwrap()[b.sheet - 1];
                let label = report.labels.iter().find(|l| l.spot_id == "50000000-0000-4000-8000-000000000001").unwrap();
                assert_eq!(label.anchor_mm, paper_spot);
                assert_eq!(spot.text(), Some(label.name.as_str()));
                assert!(spot.ancestors().all(|n| n.attribute("transform").is_none()), "labels remain upright in paper space");
                let recovered = apply(t.paper_to_mount, paper_spot);
                assert!(
                    (recovered[0] - 75.0).abs() < 0.01 && (recovered[1] - 120.0).abs() < 0.01,
                    "spot uses the same composed frame"
                );
                assert_eq!(
                    serde_json::to_vec(&session.project.as_ref().unwrap().extra["grainatlas_ui"])
                        .unwrap(),
                    registered
                );
            }
        }
    }
}
