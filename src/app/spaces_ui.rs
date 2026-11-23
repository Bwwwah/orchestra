//! The space switcher (« Mon espace ⌄ » at the bottom of the sidebar) and the move from one
//! space to another (see space.rs and docs/collaboration-serveur.md).
//!
//! Changing space is refused while an AI works: its results (a filled page, proposed steps,
//! a session's report…) would otherwise land in the space shown at the time they come back.

use super::icons::Icon;
use super::settings::menu_item;
use super::*;
use crate::space::{self, LOCAL};

impl NotesApp {
    /// The name of the space shown: « Mon espace » keeps the name chosen in the settings.
    pub(super) fn space_name(&self) -> String {
        let id = store::space_id();
        if id == LOCAL {
            let n = self.ws.settings.workspace_name.trim();
            return if n.is_empty() {
                crate::tr!("Mon espace", "My workspace").to_string()
            } else {
                n.to_string()
            };
        }
        space::load()
            .get(&id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| crate::tr!("Espace", "Space").to_string())
    }

    /// Why the space can't be changed right now (an AI or a terminal at work), if so.
    pub(super) fn space_busy(&self) -> Option<String> {
        let runs = self.runs_working();
        if runs > 0 {
            return Some(crate::trf!(
                "{runs} session(s) autonome(s) en cours",
                "{runs} autonomous session(s) running"
            ));
        }
        if self.ai.any() || !self.reviews.is_empty() {
            return Some(
                crate::tr!("une IA est en train de répondre", "an AI is answering").into(),
            );
        }
        if self.sync_run.as_ref().is_some_and(|r| !r.snapshot().2) {
            return Some(
                crate::tr!(
                    "un état des lieux est en cours",
                    "a state-of-play is running"
                )
                .into(),
            );
        }
        let terms = self.terms.iter().filter(|t| !t.exited()).count();
        if terms > 0 {
            return Some(crate::trf!(
                "{terms} terminal(aux) ouvert(s)",
                "{terms} terminal(s) open"
            ));
        }
        None
    }

    /// Show space `id`: save this one, then load that one with the person's own settings.
    pub(super) fn switch_space(&mut self, id: &str) {
        if id == store::space_id() {
            return;
        }
        if let Some(why) = self.space_busy() {
            self.info(crate::trf!(
                "Changement d'espace impossible pour l'instant : {why}. Attendez la fin (ou fermez les terminaux), puis réessayez.",
                "Can't change space right now: {why}. Wait for the end (or close the terminals), then try again."
            ));
            return;
        }
        let mut spaces = space::load();
        if spaces.get(id).is_none() {
            return;
        }
        self.detect_changes();
        self.save_now();
        self.stop_live();
        let settings = self.ws.settings.clone();
        store::use_space(id);
        let path = store::workspace_path();
        let existed = path.exists();
        let mut ws = if existed {
            store::load()
        } else {
            Workspace::default()
        };
        ws.settings = settings;
        self.ws = ws;
        self.known_mtime = store::modified(&path);
        spaces.active = id.to_string();
        if let Err(e) = space::save(&spaces) {
            self.fail(crate::trf!(
                "Espace choisi non retenu pour le prochain lancement : {e}",
                "Chosen space not kept for the next launch: {e}"
            ));
        }
        // Everything that pointed into the other space's pages.
        let first = self
            .ws
            .pages
            .iter()
            .find(|p| !p.trashed && p.parent.is_none())
            .map(|p| p.id.clone());
        self.view = if first.is_some() {
            View::Page
        } else {
            View::Home
        };
        self.current = first.clone();
        self.tabs = vec![Tab {
            page: first,
            ..Default::default()
        }];
        self.active_tab = 0;
        self.closed_tabs.clear();
        self.expanded.clear();
        self.focus = None;
        self.search_open = false;
        self.inbox_removed.clear();
        self.live_sent.clear();
        self.live_deleted_sent = 0;
        self.chat_sent.clear();
        self.new_project = None;
        self.launch_settings = None;
        self.sync_dialog = None;
        self.thread_pid = None;
        self.ai_session = None;
        self.ai.proposals.clear();
        self.ai.answers.clear();
        self.take_snapshot();
        self.dirty_since = None;
        if !existed {
            self.save_now();
        }
        let name = self.space_name();
        self.ack(crate::trf!("Espace « {name} »", "Space « {name} »"));
    }

    /// The switcher's menu: the spaces, a new one, the settings.
    pub(super) fn space_menu(&mut self, ui: &mut egui::Ui) {
        let t = tone(ui.visuals().dark_mode);
        ui.set_min_width(260.0);
        ui.set_max_width(260.0);
        let spaces = space::load();
        let here = store::space_id();
        let mut pick: Option<String> = None;
        ui.label(
            RichText::new(crate::tr!("Espaces", "Spaces"))
                .size(12.0)
                .color(t.tertiary),
        );
        for s in &spaces.spaces {
            let name = if s.is_local() {
                let n = self.ws.settings.workspace_name.trim();
                if n.is_empty() {
                    s.name.clone()
                } else {
                    n.to_string()
                }
            } else {
                s.name.clone()
            };
            if menu_item(ui, None, &name, s.id == here, &t).clicked() {
                pick = Some(s.id.clone());
                ui.close();
            }
        }
        ui.separator();
        let key = egui::Id::unique("new-space-name");
        let mut draft: String = ui.data(|d| d.get_temp(key)).unwrap_or_default();
        let mut create = false;
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut draft)
                    .hint_text(crate::tr!("Nouvel espace…", "New space…"))
                    .desired_width(170.0),
            );
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui
                .add_enabled(
                    !draft.trim().is_empty(),
                    egui::Button::new(crate::tr!("Créer", "Create")),
                )
                .clicked()
                || enter)
                && !draft.trim().is_empty()
            {
                create = true;
            }
        });
        ui.label(
            RichText::new(crate::tr!(
                "Sur cet ordinateur pour l'instant ; rejoindre un serveur viendra ensuite.",
                "On this computer for now; joining a server comes next."
            ))
            .size(11.5)
            .color(t.tertiary),
        );
        if create {
            let mut list = space::load();
            let id = list.add(&draft, space::SpaceKind::Local);
            match space::save(&list) {
                Ok(()) => pick = Some(id),
                Err(e) => self.fail(crate::trf!(
                    "Espace non créé : {e}",
                    "Space not created: {e}"
                )),
            }
            draft.clear();
            ui.close();
        }
        ui.data_mut(|d| d.insert_temp(key, draft));
        ui.separator();
        if menu_item(
            ui,
            Some(Icon::Gear),
            crate::tr!("Réglages", "Settings"),
            false,
            &t,
        )
        .clicked()
        {
            self.open_settings(SettingsTab::Preferences);
            ui.close();
        }
        if let Some(id) = pick {
            self.switch_space(&id);
        }
    }
}
