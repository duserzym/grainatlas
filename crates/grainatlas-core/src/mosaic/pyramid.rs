//! Image pyramid of square tiles (512 px in the app, 256 px in atlas bundles):
//! level 0 is full resolution, each next level halves it, down to a single tile.

use serde::{Deserialize, Serialize};

pub const TILE: usize = 512;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LevelInfo {
    pub level: usize,
    pub w: usize,
    pub h: usize,
    pub cols: usize,
    pub rows: usize,
    /// Downsampling factor relative to level 0 (2^level).
    pub scale: usize,
}

/// `mosaic.json` (plan section 5.4).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MosaicJson {
    pub w: usize,
    pub h: usize,
    pub um_per_px: f64,
    /// Origin of the mosaic micrometre frame (always 0,0: top-left, y down).
    pub origin_um: [f64; 2],
    pub tile_size: usize,
    pub format: String,
    pub levels: Vec<LevelInfo>,
    /// SHA-256 of the level-0 8-bit pixels (row-major).
    pub sha256: String,
}

/// An 8-bit grey pyramid held in memory.
#[derive(Clone, Debug)]
pub struct Pyramid {
    pub levels: Vec<(LevelInfo, std::sync::Arc<Vec<u8>>)>,
    /// Tile edge in pixels.
    pub tile: usize,
}

impl Pyramid {
    pub fn build(w: usize, h: usize, px: Vec<u8>) -> Pyramid {
        Pyramid::build_tiled(w, h, px, TILE)
    }

    /// A pyramid of `tile`-pixel tiles.
    pub fn build_tiled(w: usize, h: usize, px: Vec<u8>, tile: usize) -> Pyramid {
        Self::build_shared_tiled(w, h, std::sync::Arc::new(px), tile)
    }

    pub fn build_shared(w: usize, h: usize, px: std::sync::Arc<Vec<u8>>) -> Pyramid {
        Self::build_shared_tiled(w, h, px, TILE)
    }

    fn build_shared_tiled(w: usize, h: usize, px: std::sync::Arc<Vec<u8>>, tile: usize) -> Pyramid {
        let tile = tile.max(1);
        let mut levels = Vec::new();
        let (mut cw, mut ch, mut cur) = (w, h, px);
        let mut l = 0;
        loop {
            let info = LevelInfo { level: l, w: cw, h: ch, cols: cw.div_ceil(tile).max(1), rows: ch.div_ceil(tile).max(1), scale: 1 << l,
            };
            let last = info.cols == 1 && info.rows == 1;
            let next = if last { None } else { Some(half_u8(cw, ch, &cur)) };
            levels.push((info, cur));
            match next {
                Some((nw, nh, n)) => {
                    cw = nw;
                    ch = nh;
                    cur = std::sync::Arc::new(n);
                    l += 1;
                }
                None => break,
            }
        }
        Pyramid { levels, tile }
    }

    pub fn infos(&self) -> Vec<LevelInfo> {
        self.levels.iter().map(|(i, _)| i.clone()).collect()
    }

    /// One tile as raw grey bytes and its size (edge tiles are smaller).
    pub fn tile(&self, level: usize, col: usize, row: usize) -> Option<(usize, usize, Vec<u8>)> {
        let (info, px) = self.levels.get(level)?;
        if col >= info.cols || row >= info.rows {
            return None;
        }
        let x0 = col * self.tile;
        let y0 = row * self.tile;
        let tw = self.tile.min(info.w - x0);
        let th = self.tile.min(info.h - y0);
        let mut out = Vec::with_capacity(tw * th);
        for y in y0..y0 + th {
            out.extend_from_slice(&px[y * info.w + x0..y * info.w + x0 + tw]);
        }
        Some((tw, th, out))
    }

    /// PNG-encoded tiles: (level, col, row, bytes), for the project folder.
    pub fn png_tiles(&self) -> Vec<(usize, usize, usize, Vec<u8>)> {
        let mut jobs = Vec::new();
        for (info, _) in &self.levels {
            for r in 0..info.rows {
                for c in 0..info.cols {
                    jobs.push((info.level, c, r));
                }
            }
        }
        crate::util::par_map(jobs.len(), |k| {
            let (l, c, r) = jobs[k];
            let (w, h, px) = self.tile(l, c, r).expect("tile in range");
            (l, c, r, crate::image::encode_png_gray(w, h, &px))
        })
    }
}

fn half_u8(w: usize, h: usize, px: &[u8]) -> (usize, usize, Vec<u8>) {
    let nw = w.div_ceil(2).max(1);
    let nh = h.div_ceil(2).max(1);
    let mut out = vec![0u8; nw * nh];
    crate::util::par_rows(&mut out, nw, |y, row| {
        let y0 = (2 * y).min(h - 1);
        let y1 = (2 * y + 1).min(h - 1);
        for (x, o) in row.iter_mut().enumerate() {
            let x0 = (2 * x).min(w - 1);
            let x1 = (2 * x + 1).min(w - 1);
            let s = px[y0 * w + x0] as u32 + px[y0 * w + x1] as u32 + px[y1 * w + x0] as u32 + px[y1 * w + x1] as u32;
            *o = ((s + 2) / 4) as u8;
        }
    });
    (nw, nh, out)
}
