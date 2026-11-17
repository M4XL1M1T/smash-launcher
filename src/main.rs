#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod auth;
mod config;
mod game;
mod mrpack;
mod net;
mod options;
mod theme;
mod update;

use anyhow::{bail, Result};
use config::{Account, Config, Kind};
use eframe::egui;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Default)]
struct Shared {
    status: String,
    progress: f32,
    busy: bool,
    log: Vec<String>,
    device: Option<(String, String)>,
    updates: Vec<Account>,
    manifest: Option<Vec<game::VersionEntry>>,
    error: Option<String>,
    refresh_installed: bool,
    login_busy: bool,
    select: Option<String>,
    new_names: Vec<(String, String)>,
    update: Option<update::Info>,
    update_msg: String,
    update_restart: bool,
}

type Sh = Arc<Mutex<Shared>>;

struct Row {
    id: String,
    kind: String,
}

struct App {
    root: PathBuf,
    cfg: Config,
    sh: Sh,
    manifest: Vec<game::VersionEntry>,
    rows: Vec<Row>,
    installed: HashSet<String>,
    selected: String,
    show_release: bool,
    show_snapshot: bool,
    show_old: bool,
    creating: bool,
    deleting: Option<String>,
    search: String,
    cancel: Arc<AtomicBool>,
    settings_open: bool,
    settings_tab: u8,
    bulk_pct: u32,
    renaming: Option<(String, String)>,
    update_dismissed: bool,
    update_prompted: bool,
    exporting: Option<(String, bool)>,
    restart_pending: bool,
}

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| ".".into())
}

/// Files that remember the chosen data folder: next to the exe (portable), with a fallback in APPDATA.
fn pointer_files() -> Vec<PathBuf> {
    let mut v = vec![exe_dir().join("smash-launcher.json")];
    if let Ok(a) = std::env::var("APPDATA") {
        v.push(PathBuf::from(a).join("SmashLauncher").join("location.json"));
    }
    v
}

fn save_pointer(dir: &std::path::Path) -> bool {
    let body = serde_json::json!({ "data_dir": dir.display().to_string() }).to_string();
    pointer_files().iter().any(|f| {
        if let Some(p) = f.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        std::fs::write(f, &body).is_ok()
    })
}

/// The configured data folder, or None on first start (then the setup screen is shown).
fn root_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("MCL_HOME") {
        return Some(p.into());
    }
    for f in pointer_files() {
        if let Some(d) = std::fs::read_to_string(&f)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["data_dir"].as_str().map(PathBuf::from))
        {
            if d.is_dir() {
                return Some(d);
            }
        }
    }
    // Existing portable installs keep working: data folder next to the exe.
    let legacy = exe_dir().join("data");
    if legacy.join("config.json").exists() {
        save_pointer(&legacy);
        return Some(legacy);
    }
    None
}

fn prepare_folder(path: &str) -> Result<PathBuf, String> {
    let path = path.trim().trim_matches('"');
    if path.is_empty() {
        return Err("Please enter a folder".into());
    }
    let dir = PathBuf::from(path);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Folder cannot be created: {e}"))?;
    let probe = dir.join(".write_test");
    std::fs::write(&probe, "ok").map_err(|e| format!("No write permission in this folder: {e}"))?;
    let _ = std::fs::remove_file(probe);
    if !save_pointer(&dir) {
        return Err("The folder choice could not be saved".into());
    }
    Ok(dir)
}

/// Removes the profile's version folder and, if wanted, its game folder (worlds, mods, settings).
/// Shared libraries and assets are never touched.
fn remove_profile_files(root: &std::path::Path, id: &str, with_data: bool) -> std::io::Result<()> {
    let p = std::path::Path::new(id);
    let simple = p.components().count() == 1 && matches!(p.components().next(), Some(std::path::Component::Normal(_)));
    if !simple {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid profile name"));
    }
    let v = root.join("versions").join(id);
    if v.exists() {
        std::fs::remove_dir_all(&v)?;
    }
    if with_data {
        let i = root.join("instances").join(id);
        if i.exists() {
            std::fs::remove_dir_all(&i)?;
        }
    }
    Ok(())
}

struct Setup {
    path: String,
    error: Option<String>,
}

impl Setup {
    fn new() -> Self {
        Self { path: exe_dir().join("data").display().to_string(), error: None }
    }

    fn show(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        let mut result = None;
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::STONE_DARK).inner_margin(28.0)).show(ctx, |ui| {
            theme::paint_dirt(ctx, ui.painter(), ui.max_rect().expand(28.0));
            ui.add_space(30.0);
            ui.heading("Welcome to Smash Launcher");
            ui.add_space(8.0);
            ui.label("Choose a folder where everything is stored: Minecraft versions, mods, worlds, Java and your sign-in data. Plan for several GB of disk space.");
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(ui.available_width() - 120.0));
                if ui.button("Browse ...").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.path = p.display().to_string();
                    }
                }
            });
            if let Some(e) = &self.error {
                ui.colored_label(theme::DANGER, e);
            }
            ui.add_space(14.0);
            if ui.add(egui::Button::new(egui::RichText::new("Weiter").size(16.0)).min_size(egui::vec2(120.0, 32.0))).clicked() {
                match prepare_folder(&self.path) {
                    Ok(p) => result = Some(p),
                    Err(e) => self.error = Some(e),
                }
            }
            ui.add_space(10.0);
            ui.weak("Your sign-in data is stored unencrypted in this folder (config.json). Do not share the folder.");
        });
        result
    }
}

enum Launcher {
    Setup(Setup),
    Main(Box<App>),
}

impl eframe::App for Launcher {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let started = if let Launcher::Setup(s) = self { s.show(ctx) } else { None };
        if let Some(root) = started {
            *self = Launcher::Main(Box::new(App::new(ctx, root)));
            ctx.request_repaint();
            return;
        }
        if let Launcher::Main(a) = self {
            eframe::App::update(&mut **a, ctx, frame);
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Launcher::Main(a) = self {
            eframe::App::on_exit(&mut **a, gl);
        }
    }
}

impl App {
    fn delete_profile(&mut self, id: String, with_data: bool) {
        if self.sh.lock().unwrap().busy {
            self.sh.lock().unwrap().error = Some("Nothing can be deleted while Minecraft is running or something is loading".into());
            return;
        }
        let r = remove_profile_files(&self.root, &id, with_data);
        if let Err(e) = r {
            self.sh.lock().unwrap().error = Some(format!("Delete failed: {e}"));
        }
        self.cfg.names.remove(&id);
        for a in &mut self.cfg.accounts {
            a.recent.retain(|v| *v != id);
        }
        if self.selected == id {
            self.selected.clear();
        }
        if self.cfg.last_version == id {
            self.cfg.last_version.clear();
        }
        self.installed = game::local_versions(&self.root);
        self.rebuild_rows();
        self.cfg.save(&self.root);
    }

    fn home_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.heading("Profiles");
            let can_rename = !self.selected.is_empty() && self.installed.contains(&self.selected);
            if ui.add_enabled(can_rename, egui::Button::new("Rename")).on_hover_text("Rename the selected profile (also via right click)").clicked() {
                let cur = config::display(&self.cfg.names, &self.selected);
                self.renaming = Some((self.selected.clone(), cur));
            }
            if ui.add_enabled(can_rename, egui::Button::new("Export")).on_hover_text("Export the selected profile as a .mrpack file to share it").clicked() {
                self.exporting = Some((self.selected.clone(), false));
            }
            if ui.add_enabled(can_rename, egui::Button::new("Delete")).on_hover_text("Delete the selected profile").clicked() {
                self.deleting = Some(self.selected.clone());
            }
        });
        ui.separator();

        // Installed profiles first: the last played one in front, then alphabetical.
        let kinds: std::collections::HashMap<&str, &str> = self.rows.iter().map(|r| (r.id.as_str(), r.kind.as_str())).collect();
        let mut cards: Vec<(String, String, String)> = self
            .installed
            .iter()
            .map(|id| {
                let sub = match kinds.get(id.as_str()).copied() {
                    Some("release") => "Vanilla",
                    Some("snapshot") => "Snapshot",
                    Some("old_beta") | Some("old_alpha") => "Old",
                    _ => "Modpack",
                };
                (id.clone(), config::display(&self.cfg.names, id), sub.to_string())
            })
            .collect();
        let last = self.cfg.last_version.clone();
        cards.sort_by_key(|(id, name, _)| (*id != last, name.to_lowercase()));

        let mut pick: Option<String> = None;
        let mut launch = false;
        let mut create = false;
        let mut rename_req: Option<String> = None;
        let mut delete_req: Option<String> = None;
        let mut export_req: Option<String> = None;
        egui::ScrollArea::vertical().id_salt("home").drag_to_scroll(false).auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                for (id, name, sub) in &cards {
                    let short: String = if name.chars().count() > 26 { name.chars().take(25).collect::<String>() + "\u{2026}" } else { name.clone() };
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(220.0, 64.0), egui::Sense::click());
                    let sel = self.selected == *id;
                    let hov = resp.hovered();
                    let fill = if sel { theme::GREEN_DARK } else if hov { theme::BTN_HOVER } else { theme::BTN };
                    let border = if sel { theme::GREEN_LIGHT } else if hov { egui::Color32::from_rgb(0xdd, 0xdd, 0xe6) } else { theme::OUTLINE };
                    let p = ui.painter();
                    p.rect_filled(rect, 0.0, fill);
                    p.rect_stroke(rect, 0.0, egui::Stroke::new(2.0_f32, border));
                    let block = match sub.as_str() {
                        "Vanilla" => theme::Block::Grass,
                        "Modpack" => theme::Block::Diamond,
                        _ => theme::Block::Stone,
                    };
                    theme::paint_block(p, egui::Rect::from_min_size(rect.left_top() + egui::vec2(12.0, 16.0), egui::vec2(32.0, 32.0)), block);
                    p.text(rect.left_top() + egui::vec2(54.0, 14.0), egui::Align2::LEFT_TOP, &short, egui::FontId::proportional(16.0), theme::TEXT);
                    p.text(rect.left_top() + egui::vec2(54.0, 38.0), egui::Align2::LEFT_TOP, sub, egui::FontId::proportional(12.0), theme::WEAK);
                    let resp = resp.on_hover_text(format!("{name}\nDouble click: start, right click: rename or delete"));
                    if resp.clicked() {
                        pick = Some(id.clone());
                    }
                    if resp.double_clicked() {
                        launch = true;
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Rename ...").clicked() {
                            rename_req = Some(id.clone());
                            ui.close_menu();
                        }
                        if ui.button("Export ...").clicked() {
                            export_req = Some(id.clone());
                            ui.close_menu();
                        }
                        if ui.button("Delete ...").clicked() {
                            delete_req = Some(id.clone());
                            ui.close_menu();
                        }
                    });
                }
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(220.0, 64.0), egui::Sense::click());
                let hov = resp.hovered();
                let p = ui.painter();
                p.rect_filled(rect, 0.0, if hov { theme::BTN_HOVER } else { theme::STONE });
                p.rect_stroke(rect, 0.0, egui::Stroke::new(2.0_f32, if hov { theme::GREEN_LIGHT } else { theme::BTN }));
                p.text(rect.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(34.0), if hov { theme::GREEN_LIGHT } else { theme::WEAK });
                let plus = resp.on_hover_text("Create a new profile or import a Modrinth modpack");
                if plus.clicked() {
                    create = true;
                }
            });
            if cards.is_empty() {
                ui.add_space(8.0);
                ui.weak("No profile yet. Use + to create one or import a Modrinth modpack.");
            }
        });
        if let Some(p) = pick {
            self.selected = p;
        }
        if launch {
            self.start_launch(ctx);
        }
        if let Some(id) = rename_req {
            let cur = config::display(&self.cfg.names, &id);
            self.renaming = Some((id, cur));
        }
        if let Some(id) = delete_req {
            self.deleting = Some(id);
        }
        if let Some(id) = export_req {
            self.exporting = Some((id, false));
        }
        if create {
            self.creating = true;
        }
    }

    fn create_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        ui.horizontal(|ui| {
            ui.heading("New profile");
            if ui.button("Back").clicked() {
                self.creating = false;
            }
        });
        ui.separator();
        ui.strong("Modrinth-Modpack");
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new("Import modpack (.mrpack)")).clicked() {
                self.start_import(ctx, None);
            }
            ui.weak("or drag the .mrpack file into the window");
        });
        ui.add_space(10.0);
        ui.strong("Minecraft version (vanilla)");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.show_release, "Releases");
            ui.checkbox(&mut self.show_snapshot, "Snapshots");
            ui.checkbox(&mut self.show_old, "Beta/Alpha");
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search").desired_width(120.0));
        });
        if !self.selected.is_empty() && self.installed.contains(&self.selected) {
            ui.weak("This version already exists as a profile.");
        } else {
            ui.weak("Select a version, then press Install at the bottom.");
        }
        ui.separator();
        let q = self.search.to_lowercase();
        let list: Vec<&Row> = self
            .rows
            .iter()
            .filter(|r| match r.kind.as_str() {
                "release" => self.show_release,
                "snapshot" => self.show_snapshot,
                "custom" => false,
                _ => self.show_old,
            })
            .filter(|r| q.is_empty() || r.id.to_lowercase().contains(&q))
            .collect();
        let h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let mut pick: Option<String> = None;
        egui::ScrollArea::vertical().id_salt("versions").drag_to_scroll(false).auto_shrink([false, false]).show_rows(ui, h, list.len(), |ui, range| {
            for r in &list[range] {
                let inst = if self.installed.contains(&r.id) { "  [installed]" } else { "" };
                let kind = match r.kind.as_str() {
                    "release" => "",
                    "snapshot" => "  Snapshot",
                    _ => "  Old",
                };
                if ui.selectable_label(self.selected == r.id, format!("{}{kind}{inst}", r.id)).clicked() {
                    pick = Some(r.id.clone());
                }
            }
        });
        if let Some(p) = pick {
            self.selected = p;
        }
    }

    fn start_install_only(&mut self, ctx: &egui::Context) {
        if self.selected.is_empty() {
            return;
        }
        {
            let mut s = self.sh.lock().unwrap();
            if s.busy {
                return;
            }
            s.busy = true;
            s.log.clear();
            s.progress = 0.0;
        }
        let (sh, ctx, root, id, manifest) = (self.sh.clone(), ctx.clone(), self.root.clone(), self.selected.clone(), self.manifest.clone());
        std::thread::spawn(move || {
            let status = |t: &str, f: f32| {
                let mut s = sh.lock().unwrap();
                s.status = t.to_string();
                s.progress = f;
                ctx.request_repaint();
            };
            let r = game::ensure_installed(&root, &id, &manifest, &status).map(|_| ());
            let mut s = sh.lock().unwrap();
            s.busy = false;
            s.progress = 0.0;
            match r {
                Ok(()) => {
                    s.status = format!("Profile added: {id}");
                    s.select = Some(id);
                    s.refresh_installed = true;
                }
                Err(e) => {
                    s.status.clear();
                    s.error = Some(e.to_string());
                }
            }
            ctx.request_repaint();
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut check_now = false;
        ui.horizontal(|ui| {
            ui.heading("Settings");
            if ui.button("Back to versions").clicked() {
                self.settings_open = false;
            }
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.settings_tab, 0, "Audio");
            ui.selectable_value(&mut self.settings_tab, 1, "Graphics and display");
            ui.selectable_value(&mut self.settings_tab, 2, "Launcher");
        });
        ui.separator();
        let tab = self.settings_tab;
        egui::ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| {
            if tab != 2 {
                ui.weak("These values are written into the profile\'s options.txt before every start. Only values with a checkmark are overwritten, everything else stays as set in the game.");
                ui.add_space(8.0);
            }
            match tab {
                0 => {
                    let g = &mut self.cfg.game;
                    ui.strong("Volume");
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Set all sliders (except master) to");
                        ui.add(egui::Slider::new(&mut self.bulk_pct, 0..=100).suffix(" %"));
                        if ui.button("Set and enable").clicked() {
                            for (id, _) in options::SOUNDS.iter().filter(|(id, _)| *id != "master") {
                                *g.volume_mut(id) = options::Volume { on: true, pct: self.bulk_pct };
                            }
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Enable all").clicked() {
                            for (id, _) in options::SOUNDS {
                                g.volume_mut(id).on = true;
                            }
                            changed = true;
                        }
                        if ui.button("Disable all").clicked() {
                            for (id, _) in options::SOUNDS {
                                g.volume_mut(id).on = false;
                            }
                            changed = true;
                        }
                    });
                    ui.add_space(8.0);
                    egui::Grid::new("sound_vol").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                        for (id, label) in options::SOUNDS {
                            let v = g.volume_mut(id);
                            changed |= ui.checkbox(&mut v.on, label).changed();
                            changed |= ui.add_enabled(v.on, egui::Slider::new(&mut v.pct, 0..=100).suffix(" %")).changed();
                            ui.end_row();
                        }
                    });
                    ui.add_space(6.0);
                    ui.weak("Note: the master volume is multiplied with each individual slider. Master 30 % and music 30 % result in about 9 % in game.");
                    ui.add_space(14.0);
                    ui.strong("More audio options");
                    ui.add_space(4.0);
                    egui::Grid::new("sound_misc").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                        changed |= ui.checkbox(&mut g.subtitles_on, "Subtitles").changed();
                        changed |= ui.add_enabled(g.subtitles_on, egui::Checkbox::new(&mut g.subtitles, "show")).changed();
                        ui.end_row();
                        changed |= ui.checkbox(&mut g.directional_on, "Directional audio").changed();
                        changed |= ui.add_enabled(g.directional_on, egui::Checkbox::new(&mut g.directional, "turn on")).changed();
                        ui.end_row();
                    });
                }
                1 => {
                    let g = &mut self.cfg.game;
                    ui.strong("Graphics and display");
                    ui.add_space(4.0);
                    egui::Grid::new("game_opts").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                        changed |= ui.checkbox(&mut g.gui_on, "GUI scale").changed();
                        ui.add_enabled_ui(g.gui_on, |ui| {
                            let label = if g.gui_scale == 0 { "Auto".to_string() } else { format!("{}x", g.gui_scale) };
                            egui::ComboBox::from_id_salt("gui_scale").selected_text(label).show_ui(ui, |ui| {
                                for (v, l) in [(0, "Auto"), (1, "1x"), (2, "2x"), (3, "3x"), (4, "4x")] {
                                    if ui.selectable_value(&mut g.gui_scale, v, l).clicked() {
                                        changed = true;
                                    }
                                }
                            });
                        });
                        ui.end_row();
                        changed |= ui.checkbox(&mut g.fov_on, "Field of view (FOV)").changed();
                        changed |= ui.add_enabled(g.fov_on, egui::Slider::new(&mut g.fov, 30..=110)).changed();
                        ui.end_row();
                        changed |= ui.checkbox(&mut g.fps_on, "Max FPS").changed();
                        changed |= ui.add_enabled(g.fps_on, egui::Slider::new(&mut g.max_fps, 10..=260)).changed();
                        ui.end_row();
                        changed |= ui.checkbox(&mut g.dist_on, "Render distance (chunks)").changed();
                        changed |= ui.add_enabled(g.dist_on, egui::Slider::new(&mut g.render_distance, 2..=32)).changed();
                        ui.end_row();
                        changed |= ui.checkbox(&mut g.fullscreen_on, "Fullscreen").changed();
                        changed |= ui.add_enabled(g.fullscreen_on, egui::Checkbox::new(&mut g.fullscreen, "on start")).changed();
                        ui.end_row();
                    });
                }
                _ => {
                    ui.strong("Launcher");
                    ui.add_space(6.0);
                    ui.label("Memory for Minecraft");
                    changed |= ui.add(egui::Slider::new(&mut self.cfg.ram_mb, 1024..=16384).step_by(256.0).suffix(" MB")).changed();
                    ui.add_space(6.0);
                    ui.label("Java path (empty = download the matching version automatically)");
                    if ui.text_edit_singleline(&mut self.cfg.java_path).lost_focus() {
                        changed = true;
                    }
                    ui.add_space(6.0);
                    ui.label("Azure client ID (empty = built-in ID)");
                    if ui.text_edit_singleline(&mut self.cfg.ms_client_id).lost_focus() {
                        changed = true;
                    }
                    ui.add_space(10.0);
                    ui.strong("Updates");
                    changed |= ui.checkbox(&mut self.cfg.check_updates, "Check for new versions on start (GitHub)").changed();
                    let msg = self.sh.lock().unwrap().update_msg.clone();
                    ui.horizontal(|ui| {
                        ui.label(format!("Installed: {}", update::CURRENT));
                        if ui.button("Check for updates now").clicked() {
                            check_now = true;
                        }
                        ui.label(msg);
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(format!("Data folder: {}", self.root.display()));
                        if ui.button("Open").clicked() {
                            let _ = std::process::Command::new("explorer").arg(&self.root).spawn();
                        }
                    });
                }
            }
        });
        if check_now {
            self.start_update_check(ui.ctx(), true);
        }
        if changed {
            self.cfg.save(&self.root);
        }
    }

    fn new(ctx: &egui::Context, root: PathBuf) -> Self {
        let cfg = Config::load(&root);
        let sh: Sh = Arc::default();
        {
            let (sh, root, ctx) = (sh.clone(), root.clone(), ctx.clone());
            std::thread::spawn(move || {
                match game::fetch_manifest(&root) {
                    Ok(m) => sh.lock().unwrap().manifest = Some(m),
                    Err(e) => sh.lock().unwrap().error = Some(format!("Version list could not be loaded: {e}")),
                }
                ctx.request_repaint();
            });
        }
        let mut app = Self {
            installed: game::local_versions(&root),
            selected: String::new(),
            root,
            cfg,
            sh,
            manifest: vec![],
            rows: vec![],
            show_release: true,
            show_snapshot: false,
            show_old: false,
            creating: false,
            deleting: None,
            search: String::new(),
            cancel: Arc::default(),
            settings_open: false,
            settings_tab: 0,
            bulk_pct: 30,
            renaming: None,
            update_dismissed: false,
            update_prompted: false,
            exporting: None,
            restart_pending: false,
        };
        if app.cfg.check_updates {
            app.start_update_check(ctx, false);
        }
        app.rebuild_rows();
        app
    }

    fn rebuild_rows(&mut self) {
        let mut rows: Vec<Row> = self
            .manifest
            .iter()
            .map(|e| Row { id: e.id.clone(), kind: e.kind.clone() })
            .collect();
        let known: HashSet<&str> = self.manifest.iter().map(|e| e.id.as_str()).collect();
        let mut custom: Vec<Row> = self
            .installed
            .iter()
            .filter(|i| !known.contains(i.as_str()))
            .map(|i| Row { id: i.clone(), kind: "custom".into() })
            .collect();
        custom.sort_by(|a, b| a.id.cmp(&b.id));
        custom.append(&mut rows);
        self.rows = custom;
    }

    fn sync(&mut self) {
        let mut sh = self.sh.lock().unwrap();
        let mut changed = false;
        for a in sh.updates.drain(..) {
            self.cfg.upsert(a);
            changed = true;
        }
        if let Some(m) = sh.manifest.take() {
            self.manifest = m;
            changed = true;
        }
        let refresh = std::mem::take(&mut sh.refresh_installed);
        let select = sh.select.take();
        if std::mem::take(&mut sh.update_restart) {
            self.restart_pending = true;
        }
        for (id, name) in sh.new_names.drain(..) {
            self.cfg.names.entry(id).or_insert(name);
            changed = true;
        }
        drop(sh);
        if let Some(id) = select {
            self.selected = id;
            self.search.clear();
            self.creating = false;
        }
        if refresh {
            self.installed = game::local_versions(&self.root);
        }
        if changed || refresh {
            self.rebuild_rows();
            self.cfg.save(&self.root);
        }
    }

    fn start_update_check(&mut self, ctx: &egui::Context, manual: bool) {
        if manual {
            self.update_dismissed = false;
            self.sh.lock().unwrap().update_msg = "Checking ...".into();
        }
        let (sh, ctx) = (self.sh.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = update::check();
            {
                let mut s = sh.lock().unwrap();
                match r {
                    Ok(Some(info)) => {
                        s.update_msg = format!("Version {} is available", info.version);
                        s.update = Some(info);
                    }
                    Ok(None) => s.update_msg = if manual { "You have the latest version.".into() } else { String::new() },
                    Err(e) => s.update_msg = if manual { format!("Update check failed: {e}") } else { String::new() },
                }
            }
            ctx.request_repaint();
        });
    }

    fn start_update_install(&mut self, ctx: &egui::Context, info: update::Info) {
        {
            let mut s = self.sh.lock().unwrap();
            if s.busy {
                return;
            }
            s.busy = true;
            s.progress = 0.0;
            s.error = None;
        }
        let (sh, ctx) = (self.sh.clone(), ctx.clone());
        std::thread::spawn(move || {
            let step = |text: &str, p: f32| {
                let mut s = sh.lock().unwrap();
                s.status = text.to_string();
                s.progress = p;
                drop(s);
                ctx.request_repaint();
            };
            let r = update::install(&info, &step);
            let mut s = sh.lock().unwrap();
            s.busy = false;
            s.progress = 0.0;
            match r {
                Ok(()) => {
                    s.status = "Update installed, restarting ...".into();
                    s.update = None;
                    s.update_restart = true;
                }
                Err(e) => {
                    s.status.clear();
                    s.error = Some(format!("Update failed: {e}"));
                }
            }
            drop(s);
            ctx.request_repaint();
        });
    }

    fn selected_account(&self) -> Option<&Account> {
        self.cfg.accounts.iter().find(|a| a.key() == self.cfg.selected)
    }

    fn start_microsoft_login(&mut self, ctx: &egui::Context) {
        let cid = self.cfg.client_id();
        if cid.is_empty() {
            self.sh.lock().unwrap().error =
                Some("No client ID configured. Please enter an Azure client ID in the settings (see README).".into());
            self.settings_open = true;
            return;
        }
        self.cancel = Arc::default();
        self.sh.lock().unwrap().login_busy = true;
        let (sh, cancel, ctx) = (self.sh.clone(), self.cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = (|| -> Result<Account> {
                let dc = auth::device_code(&cid)?;
                sh.lock().unwrap().device = Some((dc.user_code.clone(), dc.verification_uri.clone()));
                ctx.request_repaint();
                let (access, refresh) = auth::poll_token(&cid, &dc, &cancel)?;
                let (_, name, uuid) = auth::minecraft_login(&access)?;
                Ok(Account { kind: Kind::Microsoft, name, uuid, refresh_token: Some(refresh), recent: vec![] })
            })();
            let mut s = sh.lock().unwrap();
            s.device = None;
            s.login_busy = false;
            match r {
                Ok(a) => s.updates.push(a),
                Err(e) => s.error = Some(e.to_string()),
            }
            ctx.request_repaint();
        });
    }

    fn start_export(&mut self, ctx: &egui::Context, id: String, include_worlds: bool) {
        {
            let mut s = self.sh.lock().unwrap();
            if s.busy {
                return;
            }
            s.busy = true;
            s.progress = 0.0;
            s.error = None;
        }
        let name = config::display(&self.cfg.names, &id);
        let (sh, ctx, root) = (self.sh.clone(), ctx.clone(), self.root.clone());
        std::thread::spawn(move || {
            let status = |t: &str, f: f32| {
                let mut s = sh.lock().unwrap();
                s.status = t.to_string();
                s.progress = f;
                ctx.request_repaint();
            };
            let r = (|| -> Result<Option<PathBuf>> {
                let Some(dest) = rfd::FileDialog::new().add_filter("Modrinth modpack", &["mrpack"]).set_file_name(format!("{name}.mrpack")).save_file() else {
                    return Ok(None);
                };
                mrpack::export(&root, &id, &name, include_worlds, &dest, &status)?;
                Ok(Some(dest))
            })();
            let mut s = sh.lock().unwrap();
            s.busy = false;
            s.progress = 0.0;
            match r {
                Ok(Some(dest)) => s.status = format!("Exported to {}", dest.display()),
                Ok(None) => s.status.clear(),
                Err(e) => {
                    s.status.clear();
                    s.error = Some(format!("Export failed: {e}"));
                }
            }
            ctx.request_repaint();
        });
    }

    fn start_import(&mut self, ctx: &egui::Context, file: Option<PathBuf>) {
        {
            let mut s = self.sh.lock().unwrap();
            if s.busy {
                return;
            }
            s.busy = true;
            s.log.clear();
            s.progress = 0.0;
        }
        let (sh, ctx, root) = (self.sh.clone(), ctx.clone(), self.root.clone());
        std::thread::spawn(move || {
            let status = |t: &str, f: f32| {
                let mut s = sh.lock().unwrap();
                s.status = t.to_string();
                s.progress = f;
                ctx.request_repaint();
            };
            let r = (|| -> Result<Option<(String, String)>> {
                let file = match file {
                    Some(f) => f,
                    None => match rfd::FileDialog::new().add_filter("Modrinth modpack", &["mrpack"]).pick_file() {
                        Some(f) => f,
                        None => return Ok(None),
                    },
                };
                Ok(Some(mrpack::import(&root, &file, &status)?))
            })();
            let mut s = sh.lock().unwrap();
            s.busy = false;
            s.progress = 0.0;
            match r {
                Ok(Some((id, name))) => {
                    s.status = format!("Modpack imported: {name}");
                    s.new_names.push((id.clone(), name));
                    s.select = Some(id);
                    s.refresh_installed = true;
                }
                Ok(None) => {}
                Err(e) => {
                    s.status.clear();
                    s.error = Some(e.to_string());
                }
            }
            ctx.request_repaint();
        });
    }

    fn start_launch(&mut self, ctx: &egui::Context) {
        if self.sh.lock().unwrap().busy {
            return;
        }
        let Some(acc) = self.selected_account().cloned() else {
            self.sh.lock().unwrap().error = Some("Please add and select an account first".into());
            return;
        };
        if self.selected.is_empty() {
            self.sh.lock().unwrap().error = Some("Please select a version".into());
            return;
        }
        self.cfg.last_version = self.selected.clone();
        let ver = self.selected.clone();
        self.cfg.remember(&acc.key(), &ver);
        self.cfg.save(&self.root);
        {
            let mut s = self.sh.lock().unwrap();
            s.busy = true;
            s.log.clear();
            s.progress = 0.0;
        }
        let (sh, ctx, root, id, manifest) = (self.sh.clone(), ctx.clone(), self.root.clone(), self.selected.clone(), self.manifest.clone());
        let (ram, java_path, cid) = (self.cfg.ram_mb, self.cfg.java_path.clone(), self.cfg.client_id());
        let game_opts = self.cfg.game.clone();
        // Nothing stays selected after a start.
        self.selected.clear();
        std::thread::spawn(move || {
            let status = |t: &str, f: f32| {
                let mut s = sh.lock().unwrap();
                s.status = t.to_string();
                s.progress = f;
                ctx.request_repaint();
            };
            let log = |l: String| {
                let mut s = sh.lock().unwrap();
                s.log.push(l);
                if s.log.len() > 500 {
                    s.log.remove(0);
                }
                ctx.request_repaint();
            };
            let r = (|| -> Result<i32> {
                // Online only: ownership of the game is verified by the profile request on every launch.
                status("Checking sign-in", 0.0);
                let Some(rt) = acc.refresh_token.clone() else { bail!("No refresh token, please sign in to the account again") };
                let (access, new_rt) = auth::refresh(&cid, &rt)?;
                let (token, name, uuid) = auth::minecraft_login(&access)?;
                sh.lock().unwrap().updates.push(Account { kind: Kind::Microsoft, name: name.clone(), uuid: uuid.clone(), refresh_token: Some(new_rt), recent: vec![] });
                let session = auth::Session { name, uuid, token, user_type: "msa" };
                let v = game::ensure_installed(&root, &id, &manifest, &status)?;
                sh.lock().unwrap().refresh_installed = true;
                let java = game::ensure_java(&root, &v, &java_path, &status)?;
                let _ = options::apply(&root.join("instances").join(&id), &game_opts);
                status("Minecraft is running", 1.0);
                game::run_game(&root, &v, &session, &java, ram, &log)
            })();
            let mut s = sh.lock().unwrap();
            s.busy = false;
            s.progress = 0.0;
            match r {
                Ok(code) => s.status = format!("Minecraft exited (exit code {code})"),
                Err(e) => {
                    s.status.clear();
                    s.error = Some(e.to_string());
                }
            }
            ctx.request_repaint();
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.sync();
        let (inner, outer, maximized, minimized) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().outer_rect, i.viewport().maximized, i.viewport().minimized));
        if maximized != Some(true) && minimized != Some(true) {
            if let Some(r) = inner.filter(|r| r.width() >= 700.0 && r.height() >= 450.0) {
                self.cfg.window_size = [r.width().round(), r.height().round()];
            }
            if let Some(r) = outer.filter(|r| r.min.x > -20000.0 && r.min.y > -20000.0) {
                self.cfg.window_pos = Some([r.min.x.round(), r.min.y.round()]);
            }
        }
        let dropped = ctx.input(|i| {
            i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mrpack")))
        });
        if let Some(p) = dropped {
            self.start_import(ctx, Some(p));
        }
        let (status, progress, busy, device, error, login_busy) = {
            let s = self.sh.lock().unwrap();
            (s.status.clone(), s.progress, s.busy, s.device.clone(), s.error.clone(), s.login_busy)
        };

        if self.restart_pending {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let pending_update = if self.update_dismissed || self.restart_pending { None } else { self.sh.lock().unwrap().update.clone() };
        if let Some(info) = pending_update {
            egui::TopBottomPanel::top("update").frame(egui::Frame::none().fill(theme::GREEN_DARK).inner_margin(egui::Margin::symmetric(14.0, 6.0))).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("New version {} available (installed: {})", info.version, update::CURRENT)).strong().color(egui::Color32::WHITE));
                    if ui.add_enabled(!busy, egui::Button::new("Update now")).on_hover_text("Downloads the new version from GitHub, verifies the checksum and restarts the launcher").clicked() {
                        self.start_update_install(ctx, info.clone());
                    }
                    if !info.page.is_empty() {
                        ui.hyperlink_to("What\'s new?", &info.page);
                    }
                    if ui.small_button("Later").clicked() {
                        self.update_dismissed = true;
                    }
                });
            });
        }

        let prompt_info = self.sh.lock().unwrap().update.clone().filter(|_| !self.update_prompted && !self.restart_pending);
        if let Some(info) = prompt_info {
            let mut start = false;
            let mut later = false;
            egui::Window::new("Update available")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new(format!("Smash Launcher {} is available.", info.version)).strong());
                    ui.label(format!("You are using version {}.", update::CURRENT));
                    ui.add_space(4.0);
                    ui.weak("The update is downloaded from GitHub, checked against its SHA-256 checksum and installed. The launcher restarts afterwards.");
                    if !info.page.is_empty() {
                        ui.hyperlink_to("What's new?", &info.page);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!busy, egui::Button::new("Update now").fill(theme::GREEN)).clicked() {
                            start = true;
                        }
                        if ui.button("Later").clicked() {
                            later = true;
                        }
                    });
                });
            if start {
                self.update_prompted = true;
                self.start_update_install(ctx, info);
            } else if later {
                self.update_prompted = true;
            }
        }

        egui::SidePanel::left("accounts").resizable(false).exact_width(260.0).frame(egui::Frame::none().inner_margin(14.0)).show(ctx, |ui| {
            theme::paint_dirt(ctx, ui.painter(), ui.max_rect().expand(14.0));
            // Header: filled edge to edge behind the title, closed off by a separator line.
            let panel = ui.max_rect().expand(14.0);
            let bg = ui.painter().add(egui::Shape::Noop);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("SMASH").monospace().size(30.0).strong().color(theme::GREEN_LIGHT));
                ui.label(egui::RichText::new("LAUNCHER").monospace().size(14.0).color(theme::TEXT));
            });
            ui.add_space(8.0);
            let bottom = ui.cursor().min.y;
            let header = egui::Rect::from_min_max(panel.min, egui::pos2(panel.max.x, bottom));
            ui.painter().set(bg, egui::Shape::rect_filled(header, 0.0, egui::Color32::from_black_alpha(110)));
            ui.painter().line_segment([header.left_bottom(), header.right_bottom()], egui::Stroke::new(2.0_f32, theme::OUTLINE));
            ui.add_space(14.0);
            let mut add_account = false;
            ui.horizontal(|ui| {
                let head = ui.heading("Accounts");
                let size = egui::vec2(ui.available_width(), head.rect.height());
                ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !login_busy && device.is_none() {
                        let plus = ui
                            .add(egui::Button::new(egui::RichText::new("+").size(15.0).strong().color(egui::Color32::WHITE)).fill(theme::GREEN).stroke(egui::Stroke::new(2.0_f32, theme::GREEN_DARK)).min_size(egui::vec2(24.0, 24.0)))
                            .on_hover_text("Add account (Microsoft sign-in, Minecraft: Java Edition must be owned)");
                        add_account = plus.clicked();
                    }
                });
            });
            ui.separator();
            let mut remove: Option<usize> = None;
            let mut pick_recent: Option<(String, String)> = None;
            let mut launch_recent = false;
            for (i, a) in self.cfg.accounts.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.selectable_label(a.key() == self.cfg.selected, &a.name).clicked() {
                        self.cfg.selected = a.key();
                    }
                    if ui.small_button("x").on_hover_text("Remove account").clicked() {
                        remove = Some(i);
                    }
                });
                // The recent profiles hang off the account with a green tree line, so it is clear whose they are.
                let top = ui.cursor().min;
                let mut mids: Vec<f32> = vec![];
                for v in &a.recent {
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        let active = a.key() == self.cfg.selected && self.selected == *v;
                        let text = egui::RichText::new(config::display(&self.cfg.names, v)).size(11.0).color(egui::Color32::WHITE);
                        let (fill, stroke) = if active {
                            (theme::GREEN_DARK, egui::Stroke::new(1.0_f32, theme::GREEN_LIGHT))
                        } else {
                            (egui::Color32::from_white_alpha(10), egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(28)))
                        };
                        let resp = ui.add(egui::Button::new(text).fill(fill).stroke(stroke)).on_hover_text("Click: select, double click: start");
                        mids.push(resp.rect.center().y);
                        if resp.clicked() || resp.double_clicked() {
                            pick_recent = Some((a.key(), v.clone()));
                        }
                        if resp.double_clicked() {
                            launch_recent = true;
                        }
                    });
                }
                if let Some(last) = mids.last().copied() {
                    let x = top.x + 8.0;
                    let stroke = egui::Stroke::new(2.0_f32, theme::GREEN);
                    let p = ui.painter();
                    p.line_segment([egui::pos2(x, top.y - 4.0), egui::pos2(x, last)], stroke);
                    for y in mids {
                        p.line_segment([egui::pos2(x, y), egui::pos2(x + 9.0, y)], stroke);
                    }
                }
            }
            if let Some((k, v)) = pick_recent {
                self.cfg.selected = k;
                self.selected = v;
                if launch_recent {
                    self.start_launch(ctx);
                }
            }
            if let Some(i) = remove {
                let a = self.cfg.accounts.remove(i);
                if a.key() == self.cfg.selected {
                    self.cfg.selected = self.cfg.accounts.first().map(|a| a.key()).unwrap_or_default();
                }
                self.cfg.save(&self.root);
            }
            if let Some((code, uri)) = &device {
                ui.separator();
                ui.label("1. Open this page:");
                ui.hyperlink(uri);
                ui.label("2. Enter this code:");
                ui.label(egui::RichText::new(code).monospace().size(22.0).strong());
                ui.horizontal(|ui| {
                    if ui.button("Copy code").clicked() {
                        ctx.copy_text(code.clone());
                    }
                    if ui.button("Cancel").clicked() {
                        self.cancel.store(true, Ordering::SeqCst);
                    }
                });
                ui.weak("Waiting for confirmation ...");
            } else if login_busy {
                ui.weak("Preparing sign-in ...");
            } else {
                if self.cfg.accounts.is_empty() {
                    ui.weak("Add an account with +");
                }
            }
            if add_account {
                self.start_microsoft_login(ctx);
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                let (fill, stroke) = if self.settings_open {
                    (theme::GREEN_DARK, egui::Stroke::new(1.0_f32, theme::GREEN_LIGHT))
                } else {
                    (egui::Color32::from_white_alpha(10), egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(28)))
                };
                if ui.add_sized([ui.available_width(), 30.0], egui::Button::new("Settings").fill(fill).stroke(stroke)).clicked() {
                    self.settings_open = !self.settings_open;
                }
                ui.separator();
            });
        });

        egui::TopBottomPanel::bottom("launch").frame(egui::Frame::none().fill(theme::STONE).inner_margin(egui::Margin::symmetric(14.0, 8.0))).show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let installed = self.installed.contains(&self.selected);
                let label = if self.selected.is_empty() {
                    "Select a profile".to_string()
                } else {
                    let sel = config::display(&self.cfg.names, &self.selected);
                    if installed { format!("Play: {sel}") } else { format!("Install: {sel}") }
                };
                if ui.add_enabled(!busy && !self.selected.is_empty(), egui::Button::new(egui::RichText::new(label).size(16.0).strong().color(egui::Color32::WHITE)).fill(theme::GREEN).stroke(egui::Stroke::new(2.0_f32, theme::GREEN_DARK)).min_size(egui::vec2(240.0, 36.0))).clicked() {
                    if installed {
                        self.start_launch(ctx);
                    } else {
                        self.start_install_only(ctx);
                    }
                }
            });
            if busy || progress > 0.0 {
                ui.add(egui::ProgressBar::new(progress).text(status.clone()));
            } else if !status.is_empty() {
                ui.label(&status);
            }
            if let Some(e) = &error {
                ui.horizontal(|ui| {
                    ui.colored_label(theme::DANGER, e);
                    if ui.small_button("OK").clicked() {
                        self.sh.lock().unwrap().error = None;
                    }
                });
            }
            ui.add_space(4.0);
        });

        if !self.settings_open {
            egui::TopBottomPanel::bottom("log").resizable(false).exact_height(120.0).frame(egui::Frame::none().fill(theme::STONE_DARK).inner_margin(egui::Margin::symmetric(14.0, 8.0))).show(ctx, |ui| {
                ui.label("Game log");
                egui::ScrollArea::vertical().id_salt("log").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                    let lines = self.sh.lock().unwrap().log.clone();
                    for l in lines {
                        ui.label(egui::RichText::new(l).monospace().size(11.0));
                    }
                });
            });
        }

        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::STONE_DARK).inner_margin(18.0)).show(ctx, |ui| {
            if self.settings_open {
                self.settings_ui(ui);
            } else if self.creating {
                self.create_ui(ui, ctx, busy);
            } else {
                self.home_ui(ui, ctx);
            }
        });

        if let Some(id) = self.deleting.clone() {
            let name = config::display(&self.cfg.names, &id);
            let (mut close, mut action) = (false, None::<bool>);
            egui::Window::new("Delete profile")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Really delete profile \"{name}\"?"));
                    ui.add_space(6.0);
                    ui.weak("Remove profile only: the game folder with worlds, mods and settings is kept.");
                    ui.colored_label(theme::DANGER, "Delete everything: worlds, mods and settings are gone permanently.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Remove profile only").clicked() {
                            action = Some(false);
                        }
                        if ui.button("Delete everything").clicked() {
                            action = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            if let Some(with_data) = action {
                self.deleting = None;
                self.delete_profile(id, with_data);
            } else if close {
                self.deleting = None;
            }
        }

        if let Some((id, mut worlds)) = self.exporting.take() {
            let name = config::display(&self.cfg.names, &id);
            let (mut open, mut go, mut cancel) = (true, false, false);
            egui::Window::new("Export profile")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(format!("Export \"{name}\" as a .mrpack file."));
                    ui.weak("It contains the Minecraft version, the Fabric or Quilt loader, mods, configs, resource packs and options. Friends import it with the + card (or any launcher that supports .mrpack).");
                    ui.add_space(6.0);
                    ui.checkbox(&mut worlds, "Include worlds (can be large)");
                    ui.colored_label(theme::DANGER, "Config files of some mods can contain private data such as API keys. Only share the file with people you trust.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!busy, egui::Button::new("Export ...").fill(theme::GREEN)).clicked() {
                            go = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if go {
                self.start_export(ctx, id, worlds);
            } else if open && !cancel {
                self.exporting = Some((id, worlds));
            }
        }

        if let Some((id, mut text)) = self.renaming.take() {
            let (mut open, mut apply, mut cancel) = (true, false, false);
            egui::Window::new("Rename profile")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.weak(format!("Internal ID: {id}"));
                    let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(260.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        apply = true;
                    }
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            apply = true;
                        }
                        if ui.button("Reset").on_hover_text("Restore the original name").clicked() {
                            text.clear();
                            apply = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if apply {
                let t = text.trim().to_string();
                if t.is_empty() {
                    self.cfg.names.remove(&id);
                } else {
                    self.cfg.names.insert(id, t);
                }
                self.cfg.save(&self.root);
            } else if open && !cancel {
                self.renaming = Some((id, text));
            }
        }
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.cfg.last_version = self.selected.clone();
        self.cfg.save(&self.root);
        if self.restart_pending {
            update::restart();
        }
    }
}

fn main() -> eframe::Result {
    update::cleanup();
    let saved = root_dir().map(|r| Config::load(&r));
    let size = saved.as_ref().map(|c| c.window_size).filter(|s| s[0] >= 760.0 && s[1] >= 520.0 && s[0] <= 8000.0 && s[1] <= 8000.0).unwrap_or([940.0, 640.0]);
    let pos = saved.as_ref().and_then(|c| c.window_pos).filter(|p| p[0] > -4000.0 && p[0] < 8000.0 && p[1] > -100.0 && p[1] < 5000.0);
    let mut viewport = egui::ViewportBuilder::default().with_inner_size(size);
    if let Some(p) = pos {
        viewport = viewport.with_position(egui::pos2(p[0], p[1]));
    }
    let opts = eframe::NativeOptions {
        viewport: viewport.with_min_inner_size([760.0, 520.0]).with_title("Smash Launcher").with_icon(theme::app_icon()),
        ..Default::default()
    };
    eframe::run_native("Smash Launcher", opts, Box::new(|cc| {
        theme::apply(&cc.egui_ctx);
        let l = match root_dir() {
            Some(root) => Launcher::Main(Box::new(App::new(&cc.egui_ctx, root))),
            None => Launcher::Setup(Setup::new()),
        };
        Ok(Box::new(l))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_keeps_or_removes_game_folder_and_rejects_bad_ids() {
        let root = std::env::temp_dir().join("smash_delete_test");
        let _ = std::fs::remove_dir_all(&root);
        for id in ["A", "B"] {
            std::fs::create_dir_all(root.join("versions").join(id)).unwrap();
            std::fs::create_dir_all(root.join("instances").join(id).join("saves")).unwrap();
        }
        std::fs::create_dir_all(root.join("libraries")).unwrap();
        remove_profile_files(&root, "A", false).unwrap();
        assert!(!root.join("versions/A").exists() && root.join("instances/A/saves").exists());
        remove_profile_files(&root, "B", true).unwrap();
        assert!(!root.join("versions/B").exists() && !root.join("instances/B").exists());
        assert!(root.join("libraries").exists());
        assert!(remove_profile_files(&root, "..", true).is_err());
        assert!(remove_profile_files(&root, "a/b", true).is_err());
        assert!(remove_profile_files(&root, "", true).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
