use super::{Artifact, Grain, Signal, WorkImage, filter, morph, shape};

/// Compatibility wrapper for callers drawing a straight line in micrometres.
pub fn split_grain(w: &WorkImage, g: &Grain, line: [f64; 4], simplify_px: f64) -> Vec<Grain> {
    split_grain_path(w, g, &[[line[0], line[1]], [line[2], line[3]]], simplify_px)
}

/// Split along a polyline in micrometres, extending each open end out of the grain.
pub fn split_grain_path(
    w: &WorkImage,
    g: &Grain,
    path_um: &[[f64; 2]],
    simplify_px: f64,
) -> Vec<Grain> {
    split_grain_for_signal(w, g, path_um, simplify_px, w.signal)
}

pub(crate) fn split_grain_for_signal(
    w: &WorkImage,
    g: &Grain,
    path_um: &[[f64; 2]],
    simplify_px: f64,
    signal: Signal,
) -> Vec<Grain> {
    if path_um.len() < 2 || path_um.iter().flatten().any(|v| !v.is_finite()) {
        return Vec::new();
    }
    let mut path = path_um.to_vec();
    path.dedup();
    if path.len() < 2 {
        return Vec::new();
    }
    let (mut m, bw, bh, origin) = shape::rasterize(g, w.um_per_px, 2);
    for p in &mut path {
        p[0] = p[0] / w.um_per_px - origin.0 as f64;
        p[1] = p[1] / w.um_per_px - origin.1 as f64;
    }
    let last = path.len() - 1;
    let start = extend(path[0], path[1], bw, bh);
    let end = extend(path[last], path[last - 1], bw, bh);
    path.insert(0, start);
    path.push(end);
    for segment in path.windows(2) {
        stamp(&mut m, bw, bh, segment[0], segment[1]);
    }
    let (lab, counts) = morph::components(&m, bw, bh);
    let total: usize = counts.iter().sum();
    let mut out = Vec::new();
    for (l, &count) in counts.iter().enumerate().skip(1) {
        if count * 20 < total {
            continue;
        }
        let part: Vec<bool> = lab.iter().map(|&v| v as usize == l).collect();
        // Give the cut pixels back to the nearer part.
        let part = morph::dilate(&part, bw, bh, 1)
            .iter()
            .zip(&m)
            .zip(&lab)
            .map(|((d, _), &lv)| *d && (lv as usize == l || lv == 0))
            .collect::<Vec<bool>>();
        let part: Vec<bool> = part
            .iter()
            .enumerate()
            .map(|(i, &v)| v && shape::inside_polygon_px(g, w.um_per_px, origin, i % bw, i / bw))
            .collect();
        if let Some(mut ng) = shape::grain_from_mask(&part, bw, bh, origin, w, simplify_px, true) {
            ng.number = 0;
            ng.sample = g.sample.clone();
            // One cut detaches a bubble: a round part far darker than the epoxy is flagged.
            if signal == Signal::Cl && filter::looks_like_bubble(&ng, g, w) {
                ng.artifact = Some(Artifact::Bubble);
                ng.manual = false;
            }
            out.push(ng);
        }
    }
    out.sort_by(|a, b| b.area_um2.total_cmp(&a.area_um2));
    if out.len() < 2 { Vec::new() } else { out }
}

// Continue the ray from `neighbour` through `at` beyond the padded raster bbox.
fn extend(at: [f64; 2], neighbour: [f64; 2], bw: usize, bh: usize) -> [f64; 2] {
    let d = [at[0] - neighbour[0], at[1] - neighbour[1]];
    let mut exit = f64::INFINITY;
    for (axis, size) in [bw, bh].into_iter().enumerate() {
        if d[axis] > 0.0 {
            exit = exit.min((size as f64 + 2.0 - at[axis]) / d[axis]);
        }
        if d[axis] < 0.0 {
            exit = exit.min((-2.0 - at[axis]) / d[axis]);
        }
    }
    if !exit.is_finite() || exit <= 0.0 {
        return at;
    }
    [at[0] + exit * d[0], at[1] + exit * d[1]]
}

// Stamp only the padded segment bbox, narrowed per row even for long diagonals.
// Work is proportional to segment length times cut width, rather than bbox area.
fn stamp(mask: &mut [bool], bw: usize, bh: usize, a: [f64; 2], b: [f64; 2]) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    if len2 <= 0.0 {
        return;
    }
    let y0 = (a[1].min(b[1]) - 1.5).ceil().max(0.0) as usize;
    let y1 = ((a[1].max(b[1]) + 1.0).ceil().max(0.0) as usize).min(bh);
    for y in y0..y1 {
        let py = y as f64 + 0.5;
        let (t0, t1) = if dy.abs() > 1e-12 {
            let t0 = (py - 1.0 - a[1]) / dy;
            let t1 = (py + 1.0 - a[1]) / dy;
            (t0.min(t1).max(0.0), t0.max(t1).min(1.0))
        } else {
            (0.0, 1.0)
        };
        if t0 > t1 {
            continue;
        }
        let (x0, x1) = (a[0] + t0 * dx, a[0] + t1 * dx);
        let lo = (x0.min(x1) - 1.5).ceil().max(0.0) as usize;
        let hi = ((x0.max(x1) + 1.0).ceil().max(0.0) as usize).min(bw);
        for x in lo..hi {
            let px = x as f64 + 0.5;
            let projection = (px - a[0]) * dx + (py - a[1]) * dy;
            // Avoid subtracting a rounded projected point: an exact 1 px hit
            // (for example on a 3:4 diagonal) must remain part of the cut.
            let cut = if projection < 0.0 {
                (px - a[0]).powi(2) + (py - a[1]).powi(2) <= 1.0
            } else if projection > len2 {
                (px - b[0]).powi(2) + (py - b[1]).powi(2) <= 1.0
            } else {
                ((px - a[0]) * dy - (py - a[1]) * dx).powi(2) <= len2
            };
            if cut {
                mask[y * bw + x] = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::stamp;

    #[test]
    fn bounded_stamps_match_point_to_segment_distance_in_every_direction() {
        let (w, h) = (80, 60);
        for i in 0..100 {
            let a = [(i * 17 % 101) as f64 - 10.5, (i * 13 % 83) as f64 - 10.5];
            let b = [(i * 23 % 103) as f64 - 10.5, (i * 19 % 89) as f64 - 10.5];
            if a == b {
                continue;
            }
            let mut mask = vec![true; w * h];
            stamp(&mut mask, w, h, a, b);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            for y in 0..h {
                for x in 0..w {
                    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                    let t = (((px - a[0]) * dx + (py - a[1]) * dy) / (dx * dx + dy * dy))
                        .clamp(0.0, 1.0);
                    // The full scan oracle uses the closest point directly. Its tiny
                    // tolerance includes exact-radius hits despite projection rounding.
                    let cut = (px - a[0] - t * dx).hypot(py - a[1] - t * dy) <= 1.0 + 1e-12;
                    assert_eq!(!mask[y * w + x], cut, "segment {a:?}–{b:?}, pixel {x},{y}");
                }
            }
        }
    }
}
