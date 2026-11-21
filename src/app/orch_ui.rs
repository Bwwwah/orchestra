//! "Orchestration": AIs working alone on projects, one or several at a time, each within a
//! share of the Claude subscription and all within a global limit; and the inbox, where
//! their questions, the ends of their sessions and the reports arrive.
//!
//! Each session drives the person's Claude Code in a hidden terminal (see auto.rs for the
//! engine and its reasons). One loop per frame moves every session along (while the window
//! isn't drawn, the conductor runs it: see conductor.rs):
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
            Wait::Trust => crate::tr!(
                "En pause : Claude Code demande si le dossier est sûr. Répondez dans le terminal (« Regarder ») : la session repart ensuite.",
                "Paused: Claude Code asks whether the folder is safe. Answer in the terminal (« Watch »): the session goes on afterwards."
            ),
            Wait::Permission => crate::tr!(
                "En pause : Claude Code demande une autorisation. Répondez dans le terminal (« Regarder ») : la session repart ensuite.",
                "Paused: Claude Code asks for a permission. Answer in the terminal (« Watch »): the session goes on afterwards."
            ),
            Wait::Question => crate::tr!(
                "En pause : l'IA attend la réponse à sa question (boîte de réception) ; elle repart dès qu'elle l'a.",
                "Paused: the AI waits for the answer to its question (inbox); it goes on as soon as it has it."
            ),
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
    /// The project's steps when the session started: (step id, done).
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
    /// Ids of the project's steps when the running turn began: steps the person adds
    /// during a turn are new to the AI, even if it then says there is nothing left.
    steps_at_turn: Vec<String>,
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
        let Some(why) = self.waiting.take() else {
            return;
        };
        let now = Instant::now();
        let still = auto::read_screen(&self.term.text());
        match why {
            Wait::Trust if still == Screen::Trust => {
                // Never accept for them; Claude Code closes and the session ends.
                self.term.write(b"\x1b");
                self.stop = Some(
                    crate::tr!(
                        "le dossier n'a pas été accepté dans Claude Code",
                        "the folder wasn't accepted in Claude Code"
                    )
                    .into(),
                );
                self.phase = Phase::Starting(now);
            }
            Wait::Trust => {
                self.phase = Phase::Starting(now);
                self.last_line =
                    crate::tr!("Démarrage de Claude Code…", "Starting Claude Code…").into();
            }
            Wait::Permission if still == Screen::Permission => {
                self.term.write(b"\x1b");
                self.denials += 1;
                self.last_line = crate::tr!(
                    "Commande refusée (non autorisée en autonome)",
                    "Command refused (not allowed when autonomous)"
                )
                .into();
                self.phase = Phase::Denied(now);
            }
            Wait::Permission => {
                self.phase = Phase::Working(now);
                self.last_line =
                    crate::trf!("Tour {} en cours…", "Turn {} running…", self.turns + 1);
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

impl Draft {
    /// A project's line in the launch card, with the choices the person last made for it
    /// (kept in the project), else the settings' defaults: `limit` and `commits`.
    fn new(pr: Option<&crate::project::Project>, limit: f32, commits: bool) -> Self {
        let limit = pr.and_then(|x| x.session_limit).unwrap_or(limit);
        let prefs = pr
            .and_then(|x| x.launch.clone())
            .unwrap_or_else(|| default_prefs(commits));
        Draft {
            mission: String::new(),
            limit,
            commits: prefs.commits,
            pick: false,
            endless: prefs.endless,
            end: prefs.end,
        }
    }

    /// The choices kept in the project for its next sessions.
    fn prefs(&self) -> crate::project::LaunchPrefs {
        crate::project::LaunchPrefs {
            commits: self.commits,
            endless: self.endless,
            end: self.end.clone(),
        }
    }
}

/// The launch choices of a project the person hasn't set yet: commits as in the settings,
/// not endless, and 4 hours when endless is switched on.
fn default_prefs(commits: bool) -> crate::project::LaunchPrefs {
    crate::project::LaunchPrefs {
        commits,
        endless: false,
        end: crate::model::EndConditions {
            hours: Some(4.0),
            ..Default::default()
        },
    }
}

/// What is left of the global limit for one project of the launch card: the global limit
/// less the sessions running and the other projects picked (`own`: this project's budget
/// when it is picked itself, so it doesn't count against itself). Whole percents.
fn room_for(global: f32, running_sum: f32, picked_sum: f32, own: Option<f32>) -> f32 {
    let others = running_sum + picked_sum - own.unwrap_or(0.0);
    (global - others).floor().max(0.0)
}

/// The budget a session gets when it starts: the one chosen, within what is left of the
/// global limit once `taken` by the sessions running; None when less than 1 % is left.
fn share_at_start(want: f32, global: f32, taken: f32) -> Option<f32> {
    let left = (global - taken).floor();
    (left >= 1.0).then(|| want.min(left).max(1.0))
}

/// What to do with a finished session's copy.
#[derive(Clone, Copy, PartialEq)]
enum CopyOp {
    /// Into the branch open in the project's folder (asked by the person).
    Merge,
    /// By itself, only when the project's folder is on main.
    ToMain,
    /// A session on the shared `dev` branch: its work stays there, its copy goes.
    Park,
    Discard,
}

/// A merge or a discard of a finished session's copy, running on its own thread.
struct CopyJob {
    id: String,
    merge: bool,
    /// Kept on `dev` rather than merged.
    park: bool,
    title: String,
    /// The project's repository.
    root: String,
    done: std::sync::mpsc::Receiver<Result<String, String>>,
}

/// The figures of one agent's subscription, as last read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Figures {
    pub five: Option<f32>,
    pub week: Option<f32>,
    pub five_reset: Option<u64>,
    pub week_reset: Option<u64>,
}

#[derive(Default)]
pub struct Orch {
    pub runs: Vec<LiveRun>,
    drafts: HashMap<String, Draft>,
    watching: Option<String>,
    /// The figures of the agents other than Claude Code (whose own are the fields below,
    /// shown on the cards), and their last reading: each session is measured and stopped on
    /// the subscription of its own agent, never on another's.
    others: HashMap<Agent, Figures>,
    others_reading: HashMap<Agent, (u64, f32, Option<u64>)>,
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
    /// A finished session's copy waiting for its second click: (run id, "merge" | "discard").
    confirm: Option<(String, &'static str)>,
    /// Copies being merged or discarded in the background (git can take a while).
    copy_jobs: Vec<CopyJob>,
    /// The window was closed during one of them: close it once they are all done.
    quit_after_jobs: bool,
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
    /// The « Screen Recording » permission at the last reading, and when it was read.
    screen: Option<(bool, Instant)>,
    /// The person put the permission card away for this launch.
    screen_dismissed: bool,
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
    crate::tr!(
        "Avance sur les étapes ouvertes du projet, dans l'ordre, en vérifiant ton travail.",
        "Move forward on the project's open steps, in order, checking your work."
    )
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

impl NotesApp {
    /// Start an autonomous session on a project.
    fn launch_run(
        &mut self,
        ctx: &egui::Context,
        project: &str,
        mission: &str,
        limit: f32,
        commits: bool,
        endless: Option<crate::model::EndConditions>,
    ) -> Result<(), String> {
        if self.orch.runs.iter().any(|r| r.project == project) {
            return Err(crate::tr!(
                "une session tourne déjà sur ce projet",
                "a session is already running on this project"
            )
            .into());
        }
        let page = self
            .ws
            .page(project)
            .ok_or(crate::tr!("projet introuvable", "project not found"))?;
        let pr = page.project.as_ref().ok_or(crate::tr!(
            "ce n'est pas un projet",
            "this is not a project"
        ))?;
        let title = page.display_title();
        let folder = pr.folder.trim().to_string();
        if folder.is_empty() {
            return Err(crate::tr!(
                "indiquez d'abord le dossier du projet",
                "set the project's folder first"
            )
            .into());
        }
        if !self.ais.plugged().contains(&crate::setup::Ai::ClaudeCode) {
            return Err(crate::tr!(
                "Claude Code n'est pas branché (Réglages › IA)",
                "Claude Code isn't plugged in (Settings › AI)"
            )
            .into());
        }
        if let Some(f) = self.orch.five
            && f >= self.ws.settings.orch.global_limit
        {
            let left = self
                .orch
                .five_reset
                .and_then(|r| crate::usage::until(r, crate::model::now_ms()));
            let when = left
                .map(|l| {
                    crate::trf!(
                        " ; la fenêtre de 5 h repart à zéro dans {}",
                        "; the 5-hour window resets in {}",
                        crate::usage::left_text(l)
                    )
                })
                .unwrap_or_default();
            return Err(crate::trf!(
                "la limite globale ({:.0} %) est déjà atteinte{when}",
                "the global limit ({:.0} %) is already reached{when}",
                self.ws.settings.orch.global_limit
            ));
        }
        let taken: f32 = self.orch.runs.iter().map(|r| r.limit).sum();
        if taken + limit > self.ws.settings.orch.global_limit + 0.01 {
            return Err(crate::trf!(
                "{limit:.0} % pour ce projet dépasserait la limite globale ({:.0} %), dont {taken:.0} % sont déjà pris",
                "{limit:.0} % for this project would go past the global limit ({:.0} %), {taken:.0} % of which is already taken",
                self.ws.settings.orch.global_limit
            ));
        }
        if let Some(w) = self.orch.week
            && crate::usage::near_reserve(w, self.ws.settings.orch.week_limit, 0.0)
        {
            return Err(crate::trf!(
                "la réserve minimum de la semaine ({:.0} %) est atteinte ou trop proche",
                "the week's minimum reserve ({:.0} %) is reached or too near",
                self.ws.settings.orch.reserve()
            ));
        }
        // The limits are checked with Claude Code's own figures: they come through the status line.
        if !crate::usage::tracking_on() {
            crate::usage::start_tracking(&std::env::current_exe().unwrap_or_default())?;
            self.ais.tracking = true;
        }
        // The session checks interfaces with take_screenshot, which macOS only allows once
        // Orchestra has the « Screen Recording » permission: ask for it now (once per launch).
        let mut granted_version = self.ws.settings.screen_granted_version.clone();
        crate::screen::ensure_permission(&mut granted_version);
        let steps_at_start: Vec<(String, bool)> = pr
            .steps
            .iter()
            .map(|s| (s.id.clone(), s.status == crate::project::StepStatus::Done))
            .collect();
        let steps_at_turn: Vec<String> = steps_at_start.iter().map(|(id, _)| id.clone()).collect();
        let agent = Agent::ClaudeCode;
        if !agent.ready() {
            return Err(crate::trf!(
                "le travail autonome n'est pas encore possible avec {}",
                "autonomous work isn't possible yet with {}",
                agent.label()
            ));
        }
        let code = crate::project::code_kind(&folder, pr.code);
        let copy = auto::make_copy(&folder, &title, !on_main(pr), code)?;
        // A project without code makes no commit (the switch isn't shown for it), and without
        // git (worked on in its own folder) there is none to make.
        let commits = commits && code && !copy.branch.is_empty();
        let o = &self.ws.settings.orch;
        let cwd = std::path::Path::new(&copy.worktree);
        agent.write_config(cwd, commits, &o.extra_tools)?;
        let cmd = agent.command(cwd, commits, &o.extra_tools);
        let c2 = ctx.clone();
        let mut term = Term::spawn_once(
            &crate::trf!("Autonome · {title}", "Autonomous · {title}"),
            &copy.worktree,
            &cmd,
            move || c2.request_repaint_after(Duration::from_millis(200)),
        )?;
        term.resize(42, 140);
        let id = crate::model::new_id();
        let mission = if mission.trim().is_empty() {
            default_mission().to_string()
        } else {
            mission.trim().to_string()
        };
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
        self.ws.settings.screen_granted_version = granted_version;
        self.dirty_since.get_or_insert_with(Instant::now);
        self.orch.runs.push(LiveRun {
            id,
            project: project.to_string(),
            title,
            agent,
            term,
            phase: Phase::Starting(Instant::now()),
            next: Some(
                auto::mission_prompt(
                    &self
                        .ws
                        .page(project)
                        .map(|p| p.display_title())
                        .unwrap_or_default(),
                    project,
                    &copy.branch,
                    &mission,
                    commits,
                ) + &endless
                    .as_ref()
                    .map(|c| auto::endless_rule(project, !c.when_done))
                    .unwrap_or_default(),
            ),
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
            steps_at_turn,
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
        let asked = ctx.input(|i| i.viewport().close_requested());
        // A copy being merged or discarded: quitting would cut git short and could leave the
        // project's folder half merged. Orchestra closes once it is done; asking again quits.
        if asked && !self.orch.copy_jobs.is_empty() && !self.orch.quit_after_jobs {
            self.orch.quit_after_jobs = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.info(crate::tr!(
                "Une copie est en cours de fusion : Orchestra se fermera dès qu'elle sera finie. Fermez à nouveau pour quitter tout de suite.",
                "A copy is being merged: Orchestra will close as soon as it is done. Close again to quit right away."
            ));
            return;
        }
        if self.orch.quit_after_jobs && self.orch.copy_jobs.is_empty() {
            self.orch.quit_after_jobs = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !asked || self.orch.runs.is_empty() {
            return;
        }
        if self
            .orch
            .close_held
            .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
        {
            return;
        }
        self.orch.close_held = Some(Instant::now());
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        let n = self.orch.runs.len();
        let s = crate::i18n::plural(n);
        self.info(crate::trf!(
            "{n} session{s} autonome{s} en cours : Orchestra continue dans le Dock et garde le Mac éveillé. Fermez à nouveau pour quitter et les arrêter.",
            "{n} autonomous session{s} running: Orchestra keeps going in the Dock and keeps the Mac awake. Close again to quit and stop them."
        ));
    }

    /// `window`: called from a frame of the window; else from the conductor, while the
    /// window isn't drawn (its input is then from the last frame: closing isn't read).
    pub(super) fn orch_tick(&mut self, ctx: &egui::Context, window: bool) {
        if !self.orch.started {
            self.orch.started = true;
            // Sessions that were running when Orchestra closed.
            for r in self.ws.runs.iter_mut().filter(|r| r.outcome.is_empty()) {
                r.outcome = crate::tr!(
                    "interrompue (Orchestra fermé)",
                    "interrupted (Orchestra closed)"
                )
                .into();
                r.ended = r.ended.max(r.started);
            }
            // The history only grows: the oldest sessions nothing needs any more go.
            if auto::trim_runs(&mut self.ws.runs, auto::KEEP_RUNS, |w| {
                std::path::Path::new(w).exists()
            }) > 0
            {
                self.dirty_since.get_or_insert_with(Instant::now);
            }
            // Copies no session needs any more (merged, discarded, or forgotten): removed
            // if they hold nothing uncommitted, their branches kept.
            auto::clean_copies_later(auto::stale_copies(&self.ws.runs));
        }
        self.usage_tick();
        self.copy_jobs_tick();
        self.resume_tick(ctx);
        self.orch.awake.set(!self.orch.runs.is_empty());
        if window {
            self.keep_running_on_close(ctx);
        }
        // Developer aid: ORCHESTRA_AUTO_RUN=<project id> starts a session once Claude Code is
        // seen, once per launch of Orchestra.
        static AUTO_RUN_DONE: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if let Ok(pid) = std::env::var("ORCHESTRA_AUTO_RUN")
            && !pid.is_empty()
            && self.orch.runs.is_empty()
            && self.ais.plugged().contains(&crate::setup::Ai::ClaudeCode)
            && !AUTO_RUN_DONE.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let (l, c) = (self.ws.settings.orch.default_limit, true);
            if let Err(e) = self.launch_run(ctx, &pid, "", l, c, None) {
                eprintln!("auto-run : {e}");
            }
        }
        let now = Instant::now();
        let global = self.ws.settings.orch.global_limit;
        let week_limit = self.ws.settings.orch.week_limit;
        let pause = self.ws.settings.orch.pause_for_answer;
        let mut paused: Vec<(String, String, Wait)> = vec![];
        let mut finished: Vec<(usize, String, String)> = vec![];
        let figures = self.figures_by_agent();
        for (i, r) in self.orch.runs.iter_mut().enumerate() {
            // The subscription of this session's own agent; none read (an agent that tells
            // nothing): no figures, and the session stops as unmeasured.
            let fig = figures.get(&r.agent).copied().unwrap_or_default();
            // How the screen moves (idle = nothing drawn for a while).
            let screen = r.term.text();
            let h = hash(&screen);
            if h != r.screen_hash {
                r.screen_hash = h;
                r.screen_changed = now;
            }
            let idle = r.screen_changed.elapsed();
            if r.term.exited() && !matches!(r.phase, Phase::Closing(_)) {
                finished.push((
                    i,
                    r.stop.clone().unwrap_or_else(|| {
                        crate::tr!("Claude Code s'est arrêté", "Claude Code stopped").into()
                    }),
                    String::new(),
                ));
                continue;
            }
            r.reader
                .poll(r.agent, std::path::Path::new(&r.worktree), r.since);
            match r.phase {
                Phase::Starting(t0) => {
                    if t0.elapsed() > Duration::from_secs(90) {
                        finished.push((
                            i,
                            crate::tr!("Claude Code n'a pas démarré", "Claude Code didn't start")
                                .into(),
                            String::new(),
                        ));
                    } else if t0.elapsed() > Duration::from_secs(3)
                        && idle > Duration::from_millis(2500)
                    {
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
                        r.week_at_turn = fig.week;
                        r.phase = if r.summarizing {
                            Phase::Summary(now)
                        } else {
                            Phase::Working(now)
                        };
                        r.last_line = if r.summarizing {
                            crate::tr!("Rédige son compte rendu…", "Writing its summary…").into()
                        } else {
                            crate::trf!("Tour {} en cours…", "Turn {} running…", r.turns + 1)
                        };
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
                    // Claude Code's own limit is reached: no turn goes through until it resets.
                    // Said in the session file, or only on the screen (still for a while, so
                    // the AI isn't at work, and the message in its last lines).
                    let on_screen = (idle > Duration::from_secs(30) && !r.reader.turn_over)
                        .then(|| last_lines(&screen, 12))
                        .filter(|tail| auto::is_limit_message(tail));
                    if let Some(msg) = r.reader.limit_hit.clone().or(on_screen) {
                        let now_ms = crate::model::now_ms();
                        let reset = auto::limit_reset(&msg, now_ms)
                            .or(if fig.week.is_some_and(|w| w >= 99.0) {
                                fig.week_reset
                            } else {
                                fig.five_reset
                            })
                            .filter(|&t| t > now_ms)
                            // Unknown: try again in an hour; still limited, it stops again.
                            .unwrap_or(now_ms + 3_600_000);
                        r.quota_reset = Some(reset);
                        let why = Stop::SubscriptionLimit.text();
                        r.stop = Some(why.clone());
                        // No summary: Claude Code would refuse it too.
                        finished.push((i, why, String::new()));
                        continue;
                    }
                    let frozen = idle > Duration::from_secs(180)
                        && !r.reader.turn_over
                        && r.reader
                            .last_growth
                            .is_none_or(|g| g.elapsed() > Duration::from_secs(180));
                    if (auto::read_screen(&screen) == Screen::Permission
                        && idle > Duration::from_millis(800))
                        || frozen
                    {
                        if pause {
                            r.pause(Wait::Permission);
                            paused.push((r.project.clone(), r.title.clone(), Wait::Permission));
                            continue;
                        }
                        r.term.write(b"\x1b");
                        r.denials += 1;
                        r.last_line = crate::tr!(
                            "Commande refusée (non autorisée en autonome)",
                            "Command refused (not allowed when autonomous)"
                        )
                        .into();
                        r.phase = Phase::Denied(now);
                        continue;
                    }
                    // The prompt didn't go: press Enter once more.
                    if !r.nudged
                        && t.elapsed() > Duration::from_secs(60)
                        && r.reader
                            .last_growth
                            .is_none_or(|g| g.elapsed() > t.elapsed())
                    {
                        r.term.write(b"\r");
                        r.nudged = true;
                    }
                    let quiet = r
                        .reader
                        .last_growth
                        .is_some_and(|g| g.elapsed() > Duration::from_secs(2));
                    let over = r.reader.turn_over && quiet && idle > Duration::from_millis(1500);
                    let too_long =
                        t.elapsed() > Duration::from_secs(if summary { 300 } else { 25 * 60 });
                    if over || too_long {
                        if too_long {
                            r.term.write(b"\x1b");
                        }
                        if summary {
                            let text = if over {
                                r.reader.last_text.clone()
                            } else {
                                String::new()
                            };
                            let why = r
                                .stop
                                .clone()
                                .unwrap_or_else(|| crate::tr!("terminée", "finished").into());
                            finished.push((i, why, text));
                            continue;
                        }
                        r.turns += 1;
                        // What this turn took from the week (a reset in between counts for nothing).
                        if let (Some(a), Some(b)) = (r.week_at_turn, fig.week)
                            && b >= a
                        {
                            r.week_turn = r.week_turn.max(b - a);
                        }
                        auto::push_copy_later(r.worktree.clone());
                        let mut sig = auto::signal(&r.reader.last_text);
                        r.last_line = crate::richtext::one_line(
                            &auto::strip_markers(&r.reader.last_text),
                            140,
                        );
                        // Steps the person added during the turn: the AI hasn't seen them, so
                        // its « nothing left » doesn't stop the session; it is sent on to them.
                        let steps_now = self
                            .ws
                            .page(&r.project)
                            .and_then(|p| p.project.as_ref())
                            .map(|pr| pr.steps.clone())
                            .unwrap_or_default();
                        let fresh = fresh_steps(&r.steps_at_turn, &steps_now);
                        if sig == Signal::Done && !fresh.is_empty() {
                            sig = Signal::Continue;
                        }
                        r.steps_at_turn = steps_now.iter().map(|s| s.id.clone()).collect();
                        let five = fig.five;
                        r.done_streak = if sig == Signal::Done {
                            r.done_streak + 1
                        } else {
                            0
                        };
                        // Endless mode: its own end conditions replace [[FINI]] and the turn limit.
                        let endless_end = r.endless.as_ref().and_then(|c| {
                            let steps = self
                                .ws
                                .page(&r.project)
                                .and_then(|p| p.project.as_ref())
                                .map(|pr| pr.steps.clone())
                                .unwrap_or_default();
                            let p = steps_since(&r.steps_at_start, &steps);
                            let steps_done = p.done.len() as u32;
                            // Steps the AI added itself and hasn't done yet.
                            let added_open = p.added_open;
                            if auto::too_many_self_steps(added_open) {
                                return Some(crate::trf!(
                                    "l'IA a ajouté {added_open} étapes sans les faire",
                                    "the AI added {added_open} steps without doing them"
                                ));
                            }
                            c.reached(
                                crate::model::now_ms().saturating_sub(r.started_wall),
                                steps_done,
                                r.turns,
                                r.done_streak,
                            )
                        });
                        let stop = stop_reason(&TurnEnd {
                            asked: r.stop.clone(),
                            too_long,
                            endless_end,
                            signal: sig,
                            endless: r.endless.is_some(),
                            pct: r.pct,
                            limit: r.limit,
                            five,
                            global,
                            week: fig.week,
                            week_limit,
                            week_turn: r.week_turn,
                            measured: r.measured,
                            turns: r.turns,
                            max_turns: r.max_turns,
                        });
                        // Stopped by a quota: when it resets, to start again then.
                        match stop.as_ref().map(Stop::quota) {
                            Some(Quota::FiveHours) => r.quota_reset = fig.five_reset,
                            Some(Quota::Week) => r.quota_reset = fig.week_reset,
                            _ => {}
                        }
                        let reason = stop.map(|s| s.text());
                        // A question to the person still unanswered: wait for it rather
                        // than send the AI on.
                        let asked = pause
                            && reason.is_none()
                            && crate::model::question_pending(
                                &self.ws.inbox,
                                &r.project,
                                r.started_wall,
                            );
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
                            None => {
                                let next = if r.endless.is_some() {
                                    auto::endless_prompt(
                                        r.endless.as_ref().is_some_and(|c| !c.when_done),
                                    )
                                } else {
                                    auto::continue_prompt()
                                };
                                r.term.paste(&format!("{}{next}", new_steps_note(&fresh)));
                            }
                        }
                        r.phase = Phase::Typed(now);
                    }
                }
                // Paused: nothing moves until the person answers, then the AI goes on.
                Phase::Waiting => {
                    let left =
                        crate::model::question_pending(&self.ws.inbox, &r.project, r.started_wall);
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
            self.ws.notify(
                "info",
                &project,
                &crate::trf!(
                    "{title} attend votre réponse",
                    "{title} waits for your answer"
                ),
                why.label(),
                None,
            );
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
        self.merge_to_main(ctx);
        auto::set_busy(self.orch.runs.iter().map(|r| r.worktree.clone()).collect());
        if !self.orch.runs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        self.daily_tick();
    }

    /// Sessions stopped by a quota start again by themselves once it has reset.
    fn resume_tick(&mut self, ctx: &egui::Context) {
        if self
            .orch
            .resume_checked
            .is_some_and(|t| t.elapsed() < Duration::from_secs(20))
        {
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
            let limit = if rec.limit > 0.0 {
                rec.limit
            } else {
                self.ws.settings.orch.default_limit
            };
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
        if self
            .orch
            .usage_polled
            .is_some_and(|t| t.elapsed() < Duration::from_secs(3))
        {
            return;
        }
        self.orch.usage_polled = Some(Instant::now());
        self.other_agents_tick();
        // A window that has reset is dropped by read(): forget its old figure too, so a
        // limit reached before the reset doesn't block what follows.
        let Some(u) = crate::usage::read() else {
            (
                self.orch.five,
                self.orch.week,
                self.orch.five_reset,
                self.orch.week_reset,
            ) = (None, None, None, None);
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
        apply_reading(
            &mut self.orch.runs,
            Agent::ClaudeCode,
            &mut self.orch.last_reading,
            u.at,
            &five,
        );
    }

    /// The same for the other agents that have sessions running and tell their figures
    /// (Codex); those that tell none (Gemini, Vibe) stay unmeasured, so their sessions stop
    /// (`Stop::Unmeasured`) rather than run with no ceiling.
    fn other_agents_tick(&mut self) {
        let mut agents: Vec<Agent> = vec![];
        for a in self.orch.runs.iter().map(|r| r.agent) {
            if a != Agent::ClaudeCode && crate::usage::reports_usage(a) && !agents.contains(&a) {
                agents.push(a);
            }
        }
        self.orch.others.retain(|a, _| agents.contains(a));
        for a in agents {
            let Some(u) = crate::usage::read_for(a) else {
                // Reset or never seen: forget the old figures (as for Claude Code).
                self.orch.others.remove(&a);
                continue;
            };
            self.orch.others.insert(a, Figures::of(&u));
            if let Some(five) = u.five() {
                let mut last = self.orch.others_reading.get(&a).copied();
                apply_reading(&mut self.orch.runs, a, &mut last, u.at, five);
                if let Some(l) = last {
                    self.orch.others_reading.insert(a, l);
                }
            }
        }
    }

    /// The figures each session is measured and stopped on: those of its own agent.
    fn figures_by_agent(&self) -> HashMap<Agent, Figures> {
        let mut m = self.orch.others.clone();
        m.insert(
            Agent::ClaudeCode,
            Figures {
                five: self.orch.five,
                week: self.orch.week,
                five_reset: self.orch.five_reset,
                week_reset: self.orch.week_reset,
            },
        );
        m
    }

    /// Merge into main the finished sessions of projects that allow it, once closed, in
    /// the background; one waits while another copy of its project is being dealt with.
    fn merge_to_main(&mut self, ctx: &egui::Context) {
        let ready: Vec<String> = self
            .orch
            .to_main
            .iter()
            .filter(|id| !self.orch.runs.iter().any(|r| &r.id == *id))
            .cloned()
            .collect();
        for id in ready {
            let Some(rec) = self.ws.runs.iter().find(|x| x.id == id).cloned() else {
                self.orch.to_main.retain(|x| *x != id);
                continue;
            };
            let op = if rec.branch == auto::DEV {
                CopyOp::Park
            } else {
                CopyOp::ToMain
            };
            if self.start_copy_job(ctx, rec, op) {
                self.orch.to_main.retain(|x| *x != id);
            }
        }
    }

    /// Start merging or discarding a session's copy on its own thread; false when another
    /// copy of the same repository is being dealt with (git would find it locked).
    fn start_copy_job(
        &mut self,
        ctx: &egui::Context,
        rec: crate::model::RunRecord,
        op: CopyOp,
    ) -> bool {
        if self
            .orch
            .copy_jobs
            .iter()
            .any(|j| j.id == rec.id || j.root == rec.root)
        {
            return false;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.orch.copy_jobs.push(CopyJob {
            id: rec.id.clone(),
            merge: op != CopyOp::Discard,
            park: op == CopyOp::Park,
            title: rec.title.clone(),
            root: rec.root.clone(),
            done: rx,
        });
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let res = match op {
                CopyOp::Merge => auto::merge(&rec),
                CopyOp::ToMain => auto::merge_into_main(&rec),
                CopyOp::Park => auto::park(&rec),
                CopyOp::Discard => auto::discard(&rec),
            };
            let _ = tx.send(res);
            ctx.request_repaint();
        });
        true
    }

    fn finish_run(&mut self, i: usize, why: &str, summary: &str) {
        let r = &mut self.orch.runs[i];
        let why = if why.is_empty() {
            r.stop
                .clone()
                .unwrap_or_else(|| crate::tr!("terminée", "finished").into())
        } else {
            why.to_string()
        };
        r.term.paste("/exit");
        r.term.write(b"\r");
        r.phase = Phase::Closing(Instant::now());
        let (id, project, base, pct, tokens, turns) = (
            r.id.clone(),
            r.project.clone(),
            r.base.clone(),
            r.pct,
            r.reader.tokens,
            r.turns,
        );
        // Stopped by a quota: start again by itself once it resets (if that is allowed).
        let now = crate::model::now_ms();
        let resume_at = r
            .quota_reset
            .filter(|&reset| self.ws.settings.orch.auto_resume && reset > now)
            .map(crate::model::resume_time)
            .unwrap_or(0);
        let before = r.steps_at_start.clone();
        let newly: Vec<String> = self
            .ws
            .page(&project)
            .and_then(|p| p.project.as_ref())
            .map(|pr| steps_since(&before, &pr.steps).done)
            .unwrap_or_default();
        let Some(rec) = self.ws.runs.iter_mut().find(|x| x.id == id) else {
            return;
        };
        rec.ended = crate::model::now_ms();
        rec.outcome = why.clone();
        rec.pct = pct;
        rec.tokens = tokens;
        rec.turns = turns;
        rec.steps_done = newly;
        rec.resume_at = resume_at;
        // No copy (a project without git): nothing to merge nor discard, the work is there.
        if auto::in_place(rec) {
            rec.closed = auto::in_place_note().into();
        }
        let rec = rec.clone();
        let (commits, dirty) = auto::work_done(&rec, &base);
        auto::push_copy_later(rec.worktree.clone());
        let md = auto::session_report(&rec, &commits, dirty, summary);
        let day = auto::local_day(rec.ended);
        let page = self.report_page(
            &day,
            &format!("{} · {}", rec.title, auto::hm(rec.started)),
            &md,
        );
        if let Some(x) = self.ws.runs.iter_mut().find(|x| x.id == id) {
            x.report = Some(page.clone());
        }
        let (e, c) = (
            crate::i18n::plural(rec.steps_done.len()),
            crate::i18n::plural(commits.len()),
        );
        let mut text = crate::trf!(
            "{why} · {} étape{e} · ≈ {:.0} % · {} commit{c}",
            "{why} · {} step{e} · ≈ {:.0} % · {} commit{c}",
            rec.steps_done.len(),
            rec.pct,
            commits.len()
        );
        if resume_at > 0 {
            text.push_str(&crate::trf!(
                " · reprendra seule dans {} (après la remise à zéro du quota)",
                " · will start again by itself in {} (after the quota resets)",
                crate::usage::left_text(resume_at.saturating_sub(now))
            ));
        }
        // Commits on main allowed for this project: merge once Claude Code has closed.
        let to_main = self
            .ws
            .page(&project)
            .and_then(|p| p.project.as_ref())
            .is_some_and(on_main);
        // On the shared dev branch: the work stays there and the copy goes, so dev can be
        // opened from the Versions card (unless the session starts again by itself).
        let on_dev = rec.branch == auto::DEV && resume_at == 0;
        if on_dev || (to_main && (!commits.is_empty() || dirty > 0)) {
            self.orch.to_main.push(id.clone());
        }
        self.ws.notify(
            "fin",
            &project,
            &crate::trf!("Fin de session sur {}", "End of session on {}", rec.title),
            &text,
            Some(page),
        );
        self.detect_changes();
        self.ack(crate::trf!(
            "Session autonome terminée sur {} : {why}.",
            "Autonomous session finished on {}: {why}.",
            rec.title
        ));
    }

    /// A page under "Rapports des IA › <day>".
    fn report_page(&mut self, day: &str, title: &str, md: &str) -> String {
        let root = {
            let id = self.ws.settings.orch.reports_page.clone();
            if self.ws.page(&id).is_some_and(|p| !p.trashed) {
                id
            } else {
                let id = self
                    .ws
                    .create_page(crate::tr!("Rapports des IA", "AI reports"), None);
                if let Some(p) = self.ws.page_mut(&id) {
                    p.icon = "📒".into();
                }
                self.ws.settings.orch.reports_page = id.clone();
                id
            }
        };
        let day_page = match self
            .ws
            .children(Some(&root))
            .iter()
            .find(|p| p.title == day && !p.trashed)
        {
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
        if self
            .orch
            .daily_checked
            .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
            || !self.orch.runs.is_empty()
        {
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
            let runs: Vec<crate::model::RunRecord> = self
                .ws
                .runs
                .iter()
                .filter(|r| r.ended > 0 && auto::local_day(r.ended) == day)
                .cloned()
                .collect();
            let mut projects: Vec<&str> = runs.iter().map(|r| r.project.as_str()).collect();
            projects.sort();
            projects.dedup();
            if projects.len() < 2 {
                continue;
            }
            let refs: Vec<&crate::model::RunRecord> = runs.iter().collect();
            let md = auto::daily_report(&day, &refs);
            let page = self.report_page(
                &day,
                crate::tr!("Bilan de la journée", "Daily summary"),
                &md,
            );
            let text = crate::trf!(
                "{} projets, {} sessions",
                "{} projects, {} sessions",
                projects.len(),
                runs.len()
            );
            self.ws.notify(
                "rapport",
                "",
                &crate::trf!("Bilan de la journée · {day}", "Daily summary · {day}"),
                &text,
                Some(page),
            );
            self.ws.settings.orch.last_daily = day;
            self.detect_changes();
        }
    }

    // ------------------------------------------------------------------ the page

    /// Stop the autonomous sessions' programs (and everything they started), before quitting
    /// or replacing the app.
    pub(super) fn stop_runs(&mut self) {
        for r in self.orch.runs.iter_mut() {
            r.term.stop();
        }
        // A copy being merged or discarded: let git finish (cut short, it could leave the
        // project's folder half merged), within a minute, and record how it went.
        let until = Instant::now() + Duration::from_secs(60);
        while !self.orch.copy_jobs.is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
            self.copy_jobs_tick();
        }
    }

    /// At launch: read the « Screen Recording » permission (no dialog). Granted: remember
    /// this version as having it. Lost with an update: say so at once, with the way back.
    pub(super) fn screen_at_launch(&mut self) {
        if !cfg!(target_os = "macos") {
            return;
        }
        let ok = crate::screen::granted();
        self.orch.screen = Some((ok, Instant::now()));
        match crate::screen::at_launch(ok, &self.ws.settings.screen_granted_version, crate::update::VERSION) {
            crate::screen::AtLaunch::Fine => self.ws.settings.screen_granted_version = crate::update::VERSION.to_string(),
            crate::screen::AtLaunch::LostWithUpdate => self.info(crate::tr!(
                "La mise à jour a fait perdre à Orchestra l'autorisation « Enregistrement de l'écran » : les IA ne peuvent plus vérifier leur travail à l'écran. Page Orchestration › « Réaccorder » pour la retrouver.",
                "The update made Orchestra lose the « Screen Recording » permission: the AIs can no longer check their work on screen. Orchestration page › « Grant again » to get it back."
            )),
            crate::screen::AtLaunch::NeverGranted => {}
        }
    }

    /// The permission is missing: what it's for, and the two ways to give it back. Read
    /// again every few seconds while the page is open (a cheap call, no dialog).
    fn screen_card(&mut self, ui: &mut egui::Ui, t: &Tone) {
        if !cfg!(target_os = "macos") || self.orch.screen_dismissed {
            return;
        }
        let stale = self
            .orch
            .screen
            .is_none_or(|(_, at)| at.elapsed() > Duration::from_secs(3));
        if stale {
            let ok = crate::screen::granted();
            if ok {
                self.ws.settings.screen_granted_version = crate::update::VERSION.to_string();
            }
            self.orch.screen = Some((ok, Instant::now()));
        }
        if self.orch.screen.is_some_and(|(ok, _)| ok) {
            return;
        }
        ui.ctx().request_repaint_after(Duration::from_secs(3));
        let lost = !self.ws.settings.screen_granted_version.is_empty();
        let mut ask = false;
        let mut open = false;
        egui::Frame::new()
            .fill(super::projects::ORANGE.gamma_multiply(0.07))
            .stroke(egui::Stroke::new(1.0, super::projects::ORANGE.gamma_multiply(0.45)))
            .corner_radius(12.0)
            .inner_margin(egui::Margin::symmetric(18, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let title = if lost { crate::tr!("Autorisation « Enregistrement de l'écran » perdue", "« Screen Recording » permission lost") } else { crate::tr!("Autorisation « Enregistrement de l'écran » manquante", "« Screen Recording » permission missing") };
                ui.label(RichText::new(title).font(semibold(14.0)).color(super::projects::ORANGE));
                ui.add(
                    egui::Label::new(
                        RichText::new(crate::tr!(
                            "Sans elle, les IA lancées ici ne peuvent pas prendre de capture (take_screenshot) pour vérifier une interface. Cochez Orchestra dans Réglages Système › Confidentialité et sécurité › Enregistrement de l'écran, puis quittez et relancez Orchestra : macOS ne l'applique qu'au lancement suivant.",
                            "Without it, the AIs started here can't take a screenshot (take_screenshot) to check an interface. Tick Orchestra in System Settings › Privacy & Security › Screen Recording, then quit and reopen Orchestra: macOS only applies it at the next launch."
                        ))
                        .size(12.5)
                        .color(t.secondary),
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button(RichText::new(crate::tr!("Ouvrir les réglages", "Open the settings")).size(13.0)).clicked() {
                        open = true;
                    }
                    let again = ui
                        .button(RichText::new(crate::tr!("Réaccorder", "Grant again")).size(13.0))
                        .on_hover_text(crate::tr!(
                            "Efface l'ancienne entrée d'Orchestra dans les réglages si elle est restée d'une version précédente, puis macOS redemande l'autorisation",
                            "Clears Orchestra's old entry in the settings if it was left by an earlier version, then macOS asks for the permission again"
                        ));
                    if again.clicked() {
                        ask = true;
                    }
                    if super::projects::quiet_link(ui, crate::tr!("Plus tard", "Later"), t).clicked() {
                        self.orch.screen_dismissed = true;
                    }
                });
            });
        ui.add_space(14.0);
        if open && let Err(e) = crate::screen::open_settings() {
            self.info(e);
        }
        if ask {
            let mut v = self.ws.settings.screen_granted_version.clone();
            let ok = crate::screen::ensure_permission(&mut v);
            self.ws.settings.screen_granted_version = v;
            self.orch.screen = Some((ok, Instant::now()));
            if !ok {
                let _ = crate::screen::open_settings();
            }
        }
    }

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
                self.screen_card(ui, &t);
                self.week_bar(ui, &t);
                ui.add_space(14.0);
                self.stats_row(ui, &t);
                ui.add_space(14.0);
                self.running_card(ui, &t, actions);
                if !self.orch.runs.is_empty() {
                    ui.add_space(14.0);
                }
                // Sessions waiting for the subscription's quota: still there, suspended.
                if self.suspended_card(ui, &t, actions) {
                    ui.add_space(14.0);
                }
                // The AIs' questions: those waiting (a paused session goes on once answered),
                // then the answers of the last days.
                if self.questions_card(ui, &t) {
                    ui.add_space(14.0);
                }
                // Work left in copies whose end message was deleted: nowhere else to deal with it.
                if self.orphans_card(ui, &t, actions) {
                    ui.add_space(14.0);
                }
                // Too many days of reports: said here, where they are made.
                let old = self.ws.old_report_days().len();
                if old > 0 {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("📒").size(13.0));
                        ui.label(
                            RichText::new(crate::trf!(
                                "{old} jour(s) de rapports au-delà des {} plus récents : la page « Rapports des IA » devient longue.",
                                "{old} day(s) of reports beyond the {} most recent: the « AI reports » page is getting long.",
                                crate::model::REPORT_DAYS_KEPT
                            ))
                            .size(12.5)
                            .color(t.secondary),
                        );
                        if super::projects::action_link(ui, crate::tr!("Condenser en une synthèse", "Condense into a summary"))
                            .on_hover_text(crate::tr!(
                                "Les jours les plus anciens deviennent une seule page « Synthèse des anciens rapports » (titres et étapes terminées) ; les pages de ces jours vont à la corbeille, d'où elles peuvent revenir.",
                                "The oldest days become one « Summary of older reports » page (titles and finished steps); those days' pages go to the trash, where they can be brought back from."
                            ))
                            .clicked()
                            && let Some((page, n)) = self.ws.condense_old_reports()
                        {
                            self.dirty_since.get_or_insert_with(Instant::now);
                            self.ack(crate::trf!("{n} jour(s) de rapports condensés en une synthèse.", "{n} day(s) of reports condensed into a summary."));
                            actions.push(Action::Open(page));
                        }
                        if super::projects::action_link(ui, crate::tr!("Voir les rapports", "See the reports")).clicked() {
                            actions.push(Action::Open(self.ws.settings.orch.reports_page.clone()));
                        }
                    });
                    ui.add_space(14.0);
                }
                // The settings and limits come first: they frame every launch below.
                self.limits_card(ui, &t);
                ui.add_space(14.0);
                if let Some(msg) = self.launch_card(ui, &ctx, &t) {
                    self.notice(msg);
                }
                ui.add_space(40.0);
            });
        });
    }

    /// The bar at the very top: the week's use of the subscription, with the reserve kept
    /// for the person (set right here): past 100 − reserve, no session starts or goes on.
    fn week_bar(&mut self, ui: &mut egui::Ui, t: &Tone) {
        let week = self.orch.week;
        let before = (
            self.ws.settings.orch.week_limit,
            self.ws.settings.orch.reserve_asked,
        );
        let red = Color32::from_rgb(224, 62, 62);
        super::projects::card(ui, t, 0.0, |ui| {
            // Until the person has chosen it: the question, with a few ready answers.
            if !self.ws.settings.orch.reserve_asked {
                ui.label(
                    RichText::new(crate::tr!(
                        "Quelle part de l'abonnement garder pour vous ?",
                        "How much of the subscription should be kept for you?"
                    ))
                    .font(semibold(14.0))
                    .color(t.text),
                );
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
                    if ui
                        .button(
                            RichText::new(crate::trf!("Garder {keep:.0} %", "Keep {keep:.0} %"))
                                .color(t.secondary),
                        )
                        .clicked()
                    {
                        self.ws.settings.orch.reserve_asked = true;
                    }
                });
                ui.add_space(8.0);
            }
            let o = &mut self.ws.settings.orch;
            // The reserve, set right here (laid out right to left: its label ends up first).
            let reserve_ctl = |ui: &mut egui::Ui, o: &mut crate::model::OrchSettings| {
                let mut reserve = o.reserve();
                let r = ui.add(egui::DragValue::new(&mut reserve).range(0.0..=95.0).speed(0.5).suffix(" %")).on_hover_text(crate::tr!(
                    "Part de la semaine gardée pour vous : les IA s'arrêtent avant de l'entamer",
                    "Share of the week kept for you: the AIs stop before touching it"
                ));
                if r.changed() {
                    o.set_reserve(reserve);
                    o.reserve_asked = true;
                }
                ui.label(
                    RichText::new(crate::tr!("Réserve minimum", "Minimum reserve"))
                        .size(13.0)
                        .color(t.secondary),
                );
            };
            // Narrow: the reserve on a line of its own rather than over the figure.
            let wide = ui.available_width() >= WEEK_ONE_LINE;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(crate::tr!("CETTE SEMAINE", "THIS WEEK"))
                        .font(semibold(11.5))
                        .color(t.tertiary),
                );
                let used = match week {
                    Some(w) => crate::trf!(
                        "{w:.0} % de l'abonnement utilisés",
                        "{w:.0} % of the subscription used"
                    ),
                    None => crate::tr!(
                        "utilisation inconnue pour l'instant",
                        "usage unknown for now"
                    )
                    .into(),
                };
                ui.label(RichText::new(used).font(semibold(14.0)).color(t.text));
                if wide {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        reserve_ctl(ui, o)
                    });
                }
            });
            if !wide {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    reserve_ctl(ui, o)
                });
            }
            ui.add_space(4.0);
            let limit = self.ws.settings.orch.week_limit;
            let w = ui.available_width();
            let (bar, _) = ui.allocate_exact_size(egui::vec2(w, 12.0), egui::Sense::hover());
            let p = ui.painter();
            p.rect_filled(bar, 6.0, t.fill);
            let lx = bar.left() + bar.width() * limit / 100.0;
            p.rect_filled(
                egui::Rect::from_min_max(egui::pos2(lx, bar.top()), bar.max),
                egui::CornerRadius {
                    nw: 0,
                    sw: 0,
                    ne: 6,
                    se: 6,
                },
                red.gamma_multiply(0.12),
            );
            if let Some(f) = week {
                let frac = super::projects::smooth(ui, "orch-week", (f / 100.0).clamp(0.0, 1.0));
                let color = if f >= limit {
                    red
                } else if f >= limit * 0.85 {
                    super::projects::ORANGE
                } else {
                    super::projects::BLUE
                };
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        bar.min,
                        egui::vec2(bar.width() * frac, bar.height()),
                    ),
                    6.0,
                    color,
                );
            }
            ui.painter().vline(
                lx,
                egui::Rangef::new(bar.top() - 3.0, bar.bottom() + 3.0),
                egui::Stroke::new(2.0, super::projects::ORANGE),
            );
            // The day's pace: what went from the week in 24 hours against a seventh of it.
            if let Some((used, span)) = self.orch.day_week {
                let (line, fast) = crate::usage::day_pace_line(used, span);
                let color = if fast {
                    super::projects::ORANGE
                } else {
                    t.secondary
                };
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(line).size(12.5).color(color)).wrap()).on_hover_text(crate::trf!(
                    "Ce qui est parti de l'abonnement de la semaine sur les dernières 24 h, par vous et par les IA. La part d'un jour est un septième de la semaine : à ce rythme, la semaine dure pile jusqu'à sa remise à zéro. Plus vite (en orange), elle s'épuiserait avant ; c'est {:.0} % de la part d'un jour.",
                    "What went from the week's subscription over the last 24 hours, by you and by the AIs. A day's share is a seventh of the week: at that pace, the week lasts exactly until it resets. Faster (in orange), it would run out before; that is {:.0} % of a day's share.",
                    crate::usage::daily_pct(used)
                ));
            }
        });
        if (
            self.ws.settings.orch.week_limit,
            self.ws.settings.orch.reserve_asked,
        ) != before
        {
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
        let five_left = self
            .orch
            .five_reset
            .and_then(|r| crate::usage::until(r, now_ms));
        let week_left = self
            .orch
            .week_reset
            .and_then(|r| crate::usage::until(r, now_ms));
        let running = self.orch.runs.len();
        let ready = self
            .ws
            .projects()
            .iter()
            .filter(|p| {
                p.project
                    .as_ref()
                    .is_some_and(|x| !x.folder.trim().is_empty())
            })
            .count();
        let today = auto::local_day(crate::model::now_ms());
        let todays: Vec<&crate::model::RunRecord> = self
            .ws
            .runs
            .iter()
            .filter(|r| !r.outcome.is_empty() && auto::local_day(r.started) == today)
            .collect();
        let steps: usize = todays.iter().map(|r| r.steps_done.len()).sum();
        let hour = self.ws.settings.orch.report_hour;
        let w = ui.available_width();
        let gap = 14.0;
        let cols = if w >= 520.0 { 3 } else { 1 };
        let tw = if cols == 3 { (w - 2.0 * gap) / 3.0 } else { w };
        let h = if cols == 3 { 112.0 } else { 92.0 };
        let draw = |ui: &mut egui::Ui, i: usize| {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(tw, h), egui::Sense::hover());
            let a = ui
                .ctx()
                .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.18);
            let accent = [
                Color32::from_rgb(212, 76, 125),
                super::projects::GREEN,
                super::projects::BLUE,
            ][i];
            let p = ui.painter();
            p.add(egui::epaint::RectShape::new(
                rect,
                14.0,
                t.card,
                egui::Stroke::new(1.0, t.border.lerp_to_gamma(accent, 0.35 * a)),
                egui::StrokeKind::Inside,
            ));
            // A soft wash of the accent in the corner.
            p.with_clip_rect(rect.shrink(1.0)).circle_filled(
                rect.right_top() + egui::vec2(-10.0, 10.0),
                52.0 + 6.0 * a,
                accent.gamma_multiply(0.07),
            );
            let x = rect.left() + 18.0;
            // Each figure once on the page: the week (and the day's pace) in the bar above, the
            // 5-hour window here (the one the sessions stop at), counts in the other tiles.
            let label = [
                crate::tr!("FENÊTRE DE 5 H", "5-HOUR WINDOW"),
                crate::tr!("IA AU TRAVAIL", "AIS AT WORK"),
                crate::tr!("AUJOURD'HUI", "TODAY"),
            ][i];
            p.text(
                egui::pos2(x, rect.top() + 16.0),
                egui::Align2::LEFT_TOP,
                label,
                semibold(11.5),
                t.tertiary,
            );
            let (big, sub): (String, String) = match i {
                0 => (
                    five.map(|f| format!("{f:.0} %"))
                        .unwrap_or_else(|| "—".into()),
                    match (five, five_left) {
                        (Some(_), Some(l)) => crate::trf!(
                            "repart dans {} · IA arrêtées à {global:.0} %",
                            "resets in {} · AIs stop at {global:.0} %",
                            crate::usage::left_text(l)
                        ),
                        (Some(_), None) => {
                            crate::trf!("IA arrêtées à {global:.0} %", "AIs stop at {global:.0} %")
                        }
                        _ => crate::tr!("inconnu pour l'instant", "unknown for now").into(),
                    },
                ),
                1 => {
                    let s = crate::i18n::plural(ready);
                    (
                        running.to_string(),
                        if running == 0 {
                            crate::trf!(
                                "{ready} projet{s} prêt{s} à lancer",
                                "{ready} project{s} ready to start"
                            )
                        } else {
                            crate::trf!(
                                "sur {ready} projet{s} prêt{s}",
                                "of {ready} project{s} ready"
                            )
                        },
                    )
                }
                _ => (
                    todays.len().to_string(),
                    if todays.is_empty() {
                        crate::trf!(
                            "session terminée · bilan à {hour} h",
                            "finished session · summary at {hour} h"
                        )
                    } else {
                        let (s, e) = (
                            crate::i18n::plural(todays.len()),
                            crate::i18n::plural(steps),
                        );
                        crate::trf!(
                            "session{s} · {steps} étape{e} faite{e}",
                            "session{s} · {steps} step{e} done"
                        )
                    },
                ),
            };
            p.text(
                egui::pos2(x, rect.top() + 36.0),
                egui::Align2::LEFT_TOP,
                &big,
                bold(30.0),
                t.text,
            );
            let mut job = egui::text::LayoutJob::single_section(
                sub,
                egui::TextFormat::simple(FontId::proportional(12.5), t.secondary),
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(
                tw - 36.0 - if i == 0 && tw >= 240.0 { 70.0 } else { 0.0 },
            );
            let g = ui.fonts_mut(|f| f.layout_job(job));
            ui.painter()
                .galley(egui::pos2(x, rect.bottom() - 28.0), g, t.secondary);
            match i {
                0 => {
                    if tw >= 240.0 {
                        // A ring: the 5-hour use, the global limit as a tick.
                        let c = egui::pos2(rect.right() - 50.0, rect.center().y + 4.0);
                        ring(
                            ui,
                            c,
                            28.0,
                            five.map(|f| f / 100.0),
                            global / 100.0,
                            accent,
                            t,
                        );
                    }
                    // The details, on hover only.
                    let mut tip = vec![crate::trf!(
                        "Part de la fenêtre de 5 h de l'abonnement Claude déjà utilisée, par les IA d'Orchestra et par vous (Claude Code, claude.ai…) : le chiffre de Claude Code lui-même.\nLe trait orange de l'anneau est la limite globale ({global:.0} %) : aucune session autonome ne démarre ni ne continue au-delà. Réglable plus bas, dans « Réglages et limites ».",
                        "Share of the Claude subscription's 5-hour window already used, by Orchestra's AIs and by you (Claude Code, claude.ai…): Claude Code's own figure.\nThe ring's orange tick is the global limit ({global:.0} %): no autonomous session starts or goes on past it. Set below, in « Settings and limits »."
                    )];
                    if let Some(d) = day {
                        tip.push(crate::trf!("Dernières 24 h : {d:.0} % (les fenêtres de 5 h de la journée, additionnées)", "Last 24 hours: {d:.0} % (the day's 5-hour windows, added up)"));
                    }
                    if let Some(l) = week_left.filter(|_| week.is_some()) {
                        tip.push(crate::trf!(
                            "La semaine repart à zéro dans {}",
                            "The week resets in {}",
                            crate::usage::left_text(l)
                        ));
                    }
                    resp.on_hover_text(tip.join("\n"));
                }
                1 if running > 0 => {
                    super::projects::pulse_dot(
                        ui,
                        egui::pos2(rect.right() - 30.0, rect.top() + 24.0),
                        super::projects::GREEN,
                    );
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
                ui.painter()
                    .rect_filled(r, 2.0, super::projects::accent_of(title));
                ui.add_space(2.0);
                ui.label(RichText::new(title).font(semibold(12.5)).color(t.secondary));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if open {
                        crate::tr!("Replier ▴", "Fold ▴")
                    } else {
                        crate::tr!("Modifier ▾", "Edit ▾")
                    };
                    if super::projects::action_link(ui, label).clicked() {
                        open = !open;
                    }
                });
            });
            if !open {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(limits_summary(o))
                        .size(12.5)
                        .color(t.tertiary),
                );
                return;
            }
            ui.add_space(8.0);
            // The 5-hour window's use is shown once, in the tile at the top (with this limit
            // as the tick of its ring): here, only the settings.
            let w = ui.available_width();
            let lk = super::settings::look(ui.visuals().dark_mode);
            let row = |ui: &mut egui::Ui,
                       label: &str,
                       hint: &str,
                       add: &mut dyn FnMut(&mut egui::Ui)| {
                ui.horizontal(|ui| {
                    ui.set_min_height(32.0);
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - 150.0).max(120.0));
                        ui.label(RichText::new(label).size(13.5).color(t.text));
                        if !hint.is_empty() {
                            ui.add(
                                egui::Label::new(RichText::new(hint).size(12.0).color(t.tertiary))
                                    .wrap(),
                            );
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        add(ui)
                    });
                });
                ui.add_space(4.0);
            };
            let two = w >= 640.0;
            let col_w = if two { (w - 28.0) / 2.0 } else { w };
            let mut left = |ui: &mut egui::Ui| {
                ui.set_width(col_w);
                row(
                    ui,
                    crate::tr!("Limite globale", "Global limit"),
                    crate::tr!(
                        "De la fenêtre de 5 h, votre propre usage compris : aucune session ne va au-delà, tous projets confondus. Chaque session a en plus son budget, compté depuis son lancement.",
                        "Of the 5-hour window, your own use included: no session goes past it, all projects together. Each session also has its budget, counted from its start."
                    ),
                    &mut |ui| {
                        ui.add(
                            egui::DragValue::new(&mut o.global_limit)
                                .range(5.0..=100.0)
                                .speed(0.5)
                                .suffix(" %"),
                        );
                    },
                );
                row(
                    ui,
                    crate::tr!("Commits autorisés", "Commits allowed"),
                    crate::tr!(
                        "Sur leur branche, envoyée sur GitHub par Orchestra",
                        "On their branch, pushed to GitHub by Orchestra"
                    ),
                    &mut |ui| {
                        super::settings::switch(ui, &mut o.allow_commits, &lk);
                    },
                );
                row(
                    ui,
                    crate::tr!("Pause quand l'IA attend", "Pause when the AI waits"),
                    crate::tr!(
                        "Autorisation, dossier à approuver ou question posée : la session attend votre réponse au lieu de refuser ou de s'arrêter",
                        "A permission, a folder to approve or a question asked: the session waits for your answer instead of refusing or stopping"
                    ),
                    &mut |ui| {
                        super::settings::switch(ui, &mut o.pause_for_answer, &lk);
                    },
                );
                row(
                    ui,
                    crate::tr!("Reprise automatique", "Automatic restart"),
                    crate::tr!(
                        "Une session arrêtée par la limite globale ou la réserve de la semaine repart seule après la remise à zéro du quota",
                        "A session stopped by the global limit or the week's reserve starts again by itself once the quota resets"
                    ),
                    &mut |ui| {
                        super::settings::switch(ui, &mut o.auto_resume, &lk);
                    },
                );
            };
            let mut right = |ui: &mut egui::Ui| {
                ui.set_width(col_w);
                row(
                    ui,
                    crate::tr!("Bilan de la journée", "Daily summary"),
                    crate::tr!(
                        "Quand plusieurs projets ont travaillé",
                        "When several projects worked"
                    ),
                    &mut |ui| {
                        let mut h = o.report_hour as u32;
                        if ui
                            .add(egui::DragValue::new(&mut h).range(0..=23).suffix(" h"))
                            .changed()
                        {
                            o.report_hour = h as u8;
                        }
                    },
                );
                row(
                    ui,
                    crate::tr!("Tours au plus", "Turns at most"),
                    crate::tr!("Par session", "Per session"),
                    &mut |ui| {
                        ui.add(egui::DragValue::new(&mut o.max_turns).range(1..=200));
                    },
                );
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
                // The rules dropped, said rather than silently ignored.
                let rejected = auto::rejected_extra(&o.extra_tools);
                if !rejected.is_empty() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(crate::trf!(
                                "Ignorées, trop larges ou contraires aux commandes toujours refusées : {}. Donnez une commande précise, ex. Bash(git log:*) plutôt que Bash(git:*).",
                                "Ignored, too wide or against the commands always refused: {}. Give a precise command, e.g. Bash(git log:*) rather than Bash(git:*).",
                                rejected.join(", ")
                            ))
                            .size(12.0)
                            .color(super::projects::ORANGE),
                        )
                        .wrap(),
                    );
                }
            });
        });
        ui.ctx().data_mut(|d| d.insert_temp(open_key, open));
        if *o != before {
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    fn running_card(&mut self, ui: &mut egui::Ui, t: &Tone, actions: &mut Vec<Action>) {
        if self.orch.runs.is_empty() {
            return;
        }
        // « Ajouter des tâches »: the project whose page to open.
        let mut add_tasks: Option<String> = None;
        let dark = ui.visuals().dark_mode;
        let watching = self.orch.watching.clone();
        let mut stop: Option<(usize, bool)> = None;
        let mut watch: Option<Option<String>> = None;
        let n = self.orch.runs.len();
        let awake = self.orch.awake.held();
        let (five, global) = (self.orch.five, self.ws.settings.orch.global_limit);
        // The step each session is on (« doing », reserved by this computer).
        let me = crate::store::computer_id();
        let working_on: HashMap<String, String> = self
            .orch
            .runs
            .iter()
            .filter_map(|r| {
                let steps = &self.ws.page(&r.project)?.project.as_ref()?.steps;
                Some((r.id.clone(), current_step(steps, &me)?.to_string()))
            })
            .collect();
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
                // The session's links; in a right-to-left layout « Forcer l'arrêt » ends up last.
                let mut links = |ui: &mut egui::Ui, r: &mut LiveRun| {
                    if super::projects::action_link(ui, crate::tr!("Forcer l'arrêt", "Force stop"))
                        .on_hover_text(crate::tr!(
                            "Ferme Claude Code tout de suite, sans compte rendu",
                            "Closes Claude Code right away, without a summary"
                        ))
                        .clicked()
                    {
                        stop = Some((i, true));
                    }
                    if r.stop.is_none()
                        && super::projects::action_link(ui, crate::tr!("Arrêter", "Stop"))
                            .on_hover_text(crate::tr!(
                                "Finit le tour en cours, écrit le compte rendu, puis s'arrête",
                                "Finishes the current turn, writes the summary, then stops"
                            ))
                            .clicked()
                    {
                        stop = Some((i, false));
                    }
                    if r.waiting.is_some()
                        && super::projects::action_link(ui, crate::tr!("Reprendre", "Resume"))
                            .on_hover_text(crate::tr!(
                                "L'IA repart ; une question restée à l'écran est refusée",
                                "The AI goes on; a question still on the screen is refused"
                            ))
                            .clicked()
                    {
                        r.resume();
                    }
                    let open = watching.as_deref() == Some(r.id.as_str());
                    if super::projects::action_link(
                        ui,
                        if open {
                            crate::tr!("Masquer", "Hide")
                        } else {
                            crate::tr!("Regarder", "Watch")
                        },
                    )
                    .clicked()
                    {
                        watch = Some(if open { None } else { Some(r.id.clone()) });
                    }
                    if super::projects::action_link(ui, crate::tr!("+ Ajouter des tâches", "+ Add tasks"))
                        .on_hover_text(crate::tr!("Ouvre les étapes du projet : l'IA prendra les nouvelles sans que la session s'arrête", "Opens the project's steps: the AI takes the new ones without the session stopping"))
                        .clicked()
                    {
                        add_tasks = Some(r.project.clone());
                    }
                };
                egui::Frame::new()
                    .fill(super::projects::GREEN.gamma_multiply(0.06))
                    .stroke(egui::Stroke::new(
                        1.0,
                        super::projects::GREEN.gamma_multiply(0.3),
                    ))
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        // Room for the title, its state and the links on one line.
                        let wide = ui.available_width() >= RUN_ONE_LINE;
                        ui.horizontal(|ui| {
                            let (dot, _) = ui
                                .allocate_exact_size(egui::vec2(12.0, 18.0), egui::Sense::hover());
                            super::projects::pulse_dot(ui, dot.center(), super::projects::GREEN);
                            ui.label(RichText::new(&r.title).font(semibold(14.5)).color(t.text));
                            let state = match r.phase {
                                Phase::Starting(_) => crate::tr!("démarre", "starting").to_string(),
                                Phase::Summary(_) => {
                                    crate::tr!("compte rendu", "summary").to_string()
                                }
                                Phase::Closing(_) => crate::tr!("se ferme", "closing").to_string(),
                                Phase::Waiting => crate::tr!("en pause", "paused").to_string(),
                                _ => format!("tour {}", r.turns + 1),
                            };
                            let state_color = if r.waiting.is_some() {
                                super::projects::ORANGE
                            } else {
                                t.tertiary
                            };
                            ui.label(RichText::new(state).size(12.5).color(state_color));
                            if let Some(c) = &r.endless {
                                ui.label(
                                    RichText::new(crate::tr!("∞ sans fin", "∞ endless"))
                                        .size(12.5)
                                        .color(super::projects::BLUE),
                                )
                                .on_hover_text(crate::trf!(
                                    "S'arrête après : {}, ou sur son budget",
                                    "Stops after: {}, or on its budget",
                                    c.describe()
                                ));
                            }
                            if wide {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| links(ui, r),
                                );
                            }
                        });
                        // Narrow: the links on a line of their own rather than over the title.
                        if !wide {
                            ui.horizontal_wrapped(|ui| {
                                ui.add_space(20.0);
                                links(ui, r);
                            });
                        }
                        share_row(ui, &r.id, r.pct, r.limit, five, global, t);
                        // The task in hand, as a short sentence.
                        if let Some(step) = working_on.get(&r.id) {
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                ui.add_space(20.0);
                                ui.label(
                                    RichText::new("✦").size(13.0).color(super::projects::BLUE),
                                );
                                let mut job = egui::text::LayoutJob::default();
                                job.append(
                                    crate::tr!("En train de s'occuper de ", "Busy with "),
                                    0.0,
                                    egui::TextFormat {
                                        font_id: FontId::proportional(13.5),
                                        color: t.secondary,
                                        italics: true,
                                        ..Default::default()
                                    },
                                );
                                job.append(
                                    &format!("« {step} »"),
                                    0.0,
                                    egui::TextFormat::simple(semibold(13.5), t.text),
                                );
                                job.wrap = egui::text::TextWrapping::truncate_at_width(
                                    ui.available_width(),
                                );
                                super::projects::cut_label(ui, job).on_hover_text(step);
                            });
                        }
                        ui.horizontal(|ui| {
                            ui.add_space(20.0);
                            let mut job = egui::text::LayoutJob::single_section(
                                r.last_line.clone(),
                                egui::TextFormat::simple(FontId::proportional(12.5), t.tertiary),
                            );
                            job.wrap =
                                egui::text::TextWrapping::truncate_at_width(ui.available_width());
                            super::projects::cut_label(ui, job);
                        });
                        if let Some(why) = &r.stop {
                            ui.horizontal(|ui| {
                                ui.add_space(20.0);
                                // Wrapped: a long reason must not widen the cards below.
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(crate::trf!(
                                            "S'arrête : {why}",
                                            "Stopping: {why}"
                                        ))
                                        .size(12.5)
                                        .color(super::projects::ORANGE),
                                    )
                                    .wrap(),
                                );
                            });
                        }
                        if watching.as_deref() == Some(r.id.as_str()) {
                            egui::Frame::new()
                                .fill(Color32::from_rgb(20, 20, 20))
                                .corner_radius(8.0)
                                .inner_margin(8)
                                .show(ui, |ui| {
                                    ui.set_height(420.0);
                                    ui.set_width(ui.available_width());
                                    super::term_ui::terminal_view(
                                        ui,
                                        &mut r.term,
                                        Color32::from_rgb(20, 20, 20),
                                        dark || true,
                                        false,
                                    );
                                });
                        }
                    });
                ui.add_space(8.0);
            }
        });
        if let Some(w) = watch {
            self.orch.watching = w;
        }
        if let Some(p) = add_tasks {
            super::projects::focus_steps(ui.ctx(), &p);
            actions.push(Action::Open(p));
        }
        if let Some((i, force)) = stop {
            if force {
                let r = &mut self.orch.runs[i];
                r.stop = Some(crate::tr!("arrêtée à la main", "stopped by hand").into());
                self.finish_run(i, crate::tr!("arrêtée à la main", "stopped by hand"), "");
                self.orch.runs.remove(i);
            } else {
                self.orch.runs[i].stop =
                    Some(crate::tr!("arrêtée à la main", "stopped by hand").into());
            }
        }
    }

    /// The sessions stopped by the 5-hour or weekly limit that start again by themselves
    /// once it resets: shown like the running ones, in orange, suspended. False when none.
    fn suspended_card(&mut self, ui: &mut egui::Ui, t: &Tone, actions: &mut Vec<Action>) -> bool {
        let now = crate::model::now_ms();
        let running: Vec<&str> = self.orch.runs.iter().map(|r| r.project.as_str()).collect();
        let list = crate::model::suspended_runs(&self.ws.runs, now, &running);
        if list.is_empty() {
            return false;
        }
        let orange = super::projects::ORANGE;
        let mut cancel: Option<String> = None;
        let mut add_tasks: Option<String> = None;
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(
                ui,
                crate::tr!("SUSPENDUES", "SUSPENDED"),
                &list.len().to_string(),
                t,
            );
            for &i in &list {
                let r = &self.ws.runs[i];
                egui::Frame::new().fill(orange.gamma_multiply(0.07)).stroke(egui::Stroke::new(1.0, orange.gamma_multiply(0.35))).corner_radius(10.0).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        // A still dot (pause sign) where a running session pulses.
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 18.0), egui::Sense::hover());
                        for dx in [-2.5, 2.5] {
                            ui.painter().line_segment([dot.center() + egui::vec2(dx, -4.0), dot.center() + egui::vec2(dx, 4.0)], egui::Stroke::new(2.0, orange));
                        }
                        ui.label(RichText::new(&r.title).font(semibold(14.5)).color(t.text));
                        ui.label(RichText::new(crate::tr!("suspendue", "suspended")).size(12.5).color(orange));
                        if r.endless.is_some() {
                            ui.label(RichText::new(crate::tr!("∞ sans fin", "∞ endless")).size(12.5).color(super::projects::BLUE));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if super::projects::action_link(ui, crate::tr!("Annuler la reprise", "Cancel the restart"))
                                .on_hover_text(crate::tr!("La session ne repartira pas d'elle-même", "The session won't start again by itself"))
                                .clicked()
                            {
                                cancel = Some(r.project.clone());
                            }
                            if super::projects::action_link(ui, crate::tr!("+ Ajouter des tâches", "+ Add tasks")).on_hover_text(crate::tr!("L'IA les prendra à la reprise", "The AI takes them when it restarts")).clicked() {
                                add_tasks = Some(r.project.clone());
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(20.0);
                        ui.label(
                            RichText::new(crate::trf!(
                                "Reprend toute seule à {} (dans {}), quand le quota de l'abonnement revient.",
                                "Starts again by itself at {} (in {}), when the subscription's quota is back.",
                                auto::hm(r.resume_at),
                                crate::usage::left_text(r.resume_at.saturating_sub(now))
                            ))
                            .size(12.5)
                            .color(t.secondary),
                        );
                    });
                    if !r.outcome.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(20.0);
                            ui.label(RichText::new(crate::trf!("Mise en pause : {}", "Paused: {}", r.outcome)).size(12.0).color(t.tertiary));
                        });
                    }
                });
                ui.add_space(8.0);
            }
        });
        if let Some(p) = cancel {
            for x in self.ws.runs.iter_mut().filter(|x| x.project == p) {
                x.resume_at = 0;
            }
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        if let Some(p) = add_tasks {
            super::projects::focus_steps(ui.ctx(), &p);
            actions.push(Action::Open(p));
        }
        // The time left changes: keep it fresh.
        ui.ctx().request_repaint_after(Duration::from_secs(30));
        true
    }

    /// The AIs' questions (ask_human) on the Orchestration page: those still waiting, with
    /// a box to answer them right here, then the recent answers. False when there is none.
    fn questions_card(&mut self, ui: &mut egui::Ui, t: &Tone) -> bool {
        let shown = orch_questions(&self.ws.inbox, crate::model::now_ms(), ORCH_ANSWERED_MS);
        if shown.is_empty() {
            return false;
        }
        let msgs: Vec<&crate::model::InboxMsg> = shown.iter().map(|&i| &self.ws.inbox[i]).collect();
        let titles: HashMap<&str, String> = msgs
            .iter()
            .filter_map(|m| {
                self.ws
                    .page(&m.project)
                    .map(|p| (m.project.as_str(), p.display_title()))
            })
            .collect();
        let mut answer: Option<(String, String)> = None;
        let mut to_inbox = false;
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(
                ui,
                crate::tr!("QUESTIONS DES IA", "THE AIS' QUESTIONS"),
                &questions_count(&msgs),
                t,
            );
            answer = question_list(ui, &msgs, |m| titles.get(m.project.as_str()).cloned(), t);
            ui.add_space(4.0);
            if super::projects::action_link(
                ui,
                crate::tr!(
                    "Tout voir dans la boîte de réception",
                    "See everything in the inbox"
                ),
            )
            .clicked()
            {
                to_inbox = true;
            }
        });
        if let Some((id, text)) = answer {
            self.save_answer_to(&id, text);
        }
        if to_inbox {
            self.view = View::Inbox;
        }
        true
    }

    /// `save_answer` for the inbox message `id` (nothing if it is gone meanwhile).
    pub(super) fn save_answer_to(&mut self, id: &str, text: String) {
        if let Some(i) = self.ws.inbox.iter().position(|m| m.id == id) {
            self.save_answer(i, text);
        }
    }

    /// The person answers question `i` of the inbox (from the inbox, the Orchestration
    /// page or the project's page): kept with the question, read by the AI at the
    /// project's next state of play.
    fn save_answer(&mut self, i: usize, text: String) {
        let Some(m) = self.ws.inbox.get_mut(i) else {
            return;
        };
        m.answer = text;
        m.answered_at = crate::model::now_ms();
        m.read = true;
        self.dirty_since.get_or_insert_with(Instant::now);
        self.ack(crate::tr!(
            "Réponse enregistrée : l'IA la verra au prochain état des lieux du projet.",
            "Answer saved: the AI will see it in the project's next state of play."
        ));
    }

    /// The projects that can be started; returns a message to show.
    fn launch_card(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        t: &Tone,
    ) -> Option<super::notices::Notice> {
        let mut msg = None;
        let projects: Vec<(String, String, String)> = self
            .ws
            .projects()
            .iter()
            .filter_map(|p| {
                p.project
                    .as_ref()
                    .map(|x| (p.id.clone(), p.display_title(), x.folder.trim().to_string()))
            })
            .filter(|(_, _, f)| !f.is_empty())
            .collect();
        let (dl, dc) = (
            self.ws.settings.orch.default_limit,
            self.ws.settings.orch.allow_commits,
        );
        for (id, _, _) in &projects {
            let pr = self.ws.page(id).and_then(|p| p.project.as_ref());
            self.orch
                .drafts
                .entry(id.clone())
                .or_insert_with(|| Draft::new(pr, dl, dc));
        }
        let prefs_before: HashMap<String, crate::project::LaunchPrefs> = self
            .orch
            .drafts
            .iter()
            .map(|(id, d)| (id.clone(), d.prefs()))
            .collect();
        let running: Vec<String> = self.orch.runs.iter().map(|r| r.project.clone()).collect();
        let plugged = self.ais.plugged().contains(&crate::setup::Ai::ClaudeCode);
        let mut start: Vec<String> = vec![];
        let picked: Vec<String> = projects
            .iter()
            .filter(|(id, _, _)| {
                self.orch.drafts.get(id).is_some_and(|d| d.pick) && !running.contains(id)
            })
            .map(|(id, _, _)| id.clone())
            .collect();
        let picked_sum: f32 = picked
            .iter()
            .filter_map(|id| self.orch.drafts.get(id))
            .map(|d| d.limit)
            .sum();
        let global = self.ws.settings.orch.global_limit;
        // Shares add up: the sessions running and the projects picked together never
        // get more than the global limit.
        let running_sum: f32 = self.orch.runs.iter().map(|r| r.limit).sum();
        let open_steps: HashMap<String, usize> = projects
            .iter()
            .map(|(id, _, _)| {
                (
                    id.clone(),
                    self.ws
                        .page(id)
                        .and_then(|p| p.project.as_ref())
                        .map(|x| {
                            x.steps
                                .iter()
                                .filter(|s| s.status != crate::project::StepStatus::Done)
                                .count()
                        })
                        .unwrap_or(0),
                )
            })
            .collect();
        // « sur main » only for code projects: a project without code (school work,
        // paperwork…) never shows git words, and its sessions never merge into main.
        let mut to_main: HashMap<String, bool> = projects
            .iter()
            .filter_map(|(id, _, _)| {
                let pr = self.ws.page(id).and_then(|p| p.project.as_ref())?;
                super::projects::shows_git(ui.ctx(), pr).then(|| (id.clone(), pr.commit_to_main))
            })
            .collect();
        let to_main_before = to_main.clone();
        let lk = super::settings::look(ui.visuals().dark_mode);
        let mut confirm_main = self.orch.confirm_main.clone();
        let mut budgets: Vec<(String, f32)> = vec![];
        let drafts = &mut self.orch.drafts;
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(
                ui,
                crate::tr!("LANCER UNE SESSION", "START A SESSION"),
                "Claude Code",
                t,
            );
            if !plugged {
                ui.label(
                    RichText::new(crate::tr!(
                        "Branchez d'abord Claude Code (Réglages › IA).",
                        "Plug in Claude Code first (Settings › AI)."
                    ))
                    .color(super::projects::ORANGE),
                );
            }
            if projects.is_empty() {
                ui.label(
                    RichText::new(crate::tr!(
                        "Aucun projet avec un dossier pour l'instant.",
                        "No project with a folder yet."
                    ))
                    .color(t.tertiary)
                    .italics(),
                );
            }
            // What can start first: the running sessions, then the projects with open steps;
            // those with nothing to do (no open step, not endless) folded under a link.
            let rows: Vec<LaunchRow> = projects
                .iter()
                .map(|(id, _, _)| LaunchRow {
                    busy: running.contains(id),
                    open: open_steps.get(id).copied().unwrap_or(0),
                    keep: drafts.get(id).is_some_and(|d| d.endless || d.pick),
                })
                .collect();
            let idle_key = egui::Id::unique("orch-show-idle");
            let show_idle = ui.ctx().data(|d| d.get_temp::<bool>(idle_key)).unwrap_or(false);
            let (order, folded) = launch_order(&rows, show_idle);
            for (id, title, folder) in order.iter().map(|&i| &projects[i]) {
                let d = drafts.get_mut(id).unwrap();
                let busy = running.contains(id);
                let bg = super::projects::hover_begin(ui);
                let open = open_steps.get(id).copied().unwrap_or(0);
                let room = room_for(global, running_sum, picked_sum, d.pick.then_some(d.limit));
                // Room for the project and its controls on one line (about 620 px).
                let wide = ui.available_width() >= LAUNCH_ONE_LINE;
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
                                let (txt, col) = if busy { (crate::tr!("au travail", "at work").to_string(), super::projects::GREEN) } else if open == 0 { (crate::tr!("aucune étape ouverte", "no open step").to_string(), t.tertiary) } else { ({ let s = crate::i18n::plural(open); crate::trf!("{open} étape{s} ouverte{s}", "{open} open step{s}") }, t.secondary) };
                                ui.label(RichText::new(txt).size(12.0).color(col));
                            });
                            let mut job = egui::text::LayoutJob::single_section(super::projects::tilde(folder), egui::TextFormat::simple(FontId::monospace(11.0), t.tertiary));
                            job.wrap = egui::text::TextWrapping::truncate_at_width(if wide { (ui.available_width() - 250.0).max(80.0) } else { ui.available_width().max(40.0) });
                            super::projects::cut_label(ui, job);
                        });
                        if wide {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                launch_controls(ui, LaunchCx { id, d: &mut *d, busy, plugged, open, room, global, to_main: &mut to_main, start: &mut start, confirm_main: &mut confirm_main, budgets: &mut budgets, lk: &lk, t });
                            });
                        }
                    });
                    // Narrow: the controls under the project, left to right, instead of over it.
                    if !wide {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(70.0);
                            launch_controls(ui, LaunchCx { id, d: &mut *d, busy, plugged, open, room, global, to_main: &mut to_main, start: &mut start, confirm_main: &mut confirm_main, budgets: &mut budgets, lk: &lk, t });
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(70.0);
                        ui.add(egui::TextEdit::singleline(&mut d.mission).hint_text(default_mission()).desired_width(f32::INFINITY).font(FontId::proportional(13.0)).margin(egui::Margin::symmetric(8, 5)));
                    });
                    if !busy {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(70.0);
                            ui.spacing_mut().item_spacing.x = 6.0;
                            super::settings::switch(ui, &mut d.endless, &lk).on_hover_text(crate::tr!(
                                "La session ne s'arrête ni au nombre de tours des réglages ni à la fin des étapes : quand il n'y en a plus, l'IA trouve elle-même les suivantes (3 à la fois au plus) et continue. Elle s'arrête seulement sur les conditions choisies ici, ou sur son budget, la limite globale et la réserve de la semaine.\nCe choix et ses conditions sont gardés pour les prochaines sessions de ce projet.",
                                "The session stops neither at the settings' number of turns nor when the steps run out: then the AI finds the next ones itself (3 at a time at most) and goes on. It only stops on the conditions chosen here, or on its budget, the global limit and the week's reserve.\nThis choice and its conditions are kept for this project's next sessions."
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
                super::projects::hover_end(
                    ui,
                    bg,
                    row.response.rect,
                    egui::Id::unique(("orch-row", id)),
                    t,
                );
                ui.add_space(10.0);
            }
            let idle = rows.iter().filter(|r| r.idle()).count();
            if idle > 0 {
                let s = crate::i18n::plural(idle);
                let text = if folded > 0 {
                    crate::trf!(
                        "{idle} projet{s} sans étape ouverte ▾",
                        "{idle} project{s} with no open step ▾"
                    )
                } else {
                    crate::tr!("Replier les projets sans étape ouverte ▴", "Fold the projects with no open step ▴").to_string()
                };
                if super::projects::action_link(ui, &text)
                    .on_hover_text(crate::tr!(
                        "Une session sur un projet sans étape ouverte s'arrête tout de suite, sauf en mode sans fin : ajoutez-lui des étapes d'abord.",
                        "A session on a project with no open step stops at once, unless endless: add steps to it first."
                    ))
                    .clicked()
                {
                    ui.ctx().data_mut(|d| d.insert_temp(idle_key, !show_idle));
                }
            }
            if !picked.is_empty() {
                ui.add_space(4.0);
                // Picked projects set to endless on main: confirmed once for the whole selection.
                let risky: Vec<&str> = projects
                    .iter()
                    .filter(|(id, _, _)| {
                        picked.contains(id)
                            && drafts.get(id).is_some_and(|d| {
                                endless_on_main(
                                    d.endless,
                                    to_main.get(id).copied().unwrap_or(false),
                                )
                            })
                    })
                    .map(|(_, title, _)| title.as_str())
                    .collect();
                if risky.is_empty() && confirm_main.as_deref() == Some(SELECTION) {
                    confirm_main = None;
                }
                if confirm_main.as_deref() == Some(SELECTION) {
                    ui.label(
                        RichText::new(crate::trf!(
                            "⚠ {} ({})",
                            "⚠ {} ({})",
                            endless_main_warning(),
                            risky.join(", ")
                        ))
                        .size(12.5)
                        .color(super::projects::ORANGE),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add(egui::Button::new(
                                RichText::new(crate::tr!("Lancer quand même", "Start anyway"))
                                    .size(13.0)
                                    .color(super::projects::ORANGE),
                            ))
                            .clicked()
                        {
                            start.extend(picked.iter().cloned());
                            confirm_main = None;
                        }
                        if super::projects::quiet_link(ui, crate::tr!("Annuler", "Cancel"), t)
                            .clicked()
                        {
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
                    let s = crate::i18n::plural(picked.len());
                    let note = crate::trf!("{} projet{s} en parallèle · {picked_sum:.0} % au total pour une limite globale de {global:.0} %", "{} project{s} in parallel · {picked_sum:.0} % in total for a global limit of {global:.0} %", picked.len());
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
        // A budget chosen by the person stays for the next sessions, and after a restart.
        for (id, limit) in budgets {
            if let Some(p) = self.ws.page_mut(&id).and_then(|p| p.project.as_mut()) {
                p.session_limit = Some(limit);
                self.dirty_since.get_or_insert_with(Instant::now);
            }
        }
        // So do commits, endless and its end conditions.
        let changed: Vec<(String, crate::project::LaunchPrefs)> = self
            .orch
            .drafts
            .iter()
            .map(|(id, d)| (id.clone(), d.prefs()))
            .filter(|(id, p)| prefs_before.get(id) != Some(p))
            .collect();
        for (id, prefs) in changed {
            if let Some(p) = self.ws.page_mut(&id).and_then(|p| p.project.as_mut()) {
                p.launch = Some(prefs);
                self.dirty_since.get_or_insert_with(Instant::now);
            }
        }
        start.dedup();
        for id in start {
            let d = self.orch.drafts.get(&id).cloned().unwrap();
            let title = self
                .ws
                .page(&id)
                .map(|p| p.display_title())
                .unwrap_or_default();
            let taken: f32 = self.orch.runs.iter().map(|r| r.limit).sum();
            // The global limit already shared out (by the projects started just before, in
            // a selection): this one doesn't start on a crumb of it.
            let Some(limit) = share_at_start(d.limit, global, taken) else {
                msg = Some(super::notices::Notice::Error(crate::trf!(
                    "{title} : la limite globale ({global:.0} %) est déjà prise par les autres sessions.",
                    "{title}: the global limit ({global:.0} %) is already taken by the other sessions."
                )));
                continue;
            };
            let endless = if d.endless { Some(d.end.clone()) } else { None };
            match self.launch_run(ctx, &id, &d.mission, limit, d.commits, endless) {
                Ok(()) => {
                    if let Some(x) = self.orch.drafts.get_mut(&id) {
                        x.pick = false;
                    }
                    msg = Some(super::notices::Notice::Ack(crate::trf!(
                        "Session autonome lancée sur {title}.",
                        "Autonomous session started on {title}."
                    )));
                }
                Err(e) => msg = Some(super::notices::Notice::Error(format!("{title} : {e}."))),
            }
        }
        msg
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
        let mut run_act: Option<(String, &'static str)> = None;
        let confirm = self.copy_states();
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
                let (now, offset) = (crate::model::now_ms(), auto::local_offset_ms());
                let mut group = "";
                for i in shown {
                    let m = &self.ws.inbox[i];
                    // A heading per day, newest first.
                    let g = day_group(m.at, now, offset);
                    if g != group {
                        if !group.is_empty() {
                            ui.add_space(10.0);
                        }
                        group = g;
                        ui.label(RichText::new(g.to_uppercase()).font(semibold(11.5)).color(t.tertiary));
                        ui.add_space(6.0);
                    }
                    let to_answer = m.kind == "question" && m.answer.trim().is_empty();
                    let (accent, kind) = match m.kind.as_str() {
                        "question" => (Color32::from_rgb(144, 101, 176), crate::tr!("Question", "Question")),
                        "fin" => (super::projects::GREEN, crate::tr!("Session", "Session")),
                        "rapport" => (super::projects::BLUE, crate::tr!("Rapport", "Report")),
                        "erreur" => (Color32::from_rgb(224, 62, 62), crate::tr!("Erreur", "Error")),
                        _ => (t.tertiary, crate::tr!("Info", "Info")),
                    };
                    let project = (!m.project.is_empty()).then(|| self.ws.page(&m.project).map(|p| p.display_title())).flatten();
                    // The end of a session: its work, still in its own copy, to merge or discard.
                    let run = run_of_message(m, &self.ws.runs, |id| self.orch.runs.iter().any(|l| l.id == id) || self.orch.to_main.iter().any(|x| x == id));
                    let bg = super::projects::hover_begin(ui);
                    let card = egui::Frame::new()
                        .fill(if m.read { Color32::TRANSPARENT } else { t.card })
                        // A question still waiting for its answer stands out.
                        .stroke(egui::Stroke::new(1.0, if to_answer { accent.gamma_multiply(0.55) } else { t.border }))
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin { left: 16, right: 10, top: 10, bottom: 10 })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let head = ui.horizontal_top(|ui| {
                                let text = ui.vertical(|ui| {
                                    ui.set_width(ui.available_width() - 30.0);
                                    ui.horizontal(|ui| {
                                        kind_badge(ui, kind, accent);
                                        // A question says at once whether it still waits for the person.
                                        if m.kind == "question" {
                                            if to_answer {
                                                kind_badge(ui, crate::tr!("À répondre", "To answer"), super::projects::ORANGE);
                                            } else {
                                                kind_badge(ui, crate::tr!("Répondue", "Answered"), super::projects::GREEN);
                                            }
                                        }
                                        if let Some(p) = &project {
                                            // The project's own colour, as on the Orchestration page.
                                            ui.add_space(2.0);
                                            let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 14.0), egui::Sense::hover());
                                            ui.painter().circle_filled(dot.center(), 3.5, project_color(p));
                                            ui.label(RichText::new(p).size(12.0).color(t.secondary));
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
                                    let body = plain_text(&m.text);
                                    if !body.is_empty() {
                                        ui.add_space(2.0);
                                        // Long texts (session summaries) folded to a few lines.
                                        let more_id = egui::Id::unique(("inbox-more", &m.id));
                                        let all = ui.data(|d| d.get_temp::<bool>(more_id)).unwrap_or(false);
                                        let mut job = egui::text::LayoutJob::single_section(body, egui::TextFormat::simple(FontId::proportional(13.0), t.secondary));
                                        job.wrap.max_width = ui.available_width();
                                        if !all {
                                            job.wrap.max_rows = TEXT_LINES;
                                        }
                                        let galley = ui.fonts_mut(|f| f.layout_job(job));
                                        let cut = galley.elided;
                                        ui.label(galley);
                                        if (cut || all) && super::projects::action_link(ui, if all { crate::tr!("Réduire", "Show less") } else { crate::tr!("Afficher tout", "Show all") }).clicked() {
                                            ui.data_mut(|d| d.insert_temp(more_id, !all));
                                        }
                                    }
                                });
                                let x = super::icons::delete_cross(ui, 22.0, t.tertiary, t.text, t.selected, crate::tr!("Supprimer", "Delete"));
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
                                if let Some(text) = answer_box(ui, m, accent, &t) {
                                    answer = Some((i, text));
                                }
                            }
                            if let Some(r) = run {
                                let asking = confirm.iter().find(|(id, _)| *id == r.id).map(|(_, a)| *a);
                                if let Some(a) = run_actions(ui, r, asking, &t) {
                                    run_act = Some((r.id.clone(), a));
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
            self.ack(crate::trf!(
                "{n} messages supprimés.",
                "{n} messages deleted."
            ));
            return;
        }
        if all_read {
            for m in self.ws.inbox.iter_mut() {
                m.read = true;
            }
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        if let Some((i, text)) = answer {
            self.save_answer(i, text);
        }
        if let Some((id, a)) = run_act {
            self.run_action(&ui.ctx().clone(), &id, a);
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

    /// The finished sessions whose work is still in their copy while their end message is
    /// gone from the inbox (its links went with it): listed here, with the same links.
    /// Shown only when there is one; returns whether it was.
    fn orphans_card(&mut self, ui: &mut egui::Ui, t: &Tone, actions: &mut Vec<Action>) -> bool {
        let busy = |id: &str| {
            self.orch.runs.iter().any(|l| l.id == id) || self.orch.to_main.iter().any(|x| x == id)
        };
        let list = orphan_copies(&self.ws.runs, &self.ws.inbox, busy, |w| {
            std::path::Path::new(w).is_dir()
        });
        if list.is_empty() {
            return false;
        }
        let confirm = self.copy_states();
        let mut act: Option<(String, &'static str)> = None;
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(
                ui,
                crate::tr!("COPIES À TRAITER", "COPIES TO DEAL WITH"),
                &list.len().to_string(),
                t,
            );
            for &i in &list {
                let r = &self.ws.runs[i];
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(&r.title).font(semibold(14.0)).color(t.text));
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            auto::local_day(r.started),
                            auto::hm(r.started)
                        ))
                        .size(12.5)
                        .color(t.tertiary),
                    );
                    if let Some(p) = &r.report
                        && super::projects::action_link(ui, crate::tr!("Rapport", "Report"))
                            .clicked()
                    {
                        act = Some((p.clone(), "report"));
                    }
                });
                let asking = confirm.iter().find(|(id, _)| *id == r.id).map(|(_, a)| *a);
                if let Some(a) = run_actions(ui, r, asking, t) {
                    act = Some((r.id.clone(), a));
                }
                ui.add_space(6.0);
            }
        });
        match act {
            Some((page, "report")) => actions.push(Action::Open(page)),
            Some((id, a)) => self.run_action(&ui.ctx().clone(), &id, a),
            None => {}
        }
        true
    }

    /// What the links under a session's copy show: being dealt with, or the second click
    /// awaited.
    fn copy_states(&self) -> Vec<(String, &'static str)> {
        let mut v: Vec<(String, &'static str)> = self
            .orch
            .copy_jobs
            .iter()
            .map(|j| (j.id.clone(), if j.merge { "merging" } else { "discarding" }))
            .collect();
        v.extend(self.orch.confirm.clone());
        v
    }

    /// The merges and discards finished in the background: their outcome recorded.
    fn copy_jobs_tick(&mut self) {
        let mut finished = vec![];
        self.orch.copy_jobs.retain(|j| match j.done.try_recv() {
            Ok(res) => {
                finished.push((
                    j.id.clone(),
                    if j.park { 2 } else { j.merge as u8 },
                    j.title.clone(),
                    res,
                ));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                finished.push((
                    j.id.clone(),
                    if j.park { 2 } else { j.merge as u8 },
                    j.title.clone(),
                    Err(crate::tr!("interrompu", "interrupted").into()),
                ));
                false
            }
        });
        for (id, merge, title, res) in finished {
            match res {
                Ok(m) => {
                    if let Some(x) = self.ws.runs.iter_mut().find(|x| x.id == id) {
                        x.closed = match merge {
                            2 => crate::tr!("gardé sur dev", "kept on dev").into(),
                            1 => crate::tr!("fusionné", "merged").into(),
                            _ => crate::tr!("abandonné", "discarded").into(),
                        };
                    }
                    self.dirty_since.get_or_insert_with(Instant::now);
                    self.detect_changes();
                    self.info(format!("{title} : {m}"));
                }
                Err(e) => self.fail(format!("{title} : {e}")),
            }
        }
    }

    /// A click on a finished session's copy: ask first, then merge or discard it on its
    /// own thread, the interface going on meanwhile.
    fn run_action(&mut self, ctx: &egui::Context, id: &str, a: &'static str) {
        match a {
            "ask-merge" => self.orch.confirm = Some((id.to_string(), "merge")),
            "ask-discard" => self.orch.confirm = Some((id.to_string(), "discard")),
            "cancel" => self.orch.confirm = None,
            "open" => {
                if let Some(r) = self.ws.runs.iter().find(|r| r.id == id) {
                    let _ = crate::opener::open_path(std::path::Path::new(&r.worktree));
                }
            }
            "merge" | "discard" => {
                self.orch.confirm = None;
                let Some(rec) = self.ws.runs.iter().find(|r| r.id == id).cloned() else {
                    return;
                };
                let title = rec.title.clone();
                let op = if a == "merge" {
                    CopyOp::Merge
                } else {
                    CopyOp::Discard
                };
                // One git operation at a time per repository: a second would find it locked.
                if !self.start_copy_job(ctx, rec, op) {
                    self.fail(crate::trf!("{} : une autre copie de ce projet est en cours de traitement, réessayez dans un instant.", "{}: another copy of this project is being dealt with, try again in a moment.", title));
                }
            }
            _ => {}
        }
    }
}

/// The session an end-of-session message speaks of (by its report page), once it is over
/// and not `busy` (running again, or about to be merged into main by itself).
fn run_of_message<'a>(
    m: &crate::model::InboxMsg,
    runs: &'a [crate::model::RunRecord],
    busy: impl Fn(&str) -> bool,
) -> Option<&'a crate::model::RunRecord> {
    if m.kind != "fin" {
        return None;
    }
    let page = m.page.as_ref().filter(|p| !p.is_empty())?;
    runs.iter()
        .find(|r| r.report.as_ref() == Some(page))
        .filter(|r| !r.outcome.is_empty() && !busy(&r.id))
}

/// The finished sessions (indices in `runs`) whose work waits in a copy still on disk,
/// not merged nor discarded, not `busy`, and whose end message is no longer in the inbox.
fn orphan_copies(
    runs: &[crate::model::RunRecord],
    inbox: &[crate::model::InboxMsg],
    busy: impl Fn(&str) -> bool,
    copy_exists: impl Fn(&str) -> bool,
) -> Vec<usize> {
    runs.iter()
        .enumerate()
        .filter(|(_, r)| !r.outcome.is_empty() && r.closed.is_empty() && !busy(&r.id))
        .filter(|(_, r)| {
            !inbox
                .iter()
                .any(|m| run_of_message(m, std::slice::from_ref(*r), |_| false).is_some())
        })
        .filter(|(_, r)| copy_exists(&r.worktree))
        .map(|(i, _)| i)
        .collect()
}

/// Under the end of a session: what became of its copy, or the links to deal with it
/// (merge into the project's folder, discard, open), each risky one asked twice.
fn run_actions(
    ui: &mut egui::Ui,
    r: &crate::model::RunRecord,
    asking: Option<&'static str>,
    t: &Tone,
) -> Option<&'static str> {
    if auto::in_place(r) {
        ui.add_space(4.0);
        ui.label(
            RichText::new(crate::trf!(
                "Pas de copie : {}.",
                "No copy: {}.",
                auto::in_place_note()
            ))
            .size(12.5)
            .color(t.tertiary),
        );
        return None;
    }
    if !r.closed.is_empty() {
        ui.add_space(4.0);
        ui.label(
            RichText::new(crate::trf!("Copie : {}", "Copy: {}", r.closed))
                .size(12.5)
                .color(t.tertiary),
        );
        return None;
    }
    // Being dealt with in the background (the copy may already be half gone).
    if let Some(a @ ("merging" | "discarding")) = asking {
        let what = if a == "merging" {
            crate::tr!("Fusion en cours…", "Merging…")
        } else {
            crate::tr!("Suppression de la copie en cours…", "Deleting the copy…")
        };
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0));
            ui.label(RichText::new(what).size(12.5).color(t.tertiary));
        });
        return None;
    }
    if !std::path::Path::new(&r.worktree).is_dir() {
        return None;
    }
    let mut act = None;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        match asking {
            Some(a) => {
                let what = if a == "merge" {
                    crate::tr!(
                        "Fusionner ce travail dans votre branche actuelle ?",
                        "Merge this work into your current branch?"
                    )
                } else {
                    crate::tr!(
                        "Supprimer la copie et sa branche, définitivement ?",
                        "Delete the copy and its branch, for good?"
                    )
                };
                ui.label(
                    RichText::new(what)
                        .size(12.5)
                        .color(super::projects::ORANGE),
                );
                if super::projects::action_link(ui, crate::tr!("Oui", "Yes")).clicked() {
                    act = Some(a);
                }
                if super::projects::action_link(ui, crate::tr!("Non", "No")).clicked() {
                    act = Some("cancel");
                }
            }
            None => {
                ui.label(
                    RichText::new(crate::tr!(
                        "Le travail est resté dans sa copie :",
                        "The work stayed in its copy:"
                    ))
                    .size(12.5)
                    .color(t.tertiary),
                );
                if super::projects::action_link(ui, crate::tr!("Fusionner", "Merge"))
                    .on_hover_text(crate::tr!(
                        "Intègre ce travail dans la branche ouverte dans le dossier du projet",
                        "Brings this work into the branch open in the project's folder"
                    ))
                    .clicked()
                {
                    act = Some("ask-merge");
                }
                if super::projects::action_link(ui, crate::tr!("Abandonner", "Discard"))
                    .on_hover_text(crate::tr!(
                        "Supprime la copie et sa branche, sans rien changer à votre dossier",
                        "Deletes the copy and its branch, nothing changes in your folder"
                    ))
                    .clicked()
                {
                    act = Some("ask-discard");
                }
                if super::projects::action_link(ui, crate::tr!("Ouvrir la copie", "Open the copy"))
                    .clicked()
                {
                    act = Some("open");
                }
            }
        }
    });
    act
}

/// A session's work on the steps since it started.
#[derive(Debug, PartialEq)]
struct StepsSince {
    /// Titles of the steps done since the start (as they read now).
    done: Vec<String>,
    /// Steps an AI added since the start and not done yet. Those the person added in the
    /// meantime don't count: adding steps while a session runs never stops it.
    added_open: usize,
}

/// Compared by step id, not by title: a step renamed during the session stays the same
/// step (it used to count as new, and as done twice), and two steps with one title stay
/// two. `before`: (step id, done) at the start.
fn steps_since(before: &[(String, bool)], steps: &[crate::project::Step]) -> StepsSince {
    let done = |s: &crate::project::Step| s.status == crate::project::StepStatus::Done;
    let at_start =
        |s: &crate::project::Step| before.iter().find(|(id, _)| *id == s.id).map(|(_, d)| *d);
    StepsSince {
        done: steps
            .iter()
            .filter(|s| done(s) && at_start(s) != Some(true))
            .map(|s| s.title.clone())
            .collect(),
        added_open: steps
            .iter()
            .filter(|s| !done(s) && at_start(s).is_none() && !s.by_person())
            .count(),
    }
}

/// The step a session is on: « doing » and reserved by this computer (`me`), or not
/// reserved at all; the most recently changed when there are several.
fn current_step<'a>(steps: &'a [crate::project::Step], me: &str) -> Option<&'a str> {
    steps
        .iter()
        .filter(|s| {
            s.status == crate::project::StepStatus::Doing
                && (s.held_by.is_empty() || s.held_by == me)
        })
        .max_by_key(|s| s.updated)
        .map(|s| s.title.as_str())
}

/// Open steps the person added (typed, accepted, received) since the turn began: `at_turn`
/// holds the step ids then. Their titles, in the project's order.
fn fresh_steps(at_turn: &[String], steps: &[crate::project::Step]) -> Vec<String> {
    steps
        .iter()
        .filter(|s| {
            s.status != crate::project::StepStatus::Done
                && s.by_person()
                && !at_turn.contains(&s.id)
        })
        .map(|s| s.title.clone())
        .collect()
}

/// Put before the next turn's prompt when the person added steps during the turn, so the
/// AI takes them without leaving its endless mode. Empty when there are none.
fn new_steps_note(fresh: &[String]) -> String {
    if fresh.is_empty() {
        return String::new();
    }
    let list: Vec<String> = fresh
        .iter()
        .take(5)
        .map(|t| format!("« {} »", crate::richtext::one_line(t, 100)))
        .collect();
    crate::trf!(
        "La personne vient d'ajouter {} étape(s) : {}. Prends-les en compte. ",
        "The person just added {} step(s): {}. Take them into account. ",
        fresh.len(),
        list.join(", ")
    )
}

/// What a session's turn ended on, to choose whether it stops.
struct TurnEnd {
    /// A stop already asked (by the person, or a limit seen during the turn).
    asked: Option<String>,
    too_long: bool,
    /// Endless mode: the end condition reached, if any.
    endless_end: Option<String>,
    signal: Signal,
    endless: bool,
    /// The session's use of the 5-hour window, and its budget.
    pct: f32,
    limit: f32,
    /// The 5-hour window, and the global limit.
    five: Option<f32>,
    global: f32,
    /// The week, the share of it the AIs may use, and the most one turn took from it.
    week: Option<f32>,
    week_limit: f32,
    week_turn: f32,
    measured: bool,
    turns: u32,
    max_turns: u32,
}

/// Why a session stops after a turn.
#[derive(Debug, Clone, PartialEq)]
enum Stop {
    Asked(String),
    TooLong,
    EndlessEnd(String),
    MissionDone,
    Blocked,
    Budget(f32),
    GlobalLimit(f32),
    /// The week's reserve (in %) is near.
    WeekReserve(f32),
    /// Claude Code said the subscription's own limit is reached.
    SubscriptionLimit,
    Unmeasured,
    Turns(u32),
}

/// The quota a stop waits for, to start again once it resets.
#[derive(Debug, PartialEq)]
enum Quota {
    None,
    FiveHours,
    Week,
}

impl Stop {
    fn quota(&self) -> Quota {
        match self {
            Stop::GlobalLimit(_) => Quota::FiveHours,
            Stop::WeekReserve(_) => Quota::Week,
            _ => Quota::None,
        }
    }

    fn text(&self) -> String {
        match self {
            Stop::Asked(s) | Stop::EndlessEnd(s) => s.clone(),
            Stop::TooLong => {
                crate::tr!("un tour a duré trop longtemps", "a turn took too long").into()
            }
            Stop::MissionDone => crate::tr!("mission terminée", "mission complete").into(),
            Stop::Blocked => crate::tr!("l'IA est bloquée", "the AI is blocked").into(),
            Stop::Budget(l) => crate::trf!(
                "limite du projet atteinte ({l:.0} %)",
                "project limit reached ({l:.0} %)"
            ),
            Stop::GlobalLimit(g) => crate::trf!(
                "limite globale atteinte ({g:.0} %)",
                "global limit reached ({g:.0} %)"
            ),
            Stop::WeekReserve(r) => crate::trf!(
                "pause : la réserve minimum de la semaine ({r:.0} %) approche",
                "paused: the week's minimum reserve ({r:.0} %) is near"
            ),
            Stop::SubscriptionLimit => crate::tr!(
                "limite de l'abonnement Claude atteinte",
                "Claude subscription limit reached"
            )
            .into(),
            Stop::Unmeasured => crate::tr!(
                "utilisation de l'abonnement impossible à mesurer",
                "subscription use can't be measured"
            )
            .into(),
            Stop::Turns(n) => crate::trf!("{n} tours atteints", "{n} turns reached"),
        }
    }
}

/// Whether a session stops after a turn, and why: the first reason in this order. None:
/// it goes on.
fn stop_reason(t: &TurnEnd) -> Option<Stop> {
    Some(if let Some(s) = &t.asked {
        Stop::Asked(s.clone())
    } else if t.too_long {
        Stop::TooLong
    } else if let Some(why) = &t.endless_end {
        Stop::EndlessEnd(why.clone())
    } else if t.signal == Signal::Done && !t.endless {
        // Endless mode has its own end conditions instead of [[FINI]].
        Stop::MissionDone
    } else if t.signal == Signal::Blocked {
        Stop::Blocked
    } else if t.pct >= t.limit {
        Stop::Budget(t.limit)
    } else if t.five.is_some_and(|f| f >= t.global) {
        Stop::GlobalLimit(t.global)
    } else if t
        .week
        .is_some_and(|w| crate::usage::near_reserve(w, t.week_limit, t.week_turn))
    {
        // Paused before the next turn bites into the reserve; starts again once the week resets.
        Stop::WeekReserve(100.0 - t.week_limit)
    } else if !t.measured && t.turns >= 2 {
        Stop::Unmeasured
    } else if !t.endless && t.turns >= t.max_turns {
        Stop::Turns(t.max_turns)
    } else {
        return None;
    })
}

impl Figures {
    fn of(u: &crate::usage::Usage) -> Self {
        Figures {
            five: u.five().map(|w| w.pct),
            week: u.week().map(|w| w.pct),
            five_reset: u.five().and_then(|w| w.resets_at),
            week_reset: u.week().and_then(|w| w.resets_at),
        }
    }
}

/// A new reading of `agent`'s 5-hour window (taken at `at`): its rise since `last` is shared
/// between that agent's running sessions only, by the tokens each used since then; the
/// sessions of other agents, on other subscriptions, get nothing of it.
fn apply_reading(
    runs: &mut [LiveRun],
    agent: Agent,
    last: &mut Option<(u64, f32, Option<u64>)>,
    at: u64,
    five: &crate::usage::Window,
) {
    let prev = *last;
    if prev.is_some_and(|(a, _, _)| a == at) {
        return;
    }
    *last = Some((at, five.pct, five.resets_at));
    let rise = window_rise(prev.map(|(_, p, r)| (p, r)), five.pct, five.resets_at);
    let sessions: Vec<(Agent, u64)> = runs
        .iter()
        .map(|r| (r.agent, r.reader.tokens.saturating_sub(r.tokens_counted)))
        .collect();
    for (r, s) in runs.iter_mut().zip(agent_shares(&sessions, agent, rise)) {
        let Some(s) = s else { continue };
        if at >= r.started_wall {
            r.measured = true;
        }
        r.pct += s;
        r.tokens_counted = r.reader.tokens;
    }
}

/// Each session's share of a rise of `agent`'s window (`sessions`: each one's agent and the
/// tokens it used since the last reading): None for the sessions of other agents.
fn agent_shares(sessions: &[(Agent, u64)], agent: Agent, rise: f32) -> Vec<Option<f32>> {
    let deltas: Vec<u64> = sessions
        .iter()
        .filter(|(a, _)| *a == agent)
        .map(|(_, d)| *d)
        .collect();
    let mut shares = split_rise(rise, &deltas).into_iter();
    sessions
        .iter()
        .map(|(a, _)| if *a == agent { shares.next() } else { None })
        .collect()
}

/// The last `n` lines of a terminal screen that hold something.
fn last_lines(screen: &str, n: usize) -> String {
    let lines: Vec<&str> = screen.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// How much the 5-hour window rose since the last reading (`prev`: its figure and reset
/// time then). A new window (another reset time) counts from zero; the first reading
/// counts for nothing: what was used before it isn't anyone's.
fn window_rise(prev: Option<(f32, Option<u64>)>, pct: f32, resets_at: Option<u64>) -> f32 {
    match (prev, resets_at) {
        (Some((p, Some(r))), Some(now)) if r.abs_diff(now) < 120_000 => (pct - p).max(0.0),
        (Some((p, None)), None) => (pct - p).max(0.0),
        (Some(_), _) => pct,
        (None, _) => 0.0,
    }
}

/// A rise of the 5-hour window shared between the running sessions, by the tokens each
/// used since the last reading (evenly when none is known). What each session shows
/// (`LiveRun::pct`) is the sum of its shares since it started: the window's own figure
/// at its start is never in it.
fn split_rise(rise: f32, tokens: &[u64]) -> Vec<f32> {
    let total: u64 = tokens.iter().sum();
    let n = tokens.len().max(1) as f32;
    tokens
        .iter()
        .map(|&d| {
            if total > 0 {
                rise * d as f32 / total as f32
            } else {
                rise / n
            }
        })
        .collect()
}

/// In a running session's card: the use of its share of the 5-hour window, as a bar and
/// its figures.
/// A running session on the 5-hour window (all in %, 0..100): what was used before it or by
/// others (`0..before`), its own use (`before..window`), and where it will stop (`stop`):
/// on its budget, or on the global limit if that comes first.
#[derive(Debug, PartialEq)]
struct WindowBar {
    before: f32,
    window: f32,
    stop: f32,
}

fn window_bar(window: f32, pct: f32, limit: f32, global: f32) -> WindowBar {
    let window = window.clamp(0.0, 100.0);
    let before = (window - pct.max(0.0)).max(0.0);
    WindowBar {
        before,
        window,
        stop: (before + limit).min(global).clamp(0.0, 100.0),
    }
}

/// Never wider than its card: the bar takes what the text leaves (the text was once given
/// a fixed 150 px it didn't fit in, and the overflow widened every card below; see the
/// tests). Too narrow for both side by side, the text goes under the bar.
/// With the 5-hour window's figure (`window`), the bar is that window: the use before the
/// session, the session's own, and a tick where it stops. Without it, the session's
/// share of its budget only.
fn share_row(
    ui: &mut egui::Ui,
    id: &str,
    pct: f32,
    limit: f32,
    window: Option<f32>,
    global: f32,
    t: &Tone,
) {
    let text = match window {
        Some(w) => crate::trf!(
            "fenêtre de 5 h : {w:.0} % · cette session : {} % (budget {limit:.0} %)",
            "5-hour window: {w:.0} % · this session: {} % (budget {limit:.0} %)",
            crate::i18n::dec1(pct)
        ),
        None => crate::trf!(
            "{} % utilisés sur un budget de {:.0} %",
            "{} % used of a {:.0} % budget",
            crate::i18n::dec1(pct),
            limit
        ),
    };
    let font = FontId::proportional(12.5);
    let text_w = ui
        .fonts_mut(|f| f.layout_no_wrap(text.clone(), font.clone(), t.secondary))
        .size()
        .x;
    let indent = 20.0;
    let gap = ui.spacing().item_spacing.x;
    let room = ui.available_width() - indent;
    let side_by_side = room - gap - text_w >= 60.0;
    let bar_w = if side_by_side {
        room - gap - text_w
    } else {
        room.max(0.0)
    };
    let draw_bar = |ui: &mut egui::Ui| {
        let (bar, _) = ui.allocate_exact_size(egui::vec2(bar_w, 8.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(bar, 4.0, t.fill);
        let x = |v: f32| bar.left() + bar.width() * (v / 100.0).clamp(0.0, 1.0);
        match window {
            Some(w) => {
                let b = window_bar(w, pct, limit, global);
                let end = super::projects::smooth(ui, ("run-window", id), b.window);
                let near = b.window >= b.stop - (b.stop - b.before) * 0.15;
                let p = ui.painter();
                // Before the session (or by others): muted; the session's own: its colour.
                p.rect_filled(
                    egui::Rect::from_min_max(bar.min, egui::pos2(x(b.before), bar.bottom())),
                    4.0,
                    t.tertiary.gamma_multiply(0.45),
                );
                p.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x(b.before), bar.top()),
                        egui::pos2(x(end.max(b.before)), bar.bottom()),
                    ),
                    0.0,
                    if near {
                        super::projects::ORANGE
                    } else {
                        super::projects::GREEN
                    },
                );
                p.vline(
                    x(b.stop),
                    egui::Rangef::new(bar.top() - 3.0, bar.bottom() + 3.0),
                    egui::Stroke::new(2.0, super::projects::ORANGE),
                );
            }
            None => {
                let frac = super::projects::smooth(
                    ui,
                    ("run-pct", id),
                    (pct / limit.max(0.1)).clamp(0.0, 1.0),
                );
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac, 8.0)),
                    4.0,
                    if frac > 0.85 {
                        super::projects::ORANGE
                    } else {
                        super::projects::GREEN
                    },
                );
            }
        }
    };
    let label = |ui: &mut egui::Ui| {
        ui.add(egui::Label::new(RichText::new(text.as_str()).font(font.clone()).color(t.secondary)).wrap()).on_hover_text(crate::tr!(
            "La barre est la fenêtre de 5 h de l'abonnement : en gris ce qui était utilisé avant cette session (ou par d'autres), en vert ce qu'elle a utilisé depuis son lancement. Le trait orange : où elle s'arrête, sur son budget ou sur la limite globale si elle vient avant. Les sessions précédentes du projet ne comptent pas dans son budget.",
            "The bar is the subscription's 5-hour window: in grey what was used before this session (or by others), in green what it used since it started. The orange tick: where it stops, on its budget or on the global limit if that comes first. The project's earlier sessions don't count in its budget."
        ));
    };
    ui.horizontal(|ui| {
        ui.add_space(indent);
        if side_by_side {
            draw_bar(ui);
            label(ui);
        } else {
            ui.vertical(|ui| {
                draw_bar(ui);
                label(ui);
            });
        }
    });
}

/// Most questions shown on the Orchestration page.
const ORCH_QUESTIONS: usize = 5;
/// How long an answered question stays on the Orchestration page.
const ORCH_ANSWERED_MS: u64 = 3 * 86_400_000;

/// The inbox questions to show (indexes in `inbox`): first those waiting for an answer,
/// whatever their age, then those answered less than `answered_for` ms ago; newest first in
/// each group, `ORCH_QUESTIONS` at most.
pub(super) fn orch_questions(
    inbox: &[crate::model::InboxMsg],
    now: u64,
    answered_for: u64,
) -> Vec<usize> {
    let newest = |a: &usize, b: &usize| inbox[*b].at.cmp(&inbox[*a].at);
    let q = |i: &usize| inbox[*i].kind == "question";
    let mut open: Vec<usize> = (0..inbox.len())
        .filter(q)
        .filter(|&i| inbox[i].answer.trim().is_empty())
        .collect();
    let mut answered: Vec<usize> = (0..inbox.len())
        .filter(q)
        .filter(|&i| {
            !inbox[i].answer.trim().is_empty()
                && inbox[i]
                    .answered_at
                    .max(inbox[i].at)
                    .saturating_add(answered_for)
                    > now
        })
        .collect();
    open.sort_by(newest);
    answered.sort_by(|a, b| inbox[*b].answered_at.cmp(&inbox[*a].answered_at));
    open.into_iter()
        .chain(answered)
        .take(ORCH_QUESTIONS)
        .collect()
}

/// The count beside a questions card's title: how many wait, or how many are shown.
pub(super) fn questions_count(msgs: &[&crate::model::InboxMsg]) -> String {
    let waiting = msgs.iter().filter(|m| m.answer.trim().is_empty()).count();
    if waiting > 0 {
        crate::trf!("{waiting} en attente", "{waiting} waiting")
    } else {
        msgs.len().to_string()
    }
}

/// AI questions one under the other (Orchestration page, project page): answered or not,
/// project (`project_of`, None to leave it out) and date, the question on three lines at
/// most, then the answer or a box to type it. Returns (message id, answer) when one is sent.
pub(super) fn question_list(
    ui: &mut egui::Ui,
    msgs: &[&crate::model::InboxMsg],
    project_of: impl Fn(&crate::model::InboxMsg) -> Option<String>,
    t: &Tone,
) -> Option<(String, String)> {
    let accent = Color32::from_rgb(144, 101, 176);
    let mut out = None;
    for (k, m) in msgs.iter().enumerate() {
        if k > 0 {
            super::projects::card_rule(ui, t);
        }
        ui.horizontal(|ui| {
            if m.answer.trim().is_empty() {
                kind_badge(
                    ui,
                    crate::tr!("À répondre", "To answer"),
                    super::projects::ORANGE,
                );
            } else {
                kind_badge(
                    ui,
                    crate::tr!("Répondue", "Answered"),
                    super::projects::GREEN,
                );
            }
            if let Some(p) = project_of(m) {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 14.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(dot.center(), 3.5, project_color(&p));
                ui.label(RichText::new(p).size(12.0).color(t.secondary));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(super::chrome::relative_time(m.at))
                        .size(12.0)
                        .color(t.tertiary),
                );
            });
        });
        // The question itself, a few lines at most: the inbox has it whole.
        let q = plain_text(&m.text);
        let q = if q.is_empty() { m.title.clone() } else { q };
        let mut job = egui::text::LayoutJob::single_section(
            q,
            egui::TextFormat::simple(FontId::proportional(13.5), t.text),
        );
        job.wrap.max_width = ui.available_width();
        job.wrap.max_rows = 3;
        ui.label(ui.fonts_mut(|f| f.layout_job(job)));
        ui.add_space(4.0);
        if let Some(text) = answer_box(ui, m, accent, t) {
            out = Some((m.id.clone(), text));
        }
    }
    out
}

/// The group of an inbox message, by its local day: today, yesterday, this week, older.
/// `offset`: the local time's offset from UTC (ms).
fn day_group(at: u64, now: u64, offset: i64) -> &'static str {
    let day = |ms: u64| (ms as i64 + offset).div_euclid(86_400_000);
    match day(now) - day(at) {
        i64::MIN..=0 => crate::tr!("Aujourd'hui", "Today"),
        1 => crate::tr!("Hier", "Yesterday"),
        2..=6 => crate::tr!("Cette semaine", "This week"),
        _ => crate::tr!("Plus ancien", "Older"),
    }
}

/// An inbox message's text for reading: Markdown headings and bold marks dropped (session
/// summaries are written in Markdown), blank lines squeezed.
fn plain_text(text: &str) -> String {
    let mut out: Vec<String> = vec![];
    for line in text.lines() {
        let l = line
            .trim_end()
            .trim_start_matches('#')
            .trim_start()
            .replace("**", "");
        if l.is_empty() && out.last().is_none_or(|p| p.is_empty()) {
            continue;
        }
        out.push(l);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// Lines of a message's text shown before « Afficher tout ».
const TEXT_LINES: usize = 4;

/// The kind of an inbox message, as a small tinted pill.
fn kind_badge(ui: &mut egui::Ui, text: &str, accent: Color32) {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), semibold(11.0), accent);
    let (rect, _) = ui.allocate_exact_size(g.size() + egui::vec2(14.0, 6.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 6.0, accent.gamma_multiply(0.14));
    ui.painter()
        .galley(rect.center() - g.size() / 2.0, g, accent);
}

/// A filter of the inbox: its name and how many messages it holds.
fn filter_chip(ui: &mut egui::Ui, text: &str, count: usize, on: bool, t: &Tone) -> egui::Response {
    let label = if count > 0 {
        format!("{text}  {count}")
    } else {
        text.to_string()
    };
    let color = if on { t.text } else { t.secondary };
    let g = ui.painter().layout_no_wrap(
        label,
        if on {
            semibold(12.5)
        } else {
            FontId::proportional(12.5)
        },
        color,
    );
    let (rect, resp) =
        ui.allocate_exact_size(g.size() + egui::vec2(20.0, 10.0), egui::Sense::click());
    let a = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.14);
    let fill = if on {
        t.selected
    } else {
        t.hover.gamma_multiply(a)
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    if on {
        ui.painter().rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter()
        .galley(rect.center() - g.size() / 2.0, g, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Under a question of the inbox: the answer given, in its own tinted box (when, and where
/// it goes), or a box to type one.
/// Returns the answer when the person sends it (button, or Cmd/Ctrl+Enter).
fn answer_box(
    ui: &mut egui::Ui,
    m: &crate::model::InboxMsg,
    accent: Color32,
    t: &Tone,
) -> Option<String> {
    let draft_id = egui::Id::unique(("inbox-draft", &m.id));
    let edit_id = egui::Id::unique(("inbox-edit", &m.id));
    let editing = ui.data(|d| d.get_temp::<bool>(edit_id)).unwrap_or(false);
    let mut out = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() - 30.0);
            if !m.answer.trim().is_empty() && !editing {
                let frame = egui::Frame::new()
                    .fill(accent.gamma_multiply(0.07))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 10,
                        top: 8,
                        bottom: 9,
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(crate::tr!("VOTRE RÉPONSE", "YOUR ANSWER"))
                                    .font(semibold(11.0))
                                    .color(accent),
                            );
                            if m.answered_at > 0 {
                                ui.label(
                                    RichText::new(super::chrome::relative_time(m.answered_at))
                                        .size(11.5)
                                        .color(t.tertiary),
                                );
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if super::projects::action_link(
                                        ui,
                                        crate::tr!("Modifier", "Edit"),
                                    )
                                    .clicked()
                                    {
                                        ui.data_mut(|d| {
                                            d.insert_temp(draft_id, m.answer.clone());
                                            d.insert_temp(edit_id, true);
                                        });
                                    }
                                },
                            );
                        });
                        ui.add_space(2.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(m.answer.trim()).size(13.5).color(t.text),
                            )
                            .wrap(),
                        );
                    });
                // A bar on the left, like a quote: the person's words, not the AI's.
                let r = frame.response.rect;
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(
                        r.left_top() + egui::vec2(0.0, 6.0),
                        egui::pos2(r.left() + 2.5, r.bottom() - 6.0),
                    ),
                    1.5,
                    accent.gamma_multiply(0.7),
                );
                ui.add_space(3.0);
                if m.answer_seen() {
                    // An AI received it: the person knows it was taken in.
                    let seen = crate::trf!(
                        "✓ Prise en compte par l'IA {}",
                        "✓ Taken in by the AI {}",
                        super::chrome::relative_time(m.seen_at)
                    );
                    ui.label(RichText::new(seen).size(11.5).color(super::projects::GREEN));
                } else {
                    ui.label(
                        RichText::new(answer_note(m.project.is_empty()))
                            .size(11.5)
                            .color(t.tertiary),
                    );
                }
                return;
            }
            let mut draft = ui
                .data(|d| d.get_temp::<String>(draft_id))
                .unwrap_or_default();
            let te = ui.add(
                egui::TextEdit::multiline(&mut draft)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text(crate::tr!("Répondre à l'IA…", "Answer the AI…")),
            );
            let send_key = te.has_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
            let mut send = false;
            ui.horizontal(|ui| {
                let ok = !draft.trim().is_empty();
                send = ui
                    .add_enabled(ok, egui::Button::new(crate::tr!("Répondre", "Answer")))
                    .clicked()
                    || (ok && send_key);
                if editing
                    && super::projects::action_link(ui, crate::tr!("Annuler", "Cancel")).clicked()
                {
                    ui.data_mut(|d| d.insert_temp(edit_id, false));
                }
                let hint = if cfg!(target_os = "macos") {
                    crate::tr!("⌘ Entrée pour envoyer", "⌘ Enter to send")
                } else {
                    crate::tr!("Ctrl+Entrée pour envoyer", "Ctrl+Enter to send")
                };
                ui.label(RichText::new(hint).size(11.5).color(t.tertiary));
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

/// Under an answer: where it goes. An answer reaches an AI through its project's state of
/// play (get_project_state); a question asked outside any project has none to go through.
fn answer_note(no_project: bool) -> &'static str {
    if no_project {
        crate::tr!(
            "Question posée hors d'un projet : la réponse est gardée ici, aucune IA ne la reçoit.",
            "Question asked outside a project: the answer is kept here, no AI receives it."
        )
    } else {
        crate::tr!(
            "Transmise à l'IA : elle la lit au prochain état des lieux du projet, au début de sa prochaine étape ou session.",
            "Passed on to the AI: it reads it in the project's next state of play, at the start of its next step or session."
        )
    }
}

/// A ring gauge: `value` (0..1, None when unknown) and a tick at `limit`.
fn ring(
    ui: &egui::Ui,
    c: egui::Pos2,
    r: f32,
    value: Option<f32>,
    limit: f32,
    color: Color32,
    t: &Tone,
) {
    let p = ui.painter();
    let arc = |from: f32, to: f32, col: Color32, w: f32| {
        let n = 48;
        let pts: Vec<egui::Pos2> = (0..=n)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2
                    + std::f32::consts::TAU * (from + (to - from) * i as f32 / n as f32);
                c + egui::vec2(a.cos(), a.sin()) * r
            })
            .collect();
        p.add(egui::Shape::line(pts, egui::Stroke::new(w, col)));
    };
    arc(0.0, 1.0, t.fill, 6.0);
    if let Some(v) = value {
        let v = super::projects::smooth(ui, "orch-ring", v.clamp(0.0, 1.0));
        if v > 0.001 {
            arc(
                0.0,
                v,
                if v >= limit {
                    Color32::from_rgb(224, 62, 62)
                } else {
                    color
                },
                6.0,
            );
        }
    }
    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * limit.clamp(0.0, 1.0);
    let d = egui::vec2(a.cos(), a.sin());
    p.line_segment(
        [c + d * (r - 6.0), c + d * (r + 6.0)],
        egui::Stroke::new(2.0, super::projects::ORANGE),
    );
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

/// The end conditions of an endless session, each one ticked or not; the first reached stops it.
fn end_conditions(ui: &mut egui::Ui, c: &mut crate::model::EndConditions, t: &Tone) {
    let small = |s: &str| RichText::new(s).size(12.0).color(t.secondary);
    let mut on = c.hours.is_some();
    if ui
        .checkbox(&mut on, small(crate::tr!("une durée", "a time")))
        .changed()
    {
        c.hours = if on { Some(4.0) } else { None };
    }
    if let Some(h) = c.hours.as_mut() {
        ui.add(
            egui::DragValue::new(h)
                .range(0.5..=72.0)
                .speed(0.25)
                .suffix(" h"),
        );
    }
    let mut on = c.steps.is_some();
    if ui
        .checkbox(
            &mut on,
            small(crate::tr!("des étapes faites", "steps done")),
        )
        .changed()
    {
        c.steps = if on { Some(5) } else { None };
    }
    if let Some(n) = c.steps.as_mut() {
        ui.add(egui::DragValue::new(n).range(1..=500));
    }
    let mut on = c.turns.is_some();
    if ui
        .checkbox(&mut on, small(crate::tr!("des tours", "turns")))
        .changed()
    {
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
    let commits = if o.allow_commits {
        crate::tr!("commits autorisés", "commits allowed")
    } else {
        crate::tr!("sans commit", "no commits")
    };
    let resume = if o.auto_resume {
        crate::tr!("reprise automatique", "automatic restart")
    } else {
        crate::tr!("sans reprise", "no restart")
    };
    crate::trf!(
        "IA arrêtées à {:.0} % de la fenêtre de 5 h · {commits} · {resume} · bilan à {} h · {} tours au plus",
        "AIs stop at {:.0} % of the 5-hour window · {commits} · {resume} · summary at {} h · {} turns at most",
        o.global_limit,
        o.report_hour,
        o.max_turns
    )
}

/// "Lancer": a filled button.
/// Below this width, the week's reserve goes under the week's figure.
const WEEK_ONE_LINE: f32 = 520.0;

/// Below this width, a running session's links go under its title.
const RUN_ONE_LINE: f32 = 460.0;

/// The project's sessions merge into main by themselves: « sur main » ticked, and a code
/// project (the box isn't shown for one without code, so an old tick there counts for nothing).
fn on_main(pr: &crate::project::Project) -> bool {
    pr.commit_to_main && crate::project::code_kind(&pr.folder, pr.code)
}

/// A project's line in « Lancer une session », for its place in the list.
#[derive(Clone, Copy, Debug, Default)]
struct LaunchRow {
    /// A session runs on it.
    busy: bool,
    /// Its steps not done.
    open: usize,
    /// Kept in view even with nothing open: endless (the AI finds its steps), or picked.
    keep: bool,
}

impl LaunchRow {
    /// Nothing to start on: a session would stop at once.
    fn idle(&self) -> bool {
        !self.busy && self.open == 0 && !self.keep
    }
}

/// The order of the lines (indices into `rows`): running first, then those with open steps,
/// then the idle ones — left out unless `show_idle`, and counted (the second value).
fn launch_order(rows: &[LaunchRow], show_idle: bool) -> (Vec<usize>, usize) {
    let rank = |r: &LaunchRow| {
        if r.busy {
            0
        } else if r.idle() {
            2
        } else {
            1
        }
    };
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&i| rank(&rows[i]));
    let folded = if show_idle { 0 } else { rows.iter().filter(|r| r.idle()).count() };
    if !show_idle {
        order.retain(|&i| !rows[i].idle());
    }
    (order, folded)
}

/// Below this width, a project's controls in « Lancer une session » go under it.
const LAUNCH_ONE_LINE: f32 = 620.0;

/// What a project's launch controls work on.
struct LaunchCx<'a> {
    id: &'a String,
    d: &'a mut Draft,
    busy: bool,
    plugged: bool,
    /// Its open steps: none (and not endless), nothing to start on.
    open: usize,
    /// What is left of the global limit for this project.
    room: f32,
    global: f32,
    to_main: &'a mut HashMap<String, bool>,
    start: &'a mut Vec<String>,
    confirm_main: &'a mut Option<String>,
    /// Budgets the person has just changed, to keep in their projects.
    budgets: &'a mut Vec<(String, f32)>,
    lk: &'a super::settings::Look,
    t: &'a Tone,
}

/// A project's controls: start, commits, « sur main », budget. In the order of a
/// right-to-left layout (start at the far right); left to right, start comes first.
fn launch_controls(ui: &mut egui::Ui, c: LaunchCx<'_>) {
    let LaunchCx {
        id,
        d,
        busy,
        plugged,
        open,
        room,
        global,
        to_main,
        start,
        confirm_main,
        budgets,
        lk,
        t,
    } = c;
    if busy {
        // A pill where the other rows have « Lancer », so the controls stay in line.
        let text = crate::tr!("En cours", "Running");
        let g =
            ui.painter()
                .layout_no_wrap(text.to_string(), semibold(13.0), super::projects::GREEN);
        let (rect, _) =
            ui.allocate_exact_size(g.size() + egui::vec2(36.0, 12.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 7.0, super::projects::GREEN.gamma_multiply(0.14));
        super::projects::pulse_dot(
            ui,
            egui::pos2(rect.left() + 14.0, rect.center().y),
            super::projects::GREEN,
        );
        ui.painter().galley(
            egui::pos2(rect.left() + 24.0, rect.center().y - g.size().y / 2.0),
            g,
            super::projects::GREEN,
        );
    } else if !plugged {
        start_button(ui, false, t).on_disabled_hover_text(crate::tr!(
            "Branchez d'abord Claude Code",
            "Plug in Claude Code first"
        ));
    } else if open == 0 && !d.endless {
        start_button(ui, false, t).on_disabled_hover_text(crate::tr!(
            "Aucune étape ouverte : la session s'arrêterait tout de suite. Ajoutez des étapes au projet, ou activez « sans fin » pour que l'IA trouve elle-même les suivantes.",
            "No open step: the session would stop at once. Add steps to the project, or turn on « endless » so the AI finds the next ones itself."
        ));
    } else if room < 1.0 {
        start_button(ui, false, t).on_disabled_hover_text(crate::trf!(
            "La limite globale ({global:.0} %) est déjà répartie entre les autres projets",
            "The global limit ({global:.0} %) is already shared among the other projects"
        ));
    } else if start_button(ui, true, t).clicked() {
        // Endless on main: the warning first, the start once confirmed.
        if endless_on_main(d.endless, to_main.get(id).copied().unwrap_or(false)) {
            *confirm_main = Some(id.clone());
        } else {
            start.push(id.clone());
        }
    }
    ui.add_space(4.0);
    // « commits » and « sur main » only for a code project (`to_main` holds only those): one
    // without code shows no git words, and its sessions make no commit.
    if to_main.contains_key(id) {
        super::settings::switch(ui, &mut d.commits, lk).on_hover_text(crate::tr!("L'IA peut faire des commits sur sa branche (Orchestra l'envoie sur GitHub après chaque tour).\nGardé pour les prochaines sessions de ce projet.", "The AI may commit on its branch (Orchestra pushes it to GitHub after each turn).\nKept for this project's next sessions."));
        ui.label(RichText::new("commits").size(12.0).color(t.tertiary));
        ui.add_space(6.0);
    }
    if let Some(m) = to_main.get_mut(id) {
        ui.checkbox(m, RichText::new(crate::tr!("sur main", "on main")).size(12.0).color(t.tertiary)).on_hover_text(crate::tr!(
            "À la fin de chaque session, son travail est fusionné tout seul dans main (seulement si le dossier du projet est sur main ou master)",
            "At the end of each session, its work is merged into main by itself (only when the project's folder is on main or master)"
        ));
        ui.add_space(6.0);
    }
    // The budget chosen stays as it is: it is only shown capped to the room left (and
    // capped again at the start), so a session running elsewhere never shrinks it for good.
    let mut shown = if busy {
        d.limit
    } else {
        d.limit.clamp(1.0, room.max(1.0))
    };
    let dv = ui.add_enabled(
        !busy && room >= 1.0,
        egui::DragValue::new(&mut shown)
            .range(1.0..=room.max(1.0))
            .speed(0.5)
            .suffix(" %"),
    );
    if dv.changed() {
        d.limit = shown;
        budgets.push((id.clone(), shown));
    }
    dv.on_hover_text(crate::trf!(
        "Budget de la prochaine session de ce projet : elle s'arrête après avoir utilisé cette part de la fenêtre de 5 h de l'abonnement, comptée depuis son lancement. Chaque session a son propre budget : en relancer une repart de zéro. Le budget choisi est gardé pour les prochaines sessions de ce projet, même après un redémarrage.\n{room:.0} % au plus ici, le reste de la limite globale ({global:.0} %) étant pris par les autres.",
        "Budget of this project's next session: it stops after using this share of the subscription's 5-hour window, counted from its start. Each session has its own budget: starting another one begins again from zero. The budget chosen is kept for this project's next sessions, even after a restart.\n{room:.0} % at most here, the rest of the global limit ({global:.0} %) being taken by the others."
    ));
    ui.label(
        RichText::new(crate::tr!("budget par session", "budget per session"))
            .size(12.0)
            .color(t.tertiary),
    );
}

fn start_button(ui: &mut egui::Ui, enabled: bool, t: &Tone) -> egui::Response {
    let text = crate::tr!("Lancer", "Start");
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), semibold(13.0), Color32::WHITE);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(g.size() + egui::vec2(24.0, 12.0), sense);
    let a = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered() && enabled, 0.15);
    let base = if enabled {
        super::projects::BLUE
    } else {
        t.fill
    };
    let fill = base.lerp_to_gamma(Color32::WHITE, 0.12 * a);
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().galley(
        rect.center() - g.size() / 2.0,
        g,
        if enabled { Color32::WHITE } else { t.tertiary },
    );
    let resp = if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    };
    // Disabled: report it so on_disabled_hover_text works.
    if enabled {
        resp
    } else {
        resp.on_hover_text(crate::tr!(
            "Branchez d'abord Claude Code",
            "Plug in Claude Code first"
        ))
    }
}

/// The mark and colour of a finished session: done, stopped on a limit or a block, failed.
#[cfg(test)]
mod tests {
    /// A project's launch line starts from the choices kept in the project, else from the
    /// settings; and what it keeps back is what it started from.
    #[test]
    fn a_draft_starts_from_the_kept_choices() {
        use super::{Draft, default_prefs};
        use crate::model::EndConditions;
        use crate::project::{LaunchPrefs, Project};
        // Never set: the settings' budget and commits, not endless, 4 h when switched on.
        let d = Draft::new(None, 20.0, true);
        assert_eq!(
            (d.limit, d.commits, d.endless, d.pick),
            (20.0, true, false, false)
        );
        assert_eq!(d.end.hours, Some(4.0));
        assert_eq!(d.prefs(), default_prefs(true));
        let fresh = Project::default();
        assert_eq!(Draft::new(Some(&fresh), 30.0, false).limit, 30.0);
        // Set before: 95 %, no commits, endless for 6 h or 12 turns.
        let kept = LaunchPrefs {
            commits: false,
            endless: true,
            end: EndConditions {
                hours: Some(6.0),
                turns: Some(12),
                ..Default::default()
            },
        };
        let p = Project {
            session_limit: Some(95.0),
            launch: Some(kept.clone()),
            ..Default::default()
        };
        let d = Draft::new(Some(&p), 20.0, true);
        assert_eq!((d.limit, d.commits, d.endless), (95.0, false, true));
        assert!(d.mission.is_empty());
        assert_eq!(d.prefs(), kept);
        // Only the budget kept: the rest from the settings.
        let p = Project {
            session_limit: Some(50.0),
            ..Default::default()
        };
        let d = Draft::new(Some(&p), 20.0, true);
        assert_eq!((d.limit, d.commits, d.endless), (50.0, true, false));
    }

    /// The global limit shared in the launch card: what each project may take.
    #[test]
    fn the_global_limit_is_shared() {
        use super::{room_for, share_at_start};
        // Nothing running nor picked: all of it.
        assert_eq!(room_for(90.0, 0.0, 0.0, None), 90.0);
        // A session running at 30 %: 60 left, whole percents only.
        assert_eq!(room_for(90.0, 30.0, 0.0, None), 60.0);
        assert_eq!(room_for(90.0, 30.5, 0.0, None), 59.0);
        // Two projects picked, 20 and 25: one not picked sees 45 left; a picked one
        // doesn't count against itself.
        assert_eq!(room_for(90.0, 0.0, 45.0, None), 45.0);
        assert_eq!(room_for(90.0, 0.0, 45.0, Some(20.0)), 65.0);
        // Over-subscribed: never below zero.
        assert_eq!(room_for(90.0, 60.0, 95.0, None), 0.0);

        // At the start: the budget chosen when it fits…
        assert_eq!(share_at_start(20.0, 90.0, 0.0), Some(20.0));
        // …cut to what is left when it doesn't…
        assert_eq!(share_at_start(95.0, 95.0, 20.0), Some(75.0));
        // …at least 1 %…
        assert_eq!(share_at_start(0.0, 90.0, 0.0), Some(1.0));
        // …and no start at all when less than 1 % is left.
        assert_eq!(share_at_start(20.0, 90.0, 89.5), None);
        assert_eq!(share_at_start(20.0, 90.0, 90.0), None);
        // A selection of two at 95 % with a global limit of 95 %: the first takes it all,
        // the second doesn't start on a crumb.
        let first = share_at_start(95.0, 95.0, 0.0).unwrap();
        assert_eq!(first, 95.0);
        assert_eq!(share_at_start(95.0, 95.0, first), None);
    }

    #[test]
    fn only_endless_on_main_is_warned() {
        assert!(super::endless_on_main(true, true));
        assert!(!super::endless_on_main(true, false));
        assert!(!super::endless_on_main(false, true));
        assert!(!super::endless_main_warning().is_empty());
    }

    #[test]
    fn a_project_without_code_never_merges_into_main() {
        let mut pr = crate::project::Project {
            commit_to_main: true,
            code: Some(true),
            ..Default::default()
        };
        assert!(super::on_main(&pr));
        // « sur main » ticked before the project was set as without code: it counts for nothing.
        pr.code = Some(false);
        assert!(!super::on_main(&pr));
        pr.code = Some(true);
        pr.commit_to_main = false;
        assert!(!super::on_main(&pr));
    }

    /// The Orchestration page shows the waiting questions first, then the recent answers;
    /// never other messages nor old answers.
    #[test]
    fn orchestration_shows_waiting_questions_then_recent_answers() {
        use crate::model::InboxMsg;
        let day = 86_400_000;
        let now = 100 * day;
        let msg = |kind: &str, at: u64, answer: &str, answered_at: u64| InboxMsg {
            kind: kind.into(),
            at,
            answer: answer.into(),
            answered_at,
            ..Default::default()
        };
        let inbox = vec![
            msg("question", now - 30 * day, "", 0), // 0: old, still waiting
            msg("question", now - day, "oui", now - day), // 1: answered yesterday
            msg("fin", now, "", 0),                 // 2: not a question
            msg("question", now - 10 * day, "non", now - 9 * day), // 3: answered long ago
            msg("question", now - 2 * day, "", 0),  // 4: waiting, newer
            msg("question", now - 5 * day, "plus tard", now - 60_000), // 5: answered a minute ago
        ];
        assert_eq!(
            super::orch_questions(&inbox, now, super::ORCH_ANSWERED_MS),
            [4, 0, 5, 1]
        );
        // The project page keeps the answers whatever their age.
        assert_eq!(
            super::orch_questions(&inbox, now, u64::MAX),
            [4, 0, 5, 1, 3]
        );
        assert!(super::orch_questions(&[], now, super::ORCH_ANSWERED_MS).is_empty());
        let many: Vec<InboxMsg> = (0..9).map(|i| msg("question", now - i, "", 0)).collect();
        assert_eq!(
            super::orch_questions(&many, now, super::ORCH_ANSWERED_MS).len(),
            super::ORCH_QUESTIONS
        );
    }

    /// An end-of-session message finds its session by the report page, and only once that
    /// session is over and nothing else deals with it.
    #[test]
    fn an_end_message_finds_its_finished_session() {
        use crate::model::{InboxMsg, RunRecord};
        let run = |id: &str, report: &str, outcome: &str| RunRecord {
            id: id.into(),
            report: Some(report.into()),
            outcome: outcome.into(),
            ..Default::default()
        };
        let runs = vec![
            run("a", "page-a", "terminée"),
            run("b", "page-b", ""),
            run("c", "page-c", "terminée"),
        ];
        let msg = |kind: &str, page: Option<&str>| InboxMsg {
            kind: kind.into(),
            page: page.map(Into::into),
            ..Default::default()
        };
        let idle = |_: &str| false;
        assert_eq!(
            super::run_of_message(&msg("fin", Some("page-a")), &runs, idle).map(|r| r.id.as_str()),
            Some("a")
        );
        // Still running, unknown page, no page, another kind of message.
        assert!(super::run_of_message(&msg("fin", Some("page-b")), &runs, idle).is_none());
        assert!(super::run_of_message(&msg("fin", Some("autre")), &runs, idle).is_none());
        assert!(super::run_of_message(&msg("fin", None), &runs, idle).is_none());
        assert!(super::run_of_message(&msg("fin", Some("")), &runs, idle).is_none());
        assert!(super::run_of_message(&msg("rapport", Some("page-a")), &runs, idle).is_none());
        // Started again, or waiting to be merged into main by itself.
        assert!(
            super::run_of_message(&msg("fin", Some("page-c")), &runs, |id| id == "c").is_none()
        );
    }

    /// Listed on the Orchestration page: only finished work still waiting in its copy whose
    /// end message (and so its links) left the inbox.
    #[test]
    fn copies_left_without_their_end_message_are_listed() {
        use crate::model::{InboxMsg, RunRecord};
        let run = |id: &str, outcome: &str, closed: &str| RunRecord {
            id: id.into(),
            worktree: format!("/w/{id}"),
            report: Some(format!("page-{id}")),
            outcome: outcome.into(),
            closed: closed.into(),
            ..Default::default()
        };
        let runs = vec![
            run("orpheline", "terminée", ""),
            run("avec-message", "terminée", ""),
            run("fusionnee", "terminée", "fusionné"),
            run("en-cours", "", ""),
            run("copie-partie", "terminée", ""),
            run("vers-main", "terminée", ""),
        ];
        let inbox = vec![InboxMsg {
            kind: "fin".into(),
            page: Some("page-avec-message".into()),
            ..Default::default()
        }];
        let exists = |w: &str| w != "/w/copie-partie";
        assert_eq!(
            super::orphan_copies(&runs, &inbox, |id| id == "vers-main", exists),
            [0]
        );
        assert_eq!(
            super::orphan_copies(&runs, &[], |_| false, exists),
            [0, 1, 5]
        );
    }

    #[test]
    fn inbox_messages_group_by_local_day() {
        let h = 3_600_000u64;
        let day = 24 * h;
        // Noon UTC on some day, local time UTC+1.
        let now = 20_000 * day + 12 * h;
        let g = |at: u64| super::day_group(at, now, h as i64);
        assert_eq!(g(now - h), g(now));
        // 22:30 UTC the day before is 23:30 local: yesterday; 23:30 UTC is 00:30 local: today.
        assert_eq!(g(now - 12 * h - 90 * 60_000), g(now - day));
        assert_eq!(g(now - 12 * h + 30 * 60_000), g(now));
        assert_ne!(g(now - day), g(now));
        assert_eq!(g(now - 3 * day), g(now - 6 * day));
        assert_ne!(g(now - 3 * day), g(now - 7 * day));
        // A clock a little behind: still today.
        assert_eq!(g(now + h), g(now));
    }

    use eframe::egui;

    /// Runs `add` in a column of `width` px, as `orchestra_view` sets it (`ui.set_width`),
    /// without a window. Two passes: egui settles sizes on the second.
    fn column(width: f32, mut add: impl FnMut(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        for _ in 0..2 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                ..Default::default()
            });
            egui::Area::new(egui::Id::unique("column"))
                .fixed_pos(egui::Pos2::ZERO)
                .show(&ctx, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        add(ui);
                    });
                });
            // No screen to send the textures to: drop them on purpose.
            ctx.end_pass().textures_delta.clear();
        }
    }

    /// A card as `projects::card` lays it out (full width, inner margin); its width.
    fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) -> f32 {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(18, 16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(ui);
            })
            .response
            .rect
            .width()
    }

    /// The running session's card: its inner frame and the row of its budget.
    fn running(ui: &mut egui::Ui) {
        let t = super::tone(false);
        egui::Frame::new()
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                super::share_row(ui, "run", 0.0, 20.0, Some(75.0), 90.0, &t);
            });
    }

    /// A card drawn before a session starts, the running session's card, then a card
    /// after it; the widths of the three.
    fn card_widths(width: f32) -> [f32; 3] {
        let mut out = [0.0; 3];
        column(width, |ui| {
            out[0] = card(ui, |ui| {
                ui.label("Cette semaine");
            });
            out[1] = card(ui, running);
            out[2] = card(ui, |ui| {
                ui.label("Réglages et limites");
            });
        });
        out
    }

    /// The mechanism behind the gap: in an egui column, something wider than the column
    /// widens it for everything that follows, never for what came before. The column has
    /// no grid nor fixed widths of its own (the cards fill `ui.available_width()`).
    #[test]
    fn a_row_wider_than_the_column_widens_what_follows() {
        let (mut before, mut after) = (0.0, 0.0);
        column(400.0, |ui| {
            before = card(ui, |ui| {
                ui.label("avant");
            });
            ui.allocate_exact_size(egui::vec2(460.0, 10.0), egui::Sense::hover());
            after = card(ui, |ui| {
                ui.label("après");
            });
        });
        assert_eq!(before, 400.0);
        assert_eq!(after, 460.0);
    }

    /// The cause was the budget row of a running session: its bar took all the width but
    /// 150 px, kept for « 0.0 % utilisés sur un budget de 20 % », which needs more, so the
    /// row came out wider than its card. Now it fills its card exactly, side by side when
    /// there is room, the text under the bar when not, with large figures too.
    #[test]
    fn the_budget_row_fits_its_card() {
        for width in [140.0, 240.0, 300.0, 480.0, 1000.0] {
            for (pct, limit, window) in [
                (0.0, 20.0, None),
                (123.4, 100.0, None),
                (5.0, 90.0, Some(80.0)),
                (12.3, 100.0, Some(100.0)),
            ] {
                let mut row = 0.0;
                column(width, |ui| {
                    ui.scope(|ui| {
                        let t = super::tone(false);
                        super::share_row(ui, "run", pct, limit, window, 90.0, &t);
                        row = ui.min_rect().width();
                    });
                });
                assert!(
                    row <= width + 0.5,
                    "à {width} px ({pct} / {limit}) : rangée de {row} px"
                );
            }
        }
    }

    /// The gap once seen when a session started: the cards from the running one down came
    /// out 57.5 px wider than the ones above.
    #[test]
    fn cards_keep_one_width_once_a_session_runs() {
        let mut uneven = vec![];
        for width in [320.0, 480.0, 620.0, 1000.0] {
            let [above, running, below] = card_widths(width);
            if (running - above).abs() >= 0.5 || (below - above).abs() >= 0.5 {
                uneven.push(format!(
                    "à {width} px : au-dessus {above}, en cours {running}, en dessous {below}"
                ));
            }
        }
        assert!(uneven.is_empty(), "{}", uneven.join(" ; "));
    }

    /// The smallest window (640 px, sidebar open) leaves the page a column of about 330 px,
    /// 296 inside a card: a project's launch controls, under it there, stay within it.
    #[test]
    fn launch_controls_fit_a_narrow_column() {
        for width in [296.0, 400.0] {
            let mut row = 0.0;
            column(width, |ui| {
                let t = super::tone(false);
                let lk = super::super::settings::look(false);
                let id = "p".to_string();
                let mut d = super::Draft {
                    mission: String::new(),
                    limit: 20.0,
                    commits: true,
                    pick: false,
                    endless: false,
                    end: Default::default(),
                };
                let mut to_main: super::HashMap<String, bool> =
                    [(id.clone(), false)].into_iter().collect();
                let (mut start, mut confirm) = (vec![], None);
                ui.scope(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(70.0);
                        super::launch_controls(
                            ui,
                            super::LaunchCx {
                                id: &id,
                                d: &mut d,
                                busy: false,
                                plugged: true,
                                open: 3,
                                room: 60.0,
                                global: 60.0,
                                to_main: &mut to_main,
                                start: &mut start,
                                confirm_main: &mut confirm,
                                budgets: &mut vec![],
                                lk: &lk,
                                t: &t,
                            },
                        );
                    });
                    row = ui.min_rect().width();
                });
            });
            assert!(row <= width + 0.5, "à {width} px : commandes sur {row} px");
        }
    }

    #[test]
    fn what_can_start_comes_first_and_idle_projects_fold() {
        use super::{LaunchRow, launch_order};
        let r = |busy, open, keep| LaunchRow { busy, open, keep };
        let rows = [
            r(false, 0, false), // 0: nothing to do
            r(false, 2, false), // 1: open steps
            r(true, 0, false),  // 2: running
            r(false, 0, true),  // 3: endless, no step yet
            r(false, 5, false), // 4: open steps
        ];
        // Running, then those that can start (in their own order), idle ones folded.
        assert_eq!(launch_order(&rows, false), (vec![2, 1, 3, 4], 1));
        // Unfolded: the idle ones last.
        assert_eq!(launch_order(&rows, true), (vec![2, 1, 3, 4, 0], 0));
        assert_eq!(launch_order(&[], false), (vec![], 0));
    }

    /// A project without code (absent from `to_main`) gets neither « commits » nor « sur main »:
    /// its controls come out narrower than a code project's.
    #[test]
    fn a_project_without_code_shows_no_git_switches() {
        let width_of = |code: bool| {
            let mut w = 0.0;
            column(2000.0, |ui| {
                let t = super::tone(false);
                let lk = super::super::settings::look(false);
                let id = "p".to_string();
                let mut d = super::Draft {
                    mission: String::new(),
                    limit: 20.0,
                    commits: true,
                    pick: false,
                    endless: false,
                    end: Default::default(),
                };
                let mut to_main: super::HashMap<String, bool> = super::HashMap::new();
                if code {
                    to_main.insert(id.clone(), false);
                }
                let (mut start, mut confirm) = (vec![], None);
                let r = ui.horizontal(|ui| {
                    super::launch_controls(
                        ui,
                        super::LaunchCx {
                            id: &id,
                            d: &mut d,
                            busy: false,
                            plugged: true,
                            open: 3,
                            room: 60.0,
                            global: 60.0,
                            to_main: &mut to_main,
                            start: &mut start,
                            confirm_main: &mut confirm,
                            budgets: &mut vec![],
                            lk: &lk,
                            t: &t,
                        },
                    );
                });
                w = r.response.rect.width();
            });
            w
        };
        let (code, nocode) = (width_of(true), width_of(false));
        // The switch, « commits », the box and « sur main »: well over 100 px.
        assert!(code - nocode > 100.0, "code {code} px, sans code {nocode} px");
    }

    /// The case reported: the 5-hour window already at 75 % when a session starts with a
    /// budget of 90 %; it then rises to 80 %. The session's card reads « 5 % utilisés sur
    /// un budget de 90 % »: its own use since it started, not the window's 80 %.
    #[test]
    fn a_session_shows_its_own_use_not_the_window() {
        let reset = Some(1_000_000u64);
        // First reading after the start: the 75 % already there are nobody's.
        let mut pct = 0.0;
        let mut prev: Option<(f32, Option<u64>)> = None;
        for (window, tokens) in [(75.0, 0u64), (78.0, 3_000), (80.0, 2_000)] {
            pct += super::split_rise(super::window_rise(prev, window, reset), &[tokens])[0];
            prev = Some((window, reset));
        }
        assert!((pct - 5.0).abs() < 0.01, "{pct}");
        // Two sessions: the rise shared by their tokens.
        let s = super::split_rise(4.0, &[3_000, 1_000]);
        assert!((s[0] - 3.0).abs() < 0.01 && (s[1] - 1.0).abs() < 0.01);
        // A new window: counted from zero; an unknown reset time on both sides: the difference.
        assert_eq!(
            super::window_rise(Some((75.0, reset)), 2.0, Some(1_000_000 + 5 * 3_600_000)),
            2.0
        );
        assert_eq!(super::window_rise(Some((10.0, None)), 12.0, None), 2.0);
    }

    /// The bar of a running session is the 5-hour window itself. The case reported: 75 %
    /// used before it, 5 % since, a budget of 90 % and a global limit of 90 %: the window
    /// reads 80 %, and it stops at the global limit (90 %), well before its budget.
    #[test]
    fn a_session_bar_is_the_five_hour_window() {
        use super::{WindowBar, window_bar};
        assert_eq!(
            window_bar(80.0, 5.0, 90.0, 90.0),
            WindowBar {
                before: 75.0,
                window: 80.0,
                stop: 90.0
            }
        );
        // A small budget stops it first.
        assert_eq!(
            window_bar(80.0, 5.0, 10.0, 90.0),
            WindowBar {
                before: 75.0,
                window: 80.0,
                stop: 85.0
            }
        );
        // Its share can't start below zero, nor anything go past 100.
        assert_eq!(
            window_bar(3.0, 5.0, 20.0, 100.0),
            WindowBar {
                before: 0.0,
                window: 3.0,
                stop: 20.0
            }
        );
        assert_eq!(window_bar(120.0, 5.0, 50.0, 200.0).stop, 100.0);
    }

    #[test]
    fn each_session_is_stopped_on_its_own_agents_subscription() {
        use super::{Agent, Figures, Stop, agent_shares, stop_reason};
        // A rise of Codex's window goes to the Codex sessions only, by their tokens.
        let sessions = [(Agent::ClaudeCode, 500), (Agent::Codex, 300), (Agent::Codex, 100)];
        let s = agent_shares(&sessions, Agent::Codex, 8.0);
        assert_eq!(s[0], None);
        assert!((s[1].unwrap() - 6.0).abs() < 0.01 && (s[2].unwrap() - 2.0).abs() < 0.01);
        let s = agent_shares(&sessions, Agent::ClaudeCode, 4.0);
        assert!((s[0].unwrap() - 4.0).abs() < 0.01 && s[1].is_none() && s[2].is_none());
        // Claude Code's window full, Codex's not: the Codex session goes on.
        let claude = Figures { five: Some(80.0), ..Default::default() };
        let codex = Figures { five: Some(10.0), week: Some(5.0), ..Default::default() };
        let end = |f: Figures| {
            let mut t = calm();
            (t.five, t.week) = (f.five, f.week);
            stop_reason(&t)
        };
        assert_eq!(end(claude), Some(Stop::GlobalLimit(60.0)));
        assert_eq!(end(codex), None);
        // An agent that tells no figures: never measured, so it stops at its second turn.
        let mut t = calm();
        (t.five, t.week, t.measured, t.turns) = (None, None, false, 2);
        assert_eq!(stop_reason(&t), Some(Stop::Unmeasured));
        // And none is ever started alone without its figures.
        for a in Agent::ALL {
            assert!(!a.ready() || crate::usage::reports_usage(a), "{a:?}");
        }
        // Their limit messages end the session on the spot.
        for m in [
            "You've hit your usage limit. Try again in 2 hours.",
            "[API Error: Quota exceeded for quota metric 'Gemini 2.5 Pro Requests']",
            "RESOURCE_EXHAUSTED",
            "Error: Rate limit exceeded (429)",
        ] {
            assert!(crate::auto::is_limit_message(m), "{m}");
        }
    }

    /// A turn that ends with nothing to stop on: under every limit, measured, few turns.
    fn calm() -> super::TurnEnd {
        super::TurnEnd {
            asked: None,
            too_long: false,
            endless_end: None,
            signal: super::Signal::Continue,
            endless: false,
            pct: 2.0,
            limit: 20.0,
            five: Some(30.0),
            global: 60.0,
            week: Some(40.0),
            week_limit: 95.0,
            week_turn: 1.0,
            measured: true,
            turns: 3,
            max_turns: 30,
        }
    }

    #[test]
    fn a_session_goes_on_or_stops_for_the_first_reason() {
        use super::{Quota, Signal, Stop, TurnEnd, stop_reason};
        assert_eq!(stop_reason(&calm()), None);
        let with = |f: &dyn Fn(&mut TurnEnd)| {
            let mut t = calm();
            f(&mut t);
            stop_reason(&t)
        };
        assert_eq!(
            with(&|t| t.asked = Some("arrêtée à la main".into())),
            Some(Stop::Asked("arrêtée à la main".into()))
        );
        assert_eq!(with(&|t| t.too_long = true), Some(Stop::TooLong));
        assert_eq!(with(&|t| t.signal = Signal::Done), Some(Stop::MissionDone));
        assert_eq!(with(&|t| t.signal = Signal::Blocked), Some(Stop::Blocked));
        assert_eq!(with(&|t| t.pct = 20.0), Some(Stop::Budget(20.0)));
        assert_eq!(
            with(&|t| t.five = Some(60.0)),
            Some(Stop::GlobalLimit(60.0))
        );
        assert_eq!(with(&|t| t.week = Some(94.5)), Some(Stop::WeekReserve(5.0)));
        assert_eq!(with(&|t| t.measured = false), Some(Stop::Unmeasured));
        assert_eq!(with(&|t| t.turns = 30), Some(Stop::Turns(30)));
        // Unmeasured only from the second turn; unknown figures stop nothing.
        assert_eq!(
            with(&|t| {
                t.measured = false;
                t.turns = 1
            }),
            None
        );
        assert_eq!(
            with(&|t| {
                t.five = None;
                t.week = None
            }),
            None
        );
        // Endless: neither [[FINI]] nor the turn limit, its own end conditions instead.
        assert_eq!(
            with(&|t| {
                t.endless = true;
                t.signal = Signal::Done;
                t.turns = 500
            }),
            None
        );
        assert_eq!(
            with(&|t| {
                t.endless = true;
                t.endless_end = Some("4 h écoulées".into())
            }),
            Some(Stop::EndlessEnd("4 h écoulées".into()))
        );
        // The order: a stop asked comes before all, the budget before the global limit…
        assert_eq!(
            with(&|t| {
                t.asked = Some("x".into());
                t.too_long = true;
                t.pct = 99.0
            }),
            Some(Stop::Asked("x".into()))
        );
        assert_eq!(
            with(&|t| {
                t.pct = 20.0;
                t.five = Some(90.0)
            }),
            Some(Stop::Budget(20.0))
        );
        assert_eq!(
            with(&|t| {
                t.five = Some(90.0);
                t.week = Some(99.0)
            }),
            Some(Stop::GlobalLimit(60.0))
        );
        // Only the quotas start again by themselves, each when its own window resets.
        assert_eq!(Stop::GlobalLimit(60.0).quota(), Quota::FiveHours);
        assert_eq!(Stop::WeekReserve(5.0).quota(), Quota::Week);
        assert_eq!(Stop::Budget(20.0).quota(), Quota::None);
        assert_eq!(Stop::MissionDone.quota(), Quota::None);
        assert!(Stop::GlobalLimit(60.0).text().contains("60"));
    }

    /// Steps counted by id: renamed, twins and new ones each count once, and right.
    #[test]
    fn steps_since_the_start_go_by_id() {
        use crate::project::{Project, StepStatus};
        let mut p = Project::default();
        let a = p.add_step("Écrire les tests", "moi");
        let b = p.add_step("Doublon", "moi");
        let c = p.add_step("Doublon", "moi");
        let d = p.add_step("Déjà faite", "moi");
        p.set_step_status(&d, StepStatus::Done);
        let before: Vec<(String, bool)> = p
            .steps
            .iter()
            .map(|s| (s.id.clone(), s.status == StepStatus::Done))
            .collect();
        assert_eq!(
            super::steps_since(&before, &p.steps),
            super::StepsSince {
                done: vec![],
                added_open: 0
            }
        );
        // During the session: A renamed then done, one of the twins done, a new step added
        // and another added then done.
        p.steps.iter_mut().find(|s| s.id == a).unwrap().title =
            "Écrire les tests (unitaires)".into();
        p.set_step_status(&a, StepStatus::Done);
        p.set_step_status(&c, StepStatus::Done);
        p.add_step("Nouvelle étape", "claude-code");
        let e = p.add_step("Nouvelle et faite", "claude-code");
        p.set_step_status(&e, StepStatus::Done);
        let s = super::steps_since(&before, &p.steps);
        assert_eq!(
            s.done,
            vec![
                "Écrire les tests (unitaires)".to_string(),
                "Doublon".into(),
                "Nouvelle et faite".into()
            ]
        );
        // Only the new open step: the renamed one is not new, the twin still open is not new.
        assert_eq!(s.added_open, 1);
        assert!(
            p.steps
                .iter()
                .any(|x| x.id == b && x.status == StepStatus::Todo)
        );
        // The person adds many steps meanwhile (typed, or AI proposals accepted, or received
        // from a teammate): they never count as the AI piling up steps, so the endless
        // session goes on.
        for i in 0..10 {
            p.add_step(&format!("À moi {i}"), crate::project::SOURCE_PERSON);
            p.add_step(&format!("Proposée {i}"), crate::project::SOURCE_ACCEPTED);
        }
        let r = p.add_step("Reçue", "autre");
        p.steps.iter_mut().find(|x| x.id == r).unwrap().received = true;
        let s = super::steps_since(&before, &p.steps);
        assert_eq!(s.added_open, 1);
        assert!(!crate::auto::too_many_self_steps(s.added_open));
        // An AI adding through MCP (default source « ia ») still counts.
        for i in 0..7 {
            p.add_step(&format!("Ajoutée par l'IA {i}"), "ia");
        }
        assert!(crate::auto::too_many_self_steps(
            super::steps_since(&before, &p.steps).added_open
        ));
    }

    /// The step shown as « busy with »: doing, ours or unreserved, the latest changed.
    #[test]
    fn current_step_is_the_doing_one_of_this_computer() {
        use crate::project::{Project, StepStatus};
        let mut p = Project::default();
        let a = p.add_step("A faire", "moi");
        assert_eq!(super::current_step(&p.steps, "ici"), None);
        let b = p.add_step("Chez un autre", "moi");
        p.set_step_status(&b, StepStatus::Doing);
        p.steps.iter_mut().find(|s| s.id == b).unwrap().held_by = "ailleurs".into();
        assert_eq!(
            super::current_step(&p.steps, "ici"),
            None,
            "reserved by another computer"
        );
        p.set_step_status(&a, StepStatus::Doing);
        p.steps.iter_mut().find(|s| s.id == a).unwrap().held_by = "ici".into();
        assert_eq!(super::current_step(&p.steps, "ici"), Some("A faire"));
        let c = p.add_step("Plus récente", "moi");
        p.set_step_status(&c, StepStatus::Doing);
        p.steps.iter_mut().find(|s| s.id == c).unwrap().updated = u64::MAX;
        assert_eq!(super::current_step(&p.steps, "ici"), Some("Plus récente"));
        p.set_step_status(&c, StepStatus::Done);
        p.set_step_status(&a, StepStatus::Done);
        assert_eq!(super::current_step(&p.steps, "ici"), None);
    }

    /// Steps the person adds during a turn reach the AI on the next one, even after a [[FINI]].
    #[test]
    fn steps_added_during_a_turn_are_sent_on() {
        use crate::project::{Project, StepStatus};
        let mut p = Project::default();
        p.add_step("Ancienne", crate::project::SOURCE_PERSON);
        let at_turn: Vec<String> = p.steps.iter().map(|s| s.id.clone()).collect();
        assert!(super::fresh_steps(&at_turn, &p.steps).is_empty());
        assert_eq!(super::new_steps_note(&[]), "");
        // During the turn: the person adds two, one gets done already; the AI adds its own.
        p.add_step("Nouvelle à faire", crate::project::SOURCE_PERSON);
        let d = p.add_step("Nouvelle déjà faite", crate::project::SOURCE_PERSON);
        p.set_step_status(&d, StepStatus::Done);
        p.add_step("Proposée et acceptée", crate::project::SOURCE_ACCEPTED);
        p.add_step("De l'IA", "claude-code");
        let fresh = super::fresh_steps(&at_turn, &p.steps);
        assert_eq!(
            fresh,
            vec![
                "Nouvelle à faire".to_string(),
                "Proposée et acceptée".into()
            ]
        );
        let note = super::new_steps_note(&fresh);
        assert!(note.contains("Nouvelle à faire") && note.contains('2'));
        // The next turn starts from what is there now: nothing new any more.
        let at_turn: Vec<String> = p.steps.iter().map(|s| s.id.clone()).collect();
        assert!(super::fresh_steps(&at_turn, &p.steps).is_empty());
    }

    #[test]
    fn inbox_text_drops_markdown_marks() {
        let s = "### Fait\n- **une** chose\n\n\n### Reste\n- rien\n\n";
        assert_eq!(super::plain_text(s), "Fait\n- une chose\n\nReste\n- rien");
    }
}
