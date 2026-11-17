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

/// Folders and files of a game directory that are not shared (logs, caches, screenshots, worlds unless asked).
fn skipped(top: &str, include_worlds: bool) -> bool {
    matches!(
        top,
        "logs" | "crash-reports" | "screenshots" | ".fabric" | ".mixin.out" | "webcache" | "debug" | "resources" | "natives"
            | "usercache.json" | "command_history.txt" | "realms_persistence.json" | "launcher_profiles.json"
    ) || top.starts_with("hs_err_pid")
        || top.ends_with(".lock")
        || (top == "saves" && !include_worlds)
}

/// Minecraft version and loader of an installed profile, read from its version json. Vanilla profiles have no
/// json of their own, so their id is the Minecraft version.
fn profile_deps(root: &Path, id: &str) -> Result<Value> {
    let path = root.join("versions").join(id).join(format!("{id}.json"));
    let Ok(text) = fs::read_to_string(&path) else {
        if !simple(id) {
            bail!("This profile cannot be exported");
        }
        return Ok(json!({ "minecraft": id }));
    };
    let v: Value = serde_json::from_str(&text)?;
    let mc = v["inheritsFrom"].as_str().filter(|s| simple(s)).context("This profile has no Minecraft version, it cannot be exported")?;
    let mut deps = json!({ "minecraft": mc });
    let libs: Vec<&str> = v["libraries"].as_array().into_iter().flatten().filter_map(|l| l["name"].as_str()).collect();
    let version_of = |prefix: &str| libs.iter().find_map(|n| n.strip_prefix(prefix).map(|r| r.to_string()));
    if libs.iter().any(|n| n.starts_with("net.minecraftforge:") || n.starts_with("net.neoforged:")) {
        bail!("Forge and NeoForge profiles cannot be exported yet, only vanilla, Fabric and Quilt");
    } else if let Some(ver) = version_of("net.fabricmc:fabric-loader:") {
        deps["fabric-loader"] = json!(ver);
    } else if let Some(ver) = version_of("org.quiltmc:quilt-loader:") {
        deps["quilt-loader"] = json!(ver);
    }
    Ok(deps)
}

/// Exports a profile as a .mrpack file: loader and Minecraft version in the index, the game folder
/// (mods, config, resource packs, options, ...) as overrides. Other launchers and Smash Launcher can import it.
pub fn export(root: &Path, id: &str, name: &str, include_worlds: bool, dest: &Path, status: Status) -> Result<()> {
    status("Reading profile", 0.0);
    let deps = profile_deps(root, id)?;
    let index = json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "1.0.0",
        "name": name,
        "summary": "Exported from Smash Launcher",
        "files": [],
        "dependencies": deps,
    });

    let inst = root.join("instances").join(id);
    let mut files: Vec<(PathBuf, String)> = vec![];
    let mut stack: Vec<(PathBuf, String)> = vec![];
    if inst.is_dir() {
        for e in fs::read_dir(&inst)?.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !skipped(&n, include_worlds) {
                stack.push((e.path(), n));
            }
        }
    }
    while let Some((path, rel)) = stack.pop() {
        let Ok(meta) = fs::symlink_metadata(&path) else { continue };
        if meta.is_dir() {
            for e in fs::read_dir(&path)?.flatten() {
                stack.push((e.path(), format!("{rel}/{}", e.file_name().to_string_lossy())));
            }
        } else if meta.is_file() {
            files.push((path, rel));
        }
    }
    files.sort_by(|a, b| a.1.cmp(&b.1));

    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    {
        let mut zip = zip::ZipWriter::new(fs::File::create(&part).context("Could not create the export file")?);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("modrinth.index.json", opts)?;
        std::io::Write::write_all(&mut zip, serde_json::to_string_pretty(&index)?.as_bytes())?;
        let total = files.len().max(1);
        for (i, (path, rel)) in files.iter().enumerate() {
            zip.start_file(format!("overrides/{rel}"), opts)?;
            std::io::copy(&mut fs::File::open(path)?, &mut zip)?;
            if i % 20 == 0 {
                status("Packing files", 0.05 + 0.9 * i as f32 / total as f32);
            }
        }
        zip.finish()?;
    }
    fs::rename(&part, dest)?;
    status("Done", 1.0);
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_roundtrip_skips_logs_and_optionally_worlds() {
        let root = std::env::temp_dir().join(format!("smash_export_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("versions/pack")).unwrap();
        fs::write(
            root.join("versions/pack/pack.json"),
            r#"{"id":"pack","inheritsFrom":"1.21.1","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.5"}]}"#,
        )
        .unwrap();
        for f in ["instances/pack/mods/a.jar", "instances/pack/logs/latest.log", "instances/pack/saves/w/level.dat", "instances/pack/options.txt"] {
            fs::create_dir_all(root.join(f).parent().unwrap()).unwrap();
            fs::write(root.join(f), b"x").unwrap();
        }
        let names = |with_worlds: bool| {
            let dest = root.join("out.mrpack");
            export(&root, "pack", "My Pack", with_worlds, &dest, &|_, _| {}).unwrap();
            let mut z = zip::ZipArchive::new(fs::File::open(&dest).unwrap()).unwrap();
            let mut index = String::new();
            z.by_name("modrinth.index.json").unwrap().read_to_string(&mut index).unwrap();
            let v: Value = serde_json::from_str(&index).unwrap();
            assert_eq!(v["dependencies"]["minecraft"], "1.21.1");
            assert_eq!(v["dependencies"]["fabric-loader"], "0.16.5");
            assert_eq!(v["name"], "My Pack");
            (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect::<Vec<_>>()
        };
        let n = names(false);
        assert!(n.contains(&"overrides/mods/a.jar".to_string()) && n.contains(&"overrides/options.txt".to_string()));
        assert!(!n.iter().any(|x| x.contains("logs") || x.contains("saves")));
        assert!(names(true).contains(&"overrides/saves/w/level.dat".to_string()));
        // a vanilla profile has no json, the id is the version
        assert_eq!(profile_deps(&root, "1.20.4").unwrap()["minecraft"], "1.20.4");
        assert!(profile_deps(&root, "bad id!").is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
