//! The terminal dock at the bottom of the window: one tab per terminal, the grid drawn
//! cell by cell, keys sent to the program while the terminal has focus.

use super::*;
use crate::terminal::{Term, key_bytes, palette};

/// "Open a terminal" asked from anywhere in the window (a button deep in a panel stores it,
/// the app picks it up at the end of the frame).
#[derive(Clone, Default)]
pub struct TermRequest {
    pub title: String,
    pub cwd: String,
    pub cmd: Option<String>,
    /// Typed into the program once it has started.
    pub input: Option<String>,
}

fn request_key() -> egui::Id {
    egui::Id::unique("term-request")
}

pub fn request(ctx: &egui::Context, title: &str, cwd: &str, cmd: Option<String>) {
    request_with_input(ctx, title, cwd, cmd, None);
}

pub fn request_with_input(ctx: &egui::Context, title: &str, cwd: &str, cmd: Option<String>, input: Option<String>) {
    ctx.data_mut(|d| d.insert_temp(request_key(), TermRequest { title: title.into(), cwd: cwd.into(), cmd, input }));
}

const FONT: f32 = 13.0;

fn rgb((r, g, b): (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn color(c: vt100::Color, default: Color32, dark: bool) -> Color32 {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => rgb(palette(i, dark)),
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

impl NotesApp {
    /// Start the terminal a button asked for, if any.
    pub(super) fn take_term_request(&mut self, ctx: &egui::Context) {
        // Text waiting for a program to start: typed after a few seconds.
        for term in self.terms.iter_mut() {
            if let Some((at, text)) = term.pending.take() {
                if at.elapsed().as_secs_f32() >= 4.0 {
                    term.paste(&text);
                    term.write(b"\r");
                } else {
                    term.pending = Some((at, text));
                    ctx.request_repaint_after(std::time::Duration::from_millis(500));
                }
            }
        }
        let Some(r) = ctx.data_mut(|d| d.remove_temp::<TermRequest>(request_key())) else { return };
        let c2 = ctx.clone();
        match Term::spawn(&r.title, &r.cwd, r.cmd.as_deref(), move || c2.request_repaint()) {
            Ok(mut t) => {
                if let Some(text) = r.input.clone() {
                    t.pending = Some((std::time::Instant::now(), text));
                    ctx.request_repaint_after(std::time::Duration::from_millis(500));
                }
                self.terms.push(t);
                self.term_active = self.terms.len() - 1;
                self.term_open = true;
                self.term_focus = true;
            }
            Err(e) => self.fail(crate::trf!("Impossible d'ouvrir le terminal : {e}", "Couldn't open the terminal: {e}")),
        }
    }

    /// Reopen the terminals that were open when the app was last closed.
    pub(super) fn restore_terminals(&mut self, ctx: &egui::Context) {
        for saved in self.ws.settings.open_terms.clone() {
            let c2 = ctx.clone();
            let cmd = crate::terminal::restore_cmd(saved.cmd.as_deref());
            if let Ok(mut t) = Term::spawn(&saved.title, &saved.cwd, cmd.as_deref(), move || c2.request_repaint()) {
                // Remember what it was first started with, not the resume command.
                t.cmd = saved.cmd.clone();
                self.terms.push(t);
            }
        }
    }

    /// Keep the list of open terminals in the settings, so they come back next time.
    pub(super) fn remember_terminals(&mut self) {
        let now: Vec<crate::model::SavedTerm> =
            self.terms.iter().map(|t| crate::model::SavedTerm { title: t.title.clone(), cwd: t.cwd.clone(), cmd: t.cmd.clone() }).collect();
        if now != self.ws.settings.open_terms {
            self.ws.settings.open_terms = now;
        }
    }

    /// The dock (bottom panel), when a terminal is open.
    pub(super) fn terminal_dock(&mut self, ui: &mut egui::Ui, page_fill: Color32) {
        if self.terms.is_empty() || !self.term_open {
            return;
        }
        let dark = ui.visuals().dark_mode;
        let t = tone(dark);
        let bg = if dark { Color32::from_rgb(24, 24, 24) } else { Color32::from_rgb(252, 252, 251) };
        let dock = egui::Panel::bottom("terminal-dock")
            .resizable(true)
            .default_size(340.0)
            .size_range(140.0..=900.0)
            .frame(egui::Frame::new().fill(bg).stroke(egui::Stroke::new(1.0, t.border)))
            .show(ui, |ui| {
                // Tabs.
                let mut close = None;
                egui::Frame::new().fill(page_fill).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, term) in self.terms.iter().enumerate() {
                            let sel = i == self.term_active;
                            let label = if term.exited() { crate::trf!("{} (terminé)", "{} (ended)", term.title) } else { term.title.clone() };
                            let r = ui.add(egui::Button::new(RichText::new(label).size(12.5)).selected(sel).frame(sel));
                            if r.clicked() {
                                self.term_active = i;
                                self.term_focus = true;
                            }
                            r.on_hover_text(&term.cwd);
                            if super::icons::delete_cross(ui, 18.0, t.tertiary, t.text, t.selected, crate::tr!("Fermer ce terminal", "Close this terminal")).clicked() {
                                close = Some(i);
                            }
                            ui.add_space(6.0);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new(RichText::new(crate::tr!("▾ Réduire", "▾ Collapse")).size(12.0)).frame(false)).clicked() {
                                self.term_open = false;
                            }
                            let has_sel = self.terms.get(self.term_active).is_some_and(|t| t.selected_text().is_some());
                            let label = if has_sel { crate::tr!("Copier la sélection", "Copy the selection") } else { crate::tr!("Copier l'écran", "Copy the screen") };
                            if ui.add(egui::Button::new(RichText::new(label).size(12.0)).frame(false)).on_hover_text(crate::tr!("Ou sélectionnez à la souris puis Cmd+C", "Or select with the mouse then Cmd+C")).clicked()
                                && let Some(term) = self.terms.get(self.term_active) {
                                    ui.ctx().copy_text(term.selected_text().unwrap_or_else(|| term.text()));
                                    self.ack(if has_sel { crate::tr!("Sélection copiée.", "Selection copied.") } else { crate::tr!("Contenu du terminal copié.", "Terminal content copied.") });
                                }
                            if ui.add(egui::Button::new(RichText::new("+ Shell").size(12.0)).frame(false)).on_hover_text(crate::tr!("Nouveau terminal", "New terminal")).clicked() {
                                let cwd = self.terms.get(self.term_active).map(|t| t.cwd.clone()).unwrap_or_default();
                                request(ui.ctx(), "Shell", &cwd, None);
                            }
                        });
                    });
                });
                if let Some(i) = close {
                    self.terms.remove(i);
                    if self.term_active >= self.terms.len() {
                        self.term_active = self.terms.len().saturating_sub(1);
                    }
                    return;
                }
                let focus = std::mem::take(&mut self.term_focus);
                if let Some(term) = self.terms.get_mut(self.term_active) {
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| terminal_view(ui, term, bg, dark, focus));
                }
            });
        self.dock_h = dock.response.rect.height();
    }
}

/// Draw one terminal and feed it the keyboard while it has focus.
pub(super) fn terminal_view(ui: &mut egui::Ui, term: &mut Term, bg: Color32, dark: bool, focus: bool) {
    let font = FontId::monospace(FONT);
    let cw = ui.fonts_mut(|f| f.glyph_width(&font, 'M')).max(1.0);
    let rh = (FONT * 1.32).ceil();
    let avail = ui.available_size();
    let cols = (avail.x / cw).floor().max(10.0) as u16;
    let rows = (avail.y / rh).floor().max(2.0) as u16;
    term.resize(rows, cols);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(cols as f32 * cw, rows as f32 * rh), egui::Sense::click_and_drag());
    // Mouse selection: drag over the cells, Cmd+C (or "Copier") copies it.
    let cell_at = |p: egui::Pos2| -> (u16, u16) {
        let c = ((p.x - rect.left()) / cw).floor().clamp(0.0, cols as f32 - 1.0) as u16;
        let r = ((p.y - rect.top()) / rh).floor().clamp(0.0, rows as f32 - 1.0) as u16;
        (r, c)
    };
    if resp.drag_started() {
        if let Some(p) = resp.interact_pointer_pos() {
            let c = cell_at(p);
            term.selection = Some((c, c));
        }
    } else if resp.dragged() {
        if let (Some(p), Some((start, _))) = (resp.interact_pointer_pos(), term.selection) {
            term.selection = Some((start, cell_at(p)));
        }
    } else if resp.clicked() {
        term.selection = None;
    }
    if resp.double_clicked() {
        // Double click: the word under the pointer.
        if let Some(p) = resp.interact_pointer_pos() {
            let (r, c) = cell_at(p);
            let word = |col: u16| term.with_screen(|s| s.cell(r, col).map(|x| x.contents().chars().any(|ch| !ch.is_whitespace())).unwrap_or(false));
            if word(c) {
                let (mut a, mut b) = (c, c);
                while a > 0 && word(a - 1) {
                    a -= 1;
                }
                while b + 1 < cols && word(b + 1) {
                    b += 1;
                }
                term.selection = Some(((r, a), (r, b)));
            }
        }
    }
    let id = resp.id;
    if resp.clicked() || focus {
        resp.request_focus();
    }
    let has_focus = resp.has_focus();
    ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));

    // Keyboard.
    if has_focus {
        let app_cursor = term.with_screen(|s| s.application_cursor());
        let events = ui.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::Copy => {
                    if let Some(txt) = term.selected_text() {
                        ui.ctx().copy_text(txt);
                    }
                }
                egui::Event::Text(s) => term.write(s.as_bytes()),
                egui::Event::Paste(s) => term.paste(&s),
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    if modifiers.mac_cmd || modifiers.command && !modifiers.ctrl {
                        continue; // Cmd+… stays the app's (copy, paste, tabs…).
                    }
                    if let Some(b) = key_bytes(key, modifiers.ctrl, modifiers.alt, modifiers.shift, app_cursor) {
                        term.write(&b);
                    }
                }
                _ => {}
            }
        }
    }
    // Wheel: to the program when it handles the mouse (Claude Code), else our history.
    // Taken from the input so that a page around doesn't scroll as well.
    if ui.rect_contains_pointer(rect) {
        let dy = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta.y));
        if dy.abs() > 0.5 {
            let acc = ui.ctx().data(|d| d.get_temp::<f32>(id).unwrap_or(0.0)) + dy;
            let lines = (acc / rh).trunc();
            ui.ctx().data_mut(|d| d.insert_temp(id, acc - lines * rh));
            if lines != 0.0 {
                let (r, c) = ui.input(|i| i.pointer.hover_pos()).map(cell_at).unwrap_or((0, 0));
                term.wheel(lines as i32, r, c);
            }
        }
    }

    // Grid.
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, bg);
    let fg_default = if dark { Color32::from_rgb(222, 222, 220) } else { Color32::from_rgb(40, 40, 38) };
    let sel = term.selection.map(crate::terminal::ordered).filter(|(a, b)| a != b);
    let in_sel = |r: u16, c: u16| sel.is_some_and(|(a, b)| (r, c) >= a && (r, c) <= b);
    let sel_color = Color32::from_rgba_unmultiplied(35, 131, 226, 90);
    term.with_screen(|s| {
        let (srows, scols) = s.size();
        let (srows, scols) = (srows.get(), scols.get());
        // Cells are grouped into runs of plain ASCII text with the same style: one text
        // shape per run instead of one per cell keeps the dock cheap to draw.
        let mut run = String::new();
        let mut run_start = 0u16;
        let mut run_style: Option<(Color32, bool, bool, bool)> = None;
        let flush = |run: &mut String, start: u16, style: Option<(Color32, bool, bool, bool)>, y: f32| {
            if run.trim().is_empty() {
                run.clear();
                return;
            }
            let Some((fg, bold, italic, underline)) = style else { return };
            let mut fmt = egui::TextFormat::simple(font.clone(), fg);
            fmt.italics = italic;
            if underline {
                fmt.underline = egui::Stroke::new(1.0, fg);
            }
            let mut job = egui::text::LayoutJob::single_section(std::mem::take(run), fmt);
            job.wrap.max_width = f32::INFINITY;
            let g = ui.fonts_mut(|f| f.layout_job(job));
            let pos = egui::pos2(rect.left() + start as f32 * cw, y + (rh - g.size().y) / 2.0);
            painter.galley(pos, g.clone(), fg);
            if bold {
                painter.galley(pos + egui::vec2(0.6, 0.0), g, fg);
            }
        };
        for r in 0..srows.min(rows) {
            let y = rect.top() + r as f32 * rh;
            let mut c = 0u16;
            while c < scols.min(cols) {
                let Some(cell) = s.cell(r, c) else { break };
                let mut fg = color(cell.fgcolor(), fg_default, dark);
                let mut bgc = color(cell.bgcolor(), Color32::TRANSPARENT, dark);
                if cell.inverse() {
                    let f = fg;
                    fg = if bgc == Color32::TRANSPARENT { bg } else { bgc };
                    bgc = f;
                }
                if cell.dim() {
                    fg = fg.gamma_multiply(0.6);
                }
                let w = if cell.is_wide() { 2.0 } else { 1.0 };
                let x = rect.left() + c as f32 * cw;
                let cell_rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(cw * w, rh));
                if bgc != Color32::TRANSPARENT {
                    painter.rect_filled(cell_rect, 0.0, bgc);
                }
                if in_sel(r, c) {
                    painter.rect_filled(cell_rect, 0.0, sel_color);
                }
                let txt = cell.contents();
                let style = (fg, cell.bold(), cell.italic(), cell.underline());
                let ascii = !cell.is_wide() && (txt.is_empty() || (txt.len() == 1 && txt.as_bytes()[0].is_ascii_graphic()) || txt == " ");
                if ascii {
                    if run_style != Some(style) {
                        flush(&mut run, run_start, run_style, y);
                        run_start = c;
                        run_style = Some(style);
                    }
                    run.push_str(if txt.is_empty() { " " } else { txt });
                } else {
                    flush(&mut run, run_start, run_style, y);
                    run_style = None;
                    run_start = c + 1;
                    let one = txt.chars().count() == 1;
                    if one && draw_block_char(&painter, cell_rect, txt.chars().next().unwrap(), fg) {
                        // Box-drawing and block characters are drawn as shapes: the font may
                        // lack them, and drawn they join up exactly from one cell to the next.
                    } else if !txt.is_empty() && !cell.is_wide_continuation() {
                        let mut tmp = txt.to_string();
                        flush(&mut tmp, c, Some(style), y);
                    }
                }
                c += if cell.is_wide() { 2 } else { 1 };
            }
            flush(&mut run, run_start, run_style, y);
            run_style = None;
        }
        // Cursor (only on the live screen).
        if s.scrollback() == 0 && !s.hide_cursor() {
            let (cr, cc) = s.cursor_position();
            if cr < rows && cc < cols {
                let r = egui::Rect::from_min_size(egui::pos2(rect.left() + cc as f32 * cw, rect.top() + cr as f32 * rh), egui::vec2(cw, rh));
                if has_focus {
                    painter.rect_filled(r, 1.0, fg_default.gamma_multiply(0.75));
                } else {
                    painter.rect_stroke(r, 1.0, egui::Stroke::new(1.0, fg_default.gamma_multiply(0.6)), egui::StrokeKind::Inside);
                }
            }
        }
        if s.scrollback() > 0 {
            let label = crate::trf!("historique : {} ligne{p} plus haut", "history: {} line{p} up", s.scrollback(), p = crate::i18n::plural(s.scrollback()));
            painter.text(rect.right_top() + egui::vec2(-8.0, 4.0), egui::Align2::RIGHT_TOP, label, FontId::proportional(11.5), Color32::from_rgb(35, 131, 226));
        }
    });
    if !has_focus {
        resp.on_hover_text(crate::tr!("Cliquer pour écrire dans le terminal", "Click to type in the terminal"));
    }
}

/// Draw box-drawing (─ │ ╭ ┼ …) and block (█ ▀ ▄ ░ …) characters as shapes.
/// Returns false for any other character.
fn draw_block_char(p: &egui::Painter, r: egui::Rect, ch: char, color: Color32) -> bool {
    // (left, right, up, down): which arms of the cell are drawn; 2 = heavy / double.
    let arms: Option<(u8, u8, u8, u8)> = match ch {
        '─' | '╌' | '┄' | '┈' | '╴' | '╶' => Some((1, 1, 0, 0)),
        '━' | '╍' | '┅' | '┉' => Some((2, 2, 0, 0)),
        '═' => Some((2, 2, 0, 0)),
        '│' | '╎' | '┆' | '┊' => Some((0, 0, 1, 1)),
        '┃' | '║' | '╏' | '┇' | '┋' => Some((0, 0, 2, 2)),
        '┌' | '╭' => Some((0, 1, 0, 1)),
        '┐' | '╮' => Some((1, 0, 0, 1)),
        '└' | '╰' => Some((0, 1, 1, 0)),
        '┘' | '╯' => Some((1, 0, 1, 0)),
        '┏' | '╔' => Some((0, 2, 0, 2)),
        '┓' | '╗' => Some((2, 0, 0, 2)),
        '┗' | '╚' => Some((0, 2, 2, 0)),
        '┛' | '╝' => Some((2, 0, 2, 0)),
        '├' => Some((0, 1, 1, 1)),
        '┤' => Some((1, 0, 1, 1)),
        '┬' => Some((1, 1, 0, 1)),
        '┴' => Some((1, 1, 1, 0)),
        '┼' => Some((1, 1, 1, 1)),
        '╠' | '┣' => Some((0, 2, 2, 2)),
        '╣' | '┫' => Some((2, 0, 2, 2)),
        '╦' | '┳' => Some((2, 2, 0, 2)),
        '╩' | '┻' => Some((2, 2, 2, 0)),
        '╬' | '╋' => Some((2, 2, 2, 2)),
        _ => None,
    };
    if let Some((l, rr, u, d)) = arms {
        let c = egui::pos2(r.center().x.round() + 0.5, r.center().y.round() + 0.5);
        let w = |k: u8| if k == 2 { 2.0 } else { 1.0 };
        if l > 0 {
            p.line_segment([egui::pos2(r.left(), c.y), c], egui::Stroke::new(w(l), color));
        }
        if rr > 0 {
            p.line_segment([c, egui::pos2(r.right(), c.y)], egui::Stroke::new(w(rr), color));
        }
        if u > 0 {
            p.line_segment([egui::pos2(c.x, r.top()), c], egui::Stroke::new(w(u), color));
        }
        if d > 0 {
            p.line_segment([c, egui::pos2(c.x, r.bottom())], egui::Stroke::new(w(d), color));
        }
        return true;
    }
    let (x0, x1, y0, y1) = (r.left(), r.right(), r.top(), r.bottom());
    let (xm, ym) = (r.center().x, r.center().y);
    let rect = |a: f32, b: f32, c2: f32, d2: f32, col: Color32| {
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(a, c2), egui::pos2(b, d2)), 0.0, col);
    };
    match ch {
        '█' => rect(x0, x1, y0, y1, color),
        '▀' => rect(x0, x1, y0, ym, color),
        '▄' => rect(x0, x1, ym, y1, color),
        '▌' => rect(x0, xm, y0, y1, color),
        '▐' => rect(xm, x1, y0, y1, color),
        '░' => rect(x0, x1, y0, y1, color.gamma_multiply(0.25)),
        '▒' => rect(x0, x1, y0, y1, color.gamma_multiply(0.5)),
        '▓' => rect(x0, x1, y0, y1, color.gamma_multiply(0.75)),
        '▘' => rect(x0, xm, y0, ym, color),
        '▝' => rect(xm, x1, y0, ym, color),
        '▖' => rect(x0, xm, ym, y1, color),
        '▗' => rect(xm, x1, ym, y1, color),
        _ => return false,
    };
    true
}
