use crate::{
    auth::Session,
    net::{self, Task},
};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const RUNTIME_URL: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
const ASSET_URL: &str = "https://resources.download.minecraft.net";

#[cfg(windows)]
const OS_NAME: &str = "windows";
#[cfg(target_os = "linux")]
const OS_NAME: &str = "linux";
#[cfg(target_os = "macos")]
const OS_NAME: &str = "osx";

#[cfg(windows)]
const CP_SEP: &str = ";";
#[cfg(not(windows))]
const CP_SEP: &str = ":";

pub type Status<'a> = &'a (dyn Fn(&str, f32) + Sync);

#[derive(Clone)]
pub struct VersionEntry {
    pub id: String,
    pub kind: String,
    pub url: String,
}

fn read_json(p: &Path) -> Result<Value> {
    Ok(serde_json::from_str(&fs::read_to_string(p).with_context(|| p.display().to_string())?)?)
}

fn version_json(root: &Path, id: &str) -> PathBuf {
    root.join("versions").join(id).join(format!("{id}.json"))
}

fn jar_path(root: &Path, id: &str) -> PathBuf {
    root.join("versions").join(id).join(format!("{id}.jar"))
}

pub fn fetch_manifest(root: &Path) -> Result<Vec<VersionEntry>> {
    let cache = root.join("version_manifest.json");
    let v = match net::get_json(MANIFEST_URL) {
        Ok(v) => {
            let _ = fs::create_dir_all(root);
            let _ = fs::write(&cache, v.to_string());
            v
        }
        Err(e) => {
            let s = fs::read_to_string(&cache).map_err(|_| e)?;
            serde_json::from_str(&s)?
        }
    };
    Ok(v["versions"]
        .as_array()
        .context("Manifest ohne Versionen")?
        .iter()
        .filter_map(|x| {
            Some(VersionEntry {
                id: x["id"].as_str()?.into(),
                kind: x["type"].as_str()?.into(),
                url: x["url"].as_str()?.into(),
            })
        })
        .collect())
}

/// Version folders that already have a json (includes custom/modded profiles).
pub fn local_versions(root: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    if let Ok(rd) = fs::read_dir(root.join("versions")) {
        for e in rd.flatten() {
            let id = e.file_name().to_string_lossy().to_string();
            let json = version_json(root, &id);
            if let Ok(s) = fs::read_to_string(&json) {
                if jar_path(root, &id).exists() || s.contains("inheritsFrom") {
                    out.insert(id);
                }
            }
        }
    }
    out
}

/// Loads a version json and resolves `inheritsFrom` (Fabric/Forge style profiles).
fn load_merged(root: &Path, id: &str) -> Result<Value> {
    let child = read_json(&version_json(root, id))?;
    let mut v = match child["inheritsFrom"].as_str() {
        Some(parent) => {
            let mut m = load_merged(root, parent)?;
            for (k, val) in child.as_object().context("ung\u{fc}ltige Version")? {
                match k.as_str() {
                    "libraries" => {
                        let mut libs = val.as_array().cloned().unwrap_or_default();
                        libs.extend(m["libraries"].as_array().cloned().unwrap_or_default());
                        m["libraries"] = Value::Array(libs);
                    }
                    "arguments" => {
                        for part in ["jvm", "game"] {
                            let mut a = m["arguments"][part].as_array().cloned().unwrap_or_default();
                            a.extend(val[part].as_array().cloned().unwrap_or_default());
                            m["arguments"][part] = Value::Array(a);
                        }
                    }
                    _ => m[k.as_str()] = val.clone(),
                }
            }
            m
        }
        None => {
            let mut c = child;
            c["_jar"] = json!(id);
            c
        }
    };
    if v["_jar"].is_null() {
        v["_jar"] = json!(id);
    }
    Ok(v)
}

fn rules_ok(rules: Option<&Value>) -> bool {
    let Some(arr) = rules.and_then(|r| r.as_array()) else { return true };
    if arr.is_empty() {
        return true;
    }
    let mut allowed = false;
    for r in arr {
        let os_ok = r.get("os").map_or(true, |o| {
            o["name"].as_str().map_or(true, |n| n == OS_NAME)
                && o["arch"].as_str().map_or(true, |a| a != "x86" || cfg!(target_arch = "x86"))
        });
        // No optional launcher features (demo mode, custom resolution, quick play) are enabled.
        if os_ok && r.get("features").is_none() {
            allowed = r["action"].as_str() == Some("allow");
        }
    }
    allowed
}

struct Lib {
    task: Task,
    native: bool,
    exclude: Vec<String>,
}

fn maven_path(name: &str) -> Option<String> {
    let mut p = name.split(':');
    let (g, a, v) = (p.next()?, p.next()?, p.next()?);
    Some(format!("{}/{a}/{v}/{a}-{v}.jar", g.replace('.', "/")))
}

fn collect_libs(root: &Path, v: &Value) -> Vec<Lib> {
    let mut out: Vec<Lib> = vec![];
    let libdir = root.join("libraries");
    for lib in v["libraries"].as_array().into_iter().flatten() {
        if !rules_ok(lib.get("rules")) {
            continue;
        }
        let dl = &lib["downloads"];
        let mk = |a: &Value| -> Option<Task> {
            Some(Task {
                url: a["url"].as_str()?.into(),
                path: libdir.join(a["path"].as_str()?),
                sha1: a["sha1"].as_str().map(Into::into),
                size: a["size"].as_u64(),
            })
        };
        if dl.is_null() {
            // Maven style entry without downloads block (Fabric etc.)
            if let (Some(name), Some(p)) = (lib["name"].as_str(), lib["name"].as_str().and_then(maven_path)) {
                let _ = name;
                let base = lib["url"].as_str().unwrap_or("https://libraries.minecraft.net/");
                out.push(Lib {
                    task: Task {
                        url: format!("{}{}", base.trim_end_matches('/').to_owned() + "/", p),
                        path: libdir.join(&p),
                        sha1: None,
                        size: None,
                    },
                    native: false,
                    exclude: vec![],
                });
            }
            continue;
        }
        if let Some(t) = dl.get("artifact").and_then(mk) {
            out.push(Lib { task: t, native: false, exclude: vec![] });
        }
        if let Some(cls) = lib["natives"][OS_NAME].as_str() {
            let key = cls.replace("${arch}", "64");
            if let Some(t) = dl["classifiers"].get(&key).and_then(mk) {
                let exclude = lib["extract"]["exclude"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                out.push(Lib { task: t, native: true, exclude });
            }
        }
    }
    out
}

fn ensure_json(root: &Path, id: &str, manifest: &[VersionEntry]) -> Result<()> {
    let p = version_json(root, id);
    if !p.exists() {
        let e = manifest
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow!("Version {id} nicht gefunden"))?;
        net::download_all(vec![Task { url: e.url.clone(), path: p.clone(), sha1: None, size: None }], &|_, _| {})?;
    }
    if let Some(parent) = read_json(&p)?["inheritsFrom"].as_str() {
        ensure_json(root, parent, manifest)?;
    }
    Ok(())
}

/// Downloads client jar, libraries and assets for a version. Returns the merged version json.
pub fn ensure_installed(root: &Path, id: &str, manifest: &[VersionEntry], status: Status) -> Result<Value> {
    status("Versionsdaten laden", 0.0);
    ensure_json(root, id, manifest)?;
    let v = load_merged(root, id)?;
    let jar_id = v["_jar"].as_str().unwrap_or(id).to_string();

    let client = &v["downloads"]["client"];
    let mut tasks = vec![Task {
        url: client["url"].as_str().context("Client-Download fehlt in der Versionsdatei")?.into(),
        path: jar_path(root, &jar_id),
        sha1: client["sha1"].as_str().map(Into::into),
        size: client["size"].as_u64(),
    }];
    tasks.extend(collect_libs(root, &v).into_iter().map(|l| l.task));

    let idx = &v["assetIndex"];
    let idx_id = idx["id"].as_str().context("assetIndex fehlt")?;
    let idx_path = root.join("assets").join("indexes").join(format!("{idx_id}.json"));
    tasks.push(Task {
        url: idx["url"].as_str().context("assetIndex-URL fehlt")?.into(),
        path: idx_path.clone(),
        sha1: idx["sha1"].as_str().map(Into::into),
        size: idx["size"].as_u64(),
    });
    status("Bibliotheken laden", 0.02);
    net::download_all(tasks, &|d, t| status(&format!("Bibliotheken {d}/{t}"), 0.02 + 0.28 * d as f32 / t as f32))?;

    let index = read_json(&idx_path)?;
    let objs: Vec<Task> = index["objects"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, o)| {
            let h = o["hash"].as_str()?;
            Some(Task {
                url: format!("{ASSET_URL}/{}/{h}", &h[..2]),
                path: root.join("assets").join("objects").join(&h[..2]).join(h),
                sha1: Some(h.into()),
                size: o["size"].as_u64(),
            })
        })
        .collect();
    status("Assets laden", 0.3);
    net::download_all(objs, &|d, t| status(&format!("Assets {d}/{t}"), 0.3 + 0.7 * d as f32 / t as f32))?;
    Ok(v)
}

fn runtime_platform() -> &'static str {
    if cfg!(all(windows, target_arch = "x86_64")) {
        "windows-x64"
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        "windows-arm64"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") { "mac-os-arm64" } else { "mac-os" }
    } else {
        "linux"
    }
}

/// Returns the java executable: user override, otherwise Mojang's bundled runtime for this version.
pub fn ensure_java(root: &Path, v: &Value, custom: &str, status: Status) -> Result<PathBuf> {
    if !custom.trim().is_empty() {
        return Ok(PathBuf::from(custom.trim()));
    }
    let component = v["javaVersion"]["component"].as_str().unwrap_or("jre-legacy");
    let dir = root.join("runtime").join(component);
    let exe = dir.join("bin").join(if cfg!(windows) { "java.exe" } else { "java" });
    let marker = dir.join(".complete");
    if marker.exists() && exe.exists() {
        return Ok(exe);
    }
    status(&format!("Java ({component}) laden"), 0.0);
    let all = net::get_json(RUNTIME_URL)?;
    let manifest_url = all[runtime_platform()][component][0]["manifest"]["url"]
        .as_str()
        .ok_or_else(|| anyhow!("Kein Java {component} f\u{fc}r {} verf\u{fc}gbar, bitte Java-Pfad in den Einstellungen setzen", runtime_platform()))?;
    let manifest = net::get_json(manifest_url)?;
    let mut tasks = vec![];
    let mut exec = vec![];
    for (name, f) in manifest["files"].as_object().into_iter().flatten() {
        if f["type"].as_str() != Some("file") {
            continue;
        }
        let raw = &f["downloads"]["raw"];
        let path = dir.join(name);
        if f["executable"].as_bool() == Some(true) {
            exec.push(path.clone());
        }
        tasks.push(Task {
            url: raw["url"].as_str().context("Runtime-Datei ohne URL")?.into(),
            path,
            sha1: raw["sha1"].as_str().map(Into::into),
            size: raw["size"].as_u64(),
        });
    }
    net::download_all(tasks, &|d, t| status(&format!("Java {d}/{t}"), d as f32 / t as f32))?;
    #[cfg(unix)]
    for p in exec {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = exec;
    fs::write(&marker, "ok")?;
    if !exe.exists() {
        bail!("Java-Runtime unvollst\u{e4}ndig: {} fehlt", exe.display());
    }
    Ok(exe)
}

fn subst(s: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = s.to_string();
    for (k, val) in vars {
        out = out.replace(&format!("${{{k}}}"), val);
    }
    out
}

fn expand_args(arr: &Value, vars: &HashMap<&str, String>, out: &mut Vec<String>) {
    for a in arr.as_array().into_iter().flatten() {
        match a {
            Value::String(s) => out.push(subst(s, vars)),
            Value::Object(_) if rules_ok(a.get("rules")) => match &a["value"] {
                Value::String(s) => out.push(subst(s, vars)),
                Value::Array(l) => out.extend(l.iter().filter_map(|x| x.as_str()).map(|s| subst(s, vars))),
                _ => {}
            },
            _ => {}
        }
    }
}

fn extract_natives(jar: &Path, exclude: &[String], dest: &Path) -> Result<()> {
    let mut z = zip::ZipArchive::new(fs::File::open(jar)?)?;
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        let name = f.name().to_string();
        if f.is_dir() || name.starts_with("META-INF") || exclude.iter().any(|e| name.starts_with(e.as_str())) {
            continue;
        }
        let Some(file_name) = Path::new(&name).file_name() else { continue };
        if let Ok(mut out) = fs::File::create(dest.join(file_name)) {
            std::io::copy(&mut f, &mut out)?;
        }
    }
    Ok(())
}

/// Builds the command line and runs the game until it exits. Returns the exit code.
pub fn run_game(
    root: &Path,
    v: &Value,
    session: &Session,
    java: &Path,
    ram_mb: u32,
    log: &(dyn Fn(String) + Sync),
) -> Result<i32> {
    let id = v["id"].as_str().unwrap_or("unknown");
    let jar_id = v["_jar"].as_str().unwrap_or(id);
    let game_dir = root.join("instances").join(id);
    fs::create_dir_all(&game_dir)?;
    let natives = root.join("versions").join(id).join("natives");
    let _ = fs::remove_dir_all(&natives);
    fs::create_dir_all(&natives)?;

    let mut cp: Vec<String> = vec![];
    for l in collect_libs(root, v) {
        if l.native {
            extract_natives(&l.task.path, &l.exclude, &natives)?;
        } else {
            let p = l.task.path.display().to_string();
            if !cp.contains(&p) {
                cp.push(p);
            }
        }
    }
    cp.push(jar_path(root, jar_id).display().to_string());

    let assets_root = root.join("assets");
    let idx_id = v["assetIndex"]["id"].as_str().unwrap_or("legacy");
    let mut game_assets = assets_root.clone();
    let index = read_json(&assets_root.join("indexes").join(format!("{idx_id}.json")))?;
    let is_virtual = index["virtual"].as_bool() == Some(true);
    let to_resources = index["map_to_resources"].as_bool() == Some(true);
    if is_virtual || to_resources {
        let target_base = if is_virtual {
            game_assets = assets_root.join("virtual").join(idx_id);
            game_assets.clone()
        } else {
            game_dir.join("resources")
        };
        for (name, o) in index["objects"].as_object().into_iter().flatten() {
            let Some(h) = o["hash"].as_str() else { continue };
            let dst = target_base.join(name);
            if !dst.exists() {
                if let Some(p) = dst.parent() {
                    fs::create_dir_all(p)?;
                }
                fs::copy(assets_root.join("objects").join(&h[..2]).join(h), &dst)?;
            }
        }
    }

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("auth_player_name", session.name.clone());
    vars.insert("version_name", id.into());
    vars.insert("game_directory", game_dir.display().to_string());
    vars.insert("assets_root", assets_root.display().to_string());
    vars.insert("assets_index_name", idx_id.into());
    vars.insert("auth_uuid", session.uuid.clone());
    vars.insert("auth_access_token", session.token.clone());
    vars.insert("auth_session", session.token.clone());
    vars.insert("auth_xuid", "0".into());
    vars.insert("clientid", "0".into());
    vars.insert("user_type", session.user_type.into());
    vars.insert("user_properties", "{}".into());
    vars.insert("version_type", v["type"].as_str().unwrap_or("release").into());
    vars.insert("natives_directory", natives.display().to_string());
    vars.insert("launcher_name", "smash-launcher".into());
    vars.insert("launcher_version", "0.1".into());
    vars.insert("classpath", cp.join(CP_SEP));
    vars.insert("game_assets", game_assets.display().to_string());

    let mut args: Vec<String> = vec![format!("-Xmx{ram_mb}M"), "-Xms256M".into()];
    if v["arguments"].is_object() {
        expand_args(&v["arguments"]["jvm"], &vars, &mut args);
        args.push(v["mainClass"].as_str().context("mainClass fehlt")?.into());
        expand_args(&v["arguments"]["game"], &vars, &mut args);
    } else {
        for a in [
            "-Djava.library.path=${natives_directory}",
            "-Dminecraft.launcher.brand=${launcher_name}",
            "-Dminecraft.launcher.version=${launcher_version}",
            "-cp",
            "${classpath}",
        ] {
            args.push(subst(a, &vars));
        }
        args.push(v["mainClass"].as_str().context("mainClass fehlt")?.into());
        for a in v["minecraftArguments"].as_str().unwrap_or_default().split_whitespace() {
            args.push(subst(a, &vars));
        }
    }

    let mut cmd = Command::new(java);
    cmd.args(&args).current_dir(&game_dir).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().with_context(|| format!("Java konnte nicht gestartet werden: {}", java.display()))?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let code = std::thread::scope(|s| {
        s.spawn(|| BufReader::new(out).lines().map_while(Result::ok).for_each(log));
        s.spawn(|| BufReader::new(err).lines().map_while(Result::ok).for_each(log));
        child.wait()
    })?;
    Ok(code.code().unwrap_or(-1))
}
