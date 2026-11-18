//! cargo run -p grainatlas-core --example synthetic_grain_export -- DESTINATION [--large [N]]
use grainatlas_core::{
    Ctl,
    grain_export::{Export, File, Options, synthetic},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn write(dir: &Path, files: Vec<File>) -> Result<(), Box<dyn std::error::Error>> {
    for (name, bytes) in files {
        let file = dir.join(name);
        std::fs::create_dir_all(file.parent().unwrap())?;
        std::fs::write(file, bytes)?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let usage = "usage: synthetic_grain_export DESTINATION [--large [GRAINS_PER_AREA]] (destination must not exist)";
    let mut args = std::env::args().skip(1).peekable();
    let mut destination = None;
    let mut large = None;
    while let Some(arg) = args.next() {
        if arg == "--large" {
            let count = if args.peek().is_some_and(|s| s.parse::<usize>().is_ok()) {
                args.next().unwrap().parse::<usize>()?
            } else {
                synthetic::DEFAULT_GRAINS_PER_AREA
            };
            if count == 0 {
                return Err("--large requires at least one grain per area".into());
            }
            large = Some(count);
        } else if arg.starts_with('-') || destination.replace(arg).is_some() {
            return Err(usage.into());
        }
    }
    let destination = destination.ok_or(usage)?;
    let dir = Path::new(&destination);
    std::fs::create_dir(dir)?;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let start = Instant::now();
        let (mut session, project) = match large {
            Some(n) => synthetic::large_mount(42, n),
            None => synthetic::mount(),
        };
        let grains: usize = project.extra["grainatlas_ui"]["grains"]
            .as_object()
            .unwrap()
            .values()
            .map(|v| v.as_array().unwrap().len())
            .sum();
        println!(
            "Grains: {grains} native; mount generation: {:.3} s",
            start.elapsed().as_secs_f64()
        );
        let options = Options {
            exported: "2026-01-01T00:00:00Z".into(),
            ..Options::default()
        };
        let start = Instant::now();
        let mut export = Export::begin(
            &session,
            &project.to_json(),
            &serde_json::to_string(&options)?,
        )?;
        let mut measurement = start.elapsed();
        let mut crops = Duration::ZERO;
        loop {
            let start = Instant::now();
            let batch = export.batch(&mut session, &Ctl::NONE)?;
            let writing = batch.stage == "writing grain images";
            write(dir, batch.files)?;
            if writing {
                crops += start.elapsed();
            } else {
                measurement += start.elapsed();
            }
            if batch.done {
                break;
            }
        }
        let start = Instant::now();
        let files = export.finish(&Ctl::NONE)?;
        let rows = files
            .iter()
            .find(|(name, _)| name == "grains.csv")
            .map(|(_, bytes)| bytes.iter().filter(|&&b| b == b'\n').count() - 1)
            .unwrap();
        write(dir, files)?;
        let finish = start.elapsed();
        println!("Exported rows (including layers): {rows}");
        println!("Begin/measure: {:.3} s", measurement.as_secs_f64());
        println!(
            "Crop writing (raster, encode, disk): {:.3} s",
            crops.as_secs_f64()
        );
        println!("Finish (metadata and disk): {:.3} s", finish.as_secs_f64());
        println!(
            "Export total: {:.3} s",
            (measurement + crops + finish).as_secs_f64()
        );
        Ok(())
    })();
    if result.is_err() {
        std::fs::remove_dir_all(dir)?;
    }
    result?;
    println!("Saved {destination}");
    Ok(())
}
