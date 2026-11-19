//! Pins the open -> stitch -> photometric -> blend -> tone canvas pipeline to
//! origin/main bfa87ee9c6217892df12d94f44d426dd79f7e7cd (before mosaic export).
//! These SHA-256 values came from a separately built copy of that original crate,
//! not from the production functions under test. f32 bytes use to_bits().to_le_bytes().
use super::{BlendMode, BlendParams};
use crate::{
    Ctl,
    project::{pipeline, session::Session},
    readers,
    source::Source,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[path = "canvas_fixture.rs"]
mod fixture;
#[path = "reference/mod.rs"]
mod reference;

const ORIGINAL: &[(&str, &str, &str, &str)] = &[
    (
        "default",
        "540c4c0a568267cc5e4455e5d29d3a480b8b4f6788245cdccdb0e4eb635ad515",
        "86ff0564d91b5fa63261f95b4de2d2a62ee1445c69cc34433580e2ec1d00ed74",
        "e2ffc9199a540350ba639bc9607c71449649a49d3d8648c092066b3b0034287b",
    ),
    (
        "joint_multiband",
        "ef19af5a78db6067fd02808fa5757aa796a05ba20627690cc209c42de41c85c7",
        "ff7983e7761e735c82be0fc29f5a126082794ed1c761b7130a537b93ad2e8501",
        "e2ffc9199a540350ba639bc9607c71449649a49d3d8648c092066b3b0034287b",
    ),
    (
        "tile_multiband",
        "01391395ac361c6bceb47250936dffb9c75b82a051a64afedc3cd32de6626f15",
        "5f7910c91500382ad2c2a126bdf804f0a002c23c6878b3ad42c89a05b20f7a23",
        "e2ffc9199a540350ba639bc9607c71449649a49d3d8648c092066b3b0034287b",
    ),
    (
        "joint_feather",
        "e95d44a4f24ae937be3b8f30068d217f2a2ad4c26243ae777c491da7b130fcdf",
        "830ac312b7d6b2a580d35deebb5644a783049e5a5fc0dba7e0a4f5114abd4872",
        "e2ffc9199a540350ba639bc9607c71449649a49d3d8648c092066b3b0034287b",
    ),
    (
        "gain_multiband",
        "1564ac30a429bfa7d1865e56cec6c9252243f435c36e8108a46c91defe038641",
        "75fb1df5665b62c678d8d660f08cc746312e91a9b77f5b7c0b2aba229c126064",
        "e2ffc9199a540350ba639bc9607c71449649a49d3d8648c092066b3b0034287b",
    ),
    (
        "none_multiband",
        "8d87faff188c12eda3fd8e1710f89fb49c3dc20b4c077373eb017f94495d2420",
        "14322ca5f15260c65c7fa4e9efd2ded9901a3d79f6a55a2fe585cb4c78485cd3",
        "e2ffc9199a540350ba639bc9607c71449649a49d3d8648c092066b3b0034287b",
    ),
    (
        "clahe_multiband",
        "d5717129caccc2ac03c621c1f29798c22efe14285cf14cc9532bcc142098350f",
        "ff7983e7761e735c82be0fc29f5a126082794ed1c761b7130a537b93ad2e8501",
        "e2ffc9199a540350ba639bc9607c71449649a49d3d8648c092066b3b0034287b",
    ),
];

fn f32_hash(values: &[f32]) -> String {
    let mut hash = Sha256::new();
    for value in values {
        hash.update(value.to_bits().to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}

#[test]
fn canvas_pipeline_matches_frozen_pre_export_blend_and_hashes() {
    let source: Arc<dyn Source> = Arc::new(fixture::package());
    let package = readers::open(source.clone()).unwrap();
    let cases = fixture::cases();
    assert_eq!(cases.len(), ORIGINAL.len());
    for ((name, params), &(expected_name, base_hash, linear_hash, coverage_hash)) in
        cases.into_iter().zip(ORIGINAL)
    {
        assert_eq!(name, expected_name);
        let mut session = Session::new("Synthetic canvas regression");
        session.set_memory_budget(usize::MAX);
        session
            .open_source(source.clone(), "", "2026-01-01T00:00:00Z")
            .unwrap();
        session
            .stitch(
                "area",
                &serde_json::to_string(&params).unwrap(),
                &Ctl::NONE,
                &|| 0.0,
            )
            .unwrap();
        let output = session.outputs.remove("area").unwrap();
        assert_eq!((output.linear.w, output.linear.h), (403, 336), "{name}");
        assert_eq!(
            crate::util::sha256_hex(&output.base),
            base_hash,
            "{name}: base differs from original pipeline"
        );
        assert_eq!(
            f32_hash(&output.linear.px),
            linear_hash,
            "{name}: pre-tone bits differ from original pipeline"
        );
        assert_eq!(
            f32_hash(&output.coverage),
            coverage_hash,
            "{name}: coverage differs from original pipeline"
        );

        // Feed the solved placement and corrected fields to a *frozen copy* of
        // the old blend. It owns its seam DP, tile pyramid, accumulation and
        // collapse; it shares no production blend/seam helpers with this test.
        let record = &session.project.as_ref().unwrap().areas[0];
        let tiles = package.tiles(0).unwrap();
        let input = pipeline::inputs(record, &tiles);
        let (corrected, photo) = crate::photometric::correct_ctl(
            &output.placement,
            &input.imgs,
            &params.photometric,
            &Ctl::NONE,
        )
        .unwrap();
        let fill = if params.photometric.background {
            photo.background_median
        } else {
            let mut sample: Vec<_> = corrected
                .iter()
                .flat_map(|tile| tile.px.iter().step_by(97).copied())
                .collect();
            sample.sort_by(f32::total_cmp);
            sample.get(sample.len() / 2).copied().unwrap_or(0.0)
        };
        let (old, coverage) = reference::blend(
            &output.placement,
            corrected,
            &params.blend,
            fill,
            &Ctl::NONE,
        )
        .unwrap();
        assert_eq!(old.px.len(), output.linear.px.len());
        for (pixel, (a, b)) in old.px.iter().zip(&output.linear.px).enumerate() {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "{name}: reference blend pixel {pixel}"
            );
        }
        assert_eq!(
            f32_hash(&old.px),
            linear_hash,
            "{name}: frozen reference changed"
        );
        assert_eq!(
            f32_hash(&coverage),
            coverage_hash,
            "{name}: frozen reference coverage changed"
        );
    }
}
