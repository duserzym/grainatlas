//! Print layout (plan section 6.8): grains (cropped from the mosaic with a
//! margin) or whole areas packed onto pages by next-fit decreasing height
//! shelves, with labels, scale bars, spots, cut marks and page numbers. Pages
//! are SVG in millimetres; PDF through `svg2pdf` with an embedded font
//! (DejaVu Sans) so the output is identical natively and in the browser.

pub mod pdf;
pub mod sheets;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::scale::round_bar_um;
use crate::segment::Grain;
use crate::spots::{Label, Spot, SpotKind};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Paper {
    pub w_mm: f64,
    pub h_mm: f64,
}

/// Paper presets (portrait).
pub fn paper(name: &str) -> Option<Paper> {
    Some(match name.to_ascii_lowercase().as_str() {
        "letter" => Paper { w_mm: 215.9, h_mm: 279.4 },
        "tabloid" | "11x17" | "ledger" => Paper { w_mm: 279.4, h_mm: 431.8 },
        "a4" => Paper { w_mm: 210.0, h_mm: 297.0 },
        "a3" => Paper { w_mm: 297.0, h_mm: 420.0 },
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Items {
    /// Each grain cropped alone, without its neighbours: an explicit choice only
    /// (see "Layout rule: context first" in docs/print_sheets.md).
    Grains,
    #[default]
    Areas,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LayoutParams {
    /// "letter", "tabloid" (default), "a4", "a3" or "custom".
    pub paper: String,
    pub custom_w_mm: f64,
    pub custom_h_mm: f64,
    pub landscape: bool,
    pub margin_mm: f64,
    /// Micrometres of sample per millimetre of paper; 0 = fit.
    pub um_per_mm: f64,
    pub items: Items,
    /// Areas to include (empty = all with a mosaic).
    pub area_ids: Vec<String>,
    pub grain_margin_um: f64,
    pub show_spots: bool,
    pub show_labels: bool,
    pub show_outlines: bool,
    pub cut_marks: bool,
    pub title: String,
    /// Target print resolution for the embedded images.
    pub dpi: f64,
    /// Order items by grain number (reading order on paper) instead of decreasing height.
    pub by_number: bool,
}

impl Default for LayoutParams {
    fn default() -> Self {
        LayoutParams {
            paper: "tabloid".into(),
            custom_w_mm: 279.4,
            custom_h_mm: 431.8,
            landscape: false,
            margin_mm: 12.0,
            um_per_mm: 0.0,
            items: Items::Areas,
            area_ids: Vec::new(),
            grain_margin_um: 20.0,
            show_spots: true,
            show_labels: true,
            show_outlines: false,
            cut_marks: true,
            title: String::new(),
            dpi: 300.0,
            by_number: false,
        }
    }
}

impl LayoutParams {
    pub fn page(&self) -> Paper {
        let p = paper(&self.paper).unwrap_or(Paper { w_mm: self.custom_w_mm.max(50.0), h_mm: self.custom_h_mm.max(50.0) });
        if self.landscape { Paper { w_mm: p.h_mm, h_mm: p.w_mm } } else { p }
    }
}

/// One area's pixels for printing: the 8-bit base mosaic with its display LUT.
pub struct AreaImage<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub sample: &'a str,
    pub w: usize,
    pub h: usize,
    pub px: &'a [u8],
    pub lut: [u8; 256],
    pub um_per_px: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LayoutResult {
    pub pages: Vec<String>,
    pub um_per_mm: f64,
    pub items: usize,
    pub paper_mm: [f64; 2],
}

/// A rectangle of an area to print, in mosaic micrometres.
#[derive(Clone, Debug)]
struct Item {
    area: usize,
    /// x0, y0, x1, y1 um
    rect: [f64; 4],
    label: String,
    grain: Option<Grain>,
    /// (row, col, rows, cols) when tiled.
    tile: Option<(usize, usize, usize, usize)>,
}

const LABEL_MM: f64 = 5.0;
const BAR_MM: f64 = 5.0;
const GAP_MM: f64 = 6.0;
const HEADER_MM: f64 = 10.0;
const FOOTER_MM: f64 = 12.0;

/// Lays out pages.
pub fn layout(p: &LayoutParams, areas: &[AreaImage], grains: &BTreeMap<String, Vec<Grain>>, spots: &[Spot], labels: &[Label]) -> LayoutResult {
    let page = p.page();
    let pw = page.w_mm - 2.0 * p.margin_mm;
    let ph = page.h_mm - 2.0 * p.margin_mm - HEADER_MM - FOOTER_MM;
    let selected: Vec<usize> = (0..areas.len()).filter(|&i| p.area_ids.is_empty() || p.area_ids.iter().any(|a| a == areas[i].id)).collect();
    let mut items: Vec<Item> = Vec::new();
    for &ai in &selected {
        let a = &areas[ai];
        match p.items {
            Items::Areas => items.push(Item { area: ai, rect: [0.0, 0.0, a.w as f64 * a.um_per_px, a.h as f64 * a.um_per_px], label: format!("{} ({})", a.name, a.id), grain: None, tile: None }),
            Items::Grains => {
                for g in grains.get(a.id).map(|v| v.as_slice()).unwrap_or(&[]).iter().filter(|g| g.is_grain()) {
                    let m = p.grain_margin_um;
                    let rect = [(g.bbox_um[0] - m).max(0.0), (g.bbox_um[1] - m).max(0.0), (g.bbox_um[2] + m).min(a.w as f64 * a.um_per_px), (g.bbox_um[3] + m).min(a.h as f64 * a.um_per_px)];
                    items.push(Item { area: ai, rect, label: format!("{} #{}", a.sample, g.number), grain: Some(g.clone()), tile: None });
                }
            }
        }
    }
    // Scale: fit = the largest item fits the printable area (areas) or a third of its width (grains).
    let um_per_mm = if p.um_per_mm > 0.0 {
        p.um_per_mm
    } else {
        let mut s: f64 = 1.0;
        for it in &items {
            let (w, h) = (it.rect[2] - it.rect[0], it.rect[3] - it.rect[1]);
            let need = match p.items {
                Items::Areas => (w / pw).max(h / (ph - LABEL_MM - BAR_MM)),
                Items::Grains => (w / (pw / 3.0)).max(h / ((ph - LABEL_MM - BAR_MM) / 3.0)),
            };
            s = s.max(need);
        }
        nice_scale(s)
    };
    // Tile items larger than the printable area (10 mm overlap).
    let maxw = pw * um_per_mm;
    let maxh = (ph - LABEL_MM - BAR_MM) * um_per_mm;
    let ov = 10.0 * um_per_mm;
    let mut placed_items: Vec<Item> = Vec::new();
    for it in items {
        let (w, h) = (it.rect[2] - it.rect[0], it.rect[3] - it.rect[1]);
        if w <= maxw + 1e-6 && h <= maxh + 1e-6 {
            placed_items.push(it);
            continue;
        }
        let cols = ((w - ov) / (maxw - ov)).ceil().max(1.0) as usize;
        let rows = ((h - ov) / (maxh - ov)).ceil().max(1.0) as usize;
        let (tw, th) = ((w + (cols - 1) as f64 * ov) / cols as f64, (h + (rows - 1) as f64 * ov) / rows as f64);
        for r in 0..rows {
            for c in 0..cols {
                let x0 = it.rect[0] + c as f64 * (tw - ov);
                let y0 = it.rect[1] + r as f64 * (th - ov);
                placed_items.push(Item { rect: [x0, y0, (x0 + tw).min(it.rect[2]), (y0 + th).min(it.rect[3])], label: format!("{} ({}/{})", it.label, r * cols + c + 1, rows * cols), tile: Some((r, c, rows, cols)), ..it.clone() });
            }
        }
    }
    // Next-fit decreasing height shelves.
    let mut order: Vec<usize> = (0..placed_items.len()).collect();
    let hmm = |it: &Item| (it.rect[3] - it.rect[1]) / um_per_mm + LABEL_MM + BAR_MM;
    let wmm = |it: &Item| ((it.rect[2] - it.rect[0]) / um_per_mm).max(22.0);
    if !p.by_number {
        order.sort_by(|&a, &b| hmm(&placed_items[b]).total_cmp(&hmm(&placed_items[a])).then(a.cmp(&b)));
    }
    let mut pages: Vec<Vec<(usize, f64, f64)>> = vec![Vec::new()];
    let (mut x, mut y, mut shelf_h) = (0.0, 0.0, 0.0);
    for i in order {
        let it = &placed_items[i];
        let (w, h) = (wmm(it), hmm(it));
        if x > 0.0 && x + w > pw {
            x = 0.0;
            y += shelf_h + GAP_MM;
            shelf_h = 0.0;
        }
        if y > 0.0 && y + h > ph {
            pages.push(Vec::new());
            x = 0.0;
            y = 0.0;
            shelf_h = 0.0;
        }
        pages.last_mut().expect("page").push((i, x, y));
        x += w + GAP_MM;
        shelf_h = f64::max(shelf_h, h);
    }
    if pages.last().is_some_and(|p| p.is_empty()) && pages.len() > 1 {
        pages.pop();
    }
    let n = pages.len();
    let svgs: Vec<String> = pages
        .iter()
        .enumerate()
        .map(|(pi, list)| page_svg(p, page, um_per_mm, pi + 1, n, list, &placed_items, areas, spots, labels))
        .collect();
    LayoutResult { pages: svgs, um_per_mm, items: placed_items.len(), paper_mm: [page.w_mm, page.h_mm] }
}

/// The next "nice" scale (1-2-2.5-5 series) at or above `s` um/mm.
pub fn nice_scale(s: f64) -> f64 {
    let mut d = 1.0;
    while d * 10.0 <= s {
        d *= 10.0;
    }
    for m in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if m * d >= s - 1e-9 {
            return m * d;
        }
    }
    10.0 * d
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[allow(clippy::too_many_arguments)]
fn page_svg(p: &LayoutParams, page: Paper, um_per_mm: f64, page_no: usize, pages: usize, list: &[(usize, f64, f64)], items: &[Item], areas: &[AreaImage], spots: &[Spot], labels: &[Label]) -> String {
    let (w, h) = (page.w_mm, page.h_mm);
    let ox = p.margin_mm;
    let oy = p.margin_mm + HEADER_MM;
    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}" font-family="DejaVu Sans, sans-serif"><rect width="{w}" height="{h}" fill="white"/>"#
    );
    // Header.
    let title = if p.title.is_empty() { "grainatlas" } else { &p.title };
    let _ = write!(s, r#"<text x="{ox}" y="{}" font-size="4.2" fill="black">{}</text>"#, p.margin_mm + 5.0, esc(title));
    let _ = write!(s, r##"<text x="{}" y="{}" font-size="3" fill="#444" text-anchor="end">1 mm = {} µm</text>"##, w - p.margin_mm, p.margin_mm + 5.0, fmt_num(um_per_mm));
    for &(i, x, y) in list {
        let it = &items[i];
        let a = &areas[it.area];
        let (x, y) = (ox + x, oy + y);
        let wmm = (it.rect[2] - it.rect[0]) / um_per_mm;
        let hmm = (it.rect[3] - it.rect[1]) / um_per_mm;
        let iy = y + LABEL_MM;
        if p.show_labels {
            let _ = write!(s, r#"<text x="{x}" y="{}" font-size="3.2" fill="black">{}</text>"#, y + 3.6, esc(&it.label));
        }
        // Image crop at the print resolution.
        let (png, _, _) = crop_png(a, it.rect, um_per_mm, p.dpi);
        let _ = write!(s, r#"<image x="{x}" y="{iy}" width="{wmm}" height="{hmm}" preserveAspectRatio="none" xlink:href="data:image/png;base64,{}"/>"#, base64(&png));
        let tr = |ux: f64, uy: f64| (x + (ux - it.rect[0]) / um_per_mm, iy + (uy - it.rect[1]) / um_per_mm);
        let _ = write!(s, r#"<g><clipPath id="c{i}"><rect x="{x}" y="{iy}" width="{wmm}" height="{hmm}"/></clipPath></g><g clip-path="url(#c{i})">"#);
        if p.show_outlines
            && let Some(g) = &it.grain
        {
            let pts: Vec<String> = g.polygon_um.iter().map(|q| {
                let (a, b) = tr(q[0], q[1]);
                format!("{a:.3},{b:.3}")
            }).collect();
            let _ = write!(s, r##"<polygon points="{}" fill="none" stroke="#56b4e9" stroke-width="0.2"/>"##, pts.join(" "));
        }
        if p.show_spots {
            for sp in spots.iter().filter(|sp| sp.area_id == a.id) {
                let inside = sp.x_um >= it.rect[0] && sp.x_um <= it.rect[2] && sp.y_um >= it.rect[1] && sp.y_um <= it.rect[3];
                if !inside {
                    continue;
                }
                let (cx, cy) = tr(sp.x_um, sp.y_um);
                match (sp.kind, sp.x2_um, sp.y2_um) {
                    (SpotKind::Line, Some(x2), Some(y2)) => {
                        let (ex, ey) = tr(x2, y2);
                        let _ = write!(s, r##"<line x1="{cx:.3}" y1="{cy:.3}" x2="{ex:.3}" y2="{ey:.3}" stroke="#3fd3a4" stroke-opacity="0.5" stroke-width="{:.3}" stroke-linecap="round"/>"##, sp.diameter_um / um_per_mm);
                    }
                    (SpotKind::Raster, Some(x2), Some(y2)) => {
                        let (ex, ey) = tr(x2, y2);
                        let _ = write!(s, r##"<rect x="{cx:.3}" y="{cy:.3}" width="{:.3}" height="{:.3}" fill="none" stroke="#cc79a7" stroke-width="0.25"/>"##, ex - cx, ey - cy);
                    }
                    _ => {
                        let _ = write!(s, r##"<circle cx="{cx:.3}" cy="{cy:.3}" r="{:.3}" fill="none" stroke="#ff8c42" stroke-width="0.25"/>"##, sp.diameter_um / 2.0 / um_per_mm);
                    }
                }
                let _ = write!(s, r##"<text x="{:.3}" y="{:.3}" font-size="1.8" fill="#ffd27a" stroke="black" stroke-width="0.25" paint-order="stroke">{}</text>"##, cx + sp.diameter_um / 2.0 / um_per_mm + 0.5, cy + 0.6, esc(&sp.name));
            }
            for l in labels.iter().filter(|l| l.area_id == a.id) {
                let (lx, ly) = tr(l.x_um, l.y_um);
                let _ = write!(s, r##"<text x="{lx:.3}" y="{ly:.3}" font-size="2" fill="white" stroke="black" stroke-width="0.25" paint-order="stroke">{}</text>"##, esc(&l.text));
            }
        }
        s.push_str("</g>");
        // Item scale bar under the image (grain bar, or a round length for areas).
        let bar_um = it.grain.as_ref().map_or_else(|| round_bar_um(wmm * um_per_mm / 4.0), |g| g.scale_bar_um);
        let bl = bar_um / um_per_mm;
        let by = iy + hmm + 2.2;
        let _ = write!(s, r#"<path d="M{x} {by} h{bl:.3}" stroke="black" stroke-width="0.5"/><text x="{:.3}" y="{:.3}" font-size="2.2" fill="black">{} µm</text>"#, x + bl + 1.0, by + 0.8, fmt_num(bar_um));
        if p.cut_marks {
            for (cx, cy, dx, dy) in [(x, iy, -1.0, -1.0), (x + wmm, iy, 1.0, -1.0), (x, iy + hmm, -1.0, 1.0), (x + wmm, iy + hmm, 1.0, 1.0)] {
                let _ = write!(
                    s,
                    r##"<path d="M{} {cy} h{} M{cx} {} v{}" stroke="#666" stroke-width="0.12"/>"##,
                    cx + dx * 0.8,
                    dx * 3.0,
                    cy + dy * 0.8,
                    dy * 3.0
                );
            }
        }
        if let Some((_, _, rows, cols)) = it.tile
            && rows * cols > 1
        {
            // Registration marks at the four corners of a tile of an oversized item.
            for (cx, cy) in [(x + 5.0, iy + 5.0), (x + wmm - 5.0, iy + 5.0), (x + 5.0, iy + hmm - 5.0), (x + wmm - 5.0, iy + hmm - 5.0)] {
                let _ = write!(s, r##"<g stroke="#e3b341" stroke-width="0.2" fill="none"><circle cx="{cx}" cy="{cy}" r="2"/><path d="M{} {cy} h6 M{cx} {} v6"/></g>"##, cx - 3.0, cy - 3.0);
            }
        }
    }
    // Footer: 10 mm bar at paper scale, um bar at image scale, page number.
    let fy = h - p.margin_mm - 4.0;
    let _ = write!(s, r#"<path d="M{ox} {fy} h10 M{ox} {} v2 M{} {} v2" stroke="black" stroke-width="0.4" id="paper-bar"/><text x="{ox}" y="{}" font-size="2.6" fill="black">10 mm (paper)</text>"#, fy - 1.0, ox + 10.0, fy - 1.0, fy + 4.0);
    let ub = round_bar_um(40.0 * um_per_mm);
    let ul = ub / um_per_mm;
    let ux = ox + 40.0;
    let _ = write!(s, r#"<path d="M{ux} {fy} h{ul:.3} M{ux} {} v2 M{:.3} {} v2" stroke="black" stroke-width="0.4" id="image-bar"/><text x="{ux}" y="{}" font-size="2.6" fill="black">{} µm (sample)</text>"#, fy - 1.0, ux + ul, fy - 1.0, fy + 4.0, fmt_num(ub));
    let _ = write!(s, r#"<text x="{}" y="{}" font-size="3" fill="black" text-anchor="end">{page_no} / {pages}</text>"#, w - p.margin_mm, fy + 3.0);
    s.push_str("</svg>");
    s
}

fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{v:.1}") }
}

/// Crops a rectangle (um) of an area at the print resolution, LUT applied, as PNG.
fn crop_png(a: &AreaImage, rect: [f64; 4], um_per_mm: f64, dpi: f64) -> (Vec<u8>, usize, usize) {
    let x0 = (rect[0] / a.um_per_px).floor().max(0.0) as usize;
    let y0 = (rect[1] / a.um_per_px).floor().max(0.0) as usize;
    let x1 = ((rect[2] / a.um_per_px).ceil() as usize).min(a.w);
    let y1 = ((rect[3] / a.um_per_px).ceil() as usize).min(a.h);
    let (cw, ch) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
    // Image pixels per paper mm vs. target pixels per mm.
    let px_per_mm = um_per_mm / a.um_per_px;
    let target = dpi / 25.4;
    let k = ((px_per_mm / target).floor() as usize).max(1);
    let (ow, oh) = (cw.div_ceil(k), ch.div_ceil(k));
    let mut out = vec![0u8; ow * oh];
    for y in 0..oh {
        for x in 0..ow {
            let (mut sum, mut n) = (0u32, 0u32);
            for yy in y0 + y * k..(y0 + (y + 1) * k).min(y1) {
                for xx in x0 + x * k..(x0 + (x + 1) * k).min(x1) {
                    sum += a.lut[a.px[yy * a.w + xx] as usize] as u32;
                    n += 1;
                }
            }
            out[y * ow + x] = (sum / n.max(1)) as u8;
        }
    }
    (crate::image::encode_png_gray(ow, oh, &out), ow, oh)
}

pub fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}
