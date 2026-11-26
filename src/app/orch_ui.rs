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
use crate::project::StepIssue;
use crate::terminal::Term;
use std::collections::HashMap;
use std::time::Duration;

mod inbox;
mod overview;
mod sessions;
#[cfg(test)]
mod tests;

use inbox::*;
use overview::*;
use sessions::*;
pub(super) use inbox::{orch_questions, question_list, questions_count};

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
    /// Where it stops, in absolute % of the 5-hour window (37 % used at its start and a
    /// budget of 13 %: 50 %); None until the window's figure is known (see `ceiling_from`).
    pub ceiling: Option<f32>,
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
    /// The session's first prompt (mission and rules), given again to each new conversation.
    first_prompt: String,
    /// Steps done since the start, as counted at the end of the last turn.
    steps_done: u32,
    /// When the pause began (wall clock, ms: a Mac asleep still lets the cache expire).
    paused_wall: u64,
    /// The most it may spend in dollars (micro-dollars): only for an AI run with an API key.
    budget_micro: Option<u64>,
}

#[derive(Clone)]
struct Draft {
    mission: String,
    /// Where the next session stops, in absolute % of the 5-hour window, as chosen (None:
    /// the default, see `ceiling_choice`).
    ceiling: Option<f32>,
    /// The ceiling was changed in the launch settings since Orchestra opened: it then wins over
    /// that of a session already running (see `draft_ceiling`).
    touched: bool,
    commits: bool,
    /// Endless mode, and its end conditions as set in the launch settings.
    endless: bool,
    end: crate::model::EndConditions,
    /// The AI the session is started with (the fallback when `auto`).
    agent: Agent,
    /// « Automatique »: the AI is picked at the start (see `pick_agent`).
    auto: bool,
    /// The model imposed for the project's sessions (empty: Orchestra chooses).
    model: String,
    saving: crate::model::Saving,
}

impl Draft {
    /// A project's launch choices, with those the person last made for it
    /// (kept in the project), else the settings' defaults (`commits`).
    fn new(pr: Option<&crate::project::Project>, commits: bool) -> Self {
        let prefs = pr
            .and_then(|x| x.launch.clone())
            .unwrap_or_else(|| default_prefs(commits));
        Draft {
            mission: String::new(),
            ceiling: pr.and_then(|x| x.session_ceiling),
            touched: false,
            commits: prefs.commits,
            endless: prefs.endless,
            end: prefs.end,
            agent: prefs.agent.unwrap_or(Agent::ClaudeCode),
            auto: prefs.auto_agent,
            model: prefs.model,
            saving: prefs.saving,
        }
    }

    /// The choices kept in the project for its next sessions.
    fn prefs(&self) -> crate::project::LaunchPrefs {
        crate::project::LaunchPrefs {
            commits: self.commits,
            endless: self.endless,
            end: self.end.clone(),
            // Claude Code, the default, isn't written.
            agent: (self.agent != Agent::ClaudeCode).then_some(self.agent),
            model: self.model.clone(),
            saving: self.saving,
            auto_agent: self.auto,
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
        agent: None,
        model: String::new(),
        saving: Default::default(),
        // A project never set: Orchestra picks its AI at each start.
        auto_agent: true,
    }
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
    /// The cross review's opinion, set by the thread when there was one.
    review: std::sync::Arc<std::sync::Mutex<Option<crate::review::Review>>>,
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
    /// Since when (ms) Claude Code's 5-hour window can't be read; None while it can.
    unmeasured_since: Option<u64>,
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
    /// An endless session on main waiting for the person to confirm the warning: its
    /// project id.
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

impl Orch {
    /// The running sessions of `project`, each with the step it is on, for that step's row.
    pub(super) fn step_sessions(
        &self,
        project: &str,
        steps: &[crate::project::Step],
        records: &[crate::model::RunRecord],
    ) -> Vec<super::projects::StepSession> {
        let me = crate::store::computer_id();
        self.runs
            .iter()
            .filter(|r| r.project == project)
            .filter_map(|r| {
                let step = session_step(sent_on(records, &r.id), steps, &holder_of(r.agent, &me))?;
                Some(super::projects::StepSession {
                    run: r.id.clone(),
                    step: step.id.clone(),
                    state: r.state_text(),
                    // « Tour 2 en cours… » only says the state again.
                    detail: if r.last_line == turn_running(r.turns) {
                        String::new()
                    } else {
                        r.last_line.clone()
                    },
                    waiting: r.waiting.is_some(),
                })
            })
            .collect()
    }
}
