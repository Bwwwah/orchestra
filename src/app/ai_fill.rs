//! The AI fills a project's cards (state of play, status, outside view) when the project is
//! added, or again on request; and it proposes steps only when asked: from the project, or
//! from a text the person pastes. Proposals wait for the person's choice.

use super::*;
use crate::model::TextModel;
use crate::sessions::Tool;
use crate::sync::Fill;
use crate::sync::openai::OpenAi;
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
        /// Framing: the AI gives its opinion on an idea and asks its questions, no step yet.
        framing: bool,
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
    /// The AI's conversation, to carry it on (shown in the right-hand panel).
    pub resume: Option<crate::sync::Resume>,
    /// The AI that answered, to send replies to the same one.
    pub tool: Option<Tool>,
    /// The exchanges before this one in the thread (question, answer, who answered).
    pub earlier: Vec<Exchange>,
}

/// One earlier exchange of a quick question's thread: question, answer, who answered.
pub type Exchange = (String, Result<String, String>, String, usize);

fn req_key() -> egui::Id {
    egui::Id::unique("ai-fill-requests")
}

/// Ask for something from anywhere in the interface.
pub fn request(ctx: &egui::Context, r: AiReq) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<AiReq>>(req_key()).push(r));
}

/// A page filled by an AI, and who did it, once done.
type FillSlot = Slot<Result<(Fill, String), String>>;
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
    /// Replies being sent on a thread: the thread so far, kept until the AI's answer comes
    /// to be added to it.
    carried: HashMap<String, QuickAnswer>,
    /// Projects whose reply being sent asks for the validated plan.
    plan_asked: std::collections::HashSet<String>,
    /// The settings' choice of who fills in and suggests steps, copied at each tick for the
    /// cards to say it (they can't reach the settings).
    pub text_model: TextModel,
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
    /// An AI is filling a page, proposing steps or answering a question.
    pub fn any(&self) -> bool {
        !self.fills.is_empty()
            || !self.pending.is_empty()
            || !self.tasks.is_empty()
            || !self.questions.is_empty()
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

/// What the dashboard holds of a project, for a model that can't ask for it: its title and
/// steps. The state of play is left out: filling in writes a new one.
fn known(title: &str, steps: &[crate::project::Step]) -> String {
    let mut out = title.trim().to_string();
    // The open ones first: a long list is cut short at the end.
    let (done, open): (Vec<_>, Vec<_>) = steps
        .iter()
        .partition(|s| s.status == crate::project::StepStatus::Done);
    for s in open.iter().chain(done.iter().rev()) {
        let mark = match s.status {
            crate::project::StepStatus::Done => "x",
            _ => " ",
        };
        out.push_str(&format!("\n- [{mark}] {}", s.title));
    }
    out
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

/// Write a validated plan (Markdown) into a project's page, under a « Plan validé » heading;
/// an earlier plan (that heading and what follows up to the next heading 1) is replaced.
pub fn put_plan(page: &mut crate::model::Page, plan: &str, by: &str) {
    use crate::model::BlockKind;
    let title = crate::tr!("Plan validé", "Validated plan");
    // The first heading of a Markdown text is taken as the page's title: a throwaway one first.
    // The parser reads « ## » as a heading 1: the plan's headings go one level down, so that
    // they stay inside the section.
    let plan: String = plan
        .lines()
        .map(|l| match l.trim_start() {
            h if h.starts_with('#') => format!("###{}", h.trim_start_matches('#')),
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let md = format!("# _\n# {title}\n{}\n\n{plan}\n", crate::trf!("Par {by}", "By {by}"));
    let mut blocks = crate::store::page_from_markdown(&md, "_").blocks;
    blocks.retain(|b| !(b.kind == BlockKind::Paragraph && b.text.is_empty()));
    if let Some(start) = page
        .blocks
        .iter()
        .position(|b| b.kind == BlockKind::Heading1 && b.text.trim() == title)
    {
        let end = page.blocks[start + 1..]
            .iter()
            .position(|b| b.kind == BlockKind::Heading1)
            .map_or(page.blocks.len(), |i| start + 1 + i);
        let t = crate::model::now_ms();
        for b in page.blocks.drain(start..end) {
            page.deleted_blocks.insert(b.id, t);
        }
        for (i, b) in blocks.into_iter().enumerate() {
            page.blocks.insert(start + i, b);
        }
    } else {
        if page.blocks.len() == 1 && page.blocks[0].text.is_empty() && page.blocks[0].kind == BlockKind::Paragraph {
            page.blocks.clear();
        }
        page.blocks.extend(blocks);
    }
    page.touch();
}

impl NotesApp {
    fn write_plan(&mut self, page_id: &str, plan: &str, by: &str) {
        if let Some(p) = self.ws.page_mut(page_id) {
            put_plan(p, plan, by);
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    fn known_of(&self, page_id: &str) -> String {
        self.ws
            .page(page_id)
            .and_then(|p| p.project.as_ref().map(|pr| known(&p.title, &pr.steps)))
            .unwrap_or_default()
    }

    pub(super) fn ai_tick(&mut self, ctx: &egui::Context) {
        self.ai.text_model.clone_from(&self.ws.settings.text_model);
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
                    let choice = self.ws.settings.text_model.clone();
                    let known = self.known_of(&page_id);
                    let slot = spawn(ctx, move || {
                        // Made here: reading its key may take a moment.
                        let model = OpenAi::chosen(&choice);
                        let model = model.as_ref();
                        match text {
                            Some(t) => {
                                crate::sync::text_to_tasks(&folder, &t, &images, prefer, model)
                            }
                            None => {
                                crate::sync::ask_tasks(&folder, &existing, prefer, model, &known)
                            }
                        }
                    });
                    self.ai.tasks.insert(page_id, slot);
                }
                AiReq::Question {
                    page_id,
                    folder,
                    question,
                    prefer,
                    framing,
                } => {
                    if self.ai.questions.contains_key(&page_id) || question.trim().is_empty() {
                        continue;
                    }
                    let q = question.clone();
                    let code = self.code_forced(&page_id);
                    let slot = spawn(ctx, move || {
                        crate::sync::ask_question(&folder, &q, code, prefer, framing)
                    });
                    self.ai.answers.remove(&page_id);
                    self.ai.questions.insert(page_id, (question, slot));
                }
            }
        }
        // Replies typed under a thread: sent on the same conversation.
        let replies: Vec<super::side::ThreadReply> = ctx
            .data_mut(|d| d.remove_temp::<Vec<super::side::ThreadReply>>(super::side::reply_key()))
            .unwrap_or_default();
        for r in replies {
            if self.ai.questions.contains_key(&r.project) || r.text.trim().is_empty() {
                continue;
            }
            let Some(prev) = self.ai.answers.get(&r.project).cloned() else {
                continue;
            };
            let (Some(resume), Some(tool)) = (prev.resume.clone(), prev.tool) else {
                self.fail(crate::tr!(
                    "Cette conversation ne peut pas être poursuivie ici.",
                    "This conversation can't be carried on here."
                ));
                continue;
            };
            let text = r.text.clone();
            let slot = spawn(ctx, move || crate::sync::ask_followup(&resume, tool, &text));
            self.ai.carried.insert(r.project.clone(), prev);
            // The plan's long request shows in the thread as a short line.
            let shown = if r.plan {
                self.ai.plan_asked.insert(r.project.clone());
                crate::tr!("Valider le plan", "Validate the plan").to_string()
            } else {
                r.text
            };
            self.ai.questions.insert(r.project, (shown, slot));
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
            let carried = self.ai.carried.remove(&id);
            let by = res
                .as_ref()
                .map(|r| r.tool.label().to_string())
                .unwrap_or_default();
            // Steps the answer proposes: to the Steps box, where they wait for the person's
            // choice like the other proposals (never added to the project by themselves).
            let tool = match &res {
                Ok(r) => Some(r.tool),
                Err(_) => carried.as_ref().and_then(|c| c.tool),
            };
            let plan_asked = self.ai.plan_asked.remove(&id);
            let (answer, steps, resume) = match res {
                Ok(r) => {
                    let (text, steps) = crate::sync::split_answer_steps(&r.answer);
                    // The validated plan goes into the project's notes, out of the thread.
                    let text = if plan_asked {
                        let (rest, plan) = crate::sync::split_answer_plan(&text);
                        match plan {
                            Some(p) => {
                                self.write_plan(&id, &p, &r.tool.label());
                                let done = crate::tr!(
                                    "Plan écrit dans les notes du projet.",
                                    "Plan written into the project's notes."
                                );
                                if rest.is_empty() { done.to_string() } else { format!("{rest}\n\n{done}") }
                            }
                            None => format!(
                                "{rest}\n\n{}",
                                crate::tr!(
                                    "L'IA n'a pas écrit de plan : réessayez « Valider le plan ».",
                                    "The AI wrote no plan: try « Validate the plan » again."
                                )
                            ),
                        }
                    } else {
                        text
                    };
                    (Ok(text), steps, r.resume)
                }
                // A reply that failed keeps the conversation it was sent on.
                Err(e) => (
                    Err(e),
                    vec![],
                    carried.as_ref().and_then(|c| c.resume.clone()),
                ),
            };
            // The thread so far, then this exchange.
            let earlier: Vec<Exchange> = carried
                .map(|mut c| {
                    let mut e = std::mem::take(&mut c.earlier);
                    e.push((c.question, c.answer, c.by, c.steps));
                    e
                })
                .unwrap_or_default();
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
                    tool,
                    earlier,
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
                Ok((f, by)) => {
                    if let Some(pr) = self.ws.page_mut(&id).and_then(|p| p.project.as_mut()) {
                        apply_fill(pr, &f, &by);
                    }
                    self.detect_changes();
                    self.info(crate::trf!(
                        "{} a rempli le projet.",
                        "{} filled in the project.",
                        by
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
        let choice = self.ws.settings.text_model.clone();
        let known = self.known_of(&page_id);
        let slot = spawn(ctx, move || {
            let model = OpenAi::chosen(&choice);
            crate::sync::ask_fill(&f, code, prefer, model.as_ref(), &known)
        });
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
    fn a_validated_plan_is_written_once_and_replaced_by_the_next() {
        let mut page = crate::store::page_from_markdown("# Projet\nIntro\n\n# Autre\nFin", "P");
        super::put_plan(&mut page, "## Objectif\n- A\n## Périmètre\n- B", "Claude Code");
        let titles = |p: &crate::model::Page| p.blocks.iter().map(|b| b.text.clone()).collect::<Vec<_>>();
        let n = page.blocks.len();
        assert!(titles(&page).iter().any(|t| t == "Objectif" || t == "A"));
        super::put_plan(&mut page, "## Objectif\n- A2", "Codex");
        let t = titles(&page);
        assert!(t.iter().any(|x| x == "A2") && !t.iter().any(|x| x == "A"), "{t:?}");
        assert!(t.iter().any(|x| x == "Autre") && t.iter().any(|x| x == "Intro"));
        assert_eq!(t.iter().filter(|x| x.starts_with("Plan") || x.starts_with("Validated")).count(), 1);
        assert!(page.blocks.len() < n);
    }

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
