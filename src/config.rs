use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Microsoft,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Account {
    pub kind: Kind,
    pub name: String,
    pub uuid: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Most recently started profiles for this account (newest first, no duplicates).
    #[serde(default)]
    pub recent: Vec<String>,
}

impl Account {
    pub fn key(&self) -> String {
        format!("{:?}:{}", self.kind, self.uuid)
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub accounts: Vec<Account>,
    pub selected: String,
    pub ms_client_id: String,
    pub java_path: String,
    pub ram_mb: u32,
    pub last_version: String,
    /// Custom display names for profiles (profile id -> name).
    pub names: HashMap<String, String>,
    /// Game settings written into options.txt before each start.
    pub game: crate::options::GameOptions,
    /// Look for a new launcher release on GitHub at startup.
    pub check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            accounts: vec![],
            selected: String::new(),
            ms_client_id: String::new(),
            java_path: String::new(),
            ram_mb: 2048,
            last_version: String::new(),
            names: HashMap::new(),
            game: Default::default(),
            check_updates: true,
        }
    }
}

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/client_id.rs"));
}

fn deobfuscate(data: &[u8], key: &[u8]) -> String {
    // black_box keeps the compiler from folding the XOR back into a plain string constant
    let (data, key) = (std::hint::black_box(data), std::hint::black_box(key));
    data.iter().zip(key.iter().cycle()).map(|(d, k)| (d ^ k) as char).collect()
}

/// Client ID of the Azure app registration (public client). It is injected at build time via the
/// SMASH_CLIENT_ID environment variable in obfuscated form (see build.rs), so the repository does
/// not contain it. Users can always override it in the settings.
pub fn default_client_id() -> String {
    deobfuscate(embedded::DATA, embedded::KEY)
}

impl Config {
    /// Remembers a started profile under the account, newest first, without duplicates.
    pub fn remember(&mut self, key: &str, ver: &str) {
        if let Some(a) = self.accounts.iter_mut().find(|a| a.key() == key) {
            a.recent.retain(|v| v != ver);
            a.recent.insert(0, ver.to_string());
            a.recent.truncate(3);
        }
    }

    pub fn client_id(&self) -> String {
        let c = self.ms_client_id.trim();
        if c.is_empty() { default_client_id() } else { c.to_string() }
    }

    pub fn load(root: &Path) -> Self {
        let mut v: serde_json::Value = match fs::read_to_string(root.join("config.json")).ok().and_then(|s| serde_json::from_str(&s).ok()) {
            Some(v) => v,
            None => return Self::default(),
        };
        // Online only: drop accounts from older versions that were not Microsoft accounts.
        if let Some(a) = v["accounts"].as_array_mut() {
            a.retain(|x| x["kind"] == "Microsoft");
        }
        serde_json::from_value(v).unwrap_or_default()
    }

    pub fn save(&self, root: &Path) {
        let _ = fs::create_dir_all(root);
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = fs::write(root.join("config.json"), s);
        }
    }

    pub fn upsert(&mut self, acc: Account) {
        let k = acc.key();
        if let Some(a) = self.accounts.iter_mut().find(|a| a.key() == k) {
            let recent = std::mem::take(&mut a.recent);
            *a = acc;
            a.recent = recent;
        } else {
            self.selected = k;
            self.accounts.push(acc);
        }
    }
}

/// Name shown in the UI: custom name if set, otherwise the profile id.
pub fn display(names: &HashMap<String, String>, id: &str) -> String {
    names.get(id).filter(|n| !n.trim().is_empty()).cloned().unwrap_or_else(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(uuid: &str) -> Account {
        Account { kind: Kind::Microsoft, name: "A".into(), uuid: uuid.into(), refresh_token: None, recent: vec![] }
    }

    #[test]
    fn recent_is_unique_newest_first_and_survives_token_refresh() {
        let mut c = Config::default();
        c.upsert(acc("1"));
        let k = c.accounts[0].key();
        for v in ["a", "b", "a", "c", "d"] {
            c.remember(&k, v);
        }
        assert_eq!(c.accounts[0].recent, vec!["d", "c", "a"]);
        c.upsert(Account { refresh_token: Some("new".into()), ..acc("1") });
        assert_eq!(c.accounts[0].recent, vec!["d", "c", "a"]);
        assert_eq!(c.accounts[0].refresh_token.as_deref(), Some("new"));
    }

    #[test]
    fn deobfuscate_roundtrip() {
        let key = [7u8, 99, 200];
        let id = "600146ed-1d5b";
        let data: Vec<u8> = id.bytes().zip(key.iter().cycle()).map(|(b, k)| b ^ k).collect();
        assert_eq!(deobfuscate(&data, &key), id);
        assert_eq!(deobfuscate(&[], &[1]), "");
    }

    #[test]
    fn embedded_client_id_matches_build_env() {
        // Only meaningful when built with SMASH_CLIENT_ID set (release builds); empty otherwise.
        assert_eq!(default_client_id(), option_env!("SMASH_CLIENT_ID").unwrap_or(""));
    }

    #[test]
    fn display_uses_custom_name() {
        let mut n = HashMap::new();
        assert_eq!(display(&n, "x"), "x");
        n.insert("x".to_string(), "Mein Pack".to_string());
        assert_eq!(display(&n, "x"), "Mein Pack");
    }
}
