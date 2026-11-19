//! Invented geometry only; no lab images or registered data fixtures.
use grainatlas_core::image::Gray;
use grainatlas_core::segment::{self, Artifact, Grain, Signal, WorkImage, filter, morph, shape};

fn fixture() -> (WorkImage, Grain) {
    let w = WorkImage {
        tiles_um: Vec::new(),
        signal: Signal::Cl,
        um_per_px: 1.0,
        img: Gray {
            w: 220,
            h: 220,
            px: vec![0.5; 220 * 220],
        },
        cov: vec![true; 220 * 220],
    };
    let g = serde_json::from_value(serde_json::json!({
        "id": "00000000-0000-0000-0000-000000000001", "number": 1, "sample": "synthetic",
        "polygon_um": [[20.,20.],[180.,20.],[180.,180.],[20.,180.]],
        "bbox_um": [20.,20.,180.,180.], "centroid_um": [100.,100.], "area_um2": 25600.,
        "eq_diameter_um": 180., "major_um": 160., "minor_um": 160., "orientation_deg": 0.,
        "mean_intensity": 0.5, "manual": false, "scale_bar_um": 20.
    }))
    .unwrap();
    (w, g)
}

#[test]
fn freehand_s_curve_preserves_the_curve_and_parent_sample() {
    let (w, g) = fixture();
    let path = [
        [10., 100.],
        [50., 70.],
        [90., 85.],
        [110., 115.],
        [150., 130.],
        [190., 100.],
    ];
    let parts = segment::split_grain_path(&w, &g, &path, 0.5);
    assert_eq!(parts.len(), 2);
    assert!(parts.iter().all(|p| p.sample == g.sample));
    let upper = parts.iter().find(|p| p.centroid_um[1] < 100.).unwrap();
    // These probes lie on opposite sides of the curve, but the same side of
    // the horizontal chord: the intermediate path points must affect the cut.
    assert!(!shape::inside_polygon_px(upper, 1., (0, 0), 50, 85));
    assert!(shape::inside_polygon_px(upper, 1., (0, 0), 150, 115));
}

#[test]
fn open_ends_inside_the_grain_are_extended_in_their_own_directions() {
    let (w, g) = fixture();
    let parts = segment::split_grain_path(&w, &g, &[[70., 70.], [100., 100.], [130., 70.]], 0.5);
    assert_eq!(parts.len(), 2);
    assert!(parts.iter().all(|p| p.area_um2 > g.area_um2 * 0.05));
}

#[test]
fn two_passes_make_three_pieces() {
    let (w, g) = fixture();
    let parts = segment::split_grain_path(
        &w,
        &g,
        &[[10., 70.], [190., 70.], [190., 130.], [10., 130.]],
        0.5,
    );
    assert_eq!(parts.len(), 3);
    assert!(parts.windows(2).all(|p| p[0].area_um2 >= p[1].area_um2));
}

#[test]
fn tiny_sliver_is_dropped_and_a_lone_survivor_is_a_noop() {
    let (w, g) = fixture();
    let path = [[23., 10.], [23., 190.], [100., 190.], [100., 10.]];
    let parts = segment::split_grain_path(&w, &g, &path, 0.5);
    assert_eq!(parts.len(), 2);
    assert!(parts.iter().all(|p| p.bbox_um[0] >= 23.));
    assert!(segment::split_grain_path(&w, &g, &path[..2], 0.5).is_empty());
}

#[test]
fn missing_and_invalid_paths_are_noops() {
    let (w, g) = fixture();
    for path in [
        vec![],
        vec![[100., 100.]],
        vec![[100., 100.], [100., 100.]],
        vec![[0., 0.], [0., 200.]],
        vec![[f64::NAN, 0.], [100., 100.]],
    ] {
        assert!(segment::split_grain_path(&w, &g, &path, 0.5).is_empty());
    }
}

fn canonical(mut parts: Vec<Grain>) -> Vec<Grain> {
    for p in &mut parts {
        p.id = uuid::Uuid::nil();
    }
    parts.sort_by(|a, b| {
        b.area_um2
            .total_cmp(&a.area_um2)
            .then(a.centroid_um[0].total_cmp(&b.centroid_um[0]))
            .then(a.centroid_um[1].total_cmp(&b.centroid_um[1]))
    });
    parts
}

#[test]
fn straight_paths_match_the_previous_infinite_line_cutter() {
    let (w, g) = fixture();
    // Also exercise the rectangle used by grain_samples.rs, at its original scale.
    let mut small = g.clone();
    small.polygon_um = vec![[20., 20.], [60., 20.], [60., 60.], [20., 60.]];
    small.bbox_um = [20., 20., 60., 60.];
    small.centroid_um = [40., 40.];
    small.area_um2 = 1600.;
    for grain in [&g, &small] {
        let [cx, cy] = grain.centroid_um;
        for [dx, dy] in [[0., 1.], [1., 0.], [1., 1.], [1., 0.37], [-0.4, 1.]] {
            let line = [cx - dx, cy - dy, cx + dx, cy + dy];
            let expected = canonical(legacy_split(&w, grain, line, 0.8));
            assert_eq!(expected.len(), 2);
            assert_eq!(
                canonical(segment::split_grain_path(
                    &w,
                    grain,
                    &[[line[0], line[1]], [line[2], line[3]]],
                    0.8
                )),
                expected
            );
            assert_eq!(
                canonical(segment::split_grain(&w, grain, line, 0.8)),
                expected
            );
        }
    }
}

// Frozen pre-freehand cutter: an independent regression oracle for straight cuts.
/// Splits a grain along a line (um). Returns the pieces larger than 5 % of the grain.
fn legacy_split(w: &WorkImage, g: &Grain, line: [f64; 4], simplify_px: f64) -> Vec<Grain> {
    legacy_for_signal(w, g, line, simplify_px, w.signal)
}

fn legacy_for_signal(
    w: &WorkImage,
    g: &Grain,
    line: [f64; 4],
    simplify_px: f64,
    signal: Signal,
) -> Vec<Grain> {
    let (mask, bw, bh, origin) = shape::rasterize(g, w.um_per_px, 2);
    let mut m = mask;
    // Cut: clear pixels within 1 px of the line.
    let k = w.um_per_px;
    let (ax, ay, bx, by) = (
        line[0] / k - origin.0 as f64,
        line[1] / k - origin.1 as f64,
        line[2] / k - origin.0 as f64,
        line[3] / k - origin.1 as f64,
    );
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
    out
}
