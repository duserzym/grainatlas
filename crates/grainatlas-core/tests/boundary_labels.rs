#[path = "../examples/support/spot_labels.rs"]
mod fixtures;
use grainatlas_core::layout::labels::{Options, place};
#[test]
fn thin_whole_area_uses_ordered_boundary_rails() {
    let (page, _) = fixtures::whole();
    let (page, report) = place(page, &Options::default());
    eprintln!("whole: {:?}", report.counts);
    assert!(report.size_mm >= 2.4);
    assert_eq!(report.counts.still_overlapping, 0);
    assert_eq!(report.counts.ambiguous, 0);
    assert!(
        report.size_mm > 2.0,
        "free rails must improve on round 6 size"
    );
    assert_in_body(&page, &report);
    check_leaders(&page, &report);
    check_tiers(&page, &report);
    let (mut shuffled, _) = fixtures::whole();
    shuffled.anchors.reverse();
    assert_eq!(report, place(shuffled, &Options::default()).1);
    for side in [false, true] {
        let mut rail: Vec<_> = report
            .labels
            .iter()
            .filter(|l| {
                !l.leader.is_empty()
                    && (l.box_mm[3] < 128.5 || l.box_mm[1] > 157.5)
                    && (l.box_mm[1] > 156.0) == side
            })
            .collect();
        rail.sort_by(|a, b| a.anchor_mm[0].total_cmp(&b.anchor_mm[0]));
        assert!(rail.windows(2).all(|l| l[0].box_mm[0] < l[1].box_mm[0]));
    }
}

fn expanded(r: [f64; 4], gap: f64) -> [f64; 4] {
    [r[0] - gap, r[1] - gap, r[2] + gap, r[3] + gap]
}
fn boxes_overlap(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}
fn segments_close(a: [[f64; 2]; 2], b: [[f64; 2]; 2]) -> bool {
    let cross = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    (cross(a[0], a[1], b[0]) * cross(a[0], a[1], b[1]) < 0.0
        && cross(b[0], b[1], a[0]) * cross(b[0], b[1], a[1]) < 0.0)
        || a.iter().any(|&p| point_segment(p, b[0], b[1]) < 0.8 - 1e-8)
        || b.iter().any(|&p| point_segment(p, a[0], a[1]) < 0.8 - 1e-8)
}

// Independent paper geometry checks, including a search for a free position on
// every closer tier. This catches a body-edge rail even when it has no collisions.
fn check_tiers(
    page: &grainatlas_core::layout::labels::Page,
    report: &grainatlas_core::layout::labels::Report,
) {
    let mut offsets = Vec::new();
    let mut tiers = std::collections::BTreeMap::<(usize, usize), Vec<_>>::new();
    for (i, label) in report.labels.iter().enumerate() {
        let r = label.box_mm;
        if r[3] >= 130.0 && r[1] <= 156.0 {
            continue;
        }
        let side = usize::from(r[1] > 156.0);
        let sign = if side == 0 { -1.0 } else { 1.0 };
        let image_edge = [130.0, 156.0][side];
        let first = [118.35, 168.25][side];
        let h = r[3] - r[1];
        let w = r[2] - r[0];
        let near = r[if side == 0 { 3 } else { 1 }];
        let tier = ((near - first).abs() / (h + 1.5)).round() as usize;
        assert!((near - first - sign * tier as f64 * (h + 1.5)).abs() < 1e-8);
        // The first tier starts at the outermost stage/grid decoration plus
        // 1.5 mm, rather than at either boundary of the page body.
        let decoration_band = [10.15, 10.75][side];
        assert!((first - image_edge).abs() < decoration_band + 1.5 + h);
        assert!(((first - image_edge).abs() - decoration_band - 1.5).abs() < 1e-8);
        tiers.entry((side, tier)).or_default().push(label);
        offsets.push(((r[0] + r[2]) / 2.0 - label.anchor_mm[0]).abs());
        for inner in 0..tier {
            for other_side in 0..2 {
                let inner_sign = if other_side == 0 { -1.0 } else { 1.0 };
                let inner_near =
                    [118.35, 168.25][other_side] + inner_sign * inner as f64 * (h + 1.5);
                for k in -160..=160 {
                    let x = label.anchor_mm[0] + k as f64 * 0.25;
                    let y = inner_near - if other_side == 0 { h } else { 0.0 };
                    let trial = [x - w / 2.0, y, x + w / 2.0, y + h];
                    let exit_y = [129.8, 156.2][other_side];
                    let dx = (x - label.anchor_mm[0]).abs();
                    if dx > (inner_near - exit_y).abs() && !(inner == 0 && dx < 3.0) {
                        continue;
                    }
                    let path = [
                        [label.anchor_mm[0], label.anchor_mm[1] + inner_sign * 0.4],
                        [label.anchor_mm[0], exit_y],
                        [x, inner_near],
                    ];
                    if trial[0] < page.body[0] + 1.5
                        || trial[2] > page.body[2] - 1.5
                        || trial[1] < page.body[1] + 1.5
                        || trial[3] > page.body[3] - 1.5
                        || page.hard.iter().any(|&b| {
                            boxes_overlap(trial, expanded(b, 1.5))
                                || path
                                    .windows(2)
                                    .any(|s| hits_box(s[0], s[1], expanded(b, 0.16)))
                        })
                    {
                        continue;
                    }
                    let blocked = report.labels.iter().enumerate().any(|(j, other)| {
                        if i == j {
                            return false;
                        }
                        let b = other.box_mm;
                        let order = (b[1] - y).abs() < 1e-8
                            && (label.anchor_mm[0] - other.anchor_mm[0]) * (trial[0] - b[0]) < 0.0;
                        // A hypothetical move must satisfy the same 0.8 mm
                        // same-tier plate spacing asserted on the final rails.
                        let rail_gap = (b[1] - y).abs() < 1e-8
                            && boxes_overlap(expanded(trial, 0.8 - 1e-8), b);
                        let marker = [
                            other.anchor_mm[0] - 0.72,
                            other.anchor_mm[1] - 0.72,
                            other.anchor_mm[0] + 0.72,
                            other.anchor_mm[1] + 0.72,
                        ];
                        order
                            || rail_gap
                            || boxes_overlap(expanded(trial, 0.16), b)
                            || boxes_overlap(trial, marker)
                            || path.windows(2).any(|s| {
                                hits_box(s[0], s[1], expanded(b, 0.16))
                                    || hits_box(s[0], s[1], marker)
                            })
                            || other.leader.windows(2).any(|s| {
                                hits_box(s[0], s[1], expanded(trial, 0.16))
                                    || path
                                        .windows(2)
                                        .any(|p| segments_close([p[0], p[1]], [s[0], s[1]]))
                            })
                    });
                    assert!(
                        blocked,
                        "{} could use tier {} on side {} at x {}",
                        label.name, inner, other_side, x
                    );
                }
            }
        }
    }
    assert!(!offsets.is_empty());
    let mean = offsets.iter().sum::<f64>() / offsets.len() as f64;
    let max = offsets.iter().copied().fold(0.0, f64::max);
    eprintln!("rail displacement: mean {mean:.3} mm, max {max:.3} mm");
    assert!(mean < 4.0, "mean {mean}");
    assert!(max < 15.0, "max {max}");
    for rail in tiers.values_mut() {
        rail.sort_by(|a, b| a.anchor_mm[0].total_cmp(&b.anchor_mm[0]));
        assert!(
            rail.windows(2)
                .all(|p| p[0].box_mm[2] + 0.8 <= p[1].box_mm[0] + 1e-8)
        );
    }
}
#[test]
fn dense_sheet_remains_adjacent() {
    let (p, _) = fixtures::dense_sheet();
    let (p, r) = place(p, &Options::default());
    assert!(r.size_mm >= 2.4);
    assert_in_body(&p, &r);
    assert_eq!(r.counts.still_overlapping, 0);
    assert_eq!(r.counts.ambiguous, 0);
    assert!(r.counts.next_to_spot >= 65, "{:?}", r.counts);
}

fn point_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]))
        .clamp(0.0, 1.0);
    (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
}
fn hits_box(a: [f64; 2], b: [f64; 2], r: [f64; 4]) -> bool {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for k in 0..2 {
        let d = b[k] - a[k];
        if d.abs() < 1e-10 {
            if a[k] < r[k] || a[k] > r[k + 2] {
                return false;
            }
        } else {
            let t0 = (r[k] - a[k]) / d;
            let t1 = (r[k + 2] - a[k]) / d;
            lo = lo.max(t0.min(t1));
            hi = hi.min(t0.max(t1));
        }
    }
    lo <= hi
}
#[test]
fn whole_leaders_clear_each_other_and_all_grid_and_corner_text() {
    let (p, _) = fixtures::whole();
    let hard = p.hard.clone();
    let (p, r) = place(p, &Options::default());
    assert_in_body(&p, &r);
    for (i, a) in r.labels.iter().enumerate() {
        for h in &hard {
            assert!(
                a.box_mm[2] <= h[0]
                    || a.box_mm[0] >= h[2]
                    || a.box_mm[3] <= h[1]
                    || a.box_mm[1] >= h[3]
            );
            assert!(
                a.leader.windows(2).all(|s| !hits_box(s[0], s[1], *h)),
                "{} over text",
                a.name
            );
        }
        for b in &r.labels[i + 1..] {
            // Independent fine sampling: 0.05 mm resolution plus exact distance
            // to the other segment, with the specified 2 mm anchor exception.
            for s in a.leader.windows(2) {
                let length = (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]);
                let steps = (length / 0.05).ceil() as usize;
                for j in 0..=steps {
                    let t = j as f64 / steps as f64;
                    let p = [
                        s[0][0] + t * (s[1][0] - s[0][0]),
                        s[0][1] + t * (s[1][1] - s[0][1]),
                    ];
                    if [a.anchor_mm, b.anchor_mm]
                        .iter()
                        .any(|c| (p[0] - c[0]).hypot(p[1] - c[1]) < 2.0)
                    {
                        continue;
                    }
                    for z in b.leader.windows(2) {
                        assert!(
                            point_segment(p, z[0], z[1]) >= 0.8 - 1e-6,
                            "{} near {} at {:?}",
                            a.name,
                            b.name,
                            p
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn tall_images_use_side_rails_and_shuffle_is_identical() {
    let make = || {
        let (mut p, _) = fixtures::whole();
        p.body = [p.body[1], p.body[0], p.body[3], p.body[2]];
        for polygon in p.images.iter_mut().chain(p.grains.iter_mut()) {
            for point in polygon {
                point.swap(0, 1);
            }
        }
        for h in &mut p.hard {
            *h = [h[1], h[0], h[3], h[2]];
        }
        for a in &mut p.anchors {
            if let grainatlas_core::layout::labels::Marker::Circle(c, _) = &mut a.marker {
                c.swap(0, 1);
            }
        }
        p
    };
    let (p, r) = place(make(), &Options::default());
    assert_eq!(r.counts.still_overlapping, 0);
    check_leaders(&p, &r);
    let mut shuffled = make();
    shuffled.anchors.reverse();
    assert_eq!(r, place(shuffled, &Options::default()).1);
}

// The fixture's grains and image are rectangles. Use an independent segment /
// box test, rather than the placer's collision or route classification.
fn check_leaders(
    page: &grainatlas_core::layout::labels::Page,
    report: &grainatlas_core::layout::labels::Report,
) {
    assert_in_body(page, report);
    let image = &page.images[0];
    let bounds = [image[0][0], image[0][1], image[2][0], image[2][1]];
    for label in report.labels.iter().filter(|l| !l.leader.is_empty()) {
        let r = label.box_mm;
        let on_rail = r[2] < bounds[0] || r[0] > bounds[2] || r[3] < bounds[1] || r[1] > bounds[3];
        assert!(
            !label.last_resort,
            "{} used A* on the whole-area fixture",
            label.name
        );
        if on_rail {
            assert_eq!(label.leader.len(), 3, "{}: {:?}", label.name, label.leader);
            if label.leader.len() == 3 {
                let p = label.leader[1];
                let axis = usize::from(bounds[3] - bounds[1] > bounds[2] - bounds[0]);
                let cross = 1 - axis;
                let side = usize::from(r[cross] > bounds[cross + 2]);
                let edge = bounds[cross + 2 * side];
                let sign = if side == 0 { -1.0 } else { 1.0 };
                assert!(
                    (p[cross] - edge - sign * 0.2).abs() < 1e-8,
                    "{} must bend just outside the image",
                    label.name
                );
                assert!((p[axis] - label.anchor_mm[axis]).abs() < 1e-8);
                assert!((label.leader[0][axis] - p[axis]).abs() < 1e-8);
                let end = label.leader[2];
                let dx = (end[axis] - p[axis]).abs();
                let dy = (end[cross] - p[cross]).abs();
                let first_near = [118.35, 168.25][side];
                let first = (end[cross] - first_near).abs() < 1e-8;
                assert!(
                    dy + 1e-8 >= dx || (first && dx < 3.0),
                    "{} has a flat outside segment",
                    label.name
                );
            }
        } else {
            assert_eq!(label.leader.len(), 2, "{}: {:?}", label.name, label.leader);
            let [a, b] = [label.leader[0], label.leader[1]];
            assert!(
                (a[0] - b[0]).hypot(a[1] - b[1]) <= 2.0 * (r[3] - r[1]) + 1e-8,
                "{} too long",
                label.name
            );
            for grain in &page.grains {
                let g = [grain[0][0], grain[0][1], grain[2][0], grain[2][1]];
                if hits_box(label.anchor_mm, label.anchor_mm, g) {
                    continue;
                }
                assert!(!hits_box(a, b, g), "{} crossed another grain", label.name);
            }
        }
    }
}

#[test]
#[ignore = "release timing, run explicitly with --release --ignored --nocapture"]
fn boundary_and_dense_release_timing() {
    for (name, fixture) in [
        ("whole area", fixtures::whole as fn() -> _),
        ("dense sheet", fixtures::dense_sheet as fn() -> _),
    ] {
        let (page, _) = fixture();
        let start = std::time::Instant::now();
        let (page, report) = place(page, &Options::default());
        let elapsed = start.elapsed();
        eprintln!(
            "{name}: {:.3} ms, {:?}",
            elapsed.as_secs_f64() * 1000.0,
            report.counts
        );
        assert_eq!(report.counts.still_overlapping, 0);
        assert_in_body(&page, &report);
        if name == "whole area" {
            check_leaders(&page, &report);
        }
        assert!(elapsed.as_secs_f64() < 0.500, "{elapsed:?}");
    }
}

fn assert_in_body(
    page: &grainatlas_core::layout::labels::Page,
    report: &grainatlas_core::layout::labels::Report,
) {
    for l in &report.labels {
        let r = l.box_mm;
        let b = page.body;
        assert!(
            r[0] >= b[0] && r[1] >= b[1] && r[2] <= b[2] && r[3] <= b[3],
            "{} outside body: {:?}",
            l.name,
            r
        );
        assert!(
            l.leader
                .iter()
                .all(|p| p[0] >= b[0] && p[1] >= b[1] && p[0] <= b[2] && p[1] <= b[3])
        );
    }
}

#[test]
fn unresolved_context_is_omitted_but_its_marker_and_owner_remain() {
    let (p, mut svg) = fixtures::context_edge();
    let report =
        grainatlas_core::layout::labels::render(&mut svg, p.clone(), &Options::default(), 1);
    assert_in_body(&p, &report);
    assert_eq!(report.counts.context_omitted, 1);
    assert_eq!(report.counts.anchors, 1);
    assert_eq!(report.counts.occurrences, 2);
    assert_eq!(report.counts.still_overlapping, 0);
    assert_eq!(report.labels.len(), 1);
    assert_eq!(report.labels[0].spot_id, "owned");
    let doc = roxmltree::Document::parse(&svg).unwrap();
    assert_eq!(
        doc.descendants()
            .filter(|n| n.attribute("data-spot-marker").is_some())
            .count(),
        2
    );
    assert_eq!(
        doc.descendants()
            .filter(|n| n.attribute("data-spot-label").is_some())
            .count(),
        1
    );
    let mut shuffled = p.clone();
    shuffled.anchors.reverse();
    assert_eq!(
        place(p, &Options::default()).1,
        place(shuffled, &Options::default()).1
    );
}

#[test]
fn association_row_keeps_largest_auto_size_without_ambiguity() {
    let (page, _) = fixtures::association_row();
    let (page, report) = place(page, &Options::default());
    assert_eq!(report.size_mm, 3.2);
    assert_eq!(report.labels.len(), 8);
    assert_eq!(report.counts.ambiguous, 0);
    assert_eq!(report.counts.still_overlapping, 0);
    assert_in_body(&page, &report);
    assert!(report.labels.iter().all(|l| l.connector_tick));
}
