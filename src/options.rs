//! Game settings that the launcher writes into the profile's options.txt before every start.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, collections::HashSet, fs, io, path::Path};

/// Sound categories of the game: (key in options.txt without prefix, label shown in the launcher).
pub const SOUNDS: [(&str, &str); 10] = [
    ("master", "Gesamtlautst\u{e4}rke (Master)"),
    ("music", "Musik"),
    ("record", "Plattenspieler / Notenbl\u{f6}cke"),
    ("weather", "Wetter"),
    ("block", "Bl\u{f6}cke"),
    ("hostile", "Feindselige Kreaturen"),
    ("neutral", "Freundliche Kreaturen"),
    ("player", "Spieler"),
    ("ambient", "Umgebung"),
    ("voice", "Stimmen / Dialoge"),
];

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Volume {
    pub on: bool,
    pub pct: u32,
}

impl Default for Volume {
    fn default() -> Self {
        Self { on: false, pct: 30 }
    }
}

/// Each value only takes effect when its `on` flag is set; everything else stays as the game has it.
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct GameOptions {
    /// Volume per sound category, keyed by the ids in `SOUNDS`.
    pub volumes: BTreeMap<String, Volume>,
    pub subtitles_on: bool,
    pub subtitles: bool,
    pub directional_on: bool,
    pub directional: bool,
    pub gui_on: bool,
    /// 0 = auto, 1 to 4 = fixed scale
    pub gui_scale: u32,
    pub fov_on: bool,
    pub fov: u32,
    pub fps_on: bool,
    pub max_fps: u32,
    pub dist_on: bool,
    pub render_distance: u32,
    pub fullscreen_on: bool,
    pub fullscreen: bool,
}

impl Default for GameOptions {
    fn default() -> Self {
        Self {
            volumes: BTreeMap::new(),
            subtitles_on: false,
            subtitles: false,
            directional_on: false,
            directional: false,
            gui_on: false,
            gui_scale: 2,
            fov_on: false,
            fov: 70,
            max_fps: 120,
            fps_on: false,
            dist_on: false,
            render_distance: 12,
            fullscreen_on: false,
            fullscreen: false,
        }
    }
}

impl GameOptions {
    pub fn volume_mut(&mut self, id: &str) -> &mut Volume {
        self.volumes.entry(id.to_string()).or_default()
    }
}

fn vol(pct: u32) -> String {
    format!("{:.2}", pct.min(100) as f32 / 100.0)
}

fn wanted(o: &GameOptions) -> Vec<(String, String)> {
    let mut set: Vec<(String, String)> = vec![];
    for (id, _) in SOUNDS {
        if let Some(v) = o.volumes.get(id).filter(|v| v.on) {
            set.push((format!("soundCategory_{id}"), vol(v.pct)));
        }
    }
    if o.subtitles_on {
        set.push(("showSubtitles".into(), o.subtitles.to_string()));
    }
    if o.directional_on {
        set.push(("directionalAudio".into(), o.directional.to_string()));
    }
    if o.gui_on {
        set.push(("guiScale".into(), o.gui_scale.min(4).to_string()));
    }
    if o.fov_on {
        set.push(("fov".into(), format!("{:.2}", (o.fov.clamp(30, 110) as f32 - 70.0) / 40.0)));
    }
    if o.fps_on {
        set.push(("maxFps".into(), o.max_fps.clamp(10, 260).to_string()));
    }
    if o.dist_on {
        set.push(("renderDistance".into(), o.render_distance.clamp(2, 32).to_string()));
    }
    if o.fullscreen_on {
        set.push(("fullscreen".into(), o.fullscreen.to_string()));
    }
    set
}

/// Patches `<game_dir>/options.txt`: replaces the wanted keys, keeps every other line as it is.
pub fn apply(game_dir: &Path, o: &GameOptions) -> io::Result<()> {
    let set = wanted(o);
    if set.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(game_dir)?;
    let path = game_dir.join("options.txt");
    let old = fs::read_to_string(&path).unwrap_or_default();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out: Vec<String> = vec![];
    for line in old.lines() {
        let key = line.split(':').next().unwrap_or("");
        match set.iter().find(|(k, _)| k == key) {
            Some((k, v)) => {
                out.push(format!("{k}:{v}"));
                seen.insert(k.as_str());
            }
            None => out.push(line.to_string()),
        }
    }
    for (k, v) in &set {
        if !seen.contains(k.as_str()) {
            out.push(format!("{k}:{v}"));
        }
    }
    fs::write(&path, out.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn patches_existing_and_adds_missing_keys() {
        let dir = tmp("smash_options_test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("options.txt"), "version:3955\nguiScale:3\nsoundCategory_master:1.0\nlang:de_de\n").unwrap();
        let mut o = GameOptions { gui_on: true, gui_scale: 2, fov_on: true, fov: 90, ..Default::default() };
        *o.volume_mut("master") = Volume { on: true, pct: 30 };
        apply(&dir, &o).unwrap();
        let s = fs::read_to_string(dir.join("options.txt")).unwrap();
        assert!(s.contains("version:3955\n") && s.contains("lang:de_de\n"));
        assert!(s.contains("guiScale:2\n") && !s.contains("guiScale:3"));
        assert!(s.contains("soundCategory_master:0.30\n") && !s.contains("1.0"));
        assert!(s.contains("fov:0.50\n"));
        assert!(!s.contains("soundCategory_music"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_category_is_independent() {
        let dir = tmp("smash_options_test2");
        let mut o = GameOptions { subtitles_on: true, subtitles: true, directional_on: true, directional: false, ..Default::default() };
        *o.volume_mut("music") = Volume { on: true, pct: 10 };
        *o.volume_mut("hostile") = Volume { on: true, pct: 80 };
        *o.volume_mut("voice") = Volume { on: false, pct: 5 };
        apply(&dir, &o).unwrap();
        let s = fs::read_to_string(dir.join("options.txt")).unwrap();
        assert!(s.contains("soundCategory_music:0.10") && s.contains("soundCategory_hostile:0.80"));
        assert!(!s.contains("soundCategory_voice") && !s.contains("soundCategory_master"));
        assert!(s.contains("showSubtitles:true") && s.contains("directionalAudio:false"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_enabled_touches_nothing() {
        let dir = tmp("smash_options_test3");
        apply(&dir, &GameOptions::default()).unwrap();
        assert!(!dir.exists());
    }
}
