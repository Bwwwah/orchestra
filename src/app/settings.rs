//! Settings as a large window over the dimmed app, like the reference picture: a search box
//! and grouped sections on the left; a big title, a subtitle and rows of
//! "setting / description / control" on the right.
//! Colours sampled on the reference (dark): window #191919, left column #202020, open item
//! #2C2C2C, 1 px border #383836, drop-down list border #30302E, switch off #4E4F4F, 44 % backdrop.

use super::icons::{self, Icon};
use super::*;

/// The attributions and licences of the software shipped with Orchestra, shown in Settings ›
/// General: the same files the packages put next to the program.
const THIRD_PARTY_TEXTS: &[&str] = &[
    include_str!("../../packaging/licenses/LICENCES-TIERCES.txt"),
    include_str!("../../packaging/licenses/rtk-LICENSE.txt"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsTab {
    Preferences,
    Shortcuts,
    General,
    Sidebar,
    ImportExport,
    Ai,
    Collab,
}

impl SettingsTab {
    fn label(self) -> &'static str {
        match self {
            SettingsTab::Preferences => crate::tr!("Préférences", "Preferences"),
            SettingsTab::Shortcuts => crate::tr!("Raccourcis clavier", "Keyboard shortcuts"),
            SettingsTab::General => crate::tr!("Général", "General"),
            SettingsTab::Sidebar => crate::tr!("Barre latérale", "Sidebar"),
            SettingsTab::ImportExport => crate::tr!("Importer et exporter", "Import and export"),
            SettingsTab::Ai => crate::tr!("IA (MCP)", "AI (MCP)"),
            SettingsTab::Collab => crate::tr!("Collaboration", "Collaboration"),
        }
    }
    fn icon(self) -> Icon {
        match self {
            SettingsTab::Preferences => Icon::Sliders,
            SettingsTab::Shortcuts => Icon::Help,
            SettingsTab::General => Icon::Gear,
            SettingsTab::Sidebar => Icon::Sidebar,
            SettingsTab::ImportExport => Icon::Download,
            SettingsTab::Ai => Icon::Sparkle,
            SettingsTab::Collab => Icon::People,
        }
    }
}

fn groups() -> [(&'static str, &'static [SettingsTab]); 3] {
    [
        (
            crate::tr!("Application", "Application"),
            &[SettingsTab::Preferences, SettingsTab::Shortcuts],
        ),
        (
            crate::tr!("Espace de travail", "Workspace"),
            &[
                SettingsTab::General,
                SettingsTab::Sidebar,
                SettingsTab::ImportExport,
            ],
        ),
        (
            crate::tr!("Fonctionnalités", "Features"),
            &[SettingsTab::Ai, SettingsTab::Collab],
        ),
    ]
}

/// Colours of the settings window for the current theme.
pub(super) struct Look {
    pub(super) window: Color32,
    nav: Color32,
    nav_selected: Color32,
    pub(super) border: Color32,
    control_border: Color32,
    field: Color32,
    switch_off: Color32,
}

pub(super) fn look(dark: bool) -> Look {
    if dark {
        Look {
            window: Color32::from_rgb(25, 25, 25),
            nav: Color32::from_rgb(32, 32, 32),
            nav_selected: Color32::from_rgb(44, 44, 44),
            border: Color32::from_rgb(56, 56, 54),
            control_border: Color32::from_rgb(48, 48, 46),
            field: Color32::from_rgb(39, 39, 39),
            switch_off: Color32::from_rgb(78, 79, 79),
        }
    } else {
        // Light values estimated from the design tokens.
        Look {
            window: Color32::WHITE,
            nav: Color32::from_rgb(249, 248, 247),
            nav_selected: Color32::from_rgb(239, 238, 236),
            border: Color32::from_rgb(230, 229, 227),
            control_border: Color32::from_rgb(223, 222, 219),
            field: Color32::from_rgb(247, 246, 245),
            switch_off: Color32::from_rgb(214, 213, 210),
        }
    }
}

// ---------------------------------------------------------------- widgets

/// Title and subtitle at the top of a settings page.
fn page_title(ui: &mut egui::Ui, title: &str, subtitle: &str, t: &Tone) {
    ui.add_space(40.0);
    ui.label(RichText::new(title).font(bold(26.0)).color(t.text));
    ui.add_space(6.0);
    ui.label(RichText::new(subtitle).size(15.0).color(t.secondary));
    ui.add_space(14.0);
}

/// A section title with its line under it.
pub(super) fn section(ui: &mut egui::Ui, title: &str, t: &Tone, l: &Look) {
    ui.add_space(32.0);
    ui.label(RichText::new(title).font(semibold(16.0)).color(t.text));
    ui.add_space(10.0);
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
    ui.painter()
        .hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, l.border));
    ui.add_space(12.0);
}

/// One setting: name and description on the left, its control on the right.
/// On a wide window the control sits at the right and the text wraps in the room left of it;
/// on a narrow one (half a screen) the control goes under the text.
pub(super) fn row(
    ui: &mut egui::Ui,
    name: &str,
    desc: &str,
    t: &Tone,
    control: impl FnOnce(&mut egui::Ui),
) {
    let w = ui.available_width();
    let text = |ui: &mut egui::Ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(name).font(semibold(14.0)).color(t.text)).wrap());
        if !desc.is_empty() {
            ui.add(egui::Label::new(RichText::new(desc).size(13.0).color(t.secondary)).wrap());
        }
        ui.add_space(8.0);
    };
    if w >= 460.0 {
        ui.allocate_ui_with_layout(
            egui::vec2(w, 52.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                control(ui);
                ui.add_space(16.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 52.0),
                    egui::Layout::top_down(egui::Align::Min),
                    text,
                );
            },
        );
    } else {
        ui.allocate_ui_with_layout(
            egui::vec2(w, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            text,
        );
        ui.horizontal(control);
        ui.add_space(8.0);
    }
}

/// Drop-down list: the current choice with a chevron; a menu of choices. Returns the new index.
fn dropdown(
    ui: &mut egui::Ui,
    id: &str,
    choices: &[&str],
    current: usize,
    t: &Tone,
    l: &Look,
) -> Option<usize> {
    let text = choices.get(current).copied().unwrap_or("");
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(14.0), t.text);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(g.size().x + 38.0, 30.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 6.0, t.hover);
    }
    p.rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0, l.control_border),
        egui::StrokeKind::Inside,
    );
    p.galley(
        egui::pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0),
        g,
        t.text,
    );
    icons::paint(
        p,
        egui::Rect::from_center_size(
            rect.right_center() - egui::vec2(15.0, 0.0),
            egui::vec2(12.0, 12.0),
        ),
        Icon::ChevronDown,
        t.secondary,
    );
    let mut picked = None;
    egui::Popup::menu(&resp)
        .id(egui::Id::unique(("settings-dd", id)))
        .show(|ui| {
            ui.set_min_width(rect.width().max(200.0));
            for (i, c) in choices.iter().enumerate() {
                if menu_item(ui, None, c, i == current, t).clicked() {
                    picked = Some(i);
                    ui.close();
                }
            }
        });
    picked
}

/// A menu row like Notion's: optional icon, label, a tick when chosen, a rounded hover.
pub(super) fn menu_item(
    ui: &mut egui::Ui,
    icon: Option<Icon>,
    label: &str,
    checked: bool,
    t: &Tone,
) -> egui::Response {
    let w = ui.available_width().max(160.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 6.0, t.selected);
    }
    let mut x = rect.left() + 10.0;
    if let Some(i) = icon {
        icons::paint(
            p,
            egui::Rect::from_center_size(
                egui::pos2(x + 9.0, rect.center().y),
                egui::vec2(18.0, 18.0),
            ),
            i,
            t.text,
        );
        x += 28.0;
    }
    p.text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        t.text,
    );
    if checked {
        let c = egui::pos2(rect.right() - 16.0, rect.center().y);
        let s = egui::Stroke::new(1.6, t.text);
        p.line_segment([c + egui::vec2(-5.0, 0.0), c + egui::vec2(-1.5, 3.5)], s);
        p.line_segment([c + egui::vec2(-1.5, 3.5), c + egui::vec2(5.0, -4.0)], s);
    }
    resp
}

/// On/off switch: grey when off, blue when on, white knob.
pub(super) fn switch(ui: &mut egui::Ui, on: &mut bool, l: &Look) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(34.0, 20.0), egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let k = ui.ctx().animate_bool(resp.id, *on);
    let track = if *on {
        super::style::BLUE
    } else {
        l.switch_off
    };
    let track = if k > 0.0 && k < 1.0 {
        lerp_color(l.switch_off, super::style::BLUE, k)
    } else {
        track
    };
    ui.painter().rect_filled(rect, 10.0, track);
    let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, k);
    ui.painter()
        .circle_filled(egui::pos2(x, rect.center().y), 8.0, Color32::WHITE);
    resp
}

fn lerp_color(a: Color32, b: Color32, k: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Text field with Notion's soft fill and border.
fn field<'a>(
    text: &'a mut String,
    hint: &str,
    width: f32,
    t: &Tone,
    l: &Look,
) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(text)
        .hint_text(RichText::new(hint.to_string()).color(t.tertiary))
        .font(FontId::proportional(14.0))
        .text_color(t.text)
        .desired_width(width)
        .frame(
            egui::Frame::new()
                .fill(l.field)
                .stroke(egui::Stroke::new(1.0, l.control_border))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(10, 6)),
        )
}

/// Flat button with a border.
pub(super) fn button(ui: &mut egui::Ui, text: &str, t: &Tone, l: &Look) -> egui::Response {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(14.0), t.text);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(g.size().x + 24.0, 30.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, t.hover);
    }
    ui.painter().rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0, l.control_border),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        egui::pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0),
        g,
        t.text,
    );
    resp
}

/// A key combination drawn as small key caps.
fn keys(ui: &mut egui::Ui, combo: &str, t: &Tone, l: &Look) {
    for part in combo.split(" + ").collect::<Vec<_>>().into_iter().rev() {
        let g =
            ui.painter()
                .layout_no_wrap(part.to_string(), FontId::proportional(13.0), t.secondary);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(g.size().x + 12.0, 22.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 4.0, l.field);
        ui.painter().rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, l.control_border),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            egui::pos2(rect.left() + 6.0, rect.center().y - g.size().y / 2.0),
            g,
            t.secondary,
        );
    }
}

// ---------------------------------------------------------------- the window

impl NotesApp {
    pub(super) fn open_settings(&mut self, tab: SettingsTab) {
        self.settings_tab = Some(tab);
    }

    pub(super) fn settings_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(tab) = self.settings_tab else { return };
        let dark = ctx.global_style().visuals.dark_mode;
        let t = tone(dark);
        let l = look(dark);
        let screen = ctx.content_rect();
        // Measured on the reference: 66 px side margins, about 724 px high on an 857 px window.
        // Smaller windows (half a screen) keep small margins and never overflow.
        let mx = if screen.width() >= 1000.0 {
            66.0
        } else if screen.width() >= 600.0 {
            24.0
        } else {
            8.0
        };
        let my = if screen.height() >= 800.0 { 66.0 } else { 16.0 };
        let size = egui::vec2(
            (screen.width() - 2.0 * mx).clamp(240.0, 1190.0),
            (screen.height() - 2.0 * my).clamp(240.0, 725.0),
        );
        let compact = size.x < 720.0;
        let mut close = false;
        let area = egui::Modal::default_area(egui::Id::unique("settings-modal"))
            .default_size(size)
            .anchor(
                egui::Align2::CENTER_CENTER,
                egui::vec2(0.0, if my > 20.0 { 18.0 } else { 0.0 }),
            );
        let resp = egui::Modal::new(egui::Id::unique("settings-modal"))
            .area(area)
            .backdrop_color(Color32::from_black_alpha(112))
            .frame(
                egui::Frame::new()
                    .fill(l.window)
                    .stroke(egui::Stroke::new(1.0, l.border))
                    .corner_radius(12.0),
            )
            .show(ctx, |ui| {
                ui.set_min_size(size);
                ui.set_max_size(size);
                let full = ui.max_rect();
                ui.set_clip_rect(full);
                // Left column: 240 px, narrower on smaller windows, icons only when very narrow.
                let nav_w = if compact {
                    56.0
                } else {
                    (size.x * 0.26).clamp(200.0, 240.0)
                };
                let nav_rect = egui::Rect::from_min_size(full.min, egui::vec2(nav_w, size.y));
                ui.painter().rect_filled(
                    nav_rect,
                    egui::CornerRadius {
                        nw: 12,
                        sw: 12,
                        ne: 0,
                        se: 0,
                    },
                    l.nav,
                );
                let mut nav = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(nav_rect.shrink2(egui::vec2(8.0, 10.0)))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                nav.spacing_mut().item_spacing.y = 2.0;
                if compact {
                    self.settings_search.clear();
                } else {
                    nav.add(
                        egui::TextEdit::singleline(&mut self.settings_search)
                            .hint_text(
                                RichText::new(crate::tr!(
                                    "Chercher dans les paramètres",
                                    "Search the settings"
                                ))
                                .color(t.tertiary),
                            )
                            .font(FontId::proportional(14.0))
                            .desired_width(f32::INFINITY)
                            .frame(
                                egui::Frame::new()
                                    .fill(l.field)
                                    .stroke(egui::Stroke::new(1.0, l.control_border))
                                    .corner_radius(6.0)
                                    .inner_margin(egui::Margin {
                                        left: 30,
                                        right: 8,
                                        top: 6,
                                        bottom: 6,
                                    }),
                            ),
                    );
                    let sr = nav.min_rect();
                    icons::paint(
                        nav.painter(),
                        egui::Rect::from_center_size(
                            egui::pos2(sr.left() + 15.0, sr.top() + 15.0),
                            egui::vec2(14.0, 14.0),
                        ),
                        Icon::Search,
                        t.tertiary,
                    );
                }
                let q = self.settings_search.trim().to_lowercase();
                egui::ScrollArea::vertical()
                    .id_salt("settings-nav")
                    .show(&mut nav, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (group, tabs) in groups() {
                            let shown: Vec<SettingsTab> = tabs
                                .iter()
                                .copied()
                                .filter(|x| {
                                    q.is_empty()
                                        || x.label().to_lowercase().contains(&q)
                                        || group.to_lowercase().contains(&q)
                                })
                                .collect();
                            if shown.is_empty() {
                                continue;
                            }
                            ui.add_space(12.0);
                            if compact {
                                let w = ui.available_width();
                                let (r, _) = ui
                                    .allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
                                ui.painter().hline(
                                    r.x_range(),
                                    r.center().y,
                                    egui::Stroke::new(1.0, l.border),
                                );
                            } else {
                                ui.label(RichText::new(group).size(13.0).color(t.tertiary));
                            }
                            ui.add_space(2.0);
                            for x in shown {
                                let w = ui.available_width();
                                let (rect, r) = ui
                                    .allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::click());
                                if x == tab {
                                    ui.painter().rect_filled(rect, 6.0, l.nav_selected);
                                } else if r.hovered() {
                                    ui.painter().rect_filled(rect, 6.0, t.hover);
                                }
                                icons::paint(
                                    ui.painter(),
                                    egui::Rect::from_center_size(
                                        egui::pos2(rect.left() + 16.0, rect.center().y),
                                        egui::vec2(18.0, 18.0),
                                    ),
                                    x.icon(),
                                    t.secondary,
                                );
                                let r = if compact {
                                    r.on_hover_text(x.label())
                                } else {
                                    let font = if x == tab {
                                        semibold(14.0)
                                    } else {
                                        FontId::proportional(14.0)
                                    };
                                    ui.painter().text(
                                        egui::pos2(rect.left() + 36.0, rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        x.label(),
                                        font,
                                        t.text,
                                    );
                                    r
                                };
                                if r.clicked() {
                                    self.settings_tab = Some(x);
                                }
                            }
                        }
                    });
                // Content.
                let content_rect =
                    egui::Rect::from_min_max(egui::pos2(nav_rect.right(), full.top()), full.max);
                ui.painter().vline(
                    nav_rect.right(),
                    full.y_range(),
                    egui::Stroke::new(1.0, l.border),
                );
                let mut content = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(content_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                // 68 px of padding as on the reference, less on narrow windows; the text column
                // never exceeds 800 px and always fits.
                let cw = content_rect.width();
                let pad: i8 = if cw > 940.0 {
                    68
                } else if cw > 560.0 {
                    40
                } else {
                    20
                };
                let inner_w = (cw - 2.0 * pad as f32 - 12.0).clamp(120.0, 800.0);
                egui::ScrollArea::vertical()
                    .id_salt(("settings-content", tab as u8))
                    .auto_shrink([false, false])
                    .show(&mut content, |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: pad,
                                right: pad,
                                top: 0,
                                bottom: 40,
                            })
                            .show(ui, |ui| {
                                ui.set_width(inner_w);
                                match tab {
                                    SettingsTab::Preferences => self.tab_preferences(ui, &t, &l),
                                    SettingsTab::Shortcuts => self.tab_shortcuts(ui, &t, &l),
                                    SettingsTab::General => self.tab_general(ui, &t, &l),
                                    SettingsTab::Sidebar => self.tab_sidebar(ui, &t, &l),
                                    SettingsTab::ImportExport => {
                                        self.tab_import_export(ui, &t, &l, actions)
                                    }
                                    SettingsTab::Ai => self.tab_ai(ui, &t, &l),
                                    SettingsTab::Collab => self.tab_collab(ui, &t, &l),
                                }
                            });
                    });
                // Close button, top right.
                let cr = egui::Rect::from_center_size(
                    egui::pos2(full.right() - 26.0, full.top() + 26.0),
                    egui::vec2(26.0, 26.0),
                );
                let c = ui.interact(cr, egui::Id::unique("settings-close"), egui::Sense::click());
                if c.hovered() {
                    ui.painter().rect_filled(cr, 6.0, t.hover);
                }
                icons::paint(ui.painter(), cr.shrink(6.0), Icon::Close, t.secondary);
                if c.on_hover_text(crate::tr!("Fermer (Échap)", "Close (Esc)"))
                    .clicked()
                {
                    close = true;
                }
            });
        if resp.should_close() || close {
            self.settings_tab = None;
        }
    }

    fn tab_preferences(&mut self, ui: &mut egui::Ui, t: &Tone, l: &Look) {
        page_title(
            ui,
            crate::tr!("Préférences", "Preferences"),
            crate::tr!(
                "Choisissez l'apparence et le comportement de Orchestra",
                "Choose how Orchestra looks and behaves"
            ),
            t,
        );
        section(ui, crate::tr!("Préférences", "Preferences"), t, l);
        let cur = match self.ws.settings.theme.as_str() {
            "light" => 1,
            "dark" => 2,
            _ if self.ws.settings.dark => 2,
            _ => 0,
        };
        row(
            ui,
            crate::tr!("Thème", "Theme"),
            crate::tr!(
                "Choisissez un thème pour Orchestra sur cet appareil",
                "Choose a theme for Orchestra on this device"
            ),
            t,
            |ui| {
                if let Some(i) = dropdown(
                    ui,
                    "theme",
                    &[
                        crate::tr!("Paramètres du système", "System settings"),
                        crate::tr!("Clair", "Light"),
                        crate::tr!("Sombre", "Dark"),
                    ],
                    cur,
                    t,
                    l,
                ) {
                    self.ws.settings.theme = ["system", "light", "dark"][i].into();
                    self.ws.settings.dark = i == 2;
                }
            },
        );
        // Language: automatic (the system's), French or English.
        let sys = if crate::i18n::system_is_english() {
            "English"
        } else {
            "Français"
        };
        let auto_label = crate::trf!("Automatique ({sys})", "Automatic ({sys})");
        let cur_lang = match self.ws.settings.language.as_str() {
            "fr" => 1,
            "en" => 2,
            _ => 0,
        };
        row(
            ui,
            crate::tr!("Langue · Language", "Language · Langue"),
            crate::tr!(
                "Automatique : celle de votre système (anglais si elle n'est pas le français).",
                "Automatic: your system's (English unless it is French)."
            ),
            t,
            |ui| {
                if let Some(i) = dropdown(
                    ui,
                    "language",
                    &[auto_label.as_str(), "Français", "English"],
                    cur_lang,
                    t,
                    l,
                ) {
                    self.ws.settings.language = ["", "fr", "en"][i].into();
                    crate::i18n::set(&self.ws.settings.language);
                    self.dirty_since.get_or_insert_with(std::time::Instant::now);
                }
            },
        );
        let zooms = [0.5f32, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];
        let z = ui.ctx().zoom_factor();
        let zi = zooms.iter().position(|v| (v - z).abs() < 0.03).unwrap_or(4);
        let labels: Vec<String> = zooms
            .iter()
            .map(|v| format!("{} %", (v * 100.0).round()))
            .collect();
        let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        row(
            ui,
            crate::tr!("Zoom", "Zoom"),
            crate::tr!(
                "Taille de l'interface (Cmd/Ctrl + = / - / 0, de 50 % à 490 %)",
                "Size of the interface (Cmd/Ctrl + = / - / 0, from 50 % to 490 %)"
            ),
            t,
            |ui| {
                if let Some(i) = dropdown(ui, "zoom", &label_refs, zi, t, l) {
                    let ctx = ui.ctx().clone();
                    self.set_zoom(&ctx, zooms[i]);
                }
            },
        );
        section(ui, crate::tr!("Démarrage", "Startup"), t, l);
        let mut reopen = !self.ws.settings.start_fresh;
        row(
            ui,
            crate::tr!(
                "Rouvrir les onglets de la dernière fois",
                "Reopen last time's tabs"
            ),
            crate::tr!(
                "Au lancement, les pages ouvertes à la fermeture reviennent dans leurs onglets.",
                "At launch, the pages open when you quit come back in their tabs."
            ),
            t,
            |ui| {
                if switch(ui, &mut reopen, l).changed() {
                    self.ws.settings.start_fresh = !reopen;
                }
            },
        );
        section(ui, crate::tr!("Aide", "Help"), t, l);
        let mut replay = false;
        row(
            ui,
            crate::tr!("Tutoriel", "Tutorial"),
            crate::tr!(
                "Refaire la visite guidée de l'application depuis le début.",
                "Take the guided tour of the app again from the start."
            ),
            t,
            |ui| {
                replay = button(ui, crate::tr!("Relancer le tuto", "Replay the tutorial"), t, l)
                    .clicked();
            },
        );
        if replay {
            self.settings_tab = None;
            self.start_tour();
        }
        section(ui, crate::tr!("Vos données", "Your data"), t, l);
        row(
            ui,
            crate::tr!("Sans compte ni abonnement", "No account, no subscription"),
            crate::tr!(
                "Tout reste sur cet ordinateur, sans limite de pages, de blocs ou de fichiers.",
                "Everything stays on this computer, with no limit on pages, blocks or files."
            ),
            t,
            |_| {},
        );
    }

    fn tab_shortcuts(&mut self, ui: &mut egui::Ui, t: &Tone, l: &Look) {
        page_title(
            ui,
            crate::tr!("Raccourcis clavier", "Keyboard shortcuts"),
            crate::tr!(
                "Les raccourcis du menu de Notion, et ceux de l'éditeur",
                "Notion's menu shortcuts, and the editor's"
            ),
            t,
        );
        for (title, list) in [
            (
                crate::tr!("Fenêtre et onglets", "Window and tabs"),
                vec![
                    ("Cmd/Ctrl + T", crate::tr!("Nouvel onglet", "New tab")),
                    (
                        "Cmd/Ctrl + W",
                        crate::tr!("Fermer l'onglet", "Close the tab"),
                    ),
                    (
                        crate::tr!("Cmd/Ctrl + Maj + T", "Cmd/Ctrl + Shift + T"),
                        crate::tr!(
                            "Rouvrir le dernier onglet fermé",
                            "Reopen the last closed tab"
                        ),
                    ),
                    ("Ctrl + Tab", crate::tr!("Onglet suivant", "Next tab")),
                    (
                        crate::tr!("Ctrl + Maj + Tab", "Ctrl + Shift + Tab"),
                        crate::tr!("Onglet précédent", "Previous tab"),
                    ),
                    (
                        "Cmd/Ctrl + 1…9",
                        crate::tr!("Aller à un onglet", "Go to a tab"),
                    ),
                    (
                        "Cmd/Ctrl + [",
                        crate::tr!("Page précédente", "Previous page"),
                    ),
                    ("Cmd/Ctrl + ]", crate::tr!("Page suivante", "Next page")),
                    (
                        "Cmd/Ctrl + \\",
                        crate::tr!(
                            "Afficher ou masquer la barre latérale",
                            "Show or hide the sidebar"
                        ),
                    ),
                    ("Cmd/Ctrl + =", crate::tr!("Zoom avant", "Zoom in")),
                    ("Cmd/Ctrl + -", crate::tr!("Zoom arrière", "Zoom out")),
                    ("Cmd/Ctrl + 0", crate::tr!("Taille réelle", "Actual size")),
                ],
            ),
            (
                crate::tr!("Pages", "Pages"),
                vec![
                    ("Cmd/Ctrl + P", crate::tr!("Rechercher", "Search")),
                    ("Cmd/Ctrl + N", crate::tr!("Nouvelle page", "New page")),
                    (
                        "Cmd/Ctrl + L",
                        crate::tr!("Copier le lien de la page", "Copy the page's link"),
                    ),
                    (
                        "Cmd/Ctrl + Alt + L",
                        crate::tr!(
                            "Copier le lien avec le titre",
                            "Copy the link with the title"
                        ),
                    ),
                ],
            ),
            (
                crate::tr!("Blocs", "Blocks"),
                vec![
                    (
                        crate::tr!("Entrée", "Enter"),
                        crate::tr!("Nouveau bloc", "New block"),
                    ),
                    (
                        crate::tr!("Maj + Entrée", "Shift + Enter"),
                        crate::tr!(
                            "Retour à la ligne dans le bloc",
                            "Line break inside the block"
                        ),
                    ),
                    (
                        "Tab",
                        crate::tr!("Décaler le bloc vers la droite", "Indent the block"),
                    ),
                    (
                        crate::tr!("Maj + Tab", "Shift + Tab"),
                        crate::tr!("Décaler le bloc vers la gauche", "Outdent the block"),
                    ),
                    (
                        "/",
                        crate::tr!("Choisir un type de bloc", "Choose a block type"),
                    ),
                    (
                        crate::tr!("# + espace", "# + space"),
                        crate::tr!(
                            "Titre 1 (## et ### pour les titres 2 et 3)",
                            "Heading 1 (## and ### for headings 2 and 3)"
                        ),
                    ),
                    (
                        crate::tr!("- + espace", "- + space"),
                        crate::tr!("Liste à puces", "Bulleted list"),
                    ),
                    (
                        crate::tr!("1. + espace", "1. + space"),
                        crate::tr!("Liste numérotée", "Numbered list"),
                    ),
                    (
                        crate::tr!("[] + espace", "[] + space"),
                        crate::tr!("Liste de tâches", "To-do list"),
                    ),
                    (
                        crate::tr!("> + espace", "> + space"),
                        crate::tr!("Menu dépliant", "Toggle list"),
                    ),
                    (
                        crate::tr!("\" + espace", "\" + space"),
                        crate::tr!("Citation", "Quote"),
                    ),
                    ("---", crate::tr!("Séparateur", "Divider")),
                    ("```", crate::tr!("Code", "Code")),
                ],
            ),
        ] {
            section(ui, title, t, l);
            for (combo, what) in list {
                ui.horizontal(|ui| {
                    ui.set_min_height(36.0);
                    ui.label(RichText::new(what).size(14.0).color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        keys(ui, combo, t, l)
                    });
                });
            }
        }
    }

    fn tab_general(&mut self, ui: &mut egui::Ui, t: &Tone, l: &Look) {
        page_title(
            ui,
            crate::tr!("Général", "General"),
            crate::tr!(
                "Le nom de votre espace et l'endroit où il est enregistré",
                "Your workspace's name and where it is saved"
            ),
            t,
        );
        section(ui, crate::tr!("Espace de travail", "Workspace"), t, l);
        row(
            ui,
            crate::tr!("Nom", "Name"),
            crate::tr!(
                "Affiché en haut et en bas de la barre latérale.",
                "Shown at the top and bottom of the sidebar."
            ),
            t,
            |ui| {
                ui.add(field(
                    &mut self.ws.settings.workspace_name,
                    crate::tr!("Mon espace", "My workspace"),
                    240.0,
                    t,
                    l,
                ));
            },
        );
        section(ui, crate::tr!("Données", "Data"), t, l);
        let dir = store::data_dir();
        let path = dir.display().to_string();
        row(
            ui,
            crate::tr!("Dossier des données", "Data folder"),
            &crate::trf!(
                "{path}\nVos pages et réglages (workspace.json), les copies des sessions autonomes, le suivi de l'abonnement.",
                "{path}\nYour pages and settings (workspace.json), the copies of autonomous sessions, the subscription tracking."
            ),
            t,
            |ui| {
                if button(ui, crate::tr!("Copier le chemin", "Copy the path"), t, l).clicked() {
                    ui.ctx().copy_text(path.clone());
                    self.ack(crate::tr!("Chemin copié.", "Path copied."));
                }
                if button(ui, crate::tr!("Ouvrir le dossier", "Open the folder"), t, l).clicked() {
                    let _ = std::fs::create_dir_all(&dir);
                    if let Err(e) = crate::opener::open_path(&dir) {
                        self.fail(crate::trf!(
                            "Impossible d'ouvrir le dossier : {e}",
                            "Couldn't open the folder: {e}"
                        ));
                    }
                }
            },
        );
        self.data_folder_choice(ui, t, l);
        // On macOS the app goes to the Trash with no uninstaller: this undoes, first, what
        // Orchestra set up outside its folder (Windows' uninstaller runs it by itself).
        row(
            ui,
            crate::tr!("Avant de désinstaller", "Before uninstalling"),
            crate::tr!(
                "Remet la ligne d'état de Claude Code comme avant et débranche Orchestra des IA, pour que rien ne tente de le relancer une fois supprimé. Vos pages restent dans le dossier des données.",
                "Puts Claude Code's status line back as before and unplugs Orchestra from the AIs, so that nothing tries to start it once removed. Your pages stay in the data folder."
            ),
            t,
            |ui| {
                if button(ui, crate::tr!("Tout débrancher", "Unplug everything"), t, l).clicked() {
                    // Runs the AIs' CLIs: off the UI thread, the result comes back as a message.
                    let (done, c) = (self.uninstall_report.clone(), ui.ctx().clone());
                    std::thread::spawn(move || {
                        *done.lock_or_recover() = Some(crate::setup::clean_up_for_uninstall());
                        c.request_repaint();
                    });
                }
            },
        );
        self.update_settings(ui, t, l);
        self.third_party(ui, t, l);
    }

    /// « Changer de dossier »: pick another data folder, used from the next launch (see
    /// data_folder.rs).
    fn data_folder_choice(&mut self, ui: &mut egui::Ui, t: &Tone, l: &Look) {
        use crate::data_folder;
        use super::projects::folder_picker;
        let title = crate::tr!("Changer de dossier", "Change folder");
        if store::forced_data_dir().is_some() {
            let desc = crate::tr!(
                "Ce dossier est imposé par la variable ORCHESTRA_DIR : lancez Orchestra sans elle pour le choisir ici.",
                "This folder is imposed by the ORCHESTRA_DIR variable: launch Orchestra without it to choose it here."
            );
            row(ui, title, desc, t, |_| {});
            return;
        }
        let (base, usual) = (data_folder::base_dir(), store::usual_data_dir());
        let current = store::data_dir();
        let chosen = data_folder::chosen_in(&base);
        if let Some(dir) = folder_picker::take("data-folder") {
            self.data_dir_draft = dir.display().to_string();
        }
        if let Some(wanted) = data_folder::wanted_in(&base) {
            let to = wanted.display().to_string();
            let what = match data_folder::check(&current, &wanted, &usual) {
                Ok(data_folder::Plan::Copy) => crate::tr!(
                    "Vos données d'aujourd'hui y seront copiées.",
                    "Today's data will be copied there."
                )
                .to_string(),
                Ok(data_folder::Plan::UseExisting) => crate::tr!(
                    "Il contient déjà des données : elles seront utilisées telles quelles, rien n'y sera copié.",
                    "It already holds data: it will be used as it is, nothing will be copied there."
                )
                .to_string(),
                Err(e) => crate::trf!("Impossible en l'état : {e}.", "Not possible as it is: {e}."),
            };
            row(
                ui,
                crate::tr!("Nouveau dossier au prochain lancement", "New folder at the next launch"),
                &crate::trf!(
                    "{to}\n{what} Fermez les sessions d'IA en cours (elles écriraient encore dans l'ancien dossier), puis quittez et rouvrez Orchestra.",
                    "{to}\n{what} Close the running AI sessions (they would still write to the old folder), then quit and reopen Orchestra."
                ),
                t,
                |ui| {
                    if button(ui, crate::tr!("Annuler", "Cancel"), t, l).clicked() {
                        data_folder::cancel_in(&base);
                    }
                },
            );
            return;
        }
        let missing = chosen.as_ref().filter(|c| **c != current);
        let desc = match missing {
            Some(c) => crate::trf!(
                "Le dossier choisi ({}) était introuvable au lancement : le dossier habituel est utilisé. Rebranchez son disque puis relancez Orchestra.",
                "The chosen folder ({}) wasn't found at launch: the usual folder is in use. Plug its disk back in, then relaunch Orchestra.",
                c.display()
            ),
            None => crate::tr!(
                "Pris en compte au prochain lancement. Un dossier vide reçoit une copie de vos données ; un dossier de données d'Orchestra est utilisé tel quel. Rien n'est effacé de l'ancien, qui garde aussi les copies des sessions autonomes.",
                "Takes effect at the next launch. An empty folder gets a copy of your data; an Orchestra data folder is used as it is. Nothing is deleted from the old one, which also keeps the copies of autonomous sessions."
            )
            .to_string(),
        };
        row(ui, title, &desc, t, |ui| {
            if missing.is_some() {
                if button(ui, crate::tr!("Oublier ce dossier", "Forget this folder"), t, l).clicked() {
                    data_folder::forget_in(&base);
                }
            } else if chosen.is_some()
                && button(ui, crate::tr!("Revenir au dossier habituel", "Back to the usual folder"), t, l).clicked()
                && let Err(e) = data_folder::request_in(&base, &current, &usual, &usual)
            {
                self.fail(crate::trf!(
                    "Impossible de changer de dossier : {e}",
                    "Couldn't change the folder: {e}"
                ));
            }
        });
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let typed = !self.data_dir_draft.trim().is_empty();
                let apply = ui
                    .add_enabled_ui(typed, |ui| {
                        button(ui, crate::tr!("Utiliser ce dossier", "Use this folder"), t, l)
                    })
                    .inner;
                if apply.clicked() {
                    let target = data_folder::expand(&self.data_dir_draft);
                    match data_folder::request_in(&base, &current, &target, &usual) {
                        Ok(()) => self.data_dir_draft.clear(),
                        Err(e) => self.fail(crate::trf!(
                            "Impossible de changer de dossier : {e}",
                            "Couldn't change the folder: {e}"
                        )),
                    }
                }
                let browse = ui
                    .add_enabled_ui(!folder_picker::busy(), |ui| {
                        button(ui, crate::tr!("Parcourir…", "Browse…"), t, l)
                    })
                    .inner;
                if browse.clicked() {
                    folder_picker::start(
                        ui.ctx(),
                        "data-folder",
                        crate::tr!("Dossier des données", "Data folder"),
                        dirs::home_dir(),
                    );
                }
                ui.add(field(
                    &mut self.data_dir_draft,
                    crate::tr!("/chemin/du/nouveau/dossier", "/path/to/the/new/folder"),
                    f32::INFINITY,
                    t,
                    l,
                ));
            });
        });
    }

    /// « Licences tierces »: the software shipped with Orchestra, its licence and where it
    /// comes from (packaging/licenses, the same text shipped next to the program).
    fn third_party(&mut self, ui: &mut egui::Ui, t: &Tone, l: &Look) {
        section(ui, crate::tr!("Licences tierces", "Third-party licences"), t, l);
        let open_key = egui::Id::unique("third-party-open");
        let mut open: bool = ui.ctx().data(|d| d.get_temp(open_key)).unwrap_or(false);
        row(
            ui,
            "RTK (rtk-oss) 0.49.0",
            crate::tr!(
                "© 2026 RTK AI Labs · Apache License 2.0 · github.com/rtk-ai/rtk. Raccourcit la sortie des commandes dans les sessions autonomes ; livré sans son moteur sous licence BUSL-1.1.",
                "© 2026 RTK AI Labs · Apache License 2.0 · github.com/rtk-ai/rtk. Shortens the output of commands in autonomous sessions; shipped without its BUSL-1.1 engine."
            ),
            t,
            |ui| {
                let label = if open {
                    crate::tr!("Masquer la licence", "Hide the licence")
                } else {
                    crate::tr!("Lire la licence", "Read the licence")
                };
                if button(ui, label, t, l).clicked() {
                    open = !open;
                    ui.ctx().data_mut(|d| d.insert_temp(open_key, open));
                }
            },
        );
        if open {
            egui::ScrollArea::vertical()
                .id_salt("third-party-text")
                .max_height(320.0)
                .show(ui, |ui| {
                    ui.style_mut().interaction.selectable_labels = true;
                    for text in THIRD_PARTY_TEXTS {
                        ui.label(RichText::new(*text).monospace().size(11.5).color(t.secondary));
                        ui.add_space(12.0);
                    }
                });
        }
    }

    fn tab_sidebar(&mut self, ui: &mut egui::Ui, t: &Tone, l: &Look) {
        page_title(
            ui,
            crate::tr!("Barre latérale", "Sidebar"),
            crate::tr!("Ce qu'affiche la barre latérale", "What the sidebar shows"),
            t,
        );
        section(ui, crate::tr!("Affichage", "Display"), t, l);
        let mut shown = !self.ws.settings.sidebar_hidden;
        row(
            ui,
            crate::tr!("Afficher la barre latérale", "Show the sidebar"),
            crate::tr!("Aussi avec Cmd/Ctrl + \\.", "Also with Cmd/Ctrl + \\."),
            t,
            |ui| {
                if switch(ui, &mut shown, l).changed() {
                    self.ws.settings.sidebar_hidden = !shown;
                }
            },
        );
        let mut card = !self.ws.settings.hide_connect_card;
        row(
            ui,
            crate::tr!(
                "Carte « Brancher la collaboration »",
                "« Set up collaboration » card"
            ),
            crate::tr!(
                "Rappel en bas de la barre latérale tant qu'aucun branchement n'est choisi.",
                "Reminder at the bottom of the sidebar while no collaboration is chosen."
            ),
            t,
            |ui| {
                if switch(ui, &mut card, l).changed() {
                    self.ws.settings.hide_connect_card = !card;
                }
            },
        );
        row(
            ui,
            crate::tr!("Largeur", "Width"),
            crate::tr!(
                "Glissez le bord de la barre latérale pour l'élargir.",
                "Drag the sidebar's edge to widen it."
            ),
            t,
            |ui| {
                if button(ui, crate::tr!("Rétablir", "Reset"), t, l).clicked() {
                    self.ws.settings.sidebar_width = 0.0;
                }
            },
        );
    }

    fn tab_import_export(
        &mut self,
        ui: &mut egui::Ui,
        t: &Tone,
        l: &Look,
        actions: &mut Vec<Action>,
    ) {
        page_title(
            ui,
            crate::tr!("Importer et exporter", "Import and export"),
            crate::tr!(
                "Vos pages en Markdown, dans les deux sens",
                "Your pages as Markdown, both ways"
            ),
            t,
        );
        section(ui, crate::tr!("Exporter", "Export"), t, l);
        row(
            ui,
            crate::tr!("Toutes les pages en Markdown", "All pages as Markdown"),
            crate::tr!(
                "Un fichier .md par page dans ce dossier.",
                "One .md file per page in this folder."
            ),
            t,
            |ui| {
                if button(ui, crate::tr!("Exporter", "Export"), t, l).clicked() {
                    let r = store::export_all(&self.ws, std::path::Path::new(&self.export_dir))
                        .map(|n| {
                            crate::trf!(
                                "{n} page(s) exportée(s) dans {}",
                                "{n} page(s) exported to {}",
                                self.export_dir
                            )
                        })
                        .map_err(|e| crate::trf!("Export impossible : {e}", "Export failed: {e}"));
                    self.notice(super::notices::Notice::from_result(r));
                }
            },
        );
        ui.add(field(
            &mut self.export_dir,
            crate::tr!("/chemin/du/dossier", "/path/to/folder"),
            f32::INFINITY,
            t,
            l,
        ));
        section(ui, crate::tr!("Importer", "Import"), t, l);
        row(
            ui,
            crate::tr!("Un fichier Markdown", "A Markdown file"),
            crate::tr!(
                "Le fichier devient une nouvelle page (titres, listes, cases, citations, code).",
                "The file becomes a new page (headings, lists, to-dos, quotes, code)."
            ),
            t,
            |ui| {
                if button(ui, crate::tr!("Importer", "Import"), t, l).clicked() {
                    let path = std::path::PathBuf::from(self.import_path.trim());
                    match std::fs::read_to_string(&path) {
                        Ok(md) => {
                            let stem = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or(crate::tr!("Import", "Import"))
                                .to_string();
                            let p = store::page_from_markdown(&md, &stem);
                            let id = p.id.clone();
                            self.ws.pages.push(p);
                            actions.push(Action::Open(id));
                            self.settings_tab = None;
                            self.ack(crate::tr!("Page importée.", "Page imported."));
                        }
                        Err(e) => self.fail(crate::trf!(
                            "Lecture impossible : {e}",
                            "Couldn't read it: {e}"
                        )),
                    }
                }
            },
        );
        ui.add(field(
            &mut self.import_path,
            "/chemin/vers/note.md",
            f32::INFINITY,
            t,
            l,
        ));
    }

    fn tab_ai(&mut self, ui: &mut egui::Ui, t: &Tone, l: &Look) {
        page_title(
            ui,
            crate::tr!("IA (MCP)", "AI (MCP)"),
            crate::tr!(
                "Les IA branchées lisent et écrivent vos projets ; elles apparaissent sur les pages des projets",
                "The AIs plugged in read and write your projects; they show up on the project pages"
            ),
            t,
        );
        self.ai_list(ui, t, l);
        let exe = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "orchestra".into());
        let mut server = serde_json::json!({"command": exe, "args": ["mcp"]});
        if let Ok(d) = std::env::var("ORCHESTRA_DIR") {
            server["env"] = serde_json::json!({"ORCHESTRA_DIR": d});
        }
        let config =
            serde_json::to_string_pretty(&serde_json::json!({"mcpServers": {"orchestra": server}}))
                .unwrap_or_default();
        section(ui, crate::tr!("Autre client MCP", "Other MCP client"), t, l);
        let cmd = format!("{exe} mcp");
        row(
            ui,
            crate::tr!("Commande du serveur MCP", "MCP server command"),
            crate::tr!(
                "À donner à votre client MCP (Claude Desktop, Cursor, un modèle local…).",
                "To give to your MCP client (Claude Desktop, Cursor, a local model…)."
            ),
            t,
            |ui| {
                if button(ui, crate::tr!("Copier", "Copy"), t, l).clicked() {
                    ui.ctx().copy_text(cmd.clone());
                    self.ack(crate::tr!("Commande copiée.", "Command copied."));
                }
            },
        );
        let mut c = cmd.clone();
        ui.add(field(&mut c, "", f32::INFINITY, t, l).font(FontId::monospace(13.0)));
        ui.add_space(8.0);
        row(
            ui,
            crate::tr!("Bloc de configuration", "Configuration block"),
            crate::tr!(
                "Le format attendu par la plupart des clients.",
                "The format most clients expect."
            ),
            t,
            |ui| {
                if button(ui, crate::tr!("Copier", "Copy"), t, l).clicked() {
                    ui.ctx().copy_text(config.clone());
                    self.ack(crate::tr!("Configuration copiée.", "Configuration copied."));
                }
            },
        );
        let mut cfg = config.clone();
        ui.add(
            egui::TextEdit::multiline(&mut cfg)
                .font(FontId::monospace(13.0))
                .desired_width(f32::INFINITY)
                .desired_rows(8)
                .frame(
                    egui::Frame::new()
                        .fill(l.field)
                        .stroke(egui::Stroke::new(1.0, l.control_border))
                        .corner_radius(6.0)
                        .inner_margin(10.0),
                ),
        );
        section(
            ui,
            crate::tr!("Ce que l'assistant peut faire", "What the assistant can do"),
            t,
            l,
        );
        for (name, what) in [
            (
                crate::tr!("Trouver des pages", "Find pages"),
                "list_pages, search_pages",
            ),
            (
                crate::tr!("Lire une page en Markdown", "Read a page as Markdown"),
                "read_page",
            ),
            (
                crate::tr!(
                    "Écrire, résumer, traduire, compléter",
                    "Write, summarise, translate, complete"
                ),
                "create_page, append_to_page, replace_page_content",
            ),
            (
                crate::tr!("Modifier, renommer, ranger", "Edit, rename, organise"),
                "update_block, update_page, trash_page, restore_page",
            ),
            (
                crate::tr!("Gérer les bases de données", "Manage databases"),
                "create_database, query_database, add / update / delete_database_row",
            ),
        ] {
            row(ui, name, what, t, |_| {});
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(crate::tr!(
                "Les modifications de l'assistant apparaissent ici en une ou deux secondes.",
                "The assistant's changes show up here within a second or two."
            ))
            .size(13.0)
            .color(t.tertiary),
        );
    }

    /// Who fills projects in, suggests their steps and talks with the person about a project
    /// without code: Claude Code or Codex, else a model behind an OpenAI-compatible address,
    /// with its name, address and key.
    fn text_model_settings(&mut self, ui: &mut egui::Ui, t: &Tone, l: &Look) {
        use crate::sync::openai::{OpenAi, Service};
        section(
            ui,
            crate::tr!("Remplir, suggérer des étapes et discuter", "Filling in, suggesting steps and chatting"),
            t,
            l,
        );
        let before = self.ws.settings.text_model.clone();
        let mut choice = before.clone();
        let service = Service::from_id(&choice.service);
        let mut choices = vec![crate::tr!("Claude Code ou Codex", "Claude Code or Codex")];
        choices.extend(Service::ALL.map(|s| match s {
            Service::Ollama => crate::tr!("Ollama (modèle local)", "Ollama (local model)"),
            Service::Mistral => crate::tr!("Mistral (clé API)", "Mistral (API key)"),
            Service::Other => crate::tr!("Autre serveur compatible OpenAI", "Other OpenAI-compatible server"),
        }));
        let current = service
            .and_then(|s| Service::ALL.iter().position(|x| *x == s))
            .map_or(0, |i| i + 1);
        row(
            ui,
            crate::tr!("IA utilisée", "AI used"),
            crate::tr!(
                "Pour « Remplir avec l'IA » (aussi à l'ajout d'un projet), « Suggérer des étapes » et la conversation d'un projet sans code (volet de droite). Claude Code et Codex lisent le dossier. Un autre modèle ne le peut pas : Orchestra envoie à son serveur le titre et les étapes du projet, les noms des fichiers, l'état de git, le début du README et de quelques fichiers (jamais les fichiers cachés), ou le texte et les images que vous donnez. Pour la conversation, il reçoit aussi vos messages, l'état des lieux et le texte de la page du projet.",
                "For « Fill in with the AI » (also when a project is added), « Suggest steps » and the conversation of a project without code (right-hand panel). Claude Code and Codex read the folder. Another model can't: Orchestra sends its server the project's title and steps, the file names, the git state, the beginning of the README and of a few files (never hidden ones), or the text and images you give. For the conversation, it also gets your messages, the state of play and the text of the project's page."
            ),
            t,
            |ui| {
                if let Some(i) = dropdown(ui, "text-model", &choices, current, t, l)
                    && i != current
                {
                    // The model and address typed were the other service's.
                    choice = crate::model::TextModel {
                        service: i
                            .checked_sub(1)
                            .map(|i| Service::ALL[i].id().to_string())
                            .unwrap_or_default(),
                        ..Default::default()
                    };
                }
            },
        );
        if let Some(service) = service.filter(|_| choice.service == before.service) {
            row(
                ui,
                crate::tr!("Modèle", "Model"),
                match service {
                    Service::Ollama => crate::tr!(
                        "Son nom, tel que « ollama list » l'affiche. Pour une image jointe, il doit savoir les lire. Sa fenêtre de contexte est souvent petite : Orchestra ne lui envoie qu'un court résumé du projet.",
                        "Its name, as « ollama list » shows it. For a joined image, it has to be able to read them. Its context window is often small: Orchestra only sends it a short summary of the project."
                    ),
                    _ => crate::tr!(
                        "Son nom, tel que le serveur l'attend.",
                        "Its name, as the server expects it."
                    ),
                },
                t,
                |ui| {
                    let hint = match service {
                        Service::Ollama => "qwen3",
                        Service::Mistral => "mistral-small-latest",
                        Service::Other => "",
                    };
                    ui.add(field(&mut choice.model, hint, 240.0, t, l));
                },
            );
            if service != Service::Mistral {
                row(
                    ui,
                    crate::tr!("Adresse", "Address"),
                    match service {
                        Service::Ollama => crate::tr!(
                            "Vide : Ollama sur cet ordinateur.",
                            "Empty: Ollama on this computer."
                        ),
                        _ => crate::tr!(
                            "L'adresse qui précède « /chat/completions ».",
                            "The address that comes before « /chat/completions »."
                        ),
                    },
                    t,
                    |ui| {
                        let hint = match service.default_url() {
                            "" => "http://localhost:1234/v1",
                            url => url,
                        };
                        ui.add(field(&mut choice.url, hint, 240.0, t, l));
                    },
                );
            }
            if let Some(var) = service.key_var() {
                self.model_key_row(ui, var, service, t, l);
            }
            // What would stop a request, said here rather than at the first one.
            if let Some(model) = OpenAi::chosen(&choice)
                && let Err(why) = model.check()
            {
                ui.label(RichText::new(why).size(13.0).color(super::style::ORANGE));
            }
        }
        if choice != before {
            self.ws.settings.text_model = choice;
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    /// The API key of a model's service: typed once, then kept out of sight in the Keychain.
    fn model_key_row(
        &mut self,
        ui: &mut egui::Ui,
        var: &str,
        service: crate::sync::openai::Service,
        t: &Tone,
        l: &Look,
    ) {
        use crate::sync::openai::Service;
        let kept = !crate::secret::api_key(var).is_empty();
        let desc = match (service, kept) {
            (_, false) if service == Service::Mistral => crate::tr!(
                "Gardée dans le trousseau, jamais dans vos données.",
                "Kept in the Keychain, never in your data."
            ),
            (_, false) => crate::tr!(
                "Seulement si le serveur en demande une. Gardée dans le trousseau.",
                "Only if the server asks for one. Kept in the Keychain."
            ),
            (Service::Mistral, true) => crate::tr!(
                "Une clé est enregistrée. Elle n'est envoyée qu'à Mistral.",
                "A key is saved. It is only sent to Mistral."
            ),
            (_, true) => crate::tr!(
                "Une clé est enregistrée. Elle est envoyée à l'adresse ci-dessus.",
                "A key is saved. It is sent to the address above."
            ),
        };
        let id = egui::Id::unique(("model-key", var));
        let mut typed: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
        let mut done = None;
        row(ui, crate::tr!("Clé API", "API key"), desc, t, |ui| {
            if kept {
                if button(ui, crate::tr!("Retirer la clé", "Remove the key"), t, l).clicked() {
                    done = Some(crate::secret::remove_api_key(var));
                }
            } else {
                // In a narrow window the row runs left to right: the field stays first.
                let save = |ui: &mut egui::Ui| {
                    button(ui, crate::tr!("Enregistrer", "Save"), t, l).clicked()
                };
                let input = |ui: &mut egui::Ui, typed: &mut String| {
                    ui.add(field(typed, "", 200.0, t, l).password(true));
                };
                let clicked = if ui.layout().main_dir() == egui::Direction::RightToLeft {
                    let clicked = save(ui);
                    input(ui, &mut typed);
                    clicked
                } else {
                    input(ui, &mut typed);
                    save(ui)
                };
                if clicked && !typed.trim().is_empty() {
                    done = Some(crate::secret::set_api_key(var, &typed));
                }
            }
        });
        match done {
            Some(Ok(())) => typed.clear(),
            Some(Err(e)) => self.fail(crate::trf!(
                "Clé API non enregistrée dans le trousseau : {e}",
                "The API key couldn't be changed in the Keychain: {e}"
            )),
            None => {}
        }
        ui.ctx().data_mut(|d| d.insert_temp(id, typed));
    }

    /// One row per known AI: where it stands, and the button that plugs it in.
    fn ai_list(&mut self, ui: &mut egui::Ui, t: &Tone, l: &Look) {
        use crate::setup::Ai;
        section(ui, crate::tr!("Vos IA", "Your AIs"), t, l);
        let states = self.ais.states.lock_or_recover().clone();
        let busy = self.ais.busy.lock_or_recover().clone();
        let ctx = ui.ctx().clone();
        let mut toggle: Option<(Ai, bool)> = None;
        let mut term: Option<(String, String, Option<String>)> = None;
        let mut install: Option<Ai> = None;
        let installing = self.ais.installing.map(|(a, ..)| a);
        let signing_in = self.ais.signing_in.is_some();
        let mut sign_in = false;
        for ai in Ai::ALL {
            let st = states
                .as_ref()
                .and_then(|s| s.iter().find(|x| x.ai == ai).cloned());
            let state = match &st {
                None => crate::tr!("vérification…", "checking…").to_string(),
                Some(x) if x.plugged => crate::tr!("branchée", "plugged in").to_string(),
                Some(x) if x.installed => {
                    crate::tr!("installée, pas branchée", "installed, not plugged in").to_string()
                }
                Some(_) if installing == Some(ai) => crate::tr!(
                    "installation en cours dans le terminal ; branchée dès qu'elle est là",
                    "installing in the terminal; plugged in as soon as it is there"
                )
                .to_string(),
                Some(_) => crate::tr!(
                    "pas installée sur cette machine",
                    "not installed on this computer"
                )
                .to_string(),
            };
            let plugged_in = st.as_ref().is_some_and(|x| x.plugged);
            let mut desc = format!("{} · {state}", ai.what());
            if let Some(account) = st.as_ref().and_then(|x| x.account.as_ref()) {
                desc.push_str(" · ");
                if ai == Ai::ClaudeCode && signing_in {
                    desc.push_str(crate::tr!(
                        "connexion en cours : terminez-la dans votre navigateur",
                        "signing in: finish in your browser"
                    ));
                } else {
                    desc.push_str(&account.describe());
                }
            }
            row(ui, ai.label(), &desc, t, |ui| {
                if busy.as_deref() == Some(ai.key()) {
                    ui.add(egui::Spinner::new().size(16.0));
                    return;
                }
                let Some(x) = st else { return };
                if x.installed {
                    if x.plugged {
                        if button(ui, crate::tr!("Débrancher", "Unplug"), t, l)
                            .on_hover_text(crate::tr!(
                                "Retire seulement l'entrée « orchestra » de sa configuration",
                                "Only removes the « orchestra » entry from its configuration"
                            ))
                            .clicked()
                        {
                            toggle = Some((ai, false));
                        }
                    } else if button(ui, crate::tr!("Brancher", "Plug in"), t, l).clicked() {
                        toggle = Some((ai, true));
                    }
                    if let Some((cmd, input)) = x.login() {
                        let label = if x.account.as_ref().is_some_and(|a| a.signed_in()) {
                            crate::tr!("Changer de compte", "Switch account")
                        } else {
                            crate::tr!("Se connecter", "Sign in")
                        };
                        let hover = if x.account.is_some() {
                            crate::trf!("Lance « {cmd} » dans le terminal intégré : la connexion à votre compte se fait dans votre navigateur. Orchestra ne voit ni mot de passe ni jeton.", "Runs « {cmd} » in the built-in terminal: you sign in to your account in your browser. Orchestra sees no password or token.")
                        } else {
                            crate::tr!("Ouvre l'IA dans le terminal intégré pour vous connecter à votre compte", "Opens the AI in the built-in terminal so you can sign in to your account").to_string()
                        };
                        if button(ui, label, t, l).on_hover_text(hover).clicked() {
                            sign_in = x.account.is_some();
                            term = Some((ai.label().to_string(), cmd.to_string(), (!input.is_empty()).then(|| input.to_string())));
                        }
                    }
                } else {
                    match &x.install {
                        Some(crate::setup::Install::Run(cmd, how)) => {
                            let label = if installing == Some(ai) {
                                crate::tr!("Relancer l'installation", "Run the install again")
                            } else {
                                crate::tr!("Installer", "Install")
                            };
                            if button(ui, label, t, l)
                                .on_hover_text({
                                    let mut hover = crate::trf!("Lance « {cmd} » dans le terminal intégré ({how}). Orchestra la branche dès qu'elle est installée.", "Runs « {cmd} » in the built-in terminal ({how}). Orchestra plugs it in as soon as it is installed.");
                                    if ai == Ai::ClaudeCode {
                                        hover.push(' ');
                                        hover.push_str(crate::tr!("La connexion à votre compte s'ouvre ensuite dans votre navigateur.", "Signing in to your account then opens in your browser."));
                                    }
                                    hover
                                })
                                .clicked()
                            {
                                term = Some((crate::trf!("Installation · {}", "Installing · {}", ai.label()), cmd.clone(), None));
                                install = Some(ai);
                            }
                        }
                        Some(crate::setup::Install::Download(url, why))
                            if button(ui, crate::tr!("Télécharger", "Download"), t, l)
                                .on_hover_text(format!("{why} · {url}"))
                                .clicked() =>
                        {
                            let _ = crate::opener::open_url(url);
                        }
                        _ => {}
                    }
                }
            });
            // Its limits, right under it, for an agent that can work on its own.
            if plugged_in
                && let Some(agent) = crate::auto::Agent::ALL.into_iter().find(|a| a.ai() == ai)
            {
                ui.push_id(("ai-limits", ai.key()), |ui| self.ai_limits(ui, agent, t));
                ui.add_space(8.0);
            }
        }
        let mut auto = !self.ws.settings.no_auto_mcp;
        row(
            ui,
            crate::tr!("Au démarrage", "At startup"),
            crate::tr!(
                "Brancher les IA installées depuis la dernière fois (sauf celles que vous avez débranchées).",
                "Plug in the AIs installed since last time (except those you unplugged)."
            ),
            t,
            |ui| {
                if switch(ui, &mut auto, l).changed() {
                    self.ws.settings.no_auto_mcp = !auto;
                }
            },
        );
        row(
            ui,
            crate::tr!("Vérifier à nouveau", "Check again"),
            crate::tr!(
                "Après avoir installé une IA ou changé sa configuration.",
                "After installing an AI or changing its configuration."
            ),
            t,
            |ui| {
                if button(ui, crate::tr!("Vérifier", "Check"), t, l).clicked() {
                    *self.ais.states.lock_or_recover() = None;
                    self.refresh_ais(&ctx, false);
                }
            },
        );
        self.text_model_settings(ui, t, l);
        self.session_settings(ui, t, l);
        // Subscription or API key, for each AI that bills by key.
        section(ui, crate::tr!("Abonnement ou clé API", "Subscription or API key"), t, l);
        for agent in crate::auto::Agent::ALL {
            if !crate::sync::API_KEY_AGENTS.contains(&agent) {
                continue;
            }
            let key = agent.ai().key();
            let mut api = self.ws.settings.api_mode(key);
            row(
                ui,
                &crate::trf!("{} : clé API", "{}: API key", agent.label()),
                crate::tr!(
                    "Désactivé : l'IA utilise votre abonnement. Activé : elle utilise la clé API gardée dans le trousseau et la dépense est facturée en dollars.",
                    "Off: the AI uses your subscription. On: it uses the API key kept in the Keychain and the spend is billed in dollars."
                ),
                t,
                |ui| {
                    if switch(ui, &mut api, l).changed() {
                        self.ws.settings.set_api_mode(key, api);
                        self.dirty_since.get_or_insert_with(Instant::now);
                    }
                },
            );
        }
        // The Claude subscription.
        section(
            ui,
            crate::tr!("Abonnement Claude", "Claude subscription"),
            t,
            l,
        );
        let mut on = self.ais.tracking;
        row(
            ui,
            crate::tr!("Afficher l'utilisation", "Show the use"),
            crate::tr!(
                "Session de 5 h et semaine, telles que Claude Code les reçoit. Orchestra devient la ligne d'état de Claude Code (la vôtre continue de s'afficher) ; aucun jeton n'est lu.",
                "5-hour session and week, as Claude Code receives them. Orchestra becomes Claude Code's status line (yours keeps showing); no token is read."
            ),
            t,
            |ui| {
                if switch(ui, &mut on, l).changed() {
                    self.set_tracking(on);
                }
            },
        );
        match &self.ais.usage {
            Some(u) => {
                ui.add_space(4.0);
                super::ai_ui::usage_strip(
                    ui,
                    crate::tr!("Abonnement Claude", "Claude subscription"),
                    u,
                    t,
                );
            }
            None if self.ais.tracking => {
                ui.label(RichText::new(crate::tr!("Les chiffres arrivent à la prochaine réponse de Claude Code (lancez-le depuis un projet).", "The figures arrive with Claude Code's next answer (start it from a project).")).size(13.0).color(t.tertiary));
            }
            None => {}
        }
        // The quota of the other plugged AIs: read when they tell it, else why not.
        let others = self.ais.others.clone();
        if !others.is_empty() {
            section(ui, crate::tr!("Quota des autres IA", "Quota of the other AIs"), t, l);
            for (agent, usage) in &others {
                let title = super::ai_ui::quota_title(*agent);
                match (usage, crate::usage::unreadable_reason(*agent)) {
                    (Some(u), _) => {
                        super::ai_ui::usage_strip(ui, &title, u, t);
                        if *agent == crate::auto::Agent::Vibe
                            && let Some(v) = &self.ais.vibe
                        {
                            super::ai_ui::vibe_details(ui, v, t);
                        }
                    }
                    (None, None)
                        if *agent == crate::auto::Agent::Vibe && self.ais.vibe.is_some() =>
                    {
                        ui.label(RichText::new(&title).size(13.0).color(t.tertiary));
                        if let Some(v) = &self.ais.vibe {
                            super::ai_ui::vibe_details(ui, v, t);
                            super::ai_ui::vibe_budget_note(ui, v, t);
                        }
                    }
                    (None, why) => {
                        let why = why.map(str::to_string).unwrap_or_else(|| {
                            crate::trf!(
                                "Les chiffres arrivent à la prochaine réponse de {}.",
                                "The figures arrive with {}'s next answer.",
                                agent.label()
                            )
                        });
                        ui.label(
                            RichText::new(format!("{title} — {why}"))
                                .size(13.0)
                                .color(t.tertiary),
                        );
                    }
                }
                ui.add_space(4.0);
            }
        }
        if let Some((ai, on)) = toggle {
            self.set_ai(&ctx, ai, on);
        }
        if let Some(ai) = install {
            self.watch_install(ai);
        }
        if sign_in {
            self.watch_sign_in();
        }
        if let Some((title, cmd, input)) = term {
            let home = dirs::home_dir()
                .map(|h| h.display().to_string())
                .unwrap_or_default();
            super::term_ui::request_with_input(&ctx, &title, &home, Some(cmd), input);
            self.settings_tab = None;
        }
    }

    fn tab_collab(&mut self, ui: &mut egui::Ui, t: &Tone, l: &Look) {
        page_title(
            ui,
            crate::tr!("Collaboration", "Collaboration"),
            crate::tr!(
                "Votre propre branchement, sans serveur imposé",
                "Your own connection, no server imposed"
            ),
            t,
        );
        section(ui, crate::tr!("Branchement", "Connection"), t, l);
        let kinds = [
            ConnectorKind::None,
            ConnectorKind::Live,
        ];
        let cur = kinds
            .iter()
            .position(|k| *k == self.ws.settings.collab.connector)
            .unwrap_or(0);
        row(
            ui,
            crate::tr!("Mode", "Mode"),
            crate::tr!(
                "Chaque bloc garde sa modification la plus récente ; ce qui est supprimé le reste chez tous.",
                "Each block keeps its latest change; what is deleted stays deleted for everyone."
            ),
            t,
            |ui| {
                if let Some(i) = dropdown(
                    ui,
                    "connector",
                    &[
                        crate::tr!("Aucun (travail seul)", "None (working alone)"),
                        crate::tr!("En direct (temps réel)", "Live (real time)"),
                    ],
                    cur,
                    t,
                    l,
                ) {
                    if kinds[i] != ConnectorKind::Live {
                        self.stop_live();
                    }
                    self.ws.settings.collab.connector = kinds[i].clone();
                }
            },
        );
        row(
            ui,
            crate::tr!("Votre nom", "Your name"),
            crate::tr!(
                "Vu par les autres à côté des pages que vous regardez.",
                "Seen by the others next to the pages you are looking at."
            ),
            t,
            |ui| {
                ui.add(field(
                    &mut self.ws.settings.collab.author,
                    crate::tr!("ex. Camille", "e.g. Camille"),
                    220.0,
                    t,
                    l,
                ));
            },
        );
        match self.ws.settings.collab.connector {
            ConnectorKind::Live => self.live_settings(ui, t, l),
            ConnectorKind::None => {}
        }
    }

    /// The real-time part: host here or join, the shared code, start/stop, who is connected.
    fn live_settings(&mut self, ui: &mut egui::Ui, t: &Tone, l: &Look) {
        section(ui, crate::tr!("En direct", "Live"), t, l);
        let running = self.live.is_some();
        let role = if self.ws.settings.collab.live_host {
            0
        } else {
            1
        };
        row(
            ui,
            crate::tr!("Rôle", "Role"),
            crate::tr!(
                "Une personne héberge (son ordinateur doit rester allumé), les autres la rejoignent.",
                "One person hosts (their computer must stay on), the others join them."
            ),
            t,
            |ui| {
                ui.add_enabled_ui(!running, |ui| {
                    if let Some(i) = dropdown(
                        ui,
                        "live-role",
                        &[
                            crate::tr!("Héberger sur cet ordinateur", "Host on this computer"),
                            crate::tr!("Rejoindre quelqu'un", "Join someone"),
                        ],
                        role,
                        t,
                        l,
                    ) {
                        self.ws.settings.collab.live_host = i == 0;
                    }
                });
            },
        );
        if self.ws.settings.collab.live_host {
            let mut port = if self.ws.settings.collab.live_port == 0 {
                crate::live::DEFAULT_PORT.to_string()
            } else {
                self.ws.settings.collab.live_port.to_string()
            };
            row(
                ui,
                crate::tr!("Port", "Port"),
                crate::tr!(
                    "7878 par défaut. À ouvrir dans le pare-feu si on vous le demande.",
                    "7878 by default. Allow it in the firewall if asked."
                ),
                t,
                |ui| {
                    if ui
                        .add_enabled(!running, field(&mut port, "7878", 90.0, t, l))
                        .changed()
                    {
                        self.ws.settings.collab.live_port = port.trim().parse().unwrap_or(0);
                    }
                },
            );
        } else {
            row(
                ui,
                crate::tr!("Adresse de l'hôte", "Host's address"),
                crate::tr!(
                    "Celle que l'hôte voit dans ses réglages, par exemple 192.168.1.20:7878.",
                    "The one the host sees in their settings, e.g. 192.168.1.20:7878."
                ),
                t,
                |_| {},
            );
            ui.add_enabled(
                !running,
                field(
                    &mut self.ws.settings.collab.live_address,
                    "192.168.1.20:7878",
                    f32::INFINITY,
                    t,
                    l,
                ),
            );
            ui.add_space(6.0);
        }
        row(
            ui,
            crate::tr!("Code d'accès", "Access code"),
            crate::tr!(
                "Le même pour tous ; sans lui, personne ne peut se connecter.",
                "The same for everyone; without it, nobody can connect."
            ),
            t,
            |ui| {
                if !running
                    && self.ws.settings.collab.live_host
                    && ui
                        .add(
                            egui::Button::new(
                                RichText::new(crate::tr!("Générer", "Generate")).size(13.0),
                            )
                            .frame_when_inactive(false),
                        )
                        .clicked()
                {
                    self.ws.settings.collab.live_code = crate::model::new_id()[..8].to_string();
                }
                ui.add_enabled(
                    !running,
                    field(
                        &mut self.ws.settings.collab.live_code,
                        crate::tr!("un code partagé", "a shared code"),
                        160.0,
                        t,
                        l,
                    ),
                );
            },
        );
        let status = self.live.as_ref().map(|x| x.status());
        let desc = match &status {
            None => crate::tr!("Arrêté.", "Stopped.").to_string(),
            Some(s) if s.hosting => crate::trf!(
                "Hébergé à l'adresse {} · {} personne(s) connectée(s)",
                "Hosted at {} · {} person(s) connected",
                s.address,
                s.peers
            ),
            Some(s) if s.connected => crate::trf!("Connecté à {}", "Connected to {}", s.address),
            Some(s) => crate::trf!(
                "En attente de {} : {}",
                "Waiting for {}: {}",
                s.address,
                s.error
                    .clone()
                    .unwrap_or_else(|| crate::tr!("connexion…", "connecting…").into())
            ),
        };
        let mut toggle = false;
        row(
            ui,
            if running {
                crate::tr!("En marche", "On")
            } else {
                crate::tr!("Démarrer", "Start")
            },
            &desc,
            t,
            |ui| {
                if button(
                    ui,
                    if running {
                        crate::tr!("Arrêter", "Stop")
                    } else {
                        crate::tr!("Démarrer", "Start")
                    },
                    t,
                    l,
                )
                .clicked()
                {
                    toggle = true;
                }
            },
        );
        if toggle {
            if running {
                self.stop_live();
            } else {
                self.start_live();
            }
        }
        if let Some(s) = &status
            && s.hosting
        {
            let addr = s.address.clone();
            let code = self.ws.settings.collab.live_code.clone();
            row(
                ui,
                crate::tr!("À donner aux autres", "To give to the others"),
                &crate::trf!(
                    "Adresse {addr} · code {code}",
                    "Address {addr} · code {code}"
                ),
                t,
                |ui| {
                    if button(ui, crate::tr!("Copier", "Copy"), t, l).clicked() {
                        ui.ctx().copy_text(crate::trf!(
                            "Adresse : {addr}\nCode : {code}",
                            "Address: {addr}\nCode: {code}"
                        ));
                        self.ack(crate::tr!(
                            "Adresse et code copiés.",
                            "Address and code copied."
                        ));
                    }
                },
            );
        }
        if !self.peers.is_empty() {
            section(ui, crate::tr!("Connectés", "Connected"), t, l);
            let titles: Vec<(String, String)> = self
                .peers
                .iter()
                .map(|(n, p)| {
                    (
                        n.clone(),
                        p.page
                            .as_ref()
                            .and_then(|id| self.ws.page(id))
                            .map(|pg| crate::trf!("sur « {} »", "on « {} »", pg.display_title()))
                            .unwrap_or_default(),
                    )
                })
                .collect();
            for (name, where_) in titles {
                row(ui, &name, &where_, t, |_| {});
            }
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(crate::tr!("La connexion est chiffrée avec le code : sans lui, rien n'est lisible. Sur un même réseau (maison, bureau) cela marche tel quel. À distance, reliez vos ordinateurs avec un réseau privé (Tailscale, ZeroTier, WireGuard…) ou lancez « orchestra serveur » sur une machine à vous. Choisissez un code d'au moins 3 mots.", "The connection is encrypted with the code: without it, nothing can be read. On the same network (home, office) it works as is. From afar, link your computers with a private network (Tailscale, ZeroTier, WireGuard…) or run « orchestra serveur » on a machine of yours. Choose a code of at least 3 words."))
                .size(13.0)
                .color(t.tertiary),
        );
    }
}

#[cfg(test)]
mod third_party_tests {
    /// The licences shown in Settings › General are those shipped: RTK's Apache-2.0 text, and
    /// an attribution naming the very version packaging/build-rtk.sh builds, without its engine.
    #[test]
    fn the_shipped_rtk_is_attributed() {
        let [notes, licence] = super::THIRD_PARTY_TEXTS else { panic!() };
        assert!(licence.trim_start().starts_with("Apache License"));
        assert!(licence.contains("Version 2.0, January 2004"));
        let script = include_str!("../../packaging/build-rtk.sh");
        let rev = script
            .lines()
            .find_map(|l| l.strip_prefix("RTK_REV=\""))
            .and_then(|r| r.strip_suffix('"'))
            .unwrap();
        assert!(notes.contains(rev), "attribution for another RTK version than {rev}");
        assert!(script.contains("--no-default-features"));
        for w in ["Apache License 2.0", "RTK AI Labs", "rtk-LICENSE.txt", "BUSL-1.1"] {
            assert!(notes.contains(w), "{w}");
        }
    }
}
