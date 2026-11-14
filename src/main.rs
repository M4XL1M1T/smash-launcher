#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod auth;
mod config;
mod game;
mod net;

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
    only_installed: bool,
    search: String,
    cancel: Arc<AtomicBool>,
    show_settings: bool,
}

fn root_dir() -> PathBuf {
    if let Ok(p) = std::env::var("MCL_HOME") {
        return p.into();
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("data")))
        .unwrap_or_else(|| "data".into())
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let root = root_dir();
        let cfg = Config::load(&root);
        let sh: Sh = Arc::default();
        {
            let (sh, root, ctx) = (sh.clone(), root.clone(), ctx.clone());
            std::thread::spawn(move || {
                match game::fetch_manifest(&root) {
                    Ok(m) => sh.lock().unwrap().manifest = Some(m),
                    Err(e) => sh.lock().unwrap().error = Some(format!("Versionsliste konnte nicht geladen werden: {e}")),
                }
                ctx.request_repaint();
            });
        }
        let mut app = Self {
            installed: game::local_versions(&root),
            selected: cfg.last_version.clone(),
            root,
            cfg,
            sh,
            manifest: vec![],
            rows: vec![],
            show_release: true,
            show_snapshot: false,
            show_old: false,
            only_installed: false,
            search: String::new(),
            cancel: Arc::default(),
            show_settings: false,
        };
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
        drop(sh);
        if refresh {
            self.installed = game::local_versions(&self.root);
        }
        if changed || refresh {
            self.rebuild_rows();
            self.cfg.save(&self.root);
        }
    }

    fn selected_account(&self) -> Option<&Account> {
        self.cfg.accounts.iter().find(|a| a.key() == self.cfg.selected)
    }

    fn start_microsoft_login(&mut self, ctx: &egui::Context) {
        let cid = self.cfg.client_id();
        if cid.is_empty() {
            self.sh.lock().unwrap().error =
                Some("Keine Client-ID hinterlegt. Bitte in den Einstellungen eine Azure Client-ID eintragen (siehe README).".into());
            self.show_settings = true;
            return;
        }
        self.cancel = Arc::default();
        let (sh, cancel, ctx) = (self.sh.clone(), self.cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = (|| -> Result<Account> {
                let dc = auth::device_code(&cid)?;
                sh.lock().unwrap().device = Some((dc.user_code.clone(), dc.verification_uri.clone()));
                ctx.request_repaint();
                let (access, refresh) = auth::poll_token(&cid, &dc, &cancel)?;
                let (_, name, uuid) = auth::minecraft_login(&access)?;
                Ok(Account { kind: Kind::Microsoft, name, uuid, refresh_token: Some(refresh) })
            })();
            let mut s = sh.lock().unwrap();
            s.device = None;
            match r {
                Ok(a) => s.updates.push(a),
                Err(e) => s.error = Some(e.to_string()),
            }
            ctx.request_repaint();
        });
    }

    fn start_launch(&mut self, ctx: &egui::Context) {
        let Some(acc) = self.selected_account().cloned() else {
            self.sh.lock().unwrap().error = Some("Bitte zuerst einen Account hinzuf\u{fc}gen und ausw\u{e4}hlen".into());
            return;
        };
        if self.selected.is_empty() {
            self.sh.lock().unwrap().error = Some("Bitte eine Version ausw\u{e4}hlen".into());
            return;
        }
        self.cfg.last_version = self.selected.clone();
        self.cfg.save(&self.root);
        {
            let mut s = self.sh.lock().unwrap();
            s.busy = true;
            s.log.clear();
            s.progress = 0.0;
        }
        let (sh, ctx, root, id, manifest) = (self.sh.clone(), ctx.clone(), self.root.clone(), self.selected.clone(), self.manifest.clone());
        let (ram, java_path, cid) = (self.cfg.ram_mb, self.cfg.java_path.clone(), self.cfg.client_id());
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
                status("Anmeldung pr\u{fc}fen", 0.0);
                let Some(rt) = acc.refresh_token.clone() else { bail!("Kein Refresh-Token, bitte Account neu anmelden") };
                let (access, new_rt) = auth::refresh(&cid, &rt)?;
                let (token, name, uuid) = auth::minecraft_login(&access)?;
                sh.lock().unwrap().updates.push(Account { kind: Kind::Microsoft, name: name.clone(), uuid: uuid.clone(), refresh_token: Some(new_rt) });
                let session = auth::Session { name, uuid, token, user_type: "msa" };
                let v = game::ensure_installed(&root, &id, &manifest, &status)?;
                sh.lock().unwrap().refresh_installed = true;
                let java = game::ensure_java(&root, &v, &java_path, &status)?;
                status("Minecraft l\u{e4}uft", 1.0);
                game::run_game(&root, &v, &session, &java, ram, &log)
            })();
            let mut s = sh.lock().unwrap();
            s.busy = false;
            s.progress = 0.0;
            match r {
                Ok(code) => s.status = format!("Minecraft beendet (Exit-Code {code})"),
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
        let (status, progress, busy, device, error) = {
            let s = self.sh.lock().unwrap();
            (s.status.clone(), s.progress, s.busy, s.device.clone(), s.error.clone())
        };

        egui::SidePanel::left("accounts").min_width(250.0).show(ctx, |ui| {
            ui.heading("Accounts");
            ui.separator();
            let mut remove: Option<usize> = None;
            for (i, a) in self.cfg.accounts.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.selectable_label(a.key() == self.cfg.selected, &a.name).clicked() {
                        self.cfg.selected = a.key();
                    }
                    if ui.small_button("x").on_hover_text("Account entfernen").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                let a = self.cfg.accounts.remove(i);
                if a.key() == self.cfg.selected {
                    self.cfg.selected = self.cfg.accounts.first().map(|a| a.key()).unwrap_or_default();
                }
                self.cfg.save(&self.root);
            }
            if self.cfg.accounts.is_empty() {
                ui.weak("Noch kein Account vorhanden");
            }
            ui.separator();
            ui.label("Microsoft-Account (Minecraft Java muss gekauft sein)");
            if let Some((code, uri)) = &device {
                ui.label("1. Seite \u{f6}ffnen:");
                ui.hyperlink(uri);
                ui.label("2. Diesen Code eingeben:");
                ui.label(egui::RichText::new(code).monospace().size(22.0).strong());
                ui.horizontal(|ui| {
                    if ui.button("Code kopieren").clicked() {
                        ctx.copy_text(code.clone());
                    }
                    if ui.button("Abbrechen").clicked() {
                        self.cancel.store(true, Ordering::SeqCst);
                    }
                });
                ui.weak("Warte auf Best\u{e4}tigung ...");
            } else if ui.button("Mit Microsoft anmelden").clicked() {
                self.start_microsoft_login(ctx);
            }
            ui.add_space(12.0);
            ui.collapsing("Einstellungen", |ui| {
                ui.checkbox(&mut self.show_settings, "Erweitert anzeigen");
            });
            if self.show_settings {
                ui.separator();
                ui.label("Azure Client-ID (f\u{fc}r Microsoft-Login)");
                if ui.text_edit_singleline(&mut self.cfg.ms_client_id).lost_focus() {
                    self.cfg.save(&self.root);
                }
                ui.label("Java-Pfad (leer = automatisch)");
                if ui.text_edit_singleline(&mut self.cfg.java_path).lost_focus() {
                    self.cfg.save(&self.root);
                }
                ui.label(format!("Datenordner: {}", self.root.display()));
            }
        });

        egui::TopBottomPanel::bottom("launch").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let sel = if self.selected.is_empty() { "keine Version".to_string() } else { self.selected.clone() };
                let installed = self.installed.contains(&self.selected);
                let label = if installed { format!("Spielen: {sel}") } else { format!("Installieren & spielen: {sel}") };
                if ui.add_enabled(!busy, egui::Button::new(egui::RichText::new(label).size(16.0)).min_size(egui::vec2(220.0, 32.0))).clicked() {
                    self.start_launch(ctx);
                }
                ui.add(egui::Slider::new(&mut self.cfg.ram_mb, 1024..=16384).step_by(256.0).text("RAM (MB)"));
            });
            if busy || progress > 0.0 {
                ui.add(egui::ProgressBar::new(progress).text(status.clone()));
            } else if !status.is_empty() {
                ui.label(&status);
            }
            if let Some(e) = &error {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(230, 90, 90), e);
                    if ui.small_button("OK").clicked() {
                        self.sh.lock().unwrap().error = None;
                    }
                });
            }
            ui.add_space(4.0);
        });

        egui::TopBottomPanel::bottom("log").resizable(true).default_height(120.0).show(ctx, |ui| {
            ui.label("Spiel-Log");
            egui::ScrollArea::vertical().id_salt("log").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                let lines = self.sh.lock().unwrap().log.clone();
                for l in lines {
                    ui.label(egui::RichText::new(l).monospace().size(11.0));
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Versionen");
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.show_release, "Releases");
                ui.checkbox(&mut self.show_snapshot, "Snapshots");
                ui.checkbox(&mut self.show_old, "Beta/Alpha");
                ui.checkbox(&mut self.only_installed, "Nur installierte");
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Suchen").desired_width(120.0));
            });
            ui.separator();
            let q = self.search.to_lowercase();
            let list: Vec<&Row> = self
                .rows
                .iter()
                .filter(|r| match r.kind.as_str() {
                    "release" => self.show_release,
                    "snapshot" => self.show_snapshot,
                    "custom" => true,
                    _ => self.show_old,
                })
                .filter(|r| !self.only_installed || self.installed.contains(&r.id))
                .filter(|r| q.is_empty() || r.id.to_lowercase().contains(&q))
                .collect();
            let h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
            let mut pick: Option<String> = None;
            egui::ScrollArea::vertical().id_salt("versions").auto_shrink([false, false]).show_rows(ui, h, list.len(), |ui, range| {
                for r in &list[range] {
                    let inst = if self.installed.contains(&r.id) { "  [installiert]" } else { "" };
                    let kind = match r.kind.as_str() {
                        "release" => "",
                        "snapshot" => "  Snapshot",
                        "custom" => "  Lokal",
                        _ => "  Alt",
                    };
                    if ui.selectable_label(self.selected == r.id, format!("{}{kind}{inst}", r.id)).clicked() {
                        pick = Some(r.id.clone());
                    }
                }
            });
            if let Some(p) = pick {
                self.selected = p;
            }
        });
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.cfg.last_version = self.selected.clone();
        self.cfg.save(&self.root);
    }
}

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 620.0]).with_title("Smash Launcher"),
        ..Default::default()
    };
    eframe::run_native("Smash Launcher", opts, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
}
