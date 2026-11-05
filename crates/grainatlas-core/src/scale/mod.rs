//! Scale registration (plan section 6.6): pixel size from metadata, or from two
//! clicked points and a typed length, or from a detected burned-in scale bar;
//! scale-bar lengths for grains and pages.

pub mod register;

use crate::image::Gray;

/// Candidate bar lengths in micrometres.
pub const BAR_LENGTHS_UM: [f64; 6] = [10.0, 20.0, 50.0, 100.0, 200.0, 500.0];

/// The largest of 10, 20, 50, 100, 200 or 500 um that fits a third of the grain's bbox width.
pub fn grain_bar_um(bbox_width_um: f64) -> f64 {
    BAR_LENGTHS_UM.iter().copied().rfind(|&l| l <= bbox_width_um / 3.0).unwrap_or(BAR_LENGTHS_UM[0])
}

/// A round bar length (1-2-5 series) no longer than `max_um`.
pub fn round_bar_um(max_um: f64) -> f64 {
    let mut best = 1.0;
    let mut decade = 1e-3;
    while decade <= 1e7 {
        for m in [1.0, 2.0, 5.0] {
            let v = m * decade;
            if v <= max_um {
                best = v;
            }
        }
        decade *= 10.0;
    }
    best
}

/// Pixel size (nm) from two points (px) a known distance apart (um).
pub fn from_two_points(a: [f64; 2], b: [f64; 2], length_um: f64) -> Option<f64> {
    let d = (a[0] - b[0]).hypot(a[1] - b[1]);
    (d > 0.0 && length_um > 0.0).then(|| length_um * 1000.0 / d)
}

/// A detected burned-in scale bar: the longest horizontal run of bright pixels in the bottom
/// band of the image. Returns (x0, x1, y) in pixels.
pub fn detect_bar(g: &Gray, band_frac: f64) -> Option<(usize, usize, usize)> {
    let y0 = ((g.h as f64) * (1.0 - band_frac)).max(0.0) as usize;
    let mut best: Option<(usize, usize, usize)> = None;
    for y in y0..g.h {
        let mut x = 0;
        while x < g.w {
            if g.px[y * g.w + x] > 0.85 {
                let s = x;
                while x < g.w && g.px[y * g.w + x] > 0.85 {
                    x += 1;
                }
                let len = x - s;
                // Ignore runs that span the whole width (borders of the data bar).
                if len < g.w * 9 / 10 && best.is_none_or(|b| len > b.1 - b.0) {
                    best = Some((s, x, y));
                }
            } else {
                x += 1;
            }
        }
    }
    best.filter(|b| b.1 - b.0 >= 10)
}
