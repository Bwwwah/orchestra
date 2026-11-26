//! The AI projects in the window: the "Projets" dashboard (one card per project) and the
//! project panel shown at the top of a project's page (status, folder, state of play,
//! steps, AI sessions). The AIs fill the same data through MCP (see mcp.rs).

use super::chrome::relative_time;
use super::*;
use crate::project::{Module, Project, ProjectMember, ProjectStatus, StepStatus};
use crate::sessions::{SessionInfo, human_tokens};
use crate::templates::Template;
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

/// The system's folder picker for a form, run on its own thread so the window keeps
/// drawing; the chosen folder is picked up on the next frame, by the form that asked.
pub(super) mod folder_picker {
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    static BUSY: AtomicBool = AtomicBool::new(false);
    static PICKED: Mutex<Option<(&str, PathBuf)>> = Mutex::new(None);

    pub fn busy() -> bool {
        BUSY.load(Ordering::Relaxed)
    }

    pub fn start(
        ctx: &eframe::egui::Context,
        form: &'static str,
        prompt: &'static str,
        from: Option<PathBuf>,
    ) {
        if BUSY.swap(true, Ordering::SeqCst) {
            return;
        }
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let got = crate::opener::pick_folder(prompt, from.as_deref());
            if let Some(p) = got {
                *PICKED.lock().unwrap_or_else(|e| e.into_inner()) = Some((form, p));
            }
            BUSY.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    /// The folder chosen for `form` since the last call, if any.
    pub fn take(form: &str) -> Option<PathBuf> {
        let mut picked = PICKED.lock().unwrap_or_else(|e| e.into_inner());
        picked.take_if(|(f, _)| *f == form).map(|(_, p)| p)
    }
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
                format!(
                    "{}={}@{};",
                    p.id,
                    pr.map(|x| x.folder.as_str()).unwrap_or(""),
                    pr.map(|x| x.sessions_cleared).unwrap_or(0)
                )
            })
            .collect();
        if Arc::ptr_eq(&self.src, &live) && sig == self.sig {
            return;
        }
        self.by_project.clear();
        let mut loose: Vec<(String, Vec<SessionInfo>)> = vec![];
        for s in live.iter().filter(|s| !s.internal) {
            match ws.project_for_path(std::path::Path::new(&s.cwd)) {
                Some(p) => {
                    // Hidden by "Effacer l'historique" until it is used again.
                    if p.project
                        .as_ref()
                        .is_none_or(|pr| pr.session_shown(s.last_activity))
                    {
                        self.by_project
                            .entry(p.id.clone())
                            .or_default()
                            .push(s.clone());
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
    let last = live
        .iter()
        .map(|s| s.last_activity)
        .max()
        .unwrap_or(0)
        .max(pr.last_activity)
        .max(updated);
    let stored_only = pr
        .sessions
        .iter()
        .filter(|x| !live.iter().any(|l| l.id == x.id))
        .count();
    let tokens = live.iter().map(|s| s.tokens.billed).sum();
    (last, live.len() + stored_only, tokens)
}

fn now() -> u64 {
    crate::model::now_ms()
}

pub(super) const BLUE: Color32 = super::style::BLUE;
pub(super) const GREEN: Color32 = super::style::GREEN;
pub(super) const ORANGE: Color32 = super::style::ORANGE;

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
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(12.0), color);
    let (rect, resp) = ui.allocate_exact_size(g.size() + egui::vec2(14.0, 6.0), sense);
    let fill = color.gamma_multiply(if resp.hovered() { 0.28 } else { 0.16 });
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter()
        .galley(rect.min + egui::vec2(7.0, 3.0), g, color);
    resp
}

/// The tool of a session ("Claude Code", "Codex"…) as a chip in a fixed-width column, so
/// the titles of the rows below each other start at the same place.
fn tool_chip(ui: &mut egui::Ui, label: &str, t: &Tone) {
    let col = ui
        .painter()
        .layout_no_wrap(
            "Claude Code".into(),
            FontId::proportional(12.0),
            t.secondary,
        )
        .size()
        .x
        + 14.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(col, 20.0), egui::Sense::hover());
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
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
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "orchestra".into());
    format!("claude mcp add --scope user orchestra -- \"{exe}\" mcp")
}

/// What the « Nouveau projet » form holds.
#[derive(Default)]
pub(super) struct NewProject {
    pub title: String,
    pub folder: String,
    /// None: a bare project, whose kind and modules are guessed from its folder.
    pub template: Option<Template>,
}

impl NewProject {
    /// Create the project in `ws` and return its page id.
    pub fn create_in(&self, ws: &mut Workspace) -> String {
        match self.template {
            Some(template) => ws.create_project_from(template, &self.title, &self.folder),
            None => ws.create_project(&self.title, &self.folder),
        }
    }
}

/// The choice of a template in the « Nouveau projet » form, and what the choice gives.
fn template_picker(ui: &mut egui::Ui, chosen: &mut Option<Template>, t: &Tone) {
    ui.horizontal_wrapped(|ui| {
        ui.label(crate::tr!("Modèle", "Template"));
        ui.selectable_value(chosen, None, crate::tr!("Aucun", "None"));
        for template in Template::ALL {
            let text = format!("{} {}", template.icon(), template.label());
            ui.selectable_value(chosen, Some(template), text);
        }
    });
    let about = match *chosen {
        Some(template) => {
            let pages: Vec<&str> = template.pages().iter().map(|p| p.title).collect();
            let pages = pages.join(", ");
            let description = template.description();
            crate::trf!("{description} Pages : {pages}.", "{description} Pages: {pages}.")
        }
        None => crate::tr!(
            "Un projet vide : ses modules suivent le contenu de son dossier.",
            "An empty project: its modules follow what its folder holds."
        )
        .to_string(),
    };
    ui.label(RichText::new(about).size(12.5).color(t.tertiary));
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
                    ui.label(RichText::new(crate::tr!("Projets", "Projects")).font(bold(super::style::TITLE)).color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.new_project.is_none() {
                            let r = super::style::primary(ui, crate::tr!("+ Nouveau projet", "+ New project"));
                            super::tutorial::mark(ui, "projects:new", r.rect);
                            if r.clicked() {
                                self.new_project = Some(NewProject::default());
                            }
                        }
                        let busy = self.sync_dialog.is_some() || self.sync_run.is_some();
                        let sync = ui
                            .add_enabled_ui(!busy, |ui| super::style::secondary(ui, crate::tr!("Synchroniser avec les IA", "Sync with the AIs")))
                            .inner
                            .on_hover_text(crate::tr!("Demande à Claude Code (ou Codex) de se brancher au tableau de bord et de faire l'état des lieux de vos projets", "Asks Claude Code (or Codex) to plug into the dashboard and write where each of your projects stands"));
                        super::tutorial::mark(ui, "projects:sync", sync.rect);
                        if sync.clicked() {
                            self.sync_dialog = Some(self.sync_candidates());
                        }
                    });
                });
                ui.label(RichText::new(crate::tr!("Où en est chaque projet et ce qu'il reste à faire.", "Where each project stands and what is left to do.")).color(t.secondary));
                self.mcp_line(ui, &t);
                ui.add_space(16.0);

                self.sync_view(ui, width, &t);

                // A folder dropped on the window: start a project from it.
                let dropped: Option<std::path::PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).find(|p| p.is_dir()));
                if let Some(dir) = dropped {
                    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    // A template already picked in the open form stays picked.
                    let template = self.new_project.as_ref().and_then(|f| f.template);
                    self.new_project = Some(NewProject { title: name, folder: dir.display().to_string(), template });
                }

                // New project form: a name, the folder, and whether an AI should analyse it.
                let mut create = None;
                let mut cancel = false;
                if let Some(NewProject { title, folder, template }) = self.new_project.as_mut() {
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
                            if let Some(dir) = folder_picker::take("new-project") {
                                *folder = dir.display().to_string();
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let busy = folder_picker::busy();
                                let browse = ui
                                    .add_enabled(!busy, egui::Button::new(crate::tr!("Parcourir…", "Browse…")))
                                    .on_hover_text(crate::tr!("Choisir le dossier dans le Finder / l'explorateur", "Pick the folder in the Finder / file explorer"))
                                    .on_disabled_hover_text(crate::tr!("Fenêtre de choix ouverte", "Picker window open"));
                                if browse.clicked() {
                                    let start = crate::project::normalize_folder(folder.trim());
                                    folder_picker::start(ui.ctx(), "new-project", crate::tr!("Dossier du projet", "Project folder"), start.is_dir().then_some(start).or_else(dirs::home_dir));
                                }
                                let fr = ui.add(egui::TextEdit::singleline(folder).hint_text(crate::tr!("~/Code/mon-projet", "~/Code/my-project")).font(FontId::monospace(13.0)).desired_width(f32::INFINITY));
                                if title.is_empty() && folder.is_empty() && !fr.has_focus() && !busy {
                                    fr.request_focus();
                                }
                            });
                            ui.end_row();
                            ui.label(crate::tr!("Nom", "Name"));
                            let auto = std::path::Path::new(folder.trim()).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                            ui.add(egui::TextEdit::singleline(title).hint_text(if auto.is_empty() { crate::tr!("Mon projet", "My project").to_string() } else { auto }).desired_width(f32::INFINITY));
                            ui.end_row();
                        });
                        ui.add_space(8.0);
                        template_picker(ui, template, &t);
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
                        // A template says the kind of project itself.
                        if folder_ok && template.is_none() {
                            // Detected once per folder typed, not on every frame.
                            let kind_key = egui::Id::unique(("new-project-code", folder.trim().to_string()));
                            let code = ui.ctx().data(|d| d.get_temp::<bool>(kind_key)).unwrap_or_else(|| {
                                let c = crate::project::is_code_folder(&crate::project::normalize_folder(folder.trim()));
                                ui.ctx().data_mut(|d| d.insert_temp(kind_key, c));
                                c
                            });
                            let text = if code { crate::tr!("Projet de code détecté", "Code project detected") } else { crate::tr!("Projet sans code (documents, cours, démarches…)", "Non-code project (documents, school, paperwork…)") };
                            ui.label(RichText::new(text).size(12.5).color(t.tertiary));
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
                                let new = NewProject { title: name, folder: folder.trim().to_string(), template: *template };
                                create = Some((new, analyse && folder_ok));
                            }
                            if ui.button(crate::tr!("Annuler", "Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                                cancel = true;
                            }
                        });
                    });
                    ui.add_space(16.0);
                }
                if let Some((new, analyse)) = create {
                    let folder = new.folder.clone();
                    actions.push(Action::NewProject(new));
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
                    ui.label(RichText::new(crate::tr!("Aucun projet pour l'instant : synchronisez avec l'IA ou ajoutez-en un ci-dessous.", "No project yet: sync with the AI or add one below.")).color(t.secondary));
                }
                let gap = 14.0;
                let cols = ((width + gap) / (300.0 + gap)).floor().max(1.0);
                let card_w = (width - gap * (cols - 1.0)) / cols;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                    for (id, icon, title, pr, updated) in &projects {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(card_w, 168.0), egui::Sense::click());
                        if projects.first().is_some_and(|f| f.0 == *id) {
                            super::tutorial::mark(ui, "projects:card", rect);
                        }
                        let p = ui.painter();
                        p.rect_filled(rect, 12.0, if resp.hovered() { t.selected } else { t.card });
                        p.rect_stroke(rect, 12.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                        let inner = rect.shrink(16.0);
                        let mut ui2 = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
                        ui2.spacing_mut().item_spacing.y = 6.0;
                        ui2.horizontal(|ui| {
                            ui.label(RichText::new(icon).size(20.0));
                            let mut job = egui::text::LayoutJob::single_section(title.clone(), egui::TextFormat::simple(semibold(super::style::HEADING), t.text));
                            job.wrap = egui::text::TextWrapping::truncate_at_width(card_w - 140.0);
                            cut_label(ui, job);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                chip(ui, pr.status.label(), status_color(pr.status, &t), egui::Sense::hover());
                            });
                        });
                        let folder = if pr.folder.trim().is_empty() { crate::tr!("Pas de dossier", "No folder").to_string() } else { format!("📁 {}", short_folder(&pr.folder)) };
                        let mut job = egui::text::LayoutJob::single_section(folder, egui::TextFormat::simple(FontId::proportional(12.0), t.tertiary));
                        job.wrap = egui::text::TextWrapping::truncate_at_width(card_w - 34.0);
                        let r = ui2.label(job);
                        if !pr.folder.trim().is_empty() {
                            r.on_hover_text(tilde(&pr.folder));
                        }
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
                self.more_projects_view(ui, width, &t);
            });
        });
    }
}

impl NotesApp {
    /// Below the cards: bring in more projects, by syncing with the AIs (every folder where
    /// Claude Code or Codex worked is offered there) or by adding one by hand.
    fn more_projects_view(&mut self, ui: &mut egui::Ui, width: f32, t: &Tone) {
        if self.sync_dialog.is_some() || self.sync_run.is_some() || self.new_project.is_some() {
            return;
        }
        let waiting = self.agents.unassigned.len();
        // The head of the page already offers both actions: this box only speaks up when it
        // has news (folders where an AI worked, with no project yet) or when the list is empty.
        if waiting == 0 && !self.ws.projects().is_empty() {
            return;
        }
        ui.add_space(28.0);
        egui::Frame::new().fill(t.card).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(14.0).inner_margin(egui::Margin::symmetric(24, 22)).show(ui, |ui| {
            ui.set_width(width - 50.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(crate::tr!("D'autres projets à suivre ?", "More projects to follow?")).font(semibold(17.0)).color(t.text));
                ui.add_space(4.0);
                let line = match waiting {
                    0 => crate::tr!("Les IA font l'état des lieux de vos dossiers, ou ajoutez un projet vous-même.", "The AIs write where your folders stand, or add a project yourself.").to_string(),
                    1 => crate::tr!("1 dossier où une IA a travaillé n'a pas encore de projet.", "1 folder where an AI worked has no project yet.").to_string(),
                    n => crate::trf!("{n} dossiers où une IA a travaillé n'ont pas encore de projet.", "{n} folders where an AI worked have no project yet."),
                };
                ui.label(RichText::new(line).size(13.5).color(t.secondary));
                ui.add_space(14.0);
                // Two buttons side by side, centred.
                let gap = 12.0;
                let total = 340.0;
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = gap;
                    if super::style::primary(ui, crate::tr!("Synchroniser avec les IA", "Sync with the AIs"))
                        .on_hover_text(crate::tr!("Choisissez les dossiers : Claude Code (ou Codex) crée les projets et fait leur état des lieux", "Pick the folders: Claude Code (or Codex) creates the projects and writes where they stand"))
                        .clicked()
                    {
                        self.sync_dialog = Some(self.sync_candidates());
                    }
                    if super::style::secondary(ui, crate::tr!("+ Nouveau projet", "+ New project")).on_hover_text(crate::tr!("À partir d'un dossier ; astuce : glissez-le sur la fenêtre", "From a folder; tip: drop it on the window")).clicked() {
                        self.new_project = Some(NewProject::default());
                    }
                });
            });
        });
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
pub(super) fn card(
    ui: &mut egui::Ui,
    t: &Tone,
    extra: f32,
    add: impl FnOnce(&mut egui::Ui),
) -> f32 {
    // The border lights up softly under the pointer (last frame's rect).
    let id = ui.next_auto_id().with("card-hover");
    let prev: Option<egui::Rect> = ui.ctx().data(|d| d.get_temp(id));
    let hovered = prev.is_some_and(|r| ui.rect_contains_pointer(r));
    let a = ui.ctx().animate_bool_with_time(id, hovered, 0.18);
    let border = t.border.lerp_to_gamma(t.tertiary, 0.6 * a);
    let r = egui::Frame::new()
        .fill(t.card)
        .shadow(egui::epaint::Shadow {
            offset: [0, (2.0 + 4.0 * a) as i8],
            blur: (6.0 + 10.0 * a) as u8,
            spread: 0,
            color: Color32::from_black_alpha((18.0 + 30.0 * a) as u8),
        })
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

/// A row's background that fades in under the pointer: call `hover_begin` before drawing
/// the row, `hover_end` with its rect after.
pub(super) fn hover_begin(ui: &egui::Ui) -> egui::layers::ShapeIdx {
    ui.painter().add(egui::Shape::Noop)
}

/// A folder as shown on screen: the home directory abbreviated to « ~ », the same way
/// everywhere (`/Users/name/Code/x` and `~/Code/x` read alike).
pub(super) fn tilde(path: &str) -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    tilde_in(path.trim(), &home)
}

/// A folder as shown in lists and headers: its last name only (« ~/Code/site » reads « site »),
/// the full path being left for the tooltip.
pub(super) fn short_folder(path: &str) -> String {
    let p = path.trim().trim_end_matches(['/', '\\']);
    match p.rsplit(['/', '\\']).next() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => tilde(path),
    }
}

#[test]
fn short_folder_keeps_the_last_name() {
    assert_eq!(short_folder("/Users/a/Code/site/"), "site");
    assert_eq!(short_folder("C:\\Code\\appli"), "appli");
    assert_eq!(short_folder("site"), "site");
}

fn tilde_in(path: &str, home: &str) -> String {
    let home = home.trim_end_matches(['/', '\\']);
    if home.is_empty() {
        return path.to_string();
    }
    match path.strip_prefix(home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with(['/', '\\']) => format!("~{rest}"),
        _ => path.to_string(),
    }
}

#[test]
fn tilde_abbreviates_the_home_folder() {
    assert_eq!(tilde_in("/Users/ana/Code/x", "/Users/ana"), "~/Code/x");
    assert_eq!(tilde_in("/Users/ana", "/Users/ana/"), "~");
    assert_eq!(tilde_in("/Users/anabel/x", "/Users/ana"), "/Users/anabel/x");
    assert_eq!(tilde_in("~/Code/x", "/Users/ana"), "~/Code/x");
    assert_eq!(tilde_in("/opt/x", ""), "/opt/x");
}

pub(super) fn hover_end(
    ui: &egui::Ui,
    idx: egui::layers::ShapeIdx,
    rect: egui::Rect,
    id: egui::Id,
    t: &Tone,
) -> f32 {
    let r = rect.expand2(egui::vec2(6.0, 2.0));
    let a = ui
        .ctx()
        .animate_bool_with_time(id, ui.rect_contains_pointer(r), 0.14);
    if a > 0.0 {
        ui.painter().set(
            idx,
            egui::Shape::rect_filled(r, 7.0, t.hover.gamma_multiply(a)),
        );
    }
    a
}

/// A dot that breathes: something is happening right now.
pub(super) fn pulse_dot(ui: &egui::Ui, c: egui::Pos2, color: Color32) {
    let time = ui.input(|i| i.time);
    let ph = (time % 1.6 / 1.6) as f32;
    ui.painter()
        .circle_filled(c, 4.0 + 5.0 * ph, color.gamma_multiply(0.45 * (1.0 - ph)));
    ui.painter().circle_filled(c, 4.0, color);
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(33));
}

/// Fraction going smoothly to `target` (bars, progress).
pub(super) fn smooth(
    ui: &egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    target: f32,
) -> f32 {
    ui.ctx()
        .animate_value_with_time(egui::Id::unique(id), target, 0.6)
}

/// "ÉTAPES        3 / 10" style header: small caps on the left, extra on the right.
pub(super) fn card_title(ui: &mut egui::Ui, title: &str, extra: &str, t: &Tone) {
    // A plain title: colour is kept for meaning (running, waiting, error), not decoration.
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(semibold(super::style::SMALL)).color(t.secondary));
        if !extra.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(extra).size(super::style::SMALL).color(t.tertiary));
            });
        }
    });
    ui.add_space(8.0);
}

/// A thin line across the card, between a list and its footer actions.
pub(super) fn card_rule(ui: &mut egui::Ui, t: &Tone) {
    ui.add_space(6.0);
    let (r, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(
        r.x_range(),
        r.center().y,
        egui::Stroke::new(1.0, t.border.gamma_multiply(0.7)),
    );
    ui.add_space(6.0);
}

/// A secondary action (delete, clear…): grey text that darkens under the pointer, quieter
/// than the blue pills of the main actions.
pub(super) fn quiet_link(ui: &mut egui::Ui, text: &str, t: &Tone) -> egui::Response {
    let font = FontId::proportional(12.5);
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, t.tertiary);
    let (rect, resp) =
        ui.allocate_exact_size(g.size() + egui::vec2(4.0, 8.0), egui::Sense::click());
    let a = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.14);
    let color = t.tertiary.lerp_to_gamma(t.secondary, a);
    let pos = egui::pos2(rect.left() + 2.0, rect.center().y - g.size().y / 2.0);
    ui.painter()
        .galley_with_override_text_color(pos, g.clone(), color);
    if a > 0.0 {
        let y = pos.y + g.size().y + 0.5;
        ui.painter().hline(
            pos.x..=pos.x + g.size().x,
            y,
            egui::Stroke::new(1.0, color.gamma_multiply(a)),
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn flag(ui: &egui::Ui, key: (&str, &str)) -> bool {
    ui.ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::unique(key)).unwrap_or(false))
}

fn toggle_flag(ui: &egui::Ui, key: (&str, &str)) {
    let v = flag(ui, key);
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::unique(key), !v));
}

fn link(ui: &mut egui::Ui, text: &str) -> bool {
    action_link(ui, text).clicked()
}

/// A small action: a soft blue pill that brightens under the pointer, an arrow that slides
/// in, and a press that sinks it a little.
pub(super) fn action_link(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let arrow = !text.starts_with('+') && !text.ends_with('▾') && !text.ends_with('▴');
    let font = FontId::proportional(12.5);
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), font.clone(), BLUE);
    let extra = if arrow { 14.0 } else { 0.0 };
    let size = g.size() + egui::vec2(18.0 + extra, 8.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let a = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.16);
    let down = ui.ctx().animate_bool_with_time(
        resp.id.with("down"),
        resp.is_pointer_button_down_on(),
        0.08,
    );
    let r = rect.shrink(1.2 * down);
    let p = ui.painter();
    p.rect_filled(r, 8.0, BLUE.gamma_multiply(0.10 + 0.14 * a));
    if a > 0.0 {
        p.rect_stroke(
            r,
            8.0,
            egui::Stroke::new(1.0, BLUE.gamma_multiply(0.45 * a)),
            egui::StrokeKind::Inside,
        );
    }
    let color = BLUE.lerp_to_gamma(Color32::from_rgb(120, 180, 255), a * 0.6);
    let pos = egui::pos2(r.left() + 9.0, r.center().y - g.size().y / 2.0);
    p.galley(pos, g, color);
    if arrow {
        let x = r.right() - 13.0 + 3.0 * a;
        p.text(
            egui::pos2(x, r.center().y),
            egui::Align2::CENTER_CENTER,
            "→",
            font,
            color.gamma_multiply(0.35 + 0.65 * a),
        );
    }
    // Screen readers (and the tests) find it by its text, like a button.
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One-line text cut at the width, with "…". Returns its response (hover shows all).
fn line(ui: &mut egui::Ui, job: egui::text::LayoutJob, width: f32) -> egui::Response {
    let mut job = job;
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(30.0));
    let g = ui.fonts_mut(|f| f.layout_job(job));
    let (r, resp) = ui.allocate_exact_size(
        egui::vec2(width.max(30.0), g.size().y),
        egui::Sense::click(),
    );
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

/// "Backend : implémenter X" → ("Backend", "implémenter X"): the theme AIs put before a
/// step (see `AI_STEP_FORMAT`); the Steps card groups the steps under it. A colon with
/// nothing after it but more text (« https://… », « 10:30 ») starts no theme.
fn split_theme(title: &str) -> (Option<&str>, &str) {
    if let Some((a, rest)) = title.split_once(':') {
        let a = a.trim();
        if !a.is_empty()
            && a.chars().count() <= 40
            && a.split_whitespace().count() <= 4
            && !a.contains(['(', ')', '«', '»', '"', '`'])
            && rest.starts_with(char::is_whitespace)
            && !rest.trim().is_empty()
        {
            return (Some(a), rest.trim());
        }
    }
    (None, title)
}

/// The steps of one theme (indexes into the project's steps).
struct StepGroup {
    /// None: the steps without a theme.
    theme: Option<String>,
    open: Vec<usize>,
    done: Vec<usize>,
}

impl StepGroup {
    /// What tells the groups apart: the theme, whatever its case.
    fn key(&self) -> String {
        theme_key(self.theme.as_deref())
    }
}

fn theme_key(theme: Option<&str>) -> String {
    theme.unwrap_or_default().to_lowercase()
}

/// The steps by theme, each theme once. Themes come in the order of their first open
/// step; those with only finished steps come after.
fn step_groups(steps: &[crate::project::Step]) -> Vec<StepGroup> {
    let mut groups: Vec<StepGroup> = vec![];
    let mut at = std::collections::HashMap::new();
    for finished in [false, true] {
        for (i, s) in steps.iter().enumerate() {
            if (s.status == StepStatus::Done) != finished {
                continue;
            }
            let theme = split_theme(&s.title).0;
            let g = *at.entry(theme_key(theme)).or_insert_with(|| {
                groups.push(StepGroup {
                    theme: theme.map(str::to_string),
                    open: vec![],
                    done: vec![],
                });
                groups.len() - 1
            });
            if finished {
                groups[g].done.push(i);
            } else {
                groups[g].open.push(i);
            }
        }
    }
    groups
}

/// A theme's open steps (`ids`, as shown) with `moved` put just before `before`, or at
/// their end with None.
fn order_with(ids: &[String], moved: &str, before: Option<&str>) -> Vec<String> {
    if before == Some(moved) {
        return ids.to_vec();
    }
    let mut out: Vec<String> = ids.iter().filter(|x| *x != moved).cloned().collect();
    let to = before
        .and_then(|b| out.iter().position(|x| x == b))
        .unwrap_or(out.len());
    out.insert(to, moved.to_string());
    out
}

/// The six dots of a drag handle, in a space `height` tall. `shown`: false leaves the
/// space empty (nothing to reorder).
fn drag_handle(ui: &mut egui::Ui, height: f32, shown: bool, held: bool, t: &Tone) -> egui::Response {
    let (r, h) = ui.allocate_exact_size(egui::vec2(10.0, height), egui::Sense::drag());
    if !shown {
        return h;
    }
    let col = if h.hovered() || h.dragged() || held {
        t.secondary
    } else {
        t.border
    };
    for dy in [-5.0, 0.0, 5.0] {
        for dx in [-2.0, 2.0] {
            ui.painter()
                .circle_filled(r.center() + egui::vec2(dx, dy), 1.3, col);
        }
    }
    h.on_hover_cursor(egui::CursorIcon::Grab)
}

/// A folded step: its text on one line, cut with "…". When it is cut, hovering shows all
/// of it; a click (the caller's) unfolds it in place.
fn step_line(ui: &mut egui::Ui, job: egui::text::LayoutJob, width: f32) -> egui::Response {
    let width = width.max(30.0);
    let mut full = job.clone();
    let mut job = job;
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let g = ui.fonts_mut(|f| f.layout_job(job));
    let cut = g.elided;
    let (r, resp) = ui.allocate_exact_size(egui::vec2(width, g.size().y), egui::Sense::click());
    ui.painter().galley(r.min, g, t_text_fallback());
    // Screen readers (and the tests) find it by its whole text.
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &full.text));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !cut {
        return resp;
    }
    // Read in the tooltip: a finished step isn't struck through there.
    for sec in full.sections.iter_mut() {
        sec.format.strikethrough = egui::Stroke::NONE;
    }
    resp.on_hover_ui(|ui| {
        ui.set_max_width(420.0);
        ui.add(egui::Label::new(full).wrap());
    })
}

/// Status of a step as a round icon: ○ to do, ◐ in progress, ● done.
fn step_icon(ui: &mut egui::Ui, step_id: &str, st: StepStatus, t: &Tone) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::click());
    let c = r.center();
    let hov = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.12);
    let color = step_color(st, t).lerp_to_gamma(BLUE, hov);
    // Filling in when it becomes "en cours" or "terminée", with a small pop.
    let doing = ui.ctx().animate_bool_with_time(
        egui::Id::unique(("step-doing", step_id)),
        st == StepStatus::Doing,
        0.25,
    );
    let done = ui.ctx().animate_bool_with_time(
        egui::Id::unique(("step-done", step_id)),
        st == StepStatus::Done,
        0.3,
    );
    let pop = 1.0 + 0.25 * (done * std::f32::consts::PI).sin();
    let p = ui.painter();
    p.circle_stroke(c, 6.5 * pop, egui::Stroke::new(1.5, color));
    if doing > 0.0 {
        p.circle_filled(c, 3.5 * doing, color);
    }
    if done > 0.0 {
        p.circle_filled(c, 7.0 * done * pop, color);
        let k = done;
        p.line_segment(
            [c + egui::vec2(-3.0, 0.2) * k, c + egui::vec2(-0.8, 2.6) * k],
            egui::Stroke::new(1.6, t.card),
        );
        p.line_segment(
            [c + egui::vec2(-0.8, 2.6) * k, c + egui::vec2(3.4, -2.4) * k],
            egui::Stroke::new(1.6, t.card),
        );
    }
    resp.on_hover_text(crate::trf!(
        "{} — cliquer pour changer",
        "{} — click to change",
        st.label()
    ))
}

fn session_row(
    ui: &mut egui::Ui,
    page_id: &str,
    s: &SessionInfo,
    modules: &[Module],
    t: &Tone,
) -> Option<super::notices::Notice> {
    let mut msg = None;
    let open_key = ("session-open", s.id.as_str());
    let open = flag(ui, open_key);
    let w = ui.available_width();
    let bg = hover_begin(ui);
    let row = ui.horizontal(|ui| {
        let live = s.is_live(now());
        let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 20.0), egui::Sense::hover());
        if live {
            pulse_dot(ui, r.center(), GREEN);
        } else {
            ui.painter().circle_filled(r.center(), 4.0, t.border);
        }
        tool_chip(ui, s.tool.label(), t);
        let when = relative_time(s.last_activity);
        let when_w = ui
            .painter()
            .layout_no_wrap(when.clone(), FontId::proportional(12.5), t.tertiary)
            .size()
            .x;
        let room = ui.available_width() - when_w - 16.0;
        let job = egui::text::LayoutJob::single_section(
            s.display_title(),
            egui::TextFormat::simple(FontId::proportional(TEXT), t.text),
        );
        let r = line(ui, job, room);
        ui.label(RichText::new(when).size(12.5).color(t.tertiary));
        r
    });
    hover_end(
        ui,
        bg,
        row.response.rect,
        egui::Id::unique(("session-row", &s.id)),
        t,
    );
    let resp = row.inner;
    if resp.clicked() {
        toggle_flag(ui, open_key);
    }
    let _ = page_id;
    if open {
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 18,
                right: 0,
                top: 2,
                bottom: 6,
            })
            .show(ui, |ui| {
                ui.set_max_width(w - 18.0);
                let n = s.user_messages + s.agent_messages;
                let mut meta = crate::trf!(
                    "{n} message{p}",
                    "{n} message{p}",
                    p = crate::i18n::plural(n as usize)
                );
                if modules.contains(&Module::Budget) {
                    if s.tokens.billed > 0 {
                        meta.push_str(&crate::trf!(
                            " · {} tokens",
                            " · {} tokens",
                            human_tokens(s.tokens.billed)
                        ));
                    }
                    let spent = crate::pricing::dollars(s.tokens.cost_micro);
                    if !spent.is_empty() {
                        meta.push_str(&format!(" · ≈ {spent}"));
                    }
                }
                for x in [&s.git_branch, &s.model] {
                    if !x.is_empty() {
                        meta.push_str(&format!(" · {x}"));
                    }
                }
                ui.label(RichText::new(meta).size(12.5).color(t.tertiary));
                if !s.last_reply.is_empty() {
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(crate::trf!(
                                "Dernière réponse : {}",
                                "Last answer: {}",
                                s.last_reply
                            ))
                            .size(13.0)
                            .color(t.secondary),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let (Some(cmd), Some(resume)) = (s.resume_command(), s.resume_args()) else {
                        ui.label(
                            RichText::new(crate::tr!(
                                "Id de session invalide : reprise impossible",
                                "Invalid session id: can't resume"
                            ))
                            .size(12.5)
                            .color(t.tertiary),
                        );
                        return;
                    };
                    // Without the terminal module, only the command to copy.
                    if modules.contains(&Module::Terminal) {
                        if action_link(ui, crate::tr!("Reprendre ici", "Resume here"))
                            .on_hover_text(crate::tr!(
                                "Rouvre la session dans le terminal intégré",
                                "Reopens the session in the built-in terminal"
                            ))
                            .clicked()
                        {
                            let title = format!(
                                "{} · {}",
                                s.tool.label(),
                                crate::richtext::one_line(&s.display_title(), 24)
                            );
                            super::term_ui::request(ui.ctx(), &title, &s.cwd, Some(resume));
                        }
                        if action_link(ui, crate::tr!("Terminal macOS", "macOS Terminal"))
                            .on_hover_text(&cmd)
                            .clicked()
                        {
                            msg = Some(match crate::sessions::open_in_terminal(&cmd) {
                                Ok(()) => super::notices::Notice::Ack(
                                    crate::tr!(
                                        "Session rouverte dans le Terminal.",
                                        "Session reopened in the Terminal."
                                    )
                                    .to_string(),
                                ),
                                Err(e) => {
                                    ui.ctx().copy_text(cmd.clone());
                                    super::notices::Notice::Error(crate::trf!(
                                        "Terminal indisponible ({e}) : commande copiée.",
                                        "Terminal unavailable ({e}): command copied."
                                    ))
                                }
                            });
                        }
                    }
                    if action_link(ui, crate::tr!("Copier la commande", "Copy the command"))
                        .on_hover_text(&cmd)
                        .clicked()
                    {
                        ui.ctx().copy_text(cmd.clone());
                        msg = Some(super::notices::Notice::Ack(
                            crate::tr!("Commande copiée.", "Command copied.").into(),
                        ));
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
    /// An autonomous session runs on this project now.
    pub running: bool,
    /// The steps those sessions are on, with where each stands, for their rows.
    pub step_sessions: Vec<StepSession>,
    /// An AI able to work alone is plugged in (Claude Code): « Lancer une IA » can start.
    pub can_launch: bool,
    /// Who else is in this (shared) project right now, and whether their AI is at work.
    pub peers: Vec<(String, bool)>,
    pub vers: &'a mut super::versions::Versions,
    /// "Nouvelle session" buttons: the plugged AIs that run in a terminal, then the shell.
    pub launchers: Vec<(&'static str, Option<&'static str>, Color32)>,
    /// The AI filling the cards, proposing steps.
    pub ai: &'a mut super::ai_fill::AiJobs,
    /// The AIs' questions about this project (waiting first, then answered), from the inbox.
    pub questions: &'a [crate::model::InboxMsg],
    /// An answer typed in the Questions card: (inbox message id, text), saved by the caller.
    pub answered: &'a mut Option<(String, String)>,
    /// The team space of a project with members (`Project::members`), for their card.
    pub team: super::members_ui::TeamCx,
}

/// A running autonomous session, as the row of the step it is on says it.
#[derive(Clone, Debug, Default)]
pub(super) struct StepSession {
    pub run: String,
    pub step: String,
    /// « tour 2 », « démarre »…
    pub state: String,
    /// What it last said or does; paused: why. May be empty.
    pub detail: String,
    /// Paused, waiting for the person.
    pub waiting: bool,
}

/// Under a step's row, the session at work on it, live: a breathing dot, where it stands
/// and the way to its terminal. True when that is asked.
fn step_session_line(ui: &mut egui::Ui, live: &StepSession, indent: f32, t: &Tone) -> bool {
    let color = if live.waiting { ORANGE } else { GREEN };
    let text = if live.waiting && !live.detail.is_empty() {
        // The reason says it is paused.
        live.detail.clone()
    } else {
        [live.state.as_str(), live.detail.as_str()]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let mut watch = false;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 18.0), egui::Sense::hover());
        pulse_dot(ui, dot.center(), color);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            watch = link(ui, crate::tr!("Regarder", "Watch"));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let color = if live.waiting { ORANGE } else { t.secondary };
                let mut job = egui::text::LayoutJob::single_section(
                    text.clone(),
                    egui::TextFormat::simple(FontId::proportional(12.5), color),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
                let r = cut_label(ui, job).on_hover_text(&text);
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
            });
        });
    });
    watch
}

#[derive(Clone, Copy)]
enum CardKind {
    Summary(f32),
    Steps,
    /// Starting a session (once the project has a folder), apart from the sessions' list.
    NewSession,
    Sessions,
    Pages,
    Versions,
    /// Known vulnerabilities of the dependencies (code projects).
    Watch,
    Questions,
    /// The people and AIs of the team space (team projects).
    Members,
}

fn draw(ui: &mut egui::Ui, cx: &mut Cx, k: CardKind, extra: f32) -> f32 {
    // Cards come in one after the other when the page opens.
    let order = match k {
        CardKind::Summary(_) => 0.0,
        CardKind::Steps | CardKind::Questions => 1.0,
        CardKind::NewSession | CardKind::Sessions | CardKind::Pages | CardKind::Members => 2.0,
        CardKind::Versions | CardKind::Watch => 3.0,
    };
    let t0: f64 = ui
        .ctx()
        .data(|d| d.get_temp(appear_key()))
        .map(|(_, t): (String, f64)| t)
        .unwrap_or(0.0);
    let p = (((ui.input(|i| i.time) - t0) as f32 - order * 0.07) / 0.35).clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - p).powi(3);
    if e < 1.0 {
        ui.ctx().request_repaint();
    }
    let top = ui.cursor().min;
    ui.scope(|ui| {
        ui.multiply_opacity(e);
        if e < 1.0 {
            ui.add_space(10.0 * (1.0 - e));
        }
        let h = draw_card(ui, cx, k, extra);
        // Where the card is, for the tutorial to point at.
        let w = ui.min_rect().width().max(ui.available_width().min(ui.min_rect().width()));
        super::tutorial::mark(
            ui,
            card_key_name(k),
            egui::Rect::from_min_size(top, egui::vec2(w, h)),
        );
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
        CardKind::NewSession => new_session_card(ui, cx, extra),
        CardKind::Sessions => sessions_card(ui, cx, extra),
        CardKind::Pages => pages_card(ui, cx, extra),
        CardKind::Questions => questions_card(ui, cx, extra),
        CardKind::Members => {
            let (h, manage) =
                super::members_ui::members_card(ui, &cx.pr.members, &cx.team, cx.t, extra);
            if manage {
                cx.actions.push(Action::ManageMembers);
            }
            h
        }
        CardKind::Watch => {
            let folder = cx.pr.folder.trim().to_string();
            let autonomous = cx.pr.has_module(Module::Autonomous);
            let auto = &mut cx.pr.cve_watch;
            super::watch::watch_card(ui, cx.page_id, &folder, auto, autonomous, cx.t, extra)
        }
        CardKind::Versions => {
            let folder = cx.pr.folder.trim().to_string();
            let prefer = cx.live.first().map(|s| s.tool);
            let title = cx.title.clone();
            super::versions::versions_card(
                ui, cx.vers, cx.page_id, &folder, &title, prefer, cx.t, extra,
            )
        }
    }
}

fn card_key(page_id: &str, k: CardKind) -> egui::Id {
    let name = match k {
        CardKind::Summary(_) => "summary",
        CardKind::Steps => "steps",
        CardKind::NewSession => "new-session",
        CardKind::Sessions => "sessions",
        CardKind::Pages => "pages",
        CardKind::Versions => "versions",
        CardKind::Watch => "watch",
        CardKind::Questions => "questions",
        CardKind::Members => "members",
    };
    egui::Id::unique(("card-h", page_id, name))
}

/// The name a card goes by in the tutorial (see `tutorial::tour_stops`).
fn card_key_name(k: CardKind) -> &'static str {
    match k {
        CardKind::Summary(_) => "card:summary",
        CardKind::Steps => "card:steps",
        CardKind::NewSession => "card:new-session",
        CardKind::Sessions => "card:sessions",
        CardKind::Pages => "card:pages",
        CardKind::Versions => "card:versions",
        CardKind::Watch => "card:watch",
        CardKind::Questions => "card:questions",
        CardKind::Members => "card:members",
    }
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
                    let extra = if last && cols.len() > 1 {
                        (target - prev.get(ci).copied().unwrap_or(target)).max(0.0)
                    } else {
                        0.0
                    };
                    total += draw(ui, cx, *k, extra);
                }
                now[ci] = total;
            });
        }
    });
    if now
        .iter()
        .zip(prev.iter().chain(std::iter::repeat(&-1.0)))
        .any(|(a, b)| (a - b).abs() > 0.5)
    {
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
        ui.ctx()
            .data_mut(|d| d.insert_temp(appear_key(), (cx.page_id.to_string(), now_t)));
    }
    let (cx_launch, cx_follow, cx_settings);
    let mut add_step_asked = false;
    {
        let (page_id, t, live) = (cx.page_id, cx.t, cx.live);
        let peers = cx.peers.clone();
        let filling = cx.ai.filling(page_id);
        let prefer = live.first().map(|s| s.tool);
        let (cx_running, cx_can_launch) = (cx.running, cx.can_launch);
        let pr = &mut *cx.pr;
        // ---- Header: status, progress, activity, tokens; folder below, small.
        let (done, total) = pr.progress();
        let (last, _, tokens) = activity(pr, 0, live);
        // Line 1: where the project stands on the left; on the right its main action,
        // « Lancer une IA » (or where to follow the one at work), then « Remplir avec l'IA ».
        let mut launch = false;
        let mut follow = false;
        let mut settings = false;
        // Why « Lancer une IA » is greyed out: also said under it, not only on hover.
        let mut blocked = None;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if cx_running {
                    if super::style::secondary(ui, crate::tr!("IA au travail · Suivre", "AI at work · Follow"))
                        .on_hover_text(crate::tr!("Voir la session dans Orchestration", "See the session in Orchestration"))
                        .clicked()
                    {
                        follow = true;
                    }
                } else if !pr.read_only && pr.has_module(Module::Autonomous) {
                    let folder_ok = !pr.folder.trim().is_empty();
                    blocked = launch_block(cx_can_launch, folder_ok, pr);
                    let r = ui
                        .add_enabled_ui(blocked.is_none(), |ui| super::style::primary(ui, crate::tr!("▶  Lancer une IA", "▶  Start an AI")))
                        .inner;
                    let why = blocked.map_or("", LaunchBlock::why);
                    let r = r.on_disabled_hover_text(why).on_hover_text(crate::tr!(
                        "Une IA avance seule sur les étapes ouvertes, dans une copie à part du dossier, avec les réglages de lancement de ce projet (« Réglages… »).",
                        "An AI works alone on the open steps, in a separate copy of the folder, with this project's launch settings (« Settings… »)."
                    ));
                    if r.clicked() {
                        launch = true;
                    }
                    let rs = super::style::secondary(ui, crate::tr!("Réglages…", "Settings…"))
                        .on_hover_text(crate::tr!("L'IA, le modèle, le plafond, une consigne, le mode sans fin : gardés pour les prochaines sessions de ce projet.", "The AI, the model, the ceiling, a mission, endless mode: kept for this project's next sessions."));
                    super::tutorial::mark(ui, "project:actions", r.rect.union(rs.rect));
                    if rs.clicked() {
                        settings = true;
                    }
                }
                    // Fill (again) the cards with the AI: after a warning, since it replaces them.
                    if filling {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(RichText::new(crate::tr!("L'IA remplit le projet… (1 à 3 min)", "The AI is filling in the project… (1 to 3 min)")).size(12.5).color(BLUE));
                    } else if !pr.read_only {
                        let folder = pr.folder.trim().to_string();
                        let ok = !folder.is_empty() && crate::project::normalize_folder(&folder).is_dir();
                        let r = ui.add_enabled_ui(ok, |ui| super::style::secondary(ui, crate::tr!("Remplir avec l'IA", "Fill in with the AI"))).inner;
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
                            // A model chosen in the settings: say who gets the project's summary.
                            if let Some(service) = crate::sync::openai::Service::from_id(&cx.ai.text_model.service) {
                                ui.add_space(2.0);
                                ui.add(egui::Label::new(RichText::new(crate::trf!("Par {} : son serveur reçoit un résumé du dossier (Paramètres › IA).", "By {}: its server gets a summary of the folder (Settings › AI).", service.label())).size(12.5).color(t.secondary)).wrap());
                            }
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
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
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
                    if tokens > 0 && pr.has_module(Module::Budget) {
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
                });
            });
        });
        if let Some(why) = blocked {
            add_step_asked = launch_block_line(ui, why, t);
        }
        let editing_folder =
            !pr.read_only && (flag(ui, ("edit-folder", page_id)) || pr.folder.trim().is_empty());
        // Line 2: the folder. Code, Lecture seule and Privé are in the page's « … » menu
        // (`project_menu`).
        ui.horizontal(|ui| {
            if editing_folder {
                let r = ui.add(egui::TextEdit::singleline(&mut pr.folder).hint_text(crate::tr!("Dossier du projet, ex. ~/Code/mon-projet", "Project folder, e.g. ~/Code/my-project")).font(FontId::monospace(12.5)).desired_width(f32::INFINITY));
                if r.lost_focus() && !pr.folder.trim().is_empty() && flag(ui, ("edit-folder", page_id)) {
                    toggle_flag(ui, ("edit-folder", page_id));
                }
            } else {
                // The folder's name only (the full path shows under the pointer).
                let r = ui.add(egui::Label::new(RichText::new(format!("📁 {}", short_folder(&pr.folder))).size(12.5).color(t.tertiary)).sense(egui::Sense::click()).truncate());
                let tip = if pr.read_only { crate::tr!("Lecture seule", "Read-only") } else { crate::tr!("Cliquer pour modifier le dossier", "Click to change the folder") };
                if r.on_hover_text(format!("{}\n{tip}", tilde(&pr.folder))).clicked() && !pr.read_only {
                    toggle_flag(ui, ("edit-folder", page_id));
                }
            }
        });
        ui.add_space(12.0);
        cx_launch = launch;
        cx_follow = follow;
        cx_settings = settings;
    }
    if cx_launch {
        cx.actions.push(Action::LaunchAuto(cx.page_id.to_string()));
    }
    if cx_follow {
        cx.actions.push(Action::ShowOrchestra);
    }
    if cx_settings {
        cx.actions.push(Action::LaunchSettings(cx.page_id.to_string()));
    }
    quick_question(ui, cx);
    ui.add_space(14.0);
    // The rest in tabs: the overview (where it stands, what is left, the sessions) is what
    // one comes for; versions, the watch, the pages and the outside view are a click away.
    let tabs = project_tabs(!cx.pr.members.is_empty(), &cx.pr.modules());
    let tab_key = egui::Id::unique(("project-tab", cx.page_id));
    let mut tab: ProjectTab = ui.ctx().data(|d| d.get_temp(tab_key)).unwrap_or_default();
    // A tour pointing at a card shows its tab.
    if let Some(want) = super::tutorial::wanted(ui.ctx())
        && let Some(to) = tab_of_mark(&want)
    {
        tab = to;
    }
    if !tabs.contains(&tab) || add_step_asked {
        tab = ProjectTab::Overview;
    }
    if add_step_asked {
        open_new_step(ui.ctx(), cx.page_id);
    }
    let tabs_top = ui.cursor().min;
    if let Some(t2) = tab_bar(ui, &tabs, tab, cx.t) {
        tab = t2;
    }
    super::tutorial::mark(
        ui,
        "project:tabs",
        egui::Rect::from_min_max(tabs_top, egui::pos2(ui.max_rect().right(), ui.cursor().min.y)),
    );
    ui.ctx().data_mut(|d| d.insert_temp(tab_key, tab));
    ui.add_space(14.0);
    let gap = 16.0;
    use CardKind::*;
    match tab {
        ProjectTab::Overview => {
            let w = ui.available_width();
            // The AIs' questions, when there are some, come first: they wait for the person.
            if !cx.questions.is_empty() {
                columns(ui, cx, &[vec![Questions]], gap);
                ui.add_space(gap);
            }
            if w >= 860.0 {
                columns(ui, cx, &[vec![Summary(380.0)], vec![Steps]], gap);
            } else {
                columns(ui, cx, &[vec![Summary(210.0), Steps]], gap);
            }
            ui.add_space(gap);
            // The activity: starting a session, and those that ran here.
            let mut act = vec![];
            if !cx.pr.folder.trim().is_empty() && cx.pr.has_module(Module::Terminal) {
                act.push(NewSession);
            }
            act.push(Sessions);
            columns(ui, cx, &[act], gap);
        }
        ProjectTab::Versions => {
            columns(ui, cx, &[vec![Versions]], gap);
        }
        ProjectTab::Watch => {
            columns(ui, cx, &[vec![Watch]], gap);
        }
        ProjectTab::Pages => {
            columns(ui, cx, &[vec![Pages]], gap);
        }
        ProjectTab::Review => review_card(ui, cx),
        ProjectTab::Members => {
            columns(ui, cx, &[vec![Members]], gap);
        }
    }
    cx.msg.take()
}

/// What keeps « Lancer une IA » greyed out.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LaunchBlock {
    NoAi,
    NoFolder,
    NoStep,
    /// There are steps, all of them finished.
    AllDone,
}

impl LaunchBlock {
    fn why(self) -> &'static str {
        match self {
            LaunchBlock::NoAi => crate::tr!(
                "Pour lancer une IA, branchez d'abord Claude Code (Paramètres › IA).",
                "To start an AI, plug in Claude Code first (Settings › AI)."
            ),
            LaunchBlock::NoFolder => crate::tr!(
                "Pour lancer une IA, indiquez d'abord le dossier du projet.",
                "To start an AI, set the project's folder first."
            ),
            LaunchBlock::NoStep => crate::tr!(
                "Pour lancer une IA, ajoutez d'abord une étape : elle les prend une à une.",
                "To start an AI, add a step first: it takes them one by one."
            ),
            LaunchBlock::AllDone => crate::tr!(
                "Toutes les étapes sont terminées : ajoutez-en une pour relancer une IA.",
                "All steps are finished: add one to start an AI again."
            ),
        }
    }
}

/// The first thing missing to start an AI on this project, if any. The endless mode
/// needs no open step.
fn launch_block(can_launch: bool, folder_ok: bool, pr: &Project) -> Option<LaunchBlock> {
    let open = pr.steps.iter().any(|s| s.status != StepStatus::Done);
    let endless = pr.launch.as_ref().is_some_and(|l| l.endless);
    if !can_launch {
        Some(LaunchBlock::NoAi)
    } else if !folder_ok {
        Some(LaunchBlock::NoFolder)
    } else if open || endless {
        None
    } else if pr.steps.is_empty() {
        Some(LaunchBlock::NoStep)
    } else {
        Some(LaunchBlock::AllDone)
    }
}

/// Room kept at the end of a step's row for its « Lancer » button.
const STEP_LAUNCH_W: f32 = 60.0;

/// What keeps a step's « Lancer » greyed out, said on hover.
fn step_launch_block(can_launch: bool, folder_ok: bool, held_elsewhere: bool) -> Option<&'static str> {
    if held_elsewhere {
        Some(crate::tr!(
            "Une IA d'un autre ordinateur a réservé cette étape.",
            "An AI on another computer has reserved this step."
        ))
    } else if !can_launch {
        Some(LaunchBlock::NoAi.why())
    } else if !folder_ok {
        Some(LaunchBlock::NoFolder.why())
    } else {
        None
    }
}

/// The reason under the greyed button, on its side of the page; when a step is what is
/// missing, with the way to add one. True when that is asked.
fn launch_block_line(ui: &mut egui::Ui, block: LaunchBlock, t: &Tone) -> bool {
    let mut add = false;
    ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
        ui.add(
            egui::Label::new(RichText::new(block.why()).size(12.5).color(t.secondary)).wrap(),
        );
        if matches!(block, LaunchBlock::NoStep | LaunchBlock::AllDone) {
            add = link(ui, crate::tr!("+ Ajouter une étape", "+ Add a step"));
        }
    });
    add
}

fn steps_scroll_key(page_id: &str) -> egui::Id {
    egui::Id::unique(("steps-scroll-once", page_id))
}

/// Opens the « Ajouter une étape » field of this project's Steps card and brings it into view.
fn open_new_step(ctx: &egui::Context, page_id: &str) {
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::unique(("adding-step", page_id)), true);
        d.insert_temp(steps_scroll_key(page_id), true);
    });
}

/// A module's name and what it shows, in the « Modules » menu.
fn module_texts(module: Module) -> (&'static str, &'static str) {
    match module {
        Module::Git => (
            crate::tr!("Git", "Git"),
            crate::tr!(
                "Les onglets Versions et Veille CVE, et la veille automatique.",
                "The Versions and CVE watch tabs, and the automatic watch."
            ),
        ),
        Module::Terminal => (
            crate::tr!("Terminal", "Terminal"),
            crate::tr!(
                "La carte pour ouvrir une session dans le terminal, et la reprise d'une session.",
                "The card to open a session in the terminal, and resuming a session."
            ),
        ),
        Module::Autonomous => (
            crate::tr!("Sessions autonomes", "Autonomous sessions"),
            crate::tr!(
                "Lancer une IA qui travaille seule, sur le projet ou sur une étape, et la veille automatique.",
                "Starting an AI that works on its own, on the project or on a step, and the automatic watch."
            ),
        ),
        Module::Budget => (
            crate::tr!("Budget", "Budget"),
            crate::tr!(
                "Les tokens et le coût estimé du projet et de ses sessions.",
                "The tokens and estimated cost of the project and its sessions."
            ),
        ),
        Module::Versions => (
            crate::tr!("Versions", "Versions"),
            crate::tr!("L'onglet Versions.", "The Versions tab."),
        ),
    }
}

/// « Modules », in a project's settings: one box per module, and the way back to the
/// defaults once a choice was made. Stays open on a click inside, to set several in a row.
fn modules_menu(ui: &mut egui::Ui, pr: &mut Project) {
    egui::containers::menu::SubMenuButton::new(crate::tr!("Modules", "Modules"))
        .config(
            egui::containers::menu::MenuConfig::new()
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
        )
        .ui(ui, |ui| {
            ui.set_min_width(220.0);
            // Read once per frame: for a code module without a choice, the folder is looked at.
            let git = pr.has_module(Module::Git);
            for module in Module::ALL {
                let (name, what) = module_texts(module);
                // The Versions tab only comes with Git (`project_tabs`): off without it.
                let needs_git = module == Module::Versions && !git;
                let mut on = !needs_git && pr.has_module(module);
                if ui
                    .add_enabled(!needs_git, egui::Checkbox::new(&mut on, name))
                    .on_hover_text(what)
                    .on_disabled_hover_text(crate::tr!(
                        "Demande le module Git.",
                        "Needs the Git module."
                    ))
                    .changed()
                {
                    pr.set_module(module, Some(on));
                }
            }
            // Only our own modules: the choice for one of a newer version is kept.
            let chosen = Module::ALL
                .iter()
                .any(|m| pr.module_choices.contains_key(m.key()));
            if chosen {
                ui.separator();
                if ui
                    .button(crate::tr!("Revenir à l'automatique", "Back to automatic"))
                    .on_hover_text(crate::tr!(
                        "Git, Terminal et Versions suivent le type du projet ; les autres sont activés.",
                        "Git, Terminal and Versions follow the project's type; the others are on."
                    ))
                    .clicked()
                {
                    for module in Module::ALL {
                        pr.set_module(module, None);
                    }
                }
            }
        });
}

/// A project's settings in its page's « … » menu: Lecture seule, code or not, its modules,
/// Privé or Partagé. Sharing has effects beyond the project (sending, notice): the choice made is
/// returned in `share` for the caller to apply; so is a local copy, in `copy`.
pub(super) fn project_menu(
    ui: &mut egui::Ui,
    pr: &mut Project,
    share: &mut Option<bool>,
    copy: &mut bool,
) {
    let mut ro = pr.read_only;
    if ui
        .checkbox(&mut ro, crate::tr!("Lecture seule", "Read-only"))
        .on_hover_text(crate::tr!("Empêche de modifier ce projet et ses pages par erreur. Les IA peuvent toujours mettre à jour l'état des lieux et les étapes.", "Prevents editing this project and its pages by mistake. The AIs can still update the current state and the steps."))
        .changed()
    {
        pr.read_only = ro;
    }
    // Code or not: detected from the folder, or forced here (the detection can be wrong).
    let detected = crate::project::detected_code(&pr.folder);
    let kind = if pr.code.unwrap_or(detected) {
        crate::tr!("Code", "Code")
    } else {
        crate::tr!("Sans code", "No code")
    };
    let label = if pr.code.is_some() {
        crate::trf!("Type : {kind}", "Type: {kind}")
    } else {
        crate::trf!("Type : {kind} (auto)", "Type: {kind} (auto)")
    };
    ui.add_enabled_ui(!pr.read_only, |ui| {
        ui.menu_button(label, |ui| {
            ui.set_min_width(220.0);
            let auto = crate::trf!(
                "Automatique (détecté : {})",
                "Automatic (detected: {})",
                if detected {
                    crate::tr!("code", "code")
                } else {
                    crate::tr!("sans code", "no code")
                }
            );
            for (choice, text) in [
                (None, auto.as_str()),
                (Some(true), crate::tr!("Projet de code", "Code project")),
                (Some(false), crate::tr!("Projet sans code", "Project without code")),
            ] {
                if ui.selectable_label(pr.code == choice, text).clicked() {
                    pr.code = choice;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(crate::tr!(
            "Projet de code ou non : décide des modules code (Versions, veille, terminal) et de ce que l'IA regarde et répond. Détecté d'après le dossier (un dépôt Git compte comme du code) ; choisissez ici si la détection se trompe.",
            "Code project or not: decides the code modules (Versions, watch, terminal) and what the AI looks at and answers. Detected from the folder (a git repository counts as code); choose here if the detection is wrong."
        ));
        modules_menu(ui, pr);
    });
    let label = if pr.shared {
        crate::tr!("Visibilité : Partagé", "Visibility: Shared")
    } else {
        crate::tr!("Visibilité : Privé", "Visibility: Private")
    };
    ui.menu_button(label, |ui| {
        ui.set_min_width(240.0);
        if pr.shared {
            if ui.button(crate::tr!("Rendre privé", "Make private")).on_hover_text(crate::tr!("Le projet ne part plus chez les autres ; ils gardent leur copie.", "The project no longer goes to the others; they keep their copy.")).clicked() {
                *share = Some(false);
                ui.close();
            }
            if ui.button(crate::tr!("Garder une copie locale", "Keep a local copy")).on_hover_text(crate::tr!("Une copie privée qui ne suit plus le projet partagé.", "A private copy that no longer follows the shared project.")).clicked() {
                *copy = true;
                ui.close();
            }
        } else if ui.button(crate::tr!("Partager avec l'équipe", "Share with the team")).on_hover_text(crate::tr!("Le projet, ses pages et sa discussion partent par la collaboration branchée.", "The project, its pages and its discussion go through the collaboration you set up.")).clicked() {
            *share = Some(true);
            ui.close();
        }
    })
    .response
    .on_hover_text(crate::tr!("Seuls les projets partagés quittent cette machine (collaboration). Les autres restent privés.", "Only shared projects leave this computer (collaboration). The others stay private."));
}

/// The tabs of a project's page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ProjectTab {
    #[default]
    Overview,
    Versions,
    Watch,
    Pages,
    Review,
    Members,
}

impl ProjectTab {
    fn label(self) -> &'static str {
        match self {
            ProjectTab::Overview => crate::tr!("Vue d'ensemble", "Overview"),
            ProjectTab::Versions => crate::tr!("Versions", "Versions"),
            ProjectTab::Watch => crate::tr!("Veille CVE", "CVE watch"),
            ProjectTab::Pages => crate::tr!("Pages", "Pages"),
            ProjectTab::Review => crate::tr!("Regard extérieur", "Outside view"),
            ProjectTab::Members => crate::tr!("Membres", "Members"),
        }
    }
}

/// The tabs shown: versions and the vulnerability watch only with the git module
/// (`modules`: those that are on, by default for code projects), versions with theirs too;
/// the members only for a project of a team space.
pub(super) fn project_tabs(team: bool, modules: &[Module]) -> Vec<ProjectTab> {
    let mut v = vec![ProjectTab::Overview];
    if modules.contains(&Module::Git) {
        if modules.contains(&Module::Versions) {
            v.push(ProjectTab::Versions);
        }
        v.push(ProjectTab::Watch);
    }
    v.extend([ProjectTab::Pages, ProjectTab::Review]);
    if team {
        v.push(ProjectTab::Members);
    }
    v
}

/// The tab holding a tour's mark (« card:versions »…).
pub(super) fn tab_of_mark(mark: &str) -> Option<ProjectTab> {
    Some(match mark {
        "card:summary" | "card:steps" | "card:questions" | "card:sessions"
        | "card:new-session" => ProjectTab::Overview,
        "card:versions" => ProjectTab::Versions,
        "card:watch" => ProjectTab::Watch,
        "card:pages" => ProjectTab::Pages,
        "card:members" => ProjectTab::Members,
        _ => return None,
    })
}

/// A row of text tabs, the open one underlined in the accent; returns the one clicked.
fn tab_bar(
    ui: &mut egui::Ui,
    tabs: &[ProjectTab],
    open: ProjectTab,
    t: &Tone,
) -> Option<ProjectTab> {
    let mut picked = None;
    let top = ui.cursor().min.y;
    let r = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 22.0;
        for &k in tabs {
            let on = k == open;
            let text = k.label().to_string();
            let font = if on { semibold(super::style::BODY) } else { FontId::proportional(super::style::BODY) };
            let g = ui.painter().layout_no_wrap(text.clone(), font, t.text);
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(g.size().x, 30.0), egui::Sense::click());
            let color = if on {
                t.text
            } else if resp.hovered() {
                t.secondary.lerp_to_gamma(t.text, 0.5)
            } else {
                t.secondary
            };
            ui.painter().galley_with_override_text_color(
                egui::pos2(rect.left(), rect.center().y - g.size().y / 2.0 - 2.0),
                g,
                color,
            );
            if on {
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(rect.left(), rect.bottom() - 2.0),
                        egui::pos2(rect.right(), rect.bottom()),
                    ),
                    1.0,
                    super::style::BLUE,
                );
            }
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, &text));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                picked = Some(k);
            }
        }
    });
    // A hairline under the whole row, the open tab's line sitting on it.
    let y = r.response.rect.bottom();
    let _ = top;
    ui.painter().hline(
        ui.max_rect().x_range(),
        y - 0.5,
        egui::Stroke::new(1.0, t.border),
    );
    picked
}

/// The questions the AIs asked about this project (ask_human) and the person's answers:
/// those waiting first, answerable right here, as on the Orchestration page.
fn questions_card(ui: &mut egui::Ui, cx: &mut Cx, extra: f32) -> f32 {
    let t = cx.t;
    let msgs: Vec<&crate::model::InboxMsg> = cx.questions.iter().collect();
    let mut sent = None;
    let h = card(ui, t, extra, |ui| {
        card_title(
            ui,
            crate::tr!("QUESTIONS DES IA", "THE AIS' QUESTIONS"),
            &super::orch_ui::questions_count(&msgs),
            t,
        );
        sent = super::orch_ui::question_list(ui, &msgs, |_| None, t);
    });
    if sent.is_some() {
        *cx.answered = sent;
    }
    h
}

fn summary_card(ui: &mut egui::Ui, cx: &mut Cx, folded_h: f32, extra: f32) -> f32 {
    let (page_id, pr, t) = (cx.page_id, &*cx.pr, cx.t);
    // ---- State of play: folded to ~8 lines unless unfolded.
    card(ui, t, extra, |ui| {
        let by = if pr.summary_by.is_empty() {
            String::new()
        } else {
            format!("{} · {}", pr.summary_by, relative_time(pr.summary_updated))
        };
        card_title(ui, crate::tr!("ÉTAT DES LIEUX", "CURRENT STATE"), &by, t);
        if cx.ai.filling(page_id) {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(
                    RichText::new(crate::tr!(
                        "L'IA lit le projet et écrit l'état des lieux…",
                        "The AI is reading the project and writing its state…"
                    ))
                    .size(13.0)
                    .color(t.secondary),
                );
            });
            ui.add_space(4.0);
        }
        if pr.summary.trim().is_empty() {
            if !cx.ai.filling(page_id) {
                ui.label(RichText::new(crate::tr!("Pas encore écrit. Cliquez sur « Remplir avec l'IA » en haut, ou demandez à votre IA : « fais l'état des lieux de ce projet dans le tableau de bord ».", "Not written yet. Click « Fill in with the AI » at the top, or ask your AI: « write the current state of this project in the dashboard ».")).color(t.tertiary).italics());
            }
            return;
        }
        // Above the text, so it shows while the card is folded.
        if !cx.ai.filling(page_id)
            && let Some(note) = pr.stale_summary_note()
        {
            ui.label(RichText::new(note).size(12.5).color(ORANGE));
            ui.add_space(6.0);
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
            child.set_clip_rect(
                egui::Rect::from_min_size(top, egui::vec2(w, folded_h)).intersect(ui.clip_rect()),
            );
        }
        child.set_max_width(760.0f32.min(w));
        super::rich::markdown(&mut child, &pr.summary, TEXT, t);
        let content_h = child.min_rect().height();
        let overflows = content_h > folded_h + 4.0;
        let shown = if open || !overflows {
            content_h
        } else {
            folded_h
        };
        let shown_rect = egui::Rect::from_min_size(top, egui::vec2(w, shown));
        ui.allocate_rect(shown_rect, egui::Sense::hover());
        // Folded, the lines past the fold can't be selected: a mouse selection begun in the
        // text that reaches the fold unfolds the card, so it can go on to the last line.
        if overflows && !open {
            let reach = ui.input(|i| {
                (i.pointer.primary_down(), i.pointer.press_origin(), i.pointer.interact_pos())
            });
            if selection_reaches_fold(reach.0, reach.1, reach.2, shown_rect) {
                toggle_flag(ui, key);
                ui.ctx().request_repaint();
            }
        }
        if overflows && !open {
            // Fade the last lines into the card.
            let fade = egui::Rect::from_min_max(
                egui::pos2(shown_rect.left(), shown_rect.bottom() - 48.0),
                shown_rect.max,
            );
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
        if overflows
            && link(
                ui,
                if open {
                    crate::tr!("Réduire ▴", "Collapse ▴")
                } else {
                    crate::tr!("Afficher tout ▾", "Show all ▾")
                },
            )
        {
            toggle_flag(ui, key);
        }
    })
}

/// A drag (button `down`, begun at `origin`) in a folded text shown in `shown`, whose pointer
/// is now at `pos`, near or past the fold.
fn selection_reaches_fold(
    down: bool,
    origin: Option<egui::Pos2>,
    pos: Option<egui::Pos2>,
    shown: egui::Rect,
) -> bool {
    down
        && origin.is_some_and(|p| shown.contains(p))
        && pos.is_some_and(|p| p.y > shown.bottom() - 12.0)
}

#[test]
fn a_selection_reaching_the_fold_unfolds_the_state_of_play() {
    let shown = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 200.0));
    let (inside, near, past) = (egui::pos2(20.0, 10.0), egui::pos2(30.0, 195.0), egui::pos2(30.0, 260.0));
    assert!(selection_reaches_fold(true, Some(inside), Some(near), shown));
    assert!(selection_reaches_fold(true, Some(inside), Some(past), shown));
    // Still in the visible lines, button up, or a drag begun elsewhere: stays folded.
    assert!(!selection_reaches_fold(true, Some(inside), Some(egui::pos2(30.0, 100.0)), shown));
    assert!(!selection_reaches_fold(false, Some(inside), Some(past), shown));
    assert!(!selection_reaches_fold(true, Some(egui::pos2(30.0, 300.0)), Some(past), shown));
}

fn steps_focus_key() -> egui::Id {
    egui::Id::unique("steps-focus")
}

/// Arriving from « Ajouter des tâches » (Orchestration page): the project's page opens on
/// its Steps card.
pub(super) fn focus_steps(ctx: &egui::Context, page_id: &str) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(steps_focus_key(), (page_id.to_string(), now)));
}

/// How long the « tapez vos tâches ici » line stays after arriving from « Ajouter des tâches ».
const STEPS_HINT_SECS: f64 = 90.0;

/// Seconds since this page was opened from « Ajouter des tâches », if it was.
fn steps_focus_age(ctx: &egui::Context, page_id: &str) -> Option<f64> {
    let (id, at): (String, f64) = ctx.data(|d| d.get_temp(steps_focus_key()))?;
    (id == page_id).then(|| ctx.input(|i| i.time) - at)
}

/// Whether the Steps card should be brought into view now: during the first moments, while
/// the cards come in and settle (their heights change the layout).
fn take_steps_scroll(ctx: &egui::Context, page_id: &str) -> bool {
    let go = steps_focus_age(ctx, page_id).is_some_and(|a| a < 0.6);
    if go {
        ctx.request_repaint();
    }
    go
}

fn steps_card(ui: &mut egui::Ui, cx: &mut Cx, extra: f32) -> f32 {
    let (page_id, t, ro) = (cx.page_id, cx.t, cx.pr.read_only);
    let asked = ui
        .ctx()
        .data_mut(|d| d.remove_temp::<bool>(steps_scroll_key(page_id)));
    if take_steps_scroll(ui.ctx(), page_id) || asked.is_some() {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    let pr = &mut *cx.pr;
    let new_step = &mut *cx.new_step;
    let ai = &mut *cx.ai;
    let prefer = cx.live.first().map(|s| s.tool);
    let me = cx.team.me.as_str();
    let (running, can_launch) = (cx.running, cx.can_launch);
    let autonomous = pr.has_module(Module::Autonomous);
    let actions = &mut *cx.actions;
    let sessions = cx.step_sessions.as_slice();
    // ---- Steps: open ones first, one line each; done ones folded.
    let (done, total) = pr.progress();
    let (start, width) = (ui.cursor().min, ui.available_width());
    let h = card(ui, t, extra, |ui| {
        card_title(
            ui,
            crate::tr!("ÉTAPES", "STEPS"),
            &format!("{done} / {total}"),
            t,
        );
        let mut cycle = None;
        let mut remove = None;
        let show_done = flag(ui, ("done-open", page_id));
        let groups = step_groups(&pr.steps);
        // A single group without a theme is a plain list: no heading.
        let headed = groups.iter().any(|g| g.theme.is_some());
        let open_groups = groups.iter().filter(|g| !g.open.is_empty()).count();
        // The rows as shown: each theme's open steps, then its finished ones when asked.
        let rows: Vec<(usize, usize)> = groups
            .iter()
            .enumerate()
            .flat_map(|(g, group)| {
                let done = group.done.iter().filter(move |_| show_done);
                group.open.iter().chain(done).map(move |&i| (g, i))
            })
            .collect();
        if pr.steps.is_empty() {
            ui.label(RichText::new(crate::tr!("Aucune étape. Ajoutez-en une, demandez des suggestions à l'IA, ou collez un texte à transformer en étapes.", "No step. Add one, ask the AI for suggestions, or paste a text to turn into steps.")).color(t.tertiary).italics());
        }
        // Open steps are put in order within their theme by dragging them by their handle
        // (left of the circle); a whole theme moves by the handle of its heading.
        let drag_key = egui::Id::unique(("step-drag", page_id));
        // (true, theme key) for a theme, (false, step id) for a step.
        let mut dragging: Option<(bool, String)> = ui.ctx().data(|d| d.get_temp(drag_key));
        // (group, step id, its row).
        let mut open_rows: Vec<(usize, String, egui::Rect)> = vec![];
        // The place each theme with open steps takes, from its heading to its last open row.
        let mut blocks: Vec<(usize, egui::Rect)> = vec![];
        let mut set_assign: Option<(String, Option<crate::auto::Agent>, String)> = None;
        let mut set_assignee: Option<(String, String)> = None;
        let mut last_group = None;
        for (g, i) in rows {
            let group = &groups[g];
            if headed && last_group != Some(g) {
                if last_group.is_some() {
                    ui.add_space(6.0);
                }
                let held = dragging
                    .as_ref()
                    .is_some_and(|(whole, k)| *whole && *k == group.key());
                let heading = ui.horizontal(|ui| {
                    if !ro {
                        let movable = !group.open.is_empty() && open_groups > 1;
                        let h = drag_handle(ui, 18.0, movable, held, t);
                        if movable {
                            if h.drag_started() {
                                dragging = Some((true, group.key()));
                            }
                            h.on_hover_text(crate::tr!(
                                "Glisser pour déplacer tout le thème",
                                "Drag to move the whole theme"
                            ));
                        }
                    }
                    ui.label(
                        RichText::new(
                            group
                                .theme
                                .as_deref()
                                .unwrap_or(crate::tr!("Sans thème", "No theme")),
                        )
                        .size(13.0)
                        .strong()
                        .color(t.secondary),
                    );
                });
                if !group.open.is_empty() {
                    blocks.push((g, heading.response.rect));
                }
                ui.add_space(2.0);
            }
            last_group = Some(g);
            let s = &pr.steps[i];
            let open_key = ("step-open", s.id.as_str());
            let open = flag(ui, open_key);
            let text = split_theme(&s.title).1;
            let held = dragging
                .as_ref()
                .is_some_and(|(whole, id)| !*whole && *id == s.id);
            let color = if s.status == StepStatus::Done {
                t.tertiary
            } else {
                t.text
            };
            let bg = hover_begin(ui);
            let row = ui.horizontal_top(|ui| {
                if !ro {
                    let movable = s.status != StepStatus::Done && group.open.len() > 1;
                    let h = drag_handle(ui, 22.0, movable, held, t);
                    if movable {
                        if h.drag_started() {
                            dragging = Some((false, s.id.clone()));
                        }
                        h.on_hover_text(crate::tr!(
                            "Glisser pour changer l'ordre",
                            "Drag to change the order"
                        ));
                    }
                }
                if step_icon(ui, &s.id, s.status, t).clicked() && !ro {
                    cycle = Some(s.id.clone());
                }
                if !s.assignee.is_empty() {
                    assignee_face(ui, &pr.members, &s.assignee, me, t);
                }
                // Entrusted to an AI: said on the row, in that AI's colour.
                if let Some(ag) = s.agent.filter(|_| s.status != StepStatus::Done) {
                    let (r, g, b) = ag.ai().accent();
                    chip(ui, ag.short_name(), Color32::from_rgb(r, g, b), egui::Sense::hover())
                        .on_hover_text(crate::trf!("Confiée à {}", "Entrusted to {}", ag.label()));
                }
                // The last session sent on it stopped blocked: why, on hover and open.
                if !s.blocked.is_empty() {
                    chip(ui, crate::tr!("Bloquée", "Blocked"), ORANGE, egui::Sense::hover())
                        .on_hover_text(&s.blocked);
                }
                // « Lancer »: a session on this step alone, with its AI. Not on the step a
                // running session is already doing.
                let offer = !ro
                    && autonomous
                    && s.agent.is_some()
                    && s.status != StepStatus::Done
                    && !(running && s.status == StepStatus::Doing)
                    && !sessions.iter().any(|x| x.step == s.id);
                let room = ui.available_width() - if offer { 26.0 + STEP_LAUNCH_W } else { 26.0 };
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
                        if !s.blocked.is_empty() {
                            let why = crate::trf!("Bloquée : {}", "Blocked: {}", s.blocked);
                            ui.add(egui::Label::new(RichText::new(why).size(12.0).color(ORANGE)).wrap());
                        }
                        // Another computer's AI has reserved it (shared project).
                        let held =
                            s.held_by_other_computer(&crate::store::computer_id(), crate::model::now_ms());
                        if !s.source.is_empty() || s.received || held {
                            let by = if s.source.is_empty() {
                                String::new()
                            } else {
                                crate::trf!(
                                    "ajoutée par {} · {}",
                                    "added by {} · {}",
                                    s.source,
                                    relative_time(s.created)
                                )
                            };
                            // Written on another computer of the shared project.
                            let from = if s.received {
                                crate::tr!(
                                    "reçue d'un autre membre du projet",
                                    "received from another member of the project"
                                )
                            } else {
                                ""
                            };
                            let hold = if held {
                                crate::tr!(
                                    "en cours par une IA d'un autre ordinateur",
                                    "in progress by an AI on another computer"
                                )
                            } else {
                                ""
                            };
                            let line = [by.as_str(), from, hold]
                                .into_iter()
                                .filter(|x| !x.is_empty())
                                .collect::<Vec<_>>()
                                .join(" · ");
                            ui.label(RichText::new(line).size(12.0).color(t.tertiary));
                        }
                        // Only where there is someone to name (a team project), or a name
                        // to take off.
                        let people = pr.members.iter().any(|m| m.ai.is_empty());
                        if !ro && s.status != StepStatus::Done && (people || !s.assignee.is_empty())
                        {
                            let mut who = s.assignee.clone();
                            step_assignee_pick(ui, &s.id, &pr.members, me, &mut who, t);
                            if who != s.assignee {
                                set_assignee = Some((s.id.clone(), who));
                            }
                        }
                        if !ro && s.status != StepStatus::Done {
                            let (mut a, mut m) = (s.agent, s.model.clone());
                            step_assign_pick(ui, &s.id, &mut a, &mut m, t);
                            if a != s.agent || m != s.model {
                                set_assign = Some((s.id.clone(), a, m));
                            }
                        }
                        r
                    })
                    .inner
                } else {
                    step_line(ui, job, room)
                };
                // The whole row up to its right edge, where the "×" sits: the cross must stay
                // visible while the pointer is on it, not only over the text.
                let row_rect = egui::Rect::from_min_max(
                    ui.min_rect().min,
                    egui::pos2(ui.max_rect().right(), ui.min_rect().bottom()),
                );
                let hovered = ui.rect_contains_pointer(row_rect) && !ro;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    let rest = if hovered {
                        t.secondary
                    } else {
                        Color32::TRANSPARENT
                    };
                    if icons::delete_cross(
                        ui,
                        20.0,
                        rest,
                        t.text,
                        t.selected,
                        crate::tr!("Supprimer l'étape", "Delete the step"),
                    )
                    .clicked()
                    {
                        remove = Some(s.id.clone());
                    }
                    if offer && let Some(ag) = s.agent {
                        let why = step_launch_block(
                            can_launch,
                            !pr.folder.trim().is_empty(),
                            s.held_by_other_computer(&crate::store::computer_id(), crate::model::now_ms()),
                        );
                        let r = ui.add_enabled(
                            why.is_none(),
                            egui::Button::new(
                                RichText::new(crate::tr!("Lancer", "Start")).size(12.0),
                            )
                            .small(),
                        );
                        let r = r
                            .on_disabled_hover_text(why.unwrap_or_default())
                            .on_hover_text(crate::trf!(
                                "Lancer {} sur cette étape seule, avec les réglages de ce projet.",
                                "Start {} on this step alone, with this project's settings.",
                                ag.label()
                            ));
                        if r.clicked() {
                            actions.push(Action::LaunchStep(page_id.to_string(), s.id.clone()));
                        }
                    }
                });
                resp
            });
            // The sessions at work on this step, right under it: part of its row.
            let mut row_rect = row.response.rect;
            for live in sessions.iter().filter(|x| x.step == s.id) {
                let indent = if ro { 4.0 } else { 26.0 };
                let line = ui.scope(|ui| step_session_line(ui, live, indent, t));
                row_rect = row_rect.union(line.response.rect);
                if line.inner {
                    actions.push(Action::WatchRun(live.run.clone()));
                }
            }
            hover_end(
                ui,
                bg,
                row_rect,
                egui::Id::unique(("step-row", &s.id)),
                t,
            );
            let moving = dragging.as_ref().is_some_and(|(whole, k)| {
                if *whole {
                    *k == group.key()
                } else {
                    *k == s.id
                }
            });
            if moving {
                ui.painter().rect_filled(
                    row_rect.expand2(egui::vec2(6.0, 2.0)),
                    6.0,
                    BLUE.gamma_multiply(0.08),
                );
            }
            if s.status != StepStatus::Done {
                open_rows.push((g, s.id.clone(), row_rect));
                if let Some((_, block)) = blocks.last_mut().filter(|b| b.0 == g) {
                    *block = block.union(row_rect);
                }
            }
            if row.inner.clicked() {
                toggle_flag(ui, open_key);
            }
            ui.add_space(4.0);
        }
        // While dragging: a line where the step would land; it moves there on release.
        // A step stays among its theme's rows; a theme goes between the other themes.
        let mut reorder: Option<Vec<String>> = None;
        let mut move_theme: Option<(Vec<String>, Option<String>)> = None;
        if let Some((whole, d)) = dragging.clone() {
            let released = ui.input(|i| i.pointer.any_released() || !i.pointer.primary_down());
            // The places it can land among: (group or step id, rect).
            let own = open_rows.iter().find(|r| r.1 == d).map(|r| r.0);
            let places: Vec<(usize, &str, egui::Rect)> = if whole {
                blocks.iter().map(|&(g, r)| (g, "", r)).collect()
            } else {
                open_rows
                    .iter()
                    .filter(|r| Some(r.0) == own)
                    .map(|(g, id, r)| (*g, id.as_str(), *r))
                    .collect()
            };
            if let (Some(pos), Some(last)) = (ui.ctx().pointer_latest_pos(), places.last()) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                let target = places.iter().find(|p| pos.y < p.2.center().y);
                let y = target.map_or(last.2.bottom() + 1.0, |p| p.2.top() - 1.0);
                let x = ui.max_rect().x_range();
                ui.painter().hline(x, y, egui::Stroke::new(2.0, BLUE));
                if released && whole {
                    let ids = |g: usize| -> Vec<String> {
                        groups[g].open.iter().map(|&i| pr.steps[i].id.clone()).collect()
                    };
                    let from = blocks.iter().find(|b| groups[b.0].key() == d).map(|b| b.0);
                    if let Some(from) = from.filter(|&f| target.is_none_or(|p| p.0 != f)) {
                        // Before the target theme's first open step, as stored.
                        let before = target.map(|p| ids(p.0).swap_remove(0));
                        move_theme = Some((ids(from), before));
                    }
                } else if let Some(own) = own.filter(|_| released) {
                    let ids: Vec<String> = groups[own]
                        .open
                        .iter()
                        .map(|&i| pr.steps[i].id.clone())
                        .collect();
                    reorder = Some(order_with(&ids, &d, target.map(|p| p.1)));
                }
            }
            if released {
                dragging = None;
            }
            ui.ctx().request_repaint();
        }
        ui.ctx().data_mut(|data| match &dragging {
            Some(d) => {
                data.insert_temp(drag_key, d.clone());
            }
            None => data.remove::<(bool, String)>(drag_key),
        });
        if let Some(order) = reorder {
            pr.reorder_steps(&order);
        }
        if let Some((ids, before)) = move_theme {
            for id in ids {
                pr.move_step(&id, before.as_deref());
            }
        }
        if done > 0 {
            // "Supprimer les terminées": a second click confirms, nothing goes on one slip.
            let confirm_key = ("clear-done", page_id);
            let confirming = flag(ui, confirm_key);
            let mut clear = false;
            ui.horizontal(|ui| {
                if link(
                    ui,
                    &if show_done {
                        crate::tr!("Masquer les étapes terminées", "Hide finished steps")
                            .to_string()
                    } else {
                        if done == 1 {
                            crate::tr!("Voir l'étape terminée", "Show the finished step")
                                .to_string()
                        } else {
                            crate::trf!(
                                "Voir les {done} étapes terminées",
                                "Show the {done} finished steps"
                            )
                        }
                    },
                ) {
                    toggle_flag(ui, ("done-open", page_id));
                }
                if ro {
                    return;
                }
                if confirming {
                    let sure = if done == 1 {
                        crate::tr!(
                            "Confirmer : supprimer 1 étape terminée",
                            "Confirm: delete 1 finished step"
                        )
                        .to_string()
                    } else {
                        crate::trf!(
                            "Confirmer : supprimer {done} étapes terminées",
                            "Confirm: delete {done} finished steps"
                        )
                    };
                    if ui
                        .add(
                            egui::Label::new(
                                RichText::new(sure)
                                    .size(13.0)
                                    .color(super::style::RED),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        clear = true;
                        toggle_flag(ui, confirm_key);
                    }
                    if quiet_link(ui, crate::tr!("Annuler", "Cancel"), t).clicked() {
                        toggle_flag(ui, confirm_key);
                    }
                } else if quiet_link(
                    ui,
                    crate::tr!("Supprimer les terminées", "Delete finished ones"),
                    t,
                )
                .clicked()
                {
                    toggle_flag(ui, confirm_key);
                }
            });
            if clear {
                pr.remove_done_steps();
            }
        }
        if let Some(id) = cycle {
            let cur = pr
                .steps
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.status)
                .unwrap_or_default();
            let next = match cur {
                StepStatus::Todo => StepStatus::Doing,
                StepStatus::Doing => StepStatus::Done,
                StepStatus::Done => StepStatus::Todo,
            };
            pr.set_step_status(&id, next);
        }
        if let Some(id) = remove {
            pr.remove_step(&id);
        }
        if let Some((id, a, m)) = set_assign {
            pr.set_step_agent(&id, a);
            pr.set_step_model(&id, &m);
        }
        if let Some((id, who)) = set_assignee {
            pr.set_step_assignee(&id, &who);
        }
        // "+ Ajouter une étape": a quiet line aligned with the steps; click to type.
        if ro {
            return;
        }
        ui.add_space(2.0);
        let adding_key = ("adding-step", page_id);
        // Arriving from « Ajouter des tâches »: the field is open, with a word on what to do.
        let arrived = steps_focus_age(ui.ctx(), page_id).filter(|a| *a < STEPS_HINT_SECS);
        if arrived.is_some_and(|a| a < 0.6) && !flag(ui, adding_key) {
            toggle_flag(ui, adding_key);
        }
        let adding = flag(ui, adding_key);
        if arrived.is_some() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new("✎").size(14.0).color(GREEN));
                ui.label(
                    RichText::new(crate::tr!(
                        "Tapez vos tâches ici, une à la fois puis Entrée. L'IA en cours les prendra au tour suivant, sans s'arrêter.",
                        "Type your tasks here, one at a time then Enter. The running AI takes them on its next turn, without stopping."
                    ))
                    .size(12.5)
                    .color(GREEN),
                );
            });
            ui.add_space(2.0);
        }
        // A row exactly as tall as the « + »: the text typed (or the label) is centred on it.
        inline_row(ui, |ui| {
            let (r, plus) =
                ui.allocate_exact_size(egui::vec2(18.0, INLINE_H), egui::Sense::click());
            let hover_row = ui.rect_contains_pointer(egui::Rect::from_min_size(
                r.min,
                egui::vec2(ui.available_width() + 18.0, 22.0),
            ));
            let col = if adding || hover_row {
                t.secondary
            } else {
                t.tertiary
            };
            let c = r.center();
            ui.painter().line_segment(
                [c - egui::vec2(5.0, 0.0), c + egui::vec2(5.0, 0.0)],
                egui::Stroke::new(1.5, col),
            );
            ui.painter().line_segment(
                [c - egui::vec2(0.0, 5.0), c + egui::vec2(0.0, 5.0)],
                egui::Stroke::new(1.5, col),
            );
            if adding {
                // A fixed id: an automatic one follows the widgets drawn above, so a step
                // added (by Enter, or by an AI), a step done, or the arrival hint fading would
                // give the field a new id and take its focus away while typing.
                let edit = ui.add(
                    inline_edit(egui::TextEdit::singleline(new_step))
                        .id(new_step_edit_id(page_id))
                        .hint_text(if arrived.is_some() {
                            crate::tr!("Votre tâche, puis Entrée", "Your task, then Enter")
                        } else {
                            crate::tr!("Nouvelle étape, puis Entrée", "New step, then Enter")
                        })
                        .desired_width(f32::INFINITY),
                );
                if !edit.has_focus()
                    && !edit.lost_focus()
                    && new_step.is_empty()
                    && !ui.ctx().memory(|m| m.has_focus(edit.id))
                {
                    edit.request_focus();
                }
                let enter = ui.input(|i| i.key_pressed(Key::Enter));
                let esc = ui.input(|i| i.key_pressed(Key::Escape));
                if edit.lost_focus() && enter && !new_step.trim().is_empty() {
                    pr.add_step(new_step, crate::project::SOURCE_PERSON);
                    new_step.clear();
                    edit.request_focus();
                } else if esc || (edit.lost_focus() && new_step.trim().is_empty()) {
                    new_step.clear();
                    toggle_flag(ui, adding_key);
                }
            } else {
                let lbl = ui.add(
                    egui::Label::new(
                        RichText::new(crate::tr!("Ajouter une étape", "Add a step"))
                            .size(TEXT)
                            .color(col),
                    )
                    .sense(egui::Sense::click()),
                );
                if lbl.clicked() || plus.clicked() {
                    toggle_flag(ui, adding_key);
                }
            }
        });
        // The AI's tools for the steps, set apart from the list by a thin line.
        card_rule(ui, t);
        ai_steps(ui, page_id, pr, ai, prefer, t);
    });
    // Arriving from « Ajouter des tâches »: the card glows green, then fades.
    if let Some(glow) = steps_focus_age(ui.ctx(), page_id)
        .map(steps_glow)
        .filter(|g| *g > 0.0)
    {
        let rect = egui::Rect::from_min_size(start, egui::vec2(width, h + extra.max(0.0)));
        let p = ui.painter();
        p.rect_filled(rect, 12.0, GREEN.gamma_multiply(0.07 * glow));
        p.rect_stroke(
            rect,
            12.0,
            egui::Stroke::new(2.0, GREEN.gamma_multiply(glow)),
            egui::StrokeKind::Inside,
        );
        p.rect_stroke(
            rect.expand(3.0),
            15.0,
            egui::Stroke::new(3.0, GREEN.gamma_multiply(0.25 * glow)),
            egui::StrokeKind::Inside,
        );
        ui.ctx().request_repaint();
    }
    h
}

/// The name of a step's responsible person, « (vous) » for oneself; None when `id` is
/// nobody among the members (gone from the space, or not received yet).
fn assignee_name(members: &[ProjectMember], id: &str, me: &str) -> Option<String> {
    let m = members.iter().find(|m| m.id == id && m.ai.is_empty())?;
    Some(if m.id == me {
        crate::trf!("{} (vous)", "{} (you)", m.name)
    } else {
        m.name.clone()
    })
}

fn unknown_assignee() -> &'static str {
    crate::tr!("Membre inconnu", "Unknown member")
}

/// The responsible person on a step's row: their initial in their colour (a grey « ? » for
/// someone not among the members), their name on hover.
fn assignee_face(ui: &mut egui::Ui, members: &[ProjectMember], id: &str, me: &str, t: &Tone) {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::hover());
    let name = assignee_name(members, id, me);
    // The colour follows the name alone, as on the members card.
    let (color, initial) = match members.iter().find(|m| m.id == id && m.ai.is_empty()) {
        Some(m) => (
            super::side::person_color(&m.name),
            m.name.chars().next().map(|c| c.to_uppercase().collect()),
        ),
        None => (t.tertiary, Some("?".to_string())),
    };
    ui.painter().circle_filled(r.center(), 9.0, color);
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        initial.unwrap_or_default(),
        semibold(10.5),
        Color32::WHITE,
    );
    let said = crate::trf!(
        "Responsable : {}",
        "Responsible: {}",
        name.as_deref().unwrap_or(unknown_assignee())
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &said));
    resp.on_hover_text(said);
}

/// « Responsable »: the person in charge of a step, among the project's people (its AIs are
/// chosen with « Confiée à »).
fn step_assignee_pick(
    ui: &mut egui::Ui,
    step_id: &str,
    members: &[ProjectMember],
    me: &str,
    who: &mut String,
    t: &Tone,
) {
    let nobody = crate::tr!("Personne", "Nobody");
    let shown = if who.is_empty() {
        nobody.to_string()
    } else {
        assignee_name(members, who, me).unwrap_or_else(|| unknown_assignee().to_string())
    };
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(crate::tr!("Responsable", "Responsible"))
                .size(12.0)
                .color(t.tertiary),
        );
        egui::ComboBox::from_id_salt(("step-assignee", step_id))
            .selected_text(RichText::new(shown).size(12.0))
            .width(170.0)
            .show_ui(ui, |ui| {
                ui.set_min_width(200.0);
                if ui
                    .add(egui::Button::selectable(who.is_empty(), nobody))
                    .clicked()
                {
                    who.clear();
                }
                for m in members.iter().filter(|m| m.ai.is_empty()) {
                    let name = assignee_name(members, &m.id, me).unwrap_or_default();
                    if ui
                        .add(egui::Button::selectable(*who == m.id, name))
                        .clicked()
                    {
                        *who = m.id.clone();
                    }
                }
            })
            .response
            .on_hover_text(crate::tr!(
                "La personne responsable de cette étape, vue par toute l'équipe.",
                "The person responsible for this step, seen by the whole team."
            ));
    });
}

/// « Confiée à »: who takes a step, in one menu. « Automatique » (the project's choice:
/// its AI, or Orchestra's pick), an AI with its model chosen by Orchestra, or an AI and one
/// of its models. Only the AIs able to work alone are offered.
fn step_assign_pick(
    ui: &mut egui::Ui,
    step_id: &str,
    agent: &mut Option<crate::auto::Agent>,
    model: &mut String,
    t: &Tone,
) {
    use crate::auto::Agent;
    let shown = match (&agent, model.is_empty()) {
        (None, true) => crate::tr!("Automatique", "Automatic").to_string(),
        (None, false) => model.clone(),
        (Some(a), true) => a.label().to_string(),
        (Some(a), false) => format!("{} · {}", a.label(), model),
    };
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(crate::tr!("Confiée à", "Entrusted to"))
                .size(12.0)
                .color(t.tertiary),
        );
        egui::ComboBox::from_id_salt(("step-assign", step_id))
            .selected_text(RichText::new(shown).size(12.0))
            .width(170.0)
            .show_ui(ui, |ui| {
                ui.set_min_width(240.0);
                if ui
                    .add(egui::Button::selectable(
                        agent.is_none() && model.is_empty(),
                        crate::tr!("✨ Automatique", "✨ Automatic"),
                    ))
                    .on_hover_text(crate::tr!(
                        "Comme le projet : son IA, ou celle qu'Orchestra choisit au lancement.",
                        "As the project: its AI, or the one Orchestra picks at the start."
                    ))
                    .clicked()
                {
                    *agent = None;
                    model.clear();
                }
                // Read only while open: the lists come from the AIs' files.
                for a in Agent::ALL.into_iter().filter(|a| a.ready()) {
                    ui.separator();
                    let whole = *agent == Some(a) && model.is_empty();
                    if ui
                        .add(egui::Button::selectable(
                            whole,
                            RichText::new(a.label()).strong(),
                        ))
                        .on_hover_text(crate::tr!(
                            "Cette IA, le modèle choisi par Orchestra",
                            "This AI, the model picked by Orchestra"
                        ))
                        .clicked()
                    {
                        *agent = Some(a);
                        model.clear();
                    }
                    for m in crate::catalog::available(a) {
                        let on = *agent == Some(a) && *model == m.id;
                        let text = RichText::new(format!("    {} · {}", m.label, m.tier.label()))
                            .size(12.5);
                        let r = ui.add_enabled(!m.needs_higher_plan, egui::Button::selectable(on, text));
                        if r.clicked() {
                            *agent = Some(a);
                            *model = m.id.clone();
                        }
                    }
                }
            })
            .response
            .on_hover_text(crate::tr!(
                "L'IA (et le modèle) de la session qui prendra cette étape. « Automatique » : comme le projet.",
                "The AI (and model) of the session that takes this step. « Automatic »: as the project."
            ));
    });
}

/// How strongly the Steps card glows `age` seconds after arriving from « Ajouter des
/// tâches »: fully for 2.5 s, then fading out over 1.5 s.
fn steps_glow(age: f64) -> f32 {
    const HOLD: f64 = 2.5;
    const FADE: f64 = 1.5;
    if !(0.0..HOLD + FADE).contains(&age) {
        return 0.0;
    }
    if age < HOLD {
        1.0
    } else {
        (1.0 - (age - HOLD) / FADE) as f32
    }
}

#[cfg(test)]
mod session_pick_tests {
    use super::{Color32, new_session_pick};

    #[test]
    fn the_new_session_line_offers_an_ai_then_its_models() {
        let l = [
            ("Claude Code", Some("claude"), Color32::RED),
            ("Mistral Vibe", Some("vibe"), Color32::BLUE),
            ("Shell", None, Color32::GRAY),
        ];
        // Nothing chosen yet: the first AI, its default model.
        let p = new_session_pick(&l, "", "");
        assert_eq!((p.label, p.cmd, p.model.as_str()), ("Claude Code", Some("claude"), ""));
        assert!(p.models.contains(&"opus"));
        // A model it offers is kept; another AI's model goes back to « par défaut ».
        assert_eq!(new_session_pick(&l, "Claude Code", "sonnet").model, "sonnet");
        assert_eq!(new_session_pick(&l, "Claude Code", "gpt-5").model, "");
        // An AI without a model list (Vibe, the shell): only « par défaut ».
        let p = new_session_pick(&l, "Mistral Vibe", "opus");
        assert!(p.models.is_empty() && p.model.is_empty());
        assert_eq!(new_session_pick(&l, "Shell", "").cmd, None);
        // An AI unplugged since: the first one again.
        assert_eq!(new_session_pick(&l, "Codex", "gpt-5").label, "Claude Code");
        assert_eq!(new_session_pick(&[], "Codex", "").label, "Shell");
        // The command: --model for a chosen model, nothing added for « par défaut ».
        assert_eq!(new_session_pick(&l, "Claude Code", "").command().as_deref(), Some("claude"));
        assert_eq!(
            new_session_pick(&l, "Claude Code", "opus").command().as_deref(),
            Some("claude --model opus")
        );
        // A model not offered never reaches the command line.
        assert_eq!(
            new_session_pick(&l, "Claude Code", "x; rm -rf ~").command().as_deref(),
            Some("claude")
        );
        assert_eq!(new_session_pick(&l, "Mistral Vibe", "opus").command().as_deref(), Some("vibe"));
        assert_eq!(new_session_pick(&l, "Shell", "opus").command(), None);
        // Every AI that opens in a terminal knows its models (possibly none).
        for a in crate::setup::Ai::ALL {
            assert_eq!(crate::setup::Ai::from_label(a.label()), Some(a));
            assert!(a.models().iter().all(|m| !m.is_empty() && !m.contains(' ')));
        }
    }
}

#[cfg(test)]
mod step_group_tests {
    use super::{StepStatus, order_with, split_theme, step_groups};
    use crate::project::Project;

    #[test]
    fn a_step_s_theme_is_what_comes_before_its_colon() {
        assert_eq!(split_theme("Ergonomie : réduire le titre"), (Some("Ergonomie"), "réduire le titre"));
        assert_eq!(split_theme("Écriture simultanée en équipe: tester"), (Some("Écriture simultanée en équipe"), "tester"));
        for plain in [
            "Corriger le bouton",
            "Voir https://exemple.org/a",
            "Réunion à 10:30",
            "Corriger (voir src/a.rs) : le bouton",
            "Un début de phrase bien trop long : suite",
            "Vide : ",
        ] {
            assert_eq!(split_theme(plain), (None, plain), "{plain}");
        }
    }

    /// Each theme once, in the order of its first open step, whatever its case; its
    /// finished steps with it; the themes all finished come last.
    #[test]
    fn steps_are_grouped_by_theme() {
        let mut p = Project::default();
        let ids: Vec<String> = [
            "Vieux : fini",
            "A : un",
            "sans thème",
            "B : deux",
            "a : trois",
            "B : quatre",
        ]
        .iter()
        .map(|t| p.add_step(t, "moi"))
        .collect();
        p.set_step_status(&ids[0], StepStatus::Done);
        p.set_step_status(&ids[3], StepStatus::Done);
        let groups: Vec<_> = step_groups(&p.steps)
            .into_iter()
            .map(|g| (g.theme, g.open, g.done))
            .collect();
        let name = |s: &str| Some(s.to_string());
        assert_eq!(
            groups,
            [
                (name("A"), vec![1, 4], vec![]),
                (None, vec![2], vec![]),
                (name("B"), vec![5], vec![3]),
                (name("Vieux"), vec![], vec![0]),
            ]
        );
    }

    #[test]
    fn a_dragged_step_lands_among_its_theme_s_steps() {
        let ids: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(order_with(&ids, "c", Some("a")), ["c", "a", "b"]);
        assert_eq!(order_with(&ids, "a", None), ["b", "c", "a"]);
        assert_eq!(order_with(&ids, "b", Some("b")), ids);
    }
}

#[cfg(test)]
mod new_step_field_tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// Drags what is left of the text `from` (its handle, `left` points before it) to the
    /// height of `to`, `dy` points off its middle.
    fn drag_by_handle(h: &mut Harness<S>, from: egui::Rect, left: f32, to: egui::Rect, dy: f32) {
        let start = egui::pos2(from.left() - left, from.center().y);
        let end = egui::pos2(start.x, to.center().y + dy);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        for event in [
            egui::Event::PointerMoved(start),
            button(start, true),
            egui::Event::PointerMoved(start.lerp(end, 0.5)),
            egui::Event::PointerMoved(end),
            button(end, false),
        ] {
            h.event(event);
            h.step();
        }
        h.run();
    }

    /// A theme's heading drags all its steps above another theme.
    #[test]
    fn dragging_a_heading_moves_the_whole_theme() {
        let mut h = harness(|ui, cx| {
            steps_card(ui, cx, 0.0);
        });
        for title in ["Membres : lister", "Modules : masquer", "Membres : inviter", "Modules : activer"] {
            h.state_mut().pr.add_step(title, "moi");
        }
        h.run();
        let (from, to) = (h.get_by_label("Modules").rect(), h.get_by_label("Membres").rect());
        let gap = h.ctx.global_style().spacing.item_spacing.x;
        drag_by_handle(&mut h, from, gap + 5.0, to, -4.0);
        let titles: Vec<&str> = h.state().pr.steps.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Modules : masquer", "Modules : activer", "Membres : lister", "Membres : inviter"]
        );
        assert!(h.get_by_label("Modules").rect().top() < h.get_by_label("Membres").rect().top());
    }

    /// The theme is a heading over its steps, said once, not repeated on each row.
    #[test]
    fn the_steps_card_heads_each_theme_once() {
        let mut h = harness(|ui, cx| {
            steps_card(ui, cx, 0.0);
        });
        for title in ["Membres : lister", "Modules : masquer", "Membres : inviter"] {
            h.state_mut().pr.add_step(title, "moi");
        }
        h.run();
        // `get_by_label` fails when two widgets carry the same text.
        h.get_by_label("Membres");
        h.get_by_label("Modules");
        assert!(h.query_by_label("Sans thème").is_none());
        // A step without a theme, beside themed ones, gets its own heading.
        h.state_mut().pr.add_step("Relire", "moi");
        h.run();
        h.get_by_label("Sans thème");
    }

    /// A step too long for its line: hovering shows all of it, a click unfolds it.
    #[test]
    fn a_cut_step_is_read_whole_on_hover_or_click() {
        let mut h = harness(|ui, cx| {
            steps_card(ui, cx, 0.0);
        });
        let long = "afficher le texte complet d'une étape bien trop longue pour tenir sur la \
                    seule ligne que la carte lui laisse, jusqu'au tout dernier mot";
        h.state_mut().pr.add_step(long, "moi");
        h.state_mut().pr.add_step("courte", "moi");
        h.run();
        // The folded row and, once hovered, its tooltip both carry the text.
        let shown = |h: &Harness<S>| h.query_all_by_label(long).count();
        let folded = h.get_by_label(long).rect();
        h.get_by_label(long).hover();
        h.run();
        assert_eq!(shown(&h), 2, "the tooltip shows the whole step");
        // A line that fits has nothing more to say.
        h.get_by_label("courte").hover();
        h.run();
        h.get_by_label("courte");
        h.get_by_label(long).click();
        h.run();
        h.event(egui::Event::PointerGone);
        h.run();
        let open = h.get_by_label(long).rect();
        assert!(open.height() > folded.height() * 1.5, "{folded:?} {open:?}");
        assert!(open.right() <= 700.0, "{open:?}");
    }

    /// A greyed « Lancer une IA » says why on the page; with no step to take, a link
    /// opens the field to add one.
    #[test]
    fn a_greyed_launch_button_says_why_on_the_page() {
        use super::{LaunchBlock, launch_block};
        let mut pr = Project::default();
        assert_eq!(launch_block(false, false, &pr), Some(LaunchBlock::NoAi));
        assert_eq!(launch_block(true, false, &pr), Some(LaunchBlock::NoFolder));
        assert_eq!(launch_block(true, true, &pr), Some(LaunchBlock::NoStep));
        pr.add_step("Relire", "moi");
        assert_eq!(launch_block(true, true, &pr), None);
        let id = pr.steps[0].id.clone();
        pr.set_step_status(&id, crate::project::StepStatus::Done);
        assert_eq!(launch_block(true, true, &pr), Some(LaunchBlock::AllDone));

        // The line and the card it leads to (the whole panel needs the system's emoji).
        let mut h = harness(|ui, cx| {
            if let Some(why) = launch_block(true, true, cx.pr)
                && super::launch_block_line(ui, why, cx.t)
            {
                super::open_new_step(ui.ctx(), cx.page_id);
            }
            steps_card(ui, cx, 0.0);
        });
        h.run();
        h.get_by_label(LaunchBlock::NoStep.why());
        h.get_by_label("+ Ajouter une étape").click();
        h.run();
        type_text(&mut h, "Relire");
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(h.state().pr.steps.len(), 1);
        // A step to take: the button is ready, the reason is gone.
        assert!(h.query_by_label(LaunchBlock::NoStep.why()).is_none());
    }

    /// A step says who is responsible for it on its row; unfolded, a menu gives it to one
    /// of the project's people (not its AIs) or takes it back.
    #[test]
    fn a_step_shows_its_responsible_person_and_lets_one_be_picked() {
        let mut h = harness(|ui, cx| {
            cx.team.me = "b".into();
            steps_card(ui, cx, 0.0);
        });
        let member = |id: &str, name: &str, ai: &str| crate::project::ProjectMember {
            id: id.into(),
            name: name.into(),
            role: Default::default(),
            ai: ai.into(),
        };
        h.state_mut().pr.members = vec![
            member("a", "Ana", ""),
            member("b", "Bob", ""),
            member("ia:Maison", "Maison", "codex"),
        ];
        h.state_mut().pr.add_step("Relire", "moi");
        h.run();
        assert!(h.query_by_label("Responsable").is_none(), "not before unfolding");
        h.get_by_label("Relire").click();
        h.run();
        h.get_by_label("Responsable");
        h.get_by_value("Personne").click();
        h.run();
        h.get_by_label("Ana");
        assert!(h.query_by_label("Maison").is_none(), "AIs aren't offered");
        h.get_by_label("Bob (vous)").click();
        h.run();
        assert_eq!(h.state().pr.steps[0].assignee, "b");
        h.get_by_label("Responsable : Bob (vous)");
        // Folded again, the row still says it.
        h.get_by_label("Relire").click();
        h.run();
        h.get_by_label("Responsable : Bob (vous)");
        // Gone from the space: said as unknown, and can still be taken off.
        h.state_mut().pr.members.retain(|m| m.id != "b");
        h.run();
        h.get_by_label("Responsable : Membre inconnu");
        h.get_by_label("Relire").click();
        h.run();
        h.get_by_value("Membre inconnu").click();
        h.run();
        h.get_by_label("Personne").click();
        h.run();
        assert!(h.state().pr.steps[0].assignee.is_empty());
        assert!(h.query_by_label("Responsable : Membre inconnu").is_none());
    }

    /// « Lancer » is on the open steps entrusted to an AI only, and asks for a session on
    /// that step.
    #[test]
    fn a_step_entrusted_to_an_ai_can_be_started_from_its_row() {
        let mut h = harness(|ui, cx| {
            steps_card(ui, cx, 0.0);
        });
        let pr = &mut h.state_mut().pr;
        pr.folder = "/tmp/projet".into();
        pr.add_step("Relire", "moi");
        let id = pr.add_step("Tester", "moi");
        h.run();
        assert!(h.query_by_label("Lancer").is_none(), "no AI, no button");
        h.state_mut().pr.set_step_agent(&id, Some(crate::auto::Agent::Codex));
        h.run();
        h.get_by_label("Lancer").click();
        h.run();
        assert!(h.state().actions.iter().any(
            |a| matches!(a, Action::LaunchStep(p, s) if p == "p1" && *s == id)
        ));
        h.state_mut().pr.set_step_status(&id, crate::project::StepStatus::Done);
        h.run();
        assert!(h.query_by_label("Lancer").is_none(), "finished");
    }

    /// A session at work on a step shows live under its row, folded or not: where it
    /// stands, its terminal one click away, and no second « Lancer » meanwhile.
    #[test]
    fn a_step_shows_the_session_at_work_on_it() {
        let mut h = harness(|ui, cx| {
            steps_card(ui, cx, 0.0);
        });
        let pr = &mut h.state_mut().pr;
        pr.folder = "/tmp/projet".into();
        let id = pr.add_step("Tester", "moi");
        pr.set_step_agent(&id, Some(crate::auto::Agent::Codex));
        h.run();
        h.get_by_label("Lancer");
        assert!(h.query_by_label("Regarder").is_none(), "no session, no line");
        h.state_mut().sessions = vec![StepSession {
            run: "r1".into(),
            step: id.clone(),
            state: "tour 2".into(),
            detail: "Lance les tests".into(),
            waiting: false,
        }];
        // The dot breathes: the card never settles, so a few frames rather than `run`.
        h.run_steps(3);
        h.get_by_label("tour 2 · Lance les tests");
        assert!(h.query_by_label("Lancer").is_none(), "already at work");
        h.get_by_label("Regarder").click();
        h.run_steps(3);
        assert!(h.state().actions.iter().any(
            |a| matches!(a, Action::WatchRun(r) if r == "r1")
        ));
        // Paused: the reason alone.
        let live = &mut h.state_mut().sessions[0];
        live.waiting = true;
        live.state = "en pause".into();
        live.detail = "En pause : Claude Code demande une autorisation.".into();
        h.run_steps(3);
        h.get_by_label("En pause : Claude Code demande une autorisation.");
        // On another step, nothing here.
        h.state_mut().sessions[0].step = "ailleurs".into();
        h.run_steps(3);
        assert!(h.query_by_label("Regarder").is_none());
        h.get_by_label("Lancer");
    }

    /// The « Nouveau projet » form offers every template, or none, and the project it
    /// creates is the one of the choice.
    #[test]
    fn the_new_project_form_offers_the_templates() {
        let mut h = Harness::builder().with_size([700.0, 200.0]).build_ui_state(
            |ui, form: &mut NewProject| {
                // The emojis come from the system's fonts, which the tests don't load.
                ui.ctx()
                    .set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
                template_picker(ui, &mut form.template, &tone(true));
            },
            NewProject::default(),
        );
        h.run();
        assert_eq!(h.state().template, None);
        for template in Template::ALL {
            h.get_by_label(&format!("{} {}", template.icon(), template.label()))
                .click();
            h.run();
            assert_eq!(h.state().template, Some(template));
            // What the choice gives is said under it.
            h.get_by_label_contains(template.description());
        }
        let mut ws = Workspace::default();
        let event = h.state().create_in(&mut ws);
        let pages = ws.pages.iter().filter(|p| p.parent.as_ref() == Some(&event));
        assert_eq!(pages.count(), Template::Event.pages().len());

        h.get_by_label(crate::tr!("Aucun", "None")).click();
        h.run();
        assert_eq!(h.state().template, None);
        let bare = h.state().create_in(&mut ws);
        assert!(ws.pages.iter().all(|p| p.parent.as_ref() != Some(&bare)));
        assert_eq!(ws.page(&bare).unwrap().project.as_ref().unwrap().code, None);
    }

    /// The project's settings turn modules off and on, several in a row from the « … » menu,
    /// and give them back to their defaults.
    #[test]
    fn the_project_settings_turn_modules_on_and_off() {
        let mut pr = Project::new("");
        pr.code = Some(true);
        let mut h = Harness::builder().with_size([700.0, 700.0]).build_ui_state(
            |ui, pr: &mut Project| {
                let open = ui.button("…");
                egui::Popup::menu(&open).show(|ui| {
                    super::project_menu(ui, pr, &mut None, &mut false);
                });
            },
            pr,
        );
        h.get_by_label("…").click();
        h.run();
        h.get_by_label_contains("Modules").click();
        h.run();
        assert!(h.query_by_label("Revenir à l'automatique").is_none());
        h.get_by_label("Terminal").click();
        h.run();
        assert!(!h.state().has_module(Module::Terminal));
        // The menu stayed open: another box, without opening it again.
        h.get_by_label("Git").click();
        h.run();
        assert!(!h.state().has_module(Module::Git));
        // Versions can't be ticked without Git, and its own choice is left alone.
        h.get_by_label("Versions").click();
        h.run();
        assert!(h.state().has_module(Module::Versions));
        h.get_by_label("Revenir à l'automatique").click();
        h.run();
        assert_eq!(h.state().modules(), Module::ALL);
        assert!(h.state().module_choices.is_empty());
    }

    /// With the autonomous module off, a step entrusted to an AI has no « Lancer ».
    #[test]
    fn a_project_without_the_autonomous_module_offers_no_step_launch() {
        let mut h = harness(|ui, cx| {
            steps_card(ui, cx, 0.0);
        });
        let pr = &mut h.state_mut().pr;
        pr.folder = "/tmp/projet".into();
        let id = pr.add_step("Tester", "moi");
        pr.set_step_agent(&id, Some(crate::auto::Agent::Codex));
        pr.set_module(crate::project::Module::Autonomous, Some(false));
        h.run();
        h.get_by_label("Tester");
        assert!(h.query_by_label("Lancer").is_none());
        h.state_mut()
            .pr
            .set_module(crate::project::Module::Autonomous, None);
        h.run();
        h.get_by_label("Lancer");
    }

    /// Without a team there is nobody to name: no « Responsable » menu.
    #[test]
    fn a_project_without_members_offers_no_responsible_person() {
        let mut h = harness(|ui, cx| {
            steps_card(ui, cx, 0.0);
        });
        h.state_mut().pr.add_step("Relire", "moi");
        h.run();
        h.get_by_label("Relire").click();
        h.run();
        h.get_by_label("Confiée à");
        assert!(h.query_by_label("Responsable").is_none());
    }

    #[derive(Default)]
    struct S {
        pr: Project,
        new_step: String,
        vers: super::super::versions::Versions,
        ai: super::super::ai_fill::AiJobs,
        actions: Vec<Action>,
        sessions: Vec<StepSession>,
        answered: Option<(String, String)>,
        /// Drawn only once the app's fonts are in (the card's labels need them).
        draw: bool,
    }

    fn type_text(h: &mut Harness<S>, s: &str) {
        h.event(egui::Event::Text(s.into()));
        h.run();
    }

    /// One card of project « p1 » drawn by `draw`, with the app's fonts.
    fn harness(draw: fn(&mut egui::Ui, &mut Cx)) -> Harness<'static, S> {
        let t = tone(false);
        let mut h = Harness::builder().with_size([700.0, 700.0]).build_ui_state(
            move |ui, s: &mut S| {
                if !s.draw {
                    return;
                }
                let mut cx = Cx {
                    page_id: "p1",
                    title: "Projet".into(),
                    pr: &mut s.pr,
                    new_step: &mut s.new_step,
                    t: &t,
                    live: &[],
                    pages: &[],
                    actions: &mut s.actions,
                    msg: None,
                    reviewing: false,
                    running: false,
                    step_sessions: s.sessions.clone(),
                    can_launch: true,
                    peers: vec![],
                    vers: &mut s.vers,
                    launchers: vec![],
                    ai: &mut s.ai,
                    questions: &[],
                    answered: &mut s.answered,
                    team: Default::default(),
                };
                draw(ui, &mut cx);
            },
            S::default(),
        );
        setup_fonts(&h.ctx);
        h.run();
        h.state_mut().draw = true;
        h
    }

    /// An answered quick question offers to carry the conversation on: the button asks for
    /// the AI's conversation in the right-hand panel (or a terminal, failing that).
    #[test]
    fn a_quick_answer_can_be_carried_on_in_the_side_panel() {
        let mut h = harness(quick_question);
        let answer = |resume| super::super::ai_fill::QuickAnswer {
            question: "Faut-il un mode hors ligne ?".into(),
            answer: Ok("Oui, petit changement.".into()),
            by: "Claude Code".into(),
            steps: 0,
            resume,
            tool: None,
            earlier: vec![],
        };
        // No conversation id given back: no button.
        h.state_mut().ai.answers.insert("p1".into(), answer(None));
        h.run();
        assert!(h.query_by_label("Poursuivre la conversation").is_none());
        let resume = crate::sync::Resume {
            session: "0199a2b3-c4d5-7e6f-8a9b-0c1d2e3f4a5b".into(),
            cwd: "/tmp/projet".into(),
            cmd: "claude --resume 0199a2b3-c4d5-7e6f-8a9b-0c1d2e3f4a5b".into(),
        };
        h.state_mut()
            .ai
            .answers
            .insert("p1".into(), answer(Some(resume.clone())));
        h.run();
        h.get_by_label("Poursuivre la conversation").click();
        h.run();
        let req = h
            .ctx
            .data_mut(|d| {
                d.remove_temp::<super::super::side::ConversationRequest>(egui::Id::unique(
                    "side-conversation-request",
                ))
            })
            .expect("the conversation is asked for in the side panel");
        assert_eq!(req.project, "p1");
        assert_eq!(req.resume, resume);
        assert_eq!(req.title, "Faut-il un mode hors ligne ?");
    }

    /// The « Ajouter une étape » field keeps the focus while steps are added above it
    /// (by Enter, or by an AI meanwhile): what is typed goes on landing in it.
    #[test]
    fn typing_a_step_is_not_cut_when_the_list_above_changes() {
        let mut h = harness(|ui, cx| {
            steps_card(ui, cx, 0.0);
        });
        // Open the field, as a click on « Ajouter une étape » does.
        h.ctx
            .data_mut(|d| d.insert_temp(egui::Id::unique(("adding-step", "p1")), true));
        h.run();
        type_text(&mut h, "Première");
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(h.state().pr.steps.len(), 1);
        // Right after Enter, the next step is typed in the same field.
        type_text(&mut h, "Deux");
        assert_eq!(h.state().new_step, "Deux");
        // An AI adds a step above while the person types: the typing goes on.
        h.state_mut().pr.add_step("Étape d'une IA", "claude-code");
        h.run();
        type_text(&mut h, "ième");
        assert_eq!(h.state().new_step, "Deuxième");
        h.key_press(egui::Key::Enter);
        h.run();
        let titles: Vec<&str> = h.state().pr.steps.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Première", "Étape d'une IA", "Deuxième"]);
    }
}

#[cfg(test)]
mod card_layout_tests {
    use super::{Module, ProjectTab, project_tabs, tab_of_mark};

    /// The sessions are grouped by AI provider, in the order each first comes (the live
    /// ones first), so each gets a box of its own.
    #[test]
    fn sessions_are_grouped_by_provider() {
        use super::provider_groups;
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let g = provider_groups(&s(&["Claude Code", "Codex", "Claude Code"]), &s(&["Gemini", "Codex"]));
        let labels: Vec<&str> = g.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(labels, ["Claude Code", "Codex", "Gemini"]);
        assert_eq!((g[0].1.clone(), g[0].2.clone()), (vec![0, 2], vec![]));
        assert_eq!((g[1].1.clone(), g[1].2.clone()), (vec![1], vec![1]));
        assert_eq!((g[2].1.clone(), g[2].2.clone()), (vec![], vec![0]));
        // One provider only: one group (its rows go straight in the card).
        assert_eq!(provider_groups(&s(&["Claude Code"]), &s(&["Claude Code"])).len(), 1);
        assert!(provider_groups(&[], &[]).is_empty());
    }

    /// The page opens on the overview; versions and the watch only with their modules (a
    /// project without code has none by itself); a tour pointing at a card finds it in its
    /// tab.
    #[test]
    fn project_tabs_follow_the_kind_of_project() {
        use ProjectTab::*;
        let mut pr = crate::project::Project::new("");
        pr.code = Some(true);
        assert_eq!(project_tabs(false, &pr.modules()), [Overview, Versions, Watch, Pages, Review]);
        pr.code = Some(false);
        assert_eq!(project_tabs(false, &pr.modules()), [Overview, Pages, Review]);
        assert_eq!(project_tabs(true, &pr.modules()), [Overview, Pages, Review, Members]);
        // A module turned off takes its tabs with it; versions need git too.
        assert_eq!(project_tabs(false, &[Module::Git]), [Overview, Watch, Pages, Review]);
        assert_eq!(project_tabs(false, &[Module::Versions]), [Overview, Pages, Review]);
        assert_eq!(tab_of_mark("card:members"), Some(Members));
        assert_eq!(ProjectTab::default(), Overview);
        assert_eq!(tab_of_mark("card:versions"), Some(Versions));
        assert_eq!(tab_of_mark("card:sessions"), Some(Overview));
        assert_eq!(tab_of_mark("card:pages"), Some(Pages));
        assert_eq!(tab_of_mark("orch:running"), None);
    }
}

#[cfg(test)]
mod glow_tests {
    /// The Steps card glows fully, then fades, then stops (never before, never after).
    #[test]
    fn steps_glow_holds_then_fades() {
        assert_eq!(super::steps_glow(-0.1), 0.0);
        assert_eq!(super::steps_glow(0.0), 1.0);
        assert_eq!(super::steps_glow(2.4), 1.0);
        let mid = super::steps_glow(3.25);
        assert!(mid > 0.4 && mid < 0.6);
        assert_eq!(super::steps_glow(4.0), 0.0);
        assert_eq!(super::steps_glow(60.0), 0.0);
    }
}

/// Height of a one-line field behind an icon (« + Ajouter une étape », « ? Question rapide »).
const INLINE_H: f32 = 22.0;

/// A row of exactly `INLINE_H`, its items centred on it. `ui.horizontal` centres each item
/// on the row as it stands when the item is added, so a field after a taller icon sat high.
fn inline_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), INLINE_H),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_height(INLINE_H);
            add(ui)
        },
    )
    .inner
}

/// The id of a project's « Ajouter une étape » field, the same whatever is drawn above it.
fn new_step_edit_id(page_id: &str) -> egui::Id {
    egui::Id::unique(("new-step-edit", page_id))
}

/// A frameless one-line field, as tall as its row, its text centred vertically.
fn inline_edit(edit: egui::TextEdit<'_>) -> egui::TextEdit<'_> {
    edit.frame(egui::Frame::NONE)
        .font(FontId::proportional(TEXT))
        .margin(egui::Margin::symmetric(4, 0))
        .min_size(egui::vec2(0.0, INLINE_H))
        .align(egui::Align2::LEFT_CENTER)
}

/// Under the steps: ask the AI for steps (from the project or from a pasted text), and
/// the proposals to tick before they become steps. Nothing is added without a click.
fn ai_steps(
    ui: &mut egui::Ui,
    page_id: &str,
    pr: &mut Project,
    ai: &mut super::ai_fill::AiJobs,
    prefer: Option<crate::sessions::Tool>,
    t: &Tone,
) {
    let folder = pr.folder.trim().to_string();
    let folder_ok = !folder.is_empty() && crate::project::normalize_folder(&folder).is_dir();
    let existing: Vec<String> = pr
        .steps
        .iter()
        .filter(|s| s.status != StepStatus::Done)
        .map(|s| s.title.clone())
        .collect();
    if let Some(list) = ai.proposals.get_mut(page_id) {
        let mut add = false;
        let mut drop = false;
        egui::Frame::new()
            .fill(BLUE.gamma_multiply(0.05))
            .stroke(egui::Stroke::new(1.0, BLUE.gamma_multiply(0.25)))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(crate::tr!(
                        "Proposées par l'IA — cochez celles à garder",
                        "Proposed by the AI — tick the ones to keep"
                    ))
                    .size(12.5)
                    .color(BLUE),
                );
                ui.add_space(2.0);
                for (title, keep) in list.iter_mut() {
                    ui.horizontal(|ui| {
                        ui.checkbox(keep, "");
                        let room = ui.available_width();
                        ui.allocate_ui_with_layout(
                            egui::vec2(room, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(room);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(title.as_str()).size(13.5).color(if *keep {
                                            t.text
                                        } else {
                                            t.tertiary
                                        }),
                                    )
                                    .wrap(),
                                );
                            },
                        );
                    });
                }
                ui.add_space(4.0);
                let n = list.iter().filter(|(_, k)| *k).count();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            n > 0,
                            egui::Button::new(
                                RichText::new(crate::trf!("Ajouter ({n})", "Add ({n})")).size(13.0),
                            ),
                        )
                        .clicked()
                    {
                        add = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(crate::tr!("Ignorer", "Dismiss"))
                                    .size(13.0)
                                    .color(t.tertiary),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        drop = true;
                    }
                });
            });
        if add {
            for (title, keep) in list.iter() {
                if *keep {
                    pr.add_step(title, crate::project::SOURCE_ACCEPTED);
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
            ui.label(
                RichText::new(crate::tr!(
                    "L'IA prépare des étapes…",
                    "The AI is preparing steps…"
                ))
                .size(13.0)
                .color(t.secondary),
            );
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
        let dropped: Vec<String> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| crate::sync::is_image(p))
                .map(|p| p.display().to_string())
                .collect()
        });
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
                let name = std::path::Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.clone());
                ui.label(
                    RichText::new(crate::trf!("Image : {name}", "Image: {name}"))
                        .size(12.5)
                        .color(t.secondary),
                )
                .on_hover_text(p.as_str());
                if icons::delete_cross(
                    ui,
                    18.0,
                    t.tertiary,
                    t.text,
                    t.selected,
                    crate::tr!("Retirer l'image", "Remove the image"),
                )
                .clicked()
                {
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
        // The actions, set apart from the field and from one another.
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.spacing_mut().button_padding = egui::vec2(12.0, 5.0);
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
        ui.add_space(4.0);
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
        let how = match crate::sync::openai::Service::from_id(&ai.text_model.service) {
            Some(service) => crate::trf!("{} reçoit un résumé du dossier (Paramètres › IA) et propose des étapes ; vous choisissez celles à garder.", "{} gets a summary of the folder (Settings › AI) and proposes steps; you choose which to keep.", service.label()),
            None => crate::tr!("L'IA lit le code sans rien modifier et propose des étapes précises (fonctionnalité, technique, sécurité), faites pour être menées par une IA en autonomie ; vous choisissez celles à garder.", "The AI reads the code without changing anything and proposes precise steps (feature, technical, security), made to be carried out by an AI on its own; you choose which to keep.").to_string(),
        };
        let r = r
            .on_hover_text(how)
            .on_disabled_hover_text(crate::tr!("Indiquez d'abord le dossier du projet", "Set the project's folder first"));
        if r.clicked() && folder_ok {
            super::ai_fill::request(ui.ctx(), super::ai_fill::AiReq::Tasks { page_id: page_id.to_string(), folder: folder.clone(), existing: existing.clone(), prefer, text: None, images: vec![] });
        }
        if action_link(ui, crate::tr!("Depuis un texte", "From a text")).on_hover_text(crate::tr!("Collez un texte, joignez des images : l'IA en fait des étapes", "Paste a text, join images: the AI turns them into steps")).clicked() {
            toggle_flag(ui, paste_key);
        }
    });
}

/// Start a new AI session in the project's folder, in the built-in terminal: its AI and
/// model, then « Ouvrir ». A card of its own, apart from the sessions' list (`sessions_card`);
/// shown only once the project has a folder.
fn new_session_card(ui: &mut egui::Ui, cx: &mut Cx, extra: f32) -> f32 {
    let (page_id, pr, t) = (cx.page_id, &*cx.pr, cx.t);
    card(ui, t, extra, |ui| {
        card_title(ui, crate::tr!("NOUVELLE SESSION", "NEW SESSION"), "", t);
        {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let name = std::path::Path::new(pr.folder.trim())
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let launchers = cx.launchers.clone();
                let (ai_key, model_key) = (
                    egui::Id::unique(("session-ai", page_id)),
                    egui::Id::unique(("session-model", page_id)),
                );
                let (mut ai, mut model) = ui.ctx().data(|d| {
                    (
                        d.get_temp::<String>(ai_key).unwrap_or_default(),
                        d.get_temp::<String>(model_key).unwrap_or_default(),
                    )
                });
                let pick = new_session_pick(&launchers, &ai, &model);
                // The AI.
                let before = ai.clone();
                egui::ComboBox::from_id_salt(("session-ai-list", page_id))
                    .selected_text(pick.label)
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        for (label, _, _) in &launchers {
                            ui.selectable_value(&mut ai, label.to_string(), *label);
                        }
                    })
                    .response
                    .on_hover_text(crate::tr!("L'IA à ouvrir", "The AI to open"));
                if ai != before {
                    // Another AI: its own models, so back to « par défaut ».
                    model.clear();
                }
                let pick = new_session_pick(&launchers, &ai, &model);
                // A model this AI doesn't offer (or no longer): back to « par défaut ».
                model = pick.model.clone();
                // Its model: « par défaut » (the AI's own choice), then those it offers.
                ui.add_enabled_ui(!pick.models.is_empty(), |ui| {
                    egui::ComboBox::from_id_salt(("session-model-list", page_id))
                        .selected_text(if model.is_empty() {
                            crate::tr!("Modèle par défaut", "Default model")
                        } else {
                            model.as_str()
                        })
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut model,
                                String::new(),
                                crate::tr!("Par défaut", "Default"),
                            );
                            for m in pick.models {
                                ui.selectable_value(&mut model, m.to_string(), *m);
                            }
                        })
                        .response
                        .on_hover_text(crate::tr!(
                            "Le modèle de la session ; « par défaut » laisse l'IA choisir",
                            "The session's model; « default » lets the AI choose"
                        ))
                        .on_disabled_hover_text(crate::tr!(
                            "Cette IA choisit son modèle dans ses propres réglages",
                            "This AI picks its model in its own settings"
                        ));
                });
                ui.ctx().data_mut(|d| {
                    d.insert_temp(ai_key, ai.clone());
                    d.insert_temp(model_key, model.clone());
                });
                // The model just chosen in the list, if any.
                let pick = new_session_pick(&launchers, &ai, &model);
                let label = pick.label;
                let accent = if pick.cmd.is_none() { t.secondary } else { pick.accent };
                if launch_pill(ui, crate::tr!("Ouvrir", "Open"), accent, t)
                    .on_hover_text(crate::trf!(
                        "Ouvrir {label} dans {name}, dans le terminal intégré",
                        "Open {label} in {name}, in the built-in terminal"
                    ))
                    .clicked()
                {
                    let title = if pick.model.is_empty() {
                        format!("{label} · {name}")
                    } else {
                        format!("{label} ({}) · {name}", pick.model)
                    };
                    super::term_ui::request(ui.ctx(), &title, pr.folder.trim(), pick.command());
                }
            });
        }
    })
}

/// The sessions grouped by AI provider, in the order they first come (the live ones first):
/// each provider's label, then the indices of its live sessions (`live`) and of its other
/// sessions (`others`), both given as their providers' labels.
fn provider_groups(live: &[String], others: &[String]) -> Vec<(String, Vec<usize>, Vec<usize>)> {
    let mut groups: Vec<(String, Vec<usize>, Vec<usize>)> = vec![];
    fn slot<'a>(g: &'a mut Vec<(String, Vec<usize>, Vec<usize>)>, l: &str) -> &'a mut (String, Vec<usize>, Vec<usize>) {
        let i = match g.iter().position(|x| x.0 == l) {
            Some(i) => i,
            None => {
                g.push((l.to_string(), vec![], vec![]));
                g.len() - 1
            }
        };
        &mut g[i]
    }
    for (i, l) in live.iter().enumerate() {
        slot(&mut groups, l).1.push(i);
    }
    for (i, l) in others.iter().enumerate() {
        slot(&mut groups, l).2.push(i);
    }
    groups
}

/// A session an AI linked itself (not read from disk): same columns as `session_row`, a
/// hollow dot, the tool, the title, the time.
fn other_session_row(ui: &mut egui::Ui, s: &crate::project::SessionRef, t: &Tone) {
    let bg = hover_begin(ui);
    let row = ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 20.0), egui::Sense::hover());
        ui.painter()
            .circle_stroke(r.center(), 3.5, egui::Stroke::new(1.2, t.border));
        tool_chip(ui, &tool_label(&s.tool), t);
        let when = relative_time(s.last_activity);
        let room = ui.available_width()
            - ui.painter()
                .layout_no_wrap(when.clone(), FontId::proportional(12.5), t.tertiary)
                .size()
                .x
            - 16.0;
        let title = if s.title.is_empty() {
            s.id.clone()
        } else {
            s.title.clone()
        };
        let job = egui::text::LayoutJob::single_section(
            title,
            egui::TextFormat::simple(FontId::proportional(TEXT), t.text),
        );
        line(ui, job, room).on_hover_text(if s.location.is_empty() {
            &s.id
        } else {
            &s.location
        });
        ui.label(RichText::new(when).size(12.5).color(t.tertiary));
    });
    hover_end(
        ui,
        bg,
        row.response.rect,
        egui::Id::unique(("session-other", &s.id)),
        t,
    );
    ui.add_space(4.0);
}

fn sessions_card(ui: &mut egui::Ui, cx: &mut Cx, extra: f32) -> f32 {
    let (page_id, pr, t, live) = (cx.page_id, &*cx.pr, cx.t, cx.live);
    let modules = pr.modules();
    // ---- Sessions: live ones read from disk, plus those an AI linked itself.
    let mut msg = None;
    let mut clear = false;
    let h = card(ui, t, extra, |ui| {
        let others: Vec<&crate::project::SessionRef> = pr
            .sessions
            .iter()
            .filter(|x| !live.iter().any(|l| l.id == x.id))
            .collect();
        let n = live.len() + others.len();
        card_title(
            ui,
            crate::tr!("SESSIONS IA", "AI SESSIONS"),
            &if n > 0 { n.to_string() } else { String::new() },
            t,
        );
        if n == 0 {
            let hint = if pr.folder.trim().is_empty() {
                crate::tr!(
                    "Indiquez le dossier du projet : ses sessions Claude Code et Codex apparaîtront ici toutes seules.",
                    "Set the project's folder: its Claude Code and Codex sessions will show up here by themselves."
                )
            } else {
                crate::tr!(
                    "Aucune session Claude Code ou Codex trouvée dans ce dossier pour l'instant.",
                    "No Claude Code or Codex session found in this folder yet."
                )
            };
            ui.label(RichText::new(hint).color(t.tertiary).italics());
        }
        let all = flag(ui, ("all-sessions", page_id));
        // One box per AI provider (Claude Code, Codex…) when there are several, each with its
        // own sessions; with a single provider, its rows straight in the card.
        let live_labels: Vec<String> = live.iter().map(|s| s.tool.label().to_string()).collect();
        let other_labels: Vec<String> = others.iter().map(|s| tool_label(&s.tool)).collect();
        let groups = provider_groups(&live_labels, &other_labels);
        let boxed = groups.len() > 1;
        for (label, live_i, other_i) in &groups {
            let mut rows = |ui: &mut egui::Ui| {
                for &i in live_i.iter().take(if all { usize::MAX } else { 4 }) {
                    if let Some(m) = session_row(ui, page_id, &live[i], &modules, t) {
                        msg = Some(m);
                    }
                }
                for &i in other_i.iter().take(if all { usize::MAX } else { 3 }) {
                    other_session_row(ui, others[i], t);
                }
            };
            if boxed {
                let accent = t.secondary;
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0, t.border))
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(label).font(FontId::proportional(13.0)).strong().color(accent));
                            let k = live_i.len() + other_i.len();
                            ui.label(RichText::new(k.to_string()).size(12.0).color(t.tertiary));
                        });
                        ui.add_space(4.0);
                        rows(ui);
                    });
                ui.add_space(8.0);
            } else {
                rows(ui);
            }
        }
        if n > 0 {
            card_rule(ui, t);
        }
        // Footer: see all (or a hint), then "Effacer l'historique", quieter, on the same line.
        let key = ("clear-sessions", page_id);
        let confirming = n > 0 && !pr.read_only && flag(ui, key);
        if n > 0 && !confirming {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if live.len() > 4 || others.len() > 3 {
                    if link(ui, &if all { crate::tr!("Voir moins", "Show less").to_string() } else { crate::trf!("Voir les {n} sessions", "Show the {n} sessions") }) {
                        toggle_flag(ui, ("all-sessions", page_id));
                    }
                } else {
                    ui.label(RichText::new(crate::tr!("Cliquer sur une session pour la reprendre.", "Click a session to resume it.")).size(12.0).color(t.tertiary));
                }
                if !pr.read_only {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if quiet_link(ui, crate::tr!("Effacer l'historique", "Clear the history"), t)
                            .on_hover_text(crate::tr!("Retire les sessions passées de ce projet, sans supprimer leurs fichiers", "Removes the past sessions from this project, without deleting their files"))
                            .clicked()
                        {
                            toggle_flag(ui, key);
                        }
                    });
                }
            });
        }
        // "Effacer l'historique": a second click confirms. The sessions' files stay on disk.
        if confirming {
            {
                ui.horizontal_wrapped(|ui| {
                    let sure = crate::tr!(
                        "Confirmer : effacer l'historique",
                        "Confirm: clear the history"
                    );
                    if ui
                        .add(
                            egui::Label::new(
                                RichText::new(sure)
                                    .size(13.0)
                                    .color(super::style::RED),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        clear = true;
                        toggle_flag(ui, key);
                    }
                    if quiet_link(ui, crate::tr!("Annuler", "Cancel"), t).clicked() {
                        toggle_flag(ui, key);
                    }
                });
                ui.label(RichText::new(crate::tr!("Les sessions disparaissent de ce projet ; leurs fichiers restent sur le disque et une session reprise réapparaît.", "The sessions leave this project; their files stay on disk and a resumed session shows up again.")).size(12.0).color(t.tertiary));
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
        for s in self.agents.all().iter().filter(|s| !s.internal) {
            let root = crate::sync::repo_root(&s.cwd);
            match out.iter_mut().find(|c| c.folder == root) {
                Some(c) => {
                    if s.last_activity > c.last {
                        c.last = s.last_activity;
                        c.tool = s.tool;
                    }
                }
                None => out.push(SyncCandidate {
                    name: std::path::Path::new(&root)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| root.clone()),
                    folder: root,
                    tool: s.tool,
                    last: s.last_activity,
                    is_project: false,
                    checked: false,
                }),
            }
        }
        for p in self.ws.projects() {
            let Some(pr) = p.project.as_ref() else {
                continue;
            };
            if pr.folder.trim().is_empty() {
                continue;
            }
            let f = crate::project::normalize_folder(&pr.folder)
                .display()
                .to_string();
            match out.iter_mut().find(|c| c.folder == f) {
                Some(c) => {
                    c.is_project = true;
                    c.name = p.display_title();
                }
                None => out.push(SyncCandidate {
                    folder: f,
                    name: p.display_title(),
                    tool: Tool::ClaudeCode,
                    last: pr.last_activity,
                    is_project: true,
                    checked: false,
                }),
            }
        }
        for c in out.iter_mut() {
            if let Some(p) = self.ws.project_for_path(std::path::Path::new(&c.folder)) {
                c.is_project = true;
                c.name = p.display_title();
            }
        }
        out.sort_by_key(|x| std::cmp::Reverse(x.last));
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
                            let job = egui::text::LayoutJob::single_section(tilde(&c.folder), egui::TextFormat::simple(FontId::monospace(11.5), t.tertiary));
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
                            Failed(_) => ("✖", super::style::RED),
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
            self.sync_run = Some(crate::sync::SyncRun::start(wanted, move || {
                ctx.request_repaint()
            }));
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
        let states = self.ais.states.lock_or_recover().clone();
        let report = self.mcp_report.lock_or_recover().clone();
        let mut open_ai = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                RichText::new(crate::tr!("IA branchées :", "AIs plugged in:"))
                    .size(12.5)
                    .color(t.tertiary),
            );
            match &states {
                None => {
                    ui.add(egui::Spinner::new().size(11.0));
                    ui.label(
                        RichText::new(crate::tr!("recherche des IA…", "looking for AIs…"))
                            .size(12.5)
                            .color(t.tertiary),
                    );
                }
                Some(s) if !s.iter().any(|x| x.plugged) => {
                    ui.label(
                        RichText::new(crate::tr!("aucune pour l'instant", "none yet"))
                            .size(12.5)
                            .color(t.tertiary),
                    );
                }
                Some(s) => {
                    for x in s.iter().filter(|x| x.plugged) {
                        let failed = report
                            .as_ref()
                            .and_then(|r| r.iter().find(|(n, _)| n == x.ai.label()))
                            .and_then(|(_, o)| {
                                if let crate::setup::Outcome::Failed(e) = o {
                                    Some(e.clone())
                                } else {
                                    None
                                }
                            });
                        match failed {
                            Some(e) => ui
                                .label(
                                    RichText::new(format!("{} ✖", x.ai.label()))
                                        .size(12.5)
                                        .color(super::style::RED),
                                )
                                .on_hover_text(e),
                            None => ui.label(
                                RichText::new(format!("{} ✔", x.ai.label()))
                                    .size(12.5)
                                    .color(GREEN),
                            ),
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
pub(super) fn source_banner(
    ui: &mut egui::Ui,
    page_id: &str,
    src: &crate::docs::PageSource,
    edited: bool,
    actions: &mut Vec<Action>,
) {
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
                let _ = crate::opener::open_path(std::path::Path::new(&src.path));
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
        card_title(
            ui,
            "PAGES",
            &if pages.is_empty() {
                String::new()
            } else {
                pages.len().to_string()
            },
            t,
        );
        if pages.is_empty() {
            ui.label(
                RichText::new(crate::tr!(
                    "Le README et le dossier docs/ du projet apparaîtront ici en pages.",
                    "The project's README and docs/ folder will show up here as pages."
                ))
                .color(t.tertiary)
                .italics(),
            );
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
pub(super) fn tree_list(
    ui: &mut egui::Ui,
    pages: &[PageRow],
    fold_from: usize,
    t: &Tone,
    actions: &mut Vec<Action>,
) {
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
        let a = ui
            .ctx()
            .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.14);
        if a > 0.0 {
            ui.painter()
                .rect_filled(rect, 6.0, t.hover.gamma_multiply(a));
        }
        let x0 = rect.left() + 4.0 + p.depth as f32 * 18.0 + 3.0 * a;
        // Fold arrow.
        let arrow =
            egui::Rect::from_min_size(egui::pos2(x0, rect.top() + 6.0), egui::vec2(16.0, 16.0));
        let mut arrow_clicked = false;
        if p.has_kids {
            let ar = ui.interact(arrow, resp.id.with("fold"), egui::Sense::click());
            icons::paint(
                ui.painter(),
                arrow.shrink(2.0),
                if folded {
                    icons::Icon::ChevronRight
                } else {
                    icons::Icon::ChevronDown
                },
                t.tertiary,
            );
            arrow_clicked = ar.clicked();
            if arrow_clicked {
                toggle_flag(ui, fold_key);
            }
        }
        let x = x0 + 20.0;
        ui.painter().text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &p.icon,
            FontId::proportional(14.0),
            t.text,
        );
        let path_w = if p.rel.is_empty() {
            0.0
        } else {
            ui.painter()
                .layout_no_wrap(p.rel.clone(), FontId::monospace(11.0), t.tertiary)
                .size()
                .x
                .min(w * 0.4)
        };
        let mut job = egui::text::LayoutJob::single_section(
            p.title.clone(),
            egui::TextFormat::simple(FontId::proportional(14.0), t.text),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(
            (rect.right() - x - 26.0 - path_w - 16.0).max(40.0),
        );
        let g = ui.fonts_mut(|f| f.layout_job(job));
        ui.painter().galley(
            egui::pos2(x + 24.0, rect.center().y - g.size().y / 2.0),
            g,
            t.text,
        );
        if !p.rel.is_empty() && resp.hovered() {
            let mut job = egui::text::LayoutJob::single_section(
                p.rel.clone(),
                egui::TextFormat::simple(FontId::monospace(11.0), t.tertiary),
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(path_w);
            let g = ui.fonts_mut(|f| f.layout_job(job));
            ui.painter().galley(
                egui::pos2(
                    rect.right() - 8.0 - g.size().x,
                    rect.center().y - g.size().y / 2.0,
                ),
                g,
                t.tertiary,
            );
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
pub(super) fn folder_view(
    ui: &mut egui::Ui,
    rel: &str,
    pages: &[PageRow],
    t: &Tone,
    actions: &mut Vec<Action>,
) {
    let files = pages.iter().filter(|p| !p.has_kids).count();
    card(ui, t, 0.0, |ui| {
        let what = if rel.is_empty() {
            crate::tr!("Dossier", "Folder").to_string()
        } else {
            crate::trf!("Dossier {rel}", "Folder {rel}")
        };
        card_title(
            ui,
            crate::tr!("CONTENU", "CONTENTS"),
            &crate::trf!(
                "{files} fichier{s}",
                "{files} file{s}",
                s = crate::i18n::plural(files)
            ),
            t,
        );
        ui.label(
            RichText::new(what)
                .font(FontId::monospace(12.0))
                .color(t.tertiary),
        );
        ui.add_space(6.0);
        if pages.is_empty() {
            ui.label(
                RichText::new(crate::tr!("Ce dossier est vide.", "This folder is empty."))
                    .color(t.tertiary)
                    .italics(),
            );
        }
        tree_list(ui, pages, usize::MAX, t, actions);
    });
}

/// "Nouvelle session" buttons: a small terminal mark tinted per tool, then the name.
/// What the « Nouvelle session » line of the « Sessions IA » card opens.
#[derive(Debug, PartialEq)]
struct SessionPick {
    label: &'static str,
    cmd: Option<&'static str>,
    accent: Color32,
    /// The models offered for it (empty: only « par défaut »).
    models: &'static [&'static str],
    /// The model chosen; empty for « par défaut ».
    model: String,
}

/// The AI chosen by its label among `launchers` (the first one when that label is gone, an
/// AI unplugged meanwhile), and the model chosen if that AI offers it (else « par défaut »).
fn new_session_pick(
    launchers: &[(&'static str, Option<&'static str>, Color32)],
    ai: &str,
    model: &str,
) -> SessionPick {
    let (label, cmd, accent) = launchers
        .iter()
        .find(|(l, _, _)| *l == ai)
        .or(launchers.first())
        .copied()
        .unwrap_or(("Shell", None, Color32::GRAY));
    let models = crate::setup::Ai::from_label(label).map(|a| a.models()).unwrap_or(&[]);
    let model = if models.contains(&model) { model.to_string() } else { String::new() };
    SessionPick { label, cmd, accent, models, model }
}

impl SessionPick {
    /// The command opened in the terminal: the AI's program, with `--model <name>` (an option
    /// Claude Code, Codex and Gemini CLI all take) for a chosen model, and nothing added for
    /// « par défaut ». None: the shell.
    fn command(&self) -> Option<String> {
        let cmd = self.cmd?;
        Some(if self.model.is_empty() {
            cmd.to_string()
        } else {
            format!("{cmd} --model {}", crate::sessions::shell_quote(&self.model))
        })
    }
}

fn launch_pill(ui: &mut egui::Ui, label: &str, accent: Color32, t: &Tone) -> egui::Response {
    let font = FontId::proportional(12.5);
    let g = ui.painter().layout_no_wrap(label.to_string(), font, t.text);
    let size = egui::vec2(g.size().x + 40.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 13.0, if resp.hovered() { t.hover } else { t.card });
    p.rect_stroke(
        rect,
        13.0,
        egui::Stroke::new(
            1.0,
            if resp.hovered() {
                accent.gamma_multiply(0.8)
            } else {
                t.border
            },
        ),
        egui::StrokeKind::Inside,
    );
    // Terminal mark: a rounded square with "›_".
    let m = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 16.0, rect.center().y),
        egui::vec2(16.0, 14.0),
    );
    p.rect_filled(m, 3.0, accent.gamma_multiply(0.22));
    let c = m.center();
    let s = egui::Stroke::new(1.4, accent);
    p.line_segment([c + egui::vec2(-4.5, -3.0), c + egui::vec2(-1.5, 0.0)], s);
    p.line_segment([c + egui::vec2(-1.5, 0.0), c + egui::vec2(-4.5, 3.0)], s);
    p.line_segment([c + egui::vec2(0.5, 3.5), c + egui::vec2(4.5, 3.5)], s);
    p.galley(
        egui::pos2(rect.left() + 30.0, rect.center().y - g.size().y / 2.0),
        g,
        t.text,
    );
    resp
}

// ---------------------------------------------------------------- "Regard extérieur"

/// The review and the AI that wrote it, or why it failed: filled by the background thread.
type ReviewResult = Arc<std::sync::Mutex<Option<Result<(String, crate::sessions::Tool), String>>>>;

/// A review being written in the background.
pub struct ReviewJob {
    result: ReviewResult,
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
        if let Some(r) = ctx.data_mut(|d| d.remove_temp::<ReviewRequest>(review_key()))
            && !self.reviews.contains_key(&r.page_id)
        {
            let out: Arc<std::sync::Mutex<Option<_>>> = Arc::default();
            let (o2, c2) = (out.clone(), ctx.clone());
            let code = self.code_forced(&r.page_id);
            std::thread::spawn(move || {
                let res = crate::sync::ask_review(&r.folder, code, r.prefer);
                *o2.lock_or_recover() = Some(res);
                c2.request_repaint();
            });
            self.reviews
                .insert(r.page_id.clone(), ReviewJob { result: out });
        }
        let done: Vec<String> = self
            .reviews
            .iter()
            .filter(|(_, j)| j.result.lock_or_recover().is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for id in done {
            let Some(job) = self.reviews.remove(&id) else {
                continue;
            };
            let Some(res) = job.result.lock_or_recover().take() else {
                continue;
            };
            match res {
                Ok((text, tool)) => {
                    if let Some(pr) = self.ws.page_mut(&id).and_then(|p| p.project.as_mut()) {
                        pr.review = Some(crate::project::Review {
                            text: crate::richtext::clean(&text, 2500),
                            by: tool.label().to_string(),
                            at: crate::model::now_ms(),
                        });
                        pr.touch();
                    }
                    self.detect_changes();
                }
                Err(e) => self.fail(crate::trf!(
                    "Regard extérieur impossible : {e}",
                    "Outside view failed: {e}"
                )),
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
    let cols = if n >= 2 && w >= 860.0 {
        n.min(3)
    } else if n >= 2 && w >= 560.0 {
        2
    } else {
        1
    };
    if cols == 1 {
        super::rich::markdown(ui, text, TEXT, t);
        return;
    }
    let accents = [
        super::style::PINK,
        super::style::PURPLE,
        GREEN,
    ];
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
                                ui.label(
                                    RichText::new(title)
                                        .font(semibold(14.0))
                                        .color(accent.lerp_to_gamma(t.text, 0.4)),
                                );
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

/// The tab title of a quick question carried on in a terminal: the question, short.
fn resume_title(question: &str) -> String {
    let q = crate::richtext::one_line(question, 40);
    if q.is_empty() {
        crate::tr!("Question rapide", "Quick question").to_string()
    } else {
        q
    }
}

/// Full-width card at the bottom of a project: an AI's short, honest outside view —
/// opinion, similar projects that appeared, what to do next.
/// "Question rapide": one line to ask the AI something about the project (« would this
/// feature be worth it? »); the answer comes in the same box, a few lines, read-only.
fn quick_question(ui: &mut egui::Ui, cx: &mut Cx) {
    let (page_id, t) = (cx.page_id, cx.t);
    let prefer = cx.live.first().map(|s| s.tool);
    let folder = cx.pr.folder.trim().to_string();
    let text_key = egui::Id::unique(("quick-question", page_id));
    let mut text: String = ui.ctx().data(|d| d.get_temp(text_key)).unwrap_or_default();
    let asking = cx.ai.asking(page_id).map(String::from);
    let hint = if cx.pr.is_code() {
        crate::tr!(
            "Question rapide à l'IA, ex. « telle fonctionnalité serait-elle intéressante ? »",
            "Quick question to the AI, e.g. « would this feature be worth it? »"
        )
    } else {
        crate::tr!(
            "Question rapide à l'IA, ex. « que me reste-t-il à rendre ? », « quel document manque ? »",
            "Quick question to the AI, e.g. « what do I still have to hand in? », « which document is missing? »"
        )
    };
    let mut close = false;
    card(ui, t, 0.0, |ui| {
        inline_row(ui, |ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, INLINE_H), egui::Sense::hover());
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "?",
                semibold(15.0),
                super::style::PURPLE,
            );
            let send_w = 180.0;
            let edit = ui.add_enabled(
                asking.is_none(),
                inline_edit(egui::TextEdit::singleline(&mut text))
                    .hint_text(hint)
                    .char_limit(crate::sync::QUESTION_MAX)
                    .desired_width((ui.available_width() - send_w).max(80.0)),
            );
            let ready = asking.is_none() && !text.trim().is_empty();
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let sent = ui
                .add_enabled_ui(ready, |ui| action_link(ui, crate::tr!("Demander", "Ask")))
                .inner
                .clicked();
            // « Cadrer »: the AI gives its opinion on the idea and asks its questions first.
            let framed = ui
                .add_enabled_ui(ready, |ui| {
                    action_link(ui, crate::tr!("Cadrer", "Frame")).on_hover_text(crate::tr!(
                        "L'IA donne son avis sur l'idée et pose ses questions avant toute étape.",
                        "The AI gives its opinion on the idea and asks its questions before any step."
                    ))
                })
                .inner
                .clicked();
            if ready && (sent || enter || framed) {
                super::ai_fill::request(
                    ui.ctx(),
                    super::ai_fill::AiReq::Question {
                        page_id: page_id.to_string(),
                        folder: folder.clone(),
                        question: text.trim().to_string(),
                        prefer,
                        framing: framed,
                    },
                );
                text.clear();
            }
        });
        if let Some(q) = &asking {
            ui.add_space(6.0);
            ui.label(RichText::new(q).size(12.5).color(t.secondary).italics());
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(
                    RichText::new(crate::tr!(
                        "L'IA réfléchit… (quelques dizaines de secondes)",
                        "The AI is thinking… (a few dozen seconds)"
                    ))
                    .size(12.5)
                    .color(t.tertiary),
                );
            });
        } else if let Some(a) = cx.ai.answers.get(page_id) {
            card_rule(ui, t);
            // The app's labels can't be selected (a click on them does something else); the
            // answer's can: drag over it to select, ⌘C to copy, or « Copier » for all of it.
            ui.style_mut().interaction.selectable_labels = true;
            let copied_key = egui::Id::unique(("quick-answer-copied", page_id));
            let copied_at: Option<f64> = ui.ctx().data(|d| d.get_temp(copied_key));
            let now_s = ui.input(|i| i.time);
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&a.question)
                            .size(12.5)
                            .color(t.secondary)
                            .italics(),
                    )
                    .wrap(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if icons::delete_cross(
                        ui,
                        20.0,
                        t.tertiary,
                        t.text,
                        t.selected,
                        crate::tr!("Fermer la réponse", "Close the answer"),
                    )
                    .clicked()
                    {
                        close = true;
                    }
                    if let Ok(md) = &a.answer {
                        let just = copied_at.is_some_and(|at| now_s - at < 1.5);
                        let label = if just {
                            crate::tr!("Copié", "Copied")
                        } else {
                            crate::tr!("Copier", "Copy")
                        };
                        if quiet_link(ui, label, t)
                            .on_hover_text(crate::tr!(
                                "Copier toute la réponse (Markdown)",
                                "Copy the whole answer (Markdown)"
                            ))
                            .clicked()
                        {
                            // The answer as the AI wrote it (Markdown).
                            ui.ctx().copy_text(md.trim().to_string());
                            ui.ctx().data_mut(|d| d.insert_temp(copied_key, now_s));
                        }
                        if just {
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                        }
                    }
                });
            });
            ui.add_space(4.0);
            match &a.answer {
                Ok(md) => {
                    super::rich::markdown(ui, md, 13.5, t);
                    if a.steps > 0 {
                        ui.add_space(4.0);
                        let s = crate::i18n::plural(a.steps);
                        ui.label(
                            RichText::new(crate::trf!(
                                "{} étape{s} proposée{s} dans l'encadré Étapes : cochez celles à garder.",
                                "{} step{s} proposed in the Steps box: tick the ones to keep.",
                                a.steps
                            ))
                            .size(12.5)
                            .color(GREEN),
                        );
                    }
                    if !a.by.is_empty() || a.resume.is_some() {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            if !a.by.is_empty() {
                                ui.label(
                                    RichText::new(crate::trf!(
                                        "réponse de {}",
                                        "answer by {}",
                                        a.by
                                    ))
                                    .size(11.5)
                                    .color(t.tertiary),
                                );
                            }
                            if action_link(ui, crate::tr!("Voir en fil", "View as thread"))
                                .on_hover_text(crate::tr!(
                                    "Afficher la question et la réponse en fil de messages dans le volet de droite",
                                    "Show the question and answer as a message thread in the right-hand panel"
                                ))
                                .clicked()
                            {
                                super::side::open_thread(ui.ctx(), page_id);
                            }
                            if let Some(resume) = &a.resume
                                && action_link(
                                    ui,
                                    crate::tr!(
                                        "Poursuivre la conversation",
                                        "Carry on the conversation"
                                    ),
                                )
                                .on_hover_text(crate::tr!(
                                    "Ouvrir cet échange avec l'IA dans le volet de droite, pour lui répondre ou creuser la question",
                                    "Open this exchange with the AI in the right-hand panel, to answer it or dig further"
                                ))
                                .clicked()
                            {
                                super::side::open_conversation(
                                    ui.ctx(),
                                    page_id,
                                    &resume_title(&a.question),
                                    resume.clone(),
                                );
                            }
                            if let Some(resume) = &a.resume
                                && cx.pr.has_module(Module::Terminal)
                                && quiet_link(
                                    ui,
                                    crate::tr!("Ouvrir dans un terminal", "Open in a terminal"),
                                    t,
                                )
                                .on_hover_text(crate::tr!(
                                    "Reprendre cette conversation dans le terminal intégré",
                                    "Carry this conversation on in the built-in terminal"
                                ))
                                .clicked()
                            {
                                super::term_ui::request(
                                    ui.ctx(),
                                    &resume_title(&a.question),
                                    &resume.cwd,
                                    Some(resume.cmd.clone()),
                                );
                            }
                        });
                    }
                }
                Err(e) => {
                    ui.add(
                        egui::Label::new(
                            RichText::new(crate::trf!("Pas de réponse : {e}", "No answer: {e}"))
                                .size(12.5)
                                .color(ORANGE),
                        )
                        .wrap(),
                    );
                }
            }
        }
    });
    if close {
        cx.ai.answers.remove(page_id);
    }
    ui.ctx().data_mut(|d| d.insert_temp(text_key, text));
}

fn review_card(ui: &mut egui::Ui, cx: &mut Cx) {
    let (page_id, t) = (cx.page_id, cx.t);
    let pr = &*cx.pr;
    let prefer = cx.live.first().map(|s| s.tool);
    let reviewing = cx.reviewing || cx.ai.filling(page_id);
    let code = pr.is_code();
    card(ui, t, 0.0, |ui| {
        let by = pr
            .review
            .as_ref()
            .map(|r| format!("{} · {}", r.by, relative_time(r.at)))
            .unwrap_or_default();
        card_title(ui, crate::tr!("REGARD EXTÉRIEUR", "OUTSIDE VIEW"), &by, t);
        let ask = |ui: &mut egui::Ui, label: &str| {
            let folder = pr.folder.trim().to_string();
            let ok = !folder.is_empty() && crate::project::normalize_folder(&folder).is_dir();
            let r = ui.add_enabled(ok, egui::Button::new(RichText::new(label).size(13.0)));
            let r = if ok {
                r.on_hover_text(crate::tr!("L'IA lit le projet sans rien modifier, regarde le web, puis répond en quelques lignes (1 à 3 min, sur votre abonnement).", "The AI reads the project without changing anything, looks at the web, then answers in a few lines (1 to 3 min, on your subscription)."))
            } else {
                r.on_disabled_hover_text(crate::tr!(
                    "Indiquez d'abord le dossier du projet",
                    "Set the project's folder first"
                ))
            };
            if r.clicked() {
                let req = ReviewRequest {
                    page_id: page_id.to_string(),
                    folder,
                    prefer,
                };
                ui.ctx().data_mut(|d| d.insert_temp(review_key(), req));
            }
        };
        if reviewing {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                let text = if code {
                    crate::tr!("L'IA lit le projet et cherche des projets similaires… (1 à 3 minutes)", "The AI reads the project and looks for similar projects… (1 to 3 minutes)")
                } else {
                    crate::tr!("L'IA parcourt les documents et cherche des ressources utiles… (1 à 3 minutes)", "The AI looks through the documents and for useful resources… (1 to 3 minutes)")
                };
                ui.label(RichText::new(text).color(t.secondary));
            });
        } else if let Some(r) = &pr.review {
            review_sections(ui, &r.text, t);
            ui.add_space(6.0);
            ask(ui, crate::tr!("Redemander un avis", "Ask again"));
        } else {
            ui.add(
                egui::Label::new(
                    RichText::new(if code {
                        crate::tr!("Demandez à l'IA branchée sur ce projet un avis court et sans complaisance, les projets similaires apparus récemment, et des idées pour la suite.", "Ask the AI plugged into this project for a short, frank opinion, the similar projects that appeared recently, and ideas for what's next.")
                    } else {
                        crate::tr!("Demandez à l'IA un avis court et sans complaisance sur ce projet, des ressources utiles (sites officiels, guides, modèles) et les prochaines actions.", "Ask the AI for a short, frank opinion on this project, useful resources (official sites, guides, templates) and the next actions.")
                    })
                        .color(t.secondary),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            ask(
                ui,
                crate::tr!("Demander un avis à l'IA", "Ask the AI for an opinion"),
            );
        }
    });
}
