//! The comments of a page (`model::Comment`): the threads shown in the margin, each next to
//! its block, with a field to answer and « Résoudre ». A window too narrow for the margin
//! shows a bubble on each commented block instead, which opens its threads over the page.
//! A comment is started with the bubble shown beside the block under the pointer, with
//! « Commenter » in the block's menu, or with ⌘⇧M.

use super::chrome::relative_time;
use super::projects::{action_link, quiet_link};
use super::*;
use crate::model::Comment;
use crate::project::ProjectMember;

/// A thread card's width in the margin and over the page.
const CARD: f32 = 280.0;
/// Between the text column and the cards: where a block's bubble button sits.
const GAP: f32 = 34.0;
/// What the margin takes right of the column, with room for the scroll bar.
const MARGIN: f32 = GAP + CARD + 12.0;
/// The least the page's column narrows to, to leave room for the margin.
pub(super) const MIN_TEXT: f32 = 420.0;
/// The same for a project page, whose cards need more room.
pub(super) const MIN_PROJECT: f32 = 760.0;

/// Who looks at the comments.
#[derive(Clone, Debug, Default)]
pub(super) struct Who {
    pub name: String,
    /// The member id in a team space; empty elsewhere.
    pub id: String,
    /// The members of the page's project, to show their current names.
    pub members: Vec<ProjectMember>,
    /// A reader of a team space reads the comments only: the server drops their changes.
    pub can_write: bool,
}

/// Where the page's column goes: the space left of it, its width, and whether the margin
/// has room for the cards. `margin` and `width` are the column as it is without comments.
/// The column moves left, then narrows (not under `min_width`); failing that, it stays and
/// there is no margin.
pub(super) fn place(avail: f32, margin: f32, width: f32, min_width: f32) -> (f32, f32, bool) {
    if avail - margin - width >= MARGIN {
        return (margin, width, true);
    }
    let left = avail - width - MARGIN;
    if left >= 10.0 {
        return (left, width, true);
    }
    let narrow = avail - 10.0 - MARGIN;
    if narrow >= min_width.min(width) {
        return (10.0, narrow, true);
    }
    (margin, width, false)
}

fn composing_key(page: &str) -> egui::Id {
    egui::Id::unique(("comment-new", page))
}

/// The block whose bubble was opened (narrow window).
fn open_key(page: &str) -> egui::Id {
    egui::Id::unique(("comment-open", page))
}

fn resolved_key(page: &str) -> egui::Id {
    egui::Id::unique(("comment-resolved", page))
}

fn focus_key() -> egui::Id {
    egui::Id::unique("comment-focus")
}

/// What is being typed: under a thread (`key`: its first comment) or for a new one (the block).
fn draft_key(key: &str) -> egui::Id {
    egui::Id::unique(("comment-draft", key))
}

/// Start a comment on a block: its field shows next to it and takes the keyboard.
pub(super) fn start(ctx: &egui::Context, page: &str, block: &str) {
    ctx.data_mut(|d| {
        d.insert_temp(composing_key(page), block.to_string());
        d.insert_temp(focus_key(), true);
    });
    ctx.request_repaint();
}

fn stop(ctx: &egui::Context, page: &str) {
    ctx.data_mut(|d| {
        if let Some(block) = d.get_temp::<String>(composing_key(page)) {
            d.remove::<String>(draft_key(&block));
        }
        d.remove::<String>(composing_key(page));
    });
}

/// The block being commented, if the page still has it.
fn composing(ctx: &egui::Context, page: &Page, who: &Who) -> Option<String> {
    ctx.data(|d| d.get_temp::<String>(composing_key(&page.id)))
        .filter(|b| who.can_write && page.blocks.iter().any(|x| &x.id == b))
}

fn shows_resolved(ctx: &egui::Context, page: &str) -> bool {
    ctx.data(|d| d.get_temp::<bool>(resolved_key(page)))
        .unwrap_or(false)
}

/// Whether the margin has something to show.
pub(super) fn wants_margin(ctx: &egui::Context, page: &Page, who: &Who) -> bool {
    let resolved = shows_resolved(ctx, &page.id);
    composing(ctx, page, who).is_some()
        || page
            .comments
            .values()
            .any(|c| !c.deleted && c.reply_to.is_empty() && (resolved || !c.resolved))
}

/// « 2 commentaires résolus », above the text: they are hidden until asked for.
pub(super) fn resolved_link(ui: &mut egui::Ui, page: &Page, gutter: f32, t: &Tone) {
    let n = page.threads().iter().filter(|th| th.first.resolved).count();
    if n == 0 {
        return;
    }
    let on = shows_resolved(ui.ctx(), &page.id);
    let s = crate::i18n::plural(n);
    let text = if on {
        crate::trf!("✓ Masquer les fils résolus ({n})", "✓ Hide resolved threads ({n})")
    } else {
        crate::trf!("✓ {n} fil{s} résolu{s}", "✓ {n} resolved thread{s}")
    };
    ui.horizontal(|ui| {
        ui.add_space(gutter);
        if quiet_link(ui, &text, t).clicked() {
            ui.data_mut(|d| d.insert_temp(resolved_key(&page.id), !on));
            ui.ctx().request_repaint();
        }
    });
}

/// A change asked for by a card, applied once everything is drawn.
#[derive(Debug, PartialEq)]
enum Op {
    Add { block: String, text: String },
    Reply { thread: String, text: String },
    Resolve { thread: String, resolved: bool },
    Delete { id: String },
    Cancel,
}

/// The threads shown on a block (or on no block of the page any more).
struct Spot {
    /// Empty: the threads whose block is gone.
    block: String,
    /// The block's row on screen.
    row: egui::Rect,
    threads: Vec<(Comment, Vec<Comment>)>,
    composing: bool,
}

/// Where each block of the page was drawn this frame (`blocks_view`).
fn rows(ctx: &egui::Context, page: &Page) -> Vec<(String, egui::Rect)> {
    page.blocks
        .iter()
        .filter_map(|b| {
            let key = egui::Id::unique(("block", &b.id)).with("row");
            ctx.data(|d| d.get_temp::<egui::Rect>(key))
                .map(|r| (b.id.clone(), r))
        })
        .collect()
}

/// The spots in the order of the page's blocks; the threads of blocks gone come last.
fn spots(
    page: &Page,
    rows: &[(String, egui::Rect)],
    composing: Option<&str>,
    resolved: bool,
) -> Vec<Spot> {
    let threads: Vec<(Comment, Vec<Comment>)> = page
        .threads()
        .into_iter()
        .filter(|th| resolved || !th.first.resolved)
        .map(|th| {
            (
                th.first.clone(),
                th.replies.into_iter().cloned().collect(),
            )
        })
        .collect();
    let mut out: Vec<Spot> = rows
        .iter()
        .filter_map(|(id, row)| {
            let here: Vec<_> = threads
                .iter()
                .filter(|th| &th.0.block == id)
                .cloned()
                .collect();
            let composing = composing == Some(id.as_str());
            (!here.is_empty() || composing).then(|| Spot {
                block: id.clone(),
                row: *row,
                threads: here,
                composing,
            })
        })
        .collect();
    let orphans: Vec<_> = threads
        .into_iter()
        .filter(|th| !page.blocks.iter().any(|b| b.id == th.0.block))
        .collect();
    if let Some((_, last)) = rows.last()
        && !orphans.is_empty()
    {
        out.push(Spot {
            block: String::new(),
            row: last.translate(egui::vec2(0.0, last.height() + 6.0)),
            threads: orphans,
            composing: false,
        });
    }
    out
}

/// A text field that sends on Enter (Maj+Entrée: new line), keeping its draft under `key`.
/// Returns the text to send.
fn field(ui: &mut egui::Ui, key: &str, hint: &str, take_focus: bool) -> Option<String> {
    let store = draft_key(key);
    let mut draft: String = ui.data(|d| d.get_temp(store)).unwrap_or_default();
    let id = store.with("field");
    // The Enter that sends must not also break the line. (`consume_key` alone would take
    // Maj+Entrée too.)
    let enter = ui.memory(|m| m.has_focus(id))
        && ui.input_mut(|i| {
            !i.modifiers.shift && i.consume_key(egui::Modifiers::NONE, Key::Enter)
        });
    let r = ui.add(
        egui::TextEdit::multiline(&mut draft)
            .id(id)
            .hint_text(hint)
            .font(FontId::proportional(13.0))
            .desired_rows(1)
            .desired_width(f32::INFINITY)
            .return_key(egui::KeyboardShortcut::new(
                egui::Modifiers::SHIFT,
                Key::Enter,
            )),
    );
    if take_focus {
        r.request_focus();
    }
    let send = (enter && !draft.trim().is_empty()).then(|| draft.trim().to_string());
    ui.data_mut(|d| {
        if send.is_some() || draft.is_empty() {
            d.remove::<String>(store);
        } else {
            d.insert_temp(store, draft);
        }
    });
    send
}

/// One comment: who, when, what; its author can delete it.
fn comment_row(
    ui: &mut egui::Ui,
    c: &Comment,
    who: &Who,
    first: bool,
    t: &Tone,
    ops: &mut Vec<Op>,
) {
    let name = c.author_name(&who.members);
    let mine = c.is_by(&who.id, &who.name);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        super::side::face(ui, name);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if mine && who.can_write {
                let more = icons::button(
                    ui,
                    icons::Icon::More,
                    22.0,
                    t.tertiary,
                    t.hover,
                    crate::tr!("Actions du commentaire", "Comment actions"),
                );
                egui::Popup::menu(&more).show(|ui| {
                    let label = if first {
                        crate::tr!("Supprimer le fil", "Delete the thread")
                    } else {
                        crate::tr!("Supprimer la réponse", "Delete the reply")
                    };
                    if ui.button(label).clicked() {
                        ops.push(Op::Delete { id: c.id.clone() });
                        ui.close();
                    }
                });
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add(
                        egui::Label::new(RichText::new(name).font(semibold(13.0)).color(t.text))
                            .truncate(),
                    );
                    ui.label(
                        RichText::new(relative_time(c.at))
                            .size(11.0)
                            .color(t.tertiary),
                    );
                });
            });
        });
    });
    super::rich::markdown(ui, &c.text, 13.0, t);
}

fn card_frame(ui: &mut egui::Ui, t: &Tone, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(t.card)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 5.0;
            add(ui);
        });
}

/// The cards of one spot, one under the other.
fn cards(ui: &mut egui::Ui, spot: &Spot, who: &Who, t: &Tone, ops: &mut Vec<Op>) {
    ui.spacing_mut().item_spacing.y = 8.0;
    for (first, replies) in &spot.threads {
        ui.push_id(&first.id, |ui| {
            card_frame(ui, t, |ui| {
                if spot.block.is_empty() {
                    ui.label(
                        RichText::new(crate::tr!("Bloc supprimé", "Deleted block"))
                            .size(11.5)
                            .italics()
                            .color(t.tertiary),
                    );
                }
                comment_row(ui, first, who, true, t, ops);
                for r in replies {
                    ui.push_id(&r.id, |ui| comment_row(ui, r, who, false, t, ops));
                }
                if first.resolved {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(crate::tr!("✓ Résolu", "✓ Resolved"))
                                .size(12.0)
                                .color(super::style::GREEN),
                        );
                        if who.can_write
                            && action_link(ui, crate::tr!("Rouvrir", "Reopen")).clicked()
                        {
                            ops.push(Op::Resolve {
                                thread: first.id.clone(),
                                resolved: false,
                            });
                        }
                    });
                } else if who.can_write {
                    let hint = crate::tr!("Répondre…", "Reply…");
                    if let Some(text) = field(ui, &first.id, hint, false) {
                        ops.push(Op::Reply {
                            thread: first.id.clone(),
                            text,
                        });
                    }
                    if quiet_link(ui, crate::tr!("✓ Résoudre", "✓ Resolve"), t).clicked() {
                        ops.push(Op::Resolve {
                            thread: first.id.clone(),
                            resolved: true,
                        });
                    }
                }
            });
        });
    }
    if spot.composing {
        card_frame(ui, t, |ui| {
            let take = ui.data_mut(|d| d.remove_temp::<bool>(focus_key())).is_some();
            let hint = crate::tr!(
                "Ajouter un commentaire… (Entrée pour envoyer)",
                "Add a comment… (Enter to send)"
            );
            if let Some(text) = field(ui, &spot.block, hint, take) {
                ops.push(Op::Add {
                    block: spot.block.clone(),
                    text,
                });
            }
            // Échap, only from this field (it has let go of the keyboard by now).
            let field_id = draft_key(&spot.block).with("field");
            let esc = ui.input(|i| i.key_pressed(Key::Escape))
                && ui.memory(|m| m.had_focus_last_frame(field_id));
            if quiet_link(ui, crate::tr!("Annuler", "Cancel"), t).clicked() || esc {
                ops.push(Op::Cancel);
            }
        });
    }
}

/// A speech bubble, with the number of comments when there are some.
fn bubble(
    ui: &mut egui::Ui,
    at: egui::Pos2,
    id: egui::Id,
    count: usize,
    t: &Tone,
    tip: &str,
) -> egui::Response {
    let rect = egui::Rect::from_min_size(at, egui::vec2(26.0, 22.0));
    let resp = ui.interact(rect, id, egui::Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, ui.visuals().panel_fill);
    if resp.hovered() {
        p.rect_filled(rect, 4.0, t.hover);
    }
    let body = egui::Rect::from_center_size(
        rect.center() - egui::vec2(0.0, 1.5),
        egui::vec2(16.0, 12.0),
    );
    let s = egui::Stroke::new(1.3, t.secondary);
    p.rect_stroke(body, 3.0, s, egui::StrokeKind::Inside);
    p.line_segment(
        [
            body.left_bottom() + egui::vec2(4.0, -0.5),
            body.left_bottom() + egui::vec2(3.0, 3.5),
        ],
        s,
    );
    p.line_segment(
        [
            body.left_bottom() + egui::vec2(3.0, 3.5),
            body.left_bottom() + egui::vec2(8.0, -0.5),
        ],
        s,
    );
    if count > 0 {
        p.text(
            body.center(),
            egui::Align2::CENTER_CENTER,
            if count > 9 {
                "9+".to_string()
            } else {
                count.to_string()
            },
            FontId::proportional(9.5),
            t.text,
        );
    }
    let tip = tip.to_string();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
    resp.on_hover_text(&tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Draw the page's comments beside its blocks, once those are drawn. `column`: the left and
/// right edges of the text column; `margin`: the margin has room for the cards.
pub(super) fn show(
    ui: &mut egui::Ui,
    page: &mut Page,
    who: &Who,
    column: egui::Rangef,
    margin: bool,
    t: &Tone,
) {
    let ctx = ui.ctx().clone();
    let rows = rows(&ctx, page);
    // ⌘⇧M: comment the block being written.
    if who.can_write
        && let Some(focused) = ctx.memory(|m| m.focused())
        && let Some((block, _)) = rows
            .iter()
            .find(|(id, _)| egui::Id::unique(("block", id)) == focused)
        && ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::M)
        })
    {
        start(&ctx, &page.id, block);
    }
    let composing = composing(&ctx, page, who);
    let spots = spots(page, &rows, composing.as_deref(), shows_resolved(&ctx, &page.id));
    let mut ops = vec![];
    // Right of the column, but never off screen.
    let bubble_x = (column.max + 4.0).min(ui.clip_rect().right() - 38.0);

    if margin {
        let x = column.max + GAP;
        let mut free = f32::NEG_INFINITY;
        for spot in &spots {
            let top = spot.row.top().max(free);
            let area = egui::Rect::from_min_size(egui::pos2(x, top), egui::vec2(CARD, 100_000.0));
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(area)
                    .layout(egui::Layout::top_down(egui::Align::Min))
                    .id_salt(("comment-spot", &spot.block)),
            );
            child.set_width(CARD);
            cards(&mut child, spot, who, t, &mut ops);
            free = child.min_rect().bottom() + 8.0;
        }
        // The page scrolls down to the last card.
        if free.is_finite() {
            ui.expand_to_include_y(free);
        }
    } else {
        let key = open_key(&page.id);
        let mut open: Option<String> = ctx.data(|d| d.get_temp(key));
        for spot in &spots {
            let count = spot.threads.iter().map(|th| 1 + th.1.len()).sum();
            let at = egui::pos2(bubble_x, spot.row.top() + 1.0);
            let id = egui::Id::unique(("comment-bubble", &page.id, &spot.block));
            let tip = crate::tr!("Voir les commentaires", "See the comments");
            let clicked = bubble(ui, at, id, count, t, tip).clicked();
            if clicked {
                if open.as_deref() == Some(spot.block.as_str()) || spot.composing {
                    open = None;
                    if spot.composing {
                        ops.push(Op::Cancel);
                    }
                    continue;
                }
                open = Some(spot.block.clone());
            }
            let shown = spot.composing
                || (composing.is_none() && open.as_deref() == Some(spot.block.as_str()));
            if !shown {
                continue;
            }
            let screen = ctx.content_rect();
            let pos = egui::pos2(
                (column.max - CARD).min(screen.right() - CARD - 12.0).max(8.0),
                spot.row.bottom().max(at.y + 24.0) + 2.0,
            );
            egui::Area::new(egui::Id::unique(("comment-popup", &page.id)))
                .order(egui::Order::Foreground)
                .fixed_pos(pos)
                .show(&ctx, |ui| {
                    ui.set_width(CARD);
                    egui::ScrollArea::vertical()
                        .max_height((screen.bottom() - pos.y - 12.0).max(180.0))
                        .show(ui, |ui| cards(ui, spot, who, t, &mut ops));
                });
            if !spot.composing && ctx.input(|i| i.key_pressed(Key::Escape)) {
                open = None;
            }
        }
        ctx.data_mut(|d| match open {
            Some(o) => {
                d.insert_temp(key, o);
            }
            None => d.remove::<String>(key),
        });
    }

    // The bubble to comment the block under the pointer (not over one already there).
    if who.can_write
        && let Some(p) = ui.input(|i| i.pointer.hover_pos())
        && ui.clip_rect().contains(p)
        && p.x >= column.min
        && p.x <= bubble_x + 30.0
        && ctx.layer_id_at(p).is_none_or(|l| l == ui.layer_id())
        && let Some((block, row)) = rows
            .iter()
            .find(|(_, r)| p.y >= r.top() - 1.0 && p.y <= r.bottom() + 1.0)
        && (margin || !spots.iter().any(|s| &s.block == block))
    {
        let at = egui::pos2(bubble_x, row.top() + 1.0);
        let id = egui::Id::unique(("comment-add", &page.id));
        let tip = crate::tr!("Commenter (⌘⇧M)", "Comment (⌘⇧M)");
        if bubble(ui, at, id, 0, t, tip).clicked() {
            start(&ctx, &page.id, block);
        }
    }

    if !ops.is_empty() {
        apply(&ctx, page, who, ops);
        ctx.request_repaint();
    }
}

fn apply(ctx: &egui::Context, page: &mut Page, who: &Who, ops: Vec<Op>) {
    let id = page.id.clone();
    for op in ops {
        match op {
            Op::Add { block, text } => {
                page.add_comment(&block, &text, &who.name, &who.id);
                stop(ctx, &id);
                // Its thread stays open over a narrow page.
                ctx.data_mut(|d| d.insert_temp(open_key(&id), block));
            }
            Op::Reply { thread, text } => {
                page.reply_comment(&thread, &text, &who.name, &who.id);
            }
            Op::Resolve { thread, resolved } => {
                page.set_thread_resolved(&thread, resolved);
            }
            Op::Delete { id } => {
                page.delete_comment(&id);
            }
            Op::Cancel => stop(ctx, &id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn the_column_makes_room_for_the_margin() {
        // A wide window: the centred column stays.
        assert_eq!(place(1600.0, 400.0, 752.0, MIN_TEXT), (400.0, 752.0, true));
        // Less room: it moves left.
        let (left, width, margin) = place(1200.0, 200.0, 752.0, MIN_TEXT);
        assert_eq!((width, margin), (752.0, true));
        assert_eq!(left, 1200.0 - 752.0 - MARGIN);
        // Less again: it narrows.
        let (left, width, margin) = place(900.0, 50.0, 752.0, MIN_TEXT);
        assert_eq!((left, margin), (10.0, true));
        assert_eq!(width, 900.0 - 10.0 - MARGIN);
        // Too narrow: nothing moves, the comments go in bubbles.
        assert_eq!(place(700.0, 10.0, 680.0, MIN_TEXT), (10.0, 680.0, false));
    }

    struct S {
        page: Page,
        who: Who,
        margin: bool,
        /// False until the app's fonts are installed.
        draw: bool,
    }

    fn harness(page: Page, margin: bool, can_write: bool) -> Harness<'static, S> {
        let who = Who {
            name: "Moi".into(),
            id: String::new(),
            members: vec![],
            can_write,
        };
        let mut h = Harness::builder().with_size([900.0, 600.0]).build_ui_state(
            |ui, s: &mut S| {
                if !s.draw {
                    return;
                }
                let t = tone(false);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(500.0);
                        resolved_link(ui, &s.page, 44.0, &t);
                        let mut actions = vec![];
                        blocks_view(ui, &mut s.page, &HashMap::new(), None, &mut actions);
                        let column = ui.max_rect().x_range();
                        show(ui, &mut s.page, &s.who, column, s.margin, &t);
                    });
                });
            },
            S {
                page,
                who,
                margin,
                draw: false,
            },
        );
        super::super::setup_fonts(&h.ctx);
        h.run_steps(1);
        h.state_mut().draw = true;
        h.run_steps(3);
        h
    }

    fn page() -> Page {
        let mut p = Page::new("P", None);
        p.blocks = vec![
            Block::new(BlockKind::Paragraph, "Alpha"),
            Block::new(BlockKind::Paragraph, "Beta"),
        ];
        p
    }

    fn send(h: &mut Harness<S>, text: &str) {
        h.event(egui::Event::Text(text.into()));
        h.run_steps(2);
        h.key_press(Key::Enter);
        h.run_steps(3);
    }

    /// A comment started on a block is typed in the margin, answered, resolved (which hides
    /// it), shown again and reopened.
    #[test]
    fn a_thread_is_written_answered_and_resolved_in_the_margin() {
        let p = page();
        let (id, block) = (p.id.clone(), p.blocks[1].id.clone());
        let mut h = harness(p, true, true);
        assert!(!wants_margin(&h.ctx, &h.state().page, &h.state().who));
        start(&h.ctx, &id, &block);
        h.run_steps(3);
        assert!(wants_margin(&h.ctx, &h.state().page, &h.state().who));
        send(&mut h, "Pourquoi ?");
        let threads = h.state().page.threads().len();
        assert_eq!(threads, 1);
        assert_eq!(h.state().page.threads()[0].first.text, "Pourquoi ?");
        assert_eq!(h.state().page.threads()[0].first.block, block);
        // The page's text didn't receive what was typed.
        assert_eq!(h.state().page.blocks[1].text, "Beta");

        let reply = draft_key(&h.state().page.threads()[0].first.id).with("field");
        h.ctx.memory_mut(|m| m.request_focus(reply));
        h.run_steps(2);
        send(&mut h, "Parce que.");
        assert_eq!(h.state().page.threads()[0].replies[0].text, "Parce que.");

        h.get_by_label("✓ Résoudre").click();
        h.run_steps(3);
        assert!(h.state().page.threads()[0].first.resolved);
        assert!(!wants_margin(&h.ctx, &h.state().page, &h.state().who));
        assert!(h.query_by_label("Rouvrir").is_none());

        h.get_by_label("✓ 1 fil résolu").click();
        h.run_steps(3);
        h.get_by_label("Rouvrir").click();
        h.run_steps(3);
        assert!(!h.state().page.threads()[0].first.resolved);
    }

    /// Maj+Entrée breaks the line; Échap gives up the new comment, only from its field.
    #[test]
    fn shift_enter_breaks_the_line_and_escape_gives_up() {
        let p = page();
        let (id, block) = (p.id.clone(), p.blocks[0].id.clone());
        let mut h = harness(p, true, true);
        start(&h.ctx, &id, &block);
        h.run_steps(3);
        h.event(egui::Event::Text("un".into()));
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::SHIFT, Key::Enter);
        h.run_steps(2);
        assert!(h.state().page.threads().is_empty());
        send(&mut h, "deux");
        assert_eq!(h.state().page.threads()[0].first.text, "un\ndeux");

        start(&h.ctx, &id, &block);
        h.run_steps(3);
        let field = draft_key(&block).with("field");
        h.ctx.memory_mut(|m| m.surrender_focus(field));
        h.run_steps(2);
        h.key_press(Key::Escape);
        h.run_steps(3);
        assert!(composing(&h.ctx, &h.state().page, &h.state().who).is_some());
        h.ctx.memory_mut(|m| m.request_focus(field));
        h.run_steps(2);
        h.key_press(Key::Escape);
        h.run_steps(3);
        assert_eq!(composing(&h.ctx, &h.state().page, &h.state().who), None);
        assert_eq!(h.state().page.threads().len(), 1);
    }

    #[test]
    fn a_narrow_page_opens_a_blocks_threads_from_its_bubble() {
        let mut p = page();
        let block = p.blocks[0].id.clone();
        p.add_comment(&block, "ici", "Ana", "").unwrap();
        let mut h = harness(p, false, true);
        assert!(h.query_by_label("✓ Résoudre").is_none());
        h.get_by_label("Voir les commentaires").click();
        h.run_steps(3);
        h.get_by_label("✓ Résoudre").click();
        h.run_steps(3);
        assert!(h.state().page.threads()[0].first.resolved);
    }

    #[test]
    fn a_reader_only_reads() {
        let mut p = page();
        let block = p.blocks[0].id.clone();
        p.add_comment(&block, "ici", "Moi", "").unwrap();
        let id = p.id.clone();
        let h = harness(p, true, false);
        assert!(h.query_by_label("✓ Résoudre").is_none());
        start(&h.ctx, &id, &block);
        assert_eq!(composing(&h.ctx, &h.state().page, &h.state().who), None);
    }

    /// A thread whose block is gone is still shown.
    #[test]
    fn threads_of_a_deleted_block_come_last() {
        let mut p = page();
        let (a, b) = (p.blocks[0].id.clone(), p.blocks[1].id.clone());
        p.add_comment(&a, "sur a", "Moi", "").unwrap();
        p.add_comment(&b, "sur b", "Moi", "").unwrap();
        p.blocks.remove(0);
        let row = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 20.0));
        let got = spots(&p, &[(b.clone(), row)], None, false);
        let blocks: Vec<&str> = got.iter().map(|s| s.block.as_str()).collect();
        assert_eq!(blocks, [b.as_str(), ""]);
        assert_eq!(got[1].threads[0].0.text, "sur a");
        assert!(got[1].row.top() > row.bottom());
    }
}
