//! A minimal, deterministic ZIP writer and reader (STORE only, no compression).
//!
//! Atlas bundles are mostly WebP tiles, which do not deflate further, so the
//! archive stores every entry as is. Every entry carries the same fixed DOS
//! time (1980-01-01 00:00) and the UTF-8 name flag, so the same files give the
//! same bytes on every platform and in the browser.

use crate::{Error, Result};

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
/// General purpose flag bit 11: names are UTF-8.
const UTF8: u16 = 1 << 11;
/// 1980-01-01 in DOS date format; time 00:00:00.
const DOS_DATE: u16 = (1 << 5) | 1;

fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    for (i, e) in t.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *e = c;
    }
    t
}

/// CRC-32 (IEEE 802.3), as ZIP uses.
pub fn crc32(bytes: &[u8]) -> u32 {
    let t = crc_table();
    let mut c = 0xFFFF_FFFFu32;
    for &b in bytes {
        c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn u32le(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// Writes `files` (in the given order) under `prefix` (e.g. `"<id>.atlas/"`, or `""`).
pub fn write(prefix: &str, files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    if files.len() > u16::MAX as usize {
        return Err(Error::Invalid(format!("{} files is more than a plain ZIP holds (65535)", files.len())));
    }
    let total: usize = files.iter().map(|(n, b)| 30 + 46 + 2 * (prefix.len() + n.len()) + b.len()).sum::<usize>() + 22;
    if total > u32::MAX as usize {
        return Err(Error::Invalid("the bundle is larger than 4 GB; save it as a folder instead".into()));
    }
    let mut out = Vec::with_capacity(total);
    let mut central = Vec::new();
    for (name, bytes) in files {
        let full = format!("{prefix}{name}");
        let crc = crc32(bytes);
        let offset = out.len() as u32;
        u32le(&mut out, LOCAL);
        u16le(&mut out, 20);
        u16le(&mut out, UTF8);
        u16le(&mut out, 0); // stored
        u16le(&mut out, 0); // time
        u16le(&mut out, DOS_DATE);
        u32le(&mut out, crc);
        u32le(&mut out, bytes.len() as u32);
        u32le(&mut out, bytes.len() as u32);
        u16le(&mut out, full.len() as u16);
        u16le(&mut out, 0);
        out.extend_from_slice(full.as_bytes());
        out.extend_from_slice(bytes);

        u32le(&mut central, CENTRAL);
        u16le(&mut central, 20); // made by: MS-DOS / FAT semantics, spec 2.0
        u16le(&mut central, 20);
        u16le(&mut central, UTF8);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u16le(&mut central, DOS_DATE);
        u32le(&mut central, crc);
        u32le(&mut central, bytes.len() as u32);
        u32le(&mut central, bytes.len() as u32);
        u16le(&mut central, full.len() as u16);
        u16le(&mut central, 0); // extra
        u16le(&mut central, 0); // comment
        u16le(&mut central, 0); // disk
        u16le(&mut central, 0); // internal attributes
        u32le(&mut central, 0); // external attributes
        u32le(&mut central, offset);
        central.extend_from_slice(full.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    u32le(&mut out, END);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, files.len() as u16);
    u16le(&mut out, files.len() as u16);
    u32le(&mut out, central.len() as u32);
    u32le(&mut out, cd_offset);
    u16le(&mut out, 0);
    Ok(out)
}

fn rd16(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| Error::Invalid("truncated zip".into()))
}
fn rd32(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| Error::Invalid("truncated zip".into()))
}

/// Reads a STORE-only zip (what [`write`] makes): (name, bytes) in central-directory order.
pub fn read(b: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let end = (0..b.len().saturating_sub(21)).rev().find(|&i| rd32(b, i).ok() == Some(END)).ok_or_else(|| Error::Invalid("not a zip".into()))?;
    let n = rd16(b, end + 10)? as usize;
    let mut at = rd32(b, end + 16)? as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if rd32(b, at)? != CENTRAL {
            return Err(Error::Invalid("bad zip central directory".into()));
        }
        let method = rd16(b, at + 10)?;
        let crc = rd32(b, at + 16)?;
        let size = rd32(b, at + 20)? as usize;
        let name_len = rd16(b, at + 28)? as usize;
        let extra = rd16(b, at + 30)? as usize;
        let comment = rd16(b, at + 32)? as usize;
        let local = rd32(b, at + 42)? as usize;
        let name = String::from_utf8_lossy(b.get(at + 46..at + 46 + name_len).ok_or_else(|| Error::Invalid("truncated zip".into()))?).into_owned();
        if method != 0 {
            return Err(Error::Unsupported(format!("{name}: compressed zip entries are not read here")));
        }
        let data_at = local + 30 + rd16(b, local + 26)? as usize + rd16(b, local + 28)? as usize;
        let data = b.get(data_at..data_at + size).ok_or_else(|| Error::Invalid("truncated zip".into()))?.to_vec();
        if crc32(&data) != crc {
            return Err(Error::Invalid(format!("{name}: CRC mismatch")));
        }
        out.push((name, data));
        at += 46 + name_len + extra + comment;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_known_string() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn round_trip() {
        let files = vec![("a.json".to_string(), b"{}".to_vec()), ("d/x.bin".to_string(), vec![0u8, 1, 2, 255])];
        let z = write("p.atlas/", &files).unwrap();
        let back = read(&z).unwrap();
        assert_eq!(back[0].0, "p.atlas/a.json");
        assert_eq!(back[1].1, vec![0u8, 1, 2, 255]);
        assert_eq!(write("p.atlas/", &files).unwrap(), z);
    }
}
