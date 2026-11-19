//! CLI replay and export refusal on a project generated entirely in a temporary directory.
#[path = "../../grainatlas-core/examples/support/mod.rs"] mod support;
use grainatlas_core::{layout::{AreaImage, sheets::{self, *}}, ownership::Ownership};
use std::{collections::BTreeMap, path::Path, process::{Command, Output}};

fn invoke(project: &Path, out: &Path, params: &Path, edits: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_grainatlas")).arg("sheets").arg(project).arg("--out").arg(out).arg("--params").arg(params).arg("--edits").arg(edits).output().unwrap()
}
#[test]
fn edits_replay_export_left_off_report_drops_and_block_invalid_output() {
    let temp=tempfile::tempdir().unwrap();let folder=temp.path().join("mount-a.grainatlas");std::fs::create_dir_all(folder.join("mosaic/a/0")).unwrap();
    let (_,mut project,px)=support::synthetic();
    let mosaic=grainatlas_core::mosaic::pyramid::MosaicJson {w:support::W,h:support::H,um_per_px:1.0,origin_um:[0.0,0.0],tile_size:512,format:"png".into(),levels:vec![],sha256:grainatlas_core::util::sha256_hex(&px)};
    project.areas[0].mosaic=Some(mosaic.clone());
    std::fs::write(folder.join("project.json"),project.to_json()).unwrap();std::fs::write(folder.join("mosaic/a/mosaic.json"),serde_json::to_vec(&mosaic).unwrap()).unwrap();
    std::fs::write(folder.join("mosaic/a/0/0_0.png"),grainatlas_core::image::encode_png_gray(support::W,support::H,&px)).unwrap();
    let gs:BTreeMap<String,Vec<grainatlas_core::segment::Grain>>=serde_json::from_value(project.extra["grainatlas_ui"]["grains"].clone()).unwrap();
    let p=SheetParams {packing:Packing::Packed,um_per_mm:8.0,enhance:Default::default(),..Default::default()};let params=temp.path().join("params.json");std::fs::write(&params,serde_json::to_vec(&p).unwrap()).unwrap();
    let rec=&project.areas[0];let areas=[SheetArea {rec,img:AreaImage { source_size_um: None, edits: None,id:&rec.id,name:&rec.name,sample:&rec.sample,w:support::W,h:support::H,px:&px,lut:std::array::from_fn(|i|i as u8),um_per_px:1.0}}];
    let plan=sheets::plan_sheets(&p,&areas,&gs,&Ownership::default(),&SheetMeta::default());let strip=plan.strips[0].key.clone();let file=temp.path().join("edits.json");
    let mut stored=PrintEdits {version:1,um_per_mm:plan.um_per_mm,params:p.clone(),edits:vec![StripEdit::Hide {strip:strip.clone()}]};std::fs::write(&file,serde_json::to_vec(&stored).unwrap()).unwrap();
    let out=temp.path().join("print-a");let result=invoke(&folder,&out,&params,&file);assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
    let reg:PrintRegistration=serde_json::from_slice(&std::fs::read(out.join("Synthetic mount_print_registration.json")).unwrap()).unwrap();assert_eq!(reg.layout,"edited");assert_eq!(reg.left_off.len(),2);assert!(reg.bands.is_empty());assert!(out.join("Synthetic mount_grain_sheets_tabloid.pdf").exists());assert!(out.join("Synthetic mount_sheet_stats.json").exists());
    let bare=temp.path().join("print-b");
    let result=Command::new(env!("CARGO_BIN_EXE_grainatlas")).arg("sheets").arg(&folder).arg("--out").arg(&bare).arg("--edits").arg(&file).output().unwrap();assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
    let replayed:PrintRegistration=serde_json::from_slice(&std::fs::read(bare.join("Synthetic mount_print_registration.json")).unwrap()).unwrap();assert_eq!(replayed.layout,"edited");assert_eq!(replayed.um_per_mm,8.0);assert_eq!(replayed.left_off.len(),2);
    stored.edits=vec![StripEdit::Move {strip,sheet:0,origin_mm:Some([0.0,0.0])}];std::fs::write(&file,serde_json::to_vec(&stored).unwrap()).unwrap();
    let blocked=temp.path().join("blocked");let result=invoke(&folder,&blocked,&params,&file);assert!(!result.status.success());assert!(String::from_utf8_lossy(&result.stderr).contains("cannot export sheets"));assert!(!blocked.exists());
    stored.params.um_per_mm=16.0;std::fs::write(&file,serde_json::to_vec(&stored).unwrap()).unwrap();let result=invoke(&folder,&temp.path().join("dropped"),&params,&file);assert!(result.status.success());assert!(String::from_utf8_lossy(&result.stderr).contains("edit 0 dropped: parameters changed (um_per_mm)"));
    stored.params=p.clone();stored.params.packing=Packing::Lines;stored.edits=vec![StripEdit::Label {strip:plan.strips[0].key.clone(),text:"area A1".into()}];
    std::fs::write(&params,serde_json::to_vec(&stored.params).unwrap()).unwrap();std::fs::write(&file,serde_json::to_vec(&stored).unwrap()).unwrap();
    let result=invoke(&folder,&temp.path().join("lines-edited"),&params,&file);assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));assert!(!String::from_utf8_lossy(&result.stderr).contains("line width"));
    stored.um_per_mm=16.0;std::fs::write(&file,serde_json::to_vec(&stored).unwrap()).unwrap();
    let result=invoke(&folder,&temp.path().join("scale-dropped"),&params,&file);assert!(result.status.success());let stderr=String::from_utf8_lossy(&result.stderr);assert!(stderr.contains("parameters changed (scale 16 → 8 µm/mm)"));assert!(stderr.contains("line width"));
    assert_eq!(std::fs::read(folder.join("project.json")).unwrap(),project.to_json().as_bytes());
}
