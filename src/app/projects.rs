//! The AI projects in the window: the "Projets" dashboard (one card per project) and the
//! project panel shown at the top of a project's page (status, folder, state of play,
//! steps, AI sessions). The AIs fill the same data through MCP (see mcp.rs).

use super::chrome::relative_time;
use super::*;
use crate::project::{Project, ProjectStatus, StepStatus};
use crate::sessions::{SessionInfo, human_tokens};
use std::collections::HashMap;
use std::sync::Arc;

/// The live sessions sorted by project (the deepest project folder holding their `cwd`),
/// recomputed only when the sessions or the projects' folders change.
#[derive(Default)]
pub struct LiveIndex {
    src: Arc<Vec<SessionInfo>>,
    sig: String,
    pub by_project: HashMap<String, Vec<SessionInfo>>,
    /// Sessions in folders no project holds, grouped by folder (most recent first).
    pub unassigned: Vec<(String, Vec<SessionInfo>)>,
}

/// A folder offered for synchronisation.
pub struct SyncCandidate {
    pub folder: String,
    pub name: String,
    pub tool: crate::sessions::Tool,
    pub last: u64,
    pub is_project: bool,
    pub checked: bool,
}

impl LiveIndex {
    pub fn all(&self) -> Arc<Vec<SessionInfo>> {
        self.src.clone()
    }

    pub fn refresh(&mut self, ws: &Workspace, live: Arc<Vec<SessionInfo>>) {
        let sig: String = ws
            .projects()
            .iter()
            .map(|p| {
                let pr = p.project.as_ref();
                format!("{}={}@{};", p.id, pr.map(|x| x.folder.as_str()).unwrap_or(""), pr.map(|x| x.sessions_cleared).unwrap_or(0))
            })
            .collect();
        if Arc::ptr_eq(&self.src, &live) && sig == self.sig {
            return;
        }
        self.by_project.clear();
        let mut loose: Vec<(String, Vec<SessionInfo>)> = vec![];
        for s in live.iter() {
            match ws.project_for_path(std::path::Path::new(&s.cwd)) {
                Some(p) => {
                    // Hidden by "Effacer l'historique" until it is used again.
                    if p.project.as_ref().is_none_or(|pr| pr.session_shown(s.last_activity)) {
                        self.by_project.entry(p.id.clone()).or_default().push(s.clone());
                    }
                }
                None => match loose.iter_mut().find(|(f, _)| *f == s.cwd) {
                    Some((_, v)) => v.push(s.clone()),
                    None => loose.push((s.cwd.clone(), vec![s.clone()])),
                },
            }
        }
        self.unassigned = loose;
        self.src = live;
        self.sig = sig;
    }
}

/// Last activity, session count and tokens of a project, live sessions included.
fn activity(pr: &Project, updated: u64, live: &[SessionInfo]) -> (u64, usize, u64) {
    let last = live.iter().map(|s| s.last_activity).max().unwrap_or(0).max(pr.last_activity).max(updated);
    let stored_only = pr.sessions.iter().filter(|x| !live.iter().any(|l| l.id == x.id)).count();
    let tokens = live.iter().map(|s| s.tokens.billed).sum();
    (last, live.len() + stored_only, tokens)
}

fn now() -> u64 {
    crate::model::now_ms()
}

pub(super) const BLUE: Color32 = Color32::from_rgb(35, 131, 226);
pub(super) const GREEN: Color32 = Color32::from_rgb(68, 131, 97);
pub(super) const ORANGE: Color32 = Color32::from_rgb(217, 115, 13);

fn status_color(s: ProjectStatus, t: &Tone) -> Color32 {
    match s {
        ProjectStatus::Idea => t.secondary,
        ProjectStatus::Active => BLUE,
        ProjectStatus::Paused => ORANGE,
        ProjectStatus::Done => GREEN,
    }
}

fn step_color(s: StepStatus, t: &Tone) -> Color32 {
    match s {
        StepStatus::Todo => t.secondary,
        StepStatus::Doing => BLUE,
        StepStatus::Done => GREEN,
    }
}

/// A small rounded tag with coloured text.
fn chip(ui: &mut egui::Ui, text: &str, color: Color32, sense: egui::Sense) -> egui::Response {
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(12.0), color);
    let (rect, resp) = ui.allocate_exact_size(g.size() + egui::vec2(14.0, 6.0), sense);
    let fill = color.gamma_multiply(if resp.hovered() { 0.28 } else { 0.16 });
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().galley(rect.min + egui::vec2(7.0, 3.0), g, color);
    resp
}

/// The tool of a session ("Claude Code", "Codex"…) as a chip in a fixed-width column, so
/// the titles of the rows below each other start at the same place.
fn tool_chip(ui: &mut egui::Ui, label: &str, t: &Tone) {
    let col = ui.painter().layout_no_wrap("Claude Code".into(), FontId::proportional(12.0), t.secondary).size().x + 14.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(col, 20.0), egui::Sense::hover());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    chip(&mut child, label, t.secondary, egui::Sense::hover());
}

/// "claude-code" → "Claude Code": the tool names written by the AIs, as shown elsewhere.
fn tool_label(tool: &str) -> String {
    match tool.trim().to_lowercase().replace(['_', ' '], "-").as_str() {
        "claude-code" | "claude" => "Claude Code".into(),
        "codex" | "codex-cli" => "Codex".into(),
        "claude-web" => "Claude (web)".into(),
        "gemini-cli" | "gemini" => "Gemini".into(),
        "chatgpt" => "ChatGPT".into(),
        _ => tool.to_string(),
    }
}

/// The command that plugs this app into Claude Code as an MCP server.
pub fn mcp_command() -> String {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "orchestra".into());
    format!("claude mcp add --scope user orchestra -- \"{exe}\" mcp")
}

impl NotesApp {
    /// "Projets": every AI project as a card, most recent activity first.
    pub(super) fn projects_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let t = tone(ui.visuals().dark_mode);
        let avail = ui.available_width();
        let width = 960.0f32.min(avail - 40.0).max(300.0);
        ui.add_space(50.0);
        ui.horizontal(|ui| {
            ui.add_space(((avail - width) / 2.0).max(20.0));
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(crate::tr!("Projets", "Projects")).font(bold(30.0)).color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.new_project.is_none() && ui.button(RichText::new(crate::tr!("+ Nouveau projet", "+ New project")).size(14.0)).clicked() {
                            self.new_project = Some((String::new(), String::new()));
                        }
                        let busy = self.sync_dialog.is_some() || self.sync_run.is_some();
                        if ui
                            .add_enabled(!busy, egui::Button::new(RichText::new(crate::tr!("Synchroniser avec les IA", "Sync with the AIs")).size(14.0)))
                            .on_hover_text(crate::tr!("Demande à Claude Code (ou Codex) de se brancher au tableau de bord et de faire l'état des lieux de vos projets", "Asks Claude Code (or Codex) to plug into the dashboard and write where each of your projects stands"))
                            .clicked()
                        {
                            self.sync_dialog = Some(self.sync_candidates());
                        }
                    });
                });
                ui.label(RichText::new(crate::tr!("Où en est chaque projet, ce qu'il reste à faire, et les sessions IA qui y ont travaillé.", "Where each project stands, what is left to do, and the AI sessions that worked on it.")).color(t.secondary));
                self.mcp_line(ui, &t);
                ui.add_space(16.0);

                self.sync_view(ui, width, &t);

                // A folder dropped on the window: start a project from it.
                let dropped: Option<std::path::PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).find(|p| p.is_dir()));
                if let Some(dir) = dropped {
                    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    self.new_project = Some((name, dir.display().to_string()));
                }

                // New project form: a name, the folder, and whether an AI should analyse it.
                let mut create = None;
                let mut cancel = false;
                if let Some((title, folder)) = self.new_project.as_mut() {
                    egui::Frame::new().fill(t.card).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(12.0).inner_margin(egui::Margin::symmetric(18, 16)).show(ui, |ui| {
                        ui.set_width(width - 38.0);
                        ui.label(RichText::new(crate::tr!("Nouveau projet", "New project")).font(semibold(16.0)).color(t.text));
                        ui.label(
                            RichText::new(crate::tr!("Un projet existant, même sans IA jusqu'ici : indiquez son dossier, une IA l'analyse et remplit le tableau de bord. Astuce : glissez un dossier sur la fenêtre.", "An existing project, even without AI so far: give its folder, an AI analyses it and fills in the dashboard. Tip: drop a folder on the window."))
                                .size(13.0)
                                .color(t.secondary),
                        );
                        ui.add_space(8.0);
                        egui::Grid::new("new-project").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                            ui.label(crate::tr!("Dossier", "Folder"));
                            let fr = ui.add(egui::TextEdit::singleline(folder).hint_text(crate::tr!("~/Code/mon-projet", "~/Code/my-project")).font(FontId::monospace(13.0)).desired_width(f32::INFINITY));
                            if title.is_empty() && folder.is_empty() && !fr.has_focus() {
                                fr.request_focus();
                            }
                            ui.end_row();
                            ui.label(crate::tr!("Nom", "Name"));
                            let auto = std::path::Path::new(folder.trim()).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                            ui.add(egui::TextEdit::singleline(title).hint_text(if auto.is_empty() { crate::tr!("Mon projet", "My project").to_string() } else { auto }).desired_width(f32::INFINITY));
                            ui.end_row();
                        });
                        let analyse_key = egui::Id::unique("new-project-analyse");
                        let mut analyse = ui.ctx().data(|d| d.get_temp::<bool>(analyse_key)).unwrap_or(true);
                        let folder_ok = crate::project::normalize_folder(folder.trim()).is_dir();
                        ui.add_space(4.0);
                        ui.add_enabled_ui(folder_ok, |ui| {
                            if ui.checkbox(&mut analyse, crate::tr!("Faire remplir le projet par une IA : état des lieux, statut, regard extérieur (lecture seule ; pas les étapes)", "Have an AI fill in the project: current state, status, outside view (read-only; not the steps)")).changed() {
                                ui.ctx().data_mut(|d| d.insert_temp(analyse_key, analyse));
                            }
                        });
                        if !folder.trim().is_empty() && !folder_ok {
                            ui.label(RichText::new(crate::tr!("Ce dossier n'existe pas sur cette machine.", "This folder doesn't exist on this computer.")).size(12.5).color(ORANGE));
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let name = if title.trim().is_empty() {
                                std::path::Path::new(folder.trim()).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                            } else {
                                title.trim().to_string()
                            };
                            let ok = !name.is_empty();
                            let enter = ui.input(|i| i.key_pressed(Key::Enter));
                            let label = if analyse && folder_ok { crate::tr!("Créer et analyser", "Create and analyse") } else { crate::tr!("Créer", "Create") };
                            if ui.add_enabled(ok, egui::Button::new(label)).clicked() || (ok && enter) {
                                create = Some((name, folder.trim().to_string(), analyse && folder_ok));
                            }
                            if ui.button(crate::tr!("Annuler", "Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                                cancel = true;
                            }
                        });
                    });
                    ui.add_space(16.0);
                }
                if let Some((title, folder, analyse)) = create {
                    actions.push(Action::NewProject(title, folder.clone()));
                    self.new_project = None;
                    if analyse {
                        super::ai_fill::request(ui.ctx(), super::ai_fill::AiReq::FillFolder { folder });
                    }
                } else if cancel {
                    self.new_project = None;
                }

                let projects: Vec<(String, String, String, Project, u64)> = self
                    .ws
                    .projects()
                    .into_iter()
                    .map(|p| (p.id.clone(), page_icon(p).to_string(), p.display_title(), p.project.clone().unwrap_or_default(), p.updated))
                    .collect();
                let mut projects = projects;
                projects.sort_by_key(|(id, _, _, pr, upd)| {
                    std::cmp::Reverse(activity(pr, *upd, self.agents.by_project.get(id).map(Vec::as_slice).unwrap_or(&[])).0)
                });
                if projects.is_empty() && self.agents.unassigned.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(crate::tr!("Aucun projet pour l'instant", "No project yet")).size(18.0).color(t.secondary));
                        ui.add_space(6.0);
                        ui.label(RichText::new(crate::tr!("Créez-en un ici, ou demandez à votre IA de le faire une fois branchée :", "Create one here, or ask your AI to do it once plugged in:")).color(t.tertiary));
                    });
                    ui.add_space(10.0);
                    let mut cmd = mcp_command();
                    ui.add(egui::TextEdit::singleline(&mut cmd).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
                    return;
                }

                // Cards, three per row on a wide window.
                if projects.is_empty() {
                    ui.label(RichText::new(crate::tr!("Aucun projet pour l'instant : créez-en un à partir d'un dossier ci-dessous.", "No project yet: create one from a folder below.")).color(t.secondary));
                }
                let gap = 14.0;
                let cols = ((width + gap) / (300.0 + gap)).floor().max(1.0);
                let card_w = (width - gap * (cols - 1.0)) / cols;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                    for (id, icon, title, pr, updated) in &projects {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(card_w, 168.0), egui::Sense::click());
                        let p = ui.painter();
                        p.rect_filled(rect, 12.0, if resp.hovered() { t.selected } else { t.card });
                        p.rect_stroke(rect, 12.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                        let inner = rect.shrink(16.0);
                        let mut ui2 = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
                        ui2.spacing_mut().item_spacing.y = 6.0;
                        ui2.horizontal(|ui| {
                            ui.label(RichText::new(icon).size(20.0));
                            let mut job = egui::text::LayoutJob::single_section(title.clone(), egui::TextFormat::simple(semibold(16.0), t.text));
                            job.wrap = egui::text::TextWrapping::truncate_at_width(card_w - 140.0);
                            cut_label(ui, job);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                chip(ui, pr.status.label(), status_color(pr.status, &t), egui::Sense::hover());
                            });
                        });
                        let folder = if pr.folder.is_empty() { crate::tr!("Pas de dossier", "No folder").to_string() } else { pr.folder.clone() };
                        let mut job = egui::text::LayoutJob::single_section(folder, egui::TextFormat::simple(FontId::monospace(11.0), t.tertiary));
                        job.wrap = egui::text::TextWrapping::truncate_at_width(card_w - 34.0);
                        ui2.label(job);
                        // Progress bar.
                        let (done, total) = pr.progress();
                        let (bar, _) = ui2.allocate_exact_size(egui::vec2(card_w - 32.0, 6.0), egui::Sense::hover());
                        ui2.painter().rect_filled(bar, 3.0, t.fill);
                        if total > 0 {
                            let w = bar.width() * done as f32 / total as f32;
                            ui2.painter().rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(w, bar.height())), 3.0, GREEN);
                        }
                        ui2.label(RichText::new(crate::trf!("{done}/{total} étapes", "{done}/{total} steps")).size(12.0).color(t.secondary));
                        let next = pr.next_step().map(|s| format!("→ {}", s.title)).unwrap_or_else(|| crate::tr!("Aucune étape en attente", "No step pending").into());
                        let mut job = egui::text::LayoutJob::single_section(next, egui::TextFormat::simple(FontId::proportional(13.0), t.text));
                        job.wrap = egui::text::TextWrapping::truncate_at_width(card_w - 34.0);
                        ui2.label(job);
                        let live = self.agents.by_project.get(id).map(Vec::as_slice).unwrap_or(&[]);
                        let (last, count, tokens) = activity(pr, *updated, live);
                        let mut meta = relative_time(last);
                        match count {
                            0 => {}
                            1 => meta.push_str(crate::tr!(" · 1 session IA", " · 1 AI session")),
                            n => meta.push_str(&crate::trf!(" · {n} sessions IA", " · {n} AI sessions")),
                        }
                        if tokens > 0 {
                            meta.push_str(&crate::trf!(" · {} tokens", " · {} tokens", human_tokens(tokens)));
                        }
                        ui2.horizontal(|ui| {
                            if live.iter().any(|s| s.is_live(now())) {
                                ui.label(RichText::new("●").size(10.0).color(GREEN)).on_hover_text(crate::tr!("Une IA travaille en ce moment sur ce projet", "An AI is working on this project right now"));
                            }
                            ui.label(RichText::new(meta).size(12.0).color(t.tertiary));
                            // Teammates in this project right now.
                            let peers = self.peers_in_project(id);
                            if !peers.is_empty() {
                                let names: Vec<String> = peers.iter().map(|(n, _)| n.clone()).collect();
                                super::live_ui::faces(ui, &names, t.card);
                            }
                        });
                        if resp.clicked() {
                            actions.push(Action::Open(id.clone()));
                        }
                    }
                });
                self.unassigned_view(ui, width, actions, &t);
            });
        });
    }
}

impl NotesApp {
    /// Folders where Claude Code / Codex worked that no project holds yet.
    fn unassigned_view(&self, ui: &mut egui::Ui, width: f32, actions: &mut Vec<Action>, t: &Tone) {
        if self.agents.unassigned.is_empty() {
            return;
        }
        ui.add_space(28.0);
        ui.label(RichText::new(crate::tr!("Dossiers avec des sessions IA, sans projet", "Folders with AI sessions, without a project")).font(semibold(16.0)).color(t.text));
        ui.label(RichText::new(crate::tr!("Détectés dans les sessions Claude Code et Codex de cette machine.", "Found in this computer's Claude Code and Codex sessions.")).size(13.0).color(t.secondary));
        ui.add_space(8.0);
        for (folder, list) in self.agents.unassigned.iter().take(12) {
            egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8.0).inner_margin(10).show(ui, |ui| {
                ui.set_width(width - 22.0);
                ui.horizontal(|ui| {
                    let name = std::path::Path::new(folder).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| folder.clone());
                    let btn_w = 130.0;
                    let room = (ui.available_width() - btn_w).max(60.0);
                    ui.vertical(|ui| {
                        ui.set_width(room);
                        ui.label(RichText::new(&name).font(semibold(14.0)).color(t.text));
                        let mut job = egui::text::LayoutJob::single_section(folder.clone(), egui::TextFormat::simple(FontId::monospace(11.0), t.tertiary));
                        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                        cut_label(ui, job);
                        let last = list.iter().map(|s| s.last_activity).max().unwrap_or(0);
                        let tools: Vec<&str> = {
                            let mut v: Vec<&str> = list.iter().map(|s| s.tool.label()).collect();
                            v.dedup();
                            v
                        };
                        ui.label(RichText::new(crate::trf!("{} session(s) · {} · {}", "{} session(s) · {} · {}", list.len(), tools.join(", "), relative_time(last))).size(12.0).color(t.secondary));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(crate::tr!("Créer le projet", "Create the project")).on_hover_text(crate::tr!("Une IA le remplit ensuite : état des lieux, statut, regard extérieur", "An AI then fills it in: current state, status, outside view")).clicked() {
                            actions.push(Action::NewProject(name.clone(), folder.clone()));
                            super::ai_fill::request(ui.ctx(), super::ai_fill::AiReq::FillFolder { folder: folder.clone() });
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }
    }
}

// ---------------------------------------------------------------- project page panel
//
// Layout, top to bottom, each part in its own card with air around it:
//   header   status · progress bar · activity · tokens · "IA au travail"; folder in small
//   summary  the state of play, folded to a few lines with a fade and "Afficher tout"
//   steps    open steps one line each (click to unfold), done steps folded away
//   sessions one compact line each, details and resume buttons on unfold

const TEXT: f32 = 14.0;

/// A card; `extra` adds blank space at its bottom (to line it up with its neighbours).
/// Returns its natural height, without that space.
pub(super) fn card(ui: &mut egui::Ui, t: &Tone, extra: f32, add: impl FnOnce(&mut egui::Ui)) -> f32 {
    // The border lights up softly under the pointer (last frame's rect).
    let id = ui.next_auto_id().with("card-hover");
    let prev: Option<egui::Rect> = ui.ctx().data(|d| d.get_temp(id));
    let hovered = prev.is_some_and(|r| ui.rect_contains_pointer(r));
    let a = ui.ctx().animate_bool_with_time(id, hovered, 0.18);
    let border = t.border.lerp_to_gamma(t.tertiary, 0.6 * a);
    let r = egui::Frame::new()
        .fill(t.card)
        .shadow(egui::epaint::Shadow { offset: [0, (2.0 + 4.0 * a) as i8], blur: (6.0 + 10.0 * a) as u8, spread: 0, color: Color32::from_black_alpha((18.0 + 30.0 * a) as u8) })
        .stroke(egui::Stroke::new(1.0, border))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(18, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
            if extra > 0.5 {
                ui.add_space(extra);
            }
        });
    ui.ctx().data_mut(|d| d.insert_temp(id, r.response.rect));
    r.response.rect.height() - extra.max(0.0)
}

/// Accent of each card, shown as a small bar before its title.
pub(super) fn accent_of(title: &str) -> Color32 {
    match title {
        "ÉTAT DES LIEUX" | "CURRENT STATE" => Color32::from_rgb(144, 101, 176),
        "ÉTAPES" | "STEPS" => GREEN,
        "SESSIONS IA" | "AI SESSIONS" => Color32::from_rgb(204, 120, 92),
        "VERSIONS" => BLUE,
        "PAGES" | "CONTENU" | "CONTENTS" => Color32::from_rgb(160, 160, 160),
        _ => Color32::from_rgb(212, 76, 125),
    }
}

/// A row's background that fades in under the pointer: call `hover_begin` before drawing
/// the row, `hover_end` with its rect after.
pub(super) fn hover_begin(ui: &egui::Ui) -> egui::layers::ShapeIdx {
    ui.painter().add(egui::Shape::Noop)
}

pub(super) fn hover_end(ui: &egui::Ui, idx: egui::layers::ShapeIdx, rect: egui::Rect, id: egui::Id, t: &Tone) -> f32 {
    let r = rect.expand2(egui::vec2(6.0, 2.0));
    let a = ui.ctx().animate_bool_with_time(id, ui.rect_contains_pointer(r), 0.14);
    if a > 0.0 {
        ui.painter().set(idx, egui::Shape::rect_filled(r, 7.0, t.hover.gamma_multiply(a)));
    }
    a
}

/// A dot that breathes: something is happening right now.
pub(super) fn pulse_dot(ui: &egui::Ui, c: egui::Pos2, color: Color32) {
    let time = ui.input(|i| i.time);
    let ph = (time % 1.6 / 1.6) as f32;
    ui.painter().circle_filled(c, 4.0 + 5.0 * ph, color.gamma_multiply(0.45 * (1.0 - ph)));
    ui.painter().circle_filled(c, 4.0, color);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

/// Fraction going smoothly to `target` (bars, progress).
pub(super) fn smooth(ui: &egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, target: f32) -> f32 {
    ui.ctx().animate_value_with_time(egui::Id::unique(id), target, 0.6)
}

/// "ÉTAPES        3 / 10" style header: small caps on the left, extra on the right.
pub(super) fn card_title(ui: &mut egui::Ui, title: &str, extra: &str, t: &Tone) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(4.0, 14.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 2.0, accent_of(title));
        ui.add_space(2.0);
        ui.label(RichText::new(title).font(semibold(12.5)).color(t.secondary));
        if !extra.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(extra).size(12.5).color(t.tertiary));
            });
        }
    });
    ui.add_space(8.0);
}

fn flag(ui: &egui::Ui, key: (&str, &str)) -> bool {
    ui.ctx().data(|d| d.get_temp::<bool>(egui::Id::unique(key)).unwrap_or(false))
}

fn toggle_flag(ui: &egui::Ui, key: (&str, &str)) {
    let v = flag(ui, key);
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(key), !v));
}

fn link(ui: &mut egui::Ui, text: &str) -> bool {
    action_link(ui, text).clicked()
}

/// A small action: a soft blue pill that brightens under the pointer, an arrow that slides
/// in, and a press that sinks it a little.
pub(super) fn action_link(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let arrow = !text.starts_with('+') && !text.ends_with('▾') && !text.ends_with('▴');
    let font = FontId::proportional(12.5);
    let g = ui.painter().layout_no_wrap(text.to_string(), font.clone(), BLUE);
    let extra = if arrow { 14.0 } else { 0.0 };
    let size = g.size() + egui::vec2(18.0 + extra, 8.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let a = ui.ctx().animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.16);
    let down = ui.ctx().animate_bool_with_time(resp.id.with("down"), resp.is_pointer_button_down_on(), 0.08);
    let r = rect.shrink(1.2 * down);
    let p = ui.painter();
    p.rect_filled(r, 8.0, BLUE.gamma_multiply(0.10 + 0.14 * a));
    if a > 0.0 {
        p.rect_stroke(r, 8.0, egui::Stroke::new(1.0, BLUE.gamma_multiply(0.45 * a)), egui::StrokeKind::Inside);
    }
    let color = BLUE.lerp_to_gamma(Color32::from_rgb(120, 180, 255), a * 0.6);
    let pos = egui::pos2(r.left() + 9.0, r.center().y - g.size().y / 2.0);
    p.galley(pos, g, color);
    if arrow {
        let x = r.right() - 13.0 + 3.0 * a;
        p.text(egui::pos2(x, r.center().y), egui::Align2::CENTER_CENTER, "→", font, color.gamma_multiply(0.35 + 0.65 * a));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One-line text cut at the width, with "…". Returns its response (hover shows all).
fn line(ui: &mut egui::Ui, job: egui::text::LayoutJob, width: f32) -> egui::Response {
    let mut job = job;
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(30.0));
    let g = ui.fonts_mut(|f| f.layout_job(job));
    let (r, resp) = ui.allocate_exact_size(egui::vec2(width.max(30.0), g.size().y), egui::Sense::click());
    ui.painter().galley(r.min, g, t_text_fallback());
    resp
}

/// Text cut at its job's width with "…", whatever the layout (a Label in a row would
/// stretch to the whole text instead, and push the card).
pub(super) fn cut_label(ui: &mut egui::Ui, job: egui::text::LayoutJob) -> egui::Response {
    let g = ui.fonts_mut(|f| f.layout_job(job));
    let (r, resp) = ui.allocate_exact_size(g.size(), egui::Sense::hover());
    ui.painter().galley(r.min, g, Color32::PLACEHOLDER);
    resp
}

fn t_text_fallback() -> Color32 {
    Color32::PLACEHOLDER
}

/// "Backend : implémenter X" → ("Backend", "implémenter X"): AIs often prefix steps
/// with an area; it becomes a small tag so the line reads faster.
fn split_area(title: &str) -> (Option<&str>, &str) {
    if let Some((a, rest)) = title.split_once(':') {
        let a = a.trim();
        if !a.is_empty() && a.chars().count() <= 18 && a.split_whitespace().count() <= 2 && !a.contains('`') && !rest.trim().is_empty() {
            return (Some(a), rest.trim());
        }
    }
    (None, title)
}

/// Status of a step as a round icon: ○ to do, ◐ in progress, ● done.
fn step_icon(ui: &mut egui::Ui, step_id: &str, st: StepStatus, t: &Tone) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::click());
    let c = r.center();
    let hov = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.12);
    let color = step_color(st, t).lerp_to_gamma(BLUE, hov);
    // Filling in when it becomes "en cours" or "terminée", with a small pop.
    let doing = ui.ctx().animate_bool_with_time(egui::Id::unique(("step-doing", step_id)), st == StepStatus::Doing, 0.25);
    let done = ui.ctx().animate_bool_with_time(egui::Id::unique(("step-done", step_id)), st == StepStatus::Done, 0.3);
    let pop = 1.0 + 0.25 * (done * std::f32::consts::PI).sin();
    let p = ui.painter();
    p.circle_stroke(c, 6.5 * pop, egui::Stroke::new(1.5, color));
    if doing > 0.0 {
        p.circle_filled(c, 3.5 * doing, color);
    }
    if done > 0.0 {
        p.circle_filled(c, 7.0 * done * pop, color);
        let k = done;
        p.line_segment([c + egui::vec2(-3.0, 0.2) * k, c + egui::vec2(-0.8, 2.6) * k], egui::Stroke::new(1.6, t.card));
        p.line_segment([c + egui::vec2(-0.8, 2.6) * k, c + egui::vec2(3.4, -2.4) * k], egui::Stroke::new(1.6, t.card));
    }
    resp.on_hover_text(crate::trf!("{} — cliquer pour changer", "{} — click to change", st.label()))
}

fn session_row(ui: &mut egui::Ui, page_id: &str, s: &SessionInfo, t: &Tone) -> Option<super::notices::Notice> {
    let mut msg = None;
    let open_key = ("session-open", s.id.as_str());
    let open = flag(ui, open_key);
    let w = ui.available_width();
    let bg = hover_begin(ui);
    let row = ui
        .horizontal(|ui| {
            let live = s.is_live(now());
            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 20.0), egui::Sense::hover());
            if live {
                pulse_dot(ui, r.center(), GREEN);
            } else {
                ui.painter().circle_filled(r.center(), 4.0, t.border);
            }
            tool_chip(ui, s.tool.label(), t);
            let when = relative_time(s.last_activity);
            let when_w = ui.painter().layout_no_wrap(when.clone(), FontId::proportional(12.5), t.tertiary).size().x;
            let room = ui.available_width() - when_w - 16.0;
            let job = egui::text::LayoutJob::single_section(s.display_title(), egui::TextFormat::simple(FontId::proportional(TEXT), t.text));
            let r = line(ui, job, room);
            ui.label(RichText::new(when).size(12.5).color(t.tertiary));
            r
        });
    hover_end(ui, bg, row.response.rect, egui::Id::unique(("session-row", &s.id)), t);
    let resp = row.inner;
    if resp.clicked() {
        toggle_flag(ui, open_key);
    }
    let _ = page_id;
    if open {
        egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 0, top: 2, bottom: 6 }).show(ui, |ui| {
            ui.set_max_width(w - 18.0);
            let mut meta = crate::trf!("{} message(s)", "{} message(s)", s.user_messages + s.agent_messages);
            if s.tokens.billed > 0 {
                meta.push_str(&crate::trf!(" · {} tokens", " · {} tokens", human_tokens(s.tokens.billed)));
            }
            for x in [&s.git_branch, &s.model] {
                if !x.is_empty() {
                    meta.push_str(&format!(" · {x}"));
                }
            }
            ui.label(RichText::new(meta).size(12.5).color(t.tertiary));
            if !s.last_reply.is_empty() {
                ui.add_space(2.0);
                ui.add(egui::Label::new(RichText::new(crate::trf!("Dernière réponse : {}", "Last answer: {}", s.last_reply)).size(13.0).color(t.secondary)).wrap());
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let cmd = s.resume_command();
                if ui.button(crate::tr!("Reprendre ici", "Resume here")).on_hover_text(crate::tr!("Rouvre la session dans le terminal intégré", "Reopens the session in the built-in terminal")).clicked() {
                    let resume = match s.tool {
                        crate::sessions::Tool::ClaudeCode => format!("claude --resume {}", s.id),
                        crate::sessions::Tool::Codex => format!("codex resume {}", s.id),
                    };
                    let title = format!("{} · {}", s.tool.label(), crate::richtext::one_line(&s.display_title(), 24));
                    super::term_ui::request(ui.ctx(), &title, &s.cwd, Some(resume));
                }
                if ui.button(crate::tr!("Terminal macOS", "macOS Terminal")).on_hover_text(&cmd).clicked() {
                    msg = Some(match crate::sessions::open_in_terminal(&cmd) {
                        Ok(()) => super::notices::Notice::Ack(crate::tr!("Session rouverte dans le Terminal.", "Session reopened in the Terminal.").to_string()),
                        Err(e) => {
                            ui.ctx().copy_text(cmd.clone());
                            super::notices::Notice::Error(crate::trf!("Terminal indisponible ({e}) : commande copiée.", "Terminal unavailable ({e}): command copied."))
                        }
                    });
                }
                if ui.button(crate::tr!("Copier la commande", "Copy the command")).on_hover_text(&cmd).clicked() {
                    ui.ctx().copy_text(cmd.clone());
                    msg = Some(super::notices::Notice::Ack(crate::tr!("Commande copiée.", "Command copied.").into()));
                }
            });
        });
    }
    ui.add_space(2.0);
    msg
}

/// The panel at the top of a project's page: header, then the three cards, side by
/// side when the window is wide enough (one, two or three columns).
/// Everything the project page's cards need.
pub(super) struct Cx<'a> {
    pub page_id: &'a str,
    pub title: String,
    pub pr: &'a mut Project,
    pub new_step: &'a mut String,
    pub t: &'a Tone,
    pub live: &'a [SessionInfo],
    pub pages: &'a [PageRow],
    pub actions: &'a mut Vec<Action>,
    pub msg: Option<super::notices::Notice>,
    /// A "Regard extérieur" is being written for this project.
    pub reviewing: bool,
    /// Who else is in this (shared) project right now, and whether their AI is at work.
    pub peers: Vec<(String, bool)>,
    pub vers: &'a mut super::versions::Versions,
    /// "Nouvelle session" buttons: the plugged AIs that run in a terminal, then the shell.
    pub launchers: Vec<(&'static str, Option<&'static str>, Color32)>,
    /// The AI filling the cards, proposing steps.
    pub ai: &'a mut super::ai_fill::AiJobs,
}

#[derive(Clone, Copy)]
enum CardKind {
    Summary(f32),
    Steps,
    Sessions,
    Pages,
    Versions,
}

fn draw(ui: &mut egui::Ui, cx: &mut Cx, k: CardKind, extra: f32) -> f32 {
    // Cards come in one after the other when the page opens.
    let order = match k {
        CardKind::Summary(_) => 0.0,
        CardKind::Steps => 1.0,
        CardKind::Sessions | CardKind::Pages => 2.0,
        CardKind::Versions => 3.0,
    };
    let t0: f64 = ui.ctx().data(|d| d.get_temp(appear_key())).map(|(_, t): (String, f64)| t).unwrap_or(0.0);
    let p = (((ui.input(|i| i.time) - t0) as f32 - order * 0.07) / 0.35).clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - p).powi(3);
    if e < 1.0 {
        ui.ctx().request_repaint();
    }
    ui.scope(|ui| {
        ui.multiply_opacity(e);
        if e < 1.0 {
            ui.add_space(10.0 * (1.0 - e));
        }
        let h = draw_card(ui, cx, k, extra);
        let key = card_key(cx.page_id, k);
        let prev: Option<f32> = ui.ctx().data(|d| d.get_temp(key));
        // Settle on a height (a little hysteresis, so cards don't hop between columns).
        if prev.is_none_or(|p| (p - h).abs() > 24.0) {
            ui.ctx().data_mut(|d| d.insert_temp(key, h));
        }
        h
    })
    .inner
}

fn appear_key() -> egui::Id {
    egui::Id::unique("project-panel-appear")
}

fn draw_card(ui: &mut egui::Ui, cx: &mut Cx, k: CardKind, extra: f32) -> f32 {
    match k {
        CardKind::Summary(h) => summary_card(ui, cx, h, extra),
        CardKind::Steps => steps_card(ui, cx, extra),
        CardKind::Sessions => sessions_card(ui, cx, extra),
        CardKind::Pages => pages_card(ui, cx, extra),
        CardKind::Versions => {
            let folder = cx.pr.folder.trim().to_string();
            let prefer = cx.live.first().map(|s| s.tool);
            let title = cx.title.clone();
            super::versions::versions_card(ui, cx.vers, cx.page_id, &folder, &title, prefer, cx.t, extra)
        }
    }
}

fn card_key(page_id: &str, k: CardKind) -> egui::Id {
    let name = match k {
        CardKind::Summary(_) => "summary",
        CardKind::Steps => "steps",
        CardKind::Sessions => "sessions",
        CardKind::Pages => "pages",
        CardKind::Versions => "versions",
    };
    egui::Id::unique(("card-h", page_id, name))
}

/// One card at the top of each column, then each other card under the column that is
/// shortest so far (heights from the last frame), so no column stays half empty.
fn balance<const N: usize>(ui: &egui::Ui, page_id: &str, heads: [CardKind; N], rest: &[CardKind]) -> Vec<Vec<CardKind>> {
    let h = |k: CardKind| -> f32 { ui.ctx().data(|d| d.get_temp(card_key(page_id, k))).unwrap_or(200.0) };
    let mut cols: Vec<Vec<CardKind>> = heads.iter().map(|k| vec![*k]).collect();
    let mut tot: Vec<f32> = heads.iter().map(|k| h(*k)).collect();
    for k in rest {
        let i = (0..N).min_by(|a, b| tot[*a].total_cmp(&tot[*b])).unwrap_or(0);
        cols[i].push(*k);
        tot[i] += h(*k) + 16.0;
    }
    cols
}

/// Columns of cards whose bottoms line up: the last card of each column is stretched to
/// the tallest column (measured on the previous frame).
fn columns(ui: &mut egui::Ui, cx: &mut Cx, cols: &[Vec<CardKind>], gap: f32) {
    let key = egui::Id::unique(("col-heights", cx.page_id, cols.len()));
    let prev: Vec<f32> = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_default();
    let target = prev.iter().cloned().fold(0.0, f32::max);
    let w = ui.available_width();
    let cw = (w - gap * (cols.len() as f32 - 1.0)) / cols.len() as f32;
    let mut now = vec![0.0; cols.len()];
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (ci, col) in cols.iter().enumerate() {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.set_width(cw);
                let mut total = 0.0;
                for (i, k) in col.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(gap);
                        total += gap;
                    }
                    let last = i + 1 == col.len();
                    let extra = if last && cols.len() > 1 { (target - prev.get(ci).copied().unwrap_or(target)).max(0.0) } else { 0.0 };
                    total += draw(ui, cx, *k, extra);
                }
                now[ci] = total;
            });
        }
    });
    if now.iter().zip(prev.iter().chain(std::iter::repeat(&-1.0))).any(|(a, b)| (a - b).abs() > 0.5) {
        if std::env::var_os("ORCHESTRA_PROFILE").is_some() {
            eprintln!("colonnes : {prev:?} -> {now:?}");
        }
        ui.ctx().data_mut(|d| d.insert_temp(key, now));
        ui.ctx().request_repaint();
    }
}

/// The panel at the top of a project's page: header, then the cards, side by side when
/// the window is wide enough (one, two or three columns), bottoms lined up.
pub(super) fn project_panel(ui: &mut egui::Ui, cx: &mut Cx) -> Option<super::notices::Notice> {
    ui.spacing_mut().item_spacing.y = 6.0;
    let shown: Option<(String, f64)> = ui.ctx().data(|d| d.get_temp(appear_key()));
    if shown.as_ref().is_none_or(|(id, _)| id != cx.page_id) {
        let now_t = ui.input(|i| i.time);
        ui.ctx().data_mut(|d| d.insert_temp(appear_key(), (cx.page_id.to_string(), now_t)));
    }
    let mut cx_copy = false;
    {
        let (page_id, t, live) = (cx.page_id, cx.t, cx.live);
        let peers = cx.peers.clone();
        let filling = cx.ai.filling(page_id);
        let prefer = live.first().map(|s| s.tool);
        let pr = &mut *cx.pr;
        // ---- Header: status, progress, activity, tokens; folder below, small.
        let (done, total) = pr.progress();
        let (last, _, tokens) = activity(pr, 0, live);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.add_enabled_ui(!pr.read_only, |ui| {
                egui::ComboBox::from_id_salt(("project-status", page_id))
                    .selected_text(RichText::new(pr.status.label()).color(status_color(pr.status, t)))
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for st in ProjectStatus::ALL {
                            ui.selectable_value(&mut pr.status, st, st.label());
                        }
                    });
            });
            let (bar, _) = ui.allocate_exact_size(egui::vec2(120.0, 6.0), egui::Sense::hover());
            ui.painter().rect_filled(bar, 3.0, t.fill);
            let frac = smooth(ui, ("progress", page_id), if total > 0 { done as f32 / total as f32 } else { 0.0 });
            if frac > 0.001 {
                ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac, 6.0)), 3.0, GREEN);
            }
            ui.label(RichText::new(crate::trf!("{done}/{total} étapes", "{done}/{total} steps")).color(t.secondary));
            ui.label(RichText::new(format!("· {}", relative_time(last))).color(t.tertiary));
            if tokens > 0 {
                ui.label(RichText::new(crate::trf!("· {} tokens", "· {} tokens", human_tokens(tokens))).color(t.tertiary))
                    .on_hover_text(crate::tr!("Total des sessions Claude Code / Codex de ce projet (hors lectures de cache)", "Total of this project's Claude Code / Codex sessions (cache reads excluded)"));
            }
            if live.iter().any(|s| s.is_live(now())) {
                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 14.0), egui::Sense::hover());
                pulse_dot(ui, r.center(), GREEN);
                ui.label(RichText::new(crate::tr!("IA au travail", "AI at work")).color(GREEN));
            }
            // The team, live: who is in the project, and whose AI is working.
            if !peers.is_empty() {
                ui.label(RichText::new("·").color(t.tertiary));
                let names: Vec<String> = peers.iter().map(|(n, _)| n.clone()).collect();
                super::live_ui::faces(ui, &names, t.card);
                let working: Vec<&str> = peers.iter().filter(|(_, ai)| *ai).map(|(n, _)| n.as_str()).collect();
                let text = if working.is_empty() { crate::trf!("{} ici", "{} here", names.join(", ")) } else { crate::trf!("IA au travail chez {}", "AI at work at {}", working.join(", ")) };
                ui.label(RichText::new(text).size(12.5).color(if working.is_empty() { t.secondary } else { GREEN }));
            }
            // Fill (again) the cards with the AI: after a warning, since it replaces them.
            if filling {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new(crate::tr!("L'IA remplit le projet… (1 à 3 min)", "The AI is filling in the project… (1 to 3 min)")).size(12.5).color(BLUE));
            } else if !pr.read_only {
                let folder = pr.folder.trim().to_string();
                let ok = !folder.is_empty() && crate::project::normalize_folder(&folder).is_dir();
                let r = ui.add_enabled_ui(ok, |ui| action_link(ui, crate::tr!("Remplir avec l'IA", "Fill in with the AI"))).inner;
                let r = r.on_disabled_hover_text(crate::tr!("Indiquez d'abord le dossier du projet", "Set the project's folder first"));
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_max_width(300.0);
                    ui.label(RichText::new(crate::tr!("Tout faire remplir par l'IA ?", "Have the AI fill everything in?")).font(semibold(14.0)).color(t.text));
                    ui.add_space(2.0);
                    let has = !pr.summary.trim().is_empty() || pr.review.is_some();
                    ui.add(
                        egui::Label::new(
                            RichText::new(if has {
                                crate::tr!("L'état des lieux, le statut et le regard extérieur actuels seront remplacés. Les étapes ne sont pas touchées.", "The current state, status and outside view will be replaced. The steps aren't touched.")
                            } else {
                                crate::tr!("L'IA lit le projet sans rien modifier et écrit l'état des lieux, le statut et un regard extérieur. Les étapes ne sont pas touchées.", "The AI reads the project without changing anything and writes the current state, the status and an outside view. The steps aren't touched.")
                            })
                            .size(12.5)
                            .color(if has { ORANGE } else { t.secondary }),
                        )
                        .wrap(),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(if has { crate::tr!("Remplacer", "Replace") } else { crate::tr!("Remplir", "Fill in") }).clicked() {
                            super::ai_fill::request(ui.ctx(), super::ai_fill::AiReq::Fill { page_id: page_id.to_string(), folder: folder.clone(), prefer });
                            ui.close();
                        }
                        if ui.button(crate::tr!("Annuler", "Cancel")).clicked() {
                            ui.close();
                        }
                    });
                });
            }
        });
        let editing_folder = !pr.read_only && (flag(ui, ("edit-folder", page_id)) || pr.folder.trim().is_empty());
        ui.horizontal(|ui| {
            // Lecture seule, on the right of the folder line.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Privé / Partagé, with the local copy.
                let (label, color) = if pr.shared { (crate::tr!("👥 Partagé ▾", "👥 Shared ▾"), Color32::from_rgb(35, 131, 226)) } else { (crate::tr!("Privé ▾", "Private ▾"), t.tertiary) };
                let btn = ui.add(egui::Button::new(RichText::new(label).size(12.5).color(color)).frame(false)).on_hover_text(
                    crate::tr!("Seuls les projets partagés quittent cette machine (collaboration). Les autres restent privés.", "Only shared projects leave this computer (collaboration). The others stay private."),
                );
                egui::Popup::menu(&btn).show(|ui| {
                    ui.set_min_width(260.0);
                    if pr.shared {
                        if ui.button(crate::tr!("Rendre privé", "Make private")).on_hover_text(crate::tr!("Le projet ne part plus chez les autres ; ils gardent leur copie.", "The project no longer goes to the others; they keep their copy.")).clicked() {
                            pr.shared = false;
                            ui.close();
                        }
                        if ui.button(crate::tr!("Garder une copie locale", "Keep a local copy")).on_hover_text(crate::tr!("Une copie privée qui ne suit plus le projet partagé.", "A private copy that no longer follows the shared project.")).clicked() {
                            cx_copy = true;
                            ui.close();
                        }
                    } else if ui.button(crate::tr!("Partager avec l'équipe", "Share with the team")).on_hover_text(crate::tr!("Le projet, ses pages et sa discussion partent par la collaboration branchée.", "The project, its pages and its discussion go through the collaboration you set up.")).clicked() {
                        pr.shared = true;
                        ui.close();
                    }
                });
                ui.add_space(10.0);
                let mut ro = pr.read_only;
                let ro_color = if ro { ORANGE } else { t.tertiary };
                if ui
                    .checkbox(&mut ro, RichText::new(crate::tr!("Lecture seule", "Read-only")).size(12.5).color(ro_color))
                    .on_hover_text(crate::tr!("Empêche de modifier ce projet et ses pages par erreur. Les IA peuvent toujours mettre à jour l'état des lieux et les étapes.", "Prevents editing this project and its pages by mistake. The AIs can still update the current state and the steps."))
                    .changed()
                {
                    pr.read_only = ro;
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if editing_folder {
                let r = ui.add(egui::TextEdit::singleline(&mut pr.folder).hint_text(crate::tr!("Dossier du projet, ex. ~/Code/mon-projet", "Project folder, e.g. ~/Code/my-project")).font(FontId::monospace(12.5)).desired_width(f32::INFINITY));
                if r.lost_focus() && !pr.folder.trim().is_empty() && flag(ui, ("edit-folder", page_id)) {
                    toggle_flag(ui, ("edit-folder", page_id));
                }
            } else {
                let r = ui.add(egui::Label::new(RichText::new(&pr.folder).font(FontId::monospace(12.5)).color(t.tertiary)).sense(egui::Sense::click()));
                if r.on_hover_text(if pr.read_only { crate::tr!("Lecture seule", "Read-only") } else { crate::tr!("Cliquer pour modifier le dossier", "Click to change the folder") }).clicked() && !pr.read_only {
                    toggle_flag(ui, ("edit-folder", page_id));
                }
            }
                });
            });
        });
        ui.add_space(12.0);

    }
    if cx_copy {
        cx.actions.push(Action::LocalCopy(cx.page_id.to_string()));
    }
    let w = ui.available_width();
    let gap = 16.0;
    use CardKind::*;
    if w >= 1380.0 {
        let cols = balance(ui, cx.page_id, [Summary(420.0), Steps, Sessions], &[Versions, Pages]);
        columns(ui, cx, &cols, gap);
    } else if w >= 860.0 {
        let cols = balance(ui, cx.page_id, [Summary(380.0), Steps], &[Sessions, Versions, Pages]);
        columns(ui, cx, &cols, gap);
    } else {
        columns(ui, cx, &[vec![Summary(210.0), Steps, Sessions, Versions, Pages]], gap);
    }
    ui.add_space(gap);
    review_card(ui, cx);
    cx.msg.take()
}

fn summary_card(ui: &mut egui::Ui, cx: &mut Cx, folded_h: f32, extra: f32) -> f32 {
    let (page_id, pr, t) = (cx.page_id, &*cx.pr, cx.t);
    // ---- State of play: folded to ~8 lines unless unfolded.
    card(ui, t, extra, |ui| {
        let by = if pr.summary_by.is_empty() { String::new() } else { format!("{} · {}", pr.summary_by, relative_time(pr.summary_updated)) };
        card_title(ui, crate::tr!("ÉTAT DES LIEUX", "CURRENT STATE"), &by, t);
        if cx.ai.filling(page_id) {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new(crate::tr!("L'IA lit le projet et écrit l'état des lieux…", "The AI is reading the project and writing its state…")).size(13.0).color(t.secondary));
            });
            ui.add_space(4.0);
        }
        if pr.summary.trim().is_empty() {
            if !cx.ai.filling(page_id) {
                ui.label(RichText::new(crate::tr!("Pas encore écrit. Cliquez sur « Remplir avec l'IA » en haut, ou demandez à votre IA : « fais l'état des lieux de ce projet dans le tableau de bord ».", "Not written yet. Click « Fill in with the AI » at the top, or ask your AI: « write the current state of this project in the dashboard ».")).color(t.tertiary).italics());
            }
            return;
        }
        let key = ("summary-open", page_id);
        let open = flag(ui, key);
        // Drawn in a clipped child, not a scroll area: the mouse wheel keeps scrolling
        // the page, wherever the pointer is.
        let w = ui.available_width();
        let top = ui.cursor().min;
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_size(top, egui::vec2(w, f32::INFINITY)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        if !open {
            child.set_clip_rect(egui::Rect::from_min_size(top, egui::vec2(w, folded_h)).intersect(ui.clip_rect()));
        }
        child.set_max_width(760.0f32.min(w));
        super::rich::markdown(&mut child, &pr.summary, TEXT, t);
        let content_h = child.min_rect().height();
        let overflows = content_h > folded_h + 4.0;
        let shown = if open || !overflows { content_h } else { folded_h };
        let shown_rect = egui::Rect::from_min_size(top, egui::vec2(w, shown));
        ui.allocate_rect(shown_rect, egui::Sense::hover());
        if overflows && !open {
            // Fade the last lines into the card.
            let fade = egui::Rect::from_min_max(egui::pos2(shown_rect.left(), shown_rect.bottom() - 48.0), shown_rect.max);
            let mut mesh = egui::Mesh::default();
            let clear = t.card.gamma_multiply(0.0);
            mesh.colored_vertex(fade.left_top(), clear);
            mesh.colored_vertex(fade.right_top(), clear);
            mesh.colored_vertex(fade.right_bottom(), t.card);
            mesh.colored_vertex(fade.left_bottom(), t.card);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            ui.painter().add(mesh);
        }
        if overflows && link(ui, if open { crate::tr!("Réduire ▴", "Collapse ▴") } else { crate::tr!("Afficher tout ▾", "Show all ▾") }) {
            toggle_flag(ui, key);
        }
    })
}

fn steps_card(ui: &mut egui::Ui, cx: &mut Cx, extra: f32) -> f32 {
    let (page_id, t, ro) = (cx.page_id, cx.t, cx.pr.read_only);
    let pr = &mut *cx.pr;
    let new_step = &mut *cx.new_step;
    let ai = &mut *cx.ai;
    let prefer = cx.live.first().map(|s| s.tool);
    // ---- Steps: open ones first, one line each; done ones folded.
    let (done, total) = pr.progress();
    card(ui, t, extra, |ui| {
        card_title(ui, crate::tr!("ÉTAPES", "STEPS"), &format!("{done} / {total}"), t);
        let mut cycle = None;
        let mut remove = None;
        let show_done = flag(ui, ("done-open", page_id));
        let rows: Vec<usize> = (0..pr.steps.len())
            .filter(|&i| pr.steps[i].status != StepStatus::Done)
            .chain((0..pr.steps.len()).filter(|&i| show_done && pr.steps[i].status == StepStatus::Done))
            .collect();
        if pr.steps.is_empty() {
            ui.label(RichText::new(crate::tr!("Aucune étape. Ajoutez-en une, demandez des suggestions à l'IA, ou collez un texte à transformer en étapes.", "No step. Add one, ask the AI for suggestions, or paste a text to turn into steps.")).color(t.tertiary).italics());
        }
        for i in rows {
            let s = &pr.steps[i];
            let open_key = ("step-open", s.id.as_str());
            let open = flag(ui, open_key);
            let (area, text) = split_area(&s.title);
            let color = if s.status == StepStatus::Done { t.tertiary } else { t.text };
            let bg = hover_begin(ui);
            let row = ui.horizontal_top(|ui| {
                if step_icon(ui, &s.id, s.status, t).clicked() && !ro {
                    cycle = Some(s.id.clone());
                }
                if let Some(a) = area {
                    chip(ui, a, t.secondary, egui::Sense::hover());
                }
                let room = ui.available_width() - 26.0;
                let mut job = super::rich::inline_job(text, TEXT, color, false, t);
                if s.status == StepStatus::Done {
                    for sec in job.sections.iter_mut() {
                        sec.format.strikethrough = egui::Stroke::new(1.0, color);
                    }
                }
                let resp = if open {
                    ui.vertical(|ui| {
                        ui.set_max_width(room);
                        let r = ui.add(egui::Label::new(job).wrap().sense(egui::Sense::click()));
                        if !s.source.is_empty() {
                            ui.label(RichText::new(crate::trf!("ajoutée par {} · {}", "added by {} · {}", s.source, relative_time(s.created))).size(12.0).color(t.tertiary));
                        }
                        r
                    })
                    .inner
                } else {
                    line(ui, job, room)
                };
                let hovered = ui.rect_contains_pointer(ui.min_rect()) && !ro;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    let x = egui::Button::new(RichText::new("×").color(if hovered { t.secondary } else { t.card })).frame(false);
                    if ui.add(x).on_hover_text(crate::tr!("Supprimer l'étape", "Delete the step")).clicked() {
                        remove = Some(s.id.clone());
                    }
                });
                resp
            });
            hover_end(ui, bg, row.response.rect, egui::Id::unique(("step-row", &s.id)), t);
            if row.inner.clicked() {
                toggle_flag(ui, open_key);
            }
            ui.add_space(2.0);
        }
        if done > 0 {
            // "Supprimer les terminées": a second click confirms, nothing goes on one slip.
            let confirm_key = ("clear-done", page_id);
            let confirming = flag(ui, confirm_key);
            let mut clear = false;
            ui.horizontal(|ui| {
                if link(ui, &if show_done { crate::tr!("Masquer les étapes terminées", "Hide finished steps").to_string() } else { if done == 1 { crate::tr!("Voir l'étape terminée", "Show the finished step").to_string() } else { crate::trf!("Voir les {done} étapes terminées", "Show the {done} finished steps") } }) {
                    toggle_flag(ui, ("done-open", page_id));
                }
                if ro {
                    return;
                }
                ui.label(RichText::new("·").color(t.tertiary));
                if confirming {
                    let sure = if done == 1 { crate::tr!("Confirmer : supprimer 1 étape terminée", "Confirm: delete 1 finished step").to_string() } else { crate::trf!("Confirmer : supprimer {done} étapes terminées", "Confirm: delete {done} finished steps") };
                    if ui.add(egui::Label::new(RichText::new(sure).size(13.0).color(Color32::from_rgb(224, 62, 62))).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        clear = true;
                        toggle_flag(ui, confirm_key);
                    }
                    if link(ui, crate::tr!("Annuler", "Cancel")) {
                        toggle_flag(ui, confirm_key);
                    }
                } else if link(ui, crate::tr!("Supprimer les terminées", "Delete finished ones")) {
                    toggle_flag(ui, confirm_key);
                }
            });
            if clear {
                pr.remove_done_steps();
            }
        }
        if let Some(id) = cycle {
            let cur = pr.steps.iter().find(|s| s.id == id).map(|s| s.status).unwrap_or_default();
            let next = match cur {
                StepStatus::Todo => StepStatus::Doing,
                StepStatus::Doing => StepStatus::Done,
                StepStatus::Done => StepStatus::Todo,
            };
            pr.set_step_status(&id, next);
        }
        if let Some(id) = remove {
            pr.steps.retain(|s| s.id != id);
            pr.touch();
        }
        // "+ Ajouter une étape": a quiet line aligned with the steps; click to type.
        if ro {
            return;
        }
        ui.add_space(2.0);
        let adding_key = ("adding-step", page_id);
        let adding = flag(ui, adding_key);
        ui.horizontal(|ui| {
            let (r, plus) = ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::click());
            let hover_row = ui.rect_contains_pointer(egui::Rect::from_min_size(r.min, egui::vec2(ui.available_width() + 18.0, 22.0)));
            let col = if adding || hover_row { t.secondary } else { t.tertiary };
            let c = r.center();
            ui.painter().line_segment([c - egui::vec2(5.0, 0.0), c + egui::vec2(5.0, 0.0)], egui::Stroke::new(1.5, col));
            ui.painter().line_segment([c - egui::vec2(0.0, 5.0), c + egui::vec2(0.0, 5.0)], egui::Stroke::new(1.5, col));
            if adding {
                let edit = ui.add(
                    egui::TextEdit::singleline(new_step)
                        .frame(egui::Frame::NONE)
                        .hint_text(crate::tr!("Nouvelle étape, puis Entrée", "New step, then Enter"))
                        .font(FontId::proportional(TEXT))
                        .desired_width(f32::INFINITY),
                );
                if !edit.has_focus() && !edit.lost_focus() && new_step.is_empty() && !ui.ctx().memory(|m| m.has_focus(edit.id)) {
                    edit.request_focus();
                }
                let enter = ui.input(|i| i.key_pressed(Key::Enter));
                let esc = ui.input(|i| i.key_pressed(Key::Escape));
                if edit.lost_focus() && enter && !new_step.trim().is_empty() {
                    pr.add_step(new_step, "moi");
                    new_step.clear();
                    edit.request_focus();
                } else if esc || (edit.lost_focus() && new_step.trim().is_empty()) {
                    new_step.clear();
                    toggle_flag(ui, adding_key);
                }
            } else {
                let lbl = ui.add(egui::Label::new(RichText::new(crate::tr!("Ajouter une étape", "Add a step")).size(TEXT).color(col)).sense(egui::Sense::click()));
                if lbl.clicked() || plus.clicked() {
                    toggle_flag(ui, adding_key);
                }
            }
        });
        ai_steps(ui, page_id, pr, ai, prefer, t);
    })
}

/// Under the steps: ask the AI for steps (from the project or from a pasted text), and
/// the proposals to tick before they become steps. Nothing is added without a click.
fn ai_steps(ui: &mut egui::Ui, page_id: &str, pr: &mut Project, ai: &mut super::ai_fill::AiJobs, prefer: Option<crate::sessions::Tool>, t: &Tone) {
    let folder = pr.folder.trim().to_string();
    let folder_ok = !folder.is_empty() && crate::project::normalize_folder(&folder).is_dir();
    let existing: Vec<String> = pr.steps.iter().filter(|s| s.status != StepStatus::Done).map(|s| s.title.clone()).collect();
    ui.add_space(6.0);
    if let Some(list) = ai.proposals.get_mut(page_id) {
        let mut add = false;
        let mut drop = false;
        egui::Frame::new().fill(BLUE.gamma_multiply(0.05)).stroke(egui::Stroke::new(1.0, BLUE.gamma_multiply(0.25))).corner_radius(8.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(crate::tr!("Proposées par l'IA — cochez celles à garder", "Proposed by the AI — tick the ones to keep")).size(12.5).color(BLUE));
            ui.add_space(2.0);
            for (title, keep) in list.iter_mut() {
                ui.horizontal(|ui| {
                    ui.checkbox(keep, "");
                    let room = ui.available_width();
                    ui.allocate_ui_with_layout(egui::vec2(room, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(room);
                        ui.add(egui::Label::new(RichText::new(title.as_str()).size(13.5).color(if *keep { t.text } else { t.tertiary })).wrap());
                    });
                });
            }
            ui.add_space(4.0);
            let n = list.iter().filter(|(_, k)| *k).count();
            ui.horizontal(|ui| {
                if ui.add_enabled(n > 0, egui::Button::new(RichText::new(crate::trf!("Ajouter ({n})", "Add ({n})")).size(13.0))).clicked() {
                    add = true;
                }
                if ui.add(egui::Button::new(RichText::new(crate::tr!("Ignorer", "Dismiss")).size(13.0).color(t.tertiary)).frame(false)).clicked() {
                    drop = true;
                }
            });
        });
        if add {
            for (title, keep) in list.iter() {
                if *keep {
                    pr.add_step(title, "IA");
                }
            }
        }
        if add || drop {
            ai.proposals.remove(page_id);
        }
        return;
    }
    if ai.tasking(page_id) {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0));
            ui.label(RichText::new(crate::tr!("L'IA prépare des étapes…", "The AI is preparing steps…")).size(13.0).color(t.secondary));
        });
        return;
    }
    let paste_key = ("paste-open", page_id);
    if flag(ui, paste_key) {
        let text_key = egui::Id::unique(("paste-text", page_id));
        let mut text: String = ui.ctx().data(|d| d.get_temp(text_key)).unwrap_or_default();
        ui.add(
            egui::TextEdit::multiline(&mut text)
                .hint_text(crate::tr!("Collez des notes, un mail, une liste en vrac… l'IA en fait des étapes claires.", "Paste notes, an email, a rough list… the AI turns it into clear steps."))
                .desired_rows(5)
                .desired_width(f32::INFINITY)
                .font(FontId::proportional(13.0)),
        );
        // Joined images (screenshots…): pasted from the clipboard or dropped on the window.
        let img_key = egui::Id::unique(("paste-images", page_id));
        let err_key = egui::Id::unique(("paste-image-error", page_id));
        let mut images: Vec<String> = ui.ctx().data(|d| d.get_temp(img_key)).unwrap_or_default();
        let mut err: Option<String> = ui.ctx().data(|d| d.get_temp(err_key)).unwrap_or_default();
        let dropped: Vec<String> = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| crate::sync::is_image(p)).map(|p| p.display().to_string()).collect());
        for p in dropped {
            if !images.contains(&p) {
                images.push(p);
                err = None;
            }
        }
        let mut remove = None;
        for (i, p) in images.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
                ui.label(RichText::new(crate::trf!("Image : {name}", "Image: {name}")).size(12.5).color(t.secondary)).on_hover_text(p.as_str());
                if ui.add(egui::Button::new(RichText::new("×").size(13.0).color(t.tertiary)).frame(false)).on_hover_text(crate::tr!("Retirer l'image", "Remove the image")).clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            images.remove(i);
        }
        if let Some(e) = &err {
            ui.label(RichText::new(e.as_str()).size(12.5).color(t.tertiary));
        }
        let mut close = false;
        ui.horizontal(|ui| {
            let ready = !text.trim().is_empty() || !images.is_empty();
            if ui.add_enabled(ready, egui::Button::new(RichText::new(crate::tr!("Transformer en étapes", "Turn into steps")).size(13.0))).clicked() {
                super::ai_fill::request(ui.ctx(), super::ai_fill::AiReq::Tasks { page_id: page_id.to_string(), folder: folder.clone(), existing: existing.clone(), prefer, text: Some(text.clone()), images: images.clone() });
                close = true;
            }
            let paste = ui
                .add(egui::Button::new(RichText::new(crate::tr!("Coller une image", "Paste an image")).size(13.0)))
                .on_hover_text(crate::tr!("Joint l'image copiée (une capture d'écran…) ; vous pouvez aussi glisser des images sur la fenêtre", "Joins the copied image (a screenshot…); you can also drop images on the window"));
            if paste.clicked() {
                match crate::clip::paste_image() {
                    Ok(p) => {
                        images.push(p.display().to_string());
                        err = None;
                    }
                    Err(e) => err = Some(crate::trf!("Pas d'image : {e}", "No image: {e}")),
                }
            }
            if ui.add(egui::Button::new(RichText::new(crate::tr!("Annuler", "Cancel")).size(13.0).color(t.tertiary)).frame(false)).clicked() {
                close = true;
            }
        });
        if close {
            text.clear();
            images.clear();
            err = None;
            toggle_flag(ui, paste_key);
        }
        ui.ctx().data_mut(|d| {
            d.insert_temp(text_key, text);
            d.insert_temp(img_key, images);
            d.insert_temp(err_key, err);
        });
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let r = ui.add_enabled_ui(folder_ok, |ui| action_link(ui, crate::tr!("Suggérer des étapes", "Suggest steps"))).inner;
        let r = r
            .on_hover_text(crate::tr!("L'IA lit le code sans rien modifier et propose des étapes précises (fonctionnalité, technique, sécurité), faites pour être menées par une IA en autonomie ; vous choisissez celles à garder.", "The AI reads the code without changing anything and proposes precise steps (feature, technical, security), made to be carried out by an AI on its own; you choose which to keep."))
            .on_disabled_hover_text(crate::tr!("Indiquez d'abord le dossier du projet", "Set the project's folder first"));
        if r.clicked() && folder_ok {
            super::ai_fill::request(ui.ctx(), super::ai_fill::AiReq::Tasks { page_id: page_id.to_string(), folder: folder.clone(), existing: existing.clone(), prefer, text: None, images: vec![] });
        }
        if action_link(ui, crate::tr!("Depuis un texte", "From a text")).on_hover_text(crate::tr!("Collez un texte, joignez des images : l'IA en fait des étapes", "Paste a text, join images: the AI turns them into steps")).clicked() {
            toggle_flag(ui, paste_key);
        }
    });
}

fn sessions_card(ui: &mut egui::Ui, cx: &mut Cx, extra: f32) -> f32 {
    let (page_id, pr, t, live) = (cx.page_id, &*cx.pr, cx.t, cx.live);
    // ---- Sessions: live ones read from disk, plus those an AI linked itself.
    let mut msg = None;
    let mut clear = false;
    let h = card(ui, t, extra, |ui| {
        let others: Vec<&crate::project::SessionRef> = pr.sessions.iter().filter(|x| !live.iter().any(|l| l.id == x.id)).collect();
        let n = live.len() + others.len();
        card_title(ui, crate::tr!("SESSIONS IA", "AI SESSIONS"), &if n > 0 { n.to_string() } else { String::new() }, t);
        if !pr.folder.trim().is_empty() {
            // Start something new in the project's folder, in the built-in terminal.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new(crate::tr!("Nouvelle session", "New session")).size(12.5).color(t.tertiary));
                let name = std::path::Path::new(pr.folder.trim()).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                for (label, cmd, accent) in cx.launchers.clone() {
                    let accent = if cmd.is_none() { t.secondary } else { accent };
                    if launch_pill(ui, label, accent, t).on_hover_text(crate::trf!("Ouvrir {label} dans {name}, dans le terminal intégré", "Open {label} in {name}, in the built-in terminal")).clicked() {
                        super::term_ui::request(ui.ctx(), &format!("{label} · {name}"), pr.folder.trim(), cmd.map(String::from));
                    }
                }
            });
            ui.add_space(8.0);
        }
        if n == 0 {
            let hint = if pr.folder.trim().is_empty() {
                crate::tr!("Indiquez le dossier du projet : ses sessions Claude Code et Codex apparaîtront ici toutes seules.", "Set the project's folder: its Claude Code and Codex sessions will show up here by themselves.")
            } else {
                crate::tr!("Aucune session Claude Code ou Codex trouvée dans ce dossier pour l'instant.", "No Claude Code or Codex session found in this folder yet.")
            };
            ui.label(RichText::new(hint).color(t.tertiary).italics());
        }
        let all = flag(ui, ("all-sessions", page_id));
        for s in live.iter().take(if all { usize::MAX } else { 4 }) {
            if let Some(m) = session_row(ui, page_id, s, t) {
                msg = Some(m);
            }
        }
        for s in others.iter().take(if all { usize::MAX } else { 3 }) {
            // Same columns as the rows above: a (hollow) dot, the tool, the title, the time.
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 20.0), egui::Sense::hover());
                ui.painter().circle_stroke(r.center(), 3.5, egui::Stroke::new(1.2, t.border));
                tool_chip(ui, &tool_label(&s.tool), t);
                let when = relative_time(s.last_activity);
                let room = ui.available_width() - ui.painter().layout_no_wrap(when.clone(), FontId::proportional(12.5), t.tertiary).size().x - 16.0;
                let title = if s.title.is_empty() { s.id.clone() } else { s.title.clone() };
                let job = egui::text::LayoutJob::single_section(title, egui::TextFormat::simple(FontId::proportional(TEXT), t.text));
                line(ui, job, room).on_hover_text(if s.location.is_empty() { &s.id } else { &s.location });
                ui.label(RichText::new(when).size(12.5).color(t.tertiary));
            });
            ui.add_space(2.0);
        }
        if live.len() > 4 || others.len() > 3 {
            if link(ui, &if all { crate::tr!("Voir moins", "Show less").to_string() } else { crate::trf!("Voir les {n} sessions", "Show the {n} sessions") }) {
                toggle_flag(ui, ("all-sessions", page_id));
            }
        } else if n > 0 {
            ui.label(RichText::new(crate::tr!("Cliquer sur une session pour la reprendre.", "Click a session to resume it.")).size(12.0).color(t.tertiary));
        }
        // "Effacer l'historique": a second click confirms. The sessions' files stay on disk.
        if n > 0 && !pr.read_only {
            let key = ("clear-sessions", page_id);
            ui.add_space(4.0);
            if flag(ui, key) {
                ui.horizontal_wrapped(|ui| {
                    let sure = crate::tr!("Confirmer : effacer l'historique", "Confirm: clear the history");
                    if ui
                        .add(egui::Label::new(RichText::new(sure).size(13.0).color(Color32::from_rgb(224, 62, 62))).sense(egui::Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        clear = true;
                        toggle_flag(ui, key);
                    }
                    if link(ui, crate::tr!("Annuler", "Cancel")) {
                        toggle_flag(ui, key);
                    }
                });
                ui.label(RichText::new(crate::tr!("Les sessions disparaissent de ce projet ; leurs fichiers restent sur le disque et une session reprise réapparaît.", "The sessions leave this project; their files stay on disk and a resumed session shows up again.")).size(12.0).color(t.tertiary));
            } else if action_link(ui, crate::tr!("Effacer l'historique", "Clear the history"))
                .on_hover_text(crate::tr!("Retire les sessions passées de ce projet, sans supprimer leurs fichiers", "Removes the past sessions from this project, without deleting their files"))
                .clicked()
            {
                toggle_flag(ui, key);
            }
        }
    });
    if msg.is_some() {
        cx.msg = msg;
    }
    if clear {
        cx.pr.clear_sessions();
    }
    h
}

// ---------------------------------------------------------------- synchronisation with the AIs

impl NotesApp {
    /// Folders to offer: every repository where Claude Code / Codex worked (last 60 days)
    /// and every project with a folder. Recent ones (30 days) are ticked, at most 8.
    pub(super) fn sync_candidates(&self) -> Vec<SyncCandidate> {
        use crate::sessions::Tool;
        let mut out: Vec<SyncCandidate> = vec![];
        for s in self.agents.all().iter() {
            let root = crate::sync::repo_root(&s.cwd);
            match out.iter_mut().find(|c| c.folder == root) {
                Some(c) => {
                    if s.last_activity > c.last {
                        c.last = s.last_activity;
                        c.tool = s.tool;
                    }
                }
                None => out.push(SyncCandidate {
                    name: std::path::Path::new(&root).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| root.clone()),
                    folder: root,
                    tool: s.tool,
                    last: s.last_activity,
                    is_project: false,
                    checked: false,
                }),
            }
        }
        for p in self.ws.projects() {
            let pr = p.project.as_ref().unwrap();
            if pr.folder.trim().is_empty() {
                continue;
            }
            let f = crate::project::normalize_folder(&pr.folder).display().to_string();
            match out.iter_mut().find(|c| c.folder == f) {
                Some(c) => {
                    c.is_project = true;
                    c.name = p.display_title();
                }
                None => out.push(SyncCandidate { folder: f, name: p.display_title(), tool: Tool::ClaudeCode, last: pr.last_activity, is_project: true, checked: false }),
            }
        }
        for c in out.iter_mut() {
            if let Some(p) = self.ws.project_for_path(std::path::Path::new(&c.folder)) {
                c.is_project = true;
                c.name = p.display_title();
            }
        }
        out.sort_by(|a, b| b.last.cmp(&a.last));
        let month_ago = now().saturating_sub(30 * 24 * 3600 * 1000);
        for c in out.iter_mut().take(8) {
            c.checked = c.last >= month_ago;
        }
        out
    }

    /// The choice of folders, then the progress of the run.
    fn sync_view(&mut self, ui: &mut egui::Ui, width: f32, t: &Tone) {
        if self.sync_dialog.is_none() && self.sync_run.is_none() {
            return;
        }
        let mut start = None;
        let mut close = false;
        egui::Frame::new().fill(t.card).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(12.0).inner_margin(egui::Margin::symmetric(18, 16)).show(ui, |ui| {
            ui.set_width(width - 38.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            if let Some(cands) = self.sync_dialog.as_mut() {
                ui.label(RichText::new(crate::tr!("Synchroniser avec les IA", "Sync with the AIs")).font(semibold(16.0)).color(t.text));
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            crate::tr!("Pour chaque dossier coché, Claude Code (ou Codex) se branche au tableau de bord, lit le projet sans rien modifier, \
                             puis écrit son état des lieux et ses prochaines étapes. Un dossier après l'autre, 1 à 3 minutes chacun, \
                             sur votre abonnement Claude / Codex.", "For each folder ticked, Claude Code (or Codex) plugs into the dashboard, reads the project without changing anything, then writes its current state and next steps. One folder after the other, 1 to 3 minutes each, on your Claude / Codex subscription."),
                        )
                        .size(13.0)
                        .color(t.secondary),
                    )
                    .wrap(),
                );
                ui.add_space(6.0);
                if cands.is_empty() {
                    ui.label(RichText::new(crate::tr!("Aucun dossier trouvé : aucune session Claude Code ou Codex sur cette machine, et aucun projet avec un dossier.", "No folder found: no Claude Code or Codex session on this computer, and no project with a folder.")).color(t.tertiary).italics());
                }
                ui.scope(|ui| {
                    for c in cands.iter_mut().take(25) {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut c.checked, "");
                            ui.label(RichText::new(&c.name).font(semibold(14.0)).color(t.text));
                            if c.is_project {
                                chip(ui, crate::tr!("projet existant", "existing project"), GREEN, egui::Sense::hover());
                            }
                            chip(ui, c.tool.label(), t.secondary, egui::Sense::hover());
                            let room = ui.available_width() - 90.0;
                            let job = egui::text::LayoutJob::single_section(c.folder.clone(), egui::TextFormat::simple(FontId::monospace(11.5), t.tertiary));
                            line(ui, job, room);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if c.last > 0 {
                                    ui.label(RichText::new(relative_time(c.last)).size(12.0).color(t.tertiary));
                                }
                            });
                        });
                    }
                });
                ui.add_space(6.0);
                let n = cands.iter().filter(|c| c.checked).count();
                ui.horizontal(|ui| {
                    if ui.add_enabled(n > 0, egui::Button::new(crate::trf!("Lancer ({n} dossier{})", "Start ({n} folder{})", if n > 1 { "s" } else { "" }))).clicked() {
                        start = Some(cands.iter().filter(|c| c.checked).map(|c| (c.folder.clone(), c.tool)).collect::<Vec<_>>());
                    }
                    if ui.button(crate::tr!("Annuler", "Cancel")).clicked() {
                        close = true;
                    }
                    if link(ui, crate::tr!("Tout cocher", "Tick all")) {
                        let all = cands.iter().all(|c| c.checked);
                        cands.iter_mut().for_each(|c| c.checked = !all);
                    }
                });
            } else if let Some(run) = &self.sync_run {
                let (jobs, notes, finished) = run.snapshot();
                let done = jobs.iter().filter(|j| matches!(j.state, crate::sync::JobState::Done(_))).count();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(crate::tr!("Synchronisation avec les IA", "Syncing with the AIs")).font(semibold(16.0)).color(t.text));
                    ui.label(RichText::new(format!("{done} / {}", jobs.len())).color(t.tertiary));
                    if !finished {
                        ui.add(egui::Spinner::new().size(14.0));
                    }
                });
                for n in &notes {
                    ui.add(egui::Label::new(RichText::new(n).size(13.0).color(ORANGE)).wrap());
                }
                for j in &jobs {
                    ui.horizontal(|ui| {
                        use crate::sync::JobState::*;
                        let (mark, color) = match &j.state {
                            Waiting => ("○", t.tertiary),
                            Running => ("◐", BLUE),
                            Done(_) => ("✔", GREEN),
                            Failed(_) => ("✖", Color32::from_rgb(224, 62, 62)),
                        };
                        ui.label(RichText::new(mark).color(color));
                        let name = std::path::Path::new(&j.folder).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        ui.label(RichText::new(name).color(t.text));
                        chip(ui, j.tool.label(), t.secondary, egui::Sense::hover());
                        let detail = match &j.state {
                            Waiting => crate::tr!("en attente", "waiting").to_string(),
                            Running => crate::tr!("analyse en cours…", "analysing…").to_string(),
                            Done(m) | Failed(m) => m.clone(),
                        };
                        let room = ui.available_width();
                        let job = egui::text::LayoutJob::single_section(detail.clone(), egui::TextFormat::simple(FontId::proportional(12.5), color));
                        line(ui, job, room).on_hover_text(detail);
                    });
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if finished {
                        if ui.button(crate::tr!("Fermer", "Close")).clicked() {
                            close = true;
                        }
                        ui.label(RichText::new(crate::tr!("Les projets se sont mis à jour au fil de l'eau.", "The projects were updated along the way.")).size(12.5).color(t.tertiary));
                    } else {
                        if ui.button(crate::tr!("Arrêter", "Stop")).clicked() {
                            run.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        ui.label(RichText::new(crate::tr!("Vous pouvez continuer à utiliser l'app pendant ce temps.", "You can keep using the app meanwhile.")).size(12.5).color(t.tertiary));
                    }
                });
            }
        });
        ui.add_space(16.0);
        if let Some(wanted) = start {
            let ctx = ui.ctx().clone();
            self.sync_run = Some(crate::sync::SyncRun::start(wanted, move || ctx.request_repaint()));
            self.sync_dialog = None;
        }
        if close {
            self.sync_dialog = None;
            self.sync_run = None;
        }
    }
}

impl NotesApp {
    /// "IA branchées : Claude Code ✔ · Codex ✔", the subscription's use, and a way to manage them.
    fn mcp_line(&mut self, ui: &mut egui::Ui, t: &Tone) {
        let states = self.ais.states.lock().unwrap().clone();
        let report = self.mcp_report.lock().unwrap().clone();
        let mut open_ai = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(crate::tr!("IA branchées :", "AIs plugged in:")).size(12.5).color(t.tertiary));
            match &states {
                None => {
                    ui.add(egui::Spinner::new().size(11.0));
                    ui.label(RichText::new(crate::tr!("recherche des IA…", "looking for AIs…")).size(12.5).color(t.tertiary));
                }
                Some(s) if !s.iter().any(|x| x.plugged) => {
                    ui.label(RichText::new(crate::tr!("aucune pour l'instant", "none yet")).size(12.5).color(t.tertiary));
                }
                Some(s) => {
                    for x in s.iter().filter(|x| x.plugged) {
                        let failed = report.as_ref().and_then(|r| r.iter().find(|(n, _)| n == x.ai.label())).and_then(|(_, o)| if let crate::setup::Outcome::Failed(e) = o { Some(e.clone()) } else { None });
                        match failed {
                            Some(e) => ui.label(RichText::new(format!("{} ✖", x.ai.label())).size(12.5).color(Color32::from_rgb(224, 62, 62))).on_hover_text(e),
                            None => ui.label(RichText::new(format!("{} ✔", x.ai.label())).size(12.5).color(GREEN)),
                        };
                    }
                }
            }
            if link(ui, crate::tr!("Gérer les IA", "Manage AIs")) {
                open_ai = true;
            }
        });
        if open_ai {
            self.open_settings(SettingsTab::Ai);
        }
    }
}

/// The strip above a page that mirrors a project file (README, docs…).
pub(super) fn source_banner(ui: &mut egui::Ui, page_id: &str, src: &crate::docs::PageSource, edited: bool, actions: &mut Vec<Action>) {
    let t = tone(ui.visuals().dark_mode);
    if src.is_dir || src.rel.is_empty() {
        return;
    }
    egui::Frame::new().fill(t.fill).corner_radius(8.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(crate::tr!("Fichier", "File")).size(12.5).color(t.tertiary));
            ui.label(RichText::new(&src.rel).font(FontId::monospace(12.0)).color(t.secondary));
            let (msg, color) = if src.missing {
                (crate::tr!("· le fichier a été supprimé, cette page est gardée car modifiée ici", "· the file was deleted, this page is kept because it was edited here"), ORANGE)
            } else if src.conflict {
                (crate::tr!("· le fichier a changé, mais cette page a été modifiée ici : elle n'est plus mise à jour", "· the file changed, but this page was edited here: it's no longer updated"), ORANGE)
            } else if edited {
                (crate::tr!("· modifiée ici : elle ne suit plus le fichier", "· edited here: it no longer follows the file"), ORANGE)
            } else {
                (crate::tr!("· suit le fichier automatiquement", "· follows the file automatically"), t.tertiary)
            };
            ui.label(RichText::new(msg).size(12.5).color(color));
            if (edited || src.conflict) && !src.missing && link(ui, crate::tr!("Revenir au fichier", "Back to the file")) {
                actions.push(Action::RevertDoc(page_id.to_string()));
            }
            if !src.missing && link(ui, crate::tr!("Ouvrir le fichier", "Open the file")) {
                let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
                let _ = std::process::Command::new(opener).arg(&src.path).spawn();
            }
        });
    });
}

/// One row of the project's page tree.
#[derive(Clone)]
pub struct PageRow {
    pub id: String,
    pub depth: usize,
    pub icon: String,
    pub title: String,
    pub has_kids: bool,
    pub rel: String,
}

/// The project's sub-pages, depth first.
pub(super) fn page_tree(ws: &Workspace, root: &str) -> Vec<PageRow> {
    fn walk(ws: &Workspace, id: &str, depth: usize, out: &mut Vec<PageRow>) {
        for c in ws.children(Some(id)) {
            let kids = !ws.children(Some(&c.id)).is_empty();
            out.push(PageRow {
                id: c.id.clone(),
                depth,
                icon: page_icon(c).to_string(),
                title: c.display_title(),
                has_kids: kids,
                rel: c.source.as_ref().map(|s| s.rel.clone()).unwrap_or_default(),
            });
            if depth < 6 {
                walk(ws, &c.id, depth + 1, out);
            }
        }
    }
    let mut out = vec![];
    walk(ws, root, 0, &mut out);
    out
}

/// The project's pages (README, documentation…) as a tidy, foldable tree.
fn pages_card(ui: &mut egui::Ui, cx: &mut Cx, extra: f32) -> f32 {
    let (page_id, t, ro) = (cx.page_id, cx.t, cx.pr.read_only);
    let pages = cx.pages;
    let actions = &mut *cx.actions;
    card(ui, t, extra, |ui| {
        card_title(ui, "PAGES", &if pages.is_empty() { String::new() } else { pages.len().to_string() }, t);
        if pages.is_empty() {
            ui.label(RichText::new(crate::tr!("Le README et le dossier docs/ du projet apparaîtront ici en pages.", "The project's README and docs/ folder will show up here as pages.")).color(t.tertiary).italics());
        }
        tree_list(ui, pages, 1, t, actions);
        if !ro {
            ui.add_space(4.0);
            if link(ui, crate::tr!("+ Nouvelle page", "+ New page")) {
                actions.push(Action::NewPage(Some(page_id.to_string())));
            }
        }
    })
}

/// Rows of a page tree: fold arrows, icon, title, the file path on hover; a click opens.
/// Branches at `fold_from` levels and deeper start folded.
pub(super) fn tree_list(ui: &mut egui::Ui, pages: &[PageRow], fold_from: usize, t: &Tone, actions: &mut Vec<Action>) {
    // Folded branches: everything below a folded row is skipped.
    let mut hide_below: Option<usize> = None;
    for p in pages {
        if let Some(d) = hide_below {
            if p.depth > d {
                continue;
            }
            hide_below = None;
        }
        let fold_key = ("page-folded", p.id.as_str());
        let folded = p.has_kids && (flag(ui, fold_key) != (p.depth >= fold_from));
        let row_h = 28.0;
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, row_h), egui::Sense::click());
        let a = ui.ctx().animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.14);
        if a > 0.0 {
            ui.painter().rect_filled(rect, 6.0, t.hover.gamma_multiply(a));
        }
        let x0 = rect.left() + 4.0 + p.depth as f32 * 18.0 + 3.0 * a;
        // Fold arrow.
        let arrow = egui::Rect::from_min_size(egui::pos2(x0, rect.top() + 6.0), egui::vec2(16.0, 16.0));
        let mut arrow_clicked = false;
        if p.has_kids {
            let ar = ui.interact(arrow, resp.id.with("fold"), egui::Sense::click());
            icons::paint(ui.painter(), arrow.shrink(2.0), if folded { icons::Icon::ChevronRight } else { icons::Icon::ChevronDown }, t.tertiary);
            arrow_clicked = ar.clicked();
            if arrow_clicked {
                toggle_flag(ui, fold_key);
            }
        }
        let x = x0 + 20.0;
        ui.painter().text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, &p.icon, FontId::proportional(14.0), t.text);
        let path_w = if p.rel.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(p.rel.clone(), FontId::monospace(11.0), t.tertiary).size().x.min(w * 0.4) };
        let mut job = egui::text::LayoutJob::single_section(p.title.clone(), egui::TextFormat::simple(FontId::proportional(14.0), t.text));
        job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - x - 26.0 - path_w - 16.0).max(40.0));
        let g = ui.fonts_mut(|f| f.layout_job(job));
        ui.painter().galley(egui::pos2(x + 24.0, rect.center().y - g.size().y / 2.0), g, t.text);
        if !p.rel.is_empty() && resp.hovered() {
            let mut job = egui::text::LayoutJob::single_section(p.rel.clone(), egui::TextFormat::simple(FontId::monospace(11.0), t.tertiary));
            job.wrap = egui::text::TextWrapping::truncate_at_width(path_w);
            let g = ui.fonts_mut(|f| f.layout_job(job));
            ui.painter().galley(egui::pos2(rect.right() - 8.0 - g.size().x, rect.center().y - g.size().y / 2.0), g, t.tertiary);
        }
        if resp.clicked() && !arrow_clicked {
            actions.push(Action::Open(p.id.clone()));
        }
        if folded {
            hide_below = Some(p.depth);
        }
    }
}

/// A documentation folder: everything inside it, as a tree.
pub(super) fn folder_view(ui: &mut egui::Ui, rel: &str, pages: &[PageRow], t: &Tone, actions: &mut Vec<Action>) {
    let files = pages.iter().filter(|p| !p.has_kids).count();
    card(ui, t, 0.0, |ui| {
        let what = if rel.is_empty() { crate::tr!("Dossier", "Folder").to_string() } else { crate::trf!("Dossier {rel}", "Folder {rel}") };
        card_title(ui, crate::tr!("CONTENU", "CONTENTS"), &crate::trf!("{files} fichier(s)", "{files} file(s)"), t);
        ui.label(RichText::new(what).font(FontId::monospace(12.0)).color(t.tertiary));
        ui.add_space(6.0);
        if pages.is_empty() {
            ui.label(RichText::new(crate::tr!("Ce dossier est vide.", "This folder is empty.")).color(t.tertiary).italics());
        }
        tree_list(ui, pages, usize::MAX, t, actions);
    });
}

/// "Nouvelle session" buttons: a small terminal mark tinted per tool, then the name.
fn launch_pill(ui: &mut egui::Ui, label: &str, accent: Color32, t: &Tone) -> egui::Response {
    let font = FontId::proportional(12.5);
    let g = ui.painter().layout_no_wrap(label.to_string(), font, t.text);
    let size = egui::vec2(g.size().x + 40.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 13.0, if resp.hovered() { t.hover } else { t.card });
    p.rect_stroke(rect, 13.0, egui::Stroke::new(1.0, if resp.hovered() { accent.gamma_multiply(0.8) } else { t.border }), egui::StrokeKind::Inside);
    // Terminal mark: a rounded square with "›_".
    let m = egui::Rect::from_center_size(egui::pos2(rect.left() + 16.0, rect.center().y), egui::vec2(16.0, 14.0));
    p.rect_filled(m, 3.0, accent.gamma_multiply(0.22));
    let c = m.center();
    let s = egui::Stroke::new(1.4, accent);
    p.line_segment([c + egui::vec2(-4.5, -3.0), c + egui::vec2(-1.5, 0.0)], s);
    p.line_segment([c + egui::vec2(-1.5, 0.0), c + egui::vec2(-4.5, 3.0)], s);
    p.line_segment([c + egui::vec2(0.5, 3.5), c + egui::vec2(4.5, 3.5)], s);
    p.galley(egui::pos2(rect.left() + 30.0, rect.center().y - g.size().y / 2.0), g, t.text);
    resp
}

// ---------------------------------------------------------------- "Regard extérieur"

/// A review being written in the background.
pub struct ReviewJob {
    result: Arc<std::sync::Mutex<Option<Result<(String, crate::sessions::Tool), String>>>>,
}

#[derive(Clone, Default)]
struct ReviewRequest {
    page_id: String,
    folder: String,
    prefer: Option<crate::sessions::Tool>,
}

fn review_key() -> egui::Id {
    egui::Id::unique("review-request")
}

impl NotesApp {
    /// Start the review a button asked for, and collect the finished ones.
    pub(super) fn review_tick(&mut self, ctx: &egui::Context) {
        if let Some(r) = ctx.data_mut(|d| d.remove_temp::<ReviewRequest>(review_key())) {
            if !self.reviews.contains_key(&r.page_id) {
                let out: Arc<std::sync::Mutex<Option<_>>> = Arc::default();
                let (o2, c2) = (out.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let res = crate::sync::ask_review(&r.folder, r.prefer);
                    *o2.lock().unwrap() = Some(res);
                    c2.request_repaint();
                });
                self.reviews.insert(r.page_id.clone(), ReviewJob { result: out });
            }
        }
        let done: Vec<String> = self.reviews.iter().filter(|(_, j)| j.result.lock().unwrap().is_some()).map(|(k, _)| k.clone()).collect();
        for id in done {
            let job = self.reviews.remove(&id).unwrap();
            let res = job.result.lock().unwrap().take().unwrap();
            match res {
                Ok((text, tool)) => {
                    if let Some(pr) = self.ws.page_mut(&id).and_then(|p| p.project.as_mut()) {
                        pr.review = Some(crate::project::Review { text: crate::richtext::clean(&text, 2500), by: tool.label().to_string(), at: crate::model::now_ms() });
                        pr.touch();
                    }
                    self.detect_changes();
                }
                Err(e) => self.fail(crate::trf!("Regard extérieur impossible : {e}", "Outside view failed: {e}")),
            }
        }
        if !self.reviews.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }

}

/// "### Avis / ### Projets similaires / ### Suggestions": side by side when there is room
/// (each in a soft tile with its accent), one under the other otherwise.
fn review_sections(ui: &mut egui::Ui, text: &str, t: &Tone) {
    let mut parts: Vec<(String, String)> = vec![];
    for line in text.lines() {
        if let Some(h) = line.trim().strip_prefix("### ") {
            parts.push((h.trim().to_string(), String::new()));
        } else if let Some(last) = parts.last_mut() {
            last.1.push_str(line);
            last.1.push('\n');
        } else if !line.trim().is_empty() {
            parts.push((String::new(), format!("{line}\n")));
        }
    }
    let w = ui.available_width();
    let n = parts.len();
    let cols = if n >= 2 && w >= 860.0 { n.min(3) } else if n >= 2 && w >= 560.0 { 2 } else { 1 };
    if cols == 1 {
        super::rich::markdown(ui, text, TEXT, t);
        return;
    }
    let accents = [Color32::from_rgb(212, 76, 125), Color32::from_rgb(144, 101, 176), GREEN];
    let gap = 12.0;
    // Tiles of a row share the tallest one's height (from the last frame); a row with
    // fewer tiles shares its width between them.
    for (ri, row) in parts.chunks(cols).enumerate() {
        let k = row.len() as f32;
        let cw = ((w - gap * (k - 1.0)) / k).floor();
        let key = egui::Id::unique(("review-row", ri, cols, (w as i32)));
        let tallest: f32 = ui.ctx().data(|d| d.get_temp(key)).unwrap_or(0.0);
        let mut now = 0.0f32;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (ci, (title, body)) in row.iter().enumerate() {
                let accent = accents[(ri * cols + ci) % accents.len()];
                let r = egui::Frame::new()
                    .fill(accent.gamma_multiply(0.06))
                    .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.25)))
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(cw - 26.0);
                        ui.vertical(|ui| {
                        ui.set_width(cw - 26.0);
                        let top = ui.cursor().top();
                        if !title.is_empty() {
                            ui.label(RichText::new(title).font(semibold(14.0)).color(accent.lerp_to_gamma(t.text, 0.4)));
                            ui.add_space(4.0);
                        }
                        super::rich::markdown(ui, body.trim(), 13.5, t);
                        let used = ui.cursor().top() - top;
                        now = now.max(used);
                        if tallest > used {
                            ui.add_space(tallest - used);
                        }
                        });
                    });
                let _ = r;
            }
        });
        if (now - tallest).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(key, now));
            ui.ctx().request_repaint();
        }
        ui.add_space(gap);
    }
}

/// Full-width card at the bottom of a project: an AI's short, honest outside view —
/// opinion, similar projects that appeared, what to do next.
fn review_card(ui: &mut egui::Ui, cx: &mut Cx) {
    let (page_id, t) = (cx.page_id, cx.t);
    let pr = &*cx.pr;
    let prefer = cx.live.first().map(|s| s.tool);
    let reviewing = cx.reviewing || cx.ai.filling(page_id);
    card(ui, t, 0.0, |ui| {
        let by = pr.review.as_ref().map(|r| format!("{} · {}", r.by, relative_time(r.at))).unwrap_or_default();
        card_title(ui, crate::tr!("REGARD EXTÉRIEUR", "OUTSIDE VIEW"), &by, t);
        let ask = |ui: &mut egui::Ui, label: &str| {
            let folder = pr.folder.trim().to_string();
            let ok = !folder.is_empty() && crate::project::normalize_folder(&folder).is_dir();
            let r = ui.add_enabled(ok, egui::Button::new(RichText::new(label).size(13.0)));
            let r = if ok { r.on_hover_text(crate::tr!("L'IA lit le projet sans rien modifier, regarde le web, puis répond en quelques lignes (1 à 3 min, sur votre abonnement).", "The AI reads the project without changing anything, looks at the web, then answers in a few lines (1 to 3 min, on your subscription).")) } else { r.on_disabled_hover_text(crate::tr!("Indiquez d'abord le dossier du projet", "Set the project's folder first")) };
            if r.clicked() {
                let req = ReviewRequest { page_id: page_id.to_string(), folder, prefer };
                ui.ctx().data_mut(|d| d.insert_temp(review_key(), req));
            }
        };
        if reviewing {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(RichText::new(crate::tr!("L'IA lit le projet et cherche des projets similaires… (1 à 3 minutes)", "The AI reads the project and looks for similar projects… (1 to 3 minutes)")).color(t.secondary));
            });
        } else if let Some(r) = &pr.review {
            review_sections(ui, &r.text, t);
            ui.add_space(6.0);
            ask(ui, crate::tr!("Redemander un avis", "Ask again"));
        } else {
            ui.add(
                egui::Label::new(
                    RichText::new(crate::tr!("Demandez à l'IA branchée sur ce projet un avis court et sans complaisance, les projets similaires apparus récemment, et des idées pour la suite.", "Ask the AI plugged into this project for a short, frank opinion, the similar projects that appeared recently, and ideas for what's next."))
                        .color(t.secondary),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            ask(ui, crate::tr!("Demander un avis à l'IA", "Ask the AI for an opinion"));
        }
    });
}
