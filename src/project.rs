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

/// A state of play built from lists, so every AI's summary has the same shape:
/// "Fait", "En cours", "Bloquants", then free notes.
pub fn summary_from_parts(done: &[String], doing: &[String], blockers: &[String], notes: &str) -> String {
    let mut out = String::new();
    for (title, items) in [(crate::tr!("Fait", "Done"), done), (crate::tr!("En cours", "In progress"), doing), (crate::tr!("Bloquants", "Blockers"), blockers)] {
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
    pub const ALL: [ProjectStatus; 4] = [ProjectStatus::Idea, ProjectStatus::Active, ProjectStatus::Paused, ProjectStatus::Done];

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
            k.label().to_lowercase() == s || serde_json::to_value(k).ok().and_then(|v| v.as_str().map(|x| x == s)) == Some(true)
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
            k.label().to_lowercase() == s || serde_json::to_value(k).ok().and_then(|v| v.as_str().map(|x| x == s)) == Some(true)
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
}

/// Longest title of a step received from another computer.
pub const RECEIVED_STEP_MAX: usize = 300;

/// Whether the merge of a shared project takes their copy of a step over ours: the most
/// recent edit wins, except when both copies hold a live reservation by two different
/// computers (two AIs took the step before seeing each other): then the earlier one wins on
/// both sides, and the other AI sees it lost at its next get_project_state.
fn takes_theirs(ours: &Step, theirs: &Step) -> bool {
    let now = now_ms();
    let live = |s: &Step| s.status == StepStatus::Doing && !s.held_by.is_empty() && now < s.held_at.saturating_add(HOLD_MS);
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
        self.status == StepStatus::Doing && !self.held_by.is_empty() && self.held_by != me && now < self.held_at.saturating_add(HOLD_MS)
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
    /// "Regard extérieur": a short outside view asked to an AI (Markdown), by whom, when.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Review {
    pub text: String,
    pub by: String,
    pub at: u64,
}

impl Project {
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
                self.read_only = o.read_only;
                self.sessions = o.sessions.clone();
                self.sessions_cleared = o.sessions_cleared;
            }
            None => {
                let theirs = std::mem::take(&mut self.steps);
                let tombs = std::mem::take(&mut self.deleted_steps);
                self.folder.clear();
                self.commit_to_main = false;
                self.sessions.clear();
                self.sessions_cleared = 0;
                self.merge_steps(&theirs, &tombs);
            }
        }
    }

    /// Bring another copy's steps into ours, by step id: each step keeps its most recent
    /// edit, steps new on their side come in, steps deleted on either side stay deleted.
    /// What comes in is cleaned (one line, `RECEIVED_STEP_MAX` characters at most) and
    /// marked `received`; a step already here keeps its own mark. Returns true when ours
    /// changed.
    pub fn merge_steps(&mut self, theirs: &[Step], their_deleted: &std::collections::BTreeMap<String, u64>) -> bool {
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
                    self.steps.push(s);
                    changed = true;
                }
            }
        }
        let tombs = &self.deleted_steps;
        let before = self.steps.len();
        self.steps.retain(|s| tombs.get(&s.id).is_none_or(|t| s.updated > *t));
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
        Project { folder: folder.into(), status: ProjectStatus::Active, last_activity: now_ms(), ..Default::default() }
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
        };
        let id = s.id.clone();
        self.steps.push(s);
        self.touch();
        id
    }

    pub fn set_step_status(&mut self, id: &str, status: StepStatus) -> bool {
        let Some(s) = self.steps.iter_mut().find(|s| s.id == id) else { return false };
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

    /// An AI of computer `me` takes step `id`: « doing », reserved for `me`. Refused while an
    /// AI of another computer holds it (see `Step::held_elsewhere`); taking it again renews
    /// the reservation. Err holds who has it, for the AI.
    pub fn claim_step(&mut self, id: &str, me: &str) -> Result<(), String> {
        let now = now_ms();
        let Some(s) = self.steps.iter_mut().find(|s| s.id == id) else { return Err(format!("no step {id}")) };
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
        let Some(from) = self.steps.iter().position(|s| s.id == id) else { return false };
        if before.is_some_and(|b| !self.steps.iter().any(|s| s.id == b)) {
            return false;
        }
        let old: Vec<String> = self.steps.iter().map(|s| s.id.clone()).collect();
        let s = self.steps.remove(from);
        let to = match before {
            Some(b) => self.steps.iter().position(|s| s.id == b).unwrap_or(self.steps.len()),
            None => self.steps.iter().rposition(|s| s.status != StepStatus::Done).map_or(0, |i| i + 1),
        };
        self.steps.insert(to, s);
        let moved = self.steps.iter().map(|s| s.id.as_str()).ne(old.iter().map(String::as_str));
        if moved {
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
        (self.steps.iter().filter(|s| s.status == StepStatus::Done).count(), self.steps.len())
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
            .or_else(|| self.steps.iter().filter(free).find(|s| s.status == StepStatus::Todo))
    }

    /// Add a session, or refresh it if already known (same tool and id).
    pub fn link_session(&mut self, mut s: SessionRef) {
        s.title = crate::richtext::one_line(&s.title, 120);
        match self.sessions.iter_mut().find(|x| x.tool == s.tool && x.id == s.id) {
            Some(old) => {
                if !s.title.is_empty() {
                    old.title = s.title;
                }
                if !s.location.is_empty() {
                    old.location = s.location;
                }
                old.started = if old.started == 0 { s.started } else { old.started.min(s.started.max(1)) };
                old.last_activity = old.last_activity.max(s.last_activity);
            }
            None => self.sessions.push(s),
        }
        self.sessions.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
        let latest = self.sessions.first().map(|s| s.last_activity).unwrap_or(0);
        self.last_activity = self.last_activity.max(latest);
    }
}

/// `~/x` → home/x; removes `.` and `..` and trailing slashes, without touching the disk.
pub fn normalize_folder(folder: &str) -> PathBuf {
    let f = folder.trim();
    let raw = match f.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            dirs::home_dir().unwrap_or_default().join(rest.trim_start_matches('/'))
        }
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

impl Workspace {
    /// Pages that are projects (not in the trash), most recent activity first.
    pub fn projects(&self) -> Vec<&Page> {
        let mut v: Vec<&Page> = self.pages.iter().filter(|p| !p.trashed && p.project.is_some()).collect();
        v.sort_by(|a, b| {
            let la = a.project.as_ref().map(|x| x.last_activity).unwrap_or(0).max(a.updated);
            let lb = b.project.as_ref().map(|x| x.last_activity).unwrap_or(0).max(b.updated);
            lb.cmp(&la)
        });
        v
    }

    /// Create a project page (top level) with an introduction block.
    pub fn create_project(&mut self, title: &str, folder: &str) -> String {
        let id = self.create_page(title, None);
        if let Some(p) = self.page_mut(&id) {
            p.icon = "🚀".into();
            p.project = Some(Project::new(folder.trim()));
            p.blocks = vec![crate::model::Block::new(crate::model::BlockKind::Paragraph, "")];
        }
        id
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
        let mut out = std::collections::HashSet::new();
        for p in self.pages.iter().filter(|p| p.project.as_ref().is_some_and(|x| x.shared)) {
            out.insert(p.id.clone());
            out.extend(self.descendants(&p.id));
        }
        out
    }

    /// Of the pages another person sent, those that may come in: only pages of shared
    /// projects, never one of ours that isn't shared, nor one placed under a private page.
    /// - a page we have: only if it is shared here, and it stays under a shared page;
    /// - a new project page: only a shared one, at the top level;
    /// - a new page: only under a shared page (ours, or one accepted in the same batch).
    pub fn accept_remote(&self, pages: Vec<Page>) -> Vec<Page> {
        let shared = self.shared_ids();
        let known: std::collections::HashMap<&str, &Option<String>> = self.pages.iter().map(|p| (p.id.as_str(), &p.parent)).collect();
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
    pub fn accept_remote_deletions(&self, deleted: std::collections::BTreeMap<String, u64>) -> std::collections::BTreeMap<String, u64> {
        let shared = self.shared_ids();
        deleted.into_iter().filter(|(id, _)| shared.contains(id) || self.page(id).is_none()).collect()
    }

    /// What may leave this computer: the shared projects, their pages and discussions.
    pub fn shared_copy(&self) -> Workspace {
        let ids = self.shared_ids();
        Workspace {
            pages: self.pages.iter().filter(|p| ids.contains(&p.id)).cloned().collect(),
            deleted: self.deleted.clone(),
            chat: self.chat.iter().filter(|(_, m)| ids.contains(&m.project)).map(|(k, v)| (k.clone(), v.clone())).collect(),
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
    let Some(pr) = &page.project else { return String::new() };
    let mut out = format!("# {}\n\n", page.display_title());
    out.push_str(&crate::trf!("- id : {}\n- statut : {}\n", "- id: {}\n- status: {}\n", page.id, pr.status.label()));
    if !pr.folder.is_empty() {
        out.push_str(&crate::trf!("- dossier : {}\n", "- folder: {}\n", pr.folder));
    }
    let (done, total) = pr.progress();
    out.push_str(&crate::trf!("- étapes : {done}/{total} terminées\n", "- steps: {done}/{total} done\n"));
    let (me, now) = (crate::store::computer_id(), now_ms());
    if let Some(n) = pr.next_step_for(&me, now) {
        out.push_str(&crate::trf!("- prochaine étape : {}\n", "- next step: {}\n", n.title));
    }
    out.push_str(crate::tr!("\n## État des lieux\n\n", "\n## Current state\n\n"));
    if pr.summary.trim().is_empty() {
        out.push_str(crate::tr!("(pas encore écrit)\n", "(not written yet)\n"));
    } else {
        out.push_str(&pr.summary);
        out.push('\n');
        if !pr.summary_by.is_empty() {
            out.push_str(&crate::trf!("\n_écrit par {}_\n", "\n_written by {}_\n", pr.summary_by));
        }
    }
    out.push_str(crate::tr!("\n## Étapes\n\n", "\n## Steps\n\n"));
    if pr.steps.is_empty() {
        out.push_str(crate::tr!("(aucune)\n", "(none)\n"));
    }
    for s in &pr.steps {
        let mark = match s.status {
            StepStatus::Done => "x",
            StepStatus::Doing => "~",
            StepStatus::Todo => " ",
        };
        let src = if s.source.is_empty() { String::new() } else { format!(" — {}", s.source) };
        // Written on another computer of the shared project: the AI weighs it as a request.
        let from = if s.received { crate::tr!(" — reçue d'un autre membre du projet partagé", " — received from another member of the shared project") } else { "" };
        // Another computer's AI is on it: never take it as well.
        let held = if s.held_elsewhere(&me, now) {
            crate::tr!(" — réservée par une IA d'un autre ordinateur, ne la prends pas", " — reserved by an AI on another computer, don't take it")
        } else {
            ""
        };
        out.push_str(&format!("- [{mark}] {} (step_id: {}){src}{from}{held}\n", s.title, s.id));
    }
    out.push_str(crate::tr!("\n## Sessions IA\n\n", "\n## AI sessions\n\n"));
    if pr.sessions.is_empty() {
        out.push_str(crate::tr!("(aucune)\n", "(none)\n"));
    }
    for s in &pr.sessions {
        let t = if s.title.is_empty() { crate::tr!("(sans titre)", "(untitled)") } else { &s.title };
        out.push_str(&format!("- {} · {} (session {})\n", s.tool, t, s.id));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Workspace;

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
        let ids: Vec<String> = ["A", "B", "C", "D"].iter().map(|t| p.add_step(t, "moi")).collect();
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
    fn clearing_sessions_hides_the_past_ones() {
        let mut p = Project::new("/tmp/x");
        p.link_session(SessionRef { tool: "claude-code".into(), id: "s1".into(), title: String::new(), location: String::new(), started: 1, last_activity: 1 });
        assert!(p.session_shown(1));
        p.clear_sessions();
        assert!(p.sessions.is_empty());
        assert!(!p.session_shown(1));
        assert!(p.session_shown(now_ms() + 1000));
    }

    #[test]
    fn deepest_folder_wins() {
        let mut ws = Workspace::default();
        let outer = ws.create_project("Code", "/home/me/Code");
        let inner = ws.create_project("Tableau", "/home/me/Code/orchestra/");
        assert_eq!(ws.project_for_path(Path::new("/home/me/Code/orchestra/src")).unwrap().id, inner);
        assert_eq!(ws.project_for_path(Path::new("/home/me/Code/autre")).unwrap().id, outer);
        assert!(ws.project_for_path(Path::new("/home/me/Docs")).is_none());
        // A sibling whose name only starts the same doesn't match.
        assert_eq!(ws.project_for_path(Path::new("/home/me/Code/orchestra-2")).unwrap().id, outer);
    }

    #[test]
    fn sessions_are_merged_by_tool_and_id() {
        let mut p = Project::new("");
        p.link_session(SessionRef { tool: "claude-code".into(), id: "s1".into(), title: "".into(), location: "".into(), started: 10, last_activity: 20 });
        p.link_session(SessionRef { tool: "claude-code".into(), id: "s1".into(), title: "Refonte".into(), location: "".into(), started: 10, last_activity: 50 });
        p.link_session(SessionRef { tool: "codex".into(), id: "s1".into(), title: "".into(), location: "".into(), started: 5, last_activity: 30 });
        assert_eq!(p.sessions.len(), 2);
        assert_eq!(p.sessions[0].title, "Refonte");
        assert_eq!(p.sessions[0].last_activity, 50);
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
            ws.chat.insert(txt.into(), crate::model::ChatMsg { id: txt.into(), project: pid.clone(), author: "moi".into(), text: txt.into(), at: 1 });
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
        let mut edited = |id: &str| {
            let mut p = ws.page(id).unwrap().clone();
            p.title = "changé".into();
            p
        };
        let (pa, pdoc, pb, pnotes, pfolder) = (edited(&a), edited(&doc), edited(&b), edited(&notes), edited(&folder));
        // A shared page moved under a private one.
        let mut moved = pdoc.clone();
        moved.parent = Some(b.clone());
        // New pages: under a shared page (even one arriving later in the batch), under a
        // private page, at the top level, a private project, a shared project.
        let child = |id: &str, parent: &str| Page { id: id.into(), parent: Some(parent.into()), ..Page::new("x", None) };
        let mut new_shared = Page::new("Autre projet", None);
        new_shared.id = "np".into();
        new_shared.project = Some(Project { shared: true, ..Project::new("") });
        let mut new_private = new_shared.clone();
        new_private.id = "npp".into();
        new_private.project.as_mut().unwrap().shared = false;
        let incoming = vec![
            child("n2", "n1"), child("n1", &doc), child("evil", &b), Page { id: "top".into(), ..Page::new("x", None) },
            child("under-np", "np"), new_shared, new_private, pa, pdoc, pb, pnotes, pfolder,
        ];
        let got: Vec<String> = ws.accept_remote(incoming).into_iter().map(|p| p.id).collect();
        for id in [a.as_str(), doc.as_str(), "n1", "n2", "np", "under-np"] {
            assert!(got.iter().any(|g| g == id), "{id} refusée");
        }
        for id in [b.as_str(), notes.as_str(), folder.as_str(), "evil", "top", "npp"] {
            assert!(!got.iter().any(|g| g == id), "{id} acceptée");
        }
        assert!(ws.accept_remote(vec![moved]).is_empty());
        // Deletions: shared pages and unknown ids, never a private page whose id they know.
        let del: std::collections::BTreeMap<String, u64> = [&doc, &b, &notes, &folder].iter().map(|i| (i.to_string(), 5)).chain([("inconnue".to_string(), 5)]).collect();
        let kept: Vec<String> = ws.accept_remote_deletions(del).into_keys().collect();
        assert!(kept.contains(&doc) && kept.contains(&"inconnue".to_string()));
        assert!(!kept.contains(&b) && !kept.contains(&notes) && !kept.contains(&folder));
    }

    #[test]
    fn status_names() {
        assert_eq!(ProjectStatus::from_name("en pause"), Some(ProjectStatus::Paused));
        assert_eq!(ProjectStatus::from_name("done"), Some(ProjectStatus::Done));
        assert_eq!(StepStatus::from_name("doing"), Some(StepStatus::Doing));
    }

    #[test]
    fn old_files_still_load() {
        // A workspace written by Notes libres (no `project` field) loads unchanged.
        let ws: Workspace = serde_json::from_str(r#"{"pages":[{"id":"a","title":"T","created":1,"updated":1}]}"#).unwrap();
        assert!(ws.pages[0].project.is_none());
        assert!(ws.projects().is_empty());
    }
}
