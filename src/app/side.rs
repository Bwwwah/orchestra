//! The right-hand panel of a project: the team discussion (first tab) and the AI
//! conversation of one of its sessions, read live from Claude Code / Codex (second tab).

use super::chrome::relative_time;
use super::*;
use crate::model::ChatMsg;
use crate::sessions::{SessionInfo, Turn, Who};
use std::sync::Arc;

const BLUE: Color32 = Color32::from_rgb(35, 131, 226);

/// A steady colour per person, for the round faces.
fn person_color(name: &str) -> Color32 {
    const COLORS: [Color32; 8] = [
        Color32::from_rgb(214, 90, 75),
        Color32::from_rgb(70, 140, 210),
        Color32::from_rgb(90, 160, 90),
        Color32::from_rgb(200, 140, 40),
        Color32::from_rgb(150, 100, 200),
        Color32::from_rgb(40, 160, 160),
        Color32::from_rgb(200, 90, 150),
        Color32::from_rgb(120, 120, 120),
    ];
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    COLORS[(h % 8) as usize]
}

fn face(ui: &mut egui::Ui, name: &str) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
    ui.painter().circle_filled(r.center(), 13.0, person_color(name));
    let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, initial, semibold(12.5), Color32::WHITE);
}

fn tab(ui: &mut egui::Ui, label: &str, on: bool, t: &Tone) -> bool {
    let g = ui.painter().layout_no_wrap(label.to_string(), semibold(13.0), if on { t.text } else { t.tertiary });
    let (r, resp) = ui.allocate_exact_size(egui::vec2(g.size().x + 20.0, 30.0), egui::Sense::click());
    if resp.hovered() && !on {
        ui.painter().rect_filled(r.shrink2(egui::vec2(0.0, 3.0)), 6.0, t.hover);
    }
    ui.painter().galley(egui::pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, if on { t.text } else { t.tertiary });
    if on {
        ui.painter().hline(r.x_range().shrink(8.0), r.bottom() - 1.0, egui::Stroke::new(2.0, t.text));
    }
    resp.clicked()
}

/// A text box that sends on Enter (Maj+Entrée: new line). Returns the text to send.
fn composer(ui: &mut egui::Ui, draft: &mut String, hint: &str, enabled: bool) -> Option<String> {
    let mut send = None;
    ui.add_enabled_ui(enabled, |ui| {
        let r = ui.add(egui::TextEdit::multiline(draft).hint_text(hint).desired_rows(2).desired_width(f32::INFINITY));
        let enter = r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
        if enter {
            // The Enter key already put a line break in: take it back out.
            let text = draft.trim_end_matches('\n').trim().to_string();
            draft.clear();
            if !text.is_empty() {
                send = Some(text);
            }
            r.request_focus();
        }
    });
    send
}

impl NotesApp {
    /// The right-hand panel, when open and the current page belongs to a project.
    pub(super) fn side_panel(&mut self, ui: &mut egui::Ui, page_fill: Color32) {
        if !self.side_open || self.view != View::Page {
            return;
        }
        let Some(cur) = self.current.clone() else { return };
        let Some(project) = self.ws.project_of(&cur).map(|p| (p.id.clone(), p.display_title(), p.project.clone().unwrap_or_default())) else { return };
        let (pid, title, pr) = project;
        let t = tone(ui.visuals().dark_mode);
        let resp = egui::Panel::right("volet")
            .resizable(true)
            .default_size(360.0)
            .size_range(280.0..=640.0)
            .frame(egui::Frame::new().fill(page_fill).stroke(egui::Stroke::new(1.0, t.border)).inner_margin(egui::Margin::symmetric(14, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if tab(ui, crate::tr!("Discussion", "Discussion"), self.side_tab == 0, &t) {
                        self.side_tab = 0;
                    }
                    if tab(ui, crate::tr!("Conversation IA", "AI conversation"), self.side_tab == 1, &t) {
                        self.side_tab = 1;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, icons::Icon::Close, 24.0, t.secondary, t.hover, crate::tr!("Fermer le volet", "Close the panel")).clicked() {
                            self.side_open = false;
                        }
                    });
                });
                let w = ui.available_width();
                let (r, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
                ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.border));
                ui.add_space(6.0);
                if self.side_tab == 0 {
                    self.discussion(ui, &pid, &title, &pr, &t);
                } else {
                    self.ai_conversation(ui, &pid, &pr, &t);
                }
            });
        self.side_w = resp.response.rect.width();
    }

    fn discussion(&mut self, ui: &mut egui::Ui, pid: &str, title: &str, pr: &crate::project::Project, t: &Tone) {
        ui.label(RichText::new(title).font(semibold(14.0)).color(t.text));
        let (status, color) = if !pr.shared {
            (crate::tr!("Projet privé : la discussion ne quitte pas cette machine. Partagez le projet pour en parler avec l'équipe.", "Private project: the discussion doesn't leave this computer. Share the project to talk about it with the team."), t.tertiary)
        } else if self.live.is_some() {
            (crate::tr!("En direct avec l'équipe.", "Live with the team."), Color32::from_rgb(68, 131, 97))
        } else if self.ws.settings.collab.connector != ConnectorKind::None {
            (crate::tr!("Partagé : les messages partent à la prochaine synchronisation.", "Shared: messages go out at the next sync."), t.tertiary)
        } else {
            (crate::tr!("Partagé, mais aucune collaboration n'est branchée (Réglages › Collaboration).", "Shared, but no collaboration is set up (Settings › Collaboration)."), Color32::from_rgb(217, 115, 13))
        };
        ui.add(egui::Label::new(RichText::new(status).size(12.0).color(color)).wrap());
        ui.add_space(6.0);
        let me = self.my_name();
        let mut msgs: Vec<&ChatMsg> = self.ws.chat.values().filter(|m| m.project == pid).collect();
        msgs.sort_by_key(|m| m.at);
        let composer_h = 86.0;
        egui::ScrollArea::vertical()
            .id_salt(("chat", pid))
            .max_height((ui.available_height() - composer_h).max(80.0))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if msgs.is_empty() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(crate::tr!("Pas encore de message.", "No message yet.")).color(t.tertiary));
                    });
                }
                let mut last_author = String::new();
                let mut last_at = 0u64;
                for m in msgs {
                    // Messages from the same person within 5 minutes are grouped.
                    let grouped = m.author == last_author && m.at.saturating_sub(last_at) < 5 * 60_000;
                    ui.horizontal_top(|ui| {
                        if grouped {
                            ui.add_space(34.0);
                        } else {
                            face(ui, &m.author);
                            ui.add_space(4.0);
                        }
                        let w = ui.available_width();
                        ui.vertical(|ui| {
                            ui.set_max_width(w);
                            if !grouped {
                                ui.horizontal(|ui| {
                                    let who = if m.author == me { format!("{} (vous)", m.author) } else { m.author.clone() };
                                    ui.label(RichText::new(who).font(semibold(13.0)).color(t.text));
                                    ui.label(RichText::new(relative_time(m.at)).size(11.5).color(t.tertiary));
                                });
                            }
                            super::rich::markdown(ui, &m.text, 13.5, t);
                        });
                    });
                    ui.add_space(if grouped { 2.0 } else { 8.0 });
                    last_author = m.author.clone();
                    last_at = m.at;
                }
            });
        ui.add_space(6.0);
        if let Some(text) = composer(ui, &mut self.chat_draft, crate::tr!("Écrire à l'équipe… (Entrée pour envoyer)", "Write to the team… (Enter to send)"), true) {
            let m = ChatMsg { id: crate::model::new_id(), project: pid.to_string(), author: me, text, at: crate::model::now_ms() };
            self.ws.chat.insert(m.id.clone(), m);
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    fn ai_conversation(&mut self, ui: &mut egui::Ui, pid: &str, pr: &crate::project::Project, t: &Tone) {
        let live: Vec<SessionInfo> = self.agents.by_project.get(pid).cloned().unwrap_or_default();
        if live.is_empty() {
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(
                    RichText::new(crate::tr!("Aucune session Claude Code ou Codex dans le dossier de ce projet pour l'instant. Lancez-en une depuis la carte « Sessions IA ».", "No Claude Code or Codex session in this project's folder yet. Start one from the « AI sessions » card."))
                        .color(t.tertiary),
                )
                .wrap(),
            );
            return;
        }
        // Which session: the chosen one if still there, else the most recent.
        let sel = self.ai_session.clone().filter(|id| live.iter().any(|s| &s.id == id)).unwrap_or_else(|| live[0].id.clone());
        let s = live.iter().find(|s| s.id == sel).unwrap().clone();
        egui::ComboBox::from_id_salt(("ai-session", pid))
            .width(ui.available_width() - 8.0)
            .selected_text(format!("{} · {}", s.tool.label(), crate::richtext::one_line(&s.display_title(), 40)))
            .show_ui(ui, |ui| {
                for o in live.iter().take(30) {
                    let label = format!("{} · {} · {}", o.tool.label(), crate::richtext::one_line(&o.display_title(), 40), relative_time(o.last_activity));
                    if ui.selectable_label(o.id == s.id, label).clicked() {
                        self.ai_session = Some(o.id.clone());
                    }
                }
            });
        ui.add_space(4.0);
        // The conversation, re-read only when the file grew.
        let len = std::fs::metadata(&s.path).map(|m| m.len()).unwrap_or(0);
        let key = s.path.display().to_string();
        let turns: Arc<Vec<Turn>> = match self.transcripts.get(&key) {
            Some((l, t)) if *l == len => t.clone(),
            _ => {
                let tr = Arc::new(crate::sessions::transcript(s.tool, &s.path, 3 << 20, 80));
                self.transcripts.insert(key, (len, tr.clone()));
                tr
            }
        };
        let composer_h = 120.0;
        egui::ScrollArea::vertical()
            .id_salt(("ai", &s.id))
            .max_height((ui.available_height() - composer_h).max(80.0))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for turn in turns.iter() {
                    match turn.who {
                        Who::User => {
                            egui::Frame::new().fill(t.fill).corner_radius(10.0).inner_margin(egui::Margin::symmetric(10, 7)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(egui::Label::new(RichText::new(&turn.text).size(13.5).color(t.text)).wrap());
                            });
                        }
                        Who::Agent => {
                            ui.horizontal_top(|ui| {
                                ui.label(RichText::new("●").size(10.0).color(Color32::from_rgb(204, 120, 92)));
                                let w = ui.available_width();
                                ui.vertical(|ui| {
                                    ui.set_max_width(w);
                                    super::rich::markdown(ui, &turn.text, 13.5, t);
                                });
                            });
                        }
                        Who::Tool => {
                            let mut job = egui::text::LayoutJob::single_section(format!("  ↳ {}", turn.text), egui::TextFormat::simple(FontId::monospace(11.5), t.tertiary));
                            job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
                            super::projects::cut_label(ui, job);
                        }
                    }
                    ui.add_space(6.0);
                }
                if s.is_live(crate::model::now_ms()) {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(RichText::new(crate::tr!("l'IA travaille…", "the AI is working…")).size(12.5).color(t.tertiary));
                    });
                }
            });
        ui.add_space(6.0);
        // Writing to the AI goes through its terminal (opened here if needed).
        // An id unfit for a shell command opens a plain terminal in the folder instead.
        let resume = s.resume_args();
        let term = if s.has_safe_id() { self.terms.iter().position(|x| x.cmd.as_deref().is_some_and(|c| c.contains(&s.id))) } else { None };
        let hint = if term.is_some() { crate::tr!("Écrire à l'IA… (Entrée pour envoyer dans son terminal)", "Write to the AI… (Enter to send it to its terminal)") } else { crate::tr!("Écrire à l'IA… (la session s'ouvrira dans le terminal)", "Write to the AI… (the session will open in the terminal)") };
        if let Some(text) = composer(ui, &mut self.ai_draft, hint, true) {
            match term {
                Some(i) => {
                    let term = &mut self.terms[i];
                    term.paste(&text);
                    term.write(b"\r");
                }
                None => {
                    let title = format!("{} · {}", s.tool.label(), crate::richtext::one_line(&s.display_title(), 24));
                    super::term_ui::request_with_input(ui.ctx(), &title, &s.cwd, resume.clone(), Some(text));
                }
            }
            self.term_open = true;
        }
        ui.horizontal(|ui| {
            if ui.small_button(crate::tr!("Ouvrir dans le terminal", "Open in the terminal")).clicked() {
                match term {
                    Some(i) => {
                        self.term_active = i;
                        self.term_open = true;
                        self.term_focus = true;
                    }
                    None => {
                        let title = format!("{} · {}", s.tool.label(), crate::richtext::one_line(&s.display_title(), 24));
                        super::term_ui::request(ui.ctx(), &title, &s.cwd, resume);
                    }
                }
            }
            let _ = pr;
            ui.label(RichText::new(crate::trf!("{} message{p}", "{} message{p}", s.user_messages + s.agent_messages, p = crate::i18n::plural((s.user_messages + s.agent_messages) as usize))).size(11.5).color(t.tertiary));
        });
        let _ = BLUE;
    }
}
