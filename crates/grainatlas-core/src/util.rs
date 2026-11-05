//! Small shared helpers: parallel row loops, hashing, text decoding.

use sha2::{Digest, Sha256};

/// Runs `f(row_index, row)` over the rows of `buf` (`w` items per row), in parallel on native.
pub fn par_rows<T: Send>(buf: &mut [T], w: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
    if w == 0 {
        return;
    }
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    {
        use rayon::prelude::*;
        buf.par_chunks_mut(w).enumerate().for_each(|(y, r)| f(y, r));
    }
    #[cfg(not(all(feature = "parallel", not(target_arch = "wasm32"))))]
    {
        for (y, r) in buf.chunks_mut(w).enumerate() {
            f(y, r);
        }
    }
}

/// `(0..n).map(f).collect()`, in parallel on native. Order is preserved.
pub fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    {
        use rayon::prelude::*;
        (0..n).into_par_iter().map(f).collect()
    }
    #[cfg(not(all(feature = "parallel", not(target_arch = "wasm32"))))]
    {
        (0..n).map(f).collect()
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex(&h.finalize())
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Decodes text that may be UTF-16LE/BE (with or without BOM) or UTF-8 (with or without BOM).
pub fn decode_text(b: &[u8]) -> String {
    if b.len() >= 2 && b[0] == 0xFF && b[1] == 0xFE {
        return utf16(&b[2..], false);
    }
    if b.len() >= 2 && b[0] == 0xFE && b[1] == 0xFF {
        return utf16(&b[2..], true);
    }
    if b.len() >= 3 && b[0] == 0xEF && b[1] == 0xBB && b[2] == 0xBF {
        return String::from_utf8_lossy(&b[3..]).into_owned();
    }
    // Heuristic: many zero bytes at odd positions means UTF-16LE without BOM.
    let zeros = b.iter().skip(1).step_by(2).take(256).filter(|&&c| c == 0).count();
    if b.len() > 8 && zeros * 2 > b.len().min(512) / 2 {
        return utf16(b, false);
    }
    String::from_utf8_lossy(b).into_owned()
}

fn utf16(b: &[u8], be: bool) -> String {
    let units: Vec<u16> = b
        .as_chunks::<2>().0.iter()
        .map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
        .collect();
    String::from_utf16_lossy(&units)
}

/// Parses an INI-like text into ordered (section, key, value) triples; never fails.
pub fn parse_ini(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut section = String::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with(';') {
            continue;
        }
        if l.starts_with('[') && l.ends_with(']') {
            section = l[1..l.len() - 1].to_string();
            continue;
        }
        if let Some(i) = l.find('=') {
            out.push((section.clone(), l[..i].trim().to_string(), l[i + 1..].trim().to_string()));
        } else {
            // Header lines without '=' (e.g. "Stage coordinates of fields") become sub-sections.
            section = l.to_string();
        }
    }
    out
}

/// Leading number of a string like "20000 Volt" or "17782.18 um".
pub fn leading_f64(s: &str) -> Option<f64> {
    let t = s.trim();
    let end = t.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E')).unwrap_or(t.len());
    t[..end].parse().ok()
}

/// Stable rounding to u8 from a 0..1 value.
#[inline]
pub fn to_u8(v: f32) -> u8 {
    (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}
