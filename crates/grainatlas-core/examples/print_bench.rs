//! Read-only print/open benchmark. Paths and specimen metadata are never reported.
//! cargo run --release -p grainatlas-core --example print_bench -- target/large-bundle
use grainatlas_core::{
    layout::{
        self,
        sheets::{self, SheetArea, SheetMeta, SheetParams},
    },
    project::{Project, session::Session},
    segment::Grain,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::args().nth(1).expect("bundle directory"));
    let t = Instant::now();
    if std::env::args().any(|a| a == "--package") {
        return package(root);
    }
    let json = std::fs::read_to_string(root.join("project.json"))?;
    let project = Project::from_json(&json)?;
    let read_ms = ms(t);
    let grains: BTreeMap<String, Vec<Grain>> =
        serde_json::from_value(project.extra["grainatlas_ui"]["grains"].clone())?;
    let mut pixels = Vec::new();
    let t = Instant::now();
    for rec in &project.areas {
        let m = rec.mosaic.as_ref().expect("mosaic");
        let mut px = vec![0; m.w * m.h];
        for y in 0..m.h.div_ceil(256) {
            for x in 0..m.w.div_ceil(256) {
                let bytes =
                    std::fs::read(root.join(format!("pyramid/{}/0/{}_{}.webp", rec.id, x, y)))?;
                let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes))?;
                let (w, h) = d.dimensions();
                let mut rgb = vec![0; d.output_buffer_size().unwrap()];
                d.read_image(&mut rgb)?;
                let channels = if d.has_alpha() { 4 } else { 3 };
                for yy in 0..h as usize {
                    for xx in 0..w as usize {
                        px[(y * 256 + yy) * m.w + x * 256 + xx] =
                            rgb[(yy * w as usize + xx) * channels];
                    }
                }
            }
        }
        pixels.push(px);
    }
    let decode_ms = ms(t);
    let mut session = Session::new("benchmark");
    session.load_project(&json)?;
    let t = Instant::now();
    for (rec, px) in project.areas.iter().zip(&pixels) {
        let m = rec.mosaic.as_ref().unwrap();
        session.load_bundle_area(&rec.id, m.w, m.h, px.clone())?;
    }
    let mosaic_pyramid_ms = ms(t);
    measure(
        &session,
        &project,
        &pixels,
        &grains,
        serde_json::json!({"read_ms":read_ms,"decode_ms":decode_ms,"mosaic_pyramid_ms":mosaic_pyramid_ms}),
    )
}
fn measure(
    session: &Session,
    project: &Project,
    pixels: &[Vec<u8>],
    grains: &BTreeMap<String, Vec<Grain>>,
    mut timings: serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let areas: Vec<_> = project
        .areas
        .iter()
        .zip(pixels)
        .map(|(rec, px)| {
            let m = rec.mosaic.as_ref().unwrap();
            SheetArea {
                rec,
                img: layout::AreaImage { source_size_um: None, edits: None,
                    id: &rec.id,
                    name: &rec.name,
                    sample: &rec.sample,
                    w: m.w,
                    h: m.h,
                    px,
                    lut: std::array::from_fn(|i| i as u8),
                    um_per_px: rec.um_per_px(),
                },
            }
        })
        .collect();
    let p: SheetParams = serde_json::from_value(
        serde_json::json!({"landscape":true,"enhance":{"denoise":true,"sharpen":true}}),
    )?;
    let t = Instant::now();
    let own = grainatlas_core::ownership::assign(&project.areas, grains, &Default::default());
    let plan = sheets::plan_sheets(&p, &areas, grains, &own, &SheetMeta::default());
    let planning_ms = ms(t);
    let image_ns = AtomicU64::new(0);
    let png_ns = AtomicU64::new(0);
    let encoded = Mutex::new(Vec::new());
    let t = Instant::now();
    let r = sheets::render_plan_with_images(&plan, &areas, grains, &own, &|ai, rect| {
        let t = Instant::now();
        let (w, h, px, bg) =
            sheets::band_pixels(&areas[ai].img, rect, &grains[&areas[ai].rec.id], &p.enhance);
        image_ns.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
        let t = Instant::now();
        let result = (grainatlas_core::image::encode_png_gray(w, h, &px), bg);
        png_ns.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
        encoded.lock().unwrap().push(result.0.clone());
        result
    });
    let render_ms = ms(t);
    let t = Instant::now();
    let pdf = layout::pdf::pdf_from_svgs(&r.pages)?;
    let pdf_ms = ms(t);
    let t = Instant::now();
    let json = serde_json::to_string(&r)?;
    let json_ms = ms(t);
    let t = Instant::now();
    let base64_bytes: usize = encoded
        .lock()
        .unwrap()
        .iter()
        .map(|b| layout::base64(b).len())
        .sum();
    let base64_ms = ms(t);
    timings.as_object_mut().unwrap().extend(serde_json::json!({"pdf_ms":pdf_ms,"pdf_bytes":pdf.len(),"png_cpu_ms":png_ns.load(Ordering::Relaxed) as f64/1e6,"planning_ms":planning_ms,"render_ms":render_ms,"band_cpu_ms":image_ns.load(Ordering::Relaxed) as f64/1e6,"json_ms":json_ms,"json_bytes":json.len(),"content_sha256":grainatlas_core::util::sha256_hex(json.as_bytes()),"base64_ms":base64_ms,"base64_bytes":base64_bytes,"pages":r.pages.len(),"bands":r.stats.bands}).as_object().unwrap().clone());
    let params = serde_json::to_string(&p)?;
    let gs = serde_json::to_string(grains)?;
    let tone = serde_json::to_string(
        &project
            .areas
            .iter()
            .map(|a| {
                (
                    a.id.clone(),
                    grainatlas_core::photometric::ToneParams::neutral(),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    )?;
    let t = Instant::now();
    let preview: serde_json::Value =
        serde_json::from_str(&session.print_sheets_preview(&params, &gs, &tone, "")?)?;
    let preview_ms = ms(t);
    let id = preview["job_id"].as_u64().unwrap() as usize;
    let count = preview["pages"].as_array().unwrap().len();
    let preview_bytes: usize = preview["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().len())
        .sum();
    let t = Instant::now();
    for page in 0..count {
        session.print_task(id, "quality", page)?;
    }
    let quality_ms = ms(t);
    let full = session.print_task(id, "full", 0)?;
    let expected = session.print_sheets(&params, &gs, &tone)?;
    assert_eq!(full, expected.as_bytes());
    let t = Instant::now();
    session.print_task(id, "pdf_begin", 0)?;
    for page in 0..count {
        session.print_task(id, "pdf_prepare", page)?;
    }
    for page in 0..count {
        session.print_task(id, "pdf_page", page)?;
    }
    let staged_pdf = session.print_task(id, "pdf_finish", 0)?;
    let staged_pdf_ms = ms(t);
    if std::env::var_os("PRINT_BENCH_ARTIFACTS").is_some() {
        std::fs::create_dir_all("target/perf")?;
        std::fs::write("target/perf/print-new.pdf", &pdf)?;
        std::fs::write("target/perf/print-first.svg", &r.pages[0])?;
    }
    let t = Instant::now();
    let packed = SheetParams {
        packing: sheets::Packing::Packed,
        ..p.clone()
    };
    let _ = session.print_sheets(&serde_json::to_string(&packed)?, &gs, &tone)?;
    let reused_packing_ms = ms(t);
    timings.as_object_mut().unwrap().extend(serde_json::json!({"preview_ms":preview_ms,"preview_svg_bytes":preview_bytes,"quality_ms":quality_ms,"staged_pdf_ms":staged_pdf_ms,"staged_pdf_bytes":staged_pdf.len(),"reused_packing_ms":reused_packing_ms,"staged_matches_full":true}).as_object().unwrap().clone());
    println!("{timings}");
    Ok(())
}

fn package(root: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    use grainatlas_core::{Ctl, source::DirSource};
    let mut session = Session::new("benchmark");
    let t = Instant::now();
    session.open_source(
        std::sync::Arc::new(DirSource::new(root)?),
        "",
        "2026-01-01T00:00:00Z",
    )?;
    let read_ms = ms(t);
    let ids: Vec<_> = session
        .project
        .as_ref()
        .unwrap()
        .areas
        .iter()
        .map(|a| a.id.clone())
        .collect();
    let mut grains = BTreeMap::new();
    let mut steps = BTreeMap::<String, f64>::new();
    for id in &ids {
        let start = Instant::now();
        let clock = || start.elapsed().as_secs_f64();
        let summary: serde_json::Value =
            serde_json::from_str(&session.stitch(id, "", &Ctl::NONE, &clock)?)?;
        for step in summary["timings"].as_array().unwrap() {
            *steps.entry(step[0].as_str().unwrap().into()).or_default() +=
                step[1].as_f64().unwrap() * 1000.0;
        }
        let params = serde_json::to_string(&grainatlas_core::segment::SegmentParams::default())?;
        grains.insert(
            id.clone(),
            serde_json::from_str(&session.segment(id, &params, "", "")?)?,
        );
    }
    let project = session.project.as_ref().unwrap();
    let pixels: Vec<_> = project
        .areas
        .iter()
        .map(|a| session.outputs[&a.id].base.as_ref().clone())
        .collect();
    measure(
        &session,
        project,
        &pixels,
        &grains,
        serde_json::json!({"read_ms":read_ms,"open_steps_ms":steps}),
    )
}
