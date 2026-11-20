//! The window: sidebar with the page tree, block editor, databases, quick search,
//! trash, and the AI (MCP) and collaboration panels.

use crate::collab;
use crate::model::*;
use crate::store;
use eframe::egui::{self, Color32, FontId, Key, RichText};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime};

const ICONS: [&str; 20] = [
    "📄", "📝", "✅", "📌", "💡", "📚", "🎯", "📅", "🏠", "⭐", "🔥", "🔑", "💼", "🎨", "🔧", "🌱", "🚀", "❤", "🎵", "✈",
];

#[derive(PartialEq, Clone, Copy)]
enum View {
    Page,
    Home,
    Trash,
    Projects,
    Orchestra,
    Inbox,
}

enum Action {
    Open(String),
    NewPage(Option<String>),
    NewDatabase(Option<String>),
    ToggleExpand(String),
    Trash(String),
    Restore(String),
    DeleteForever(String),
    Duplicate(String),
    OpenRow(String, String),
    InsertSubpage { page: String, at: usize },
    /// "/kanban", "/base"…: a sub-page holding a database shown in `view`, linked at block `at`.
    InsertDatabase { page: String, at: usize, view: DbView },
    Export(String),
    NewProject(String, String),
    RevertDoc(String),
    LocalCopy(String),
}

pub struct NotesApp {
    ws: Workspace,
    current: Option<String>,
    view: View,
    expanded: HashSet<String>,
    focus: Option<(String, Option<usize>)>,
    search_open: bool,
    search_query: String,
    search_focus: bool,
    search_ask: search::Ask,
    search_sel: usize,
    /// Popups on screen (see notices.rs); the messages themselves go to the inbox.
    toasts: Vec<notices::Toast>,
    /// Messages raised while the workspace is borrowed, shown at the end of the frame.
    said: Vec<notices::Notice>,
    toast_seq: u64,
    snapshot: HashMap<String, String>,
    settings_snapshot: String,
    dirty_since: Option<Instant>,
    known_mtime: Option<SystemTime>,
    /// Inbox messages deleted here: a merge with the file on disk must not bring them back.
    inbox_removed: HashSet<String>,
    last_poll: Instant,
    last_sync: Instant,
    sync_log: Vec<String>,
    export_dir: String,
    import_path: String,
    new_col: (String, ColumnKind),
    applied_theme: Option<egui::ThemePreference>,
    shot: Option<crate::shot::Shot>,
    settings_tab: Option<SettingsTab>,
    settings_search: String,
    ctx: egui::Context,
    live: Option<crate::live::Live>,
    peers: std::collections::BTreeMap<String, live_ui::Peer>,
    live_sent: HashMap<String, u64>,
    live_deleted_sent: usize,
    live_presence: Option<(Option<String>, Instant)>,
    tabs: Vec<Tab>,
    active_tab: usize,
    closed_tabs: Vec<Tab>,
    /// "Nouveau projet" form: (name, folder).
    new_project: Option<(String, String)>,
    /// Text of the "add a step" field on a project page.
    new_step: String,
    /// Claude Code / Codex sessions read live from disk, and which project each belongs to.
    watcher: Option<crate::sessions::Watcher>,
    agents: projects::LiveIndex,
    /// "Synchroniser avec les IA": the folders offered, then the run in progress.
    sync_dialog: Option<Vec<projects::SyncCandidate>>,
    sync_run: Option<crate::sync::SyncRun>,
    /// Terminals in the bottom dock.
    terms: Vec<crate::terminal::Term>,
    term_active: usize,
    term_open: bool,
    term_focus: bool,
    /// What the start-up MCP setup did (filled by a background thread).
    mcp_report: std::sync::Arc<std::sync::Mutex<Option<crate::setup::Report>>>,
    mcp_report_shown: bool,
    ais: ai_ui::Ais,
    orch: orch_ui::Orch,
    upd: update_ui::Upd,
    versions: versions::Versions,
    /// Last look at the projects' documentation files.
    docs_checked: Option<Instant>,
    /// "Regard extérieur" asked to an AI, per project page, while it runs.
    reviews: HashMap<String, projects::ReviewJob>,
    ai: ai_fill::AiJobs,
    /// Discussion messages already sent to the others (real time).
    chat_sent: HashSet<String>,
    /// Right-hand panel (discussion / AI conversation) open, and which tab.
    side_open: bool,
    side_tab: u8,
    chat_draft: String,
    ai_draft: String,
    ai_session: Option<String>,
    transcripts: HashMap<String, (u64, std::sync::Arc<Vec<crate::sessions::Turn>>)>,
    /// Space taken this frame by the side panel (right) and the terminal dock (bottom).
    side_w: f32,
    dock_h: f32,
}

/// One tab: the page it shows and its back/forward history
/// (isNavigationHistoryEnabled is on by default in the original).
#[derive(Clone, Default)]
struct Tab {
    page: Option<String>,
    back: Vec<String>,
    fwd: Vec<String>,
}

impl NotesApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<String>) -> Self {
        // The system's language first (for a brand-new workspace), then the chosen one.
        crate::i18n::set("auto");
        let ws = store::load();
        crate::i18n::set(&ws.settings.language);
        let first = ws.pages.iter().find(|p| !p.trashed && p.parent.is_none()).map(|p| p.id.clone());
        let export_dir = dirs::document_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("Orchestra - export")
            .display()
            .to_string();
        let mut app = NotesApp {
            ws,
            current: first,
            view: View::Page,
            expanded: HashSet::new(),
            focus: None,
            search_open: false,
            search_query: String::new(),
            search_focus: false,
            search_ask: Default::default(),
            search_sel: 0,
            toasts: vec![],
            said: vec![],
            toast_seq: 0,
            snapshot: HashMap::new(),
            settings_snapshot: String::new(),
            dirty_since: None,
            known_mtime: None,
            inbox_removed: HashSet::new(),
            last_poll: Instant::now(),
            last_sync: Instant::now(),
            sync_log: vec![],
            export_dir,
            import_path: String::new(),
            new_col: (String::new(), ColumnKind::Text),
            applied_theme: None,
            shot: crate::shot::Shot::from_env(),
            settings_tab: None,
            settings_search: String::new(),
            ctx: cc.egui_ctx.clone(),
            live: None,
            peers: Default::default(),
            live_sent: HashMap::new(),
            live_deleted_sent: 0,
            live_presence: None,
            tabs: vec![],
            active_tab: 0,
            closed_tabs: vec![],
            new_project: None,
            new_step: String::new(),
            watcher: None,
            agents: Default::default(),
            sync_dialog: None,
            sync_run: None,
            terms: vec![],
            term_active: 0,
            term_open: false,
            term_focus: false,
            mcp_report: Default::default(),
            mcp_report_shown: false,
            ais: Default::default(),
            orch: Default::default(),
            upd: Default::default(),
            versions: Default::default(),
            docs_checked: None,
            reviews: HashMap::new(),
            ai: ai_fill::AiJobs::default(),
            chat_sent: HashSet::new(),
            side_open: false,
            side_tab: 0,
            chat_draft: String::new(),
            ai_draft: String::new(),
            ai_session: None,
            transcripts: HashMap::new(),
            side_w: 0.0,
            dock_h: 0.0,
        };
        app.restore_terminals(&cc.egui_ctx);
        if std::env::var_os("ORCHESTRA_NO_SESSIONS").is_none() {
            let plug = !app.ws.settings.no_auto_mcp;
            app.refresh_ais(&cc.egui_ctx, plug);
        }
        if std::env::var_os("ORCHESTRA_NO_SESSIONS").is_none() {
            let ctx = cc.egui_ctx.clone();
            app.watcher = Some(crate::sessions::Watcher::start(move || ctx.request_repaint()));
        }
        // onStartup "continue": reopen the tabs that were open last time.
        let alive = |id: &String| app.ws.page(id).map(|p| !p.trashed).unwrap_or(false);
        let restored: Vec<Tab> = app
            .ws
            .settings
            .open_tabs
            .iter()
            .filter(|id| alive(id))
            .map(|id| Tab { page: Some(id.clone()), ..Default::default() })
            .collect();
        if restored.is_empty() || app.ws.settings.start_fresh {
            app.tabs = vec![Tab { page: app.current.clone(), ..Default::default() }];
            app.active_tab = 0;
        } else {
            app.active_tab = app.ws.settings.active_tab.min(restored.len() - 1);
            app.tabs = restored;
            app.current = app.tabs[app.active_tab].page.clone();
        }
        if let Some(id) = open.filter(|id| alive(id)) {
            app.navigate(id);
        }
        // Developer aid for screenshots: ORCHESTRA_DEMO=settings | ai | collab | slash[:text].
        if let Ok(demo) = std::env::var("ORCHESTRA_DEMO") {
            match demo.split(':').next().unwrap_or("") {
                "settings" => app.settings_tab = Some(SettingsTab::Preferences),
                "sidebar-settings" => app.settings_tab = Some(SettingsTab::Sidebar),
                "ai" => app.settings_tab = Some(SettingsTab::Ai),
                "collab" => app.settings_tab = Some(SettingsTab::Collab),
                "projects" => app.view = View::Projects,
                "orchestra" => app.view = View::Orchestra,
                "inbox" => app.view = View::Inbox,
                "side" => app.side_open = true,
                "fill" | "tasks" => {
                    if let Some(c) = app.current.clone() {
                        let folder = app.ws.page(&c).and_then(|p| p.project.as_ref()).map(|p| p.folder.clone()).unwrap_or_default();
                        let r = if demo.starts_with("fill") { ai_fill::AiReq::Fill { page_id: c, folder, prefer: None } } else { ai_fill::AiReq::Tasks { page_id: c, folder, existing: vec![], prefer: None, text: None, images: vec![] } };
                        ai_fill::request(&cc.egui_ctx, r);
                    }
                }
                "proposals" => {
                    if let Some(c) = app.current.clone() {
                        app.ai.proposals.insert(c, vec![("UI : refaire la page d'accueil".into(), true), ("Écrire les tests de l'API".into(), true), ("Publier la version 1.0".into(), false)]);
                    }
                }
                "search" => {
                    app.search_open = true;
                    app.search_focus = true;
                    app.search_query = demo.split_once(':').map(|x| x.1.to_string()).unwrap_or_default();
                }
                "terminal" => {
                    let cmd = std::env::var("ORCHESTRA_DEMO_CMD").ok();
                    term_ui::request(&cc.egui_ctx, "Claude Code · bind", "/tmp", cmd);
                }
                "sync" => {
                    let mut sc = crate::sessions::Scanner::new(crate::sessions::default_roots());
                    sc.scan();
                    app.agents.refresh(&app.ws, std::sync::Arc::new(sc.sessions()));
                    app.view = View::Projects;
                    app.sync_dialog = Some(app.sync_candidates());
                }
                "slash" => {
                    let text = format!("/{}", demo.split_once(':').map(|x| x.1).unwrap_or(""));
                    if let Some(page) = app.current.clone().and_then(|c| app.ws.page_mut(&c).map(|p| p.id.clone())) {
                        let b = Block::new(BlockKind::Paragraph, text);
                        let id = b.id.clone();
                        if let Some(p) = app.ws.page_mut(&page) {
                            p.blocks.insert(1.min(p.blocks.len()), b);
                        }
                        app.focus = Some((id, None));
                    }
                }
                _ => {}
            }
        }
        // Real-time collaboration that was on last time starts again.
        if app.ws.settings.collab.connector == ConnectorKind::Live && app.ws.settings.collab.live_on {
            app.start_live();
        }
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let z = if app.ws.settings.zoom <= 0.0 { original::ZOOM_DEFAULT } else { original::clamp_zoom(app.ws.settings.zoom) };
        cc.egui_ctx.set_zoom_factor(z);
        if !store::workspace_path().exists() {
            app.save_now();
        }
        app.known_mtime = store::modified(&store::workspace_path());
        app.take_snapshot();
        setup_fonts(&cc.egui_ctx);
        setup_style(&cc.egui_ctx);
        // Apply the chosen theme before the first frame (not the system's).
        let pref = app.theme_preference();
        cc.egui_ctx.set_theme(pref);
        app.applied_theme = Some(pref);
        app
    }

    fn take_snapshot(&mut self) {
        self.snapshot = self.ws.pages.iter().map(|p| (p.id.clone(), page_key(p))).collect();
        self.settings_snapshot = serde_json::to_string(&(&self.ws.settings, &self.ws.deleted)).unwrap_or_default();
    }

    /// Notice edits: pages whose content changed get a new `updated` time, and a save is scheduled.
    fn detect_changes(&mut self) {
        let mut changed = false;
        let t = now_ms();
        let ids: HashSet<&String> = self.ws.pages.iter().map(|p| &p.id).collect();
        if ids.len() != self.snapshot.len() || self.snapshot.keys().any(|k| !ids.contains(k)) {
            changed = true;
        }
        for p in self.ws.pages.iter_mut() {
            let key = page_key(p);
            match self.snapshot.get(&p.id) {
                Some(old) if *old == key => {}
                Some(old) => {
                    // Stamp the blocks that changed and remember the ones removed, so merges with
                    // other people work block by block.
                    if let Ok(oldp) = serde_json::from_str::<Page>(old) {
                        let before: HashMap<&str, String> = oldp.blocks.iter().map(|b| (b.id.as_str(), block_key(b))).collect();
                        for b in p.blocks.iter_mut() {
                            if before.get(b.id.as_str()) != Some(&block_key(b)) {
                                b.updated = t;
                            }
                        }
                        let now: HashSet<&str> = p.blocks.iter().map(|b| b.id.as_str()).collect();
                        let gone: Vec<String> = oldp.blocks.iter().filter(|b| !now.contains(b.id.as_str())).map(|b| b.id.clone()).collect();
                        for id in gone {
                            p.deleted_blocks.insert(id, t);
                        }
                    }
                    p.updated = t.max(p.updated);
                    changed = true;
                }
                None => changed = true,
            }
        }
        let s = serde_json::to_string(&(&self.ws.settings, &self.ws.deleted)).unwrap_or_default();
        if s != self.settings_snapshot {
            changed = true;
        }
        if changed {
            self.take_snapshot();
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    fn save_now(&mut self) {
        // Written meanwhile by someone else (an AI through MCP, during autonomous work):
        // merge it in first rather than overwrite it.
        let disk_m = store::modified(&store::workspace_path());
        if disk_m.is_some() && disk_m != self.known_mtime {
            let disk = store::load();
            crate::collab::merge(&mut self.ws, &disk);
            for m in &disk.inbox {
                match self.ws.inbox.iter_mut().find(|x| x.id == m.id) {
                    // An AI received the answer (written on disk by the MCP server).
                    Some(x) => x.seen_at = x.seen_at.max(m.seen_at),
                    None if !self.inbox_removed.contains(&m.id) => self.ws.inbox.push(m.clone()),
                    None => {}
                }
            }
            self.ws.inbox.sort_by_key(|m| m.at);
            self.take_snapshot();
        }
        match store::save(&self.ws) {
            Ok(()) => {
                self.known_mtime = store::modified(&store::workspace_path());
                self.dirty_since = None;
            }
            Err(e) => self.fail(crate::trf!("Enregistrement impossible : {e}", "Couldn't save: {e}")),
        }
    }

    /// Pick up changes written by someone else (the MCP server, a sync).
    fn poll_file(&mut self) {
        if self.last_poll.elapsed() < Duration::from_millis(1200) || self.dirty_since.is_some() {
            return;
        }
        self.last_poll = Instant::now();
        let m = store::modified(&store::workspace_path());
        if m.is_some() && m != self.known_mtime {
            self.ws = store::load();
            self.known_mtime = m;
            self.take_snapshot();
            if let Some(c) = &self.current {
                if self.ws.page(c).is_none() {
                    self.current = None;
                }
            }
        }
    }

    /// Sync with the team. `by_hand`: asked from the settings (a short confirmation then);
    /// the automatic one every 30 s stays silent unless it fails.
    fn run_sync(&mut self, by_hand: bool) {
        self.last_sync = Instant::now();
        self.detect_changes();
        let res = collab::sync(&mut self.ws);
        let line = match &res {
            Ok(r) => r.clone(),
            Err(e) => crate::trf!("Échec : {e}", "Failed: {e}"),
        };
        self.sync_log.insert(0, line.clone());
        self.sync_log.truncate(20);
        match res {
            Ok(_) if by_hand => self.ack(line),
            Ok(_) => {}
            Err(e) => self.fail(crate::trf!("Synchronisation impossible : {e}", "Sync failed: {e}")),
        }
        // Merged pages keep their own times: refresh the snapshot without touching them.
        self.take_snapshot();
        self.save_now();
    }

    /// "system" (default, like Notion), "light" or "dark"; older files only had `dark`.
    fn theme_preference(&self) -> egui::ThemePreference {
        match self.ws.settings.theme.as_str() {
            "light" => egui::ThemePreference::Light,
            "dark" => egui::ThemePreference::Dark,
            _ if self.ws.settings.dark => egui::ThemePreference::Dark,
            _ => egui::ThemePreference::System,
        }
    }

    /// Show a page in the active tab, remembering where we came from.
    fn navigate(&mut self, id: String) {
        self.view = View::Page;
        if self.current.as_deref() == Some(id.as_str()) {
            return;
        }
        if let Some(prev) = self.current.take() {
            if let Some(t) = self.tabs.get_mut(self.active_tab) {
                t.back.push(prev);
                t.fwd.clear();
            }
        }
        self.current = Some(id);
    }

    fn go_back(&mut self) {
        let alive: Vec<String> = self.ws.pages.iter().filter(|p| !p.trashed).map(|p| p.id.clone()).collect();
        let Some(t) = self.tabs.get_mut(self.active_tab) else { return };
        while let Some(prev) = t.back.pop() {
            if alive.contains(&prev) {
                if let Some(c) = self.current.take() {
                    t.fwd.push(c);
                }
                self.current = Some(prev);
                self.view = View::Page;
                return;
            }
        }
    }

    fn go_forward(&mut self) {
        let alive: Vec<String> = self.ws.pages.iter().filter(|p| !p.trashed).map(|p| p.id.clone()).collect();
        let Some(t) = self.tabs.get_mut(self.active_tab) else { return };
        while let Some(next) = t.fwd.pop() {
            if alive.contains(&next) {
                if let Some(c) = self.current.take() {
                    t.back.push(c);
                }
                self.current = Some(next);
                self.view = View::Page;
                return;
            }
        }
    }

    /// Store the active tab's page back into the tab list, then switch.
    fn select_tab(&mut self, i: usize) {
        if self.tabs.is_empty() {
            return;
        }
        let i = i.min(self.tabs.len() - 1);
        self.tabs[self.active_tab].page = self.current.clone();
        self.active_tab = i;
        self.current = self.tabs[i].page.clone();
        self.view = View::Page;
    }

    /// New tab (CmdOrCtrl+T). isNewTabSearchEnabled is on by default: it opens on search.
    fn new_tab(&mut self) {
        self.tabs[self.active_tab].page = self.current.clone();
        self.tabs.insert(self.active_tab + 1, Tab::default());
        self.active_tab += 1;
        self.current = None;
        self.view = View::Page;
        self.search_open = true;
        self.search_focus = true;
    }

    /// Close a tab (CmdOrCtrl+W). The last tab stays, emptied of nothing: it keeps its page.
    fn close_tab(&mut self, i: usize) {
        if self.tabs.len() <= 1 {
            // The last tab stays, emptied: it shows the home page.
            if let Some(t) = self.tabs.first().cloned() {
                let mut t = t;
                t.page = self.current.clone();
                self.closed_tabs.push(t);
            }
            self.current = None;
            if let Some(t) = self.tabs.first_mut() {
                t.page = None;
                t.back.clear();
                t.fwd.clear();
            }
            return;
        }
        self.tabs[self.active_tab].page = self.current.clone();
        let t = self.tabs.remove(i);
        self.closed_tabs.push(t);
        if self.active_tab > i || self.active_tab >= self.tabs.len() {
            self.active_tab = self.active_tab.saturating_sub(1);
        }
        self.current = self.tabs[self.active_tab].page.clone();
    }

    /// Reopen the last closed tab (CmdOrCtrl+Shift+T).
    fn reopen_tab(&mut self) {
        if let Some(t) = self.closed_tabs.pop() {
            self.tabs[self.active_tab].page = self.current.clone();
            self.tabs.insert(self.active_tab + 1, t);
            self.active_tab += 1;
            self.current = self.tabs[self.active_tab].page.clone();
            self.view = View::Page;
        }
    }

    fn set_zoom(&mut self, ctx: &egui::Context, z: f32) {
        let z = original::clamp_zoom(z);
        ctx.set_zoom_factor(z);
        self.ws.settings.zoom = z;
        self.ack(crate::trf!("Zoom {} %", "Zoom {} %", (z * 100.0).round()));
    }

    /// The desktop menu's shortcuts, as the original defines them (MECHANICS.md, "Shortcuts").
    fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        use egui::Modifiers as M;
        let cmd = M::COMMAND;
        let cmd_shift = M::COMMAND | M::SHIFT;
        let cmd_alt = M::COMMAND | M::ALT;
        let ctrl = M::CTRL;
        let ctrl_shift = M::CTRL | M::SHIFT;
        let mac = cfg!(target_os = "macos");
        let mut ev: Vec<&'static str> = vec![];
        ctx.input_mut(|i| {
            let mut k = |m: M, key: Key, name: &'static str| {
                if i.consume_key(m, key) {
                    ev.push(name);
                }
            };
            // Order matters: the longer combinations first.
            k(cmd_shift, Key::T, "reopen_tab");
            if mac {
                // Hidden macOS-only menu items in the original (j = darwin).
                k(cmd_shift, Key::OpenBracket, "prev_tab");
                k(cmd_shift, Key::CloseBracket, "next_tab");
                k(cmd_alt, Key::ArrowLeft, "prev_tab");
                k(cmd_alt, Key::ArrowRight, "next_tab");
            }
            k(ctrl_shift, Key::Tab, "prev_tab");
            k(ctrl, Key::Tab, "next_tab");
            k(cmd_alt, Key::L, "copy_named_link");
            k(cmd, Key::T, "new_tab");
            k(cmd, Key::W, "close_tab");
            k(cmd, Key::L, "copy_link");
            k(cmd, Key::Backslash, "sidebar");
            k(cmd, Key::OpenBracket, "back");
            k(cmd, Key::CloseBracket, "forward");
            k(cmd, Key::Plus, "zoom_in");
            k(cmd, Key::Equals, "zoom_in");
            k(cmd, Key::Minus, "zoom_out");
            k(cmd, Key::Num0, "zoom_reset");
            k(cmd, Key::P, "search");
            k(cmd, Key::K, "search");
            k(cmd, Key::N, "new_page");
            let names = ["tab1", "tab2", "tab3", "tab4", "tab5", "tab6", "tab7", "tab8"];
            for (n, key) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8].into_iter().enumerate() {
                k(cmd, key, names[n]);
            }
            k(cmd, Key::Num9, "last_tab");
        });
        for e in ev {
            match e {
                "reopen_tab" => self.reopen_tab(),
                "prev_tab" => {
                    let n = self.tabs.len();
                    self.select_tab((self.active_tab + n - 1) % n);
                }
                "next_tab" => {
                    let n = self.tabs.len();
                    self.select_tab((self.active_tab + 1) % n);
                }
                "new_tab" => self.new_tab(),
                "close_tab" => self.close_tab(self.active_tab),
                "copy_link" | "copy_named_link" => {
                    if let Some(p) = self.current.as_ref().and_then(|c| self.ws.page(c)) {
                        let link = format!("orchestra://page/{}", p.id);
                        let text = if e == "copy_named_link" { format!("[{}]({link})", p.display_title()) } else { link };
                        ctx.copy_text(text);
                        self.ack(crate::tr!("Lien copié.", "Link copied."));
                    }
                }
                "sidebar" => self.ws.settings.sidebar_hidden = !self.ws.settings.sidebar_hidden,
                "back" => self.go_back(),
                "forward" => self.go_forward(),
                "zoom_in" => self.set_zoom(ctx, ctx.zoom_factor() + original::ZOOM_STEP),
                "zoom_out" => self.set_zoom(ctx, ctx.zoom_factor() - original::ZOOM_STEP),
                "zoom_reset" => self.set_zoom(ctx, original::ZOOM_DEFAULT),
                "search" => {
                    self.search_open = true;
                    self.search_focus = true;
                }
                "new_page" => actions.push(Action::NewPage(None)),
                "last_tab" => self.select_tab(self.tabs.len() - 1),
                t if t.starts_with("tab") => {
                    let n: usize = t[3..].parse().unwrap_or(1);
                    if n <= self.tabs.len() {
                        self.select_tab(n - 1);
                    }
                }
                _ => {}
            }
        }
    }

    fn apply(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Open(id) => {
                    for anc in self.ws.path_of(&id) {
                        self.expanded.insert(anc);
                    }
                    self.navigate(id);
                    self.view = View::Page;
                }
                Action::NewPage(parent) => {
                    let id = self.ws.create_page("", parent.clone());
                    if let Some(p) = parent {
                        self.expanded.insert(p);
                    }
                    self.navigate(id.clone());
                    self.view = View::Page;
                    self.focus = Some((format!("title-{id}"), None));
                }
                Action::NewDatabase(parent) => {
                    let id = self.ws.create_page(crate::tr!("Base de données", "Database"), parent);
                    if let Some(p) = self.ws.page_mut(&id) {
                        p.blocks.clear();
                        p.database = Some(Database::new_default());
                        p.icon = "📋".into();
                    }
                    self.navigate(id);
                    self.view = View::Page;
                }
                Action::ToggleExpand(id) => {
                    if !self.expanded.remove(&id) {
                        self.expanded.insert(id);
                    }
                }
                Action::Trash(id) => {
                    let parent = self.ws.page(&id).and_then(|p| p.parent.clone());
                    self.ws.set_trashed(&id, true);
                    let gone = self.current.as_ref().map(|c| self.ws.page(c).map(|p| p.trashed).unwrap_or(true)).unwrap_or(false);
                    if gone {
                        self.current = parent
                            .filter(|p| self.ws.page(p).map(|x| !x.trashed).unwrap_or(false))
                            .or_else(|| self.ws.pages.iter().find(|p| !p.trashed).map(|p| p.id.clone()));
                    }
                    self.ack(crate::tr!("Page placée dans la corbeille.", "Page moved to the trash."));
                }
                Action::Restore(id) => {
                    self.ws.set_trashed(&id, false);
                    self.ack(crate::tr!("Page restaurée.", "Page restored."));
                }
                Action::DeleteForever(id) => {
                    self.ws.delete_forever(&id);
                }
                Action::Duplicate(id) => {
                    if let Some(n) = self.ws.duplicate(&id) {
                        self.navigate(n);
                    }
                }
                Action::Export(id) => {
                    if let Some(p) = self.ws.page(&id) {
                        let dir = std::path::PathBuf::from(&self.export_dir);
                        let file = dir.join(format!("{}.md", store::safe_name(&p.display_title())));
                        let md = store::page_to_markdown(&self.ws, p);
                        self.notice(notices::Notice::from_result(match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, md)) {
                            Ok(()) => Ok(crate::trf!("Exporté : {}", "Exported: {}", file.display())),
                            Err(e) => Err(crate::trf!("Export impossible : {e}", "Export failed: {e}")),
                        }));
                    }
                }
                Action::OpenRow(db_page, row) => {
                    let found = self.ws.page(&db_page).and_then(|p| p.database.as_ref()).and_then(|db| {
                        db.rows.iter().find(|r| r.id == row).map(|r| (r.page.clone(), db.row_title(r)))
                    });
                    let Some((existing, title)) = found else { continue };
                    let existing = existing.filter(|pid| self.ws.page(pid).map(|p| !p.trashed).unwrap_or(false));
                    let target = match existing {
                        Some(pid) => pid,
                        None => {
                            let pid = self.ws.create_page(&title, Some(db_page.clone()));
                            if let Some(r) = self
                                .ws
                                .page_mut(&db_page)
                                .and_then(|p| p.database.as_mut())
                                .and_then(|d| d.rows.iter_mut().find(|r| r.id == row))
                            {
                                r.page = Some(pid.clone());
                            }
                            pid
                        }
                    };
                    self.navigate(target);
                    self.view = View::Page;
                }
                Action::LocalCopy(id) => {
                    if let Some(n) = self.ws.local_copy(&id) {
                        self.info(crate::tr!("Copie locale créée : elle reste privée et ne suit plus le projet partagé.", "Local copy created: it stays private and no longer follows the shared project."));
                        self.navigate(n);
                    }
                }
                Action::RevertDoc(id) => match crate::docs::revert_to_file(&mut self.ws, &id) {
                    Ok(()) => self.ack(crate::tr!("Page remise à l'identique du fichier.", "Page reset to match the file.")),
                    Err(e) => self.fail(crate::trf!("Impossible de relire le fichier : {e}", "Couldn't read the file again: {e}")),
                },
                Action::NewProject(title, folder) => {
                    let id = self.ws.create_project(&title, &folder);
                    crate::docs::sync_project(&mut self.ws, &id);
                    self.navigate(id);
                    self.view = View::Page;
                }
                Action::InsertSubpage { page, at } => {
                    let p = Page::new("", Some(page.clone()));
                    let id = p.id.clone();
                    self.ws.pages.push(p);
                    if let Some(par) = self.ws.page_mut(&page) {
                        if let Some(b) = par.blocks.get_mut(at) {
                            b.kind = BlockKind::PageLink;
                            b.text.clear();
                            b.extra = id.clone();
                        }
                    }
                    self.expanded.insert(page);
                    self.navigate(id.clone());
                    self.focus = Some((format!("title-{id}"), None));
                }
                Action::InsertDatabase { page, at, view } => {
                    let id = insert_database(&mut self.ws, &page, at, view);
                    self.expanded.insert(page);
                    self.navigate(id);
                }
            }
        }
    }

    // ---------------------------------------------------------------- sidebar

    // ---------------------------------------------------------------- page

    fn page_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let cur = match self.current.clone() {
            Some(c) if self.ws.page(&c).is_some() => c,
            _ => {
                self.current = None;
                ui.vertical_centered(|ui| {
                    ui.add_space(120.0);
                    ui.label(RichText::new(crate::tr!("Aucune page ouverte", "No page open")).size(20.0).weak());
                    if ui.button(crate::tr!("Créer une page", "Create a page")).clicked() {
                        actions.push(Action::NewPage(None));
                    }
                });
                return;
            }
        };
        let path = self.ws.path_of(&cur);
        let titles: HashMap<String, String> = self.ws.pages.iter().map(|p| (p.id.clone(), title_with_icon(p))).collect();
        let move_choices: Vec<(String, String)> = {
            let banned: HashSet<String> = self.ws.descendants(&cur).into_iter().chain(std::iter::once(cur.clone())).collect();
            self.ws
                .pages
                .iter()
                .filter(|p| !p.trashed && !banned.contains(&p.id) && p.database.is_none())
                .map(|p| (p.id.clone(), title_with_icon(p)))
                .collect()
        };
        self.page_top_bar(ui, &cur, &path, &move_choices, actions);

        let full = self.ws.page(&cur).map(|p| p.full_width).unwrap_or(false);
        // Project pages get a wide column so their cards can sit side by side.
        let project_page = self.ws.page(&cur).is_some_and(|p| p.project.is_some());
        let locked = self.ws.is_locked(&cur);
        let tree = if project_page { projects::page_tree(&self.ws, &cur) } else { vec![] };
        // A documentation folder shows what's inside it.
        let dir_tree = self.ws.page(&cur).and_then(|p| p.source.as_ref()).filter(|s| s.is_dir).map(|s| (s.rel.clone(), projects::page_tree(&self.ws, &cur)));
        let peers = if project_page { self.peers_in_project(&cur) } else { vec![] };
        // The AIs' questions about this project, and the answers given (inbox).
        let questions: Vec<crate::model::InboxMsg> = if project_page {
            let mine: Vec<crate::model::InboxMsg> = self.ws.inbox.iter().filter(|m| m.project == cur).cloned().collect();
            orch_ui::orch_questions(&mine, crate::model::now_ms(), u64::MAX).into_iter().map(|i| mine[i].clone()).collect()
        } else {
            vec![]
        };
        let mut answered: Option<(String, String)> = None;
        let focus = self.focus.take();
        let mut new_focus: Option<(String, Option<usize>)> = None;
        let mut focus_used = false;
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("page", &cur)).show(ui, |ui| {
            let avail = ui.available_width();
            // Notion's page column: 708 px of text (900 px page minus 96 px padding on each side),
            // plus the 44 px block gutter on the left. Full width keeps 96 px padding.
            let gutter = BLOCK_GUTTER;
            let width = if full {
                (avail - 2.0 * 96.0 + gutter).max(240.0)
            } else if project_page {
                (1500.0 + gutter).min(avail - 2.0 * 56.0 + gutter).max(240.0)
            } else {
                (708.0 + gutter).min(avail - 20.0).max(240.0)
            };
            let margin = ((avail - width - gutter) / 2.0).max(10.0);
            ui.horizontal_top(|ui| {
                ui.add_space(margin);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    let t = tone(ui.visuals().dark_mode);
                    let page = self.ws.page_mut(&cur).unwrap();
                    let header_key = egui::Id::unique(("page-header", &page.id));
                    let header_hovered = ui
                        .data(|d| d.get_temp::<egui::Rect>(header_key))
                        .map(|r| ui.rect_contains_pointer(r))
                        .unwrap_or(false);
                    let header_top = ui.cursor().top();
                    // A short gap from the page bar to the icon (the 78 px glyph already has room above it).
                    ui.add_space(if page.icon.is_empty() { 40.0 } else { 32.0 });
                    // Icon (78 px), or "Ajouter une icône" shown on hover.
                    ui.horizontal(|ui| {
                        ui.add_space(gutter);
                        let has_icon = !page.icon.is_empty();
                        let label = if has_icon {
                            RichText::new(page.icon.clone()).size(78.0)
                        } else {
                            RichText::new(crate::tr!("☺  Ajouter une icône", "☺  Add an icon")).size(14.0).color(if header_hovered { t.tertiary } else { Color32::TRANSPARENT })
                        };
                        let btn = ui.add(egui::Button::new(label).frame(false));
                        if locked {
                            return;
                        }
                        egui::Popup::menu(&btn).show(|ui| {
                            egui::Grid::new("icons").show(ui, |ui| {
                                for (i, ic) in ICONS.iter().enumerate() {
                                    if ui.add(egui::Button::new(RichText::new(*ic).size(22.0)).frame(false)).clicked() {
                                        page.icon = ic.to_string();
                                        ui.close();
                                    }
                                    if i % 5 == 4 {
                                        ui.end_row();
                                    }
                                }
                            });
                            if ui.button(crate::tr!("Retirer l'icône", "Remove icon")).clicked() {
                                page.icon.clear();
                                ui.close();
                            }
                        });
                    });
                    if !page.icon.is_empty() {
                        ui.add_space(26.0);
                    }
                    // Title: 40 px bold.
                    let title_key = format!("title-{}", page.id);
                    let title_id = egui::Id::unique(&title_key);
                    let tr = ui
                        .horizontal(|ui| {
                            ui.add_space(gutter);
                            ui.add(
                                egui::TextEdit::singleline(&mut page.title)
                                    .id(title_id)
                                    .interactive(!locked)
                                    .font(bold(40.0))
                                    .text_color(t.text)
                                    .hint_text(RichText::new(crate::tr!("Sans titre", "Untitled")).color(t.border))
                                    .frame(egui::Frame::NONE)
                                    .desired_width(f32::INFINITY),
                            )
                        })
                        .inner;
                    let header = egui::Rect::from_min_max(egui::pos2(ui.min_rect().left(), header_top), egui::pos2(ui.min_rect().right(), tr.rect.bottom()));
                    ui.data_mut(|d| d.insert_temp(header_key, header));
                    if focus.as_ref().map(|f| f.0 == title_key).unwrap_or(false) {
                        tr.request_focus();
                        focus_used = true;
                    }
                    if tr.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        if let Some(b) = page.blocks.first() {
                            new_focus = Some((b.id.clone(), Some(0)));
                        }
                    }
                    ui.add_space(20.0);
                    let page_id = page.id.clone();
                    let page_title = page.display_title();
                    if let Some(pr) = page.project.as_mut() {
                        egui::Frame::new()
                            .inner_margin(egui::Margin { left: gutter as i8, right: 0, top: 0, bottom: 0 })
                            .show(ui, |ui| {
                                let live = self.agents.by_project.get(&page_id).map(Vec::as_slice).unwrap_or(&[]);
                                let reviewing = self.reviews.contains_key(&page_id);
                                let peers = peers.clone();
                                let launchers = self.ais.launchers();
                                let mut cx = projects::Cx { page_id: &page_id, pr, new_step: &mut self.new_step, t: &t, live, pages: &tree, actions, msg: None, reviewing, peers, vers: &mut self.versions, launchers, title: page_title.clone(), ai: &mut self.ai, questions: &questions, answered: &mut answered };
                                let _p = Section::start("panneau projet");
                                if let Some(m) = projects::project_panel(ui, &mut cx) {
                                    self.said.push(m);
                                }
                            });
                        ui.add_space(16.0);
                    }
                    if let Some(src) = page.source.clone() {
                        egui::Frame::new()
                            .inner_margin(egui::Margin { left: gutter as i8, right: 0, top: 0, bottom: 0 })
                            .show(ui, |ui| projects::source_banner(ui, &page_id, &src, crate::docs::content_hash(page) != src.page_hash, actions));
                        ui.add_space(10.0);
                    }
                    if let Some((rel, rows)) = &dir_tree {
                        egui::Frame::new()
                            .inner_margin(egui::Margin { left: gutter as i8, right: 0, top: 0, bottom: 0 })
                            .show(ui, |ui| projects::folder_view(ui, rel, rows, &t, actions));
                        ui.add_space(10.0);
                    }
                    if let Some(db) = page.database.as_mut() {
                        // Indent to the text column with a margin frame (keeps a finite width
                        // for the horizontal scroll area inside).
                        egui::Frame::new()
                            .inner_margin(egui::Margin { left: gutter as i8, right: 0, top: 0, bottom: 0 })
                            .show(ui, |ui| database_view(ui, &page_id, db, &mut self.new_col, actions));
                        ui.add_space(16.0);
                    }
                    // Lecture seule: the text is shown as usual but can't be changed.
                    let (nf, used) = ui
                        .scope(|ui| {
                            if locked {
                                // Opacity first, then disable: `disable` fades by that amount,
                                // and the text must keep its normal look.
                                ui.visuals_mut().disabled_alpha = 1.0;
                                ui.disable();
                            }
                            let _p = Section::start("texte de la page");
                            blocks_view(ui, page, &titles, focus.as_ref(), actions)
                        })
                        .inner;
                    focus_used |= used;
                    if nf.is_some() {
                        new_focus = nf;
                    }
                    // Click below the last block to keep writing.
                    let h = ui.available_height().max(160.0);
                    let rest = ui.allocate_response(egui::vec2(width, h), egui::Sense::click());
                    if rest.clicked() {
                        let last_empty = page.blocks.last().map(|b| b.kind == BlockKind::Paragraph && b.text.is_empty()).unwrap_or(false);
                        if !last_empty {
                            page.blocks.push(Block::new(BlockKind::Paragraph, ""));
                        }
                        new_focus = page.blocks.last().map(|b| (b.id.clone(), None));
                    }
                });
            });
        });
        if let Some((id, text)) = answered {
            self.save_answer_to(&id, text);
        }
        if new_focus.is_some() {
            self.focus = new_focus;
        } else if !focus_used {
            // The target wasn't drawn yet (a new page appears next frame): keep asking.
            self.focus = focus;
        }
    }

    // ---------------------------------------------------------------- other views

    /// The trash, laid out like the other pages: a centred column, one row per page with its
    /// parent and last edit, and nothing deleted for good on a single click.
    fn trash_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let t = tone(ui.visuals().dark_mode);
        let mut roots: Vec<(String, String, String, u64)> = self
            .ws
            .pages
            .iter()
            .filter(|p| p.trashed && p.parent.as_deref().and_then(|pp| self.ws.page(pp)).map(|pp| !pp.trashed).unwrap_or(true))
            .map(|p| {
                let parent = p.parent.as_deref().and_then(|pp| self.ws.page(pp)).map(|pp| pp.display_title()).unwrap_or_default();
                (p.id.clone(), title_with_icon(p), parent, p.updated)
            })
            .collect();
        roots.sort_by(|a, b| b.3.cmp(&a.3));
        // The row (or "*" for the whole trash) waiting for its second click.
        let confirm_id = egui::Id::unique("trash-confirm");
        let mut confirm: Option<String> = ui.ctx().data(|d| d.get_temp(confirm_id));
        let avail = ui.available_width();
        let width = 708.0f32.min(avail - 40.0).max(300.0);
        ui.add_space(50.0);
        ui.horizontal(|ui| {
            ui.add_space(((avail - width) / 2.0).max(20.0));
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(crate::tr!("Corbeille", "Trash")).font(bold(30.0)).color(t.text));
                    if !roots.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let all = confirm.as_deref() == Some("*");
                            let text = if all { crate::trf!("Confirmer : supprimer {} page(s) pour de bon", "Confirm: delete {} page(s) for good", roots.len()) } else { crate::tr!("Vider la corbeille", "Empty the trash").to_string() };
                            let color = if all { projects::ORANGE } else { t.text };
                            if ui.button(RichText::new(text).size(14.0).color(color)).clicked() {
                                if all {
                                    actions.extend(roots.iter().map(|r| Action::DeleteForever(r.0.clone())));
                                    confirm = None;
                                } else {
                                    confirm = Some("*".into());
                                }
                            }
                        });
                    }
                });
                ui.label(RichText::new(crate::tr!("Les pages restent ici jusqu'à ce que vous les supprimiez définitivement.", "Pages stay here until you delete them for good.")).color(t.secondary));
                ui.add_space(20.0);
                if roots.is_empty() {
                    ui.label(RichText::new(crate::tr!("La corbeille est vide.", "The trash is empty.")).color(t.tertiary).italics());
                    return;
                }
                for (id, title, parent, updated) in &roots {
                    let bg = projects::hover_begin(ui);
                    let row = ui.horizontal(|ui| {
                        ui.set_min_height(30.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(RichText::new(title).size(14.5).color(t.text));
                            let when = chrome::relative_time(*updated);
                            let info = if parent.is_empty() { when } else { crate::trf!("dans {parent} · {when}", "in {parent} · {when}") };
                            ui.label(RichText::new(info).size(12.0).color(t.tertiary));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if confirm.as_deref() == Some(id.as_str()) {
                                if projects::quiet_link(ui, crate::tr!("Annuler", "Cancel"), &t).clicked() {
                                    confirm = None;
                                }
                                if ui.button(RichText::new(crate::tr!("Confirmer la suppression", "Confirm the deletion")).size(13.0).color(projects::ORANGE)).clicked() {
                                    actions.push(Action::DeleteForever(id.clone()));
                                    confirm = None;
                                }
                            } else {
                                if projects::quiet_link(ui, crate::tr!("Supprimer définitivement", "Delete for good"), &t).clicked() {
                                    confirm = Some(id.clone());
                                }
                                if projects::action_link(ui, crate::tr!("Restaurer", "Restore")).clicked() {
                                    actions.push(Action::Restore(id.clone()));
                                }
                            }
                        });
                    });
                    projects::hover_end(ui, bg, row.response.rect, egui::Id::unique(("trash-row", id)), &t);
                    ui.add_space(6.0);
                }
            });
        });
        ui.ctx().data_mut(|d| {
            if let Some(c) = confirm {
                d.insert_temp(confirm_id, c);
            } else {
                d.remove::<String>(confirm_id);
            }
        });
    }

}

/// A block's content without its time.
fn block_key(b: &Block) -> String {
    let mut c = b.clone();
    c.updated = 0;
    serde_json::to_string(&c).unwrap_or_default()
}

/// A page's content without its `updated` time, to notice edits.
fn page_key(p: &Page) -> String {
    let mut c = p.clone();
    c.updated = 0;
    serde_json::to_string(&c).unwrap_or_default()
}

impl eframe::App for NotesApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let _prof = Profile::frame(&ctx, _frame.info().cpu_usage);
        let pref = self.theme_preference();
        if self.applied_theme != Some(pref) {
            ctx.set_theme(pref);
            self.applied_theme = Some(pref);
        }
        self.poll_file();
        if let Some(w) = &self.watcher {
            self.agents.refresh(&self.ws, w.sessions());
        }
        if let Some(shot) = self.shot.as_mut() {
            shot.tick(&ctx);
        }
        let mut actions = vec![];
        self.take_term_request(&ctx);
        self.review_tick(&ctx);
        self.ai_tick(&ctx);
        self.ais_tick();
        self.orch_tick(&ctx);
        self.install_tick(&ctx);
        self.update_tick(&ctx);
        self.versions.tick(&ctx);
        if let Some(m) = self.versions.status.take() {
            self.notice(m);
        }
        // Every 10 s, bring the projects' documentation pages in line with their files.
        if self.docs_checked.is_none_or(|t| t.elapsed().as_secs() >= 10) {
            self.docs_checked = Some(Instant::now());
            let ids: Vec<String> = self.ws.projects().iter().map(|p| p.id.clone()).collect();
            let mut changed = false;
            for id in ids {
                changed |= crate::docs::sync_project(&mut self.ws, &id) != crate::docs::Stats::default();
            }
            if changed {
                self.detect_changes();
            }
            ctx.request_repaint_after(std::time::Duration::from_secs(10));
        }
        if !self.mcp_report_shown {
            let done = self.mcp_report.lock().unwrap().as_ref().map(|r| (crate::setup::summary(r), r.iter().any(|(_, o)| matches!(o, crate::setup::Outcome::Failed(_)))));
            if let Some((msg, failed)) = done {
                self.mcp_report_shown = true;
                match msg {
                    Some(m) if failed => self.fail(m),
                    Some(m) => self.info(m),
                    None => {}
                }
            }
        }
        self.shortcuts(&ctx, &mut actions);
        // Ctrl+` shows / hides the terminals, as in VS Code.
        if !self.terms.is_empty() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Backtick)) {
            self.term_open = !self.term_open;
            self.term_focus = self.term_open;
        }
        let dark = ctx.global_style().visuals.dark_mode;
        let side_fill = chrome::side_fill(dark);
        let page_fill = rgb(if dark { original::dark::BACKGROUND } else { original::light::BACKGROUND });
        // Sidebar first: it runs the full height, beside the window buttons.
        if !self.ws.settings.sidebar_hidden {
            let w = if self.ws.settings.sidebar_width >= 200.0 { self.ws.settings.sidebar_width } else { SIDEBAR_WIDTH };
            let _p = Section::start("barre latérale");
            let resp = egui::Panel::left("sidebar")
                .default_size(w)
                .size_range(220.0..=480.0)
                .frame(egui::Frame::new().fill(side_fill).inner_margin(egui::Margin { left: 10, right: 10, top: 0, bottom: 8 }))
                .show(ui, |ui| self.sidebar(ui, &mut actions));
            // A thin violet-grey line sets the sidebar apart from the page.
            let r = resp.response.rect;
            ui.painter().vline(r.right() - 0.5, r.y_range(), egui::Stroke::new(1.0, chrome::side_tone(dark).border));
            let now = resp.response.rect.width();
            if (now - self.ws.settings.sidebar_width).abs() > 1.0 && ctx.input(|i| !i.pointer.any_down()) {
                self.ws.settings.sidebar_width = now;
            }
        }
        // Tab strip over the page only.
        egui::Panel::top("tabs")
            .exact_size(original::TAB_BAR_HEIGHT)
            .frame(egui::Frame::new().fill(page_fill))
            .show(ui, |ui| self.tab_strip(ui));
        self.side_w = 0.0;
        self.dock_h = 0.0;
        self.terminal_dock(ui, page_fill);
        self.side_panel(ui, page_fill);
        let pad = egui::Frame::new().inner_margin(egui::Margin::symmetric(32, 16));
        let _p = Section::start("page entière");
        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(page_fill)).show(ui, |ui| match self.view {
            View::Page => self.page_view(ui, &mut actions),
            View::Home => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.home_view(ui, &mut actions));
            }
            View::Trash => {
                pad.show(ui, |ui| self.trash_view(ui, &mut actions));
            }
            View::Projects => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.projects_view(ui, &mut actions));
            }
            View::Orchestra => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("orchestra").show(ui, |ui| self.orchestra_view(ui, &mut actions));
            }
            View::Inbox => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("inbox").show(ui, |ui| self.inbox_view(ui, &mut actions));
            }
        });
        // The round AI button only while no AI is plugged in (it leads to plugging one).
        let fab = self.settings_tab.is_none() && self.ais.none_plugged();
        if fab {
            self.ai_button(&ctx);
        }
        for m in std::mem::take(&mut self.said) {
            self.notice(m);
        }
        self.toasts_ui(&ctx, fab);
        self.search_window(&ctx, &mut actions);
        self.settings_window(&ctx, &mut actions);
        let had_actions = !actions.is_empty();
        self.apply(actions);
        // Keep the active tab and the list of open tabs in the settings (restored at start).
        if let Some(t) = self.tabs.get_mut(self.active_tab) {
            t.page = self.current.clone();
        }
        self.remember_terminals();
        let open: Vec<String> = self.tabs.iter().filter_map(|t| t.page.clone()).collect();
        let active = self.tabs.iter().take(self.active_tab).filter(|t| t.page.is_some()).count();
        if open != self.ws.settings.open_tabs || active != self.ws.settings.active_tab {
            self.ws.settings.open_tabs = open;
            self.ws.settings.active_tab = active;
        }

        // Only events that can edit something: scrolling or moving the mouse never does, and
        // looking for edits means comparing every page, too slow to do on each scroll frame.
        let had_input = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(e, egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Cut | egui::Event::PointerButton { pressed: false, .. })
            })
        });
        if had_input || had_actions {
            self.detect_changes();
        }
        self.live_tick();
        if let Some(t) = self.dirty_since {
            if t.elapsed() > Duration::from_millis(400) {
                self.save_now();
            }
        }
        let c = &self.ws.settings.collab;
        if c.auto_sync && c.connector != ConnectorKind::None && self.last_sync.elapsed() > Duration::from_secs(30) {
            self.run_sync(false);
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn on_exit(&mut self) {
        self.save_on_exit();
    }
}

impl NotesApp {
    fn save_on_exit(&mut self) {
        self.detect_changes();
        if self.dirty_since.is_some() {
            self.save_now();
        }
    }
}

// -------------------------------------------------------------------- blocks

/// Font family name of the semi-bold weight of the system font (see `setup_fonts`).
pub const SEMIBOLD: &str = "semibold";

/// Width of the "+" and drag handle left of each block (22 + 2 + 18 + 2).
pub const BLOCK_GUTTER: f32 = 44.0;

/// Font family name of the bold (700) weight, used for page titles.
pub const BOLD: &str = "bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name(BOLD.into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name(SEMIBOLD.into()))
}

/// Block sizes: body 16 px, headings 1.875 / 1.5 / 1.25 em (estimated: the editor isn't in the package).
/// Line height of a block: 1.5 for text (24 px at 16 px), 1.3 for headings (estimated).
fn block_line_height(kind: BlockKind) -> f32 {
    match kind {
        BlockKind::Heading1 => 39.0,
        BlockKind::Heading2 => 31.0,
        BlockKind::Heading3 => 26.0,
        BlockKind::Code => 21.0,
        _ => 24.0,
    }
}

/// Inline styles inside a block, written in Markdown: `**gras**`, `*italique*`, `` `code` ``,
/// `~~barré~~`. Returns (start, end, marker length, style) as byte ranges, markers included.
#[derive(Clone, Copy, PartialEq)]
enum Inline {
    Bold,
    Italic,
    Code,
    Strike,
}

fn inline_spans(text: &str) -> Vec<(usize, usize, usize, Inline)> {
    let mut out = vec![];
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let rest = &text[i..];
        let (marker, style) = if rest.starts_with("**") {
            ("**", Inline::Bold)
        } else if rest.starts_with("~~") {
            ("~~", Inline::Strike)
        } else if rest.starts_with('`') {
            ("`", Inline::Code)
        } else if rest.starts_with('*') || rest.starts_with('_') {
            (&rest[..1], Inline::Italic)
        } else {
            i += rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            continue;
        };
        let m = marker.len();
        // A closing marker on the same line, with something in between.
        let after = &text[i + m..];
        let line_end = after.find('\n').unwrap_or(after.len());
        match after[..line_end].find(marker) {
            Some(close) if close > 0 && !(style == Inline::Italic && after[close..].starts_with("**")) => {
                let end = i + m + close + m;
                out.push((i, end, m, style));
                i = end;
            }
            _ => i += m,
        }
    }
    out
}

/// Lay out a block's text. The text is never changed, so the cursor maps one to one: styled
/// parts are drawn styled, and their markers fade (editing) or shrink to nothing (reading).
fn block_job(text: &str, kind: BlockKind, done: bool, editing: bool, t: &Tone) -> egui::text::LayoutJob {
    let font = block_font(kind);
    let lh = block_line_height(kind);
    let base_color = if done { t.tertiary } else { t.text };
    let fmt = |font: FontId, color: Color32| egui::TextFormat { font_id: font, color, line_height: Some(lh), valign: egui::Align::Center, ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    let plain = |job: &mut egui::text::LayoutJob, s: &str| {
        if s.is_empty() {
            return;
        }
        let mut f = fmt(font.clone(), base_color);
        if done {
            f.strikethrough = egui::Stroke::new(1.0, base_color);
        }
        job.append(s, 0.0, f);
    };
    if kind == BlockKind::Code {
        plain(&mut job, text);
        return job;
    }
    let marker_fmt = || if editing { fmt(font.clone(), t.tertiary) } else { fmt(FontId::proportional(1.0), Color32::TRANSPARENT) };
    let mut pos = 0;
    for (start, end, m, style) in inline_spans(text) {
        plain(&mut job, &text[pos..start]);
        job.append(&text[start..start + m], 0.0, marker_fmt());
        let inner = &text[start + m..end - m];
        let mut f = fmt(font.clone(), base_color);
        match style {
            Inline::Bold => f.font_id = semibold(font.size),
            Inline::Italic => f.italics = true,
            Inline::Strike => f.strikethrough = egui::Stroke::new(1.0, base_color),
            Inline::Code => {
                // Inline code: red text on a light grey wash, 85 % size (estimated).
                f.font_id = FontId::monospace(font.size * 0.85);
                f.color = Color32::from_rgb(235, 87, 87);
                f.background = Color32::from_rgba_unmultiplied(135, 131, 120, 38);
            }
        }
        if done {
            f.strikethrough = egui::Stroke::new(1.0, base_color);
        }
        job.append(inner, 0.0, f);
        job.append(&text[end - m..end], 0.0, marker_fmt());
        pos = end;
    }
    plain(&mut job, &text[pos..]);
    job
}

fn block_font(kind: BlockKind) -> FontId {
    match kind {
        BlockKind::Heading1 => semibold(30.0),
        BlockKind::Heading2 => semibold(24.0),
        BlockKind::Heading3 => semibold(20.0),
        BlockKind::Code => FontId::monospace(14.0),
        _ => FontId::proportional(16.0),
    }
}

/// Markdown typed at the start of a plain text block turns it into another kind.
fn markdown_shortcut(text: &str) -> Option<(BlockKind, usize)> {
    let rules: [(&str, BlockKind); 13] = [
        ("### ", BlockKind::Heading3),
        ("## ", BlockKind::Heading2),
        ("# ", BlockKind::Heading1),
        ("[] ", BlockKind::Todo),
        ("[ ] ", BlockKind::Todo),
        ("- ", BlockKind::Bullet),
        ("* ", BlockKind::Bullet),
        ("1. ", BlockKind::Numbered),
        ("> ", BlockKind::Toggle),
        ("\" ", BlockKind::Quote),
        ("```", BlockKind::Code),
        ("---", BlockKind::Divider),
        ("! ", BlockKind::Callout),
    ];
    rules.iter().find(|(p, _)| text.starts_with(p)).map(|(p, k)| (*k, p.len()))
}

fn set_cursor(ctx: &egui::Context, id: egui::Id, at: usize) {
    let mut st = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
    st.store(ctx, id);
}

/// Colours of the current theme, from the original's design tokens.
pub struct Tone {
    pub text: Color32,
    pub secondary: Color32,
    pub tertiary: Color32,
    pub accent_text: Color32,
    pub fill: Color32,
    pub border: Color32,
    pub hover: Color32,
    /// The open item in the sidebar (interactiveTranslucent; hover is a little lighter).
    pub selected: Color32,
    /// Cards and popups: background.elevated.
    pub card: Color32,
}

pub fn tone(dark: bool) -> Tone {
    if dark {
        use original::dark as d;
        Tone {
            text: rgb(d::TEXT),
            secondary: rgb(d::TEXT_SECONDARY),
            tertiary: rgb(d::TEXT_TERTIARY),
            accent_text: Color32::from_rgb(188, 186, 182), // text.dark.accentPrimary
            fill: rgb(d::TERTIARY),
            border: rgb(d::BORDER),
            hover: Color32::from_rgba_unmultiplied(255, 255, 255, 10),
            selected: Color32::from_rgba_unmultiplied(255, 255, 255, 14), // background.dark.interactiveTranslucent 0.056
            card: rgb(d::SIDEBAR),                                        // background.dark.elevated (32,32,32)
        }
    } else {
        use original::light as l;
        Tone {
            text: rgb(l::TEXT),
            secondary: rgb(l::TEXT_SECONDARY),
            tertiary: rgb(l::TEXT_TERTIARY),
            accent_text: Color32::from_rgb(95, 94, 89), // text.light.accentPrimary
            fill: rgb(l::TERTIARY),
            border: rgb(l::BORDER),
            hover: Color32::from_rgba_unmultiplied(33, 27, 23, 10),
            selected: Color32::from_rgba_unmultiplied(33, 27, 23, 13), // background.light.interactiveTranslucent 0.05
            card: rgb(l::BACKGROUND),                                  // background.light.elevated (255,255,255)
        }
    }
}

/// The two controls left of a block, shown on hover: "+" (add a block below) and the
/// six-dot handle (drag to move, click for the block menu).
fn block_gutter(ui: &mut egui::Ui, visible: bool, t: &Tone, line: f32) -> (egui::Response, egui::Response) {
    let size = egui::vec2(22.0, line);
    let (r_plus, plus) = ui.allocate_exact_size(size, egui::Sense::click());
    let (r_handle, handle) = ui.allocate_exact_size(egui::vec2(18.0, line), egui::Sense::click_and_drag());
    if visible {
        let p = ui.painter();
        for (r, resp) in [(r_plus, &plus), (r_handle, &handle)] {
            if resp.hovered() || resp.dragged() {
                p.rect_filled(r.shrink2(egui::vec2(1.0, 2.0)), 4.0, t.hover);
            }
        }
        let c = r_plus.center();
        let s = egui::Stroke::new(1.6, t.tertiary);
        p.line_segment([c - egui::vec2(6.0, 0.0), c + egui::vec2(6.0, 0.0)], s);
        p.line_segment([c - egui::vec2(0.0, 6.0), c + egui::vec2(0.0, 6.0)], s);
        let c = r_handle.center();
        for dx in [-3.0, 3.0] {
            for dy in [-5.0, 0.0, 5.0] {
                p.circle_filled(c + egui::vec2(dx, dy), 1.5, t.tertiary);
            }
        }
    }
    if handle.hovered() && !handle.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let plus = plus.on_hover_text(crate::tr!("Cliquer pour ajouter un bloc en dessous", "Click to add a block below"));
    let handle = handle.on_hover_text(crate::tr!("Glisser pour déplacer\nCliquer pour ouvrir le menu", "Drag to move\nClick to open the menu"));
    (plus, handle)
}

/// Notion-style check box: rounded square, blue with a white tick when done.
fn todo_box(ui: &mut egui::Ui, checked: &mut bool, t: &Tone) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
    let r = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
    let p = ui.painter();
    if *checked {
        p.rect_filled(r, 3.0, rgb(ACCENT));
        let s = egui::Stroke::new(2.0, Color32::WHITE);
        let a = r.left_center() + egui::vec2(3.5, 0.5);
        let b = r.center_bottom() + egui::vec2(-1.5, -4.0);
        let c = r.right_top() + egui::vec2(-3.5, 4.0);
        p.line_segment([a, b], s);
        p.line_segment([b, c], s);
    } else {
        if resp.hovered() {
            p.rect_filled(r, 3.0, t.hover);
        }
        p.rect_stroke(r, 3.0, egui::Stroke::new(1.5, t.text), egui::StrokeKind::Inside);
    }
    if resp.clicked() {
        *checked = !*checked;
    }
}

/// Draw the blocks of a page. Returns a block to focus next frame and whether the asked focus was used.
fn blocks_view(
    ui: &mut egui::Ui,
    page: &mut Page,
    titles: &HashMap<String, String>,
    focus: Option<&(String, Option<usize>)>,
    actions: &mut Vec<Action>,
) -> (Option<(String, Option<usize>)>, bool) {
    let mut new_focus = None;
    let mut focus_used = false;
    let mut remove: Option<usize> = None;
    let mut insert: Option<(usize, Block)> = None;
    let mut number = 0;
    let n = page.blocks.len();
    let page_id = page.id.clone();
    let dark = ui.visuals().dark_mode;
    let t = tone(dark);
    let drag_key = egui::Id::unique(("block-drag", &page_id));
    let mut dragging: Option<String> = ui.data(|d| d.get_temp::<String>(drag_key));
    let mut drag_stopped = false;
    let mut rows: Vec<egui::Rect> = Vec::with_capacity(n);
    let pointer = ui.input(|i| i.pointer.hover_pos());
    ui.spacing_mut().item_spacing.y = 6.0;
    for i in 0..n {
        let prev_id = if i > 0 { Some(page.blocks[i - 1].id.clone()) } else { None };
        let prev_indent_limit = if i > 0 { page.blocks[i - 1].indent.saturating_add(1).min(8) } else { 0 };
        let next_id = page.blocks.get(i + 1).map(|b| b.id.clone());
        let b = &mut page.blocks[i];
        number = if b.kind == BlockKind::Numbered { number + 1 } else { 0 };
        let edit_id = egui::Id::unique(("block", &b.id));
        let row_key = edit_id.with("row");
        let last_rect: Option<egui::Rect> = ui.data(|d| d.get_temp(row_key));
        let hovered = matches!((last_rect, pointer), (Some(r), Some(p)) if r.expand2(egui::vec2(0.0, 1.0)).contains(p));
        let is_dragged = dragging.as_deref() == Some(b.id.as_str());
        let was_empty = b.text.is_empty();
        let mut slash_open = false;
        let mut slash_sel = 0usize;
        let mut slash_scroll = false;
        let mut slash_anchor = egui::Rect::NOTHING;
        let mut quote_x: Option<f32> = None;
        // Space above headings (Notion: about 2 em, 1.4 em and 1 em; estimated).
        let above = match b.kind {
            BlockKind::Heading1 if i > 0 => 22.0,
            BlockKind::Heading2 if i > 0 => 16.0,
            BlockKind::Heading3 if i > 0 => 10.0,
            _ => 0.0,
        };
        if above > 0.0 {
            ui.add_space(above);
        }
        let line = block_line_height(b.kind);
        let row = ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(b.indent as f32 * 26.0);
            let (plus, handle) = block_gutter(ui, (hovered && dragging.is_none()) || is_dragged, &t, line);
            if plus.clicked() {
                let nb = Block::new(BlockKind::Paragraph, "/");
                new_focus = Some((nb.id.clone(), Some(1)));
                insert = Some((i + 1, nb));
            }
            if handle.drag_started() {
                dragging = Some(b.id.clone());
                ui.data_mut(|d| d.insert_temp(drag_key, b.id.clone()));
            }
            if handle.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if handle.drag_stopped() {
                drag_stopped = true;
            }
            egui::Popup::menu(&handle).show(|ui| {
                ui.set_min_width(200.0);
                ui.label(RichText::new(crate::tr!("Transformer en", "Turn into")).small().color(t.secondary));
                for k in BlockKind::ALL {
                    if k == BlockKind::PageLink {
                        continue;
                    }
                    if ui.selectable_label(b.kind == k, k.label()).clicked() {
                        b.kind = k;
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button(crate::tr!("Dupliquer", "Duplicate")).clicked() {
                    let mut c = b.clone();
                    c.id = new_id();
                    insert = Some((i + 1, c));
                    ui.close();
                }
                if ui.button(crate::tr!("Supprimer", "Delete")).clicked() {
                    remove = Some(i);
                    ui.close();
                }
            });
            match b.kind {
                BlockKind::Divider => {
                    let w = ui.available_width();
                    let (r, _) = ui.allocate_exact_size(egui::vec2(w, 24.0), egui::Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.border));
                    return;
                }
                BlockKind::PageLink => {
                    let label = titles.get(&b.extra).cloned().unwrap_or_else(|| crate::tr!("📄 Page supprimée", "📄 Deleted page").into());
                    let resp = ui.add(egui::Label::new(RichText::new(label).size(16.0).underline().color(t.text)).sense(egui::Sense::click()));
                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if resp.hovered() {
                        ui.painter().rect_filled(resp.rect.expand(2.0), 3.0, t.hover);
                    }
                    if resp.clicked() && !b.extra.is_empty() {
                        actions.push(Action::Open(b.extra.clone()));
                    }
                    return;
                }
                BlockKind::Todo => todo_box(ui, &mut b.checked, &t),
                BlockKind::Bullet => {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(24.0, line), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 2.8, t.text);
                }
                BlockKind::Numbered => {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(24.0, line), egui::Sense::hover());
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, format!("{number}."), FontId::proportional(16.0), t.text);
                }
                BlockKind::Toggle => {
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(24.0, line), egui::Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r.shrink(2.0), 4.0, t.hover);
                    }
                    let c = r.center();
                    let pts = if b.checked {
                        vec![c + egui::vec2(-5.0, -3.0), c + egui::vec2(5.0, -3.0), c + egui::vec2(0.0, 4.0)]
                    } else {
                        vec![c + egui::vec2(-3.0, -5.0), c + egui::vec2(4.0, 0.0), c + egui::vec2(-3.0, 5.0)]
                    };
                    ui.painter().add(egui::Shape::convex_polygon(pts, t.text, egui::Stroke::NONE));
                    if resp.clicked() {
                        b.checked = !b.checked;
                    }
                }
                BlockKind::Quote => {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, line), egui::Sense::hover());
                    quote_x = Some(r.left() + 1.5);
                }
                _ => {}
            }
            let color = if b.kind == BlockKind::Todo && b.checked { t.tertiary } else { t.text };
            let hint = match b.kind {
                BlockKind::Paragraph => crate::tr!("Écrivez, ou tapez « / » pour les commandes…", "Write, or type « / » for commands…"),
                BlockKind::Heading1 => crate::tr!("Titre 1", "Heading 1"),
                BlockKind::Heading2 => crate::tr!("Titre 2", "Heading 2"),
                BlockKind::Heading3 => crate::tr!("Titre 3", "Heading 3"),
                BlockKind::Todo => crate::tr!("À faire", "To-do"),
                BlockKind::Toggle => crate::tr!("Liste dépliante", "Toggle list"),
                BlockKind::Quote => crate::tr!("Citation", "Quote"),
                BlockKind::Code => "Code",
                BlockKind::Callout => crate::tr!("Encadré", "Callout"),
                _ => crate::tr!("Liste", "List"),
            };
            let is_code = b.kind == BlockKind::Code;
            let has_focus_before = ui.memory(|m| m.has_focus(edit_id));
            if has_focus_before && !is_code {
                // Tab nests the block under the one above (at most one level deeper), Shift+Tab un-nests.
                let (tab, untab) = ui.input_mut(|inp| (inp.consume_key(egui::Modifiers::NONE, Key::Tab), inp.consume_key(egui::Modifiers::SHIFT, Key::Tab)));
                if tab && b.indent < prev_indent_limit {
                    b.indent += 1;
                }
                if untab && b.indent > 0 {
                    b.indent -= 1;
                }
            }
            // "/" menu: open while the block starts with "/" (and Escape wasn't pressed on this text).
            let slash_key = edit_id.with("slash");
            let dismissed: Option<String> = ui.data(|d| d.get_temp(slash_key.with("off")));
            let slash_active = has_focus_before && !is_code && b.text.starts_with('/') && !b.text.contains('\n') && dismissed.as_deref() != Some(b.text.as_str());
            if slash_active {
                let n = slash::matches(&b.text).len().max(1);
                let last_q: Option<String> = ui.data(|d| d.get_temp(slash_key.with("q")));
                let mut sel: usize = if last_q.as_deref() == Some(b.text.as_str()) { ui.data(|d| d.get_temp(slash_key).unwrap_or(0)) } else { 0 };
                let (dn, upk, esc) = ui.input_mut(|inp| {
                    (inp.consume_key(egui::Modifiers::NONE, Key::ArrowDown), inp.consume_key(egui::Modifiers::NONE, Key::ArrowUp), inp.consume_key(egui::Modifiers::NONE, Key::Escape))
                });
                if dn {
                    sel = (sel + 1) % n;
                    slash_scroll = true;
                }
                if upk {
                    sel = (sel + n - 1) % n;
                    slash_scroll = true;
                }
                if esc {
                    ui.data_mut(|d| d.insert_temp(slash_key.with("off"), b.text.clone()));
                }
                ui.data_mut(|d| {
                    d.insert_temp(slash_key, sel.min(n - 1));
                    d.insert_temp(slash_key.with("q"), b.text.clone());
                });
                slash_sel = sel.min(n - 1);
                slash_open = !esc;
            }
            let done = b.kind == BlockKind::Todo && b.checked;
            let kind = b.kind;
            let tone_copy = tone(dark);
            let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                let mut job = block_job(buf.as_str(), kind, done, has_focus_before, &tone_copy);
                job.wrap.max_width = wrap;
                ui.fonts_mut(|f| f.layout_job(job))
            };
            let mut te = egui::TextEdit::multiline(&mut b.text)
                .id(edit_id)
                .font(block_font(b.kind))
                .text_color(color)
                .desired_rows(1)
                .desired_width(f32::INFINITY)
                .lock_focus(true)
                .layouter(&mut layouter)
                .frame(egui::Frame::NONE);
            if !is_code {
                te = te.return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, Key::Enter));
            }
            if has_focus_before {
                te = te.hint_text(RichText::new(hint).color(t.tertiary));
            }
            // Callout and code: Notion's soft grey (dark #252525, light #F1F1EF; estimated).
            let soft = if dark { Color32::from_rgb(37, 37, 37) } else { Color32::from_rgb(241, 241, 239) };
            let query_bg = ui.painter().add(egui::Shape::Noop);
            let out = match b.kind {
                BlockKind::Callout => egui::Frame::new()
                    .fill(soft)
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin { left: 12, right: 16, top: 14, bottom: 14 })
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.label(RichText::new("💡").size(18.0));
                            te.show(ui)
                        })
                        .inner
                    })
                    .inner,
                BlockKind::Code => egui::Frame::new()
                    .fill(soft)
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin { left: 20, right: 16, top: 20, bottom: 20 })
                    .show(ui, |ui| te.show(ui))
                    .inner,
                _ => te.show(ui),
            };
            if slash_open {
                let r = egui::Rect::from_min_size(out.galley_pos, out.galley.size()).expand2(egui::vec2(5.0, 2.0));
                let fill = if dark { Color32::from_rgb(32, 32, 32) } else { Color32::from_rgb(241, 241, 239) };
                ui.painter().set(query_bg, egui::Shape::rect_filled(r, 6.0, fill));
            }
            slash_anchor = egui::Rect::from_min_size(out.galley_pos, out.galley.size());
            let resp = out.response.response.clone();
            // Screen readers announce the block's type ("Titre 1", "Liste de tâches"...).
            let kind_label = b.kind.label();
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, true, kind_label));
            if let Some((fid, at)) = focus {
                if *fid == b.id {
                    resp.request_focus();
                    let pos = at.unwrap_or_else(|| b.text.chars().count());
                    set_cursor(ui.ctx(), edit_id, pos);
                    focus_used = true;
                }
            }
            let cursor = out.cursor_range.map(|r| r.primary.index.0);
            if resp.has_focus() {
                // Markdown shortcuts.
                if b.kind == BlockKind::Paragraph && resp.changed() {
                    if let Some((k, len)) = markdown_shortcut(&b.text) {
                        b.kind = k;
                        b.text = b.text[len..].to_string();
                        if k == BlockKind::Divider {
                            b.text.clear();
                            let nb = Block::new(BlockKind::Paragraph, "");
                            new_focus = Some((nb.id.clone(), Some(0)));
                            insert = Some((i + 1, nb));
                        }
                    }
                }
                let (enter, backspace, up, down, esc) = ui.input(|inp| {
                    (
                        inp.key_pressed(Key::Enter) && !inp.modifiers.shift && !inp.modifiers.command,
                        inp.key_pressed(Key::Backspace),
                        inp.key_pressed(Key::ArrowUp),
                        inp.key_pressed(Key::ArrowDown),
                        inp.key_pressed(Key::Escape),
                    )
                });
                let _ = esc;
                let picks = if slash_open { slash::matches(&b.text) } else { vec![] };
                if slash_open && enter && !picks.is_empty() {
                    let k = picks[slash_sel.min(picks.len() - 1)].0;
                    apply_item(b, k, i, &page_id, actions, &mut insert, &mut new_focus);
                } else if enter && !is_code {
                    let list = matches!(b.kind, BlockKind::Todo | BlockKind::Bullet | BlockKind::Numbered);
                    if list && b.text.is_empty() {
                        b.kind = BlockKind::Paragraph;
                    } else {
                        let len = b.text.chars().count();
                        let at = cursor.unwrap_or(len).min(len);
                        let byte = b.text.char_indices().nth(at).map(|(x, _)| x).unwrap_or(b.text.len());
                        let tail = b.text[byte..].to_string();
                        b.text.truncate(byte);
                        let kind = if list { b.kind } else { BlockKind::Paragraph };
                        let nb = Block::new(kind, tail);
                        new_focus = Some((nb.id.clone(), Some(0)));
                        insert = Some((i + 1, nb));
                    }
                } else if backspace && was_empty {
                    if b.kind != BlockKind::Paragraph {
                        b.kind = BlockKind::Paragraph;
                    } else if n > 1 {
                        remove = Some(i);
                        if let Some(p) = &prev_id {
                            new_focus = Some((p.clone(), None));
                        }
                    }
                } else if up && !b.text.contains('\n') && cursor.unwrap_or(0) == 0 {
                    if let Some(p) = &prev_id {
                        new_focus = Some((p.clone(), None));
                    }
                } else if down && !b.text.contains('\n') && cursor.map(|c| c >= b.text.chars().count()).unwrap_or(true) {
                    if let Some(nx) = &next_id {
                        new_focus = Some((nx.clone(), Some(0)));
                    }
                }
            }
        });
        let rect = row.response.rect;
        if let Some(x) = quote_x {
            ui.painter().vline(x, rect.y_range().shrink(2.0), egui::Stroke::new(3.0, t.text));
        }
        if is_dragged {
            // The block being moved is dimmed in place.
            ui.painter().rect_filled(rect, 3.0, ui.visuals().panel_fill.gamma_multiply(0.6));
        }
        ui.data_mut(|d| d.insert_temp(row_key, rect));
        rows.push(rect);
        let b = &mut page.blocks[i];
        // Toggle content.
        if b.kind == BlockKind::Toggle && b.checked {
            ui.horizontal(|ui| {
                ui.add_space(70.0);
                ui.add(
                    egui::TextEdit::multiline(&mut b.extra)
                        .hint_text(RichText::new(crate::tr!("Liste dépliante vide. Cliquez pour écrire.", "Empty toggle list. Click to write.")).color(t.tertiary))
                        .frame(egui::Frame::NONE)
                        .font(FontId::proportional(16.0))
                        .desired_rows(1)
                        .desired_width(f32::INFINITY),
                );
            });
        }
        // "/" menu, floating over the page.
        if slash_open {
            let edit_id = egui::Id::unique(("block", &b.id));
            match slash::popup(ui.ctx(), edit_id.with("slash-popup"), slash_anchor, &b.text, slash_sel, slash_scroll) {
                Some(slash::SlashEvent::Pick(k)) => {
                    apply_item(b, k, i, &page_id, actions, &mut insert, &mut new_focus);
                    if new_focus.is_none() {
                        new_focus = Some((b.id.clone(), None));
                    }
                }
                Some(slash::SlashEvent::Hover(j)) => {
                    ui.data_mut(|d| d.insert_temp(edit_id.with("slash"), j));
                }
                Some(slash::SlashEvent::Close) => {
                    ui.data_mut(|d| d.insert_temp(edit_id.with("slash").with("off"), b.text.clone()));
                    new_focus = Some((b.id.clone(), None));
                }
                None => {}
            }
        }
    }

    // Drag and drop: where the block would land, a blue line, and the move on release.
    let mut moved: Option<(usize, usize)> = None;
    if let Some(drag_id) = dragging.clone() {
        let from = page.blocks.iter().position(|b| b.id == drag_id);
        let cancel = ui.input(|i| i.key_pressed(Key::Escape));
        match (from, pointer) {
            (Some(from), Some(p)) if !cancel && !rows.is_empty() => {
                // The block moves with the blocks nested under it.
                let base = page.blocks[from].indent;
                let group = 1 + page.blocks[from + 1..].iter().take_while(|b| b.indent > base).count();
                let target = rows.iter().filter(|r| r.center().y < p.y).count();
                let useful = target < from || target > from + group;
                if useful {
                    let y = if target == 0 {
                        rows[0].top() - 1.0
                    } else if target >= rows.len() {
                        rows[rows.len() - 1].bottom() + 1.0
                    } else {
                        (rows[target - 1].bottom() + rows[target].top()) / 2.0
                    };
                    let left = rows[0].left() + 40.0;
                    let right = rows.iter().map(|r| r.right()).fold(left, f32::max);
                    // Palette blue 500: rgba(35, 131, 226, 0.57).
                    let blue = Color32::from_rgba_unmultiplied(ACCENT[0], ACCENT[1], ACCENT[2], 145);
                    ui.painter().line_segment([egui::pos2(left, y), egui::pos2(right, y)], egui::Stroke::new(4.0, blue));
                }
                // A faint copy of the block follows the pointer.
                let ghost = page.blocks[from].text.lines().next().unwrap_or("").chars().take(60).collect::<String>();
                let layer = egui::LayerId::new(egui::Order::Tooltip, drag_key);
                let painter = ui.ctx().layer_painter(layer);
                let label = if ghost.trim().is_empty() { page.blocks[from].kind.label().to_string() } else { ghost };
                let galley = painter.layout_no_wrap(label, FontId::proportional(16.0), t.text.gamma_multiply(0.5));
                let pos = p + egui::vec2(14.0, -10.0);
                painter.rect_filled(egui::Rect::from_min_size(pos - egui::vec2(6.0, 3.0), galley.size() + egui::vec2(12.0, 6.0)), 4.0, t.hover);
                painter.galley(pos, galley, t.text);
                // Scroll when the pointer nears the top or bottom edge.
                let clip = ui.clip_rect();
                if p.y < clip.top() + 40.0 {
                    ui.scroll_with_delta(egui::vec2(0.0, 10.0));
                } else if p.y > clip.bottom() - 40.0 {
                    ui.scroll_with_delta(egui::vec2(0.0, -10.0));
                }
                ui.ctx().request_repaint();
                if drag_stopped || ui.input(|i| i.pointer.any_released()) {
                    if useful {
                        moved = Some((from, target));
                    }
                    ui.data_mut(|d| d.remove::<String>(drag_key));
                }
            }
            _ => ui.data_mut(|d| d.remove::<String>(drag_key)),
        }
    }
    if let Some((from, target)) = moved {
        let base = page.blocks[from].indent;
        let group = 1 + page.blocks[from + 1..].iter().take_while(|b| b.indent > base).count();
        let moved_blocks: Vec<Block> = page.blocks.drain(from..from + group).collect();
        let to = if target > from { target - group } else { target };
        let to = to.min(page.blocks.len());
        // Keep the nesting valid where it lands: at most one level under the block above.
        let limit = if to == 0 { 0 } else { page.blocks[to - 1].indent + 1 };
        let shift = base.saturating_sub(limit);
        for (k, mut b) in moved_blocks.into_iter().enumerate() {
            b.indent = b.indent.saturating_sub(shift);
            page.blocks.insert(to + k, b);
        }
    } else {
        if let Some(r) = remove {
            if r < page.blocks.len() {
                page.blocks.remove(r);
            }
        }
        if let Some((at, b)) = insert {
            let at = at.min(page.blocks.len());
            page.blocks.insert(at, b);
        }
    }
    if page.blocks.is_empty() && page.database.is_none() {
        page.blocks.push(Block::new(BlockKind::Paragraph, ""));
    }
    (new_focus, focus_used)
}

/// Make a database sub-page of `page` shown in `view`, and link it at block `at`; returns its id.
fn insert_database(ws: &mut Workspace, page: &str, at: usize, view: DbView) -> String {
    let (title, icon) = match view {
        DbView::Board => (crate::tr!("Kanban", "Kanban"), "📋"),
        DbView::Table => (crate::tr!("Base de données", "Database"), "🗃️"),
        DbView::List => (crate::tr!("Liste", "List"), "📝"),
    };
    let mut p = Page::new(title, Some(page.to_string()));
    p.icon = icon.into();
    p.blocks.clear();
    let mut db = Database::new_default();
    db.view = view;
    p.database = Some(db);
    let id = p.id.clone();
    ws.pages.push(p);
    if let Some(b) = ws.page_mut(page).and_then(|par| par.blocks.get_mut(at)) {
        b.kind = BlockKind::PageLink;
        b.text.clear();
        b.extra = id.clone();
    }
    id
}

/// What the "/" menu does with the chosen entry.
fn apply_item(
    b: &mut Block,
    item: slash::Item,
    i: usize,
    page_id: &str,
    actions: &mut Vec<Action>,
    insert: &mut Option<(usize, Block)>,
    new_focus: &mut Option<(String, Option<usize>)>,
) {
    match item {
        slash::Item::Block(k) => apply_kind(b, k, i, page_id, actions, insert, new_focus),
        slash::Item::Database(view) => {
            b.text.clear();
            b.kind = BlockKind::PageLink;
            actions.push(Action::InsertDatabase { page: page_id.to_string(), at: i, view });
        }
        slash::Item::Today => {
            b.text = format!("{} ", ai_ui::today_label());
            *new_focus = Some((b.id.clone(), Some(b.text.chars().count())));
        }
    }
}

fn apply_kind(
    b: &mut Block,
    k: BlockKind,
    i: usize,
    page_id: &str,
    actions: &mut Vec<Action>,
    insert: &mut Option<(usize, Block)>,
    new_focus: &mut Option<(String, Option<usize>)>,
) {
    b.text.clear();
    b.kind = k;
    match k {
        BlockKind::PageLink => actions.push(Action::InsertSubpage { page: page_id.to_string(), at: i }),
        BlockKind::Divider => {
            let nb = Block::new(BlockKind::Paragraph, "");
            *new_focus = Some((nb.id.clone(), Some(0)));
            *insert = Some((i + 1, nb));
        }
        _ => {}
    }
}

// -------------------------------------------------------------------- databases

mod ai_fill;
mod notices;
mod ai_ui;
mod chrome;
mod projects;
mod rich;
mod search;
mod term_ui;
mod update_ui;
mod versions;
mod side;
mod live_ui;
mod orch_ui;
mod settings;
mod slash;
use settings::SettingsTab;
mod database;
mod icons;
use chrome::SIDEBAR_WIDTH;
use database::database_view;

fn board_view(ui: &mut egui::Ui, page_id: &str, db: &mut Database, actions: &mut Vec<Action>) {
    let Some(group) = db.columns.iter().find(|c| c.kind == ColumnKind::Select).cloned() else {
        ui.label(RichText::new(crate::tr!("Ajoutez une colonne « Sélection » pour regrouper les lignes en Kanban.", "Add a « Select » column to group the rows as a Kanban.")).weak());
        return;
    };
    let title_id = db.title_col().map(|c| c.id.clone()).unwrap_or_default();
    let mut groups: Vec<String> = vec![String::new()];
    groups.extend(group.options.iter().cloned());
    let rows = db.visible_rows();
    let mut moves: Vec<(String, String)> = vec![];
    let mut add_in: Option<String> = None;
    egui::ScrollArea::horizontal().id_salt(("board", page_id)).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for (gi, g) in groups.iter().enumerate() {
                let in_group: Vec<usize> =
                    rows.iter().copied().filter(|&r| db.rows[r].cells.get(&group.id).cloned().unwrap_or_default() == *g).collect();
                if g.is_empty() && in_group.is_empty() {
                    continue;
                }
                ui.vertical(|ui| {
                    ui.set_width(220.0);
                    let name = if g.is_empty() { crate::tr!("Sans statut", "No status") } else { g.as_str() };
                    ui.label(RichText::new(format!("{name}  {}", in_group.len())).strong());
                    for r in in_group {
                        let rid = db.rows[r].id.clone();
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_width(205.0);
                            let t = db.rows[r].cells.entry(title_id.clone()).or_default();
                            ui.push_id(("card", &rid), |ui| {
                                ui.add(egui::TextEdit::singleline(t).hint_text(crate::tr!("Sans titre", "Untitled")).frame(egui::Frame::NONE).desired_width(200.0));
                            });
                            ui.horizontal(|ui| {
                                if gi > 1 && ui.small_button("◀").clicked() {
                                    moves.push((rid.clone(), groups[gi - 1].clone()));
                                }
                                if gi + 1 < groups.len() && ui.small_button("▶").clicked() {
                                    moves.push((rid.clone(), groups[gi + 1].clone()));
                                }
                                if ui.small_button("↗").on_hover_text(crate::tr!("Ouvrir comme page", "Open as page")).clicked() {
                                    actions.push(Action::OpenRow(page_id.to_string(), rid.clone()));
                                }
                            });
                        });
                    }
                    if !g.is_empty() && ui.add(egui::Button::new(RichText::new(crate::tr!("+ Nouveau", "+ New")).weak()).frame(false)).clicked() {
                        add_in = Some(g.clone());
                    }
                });
                ui.add_space(8.0);
            }
        });
    });
    for (rid, g) in moves {
        if let Some(r) = db.rows.iter_mut().find(|r| r.id == rid) {
            r.cells.insert(group.id.clone(), g);
        }
    }
    if let Some(g) = add_in {
        let rid = db.add_row("");
        if let Some(r) = db.rows.iter_mut().find(|r| r.id == rid) {
            r.cells.insert(group.id.clone(), g);
        }
    }
}

fn list_view(ui: &mut egui::Ui, page_id: &str, db: &mut Database, actions: &mut Vec<Action>) {
    for r in db.visible_rows() {
        let row = &db.rows[r];
        let title = db.row_title(row);
        let rest: Vec<String> = db
            .columns
            .iter()
            .skip(1)
            .filter_map(|c| {
                let v = row.cells.get(&c.id)?;
                match c.kind {
                    ColumnKind::Checkbox if v == "true" => Some(format!("☑ {}", c.name)),
                    ColumnKind::Checkbox => None,
                    _ if v.is_empty() => None,
                    _ => Some(v.clone()),
                }
            })
            .collect();
        ui.horizontal(|ui| {
            let label = format!("📄 {}", if title.is_empty() { crate::tr!("Sans titre", "Untitled") } else { &title });
            if ui.add(egui::Button::new(RichText::new(label).size(15.0)).frame(false)).clicked() {
                actions.push(Action::OpenRow(page_id.to_string(), row.id.clone()));
            }
            ui.label(RichText::new(rest.join("  ·  ")).weak());
        });
    }
    if ui.button(crate::tr!("+ Nouvelle ligne", "+ New row")).clicked() {
        db.add_row("");
    }
}

// -------------------------------------------------------------------- helpers

fn page_icon(p: &Page) -> &str {
    if p.icon.is_empty() {
        if p.database.is_some() { "📋" } else { "📄" }
    } else {
        p.icon.as_str()
    }
}

fn title_with_icon(p: &Page) -> String {
    format!("{} {}", page_icon(p), p.display_title())
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Accent blue, palette blue 600 = rgba(35, 131, 226, 1); selection uses blue 400 (alpha 0.35).
const ACCENT: [u8; 3] = [35, 131, 226];

fn setup_style(ctx: &egui::Context) {
    use original::{dark as d, light as l};
    ctx.style_mut_of(egui::Theme::Light, |s| {
        s.visuals.panel_fill = rgb(l::BACKGROUND);
        s.visuals.window_fill = rgb(l::BACKGROUND);
        s.visuals.extreme_bg_color = rgb(l::BACKGROUND);
        s.visuals.faint_bg_color = rgb(l::SIDEBAR);
        s.visuals.override_text_color = Some(rgb(l::TEXT));
        s.visuals.weak_text_color = Some(rgb(l::TEXT_SECONDARY));
        s.visuals.window_stroke.color = rgb(l::BORDER);
        s.visuals.widgets.noninteractive.bg_stroke.color = rgb(l::BORDER);
        s.visuals.widgets.hovered.weak_bg_fill = rgb(l::INTERACTIVE);
        s.visuals.selection.bg_fill = rgb(ACCENT).gamma_multiply(0.35);
        s.visuals.hyperlink_color = rgb(ACCENT);
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
    });
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.visuals.panel_fill = rgb(d::BACKGROUND);
        s.visuals.window_fill = rgb(d::SIDEBAR);
        s.visuals.extreme_bg_color = rgb(d::BACKGROUND);
        s.visuals.faint_bg_color = rgb(d::SIDEBAR);
        s.visuals.override_text_color = Some(rgb(d::TEXT));
        s.visuals.weak_text_color = Some(rgb(d::TEXT_SECONDARY));
        s.visuals.window_stroke.color = rgb(d::BORDER);
        s.visuals.widgets.noninteractive.bg_stroke.color = rgb(d::BORDER);
        s.visuals.widgets.hovered.weak_bg_fill = rgb(d::INTERACTIVE);
        s.visuals.selection.bg_fill = rgb(ACCENT).gamma_multiply(0.35);
        s.visuals.hyperlink_color = rgb(ACCENT);
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
    });
    // Shared flat look: borderless buttons with a rounded hover tint, rounded menus with the
    // original's outline shadow, thin floating scroll bars.
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let t = tone(theme == egui::Theme::Dark);
        ctx.style_mut_of(theme, |s| {
            let v = &mut s.visuals;
            v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, t.border);
            v.widgets.hovered.weak_bg_fill = t.hover;
            v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, t.border);
            v.widgets.active.weak_bg_fill = t.hover;
            v.widgets.open.weak_bg_fill = t.hover;
            for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
                w.corner_radius = egui::CornerRadius::same(6);
                w.fg_stroke.color = t.text;
            }
            v.widgets.inactive.fg_stroke.color = t.accent_text;
            v.window_corner_radius = egui::CornerRadius::same(10);
            // Menus and pop-ups: #252525 in dark (sampled on the reference), white in light.
            v.window_fill = if theme == egui::Theme::Dark { Color32::from_rgb(37, 37, 37) } else { Color32::WHITE };
            v.widgets.hovered.weak_bg_fill = t.selected;
            s.spacing.interact_size.y = 28.0;
            v.menu_corner_radius = egui::CornerRadius::same(10);
            // Light "outline.lg": 0 20 24 rgba(25,25,25,0.05) + 1 px rgba(42,28,0,0.07) (0x1a3fde).
            let shadow = egui::Shadow { offset: [0, 12], blur: 24, spread: 0, color: Color32::from_rgba_unmultiplied(25, 25, 25, if theme == egui::Theme::Dark { 90 } else { 22 }) };
            v.popup_shadow = shadow;
            v.window_shadow = shadow;
            v.window_stroke = egui::Stroke::new(1.0, if theme == egui::Theme::Dark { t.border } else { Color32::from_rgba_unmultiplied(42, 28, 0, 18) });
            v.text_cursor.stroke = egui::Stroke::new(1.5, t.text);
            s.spacing.button_padding = egui::vec2(8.0, 4.0);
            s.spacing.menu_margin = egui::Margin::same(6);
            s.spacing.scroll = egui::style::ScrollStyle::thin();
            s.interaction.selectable_labels = false;
        });
    }
}

/// Notion's font stack starts with the system font (`ui-sans-serif, -apple-system,
/// BlinkMacSystemFont, "Segoe UI Variable Display", "Segoe UI", Helvetica…`, renderer/tabs
/// @0x398af). Load it (regular, and semi-bold for titles) in front of egui's fonts, which stay as
/// fallbacks for emoji and missing characters.
pub fn setup_fonts(ctx: &egui::Context) {
    use egui::epaint::text::{FontData, FontTweak, VariationCoords};
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    let read = |paths: &[&str]| paths.iter().find_map(|p| std::fs::read(p).ok());
    // (regular file, semi-bold file or None when the regular one is a variable font)
    let (regular, bold_file) = if cfg!(target_os = "macos") {
        (read(&["/System/Library/Fonts/SFNS.ttf"]), None)
    } else if cfg!(windows) {
        (read(&["C:\\Windows\\Fonts\\segoeui.ttf"]), read(&["C:\\Windows\\Fonts\\seguisb.ttf"]))
    } else {
        (
            read(&["/usr/share/fonts/truetype/inter/Inter-Regular.ttf", "/usr/share/fonts/opentype/inter/Inter-Regular.otf"]),
            read(&["/usr/share/fonts/truetype/inter/Inter-SemiBold.ttf", "/usr/share/fonts/opentype/inter/Inter-SemiBold.otf"]),
        )
    };
    let fallbacks = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    let mut semibold_family = vec![];
    let mut bold_family: Vec<String> = vec![];
    if let Some(bytes) = regular {
        fonts.font_data.insert("system".into(), Arc::new(FontData::from_owned(bytes.clone())));
        if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            list.insert(0, "system".into());
        }
        let weight = |w: f32| FontTweak { coords: VariationCoords::new([(b"wght", w)]), ..Default::default() };
        let (semibold_data, bold_data) = match bold_file {
            Some(b) => (FontData::from_owned(b.clone()), FontData::from_owned(b)),
            None => (FontData::from_owned(bytes.clone()).tweak(weight(600.0)), FontData::from_owned(bytes).tweak(weight(700.0))),
        };
        fonts.font_data.insert("system-semibold".into(), Arc::new(semibold_data));
        fonts.font_data.insert("system-bold".into(), Arc::new(bold_data));
        semibold_family.push("system-semibold".to_string());
        bold_family.push("system-bold".to_string());
    }
    semibold_family.extend(fallbacks.clone());
    bold_family.extend(fallbacks);
    fonts.families.insert(egui::FontFamily::Name(SEMIBOLD.into()), semibold_family);
    fonts.families.insert(egui::FontFamily::Name(BOLD.into()), bold_family);
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    fn page_abc() -> Page {
        let mut p = Page::new("Test", None);
        p.blocks = vec![
            Block::new(BlockKind::Paragraph, "Alpha"),
            Block::new(BlockKind::Paragraph, "Beta"),
            Block::new(BlockKind::Paragraph, "Gamma"),
        ];
        p
    }

    fn harness() -> Harness<'static, Page> {
        Harness::builder().with_size([900.0, 600.0]).build_ui_state(
            |ui, page: &mut Page| {
                let mut actions = vec![];
                blocks_view(ui, page, &HashMap::new(), None, &mut actions);
            },
            page_abc(),
        )
    }

    fn row_rect(h: &Harness<Page>, block_id: &str) -> egui::Rect {
        h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("block", block_id)).with("row"))).expect("row drawn")
    }

    /// Centre of the six-dot handle: after the 22 px "+" and 2 px spacing, 18 px wide.
    fn handle_of(r: egui::Rect) -> egui::Pos2 {
        egui::pos2(r.left() + 22.0 + 2.0 + 9.0, r.center().y)
    }

    fn drag(h: &mut Harness<Page>, from: egui::Pos2, to: egui::Pos2) {
        h.hover_at(from);
        h.run_steps(2);
        h.drag_at(from);
        h.run_steps(1);
        for k in 1..=10 {
            let p = from + (to - from) * (k as f32 / 10.0);
            h.hover_at(p);
            h.run_steps(1);
        }
        h.drop_at(to);
        h.run_steps(3);
    }

    fn texts(h: &Harness<Page>) -> Vec<String> {
        h.state().blocks.iter().map(|b| b.text.clone()).collect()
    }

    #[test]
    fn dragging_a_block_down_moves_it() {
        let mut h = harness();
        h.run_steps(3);
        let a = h.state().blocks[0].id.clone();
        let c = h.state().blocks[2].id.clone();
        let from = handle_of(row_rect(&h, &a));
        let to = egui::pos2(from.x + 40.0, row_rect(&h, &c).bottom() + 4.0);
        drag(&mut h, from, to);
        assert_eq!(texts(&h), ["Beta", "Gamma", "Alpha"]);
    }

    #[test]
    fn dragging_a_block_up_moves_it() {
        let mut h = harness();
        h.run_steps(3);
        let a = h.state().blocks[0].id.clone();
        let c = h.state().blocks[2].id.clone();
        let from = handle_of(row_rect(&h, &c));
        let to = egui::pos2(from.x, row_rect(&h, &a).top() + 2.0);
        drag(&mut h, from, to);
        assert_eq!(texts(&h), ["Gamma", "Alpha", "Beta"]);
    }

    #[test]
    fn inline_styles_are_found_without_changing_the_text() {
        let t = "Tapez `/` puis **gras** et *italique* et ~~barré~~";
        let spans = inline_spans(t);
        let styles: Vec<Inline> = spans.iter().map(|s| s.3).collect();
        assert!(styles == [Inline::Code, Inline::Bold, Inline::Italic, Inline::Strike]);
        let job = block_job(t, BlockKind::Paragraph, false, false, &tone(false));
        assert_eq!(job.text, t);
    }

    #[test]
    fn a_block_moves_with_its_nested_children() {
        let mut h = harness();
        {
            let page = h.state_mut();
            page.blocks.insert(1, Block::new(BlockKind::Paragraph, "Alpha child"));
            page.blocks[1].indent = 1;
        }
        h.run_steps(3);
        let a = h.state().blocks[0].id.clone();
        let c = h.state().blocks[3].id.clone();
        let from = handle_of(row_rect(&h, &a));
        let to = egui::pos2(from.x + 40.0, row_rect(&h, &c).bottom() + 4.0);
        drag(&mut h, from, to);
        assert_eq!(texts(&h), ["Beta", "Gamma", "Alpha", "Alpha child"]);
        assert_eq!(h.state().blocks[3].indent, 1);
    }

    #[test]
    fn slash_menu_filters_in_notion_order() {
        use slash::Item;
        let all = slash::matches("/");
        assert_eq!(all.len(), 17);
        assert_eq!(all[0].0, Item::Block(BlockKind::Paragraph));
        assert_eq!(all[13].0, Item::Database(DbView::Table));
        assert_eq!(all[14].0, Item::Database(DbView::Board));
        assert_eq!(all[15].0, Item::Database(DbView::List));
        assert_eq!(all[16].0, Item::Today);
        let l: Vec<Item> = slash::matches("/liste").into_iter().map(|x| x.0).collect();
        let mut want = [BlockKind::Bullet, BlockKind::Numbered, BlockKind::Todo, BlockKind::Toggle].map(Item::Block).to_vec();
        want.push(Item::Database(DbView::List));
        assert_eq!(l, want);
        assert!(slash::matches("/zzz").is_empty());
        for q in ["/kanban", "/kan", "/tableau", "/board"] {
            assert!(slash::matches(q).iter().any(|x| x.0 == Item::Database(DbView::Board)), "{q}");
        }
        for q in ["/base", "/database", "/table"] {
            assert!(slash::matches(q).iter().any(|x| x.0 == Item::Database(DbView::Table)), "{q}");
        }
        for q in ["/date", "/aujourd", "/today"] {
            assert!(slash::matches(q).iter().any(|x| x.0 == Item::Today), "{q}");
        }
    }

    #[test]
    fn slash_database_makes_a_subpage_in_its_view() {
        for view in [DbView::Board, DbView::Table, DbView::List] {
            let mut ws = Workspace::default();
            let page = ws.create_page("Notes", None);
            ws.page_mut(&page).unwrap().blocks = vec![Block::new(BlockKind::Paragraph, "/base")];
            let id = insert_database(&mut ws, &page, 0, view);
            let kb = ws.page(&id).unwrap();
            assert_eq!(kb.parent.as_deref(), Some(page.as_str()));
            let db = kb.database.as_ref().unwrap();
            assert_eq!(db.view, view);
            assert!(db.columns.iter().any(|c| c.kind == ColumnKind::Select));
            let link = &ws.page(&page).unwrap().blocks[0];
            assert_eq!((link.kind, link.extra.as_str(), link.text.as_str()), (BlockKind::PageLink, id.as_str(), ""));
        }
    }

    #[test]
    fn slash_today_writes_the_date() {
        let mut b = Block::new(BlockKind::Paragraph, "/date");
        let (mut actions, mut insert, mut focus) = (vec![], None, None);
        apply_item(&mut b, slash::Item::Today, 0, "p", &mut actions, &mut insert, &mut focus);
        assert!(b.text.ends_with(' ') && b.text.chars().any(|c| c.is_ascii_digit()) && !b.text.starts_with('/'), "{}", b.text);
        assert_eq!(b.kind, BlockKind::Paragraph);
        assert!(actions.is_empty() && insert.is_none());
        assert_eq!(focus, Some((b.id.clone(), Some(b.text.chars().count()))));
    }

    #[test]
    fn notion_markdown_shortcuts() {
        assert_eq!(markdown_shortcut("> x").map(|x| x.0), Some(BlockKind::Toggle));
        assert_eq!(markdown_shortcut("\" x").map(|x| x.0), Some(BlockKind::Quote));
        assert_eq!(markdown_shortcut("## x").map(|x| x.0), Some(BlockKind::Heading2));
    }

    #[test]
    fn dropping_in_place_changes_nothing() {
        let mut h = harness();
        h.run_steps(3);
        let b = h.state().blocks[1].id.clone();
        let from = handle_of(row_rect(&h, &b));
        drag(&mut h, from, from + egui::vec2(30.0, 2.0));
        assert_eq!(texts(&h), ["Alpha", "Beta", "Gamma"]);
    }
}

/// ORCHESTRA_PROFILE=1: time every frame and print the average every 60 frames (to find
/// what makes scrolling slow).
struct Profile {
    start: Option<Instant>,
}

impl Profile {
    fn frame(ctx: &egui::Context, cpu: Option<f32>) -> Profile {
        if std::env::var_os("ORCHESTRA_PROFILE").is_none() {
            return Profile { start: None };
        }
        static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = LAST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n % 60 == 0 {
            let dt = ctx.input(|i| i.unstable_dt);
            let size = ctx.content_rect().size() * ctx.pixels_per_point();
            eprintln!(
                "image complète (interface + dessin) : {:.1} ms · {:.0} images/s · fenêtre {:.0}×{:.0} pixels",
                cpu.unwrap_or(0.0) * 1000.0,
                if dt > 0.0 { 1.0 / dt } else { 0.0 },
                size.x,
                size.y
            );
            Section::report();
        }
        ctx.request_repaint();
        Profile { start: Some(Instant::now()) }
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        static TOTAL: AtomicU64 = AtomicU64::new(0);
        static MAX: AtomicU64 = AtomicU64::new(0);
        if let Some(s) = self.start {
            let us = s.elapsed().as_micros() as u64;
            TOTAL.fetch_add(us, Ordering::Relaxed);
            MAX.fetch_max(us, Ordering::Relaxed);
            if N.fetch_add(1, Ordering::Relaxed) % 60 == 59 {
                eprintln!("ui(): moyenne {} µs, max {} µs", TOTAL.swap(0, Ordering::Relaxed) / 60, MAX.swap(0, Ordering::Relaxed));
            }
        }
    }
}

/// Time spent in parts of the window (ORCHESTRA_PROFILE=1), reported with the frames.
struct Section {
    name: &'static str,
    start: Option<Instant>,
}

static SECTIONS: std::sync::Mutex<Vec<(&'static str, u64, u64)>> = std::sync::Mutex::new(Vec::new());

impl Section {
    fn start(name: &'static str) -> Section {
        let on = std::env::var_os("ORCHESTRA_PROFILE").is_some();
        Section { name, start: on.then(Instant::now) }
    }

    fn report() {
        let mut v = SECTIONS.lock().unwrap();
        for (name, total, n) in v.iter() {
            if *n > 0 {
                eprintln!("   {name} : {} µs en moyenne", total / n);
            }
        }
        v.clear();
    }
}

impl Drop for Section {
    fn drop(&mut self) {
        if let Some(s) = self.start {
            let us = s.elapsed().as_micros() as u64;
            let mut v = SECTIONS.lock().unwrap();
            match v.iter_mut().find(|x| x.0 == self.name) {
                Some(x) => {
                    x.1 += us;
                    x.2 += 1;
                }
                None => v.push((self.name, us, 1)),
            }
        }
    }
}
