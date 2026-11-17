//! Self update via GitHub Releases.
//!
//! The launcher asks the GitHub API for the latest release of its own repository. If the tag is newer
//! than the running version, the release exe is downloaded, checked against `SHA256SUMS.txt` of the same
//! release and swapped in place of the running exe (Windows allows renaming a running exe, so the old
//! one is moved to `<exe>.old` and removed on the next start). Nothing is sent to GitHub except the
//! usual HTTP request, no account data and no telemetry.
use crate::net;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "M4XL1M1T/smash-launcher";
const SUMS_NAME: &str = "SHA256SUMS.txt";
const EXE_SUFFIX: &str = "-windows-x64.exe";
const MAX_EXE_BYTES: usize = 200 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Info {
    pub version: String,
    pub exe_name: String,
    pub exe_url: String,
    pub sums_url: String,
    pub page: String,
}

/// Parses "v1.2.3" or "1.2.3-beta" into [1, 2, 3]. Returns None if it is not a version number.
pub fn parse_version(s: &str) -> Option<Vec<u64>> {
    let core = s.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let parts: Option<Vec<u64>> = core.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

pub fn is_newer(remote: &str, local: &str) -> bool {
    match (parse_version(remote), parse_version(local)) {
        (Some(mut r), Some(mut l)) => {
            let n = r.len().max(l.len());
            r.resize(n, 0);
            l.resize(n, 0);
            r > l
        }
        _ => false,
    }
}

/// Only files from our own release downloads are accepted, whatever the API answers.
fn trusted(url: &str) -> bool {
    url.starts_with(&format!("https://github.com/{REPO}/releases/download/"))
}

/// Looks for a newer release. Ok(None) means the running version is current.
pub fn check() -> Result<Option<Info>> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = net::agent()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => anyhow!("No release published on GitHub yet"),
            e => anyhow!("{e}"),
        })?;
    let json: Value = resp.into_json()?;
    parse_release(&json, CURRENT)
}

fn parse_release(json: &Value, local: &str) -> Result<Option<Info>> {
    let tag = json["tag_name"].as_str().ok_or_else(|| anyhow!("Release without a version number"))?;
    if !is_newer(tag, local) {
        return Ok(None);
    }
    let assets = json["assets"].as_array().ok_or_else(|| anyhow!("Release without files"))?;
    let find = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let (name, url) = (a["name"].as_str()?, a["browser_download_url"].as_str()?);
            (pred(name) && trusted(url)).then(|| (name.to_string(), url.to_string()))
        })
    };
    let (exe_name, exe_url) = find(&|n| n.ends_with(EXE_SUFFIX)).ok_or_else(|| anyhow!("Release contains no Windows file"))?;
    let (_, sums_url) = find(&|n| n == SUMS_NAME).ok_or_else(|| anyhow!("Release contains no {SUMS_NAME}, update rejected for security reasons"))?;
    Ok(Some(Info {
        version: tag.trim_start_matches('v').to_string(),
        exe_name,
        exe_url,
        sums_url,
        page: json["html_url"].as_str().unwrap_or("").to_string(),
    }))
}

/// Finds the expected SHA-256 of `name` in the text of a SHA256SUMS file ("<hash>  <name>" per line).
fn expected_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (h, n) = l.trim().split_once(char::is_whitespace)?;
        let n = n.trim().trim_start_matches('*');
        (n == name && h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| h.to_ascii_lowercase())
    })
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = p.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Downloads, verifies and swaps the exe. After Ok the new version is in place of the running exe,
/// the launcher should restart.
pub fn install(info: &Info, on_step: &dyn Fn(&str, f32)) -> Result<()> {
    on_step("Loading checksum ...", 0.05);
    let sums = String::from_utf8(net::get_bytes(&info.sums_url)?).context("SHA256SUMS.txt is not text")?;
    let want = expected_hash(&sums, &info.exe_name).ok_or_else(|| anyhow!("No checksum found for {}", info.exe_name))?;

    on_step("Downloading update ...", 0.15);
    let bytes = net::get_bytes(&info.exe_url)?;
    if bytes.len() > MAX_EXE_BYTES || bytes.len() < 1024 * 1024 {
        bail!("Downloaded file has an implausible size ({} bytes)", bytes.len());
    }
    on_step("Verifying update ...", 0.8);
    let got = format!("{:x}", Sha256::digest(&bytes));
    if got != want {
        bail!("Checksum does not match, update was not installed");
    }
    if !bytes.starts_with(b"MZ") {
        bail!("Downloaded file is not a Windows executable");
    }

    on_step("Installing update ...", 0.9);
    let exe = std::env::current_exe()?;
    let (new, old) = (with_suffix(&exe, ".new"), with_suffix(&exe, ".old"));
    fs::write(&new, &bytes).context("New file could not be written next to the program")?;
    let _ = fs::remove_file(&old);
    if let Err(e) = fs::rename(&exe, &old) {
        let _ = fs::remove_file(&new);
        bail!("Current program could not be replaced: {e}");
    }
    if let Err(e) = fs::rename(&new, &exe) {
        let _ = fs::rename(&old, &exe);
        let _ = fs::remove_file(&new);
        bail!("New version could not be put in place: {e}");
    }
    Ok(())
}

/// Removes leftovers of a previous update. Call once at startup.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = fs::remove_file(with_suffix(&exe, ".old"));
        let _ = fs::remove_file(with_suffix(&exe, ".new"));
    }
}

/// Starts the (already replaced) exe again.
pub fn restart() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.1.1"));
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(is_newer("0.2.0-beta", "0.1.0"));
    }

    #[test]
    fn sums_lookup() {
        let h = "a".repeat(64);
        let s = format!("{h}  smash-launcher-v0.2.0-windows-x64.exe\n{}  other.exe\n", "b".repeat(64));
        assert_eq!(expected_hash(&s, "smash-launcher-v0.2.0-windows-x64.exe"), Some(h));
        assert_eq!(expected_hash(&s, "missing.exe"), None);
        assert_eq!(expected_hash("zzzz  x.exe", "x.exe"), None);
    }

    #[test]
    fn release_parsing_only_trusts_own_downloads() {
        let base = format!("https://github.com/{REPO}/releases/download/v0.2.0");
        let rel = |exe_url: String| {
            serde_json::json!({
                "tag_name": "v0.2.0", "html_url": "https://github.com/x",
                "assets": [
                    {"name": "smash-launcher-v0.2.0-windows-x64.exe", "browser_download_url": exe_url},
                    {"name": "SHA256SUMS.txt", "browser_download_url": format!("{base}/SHA256SUMS.txt")},
                ]
            })
        };
        let info = parse_release(&rel(format!("{base}/smash-launcher-v0.2.0-windows-x64.exe")), "0.1.0").unwrap().unwrap();
        assert_eq!(info.version, "0.2.0");
        assert!(parse_release(&rel("https://evil.example/a-windows-x64.exe".into()), "0.1.0").is_err());
        assert!(parse_release(&rel(format!("{base}/x-windows-x64.exe")), "0.2.0").unwrap().is_none());
    }
}
