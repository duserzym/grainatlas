//! UPb Lab's one-time handoff. Never format the code, argv, or request body in a log.
use serde::Serialize;

#[derive(Clone, Serialize, PartialEq)]
pub struct Launch {
    pub project: String,
    pub server: String,
    pub code: String,
    pub revision: Option<u64>,
}
impl std::fmt::Debug for Launch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Launch").field("project", &self.project).field("server", &self.server)
            .field("code", &"[redacted]").field("revision", &self.revision).finish()
    }
}
const FLAGS: [&str; 4] = ["--project", "--server", "--code", "--revision"];
pub fn parse(args: &[String]) -> Result<Option<Launch>, &'static str> {
    let mut values: [Option<&str>; 4] = [None; 4];
    let mut i = 0;
    while i < args.len() {
        if let Some(index) = FLAGS.iter().position(|flag| *flag == args[i]) {
            let value = args.get(i + 1).filter(|s| !s.is_empty() && !s.starts_with("--"))
                .ok_or("Missing launch argument")?;
            if values[index].replace(value).is_some() { return Err("Repeated launch argument"); }
            i += 1;
        } else if args[i] == "--profile" {
            // A profile name may resemble a flag; never interpret it as handoff content.
            i += 1;
        }
        i += 1;
    }
    if values.iter().all(Option::is_none) { return Ok(None); }
    let project = values[0].ok_or("Missing project")?;
    if project.len() > 128 || !project.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        || !project.bytes().all(|c| c.is_ascii_alphanumeric() || b"._~-".contains(&c)) {
        return Err("Invalid project id");
    }
    let server = values[1].ok_or("Missing server")?;
    if server.len() > 2048 || server.bytes().any(|c| c <= 32) { return Err("Invalid server"); }
    let url = tauri::Url::parse(server).map_err(|_| "Invalid server")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("Invalid server");
    }
    let code = values[2].filter(|s| !s.trim().is_empty()).ok_or("Missing code")?;
    let revision = values[3].map(|r| r.parse::<u64>().map_err(|_| "Invalid revision")).transpose()?;
    Ok(Some(Launch { project: project.to_owned(), server: server.to_owned(), code: code.to_owned(), revision }))
}
/// Keep codes and handoff values out of the ordinary filesystem opener too.
pub fn without_launch(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if FLAGS.contains(&args[i].as_str()) {
            if args.get(i + 1).is_some_and(|s| !s.starts_with("--")) { i += 1; }
        } else { out.push(args[i].clone()); }
        i += 1;
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(s: &[&str]) -> Vec<String> { s.iter().map(|s| s.to_string()).collect() }
    #[test]
    fn launch_and_redaction() {
        let a = args(&["--profile", "scratch", "--project", "p-1", "--server", "https://db.test/base/", "--code", "secret", "--revision", "12"]);
        let launch = parse(&a).unwrap().unwrap();
        assert_eq!(launch.revision, Some(12));
        assert_eq!(launch.code, "secret");
        assert!(!format!("{launch:?}").contains("secret"));
        assert_eq!(without_launch(&a), args(&["--profile", "scratch"]));
        assert!(parse(&args(&["package.gaproj", "--stitch"])).unwrap().is_none());
    }
    #[test]
    fn rejects_partial_repeated_and_invalid_args_without_echoing_secrets() {
        for tail in [vec![], vec!["--code"], vec!["--code", "secret", "--revision", "-1"], vec!["--code", "secret", "--code", "secret"]] {
            let mut a = args(&["--project", "p", "--server", "https://db.test"]);
            a.extend(args(&tail));
            assert!(parse(&a).is_err());
        }
        for server in ["file:///tmp", "https://", "https://db.test/?code=secret", "https://user:secret@db.test", "https://db.test/ bad"] {
            assert!(parse(&args(&["--project", "p", "--server", server, "--code", "secret"])).is_err());
        }
        assert!(parse(&args(&["--project", "a/b", "--server", "https://db.test", "--code", "secret"])).is_err());
    }
}
