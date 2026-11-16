//! Invented pixels and stable identifiers only; no instrument or project data.
use grainatlas_core::{project::{Project, session::Session}, readers::{AreaMeta, PackageInfo}};
use serde_json::json;

pub const PROJECT_ID: &str = "10000000-0000-4000-8000-000000000001";
pub const MOUNT_ID: &str = "20000000-0000-4000-8000-000000000001";
pub const W: usize = 384;
pub const H: usize = 320;

pub fn synthetic() -> (Session, Project, Vec<u8>) {
    let pixels: Vec<u8> = (0..W*H).map(|i| (35 + ((i % W + i / W) % 190)) as u8).collect();
    let area = AreaMeta { id: "a".into(), name: "Synthetic area".into(), grid: None, pixel_size_nm: 1000.0,
        field_um: None, pitch_um: None, stage_sign: [1.0, 1.0], tiles: vec![], meta: Default::default() };
    let info = PackageInfo { reader: "synthetic".into(), kind: "package".into(), name: "Synthetic mount".into(), areas: vec![area], meta: Default::default(), navigation: None };
    let mut project = Project::new(&info, "", "2026-01-01T00:00:00Z");
    project.uuid = PROJECT_ID.parse().unwrap();
    project.areas[0].uuid = "30000000-0000-4000-8000-000000000001".parse().unwrap();
    project.upbdb.mount_id = Some(MOUNT_ID.parse().unwrap());
    project.areas[0].params.tone.low_pct = 2.0;
    project.areas[0].params.tone.gamma = 1.3;
    let grains: Vec<_> = [(80.0, 1), (240.0, 2)].iter().map(|&(x, n)| json!({
        "id": format!("40000000-0000-4000-8000-{n:012}"), "number": n, "label": format!("Synthetic/{n}"),
        "polygon_um": [[x-30.0,80.0],[x+30.0,80.0],[x+30.0,160.0],[x-30.0,160.0]],
        "bbox_um": [x-30.0,80.0,x+30.0,160.0], "centroid_um": [x,120.0],
        "area_um2": 4800.0, "eq_diameter_um": 78.0, "major_um": 80.0, "minor_um": 60.0,
        "orientation_deg": 0.0, "mean_intensity": 0.6, "manual": true, "scale_bar_um": 20.0
    })).collect();
    let spots: Vec<_> = [(75.0,1),(85.0,2),(240.0,3)].iter().map(|&(x,n)| json!({
        "id": format!("50000000-0000-4000-8000-{n:012}"), "area_id": "a", "name": format!("Synthetic_{n}"),
        "sample": "Synthetic area", "grain_id": format!("40000000-0000-4000-8000-{:012}", if n == 3 {2} else {1}),
        "grain_number": if n == 3 {2} else {1}, "n": n, "x_um": x, "y_um": 120.0, "diameter_um": 15.0
    })).collect();
    project.extra.insert("grainatlas_ui".into(), json!({ "grains": { "a": grains }, "spots": spots,
        "labels": [{"id":"60000000-0000-4000-8000-000000000001", "area_id":"a", "x_um":90.0,"y_um":130.0,"text":"Synthetic label"}],
        "tone": {"a": project.areas[0].params.tone} }));
    let mut session = Session::new("synthetic");
    session.load_project(&project.to_json()).unwrap();
    let mosaic = serde_json::from_str(&session.load_bundle_area("a", W, H, pixels.clone()).unwrap()).unwrap();
    project.areas[0].mosaic = Some(mosaic);
    session.load_project(&project.to_json()).unwrap();
    session.set_package_digest(&"d".repeat(64), 1);
    (session, project, pixels)
}
