//! "VERSIONS" card of a project: where its git repository stands, and three buttons,
//! Enregistrer, Envoyer, Publier une version. Everything runs in the background with the
//! person's own git and gh; nothing leaves without a click.

use super::*;
use super::projects::{BLUE, GREEN, ORANGE};
use crate::git::GitState;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type Slot<T> = Arc<Mutex<Option<T>>>;

struct Entry {
    /// (generation asked, result): a read started before an action finished is ignored.
    state: Slot<(u64, Result<GitState, String>)>,
    last: Option<Result<GitState, String>>,
    fetched: Instant,
    generation: u64,
    /// An action just finished: the state shown is older than it, buttons wait.
    stale: bool,
    /// Asked the remote (git fetch) once since the app started.
    checked: bool,
    /// GitHub Actions runs: slot, last known, when asked.
    runs: Slot<Vec<crate::git::Run>>,
    runs_last: Vec<crate::git::Run>,
    runs_at: Option<Instant>,
    /// Just pushed or published: watch for the new runs more often for a while.
    hot: Option<Instant>,
}

enum Outcome {
    Done(String),
    /// Finished, nothing to say.
    Quiet,
    Failed(String),
    Message(String),
    Release(String, String),
}

struct Job {
    folder: String,
    what: &'static str,
    out: Slot<Outcome>,
}

#[derive(Clone)]
enum Draft {
    Commit { msg: String },
    Release { version: String, notes: String },
}

#[derive(Default)]
pub struct Versions {
    repos: HashMap<String, Entry>,
    /// Running job per project page.
    jobs: HashMap<String, Job>,
    drafts: HashMap<String, Draft>,
    pub status: Option<super::notices::Notice>,
    /// gh: (installed, signed in), checked in the background every 30 s while needed.
    gh: Slot<(bool, bool)>,
    gh_last: Option<(bool, bool)>,
    gh_checked: Option<Instant>,
    /// New GitHub repository per project: (name, private).
    new_repo: HashMap<String, (String, bool)>,
}

impl Versions {
    /// The last known state of the folder's repository, fetched again when older than 15 s.
    fn state(&mut self, folder: &str, ctx: &egui::Context) -> Option<Result<GitState, String>> {
        let e = self.repos.entry(folder.to_string()).or_insert_with(|| Entry {
            state: Arc::default(),
            last: None,
            fetched: Instant::now() - std::time::Duration::from_secs(60),
            generation: 0,
            stale: false,
            checked: false,
            runs: Arc::default(),
            runs_last: vec![],
            runs_at: None,
            hot: None,
        });
        if let Some((g, r)) = e.state.lock().unwrap().take() {
            if g == e.generation {
                e.last = Some(r);
                e.stale = false;
            }
        }
        if e.fetched.elapsed().as_secs() >= 15 {
            e.fetched = Instant::now();
            let (slot, f, c, g) = (e.state.clone(), folder.to_string(), ctx.clone(), e.generation);
            std::thread::spawn(move || {
                let r = crate::git::state(&f);
                *slot.lock().unwrap() = Some((g, r));
                c.request_repaint();
            });
        }
        e.last.clone()
    }

    /// Read the repository again now; until then the card waits (buttons greyed).
    fn refresh(&mut self, folder: &str) {
        if let Some(e) = self.repos.get_mut(folder) {
            e.fetched = Instant::now() - std::time::Duration::from_secs(60);
            e.generation += 1;
            e.stale = e.last.is_some();
            e.runs_at = None;
            e.hot = Some(Instant::now());
        }
    }

    fn stale(&self, folder: &str) -> bool {
        self.repos.get(folder).is_some_and(|e| e.stale)
    }

    /// The GitHub Actions runs of the repository, asked again every 20 s while one runs,
    /// every 3 min otherwise.
    fn runs(&mut self, folder: &str, root: &str, ctx: &egui::Context) -> Vec<crate::git::Run> {
        let Some(e) = self.repos.get_mut(folder) else { return vec![] };
        if let Some(r) = e.runs.lock().unwrap().take() {
            e.runs_last = r;
        }
        let every = if e.runs_last.iter().any(|r| r.running()) || e.hot.is_some_and(|h| h.elapsed().as_secs() < 120) { 15 } else { 180 };
        if e.runs_at.is_none_or(|t| t.elapsed().as_secs() >= every) {
            e.runs_at = Some(Instant::now());
            let (slot, r, c) = (e.runs.clone(), root.to_string(), ctx.clone());
            std::thread::spawn(move || {
                if let Ok(list) = crate::git::runs(&r) {
                    *slot.lock().unwrap() = Some(list);
                    c.request_repaint();
                }
            });
        }
        if e.runs_last.iter().any(|r| r.running()) {
            ctx.request_repaint_after(std::time::Duration::from_secs(5));
        }
        e.runs_last.clone()
    }

    fn start(&mut self, page_id: &str, folder: &str, what: &'static str, ctx: &egui::Context, work: impl FnOnce() -> Outcome + Send + 'static) {
        let out: Slot<Outcome> = Arc::default();
        let (o, c) = (out.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = work();
            *o.lock().unwrap() = Some(r);
            c.request_repaint();
        });
        self.jobs.insert(page_id.to_string(), Job { folder: folder.to_string(), what, out });
    }

    /// Collect finished jobs (call once per frame).
    pub fn tick(&mut self, ctx: &egui::Context) {
        let done: Vec<String> = self.jobs.iter().filter(|(_, j)| j.out.lock().unwrap().is_some()).map(|(k, _)| k.clone()).collect();
        for id in done {
            let job = self.jobs.remove(&id).unwrap();
            let out = job.out.lock().unwrap().take().unwrap();
            match out {
                Outcome::Done(m) => {
                    self.status = Some(super::notices::Notice::Info(m));
                    self.drafts.remove(&id);
                }
                Outcome::Quiet => {}
                Outcome::Failed(e) => self.status = Some(super::notices::Notice::Error(format!("{} : {e}", job.what))),
                Outcome::Message(m) => {
                    if let Some(Draft::Commit { msg }) = self.drafts.get_mut(&id) {
                        *msg = m;
                    }
                }
                Outcome::Release(v, n) => {
                    if let Some(Draft::Release { version, notes }) = self.drafts.get_mut(&id) {
                        *version = v;
                        *notes = n;
                    }
                }
            }
            self.refresh(&job.folder);
        }
        if !self.jobs.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

impl Versions {
    fn gh(&mut self, ctx: &egui::Context) -> Option<(bool, bool)> {
        if let Some(x) = self.gh.lock().unwrap().take() {
            self.gh_last = Some(x);
        }
        if self.gh_checked.is_none_or(|t| t.elapsed().as_secs() >= 30) {
            self.gh_checked = Some(Instant::now());
            let (slot, c) = (self.gh.clone(), ctx.clone());
            std::thread::spawn(move || {
                *slot.lock().unwrap() = Some(crate::git::gh_info());
                c.request_repaint();
            });
        }
        self.gh_last
    }
}

/// "Mettre le projet sur GitHub": name, private or public, one button; or what to do first
/// (install gh, sign in), each in the built-in terminal.
fn github_panel(ui: &mut egui::Ui, v: &mut Versions, page_id: &str, folder: &str, title: &str, idle: bool, t: &Tone) {
    let ctx = ui.ctx().clone();
    let gh = v.gh(&ctx);
    egui::Frame::new()
        .fill(BLUE.gamma_multiply(0.06))
        .stroke(egui::Stroke::new(1.0, BLUE.gamma_multiply(0.25)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(crate::tr!("Mettre le projet sur GitHub", "Put the project on GitHub")).font(semibold(13.5)).color(t.text));
            ui.add_space(2.0);
            match gh {
                None => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(small(crate::tr!("Recherche de l'outil GitHub (gh)…", "Looking for GitHub's tool (gh)…"), t.tertiary));
                    });
                }
                Some((false, _)) => {
                    ui.label(small(crate::tr!("Il faut d'abord l'outil officiel de GitHub, « gh » : il se connecte avec votre compte, sans jeton à gérer.", "GitHub's official tool, « gh », is needed first: it signs in with your account, no token to handle."), t.secondary));
                    ui.add_space(4.0);
                    let mac_brew = cfg!(target_os = "macos") && crate::sync::which("brew", &crate::sync::shell_path()).is_some();
                    if mac_brew {
                        if super::projects::action_link(ui, crate::tr!("Installer gh et se connecter", "Install gh and sign in")).on_hover_text(crate::tr!("brew install gh, puis gh auth login, dans le terminal intégré", "brew install gh, then gh auth login, in the built-in terminal")).clicked() {
                            super::term_ui::request(ui.ctx(), "GitHub", folder, Some("brew install gh && gh auth login".into()));
                            v.gh_checked = None;
                        }
                    } else if super::projects::action_link(ui, crate::tr!("Télécharger gh", "Download gh")).clicked() {
                        open_url("https://cli.github.com");
                    }
                }
                Some((true, false)) => {
                    ui.label(small(crate::tr!("« gh » est installé ; connectez-vous une fois à votre compte GitHub.", "« gh » is installed; sign in once to your GitHub account."), t.secondary));
                    ui.add_space(4.0);
                    if super::projects::action_link(ui, crate::tr!("Se connecter à GitHub", "Sign in to GitHub")).on_hover_text(crate::tr!("gh auth login dans le terminal intégré", "gh auth login in the built-in terminal")).clicked() {
                        super::term_ui::request(ui.ctx(), "GitHub", folder, Some("gh auth login".into()));
                        v.gh_checked = None;
                    }
                }
                Some((true, true)) => {
                    let default = {
                        let base = std::path::Path::new(folder.trim_end_matches('/')).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        crate::git::repo_name(if base.is_empty() { title } else { &base })
                    };
                    let entry = v.new_repo.entry(page_id.to_string()).or_insert((default, true));
                    let (mut name, mut private) = entry.clone();
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(small(crate::tr!("Nom", "Name"), t.secondary));
                        ui.add(egui::TextEdit::singleline(&mut name).desired_width(180.0).font(FontId::monospace(12.5)));
                        ui.radio_value(&mut private, true, small(crate::tr!("Privé", "Private"), t.text));
                        ui.radio_value(&mut private, false, small(crate::tr!("Public", "Public"), t.text));
                    });
                    v.new_repo.insert(page_id.to_string(), (name.clone(), private));
                    ui.add_space(4.0);
                    let ok = idle && !name.trim().is_empty() && !name.contains(char::is_whitespace);
                    let r = ui.add_enabled(ok, egui::Button::new(RichText::new(crate::tr!("Créer sur GitHub", "Create on GitHub")).size(13.0)));
                    if r.on_hover_text(crate::tr!("Crée le dépôt sur votre compte, le relie à ce dossier et y envoie tout (gh repo create)", "Creates the repository on your account, links it to this folder and pushes everything (gh repo create)")).clicked() {
                        let (f, n) = (folder.to_string(), name.trim().to_string());
                        v.start(page_id, folder, crate::tr!("Création sur GitHub", "Creating on GitHub"), &ctx, move || match crate::git::create_github(&f, &n, private) {
                            Ok(m) => Outcome::Done(m),
                            Err(e) => Outcome::Failed(e),
                        });
                    }
                }
            }
        });
}

fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(url).spawn();
}

fn small(text: impl Into<String>, color: Color32) -> RichText {
    RichText::new(text).size(12.5).color(color)
}

/// The card. `prefer`: the agent that worked on the project last.
pub(super) fn versions_card(ui: &mut egui::Ui, v: &mut Versions, page_id: &str, folder: &str, title: &str, prefer: Option<crate::sessions::Tool>, t: &Tone, extra: f32) -> f32 {
    let ctx = ui.ctx().clone();
    let st = if folder.is_empty() { None } else { v.state(folder, &ctx) };
    let stale = !folder.is_empty() && v.stale(folder);
    let busy = v.jobs.get(page_id).map(|j| j.what).or(if stale { Some(crate::tr!("Mise à jour de l'état", "Updating the state")) } else { None });
    let head = match &st {
        Some(Ok(s)) => format!("{}{}", s.branch, s.last_tag.as_ref().map(|t| format!(" · {t}")).unwrap_or_default()),
        _ => String::new(),
    };
    super::projects::card(ui, t, extra, |ui| {
        super::projects::card_title(ui, "VERSIONS", &head, t);
        let s = match st {
            _ if folder.is_empty() => {
                ui.label(small(crate::tr!("Indiquez le dossier du projet pour suivre ses versions.", "Set the project's folder to follow its versions."), t.tertiary));
                return;
            }
            None => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.label(small(crate::tr!("Lecture du dépôt…", "Reading the repository…"), t.tertiary));
                });
                return;
            }
            Some(Err(e)) => {
                ui.label(small(crate::trf!("Ce dossier n'est {}.", "This folder is {}.", if e.contains("dépôt") || e.contains("repository") { crate::tr!("pas encore suivi par git", "not tracked by git yet") } else { crate::tr!("pas lisible", "not readable") }), t.secondary));
                if e.contains("dépôt") || e.contains("repository") {
                    ui.add_space(6.0);
                    let r = ui.add_enabled(busy.is_none(), egui::Button::new(RichText::new(crate::tr!("Créer le dépôt git", "Create the git repository")).size(13.0)));
                    if r.on_hover_text(crate::tr!("git init, puis un premier enregistrement de tous les fichiers", "git init, then a first commit of all the files")).clicked() {
                        let f = folder.to_string();
                        v.start(page_id, folder, crate::tr!("Création du dépôt", "Creating the repository"), &ctx, move || match crate::git::init(&f) {
                            Ok(m) => Outcome::Done(m),
                            Err(e) => Outcome::Failed(e),
                        });
                    }
                    ui.add_space(8.0);
                    github_panel(ui, v, page_id, folder, title, busy.is_none(), t);
                } else {
                    ui.label(small(e, t.tertiary));
                }
                return;
            }
            Some(Ok(s)) => s,
        };
        // Ask the remote once per session, quietly, so "à récupérer" is right from the start.
        if s.remote.is_some() && busy.is_none() && v.repos.get(folder).is_some_and(|e| !e.checked) {
            if let Some(e) = v.repos.get_mut(folder) {
                e.checked = true;
            }
            let root = s.root.clone();
            v.start(page_id, folder, crate::tr!("Vérification auprès de GitHub", "Checking with GitHub"), &ctx, move || match crate::git::check(&root) {
                Ok(_) => Outcome::Quiet,
                Err(e) => Outcome::Failed(e),
            });
        }
        let runs = if s.gh && s.github.is_some() && std::path::Path::new(&s.root).join(".github/workflows").is_dir() { crate::git::runs_to_show(&v.runs(folder, &s.root, &ctx), &s) } else { vec![] };
        // Wide enough: state and actions on the left, the commits on the right.
        let w = ui.available_width();
        let wide = w >= 620.0 && !s.commits.is_empty();
        let mut main = |ui: &mut egui::Ui| {
            // Where it stands.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                // Each piece stays whole and goes to the next line when it doesn't fit.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                if s.changes.is_empty() {
                    ui.label(small(crate::tr!("Tout est enregistré", "Everything is saved"), GREEN));
                } else {
                    let files: Vec<String> = s.changes.iter().take(12).map(|l| l.get(3..).unwrap_or(l).to_string()).collect();
                    let more = if s.changes.len() > 12 { crate::trf!("\n… et {} autres", "\n… and {} more", s.changes.len() - 12) } else { String::new() };
                    ui.label(small(crate::trf!("{} modification(s) non enregistrée(s)", "{} unsaved change(s)", s.changes.len()), ORANGE)).on_hover_text(files.join("\n") + &more);
                }
                if s.ahead > 0 && s.behind > 0 {
                    ui.label(small(crate::trf!("Divergé : ↑ {} ici, ↓ {} sur GitHub", "Diverged: ↑ {} here, ↓ {} on GitHub", s.ahead, s.behind), ORANGE))
                        .on_hover_text(crate::tr!("Des commits des deux côtés. « Récupérer » replace les vôtres après ceux de GitHub, puis vous pourrez envoyer.", "Commits on both sides. « Pull » puts yours after GitHub's, then you can push."));
                } else if s.ahead > 0 {
                    ui.label(small(crate::trf!("↑ {} à envoyer", "↑ {} to push", s.ahead), BLUE));
                } else if s.behind > 0 {
                    ui.label(small(crate::trf!("↓ {} à récupérer", "↓ {} to pull", s.behind), BLUE));
                } else if s.upstream.is_some() && s.changes.is_empty() {
                    ui.label(small(crate::tr!("= identique à GitHub", "= same as GitHub"), t.tertiary));
                }
                if s.since_tag > 0 && s.last_tag.is_some() {
                    ui.label(small(crate::trf!("{} commit(s) depuis {}", "{} commit(s) since {}", s.since_tag, s.last_tag.as_deref().unwrap_or("")), t.tertiary));
                }
                if s.remote.is_some() {
                    let r = ui.add_enabled_ui(busy.is_none(), |ui| super::projects::action_link(ui, crate::tr!("Vérifier", "Check"))).inner;
                    if r.on_hover_text(crate::tr!("Demande à GitHub ce qui a changé (git fetch) et compare avec vos fichiers", "Asks GitHub what changed (git fetch) and compares with your files")).clicked() && busy.is_none() {
                        let root = s.root.clone();
                        v.start(page_id, folder, crate::tr!("Vérification auprès de GitHub", "Checking with GitHub"), &ctx, move || match crate::git::check(&root) {
                            Ok(m) => Outcome::Done(m),
                            Err(e) => Outcome::Failed(e),
                        });
                    }
                }
                if let Some(g) = &s.github {
                    if super::projects::action_link(ui, crate::tr!("Ouvrir sur GitHub", "Open on GitHub")).clicked() {
                        open_url(g);
                    }
                }
            });
            if !runs.is_empty() {
                ui.add_space(4.0);
                run_pills(ui, &runs, t);
            }
            ui.add_space(6.0);
            // The branch: click to work on another one, or start a new one.
            let idle = busy.is_none();
            ui.horizontal(|ui| {
                ui.label(small(crate::tr!("Branche", "Branch"), t.tertiary));
                let btn = ui.add_enabled_ui(idle, |ui| branch_pill(ui, &s.branch, t)).inner;
                let changes = !s.changes.is_empty();
                egui::Popup::menu(&btn).show(|ui| {
                    ui.set_min_width(240.0);
                    ui.label(small(crate::tr!("Travailler sur", "Work on"), t.tertiary));
                    let mut pick: Option<(String, bool)> = None;
                    for b in &s.branches {
                        let cur = *b == s.branch;
                        let label = if cur { format!("✔ {b}") } else { format!("    {b}") };
                        if ui.add_enabled(!cur && idle, egui::Button::new(RichText::new(label).size(13.0)).frame_when_inactive(false)).clicked() {
                            pick = Some((b.clone(), false));
                            ui.close();
                        }
                    }
                    if !s.remote_branches.is_empty() {
                        ui.separator();
                        ui.label(small(crate::tr!("Sur GitHub seulement", "Only on GitHub"), t.tertiary));
                        for b in &s.remote_branches {
                            if ui.add_enabled(idle, egui::Button::new(RichText::new(format!("    {b}")).size(13.0)).frame_when_inactive(false)).on_hover_text(crate::tr!("La récupère et la suit", "Fetches and follows it")).clicked() {
                                pick = Some((b.clone(), false));
                                ui.close();
                            }
                        }
                    }
                    ui.separator();
                    let key = egui::Id::unique(("new-branch", page_id));
                    let mut name: String = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_default();
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text(crate::tr!("nouvelle-branche", "new-branch")).desired_width(150.0).font(FontId::monospace(12.5)));
                        let ok = crate::git::valid_branch(name.trim()) && !s.branches.iter().any(|b| b == name.trim());
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if (ui.add_enabled(ok && idle, egui::Button::new(RichText::new(crate::tr!("Créer", "Create")).size(13.0))).clicked() || (enter && ok)) && idle {
                            pick = Some((name.trim().to_string(), true));
                            ui.close();
                        }
                    });
                    ui.ctx().data_mut(|d| d.insert_temp(key, if pick.is_some() { String::new() } else { name }));
                    if changes {
                        ui.add_space(2.0);
                        ui.label(small(crate::tr!("Vos modifications non enregistrées vous suivent (git refuse s'il ne peut pas).", "Your unsaved changes come along (git refuses if it can't)."), t.tertiary));
                    }
                    if let Some((b, create)) = pick {
                        let root = s.root.clone();
                        v.start(page_id, folder, crate::tr!("Changement de branche", "Switching branch"), &ctx, move || match crate::git::switch(&root, &b, create) {
                            Ok(m) => Outcome::Done(m),
                            Err(e) => Outcome::Failed(e),
                        });
                    }
                });
            });
            ui.add_space(8.0);
            if let Some(what) = busy {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.label(small(format!("{what}…"), t.secondary));
                });
                ui.add_space(6.0);
            }
            let draft = v.drafts.get(page_id).cloned();
            match draft {
                None => {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let can_commit = idle && !s.changes.is_empty();
                        let r = ui.add_enabled(can_commit, egui::Button::new(RichText::new(crate::tr!("Enregistrer", "Save")).size(13.0)));
                        if r.on_hover_text(crate::tr!("Un commit de toutes les modifications ; l'IA propose le message", "A commit of all the changes; the AI proposes the message")).on_disabled_hover_text(if idle { crate::tr!("Rien à enregistrer", "Nothing to save") } else { crate::tr!("Une action est en cours", "An action is running") }).clicked() {
                            v.drafts.insert(page_id.to_string(), Draft::Commit { msg: String::new() });
                            suggest_commit(v, page_id, &s, prefer, &ctx);
                        }
                        if s.behind > 0 {
                            let diverged = s.ahead > 0;
                            let r = ui.add_enabled(idle, egui::Button::new(RichText::new(crate::trf!("↓ Récupérer ({})", "↓ Pull ({})", s.behind)).size(13.0).color(if idle { BLUE } else { t.tertiary })));
                            let tip = if diverged { crate::tr!("git pull --rebase : vos commits sont replacés après ceux de GitHub (annulé s'il y a un conflit)", "git pull --rebase: your commits go after GitHub's (undone on a conflict)") } else { crate::tr!("git pull : récupère les nouveautés de GitHub (vos modifications non enregistrées sont gardées)", "git pull: brings GitHub's new commits (your unsaved changes are kept)") };
                            if r.on_hover_text(tip).clicked() {
                                let root = s.root.clone();
                                v.start(page_id, folder, crate::tr!("Récupération", "Pulling"), &ctx, move || match crate::git::pull(&root, diverged) {
                                    Ok(m) => Outcome::Done(m),
                                    Err(e) => Outcome::Failed(e),
                                });
                            }
                        }
                        let can_push = idle && s.remote.is_some() && s.behind == 0 && (s.ahead > 0 || s.upstream.is_none()) && !s.commits.is_empty();
                        let r = ui.add_enabled(can_push, egui::Button::new(RichText::new(crate::tr!("Envoyer", "Push")).size(13.0)));
                        let why = if !idle { crate::tr!("Une action est en cours", "An action is running") } else if s.remote.is_none() { crate::tr!("Pas encore de dépôt distant", "No remote repository yet") } else if s.behind > 0 { crate::tr!("Récupérez d'abord les nouveautés de GitHub", "Pull GitHub's new commits first") } else { crate::tr!("Rien à envoyer", "Nothing to push") };
                        if r.on_hover_text(crate::tr!("git push vers le dépôt distant", "git push to the remote repository")).on_disabled_hover_text(why).clicked() {
                            let (root, up) = (s.root.clone(), s.upstream.is_some());
                            v.start(page_id, folder, crate::tr!("Envoi", "Pushing"), &ctx, move || match crate::git::push(&root, up) {
                                Ok(m) => Outcome::Done(m),
                                Err(e) => Outcome::Failed(e),
                            });
                        }
                        let can_release = idle && s.changes.is_empty() && s.since_tag > 0;
                        let r = ui.add_enabled(can_release, egui::Button::new(RichText::new(crate::tr!("Publier une version", "Publish a version")).size(13.0)));
                        let why = if !idle { crate::tr!("Une action est en cours", "An action is running") } else if !s.changes.is_empty() { crate::tr!("Enregistrez d'abord les modifications", "Save the changes first") } else { crate::tr!("Rien de nouveau depuis la dernière version", "Nothing new since the last version") };
                        if r.on_hover_text(crate::tr!("Un numéro de version et ses notes, proposés par l'IA ; puis l'étiquette et la page de version sur GitHub", "A version number and its notes, proposed by the AI; then the tag and the release page on GitHub")).on_disabled_hover_text(why).clicked() {
                            let [_, minor, _] = crate::git::next_versions(s.last_tag.as_deref());
                            v.drafts.insert(page_id.to_string(), Draft::Release { version: minor, notes: String::new() });
                            suggest_release(v, page_id, &s, prefer, &ctx);
                        }
                    });
                    if s.remote.is_none() {
                        ui.add_space(8.0);
                        github_panel(ui, v, page_id, folder, title, idle, t);
                    } else if !s.gh && s.github.is_some() {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(small(crate::tr!("Pour les pages de version :", "For the release pages:"), t.tertiary));
                            if super::projects::action_link(ui, crate::tr!("se connecter avec gh", "sign in with gh")).clicked() {
                                super::term_ui::request(ui.ctx(), "GitHub", folder, Some("gh auth login".into()));
                            }
                        });
                    }
                }
                Some(Draft::Commit { mut msg }) => {
                    ui.label(small(crate::tr!("Message de l'enregistrement", "Commit message"), t.secondary));
                    let hint = if busy.is_some() { crate::tr!("L'IA lit les modifications…", "The AI is reading the changes…") } else { crate::tr!("Ce qui change, en une ligne", "What changes, in one line") };
                    ui.add_enabled(idle, egui::TextEdit::multiline(&mut msg).hint_text(hint).desired_rows(3).desired_width(f32::INFINITY).font(FontId::proportional(13.0)));
                    let mut close = false;
                    ui.horizontal(|ui| {
                        if ui.add_enabled(idle && !msg.trim().is_empty(), egui::Button::new(RichText::new(crate::tr!("Enregistrer", "Save")).size(13.0))).clicked() {
                            let (root, m) = (s.root.clone(), msg.clone());
                            v.start(page_id, folder, crate::tr!("Enregistrement", "Saving"), &ctx, move || match crate::git::commit(&root, &m) {
                                Ok(m) => Outcome::Done(m),
                                Err(e) => Outcome::Failed(e),
                            });
                        }
                        if idle && super::projects::action_link(ui, crate::tr!("Reproposer", "Propose again")).clicked() {
                            suggest_commit(v, page_id, &s, prefer, &ctx);
                        }
                        if ui.add_enabled(idle, egui::Button::new(small(crate::tr!("Annuler", "Cancel"), t.tertiary)).frame(false)).clicked() {
                            close = true;
                        }
                    });
                    if close {
                        v.drafts.remove(page_id);
                    } else if v.drafts.contains_key(page_id) {
                        v.drafts.insert(page_id.to_string(), Draft::Commit { msg });
                    }
                }
                Some(Draft::Release { mut version, mut notes }) => {
                    let [patch, minor, major] = crate::git::next_versions(s.last_tag.as_deref());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(small(crate::tr!("Version", "Version"), t.secondary));
                        ui.add(egui::TextEdit::singleline(&mut version).desired_width(90.0).font(FontId::monospace(13.0)));
                        for (label, val) in [("correctif", &patch), (crate::tr!("nouveautés", "new features"), &minor), ("majeure", &major)] {
                            if ui.add(egui::Button::new(small(format!("{label} {val}"), if version == *val { BLUE } else { t.tertiary })).frame(false)).clicked() {
                                version = val.clone();
                            }
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(small(crate::tr!("Notes de version", "Release notes"), t.secondary));
                    let hint = if busy.is_some() { crate::tr!("L'IA lit les commits depuis la dernière version…", "The AI is reading the commits since the last version…") } else { crate::tr!("Ce qui change pour les utilisateurs", "What changes for the users") };
                    ui.add_enabled(idle, egui::TextEdit::multiline(&mut notes).hint_text(hint).desired_rows(6).desired_width(f32::INFINITY).font(FontId::proportional(13.0)));
                    let builds = std::path::Path::new(&s.root).join(".github/workflows").is_dir();
                    ui.label(small(
                        match (s.gh, builds) {
                            (true, true) => crate::tr!("Publier : étiquette envoyée, page de version créée ; GitHub fabrique ensuite les apps (quelques minutes).", "Publish: tag pushed, release page created; GitHub then builds the apps (a few minutes)."),
                            (true, false) => crate::tr!("Publier : étiquette envoyée et page de version créée sur GitHub.", "Publish: tag pushed and release page created on GitHub."),
                            (false, _) => crate::tr!("Publier : étiquette envoyée (connectez « gh » pour créer aussi la page de version).", "Publish: tag pushed (sign in with « gh » to also create the release page)."),
                        },
                        t.tertiary,
                    ));
                    let mut close = false;
                    ui.horizontal(|ui| {
                        let ok = idle && crate::git::valid_version(version.trim()) && s.remote.is_some();
                        let r = ui.add_enabled(ok, egui::Button::new(RichText::new(crate::trf!("Publier {}", "Publish {}", version.trim())).size(13.0)));
                        if r.on_disabled_hover_text(if s.remote.is_none() { crate::tr!("Pas encore de dépôt distant", "No remote repository yet") } else { crate::tr!("Numéro de version attendu, ex. v1.2.0", "Version number expected, e.g. v1.2.0") }).clicked() {
                            let (root, ver, n, gh) = (s.root.clone(), version.trim().to_string(), notes.clone(), s.gh);
                            v.start(page_id, folder, crate::tr!("Publication", "Publishing"), &ctx, move || match crate::git::release(&root, &ver, &n, gh) {
                                Ok(m) => Outcome::Done(m),
                                Err(e) => Outcome::Failed(e),
                            });
                        }
                        if idle && super::projects::action_link(ui, crate::tr!("Reproposer", "Propose again")).clicked() {
                            suggest_release(v, page_id, &s, prefer, &ctx);
                        }
                        if ui.add_enabled(idle, egui::Button::new(small(crate::tr!("Annuler", "Cancel"), t.tertiary)).frame(false)).clicked() {
                            close = true;
                        }
                    });
                    if close {
                        v.drafts.remove(page_id);
                    } else if v.drafts.contains_key(page_id) {
                        v.drafts.insert(page_id.to_string(), Draft::Release { version, notes });
                    }
                }
            }
        };
        if wide {
            // Two children cut out of the card's width exactly: neither can push the card.
            let gap = 29.0;
            let left_w = ((w - gap) * 0.56).floor();
            let right_w = w - gap - left_w;
            let top = ui.cursor().min;
            let mut left = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(top, egui::vec2(left_w, 10_000.0))).layout(egui::Layout::top_down(egui::Align::Min)));
            left.set_width(left_w);
            left.set_clip_rect(egui::Rect::from_min_size(top, egui::vec2(left_w + 2.0, 10_000.0)).intersect(ui.clip_rect()));
            main(&mut left);
            let h_left = left.min_rect().height();
            let rx = top.x + left_w + gap;
            let mut right = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::pos2(rx, top.y), egui::vec2(right_w, 10_000.0))).layout(egui::Layout::top_down(egui::Align::Min)));
            right.set_width(right_w);
            right.set_clip_rect(egui::Rect::from_min_size(egui::pos2(rx, top.y), egui::vec2(right_w, 10_000.0)).intersect(ui.clip_rect()));
            right.label(small(crate::tr!("Derniers commits", "Latest commits"), t.tertiary));
            right.add_space(4.0);
            commit_list(&mut right, &s, t);
            let h = h_left.max(right.min_rect().height());
            let sx = top.x + left_w + gap / 2.0;
            ui.painter().vline(sx, egui::Rangef::new(top.y, top.y + h), egui::Stroke::new(1.0, t.border));
            ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
        } else {
            main(ui);
            if !s.commits.is_empty() {
                ui.add_space(10.0);
                commit_list(ui, &s, t);
            }
        }
    })
}

/// The last commits as a small timeline; the time sits on the right.
fn commit_list(ui: &mut egui::Ui, s: &GitState, t: &Tone) {
    let n = s.commits.len().min(5);
    for (i, c) in s.commits.iter().take(5).enumerate() {
        let bg = super::projects::hover_begin(ui);
        let w = ui.available_width();
        let row = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            // A dot per commit, joined by a line; the newest is filled.
            let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 20.0), egui::Sense::hover());
            let c0 = r.center();
            if i + 1 < n {
                ui.painter().line_segment([c0 + egui::vec2(0.0, 4.0), egui::pos2(c0.x, r.bottom() + 6.0)], egui::Stroke::new(1.2, t.border));
            }
            if i == 0 {
                ui.painter().circle_filled(c0, 4.0, BLUE);
            } else {
                ui.painter().circle_stroke(c0, 3.5, egui::Stroke::new(1.3, t.tertiary));
            }
            let when = super::chrome::relative_time(c.at);
            let when_w = ui.painter().layout_no_wrap(when.clone(), FontId::proportional(12.5), t.tertiary).size().x;
            let show_hash = w >= 300.0;
            let hash_w = if show_hash { ui.painter().layout_no_wrap(c.hash.clone(), FontId::monospace(11.5), t.tertiary).size().x + 8.0 } else { 0.0 };
            if show_hash {
                ui.label(RichText::new(&c.hash).font(FontId::monospace(11.5)).color(t.tertiary));
            }
            let room = (w - 20.0 - hash_w - when_w - 16.0).max(40.0);
            let mut job = egui::text::LayoutJob::single_section(c.subject.clone(), egui::TextFormat::simple(FontId::proportional(12.5), t.secondary));
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            ui.allocate_ui_with_layout(egui::vec2(room, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_width(room);
                super::projects::cut_label(ui, job).on_hover_text(format!("{} · {} · {}", c.hash, c.author, c.subject));
            });
            ui.label(small(when, t.tertiary));
        });
        super::projects::hover_end(ui, bg, row.response.rect, egui::Id::unique(("commit", &c.hash)), t);
    }
}

/// GitHub Actions for the current commit: one pill per branch or tag, colour by result;
/// a click opens the run on GitHub.
fn run_pills(ui: &mut egui::Ui, runs: &[crate::git::Run], t: &Tone) {
    let red = Color32::from_rgb(224, 62, 62);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(small(crate::tr!("Actions", "Actions"), t.tertiary));
        for r in runs {
            let (word, color) = match (r.status.as_str(), r.conclusion.as_str()) {
                ("completed", "success") => (crate::tr!("réussi", "passed"), GREEN),
                ("completed", "failure" | "startup_failure" | "timed_out") => (crate::tr!("échoué", "failed"), red),
                ("completed", "cancelled") => (crate::tr!("annulé", "cancelled"), t.tertiary),
                ("completed", _) => (crate::tr!("terminé", "finished"), t.tertiary),
                ("queued" | "waiting" | "pending" | "requested", _) => (crate::tr!("en attente", "queued"), t.secondary),
                _ => (crate::tr!("en cours", "running"), BLUE),
            };
            let label = format!("{} · {word}", r.branch);
            let font = FontId::proportional(12.0);
            let g = ui.painter().layout_no_wrap(label, font, t.text);
            let (rect, resp) = ui.allocate_exact_size(g.size() + egui::vec2(26.0, 8.0), egui::Sense::click());
            let a = ui.ctx().animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.15);
            let p = ui.painter();
            p.rect_filled(rect, 10.0, color.gamma_multiply(0.10 + 0.08 * a));
            p.rect_stroke(rect, 10.0, egui::Stroke::new(1.0, color.gamma_multiply(0.35 + 0.4 * a)), egui::StrokeKind::Inside);
            let dot = egui::pos2(rect.left() + 10.0, rect.center().y);
            if r.running() {
                super::projects::pulse_dot(ui, dot, color);
            } else {
                ui.painter().circle_filled(dot, 3.5, color);
            }
            ui.painter().galley(egui::pos2(rect.left() + 18.0, rect.center().y - g.size().y / 2.0), g, t.text);
            let when = super::chrome::relative_time(r.at);
            if resp.on_hover_text(format!("{} — {}\n{when}\n{}", r.workflow, r.title, crate::tr!("Cliquer pour ouvrir sur GitHub", "Click to open on GitHub"))).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                open_url(&r.url);
            }
        }
    });
}

/// "main ▾" as a soft pill that brightens under the pointer.
fn branch_pill(ui: &mut egui::Ui, name: &str, t: &Tone) -> egui::Response {
    let font = FontId::monospace(12.5);
    let g = ui.painter().layout_no_wrap(name.to_string(), font, t.text);
    let (rect, resp) = ui.allocate_exact_size(g.size() + egui::vec2(36.0, 8.0), egui::Sense::click());
    let a = ui.ctx().animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.15);
    let p = ui.painter();
    p.rect_filled(rect, 7.0, t.fill.lerp_to_gamma(t.hover, a));
    p.rect_stroke(rect, 7.0, egui::Stroke::new(1.0, t.border.lerp_to_gamma(BLUE, 0.6 * a)), egui::StrokeKind::Inside);
    // A small branch mark: two dots and a fork.
    let c = egui::pos2(rect.left() + 11.0, rect.center().y);
    let st = egui::Stroke::new(1.3, BLUE);
    p.circle_stroke(c + egui::vec2(-2.0, 4.0), 1.8, st);
    p.circle_stroke(c + egui::vec2(-2.0, -4.0), 1.8, st);
    p.circle_stroke(c + egui::vec2(3.0, -3.0), 1.8, st);
    p.line_segment([c + egui::vec2(-2.0, -2.2), c + egui::vec2(-2.0, 2.2)], st);
    p.line_segment([c + egui::vec2(3.0, -1.2), c + egui::vec2(-1.0, 2.6)], st);
    p.galley(egui::pos2(rect.left() + 20.0, rect.center().y - g.size().y / 2.0), g, t.text);
    p.text(egui::pos2(rect.right() - 9.0, rect.center().y), egui::Align2::CENTER_CENTER, "▾", FontId::proportional(11.0), t.tertiary);
    resp.on_hover_text(crate::tr!("Changer de branche", "Switch branch")).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn suggest_commit(v: &mut Versions, page_id: &str, s: &GitState, prefer: Option<crate::sessions::Tool>, ctx: &egui::Context) {
    let st = s.clone();
    v.start(page_id, &s.root.clone(), crate::tr!("L'IA rédige le message", "The AI is writing the message"), ctx, move || match crate::git::suggest_commit(&st, prefer) {
        Ok(m) => Outcome::Message(m),
        Err(e) => Outcome::Failed(e),
    });
}

fn suggest_release(v: &mut Versions, page_id: &str, s: &GitState, prefer: Option<crate::sessions::Tool>, ctx: &egui::Context) {
    let st = s.clone();
    v.start(page_id, &s.root.clone(), crate::tr!("L'IA prépare les notes de version", "The AI is preparing the release notes"), ctx, move || match crate::git::suggest_release(&st, prefer) {
        Ok((ver, notes)) => Outcome::Release(ver, notes),
        Err(e) => Outcome::Failed(e),
    });
}
