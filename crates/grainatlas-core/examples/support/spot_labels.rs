//! Synthetic paper geometry shared by the review helper and integration tests.
use grainatlas_core::layout::labels::{Anchor, Marker, Page};
use std::fmt::Write;

fn rectangle(r: [f64; 4]) -> Vec<[f64; 2]> {
    vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
}

pub fn whole() -> (Page, String) {
    let mut p = Page::new([8.0, 32.0, 424.0, 264.0]);
    p.images.push(rectangle([76.0, 130.0, 356.0, 156.0]));
    let mut svg = String::from(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="432mm" height="279mm" viewBox="0 0 432 279"><rect width="432" height="279" fill="white"/><g font-family="DejaVu Sans"><text x="8" y="12" font-size="4">Synthetic whole area</text><text x="8" y="18" font-size="2.4">1 mm paper = 25 µm sample</text><rect x="76" y="130" width="280" height="26" fill="#222"/>"##,
    );
    for j in 1..10 {
        let x = 76.0 + j as f64 * 28.0;
        let _ = write!(
            svg,
            r##"<g class="whole-grid"><path d="M{x} 130 V156" stroke="#aaa" stroke-width="0.1" opacity="0.6"/></g>"##
        );
    }
    for j in 0..10 {
        let x = 76.0 + (j as f64 + 0.5) * 28.0;
        let letter = char::from(b'A' + j);
        for y in [126.0, 161.0] {
            let _ = write!(
                svg,
                r#"<text x="{x}" y="{y}" text-anchor="middle" font-size="2.4">{letter}</text>"#
            );
        }
    }
    for j in 0..=2 {
        let y = 130.0 + j as f64 * 13.0;
        for x in [73.0, 359.0] {
            let _ = write!(svg, r#"<text x="{x}" y="{y}" font-size="2">{j}</text>"#);
        }
    }
    for (x, anchor) in [(76, "start"), (356, "end")] {
        for y in [122, 166] {
            let _ = write!(
                svg,
                r#"<text x="{x}" y="{y}" text-anchor="{anchor}" font-size="2">stage X 1.000, Y 2.000 mm</text>"#
            );
        }
    }
    let mut x = 109.0;
    for i in 0..45 {
        let y = 143.0 + [-3.0, 2.0, -1.0, 3.0, 0.0][i % 5];
        let r = [x - 1.1, y - 4.0, x + 1.1, y + 4.0];
        p.grains.push(rectangle(r));
        let _ = write!(
            svg,
            r##"<ellipse cx="{x}" cy="{y}" rx="1.1" ry="4" fill="#999"/>"##
        );
        p.anchors.push(Anchor {
            spot_id: format!("synthetic-{i:03}"),
            occurrence: "whole/synthetic".into(),
            name: format!("ABC12_S_{}", 197 + i),
            marker: Marker::Circle([x, y], 0.4),
            image: 0,
            repeated: false,
        });
        x += [2.0, 6.0, 4.0][i % 3];
    }
    svg.push_str(
        r#"<text x="8" y="272" font-size="2">Synthetic review · page 1/1</text></g></svg>"#,
    );
    grainatlas_core::layout::labels::decorations(&svg, &mut p);
    (p, svg)
}

pub fn dense_sheet() -> (Page, String) {
    let mut p = Page::new([8.0, 32.0, 424.0, 264.0]);
    let mut svg = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="432mm" height="279mm" viewBox="0 0 432 279"><rect width="432" height="279" fill="white"/><text x="8" y="12" font-size="4">Synthetic grain sheet</text>"#,
    );
    for row in 0..4 {
        let y = 50.0 + row as f64 * 48.0;
        p.images.push(rectangle([12.0, y, 416.0, y + 34.0]));
        let _ = write!(
            svg,
            r##"<rect x="12" y="{y}" width="404" height="34" fill="#222"/>"##
        );
        for j in 0..18 {
            let c = [24.0 + j as f64 * 21.5, y + 17.0 + (j % 3) as f64 * 2.0];
            p.grains
                .push(rectangle([c[0] - 3.0, c[1] - 8.0, c[0] + 3.0, c[1] + 8.0]));
            let _ = write!(
                svg,
                r##"<ellipse cx="{}" cy="{}" rx="3" ry="8" fill="#999"/>"##,
                c[0], c[1]
            );
            p.anchors.push(Anchor {
                spot_id: format!("synthetic-{row}-{j:03}"),
                occurrence: format!("band/{row}"),
                name: format!("ABC12_S_{}", row * 18 + j + 1),
                marker: Marker::Circle(c, 1.2),
                image: row,
                repeated: false,
            });
        }
    }
    svg.push_str("</svg>");
    grainatlas_core::layout::labels::decorations(&svg, &mut p);
    (p, svg)
}

/// A repeated-strip spot at the left paper edge has no clear text slot.
/// Its owning band labels the same spot elsewhere; only this marker is needed.
pub fn context_edge() -> (Page, String) {
    let mut p = Page::new([8.0, 32.0, 120.0, 90.0]);
    p.images.push(rectangle([8.0, 36.0, 116.0, 80.0]));
    p.hard.push([8.0, 32.0, 60.0, 90.0]);
    for (id, x, repeated) in [("context", 8.5, true), ("owned", 80.0, false)] {
        p.anchors.push(Anchor {
            spot_id: id.into(),
            occurrence: "band/0".into(),
            name: format!("ABC12_S_{id}"),
            marker: Marker::Circle([x, 60.0], 0.4),
            image: 0,
            repeated,
        });
    }
    let svg = String::from(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="128mm" height="100mm" viewBox="0 0 128 100"><rect width="128" height="100" fill="white"/><text x="8" y="12" font-size="4">Synthetic repeated edge</text><rect x="8" y="36" width="108" height="44" fill="#222"/><path d="M8 36 V80 M10 36 V80 M12 36 V80" stroke="#aaa" stroke-width="0.2"/><rect x="8" y="32" width="52" height="58" fill="#999" fill-opacity="0.25"/><text x="14" y="42" fill="white" font-size="2.4">Reserved decoration</text></svg>"##,
    );
    (p, svg)
}

/// Eight one-spot grains at 8 µm/mm, with the stagger and spacing that made
/// corner plates visually attach to a neighbouring spot in round 7.
pub fn association_row() -> (Page, String) {
    let mut p = Page::new([8.0, 20.0, 218.0, 100.0]);
    p.images.push(rectangle([12.0, 30.0, 214.0, 90.0]));
    let mut svg = String::from(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="226mm" height="108mm" viewBox="0 0 226 108"><rect width="226" height="108" fill="white"/><text x="8" y="12" font-size="4">Synthetic association row · 8 µm/mm</text><rect x="12" y="30" width="202" height="60" fill="#222"/>"##,
    );
    for i in 0..8 {
        let c = [30.0 + i as f64 * 21.5, 60.0 + (i % 3) as f64 * 2.0];
        p.grains
            .push(rectangle([c[0] - 3.0, c[1] - 8.0, c[0] + 3.0, c[1] + 8.0]));
        let _ = write!(
            svg,
            r##"<ellipse cx="{}" cy="{}" rx="3" ry="8" fill="#999"/>"##,
            c[0], c[1]
        );
        p.anchors.push(Anchor {
            spot_id: format!("synthetic-association-{i}"),
            occurrence: "band/0".into(),
            name: format!("AB12_S_{}", 211 + i),
            marker: Marker::Circle(c, 25.0 / 8.0 / 2.0),
            image: 0,
            repeated: false,
        });
    }
    svg.push_str("</svg>");
    (p, svg)
}
