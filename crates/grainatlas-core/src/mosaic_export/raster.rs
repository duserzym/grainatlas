//! Scanline encoders. PNG retains its deflater and at most two rows. JPEG uses
//! independent eight-row restart intervals, with identical quantisation tables.
//! image's MIT/Apache JPEG encoder visits blocks in raster order and keeps no image copy.
use super::{Result, invalid};
use std::io::Write;
use std::sync::{Arc, Mutex};
#[derive(Clone, Default)]
pub struct Queue(Arc<Mutex<Vec<u8>>>);
impl Write for Queue {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Queue {
    pub fn drain(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}
pub struct Png {
    stream: Option<png::StreamWriter<'static, Queue>>,
    queue: Queue,
}
impl Png {
    pub fn new(w: u32, h: u32, bits: u16) -> Result<Self> {
        let q = Queue::default();
        let mut e = png::Encoder::new(q.clone(), w, h);
        e.set_color(png::ColorType::Grayscale);
        e.set_depth(if bits == 16 {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        });
        let stream = e
            .write_header()
            .map_err(invalid)?
            .into_stream_writer()
            .map_err(invalid)?;
        Ok(Self {
            stream: Some(stream),
            queue: q,
        })
    }
    /// Samples are big endian, as PNG specifies. Finish writes IEND before draining.
    pub fn rows(&mut self, bytes: &[u8], last: bool) -> Result<Vec<u8>> {
        self.stream
            .as_mut()
            .ok_or_else(|| invalid("Finished PNG stream"))?
            .write_all(bytes)
            .map_err(invalid)?;
        if last {
            self.stream.take().unwrap().finish().map_err(invalid)?;
        }
        Ok(self.queue.drain())
    }
}
/// Every interval contains ceil(width/8) MCUs. A restart marker resets the DC
/// predictor between independently encoded strips; entropy bytes remain untouched.
pub fn jpeg_strip(w: u32, h: u32, y: u32, px: &[u8], quality: u8) -> Result<Vec<u8>> {
    let rows = (h - y).min(8);
    let mut bytes = Vec::new();
    ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode(px, w, rows, ::image::ExtendedColorType::L8)
        .map_err(invalid)?;
    let mut i = 2;
    let mut scan = 0;
    let mut sof = 0;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xff {
            return Err(invalid("Invalid JPEG segment"));
        }
        let marker = bytes[i + 1];
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if marker == 0xc0 {
            sof = i;
        }
        if marker == 0xda {
            scan = i + 2 + len;
            break;
        }
        i += 2 + len;
    }
    if scan == 0 || sof == 0 || bytes[bytes.len() - 2..] != [0xff, 0xd9] {
        return Err(invalid("Invalid JPEG encoder output"));
    }
    let mut out = Vec::new();
    if y == 0 {
        bytes[sof + 5..sof + 7].copy_from_slice(&(h as u16).to_be_bytes());
        out.extend_from_slice(&bytes[..i]);
        out.extend_from_slice(&[0xff, 0xdd, 0, 4]);
        out.extend_from_slice(&(w.div_ceil(8) as u16).to_be_bytes());
        out.extend_from_slice(&bytes[i..scan]);
    }
    out.extend_from_slice(&bytes[scan..bytes.len() - 2]);
    if y + rows == h {
        out.extend_from_slice(&[0xff, 0xd9]);
    } else {
        out.extend_from_slice(&[0xff, 0xd0 + ((y / 8) % 8) as u8]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jpeg_restart_intervals_include_a_partial_last_row() {
        let (w, h) = (17u32, 19u32);
        let px: Vec<_> = (0..w * h)
            .map(|j| (32 + j % w * 7 + j / w * 3) as u8)
            .collect();
        let mut encoded = Vec::new();
        for y in (0..h).step_by(8) {
            let rows = (h - y).min(8);
            encoded.extend(
                jpeg_strip(
                    w,
                    h,
                    y,
                    &px[(y * w) as usize..((y + rows) * w) as usize],
                    90,
                )
                .unwrap(),
            );
        }
        let image = ::image::load_from_memory(&encoded).unwrap().to_luma8();
        assert_eq!(image.dimensions(), (w, h));
        let mse = image
            .as_raw()
            .iter()
            .zip(&px)
            .map(|(&a, &b)| (a as f64 - b as f64).powi(2))
            .sum::<f64>()
            / (w * h) as f64;
        assert!(mse.sqrt() < 4.);
    }
}
