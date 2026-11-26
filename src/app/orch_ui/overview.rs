//! The Orchestration page, overview of the sessions: usage of the subscriptions, the running
//! and suspended sessions; the launch of a session from a project, and the AI settings shown
//! in Settings › AI.

use super::*;

impl NotesApp {
    /// At launch: read the « Screen Recording » permission (no dialog). Granted: remember
    /// this version as having it. Lost with an update: say so at once, with the way back.
    pub(in crate::app) fn screen_at_launch(&mut self) {
        if !cfg!(target_os = "macos") {
            return;
        }
        let ok = crate::screen::granted();
        self.orch.screen = Some((ok, Instant::now()));
        match crate::screen::at_launch(ok, &self.ws.settings.screen_granted_version, crate::update::VERSION) {
            crate::screen::AtLaunch::Fine => self.ws.settings.screen_granted_version = crate::update::VERSION.to_string(),
            crate::screen::AtLaunch::LostWithUpdate => self.info(crate::tr!(
                "La mise à jour a fait perdre à Orchestra l'autorisation « Enregistrement de l'écran » : les IA ne peuvent plus vérifier leur travail à l'écran. Page Orchestration › « Réaccorder » pour la retrouver.",
                "The update made Orchestra lose the « Screen Recording » permission: the AIs can no longer check their work on screen. Orchestration page › « Grant again » to get it back."
            )),
            crate::screen::AtLaunch::NeverGranted => {}
        }
    }

    /// The permission is missing: what it's for, and the two ways to give it back. Read
    /// again every few seconds while the page is open (a cheap call, no dialog).
    /// An alert while the subscription's use can't be read (see `measure_alert`).
    pub(super) fn measure_card(&self, ui: &mut egui::Ui, t: &Tone) {
        let running = self.orch.runs.iter().any(|r| r.agent == Agent::ClaudeCode);
        let now = crate::model::now_ms();
        let Some((title, text)) =
            measure_alert(self.orch.unmeasured_since, self.orch.five, running, now)
        else {
            if self.orch.unmeasured_since.is_some() {
                // To show it once the grace is over, even with nothing else moving.
                ui.ctx().request_repaint_after(Duration::from_secs(5));
            }
            return;
        };
        ui.ctx().request_repaint_after(Duration::from_secs(30));
        egui::Frame::new()
            .fill(super::projects::ORANGE.gamma_multiply(0.07))
            .stroke(egui::Stroke::new(1.0, super::projects::ORANGE.gamma_multiply(0.45)))
            .corner_radius(12.0)
            .inner_margin(egui::Margin::symmetric(18, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(title).font(semibold(14.0)).color(super::projects::ORANGE));
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(t.secondary)).wrap());
            });
        ui.add_space(14.0);
    }

    pub(super) fn screen_card(&mut self, ui: &mut egui::Ui, t: &Tone) {
        if !cfg!(target_os = "macos") || self.orch.screen_dismissed {
            return;
        }
        let stale = self
            .orch
            .screen
            .is_none_or(|(_, at)| at.elapsed() > Duration::from_secs(3));
        if stale {
            let ok = crate::screen::granted();
            if ok {
                self.ws.settings.screen_granted_version = crate::update::VERSION.to_string();
            }
            self.orch.screen = Some((ok, Instant::now()));
        }
        if self.orch.screen.is_some_and(|(ok, _)| ok) {
            return;
        }
        ui.ctx().request_repaint_after(Duration::from_secs(3));
        let lost = !self.ws.settings.screen_granted_version.is_empty();
        let mut ask = false;
        let mut open = false;
        egui::Frame::new()
            .fill(super::projects::ORANGE.gamma_multiply(0.07))
            .stroke(egui::Stroke::new(1.0, super::projects::ORANGE.gamma_multiply(0.45)))
            .corner_radius(12.0)
            .inner_margin(egui::Margin::symmetric(18, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let title = if lost { crate::tr!("Autorisation « Enregistrement de l'écran » perdue", "« Screen Recording » permission lost") } else { crate::tr!("Autorisation « Enregistrement de l'écran » manquante", "« Screen Recording » permission missing") };
                ui.label(RichText::new(title).font(semibold(14.0)).color(super::projects::ORANGE));
                ui.add(
                    egui::Label::new(
                        RichText::new(crate::tr!(
                            "Sans elle, les IA lancées ici ne peuvent pas prendre de capture (take_screenshot) pour vérifier une interface. Cochez Orchestra dans Réglages Système › Confidentialité et sécurité › Enregistrement de l'écran, puis quittez et relancez Orchestra : macOS ne l'applique qu'au lancement suivant.",
                            "Without it, the AIs started here can't take a screenshot (take_screenshot) to check an interface. Tick Orchestra in System Settings › Privacy & Security › Screen Recording, then quit and reopen Orchestra: macOS only applies it at the next launch."
                        ))
                        .size(12.5)
                        .color(t.secondary),
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.button(RichText::new(crate::tr!("Ouvrir les réglages", "Open the settings")).size(13.0)).clicked() {
                        open = true;
                    }
                    let again = ui
                        .button(RichText::new(crate::tr!("Réaccorder", "Grant again")).size(13.0))
                        .on_hover_text(crate::tr!(
                            "Efface l'ancienne entrée d'Orchestra dans les réglages si elle est restée d'une version précédente, puis macOS redemande l'autorisation",
                            "Clears Orchestra's old entry in the settings if it was left by an earlier version, then macOS asks for the permission again"
                        ));
                    if again.clicked() {
                        ask = true;
                    }
                    if super::projects::quiet_link(ui, crate::tr!("Plus tard", "Later"), t).clicked() {
                        self.orch.screen_dismissed = true;
                    }
                });
            });
        ui.add_space(14.0);
        if open && let Err(e) = crate::screen::open_settings() {
            self.info(e);
        }
        if ask {
            let mut v = self.ws.settings.screen_granted_version.clone();
            let ok = crate::screen::ensure_permission(&mut v);
            self.ws.settings.screen_granted_version = v;
            self.orch.screen = Some((ok, Instant::now()));
            if !ok {
                let _ = crate::screen::open_settings();
            }
        }
    }

    pub(in crate::app) fn orchestra_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let t = tone(ui.visuals().dark_mode);
        let avail = ui.available_width();
        let width = 1000.0f32.min(avail - 40.0).max(320.0);
        ui.add_space(50.0);
        ui.horizontal(|ui| {
            ui.add_space(((avail - width) / 2.0).max(20.0));
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.label(RichText::new(crate::tr!("Orchestration", "Orchestration")).font(bold(super::style::TITLE)).color(t.text));
                ui.label(RichText::new(crate::tr!("Des IA qui avancent seules sur vos projets, dans une copie à part.", "AIs moving your projects forward on their own, in a separate copy.")).color(t.secondary));
                ui.add_space(14.0);
                // The summary first: AIs at work and today's work, at a glance.
                let top = ui.cursor().min;
                self.stats_row(ui, &t);
                mark_from(ui, "orch:stats", top);
                ui.add_space(14.0);
                self.screen_card(ui, &t);
                self.measure_card(ui, &t);
                // Then what is happening: sessions at work or suspended, questions waiting,
                // work left to deal with. Sessions start from a project or one of its steps,
                // and their settings live in Settings › AI.
                let top = ui.cursor().min;
                self.running_card(ui, &t, actions);
                if !self.orch.runs.is_empty() {
                    ui.add_space(14.0);
                }
                // Sessions waiting for the subscription's quota: still there, suspended.
                let suspended = self.suspended_card(ui, &t, actions);
                if suspended {
                    ui.add_space(14.0);
                }
                if self.orch.runs.is_empty() && !suspended {
                    self.quiet_card(ui, &t, actions);
                    ui.add_space(14.0);
                }
                mark_from(ui, "orch:running", top);
                // The AIs' questions: those waiting (a paused session goes on once answered),
                // then the answers of the last days.
                if self.questions_card(ui, &t) {
                    ui.add_space(14.0);
                }
                // Work left in copies whose end message was deleted: nowhere else to deal with it.
                if self.orphans_card(ui, &t, actions) {
                    ui.add_space(14.0);
                }
                // Too many days of reports: said here, where they are made.
                let old = self.ws.old_report_days().len();
                if old > 0 {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("📒").size(13.0));
                        ui.label(
                            RichText::new(crate::trf!(
                                "{old} jour(s) de rapports au-delà des {} plus récents : la page « Rapports des IA » devient longue.",
                                "{old} day(s) of reports beyond the {} most recent: the « AI reports » page is getting long.",
                                crate::model::REPORT_DAYS_KEPT
                            ))
                            .size(12.5)
                            .color(t.secondary),
                        );
                        if super::projects::action_link(ui, crate::tr!("Condenser en une synthèse", "Condense into a summary"))
                            .on_hover_text(crate::tr!(
                                "Les jours les plus anciens deviennent une seule page « Synthèse des anciens rapports » (titres et étapes terminées) ; les pages de ces jours vont à la corbeille, d'où elles peuvent revenir.",
                                "The oldest days become one « Summary of older reports » page (titles and finished steps); those days' pages go to the trash, where they can be brought back from."
                            ))
                            .clicked()
                            && let Some((page, n)) = self.ws.condense_old_reports()
                        {
                            self.dirty_since.get_or_insert_with(Instant::now);
                            self.ack(crate::trf!("{n} jour(s) de rapports condensés en une synthèse.", "{n} day(s) of reports condensed into a summary."));
                            actions.push(Action::Open(page));
                        }
                        if super::projects::action_link(ui, crate::tr!("Voir les rapports", "See the reports")).clicked() {
                            actions.push(Action::Open(self.ws.settings.orch.reports_page.clone()));
                        }
                    });
                    ui.add_space(14.0);
                }
                if !self.ws.settings.orch.reserve_asked {
                    self.reserve_card(ui, &t);
                    ui.add_space(14.0);
                }
                ui.add_space(40.0);
            });
        });
    }

    /// With no session running or suspended: said so, with where one starts from, and the
    /// way to the projects that have open steps (the latest active first).
    pub(super) fn quiet_card(&mut self, ui: &mut egui::Ui, t: &Tone, actions: &mut Vec<Action>) {
        let ready = projects_with_open_steps(&self.ws);
        super::projects::card(ui, t, 0.0, |ui| {
            ui.label(
                RichText::new(crate::tr!("Aucune session en cours", "No session running"))
                    .font(semibold(14.5))
                    .color(t.text),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(crate::tr!(
                        "Une IA se lance depuis la page d'un projet (« Lancer une IA ») ou depuis une étape qui lui est confiée (« Lancer »). Ses réglages et ses limites sont dans Paramètres › IA.",
                        "An AI starts from a project's page (« Start an AI ») or from a step entrusted to it (« Start »). Its settings and limits are in Settings › AI."
                    ))
                    .size(12.5)
                    .color(t.secondary),
                )
                .wrap(),
            );
            if ready.is_empty() {
                return;
            }
            ui.add_space(6.0);
            for (id, title, open) in ready.iter().take(QUIET_PROJECTS) {
                ui.horizontal_wrapped(|ui| {
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 14.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.5, project_color(title));
                    if super::projects::action_link(ui, title).clicked() {
                        actions.push(Action::Open(id.clone()));
                    }
                    let s = crate::i18n::plural(*open);
                    ui.label(
                        RichText::new(crate::trf!("{open} étape{s} ouverte{s}", "{open} open step{s}"))
                            .size(12.0)
                            .color(t.tertiary),
                    );
                });
            }
        });
    }

    /// The settings common to every autonomous session, in Settings › AI.
    pub(in crate::app) fn session_settings(&mut self, ui: &mut egui::Ui, t: &Tone, l: &super::settings::Look) {
        use super::settings::{row, section, switch};
        let before = self.ws.settings.orch.clone();
        let o = &mut self.ws.settings.orch;
        section(ui, crate::tr!("Sessions autonomes", "Autonomous sessions"), t, l);
        row(
            ui,
            crate::tr!("Limite par défaut", "Default limit"),
            crate::tr!(
                "De la fenêtre de 5 h, votre propre usage compris, pour les IA sans limite à elles. Chaque session a en plus son propre plafond (« arrêt à », dans les réglages de lancement du projet).",
                "Of the 5-hour window, your own use included, for the AIs without a limit of their own. Each session also has its own ceiling (« stops at », in the project's launch settings)."
            ),
            t,
            |ui| {
                ui.add(
                    egui::DragValue::new(&mut o.global_limit)
                        .range(5.0..=100.0)
                        .speed(0.5)
                        .suffix(" %"),
                );
            },
        );
        row(
            ui,
            crate::tr!("Commits autorisés", "Commits allowed"),
            crate::tr!(
                "Sur leur branche, envoyée sur GitHub par Orchestra.",
                "On their branch, pushed to GitHub by Orchestra."
            ),
            t,
            |ui| {
                switch(ui, &mut o.allow_commits, l);
            },
        );
        row(
            ui,
            crate::tr!("Pause quand l'IA attend", "Pause when the AI waits"),
            crate::tr!(
                "Autorisation, dossier à approuver ou question posée : la session attend votre réponse au lieu de refuser ou de s'arrêter.",
                "A permission, a folder to approve or a question asked: the session waits for your answer instead of refusing or stopping."
            ),
            t,
            |ui| {
                switch(ui, &mut o.pause_for_answer, l);
            },
        );
        row(
            ui,
            crate::tr!("Reprise automatique", "Automatic restart"),
            crate::tr!(
                "Une session arrêtée par la limite globale ou la réserve de la semaine repart seule après la remise à zéro du quota.",
                "A session stopped by the global limit or the week's reserve starts again by itself once the quota resets."
            ),
            t,
            |ui| {
                switch(ui, &mut o.auto_resume, l);
            },
        );
        row(
            ui,
            crate::tr!("Bilan de la journée", "Daily summary"),
            crate::tr!("Dès qu'une session a travaillé.", "Once a session has worked."),
            t,
            |ui| {
                let mut h = o.report_hour as u32;
                if ui
                    .add(egui::DragValue::new(&mut h).range(0..=23).suffix(" h"))
                    .changed()
                {
                    o.report_hour = h as u8;
                }
            },
        );
        row(
            ui,
            crate::tr!("Tours au plus", "Turns at most"),
            crate::tr!("Par session.", "Per session."),
            t,
            |ui| {
                ui.add(egui::DragValue::new(&mut o.max_turns).range(1..=200));
            },
        );
        ui.collapsing(RichText::new(crate::tr!("Commandes autorisées", "Allowed commands")).size(13.0).color(t.secondary), |ui| {
            ui.label(
                RichText::new(crate::tr!("Déjà permis : lire et modifier les fichiers de la copie, git status / diff / log, compiler et tester (cargo build / test, npm test, pytest, make test, go test…) et git add / commit si les commits sont autorisés. Pas python, node, npm, cargo ou make en entier : ils exécutent n'importe quel code. Toujours refusés : rm, sudo, curl, wget, shells, git push / reset / checkout… Une autre commande est refusée, l'IA continue sans.", "Already allowed: reading and editing the copy's files, git status / diff / log, building and testing (cargo build / test, npm test, pytest, make test, go test…) and git add / commit when commits are allowed. Not python, node, npm, cargo or make as a whole: they run any code. Always refused: rm, sudo, curl, wget, shells, git push / reset / checkout… Any other command is refused and the AI goes on without it."))
                    .size(12.5)
                    .color(t.tertiary),
            );
            ui.add(egui::TextEdit::singleline(&mut o.extra_tools).hint_text(crate::tr!("En plus, ex. Bash(just:*), Bash(docker compose:*)", "More, e.g. Bash(just:*), Bash(docker compose:*)")).desired_width(f32::INFINITY).font(FontId::monospace(12.5)));
            // The rules dropped, said rather than silently ignored.
            let rejected = auto::rejected_extra(&o.extra_tools);
            if !rejected.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(crate::trf!(
                            "Ignorées, trop larges ou contraires aux commandes toujours refusées : {}. Donnez une commande précise, ex. Bash(git log:*) plutôt que Bash(git:*).",
                            "Ignored, too wide or against the commands always refused: {}. Give a precise command, e.g. Bash(git log:*) rather than Bash(git:*).",
                            rejected.join(", ")
                        ))
                        .size(12.0)
                        .color(super::projects::ORANGE),
                    )
                    .wrap(),
                );
            }
        });
        if self.ws.settings.orch != before {
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    /// The question asked once, on the Orchestration page: how much of the week to keep for
    /// the person. Its answer is the common reserve; each AI may then differ, in its settings.
    pub(super) fn reserve_card(&mut self, ui: &mut egui::Ui, t: &Tone) {
        let before = self.ws.settings.orch.clone();
        let o = &mut self.ws.settings.orch;
        super::projects::card(ui, t, 0.0, |ui| {
            ui.label(
                RichText::new(crate::tr!(
                    "Quelle part de vos abonnements garder pour vous ?",
                    "How much of your subscriptions should be kept for you?"
                ))
                .font(semibold(14.0))
                .color(t.text),
            );
            ui.label(
                RichText::new(crate::tr!(
                    "Les IA s'arrêtent avant d'entamer cette réserve de la semaine, et reprennent quand elle repart à zéro. Réglable ensuite pour chaque IA, dans Paramètres › IA.",
                    "The AIs stop before touching this reserve of the week, and start again once it resets. Then adjustable for each AI, in Settings › AI."
                ))
                .size(12.5)
                .color(t.secondary),
            );
            ui.horizontal(|ui| {
                for r in [5.0, 10.0, 20.0, 30.0] {
                    if super::style::secondary(ui, &format!("{r:.0} %")).clicked() {
                        o.set_reserve(r);
                        o.reserve_asked = true;
                    }
                }
                let keep = o.reserve();
                if super::style::quiet(ui, &crate::trf!("Garder {keep:.0} %", "Keep {keep:.0} %"))
                    .clicked()
                {
                    o.reserve_asked = true;
                }
            });
        });
        if self.ws.settings.orch != before {
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    /// The limits of one AI, under its name in Settings › AI: its windows (5 hours and week,
    /// or Mistral's month) as bars, with where its sessions stop as a handle to drag along
    /// the bar (or a number to type).
    pub(in crate::app) fn ai_limits(&mut self, ui: &mut egui::Ui, a: Agent, t: &Tone) {
        let before = self.ws.settings.orch.clone();
        // The figures its running sessions are measured on; else the last ones the app read.
        let fig = self
            .figures_by_agent()
            .get(&a)
            .copied()
            .or_else(|| {
                self.ais
                    .others
                    .iter()
                    .find(|(x, _)| *x == a)
                    .and_then(|(_, u)| u.as_ref().map(Figures::of))
            })
            .unwrap_or_default();
        agent_limits(ui, a, fig, &mut self.ws.settings.orch, t);
        // Claude Code's pace over the last day, read from its history.
        if a == Agent::ClaudeCode
            && let Some((used, span)) = self.orch.day_week
        {
            let (line, fast) = crate::usage::day_pace_line(used, span);
            let color = if fast { super::projects::ORANGE } else { t.tertiary };
            ui.horizontal(|ui| {
                ui.add_space(LIMIT_LABEL_W);
                ui.add(egui::Label::new(RichText::new(line).size(12.0).color(color)).wrap())
                    .on_hover_text(crate::trf!(
                        "Ce qui est parti de la semaine sur les dernières 24 h, par vous et par les IA. La part d'un jour est un septième de la semaine ; plus vite (en orange), elle s'épuiserait avant sa remise à zéro. C'est {:.0} % de la part d'un jour.",
                        "What went from the week over the last 24 hours, by you and by the AIs. A day's share is a seventh of the week; faster (in orange), it would run out before it resets. That is {:.0} % of a day's share.",
                        crate::usage::daily_pct(used)
                    ));
            });
        }
        if self.ws.settings.orch != before {
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    /// Two tiles at the top of the page: the AIs at work, today's work.
    pub(super) fn stats_row(&mut self, ui: &mut egui::Ui, t: &Tone) {
        let running = self.orch.runs.len();
        let ready = self
            .ws
            .projects()
            .iter()
            .filter(|p| {
                p.project
                    .as_ref()
                    .is_some_and(|x| !x.folder.trim().is_empty())
            })
            .count();
        let today = auto::local_day(crate::model::now_ms());
        let todays: Vec<&crate::model::RunRecord> = self
            .ws
            .runs
            .iter()
            .filter(|r| !r.outcome.is_empty() && auto::local_day(r.started) == today)
            .collect();
        let steps: usize = todays.iter().map(|r| r.steps_done.len()).sum();
        let hour = self.ws.settings.orch.report_hour;
        let w = ui.available_width();
        let gap = 14.0;
        // Each AI's windows are in its settings: here, what is common to all of them.
        let cols = if w >= 520.0 { 2 } else { 1 };
        let tw = if cols == 2 { (w - gap) / 2.0 } else { w };
        let h = if cols == 2 { 112.0 } else { 92.0 };
        let draw = |ui: &mut egui::Ui, i: usize| {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(tw, h), egui::Sense::hover());
            let a = ui
                .ctx()
                .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.18);
            let accent = [super::projects::GREEN, super::projects::BLUE][i];
            let p = ui.painter();
            p.add(egui::epaint::RectShape::new(
                rect,
                14.0,
                t.card,
                egui::Stroke::new(1.0, t.border.lerp_to_gamma(accent, 0.35 * a)),
                egui::StrokeKind::Inside,
            ));
            // A soft wash of the accent in the corner.
            p.with_clip_rect(rect.shrink(1.0)).circle_filled(
                rect.right_top() + egui::vec2(-10.0, 10.0),
                52.0 + 6.0 * a,
                accent.gamma_multiply(0.07),
            );
            let x = rect.left() + 18.0;
            let label = [
                crate::tr!("IA AU TRAVAIL", "AIS AT WORK"),
                crate::tr!("AUJOURD'HUI", "TODAY"),
            ][i];
            p.text(
                egui::pos2(x, rect.top() + 16.0),
                egui::Align2::LEFT_TOP,
                label,
                semibold(11.5),
                t.tertiary,
            );
            let (big, sub): (String, String) = match i {
                0 => {
                    let s = crate::i18n::plural(ready);
                    (
                        running.to_string(),
                        if running == 0 {
                            crate::trf!(
                                "{ready} projet{s} prêt{s} à lancer",
                                "{ready} project{s} ready to start"
                            )
                        } else {
                            crate::trf!(
                                "sur {ready} projet{s} prêt{s}",
                                "of {ready} project{s} ready"
                            )
                        },
                    )
                }
                _ => (
                    todays.len().to_string(),
                    if todays.is_empty() {
                        crate::trf!(
                            "session terminée · bilan à {hour} h",
                            "finished session · summary at {hour} h"
                        )
                    } else {
                        let (s, e) = (
                            crate::i18n::plural(todays.len()),
                            crate::i18n::plural(steps),
                        );
                        crate::trf!(
                            "session{s} · {steps} étape{e} faite{e}",
                            "session{s} · {steps} step{e} done"
                        )
                    },
                ),
            };
            p.text(
                egui::pos2(x, rect.top() + 36.0),
                egui::Align2::LEFT_TOP,
                &big,
                bold(30.0),
                t.text,
            );
            let mut job = egui::text::LayoutJob::single_section(
                sub,
                egui::TextFormat::simple(FontId::proportional(12.5), t.secondary),
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(
                tw - 36.0,
            );
            let g = ui.fonts_mut(|f| f.layout_job(job));
            ui.painter()
                .galley(egui::pos2(x, rect.bottom() - 28.0), g, t.secondary);
            match i {
                0 if running > 0 => {
                    super::projects::pulse_dot(
                        ui,
                        egui::pos2(rect.right() - 30.0, rect.top() + 24.0),
                        super::projects::GREEN,
                    );
                }
                _ => {}
            }
        };
        if cols == 2 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for i in 0..2 {
                    draw(ui, i);
                }
            });
        } else {
            for i in 0..2 {
                draw(ui, i);
                ui.add_space(8.0);
            }
        }
    }

    pub(super) fn running_card(&mut self, ui: &mut egui::Ui, t: &Tone, actions: &mut Vec<Action>) {
        if self.orch.runs.is_empty() {
            return;
        }
        // « Ajouter des tâches »: the project whose page to open.
        let mut add_tasks: Option<String> = None;
        let dark = ui.visuals().dark_mode;
        let watching = self.orch.watching.clone();
        let mut stop: Option<(usize, bool)> = None;
        let mut watch: Option<Option<String>> = None;
        // « Ouvrir le terminal » of a paused session (`watch` belongs to the links).
        let mut open_terminal: Option<String> = None;
        let n = self.orch.runs.len();
        let awake = self.orch.awake.held();
        // Each session's bar on the 5-hour window of its own AI's subscription.
        let figs = self.figures_by_agent();
        let limits = self.ws.settings.orch.clone();
        // The step each session is on (« doing », reserved by this computer).
        let me = crate::store::computer_id();
        let working_on: HashMap<String, String> = self
            .orch
            .runs
            .iter()
            .filter_map(|r| {
                let steps = &self.ws.page(&r.project)?.project.as_ref()?.steps;
                // Under the name of this session's AI, so two sessions each show their own step.
                let who = holder_of(r.agent, &me);
                let step = session_step(sent_on(&self.ws.runs, &r.id), steps, &who)?;
                Some((r.id.clone(), step.title.clone()))
            })
            .collect();
        // Each session's model, and why it was chosen (its record, written at launch).
        let models: HashMap<String, (String, String)> = self
            .ws
            .runs
            .iter()
            .filter(|x| self.orch.runs.iter().any(|r| r.id == x.id))
            .filter_map(|x| {
                let why = auto::model_note(x)?.replace('`', "");
                let name = if x.model.is_empty() {
                    crate::tr!("modèle par défaut", "default model").to_string()
                } else {
                    x.model.clone()
                };
                Some((x.id.clone(), (name, why)))
            })
            .collect();
        // The question each paused session waits on, the latest it asked.
        let questions: HashMap<String, String> = self
            .orch
            .runs
            .iter()
            .filter(|r| r.waiting == Some(Wait::Question))
            .filter_map(|r| {
                let m = self.ws.inbox.iter().rev().find(|m| {
                    m.kind == "question"
                        && m.project == r.project
                        && m.at >= r.started_wall
                        && m.answer.trim().is_empty()
                })?;
                let text = if m.text.trim().is_empty() { &m.title } else { &m.text };
                Some((r.id.clone(), plain_text(split_analysis(text).0)))
            })
            .collect();
        // « Répondre à sa question »: to the inbox.
        let mut to_inbox = false;
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(ui, crate::tr!("EN COURS", "RUNNING"), &n.to_string(), t);
            if awake {
                ui.label(RichText::new(crate::tr!(
                    "L'ordinateur reste éveillé (sur secteur, même capot rabattu). Fermer la fenêtre ne coupe pas les sessions.",
                    "The computer stays awake (on mains power, lid closed too). Closing the window does not stop the sessions."
                )).size(12.0).color(t.tertiary));
                ui.add_space(6.0);
            }
            for (i, r) in self.orch.runs.iter_mut().enumerate() {
                // The session's links; in a right-to-left layout « Forcer l'arrêt » ends up last.
                let mut links = |ui: &mut egui::Ui, r: &mut LiveRun| {
                    if super::projects::action_link(ui, crate::tr!("Forcer l'arrêt", "Force stop"))
                        .on_hover_text(crate::tr!(
                            "Ferme Claude Code tout de suite, sans compte rendu",
                            "Closes Claude Code right away, without a summary"
                        ))
                        .clicked()
                    {
                        stop = Some((i, true));
                    }
                    if r.stop.is_none()
                        && super::projects::action_link(ui, crate::tr!("Arrêter", "Stop"))
                            .on_hover_text(crate::tr!(
                                "Finit le tour en cours, écrit le compte rendu, puis s'arrête",
                                "Finishes the current turn, writes the summary, then stops"
                            ))
                            .clicked()
                    {
                        stop = Some((i, false));
                    }
                    if r.waiting.is_some()
                        && super::projects::action_link(ui, crate::tr!("Reprendre", "Resume"))
                            .on_hover_text(crate::tr!(
                                "L'IA repart ; une question restée à l'écran est refusée",
                                "The AI goes on; a question still on the screen is refused"
                            ))
                            .clicked()
                    {
                        r.resume();
                    }
                    let open = watching.as_deref() == Some(r.id.as_str());
                    if super::projects::action_link(
                        ui,
                        if open {
                            crate::tr!("Masquer", "Hide")
                        } else {
                            crate::tr!("Regarder", "Watch")
                        },
                    )
                    .clicked()
                    {
                        watch = Some(if open { None } else { Some(r.id.clone()) });
                    }
                    if super::projects::action_link(ui, crate::tr!("+ Ajouter des tâches", "+ Add tasks"))
                        .on_hover_text(crate::tr!("Ouvre les étapes du projet : l'IA prendra les nouvelles sans que la session s'arrête", "Opens the project's steps: the AI takes the new ones without the session stopping"))
                        .clicked()
                    {
                        add_tasks = Some(r.project.clone());
                    }
                };
                egui::Frame::new()
                    .fill(super::projects::GREEN.gamma_multiply(0.06))
                    .stroke(egui::Stroke::new(
                        1.0,
                        super::projects::GREEN.gamma_multiply(0.3),
                    ))
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        // Room for the title, its state and the links on one line.
                        let wide = ui.available_width() >= RUN_ONE_LINE;
                        ui.horizontal(|ui| {
                            let (dot, _) = ui
                                .allocate_exact_size(egui::vec2(12.0, 18.0), egui::Sense::hover());
                            super::projects::pulse_dot(ui, dot.center(), super::projects::GREEN);
                            ui.label(RichText::new(&r.title).font(semibold(14.5)).color(t.text));
                            let state = r.state_text();
                            let state_color = if r.waiting.is_some() {
                                super::projects::ORANGE
                            } else {
                                t.tertiary
                            };
                            ui.label(RichText::new(state).size(12.5).color(state_color));
                            if let Some((name, why)) = models.get(&r.id) {
                                ui.label(RichText::new(name).size(12.5).color(t.secondary))
                                    .on_hover_text(crate::trf!(
                                        "Modèle : {why}",
                                        "Model: {why}"
                                    ));
                            }
                            if let Some(c) = &r.endless {
                                ui.label(
                                    RichText::new(crate::tr!("∞ sans fin", "∞ endless"))
                                        .size(12.5)
                                        .color(super::projects::BLUE),
                                )
                                .on_hover_text(crate::trf!(
                                    "S'arrête après : {}, ou à son plafond",
                                    "Stops after: {}, or at its ceiling",
                                    c.describe()
                                ));
                            }
                            if wide {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| links(ui, r),
                                );
                            }
                        });
                        // Narrow: the links on a line of their own rather than over the title.
                        if !wide {
                            ui.horizontal_wrapped(|ui| {
                                ui.add_space(20.0);
                                links(ui, r);
                            });
                        }
                        let five = figs.get(&r.agent).and_then(|f| f.five);
                        share_row(ui, &r.id, r.pct, r.limit, r.ceiling, five, limits.five_limit(r.agent), t);
                        // The task in hand, as a short sentence.
                        if let Some(step) = working_on.get(&r.id) {
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                ui.add_space(20.0);
                                working_mark(ui, t);
                                let mut job = egui::text::LayoutJob::default();
                                job.append(
                                    crate::tr!("En train de s'occuper de ", "Busy with "),
                                    0.0,
                                    egui::TextFormat {
                                        font_id: FontId::proportional(13.5),
                                        color: t.secondary,
                                        italics: true,
                                        ..Default::default()
                                    },
                                );
                                job.append(
                                    &format!("« {step} »"),
                                    0.0,
                                    egui::TextFormat::simple(semibold(13.5), t.text),
                                );
                                job.wrap = egui::text::TextWrapping::truncate_at_width(
                                    ui.available_width(),
                                );
                                super::projects::cut_label(ui, job).on_hover_text(step);
                            });
                        }
                        if let Some(why) = r.waiting {
                            // Blocked on the person: the whole reason, and the way to act.
                            let action = Unblock::of_wait(why, &r.id);
                            ui.add_space(4.0);
                            let open = watching.as_deref() == Some(r.id.as_str());
                            if blocked_box(ui, why.label(), questions.get(&r.id), (!(open && action != Unblock::Answer)).then(|| action.label()), t) {
                                match action {
                                    Unblock::Terminal(id) => open_terminal = Some(id),
                                    _ => to_inbox = true,
                                }
                            }
                        } else {
                            ui.horizontal(|ui| {
                                ui.add_space(20.0);
                                let mut job = egui::text::LayoutJob::single_section(
                                    r.last_line.clone(),
                                    egui::TextFormat::simple(FontId::proportional(12.5), t.tertiary),
                                );
                                job.wrap =
                                    egui::text::TextWrapping::truncate_at_width(ui.available_width());
                                super::projects::cut_label(ui, job);
                            });
                        }
                        if let Some(why) = &r.stop {
                            ui.horizontal(|ui| {
                                ui.add_space(20.0);
                                // Wrapped: a long reason must not widen the cards below.
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(crate::trf!(
                                            "S'arrête : {why}",
                                            "Stopping: {why}"
                                        ))
                                        .size(12.5)
                                        .color(super::projects::ORANGE),
                                    )
                                    .wrap(),
                                );
                            });
                        }
                        if watching.as_deref() == Some(r.id.as_str()) {
                            egui::Frame::new()
                                .fill(Color32::from_rgb(20, 20, 20))
                                .corner_radius(8.0)
                                .inner_margin(8)
                                .show(ui, |ui| {
                                    ui.set_height(420.0);
                                    ui.set_width(ui.available_width());
                                    super::term_ui::terminal_view(
                                        ui,
                                        &mut r.term,
                                        Color32::from_rgb(20, 20, 20),
                                        dark || true,
                                        false,
                                    );
                                });
                        }
                    });
                ui.add_space(8.0);
            }
        });
        if let Some(w) = open_terminal.map(Some).or(watch) {
            self.orch.watching = w;
        }
        if to_inbox {
            ui.data_mut(|d| d.insert_temp(inbox_filter_id(), 1u8));
            self.view = View::Inbox;
        }
        if let Some(p) = add_tasks {
            super::projects::focus_steps(ui.ctx(), &p);
            actions.push(Action::Open(p));
        }
        if let Some((i, force)) = stop {
            if force {
                let r = &mut self.orch.runs[i];
                r.stop = Some(crate::tr!("arrêtée à la main", "stopped by hand").into());
                self.finish_run(i, crate::tr!("arrêtée à la main", "stopped by hand"), "");
                self.orch.runs.remove(i);
            } else {
                self.orch.runs[i].stop =
                    Some(crate::tr!("arrêtée à la main", "stopped by hand").into());
            }
        }
    }

    /// The sessions stopped by the 5-hour or weekly limit that start again by themselves
    /// once it resets: shown like the running ones, in orange, suspended. False when none.
    pub(super) fn suspended_card(&mut self, ui: &mut egui::Ui, t: &Tone, actions: &mut Vec<Action>) -> bool {
        let now = crate::model::wall_ms();
        let running: Vec<&str> = self.orch.runs.iter().map(|r| r.project.as_str()).collect();
        let list = crate::model::suspended_runs(&self.ws.runs, now, &running);
        if list.is_empty() {
            return false;
        }
        let orange = super::projects::ORANGE;
        let mut cancel: Option<String> = None;
        let mut add_tasks: Option<String> = None;
        super::projects::card(ui, t, 0.0, |ui| {
            super::projects::card_title(
                ui,
                crate::tr!("SUSPENDUES", "SUSPENDED"),
                &list.len().to_string(),
                t,
            );
            for &i in &list {
                let r = &self.ws.runs[i];
                egui::Frame::new().fill(orange.gamma_multiply(0.07)).stroke(egui::Stroke::new(1.0, orange.gamma_multiply(0.35))).corner_radius(10.0).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        // A still dot (pause sign) where a running session pulses.
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 18.0), egui::Sense::hover());
                        for dx in [-2.5, 2.5] {
                            ui.painter().line_segment([dot.center() + egui::vec2(dx, -4.0), dot.center() + egui::vec2(dx, 4.0)], egui::Stroke::new(2.0, orange));
                        }
                        ui.label(RichText::new(&r.title).font(semibold(14.5)).color(t.text));
                        ui.label(RichText::new(crate::tr!("suspendue", "suspended")).size(12.5).color(orange));
                        if r.endless.is_some() {
                            ui.label(RichText::new(crate::tr!("∞ sans fin", "∞ endless")).size(12.5).color(super::projects::BLUE));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if super::projects::action_link(ui, crate::tr!("Annuler la reprise", "Cancel the restart"))
                                .on_hover_text(crate::tr!("La session ne repartira pas d'elle-même", "The session won't start again by itself"))
                                .clicked()
                            {
                                cancel = Some(r.project.clone());
                            }
                            if super::projects::action_link(ui, crate::tr!("+ Ajouter des tâches", "+ Add tasks")).on_hover_text(crate::tr!("L'IA les prendra à la reprise", "The AI takes them when it restarts")).clicked() {
                                add_tasks = Some(r.project.clone());
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(20.0);
                        ui.label(
                            RichText::new(crate::trf!(
                                "Reprend toute seule à {} (dans {}), quand le quota de l'abonnement revient.",
                                "Starts again by itself at {} (in {}), when the subscription's quota is back.",
                                auto::hm(r.resume_at),
                                crate::usage::left_text(r.resume_at.saturating_sub(now))
                            ))
                            .size(12.5)
                            .color(t.secondary),
                        );
                    });
                    if !r.outcome.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(20.0);
                            ui.label(RichText::new(crate::trf!("Mise en pause : {}", "Paused: {}", r.outcome)).size(12.0).color(t.tertiary));
                        });
                    }
                });
                ui.add_space(8.0);
            }
        });
        if let Some(p) = cancel {
            for left in cancel_resumes(&mut self.ws.runs, &p) {
                self.end_run_step(&left, &StepIssue::Interrupted);
            }
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        if let Some(p) = add_tasks {
            super::projects::focus_steps(ui.ctx(), &p);
            actions.push(Action::Open(p));
        }
        // The time left changes: keep it fresh.
        ui.ctx().request_repaint_after(Duration::from_secs(30));
        true
    }

    /// The AI a project's next session starts with: the one chosen, or with « Automatique »
    /// the one Orchestra picks now (see `pick_auto`), with the reason to tell the person.
    pub(super) fn pick_agent(&self, id: &str, d: &Draft) -> (Agent, Option<String>) {
        if !d.auto {
            return (d.agent, None);
        }
        let Some(pr) = self.ws.page(id).and_then(|p| p.project.as_ref()) else {
            return (d.agent, None);
        };
        // The step taken first is entrusted to an AI: that one (the start does the same).
        let me = crate::store::computer_id();
        if let Some(a) = pr
            .next_step_for(&me, crate::model::now_ms())
            .and_then(|s| s.agent)
        {
            return (
                a,
                Some(crate::trf!(
                    "{}, à qui la première étape est confiée",
                    "{}, entrusted with the first step",
                    a.label()
                )),
            );
        }
        let figs = self.figures_by_agent();
        let o = &self.ws.settings.orch;
        let plugged = self.ais.plugged();
        let space = crate::space::current();
        let cands: Vec<AutoCandidate> = Agent::ALL
            .into_iter()
            .filter(|a| {
                plugged.contains(&a.ai())
                    && agent_unavailable(*a).is_none()
                    && space.may_run(a.ai().key())
            })
            .map(|a| {
                let f = figs.get(&a).copied().unwrap_or_default();
                AutoCandidate {
                    agent: a,
                    room: o.five_limit(a) - f.five.unwrap_or(0.0),
                    week_full: f
                        .week
                        .is_some_and(|w| crate::usage::near_reserve(w, o.week_limit_of(a), 0.0)),
                    models: crate::catalog::choosable(a),
                }
            })
            .collect();
        let need = crate::catalog::estimate_session(
            pr,
            &d.mission,
            false,
            &crate::store::computer_id(),
            crate::model::now_ms(),
        )
        .with_saving(d.saving, false)
        .tier;
        match pick_auto(cands, need) {
            Some((a, why)) => (a, Some(why)),
            None => (
                d.agent,
                Some(crate::trf!(
                    "aucune IA n'a de marge sous ses limites : {} par défaut",
                    "no AI has room under its limits: {} by default",
                    d.agent.label()
                )),
            ),
        }
    }

    /// « Lancer une IA » on a project's page, with the choices kept for that project (AI,
    /// ceiling, commits, endless). What needs a decision first (an endless session merging
    /// into main) opens its launch settings, which ask for it.
    ///
    /// `step`: « Lancer » on the line of a step entrusted to an AI. A session on that step
    /// alone, with its AI, never endless; another AI's session on the project may be running.
    pub(in crate::app) fn launch_from_project(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        step: Option<&str>,
    ) -> Option<super::notices::Notice> {
        self.start_from_project(ctx, id, step, false)
    }

    /// `launch_from_project`; `confirmed`: the person has just accepted the warning about
    /// an endless session merging into main.
    pub(super) fn start_from_project(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        step: Option<&str>,
        confirmed: bool,
    ) -> Option<super::notices::Notice> {
        if step.is_none() && self.orch.runs.iter().any(|r| r.project == id) {
            self.view = View::Orchestra;
            return None;
        }
        let step_agent = match step {
            Some(sid) => Some(
                self.ws
                    .page(id)
                    .and_then(|p| p.project.as_ref())?
                    .steps
                    .iter()
                    .find(|s| s.id == sid)?
                    .agent?,
            ),
            None => None,
        };
        let dc = self.ws.settings.orch.allow_commits;
        let pr = self.ws.page(id).and_then(|p| p.project.as_ref())?;
        let main = on_main(pr);
        let d = self
            .orch
            .drafts
            .entry(id.to_string())
            .or_insert_with(|| Draft::new(Some(pr), dc))
            .clone();
        if !confirmed && step.is_none() && endless_on_main(d.endless, main) {
            self.orch.confirm_main = Some(id.to_string());
            self.launch_settings = Some(id.to_string());
            return None;
        }
        let title = self
            .ws
            .page(id)
            .map(|p| p.display_title())
            .unwrap_or_default();
        // « Automatique »: the AI picked now; the rest is measured on its subscription.
        // A step's own AI is the person's choice: nothing to pick.
        let (agent, why) = match step_agent {
            Some(a) => (a, None),
            None => self.pick_agent(id, &d),
        };
        let d = Draft {
            agent,
            endless: d.endless && step.is_none(),
            ..d
        };
        let global = self.ws.settings.orch.five_limit(d.agent);
        let figs = self.figures_by_agent();
        let five = figs.get(&d.agent).and_then(|f| f.five);
        if !room_left(five, global) {
            return Some(super::notices::Notice::Error(crate::trf!(
                "La fenêtre de 5 h de {} a atteint sa limite ({global:.0} %) : attendez qu'elle reparte à zéro.",
                "{}'s 5-hour window has reached its limit ({global:.0} %): wait for it to reset.",
                d.agent.label()
            )));
        }
        let ceilings_run: Vec<(Agent, Option<f32>)> =
            self.orch.runs.iter().map(|r| (r.agent, r.ceiling)).collect();
        let want = draft_ceiling(&d, running_ceiling_of(&ceilings_run, d.agent));
        let ceiling = ceiling_choice(want, five, self.ws.settings.orch.default_limit, global);
        let endless = if d.endless { Some(d.end.clone()) } else { None };
        Some(
            match self.launch_run(ctx, id, &d.mission, ceiling, d.commits, endless, d.agent, false, "", step) {
                Ok(()) => super::notices::Notice::Ack(match &why {
                    Some(w) => crate::trf!(
                        "Session autonome lancée sur {title} · IA choisie : {w}.",
                        "Autonomous session started on {title} · AI picked: {w}."
                    ),
                    None => crate::trf!(
                        "Session autonome lancée sur {title} (arrêt à {ceiling:.0} % de la fenêtre de 5 h).",
                        "Autonomous session started on {title} (stops at {ceiling:.0} % of the 5-hour window)."
                    ),
                }),
                Err(e) => super::notices::Notice::Error(format!("{title} : {e}.")),
            },
        )
    }

    /// « Réglages de lancement » of one project, in a window: the AI, its model and economy,
    /// commits and « sur main » (code projects), the ceiling, a mission, endless mode. Opened
    /// from the project's page; kept in the project. Opened by « Lancer une IA » for an
    /// endless session merging into main, it also asks to confirm that start.
    pub(in crate::app) fn launch_settings_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.launch_settings.clone() else {
            return;
        };
        let Some((title, code, on_main_before, pr_launch, pr_ceiling)) =
            self.ws.page(&id).and_then(|p| {
                let pr = p.project.as_ref()?;
                Some((
                    p.display_title(),
                    super::projects::shows_git(ctx, pr),
                    pr.commit_to_main,
                    pr.launch.clone(),
                    pr.session_ceiling,
                ))
            })
        else {
            self.launch_settings = None;
            return;
        };
        let dark = ctx.global_style().visuals.dark_mode;
        let t = tone(dark);
        let lk = super::settings::look(dark);
        let busy = self.orch.runs.iter().any(|r| r.project == id);
        // A start waiting for the person's yes.
        let asking = !busy && self.orch.confirm_main.as_deref() == Some(id.as_str());
        let mut start = false;
        let space = crate::space::current();
        let connected: Vec<Agent> = self
            .ais
            .plugged()
            .into_iter()
            .filter(|a| space.may_run(a.key()))
            .filter_map(Agent::from_ai)
            .collect();
        let figs = self.figures_by_agent();
        let limits = self.ws.settings.orch.clone();
        let default_share = self.ws.settings.orch.default_limit;
        let ceilings_run: Vec<(Agent, Option<f32>)> =
            self.orch.runs.iter().map(|r| (r.agent, r.ceiling)).collect();
        let dc = self.ws.settings.orch.allow_commits;
        if !self.orch.drafts.contains_key(&id) {
            let d = Draft::new(self.ws.page(&id).and_then(|p| p.project.as_ref()), dc);
            self.orch.drafts.insert(id.clone(), d);
        }
        let d = self.orch.drafts.get_mut(&id).unwrap();
        let mut to_main: HashMap<String, bool> = HashMap::new();
        if code {
            to_main.insert(id.clone(), on_main_before);
        }
        let mut close = false;
        let resp = egui::Modal::new(egui::Id::unique("launch-settings"))
            .backdrop_color(Color32::from_black_alpha(100))
            .frame(
                egui::Frame::new()
                    .fill(lk.window)
                    .stroke(egui::Stroke::new(1.0, lk.border))
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::same(22)),
            )
            .show(ctx, |ui| {
                ui.set_width(560.0);
                ui.spacing_mut().item_spacing.y = 10.0;
                ui.label(
                    RichText::new(crate::tr!("Réglages de lancement", "Launch settings"))
                        .font(semibold(super::style::HEADING + 2.0))
                        .color(t.text),
                );
                ui.label(
                    RichText::new(crate::trf!(
                        "{title} · gardés pour les prochaines sessions de ce projet",
                        "{title} · kept for this project's next sessions"
                    ))
                    .size(super::style::SMALL)
                    .color(t.secondary),
                );
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    launch_options(ui, &id, d, busy, &connected, &mut to_main, &lk, &t);
                });
                // The ceiling, on the subscription of the AI chosen.
                let five = figs.get(&d.agent).and_then(|f| f.five);
                let global = limits.five_limit(d.agent);
                let running_at = running_ceiling_of(&ceilings_run, d.agent);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(crate::tr!("Arrêt à", "Stops at")).size(12.5).color(t.secondary));
                    let floor = ceiling_floor(five, global);
                    let mut shown = ceiling_choice(draft_ceiling(d, running_at), five, default_share, global);
                    let dv = ui.add_enabled(
                        !busy,
                        egui::DragValue::new(&mut shown)
                            .range(floor..=global.max(floor))
                            .speed(0.5)
                            .suffix(" %"),
                    );
                    if dv.changed() {
                        d.ceiling = Some(shown);
                        d.touched = true;
                    }
                    // The ceiling chosen stays as it is: it is only shown raised above the
                    // window's figure (and again at the start).
                    let mut now = five
                        .map(|f| crate::trf!("La fenêtre est à {f:.0} % : ", "The window is at {f:.0} %: "))
                        .unwrap_or_default();
                    if !busy {
                        now = parallel_note(running_at, shown, d.touched) + &now;
                    }
                    dv.on_hover_text(crate::trf!(
                        "Plafond de la prochaine session de ce projet, en % de la fenêtre de 5 h de l'abonnement, votre propre usage compris : elle s'arrête quand la fenêtre l'atteint (ex. 37 % utilisés et un plafond de 50 % : elle peut en prendre 13). {now}de {floor:.0} à {global:.0} % (la limite globale). Le plafond choisi est gardé pour les prochaines sessions de ce projet, même après un redémarrage.",
                        "Ceiling of this project's next session, in % of the subscription's 5-hour window, your own use included: it stops when the window reaches it (e.g. 37 % used and a ceiling of 50 %: it may take 13). {now}from {floor:.0} to {global:.0} % (the global limit). The ceiling chosen is kept for this project's next sessions, even after a restart."
                    ));
                    ui.label(
                        RichText::new(crate::tr!("de la fenêtre de 5 h de l'abonnement", "of the subscription's 5-hour window"))
                            .size(12.5)
                            .color(t.tertiary),
                    );
                });
                ui.label(RichText::new(crate::tr!("Consigne (facultatif)", "Mission (optional)")).size(12.5).color(t.secondary));
                ui.add(
                    egui::TextEdit::multiline(&mut d.mission)
                        .hint_text(default_mission())
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .font(FontId::proportional(13.0)),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    super::settings::switch(ui, &mut d.endless, &lk).on_hover_text(crate::tr!(
                        "Quand les étapes sont finies, l'IA trouve elle-même les suivantes et continue, jusqu'aux conditions choisies, son plafond, la limite globale ou la réserve de la semaine.",
                        "When the steps run out, the AI finds the next ones itself and goes on, until the conditions chosen, its ceiling, the global limit or the week's reserve."
                    ));
                    ui.label(RichText::new(crate::tr!("sans fin", "endless")).size(12.5).color(if d.endless { t.text } else { t.tertiary }));
                    if d.endless {
                        ui.label(RichText::new(crate::tr!("· s'arrête dès que l'une est atteinte :", "· stops as soon as one is reached:")).size(12.0).color(t.tertiary));
                        end_conditions(ui, &mut d.end, &t);
                    }
                });
                if endless_on_main(d.endless, to_main.get(&id).copied().unwrap_or(false)) {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("⚠ {}", endless_main_warning()))
                                .size(12.5)
                                .color(super::style::ORANGE),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(6.0);
                // Settings only (a session starts from the project's page), except to
                // confirm the start that opened this window.
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !asking {
                            if super::style::primary(ui, crate::tr!("Fermer", "Close")).clicked() {
                                close = true;
                            }
                            return;
                        }
                        let risky = endless_on_main(d.endless, to_main.get(&id).copied().unwrap_or(false));
                        let label = if risky {
                            crate::tr!("Lancer quand même", "Start anyway")
                        } else {
                            crate::tr!("Lancer", "Start")
                        };
                        if super::style::primary(ui, label).clicked() {
                            start = true;
                            close = true;
                        }
                        if super::style::secondary(ui, crate::tr!("Annuler", "Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        let d = self.orch.drafts.get(&id).cloned().unwrap();
        // Kept in the project.
        if let Some(p) = self.ws.page_mut(&id).and_then(|p| p.project.as_mut()) {
            let mut changed = false;
            if pr_launch.as_ref() != Some(&d.prefs()) {
                p.launch = Some(d.prefs());
                changed = true;
            }
            if d.touched && d.ceiling != pr_ceiling {
                p.session_ceiling = d.ceiling;
                changed = true;
            }
            if let Some(m) = to_main.get(&id)
                && *m != on_main_before
            {
                p.commit_to_main = *m;
                changed = true;
            }
            if changed {
                self.dirty_since.get_or_insert_with(Instant::now);
            }
        }
        if close || resp.should_close() {
            self.launch_settings = None;
            self.orch.confirm_main = None;
        }
        if start && let Some(n) = self.start_from_project(ctx, &id, None, true) {
            self.notice(n);
        }
    }
}

/// The lowest ceiling a session can start with: a whole percent above the 5-hour window's
/// figure (`five`), never past the global limit.
pub(super) fn ceiling_floor(five: Option<f32>, global: f32) -> f32 {
    (five.unwrap_or(0.0).floor() + 1.0).min(global)
}

/// Where a project's next session stops, in absolute % of the 5-hour window: the ceiling
/// chosen, else the window's figure plus the settings' default share (`default_share`);
/// between `ceiling_floor` and the global limit, whole percents.
pub(super) fn ceiling_choice(chosen: Option<f32>, five: Option<f32>, default_share: f32, global: f32) -> f32 {
    let want = chosen.unwrap_or_else(|| five.unwrap_or(0.0) + default_share).round();
    want.clamp(ceiling_floor(five, global), global.max(1.0))
}

/// The ceiling of the sessions already running on the subscription (the highest, if they
/// differ): where a session started beside them begins (see `draft_ceiling`).
pub(super) fn running_ceiling(ceilings: impl IntoIterator<Item = Option<f32>>) -> Option<f32> {
    ceilings.into_iter().flatten().reduce(f32::max)
}

/// `running_ceiling` among the sessions of one AI only (`runs`: each one's AI and ceiling):
/// the ceilings of another AI are on another subscription and never count.
pub(super) fn running_ceiling_of(runs: &[(Agent, Option<f32>)], agent: Agent) -> Option<f32> {
    running_ceiling(runs.iter().filter(|(a, _)| *a == agent).map(|(_, c)| *c))
}

/// The ceiling a project's next session is offered (before `ceiling_choice` puts it in
/// bounds): beside a session already running, that session's (« stop at 50 % » for both),
/// unless the person changed this project's since Orchestra opened; else the one kept.
pub(super) fn draft_ceiling(d: &Draft, running: Option<f32>) -> Option<f32> {
    if d.touched { d.ceiling } else { running.or(d.ceiling) }
}

/// What the ceiling of a session started beside running ones (`running`: theirs) means,
/// for the tooltip in the launch settings: empty with none running. `chosen`: this one's ceiling,
/// `touched`: changed by the person (higher or lower than theirs, for this one only).
pub(super) fn parallel_note(running: Option<f32>, chosen: f32, touched: bool) -> String {
    let Some(r) = running else { return String::new() };
    if !touched || (chosen - r).abs() < 0.5 {
        crate::trf!(
            "Une session tourne déjà avec un plafond de {r:.0} % : celle-ci part du même ; vous pouvez le monter (ex. {:.0} %) pour celle-ci seulement. ",
            "A session is already running with a ceiling of {r:.0} %: this one starts from the same; you may raise it (e.g. {:.0} %) for this one only. ",
            (r + 10.0).min(100.0)
        )
    } else {
        crate::trf!(
            "Celle-ci s'arrêtera à {chosen:.0} %, celle déjà en cours garde son plafond ({r:.0} %). ",
            "This one will stop at {chosen:.0} %, the one already running keeps its ceiling ({r:.0} %). "
        )
    }
}

/// An endless session whose work goes into main by itself: hours of unreviewed commits
/// land on the main branch. Worth a warning before it starts.
pub(super) fn endless_on_main(endless: bool, to_main: bool) -> bool {
    endless && to_main
}

pub(super) fn endless_main_warning() -> &'static str {
    crate::tr!(
        "Attention : mode sans fin avec « sur main » coché. Le travail de chaque session, y compris les étapes que l'IA s'invente, sera fusionné tout seul dans main sans relecture, potentiellement pendant des heures.",
        "Careful: endless mode with « on main » ticked. Each session's work, including the steps the AI makes up, will be merged into main by itself without review, possibly for hours."
    )
}

pub(super) fn default_mission() -> &'static str {
    crate::tr!(
        "Avance sur les étapes ouvertes du projet, dans l'ordre, en vérifiant ton travail.",
        "Move forward on the project's open steps, in order, checking your work."
    )
}

/// In a running session's card: the use of its share of the 5-hour window, as a bar and
/// its figures.
/// A running session on the 5-hour window (all in %, 0..100): what was used before it or by
/// others (`0..before`), its own use (`before..window`), and where it will stop (`stop`):
/// on its budget, or on the global limit if that comes first.
#[derive(Debug, PartialEq)]
pub(super) struct WindowBar {
    pub(super) before: f32,
    pub(super) window: f32,
    pub(super) stop: f32,
}

/// `ceiling`: the session's own (absolute), else worked out from its budget.
pub(super) fn window_bar(window: f32, pct: f32, limit: f32, ceiling: Option<f32>, global: f32) -> WindowBar {
    let window = window.clamp(0.0, 100.0);
    let before = (window - pct.max(0.0)).max(0.0);
    WindowBar {
        before,
        window,
        stop: ceiling
            .unwrap_or(before + limit)
            .min(before + limit)
            .min(global)
            .clamp(0.0, 100.0),
    }
}

/// Never wider than its card: the bar takes what the text leaves (the text was once given
/// a fixed 150 px it didn't fit in, and the overflow widened every card below; see the
/// tests). Too narrow for both side by side, the text goes under the bar.
/// With the 5-hour window's figure (`window`), the bar is that window: the use before the
/// session, the session's own, and a tick where it stops. Without it, the session's
/// share of its budget only.
#[allow(clippy::too_many_arguments)]
pub(super) fn share_row(
    ui: &mut egui::Ui,
    id: &str,
    pct: f32,
    limit: f32,
    ceiling: Option<f32>,
    window: Option<f32>,
    global: f32,
    t: &Tone,
) {
    let text = match window {
        Some(w) => crate::trf!(
            "fenêtre de 5 h : {w:.0} % · cette session : {} % · arrêt à {:.0} %",
            "5-hour window: {w:.0} % · this session: {} % · stops at {:.0} %",
            crate::i18n::dec1(pct),
            window_bar(w, pct, limit, ceiling, global).stop
        ),
        None => match ceiling {
            Some(c) => crate::trf!(
                "{} % utilisés · arrêt à {c:.0} % de la fenêtre de 5 h",
                "{} % used · stops at {c:.0} % of the 5-hour window",
                crate::i18n::dec1(pct)
            ),
            None => crate::trf!(
                "{} % utilisés sur {:.0} %",
                "{} % used of {:.0} %",
                crate::i18n::dec1(pct),
                limit
            ),
        },
    };
    let font = FontId::proportional(12.5);
    let text_w = ui
        .fonts_mut(|f| f.layout_no_wrap(text.clone(), font.clone(), t.secondary))
        .size()
        .x;
    let indent = 20.0;
    let gap = ui.spacing().item_spacing.x;
    let room = ui.available_width() - indent;
    let side_by_side = room - gap - text_w >= 60.0;
    let bar_w = if side_by_side {
        room - gap - text_w
    } else {
        room.max(0.0)
    };
    let draw_bar = |ui: &mut egui::Ui| {
        let (bar, _) = ui.allocate_exact_size(egui::vec2(bar_w, 8.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(bar, 4.0, t.fill);
        let x = |v: f32| bar.left() + bar.width() * (v / 100.0).clamp(0.0, 1.0);
        match window {
            Some(w) => {
                let b = window_bar(w, pct, limit, ceiling, global);
                let end = super::projects::smooth(ui, ("run-window", id), b.window);
                let near = b.window >= b.stop - (b.stop - b.before) * 0.15;
                let p = ui.painter();
                // Before the session (or by others): muted; the session's own: its colour.
                p.rect_filled(
                    egui::Rect::from_min_max(bar.min, egui::pos2(x(b.before), bar.bottom())),
                    4.0,
                    t.tertiary.gamma_multiply(0.45),
                );
                p.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x(b.before), bar.top()),
                        egui::pos2(x(end.max(b.before)), bar.bottom()),
                    ),
                    0.0,
                    if near {
                        super::projects::ORANGE
                    } else {
                        super::projects::GREEN
                    },
                );
                p.vline(
                    x(b.stop),
                    egui::Rangef::new(bar.top() - 3.0, bar.bottom() + 3.0),
                    egui::Stroke::new(2.0, super::projects::ORANGE),
                );
            }
            None => {
                let frac = super::projects::smooth(
                    ui,
                    ("run-pct", id),
                    (pct / limit.max(0.1)).clamp(0.0, 1.0),
                );
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac, 8.0)),
                    4.0,
                    if frac > 0.85 {
                        super::projects::ORANGE
                    } else {
                        super::projects::GREEN
                    },
                );
            }
        }
    };
    let label = |ui: &mut egui::Ui| {
        ui.add(egui::Label::new(RichText::new(text.as_str()).font(font.clone()).color(t.secondary)).wrap()).on_hover_text(crate::tr!(
            "La barre est la fenêtre de 5 h de l'abonnement : en gris ce qui était utilisé avant cette session (ou par d'autres), en vert ce qu'elle a utilisé depuis son lancement. Le trait orange : où elle s'arrête, le plafond choisi à son lancement en % de la fenêtre (ex. 37 % utilisés et un plafond de 50 % : elle peut en prendre 13), ou la limite globale si elle vient avant.",
            "The bar is the subscription's 5-hour window: in grey what was used before this session (or by others), in green what it used since it started. The orange tick: where it stops, the ceiling chosen at its start in % of the window (e.g. 37 % used and a ceiling of 50 %: it may take 13), or the global limit if that comes first."
        ));
    };
    ui.horizontal(|ui| {
        ui.add_space(indent);
        if side_by_side {
            draw_bar(ui);
            label(ui);
        } else {
            ui.vertical(|ui| {
                draw_bar(ui);
                label(ui);
            });
        }
    });
}

/// The mark before « En train de s'occuper de … »: a step in progress, as in the steps card
/// (a ring with a dot), drawn rather than a glyph (« ✦ » is missing from the app's font and
/// showed as a stack of lines), in the card's muted grey: the same size as the green dot of
/// the session's title (radius 4), so the two marks read as one family, the green one alone
/// carrying colour.
pub(super) fn working_mark(ui: &mut egui::Ui, t: &Tone) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 14.0), egui::Sense::hover());
    let c = r.center();
    ui.painter()
        .circle_stroke(c, 4.0, egui::Stroke::new(1.2, t.tertiary));
    ui.painter().circle_filled(c, 1.6, t.tertiary);
}

/// A colour per project, stable (from its name).
pub(super) fn project_color(title: &str) -> Color32 {
    const HUES: [Color32; 6] = [
        super::style::BLUE,
        Color32::from_rgb(68, 160, 110),
        super::style::PINK,
        super::style::ORANGE,
        super::style::PURPLE,
        Color32::from_rgb(0, 150, 160),
    ];
    HUES[(hash(title) % HUES.len() as u64) as usize]
}

/// The end conditions of an endless session, each one ticked or not; the first reached stops it.
pub(super) fn end_conditions(ui: &mut egui::Ui, c: &mut crate::model::EndConditions, t: &Tone) {
    let small = |s: &str| RichText::new(s).size(12.0).color(t.secondary);
    let mut on = c.hours.is_some();
    if ui
        .checkbox(&mut on, small(crate::tr!("une durée", "a time")))
        .changed()
    {
        c.hours = if on { Some(4.0) } else { None };
    }
    if let Some(h) = c.hours.as_mut() {
        ui.add(
            egui::DragValue::new(h)
                .range(0.5..=72.0)
                .speed(0.25)
                .suffix(" h"),
        );
    }
    let mut on = c.steps.is_some();
    if ui
        .checkbox(
            &mut on,
            small(crate::tr!("des étapes faites", "steps done")),
        )
        .changed()
    {
        c.steps = if on { Some(5) } else { None };
    }
    if let Some(n) = c.steps.as_mut() {
        ui.add(egui::DragValue::new(n).range(1..=500));
    }
    let mut on = c.turns.is_some();
    if ui
        .checkbox(&mut on, small(crate::tr!("des tours", "turns")))
        .changed()
    {
        c.turns = if on { Some(60) } else { None };
    }
    if let Some(n) = c.turns.as_mut() {
        ui.add(egui::DragValue::new(n).range(1..=1000));
    }
    ui.checkbox(&mut c.when_done, small(crate::tr!("toutes les étapes faites", "all steps done"))).on_hover_text(crate::tr!(
        "Coché : la session s'arrête une fois les étapes faites. Décoché : l'IA crée elle-même les étapes suivantes et continue.",
        "Ticked: the session stops once the steps are done. Unticked: the AI creates the next steps itself and goes on."
    ));
    if c.hours.is_none() && c.steps.is_none() && c.turns.is_none() && !c.when_done {
        ui.label(RichText::new(crate::tr!("(aucune : seulement à « arrêt à »)", "(none: only at « stops at »)")).size(12.0).color(t.tertiary)).on_hover_text(crate::tr!(
            "Sans condition, la session tourne jusqu'à son plafond, la limite globale ou la réserve de la semaine (ou deux tours de suite sans rien à faire).",
            "With no condition, the session runs until its ceiling, the global limit or the week's reserve (or two turns in a row with nothing to do)."
        ));
    }
}

/// Projects listed by the Orchestration page when nothing runs.
pub(super) const QUIET_PROJECTS: usize = 5;

/// The projects a session could work on: a folder and steps not done. Each with its id,
/// title and number of open steps, the latest active first.
pub(super) fn projects_with_open_steps(ws: &crate::model::Workspace) -> Vec<(String, String, usize)> {
    ws.launch_projects()
        .iter()
        .filter_map(|p| {
            let x = p.project.as_ref()?;
            let open = x
                .steps
                .iter()
                .filter(|s| s.status != crate::project::StepStatus::Done)
                .count();
            (open > 0 && !x.folder.trim().is_empty())
                .then(|| (p.id.clone(), p.display_title(), open))
        })
        .collect()
}

/// Width of the window's name (« 5 h », « Semaine ») before an AI's bars.
pub(super) const LIMIT_LABEL_W: f32 = 84.0;

/// One AI's limits: a bar per window it has, each with its limit, then the way back to the
/// common ones once it has its own; an AI whose use can't be read says so instead.
pub(super) fn agent_limits(
    ui: &mut egui::Ui,
    a: Agent,
    fig: Figures,
    o: &mut crate::model::OrchSettings,
    t: &Tone,
) {
    if let Some(why) = crate::usage::unreadable_reason(a) {
        ui.horizontal(|ui| {
            ui.add_space(LIMIT_LABEL_W);
            ui.add(
                egui::Label::new(
                    RichText::new(crate::trf!(
                        "Utilisation non mesurable : {why} Ses sessions autonomes s'arrêtent faute de mesure.",
                        "Usage can't be measured: {why} Its autonomous sessions stop for lack of a measure."
                    ))
                    .size(12.0)
                    .color(t.tertiary),
                )
                .wrap(),
            );
        });
        return;
    }
    // Mistral Vibe is measured on its monthly budget, in place of a 5-hour window.
    let first = if a == Agent::Vibe {
        crate::tr!("Mois", "Month")
    } else {
        crate::tr!("5 h", "5 h")
    };
    let mut five = o.five_limit(a);
    if limit_line(ui, first, fig.five, &mut five, LimitKind::Stop, t) {
        o.set_five_limit(a, five);
    }
    if a != Agent::Vibe {
        let mut week = o.week_limit_of(a);
        if limit_line(ui, crate::tr!("Semaine", "Week"), fig.week, &mut week, LimitKind::Reserve, t) {
            o.set_reserve_of(a, 100.0 - week);
        }
    }
    let own = o
        .per_agent
        .get(crate::model::agent_key(a))
        .is_some_and(|l| l.five.is_some() || l.week.is_some());
    if own {
        ui.horizontal(|ui| {
            ui.add_space(LIMIT_LABEL_W);
            if super::style::quiet(ui, crate::tr!("↺ Valeurs communes", "↺ Common values"))
                .on_hover_text(crate::trf!(
                    "Revenir aux limites communes : arrêt à {:.0} % de la fenêtre, réserve de {:.0} %.",
                    "Back to the common limits: stop at {:.0} % of the window, reserve of {:.0} %.",
                    o.global_limit,
                    o.reserve()
                ))
                .clicked()
            {
                o.per_agent.remove(crate::model::agent_key(a));
            }
        });
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum LimitKind {
    /// « arrêt à X % »: where the sessions stop.
    Stop,
    /// « réserve X % »: what is kept, past the handle.
    Reserve,
}

/// A window's line: its name, the bar (use so far, the zone past the limit tinted red, the
/// limit as a handle that can be dragged), the figure, and the limit as a number. True when
/// the limit changed (`limit`: % of the window the AIs may reach).
pub(super) fn limit_line(
    ui: &mut egui::Ui,
    label: &str,
    used: Option<f32>,
    limit: &mut f32,
    kind: LimitKind,
    t: &Tone,
) -> bool {
    let mut changed = false;
    let (lo, hi) = match kind {
        LimitKind::Stop => (5.0, 100.0),
        LimitKind::Reserve => (5.0, 100.0),
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (lr, _) = ui.allocate_exact_size(egui::vec2(LIMIT_LABEL_W - 10.0, 22.0), egui::Sense::hover());
        ui.painter().text(
            lr.left_center() + egui::vec2(16.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.5),
            t.secondary,
        );
        let right = 190.0;
        let bw = (ui.available_width() - right).max(80.0);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(bw, 22.0), egui::Sense::click_and_drag());
        let bar = egui::Rect::from_center_size(rect.center(), egui::vec2(bw, 8.0));
        let x_of = |v: f32| bar.left() + bar.width() * (v / 100.0).clamp(0.0, 1.0);
        // Dragging or clicking along the bar puts the limit there.
        if (resp.dragged() || resp.clicked())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let v = (((p.x - bar.left()) / bar.width()) * 100.0).round().clamp(lo, hi);
            if (v - *limit).abs() >= 0.5 {
                *limit = v;
                changed = true;
            }
        }
        let hov = ui
            .ctx()
            .animate_bool_with_time(resp.id.with("hov"), resp.hovered() || resp.dragged(), 0.12);
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        let p = ui.painter();
        p.rect_filled(bar, 4.0, t.fill);
        let lx = x_of(*limit);
        p.rect_filled(
            egui::Rect::from_min_max(egui::pos2(lx, bar.top()), bar.max),
            egui::CornerRadius { nw: 0, sw: 0, ne: 4, se: 4 },
            super::style::RED.gamma_multiply(0.14),
        );
        if let Some(u) = used {
            let frac = ui
                .ctx()
                .animate_value_with_time(resp.id.with("v"), (u / 100.0).clamp(0.0, 1.0), 0.6);
            let color = if u >= *limit {
                super::style::RED
            } else if u >= *limit * 0.85 {
                super::style::ORANGE
            } else {
                super::style::BLUE
            };
            p.rect_filled(
                egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac, bar.height())),
                4.0,
                color,
            );
        }
        // The handle: a line across the bar and a round grip that grows under the pointer.
        p.vline(
            lx,
            egui::Rangef::new(bar.top() - 4.0, bar.bottom() + 4.0),
            egui::Stroke::new(2.0, super::style::ORANGE),
        );
        p.circle_filled(egui::pos2(lx, bar.top() - 5.0), 4.0 + 1.5 * hov, super::style::ORANGE);
        resp.on_hover_text(match kind {
            LimitKind::Stop => crate::tr!(
                "Glissez le repère : les sessions de cette IA s'arrêtent quand la fenêtre l'atteint, votre propre usage compris. Chaque session a en plus son plafond (« arrêt à » au lancement).",
                "Drag the handle: this AI's sessions stop when the window reaches it, your own use included. Each session also has its own ceiling (« stops at » at its start)."
            ),
            LimitKind::Reserve => crate::tr!(
                "Glissez le repère : au-delà, c'est votre réserve. Les sessions de cette IA s'arrêtent avant de l'entamer et reprennent quand la semaine repart à zéro.",
                "Drag the handle: past it is your reserve. This AI's sessions stop before touching it and start again once the week resets."
            ),
        });
        // The figure, then the limit as a number.
        let fig = used.map(|u| format!("{u:.0} %")).unwrap_or_else(|| "—".into());
        let (fr, _) = ui.allocate_exact_size(egui::vec2(42.0, 22.0), egui::Sense::hover());
        ui.painter().text(
            fr.right_center(),
            egui::Align2::RIGHT_CENTER,
            fig,
            semibold(12.5),
            if used.is_some() { t.text } else { t.tertiary },
        );
        let (word, mut v) = match kind {
            LimitKind::Stop => (crate::tr!("arrêt à", "stop at"), *limit),
            LimitKind::Reserve => (crate::tr!("réserve", "reserve"), 100.0 - *limit),
        };
        ui.label(RichText::new(word).size(12.0).color(t.tertiary));
        let range = match kind {
            LimitKind::Stop => lo..=hi,
            LimitKind::Reserve => 0.0..=95.0,
        };
        if ui
            .add(egui::DragValue::new(&mut v).range(range).speed(0.5).suffix(" %"))
            .changed()
        {
            *limit = match kind {
                LimitKind::Stop => v,
                LimitKind::Reserve => 100.0 - v,
            };
            changed = true;
        }
    });
    changed
}

/// Record for the tutorial the area drawn since `top` (the column's width).
pub(super) fn mark_from(ui: &egui::Ui, name: &str, top: egui::Pos2) {
    let bottom = ui.cursor().min.y;
    if bottom > top.y {
        let w = ui.max_rect().width().max(1.0);
        let r = egui::Rect::from_min_max(top, egui::pos2(top.x + w, bottom));
        super::tutorial::mark(ui, name, r);
    }
}

/// Below this width, a running session's links go under its title.
pub(super) const RUN_ONE_LINE: f32 = 460.0;

/// The project's sessions merge into main by themselves: « sur main » ticked, and a code
/// project (the box isn't shown for one without code, so an old tick there counts for nothing).
pub(super) fn on_main(pr: &crate::project::Project) -> bool {
    pr.commit_to_main && crate::project::code_kind(&pr.folder, pr.code)
}

/// An AI « Automatique » may pick: its room left under its 5-hour limit (points of its
/// window), whether its week's reserve is near, and the models it offers here.
pub(super) struct AutoCandidate {
    pub(super) agent: Agent,
    pub(super) room: f32,
    pub(super) week_full: bool,
    pub(super) models: Vec<crate::catalog::ModelInfo>,
}

/// « IA : Automatique »: among the AIs with room (at least a point under their 5-hour
/// limit, their week's reserve not near), the cheapest model able to do work of tier `need`;
/// on equal cost the AI with the most room. None when no AI has room. With the reason, in
/// a few words: « Codex · GPT-5 mini, le moins cher des modèles moyen ou plus ».
pub(super) fn pick_auto(
    mut cands: Vec<AutoCandidate>,
    need: crate::catalog::Tier,
) -> Option<(Agent, String)> {
    cands.retain(|c| c.room >= 1.0 && !c.week_full);
    if cands.is_empty() {
        return None;
    }
    // The most room first: `choose` keeps the first AI on equal cost.
    cands.sort_by(|a, b| b.room.total_cmp(&a.room));
    let lists: Vec<(Agent, Vec<crate::catalog::ModelInfo>)> =
        cands.iter().map(|c| (c.agent, c.models.clone())).collect();
    Some(match crate::catalog::choose(&lists, need) {
        Some(c) => (
            c.agent,
            format!("{} · {}, {}", c.agent.label(), c.model.label, c.reason),
        ),
        // No model list read (an AI that doesn't give one): the one with the most room.
        None => (
            cands[0].agent,
            crate::trf!(
                "{}, la plus de marge sous ses limites",
                "{}, the most room under its limits",
                cands[0].agent.label()
            ),
        ),
    })
}

/// Why an AI can't be chosen for a session started alone, if it can't (None: it can).
pub(super) fn agent_unavailable(agent: Agent) -> Option<&'static str> {
    if agent.ready() {
        None
    } else if !crate::usage::reports_usage(agent) {
        Some(crate::tr!(
            "Pas encore possible : une session s'arrête à son « arrêt à », un % de l'abonnement, et cette IA ne donne pas ce % (voir sa tuile à gauche). Orchestra ne saurait pas quand l'arrêter.",
            "Not possible yet: a session stops at its « stop at », a % of the subscription, and this AI doesn't give that % (see its tile on the left). Orchestra wouldn't know when to stop it."
        ))
    } else {
        Some(crate::tr!(
            "Pas encore possible : son travail autonome n'est pas encore vérifié sur cet ordinateur.",
            "Not possible yet: its autonomous work isn't checked on this computer yet."
        ))
    }
}

/// The AIs listed in a project's choice of the AI: those plugged into Orchestra, in the usual
/// order, plus the one already chosen even if it's no longer plugged (so the choice shown is
/// never hidden). AIs not installed or unplugged are left out.
pub(super) fn agent_choices(connected: &[Agent], current: Agent) -> Vec<Agent> {
    Agent::ALL
        .into_iter()
        .filter(|a| *a == current || connected.contains(a))
        .collect()
}

/// The AI a project's sessions are started with, among the plugged ones: those wired for
/// working alone can be picked, the others are listed greyed out with the reason. Kept for
/// the next sessions.
/// The first line of the launch settings: the AI, then for a code project « commits » and
/// « sur main » (`to_main` holds only those: one without code shows no git words, and its
/// sessions make no commit).
#[allow(clippy::too_many_arguments)]
pub(super) fn launch_options(
    ui: &mut egui::Ui,
    id: &str,
    d: &mut Draft,
    busy: bool,
    connected: &[Agent],
    to_main: &mut HashMap<String, bool>,
    lk: &super::settings::Look,
    t: &Tone,
) {
    ui.label(RichText::new(crate::tr!("IA", "AI")).size(12.0).color(t.tertiary));
    agent_pick(ui, id, d, busy, connected);
    model_pick(ui, id, d, busy);
    saving_pick(ui, id, d, busy);
    ui.add_space(8.0);
    if to_main.contains_key(id) {
        super::settings::switch(ui, &mut d.commits, lk).on_hover_text(crate::tr!("L'IA peut faire des commits sur sa branche (Orchestra l'envoie sur GitHub après chaque tour).\nGardé pour les prochaines sessions de ce projet.", "The AI may commit on its branch (Orchestra pushes it to GitHub after each turn).\nKept for this project's next sessions."));
        ui.label(RichText::new("commits").size(12.0).color(t.tertiary));
        ui.add_space(8.0);
    }
    if let Some(m) = to_main.get_mut(id) {
        ui.checkbox(m, RichText::new(crate::tr!("sur main", "on main")).size(12.0).color(t.tertiary)).on_hover_text(crate::tr!(
            "À la fin de chaque session, son travail est fusionné tout seul dans main (seulement si le dossier du projet est sur main ou master)",
            "At the end of each session, its work is merged into main by itself (only when the project's folder is on main or master)"
        ));
    }
}

pub(super) fn agent_pick(ui: &mut egui::Ui, id: &str, d: &mut Draft, busy: bool, connected: &[Agent]) {
    ui.add_enabled_ui(!busy, |ui| {
        let shown = if d.auto {
            crate::tr!("✨ Automatique", "✨ Automatic")
        } else {
            d.agent.label()
        };
        egui::ComboBox::from_id_salt(("run-agent", id))
            .selected_text(RichText::new(shown).size(12.5))
            .width(130.0)
            .show_ui(ui, |ui| {
                // Wide enough for the reason written under an AI that can't be chosen.
                ui.set_min_width(260.0);
                ui.set_max_width(260.0);
                // First and by default: Orchestra picks the AI and its model at each start.
                if ui
                    .add(egui::Button::selectable(d.auto, crate::tr!("✨ Automatique", "✨ Automatic")))
                    .clicked()
                {
                    d.auto = true;
                    d.model.clear();
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(crate::tr!(
                            "À chaque lancement, l'IA et le modèle les moins chers capables de l'étape, parmi celles qui ont de la marge sous leurs limites.",
                            "At each start, the cheapest AI and model able to do the step, among those with room under their limits."
                        ))
                        .size(11.5)
                        .color(ui.visuals().weak_text_color()),
                    )
                    .wrap(),
                );
                ui.separator();
                for a in agent_choices(connected, d.agent) {
                    let why = if connected.contains(&a) {
                        agent_unavailable(a)
                    } else {
                        Some(crate::tr!(
                            "Pas branchée à Orchestra sur cet ordinateur : branchez-la dans Réglages › IA.",
                            "Not plugged into Orchestra on this computer: plug it in under Settings › AI."
                        ))
                    };
                    let r = ui.add_enabled(
                        why.is_none(),
                        egui::Button::selectable(!d.auto && d.agent == a, a.label()),
                    );
                    if r.clicked() {
                        // Another AI: its own models, so back to Orchestra's choice.
                        if d.agent != a || d.auto {
                            d.model.clear();
                        }
                        d.agent = a;
                        d.auto = false;
                    }
                    // Said under it, not only on hover: a greyed AI with no word looks broken.
                    if let Some(w) = why {
                        ui.add(
                            egui::Label::new(
                                RichText::new(w).size(11.5).color(ui.visuals().weak_text_color()),
                            )
                            .wrap(),
                        );
                        ui.add_space(4.0);
                    }
                }
            })
            .response
            .on_hover_text(crate::tr!(
                "L'IA qui travaille sur ce projet : choisie par Orchestra à chaque lancement (« Automatique »), ou toujours la même. Une étape confiée à une IA passe avant. Gardé pour les prochaines sessions de ce projet.",
                "The AI working on this project: picked by Orchestra at each start (« Automatic »), or always the same. A step entrusted to an AI comes first. Kept for this project's next sessions."
            ));
    });
}

/// The model of the project's sessions: « Automatique » (the cheapest able to do the step,
/// see `catalog::choose`), or one the AI offers, imposed. A step's own model wins over it.
pub(super) fn model_pick(ui: &mut egui::Ui, id: &str, d: &mut Draft, busy: bool) {
    if d.auto {
        // Chosen with the AI, at each start.
        ui.label(
            RichText::new(crate::tr!("modèle choisi avec l'IA", "model picked with the AI"))
                .size(12.0)
                .color(ui.visuals().weak_text_color()),
        );
        return;
    }
    ui.add_enabled_ui(!busy, |ui| {
        egui::ComboBox::from_id_salt(("run-model", id))
            .selected_text(RichText::new(if d.model.is_empty() {
                crate::tr!("Modèle auto", "Auto model")
            } else {
                d.model.as_str()
            }).size(12.5))
            .width(130.0)
            .show_ui(ui, |ui| {
                ui.set_min_width(200.0);
                ui.selectable_value(
                    &mut d.model,
                    String::new(),
                    crate::tr!("Automatique", "Automatic"),
                );
                // Read only while open: the lists come from the AIs' files.
                for m in crate::catalog::available(d.agent) {
                    let text = format!("{} · {}", m.label, m.tier.label());
                    ui.add_enabled_ui(!m.needs_higher_plan, |ui| {
                        ui.selectable_value(&mut d.model, m.id.clone(), text);
                    });
                }
            })
            .response
            .on_hover_text(crate::tr!(
                "Le modèle des sessions de ce projet. « Automatique » : le moins cher capable de faire l'étape. Un modèle imposé sur une étape passe avant.\nGardé pour les prochaines sessions de ce projet.",
                "The model of this project's sessions. « Automatic »: the cheapest able to do the step. A model imposed on a step comes first.\nKept for this project's next sessions."
            ));
    });
}

/// The economy slider of the project: Économe / Équilibré / Qualité (see `model::Saving`).
pub(super) fn saving_pick(ui: &mut egui::Ui, id: &str, d: &mut Draft, busy: bool) {
    let thinks = crate::catalog::accepts_effort(d.agent);
    ui.add_enabled_ui(!busy, |ui| {
        egui::ComboBox::from_id_salt(("run-saving", id))
            .selected_text(RichText::new(d.saving.label()).size(12.5))
            .width(90.0)
            .show_ui(ui, |ui| {
                for s in crate::model::Saving::ALL {
                    ui.selectable_value(&mut d.saving, s, s.label());
                }
            })
            .response
            .on_hover_text(crate::tr!(
                "Curseur d'économie : Économe = modèle plus léger et peu de réflexion, Qualité = modèle plus haut et plus de réflexion. Sans effet sur un modèle imposé.\nGardé pour les prochaines sessions de ce projet.",
                "Economy slider: Economical = lighter model and little thinking, Quality = higher model and more thinking. No effect on an imposed model.\nKept for this project's next sessions."
            ));
    });
    if !thinks && d.saving != crate::model::Saving::Balanced {
        ui.label(RichText::new(crate::tr!("modèle seulement", "model only")).size(11.5).weak())
            .on_hover_text(crate::tr!(
                "Cette IA n'a pas de niveau de réflexion réglable : le curseur n'agit que sur le choix du modèle (aucun effet si le modèle est imposé).",
                "This AI has no adjustable thinking level: the slider only acts on the model choice (no effect if the model is imposed)."
            ));
    }
}
