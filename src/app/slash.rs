//! The "/" menu, drawn after the reference picture: a floating window about 375 px wide with a
//! heading ("Blocs de base", or "Résultats filtrés" once you type), 32 px rows with a line icon,
//! the name and the Markdown shortcut on the right, the chosen row shaded, a scroll bar past
//! eight rows, and "Fermer le menu · esc" at the bottom.
//! Colours sampled on the reference (dark): window #252525, chosen row #313131, border #383836.

use super::icons::{self, Icon};
use super::*;

/// Block kinds in the order of the "Blocs de base" menu, with their icon and shortcut.
pub const ORDER: [(BlockKind, &str); 13] = [
    (BlockKind::Paragraph, ""),
    (BlockKind::Heading1, "#"),
    (BlockKind::Heading2, "##"),
    (BlockKind::Heading3, "###"),
    (BlockKind::Bullet, "-"),
    (BlockKind::Numbered, "1."),
    (BlockKind::Todo, "[]"),
    (BlockKind::Toggle, ">"),
    (BlockKind::PageLink, ""),
    (BlockKind::Callout, ""),
    (BlockKind::Quote, "\""),
    (BlockKind::Divider, "---"),
    (BlockKind::Code, "```"),
];

pub fn kind_icon(k: BlockKind) -> Icon {
    match k {
        BlockKind::Paragraph => Icon::TextT,
        BlockKind::Heading1 => Icon::Heading(1),
        BlockKind::Heading2 => Icon::Heading(2),
        BlockKind::Heading3 => Icon::Heading(3),
        BlockKind::Todo => Icon::TodoList,
        BlockKind::Bullet => Icon::BulletList,
        BlockKind::Numbered => Icon::NumberList,
        BlockKind::Toggle => Icon::ToggleList,
        BlockKind::Quote => Icon::Quote,
        BlockKind::Callout => Icon::Callout,
        BlockKind::Code => Icon::Code,
        BlockKind::Divider => Icon::Divider,
        BlockKind::PageLink => Icon::Page,
    }
}

/// Kinds matching what was typed after "/", in menu order.
pub fn matches(text: &str) -> Vec<(BlockKind, &'static str)> {
    let q = text.trim_start_matches('/').trim().to_lowercase();
    ORDER
        .into_iter()
        .filter(|(k, _)| q.is_empty() || k.label().to_lowercase().contains(&q) || k.keywords().iter().any(|w| w.starts_with(&q)))
        .collect()
}

pub enum SlashEvent {
    Pick(BlockKind),
    Hover(usize),
    Close,
}

const WIDTH: f32 = 375.0;
const ROW: f32 = 32.0;
const HEAD: f32 = 34.0;
const FOOT: f32 = 41.0;
const MAX_ROWS: f32 = 8.0;

/// Show the menu next to `anchor` (the block's text). Below it when there is room, above otherwise.
pub fn popup(ctx: &egui::Context, id: egui::Id, anchor: egui::Rect, text: &str, selected: usize, scroll_to_selected: bool) -> Option<SlashEvent> {
    let dark = ctx.global_style().visuals.dark_mode;
    let t = tone(dark);
    let (bg, sel_bg) = if dark { (Color32::from_rgb(37, 37, 37), Color32::from_rgb(49, 49, 49)) } else { (Color32::WHITE, Color32::from_rgb(239, 238, 236)) };
    let list = matches(text);
    let rows_h = if list.is_empty() { ROW } else { (list.len() as f32).min(MAX_ROWS + 0.5) * ROW };
    let height = HEAD + rows_h + 8.0 + FOOT;
    let screen = ctx.content_rect();
    let below = anchor.bottom() + 6.0 + height <= screen.bottom() - 8.0;
    let y = if below { anchor.bottom() + 6.0 } else { (anchor.top() - 6.0 - height).max(screen.top() + 8.0) };
    let x = (anchor.left() - 4.0).min(screen.right() - WIDTH - 8.0);
    let mut event = None;
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(x, y))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(bg)
                .stroke(egui::Stroke::new(1.0, t.border))
                .corner_radius(12.0)
                .shadow(ui.visuals().popup_shadow)
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                    let q = text.trim_start_matches('/').trim();
                    let head = if q.is_empty() { crate::tr!("Blocs de base", "Basic blocks") } else { crate::tr!("Résultats filtrés", "Filtered results") };
                    let (hr, _) = ui.allocate_exact_size(egui::vec2(WIDTH, HEAD), egui::Sense::hover());
                    ui.painter().text(egui::pos2(hr.left() + 12.0, hr.center().y + 2.0), egui::Align2::LEFT_CENTER, head, semibold(14.0), t.secondary);
                    egui::ScrollArea::vertical().id_salt(id.with("scroll")).max_height(MAX_ROWS * ROW + ROW / 2.0).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                        if list.is_empty() {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(WIDTH, ROW), egui::Sense::hover());
                            ui.painter().text(egui::pos2(r.left() + 12.0, r.center().y), egui::Align2::LEFT_CENTER, crate::tr!("Aucun résultat", "No results"), FontId::proportional(16.0), t.tertiary);
                        }
                        for (i, (kind, hint)) in list.iter().enumerate() {
                            let (r, resp) = ui.allocate_exact_size(egui::vec2(WIDTH, ROW), egui::Sense::click());
                            let inner = r.shrink2(egui::vec2(4.0, 0.0));
                            if i == selected {
                                ui.painter().rect_filled(inner, 8.0, sel_bg);
                                if scroll_to_selected {
                                    resp.scroll_to_me(None);
                                }
                            }
                            icons::paint(ui.painter(), egui::Rect::from_center_size(egui::pos2(r.left() + 21.0, r.center().y), egui::vec2(18.0, 18.0)), kind_icon(*kind), t.text);
                            ui.painter().text(egui::pos2(r.left() + 40.0, r.center().y), egui::Align2::LEFT_CENTER, kind.label(), FontId::proportional(16.0), t.text);
                            if !hint.is_empty() {
                                ui.painter().text(egui::pos2(r.right() - 30.0, r.center().y), egui::Align2::RIGHT_CENTER, *hint, FontId::proportional(14.0), t.tertiary);
                            }
                            if resp.hovered() && i != selected && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO) {
                                event = Some(SlashEvent::Hover(i));
                            }
                            if resp.clicked() {
                                event = Some(SlashEvent::Pick(*kind));
                            }
                        }
                    });
                    ui.add_space(8.0);
                    let (fr, fresp) = ui.allocate_exact_size(egui::vec2(WIDTH, FOOT), egui::Sense::click());
                    ui.painter().hline(fr.x_range(), fr.top(), egui::Stroke::new(1.0, t.border));
                    if fresp.hovered() {
                        ui.painter().rect_filled(fr.shrink(4.0), 8.0, t.hover);
                    }
                    ui.painter().text(egui::pos2(fr.left() + 12.0, fr.center().y), egui::Align2::LEFT_CENTER, crate::tr!("Fermer le menu", "Close the menu"), FontId::proportional(16.0), t.text);
                    ui.painter().text(egui::pos2(fr.right() - 12.0, fr.center().y), egui::Align2::RIGHT_CENTER, "esc", FontId::proportional(14.0), t.tertiary);
                    if fresp.clicked() {
                        event = Some(SlashEvent::Close);
                    }
                });
        });
    event
}
