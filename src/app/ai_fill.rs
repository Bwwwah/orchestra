//! The AI fills a project's cards (state of play, status, outside view) when the project is
//! added, or again on request; and it proposes steps only when asked: from the project, or
//! from a text the person pastes. Proposals wait for the person's choice.

use super::*;
use crate::sessions::Tool;
use crate::sync::Fill;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type Slot<T> = Arc<Mutex<Option<T>>>;

/// A request from a card, carried to the next tick (cards can't reach the app).
#[derive(Clone)]
pub enum AiReq {
    /// Fill the cards of this project.
    Fill {
        page_id: String,
        folder: String,
        prefer: Option<Tool>,
    },
    /// Fill the project that holds this folder, once it exists (just created).
    FillFolder { folder: String },
    /// Propose steps: from the project (`text` None) or from a pasted text and its joined
    /// images (file paths).
    Tasks {
        page_id: String,
        folder: String,
        existing: Vec<String>,
        prefer: Option<Tool>,
        text: Option<String>,
        images: Vec<String>,
    },
    /// A quick question about the project, answered in its box.
    Question {
        page_id: String,
        folder: String,
        question: String,
        prefer: Option<Tool>,
    },
}

/// The last quick question of a project and its answer (or why there is none).
#[derive(Clone)]
pub struct QuickAnswer {
    pub question: String,
    pub answer: Result<String, String>,
    /// Who answered ("Claude Code", "Codex").
    pub by: String,
    /// How many steps the answer proposed (added to the Steps box's proposals).
    pub steps: usize,
    /// (folder, command) reopening the AI's conversation in a terminal, to carry it on.
    pub resume: Option<(String, String)>,
}

fn req_key() -> egui::Id {
    egui::Id::unique("ai-fill-requests")
}

/// Ask for something from anywhere in the interface.
pub fn request(ctx: &egui::Context, r: AiReq) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<AiReq>>(req_key()).push(r));
}

/// A page filled by an AI, and which one, once done.
type FillSlot = Slot<Result<(Fill, Tool), String>>;
/// A quick question: the answer, the AI that gave it and its conversation, once done.
type AnswerSlot = Slot<Result<crate::sync::QuestionReply, String>>;

#[derive(Default)]
pub struct AiJobs {
    fills: HashMap<String, FillSlot>,
    /// Folders of just-created projects, waiting for their page (with when asked).
    pending: Vec<(String, Instant)>,
    tasks: HashMap<String, Slot<Result<Vec<String>, String>>>,
    /// Proposed steps per project, each ticked or not.
    pub proposals: HashMap<String, Vec<(String, bool)>>,
    /// Quick questions being answered, per project (with the question).
    questions: HashMap<String, (String, AnswerSlot)>,
    /// The last answer per project, shown in the question box until closed.
    pub answers: HashMap<String, QuickAnswer>,
}

impl AiJobs {
    /// The question being answered for this project, if any.
    pub fn asking(&self, page_id: &str) -> Option<&str> {
        self.questions.get(page_id).map(|(q, _)| q.as_str())
    }
    pub fn filling(&self, page_id: &str) -> bool {
        self.fills.contains_key(page_id)
    }
    pub fn tasking(&self, page_id: &str) -> bool {
        self.tasks.contains_key(page_id)
    }
}

fn spawn<T: Send + 'static>(
    ctx: &egui::Context,
    work: impl FnOnce() -> T + Send + 'static,
) -> Slot<T> {
    let slot: Slot<T> = Arc::default();
    let (s, c) = (slot.clone(), ctx.clone());
    std::thread::spawn(move || {
        let r = work();
        *s.lock_or_recover() = Some(r);
        c.request_repaint();
    });
    slot
}

/// Add `steps` to a project's proposals (ticked), less those already proposed or already in
/// the project (`existing`, same title whatever the case); how many were added.
fn merge_proposals(proposals: &mut Vec<(String, bool)>, existing: &[String], steps: Vec<String>) -> usize {
    let same = |a: &str, b: &str| a.trim().to_lowercase() == b.trim().to_lowercase();
    let mut n = 0;
    for s in steps {
        if existing.iter().any(|e| same(e, &s)) || proposals.iter().any(|(p, _)| same(p, &s)) {
            continue;
        }
        proposals.push((s, true));
        n += 1;
    }
    n
}

/// Write an AI's fill into a project: status, state of play, outside view. Steps untouched.
pub fn apply_fill(pr: &mut crate::project::Project, f: &Fill, by: &str) {
    if let Some(st) = f.status {
        pr.status = st;
    }
    if !f.done.is_empty() || !f.doing.is_empty() || !f.blockers.is_empty() || !f.notes.is_empty() {
        pr.set_summary(
            &crate::project::summary_from_parts(&f.done, &f.doing, &f.blockers, &f.notes),
            by,
        );
    }
    if !f.review.trim().is_empty() {
        pr.review = Some(crate::project::Review {
            text: crate::richtext::clean(&f.review, 2500),
            by: by.to_string(),
            at: crate::model::now_ms(),
        });
    }
    pr.touch();
}

impl NotesApp {
    pub(super) fn ai_tick(&mut self, ctx: &egui::Context) {
        let reqs: Vec<AiReq> = ctx
            .data_mut(|d| d.remove_temp::<Vec<AiReq>>(req_key()))
            .unwrap_or_default();
        for r in reqs {
            match r {
                AiReq::Fill {
                    page_id,
                    folder,
                    prefer,
                } => self.start_fill(ctx, page_id, folder, prefer),
                AiReq::FillFolder { folder } => self.ai.pending.push((folder, Instant::now())),
                AiReq::Tasks {
                    page_id,
                    folder,
                    existing,
                    prefer,
                    text,
                    images,
                } => {
                    if self.ai.tasks.contains_key(&page_id) {
                        continue;
                    }
                    let slot = spawn(ctx, move || match text {
                        Some(t) => crate::sync::text_to_tasks(&folder, &t, &images, prefer),
                        None => crate::sync::ask_tasks(&folder, &existing, prefer),
                    });
                    self.ai.tasks.insert(page_id, slot);
                }
                AiReq::Question {
                    page_id,
                    folder,
                    question,
                    prefer,
                } => {
                    if self.ai.questions.contains_key(&page_id) || question.trim().is_empty() {
                        continue;
                    }
                    let q = question.clone();
                    let code = self.code_forced(&page_id);
                    let slot = spawn(ctx, move || {
                        crate::sync::ask_question(&folder, &q, code, prefer)
                    });
                    self.ai.answers.remove(&page_id);
                    self.ai.questions.insert(page_id, (question, slot));
                }
            }
        }
        // Answered quick questions.
        let done: Vec<String> = self
            .ai
            .questions
            .iter()
            .filter(|(_, (_, s))| s.lock_or_recover().is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for id in done {
            let (question, slot) = self.ai.questions.remove(&id).unwrap();
            let res = slot.lock_or_recover().take().unwrap();
            let by = res
                .as_ref()
                .map(|r| r.tool.label().to_string())
                .unwrap_or_default();
            // Steps the answer proposes: to the Steps box, where they wait for the person's
            // choice like the other proposals (never added to the project by themselves).
            let (answer, steps, resume) = match res {
                Ok(r) => {
                    let (text, steps) = crate::sync::split_answer_steps(&r.answer);
                    (Ok(text), steps, r.resume)
                }
                Err(e) => (Err(e), vec![], None),
            };
            let existing: Vec<String> = self
                .ws
                .page(&id)
                .and_then(|p| p.project.as_ref())
                .map(|pr| pr.steps.iter().map(|s| s.title.clone()).collect())
                .unwrap_or_default();
            let n = merge_proposals(
                self.ai.proposals.entry(id.clone()).or_default(),
                &existing,
                steps,
            );
            if self.ai.proposals.get(&id).is_some_and(Vec::is_empty) {
                self.ai.proposals.remove(&id);
            }
            self.ai.answers.insert(
                id,
                QuickAnswer {
                    question,
                    answer,
                    by,
                    steps: n,
                    resume,
                },
            );
        }
        // Just-created projects: their page appears a frame later.
        let pending = std::mem::take(&mut self.ai.pending);
        for (folder, at) in pending {
            let dir = crate::project::normalize_folder(&folder);
            let found = self
                .ws
                .projects()
                .into_iter()
                .find(|p| {
                    p.project
                        .as_ref()
                        .is_some_and(|pr| crate::project::normalize_folder(&pr.folder) == dir)
                })
                .map(|p| p.id.clone());
            match found {
                Some(id) => self.start_fill(ctx, id, folder, None),
                None if at.elapsed().as_secs() < 10 => self.ai.pending.push((folder, at)),
                None => {}
            }
        }
        // Finished fills.
        let done: Vec<String> = self
            .ai
            .fills
            .iter()
            .filter(|(_, s)| s.lock_or_recover().is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for id in done {
            let res = self
                .ai
                .fills
                .remove(&id)
                .unwrap()
                .lock_or_recover()
                .take()
                .unwrap();
            match res {
                Ok((f, tool)) => {
                    if let Some(pr) = self.ws.page_mut(&id).and_then(|p| p.project.as_mut()) {
                        apply_fill(pr, &f, tool.label());
                    }
                    self.detect_changes();
                    self.info(crate::trf!(
                        "{} a rempli le projet.",
                        "{} filled in the project.",
                        tool.label()
                    ));
                }
                Err(e) => self.fail(crate::trf!(
                    "Remplissage par l'IA impossible : {e}",
                    "The AI couldn't fill in the project: {e}"
                )),
            }
        }
        // Finished task proposals.
        let done: Vec<String> = self
            .ai
            .tasks
            .iter()
            .filter(|(_, s)| s.lock_or_recover().is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for id in done {
            let res = self
                .ai
                .tasks
                .remove(&id)
                .unwrap()
                .lock_or_recover()
                .take()
                .unwrap();
            match res {
                Ok(list) => {
                    self.ai
                        .proposals
                        .insert(id, list.into_iter().map(|t| (t, true)).collect());
                }
                Err(e) => self.fail(crate::trf!(
                    "Pas de tâches proposées : {e}",
                    "No task proposed: {e}"
                )),
            }
        }
        if !self.ai.fills.is_empty()
            || !self.ai.tasks.is_empty()
            || !self.ai.pending.is_empty()
            || !self.ai.questions.is_empty()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    fn start_fill(
        &mut self,
        ctx: &egui::Context,
        page_id: String,
        folder: String,
        prefer: Option<Tool>,
    ) {
        if self.ai.fills.contains_key(&page_id)
            || folder.trim().is_empty()
            || !crate::project::normalize_folder(&folder).is_dir()
        {
            return;
        }
        let f = crate::project::normalize_folder(&folder)
            .display()
            .to_string();
        let code = self.code_forced(&page_id);
        let slot = spawn(ctx, move || crate::sync::ask_fill(&f, code, prefer));
        self.ai.fills.insert(page_id, slot);
    }

    /// The code / non-code kind the person forced on this project, if any.
    pub(super) fn code_forced(&self, page_id: &str) -> Option<bool> {
        self.ws
            .page(page_id)
            .and_then(|p| p.project.as_ref())
            .and_then(|pr| pr.code)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn steps_of_an_answer_join_the_proposals_once() {
        let mut p = vec![("Tests : écrire le test du lecteur".to_string(), false)];
        let existing = ["Interface : ajouter un bouton".to_string()];
        let n = super::merge_proposals(
            &mut p,
            &existing,
            vec![
                "interface : ajouter un bouton".into(),
                "Tests : écrire le test du lecteur".into(),
                "Docs : décrire les sessions".into(),
            ],
        );
        assert_eq!(n, 1);
        assert_eq!(p.len(), 2);
        // The person's choice on an older proposal is kept; the new one comes ticked.
        assert!(!p[0].1);
        assert_eq!(p[1], ("Docs : décrire les sessions".to_string(), true));
    }
}
