//! Separate browser storage for development and named test launches.
use sha2::{Digest, Sha256};

pub fn name(args: &[String], env: Option<&str>, debug: bool) -> Option<String> {
    args.iter().position(|a| a == "--profile")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty() && !s.starts_with("--"))
        .or_else(|| env.filter(|s| !s.trim().is_empty()))
        .map(str::to_owned)
        .or_else(|| debug.then(|| "dev".to_owned()))
}

/// The profile value can itself be an existing directory; never pass it to the opener.
pub fn without_profile(args: &[String]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--profile" {
            if args.clone().next().is_some_and(|value| !value.starts_with("--")) {
                args.next();
            }
        } else {
            paths.push(arg.clone());
        }
    }
    paths
}

pub fn identifier(name: &str) -> [u8; 16] {
    let digest = Sha256::digest(name.as_bytes());
    let mut id = [0; 16];
    id.copy_from_slice(&digest[..16]);
    id
}

/// Keep arbitrary profile names inside the profiles directory, including names containing `/`.
pub fn directory_name(name: &str) -> String {
    name.bytes().map(|b| {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            char::from(b).to_string()
        } else {
            format!("%{b:02X}")
        }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn profile_from_env_cli_or_debug_default() {
        assert_eq!(name(&[], Some("test"), false).as_deref(), Some("test"));
        assert_eq!(name(&args(&["--profile", "scratch"]), Some("test"), true).as_deref(), Some("scratch"));
        assert_eq!(name(&[], None, true).as_deref(), Some("dev"));
        assert_eq!(name(&[], Some(""), false), None);
        assert_eq!(name(&[], Some("  "), false), None);
        assert_eq!(name(&args(&["--profile"]), None, false), None);
        assert_eq!(name(&args(&["--profile", "--stitch"]), None, false), None);
    }

    #[test]
    fn startup_skips_profile_and_its_value() {
        let directory = std::env::current_dir().unwrap().display().to_string();
        assert_eq!(without_profile(&args(&["--profile", &directory, "mount.gaproj", "--stitch"])), args(&["mount.gaproj", "--stitch"]));
        assert_eq!(without_profile(&args(&["--profile", "--stitch"])), args(&["--stitch"]));
    }

    #[test]
    fn store_id_is_stable_and_names_cannot_escape_the_data_directory() {
        assert_eq!(identifier("test"), [159, 134, 208, 129, 136, 76, 125, 101, 154, 47, 234, 160, 197, 90, 208, 21]);
        assert_ne!(identifier("test"), identifier("dev"));
        assert_eq!(directory_name("test"), "test");
        assert_eq!(directory_name("../test"), "%2E%2E%2Ftest");
    }
}
