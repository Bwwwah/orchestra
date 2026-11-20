//! Updates in the window: a check at start-up (then every 6 hours), a card in the sidebar
//! when a newer version is out, and the settings (repository, automatic check).

use super::*;
use crate::update::{self, Release};
use std::sync::{Arc, Mutex};

type Slot<T> = Arc<Mutex<Option<T>>>;

#[derive(Default)]
pub struct Upd {
    check: Slot<Result<Release, String>>,
    checking: bool,
    checked: Option<Instant>,
    started: Option<Instant>,
    pub available: Option<Release>,
    /// The latest published version, newer or not: its notes are shown in the settings.
    latest: Option<Release>,
    /// The settings show the release notes (asked before the check came back: shown then).
    notes_open: bool,
    pub last: Option<Result<String, String>>,
    install: Slot<Result<std::path::PathBuf, String>>,
    installing: bool,
    ready: Option<std::path::PathBuf>,
}

impl NotesApp {
    fn update_repo(&self) -> String {
        let r = self.ws.settings.update_repo.trim();
        if r.is_empty() { update::built_in_repo().unwrap_or("").to_string() } else { r.to_string() }
    }

    pub(super) fn check_updates(&mut self, ctx: &egui::Context) {
        let repo = self.update_repo();
        if repo.is_empty() || self.upd.checking {
            return;
        }
        self.upd.checking = true;
        self.upd.checked = Some(Instant::now());
        let (slot, c) = (self.upd.check.clone(), ctx.clone());
        std::thread::spawn(move || {
            *slot.lock().unwrap() = Some(update::latest(&repo));
            c.request_repaint();
        });
    }

    /// Once per frame: start the checks, collect their results.
    pub(super) fn update_tick(&mut self, ctx: &egui::Context) {
        let started = *self.upd.started.get_or_insert_with(Instant::now);
        let auto = !self.ws.settings.no_update_check && std::env::var_os("ORCHESTRA_NO_SESSIONS").is_none();
        let due = match self.upd.checked {
            None => started.elapsed().as_secs() >= 5,
            Some(t) => t.elapsed().as_secs() >= 6 * 3600,
        };
        if auto && due {
            self.check_updates(ctx);
        }
        if let Some(r) = self.upd.check.lock().unwrap().take() {
            self.upd.checking = false;
            if let Ok(rel) = &r {
                self.upd.latest = Some(rel.clone());
            }
            match r {
                Ok(rel) if update::newer(&rel.tag, update::VERSION) => {
                    self.upd.last = Some(Ok(crate::trf!("Version {} disponible.", "Version {} available.", rel.tag)));
                    self.upd.available = Some(rel);
                }
                Ok(rel) => {
                    self.upd.last = Some(Ok(crate::trf!("À jour (dernière version : {}).", "Up to date (latest version: {}).", rel.tag)));
                    self.upd.available = None;
                }
                Err(e) => self.upd.last = Some(Err(e)),
            }
        }
        let done = self.upd.install.lock().unwrap().take();
        if let Some(r) = done {
            self.upd.installing = false;
            match r {
                Ok(p) => {
                    self.upd.ready = Some(p);
                    self.info(crate::tr!("Mise à jour installée : redémarrez pour l'utiliser.", "Update installed: restart to use it."));
                }
                Err(e) => self.fail(crate::trf!("Mise à jour impossible : {e}", "Update failed: {e}")),
            }
        }
    }

    fn install_update(&mut self, ctx: &egui::Context) {
        let Some(rel) = self.upd.available.clone() else { return };
        let repo = self.update_repo();
        self.upd.installing = true;
        let (slot, c) = (self.upd.install.clone(), ctx.clone());
        std::thread::spawn(move || {
            *slot.lock().unwrap() = Some(update::install(&repo, &rel));
            c.request_repaint();
        });
    }

    fn restart_now(&mut self) {
        if let Some(p) = self.upd.ready.clone() {
            // `exit` below runs no destructor: the terminals and sessions are stopped first
            // (a merge of a copy finishing meanwhile), then everything is saved.
            self.stop_children();
            self.save_now();
            update::restart(&p);
            std::process::exit(0);
        }
    }

    /// Sidebar: "Mise à jour v0.3.0" with its notes on hover, then Installer / Redémarrer.
    pub(super) fn update_card(&mut self, ui: &mut egui::Ui, t: &Tone) {
        let Some(rel) = self.upd.available.clone() else { return };
        ui.add_space(6.0);
        let accent = Color32::from_rgb(144, 101, 176);
        let ctx = ui.ctx().clone();
        let mut act = 0;
        egui::Frame::new()
            .fill(accent.gamma_multiply(0.12))
            .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.45)))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let head = ui.label(RichText::new(crate::trf!("Mise à jour {}", "Update {}", rel.tag)).font(semibold(13.0)).color(t.text));
                if !rel.notes.is_empty() {
                    head.on_hover_ui(|ui| {
                        ui.set_max_width(320.0);
                        super::rich::markdown(ui, &rel.notes, 13.0, t);
                    });
                }
                ui.label(RichText::new(crate::trf!("Vous avez la {}.", "You have {}.", update::VERSION)).size(12.0).color(t.tertiary));
                ui.add_space(4.0);
                if self.upd.ready.is_some() {
                    if super::projects::action_link(ui, crate::tr!("Redémarrer", "Restart")).clicked() {
                        act = 2;
                    }
                } else if self.upd.installing {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(RichText::new(crate::tr!("Installation…", "Installing…")).size(12.5).color(t.secondary));
                    });
                } else if update::target().is_none() {
                    ui.label(RichText::new(crate::tr!("Lancée depuis le code : git pull puis cargo build.", "Running from source: git pull then cargo build.")).size(12.0).color(t.tertiary));
                } else if super::projects::action_link(ui, crate::tr!("Installer", "Install")).clicked() {
                    act = 1;
                }
            });
        match act {
            1 => self.install_update(&ctx),
            2 => self.restart_now(),
            _ => {}
        }
    }

    /// Settings › General › Updates.
    pub(super) fn update_settings(&mut self, ui: &mut egui::Ui, t: &Tone, l: &super::settings::Look) {
        super::settings::section(ui, crate::tr!("Mises à jour", "Updates"), t, l);
        let built = update::built_in_repo().unwrap_or("");
        let ctx = ui.ctx().clone();
        super::settings::row(ui, crate::trf!("Version {}", "Version {}", update::VERSION).as_str(), crate::tr!("Les nouvelles versions viennent des versions publiées sur le dépôt GitHub du projet.", "New versions come from the releases published on the project's GitHub repository."), t, |ui| {
            if self.upd.checking {
                ui.add(egui::Spinner::new().size(16.0));
            } else if super::settings::button(ui, crate::tr!("Vérifier", "Check"), t, l).clicked() {
                self.check_updates(&ctx);
            }
        });
        if let Some(r) = &self.upd.last {
            let (text, color) = match r {
                Ok(m) => (m.clone(), t.secondary),
                Err(e) => (e.clone(), super::projects::ORANGE),
            };
            ui.label(RichText::new(text).size(13.0).color(color));
        }
        // What's in the latest version: its notes, fetched with the check.
        let repo = self.update_repo();
        super::settings::row(ui, crate::tr!("Notes de version", "Release notes"), crate::tr!("Ce qui a changé dans la dernière version publiée.", "What changed in the latest published version."), t, |ui| {
            let label = if self.upd.notes_open { crate::tr!("Masquer", "Hide") } else { crate::tr!("Voir", "Show") };
            if super::settings::button(ui, label, t, l).clicked() {
                self.upd.notes_open = !self.upd.notes_open;
                if self.upd.notes_open && self.upd.latest.is_none() {
                    self.check_updates(&ctx);
                }
            }
        });
        if self.upd.notes_open {
            egui::Frame::new()
                .fill(t.card)
                .stroke(egui::Stroke::new(1.0, t.border))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match &self.upd.latest {
                        Some(rel) => {
                            ui.label(RichText::new(rel.tag.as_str()).font(semibold(14.0)).color(t.text));
                            if rel.notes.trim().is_empty() {
                                ui.label(RichText::new(crate::tr!("Pas de notes pour cette version.", "No notes for this version.")).size(13.0).color(t.tertiary).italics());
                            } else {
                                egui::ScrollArea::vertical().id_salt("release-notes").max_height(320.0).show(ui, |ui| {
                                    super::rich::markdown(ui, &rel.notes, 13.0, t);
                                });
                            }
                        }
                        None if self.upd.checking => {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new().size(12.0));
                                ui.label(RichText::new(crate::tr!("Lecture des versions publiées…", "Reading the published versions…")).size(13.0).color(t.secondary));
                            });
                        }
                        None => {
                            let why = if repo.is_empty() { crate::tr!("Indiquez le dépôt GitHub ci-dessous.", "Set the GitHub repository below.") } else { crate::tr!("Notes indisponibles pour l'instant.", "Notes unavailable for now.") };
                            ui.label(RichText::new(why).size(13.0).color(t.tertiary));
                        }
                    }
                    if !repo.is_empty() && super::projects::action_link(ui, crate::tr!("Toutes les versions sur GitHub", "All versions on GitHub")).clicked()
                        && let Err(e) = crate::opener::open_url(&format!("https://github.com/{}/releases", repo.trim())) {
                            self.fail(e);
                        }
                });
        }
        let hint = if built.is_empty() { "propriétaire/dépôt".to_string() } else { built.to_string() };
        super::settings::row(ui, crate::tr!("Dépôt", "Repository"), crate::tr!("propriétaire/dépôt sur GitHub. Privé : chaque personne doit y avoir accès et être connectée avec « gh ».", "owner/repository on GitHub. Private: each person needs access to it and to be signed in with « gh »."), t, |ui| {
            if ui.add(egui::TextEdit::singleline(&mut self.ws.settings.update_repo).hint_text(hint).desired_width(220.0).font(FontId::monospace(12.5))).changed() {
                self.dirty_since.get_or_insert_with(Instant::now);
            }
        });
        let mut auto = !self.ws.settings.no_update_check;
        super::settings::row(ui, crate::tr!("Vérifier automatiquement", "Check automatically"), crate::tr!("Au démarrage, puis toutes les 6 heures.", "At start-up, then every 6 hours."), t, |ui| {
            if super::settings::switch(ui, &mut auto, l).changed() {
                self.ws.settings.no_update_check = !auto;
                self.dirty_since.get_or_insert_with(Instant::now);
            }
        });
    }
}
