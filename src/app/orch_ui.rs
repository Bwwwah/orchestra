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
    /// Paused: the AI waits for the person's answer (see `LiveRun::waiting`).
    Waiting,
}

/// What a paused session waits for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Wait {
    /// Claude Code asks whether the folder is safe.
    Trust,
    /// Claude Code asks to allow a command or an edit (or a question it didn't explain).
    Permission,
    /// The AI asked the person a question (ask_human) and has nothing else to do.
    Question,
}

impl Wait {
    pub fn label(self) -> &'static str {
        match self {
            Wait::Trust => crate::tr!("En pause : Claude Code demande si le dossier est sûr. Répondez dans le terminal (« Regarder ») : la session repart ensuite.", "Paused: Claude Code asks whether the folder is safe. Answer in the terminal (« Watch »): the session goes on afterwards."),
            Wait::Permission => crate::tr!("En pause : Claude Code demande une autorisation. Répondez dans le terminal (« Regarder ») : la session repart ensuite.", "Paused: Claude Code asks for a permission. Answer in the terminal (« Watch »): the session goes on afterwards."),
            Wait::Question => crate::tr!("En pause : l'IA attend la réponse à sa question (boîte de réception) ; elle repart dès qu'elle l'a.", "Paused: the AI waits for the answer to its question (inbox); it goes on as soon as it has it."),
        }
    }
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
    /// The project's repository (the copy's origin), for its trust in Claude Code.
    root: String,
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
    /// Stopped by the 5-hour or weekly limit: when that window resets (to start again).
    quota_reset: Option<u64>,
    /// The week's figure when the running turn began.
    week_at_turn: Option<f32>,
    /// The most one turn took from the week so far (percentage points).
    week_turn: f32,
    /// Endless mode and its end conditions (None: the usual turn limit and [[FINI]]).
    pub endless: Option<crate::model::EndConditions>,
    /// Turns in a row the AI said it had nothing left ([[FINI]]).
    done_streak: u32,
    /// Paused, waiting for the person (with `Phase::Waiting`).
    pub waiting: Option<Wait>,
    /// The screen when the pause began: the answer given in the terminal changes it.
    paused_screen: u64,
}

impl LiveRun {
    /// Put the session on hold until the person answers.
    fn pause(&mut self, why: Wait) {
        self.waiting = Some(why);
        self.phase = Phase::Waiting;
        self.paused_screen = self.screen_hash;
        self.last_line = why.label().into();
    }

    /// The person answered what the session waits for: in the terminal (the question is
    /// gone from a screen that moved since the pause), or in the inbox (`question_left`:
    /// one of its questions still has no answer).
    fn answered(&self, screen: &str, idle: Duration, question_left: bool) -> bool {
        let moved = self.screen_hash != self.paused_screen && idle > Duration::from_millis(1500);
        match self.waiting {
            Some(Wait::Trust) => moved && auto::read_screen(screen) != Screen::Trust,
            Some(Wait::Permission) => moved && auto::read_screen(screen) != Screen::Permission,
            Some(Wait::Question) => !question_left,
            None => false,
        }
    }

    /// Leave the pause (« Reprendre »). A question still on the screen is refused, as
    /// without the pause.
    pub fn resume(&mut self) {
        let Some(why) = self.waiting.take() else { return };
        let now = Instant::now();
        let still = auto::read_screen(&self.term.text());
        match why {
            Wait::Trust if still == Screen::Trust => {
                // Never accept for them; Claude Code closes and the session ends.
                self.term.write(b"\x1b");
                self.stop = Some(crate::tr!("le dossier n'a pas été accepté dans Claude Code", "the folder wasn't accepted in Claude Code").into());
                self.phase = Phase::Starting(now);
            }
            Wait::Trust => {
                self.phase = Phase::Starting(now);
                self.last_line = crate::tr!("Démarrage de Claude Code…", "Starting Claude Code…").into();
            }
            Wait::Permission if still == Screen::Permission => {
                self.term.write(b"\x1b");
                self.denials += 1;
                self.last_line = crate::tr!("Commande refusée (non autorisée en autonome)", "Command refused (not allowed when autonomous)").into();
                self.phase = Phase::Denied(now);
            }
            Wait::Permission => {
                self.phase = Phase::Working(now);
                self.last_line = crate::trf!("Tour {} en cours…", "Turn {} running…", self.turns + 1);
            }
            Wait::Question => {
                match &self.endless {
                    Some(c) => self.term.paste(auto::endless_prompt(!c.when_done)),
                    None => self.term.paste(auto::continue_prompt()),
                }
                self.phase = Phase::Typed(now);
            }
        }
        self.screen_changed = now;
    }
}

#[derive(Clone)]
struct Draft {
    mission: String,
    limit: f32,
    commits: bool,
    pick: bool,
    /// Endless mode, and its end conditions as set in the launch card.
    endless: bool,
    end: crate::model::EndConditions,
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
    /// When the 5-hour and the weekly windows start again (ms since epoch).
    pub five_reset: Option<u64>,
    pub week_reset: Option<u64>,
    /// The 5-hour windows of the last 24 hours, added up.
    pub day_total: Option<f32>,
    /// What went from the week in the last 24 hours, and over how long the readings go back.
    pub day_week: Option<(f32, u64)>,
    /// Time of the reading the 24-hour figures were worked out from.
    history_at: u64,
    usage_polled: Option<Instant>,
    resume_checked: Option<Instant>,
    daily_checked: Option<Instant>,
    confirm: Option<(String, &'static str)>,
    /// An endless session on main waiting for the person to confirm the warning: a project
    /// id, or SELECTION for « Lancer la sélection ».
    confirm_main: Option<String>,
    started: bool,
    /// Finished sessions to merge into main by themselves, once their terminal is closed.
    to_main: Vec<String>,
    /// Keeps the computer awake while sessions run.
    awake: crate::awake::Awake,
    /// When closing the window last only hid it because sessions were running.
    close_held: Option<Instant>,
}

/// `Orch::confirm_main` for the « Lancer la sélection » button.
const SELECTION: &str = "*selection*";

/// An endless session whose work goes into main by itself: hours of unreviewed commits
/// land on the main branch. Worth a warning before it starts.
fn endless_on_main(endless: bool, to_main: bool) -> bool {
    endless && to_main
}

fn endless_main_warning() -> &'static str {
    crate::tr!(
        "Attention : mode sans fin avec « sur main » coché. Le travail de chaque session, y compris les étapes que l'IA s'invente, sera fusionné tout seul dans main sans relecture, potentiellement pendant des heures.",
        "Careful: endless mode with « on main » ticked. Each session's work, including the steps the AI makes up, will be merged into main by itself without review, possibly for hours."
    )
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
    fn launch_run(&mut self, ctx: &egui::Context, project: &str, mission: &str, limit: f32, commits: bool, endless: Option<crate::model::EndConditions>) -> Result<(), String> {
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
                let left = self.orch.five_reset.and_then(|r| crate::usage::until(r, crate::model::now_ms()));
                let when = left.map(|l| crate::trf!(" ; la fenêtre de 5 h repart à zéro dans {}", "; the 5-hour window resets in {}", crate::usage::left_text(l))).unwrap_or_default();
                return Err(crate::trf!("la limite globale ({:.0} %) est déjà atteinte{when}", "the global limit ({:.0} %) is already reached{when}", self.ws.settings.orch.global_limit));
            }
        }
        let taken: f32 = self.orch.runs.iter().map(|r| r.limit).sum();
        if taken + limit > self.ws.settings.orch.global_limit + 0.01 {
            return Err(crate::trf!(
                "{limit:.0} % pour ce projet dépasserait la limite globale ({:.0} %), dont {taken:.0} % sont déjà pris",
                "{limit:.0} % for this project would go past the global limit ({:.0} %), {taken:.0} % of which is already taken",
                self.ws.settings.orch.global_limit
            ));
        }
        if let Some(w) = self.orch.week {
            if crate::usage::near_reserve(w, self.ws.settings.orch.week_limit, 0.0) {
                return Err(crate::trf!(
                    "la réserve minimum de la semaine ({:.0} %) est atteinte ou trop proche",
                    "the week's minimum reserve ({:.0} %) is reached or too near",
                    self.ws.settings.orch.reserve()
                ));
            }
        }
        // The limits are checked with Claude Code's own figures: they come through the status line.
        if !crate::usage::tracking_on() {
            crate::usage::start_tracking(&std::env::current_exe().unwrap_or_default())?;
            self.ais.tracking = true;
        }
        // The session checks interfaces with take_screenshot, which macOS only allows once
        // Orchestra has the « Screen Recording » permission: ask for it now (once per launch).
        crate::screen::ensure_permission();
        let steps_at_start: Vec<(String, bool)> = pr.steps.iter().map(|s| (s.title.clone(), s.status == crate::project::StepStatus::Done)).collect();
        let copy = auto::make_copy(&folder, &title)?;
        let agent = Agent::ClaudeCode;
        let o = &self.ws.settings.orch;
        let cmd = agent.command(commits, &o.extra_tools);
        let c2 = ctx.clone();
        let mut term = Term::spawn_once(&crate::trf!("Autonome · {title}", "Autonomous · {title}"), &copy.worktree, &cmd, move || c2.request_repaint_after(Duration::from_millis(200)))?;
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
            mission: mission.clone(),
            limit,
            endless: endless.clone(),
            ..Default::default()
        };
        // A new session replaces any planned restart of this project.
        for x in self.ws.runs.iter_mut().filter(|x| x.project == project) {
            x.resume_at = 0;
        }
        self.ws.runs.push(rec);
        self.dirty_since.get_or_insert_with(Instant::now);
        self.orch.runs.push(LiveRun {
            id,
            project: project.to_string(),
            title,
            agent,
            term,
            phase: Phase::Starting(Instant::now()),
            next: Some(auto::mission_prompt(&self.ws.page(project).map(|p| p.display_title()).unwrap_or_default(), project, &copy.branch, &mission, commits) + &endless.as_ref().map(|c| auto::endless_rule(project, !c.when_done)).unwrap_or_default()),
            quota_reset: None,
            week_at_turn: None,
            week_turn: 0.0,
            reader: auto::Reader::default(),
            since: std::time::SystemTime::now() - Duration::from_secs(2),
            base: copy.base,
            root: copy.root,
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
            endless,
            done_streak: 0,
            waiting: None,
            paused_screen: 0,
        });
        Ok(())
    }

    /// Once per frame: move every session along, follow the use of the subscription,
    /// write the day's report when it is time.
    /// Closing the window while autonomous sessions run only puts it in the Dock: Orchestra
    /// goes on driving them. Closing it again within a minute quits for real.
    fn keep_running_on_close(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.orch.runs.is_empty() {
            return;
        }
        if self.orch.close_held.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return;
        }
        self.orch.close_held = Some(Instant::now());
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        let n = self.orch.runs.len();
        self.info(crate::trf!(
            "{n} session(s) autonome(s) en cours : Orchestra continue dans le Dock et garde le Mac éveillé. Fermez à nouveau pour quitter et les arrêter.",
            "{n} autonomous session(s) running: Orchestra keeps going in the Dock and keeps the Mac awake. Close again to quit and stop them."
        ));
    }

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
        self.resume_tick(ctx);
        self.orch.awake.set(!self.orch.runs.is_empty());
        self.keep_running_on_close(ctx);
        // Developer aid: ORCHESTRA_AUTO_RUN=<project id> starts a session once Claude Code is seen.
        if let Ok(pid) = std::env::var("ORCHESTRA_AUTO_RUN") {
            if !pid.is_empty() && self.orch.runs.is_empty() && !self.ws.runs.iter().any(|r| r.project == pid) && self.ais.plugged().contains(&crate::setup::Ai::ClaudeCode) {
                let (l, c) = (self.ws.settings.orch.default_limit, true);
                if let Err(e) = self.launch_run(ctx, &pid, "", l, c, None) {
                    eprintln!("auto-run : {e}");
                }
            }
        }
        let now = Instant::now();
        let global = self.ws.settings.orch.global_limit;
        let week_limit = self.ws.settings.orch.week_limit;
        let pause = self.ws.settings.orch.pause_for_answer;
        let mut paused: Vec<(String, String, Wait)> = vec![];
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
                            Screen::Trust if auto::folder_trusted(&r.root) => {
                                // The person already trusts this repository in Claude Code;
                                // the copy is the same repository, in a new folder.
                                r.term.write(b"\r");
                                r.screen_changed = now;
                            }
                            Screen::Trust if pause => {
                                // The person decides, in the terminal.
                                r.pause(Wait::Trust);
                                paused.push((r.project.clone(), r.title.clone(), Wait::Trust));
                            }
                            Screen::Trust => {
                                // Never accept for them: it would turn on the repository's
                                // hooks, MCP servers and settings.
                                r.term.write(b"\x1b");
                                finished.push((i, crate::trf!(
                                    "Claude Code demande si le dossier est sûr ; ouvrez-le une fois dans « {} » et acceptez, puis relancez",
                                    "Claude Code asks whether the folder is safe; open it once in « {} » and accept, then start again",
                                    r.root
                                ), String::new()));
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
                        r.week_at_turn = self.orch.week;
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
                    // While Claude Code works, its spinner redraws the screen every second: a
                    // screen still for minutes, with nothing written to the session either, is
                    // a question we didn't recognise. Answer it like a permission.
                    let frozen = idle > Duration::from_secs(180)
                        && !r.reader.turn_over
                        && r.reader.last_growth.is_none_or(|g| g.elapsed() > Duration::from_secs(180));
                    if (auto::read_screen(&screen) == Screen::Permission && idle > Duration::from_millis(800)) || frozen {
                        if pause {
                            r.pause(Wait::Permission);
                            paused.push((r.project.clone(), r.title.clone(), Wait::Permission));
                            continue;
                        }
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
                        // What this turn took from the week (a reset in between counts for nothing).
                        if let (Some(a), Some(b)) = (r.week_at_turn, self.orch.week) {
                            if b >= a {
                                r.week_turn = r.week_turn.max(b - a);
                            }
                        }
                        auto::push_copy_later(r.worktree.clone());
                        let sig = auto::signal(&r.reader.last_text);
                        r.last_line = crate::richtext::one_line(&auto::strip_markers(&r.reader.last_text), 140);
                        let five = self.orch.five;
                        r.done_streak = if sig == Signal::Done { r.done_streak + 1 } else { 0 };
                        // Endless mode: its own end conditions replace [[FINI]] and the turn limit.
                        let endless_end = r.endless.as_ref().and_then(|c| {
                            let before = &r.steps_at_start;
                            let steps = self.ws.page(&r.project).and_then(|p| p.project.as_ref()).map(|pr| pr.steps.clone()).unwrap_or_default();
                            let done = |s: &crate::project::Step| s.status == crate::project::StepStatus::Done;
                            let steps_done = steps.iter().filter(|s| done(s) && !before.iter().any(|(b, d)| *b == s.title && *d)).count() as u32;
                            // Steps the AI added itself and hasn't done yet.
                            let added_open = steps.iter().filter(|s| !done(s) && !before.iter().any(|(b, _)| *b == s.title)).count();
                            if auto::too_many_self_steps(added_open) {
                                return Some(crate::trf!("l'IA a ajouté {added_open} étapes sans les faire", "the AI added {added_open} steps without doing them"));
                            }
                            c.reached(crate::model::now_ms().saturating_sub(r.started_wall), steps_done, r.turns, r.done_streak)
                        });
                        let reason = if let Some(s) = r.stop.clone() {
                            Some(s)
                        } else if too_long {
                            Some(crate::tr!("un tour a duré trop longtemps", "a turn took too long").into())
                        } else if let Some(why) = endless_end {
                            Some(why)
                        } else if sig == Signal::Done && r.endless.is_none() {
                            Some(crate::tr!("mission terminée", "mission complete").into())
                        } else if sig == Signal::Blocked {
                            Some(crate::tr!("l'IA est bloquée", "the AI is blocked").into())
                        } else if r.pct >= r.limit {
                            Some(crate::trf!("limite du projet atteinte ({:.0} %)", "project limit reached ({:.0} %)", r.limit))
                        } else if five.is_some_and(|f| f >= global) {
                            r.quota_reset = self.orch.five_reset;
                            Some(crate::trf!("limite globale atteinte ({global:.0} %)", "global limit reached ({global:.0} %)"))
                        } else if self.orch.week.is_some_and(|w| crate::usage::near_reserve(w, week_limit, r.week_turn)) {
                            // Paused before the next turn bites into the reserve; starts again once the week resets.
                            r.quota_reset = self.orch.week_reset;
                            Some(crate::trf!("pause : la réserve minimum de la semaine ({:.0} %) approche", "paused: the week's minimum reserve ({:.0} %) is near", 100.0 - week_limit))
                        } else if !r.measured && r.turns >= 2 {
                            Some(crate::tr!("utilisation de l'abonnement impossible à mesurer", "subscription use can't be measured").into())
                        } else if r.endless.is_none() && r.turns >= r.max_turns {
                            Some(crate::trf!("{} tours atteints", "{} turns reached", r.max_turns))
                        } else {
                            None
                        };
                        // A question to the person still unanswered: wait for it rather
                        // than send the AI on.
                        let asked = pause && reason.is_none() && crate::model::question_pending(&self.ws.inbox, &r.project, r.started_wall);
                        if asked {
                            r.pause(Wait::Question);
                            paused.push((r.project.clone(), r.title.clone(), Wait::Question));
                            continue;
                        }
                        match reason {
                            Some(why) => {
                                r.stop = Some(why.clone());
                                r.summarizing = true;
                                r.term.paste(&auto::summary_prompt(&r.project, &why));
                            }
                            None if r.endless.is_some() => r.term.paste(auto::endless_prompt(r.endless.as_ref().is_some_and(|c| !c.when_done))),
                            None => r.term.paste(auto::continue_prompt()),
                        }
                        r.phase = Phase::Typed(now);
                    }
                }
                // Paused: nothing moves until the person answers, then the AI goes on.
                Phase::Waiting => {
                    let left = crate::model::question_pending(&self.ws.inbox, &r.project, r.started_wall);
                    if r.answered(&screen, idle, left) {
                        r.resume();
                    }
                }
                Phase::Closing(t) => {
                    if t.elapsed() > Duration::from_secs(3) || r.term.exited() {
                        finished.push((i, String::new(), String::new()));
                    }
                }
            }
        }
        for (project, title, why) in paused {
            self.ws.notify("info", &project, &crate::trf!("{title} attend votre réponse", "{title} waits for your answer"), why.label(), None);
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        // Finished sessions: report, inbox, then close the terminal.
        for (i, why, summary) in finished.into_iter().rev() {
            if matches!(self.orch.runs[i].phase, Phase::Closing(_)) {
                self.orch.runs.remove(i);
                continue;
            }
            self.finish_run(i, &why, &summary);
        }
        self.merge_to_main();
        if !self.orch.runs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        self.daily_tick();
    }

    /// Sessions stopped by a quota start again by themselves once it has reset.
    fn resume_tick(&mut self, ctx: &egui::Context) {
        if self.orch.resume_checked.is_some_and(|t| t.elapsed() < Duration::from_secs(20)) {
            return;
        }
        self.orch.resume_checked = Some(Instant::now());
        let now = crate::model::now_ms();
        if !self.ws.settings.orch.auto_resume {
            return;
        }
        let running: Vec<&str> = self.orch.runs.iter().map(|r| r.project.as_str()).collect();
        let due = crate::model::due_resumes(&self.ws.runs, now, &running);
        for i in due {
            let rec = self.ws.runs[i].clone();
            // Once only: a failure doesn't try again every 20 seconds.
            for x in self.ws.runs.iter_mut().filter(|x| x.project == rec.project) {
                x.resume_at = 0;
            }
            self.dirty_since.get_or_insert_with(Instant::now);
            let limit = if rec.limit > 0.0 { rec.limit } else { self.ws.settings.orch.default_limit };
            match self.launch_run(ctx, &rec.project, &rec.mission, limit, rec.commits_allowed, rec.endless.clone()) {
                Ok(()) => self.ws.notify(
                    "info",
                    &rec.project,
                    &crate::trf!("Reprise automatique sur {}", "Automatic restart on {}", rec.title),
                    crate::tr!("Le quota est revenu : la session autonome repart avec la même mission.", "The quota is back: the autonomous session starts again with the same mission."),
                    None,
                ),
                Err(e) => self.ws.notify(
                    "erreur",
                    &rec.project,
                    &crate::trf!("Reprise automatique impossible sur {}", "Couldn't restart {} automatically", rec.title),
                    &e,
                    None,
                ),
            }
        }
    }

    /// Follow Claude Code's figures; share each rise of the 5-hour window between the
    /// running sessions by the tokens each used since the last reading.
    fn usage_tick(&mut self) {
        if self.orch.usage_polled.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            return;
        }
        self.orch.usage_polled = Some(Instant::now());
        // A window that has reset is dropped by read(): forget its old figure too, so a
        // limit reached before the reset doesn't block what follows.
        let Some(u) = crate::usage::read() else {
            (self.orch.five, self.orch.week, self.orch.five_reset, self.orch.week_reset) = (None, None, None, None);
            return;
        };
        if self.orch.history_at != u.at {
            self.orch.history_at = u.at;
            let h = crate::usage::record(&u);
            let now = crate::model::now_ms();
            self.orch.day_total = crate::usage::day_total(&h, now);
            self.orch.day_week = crate::usage::day_week_used(&h, now);
        }
        let five = u.five().cloned();
        self.orch.week = u.week().map(|w| w.pct);
        self.orch.week_reset = u.week().and_then(|w| w.resets_at);
        self.orch.five_reset = five.as_ref().and_then(|w| w.resets_at);
        let Some(five) = five else {
            self.orch.five = None;
            return;
        };
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

    /// Merge into main the finished sessions of projects that allow it, once closed.
    fn merge_to_main(&mut self) {
        let ready: Vec<String> = self.orch.to_main.iter().filter(|id| !self.orch.runs.iter().any(|r| &r.id == *id)).cloned().collect();
        if ready.is_empty() {
            return;
        }
        self.orch.to_main.retain(|id| !ready.contains(id));
        for id in ready {
            let Some(rec) = self.ws.runs.iter().find(|x| x.id == id).cloned() else { continue };
            match auto::merge_into_main(&rec) {
                Ok(m) => {
                    if let Some(x) = self.ws.runs.iter_mut().find(|x| x.id == id) {
                        x.closed = crate::tr!("fusionné", "merged").into();
                    }
                    self.dirty_since.get_or_insert_with(Instant::now);
                    self.info(format!("{} : {m}", rec.title));
                }
                Err(e) => self.fail(format!("{} : {e}", rec.title)),
            }
        }
        self.detect_changes();
    }

    fn finish_run(&mut self, i: usize, why: &str, summary: &str) {
        let r = &mut self.orch.runs[i];
        let why = if why.is_empty() { r.stop.clone().unwrap_or_else(|| crate::tr!("terminée", "finished").into()) } else { why.to_string() };
        r.term.paste("/exit");
        r.term.write(b"\r");
        r.phase = Phase::Closing(Instant::now());
        let (id, project, base, pct, tokens, turns) = (r.id.clone(), r.project.clone(), r.base.clone(), r.pct, r.reader.tokens, r.turns);
        // Stopped by a quota: start again by itself once it resets (if that is allowed).
        let now = crate::model::now_ms();
        let resume_at = r.quota_reset.filter(|&reset| self.ws.settings.orch.auto_resume && reset > now).map(crate::model::resume_time).unwrap_or(0);
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
        rec.resume_at = resume_at;
        let rec = rec.clone();
        let (commits, dirty) = auto::work_done(&rec, &base);
        auto::push_copy_later(rec.worktree.clone());
        let md = auto::session_report(&rec, &commits, dirty, summary);
        let day = auto::local_day(rec.ended);
        let page = self.report_page(&day, &format!("{} · {}", rec.title, auto::hm(rec.started)), &md);
        if let Some(x) = self.ws.runs.iter_mut().find(|x| x.id == id) {
            x.report = Some(page.clone());
        }
        let mut text = crate::trf!("{why} · {} étape(s) · ≈ {:.0} % · {} commit(s)", "{why} · {} step(s) · ≈ {:.0} % · {} commit(s)", rec.steps_done.len(), rec.pct, commits.len());
        if resume_at > 0 {
            text.push_str(&crate::trf!(
                " · reprendra seule dans {} (après la remise à zéro du quota)",
                " · will start again by itself in {} (after the quota resets)",
                crate::usage::left_text(resume_at.saturating_sub(now))
            ));
        }
        // Commits on main allowed for this project: merge once Claude Code has closed.
        let to_main = self.ws.page(&project).and_then(|p| p.project.as_ref()).is_some_and(|p| p.commit_to_main);
        if to_main && (!commits.is_empty() || dirty > 0) {
            self.orch.to_main.push(id.clone());
        }
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
                self.week_bar(ui, &t);
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
                // The settings and limits come before the past sessions, which only grow.
                ui.add_space(14.0);
                self.limits_card(ui, &t);
                ui.add_space(14.0);
                self.history_card(ui, &t, actions);
                ui.add_space(40.0);
            });
        });
    }

    /// The bar at the very top: the week's use of the subscription, with the reserve kept
    /// for the person (set right here): past 100 − reserve, no session starts or goes on.
    fn week_bar(&mut self, ui: &mut egui::Ui, t: &Tone) {
        let week = self.orch.week;
        let before = (self.ws.settings.orch.week_limit, self.ws.settings.orch.reserve_asked);
        let red = Color32::from_rgb(224, 62, 62);
        super::projects::card(ui, t, 0.0, |ui| {
            // Until the person has chosen it: the question, with a few ready answers.
            if !self.ws.settings.orch.reserve_asked {
                ui.label(RichText::new(crate::tr!("Quelle part de l'abonnement garder pour vous ?", "How much of the subscription should be kept for you?")).font(semibold(14.0)).color(t.text));
                ui.label(
                    RichText::new(crate::tr!(
                        "Les IA s'arrêtent avant d'entamer cette réserve de la semaine, et reprennent quand la semaine repart à zéro. Modifiable ensuite à droite de la barre.",
                        "The AIs stop before touching this reserve of the week, and start again once the week resets. You can change it later, right of the bar."
                    ))
                    .size(12.5)
                    .color(t.secondary),
                );
                ui.horizontal(|ui| {
                    for r in [5.0, 10.0, 20.0, 30.0] {
                        if ui.button(format!("{r:.0} %")).clicked() {
                            self.ws.settings.orch.set_reserve(r);
                            self.ws.settings.orch.reserve_asked = true;
                        }
                    }
                    let keep = self.ws.settings.orch.reserve();
                    if ui.button(RichText::new(crate::trf!("Garder {keep:.0} %", "Keep {keep:.0} %")).color(t.secondary)).clicked() {
                        self.ws.settings.orch.reserve_asked = true;
                    }
                });
                ui.add_space(8.0);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(crate::tr!("CETTE SEMAINE", "THIS WEEK")).font(semibold(11.5)).color(t.tertiary));
                let used = match week {
                    Some(w) => crate::trf!("{w:.0} % de l'abonnement utilisés", "{w:.0} % of the subscription used"),
                    None => crate::tr!("utilisation inconnue pour l'instant", "usage unknown for now").into(),
                };
                ui.label(RichText::new(used).font(semibold(14.0)).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut reserve = self.ws.settings.orch.reserve();
                    let r = ui.add(egui::DragValue::new(&mut reserve).range(0.0..=95.0).speed(0.5).suffix(" %")).on_hover_text(crate::tr!(
                        "Part de la semaine gardée pour vous : les IA s'arrêtent avant de l'entamer",
                        "Share of the week kept for you: the AIs stop before touching it"
                    ));
                    if r.changed() {
                        self.ws.settings.orch.set_reserve(reserve);
                        self.ws.settings.orch.reserve_asked = true;
                    }
                    ui.label(RichText::new(crate::tr!("Réserve minimum", "Minimum reserve")).size(13.0).color(t.secondary));
                });
            });
            ui.add_space(4.0);
            let limit = self.ws.settings.orch.week_limit;
            let w = ui.available_width();
            let (bar, _) = ui.allocate_exact_size(egui::vec2(w, 12.0), egui::Sense::hover());
            let p = ui.painter();
            p.rect_filled(bar, 6.0, t.fill);
            let lx = bar.left() + bar.width() * limit / 100.0;
            p.rect_filled(egui::Rect::from_min_max(egui::pos2(lx, bar.top()), bar.max), egui::CornerRadius { nw: 0, sw: 0, ne: 6, se: 6 }, red.gamma_multiply(0.12));
            if let Some(f) = week {
                let frac = super::projects::smooth(ui, "orch-week", (f / 100.0).clamp(0.0, 1.0));
                let color = if f >= limit { red } else if f >= limit * 0.85 { super::projects::ORANGE } else { super::projects::BLUE };
                ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac, bar.height())), 6.0, color);
            }
            ui.painter().vline(lx, egui::Rangef::new(bar.top() - 3.0, bar.bottom() + 3.0), egui::Stroke::new(2.0, super::projects::ORANGE));
            // The day's pace: what went from the week in 24 hours against a seventh of it.
            if let Some((used, span)) = self.orch.day_week {
                let d = crate::usage::daily_pct(used);
                let over = if span >= 23 * 3_600_000 { crate::tr!("sur 24 h", "over 24 h").to_string() } else { crate::trf!("depuis {}", "for the last {}", crate::usage::left_text(span)) };
                let color = if d > 100.0 { super::projects::ORANGE } else { t.secondary };
                ui.add_space(4.0);
                ui.label(RichText::new(crate::trf!("Aujourd'hui : {d:.0} % de la part du jour ({over})", "Today: {d:.0} % of the day's share ({over})")).size(12.5).color(color)).on_hover_text(crate::trf!(
                    "{used:.1} % de la semaine utilisés, pour une part du jour d'un septième de la semaine (≈ 14,3 %). Au-delà de 100 %, le rythme épuiserait la semaine avant sa fin.",
                    "{used:.1} % of the week used, against a day's share of one seventh of the week (≈ 14.3 %). Past 100 %, this pace would use up the week before it ends."
                ));
            }
        });
        if (self.ws.settings.orch.week_limit, self.ws.settings.orch.reserve_asked) != before {
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    /// Three tiles at the top: the subscription, the AIs at work, today's work.
    fn stats_row(&mut self, ui: &mut egui::Ui, t: &Tone) {
        let five = self.orch.five;
        let week = self.orch.week;
        let day = self.orch.day_total;
        let global = self.ws.settings.orch.global_limit;
        let now_ms = crate::model::now_ms();
        let five_left = self.orch.five_reset.and_then(|r| crate::usage::until(r, now_ms));
        let week_left = self.orch.week_reset.and_then(|r| crate::usage::until(r, now_ms));
        let running = self.orch.runs.len();
        let ready = self.ws.projects().iter().filter(|p| p.project.as_ref().is_some_and(|x| !x.folder.trim().is_empty())).count();
        let today = auto::local_day(crate::model::now_ms());
        let todays: Vec<&crate::model::RunRecord> = self.ws.runs.iter().filter(|r| !r.outcome.is_empty() && auto::local_day(r.started) == today).collect();
        let steps: usize = todays.iter().map(|r| r.steps_done.len()).sum();
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
            // Each figure once on the page: the week (and the day's pace) in the bar above, the
            // 5-hour window here (the one the sessions stop at), counts in the other tiles.
            let label = [crate::tr!("FENÊTRE DE 5 H", "5-HOUR WINDOW"), crate::tr!("IA AU TRAVAIL", "AIS AT WORK"), crate::tr!("AUJOURD'HUI", "TODAY")][i];
            p.text(egui::pos2(x, rect.top() + 16.0), egui::Align2::LEFT_TOP, label, semibold(11.5), t.tertiary);
            let (big, sub): (String, String) = match i {
                0 => (
                    five.map(|f| format!("{f:.0} %")).unwrap_or_else(|| "—".into()),
                    match (five, five_left) {
                        (Some(_), Some(l)) => crate::trf!("à zéro dans {} · limite {global:.0} %", "resets in {} · limit {global:.0} %", crate::usage::left_text(l)),
                        (Some(_), None) => crate::trf!("limite des sessions : {global:.0} %", "sessions' limit: {global:.0} %"),
                        _ => crate::tr!("inconnu pour l'instant", "unknown for now").into(),
                    },
                ),
                1 => (running.to_string(), if running == 0 { crate::trf!("{ready} projet(s) prêt(s) à lancer", "{ready} project(s) ready to start") } else { crate::trf!("sur {ready} projet(s) prêt(s)", "of {ready} project(s) ready") }),
                _ => (
                    todays.len().to_string(),
                    if todays.is_empty() { crate::trf!("session terminée · bilan à {hour} h", "finished session · summary at {hour} h") } else { crate::trf!("session(s) · {steps} étape(s) faite(s)", "session(s) · {steps} step(s) done") },
                ),
            };
            p.text(egui::pos2(x, rect.top() + 36.0), egui::Align2::LEFT_TOP, &big, bold(30.0), t.text);
            let mut job = egui::text::LayoutJob::single_section(sub, egui::TextFormat::simple(FontId::proportional(12.5), t.secondary));
            job.wrap = egui::text::TextWrapping::truncate_at_width(tw - 36.0 - if i == 0 && tw >= 240.0 { 70.0 } else { 0.0 });
            let g = ui.fonts_mut(|f| f.layout_job(job));
            ui.painter().galley(egui::pos2(x, rect.bottom() - 28.0), g, t.secondary);
            match i {
                0 => {
                    if tw >= 240.0 {
                        // A ring: the 5-hour use, the global limit as a tick.
                        let c = egui::pos2(rect.right() - 50.0, rect.center().y + 4.0);
                        ring(ui, c, 28.0, five.map(|f| f / 100.0), global / 100.0, accent, t);
                    }
                    // The details, on hover only.
                    let mut tip = vec![crate::trf!(
                        "Part de la fenêtre de 5 h de l'abonnement utilisée. Les sessions s'arrêtent à la limite globale ({global:.0} %), réglable plus bas.",
                        "Share of the subscription's 5-hour window used. Sessions stop at the global limit ({global:.0} %), set below."
                    )];
                    if let Some(d) = day {
                        tip.push(crate::trf!("Dernières 24 h : {d:.0} % (les fenêtres de 5 h de la journée, additionnées)", "Last 24 hours: {d:.0} % (the day's 5-hour windows, added up)"));
                    }
                    if let Some(l) = week_left.filter(|_| week.is_some()) {
                        tip.push(crate::trf!("La semaine repart à zéro dans {}", "The week resets in {}", crate::usage::left_text(l)));
                    }
                    resp.on_hover_text(tip.join("\n"));
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
        let o = &mut self.ws.settings.orch;
        let before = o.clone();
        // Folded by default to one line, so that the last finished sessions, just below,
        // show without scrolling the page; unfolded on demand.
        let open_key = egui::Id::unique("orch-limits-open");
        let mut open: bool = ui.ctx().data(|d| d.get_temp(open_key)).unwrap_or(false);
        super::projects::card(ui, t, 0.0, |ui| {
            let title = crate::tr!("RÉGLAGES ET LIMITES", "SETTINGS AND LIMITS");
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(4.0, 14.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 2.0, super::projects::accent_of(title));
                ui.add_space(2.0);
                ui.label(RichText::new(title).font(semibold(12.5)).color(t.secondary));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if open { crate::tr!("Replier ▴", "Fold ▴") } else { crate::tr!("Modifier ▾", "Edit ▾") };
                    if super::projects::action_link(ui, label).clicked() {
                        open = !open;
                    }
                });
            });
            if !open {
                ui.add_space(4.0);
                ui.label(RichText::new(limits_summary(o)).size(12.5).color(t.tertiary));
                return;
            }
            ui.add_space(8.0);
            // The 5-hour window's use is shown once, in the tile at the top (with this limit
            // as the tick of its ring): here, only the settings.
            let w = ui.available_width();
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
                row(ui, crate::tr!("Limite globale", "Global limit"), crate::tr!("De la fenêtre de 5 h : aucune session ne va au-delà, tous projets confondus", "Of the 5-hour window: no session goes past it, all projects together"), &mut |ui| {
                    ui.add(egui::DragValue::new(&mut o.global_limit).range(5.0..=100.0).speed(0.5).suffix(" %"));
                });
                row(ui, crate::tr!("Commits autorisés", "Commits allowed"), crate::tr!("Sur leur branche, envoyée sur GitHub par Orchestra", "On their branch, pushed to GitHub by Orchestra"), &mut |ui| {
                    super::settings::switch(ui, &mut o.allow_commits, &lk);
                });
                row(ui, crate::tr!("Pause quand l'IA attend", "Pause when the AI waits"), crate::tr!("Autorisation, dossier à approuver ou question posée : la session attend votre réponse au lieu de refuser ou de s'arrêter", "A permission, a folder to approve or a question asked: the session waits for your answer instead of refusing or stopping"), &mut |ui| {
                    super::settings::switch(ui, &mut o.pause_for_answer, &lk);
                });
                row(ui, crate::tr!("Reprise automatique", "Automatic restart"), crate::tr!("Une session arrêtée par la limite globale ou la réserve de la semaine repart seule après la remise à zéro du quota", "A session stopped by the global limit or the week's reserve starts again by itself once the quota resets"), &mut |ui| {
                    super::settings::switch(ui, &mut o.auto_resume, &lk);
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
                    RichText::new(crate::tr!("Déjà permis : lire et modifier les fichiers de la copie, git status / diff / log, compiler et tester (cargo build / test, npm test, pytest, make test, go test…) et git add / commit si les commits sont autorisés. Pas python, node, npm, cargo ou make en entier : ils exécutent n'importe quel code. Toujours refusés : rm, sudo, curl, wget, shells, git push / reset / checkout… Une autre commande est refusée, l'IA continue sans.", "Already allowed: reading and editing the copy's files, git status / diff / log, building and testing (cargo build / test, npm test, pytest, make test, go test…) and git add / commit when commits are allowed. Not python, node, npm, cargo or make as a whole: they run any code. Always refused: rm, sudo, curl, wget, shells, git push / reset / checkout… Any other command is refused and the AI goes on without it."))
                        .size(12.5)
                        .color(t.tertiary),
                );
                ui.add(egui::TextEdit::singleline(&mut o.extra_tools).hint_text(crate::tr!("En plus, ex. Bash(just:*), Bash(docker compose:*)", "More, e.g. Bash(just:*), Bash(docker compose:*)")).desired_width(f32::INFINITY).font(FontId::monospace(12.5)));
            });
        });
        ui.ctx().data_mut(|d| d.insert_temp(open_key, open));
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
        let awake = self.orch.awake.held();
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(ui, crate::tr!("EN COURS", "RUNNING"), &n.to_string(), t);
            if awake {
                ui.label(RichText::new(crate::tr!(
                    "L'ordinateur reste éveillé (sur secteur, même capot rabattu). Fermer la fenêtre ne coupe pas les sessions.",
                    "The computer stays awake (on mains power, lid closed too). Closing the window does not stop the sessions."
                )).size(12.0).color(t.tertiary));
                ui.add_space(6.0);
            }
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
                        Phase::Waiting => crate::tr!("en pause", "paused").to_string(),
                        _ => format!("tour {}", r.turns + 1),
                    };
                    let state_color = if r.waiting.is_some() { super::projects::ORANGE } else { t.tertiary };
                    ui.label(RichText::new(state).size(12.5).color(state_color));
                    if let Some(c) = &r.endless {
                        ui.label(RichText::new(crate::tr!("∞ sans fin", "∞ endless")).size(12.5).color(super::projects::BLUE))
                            .on_hover_text(crate::trf!("S'arrête après : {}, ou sur son budget", "Stops after: {}, or on its budget", c.describe()));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if super::projects::action_link(ui, crate::tr!("Forcer l'arrêt", "Force stop")).on_hover_text(crate::tr!("Ferme Claude Code tout de suite, sans compte rendu", "Closes Claude Code right away, without a summary")).clicked() {
                            stop = Some((i, true));
                        }
                        if r.stop.is_none() && super::projects::action_link(ui, crate::tr!("Arrêter", "Stop")).on_hover_text(crate::tr!("Finit le tour en cours, écrit le compte rendu, puis s'arrête", "Finishes the current turn, writes the summary, then stops")).clicked() {
                            stop = Some((i, false));
                        }
                        if r.waiting.is_some()
                            && super::projects::action_link(ui, crate::tr!("Reprendre", "Resume"))
                                .on_hover_text(crate::tr!("L'IA repart ; une question restée à l'écran est refusée", "The AI goes on; a question still on the screen is refused"))
                                .clicked()
                        {
                            r.resume();
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
                    ui.label(RichText::new(crate::trf!("{:.1} % utilisés sur un budget de {:.0} %", "{:.1} % used of a {:.0} % budget", r.pct, r.limit)).size(12.5).color(t.secondary)).on_hover_text(crate::tr!(
                        "Part de la fenêtre de 5 h utilisée par cette session depuis son lancement, et son budget : elle s'arrête en l'atteignant. Les sessions précédentes du projet ne comptent pas.",
                        "Share of the 5-hour window this session used since it started, and its budget: it stops on reaching it. The project's earlier sessions don't count."
                    ));
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
            self.orch.drafts.entry(id.clone()).or_insert(Draft { mission: String::new(), limit: dl, commits: dc, pick: false, endless: false, end: crate::model::EndConditions { hours: Some(4.0), ..Default::default() } });
        }
        let running: Vec<String> = self.orch.runs.iter().map(|r| r.project.clone()).collect();
        let plugged = self.ais.plugged().contains(&crate::setup::Ai::ClaudeCode);
        let mut start: Vec<String> = vec![];
        let picked: Vec<String> = projects.iter().filter(|(id, _, _)| self.orch.drafts.get(id).is_some_and(|d| d.pick) && !running.contains(id)).map(|(id, _, _)| id.clone()).collect();
        let picked_sum: f32 = picked.iter().filter_map(|id| self.orch.drafts.get(id)).map(|d| d.limit).sum();
        let global = self.ws.settings.orch.global_limit;
        // Shares add up: the sessions running and the projects picked together never
        // get more than the global limit.
        let running_sum: f32 = self.orch.runs.iter().map(|r| r.limit).sum();
        let open_steps: HashMap<String, usize> = projects
            .iter()
            .map(|(id, _, _)| (id.clone(), self.ws.page(id).and_then(|p| p.project.as_ref()).map(|x| x.steps.iter().filter(|s| s.status != crate::project::StepStatus::Done).count()).unwrap_or(0)))
            .collect();
        let mut to_main: HashMap<String, bool> = projects
            .iter()
            .map(|(id, _, _)| (id.clone(), self.ws.page(id).and_then(|p| p.project.as_ref()).is_some_and(|x| x.commit_to_main)))
            .collect();
        let to_main_before = to_main.clone();
        let lk = super::settings::look(ui.visuals().dark_mode);
        let mut confirm_main = self.orch.confirm_main.clone();
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
                let others = running_sum + if d.pick { picked_sum - d.limit } else { picked_sum };
                let room = (global - others).floor().max(0.0);
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
                            } else if !plugged {
                                start_button(ui, false, t).on_disabled_hover_text(crate::tr!("Branchez d'abord Claude Code", "Plug in Claude Code first"));
                            } else if room < 1.0 {
                                start_button(ui, false, t).on_disabled_hover_text(crate::trf!("La limite globale ({global:.0} %) est déjà répartie entre les autres projets", "The global limit ({global:.0} %) is already shared among the other projects"));
                            } else if start_button(ui, true, t).clicked() {
                                // Endless on main: the warning first, the start once confirmed.
                                if endless_on_main(d.endless, to_main.get(id).copied().unwrap_or(false)) {
                                    confirm_main = Some(id.clone());
                                } else {
                                    start.push(id.clone());
                                }
                            }
                            ui.add_space(4.0);
                            super::settings::switch(ui, &mut d.commits, &lk).on_hover_text(crate::tr!("L'IA peut faire des commits sur sa branche (Orchestra l'envoie sur GitHub après chaque tour)", "The AI may commit on its branch (Orchestra pushes it to GitHub after each turn)"));
                            ui.label(RichText::new("commits").size(12.0).color(t.tertiary));
                            ui.add_space(6.0);
                            if let Some(m) = to_main.get_mut(id) {
                                ui.checkbox(m, RichText::new(crate::tr!("sur main", "on main")).size(12.0).color(t.tertiary)).on_hover_text(crate::tr!(
                                    "À la fin de chaque session, son travail est fusionné tout seul dans main (seulement si le dossier du projet est sur main ou master)",
                                    "At the end of each session, its work is merged into main by itself (only when the project's folder is on main or master)"
                                ));
                                ui.add_space(6.0);
                            }
                            // A picked project keeps its share within the room left; the others
                            // only show it capped, so unpicking one doesn't shrink the rest for good.
                            if d.pick && !busy {
                                d.limit = d.limit.clamp(1.0, room.max(1.0));
                            }
                            let mut shown = if busy { d.limit } else { d.limit.clamp(1.0, room.max(1.0)) };
                            let dv = ui.add_enabled(!busy && room >= 1.0, egui::DragValue::new(&mut shown).range(1.0..=room.max(1.0)).speed(0.5).suffix(" %"));
                            if dv.changed() {
                                d.limit = shown;
                            }
                            dv.on_hover_text(crate::trf!(
                                "Budget de la prochaine session de ce projet : elle s'arrête après avoir utilisé cette part de la fenêtre de 5 h de l'abonnement, comptée depuis son lancement. Chaque session a son propre budget : en relancer une repart de zéro.\n{room:.0} % au plus ici, le reste de la limite globale ({global:.0} %) étant pris par les autres.",
                                "Budget of this project's next session: it stops after using this share of the subscription's 5-hour window, counted from its start. Each session has its own budget: starting another one begins again from zero.\n{room:.0} % at most here, the rest of the global limit ({global:.0} %) being taken by the others."
                            ));
                            ui.label(RichText::new(crate::tr!("budget par session", "budget per session")).size(12.0).color(t.tertiary));
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(70.0);
                        ui.add(egui::TextEdit::singleline(&mut d.mission).hint_text(default_mission()).desired_width(f32::INFINITY).font(FontId::proportional(13.0)).margin(egui::Margin::symmetric(8, 5)));
                    });
                    if !busy {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(70.0);
                            ui.spacing_mut().item_spacing.x = 6.0;
                            super::settings::switch(ui, &mut d.endless, &lk).on_hover_text(crate::tr!(
                                "La session ne s'arrête ni au nombre de tours des réglages ni à la fin des étapes : quand il n'y en a plus, l'IA trouve elle-même les suivantes (3 à la fois au plus) et continue. Elle s'arrête seulement sur les conditions choisies ici, ou sur son budget, la limite globale et la réserve de la semaine.",
                                "The session stops neither at the settings' number of turns nor when the steps run out: then the AI finds the next ones itself (3 at a time at most) and goes on. It only stops on the conditions chosen here, or on its budget, the global limit and the week's reserve."
                            ));
                            ui.label(RichText::new(crate::tr!("sans fin", "endless")).size(12.0).color(if d.endless { t.text } else { t.tertiary }));
                            if d.endless {
                                ui.label(RichText::new(crate::tr!("· s'arrête après", "· stops after")).size(12.0).color(t.tertiary));
                                end_conditions(ui, &mut d.end, t);
                            }
                        });
                        let risky = endless_on_main(d.endless, to_main.get(id).copied().unwrap_or(false));
                        if !risky && confirm_main.as_deref() == Some(id.as_str()) {
                            confirm_main = None;
                        }
                        if risky {
                            ui.horizontal(|ui| {
                                ui.add_space(70.0);
                                egui::Frame::new()
                                    .fill(super::projects::ORANGE.gamma_multiply(0.08))
                                    .stroke(egui::Stroke::new(1.0, super::projects::ORANGE.gamma_multiply(0.45)))
                                    .corner_radius(8.0)
                                    .inner_margin(egui::Margin::symmetric(10, 8))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.add(egui::Label::new(RichText::new(format!("⚠ {}", endless_main_warning())).size(12.5).color(super::projects::ORANGE)).wrap());
                                        if confirm_main.as_deref() == Some(id.as_str()) {
                                            ui.add_space(4.0);
                                            ui.horizontal_wrapped(|ui| {
                                                if ui.add(egui::Button::new(RichText::new(crate::tr!("Lancer quand même", "Start anyway")).size(13.0).color(super::projects::ORANGE))).clicked() {
                                                    start.push(id.clone());
                                                    confirm_main = None;
                                                }
                                                if super::projects::quiet_link(ui, crate::tr!("Annuler", "Cancel"), t).clicked() {
                                                    confirm_main = None;
                                                }
                                                ui.label(RichText::new(crate::tr!("ou décochez « sur main » : le travail restera sur sa branche, à fusionner vous-même.", "or untick « on main »: the work stays on its branch, for you to merge.")).size(12.0).color(t.tertiary));
                                            });
                                        }
                                    });
                            });
                        }
                    }
                });
                super::projects::hover_end(ui, bg, row.response.rect, egui::Id::unique(("orch-row", id)), t);
                ui.add_space(10.0);
            }
            if !picked.is_empty() {
                ui.add_space(4.0);
                // Picked projects set to endless on main: confirmed once for the whole selection.
                let risky: Vec<&str> = projects
                    .iter()
                    .filter(|(id, _, _)| picked.contains(id) && drafts.get(id).is_some_and(|d| endless_on_main(d.endless, to_main.get(id).copied().unwrap_or(false))))
                    .map(|(_, title, _)| title.as_str())
                    .collect();
                if risky.is_empty() && confirm_main.as_deref() == Some(SELECTION) {
                    confirm_main = None;
                }
                if confirm_main.as_deref() == Some(SELECTION) {
                    ui.label(RichText::new(crate::trf!("⚠ {} ({})", "⚠ {} ({})", endless_main_warning(), risky.join(", "))).size(12.5).color(super::projects::ORANGE));
                    ui.horizontal_wrapped(|ui| {
                        if ui.add(egui::Button::new(RichText::new(crate::tr!("Lancer quand même", "Start anyway")).size(13.0).color(super::projects::ORANGE))).clicked() {
                            start.extend(picked.iter().cloned());
                            confirm_main = None;
                        }
                        if super::projects::quiet_link(ui, crate::tr!("Annuler", "Cancel"), t).clicked() {
                            confirm_main = None;
                        }
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(plugged, egui::Button::new(RichText::new(crate::trf!("Lancer la sélection ({})", "Start the selection ({})", picked.len())).size(13.5))).clicked() {
                        if risky.is_empty() {
                            start.extend(picked.iter().cloned());
                        } else {
                            confirm_main = Some(SELECTION.to_string());
                        }
                    }
                    let note = crate::trf!("{} projet(s) en parallèle · {picked_sum:.0} % au total pour une limite globale de {global:.0} %", "{} project(s) in parallel · {picked_sum:.0} % in total for a global limit of {global:.0} %", picked.len());
                    ui.label(RichText::new(note).size(12.5).color(if picked_sum > global { super::projects::ORANGE } else { t.tertiary }));
                });
            }
        });
        self.orch.confirm_main = confirm_main;
        if to_main != to_main_before {
            for (id, on) in &to_main {
                if let Some(p) = self.ws.page_mut(id).and_then(|p| p.project.as_mut()) {
                    p.commit_to_main = *on;
                }
            }
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        start.dedup();
        for id in start {
            let d = self.orch.drafts.get(&id).cloned().unwrap();
            let title = self.ws.page(&id).map(|p| p.display_title()).unwrap_or_default();
            let taken: f32 = self.orch.runs.iter().map(|r| r.limit).sum();
            let limit = d.limit.min((global - taken).floor()).max(1.0);
            let endless = if d.endless { Some(d.end.clone()) } else { None };
            match self.launch_run(ctx, &id, &d.mission, limit, d.commits, endless) {
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
        let done: Vec<crate::model::RunRecord> = self.ws.runs.iter().rev().filter(|r| !r.outcome.is_empty()).take(50).cloned().collect();
        if done.is_empty() {
            return;
        }
        let mut act: Option<(String, &'static str)> = None;
        let confirm = self.orch.confirm.clone();
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(ui, crate::tr!("SESSIONS TERMINÉES", "FINISHED SESSIONS"), &done.len().to_string(), t);
            // Five sessions in view, the others by scrolling: the list's height is that of its
            // first five rows (measured on the last frame, the rows wrap with the width).
            let height_key = egui::Id::unique("orch-done-height");
            // Never fewer than three rows in view before the list asks to scroll.
            let three_key = egui::Id::unique("orch-done-height-3");
            let three: f32 = ui.ctx().data(|d| d.get_temp(three_key)).unwrap_or(3.0 * 62.0);
            let five: f32 = ui.ctx().data(|d| d.get_temp::<f32>(height_key)).unwrap_or(5.0 * 62.0).max(three);
            egui::ScrollArea::vertical().id_salt("orch-done-list").max_height(five).auto_shrink([false, true]).show(ui, |ui| {
            let top = ui.min_rect().top();
            for (i, r) in done.iter().enumerate() {
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
                        let used = if r.limit > 0.0 { crate::trf!("≈ {:.0} % / {:.0} % de budget", "≈ {:.0} % / {:.0} % budget", r.pct, r.limit) } else { format!("≈ {:.0} %", r.pct) };
                        ui.label(RichText::new(crate::trf!("{} · {} étape(s) · {used}", "{} · {} step(s) · {used}", r.outcome, r.steps_done.len())).size(12.5).color(t.secondary)).on_hover_text(crate::tr!(
                            "Part de la fenêtre de 5 h utilisée par cette session, sur le budget qu'elle avait",
                            "Share of the 5-hour window this session used, out of the budget it had"
                        ));
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
                let h = row.response.rect.bottom() + 8.0 - top;
                if i == 2 || i + 1 == done.len().min(3) {
                    ui.ctx().data_mut(|d| d.insert_temp(three_key, h));
                }
                if i == 4 || i + 1 == done.len().min(5) {
                    ui.ctx().data_mut(|d| d.insert_temp(height_key, h));
                }
            }
            });
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
        let mut clear_all = false;
        let mut answer: Option<(usize, String)> = None;
        ui.add_space(50.0);
        ui.horizontal(|ui| {
            ui.add_space(((avail - width) / 2.0).max(20.0));
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(crate::tr!("Boîte de réception", "Inbox")).font(bold(30.0)).color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // "Tout supprimer": a second click confirms, nothing goes on one slip.
                        let confirm_id = egui::Id::unique("inbox-clear-confirm");
                        let confirming = ui.data(|d| d.get_temp::<bool>(confirm_id)).unwrap_or(false);
                        if !self.ws.inbox.is_empty() {
                            if confirming {
                                if super::projects::action_link(ui, crate::tr!("Annuler", "Cancel")).clicked() {
                                    ui.data_mut(|d| d.insert_temp(confirm_id, false));
                                }
                                let n = self.ws.inbox.len();
                                let sure = crate::trf!("Confirmer : supprimer les {n} messages", "Confirm: delete the {n} messages");
                                if ui.add(egui::Label::new(RichText::new(sure).size(13.0).color(Color32::from_rgb(224, 62, 62))).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    clear_all = true;
                                    ui.data_mut(|d| d.insert_temp(confirm_id, false));
                                }
                            } else if super::projects::action_link(ui, crate::tr!("Tout supprimer", "Delete all")).clicked() {
                                ui.data_mut(|d| d.insert_temp(confirm_id, true));
                            }
                        }
                        if !confirming && self.ws.unread() > 0 {
                            ui.label(RichText::new("·").color(t.tertiary));
                            if super::projects::action_link(ui, crate::tr!("Tout marquer comme lu", "Mark all as read")).clicked() {
                                all_read = true;
                            }
                        }
                    });
                });
                ui.label(RichText::new(crate::tr!("Les questions des IA, la fin de leurs sessions de travail, les rapports (rangés dans la page « Rapports des IA ») et les messages de l'application, dont les erreurs.", "The AIs' questions, the end of their work sessions, the reports (kept in the « AI reports » page) and the app's messages, errors included.")).color(t.secondary));
                ui.add_space(14.0);
                // Filters: everything, the questions still waiting for an answer, the unread.
                let filter_id = egui::Id::unique("inbox-filter");
                let mut filter = ui.data(|d| d.get_temp::<u8>(filter_id)).unwrap_or(0);
                let waiting = self.ws.inbox.iter().filter(|m| m.kind == "question" && m.answer.trim().is_empty()).count();
                let unread = self.ws.unread();
                ui.horizontal(|ui| {
                    for (k, label, n) in [
                        (0u8, crate::tr!("Tout", "All"), self.ws.inbox.len()),
                        (1, crate::tr!("À répondre", "To answer"), waiting),
                        (2, crate::tr!("Non lus", "Unread"), unread),
                    ] {
                        if filter_chip(ui, label, n, filter == k, &t).clicked() {
                            filter = k;
                        }
                    }
                });
                ui.data_mut(|d| d.insert_temp(filter_id, filter));
                ui.add_space(12.0);
                let shown: Vec<usize> = (0..self.ws.inbox.len())
                    .rev()
                    .filter(|&i| {
                        let m = &self.ws.inbox[i];
                        match filter {
                            1 => m.kind == "question" && m.answer.trim().is_empty(),
                            2 => !m.read,
                            _ => true,
                        }
                    })
                    .collect();
                if shown.is_empty() {
                    let empty = match filter {
                        1 => crate::tr!("Aucune question en attente de réponse.", "No question waiting for an answer."),
                        2 => crate::tr!("Tout est lu.", "Everything is read."),
                        _ => crate::tr!("Rien pour l'instant.", "Nothing yet."),
                    };
                    ui.label(RichText::new(empty).color(t.tertiary).italics());
                }
                for i in shown {
                    let m = &self.ws.inbox[i];
                    let (accent, kind) = match m.kind.as_str() {
                        "question" => (Color32::from_rgb(144, 101, 176), crate::tr!("Question", "Question")),
                        "fin" => (super::projects::GREEN, crate::tr!("Session", "Session")),
                        "rapport" => (super::projects::BLUE, crate::tr!("Rapport", "Report")),
                        "erreur" => (Color32::from_rgb(224, 62, 62), crate::tr!("Erreur", "Error")),
                        _ => (t.tertiary, crate::tr!("Info", "Info")),
                    };
                    let project = (!m.project.is_empty()).then(|| self.ws.page(&m.project).map(|p| p.display_title())).flatten();
                    let bg = super::projects::hover_begin(ui);
                    let card = egui::Frame::new()
                        .fill(if m.read { Color32::TRANSPARENT } else { t.card })
                        .stroke(egui::Stroke::new(1.0, t.border))
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin { left: 16, right: 10, top: 10, bottom: 10 })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let head = ui.horizontal_top(|ui| {
                                let text = ui.vertical(|ui| {
                                    ui.set_width(ui.available_width() - 30.0);
                                    ui.horizontal(|ui| {
                                        kind_badge(ui, kind, accent);
                                        if let Some(p) = &project {
                                            ui.label(RichText::new(p).size(12.0).color(t.tertiary));
                                        }
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(RichText::new(super::chrome::relative_time(m.at)).size(12.0).color(t.tertiary));
                                            if !m.read {
                                                let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                                                ui.painter().circle_filled(dot.center(), 3.5, super::projects::BLUE);
                                            }
                                        });
                                    });
                                    ui.add_space(3.0);
                                    ui.add(egui::Label::new(RichText::new(&m.title).font(if m.read { FontId::proportional(14.5) } else { semibold(14.5) }).color(t.text)).wrap());
                                    if !m.text.is_empty() {
                                        ui.add_space(2.0);
                                        ui.add(egui::Label::new(RichText::new(&m.text).size(13.0).color(t.secondary)).wrap());
                                    }
                                });
                                let x = ui.add(egui::Button::new(RichText::new("×").size(16.0).color(t.tertiary)).frame(false)).on_hover_text(crate::tr!("Supprimer", "Delete"));
                                if x.clicked() {
                                    remove = Some(i);
                                }
                                text.response.rect
                            });
                            // Only the text of the head opens the page: the answer box below stays free to
                            // type in, and the × keeps its click (a zone over it would take it).
                            let resp = ui.interact(head.inner, egui::Id::unique(("inbox", &m.id)), egui::Sense::click());
                            let resp = if m.page.is_some() || !m.project.is_empty() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
                            if resp.clicked() && remove.is_none() {
                                open = Some(i);
                            }
                            if m.kind == "question" {
                                ui.add_space(6.0);
                                if let Some(text) = answer_box(ui, m, &t) {
                                    answer = Some((i, text));
                                }
                            }
                        });
                    let rect = card.response.rect;
                    // A coloured edge on the left: the kind at a glance.
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(rect.left_top() + egui::vec2(0.0, 8.0), egui::pos2(rect.left() + 3.0, rect.bottom() - 8.0)),
                        2.0,
                        accent.gamma_multiply(if m.read { 0.45 } else { 1.0 }),
                    );
                    super::projects::hover_end(ui, bg, rect.shrink2(egui::vec2(6.0, 2.0)), egui::Id::unique(("inbox-h", &m.id)), &t);
                    ui.add_space(8.0);
                }
            });
        });
        if clear_all {
            let n = self.ws.inbox.len();
            for m in self.ws.inbox.drain(..) {
                self.inbox_removed.insert(m.id);
            }
            self.dirty_since.get_or_insert_with(Instant::now);
            self.ack(crate::trf!("{n} messages supprimés.", "{n} messages deleted."));
            return;
        }
        if all_read {
            for m in self.ws.inbox.iter_mut() {
                m.read = true;
            }
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        if let Some((i, text)) = answer {
            let m = &mut self.ws.inbox[i];
            m.answer = text;
            m.answered_at = crate::model::now_ms();
            m.read = true;
            self.dirty_since.get_or_insert_with(Instant::now);
            self.ack(crate::tr!("Réponse enregistrée : l'IA la verra au prochain état des lieux du projet.", "Answer saved: the AI will see it in the project's next state of play."));
        }
        if let Some(i) = remove {
            let m = self.ws.inbox.remove(i);
            self.inbox_removed.insert(m.id);
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


/// The kind of an inbox message, as a small tinted pill.
fn kind_badge(ui: &mut egui::Ui, text: &str, accent: Color32) {
    let g = ui.painter().layout_no_wrap(text.to_string(), semibold(11.0), accent);
    let (rect, _) = ui.allocate_exact_size(g.size() + egui::vec2(14.0, 6.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 6.0, accent.gamma_multiply(0.14));
    ui.painter().galley(rect.center() - g.size() / 2.0, g, accent);
}

/// A filter of the inbox: its name and how many messages it holds.
fn filter_chip(ui: &mut egui::Ui, text: &str, count: usize, on: bool, t: &Tone) -> egui::Response {
    let label = if count > 0 { format!("{text}  {count}") } else { text.to_string() };
    let color = if on { t.text } else { t.secondary };
    let g = ui.painter().layout_no_wrap(label, if on { semibold(12.5) } else { FontId::proportional(12.5) }, color);
    let (rect, resp) = ui.allocate_exact_size(g.size() + egui::vec2(20.0, 10.0), egui::Sense::click());
    let a = ui.ctx().animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.14);
    let fill = if on { t.selected } else { t.hover.gamma_multiply(a) };
    ui.painter().rect_filled(rect, 8.0, fill);
    if on {
        ui.painter().rect_stroke(rect, 8.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    }
    ui.painter().galley(rect.center() - g.size() / 2.0, g, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Under a question of the inbox: the answer given, or a box to type one.
/// Returns the answer when the person sends it (button, or Cmd/Ctrl+Enter).
fn answer_box(ui: &mut egui::Ui, m: &crate::model::InboxMsg, t: &Tone) -> Option<String> {
    let draft_id = egui::Id::unique(("inbox-draft", &m.id));
    let edit_id = egui::Id::unique(("inbox-edit", &m.id));
    let editing = ui.data(|d| d.get_temp::<bool>(edit_id)).unwrap_or(false);
    let mut out = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() - 30.0);
            if !m.answer.is_empty() && !editing {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(crate::tr!("Votre réponse :", "Your answer:")).size(12.0).color(t.tertiary));
                    ui.add(egui::Label::new(RichText::new(&m.answer).size(13.0).color(t.text)).wrap());
                });
                if super::projects::action_link(ui, crate::tr!("Modifier la réponse", "Edit the answer")).clicked() {
                    ui.data_mut(|d| {
                        d.insert_temp(draft_id, m.answer.clone());
                        d.insert_temp(edit_id, true);
                    });
                }
                return;
            }
            let mut draft = ui.data(|d| d.get_temp::<String>(draft_id)).unwrap_or_default();
            let te = ui.add(
                egui::TextEdit::multiline(&mut draft)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text(crate::tr!("Répondre à l'IA…", "Answer the AI…")),
            );
            let send_key = te.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
            let mut send = false;
            ui.horizontal(|ui| {
                let ok = !draft.trim().is_empty();
                send = ui.add_enabled(ok, egui::Button::new(crate::tr!("Répondre", "Answer"))).clicked() || (ok && send_key);
                if editing && super::projects::action_link(ui, crate::tr!("Annuler", "Cancel")).clicked() {
                    ui.data_mut(|d| d.insert_temp(edit_id, false));
                }
            });
            if send {
                out = Some(crate::richtext::clean(draft.trim(), 4000));
                draft.clear();
                ui.data_mut(|d| d.insert_temp(edit_id, false));
            }
            ui.data_mut(|d| d.insert_temp(draft_id, draft));
        });
    });
    out
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
#[cfg(test)]
mod tests {
    #[test]
    fn only_endless_on_main_is_warned() {
        assert!(super::endless_on_main(true, true));
        assert!(!super::endless_on_main(true, false));
        assert!(!super::endless_on_main(false, true));
        assert!(!super::endless_main_warning().is_empty());
    }
}

/// The end conditions of an endless session, each one ticked or not; the first reached stops it.
fn end_conditions(ui: &mut egui::Ui, c: &mut crate::model::EndConditions, t: &Tone) {
    let small = |s: &str| RichText::new(s).size(12.0).color(t.secondary);
    let mut on = c.hours.is_some();
    if ui.checkbox(&mut on, small(crate::tr!("une durée", "a time"))).changed() {
        c.hours = if on { Some(4.0) } else { None };
    }
    if let Some(h) = c.hours.as_mut() {
        ui.add(egui::DragValue::new(h).range(0.5..=72.0).speed(0.25).suffix(" h"));
    }
    let mut on = c.steps.is_some();
    if ui.checkbox(&mut on, small(crate::tr!("des étapes faites", "steps done"))).changed() {
        c.steps = if on { Some(5) } else { None };
    }
    if let Some(n) = c.steps.as_mut() {
        ui.add(egui::DragValue::new(n).range(1..=500));
    }
    let mut on = c.turns.is_some();
    if ui.checkbox(&mut on, small(crate::tr!("des tours", "turns"))).changed() {
        c.turns = if on { Some(60) } else { None };
    }
    if let Some(n) = c.turns.as_mut() {
        ui.add(egui::DragValue::new(n).range(1..=1000));
    }
    ui.checkbox(&mut c.when_done, small(crate::tr!("plus d'étape ouverte", "no open step left"))).on_hover_text(crate::tr!(
        "Coché : la session s'arrête une fois les étapes faites. Décoché : l'IA crée elle-même les étapes suivantes et continue.",
        "Ticked: the session stops once the steps are done. Unticked: the AI creates the next steps itself and goes on."
    ));
    if c.hours.is_none() && c.steps.is_none() && c.turns.is_none() && !c.when_done {
        ui.label(RichText::new(crate::tr!("(budget seulement)", "(budget only)")).size(12.0).color(super::projects::ORANGE)).on_hover_text(crate::tr!(
            "Sans condition, la session tourne jusqu'à son budget, la limite globale ou la réserve de la semaine (ou deux tours de suite sans rien à faire).",
            "With no condition, the session runs until its budget, the global limit or the week's reserve (or two turns in a row with nothing to do)."
        ));
    }
}

/// The settings of the folded "Settings and limits" card, on one line.
fn limits_summary(o: &crate::model::OrchSettings) -> String {
    let commits = if o.allow_commits { crate::tr!("commits autorisés", "commits allowed") } else { crate::tr!("sans commit", "no commits") };
    let resume = if o.auto_resume { crate::tr!("reprise automatique", "automatic restart") } else { crate::tr!("sans reprise", "no restart") };
    crate::trf!(
        "Limite globale {:.0} % · {commits} · {resume} · bilan à {} h · {} tours au plus",
        "Global limit {:.0} % · {commits} · {resume} · summary at {} h · {} turns at most",
        o.global_limit,
        o.report_hour,
        o.max_turns
    )
}

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
