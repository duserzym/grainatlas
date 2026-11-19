#[path = "../examples/support/mod.rs"]
mod support;
use serde_json::{Value, json};

#[test]
fn staged_full_result_is_byte_identical_and_preview_keeps_geometry() {
    for strip in [false, true] {
        let (session, project) = support::print_edit();
        let grains = project.extra["grainatlas_ui"]["grains"].to_string();
        let tone = project.extra["grainatlas_ui"]["tone"].to_string();
        let params = json!({"landscape":true,"um_per_mm":2.0,"enhance":{"denoise":true,"sharpen":true,"strip":strip},"printed":"fixed"}).to_string();
        let before = session.print_sheets(&params, &grains, &tone).unwrap();
        let preview: Value = serde_json::from_str(
            &session
                .print_sheets_preview(&params, &grains, &tone, "")
                .unwrap(),
        )
        .unwrap();
        let original: Value = serde_json::from_str(&before).unwrap();
        assert_eq!(preview["plan"], original["plan"]);
        let rects = |v: &Value| {
            v["registration"]["bands"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| {
                    (
                        b["sheet"].clone(),
                        b["mosaic_rect_um"].clone(),
                        b["sheet_rect_mm"].clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(rects(&preview), rects(&original));
        assert_ne!(preview["pages"], original["pages"]);
        let id = preview["job_id"].as_u64().unwrap() as usize;
        assert!(session.print_task(id, "pdf_begin", 0).is_err());
        for page in 0..preview["pages"].as_array().unwrap().len() {
            let status: Value =
                serde_json::from_slice(&session.print_task(id, "quality", page).unwrap()).unwrap();
            assert!(status["resident_bytes"].as_u64().unwrap() > 0);
            if status["ready"] == true {
                let bytes: usize = original["pages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap().len())
                    .sum();
                assert_eq!(status["resident_bytes"], bytes);
            }
        }
        let full = session.print_task(id, "full", 0).unwrap();
        assert_eq!(
            before.as_bytes(),
            full,
            "every SVG, PNG byte, statistic and plan must match"
        );
        session.print_task(id, "pdf_begin", 0).unwrap();
        assert!(session.print_task(id, "pdf_finish", 0).is_err());
        assert!(session.print_task(id, "pdf_page", 0).is_err());
        for page in 0..preview["pages"].as_array().unwrap().len() {
            session.print_task(id, "pdf_prepare", page).unwrap();
        }
        for page in 0..preview["pages"].as_array().unwrap().len() {
            session.print_task(id, "pdf_page", page).unwrap();
        }
        let pdf = session.print_task(id, "pdf_finish", 0).unwrap();
        let pages: Vec<String> = serde_json::from_value(original["pages"].clone()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.windows(14).any(|v| v == b"/EmbeddedFiles"));
        assert!(pdf.windows(23).any(|v| v == b"print_registration.json"));
        let count = format!("/Count {}", pages.len());
        assert!(pdf.windows(count.len()).any(|v| v == count.as_bytes()));
        // Packing and strip edits use the same enhanced pixels.
        let plan = original["plan"].to_string();
        let edits = json!([{"op":"add_sheet","at":1}]).to_string();
        let edited = session
            .print_plan_render(&plan, &edits, &grains, &tone)
            .unwrap();
        let preview: Value = serde_json::from_str(
            &session
                .print_plan_preview(&plan, &edits, &grains, &tone)
                .unwrap(),
        )
        .unwrap();
        let id = preview["job_id"].as_u64().unwrap() as usize;
        for page in 0..preview["pages"].as_array().unwrap().len() {
            session.print_task(id, "quality", page).unwrap();
        }
        assert_eq!(
            edited.as_bytes(),
            session.print_task(id, "full", 0).unwrap()
        );
    }
}
#[test]
fn replaced_cancelled_and_changed_sources_are_rejected() {
    let (mut session, project, pixels) = support::synthetic();
    let grains = project.extra["grainatlas_ui"]["grains"].to_string();
    let make = |session: &grainatlas_core::project::session::Session| -> usize {
        let v: Value = serde_json::from_str(
            &session
                .print_sheets_preview("{}", &grains, "{}", "")
                .unwrap(),
        )
        .unwrap();
        v["job_id"].as_u64().unwrap() as usize
    };
    let first = make(&session);
    let second = make(&session);
    assert!(session.print_task(first, "quality", 0).is_err());
    assert!(session.print_task(second, "quality", 1).is_err());
    session.print_task(second, "cancel", 0).unwrap();
    assert!(session.print_task(second, "quality", 0).is_err());
    let third = make(&session);
    session
        .load_bundle_area("a", support::W, support::H, pixels)
        .unwrap();
    assert!(
        session
            .print_task(third, "quality", 0)
            .unwrap_err()
            .to_string()
            .contains("source changed")
    );
    // Cleanup remains available after a source revision makes export stale.
    session.print_task(third, "pdf_abort", 0).unwrap();
}
#[test]
fn whole_and_older_layout_previews_export_the_original_full_pixels() {
    let (session, project, _) = support::synthetic();
    let ui = &project.extra["grainatlas_ui"];
    let (grains, spots, labels, tone) = (
        ui["grains"].to_string(),
        ui["spots"].to_string(),
        ui["labels"].to_string(),
        ui["tone"].to_string(),
    );
    let params = json!({"area_id":"a","printed":"fixed"}).to_string();
    let before = session.print_whole(&params, &tone).unwrap();
    let preview: Value =
        serde_json::from_str(&session.print_whole_preview(&params, &tone).unwrap()).unwrap();
    let id = preview["job_id"].as_u64().unwrap() as usize;
    session.print_task(id, "quality", 0).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&before).unwrap(),
        serde_json::from_slice::<Value>(&session.print_task(id, "full", 0).unwrap()).unwrap()
    );
    let params = json!({"items":"grains","show_spots":true,"show_outlines":true}).to_string();
    let before = session
        .layout(&params, &grains, &spots, &labels, &tone)
        .unwrap();
    let preview: Value = serde_json::from_str(
        &session
            .layout_preview(&params, &grains, &spots, &labels, &tone)
            .unwrap(),
    )
    .unwrap();
    let id = preview["job_id"].as_u64().unwrap() as usize;
    for page in 0..preview["pages"].as_array().unwrap().len() {
        session.print_task(id, "quality", page).unwrap();
    }
    assert_eq!(
        serde_json::from_str::<Value>(&before).unwrap(),
        serde_json::from_slice::<Value>(&session.print_task(id, "full", 0).unwrap()).unwrap()
    );
}
