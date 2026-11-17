#[path = "../examples/support/mod.rs"]
mod support;
use grainatlas_core::{Ctl, photometric::ToneParams};
use serde_json::{Value, json};
use support::*;
fn json(v: &impl serde::Serialize) -> String { serde_json::to_string(v).unwrap() }

#[test]
fn published_source_retone_eviction_preview_and_bake() {
    let (mut session, project, pixels) = synthetic();
    let record = project.areas[0].clone();
    assert!(session.load_bundle_area("missing", 1, 1, vec![0]).is_err());
    assert!(session.load_bundle_area("a", 2, 2, vec![0]).is_err());
    let neutral = ToneParams::neutral();
    session.retone("a", &json(&neutral)).unwrap();
    assert_eq!(&*session.outputs["a"].base, &pixels);
    assert_eq!(session.project.as_ref().unwrap().areas[0], record);
    let mut tone = neutral.clone(); tone.low_pct = 20.0;
    session.retone("a", &json(&tone)).unwrap();
    let base = session.outputs["a"].base.clone();
    assert_ne!(&*base, &pixels);
    assert_eq!(session.project.as_ref().unwrap().areas[0], record);
    assert!(session.stitch("a", "", &Ctl::NONE, &|| 0.0).unwrap_err().to_string().contains("need the raw package"));
    let params = json(&record.params);
    assert!(session.recompute("a", &params, &Ctl::NONE, &|| 0.0).unwrap_err().to_string().contains("need the raw package"));
    session.evict_to(0);
    assert!(session.outputs["a"].linear.px.is_empty());
    assert_eq!(session.histogram("a").unwrap().iter().sum::<u32>(), (W*H) as u32);
    let calls = session.rebuild_heavy_calls;
    let mut preview_params = record.params.clone(); preview_params.tone = neutral.clone();
    let preview = session.preview("a", &json(&preview_params), 1).unwrap();
    assert_eq!(preview.len(), 8 + W*H/4);
    assert_eq!(preview[8], ((pixels[0] as u32 + pixels[1] as u32 + pixels[W] as u32 + pixels[W+1] as u32) as f32 / 4.0).round() as u8);
    assert_eq!(session.rebuild_heavy_calls, calls, "preview must not rehydrate full resolution");
    session.retone("a", &json(&tone)).unwrap();
    assert_eq!(session.rebuild_heavy_calls, calls+1);
    assert_eq!(&*session.outputs["a"].base, &*base);
    let auto: grainatlas_core::project::AreaParams = serde_json::from_str(&session.auto_params("a", &params).unwrap()).unwrap();
    assert_eq!(auto.stitch, record.params.stitch);
    assert_eq!(auto.photometric, record.params.photometric);
    assert_eq!(auto.blend, record.params.blend);
    let mut display = neutral.clone(); display.gamma = 1.6;
    let lut = grainatlas_core::photometric::tone::lut(&display);
    let expected: Vec<_> = base.iter().map(|&v| lut[v as usize]).collect();
    let mosaic: Value = serde_json::from_str(&session.bake_bundle_area("a", &json(&display)).unwrap()).unwrap();
    assert_eq!(mosaic["sha256"], grainatlas_core::util::sha256_hex(&expected));
    session.evict_to(0);
    session.retone("a", &json(&neutral)).unwrap();
    assert_eq!(&*session.outputs["a"].base, &expected);
    assert_eq!(session.project.as_ref().unwrap().areas[0], record);
}

#[test]
fn bundle_export_uses_only_overlay_and_strips_it_without_mutating_project() {
    let (session, mut project, pixels) = synthetic();
    let digest = session.package_digest().unwrap();
    assert_eq!(digest.sha256, "d".repeat(64)); assert_eq!(digest.files, 1);
    let mut display = ToneParams::neutral(); display.gamma = 2.0;
    project.extra.get_mut("grainatlas_ui").unwrap()["bundle_tone"] = json!({"a":{"curve":ToneParams::neutral(),"display":display,"on_sha256":"source"}});
    let files = session.atlas_files(&project.to_json(), "").unwrap();
    let saved: Value = serde_json::from_slice(&files.iter().find(|(p,_)| p=="project.json").unwrap().1).unwrap();
    assert!(saved["grainatlas_ui"].get("bundle_tone").is_none());
    assert!(project.extra["grainatlas_ui"].get("bundle_tone").is_some());
    assert_eq!(serde_json::from_value::<ToneParams>(saved["areas"][0]["params"]["tone"].clone()).unwrap(), project.areas[0].params.tone);
    let mut expected_spots = project.extra["grainatlas_ui"]["spots"].clone();
    grainatlas_core::atlas::canonical(&mut expected_spots);
    assert_eq!(saved["grainatlas_ui"]["spots"], expected_spots);
    let tile = &files.iter().find(|(p,_)| p=="pyramid/a/0/0_0.webp").unwrap().1;
    let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(tile)).unwrap();
    let mut rgba = vec![0; decoder.output_buffer_size().unwrap()]; decoder.read_image(&mut rgba).unwrap();
    let lut = grainatlas_core::photometric::tone::lut(&display);
    for y in 0..256 { for x in 0..256 { assert_eq!(rgba[(y*256+x)*3], lut[pixels[y*W+x] as usize]); } }
    let identity_files = session.atlas_files(&support::synthetic().1.to_json(), "").unwrap();
    assert_ne!(files, identity_files);
}

#[test]
fn bundle_prints_preserve_annotations_and_work_after_eviction() {
    let (mut session, project, _) = synthetic();
    session.evict_to(0);
    let ui = &project.extra["grainatlas_ui"];
    let tones = json!({"a":ToneParams::neutral()});
    let sheets: Value = serde_json::from_str(&session.print_sheets("", &json(&ui["grains"]), &json(&tones)).unwrap()).unwrap();
    let pages = sheets["pages"].as_array().unwrap().iter().map(|p|p.as_str().unwrap()).collect::<String>();
    assert!(!pages.is_empty());
    // Sheets are for spotting by hand at the laser: no digital spots are drawn on them.
    for spot in ui["spots"].as_array().unwrap() { assert!(!pages.contains(spot["id"].as_str().unwrap())); }
    let registration = json(&sheets["registration"]);
    for grain in ui["grains"]["a"].as_array().unwrap() { assert!(registration.contains(grain["id"].as_str().unwrap())); }
    let layout: Value = serde_json::from_str(&session.layout(&json(&grainatlas_core::layout::LayoutParams::default()), &json(&ui["grains"]), &json(&ui["spots"]), &json(&ui["labels"]), &json(&tones)).unwrap()).unwrap();
    let pages = json(&layout["pages"]);
    for spot in ui["spots"].as_array().unwrap() { assert!(pages.contains(spot["name"].as_str().unwrap())); }
    assert_eq!(session.project.as_ref().unwrap(), &project);
}

#[test]
fn raw_bundle_ignores_overlay_and_keeps_legacy_bytes() {
    let (_, mut project, pixels) = synthetic();
    let digest = grainatlas_core::atlas::PackageDigest { sha256: "d".repeat(64), files: 1 };
    let build = |p: &grainatlas_core::project::Project| {
        grainatlas_core::atlas::build(p, &[grainatlas_core::atlas::AreaPixels { rec: &p.areas[0], w: W, h: H, base: &pixels, bundle_backed: false }], &digest, &Default::default()).unwrap()
    };
    let before = build(&project);
    project.extra.get_mut("grainatlas_ui").unwrap()["bundle_tone"] = json!({"a":{"display":{"gamma":2.0}}});
    assert_eq!(build(&project), before);
    // Golden from the pre-change atlas builder, with this invented project and fixed ids.
    // atlas.json hashes every other file, so this checks the complete legacy bundle bytes.
    let atlas = &before.iter().find(|(p,_)| p=="atlas.json").unwrap().1;
    // Normalise only the serialized crate version to the golden's 0.1.1, preserving all other bytes.
    let atlas = std::str::from_utf8(atlas).unwrap();
    let version = format!("\"grainatlas_version\": \"{}\"", env!("CARGO_PKG_VERSION"));
    assert_eq!(atlas.matches(&version).count(), 1);
    let atlas = atlas.replace(&version, "\"grainatlas_version\": \"0.1.1\"");
    assert_eq!(grainatlas_core::util::sha256_hex(atlas.as_bytes()), "6ee95ef7649b3592135041413854cdb5718dd20bcbeb3d11f7836c83caf0dbb2");
}
