//! Offline synthetic review: cargo run --release -p grainatlas-core --example spot_label_demo -- OUT_DIR
#[path = "support/spot_labels.rs"]
mod fixtures;
use grainatlas_core::layout::labels::{Options, render};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args_os().nth(1).ok_or("expected OUT_DIR")?;
    std::fs::create_dir_all(&out)?;
    for (name, (page, mut svg)) in [
        ("whole-area", fixtures::whole()),
        ("dense-grain-sheet", fixtures::dense_sheet()),
        ("context-edge", fixtures::context_edge()),
        ("association-row", fixtures::association_row()),
    ] {
        let start = std::time::Instant::now();
        let report = render(&mut svg, page, &Options::default(), 1);
        let elapsed = start.elapsed();
        let pdf = grainatlas_core::layout::pdf::pdf_from_svgs(&[svg.clone()])?;
        assert!(String::from_utf8_lossy(&pdf).contains("IBMPlexSansCond-Medium"));
        std::fs::write(std::path::Path::new(&out).join(format!("{name}.pdf")), pdf)?;
        std::fs::write(std::path::Path::new(&out).join(format!("{name}.svg")), svg)?;
        std::fs::write(
            std::path::Path::new(&out).join(format!("{name}.json")),
            serde_json::to_vec_pretty(&report)?,
        )?;
        println!(
            "{name}: {:.1} mm, {} adjacent, {} ticks, {} leaders, {} conflicts, {} ambiguous, {} context omitted, {:.3} ms",
            report.size_mm,
            report.counts.next_to_spot,
            report.counts.with_ticks,
            report.counts.with_leaders,
            report.counts.still_overlapping,
            report.counts.ambiguous,
            report.counts.context_omitted,
            elapsed.as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
