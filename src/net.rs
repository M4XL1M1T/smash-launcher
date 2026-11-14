use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    time::Duration,
};

pub fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(60))
            .user_agent("smash-launcher/0.1")
            .build()
    })
}

pub fn get_bytes(url: &str) -> Result<Vec<u8>> {
    let resp = agent().get(url).call().map_err(|e| anyhow!("{url}: {e}"))?;
    let mut buf = Vec::new();
    resp.into_reader().take(1 << 30).read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn get_json(url: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&get_bytes(url)?)?)
}

fn finish(r: Result<ureq::Response, ureq::Error>) -> Result<(u16, Value)> {
    match r {
        Ok(resp) => {
            let s = resp.status();
            Ok((s, resp.into_json().unwrap_or(Value::Null)))
        }
        Err(ureq::Error::Status(c, resp)) => Ok((c, resp.into_json().unwrap_or(Value::Null))),
        Err(e) => Err(anyhow!(e)),
    }
}

pub fn post_form(url: &str, form: &[(&str, &str)]) -> Result<(u16, Value)> {
    finish(agent().post(url).send_form(form))
}

pub fn post_json(url: &str, body: Value) -> Result<(u16, Value)> {
    finish(agent().post(url).set("Accept", "application/json").send_json(body))
}

pub fn get_auth(url: &str, bearer: &str) -> Result<(u16, Value)> {
    finish(agent().get(url).set("Authorization", &format!("Bearer {bearer}")).call())
}

#[derive(Clone)]
pub struct Task {
    pub url: String,
    pub path: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

fn already_ok(t: &Task) -> bool {
    let Ok(meta) = fs::metadata(&t.path) else { return false };
    if let Some(s) = t.size {
        return meta.len() == s;
    }
    if let Some(h) = &t.sha1 {
        return fs::read(&t.path).map(|b| &format!("{:x}", Sha1::digest(&b)) == h).unwrap_or(false);
    }
    true
}

fn fetch_one(t: &Task) -> Result<()> {
    if already_ok(t) {
        return Ok(());
    }
    let mut last = anyhow!("unbekannter Fehler");
    for _ in 0..3 {
        match get_bytes(&t.url) {
            Ok(b) => {
                if let Some(h) = &t.sha1 {
                    if &format!("{:x}", Sha1::digest(&b)) != h {
                        last = anyhow!("Pr\u{fc}fsumme falsch");
                        continue;
                    }
                }
                if let Some(p) = t.path.parent() {
                    fs::create_dir_all(p)?;
                }
                let mut part = t.path.clone().into_os_string();
                part.push(".part");
                let part = PathBuf::from(part);
                fs::write(&part, &b)?;
                fs::rename(&part, &t.path)?;
                return Ok(());
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Downloads all tasks in parallel, skipping files that are already present and valid.
pub fn download_all(tasks: Vec<Task>, on_progress: &(dyn Fn(usize, usize) + Sync)) -> Result<()> {
    let total = tasks.len();
    if total == 0 {
        return Ok(());
    }
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let errors = Mutex::new(Vec::<String>::new());
    std::thread::scope(|s| {
        for _ in 0..16.min(total) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= total {
                    break;
                }
                if let Err(e) = fetch_one(&tasks[i]) {
                    errors.lock().unwrap().push(format!("{}: {e}", tasks[i].url));
                }
                let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                on_progress(d, total);
            });
        }
    });
    let errs = errors.into_inner().unwrap();
    if !errs.is_empty() {
        bail!("{} Downloads fehlgeschlagen, z. B. {}", errs.len(), errs[0]);
    }
    Ok(())
}
