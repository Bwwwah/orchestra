//! The window's frame, drawn after the reference picture of Notion: the sidebar running the
//! full height beside the window buttons, the tab strip above the page only, the page bar
//! ("Privée", "Partager", link, star, •••), the home and library views and the round AI button.

use super::icons::{self, Icon};
use super::*;

/// Sidebar width measured on the reference picture (408 of 2000 px at 2626 px / 2x).
pub const SIDEBAR_WIDTH: f32 = 268.0;
/// Room left for the macOS window buttons in the sidebar's first row.
pub const TRAFFIC_LIGHTS: f32 = 76.0;

/// Room left for the macOS window buttons: none in full screen, where they are hidden.
pub fn lights_room(ctx: &egui::Context) -> f32 {
    let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    if cfg!(target_os = "macos") && !full {
        TRAFFIC_LIGHTS
    } else {
        4.0
    }
}
/// Rows in the sidebar: 47 of 2000 px on the reference, about 31 px.
pub const SIDE_ROW: f32 = 30.0;

/// Orchestra's own violet, taken from the app icon: the sidebar's signature colour.
pub fn violet(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(167, 139, 250)
    } else {
        Color32::from_rgb(109, 74, 222)
    }
}

/// The sidebar's background: a faint violet tint rather than Notion's warm grey.
pub fn side_fill(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(27, 25, 35)
    } else {
        Color32::from_rgb(246, 244, 252)
    }
}

/// The sidebar's colours: the theme's, with violet-tinted hover and selection.
pub fn side_tone(dark: bool) -> Tone {
    let mut t = tone(dark);
    let v = violet(dark);
    let tint = |a: u8| Color32::from_rgba_unmultiplied(v.r(), v.g(), v.b(), a);
    t.hover = tint(if dark { 22 } else { 16 });
    t.selected = tint(if dark { 40 } else { 30 });
    t.fill = tint(if dark { 34 } else { 24 });
    t.border = tint(if dark { 48 } else { 40 });
    t
}

pub enum SideIcon<'a> {
    Emoji(&'a str),
    Line(Icon),
}

/// One sidebar row: icon, 14 px label, rounded hover tint; the open item is highlighted.
pub fn side_item(
    ui: &mut egui::Ui,
    icon: SideIcon,
    label: &str,
    selected: bool,
    indent: f32,
    t: &Tone,
) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, SIDE_ROW), egui::Sense::click());
    let painter = ui.painter().with_clip_rect(rect);
    let dark = ui.visuals().dark_mode;
    if selected {
        // The open item: violet tint and a short violet bar on its left edge.
        painter.rect_filled(rect, 8.0, t.selected);
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top() + 7.0),
            egui::pos2(rect.left() + 3.0, rect.bottom() - 7.0),
        );
        painter.rect_filled(bar, 1.5, violet(dark));
    } else if resp.hovered() {
        painter.rect_filled(rect, 8.0, t.hover);
    }
    let x = rect.left() + indent;
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(x + 18.0, rect.center().y),
        egui::vec2(20.0, 20.0),
    );
    let icon_color = if selected {
        violet(dark)
    } else {
        t.accent_text
    };
    match icon {
        SideIcon::Emoji(e) => {
            painter.text(
                icon_rect.center(),
                egui::Align2::CENTER_CENTER,
                e,
                FontId::proportional(16.0),
                icon_color,
            );
        }
        SideIcon::Line(i) => icons::paint(&painter, icon_rect, i, icon_color),
    }
    let color = if selected { t.text } else { t.accent_text };
    let max_w = (rect.right() - (x + 36.0) - 28.0).max(20.0);
    let font = if selected {
        semibold(14.0)
    } else {
        FontId::proportional(14.0)
    };
    let mut job = egui::text::LayoutJob::single_section(
        label.to_string(),
        egui::TextFormat::simple(font, color),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    painter.galley(
        egui::pos2(x + 36.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    resp
}

/// Small section title in spaced capitals ("PAGES PRIVÉES"…), followed by a hairline.
pub fn section_header(ui: &mut egui::Ui, label: &str, t: &Tone) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 28.0), egui::Sense::hover());
    let mut format = egui::TextFormat::simple(semibold(10.5), t.tertiary);
    format.extra_letter_spacing = 0.8;
    let job = egui::text::LayoutJob::single_section(label.to_uppercase(), format);
    let g = ui.fonts_mut(|f| f.layout_job(job));
    let y = rect.center().y + 2.0;
    let end = rect.left() + 8.0 + g.size().x;
    ui.painter().galley(
        egui::pos2(rect.left() + 8.0, y - g.size().y / 2.0),
        g,
        t.tertiary,
    );
    if end + 10.0 < rect.right() - 6.0 {
        ui.painter().hline(
            end + 8.0..=rect.right() - 6.0,
            y,
            egui::Stroke::new(1.0, t.border),
        );
    }
}

/// The "+ Ajouter" row under a section.
fn add_row(ui: &mut egui::Ui, t: &Tone) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, SIDE_ROW), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, t.hover);
    }
    let ir = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 18.0, rect.center().y),
        egui::vec2(18.0, 18.0),
    );
    icons::paint(ui.painter(), ir, Icon::Plus, t.tertiary);
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        crate::tr!("Ajouter", "Add"),
        FontId::proportional(14.0),
        t.tertiary,
    );
    resp
}

/// Empty parts of the title area move the window (its title bar is hidden on macOS).
pub fn window_drag_area(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let r = ui.interact(rect, id, egui::Sense::click_and_drag());
    if r.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if r.double_clicked() {
        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
    }
}

/// The name of a view that isn't a page, for the open tab's title.
fn view_label(v: View) -> Option<&'static str> {
    match v {
        View::Page => None,
        View::Home => Some(crate::tr!("Accueil", "Home")),
        View::Trash => Some(crate::tr!("Corbeille", "Trash")),
        View::Projects => Some(crate::tr!("Projets", "Projects")),
        View::Orchestra => Some(crate::tr!("Orchestration", "Orchestration")),
        View::Inbox => Some(crate::tr!("Boîte de réception", "Inbox")),
    }
}

pub(super) fn relative_time(ms: u64) -> String {
    let now = now_ms();
    let d = now.saturating_sub(ms) / 1000;
    match d {
        0..=59 => crate::tr!("à l'instant", "just now").into(),
        60..=3599 => crate::trf!("il y a {} min", "{} min ago", d / 60),
        3600..=86_399 => crate::trf!("il y a {} h", "{} h ago", d / 3600),
        _ => crate::trf!("il y a {} j", "{} d ago", d / 86_400),
    }
}

impl NotesApp {
    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let dark = ui.visuals().dark_mode;
        let t = side_tone(dark);
        ui.spacing_mut().item_spacing.y = 2.0;
        // First row, level with the tab strip: window buttons, sidebar toggle, new page.
        let w = ui.available_width();
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(w, original::TAB_BAR_HEIGHT),
            egui::Sense::hover(),
        );
        let lights = lights_room(ui.ctx());
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.add_space(lights);
        if icons::button(
            &mut child,
            Icon::Sidebar,
            26.0,
            t.secondary,
            t.hover,
            crate::tr!(
                "Masquer la barre latérale (Cmd/Ctrl+\\)",
                "Hide the sidebar (Cmd/Ctrl+\\)"
            ),
        )
        .clicked()
        {
            self.ws.settings.sidebar_hidden = true;
        }
        let drag_from = child.cursor().left();
        // The app's name, painted only (so the empty title area still moves the window).
        if row.right() - drag_from > 190.0 {
            let pos = egui::pos2(drag_from + 6.0, row.center().y);
            child.painter().circle_stroke(
                pos + egui::vec2(5.0, 0.0),
                4.5,
                egui::Stroke::new(2.0, violet(dark)),
            );
            child.painter().text(
                pos + egui::vec2(15.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "Orchestra",
                semibold(13.5),
                t.secondary,
            );
        }
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            if icons::button(
                ui,
                Icon::Compose,
                26.0,
                t.secondary,
                t.hover,
                crate::tr!("Nouvelle page (Cmd/Ctrl+N)", "New page (Cmd/Ctrl+N)"),
            )
            .clicked()
            {
                actions.push(Action::NewPage(None));
            }
            // The AI and collaboration shortcuts, next to it.
            let co = self.settings_tab == Some(SettingsTab::Collab);
            if icons::button(
                ui,
                Icon::People,
                26.0,
                if co { t.text } else { t.secondary },
                t.hover,
                crate::tr!(
                    "Collaboration : partager des projets en équipe",
                    "Collaboration: share projects with a team"
                ),
            )
            .clicked()
            {
                self.open_settings(SettingsTab::Collab);
            }
            let ai = self.settings_tab == Some(SettingsTab::Ai);
            if icons::button(
                ui,
                Icon::Sparkle,
                26.0,
                if ai { t.text } else { t.secondary },
                t.hover,
                crate::tr!(
                    "Brancher et régler les IA (MCP)",
                    "Plug in and set up the AIs (MCP)"
                ),
            )
            .clicked()
            {
                self.open_settings(SettingsTab::Ai);
            }
        });
        let drag = egui::Rect::from_min_max(
            egui::pos2(drag_from + 4.0, row.top()),
            egui::pos2(row.right() - 92.0, row.bottom()),
        );
        window_drag_area(ui, drag, ui.scope_id().with("side-drag"));

        // Search box with its shortcut badge.
        ui.add_space(6.0);
        {
            let w = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 32.0), egui::Sense::click());
            let p = ui.painter();
            p.rect_filled(
                rect,
                8.0,
                if resp.hovered() {
                    t.hover
                } else {
                    Color32::TRANSPARENT
                },
            );
            p.rect_stroke(
                rect,
                8.0,
                egui::Stroke::new(1.0, t.border),
                egui::StrokeKind::Inside,
            );
            p.text(
                rect.left_center() + egui::vec2(12.0, 0.0),
                egui::Align2::LEFT_CENTER,
                crate::tr!("Chercher ou demander", "Search or ask"),
                FontId::proportional(14.0),
                t.tertiary,
            );
            let key = if cfg!(target_os = "macos") {
                "⌘K"
            } else {
                "Ctrl+K"
            };
            let g = p.layout_no_wrap(key.into(), FontId::proportional(11.0), t.secondary);
            let badge = egui::Rect::from_center_size(
                rect.right_center() - egui::vec2(10.0 + g.size().x / 2.0 + 4.0, 0.0),
                g.size() + egui::vec2(8.0, 4.0),
            );
            p.rect_filled(badge, 4.0, t.fill);
            p.galley(badge.min + egui::vec2(4.0, 2.0), g, t.secondary);
            if resp.clicked() {
                self.search_open = true;
                self.search_focus = true;
            }
        }
        // "Accueil": a row like the others, highlighted only when open.
        ui.add_space(10.0);
        if side_item(
            ui,
            SideIcon::Line(Icon::Home),
            crate::tr!("Accueil", "Home"),
            self.view == View::Home,
            0.0,
            &t,
        )
        .clicked()
        {
            self.view = View::Home;
        }
        if side_item(
            ui,
            SideIcon::Line(Icon::TodoList),
            crate::tr!("Projets", "Projects"),
            self.view == View::Projects,
            0.0,
            &t,
        )
        .clicked()
        {
            self.view = View::Projects;
        }
        let n = self.orch.runs.len();
        let label = if n > 0 {
            crate::trf!(
                "Orchestration · {n} en cours",
                "Orchestration · {n} running"
            )
        } else {
            crate::tr!("Orchestration", "Orchestration").to_string()
        };
        if side_item(
            ui,
            SideIcon::Line(Icon::Sparkle),
            &label,
            self.view == View::Orchestra,
            0.0,
            &t,
        )
        .clicked()
        {
            self.view = View::Orchestra;
        }
        let unread = self.ws.unread();
        let label = if unread > 0 {
            crate::trf!("Boîte de réception ({unread})", "Inbox ({unread})")
        } else {
            crate::tr!("Boîte de réception", "Inbox").to_string()
        };
        if side_item(
            ui,
            SideIcon::Line(Icon::Download),
            &label,
            self.view == View::Inbox,
            0.0,
            &t,
        )
        .clicked()
        {
            self.view = View::Inbox;
        }
        if !self.terms.is_empty() {
            let label = crate::trf!("Terminaux ({})", "Terminals ({})", self.terms.len());
            if side_item(
                ui,
                SideIcon::Line(Icon::Code),
                &label,
                self.term_open,
                0.0,
                &t,
            )
            .on_hover_text(crate::tr!(
                "Afficher / masquer (Ctrl+`)",
                "Show / hide (Ctrl+`)"
            ))
            .clicked()
            {
                self.term_open = !self.term_open;
                self.term_focus = self.term_open;
            }
        }

        let show_card = !self.ws.settings.hide_connect_card
            && self.ws.settings.collab.connector == ConnectorKind::None;
        // The bottom block's real height from the last frame (estimated on the first), so the
        // page list never runs into the workspace switcher.
        let bottom_key = egui::Id::unique(("sidebar-bottom", show_card));
        let bottom_h = ui
            .data(|d| d.get_temp::<f32>(bottom_key))
            .unwrap_or(2.0 * (SIDE_ROW + 2.0) + 60.0 + if show_card { 86.0 } else { 0.0 });
        let max_h = (ui.available_height() - bottom_h - 8.0).max(60.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(max_h)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_space(14.0);
                let favs: Vec<(String, String, String)> = self
                    .ws
                    .pages
                    .iter()
                    .filter(|p| p.favorite && !p.trashed)
                    .map(|p| (p.id.clone(), page_icon(p).to_string(), p.display_title()))
                    .collect();
                if !favs.is_empty() {
                    section_header(ui, crate::tr!("Favoris", "Favorites"), &t);
                    for (id, icon, title) in favs {
                        let sel =
                            self.current.as_deref() == Some(id.as_str()) && self.view == View::Page;
                        if side_item(ui, SideIcon::Emoji(&icon), &title, sel, 0.0, &t).clicked() {
                            actions.push(Action::Open(id));
                        }
                    }
                    ui.add_space(14.0);
                }
                // Shared projects first, then everything that stays on this computer.
                let is_shared = |p: &Page| p.project.as_ref().is_some_and(|x| x.shared);
                let shared_roots: Vec<String> = self
                    .ws
                    .pages
                    .iter()
                    .filter(|p| !p.trashed && is_shared(p))
                    .map(|p| p.id.clone())
                    .collect();
                if !shared_roots.is_empty() {
                    section_header(ui, crate::tr!("Projets partagés", "Shared projects"), &t);
                    let mut rows = vec![];
                    for id in shared_roots {
                        let r = self.tree_row(ui, &id, 0, actions, &t);
                        rows.push((id, r));
                    }
                    self.reorder_roots(ui, &rows);
                    ui.add_space(10.0);
                }
                // Projects apart from the notes: they are what the AIs work on, each with a
                // dot when something happens there (an AI at work, a question waiting).
                let own_projects: Vec<String> = self
                    .ws
                    .children(None)
                    .iter()
                    .filter(|p| !is_shared(p) && p.project.is_some())
                    .map(|p| p.id.clone())
                    .collect();
                let projects_top = ui.cursor().min;
                section_header(ui, crate::tr!("Projets", "Projects"), &t);
                let mut rows = vec![];
                for id in own_projects {
                    let r = self.tree_row(ui, &id, 0, actions, &t);
                    rows.push((id, r));
                }
                self.reorder_roots(ui, &rows);
                let add = add_row(ui, &t);
                super::tutorial::mark(
                    ui,
                    "side:projects",
                    egui::Rect::from_min_max(projects_top, add.rect.right_bottom()),
                );
                if add
                    .on_hover_text(crate::tr!("Nouveau projet", "New project"))
                    .clicked()
                {
                    self.view = View::Projects;
                    if self.new_project.is_none() {
                        self.new_project = Some(Default::default());
                    }
                }
                ui.add_space(10.0);
                section_header(ui, crate::tr!("Pages", "Pages"), &t);
                // « Rapports des IA » stays last, and can't be dragged nor dropped around.
                let reports = self.ws.settings.orch.reports_page.clone();
                let (pinned, roots): (Vec<String>, Vec<String>) = self
                    .ws
                    .children(None)
                    .iter()
                    .filter(|p| !is_shared(p) && p.project.is_none())
                    .map(|p| p.id.clone())
                    .partition(|id| !reports.is_empty() && *id == reports);
                let mut rows = vec![];
                for id in roots {
                    let r = self.tree_row(ui, &id, 0, actions, &t);
                    rows.push((id, r));
                }
                for id in pinned {
                    self.tree_row(ui, &id, 0, actions, &t);
                }
                self.reorder_roots(ui, &rows);
                if add_row(ui, &t).clicked() {
                    actions.push(Action::NewPage(None));
                }
            });

        // Bottom: trash, settings, the collaboration card, the workspace switcher.
        let bottom = ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(10.0);
            let name = self.space_name();
            ui.horizontal(|ui| {
                let w = ui.available_width() - 40.0;
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(w, 34.0), egui::Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, t.hover);
                }
                let sq = egui::Rect::from_center_size(
                    egui::pos2(rect.left() + 18.0, rect.center().y),
                    egui::vec2(22.0, 22.0),
                );
                ui.painter().rect_filled(sq, 4.0, t.fill);
                let initial: String = name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().collect())
                    .unwrap_or_default();
                ui.painter().text(
                    sq.center(),
                    egui::Align2::CENTER_CENTER,
                    initial,
                    semibold(13.0),
                    t.secondary,
                );
                let mut job = egui::text::LayoutJob::single_section(
                    name.clone(),
                    egui::TextFormat::simple(semibold(14.0), t.text),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(w - 70.0);
                let g = ui.fonts_mut(|f| f.layout_job(job));
                let gw = g.size().x;
                ui.painter().galley(
                    egui::pos2(rect.left() + 36.0, rect.center().y - g.size().y / 2.0),
                    g,
                    t.text,
                );
                icons::paint(
                    ui.painter(),
                    egui::Rect::from_center_size(
                        egui::pos2(rect.left() + 46.0 + gw, rect.center().y),
                        egui::vec2(12.0, 12.0),
                    ),
                    Icon::ChevronDown,
                    t.secondary,
                );
                // Stays open on a click inside: the new space's name is typed in it.
                egui::Popup::menu(&resp)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| self.space_menu(ui));
                if icons::button(
                    ui,
                    Icon::Help,
                    30.0,
                    t.secondary,
                    t.hover,
                    crate::tr!("Aide et raccourcis", "Help and shortcuts"),
                )
                .clicked()
                {
                    self.open_settings(SettingsTab::Shortcuts);
                }
            });
            if show_card {
                ui.add_space(8.0);
                let w = ui.available_width();
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(w, 72.0), egui::Sense::click());
                let p = ui.painter();
                p.rect_filled(
                    rect,
                    10.0,
                    if resp.hovered() {
                        t.hover
                    } else {
                        Color32::TRANSPARENT
                    },
                );
                p.rect_stroke(
                    rect,
                    10.0,
                    egui::Stroke::new(1.0, t.border),
                    egui::StrokeKind::Inside,
                );
                icons::paint(
                    p,
                    egui::Rect::from_center_size(
                        rect.left_top() + egui::vec2(24.0, 24.0),
                        egui::vec2(18.0, 18.0),
                    ),
                    Icon::People,
                    t.text,
                );
                p.text(
                    rect.left_top() + egui::vec2(44.0, 24.0),
                    egui::Align2::LEFT_CENTER,
                    crate::tr!("Brancher la collaboration", "Set up collaboration"),
                    semibold(13.0),
                    t.text,
                );
                p.text(
                    rect.left_top() + egui::vec2(44.0, 46.0),
                    egui::Align2::LEFT_CENTER,
                    crate::tr!("Travaillez en équipe.", "Work as a team."),
                    FontId::proportional(12.0),
                    t.secondary,
                );
                let close = egui::Rect::from_center_size(
                    rect.right_top() + egui::vec2(-20.0, 20.0),
                    egui::vec2(20.0, 20.0),
                );
                let cr = ui.interact(
                    close,
                    ui.scope_id().with("card-close"),
                    egui::Sense::click(),
                );
                icons::paint(
                    ui.painter(),
                    close.shrink(4.0),
                    Icon::Close,
                    if cr.hovered() { t.text } else { t.secondary },
                );
                if cr.clicked() {
                    self.ws.settings.hide_connect_card = true;
                } else if resp.clicked() {
                    self.open_settings(SettingsTab::Collab);
                }
                ui.add_space(4.0);
            }
            // All the settings, from their first page (« Général »); customising the sidebar
            // is one of their tabs.
            if side_item(
                ui,
                SideIcon::Line(Icon::Gear),
                crate::tr!("Paramètres", "Settings"),
                self.settings_tab.is_some(),
                0.0,
                &t,
            )
            .clicked()
            {
                self.open_settings(SettingsTab::General);
            }
            if side_item(
                ui,
                SideIcon::Line(Icon::Trash),
                crate::tr!("Corbeille", "Trash"),
                self.view == View::Trash,
                0.0,
                &t,
            )
            .clicked()
            {
                self.view = View::Trash;
            }
            self.usage_widget(ui, &t);
            self.update_card(ui, &t);
        });
        let h = bottom.response.rect.height();
        ui.data_mut(|d| d.insert_temp(bottom_key, h));
    }

    /// Top-level pages of one sidebar section, put in order by dragging them: a line shows
    /// where the page will land, it moves there on release. `rows`: each page's row, in order.
    fn reorder_roots(&mut self, ui: &mut egui::Ui, rows: &[(String, Option<egui::Response>)]) {
        let key = egui::Id::unique("sidebar-drag-page");
        let rows: Vec<(&String, &egui::Response)> = rows
            .iter()
            .filter_map(|(id, r)| r.as_ref().map(|r| (id, r)))
            .collect();
        if let Some((id, _)) = rows.iter().find(|(_, r)| r.drag_started()) {
            ui.data_mut(|d| d.insert_temp(key, (*id).clone()));
        }
        let Some(dragged) = ui.data(|d| d.get_temp::<String>(key)) else {
            return;
        };
        // The drag belongs to another section.
        if !rows.iter().any(|(id, _)| **id == dragged) {
            return;
        }
        let (Some(pos), Some((_, first)), Some((last_id, last))) =
            (ui.ctx().pointer_latest_pos(), rows.first(), rows.last())
        else {
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        // Before the first row whose middle is below the pointer, else after the last one.
        let (target, after, y) = match rows.iter().find(|(_, r)| pos.y < r.rect.center().y) {
            Some((id, r)) => ((*id).clone(), false, r.rect.top()),
            None => ((*last_id).clone(), true, last.rect.bottom()),
        };
        ui.painter().hline(
            first.rect.left() + 6.0..=first.rect.right() - 6.0,
            y,
            egui::Stroke::new(2.0, super::projects::BLUE),
        );
        if ui.input(|i| i.pointer.any_released() || !i.pointer.primary_down()) {
            if self.ws.move_page_near(&dragged, &target, after) {
                self.dirty_since.get_or_insert_with(Instant::now);
            }
            ui.data_mut(|d| d.remove::<String>(key));
        }
        ui.ctx().request_repaint();
    }

    /// One page of the tree (and its open sub-pages); returns its own row.
    fn tree_row(
        &self,
        ui: &mut egui::Ui,
        id: &str,
        depth: usize,
        actions: &mut Vec<Action>,
        t: &Tone,
    ) -> Option<egui::Response> {
        let p = self.ws.page(id)?;
        let kids: Vec<String> = self
            .ws
            .children(Some(id))
            .iter()
            .map(|c| c.id.clone())
            .collect();
        // In a project in "lecture seule", nothing can be added.
        let locked = self.ws.is_locked(id);
        // A page with nothing inside has no arrow: there would be nothing to show.
        let foldable = !kids.is_empty();
        let open = self.expanded.contains(id) && foldable;
        let selected = self.current.as_deref() == Some(id) && self.view == View::Page;
        let indent = depth as f32 * 12.0;
        let icon = page_icon(p).to_string();
        // Top-level pages can be dragged to another place: a drag-only area under the row
        // (clicks, the arrow, "+" and the menu stay the row's; the drag falls through to it).
        let below =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), SIDE_ROW));
        let pinned = id == self.ws.settings.orch.reports_page;
        let drag = (depth == 0 && !pinned).then(|| {
            ui.interact(
                below,
                egui::Id::unique(("sidebar-reorder", id)),
                egui::Sense::drag(),
            )
        });
        let row = side_item(
            ui,
            SideIcon::Emoji(&icon),
            &p.display_title(),
            selected,
            indent,
            t,
        );
        let rect = row.rect;
        // A project's live state, on the right of its row (where « + » shows on hover).
        if depth == 0 && !ui.rect_contains_pointer(rect)
            && let Some((color, pulse, tip)) = self.project_signal(id)
        {
            let c = egui::pos2(rect.right() - 16.0, rect.center().y);
            if pulse {
                super::projects::pulse_dot(ui, c, color);
            } else {
                ui.painter().circle_filled(c, 3.5, color);
            }
            let dot = egui::Rect::from_center_size(c, egui::vec2(14.0, 14.0));
            ui.interact(dot, row.id.with("signal"), egui::Sense::hover())
                .on_hover_text(tip);
        }
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 18.0 + indent, rect.center().y),
            egui::vec2(22.0, 22.0),
        );
        let plus_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 16.0, rect.center().y),
            egui::vec2(22.0, 22.0),
        );
        let arrow = ui.interact(icon_rect, row.id.with("arrow"), egui::Sense::click());
        let plus = ui.interact(plus_rect, row.id.with("plus"), egui::Sense::click());
        if ui.rect_contains_pointer(rect) {
            // On hover the icon becomes the expand arrow, and "+" appears at the right.
            let painter = ui.painter();
            let cover = icon_rect.shrink(1.0);
            painter.rect_filled(cover, 4.0, side_fill(ui.visuals().dark_mode));
            painter.rect_filled(cover, 4.0, if selected { t.selected } else { t.hover });
            if arrow.hovered() {
                painter.rect_filled(cover, 4.0, t.hover);
            }
            if !(selected || row.hovered()) {
                painter.rect_filled(rect, 6.0, t.hover);
            }
            if foldable {
                icons::paint(
                    painter,
                    icon_rect.shrink(4.0),
                    if open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    t.secondary,
                );
            } else {
                painter.text(
                    icon_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &icon,
                    FontId::proportional(14.0),
                    t.text,
                );
            }
            if !locked {
                if plus.hovered() {
                    painter.rect_filled(plus_rect.shrink(1.0), 4.0, t.hover);
                }
                icons::paint(painter, plus_rect.shrink(4.0), Icon::Plus, t.secondary);
            }
        }
        if arrow.clicked() && foldable {
            actions.push(Action::ToggleExpand(id.to_string()));
        } else if !locked
            && plus
                .on_hover_text(crate::tr!(
                    "Ajouter une page à l'intérieur",
                    "Add a page inside"
                ))
                .clicked()
        {
            actions.push(Action::NewPage(Some(id.to_string())));
        } else if row.clicked() || (!foldable && arrow.clicked()) {
            actions.push(Action::Open(id.to_string()));
        }
        if locked {
            row.context_menu(|ui| {
                ui.label(
                    RichText::new(crate::tr!("Projet en lecture seule", "Read-only project"))
                        .color(t.tertiary),
                );
            });
        } else {
            row.context_menu(|ui| {
                if ui
                    .button(crate::tr!("Ajouter une sous-page", "Add a subpage"))
                    .clicked()
                {
                    actions.push(Action::NewPage(Some(id.to_string())));
                    ui.close();
                }
                if ui
                    .button(crate::tr!("Ajouter une base de données", "Add a database"))
                    .clicked()
                {
                    actions.push(Action::NewDatabase(Some(id.to_string())));
                    ui.close();
                }
                if ui.button(crate::tr!("Dupliquer", "Duplicate")).clicked() {
                    actions.push(Action::Duplicate(id.to_string()));
                    ui.close();
                }
                if ui
                    .button(crate::tr!("Mettre à la corbeille", "Move to trash"))
                    .clicked()
                {
                    actions.push(Action::Trash(id.to_string()));
                    ui.close();
                }
            });
        }
        if open {
            // README and Documentation sit under one « Docs » item, where the first one was.
            let docs = crate::docs::docs_group(&self.ws, id);
            for k in &kids {
                if !docs.contains(k) {
                    self.tree_row(ui, k, depth + 1, actions, t);
                } else if docs.first() == Some(k) {
                    self.docs_row(ui, id, &docs, depth + 1, actions, t);
                }
            }
        }
        drag.map(|d| d.with_new_rect(rect))
    }

    /// A project's « Docs » item, folded until asked for: not a page, a click unfolds it.
    fn docs_row(
        &self,
        ui: &mut egui::Ui,
        project: &str,
        pages: &[String],
        depth: usize,
        actions: &mut Vec<Action>,
        t: &Tone,
    ) {
        let key = crate::docs::docs_group_key(project);
        let open = self.expanded.contains(&key);
        let next =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), SIDE_ROW));
        // On hover the icon becomes the expand arrow, as on the other rows.
        let icon = if !ui.rect_contains_pointer(next) {
            SideIcon::Emoji("📚")
        } else if open {
            SideIcon::Line(Icon::ChevronDown)
        } else {
            SideIcon::Line(Icon::ChevronRight)
        };
        if side_item(ui, icon, "Docs", false, depth as f32 * 12.0, t).clicked() {
            actions.push(Action::ToggleExpand(key));
        }
        if open {
            for page in pages {
                self.tree_row(ui, page, depth + 1, actions, t);
            }
        }
    }

    /// What a project's dot in the sidebar says: an AI at work there (green, pulsing), else
    /// a question of the AIs waiting for the person (orange). None: nothing to say.
    fn project_signal(&self, id: &str) -> Option<(Color32, bool, &'static str)> {
        if self.orch.runs.iter().any(|r| r.project == id) {
            return Some((
                super::style::GREEN,
                true,
                crate::tr!("Une IA travaille sur ce projet", "An AI is working on this project"),
            ));
        }
        if self
            .ws
            .inbox
            .iter()
            .any(|m| m.project == id && m.waits_for_person())
        {
            return Some((
                super::style::ORANGE,
                false,
                crate::tr!("Une question attend votre réponse", "A question is waiting for your answer"),
            ));
        }
        None
    }

    /// The tab strip: back/forward, Firefox-like tabs (the open one is a rounded card joining the page),
    /// "+", and the rest of the strip moves the window.
    pub(super) fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let t = tone(ui.visuals().dark_mode);
        let full = ui.max_rect();
        let mut select = None;
        let mut close = None;
        let mut add = false;
        let mut end_x = full.left();
        let mut active_span: Option<egui::Rangef> = None;
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if self.ws.settings.sidebar_hidden {
                ui.add_space(lights_room(ui.ctx()));
                if icons::button(
                    ui,
                    Icon::Sidebar,
                    26.0,
                    t.secondary,
                    t.hover,
                    crate::tr!(
                        "Afficher la barre latérale (Cmd/Ctrl+\\)",
                        "Show the sidebar (Cmd/Ctrl+\\)"
                    ),
                )
                .clicked()
                {
                    self.ws.settings.sidebar_hidden = false;
                }
            }
            ui.add_space(6.0);
            let can_back = self
                .tabs
                .get(self.active_tab)
                .map(|t| !t.back.is_empty())
                .unwrap_or(false);
            let can_fwd = self
                .tabs
                .get(self.active_tab)
                .map(|t| !t.fwd.is_empty())
                .unwrap_or(false);
            if icons::button(
                ui,
                Icon::ChevronLeft,
                26.0,
                if can_back { t.secondary } else { t.border },
                t.hover,
                crate::tr!("Retour (Cmd/Ctrl+[)", "Back (Cmd/Ctrl+[)"),
            )
            .clicked()
            {
                self.go_back();
            }
            if icons::button(
                ui,
                Icon::ChevronRight,
                26.0,
                if can_fwd { t.secondary } else { t.border },
                t.hover,
                crate::tr!("Suivant (Cmd/Ctrl+])", "Forward (Cmd/Ctrl+])"),
            )
            .clicked()
            {
                self.go_forward();
            }
            ui.add_space(8.0);
            let n = self.tabs.len().max(1) as f32;
            let avail = (ui.available_width() - 60.0).max(40.0);
            let w = (avail / n).clamp(original::TAB_MIN_WIDTH, original::TAB_MAX_WIDTH);
            // Firefox-like tabs: the open one is a card with rounded top corners that
            // joins the page; the others stay flat, with short separators between them.
            let mut spans: Vec<(egui::Rect, bool)> = Vec::new();
            for i in 0..self.tabs.len() {
                let page = if i == self.active_tab {
                    self.current.clone()
                } else {
                    self.tabs[i].page.clone()
                };
                // The open tab shows the view on screen (Accueil, Projets…) rather than the
                // page it last held: the title always says what is shown.
                let view_title = if i == self.active_tab {
                    view_label(self.view)
                } else {
                    None
                };
                let (icon, title) = match view_title {
                    Some(v) => ("".to_string(), v.to_string()),
                    None => page
                        .and_then(|id| {
                            self.ws
                                .page(&id)
                                .map(|p| (page_icon(p).to_string(), p.display_title()))
                        })
                        .unwrap_or_else(|| {
                            ("".into(), crate::tr!("Nouvel onglet", "New tab").into())
                        }),
                };
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(w, full.height()), egui::Sense::click());
                let active = i == self.active_tab;
                // The close button covers part of the tab: the tab itself is no longer
                // "hovered" under it, so test the pointer against the tab's rect.
                let over = ui.rect_contains_pointer(rect);
                let card = egui::Rect::from_min_max(
                    egui::pos2(rect.left(), full.top() + 6.0),
                    egui::pos2(rect.right(), full.bottom()),
                );
                // Text sits in the middle of the card; quiet tabs centre it above the gap left at the bottom.
                let mid_y = if active {
                    card.center().y
                } else {
                    card.center().y - 2.5
                };
                if active {
                    ui.painter().rect_filled(
                        card,
                        egui::CornerRadius {
                            nw: 8,
                            ne: 8,
                            sw: 0,
                            se: 0,
                        },
                        t.border,
                    );
                    let inner = egui::Rect::from_min_max(
                        card.min + egui::vec2(1.0, 1.0),
                        egui::pos2(card.right() - 1.0, card.bottom() + 1.0),
                    );
                    ui.painter().rect_filled(
                        inner,
                        egui::CornerRadius {
                            nw: 7,
                            ne: 7,
                            sw: 0,
                            se: 0,
                        },
                        ui.visuals().panel_fill,
                    );
                    active_span = Some(card.x_range());
                } else if over {
                    let pill = egui::Rect::from_min_max(
                        card.min + egui::vec2(2.0, 0.0),
                        egui::pos2(card.right() - 2.0, card.bottom() - 5.0),
                    );
                    ui.painter().rect_filled(pill, 7.0, t.hover);
                }
                spans.push((card, active || over));
                let color = if active { t.text } else { t.secondary };
                let label = if icon.is_empty() {
                    title.clone()
                } else {
                    format!("{icon}  {title}")
                };
                let mut job = egui::text::LayoutJob::single_section(
                    label,
                    egui::TextFormat::simple(FontId::proportional(14.0), color),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(w - 40.0);
                let g = ui.fonts_mut(|f| f.layout_job(job));
                ui.painter().galley(
                    egui::pos2(rect.left() + 12.0, mid_y - g.size().y / 2.0),
                    g,
                    color,
                );
                if over && (self.tabs.len() > 1 || self.current.is_some()) {
                    let cr = egui::Rect::from_center_size(
                        egui::pos2(rect.right() - 16.0, mid_y),
                        egui::vec2(18.0, 18.0),
                    );
                    let c = ui.interact(
                        cr,
                        ui.scope_id().with(("tab-close", i)),
                        egui::Sense::click(),
                    );
                    if c.hovered() {
                        ui.painter().rect_filled(cr, 4.0, t.hover);
                    }
                    icons::paint(ui.painter(), cr.shrink(3.0), Icon::Close, t.secondary);
                    if c.clicked() {
                        close = Some(i);
                    }
                }
                if resp.middle_clicked() {
                    close = Some(i);
                } else if resp.clicked() && close.is_none() {
                    select = Some(i);
                }
                resp.on_hover_text(title);
            }
            // Separators only between two quiet tabs, as in Firefox.
            for pair in spans.windows(2) {
                let ((a, a_lit), (_, b_lit)) = (pair[0], pair[1]);
                if !a_lit && !b_lit {
                    let c = a.center().y - 2.5;
                    ui.painter().vline(
                        a.right() + 1.0,
                        egui::Rangef::new(c - 8.0, c + 8.0),
                        egui::Stroke::new(1.0, t.border),
                    );
                }
            }
            ui.add_space(6.0);
            if icons::button(
                ui,
                Icon::Plus,
                28.0,
                t.secondary,
                t.hover,
                crate::tr!("Nouvel onglet (Cmd/Ctrl+T)", "New tab (Cmd/Ctrl+T)"),
            )
            .clicked()
            {
                add = true;
            }
            end_x = ui.cursor().left();
        });
        // Bottom line, open under the active tab so it joins the page.
        let y = full.bottom() - 0.5;
        let stroke = egui::Stroke::new(1.0, t.border);
        match active_span {
            Some(span) => {
                ui.painter()
                    .hline(egui::Rangef::new(full.left(), span.min - 1.0), y, stroke);
                ui.painter()
                    .hline(egui::Rangef::new(span.max + 1.0, full.right()), y, stroke);
            }
            None => {
                ui.painter().hline(full.x_range(), y, stroke);
            }
        }
        let drag = egui::Rect::from_min_max(
            egui::pos2(end_x + 4.0, full.top()),
            egui::pos2(full.right(), full.bottom() - 1.0),
        );
        window_drag_area(ui, drag, ui.scope_id().with("tab-drag"));
        if let Some(i) = select {
            self.select_tab(i);
        }
        if let Some(i) = close {
            self.close_tab(i);
        }
        if add {
            self.new_tab();
        }
    }

    /// Bar above the page: breadcrumb, "Privée" / "Partagée", then "Partager", link, star, •••.
    pub(super) fn page_top_bar(
        &mut self,
        ui: &mut egui::Ui,
        cur: &str,
        path: &[String],
        move_choices: &[(String, String)],
        actions: &mut Vec<Action>,
    ) {
        let t = tone(ui.visuals().dark_mode);
        // Only shared projects leave this computer: everything else is private.
        let in_project = self
            .ws
            .project_of(cur)
            .map(|p| p.project.as_ref().is_some_and(|x| x.shared));
        let shared =
            in_project == Some(true) && self.ws.settings.collab.connector != ConnectorKind::None;
        let live_on = self.live.is_some() && in_project == Some(true);
        let here = self.peers_on(cur);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.set_height(32.0);
            ui.add_space(12.0);
            for (i, pid) in path.iter().enumerate() {
                if i > 0 {
                    ui.label(RichText::new("/").size(14.0).color(t.tertiary));
                }
                if let Some(p) = self.ws.page(pid) {
                    let label = format!("{}  {}", page_icon(p), p.display_title());
                    if ui.add(egui::Button::new(RichText::new(label).size(14.0).color(t.text)).frame_when_inactive(false)).clicked() {
                        actions.push(Action::Open(pid.clone()));
                    }
                }
            }
            ui.add_space(4.0);
            // « Partagée » or « En direct » when the page leaves this computer; a private page
            // says nothing here (« Partager », on the right, opens the same menu).
            if shared || live_on {
            let label = if live_on { crate::tr!("En direct", "Live") } else { crate::tr!("Partagée", "Shared") };
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(if live_on { 108.0 } else { 104.0 }, 26.0), egui::Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 6.0, t.hover);
            }
            icons::paint(ui.painter(), egui::Rect::from_center_size(rect.left_center() + egui::vec2(12.0, 0.0), egui::vec2(15.0, 15.0)), if shared { Icon::People } else { Icon::Lock }, t.tertiary);
            ui.painter().text(rect.left_center() + egui::vec2(24.0, 0.0), egui::Align2::LEFT_CENTER, label, FontId::proportional(14.0), if live_on { super::style::GREEN } else { t.tertiary });
            icons::paint(ui.painter(), egui::Rect::from_center_size(rect.right_center() - egui::vec2(10.0, 0.0), egui::vec2(11.0, 11.0)), Icon::ChevronDown, t.tertiary);
            let resp = resp.on_hover_text(crate::tr!("Qui voit cette page", "Who sees this page"));
            egui::Popup::menu(&resp).show(|ui| self.share_menu(ui, cur, actions));
            }
            // Why nothing can be edited here; unticked in the project's « … » menu.
            if self.ws.is_locked(cur) {
                ui.add_space(6.0);
                ui.label(RichText::new(crate::tr!("Lecture seule", "Read-only")).size(14.0).color(super::projects::ORANGE))
                    .on_hover_text(crate::tr!("Se décoche dans le menu « … » de la page du projet", "Untick it in the “…” menu of the project's page"));
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_space(8.0);
                let more = icons::button(ui, Icon::More, 28.0, t.text, t.hover, "");
                // Gone meanwhile (deleted from another window or by an AI): nothing to show.
                let Some(page) = self.ws.page_mut(cur) else { return };
                let words: usize = page.blocks.iter().map(|b| b.text.split_whitespace().count()).sum();
                let mut reformatted = None;
                let mut share = None;
                let mut local_copy = false;
                egui::Popup::menu(&more).show(|ui| {
                    ui.set_min_width(230.0);
                    if let Some(pr) = page.project.as_mut() {
                        super::projects::project_menu(ui, pr, &mut share, &mut local_copy);
                        ui.separator();
                    }
                    ui.checkbox(&mut page.full_width, crate::tr!("Pleine largeur", "Full width"));
                    if ui.button(crate::tr!("Dupliquer", "Duplicate")).clicked() {
                        actions.push(Action::Duplicate(cur.to_string()));
                        ui.close();
                    }
                    ui.menu_button(crate::tr!("Déplacer vers…", "Move to…"), |ui| {
                        if ui.button(crate::tr!("Niveau principal", "Top level")).clicked() {
                            page.parent = None;
                            ui.close();
                        }
                        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                            for (id, title) in move_choices {
                                if ui.button(title).clicked() {
                                    page.parent = Some(id.clone());
                                    ui.close();
                                }
                            }
                        });
                    });
                    if ui.button(crate::tr!("Versions de la page…", "Page versions…")).on_hover_text(crate::tr!("Les versions gardées sur cet ordinateur, à remettre en place", "The versions kept on this computer, to put back")).clicked() {
                        actions.push(Action::PageVersions(cur.to_string()));
                        ui.close();
                    }
                    if ui.button(crate::tr!("Exporter en Markdown", "Export as Markdown")).clicked() {
                        actions.push(Action::Export(cur.to_string()));
                        ui.close();
                    }
                    if ui.button(crate::tr!("Remettre en forme le texte des IA", "Reformat the AIs' text")).on_hover_text(crate::tr!("Tableaux, listes et titres écrits en Markdown brut redeviennent des blocs propres", "Tables, lists and headings written in raw Markdown become clean blocks again")).clicked() {
                        reformatted = Some(crate::store::reformat_page(page));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(crate::tr!("Mettre à la corbeille", "Move to trash")).clicked() {
                        actions.push(Action::Trash(cur.to_string()));
                        ui.close();
                    }
                    ui.separator();
                    ui.label(RichText::new(crate::trf!("{words} mots · modifiée {}", "{words} words · edited {}", relative_time(page.updated))).small().color(t.tertiary));
                });
                let fav = page.favorite;
                if icons::button(ui, if fav { Icon::StarFilled } else { Icon::Star }, 28.0, if fav { Color32::from_rgb(232, 179, 52) } else { t.text }, t.hover, crate::tr!("Ajouter aux favoris", "Add to favorites")).clicked() {
                    page.favorite = !fav;
                }
                if let Some(n) = reformatted {
                    self.ack(crate::trf!("{n} bloc{s} remis en forme.", "{n} block{s} reformatted.", s = crate::i18n::plural(n)));
                }
                if let Some(v) = share {
                    self.set_project_shared(cur, v);
                }
                if local_copy {
                    actions.push(Action::LocalCopy(cur.to_string()));
                }
                if icons::button(ui, Icon::Link, 28.0, t.text, t.hover, crate::tr!("Copier le lien (Cmd/Ctrl+L)", "Copy link (Cmd/Ctrl+L)")).clicked() {
                    ui.ctx().copy_text(format!("orchestra://page/{cur}"));
                    self.ack(crate::tr!("Lien copié.", "Link copied."));
                }
                let g = ui.painter().layout_no_wrap(crate::tr!("Partager", "Share").into(), FontId::proportional(14.0), t.text);
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(g.size().x + 36.0, 28.0), egui::Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, t.hover);
                }
                icons::paint(ui.painter(), egui::Rect::from_center_size(rect.left_center() + egui::vec2(14.0, 0.0), egui::vec2(16.0, 16.0)), Icon::Lock, t.text);
                ui.painter().galley(egui::pos2(rect.left() + 26.0, rect.center().y - g.size().y / 2.0), g, t.text);
                let resp = resp.on_hover_text(crate::tr!("Partager le projet avec l'équipe", "Share the project with the team"));
                egui::Popup::menu(&resp).show(|ui| self.share_menu(ui, cur, actions));
                // The project's side panel (team discussion, AI conversation).
                if in_project.is_some() {
                    let on = self.side_open;
                    if icons::button(ui, Icon::People, 28.0, if on { super::style::BLUE } else { t.text }, t.hover, crate::tr!("Discussion et conversation IA du projet", "Project discussion and AI conversation")).clicked() {
                        self.side_open = !on;
                    }
                }
                // Faces of the people looking at this page right now.
                if !here.is_empty() {
                    ui.add_space(6.0);
                    live_ui::faces(ui, &here, ui.visuals().panel_fill);
                }
            });
        });
    }

    /// Shares the project with the team, or makes it private again.
    fn set_project_shared(&mut self, pid: &str, shared: bool) {
        let Some(page) = self.ws.page_mut(pid) else {
            return;
        };
        if let Some(x) = page.project.as_mut() {
            x.shared = shared;
        }
        page.updated = crate::model::now_ms();
        let title = page.display_title();
        self.dirty_since.get_or_insert_with(std::time::Instant::now);
        if shared {
            self.send_everything();
            self.ack(crate::trf!(
                "« {title} » est partagé.",
                "« {title} » is shared."
            ));
        } else {
            self.ack(crate::trf!(
                "« {title} » est redevenu privé.",
                "« {title} » is private again."
            ));
        }
    }

    /// The "Partager" menu: share or unshare the page's project, and the state of the connection.
    fn share_menu(&mut self, ui: &mut egui::Ui, cur: &str, actions: &mut Vec<Action>) {
        let t = tone(ui.visuals().dark_mode);
        ui.set_min_width(300.0);
        let project = self.ws.project_of(cur).map(|p| {
            (
                p.id.clone(),
                p.display_title(),
                p.project.as_ref().is_some_and(|x| x.shared),
            )
        });
        let Some((pid, title, shared)) = project else {
            ui.label(
                RichText::new(crate::tr!("Cette page est privée", "This page is private")).strong(),
            );
            ui.label(RichText::new(crate::tr!("Seuls les projets partagés quittent cette machine. Rangez la page dans un projet, puis partagez le projet.", "Only shared projects leave this computer. Put the page in a project, then share the project.")).size(12.5).color(t.tertiary));
            return;
        };
        ui.label(RichText::new(crate::trf!("Partager « {title} »", "Share « {title} »")).strong());
        ui.add_space(2.0);
        let mut set: Option<bool> = None;
        if shared {
            ui.label(
                RichText::new(crate::tr!(
                    "Partagé : le projet, ses pages et sa discussion partent chez l'équipe.",
                    "Shared: the project, its pages and its discussion go to the team."
                ))
                .size(12.5)
                .color(super::style::GREEN),
            );
            ui.horizontal(|ui| {
                if ui
                    .button(crate::tr!("Rendre privé", "Make private"))
                    .on_hover_text(crate::tr!(
                        "Le projet ne part plus chez les autres ; ils gardent leur copie.",
                        "The project no longer goes to the others; they keep their copy."
                    ))
                    .clicked()
                {
                    set = Some(false);
                    ui.close();
                }
                if ui
                    .button(crate::tr!("Garder une copie locale", "Keep a local copy"))
                    .clicked()
                {
                    actions.push(Action::LocalCopy(pid.clone()));
                    ui.close();
                }
            });
        } else {
            ui.label(
                RichText::new(crate::tr!(
                    "Privé : rien ne quitte cette machine.",
                    "Private: nothing leaves this computer."
                ))
                .size(12.5)
                .color(t.tertiary),
            );
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(crate::tr!("Partager avec l'équipe", "Share with the team"))
                            .color(Color32::WHITE),
                    )
                    .fill(super::style::BLUE),
                )
                .clicked()
            {
                set = Some(true);
                ui.close();
            }
        }
        if let Some(v) = set {
            self.set_project_shared(&pid, v);
        }
        ui.separator();
        // How it leaves this computer.
        let c = &self.ws.settings.collab;
        let line = match c.connector {
            ConnectorKind::None => crate::tr!(
                "Aucune collaboration branchée : le partage attend un branchement.",
                "No collaboration set up: sharing waits for a connection."
            )
            .to_string(),
            ConnectorKind::Live => match &self.live {
                None => crate::tr!("En direct, arrêté.", "Live, stopped.").to_string(),
                Some(l) => {
                    let s = l.status();
                    let n = self.peers.len();
                    let who = if n == 0 {
                        crate::tr!("personne de connecté", "nobody connected").to_string()
                    } else {
                        if crate::i18n::en() {
                            format!("{n} connected")
                        } else {
                            format!("{n} connecté{}", crate::i18n::plural(n))
                        }
                    };
                    if s.hosting {
                        crate::trf!(
                            "En direct, chez vous ({}) · {who}",
                            "Live, hosted here ({}) · {who}",
                            s.address
                        )
                    } else if s.connected {
                        crate::trf!(
                            "En direct, relié à {} · {who}",
                            "Live, connected to {} · {who}",
                            c.live_address
                        )
                    } else {
                        crate::tr!("En direct : connexion en cours…", "Live: connecting…")
                            .to_string()
                    }
                }
            },
        };
        ui.label(RichText::new(line).size(12.5).color(t.secondary));
        let invite = match (&self.live, c.connector == ConnectorKind::Live) {
            (Some(l), true) if l.status().hosting => Some(crate::trf!(
                "Adresse : {}\nCode : {}",
                "Address: {}\nCode: {}",
                l.status().address,
                c.live_code
            )),
            _ => None,
        };
        let live_off = c.connector == ConnectorKind::Live && self.live.is_none();
        ui.horizontal(|ui| {
            if let Some(inv) = invite
                && ui
                    .button(crate::tr!("Copier l'invitation", "Copy the invitation"))
                    .on_hover_text(crate::tr!(
                        "Adresse et code à donner aux autres",
                        "Address and code to give to the others"
                    ))
                    .clicked()
            {
                ui.ctx().copy_text(inv);
                self.ack(crate::tr!(
                    "Adresse et code copiés.",
                    "Address and code copied."
                ));
                ui.close();
            }
            if live_off && ui.button(crate::tr!("Démarrer", "Start")).clicked() {
                self.start_live();
                ui.close();
            }
            if ui.button(crate::tr!("Réglages…", "Settings…")).clicked() {
                self.open_settings(SettingsTab::Collab);
                ui.close();
            }
        });
    }

    /// Home: what Orchestra is, where things stand now (questions, sessions, AIs,
    /// projects), the four steps to get going (ticked as they are done), then the recently
    /// edited pages as cards.
    pub(super) fn home_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let t = tone(ui.visuals().dark_mode);
        let mut pages: Vec<&Page> = self.ws.pages.iter().filter(|p| !p.trashed).collect();
        pages.sort_by_key(|p| std::cmp::Reverse(p.updated));
        let running: Vec<String> = self.orch.runs.iter().map(|r| r.project.clone()).collect();
        let projects: Vec<HomeProject> = self
            .ws
            .projects()
            .into_iter()
            .filter_map(|p| {
                let pr = p.project.as_ref()?;
                let (done, total) = pr.progress();
                Some(HomeProject {
                    id: p.id.clone(),
                    title: p.display_title(),
                    icon: page_icon(p).to_string(),
                    done,
                    total,
                    running: running.contains(&p.id),
                    last: pr.last_activity.max(p.updated),
                })
            })
            .collect();
        let now = HomeNow {
            plugged: self.ais.plugged().len(),
            projects: self.ws.projects().len(),
            running: self.orch.runs.len(),
            ran: !self.ws.runs.is_empty() || !self.orch.runs.is_empty(),
            waiting: self
                .ws
                .inbox
                .iter()
                .filter(|m| m.waits_for_person())
                .count(),
            checking: self.ais.states.lock_or_recover().is_none(),
        };
        let mut go: Option<HomeGo> = None;
        let avail = ui.available_width();
        let width = 708.0f32.min(avail - 40.0).max(300.0);
        ui.add_space(56.0);
        ui.horizontal(|ui| {
            ui.add_space(((avail - width) / 2.0).max(20.0));
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.label(RichText::new(crate::tr!("Bonjour !", "Hello!")).font(bold(super::style::TITLE)).color(t.text));
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(RichText::new(crate::tr!(
                        "Vos projets, les IA qui travaillent dessus, et ce qui attend votre réponse.",
                        "Your projects, the AIs working on them, and what is waiting for your answer."
                    )).size(14.5).color(t.secondary))
                    .wrap(),
                );
                ui.add_space(18.0);
                // Where things stand, each chip opens its page.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                    for (text, color, target) in home_status(&now) {
                        let c = match color {
                            HomeTone::Alert => super::projects::ORANGE,
                            HomeTone::Live => super::projects::GREEN,
                            HomeTone::Calm => t.secondary,
                        };
                        if home_chip(ui, &text, c, color == HomeTone::Live, &t).clicked() {
                            go = Some(target);
                        }
                    }
                });
                // The four steps, while one is left to do.
                let done = home_steps(&now);
                if done.iter().any(|d| !d) {
                    ui.add_space(28.0);
                    let n_done = done.iter().filter(|d| **d).count();
                    super::projects::card(ui, &t, 0.0, |ui| {
                    super::projects::card_title(ui, crate::tr!("POUR COMMENCER", "TO GET GOING"), &format!("{n_done} / 4"), &t);
                    home_bar(ui, "home-start-bar", n_done as f32 / 4.0, super::projects::GREEN, &t);
                    ui.add_space(12.0);
                    let steps = [
                        (crate::tr!("Brancher une IA", "Plug in an AI"), crate::tr!("Paramètres › IA : installez et branchez Claude Code, Codex, Gemini CLI ou Mistral Vibe.", "Settings › AI: install and plug in Claude Code, Codex, Gemini CLI or Mistral Vibe."), HomeGo::Ais),
                        (crate::tr!("Créer un projet", "Create a project"), crate::tr!("Donnez son dossier de code ; l'IA remplit l'état des lieux et propose des étapes.", "Give its code folder; the AI fills in the state of play and proposes steps."), HomeGo::Projects),
                        (crate::tr!("Lancer une IA en autonomie", "Start an AI on its own"), crate::tr!("« Lancer une IA » sur la page du projet : elle avance étape par étape dans une copie à part.", "« Start an AI » on the project's page: it goes step by step in a separate copy."), HomeGo::Orchestra),
                        (crate::tr!("Répondre à ses questions", "Answer its questions"), crate::tr!("Boîte de réception : votre réponse lui parvient au prochain état des lieux.", "Inbox: your answer reaches it at the next state of play."), HomeGo::Inbox),
                    ];
                    for (i, ((title, what, target), ok)) in steps.into_iter().zip(done).enumerate() {
                        if home_step(ui, i + 1, title, what, ok, &t).clicked() {
                            go = Some(target);
                        }
                    }
                    });
                }
                // The projects, most recently active first, with their progress.
                if !projects.is_empty() {
                    ui.add_space(20.0);
                    let extra = if projects.len() > HOME_PROJECTS { crate::trf!("{} au total", "{} in all", projects.len()) } else { String::new() };
                    super::projects::card(ui, &t, 0.0, |ui| {
                        super::projects::card_title(ui, crate::tr!("PROJETS", "PROJECTS"), &extra, &t);
                        for (i, p) in projects.iter().take(HOME_PROJECTS).enumerate() {
                            if home_project(ui, i, p, &t).clicked() {
                                actions.push(Action::Open(p.id.clone()));
                            }
                        }
                        if projects.len() > HOME_PROJECTS {
                            super::projects::card_rule(ui, &t);
                            if super::projects::quiet_link(ui, crate::tr!("Tous les projets →", "All projects →"), &t).clicked() {
                                go = Some(HomeGo::Projects);
                            }
                        }
                    });
                }
                if !pages.is_empty() {
                    ui.add_space(30.0);
                    super::projects::card_title(ui, crate::tr!("PAGES RÉCENTES", "RECENT PAGES"), "", &t);
                    ui.add_space(4.0);
                    // As many tiles per row as fit, stretched to fill the column evenly.
                    let gap = 14.0;
                    let per_row = ((width + gap) / (150.0 + gap)).floor().max(1.0);
                    let tile = ((width - gap * (per_row - 1.0)) / per_row).floor();
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                        for p in pages.iter().take(12) {
                            if home_tile(ui, p, tile, &t).clicked() {
                                actions.push(Action::Open(p.id.clone()));
                            }
                        }
                    });
                }
                ui.add_space(48.0);
            });
        });
        match go {
            Some(HomeGo::Ais) => self.open_settings(SettingsTab::Ai),
            Some(HomeGo::Projects) => self.view = View::Projects,
            Some(HomeGo::Orchestra) => self.view = View::Orchestra,
            Some(HomeGo::Inbox) => self.view = View::Inbox,
            None => {}
        }
    }

    /// The round button at the bottom right that opens the AI (MCP) panel.
    pub(super) fn ai_button(&mut self, ctx: &egui::Context) {
        let dark = ctx.global_style().visuals.dark_mode;
        let t = tone(dark);
        egui::Area::new(egui::Id::unique("ai-fab"))
            // Kept clear of the side panel and the terminal dock.
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(-22.0 - self.side_w, -22.0 - self.dock_h),
            )
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(46.0, 46.0), egui::Sense::click());
                let p = ui.painter();
                p.add(egui::Shape::Rect(egui::epaint::RectShape::filled(
                    rect.translate(egui::vec2(0.0, 3.0)),
                    23.0,
                    Color32::from_black_alpha(if dark { 90 } else { 18 }),
                )));
                p.circle_filled(
                    rect.center(),
                    23.0,
                    if dark { t.card } else { Color32::WHITE },
                );
                p.circle_stroke(rect.center(), 23.0, egui::Stroke::new(1.0, t.border));
                if resp.hovered() {
                    p.circle_filled(rect.center(), 23.0, t.hover);
                }
                icons::paint(p, rect.shrink(12.0), Icon::Sparkle, t.text);
                if resp
                    .on_hover_text(crate::tr!("IA par MCP", "AI through MCP"))
                    .clicked()
                {
                    self.open_settings(SettingsTab::Ai);
                }
            });
    }
}

/// Where things stand, for the home page.
struct HomeNow {
    /// AIs plugged into Orchestra.
    plugged: usize,
    projects: usize,
    /// Autonomous sessions running now.
    running: usize,
    /// An autonomous session ran once (or runs).
    ran: bool,
    /// AI questions waiting for an answer.
    waiting: usize,
    /// The AIs are still being checked (startup).
    checking: bool,
}

/// Where a click on the home page leads.
#[derive(Clone, Copy, Debug, PartialEq)]
enum HomeGo {
    Ais,
    Projects,
    Orchestra,
    Inbox,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum HomeTone {
    /// Waits for the person.
    Alert,
    /// At work.
    Live,
    Calm,
}

/// The home page's status chips: questions waiting, sessions running, AIs plugged in,
/// projects. What waits for the person comes first.
fn home_status(n: &HomeNow) -> Vec<(String, HomeTone, HomeGo)> {
    let mut out = vec![];
    let pl = crate::i18n::plural;
    if n.waiting > 0 {
        out.push((
            crate::trf!(
                "{} question{s} à répondre",
                "{} question{s} to answer",
                n.waiting,
                s = pl(n.waiting)
            ),
            HomeTone::Alert,
            HomeGo::Inbox,
        ));
    }
    if n.running > 0 {
        // « IA » doesn't change in the plural; only the English text takes the mark.
        let text = if crate::i18n::en() {
            format!("{} AI{} at work", n.running, pl(n.running))
        } else {
            format!("{} IA au travail", n.running)
        };
        out.push((text, HomeTone::Live, HomeGo::Orchestra));
    }
    if !n.checking {
        if n.plugged == 0 {
            out.push((
                crate::tr!("Aucune IA branchée", "No AI plugged in").to_string(),
                HomeTone::Alert,
                HomeGo::Ais,
            ));
        } else {
            out.push((
                crate::trf!(
                    "{} IA branchée{s}",
                    "{} AI{s} plugged in",
                    n.plugged,
                    s = pl(n.plugged)
                ),
                HomeTone::Calm,
                HomeGo::Ais,
            ));
        }
    }
    out.push((
        crate::trf!(
            "{} projet{s}",
            "{} project{s}",
            n.projects,
            s = pl(n.projects)
        ),
        HomeTone::Calm,
        HomeGo::Projects,
    ));
    out
}

/// The four steps to get going, done or not: an AI plugged in, a project, an autonomous
/// session, no question left unanswered (only once a session ran: before, nothing asked).
fn home_steps(n: &HomeNow) -> [bool; 4] {
    [
        n.plugged > 0,
        n.projects > 0,
        n.ran,
        n.ran && n.waiting == 0,
    ]
}

/// Projects listed on the home page; the others are a click away.
const HOME_PROJECTS: usize = 5;

/// A project as the home page shows it.
struct HomeProject {
    id: String,
    title: String,
    icon: String,
    done: usize,
    total: usize,
    /// An autonomous session works on it now.
    running: bool,
    last: u64,
}

/// What a project row says on the right: « 3 / 10 étapes », or « aucune étape ».
fn home_progress_text(done: usize, total: usize) -> String {
    if total == 0 {
        crate::tr!("aucune étape", "no steps").to_string()
    } else {
        crate::trf!(
            "{done} / {total} étape{s}",
            "{done} / {total} step{s}",
            s = crate::i18n::plural(total)
        )
    }
}

/// A status chip of the home page: a coloured dot (breathing while at work), then its text.
fn home_chip(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    live: bool,
    t: &Tone,
) -> egui::Response {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(13.0), t.text);
    let (rect, resp) =
        ui.allocate_exact_size(g.size() + egui::vec2(40.0, 14.0), egui::Sense::click());
    let a = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.14);
    ui.painter()
        .rect_filled(rect, rect.height() / 2.0, t.card.lerp_to_gamma(t.fill, a));
    ui.painter().rect_stroke(
        rect,
        rect.height() / 2.0,
        egui::Stroke::new(1.0, t.border.lerp_to_gamma(color, 0.35 + 0.3 * a)),
        egui::StrokeKind::Inside,
    );
    let dot = egui::pos2(rect.left() + 15.0, rect.center().y);
    if live {
        super::projects::pulse_dot(ui, dot, color);
    } else {
        ui.painter().circle_filled(dot, 4.0, color);
    }
    ui.painter().galley(
        egui::pos2(rect.left() + 27.0, rect.center().y - g.size().y / 2.0),
        g,
        t.text,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A thin rounded progress bar across the available width.
fn home_bar(ui: &mut egui::Ui, id: &str, frac: f32, color: Color32, t: &Tone) {
    let (r, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 4.0), egui::Sense::hover());
    home_bar_in(ui, r, id, frac, color, t);
}

fn home_bar_in(
    ui: &egui::Ui,
    r: egui::Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    frac: f32,
    color: Color32,
    t: &Tone,
) {
    let f = super::projects::smooth(ui, id, frac.clamp(0.0, 1.0));
    ui.painter()
        .rect_filled(r, r.height() / 2.0, t.border.gamma_multiply(0.8));
    if f > 0.0 {
        let fill = egui::Rect::from_min_size(
            r.min,
            egui::vec2((r.width() * f).max(r.height()), r.height()),
        );
        ui.painter().rect_filled(fill, r.height() / 2.0, color);
    }
}

/// A project row: icon, name (and « au travail » while a session runs), when it last
/// moved, its steps done and a small progress bar.
fn home_project(ui: &mut egui::Ui, i: usize, p: &HomeProject, t: &Tone) -> egui::Response {
    let bg = super::projects::hover_begin(ui);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::click());
    let painter = ui.painter();
    let y = rect.center().y;
    painter.text(
        egui::pos2(rect.left() + 10.0, y),
        egui::Align2::CENTER_CENTER,
        &p.icon,
        FontId::proportional(16.0),
        t.text,
    );
    // Right side, from the edge: bar, steps, time.
    let bar = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 72.0, y - 2.0),
        egui::vec2(72.0, 4.0),
    );
    let color = if p.total > 0 && p.done == p.total {
        super::projects::GREEN
    } else {
        super::projects::BLUE
    };
    home_bar_in(
        ui,
        bar,
        ("home-project-bar", i),
        if p.total == 0 {
            0.0
        } else {
            p.done as f32 / p.total as f32
        },
        color,
        t,
    );
    let steps = painter.layout_no_wrap(
        home_progress_text(p.done, p.total),
        FontId::proportional(12.5),
        t.secondary,
    );
    let steps_x = bar.left() - 12.0 - steps.size().x;
    let when = painter.layout_no_wrap(
        relative_time(p.last),
        FontId::proportional(12.0),
        t.tertiary,
    );
    let when_x = steps_x - 16.0 - when.size().x;
    // The name, with what is left before the time.
    let mut right_end = when_x - 12.0;
    if p.running {
        let g = painter.layout_no_wrap(
            crate::tr!("au travail", "at work").to_string(),
            FontId::proportional(12.0),
            super::projects::GREEN,
        );
        right_end -= g.size().x + 18.0;
        super::projects::pulse_dot(ui, egui::pos2(right_end + 10.0, y), super::projects::GREEN);
        ui.painter().galley(
            egui::pos2(right_end + 18.0, y - g.size().y / 2.0),
            g,
            super::projects::GREEN,
        );
    }
    let mut job = egui::text::LayoutJob::single_section(
        p.title.clone(),
        egui::TextFormat::simple(semibold(14.0), t.text),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: (right_end - rect.left() - 30.0).max(40.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let name = ui.fonts_mut(|f| f.layout_job(job));
    let painter = ui.painter();
    painter.galley(
        egui::pos2(rect.left() + 30.0, y - name.size().y / 2.0),
        name,
        t.text,
    );
    painter.galley(
        egui::pos2(when_x, y - when.size().y / 2.0),
        when,
        t.tertiary,
    );
    painter.galley(
        egui::pos2(steps_x, y - steps.size().y / 2.0),
        steps,
        t.secondary,
    );
    super::projects::hover_end(
        ui,
        bg,
        rect.shrink2(egui::vec2(0.0, 1.0)),
        egui::Id::unique(("home-project", i)),
        t,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A recently edited page: a card with its icon over a tinted band, its title and when
/// it was edited; it lifts a little under the pointer.
fn home_tile(ui: &mut egui::Ui, p: &Page, side: f32, t: &Tone) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(side, 132.0), egui::Sense::click());
    let a = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.16);
    let painter = ui.painter();
    let shadow = egui::epaint::Shadow {
        offset: [0, (2.0 + 4.0 * a) as i8],
        blur: (6.0 + 10.0 * a) as u8,
        spread: 0,
        color: Color32::from_black_alpha((16.0 + 28.0 * a) as u8),
    };
    painter.add(shadow.as_shape(rect, 12.0));
    painter.rect_filled(rect, 12.0, t.card);
    let band = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 40.0));
    painter.rect_filled(
        band,
        egui::CornerRadius {
            nw: 12,
            ne: 12,
            sw: 0,
            se: 0,
        },
        t.fill,
    );
    painter.rect_stroke(
        rect,
        12.0,
        egui::Stroke::new(1.0, t.border.lerp_to_gamma(t.tertiary, 0.6 * a)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        egui::pos2(rect.left() + 28.0, band.bottom()),
        egui::Align2::CENTER_CENTER,
        page_icon(p),
        FontId::proportional(24.0),
        t.text,
    );
    let mut job = egui::text::LayoutJob::single_section(
        p.display_title(),
        egui::TextFormat::simple(semibold(14.0), t.text),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: rect.width() - 28.0,
        max_rows: 2,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let g = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().galley(
        egui::pos2(rect.left() + 14.0, band.bottom() + 20.0),
        g,
        t.text,
    );
    ui.painter().text(
        rect.left_bottom() + egui::vec2(14.0, -15.0),
        egui::Align2::LEFT_CENTER,
        relative_time(p.updated),
        FontId::proportional(12.0),
        t.tertiary,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One of the steps to get going: its number (a tick once done), title and what to do.
fn home_step(
    ui: &mut egui::Ui,
    n: usize,
    title: &str,
    what: &str,
    done: bool,
    t: &Tone,
) -> egui::Response {
    let bg = super::projects::hover_begin(ui);
    let row = ui.horizontal_top(|ui| {
        let (b, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
        let col = if done {
            super::projects::GREEN
        } else {
            t.secondary
        };
        ui.painter()
            .circle_filled(b.center(), 12.0, col.gamma_multiply(0.15));
        let mark = if done {
            "✓".to_string()
        } else {
            n.to_string()
        };
        ui.painter().text(
            b.center(),
            egui::Align2::CENTER_CENTER,
            mark,
            semibold(13.0),
            col,
        );
        ui.vertical(|ui| {
            let title_color = if done { t.tertiary } else { t.text };
            ui.label(RichText::new(title).font(semibold(14.0)).color(title_color));
            ui.add(egui::Label::new(RichText::new(what).size(13.0).color(t.tertiary)).wrap());
        });
    });
    let rect = row.response.rect;
    super::projects::hover_end(
        ui,
        bg,
        rect.expand2(egui::vec2(6.0, 3.0)),
        egui::Id::unique(("home-step", n)),
        t,
    );
    ui.add_space(6.0);
    ui.interact(
        rect,
        egui::Id::unique(("home-step-click", n)),
        egui::Sense::click(),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod home_tests {
    use super::*;

    fn now(plugged: usize, projects: usize, running: usize, ran: bool, waiting: usize) -> HomeNow {
        HomeNow {
            plugged,
            projects,
            running,
            ran,
            waiting,
            checking: false,
        }
    }

    /// A fresh install: nothing done, « no AI » flagged; then each step ticks as it is done.
    #[test]
    fn home_steps_tick_as_things_are_done() {
        assert_eq!(home_steps(&now(0, 0, 0, false, 0)), [false; 4]);
        assert_eq!(
            home_steps(&now(1, 2, 0, false, 0)),
            [true, true, false, false]
        );
        assert_eq!(
            home_steps(&now(1, 2, 0, true, 1)),
            [true, true, true, false]
        );
        assert_eq!(home_steps(&now(1, 2, 1, true, 0)), [true; 4]);
    }

    /// What waits for the person comes first; « no AI » only once the check is over.
    #[test]
    fn home_status_puts_what_waits_first() {
        let s = home_status(&now(0, 3, 2, true, 1));
        let go: Vec<HomeGo> = s.iter().map(|x| x.2).collect();
        assert_eq!(
            go,
            [
                HomeGo::Inbox,
                HomeGo::Orchestra,
                HomeGo::Ais,
                HomeGo::Projects
            ]
        );
        assert_eq!(s[2].1, HomeTone::Alert);
        let checking = HomeNow {
            checking: true,
            ..now(0, 0, 0, false, 0)
        };
        assert_eq!(
            home_status(&checking)
                .iter()
                .map(|x| x.2)
                .collect::<Vec<_>>(),
            [HomeGo::Projects]
        );
    }
}
