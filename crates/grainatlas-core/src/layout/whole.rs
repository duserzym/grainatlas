//! A continuous mosaic on one sheet. No segmentation, cuts or rearrangement.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::sheets::{
    BandRecord, FOOTER_MM, PrintRegistration, SheetArea, SheetBody, SheetMeta, SheetParams,
};
use super::{AreaImage, Paper, base64, esc, paper};
use crate::scale::round_bar_um;
use crate::{Error, Result};

pub const HEADER_MM: f64 = 30.0;
/// Space for ticks, cell labels and stage corner coordinates on each edge.
pub const GUTTER_MM: f64 = 12.0;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WholeParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub print_grains: Option<Vec<crate::segment::Grain>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub print_spots: Option<Vec<crate::spots::Spot>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub print_labels: Option<super::labels::Options>,
    pub paper: String,
    /// "auto", "landscape" or "portrait". Only the paper rotates.
    pub orientation: String,
    pub margin_mm: f64,
    #[serde(with = "super::sheets::scale_serde")]
    pub um_per_mm: f64,
    pub dpi: f64,
    pub grid: bool,
    pub title: String,
    pub lab: String,
    pub registration_file: String,
    pub area_id: String,
    /// Optional output stem, without extension or directory.
    pub name: String,
    pub project: String,
    /// Optional print date override; empty uses the current UTC date.
    pub printed: String,
}

impl Default for WholeParams {
    fn default() -> Self {
        Self {
            print_spots: None, print_labels: None, print_grains: None,
            paper: "tabloid".into(),
            orientation: "auto".into(),
            margin_mm: SheetParams::default().margin_mm,
            um_per_mm: 0.0,
            dpi: 600.0,
            grid: true,
            title: String::new(),
            lab: String::new(),
            registration_file: String::new(),
            area_id: String::new(),
            name: String::new(),
            project: String::new(),
            printed: String::new(),
        }
    }
}

impl WholeParams {
    pub fn output_stem(&self, package: &str) -> String {
        if self.name.is_empty() {
            format!("{package}_whole_{}", self.paper)
        } else {
            self.name.clone()
        }
    }

    /// Select before reading any pixels. Multiple areas always require an explicit id.
    pub fn select_area<'a>(
        &self,
        areas: &'a [crate::project::AreaRecord],
    ) -> Result<&'a crate::project::AreaRecord> {
        if self.area_id.is_empty() {
            match areas {
                [a] => Ok(a),
                [] => Err(Error::Invalid("no area to print".into())),
                _ => Err(Error::Invalid(
                    "whole-area printing requires area_id (--area) when several areas exist".into(),
                )),
            }
        } else {
            areas
                .iter()
                .find(|a| a.id == self.area_id)
                .ok_or_else(|| Error::Invalid(format!("unknown area {}", self.area_id)))
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WholeStats {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot_labels: Option<Vec<super::labels::Report>>,
    pub sheets: usize,
    pub orientation: String,
    pub um_per_mm: f64,
    /// Unrounded minimum that fits, before R10 rounding.
    pub minimum_um_per_mm: f64,
    pub image_rect_mm: [f64; 4],
    pub gutter_rect_mm: [f64; 4],
    pub embedded_px: [usize; 2],
    pub embedded_dpi: [f64; 2],
    pub source_pixel_um: f64,
    pub smallest_detail_300_um: f64,
    pub smallest_detail_600_um: f64,
    /// TL, TR, BL, BR, using AreaRecord::stage_of; absent when stage data is unavailable.
    pub stage_corners_mm: [Option<[f64; 2]>; 4],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WholeResult {
    pub pages: Vec<String>,
    pub registration: PrintRegistration,
    pub stats: WholeStats,
}

/// Next coarser R10 scale, extended through every decade (unlike the grain-size range).
pub fn r10_ceil(value: f64) -> f64 {
    let decade = 10.0_f64.powf(value.log10().floor());
    [1.0, 1.25, 1.6, 2.0, 2.5, 3.2, 4.0, 5.0, 6.3, 8.0, 10.0]
        .into_iter()
        .map(|m| m * decade)
        .find(|v| *v >= value * (1.0 - 1e-12))
        .unwrap_or(10.0 * decade)
}

/// Excel-style column names: A..Z, AA..AZ, ... . Row and column indices start at zero.
pub fn grid_column(mut column: usize) -> String {
    let mut chars = Vec::new();
    loop {
        chars.push((b'A' + (column % 26) as u8) as char);
        if column < 26 {
            break;
        }
        column = column / 26 - 1;
    }
    chars.into_iter().rev().collect()
}

/// Cell bounds in mosaic um, clipped to the full mosaic at right/bottom.
pub fn grid_cell_rect(reg: &PrintRegistration, label: &str) -> Option<[f64; 4]> {
    let step = reg.grid_step_um.filter(|v| v.is_finite() && *v > 0.0)?;
    let origin = reg.grid_origin_um?;
    let split = label.find(|c: char| !c.is_ascii_uppercase())?;
    if split == 0 {
        return None;
    }
    let mut col = 0usize;
    for c in label[..split].bytes() {
        col = col.checked_mul(26)?.checked_add((c - b'A' + 1) as usize)?;
    }
    let row = label[split..].parse::<usize>().ok()?.checked_sub(1)?;
    let band = reg.bands.first()?;
    let x = origin[0] + (col - 1) as f64 * step;
    let y = origin[1] + row as f64 * step;
    let r = band.mosaic_rect_um;
    (x >= r[0] && y >= r[1] && x < r[2] && y < r[3]).then_some([
        x,
        y,
        (x + step).min(r[2]),
        (y + step).min(r[3]),
    ])
}

fn body(page: Paper, m: f64) -> [f64; 4] {
    [
        m,
        m + HEADER_MM + 1.0,
        page.w_mm - m,
        page.h_mm - m - FOOTER_MM,
    ]
}

fn minimum(page: Paper, m: f64, size: [f64; 2]) -> f64 {
    let b = body(page, m);
    (size[0] / (b[2] - b[0] - 2.0 * GUTTER_MM)).max(size[1] / (b[3] - b[1] - 2.0 * GUTTER_MM))
}

/// One page and the existing print_sheet frame; convert pages with pdf::pdf_from_svgs.
pub fn whole_sheet(p: &WholeParams, area: &SheetArea, meta: &SheetMeta) -> Result<WholeResult> {
    whole_sheet_with_image(p, area, meta, &|a, w, h| area_png(a, w, h))
}
/// Same physical geometry and stats, with a preview image provider.
pub fn whole_sheet_with_image(p: &WholeParams, area: &SheetArea, meta: &SheetMeta, image: &impl Fn(&AreaImage, usize, usize) -> Vec<u8>) -> Result<WholeResult> {
    let a = &area.img;
    if a.w == 0
        || a.h == 0
        || a.w.checked_mul(a.h) != Some(a.px.len())
        || !a.um_per_px.is_finite()
        || a.um_per_px <= 0.0
    {
        return Err(Error::Invalid(
            "invalid whole-area image dimensions or pixel size".into(),
        ));
    }
    if !p.area_id.is_empty() && p.area_id != area.rec.id {
        return Err(Error::Invalid(format!(
            "area_id {} does not match {}",
            p.area_id, area.rec.id
        )));
    }
    if !p.margin_mm.is_finite()
        || p.margin_mm < 0.0
        || !p.um_per_mm.is_finite()
        || p.um_per_mm < 0.0
        || !p.dpi.is_finite()
        || p.dpi <= 0.0
    {
        return Err(Error::Invalid(
            "margin and scale must be finite and nonnegative; dpi must be positive and finite"
                .into(),
        ));
    }
    if area.rec.stage_sign.iter().any(|v| *v != -1.0 && *v != 1.0) {
        return Err(Error::Invalid(
            "stage_sign must contain -1 or 1 for each axis".into(),
        ));
    }
    let portrait =
        paper(&p.paper).ok_or_else(|| Error::Invalid(format!("unknown paper {}", p.paper)))?;
    let landscape = Paper {
        w_mm: portrait.h_mm,
        h_mm: portrait.w_mm,
    };
    let size = [a.display_size()[0] as f64 * a.um_per_px, a.display_size()[1] as f64 * a.um_per_px];
    if !size.iter().all(|v| v.is_finite())
        || portrait.w_mm - 2.0 * p.margin_mm <= 2.0 * GUTTER_MM + HEADER_MM + 1.0 + FOOTER_MM
    {
        return Err(Error::Invalid(
            "no room for the image and gutters inside the sheet body".into(),
        ));
    }
    let page = match p.orientation.as_str() {
        "portrait" => portrait,
        "landscape" => landscape,
        "auto" => {
            if minimum(landscape, p.margin_mm, size) <= minimum(portrait, p.margin_mm, size) {
                landscape
            } else {
                portrait
            }
        }
        _ => {
            return Err(Error::Invalid(
                "orientation must be auto, landscape or portrait".into(),
            ));
        }
    };
    let need = minimum(page, p.margin_mm, size);
    let fits = r10_ceil(need);
    let k = if p.um_per_mm == 0.0 {
        fits
    } else {
        p.um_per_mm
    };
    if !k.is_finite() || k < need * (1.0 - 1e-12) {
        return Err(Error::Invalid(format!(
            "scale {} um/mm does not fit; use {} um/mm or coarser",
            p.um_per_mm, fits
        )));
    }
    let b = body(page, p.margin_mm);
    let (w, h) = (size[0] / k, size[1] / k);
    let (x, y) = ((b[0] + b[2] - w) / 2.0, (b[1] + b[3] - h) / 2.0);
    let rect = [x, y, x + w, y + h];
    let ow = ((w * p.dpi / 25.4).round() as usize).clamp(1, a.display_size()[0]);
    let oh = ((h * p.dpi / 25.4).round() as usize).clamp(1, a.display_size()[1]);
    let png = image(a, ow, oh);
    let step = p.grid.then(|| round_bar_um(40.0 * k));
    let corners = [[0.0, 0.0], [size[0], 0.0], [0.0, size[1]], size];
    let mut stats = WholeStats {
        spot_labels: None,
        sheets: 1,
        orientation: if page.w_mm > page.h_mm {
            "landscape"
        } else {
            "portrait"
        }
        .into(),
        um_per_mm: k,
        minimum_um_per_mm: need,
        image_rect_mm: rect,
        gutter_rect_mm: [
            x - GUTTER_MM,
            y - GUTTER_MM,
            x + w + GUTTER_MM,
            y + h + GUTTER_MM,
        ],
        embedded_px: [ow, oh],
        embedded_dpi: [ow as f64 * 25.4 / w, oh as f64 * 25.4 / h],
        source_pixel_um: a.um_per_px,
        smallest_detail_300_um: 2.0 * 25.4 / 300.0 * k,
        smallest_detail_600_um: 2.0 * 25.4 / 600.0 * k,
        stage_corners_mm: corners.map(|c| {
            let c=crate::project::image_edits::apply(crate::project::image_edits::inverse(a.image_transform()),c);
            area.rec
                .stage_of(c[0], c[1])
                .map(|v| [v[0] / 1e6, v[1] / 1e6])
        }),
    };
    let mut reg = PrintRegistration {
        spot_labels: None,
        project_uuid: None,
        project_path: String::new(),
        frame: "print_sheet".into(),
        note: "paper sheet <-> grainatlas mosaic; not the laser stage registration".into(),
        package: meta.package.clone(),
        project: p.project.clone(),
        paper: format!(
            "{} {} {} x {} mm",
            p.paper, stats.orientation, page.w_mm, page.h_mm
        ),
        paper_mm: [page.w_mm, page.h_mm],
        um_per_mm: k,
        scale: if p.um_per_mm == 0.0 { "auto" } else { "fixed" }.into(),
        layout: "whole_area".into(),
        grid_step_um: step,
        grid_origin_um: step.map(|_| [0.0, 0.0]),
        auto_scale: None,
        spot_sizes_left_off_um: Vec::new(),
        sheet_origin: "sheet top-left corner, mm, y down".into(),
        mapping: "mosaic_um = mosaic_rect_um[0:2] + (sheet_mm - sheet_rect_mm[0:2]) * um_per_mm"
            .into(),
        registration_file: if p.registration_file.is_empty() {
            format!("{}_print_registration.json", p.output_stem(&meta.package))
        } else {
            p.registration_file.clone()
        },
        body_rect_mm: b,
        sheet_bodies: vec![SheetBody {
            sheet: 1,
            header_mm: HEADER_MM,
            body_rect_mm: b,
        }],
        strays: Vec::new(),
        strip_blocks: Vec::new(),
        left_off: Vec::new(),
        edits: None,
        bands: vec![BandRecord {
            transform: None,
            sheet: 1,
            sample: a.sample.into(),
            area: area.rec.id.clone(),
            row: 1,
            grains: Vec::new(),
            context: Vec::new(),
            repeated_um: Vec::new(),
            cut_sources: Vec::new(),
            sheet_rect_mm: rect,
            mosaic_rect_um: [0.0, 0.0, size[0], size[1]],
            background: "kept".into(),
            paper_to_mount: a.edits.map(|_| super::sheets::paper_to_mount(a, [0.0,0.0,size[0],size[1]], [rect[0],rect[1]], k)),
            image_crop: a.edits.and_then(|e| e.crop.clone()),
        }],
    };
    let mut s = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{}mm" height="{}mm" viewBox="0 0 {} {}" font-family="DejaVu Sans, sans-serif"><rect width="100%" height="100%" fill="white"/>"#,
        page.w_mm, page.h_mm, page.w_mm, page.h_mm
    );
    header(&mut s, p, a, meta, &stats, page);
    let _ = write!(
        s,
        r#"<image id="whole-mosaic" x="{x}" y="{y}" width="{w}" height="{h}" preserveAspectRatio="none" xlink:href="data:image/png;base64,{}"/>"#,
        base64(&png)
    );
    super::sheets::image_axes(&mut s, a, rect);
    coordinates(&mut s, rect, size, k, step);
    orientation(&mut s, area, &stats, page.w_mm - p.margin_mm);
    let fy = page.h_mm - p.margin_mm - 2.5;
    text(
        &mut s,
        p.margin_mm,
        fy,
        2.1,
        "start",
        &format!("registration {} · frame print_sheet", reg.registration_file),
    );
    let printed = if p.printed.is_empty() {
        today_utc()
    } else {
        p.printed.clone()
    };
    text(
        &mut s,
        p.margin_mm,
        fy + 2.4,
        2.1,
        "start",
        &format!("printed {printed}"),
    );
    text(
        &mut s,
        page.w_mm - p.margin_mm,
        fy + 2.4,
        2.1,
        "end",
        "page 1/1",
    );
    s.push_str("</svg>");
    if let Some(spots)=&p.print_spots {
        // Keep the whole-area gutter free for its labels/ticks and coordinates.
        let mut labels=super::labels::Page::new(b);
        let tr=[1.0/k,0.0,0.0,1.0/k,x,y];
        let image=super::labels::Image{area:a.id,occurrence:format!("whole/{}",a.id),crop:[0.0,0.0,size[0],size[1]],display_to_paper:tr,mount_to_display:a.image_transform(),edits:a.edits,repeated:vec![]};
        labels.image(&image,spots);
        for grain in p.print_grains.as_deref().unwrap_or(&[]) { let display=grain.polygon_um.iter().map(|point|super::sheets::arrange::point(a.image_transform(),*point)).collect::<Vec<_>>();labels.grain(&image,&display); }
        super::labels::decorations(&s,&mut labels);
        let report=super::labels::render(&mut s,labels,&p.print_labels.clone().unwrap_or_default(),1);
        reg.spot_labels=Some(vec![report.clone()]);stats.spot_labels=Some(vec![report]);
    }
    Ok(WholeResult {
        pages: vec![s],
        registration: reg,
        stats,
    })
}

/// Exact box-area filter, with fractional weights at source pixel boundaries. Applies the
/// tone LUT before averaging, and allocates only the downsampled grey output.
fn area_png(a: &AreaImage, ow: usize, oh: usize) -> Vec<u8> {
    let size=a.display_size();
    let inv=crate::project::image_edits::inverse(a.image_transform());
    let mut px = vec![0; ow * oh];
    let (sx, sy) = (size[0] as f64 / ow as f64, size[1] as f64 / oh as f64);
    for y in 0..oh {
        let (y0, y1) = (y as f64 * sy, ((y + 1) as f64 * sy).min(size[1] as f64));
        for x in 0..ow {
            let (x0, x1) = (x as f64 * sx, ((x + 1) as f64 * sx).min(size[0] as f64));
            let mut sum = 0.0;
            for yy in y0.floor() as usize..(y1.ceil() as usize).min(size[1]) {
                let wy = (y1.min((yy + 1) as f64) - y0.max(yy as f64)).max(0.0);
                for xx in x0.floor() as usize..(x1.ceil() as usize).min(size[0]) {
                    let wx = (x1.min((xx + 1) as f64) - x0.max(xx as f64)).max(0.0);
                    sum += a.sample_mount(crate::project::image_edits::apply(inv,[(xx as f64+0.5)*a.um_per_px,(yy as f64+0.5)*a.um_per_px]),255) as f64 * wx * wy;
                }
            }
            px[y * ow + x] = (sum / ((x1 - x0) * (y1 - y0))).round() as u8;
        }
    }
    crate::image::encode_png_gray(ow, oh, &px)
}

// uuid uses the native clock or JavaScript Date on wasm, as it does for project ids.
fn today_utc() -> String {
    let (seconds, _) = uuid::Timestamp::now(uuid::NoContext).to_unix();
    let z = (seconds / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} UTC")
}

fn text(s: &mut String, x: f64, y: f64, size: f64, anchor: &str, value: &str) {
    let _ = write!(
        s,
        r##"<text x="{x}" y="{y}" font-size="{size}" text-anchor="{anchor}" fill="#333">{}</text>"##,
        esc(value)
    );
}

fn header(
    s: &mut String,
    p: &WholeParams,
    a: &AreaImage,
    meta: &SheetMeta,
    st: &WholeStats,
    page: Paper,
) {
    let m = p.margin_mm;
    text(
        s,
        m,
        m + 5.0,
        4.5,
        "start",
        if p.title.is_empty() {
            &meta.package
        } else {
            &p.title
        },
    );
    text(
        s,
        m,
        m + 9.0,
        2.7,
        "start",
        &if p.lab.trim().is_empty() {
            format!("{} · area {}", a.sample, a.name)
        } else {
            format!("{} · area {} · {}", a.sample, a.name, p.lab)
        },
    );
    let k = st.um_per_mm;
    text(
        s,
        m,
        m + 13.0,
        2.7,
        "start",
        &format!("1 mm on paper = {k} um of sample ({k} um/mm)"),
    );
    text(
        s,
        m,
        m + 17.0,
        2.2,
        "start",
        &format!(
            "{} · {} · {} kV · magnification {} · {:.3} um/px · {} tiles · {} {}–{}",
            meta.instrument,
            meta.detector,
            meta.kv.map_or("—".into(), |v| v.to_string()),
            meta.magnification,
            a.um_per_px,
            meta.tiles,
            meta.date,
            meta.t_start,
            meta.t_end
        ),
    );
    text(
        s,
        m,
        m + 21.0,
        2.2,
        "start",
        &format!(
            "Mosaic {} x {} px · {:.3} x {:.3} mm of sample",
            a.w,
            a.h,
            a.display_size()[0] as f64 * a.um_per_px / 1000.0,
            a.display_size()[1] as f64 * a.um_per_px / 1000.0
        ),
    );
    text(
        s,
        m,
        m + 25.0,
        2.2,
        "start",
        &format!(
            "smallest printed detail about {:.0} um (300 dpi) / {:.0} um (600 dpi); source pixel {:.2} um",
            st.smallest_detail_300_um, st.smallest_detail_600_um, a.um_per_px
        ),
    );
    let bar = round_bar_um(50.0 * k);
    let bw = bar / k;
    let bx = page.w_mm - m - bw;
    let by = m + 10.0;
    let _ = write!(
        s,
        r#"<path id="sample-bar" d="M{bx} {by} h{bw} M{bx} {} v2 M{} {} v2" stroke="black" stroke-width="0.35"/>"#,
        by - 1.0,
        bx + bw,
        by - 1.0
    );
    text(
        s,
        bx + bw / 2.0,
        by - 2.0,
        2.2,
        "middle",
        &format!("{} mm sample", bar / 1000.0),
    );
}

fn coordinates(s: &mut String, r: [f64; 4], size: [f64; 2], k: f64, step: Option<f64>) {
    let [x, y, right, bottom] = r;
    for axis in 0..2 {
        for n in 0..=(size[axis] / 1000.0).floor() as usize {
            let t = r[axis] + n as f64 * 1000.0 / k;
            let major = n % 5 == 0;
            let len = if major { 2.0 } else { 1.0 };
            let d = if axis == 0 {
                format!("M{t} {y} v-{} M{t} {bottom} v{len}", len)
            } else {
                format!("M{x} {t} h-{} M{right} {t} h{len}", len)
            };
            let _ = write!(
                s,
                r##"<path class="edge-tick" d="{d}" stroke="#444" stroke-width="0.15"/>"##
            );
            if major {
                if axis == 0 {
                    text(s, t, y - 3.0, 2.0, "middle", &n.to_string());
                    text(s, t, bottom + 4.0, 2.0, "middle", &n.to_string());
                } else {
                    text(s, x - 3.0, t + 0.7, 2.0, "end", &n.to_string());
                    text(s, right + 3.0, t + 0.7, 2.0, "start", &n.to_string());
                }
            }
        }
    }
    if let Some(step) = step {
        let mut path = String::new();
        for axis in 0..2 {
            let cells = (size[axis] / step).ceil() as usize;
            for i in 0..cells {
                let at = r[axis] + i as f64 * step / k;
                let mid = r[axis]
                    + ((i as f64 + 0.5) * step).min((i as f64 * step + size[axis]) / 2.0) / k;
                if i > 0 {
                    if axis == 0 {
                        let _ = write!(path, "M{at} {y} V{bottom} ");
                    } else {
                        let _ = write!(path, "M{x} {at} H{right} ");
                    }
                }
                let label = if axis == 0 {
                    grid_column(i)
                } else {
                    (i + 1).to_string()
                };
                if axis == 0 {
                    text(s, mid, y - 6.5, 2.6, "middle", &label);
                    text(s, mid, bottom + 7.5, 2.6, "middle", &label);
                } else {
                    text(s, x - 8.0, mid + 0.9, 2.6, "middle", &label);
                    text(s, right + 8.0, mid + 0.9, 2.6, "middle", &label);
                }
            }
        }
        let _ = write!(
            s,
            r##"<g class="whole-grid" fill="none"><path d="{path}" stroke="#eee" stroke-width="0.25" opacity="0.5"/><path d="{path}" stroke="#888" stroke-width="0.1" opacity="0.65"/></g>"##
        );
    }
}

fn orientation(s: &mut String, area: &SheetArea, st: &WholeStats, page_right: f64) {
    let [x, y, right, bottom] = st.image_rect_mm;
    for (i, corner) in st.stage_corners_mm.iter().enumerate() {
        let label = corner.map_or("stage X/Y unavailable".into(), |c| {
            format!("stage X {:.3}, Y {:.3} mm", c[0], c[1])
        });
        text(
            s,
            if i % 2 == 0 { x } else { right },
            if i < 2 { y - 10.0 } else { bottom + 11.0 },
            2.0,
            if i % 2 == 0 { "start" } else { "end" },
            &label,
        );
    }
    // In the right-hand margin beside the bottom-right image corner, clear of the mosaic;
    // the thin light halo keeps the arrows legible if a narrow page puts them over an image.
    // The labels reach about 15 mm right of the origin.
    let (ox, oy) = if right + 27.0 + 15.0 <= page_right {
        (right + 27.0, bottom - 12.0)
    } else {
        (right - 17.0, bottom - 17.0)
    };
    s.push_str(r#"<g stroke="white" stroke-width="0.65" paint-order="stroke">"#);
    for axis in 0..2 {
        let dir = area.rec.stage_sign[axis];
        let m=area.img.image_transform();
        let (dx,dy)=if axis==0 { (dir*8.0*m[0],dir*8.0*m[1]) } else { (dir*8.0*m[2],dir*8.0*m[3]) };
        let (ex, ey) = (ox + dx, oy + dy);
        let (hx, hy) = (ex - dx * 0.23, ey - dy * 0.23);
        let _ = write!(
            s,
            r#"<path d="M{ox} {oy} L{ex} {ey} M{} {} L{ex} {ey} L{} {}" stroke="white" stroke-width="0.85" fill="none"/>"#,
            hx - dy * 0.12,
            hy + dx * 0.12,
            hx + dy * 0.12,
            hy - dx * 0.12
        );
        let _ = write!(
            s,
            r#"<path id="stage-{}" data-direction="{dir}" d="M{ox} {oy} L{ex} {ey} M{} {} L{ex} {ey} L{} {}" stroke="black" stroke-width="0.3" fill="none"/>"#,
            if axis == 0 { "x" } else { "y" },
            hx - dy * 0.12,
            hy + dx * 0.12,
            hx + dy * 0.12,
            hy - dx * 0.12
        );
        text(
            s,
            if axis == 0 { ox } else { ox + 2.0 },
            if axis == 0 { oy + 4.0 } else { ey },
            2.2,
            if axis == 0 { "middle" } else { "start" },
            if axis == 0 { "+X stage" } else { "+Y stage" },
        );
    }
    s.push_str("</g>");
}
