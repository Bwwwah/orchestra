//! The conversation with an AI that a project without code has in the right-hand panel (the
//! question sent is in `sync::talk`). It stays on this computer, one file per project next to
//! the workspace: never in workspace.json, which a team shares. An answer is waited for
//! whether the panel is open or not.

use super::chrome::relative_time;
use super::*;
use crate::project::{Project, StepStatus};
use crate::sessions::Tool;
use crate::sync::openai::{OpenAi, Service};
use crate::sync::talk::{About, MESSAGE_MAX, Said};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Most messages kept: the oldest go first.
const KEPT: usize = 200;

/// One message of a project's conversation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Message {
    /// Written by the person (else by an AI).
    mine: bool,
    text: String,
    /// The AI that answered ("" for the person).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    by: String,
    at: u64,
    /// How many steps the answer proposed.
    #[serde(default, skip_serializing_if = "is_zero")]
    steps: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// An answer and who gave it, once there.
type Slot = Arc<Mutex<Option<Result<(String, String), String>>>>;

/// One project's conversation, as the app holds it.
#[derive(Default)]
struct Talk {
    messages: Vec<Message>,
    /// Why its file couldn't be read or written. While it can't be read, nothing is written
    /// over it.
    unreadable: Option<String>,
    unsaved: Option<String>,
    /// The page's project, to give it the steps an answer proposes.
    project: String,
    /// The answer being waited for.
    job: Option<Slot>,
    /// Why the last message got no answer.
    error: Option<String>,
    draft: String,
    /// « Effacer » was clicked once: a second click does it.
    clearing: bool,
}

/// The conversations opened since the app started, by their file: each space has its own.
#[derive(Default)]
pub struct Talks {
    open: HashMap<PathBuf, Talk>,
    /// The project whose panel shows its Claude Code / Codex sessions instead.
    pub sessions_of: Option<String>,
}

/// The file of a project's conversation in `dir`. None for an id that isn't a plain name: a
/// shared project's id comes from someone else's computer.
fn file_in(dir: &Path, pid: &str) -> Option<PathBuf> {
    let plain = !pid.is_empty()
        && pid.len() <= 80
        && pid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    plain.then(|| dir.join(format!("{pid}.json")))
}

fn file_of(pid: &str) -> Option<PathBuf> {
    file_in(
        &crate::store::workspace_path().parent()?.join("conversations"),
        pid,
    )
}

fn read(path: &Path) -> Result<Vec<Message>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.to_string()),
    }
}

/// Keep `messages` at `path`; none left: no file.
fn write(path: &Path, messages: &[Message]) -> std::io::Result<()> {
    if messages.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::store::write_atomic(path, &serde_json::to_vec(messages)?)
}

impl Talk {
    fn load(path: &Path) -> Self {
        match read(path) {
            Ok(messages) => Talk {
                messages,
                ..Default::default()
            },
            Err(e) => Talk {
                unreadable: Some(e),
                ..Default::default()
            },
        }
    }

    fn push(&mut self, path: &Path, message: Message) {
        self.messages.push(message);
        let over = self.messages.len().saturating_sub(KEPT);
        self.messages.drain(..over);
        self.save(path);
    }

    fn save(&mut self, path: &Path) {
        if self.unreadable.is_none() {
            self.unsaved = write(path, &self.messages).err().map(|e| e.to_string());
        }
    }

    /// Take an answer in: the steps it proposes come out of its text, to be returned.
    fn take(&mut self, path: &Path, answer: Result<(String, String), String>) -> Vec<String> {
        match answer {
            Ok((answer, by)) => {
                let (text, steps) = crate::sync::split_answer_steps(&answer);
                self.error = None;
                self.push(
                    path,
                    Message {
                        mine: false,
                        text,
                        by,
                        at: crate::model::now_ms(),
                        steps: steps.len(),
                    },
                );
                steps
            }
            Err(e) => {
                self.error = Some(e);
                vec![]
            }
        }
    }

    /// The message waiting for an answer and the exchanges before it.
    fn unanswered(&self) -> Option<(&Message, &[Message])> {
        self.messages
            .split_last()
            .filter(|(last, _)| last.mine)
    }
}

/// What the dashboard holds of a project, as the AI is told.
fn card(title: &str, pr: &Project) -> String {
    let mut out = format!(
        "{} : {title}\n{} : {}",
        crate::tr!("Titre", "Title"),
        crate::tr!("Statut", "Status"),
        pr.status.label()
    );
    if !pr.summary.trim().is_empty() {
        out.push_str(&format!(
            "\n\n## {}\n{}",
            crate::tr!("État des lieux", "State of play"),
            pr.summary.trim()
        ));
    }
    if !pr.steps.is_empty() {
        out.push_str(&format!("\n\n## {}", crate::tr!("Étapes", "Steps")));
        // The open ones first: a long list is cut short at the end.
        let (done, open): (Vec<_>, Vec<_>) =
            pr.steps.iter().partition(|s| s.status == StepStatus::Done);
        for s in open.iter().chain(done.iter().rev()) {
            let mark = if s.status == StepStatus::Done { "x" } else { " " };
            out.push_str(&format!("\n- [{mark}] {}", s.title));
        }
    }
    out
}

impl NotesApp {
    /// Send the unanswered message of the conversation at `path` to the AI.
    fn ask_talk(&mut self, ctx: &egui::Context, path: &Path, pid: &str, prefer: Option<Tool>) {
        let Some(page) = self.ws.page(pid) else {
            return;
        };
        let Some(pr) = page.project.as_ref() else {
            return;
        };
        let folder = pr.folder.clone();
        let about = About {
            card: card(&page.display_title(), pr),
            notes: crate::store::page_to_markdown(&self.ws, page),
        };
        let choice = self.ws.settings.text_model.clone();
        let Some(talk) = self.talks.open.get_mut(path) else {
            return;
        };
        let Some((last, before)) = talk.unanswered() else {
            return;
        };
        if talk.job.is_some() {
            return;
        }
        let message = last.text.clone();
        let before = before.to_vec();
        let slot: Slot = Arc::default();
        let (done, ctx) = (slot.clone(), ctx.clone());
        std::thread::spawn(move || {
            let history: Vec<Said> = before
                .iter()
                .map(|m| Said {
                    mine: m.mine,
                    text: &m.text,
                })
                .collect();
            // Made here: reading its key may take a moment.
            let model = OpenAi::chosen(&choice);
            let answer = crate::sync::talk::reply(
                &folder,
                &about,
                &history,
                &message,
                prefer,
                model.as_ref(),
            );
            *done.lock_or_recover() = Some(answer);
            ctx.request_repaint();
        });
        talk.project = pid.to_string();
        talk.error = None;
        talk.job = Some(slot);
    }

    /// Take in the answers that came, at each tick.
    pub(super) fn talk_tick(&mut self, ctx: &egui::Context) {
        let mut waiting = false;
        let mut proposed: Vec<(String, Vec<String>)> = vec![];
        for (path, talk) in &mut self.talks.open {
            let Some(slot) = &talk.job else {
                continue;
            };
            let Some(answer) = slot.lock_or_recover().take() else {
                waiting = true;
                continue;
            };
            talk.job = None;
            let steps = talk.take(path, answer);
            if !steps.is_empty() {
                proposed.push((talk.project.clone(), steps));
            }
        }
        for (pid, steps) in proposed {
            self.propose_steps(&pid, steps);
        }
        if waiting {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// The conversation of a project, in the « AI conversation » tab of the panel.
    pub(super) fn project_talk(
        &mut self,
        ui: &mut egui::Ui,
        pid: &str,
        prefer: Option<Tool>,
        t: &Tone,
    ) {
        let Some(path) = file_of(pid) else {
            ui.label(
                RichText::new(crate::tr!(
                    "Ce projet ne peut pas avoir de conversation.",
                    "This project can't have a conversation."
                ))
                .color(t.tertiary),
            );
            return;
        };
        let service = Service::from_id(&self.ws.settings.text_model.service);
        let who = match service {
            Some(s) => s.label(),
            None => crate::tr!("Claude Code ou Codex", "Claude Code or Codex"),
        };
        let talk = self
            .talks
            .open
            .entry(path.clone())
            .or_insert_with(|| Talk::load(&path));
        let busy = talk.job.is_some();
        let (mut send, mut retry) = (None, false);

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(crate::trf!("Avec {who}", "With {who}"))
                    .font(semibold(13.0))
                    .color(t.text),
            );
            if talk.messages.is_empty() && talk.unreadable.is_none() {
                return;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if talk.clearing {
                    crate::tr!("Confirmer l'effacement", "Confirm the erasing")
                } else {
                    crate::tr!("Effacer", "Erase")
                };
                let r = ui
                    .add_enabled_ui(!busy, |ui| super::projects::action_link(ui, label))
                    .inner
                    .on_hover_text(crate::tr!(
                        "Efface toute la conversation de ce projet, définitivement.",
                        "Erases the whole conversation of this project, for good."
                    ));
                if r.clicked() {
                    if talk.clearing {
                        talk.messages.clear();
                        talk.error = None;
                        talk.unreadable = None;
                        talk.save(&path);
                    }
                    talk.clearing = !talk.clearing;
                } else if talk.clearing && r.clicked_elsewhere() {
                    talk.clearing = false;
                }
            });
        });
        let sent = match service {
            Some(_) => crate::tr!(
                "Reçoit la conversation, la fiche et la page du projet, et un résumé de son dossier. Ne modifie rien. L'historique reste sur cet ordinateur.",
                "Gets the conversation, the project's card and page, and a summary of its folder. Changes nothing. The history stays on this computer."
            ),
            None => crate::tr!(
                "Reçoit la conversation, la fiche et la page du projet, peut lire son dossier et chercher sur le web. Ne modifie rien. L'historique reste sur cet ordinateur.",
                "Gets the conversation, the project's card and page, may read its folder and search the web. Changes nothing. The history stays on this computer."
            ),
        };
        ui.add(egui::Label::new(RichText::new(sent).size(12.0).color(t.tertiary)).wrap());
        ui.add_space(6.0);

        let composer_h = 90.0;
        egui::ScrollArea::vertical()
            .id_salt(("talk", pid))
            .max_height((ui.available_height() - composer_h).max(80.0))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(e) = &talk.unreadable {
                    warning(
                        ui,
                        &crate::trf!(
                            "Conversation illisible, laissée telle quelle : {e}",
                            "Conversation unreadable, left as it is: {e}"
                        ),
                    );
                } else if talk.messages.is_empty() {
                    ui.add_space(14.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(crate::tr!(
                                "Posez une question sur le projet, demandez un texte à écrire, ou les étapes pour avancer : vous cocherez celles à garder.",
                                "Ask a question about the project, ask for a text to write, or for the steps to move on: you will tick those to keep."
                            ))
                            .size(13.0)
                            .color(t.secondary),
                        )
                        .wrap(),
                    );
                }
                for m in &talk.messages {
                    bubble(ui, m, t);
                    ui.add_space(8.0);
                }
                if busy {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(
                            RichText::new(crate::tr!("L'IA répond…", "The AI is answering…"))
                                .size(12.5)
                                .color(t.secondary),
                        );
                    });
                } else if talk.unanswered().is_some() {
                    let why = match &talk.error {
                        Some(e) if e == crate::sync::AUTH_ERROR => {
                            crate::sync::auth_error_text().to_string()
                        }
                        Some(e) => crate::trf!("Pas de réponse : {e}", "No answer: {e}"),
                        None => crate::tr!(
                            "Ce message est resté sans réponse.",
                            "This message got no answer."
                        )
                        .to_string(),
                    };
                    warning(ui, &why);
                    retry = super::projects::action_link(ui, crate::tr!("Réessayer", "Try again"))
                        .clicked();
                }
                if let Some(e) = &talk.unsaved {
                    warning(
                        ui,
                        &crate::trf!(
                            "Conversation non enregistrée : {e}",
                            "Conversation not saved: {e}"
                        ),
                    );
                }
            });
        ui.add_space(6.0);
        let hint = crate::tr!(
            "Écrire à l'IA… (Entrée pour envoyer)",
            "Write to the AI… (Enter to send)"
        );
        // What is typed while the AI answers waits in the box.
        let ready = !busy && talk.unreadable.is_none();
        ui.add_enabled_ui(talk.unreadable.is_none(), |ui| {
            let r = ui.add(
                egui::TextEdit::multiline(&mut talk.draft)
                    .id_salt(("talk-draft", pid))
                    .hint_text(hint)
                    .char_limit(MESSAGE_MAX)
                    .return_key(egui::KeyboardShortcut::new(
                        egui::Modifiers::SHIFT,
                        Key::Enter,
                    ))
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            let enter = r.has_focus()
                && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
            if ready && enter && !talk.draft.trim().is_empty() {
                send = Some(std::mem::take(&mut talk.draft).trim().to_string());
            }
        });

        let again = retry || send.is_some();
        if let Some(text) = send {
            talk.push(
                &path,
                Message {
                    mine: true,
                    text,
                    at: crate::model::now_ms(),
                    ..Default::default()
                },
            );
        }
        if again {
            self.ask_talk(ui.ctx(), &path, pid, prefer);
        }
    }
}

fn warning(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(RichText::new(text).size(12.5).color(super::style::ORANGE)).wrap(),
    );
}

/// A message: the person's in a bubble, the AI's (Markdown) after a dot, with who wrote it.
fn bubble(ui: &mut egui::Ui, m: &Message, t: &Tone) {
    if m.mine {
        egui::Frame::new()
            .fill(t.fill)
            .corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(&m.text).size(13.5).color(t.text)).wrap());
            });
        return;
    }
    ui.horizontal_top(|ui| {
        ui.label(RichText::new("●").size(10.0).color(super::style::CORAL));
        let w = ui.available_width();
        ui.vertical(|ui| {
            ui.set_max_width(w);
            super::rich::markdown(ui, &m.text, 13.5, t);
            if m.steps > 0 {
                let s = crate::i18n::plural(m.steps);
                ui.add(
                    egui::Label::new(
                        RichText::new(crate::trf!(
                            "{} étape{s} proposée{s} dans l'encadré Étapes : cochez celles à garder.",
                            "{} step{s} proposed in the Steps box: tick the ones to keep.",
                            m.steps
                        ))
                        .size(12.5)
                        .color(super::style::GREEN),
                    )
                    .wrap(),
                );
            }
            ui.label(
                RichText::new(format!("{} · {}", m.by, relative_time(m.at)))
                    .size(11.5)
                    .color(t.tertiary),
            );
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("orchestra-side-chat-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn mine(text: &str) -> Message {
        Message {
            mine: true,
            text: text.into(),
            at: 1,
            ..Default::default()
        }
    }

    #[test]
    fn a_conversation_comes_back_as_it_was_and_erased_leaves_no_file() {
        let dir = scratch("kept");
        let path = file_in(&dir, "a6938e6c6766").unwrap();
        let mut talk = Talk::load(&path);
        assert!(talk.messages.is_empty() && talk.unreadable.is_none());
        talk.push(&path, mine("Par quoi commencer ?"));
        let steps = talk.take(
            &path,
            Ok((
                "Par le plan.\n```etapes\n- Plan : écrire le plan\n```".into(),
                "Codex".into(),
            )),
        );
        assert_eq!(steps, ["Plan : écrire le plan"]);
        let back = Talk::load(&path);
        assert_eq!(back.messages, talk.messages);
        assert_eq!(back.messages[1].text, "Par le plan.");
        assert_eq!((back.messages[1].steps, back.messages[1].by.as_str()), (1, "Codex"));
        assert!(back.unanswered().is_none());
        talk.messages.clear();
        talk.save(&path);
        assert_eq!(talk.unsaved, None);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_answer_leaves_the_message_waiting() {
        let dir = scratch("failed");
        let path = file_in(&dir, "p").unwrap();
        let mut talk = Talk::load(&path);
        talk.push(&path, mine("un"));
        talk.push(&path, mine("deux"));
        assert!(talk.take(&path, Err("panne".into())).is_empty());
        assert_eq!(talk.error.as_deref(), Some("panne"));
        // Still there at the next launch, to be sent again with what came before it.
        let back = Talk::load(&path);
        let (last, before) = back.unanswered().unwrap();
        assert_eq!((last.text.as_str(), before.len()), ("deux", 1));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_file_is_never_written_over() {
        let dir = scratch("unreadable");
        let path = file_in(&dir, "p").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "pas du JSON").unwrap();
        let mut talk = Talk::load(&path);
        assert!(talk.unreadable.is_some());
        talk.push(&path, mine("?"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "pas du JSON");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_the_latest_messages_are_kept() {
        let dir = scratch("many");
        let path = file_in(&dir, "p").unwrap();
        let mut talk = Talk::load(&path);
        talk.messages = (0..KEPT).map(|i| mine(&i.to_string())).collect();
        talk.push(&path, mine("dernier"));
        assert_eq!(talk.messages.len(), KEPT);
        assert_eq!(talk.messages[0].text, "1");
        assert_eq!(talk.messages[KEPT - 1].text, "dernier");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_project_id_never_leaves_the_conversations_folder() {
        let dir = Path::new("/x/conversations");
        assert_eq!(file_in(dir, "ab-1_2"), Some(dir.join("ab-1_2.json")));
        for evil in ["", "..", "../secret", "a/b", "a\\b", "/etc/passwd", "a.b"] {
            assert_eq!(file_in(dir, evil), None, "{evil}");
        }
    }
}
