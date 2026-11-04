//! After stitching, every photometric control must change the mosaic without a re-stitch:
//! `Session::recompute` re-runs correction, blend and tone on the solved placement, and
//! `Session::preview` renders the same at a pyramid level for live feedback.

use std::path::PathBuf;
use std::sync::Arc;

use grainatlas_core::Ctl;
use grainatlas_core::project::AreaParams;
use grainatlas_core::project::session::Session;
use grainatlas_core::source::DirSource;

const AREA: &str = "001_HJ-GC1";

fn session() -> Session {
    let mut s = Session::new("t");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tm4000/Sweden_XS_150X");
    s.open_source(Arc::new(DirSource::new(root).unwrap()), "pkg", "now").unwrap();
    s
}

fn sha(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json).unwrap()["mosaic"]["sha256"].as_str().unwrap().to_string()
}

fn j(p: &AreaParams) -> String {
    serde_json::to_string(p).unwrap()
}

#[test]
fn background_and_gain_changes_recompute_without_restitch_and_match_a_full_stitch() {
    let mut s = session();
    let p0 = AreaParams::default();
    let h0 = sha(&s.stitch(AREA, &j(&p0), &Ctl::NONE, &|| 0.0).unwrap());

    let mut p1 = p0.clone();
    p1.photometric.background = false;
    let r1 = s.recompute(AREA, &j(&p1), &Ctl::NONE, &|| 0.0).unwrap();
    assert_ne!(sha(&r1), h0, "background leveling off must change the mosaic");
    let steps: Vec<String> = serde_json::from_str::<serde_json::Value>(&r1).unwrap()["timings"].as_array().unwrap().iter().map(|t| t[0].as_str().unwrap().to_string()).collect();
    assert!(!steps.contains(&"stitch".to_string()), "no re-stitch: {steps:?}");

    // The same parameters from scratch give the same pixels.
    let mut fresh = session();
    assert_eq!(sha(&fresh.stitch(AREA, &j(&p1), &Ctl::NONE, &|| 0.0).unwrap()), sha(&r1));

    // Gain model and blend mode too.
    let mut p2 = p1.clone();
    p2.photometric.model = grainatlas_core::photometric::GainModel::None;
    let r2 = s.recompute(AREA, &j(&p2), &Ctl::NONE, &|| 0.0).unwrap();
    assert_ne!(sha(&r2), sha(&r1));

    // Back to the defaults: back to the first mosaic.
    assert_eq!(sha(&s.recompute(AREA, &j(&p0), &Ctl::NONE, &|| 0.0).unwrap()), h0);
}

#[test]
fn tone_changes_retone_and_stitch_changes_restitch() {
    let mut s = session();
    let p0 = AreaParams::default();
    let h0 = sha(&s.stitch(AREA, &j(&p0), &Ctl::NONE, &|| 0.0).unwrap());
    let mut p1 = p0.clone();
    p1.tone.clahe = true;
    let r1 = s.recompute(AREA, &j(&p1), &Ctl::NONE, &|| 0.0).unwrap();
    assert_ne!(sha(&r1), h0);
    let steps = serde_json::from_str::<serde_json::Value>(&r1).unwrap()["timings"].to_string();
    assert!(steps.contains("tone") && !steps.contains("blend"), "{steps}");
    let mut p2 = p1.clone();
    p2.stitch.min_psr += 1.0;
    let r2 = s.recompute(AREA, &j(&p2), &Ctl::NONE, &|| 0.0).unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&r2).unwrap()["timings"].to_string().contains("stitch"));
}

#[test]
fn preview_is_the_mosaic_at_a_pyramid_level_and_follows_the_parameters() {
    let mut s = session();
    let p0 = AreaParams::default();
    let r = s.stitch(AREA, &j(&p0), &Ctl::NONE, &|| 0.0).unwrap();
    let m: serde_json::Value = serde_json::from_str(&r).unwrap();
    let (w, h) = (m["mosaic"]["w"].as_u64().unwrap() as i64, m["mosaic"]["h"].as_u64().unwrap() as i64);
    let dims = |b: &[u8]| (u32::from_le_bytes(b[0..4].try_into().unwrap()) as i64, u32::from_le_bytes(b[4..8].try_into().unwrap()) as i64);

    let a = s.preview(AREA, &j(&p0), 2).unwrap();
    let (pw, ph) = dims(&a);
    assert!((pw - w / 4).abs() <= 2 && (ph - h / 4).abs() <= 2, "{pw}x{ph} vs {w}x{h}");

    let mut p1 = p0.clone();
    p1.photometric.background = false;
    let b = s.preview(AREA, &j(&p1), 2).unwrap();
    assert_eq!(dims(&b), (pw, ph));
    let diff: u64 = a[8..].iter().zip(&b[8..]).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    assert!(diff > (pw * ph) as u64, "background off changes the preview (mean diff {})", diff as f64 / (pw * ph) as f64);

    let mut p2 = p0.clone();
    p2.tone.low_pct = 5.0;
    let c = s.preview(AREA, &j(&p2), 2).unwrap();
    assert_ne!(a, c, "tone-only previews follow the stretch");
}
