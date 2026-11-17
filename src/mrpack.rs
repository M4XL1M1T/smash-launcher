//! Import of Modrinth modpacks (.mrpack). Supports Fabric, Quilt and vanilla packs.
use crate::{
    game::Status,
    net::{self, Task},
};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

/// The .mrpack spec only allows downloads from these hosts.
const ALLOWED_HOSTS: [&str; 4] = ["cdn.modrinth.com", "github.com", "raw.githubusercontent.com", "gitlab.com"];

fn safe_rel(p: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in Path::new(p).components() {
        match c {
            Component::Normal(x) => out.push(x),
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() { None } else { Some(out) }
}

fn host_ok(url: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|r| r.split('/').next())
        .is_some_and(|h| ALLOWED_HOSTS.contains(&h))
}

fn simple(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
}

fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else if c == ' ' { '-' } else { '_' })
        .take(40)
        .collect();
    let s = s.trim_matches(|c| c == '.' || c == '-' || c == '_').to_string();
    if s.is_empty() { "modpack".into() } else { s }
}

/// Imports a .mrpack file and returns the id of the created profile (also its instance folder name).
pub fn import(root: &Path, file: &Path, status: Status) -> Result<(String, String)> {
    status("Reading modpack", 0.0);
    let mut zip = zip::ZipArchive::new(fs::File::open(file).context("File could not be opened")?)
        .context("not a valid .mrpack file")?;
    let index: Value = {
        let mut f = zip
            .by_name("modrinth.index.json")
            .context("modrinth.index.json is missing, is this really a .mrpack file?")?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        serde_json::from_str(&s)?
    };
    if index["game"].as_str() != Some("minecraft") {
        bail!("This modpack is not for Minecraft");
    }
    let deps = &index["dependencies"];
    let mc = deps["minecraft"].as_str().filter(|s| simple(s)).context("Minecraft version missing in the modpack")?;
    let pack_name = index["name"].as_str().unwrap_or("modpack").trim().to_string();
    let id = sanitize(&pack_name);

    let loader_url = if let Some(v) = deps["fabric-loader"].as_str() {
        if !simple(v) {
            bail!("invalid Fabric version");
        }
        Some(format!("https://meta.fabricmc.net/v2/versions/loader/{mc}/{v}/profile/json"))
    } else if let Some(v) = deps["quilt-loader"].as_str() {
        if !simple(v) {
            bail!("invalid Quilt version");
        }
        Some(format!("https://meta.quiltmc.org/v3/versions/loader/{mc}/{v}/profile/json"))
    } else if deps.get("forge").is_some() || deps.get("neoforge").is_some() {
        bail!("Forge and NeoForge are not supported yet, only Fabric, Quilt and vanilla packs");
    } else {
        None
    };

    status("Loading loader profile", 0.05);
    let mut profile = match loader_url {
        Some(u) => net::get_json(&u).context("Loader profile could not be loaded")?,
        None => json!({}),
    };
    profile["id"] = json!(id);
    profile["inheritsFrom"] = json!(mc);
    let vdir = root.join("versions").join(&id);
    fs::create_dir_all(&vdir)?;
    fs::write(vdir.join(format!("{id}.json")), profile.to_string())?;

    let inst = root.join("instances").join(&id);
    fs::create_dir_all(&inst)?;

    let mut tasks = vec![];
    for f in index["files"].as_array().into_iter().flatten() {
        if f["env"]["client"].as_str() == Some("unsupported") {
            continue;
        }
        let p = f["path"].as_str().unwrap_or_default();
        let Some(rel) = safe_rel(p) else { bail!("Unsafe file path in the modpack: {p}") };
        let url = f["downloads"][0].as_str().context("File without download link")?;
        if !host_ok(url) {
            bail!("Download from a disallowed host rejected: {url}");
        }
        tasks.push(Task {
            url: url.into(),
            path: inst.join(rel),
            sha1: Some(f["hashes"]["sha1"].as_str().context("File without SHA-1")?.to_lowercase()),
            size: f["fileSize"].as_u64(),
        });
    }
    status("Loading mods", 0.1);
    net::download_all(tasks, &|d, t| status(&format!("Mods {d}/{t}"), 0.1 + 0.8 * d as f32 / t as f32))?;

    status("Copying configuration", 0.95);
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let name = f.name().to_string();
        let Some(rest) = name.strip_prefix("overrides/").or_else(|| name.strip_prefix("client-overrides/")) else { continue };
        if f.is_dir() {
            continue;
        }
        let Some(rel) = safe_rel(rest) else { continue };
        let dst = inst.join(rel);
        if let Some(p) = dst.parent() {
            fs::create_dir_all(p)?;
        }
        let mut out = fs::File::create(&dst)?;
        std::io::copy(&mut f, &mut out)?;
    }
    status("Done", 1.0);
    let shown = if pack_name.is_empty() { id.clone() } else { pack_name };
    Ok((id, shown))
}
