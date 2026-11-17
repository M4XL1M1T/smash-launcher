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
    status("Modpack lesen", 0.0);
    let mut zip = zip::ZipArchive::new(fs::File::open(file).context("Datei konnte nicht ge\u{f6}ffnet werden")?)
        .context("keine g\u{fc}ltige .mrpack-Datei")?;
    let index: Value = {
        let mut f = zip
            .by_name("modrinth.index.json")
            .context("modrinth.index.json fehlt, ist das wirklich eine .mrpack-Datei?")?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        serde_json::from_str(&s)?
    };
    if index["game"].as_str() != Some("minecraft") {
        bail!("Dieses Modpack ist nicht f\u{fc}r Minecraft");
    }
    let deps = &index["dependencies"];
    let mc = deps["minecraft"].as_str().filter(|s| simple(s)).context("Minecraft-Version fehlt im Modpack")?;
    let pack_name = index["name"].as_str().unwrap_or("modpack").trim().to_string();
    let id = sanitize(&pack_name);

    let loader_url = if let Some(v) = deps["fabric-loader"].as_str() {
        if !simple(v) {
            bail!("ung\u{fc}ltige Fabric-Version");
        }
        Some(format!("https://meta.fabricmc.net/v2/versions/loader/{mc}/{v}/profile/json"))
    } else if let Some(v) = deps["quilt-loader"].as_str() {
        if !simple(v) {
            bail!("ung\u{fc}ltige Quilt-Version");
        }
        Some(format!("https://meta.quiltmc.org/v3/versions/loader/{mc}/{v}/profile/json"))
    } else if deps.get("forge").is_some() || deps.get("neoforge").is_some() {
        bail!("Forge und NeoForge werden noch nicht unterst\u{fc}tzt, nur Fabric, Quilt und Vanilla-Packs");
    } else {
        None
    };

    status("Loader-Profil laden", 0.05);
    let mut profile = match loader_url {
        Some(u) => net::get_json(&u).context("Loader-Profil konnte nicht geladen werden")?,
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
        let Some(rel) = safe_rel(p) else { bail!("Unsicherer Dateipfad im Modpack: {p}") };
        let url = f["downloads"][0].as_str().context("Datei ohne Download-Link")?;
        if !host_ok(url) {
            bail!("Download von nicht erlaubtem Host abgelehnt: {url}");
        }
        tasks.push(Task {
            url: url.into(),
            path: inst.join(rel),
            sha1: Some(f["hashes"]["sha1"].as_str().context("Datei ohne SHA-1")?.to_lowercase()),
            size: f["fileSize"].as_u64(),
        });
    }
    status("Mods laden", 0.1);
    net::download_all(tasks, &|d, t| status(&format!("Mods {d}/{t}"), 0.1 + 0.8 * d as f32 / t as f32))?;

    status("Konfiguration kopieren", 0.95);
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
    status("Fertig", 1.0);
    let shown = if pack_name.is_empty() { id.clone() } else { pack_name };
    Ok((id, shown))
}
