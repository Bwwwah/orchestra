//! The right-hand panel of a project: the team discussion (first tab) and the AI
//! conversation of one of its sessions, read live from Claude Code / Codex (second tab).

use super::chrome::relative_time;
use super::*;
use crate::model::ChatMsg;
use crate::sessions::{SessionInfo, Turn, Who};
use std::sync::Arc;

const BLUE: Color32 = super::style::BLUE;

/// « Poursuivre la conversation » of a quick question: its AI conversation, to show in
/// this panel (« Conversation IA »), asked from a card deep in the project page.
#[derive(Clone, Default)]
pub struct ConversationRequest {
    pub project: String,
    /// The terminal tab's title, if the conversation must open in a terminal instead.
    pub title: String,
    pub resume: crate::sync::Resume,
    /// When it was asked (`egui` time): the session list catches up within seconds.
    pub at: f64,
}

/// How long to wait for the conversation to appear among the project's sessions before
/// opening it in a terminal instead.
const CONVERSATION_WAIT_S: f64 = 15.0;

fn conversation_key() -> egui::Id {
    egui::Id::unique("side-conversation-request")
}

fn pending_key() -> egui::Id {
    egui::Id::unique("side-conversation-pending")
}

/// A reply typed under a quick question's thread, waiting to be sent to the AI.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadReply {
    pub project: String,
    pub text: String,
    /// The reply asks for the validated plan, which is written into the project's notes.
    pub plan: bool,
}

pub fn reply_key() -> egui::Id {
    egui::Id::unique("side-quick-thread-reply")
}

fn thread_key() -> egui::Id {
    egui::Id::unique("side-quick-thread-request")
}

/// Open this panel on the question/answer thread of a project's quick question.
pub fn open_thread(ctx: &egui::Context, project: &str) {
    ctx.data_mut(|d| d.insert_temp(thread_key(), project.to_string()));
}

/// A quick question and its answer as a thread of messages: the question in a bubble, the
/// answer (Markdown) below it, or the error that came instead.
fn thread(ui: &mut egui::Ui, q: &str, a: &Result<String, String>, by: &str, steps: usize, t: &Tone) {
    egui::Frame::new()
        .fill(t.fill)
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(q).size(13.5).color(t.text)).wrap());
        });
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        ui.label(
            RichText::new("●")
                .size(10.0)
                .color(super::style::CORAL),
        );
        let w = ui.available_width();
        ui.vertical(|ui| {
            ui.set_max_width(w);
            match a {
                Ok(md) => {
                    super::rich::markdown(ui, md, 13.5, t);
                    // Each answer of the thread proposes its steps in the Steps box.
                    if steps > 0 {
                        let s = crate::i18n::plural(steps);
                        ui.label(
                            RichText::new(crate::trf!(
                                "{} étape{s} proposée{s} dans l'encadré Étapes : cochez celles à garder.",
                                "{} step{s} proposed in the Steps box: tick the ones to keep.",
                                steps
                            ))
                            .size(12.5)
                            .color(super::style::GREEN),
                        );
                    }
                    if !by.is_empty() {
                        ui.label(
                            RichText::new(crate::trf!("réponse de {}", "answer by {}", by))
                                .size(11.5)
                                .color(t.tertiary),
                        );
                    }
                }
                Err(e) => {
                    ui.add(
                        egui::Label::new(
                            RichText::new(crate::trf!("Pas de réponse : {e}", "No answer: {e}"))
                                .size(12.5)
                                .color(super::style::ORANGE),
                        )
                        .wrap(),
                    );
                }
            }
        });
    });
}

pub fn open_conversation(
    ctx: &egui::Context,
    project: &str,
    title: &str,
    resume: crate::sync::Resume,
) {
    let at = ctx.input(|i| i.time);
    ctx.data_mut(|d| {
        d.insert_temp(
            conversation_key(),
            ConversationRequest {
                project: project.into(),
                title: title.into(),
                resume,
                at,
            },
        )
    });
}

/// What the « Conversation IA » tab does about a conversation asked for and not yet listed
/// among the project's sessions (`listed`): show it, wait for it, or give up on the panel.
#[derive(Debug, PartialEq)]
enum Waiting {
    Listed,
    Wait,
    /// Not listed in time: open it in a terminal.
    GiveUp,
    /// Asked long ago (the panel was closed meanwhile): forget it, no terminal by surprise.
    Expired,
}

fn waiting(listed: bool, asked_at: f64, now: f64) -> Waiting {
    let age = now - asked_at;
    if listed {
        Waiting::Listed
    } else if age < CONVERSATION_WAIT_S {
        Waiting::Wait
    } else if age < CONVERSATION_WAIT_S + 10.0 {
        Waiting::GiveUp
    } else {
        Waiting::Expired
    }
}

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
    let h = name
        .bytes()
        .fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    COLORS[(h % 8) as usize]
}

fn face(ui: &mut egui::Ui, name: &str) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
    ui.painter()
        .circle_filled(r.center(), 13.0, person_color(name));
    let initial: String = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        semibold(12.5),
        Color32::WHITE,
    );
}

fn tab(ui: &mut egui::Ui, label: &str, on: bool, t: &Tone) -> bool {
    let g = ui.painter().layout_no_wrap(
        label.to_string(),
        semibold(13.0),
        if on { t.text } else { t.tertiary },
    );
    let (r, resp) =
        ui.allocate_exact_size(egui::vec2(g.size().x + 20.0, 30.0), egui::Sense::click());
    if resp.hovered() && !on {
        ui.painter()
            .rect_filled(r.shrink2(egui::vec2(0.0, 3.0)), 6.0, t.hover);
    }
    ui.painter().galley(
        egui::pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0),
        g,
        if on { t.text } else { t.tertiary },
    );
    if on {
        ui.painter().hline(
            r.x_range().shrink(8.0),
            r.bottom() - 1.0,
            egui::Stroke::new(2.0, t.text),
        );
    }
    resp.clicked()
}

/// A text box that sends on Enter (Maj+Entrée: new line). Returns the text to send.
fn composer(ui: &mut egui::Ui, draft: &mut String, hint: &str, enabled: bool) -> Option<String> {
    let mut send = None;
    ui.add_enabled_ui(enabled, |ui| {
        let r = ui.add(
            egui::TextEdit::multiline(draft)
                .hint_text(hint)
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
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
    /// Open this panel on the conversation a button asked for, if any.
    pub(super) fn take_conversation_request(&mut self, ctx: &egui::Context) {
        if let Some(p) = ctx.data_mut(|d| d.remove_temp::<String>(thread_key())) {
            self.thread_pid = Some(p);
            self.side_open = true;
            self.side_tab = 1;
            self.ai_session = None;
        }
        let Some(r) = ctx.data_mut(|d| d.remove_temp::<ConversationRequest>(conversation_key()))
        else {
            return;
        };
        self.side_open = true;
        self.side_tab = 1;
        self.thread_pid = None;
        self.ai_session = Some(r.resume.session.clone());
        ctx.data_mut(|d| d.insert_temp(pending_key(), r));
    }

    /// The right-hand panel, when open and the current page belongs to a project.
    pub(super) fn side_panel(&mut self, ui: &mut egui::Ui, page_fill: Color32) {
        if !self.side_open || self.view != View::Page {
            return;
        }
        let Some(cur) = self.current.clone() else {
            return;
        };
        let Some(project) = self.ws.project_of(&cur).map(|p| {
            (
                p.id.clone(),
                p.display_title(),
                p.project.clone().unwrap_or_default(),
            )
        }) else {
            return;
        };
        let (pid, title, pr) = project;
        let t = tone(ui.visuals().dark_mode);
        let resp = egui::Panel::right("volet")
            .resizable(true)
            .default_size(360.0)
            .size_range(280.0..=640.0)
            .frame(
                egui::Frame::new()
                    .fill(page_fill)
                    .stroke(egui::Stroke::new(1.0, t.border))
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if tab(
                        ui,
                        crate::tr!("Discussion", "Discussion"),
                        self.side_tab == 0,
                        &t,
                    ) {
                        self.side_tab = 0;
                    }
                    if tab(
                        ui,
                        crate::tr!("Conversation IA", "AI conversation"),
                        self.side_tab == 1,
                        &t,
                    ) {
                        self.side_tab = 1;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(
                            ui,
                            icons::Icon::Close,
                            24.0,
                            t.secondary,
                            t.hover,
                            crate::tr!("Fermer le volet", "Close the panel"),
                        )
                        .clicked()
                        {
                            self.side_open = false;
                        }
                    });
                });
                let w = ui.available_width();
                let (r, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
                ui.painter()
                    .hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.border));
                ui.add_space(6.0);
                if self.side_tab == 0 {
                    self.discussion(ui, &pid, &title, &pr, &t);
                } else {
                    self.ai_conversation(ui, &pid, &pr, &t);
                }
            });
        self.side_w = resp.response.rect.width();
    }

    fn discussion(
        &mut self,
        ui: &mut egui::Ui,
        pid: &str,
        title: &str,
        pr: &crate::project::Project,
        t: &Tone,
    ) {
        ui.label(RichText::new(title).font(semibold(14.0)).color(t.text));
        let (status, color) = if !pr.shared {
            (
                crate::tr!(
                    "Projet privé : la discussion ne quitte pas cette machine. Partagez le projet pour en parler avec l'équipe.",
                    "Private project: the discussion doesn't leave this computer. Share the project to talk about it with the team."
                ),
                t.tertiary,
            )
        } else if self.live.is_some() {
            (
                crate::tr!("En direct avec l'équipe.", "Live with the team."),
                super::style::GREEN,
            )
        } else if self.ws.settings.collab.connector != ConnectorKind::None {
            (
                crate::tr!(
                    "Partagé : les messages partent à la prochaine synchronisation.",
                    "Shared: messages go out at the next sync."
                ),
                t.tertiary,
            )
        } else {
            (
                crate::tr!(
                    "Partagé, mais aucune collaboration n'est branchée (Réglages › Collaboration).",
                    "Shared, but no collaboration is set up (Settings › Collaboration)."
                ),
                super::style::ORANGE,
            )
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
                        ui.label(
                            RichText::new(crate::tr!("Pas encore de message.", "No message yet."))
                                .color(t.tertiary),
                        );
                    });
                }
                let mut last_author = String::new();
                let mut last_at = 0u64;
                for m in msgs {
                    // Messages from the same person within 5 minutes are grouped.
                    let grouped =
                        m.author == last_author && m.at.saturating_sub(last_at) < 5 * 60_000;
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
                                    let who = if m.author == me {
                                        format!("{} (vous)", m.author)
                                    } else {
                                        m.author.clone()
                                    };
                                    ui.label(RichText::new(who).font(semibold(13.0)).color(t.text));
                                    ui.label(
                                        RichText::new(relative_time(m.at))
                                            .size(11.5)
                                            .color(t.tertiary),
                                    );
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
        if let Some(text) = composer(
            ui,
            &mut self.chat_draft,
            crate::tr!(
                "Écrire à l'équipe… (Entrée pour envoyer)",
                "Write to the team… (Enter to send)"
            ),
            true,
        ) {
            let m = ChatMsg {
                id: crate::model::new_id(),
                project: pid.to_string(),
                author: me,
                text,
                at: crate::model::now_ms(),
            };
            self.ws.chat.insert(m.id.clone(), m);
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    /// The thread of the project's quick question with a field under it to answer the AI.
    /// False when the project has no quick answer. A reply sent is queued as a
    /// `ThreadReply`, for whoever carries the conversation on.
    fn quick_thread_view(&mut self, ui: &mut egui::Ui, pid: &str, t: &Tone) -> bool {
        let Some(a) = self.ai.answers.get(pid) else {
            return false;
        };
        let busy = self.ai.asking(pid).is_some();
        let composer_h = 90.0;
        egui::ScrollArea::vertical()
            .id_salt(("quick-thread", pid))
            .max_height((ui.available_height() - composer_h).max(80.0))
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (q, ans, by, n) in &a.earlier {
                    thread(ui, q, ans, by, *n, t);
                    ui.add_space(10.0);
                }
                thread(ui, &a.question, &a.answer, &a.by, a.steps, t);
                // A reply being sent: its bubble shows at once, the answer follows.
                if let Some(q) = self.ai.asking(pid).filter(|q| *q != a.question) {
                    ui.add_space(10.0);
                    egui::Frame::new()
                        .fill(t.fill)
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(egui::Label::new(RichText::new(q).size(13.5).color(t.text)).wrap());
                        });
                }
                // The AI is working on it: a spinner and a line, until its answer comes.
                if busy {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(
                            RichText::new(crate::tr!("L'IA répond…", "The AI is answering…"))
                                .size(12.5)
                                .color(t.secondary),
                        );
                    });
                }
            });
        ui.add_space(6.0);
        let hint = crate::tr!(
            "Répondre à l'IA… (Entrée pour envoyer)",
            "Reply to the AI… (Enter to send)"
        );
        if let Some(text) = composer(ui, &mut self.thread_draft, hint, !busy) {
            ui.ctx().data_mut(|d| {
                d.get_temp_mut_or_default::<Vec<ThreadReply>>(reply_key())
                    .push(ThreadReply {
                        project: pid.to_string(),
                        text,
                        plan: false,
                    })
            });
        }
        // The validated plan turned into proposed steps, ticked by the person before any is added.
        if ui
            .add_enabled_ui(!busy, |ui| {
                super::projects::action_link(ui, crate::tr!("Proposer les étapes", "Propose the steps"))
                    .on_hover_text(crate::tr!(
                        "L'IA propose les étapes du plan validé dans l'encadré Étapes ; vous cochez celles à ajouter.",
                        "The AI proposes the validated plan's steps in the Steps box; you tick the ones to add."
                    ))
            })
            .inner
            .clicked()
        {
            ui.ctx().data_mut(|d| {
                d.get_temp_mut_or_default::<Vec<ThreadReply>>(reply_key())
                    .push(ThreadReply {
                        project: pid.to_string(),
                        text: crate::sync::plan_steps_request().to_string(),
                        plan: false,
                    })
            });
        }
        // Once the questions are answered: the AI writes the plan into the project's notes.
        if ui
            .add_enabled_ui(!busy, |ui| {
                super::projects::action_link(ui, crate::tr!("Valider le plan", "Validate the plan"))
                    .on_hover_text(crate::tr!(
                        "L'IA écrit l'objectif, le périmètre et le hors périmètre dans les notes du projet.",
                        "The AI writes the goal, scope and out-of-scope into the project's notes."
                    ))
            })
            .inner
            .clicked()
        {
            ui.ctx().data_mut(|d| {
                d.get_temp_mut_or_default::<Vec<ThreadReply>>(reply_key())
                    .push(ThreadReply {
                        project: pid.to_string(),
                        text: crate::sync::plan_request().to_string(),
                        plan: true,
                    })
            });
        }
        // The end of the framing: how much effort the project's sessions will put in.
        let mut saving = self.project_saving(pid);
        let before = saving;
        ui.horizontal(|ui| {
            ui.label(RichText::new(crate::tr!("Effort", "Effort")).size(12.5).color(t.secondary));
            egui::ComboBox::from_id_salt(("frame-saving", pid))
                .selected_text(RichText::new(saving.label()).size(12.5))
                .width(100.0)
                .show_ui(ui, |ui| {
                    for s in crate::model::Saving::ALL {
                        ui.selectable_value(&mut saving, s, s.label());
                    }
                })
                .response
                .on_hover_text(crate::tr!(
                    "Économe = modèle plus léger et peu de réflexion, Qualité = modèle plus haut et plus de réflexion. Gardé pour les prochaines sessions du projet (modifiable dans le lancement autonome).",
                    "Economical = lighter model and little thinking, Quality = higher model and more thinking. Kept for the project's next sessions (also editable in the autonomous launch card)."
                ));
        });
        if saving != before {
            self.set_project_saving(pid, saving);
        }
        true
    }

    fn ai_conversation(
        &mut self,
        ui: &mut egui::Ui,
        pid: &str,
        pr: &crate::project::Project,
        t: &Tone,
    ) {
        let live: Vec<SessionInfo> = self.agents.by_project.get(pid).cloned().unwrap_or_default();
        // The thread shown before the panel was closed comes back as it was (the answers and
        // the replies live in the app, not in the panel).
        if self.thread_pid.as_deref() == Some(pid) {
            if self.quick_thread_view(ui, pid, t) {
                if self.ai.asking(pid).is_some() {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                }
                return;
            }
            self.thread_pid = None;
        }
        // A quick question carried on: its session shows up in the list within seconds.
        let pending = ui
            .ctx()
            .data(|d| d.get_temp::<ConversationRequest>(pending_key()))
            .filter(|r| r.project == pid);
        if let Some(r) = pending {
            let listed = live.iter().any(|s| s.id == r.resume.session);
            match waiting(listed, r.at, ui.input(|i| i.time)) {
                Waiting::Listed | Waiting::Expired => {
                    ui.ctx()
                        .data_mut(|d| d.remove::<ConversationRequest>(pending_key()));
                }
                Waiting::Wait => {
                    if self.quick_thread_view(ui, pid, t) {
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                        return;
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(
                            RichText::new(crate::tr!(
                                "La conversation de la question rapide arrive…",
                                "The quick question's conversation is on its way…"
                            ))
                            .size(12.5)
                            .color(t.tertiary),
                        );
                    });
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                    return;
                }
                Waiting::GiveUp => {
                    // Not found in the project's sessions: carried on in a terminal.
                    ui.ctx()
                        .data_mut(|d| d.remove::<ConversationRequest>(pending_key()));
                    self.ai_session = None;
                    super::term_ui::request(
                        ui.ctx(),
                        &r.title,
                        &r.resume.cwd,
                        Some(r.resume.cmd.clone()),
                    );
                }
            }
        }
        if live.is_empty() {
            if self.quick_thread_view(ui, pid, t) {
                return;
            }
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
        let sel = self
            .ai_session
            .clone()
            .filter(|id| live.iter().any(|s| &s.id == id))
            .unwrap_or_else(|| live[0].id.clone());
        let s = live
            .iter()
            .find(|s| s.id == sel)
            .unwrap_or(&live[0])
            .clone();
        egui::ComboBox::from_id_salt(("ai-session", pid))
            .width(ui.available_width() - 8.0)
            .selected_text(format!(
                "{} · {}",
                s.tool.label(),
                crate::richtext::one_line(&s.display_title(), 40)
            ))
            .show_ui(ui, |ui| {
                for o in live.iter().take(30) {
                    let label = format!(
                        "{} · {} · {}",
                        o.tool.label(),
                        crate::richtext::one_line(&o.display_title(), 40),
                        relative_time(o.last_activity)
                    );
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
                            egui::Frame::new()
                                .fill(t.fill)
                                .corner_radius(10.0)
                                .inner_margin(egui::Margin::symmetric(10, 7))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&turn.text).size(13.5).color(t.text),
                                        )
                                        .wrap(),
                                    );
                                });
                        }
                        Who::Agent => {
                            ui.horizontal_top(|ui| {
                                ui.label(
                                    RichText::new("●")
                                        .size(10.0)
                                        .color(super::style::CORAL),
                                );
                                let w = ui.available_width();
                                ui.vertical(|ui| {
                                    ui.set_max_width(w);
                                    super::rich::markdown(ui, &turn.text, 13.5, t);
                                });
                            });
                        }
                        Who::Tool => {
                            let mut job = egui::text::LayoutJob::single_section(
                                format!("  ↳ {}", turn.text),
                                egui::TextFormat::simple(FontId::monospace(11.5), t.tertiary),
                            );
                            job.wrap =
                                egui::text::TextWrapping::truncate_at_width(ui.available_width());
                            super::projects::cut_label(ui, job);
                        }
                    }
                    ui.add_space(6.0);
                }
                if s.is_live(crate::model::now_ms()) {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(
                            RichText::new(crate::tr!("l'IA travaille…", "the AI is working…"))
                                .size(12.5)
                                .color(t.tertiary),
                        );
                    });
                }
            });
        ui.add_space(6.0);
        // Writing to the AI goes through its terminal (opened here if needed).
        // An id unfit for a shell command opens a plain terminal in the folder instead.
        let resume = s.resume_args();
        let term = if s.has_safe_id() {
            self.terms
                .iter()
                .position(|x| x.cmd.as_deref().is_some_and(|c| c.contains(&s.id)))
        } else {
            None
        };
        let hint = if term.is_some() {
            crate::tr!(
                "Écrire à l'IA… (Entrée pour envoyer dans son terminal)",
                "Write to the AI… (Enter to send it to its terminal)"
            )
        } else {
            crate::tr!(
                "Écrire à l'IA… (la session s'ouvrira dans le terminal)",
                "Write to the AI… (the session will open in the terminal)"
            )
        };
        if let Some(text) = composer(ui, &mut self.ai_draft, hint, true) {
            match term {
                Some(i) => {
                    let term = &mut self.terms[i];
                    term.paste(&text);
                    term.write(b"\r");
                }
                None => {
                    let title = format!(
                        "{} · {}",
                        s.tool.label(),
                        crate::richtext::one_line(&s.display_title(), 24)
                    );
                    super::term_ui::request_with_input(
                        ui.ctx(),
                        &title,
                        &s.cwd,
                        resume.clone(),
                        Some(text),
                    );
                }
            }
            self.term_open = true;
        }
        ui.horizontal(|ui| {
            if ui
                .small_button(crate::tr!(
                    "Ouvrir dans le terminal",
                    "Open in the terminal"
                ))
                .clicked()
            {
                match term {
                    Some(i) => {
                        self.term_active = i;
                        self.term_open = true;
                        self.term_focus = true;
                    }
                    None => {
                        let title = format!(
                            "{} · {}",
                            s.tool.label(),
                            crate::richtext::one_line(&s.display_title(), 24)
                        );
                        super::term_ui::request(ui.ctx(), &title, &s.cwd, resume);
                    }
                }
            }
            let _ = pr;
            ui.label(
                RichText::new(crate::trf!(
                    "{} message{p}",
                    "{} message{p}",
                    s.user_messages + s.agent_messages,
                    p = crate::i18n::plural((s.user_messages + s.agent_messages) as usize)
                ))
                .size(11.5)
                .color(t.tertiary),
            );
        });
        let _ = BLUE;
    }
}

#[cfg(test)]
mod tests {
    use super::{CONVERSATION_WAIT_S as W, Waiting, waiting};

    #[test]
    fn a_carried_on_conversation_waits_for_its_session_then_falls_back() {
        assert_eq!(waiting(true, 0.0, 1.0), Waiting::Listed);
        assert_eq!(waiting(true, 0.0, 999.0), Waiting::Listed);
        assert_eq!(waiting(false, 10.0, 11.0), Waiting::Wait);
        assert_eq!(waiting(false, 10.0, 10.0 + W + 1.0), Waiting::GiveUp);
        assert_eq!(waiting(false, 10.0, 10.0 + W + 60.0), Waiting::Expired);
    }
}
