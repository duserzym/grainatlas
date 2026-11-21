//! Every pixel and record is invented, and every filesystem write is temporary.
#[allow(dead_code)]
#[path = "../../grainatlas-core/examples/support/mod.rs"]
mod support;
use serde_json::{Value, json};
use std::{path::Path, process::Command};

fn run(folder: &Path, out: &Path, args: &[&str]) -> Value {
    let result = Command::new(env!("CARGO_BIN_EXE_grainatlas"))
        .arg("sheets")
        .arg(folder)
        .arg("--out")
        .arg(out)
        .arg("--svg")
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let file = std::fs::read_dir(out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("stats.json")
        })
        .unwrap();
    serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap()
}
#[test]
fn stored_spots_size_and_pins_export_read_only_in_all_modes() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("mount-a.grainatlas");
    std::fs::create_dir_all(folder.join("mosaic/a/0")).unwrap();
    let (_, mut project, px) = support::synthetic();
    let mosaic = grainatlas_core::mosaic::pyramid::MosaicJson {
        w: support::W,
        h: support::H,
        um_per_px: 1.0,
        origin_um: [0.0, 0.0],
        tile_size: 512,
        format: "png".into(),
        levels: vec![],
        sha256: grainatlas_core::util::sha256_hex(&px),
    };
    project.areas[0].mosaic = Some(mosaic.clone());
    project.extra.get_mut("grainatlas_ui").unwrap()["print_label_size_mm"] = json!(3.0);
    let spots = project.extra["grainatlas_ui"]["spots"].clone();
    assert!(!spots.as_array().unwrap().is_empty());
    std::fs::write(
        folder.join("mosaic/a/mosaic.json"),
        serde_json::to_vec(&mosaic).unwrap(),
    )
    .unwrap();
    std::fs::write(
        folder.join("mosaic/a/0/0_0.png"),
        grainatlas_core::image::encode_png_gray(support::W, support::H, &px),
    )
    .unwrap();
    for mode in ["lines", "packed", "edits", "whole"] {
        std::fs::write(folder.join("project.json"), project.to_json()).unwrap();
        let source = std::fs::read(folder.join("project.json")).unwrap();
        let out = temp.path().join(format!("print-{mode}"));
        let edits = temp.path().join("edits.json");
        std::fs::write(
            &edits,
            serde_json::to_vec(
                &json!({"version":1,"params":{"um_per_mm":8.0},"um_per_mm":8.0,"edits":[]}),
            )
            .unwrap(),
        )
        .unwrap();
        let edits_path = edits.to_str().unwrap();
        let args = match mode {
            "whole" => vec!["--layout", "whole", "--scale", "8"],
            "edits" => vec!["--edits", edits_path],
            _ => vec!["--packing", mode, "--scale", "8"],
        };
        let off = run(&folder, &temp.path().join(format!("off-{mode}")), &args);
        assert!(off.get("spot_labels").is_none());
        let mut on_args = args.clone();
        on_args.push("--spots");
        let stats = run(&folder, &out, &on_args);
        let labels = stats["spot_labels"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r["labels"].as_array().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), spots.as_array().unwrap().len());
        for spot in spots.as_array().unwrap() {
            assert!(
                labels
                    .iter()
                    .any(|l| l["spot_id"] == spot["id"] && l["name"] == spot["name"])
            );
        }
        let label = labels[0];
        let size = grainatlas_core::layout::labels::text_size(label["name"].as_str().unwrap(), 3.0);
        let rect = &label["box_mm"];
        assert!((rect[3].as_f64().unwrap() - rect[1].as_f64().unwrap() - size[1]).abs() < 1e-8);
        let key = format!(
            "{}/{}",
            label["spot_id"].as_str().unwrap(),
            label["occurrence"].as_str().unwrap()
        );
        let offset = [
            rect[0].as_f64().unwrap() - label["anchor_mm"][0].as_f64().unwrap(),
            rect[1].as_f64().unwrap() - label["anchor_mm"][1].as_f64().unwrap(),
        ];
        let mut pinned = project.clone();
        pinned.extra.get_mut("grainatlas_ui").unwrap()["print_label_pins"] = json!({key:offset});
        std::fs::write(folder.join("project.json"), pinned.to_json()).unwrap();
        let pinned_source = std::fs::read(folder.join("project.json")).unwrap();
        let stats = run(
            &folder,
            &temp.path().join(format!("pinned-{mode}")),
            &on_args,
        );
        let after = stats["spot_labels"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r["labels"].as_array().unwrap())
            .find(|l| l["pinned"] == true)
            .unwrap();
        assert_eq!(after["box_mm"], label["box_mm"]);
        assert_eq!(
            std::fs::read(folder.join("project.json")).unwrap(),
            pinned_source
        );
        on_args.extend(["--label-size", "2"]);
        let stats = run(
            &folder,
            &temp.path().join(format!("override-{mode}")),
            &on_args,
        );
        let after = &stats["spot_labels"][0]["labels"][0];
        let size = grainatlas_core::layout::labels::text_size(after["name"].as_str().unwrap(), 2.0);
        assert!(
            (after["box_mm"][3].as_f64().unwrap() - after["box_mm"][1].as_f64().unwrap() - size[1])
                .abs()
                < 1e-8
        );
        assert_eq!(
            std::fs::read(folder.join("project.json")).unwrap(),
            pinned_source
        );
        std::fs::write(folder.join("project.json"), &source).unwrap();
    }
    let mut names = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["mosaic", "project.json"]);
}
