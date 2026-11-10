//! Hand editing of grain outlines, for grains the segmentation cannot get right (CL-dark cores
//! with bright rims, grains touching bubble halos, sector zoning).
//!
//! - A drawn outline (freehand lasso or clicked polygon), optionally snapped to the image's edges
//!   with PhotoCraft's Magnetic Lasso (`photocraft_algo::magnetic`, the live wire of Mortensen &
//!   Barrett).
//! - A brush stroke grown to the nearby edges with PhotoCraft's Quick Selection
//!   (`photocraft_algo::segment::quick`, a graph cut after Boykov & Jolly).
//!
//! Either makes a new grain, adds to a grain or cuts from it. The result is `drawn`: a re-run of
//! the segmentation keeps it as it is.

use photocraft_algo::magnetic::{Settings, Tracer};
use photocraft_algo::segment::Sampler;
use photocraft_algo::segment::quick::quick_select;
use photocraft_geom::Rect;
use serde::Deserialize;

use super::{Grain, WorkImage, morph, shape};

/// What an edit does with the grain it is applied to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    /// The stroke or outline is the grain.
    New,
    /// The grain grows by it.
    Add,
    /// The grain loses it.
    Cut,
}

/// The toned 8-bit mosaic at full resolution: what the user sees, and what the tools read.
pub struct View<'a> {
    pub w: usize,
    pub h: usize,
    pub px: &'a [u8],
    /// Micrometres per mosaic pixel.
    pub um_per_px: f64,
}

impl View<'_> {
    fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w as i32, self.h as i32)
    }

    /// The 3 x 3 mean at (x, y): CL mosaics are noisy at full resolution, and the edge tracer
    /// should follow grain boundaries, not single noisy pixels.
    fn smooth(&self, x: i32, y: i32) -> u8 {
        let (mut s, mut n) = (0u32, 0u32);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (u, v) = (x + dx, y + dy);
                if u >= 0 && v >= 0 && (u as usize) < self.w && (v as usize) < self.h {
                    s += u32::from(self.px[v as usize * self.w + u as usize]);
                    n += 1;
                }
            }
        }
        (s / n.max(1)) as u8
    }
}

impl Sampler for View<'_> {
    fn rgba(&self, r: Rect) -> Vec<[f32; 4]> {
        let mut out = Vec::with_capacity(r.width() as usize * r.height() as usize);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let v = f32::from(self.smooth(x, y)) / 255.0;
                out.push([v, v, v, 1.0]);
            }
        }
        out
    }
}

/// A closed outline through the pointer's path `path_um`, running along the strongest edges
/// within `width_um` of it (where there is no edge stronger than `contrast`, 0..1, it keeps to
/// the path). The path is closed back to its start the same way.
pub fn snap_outline(v: &View, path_um: &[[f64; 2]], width_um: f64, contrast: f32) -> Vec<[f64; 2]> {
    if path_um.len() < 3 || v.w == 0 || v.h == 0 {
        return path_um.to_vec();
    }
    let k = v.um_per_px;
    let px: Vec<[f64; 2]> = path_um.iter().map(|p| [p[0] / k, p[1] / k]).collect();
    let s = Settings::new(width_um / k, contrast);
    let mut tracer = Tracer::new(v.bounds());
    let mut fetch = |r: Rect| -> Vec<[u8; 4]> {
        let mut out = Vec::with_capacity(r.width() as usize * r.height() as usize);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let g = v.smooth(x, y);
                out.push([g, g, g, 255]);
            }
        }
        out
    };
    // Fastening points along the path, as the Magnetic Lasso places them while the pointer
    // moves: each stretch is traced between two nearby points, so the cheapest path can never
    // cut across the loop to its other side.
    let step = (6.0 * s.width).max(30.0);
    let mut anchors = vec![0usize];
    let mut acc = 0.0;
    for i in 1..px.len() {
        acc += (px[i][0] - px[i - 1][0]).hypot(px[i][1] - px[i - 1][1]);
        if acc >= step {
            anchors.push(i);
            acc = 0.0;
        }
    }
    if *anchors.last().expect("one") != px.len() - 1 {
        anchors.push(px.len() - 1);
    }
    let fixed: Vec<[f64; 2]> = anchors.iter().map(|&i| tracer.snap(&mut fetch, px[i], s).unwrap_or(px[i])).collect();
    let mut out = vec![fixed[0]];
    for (k2, w2) in anchors.windows(2).enumerate() {
        let piece = tracer.trace(&mut fetch, fixed[k2], fixed[k2 + 1], &px[w2[0] + 1..w2[1]], s);
        out.extend(piece.into_iter().skip(1));
    }
    let (first, last) = (fixed[0], *fixed.last().expect("one"));
    let gap = (last[0] - first[0]).hypot(last[1] - first[1]);
    if gap > 1.0 {
        // Back to the start, in steps too when the gap is long.
        let m = (gap / step).ceil().max(1.0) as usize;
        let mut cur = last;
        for i in 1..=m {
            let t = i as f64 / m as f64;
            let q = [last[0] + (first[0] - last[0]) * t, last[1] + (first[1] - last[1]) * t];
            let q = if i == m { first } else { tracer.snap(&mut fetch, q, s).unwrap_or(q) };
            out.extend(tracer.trace(&mut fetch, cur, q, &[], s).into_iter().skip(1));
            cur = q;
        }
    }
    if out.len() > 1 && out.first() == out.last() {
        out.pop();
    }
    if out.len() < 3 {
        return path_um.to_vec();
    }
    out.iter().map(|p| [p[0] * k, p[1] * k]).collect()
}

/// A mask at work resolution placed at `origin` (work pixels).
struct Local {
    m: Vec<bool>,
    w: usize,
    h: usize,
    origin: (usize, usize),
}

impl Local {
    fn get(&self, gx: usize, gy: usize) -> bool {
        gx >= self.origin.0 && gy >= self.origin.1 && gx < self.origin.0 + self.w && gy < self.origin.1 + self.h && self.m[(gy - self.origin.1) * self.w + gx - self.origin.0]
    }
}

/// The work pixels of a box (um) padded by `pad`, clamped to the image.
fn window(w: &WorkImage, bbox_um: [f64; 4], pad: usize) -> (usize, usize, usize, usize) {
    let k = w.um_per_px;
    let x0 = ((bbox_um[0] / k).floor() as i64 - pad as i64).max(0) as usize;
    let y0 = ((bbox_um[1] / k).floor() as i64 - pad as i64).max(0) as usize;
    let x1 = (((bbox_um[2] / k).ceil() as i64 + pad as i64).max(0) as usize).min(w.img.w);
    let y1 = (((bbox_um[3] / k).ceil() as i64 + pad as i64).max(0) as usize).min(w.img.h);
    (x0, y0, x1.max(x0 + 1), y1.max(y0 + 1))
}

fn bbox_of(p: &[[f64; 2]]) -> [f64; 4] {
    p.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, q| [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])])
}

/// The inside of a closed outline (um), holes filled (a lasso may cross itself).
fn polygon_mask(w: &WorkImage, poly_um: &[[f64; 2]]) -> Local {
    let k = w.um_per_px;
    let (x0, y0, x1, y1) = window(w, bbox_of(poly_um), 1);
    let (bw, bh) = (x1 - x0, y1 - y0);
    let mut m = vec![false; bw * bh];
    for y in 0..bh {
        for x in 0..bw {
            m[y * bw + x] = shape::inside(poly_um, (x0 + x) as f64 * k + 0.5 * k, (y0 + y) as f64 * k + 0.5 * k);
        }
    }
    Local { m: morph::fill_holes(&m, bw, bh), w: bw, h: bh, origin: (x0, y0) }
}

/// A brush stroke (um) of diameter `size_um`, grown to the nearby edges by Quick Selection on
/// the mosaic, at work resolution.
fn brush_mask(w: &WorkImage, v: &View, stroke_um: &[[f64; 2]], size_um: f64) -> Option<Local> {
    let kv = v.um_per_px;
    let pts: Vec<(f32, f32)> = stroke_um.iter().map(|p| ((p[0] / kv) as f32, (p[1] / kv) as f32)).collect();
    let region = quick_select(v, v.bounds(), &pts, (size_um / kv) as f32)?;
    let b = region.bbox;
    let bbox_um = [f64::from(b.x0) * kv, f64::from(b.y0) * kv, f64::from(b.x1) * kv, f64::from(b.y1) * kv];
    let (x0, y0, x1, y1) = window(w, bbox_um, 1);
    let (bw, bh) = (x1 - x0, y1 - y0);
    let k = w.um_per_px;
    let mut m = vec![false; bw * bh];
    for y in 0..bh {
        for x in 0..bw {
            // The mosaic pixel under the work pixel's centre.
            let mx = (((x0 + x) as f64 + 0.5) * k / kv).floor() as i32;
            let my = (((y0 + y) as f64 + 0.5) * k / kv).floor() as i32;
            m[y * bw + x] = region.at(mx, my) >= 0.5;
        }
    }
    Some(Local { m: morph::fill_holes(&m, bw, bh), w: bw, h: bh, origin: (x0, y0) })
}

/// Applies a stroke or outline to `base` (or makes a grain of it). The grain keeps `base`'s id
/// and number; `None` when nothing is left (a cut through the whole grain) or nothing was drawn.
fn apply(w: &WorkImage, stroke: &Local, base: Option<&Grain>, op: Op, simplify_px: f64) -> Option<Grain> {
    let base = if op == Op::New { None } else { base };
    if op == Op::Cut && base.is_none() {
        return None;
    }
    let k = w.um_per_px;
    let sb = [stroke.origin.0 as f64 * k, stroke.origin.1 as f64 * k, (stroke.origin.0 + stroke.w) as f64 * k, (stroke.origin.1 + stroke.h) as f64 * k];
    let bb = base.map_or(sb, |g| g.bbox_um);
    let union = [sb[0].min(bb[0]), sb[1].min(bb[1]), sb[2].max(bb[2]), sb[3].max(bb[3])];
    let (x0, y0, x1, y1) = window(w, union, 3);
    let (bw, bh) = (x1 - x0, y1 - y0);
    let mut m = vec![false; bw * bh];
    for y in 0..bh {
        for x in 0..bw {
            let s = stroke.get(x0 + x, y0 + y);
            let g = base.is_some_and(|g| shape::inside_polygon_px(g, k, (x0, y0), x, y));
            m[y * bw + x] = match op {
                Op::New => s,
                Op::Add => s || g,
                Op::Cut => g && !s,
            };
        }
    }
    if op == Op::Add {
        // A stroke beside the grain rather than over it: join the two.
        let mut r = 1;
        while morph::components(&m, bw, bh).1.len() > 2 && r < 12 {
            m = morph::close(&m, bw, bh, r);
            r += 1;
        }
    }
    if op != Op::Cut {
        m = morph::fill_holes(&m, bw, bh);
    }
    let mut g = shape::grain_from_mask(&m, bw, bh, (x0, y0), w, simplify_px, true)?;
    g.drawn = true;
    if let Some(b) = base {
        g.id = b.id;
        g.number = b.number;
        g.kept = b.kept;
    } else {
        g.number = 0;
    }
    Some(g)
}

/// A grain from a drawn outline (um): a new grain, or `base` grown or cut by it.
pub fn outline_grain(w: &WorkImage, poly_um: &[[f64; 2]], base: Option<&Grain>, op: Op, simplify_px: f64) -> Option<Grain> {
    if poly_um.len() < 3 {
        return None;
    }
    apply(w, &polygon_mask(w, poly_um), base, op, simplify_px)
}

/// Pads the newly drawn region before adding it, so the existing part of a grain stays put.
/// Cuts and disabled padding use the original editing path exactly.
pub fn outline_grain_padded(w: &WorkImage, poly_um: &[[f64; 2]], base: Option<&Grain>, op: Op, simplify_px: f64, outline: super::OutlinePad) -> Option<Grain> {
    if op == Op::Cut || outline.margin_um <= 0.0 && !outline.smooth {
        return outline_grain(w, poly_um, base, op, simplify_px);
    }
    if poly_um.len() < 3 {
        return None;
    }
    let stroke = polygon_mask(w, poly_um);
    let padded = super::pad::pad_mask(w, &stroke.m, stroke.w, stroke.h, stroke.origin, outline);
    apply(w, &Local { m: padded.mask, w: padded.w, h: padded.h, origin: padded.origin }, base, op, simplify_px)
}

/// A grain from a brush stroke (um, brush diameter `size_um`) grown to the edges near it.
pub fn brush_grain(w: &WorkImage, v: &View, stroke_um: &[[f64; 2]], size_um: f64, base: Option<&Grain>, op: Op, simplify_px: f64) -> Option<Grain> {
    if stroke_um.is_empty() || size_um <= 0.0 {
        return None;
    }
    let stroke = brush_mask(w, v, stroke_um, size_um)?;
    apply(w, &stroke, base, op, simplify_px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::Gray;

    /// A 200 x 120 px image (1 um/px): a bright 60 x 40 px block at (40, 40) with a dark core,
    /// on a mid-grey epoxy.
    fn scene() -> (Vec<u8>, WorkImage) {
        let (w, h) = (200usize, 120usize);
        let mut px = vec![90u8; w * h];
        for y in 40..80 {
            for x in 40..100 {
                let core = (55..65).contains(&y) && (55..85).contains(&x);
                px[y * w + x] = if core { 20 } else { 220 };
            }
        }
        let img = Gray { w, h, px: px.iter().map(|&v| f32::from(v) / 255.0).collect() };
        (px, WorkImage { tiles_um: Vec::new(), signal: crate::segment::Signal::Cl, img, cov: vec![true; w * h], um_per_px: 1.0 })
    }

    #[test]
    fn a_rough_lasso_snaps_to_the_block() {
        let (px, _) = scene();
        let v = View { w: 200, h: 120, px: &px, um_per_px: 1.0 };
        // About 4 um outside the block, all round.
        let path = [[37.0, 37.0], [70.0, 35.5], [103.0, 37.0], [104.0, 60.0], [103.0, 83.0], [70.0, 84.0], [36.0, 83.0], [35.5, 60.0]];
        let snapped = snap_outline(&v, &path, 8.0, 0.1);
        let b = bbox_of(&snapped);
        for (got, want) in b.iter().zip([40.0, 40.0, 100.0, 80.0]) {
            assert!((got - want).abs() < 1.5, "snapped box {b:?}");
        }
    }

    #[test]
    fn new_add_and_cut() {
        let (_, w) = scene();
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        let g = outline_grain(&w, &rect(40.0, 40.0, 70.0, 80.0), None, Op::New, 0.8).unwrap();
        assert!(g.drawn && g.manual && (g.area_um2 - 1200.0).abs() < 80.0, "{}", g.area_um2);
        // Add the other half, overlapping a little: one grain, same id.
        let a = outline_grain(&w, &rect(65.0, 40.0, 100.0, 80.0), Some(&g), Op::Add, 0.8).unwrap();
        assert_eq!(a.id, g.id);
        assert!((a.area_um2 - 2400.0).abs() < 120.0, "{}", a.area_um2);
        // A stroke beside the grain, not touching it, is joined too.
        let j = outline_grain(&w, &rect(73.0, 50.0, 90.0, 70.0), Some(&g), Op::Add, 0.8).unwrap();
        assert!(j.area_um2 > g.area_um2 + 300.0, "{}", j.area_um2);
        // Cut the right half off again.
        let c = outline_grain(&w, &rect(70.0, 30.0, 110.0, 90.0), Some(&a), Op::Cut, 0.8).unwrap();
        assert_eq!(c.id, g.id);
        assert!((c.area_um2 - 1200.0).abs() < 80.0, "{}", c.area_um2);
        // Cutting everything leaves nothing; cutting with no grain does nothing.
        assert!(outline_grain(&w, &rect(30.0, 30.0, 110.0, 90.0), Some(&a), Op::Cut, 0.8).is_none());
        assert!(outline_grain(&w, &rect(30.0, 30.0, 110.0, 90.0), None, Op::Cut, 0.8).is_none());
    }

    #[test]
    fn padding_grows_the_drawn_region_without_padding_the_base() {
        let (_, w) = scene();
        let rect = |x0, x1| [[x0, 40.0], [x1, 40.0], [x1, 80.0], [x0, 80.0]];
        let pad = super::super::OutlinePad::default();
        let raw = outline_grain(&w, &rect(40.0, 100.0), None, Op::New, 0.8).unwrap();
        let padded = outline_grain_padded(&w, &rect(40.0, 100.0), None, Op::New, 0.8, pad).unwrap();
        for (got, want) in padded.bbox_um.iter().zip([36.0, 36.0, 104.0, 84.0]) {
            assert!((got - want).abs() <= w.um_per_px, "{:?}", padded.bbox_um);
        }
        let added = outline_grain_padded(&w, &rect(90.0, 120.0), Some(&raw), Op::Add, 0.8, pad).unwrap();
        assert_eq!(added.bbox_um[0], raw.bbox_um[0], "the base's far rim stays put");
        assert_eq!(added.id, raw.id);
        assert!(added.drawn && added.manual);
        assert!(added.bbox_um[2] >= 123.0);
        let off = super::super::OutlinePad { margin_um: 0.0, smooth: false };
        let exact = outline_grain_padded(&w, &rect(40.0, 100.0), None, Op::New, 0.8, off).unwrap();
        assert_eq!(exact.polygon_um, raw.polygon_um);
        assert_eq!(exact.area_um2, raw.area_um2);
    }

    #[test]
    fn find_again_keeps_a_drawn_outline() {
        let (_, w) = scene();
        let p = crate::segment::SegmentParams::default();
        let auto = crate::segment::segment(&w, &p, &[]);
        let target = auto.iter().find(|g| g.is_grain() && super::shape::inside(&g.polygon_um, 70.0, 45.0)).expect("the block is found");
        // Redraw it as the left half only, then run the segmentation again.
        let rect = [[40.0, 40.0], [70.0, 40.0], [70.0, 80.0], [40.0, 80.0]];
        let mut drawn = outline_grain(&w, &rect, None, Op::New, 0.8).unwrap();
        drawn.id = target.id;
        drawn.number = target.number;
        let prev: Vec<Grain> = auto.iter().map(|g| if g.id == target.id { drawn.clone() } else { g.clone() }).collect();
        let again = crate::segment::segment(&w, &p, &prev);
        let g = again.iter().find(|g| g.id == target.id).expect("kept");
        assert!(g.drawn && g.polygon_um == drawn.polygon_um && g.number == target.number);
        assert_eq!(again.iter().filter(|g| g.is_grain() && super::shape::inside(&g.polygon_um, 55.0, 45.0)).count(), 1, "no detection on top of it");
    }

    #[test]
    fn a_brush_stroke_over_the_rim_takes_the_whole_block() {
        let (px, w) = scene();
        let v = View { w: 200, h: 120, px: &px, um_per_px: 1.0 };
        // A short stroke across the bright rim and the dark core.
        let g = brush_grain(&w, &v, &[[50.0, 50.0], [60.0, 58.0], [70.0, 60.0]], 10.0, None, Op::New, 0.8).unwrap();
        let b = g.bbox_um;
        assert!(b[0] > 35.0 && b[1] > 35.0 && b[2] < 105.0 && b[3] < 85.0, "stays on the block: {b:?}");
        assert!(g.area_um2 > 300.0, "{}", g.area_um2);
    }
}
