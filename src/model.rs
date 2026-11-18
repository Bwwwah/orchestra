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
        if let Some(c) = self.columns.iter().find(|c| c.kind == ColumnKind::Select) {
            if let Some(o) = c.options.first() {
                row.cells.insert(c.id.clone(), o.clone());
            }
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
    #[serde(default)]
    pub live_code: String,
    /// Live was switched on (it starts again with the app).
    #[serde(default)]
    pub live_on: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub dark: bool,
    #[serde(default)]
    pub collab: CollabSettings,
    #[serde(default)]
    pub workspace_name: String,
    /// Pages open in tabs, restored at start (the default).
    #[serde(default)]
    pub open_tabs: Vec<String>,
    #[serde(default)]
    pub active_tab: usize,
    /// Zoom factor; 0 means never set (default 1.0).
    #[serde(default)]
    pub zoom: f32,
    #[serde(default)]
    pub sidebar_hidden: bool,
    /// "system" (default, follows the system), "light" or "dark".
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub hide_connect_card: bool,
    #[serde(default)]
    pub sidebar_width: f32,
    /// Start with no tabs restored.
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

/// The app's design values: window, zoom, tabs and colours.
pub mod design {
    /// Default window size.
    pub const DEFAULT_WINDOW_WIDTH: f32 = 1320.0;
    pub const DEFAULT_WINDOW_HEIGHT: f32 = 860.0;
    /// Minimum window size.
    pub const MIN_WINDOW_WIDTH: f32 = 640.0;
    pub const MIN_WINDOW_HEIGHT: f32 = 480.0;
    /// Zoom range, step and default.
    pub const ZOOM_MIN: f32 = 0.5;
    pub const ZOOM_MAX: f32 = 4.9;
    pub const ZOOM_STEP: f32 = 0.1;
    pub const ZOOM_DEFAULT: f32 = 1.0;
    /// Tab strip height and tab width range.
    pub const TAB_BAR_HEIGHT: f32 = 36.0;
    pub const TAB_MAX_WIDTH: f32 = 200.0;
    pub const TAB_MIN_WIDTH: f32 = 36.0;

    /// Clamp a zoom factor to the range, rounding to one decimal.
    pub fn clamp_zoom(z: f32) -> f32 {
        ((z.clamp(ZOOM_MIN, ZOOM_MAX)) * 10.0).round() / 10.0
    }

    /// Colour tokens (rgb) for the light and dark themes.
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
        pub const BORDER: [u8; 3] = [56, 56, 54];
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
    /// "question", "fin", "rapport", "info".
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
}

impl Workspace {
    pub fn unread(&self) -> usize {
        self.inbox.iter().filter(|m| !m.read).count()
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
}

impl Default for OrchSettings {
    fn default() -> Self {
        OrchSettings {
            global_limit: 60.0,
            default_limit: 20.0,
            allow_commits: false,
            report_hour: 19,
            max_turns: 30,
            extra_tools: String::new(),
            last_daily: String::new(),
            reports_page: String::new(),
        }
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
            Block::new(BlockKind::Callout, crate::tr!("L'IA passe par MCP : Paramètres → IA (MCP) explique comment brancher l'assistant de votre choix.", "AI goes through MCP: Settings → AI (MCP) explains how to plug in the assistant of your choice.")),
            Block::new(BlockKind::Callout, crate::tr!("La collaboration passe par un branchement que vous choisissez : dossier partagé ou votre propre commande.", "Collaboration goes through a connection you choose: live, a shared folder or your own command.")),
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
        if let Some(t) = ws.pages.iter_mut().find(|p| p.id == "start-taches") {
            if let Some(db) = t.database.as_mut() {
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
        if let Some(pid) = parent {
            if let Some(par) = self.page_mut(&pid) {
                if par.database.is_none() {
                    let mut b = Block::new(BlockKind::PageLink, "");
                    b.extra = id.clone();
                    par.blocks.push(b);
                }
                par.touch();
            }
        }
        id
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
            if let Some(par) = parent {
                if self.page(&par).map(|p| p.trashed).unwrap_or(true) {
                    if let Some(p) = self.page_mut(id) {
                        p.parent = None;
                    }
                }
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
            if hit.is_none() {
                if let Some(db) = &p.database {
                    hit = db.rows.iter().flat_map(|r| r.cells.values()).find(|v| v.to_lowercase().contains(&q)).cloned();
                }
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
    use super::design::*;

    #[test]
    fn zoom_is_clamped() {
        assert_eq!(clamp_zoom(0.3), ZOOM_MIN);
        assert_eq!(clamp_zoom(9.0), ZOOM_MAX);
        assert_eq!(clamp_zoom(ZOOM_DEFAULT + ZOOM_STEP), 1.1);
        assert_eq!(clamp_zoom(1.0 - 5.0 * ZOOM_STEP), 0.5);
    }
}
