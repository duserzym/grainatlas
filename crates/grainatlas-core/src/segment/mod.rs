//! Grain segmentation (plan section 6.5): Otsu threshold on the smoothed
//! mosaic, morphological open / close and hole filling, distance transform with
//! h-maxima markers and a marker-controlled watershed to split touching grains,
//! optional GrabCut refinement per grain (`photocraft_algo::segment::grabcut`),
//! polygons and statistics, reading-order numbering that is stable across
//! re-runs, and the manual tools (click-grow add, split, merge, renumber).

pub mod filter;
pub mod morph;
pub mod shape;
pub mod watershed;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::image::Gray;
use crate::photometric::background::{gaussian_blur, otsu};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Polarity {
    /// Grains brighter than the epoxy (CL, BSE, SE).
    #[default]
    Bright,
    /// Grains darker than the background (transmitted light).
    Dark,
    /// Bright grains as `Bright`, plus grains brighter or darker than the local epoxy level
    /// that overlap none of them: CL-dark (metamict) zircons in cathodoluminescence.
    Both,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThresholdMethod {
    /// Background level (histogram mode) plus a margin from the background noise and the
    /// grain contrast (ADR-0004): robust when grains have dark internal zones.
    #[default]
    Background,
    /// Otsu over the covered pixels.
    Otsu,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SegmentParams {
    pub polarity: Polarity,
    pub method: ThresholdMethod,
    /// Background method: threshold = bg + max(noise_k · σ, contrast_frac · (p99.5 − bg)).
    pub noise_k: f32,
    pub contrast_frac: f32,
    /// Threshold on the 0..1 mosaic; `None` = Otsu.
    pub threshold: Option<f32>,
    /// Gaussian smoothing before the threshold (work pixels).
    pub smooth_px: f32,
    pub open_px: usize,
    pub close_px: usize,
    /// Components smaller than a disc of this diameter are dropped.
    pub min_size_um: f64,
    /// Split touching grains by watershed.
    pub split: bool,
    /// h of the h-maxima markers (work pixels of distance): larger merges more.
    pub split_h: f32,
    /// Regions are kept apart only where the neck distance is below this fraction of the
    /// smaller region's largest distance.
    pub neck_ratio: f32,
    /// GrabCut refinement of every grain (slower).
    pub grabcut: bool,
    /// Douglas-Peucker tolerance of the outlines (work pixels).
    pub simplify_px: f64,
    /// Row tolerance for reading order, as a fraction of the median grain height.
    pub row_tolerance: f64,
    /// Work resolution: the mosaic is reduced by this factor.
    pub work_scale: usize,
    /// `Both`: a pixel is dark when it is below the local epoxy level by this fraction of that
    /// level (or by the noise margin, whichever is smaller).
    pub dark_rel: f32,
    /// Size of the running median that gives the local epoxy level (um).
    pub background_window_um: f64,
    /// Flag epoxy bubbles and pits (round dark blobs from 12 um to 0.68 x the reference size,
    /// solidity > 0.88, aspect < 1.35), keep their bright halo out of the grain mask, and flag
    /// halo crescents and bubble rims that survive.
    pub flag_bubbles: bool,
    /// Flag tile-seam slivers (minor axis < 15 um, or aspect > 5, minor axis < 30 um and straight
    /// along a tile axis) and cut seam lines that bridge grains with an opening of 0.037 x the
    /// reference size.
    pub flag_slivers: bool,
    /// Flag exposure-step tile blocks: weakly contrasted or box-shaped regions; blobs over 3 x the
    /// reference size are split back into their raw parts first (real grains sit inside them).
    pub flag_blocks: bool,
    /// Flag specks: smaller than 0.22 x the reference size and weakly contrasted or far from
    /// every grain.
    pub flag_specks: bool,
    /// Grain size the artifact sizes scale with (eq. diameter, um); 0 = the median of the area's
    /// grains (about 110 um on the Boise S mount, 175 um on the L mount).
    pub reference_size_um: f64,
    /// Largest grain kept (eq. diameter, um); 0 = no limit.
    pub max_size_um: f64,
}

impl Default for SegmentParams {
    fn default() -> Self {
        Self {
            polarity: Polarity::Bright,
            method: ThresholdMethod::Background,
            noise_k: 6.0,
            contrast_frac: 0.12,
            threshold: None,
            smooth_px: 1.0,
            open_px: 2,
            close_px: 3,
            min_size_um: 20.0,
            split: true,
            split_h: 4.0,
            neck_ratio: 0.75,
            grabcut: false,
            simplify_px: 0.8,
            row_tolerance: 0.6,
            work_scale: 2,
            dark_rel: 0.45,
            background_window_um: 330.0,
            flag_bubbles: true,
            flag_slivers: true,
            flag_blocks: true,
            flag_specks: true,
            reference_size_um: 0.0,
            max_size_um: 0.0,
        }
    }
}

/// Why a detection is not a grain. Artifacts stay in the grain list, unnumbered and dimmed, so
/// the user can confirm them or keep one as a grain; sheets, spots and ownership leave them out.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Artifact {
    /// An epoxy bubble or pit: a round dark blob.
    Bubble,
    /// A bubble's bright halo, a crescent of it, or a bubble rim.
    Halo,
    /// A tile-seam sliver or line.
    Sliver,
    /// An exposure-step tile block or a region of one.
    Block,
    /// A small, weak or isolated speck.
    Speck,
    /// Marked by the user.
    Manual,
}

impl Artifact {
    pub fn label(self) -> &'static str {
        match self {
            Artifact::Bubble => "bubble",
            Artifact::Halo => "halo",
            Artifact::Sliver => "sliver",
            Artifact::Block => "block",
            Artifact::Speck => "speck",
            Artifact::Manual => "manual",
        }
    }
}

/// One grain, in mosaic micrometres (y down).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Grain {
    pub id: Uuid,
    pub number: u32,
    pub polygon_um: Vec<[f64; 2]>,
    pub bbox_um: [f64; 4],
    pub centroid_um: [f64; 2],
    pub area_um2: f64,
    pub eq_diameter_um: f64,
    pub major_um: f64,
    pub minor_um: f64,
    /// Long-axis orientation, degrees from +x towards +y (y down), -90..90.
    pub orientation_deg: f64,
    /// Mean of the corrected mosaic (0..1) inside the grain.
    pub mean_intensity: f64,
    #[serde(default)]
    pub manual: bool,
    /// Per-grain scale bar length (plan section 6.6).
    pub scale_bar_um: f64,
    /// Not a grain, and why (`None` = a grain).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Artifact>,
    /// The user kept this detection as a grain although it was flagged; re-runs keep it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub kept: bool,
}

impl Grain {
    /// A grain, not an artifact.
    pub fn is_grain(&self) -> bool {
        self.artifact.is_none()
    }
}

/// Segmentation input: the corrected mosaic reduced to work resolution.
pub struct WorkImage {
    pub img: Gray,
    pub cov: Vec<bool>,
    /// Micrometres per work pixel.
    pub um_per_px: f64,
}

impl WorkImage {
    pub fn new(linear: &Gray, coverage: &[f32], mosaic_um_per_px: f64, scale: usize) -> WorkImage {
        let k = scale.max(1);
        let img = linear.downsample(k);
        let covg = Gray { w: linear.w, h: linear.h, px: coverage.to_vec() }.downsample(k);
        WorkImage { cov: covg.px.iter().map(|&c| c >= 0.5).collect(), img, um_per_px: mosaic_um_per_px * k as f64 }
    }
}

/// Threshold mask of grains (after smoothing, open, close, hole filling).
pub fn grain_mask(w: &WorkImage, p: &SegmentParams) -> (Vec<bool>, f32) {
    let sm = gaussian_blur(&w.img, p.smooth_px);
    let thr = p.threshold.unwrap_or_else(|| auto_threshold(&sm, &w.cov, p));
    let (iw, ih) = (sm.w, sm.h);
    let mut m: Vec<bool> = sm.px.iter().zip(&w.cov).map(|(&v, &c)| c && if p.polarity != Polarity::Dark { v > thr } else { v < thr }).collect();
    if p.open_px > 0 {
        m = morph::open(&m, iw, ih, p.open_px);
    }
    if p.close_px > 0 {
        m = morph::close(&m, iw, ih, p.close_px);
    }
    m = morph::fill_holes(&m, iw, ih);
    (m, thr)
}

/// Automatic threshold on a smoothed work image.
pub fn auto_threshold(sm: &Gray, cov: &[bool], p: &SegmentParams) -> f32 {
    let vals = || sm.px.iter().zip(cov).filter(|(_, c)| **c).map(|(v, _)| *v);
    match p.method {
        ThresholdMethod::Otsu => otsu(vals()),
        ThresholdMethod::Background => {
            let bright = p.polarity != Polarity::Dark;
            let mut h = vec![0u64; 1024];
            let mut n = 0u64;
            for v in vals() {
                let v = if bright { v } else { 1.0 - v };
                h[(v.clamp(0.0, 1.0) * 1023.0) as usize] += 1;
                n += 1;
            }
            if n == 0 {
                return 0.5;
            }
            // Background = histogram mode (smoothed); noise from the half width below the mode.
            let hs: Vec<f64> = (0..1024usize).map(|i| (i.saturating_sub(3)..(i + 4).min(1024)).map(|j| h[j] as f64).sum::<f64>()).collect();
            let mode = (0..1024).max_by(|&a, &b| hs[a].total_cmp(&hs[b])).unwrap_or(0);
            let half = hs[mode] / 2.0;
            let mut lo = mode;
            while lo > 0 && hs[lo] > half {
                lo -= 1;
            }
            let sigma = ((mode - lo) as f32 / 1023.0 / 1.1774).max(0.5 / 1023.0);
            let bg = mode as f32 / 1023.0;
            let mut acc = 0u64;
            let mut p995 = 1.0;
            for (i, &c) in h.iter().enumerate() {
                acc += c;
                if acc as f64 >= 0.995 * n as f64 {
                    p995 = i as f32 / 1023.0;
                    break;
                }
            }
            let t = bg + (p.noise_k * sigma).max(p.contrast_frac * (p995 - bg));
            if bright { t } else { 1.0 - t }
        }
    }
}

/// Grains of a threshold mask: labels (watershed split when asked), the size limit, outlines.
pub fn grains_of_mask(w: &WorkImage, p: &SegmentParams, mask: &[bool]) -> Vec<Grain> {
    let (iw, ih) = (w.img.w, w.img.h);
    let labels = if p.split {
        watershed::split(mask, iw, ih, p.split_h, p.neck_ratio)
    } else {
        morph::components(mask, iw, ih).0
    };
    let min_px = std::f64::consts::PI / 4.0 * (p.min_size_um / w.um_per_px).powi(2);
    let n = labels.iter().copied().max().unwrap_or(0) as usize;
    // Bounding boxes per label.
    let mut bb = vec![[usize::MAX, usize::MAX, 0usize, 0usize]; n + 1];
    let mut cnt = vec![0usize; n + 1];
    for (i, &l) in labels.iter().enumerate() {
        if l == 0 {
            continue;
        }
        let (x, y) = (i % iw, i / iw);
        let b = &mut bb[l as usize];
        b[0] = b[0].min(x);
        b[1] = b[1].min(y);
        b[2] = b[2].max(x);
        b[3] = b[3].max(y);
        cnt[l as usize] += 1;
    }
    let keep: Vec<u32> = (1..=n as u32).filter(|&l| cnt[l as usize] as f64 >= min_px).collect();
    crate::util::par_map(keep.len(), |k| {
        let l = keep[k];
        let b = bb[l as usize];
        let (bw, bh) = (b[2] - b[0] + 1, b[3] - b[1] + 1);
        let mut local = vec![false; bw * bh];
        for y in 0..bh {
            for x in 0..bw {
                local[y * bw + x] = labels[(b[1] + y) * iw + b[0] + x] == l;
            }
        }
        if p.grabcut
            && let Some((m2, o2, w2, h2)) = shape::grabcut_refine(&w.img, &local, (b[0], b[1]), bw, bh)
        {
            return shape::grain_from_mask(&m2, w2, h2, o2, w, p.simplify_px, false);
        }
        shape::grain_from_mask(&local, bw, bh, (b[0], b[1]), w, p.simplify_px, false)
    })
    .into_iter()
    .flatten()
    .collect()
}

/// Full automatic segmentation. `previous` grains keep their ids and numbers where they match,
/// and the user's artifact decisions (kept as a grain, or marked) carry over.
///
/// On top of the threshold segmentation: with `Polarity::Both` the CL-dark grains, and with the
/// `flag_*` switches the artifact classifier ([`filter`]), whose sizes scale with the area's
/// median grain size. Artifacts stay in the list with their reason, unnumbered.
pub fn segment(w: &WorkImage, p: &SegmentParams, previous: &[Grain]) -> Vec<Grain> {
    let (mask, _) = grain_mask(w, p);
    let first = grains_of_mask(w, p, &mask);
    let flags = p.flag_bubbles || p.flag_slivers || p.flag_blocks || p.flag_specks;
    let mut grains = first.clone();
    if p.polarity == Polarity::Both || flags {
        let mut r = if p.reference_size_um > 0.0 { p.reference_size_um } else { filter::reference_size(&first) };
        // The first estimate comes from raw detections, which split zoned grains into fragments;
        // when the whole grains found with it say otherwise, run again with theirs.
        for pass in 0..2 {
            grains = second_pass(w, p, &mask, first.clone(), r);
            if p.reference_size_um > 0.0 || pass == 1 {
                break;
            }
            let r2 = filter::reference_size(&grains);
            if (r2 / r - 1.0).abs() < 0.25 {
                break;
            }
            r = r2;
        }
    }
    number(&mut grains, previous, p.row_tolerance);
    grains
}

/// The artifact classifier and the CL-dark supplement over the primary detections, with the
/// sizes scaled to the reference grain size `r`.
fn second_pass(w: &WorkImage, p: &SegmentParams, mask: &[bool], mut grains: Vec<Grain>, r: f64) -> Vec<Grain> {
    let an = filter::analyse(w, p, r);
    // Bubble halos out of the bright mask, seam lines cut by an opening.
    if let Some(m2) = filter::clean_mask(mask, w, p, &an, r) {
        grains = grains_of_mask(w, p, &m2);
    }
    if p.flag_blocks {
        filter::split_blocks(&mut grains, w, p, &an, r);
    }
    if p.polarity == Polarity::Both {
        filter::merge_footprints(&mut grains, w, p, &an, r);
    }
    filter::drop_nested(&mut grains);
    filter::classify(&mut grains, w, p, &an, r);
    // A dark round blob inside a grain is an inclusion, not a bubble.
    let inside = |b: &Grain| grains.iter().any(|g| g.is_grain() && shape::inside(&g.polygon_um, b.centroid_um[0], b.centroid_um[1]));
    let bubbles: Vec<Grain> = an.bubble_grains.iter().filter(|b| !inside(b)).cloned().collect();
    grains.extend(bubbles);
    grains
}

/// Reading-order numbering (rows by centroid y with a tolerance, then x), keeping the ids and
/// numbers of grains that match `previous` (centroid inside or within half a diameter).
pub fn number(grains: &mut [Grain], previous: &[Grain], tolerance: f64) {
    let mut taken = vec![false; previous.len()];
    let mut matched = vec![false; grains.len()];
    for (gi, g) in grains.iter_mut().enumerate() {
        let best = previous
            .iter()
            .enumerate()
            .filter(|(k, _)| !taken[*k])
            .map(|(k, q)| (k, (q.centroid_um[0] - g.centroid_um[0]).hypot(q.centroid_um[1] - g.centroid_um[1]), q))
            .filter(|(_, d, q)| *d < 0.5 * q.eq_diameter_um.max(g.eq_diameter_um))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((k, _, q)) = best {
            taken[k] = true;
            matched[gi] = true;
            g.id = q.id;
            g.number = q.number;
            g.manual = q.manual;
            // The user's decisions carry over: kept as a grain, or marked as an artifact.
            if q.kept {
                g.kept = true;
                g.artifact = None;
            } else if q.artifact == Some(Artifact::Manual) {
                g.artifact = Some(Artifact::Manual);
            }
        }
    }
    // Artifacts have no number; grains keep theirs, new grains continue in reading order.
    for g in grains.iter_mut().filter(|g| !g.is_grain()) {
        g.number = 0;
    }
    let real: Vec<usize> = (0..grains.len()).filter(|&i| grains[i].is_grain()).collect();
    let sub: Vec<Grain> = real.iter().map(|&i| grains[i].clone()).collect();
    let order: Vec<usize> = reading_order(&sub, tolerance).into_iter().map(|k| real[k]).collect();
    let mut next = previous.iter().map(|q| q.number).max().unwrap_or(0).max(grains.iter().map(|g| g.number).max().unwrap_or(0));
    for i in order {
        if !matched[i] || grains[i].number == 0 {
            next += 1;
            grains[i].number = next;
        }
    }
    sort_grains(grains);
}

/// Grains by number, then the artifacts (by position).
pub fn sort_grains(grains: &mut [Grain]) {
    grains.sort_by(|a, b| a.artifact.is_some().cmp(&b.artifact.is_some()).then(a.number.cmp(&b.number)).then(a.centroid_um[0].total_cmp(&b.centroid_um[0])));
}

/// Indices of `grains` in reading order.
pub fn reading_order(grains: &[Grain], tolerance: f64) -> Vec<usize> {
    if grains.is_empty() {
        return Vec::new();
    }
    let mut hs: Vec<f64> = grains.iter().map(|g| g.bbox_um[3] - g.bbox_um[1]).collect();
    hs.sort_by(f64::total_cmp);
    let tol = hs[hs.len() / 2] * tolerance;
    let mut idx: Vec<usize> = (0..grains.len()).collect();
    idx.sort_by(|&a, &b| grains[a].centroid_um[1].total_cmp(&grains[b].centroid_um[1]));
    // A grain joins the current row if it overlaps the row's band: its top is above the
    // lowest centroid of the row, or its centroid is within the tolerance of the row's first.
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let (mut row_first, mut row_line) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for i in idx {
        let (y, top) = (grains[i].centroid_um[1], grains[i].bbox_um[1]);
        if rows.is_empty() || (top > row_line && y > row_first + tol) {
            rows.push(vec![i]);
            row_first = y;
            row_line = y;
        } else {
            rows.last_mut().expect("row").push(i);
            row_line = row_line.max(y);
        }
    }
    let mut out = Vec::with_capacity(grains.len());
    for mut r in rows {
        r.sort_by(|&a, &b| grains[a].centroid_um[0].total_cmp(&grains[b].centroid_um[0]));
        out.extend(r);
    }
    out
}

/// Renumbers all grains in reading order from 1, or (with `from`) sets one grain's number and
/// numbers the grains after it in reading order consecutively ("renumber from here").
pub fn renumber(grains: &mut [Grain], tolerance: f64, from: Option<(Uuid, u32)>) {
    // Artifacts have no number and take no place in the reading order.
    for g in grains.iter_mut().filter(|g| !g.is_grain()) {
        g.number = 0;
    }
    let real: Vec<usize> = (0..grains.len()).filter(|&i| grains[i].is_grain()).collect();
    let sub: Vec<Grain> = real.iter().map(|&i| grains[i].clone()).collect();
    let order: Vec<usize> = reading_order(&sub, tolerance).into_iter().map(|k| real[k]).collect();
    match from {
        None => {
            for (k, i) in order.into_iter().enumerate() {
                grains[i].number = k as u32 + 1;
            }
        }
        Some((id, n)) => {
            let Some(pos) = order.iter().position(|&i| grains[i].id == id) else { return };
            for (k, &i) in order[pos..].iter().enumerate() {
                grains[i].number = n + k as u32;
            }
        }
    }
    sort_grains(grains);
}

/// Click-grow: the grain containing (or nearest to) a point, grown by flood fill above the
/// local threshold within a window. `None` if the point is on background.
pub fn grain_at(w: &WorkImage, p: &SegmentParams, x_um: f64, y_um: f64) -> Option<Grain> {
    let (iw, ih) = (w.img.w, w.img.h);
    let cx = (x_um / w.um_per_px).round() as i64;
    let cy = (y_um / w.um_per_px).round() as i64;
    if cx < 0 || cy < 0 || cx >= iw as i64 || cy >= ih as i64 {
        return None;
    }
    // Window of 600 um around the click.
    let r = (300.0 / w.um_per_px) as i64;
    let (x0, y0) = ((cx - r).max(0) as usize, (cy - r).max(0) as usize);
    let (x1, y1) = (((cx + r) as usize).min(iw - 1), ((cy + r) as usize).min(ih - 1));
    let sub = gaussian_blur(&w.img.crop(x0, y0, x1 - x0 + 1, y1 - y0 + 1), p.smooth_px);
    let (sw, sh) = (sub.w, sub.h);
    let bg = otsu(w.img.px.iter().step_by(13).copied());
    let seed_v = sub.px[(cy as usize - y0) * sw + (cx as usize - x0)];
    let bright = p.polarity != Polarity::Dark;
    let inside = |v: f32, t: f32| if bright { v > t } else { v < t };
    // Threshold half way between the seed and the background level; the seed must be a grain.
    let t = p.threshold.unwrap_or_else(|| {
        let bgl = {
            let mut v: Vec<f32> = sub.px.iter().copied().filter(|&v| !inside(v, bg)).collect();
            v.sort_by(f32::total_cmp);
            v.get(v.len() / 2).copied().unwrap_or(0.0)
        };
        0.5 * (seed_v + bgl)
    });
    if !inside(seed_v, t) {
        return None;
    }
    let mut mask = vec![false; sw * sh];
    let mut stack = vec![(cy as usize - y0) * sw + (cx as usize - x0)];
    mask[stack[0]] = true;
    while let Some(i) = stack.pop() {
        let (x, y) = (i % sw, i / sw);
        let nb = [(x > 0).then(|| i - 1), (x + 1 < sw).then(|| i + 1), (y > 0).then(|| i - sw), (y + 1 < sh).then(|| i + sw)];
        for j in nb.into_iter().flatten() {
            if !mask[j] && inside(sub.px[j], t) {
                mask[j] = true;
                stack.push(j);
            }
        }
    }
    let mut m = morph::close(&mask, sw, sh, p.close_px.max(1));
    m = morph::fill_holes(&m, sw, sh);
    let mut g = shape::grain_from_mask(&m, sw, sh, (x0, y0), w, p.simplify_px, true)?;
    g.number = 0;
    Some(g)
}

/// Splits a grain along a line (um). Returns the pieces larger than 5 % of the grain.
pub fn split_grain(w: &WorkImage, g: &Grain, line: [f64; 4], simplify_px: f64) -> Vec<Grain> {
    let (mask, bw, bh, origin) = shape::rasterize(g, w.um_per_px, 2);
    let mut m = mask;
    // Cut: clear pixels within 1 px of the line.
    let k = w.um_per_px;
    let (ax, ay, bx, by) = (line[0] / k - origin.0 as f64, line[1] / k - origin.1 as f64, line[2] / k - origin.0 as f64, line[3] / k - origin.1 as f64);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = (dx * dx + dy * dy).max(1e-9);
    // Extend the line to cross the whole grain.
    for y in 0..bh {
        for x in 0..bw {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let t = ((px - ax) * dx + (py - ay) * dy) / len2;
            let (qx, qy) = (ax + t * dx, ay + t * dy);
            if (px - qx).hypot(py - qy) <= 1.0 {
                m[y * bw + x] = false;
            }
        }
    }
    let (lab, counts) = morph::components(&m, bw, bh);
    let total: usize = counts.iter().sum();
    let mut out = Vec::new();
    for l in 1..counts.len() {
        if counts[l] * 20 < total {
            continue;
        }
        let part: Vec<bool> = lab.iter().map(|&v| v as usize == l).collect();
        // Give the cut pixels back to the nearer part.
        let part = morph::dilate(&part, bw, bh, 1).iter().zip(&m).zip(&lab).map(|((d, _), &lv)| *d && (lv as usize == l || lv == 0)).collect::<Vec<bool>>();
        let part: Vec<bool> = part.iter().enumerate().map(|(i, &v)| v && shape::inside_polygon_px(g, w.um_per_px, origin, i % bw, i / bw)).collect();
        if let Some(mut ng) = shape::grain_from_mask(&part, bw, bh, origin, w, simplify_px, true) {
            ng.number = 0;
            out.push(ng);
        }
    }
    out
}

/// Merges grains into one (union, closed until connected; convex hull as a last resort).
pub fn merge_grains(w: &WorkImage, gs: &[Grain], simplify_px: f64) -> Option<Grain> {
    let first = gs.first()?;
    let k = w.um_per_px;
    let x0 = gs.iter().map(|g| g.bbox_um[0]).fold(f64::INFINITY, f64::min) / k;
    let y0 = gs.iter().map(|g| g.bbox_um[1]).fold(f64::INFINITY, f64::min) / k;
    let x1 = gs.iter().map(|g| g.bbox_um[2]).fold(f64::NEG_INFINITY, f64::max) / k;
    let y1 = gs.iter().map(|g| g.bbox_um[3]).fold(f64::NEG_INFINITY, f64::max) / k;
    let origin = ((x0.floor() as i64 - 4).max(0) as usize, (y0.floor() as i64 - 4).max(0) as usize);
    let (bw, bh) = ((x1.ceil() as usize + 5).saturating_sub(origin.0), (y1.ceil() as usize + 5).saturating_sub(origin.1));
    let mut m = vec![false; bw * bh];
    for g in gs {
        for y in 0..bh {
            for x in 0..bw {
                if shape::inside_polygon_px(g, k, origin, x, y) {
                    m[y * bw + x] = true;
                }
            }
        }
    }
    let mut r = 1;
    while morph::components(&m, bw, bh).1.len() > 2 && r < 12 {
        m = morph::close(&m, bw, bh, r);
        r += 1;
    }
    m = morph::fill_holes(&m, bw, bh);
    let mut g = shape::grain_from_mask(&m, bw, bh, origin, w, simplify_px, true)?;
    g.id = first.id;
    g.number = gs.iter().map(|q| q.number).min().unwrap_or(0);
    Some(g)
}

/// GrabCut refinement of one grain on demand.
pub fn refine_grain(w: &WorkImage, g: &Grain, simplify_px: f64) -> Option<Grain> {
    let (mask, bw, bh, origin) = shape::rasterize(g, w.um_per_px, 6);
    let (m2, o2, w2, h2) = shape::grabcut_refine(&w.img, &mask, origin, bw, bh)?;
    let mut ng = shape::grain_from_mask(&m2, w2, h2, o2, w, simplify_px, true)?;
    ng.id = g.id;
    ng.number = g.number;
    Some(ng)
}

/// Label image (u16, 0 = background) of the grains at work resolution, by number.
pub fn label_image(w: &WorkImage, grains: &[Grain]) -> Vec<u16> {
    let (iw, ih) = (w.img.w, w.img.h);
    let mut out = vec![0u16; iw * ih];
    for g in grains.iter().filter(|g| g.is_grain()) {
        let (mask, bw, bh, origin) = shape::rasterize(g, w.um_per_px, 0);
        for y in 0..bh {
            for x in 0..bw {
                let (gx, gy) = (origin.0 + x, origin.1 + y);
                if mask[y * bw + x] && gx < iw && gy < ih {
                    out[gy * iw + gx] = g.number.min(u16::MAX as u32) as u16;
                }
            }
        }
    }
    out
}

/// Grain table CSV (with stage positions when the area has them).
pub fn grains_csv(grains: &[Grain], stage: impl Fn(f64, f64) -> Option<[f64; 2]>, spot_counts: impl Fn(&Uuid) -> usize) -> String {
    let mut s = String::from("number,id,x_um,y_um,area_um2,eq_diameter_um,major_um,minor_um,orientation_deg,mean_intensity,bbox_x0_um,bbox_y0_um,bbox_x1_um,bbox_y1_um,scale_bar_um,stage_x_nm,stage_y_nm,spots,manual,artifact\n");
    for g in grains {
        let st = stage(g.centroid_um[0], g.centroid_um[1]);
        s.push_str(&format!(
            "{},{},{:.2},{:.2},{:.1},{:.2},{:.2},{:.2},{:.1},{:.4},{:.2},{:.2},{:.2},{:.2},{},{},{},{},{},{}\n",
            g.number,
            g.id,
            g.centroid_um[0],
            g.centroid_um[1],
            g.area_um2,
            g.eq_diameter_um,
            g.major_um,
            g.minor_um,
            g.orientation_deg,
            g.mean_intensity,
            g.bbox_um[0],
            g.bbox_um[1],
            g.bbox_um[2],
            g.bbox_um[3],
            g.scale_bar_um,
            st.map_or(String::new(), |s| format!("{:.0}", s[0])),
            st.map_or(String::new(), |s| format!("{:.0}", s[1])),
            spot_counts(&g.id),
            g.manual,
            g.artifact.map_or("", |a| a.label())
        ));
    }
    s
}
