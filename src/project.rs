//! AI projects: a page can be a project that gathers what the AIs did on it — the code
//! folder, its status, a state-of-play summary written by an AI, the steps to follow and
//! the AI sessions (Claude Code, Codex, claude.ai…) that worked on it.
//!
//! A project lives inside its page (`Page::project`), so it is saved, synced, merged and
//! exported like any page, and the AIs reach it through the MCP tools in mcp.rs.

use crate::model::{Page, Workspace, is_zero, new_id, now_ms};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// Longest state of play kept (characters), and longest step title. Step titles are what
/// the AIs work from, so they are kept whole (the window shows one line, a click shows all).
pub const SUMMARY_MAX: usize = 4000;
pub const STEP_MAX: usize = 2000;

/// How an AI must write a step, said in its prompts and in the MCP tools' errors.
pub const AI_STEP_FORMAT: &str = "« Theme: action » — a short theme (1 to 4 words), a colon, then ONE action with its verb first (in French: « Thème : action à l'infinitif »), e.g. « Sécurité : échapper le nom du dossier passé à git (src/git.rs) ». One action per step: no « ; », « puis », « then », no second sentence.";

/// A step written by an AI, checked against `AI_STEP_FORMAT` and tidied (one line, « Thème :
/// action » spacing, the theme's first letter upper case). Err: why it doesn't fit, to tell
/// the AI so it rewrites the step. The person's own steps are never checked.
pub fn ai_step(title: &str) -> Result<String, String> {
    let t = crate::richtext::step_title(title, STEP_MAX);
    let bad = |why: &str| Err(format!("« {t} » : {why}. Expected {AI_STEP_FORMAT}"));
    let Some((theme, action)) = t.split_once(':') else {
        return bad("no theme before a colon");
    };
    let (theme, action) = (theme.trim(), action.trim());
    let words = theme.split_whitespace().count();
    if words == 0 || words > 4 || theme.chars().count() > 40 {
        return bad("the theme must be 1 to 4 words");
    }
    if theme.chars().any(|c| matches!(c, '(' | ')' | '«' | '»' | '"' | '`')) {
        return bad("the theme must be a plain word or two");
    }
    if action.chars().filter(|c| c.is_alphabetic()).count() < 3 {
        return bad("no action after the colon");
    }
    let low = format!(" {} ", action.to_lowercase());
    let two = action.contains(';')
        || [" puis ", " ensuite ", " then ", " and then ", " et ensuite "]
            .iter()
            .any(|w| low.contains(w))
        // A second sentence: « . » followed by a capital letter.
        || action
            .split(". ")
            .skip(1)
            .any(|s| s.chars().next().is_some_and(char::is_uppercase));
    if two {
        return bad("one action per step (split it into several steps)");
    }
    let mut c = theme.chars();
    let theme: String = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
    Ok(format!("{theme} : {action}"))
}

/// A state of play built from lists, so every AI's summary has the same shape:
/// "Fait", "En cours", "Bloquants", then free notes.
pub fn summary_from_parts(
    done: &[String],
    doing: &[String],
    blockers: &[String],
    notes: &str,
) -> String {
    let mut out = String::new();
    for (title, items) in [
        (crate::tr!("Fait", "Done"), done),
        (crate::tr!("En cours", "In progress"), doing),
        (crate::tr!("Bloquants", "Blockers"), blockers),
    ] {
        if items.is_empty() {
            continue;
        }
        out.push_str(&format!("### {title}\n"));
        for it in items {
            out.push_str(&format!("- {}\n", crate::richtext::one_line_md(it, 300)));
        }
        out.push('\n');
    }
    if !notes.trim().is_empty() {
        out.push_str(notes.trim());
    }
    out.trim().to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    #[default]
    Idea,
    Active,
    Paused,
    Done,
}

impl ProjectStatus {
    pub const ALL: [ProjectStatus; 4] = [
        ProjectStatus::Idea,
        ProjectStatus::Active,
        ProjectStatus::Paused,
        ProjectStatus::Done,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ProjectStatus::Idea => crate::tr!("Idée", "Idea"),
            ProjectStatus::Active => crate::tr!("En cours", "In progress"),
            ProjectStatus::Paused => crate::tr!("En pause", "Paused"),
            ProjectStatus::Done => crate::tr!("Terminé", "Done"),
        }
    }

    /// Accepts the JSON name or the French label, ignoring case.
    pub fn from_name(s: &str) -> Option<Self> {
        let s = s.trim().to_lowercase();
        Self::ALL.into_iter().find(|k| {
            k.label().to_lowercase() == s
                || serde_json::to_value(k)
                    .ok()
                    .and_then(|v| v.as_str().map(|x| x == s))
                    == Some(true)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    #[default]
    Todo,
    Doing,
    Done,
}

impl StepStatus {
    pub const ALL: [StepStatus; 3] = [StepStatus::Todo, StepStatus::Doing, StepStatus::Done];

    pub fn label(self) -> &'static str {
        match self {
            StepStatus::Todo => crate::tr!("À faire", "To-do"),
            StepStatus::Doing => crate::tr!("En cours", "In progress"),
            StepStatus::Done => crate::tr!("Terminé", "Done"),
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        let s = s.trim().to_lowercase();
        Self::ALL.into_iter().find(|k| {
            k.label().to_lowercase() == s
                || serde_json::to_value(k)
                    .ok()
                    .and_then(|v| v.as_str().map(|x| x == s))
                    == Some(true)
        })
    }
}

/// One step to follow. `source` says who added it: "moi", "claude-code", "codex"…
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub status: StepStatus,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub updated: u64,
    /// Received from another member of a shared project. Decided here only, never taken
    /// from the other copy (see `Project::merge_steps`): the AIs are told such a step comes
    /// from elsewhere, to weigh it as a request rather than an order from this computer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub received: bool,
    /// The computer whose AI set the step « doing » (`store::computer_id`), and when: the
    /// step is reserved, so that two AIs on a shared project never do the same step. Shared
    /// with the other copies; cleared when the step leaves « doing ».
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub held_by: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub held_at: u64,
    /// The model the person imposed for this step (empty: Orchestra chooses). Wins over the
    /// project's (`LaunchPrefs::model`) when the step is the one a session takes first.
    /// Decided on this computer only: the models offered depend on the AIs plugged in here.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// The AI the person assigned to this step (None: the project's, `LaunchPrefs::agent`).
    /// Decided on this computer only, like the model: the AIs plugged in differ per computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<crate::auto::Agent>,
}

/// `source` of a step typed by the person in the window.
pub const SOURCE_PERSON: &str = "moi";
/// `source` of a step an AI proposed in the window and the person accepted.
pub const SOURCE_ACCEPTED: &str = "IA";

impl Step {
    /// Added by the person (typed, or an AI proposal they accepted) or received from another
    /// member: not by an AI working through MCP (add_steps, whose default source is « ia »).
    pub fn by_person(&self) -> bool {
        self.received || self.source == SOURCE_PERSON || self.source == SOURCE_ACCEPTED
    }
}

/// Longest title of a step received from another computer.
pub const RECEIVED_STEP_MAX: usize = 300;

/// Longest message of a team discussion received from another computer (characters).
pub const RECEIVED_CHAT_MAX: usize = 4000;
/// Longest name of its author.
pub const RECEIVED_AUTHOR_MAX: usize = 60;

/// Whether the merge of a shared project takes their copy of a step over ours: the most
/// recent edit wins, except when both copies hold a live reservation by two different
/// computers (two AIs took the step before seeing each other): then the earlier one wins on
/// both sides, and the other AI sees it lost at its next get_project_state.
fn takes_theirs(ours: &Step, theirs: &Step) -> bool {
    let now = now_ms();
    let live = |s: &Step| {
        s.status == StepStatus::Doing
            && !s.held_by.is_empty()
            && now < s.held_at.saturating_add(HOLD_MS)
    };
    if live(ours) && live(theirs) && ours.held_by != theirs.held_by {
        return theirs.hold_key() < ours.hold_key();
    }
    theirs.updated > ours.updated
}

/// How long an AI's reservation of a step lasts without news: past that, the AI is taken as
/// gone (stopped, computer off) and another may take the step.
pub const HOLD_MS: u64 = 3 * 3600 * 1000;

impl Step {
    /// Reserved by an AI on another computer, and still within `HOLD_MS` at `now`.
    pub fn held_elsewhere(&self, me: &str, now: u64) -> bool {
        self.status == StepStatus::Doing
            && !self.held_by.is_empty()
            && self.held_by != me
            && now < self.held_at.saturating_add(HOLD_MS)
    }

    /// Like `held_elsewhere`, but only for another *computer*: an AI of this computer
    /// (`holder_id` = computer id, then `#` and the AI's name) doesn't count.
    pub fn held_by_other_computer(&self, computer: &str, now: u64) -> bool {
        self.status == StepStatus::Doing
            && !self.held_by.is_empty()
            && self.held_by.split('#').next() != Some(computer)
            && now < self.held_at.saturating_add(HOLD_MS)
    }

    /// Order of two reservations of one step: the earlier wins, the computer id breaks a tie,
    /// so both copies of a shared project pick the same AI.
    fn hold_key(&self) -> (u64, &str) {
        (self.held_at, self.held_by.as_str())
    }
}

/// A conversation or coding session that worked on the project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRef {
    /// "claude-code", "codex", "gemini-cli", "claude-web", "chatgpt"…
    pub tool: String,
    /// The tool's own session id (used to resume it).
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// Where the session is kept (a JSONL file) or its web address.
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub started: u64,
    #[serde(default)]
    pub last_activity: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Project {
    /// The project's code folder (absolute path, `~` allowed). Sessions started in it or
    /// below it belong to the project.
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub status: ProjectStatus,
    /// State of play, written by an AI (Markdown).
    #[serde(default)]
    pub summary: String,
    /// When the summary was last written, and by whom.
    #[serde(default)]
    pub summary_updated: u64,
    #[serde(default)]
    pub summary_by: String,
    #[serde(default)]
    pub steps: Vec<Step>,
    /// Steps deleted, and when: a shared project's copies agree on them (a step deleted on
    /// one computer doesn't come back from another).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub deleted_steps: std::collections::BTreeMap<String, u64>,
    #[serde(default)]
    pub sessions: Vec<SessionRef>,
    /// "Effacer l'historique": sessions whose last activity is before this are no longer
    /// shown with the project (their files are left alone; a resumed one comes back).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub sessions_cleared: u64,
    /// Last thing that happened on the project (step, summary, session…).
    #[serde(default)]
    pub last_activity: u64,
    /// Lecture seule: the project and its pages can't be edited in the window (the AIs
    /// still update the state of play and the steps through MCP).
    #[serde(default)]
    pub read_only: bool,
    /// Shared with the team (collaboration): only shared projects leave this computer.
    #[serde(default)]
    pub shared: bool,
    /// Autonomous sessions may land their work on the main branch: at the end of a
    /// session, its copy is merged into main by itself (Orchestration page).
    #[serde(default)]
    pub commit_to_main: bool,
    /// The budget per session chosen on the Orchestration page (% of the 5-hour window):
    /// kept from one session to the next, and across restarts (None: the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_limit: Option<f32>,
    /// Where the next session stops, in absolute % of the 5-hour window, as chosen on the
    /// Orchestration page (replaces `session_limit`, a share counted from the session's
    /// start, no longer asked); kept the same way (None: the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_ceiling: Option<f32>,
    /// The other launch choices of the Orchestration page (commits, endless and its end
    /// conditions), kept the same way (None: the defaults).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<LaunchPrefs>,
    /// "Regard extérieur": a short outside view asked to an AI (Markdown), by whom, when.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
    /// Code project or not, forced by the person (None: detected from the folder, see
    /// `shows_git_card`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<bool>,
    /// « Veille automatique » (Veille · CVE card): the dependencies are checked once a day
    /// while Orchestra is open, and an important vulnerability not handled yet starts a fix
    /// session by itself. Decided on this computer only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cve_watch: bool,
    /// When the last check taken into account by the automatic watch was made (ms).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cve_seen: u64,
    /// The example project the tutorial walks through (see `Workspace::create_example_project`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub example: bool,
}

/// How the next autonomous session of a project starts, as last chosen by the person.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LaunchPrefs {
    #[serde(default)]
    pub commits: bool,
    #[serde(default)]
    pub endless: bool,
    #[serde(default)]
    pub end: crate::model::EndConditions,
    /// The AI the sessions are started with (None: Claude Code).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<crate::auto::Agent>,
    /// The model the person imposed for this project's sessions (empty: Orchestra chooses
    /// the cheapest able to do the step). A step's own model wins over it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// The economy slider (Économe / Équilibré / Qualité); Balanced isn't written.
    #[serde(default, skip_serializing_if = "is_balanced")]
    pub saving: crate::model::Saving,
    /// « IA : Automatique »: at each start Orchestra picks the AI (and its model) among those
    /// with room under their limits, the cheapest able to do the step; `agent` is then only
    /// the fallback. Choices kept before it existed read false: their AI stays theirs.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_agent: bool,
}

fn is_balanced(s: &crate::model::Saving) -> bool {
    *s == crate::model::Saving::Balanced
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Review {
    pub text: String,
    pub by: String,
    pub at: u64,
}

/// A page as it may leave this computer: a project's page without what only this
/// computer knows (`Project::strip_local`); the others as they are.
pub fn shareable(p: &crate::model::Page) -> crate::model::Page {
    let mut p = p.clone();
    if let Some(x) = p.project.as_mut() {
        x.strip_local();
    }
    p
}

impl Project {
    /// Remove what only this computer decides or knows: the code folder (a path of this
    /// computer, with its user name), « sur main », the launch choices and the AI sessions
    /// (their files and commands). Done before a shared project leaves this computer, and
    /// on a project received new here.
    pub fn strip_local(&mut self) {
        self.folder.clear();
        self.commit_to_main = false;
        self.session_limit = None;
        self.session_ceiling = None;
        self.launch = None;
        for s in &mut self.steps {
            s.model.clear();
            s.agent = None;
        }
        self.sessions.clear();
        self.sessions_cleared = 0;
        self.cve_watch = false;
        self.cve_seen = 0;
    }

    /// A project received from another person, with what only this computer decides put
    /// back: `ours` is our copy before the merge (None for a project new here).
    ///
    /// - the code folder: theirs would make our AIs work in a folder of their choosing;
    /// - « sur main »: lets a session merge its work into main by itself;
    /// - the AI sessions (files and commands of their computer), read only.
    ///
    /// The steps are shared, but merged rather than taken (`merge_steps`): ours stay, theirs
    /// come in marked as received. A project new here takes its steps that way too.
    pub fn keep_local(&mut self, ours: Option<&Project>) {
        match ours {
            Some(o) => {
                self.folder = o.folder.clone();
                self.steps = o.steps.clone();
                self.deleted_steps = o.deleted_steps.clone();
                self.commit_to_main = o.commit_to_main;
                self.session_limit = o.session_limit;
                self.session_ceiling = o.session_ceiling;
                self.launch = o.launch.clone();
                self.read_only = o.read_only;
                self.sessions = o.sessions.clone();
                self.sessions_cleared = o.sessions_cleared;
                self.cve_watch = o.cve_watch;
                self.cve_seen = o.cve_seen;
            }
            None => {
                let theirs = std::mem::take(&mut self.steps);
                let tombs = std::mem::take(&mut self.deleted_steps);
                self.strip_local();
                self.merge_steps(&theirs, &tombs);
            }
        }
    }

    /// Bring another copy's steps into ours, by step id: each step keeps its most recent
    /// edit, steps new on their side come in, steps deleted on either side stay deleted.
    /// What comes in is cleaned (one line, `RECEIVED_STEP_MAX` characters at most) and
    /// marked `received`; a step already here keeps its own mark. Returns true when ours
    /// changed.
    pub fn merge_steps(
        &mut self,
        theirs: &[Step],
        their_deleted: &std::collections::BTreeMap<String, u64>,
    ) -> bool {
        let mut changed = false;
        for (id, t) in their_deleted {
            let e = self.deleted_steps.entry(id.clone()).or_insert(*t);
            if *t > *e {
                *e = *t;
            }
        }
        let clean = |s: &Step| crate::richtext::step_title(&s.title, RECEIVED_STEP_MAX);
        for ts in theirs {
            if ts.id.trim().is_empty() || clean(ts).is_empty() {
                continue;
            }
            match self.steps.iter_mut().find(|s| s.id == ts.id) {
                Some(s) if takes_theirs(s, ts) => {
                    s.title = clean(ts);
                    s.status = ts.status;
                    s.updated = ts.updated;
                    s.held_by = ts.held_by.clone();
                    s.held_at = ts.held_at;
                    changed = true;
                }
                Some(_) => {}
                None => {
                    let mut s = ts.clone();
                    s.title = clean(ts);
                    s.received = true;
                    // Decided on each computer: another's AIs and models aren't ours.
                    s.model.clear();
                    s.agent = None;
                    self.steps.push(s);
                    changed = true;
                }
            }
        }
        let tombs = &self.deleted_steps;
        let before = self.steps.len();
        self.steps
            .retain(|s| tombs.get(&s.id).is_none_or(|t| s.updated > *t));
        changed | (before != self.steps.len())
    }

    /// Delete one step, remembered so that a shared copy doesn't bring it back.
    pub fn remove_step(&mut self, id: &str) -> bool {
        let before = self.steps.len();
        self.steps.retain(|s| s.id != id);
        if self.steps.len() == before {
            return false;
        }
        self.deleted_steps.insert(id.to_string(), now_ms());
        self.touch();
        true
    }

    pub fn new(folder: impl Into<String>) -> Self {
        Project {
            folder: folder.into(),
            status: ProjectStatus::Active,
            last_activity: now_ms(),
            ..Default::default()
        }
    }

    pub fn touch(&mut self) {
        self.last_activity = now_ms();
    }

    pub fn add_step(&mut self, title: &str, source: &str) -> String {
        let t = now_ms();
        let s = Step {
            id: new_id(),
            title: crate::richtext::step_title(title, STEP_MAX),
            status: StepStatus::Todo,
            source: source.to_string(),
            created: t,
            updated: t,
            received: false,
            held_by: String::new(),
            held_at: 0,
            model: String::new(),
            agent: None,
        };
        let id = s.id.clone();
        self.steps.push(s);
        self.touch();
        id
    }

    pub fn set_step_status(&mut self, id: &str, status: StepStatus) -> bool {
        let Some(s) = self.steps.iter_mut().find(|s| s.id == id) else {
            return false;
        };
        if s.status != status {
            s.status = status;
            s.updated = now_ms();
            self.last_activity = s.updated;
            if status != StepStatus::Doing {
                s.held_by.clear();
                s.held_at = 0;
            }
        }
        true
    }

    /// Impose `model` for step `id` (empty: Orchestra chooses again). False: no such step.
    pub fn set_step_model(&mut self, id: &str, model: &str) -> bool {
        let Some(s) = self.steps.iter_mut().find(|s| s.id == id) else {
            return false;
        };
        let model = model.trim();
        if s.model != model {
            s.model = model.to_string();
            self.touch();
        }
        true
    }

    /// Assign `agent` to step `id` (None: the project's AI again). False: no such step.
    pub fn set_step_agent(&mut self, id: &str, agent: Option<crate::auto::Agent>) -> bool {
        let Some(s) = self.steps.iter_mut().find(|s| s.id == id) else {
            return false;
        };
        if s.agent != agent {
            s.agent = agent;
            self.touch();
        }
        true
    }

    /// An AI of computer `me` takes step `id`: « doing », reserved for `me`. Refused while an
    /// AI of another computer holds it (see `Step::held_elsewhere`); taking it again renews
    /// the reservation. Err holds who has it, for the AI.
    pub fn claim_step(&mut self, id: &str, me: &str) -> Result<(), String> {
        let now = now_ms();
        let Some(s) = self.steps.iter_mut().find(|s| s.id == id) else {
            return Err(format!("no step {id}"));
        };
        if s.held_elsewhere(me, now) {
            let mins = (now - s.held_at) / 60_000;
            return Err(crate::trf!(
                "l'étape « {} » est déjà en cours, réservée il y a {mins} min par une IA d'un autre ordinateur du projet partagé : ne la fais pas, prends une autre étape ouverte",
                "the step « {} » is already in progress, reserved {mins} min ago by an AI on another computer of the shared project: don't do it, take another open step",
                s.title
            ));
        }
        s.status = StepStatus::Doing;
        s.held_by = me.to_string();
        s.held_at = now;
        s.updated = now;
        self.last_activity = now;
        Ok(())
    }

    /// The state of play. Any AI's Markdown is cleaned (see richtext.rs) and kept short.
    pub fn set_summary(&mut self, text: &str, by: &str) {
        self.summary = crate::richtext::clean(text, SUMMARY_MAX);
        self.summary_by = by.to_string();
        self.summary_updated = now_ms();
        self.last_activity = self.summary_updated;
    }

    /// Steps added since the state of play was written (0 while none is written).
    pub fn steps_since_summary(&self) -> usize {
        if self.summary.trim().is_empty() || self.summary_updated == 0 {
            return 0;
        }
        self.steps
            .iter()
            .filter(|s| s.created > self.summary_updated)
            .count()
    }

    /// The line saying the state of play is out of date, if steps were added since.
    pub fn stale_summary_note(&self) -> Option<String> {
        match self.steps_since_summary() {
            0 => None,
            1 => Some(
                crate::tr!(
                    "Peut-être périmé : 1 étape ajoutée depuis son écriture.",
                    "May be out of date: 1 step added since it was written."
                )
                .to_string(),
            ),
            n => Some(crate::trf!(
                "Peut-être périmé : {n} étapes ajoutées depuis son écriture.",
                "May be out of date: {n} steps added since it was written."
            )),
        }
    }

    /// (done, total)
    /// Clear the sessions history: forget the linked sessions and hide the ones found on
    /// disk up to now. Nothing is deleted from disk.
    pub fn clear_sessions(&mut self) {
        self.sessions.clear();
        self.sessions_cleared = now_ms();
    }

    /// Whether a session last active at `last_activity` still shows with the project.
    pub fn session_shown(&self, last_activity: u64) -> bool {
        last_activity > self.sessions_cleared
    }

    /// Delete every finished step; returns how many went.
    /// Move step `id` just before step `before`, or, with None, just after the last step not
    /// yet done (the end of the open list, as shown). False when nothing moved.
    pub fn move_step(&mut self, id: &str, before: Option<&str>) -> bool {
        if before == Some(id) {
            return false;
        }
        let Some(from) = self.steps.iter().position(|s| s.id == id) else {
            return false;
        };
        if before.is_some_and(|b| !self.steps.iter().any(|s| s.id == b)) {
            return false;
        }
        let old: Vec<String> = self.steps.iter().map(|s| s.id.clone()).collect();
        let s = self.steps.remove(from);
        let to = match before {
            Some(b) => self
                .steps
                .iter()
                .position(|s| s.id == b)
                .unwrap_or(self.steps.len()),
            None => self
                .steps
                .iter()
                .rposition(|s| s.status != StepStatus::Done)
                .map_or(0, |i| i + 1),
        };
        self.steps.insert(to, s);
        let moved = self
            .steps
            .iter()
            .map(|s| s.id.as_str())
            .ne(old.iter().map(String::as_str));
        if moved {
            self.touch();
        }
        moved
    }

    /// Put the steps `ids` in that order, in the places they already take: the other
    /// steps don't move. False when nothing moved.
    pub fn reorder_steps(&mut self, ids: &[String]) -> bool {
        let slots: Vec<usize> = (0..self.steps.len())
            .filter(|&i| ids.contains(&self.steps[i].id))
            .collect();
        let wanted: Vec<Step> = ids
            .iter()
            .filter_map(|id| self.steps.iter().find(|s| s.id == *id).cloned())
            .collect();
        // An unknown or repeated id: nothing is touched.
        if wanted.len() != ids.len() || slots.len() != ids.len() {
            return false;
        }
        let moved = slots.iter().zip(&wanted).any(|(&i, s)| self.steps[i].id != s.id);
        if moved {
            for (i, s) in slots.into_iter().zip(wanted) {
                self.steps[i] = s;
            }
            self.touch();
        }
        moved
    }

    pub fn remove_done_steps(&mut self) -> usize {
        let before = self.steps.len();
        let t = now_ms();
        for s in self.steps.iter().filter(|s| s.status == StepStatus::Done) {
            self.deleted_steps.insert(s.id.clone(), t);
        }
        self.steps.retain(|s| s.status != StepStatus::Done);
        let n = before - self.steps.len();
        if n > 0 {
            self.touch();
        }
        n
    }

    pub fn progress(&self) -> (usize, usize) {
        (
            self.steps
                .iter()
                .filter(|s| s.status == StepStatus::Done)
                .count(),
            self.steps.len(),
        )
    }

    /// The step in progress, else the first one still to do.
    pub fn next_step(&self) -> Option<&Step> {
        self.next_step_for("", 0)
    }

    /// The next step for an AI of computer `me` at `now`: those reserved by another
    /// computer's AI are skipped. (`next_step` skips none.)
    pub fn next_step_for(&self, me: &str, now: u64) -> Option<&Step> {
        let free = |s: &&Step| me.is_empty() || !s.held_elsewhere(me, now);
        self.steps
            .iter()
            .filter(free)
            .find(|s| s.status == StepStatus::Doing)
            .or_else(|| {
                self.steps
                    .iter()
                    .filter(free)
                    .find(|s| s.status == StepStatus::Todo)
            })
    }

    /// The AI that should run the next step: the one assigned to it, else `default`.
    pub fn agent_for_next(&self, default: crate::auto::Agent, me: &str, now: u64) -> crate::auto::Agent {
        self.next_step_for(me, now).and_then(|s| s.agent).unwrap_or(default)
    }

    /// Add a session, or refresh it if already known (same tool and id).
    pub fn link_session(&mut self, mut s: SessionRef) {
        s.title = crate::richtext::one_line(&s.title, 120);
        match self
            .sessions
            .iter_mut()
            .find(|x| x.tool == s.tool && x.id == s.id)
        {
            Some(old) => {
                if !s.title.is_empty() {
                    old.title = s.title;
                }
                if !s.location.is_empty() {
                    old.location = s.location;
                }
                old.started = if old.started == 0 {
                    s.started
                } else {
                    old.started.min(s.started.max(1))
                };
                old.last_activity = old.last_activity.max(s.last_activity);
            }
            None => self.sessions.push(s),
        }
        self.sessions
            .sort_by_key(|s| std::cmp::Reverse(s.last_activity));
        let latest = self.sessions.first().map(|s| s.last_activity).unwrap_or(0);
        self.last_activity = self.last_activity.max(latest);
    }
}

/// `~/x` → home/x; removes `.` and `..` and trailing slashes, without touching the disk.
pub fn normalize_folder(folder: &str) -> PathBuf {
    let f = folder.trim();
    let raw = match f.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => dirs::home_dir()
            .unwrap_or_default()
            .join(rest.trim_start_matches('/')),
        _ => PathBuf::from(f),
    };
    let mut out = PathBuf::new();
    for c in raw.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Files whose presence at the root of a folder marks a code project.
const CODE_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "requirements.txt",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "Gemfile",
    "composer.json",
    "Makefile",
    "CMakeLists.txt",
    "Package.swift",
    "pubspec.yaml",
    "mix.exs",
    "deno.json",
    "tsconfig.json",
    "Dockerfile",
];

/// Whether a project with this folder gets the git (Versions) card: code projects, and
/// projects without a usable folder yet (the card then asks for one). A folder of documents,
/// school work or paperwork doesn't.
pub fn shows_git_card(folder: &str) -> bool {
    let folder = folder.trim();
    if folder.is_empty() {
        return true;
    }
    let dir = normalize_folder(folder);
    !dir.is_dir() || is_code_folder(&dir)
}

/// Whether a project is treated as code: what the person forced, else detected.
pub fn code_kind(folder: &str, forced: Option<bool>) -> bool {
    forced.unwrap_or_else(|| shows_git_card(folder))
}

/// Source file extensions counted when no marker is found.
const CODE_EXTS: &[&str] = &[
    "rs", "py", "js", "jsx", "ts", "tsx", "go", "java", "kt", "swift", "c", "h", "cpp", "hpp",
    "cc", "cs", "rb", "php", "dart", "ex", "exs", "scala", "lua", "sh", "vue", "svelte", "m", "mm",
    "zig",
];

/// Whether `dir` looks like a code project: a known marker at its root (`.git`, `Cargo.toml`,
/// `package.json`…), or at least 3 source files in it or one level down (hidden folders and
/// dependency folders skipped). A missing folder is not code. Reads at most a few hundred entries.
pub fn is_code_folder(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    if CODE_MARKERS.iter().any(|m| dir.join(m).exists()) {
        return true;
    }
    let is_src = |p: &Path| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| CODE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
    };
    let mut found = 0;
    let mut seen = 0;
    let mut subdirs = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        seen += 1;
        if seen > 400 {
            break;
        }
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !name.starts_with('.')
                && !matches!(
                    name.as_str(),
                    "node_modules" | "target" | "vendor" | "venv" | "__pycache__"
                )
            {
                subdirs.push(p);
            }
        } else if is_src(&p) {
            found += 1;
        }
    }
    for sub in subdirs.into_iter().take(40) {
        if found >= 3 {
            break;
        }
        for e in std::fs::read_dir(&sub)
            .into_iter()
            .flatten()
            .flatten()
            .take(200)
        {
            if e.path().is_file() && is_src(&e.path()) {
                found += 1;
            }
        }
    }
    found >= 3
}

impl Workspace {
    /// Pages that are projects (not in the trash), most recent activity first.
    pub fn projects(&self) -> Vec<&Page> {
        let mut v: Vec<&Page> = self
            .pages
            .iter()
            .filter(|p| !p.trashed && p.project.is_some())
            .collect();
        v.sort_by(|a, b| {
            let la = a
                .project
                .as_ref()
                .map(|x| x.last_activity)
                .unwrap_or(0)
                .max(a.updated);
            let lb = b
                .project
                .as_ref()
                .map(|x| x.last_activity)
                .unwrap_or(0)
                .max(b.updated);
            lb.cmp(&la)
        });
        v
    }

    /// Projects in « Lancer une session »: most recent real activity first (steps, sessions,
    /// state of play), without the page's own edit time, which a change of their launch
    /// controls (« arrêt à » %, commits, sans fin) bumps: the project being set would jump
    /// to the top under the pointer and swap places with the others.
    pub fn launch_projects(&self) -> Vec<&Page> {
        let mut v: Vec<&Page> = self
            .pages
            .iter()
            .filter(|p| !p.trashed && p.project.is_some())
            .collect();
        // Stable: equal activity keeps the pages' own order.
        v.sort_by_key(|p| std::cmp::Reverse(p.project.as_ref().map_or(0, |x| x.last_activity)));
        v
    }

    /// Create a project page (top level) with an introduction block.
    pub fn create_project(&mut self, title: &str, folder: &str) -> String {
        let id = self.create_page(title, None);
        if let Some(p) = self.page_mut(&id) {
            p.icon = "🚀".into();
            p.project = Some(Project::new(folder.trim()));
            p.blocks = vec![crate::model::Block::new(
                crate::model::BlockKind::Paragraph,
                "",
            )];
        }
        id
    }

    /// Create the example project the tutorial is built on: no folder, a state of play and a
    /// few steps in each status, so that every card has something to show. Marked `example`.
    pub fn create_example_project(&mut self) -> String {
        use crate::model::{Block, BlockKind};
        let id = self.create_project(&crate::tr!("Projet exemple", "Example project"), "");
        if let Some(p) = self.page_mut(&id) {
            p.icon = "🎓".into();
            p.blocks = vec![Block::new(
                BlockKind::Paragraph,
                crate::tr!(
                    "Un projet pour découvrir Orchestra : tu peux le modifier ou le supprimer.",
                    "A project to discover Orchestra: you can edit or delete it."
                ),
            )];
            if let Some(x) = p.project.as_mut() {
                x.example = true;
                let done = x.add_step(&crate::tr!("Exemple : créer le projet", "Example: create the project"), "Orchestra");
                x.set_step_status(&done, StepStatus::Done);
                let doing = x.add_step(&crate::tr!("Exemple : découvrir les étapes", "Example: discover the steps"), "Orchestra");
                x.set_step_status(&doing, StepStatus::Doing);
                x.add_step(&crate::tr!("Exemple : lancer une session autonome", "Example: start an autonomous session"), "Orchestra");
                // After the steps: it must not show as out of date.
                x.set_summary(
                    &crate::tr!(
                        "### Fait\n- Le projet exemple est créé.\n\n### En cours\n- Découvrir les étapes.\n\n### Bloquants\n- Aucun.",
                        "### Done\n- The example project is created.\n\n### In progress\n- Discovering the steps.\n\n### Blockers\n- None."
                    ),
                    "Orchestra",
                );
            }
        }
        id
    }

    /// The project the tutorial walks through: the example project, or, if it was deleted, the
    /// first other project that exists (not in the trash). None when there is no project.
    pub fn tutorial_project(&self) -> Option<String> {
        let live = || self.pages.iter().filter(|p| !p.trashed && p.project.is_some());
        live()
            .find(|p| p.project.as_ref().is_some_and(|x| x.example))
            .or_else(|| live().next())
            .map(|p| p.id.clone())
    }

    /// `tutorial_project`, or, when no project exists at all, a new example project (so the
    /// tutorial always has something to show).
    pub fn ensure_tutorial_project(&mut self) -> String {
        match self.tutorial_project() {
            Some(id) => id,
            None => self.create_example_project(),
        }
    }

    /// The project a page belongs to (itself, or its nearest parent that is a project).
    pub fn project_of(&self, id: &str) -> Option<&Page> {
        let mut cur = self.page(id);
        for _ in 0..64 {
            let p = cur?;
            if p.project.is_some() {
                return Some(p);
            }
            cur = p.parent.as_deref().and_then(|pid| self.page(pid));
        }
        None
    }

    /// Ids of every page that belongs to a shared project (the project page and below).
    pub fn shared_ids(&self) -> std::collections::HashSet<String> {
        if self.whole_shared {
            return self.pages.iter().map(|p| p.id.clone()).collect();
        }
        let mut out = std::collections::HashSet::new();
        for p in self
            .pages
            .iter()
            .filter(|p| p.project.as_ref().is_some_and(|x| x.shared))
        {
            out.insert(p.id.clone());
            out.extend(self.descendants(&p.id));
        }
        out
    }

    /// Messages of the team discussions another person sent: only those of projects shared
    /// here, new ones, cut to a reasonable size (`RECEIVED_CHAT_MAX` characters, the author
    /// on one short line). Returns how many came in.
    pub fn accept_chat(&mut self, msgs: impl IntoIterator<Item = crate::model::ChatMsg>) -> usize {
        let shared = self.shared_ids();
        let mut n = 0;
        for mut m in msgs {
            if m.id.is_empty() || !shared.contains(&m.project) || self.chat.contains_key(&m.id) {
                continue;
            }
            if m.text.chars().count() > RECEIVED_CHAT_MAX {
                m.text = m.text.chars().take(RECEIVED_CHAT_MAX).collect::<String>() + "…";
            }
            m.author = crate::richtext::one_line(&m.author, RECEIVED_AUTHOR_MAX);
            self.chat.insert(m.id.clone(), m);
            n += 1;
        }
        n
    }

    /// Of the pages another person sent, those that may come in: only pages of shared
    /// projects, never one of ours that isn't shared, nor one placed under a private page.
    /// - a page we have: only if it is shared here, and it stays under a shared page;
    /// - a new project page: only a shared one, at the top level;
    /// - a new page: only under a shared page (ours, or one accepted in the same batch).
    pub fn accept_remote(&self, pages: Vec<Page>) -> Vec<Page> {
        // A team space: everything in it is the team's.
        if self.whole_shared {
            return pages;
        }
        let shared = self.shared_ids();
        let known: std::collections::HashMap<&str, &Option<String>> = self
            .pages
            .iter()
            .map(|p| (p.id.as_str(), &p.parent))
            .collect();
        let mut allowed = shared.clone();
        let mut rest = pages;
        let mut out = vec![];
        // Pages may come in any order: take them while one more fits.
        loop {
            let before = out.len();
            let mut left = vec![];
            for p in rest {
                let ours = known.get(p.id.as_str());
                // Where it already is here (a shared project may sit in a private folder),
                // or under a shared page.
                let parent_ok = ours.is_some_and(|par| **par == p.parent)
                    || match &p.parent {
                        None => p.project.as_ref().is_some_and(|x| x.shared),
                        Some(par) => allowed.contains(par),
                    };
                let page_ok = if ours.is_some() {
                    shared.contains(&p.id)
                } else {
                    // A new page inside a shared project can't be a project of its own.
                    p.parent.is_none() || p.project.is_none()
                };
                if parent_ok && page_ok && p.project.as_ref().is_none_or(|x| x.shared) {
                    allowed.insert(p.id.clone());
                    out.push(p);
                } else {
                    left.push(p);
                }
            }
            rest = left;
            if out.len() == before || rest.is_empty() {
                break;
            }
        }
        out
    }

    /// Of the deletions another person sent, those that may apply here: pages of shared
    /// projects, and ids we don't have (a page they deleted before it reached us). Never
    /// one of our private pages, even if its id is known to them.
    pub fn accept_remote_deletions(
        &self,
        deleted: std::collections::BTreeMap<String, u64>,
    ) -> std::collections::BTreeMap<String, u64> {
        if self.whole_shared {
            return deleted;
        }
        let shared = self.shared_ids();
        deleted
            .into_iter()
            .filter(|(id, _)| shared.contains(id) || self.page(id).is_none())
            .collect()
    }

    /// What may leave this computer: the shared projects, their pages and discussions,
    /// without what is this computer's own (`shareable`).
    pub fn shared_copy(&self) -> Workspace {
        let ids = self.shared_ids();
        Workspace {
            pages: self
                .pages
                .iter()
                .filter(|p| ids.contains(&p.id))
                .map(shareable)
                .collect(),
            deleted: self.deleted.clone(),
            chat: self
                .chat
                .iter()
                .filter(|(_, m)| ids.contains(&m.project))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            ..Default::default()
        }
    }

    /// A private copy of a project and its pages, with new ids, that no longer follows the
    /// shared one. Returns the new project's id.
    pub fn local_copy(&mut self, id: &str) -> Option<String> {
        let mut ids = vec![id.to_string()];
        ids.extend(self.descendants(id));
        let mut map = std::collections::HashMap::new();
        for old in &ids {
            map.insert(old.clone(), new_id());
        }
        let t = now_ms();
        let mut copies = vec![];
        for old in &ids {
            let mut p = self.page(old)?.clone();
            p.id = map[old].clone();
            p.parent = p.parent.and_then(|par| map.get(&par).cloned());
            if old == id {
                p.title = format!("{} (copie locale)", p.display_title());
                if let Some(pr) = p.project.as_mut() {
                    pr.shared = false;
                }
            }
            for b in p.blocks.iter_mut() {
                b.id = new_id();
                if let Some(n) = map.get(&b.extra) {
                    b.extra = n.clone();
                }
            }
            p.deleted_blocks.clear();
            p.created = t;
            p.updated = t;
            copies.push(p);
        }
        self.pages.extend(copies);
        Some(map[id].clone())
    }

    /// The page, or one of its parents, is a project in "lecture seule".
    pub fn is_locked(&self, id: &str) -> bool {
        let mut cur = self.page(id);
        for _ in 0..64 {
            let Some(p) = cur else { return false };
            if let Some(pr) = &p.project {
                return pr.read_only;
            }
            cur = p.parent.as_deref().and_then(|pid| self.page(pid));
        }
        false
    }

    /// Find a project by id or title (like `find_page`), only among projects.
    pub fn find_project(&self, key: &str) -> Option<&Page> {
        if let Some(p) = self.page(key).filter(|p| p.project.is_some()) {
            return Some(p);
        }
        let k = key.trim().to_lowercase();
        let list = self.projects();
        list.iter()
            .find(|p| p.title.to_lowercase() == k)
            .or_else(|| list.iter().find(|p| p.title.to_lowercase().contains(&k)))
            .copied()
    }

    /// The project whose folder holds `path` (the deepest one wins).
    pub fn project_for_path(&self, path: &Path) -> Option<&Page> {
        let path = normalize_folder(&path.to_string_lossy());
        self.projects()
            .into_iter()
            .filter_map(|p| {
                let f = p.project.as_ref()?.folder.trim();
                if f.is_empty() {
                    return None;
                }
                let f = normalize_folder(f);
                path.starts_with(&f).then(|| (f.components().count(), p))
            })
            .max_by_key(|(n, _)| *n)
            .map(|(_, p)| p)
    }
}

/// The project as Markdown, for the AIs: status, folder, summary, steps, sessions.
pub fn project_to_markdown(page: &Page) -> String {
    let Some(pr) = &page.project else {
        return String::new();
    };
    let mut out = format!("# {}\n\n", page.display_title());
    out.push_str(&crate::trf!(
        "- id : {}\n- statut : {}\n",
        "- id: {}\n- status: {}\n",
        page.id,
        pr.status.label()
    ));
    if !pr.folder.is_empty() {
        out.push_str(&crate::trf!(
            "- dossier : {}\n",
            "- folder: {}\n",
            pr.folder
        ));
    }
    // Code or not, so that the AIs adapt (forced by the person, or detected).
    if pr.code.is_some() || !pr.folder.trim().is_empty() {
        let code = code_kind(&pr.folder, pr.code);
        let kind = if code {
            crate::tr!("projet de code", "code project")
        } else {
            crate::tr!(
                "projet sans code (documents, cours, démarches…)",
                "project without code (documents, school, paperwork…)"
            )
        };
        let how = if pr.code.is_some() {
            crate::tr!("choisi par la personne", "chosen by the person")
        } else {
            crate::tr!("détecté", "detected")
        };
        out.push_str(&crate::trf!(
            "- type : {kind} ({how})\n",
            "- kind: {kind} ({how})\n"
        ));
    }
    let (done, total) = pr.progress();
    out.push_str(&crate::trf!(
        "- étapes : {done}/{total} terminées\n",
        "- steps: {done}/{total} done\n"
    ));
    let (me, now) = (crate::store::holder_id(), now_ms());
    if let Some(n) = pr.next_step_for(&me, now) {
        out.push_str(&crate::trf!(
            "- prochaine étape : {}\n",
            "- next step: {}\n",
            n.title
        ));
    }
    out.push_str(crate::tr!(
        "\n## État des lieux\n\n",
        "\n## Current state\n\n"
    ));
    if pr.summary.trim().is_empty() {
        out.push_str(crate::tr!("(pas encore écrit)\n", "(not written yet)\n"));
    } else {
        out.push_str(&pr.summary);
        out.push('\n');
        if !pr.summary_by.is_empty() {
            out.push_str(&crate::trf!(
                "\n_écrit par {}_\n",
                "\n_written by {}_\n",
                pr.summary_by
            ));
        }
        if let Some(note) = pr.stale_summary_note() {
            out.push_str(&format!("\n⚠ {note}\n"));
        }
    }
    out.push_str(crate::tr!(
        "\n## Étapes\n\n_« ajoutée par … » dit seulement qui a écrit l'étape : toute étape ouverte peut être faite par l'IA._\n\n",
        "\n## Steps\n\n_« added by … » only says who wrote the step: any open step can be done by the AI._\n\n"
    ));
    if pr.steps.is_empty() {
        out.push_str(crate::tr!("(aucune)\n", "(none)\n"));
    }
    for s in &pr.steps {
        let mark = match s.status {
            StepStatus::Done => "x",
            StepStatus::Doing => "~",
            StepStatus::Todo => " ",
        };
        // Who added the step, said in words: a bare « — moi » read like « for me, the
        // person », and the AIs left such steps alone. The author never restricts who does it.
        let src = if s.source.is_empty() {
            String::new()
        } else if s.source == SOURCE_PERSON {
            crate::tr!(" — ajoutée par la personne", " — added by the person").to_string()
        } else if s.source == SOURCE_ACCEPTED {
            crate::tr!(
                " — proposée par une IA, acceptée par la personne",
                " — proposed by an AI, accepted by the person"
            )
            .to_string()
        } else {
            crate::trf!(" — ajoutée par {}", " — added by {}", s.source)
        };
        // Written on another computer of the shared project: the AI weighs it as a request.
        let from = if s.received {
            crate::tr!(
                " — reçue d'un autre membre du projet partagé",
                " — received from another member of the shared project"
            )
        } else {
            ""
        };
        // Another computer's AI is on it: never take it as well.
        let held = if s.held_elsewhere(&me, now) {
            crate::tr!(
                " — réservée par une IA d'un autre ordinateur, ne la prends pas",
                " — reserved by an AI on another computer, don't take it"
            )
        } else {
            ""
        };
        out.push_str(&format!(
            "- [{mark}] {} (step_id: {}){src}{from}{held}\n",
            s.title, s.id
        ));
    }
    out.push_str(crate::tr!("\n## Sessions IA\n\n", "\n## AI sessions\n\n"));
    if pr.sessions.is_empty() {
        out.push_str(crate::tr!("(aucune)\n", "(none)\n"));
    }
    for s in &pr.sessions {
        let t = if s.title.is_empty() {
            crate::tr!("(sans titre)", "(untitled)")
        } else {
            &s.title
        };
        out.push_str(&format!("- {} · {} (session {})\n", s.tool, t, s.id));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Workspace;

    #[test]
    fn two_ais_of_one_computer_never_take_the_same_step() {
        let mut p = Project::default();
        let a = p.add_step("Une", "moi");
        let b = p.add_step("Deux", "moi");
        p.claim_step(&a, "pc#claude-code").unwrap();
        // The other AI of the same computer is refused, and is offered the next step.
        assert!(p.claim_step(&a, "pc#codex").is_err());
        assert_eq!(p.next_step_for("pc#codex", now_ms()).map(|s| s.id.clone()), Some(b.clone()));
        p.claim_step(&b, "pc#codex").unwrap();
        // The same AI takes its own step again (renewal).
        p.claim_step(&a, "pc#claude-code").unwrap();
        // For the screen, both are « here »: only another computer counts as elsewhere.
        let s = p.steps.iter().find(|s| s.id == a).unwrap();
        assert!(!s.held_by_other_computer("pc", now_ms()));
        assert!(s.held_by_other_computer("autre", now_ms()));
    }

    #[test]
    fn a_step_keeps_its_assigned_ai_on_this_computer_only() {
        use crate::auto::Agent;
        let mut pr = Project::default();
        let a = pr.add_step("première", "moi");
        assert_eq!(pr.steps[0].agent, None);
        assert!(pr.set_step_agent(&a, Some(Agent::Codex)));
        assert!(!pr.set_step_agent("nope", Some(Agent::Codex)));
        assert_eq!(pr.steps[0].agent, Some(Agent::Codex));
        // Written and read back; an old file without the field reads as None.
        let json = serde_json::to_string(&pr.steps[0]).unwrap();
        let back: Step = serde_json::from_str(&json).unwrap();
        assert_eq!(back.agent, Some(Agent::Codex));
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("agent");
        assert_eq!(serde_json::from_value::<Step>(v).unwrap().agent, None);
        // Leaving this computer, the assignment stays here.
        pr.strip_local();
        assert_eq!(pr.steps[0].agent, None);
    }

    #[test]
    fn the_next_step_decides_which_ai_launches() {
        use crate::auto::Agent;
        let mut pr = Project::default();
        let a = pr.add_step("première", "moi");
        pr.add_step("seconde", "moi");
        assert_eq!(pr.agent_for_next(Agent::ClaudeCode, "", 0), Agent::ClaudeCode);
        let b = pr.steps[1].id.clone();
        pr.set_step_agent(&b, Some(Agent::Codex));
        // The first open step has none: the project's AI.
        assert_eq!(pr.agent_for_next(Agent::ClaudeCode, "", 0), Agent::ClaudeCode);
        pr.set_step_agent(&a, Some(Agent::Vibe));
        assert_eq!(pr.agent_for_next(Agent::ClaudeCode, "", 0), Agent::Vibe);
        pr.steps[0].status = StepStatus::Done;
        assert_eq!(pr.agent_for_next(Agent::ClaudeCode, "", 0), Agent::Codex);
    }

    #[test]
    fn ai_steps_are_one_action_under_a_theme() {
        // Fitting, tidied: « Thème : action », first letter of the theme upper case.
        for (given, kept) in [
            ("Sécurité : échapper le nom du dossier (src/git.rs)", "Sécurité : échapper le nom du dossier (src/git.rs)"),
            ("interface: ajouter un bouton", "Interface : ajouter un bouton"),
            ("Mesure d'usage :garder la dernière valeur", "Mesure d'usage : garder la dernière valeur"),
            ("Docs: write the guide (v2.1. see notes)", "Docs : write the guide (v2.1. see notes)"),
            ("Quota : fixer le plafond à 50 % (src/app/orch_ui.rs: stop_reason)", "Quota : fixer le plafond à 50 % (src/app/orch_ui.rs: stop_reason)"),
        ] {
            assert_eq!(ai_step(given).as_deref(), Ok(kept), "{given}");
        }
        // Not fitting: no theme, a long theme, no action, two actions.
        for bad in [
            "Ajouter un bouton",
            "Il faudrait vraiment revoir tout ceci : ajouter un bouton",
            "Interface :",
            "Interface : ?!",
            "(src) : corriger",
            "Interface : ajouter un bouton ; le tester",
            "Interface : ajouter un bouton puis le tester",
            "Interface : ajouter un bouton. Ensuite le tester",
            "Tests: write it, then run it",
        ] {
            let e = ai_step(bad).unwrap_err();
            assert!(e.contains("Expected") && e.contains("Theme"), "{bad}: {e}");
        }
    }

    /// The tutorial's example project has no folder, is marked, stays private, and has steps
    /// in each status.
    #[test]
    fn example_project_is_marked_and_has_steps_in_each_status() {
        let mut ws = Workspace::default();
        let id = ws.create_example_project();
        let p = ws.page(&id).and_then(|p| p.project.as_ref()).unwrap();
        assert!(p.example && p.folder.is_empty() && !p.shared);
        assert!(!p.summary.is_empty());
        for st in [StepStatus::Todo, StepStatus::Doing, StepStatus::Done] {
            assert!(p.steps.iter().any(|s| s.status == st), "{st:?}");
        }
    }

    /// The tutorial uses the example project; once it is deleted (trashed), another existing
    /// project; with none left, nothing.
    #[test]
    fn tutorial_project_falls_back_to_another_project() {
        let mut ws = Workspace::default();
        assert_eq!(ws.tutorial_project(), None);
        let other = ws.create_project("Autre", "");
        assert_eq!(ws.tutorial_project(), Some(other.clone()));
        let ex = ws.create_example_project();
        assert_eq!(ws.tutorial_project(), Some(ex.clone()));
        ws.set_trashed(&ex, true);
        assert_eq!(ws.tutorial_project(), Some(other.clone()));
        ws.set_trashed(&other, true);
        assert_eq!(ws.tutorial_project(), None);
    }

    /// With no project left, the tutorial gets a new example project; otherwise it reuses one.
    #[test]
    fn tutorial_project_is_recreated_when_none_exists() {
        let mut ws = Workspace::default();
        let ex = ws.ensure_tutorial_project();
        assert!(ws.page(&ex).and_then(|p| p.project.as_ref()).is_some_and(|x| x.example));
        assert_eq!(ws.ensure_tutorial_project(), ex, "reused, not duplicated");
        ws.set_trashed(&ex, true);
        let again = ws.ensure_tutorial_project();
        assert_ne!(again, ex);
        assert!(ws.page(&again).and_then(|p| p.project.as_ref()).is_some_and(|x| x.example));
    }

    /// The budget per session is saved with the project, comes back on loading, and stays
    /// this computer's own when a shared copy comes in.
    #[test]
    fn the_session_budget_is_kept() {
        let prefs = LaunchPrefs {
            commits: true,
            endless: true,
            end: crate::model::EndConditions {
                hours: Some(6.0),
                turns: Some(12),
                ..Default::default()
            },
            agent: None, model: "opus".into(), saving: Default::default(), auto_agent: false,
        };
        let p = Project {
            session_limit: Some(95.0),
            launch: Some(prefs.clone()),
            ..Default::default()
        };
        let back: Project = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.session_limit, Some(95.0));
        assert_eq!(back.launch, Some(prefs.clone()));
        // An older file, without them: the defaults.
        let old: Project = serde_json::from_str("{}").unwrap();
        assert_eq!(old.session_limit, None);
        assert_eq!(old.launch, None);
        let mut theirs = Project {
            session_limit: Some(10.0),
            launch: Some(LaunchPrefs {
                commits: false,
                endless: false,
                end: Default::default(),
                agent: None, model: String::new(), saving: Default::default(), auto_agent: false,
            }),
            ..Default::default()
        };
        theirs.keep_local(Some(&p));
        assert_eq!(theirs.session_limit, Some(95.0));
        assert_eq!(theirs.launch, Some(prefs));
        let mut new_here = Project {
            session_limit: Some(10.0),
            launch: p.launch.clone(),
            ..Default::default()
        };
        new_here.keep_local(None);
        assert_eq!(new_here.session_limit, None);
        assert_eq!(new_here.launch, None);
    }

    #[test]
    fn code_folder_detection() {
        let base =
            std::env::temp_dir().join(format!("orch-codecheck-{}", uuid::Uuid::new_v4().simple()));
        let w = |p: &str| {
            let f = base.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, "x").unwrap();
        };
        // Missing folder: not code.
        assert!(!is_code_folder(&base.join("absent")));
        // School / admin documents: not code.
        w("ecole/memoire.docx");
        w("ecole/notes.md");
        w("ecole/sources/article.pdf");
        w("ecole/sources/script.py");
        assert!(!is_code_folder(&base.join("ecole")));
        // A marker at the root: code.
        w("rust/Cargo.toml");
        assert!(is_code_folder(&base.join("rust")));
        // No marker, but source files one level down: code.
        w("site/src/a.ts");
        w("site/src/b.ts");
        w("site/index.js");
        assert!(is_code_folder(&base.join("site")));
        // Sources hidden in a dependency folder don't count.
        w("deps/node_modules/a.js");
        w("deps/node_modules/b.js");
        w("deps/node_modules/c.js");
        assert!(!is_code_folder(&base.join("deps")));
        // The git card: code projects and projects without a usable folder, not documents.
        let s = |p: &str| base.join(p).display().to_string();
        assert!(shows_git_card(&s("rust")));
        assert!(!shows_git_card(&s("ecole")));
        assert!(shows_git_card(""));
        assert!(shows_git_card("  "));
        assert!(shows_git_card(&s("absent")));
        // Forced by the person: wins over the detection, both ways.
        assert!(code_kind(&s("ecole"), Some(true)));
        assert!(!code_kind(&s("rust"), Some(false)));
        assert!(code_kind(&s("rust"), None) && !code_kind(&s("ecole"), None));
        let mut p = Project::new("/tmp/x");
        assert!(
            !serde_json::to_string(&p).unwrap().contains("\"code\""),
            "auto is not written"
        );
        p.code = Some(false);
        let back: Project = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.code, Some(false));
        // The AIs read the kind in the state of play.
        let mut page = Page::new("École", None);
        page.project = Some(Project::new(s("ecole")));
        let md = project_to_markdown(&page);
        assert!(md.contains("sans code") || md.contains("without code"));
        assert!(md.contains("détecté") || md.contains("detected"));
        page.project.as_mut().unwrap().code = Some(true);
        let md = project_to_markdown(&page);
        assert!(
            (md.contains("projet de code") || md.contains("code project"))
                && (md.contains("choisi") || md.contains("chosen"))
        );
        page.project = Some(Project::new(""));
        assert!(
            !project_to_markdown(&page).contains("type :")
                && !project_to_markdown(&page).contains("kind:"),
            "no folder, nothing forced: no kind"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The author of a step is said in words: a bare « — moi » read as « kept for the
    /// person » and the AIs skipped those steps.
    #[test]
    fn step_author_reads_as_author_not_owner() {
        let mut page = Page::new("Auteurs", None);
        let mut p = Project::new("");
        let a = p.add_step("Tapée", SOURCE_PERSON);
        let b = p.add_step("Acceptée", SOURCE_ACCEPTED);
        let c = p.add_step("Par l'IA", "claude-code");
        page.project = Some(p);
        let md = project_to_markdown(&page);
        let line = |id: &str| {
            md.lines()
                .find(|l| l.contains(id))
                .unwrap_or_else(|| panic!("{md}"))
                .to_string()
        };
        let (la, lb, lc) = (line(&a), line(&b), line(&c));
        assert!(!la.ends_with("— moi") && !lb.ends_with("— IA"), "{md}");
        assert!(
            la.ends_with("ajoutée par la personne") || la.ends_with("added by the person"),
            "{la}"
        );
        assert!(
            lb.contains("acceptée par la personne") || lb.contains("accepted by the person"),
            "{lb}"
        );
        assert!(
            lc.ends_with("ajoutée par claude-code") || lc.ends_with("added by claude-code"),
            "{lc}"
        );
        assert!(
            md.contains("toute étape ouverte peut être faite par l'IA")
                || md.contains("any open step can be done by the AI"),
            "{md}"
        );
    }

    #[test]
    fn steps_progress_and_next() {
        let mut p = Project::new("/tmp/x");
        let a = p.add_step("Écrire le modèle", "moi");
        let b = p.add_step("Brancher MCP", "claude-code");
        assert_eq!(p.progress(), (0, 2));
        assert_eq!(p.next_step().unwrap().id, a);
        p.set_step_status(&b, StepStatus::Doing);
        assert_eq!(p.next_step().unwrap().id, b);
        p.set_step_status(&a, StepStatus::Done);
        assert_eq!(p.progress(), (1, 2));
        assert!(!p.set_step_status("nope", StepStatus::Done));
        assert_eq!(p.remove_done_steps(), 1);
        assert_eq!(p.progress(), (0, 1));
        assert_eq!(p.steps[0].id, b);
        assert_eq!(p.remove_done_steps(), 0);
    }

    #[test]
    fn steps_are_reordered() {
        let mut p = Project::new("");
        let ids: Vec<String> = ["A", "B", "C", "D"]
            .iter()
            .map(|t| p.add_step(t, "moi"))
            .collect();
        let order = |p: &Project| p.steps.iter().map(|s| s.title.as_str()).collect::<String>();
        p.set_step_status(&ids[3], StepStatus::Done); // D done: shown apart, after the open ones
        assert!(p.move_step(&ids[2], Some(&ids[0])));
        assert_eq!(order(&p), "CABD");
        // To the end of the open list: before the done step.
        assert!(p.move_step(&ids[2], None));
        assert_eq!(order(&p), "ABCD");
        assert!(!p.move_step(&ids[2], None), "already there");
        assert!(!p.move_step(&ids[1], Some(&ids[1])));
        assert!(!p.move_step("absent", None));
        assert!(!p.move_step(&ids[0], Some("absent")));
        assert_eq!(order(&p), "ABCD");
        // The next step follows the new order.
        p.move_step(&ids[1], Some(&ids[0]));
        assert_eq!(p.next_step().unwrap().title, "B");
    }

    #[test]
    fn steps_are_reordered_in_their_own_places() {
        let mut p = Project::new("/tmp/x");
        let ids: Vec<String> = ["A", "B", "C", "D"]
            .iter()
            .map(|t| p.add_step(t, SOURCE_PERSON))
            .collect();
        let order = |p: &Project| p.steps.iter().map(|s| s.title.as_str()).collect::<String>();
        // A and C swap; B and D stay where they are.
        assert!(p.reorder_steps(&[ids[2].clone(), ids[0].clone()]));
        assert_eq!(order(&p), "CBAD");
        assert!(!p.reorder_steps(&[ids[2].clone(), ids[0].clone()]), "already so");
        assert!(!p.reorder_steps(&[ids[0].clone(), "absent".into()]));
        assert!(!p.reorder_steps(&[ids[0].clone(), ids[0].clone()]));
        assert_eq!(order(&p), "CBAD");
    }

    #[test]
    fn clearing_sessions_hides_the_past_ones() {
        let mut p = Project::new("/tmp/x");
        p.link_session(SessionRef {
            tool: "claude-code".into(),
            id: "s1".into(),
            title: String::new(),
            location: String::new(),
            started: 1,
            last_activity: 1,
        });
        assert!(p.session_shown(1));
        p.clear_sessions();
        assert!(p.sessions.is_empty());
        assert!(!p.session_shown(1));
        assert!(p.session_shown(now_ms() + 1000));
    }

    #[test]
    fn launch_list_keeps_its_order_when_a_ceiling_changes() {
        let mut ws = Workspace::default();
        let a = ws.create_project("A", "/a");
        let b = ws.create_project("B", "/b");
        ws.page_mut(&a).unwrap().project.as_mut().unwrap().last_activity = 200;
        ws.page_mut(&b).unwrap().project.as_mut().unwrap().last_activity = 100;
        let ids = |v: Vec<&Page>| v.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(ws.launch_projects()), [a.clone(), b.clone()]);
        // « arrêt à » changed on B: its page is edited (new `updated`), not its activity.
        let pb = ws.page_mut(&b).unwrap();
        pb.project.as_mut().unwrap().session_ceiling = Some(42.0);
        pb.updated = u64::MAX;
        assert_eq!(ids(ws.launch_projects()), [a.clone(), b.clone()]);
        // Real activity (a step, a session) still brings a project up.
        ws.page_mut(&b).unwrap().project.as_mut().unwrap().last_activity = 300;
        assert_eq!(ids(ws.launch_projects()), [b, a]);
    }

    #[test]
    fn deepest_folder_wins() {
        let mut ws = Workspace::default();
        let outer = ws.create_project("Code", "/home/me/Code");
        let inner = ws.create_project("Tableau", "/home/me/Code/orchestra/");
        assert_eq!(
            ws.project_for_path(Path::new("/home/me/Code/orchestra/src"))
                .unwrap()
                .id,
            inner
        );
        assert_eq!(
            ws.project_for_path(Path::new("/home/me/Code/autre"))
                .unwrap()
                .id,
            outer
        );
        assert!(ws.project_for_path(Path::new("/home/me/Docs")).is_none());
        // A sibling whose name only starts the same doesn't match.
        assert_eq!(
            ws.project_for_path(Path::new("/home/me/Code/orchestra-2"))
                .unwrap()
                .id,
            outer
        );
    }

    #[test]
    fn sessions_are_merged_by_tool_and_id() {
        let mut p = Project::new("");
        p.link_session(SessionRef {
            tool: "claude-code".into(),
            id: "s1".into(),
            title: "".into(),
            location: "".into(),
            started: 10,
            last_activity: 20,
        });
        p.link_session(SessionRef {
            tool: "claude-code".into(),
            id: "s1".into(),
            title: "Refonte".into(),
            location: "".into(),
            started: 10,
            last_activity: 50,
        });
        p.link_session(SessionRef {
            tool: "codex".into(),
            id: "s1".into(),
            title: "".into(),
            location: "".into(),
            started: 5,
            last_activity: 30,
        });
        assert_eq!(p.sessions.len(), 2);
        assert_eq!(p.sessions[0].title, "Refonte");
        assert_eq!(p.sessions[0].last_activity, 50);
    }

    #[test]
    fn steps_added_after_the_state_of_play_make_it_stale() {
        let mut pr = Project::new("");
        pr.add_step("A : faire", "ia");
        // None written: nothing to be out of date.
        assert!(pr.stale_summary_note().is_none());
        pr.set_summary("- fait", "ia");
        pr.steps[0].created = pr.summary_updated - 10;
        assert_eq!(pr.steps_since_summary(), 0);
        let id = pr.add_step("B : faire", "ia");
        pr.steps[1].created = pr.summary_updated + 10;
        assert_eq!(pr.steps_since_summary(), 1);
        assert!(pr.stale_summary_note().is_some());
        // A status change alone doesn't count, and writing it again clears the note.
        pr.set_step_status(&id, StepStatus::Done);
        assert_eq!(pr.steps_since_summary(), 1);
        pr.steps[1].created = pr.summary_updated;
        assert!(pr.stale_summary_note().is_none());

        let mut ws = crate::model::Workspace::default();
        let ex = ws.create_example_project();
        let x = ws.page_mut(&ex).unwrap().project.as_ref().unwrap();
        assert_eq!(x.steps_since_summary(), 0);
    }

    #[test]
    fn structured_summary() {
        let s = summary_from_parts(&["- [x] **Modèle**".into()], &[], &["Accès API".into()], "");
        assert_eq!(s, "### Fait\n- **Modèle**\n\n### Bloquants\n- Accès API");
    }

    #[test]
    fn only_shared_projects_leave() {
        let mut ws = Workspace::default();
        let a = ws.create_project("Partagé", "");
        let sub = ws.create_page("Doc", Some(a.clone()));
        let b = ws.create_project("Privé", "");
        ws.create_page("Notes perso", None);
        ws.page_mut(&a).unwrap().project.as_mut().unwrap().shared = true;
        for (pid, txt) in [(&a, "salut"), (&b, "secret")] {
            ws.chat.insert(
                txt.into(),
                crate::model::ChatMsg {
                    id: txt.into(),
                    project: pid.clone(),
                    author: "moi".into(),
                    text: txt.into(),
                    at: 1,
                },
            );
        }
        let out = ws.shared_copy();
        let ids: Vec<&str> = out.pages.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&a.as_str()) && ids.contains(&sub.as_str()));
        assert_eq!(out.pages.len(), 2);
        assert_eq!(out.chat.len(), 1);
        assert!(out.chat.contains_key("salut"));
        // Local copy: new ids, private, and its tree follows.
        let c = ws.local_copy(&a).unwrap();
        let copy = ws.page(&c).unwrap();
        assert!(!copy.project.as_ref().unwrap().shared);
        assert!(copy.title.ends_with("(copie locale)"));
        assert_eq!(ws.children(Some(&c)).len(), 1);
        assert!(!ws.shared_ids().contains(&c));
    }

    /// A shared project leaves this computer without its folder, its AI sessions and its
    /// launch choices; its steps, state of play and pages go out; ours stay as they were.
    #[test]
    fn a_shared_project_leaves_without_what_is_ours() {
        let mut ws = Workspace::default();
        let a = ws.create_project("Partagé", "");
        let sub = ws.create_page("Doc", Some(a.clone()));
        {
            let p = ws.page_mut(&a).unwrap().project.as_mut().unwrap();
            p.shared = true;
            p.folder = "/Users/moi/code/app".into();
            p.commit_to_main = true;
            p.session_limit = Some(95.0);
            p.launch = Some(LaunchPrefs {
                commits: true,
                endless: true,
                end: Default::default(),
                agent: None, model: String::new(), saving: Default::default(), auto_agent: false,
            });
            p.sessions.push(SessionRef {
                tool: "claude-code".into(),
                id: "abc".into(),
                title: String::new(),
                location: "/Users/moi/.claude/x.jsonl".into(),
                started: 1,
                last_activity: 2,
            });
            p.sessions_cleared = 5;
            p.summary = "État partagé".into();
        }
        let out = ws.shared_copy();
        let sent = out
            .pages
            .iter()
            .find(|p| p.id == a)
            .unwrap()
            .project
            .as_ref()
            .unwrap();
        assert!(sent.folder.is_empty() && !sent.commit_to_main);
        assert_eq!(
            (
                sent.session_limit,
                sent.launch.clone(),
                sent.sessions.len(),
                sent.sessions_cleared
            ),
            (None, None, 0, 0)
        );
        assert!(sent.shared);
        assert_eq!(sent.summary, "État partagé");
        assert!(out.pages.iter().any(|p| p.id == sub));
        // Nothing of the JSON that goes out names the folder nor the session file.
        let json = serde_json::to_string(&out).unwrap();
        assert!(!json.contains("/Users/moi"), "{json}");
        // Ours is untouched.
        let ours = ws.page(&a).unwrap().project.as_ref().unwrap();
        assert_eq!(ours.folder, "/Users/moi/code/app");
        assert_eq!((ours.session_limit, ours.sessions.len()), (Some(95.0), 1));
    }

    /// Messages received: only for a project shared here, once, cut to size.
    #[test]
    fn only_shared_discussions_come_in() {
        use crate::model::ChatMsg;
        let mut ws = Workspace::default();
        let shared = ws.create_project("Partagé", "");
        ws.page_mut(&shared)
            .unwrap()
            .project
            .as_mut()
            .unwrap()
            .shared = true;
        let private = ws.create_project("Privé", "");
        let msg = |id: &str, project: &str, text: String| ChatMsg {
            id: id.into(),
            project: project.into(),
            author: "Camille\nseconde ligne".into(),
            text,
            at: 1,
        };
        let n = ws.accept_chat(vec![
            msg("a", &shared, "salut".into()),
            msg("b", &private, "intrus".into()),
            msg("c", "inconnu", "intrus".into()),
            msg("", &shared, "sans id".into()),
            msg("d", &shared, "x".repeat(RECEIVED_CHAT_MAX + 500)),
        ]);
        assert_eq!(n, 2);
        assert!(ws.chat.contains_key("a") && ws.chat.contains_key("d"));
        assert!(!ws.chat.contains_key("b") && !ws.chat.contains_key("c"));
        assert_eq!(ws.chat["a"].author, "Camille");
        assert_eq!(ws.chat["d"].text.chars().count(), RECEIVED_CHAT_MAX + 1);
        // Already known: not taken again, not replaced.
        assert_eq!(ws.accept_chat(vec![msg("a", &shared, "changé".into())]), 0);
        assert_eq!(ws.chat["a"].text, "salut");
    }

    #[test]
    fn only_shared_pages_come_in() {
        let mut ws = Workspace::default();
        let folder = ws.create_page("Dossier privé", None);
        let a = ws.create_project("Partagé", "");
        ws.page_mut(&a).unwrap().parent = Some(folder.clone());
        ws.page_mut(&a).unwrap().project.as_mut().unwrap().shared = true;
        let doc = ws.create_page("Doc", Some(a.clone()));
        let b = ws.create_project("Privé", "");
        let notes = ws.create_page("Notes perso", None);
        let edited = |id: &str| {
            let mut p = ws.page(id).unwrap().clone();
            p.title = "changé".into();
            p
        };
        let (pa, pdoc, pb, pnotes, pfolder) = (
            edited(&a),
            edited(&doc),
            edited(&b),
            edited(&notes),
            edited(&folder),
        );
        // A shared page moved under a private one.
        let mut moved = pdoc.clone();
        moved.parent = Some(b.clone());
        // New pages: under a shared page (even one arriving later in the batch), under a
        // private page, at the top level, a private project, a shared project.
        let child = |id: &str, parent: &str| Page {
            id: id.into(),
            parent: Some(parent.into()),
            ..Page::new("x", None)
        };
        let mut new_shared = Page::new("Autre projet", None);
        new_shared.id = "np".into();
        new_shared.project = Some(Project {
            shared: true,
            ..Project::new("")
        });
        let mut new_private = new_shared.clone();
        new_private.id = "npp".into();
        new_private.project.as_mut().unwrap().shared = false;
        let incoming = vec![
            child("n2", "n1"),
            child("n1", &doc),
            child("evil", &b),
            Page {
                id: "top".into(),
                ..Page::new("x", None)
            },
            child("under-np", "np"),
            new_shared,
            new_private,
            pa,
            pdoc,
            pb,
            pnotes,
            pfolder,
        ];
        let got: Vec<String> = ws
            .accept_remote(incoming)
            .into_iter()
            .map(|p| p.id)
            .collect();
        for id in [a.as_str(), doc.as_str(), "n1", "n2", "np", "under-np"] {
            assert!(got.iter().any(|g| g == id), "{id} refusée");
        }
        for id in [
            b.as_str(),
            notes.as_str(),
            folder.as_str(),
            "evil",
            "top",
            "npp",
        ] {
            assert!(!got.iter().any(|g| g == id), "{id} acceptée");
        }
        assert!(ws.accept_remote(vec![moved]).is_empty());
        // Deletions: shared pages and unknown ids, never a private page whose id they know.
        let del: std::collections::BTreeMap<String, u64> = [&doc, &b, &notes, &folder]
            .iter()
            .map(|i| (i.to_string(), 5))
            .chain([("inconnue".to_string(), 5)])
            .collect();
        let kept: Vec<String> = ws.accept_remote_deletions(del).into_keys().collect();
        assert!(kept.contains(&doc) && kept.contains(&"inconnue".to_string()));
        assert!(!kept.contains(&b) && !kept.contains(&notes) && !kept.contains(&folder));
    }

    #[test]
    fn status_names() {
        assert_eq!(
            ProjectStatus::from_name("en pause"),
            Some(ProjectStatus::Paused)
        );
        assert_eq!(ProjectStatus::from_name("done"), Some(ProjectStatus::Done));
        assert_eq!(StepStatus::from_name("doing"), Some(StepStatus::Doing));
    }

    #[test]
    fn old_files_still_load() {
        // A workspace written by Notes libres (no `project` field) loads unchanged.
        let ws: Workspace =
            serde_json::from_str(r#"{"pages":[{"id":"a","title":"T","created":1,"updated":1}]}"#)
                .unwrap();
        assert!(ws.pages[0].project.is_none());
        assert!(ws.projects().is_empty());
    }
}
