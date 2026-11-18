use super::{ImageFormat, Options, PixelSource};
use crate::{Ctl, Error, Result, project::pipeline::AreaOutput, segment::Grain};
use image::ImageEncoder;

pub enum Pixels<'a> {
    Float(&'a [f32]),
    Byte(&'a [u8], Box<[u8; 256]>),
}
impl Pixels<'_> {
    pub fn at(&self, i: usize) -> f64 {
        match self {
            Self::Float(v) => v[i] as f64,
            Self::Byte(v, lut) => lut[v[i] as usize] as f64 / 255.0,
        }
    }
    pub fn hash(&self, ctl: &Ctl) -> Result<String> {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        match self {
            Self::Byte(v, lut) => {
                for chunk in v.chunks(1024 * 1024) {
                    ctl.check()?;
                    h.update(chunk.iter().map(|v| lut[*v as usize]).collect::<Vec<_>>());
                }
            }
            Self::Float(v) => {
                for chunk in v.chunks(262144) {
                    ctl.check()?;
                    let bytes: Vec<u8> = chunk.iter().flat_map(|x| x.to_le_bytes()).collect();
                    h.update(bytes);
                }
            }
        }
        Ok(format!("{:x}", h.finalize()))
    }
}
pub fn source<'a>(
    o: &'a AreaOutput,
    published: Option<&'a [u8]>,
    opts: &Options,
    tone: &crate::photometric::ToneParams,
) -> (Pixels<'a>, &'static str) {
    if opts.pixel_values == PixelSource::Displayed {
        return (
            Pixels::Byte(&o.base, Box::new(crate::photometric::tone::lut(tone))),
            if published.is_some() {
                "published-8bit"
            } else {
                "displayed-8bit"
            },
        );
    }
    if let Some(px) = published {
        (
            Pixels::Byte(px, Box::new(std::array::from_fn(|i| i as u8))),
            "published-8bit",
        )
    } else {
        (Pixels::Float(&o.linear.px), "original-f32")
    }
}

pub fn inside(poly: &[[f64; 2]], x: f64, y: f64) -> bool {
    let mut yes = false;
    for (a, b) in poly
        .iter()
        .zip(poly.iter().cycle().skip(1))
        .take(poly.len())
    {
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            yes = !yes;
        }
    }
    yes
}

pub struct Raster {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub mask: Vec<u8>,
    pub values: Vec<f64>,
}
pub fn raster(
    g: &Grain,
    scale: f64,
    w: usize,
    h: usize,
    margin: f64,
    px: &Pixels,
    ctl: &Ctl,
) -> Result<Raster> {
    let bounds = g.polygon_um.iter().fold(
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
    );
    let x = ((bounds[0] - margin) / scale).floor().clamp(0.0, w as f64) as usize;
    let y = ((bounds[1] - margin) / scale).floor().clamp(0.0, h as f64) as usize;
    let right = ((bounds[2] + margin) / scale)
        .ceil()
        .clamp(x as f64, w as f64) as usize;
    let bottom = ((bounds[3] + margin) / scale)
        .ceil()
        .clamp(y as f64, h as f64) as usize;
    let (cw, ch) = (right - x, bottom - y);
    let mut r = Raster {
        x,
        y,
        w: cw,
        h: ch,
        mask: vec![0; cw * ch],
        values: vec![0.0; cw * ch],
    };
    let mut crossings = Vec::with_capacity(g.polygon_um.len());
    for row in 0..ch {
        if row % 32 == 0 {
            ctl.check()?;
        }
        let offset = row * cw;
        for col in 0..cw {
            let value = px.at((row + y) * w + col + x);
            if !value.is_finite() {
                return Err(Error::Invalid("non-finite mosaic pixel".into()));
            }
            r.values[offset + col] = value;
        }
        mask_row(
            &g.polygon_um,
            x,
            (row + y) as f64 * scale + scale / 2.0,
            scale,
            &mut r.mask[offset..offset + cw],
            &mut crossings,
        );
    }
    Ok(r)
}

/// Same ray crossings and arithmetic as `inside`, once per scanline. An edge
/// straddles y with its lower endpoint included and upper endpoint excluded.
/// Sorted crossings delimit runs whose centres satisfy left <= centre < right.
fn mask_row(
    poly: &[[f64; 2]],
    x: usize,
    y: f64,
    scale: f64,
    mask: &mut [u8],
    crossings: &mut Vec<f64>,
) {
    crossings.clear();
    for (a, b) in poly
        .iter()
        .zip(poly.iter().cycle().skip(1))
        .take(poly.len())
    {
        if (a[1] > y) != (b[1] > y) {
            let cross = (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0];
            // `centre < NaN` never toggles the reference ray test either.
            if !cross.is_nan() {
                crossings.push(cross);
            }
        }
    }
    crossings.sort_unstable_by(f64::total_cmp);
    let mut start = 0;
    let mut inside = !crossings.len().is_multiple_of(2);
    for &cross in crossings.iter() {
        // Search the original centre expression, rather than rounding
        // cross/scale - 0.5: that division can shift a boundary by one ulp.
        let (mut lo, mut hi) = (start, mask.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if (x + mid) as f64 * scale + scale / 2.0 < cross {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if inside {
            mask[start..lo].fill(255);
        }
        inside = !inside;
        start = lo;
    }
}

pub fn png(w: usize, h: usize, bytes: &[u8], sixteen: bool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(if sixteen {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        });
        enc.write_header()
            .map_err(|e| Error::Invalid(e.to_string()))?
            .write_image_data(bytes)
            .map_err(|e| Error::Invalid(e.to_string()))?;
    }
    Ok(out)
}
pub fn encode(r: &Raster, byte: bool, format: ImageFormat) -> Result<Vec<u8>> {
    if format == ImageFormat::Tiff {
        let mut out = std::io::Cursor::new(Vec::new());
        let enc = image::codecs::tiff::TiffEncoder::new(&mut out);
        if byte {
            let b: Vec<u8> = r
                .values
                .iter()
                .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
                .collect();
            enc.write_image(&b, r.w as u32, r.h as u32, image::ExtendedColorType::L8)
        } else {
            let b: Vec<u8> = r
                .values
                .iter()
                .flat_map(|v| ((v * 65535.0).round().clamp(0.0, 65535.0) as u16).to_ne_bytes())
                .collect();
            enc.write_image(&b, r.w as u32, r.h as u32, image::ExtendedColorType::L16)
        }
        .map_err(|e| Error::Invalid(e.to_string()))?;
        Ok(out.into_inner())
    } else {
        let b: Vec<u8> = if byte {
            r.values
                .iter()
                .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
                .collect()
        } else {
            r.values
                .iter()
                .flat_map(|v| ((v * 65535.0).round().clamp(0.0, 65535.0) as u16).to_be_bytes())
                .collect()
        };
        png(r.w, r.h, &b, !byte)
    }
}
