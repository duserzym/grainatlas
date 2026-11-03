//! HDR merge of a field captured several times at different brightness or
//! contrast, through `photocraft_algo::hdr`: Ward MTB alignment, exposure
//! estimation, Debevec response recovery and radiance merge, tone mapping.

use photocraft_algo::hdr::{MergeOptions, ToneMethod, estimate_exposures, merge, mtb_offset, tone_map};

use crate::image::Gray;

/// Result of an HDR merge of one field.
pub struct HdrMerge {
    pub image: Gray,
    /// Relative linear exposures found per input (darkest = 1).
    pub exposures: Vec<f64>,
    /// Alignment offset of each input to the first (px).
    pub offsets: Vec<(i32, i32)>,
    /// Dynamic range of the merge in stops.
    pub stops: f64,
}

fn to_rgba(g: &Gray) -> Vec<[f32; 4]> {
    g.px.iter().map(|&v| [v.clamp(0.0, 1.0), v.clamp(0.0, 1.0), v.clamp(0.0, 1.0), 1.0]).collect()
}

fn shift(g: &Gray, dx: i32, dy: i32) -> Gray {
    let mut out = Gray::new(g.w, g.h);
    for y in 0..g.h {
        for x in 0..g.w {
            out.px[y * g.w + x] = g.atc(x as i64 - dx as i64, y as i64 - dy as i64);
        }
    }
    out
}

/// Merges same-size exposures of one field and tone-maps the result back to 0..1.
pub fn merge_exposures(imgs: &[Gray]) -> Option<HdrMerge> {
    let first = imgs.first()?;
    if imgs.len() < 2 || imgs.iter().any(|g| g.w != first.w || g.h != first.h) {
        return None;
    }
    let (w, h) = (first.w, first.h);
    let levels = ((w.min(h) as f32).log2() as usize).saturating_sub(4).clamp(1, 6);
    let mut offsets = vec![(0, 0)];
    let mut aligned = vec![first.clone()];
    for g in &imgs[1..] {
        let (dx, dy) = mtb_offset(w, h, &first.px, &g.px, levels);
        offsets.push((dx, dy));
        aligned.push(if dx == 0 && dy == 0 { g.clone() } else { shift(g, dx, dy) });
    }
    let rgba: Vec<Vec<[f32; 4]>> = aligned.iter().map(to_rgba).collect();
    let refs: Vec<&[[f32; 4]]> = rgba.iter().map(|v| v.as_slice()).collect();
    let exposures = estimate_exposures(&refs);
    let merged = merge(w, h, &refs, &MergeOptions { exposures: exposures.clone(), remove_ghosts: false, ghost_base: None, response: None });
    let mut px = merged.px;
    tone_map(&mut px, w, h, &ToneMethod::HighlightCompression);
    let image = Gray { w, h, px: px.iter().map(|q| q[1].clamp(0.0, 1.0)).collect() };
    Some(HdrMerge { image, exposures, offsets, stops: merged.stops })
}
