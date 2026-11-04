//! Grey WebP tiles for atlas bundles, with the pure-Rust VP8L (lossless WebP)
//! encoder of `image-webp` copied into [`super::vp8l`] with a stable sort, so
//! native and wasm builds write the same bytes.
//!
//! "Quality" below 100 is near-lossless in the libwebp sense: pixel values are
//! rounded to a coarser step (2 grey levels at 95 to 99, 4 at 90 to 94) before
//! the lossless encoder, which compresses the result further. No pure-Rust lossy
//! VP8 encoder runs identically on both targets, so the lossy WebP mode is not
//! offered; the error stays bounded (at most 1 or 2 grey levels).

use crate::{Error, Result};

/// Encoding of the tiles: 100 = lossless, 90 to 99 = near-lossless.
pub fn quality_bits(quality: u8) -> u32 {
    match quality {
        100.. => 0,
        95..=99 => 1,
        _ => 2,
    }
}

pub fn encoding_name(quality: u8) -> &'static str {
    if quality_bits(quality) == 0 { "lossless" } else { "near-lossless" }
}

/// One grey tile as a WebP file.
pub fn encode_gray(w: usize, h: usize, px: &[u8], quality: u8) -> Result<Vec<u8>> {
    let bits = quality_bits(quality);
    let data: std::borrow::Cow<[u8]> = if bits == 0 {
        px.into()
    } else {
        let half = 1u32 << (bits - 1);
        px.iter().map(|&v| (((v as u32 + half) >> bits) << bits).min(255) as u8).collect::<Vec<u8>>().into()
    };
    let mut out = Vec::new();
    super::vp8l::WebPEncoder::new(&mut out)
        .encode(&data, w as u32, h as u32, super::vp8l::ColorType::L8)
        .map_err(|e| Error::Invalid(format!("webp: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lossless_round_trip() {
        let (w, h) = (37, 19);
        let px: Vec<u8> = (0..w * h).map(|i| ((i * 37) % 251) as u8).collect();
        let b = encode_gray(w, h, &px, 100).unwrap();
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(&b[8..12], b"WEBP");
        let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(&b)).unwrap();
        let mut rgba = vec![0u8; d.output_buffer_size().unwrap()];
        d.read_image(&mut rgba).unwrap();
        let stride = rgba.len() / (w * h);
        for (i, v) in px.iter().enumerate() {
            assert_eq!(rgba[i * stride], *v);
        }
    }

    #[test]
    fn near_lossless_error_is_bounded() {
        let px: Vec<u8> = (0..=255u8).collect();
        for (q, max) in [(97u8, 1i32), (90, 2)] {
            let b = encode_gray(16, 16, &px, q).unwrap();
            let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(&b)).unwrap();
            let mut rgba = vec![0u8; d.output_buffer_size().unwrap()];
            d.read_image(&mut rgba).unwrap();
            let stride = rgba.len() / 256;
            for (i, v) in px.iter().enumerate() {
                assert!((rgba[i * stride] as i32 - *v as i32).abs() <= max, "q {q} at {v}");
            }
        }
    }
}
