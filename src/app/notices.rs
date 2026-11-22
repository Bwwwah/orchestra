//! What the window has to say. Three levels:
//! - `Ack`: a quick confirmation (« Lien copié »): a small popup that fades away, nothing kept;
//! - `Info`: something worth keeping (an export, an AI plugged in): it goes to the inbox;
//! - `Error`: it goes to the inbox AND a red popup shows it until it's closed or times out.

use super::*;

/// One message, before it's shown.
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    Ack(String),
    Info(String),
    Error(String),
}

impl Notice {
    /// Ok → `Info`, Err → `Error`.
    pub fn from_result(r: Result<String, String>) -> Notice {
        match r {
            Ok(m) => Notice::Info(m),
            Err(e) => Notice::Error(e),
        }
    }
}

/// A popup on screen.
pub struct Toast {
    id: u64,
    text: String,
    error: bool,
    born: Instant,
}

const ACK_SECS: f32 = 2.5;
const ERROR_SECS: f32 = 9.0;
/// Longest inbox title; the full text goes below it.
const TITLE_MAX: usize = 90;

impl NotesApp {
    pub(super) fn ack(&mut self, msg: impl Into<String>) {
        self.notice(Notice::Ack(msg.into()));
    }

    pub(super) fn info(&mut self, msg: impl Into<String>) {
        self.notice(Notice::Info(msg.into()));
    }

    pub(super) fn fail(&mut self, msg: impl Into<String>) {
        self.notice(Notice::Error(msg.into()));
    }

    pub(super) fn notice(&mut self, n: Notice) {
        let (text, kind) = match n {
            Notice::Ack(m) => {
                self.push_toast(m, false);
                return;
            }
            Notice::Info(m) => (m, "info"),
            Notice::Error(m) => (m, "erreur"),
        };
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let (title, body) = if text.chars().count() > TITLE_MAX {
            (crate::richtext::one_line(&text, TITLE_MAX), text.clone())
        } else {
            (text.clone(), String::new())
        };
        // The same message again (a retry loop, a save that keeps failing): bring the unread
        // one back to the top instead of piling copies up, and pop it up at most once a minute.
        let now = now_ms();
        let mut fresh = true;
        if let Some(m) = self
            .ws
            .inbox
            .iter_mut()
            .rev()
            .find(|m| !m.read && m.kind == kind && m.title == title && m.text == body)
        {
            fresh = now.saturating_sub(m.at) >= 60_000;
            m.at = now;
        } else {
            self.ws.notify(kind, "", &title, &body, None);
        }
        if kind == "erreur" && fresh {
            self.push_toast(text, true);
        }
        self.ws.inbox.sort_by_key(|m| m.at);
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    fn push_toast(&mut self, text: String, error: bool) {
        if text.trim().is_empty() {
            return;
        }
        // Same text already up: start it again rather than stacking it.
        self.toasts.retain(|t| t.text != text);
        self.toast_seq += 1;
        self.toasts.push(Toast {
            id: self.toast_seq,
            text,
            error,
            born: Instant::now(),
        });
        // At most four at once, the oldest leave first.
        if self.toasts.len() > 4 {
            let cut = self.toasts.len() - 4;
            self.toasts.drain(..cut);
        }
    }

    /// The popups, stacked at the bottom right, clear of the side panel, the terminal dock
    /// and the round AI button.
    pub(super) fn toasts_ui(&mut self, ctx: &egui::Context, fab: bool) {
        if self.toasts.is_empty() {
            return;
        }
        let t = tone(ctx.global_style().visuals.dark_mode);
        let red = super::style::RED;
        let mut close: Vec<u64> = vec![];
        let mut open_inbox = false;
        let mut next_wake = f32::MAX;
        let bottom = 22.0 + self.dock_h + if fab { 60.0 } else { 0.0 };
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(-22.0 - self.side_w, -bottom),
            )
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                for toast in self.toasts.iter_mut().rev() {
                    let life = if toast.error { ERROR_SECS } else { ACK_SECS };
                    let age = toast.born.elapsed().as_secs_f32();
                    if age >= life {
                        close.push(toast.id);
                        continue;
                    }
                    // Fade out over the last 0.4 s.
                    let alpha = ((life - age) / 0.4).clamp(0.0, 1.0);
                    next_wake = next_wake.min(if alpha < 1.0 { 0.03 } else { life - age - 0.4 });
                    let frame = egui::Frame::new()
                        .fill(t.card.gamma_multiply(alpha))
                        .stroke(egui::Stroke::new(
                            1.0,
                            if toast.error {
                                red.gamma_multiply(0.6 * alpha)
                            } else {
                                t.border.gamma_multiply(alpha)
                            },
                        ))
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin::symmetric(12, 9))
                        .shadow(egui::Shadow {
                            offset: [0, 4],
                            blur: 14,
                            spread: 0,
                            color: Color32::from_black_alpha((40.0 * alpha) as u8),
                        });
                    let r = frame.show(ui, |ui| {
                        ui.set_width(336.0);
                        ui.horizontal_top(|ui| {
                            if toast.error {
                                let (dot, _) = ui.allocate_exact_size(
                                    egui::vec2(10.0, 18.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(
                                    dot.center(),
                                    4.0,
                                    red.gamma_multiply(alpha),
                                );
                            }
                            ui.vertical(|ui| {
                                ui.set_width(ui.available_width() - 24.0);
                                if toast.error {
                                    ui.label(
                                        RichText::new(crate::tr!("Erreur", "Error"))
                                            .size(11.5)
                                            .color(red.gamma_multiply(alpha)),
                                    );
                                }
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&toast.text)
                                            .size(13.0)
                                            .color(t.text.gamma_multiply(alpha)),
                                    )
                                    .wrap(),
                                );
                            });
                            if icons::button(
                                ui,
                                icons::Icon::Close,
                                20.0,
                                t.secondary,
                                t.hover,
                                crate::tr!("Fermer", "Close"),
                            )
                            .clicked()
                            {
                                close.push(toast.id);
                            }
                        });
                    });
                    let resp = ui.interact(
                        r.response.rect,
                        egui::Id::unique(("toast", toast.id)),
                        egui::Sense::click(),
                    );
                    // Hovered: it stays up while it's read.
                    if resp.hovered() {
                        toast.born = Instant::now();
                    }
                    if toast.error
                        && resp
                            .on_hover_text(crate::tr!(
                                "Ouvrir la boîte de réception",
                                "Open the inbox"
                            ))
                            .clicked()
                        && !close.contains(&toast.id)
                    {
                        open_inbox = true;
                        close.push(toast.id);
                    }
                    ui.add_space(8.0);
                }
            });
        self.toasts.retain(|x| !close.contains(&x.id));
        if open_inbox {
            self.view = View::Inbox;
        }
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(
                next_wake.clamp(0.016, 10.0),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_map_to_levels() {
        assert_eq!(
            Notice::from_result(Ok("ok".into())),
            Notice::Info("ok".into())
        );
        assert_eq!(
            Notice::from_result(Err("ko".into())),
            Notice::Error("ko".into())
        );
    }
}
