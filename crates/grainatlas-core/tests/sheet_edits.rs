//! Entirely generated grains, pixels and stable ids for the strip-edit contract.
#[path = "../examples/support/mod.rs"] mod support;
use std::collections::{BTreeMap, BTreeSet};
use grainatlas_core::{layout::{AreaImage, sheets::{self, *}, sheets::edit::Axis}, ownership::Ownership, project::AreaRecord, segment::Grain};
use serde_json::json;

struct Fixture { rec: AreaRecord, px: Vec<u8>, gs: BTreeMap<String, Vec<Grain>>, params: SheetParams }
impl Fixture {
    fn new() -> Self {
        let (_, project, _) = support::synthetic();
        let mut rec = project.areas[0].clone(); rec.id = "A1".into(); rec.name = "zr-1".into(); rec.sample = "zr-1".into(); rec.mosaic = None; rec.placement = None;
        let gs = (0..8).map(|i| {
            let x = 80.0 + i as f64 * 160.0;
            serde_json::from_value(json!({ "id": format!("10000000-0000-4000-8000-{:012}", i+1), "number": i+1,
                "polygon_um": [[x-20.0,80.0],[x+20.0,80.0],[x+20.0,140.0],[x-20.0,140.0]],
                "bbox_um": [x-20.0,80.0,x+20.0,140.0], "centroid_um": [x,110.0], "area_um2":2400.0,
                "eq_diameter_um":55.0,"major_um":60.0,"minor_um":40.0,"orientation_deg":0.0,"mean_intensity":0.5,"manual":true,"scale_bar_um":20.0 })).unwrap()
        }).collect();
        Self { rec, px: vec![110; 1600*300], gs: BTreeMap::from([("A1".into(), gs)]), params: SheetParams { packing: Packing::Packed, um_per_mm: 8.0, context_um: 180.0, pad_um: 10.0, min_cluster_grains: 0, min_cut_grains: 1, enhance: Default::default(), ..Default::default() } }
    }
    fn areas(&self) -> Vec<SheetArea<'_>> { vec![SheetArea { rec: &self.rec, img: AreaImage { id: "A1", name: "zr-1", sample: "zr-1", w:1600, h:300, px:&self.px, lut:std::array::from_fn(|i| i as u8), um_per_px:1.0 } }] }
    fn plan(&self) -> SheetPlan { plan_sheets(&self.params, &self.areas(), &self.gs, &Ownership::default(), &SheetMeta { package:"mount A".into(), ..Default::default() }) }
    fn apply(&self, plan: &SheetPlan, edits: &[StripEdit]) -> EditedPlan { apply_edits(plan, edits, &self.areas(), &self.gs) }
    fn render(&self, p: &SheetPlan) -> SheetResult { render_plan(p, &self.areas(), &self.gs, &Ownership::default()) }
    fn check(&self, p: &SheetPlan) {
        let r = self.render(p);
        for b in &r.registration.bands {
            for d in 0..2 { assert!(((b.sheet_rect_mm[d+2] - b.sheet_rect_mm[d]) * p.um_per_mm - (b.mosaic_rect_um[d+2] - b.mosaic_rect_um[d])).abs() < 1e-8); }
        }
        for (area, ids) in &p.targets { for id in ids {
            let owners: Vec<_> = r.registration.bands.iter().filter(|b| &b.area == area && b.grains.iter().any(|g| &g.id == id)).collect();
            let off = p.left_off.iter().filter(|g| &g.grain.area == area && &g.grain.id == id).count();
            assert_eq!(owners.len() + off, 1, "one owner or left off");
            if let Some(b) = owners.first() { let g = self.gs[area].iter().find(|g| &g.id == id).unwrap(); for d in 0..2 { assert!(b.mosaic_rect_um[d] <= g.bbox_um[d] && b.mosaic_rect_um[d+2] >= g.bbox_um[d+2]); } }
        } }
        for (kind, count) in [(ProblemKind::Overlap,r.stats.bands_overlapping),(ProblemKind::OutsideBody,r.stats.bands_outside_body),(ProblemKind::GrainCut,r.stats.grains_cut),(ProblemKind::PrintedTwice,r.stats.grains_twice)] { assert_eq!(r.problems.iter().filter(|p| p.kind == kind).count(), count, "audit agrees with {kind:?}"); }
    }
}
fn keys(plan: &SheetPlan) -> (StripKey, StripKey) { (plan.strips[0].key.clone(), plan.strips[0].bands[0].key.clone()) }
fn split_edit(band: &StripKey) -> StripEdit { StripEdit::Split { band:band.clone(), axis:Axis::X, at_um:640.0 } }

#[test]
fn project_print_edits_round_trip_preserves_registered_data_and_unknown_keys() {
    use grainatlas_core::project::Project;
    let (_,mut project,_)=support::synthetic();
    project.extra.insert("future_key".into(),json!({"ids":["A1","zr-1"],"value":42,"nested":{"keep":true}}));
    let legacy=project.to_json();let reopened=Project::from_json(&legacy).unwrap();
    assert!(reopened.print_sheets.is_none());assert!(!serde_json::to_value(&reopened).unwrap().as_object().unwrap().contains_key("print_sheets"));assert_eq!(reopened.to_json(),legacy);
    let f=Fixture::new();let plan=f.plan();let (strip,band)=keys(&plan);
    let stored=PrintEdits {params:plan.params.clone(),um_per_mm:plan.um_per_mm,edits:vec![StripEdit::Label {strip:strip.clone(),text:"zr-1".into()},split_edit(&band),StripEdit::Hide {strip}]};
    // Simulate the previously untyped extra key as it appears in an existing project.json.
    let mut old:serde_json::Value=serde_json::from_str(&legacy).unwrap();old["print_sheets"]=serde_json::to_value(&stored).unwrap();
    let typed=Project::from_json(&old.to_string()).unwrap();assert_eq!(typed.print_sheets,Some(stored.into()));assert!(!typed.extra.contains_key("print_sheets"));
    let mut without_print=typed.clone();without_print.print_sheets=None;
    assert_eq!(without_print.to_json(),legacy,"every other key, including grains, spots, labels, ids, grainatlas_ui and upbdb survives");
    let saved=typed.to_json();assert_eq!(Project::from_json(&saved).unwrap().to_json().as_bytes(),saved.as_bytes());
}

#[test]
fn unreadable_project_print_edits_preserve_raw_json_and_registered_data() {
    use grainatlas_core::project::{Project, StoredPrintEdits};
    let (mut session,mut project,_)=support::synthetic();
    project.extra.insert("future_key".into(),json!({"ids":["A1","zr-1"],"nested":{"keep":true}}));
    let baseline=project.to_json();let before:serde_json::Value=serde_json::from_str(&baseline).unwrap();
    let atlas=session.atlas_files(&baseline,"").unwrap();let published_before:serde_json::Value=serde_json::from_slice(&atlas.iter().find(|(name,_)|name=="project.json").unwrap().1).unwrap();
    let raw=json!({"params":{"packing":"packed"},"um_per_mm":8.0,"edits":[{"op":"repack"},{"op":"future_edit","strip":{"area":"A1","grains_hash":"a1","split":0},"value":[42.12345678912345,{"keep":true}]}],"future_key":"zr-1"});
    for raw in [raw,json!({"params":{},"edits":[{"op":"move","sheet":"bad"}]}),json!({"params":42,"edits":[]}),json!(null),json!(false),json!([42]),json!("zr-1")] {
        let mut input=before.clone();input["print_sheets"]=raw.clone();let input=serde_json::to_string_pretty(&input).unwrap();
        let loaded=Project::from_json(&input).unwrap();assert_eq!(loaded.print_sheets,Some(StoredPrintEdits::Raw(raw.clone())));
        assert_eq!(serde_json::from_str::<Project>(&input).unwrap(),loaded);
        let saved=loaded.to_json();let mut value:serde_json::Value=serde_json::from_str(&saved).unwrap();
        assert_eq!(value["print_sheets"].to_string().as_bytes(),raw.to_string().as_bytes(),"raw JSON value survives byte-for-byte");
        value.as_object_mut().unwrap().remove("print_sheets");assert_eq!(value,before,"all other project keys survive");
        assert_eq!(Project::from_json(&saved).unwrap().to_json().as_bytes(),saved.as_bytes());
        session.load_project(&saved).unwrap();assert_eq!(session.project.as_ref().unwrap(),&loaded);
        let files=session.project_files(&saved).unwrap();let folder=&files.iter().find(|(name,_)|name=="project.json").unwrap().1;
        assert_eq!(serde_json::from_slice::<serde_json::Value>(folder).unwrap()["print_sheets"],raw);
        let atlas=session.atlas_files(&saved,"").unwrap();let mut published:serde_json::Value=serde_json::from_slice(&atlas.iter().find(|(name,_)|name=="project.json").unwrap().1).unwrap();
        assert_eq!(published["print_sheets"].to_string().as_bytes(),raw.to_string().as_bytes());
        published.as_object_mut().unwrap().remove("print_sheets");assert_eq!(published,published_before);
    }
}

#[test]
fn reopened_project_replays_the_same_registration_and_survives_all_save_records() {
    use grainatlas_core::project::session::Session;
    let (mut session,mut project,pixels)=support::synthetic();
    let gs=project.extra["grainatlas_ui"]["grains"].to_string();
    let params=SheetParams {packing:Packing::Packed,um_per_mm:8.0,enhance:Default::default(),..Default::default()};let params_json=serde_json::to_string(&params).unwrap();
    let automatic:serde_json::Value=serde_json::from_str(&session.print_sheets(&params_json,&gs,"{}").unwrap()).unwrap();
    let plan:SheetPlan=serde_json::from_value(automatic["plan"].clone()).unwrap();
    project.print_sheets=Some(PrintEdits {params:plan.params.clone(),um_per_mm:plan.um_per_mm,edits:vec![StripEdit::AddSheet {at:1},StripEdit::Move {strip:plan.strips[0].key.clone(),sheet:1,origin_mm:Some([20.0,90.0])}]}.into());
    let edits=serde_json::to_string(project.print_sheets.as_ref().unwrap()).unwrap();
    let before:serde_json::Value=serde_json::from_str(&session.print_plan_render(&automatic["plan"].to_string(),&edits,&gs,"{}").unwrap()).unwrap();assert!(before.get("dropped").is_none_or(|d|d.as_array().unwrap().is_empty()));assert_eq!(before["registration"]["bands"][0]["sheet"],2);
    let saved=project.to_json();
    let files=session.project_files(&saved).unwrap();let folder=&files.iter().find(|(name,_)| name=="project.json").unwrap().1;
    assert_eq!(serde_json::from_slice::<serde_json::Value>(folder).unwrap(),serde_json::from_str::<serde_json::Value>(&saved).unwrap());
    let atlas=session.atlas_files(&saved,"").unwrap();let published=&atlas.iter().find(|(name,_)| name=="project.json").unwrap().1;
    let mut expected=serde_json::to_value(&project.print_sheets).unwrap();grainatlas_core::atlas::canonical(&mut expected);
    assert_eq!(serde_json::from_slice::<serde_json::Value>(published).unwrap()["print_sheets"],expected);
    let mut reopened=Session::new("mount A");reopened.load_project(&saved).unwrap();reopened.load_bundle_area("a",support::W,support::H,pixels).unwrap();
    assert_eq!(reopened.project.as_ref().unwrap().to_json().as_bytes(),saved.as_bytes());
    let automatic:serde_json::Value=serde_json::from_str(&reopened.print_sheets(&params_json,&gs,"{}").unwrap()).unwrap();
    let stored=serde_json::to_string(reopened.project.as_ref().unwrap().print_sheets.as_ref().unwrap()).unwrap();
    let after:serde_json::Value=serde_json::from_str(&reopened.print_plan_render(&automatic["plan"].to_string(),&stored,&gs,"{}").unwrap()).unwrap();
    assert!(after.get("dropped").is_none_or(|d|d.as_array().unwrap().is_empty()));assert_eq!(before["registration"].to_string().as_bytes(),after["registration"].to_string().as_bytes());assert_eq!(before["pages"],after["pages"]);
}

#[test]
fn moving_changes_only_sheet_and_paper_translation() {
    let f=Fixture::new(); let p=f.plan(); let (strip,_)=keys(&p); let before=f.render(&p);
    let result=f.apply(&p,&[StripEdit::AddSheet {at:1},StripEdit::Move {strip,sheet:1,origin_mm:Some([30.0,90.0])}]);
    assert_eq!(result.applied,2); assert!(result.dropped.is_empty()); let after=f.render(&result.plan);
    for (a,b) in before.registration.bands.iter().zip(&after.registration.bands) { let mut a=a.clone(); a.sheet=b.sheet; a.sheet_rect_mm=b.sheet_rect_mm; assert_eq!(&a,b); }
    f.check(&result.plan);
}
#[test]
fn crop_refuses_target_crossings_and_clamps_to_image_edges() {
    let f=Fixture::new();let p=f.plan();let (_,band)=keys(&p);
    let bad=f.apply(&p,&[StripEdit::Crop {band:band.clone(),mosaic_rect_um:[80.0,0.0,1600.0,300.0]}]);
    assert_eq!(bad.applied,0);assert_eq!(bad.plan,p);assert_eq!(bad.dropped[0].reason,"would cut grain 1");
    let good=f.apply(&p,&[StripEdit::Crop {band,mosaic_rect_um:[-50.0,-20.0,2000.0,400.0]}]);
    assert_eq!(good.plan.strips[0].bands[0].mosaic_rect_um,[0.0,0.0,1600.0,300.0]);f.check(&good.plan);
}
#[test]
fn shrinking_and_growing_crops_account_for_every_target() {
    let f=Fixture::new();let p=f.plan();let (_,band)=keys(&p);
    let shrink=StripEdit::Crop {band:band.clone(),mosaic_rect_um:[300.0,60.0,1250.0,160.0]};
    let r=f.apply(&p,std::slice::from_ref(&shrink));
    assert_eq!(r.left_off.iter().map(|g|g.grain.number).collect::<Vec<_>>(),[1,2]);assert!(r.left_off.iter().all(|g|g.reason=="cropped"));f.check(&r.plan);
    let grown=f.apply(&p,&[shrink,StripEdit::Crop {band,mosaic_rect_um:p.strips[0].bands[0].mosaic_rect_um}]);
    assert!(grown.left_off.is_empty());f.check(&grown.plan);
}
#[test]
fn growth_adds_context_without_stealing_and_crop_loss_is_adopted() {
    let mut f=Fixture::new();f.params.context_um=20.0;let p=f.plan();assert!(p.strips.len()>=2);
    let a=p.strips[0].bands[0].key.clone();let b=p.strips[1].bands[0].key.clone();
    let grow=StripEdit::Crop {band:a.clone(),mosaic_rect_um:[30.0,60.0,290.0,160.0]};
    let r=f.apply(&p,std::slice::from_ref(&grow));let rendered=f.render(&r.plan);
    let first=rendered.registration.bands.iter().find(|b|b.grains.iter().any(|g|g.number==1)).unwrap();
    assert_eq!(first.grains.len(),1);assert!(first.context.iter().any(|g|g.number==2));f.check(&r.plan);
    let adopted=f.apply(&p,&[grow,StripEdit::Crop {band:b,mosaic_rect_um:[260.0,60.0,290.0,160.0]}]);
    assert!(adopted.left_off.is_empty());assert_eq!(adopted.plan.strips[0].bands[0].grains.len(),2);f.check(&adopted.plan);
}
#[test]
fn split_uses_repeated_cut_geometry_and_join_restores_parent() {
    let f=Fixture::new();let p=f.plan();let (_,band)=keys(&p);
    let split=f.apply(&p,&[split_edit(&band)]);assert!(split.dropped.is_empty());let bs=&split.plan.strips[0].bands;assert_eq!(bs.len(),2);assert!(f.render(&split.plan).registration.bands.iter().all(|b|!b.repeated_um.is_empty()));
    let join=StripEdit::Join {band_a:bs[0].key.clone(),band_b:bs[1].key.clone()};
    let r=f.apply(&p,&[split_edit(&band),join]);assert!(r.dropped.is_empty());
    // Inverse edits restore packed pages and band records, strays and left_off.
    // Registration still says edited and retains the split/join counts.
    let before=f.render(&p);let after=f.render(&r.plan);assert_eq!(serde_json::to_vec(&before.registration.bands).unwrap(),serde_json::to_vec(&after.registration.bands).unwrap());assert_eq!(before.pages,after.pages);
    assert_eq!(before.registration.strays,after.registration.strays);assert!(before.registration.left_off.is_empty() && after.registration.left_off.is_empty());
    assert_eq!(before.registration.layout,"packed");assert!(before.registration.edits.is_none());assert_eq!(after.registration.layout,"edited");
    let counts=after.registration.edits.unwrap();assert_eq!((counts.splits,counts.joins),(1,1));f.check(&r.plan);
    let bad=f.apply(&p,&[StripEdit::Split {band,axis:Axis::X,at_um:80.0}]);assert_eq!(bad.dropped[0].reason,"would cut grain 1");
}
#[test]
fn hide_show_restores_geometry_ownership_and_svg() {
    let f=Fixture::new();let p=f.plan();let (strip,_)=keys(&p);let hide=StripEdit::Hide {strip:strip.clone()};
    let hidden=f.apply(&p,std::slice::from_ref(&hide));assert_eq!(f.render(&hidden.plan).left_off_summary.hidden,8);assert_eq!(hidden.left_off.len(),8);assert!(hidden.left_off.iter().all(|g|g.reason=="hidden"));assert!(f.render(&hidden.plan).registration.bands.is_empty());f.check(&hidden.plan);
    // Inverse edits restore packed pages and band records, strays and left_off.
    // Registration still says edited and retains the hide count.
    let shown=f.apply(&p,&[hide,StripEdit::Show {strip}]);let a=f.render(&p);let b=f.render(&shown.plan);assert_eq!(a.pages,b.pages);assert_eq!(a.registration.bands,b.registration.bands);assert!(shown.left_off.is_empty());assert_eq!(shown.plan.strips,p.strips);
    assert_eq!(a.registration.strays,b.registration.strays);assert!(a.registration.left_off.is_empty() && b.registration.left_off.is_empty());assert_eq!(a.registration.layout,"packed");assert!(a.registration.edits.is_none());assert_eq!(b.registration.layout,"edited");assert_eq!(b.registration.edits.unwrap().hidden_strips,1);f.check(&shown.plan);
}
#[test]
fn separated_notes_follow_neighbours_sheet_at_render_time() {
    let f=Fixture::new();let p=f.plan();let (strip,band)=keys(&p);
    let split=f.apply(&p,&[split_edit(&band)]);let second=split.plan.strips[0].bands[1].key.clone();
    let edits=[split_edit(&band),StripEdit::Separate {strip},StripEdit::AddSheet {at:1},StripEdit::Move {strip:second,sheet:1,origin_mm:Some([10.0,80.0])}];
    let r=f.apply(&p,&edits);assert!(r.dropped.is_empty());assert_eq!(r.plan.strips.len(),2);assert!(r.plan.strips.iter().all(|s|s.labelled && s.bands.len()==1));
    let rendered=f.render(&r.plan);assert!(rendered.pages[0].contains("continues on sheet 2"));assert!(rendered.pages[1].contains("continues on sheet 1"));f.check(&r.plan);
}
#[test]
fn overlap_bounds_and_header_problems_agree_with_audit() {
    let mut f=Fixture::new();f.params.context_um=20.0;let p=f.plan();let first=&p.strips[0];let key=p.strips[1].key.clone();
    for origin in [first.origin_mm,[0.0,0.0],[420.0,270.0]] { let r=f.apply(&p,&[StripEdit::Move {strip:key.clone(),sheet:0,origin_mm:Some(origin)}]);assert!(!r.problems.is_empty());f.check(&r.plan); }
}
#[test]
fn replay_reports_parameter_and_missing_key_changes() {
    let f=Fixture::new();let p=f.plan();let (strip,_)=keys(&p);let edits=vec![StripEdit::Hide {strip}];
    let mut stored=PrintEdits {um_per_mm:p.um_per_mm,params:p.params.clone(),edits:edits.clone()};stored.params.um_per_mm=16.0;
    let r=replay(&p,&stored,&f.areas(),&f.gs);assert_eq!(r.applied,0);assert_eq!(r.dropped[0].reason,"parameters changed (um_per_mm)");
    let mut changed=p.clone();changed.strips[0].key.grains_hash="removed".into();
    let r=f.apply(&changed,&edits);assert_eq!(r.dropped[0].reason,"strip no longer exists (grains or context changed)");
    stored.params=p.params.clone();stored.params.title="mount A".into();stored.params.enhance.denoise=true;stored.params.spot_sizes_um=vec![12.0];
    assert_eq!(replay(&p,&stored,&f.areas(),&f.gs).applied,1);
}
#[test]
fn deterministic_repack_pins_touched_strips_and_leaves_no_overlaps() {
    let mut f=Fixture::new();f.params.context_um=20.0;let p=f.plan();let key=p.strips[0].key.clone();let origin=[30.0,100.0];
    let edits=[StripEdit::Move {strip:key.clone(),sheet:0,origin_mm:Some(origin)},StripEdit::Repack {}];
    let a=f.apply(&p,&edits);let b=f.apply(&p,&edits);assert_eq!(a.plan.strips[0].origin_mm,origin);assert!(a.problems.is_empty(),"{:?}",a.problems);assert_eq!(serde_json::to_vec(&f.render(&a.plan)).unwrap(),serde_json::to_vec(&f.render(&b.plan)).unwrap());f.check(&a.plan);
    assert_eq!(sheets::edit::snap_origin(&a.plan,&key,0,[8.1,100.0],0.2).unwrap()[0],8.0);
}
#[test]
fn lines_drop_separators_keep_dividers_and_label_independent_strips() {
    let mut f=Fixture::new();f.params.context_um=20.0;f.params.packing=Packing::Lines;let p=f.plan();let i=p.strips.iter().position(|s|!s.labelled).unwrap();let s=&p.strips[i];let key=s.key.clone();
    let r=f.apply(&p,&[StripEdit::AddSheet {at:1}]);assert!(r.plan.lines.is_none());assert!(r.plan.strips.iter().zip(&p.strips).all(|(a,b)|a.origin_mm==b.origin_mm && a.bands==b.bands));
    let block=sheets::edit::strip_block(&r.plan.strips[i]);assert_eq!(block.rect_mm[0],s.origin_mm[0]-2.5);
    let moved=f.apply(&p,&[StripEdit::Move {strip:key,sheet:s.sheet,origin_mm:Some(s.origin_mm)}]);assert!(moved.plan.strips[i].labelled);assert_eq!(moved.plan.strips[i].bands[0].offset_mm,s.bands[0].offset_mm);f.check(&moved.plan);
}
#[test]
fn sheet_operations_require_empty_removal_and_preserve_positions() {
    let f=Fixture::new();let p=f.plan();let bad=f.apply(&p,&[StripEdit::RemoveSheet {sheet:0}]);assert_eq!(bad.dropped[0].reason,"sheet is not empty");
    let r=f.apply(&p,&[StripEdit::AddSheet {at:0},StripEdit::MoveSheet {from:1,to:0},StripEdit::RemoveSheet {sheet:1}]);assert_eq!(r.applied,3);assert_eq!(r.plan.strips[0].origin_mm,p.strips[0].origin_mm);assert_eq!(r.plan.sheet_bodies,p.sheet_bodies);f.check(&r.plan);
}
#[test]
fn fixed_seed_edit_sequences_keep_exact_ownership_and_mapping() {
    let f=Fixture::new();let p=f.plan();let (strip,band)=keys(&p);let mut edits=Vec::new();let mut seed=23u64;
    for _ in 0..50 { seed=seed.wrapping_mul(6364136223846793005).wrapping_add(1);let op=match (seed>>32)%5 {
        0=>StripEdit::Hide {strip:strip.clone()},1=>StripEdit::Show {strip:strip.clone()},
        2=>StripEdit::Crop {band:band.clone(),mosaic_rect_um:[if seed&1==0 {300.0}else{0.0},60.0,1250.0,160.0]},
        3=>StripEdit::Move {strip:strip.clone(),sheet:0,origin_mm:Some([8.0+(seed%20) as f64,80.0])},
        _=>StripEdit::Label {strip:strip.clone(),text:"area A1".into()} };
        edits.push(op);let r=f.apply(&p,&edits);assert!(r.dropped.is_empty());f.check(&r.plan);
        let owned:BTreeSet<_>=r.plan.strips.iter().filter(|s|!s.hidden).flat_map(|s|s.bands.iter().flat_map(|b|b.grains.iter())).collect();assert_eq!(owned.len()+r.left_off.len(),8);
    }
}
#[test]
fn session_renders_edits_and_band_endpoint_returns_embedded_pixels() {
    let (session,project,_)=support::synthetic();let gs=serde_json::to_string(&project.extra["grainatlas_ui"]["grains"]).unwrap();let p=SheetParams {packing:Packing::Packed,um_per_mm:8.0,enhance:Default::default(),..Default::default()};
    let automatic:serde_json::Value=serde_json::from_str(&session.print_sheets(&serde_json::to_string(&p).unwrap(),&gs,"{}").unwrap()).unwrap();
    let plan:SheetPlan=serde_json::from_value(automatic["plan"].clone()).unwrap();let key=plan.strips[0].key.clone();let rect=plan.strips[0].bands[0].mosaic_rect_um;
    let edits=vec![StripEdit::Move {strip:key,sheet:0,origin_mm:Some([20.0,90.0])}];
    let output:serde_json::Value=serde_json::from_str(&session.print_plan_render(&automatic["plan"].to_string(),&serde_json::to_string(&edits).unwrap(),&gs,"{}").unwrap()).unwrap();
    assert_eq!(output["registration"]["layout"],"edited");assert_eq!(output["plan"]["strips"][0]["origin_mm"],json!([20.0,90.0]));
    let png=session.print_band_image("a",rect,"{}").unwrap();let encoded=grainatlas_core::layout::base64(&png);assert!(output["pages"][0].as_str().unwrap().contains(&encoded));
    assert!(session.print_band_image("a",[0.0,0.0,0.0,1.0],"{}").is_err());
    let mut invalid=plan.clone();invalid.strips[0].bands[0].key.area="missing".into();
    assert!(session.print_plan_render(&serde_json::to_string(&invalid).unwrap(),"[]",&gs,"{}").is_err());
    invalid=plan.clone();invalid.lines=Some(vec![SheetLine {sheet:0,origin_mm:[0.0,0.0],band_height_mm:1.0,right_mm:1.0,strips:vec![999]}]);
    assert!(session.print_plan_render(&serde_json::to_string(&invalid).unwrap(),"[]",&gs,"{}").is_err());
}

#[test]
fn join_after_separation_and_nested_split_replays_by_direct_parent_key() {
    let f=Fixture::new();let p=f.plan();let (strip,band)=keys(&p);
    let split=f.apply(&p,&[split_edit(&band)]);let children=&split.plan.strips[0].bands;
    let joined=f.apply(&p,&[split_edit(&band),StripEdit::Separate {strip},StripEdit::Join {band_a:children[1].key.clone(),band_b:children[0].key.clone()}]);
    assert!(joined.dropped.is_empty(),"{:?}",joined.dropped);assert_eq!(joined.plan.strips.len(),1);assert_eq!(joined.plan.strips[0].bands[0].key,band);f.check(&joined.plan);
    let nested=StripEdit::Split {band:children[0].key.clone(),axis:Axis::X,at_um:320.0};
    let r=f.apply(&p,&[split_edit(&band),nested.clone()]);assert!(r.dropped.is_empty());let bs=&r.plan.strips[0].bands;
    let edits=[split_edit(&band),nested,StripEdit::Join {band_a:bs[0].key.clone(),band_b:bs[1].key.clone()},StripEdit::Join {band_a:children[0].key.clone(),band_b:children[1].key.clone()}];
    let r=f.apply(&p,&edits);assert!(r.dropped.is_empty());assert_eq!(f.render(&r.plan).pages,f.render(&p).pages);f.check(&r.plan);
}

#[test]
fn cached_session_move_on_three_hundred_bands() {
    let (session,project,_)=support::synthetic();
    let prototype:Grain=serde_json::from_value(project.extra["grainatlas_ui"]["grains"]["a"][0].clone()).unwrap();
    let gs:Vec<_>=(0..300).map(|i| {
        let mut g=prototype.clone();g.id=uuid::Uuid::from_u128(i+1);g.number=(i+1) as u32;g.label=None;
        let x=6.0+(i%20) as f64*18.0;let y=7.0+(i/20) as f64*20.0;
        g.bbox_um=[x-4.0,y-4.0,x+4.0,y+4.0];g.centroid_um=[x,y];g.polygon_um=vec![[x-4.0,y-4.0],[x+4.0,y-4.0],[x+4.0,y+4.0],[x-4.0,y+4.0]];g
    }).collect();
    let grains=serde_json::to_string(&BTreeMap::from([("a",gs)])).unwrap();
    let p=SheetParams {packing:Packing::Packed,um_per_mm:8.0,context_um:1.0,pad_um:0.0,min_group:1,min_cluster_grains:0,enhance:grainatlas_core::photometric::enhance::EnhanceParams {denoise:true,sharpen:true,..Default::default()},..Default::default()};
    let r:SheetResult=serde_json::from_str(&session.print_sheets(&serde_json::to_string(&p).unwrap(),&grains,"{}").unwrap()).unwrap();assert_eq!(r.stats.bands,300);
    let s=&r.plan.strips[0];let edits=vec![StripEdit::Move {strip:s.key.clone(),sheet:s.sheet,origin_mm:Some([s.origin_mm[0]+0.5,s.origin_mm[1]])}];
    let start=std::time::Instant::now();let output:SheetResult=serde_json::from_str(&session.print_plan_render(&serde_json::to_string(&r.plan).unwrap(),&serde_json::to_string(&edits).unwrap(),&grains,"{}").unwrap()).unwrap();
    eprintln!("cached move, 300 bands: {:.1} ms",start.elapsed().as_secs_f64()*1000.0);assert_eq!(output.stats.bands,300);assert_eq!(output.registration.bands[0].mosaic_rect_um,r.registration.bands[0].mosaic_rect_um);
}

#[test]
fn replay_detects_resolved_auto_scale_drift_and_accepts_unknown_or_roundoff() {
    let mut f=Fixture::new();f.params.um_per_mm=0.0;let p=f.plan();let (strip,band)=keys(&p);
    let stored=PrintEdits {params:p.params.clone(),um_per_mm:p.um_per_mm,edits:vec![StripEdit::Hide {strip},StripEdit::Label {strip:p.strips[0].key.clone(),text:"area A1".into()}]};
    for g in f.gs.get_mut("A1").unwrap() { g.eq_diameter_um*=2.0; }
    let changed=f.plan();assert_ne!(changed.um_per_mm,p.um_per_mm);assert_eq!(changed.params,p.params);assert_eq!(changed.strips[0].bands[0].key,band);
    let r=replay(&changed,&stored,&f.areas(),&f.gs);assert_eq!(r.applied,0);assert_eq!(r.plan,changed);assert_eq!(r.dropped.len(),2);
    for (i,d) in r.dropped.iter().enumerate() { assert_eq!(d.index,i);assert_eq!(d.reason,format!("parameters changed (scale {} → {} µm/mm)",p.um_per_mm,changed.um_per_mm)); }
    let mut close=stored.clone();close.um_per_mm=changed.um_per_mm*(1.0+0.5e-9);assert_eq!(replay(&changed,&close,&f.areas(),&f.gs).applied,2);
    close.um_per_mm=changed.um_per_mm*(1.0+2e-9);assert_eq!(replay(&changed,&close,&f.areas(),&f.gs).applied,0);
    let mut legacy=serde_json::to_value(&stored).unwrap();legacy.as_object_mut().unwrap().remove("um_per_mm");
    let legacy:PrintEdits=serde_json::from_value(legacy).unwrap();assert_eq!(legacy.um_per_mm,0.0);assert_eq!(replay(&changed,&legacy,&f.areas(),&f.gs).applied,2);
}

#[test]
fn lines_shared_gap_blocks_touch_until_either_strip_is_touched() {
    let mut f=Fixture::new();f.params.packing=Packing::Lines;
    f.gs.get_mut("A1").unwrap().retain(|g|g.number<=3 || g.number>=6);
    let p=f.plan();assert_eq!(p.strips.len(),2);let (a,b)=(&p.strips[0],&p.strips[1]);
    assert!(!b.labelled);assert!(a.bands.last().unwrap().gap_after_um.is_some());assert_eq!(a.shares_gap_with.as_ref(),Some(&b.key));
    let add=StripEdit::AddSheet {at:p.sheet_bodies.len()};let r=f.apply(&p,std::slice::from_ref(&add));
    assert!(r.dropped.is_empty());assert!(r.problems.is_empty(),"{:?}",r.problems);assert_eq!(f.render(&r.plan).stats.bands_overlapping,0);
    assert_eq!(sheets::edit::strip_block(&r.plan.strips[0]).rect_mm[2],sheets::edit::strip_block(&r.plan.strips[1]).rect_mm[0]);f.check(&r.plan);
    for key in [&a.key,&b.key] {
        let r=f.apply(&p,&[add.clone(),StripEdit::Move {strip:key.clone(),sheet:0,origin_mm:Some([8.0,120.0])}]);
        assert!(r.dropped.is_empty());assert!(r.problems.is_empty(),"{:?}",r.problems);assert_eq!(f.render(&r.plan).stats.bands_overlapping,0);
        assert!(r.plan.strips.iter().all(|s|s.shares_gap_with.is_none()));f.check(&r.plan);
    }
    let first_free=f.apply(&p,&[StripEdit::Move {strip:b.key.clone(),sheet:0,origin_mm:None}]);
    assert!(first_free.dropped.is_empty());assert!(first_free.problems.is_empty(),"{:?}",first_free.problems);f.check(&first_free.plan);
    for edit in [StripEdit::Label {strip:b.key.clone(),text:"area A1".into()},StripEdit::Separate {strip:b.key.clone()}] {
        let r=f.apply(&p,&[edit]);assert!(r.dropped.is_empty());assert!(r.plan.strips.iter().all(|s|s.shares_gap_with.is_none()));
        assert!(sheets::edit::strip_block(&r.plan.strips[0]).rect_mm[2]>sheets::edit::strip_block(&p.strips[0]).rect_mm[2]);
    }
}

#[test]
fn repack_removes_only_trailing_empty_sheets_and_keeps_one() {
    let mut f=Fixture::new();f.params.context_um=20.0;let p=f.plan();let (key,_)=keys(&p);
    let add=[StripEdit::AddSheet {at:1},StripEdit::AddSheet {at:2},StripEdit::AddSheet {at:3}];
    let mut edits=add.to_vec();edits.push(StripEdit::Repack {});let r=f.apply(&p,&edits);
    assert!(r.dropped.is_empty());assert_eq!(r.plan.sheet_bodies.len(),1);assert!(r.problems.is_empty());f.check(&r.plan);
    let mut edits=add.to_vec();edits.extend([StripEdit::Move {strip:key,sheet:2,origin_mm:None},StripEdit::Repack {}]);let r=f.apply(&p,&edits);
    assert!(r.dropped.is_empty());assert_eq!(r.plan.sheet_bodies.len(),3);assert!(!r.plan.strips.iter().any(|s|!s.hidden && s.sheet==1));assert_eq!(r.plan.strips[0].sheet,2);assert!(r.problems.is_empty());f.check(&r.plan);
    let mut edits=add.to_vec();edits.extend(p.strips.iter().map(|s|StripEdit::Hide {strip:s.key.clone()}));edits.push(StripEdit::Repack {});let r=f.apply(&p,&edits);
    assert_eq!(r.plan.sheet_bodies.len(),1);assert!(r.problems.is_empty());assert_eq!(r.left_off.len(),8);assert!(r.plan.strips.iter().all(|s|s.sheet==0));f.check(&r.plan);
}

#[test]
fn thumbnail_move_uses_first_free_place_ignores_self_and_hidden_and_refuses_full_sheet() {
    let mut f=Fixture::new();f.params.context_um=20.0;let p=f.plan();let (a,b)=(&p.strips[0],&p.strips[1]);
    let body=p.sheet_bodies[0].body_rect_mm;
    let r=f.apply(&p,&[StripEdit::Move {strip:a.key.clone(),sheet:0,origin_mm:None}]);
    assert!(r.dropped.is_empty());assert_eq!(r.plan.strips[0].origin_mm,[body[0],body[1]]);assert!(r.problems.is_empty());f.check(&r.plan);
    let edits=[StripEdit::AddSheet {at:1},StripEdit::Move {strip:b.key.clone(),sheet:1,origin_mm:None},StripEdit::Move {strip:a.key.clone(),sheet:1,origin_mm:None}];
    let r=f.apply(&p,&edits);assert!(r.dropped.is_empty());let b=sheets::edit::strip_block(&r.plan.strips[1]);
    assert_eq!(r.plan.strips[0].origin_mm,[b.rect_mm[2]+2.5,b.rect_mm[1]]);assert!(r.problems.is_empty());f.check(&r.plan);
    let mut hidden=edits[..2].to_vec();hidden.extend([StripEdit::Hide {strip:p.strips[1].key.clone()},edits[2].clone()]);let r=f.apply(&p,&hidden);
    assert_eq!(r.plan.strips[0].origin_mm,[r.plan.sheet_bodies[1].body_rect_mm[0],r.plan.sheet_bodies[1].body_rect_mm[1]]);assert!(r.problems.is_empty());f.check(&r.plan);
    let mut full=p.clone();full.strips[1].origin_mm=[body[0],body[1]];full.strips[1].size_mm=[body[2]-body[0],body[3]-body[1]];
    let edit=StripEdit::Move {strip:a.key.clone(),sheet:0,origin_mm:None};let r=f.apply(&full,std::slice::from_ref(&edit));assert_eq!(r.applied,0);assert_eq!(r.plan,full);assert_eq!(r.dropped[0].reason,"no room on sheet 1");
    let mut json=serde_json::to_value(&edit).unwrap();assert!(json["origin_mm"].is_null());json.as_object_mut().unwrap().remove("origin_mm");assert_eq!(serde_json::from_value::<StripEdit>(json).unwrap(),edit);
    let mut oversize=p.clone();oversize.strips[0].label="A1 ".repeat(200);let r=f.apply(&oversize,&[edit]);assert_eq!(r.dropped[0].reason,"no room on sheet 1");assert_eq!(r.plan,oversize);
}
