//! Grey images in `f32` (0..1), decoding and encoding.
//!
//! JPEG goes through zune-jpeg with its SIMD paths disabled so the native and
//! wasm builds decode bit-identical pixels (plan section 9, parity).

use crate::{Error, Result};

/// Row-major single-channel image, samples nominally in `0..=1`.
#[derive(Clone, Debug, PartialEq)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

impl Gray {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, px: vec![0.0; w * h] }
    }
    pub fn filled(w: usize, h: usize, v: f32) -> Self {
        Self { w, h, px: vec![v; w * h] }
    }
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.px[y * self.w + x]
    }
    /// Clamped sample.
    #[inline]
    pub fn atc(&self, x: i64, y: i64) -> f32 {
        let x = x.clamp(0, self.w as i64 - 1) as usize;
        let y = y.clamp(0, self.h as i64 - 1) as usize;
        self.px[y * self.w + x]
    }
    /// Bilinear sample with clamping.
    pub fn bilinear(&self, x: f32, y: f32) -> f32 {
        let x0 = x.floor();
        let y0 = y.floor();
        let fx = x - x0;
        let fy = y - y0;
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = self.atc(x0, y0);
        let b = self.atc(x0 + 1, y0);
        let c = self.atc(x0, y0 + 1);
        let d = self.atc(x0 + 1, y0 + 1);
        (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy
    }
    /// Crop (clamped to the image).
    pub fn crop(&self, x0: usize, y0: usize, w: usize, h: usize) -> Gray {
        let x0 = x0.min(self.w);
        let y0 = y0.min(self.h);
        let w = w.min(self.w - x0);
        let h = h.min(self.h - y0);
        let mut out = Gray::new(w, h);
        for y in 0..h {
            out.px[y * w..(y + 1) * w].copy_from_slice(&self.px[(y0 + y) * self.w + x0..(y0 + y) * self.w + x0 + w]);
        }
        out
    }
    /// 2x2 box downsample (ceil size; edge pixels replicate).
    pub fn half(&self) -> Gray {
        let w = self.w.div_ceil(2).max(1);
        let h = self.h.div_ceil(2).max(1);
        let mut out = Gray::new(w, h);
        crate::util::par_rows(&mut out.px, w, |y, row| {
            let y0 = (2 * y).min(self.h - 1);
            let y1 = (2 * y + 1).min(self.h - 1);
            for (x, o) in row.iter_mut().enumerate() {
                let x0 = (2 * x).min(self.w - 1);
                let x1 = (2 * x + 1).min(self.w - 1);
                *o = (self.px[y0 * self.w + x0] + self.px[y0 * self.w + x1] + self.px[y1 * self.w + x0] + self.px[y1 * self.w + x1]) * 0.25;
            }
        });
        out
    }
    /// Downsample by an integer factor (box filter).
    pub fn downsample(&self, k: usize) -> Gray {
        if k <= 1 {
            return self.clone();
        }
        let w = self.w.div_ceil(k).max(1);
        let h = self.h.div_ceil(k).max(1);
        let mut out = Gray::new(w, h);
        crate::util::par_rows(&mut out.px, w, |y, row| {
            for (x, o) in row.iter_mut().enumerate() {
                let (mut s, mut n) = (0.0f32, 0u32);
                for yy in y * k..((y + 1) * k).min(self.h) {
                    for xx in x * k..((x + 1) * k).min(self.w) {
                        s += self.px[yy * self.w + xx];
                        n += 1;
                    }
                }
                *o = s / n.max(1) as f32;
            }
        });
        out
    }
    pub fn to_u8(&self) -> Vec<u8> {
        self.px.iter().map(|&v| crate::util::to_u8(v)).collect()
    }
    pub fn from_u8(w: usize, h: usize, b: &[u8]) -> Gray {
        Gray { w, h, px: b.iter().map(|&v| v as f32 / 255.0).collect() }
    }
    pub fn mean(&self) -> f32 {
        (self.px.iter().map(|&v| v as f64).sum::<f64>() / self.px.len().max(1) as f64) as f32
    }
}

/// Bit depth of the source, kept so 16-bit inputs are not quantised early.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Depth {
    U8,
    U16,
}

/// Decodes JPEG, PNG or TIFF bytes to grey (RGB averaged with Rec. 709 weights).
pub fn decode(bytes: &[u8]) -> Result<(Gray, Depth)> {
    if bytes.len() > 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        return decode_jpeg(bytes).map(|g| (g, Depth::U8));
    }
    let img = image::load_from_memory(bytes).map_err(|e| Error::Read("image".into(), e.to_string()))?;
    let depth = match img.color() {
        image::ColorType::L16 | image::ColorType::La16 | image::ColorType::Rgb16 | image::ColorType::Rgba16 => Depth::U16,
        _ => Depth::U8,
    };
    let l = img.to_luma16();
    let (w, h) = (l.width() as usize, l.height() as usize);
    let px = l.into_raw().into_iter().map(|v| v as f32 / 65535.0).collect();
    Ok((Gray { w, h, px }, depth))
}

fn decode_jpeg(bytes: &[u8]) -> Result<Gray> {
    use zune_core::bytestream::ZCursor;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let opts = DecoderOptions::default().set_use_unsafe(false).jpeg_set_out_colorspace(ColorSpace::Luma);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), opts);
    let px = d.decode().map_err(|e| Error::Read("jpeg".into(), format!("{e:?}")))?;
    let info = d.info().ok_or_else(|| Error::Read("jpeg".into(), "no header".into()))?;
    let (w, h) = (info.width as usize, info.height as usize);
    if px.len() != w * h {
        return Err(Error::Read("jpeg".into(), format!("expected {} grey samples, got {}", w * h, px.len())));
    }
    Ok(Gray::from_u8(w, h, &px))
}

/// Image size without decoding pixels (JPEG SOF scan or full decode fallback).
pub fn probe_size(bytes: &[u8]) -> Option<(usize, usize)> {
    if bytes.len() > 4 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let m = bytes[i + 1];
            if (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as usize;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as usize;
                return Some((w, h));
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            i += 2 + len;
        }
        return None;
    }
    image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok().map(|(w, h)| (w as usize, h as usize))
}

/// Lossless 8-bit grey PNG (fast compression; deterministic).
pub fn encode_png_gray(w: usize, h: usize, px: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w as u32, h as u32);
        e.set_color(png::ColorType::Grayscale);
        e.set_depth(png::BitDepth::Eight);
        e.set_compression(png::Compression::Fast);
        let mut wr = e.write_header().expect("png header");
        wr.write_image_data(px).expect("png data");
    }
    out
}

/// 8-bit RGBA PNG.
pub fn encode_png_rgba(w: usize, h: usize, px: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w as u32, h as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.set_compression(png::Compression::Fast);
        let mut wr = e.write_header().expect("png header");
        wr.write_image_data(px).expect("png data");
    }
    out
}

/// 16-bit grey PNG (label images).
pub fn encode_png_gray16(w: usize, h: usize, px: &[u16]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w as u32, h as u32);
        e.set_color(png::ColorType::Grayscale);
        e.set_depth(png::BitDepth::Sixteen);
        e.set_compression(png::Compression::Fast);
        let mut wr = e.write_header().expect("png header");
        let bytes: Vec<u8> = px.iter().flat_map(|v| v.to_be_bytes()).collect();
        wr.write_image_data(&bytes).expect("png data");
    }
    out
}

/// Baseline 16-bit grey TIFF (uncompressed) for exports.
pub fn encode_tiff_gray16(w: usize, h: usize, px: &[u16]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    let raw: Vec<u8> = px.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let buf: image::ImageBuffer<image::Luma<u16>, Vec<u16>> = image::ImageBuffer::from_raw(w as u32, h as u32, px.to_vec()).expect("size");
    let _ = raw;
    image::DynamicImage::ImageLuma16(buf).write_to(&mut out, image::ImageFormat::Tiff).expect("tiff");
    out.into_inner()
}
