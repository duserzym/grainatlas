//! Notice for UPb Lab after a sync or publish in the standalone app: one small JSON file in
//! UPb Lab's inbox, `<data dir>/edu.umn.upbstudio/inbox/atlas-synced/<mount id>.json`,
//! written to a temporary file in the same folder and renamed. UPb Lab reads and deletes it.
//! Nothing here launches or focuses UPb Lab.

use std::fs;
use std::path::{Path, PathBuf};

/// The lower-case canonical form of a hyphenated UUID, or None.
pub fn valid_uuid(value: &str) -> Option<String> {
    let v = value.trim();
    let groups: Vec<&str> = v.split('-').collect();
    let sizes = [8, 4, 4, 4, 12];
    if groups.len() != 5
        || groups
            .iter()
            .zip(sizes)
            .any(|(g, n)| g.len() != n || !g.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(v.to_ascii_lowercase())
}

/// The notice body (under 4 KB), after validating every field.
pub fn notice_json(
    mount_id: &str,
    project_uuid: &str,
    revision: i64,
    server: &str,
    at: &str,
) -> Result<(String, String), String> {
    let mount = valid_uuid(mount_id).ok_or("mount id is not a UUID")?;
    let project = valid_uuid(project_uuid).ok_or("project uuid is not a UUID")?;
    if revision < 0 {
        return Err("revision must be a nonnegative integer".into());
    }
    let server = server.trim();
    if !(server.starts_with("http://") || server.starts_with("https://"))
        || server.len() > 1024
        || server.chars().any(char::is_control)
    {
        return Err("server must be an http(s) URL of at most 1024 characters".into());
    }
    let body = serde_json::json!({
        "v": 1, "type": "lab.atlas.synced", "mount_id": mount, "project_uuid": project,
        "revision": revision, "server": server, "at": at, "source": "grainatlas-standalone",
    })
    .to_string();
    if body.len() >= 4096 {
        return Err("notice is too large".into());
    }
    Ok((mount, body))
}

/// Writes the notice under `data_dir` (the platform app-data base). UPb Lab's own folder
/// must exist already; only the inbox subfolders are created.
pub fn write_notice(
    data_dir: &Path,
    mount_id: &str,
    project_uuid: &str,
    revision: i64,
    server: &str,
    at: &str,
) -> Result<PathBuf, String> {
    let (mount, body) = notice_json(mount_id, project_uuid, revision, server, at)?;
    let lab = data_dir.join("edu.umn.upbstudio");
    if !lab.is_dir() {
        return Err(format!(
            "{} does not exist (UPb Lab not installed here)",
            lab.display()
        ));
    }
    let inbox = lab.join("inbox").join("atlas-synced");
    fs::create_dir_all(&inbox).map_err(|e| format!("{}: {e}", inbox.display()))?;
    let path = inbox.join(format!("{mount}.json"));
    crate::project_save::write(&path, body.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &str = "01972c4e-0000-7000-8000-0000000000B1";
    const P: &str = "01972c4e-0000-7000-8000-00000000a7a5";

    #[test]
    fn uuids_and_revision_are_validated() {
        assert_eq!(
            valid_uuid(M).as_deref(),
            Some("01972c4e-0000-7000-8000-0000000000b1")
        );
        for bad in [
            "",
            "../../etc/passwd",
            "01972c4e00007000800000000000000b",
            "01972c4e-0000-7000-8000-0000000000b1/x",
            "01972c4e-0000-7000-8000-0000000000bz",
            "01972c4e-0000-7000-8000-0000000000b1-",
        ] {
            assert!(valid_uuid(bad).is_none(), "{bad}");
        }
        assert!(notice_json("../x", P, 1, "http://h", "t").is_err());
        assert!(notice_json(M, "nope", 1, "http://h", "t").is_err());
        assert!(notice_json(M, P, -1, "http://h", "t").is_err());
        assert!(notice_json(M, P, 1, "file:///etc", "t").is_err());
        let (mount, body) =
            notice_json(M, P, 7, "http://localhost:8000", "2026-11-14T10:00:00Z").unwrap();
        assert_eq!(mount, "01972c4e-0000-7000-8000-0000000000b1");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["type"], "lab.atlas.synced");
        assert_eq!(v["revision"], 7);
        assert_eq!(v["project_uuid"], P);
        assert_eq!(v["source"], "grainatlas-standalone");
        assert!(body.len() < 4096);
    }

    #[test]
    fn writes_only_into_an_existing_lab_folder() {
        let base = std::env::temp_dir().join(format!("grainatlas-upblab-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        assert!(write_notice(&base, M, P, 3, "http://h", "t").is_err());
        assert!(!base.join("edu.umn.upbstudio").exists());
        fs::create_dir(base.join("edu.umn.upbstudio")).unwrap();
        let path = write_notice(&base, M, P, 3, "http://h", "t").unwrap();
        assert_eq!(
            path,
            base.join(
                "edu.umn.upbstudio/inbox/atlas-synced/01972c4e-0000-7000-8000-0000000000b1.json"
            )
        );
        let v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["revision"], 3);
        assert_eq!(
            fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1,
            "no temporary file left"
        );
        fs::remove_dir_all(&base).unwrap();
    }
}
