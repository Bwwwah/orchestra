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
        // A team space: all its pages are the team's (a new one too, made empty here).
        ws.whole_shared = space::current().server().is_some();
        crate::collab::give_documents(&mut ws, None, true);
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
        self.start_team();
        let name = self.space_name();
        self.ack(crate::trf!("Espace « {name} »", "Space « {name} »"));
    }

    /// A team space: connect to its server (nothing for a space of this computer).
    pub(super) fn start_team(&mut self) {
        let sp = space::current();
        let Some((url, invite)) = sp.server() else {
            return;
        };
        // Not `stop_live`: the person's live setting of « Mon espace » stays as it is.
        if let Some(l) = self.live.take() {
            l.send(&crate::live::LiveMsg::Bye {
                name: self.my_name(),
            });
        }
        let ctx = self.ctx.clone();
        let wake: crate::live::Wake = std::sync::Arc::new(move || ctx.request_repaint());
        self.live = Some(crate::team::connect(
            &sp.id,
            url,
            invite,
            &self.my_name(),
            self.ws.server_seq,
            wake,
        ));
        self.live_sent.clear();
        self.team_marks.clear();
        self.peers.clear();
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
                "Un espace sur cet ordinateur seulement.",
                "A space on this computer only."
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

        // Join a team space with the invitation link its server gave.
        let jkey = egui::Id::unique("join-space-link");
        let mut link: String = ui.data(|d| d.get_temp(jkey)).unwrap_or_default();
        let parsed = crate::team::parse_invitation(&link);
        let mut join = false;
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut link)
                    .hint_text(crate::tr!("Lien d'invitation…", "Invitation link…"))
                    .desired_width(170.0),
            );
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui
                .add_enabled(
                    parsed.is_some(),
                    egui::Button::new(crate::tr!("Rejoindre", "Join")),
                )
                .clicked()
                || enter)
                && parsed.is_some()
            {
                join = true;
            }
        });
        if !link.trim().is_empty() && parsed.is_none() {
            ui.label(
                RichText::new(crate::tr!(
                    "Lien attendu : ws://adresse:port/?invitation=…",
                    "Expected link: ws://address:port/?invitation=…"
                ))
                .size(11.5)
                .color(super::style::RED),
            );
        }
        if join && let Some((url, invite)) = parsed {
            let mut list = space::load();
            let id = list.add(
                crate::tr!("Espace d'équipe", "Team space"),
                space::SpaceKind::Server { url, invite },
            );
            match space::save(&list) {
                Ok(()) => pick = Some(id),
                Err(e) => self.fail(crate::trf!(
                    "Espace non ajouté : {e}",
                    "Space not added: {e}"
                )),
            }
            link.clear();
            ui.close();
        }
        ui.data_mut(|d| d.insert_temp(jkey, link));
        ui.separator();
        // Stays open on a click inside: several things can be set in a row.
        egui::containers::menu::SubMenuButton::new(crate::tr!("Cet espace", "This space"))
            .config(
                egui::containers::menu::MenuConfig::new()
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
            )
            .ui(ui, |ui| self.this_space_menu(ui));
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

    /// « Cet espace › »: its name, the AIs it allows, taking it off the list.
    fn this_space_menu(&mut self, ui: &mut egui::Ui) {
        let t = tone(ui.visuals().dark_mode);
        ui.set_min_width(280.0);
        ui.set_max_width(280.0);
        let here = store::space_id();
        let mut list = space::load();
        let Some(cur) = list.get(&here).cloned() else {
            return;
        };
        let mut changed = false;

        // Name.
        let key = egui::Id::unique(("space-rename", here.as_str()));
        let mut name: String = ui
            .data(|d| d.get_temp(key))
            .unwrap_or_else(|| self.space_name());
        ui.label(
            RichText::new(crate::tr!("Nom", "Name"))
                .size(12.0)
                .color(t.tertiary),
        );
        // A team space is named by its server.
        let from_server = cur.server().is_some();
        ui.add_enabled_ui(!from_server, |ui| {
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(180.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ok = !name.trim().is_empty() && name.trim() != self.space_name();
                if (ui
                    .add_enabled(ok, egui::Button::new(crate::tr!("Renommer", "Rename")))
                    .clicked()
                    || enter)
                    && ok
                {
                    let n = name.trim().to_string();
                    if cur.is_local() {
                        // « Mon espace » keeps its name in the settings, as before.
                        self.ws.settings.workspace_name = n.clone();
                    }
                    if let Some(s) = list.spaces.iter_mut().find(|s| s.id == here) {
                        s.name = n;
                    }
                    changed = true;
                }
            })
        });
        ui.data_mut(|d| d.insert_temp(key, name));
        if let Some((url, _)) = cur.server() {
            let st = self.live.as_ref().map(|l| l.status());
            let line = match st {
                Some(s) if s.connected => crate::trf!("Relié à {url}", "Connected to {url}"),
                Some(s) => match s.error {
                    Some(e) => crate::trf!("{url} : {e}", "{url}: {e}"),
                    None => crate::trf!("Connexion à {url}…", "Connecting to {url}…"),
                },
                None => crate::trf!("Non relié à {url}", "Not connected to {url}"),
            };
            ui.label(RichText::new(line).size(11.5).color(t.tertiary));
            if let Some(role) = crate::team::Role::from_key(&cur.role) {
                ui.label(
                    RichText::new(crate::trf!(
                        "Votre rôle : {}",
                        "Your role: {}",
                        role.label()
                    ))
                    .size(11.5)
                    .color(t.tertiary),
                );
                if role == crate::team::Role::Admin
                    && ui
                        .button(crate::tr!("Administrer l'espace…", "Manage the space…"))
                        .clicked()
                {
                    self.admin_open = true;
                    self.send_admin(crate::team::AdminOp::List);
                    ui.close();
                }
            }
            if self.live.as_ref().is_none_or(|l| l.stopped())
                && ui
                    .button(crate::tr!("Se reconnecter", "Reconnect"))
                    .clicked()
            {
                self.start_team();
            }
        }

        // AIs allowed (not for « Mon espace »: the person's own AIs always work there).
        if !cur.is_local() {
            ui.separator();
            ui.label(
                RichText::new(crate::tr!("IA autorisées ici", "AIs allowed here"))
                    .size(12.0)
                    .color(t.tertiary),
            );
            let mut rule = cur.ai_rule;
            let mut allowed = cur.allowed_ai.clone();
            ui.add_enabled_ui(!from_server, |ui| {
            ui.radio_value(
                &mut rule,
                space::AiRule::Free,
                crate::tr!("Vos IA", "Your AIs"),
            );
            ui.radio_value(
                &mut rule,
                space::AiRule::SpaceOnly,
                crate::tr!("IA de l'espace seulement", "The space's AIs only"),
            );
            ui.radio_value(
                &mut rule,
                space::AiRule::NoAi,
                crate::tr!("Aucune IA", "No AI"),
            );
            if rule == space::AiRule::Free {
                ui.label(
                    RichText::new(crate::tr!(
                        "Rien de coché : toutes vos IA.",
                        "Nothing ticked: all your AIs."
                    ))
                    .size(11.5)
                    .color(t.tertiary),
                );
                for a in crate::setup::Ai::ALL {
                    let mut on = allowed.iter().any(|k| k == a.key());
                    if ui.checkbox(&mut on, a.label()).changed() {
                        allowed.retain(|k| k != a.key());
                        if on {
                            allowed.push(a.key().to_string());
                        }
                    }
                }
            }
            if !cur.ais.is_empty() {
                ui.label(
                    RichText::new(crate::tr!("IA fournies par l'espace", "AIs the space provides"))
                        .size(12.0)
                        .color(t.tertiary),
                );
                for a in &cur.ais {
                    let tool = crate::setup::Ai::ALL
                        .into_iter()
                        .find(|x| x.key() == a.tool)
                        .map(|x| x.label())
                        .unwrap_or(a.tool.as_str());
                    ui.label(RichText::new(format!("• {} — {tool}", a.name)).size(12.5));
                }
                ui.label(
                    RichText::new(crate::tr!(
                        "Utilisées à la place de votre abonnement pour ces outils, dans cet espace.",
                        "Used instead of your subscription for these tools, in this space."
                    ))
                    .size(11.5)
                    .color(t.tertiary),
                );
            } else if rule == space::AiRule::SpaceOnly {
                ui.label(
                    RichText::new(crate::tr!(
                        "L'espace ne fournit pas encore d'IA : aucune IA ne travaille ici.",
                        "The space provides no AI yet: no AI works here."
                    ))
                    .size(11.5)
                    .color(t.tertiary),
                );
            }
            });
            if from_server {
                ui.label(
                    RichText::new(crate::tr!(
                        "Fixée par le serveur de l'espace.",
                        "Set by the space's server."
                    ))
                    .size(11.5)
                    .color(t.tertiary),
                );
            }
            if !from_server && (rule != cur.ai_rule || allowed != cur.allowed_ai) {
                if let Some(s) = list.spaces.iter_mut().find(|s| s.id == here) {
                    s.ai_rule = rule;
                    s.allowed_ai = allowed;
                }
                changed = true;
            }
            ui.label(
                RichText::new(crate::tr!(
                    "Orchestra respecte cette règle ; il ne peut pas empêcher de lancer une IA hors de l'application.",
                    "Orchestra keeps to this rule; it can't stop an AI being started outside the app."
                ))
                .size(11.5)
                .color(t.tertiary),
            );
        }

        if changed {
            if let Err(e) = space::save(&list) {
                self.fail(crate::trf!(
                    "Espace non enregistré : {e}",
                    "Space not saved: {e}"
                ));
            }
            self.dirty_since.get_or_insert_with(Instant::now);
        }

        // Take it off the list (two clicks; its files stay on disk).
        if !cur.is_local() {
            ui.separator();
            let confirm = egui::Id::unique(("space-remove", here.as_str()));
            let asked: bool = ui.data(|d| d.get_temp(confirm)).unwrap_or(false);
            let label = if asked {
                crate::tr!(
                    "Confirmer : retirer cet espace",
                    "Confirm: remove this space"
                )
            } else {
                crate::tr!("Retirer cet espace…", "Remove this space…")
            };
            if ui
                .button(RichText::new(label).color(super::style::RED))
                .clicked()
            {
                if asked {
                    ui.data_mut(|d| d.remove_temp::<bool>(confirm));
                    ui.close();
                    self.remove_space(&here);
                } else {
                    ui.data_mut(|d| d.insert_temp(confirm, true));
                }
            }
        }
    }

    /// Take space `id` off the list, after going back to « Mon espace ». Its folder stays on
    /// disk (nothing is deleted).
    fn remove_space(&mut self, id: &str) {
        self.switch_space(LOCAL);
        if store::space_id() != LOCAL {
            return; // refused (an AI at work): said by `switch_space`
        }
        let mut list = space::load();
        let name = list.get(id).map(|s| s.name.clone()).unwrap_or_default();
        list.remove(id);
        match space::save(&list) {
            Ok(()) => self.ack(crate::trf!(
                "Espace « {name} » retiré de la liste ; ses fichiers restent dans {}",
                "Space « {name} » taken off the list; its files stay in {}",
                space::dir_of(id).display()
            )),
            Err(e) => self.fail(crate::trf!(
                "Espace non retiré : {e}",
                "Space not removed: {e}"
            )),
        }
    }
}

/// Drafts of the administration window, kept between frames.
#[derive(Clone, Default)]
struct AdminDraft {
    invite_role: Option<crate::team::Role>,
    ai_name: String,
    ai_tool: String,
    ai_url: String,
    ai_model: String,
    /// Per AI: the key being typed, and the quota.
    keys: std::collections::HashMap<String, String>,
    quotas: std::collections::HashMap<String, String>,
    name: Option<String>,
}

impl NotesApp {
    /// Ask the team server for something (administrators only; it checks).
    pub(super) fn send_admin(&mut self, op: crate::team::AdminOp) {
        match &self.live {
            Some(l) if l.status().connected => l.send(&crate::live::LiveMsg::Admin {
                req: serde_json::to_value(op).unwrap_or_default(),
            }),
            _ => self.fail(crate::tr!(
                "Pas relié au serveur de l'espace.",
                "Not connected to the space's server."
            )),
        }
    }

    /// The administration window of a team space: members, invitations, the space's AIs.
    pub(super) fn team_admin_window(&mut self, ctx: &egui::Context) {
        use crate::team::{AdminOp, Role};
        if !self.admin_open {
            return;
        }
        let dark = ctx.global_style().visuals.dark_mode;
        let t = tone(dark);
        let key = egui::Id::unique("team-admin-draft");
        let mut d: AdminDraft = ctx.data(|x| x.get_temp(key)).unwrap_or_default();
        let view = self.team_admin.clone().unwrap_or_default();
        let mut ops: Vec<AdminOp> = vec![];
        let mut close = false;
        let role_label = |r: Role| r.label().to_string();
        let resp = egui::Modal::new(egui::Id::unique("team-admin")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading(crate::trf!("Administrer « {} »", "Manage « {} »", view.name));
            egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
                // The space.
                ui.add_space(6.0);
                ui.label(RichText::new(crate::tr!("Espace", "Space")).strong());
                let mut name = d.name.clone().unwrap_or_else(|| view.name.clone());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut name).desired_width(260.0));
                    if ui
                        .add_enabled(
                            !name.trim().is_empty() && name.trim() != view.name,
                            egui::Button::new(crate::tr!("Renommer", "Rename")),
                        )
                        .clicked()
                    {
                        ops.push(AdminOp::Rename {
                            name: name.trim().to_string(),
                        });
                    }
                });
                // Kept only once changed: until the server's answer, the name is unknown.
                d.name = (name != view.name).then_some(name);
                let mut rule = view.rule;
                ui.horizontal(|ui| {
                    ui.label(crate::tr!("IA autorisées :", "AIs allowed:"));
                    ui.radio_value(&mut rule, crate::space::AiRule::Free, crate::tr!("leurs IA", "their AIs"));
                    ui.radio_value(
                        &mut rule,
                        crate::space::AiRule::SpaceOnly,
                        crate::tr!("celles de l'espace", "the space's"),
                    );
                    ui.radio_value(&mut rule, crate::space::AiRule::NoAi, crate::tr!("aucune", "none"));
                });
                if rule != view.rule {
                    ops.push(AdminOp::Rule {
                        rule,
                        allowed: view.allowed.clone(),
                    });
                }

                // Members.
                ui.separator();
                ui.label(RichText::new(crate::tr!("Membres", "Members")).strong());
                egui::Grid::new("admin-members").striped(true).show(ui, |ui| {
                    for m in &view.members {
                        let mut label = m.name.clone();
                        if m.me {
                            label.push_str(crate::tr!(" (vous)", " (you)"));
                        }
                        ui.label(if m.revoked {
                            RichText::new(label).strikethrough().color(t.tertiary)
                        } else {
                            RichText::new(label)
                        });
                        ui.label(RichText::new(&m.key[..12.min(m.key.len())]).monospace().color(t.tertiary));
                        ui.add_enabled_ui(!m.me && !m.revoked, |ui| {
                            egui::ComboBox::from_id_salt(("role", &m.key))
                                .selected_text(role_label(m.role))
                                .show_ui(ui, |ui| {
                                    for r in [Role::Admin, Role::Member, Role::Reader] {
                                        if ui.selectable_label(m.role == r, role_label(r)).clicked() && r != m.role {
                                            ops.push(AdminOp::SetRole {
                                                key: m.key.clone(),
                                                role: r,
                                            });
                                        }
                                    }
                                });
                        });
                        if !m.me {
                            if m.revoked {
                                if ui.button(crate::tr!("Réintégrer", "Restore")).clicked() {
                                    ops.push(AdminOp::Restore { key: m.key.clone() });
                                }
                            } else if ui
                                .button(RichText::new(crate::tr!("Retirer", "Remove")).color(super::style::RED))
                                .clicked()
                            {
                                ops.push(AdminOp::Revoke { key: m.key.clone() });
                            }
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    let r = d.invite_role.unwrap_or(Role::Member);
                    egui::ComboBox::from_id_salt("invite-role")
                        .selected_text(role_label(r))
                        .show_ui(ui, |ui| {
                            for x in [Role::Member, Role::Reader, Role::Admin] {
                                if ui.selectable_label(r == x, role_label(x)).clicked() {
                                    d.invite_role = Some(x);
                                }
                            }
                        });
                    if ui
                        .button(crate::tr!("Créer une invitation (copiée)", "Create an invitation (copied)"))
                        .clicked()
                    {
                        ops.push(AdminOp::Invite { role: r });
                    }
                });

                // The space's AIs.
                ui.separator();
                ui.label(RichText::new(crate::tr!("IA de l'espace", "The space's AIs")).strong());
                ui.label(
                    RichText::new(crate::tr!(
                        "Les membres passent par ce serveur : ils ne voient jamais la clé, et un membre retiré est coupé aussitôt.",
                        "Members go through this server: they never see the key, and a removed member is cut off at once."
                    ))
                    .size(11.5)
                    .color(t.tertiary),
                );
                for a in &view.ais {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&a.name).strong());
                        ui.label(RichText::new(format!("{} · {} · {}", a.tool, a.base_url, if a.model.is_empty() { "—" } else { &a.model })).size(12.0).color(t.tertiary));
                        if ui
                            .small_button(RichText::new(crate::tr!("Supprimer", "Delete")).color(super::style::RED))
                            .clicked()
                        {
                            ops.push(AdminOp::RemoveAi { name: a.name.clone() });
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(crate::trf!("{} clé(s)", "{} key(s)", a.keys));
                        let k = d.keys.entry(a.name.clone()).or_default();
                        ui.add(
                            egui::TextEdit::singleline(k)
                                .password(true)
                                .hint_text(crate::tr!("clé de l'entreprise, pour tous", "company key, for everyone"))
                                .desired_width(200.0),
                        );
                        if ui
                            .add_enabled(!k.trim().is_empty(), egui::Button::new(crate::tr!("Enregistrer", "Save")))
                            .clicked()
                        {
                            ops.push(AdminOp::SetKey {
                                ai: a.name.clone(),
                                member: "*".into(),
                                key: k.trim().to_string(),
                            });
                            k.clear();
                        }
                        let q = d
                            .quotas
                            .entry(a.name.clone())
                            .or_insert_with(|| a.quota.to_string());
                        ui.label(crate::tr!("quota/jour :", "quota/day:"));
                        ui.add(egui::TextEdit::singleline(q).desired_width(50.0));
                        if let Ok(n) = q.trim().parse::<u64>()
                            && n != a.quota
                            && ui.small_button("✓").clicked()
                        {
                            ops.push(AdminOp::Quota {
                                ai: a.name.clone(),
                                per_day: n,
                            });
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.ai_name).hint_text(crate::tr!("nom", "name")).desired_width(100.0));
                    if d.ai_tool.is_empty() {
                        d.ai_tool = "claude-code".into();
                    }
                    egui::ComboBox::from_id_salt("ai-tool")
                        .selected_text(&d.ai_tool)
                        .show_ui(ui, |ui| {
                            for x in ["claude-code", "codex"] {
                                ui.selectable_value(&mut d.ai_tool, x.to_string(), x);
                            }
                        });
                    ui.add(egui::TextEdit::singleline(&mut d.ai_url).hint_text("https://…").desired_width(150.0));
                    ui.add(egui::TextEdit::singleline(&mut d.ai_model).hint_text(crate::tr!("modèle", "model")).desired_width(80.0));
                    if ui.button(crate::tr!("Ajouter", "Add")).clicked() {
                        ops.push(AdminOp::AddAi {
                            ai: crate::space::SpaceAi {
                                name: d.ai_name.trim().to_string(),
                                tool: d.ai_tool.clone(),
                                base_url: d.ai_url.trim().to_string(),
                                model: d.ai_model.trim().to_string(),
                                ..Default::default()
                            },
                        });
                        d.ai_name.clear();
                        d.ai_url.clear();
                        d.ai_model.clear();
                    }
                });
                if !view.today.is_empty() {
                    ui.label(RichText::new(crate::tr!("Appels aujourd'hui", "Calls today")).size(12.0).color(t.tertiary));
                    for (who, ai, n) in &view.today {
                        ui.label(format!("{who} · {ai} · {n}"));
                    }
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(crate::tr!(
                        "Les changements atteignent les membres à leur prochaine connexion (un membre retiré : dans les secondes).",
                        "Changes reach members at their next connection (a removed member: within seconds)."
                    ))
                    .size(11.5)
                    .color(t.tertiary),
                );
            });
            ui.horizontal(|ui| {
                if ui.button(crate::tr!("Actualiser", "Refresh")).clicked() {
                    ops.push(AdminOp::List);
                }
                if ui.button(crate::tr!("Fermer", "Close")).clicked() {
                    close = true;
                }
            });
        });
        if resp.should_close() {
            close = true;
        }
        ctx.data_mut(|x| x.insert_temp(key, d));
        for op in ops {
            self.send_admin(op);
        }
        if close {
            self.admin_open = false;
        }
    }
}
