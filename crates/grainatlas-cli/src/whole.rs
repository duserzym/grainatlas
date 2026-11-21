//! Whole-area exports borrow only a saved base mosaic, without grains or heavy buffers.
use super::*;
use grainatlas_core::layout::whole::{WholeParams, whole_sheet};

pub(super) fn render(folder: &Path, out: &Path, mut p: WholeParams, svg: bool, spots: bool, label_size: Option<f64>) -> Result<()> {
    let folder = folder.canonicalize()?;
    let out = resolved_path(out)?;
    if out.starts_with(&folder) {
        bail!("--out must be outside the project folder");
    }
    let project = Project::from_json(&std::fs::read_to_string(folder.join("project.json"))?)?;
    if spots {
        let (spots, labels) = print_spots(&project, label_size)?;
        p.print_spots = Some(spots);
        p.print_labels = Some(labels);
        // Whole-area rendering also uses saved grain polygons as soft obstacles.
        let id = &p.select_area(&project.areas)?.id;
        p.print_grains = project.extra.get("grainatlas_ui").and_then(|u| u.get("grains")).and_then(|g| g.get(id)).map(|v| serde_json::from_value(v.clone())).transpose()?;
    }
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
        img: grainatlas_core::layout::AreaImage { source_size_um: Some([m.w as f64*m.um_per_px,m.h as f64*m.um_per_px]), edits: project.image_edits.as_ref().filter(|e| e.version==1).and_then(|e| e.areas.get(&rec.id)),
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
    if area.img.edits.is_some() { r.registration.mapping = "mount_um = band.paper_to_mount × [sheet_x_mm, sheet_y_mm, 1]; SVG affine order [a,b,c,d,e,f]".into(); }
    let mut files = vec![
        (
            format!("{stem}.pdf"),
            registered_pdf(&r.pages, &r.registration)?,
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

pub(super) fn registered_pdf(pages: &[String], registration: &grainatlas_core::layout::sheets::PrintRegistration) -> Result<Vec<u8>> {
    let mut pdf = grainatlas_core::layout::pdf::PdfBuilder::new();
    pdf.attach(&registration.registration_file, serde_json::to_vec_pretty(registration)?);
    for page in pages { pdf.prepare(page)?; }
    for page in pages { pdf.push(page)?; }
    Ok(pdf.finish())
}
