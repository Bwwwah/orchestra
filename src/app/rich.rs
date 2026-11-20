//! Drawing AI-written Markdown (state of play, notes) in the app's own style. The text is
//! parsed by `richtext`, so every AI's output looks the same: wrapped to the panel width,
//! headings, lists, to-dos, quotes, code and tables that scroll sideways instead of
//! overflowing.

use super::*;
use crate::richtext::{RBlock, clean, parse};

#[derive(Clone, Copy, Default, PartialEq)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
}

/// Split inline Markdown into styled runs. Styles nest (`**bold with `code`**`), markers
/// without a partner stay as text, and `_` only marks italics at word edges, so
/// `USE_MOCK_MESSAGES` or `snake_case` stay as written.
fn runs(text: &str) -> Vec<(String, Style)> {
    let c: Vec<char> = text.chars().collect();
    let n = c.len();
    let word = |i: usize| i < n && c[i].is_alphanumeric();
    let find = |from: usize, pat: &[char]| (from..n.saturating_sub(pat.len() - 1)).find(|&k| c[k..k + pat.len()] == *pat);
    let mut out: Vec<(String, Style)> = vec![];
    let mut cur = String::new();
    let mut st = Style::default();
    let mut italic_mark = '*';
    let push = |out: &mut Vec<(String, Style)>, cur: &mut String, st: Style| {
        if !cur.is_empty() {
            out.push((std::mem::take(cur), st));
        }
    };
    let mut i = 0;
    while i < n {
        let ch = c[i];
        if ch == '`'
            && let Some(end) = find(i + 1, &['`']).filter(|&e| e > i + 1) {
                push(&mut out, &mut cur, st);
                out.push((c[i + 1..end].iter().collect(), Style { code: true, ..st }));
                i = end + 1;
                continue;
            }
        let mut toggled = false;
        for (pat, bold) in [(['*', '*'], true), (['~', '~'], false)] {
            if i + 1 < n && c[i] == pat[0] && c[i + 1] == pat[1] {
                let on = if bold { st.bold } else { st.strike };
                if on || find(i + 2, &pat).is_some() {
                    push(&mut out, &mut cur, st);
                    if bold {
                        st.bold = !on
                    } else {
                        st.strike = !on
                    }
                    i += 2;
                    toggled = true;
                    break;
                }
            }
        }
        if toggled {
            continue;
        }
        if (ch == '*' && !(i + 1 < n && c[i + 1] == '*')) || ch == '_' {
            let opening = !st.italic
                && i + 1 < n
                && !c[i + 1].is_whitespace()
                && (ch == '*' || i == 0 || !word(i - 1))
                && (i + 2..n).any(|k| c[k] == ch && !c[k - 1].is_whitespace() && (ch == '*' || !word(k + 1)));
            let closing = st.italic && ch == italic_mark && i > 0 && !c[i - 1].is_whitespace() && (ch == '*' || !word(i + 1));
            if opening || closing {
                push(&mut out, &mut cur, st);
                st.italic = !st.italic;
                italic_mark = ch;
                i += 1;
                continue;
            }
        }
        cur.push(ch);
        i += 1;
    }
    push(&mut out, &mut cur, st);
    out
}

/// Inline `**bold**`, `*italic*`, `` `code` ``, `~~strike~~`, markers hidden, with a
/// comfortable line height.
pub(super) fn inline_job(text: &str, size: f32, color: Color32, bold_all: bool, t: &Tone) -> egui::text::LayoutJob {
    let _ = t;
    let mut job = egui::text::LayoutJob::default();
    for (txt, st) in runs(text) {
        let mut f = egui::TextFormat {
            font_id: if bold_all || st.bold { semibold(size) } else { FontId::proportional(size) },
            color,
            line_height: Some((size * 1.5).round()),
            valign: egui::Align::Center,
            ..Default::default()
        };
        f.italics = st.italic;
        if st.strike {
            f.strikethrough = egui::Stroke::new(1.0, color);
        }
        if st.code {
            f.font_id = FontId::monospace(size * 0.86);
            f.color = Color32::from_rgb(224, 108, 108);
            f.background = Color32::from_rgba_unmultiplied(135, 131, 120, 34);
        }
        job.append(&txt, 0.0, f);
    }
    job
}

/// A wrapped paragraph that uses exactly the width left on the line.
fn wrapped(ui: &mut egui::Ui, job: egui::text::LayoutJob) {
    let w = ui.available_width().max(40.0);
    ui.vertical(|ui| {
        ui.set_max_width(w);
        ui.add(egui::Label::new(job).wrap());
    });
}

/// A list line: indent, a marker column, then the wrapped text.
fn list_line(ui: &mut egui::Ui, indent: u8, marker: &str, marker_color: Color32, job: egui::text::LayoutJob, size: f32) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(indent as f32 * 18.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, size * 1.35), egui::Sense::hover());
        ui.painter().text(r.left_center() + egui::vec2(2.0, 0.0), egui::Align2::LEFT_CENTER, marker, FontId::proportional(size), marker_color);
        wrapped(ui, job);
    });
}

/// Draw AI Markdown at `size` px (body text).
pub(super) fn markdown(ui: &mut egui::Ui, md: &str, size: f32, t: &Tone) {
    ui.spacing_mut().item_spacing.y = 3.0;
    // Parsing on every frame is wasted work: keep the last results, keyed by the text.
    let key = egui::Id::unique(("md-cache", md));
    let blocks: std::sync::Arc<Vec<RBlock>> = match ui.ctx().data(|d| d.get_temp::<std::sync::Arc<Vec<RBlock>>>(key)) {
        Some(b) => b,
        None => {
            let mut v = parse(&clean(md, 20_000));
            if matches!(v.first(), Some(RBlock::Heading(1, _))) {
                v.remove(0);
            }
            let v = std::sync::Arc::new(v);
            ui.ctx().data_mut(|d| d.insert_temp(key, v.clone()));
            v
        }
    };
    for (i, b) in blocks.iter().enumerate() {
        match b {
            RBlock::Heading(level, text) => {
                if i > 0 {
                    ui.add_space(12.0);
                }
                let s = match level {
                    1 => size + 4.0,
                    2 => size + 2.0,
                    _ => size + 1.0,
                };
                wrapped(ui, inline_job(text, s, t.text, true, t));
            }
            RBlock::Para(text) => {
                if i > 0 && !matches!(blocks.get(i - 1), Some(RBlock::Heading(..))) {
                    ui.add_space(6.0);
                }
                wrapped(ui, inline_job(text, size, t.text, false, t));
            }
            RBlock::Bullet { indent, text } => {
                let m = if *indent % 2 == 0 { "•" } else { "◦" };
                list_line(ui, *indent, m, t.secondary, inline_job(text, size, t.text, false, t), size);
            }
            RBlock::Numbered { indent, num, text } => {
                list_line(ui, *indent, &format!("{num}."), t.secondary, inline_job(text, size, t.text, false, t), size);
            }
            RBlock::Todo { indent, done, text } => {
                let color = if *done { t.tertiary } else { t.text };
                let mut job = inline_job(text, size, color, false, t);
                if *done {
                    for s in job.sections.iter_mut() {
                        s.format.strikethrough = egui::Stroke::new(1.0, color);
                    }
                }
                list_line(ui, *indent, if *done { "☑" } else { "☐" }, t.secondary, job, size);
            }
            RBlock::Quote(text) => {
                egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 0, top: 2, bottom: 2 }).show(ui, |ui| {
                    let top = ui.cursor().top();
                    wrapped(ui, inline_job(text, size, t.secondary, false, t));
                    let x = ui.min_rect().left() - 10.0;
                    ui.painter().vline(x, top..=ui.min_rect().bottom(), egui::Stroke::new(3.0, t.border));
                });
            }
            RBlock::Code(code) => {
                egui::Frame::new().fill(t.fill).corner_radius(6.0).inner_margin(10).show(ui, |ui| {
                    let w = ui.available_width();
                    ui.set_width(w);
                    egui::ScrollArea::horizontal().id_salt(("code", i)).max_width(w).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(code).font(FontId::monospace(size * 0.85)).color(t.text)).extend());
                    });
                });
            }
            RBlock::Table(rows) => {
                // Columns share the width; cells wrap. No sideways overflow, whatever the AI wrote.
                let cols = rows.first().map(Vec::len).unwrap_or(1).max(1) as f32;
                let gap = 14.0;
                let w = ui.available_width() - 18.0;
                let col_w = ((w - gap * (cols - 1.0)) / cols).max(50.0);
                egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6.0).inner_margin(8).show(ui, |ui| {
                    egui::Grid::new(("grid", i)).striped(true).spacing([gap, 6.0]).min_col_width(col_w).max_col_width(col_w).show(ui, |ui| {
                        for (r, row) in rows.iter().enumerate() {
                            for cell in row {
                                ui.add(egui::Label::new(inline_job(cell, size - 1.0, if r == 0 { t.secondary } else { t.text }, r == 0, t)).wrap());
                            }
                            ui.end_row();
                        }
                    });
                });
            }
            RBlock::Rule => {
                ui.add_space(2.0);
                let w = ui.available_width();
                let (r, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
                ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.border));
                ui.add_space(2.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(text: &str) -> String {
        runs(text)
            .into_iter()
            .map(|(t, s)| {
                let mut m = String::new();
                if s.bold { m.push('B') }
                if s.italic { m.push('I') }
                if s.code { m.push('C') }
                if s.strike { m.push('S') }
                format!("[{m}]{t}")
            })
            .collect()
    }

    #[test]
    fn nesting_and_underscores() {
        assert_eq!(show("Travail **directement sur `main`**, ok"), "[]Travail [B]directement sur [BC]main[], ok");
        assert_eq!(show("retirer USE_MOCK_MESSAGES"), "[]retirer USE_MOCK_MESSAGES");
        assert_eq!(show("un _mot_ et *autre*"), "[]un [I]mot[] et [I]autre");
        assert_eq!(show("2 * 3 = 6"), "[]2 * 3 = 6");
        assert_eq!(show("**gras sans fin"), "[]**gras sans fin");
        assert_eq!(show("~~barré~~"), "[S]barré");
    }
}
