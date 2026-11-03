#![allow(dead_code)]
use std::path::PathBuf;
use std::sync::Arc;

use grainatlas_core::image::Gray;
use grainatlas_core::readers::{self, Package};
use grainatlas_core::source::DirSource;

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tm4000/Sweden_XS_150X")
}

pub fn fixture() -> Box<dyn Package> {
    readers::open(Arc::new(DirSource::new(fixture_dir()).unwrap())).unwrap()
}

/// Small deterministic RNG (xorshift).
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A smooth synthetic "grain mount": bright blobs on a dark background, defined continuously so
/// tiles can be cut at fractional offsets.
pub struct Scene {
    blobs: Vec<(f64, f64, f64, f64)>,
}

impl Scene {
    pub fn new(w: f64, h: f64, n: usize, seed: u64) -> Scene {
        let mut r = Rng(seed);
        let blobs = (0..n).map(|_| (r.next() * w, r.next() * h, 3.0 + r.next() * 12.0, 0.3 + 0.7 * r.next())).collect();
        Scene { blobs }
    }
    pub fn at(&self, x: f64, y: f64) -> f64 {
        let mut v = 0.08;
        for &(cx, cy, s, a) in &self.blobs {
            let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            if d2 < 25.0 * s * s {
                v += a * (-d2 / (2.0 * s * s)).exp();
            }
        }
        v.min(1.0)
    }
    /// Tile with top-left at (x0, y0), with gain and offset applied.
    pub fn tile(&self, x0: f64, y0: f64, w: usize, h: usize, gain: f64, offset: f64) -> Gray {
        let mut g = Gray::new(w, h);
        for y in 0..h {
            for x in 0..w {
                g.px[y * w + x] = (self.at(x0 + x as f64, y0 + y as f64) * gain + offset) as f32;
            }
        }
        g
    }
}
