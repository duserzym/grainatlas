//! Sequential, little-endian tiled TIFF. Uncompressed tile lengths are known in
//! advance, so the IFD precedes pixels: neither seeking nor a file-sized buffer
//! is needed (including on FileSystemWritableFileStream).
use crate::{Error, Result};
pub const TILE: u64 = 512;
// Leave headroom below classic TIFF's 32-bit offset limit.
pub const BIG_THRESHOLD: u64 = 3_500_000_000;
fn invalid(s: impl ToString) -> Error {
    Error::Invalid(s.to_string())
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub bits: u16,
    pub channels: u16,
    pub tiles: u64,
    pub tile_bytes: u64,
    pub pixel_bytes: u64,
    pub big: bool,
}
impl Layout {
    pub fn new(w: u64, h: u64, bits: u16, channels: u16) -> Result<Self> {
        if w == 0
            || h == 0
            || w > u32::MAX as u64
            || h > u32::MAX as u64
            || ![8, 16].contains(&bits)
            || ![1, 3].contains(&channels)
        {
            return Err(invalid("Invalid TIFF dimensions or sample type"));
        }
        let tiles = w
            .div_ceil(TILE)
            .checked_mul(h.div_ceil(TILE))
            .ok_or_else(|| invalid("TIFF is too large"))?;
        let tile_bytes = TILE * TILE * channels as u64 * (bits / 8) as u64;
        let pixel_bytes = tiles
            .checked_mul(tile_bytes)
            .ok_or_else(|| invalid("TIFF is too large"))?;
        // Includes both offset arrays and a generous metadata allowance.
        let maximum = pixel_bytes
            .checked_add(
                tiles
                    .checked_mul(16)
                    .ok_or_else(|| invalid("TIFF is too large"))?,
            )
            .and_then(|n| n.checked_add(1_048_576))
            .ok_or_else(|| invalid("TIFF is too large"))?;
        Ok(Self {
            width: w as u32,
            height: h as u32,
            bits,
            channels,
            tiles,
            tile_bytes,
            pixel_bytes,
            big: maximum >= BIG_THRESHOLD,
        })
    }
    pub fn cols(&self) -> u64 {
        (self.width as u64).div_ceil(TILE)
    }
    /// Conservative estimate, including metadata, padded edge tiles and IFD arrays.
    pub fn estimate(&self) -> u64 {
        self.pixel_bytes + self.tiles * if self.big { 16 } else { 8 } + 1_048_576
    }
    pub fn header(&self, pixel_um: [f64; 2], description: &str) -> Result<Vec<u8>> {
        if pixel_um.iter().any(|v| !v.is_finite() || *v <= 0.0) || description.len() > 1_000_000 {
            return Err(invalid("Invalid TIFF resolution or description"));
        }
        let inline = if self.big { 8 } else { 4 };
        let n = 16u64;
        let start = if self.big {
            16 + 8 + n * 20 + 8
        } else {
            8 + 2 + n * 12 + 4
        };
        let mut tail = Vec::new();
        let mut entries = Vec::new();
        let mut field = |tag: u16, kind: u16, count: u64, data: Vec<u8>| {
            let value = if data.len() <= inline {
                let mut b = data;
                b.resize(inline, 0);
                b
            } else {
                while !(start + tail.len() as u64).is_multiple_of(8) {
                    tail.push(0);
                }
                let offset = start + tail.len() as u64;
                tail.extend(data);
                if self.big {
                    offset.to_le_bytes().to_vec()
                } else {
                    (offset as u32).to_le_bytes().to_vec()
                }
            };
            entries.push((tag, kind, count, value));
        };
        let short = |v: u16| v.to_le_bytes().to_vec();
        let long = |v: u32| v.to_le_bytes().to_vec();
        field(256, 4, 1, long(self.width));
        field(257, 4, 1, long(self.height));
        field(
            258,
            3,
            self.channels as u64,
            (0..self.channels).flat_map(|_| short(self.bits)).collect(),
        );
        field(259, 3, 1, short(1));
        field(262, 3, 1, short(if self.channels == 1 { 1 } else { 2 }));
        let mut desc = description.as_bytes().to_vec();
        desc.push(0);
        field(270, 2, desc.len() as u64, desc);
        field(277, 3, 1, short(self.channels));
        // Centimetres: 10,000 µm per cm. Rational is rounded at 10^-4 pixels/cm.
        for (tag, um) in [(282, pixel_um[0]), (283, pixel_um[1])] {
            let resolution = 10_000.0 / um;
            let denominator = (u32::MAX as f64 / resolution).floor().clamp(1.0, 10_000.0) as u32;
            let numerator = (resolution * denominator as f64)
                .round()
                .clamp(1.0, u32::MAX as f64) as u32;
            field(tag, 5, 1, [long(numerator), long(denominator)].concat());
        }
        field(284, 3, 1, short(1));
        field(296, 3, 1, short(3));
        field(322, 4, 1, long(TILE as u32));
        field(323, 4, 1, long(TILE as u32));
        // Reserve arrays first; offsets are filled once the complete header length is known.
        let offset_kind = if self.big { 16 } else { 4 };
        field(
            324,
            offset_kind,
            self.tiles,
            vec![0; usize::try_from(self.tiles).map_err(invalid)? * inline],
        );
        field(
            325,
            offset_kind,
            self.tiles,
            (0..self.tiles)
                .flat_map(|_| {
                    if self.big {
                        self.tile_bytes.to_le_bytes().to_vec()
                    } else {
                        (self.tile_bytes as u32).to_le_bytes().to_vec()
                    }
                })
                .collect(),
        );
        field(
            339,
            3,
            self.channels as u64,
            (0..self.channels).flat_map(|_| short(1)).collect(),
        );
        let mut out = if self.big {
            b"II\x2b\x00\x08\x00\x00\x00\x10\x00\x00\x00\x00\x00\x00\x00".to_vec()
        } else {
            b"II\x2a\x00\x08\x00\x00\x00".to_vec()
        };
        if self.big {
            out.extend(n.to_le_bytes());
        } else {
            out.extend((n as u16).to_le_bytes());
        }
        let first_tile = (start + tail.len() as u64).div_ceil(8) * 8;
        for (tag, kind, count, mut value) in entries {
            if tag == 324 && self.tiles == 1 {
                value = if self.big {
                    first_tile.to_le_bytes().to_vec()
                } else {
                    (first_tile as u32).to_le_bytes().to_vec()
                };
            } else if tag == 324 {
                let offset = if self.big {
                    u64::from_le_bytes(value.clone().try_into().unwrap())
                } else {
                    u32::from_le_bytes(value.clone().try_into().unwrap()) as u64
                };
                let at = (offset - start) as usize;
                for i in 0..self.tiles {
                    let v = first_tile + i * self.tile_bytes;
                    let bytes = if self.big {
                        v.to_le_bytes().to_vec()
                    } else {
                        (v as u32).to_le_bytes().to_vec()
                    };
                    tail[at + i as usize * inline..at + (i as usize + 1) * inline]
                        .copy_from_slice(&bytes);
                }
            }
            out.extend(tag.to_le_bytes());
            out.extend(kind.to_le_bytes());
            if self.big {
                out.extend(count.to_le_bytes());
            } else {
                out.extend((count as u32).to_le_bytes());
            }
            out.extend(value);
        }
        out.resize(start as usize, 0);
        out.extend(tail);
        out.resize(first_tile as usize, 0);
        Ok(out)
    }
}
