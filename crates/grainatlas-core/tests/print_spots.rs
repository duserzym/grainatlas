//! All records and image pixels are invented; no package fixture is read.
#[allow(dead_code)]
#[path = "../examples/support/mod.rs"]
mod support;
use grainatlas_core::layout::labels::Options;
use grainatlas_core::layout::sheets::SheetPlan;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn assert_report_bodies(output: &Value) {
    let registration = &output["registration"];
    for report in registration["spot_labels"].as_array().into_iter().flatten() {
        let body = registration["sheet_bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["sheet"] == report["sheet"])
            .unwrap();
        let b: [f64; 4] = serde_json::from_value(body["body_rect_mm"].clone()).unwrap();
        for label in report["labels"].as_array().unwrap() {
            let r: [f64; 4] = serde_json::from_value(label["box_mm"].clone()).unwrap();
            assert!(
                r[0] >= b[0] && r[1] >= b[1] && r[2] <= b[2] && r[3] <= b[3],
                "{label}"
            );
            for point in label["leader"].as_array().unwrap() {
                let p: [f64; 2] = serde_json::from_value(point.clone()).unwrap();
                assert!(p[0] >= b[0] && p[1] >= b[1] && p[0] <= b[2] && p[1] <= b[3]);
            }
        }
    }
}

fn params() -> Value {
    json!({"paper":"tabloid","landscape":true,"um_per_mm":4.0,"printed":"2026-01-01 00:00 UTC","enhance":{"denoise":false,"sharpen":false}})
}
#[test]
fn spots_off_svg_matches_original_revision_bytes() {
    let (session, project) = support::print_edit();
    let ui = &project.extra["grainatlas_ui"];
    let output: Value = serde_json::from_str(
        &session
            .print_sheets(
                &params().to_string(),
                &ui["grains"].to_string(),
                &ui["tone"].to_string(),
            )
            .unwrap(),
    )
    .unwrap();
    let mut hash = Sha256::new();
    for page in output["pages"].as_array().unwrap() {
        hash.update(page.as_str().unwrap().as_bytes());
    }
    // Generated from the same existing synthetic print_edit fixture at cc57ab0.
    assert_eq!(
        format!("{:x}", hash.finalize()),
        "c40e97920e1419c210077a3a297561052a23a914548d76195f15a479add6378e"
    );
    assert!(
        output["pages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| !p.as_str().unwrap().contains("@font-face"))
    );
    assert!(output["registration"].get("spot_labels").is_none());
    assert!(output["stats"].get("spot_labels").is_none());
}
fn spots() -> Value {
    json!([
        {"id":"70000000-0000-4000-8000-000000000001","area_id":"a","name":"MNT-A_12 <&\" Ω","x_um":200,"y_um":200,"diameter_um":20},
        {"id":"70000000-0000-4000-8000-000000000002","area_id":"a","name":"Z-002","kind":"line","x_um":400,"y_um":200,"x2_um":420,"y2_um":210,"diameter_um":25},
        {"id":"70000000-0000-4000-8000-000000000003","area_id":"a","name":"Z-003","kind":"raster","x_um":600,"y_um":200,"x2_um":630,"y2_um":220,"diameter_um":30}
    ])
}
#[test]
fn every_kind_uses_the_shared_plan_and_whole_pipeline() {
    let (session, project) = support::print_edit();
    let ui = &project.extra["grainatlas_ui"];
    for packing in ["lines", "packed"] {
        let mut p = params();
        p["packing"] = json!(packing);
        let output: Value = serde_json::from_str(
            &session
                .print_sheets(&p.to_string(), &ui["grains"].to_string(), "{}")
                .unwrap(),
        )
        .unwrap();
        let mut plan: SheetPlan = serde_json::from_value(output["plan"].clone()).unwrap();
        plan.print_spots = Some(serde_json::from_value(spots()).unwrap());
        plan.print_labels = Some(Options::default());
        let rendered: Value = serde_json::from_str(
            &session
                .print_plan_render(
                    &serde_json::to_string(&plan).unwrap(),
                    "[]",
                    &ui["grains"].to_string(),
                    "{}",
                )
                .unwrap(),
        )
        .unwrap();
        assert_report_bodies(&rendered);
        let mut names = Vec::new();
        let mut markers = 0;
        for page in rendered["pages"].as_array().unwrap() {
            let svg = page.as_str().unwrap();
            let doc = roxmltree::Document::parse(svg).unwrap();
            let has_labels = doc
                .descendants()
                .any(|n| n.attribute("data-spot-label").is_some());
            let needs_fallback = doc.descendants().any(|n| {
                n.attribute("data-spot-label").is_some() && n.text().unwrap_or("").contains('Ω')
            });
            assert_eq!(
                svg.matches("@font-face").count(),
                usize::from(has_labels) + usize::from(needs_fallback)
            );
            if has_labels {
                assert!(svg.contains("data:font/ttf;base64,"));
                let pdf = grainatlas_core::layout::pdf::pdf_from_svgs(&[svg.to_string()]).unwrap();
                assert!(String::from_utf8_lossy(&pdf).contains("IBMPlexSansCond-Medium"));
            }
            for n in doc.descendants() {
                if n.attribute("data-spot-marker").is_some() {
                    markers += 1;
                }
                if n.attribute("data-spot-label").is_some() {
                    names.push(n.text().unwrap().to_string());
                    assert_eq!(
                        n.attribute("font-family"),
                        Some("IBM Plex Sans Condensed, DejaVu Sans")
                    );
                }
            }
        }
        names.sort();
        assert_eq!(markers, 3);
        assert_eq!(names, vec!["MNT-A_12 <&\" Ω", "Z-002", "Z-003"]);
        assert!(
            rendered["registration"]["spot_labels"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["counts"]["still_overlapping"] == 0)
        );
        assert_eq!(
            rendered["registration"]["bands"],
            output["registration"]["bands"]
        );
    }
    let p = json!({"paper":"tabloid","area_id":"a","grid":true,"print_spots":spots(),"print_labels":{"size_mm":2.4,"pins":{}},"print_grains":ui["grains"]["a"]});
    let output: Value =
        serde_json::from_str(&session.print_whole(&p.to_string(), "{}").unwrap()).unwrap();
    assert_report_bodies(&output);
    assert_eq!(
        output["registration"]["spot_labels"][0]["counts"]["anchors"],
        3
    );
    let doc = roxmltree::Document::parse(output["pages"][0].as_str().unwrap()).unwrap();
    assert_eq!(
        doc.descendants()
            .filter(|n| n.attribute("data-spot-label").is_some())
            .count(),
        3
    );
}

#[test]
fn supervisor_twelve_spots_size_change_and_pin_on_real_sheet_geometry() {
    let (session, project) = support::print_edit();
    let ui = &project.extra["grainatlas_ui"];
    let spots = browser_spots();
    for packing in ["lines", "packed"] {
        let mut params = browser_sheet_params();
        params["packing"] = json!(packing);
        let output: Value = serde_json::from_str(
            &session
                .print_sheets(&params.to_string(), &ui["grains"].to_string(), "{}")
                .unwrap(),
        )
        .unwrap();
        let mut plan: SheetPlan = serde_json::from_value(output["plan"].clone()).unwrap();
        plan.print_spots = Some(serde_json::from_value(json!(spots)).unwrap());
        let render = |plan: &SheetPlan| -> Vec<grainatlas_core::layout::labels::Report> {
            let result: Value = serde_json::from_str(
                &session
                    .print_plan_render(
                        &serde_json::to_string(plan).unwrap(),
                        "[]",
                        &ui["grains"].to_string(),
                        &ui["tone"].to_string(),
                    )
                    .unwrap(),
            )
            .unwrap();
            assert_report_bodies(&result);
            let preview: Value = serde_json::from_str(
                &session
                    .print_plan_preview(
                        &serde_json::to_string(plan).unwrap(),
                        "[]",
                        &ui["grains"].to_string(),
                        &ui["tone"].to_string(),
                    )
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                result["registration"]["spot_labels"], preview["registration"]["spot_labels"],
                "native render and wasm-facing preview JSON must place the same labels"
            );
            serde_json::from_value(result["stats"]["spot_labels"].clone()).unwrap()
        };
        plan.print_labels = Some(Options::default());
        assert!(
            render(&plan)
                .iter()
                .all(|r| r.counts.still_overlapping == 0)
        );
        plan.print_labels.as_mut().unwrap().size_mm = 3.0;
        let before = render(&plan);
        assert!(
            before.iter().all(|r| r.counts.still_overlapping == 0),
            "{packing}: size change {before:?}"
        );
        let first = before.iter().flat_map(|r| &r.labels).next().unwrap();
        // PanZoom.fit: 1280 - 21*16 sidebar - 3*8 spacing = 920 CSS px.
        // Width limits landscape tabloid at 800 px height; fit has a 0.94 inset.
        plan.print_labels.as_mut().unwrap().pins.insert(
            format!("{}/{}", first.spot_id, first.occurrence),
            [
                first.box_mm[0] - first.anchor_mm[0] + 15.0 / (920.0 * 0.94 / 431.8),
                first.box_mm[1] - first.anchor_mm[1] + 12.0 / (920.0 * 0.94 / 431.8),
            ],
        );
        let after = render(&plan);
        eprintln!(
            "browser {packing} pin: {:?}",
            after.iter().map(|r| &r.counts).collect::<Vec<_>>()
        );
        assert!(
            after.iter().all(|r| r.counts.still_overlapping == 0),
            "{packing}: pin {after:?}"
        );
        assert_eq!(after.iter().map(|r| r.counts.pinned).sum::<usize>(), 1);
        assert_eq!(
            after
                .iter()
                .map(|r| r.counts.pinned_overlapping)
                .sum::<usize>(),
            0
        );
        let last = before.iter().flat_map(|r| &r.labels).last().unwrap();
        let pins = &mut plan.print_labels.as_mut().unwrap().pins;
        pins.clear();
        pins.insert(format!("{}/{}", last.spot_id, last.occurrence), [0.0, 0.0]);
        let deliberate = render(&plan);
        assert!(
            deliberate.iter().all(|r| r.counts.still_overlapping == 0),
            "{packing}: {deliberate:?}"
        );
        assert_eq!(
            deliberate
                .iter()
                .map(|r| r.counts.pinned_overlapping)
                .sum::<usize>(),
            1
        );
    }
}

// Exactly the twelve server records from ui/e2e/print-spots.spec.ts.
fn browser_spots() -> Vec<Value> {
    let spots: Vec<Value> = (0..12).map(|i| json!({
        "id":format!("70000000-0000-4000-8000-{:012}",i+1),"area_id":"a",
        "name":if i==0 {"MNT-A_12 <&\" Ω".to_string()} else {format!("Z-{:03}",i+1)},
        "sample":"MNT-A","grain_id":null,"grain_number":null,"n":i+1,
        "x_um":174+i%4*32,"y_um":174+i/4*32,"diameter_um":20,
        "comment":"","created":"","created_by":"",
        "kind":if i==10 {"line"} else if i==11 {"raster"} else {"spot"},
        "x2_um":if i>=10 {Some(184+i%4*32)} else {None},"y2_um":if i>=10 {Some(184+i/4*32)} else {None}
    })).collect();
    spots
}

#[test]
fn browser_whole_auto_grid_twelve_spots() {
    let (session, project) = support::print_edit();
    let ui = &project.extra["grainatlas_ui"];
    // wholePrintOptions + Layout.svelte, independent of the grain-sheet scale.
    let p = json!({"paper":"tabloid","margin_mm":8,"title":"","lab":"",
        "registration_file":"mount A_print_registration.json","project":"mount A.grainatlas",
        "printed":"2026-01-01 00:00 UTC","area_id":"a","orientation":"auto",
        "grid":true,"um_per_mm":"auto","dpi":600,"name":"",
        "print_spots":browser_spots(),"print_labels":{"size_mm":"auto","pins":{}},
        "print_grains":ui["grains"]["a"]});
    let output: Value = serde_json::from_str(
        &session
            .print_whole(&p.to_string(), &ui["tone"].to_string())
            .unwrap(),
    )
    .unwrap();
    assert_report_bodies(&output);
    let preview: Value = serde_json::from_str(
        &session
            .print_whole_preview(&p.to_string(), &ui["tone"].to_string())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        output["registration"]["spot_labels"], preview["registration"]["spot_labels"],
        "native render and wasm-facing preview JSON must place the same labels"
    );
    let counts = &output["registration"]["spot_labels"][0]["counts"];
    eprintln!("browser whole: {counts}");
    assert_eq!(counts["still_overlapping"], 0, "{counts}");
}

// Layout.svelte's defaults after the spec sets scale=4 and disables enhancement.
fn browser_sheet_params() -> Value {
    json!({"paper":"tabloid","landscape":true,"packing":"lines","margin_mm":8,
        "um_per_mm":4,"pad_um":22,"gap_break_um":420,"context_um":0,"min_group":3,
        "min_cluster_grains":5,"overlap_um":0,"min_grain_um":35,"max_grain_um":0,
        "spot_sizes_um":[15,20,25,30,35,50],"title":"","lab":"",
        "registration_file":"mount A_print_registration.json","project":"mount A.grainatlas",
        "printed":"2026-01-01 00:00 UTC",
        "enhance":{"denoise":false,"denoise_h":0.7,"sharpen":false,"sharpen_radius":1.6,
            "sharpen_amount":0.55,"strip":false,"strip_grey":22,"strip_margin_um":6}})
}
