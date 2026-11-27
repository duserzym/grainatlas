use grainatlas_core::{
    image::Gray,
    segment::{self, Polarity, SegmentParams, Signal, WorkImage},
};

fn image(dark: bool) -> WorkImage {
    let mut px = vec![0.05; 64 * 64];
    for y in 10..54 {
        for x in 10..54 {
            px[y * 64 + x] = 0.4;
        }
    }
    for y in 20..44 {
        for x in 20..44 {
            px[y * 64 + x] = 0.9;
        }
    }
    if dark {
        for v in &mut px {
            *v = 1.0 - *v;
        }
    }
    WorkImage {
        img: Gray { w: 64, h: 64, px },
        cov: vec![true; 64 * 64],
        um_per_px: 1.0,
        tiles_um: vec![],
        signal: Signal::Cl,
    }
}
#[test]
fn click_sensitivity_grows_bright_and_dark_grains_without_changing_the_image() {
    for dark in [false, true] {
        let w = image(dark);
        let before = w.img.px.clone();
        let mut params = SegmentParams {
            polarity: if dark {
                Polarity::Dark
            } else {
                Polarity::Bright
            },
            smooth_px: 0.0,
            close_px: 1,
            simplify_px: 0.1,
            ..Default::default()
        };
        params.click_sensitivity = Some(0.1);
        let small = segment::grain_at(&w, &params, 32.0, 32.0).unwrap();
        params.click_sensitivity = Some(0.9);
        let large = segment::grain_at(&w, &params, 32.0, 32.0).unwrap();
        assert!(large.area_um2 > small.area_um2 * 2.0);
        assert_eq!(w.img.px, before);
    }
}
#[test]
fn legacy_click_threshold_and_default_serialization_stay_compatible() {
    let w = image(false);
    let mut params = SegmentParams {
        smooth_px: 0.0,
        close_px: 1,
        ..Default::default()
    };
    let legacy = segment::grain_at(&w, &params, 32.0, 32.0).unwrap();
    assert!(
        serde_json::to_value(&params)
            .unwrap()
            .get("click_sensitivity")
            .is_none()
    );
    let decoded: SegmentParams =
        serde_json::from_value(serde_json::json!({"threshold":0.3})).unwrap();
    assert_eq!(decoded.click_sensitivity, None);
    params.click_sensitivity = Some(0.5);
    let normal = segment::grain_at(&w, &params, 32.0, 32.0).unwrap();
    assert_eq!(legacy.polygon_um, normal.polygon_um);
    assert_eq!(legacy.area_um2, normal.area_um2);
}
