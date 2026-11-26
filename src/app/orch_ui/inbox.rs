//! The inbox, where the AIs' questions, the ends of their sessions and the reports
//! arrive; and the questions as shown elsewhere.

use super::*;

impl NotesApp {
    /// `save_answer` for the inbox message `id` (nothing if it is gone meanwhile).
    pub(in crate::app) fn save_answer_to(&mut self, id: &str, text: String) {
        if let Some(i) = self.ws.inbox.iter().position(|m| m.id == id) {
            self.save_answer(i, text);
        }
    }

    /// The AIs' questions are answered in the inbox (and on each project's page): here, one
    /// line saying how many wait, and the way there. False when none waits.
    pub(super) fn questions_card(&mut self, ui: &mut egui::Ui, t: &Tone) -> bool {
        let waiting = self
            .ws
            .inbox
            .iter()
            .filter(|m| m.waits_for_person())
            .count();
        if waiting == 0 {
            return false;
        }
        let mut go = false;
        super::projects::card(ui, t, 0.0, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 16.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, super::style::ORANGE);
                ui.label(
                    RichText::new(if waiting > 1 {
                        crate::trf!("{waiting} questions des IA attendent votre réponse", "{waiting} AI questions waiting for your answer")
                    } else {
                        crate::tr!("Une question d'une IA attend votre réponse", "An AI question is waiting for your answer").to_string()
                    })
                    .font(semibold(super::style::BODY))
                    .color(t.text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::style::secondary(ui, crate::tr!("Répondre", "Answer"))
                        .on_hover_text(crate::tr!("Dans la boîte de réception ; une session en pause reprend une fois répondu.", "In the inbox; a paused session goes on once answered."))
                        .clicked()
                    {
                        go = true;
                    }
                });
            });
        });
        if go {
            self.view = View::Inbox;
        }
        true
    }

    /// The person answers question `i` of the inbox (from the inbox, the Orchestration
    /// page or the project's page): kept with the question, read by the AI at the
    /// project's next state of play.
    pub(super) fn save_answer(&mut self, i: usize, text: String) {
        let Some(m) = self.ws.inbox.get_mut(i) else {
            return;
        };
        m.answer = text;
        m.answered_at = crate::model::now_ms();
        m.read = true;
        self.dirty_since.get_or_insert_with(Instant::now);
        self.ack(crate::tr!(
            "Réponse enregistrée : l'IA la verra au prochain état des lieux du projet.",
            "Answer saved: the AI will see it in the project's next state of play."
        ));
    }

    // ------------------------------------------------------------------ the inbox

    pub(in crate::app) fn inbox_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let t = tone(ui.visuals().dark_mode);
        let avail = ui.available_width();
        let width = 860.0f32.min(avail - 40.0).max(320.0);
        let mut open: Option<usize> = None;
        let mut remove: Option<usize> = None;
        let mut all_read = false;
        let mut clear_all = false;
        let mut answer: Option<(usize, String)> = None;
        let mut run_act: Option<(String, &'static str)> = None;
        let confirm = self.copy_states();
        let paused: Vec<(String, String, Wait)> = self
            .orch
            .runs
            .iter()
            .filter_map(|r| Some((r.id.clone(), r.title.clone(), r.waiting?)))
            .collect();
        let mut unblock: Option<Unblock> = None;
        ui.add_space(50.0);
        ui.horizontal(|ui| {
            ui.add_space(((avail - width) / 2.0).max(20.0));
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(crate::tr!("Boîte de réception", "Inbox")).font(bold(super::style::TITLE)).color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // "Tout supprimer": a second click confirms, nothing goes on one slip.
                        let confirm_id = egui::Id::unique("inbox-clear-confirm");
                        let confirming = ui.data(|d| d.get_temp::<bool>(confirm_id)).unwrap_or(false);
                        if !self.ws.inbox.is_empty() {
                            if confirming {
                                if super::style::quiet(ui, crate::tr!("Annuler", "Cancel")).clicked() {
                                    ui.data_mut(|d| d.insert_temp(confirm_id, false));
                                }
                                let n = self.ws.inbox.len();
                                let sure = crate::trf!("Confirmer : supprimer les {n} messages", "Confirm: delete the {n} messages");
                                if ui.add(egui::Label::new(RichText::new(sure).size(13.0).color(super::style::RED)).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    clear_all = true;
                                    ui.data_mut(|d| d.insert_temp(confirm_id, false));
                                }
                            } else if super::style::quiet(ui, crate::tr!("Tout supprimer", "Delete all")).clicked() {
                                ui.data_mut(|d| d.insert_temp(confirm_id, true));
                            }
                        }
                        if !confirming && self.ws.unread() > 0 {
                            if super::style::quiet(ui, crate::tr!("Tout marquer comme lu", "Mark all as read")).clicked() {
                                all_read = true;
                            }
                        }
                    });
                });
                ui.label(RichText::new(crate::tr!("Questions des IA, mentions, fins de session, rapports et messages de l'application.", "AI questions, mentions, session ends, reports and the app's messages.")).color(t.secondary));
                ui.add_space(14.0);
                // Filters: everything, the questions still waiting for an answer, the unread.
                let filter_id = inbox_filter_id();
                let mut filter = ui.data(|d| d.get_temp::<u8>(filter_id)).unwrap_or(0);
                let waiting = self.ws.inbox.iter().filter(|m| m.waits_for_person()).count();
                let unread = self.ws.unread();
                let filters = ui.horizontal(|ui| {
                    for (k, label, n) in [
                        (0u8, crate::tr!("Tout", "All"), self.ws.inbox.iter().filter(|m| !m.being_sorted()).count()),
                        (1, crate::tr!("À répondre", "To answer"), waiting),
                        (2, crate::tr!("Non lus", "Unread"), unread),
                    ] {
                        if filter_chip(ui, label, n, filter == k, &t).clicked() {
                            filter = k;
                        }
                    }
                });
                super::tutorial::mark(ui, "inbox:filters", filters.response.rect);
                ui.data_mut(|d| d.insert_temp(filter_id, filter));
                ui.add_space(12.0);
                let shown: Vec<usize> = (0..self.ws.inbox.len())
                    .rev()
                    .filter(|&i| {
                        let m = &self.ws.inbox[i];
                        // With the responder: not the person's yet.
                        if m.being_sorted() {
                            return false;
                        }
                        match filter {
                            1 => m.waits_for_person(),
                            2 => !m.read,
                            _ => true,
                        }
                    })
                    .collect();
                if shown.is_empty() {
                    let empty = match filter {
                        1 => crate::tr!("Aucune question en attente de réponse.", "No question waiting for an answer."),
                        2 => crate::tr!("Tout est lu.", "Everything is read."),
                        _ => crate::tr!("Rien pour l'instant.", "Nothing yet."),
                    };
                    ui.label(RichText::new(empty).color(t.tertiary).italics());
                }
                let (now, offset) = (crate::model::now_ms(), auto::local_offset_ms());
                let mut group = "";
                let mut first_card = true;
                for i in shown {
                    let m = &self.ws.inbox[i];
                    // A heading per day, newest first.
                    let g = day_group(m.at, now, offset);
                    if g != group {
                        if !group.is_empty() {
                            ui.add_space(10.0);
                        }
                        group = g;
                        ui.label(RichText::new(g.to_uppercase()).font(semibold(11.5)).color(t.tertiary));
                        ui.add_space(6.0);
                    }
                    let to_answer = m.waits_for_person();
                    let (accent, kind) = match m.kind.as_str() {
                        "question" => (super::style::PURPLE, crate::tr!("Question", "Question")),
                        "fin" => (super::projects::GREEN, crate::tr!("Session", "Session")),
                        "rapport" => (super::projects::BLUE, crate::tr!("Rapport", "Report")),
                        "erreur" => (super::style::RED, crate::tr!("Erreur", "Error")),
                        crate::mentions::KIND => (super::projects::ORANGE, "Mention"),
                        _ => (t.tertiary, crate::tr!("Info", "Info")),
                    };
                    let project = (!m.project.is_empty()).then(|| self.ws.page(&m.project).map(|p| p.display_title())).flatten();
                    // The end of a session: its work, still in its own copy, to merge or discard.
                    let run = run_of_message(m, &self.ws.runs, |id| self.orch.runs.iter().any(|l| l.id == id) || self.orch.to_main.iter().any(|x| x == id));
                    let bg = super::projects::hover_begin(ui);
                    let card = egui::Frame::new()
                        .fill(if m.read { Color32::TRANSPARENT } else { t.card })
                        // A question still waiting for its answer stands out.
                        .stroke(egui::Stroke::new(1.0, if to_answer { accent.gamma_multiply(0.55) } else { t.border }))
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin { left: 16, right: 10, top: 10, bottom: 10 })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let head = ui.horizontal_top(|ui| {
                                let text = ui.vertical(|ui| {
                                    ui.set_width(ui.available_width() - 30.0);
                                    ui.horizontal(|ui| {
                                        kind_badge(ui, kind, accent);
                                        // A question says at once whether it still waits for the person.
                                        if m.kind == "question" {
                                            if to_answer && m.triage == crate::model::TRIAGE_ACTION {
                                                kind_badge(ui, crate::tr!("À faire", "To do"), super::projects::ORANGE);
                                            } else if to_answer {
                                                kind_badge(ui, crate::tr!("À répondre", "To answer"), super::projects::ORANGE);
                                            } else if m.triage == crate::model::TRIAGE_ANSWERED {
                                                kind_badge(ui, crate::tr!("Répondue par l'IA", "Answered by the AI"), super::projects::GREEN);
                                            } else {
                                                kind_badge(ui, crate::tr!("Répondue", "Answered"), super::projects::GREEN);
                                            }
                                        }
                                        if let Some(p) = &project {
                                            // The project's own colour, as on the Orchestration page.
                                            ui.add_space(2.0);
                                            let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 14.0), egui::Sense::hover());
                                            ui.painter().circle_filled(dot.center(), 3.5, project_color(p));
                                            ui.label(RichText::new(p).size(12.0).color(t.secondary));
                                        }
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(RichText::new(super::chrome::relative_time(m.at)).size(12.0).color(t.tertiary));
                                            if !m.read {
                                                let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                                                ui.painter().circle_filled(dot.center(), 3.5, super::projects::BLUE);
                                            }
                                        });
                                    });
                                    ui.add_space(3.0);
                                    ui.add(egui::Label::new(RichText::new(&m.title).font(if m.read { FontId::proportional(14.5) } else { semibold(14.5) }).color(t.text)).wrap());
                                    let (question_text, analysis) = split_analysis(&m.text);
                                    let body = plain_text(question_text);
                                    if !body.is_empty() {
                                        ui.add_space(2.0);
                                        // Long texts (session summaries) folded to a few lines.
                                        let more_id = egui::Id::unique(("inbox-more", &m.id));
                                        let all = ui.data(|d| d.get_temp::<bool>(more_id)).unwrap_or(false);
                                        let mut job = egui::text::LayoutJob::single_section(body, egui::TextFormat::simple(FontId::proportional(13.0), t.secondary));
                                        job.wrap.max_width = ui.available_width();
                                        if !all {
                                            job.wrap.max_rows = TEXT_LINES;
                                        }
                                        let galley = ui.fonts_mut(|f| f.layout_job(job));
                                        let cut = galley.elided;
                                        ui.label(galley);
                                        if (cut || all) && super::projects::action_link(ui, if all { crate::tr!("Réduire", "Show less") } else { crate::tr!("Afficher tout", "Show all") }).clicked() {
                                            ui.data_mut(|d| d.insert_temp(more_id, !all));
                                        }
                                    }
                                    // The responder's analysis, apart and whole: it is what helps to decide.
                                    if let Some(a) = analysis.map(plain_text).filter(|a| !a.is_empty()) {
                                        ui.add_space(6.0);
                                        egui::Frame::new()
                                            .fill(accent.gamma_multiply(0.08))
                                            .corner_radius(8.0)
                                            .inner_margin(egui::Margin::symmetric(10, 8))
                                            .show(ui, |ui| {
                                                ui.set_width(ui.available_width());
                                                ui.label(RichText::new(crate::tr!("Analyse du répondeur", "Responder's analysis")).font(semibold(12.0)).color(accent));
                                                ui.add_space(3.0);
                                                ui.add(egui::Label::new(RichText::new(a).size(13.0).color(t.text)).wrap());
                                            });
                                    }
                                });
                                let x = super::icons::delete_cross(ui, 22.0, t.tertiary, t.text, t.selected, crate::tr!("Supprimer", "Delete"));
                                if x.clicked() {
                                    remove = Some(i);
                                }
                                text.response.rect
                            });
                            // Only the text of the head opens the page: the answer box below stays free to
                            // type in, and the × keeps its click (a zone over it would take it).
                            let resp = ui.interact(head.inner, egui::Id::unique(("inbox", &m.id)), egui::Sense::click());
                            let resp = if m.page.is_some() || !m.project.is_empty() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
                            if resp.clicked() && remove.is_none() {
                                open = Some(i);
                            }
                            if m.kind == "question" {
                                ui.add_space(6.0);
                                if let Some(text) = answer_box(ui, m, accent, &t) {
                                    answer = Some((i, text));
                                }
                            }
                            if let Some(a) = inbox_unblock(m, &self.ws.inbox, &self.ws.runs, run, &paused) {
                                ui.add_space(6.0);
                                if super::style::secondary(ui, a.label()).clicked() {
                                    unblock = Some(a);
                                }
                            }
                            if let Some(r) = run {
                                let asking = confirm.iter().find(|(id, _)| *id == r.id).map(|(_, a)| *a);
                                if let Some(a) = run_actions(ui, r, asking, &t) {
                                    run_act = Some((r.id.clone(), a));
                                }
                            }
                        });
                    let rect = card.response.rect;
                    if std::mem::take(&mut first_card) {
                        super::tutorial::mark(ui, "inbox:message", rect);
                    }
                    // A coloured edge on the left: the kind at a glance.
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(rect.left_top() + egui::vec2(0.0, 8.0), egui::pos2(rect.left() + 3.0, rect.bottom() - 8.0)),
                        2.0,
                        accent.gamma_multiply(if m.read { 0.45 } else { 1.0 }),
                    );
                    super::projects::hover_end(ui, bg, rect.shrink2(egui::vec2(6.0, 2.0)), egui::Id::unique(("inbox-h", &m.id)), &t);
                    ui.add_space(8.0);
                }
            });
        });
        if clear_all {
            let n = self.ws.inbox.len();
            for m in self.ws.inbox.drain(..) {
                self.inbox_removed.insert(m.id);
            }
            self.dirty_since.get_or_insert_with(Instant::now);
            self.ack(crate::trf!(
                "{n} messages supprimés.",
                "{n} messages deleted."
            ));
            return;
        }
        if all_read {
            for m in self.ws.inbox.iter_mut() {
                m.read = true;
            }
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        if let Some((i, text)) = answer {
            self.save_answer(i, text);
        }
        match unblock {
            Some(Unblock::Answer) => {
                ui.data_mut(|d| d.insert_temp(inbox_filter_id(), 1u8));
            }
            Some(Unblock::Terminal(id)) => {
                self.orch.watching = Some(id);
                self.view = View::Orchestra;
            }
            Some(Unblock::Steps(project)) => {
                super::projects::focus_steps(ui.ctx(), &project);
                actions.push(Action::Open(project));
            }
            None => {}
        }
        if let Some((id, a)) = run_act {
            self.run_action(&ui.ctx().clone(), &id, a);
        }
        if let Some(i) = remove {
            let m = self.ws.inbox.remove(i);
            self.inbox_removed.insert(m.id);
            self.dirty_since.get_or_insert_with(Instant::now);
        } else if let Some(i) = open {
            let m = &mut self.ws.inbox[i];
            m.read = true;
            if let Some(p) = m.page.clone().filter(|p| !p.is_empty()) {
                actions.push(Action::Open(p));
            } else if !m.project.is_empty() {
                actions.push(Action::Open(m.project.clone()));
            }
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    /// The finished sessions whose work is still in their copy while their end message is
    /// gone from the inbox (its links went with it): listed here, with the same links.
    /// Shown only when there is one; returns whether it was.
    pub(super) fn orphans_card(&mut self, ui: &mut egui::Ui, t: &Tone, actions: &mut Vec<Action>) -> bool {
        let busy = |id: &str| {
            self.orch.runs.iter().any(|l| l.id == id) || self.orch.to_main.iter().any(|x| x == id)
        };
        let list = orphan_copies(&self.ws.runs, &self.ws.inbox, busy, |w| {
            std::path::Path::new(w).is_dir()
        });
        if list.is_empty() {
            return false;
        }
        let confirm = self.copy_states();
        let mut act: Option<(String, &'static str)> = None;
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(
                ui,
                crate::tr!("COPIES À TRAITER", "COPIES TO DEAL WITH"),
                &list.len().to_string(),
                t,
            );
            for &i in &list {
                let r = &self.ws.runs[i];
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(&r.title).font(semibold(14.0)).color(t.text));
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            auto::local_day(r.started),
                            auto::hm(r.started)
                        ))
                        .size(12.5)
                        .color(t.tertiary),
                    );
                    if let Some(p) = &r.report
                        && super::projects::action_link(ui, crate::tr!("Rapport", "Report"))
                            .clicked()
                    {
                        act = Some((p.clone(), "report"));
                    }
                });
                let asking = confirm.iter().find(|(id, _)| *id == r.id).map(|(_, a)| *a);
                if let Some(a) = run_actions(ui, r, asking, t) {
                    act = Some((r.id.clone(), a));
                }
                ui.add_space(6.0);
            }
        });
        match act {
            Some((page, "report")) => actions.push(Action::Open(page)),
            Some((id, a)) => self.run_action(&ui.ctx().clone(), &id, a),
            None => {}
        }
        true
    }
}

/// The session an end-of-session message speaks of (by its report page), once it is over
/// and not `busy` (running again, or about to be merged into main by itself).
pub(super) fn run_of_message<'a>(
    m: &crate::model::InboxMsg,
    runs: &'a [crate::model::RunRecord],
    busy: impl Fn(&str) -> bool,
) -> Option<&'a crate::model::RunRecord> {
    if m.kind != "fin" {
        return None;
    }
    let page = m.page.as_ref().filter(|p| !p.is_empty())?;
    runs.iter()
        .find(|r| r.report.as_ref() == Some(page))
        .filter(|r| !r.outcome.is_empty() && !busy(&r.id))
}

/// The finished sessions (indices in `runs`) whose work waits in a copy still on disk,
/// not merged nor discarded, not `busy`, and whose end message is no longer in the inbox.
pub(super) fn orphan_copies(
    runs: &[crate::model::RunRecord],
    inbox: &[crate::model::InboxMsg],
    busy: impl Fn(&str) -> bool,
    copy_exists: impl Fn(&str) -> bool,
) -> Vec<usize> {
    runs.iter()
        .enumerate()
        .filter(|(_, r)| !r.outcome.is_empty() && r.closed.is_empty() && !busy(&r.id))
        .filter(|(_, r)| {
            !inbox
                .iter()
                .any(|m| run_of_message(m, std::slice::from_ref(*r), |_| false).is_some())
        })
        .filter(|(_, r)| copy_exists(&r.worktree))
        .map(|(i, _)| i)
        .collect()
}

/// Under the end of a session: what became of its copy, or the links to deal with it
/// (merge into the project's folder, discard, open), each risky one asked twice.
pub(super) fn run_actions(
    ui: &mut egui::Ui,
    r: &crate::model::RunRecord,
    asking: Option<&'static str>,
    t: &Tone,
) -> Option<&'static str> {
    if auto::in_place(r) {
        ui.add_space(4.0);
        ui.label(
            RichText::new(crate::trf!(
                "Pas de copie : {}.",
                "No copy: {}.",
                if r.closed.is_empty() { auto::in_place_note() } else { r.closed.as_str() }
            ))
            .size(12.5)
            .color(t.tertiary),
        );
        return None;
    }
    if !r.closed.is_empty() {
        ui.add_space(4.0);
        ui.label(
            RichText::new(crate::trf!("Copie : {}", "Copy: {}", r.closed))
                .size(12.5)
                .color(t.tertiary),
        );
        return None;
    }
    // Being dealt with in the background (the copy may already be half gone).
    if let Some(a @ ("merging" | "discarding")) = asking {
        let what = if a == "merging" {
            crate::tr!("Fusion en cours…", "Merging…")
        } else {
            crate::tr!("Suppression de la copie en cours…", "Deleting the copy…")
        };
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0));
            ui.label(RichText::new(what).size(12.5).color(t.tertiary));
        });
        return None;
    }
    if !std::path::Path::new(&r.worktree).is_dir() {
        return None;
    }
    let mut act = None;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        match asking {
            Some(a) => {
                let what = if a == "merge" {
                    crate::tr!(
                        "Fusionner ce travail dans votre branche actuelle ?",
                        "Merge this work into your current branch?"
                    )
                } else {
                    crate::tr!(
                        "Supprimer la copie et sa branche, définitivement ?",
                        "Delete the copy and its branch, for good?"
                    )
                };
                ui.label(
                    RichText::new(what)
                        .size(12.5)
                        .color(super::projects::ORANGE),
                );
                if super::projects::action_link(ui, crate::tr!("Oui", "Yes")).clicked() {
                    act = Some(a);
                }
                if super::projects::action_link(ui, crate::tr!("Non", "No")).clicked() {
                    act = Some("cancel");
                }
            }
            None => {
                ui.label(
                    RichText::new(crate::tr!(
                        "Le travail est resté dans sa copie :",
                        "The work stayed in its copy:"
                    ))
                    .size(12.5)
                    .color(t.tertiary),
                );
                if super::projects::action_link(ui, crate::tr!("Fusionner", "Merge"))
                    .on_hover_text(crate::tr!(
                        "Intègre ce travail dans la branche ouverte dans le dossier du projet",
                        "Brings this work into the branch open in the project's folder"
                    ))
                    .clicked()
                {
                    act = Some("ask-merge");
                }
                if super::projects::action_link(ui, crate::tr!("Abandonner", "Discard"))
                    .on_hover_text(crate::tr!(
                        "Supprime la copie et sa branche, sans rien changer à votre dossier",
                        "Deletes the copy and its branch, nothing changes in your folder"
                    ))
                    .clicked()
                {
                    act = Some("ask-discard");
                }
                if super::projects::action_link(ui, crate::tr!("Ouvrir la copie", "Open the copy"))
                    .clicked()
                {
                    act = Some("open");
                }
            }
        }
    });
    act
}

/// What the person can do for a blocked AI: the button shown with the reason.
#[derive(Debug, PartialEq)]
pub(super) enum Unblock {
    /// It asked a question: answer it in the inbox.
    Answer,
    /// Claude Code asks something on its screen: answer in the session's terminal (its id).
    Terminal(String),
    /// It stopped, nothing asked: the project's steps (its id), to change them or start again.
    Steps(String),
}

impl Unblock {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Unblock::Answer => crate::tr!("Répondre à sa question", "Answer its question"),
            Unblock::Terminal(_) => crate::tr!("Ouvrir le terminal", "Open the terminal"),
            Unblock::Steps(_) => crate::tr!("Voir les étapes du projet", "See the project's steps"),
        }
    }

    /// For a paused session `id`.
    pub(super) fn of_wait(why: Wait, id: &str) -> Self {
        match why {
            Wait::Question => Unblock::Answer,
            Wait::Trust | Wait::Permission => Unblock::Terminal(id.to_string()),
        }
    }
}

/// The button of an inbox message about a blocked AI, only while it is of use: a pause still
/// going on (`paused`: id, title and reason of each paused session; the newest message of
/// each), or the end of a blocked session (`run`) with no session on its project since.
pub(super) fn inbox_unblock(
    m: &crate::model::InboxMsg,
    inbox: &[crate::model::InboxMsg],
    runs: &[crate::model::RunRecord],
    run: Option<&crate::model::RunRecord>,
    paused: &[(String, String, Wait)],
) -> Option<Unblock> {
    if m.kind == "info" {
        let (id, title, why) = paused.iter().find(|(id, title, _)| {
            runs.iter().any(|r| &r.id == id && r.project == m.project)
                && m.title == waiting_title(title)
        })?;
        let newer = inbox
            .iter()
            .any(|x| x.at > m.at && x.project == m.project && x.title == waiting_title(title));
        return (!newer).then(|| Unblock::of_wait(*why, id));
    }
    let run = run.filter(|r| r.outcome.starts_with(blocked_text()))?;
    if runs
        .iter()
        .any(|r| r.project == run.project && r.started > run.started)
    {
        return None;
    }
    Some(
        if inbox
            .iter()
            .any(|x| x.project == run.project && x.waits_for_person())
        {
            Unblock::Answer
        } else {
            Unblock::Steps(run.project.clone())
        },
    )
}

/// The inbox's chosen filter: 0 all, 1 to answer, 2 unread.
pub(super) fn inbox_filter_id() -> egui::Id {
    egui::Id::unique("inbox-filter")
}

/// Most questions shown on the Orchestration page.
pub(super) const ORCH_QUESTIONS: usize = 5;

/// How long an answered question stayed on the Orchestration page (its questions now live
/// in the inbox; kept for the tests of `orch_questions`).
#[cfg(test)]
pub(super) const ORCH_ANSWERED_MS: u64 = 3 * 86_400_000;

/// The inbox questions to show (indexes in `inbox`): first those waiting for an answer,
/// whatever their age, then those answered less than `answered_for` ms ago; newest first in
/// each group, `ORCH_QUESTIONS` at most.
pub(in crate::app) fn orch_questions(
    inbox: &[crate::model::InboxMsg],
    now: u64,
    answered_for: u64,
) -> Vec<usize> {
    let newest = |a: &usize, b: &usize| inbox[*b].at.cmp(&inbox[*a].at);
    let q = |i: &usize| inbox[*i].kind == "question";
    let mut open: Vec<usize> = (0..inbox.len())
        .filter(|&i| inbox[i].waits_for_person())
        .collect();
    let mut answered: Vec<usize> = (0..inbox.len())
        .filter(q)
        .filter(|&i| {
            !inbox[i].answer.trim().is_empty()
                && inbox[i]
                    .answered_at
                    .max(inbox[i].at)
                    .saturating_add(answered_for)
                    > now
        })
        .collect();
    open.sort_by(newest);
    answered.sort_by(|a, b| inbox[*b].answered_at.cmp(&inbox[*a].answered_at));
    open.into_iter()
        .chain(answered)
        .take(ORCH_QUESTIONS)
        .collect()
}

/// The count beside a questions card's title: how many wait, or how many are shown.
pub(in crate::app) fn questions_count(msgs: &[&crate::model::InboxMsg]) -> String {
    let waiting = msgs.iter().filter(|m| m.answer.trim().is_empty()).count();
    if waiting > 0 {
        crate::trf!("{waiting} en attente", "{waiting} waiting")
    } else {
        msgs.len().to_string()
    }
}

/// AI questions one under the other (Orchestration page, project page): answered or not,
/// project (`project_of`, None to leave it out) and date, the question on three lines at
/// most, then the answer or a box to type it. Returns (message id, answer) when one is sent.
pub(in crate::app) fn question_list(
    ui: &mut egui::Ui,
    msgs: &[&crate::model::InboxMsg],
    project_of: impl Fn(&crate::model::InboxMsg) -> Option<String>,
    t: &Tone,
) -> Option<(String, String)> {
    let accent = super::style::PURPLE;
    let mut out = None;
    for (k, m) in msgs.iter().enumerate() {
        if k > 0 {
            super::projects::card_rule(ui, t);
        }
        ui.horizontal(|ui| {
            if m.answer.trim().is_empty() {
                kind_badge(
                    ui,
                    crate::tr!("À répondre", "To answer"),
                    super::projects::ORANGE,
                );
            } else {
                kind_badge(
                    ui,
                    crate::tr!("Répondue", "Answered"),
                    super::projects::GREEN,
                );
            }
            if let Some(p) = project_of(m) {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 14.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(dot.center(), 3.5, project_color(&p));
                ui.label(RichText::new(p).size(12.0).color(t.secondary));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(super::chrome::relative_time(m.at))
                        .size(12.0)
                        .color(t.tertiary),
                );
            });
        });
        // The question itself, a few lines at most: the inbox has it whole.
        let q = plain_text(&m.text);
        let q = if q.is_empty() { m.title.clone() } else { q };
        let mut job = egui::text::LayoutJob::single_section(
            q,
            egui::TextFormat::simple(FontId::proportional(13.5), t.text),
        );
        job.wrap.max_width = ui.available_width();
        job.wrap.max_rows = 3;
        // Selectable, to copy it (the app's other labels are not: a click acts on them).
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        ui.add(egui::Label::new(galley).selectable(true));
        ui.add_space(4.0);
        if let Some(text) = answer_box(ui, m, accent, t) {
            out = Some((m.id.clone(), text));
        }
    }
    out
}

/// The group of an inbox message, by its local day: today, yesterday, this week, older.
/// `offset`: the local time's offset from UTC (ms).
pub(super) fn day_group(at: u64, now: u64, offset: i64) -> &'static str {
    let day = |ms: u64| (ms as i64 + offset).div_euclid(86_400_000);
    match day(now) - day(at) {
        i64::MIN..=0 => crate::tr!("Aujourd'hui", "Today"),
        1 => crate::tr!("Hier", "Yesterday"),
        2..=6 => crate::tr!("Cette semaine", "This week"),
        _ => crate::tr!("Plus ancien", "Older"),
    }
}

/// An inbox message's text for reading: Markdown headings and bold marks dropped (session
/// summaries are written in Markdown), blank lines squeezed.
pub(super) fn plain_text(text: &str) -> String {
    let mut out: Vec<String> = vec![];
    for line in text.lines() {
        let l = line
            .trim_end()
            .trim_start_matches('#')
            .trim_start()
            .replace("**", "");
        if l.is_empty() && out.last().is_none_or(|p| p.is_empty()) {
            continue;
        }
        out.push(l);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// A question's text split from the responder's analysis appended to it by `triage`
/// (« — Analyse du répondeur : »).
pub(super) fn split_analysis(text: &str) -> (&str, Option<&str>) {
    const MARK: &str = "— Analyse du répondeur :";
    match text.find(MARK) {
        Some(i) => (text[..i].trim_end(), Some(text[i + MARK.len()..].trim())),
        None => (text, None),
    }
}

/// Lines of a message's text shown before « Afficher tout ».
pub(super) const TEXT_LINES: usize = 4;

/// In a paused session's card: why the AI is stopped (and the question it asked), in full,
/// with the button to act; true when clicked.
pub(super) fn blocked_box(
    ui: &mut egui::Ui,
    why: &str,
    question: Option<&String>,
    button: Option<&str>,
    t: &Tone,
) -> bool {
    let orange = super::projects::ORANGE;
    let mut clicked = false;
    egui::Frame::new()
        .fill(orange.gamma_multiply(0.08))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(why).size(13.0).color(t.text)).wrap());
            if let Some(q) = question {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(q).font(semibold(13.0)).color(t.text)).wrap());
            }
            if let Some(b) = button {
                ui.add_space(6.0);
                clicked = super::style::primary(ui, b).clicked();
            }
        });
    clicked
}

/// The kind of an inbox message, as a small tinted pill.
pub(super) fn kind_badge(ui: &mut egui::Ui, text: &str, accent: Color32) {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), semibold(11.0), accent);
    let (rect, _) = ui.allocate_exact_size(g.size() + egui::vec2(14.0, 6.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 6.0, accent.gamma_multiply(0.14));
    ui.painter()
        .galley(rect.center() - g.size() / 2.0, g, accent);
}

/// A filter of the inbox: its name and how many messages it holds.
pub(super) fn filter_chip(ui: &mut egui::Ui, text: &str, count: usize, on: bool, t: &Tone) -> egui::Response {
    let label = if count > 0 {
        format!("{text}  {count}")
    } else {
        text.to_string()
    };
    let color = if on { t.text } else { t.secondary };
    let g = ui.painter().layout_no_wrap(
        label,
        if on {
            semibold(12.5)
        } else {
            FontId::proportional(12.5)
        },
        color,
    );
    let (rect, resp) =
        ui.allocate_exact_size(g.size() + egui::vec2(20.0, 10.0), egui::Sense::click());
    let a = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.14);
    let fill = if on {
        t.selected
    } else {
        t.hover.gamma_multiply(a)
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    if on {
        ui.painter().rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter()
        .galley(rect.center() - g.size() / 2.0, g, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Under a question of the inbox: the answer given, in its own tinted box (when, and where
/// it goes), or a box to type one.
/// Returns the answer when the person sends it (button, or Cmd/Ctrl+Enter).
pub(super) fn answer_box(
    ui: &mut egui::Ui,
    m: &crate::model::InboxMsg,
    accent: Color32,
    t: &Tone,
) -> Option<String> {
    let draft_id = egui::Id::unique(("inbox-draft", &m.id));
    let edit_id = egui::Id::unique(("inbox-edit", &m.id));
    let editing = ui.data(|d| d.get_temp::<bool>(edit_id)).unwrap_or(false);
    let mut out = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() - 30.0);
            if !m.answer.trim().is_empty() && !editing {
                let frame = egui::Frame::new()
                    .fill(accent.gamma_multiply(0.07))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 10,
                        top: 8,
                        bottom: 9,
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(crate::tr!("VOTRE RÉPONSE", "YOUR ANSWER"))
                                    .font(semibold(11.0))
                                    .color(accent),
                            );
                            if m.answered_at > 0 {
                                ui.label(
                                    RichText::new(super::chrome::relative_time(m.answered_at))
                                        .size(11.5)
                                        .color(t.tertiary),
                                );
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if super::projects::action_link(
                                        ui,
                                        crate::tr!("Modifier", "Edit"),
                                    )
                                    .clicked()
                                    {
                                        ui.data_mut(|d| {
                                            d.insert_temp(draft_id, m.answer.clone());
                                            d.insert_temp(edit_id, true);
                                        });
                                    }
                                },
                            );
                        });
                        ui.add_space(2.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(m.answer.trim()).size(13.5).color(t.text),
                            )
                            .wrap(),
                        );
                    });
                // A bar on the left, like a quote: the person's words, not the AI's.
                let r = frame.response.rect;
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(
                        r.left_top() + egui::vec2(0.0, 6.0),
                        egui::pos2(r.left() + 2.5, r.bottom() - 6.0),
                    ),
                    1.5,
                    accent.gamma_multiply(0.7),
                );
                ui.add_space(3.0);
                if m.answer_seen() {
                    // An AI received it: the person knows it was taken in.
                    let seen = crate::trf!(
                        "✓ Prise en compte par l'IA {}",
                        "✓ Taken in by the AI {}",
                        super::chrome::relative_time(m.seen_at)
                    );
                    ui.label(RichText::new(seen).size(11.5).color(super::projects::GREEN));
                } else {
                    ui.label(
                        RichText::new(answer_note(m.project.is_empty()))
                            .size(11.5)
                            .color(t.tertiary),
                    );
                }
                return;
            }
            let mut draft = ui
                .data(|d| d.get_temp::<String>(draft_id))
                .unwrap_or_default();
            let te = ui.add(
                egui::TextEdit::multiline(&mut draft)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text(crate::tr!("Répondre à l'IA…", "Answer the AI…")),
            );
            let send_key = te.has_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
            let mut send = false;
            ui.horizontal(|ui| {
                let ok = !draft.trim().is_empty();
                send = ui
                    .add_enabled(ok, egui::Button::new(crate::tr!("Répondre", "Answer")))
                    .clicked()
                    || (ok && send_key);
                if editing
                    && super::projects::action_link(ui, crate::tr!("Annuler", "Cancel")).clicked()
                {
                    ui.data_mut(|d| d.insert_temp(edit_id, false));
                }
                let hint = if cfg!(target_os = "macos") {
                    crate::tr!("⌘ Entrée pour envoyer", "⌘ Enter to send")
                } else {
                    crate::tr!("Ctrl+Entrée pour envoyer", "Ctrl+Enter to send")
                };
                ui.label(RichText::new(hint).size(11.5).color(t.tertiary));
            });
            if send {
                out = Some(crate::richtext::clean(draft.trim(), 4000));
                draft.clear();
                ui.data_mut(|d| d.insert_temp(edit_id, false));
            }
            ui.data_mut(|d| d.insert_temp(draft_id, draft));
        });
    });
    out
}

/// Under an answer: where it goes. An answer reaches an AI through its project's state of
/// play (get_project_state); a question asked outside any project has none to go through.
pub(super) fn answer_note(no_project: bool) -> &'static str {
    if no_project {
        crate::tr!(
            "Question posée hors d'un projet : la réponse est gardée ici, aucune IA ne la reçoit.",
            "Question asked outside a project: the answer is kept here, no AI receives it."
        )
    } else {
        crate::tr!(
            "Transmise à l'IA : elle la lit au prochain état des lieux du projet, au début de sa prochaine étape ou session.",
            "Passed on to the AI: it reads it in the project's next state of play, at the start of its next step or session."
        )
    }
}
