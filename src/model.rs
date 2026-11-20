//! Data model: a workspace made of nested pages, each page made of blocks,
//! and optionally holding a database (table / board / list views).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub use crate::project::Project;
use std::time::{SystemTime, UNIX_EPOCH};

static CLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Milliseconds since 1970, never going backwards and never behind a time seen from another
/// computer (a hybrid clock), so "most recent edit wins" holds even if clocks differ a little.
pub fn now_ms() -> u64 {
    use std::sync::atomic::Ordering;
    let sys = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let mut cur = CLOCK.load(Ordering::SeqCst);
    loop {
        let next = sys.max(cur + 1);
        match CLOCK.compare_exchange(cur, next, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return next,
            Err(v) => cur = v,
        }
    }
}

/// Note a time received from another computer.
pub fn observe_clock(t: u64) {
    CLOCK.fetch_max(t, std::sync::atomic::Ordering::SeqCst);
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Paragraph,
    Heading1,
    Heading2,
    Heading3,
    Todo,
    Bullet,
    Numbered,
    Toggle,
    Quote,
    Callout,
    Code,
    Divider,
    PageLink,
}

impl BlockKind {
    pub const ALL: [BlockKind; 13] = [
        BlockKind::Paragraph,
        BlockKind::Heading1,
        BlockKind::Heading2,
        BlockKind::Heading3,
        BlockKind::Todo,
        BlockKind::Bullet,
        BlockKind::Numbered,
        BlockKind::Toggle,
        BlockKind::Quote,
        BlockKind::Callout,
        BlockKind::Code,
        BlockKind::Divider,
        BlockKind::PageLink,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BlockKind::Paragraph => crate::tr!("Texte", "Text"),
            BlockKind::Heading1 => crate::tr!("Titre 1", "Heading 1"),
            BlockKind::Heading2 => crate::tr!("Titre 2", "Heading 2"),
            BlockKind::Heading3 => crate::tr!("Titre 3", "Heading 3"),
            BlockKind::Todo => crate::tr!("Liste de tâches", "To-do list"),
            BlockKind::Bullet => crate::tr!("Liste à puces", "Bulleted list"),
            BlockKind::Numbered => crate::tr!("Liste numérotée", "Numbered list"),
            BlockKind::Toggle => crate::tr!("Menu dépliant", "Toggle list"),
            BlockKind::Quote => crate::tr!("Citation", "Quote"),
            BlockKind::Callout => crate::tr!("Encadré", "Callout"),
            BlockKind::Code => "Code",
            BlockKind::Divider => crate::tr!("Séparateur", "Divider"),
            BlockKind::PageLink => "Page",
        }
    }

    /// Words accepted after "/" to pick this kind (French and English).
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            BlockKind::Paragraph => &["texte", "text", "paragraphe"],
            BlockKind::Heading1 => &["titre1", "h1", "heading1", "titre 1"],
            BlockKind::Heading2 => &["titre2", "h2", "heading2", "titre 2"],
            BlockKind::Heading3 => &["titre3", "h3", "heading3", "titre 3"],
            BlockKind::Todo => &["todo", "tache", "tâche", "case"],
            BlockKind::Bullet => &["puce", "bullet", "liste"],
            BlockKind::Numbered => &["numero", "numéro", "numbered", "1."],
            BlockKind::Toggle => &["toggle", "depliant", "dépliant", "menu", "liste dépliante"],
            BlockKind::Quote => &["citation", "quote"],
            BlockKind::Callout => &["encadre", "encadré", "callout"],
            BlockKind::Code => &["code"],
            BlockKind::Divider => &["separateur", "séparateur", "divider", "---"],
            BlockKind::PageLink => &["page", "souspage", "sous-page"],
        }
    }

    pub fn from_name(s: &str) -> Option<BlockKind> {
        let s = s.trim().to_lowercase();
        BlockKind::ALL.into_iter().find(|k| {
            serde_json::to_value(k).ok().and_then(|v| v.as_str().map(|x| x == s)) == Some(true)
                || k.keywords().contains(&s.as_str())
                || k.label().to_lowercase() == s
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    pub id: String,
    pub kind: BlockKind,
    #[serde(default)]
    pub text: String,
    /// Todo: ticked. Toggle: open.
    #[serde(default)]
    pub checked: bool,
    /// Toggle: hidden content. PageLink: target page id.
    #[serde(default)]
    pub extra: String,
    /// Nesting level (Tab / Shift+Tab), drawn 26 px further right per level.
    #[serde(default)]
    pub indent: u8,
    /// Last edit of this block, for merging edits block by block.
    #[serde(default)]
    pub updated: u64,
}

impl Block {
    pub fn new(kind: BlockKind, text: impl Into<String>) -> Self {
        Block { id: new_id(), kind, text: text.into(), checked: false, extra: String::new(), indent: 0, updated: now_ms() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKind {
    Text,
    Number,
    Select,
    Checkbox,
    Date,
}

impl ColumnKind {
    pub const ALL: [ColumnKind; 5] =
        [ColumnKind::Text, ColumnKind::Number, ColumnKind::Select, ColumnKind::Checkbox, ColumnKind::Date];
    pub fn label(self) -> &'static str {
        match self {
            ColumnKind::Text => crate::tr!("Texte", "Text"),
            ColumnKind::Number => crate::tr!("Nombre", "Number"),
            ColumnKind::Select => crate::tr!("Sélection", "Select"),
            ColumnKind::Checkbox => crate::tr!("Case à cocher", "Checkbox"),
            ColumnKind::Date => "Date",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Column {
    pub id: String,
    pub name: String,
    pub kind: ColumnKind,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub id: String,
    /// Column id -> value (checkbox: "true"/"false").
    #[serde(default)]
    pub cells: BTreeMap<String, String>,
    /// The page that holds this row's own content, once opened.
    #[serde(default)]
    pub page: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DbView {
    #[default]
    Table,
    Board,
    List,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Database {
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    #[serde(default)]
    pub view: DbView,
    #[serde(default)]
    pub sort: Option<(String, bool)>,
    #[serde(default)]
    pub filter: String,
}

impl Database {
    pub fn new_default() -> Self {
        let name = Column { id: new_id(), name: crate::tr!("Nom", "Name").into(), kind: ColumnKind::Text, options: vec![] };
        let status = Column {
            id: new_id(),
            name: crate::tr!("Statut", "Status").into(),
            kind: ColumnKind::Select,
            options: vec![crate::tr!("À faire", "To do").into(), crate::tr!("En cours", "In progress").into(), crate::tr!("Terminé", "Done").into()],
        };
        let date = Column { id: new_id(), name: "Date".into(), kind: ColumnKind::Date, options: vec![] };
        let mut db = Database { columns: vec![name, status, date], rows: vec![], view: DbView::Table, sort: None, filter: String::new() };
        db.add_row(crate::tr!("Première ligne", "First row"));
        db
    }

    pub fn title_col(&self) -> Option<&Column> {
        self.columns.first()
    }

    pub fn add_row(&mut self, title: &str) -> String {
        let mut row = Row { id: new_id(), cells: BTreeMap::new(), page: None };
        if let Some(c) = self.columns.first() {
            row.cells.insert(c.id.clone(), title.to_string());
        }
        if let Some(c) = self.columns.iter().find(|c| c.kind == ColumnKind::Select)
            && let Some(o) = c.options.first() {
                row.cells.insert(c.id.clone(), o.clone());
            }
        let id = row.id.clone();
        self.rows.push(row);
        id
    }

    pub fn row_title(&self, row: &Row) -> String {
        self.title_col().and_then(|c| row.cells.get(&c.id).cloned()).unwrap_or_default()
    }

    pub fn column_by_name(&self, name: &str) -> Option<&Column> {
        let n = name.to_lowercase();
        self.columns.iter().find(|c| c.name.to_lowercase() == n || c.id == name)
    }

    /// Rows after the filter text and sort setting are applied (indexes into `rows`).
    pub fn visible_rows(&self) -> Vec<usize> {
        let f = self.filter.trim().to_lowercase();
        let mut idx: Vec<usize> = (0..self.rows.len())
            .filter(|&i| f.is_empty() || self.rows[i].cells.values().any(|v| v.to_lowercase().contains(&f)))
            .collect();
        if let Some((col, asc)) = &self.sort {
            let kind = self.columns.iter().find(|c| &c.id == col).map(|c| c.kind);
            idx.sort_by(|&a, &b| {
                let va = self.rows[a].cells.get(col).cloned().unwrap_or_default();
                let vb = self.rows[b].cells.get(col).cloned().unwrap_or_default();
                let o = if kind == Some(ColumnKind::Number) {
                    let na = va.trim().replace(',', ".").parse::<f64>().unwrap_or(f64::MIN);
                    let nb = vb.trim().replace(',', ".").parse::<f64>().unwrap_or(f64::MIN);
                    na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
                } else {
                    va.to_lowercase().cmp(&vb.to_lowercase())
                };
                if *asc { o } else { o.reverse() }
            });
        }
        idx
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub database: Option<Database>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub trashed: bool,
    #[serde(default)]
    pub full_width: bool,
    pub created: u64,
    pub updated: u64,
    /// Blocks removed from this page: id -> time, so a merge doesn't bring them back.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub deleted_blocks: BTreeMap<String, u64>,
    /// Set when this page is an AI project (see `Project`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Project>,
    /// Set when this page mirrors a file of a project (README, docs…), see docs.rs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::docs::PageSource>,
}

impl Page {
    pub fn new(title: impl Into<String>, parent: Option<String>) -> Self {
        let t = now_ms();
        Page {
            id: new_id(),
            title: title.into(),
            icon: String::new(),
            parent,
            blocks: vec![Block::new(BlockKind::Paragraph, "")],
            database: None,
            favorite: false,
            trashed: false,
            full_width: false,
            created: t,
            updated: t,
            deleted_blocks: BTreeMap::new(),
            project: None,
            source: None,
        }
    }

    pub fn display_title(&self) -> String {
        if self.title.trim().is_empty() { "Sans titre".to_string() } else { self.title.clone() }
    }

    pub fn touch(&mut self) {
        self.updated = now_ms();
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorKind {
    #[default]
    None,
    SharedFolder,
    Command,
    /// Real time: host on this computer or join another one.
    Live,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CollabSettings {
    #[serde(default)]
    pub connector: ConnectorKind,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub auto_sync: bool,
    #[serde(default)]
    pub author: String,
    /// Live: true to host here, false to join `live_address`.
    #[serde(default)]
    pub live_host: bool,
    #[serde(default)]
    pub live_address: String,
    #[serde(default)]
    pub live_port: u16,
    /// Kept in the Keychain (secret.rs), never written to workspace.json: read from it only
    /// to move the code of older versions there.
    #[serde(default, skip_serializing)]
    pub live_code: String,
    /// Live was switched on (it starts again with the app).
    #[serde(default)]
    pub live_on: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub dark: bool,
    /// The last version of Orchestra that had the « Screen Recording » permission. When a
    /// later version doesn't, its entry in System Settings is a stale one left by the update:
    /// see `screen::ensure_permission`.
    #[serde(default)]
    pub screen_granted_version: String,
    #[serde(default)]
    pub collab: CollabSettings,
    #[serde(default)]
    pub workspace_name: String,
    /// Pages open in tabs, restored at start ("continue", the original's default `onStartup`).
    #[serde(default)]
    pub open_tabs: Vec<String>,
    #[serde(default)]
    pub active_tab: usize,
    /// Zoom factor; 0 means never set (default 1.0).
    #[serde(default)]
    pub zoom: f32,
    #[serde(default)]
    pub sidebar_hidden: bool,
    /// "system" (default, like Notion's "Use system setting"), "light" or "dark".
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub hide_connect_card: bool,
    #[serde(default)]
    pub sidebar_width: f32,
    /// Start with no tabs restored (the original's onStartup "default_page").
    #[serde(default)]
    pub start_fresh: bool,
    /// Terminals open in the dock, reopened at start (agents resume their conversation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_terms: Vec<SavedTerm>,
    /// Don't plug the AIs found on this machine into the MCP server at start.
    #[serde(default)]
    pub no_auto_mcp: bool,
    /// AIs the person unplugged in the settings (by key): never plugged back automatically.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ai_off: Vec<String>,
    #[serde(default)]
    pub orch: OrchSettings,
    /// Repository the updates come from (owner/name); empty: the one built in.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub update_repo: String,
    #[serde(default)]
    pub no_update_check: bool,
    /// "" or "auto" (the system's language), "fr", "en".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct SavedTerm {
    pub title: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
}

/// Values read from the original Notion 7.44.0 desktop code (app.asar, .webpack/main/index.js;
/// offsets are byte positions in that file). See MECHANICS.md.
pub mod original {
    /// DEFAULT_WINDOW_WIDTH / HEIGHT, module 772, offset 0xde480.
    pub const DEFAULT_WINDOW_WIDTH: f32 = 1320.0;
    pub const DEFAULT_WINDOW_HEIGHT: f32 = 860.0;
    /// MIN_WINDOW_WIDTH / HEIGHT (gt, mt), offset 0x9c8c.
    pub const MIN_WINDOW_WIDTH: f32 = 640.0;
    pub const MIN_WINDOW_HEIGHT: f32 = 480.0;
    /// ELECTRON_ZOOM_MINIMUM / MAXIMUM / INCREMENT / DEFAULT, offset 0x1be424.
    pub const ZOOM_MIN: f32 = 0.5;
    pub const ZOOM_MAX: f32 = 4.9;
    pub const ZOOM_STEP: f32 = 0.1;
    pub const ZOOM_DEFAULT: f32 = 1.0;
    /// TAB_BAR_HEIGHT_PX, MAX_TAB_WIDTH_PX, MIN_TAB_WIDTH_PX, offset 0x2c73.
    pub const TAB_BAR_HEIGHT: f32 = 36.0;
    pub const TAB_MAX_WIDTH: f32 = 200.0;
    pub const TAB_MIN_WIDTH: f32 = 36.0;

    /// Clamp a zoom factor the way setZoom does, rounding to one decimal.
    pub fn clamp_zoom(z: f32) -> f32 {
        ((z.clamp(ZOOM_MIN, ZOOM_MAX)) * 10.0).round() / 10.0
    }

    /// Colour tokens (rgba) from the design tokens table, offsets 0x1a332e (text),
    /// 0x1a3a2e (light background), 0x1a3bf2 (dark background).
    pub mod light {
        pub const BACKGROUND: [u8; 3] = [255, 255, 255];
        pub const SIDEBAR: [u8; 3] = [249, 248, 247]; // background.secondary
        pub const TERTIARY: [u8; 3] = [240, 239, 237]; // background.tertiary
        pub const INTERACTIVE: [u8; 3] = [244, 243, 243];
        pub const TEXT: [u8; 3] = [44, 44, 43];
        pub const TEXT_SECONDARY: [u8; 3] = [125, 122, 117];
        pub const TEXT_TERTIARY: [u8; 3] = [161, 158, 153];
        pub const BORDER: [u8; 3] = [230, 229, 227];
    }
    pub mod dark {
        pub const BACKGROUND: [u8; 3] = [25, 25, 25];
        pub const SIDEBAR: [u8; 3] = [32, 32, 32];
        pub const TERTIARY: [u8; 3] = [56, 56, 54];
        pub const INTERACTIVE: [u8; 3] = [38, 38, 38];
        pub const TEXT: [u8; 3] = [240, 239, 237];
        pub const TEXT_SECONDARY: [u8; 3] = [173, 169, 163];
        pub const TEXT_TERTIARY: [u8; 3] = [125, 122, 117];
        pub const BORDER: [u8; 3] = [56, 56, 54]; // border.dark.primary, offset 0x1a3bf2-0x1c4
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Workspace {
    #[serde(default)]
    pub pages: Vec<Page>,
    /// Pages deleted for good: id -> time, so a sync doesn't bring them back.
    #[serde(default)]
    pub deleted: BTreeMap<String, u64>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub order_version: u32,
    /// Team discussion of the shared projects: message id -> message (merged by union).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub chat: BTreeMap<String, ChatMsg>,
    /// Inbox: questions from the AIs, end of their work sessions, reports, mentions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbox: Vec<InboxMsg>,
    /// Autonomous work sessions (see auto.rs), finished or not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<RunRecord>,
}

/// One message of the inbox.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct InboxMsg {
    pub id: String,
    pub at: u64,
    /// "question", "fin", "rapport", "info", "erreur".
    pub kind: String,
    /// Project page id, when it is about one.
    #[serde(default)]
    pub project: String,
    pub title: String,
    #[serde(default)]
    pub text: String,
    /// A page to open (a report).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    #[serde(default)]
    pub read: bool,
    /// The person's answer to a question, typed in the inbox (empty: not answered yet).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub answer: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub answered_at: u64,
    /// When an AI last received the answer (get_project_state). Older than `answered_at`:
    /// the answer, or its last edit, has not reached an AI yet.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seen_at: u64,
}

impl InboxMsg {
    /// An answer was given and an AI has received it since its last edit.
    pub fn answer_seen(&self) -> bool {
        !self.answer.trim().is_empty() && self.seen_at > 0 && self.seen_at >= self.answered_at
    }
}

pub(crate) fn is_zero(x: &u64) -> bool {
    *x == 0
}

/// An autonomous work session of an AI on a project, in its own copy of the repository.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct RunRecord {
    pub id: String,
    pub project: String,
    pub title: String,
    pub tool: String,
    /// The repository of the project, the copy (git worktree) and its branch.
    pub root: String,
    pub worktree: String,
    pub branch: String,
    pub started: u64,
    #[serde(default)]
    pub ended: u64,
    /// Why it stopped; empty while it runs.
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub turns: u32,
    /// Share of the 5-hour window it used, in points of percentage.
    #[serde(default)]
    pub pct: f32,
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub steps_done: Vec<String>,
    #[serde(default)]
    pub commits_allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    /// "fusionné" or "abandonné" once dealt with.
    #[serde(default)]
    pub closed: String,
    /// The mission and the project's share it was given, to start it again.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mission: String,
    #[serde(default)]
    pub limit: f32,
    /// Stopped by a quota: starts again by itself at this time (ms); 0 = no.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub resume_at: u64,
    /// Endless mode: the session doesn't stop at the turn limit nor when the steps run out,
    /// only on one of these conditions (and always on the budgets and quotas).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endless: Option<EndConditions>,
}

/// When an endless session stops. Each one is optional; the first reached stops it. The
/// budget of the session, the global limit and the week's reserve always apply on top.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct EndConditions {
    /// After this many hours of work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours: Option<f32>,
    /// Once this many steps are done during the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    /// After this many turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns: Option<u32>,
    /// As soon as the AI says there is nothing left to do ([[FINI]]).
    #[serde(default)]
    pub when_done: bool,
}

impl EndConditions {
    /// Why an endless session stops now, if it does. `elapsed_ms`: since it started;
    /// `done_streak`: how many turns in a row the AI said it had nothing left — twice
    /// stops it anyway, so it never spends the budget going round in circles.
    pub fn reached(&self, elapsed_ms: u64, steps_done: u32, turns: u32, done_streak: u32) -> Option<String> {
        if let Some(h) = self.hours.filter(|h| *h > 0.0)
            && elapsed_ms as f64 >= h as f64 * 3_600_000.0 {
                return Some(crate::trf!("durée atteinte ({h} h)", "time reached ({h} h)"));
            }
        if let Some(n) = self.steps.filter(|n| *n > 0)
            && steps_done >= n {
                return Some(crate::trf!("{n} étape(s) terminée(s)", "{n} step(s) done"));
            }
        if let Some(n) = self.turns.filter(|n| *n > 0)
            && turns >= n {
                return Some(crate::trf!("{n} tours atteints", "{n} turns reached"));
            }
        if done_streak >= 1 && self.when_done {
            return Some(crate::tr!("mission terminée", "mission complete").into());
        }
        if done_streak >= 2 {
            return Some(crate::tr!("plus rien à faire, deux tours de suite", "nothing left to do, two turns in a row").into());
        }
        None
    }

    /// The conditions in a few words, for the session's card and report.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = vec![];
        if let Some(h) = self.hours.filter(|h| *h > 0.0) {
            parts.push(crate::trf!("{h} h", "{h} h"));
        }
        if let Some(n) = self.steps.filter(|n| *n > 0) {
            parts.push(crate::trf!("{n} étape(s)", "{n} step(s)"));
        }
        if let Some(n) = self.turns.filter(|n| *n > 0) {
            parts.push(crate::trf!("{n} tours", "{n} turns"));
        }
        if self.when_done {
            parts.push(crate::tr!("plus d'étape", "no step left").into());
        }
        if parts.is_empty() {
            crate::tr!("budget seulement", "budget only").into()
        } else {
            parts.join(crate::tr!(" ou ", " or "))
        }
    }
}

/// When a session stopped by a quota may start again: one minute after the window
/// resets (Claude Code's figures need a moment to follow).
pub fn resume_time(resets_at: u64) -> u64 {
    resets_at + 60_000
}

/// The sessions due to start again at `now`: the latest per project, none whose project
/// already has a session running.
pub fn due_resumes(runs: &[RunRecord], now: u64, running: &[&str]) -> Vec<usize> {
    let mut out: Vec<usize> = vec![];
    for (i, r) in runs.iter().enumerate().rev() {
        if r.resume_at == 0 || r.resume_at > now || running.contains(&r.project.as_str()) {
            continue;
        }
        if !out.iter().any(|&j| runs[j].project == r.project) {
            out.push(i);
        }
    }
    out
}

/// Whether a question asked (ask_human) about `project` since `since` still waits for the
/// person's answer.
pub fn question_pending(inbox: &[InboxMsg], project: &str, since: u64) -> bool {
    inbox.iter().any(|m| m.kind == "question" && m.project == project && m.answer.trim().is_empty() && m.at >= since)
}

/// How many of a project's latest questions its state of play shows the AIs.
const QUESTIONS_SHOWN: usize = 15;

impl Workspace {
    pub fn unread(&self) -> usize {
        self.inbox.iter().filter(|m| !m.read).count()
    }

    /// The questions the AIs asked about a project and the person's answers, as Markdown
    /// for the AIs (MCP `get_project_state`). Newest first; None when there are none.
    /// The answers `questions_markdown` hands an AI and that no AI had received since their
    /// last edit: marked received at `now`. Returns their questions, for a notice.
    pub fn mark_answers_seen(&mut self, project: &str, now: u64) -> Vec<String> {
        let mut out = vec![];
        for m in self.inbox.iter_mut().rev().filter(|m| m.kind == "question" && m.project == project).take(QUESTIONS_SHOWN) {
            if !m.answer.trim().is_empty() && !m.answer_seen() {
                m.seen_at = now.max(m.answered_at);
                out.push(crate::richtext::one_line(m.text.trim(), 120));
            }
        }
        out
    }

    pub fn questions_markdown(&self, project: &str) -> Option<String> {
        let qs: Vec<&InboxMsg> = self.inbox.iter().rev().filter(|m| m.kind == "question" && m.project == project).take(QUESTIONS_SHOWN).collect();
        if qs.is_empty() {
            return None;
        }
        let mut out = String::new();
        for m in qs {
            let q = m.text.trim().replace('\n', "\n  ");
            if m.answer.trim().is_empty() {
                out.push_str(&format!("- Question : {q}\n  Réponse : (pas encore de réponse)\n"));
            } else {
                let a = m.answer.trim().replace('\n', "\n  ");
                out.push_str(&format!("- Question : {q}\n  Réponse de la personne : {a}\n"));
            }
        }
        Some(out)
    }

    pub fn notify(&mut self, kind: &str, project: &str, title: &str, text: &str, page: Option<String>) {
        self.inbox.push(InboxMsg {
            id: new_id(),
            at: now_ms(),
            kind: kind.into(),
            project: project.into(),
            title: title.into(),
            text: text.into(),
            page,
            read: false,
            ..Default::default()
        });
        // Keep the inbox to a sane size.
        if self.inbox.len() > 500 {
            let cut = self.inbox.len() - 500;
            self.inbox.drain(..cut);
        }
    }
}

/// Autonomous work: limits and habits (page Orchestration).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OrchSettings {
    /// Never go past this share of the 5-hour window (all projects together).
    pub global_limit: f32,
    /// No session starts or goes on once the week's use reaches this. Shown as the reserve
    /// to keep (100 − this): see `reserve`.
    pub week_limit: f32,
    /// The person chose the reserve (until then, the Orchestration page asks for it).
    pub reserve_asked: bool,
    /// Default share one project may use.
    pub default_limit: f32,
    /// The AI may commit on its own branch.
    pub allow_commits: bool,
    /// Local hour of the day's overall report (when several projects ran).
    pub report_hour: u8,
    /// Most turns in one session.
    pub max_turns: u32,
    /// Commands allowed beyond the defaults (Claude Code's syntax, comma separated).
    pub extra_tools: String,
    /// Last day an overall report was written (YYYY-MM-DD).
    pub last_daily: String,
    /// The "Rapports des IA" page.
    pub reports_page: String,
    /// A session stopped by the 5-hour or weekly limit starts again by itself once that
    /// window resets.
    pub auto_resume: bool,
    /// When the AI waits for an answer (a permission, « trust this folder », a question
    /// asked with ask_human), the session pauses for the person instead of refusing or
    /// stopping.
    pub pause_for_answer: bool,
}

impl Default for OrchSettings {
    fn default() -> Self {
        OrchSettings {
            global_limit: 60.0,
            week_limit: 95.0,
            reserve_asked: false,
            default_limit: 20.0,
            allow_commits: false,
            report_hour: 19,
            max_turns: 30,
            extra_tools: String::new(),
            last_daily: String::new(),
            reports_page: String::new(),
            auto_resume: true,
            pause_for_answer: false,
        }
    }
}

impl OrchSettings {
    /// Share of the week's subscription kept for the person, never used by the AIs.
    pub fn reserve(&self) -> f32 {
        (100.0 - self.week_limit).clamp(0.0, 95.0)
    }

    pub fn set_reserve(&mut self, reserve: f32) {
        self.week_limit = 100.0 - reserve.clamp(0.0, 95.0);
    }
}

/// One message of a project's team discussion.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChatMsg {
    pub id: String,
    /// The project page it belongs to.
    pub project: String,
    pub author: String,
    pub text: String,
    pub at: u64,
}

impl Workspace {
    pub fn starter() -> Self {
        let mut ws = Workspace::default();
        ws.settings.workspace_name = crate::tr!("Mon espace", "My workspace").into();
        // Fixed ids and times: two people who start fresh and then sync share these pages
        // instead of getting two copies of each.
        let mut welcome = Page::new(crate::tr!("Bienvenue dans Orchestra !", "Welcome to Orchestra!"), None);
        welcome.id = "start-bienvenue".into();
        welcome.created = 1;
        welcome.updated = 1;
        welcome.icon = "👋".into();
        welcome.blocks = vec![
            {
                let mut b = Block::new(BlockKind::Todo, crate::tr!("Installer Orchestra", "Install Orchestra"));
                b.checked = true;
                b
            },
            Block::new(BlockKind::Todo, crate::tr!("Cliquez n'importe où et **tapez** `/` pour choisir un bloc : titres, tableaux, cases à cocher…", "Click anywhere and **type** `/` to choose a block: headings, tables, checkboxes…")),
            {
                let mut b = Block::new(BlockKind::Todo, crate::tr!("Tapez `/page` pour créer une **sous-page** et y ranger ce que vous voulez", "Type `/page` to create a **subpage** and keep whatever you like in it"));
                b.indent = 1;
                b
            },
            Block::new(BlockKind::Todo, crate::tr!("**Retrouvez et rangez vos pages** dans la barre latérale à gauche 👈", "**Find and organise your pages** in the sidebar on the left 👈")),
            {
                let mut b = Block::new(BlockKind::Toggle, crate::tr!("Ceci est une liste dépliante. Cliquez sur le petit triangle pour d'autres astuces !", "This is a toggle list. Click the small triangle for more tips!"));
                b.extra = crate::tr!("Entrée crée un bloc, Maj+Entrée va à la ligne, Tab décale un bloc vers la droite.\n\
Glissez la poignée ⋯ à gauche d'un bloc pour le déplacer.\n\
Cmd/Ctrl+P ou K pour chercher dans toutes les pages.", "Enter creates a block, Shift+Enter breaks the line, Tab indents a block.\n\
Drag the ⋯ handle left of a block to move it.\n\
Cmd/Ctrl+P or K to search all pages.").into();
                b
            },
            Block::new(BlockKind::Callout, crate::tr!("L'IA passe par MCP : le bouton rond en bas à droite explique comment brancher l'assistant de votre choix.", "AI goes through MCP: the round button at the bottom right explains how to plug in the assistant of your choice.")),
            Block::new(BlockKind::Callout, crate::tr!("La collaboration passe par un branchement que vous choisissez : en direct, dossier partagé ou votre propre commande.", "Collaboration goes through a connection you choose: live, a shared folder or your own command.")),
        ];
        let welcome_id = welcome.id.clone();
        ws.pages.push(welcome);
        let mut tasks = Page::new(crate::tr!("Tâches", "Tasks"), None);
        tasks.id = "start-taches".into();
        tasks.created = 1;
        tasks.updated = 1;
        tasks.icon = "✅".into();
        tasks.blocks.clear();
        tasks.database = Some(Database::new_default());
        ws.pages.push(tasks);
        let mut child = Page::new(crate::tr!("Notes", "Notes"), Some(welcome_id));
        child.id = "start-notes".into();
        child.created = 1;
        child.updated = 1;
        child.icon = "📄".into();
        ws.pages.push(child);
        // The welcome page links to its sub-page, like any parent page.
        if let Some(w) = ws.pages.iter_mut().find(|p| p.id == "start-bienvenue") {
            let mut link = Block::new(BlockKind::PageLink, "");
            link.id = "start-lien-notes".into();
            link.extra = "start-notes".into();
            w.blocks.push(link);
            for (i, b) in w.blocks.iter_mut().enumerate() {
                b.id = format!("start-b{i}");
            }
        }
        if let Some(t) = ws.pages.iter_mut().find(|p| p.id == "start-taches")
            && let Some(db) = t.database.as_mut() {
                for (i, c) in db.columns.iter_mut().enumerate() {
                    let old = std::mem::replace(&mut c.id, format!("start-col{i}"));
                    for r in db.rows.iter_mut() {
                        if let Some(v) = r.cells.remove(&old) {
                            r.cells.insert(c.id.clone(), v);
                        }
                    }
                }
                for (i, r) in db.rows.iter_mut().enumerate() {
                    r.id = format!("start-row{i}");
                }
            }
        ws
    }

    pub fn page(&self, id: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == id)
    }

    pub fn page_mut(&mut self, id: &str) -> Option<&mut Page> {
        self.pages.iter_mut().find(|p| p.id == id)
    }

    /// Find a page by id, or by exact (then partial) title, ignoring case.
    pub fn find_page(&self, key: &str) -> Option<&Page> {
        if let Some(p) = self.page(key) {
            return Some(p);
        }
        let k = key.trim().to_lowercase();
        self.pages
            .iter()
            .filter(|p| !p.trashed)
            .find(|p| p.title.to_lowercase() == k)
            .or_else(|| self.pages.iter().filter(|p| !p.trashed).find(|p| p.title.to_lowercase().contains(&k)))
    }

    pub fn children(&self, parent: Option<&str>) -> Vec<&Page> {
        self.pages.iter().filter(|p| !p.trashed && p.parent.as_deref() == parent).collect()
    }

    pub fn path_of(&self, id: &str) -> Vec<String> {
        let mut out = vec![];
        let mut cur = self.page(id);
        let mut guard = 0;
        while let Some(p) = cur {
            out.push(p.id.clone());
            cur = p.parent.as_deref().and_then(|pid| self.page(pid));
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        out.reverse();
        out
    }

    pub fn descendants(&self, id: &str) -> Vec<String> {
        let mut out = vec![];
        let mut stack = vec![id.to_string()];
        while let Some(cur) = stack.pop() {
            for p in self.pages.iter().filter(|p| p.parent.as_deref() == Some(cur.as_str())) {
                if !out.contains(&p.id) && p.id != id {
                    out.push(p.id.clone());
                    stack.push(p.id.clone());
                }
            }
        }
        out
    }

    pub fn create_page(&mut self, title: &str, parent: Option<String>) -> String {
        let p = Page::new(title, parent.clone());
        let id = p.id.clone();
        self.pages.push(p);
        if let Some(pid) = parent
            && let Some(par) = self.page_mut(&pid) {
                // A project lists its pages in its "Pages" card: no link at the bottom of its text.
                if par.database.is_none() && par.project.is_none() {
                    let mut b = Block::new(BlockKind::PageLink, "");
                    b.extra = id.clone();
                    par.blocks.push(b);
                }
                par.touch();
            }
        id
    }

    /// Put page `id` just before (or, with `after`, just after) page `target` in the order
    /// pages are listed (the sidebar lists the pages of one level in the workspace's order).
    /// False when nothing moved.
    pub fn move_page_near(&mut self, id: &str, target: &str, after: bool) -> bool {
        if id == target || self.page(target).is_none() {
            return false;
        }
        let Some(from) = self.pages.iter().position(|p| p.id == id) else { return false };
        let p = self.pages.remove(from);
        let at = self.pages.iter().position(|p| p.id == target).unwrap_or(self.pages.len()) + usize::from(after);
        let moved = at != from;
        self.pages.insert(at, p);
        moved
    }

    /// Move a page and its sub-pages to the trash (or bring them back).
    pub fn set_trashed(&mut self, id: &str, trashed: bool) {
        let mut ids = self.descendants(id);
        ids.push(id.to_string());
        let t = now_ms();
        for p in self.pages.iter_mut().filter(|p| ids.contains(&p.id)) {
            p.trashed = trashed;
            p.updated = t;
        }
        if !trashed {
            // A restored page whose parent is still in the trash goes to the top level.
            let parent = self.page(id).and_then(|p| p.parent.clone());
            if let Some(par) = parent
                && self.page(&par).map(|p| p.trashed).unwrap_or(true)
                    && let Some(p) = self.page_mut(id) {
                        p.parent = None;
                    }
        }
    }

    pub fn delete_forever(&mut self, id: &str) {
        let mut ids = self.descendants(id);
        ids.push(id.to_string());
        let t = now_ms();
        self.pages.retain(|p| !ids.contains(&p.id));
        for i in ids {
            self.deleted.insert(i, t);
        }
    }

    pub fn duplicate(&mut self, id: &str) -> Option<String> {
        let mut copy = self.page(id)?.clone();
        copy.id = new_id();
        copy.title = format!("{} (copie)", copy.title);
        for b in copy.blocks.iter_mut() {
            b.id = new_id();
        }
        let t = now_ms();
        copy.created = t;
        copy.updated = t;
        let nid = copy.id.clone();
        self.pages.push(copy);
        Some(nid)
    }

    /// Text search over titles, blocks and database cells. Returns (page id, snippet).
    pub fn search(&self, query: &str) -> Vec<(String, String)> {
        let q = query.trim().to_lowercase();
        let mut out = vec![];
        if q.is_empty() {
            for p in self.pages.iter().filter(|p| !p.trashed) {
                out.push((p.id.clone(), String::new()));
            }
            return out;
        }
        for p in self.pages.iter().filter(|p| !p.trashed) {
            if p.title.to_lowercase().contains(&q) {
                out.push((p.id.clone(), String::new()));
                continue;
            }
            let mut hit = p.blocks.iter().map(|b| format!("{} {}", b.text, if b.kind == BlockKind::Toggle { &b.extra } else { "" })).find(|t| t.to_lowercase().contains(&q));
            if hit.is_none()
                && let Some(db) = &p.database {
                    hit = db.rows.iter().flat_map(|r| r.cells.values()).find(|v| v.to_lowercase().contains(&q)).cloned();
                }
            if let Some(h) = hit {
                out.push((p.id.clone(), snippet(&h, &q)));
            }
        }
        out
    }
}

fn snippet(text: &str, q: &str) -> String {
    let lower = text.to_lowercase();
    let chars: Vec<char> = text.chars().collect();
    let pos = lower.find(q).map(|b| lower[..b].chars().count()).unwrap_or(0);
    let start = pos.saturating_sub(30);
    let end = (pos + q.chars().count() + 50).min(chars.len());
    let mut s: String = chars[start..end].iter().collect();
    s = s.replace('\n', " ");
    if start > 0 {
        s = format!("…{s}");
    }
    if end < chars.len() {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::original::*;

    #[test]
    fn a_question_waits_until_answered() {
        let q = |project: &str, at: u64, answer: &str| super::InboxMsg { kind: "question".into(), project: project.into(), at, answer: answer.into(), ..Default::default() };
        let mut inbox = vec![q("p", 100, "")];
        assert!(super::question_pending(&inbox, "p", 50));
        // Another project's question, or one from before the session, doesn't hold it.
        assert!(!super::question_pending(&inbox, "x", 50));
        assert!(!super::question_pending(&inbox, "p", 200));
        inbox[0].answer = "oui".into();
        assert!(!super::question_pending(&inbox, "p", 50));
        // A blank answer isn't one.
        inbox[0].answer = "  ".into();
        assert!(super::question_pending(&inbox, "p", 50));
    }

    #[test]
    fn endless_sessions_stop_on_their_conditions() {
        let none = super::EndConditions::default();
        // No condition: only the budgets stop it, or nothing left to do twice in a row.
        assert!(none.reached(100 * 3_600_000, 50, 500, 0).is_none());
        assert!(none.reached(0, 0, 1, 1).is_none(), "one [[FINI]] isn't enough without « when done »");
        assert!(none.reached(0, 0, 2, 2).is_some());
        let c = super::EndConditions { hours: Some(2.0), steps: Some(3), turns: Some(10), when_done: false };
        assert!(c.reached(3_600_000, 2, 9, 0).is_none());
        assert!(c.reached(2 * 3_600_000, 0, 0, 0).is_some(), "time");
        assert!(c.reached(0, 3, 0, 0).is_some(), "steps");
        assert!(c.reached(0, 0, 10, 0).is_some(), "turns");
        let d = super::EndConditions { when_done: true, ..Default::default() };
        assert!(d.reached(0, 0, 1, 1).is_some());
        // Zero means « not set ».
        let z = super::EndConditions { hours: Some(0.0), steps: Some(0), turns: Some(0), when_done: false };
        assert!(z.reached(10 * 3_600_000, 10, 10, 0).is_none());
        assert!(!c.describe().is_empty() && !none.describe().is_empty());
    }

    #[test]
    fn top_level_pages_are_reordered() {
        let mut ws = super::Workspace::default();
        let ids: Vec<String> = ["A", "B", "C"].iter().map(|t| ws.create_page(t, None)).collect();
        let kid = ws.create_page("A1", Some(ids[0].clone()));
        let roots = |ws: &super::Workspace| ws.children(None).iter().map(|p| p.title.clone()).collect::<String>();
        assert_eq!(roots(&ws), "ABC");
        assert!(ws.move_page_near(&ids[2], &ids[0], false));
        assert_eq!(roots(&ws), "CAB");
        assert!(ws.move_page_near(&ids[2], &ids[1], true));
        assert_eq!(roots(&ws), "ABC");
        assert!(!ws.move_page_near(&ids[2], &ids[2], false));
        assert!(!ws.move_page_near(&ids[0], "absent", false));
        // Sub-pages stay under their parent.
        assert_eq!(ws.children(Some(&ids[0])).iter().map(|p| p.id.clone()).collect::<Vec<_>>(), vec![kid]);
    }

    #[test]
    fn zoom_is_clamped_like_the_original() {
        assert_eq!(clamp_zoom(0.3), ZOOM_MIN);
        assert_eq!(clamp_zoom(9.0), ZOOM_MAX);
        assert_eq!(clamp_zoom(ZOOM_DEFAULT + ZOOM_STEP), 1.1);
        assert_eq!(clamp_zoom(1.0 - 5.0 * ZOOM_STEP), 0.5);
    }

    #[test]
    fn stopped_sessions_resume_once_per_project() {
        use super::{RunRecord, due_resumes, resume_time};
        let run = |p: &str, at: u64| RunRecord { project: p.into(), resume_at: at, ..Default::default() };
        let runs = vec![run("a", resume_time(1_000)), run("a", resume_time(2_000)), run("b", 0), run("c", 999_999_999), run("d", 10)];
        // Due: the latest of "a" and "d"; "b" has none, "c" not yet.
        assert_eq!(due_resumes(&runs, 100_000, &[]), vec![4, 1]);
        // A project already at work isn't started twice.
        assert_eq!(due_resumes(&runs, 100_000, &["a"]), vec![4]);
        assert!(due_resumes(&runs, 5, &[]).is_empty());
    }

    #[test]
    fn answers_reach_the_project_state() {
        let mut ws = super::Workspace::default();
        assert!(ws.questions_markdown("p1").is_none());
        ws.notify("question", "p1", "Question sur X", "Garder l'ancienne API ?", None);
        ws.notify("question", "p2", "Question sur Y", "Autre projet", None);
        ws.notify("info", "p1", "Info", "pas une question", None);
        let md = ws.questions_markdown("p1").unwrap();
        assert!(md.contains("Garder l'ancienne API ?") && md.contains("pas encore de réponse"));
        assert!(!md.contains("Autre projet") && !md.contains("pas une question"));
        ws.inbox[0].answer = "Oui, jusqu'à la v1.".into();
        let md = ws.questions_markdown("p1").unwrap();
        assert!(md.contains("Réponse de la personne : Oui, jusqu'à la v1."));
    }

    /// An answer is marked received once, when an AI gets it; again after an edit; never
    /// an unanswered question nor another project's.
    #[test]
    fn answers_are_marked_received_once() {
        let mut ws = super::Workspace::default();
        ws.notify("question", "p1", "Q", "Garder l'API ?", None);
        ws.notify("question", "p1", "Q", "Sans réponse", None);
        ws.notify("question", "p2", "Q", "Autre projet", None);
        assert!(ws.mark_answers_seen("p1", 10).is_empty(), "nothing answered yet");
        for i in [0, 2] {
            ws.inbox[i].answer = "oui".into();
            ws.inbox[i].answered_at = 5;
        }
        assert_eq!(ws.mark_answers_seen("p1", 10), ["Garder l'API ?"]);
        assert!(ws.inbox[0].answer_seen() && !ws.inbox[1].answer_seen() && !ws.inbox[2].answer_seen());
        assert!(ws.mark_answers_seen("p1", 20).is_empty(), "only once");
        // Edited after: received again at the next state of play.
        ws.inbox[0].answer = "non".into();
        ws.inbox[0].answered_at = 30;
        assert!(!ws.inbox[0].answer_seen());
        assert_eq!(ws.mark_answers_seen("p1", 40).len(), 1);
        assert_eq!(ws.inbox[0].seen_at, 40);
    }
}
