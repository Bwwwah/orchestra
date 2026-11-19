//! "Orchestration": AIs working alone on projects, one or several at a time, each within a
//! share of the Claude subscription and all within a global limit; and the inbox, where
//! their questions, the ends of their sessions and the reports arrive.
//!
//! Each session drives the person's Claude Code in a hidden terminal (see auto.rs for the
//! engine and its reasons). One loop per frame moves every session along:
//! start → (trust the folder) → mission → turns → summary → report.

use super::*;
use crate::auto::{self, Agent, Screen, Signal};
use crate::terminal::Term;
use std::collections::HashMap;
use std::time::Duration;

enum Phase {
    /// Claude Code is starting.
    Starting(Instant),
    /// Text pasted; Enter follows shortly.
    Typed(Instant),
    /// A turn is running.
    Working(Instant),
    /// A command was refused: Esc sent, the note follows.
    Denied(Instant),
    /// The last turn: the summary.
    Summary(Instant),
    /// /exit sent; closing.
    Closing(Instant),
}

pub struct LiveRun {
    pub id: String,
    pub project: String,
    pub title: String,
    agent: Agent,
    term: Term,
    phase: Phase,
    /// Text to type next (sent on Starting → ready, or after a refusal).
    next: Option<String>,
    reader: auto::Reader,
    since: std::time::SystemTime,
    base: String,
    worktree: String,
    pub limit: f32,
    pub pct: f32,
    tokens_counted: u64,
    pub turns: u32,
    max_turns: u32,
    /// Asked to stop (by the person or a limit), with the reason.
    stop: Option<String>,
    summarizing: bool,
    pub last_line: String,
    steps_at_start: Vec<(String, bool)>,
    screen_hash: u64,
    screen_changed: Instant,
    denials: u32,
    nudged: bool,
    started_wall: u64,
    measured: bool,
}

#[derive(Clone)]
struct Draft {
    mission: String,
    limit: f32,
    commits: bool,
    pick: bool,
}

#[derive(Default)]
pub struct Orch {
    pub runs: Vec<LiveRun>,
    drafts: HashMap<String, Draft>,
    watching: Option<String>,
    /// (time of the reading, 5-hour %, its reset time).
    last_reading: Option<(u64, f32, Option<u64>)>,
    pub five: Option<f32>,
    pub week: Option<f32>,
    usage_polled: Option<Instant>,
    daily_checked: Option<Instant>,
    confirm: Option<(String, &'static str)>,
    started: bool,
}

fn default_mission() -> &'static str {
    crate::tr!("Avance sur les étapes ouvertes du projet, dans l'ordre, en vérifiant ton travail.", "Move forward on the project's open steps, in order, checking your work.")
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

impl NotesApp {
    /// Start an autonomous session on a project.
    fn launch_run(&mut self, ctx: &egui::Context, project: &str, mission: &str, limit: f32, commits: bool) -> Result<(), String> {
        if self.orch.runs.iter().any(|r| r.project == project) {
            return Err(crate::tr!("une session tourne déjà sur ce projet", "a session is already running on this project").into());
        }
        let page = self.ws.page(project).ok_or(crate::tr!("projet introuvable", "project not found"))?;
        let pr = page.project.as_ref().ok_or(crate::tr!("ce n'est pas un projet", "this is not a project"))?;
        let title = page.display_title();
        let folder = pr.folder.trim().to_string();
        if folder.is_empty() {
            return Err(crate::tr!("indiquez d'abord le dossier du projet", "set the project's folder first").into());
        }
        if !self.ais.plugged().contains(&crate::setup::Ai::ClaudeCode) {
            return Err(crate::tr!("Claude Code n'est pas branché (Réglages › IA)", "Claude Code isn't plugged in (Settings › AI)").into());
        }
        if let Some(f) = self.orch.five {
            if f >= self.ws.settings.orch.global_limit {
                return Err(crate::trf!("la limite globale ({:.0} %) est déjà atteinte", "the global limit ({:.0} %) is already reached", self.ws.settings.orch.global_limit));
            }
        }
        // The limits are checked with Claude Code's own figures: they come through the status line.
        if !crate::usage::tracking_on() {
            crate::usage::start_tracking(&std::env::current_exe().unwrap_or_default())?;
            self.ais.tracking = true;
        }
        let steps_at_start: Vec<(String, bool)> = pr.steps.iter().map(|s| (s.title.clone(), s.status == crate::project::StepStatus::Done)).collect();
        let copy = auto::make_copy(&folder, &title)?;
        let agent = Agent::ClaudeCode;
        let o = &self.ws.settings.orch;
        let cmd = agent.command(commits, &o.extra_tools);
        let c2 = ctx.clone();
        let mut term = Term::spawn(&crate::trf!("Autonome · {title}", "Autonomous · {title}"), &copy.worktree, Some(&cmd), move || c2.request_repaint_after(Duration::from_millis(200)))?;
        term.resize(42, 140);
        let id = crate::model::new_id();
        let mission = if mission.trim().is_empty() { default_mission().to_string() } else { mission.trim().to_string() };
        let rec = crate::model::RunRecord {
            id: id.clone(),
            project: project.to_string(),
            title: title.clone(),
            tool: agent.label().into(),
            root: copy.root.clone(),
            worktree: copy.worktree.clone(),
            branch: copy.branch.clone(),
            started: crate::model::now_ms(),
            commits_allowed: commits,
            ..Default::default()
        };
        self.ws.runs.push(rec);
        self.dirty_since.get_or_insert_with(Instant::now);
        self.orch.runs.push(LiveRun {
            id,
            project: project.to_string(),
            title,
            agent,
            term,
            phase: Phase::Starting(Instant::now()),
            next: Some(auto::mission_prompt(&self.ws.page(project).map(|p| p.display_title()).unwrap_or_default(), project, &copy.branch, &mission, commits)),
            reader: auto::Reader::default(),
            since: std::time::SystemTime::now() - Duration::from_secs(2),
            base: copy.base,
            worktree: copy.worktree,
            limit,
            pct: 0.0,
            tokens_counted: 0,
            turns: 0,
            max_turns: o.max_turns.max(1),
            stop: None,
            summarizing: false,
            last_line: crate::tr!("Démarrage de Claude Code…", "Starting Claude Code…").into(),
            steps_at_start,
            screen_hash: 0,
            screen_changed: Instant::now(),
            denials: 0,
            nudged: false,
            started_wall: crate::model::now_ms(),
            measured: false,
        });
        Ok(())
    }

    /// Once per frame: move every session along, follow the use of the subscription,
    /// write the day's report when it is time.
    pub(super) fn orch_tick(&mut self, ctx: &egui::Context) {
        if !self.orch.started {
            self.orch.started = true;
            // Sessions that were running when Orchestra closed.
            for r in self.ws.runs.iter_mut().filter(|r| r.outcome.is_empty()) {
                r.outcome = crate::tr!("interrompue (Orchestra fermé)", "interrupted (Orchestra closed)").into();
                r.ended = r.ended.max(r.started);
            }
        }
        self.usage_tick();
        // Developer aid: ORCHESTRA_AUTO_RUN=<project id> starts a session once Claude Code is seen.
        if let Ok(pid) = std::env::var("ORCHESTRA_AUTO_RUN") {
            if !pid.is_empty() && self.orch.runs.is_empty() && !self.ws.runs.iter().any(|r| r.project == pid) && self.ais.plugged().contains(&crate::setup::Ai::ClaudeCode) {
                let (l, c) = (self.ws.settings.orch.default_limit, true);
                if let Err(e) = self.launch_run(ctx, &pid, "", l, c) {
                    eprintln!("auto-run : {e}");
                }
            }
        }
        let now = Instant::now();
        let global = self.ws.settings.orch.global_limit;
        let mut finished: Vec<(usize, String, String)> = vec![];
        for (i, r) in self.orch.runs.iter_mut().enumerate() {
            // How the screen moves (idle = nothing drawn for a while).
            let screen = r.term.text();
            let h = hash(&screen);
            if h != r.screen_hash {
                r.screen_hash = h;
                r.screen_changed = now;
            }
            let idle = r.screen_changed.elapsed();
            if r.term.exited() && !matches!(r.phase, Phase::Closing(_)) {
                finished.push((i, r.stop.clone().unwrap_or_else(|| crate::tr!("Claude Code s'est arrêté", "Claude Code stopped").into()), String::new()));
                continue;
            }
            r.reader.poll(r.agent, std::path::Path::new(&r.worktree), r.since);
            match r.phase {
                Phase::Starting(t0) => {
                    if t0.elapsed() > Duration::from_secs(90) {
                        finished.push((i, crate::tr!("Claude Code n'a pas démarré", "Claude Code didn't start").into(), String::new()));
                    } else if t0.elapsed() > Duration::from_secs(3) && idle > Duration::from_millis(2500) {
                        match auto::read_screen(&screen) {
                            Screen::Trust => {
                                // The copy is a folder Orchestra made for this session.
                                r.term.write(b"\r");
                                r.screen_changed = now;
                            }
                            _ => {
                                if let Some(text) = r.next.take() {
                                    r.term.paste(&text);
                                    r.phase = Phase::Typed(now);
                                }
                            }
                        }
                    }
                }
                Phase::Typed(t) => {
                    if t.elapsed() > Duration::from_millis(500) {
                        r.term.write(b"\r");
                        r.reader.turn_over = false;
                        r.nudged = false;
                        r.phase = if r.summarizing { Phase::Summary(now) } else { Phase::Working(now) };
                        r.last_line = if r.summarizing { crate::tr!("Rédige son compte rendu…", "Writing its summary…").into() } else { crate::trf!("Tour {} en cours…", "Turn {} running…", r.turns + 1) };
                    }
                }
                Phase::Denied(t) => {
                    if t.elapsed() > Duration::from_millis(1500) {
                        r.term.paste(auto::denied_prompt());
                        r.phase = Phase::Typed(now);
                    }
                }
                Phase::Working(t) | Phase::Summary(t) => {
                    let summary = matches!(r.phase, Phase::Summary(_));
                    if auto::read_screen(&screen) == Screen::Permission && idle > Duration::from_millis(800) {
                        r.term.write(b"\x1b");
                        r.denials += 1;
                        r.last_line = crate::tr!("Commande refusée (non autorisée en autonome)", "Command refused (not allowed when autonomous)").into();
                        r.phase = Phase::Denied(now);
                        continue;
                    }
                    // The prompt didn't go: press Enter once more.
                    if !r.nudged && t.elapsed() > Duration::from_secs(60) && r.reader.last_growth.is_none_or(|g| g.elapsed() > t.elapsed()) {
                        r.term.write(b"\r");
                        r.nudged = true;
                    }
                    let quiet = r.reader.last_growth.is_some_and(|g| g.elapsed() > Duration::from_secs(2));
                    let over = r.reader.turn_over && quiet && idle > Duration::from_millis(1500);
                    let too_long = t.elapsed() > Duration::from_secs(if summary { 300 } else { 25 * 60 });
                    if over || too_long {
                        if too_long {
                            r.term.write(b"\x1b");
                        }
                        if summary {
                            let text = if over { r.reader.last_text.clone() } else { String::new() };
                            let why = r.stop.clone().unwrap_or_else(|| crate::tr!("terminée", "finished").into());
                            finished.push((i, why, text));
                            continue;
                        }
                        r.turns += 1;
                        let sig = auto::signal(&r.reader.last_text);
                        r.last_line = crate::richtext::one_line(&auto::strip_markers(&r.reader.last_text), 140);
                        let five = self.orch.five;
                        let reason = if let Some(s) = r.stop.clone() {
                            Some(s)
                        } else if too_long {
                            Some(crate::tr!("un tour a duré trop longtemps", "a turn took too long").into())
                        } else if sig == Signal::Done {
                            Some(crate::tr!("mission terminée", "mission complete").into())
                        } else if sig == Signal::Blocked {
                            Some(crate::tr!("l'IA est bloquée", "the AI is blocked").into())
                        } else if r.pct >= r.limit {
                            Some(crate::trf!("limite du projet atteinte ({:.0} %)", "project limit reached ({:.0} %)", r.limit))
                        } else if five.is_some_and(|f| f >= global) {
                            Some(crate::trf!("limite globale atteinte ({global:.0} %)", "global limit reached ({global:.0} %)"))
                        } else if self.orch.week.is_some_and(|w| w >= 95.0) {
                            Some(crate::tr!("limite de la semaine presque atteinte", "weekly limit almost reached").into())
                        } else if !r.measured && r.turns >= 2 {
                            Some(crate::tr!("utilisation de l'abonnement impossible à mesurer", "subscription use can't be measured").into())
                        } else if r.turns >= r.max_turns {
                            Some(crate::trf!("{} tours atteints", "{} turns reached", r.max_turns))
                        } else {
                            None
                        };
                        match reason {
                            Some(why) => {
                                r.stop = Some(why.clone());
                                r.summarizing = true;
                                r.term.paste(&auto::summary_prompt(&r.project, &why));
                            }
                            None => r.term.paste(auto::continue_prompt()),
                        }
                        r.phase = Phase::Typed(now);
                    }
                }
                Phase::Closing(t) => {
                    if t.elapsed() > Duration::from_secs(3) || r.term.exited() {
                        finished.push((i, String::new(), String::new()));
                    }
                }
            }
        }
        // Finished sessions: report, inbox, then close the terminal.
        for (i, why, summary) in finished.into_iter().rev() {
            if matches!(self.orch.runs[i].phase, Phase::Closing(_)) {
                self.orch.runs.remove(i);
                continue;
            }
            self.finish_run(i, &why, &summary);
        }
        if !self.orch.runs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        self.daily_tick();
    }

    /// Follow Claude Code's figures; share each rise of the 5-hour window between the
    /// running sessions by the tokens each used since the last reading.
    fn usage_tick(&mut self) {
        if self.orch.usage_polled.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            return;
        }
        self.orch.usage_polled = Some(Instant::now());
        let Some(u) = crate::usage::read() else { return };
        let five = u.windows.iter().find(|w| w.label.starts_with("Session")).cloned();
        self.orch.week = u.windows.iter().find(|w| w.label == "Semaine").map(|w| w.pct);
        let Some(five) = five else { return };
        self.orch.five = Some(five.pct);
        let prev = self.orch.last_reading;
        if prev.is_some_and(|(at, _, _)| at == u.at) {
            return;
        }
        self.orch.last_reading = Some((u.at, five.pct, five.resets_at));
        let rise = match prev {
            Some((_, p, r)) if r.is_some() && five.resets_at.is_some() && r.unwrap().abs_diff(five.resets_at.unwrap()) < 120_000 => (five.pct - p).max(0.0),
            Some((_, p, None)) if five.resets_at.is_none() => (five.pct - p).max(0.0),
            Some(_) => five.pct,
            None => 0.0,
        };
        let deltas: Vec<u64> = self.orch.runs.iter().map(|r| r.reader.tokens.saturating_sub(r.tokens_counted)).collect();
        let total: u64 = deltas.iter().sum();
        let n = self.orch.runs.len().max(1) as f32;
        for (r, d) in self.orch.runs.iter_mut().zip(deltas) {
            if u.at >= r.started_wall {
                r.measured = true;
            }
            r.pct += if total > 0 { rise * d as f32 / total as f32 } else { rise / n };
            r.tokens_counted = r.reader.tokens;
        }
    }

    fn finish_run(&mut self, i: usize, why: &str, summary: &str) {
        let r = &mut self.orch.runs[i];
        let why = if why.is_empty() { r.stop.clone().unwrap_or_else(|| crate::tr!("terminée", "finished").into()) } else { why.to_string() };
        r.term.paste("/exit");
        r.term.write(b"\r");
        r.phase = Phase::Closing(Instant::now());
        let (id, project, base, pct, tokens, turns) = (r.id.clone(), r.project.clone(), r.base.clone(), r.pct, r.reader.tokens, r.turns);
        let before = r.steps_at_start.clone();
        let done_now: Vec<String> = self
            .ws
            .page(&project)
            .and_then(|p| p.project.as_ref())
            .map(|pr| pr.steps.iter().filter(|s| s.status == crate::project::StepStatus::Done).map(|s| s.title.clone()).collect())
            .unwrap_or_default();
        let newly: Vec<String> = done_now.into_iter().filter(|t| !before.iter().any(|(b, d)| b == t && *d)).collect();
        let Some(rec) = self.ws.runs.iter_mut().find(|x| x.id == id) else { return };
        rec.ended = crate::model::now_ms();
        rec.outcome = why.clone();
        rec.pct = pct;
        rec.tokens = tokens;
        rec.turns = turns;
        rec.steps_done = newly;
        let rec = rec.clone();
        let (commits, dirty) = auto::work_done(&rec, &base);
        let md = auto::session_report(&rec, &commits, dirty, summary);
        let day = auto::local_day(rec.ended);
        let page = self.report_page(&day, &format!("{} · {}", rec.title, auto::hm(rec.started)), &md);
        if let Some(x) = self.ws.runs.iter_mut().find(|x| x.id == id) {
            x.report = Some(page.clone());
        }
        let text = crate::trf!("{why} · {} étape(s) · ≈ {:.0} % · {} commit(s)", "{why} · {} step(s) · ≈ {:.0} % · {} commit(s)", rec.steps_done.len(), rec.pct, commits.len());
        self.ws.notify("fin", &project, &crate::trf!("Fin de session sur {}", "End of session on {}", rec.title), &text, Some(page));
        self.detect_changes();
        self.ack(crate::trf!("Session autonome terminée sur {} : {why}.", "Autonomous session finished on {}: {why}.", rec.title));
    }

    /// A page under "Rapports des IA › <day>".
    fn report_page(&mut self, day: &str, title: &str, md: &str) -> String {
        let root = {
            let id = self.ws.settings.orch.reports_page.clone();
            if self.ws.page(&id).is_some_and(|p| !p.trashed) {
                id
            } else {
                let id = self.ws.create_page(crate::tr!("Rapports des IA", "AI reports"), None);
                if let Some(p) = self.ws.page_mut(&id) {
                    p.icon = "📒".into();
                }
                self.ws.settings.orch.reports_page = id.clone();
                id
            }
        };
        let day_page = match self.ws.children(Some(&root)).iter().find(|p| p.title == day && !p.trashed) {
            Some(p) => p.id.clone(),
            None => {
                let id = self.ws.create_page(day, Some(root.clone()));
                if let Some(p) = self.ws.page_mut(&id) {
                    p.icon = "📅".into();
                }
                id
            }
        };
        let mut page = crate::store::page_from_markdown(md, title);
        page.title = title.to_string();
        page.icon = "📋".into();
        page.parent = Some(day_page);
        let id = page.id.clone();
        self.ws.pages.push(page);
        id
    }

    /// The day's overall report, at the chosen hour, when several projects ran that day
    /// (and for yesterday, if Orchestra was closed at that hour).
    fn daily_tick(&mut self) {
        if self.orch.daily_checked.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) || !self.orch.runs.is_empty() {
            return;
        }
        self.orch.daily_checked = Some(Instant::now());
        let now = crate::model::now_ms();
        let today = auto::local_day(now);
        let yesterday = auto::local_day(now.saturating_sub(86_400_000));
        let mut days = vec![yesterday];
        if auto::local_hour(now) >= self.ws.settings.orch.report_hour {
            days.push(today);
        }
        for day in days {
            if self.ws.settings.orch.last_daily >= day {
                continue;
            }
            let runs: Vec<crate::model::RunRecord> = self.ws.runs.iter().filter(|r| r.ended > 0 && auto::local_day(r.ended) == day).cloned().collect();
            let mut projects: Vec<&str> = runs.iter().map(|r| r.project.as_str()).collect();
            projects.sort();
            projects.dedup();
            if projects.len() < 2 {
                continue;
            }
            let refs: Vec<&crate::model::RunRecord> = runs.iter().collect();
            let md = auto::daily_report(&day, &refs);
            let page = self.report_page(&day, crate::tr!("Bilan de la journée", "Daily summary"), &md);
            let text = crate::trf!("{} projets, {} sessions", "{} projects, {} sessions", projects.len(), runs.len());
            self.ws.notify("rapport", "", &crate::trf!("Bilan de la journée · {day}", "Daily summary · {day}"), &text, Some(page));
            self.ws.settings.orch.last_daily = day;
            self.detect_changes();
        }
    }

    // ------------------------------------------------------------------ the page

    pub(super) fn orchestra_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let t = tone(ui.visuals().dark_mode);
        let avail = ui.available_width();
        let width = 1000.0f32.min(avail - 40.0).max(320.0);
        let ctx = ui.ctx().clone();
        ui.add_space(50.0);
        ui.horizontal(|ui| {
            ui.add_space(((avail - width) / 2.0).max(20.0));
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.label(RichText::new(crate::tr!("Orchestration", "Orchestration")).font(bold(30.0)).color(t.text));
                ui.label(RichText::new(crate::tr!("Des IA qui avancent seules sur vos projets, chacune dans une copie à part, dans les limites de votre abonnement.", "AIs that move your projects forward on their own, each in a separate copy, within your subscription's limits.")).color(t.secondary));
                ui.add_space(14.0);
                self.stats_row(ui, &t);
                ui.add_space(14.0);
                self.running_card(ui, &t);
                if !self.orch.runs.is_empty() {
                    ui.add_space(14.0);
                }
                if let Some(msg) = self.launch_card(ui, &ctx, &t) {
                    self.notice(msg);
                }
                ui.add_space(14.0);
                self.history_card(ui, &t, actions);
                ui.add_space(14.0);
                self.limits_card(ui, &t);
                ui.add_space(40.0);
            });
        });
    }

    /// Three tiles at the top: the subscription, the AIs at work, today's work.
    fn stats_row(&mut self, ui: &mut egui::Ui, t: &Tone) {
        let five = self.orch.five;
        let week = self.orch.week;
        let global = self.ws.settings.orch.global_limit;
        let running = self.orch.runs.len();
        let ready = self.ws.projects().iter().filter(|p| p.project.as_ref().is_some_and(|x| !x.folder.trim().is_empty())).count();
        let today = auto::local_day(crate::model::now_ms());
        let todays: Vec<&crate::model::RunRecord> = self.ws.runs.iter().filter(|r| !r.outcome.is_empty() && auto::local_day(r.started) == today).collect();
        let steps: usize = todays.iter().map(|r| r.steps_done.len()).sum();
        let used_today: f32 = todays.iter().map(|r| r.pct).sum();
        let hour = self.ws.settings.orch.report_hour;
        let w = ui.available_width();
        let gap = 14.0;
        let cols = if w >= 520.0 { 3 } else { 1 };
        let tw = if cols == 3 { (w - 2.0 * gap) / 3.0 } else { w };
        let h = if cols == 3 { 112.0 } else { 92.0 };
        let draw = |ui: &mut egui::Ui, i: usize| {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(tw, h), egui::Sense::hover());
            let a = ui.ctx().animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.18);
            let accent = [Color32::from_rgb(212, 76, 125), super::projects::GREEN, super::projects::BLUE][i];
            let p = ui.painter();
            p.add(egui::epaint::RectShape::new(rect, 14.0, t.card, egui::Stroke::new(1.0, t.border.lerp_to_gamma(accent, 0.35 * a)), egui::StrokeKind::Inside));
            // A soft wash of the accent in the corner.
            p.with_clip_rect(rect.shrink(1.0)).circle_filled(rect.right_top() + egui::vec2(-10.0, 10.0), 52.0 + 6.0 * a, accent.gamma_multiply(0.07));
            let x = rect.left() + 18.0;
            let label = [crate::tr!("ABONNEMENT · 5 H", "SUBSCRIPTION · 5 H"), crate::tr!("IA AU TRAVAIL", "AIS AT WORK"), crate::tr!("AUJOURD'HUI", "TODAY")][i];
            p.text(egui::pos2(x, rect.top() + 16.0), egui::Align2::LEFT_TOP, label, semibold(11.5), t.tertiary);
            let (big, sub): (String, String) = match i {
                0 => (
                    five.map(|f| format!("{f:.0} %")).unwrap_or_else(|| "—".into()),
                    match (five, week) {
                        (Some(_), Some(wk)) => crate::trf!("limite {global:.0} % · semaine {wk:.0} %", "limit {global:.0} % · week {wk:.0} %"),
                        (Some(_), None) => crate::trf!("limite globale {global:.0} %", "global limit {global:.0} %"),
                        _ => crate::tr!("inconnu pour l'instant", "unknown for now").into(),
                    },
                ),
                1 => (running.to_string(), if running == 0 { crate::trf!("{ready} projet(s) prêt(s) à lancer", "{ready} project(s) ready to start") } else { crate::trf!("sur {ready} projet(s) prêt(s)", "of {ready} project(s) ready") }),
                _ => (
                    todays.len().to_string(),
                    if todays.is_empty() { crate::trf!("session terminée · bilan à {hour} h", "finished session · summary at {hour} h") } else { crate::trf!("session(s) · {steps} étape(s) faite(s) · ≈ {used_today:.0} %", "session(s) · {steps} step(s) done · ≈ {used_today:.0} %") },
                ),
            };
            p.text(egui::pos2(x, rect.top() + 36.0), egui::Align2::LEFT_TOP, &big, bold(30.0), t.text);
            let mut job = egui::text::LayoutJob::single_section(sub, egui::TextFormat::simple(FontId::proportional(12.5), t.secondary));
            job.wrap = egui::text::TextWrapping::truncate_at_width(tw - 36.0 - if i == 0 && tw >= 240.0 { 70.0 } else { 0.0 });
            let g = ui.fonts_mut(|f| f.layout_job(job));
            ui.painter().galley(egui::pos2(x, rect.bottom() - 28.0), g, t.secondary);
            match i {
                0 if tw >= 240.0 => {
                    // A ring: the 5-hour use, the global limit as a tick.
                    let c = egui::pos2(rect.right() - 50.0, rect.center().y + 4.0);
                    ring(ui, c, 28.0, five.map(|f| f / 100.0), global / 100.0, accent, t);
                }
                1 if running > 0 => {
                    super::projects::pulse_dot(ui, egui::pos2(rect.right() - 30.0, rect.top() + 24.0), super::projects::GREEN);
                }
                _ => {}
            }
        };
        if cols == 3 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for i in 0..3 {
                    draw(ui, i);
                }
            });
        } else {
            for i in 0..3 {
                draw(ui, i);
                ui.add_space(8.0);
            }
        }
    }

    fn limits_card(&mut self, ui: &mut egui::Ui, t: &Tone) {
        let five = self.orch.five;
        let o = &mut self.ws.settings.orch;
        let before = o.clone();
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(ui, crate::tr!("RÉGLAGES ET LIMITES", "SETTINGS AND LIMITS"), "", t);
            // The 5-hour window: what is used, and up to where the AIs may go.
            let w = ui.available_width();
            let (area, _) = ui.allocate_exact_size(egui::vec2(w, 34.0), egui::Sense::hover());
            let bar = egui::Rect::from_min_size(egui::pos2(area.left(), area.top() + 18.0), egui::vec2(w, 10.0));
            let p = ui.painter();
            p.rect_filled(bar, 5.0, t.fill);
            let lx = bar.left() + bar.width() * o.global_limit / 100.0;
            // Beyond the limit: hatched-looking darker zone.
            p.rect_filled(egui::Rect::from_min_max(egui::pos2(lx, bar.top()), bar.max), egui::CornerRadius { nw: 0, sw: 0, ne: 5, se: 5 }, Color32::from_rgb(224, 62, 62).gamma_multiply(0.12));
            if let Some(f) = five {
                let frac = super::projects::smooth(ui, "orch-five", f / 100.0);
                let color = if f >= o.global_limit { Color32::from_rgb(224, 62, 62) } else { super::projects::GREEN };
                ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac, bar.height())), 5.0, color);
            }
            let p = ui.painter();
            p.vline(lx, egui::Rangef::new(bar.top() - 3.0, bar.bottom() + 3.0), egui::Stroke::new(2.0, super::projects::ORANGE));
            let lab = crate::trf!("limite {:.0} %", "limit {:.0} %", o.global_limit);
            let align = if lx > bar.right() - 60.0 { egui::Align2::RIGHT_BOTTOM } else { egui::Align2::CENTER_BOTTOM };
            p.text(egui::pos2(lx, bar.top() - 4.0), align, lab, FontId::proportional(11.5), super::projects::ORANGE);
            let used = match five {
                Some(f) => crate::trf!("{f:.0} % utilisés", "{f:.0} % used"),
                None => crate::tr!("utilisation inconnue", "usage unknown").into(),
            };
            p.text(egui::pos2(bar.left(), bar.top() - 4.0), egui::Align2::LEFT_BOTTOM, used, FontId::proportional(11.5), t.secondary);
            ui.add_space(10.0);
            let lk = super::settings::look(ui.visuals().dark_mode);
            let row = |ui: &mut egui::Ui, label: &str, hint: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
                ui.horizontal(|ui| {
                    ui.set_min_height(32.0);
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - 150.0).max(120.0));
                        ui.label(RichText::new(label).size(13.5).color(t.text));
                        if !hint.is_empty() {
                            ui.add(egui::Label::new(RichText::new(hint).size(12.0).color(t.tertiary)).wrap());
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| add(ui));
                });
                ui.add_space(4.0);
            };
            let two = w >= 640.0;
            let col_w = if two { (w - 28.0) / 2.0 } else { w };
            let mut left = |ui: &mut egui::Ui| {
                ui.set_width(col_w);
                row(ui, crate::tr!("Limite globale", "Global limit"), crate::tr!("Aucune session ne va au-delà, tous projets confondus", "No session goes past it, all projects together"), &mut |ui| {
                    ui.add(egui::DragValue::new(&mut o.global_limit).range(5.0..=100.0).speed(0.5).suffix(" %"));
                });
                row(ui, crate::tr!("Part par projet", "Share per project"), crate::tr!("Par défaut, modifiable au lancement", "By default, can be changed at start"), &mut |ui| {
                    ui.add(egui::DragValue::new(&mut o.default_limit).range(1.0..=100.0).speed(0.5).suffix(" %"));
                });
                row(ui, crate::tr!("Commits autorisés", "Commits allowed"), crate::tr!("Sur leur branche, jamais de push", "On their branch, never a push"), &mut |ui| {
                    super::settings::switch(ui, &mut o.allow_commits, &lk);
                });
            };
            let mut right = |ui: &mut egui::Ui| {
                ui.set_width(col_w);
                row(ui, crate::tr!("Bilan de la journée", "Daily summary"), crate::tr!("Quand plusieurs projets ont travaillé", "When several projects worked"), &mut |ui| {
                    let mut h = o.report_hour as u32;
                    if ui.add(egui::DragValue::new(&mut h).range(0..=23).suffix(" h")).changed() {
                        o.report_hour = h as u8;
                    }
                });
                row(ui, crate::tr!("Tours au plus", "Turns at most"), crate::tr!("Par session", "Per session"), &mut |ui| {
                    ui.add(egui::DragValue::new(&mut o.max_turns).range(1..=200));
                });
            };
            if two {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 28.0;
                    ui.vertical(|ui| left(ui));
                    ui.vertical(|ui| right(ui));
                });
            } else {
                left(ui);
                right(ui);
            }
            ui.collapsing(RichText::new(crate::tr!("Commandes autorisées", "Allowed commands")).size(13.0).color(t.secondary), |ui| {
                ui.label(
                    RichText::new(crate::tr!("Déjà permis : lire et modifier les fichiers de la copie, git status / diff / log, cargo, npm, pnpm, yarn, node, python, pytest, make, go (et git add / commit si les commits sont autorisés). Toujours refusés : git push, reset, checkout, switch, rm -rf, sudo, curl, wget. Une autre commande est refusée, l'IA continue sans.", "Already allowed: reading and editing the copy's files, git status / diff / log, cargo, npm, pnpm, yarn, node, python, pytest, make, go (and git add / commit when commits are allowed). Always refused: git push, reset, checkout, switch, rm -rf, sudo, curl, wget. Any other command is refused and the AI goes on without it."))
                        .size(12.5)
                        .color(t.tertiary),
                );
                ui.add(egui::TextEdit::singleline(&mut o.extra_tools).hint_text(crate::tr!("En plus, ex. Bash(just:*), Bash(docker compose:*)", "More, e.g. Bash(just:*), Bash(docker compose:*)")).desired_width(f32::INFINITY).font(FontId::monospace(12.5)));
            });
        });
        if *o != before {
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    fn running_card(&mut self, ui: &mut egui::Ui, t: &Tone) {
        if self.orch.runs.is_empty() {
            return;
        }
        let dark = ui.visuals().dark_mode;
        let watching = self.orch.watching.clone();
        let mut stop: Option<(usize, bool)> = None;
        let mut watch: Option<Option<String>> = None;
        let n = self.orch.runs.len();
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(ui, crate::tr!("EN COURS", "RUNNING"), &n.to_string(), t);
            for (i, r) in self.orch.runs.iter_mut().enumerate() {
                egui::Frame::new().fill(super::projects::GREEN.gamma_multiply(0.06)).stroke(egui::Stroke::new(1.0, super::projects::GREEN.gamma_multiply(0.3))).corner_radius(10.0).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 18.0), egui::Sense::hover());
                    super::projects::pulse_dot(ui, dot.center(), super::projects::GREEN);
                    ui.label(RichText::new(&r.title).font(semibold(14.5)).color(t.text));
                    let state = match r.phase {
                        Phase::Starting(_) => crate::tr!("démarre", "starting").to_string(),
                        Phase::Summary(_) => crate::tr!("compte rendu", "summary").to_string(),
                        Phase::Closing(_) => crate::tr!("se ferme", "closing").to_string(),
                        _ => format!("tour {}", r.turns + 1),
                    };
                    ui.label(RichText::new(state).size(12.5).color(t.tertiary));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if super::projects::action_link(ui, crate::tr!("Forcer l'arrêt", "Force stop")).on_hover_text(crate::tr!("Ferme Claude Code tout de suite, sans compte rendu", "Closes Claude Code right away, without a summary")).clicked() {
                            stop = Some((i, true));
                        }
                        if r.stop.is_none() && super::projects::action_link(ui, crate::tr!("Arrêter", "Stop")).on_hover_text(crate::tr!("Finit le tour en cours, écrit le compte rendu, puis s'arrête", "Finishes the current turn, writes the summary, then stops")).clicked() {
                            stop = Some((i, false));
                        }
                        let open = watching.as_deref() == Some(r.id.as_str());
                        if super::projects::action_link(ui, if open { crate::tr!("Masquer", "Hide") } else { crate::tr!("Regarder", "Watch") }).clicked() {
                            watch = Some(if open { None } else { Some(r.id.clone()) });
                        }
                    });
                });
                // Use of the project's share.
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    let w = (ui.available_width() - 150.0).max(80.0);
                    let (bar, _) = ui.allocate_exact_size(egui::vec2(w, 8.0), egui::Sense::hover());
                    ui.painter().rect_filled(bar, 4.0, t.fill);
                    let frac = super::projects::smooth(ui, ("run-pct", &r.id), (r.pct / r.limit.max(0.1)).clamp(0.0, 1.0));
                    ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac, 8.0)), 4.0, if frac > 0.85 { super::projects::ORANGE } else { super::projects::GREEN });
                    ui.label(RichText::new(format!("{:.1} / {:.0} %", r.pct, r.limit)).size(12.5).color(t.secondary));
                });
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    let mut job = egui::text::LayoutJob::single_section(r.last_line.clone(), egui::TextFormat::simple(FontId::proportional(12.5), t.tertiary));
                    job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
                    super::projects::cut_label(ui, job);
                });
                if let Some(why) = &r.stop {
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.label(RichText::new(crate::trf!("S'arrête : {why}", "Stopping: {why}")).size(12.5).color(super::projects::ORANGE));
                    });
                }
                if watching.as_deref() == Some(r.id.as_str()) {
                    egui::Frame::new().fill(Color32::from_rgb(20, 20, 20)).corner_radius(8.0).inner_margin(8).show(ui, |ui| {
                        ui.set_height(420.0);
                        ui.set_width(ui.available_width());
                        super::term_ui::terminal_view(ui, &mut r.term, Color32::from_rgb(20, 20, 20), dark || true, false);
                    });
                }
                });
                ui.add_space(8.0);
            }
        });
        if let Some(w) = watch {
            self.orch.watching = w;
        }
        if let Some((i, force)) = stop {
            if force {
                let r = &mut self.orch.runs[i];
                r.stop = Some(crate::tr!("arrêtée à la main", "stopped by hand").into());
                self.finish_run(i, crate::tr!("arrêtée à la main", "stopped by hand"), "");
                self.orch.runs.remove(i);
            } else {
                self.orch.runs[i].stop = Some(crate::tr!("arrêtée à la main", "stopped by hand").into());
            }
        }
    }

    /// The projects that can be started; returns a message to show.
    fn launch_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, t: &Tone) -> Option<super::notices::Notice> {
        let mut msg = None;
        let projects: Vec<(String, String, String)> = self
            .ws
            .projects()
            .iter()
            .filter_map(|p| p.project.as_ref().map(|x| (p.id.clone(), p.display_title(), x.folder.trim().to_string())))
            .filter(|(_, _, f)| !f.is_empty())
            .collect();
        let (dl, dc) = (self.ws.settings.orch.default_limit, self.ws.settings.orch.allow_commits);
        for (id, _, _) in &projects {
            self.orch.drafts.entry(id.clone()).or_insert(Draft { mission: String::new(), limit: dl, commits: dc, pick: false });
        }
        let running: Vec<String> = self.orch.runs.iter().map(|r| r.project.clone()).collect();
        let plugged = self.ais.plugged().contains(&crate::setup::Ai::ClaudeCode);
        let mut start: Vec<String> = vec![];
        let picked: Vec<String> = projects.iter().filter(|(id, _, _)| self.orch.drafts.get(id).is_some_and(|d| d.pick) && !running.contains(id)).map(|(id, _, _)| id.clone()).collect();
        let picked_sum: f32 = picked.iter().filter_map(|id| self.orch.drafts.get(id)).map(|d| d.limit).sum();
        let global = self.ws.settings.orch.global_limit;
        let open_steps: HashMap<String, usize> = projects
            .iter()
            .map(|(id, _, _)| (id.clone(), self.ws.page(id).and_then(|p| p.project.as_ref()).map(|x| x.steps.iter().filter(|s| s.status != crate::project::StepStatus::Done).count()).unwrap_or(0)))
            .collect();
        let lk = super::settings::look(ui.visuals().dark_mode);
        let drafts = &mut self.orch.drafts;
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(ui, crate::tr!("LANCER UNE SESSION", "START A SESSION"), "Claude Code", t);
            if !plugged {
                ui.label(RichText::new(crate::tr!("Branchez d'abord Claude Code (Réglages › IA).", "Plug in Claude Code first (Settings › AI).")).color(super::projects::ORANGE));
            }
            if projects.is_empty() {
                ui.label(RichText::new(crate::tr!("Aucun projet avec un dossier pour l'instant.", "No project with a folder yet.")).color(t.tertiary).italics());
            }
            for (id, title, folder) in &projects {
                let d = drafts.get_mut(id).unwrap();
                let busy = running.contains(id);
                let bg = super::projects::hover_begin(ui);
                let open = open_steps.get(id).copied().unwrap_or(0);
                let row = ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.add_enabled(!busy, egui::Checkbox::without_text(&mut d.pick)).on_hover_text(crate::tr!("Sélectionner pour lancer plusieurs projets ensemble", "Select to start several projects together"));
                        // A round badge with the project's initial.
                        let (r, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
                        let hue = project_color(title);
                        ui.painter().circle_filled(r.center(), 16.0, hue.gamma_multiply(0.18));
                        let initial: String = title.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, initial, semibold(15.0), hue);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(title).font(semibold(14.5)).color(t.text));
                                let (txt, col) = if busy { (crate::tr!("au travail", "at work").to_string(), super::projects::GREEN) } else if open == 0 { (crate::tr!("aucune étape ouverte", "no open step").to_string(), t.tertiary) } else { (crate::trf!("{open} étape(s) ouverte(s)", "{open} open step(s)"), t.secondary) };
                                ui.label(RichText::new(txt).size(12.0).color(col));
                            });
                            let mut job = egui::text::LayoutJob::single_section(folder.clone(), egui::TextFormat::simple(FontId::monospace(11.0), t.tertiary));
                            job.wrap = egui::text::TextWrapping::truncate_at_width((ui.available_width() - 250.0).max(80.0));
                            super::projects::cut_label(ui, job);
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if busy {
                                let (r, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                                super::projects::pulse_dot(ui, r.center(), super::projects::GREEN);
                            } else if start_button(ui, plugged, t).on_disabled_hover_text(crate::tr!("Branchez d'abord Claude Code", "Plug in Claude Code first")).clicked() {
                                start.push(id.clone());
                            }
                            ui.add_space(4.0);
                            super::settings::switch(ui, &mut d.commits, &lk).on_hover_text(crate::tr!("L'IA peut faire des commits sur sa branche (jamais de push)", "The AI may commit on its branch (never push)"));
                            ui.label(RichText::new("commits").size(12.0).color(t.tertiary));
                            ui.add_space(6.0);
                            ui.add(egui::DragValue::new(&mut d.limit).range(1.0..=100.0).speed(0.5).suffix(" %")).on_hover_text(crate::tr!("Part de la session de 5 h que ce projet peut utiliser", "Share of the 5-hour window this project may use"));
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(70.0);
                        ui.add(egui::TextEdit::singleline(&mut d.mission).hint_text(default_mission()).desired_width(f32::INFINITY).font(FontId::proportional(13.0)).margin(egui::Margin::symmetric(8, 5)));
                    });
                });
                super::projects::hover_end(ui, bg, row.response.rect, egui::Id::unique(("orch-row", id)), t);
                ui.add_space(10.0);
            }
            if !picked.is_empty() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(plugged, egui::Button::new(RichText::new(crate::trf!("Lancer la sélection ({})", "Start the selection ({})", picked.len())).size(13.5))).clicked() {
                        start.extend(picked.iter().cloned());
                    }
                    let note = crate::trf!("{} projet(s) en parallèle · {picked_sum:.0} % au total pour une limite globale de {global:.0} %", "{} project(s) in parallel · {picked_sum:.0} % in total for a global limit of {global:.0} %", picked.len());
                    ui.label(RichText::new(note).size(12.5).color(if picked_sum > global { super::projects::ORANGE } else { t.tertiary }));
                });
            }
        });
        start.dedup();
        for id in start {
            let d = self.orch.drafts.get(&id).cloned().unwrap();
            let title = self.ws.page(&id).map(|p| p.display_title()).unwrap_or_default();
            match self.launch_run(ctx, &id, &d.mission, d.limit, d.commits) {
                Ok(()) => {
                    if let Some(x) = self.orch.drafts.get_mut(&id) {
                        x.pick = false;
                    }
                    msg = Some(super::notices::Notice::Ack(crate::trf!("Session autonome lancée sur {title}.", "Autonomous session started on {title}.")));
                }
                Err(e) => msg = Some(super::notices::Notice::Error(format!("{title} : {e}."))),
            }
        }
        msg
    }

    fn history_card(&mut self, ui: &mut egui::Ui, t: &Tone, actions: &mut Vec<Action>) {
        let done: Vec<crate::model::RunRecord> = self.ws.runs.iter().rev().filter(|r| !r.outcome.is_empty()).take(20).cloned().collect();
        if done.is_empty() {
            return;
        }
        let mut act: Option<(String, &'static str)> = None;
        let confirm = self.orch.confirm.clone();
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(ui, crate::tr!("SESSIONS TERMINÉES", "FINISHED SESSIONS"), &done.len().to_string(), t);
            for r in &done {
                let bg = super::projects::hover_begin(ui);
                let row = ui.horizontal_top(|ui| {
                    let (b, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                    let (mark, col) = outcome_mark(&r.outcome, &r.closed, t);
                    ui.painter().circle_filled(b.center(), 12.0, col.gamma_multiply(0.15));
                    ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, mark, FontId::proportional(13.0), col);
                    ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new(&r.title).font(semibold(14.0)).color(t.text));
                        ui.label(RichText::new(format!("{} · {} → {}", auto::local_day(r.started), auto::hm(r.started), auto::hm(r.ended.max(r.started)))).size(12.5).color(t.tertiary));
                        ui.label(RichText::new(crate::trf!("{} · {} étape(s) · ≈ {:.0} %", "{} · {} step(s) · ≈ {:.0} %", r.outcome, r.steps_done.len(), r.pct)).size(12.5).color(t.secondary));
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(RichText::new(&r.branch).font(FontId::monospace(11.5)).color(t.tertiary));
                        if let Some(p) = &r.report {
                            if super::projects::action_link(ui, crate::tr!("Rapport", "Report")).clicked() {
                                actions.push(Action::Open(p.clone()));
                            }
                        }
                        if !r.closed.is_empty() {
                            ui.label(RichText::new(&r.closed).size(12.5).color(t.tertiary));
                            return;
                        }
                        if super::projects::action_link(ui, crate::tr!("Ouvrir la copie", "Open the copy")).clicked() {
                            let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
                            let _ = std::process::Command::new(opener).arg(&r.worktree).spawn();
                        }
                        let asking = confirm.as_ref().filter(|(id, _)| *id == r.id).map(|(_, a)| *a);
                        match asking {
                            Some(a) => {
                                let what = if a == "merge" { crate::tr!("Fusionner dans votre branche actuelle ?", "Merge into your current branch?") } else { crate::tr!("Supprimer la copie et sa branche, définitivement ?", "Delete the copy and its branch, for good?") };
                                ui.label(RichText::new(what).size(12.5).color(super::projects::ORANGE));
                                if super::projects::action_link(ui, crate::tr!("Oui", "Yes")).clicked() {
                                    act = Some((r.id.clone(), a));
                                }
                                if super::projects::action_link(ui, crate::tr!("Non", "No")).clicked() {
                                    act = Some((r.id.clone(), "cancel"));
                                }
                            }
                            None => {
                                if super::projects::action_link(ui, crate::tr!("Fusionner", "Merge")).on_hover_text(crate::tr!("Intègre ce travail dans la branche ouverte dans le dossier du projet", "Brings this work into the branch open in the project's folder")).clicked() {
                                    act = Some((r.id.clone(), "ask-merge"));
                                }
                                if super::projects::action_link(ui, crate::tr!("Abandonner", "Discard")).clicked() {
                                    act = Some((r.id.clone(), "ask-discard"));
                                }
                            }
                        }
                    });
                    });
                });
                super::projects::hover_end(ui, bg, row.response.rect, egui::Id::unique(("orch-done", &r.id)), t);
                ui.add_space(8.0);
            }
        });
        if let Some((id, a)) = act {
            match a {
                "ask-merge" => self.orch.confirm = Some((id, "merge")),
                "ask-discard" => self.orch.confirm = Some((id, "discard")),
                "cancel" => self.orch.confirm = None,
                _ => {
                    self.orch.confirm = None;
                    if let Some(rec) = self.ws.runs.iter().find(|r| r.id == id).cloned() {
                        let res = if a == "merge" { auto::merge(&rec) } else { auto::discard(&rec) };
                        match res {
                            Ok(m) => {
                                if let Some(x) = self.ws.runs.iter_mut().find(|r| r.id == id) {
                                    x.closed = if a == "merge" { crate::tr!("fusionné", "merged").into() } else { crate::tr!("abandonné", "discarded").into() };
                                }
                                self.dirty_since.get_or_insert_with(Instant::now);
                                self.info(m);
                            }
                            Err(e) => self.fail(format!("{} : {e}", rec.title)),
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------ the inbox

    pub(super) fn inbox_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let t = tone(ui.visuals().dark_mode);
        let avail = ui.available_width();
        let width = 860.0f32.min(avail - 40.0).max(320.0);
        let mut open: Option<usize> = None;
        let mut remove: Option<usize> = None;
        let mut all_read = false;
        ui.add_space(50.0);
        ui.horizontal(|ui| {
            ui.add_space(((avail - width) / 2.0).max(20.0));
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(crate::tr!("Boîte de réception", "Inbox")).font(bold(30.0)).color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.ws.unread() > 0 && super::projects::action_link(ui, crate::tr!("Tout marquer comme lu", "Mark all as read")).clicked() {
                            all_read = true;
                        }
                    });
                });
                ui.label(RichText::new(crate::tr!("Les questions des IA, la fin de leurs sessions de travail, les rapports (rangés dans la page « Rapports des IA ») et les messages de l'application, dont les erreurs.", "The AIs' questions, the end of their work sessions, the reports (kept in the « AI reports » page) and the app's messages, errors included.")).color(t.secondary));
                ui.add_space(12.0);
                if self.ws.inbox.is_empty() {
                    ui.label(RichText::new(crate::tr!("Rien pour l'instant.", "Nothing yet.")).color(t.tertiary).italics());
                }
                for (i, m) in self.ws.inbox.iter().enumerate().rev() {
                    let (accent, kind) = match m.kind.as_str() {
                        "question" => (Color32::from_rgb(144, 101, 176), crate::tr!("Question", "Question")),
                        "fin" => (super::projects::GREEN, crate::tr!("Session", "Session")),
                        "rapport" => (super::projects::BLUE, crate::tr!("Rapport", "Report")),
                        "erreur" => (Color32::from_rgb(224, 62, 62), crate::tr!("Erreur", "Error")),
                        _ => (t.tertiary, crate::tr!("Info", "Info")),
                    };
                    let bg = super::projects::hover_begin(ui);
                    let row = ui.horizontal_top(|ui| {
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 22.0), egui::Sense::hover());
                        if !m.read {
                            ui.painter().circle_filled(dot.center(), 4.0, super::projects::BLUE);
                        }
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width() - 30.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(kind).size(11.5).color(accent));
                                ui.label(RichText::new(&m.title).font(if m.read { FontId::proportional(14.0) } else { semibold(14.0) }).color(t.text));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(super::chrome::relative_time(m.at)).size(12.0).color(t.tertiary));
                                });
                            });
                            if !m.text.is_empty() {
                                ui.add(egui::Label::new(RichText::new(&m.text).size(13.0).color(t.secondary)).wrap());
                            }
                        });
                        let x = ui.add(egui::Button::new(RichText::new("×").color(t.tertiary)).frame(false)).on_hover_text(crate::tr!("Supprimer", "Delete"));
                        if x.clicked() {
                            remove = Some(i);
                        }
                    });
                    let resp = ui.interact(row.response.rect, egui::Id::unique(("inbox", &m.id)), egui::Sense::click());
                    super::projects::hover_end(ui, bg, row.response.rect, egui::Id::unique(("inbox-h", &m.id)), &t);
                    if resp.clicked() && remove.is_none() {
                        open = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        });
        if all_read {
            for m in self.ws.inbox.iter_mut() {
                m.read = true;
            }
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        if let Some(i) = remove {
            self.ws.inbox.remove(i);
            self.dirty_since.get_or_insert_with(Instant::now);
        } else if let Some(i) = open {
            let m = &mut self.ws.inbox[i];
            m.read = true;
            if let Some(p) = m.page.clone().filter(|p| !p.is_empty()) {
                actions.push(Action::Open(p));
            } else if !m.project.is_empty() {
                actions.push(Action::Open(m.project.clone()));
            }
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }
}


/// A ring gauge: `value` (0..1, None when unknown) and a tick at `limit`.
fn ring(ui: &egui::Ui, c: egui::Pos2, r: f32, value: Option<f32>, limit: f32, color: Color32, t: &Tone) {
    let p = ui.painter();
    let arc = |from: f32, to: f32, col: Color32, w: f32| {
        let n = 48;
        let pts: Vec<egui::Pos2> = (0..=n)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * (from + (to - from) * i as f32 / n as f32);
                c + egui::vec2(a.cos(), a.sin()) * r
            })
            .collect();
        p.add(egui::Shape::line(pts, egui::Stroke::new(w, col)));
    };
    arc(0.0, 1.0, t.fill, 6.0);
    if let Some(v) = value {
        let v = super::projects::smooth(ui, "orch-ring", v.clamp(0.0, 1.0));
        if v > 0.001 {
            arc(0.0, v, if v >= limit { Color32::from_rgb(224, 62, 62) } else { color }, 6.0);
        }
    }
    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * limit.clamp(0.0, 1.0);
    let d = egui::vec2(a.cos(), a.sin());
    p.line_segment([c + d * (r - 6.0), c + d * (r + 6.0)], egui::Stroke::new(2.0, super::projects::ORANGE));
}

/// A colour per project, stable (from its name).
fn project_color(title: &str) -> Color32 {
    const HUES: [Color32; 6] = [
        Color32::from_rgb(35, 131, 226),
        Color32::from_rgb(68, 160, 110),
        Color32::from_rgb(212, 76, 125),
        Color32::from_rgb(217, 115, 13),
        Color32::from_rgb(144, 101, 176),
        Color32::from_rgb(0, 150, 160),
    ];
    HUES[(hash(title) % HUES.len() as u64) as usize]
}

/// The mark and colour of a finished session: done, stopped on a limit or a block, failed.
fn outcome_mark(outcome: &str, closed: &str, t: &Tone) -> (&'static str, Color32) {
    let o = outcome.to_lowercase();
    if !closed.is_empty() {
        return ("✔", t.tertiary);
    }
    if o.contains("termin") || o.contains("finish") || o.contains("fini") || o.contains("done") {
        ("✔", super::projects::GREEN)
    } else if o.contains("bloq") || o.contains("block") || o.contains("limit") {
        ("!", super::projects::ORANGE)
    } else if o.contains("erreur") || o.contains("error") || o.contains("interromp") || o.contains("interrupt") {
        ("×", Color32::from_rgb(224, 62, 62))
    } else {
        ("■", t.tertiary)
    }
}

/// "Lancer": a filled button.
fn start_button(ui: &mut egui::Ui, enabled: bool, t: &Tone) -> egui::Response {
    let text = crate::tr!("Lancer", "Start");
    let g = ui.painter().layout_no_wrap(text.to_string(), semibold(13.0), Color32::WHITE);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(g.size() + egui::vec2(24.0, 12.0), sense);
    let a = ui.ctx().animate_bool_with_time(resp.id.with("hov"), resp.hovered() && enabled, 0.15);
    let base = if enabled { super::projects::BLUE } else { t.fill };
    let fill = base.lerp_to_gamma(Color32::WHITE, 0.12 * a);
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, if enabled { Color32::WHITE } else { t.tertiary });
    let resp = if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    // Disabled: report it so on_disabled_hover_text works.
    if enabled { resp } else { resp.on_hover_text(crate::tr!("Branchez d'abord Claude Code", "Plug in Claude Code first")) }
}
