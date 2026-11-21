//! Deterministic paper-space annotations. Nothing here changes mount geometry.
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::sync::OnceLock;

use super::esc;
use super::sheets::arrange;
use crate::spots::{Spot, SpotKind};
use serde::{Deserialize, Serialize};

pub type Point = [f64; 2];
pub type Rect = [f64; 4];
const GAP: f64 = 0.32;
const TICK_GAP: f64 = 0.8;
const AMBIGUITY_COST: f64 = 10_000.0;
const CELL: f64 = 8.0;
const LEADER_CLEARANCE: f64 = 0.8;
const RAIL_GAP: f64 = 1.5;
const RAIL_EXIT: f64 = 0.2;
const IN_IMAGE_HEIGHTS: f64 = 2.0;
const ASTAR_DIR: usize = 12;
const GROUP_ROUTE_LIMIT: usize = 512;
const PAGE_ROUTE_LIMIT: usize = 1_024;
const PAGE_ASTAR_VISITS: usize = 32_000;
const PAGE_REPAIR_CALLS: usize = 2;
const PAGE_REPAIR_PAIRS: usize = 6_000_000;
type RouteKey = (usize, [u64; 4], bool);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Options {
    /// SVG em size in paper mm; zero is Auto, serialized as "auto".
    #[serde(with = "label_size")]
    pub size_mm: f64,
    pub pins: BTreeMap<String, Point>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            size_mm: 0.0,
            pins: BTreeMap::new(),
        }
    }
}
impl Options {
    pub fn size(&self) -> f64 {
        if self.size_mm.is_finite() && self.size_mm > 0.0 {
            self.size_mm.clamp(1.6, 12.0)
        } else {
            2.4
        }
    }
}

// Keep the numeric Rust API for fixed-size callers and old saved plans.
mod label_size {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(size: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        if *size == 0.0 {
            serializer.serialize_str("auto")
        } else {
            serializer.serialize_f64(*size)
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Size {
            Fixed(f64),
            Auto(String),
        }
        match Size::deserialize(deserializer)? {
            Size::Fixed(size) => Ok(size),
            Size::Auto(s) if s == "auto" => Ok(0.0),
            _ => Err(serde::de::Error::custom(
                "label size must be auto or paper mm",
            )),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Marker {
    Circle(Point, f64),
    Line(Point, Point, f64),
    Polygon(Vec<Point>),
}
#[derive(Clone, Debug)]
pub struct Anchor {
    pub spot_id: String,
    /// Stable image identity, independent of page and position.
    pub occurrence: String,
    pub name: String,
    pub marker: Marker,
    pub image: usize,
    pub repeated: bool,
}
impl Anchor {
    pub fn centre(&self) -> Point {
        self.marker.centre()
    }
    pub fn key(&self) -> String {
        format!("{}/{}", self.spot_id, self.occurrence)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlacedLabel {
    pub spot_id: String,
    pub occurrence: String,
    pub name: String,
    pub anchor_mm: Point,
    pub box_mm: Rect,
    pub leader: Vec<Point>,
    pub leader_path: String,
    /// Straight marker-to-box connector, rendered as a 0.1 mm hairline.
    #[serde(default)]
    pub connector_tick: bool,
    /// The bounded A* exception; ordinary in-image leaders stay straight.
    #[serde(default)]
    pub last_resort: bool,
    pub pinned: bool,
    pub repeated: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Counts {
    /// Unique, non-repeated anchors; occurrences includes repeated context.
    pub anchors: usize,
    pub occurrences: usize,
    pub next_to_spot: usize,
    /// Straight marker-to-box connectors shorter than 3 mm.
    #[serde(default)]
    pub with_ticks: usize,
    /// Longer connectors and all rail or routed leaders.
    pub with_leaders: usize,
    pub pinned: usize,
    /// Repeated-strip labels with no clear slot; their markers remain.
    #[serde(default)]
    pub context_omitted: usize,
    /// Pinned labels involved in any hard conflict, counted once each.
    #[serde(default)]
    pub pinned_overlapping: usize,
    /// Automatic labels involved in any hard conflict, counted once each.
    pub still_overlapping: usize,
    /// Unconnected labels with another marker within 1.2 times their own marker distance.
    #[serde(default)]
    pub ambiguous: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Report {
    pub sheet: usize,
    /// Chosen SVG em size for this page, including Auto.
    #[serde(default)]
    pub size_mm: f64,
    pub labels: Vec<PlacedLabel>,
    pub counts: Counts,
}

#[derive(Clone)]
pub struct Page {
    pub body: Rect,
    pub images: Vec<Vec<Point>>,
    pub hard: Vec<Rect>,
    // Boundary-rail gutters for text and other hard decorations.
    // Missing entries use the default gutter too.
    rail_gaps: Vec<f64>,
    pub grains: Vec<Vec<Point>>,
    pub anchors: Vec<Anchor>,
}

fn font() -> &'static ttf_parser::Face<'static> {
    static FACE: OnceLock<ttf_parser::Face<'static>> = OnceLock::new();
    FACE.get_or_init(|| {
        ttf_parser::Face::parse(
            include_bytes!("../../assets/fonts/IBMPlexSansCondensed-Medium.ttf"),
            0,
        )
        .expect("embedded print font")
    })
}
fn fallback_font() -> &'static ttf_parser::Face<'static> {
    static FACE: OnceLock<ttf_parser::Face<'static>> = OnceLock::new();
    FACE.get_or_init(|| {
        ttf_parser::Face::parse(include_bytes!("../../assets/fonts/DejaVuSans.ttf"), 0)
            .expect("embedded fallback print font")
    })
}
/// Advance widths and ascender/descender from the fonts embedded by pdf.rs,
/// with plate padding. SVG disables kerning and discretionary ligatures.
pub fn text_size(text: &str, size: f64) -> Point {
    measured_text(font(), Some(fallback_font()), text, size)
}
fn measured_text(
    f: &ttf_parser::Face<'_>,
    fallback: Option<&ttf_parser::Face<'_>>,
    text: &str,
    size: f64,
) -> Point {
    let scale = size / f.units_per_em() as f64;
    let width: f64 = text
        .chars()
        .map(|c| {
            if let Some(advance) = f.glyph_index(c).and_then(|g| f.glyph_hor_advance(g)) {
                return advance as f64;
            }
            let face = fallback.unwrap_or(f);
            face.glyph_index(c)
                .and_then(|g| face.glyph_hor_advance(g))
                .unwrap_or(face.units_per_em()) as f64
                * f.units_per_em() as f64
                / face.units_per_em() as f64
        })
        .sum();
    [
        width * scale + 0.8,
        (f.ascender() - f.descender()) as f64 * scale + 0.5,
    ]
}
fn bounds(points: &[Point]) -> Rect {
    points.iter().fold(
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
    )
}
fn expand(r: Rect, d: f64) -> Rect {
    [r[0] - d, r[1] - d, r[2] + d, r[3] + d]
}
fn overlap(a: Rect, b: Rect) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}
fn contains(r: Rect, p: Point) -> bool {
    p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]
}
fn inside(r: Rect, b: Rect) -> bool {
    contains(r, [b[0], b[1]]) && contains(r, [b[2], b[3]])
}
fn corners(r: Rect) -> Vec<Point> {
    vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
}
fn centre_of(r: Rect) -> Point {
    [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0]
}
fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn segments(a: Point, b: Point, c: Point, d: Point) -> bool {
    // Include touches and collinear overlaps; casing needs clearance too.
    let e = 1e-8;
    cross(a, b, c) * cross(a, b, d) <= e
        && cross(c, d, a) * cross(c, d, b) <= e
        && overlap(expand(bounds(&[a, b]), e), expand(bounds(&[c, d]), e))
}
fn nearest_on_segment(p: Point, a: Point, b: Point) -> Point {
    let den = (b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2);
    let t = if den > 0.0 {
        ((p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1])) / den
    } else {
        0.0
    }
    .clamp(0.0, 1.0);
    [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
}
fn point_segment(p: Point, a: Point, b: Point) -> f64 {
    distance(p, nearest_on_segment(p, a, b))
}
fn nearest_in_box(p: Point, r: Rect) -> Point {
    [p[0].clamp(r[0], r[2]), p[1].clamp(r[1], r[3])]
}
fn segment_box_nearest(a: Point, b: Point, r: Rect) -> [Point; 2] {
    if segment_rect(a, b, r) {
        let p = nearest_in_box(a, r);
        return [p, p];
    }
    [[a, nearest_in_box(a, r)], [b, nearest_in_box(b, r)]]
        .into_iter()
        .chain(
            corners(r)
                .into_iter()
                .map(|p| [nearest_on_segment(p, a, b), p]),
        )
        .min_by(|a, b| distance(a[0], a[1]).total_cmp(&distance(b[0], b[1])))
        .unwrap()
}
fn in_polygon(p: Point, poly: &[Point]) -> bool {
    let mut yes = false;
    for (a, b) in poly
        .iter()
        .zip(poly.iter().cycle().skip(1))
        .take(poly.len())
    {
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            yes = !yes;
        }
    }
    yes
}
fn segment_rect(a: Point, b: Point, r: Rect) -> bool {
    contains(r, a)
        || contains(r, b)
        || corners(r)
            .iter()
            .zip(corners(r).iter().cycle().skip(1))
            .take(4)
            .any(|(c, d)| segments(a, b, *c, *d))
}
fn polygon_rect(poly: &[Point], r: Rect) -> bool {
    overlap(bounds(poly), r)
        && (poly.iter().any(|p| contains(r, *p))
            || corners(r).iter().any(|p| in_polygon(*p, poly))
            || poly
                .iter()
                .zip(poly.iter().cycle().skip(1))
                .take(poly.len())
                .any(|(a, b)| segment_rect(*a, *b, r)))
}
impl Marker {
    fn bounds(&self) -> Rect {
        match self {
            Self::Circle(p, r) => [p[0] - r, p[1] - r, p[0] + r, p[1] + r],
            Self::Line(a, b, r) => expand(bounds(&[*a, *b]), *r),
            Self::Polygon(p) => bounds(p),
        }
    }
    /// Exact nearest points on the physical marker and label plate (no casing).
    fn box_connection(&self, rect: Rect) -> [Point; 2] {
        match self {
            Self::Circle(p, radius) => {
                let end = nearest_in_box(*p, rect);
                let d = distance(*p, end);
                if d <= *radius {
                    return [end, end];
                }
                [
                    [
                        p[0] + (end[0] - p[0]) * radius / d,
                        p[1] + (end[1] - p[1]) * radius / d,
                    ],
                    end,
                ]
            }
            Self::Line(a, b, radius) => {
                let [start, end] = segment_box_nearest(*a, *b, rect);
                let d = distance(start, end);
                if d <= *radius {
                    return [end, end];
                }
                [
                    [
                        start[0] + (end[0] - start[0]) * radius / d,
                        start[1] + (end[1] - start[1]) * radius / d,
                    ],
                    end,
                ]
            }
            Self::Polygon(poly) => {
                if polygon_rect(poly, rect) {
                    let p = nearest_in_box(self.centre(), rect);
                    return [p, p];
                }
                poly.iter()
                    .zip(poly.iter().cycle().skip(1))
                    .take(poly.len())
                    .map(|(&a, &b)| segment_box_nearest(a, b, rect))
                    .min_by(|a, b| distance(a[0], a[1]).total_cmp(&distance(b[0], b[1])))
                    .unwrap()
            }
        }
    }
    fn box_distance(&self, rect: Rect) -> f64 {
        let [start, end] = self.box_connection(rect);
        distance(start, end)
    }
    fn hits_box(&self, r: Rect) -> bool {
        match self {
            Self::Circle(p, d) => {
                distance(*p, [p[0].clamp(r[0], r[2]), p[1].clamp(r[1], r[3])]) < *d + GAP
            }
            Self::Line(a, b, d) => segment_rect(*a, *b, expand(r, *d + GAP)),
            Self::Polygon(p) => polygon_rect(p, expand(r, GAP)),
        }
    }
    fn hits_path(&self, path: &[Point]) -> bool {
        path.windows(2).any(|s| match self {
            Self::Circle(p, r) => point_segment(*p, s[0], s[1]) < r + 0.12,
            Self::Line(a, b, r) => {
                segments(s[0], s[1], *a, *b)
                    || [
                        point_segment(s[0], *a, *b),
                        point_segment(s[1], *a, *b),
                        point_segment(*a, s[0], s[1]),
                        point_segment(*b, s[0], s[1]),
                    ]
                    .iter()
                    .any(|d| *d < r + 0.12)
            }
            Self::Polygon(p) => {
                in_polygon(s[0], p)
                    || p.iter()
                        .zip(p.iter().cycle().skip(1))
                        .take(p.len())
                        .any(|(a, b)| segments(s[0], s[1], *a, *b))
            }
        })
    }
    fn centre(&self) -> Point {
        match self {
            Self::Circle(p, _) => *p,
            Self::Line(a, b, _) => [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0],
            Self::Polygon(p) => {
                let sum = p.iter().fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
                [sum[0] / p.len() as f64, sum[1] / p.len() as f64]
            }
        }
    }
    fn edge(&self, towards: Point) -> Point {
        let c = self.centre();
        let d = distance(c, towards).max(1e-9);
        let ray = |r: f64| {
            [
                c[0] + (towards[0] - c[0]) * r / d,
                c[1] + (towards[1] - c[1]) * r / d,
            ]
        };
        if let Self::Circle(_, r) = self {
            return ray(*r);
        }
        let b = self.bounds();
        let mut lo = 0.0;
        let mut hi = (b[2] - b[0]).hypot(b[3] - b[1]) + 1.0;
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            let p = ray(mid);
            let inside = match self {
                Self::Line(a, b, r) => point_segment(p, *a, *b) <= *r,
                Self::Polygon(poly) => in_polygon(p, poly),
                _ => false,
            };
            if inside {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        ray(lo)
    }
}
fn paths_cross(a: &[Point], b: &[Point]) -> bool {
    a.windows(2)
        .any(|x| b.windows(2).any(|y| segments(x[0], x[1], y[0], y[1])))
}
/// Clearance excludes only the first 2 mm around either anchor. Clip segments
/// at those circles before measuring, rather than sampling long segments.
fn leaders_close(a: &[Point], b: &[Point], anchors: [Point; 2]) -> bool {
    fn trim(a: Point, b: Point, anchors: [Point; 2]) -> ([[Point; 2]; 5], usize) {
        let d = [b[0] - a[0], b[1] - a[1]];
        let aa = d[0] * d[0] + d[1] * d[1];
        let mut out = [[[0.0; 2]; 2]; 5];
        if aa < 1e-12 {
            return (out, 0);
        }
        let mut ts = [0.0; 6];
        ts[1] = 1.0;
        let mut n = 2;
        for centre in anchors {
            let f = [a[0] - centre[0], a[1] - centre[1]];
            let bb = 2.0 * (f[0] * d[0] + f[1] * d[1]);
            let cc = f[0] * f[0] + f[1] * f[1] - 4.0;
            let disc = bb * bb - 4.0 * aa * cc;
            if disc > 0.0 {
                for t in [
                    (-bb - disc.sqrt()) / (2.0 * aa),
                    (-bb + disc.sqrt()) / (2.0 * aa),
                ] {
                    if t > 0.0 && t < 1.0 {
                        ts[n] = t;
                        n += 1;
                    }
                }
            }
        }
        ts[..n].sort_by(f64::total_cmp);
        let at = |t: f64| [a[0] + t * d[0], a[1] + t * d[1]];
        let mut count = 0;
        for t in ts[..n].windows(2) {
            let middle = at((t[0] + t[1]) / 2.0);
            if anchors.iter().all(|&c| distance(middle, c) >= 2.0 - 1e-9) {
                out[count] = [at(t[0]), at(t[1])];
                count += 1;
            }
        }
        (out, count)
    }
    if a.len() < 2 || b.len() < 2 || !overlap(expand(bounds(a), LEADER_CLEARANCE), bounds(b)) {
        return false;
    }
    for x in a.windows(2) {
        for y in b.windows(2) {
            if !overlap(expand(bounds(x), LEADER_CLEARANCE), bounds(y)) {
                continue;
            }
            let (xx, nx) = trim(x[0], x[1], anchors);
            let (yy, ny) = trim(y[0], y[1], anchors);
            for x in &xx[..nx] {
                for y in &yy[..ny] {
                    if segments(x[0], x[1], y[0], y[1])
                        || [
                            point_segment(x[0], y[0], y[1]),
                            point_segment(x[1], y[0], y[1]),
                            point_segment(y[0], x[0], x[1]),
                            point_segment(y[1], x[0], x[1]),
                        ]
                        .into_iter()
                        .fold(f64::INFINITY, f64::min)
                            < LEADER_CLEARANCE - 1e-8
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn path_box(p: &[Point], r: Rect) -> bool {
    p.windows(2)
        .any(|s| segment_rect(s[0], s[1], expand(r, 0.16)))
}

/// Uniform grid: remove/insert only touched cells on a discrete move.
#[derive(Default)]
struct Grid {
    cells: HashMap<(i32, i32), Vec<usize>>,
}
impl Grid {
    fn keys(r: Rect) -> impl Iterator<Item = (i32, i32)> {
        let x0 = (r[0] / CELL).floor() as i32;
        let x1 = (r[2] / CELL).floor() as i32;
        let y0 = (r[1] / CELL).floor() as i32;
        let y1 = (r[3] / CELL).floor() as i32;
        (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
    }
    fn insert(&mut self, id: usize, r: Rect) {
        for k in Self::keys(r) {
            self.cells.entry(k).or_default().push(id);
        }
    }
    fn remove(&mut self, id: usize, r: Rect) {
        for k in Self::keys(r) {
            if let Some(v) = self.cells.get_mut(&k) {
                v.retain(|x| *x != id);
            }
        }
    }
    fn query(&self, r: Rect) -> Vec<usize> {
        let mut ids = Vec::new();
        for k in Self::keys(r) {
            if let Some(v) = self.cells.get(&k) {
                ids.extend(v);
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}
fn cost_key(cost: f64) -> u64 {
    let bits = cost.to_bits();
    if bits >> 63 == 1 {
        !bits
    } else {
        bits ^ (1 << 63)
    }
}
#[derive(Clone)]
struct Candidate {
    rect: Rect,
    path: Vec<Point>,
    dir: usize,
    cost: f64,
}
impl Candidate {
    fn bounds(&self) -> Rect {
        if self.path.is_empty() {
            self.rect
        } else {
            let b = bounds(&self.path);
            [
                b[0].min(self.rect[0]),
                b[1].min(self.rect[1]),
                b[2].max(self.rect[2]),
                b[3].max(self.rect[3]),
            ]
        }
    }
}
struct Search<'a> {
    page: &'a Page,
    markers: Grid,
    fixed: Grid,
    grains: Grid,
    images: Grid,
    placed: Grid,
    current: Vec<Option<Candidate>>,
    pinned: Vec<bool>,
    generation: usize,
    routes_left: Cell<usize>,
    group_routes_left: Cell<usize>,
    astar_left: Cell<usize>,
    repair_calls_left: Cell<usize>,
    repair_pairs_left: Cell<usize>,
    route_cache: RefCell<HashMap<RouteKey, (usize, Candidate)>>,
}
impl Search<'_> {
    fn path_static_clear(&self, i: usize, path: &[Point]) -> bool {
        let region = bounds(path);
        path.iter().all(|&p| contains(self.page.body, p))
            && self
                .markers
                .query(expand(region, GAP))
                .iter()
                .all(|&j| j == i || !self.page.anchors[j].marker.hits_path(path))
            && self
                .fixed
                .query(region)
                .iter()
                .all(|&j| !path_box(path, self.page.hard[j]))
            && self.images.query(region).iter().all(|&j| {
                j == self.page.anchors[i].image || {
                    let poly = &self.page.images[j];
                    !path.iter().any(|&p| in_polygon(p, poly))
                        && !path.windows(2).any(|s| {
                            poly.iter()
                                .zip(poly.iter().cycle().skip(1))
                                .take(poly.len())
                                .any(|(a, b)| segments(s[0], s[1], *a, *b))
                        })
                }
            })
    }
    fn covers_middle(&self, i: usize, c: &Candidate) -> bool {
        self.grains.query(c.rect).into_iter().any(|j| {
            let poly = &self.page.grains[j];
            !in_polygon(self.page.anchors[i].centre(), poly)
                && contains(c.rect, centre_of(bounds(poly)))
        })
    }

    /// Reject impossible plates before testing all of their leader geometry.
    fn plate_possible(&self, rect: Rect) -> bool {
        inside(self.page.body, rect)
            && self
                .markers
                .query(expand(rect, GAP))
                .into_iter()
                .all(|j| !self.page.anchors[j].marker.hits_box(rect))
    }
    fn static_conflicts(&self, i: usize, c: &Candidate) -> usize {
        let a = &self.page.anchors[i];
        let mut n = usize::from(!inside(self.page.body, c.rect));
        n += self
            .fixed
            .query(c.bounds())
            .iter()
            .filter(|&&j| {
                overlap(c.rect, self.page.hard[j]) || path_box(&c.path, self.page.hard[j])
            })
            .count();
        n += self
            .markers
            .query(expand(c.bounds(), GAP))
            .iter()
            .filter(|&&j| {
                let m = &self.page.anchors[j].marker;
                m.hits_box(c.rect) || (j != i && m.hits_path(&c.path))
            })
            .count();
        // A rail must be reachable without cutting across another band.
        n += self
            .images
            .query(c.bounds())
            .iter()
            .filter(|&&j| {
                if j == a.image {
                    return false;
                }
                let polygon = &self.page.images[j];
                polygon_rect(polygon, c.rect)
                    || c.path.iter().any(|p| in_polygon(*p, polygon))
                    || c.path.windows(2).any(|s| {
                        polygon
                            .iter()
                            .zip(polygon.iter().cycle().skip(1))
                            .take(polygon.len())
                            .any(|(a, b)| segments(s[0], s[1], *a, *b))
                    })
            })
            .count();
        if !self.pinned[i] && (8..ASTAR_DIR).contains(&c.dir) {
            n += usize::from(!self.rail_geometry_clear(i, c));
        }
        // Automatic in-image plates must have a short, straight, grain-clear
        // connection. Only the explicitly bounded A* last resort may bend.
        if !c.path.is_empty() && !self.pinned[i] && c.dir != ASTAR_DIR {
            let image = &self.page.images[a.image];
            if polygon_rect(image, c.rect) {
                n += usize::from(
                    c.path.len() != 2
                        || distance(c.path[0], *c.path.last().unwrap())
                            > IN_IMAGE_HEIGHTS * (c.rect[3] - c.rect[1]) + 1e-9,
                );
            }
        }
        if !c.path.is_empty()
            && !self.pinned[i]
            && (polygon_rect(&self.page.images[a.image], c.rect) || c.dir == ASTAR_DIR)
        {
            n += self
                .grains
                .query(c.bounds())
                .iter()
                .filter(|&&j| {
                    let poly = &self.page.grains[j];
                    !in_polygon(a.centre(), poly)
                        && c.path.windows(2).any(|s| {
                            in_polygon(s[0], poly)
                                || in_polygon(s[1], poly)
                                || poly
                                    .iter()
                                    .zip(poly.iter().cycle().skip(1))
                                    .take(poly.len())
                                    .any(|(a, b)| segments(s[0], s[1], *a, *b))
                        })
                })
                .count();
        }
        if c.path.iter().any(|p| !contains(self.page.body, *p)) {
            n += 1;
        }
        n
    }
    fn conflicts(&self, i: usize, c: &Candidate) -> usize {
        self.static_conflicts(i, c)
            + self
                .placed
                .query(expand(c.bounds(), LEADER_CLEARANCE))
                .iter()
                .filter(|&&j| {
                    j != i
                        && self.current[j].as_ref().is_some_and(|b| {
                            overlap(expand(c.rect, GAP / 2.0), b.rect)
                                || path_box(&c.path, b.rect)
                                || path_box(&b.path, c.rect)
                                || paths_cross(&c.path, &b.path)
                                || leaders_close(
                                    &c.path,
                                    &b.path,
                                    [self.page.anchors[i].centre(), self.page.anchors[j].centre()],
                                )
                        })
                })
                .count()
    }
    fn score(&self, i: usize, c: &Candidate) -> f64 {
        self.soft_score(i, c) + self.conflicts(i, c) as f64 * 1e6
    }
    fn ambiguous(&self, i: usize, c: &Candidate) -> bool {
        // A drawn tick or leader identifies its marker without a distance guess.
        if !c.path.is_empty() {
            return false;
        }
        let own = self.page.anchors[i].marker.box_distance(c.rect);
        // Search::set adds a connector to distant bare candidates. Do not
        // penalise their temporary path-free form before that happens.
        if own > TICK_GAP {
            return false;
        }
        let reach = 1.2 * own;
        self.markers
            .query(expand(c.rect, reach + 1e-9))
            .into_iter()
            .any(|j| j != i && self.page.anchors[j].marker.box_distance(c.rect) <= reach + 1e-9)
    }
    fn ambiguity_cost(&self, i: usize, c: &Candidate) -> f64 {
        if self.ambiguous(i, c) {
            AMBIGUITY_COST
        } else {
            0.0
        }
    }
    fn soft_score(&self, i: usize, c: &Candidate) -> f64 {
        let length = c.path.windows(2).map(|s| distance(s[0], s[1])).sum::<f64>();
        let mut cost = c.cost
            + length * 0.08
            + c.path.len().saturating_sub(2) as f64 * 0.03
            + self.ambiguity_cost(i, c);
        // Prefer a short axial tick to a distant corner plate covering less
        // grain. This also lets Auto retain readable type within its limit.
        if c.dir < 8 && c.path.len() == 2 && length >= 3.0 {
            cost += 24.0;
        }
        if c.rect[2] - c.rect[0] > 3.0 * (c.rect[3] - c.rect[1]) {
            let room = expand(c.rect, 2.5);
            cost += self
                .markers
                .query(room)
                .into_iter()
                .filter(|&j| self.page.anchors[j].marker.hits_box(room))
                .count() as f64
                * 48.0;
        }
        for index in self.grains.query(c.bounds()) {
            let poly = &self.page.grains[index];
            if polygon_rect(poly, c.rect) {
                cost += 12.0;
            }
            if c.path.windows(2).any(|s| {
                poly.iter()
                    .zip(poly.iter().cycle().skip(1))
                    .take(poly.len())
                    .any(|(a, b)| segments(s[0], s[1], *a, *b))
            }) {
                cost += 2.0;
            }
        }
        let image = &self.page.images[self.page.anchors[i].image];
        let in_count = corners(c.rect)
            .iter()
            .filter(|p| in_polygon(**p, image))
            .count();
        if in_count > 0 && in_count < 4 {
            cost += 8.0;
        }
        // Epoxy and white paper are preferred to grain interiors. A box wholly
        // on white paper gets a small preference over otherwise equal image slots.
        if in_count == 0 {
            cost -= 0.2;
        }
        // Nearby direction consistency is a preference, never a hard condition.
        for j in self
            .markers
            .query(expand(self.page.anchors[i].marker.bounds(), 12.0))
        {
            if j != i && self.current[j].as_ref().is_some_and(|b| b.dir != c.dir) {
                cost += 0.04;
            }
        }
        cost
    }
    fn bad(&self, i: usize) -> bool {
        self.conflicts(i, self.current[i].as_ref().unwrap()) > 0
    }
    fn remove(&mut self, i: usize) {
        self.generation += 1;
        if let Some(c) = self.current[i].take() {
            self.placed.remove(i, expand(c.bounds(), GAP));
        }
    }
    fn anneal(&mut self, active: &[usize], sizes: &[Point], rounds: usize) {
        if active.is_empty() {
            return;
        }
        let budget = (active.len() * rounds).min(1_024);
        let mut rng = 0x6a09e667f3bcc909u64;
        for iteration in 0..budget {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let i = active[rng as usize % active.len()];
            // Clear, unambiguous labels remain exactly where they are.
            if !self.bad(i) && !self.ambiguous(i, self.current[i].as_ref().unwrap()) {
                continue;
            }
            let old = self.current[i].as_ref().unwrap().clone();
            let raw = candidate(
                &self.page.anchors[i],
                sizes[i],
                ((rng >> 16) % 8) as usize,
                ((rng >> 24) % 6) as usize,
            );
            if self.conflicts(
                i,
                &Candidate {
                    path: vec![],
                    ..raw.clone()
                },
            ) > 0
            {
                continue;
            }
            let c = self.route_mode(i, raw, false);
            let delta = self.score(i, &c) - self.score(i, &old);
            let temp = 0.5 * (1.0 - iteration as f64 / budget as f64) + 0.001;
            let uniform = ((rng >> 32) as f64 + 1.0) / (u32::MAX as f64 + 2.0);
            if self.conflicts(i, &c) == 0 && (delta < 0.0 || uniform < (-delta / temp).exp()) {
                self.set(i, c);
            }
        }
    }
    /// Conflicting markers join within `3*max(height) + max(width)` of their
    /// centres. Grow by contained plates; large dense components also include
    /// nearby blocking plates and leaders so they cannot fence in the group.
    fn conflict_groups(&self, sizes: &[Point]) -> Vec<Vec<usize>> {
        let n = sizes.len();
        let bad: Vec<_> = (0..n).filter(|&i| self.bad(i)).collect();
        let mut groups: Vec<Vec<usize>> = bad.iter().map(|&i| vec![i]).collect();
        loop {
            let mut changed = false;
            for group in &mut groups {
                let hull = bounds(
                    &group
                        .iter()
                        .map(|&i| self.page.anchors[i].centre())
                        .collect::<Vec<_>>(),
                );
                let h = group.iter().map(|&i| sizes[i][1]).fold(0.0, f64::max);
                for i in 0..n {
                    let near = group.iter().any(|&j| {
                        let displaced = bad.binary_search(&j).is_ok()
                            && !self.pinned[j]
                            && !inside(self.page.body, self.current[j].as_ref().unwrap().rect);
                        (bad.binary_search(&i).is_ok() || displaced)
                            && distance(
                                self.page.anchors[i].centre(),
                                self.page.anchors[j].centre(),
                            ) <= 3.0 * sizes[i][1].max(sizes[j][1])
                                + sizes[i][0].max(sizes[j][0])
                    });
                    if !group.contains(&i)
                        && (near
                            || if group.len() >= 30 {
                                overlap(
                                    expand(hull, 3.0 * h + GAP),
                                    self.current[i].as_ref().unwrap().bounds(),
                                )
                            } else {
                                inside(expand(hull, h), self.current[i].as_ref().unwrap().rect)
                            })
                    {
                        group.push(i);
                        changed = true;
                    }
                }
            }
            let mut a = 0;
            while a < groups.len() {
                let mut b = a + 1;
                while b < groups.len() {
                    if groups[b].iter().any(|i| groups[a].contains(i)) {
                        let other = groups.remove(b);
                        groups[a].extend(other);
                        groups[a].sort_unstable();
                        groups[a].dedup();
                        changed = true;
                    } else {
                        b += 1;
                    }
                }
                a += 1;
            }
            if !changed {
                break;
            }
        }
        for g in &mut groups {
            g.sort_unstable();
        }
        groups.sort_by_key(|g| g[0]);
        groups
    }
    /// The label edge facing the image, measured from nearby decoration outward.
    fn rail_near(&self, image: usize, axis: usize, side: usize) -> f64 {
        let b = bounds(&self.page.images[image]);
        let cross = 1 - axis;
        let sign = if side == 0 { -1.0 } else { 1.0 };
        let edge = b[cross + side * 2];
        let mut near = edge + sign * RAIL_GAP;
        for (j, r) in self.page.hard.iter().enumerate() {
            if self.page.rail_gaps.get(j).copied().unwrap_or(RAIL_GAP) == 0.0 {
                continue;
            }
            if r[axis] < b[axis + 2] + 12.0
                && r[axis + 2] > b[axis] - 12.0
                && r[cross] < edge + 12.0
                && r[cross + 2] > edge - 12.0
            {
                if side == 0 {
                    near = near.min(r[cross] - RAIL_GAP);
                } else {
                    near = near.max(r[cross + 2] + RAIL_GAP);
                }
            }
        }
        near
    }

    fn rail_candidate(&self, i: usize, rect: Rect, axis: usize, side: usize) -> Candidate {
        let a = &self.page.anchors[i];
        let cross = 1 - axis;
        let mut exit = a.centre();
        exit[cross] = bounds(&self.page.images[a.image])[cross + side * 2]
            + if side == 0 { -RAIL_EXIT } else { RAIL_EXIT };
        let mut end = centre_of(rect);
        end[cross] = rect[cross + if side == 0 { 2 } else { 0 }];
        Candidate {
            rect,
            path: vec![a.marker.edge(exit), exit, end],
            dir: 8 + side + axis * 2,
            cost: 0.0,
        }
    }

    fn rail_prefix_clear(&self, i: usize, axis: usize, side: usize) -> bool {
        let c = self.rail_candidate(i, [0.0; 4], axis, side);
        let path = &c.path[..2];
        self.path_static_clear(i, path)
            && self
                .placed
                .query(expand(bounds(path), LEADER_CLEARANCE))
                .iter()
                .all(|&j| {
                    j == i
                        || self.current[j].as_ref().is_none_or(|d| {
                            !path_box(path, d.rect)
                                && !paths_cross(path, &d.path)
                                && !leaders_close(
                                    path,
                                    &d.path,
                                    [self.page.anchors[i].centre(), self.page.anchors[j].centre()],
                                )
                        })
                })
    }

    fn rail_geometry_clear(&self, i: usize, c: &Candidate) -> bool {
        let axis = (c.dir - 8) / 2;
        let side = (c.dir - 8) % 2;
        let cross = 1 - axis;
        let p = self.page.anchors[i].centre();
        let expected = self.rail_candidate(i, c.rect, axis, side);
        let exit = expected.path[1];
        let end = expected.path[2];
        let dx = (end[axis] - p[axis]).abs();
        let dy = (end[cross] - exit[cross]).abs();
        let first =
            (end[cross] - self.rail_near(self.page.anchors[i].image, axis, side)).abs() < 1e-8;
        c.path == expected.path && (dx <= dy + 1e-9 || (first && dx < 3.0))
    }

    /// Keep clear adjacent plates fixed. Only unresolved/expensive plates and
    /// leaders participate in the image boundary assignment.
    fn boundary_rails(&mut self, sizes: &[Point], pinned: &[bool]) {
        for image in 0..self.page.images.len() {
            let mut movable = Vec::new();
            for (i, &pin) in pinned.iter().enumerate() {
                if pin || self.page.anchors[i].image != image {
                    continue;
                }
                let c = self.current[i].as_ref().unwrap();
                let covers_middle = self.covers_middle(i, c);
                if self.bad(i) || c.dir >= 8 || (covers_middle && c.path.is_empty()) {
                    movable.push(i);
                }
            }
            if movable.is_empty() {
                continue;
            }
            let b = bounds(&self.page.images[image]);
            let axis = usize::from(b[3] - b[1] > b[2] - b[0]);
            let cross = 1 - axis;
            let maxw = movable.iter().map(|&i| sizes[i][axis]).fold(0.0, f64::max);
            let maxh = movable.iter().map(|&i| sizes[i][cross]).fold(0.0, f64::max);
            let lo = self.page.body[axis];
            let hi = self.page.body[axis + 2];
            let capacity =
                ((hi - lo + LEADER_CLEARANCE) / (maxw + LEADER_CLEARANCE)).floor() as usize;
            if capacity == 0 {
                continue;
            }
            let old_bad = (0..sizes.len()).filter(|&i| self.bad(i)).count();
            movable.sort_by(|&a, &z| {
                self.page.anchors[a].centre()[axis]
                    .total_cmp(&self.page.anchors[z].centre()[axis])
                    .then(a.cmp(&z))
            });
            let previous: Vec<_> = movable
                .iter()
                .map(|&i| self.current[i].clone().unwrap())
                .collect();
            for &i in &movable {
                self.remove(i);
            }
            let near_edges = [
                self.rail_near(image, axis, 0),
                self.rail_near(image, axis, 1),
            ];
            // Build each tier from the image outward. Failed insertions stay
            // pending for the other side or the next tier; a capacity estimate
            // must never force a whole row to a distant page-edge rail.
            let reachable: HashMap<_, _> = movable
                .iter()
                .map(|&i| {
                    (
                        i,
                        [
                            self.rail_prefix_clear(i, axis, 0),
                            self.rail_prefix_clear(i, axis, 1),
                        ],
                    )
                })
                .collect();
            let mut pending: Vec<_> = movable
                .iter()
                .copied()
                .filter(|i| reachable[i].iter().any(|v| *v))
                .collect();
            let max_tiers = ((self.page.body[cross + 2] - self.page.body[cross])
                / (maxh + RAIL_GAP))
                .ceil() as usize;
            for tier in 0..max_tiers {
                if pending.is_empty() {
                    break;
                }
                let mut groups: [Vec<usize>; 2] = [vec![], vec![]];
                let mut candidates: [Vec<Candidate>; 2] = [vec![], vec![]];
                let mut remaining = Vec::new();
                for i in pending {
                    let mut best = None;
                    for side in 0..2 {
                        if !reachable[&i][side] || groups[side].len() == capacity {
                            continue;
                        }
                        let mut ids = groups[side].clone();
                        ids.push(i);
                        let sign = if side == 0 { -1.0 } else { 1.0 };
                        let near = near_edges[side] + sign * tier as f64 * (maxh + RAIL_GAP);
                        if let Some(placed) =
                            self.ordered_rail(&ids, sizes, axis, side, lo, hi, near)
                        {
                            let displacement = |ids: &[usize], cs: &[Candidate]| {
                                ids.iter()
                                    .zip(cs)
                                    .map(|(&j, c)| {
                                        (centre_of(c.rect)[axis]
                                            - self.page.anchors[j].centre()[axis])
                                            .powi(2)
                                    })
                                    .sum::<f64>()
                            };
                            let cost = displacement(&ids, &placed)
                                - displacement(&groups[side], &candidates[side])
                                + (self.page.anchors[i].centre()[cross] - near).abs();
                            if best.as_ref().is_none_or(|(old, _, _)| cost < *old) {
                                best = Some((cost, side, placed));
                            }
                        }
                    }
                    if let Some((_, side, placed)) = best {
                        groups[side].push(i);
                        candidates[side] = placed;
                    } else {
                        remaining.push(i);
                    }
                }
                for side in 0..2 {
                    for (&i, c) in groups[side].iter().zip(&candidates[side]) {
                        self.set(i, c.clone());
                    }
                }
                pending = remaining;
            }
            for (&i, old) in movable.iter().zip(&previous) {
                if self.current[i].is_none() {
                    self.set(i, old.clone());
                }
            }
            if (0..sizes.len()).filter(|&i| self.bad(i)).count() > old_bad {
                for &i in &movable {
                    self.remove(i);
                }
                for (&i, old) in movable.iter().zip(previous) {
                    self.set(i, old);
                }
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn ordered_rail(
        &self,
        ids: &[usize],
        sizes: &[Point],
        axis: usize,
        side: usize,
        lo: f64,
        hi: f64,
        near: f64,
    ) -> Option<Vec<Candidate>> {
        let cross = 1 - axis;
        let sign = if side == 0 { -1.0 } else { 1.0 };
        if ids.iter().any(|&i| !self.rail_prefix_clear(i, axis, side)) {
            return None;
        }
        // A sliver against the body boundary is not usable white paper: keep
        // the same gutter from header/footer territory as from decoration.
        if near.min(near + sign * sizes[ids[0]][cross]) < self.page.body[cross] + RAIL_GAP
            || near.max(near + sign * sizes[ids[0]][cross]) > self.page.body[cross + 2] - RAIL_GAP
        {
            return None;
        }
        // Transform left edges by the cumulative widths. Nondecreasing offsets
        // then enforce order and 0.8 mm plate spacing, including unequal names.
        let widths: f64 = ids.iter().map(|&i| sizes[i][axis]).sum();
        let slack = hi - lo - widths - (ids.len() - 1) as f64 * LEADER_CLEARANCE;
        if slack < 0.0 {
            return None;
        }
        let mut offsets: Vec<f64> = (0..=(slack / 0.5).floor() as usize)
            .map(|k| k as f64 * 0.5)
            .collect();
        offsets.push(slack);
        let mut prefix = 0.0;
        for &i in ids {
            offsets.push(
                (self.page.anchors[i].centre()[axis] - sizes[i][axis] / 2.0 - lo - prefix)
                    .clamp(0.0, slack),
            );
            prefix += sizes[i][axis] + LEADER_CLEARANCE;
        }
        offsets.sort_by(f64::total_cmp);
        offsets.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let mut options = Vec::new();
        prefix = 0.0;
        for &i in ids {
            let p = self.page.anchors[i].centre();
            let mut choices = Vec::new();
            for (slot, &offset) in offsets.iter().enumerate() {
                let mut r = [0.0; 4];
                r[axis] = lo + prefix + offset;
                r[axis + 2] = r[axis] + sizes[i][axis];
                r[cross] = near - if side == 0 { sizes[i][cross] } else { 0.0 };
                r[cross + 2] = r[cross] + sizes[i][cross];
                let c = self.rail_candidate(i, r, axis, side);
                if !self.rail_geometry_clear(i, &c)
                    || self.conflicts(i, &c) > 0
                    || self.page.hard.iter().enumerate().any(|(j, &h)| {
                        let gap = self.page.rail_gaps.get(j).copied().unwrap_or(RAIL_GAP);
                        overlap(r, expand(h, gap)) || path_box(&c.path, expand(h, gap.min(0.16)))
                    })
                {
                    continue;
                }
                let cost = (centre_of(r)[axis] - p[axis]).powi(2);
                choices.push((slot, cost, c));
            }
            if choices.is_empty() {
                return None;
            }
            options.push(choices);
            prefix += sizes[i][axis] + LEADER_CLEARANCE;
        }
        // Dynamic programming minimises total squared displacement instead of
        // accepting the first feasible DFS branch. Adjacent routes must clear;
        // the final assignment is also checked against every nonadjacent route.
        let anchors: Vec<_> = ids.iter().map(|&i| self.page.anchors[i].centre()).collect();
        let mut costs: Vec<f64> = options[0].iter().map(|c| c.1).collect();
        let mut parents = vec![vec![]];
        for row in 1..ids.len() {
            let mut next = vec![f64::INFINITY; options[row].len()];
            let mut from = vec![usize::MAX; next.len()];
            // Inspect predecessors by cost. The first compatible one is the
            // exact minimum, avoiding a quadratic scan across distant slots.
            let mut ranked: Vec<_> = (0..costs.len()).filter(|&q| costs[q].is_finite()).collect();
            ranked.sort_by(|&a, &b| costs[a].total_cmp(&costs[b]).then(a.cmp(&b)));
            for (k, (slot, cost, c)) in options[row].iter().enumerate() {
                for &q in &ranked {
                    let (prev, _, d) = &options[row - 1][q];
                    if prev > slot || costs[q] + cost >= next[k] {
                        continue;
                    }
                    if paths_cross(&c.path, &d.path)
                        || leaders_close(&c.path, &d.path, [anchors[row], anchors[row - 1]])
                        || path_box(&c.path, d.rect)
                        || path_box(&d.path, c.rect)
                    {
                        continue;
                    }
                    next[k] = costs[q] + cost;
                    from[k] = q;
                    break;
                }
            }
            costs = next;
            parents.push(from);
        }
        let mut k =
            (0..costs.len()).min_by(|&a, &b| costs[a].total_cmp(&costs[b]).then(a.cmp(&b)))?;
        if !costs[k].is_finite() {
            return None;
        }
        let mut selected = Vec::new();
        for row in (0..ids.len()).rev() {
            selected.push(options[row][k].2.clone());
            if row > 0 {
                k = parents[row][k];
            }
        }
        selected.reverse();
        for (row, c) in selected.iter().enumerate() {
            for (other, d) in selected[..row].iter().enumerate() {
                if paths_cross(&c.path, &d.path)
                    || leaders_close(&c.path, &d.path, [anchors[row], anchors[other]])
                    || path_box(&c.path, d.rect)
                    || path_box(&d.path, c.rect)
                {
                    return None;
                }
            }
        }
        Some(selected)
    }

    /// Pull resolved rails inward after the joint repair. Each accepted move
    /// preserves every hard constraint and anchor order within its tier.
    fn compact_rails(&mut self, sizes: &[Point], pinned: &[bool]) {
        for _ in 0..32 {
            let mut changed = false;
            for (i, &pin) in pinned.iter().enumerate() {
                let old = self.current[i].as_ref().unwrap().clone();
                if pin || !(8..ASTAR_DIR).contains(&old.dir) || self.bad(i) {
                    continue;
                }
                let axis = (old.dir - 8) / 2;
                let cross = 1 - axis;
                let old_side = (old.dir - 8) % 2;
                let p = self.page.anchors[i].centre();
                let image = self.page.anchors[i].image;
                let pitch = sizes[i][cross] + RAIL_GAP;
                let old_near = old.rect[cross + if old_side == 0 { 2 } else { 0 }];
                let old_tier = ((old_near - self.rail_near(image, axis, old_side)).abs() / pitch)
                    .round() as usize;
                let mut best = old.clone();
                let mut best_key = (old_tier, (centre_of(old.rect)[axis] - p[axis]).abs());
                self.remove(i);
                for tier in 0..=old_tier {
                    for side in 0..2 {
                        let sign = if side == 0 { -1.0 } else { 1.0 };
                        let near = self.rail_near(image, axis, side) + sign * tier as f64 * pitch;
                        let mut offsets = vec![0.0, centre_of(old.rect)[axis] - p[axis]];
                        offsets.extend((-160..=160).map(|k| k as f64 * 0.25));
                        for dx in offsets {
                            if tier > best_key.0
                                || (tier == best_key.0 && dx.abs() >= best_key.1 - 1e-9)
                            {
                                continue;
                            }
                            let mut r = [0.0; 4];
                            r[axis] = p[axis] + dx - sizes[i][axis] / 2.0;
                            r[axis + 2] = r[axis] + sizes[i][axis];
                            r[cross] = near - if side == 0 { sizes[i][cross] } else { 0.0 };
                            r[cross + 2] = r[cross] + sizes[i][cross];
                            let c = self.rail_candidate(i, r, axis, side);
                            if !self.rail_geometry_clear(i, &c)
                                || self.conflicts(i, &c) != 0
                                || self.page.hard.iter().enumerate().any(|(j, &h)| {
                                    let gap =
                                        self.page.rail_gaps.get(j).copied().unwrap_or(RAIL_GAP);
                                    overlap(r, expand(h, gap))
                                        || path_box(&c.path, expand(h, gap.min(0.16)))
                                })
                                || self.current.iter().enumerate().any(|(j, d)| {
                                    d.as_ref().is_some_and(|d| {
                                        j != i
                                            && self.page.anchors[j].image == image
                                            && d.dir == c.dir
                                            && (d.rect[cross] - r[cross]).abs() < 1e-8
                                            && ((p[axis] - self.page.anchors[j].centre()[axis])
                                                * (r[axis] - d.rect[axis])
                                                < 0.0
                                                || overlap(expand(r, LEADER_CLEARANCE), d.rect))
                                    })
                                })
                            {
                                continue;
                            }
                            best = c;
                            best_key = (tier, dx.abs());
                        }
                    }
                    if best_key.0 == tier {
                        break;
                    }
                }
                changed |= best.rect != old.rect;
                self.set(i, best);
            }
            if !changed {
                break;
            }
        }
    }

    fn reflow_rails(&mut self, sizes: &[Point], pinned: &[bool]) {
        let mut tiers: BTreeMap<(usize, usize, u64), Vec<usize>> = BTreeMap::new();
        for (i, &pin) in pinned.iter().enumerate() {
            let c = self.current[i].as_ref().unwrap();
            if pin || !(8..ASTAR_DIR).contains(&c.dir) {
                continue;
            }
            let cross = 1 - (c.dir - 8) / 2;
            let side = (c.dir - 8) % 2;
            let near = c.rect[cross + if side == 0 { 2 } else { 0 }];
            tiers
                .entry((self.page.anchors[i].image, c.dir, near.to_bits()))
                .or_default()
                .push(i);
        }
        for ((_, dir, near), mut ids) in tiers {
            let axis = (dir - 8) / 2;
            let side = (dir - 8) % 2;
            ids.sort_by(|&a, &b| {
                self.page.anchors[a].centre()[axis]
                    .total_cmp(&self.page.anchors[b].centre()[axis])
                    .then(a.cmp(&b))
            });
            let old: Vec<_> = ids
                .iter()
                .map(|&i| self.current[i].as_ref().unwrap().clone())
                .collect();
            let objective = |cs: &[Candidate]| {
                ids.iter()
                    .zip(cs)
                    .map(|(&i, c)| {
                        (centre_of(c.rect)[axis] - self.page.anchors[i].centre()[axis]).powi(2)
                    })
                    .sum::<f64>()
            };
            for &i in &ids {
                self.remove(i);
            }
            let placed = self
                .ordered_rail(
                    &ids,
                    sizes,
                    axis,
                    side,
                    self.page.body[axis],
                    self.page.body[axis + 2],
                    f64::from_bits(near),
                )
                .filter(|cs| objective(cs) <= objective(&old) + 1e-9)
                .unwrap_or(old);
            for (&i, c) in ids.iter().zip(placed) {
                self.set(i, c);
            }
        }
    }
    /// Joint short-slot/rail refinement for an image with blocked exits. Keeping
    /// both choices in one bounded search lets a rail plate free a short slot.
    fn repair_boundary(&mut self, sizes: &[Point], pinned: &[bool], local: bool) {
        for image in 0..self.page.images.len() {
            let bad: Vec<_> = (0..sizes.len())
                .filter(|&i| self.page.anchors[i].image == image && self.bad(i))
                .collect();
            let ids: Vec<_> = (0..sizes.len())
                .filter(|&i| {
                    !pinned[i]
                        && self.page.anchors[i].image == image
                        && (!local
                            || bad.iter().any(|&j| {
                                distance(
                                    self.page.anchors[i].centre(),
                                    self.page.anchors[j].centre(),
                                ) < sizes[i][0] + 3.0 * sizes[i][1]
                            }))
                })
                .collect();
            if !ids.iter().any(|&i| self.bad(i)) || ids.len() > 80 {
                continue;
            }
            let b = bounds(&self.page.images[image]);
            let axis = usize::from(b[3] - b[1] > b[2] - b[0]);
            let cross = 1 - axis;
            // This repair is for boundary rails with substantial white paper.
            if (b[cross] - self.page.body[cross]).max(self.page.body[cross + 2] - b[cross + 2])
                < 20.0
            {
                continue;
            }
            if self.repair_calls_left.get() == 0 || self.repair_pairs_left.get() == 0 {
                continue;
            }
            self.repair_calls_left.set(self.repair_calls_left.get() - 1);
            let previous: Vec<_> = ids
                .iter()
                .map(|&i| self.current[i].clone().unwrap())
                .collect();
            for &i in &ids {
                self.remove(i);
            }
            let foreign_images: Vec<_> = self
                .page
                .images
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != image)
                .collect();
            let edge_markers: Vec<_> = self
                .page
                .anchors
                .iter()
                .enumerate()
                .filter(|(_, a)| {
                    !inside(
                        bounds(&self.page.images[a.image]),
                        expand(a.marker.bounds(), GAP),
                    )
                })
                .collect();
            let scores = RefCell::new(HashMap::new());
            let score_key = |i: usize, c: &Candidate| {
                (
                    i,
                    c.rect.map(f64::to_bits),
                    c.path.get(1).copied().unwrap_or([0.0; 2]).map(f64::to_bits),
                    c.dir,
                    c.cost.to_bits(),
                )
            };
            let score = |i: usize, c: &Candidate| {
                let key = score_key(i, c);
                if let Some(&v) = scores.borrow().get(&key) {
                    return v;
                }
                let v = self.soft_score(i, c);
                scores.borrow_mut().insert(key, v);
                v
            };
            let mut pools = Vec::new();
            for (row, &i) in ids.iter().enumerate() {
                let p = self.page.anchors[i].centre();
                let [w, h] = sizes[i];
                let mut pool = vec![previous[row].clone()];
                for dir in 0..8 {
                    let c = candidate(&self.page.anchors[i], sizes[i], dir, 0);
                    if self.static_conflicts(i, &c) == 0 && !self.covers_middle(i, &c) {
                        pool.push(c);
                    }
                }
                let mut xs: Vec<_> = (-((2.0 * w / h).ceil() as i32 + 4)..=4)
                    .map(|dx| p[0] + dx as f64 * h * 0.5)
                    .collect();
                let mut ys: Vec<_> = (-6..=4).map(|dy| p[1] + dy as f64 * h * 0.5).collect();
                let mut neighbours: Vec<_> = self
                    .page
                    .anchors
                    .iter()
                    .filter(|a| a.image == image && distance(p, a.centre()) < w + 3.0 * h)
                    .collect();
                neighbours.sort_by(|a, b| {
                    distance(p, a.centre())
                        .total_cmp(&distance(p, b.centre()))
                        .then(a.spot_id.cmp(&b.spot_id))
                });
                neighbours.truncate(16);
                for a in neighbours {
                    let r = a.marker.bounds();
                    ys.extend([r[1] - GAP - 0.02 - h, r[3] + GAP + 0.02]);
                    xs.extend([a.centre()[0] + 0.18, a.centre()[0] - 0.18 - w]);
                }
                // Font heights can leave narrow usable corridors between a
                // decoration and the marker row. Include their exact edges,
                // rather than relying only on a half-height lattice.
                for &r in &self.page.hard {
                    if overlap(expand(bounds(&[p]), w + 3.0 * h), r) {
                        xs.extend([r[0] - GAP - 0.02 - w, r[2] + GAP + 0.02]);
                        ys.extend([r[1] - GAP - 0.02 - h, r[3] + GAP + 0.02]);
                    }
                }
                xs.retain(|&x| x >= p[0] - w - 2.0 * h && x <= p[0] + 2.0 * h);
                ys.retain(|&y| y >= p[1] - 3.0 * h && y <= p[1] + 2.0 * h);
                xs.sort_by(f64::total_cmp);
                xs.dedup();
                ys.sort_by(f64::total_cmp);
                ys.dedup();
                for &x in &xs {
                    for &y in &ys {
                        let rect = [x, y, x + w, y + h];
                        let end = [p[0].clamp(x, x + w), p[1].clamp(y, y + h)];
                        let c = Candidate {
                            rect,
                            path: vec![self.page.anchors[i].marker.edge(end), end],
                            dir: 0,
                            cost: 3.0,
                        };
                        if self.static_conflicts(i, &c) == 0 {
                            pool.push(c);
                        } else if ids.len() <= 16 && self.plate_possible(rect) {
                            pool.extend(self.straight_alternatives(i, &c));
                        }
                    }
                }
                let wide_room = w > 3.0 * h
                    && self
                        .markers
                        .query(expand(previous[row].rect, 2.5))
                        .into_iter()
                        .any(|j| {
                            self.page.anchors[j]
                                .marker
                                .hits_box(expand(previous[row].rect, 2.5))
                        });
                if ids.len() <= 20
                    && (self.static_conflicts(i, &previous[row]) > 0
                        || pool.len() < 4
                        || (ids.len() <= 16 && pool.len() < 32)
                        || wide_room)
                {
                    let mut edges = edge_slots(self.page, &self.page.anchors[i], sizes[i]);
                    for dx in -6..=16 {
                        for dy in -5..=6 {
                            let x = p[0] + dx as f64 * h * 0.5;
                            let y = p[1] + dy as f64 * h * 0.5;
                            let rect = [x, y, x + w, y + h];
                            let plate = Candidate {
                                rect,
                                path: vec![],
                                dir: 0,
                                cost: 3.0,
                            };
                            if self.static_conflicts(i, &plate) == 0 {
                                edges.push(rect);
                            }
                        }
                    }
                    if w > 3.0 * h {
                        edges.retain(|r| {
                            let room = expand(*r, 2.5);
                            self.markers
                                .query(room)
                                .into_iter()
                                .all(|j| !self.page.anchors[j].marker.hits_box(room))
                        });
                    }
                    edges.sort_by(|a, b| {
                        distance(p, centre_of(*a))
                            .total_cmp(&distance(p, centre_of(*b)))
                            .then_with(|| {
                                a.iter()
                                    .zip(b)
                                    .find_map(|(a, b)| {
                                        let order = a.total_cmp(b);
                                        (order != std::cmp::Ordering::Equal).then_some(order)
                                    })
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            })
                    });
                    edges.dedup();
                    // Nearest-only route trials can all use the same blocked
                    // exit. Keep nearby slots and a spatially varied remainder
                    // within the existing 32-route budget.
                    let mut trials: Vec<_> = edges.drain(..edges.len().min(16)).collect();
                    while trials.len() < 32 && !edges.is_empty() {
                        let at = (0..edges.len())
                            .max_by_key(|&j| {
                                cost_key(
                                    trials
                                        .iter()
                                        .map(|r| distance(centre_of(*r), centre_of(edges[j])))
                                        .fold(f64::INFINITY, f64::min),
                                )
                            })
                            .unwrap();
                        trials.push(edges.remove(at));
                    }
                    for rect in trials {
                        let c = self.route(
                            i,
                            Candidate {
                                rect,
                                path: vec![p, [rect[0], rect[1]]],
                                dir: 0,
                                cost: 3.0,
                            },
                        );
                        if self.static_conflicts(i, &c) == 0 {
                            pool.push(c);
                        }
                    }
                }
                pool.sort_by_cached_key(|a| cost_key(self.soft_score(i, a)));

                for side in 0..2 {
                    let rail_dir = 8 + side + axis * 2;
                    let consistency = self
                        .markers
                        .query(expand(self.page.anchors[i].marker.bounds(), 12.0))
                        .iter()
                        .filter(|&&j| {
                            j != i && self.current[j].as_ref().is_some_and(|b| b.dir != rail_dir)
                        })
                        .count() as f64
                        * 0.04;
                    let sign = if side == 0 { -1.0 } else { 1.0 };
                    let edge = b[cross + side * 2];
                    let first_near = self.rail_near(image, axis, side);
                    let mut rail = Vec::new();
                    for tier in 0..4 {
                        let near = first_near + sign * tier as f64 * (sizes[i][cross] + RAIL_GAP);
                        let mut exit = p;
                        exit[cross] = edge + sign * RAIL_EXIT;
                        let start = self.page.anchors[i].marker.edge(exit);
                        let prefix = [start, exit];
                        let exits: Vec<_> = if self.path_static_clear(i, &prefix) {
                            vec![(start, exit, 0.0)]
                        } else {
                            vec![]
                        };
                        let mut x = (p[axis] - sizes[i][axis] / 2.0 - 15.0)
                            .max(self.page.body[axis] + RAIL_GAP);
                        let right = (p[axis] + sizes[i][axis] / 2.0 + 15.0)
                            .min(self.page.body[axis + 2] - RAIL_GAP);
                        while x + sizes[i][axis] <= right {
                            let mut rect = [0.0; 4];
                            rect[axis] = x;
                            rect[axis + 2] = x + sizes[i][axis];
                            rect[cross] = near - if side == 0 { sizes[i][cross] } else { 0.0 };
                            rect[cross + 2] = rect[cross] + sizes[i][cross];
                            let mut end = centre_of(rect);
                            end[cross] = near;
                            let mut alternatives = Vec::new();
                            for &(start, exit, grain_cost) in &exits {
                                let c = Candidate {
                                    rect,
                                    path: vec![exit, end],
                                    dir: 8 + side + axis * 2,
                                    cost: (centre_of(rect)[axis] - p[axis]).powi(2)
                                        + distance(p, end) * 0.1
                                        + tier as f64 * 1000.0,
                                };
                                let region = c.bounds();
                                if !inside(self.page.body, rect)
                                    || c.path.iter().any(|&p| !contains(self.page.body, p))
                                    || edge_markers.iter().any(|&(j, a)| {
                                        overlap(expand(a.marker.bounds(), GAP), region)
                                            && (a.marker.hits_box(rect)
                                                || (j != i && a.marker.hits_path(&c.path)))
                                    })
                                    || foreign_images.iter().any(|&(_, poly)| {
                                        overlap(bounds(poly), region)
                                            && (polygon_rect(poly, rect)
                                                || c.path.iter().any(|&p| in_polygon(p, poly))
                                                || c.path.windows(2).any(|s| {
                                                    poly.iter()
                                                        .zip(poly.iter().cycle().skip(1))
                                                        .take(poly.len())
                                                        .any(|(a, b)| segments(s[0], s[1], *a, *b))
                                                }))
                                    })
                                    || self.page.hard.iter().enumerate().any(|(j, &r)| {
                                        let gap =
                                            self.page.rail_gaps.get(j).copied().unwrap_or(RAIL_GAP);
                                        overlap(rect, expand(r, gap))
                                            || path_box(&c.path, expand(r, gap.min(0.16)))
                                    })
                                {
                                    continue;
                                }
                                let score = c.cost
                                    + (distance(start, exit) + distance(exit, end)) * 0.08
                                    + 0.03
                                    + grain_cost
                                    - 0.2;
                                let full = Candidate {
                                    path: vec![start, exit, end],
                                    ..c
                                };
                                if !self.rail_geometry_clear(i, &full) {
                                    continue;
                                }
                                scores
                                    .borrow_mut()
                                    .insert(score_key(i, &full), score + consistency);
                                alternatives.push((score, full));
                            }
                            alternatives.sort_by(|a, b| a.0.total_cmp(&b.0));
                            if let Some((_, best)) = alternatives.first() {
                                rail.push(best.clone());
                            }
                            x += 0.5;
                        }
                    }
                    rail.sort_by_cached_key(|a| cost_key(score(i, a)));

                    pool.extend(rail);
                }
                pools.push(pool);
            }
            for (row, pool) in pools.iter_mut().enumerate() {
                pool.retain(|c| self.conflicts(ids[row], c) == 0);
                pool.sort_by_cached_key(|a| (usize::from(a.dir < 8), cost_key(score(ids[row], a))));
            }
            // Keep a small, spatially varied set in each of the three lanes.
            for (row, pool) in pools.iter_mut().enumerate() {
                let mut compact = Vec::new();
                for lane in 0..3 {
                    let mut available: Vec<_> = pool
                        .iter()
                        .filter(|c| {
                            if lane == 0 {
                                c.dir < 8 || c.dir == ASTAR_DIR
                            } else {
                                c.dir == 8 + (lane - 1) + axis * 2
                            }
                        })
                        .cloned()
                        .collect();
                    let mut chosen: Vec<Candidate> = Vec::new();
                    let mut nearest = vec![f64::INFINITY; available.len()];
                    while !available.is_empty() && chosen.len() < if lane == 0 { 64 } else { 32 } {
                        let at = if chosen.is_empty() {
                            available
                                .iter()
                                .position(|c| {
                                    c.rect == previous[row].rect && c.path == previous[row].path
                                })
                                .unwrap_or(0)
                        } else {
                            (0..available.len())
                                .max_by(|&a, &b| nearest[a].total_cmp(&nearest[b]).then(b.cmp(&a)))
                                .unwrap()
                        };
                        let c = available.remove(at);
                        nearest.remove(at);
                        for (j, d) in available.iter().enumerate() {
                            let spread = distance([c.rect[0], c.rect[1]], [d.rect[0], d.rect[1]])
                                + if c.dir >= 8 && d.dir >= 8 {
                                    2.0 * (c.path[1][axis] - d.path[1][axis]).abs()
                                } else {
                                    0.0
                                };
                            nearest[j] = nearest[j].min(spread);
                        }
                        chosen.push(c);
                    }
                    compact.extend(chosen);
                }
                *pool = compact;
            }
            if pools.iter().any(Vec::is_empty) {
                for (&i, c) in ids.iter().zip(previous) {
                    self.set(i, c);
                }
                continue;
            }
            let count = ids.len();
            let mut clash: Vec<Vec<Vec<u128>>> = pools
                .iter()
                .map(|p| vec![vec![0; count]; p.len()])
                .collect();
            let regions: Vec<Vec<_>> = pools
                .iter()
                .map(|p| p.iter().map(Candidate::bounds).collect())
                .collect();
            let mut complete = true;
            'pairs: for row in 0..count {
                for other in row + 1..count {
                    let anchors = [
                        self.page.anchors[ids[row]].centre(),
                        self.page.anchors[ids[other]].centre(),
                    ];
                    for (k, a) in pools[row].iter().enumerate() {
                        for (q, b) in pools[other].iter().enumerate() {
                            let order = a.dir >= 8
                                && a.dir == b.dir
                                && (a.rect[cross] - b.rect[cross]).abs() < 1e-8
                                && ((anchors[0][axis] - anchors[1][axis])
                                    * (a.rect[axis] - b.rect[axis])
                                    < 0.0
                                    || overlap(expand(a.rect, LEADER_CLEARANCE), b.rect));
                            if order {
                                clash[row][k][other] |= 1u128 << q;
                                clash[other][q][row] |= 1u128 << k;
                                continue;
                            }
                            if !overlap(
                                expand(regions[row][k], LEADER_CLEARANCE),
                                regions[other][q],
                            ) {
                                continue;
                            }
                            if self.repair_pairs_left.get() == 0 {
                                complete = false;
                                break 'pairs;
                            }
                            self.repair_pairs_left.set(self.repair_pairs_left.get() - 1);
                            // The tail of a rail is outside the image. When
                            // the other route stays inside, only its exit
                            // segment can interact with it. Opposite rail tails
                            // also occupy disjoint half planes.
                            let a_rail = (8..ASTAR_DIR).contains(&a.dir);
                            let b_rail = (8..ASTAR_DIR).contains(&b.dir);
                            let opposite = a_rail && b_rail && a.dir != b.dir;
                            let ap = if a_rail
                                && (opposite
                                    || (b.dir < 8
                                        && inside(
                                            bounds(&self.page.images[image]),
                                            regions[other][q],
                                        )))
                            {
                                &a.path[..2]
                            } else {
                                &a.path[..]
                            };
                            let bp = if b_rail
                                && (opposite
                                    || (a.dir < 8
                                        && inside(
                                            bounds(&self.page.images[image]),
                                            regions[row][k],
                                        )))
                            {
                                &b.path[..2]
                            } else {
                                &b.path[..]
                            };
                            let ar = if ap.len() != a.path.len() {
                                bounds(ap)
                            } else {
                                regions[row][k]
                            };
                            let br = if bp.len() != b.path.len() {
                                bounds(bp)
                            } else {
                                regions[other][q]
                            };
                            let collision = order
                                || (overlap(expand(ar, LEADER_CLEARANCE), br)
                                    && (overlap(expand(a.rect, GAP / 2.0), b.rect)
                                        || path_box(ap, b.rect)
                                        || path_box(bp, a.rect)
                                        || paths_cross(ap, bp)
                                        || leaders_close(ap, bp, anchors)));
                            if collision {
                                clash[row][k][other] |= 1u128 << q;
                                clash[other][q][row] |= 1u128 << k;
                            }
                        }
                    }
                }
            }
            if !complete {
                for (&i, c) in ids.iter().zip(previous) {
                    self.set(i, c);
                }
                continue;
            }
            let costs: Vec<Vec<f64>> = pools
                .iter()
                .enumerate()
                .map(|(r, p)| p.iter().map(|c| score(ids[r], c)).collect())
                .collect();
            let mut selected = vec![0; count];
            let conflicts = |row: usize, k: usize, state: &[usize]| {
                (0..count)
                    .filter(|&j| j != row && clash[row][k][j] & (1u128 << state[j]) != 0)
                    .collect::<Vec<_>>()
            };
            for row in 0..count {
                selected[row] = (0..pools[row].len())
                    .min_by_key(|&k| {
                        (
                            (0..row)
                                .filter(|&j| clash[row][k][j] & (1u128 << selected[j]) != 0)
                                .count(),
                            cost_key(costs[row][k]),
                        )
                    })
                    .unwrap();
            }
            if count <= 20 {
                fn solve(
                    domains: Vec<u128>,
                    chosen: &mut [usize],
                    clash: &[Vec<Vec<u128>>],
                    budget: &mut usize,
                ) -> bool {
                    if *budget == 0 {
                        return false;
                    }
                    *budget -= 1;
                    let row = (0..chosen.len())
                        .filter(|&r| chosen[r] == usize::MAX)
                        .min_by_key(|&r| (domains[r].count_ones(), r));
                    let Some(row) = row else {
                        return true;
                    };
                    let mut options = domains[row];
                    while options != 0 {
                        let k = options.trailing_zeros() as usize;
                        options &= options - 1;
                        let mut next = domains.clone();
                        let mut viable = true;
                        for other in 0..chosen.len() {
                            if other != row && chosen[other] == usize::MAX {
                                next[other] &= !clash[row][k][other];
                                viable &= next[other] != 0;
                            }
                        }
                        if viable {
                            chosen[row] = k;
                            if solve(next, chosen, clash, budget) {
                                return true;
                            }
                            chosen[row] = usize::MAX;
                        }
                    }
                    false
                }
                let mut solution = vec![usize::MAX; count];
                let mut solved = false;
                // Prefer room to move a wide label. A tight edge corridor may
                // fit the plate but leave its first small drag on the marker.
                for roomy in [true, false] {
                    let domains = pools
                        .iter()
                        .enumerate()
                        .map(|(row, pool)| {
                            let a = &self.page.anchors[ids[row]];
                            let wide = sizes[ids[row]][0] > 3.0 * sizes[ids[row]][1];
                            pool.iter().enumerate().fold(0u128, |bits, (k, c)| {
                                let p = a.centre();
                                let end = [
                                    p[0].clamp(c.rect[0], c.rect[2]),
                                    p[1].clamp(c.rect[1], c.rect[3]),
                                ];
                                let room = expand(c.rect, 2.5);
                                let marker_room = self
                                    .markers
                                    .query(room)
                                    .into_iter()
                                    .all(|j| !self.page.anchors[j].marker.hits_box(room));
                                if !roomy
                                    || !wide
                                    || (distance(a.marker.edge(end), end) >= sizes[ids[row]][1]
                                        && marker_room)
                                {
                                    bits | (1u128 << k)
                                } else {
                                    bits
                                }
                            })
                        })
                        .collect();
                    if solve(domains, &mut solution, &clash, &mut 20_000) {
                        solved = true;
                        break;
                    }
                }
                if solved {
                    for (row, &i) in ids.iter().enumerate() {
                        self.set(i, pools[row][solution[row]].clone());
                    }
                    continue;
                }
            }
            let mut best = selected.clone();
            let mut best_count = usize::MAX;
            let mut tabu: Vec<Vec<usize>> = pools.iter().map(|p| vec![0; p.len()]).collect();
            let mut violation: Vec<Vec<i32>> = pools
                .iter()
                .enumerate()
                .map(|(r, p)| {
                    (0..p.len())
                        .map(|k| conflicts(r, k, &selected).len() as i32)
                        .collect()
                })
                .collect();
            let mut energy = (0..count).map(|r| violation[r][selected[r]]).sum::<i32>() / 2;
            let mut best_energy = energy;
            let mut rng = 0x6a09e667f3bcc909u64;
            for step in 1..=8_192 {
                if energy < best_energy || best_count == usize::MAX {
                    best_energy = energy;
                    best = selected.clone();
                    best_count = (0..count)
                        .filter(|&r| violation[r][selected[r]] > 0)
                        .count();
                }
                if energy == 0 {
                    break;
                }
                let mut choice = None;
                let mut key = (i32::MAX, u64::MAX);
                for row in 0..count {
                    for k in 0..pools[row].len() {
                        if k == selected[row] {
                            continue;
                        }
                        let delta = violation[row][k] - violation[row][selected[row]];
                        if tabu[row][k] > step && energy + delta >= best_energy {
                            continue;
                        }
                        rng ^= rng << 13;
                        rng ^= rng >> 7;
                        rng ^= rng << 17;
                        let next = (delta, rng);
                        if next < key {
                            key = next;
                            choice = Some((row, k));
                        }
                    }
                }
                let Some((row, k)) = choice else {
                    break;
                };
                let old = selected[row];
                energy += violation[row][k] - violation[row][old];
                for other in 0..count {
                    if other == row {
                        continue;
                    }
                    for q in 0..pools[other].len() {
                        violation[other][q] += i32::from(clash[other][q][row] & (1u128 << k) != 0)
                            - i32::from(clash[other][q][row] & (1u128 << old) != 0);
                    }
                }
                selected[row] = k;
                tabu[row][old] = step + 7 + (rng as usize % 13);
            }
            for (row, &i) in ids.iter().enumerate() {
                self.set(i, pools[row][best[row]].clone());
            }
        }
    }
    /// A dense component may be fenced in by otherwise valid leaders. Permit
    /// clear members of that component to move to another clear slot, opening
    /// a corridor without ever introducing a new hard conflict.
    fn open_corridors(&mut self, active: &[usize], sizes: &[Point]) {
        let mut rng = 0x510e527fade682d1u64;
        let budget = (active.len() * 80).min(1_024);
        for step in 0..budget {
            if step % active.len() == 0 && active.iter().all(|&i| !self.bad(i)) {
                break;
            }
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let i = active[rng as usize % active.len()];
            let old = self.current[i].as_ref().unwrap().clone();
            let c = candidate(
                &self.page.anchors[i],
                sizes[i],
                ((rng >> 16) % 8) as usize,
                ((rng >> 24) % 14) as usize,
            );
            if self.conflicts(
                i,
                &Candidate {
                    path: vec![],
                    ..c.clone()
                },
            ) > 0
            {
                continue;
            }
            let c = self.route_mode(i, c, false);
            if self.conflicts(i, &c) > 0 {
                continue;
            }
            let delta = self.soft_score(i, &c) - self.soft_score(i, &old);
            let temp = 20.0 * (1.0 - step as f64 / budget as f64) + 0.5;
            let uniform = ((rng >> 32) as f64 + 1.0) / (u32::MAX as f64 + 2.0);
            if self.bad(i) || delta < 0.0 || uniform < (-delta / temp).exp() {
                self.set(i, c);
            }
        }
    }
    /// Resolve a small dense group jointly: ticks can close corridors that
    /// were free when adjacent plates had no connection. Clear outsiders stay
    /// fixed. Candidate, pair and search budgets are deterministic.
    fn repair_local_group(
        &mut self,
        group: &[usize],
        sizes: &[Point],
        attempted: &mut std::collections::HashSet<usize>,
    ) {
        let mut ids = group.to_vec();
        let mut at = 0;
        while at < ids.len() && ids.len() <= 16 {
            let i = ids[at];
            let p = self.page.anchors[i].centre();
            let reach = 2.0 * sizes[i][1] + GAP;
            for j in self.markers.query(expand(bounds(&[p]), reach)) {
                if !self.pinned[j]
                    && !ids.contains(&j)
                    && self.page.anchors[i].image == self.page.anchors[j].image
                    && distance(p, self.page.anchors[j].centre()) <= reach
                {
                    ids.push(j);
                }
            }
            at += 1;
        }
        ids.sort_unstable();
        if ids.len() > 16 || ids.is_empty() {
            return;
        }
        if ids.iter().all(|i| attempted.contains(i)) {
            return;
        }
        attempted.extend(ids.iter().copied());
        let ids = &ids[..];
        let previous: Vec<_> = ids
            .iter()
            .map(|&i| self.current[i].clone().unwrap())
            .collect();
        for &i in ids {
            self.remove(i);
        }
        let mut pools = Vec::new();
        for (row, &i) in ids.iter().enumerate() {
            let mut pool = vec![previous[row].clone()];
            for ring in 0..6 {
                for dir in 0..8 {
                    pool.push(self.route_mode(
                        i,
                        candidate(&self.page.anchors[i], sizes[i], dir, ring),
                        false,
                    ));
                }
            }
            pool.retain(|c| self.plate_possible(c.rect) && self.conflicts(i, c) == 0);
            if pool.len() < 8 {
                let mut plates = Vec::new();
                for ring in 1..9 {
                    for dir in 0..8 {
                        let raw = candidate(&self.page.anchors[i], sizes[i], dir, ring);
                        let plate = Candidate {
                            path: vec![],
                            ..raw.clone()
                        };
                        let end = self.page.anchors[i].marker.box_connection(raw.rect)[1];
                        let blocked_end = self.grains.query(bounds(&[end])).into_iter().any(|j| {
                            !in_polygon(self.page.anchors[i].centre(), &self.page.grains[j])
                                && in_polygon(end, &self.page.grains[j])
                        });
                        if self.conflicts(i, &plate) == 0 && !blocked_end {
                            plates.push(raw);
                        }
                    }
                }
                plates.sort_by_cached_key(|c| {
                    cost_key(self.page.anchors[i].marker.box_distance(c.rect))
                });
                let mut added = 0;
                let mut directions = std::collections::HashSet::new();
                for raw in plates.into_iter().take(32) {
                    if directions.contains(&raw.dir) {
                        continue;
                    }
                    let dir = raw.dir;
                    let routed = self.route(i, raw);
                    if self.conflicts(i, &routed) == 0 {
                        // Keep both sides of an epoxy corridor available to
                        // the joint assignment without spending more A* visits.
                        let centre = self.page.anchors[i].centre();
                        for axes in [[true, false], [false, true], [true, true]] {
                            let mut reflected = routed.clone();
                            for axis in 0..2 {
                                if axes[axis] {
                                    reflected.rect[axis] =
                                        2.0 * centre[axis] - routed.rect[axis + 2];
                                    reflected.rect[axis + 2] =
                                        2.0 * centre[axis] - routed.rect[axis];
                                    for p in &mut reflected.path {
                                        p[axis] = 2.0 * centre[axis] - p[axis];
                                    }
                                }
                            }
                            reflected.path[0] = self.page.anchors[i].marker.edge(reflected.path[1]);
                            if reflected.dir < 8 {
                                reflected.path = self.page.anchors[i]
                                    .marker
                                    .box_connection(reflected.rect)
                                    .to_vec();
                            }
                            if self.conflicts(i, &reflected) == 0 {
                                pool.push(reflected);
                            }
                        }
                        pool.push(routed);
                        directions.insert(dir);
                        added += 1;
                        if added == 2 {
                            break;
                        }
                    }
                }
            }
            pool.sort_by_cached_key(|c| cost_key(self.soft_score(i, c)));
            let mut seen = std::collections::HashSet::new();
            pool.retain(|c| {
                seen.insert((
                    c.rect.map(f64::to_bits),
                    c.path
                        .iter()
                        .map(|p| p.map(f64::to_bits))
                        .collect::<Vec<_>>(),
                ))
            });
            pool.truncate(32);
            pools.push(pool);
        }
        let count = ids.len();
        let mut clash: Vec<Vec<Vec<u32>>> = pools
            .iter()
            .map(|p| vec![vec![0; count]; p.len()])
            .collect();
        let mut complete = pools.iter().all(|p| !p.is_empty());
        let regions: Vec<Vec<_>> = pools
            .iter()
            .map(|p| p.iter().map(Candidate::bounds).collect())
            .collect();
        'pairs: for row in 0..count {
            for other in 0..row {
                for (k, a) in pools[row].iter().enumerate() {
                    for (q, b) in pools[other].iter().enumerate() {
                        if !overlap(expand(regions[row][k], LEADER_CLEARANCE), regions[other][q]) {
                            continue;
                        }
                        if self.repair_pairs_left.get() == 0 {
                            complete = false;
                            break 'pairs;
                        }
                        self.repair_pairs_left.set(self.repair_pairs_left.get() - 1);
                        if overlap(expand(a.rect, GAP / 2.0), b.rect)
                            || path_box(&a.path, b.rect)
                            || path_box(&b.path, a.rect)
                            || paths_cross(&a.path, &b.path)
                            || leaders_close(
                                &a.path,
                                &b.path,
                                [
                                    self.page.anchors[ids[row]].centre(),
                                    self.page.anchors[ids[other]].centre(),
                                ],
                            )
                        {
                            clash[row][k][other] |= 1 << q;
                            clash[other][q][row] |= 1 << k;
                        }
                    }
                }
            }
        }
        fn solve(
            allowed: Vec<u32>,
            chosen: &mut [usize],
            clash: &[Vec<Vec<u32>>],
            left: &mut usize,
        ) -> bool {
            if *left == 0 || allowed.contains(&0) {
                return false;
            }
            *left -= 1;
            let Some(row) = (0..allowed.len())
                .filter(|&r| chosen[r] == usize::MAX)
                .min_by_key(|&r| (allowed[r].count_ones(), r))
            else {
                return true;
            };
            let mut options = allowed[row];
            while options != 0 {
                let k = options.trailing_zeros() as usize;
                options &= options - 1;
                let mut next = allowed.clone();
                for r in 0..next.len() {
                    if r != row && chosen[r] == usize::MAX {
                        next[r] &= !clash[row][k][r];
                    }
                }
                chosen[row] = k;
                if solve(next, chosen, clash, left) {
                    return true;
                }
                chosen[row] = usize::MAX;
            }
            false
        }
        let allowed = pools
            .iter()
            .map(|p| {
                if p.len() == 32 {
                    u32::MAX
                } else {
                    (1 << p.len()) - 1
                }
            })
            .collect();
        let mut selected = vec![usize::MAX; count];
        let mut nodes_left = 4096;
        let solved = complete && solve(allowed, &mut selected, &clash, &mut nodes_left);
        for (row, (&i, old)) in ids.iter().zip(previous).enumerate() {
            self.set(
                i,
                if solved {
                    pools[row][selected[row]].clone()
                } else {
                    old
                },
            );
        }
    }
    fn rails(&mut self, group: &[usize], movable: &[usize], sizes: &[Point]) {
        let mut best_bad = group.iter().filter(|&&i| self.bad(i)).count();
        let mut previous: Vec<_> = movable
            .iter()
            .map(|&i| self.current[i].clone().unwrap())
            .collect();
        let maxw = group.iter().map(|&i| sizes[i][0]).fold(0.0, f64::max);
        let maxh = group.iter().map(|&i| sizes[i][1]).fold(0.0, f64::max);
        let cluster = bounds(
            &group
                .iter()
                .map(|&i| self.page.anchors[i].centre())
                .collect::<Vec<_>>(),
        );
        for padding in [12.0, 20.0, 32.0, 48.0, 72.0, 100.0] {
            if best_bad == 0 {
                break;
            }
            for &i in movable {
                self.remove(i);
            }
            let left = (cluster[0] - padding - maxw).max(self.page.body[0] + 0.7);
            let top = (cluster[1] - padding - maxh).max(self.page.body[1] + 0.7);
            let right = (cluster[2] + padding + maxw).min(self.page.body[2] - 0.7);
            let bottom = (cluster[3] + padding + maxh).min(self.page.body[3] - 0.7);
            let mut slots = Vec::new();
            let mut x = left;
            while x + maxw <= right {
                slots.push([x, top, x + maxw, top + maxh]);
                slots.push([x, bottom - maxh, x + maxw, bottom]);
                x += maxw + 0.8;
            }
            let mut y = top + maxh + 0.8;
            while y + maxh < bottom - maxh - 0.8 {
                slots.push([left, y, left + maxw, y + maxh]);
                slots.push([right - maxw, y, right, y + maxh]);
                y += maxh + 0.8;
            }
            // Exclude hard obstacles per row (different images and label widths).
            // A finite sentinel lets Hungarian report infeasible rows safely.
            let costs: Vec<Vec<f64>> = movable
                .iter()
                .map(|&i| {
                    slots
                        .iter()
                        .map(|&slot| {
                            let rect = [
                                slot[0],
                                slot[1],
                                slot[0] + sizes[i][0],
                                slot[1] + sizes[i][1],
                            ];
                            let p = self.page.anchors[i].centre();
                            let c = Candidate {
                                rect,
                                path: vec![
                                    self.page.anchors[i].marker.edge([rect[0], rect[1]]),
                                    [rect[0], rect[1]],
                                ],
                                dir: 0,
                                cost: 0.0,
                            };
                            if self.conflicts(
                                i,
                                &Candidate {
                                    path: vec![],
                                    ..c.clone()
                                },
                            ) > 0
                                || self
                                    .markers
                                    .query(expand(rect, maxh + GAP))
                                    .iter()
                                    .any(|&j| {
                                        self.page.anchors[j].marker.hits_box(expand(rect, maxh))
                                    })
                            {
                                1e12
                            } else {
                                distance(p, centre_of(rect))
                                    + self.ambiguity_cost(i, &c)
                                    + self
                                        .grains
                                        .query(c.bounds())
                                        .iter()
                                        .filter(|&&j| {
                                            let poly = &self.page.grains[j];
                                            !in_polygon(p, poly)
                                                && c.path.windows(2).any(|s| {
                                                    poly.iter()
                                                        .zip(poly.iter().cycle().skip(1))
                                                        .take(poly.len())
                                                        .any(|(a, b)| segments(s[0], s[1], *a, *b))
                                                })
                                        })
                                        .count() as f64
                                        * 30.0
                            }
                        })
                        .collect()
                })
                .collect();
            if slots.len() >= movable.len() {
                let assignment = matching(&costs);
                if assignment
                    .iter()
                    .enumerate()
                    .all(|(row, &slot)| costs[row][slot] < 1e12)
                {
                    for variant in 0..if movable.len() >= 30 { 4 } else { 1 } {
                        for &i in movable {
                            self.remove(i);
                        }
                        for (row, &i) in movable.iter().enumerate() {
                            let r = slots[assignment[row]];
                            self.set(
                                i,
                                Candidate {
                                    rect: [r[0], r[1], r[0] + sizes[i][0], r[1] + sizes[i][1]],
                                    path: vec![],
                                    dir: 0,
                                    cost: 0.0,
                                },
                            );
                        }
                        let mut routing: Vec<_> = (0..movable.len()).collect();
                        routing.sort_by(|&a, &b| {
                            costs[a][assignment[a]]
                                .total_cmp(&costs[b][assignment[b]])
                                .then(a.cmp(&b))
                        });
                        match variant {
                            1 => routing.reverse(),
                            2 => routing.sort_by_key(|&row| movable[row]),
                            3 => routing.sort_by_key(|&row| std::cmp::Reverse(movable[row])),
                            _ => {}
                        }
                        for row in routing {
                            let i = movable[row];
                            let mut c = self.current[i].clone().unwrap();
                            c.path = vec![self.page.anchors[i].centre(), [c.rect[0], c.rect[1]]];
                            let c = self.route(i, c);
                            self.set(i, c);
                        }
                        let now = group.iter().filter(|&&i| self.bad(i)).count();
                        // Outsiders are hard obstacles, including their existing leaders.
                        let outside_clear = movable.iter().all(|&i| {
                            let c = self.current[i].as_ref().unwrap();
                            self.placed
                                .query(expand(c.bounds(), LEADER_CLEARANCE))
                                .iter()
                                .all(|&j| {
                                    movable.contains(&j)
                                        || self.current[j].as_ref().is_none_or(|b| {
                                            !overlap(expand(c.rect, GAP / 2.0), b.rect)
                                                && !path_box(&c.path, b.rect)
                                                && !path_box(&b.path, c.rect)
                                                && !paths_cross(&c.path, &b.path)
                                                && !leaders_close(
                                                    &c.path,
                                                    &b.path,
                                                    [
                                                        self.page.anchors[i].centre(),
                                                        self.page.anchors[j].centre(),
                                                    ],
                                                )
                                        })
                                })
                        });
                        if now < best_bad && outside_clear {
                            best_bad = now;
                            previous = movable
                                .iter()
                                .map(|&i| self.current[i].clone().unwrap())
                                .collect();
                        }
                        if best_bad == 0 {
                            break;
                        }
                    }
                }
            }
            for (&i, c) in movable.iter().zip(&previous) {
                self.set(i, c.clone());
            }
        }
    }
    fn far_slots(&mut self, movable: &[usize], sizes: &[Point]) {
        for &i in movable {
            if !self.bad(i) {
                continue;
            }
            let mut far = candidates(self.page, &self.page.anchors[i], sizes[i]);
            far.sort_by_cached_key(|c| cost_key(self.soft_score(i, c)));
            let mut attempts = 0;
            for c in far {
                if self.conflicts(
                    i,
                    &Candidate {
                        path: vec![],
                        ..c.clone()
                    },
                ) > 0
                {
                    continue;
                }
                attempts += 1;
                if attempts > if movable.len() < 8 { 16 } else { 8 } {
                    break;
                }
                let c = self.route_mode(i, c, attempts > 4);
                if self.conflicts(i, &c) == 0 {
                    self.set(i, c);
                    break;
                }
            }
        }
    }
    /// Resolve sub-millimetre slivers left by the coarse local slot lattice.
    /// Move a conflicting pair together; pins and clear outsiders stay fixed.
    /// Spend available room on marker clearance, retaining a short straight
    /// route. Small manual moves should not immediately cover a marker.
    fn relax_short_clearance(&mut self, pinned: &[bool]) {
        for (i, &pin) in pinned.iter().enumerate() {
            let old = self.current[i].as_ref().unwrap().clone();
            if pin
                || old.dir >= 8
                || old.path.len() != 2
                || self.bad(i)
                || (old.cost < 1.0
                    && old.path == self.page.anchors[i].marker.box_connection(old.rect))
            {
                continue;
            }
            let p = self.page.anchors[i].centre();
            let end = *old.path.last().unwrap();
            let d = distance(p, end);
            if d < 1e-9 {
                continue;
            }
            let height = old.rect[3] - old.rect[1];
            let mut clearance = distance(old.path[0], end);
            if clearance >= height {
                continue;
            }
            let mut best = old.clone();
            for shift in [0.2, 0.4, 0.6, 0.65, 0.7, 0.75, 0.8, 1.0, 1.2, 1.6, 2.0] {
                let dx = shift * (end[0] - p[0]) / d;
                let dy = shift * (end[1] - p[1]) / d;
                let rect = [
                    old.rect[0] + dx,
                    old.rect[1] + dy,
                    old.rect[2] + dx,
                    old.rect[3] + dy,
                ];
                if !inside(self.page.body, rect) {
                    continue;
                }
                let c = self.route_mode(
                    i,
                    Candidate {
                        rect,
                        ..old.clone()
                    },
                    false,
                );
                let gap = distance(c.path[0], *c.path.last().unwrap());
                if gap > clearance
                    && gap <= height
                    && self.conflicts(i, &c) == 0
                    && self.ambiguity_cost(i, &c) <= self.ambiguity_cost(i, &old)
                {
                    clearance = gap;
                    best = c;
                }
            }
            self.set(i, best);
        }
    }
    fn set(&mut self, i: usize, mut c: Candidate) {
        if c.path.is_empty()
            && inside(self.page.body, c.rect)
            && self.page.anchors[i].marker.box_distance(c.rect) > TICK_GAP
        {
            c.path = self.page.anchors[i].marker.box_connection(c.rect).to_vec();
        }
        self.generation += 1;
        if let Some(old) = &self.current[i] {
            self.placed.remove(i, expand(old.bounds(), GAP));
        }
        self.placed.insert(i, expand(c.bounds(), GAP));
        self.current[i] = Some(c);
    }
    /// A nearest-point connector can run through the next marker even when
    /// a corner of the same plate is reachable through the diagonal corridor.
    fn straight_alternatives(&self, i: usize, c: &Candidate) -> Vec<Candidate> {
        let mut out = Vec::new();
        let r = c.rect;
        let p = self.page.anchors[i].centre();
        for end in corners(r).into_iter().chain([
            [(r[0] + r[2]) / 2.0, r[1]],
            [(r[0] + r[2]) / 2.0, r[3]],
            [r[0], (r[1] + r[3]) / 2.0],
            [r[2], (r[1] + r[3]) / 2.0],
        ]) {
            // Connect only to the facing edge, never through the plate.
            if segment_rect(p, end, [r[0] + 1e-6, r[1] + 1e-6, r[2] - 1e-6, r[3] - 1e-6]) {
                continue;
            }
            let candidate = Candidate {
                path: vec![self.page.anchors[i].marker.edge(end), end],
                ..c.clone()
            };
            if self.static_conflicts(i, &candidate) == 0 {
                out.push(candidate);
            }
        }
        out
    }
    fn route(&self, i: usize, c: Candidate) -> Candidate {
        self.route_mode(i, c, true)
    }
    fn route_mode(&self, i: usize, c: Candidate, detour: bool) -> Candidate {
        if c.path.is_empty() {
            return c;
        }
        // Straight trials need only two nearest points. Caching thousands of
        // one-use grid slots costs more than constructing this geometry.
        if !detour {
            return Candidate {
                path: self.page.anchors[i].marker.box_connection(c.rect).to_vec(),
                ..c
            };
        }
        let key = (i, c.rect.map(f64::to_bits), detour);
        if let Some((generation, cached)) = self.route_cache.borrow().get(&key)
            && (*generation == self.generation || self.conflicts(i, cached) == 0)
        {
            return cached.clone();
        }
        let a = &self.page.anchors[i];
        let mut best = Candidate {
            path: a.marker.box_connection(c.rect).to_vec(),
            ..c
        };
        if self.routes_left.get() == 0 || self.group_routes_left.get() == 0 {
            return best;
        }
        if detour {
            self.routes_left.set(self.routes_left.get() - 1);
            self.group_routes_left.set(self.group_routes_left.get() - 1);
        }
        // No local elbows or curves: a longer connection belongs on a rail.
        // If rails are unavailable, only bounded A* may supply a last resort.
        if detour
            && self.conflicts(i, &best) > 0
            && self.conflicts(
                i,
                &Candidate {
                    path: vec![],
                    ..best.clone()
                },
            ) == 0
            && let Some(path) = self.detour(i, &best)
        {
            best.path = path;
            best.dir = ASTAR_DIR;
        }
        self.route_cache
            .borrow_mut()
            .insert(key, (self.generation, best.clone()));
        best
    }
    /// Grid A* is a fallback for a corridor that simple elbows cannot reach.
    /// Test whole segments, including existing flattened leaders; diagonal moves
    /// cannot cut a marker corner or step across a thin leader between cells.
    fn detour(&self, i: usize, c: &Candidate) -> Option<Vec<Point>> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let centre = self.page.anchors[i].centre();
        let target = [
            centre[0].clamp(c.rect[0], c.rect[2]),
            centre[1].clamp(c.rect[1], c.rect[3]),
        ];
        let area = expand(bounds(&[centre, target]), 40.0);
        let area = [
            area[0].max(self.page.body[0]),
            area[1].max(self.page.body[1]),
            area[2].min(self.page.body[2]),
            area[3].min(self.page.body[3]),
        ];
        let step = 0.4;
        let w = ((area[2] - area[0]) / step).ceil().max(1.0) as usize + 1;
        let h = ((area[3] - area[1]) / step).ceil().max(1.0) as usize + 1;
        if w * h > 400_000 {
            return None;
        }
        let coordinate = |id: usize| {
            [
                area[0] + (id % w) as f64 * step,
                area[1] + (id / w) as f64 * step,
            ]
        };
        let cell = |p: Point| {
            let x = ((p[0] - area[0]) / step).round().clamp(0.0, (w - 1) as f64) as usize;
            let y = ((p[1] - area[1]) / step).round().clamp(0.0, (h - 1) as f64) as usize;
            y * w + x
        };
        let start = cell(centre);
        let end = cell(target);
        let clear = |a: Point, b: Point| {
            if !contains(self.page.body, b) {
                return false;
            }
            let region = expand(bounds(&[a, b]), LEADER_CLEARANCE);
            let path = [a, b];
            if self
                .markers
                .query(region)
                .iter()
                .any(|&j| j != i && self.page.anchors[j].marker.hits_path(&path))
            {
                return false;
            }
            if self
                .fixed
                .query(region)
                .iter()
                .any(|&j| path_box(&path, self.page.hard[j]))
            {
                return false;
            }
            if self.grains.query(region).iter().any(|&j| {
                let poly = &self.page.grains[j];
                !in_polygon(centre, poly)
                    && (in_polygon(a, poly)
                        || in_polygon(b, poly)
                        || poly
                            .iter()
                            .zip(poly.iter().cycle().skip(1))
                            .take(poly.len())
                            .any(|(c, d)| segments(a, b, *c, *d)))
            }) {
                return false;
            }
            !self.placed.query(region).iter().any(|&j| {
                j != i
                    && self.current[j].as_ref().is_some_and(|l| {
                        path_box(&path, l.rect)
                            || paths_cross(&path, &l.path)
                            || leaders_close(
                                &path,
                                &l.path,
                                [centre, self.page.anchors[j].centre()],
                            )
                    })
            })
        };
        // A small visibility graph gives A* useful waypoints before the grid.
        // The same segment checks and two-bend limit apply to both searches.
        let mut waypoints = Vec::new();
        for radius in [2.25, 4.5, 9.0] {
            for d in DIRECTIONS {
                waypoints.push([centre[0] + radius * d[0], centre[1] + radius * d[1]]);
                waypoints.push([target[0] + radius * d[0], target[1] + radius * d[1]]);
            }
        }
        for step in 0..6 {
            let d = if step % 2 == 0 { 1.0 } else { -1.0 } * (1 + step / 2) as f64 * 2.8;
            waypoints.push([centre[0], target[1] + d]);
            waypoints.push([target[0] + d, centre[1]]);
        }
        // Exact grain corners expose narrow epoxy corridors between adjacent
        // grains that the fixed-radius waypoints and 0.4 mm grid can miss.
        for j in self.grains.query(expand(bounds(&[centre, target]), 10.0)) {
            if !in_polygon(centre, &self.page.grains[j]) {
                waypoints.extend(corners(expand(bounds(&self.page.grains[j]), 0.12)));
            }
        }
        for j in self.placed.query(expand(bounds(&[centre, target]), 10.0)) {
            if j != i
                && let Some(l) = &self.current[j]
            {
                waypoints.extend(corners(expand(l.rect, LEADER_CLEARANCE)));
            }
        }
        waypoints.sort_by(|&a, &b| {
            (distance(centre, a) + distance(a, target))
                .total_cmp(&(distance(centre, b) + distance(b, target)))
                .then(a[0].total_cmp(&b[0]))
                .then(a[1].total_cmp(&b[1]))
        });
        waypoints.dedup();
        waypoints.truncate(48);
        waypoints.insert(0, centre);
        let mut frontier = BinaryHeap::from([Reverse((0u32, 0u32, 0usize, 0usize))]);
        let mut costs = vec![[u32::MAX; 3]; waypoints.len()];
        let mut prev = vec![[usize::MAX; 3]; waypoints.len()];
        costs[0][0] = 0;
        let mut checks = 0;
        let mut visible = HashMap::new();
        while let Some(Reverse((_, g, at, depth))) = frontier.pop() {
            if checks >= 512 || self.astar_left.get() == 0 {
                break;
            }
            if g != costs[at][depth] {
                continue;
            }
            checks += 1;
            self.astar_left.set(self.astar_left.get() - 1);
            if clear(waypoints[at], target) {
                let mut path = vec![target, waypoints[at]];
                let mut cursor = at;
                for d in (1..=depth).rev() {
                    cursor = prev[cursor][d];
                    path.push(waypoints[cursor]);
                }
                path.reverse();
                path[0] = self.page.anchors[i].marker.edge(path[1]);
                let routed = Candidate {
                    path: path.clone(),
                    dir: ASTAR_DIR,
                    ..c.clone()
                };
                if self.conflicts(i, &routed) == 0 {
                    return Some(path);
                }
            }
            if depth == 2 {
                continue;
            }
            for next in 1..waypoints.len() {
                let ng = g + (distance(waypoints[at], waypoints[next]) * 1000.0) as u32;
                if ng >= costs[next][depth + 1] {
                    continue;
                }
                if checks >= 512 || self.astar_left.get() == 0 {
                    break;
                }
                let key = (at.min(next), at.max(next));
                let reachable = *visible.entry(key).or_insert_with(|| {
                    checks += 1;
                    self.astar_left.set(self.astar_left.get() - 1);
                    clear(waypoints[at], waypoints[next])
                });
                if !reachable {
                    continue;
                }
                costs[next][depth + 1] = ng;
                prev[next][depth + 1] = at;
                let h = (distance(waypoints[next], target) * 1000.0) as u32;
                frontier.push(Reverse((ng + h, ng, next, depth + 1)));
            }
        }
        let heuristic =
            |id: usize| ((distance(coordinate(id), coordinate(end)) / step) * 1000.0) as u32;
        let mut cost = vec![u32::MAX; w * h];
        let mut previous = vec![usize::MAX; w * h];
        let mut heap = BinaryHeap::new();
        cost[start] = 0;
        heap.push(Reverse((heuristic(start), 0u32, start)));
        let mut visits = 0;
        while let Some(Reverse((_, g, at))) = heap.pop() {
            if g != cost[at] {
                continue;
            }
            if clear(coordinate(at), target) {
                let mut path = vec![target];
                let mut at = at;
                while at != start {
                    path.push(coordinate(at));
                    at = previous[at];
                }
                path.push(centre);
                path.reverse();
                // Farthest visible waypoint shortcut keeps a detour compact.
                let mut simple = vec![centre];
                let mut at = 0;
                while at + 1 < path.len() {
                    let next = (at + 1..path.len())
                        .rev()
                        .find(|&j| clear(path[at], path[j]))
                        .unwrap_or(at + 1);
                    simple.push(path[next]);
                    at = next;
                }
                if simple.len() > 1 {
                    simple[0] = self.page.anchors[i].marker.edge(simple[1]);
                }
                let candidate = Candidate {
                    path: simple.clone(),
                    dir: ASTAR_DIR,
                    ..c.clone()
                };
                return (simple.len() <= 4 && self.conflicts(i, &candidate) == 0).then_some(simple);
            }
            visits += 1;
            if self.astar_left.get() == 0 || visits > 512 {
                break;
            }
            self.astar_left.set(self.astar_left.get() - 1);
            let x = at % w;
            let y = at / w;
            let a = coordinate(at);
            for (dx, dy) in [
                (1i32, 0i32),
                (0, 1),
                (-1, 0),
                (0, -1),
                (1, 1),
                (-1, 1),
                (-1, -1),
                (1, -1),
            ] {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                let next = ny as usize * w + nx as usize;
                let ng = g + if dx == 0 || dy == 0 { 1000 } else { 1415 };
                if ng >= cost[next] || !clear(a, coordinate(next)) {
                    continue;
                }
                cost[next] = ng;
                previous[next] = at;
                heap.push(Reverse((ng + heuristic(next), ng, next)));
            }
        }
        None
    }
}
const DIRECTIONS: [Point; 8] = [
    [1.0, -1.0],
    [1.0, 1.0],
    [-1.0, -1.0],
    [-1.0, 1.0],
    [1.0, 0.0],
    [-1.0, 0.0],
    [0.0, -1.0],
    [0.0, 1.0],
];
fn candidate(a: &Anchor, size: Point, dir: usize, ring: usize) -> Candidate {
    let b = a.marker.bounds();
    let p = a.centre();
    let d = DIRECTIONS[dir];
    // Adjacent plates stay close to the marker; diagonal clearance still
    // includes its curved edge. More distant rings keep the full search range.
    // Pins remain exact, even if deliberately invalid.
    let gap = if ring == 0 {
        if dir < 4 {
            size[1].max(0.7)
        } else {
            size[1].clamp(0.7, 2.9)
        }
    } else {
        0.7 + (ring - 1) as f64 * size[1] * 0.5
    };
    let x = if d[0] > 0.0 {
        b[2] + gap
    } else if d[0] < 0.0 {
        b[0] - gap - size[0]
    } else {
        p[0] - size[0] / 2.0
    };
    let y = if d[1] > 0.0 {
        b[3] + gap
    } else if d[1] < 0.0 {
        b[1] - gap - size[1]
    } else {
        p[1] - size[1] / 2.0
    };
    let rect = [x, y, x + size[0], y + size[1]];
    Candidate {
        rect,
        path: if ring == 0 {
            let connection = a.marker.box_connection(rect);
            if distance(connection[0], connection[1]) > TICK_GAP {
                connection.to_vec()
            } else {
                vec![]
            }
        } else {
            vec![p, [x, y]]
        },
        dir,
        cost: ring as f64 * size[1]
            + if ring == 0 {
                (dir % 4) as f64 * 0.12
            } else {
                dir as f64 * 0.12
            }
            + if ring > 0 { 3.0 } else { 0.0 },
    }
}
/// Exact decoration-edge slots fill corridors missed by a font-height lattice.
/// Keep room for a small manual move, as with the ordinary adjacent candidates.
fn edge_slots(page: &Page, a: &Anchor, size: Point) -> Vec<Rect> {
    let p = a.centre();
    let [w, h] = size;
    let mut ys = Vec::new();
    for &r in &page.hard {
        if overlap(expand(bounds(&[p]), w + 4.0 * h), r) {
            ys.extend([r[1] - GAP - 0.02 - h, r[3] + GAP + 0.02]);
        }
    }
    ys.retain(|y| (*y - p[1]).abs() <= 4.0 * h);
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut out = Vec::new();
    for y in ys {
        for dx in -12..=16 {
            let x = p[0] + dx as f64 * h * 0.5;
            let r = [x, y, x + w, y + h];
            if inside(page.body, r)
                && !page
                    .anchors
                    .iter()
                    .any(|a| a.marker.hits_box(expand(r, h * 0.5)))
            {
                out.push(r);
            }
        }
    }
    out
}
fn candidates(page: &Page, a: &Anchor, size: Point) -> Vec<Candidate> {
    let mut v = Vec::new();
    for ring in 0..6 {
        for dir in 0..8 {
            v.push(candidate(a, size, dir, ring));
        }
    }
    // Free slots along the image edge and adjacent white paper, searched from
    // the anchor outward. A label may extend outside its own image, not another.
    let b = bounds(&page.images[a.image]);
    let p = a.centre();
    for ring in 0..8 {
        for sign in [-1.0, 1.0] {
            let d = ring as f64 * (size[1] + GAP) * sign;
            for (dir, x, y) in [
                (4, b[2] + 0.8, p[1] + d - size[1] / 2.0),
                (5, b[0] - 0.8 - size[0], p[1] + d - size[1] / 2.0),
                (6, p[0] + d - size[0] / 2.0, b[1] - 0.8 - size[1]),
                (7, p[0] + d - size[0] / 2.0, b[3] + 0.8),
            ] {
                let rect = [x, y, x + size[0], y + size[1]];
                v.push(Candidate {
                    rect,
                    path: vec![p, [x, y]],
                    dir,
                    cost: distance(p, [x + size[0] / 2.0, y + size[1] / 2.0]) + 4.0,
                });
            }
        }
    }
    // Full outward square rings of free slots, rather than only eight rays.
    // Shared paper-space lattice makes adjacent far slots pack without slivers.
    let origin = [page.body[0] + 0.6, page.body[1] + 0.6];
    let sx = size[0] + 0.8;
    let sy = size[1] + 0.8;
    let cx = ((p[0] - origin[0]) / sx).round() as i32;
    let cy = ((p[1] - origin[1]) / sy).round() as i32;
    for ring in 1i32..=6 {
        for dx in -ring..=ring {
            for dy in -ring..=ring {
                if dx.abs() != ring && dy.abs() != ring {
                    continue;
                }
                let x = origin[0] + (cx + dx) as f64 * sx;
                let y = origin[1] + (cy + dy) as f64 * sy;
                let rect = [x, y, x + size[0], y + size[1]];
                if !inside(page.body, rect) {
                    continue;
                }
                v.push(Candidate {
                    rect,
                    path: vec![p, [x, y]],
                    dir: if dx >= 0 { 4 } else { 5 },
                    cost: distance(p, [x, y]) + 4.0,
                });
            }
        }
    }
    v
}

/// Rectangular Hungarian assignment, canonical row/slot order resolves ties.
fn matching(cost: &[Vec<f64>]) -> Vec<usize> {
    let n = cost.len();
    let m = cost[0].len();
    let mut u = vec![0.0; n + 1];
    let mut v = vec![0.0; m + 1];
    let mut p = vec![0usize; m + 1];
    let mut way = vec![0; m + 1];
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0;
        let mut min = vec![f64::INFINITY; m + 1];
        let mut used = vec![false; m + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = f64::INFINITY;
            let mut j1 = 0;
            for j in 1..=m {
                if !used[j] {
                    let c = cost[i0 - 1][j - 1] - u[i0] - v[j];
                    if c < min[j] {
                        min[j] = c;
                        way[j] = j0;
                    }
                    if min[j] < delta {
                        delta = min[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=m {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    min[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut result = vec![0; n];
    for j in 1..=m {
        if p[j] > 0 {
            result[p[j] - 1] = j - 1;
        }
    }
    result
}

/// Bound cubic matching work per local group, never by the page's spot count.
fn split_group(page: &Page, mut group: Vec<usize>, parts: &mut Vec<Vec<usize>>) {
    if group.len() <= 80 {
        group.sort_unstable();
        parts.push(group);
        return;
    }
    let hull = bounds(
        &group
            .iter()
            .map(|&i| page.anchors[i].centre())
            .collect::<Vec<_>>(),
    );
    let axis = usize::from(hull[3] - hull[1] > hull[2] - hull[0]);
    group.sort_by(|&a, &b| {
        page.anchors[a].centre()[axis]
            .total_cmp(&page.anchors[b].centre()[axis])
            .then(a.cmp(&b))
    });
    let other = group.split_off(group.len() / 2);
    split_group(page, group, parts);
    split_group(page, other, parts);
}

/// Sizes are tried in descending order; actual grain polygons are already in
/// paper mm, so their equivalent-area diameter includes the print transform.
const AUTO_SIZES: [f64; 4] = [3.2, 2.9, 2.6, 2.4];
fn auto_sizes(page: &Page) -> Vec<f64> {
    let mut diameters: Vec<_> = page
        .grains
        .iter()
        .filter_map(|poly| {
            let area = poly
                .iter()
                .zip(poly.iter().cycle().skip(1))
                .take(poly.len())
                .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
                .sum::<f64>()
                .abs()
                / 2.0;
            (area > 0.0).then(|| (4.0 * area / std::f64::consts::PI).sqrt())
        })
        .collect();
    diameters.sort_by(f64::total_cmp);
    let cap = if diameters.is_empty() {
        3.2
    } else {
        // Include plate padding in the height cap; do not exceed 1.5 grains.
        let median = (diameters[(diameters.len() - 1) / 2] + diameters[diameters.len() / 2]) / 2.0;
        ((1.5 * median - 0.5) * font().units_per_em() as f64
            / (font().ascender() - font().descender()) as f64)
            .clamp(2.4, 3.2)
    };
    let mut sizes: Vec<_> = AUTO_SIZES.into_iter().filter(|&s| s <= cap).collect();
    if sizes.is_empty() {
        sizes.push(cap);
    }
    sizes
}
pub fn place(mut page: Page, options: &Options) -> (Page, Report) {
    page.anchors.sort_by(|a, b| {
        a.spot_id
            .cmp(&b.spot_id)
            .then(a.occurrence.cmp(&b.occurrence))
    });
    let mut search = new_search(&page);
    let sizes = if options.size_mm == 0.0 {
        auto_sizes(&page)
    } else {
        vec![options.size()]
    };
    let qualifies = |r: &Report| {
        r.counts.still_overlapping == 0
            && r.counts.ambiguous == 0
            && in_image_leaders(&page, r) * 100 <= r.labels.len() * 15
    };
    let mut report = None;
    // Reuse static grids for descending short-slot trials. The full bounded
    // rail/repair search runs once, at the floor if no trial qualifies.
    for size in sizes {
        let trial = place_size(&mut search, options, size, options.size_mm == 0.0);
        let done = qualifies(&trial);
        report = Some(trial);
        if done {
            break;
        }
    }
    let mut report = report.unwrap();
    if options.size_mm == 0.0 && (!qualifies(&report) || report.counts.context_omitted > 0) {
        let refined = place_size(&mut search, options, report.size_mm, false);
        let score = |r: &Report| {
            (
                r.counts.still_overlapping,
                r.counts.ambiguous,
                r.counts.context_omitted,
                in_image_leaders(&page, r),
                r.counts.with_ticks + r.counts.with_leaders,
                cost_key(
                    r.labels
                        .iter()
                        .flat_map(|l| l.leader.windows(2))
                        .map(|p| distance(p[0], p[1]))
                        .sum(),
                ),
            )
        };
        if score(&refined) <= score(&report) {
            report = refined;
        }
    }
    (page, report)
}
fn in_image_leaders(page: &Page, report: &Report) -> usize {
    report
        .labels
        .iter()
        .filter(|l| {
            !l.leader.is_empty()
                && !(l.connector_tick
                    && l.leader
                        .windows(2)
                        .map(|s| distance(s[0], s[1]))
                        .sum::<f64>()
                        < 3.0)
                && page.images.iter().any(|p| polygon_rect(p, l.box_mm))
        })
        .count()
}
fn new_search(page: &Page) -> Search<'_> {
    let n = page.anchors.len();
    let mut search = Search {
        page,
        markers: Grid::default(),
        fixed: Grid::default(),
        grains: Grid::default(),
        images: Grid::default(),
        placed: Grid::default(),
        current: vec![None; n],
        pinned: vec![false; n],
        generation: 0,
        routes_left: Cell::new(PAGE_ROUTE_LIMIT),
        group_routes_left: Cell::new(GROUP_ROUTE_LIMIT),
        astar_left: Cell::new(PAGE_ASTAR_VISITS),
        repair_calls_left: Cell::new(PAGE_REPAIR_CALLS),
        repair_pairs_left: Cell::new(PAGE_REPAIR_PAIRS),
        route_cache: RefCell::new(HashMap::new()),
    };
    for (i, a) in page.anchors.iter().enumerate() {
        search.markers.insert(i, expand(a.marker.bounds(), GAP));
    }
    for (i, r) in page.hard.iter().enumerate() {
        search.fixed.insert(i, expand(*r, GAP));
    }
    for (i, polygon) in page.grains.iter().enumerate() {
        if !polygon.is_empty() {
            search.grains.insert(i, bounds(polygon));
        }
    }
    for (i, polygon) in page.images.iter().enumerate() {
        search.images.insert(i, bounds(polygon));
    }
    search
}
fn place_size(search: &mut Search<'_>, options: &Options, size: f64, probe: bool) -> Report {
    let page = search.page;
    let n = page.anchors.len();
    search.current.fill(None);
    search.pinned.fill(false);
    search.placed.cells.clear();
    search.route_cache.borrow_mut().clear();
    search.generation = 0;
    search.routes_left.set(PAGE_ROUTE_LIMIT);
    search.group_routes_left.set(GROUP_ROUTE_LIMIT);
    search.astar_left.set(PAGE_ASTAR_VISITS);
    search.repair_calls_left.set(PAGE_REPAIR_CALLS);
    search.repair_pairs_left.set(PAGE_REPAIR_PAIRS);
    let sizes: Vec<_> = page
        .anchors
        .iter()
        .map(|a| text_size(&a.name, size))
        .collect();
    let mut pinned = vec![false; n];
    for (i, a) in page.anchors.iter().enumerate() {
        if let Some(offset) = options
            .pins
            .get(&a.key())
            .filter(|p| p.iter().all(|v| v.is_finite()))
        {
            pinned[i] = true;
            search.pinned[i] = true;
            let p = a.centre();
            let x = p[0] + offset[0];
            let y = p[1] + offset[1];
            let c = Candidate {
                rect: [x, y, x + sizes[i][0], y + sizes[i][1]],
                path: vec![p, [x, y]],
                dir: 0,
                cost: 0.0,
            };
            let c = search.route(i, c);
            search.set(i, c);
        }
    }
    let mut order: Vec<_> = (0..n)
        .filter(|i| !pinned[*i])
        .map(|i| {
            let free = (0..8)
                .filter(|&d| search.conflicts(i, &candidate(&page.anchors[i], sizes[i], d, 0)) == 0)
                .count();
            (free, i)
        })
        .collect();
    let centre = if n == 0 {
        [0.0, 0.0]
    } else {
        let sum = page.anchors.iter().fold([0.0, 0.0], |s, a| {
            [s[0] + a.centre()[0], s[1] + a.centre()[1]]
        });
        [sum[0] / n as f64, sum[1] / n as f64]
    };
    order.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(
                distance(page.anchors[a.1].centre(), centre)
                    .total_cmp(&distance(page.anchors[b.1].centre(), centre)),
            )
            .then(a.1.cmp(&b.1))
    });
    for (_, i) in &order {
        let i = *i;
        let a = &page.anchors[i];
        let mut best = candidate(a, sizes[i], 0, 0);
        let mut score = f64::INFINITY;
        for d in 0..8 {
            let c = candidate(a, sizes[i], d, 0);
            let s = search.score(i, &c);
            if s < score {
                score = s;
                best = c;
            }
        }
        search.set(i, best);
    }
    // A short first refinement touches only conflicts or ambiguous labels.
    let active: Vec<_> = (0..n)
        .filter(|&i| {
            !pinned[i]
                && (search.bad(i) || search.ambiguous(i, search.current[i].as_ref().unwrap()))
        })
        .collect();
    if !probe {
        search.anneal(&active, &sizes, 8);
    }
    // Remove provisional conflicting plates together so they cannot fence one
    // another out of the short straight slots before boundary assignment.
    let unresolved: Vec<_> = order
        .iter()
        .filter(|&&(_, i)| {
            search.bad(i)
                || search.ambiguous(i, search.current[i].as_ref().unwrap())
                || search.covers_middle(i, search.current[i].as_ref().unwrap())
        })
        .map(|&(_, i)| (i, search.current[i].clone().unwrap()))
        .collect();
    for &(i, _) in &unresolved {
        search.remove(i);
    }
    for &(i, _) in &unresolved {
        let mut best: Option<(f64, Candidate)> = None;
        for ring in 0..=if probe { 2 } else { 4 } {
            for dir in 0..8 {
                let raw = candidate(&page.anchors[i], sizes[i], dir, ring);
                if !search.plate_possible(raw.rect) {
                    continue;
                }
                let c = search.route_mode(i, raw, false);

                if search.conflicts(i, &c) == 0
                    && !(c.path.is_empty() && search.covers_middle(i, &c))
                {
                    let score = search.soft_score(i, &c);
                    if best.as_ref().is_none_or(|(old, _)| score < *old) {
                        best = Some((score, c));
                    }
                }
            }
        }
        let p = page.anchors[i].centre();
        let [w, h] = sizes[i];
        if !probe {
            for dx in -((2.0 * w / h).ceil() as i32 + 4)..=4 {
                for dy in -6..=4 {
                    let x = p[0] + dx as f64 * h * 0.5;
                    let y = p[1] + dy as f64 * h * 0.5;
                    let raw = Candidate {
                        rect: [x, y, x + w, y + h],
                        path: vec![p, [x, y]],
                        dir: 0,
                        cost: 3.0,
                    };
                    if !search.plate_possible(raw.rect) {
                        continue;
                    }
                    let c = search.route_mode(i, raw, false);
                    if search.conflicts(i, &c) == 0 {
                        let score = search.soft_score(i, &c);
                        if best.as_ref().is_none_or(|(old, _)| score < *old) {
                            best = Some((score, c));
                        }
                    }
                }
            }
        }
        if let Some((_, c)) = best {
            search.set(i, c);
        }
    }
    if probe {
        for (i, old) in unresolved {
            if search.current[i].is_none() {
                search.set(i, old);
            }
        }
        return placement_report(search, size);
    }
    let mut pending = Vec::new();
    for (i, old) in unresolved {
        if search.current[i].is_none() {
            // Keep an unresolved provisional box out of the clear local slots
            // until the simultaneous boundary assignment has had its turn.
            let mut placeholder = old.clone();
            placeholder.rect = [-100.0, -100.0, -99.0, -99.0];
            placeholder.path.clear();
            search.set(i, placeholder);
            pending.push((i, old));
        }
    }
    search.group_routes_left.set(GROUP_ROUTE_LIMIT);
    search.boundary_rails(&sizes, &pinned);
    for (i, old) in pending {
        if search.current[i].as_ref().unwrap().rect[0] == -100.0 {
            search.set(i, old);
        }
    }

    search.repair_boundary(&sizes, &pinned, false);
    search.repair_boundary(&sizes, &pinned, true);

    let groups = search.conflict_groups(&sizes);
    let mut local_attempted = std::collections::HashSet::new();
    for group in groups {
        let mut parts = Vec::new();
        split_group(page, group, &mut parts);
        parts.sort_by_key(|g| *g.iter().min().unwrap());
        for part in parts {
            let movable: Vec<_> = part.iter().copied().filter(|&i| !pinned[i]).collect();
            if movable.is_empty() || !part.iter().any(|&i| search.bad(i)) {
                continue;
            }
            search.group_routes_left.set(GROUP_ROUTE_LIMIT);
            for &i in &movable {
                if search.bad(i) {
                    search.repair_local_group(&[i], &sizes, &mut local_attempted);
                }
            }
            if !part.iter().any(|&i| search.bad(i)) {
                continue;
            }
            search.rails(&part, &movable, &sizes);
            if part.iter().any(|&i| search.bad(i)) {
                search.open_corridors(&movable, &sizes);
            }
            if part.iter().any(|&i| search.bad(i)) {
                search.far_slots(&movable, &sizes);
                let active: Vec<_> = movable.iter().copied().filter(|&i| search.bad(i)).collect();
                search.anneal(&active, &sizes, 36);
            }
        }
    }
    // A new pin can close a corridor in a small otherwise settled component.
    if pinned.iter().any(|p| *p) {
        let bad: Vec<_> = (0..n).filter(|&i| !pinned[i] && search.bad(i)).collect();
        let reach = sizes.iter().map(|s| s[0] + 3.0 * s[1]).fold(0.0, f64::max);
        let nearby: Vec<_> = (0..n)
            .filter(|&i| {
                !pinned[i]
                    && bad.iter().any(|&j| {
                        page.anchors[i].image == page.anchors[j].image
                            && distance(page.anchors[i].centre(), page.anchors[j].centre()) <= reach
                    })
            })
            .collect();
        if !nearby.is_empty() {
            search.open_corridors(&nearby, &sizes);
        }
    }
    // A failed group can leave several mutually blocking provisional slots.
    // Revisit only those labels after the other groups have settled.
    for _ in 0..3 {
        let active: Vec<_> = (0..n).filter(|&i| !pinned[i] && search.bad(i)).collect();
        if active.is_empty() {
            break;
        }
        search.far_slots(&active, &sizes);
        search.anneal(&active, &sizes, 36);
    }

    if (0..n).any(|i| search.bad(i)) {
        for (i, &pin) in pinned.iter().enumerate() {
            if pin {
                continue;
            }
            let b = bounds(&page.images[page.anchors[i].image]);
            let axis = usize::from(b[3] - b[1] > b[2] - b[0]);
            let cross = 1 - axis;
            let mut c = search.current[i].clone().unwrap();
            if c.rect[cross + 2] < b[cross] || c.rect[cross] > b[cross + 2] {
                c.dir = 8 + usize::from(c.rect[cross] > b[cross + 2]) + axis * 2;
                search.set(i, c);
            }
        }
        search.reflow_rails(&sizes, &pinned);
        if (0..n).any(|i| search.bad(i)) {
            search.boundary_rails(&sizes, &pinned);
        }
    }

    // Deterministic 2-opt exchanges of margin slots, including full flattened
    // leader checks. Never displace pins or accept a new hard conflict.
    for _ in 0..4 {
        let mut changed = false;
        for i in 0..n {
            if pinned[i] {
                continue;
            }
            let a = search.current[i].as_ref().unwrap().clone();
            if a.path.is_empty() {
                continue;
            }
            for j in search.placed.query(a.bounds()) {
                if j <= i || pinned[j] {
                    continue;
                }
                let b = search.current[j].as_ref().unwrap().clone();
                if !paths_cross(&a.path, &b.path) {
                    continue;
                }
                let mut ca = b.clone();
                ca.rect[2] = ca.rect[0] + sizes[i][0];
                ca.rect[3] = ca.rect[1] + sizes[i][1];
                let mut cb = a.clone();
                cb.rect[2] = cb.rect[0] + sizes[j][0];
                cb.rect[3] = cb.rect[1] + sizes[j][1];
                let before = search.conflicts(i, &a) + search.conflicts(j, &b);
                search.placed.remove(i, expand(a.bounds(), GAP));
                search.placed.remove(j, expand(b.bounds(), GAP));
                ca = search.route(i, ca);
                cb = search.route(j, cb);
                let after = search.conflicts(i, &ca)
                    + search.conflicts(j, &cb)
                    + usize::from(
                        paths_cross(&ca.path, &cb.path)
                            || leaders_close(
                                &ca.path,
                                &cb.path,
                                [page.anchors[i].centre(), page.anchors[j].centre()],
                            )
                            || path_box(&ca.path, cb.rect)
                            || path_box(&cb.path, ca.rect)
                            || overlap(ca.rect, cb.rect),
                    );
                if after < before {
                    search.set(i, ca);
                    search.set(j, cb);
                    changed = true;
                } else {
                    search.set(i, a.clone());
                    search.set(j, b);
                }
            }
        }
        if !changed {
            break;
        }
    }

    search.repair_boundary(&sizes, &pinned, true);
    search.relax_short_clearance(&pinned);
    search.compact_rails(&sizes, &pinned);
    search.reflow_rails(&sizes, &pinned);
    search.compact_rails(&sizes, &pinned);
    placement_report(search, size)
}
fn placement_report(search: &mut Search<'_>, size: f64) -> Report {
    let page = search.page;
    let n = page.anchors.len();
    for i in 0..n {
        let mut c = search.current[i].clone().unwrap();
        if c.path.is_empty() && page.anchors[i].marker.box_distance(c.rect) > TICK_GAP {
            c.path = page.anchors[i].marker.box_connection(c.rect).to_vec();
            search.set(i, c.clone());
        }
        if !page.anchors[i].repeated && !inside(page.body, c.bounds()) {
            for axis in 0..2 {
                let extent = c.rect[axis + 2] - c.rect[axis];
                let start = c.rect[axis].clamp(
                    page.body[axis],
                    (page.body[axis + 2] - extent).max(page.body[axis]),
                );
                c.rect[axis] = start;
                c.rect[axis + 2] = start + extent;
            }
            // Keep an impossible owned label visible inside the body. It may
            // overlap another object, and that conflict is still reported.
            let anchor = page.anchors[i].centre();
            let end = [
                anchor[0].clamp(c.rect[0], c.rect[2]),
                anchor[1].clamp(c.rect[1], c.rect[3]),
            ];
            c.path = vec![page.anchors[i].marker.edge(end), end];
            for p in &mut c.path {
                for (axis, coordinate) in p.iter_mut().enumerate() {
                    *coordinate = coordinate.clamp(page.body[axis], page.body[axis + 2]);
                }
            }
            c.dir = 0;
            search.set(i, c);
        }
    }
    // Remove only unresolved context, in stable occurrence order. Recount
    // afterwards so a removed plate cannot leave a false conflict on its owner.
    let mut context_omitted = 0;
    for i in 0..n {
        if page.anchors[i].repeated && search.bad(i) {
            search.remove(i);
            context_omitted += 1;
        }
    }
    let mut leader_paths = vec![String::new(); n];
    for (i, svg) in leader_paths.iter_mut().enumerate() {
        let Some(c) = search.current[i].clone() else {
            continue;
        };
        if c.path.is_empty() {
            continue;
        }
        if c.dir >= 8 {
            *svg = polyline_svg(&c.path);
            continue;
        }
        let (points, path) = rounded_path(&c.path);
        let rounded = Candidate {
            path: points,
            ..c.clone()
        };
        if search.conflicts(i, &rounded) <= search.conflicts(i, &c) {
            search.set(i, rounded);
            *svg = path;
        } else {
            *svg = polyline_svg(&c.path);
        }
    }
    let mut report = Report {
        size_mm: size,
        ..Report::default()
    };
    report.counts.context_omitted = context_omitted;
    report.counts.occurrences = n;
    let mut unique = std::collections::BTreeSet::new();
    for (i, leader_path) in leader_paths.iter().enumerate() {
        let a = &page.anchors[i];
        let Some(c) = search.current[i].as_ref() else {
            continue;
        };
        if !a.repeated {
            unique.insert(&a.spot_id);
        }
        let connector_tick =
            c.dir < 8 && c.path.len() == 2 && c.path == a.marker.box_connection(c.rect);
        let short_tick = connector_tick && distance(c.path[0], c.path[1]) < 3.0;
        report.counts.next_to_spot += usize::from(c.path.is_empty() || short_tick);
        report.counts.with_ticks += usize::from(short_tick);
        report.counts.with_leaders += usize::from(!c.path.is_empty() && !short_tick);
        report.counts.pinned += usize::from(search.pinned[i]);
        let overlapping = usize::from(search.conflicts(i, c) > 0);
        if search.pinned[i] {
            report.counts.pinned_overlapping += overlapping;
        } else {
            report.counts.still_overlapping += overlapping;
        }
        report.counts.ambiguous += usize::from(search.ambiguous(i, c));
        report.labels.push(PlacedLabel {
            spot_id: a.spot_id.clone(),
            occurrence: a.occurrence.clone(),
            name: a.name.clone(),
            anchor_mm: a.centre(),
            box_mm: c.rect,
            leader: c.path.clone(),
            leader_path: leader_path.clone(),
            connector_tick,
            last_resort: c.dir == ASTAR_DIR,
            pinned: search.pinned[i],
            repeated: a.repeated,
        });
    }
    report.counts.anchors = unique.len();
    report
}

fn polyline_svg(points: &[Point]) -> String {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{}{} {}", if i == 0 { "M" } else { "L" }, p[0], p[1]))
        .collect::<Vec<_>>()
        .join(" ")
}
/// Round detour corners, then test these exact flattened quadratics before
/// accepting their SVG. A rejected rounding keeps the collision-free elbow.
fn rounded_path(path: &[Point]) -> (Vec<Point>, String) {
    if path.len() < 3 {
        return (path.to_vec(), polyline_svg(path));
    }
    let mut points = vec![path[0]];
    let mut svg = format!("M{} {}", path[0][0], path[0][1]);
    for i in 1..path.len() - 1 {
        let p = path[i];
        let before = path[i - 1];
        let after = path[i + 1];
        let r = 0.5_f64
            .min(distance(before, p) * 0.2)
            .min(distance(p, after) * 0.2);
        let a = [
            p[0] + (before[0] - p[0]) * r / distance(before, p).max(1e-9),
            p[1] + (before[1] - p[1]) * r / distance(before, p).max(1e-9),
        ];
        let b = [
            p[0] + (after[0] - p[0]) * r / distance(after, p).max(1e-9),
            p[1] + (after[1] - p[1]) * r / distance(after, p).max(1e-9),
        ];
        points.push(a);
        let _ = write!(
            svg,
            " L{} {} Q{} {} {} {}",
            a[0], a[1], p[0], p[1], b[0], b[1]
        );
        for j in 1..=4 {
            let t = j as f64 / 4.0;
            points.push([
                (1.0 - t).powi(2) * a[0] + 2.0 * (1.0 - t) * t * p[0] + t * t * b[0],
                (1.0 - t).powi(2) * a[1] + 2.0 * (1.0 - t) * t * p[1] + t * t * b[1],
            ]);
        }
    }
    let p = *path.last().unwrap();
    points.push(p);
    let _ = write!(svg, " L{} {}", p[0], p[1]);
    (points, svg)
}

/// One band occurrence in paper space. display_to_paper includes per-piece scale
/// and rotation; mount_to_display includes image edits. Repeated regions are display µm.
pub struct Image<'a> {
    pub area: &'a str,
    pub occurrence: String,
    pub crop: Rect,
    pub display_to_paper: [f64; 6],
    pub mount_to_display: [f64; 6],
    pub repeated: Vec<Rect>,
    pub edits: Option<&'a crate::project::image_edits::AreaImageEdits>,
}
impl Page {
    pub fn new(body: Rect) -> Self {
        Self {
            body,
            images: vec![],
            hard: vec![],
            rail_gaps: vec![],
            grains: vec![],
            anchors: vec![],
        }
    }
    /// Only visible zoning is an obstacle; do not project cropped-away grain
    /// polygons onto the neighbouring white paper.
    pub fn grain(&mut self, image: &Image<'_>, polygon: &[Point]) {
        let mut polygon = polygon.to_vec();
        for (axis, edge, lower) in [
            (0, image.crop[0], true),
            (0, image.crop[2], false),
            (1, image.crop[1], true),
            (1, image.crop[3], false),
        ] {
            if polygon.is_empty() {
                break;
            }
            let mut clipped = Vec::new();
            for (a, b) in polygon
                .iter()
                .zip(polygon.iter().cycle().skip(1))
                .take(polygon.len())
            {
                let ain = if lower {
                    a[axis] >= edge
                } else {
                    a[axis] <= edge
                };
                let bin = if lower {
                    b[axis] >= edge
                } else {
                    b[axis] <= edge
                };
                if ain {
                    clipped.push(*a);
                }
                if ain != bin {
                    let t = (edge - a[axis]) / (b[axis] - a[axis]);
                    clipped.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
                }
            }
            polygon = clipped;
        }
        if polygon.len() >= 3 {
            self.grains.push(
                polygon
                    .into_iter()
                    .map(|p| arrange::point(image.display_to_paper, p))
                    .collect(),
            );
        }
    }
    pub fn image<'s>(&mut self, image: &Image<'_>, spots: impl IntoIterator<Item = &'s Spot>) {
        let index = self.images.len();
        self.images.push(
            corners(image.crop)
                .into_iter()
                .map(|p| arrange::point(image.display_to_paper, p))
                .collect(),
        );
        let paper = |p| {
            arrange::point(
                image.display_to_paper,
                arrange::point(image.mount_to_display, p),
            )
        };
        let scale = image.display_to_paper[0].hypot(image.display_to_paper[1]);
        for s in spots.into_iter().filter(|s| s.area_id == image.area) {
            let p = arrange::point(image.mount_to_display, [s.x_um, s.y_um]);
            if image.edits.is_some_and(|e| !e.visible([s.x_um, s.y_um]))
                || !contains(image.crop, p)
                || !s.diameter_um.is_finite()
                || s.diameter_um <= 0.0
            {
                continue;
            }
            let c = paper([s.x_um, s.y_um]);
            let end = [s.x2_um.unwrap_or(s.x_um), s.y2_um.unwrap_or(s.y_um)];
            let r = s.diameter_um * scale / 2.0;
            let marker = match s.kind {
                SpotKind::Spot => Marker::Circle(c, r),
                SpotKind::Line => Marker::Line(c, paper(end), r),
                SpotKind::Raster => Marker::Polygon(vec![
                    c,
                    paper([end[0], s.y_um]),
                    paper(end),
                    paper([s.x_um, end[1]]),
                ]),
            };
            self.anchors.push(Anchor {
                spot_id: s.id.to_string(),
                occurrence: image.occurrence.clone(),
                name: s.name.clone(),
                marker,
                image: index,
                repeated: image.repeated.iter().any(|r| contains(*r, p)),
            });
        }
    }
}

pub fn render(svg: &mut String, page: Page, options: &Options, sheet: usize) -> Report {
    let (page, mut report) = place(page, options);
    report.sheet = sheet;
    let end = svg.rfind("</svg>").expect("generated SVG");
    svg.truncate(end);
    if !report.labels.is_empty() {
        let _ = write!(
            svg,
            r#"<style>@font-face{{font-family:'IBM Plex Sans Condensed';font-style:normal;font-weight:500;src:url(data:font/ttf;base64,{}) format('truetype');}}</style>"#,
            super::base64(include_bytes!(
                "../../assets/fonts/IBMPlexSansCondensed-Medium.ttf"
            ))
        );
        // Plex Condensed lacks Greek, including Ω. Embed the same fallback used
        // by the PDF exporter and by text_size, rather than a system font.
        if report
            .labels
            .iter()
            .any(|l| l.name.chars().any(|c| font().glyph_index(c).is_none()))
        {
            let _ = write!(
                svg,
                r#"<style>@font-face{{font-family:'DejaVu Sans';font-style:normal;font-weight:500;src:url(data:font/ttf;base64,{}) format('truetype');}}</style>"#,
                super::base64(include_bytes!("../../assets/fonts/DejaVuSans.ttf"))
            );
        }
    }
    for (i, poly) in page.images.iter().enumerate() {
        let points = poly
            .iter()
            .map(|p| format!("{},{}", p[0], p[1]))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = write!(
            svg,
            r#"<clipPath id="spot-image-{i}"><polygon points="{points}"/></clipPath>"#
        );
    }
    for a in &page.anchors {
        let mut shape = String::new();
        match &a.marker {
            Marker::Circle(p, r) => {
                let _ = write!(shape, r#"<circle cx="{}" cy="{}" r="{r}"/>"#, p[0], p[1]);
            }
            Marker::Line(p, q, r) => {
                let _ = write!(
                    shape,
                    r#"<path d="M{} {} L{} {}" stroke-width="{}" stroke-linecap="round"/>"#,
                    p[0],
                    p[1],
                    q[0],
                    q[1],
                    2.0 * r
                );
            }
            Marker::Polygon(p) => {
                let _ = write!(
                    shape,
                    r#"<polygon points="{}"/>"#,
                    p.iter()
                        .map(|p| format!("{},{}", p[0], p[1]))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
        }
        let light = match &a.marker {
            Marker::Line(_, _, r) => shape.replace(
                &format!("stroke-width=\"{}\"", 2.0 * r),
                &format!("stroke-width=\"{}\"", (2.0 * r - 0.18).max(2.0 * r * 0.6)),
            ),
            _ => shape.clone(),
        };
        let _ = write!(
            svg,
            r##"<g data-spot-marker="{}" clip-path="url(#spot-image-{})" fill="none" stroke="#333" stroke-width="0.36">{shape}<g stroke="white" stroke-width="0.18">{light}</g></g>"##,
            esc(&a.spot_id),
            a.image
        );
        if let Marker::Circle(p, r) = &a.marker
            && 2.0 * r < 0.8
        {
            let _ = write!(
                svg,
                r##"<circle cx="{}" cy="{}" r="0.125" fill="#333" clip-path="url(#spot-image-{})"/>"##,
                p[0], p[1], a.image
            );
        }
        // For lines the actual width is their diameter; the white inset leaves a
        // dark casing without changing their physical outer footprint.
    }
    for l in &report.labels {
        if !l.leader.is_empty() {
            let mut points = l.leader.clone();
            for reverse in [false, true].into_iter().filter(|_| !l.connector_tick) {
                if reverse {
                    points.reverse();
                }
                let length = distance(points[0], points[1]);
                let gap = 0.25_f64.min(length * 0.45);
                points[0] = [
                    points[0][0] + (points[1][0] - points[0][0]) * gap / length.max(1e-9),
                    points[0][1] + (points[1][1] - points[0][1]) * gap / length.max(1e-9),
                ];
                if reverse {
                    points.reverse();
                }
            }
            let path = polyline_svg(&points);
            let width = if l.connector_tick { 0.1 } else { 0.14 };
            let _ = write!(
                svg,
                r##"<g data-spot-leader="{}" fill="none" stroke-linejoin="round" stroke-linecap="butt"><path d="{path}" clip-path="url(#spot-image-{})" stroke="white" stroke-width="0.48"/><path d="{path}" stroke="#333" stroke-width="{width}"/></g>"##,
                esc(&l.spot_id),
                page.anchors
                    .iter()
                    .find(|a| a.spot_id == l.spot_id && a.occurrence == l.occurrence)
                    .unwrap()
                    .image
            );
        }
    }
    for l in &report.labels {
        let b = l.box_mm;
        let colour = if l.repeated { "#777" } else { "#111" };
        let size = report.size_mm;
        let baseline = b[1] + 0.25 + font().ascender() as f64 / font().units_per_em() as f64 * size;
        let on_image = page.images.iter().any(|poly| polygon_rect(poly, b));
        let _ = write!(
            svg,
            r#"<g data-spot-label-box="{}" data-occurrence="{}">"#,
            esc(&l.spot_id),
            esc(&l.occurrence)
        );
        if on_image {
            let _ = write!(
                svg,
                r##"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="white" fill-opacity="0.9" stroke="#888" stroke-width="0.08"/>"##,
                b[0],
                b[1],
                b[2] - b[0],
                b[3] - b[1],
                (b[3] - b[1]) * 0.35
            );
        }
        let _ = write!(
            svg,
            r#"<text data-spot-label="{}" x="{}" y="{baseline}" font-family="IBM Plex Sans Condensed, DejaVu Sans" font-weight="500" font-size="{size}" font-kerning="none" style="font-variant-ligatures:none" fill="{colour}">{}</text></g>"#,
            esc(&l.spot_id),
            b[0] + 0.4,
            esc(&l.name)
        );
    }
    svg.push_str("</svg>");
    report
}

// Image payloads carry no vector obstacles. Skip their bytes with str::find
// before XML parsing; preserve tags, namespaces and every other attribute.
fn without_image_data(svg: &str) -> std::borrow::Cow<'_, str> {
    let mut output = String::new();
    let mut copied = 0;
    let mut cursor = 0;
    while let Some(at) = svg[cursor..].find("<image") {
        let tag = cursor + at + 6;
        cursor = tag;
        if !svg
            .as_bytes()
            .get(tag)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'/' || *b == b'>')
        {
            continue;
        }
        let Some(length) = svg[tag..].find('>') else {
            break;
        };
        let tag_end = tag + length;
        cursor = tag_end + 1;
        let Some(at) = svg[tag..tag_end].find("href=") else {
            continue;
        };
        let at = tag + at + 5;
        let Some(quote @ (b'"' | b'\'')) = svg.as_bytes().get(at).copied() else {
            continue;
        };
        let start = at + 1;
        let Some(length) = svg[start..tag_end].find(quote as char) else {
            continue;
        };
        let end = start + length;
        if svg[start..end].starts_with("data:image/") {
            output.push_str(&svg[copied..start]);
            copied = end;
        }
    }
    if copied == 0 {
        std::borrow::Cow::Borrowed(svg)
    } else {
        output.push_str(&svg[copied..]);
        std::borrow::Cow::Owned(output)
    }
}

/// Reserve existing vector decorations after geometry is known. Image pixels and
/// borders are soft, but text, scale bars, ticks, gap marks and notes are hard.
/// Paths are reserved per subpath, not by their combined page-sized envelope.
pub fn decorations(svg: &str, page: &mut Page) {
    let vectors = without_image_data(svg);
    let Ok(doc) = roxmltree::Document::parse(&vectors) else {
        return;
    };
    fn walk(n: roxmltree::Node<'_, '_>, m: [f64; 6], page: &mut Page) {
        let tag = n.tag_name().name();
        if matches!(tag, "defs" | "clipPath" | "pattern" | "image") {
            return;
        }
        let mut m = m;
        if let Some(t) = n
            .attribute("transform")
            .and_then(|v| v.parse::<svgtypes::Transform>().ok())
        {
            m = crate::project::image_edits::compose(m, [t.a, t.b, t.c, t.d, t.e, t.f]);
        }
        let num = |k: &str, default: f64| {
            n.attribute(k)
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(default)
        };
        let grid_stroke = n.ancestors().any(|node| {
            node.attribute("class")
                .is_some_and(|c| c.split_whitespace().any(|c| c == "whole-grid"))
        });
        // The faint grid is a background reference, like the image border.
        // Its text and edge ticks are still reserved by the normal walk.
        if grid_stroke && tag == "path" {
            return;
        }
        let mut reserve = |r: Rect| {
            let r = arrange::bounds(m, r);
            if overlap(r, page.body) {
                page.rail_gaps.resize(page.hard.len(), RAIL_GAP);
                page.hard.push(expand(r, 0.15));
                page.rail_gaps.push(RAIL_GAP);
            }
        };
        match tag {
            "text" => {
                let text = n
                    .descendants()
                    .filter(|x| x.is_text())
                    .filter_map(|x| x.text())
                    .collect::<String>();
                let fs = num("font-size", 2.4);
                // Decorations retain the original DejaVu metrics.
                let f =
                    ttf_parser::Face::parse(include_bytes!("../../assets/fonts/DejaVuSans.ttf"), 0)
                        .unwrap();
                let size = measured_text(&f, None, &text, fs);
                let x = num("x", 0.0);
                let y = num("y", 0.0);
                let x = match n.attribute("text-anchor") {
                    Some("middle") => x - size[0] / 2.0,
                    Some("end") => x - size[0],
                    _ => x,
                };
                reserve([x, y - fs, x + size[0], y + fs * 0.3]);
            }
            "circle" => {
                let x = num("cx", 0.0);
                let y = num("cy", 0.0);
                let r = num("r", 0.0);
                reserve([x - r, y - r, x + r, y + r]);
            }
            "path" => {
                let mut pts = Vec::new();
                for segment in
                    svgtypes::SimplifyingPathParser::from(n.attribute("d").unwrap_or("")).flatten()
                {
                    use svgtypes::SimplePathSegment::*;
                    match segment {
                        MoveTo { x, y } => {
                            if !pts.is_empty() {
                                reserve(expand(bounds(&pts), num("stroke-width", 0.2) / 2.0));
                            }
                            pts = vec![[x, y]];
                        }
                        LineTo { x, y } => pts.push([x, y]),
                        CurveTo {
                            x1,
                            y1,
                            x2,
                            y2,
                            x,
                            y,
                        } => pts.extend([[x1, y1], [x2, y2], [x, y]]),
                        Quadratic { x1, y1, x, y } => pts.extend([[x1, y1], [x, y]]),
                        ClosePath => {}
                    }
                }
                if !pts.is_empty() {
                    reserve(expand(bounds(&pts), num("stroke-width", 0.2) / 2.0));
                }
            }
            _ => {}
        }
        for child in n.children().filter(|n| n.is_element()) {
            walk(child, m, page);
        }
    }
    walk(doc.root_element(), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], page);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    fn assert_body(body: Rect, report: &Report) {
        for l in &report.labels {
            assert!(
                inside(body, l.box_mm),
                "{} outside body: {:?}",
                l.name,
                l.box_mm
            );
            assert!(l.leader.iter().all(|&p| contains(body, p)));
        }
    }
    // Every test page, including deliberate conflicts and pins, checks bounds.
    fn place(page: Page, options: &Options) -> (Page, Report) {
        let (page, report) = super::place(page, options);
        assert_body(page.body, &report);
        (page, report)
    }
    fn render(svg: &mut String, page: Page, options: &Options, sheet: usize) -> Report {
        let body = page.body;
        let report = super::render(svg, page, options, sheet);
        assert_body(body, &report);
        report
    }
    fn grid(n: usize, columns: usize, spacing: Point) -> Page {
        let mut p = Page::new([0.0, 0.0, 420.0, 280.0]);
        p.images.push(corners([5.0, 5.0, 415.0, 275.0]));
        for i in 0..n {
            p.anchors.push(Anchor {
                spot_id: format!("{i:04}"),
                occurrence: "band/0".into(),
                name: format!("Z-{i:03}"),
                marker: Marker::Circle(
                    [
                        30.0 + (i % columns) as f64 * spacing[0],
                        30.0 + (i / columns) as f64 * spacing[1],
                    ],
                    1.0,
                ),
                image: 0,
                repeated: false,
            });
        }
        p
    }
    fn verify_count(page: &Page, r: &Report) {
        let mut search = Search {
            page,
            markers: Grid::default(),
            fixed: Grid::default(),
            grains: Grid::default(),
            images: Grid::default(),
            placed: Grid::default(),
            current: vec![None; page.anchors.len()],
            pinned: page
                .anchors
                .iter()
                .map(|a| {
                    r.labels
                        .iter()
                        .any(|l| l.spot_id == a.spot_id && l.occurrence == a.occurrence && l.pinned)
                })
                .collect(),
            generation: 0,
            routes_left: Cell::new(PAGE_ROUTE_LIMIT),
            group_routes_left: Cell::new(GROUP_ROUTE_LIMIT),
            astar_left: Cell::new(PAGE_ASTAR_VISITS),
            repair_calls_left: Cell::new(PAGE_REPAIR_CALLS),
            repair_pairs_left: Cell::new(PAGE_REPAIR_PAIRS),
            route_cache: RefCell::new(HashMap::new()),
        };
        for (i, a) in page.anchors.iter().enumerate() {
            search.markers.insert(i, expand(a.marker.bounds(), GAP));
        }
        for (i, b) in page.hard.iter().enumerate() {
            search.fixed.insert(i, *b);
        }
        for (i, polygon) in page.grains.iter().enumerate() {
            search.grains.insert(i, bounds(polygon));
        }
        for (i, polygon) in page.images.iter().enumerate() {
            search.images.insert(i, bounds(polygon));
        }
        for l in &r.labels {
            let i = page
                .anchors
                .iter()
                .position(|a| a.spot_id == l.spot_id && a.occurrence == l.occurrence)
                .unwrap();
            assert!(
                inside(page.body, l.box_mm),
                "{} outside body: {:?}",
                l.name,
                l.box_mm
            );
            assert!(l.leader.iter().all(|&p| contains(page.body, p)));
            search.set(
                i,
                Candidate {
                    rect: l.box_mm,
                    path: l.leader.clone(),
                    dir: if l.last_resort {
                        ASTAR_DIR
                    } else if !polygon_rect(&page.images[page.anchors[i].image], l.box_mm) {
                        let b = bounds(&page.images[page.anchors[i].image]);
                        let axis = usize::from(b[3] - b[1] > b[2] - b[0]);
                        8 + 2 * axis + usize::from(l.box_mm[1 - axis] > b[3 - axis])
                    } else {
                        0
                    },
                    cost: 0.0,
                },
            );
        }
        assert_eq!(
            r.labels.len() + r.counts.context_omitted,
            page.anchors.len()
        );
        let actual = (0..page.anchors.len())
            .filter(|&i| !search.pinned[i] && search.current[i].is_some() && search.bad(i))
            .count();
        assert_eq!(
            r.counts.still_overlapping, actual,
            "every unresolved automatic label is counted"
        );
        let pinned_actual = (0..page.anchors.len())
            .filter(|&i| search.pinned[i] && search.current[i].is_some() && search.bad(i))
            .count();
        assert_eq!(
            r.counts.pinned_overlapping, pinned_actual,
            "every conflicting pin is counted separately"
        );
        let ticks = r
            .labels
            .iter()
            .filter(|l| {
                l.connector_tick
                    && l.leader
                        .windows(2)
                        .map(|s| distance(s[0], s[1]))
                        .sum::<f64>()
                        < 3.0
            })
            .count();
        assert_eq!(r.counts.with_ticks, ticks);
        assert_eq!(
            r.counts.with_leaders,
            r.labels.iter().filter(|l| !l.leader.is_empty()).count() - ticks
        );
        let ambiguous = search
            .current
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                c.as_ref().is_some_and(|c| {
                    let own = page.anchors[*i].marker.box_distance(c.rect);
                    c.path.is_empty()
                        && page.anchors.iter().enumerate().any(|(j, a)| {
                            j != *i && a.marker.box_distance(c.rect) <= 1.2 * own + 1e-9
                        })
                })
            })
            .count();
        assert_eq!(
            r.counts.ambiguous, ambiguous,
            "every ambiguous label is counted"
        );
    }
    fn verify(page: &Page, r: &Report) {
        verify_count(page, r);
        assert_eq!(r.counts.still_overlapping, 0);
    }
    #[test]
    fn auto_uses_larger_type_on_sparse_paper_and_pins_keep_mm_offsets() {
        let (_, sparse) = place(grid(12, 4, [24.0, 24.0]), &Options::default());
        let (_, dense) = place(grid(60, 10, [4.5, 4.5]), &Options::default());
        assert!(
            sparse.size_mm > dense.size_mm,
            "{} > {}",
            sparse.size_mm,
            dense.size_mm
        );
        for spacing in [[24.0, 24.0], [4.5, 4.5]] {
            let p = grid(60, 10, spacing);
            let key = p.anchors[0].key();
            let centre = p.anchors[0].centre();
            let mut options = Options::default();
            options.pins.insert(key, [10.0, 8.0]);
            let (_, report) = place(p, &options);
            assert_eq!(
                report.labels[0].box_mm[..2],
                [centre[0] + 10.0, centre[1] + 8.0]
            );
            assert!(report.labels[0].pinned);
        }
    }
    #[test]
    fn auto_ignores_white_paper_leaders_and_fixed_sizes_can_be_smaller() {
        let mut page = grid(10, 10, [30.0, 0.0]);
        page.images = vec![corners([10.0, 10.0, 415.0, 40.0])];
        let mut options = Options::default();
        for a in &page.anchors {
            options.pins.insert(a.key(), [0.0, 20.0]);
        }
        let (p, r) = place(page.clone(), &options);
        verify(&p, &r);
        assert_eq!(r.size_mm, 3.2);
        assert_eq!(r.counts.with_leaders, 10);
        assert_eq!(in_image_leaders(&p, &r), 0);
        options.size_mm = 1.6;
        assert_eq!(place(page, &options).1.size_mm, 1.6);
    }
    #[test]
    fn auto_accepts_fifteen_percent_in_image_leaders_and_otherwise_uses_floor() {
        for (leaders, expected) in [(3, 3.2), (4, 2.4)] {
            let page = grid(20, 10, [18.0, 20.0]);
            let mut options = Options::default();
            for a in &page.anchors[..leaders] {
                options.pins.insert(a.key(), [0.0, 4.0]);
            }
            let (p, r) = place(page, &options);
            verify(&p, &r);
            assert_eq!(in_image_leaders(&p, &r), leaders);
            assert_eq!(r.size_mm, expected);
        }
    }
    #[test]
    fn auto_accepts_connected_associations_even_near_another_marker() {
        let mut page = grid(2, 2, [30.0, 0.0]);
        page.anchors[0].name = "AB12_S_211".into();
        let width = (text_size("AB12_S_211", 3.2)[0] + text_size("AB12_S_211", 2.9)[0]) / 2.0;
        // This pin used to force Auto to 2.9 mm by counting proximity to
        // another marker, even though its 2.5 mm tick identifies its own.
        let threshold = (4.0_f64.powi(2) - 3.5_f64.powi(2)).sqrt();
        page.anchors[1].marker = Marker::Circle([30.0 + width + threshold, 30.0], 1.0);
        let mut options = Options {
            size_mm: 3.2,
            ..Default::default()
        };
        options.pins.insert(page.anchors[0].key(), [0.0, 3.5]);
        options.pins.insert(page.anchors[1].key(), [3.0, 0.0]);
        let (p, fixed) = place(page.clone(), &options);
        verify(&p, &fixed);
        assert!(fixed.labels[0].connector_tick);
        assert_eq!(fixed.counts.ambiguous, 0);
        assert_eq!(in_image_leaders(&p, &fixed), 0);
        options.size_mm = 0.0;
        let (p, automatic) = place(page, &options);
        verify(&p, &automatic);
        assert_eq!(automatic.size_mm, 3.2);
        assert_eq!(automatic.counts.ambiguous, 0);
    }
    #[test]
    fn only_unconnected_labels_pay_ambiguity_cost() {
        let mut page = grid(2, 2, [30.0, 0.0]);
        page.anchors[1].marker = Marker::Circle([40.0, 30.0], 1.0);
        let search = new_search(&page);
        let mut c = Candidate {
            rect: [30.0, 31.7, 40.0, 36.0],
            path: vec![],
            dir: 0,
            cost: 0.0,
        };
        assert!((page.anchors[0].marker.box_distance(c.rect) - 0.7).abs() < 1e-9);
        assert!(search.ambiguous(0, &c));
        assert_eq!(search.ambiguity_cost(0, &c), AMBIGUITY_COST);
        c.path = page.anchors[0].marker.box_connection(c.rect).to_vec();
        assert!(!search.ambiguous(0, &c));
        assert_eq!(search.ambiguity_cost(0, &c), 0.0);
        c.path.push([40.0, 31.7]);
        c.dir = 8;
        assert!(!search.ambiguous(0, &c));
        assert_eq!(search.ambiguity_cost(0, &c), 0.0);
        c.path.clear();
        c.rect = [30.0, 35.0, 40.0, 39.0];
        assert!(
            !search.ambiguous(0, &c),
            "a distant candidate will receive a connector"
        );
        assert_eq!(search.ambiguity_cost(0, &c), 0.0);
    }
    #[test]
    fn marker_box_distances_use_physical_edges_and_nearest_points() {
        let rect = [10.0, 10.0, 20.0, 20.0];
        for (marker, expected) in [
            (
                Marker::Circle([5.0, 15.0], 2.0),
                [[7.0, 15.0], [10.0, 15.0]],
            ),
            (
                Marker::Line([5.0, 5.0], [5.0, 25.0], 2.0),
                [[7.0, 10.0], [10.0, 10.0]],
            ),
            (
                Marker::Polygon(corners([3.0, 5.0, 5.0, 25.0])),
                [[5.0, 10.0], [10.0, 10.0]],
            ),
        ] {
            let connection = marker.box_connection(rect);
            assert_eq!(connection, expected);
            assert_eq!(
                marker.box_distance(rect),
                distance(expected[0], expected[1])
            );
            let mut page = grid(1, 1, [20.0, 20.0]);
            page.anchors[0].marker = marker;
            let mut options = Options {
                size_mm: 3.2,
                ..Default::default()
            };
            let centre = page.anchors[0].centre();
            options
                .pins
                .insert(page.anchors[0].key(), [10.0 - centre[0], 10.0 - centre[1]]);
            let mut svg = "<svg></svg>".into();
            let report = render(&mut svg, page, &options, 1);
            assert!(report.labels[0].connector_tick);
            assert!(svg.contains("stroke-width=\"0.1\""));
            assert!(svg.contains("stroke=\"white\" stroke-width=\"0.48\""));
        }
    }
    #[test]
    fn crop_like_eight_grain_row_has_no_ambiguous_associations() {
        let mut page = grid(8, 8, [21.5, 0.0]);
        for (i, a) in page.anchors.iter_mut().enumerate() {
            let c = [30.0 + i as f64 * 21.5, 60.0 + (i % 3) as f64 * 2.0];
            a.marker = Marker::Circle(c, 25.0 / 8.0 / 2.0);
            a.name = format!("AB12_S_{}", 211 + i);
            page.grains
                .push(corners([c[0] - 3.0, c[1] - 8.0, c[0] + 3.0, c[1] + 8.0]));
        }
        let search = new_search(&page);
        // Round 7's upper-right corner puts at least one plate nearer the
        // next grain's marker. The old corners remain in the candidate pool.
        assert!((0..7).any(|i| {
            let old = candidate(
                &page.anchors[i],
                text_size(&page.anchors[i].name, 3.2),
                0,
                0,
            );
            page.anchors[i + 1].marker.box_distance(old.rect)
                < page.anchors[i].marker.box_distance(old.rect)
                && !old.path.is_empty()
                && !search.ambiguous(i, &old)
        }));
        let options = Options {
            size_mm: 3.2,
            ..Default::default()
        };
        let (page, report) = place(page, &options);
        verify(&page, &report);
        assert_eq!(report.labels.len(), 8);
        assert_eq!(report.counts.ambiguous, 0);
        // Independent exhaustive circle-to-box distances verify the grid query.
        for (i, l) in report.labels.iter().enumerate() {
            let own = page.anchors[i].marker.box_distance(l.box_mm);
            for (j, a) in page.anchors.iter().enumerate() {
                if i != j && l.leader.is_empty() {
                    assert!(a.marker.box_distance(l.box_mm) > 1.2 * own);
                }
            }
            if own > 0.8 {
                assert_eq!(l.leader, page.anchors[i].marker.box_connection(l.box_mm));
                assert!(l.connector_tick);
                assert_eq!(l.leader.len(), 2);
            }
        }
        let mut shuffled = page.clone();
        shuffled.anchors.reverse();
        assert_eq!(report, place(shuffled, &options).1);
    }
    #[test]
    fn auto_height_cap_tracks_grains_in_paper_space_and_size_round_trips() {
        let mut page = grid(1, 1, [20.0, 20.0]);
        page.grains.push(corners([20.0, 20.0, 23.0, 23.0]));
        let sizes = auto_sizes(&page);
        let median = (36.0 / std::f64::consts::PI).sqrt();
        assert!(text_size("ABC12", sizes[0])[1] <= 1.5 * median);
        page.grains = vec![corners([20.0, 20.0, 21.0, 21.0])];
        assert_eq!(
            auto_sizes(&page),
            vec![2.4],
            "readability overrides a smaller scale cap"
        );
        let options: Options = serde_json::from_str(r#"{"size_mm":"auto"}"#).unwrap();
        assert_eq!(options, Options::default());
        assert_eq!(serde_json::to_value(options).unwrap()["size_mm"], "auto");
    }
    #[test]
    fn plex_is_embedded_once_only_when_labels_exist_and_white_paper_has_no_plate() {
        let mut svg = "<svg></svg>".to_string();
        render(
            &mut svg,
            Page::new([0.0, 0.0, 100.0, 100.0]),
            &Options::default(),
            1,
        );
        assert!(!svg.contains("@font-face"));
        let mut p = grid(1, 1, [20.0, 20.0]);
        let mut options = Options::default();
        options.pins.insert(p.anchors[0].key(), [0.0, -20.0]);
        p.images = vec![corners([20.0, 20.0, 80.0, 80.0])];
        render(&mut svg, p, &options, 1);
        assert_eq!(svg.matches("@font-face").count(), 1);
        assert!(svg.contains("data:font/ttf;base64,"));
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let label = doc
            .descendants()
            .find(|n| n.attribute("data-spot-label-box").is_some())
            .unwrap();
        assert!(!label.children().any(|n| n.has_tag_name("rect")));
        let text = label.children().find(|n| n.has_tag_name("text")).unwrap();
        assert_eq!(
            text.attribute("font-family"),
            Some("IBM Plex Sans Condensed, DejaVu Sans")
        );
        assert_eq!(text.attribute("font-kerning"), Some("none"));
    }
    #[test]
    fn missing_plex_glyph_uses_the_embedded_pdf_fallback_advance() {
        assert!(font().glyph_index('Ω').is_none());
        let fallback = fallback_font();
        let advance = fallback
            .glyph_hor_advance(fallback.glyph_index('Ω').unwrap())
            .unwrap();
        assert_eq!(advance, 1565);
        assert_eq!(fallback.units_per_em(), 2048);
        for size in [2.4, 3.0, 3.2] {
            assert!((text_size("Ω", size)[0] - 0.8 - size * 1565.0 / 2048.0).abs() < 1e-9);
            assert!((text_size("Z-002", size)[0] - 0.8 - size * 2.514).abs() < 1e-9);
        }
        let mut page = grid(1, 1, [20.0, 20.0]);
        page.anchors[0].name = "MNT-A_12 <&\" Ω".into();
        let mut svg = "<svg></svg>".to_string();
        render(&mut svg, page, &Options::default(), 1);
        assert_eq!(svg.matches("@font-face").count(), 2);
        assert!(svg.contains("font-family:'DejaVu Sans'"));
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let text = doc
            .descendants()
            .find(|n| n.attribute("data-spot-label").is_some())
            .unwrap();
        assert_eq!(text.text(), Some("MNT-A_12 <&\" Ω"));
        assert_eq!(
            text.attribute("font-family"),
            Some("IBM Plex Sans Condensed, DejaVu Sans")
        );
    }
    #[test]
    fn clearance_includes_parallel_leaders_and_exempts_only_anchor_neighbourhoods() {
        let a = [[0.0, 0.0], [12.0, 0.0]];
        let b = [[0.0, 0.79], [12.0, 0.79]];
        assert!(leaders_close(&a, &b, [a[0], b[0]]));
        let b = [[0.0, 0.81], [12.0, 0.81]];
        assert!(!leaders_close(&a, &b, [a[0], b[0]]));
        assert!(!leaders_close(
            &[[0.0, 0.0], [1.0, 0.0]],
            &[[0.0, 0.1], [1.0, 0.1]],
            [[0.0, 0.0], [0.0, 0.1]]
        ));
    }
    #[test]
    fn whole_grid_strokes_are_background_but_labels_and_ticks_stay_hard() {
        let mut page = Page::new([0.0, 0.0, 100.0, 100.0]);
        decorations(
            r#"<svg><g class="background whole-grid"><g transform="translate(5 5)">
            <path d="M10 0 V80 M0 10 H80" stroke-width="0.25"/>
            </g><text x="20" y="20">A1</text></g>
            <path d="M40 30 V32" stroke-width="0.2"/></svg>"#,
            &mut page,
        );
        assert_eq!(page.hard.len(), 2);
        assert!(page.hard.iter().any(|r| contains(*r, [20.0, 20.0])));
        assert!(page.hard.iter().any(|r| contains(*r, [40.0, 31.0])));
        assert!(!page.hard.iter().any(|r| contains(*r, [15.0, 50.0])));
    }
    #[test]
    fn image_data_is_skipped_but_vector_attributes_are_preserved() {
        let svg = r#"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href='data:image/png;base64,AAAA' transform="translate(1 2)"/><image href="data:image/png;base64,BBBB"/><text x="20" y="20">Scale</text></svg>"#;
        let stripped = without_image_data(svg);
        assert!(!stripped.contains("AAAA"));
        assert!(!stripped.contains("BBBB"));
        let mut a = Page::new([0.0, 0.0, 100.0, 100.0]);
        let mut b = Page::new(a.body);
        decorations(svg, &mut a);
        decorations(&stripped, &mut b);
        assert_eq!(a.hard, b.hard);
        assert_eq!(a.hard.len(), 1);
        assert!(stripped.contains("translate(1 2)"));
    }
    #[test]
    fn transformed_kinds_repeat_and_pin_offsets_use_paper_mm() {
        let spots:Vec<Spot>=serde_json::from_value(serde_json::json!([
            {"id":"70000000-0000-4000-8000-000000000001","area_id":"a","name":"Z-001","x_um":20,"y_um":30,"diameter_um":24},
            {"id":"70000000-0000-4000-8000-000000000002","area_id":"a","name":"Z-002","kind":"line","x_um":20,"y_um":30,"x2_um":40,"y2_um":50,"diameter_um":24},
            {"id":"70000000-0000-4000-8000-000000000003","area_id":"a","name":"Z-003","kind":"raster","x_um":20,"y_um":30,"x2_um":40,"y2_um":50,"diameter_um":24}
        ])).unwrap();
        let image = Image {
            area: "a",
            occurrence: "band/0".into(),
            crop: [0.0, 0.0, 80.0, 80.0],
            display_to_paper: [0.0, 0.25, -0.25, 0.0, 100.0, 50.0],
            mount_to_display: [-1.0, 0.0, 0.0, 1.0, 80.0, 0.0],
            repeated: vec![[50.0, 20.0, 70.0, 40.0]],
            edits: None,
        };
        let mut page = Page::new([0.0, 0.0, 200.0, 150.0]);
        page.image(&image, &spots);
        assert_eq!(page.anchors[0].marker, Marker::Circle([92.5, 65.0], 3.0));
        assert_eq!(
            page.anchors[1].marker,
            Marker::Line([92.5, 65.0], [87.5, 60.0], 3.0)
        );
        assert!(page.anchors.iter().all(|a| a.repeated));
        let mut options = Options::default();
        options.pins.insert(page.anchors[0].key(), [12.0, 8.0]);
        let (_, before) = place(page, &options);
        let mut moved = Page::new([0.0, 0.0, 200.0, 150.0]);
        moved.image(
            &Image {
                display_to_paper: [0.0, 0.25, -0.25, 0.0, 110.0, 55.0],
                ..image
            },
            &spots,
        );
        let (_, after) = place(moved, &options);
        assert_eq!(before.counts.anchors, 0);
        assert_eq!(before.counts.occurrences, 3);
        assert_eq!(after.labels[0].box_mm[0] - before.labels[0].box_mm[0], 10.0);
        assert_eq!(after.labels[0].box_mm[1] - before.labels[0].box_mm[1], 5.0);
    }
    #[test]
    fn labels_avoid_foreign_images_and_decorations() {
        let mut page = grid(1, 1, [18.0, 20.0]);
        page.body = [0.0, 0.0, 100.0, 90.0];
        page.images = vec![
            corners([10.0, 10.0, 40.0, 60.0]),
            corners([42.0, 10.0, 80.0, 60.0]),
        ];
        page.anchors[0].marker = Marker::Circle([39.0, 30.0], 1.0);
        page.hard.push([18.0, 24.0, 38.0, 29.0]);
        let (page, report) = place(page, &Options::default());
        verify(&page, &report);
        assert!(!polygon_rect(&page.images[1], report.labels[0].box_mm));
    }
    #[test]
    fn sparse_200_next_to_spots() {
        let (p, r) = place(grid(200, 20, [18.0, 20.0]), &Options::default());
        verify(&p, &r);
        assert_eq!(r.counts.next_to_spot, 200);
        assert_eq!(r.counts.with_ticks, 200);
        assert_eq!(r.counts.with_leaders, 0);
        assert!(
            r.labels
                .iter()
                .all(|l| l.connector_tick && l.leader.len() == 2)
        );
        assert_eq!(in_image_leaders(&p, &r), 0);
    }
    #[test]
    fn crowded_60_leaders() {
        let mut page = grid(60, 10, [4.5, 4.5]);
        page.grains = vec![
            corners([27.0, 27.0, 49.0, 42.0]),
            corners([49.5, 27.0, 74.0, 42.0]),
            corners([27.0, 42.5, 74.0, 56.0]),
        ];
        let (p, r) = place(page, &Options::default());
        verify_count(&p, &r);
        assert!(r.counts.with_leaders > 0);
    }
    #[test]
    fn touching_grains_and_image_edges() {
        let mut p = grid(20, 10, [4.5, 4.5]);
        p.grains = vec![
            corners([27.0, 27.0, 49.0, 39.0]),
            corners([49.1, 27.0, 74.0, 39.0]),
        ];
        p.anchors[0].marker = Marker::Circle([5.5, 5.5], 1.0);
        p.anchors[1].marker = Marker::Circle([414.0, 274.0], 1.0);
        let (p, r) = place(p, &Options::default());
        verify(&p, &r);
    }
    #[test]
    fn deterministic_under_shuffle() {
        let p = grid(60, 10, [4.5, 4.5]);
        let mut shuffled = grid(60, 10, [4.5, 4.5]);
        shuffled.anchors.reverse();
        let (_, a) = place(p, &Options::default());
        let (_, b) = place(shuffled, &Options::default());
        assert_eq!(a, b);
        assert_eq!(a, place(grid(60, 10, [4.5, 4.5]), &Options::default()).1);
    }
    #[test]
    fn pins_are_exact_and_obstacles() {
        let p = grid(20, 10, [18.0, 20.0]);
        let a = &p.anchors[0];
        let anchor = a.centre();
        let mut options = Options::default();
        options.pins.insert(a.key(), [10.0, 8.0]);
        let (p, r) = place(p, &options);
        let l = &r.labels[0];
        assert_eq!(l.box_mm[0], anchor[0] + 10.0);
        assert_eq!(l.box_mm[1], anchor[1] + 8.0);
        assert!(l.pinned);
        verify(&p, &r);
    }
    #[test]
    fn pinned_conflict_is_separate_and_automatic_labels_avoid_the_pin() {
        let p = grid(3, 3, [12.0, 0.0]);
        let mut options = Options {
            size_mm: 2.4,
            ..Options::default()
        };
        // Deliberately cover this pin's own marker. Its neighbours must still
        // find clear plates and leaders, and the user must see the pin warning.
        options.pins.insert(p.anchors[0].key(), [0.0, 0.0]);
        let (page, report) = place(p, &options);
        verify(&page, &report);
        assert_eq!(report.counts.pinned, 1);
        assert_eq!(report.counts.pinned_overlapping, 1);
        assert_eq!(report.labels[0].box_mm[..2], page.anchors[0].centre());
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["counts"]["still_overlapping"], 0);
        assert_eq!(value["counts"]["pinned_overlapping"], 1);
    }
    #[test]
    fn off_body_pins_stay_in_body_and_context_removal_clears_owner_conflict() {
        let mut p = grid(2, 2, [30.0, 0.0]);
        p.anchors[0].repeated = true;
        let mut options = Options {
            size_mm: 2.4,
            ..Options::default()
        };
        // Both boxes are pinned to one valid position. Context is optional;
        // removing it must also remove the owner's conflict with that box.
        for a in &p.anchors {
            options
                .pins
                .insert(a.key(), [90.0 - a.centre()[0], 60.0 - a.centre()[1]]);
        }
        let (p, r) = place(p, &options);
        verify(&p, &r);
        assert_eq!(r.counts.context_omitted, 1);
        assert_eq!(r.labels.len(), 1);
        assert!(!r.labels[0].repeated);
        options.pins.insert(p.anchors[1].key(), [-1000.0, -1000.0]);
        let (_, r) = place(p, &options);
        assert!(r.labels.iter().any(|l| !l.repeated));
    }
    #[test]
    fn impossible_is_reported_and_names_are_exact() {
        let mut p = grid(2, 2, [0.0, 0.0]);
        // The body must fit the full name. Fill it with a hard obstacle to
        // make placement impossible without demanding text wider than paper.
        p.body = [8.0, 8.0, 60.0, 50.0];
        p.hard.push(p.body);
        p.anchors[0].name = "MNT-A_12 <&\" Ω".into();
        let mut svg = "<svg></svg>".to_string();
        let r = render(&mut svg, p, &Options::default(), 1);
        assert!(r.counts.still_overlapping > 0);
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let label = doc.descendants().find(|n| n.has_tag_name("text")).unwrap();
        assert_eq!(label.text(), Some("MNT-A_12 <&\" Ω"));
    }
    fn mixed_page(spaced: usize, groups: usize, per_group: usize) -> Page {
        let mut p = grid(spaced, 20, [22.0, 18.0]);
        p.body = [0.0, 0.0, 500.0, 500.0];
        p.images = vec![corners([5.0, 5.0, 495.0, 495.0])];
        for g in 0..groups {
            for j in 0..per_group {
                p.anchors.push(Anchor {
                    spot_id: format!("group-{g:02}-{j:02}"),
                    occurrence: "band/0".into(),
                    name: format!("Z-{j:03}"),
                    marker: Marker::Circle(
                        [
                            80.0 + g as f64 * 140.0 + (j % 4) as f64 * 4.5,
                            370.0 + (j / 4) as f64 * 4.5,
                        ],
                        1.0,
                    ),
                    image: 0,
                    repeated: false,
                });
            }
        }
        p
    }
    #[test]
    fn local_rails_leave_forty_adjacent_labels_unchanged() {
        // Compare placement at the same size; Auto intentionally resizes the
        // whole sheet when a dense group is added.
        let fixed = Options {
            size_mm: 2.4,
            ..Default::default()
        };
        let (_, before) = place(mixed_page(40, 0, 0), &fixed);
        let (p, after) = place(mixed_page(40, 1, 12), &fixed);
        verify(&p, &after);
        assert_eq!(&after.labels[..40], before.labels.as_slice());
        assert!(after.labels[..40].iter().all(|l| l.leader.is_empty()
            || (l.connector_tick && distance(l.leader[0], l.leader[1]) < 3.0)));
    }
    #[test]
    fn three_groups_on_a_page_above_the_old_cap() {
        let (p, r) = place(mixed_page(300, 3, 15), &Options::default());
        verify(&p, &r);
        assert!(r.labels[..300].iter().all(|l| l.leader.is_empty()
            || (l.connector_tick && distance(l.leader[0], l.leader[1]) < 3.0)));
    }
    #[test]
    fn twelve_at_eight_mm_with_three_mm_labels_and_dragged_pin() {
        let make = || {
            let mut p = grid(12, 4, [8.0, 8.0]);
            for (i, a) in p.anchors.iter_mut().enumerate() {
                a.name = if i == 0 {
                    "MNT-A_12 <&\" Ω".into()
                } else {
                    format!("Z-{:03}", i + 1)
                };
                let raw = a.centre();
                let c = [raw[0] + 30.0, raw[1] + 40.0];
                a.marker = match i {
                    10 => Marker::Line(c, [c[0] + 2.5, c[1] + 2.5], 2.5),
                    11 => Marker::Polygon(corners([c[0], c[1], c[0] + 2.5, c[1] + 2.5])),
                    _ => Marker::Circle(c, 2.5),
                };
            }
            p
        };
        let mut options = Options {
            size_mm: 3.0,
            ..Default::default()
        };
        let (p, base) = place(make(), &options);
        verify(&p, &base);
        let first = &base.labels[0];
        options.pins.insert(
            p.anchors[0].key(),
            [
                first.box_mm[0] - first.anchor_mm[0] + 15.0 / 6.0,
                first.box_mm[1] - first.anchor_mm[1] + 12.0 / 6.0,
            ],
        );
        let (p, r) = place(make(), &options);
        verify(&p, &r);
        assert_eq!(r.counts.pinned, 1);
    }
    #[test]
    fn large_groups_split_canonically_on_the_long_axis() {
        let page = grid(181, 181, [1.0, 0.0]);
        let mut a = Vec::new();
        let mut b = Vec::new();
        split_group(&page, (0..181).collect(), &mut a);
        split_group(&page, (0..181).rev().collect(), &mut b);
        assert_eq!(a, b);
        assert!(a.iter().all(|p| p.len() <= 80));
        assert_eq!(a.concat(), (0..181).collect::<Vec<_>>());
    }
    #[test]
    fn sparse_six_hundred_debug_budget() {
        let start = Instant::now();
        let (p, r) = place(grid(600, 25, [15.0, 10.0]), &Options::default());
        verify(&p, &r);
        assert!(start.elapsed().as_secs_f64() < 8.0);
    }
    #[test]
    #[ignore = "release timing, run explicitly with --release --ignored --nocapture"]
    fn sparse_600_release_timing() {
        let start = Instant::now();
        let (p, r) = place(grid(600, 25, [15.0, 10.0]), &Options::default());
        let elapsed = start.elapsed();
        verify(&p, &r);
        eprintln!(
            "600 sparse anchors: {:.3} ms",
            elapsed.as_secs_f64() * 1000.0
        );
        let start = Instant::now();
        let (p, r) = place(grid(60, 10, [4.5, 4.5]), &Options::default());
        let dense_time = start.elapsed();
        verify_count(&p, &r);
        assert!(dense_time.as_secs_f64() < 0.500, "{dense_time:?}");
        eprintln!(
            "60 crowded anchors: {:.3} ms, {} ticks, {} leaders, {} overlaps, {} ambiguous",
            dense_time.as_secs_f64() * 1000.0,
            r.counts.with_ticks,
            r.counts.with_leaders,
            r.counts.still_overlapping,
            r.counts.ambiguous
        );
        assert!(elapsed.as_secs_f64() < 0.150);
    }
    fn realistic_page() -> Page {
        // 420 single-grain anchors, 180 in groups of 4..12 on adjacent grains.
        // A2 landscape body; single-grain spacings are 22 by 12 mm.
        let mut p = Page::new([0.0, 0.0, 594.0, 420.0]);
        p.images.push(corners([5.0, 5.0, 589.0, 415.0]));
        for i in 0..420 {
            let c = [25.0 + (i % 24) as f64 * 22.0, 25.0 + (i / 24) as f64 * 12.0];
            p.anchors.push(Anchor {
                spot_id: format!("single-{i:04}"),
                occurrence: "band/0".into(),
                name: format!("ABC12_S_{i:03}"),
                marker: Marker::Circle(c, (20 + i % 16) as f64 / 8.0 / 2.0),
                image: 0,
                repeated: false,
            });
            p.grains
                .push(corners([c[0] - 4.0, c[1] - 4.0, c[0] + 4.0, c[1] + 4.0]));
        }
        let mut remaining = 180;
        let mut g = 0;
        while remaining > 0 {
            let count = (4 + g % 9).min(remaining);
            let origin = [25.0 + (g % 6) as f64 * 94.0, 260.0 + (g / 6) as f64 * 40.0];
            for j in 0..count {
                let c = [
                    origin[0] + (j % 4) as f64 * 7.0,
                    origin[1] + (j / 4) as f64 * 7.0,
                ];
                p.anchors.push(Anchor {
                    spot_id: format!("group-{g:02}-{j:02}"),
                    occurrence: "band/0".into(),
                    name: format!("ABC12_S_{:03}", 420 + 180 - remaining + j),
                    marker: Marker::Circle(c, (20 + j % 16) as f64 / 8.0 / 2.0),
                    image: 0,
                    repeated: false,
                });
                p.grains
                    .push(corners([c[0] - 3.3, c[1] - 3.3, c[0] + 3.3, c[1] + 3.3]));
            }
            remaining -= count;
            g += 1;
        }
        p
    }
    #[test]
    fn six_hundred_debug_budget() {
        let start = Instant::now();
        let (p, r) = place(realistic_page(), &Options::default());
        let elapsed = start.elapsed();
        verify(&p, &r);
        assert!(elapsed.as_secs_f64() < 8.0, "{elapsed:?}");
    }
    #[test]
    #[ignore = "release timing, run explicitly with --release --ignored --nocapture"]
    fn labels_600_release_timing() {
        let start = Instant::now();
        let (p, r) = place(realistic_page(), &Options::default());
        let elapsed = start.elapsed();
        verify(&p, &r);
        eprintln!(
            "600 realistic anchors: {:.3} ms, {} ticks, {} leaders, {} overlaps, {} ambiguous",
            elapsed.as_secs_f64() * 1000.0,
            r.counts.with_ticks,
            r.counts.with_leaders,
            r.counts.still_overlapping,
            r.counts.ambiguous
        );
        let svg = format!(
            r#"<svg><g><image href="data:image/png;base64,{}"/><text x="20" y="20">Scale</text><path d="M10 30 L30 30"/></g></svg>"#,
            "A".repeat(10 * 1024 * 1024)
        );
        let mut page = Page::new([0.0, 0.0, 594.0, 420.0]);
        let start = Instant::now();
        decorations(&svg, &mut page);
        let parsing = start.elapsed();
        eprintln!(
            "decorations with 10 MiB base64: {:.3} ms",
            parsing.as_secs_f64() * 1000.0
        );
        assert_eq!(page.hard.len(), 2);
        assert!(elapsed.as_secs_f64() < 0.300, "{elapsed:?}");
    }
}
