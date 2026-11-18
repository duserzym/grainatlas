use super::*;
use crate::project::pipeline::AreaOutput;

pub(super) fn one(
    mut row: Row,
    rec: &AreaRecord,
    output: &AreaOutput,
    published: Option<&[u8]>,
    opts: &Options,
    ctl: &Ctl,
) -> Result<(Row, bool)> {
    ctl.check()?;
    let (px, kind) = source(
        output,
        published,
        opts,
        opts.display_tone.get(&rec.id).unwrap_or(&rec.params.tone),
    );
    let ras = raster(
        &row.grain,
        rec.um_per_px(),
        output.linear.w,
        output.linear.h,
        0.0,
        &px,
        ctl,
    )?;
    if !row.reference {
        let complete = ras.mask.iter().enumerate().all(|(j, &m)| {
            m == 0
                || output.coverage[(ras.y + j / ras.w) * output.linear.w + ras.x + j % ras.w] >= 0.5
        });
        if !complete || !ras.mask.contains(&255) {
            row.selected = false;
            return Ok((row, false));
        }
    }
    let mut values: Vec<f64> = ras
        .values
        .iter()
        .zip(&ras.mask)
        .filter_map(|(&v, &m)| (m != 0).then_some(v))
        .collect();
    row.stats = stats::describe(&mut values);
    if row.selected {
        (row.stats.glcm_contrast, row.stats.glcm_homogeneity) =
            stats::texture(&ras.values, &ras.mask, ras.w, ras.h, &values, ctl)?;
    }
    row.pixel_source = kind.into();
    // Published output placement is synthetic, never a known SEM tile.
    row.tile = if published.is_some() {
        None
    } else {
        let x = row.grain.centroid_um[0] / rec.um_per_px();
        let y = row.grain.centroid_um[1] / rec.um_per_px();
        output
            .placement
            .tiles
            .iter()
            .filter(|t| {
                !t.failed
                    && x >= t.x as f64
                    && y >= t.y as f64
                    && x < (t.x + t.w as i64) as f64
                    && y < (t.y + t.h as i64) as f64
            })
            .min_by(|a, b| {
                let d = |t: &crate::mosaic::TilePlacement| {
                    (x - t.x as f64 - t.w as f64 / 2.0).hypot(y - t.y as f64 - t.h as f64 / 2.0)
                };
                d(a).total_cmp(&d(b)).then(a.index.cmp(&b.index))
            })
            .map(|t| t.index)
    };
    Ok((row, true))
}

/// Up to eight small rasters (at most about 16 MiB of working pixels) in
/// parallel on native. A large grain is processed alone. Wasm yields per grain.
pub(super) fn batch_end(rows: &[Row], start: usize, rec: &AreaRecord) -> usize {
    let limit = if cfg!(target_arch = "wasm32") { 1 } else { 8 };
    let mut pixels = 0.0;
    let mut end = start;
    for row in rows
        .iter()
        .skip(start)
        .take(limit)
        .take_while(|r| r.area == rec.id)
    {
        let b = row.grain.polygon_um.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            },
        );
        let size =
            ((b[2] - b[0]) / rec.um_per_px() + 2.0) * ((b[3] - b[1]) / rec.um_per_px() + 2.0);
        if end > start && pixels + size > 1_000_000.0 {
            break;
        }
        pixels += size;
        end += 1;
    }
    end
}
