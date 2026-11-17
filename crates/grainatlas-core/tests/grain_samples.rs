use grainatlas_core::image::Gray;
use grainatlas_core::segment::{self, Grain, OutlinePad, SegmentParams, Signal, WorkImage};

fn grain_json(sample: Option<&str>) -> String {
    let sample = sample.map_or_else(String::new, |s| format!(",\"sample\":\"{s}\""));
    format!(
        r#"{{"id":"00000000-0000-0000-0000-000000000001","number":1{sample},"polygon_um":[[20.0,20.0],[60.0,20.0],[60.0,60.0],[20.0,60.0]],"bbox_um":[20.0,20.0,60.0,60.0],"centroid_um":[40.0,40.0],"area_um2":1600.0,"eq_diameter_um":40.0,"major_um":40.0,"minor_um":40.0,"orientation_deg":0.0,"mean_intensity":0.5,"manual":false,"scale_bar_um":20.0}}"#
    )
}

#[test]
fn optional_sample_round_trips_without_changing_legacy_bytes() {
    for sample in [None, Some("S1"), Some("")] {
        let json = grain_json(sample);
        let grain: Grain = serde_json::from_str(&json).unwrap();
        assert_eq!(grain.sample.as_deref(), sample);
        assert_eq!(serde_json::to_string(&grain).unwrap(), json);
    }
}

#[test]
fn padding_numbering_merging_splitting_and_find_missing_keep_sample() {
    let original: Grain = serde_json::from_str(&grain_json(Some("S1"))).unwrap();
    let w = WorkImage {
        tiles_um: Vec::new(),
        signal: Signal::Cl,
        img: Gray {
            w: 100,
            h: 100,
            px: vec![0.5; 10000],
        },
        cov: vec![true; 10000],
        um_per_px: 1.0,
    };
    let padded = segment::pad_outlines(
        &w,
        std::slice::from_ref(&original),
        OutlinePad::default(),
        0.8,
        |_| true,
    );
    assert!(padded[0].area_um2 > original.area_um2);
    assert_eq!(padded[0].sample, original.sample);
    let mut numbered = padded.clone();
    numbered[0].sample = None;
    segment::number(&mut numbered, std::slice::from_ref(&original), 20.0);
    assert_eq!(numbered[0].sample, original.sample);
    let input = serde_json::to_string(&numbered).unwrap();
    let output = grainatlas_core::project::session::renumber(&input, 20.0, "").unwrap();
    let renumbered: Vec<Grain> = serde_json::from_str(&output).unwrap();
    assert_eq!(renumbered[0].sample, original.sample);
    let split = segment::split_grain(&w, &original, [40.0, 0.0, 40.0, 100.0], 0.8);
    assert_eq!(split.len(), 2);
    assert!(split.iter().all(|g| g.sample == original.sample));
    let merged = segment::merge_grains(&w, &split, 0.8).unwrap();
    assert_eq!(merged.id, split[0].id);
    assert_eq!(merged.sample, original.sample);
    let missing = segment::add_missing(
        &w,
        &SegmentParams::default(),
        std::slice::from_ref(&original),
        OutlinePad::default(),
    );
    assert_eq!(missing[0], original);
    assert!(missing.iter().skip(1).all(|g| g.sample.is_none()));
}
