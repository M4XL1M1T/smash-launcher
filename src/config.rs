use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

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
        }
    }
}

/// Client ID of the Azure app registration (public client). It is injected at build time via the
/// SMASH_CLIENT_ID environment variable, so the repository itself does not contain it.
/// Users can always override it in the settings.
pub const DEFAULT_CLIENT_ID: &str = match option_env!("SMASH_CLIENT_ID") {
    Some(v) => v,
    None => "",
};

impl Config {
    pub fn client_id(&self) -> String {
        let c = self.ms_client_id.trim();
        if c.is_empty() { DEFAULT_CLIENT_ID.to_string() } else { c.to_string() }
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
            *a = acc;
        } else {
            self.selected = k;
            self.accounts.push(acc);
        }
    }
}
