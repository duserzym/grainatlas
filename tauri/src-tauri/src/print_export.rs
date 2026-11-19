//! One dialog selects the PDF. Its registration sidecar always follows that path.
use std::{io, path::{Path, PathBuf}};

pub fn unpack(body: &[u8]) -> Result<(&[u8], Option<&[u8]>), String> {
    let length: [u8;4] = body.get(..4).ok_or("Missing print export header")?.try_into().unwrap();
    let n = u32::from_le_bytes(length) as usize;
    let json = body.get(4..4usize.checked_add(n).ok_or("Invalid registration length")?).ok_or("Incomplete registration")?;
    let pdf = body.get(4+n..).ok_or("Incomplete PDF")?;
    if !pdf.starts_with(b"%PDF-") { return Err("Invalid print PDF".into()); }
    if n > 0 { serde_json::from_slice::<serde_json::Value>(json).map_err(|e| format!("Invalid print registration: {e}"))?; }
    Ok((pdf, (n > 0).then_some(json)))
}
pub fn registration_path(pdf: &Path) -> io::Result<PathBuf> {
    let mut stem = pdf.file_stem().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput,"Missing PDF name"))?.to_os_string();
    stem.push("_registration.json"); Ok(pdf.with_file_name(stem))
}
pub fn write(path: &Path, pdf: &[u8], registration: Option<&[u8]>) -> io::Result<()> {
    // The PDF itself also contains this registration. Preserve complete files
    // with the same atomic replacement helper as project exports.
    crate::project_save::write(path, pdf)?;
    if let Some(json) = registration { crate::project_save::write(&registration_path(path)?, json)?; }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_packet_keeps_all_registration_bytes_and_uses_the_chosen_pdf_stem() {
        let json = br#"{"spots":[{"id":"registered"}]}"#;
        let mut body = (json.len() as u32).to_le_bytes().to_vec(); body.extend(json); body.extend(b"%PDF-1.7\nbody");
        let (pdf, reg) = unpack(&body).unwrap(); assert_eq!(pdf,b"%PDF-1.7\nbody"); assert_eq!(reg,Some(json.as_slice()));
        assert_eq!(registration_path(Path::new("chosen name.PDF")).unwrap(), PathBuf::from("chosen name_registration.json"));
        assert_eq!(registration_path(Path::new("chosen.name.pdf")).unwrap(), PathBuf::from("chosen.name_registration.json"));
        assert!(unpack(&body[..3]).is_err()); assert!(unpack(&body[..10]).is_err());
        let mut bad = body.clone(); bad[4] = 0xff; assert!(unpack(&bad).is_err());
    }
    #[test]
    fn writes_pdf_and_sidecar_without_a_second_path_choice() {
        let dir = std::env::temp_dir().join(format!("grainatlas-print-save-{}",std::process::id()));
        std::fs::create_dir_all(&dir).unwrap(); let pdf = dir.join("owner choice.pdf");
        write(&pdf,b"%PDF-1.7",Some(br#"{"id":"kept"}"#)).unwrap();
        assert_eq!(std::fs::read(&pdf).unwrap(),b"%PDF-1.7");
        assert_eq!(std::fs::read(registration_path(&pdf).unwrap()).unwrap(),br#"{"id":"kept"}"#);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
