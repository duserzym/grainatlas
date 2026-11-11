//! Whole-area exports borrow only a saved base mosaic, without grains or heavy buffers.
use super::*;
use grainatlas_core::layout::whole::{WholeParams, whole_sheet};

pub(super) fn render(folder: &Path, out: &Path, mut p: WholeParams, svg: bool) -> Result<()> {
    let folder = folder.canonicalize()?;
    let out = resolved_path(out)?;
    if out.starts_with(&folder) {
        bail!("--out must be outside the project folder");
    }
    let project = Project::from_json(&std::fs::read_to_string(folder.join("project.json"))?)?;
    let rec = p.select_area(&project.areas)?;
    let (m, px) = saved_mosaic(&folder.join("mosaic").join(&rec.id))?;
    let meta = match open(Path::new(&project.package.path)) {
        Ok((pkg, _, _)) => {
            let mut info = pkg.info().clone();
            info.areas.retain(|a| a.id == rec.id);
            sheets::sheet_meta(&info)
        }
        Err(_) => SheetMeta {
            package: project.package.name.clone(),
            areas: 1,
            tiles: rec.tiles.len(),
            um_per_px: rec.um_per_px(),
            ..Default::default()
        },
    };
    p.project = folder
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into();
    p.printed = now_rfc3339();
    let area = SheetArea {
        rec,
        img: grainatlas_core::layout::AreaImage {
            id: &rec.id,
            name: &rec.name,
            sample: if rec.sample.is_empty() {
                &rec.name
            } else {
                &rec.sample
            },
            w: m.w,
            h: m.h,
            px: &px,
            lut: grainatlas_core::photometric::tone::lut(&rec.params.tone),
            um_per_px: rec.um_per_px(),
        },
    };
    export(&out, &folder, &project, &p, &area, &meta, svg)
}

pub(super) fn export(
    out: &Path,
    folder: &Path,
    project: &Project,
    p: &WholeParams,
    area: &SheetArea,
    meta: &SheetMeta,
    svg: bool,
) -> Result<()> {
    // Output stem controls every filename, including the registration named on paper.
    let stem = p.output_stem(&meta.package);
    let mut p = p.clone();
    p.registration_file = format!("{stem}_print_registration.json");
    let mut r = whole_sheet(&p, area, meta)?;
    r.registration.project_uuid = Some(project.uuid);
    r.registration.project_path = abs(folder);
    let mut files = vec![
        (
            format!("{stem}.pdf"),
            grainatlas_core::layout::pdf::pdf_from_svgs(&r.pages)?,
        ),
        (
            p.registration_file,
            serde_json::to_vec_pretty(&r.registration)?,
        ),
        (
            format!("{stem}_stats.json"),
            serde_json::to_vec_pretty(&r.stats)?,
        ),
    ];
    if svg {
        files.push((format!("{stem}.svg"), r.pages[0].as_bytes().to_vec()));
    }
    for (name, _) in &files {
        if Path::new(name)
            .file_name()
            .is_none_or(|n| Path::new(n) != Path::new(name))
        {
            bail!("export name must be a file without a directory: {name}");
        }
        // stitch exports live inside exports/; saved-project exports must stay outside.
        let dest = resolved_path(&out.join(name))?;
        let allowed = resolved_path(out)?;
        if dest.parent() != Some(allowed.as_path()) {
            bail!("export destination escapes output directory: {name}");
        }
    }
    for (name, bytes) in files {
        write(&out.join(name), &bytes)?;
    }
    eprintln!(
        "whole area: 1 sheet, {} at {} um/mm, embedded {} x {} px",
        r.stats.orientation, r.stats.um_per_mm, r.stats.embedded_px[0], r.stats.embedded_px[1]
    );
    Ok(())
}
