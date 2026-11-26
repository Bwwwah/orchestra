//! The engine of the sessions: launch, the per-frame loop that moves each one along,
//! limits and measures, the end of a session and the work left in its copy.

use super::*;

impl NotesApp {
    /// The Orchestration page, with the terminal of the running session `run` open.
    pub(in crate::app) fn watch_run(&mut self, run: String) {
        self.orch.watching = Some(run);
        self.view = View::Orchestra;
    }

    /// The project's effort level (economy slider) as the launch settings show it: the draft
    /// of the card and the choices kept in the project change together.
    pub(in crate::app) fn project_saving(&self, pid: &str) -> crate::model::Saving {
        self.orch
            .drafts
            .get(pid)
            .map(|d| d.saving)
            .or_else(|| {
                let p = self.ws.page(pid)?.project.as_ref()?;
                Some(p.launch.as_ref().map(|l| l.saving).unwrap_or_default())
            })
            .unwrap_or_default()
    }

    /// Sets the project's effort level for its next sessions (the end of the framing).
    pub(in crate::app) fn set_project_saving(&mut self, pid: &str, s: crate::model::Saving) {
        if let Some(d) = self.orch.drafts.get_mut(pid) {
            d.saving = s;
        }
        let commits = self.ws.settings.orch.allow_commits;
        if let Some(p) = self.ws.page_mut(pid).and_then(|p| p.project.as_mut()) {
            let mut prefs = p.launch.clone().unwrap_or_else(|| default_prefs(commits));
            if prefs.saving == s {
                return;
            }
            prefs.saving = s;
            p.launch = Some(prefs);
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    /// « Veille automatique » (Veille · CVE card): every watched code project is checked
    /// once a day; a check that finds an important vulnerability no step names yet asks for
    /// a fix session (`cve_fix_tick`), unless a session already runs on the project (the
    /// result then waits for it to end). Looked at every 30 seconds.
    pub(super) fn cve_watch_tick(&mut self, ctx: &egui::Context) {
        let key = egui::Id::unique("cve-watch-tick");
        let t = ctx.input(|i| i.time);
        if ctx.data(|d| d.get_temp::<f64>(key)).is_some_and(|at| t - at < 30.0) {
            return;
        }
        ctx.data_mut(|d| d.insert_temp(key, t));
        // One fix request at a time (the button's, or one already asked for).
        if super::watch::fix_pending(ctx) {
            return;
        }
        let now = crate::model::now_ms();
        let busy: Vec<String> = self.orch.runs.iter().map(|r| r.project.clone()).collect();
        let mut fix: Option<super::watch::FixRequest> = None;
        for page in self.ws.pages.iter_mut().filter(|p| !p.trashed) {
            let Some(pr) = page.project.as_mut() else { continue };
            let folder = pr.folder.trim().to_string();
            if !pr.cve_watch || folder.is_empty() || super::watch::checking(&folder) {
                continue;
            }
            let last = super::watch::last_result(&folder);
            if let Some((at, urgent)) = &last
                && *at > pr.cve_seen
            {
                if busy.contains(&page.id) {
                    continue;
                }
                pr.cve_seen = *at;
                self.dirty_since.get_or_insert_with(Instant::now);
                let titles: Vec<String> = pr.steps.iter().map(|s| s.title.clone()).collect();
                let new = super::watch::untreated(urgent, &titles);
                if !new.is_empty() && fix.is_none() {
                    fix = Some(super::watch::FixRequest { project: page.id.clone(), findings: new });
                }
                continue;
            }
            if super::watch::check_due(pr.cve_seen, last.map(|(at, _)| at), now)
                && crate::project::code_kind(&folder, pr.code)
                && !crate::audit::ecosystems(std::path::Path::new(&folder)).is_empty()
            {
                super::watch::start_check(&folder, ctx);
            }
        }
        if let Some(req) = fix {
            super::watch::request_fix(ctx, req);
        }
    }

    /// « Corriger dans une branche fix » (Veille · CVE card): one « Veille CVE » step per
    /// important vulnerability not already listed, then a session on a fix branch, with the
    /// project's own ceiling and AI.
    pub(super) fn cve_fix_tick(&mut self, ctx: &egui::Context) {
        let Some(req) = super::watch::take_fix_request(ctx) else {
            return;
        };
        let Some(pr) = self.ws.page_mut(&req.project).and_then(|p| p.project.as_mut()) else {
            return;
        };
        for s in super::watch::fix_steps(&req.findings) {
            let open = pr
                .steps
                .iter()
                .any(|x| x.title == s && x.status != crate::project::StepStatus::Done);
            if !open {
                pr.add_step(&s, "veille");
            }
        }
        let d = Draft::new(Some(pr), true);
        let ceiling_kept = pr.session_ceiling;
        self.dirty_since.get_or_insert_with(Instant::now);
        let o = &self.ws.settings.orch;
        let five = self.agent_figures(d.agent).five;
        let ceiling = ceiling_choice(ceiling_kept, five, o.default_limit, o.five_limit(d.agent));
        let mission = super::watch::fix_mission(&req.findings);
        let n = match self.launch_run(ctx, &req.project, &mission, ceiling, true, None, d.agent, true, "", None) {
            Ok(()) => super::notices::Notice::Info(crate::tr!(
                "Session de correction lancée sur une branche fix : fusionnez-la une fois vérifiée.",
                "Fix session started on a fix branch: merge it once checked."
            ).into()),
            Err(e) => super::notices::Notice::Error(crate::trf!(
                "Étapes « Veille CVE » ajoutées, mais la session n'a pas pu partir : {e}",
                "« CVE watch » steps added, but the session couldn't start: {e}"
            )),
        };
        self.notice(n);
    }

    /// Start an autonomous session on a project. `fix`: a security fix (CVE watch), always on
    /// a branch of its own named `orchestra/auto-fix-…`, never on `dev`. `model`: the model
    /// the AI works with (empty: its own default). `step`: the only step the session is
    /// sent on (`auto::step_mission`; `mission` then only completes it); it is « doing »,
    /// reserved for the session, once that has started.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn launch_run(
        &mut self,
        ctx: &egui::Context,
        project: &str,
        mission: &str,
        ceiling: f32,
        commits: bool,
        endless: Option<crate::model::EndConditions>,
        agent: Agent,
        fix: bool,
        model: &str,
        step: Option<&str>,
    ) -> Result<(), String> {
        let page = self
            .ws
            .page(project)
            .ok_or(crate::tr!("projet introuvable", "project not found"))?;
        let pr = page.project.as_ref().ok_or(crate::tr!(
            "ce n'est pas un projet",
            "this is not a project"
        ))?;
        let title = page.display_title();
        let folder = pr.folder.trim().to_string();
        if folder.is_empty() {
            return Err(crate::tr!(
                "indiquez d'abord le dossier du projet",
                "set the project's folder first"
            )
            .into());
        }
        // Nothing to work on (and not endless, where the AI finds its steps): it would stop
        // at once, after starting Claude Code for nothing.
        if endless.is_none()
            && !pr
                .steps
                .iter()
                .any(|s| s.status != crate::project::StepStatus::Done)
        {
            return Err(crate::tr!(
                "aucune étape ouverte : ajoutez des étapes, ou activez « sans fin »",
                "no open step: add steps, or turn on « endless »"
            )
            .into());
        }
        // What the session takes first: the step it is sent on, else the project's next one.
        let aim = match step {
            Some(id) => std::borrow::Cow::Owned(
                pr.focused_on(id, &crate::store::computer_id(), crate::model::now_ms())
                    .ok_or(crate::tr!(
                        "cette étape est terminée, supprimée ou réservée par l'IA d'un autre ordinateur",
                        "this step is finished, deleted or reserved by another computer's AI"
                    ))?,
            ),
            None => std::borrow::Cow::Borrowed(pr),
        };
        // The AI assigned to the step taken first wins over the card's; none: the card's.
        let agent = aim.agent_for_next(agent, &crate::store::computer_id(), crate::model::now_ms());
        // One session at a time per project and per AI. Two different AIs go in parallel: each
        // works in its own copy and reserves its steps under its own name (`Agent::holder`), so
        // they never take the same step. Merging stays per branch.
        if self
            .orch
            .runs
            .iter()
            .any(|r| r.project == project && r.agent == agent)
        {
            return Err(crate::tr!(
                "une session tourne déjà sur ce projet avec cette IA",
                "a session is already running on this project with this AI"
            )
            .into());
        }
        // A step that a session running here is doing isn't given to a second one.
        if let Some(id) = step {
            let computer = crate::store::computer_id();
            let held = pr
                .steps
                .iter()
                .find(|s| s.id == id)
                .map_or("", |s| s.held_by.as_str());
            if self.orch.runs.iter().any(|r| {
                r.project == project
                    && (held == holder_of(r.agent, &computer)
                        || self.ws.runs.iter().any(|x| x.id == r.id && x.step == id))
            }) {
                return Err(crate::tr!(
                    "une session travaille déjà sur cette étape",
                    "a session is already working on this step"
                )
                .into());
            }
        }
        // The AI chosen: wired for working alone, and plugged in.
        if !agent.ready() {
            return Err(crate::trf!(
                "le travail autonome n'est pas encore possible avec {}",
                "autonomous work isn't possible yet with {}",
                agent.label()
            ));
        }
        if !self.ais.plugged().contains(&agent.ai()) {
            return Err(crate::trf!(
                "{} n'est pas branché (Réglages › IA)",
                "{} isn't plugged in (Settings › AI)",
                agent.label()
            ));
        }
        if let Some(why) = crate::space::refusal(agent.ai().key(), agent.label()) {
            return Err(why);
        }
        // Every figure below is that of the AI chosen, on its own subscription: never another
        // AI's (Claude Code's window full doesn't stop a Codex session, nor the reverse).
        let fig = self.agent_figures(agent);
        let global = self.ws.settings.orch.five_limit(agent);
        if let Some(f) = fig.five
            && f >= global
        {
            let left = fig
                .five_reset
                .and_then(|r| crate::usage::until(r, crate::model::now_ms()));
            let when = left
                .map(|l| {
                    crate::trf!(
                        " ; la fenêtre de 5 h repart à zéro dans {}",
                        "; the 5-hour window resets in {}",
                        crate::usage::left_text(l)
                    )
                })
                .unwrap_or_default();
            return Err(crate::trf!(
                "la limite de {} ({:.0} %) est déjà atteinte{when}",
                "{}'s limit ({:.0} %) is already reached{when}",
                agent.label(),
                global
            ));
        }
        // The ceiling is absolute (% of the 5-hour window), never past the global limit; one
        // already reached would stop the session at its first turn.
        let ceiling = ceiling.min(global);
        if let Some(f) = fig.five
            && f >= ceiling
        {
            return Err(crate::trf!(
                "le plafond choisi ({ceiling:.0} %) est déjà atteint : la fenêtre de 5 h est à {f:.0} %",
                "the ceiling chosen ({ceiling:.0} %) is already reached: the 5-hour window is at {f:.0} %"
            ));
        }
        // Its share from its start, for the record and as a second guard (the whole ceiling
        // while the window's figure is unknown).
        let limit = (ceiling - fig.five.unwrap_or(0.0)).max(1.0);
        if let Some(w) = fig.week
            && crate::usage::near_reserve(w, self.ws.settings.orch.week_limit_of(agent), 0.0)
        {
            return Err(crate::trf!(
                "la réserve minimum de {} ({:.0} %) est atteinte ou trop proche",
                "{}'s minimum reserve ({:.0} %) is reached or too near",
                agent.label(),
                self.ws.settings.orch.reserve_of(agent)
            ));
        }
        // The limits are checked with Claude Code's own figures: they come through the status line.
        // Not only in place: working (a status line left by an Orchestra since moved or
        // deleted measures nothing, and the session would stop as unmeasured).
        if agent == Agent::ClaudeCode && !crate::usage::tracking_works() {
            crate::usage::start_tracking(&std::env::current_exe().unwrap_or_default())?;
            self.ais.tracking = true;
        }
        // The session checks interfaces with take_screenshot, which macOS only allows once
        // Orchestra has the « Screen Recording » permission: ask for it now (once per launch).
        let mut granted_version = self.ws.settings.screen_granted_version.clone();
        crate::screen::ensure_permission(&mut granted_version);
        let steps_at_start: Vec<(String, bool)> = pr
            .steps
            .iter()
            .map(|s| (s.id.clone(), s.status == crate::project::StepStatus::Done))
            .collect();
        let steps_at_turn: Vec<String> = steps_at_start.iter().map(|(id, _)| id.clone()).collect();
        let code = crate::project::code_kind(&folder, pr.code);
        // Estimated before anything starts, from the step the AI will take first.
        let difficulty = crate::catalog::estimate_session(
            &aim,
            mission,
            fix,
            &crate::store::computer_id(),
            crate::model::now_ms(),
        )
        .with_saving(pr.launch.as_ref().map(|l| l.saving).unwrap_or_default(), fix);
        // A restart keeps the model first chosen. Else the one the person imposed for the
        // step taken first, or for the project. Else the cheapest able to do the step, among
        // the models of the AI chosen in the card, whose limits were just checked. None found
        // (Vibe without its own list): the AI's own default.
        let imposed = if model.trim().is_empty() {
            crate::catalog::imposed(
                &aim,
                agent,
                &crate::store::computer_id(),
                crate::model::now_ms(),
            )
        } else {
            None
        };
        let (picked, model_reason) = if let Some(i) = &imposed {
            (Some(i.model.clone()), i.reason.clone())
        } else if model.trim().is_empty() {
            match crate::catalog::choose(
                &[(agent, crate::catalog::choosable(agent))],
                difficulty.tier,
            ) {
                Some(c) => (Some(c.model.id), c.reason),
                None => (None, auto::default_model_reason().to_string()),
            }
        } else {
            (None, auto::restart_model_reason().to_string())
        };
        let model = picked.as_deref().unwrap_or(model);
        let copy = if fix {
            auto::make_copy(&folder, &format!("fix {title}"), false, code)?
        } else {
            auto::make_copy(&folder, &title, !on_main(pr), code)?
        };
        // A project without code makes no commit (the switch isn't shown for it), and without
        // git (worked on in its own folder) there is none to make.
        let commits = commits && code && !copy.branch.is_empty();
        let o = &self.ws.settings.orch;
        let cwd = std::path::Path::new(&copy.worktree);
        agent.write_config(cwd, commits, &o.extra_tools, model)?;
        // A model the person imposed wins over the economy slider: it picks neither the model
        // (above) nor a thinking level, which stays the AI's own.
        let saving = if imposed.is_some() {
            crate::model::Saving::Balanced
        } else {
            pr.launch.as_ref().map(|l| l.saving).unwrap_or_default()
        };
        // The space's own AI for this agent, if it provides one: its model, its gateway and
        // key (in the environment), its options after the program's name.
        let space_ai = crate::space::ai_run(agent.ai().key());
        let model = if space_ai.is_some() { "" } else { model };
        let mut cmd = agent.command_saving(cwd, commits, &o.extra_tools, model, saving);
        if let Some(run) = &space_ai
            && !run.args.is_empty()
            && let Some(rest) = cmd.strip_prefix("codex ")
        {
            let opts: Vec<String> = run
                .args
                .iter()
                .map(|a| crate::sessions::shell_quote(a))
                .collect();
            cmd = format!("codex {} {rest}", opts.join(" "));
        }
        // After the command's own MCP servers, which they complete.
        if let Some(run) = &space_ai {
            for a in &run.late_args {
                cmd.push(' ');
                cmd.push_str(&crate::sessions::shell_quote(a));
            }
        }
        let c2 = ctx.clone();
        let mut envs = match &space_ai {
            Some(run) => run.env.clone(),
            None => crate::sync::api_env(&self.ws.settings, agent),
        };
        // For an AI whose MCP servers can't be set per session (Antigravity): they inherit it.
        let space = store::space_id();
        if space != crate::space::LOCAL {
            envs.push(("ORCHESTRA_SPACE".into(), space));
        }
        let mut term = Term::spawn_once_env(
            &crate::trf!("Autonome · {title}", "Autonomous · {title}"),
            &copy.worktree,
            &cmd,
            &envs,
            move || c2.request_repaint_after(Duration::from_millis(200)),
        )?;
        term.resize(42, 140);
        let id = crate::model::new_id();
        // Kept in the record, to start again: on one step, only what completes its mission.
        let kept_mission = match step {
            None if mission.trim().is_empty() => default_mission().to_string(),
            _ => mission.trim().to_string(),
        };
        let mission = match step {
            Some(id) => auto::step_mission(id, &kept_mission),
            None => kept_mission.clone(),
        };
        let first_prompt = auto::mission_prompt(
            &self
                .ws
                .page(project)
                .map(|p| p.display_title())
                .unwrap_or_default(),
            project,
            &copy.branch,
            &mission,
            commits,
        ) + &endless
            .as_ref()
            .map(|c| auto::endless_rule(project, !c.when_done))
            .unwrap_or_default();
        let rec = crate::model::RunRecord {
            id: id.clone(),
            project: project.to_string(),
            title: title.clone(),
            tool: agent.label().into(),
            root: copy.root.clone(),
            worktree: copy.worktree.clone(),
            branch: copy.branch.clone(),
            started: crate::model::now_ms(),
            commits_allowed: commits,
            mission: kept_mission,
            step: step.unwrap_or_default().to_string(),
            limit,
            ceiling,
            endless: endless.clone(),
            model: auto::model_arg(model).unwrap_or_default().to_string(),
            difficulty: format!("{} · {}", difficulty.tier.label(), difficulty.reason),
            model_reason,
            ..Default::default()
        };
        // A new session replaces any planned restart of this project.
        let replaced = cancel_resumes(&mut self.ws.runs, project);
        self.ws.runs.push(rec);
        self.ws.settings.screen_granted_version = granted_version;
        self.dirty_since.get_or_insert_with(Instant::now);
        self.orch.runs.push(LiveRun {
            id,
            project: project.to_string(),
            title,
            agent,
            term,
            phase: Phase::Starting(Instant::now()),
            next: Some(first_prompt.clone()),
            first_prompt,
            steps_done: 0,
            paused_wall: 0,
            quota_reset: None,
            week_at_turn: None,
            week_turn: 0.0,
            reader: auto::Reader::default(),
            since: std::time::SystemTime::now() - Duration::from_secs(2),
            base: copy.base,
            root: copy.root,
            worktree: copy.worktree,
            limit,
            ceiling: Some(ceiling),
            pct: 0.0,
            tokens_counted: 0,
            turns: 0,
            max_turns: o.max_turns.max(1),
            stop: None,
            summarizing: false,
            last_line: crate::trf!("Démarrage de {}…", "Starting {}…", agent.label()),
            steps_at_start,
            screen_hash: 0,
            screen_changed: Instant::now(),
            denials: 0,
            nudged: false,
            started_wall: session_start(),
            measured: false,
            endless,
            done_streak: 0,
            steps_at_turn,
            waiting: None,
            paused_screen: 0,
            budget_micro: o.budget_micro(self.ws.settings.api_mode(agent.ai().key())),
        });
        // The step a replaced restart was waiting on is released.
        for left in replaced {
            if Some(left.step.as_str()) != step {
                self.end_run_step(&left, &StepIssue::Interrupted);
            }
        }
        // Now that the session exists: its step shows « doing », under the name its own MCP
        // server reserves with, which its update_step then renews.
        if let Some(id) = step
            && let Some(pr) = self.ws.page_mut(project).and_then(|p| p.project.as_mut())
        {
            pr.start_step(id, &holder_of(agent, &crate::store::computer_id()));
        }
        Ok(())
    }

    /// Once per frame: move every session along, follow the use of the subscription,
    /// write the day's report when it is time.
    /// Closing the window while autonomous sessions run only puts it in the Dock: Orchestra
    /// goes on driving them. Closing it again within a minute quits for real.
    pub(super) fn keep_running_on_close(&mut self, ctx: &egui::Context) {
        let asked = ctx.input(|i| i.viewport().close_requested());
        // A copy being merged or discarded: quitting would cut git short and could leave the
        // project's folder half merged. Orchestra closes once it is done; asking again quits.
        if asked && !self.orch.copy_jobs.is_empty() && !self.orch.quit_after_jobs {
            self.orch.quit_after_jobs = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.info(crate::tr!(
                "Une copie est en cours de fusion : Orchestra se fermera dès qu'elle sera finie. Fermez à nouveau pour quitter tout de suite.",
                "A copy is being merged: Orchestra will close as soon as it is done. Close again to quit right away."
            ));
            return;
        }
        if self.orch.quit_after_jobs && self.orch.copy_jobs.is_empty() {
            self.orch.quit_after_jobs = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !asked || self.orch.runs.is_empty() {
            return;
        }
        if self
            .orch
            .close_held
            .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
        {
            return;
        }
        self.orch.close_held = Some(Instant::now());
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        let n = self.orch.runs.len();
        let s = crate::i18n::plural(n);
        self.info(crate::trf!(
            "{n} session{s} autonome{s} en cours : Orchestra continue dans le Dock et garde le Mac éveillé. Fermez à nouveau pour quitter et les arrêter.",
            "{n} autonomous session{s} running: Orchestra keeps going in the Dock and keeps the Mac awake. Close again to quit and stop them."
        ));
    }

    /// `window`: called from a frame of the window; else from the conductor, while the
    /// window isn't drawn (its input is then from the last frame: closing isn't read).
    pub(in crate::app) fn orch_tick(&mut self, ctx: &egui::Context, window: bool) {
        if !self.orch.started {
            self.orch.started = true;
            // Sessions that were running when Orchestra closed.
            let mut cut = vec![];
            for r in self.ws.runs.iter_mut().filter(|r| r.outcome.is_empty()) {
                r.outcome = crate::tr!(
                    "interrompue (Orchestra fermé)",
                    "interrupted (Orchestra closed)"
                )
                .into();
                r.ended = r.ended.max(r.started);
                cut.push(r.clone());
            }
            for r in cut {
                self.end_run_step(&r, &StepIssue::Interrupted);
            }
            // The history only grows: the oldest sessions nothing needs any more go.
            if auto::trim_runs(&mut self.ws.runs, auto::KEEP_RUNS, |w| {
                std::path::Path::new(w).exists()
            }) > 0
            {
                self.dirty_since.get_or_insert_with(Instant::now);
            }
            // Copies no session needs any more (merged, discarded, or forgotten): removed
            // if they hold nothing uncommitted, their branches kept.
            auto::clean_copies_later(auto::stale_copies(&self.ws.runs));
        }
        self.usage_tick();
        self.copy_jobs_tick();
        self.resume_tick(ctx);
        self.orch.awake.set(!self.orch.runs.is_empty());
        if window {
            self.keep_running_on_close(ctx);
        }
        // Developer aid: ORCHESTRA_AUTO_RUN=<project id> starts a session once Claude Code is
        // seen, once per launch of Orchestra.
        static AUTO_RUN_DONE: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if let Ok(pid) = std::env::var("ORCHESTRA_AUTO_RUN")
            && !pid.is_empty()
            && self.orch.runs.is_empty()
            && self.ais.plugged().contains(&crate::setup::Ai::ClaudeCode)
            && !AUTO_RUN_DONE.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let o = &self.ws.settings.orch;
            let l = ceiling_choice(None, self.orch.five, o.default_limit, o.five_limit(Agent::ClaudeCode));
            if let Err(e) = self.launch_run(ctx, &pid, "", l, true, None, Agent::ClaudeCode, false, "", None) {
                eprintln!("auto-run : {e}");
            }
        }
        self.cve_watch_tick(ctx);
        self.cve_fix_tick(ctx);
        let now = Instant::now();
        let limits = self.ws.settings.orch.clone();
        let pause = self.ws.settings.orch.pause_for_answer;
        let mut paused: Vec<(String, String, Wait)> = vec![];
        let mut finished: Vec<(usize, String, String)> = vec![];
        let figures = self.figures_by_agent();
        for (i, r) in self.orch.runs.iter_mut().enumerate() {
            // The limits of this session's own AI, on its own subscription.
            let global = limits.five_limit(r.agent);
            let week_limit = limits.week_limit_of(r.agent);
            // The subscription of this session's own agent; none read (an agent that tells
            // nothing): no figures, and the session stops as unmeasured.
            let fig = figures.get(&r.agent).copied().unwrap_or_default();
            // How the screen moves (idle = nothing drawn for a while).
            let screen = r.term.text();
            let h = hash(&screen);
            if h != r.screen_hash {
                r.screen_hash = h;
                r.screen_changed = now;
            }
            let idle = r.screen_changed.elapsed();
            if r.term.exited() && !matches!(r.phase, Phase::Closing(_)) {
                finished.push((
                    i,
                    r.stop.clone().unwrap_or_else(|| {
                        crate::trf!("{} s'est arrêté", "{} stopped", r.agent.label())
                    }),
                    String::new(),
                ));
                continue;
            }
            r.reader
                .poll(r.agent, std::path::Path::new(&r.worktree), r.since);
            match r.phase {
                Phase::Starting(t0) => {
                    if t0.elapsed() > Duration::from_secs(90) {
                        finished.push((
                            i,
                            crate::tr!("Claude Code n'a pas démarré", "Claude Code didn't start")
                                .into(),
                            String::new(),
                        ));
                    } else if t0.elapsed() > Duration::from_secs(3)
                        && idle > Duration::from_millis(2500)
                    {
                        // A headless agent's loop asks nothing: its screen is never a question.
                        let seen = if r.agent.headless() {
                            Screen::Other
                        } else {
                            auto::read_screen(&screen)
                        };
                        match seen {
                            Screen::Trust if auto::folder_trusted(&r.root) => {
                                // The person already trusts this repository in Claude Code;
                                // the copy is the same repository, in a new folder.
                                r.term.write(b"\r");
                                r.screen_changed = now;
                            }
                            Screen::Trust if pause => {
                                // The person decides, in the terminal.
                                r.pause(Wait::Trust);
                                paused.push((r.project.clone(), r.title.clone(), Wait::Trust));
                            }
                            Screen::Trust => {
                                // Never accept for them: it would turn on the repository's
                                // hooks, MCP servers and settings.
                                r.term.write(b"\x1b");
                                finished.push((i, crate::trf!(
                                    "Claude Code demande si le dossier est sûr ; ouvrez-le une fois dans « {} » et acceptez, puis relancez",
                                    "Claude Code asks whether the folder is safe; open it once in « {} » and accept, then start again",
                                    r.root
                                ), String::new()));
                            }
                            _ => {
                                if let Some(text) = r.next.take() {
                                    r.send(&text);
                                    r.phase = Phase::Typed(now);
                                }
                            }
                        }
                    }
                }
                Phase::Typed(t) => {
                    if t.elapsed() > Duration::from_millis(500) {
                        if !r.agent.headless() {
                            r.term.write(b"\r");
                        }
                        r.reader.turn_over = false;
                        r.nudged = false;
                        r.week_at_turn = fig.week;
                        r.phase = if r.summarizing {
                            Phase::Summary(now)
                        } else {
                            Phase::Working(now)
                        };
                        r.last_line = if r.summarizing {
                            crate::tr!("Rédige son compte rendu…", "Writing its summary…").into()
                        } else {
                            turn_running(r.turns)
                        };
                    }
                }
                Phase::Denied(t) => {
                    if t.elapsed() > Duration::from_millis(1500) {
                        r.send(auto::denied_prompt());
                        r.phase = Phase::Typed(now);
                    }
                }
                Phase::Working(t) | Phase::Summary(t) => {
                    let summary = matches!(r.phase, Phase::Summary(_));
                    // While Claude Code works, its spinner redraws the screen every second: a
                    // screen still for minutes, with nothing written to the session either, is
                    // a question we didn't recognise. Answer it like a permission.
                    // Claude Code's own limit is reached: no turn goes through until it resets.
                    // Said in the session file, or only on the screen (still for a while, so
                    // the AI isn't at work, and the message in its last lines).
                    let on_screen = (idle > Duration::from_secs(30) && !r.reader.turn_over)
                        .then(|| last_lines(&screen, 12))
                        .filter(|tail| auto::is_limit_message(tail));
                    if let Some(msg) = r.reader.limit_hit.clone().or(on_screen) {
                        let now_ms = crate::model::wall_ms();
                        let reset = auto::limit_reset(&msg, now_ms)
                            .or(if fig.week.is_some_and(|w| w >= 99.0) {
                                fig.week_reset
                            } else {
                                fig.five_reset
                            })
                            .filter(|&t| t > now_ms)
                            // Unknown: try again in an hour; still limited, it stops again.
                            .unwrap_or(now_ms + 3_600_000);
                        r.quota_reset = Some(reset);
                        let why = Stop::SubscriptionLimit.text();
                        r.stop = Some(why.clone());
                        // No summary: Claude Code would refuse it too.
                        finished.push((i, why, String::new()));
                        continue;
                    }
                    let frozen = idle > Duration::from_secs(180)
                        && !r.reader.turn_over
                        && r.reader
                            .last_growth
                            .is_none_or(|g| g.elapsed() > Duration::from_secs(180));
                    // A headless agent refuses by itself what needs an approval, and its screen
                    // stays still while it works (its answer is shown at the end of the run).
                    if !r.agent.headless()
                        && ((auto::read_screen(&screen) == Screen::Permission
                            && idle > Duration::from_millis(800))
                            || frozen)
                    {
                        if pause {
                            r.pause(Wait::Permission);
                            paused.push((r.project.clone(), r.title.clone(), Wait::Permission));
                            continue;
                        }
                        r.term.write(b"\x1b");
                        r.denials += 1;
                        r.last_line = crate::tr!(
                            "Commande refusée (non autorisée en autonome)",
                            "Command refused (not allowed when autonomous)"
                        )
                        .into();
                        r.phase = Phase::Denied(now);
                        continue;
                    }
                    // The prompt didn't go: press Enter once more.
                    if !r.nudged
                        && !r.agent.headless()
                        && t.elapsed() > Duration::from_secs(60)
                        && r.reader
                            .last_growth
                            .is_none_or(|g| g.elapsed() > t.elapsed())
                    {
                        r.term.write(b"\r");
                        r.nudged = true;
                    }
                    let quiet = r
                        .reader
                        .last_growth
                        .is_some_and(|g| g.elapsed() > Duration::from_secs(2));
                    let over = r.reader.turn_over && quiet && idle > Duration::from_millis(1500);
                    // A headless run cuts itself at 25 min (`--time-limit`): Orchestra steps in
                    // only past that, with Ctrl-C, should it not have stopped.
                    let headless = r.agent.headless();
                    let max_turn = if headless { 30 * 60 } else { 25 * 60 };
                    let too_long =
                        t.elapsed() > Duration::from_secs(if summary { 300 } else { max_turn });
                    if over || too_long {
                        if too_long {
                            r.term.write(if headless { b"\x03" } else { b"\x1b" });
                        }
                        // The headless run failed without an answer (not signed in, Vibe
                        // missing or broken…): the next prompts would fail the same way.
                        if over
                            && headless
                            && r.reader.last_text.trim().is_empty()
                            && let Some(code) = r.reader.exit_code.filter(|c| *c != 0 && *c != 3)
                        {
                            let why = crate::trf!(
                                "{} s'est arrêté sans réponse (code {code}) ; voir son terminal",
                                "{} stopped without an answer (code {code}); see its terminal",
                                r.agent.label()
                            );
                            r.stop = Some(why.clone());
                            finished.push((i, why, String::new()));
                            continue;
                        }
                        if summary {
                            let text = if over {
                                r.reader.last_text.clone()
                            } else {
                                String::new()
                            };
                            let why = r
                                .stop
                                .clone()
                                .unwrap_or_else(|| crate::tr!("terminée", "finished").into());
                            finished.push((i, why, text));
                            continue;
                        }
                        r.turns += 1;
                        // What this turn took from the week (a reset in between counts for nothing).
                        if let (Some(a), Some(b)) = (r.week_at_turn, fig.week)
                            && b >= a
                        {
                            r.week_turn = r.week_turn.max(b - a);
                        }
                        auto::push_copy_later(r.worktree.clone());
                        let mut sig = auto::signal(&r.reader.last_text);
                        r.last_line = crate::richtext::one_line(
                            &auto::strip_markers(&r.reader.last_text),
                            140,
                        );
                        // Steps the person added during the turn: the AI hasn't seen them, so
                        // its « nothing left » doesn't stop the session; it is sent on to them.
                        let steps_now = self
                            .ws
                            .page(&r.project)
                            .and_then(|p| p.project.as_ref())
                            .map(|pr| pr.steps.clone())
                            .unwrap_or_default();
                        let fresh = fresh_steps(&r.steps_at_turn, &steps_now);
                        if sig == Signal::Done && !fresh.is_empty() {
                            sig = Signal::Continue;
                        }
                        r.steps_at_turn = steps_now.iter().map(|s| s.id.clone()).collect();
                        // A step finished during this turn: the next starts a new conversation.
                        let done_now = steps_since(&r.steps_at_start, &steps_now).done.len() as u32;
                        let step_finished = done_now > r.steps_done;
                        r.steps_done = done_now;
                        let five = fig.five;
                        // Started before the window's figure was known: its ceiling now.
                        if r.ceiling.is_none() {
                            r.ceiling = ceiling_from(five, r.pct, r.limit, global);
                        }
                        r.done_streak = if sig == Signal::Done {
                            r.done_streak + 1
                        } else {
                            0
                        };
                        // Endless mode: its own end conditions replace [[FINI]] and the turn limit.
                        let endless_end = r.endless.as_ref().and_then(|c| {
                            let steps = self
                                .ws
                                .page(&r.project)
                                .and_then(|p| p.project.as_ref())
                                .map(|pr| pr.steps.clone())
                                .unwrap_or_default();
                            let p = steps_since(&r.steps_at_start, &steps);
                            let steps_done = p.done.len() as u32;
                            // Steps the AI added itself and hasn't done yet.
                            let added_open = p.added_open;
                            if auto::too_many_self_steps(added_open) {
                                return Some(crate::trf!(
                                    "l'IA a ajouté {added_open} étapes sans les faire",
                                    "the AI added {added_open} steps without doing them"
                                ));
                            }
                            c.reached(
                                crate::model::wall_ms().saturating_sub(r.started_wall),
                                steps_done,
                                r.turns,
                                r.done_streak,
                            )
                        });
                        let stop = stop_reason(&TurnEnd {
                            asked: r.stop.clone(),
                            too_long,
                            endless_end,
                            signal: sig,
                            blocked_why: if sig == Signal::Blocked {
                                auto::block_reason(&r.reader.last_text)
                            } else {
                                String::new()
                            },
                            endless: r.endless.is_some(),
                            pct: r.pct,
                            limit: r.limit,
                            ceiling: r.ceiling,
                            five,
                            global,
                            week: fig.week,
                            week_limit,
                            week_turn: r.week_turn,
                            // Its dollars are a measure of their own in API mode.
                            measured: r.measured || r.budget_micro.is_some(),
                            turns: r.turns,
                            max_turns: r.max_turns,
                            spent_micro: r.reader.cost_micro,
                            budget_micro: r.budget_micro,
                        });
                        // Stopped by a quota: when it resets, to start again then.
                        match stop.as_ref().map(Stop::quota) {
                            Some(Quota::FiveHours) => r.quota_reset = fig.five_reset,
                            Some(Quota::Week) => r.quota_reset = fig.week_reset,
                            _ => {}
                        }
                        let reason = stop.map(|s| s.text());
                        // A question to the person still unanswered: wait for it rather
                        // than send the AI on.
                        let asked = pause
                            && reason.is_none()
                            && crate::model::question_pending(
                                &self.ws.inbox,
                                &r.project,
                                r.started_wall,
                            );
                        if asked {
                            r.pause(Wait::Question);
                            paused.push((r.project.clone(), r.title.clone(), Wait::Question));
                            continue;
                        }
                        match reason {
                            Some(why) => {
                                r.stop = Some(why.clone());
                                r.summarizing = true;
                                r.send(&auto::summary_prompt(&r.project, &why));
                            }
                            None if step_finished => {
                                r.new_conversation(
                                    auto::fresh_conversation_note(false),
                                    &new_steps_note(&fresh),
                                    now,
                                );
                                continue;
                            }
                            None => {
                                let next = if r.endless.is_some() {
                                    auto::endless_prompt(
                                        r.endless.as_ref().is_some_and(|c| !c.when_done),
                                    )
                                } else {
                                    auto::continue_prompt()
                                };
                                r.send(&format!("{}{next}", new_steps_note(&fresh)));
                            }
                        }
                        r.phase = Phase::Typed(now);
                    }
                }
                // Paused: nothing moves until the person answers, then the AI goes on.
                Phase::Waiting => {
                    let left =
                        crate::model::question_pending(&self.ws.inbox, &r.project, r.started_wall);
                    if r.answered(&screen, idle, left) {
                        r.resume();
                    }
                }
                Phase::Closing(t) => {
                    if t.elapsed() > Duration::from_secs(3) || r.term.exited() {
                        finished.push((i, String::new(), String::new()));
                    }
                }
            }
        }
        for (project, title, why) in paused {
            self.ws.notify(
                "info",
                &project,
                &waiting_title(&title),
                why.label(),
                None,
            );
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        // Finished sessions: report, inbox, then close the terminal.
        for (i, why, summary) in finished.into_iter().rev() {
            if matches!(self.orch.runs[i].phase, Phase::Closing(_)) {
                self.orch.runs.remove(i);
                continue;
            }
            self.finish_run(i, &why, &summary);
        }
        self.merge_to_main(ctx);
        auto::set_busy(self.orch.runs.iter().map(|r| r.worktree.clone()).collect());
        if !self.orch.runs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        self.daily_tick();
    }

    /// Sessions stopped by a quota start again by themselves once it has reset.
    pub(super) fn resume_tick(&mut self, ctx: &egui::Context) {
        if self
            .orch
            .resume_checked
            .is_some_and(|t| t.elapsed() < Duration::from_secs(20))
        {
            return;
        }
        self.orch.resume_checked = Some(Instant::now());
        let now = crate::model::wall_ms();
        if !self.ws.settings.orch.auto_resume {
            return;
        }
        let running: Vec<&str> = self.orch.runs.iter().map(|r| r.project.as_str()).collect();
        let due = crate::model::due_resumes(&self.ws.runs, now, &running);
        for i in due {
            let rec = self.ws.runs[i].clone();
            // Once only: a failure doesn't try again every 20 seconds.
            for x in self.ws.runs.iter_mut().filter(|x| x.project == rec.project) {
                x.resume_at = 0;
            }
            self.dirty_since.get_or_insert_with(Instant::now);
            // The same ceiling as before (the window has reset since); older records without
            // one: the same share, from the window's figure now.
            // The same AI as before (named by its label in the record).
            let agent = agent_of(&rec);
            let five = self.agent_figures(agent).five;
            let o = &self.ws.settings.orch;
            let ceiling = if rec.ceiling > 0.0 {
                rec.ceiling
            } else {
                let share = if rec.limit > 0.0 { rec.limit } else { o.default_limit };
                ceiling_choice(None, five, share, o.five_limit(agent))
            };
            let step = Some(rec.step.as_str()).filter(|s| !s.is_empty());
            match self.launch_run(ctx, &rec.project, &rec.mission, ceiling, rec.commits_allowed, rec.endless.clone(), agent, rec.branch.contains("/auto-fix-"), &rec.model, step) {
                Ok(()) => self.ws.notify(
                    "info",
                    &rec.project,
                    &crate::trf!("Reprise automatique sur {}", "Automatic restart on {}", rec.title),
                    crate::tr!("Le quota est revenu : la session autonome repart avec la même mission.", "The quota is back: the autonomous session starts again with the same mission."),
                    None,
                ),
                Err(e) => {
                    self.end_run_step(&rec, &StepIssue::Interrupted);
                    self.ws.notify(
                        "erreur",
                        &rec.project,
                        &crate::trf!("Reprise automatique impossible sur {}", "Couldn't restart {} automatically", rec.title),
                        &e,
                        None,
                    );
                }
            }
        }
    }

    /// Follow Claude Code's figures; share each rise of the 5-hour window between the
    /// running sessions by the tokens each used since the last reading.
    pub(super) fn usage_tick(&mut self) {
        if self
            .orch
            .usage_polled
            .is_some_and(|t| t.elapsed() < Duration::from_secs(3))
        {
            return;
        }
        self.orch.usage_polled = Some(Instant::now());
        self.other_agents_tick();
        // A window that has reset is dropped by read(): its old figure is forgotten too, so a
        // limit reached before the reset doesn't block what follows. A reading that fails
        // keeps the last figures until their reset (never taken for 0 %); tried again in 3 s.
        let u = crate::usage::read();
        let now = crate::model::now_ms();
        let last = Figures {
            five: self.orch.five,
            week: self.orch.week,
            five_reset: self.orch.five_reset,
            week_reset: self.orch.week_reset,
        };
        let f = last.merged(u.as_ref().map(Figures::of), now);
        (
            self.orch.five,
            self.orch.week,
            self.orch.five_reset,
            self.orch.week_reset,
        ) = (f.five, f.week, f.five_reset, f.week_reset);
        if u.as_ref().is_some_and(|u| u.five().is_some()) {
            self.orch.unmeasured_since = None;
        } else {
            self.orch.unmeasured_since.get_or_insert(now);
        }
        let Some(u) = u else { return };
        if self.orch.history_at != u.at {
            self.orch.history_at = u.at;
            let h = crate::usage::record(&u);
            self.orch.day_total = crate::usage::day_total(&h, now);
            self.orch.day_week = crate::usage::day_week_used(&h, now);
        }
        // Only a fresh figure is shared between the sessions.
        let Some(five) = u.five().cloned() else { return };
        apply_reading(
            &mut self.orch.runs,
            Agent::ClaudeCode,
            &mut self.orch.last_reading,
            u.at,
            &five,
        );
    }

    /// The same for the other agents that have sessions running and tell their figures
    /// (Codex, Vibe's monthly budget); those that tell none (Gemini) stay unmeasured, so their sessions stop
    /// (`Stop::Unmeasured`) rather than run with no ceiling.
    pub(super) fn other_agents_tick(&mut self) {
        let mut agents: Vec<Agent> = vec![];
        for a in self.orch.runs.iter().map(|r| r.agent) {
            if a != Agent::ClaudeCode && crate::usage::reports_usage(a) && !agents.contains(&a) {
                agents.push(a);
            }
        }
        self.orch.others.retain(|a, _| agents.contains(a));
        let now = crate::model::now_ms();
        for a in agents {
            // As for Claude Code: a failed reading keeps the last figures until their reset.
            let u = crate::usage::read_for(a);
            let last = self.orch.others.get(&a).copied().unwrap_or_default();
            let f = last.merged(u.as_ref().map(Figures::of), now);
            if f == Figures::default() {
                self.orch.others.remove(&a);
            } else {
                self.orch.others.insert(a, f);
            }
            let Some(u) = u else { continue };
            // Its 5-hour window, or the month of its budget (Mistral Vibe).
            if let Some(five) = u.measured() {
                let mut last = self.orch.others_reading.get(&a).copied();
                apply_reading(&mut self.orch.runs, a, &mut last, u.at, five);
                if let Some(l) = last {
                    self.orch.others_reading.insert(a, l);
                }
            }
        }
    }

    /// The figures of one agent's subscription: Claude Code's as followed, another's as last
    /// read for its running sessions, else read now; none for an agent that tells none.
    pub(super) fn agent_figures(&self, agent: Agent) -> Figures {
        if let Some(f) = self.figures_by_agent().get(&agent) {
            return *f;
        }
        crate::usage::read_for(agent)
            .map(|u| Figures::of(&u))
            .unwrap_or_default()
    }

    /// The figures each session is measured and stopped on: those of its own agent.
    pub(super) fn figures_by_agent(&self) -> HashMap<Agent, Figures> {
        let mut m = self.orch.others.clone();
        m.insert(
            Agent::ClaudeCode,
            Figures {
                five: self.orch.five,
                week: self.orch.week,
                five_reset: self.orch.five_reset,
                week_reset: self.orch.week_reset,
            },
        );
        m
    }

    /// Merge into main the finished sessions of projects that allow it, once closed, in
    /// the background; one waits while another copy of its project is being dealt with.
    pub(super) fn merge_to_main(&mut self, ctx: &egui::Context) {
        let ready: Vec<String> = self
            .orch
            .to_main
            .iter()
            .filter(|id| !self.orch.runs.iter().any(|r| &r.id == *id))
            .cloned()
            .collect();
        for id in ready {
            let Some(rec) = self.ws.runs.iter().find(|x| x.id == id).cloned() else {
                self.orch.to_main.retain(|x| *x != id);
                continue;
            };
            let op = if rec.branch == auto::DEV {
                CopyOp::Park
            } else {
                CopyOp::ToMain
            };
            if self.start_copy_job(ctx, rec, op) {
                self.orch.to_main.retain(|x| *x != id);
            }
        }
    }

    /// Start merging or discarding a session's copy on its own thread; false when another
    /// copy of the same repository is being dealt with (git would find it locked).
    pub(super) fn start_copy_job(
        &mut self,
        ctx: &egui::Context,
        rec: crate::model::RunRecord,
        op: CopyOp,
    ) -> bool {
        if self
            .orch
            .copy_jobs
            .iter()
            .any(|j| j.id == rec.id || j.root == rec.root)
        {
            return false;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let review_slot = std::sync::Arc::new(std::sync::Mutex::new(None));
        self.orch.copy_jobs.push(CopyJob {
            id: rec.id.clone(),
            merge: op != CopyOp::Discard,
            park: op == CopyOp::Park,
            title: rec.title.clone(),
            root: rec.root.clone(),
            done: rx,
            review: review_slot.clone(),
        });
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let res = match op {
                CopyOp::Merge => auto::merge(&rec),
                CopyOp::ToMain => match crate::review::review_run(&rec) {
                    // Another AI found a defect: no automatic merge, the person decides.
                    Ok(rv) if rv.verdict == crate::review::Verdict::Blocking => {
                        let msg = crate::trf!(
                            "relecture par {} : défaut bloquant, fusion laissée à faire à la main. {}",
                            "review by {}: blocking defect, merge left to do by hand. {}",
                            rv.reviewer,
                            rv.text
                        );
                        *review_slot.lock().unwrap() = Some(rv);
                        Err(msg)
                    }
                    // Approved, remarks: the merge goes on.
                    Ok(rv) => {
                        *review_slot.lock().unwrap() = Some(rv);
                        auto::merge_into_main(&rec)
                    }
                    // No reviewer available: the merge goes on.
                    Err(_) => auto::merge_into_main(&rec),
                },
                CopyOp::Park => auto::park(&rec),
                CopyOp::Discard => auto::discard(&rec),
            };
            let _ = tx.send(res);
            ctx.request_repaint();
        });
        true
    }

    pub(super) fn finish_run(&mut self, i: usize, why: &str, summary: &str) {
        let r = &mut self.orch.runs[i];
        let why = if why.is_empty() {
            r.stop
                .clone()
                .unwrap_or_else(|| crate::tr!("terminée", "finished").into())
        } else {
            why.to_string()
        };
        if r.agent.headless() {
            // Its loop reads no keyboard: Ctrl-C ends it (and a run still going).
            r.term.write(b"\x03");
        } else {
            r.term.paste("/exit");
            r.term.write(b"\r");
        }
        r.phase = Phase::Closing(Instant::now());
        let (id, project, base, pct, tokens, turns) = (
            r.id.clone(),
            r.project.clone(),
            r.base.clone(),
            r.pct,
            r.reader.tokens,
            r.turns,
        );
        // Stopped by a quota: start again by itself once it resets (if that is allowed).
        let now = crate::model::wall_ms();
        let resume_at = r
            .quota_reset
            .filter(|&reset| self.ws.settings.orch.auto_resume && reset > now)
            .map(crate::model::resume_time)
            .unwrap_or(0);
        let before = r.steps_at_start.clone();
        let asked = crate::model::question_pending(&self.ws.inbox, &project, r.started_wall);
        // A session on one step: the step follows how it ended, before the report lists
        // what was finished.
        if let Some(rec) = self.ws.runs.iter().find(|x| x.id == id).cloned()
            && let Some(issue) = step_issue(&why, resume_at > 0, asked)
        {
            self.end_run_step(&rec, &issue);
        }
        let newly: Vec<String> = self
            .ws
            .page(&project)
            .and_then(|p| p.project.as_ref())
            .map(|pr| steps_since(&before, &pr.steps).done)
            .unwrap_or_default();
        let Some(rec) = self.ws.runs.iter_mut().find(|x| x.id == id) else {
            return;
        };
        rec.ended = crate::model::now_ms();
        rec.outcome = why.clone();
        rec.pct = pct;
        rec.tokens = tokens;
        rec.turns = turns;
        rec.steps_done = newly;
        rec.resume_at = resume_at;
        // No copy (a project without git): nothing to merge nor discard, the work is there.
        if auto::in_place(rec) {
            // Where the folder was backed up before the session (`base` for a session in place).
            rec.closed = if base.is_empty() {
                auto::in_place_note().into()
            } else {
                crate::trf!(
                    "{} ; sauvegarde d'avant la session : {}",
                    "{}; backup from before the session: {}",
                    auto::in_place_note(),
                    super::projects::tilde(&base)
                )
            };
        }
        let rec = rec.clone();
        let (commits, dirty) = auto::work_done(&rec, &base);
        auto::push_copy_later(rec.worktree.clone());
        let md = auto::session_report(&rec, &commits, dirty, summary);
        let day = auto::local_day(rec.ended);
        let page = self.report_page(
            Some(&rec.project),
            &day,
            &format!("{} · {}", rec.title, auto::hm(rec.started)),
            &md,
        );
        if let Some(x) = self.ws.runs.iter_mut().find(|x| x.id == id) {
            x.report = Some(page.clone());
        }
        let (end_title, text) = end_message(&SessionEnd {
            project: &rec.title,
            why: &why,
            steps: rec.steps_done.len(),
            saved: commits.len(),
            unsaved: dirty,
            pct: rec.pct,
            resume_in: (resume_at > 0)
                .then(|| crate::usage::left_text(resume_at.saturating_sub(now))),
        });
        // Commits on main allowed for this project: merge once Claude Code has closed.
        let to_main = self
            .ws
            .page(&project)
            .and_then(|p| p.project.as_ref())
            .is_some_and(on_main);
        // On the shared dev branch: the work stays there and the copy goes, so dev can be
        // opened from the Versions card (unless the session starts again by itself).
        let on_dev = rec.branch == auto::DEV && resume_at == 0;
        if on_dev || (to_main && (!commits.is_empty() || dirty > 0)) {
            self.orch.to_main.push(id.clone());
        }
        self.ws.notify(
            "fin",
            &project,
            &end_title,
            &text,
            Some(page),
        );
        self.detect_changes();
        self.ack(crate::trf!(
            "Session autonome terminée sur {} : {why}.",
            "Autonomous session finished on {}: {why}.",
            rec.title
        ));
    }

    /// The step the session `rec` was sent on, if any, once `rec` is over.
    pub(super) fn end_run_step(&mut self, rec: &crate::model::RunRecord, issue: &StepIssue) {
        if rec.step.is_empty() {
            return;
        }
        let holder = holder_of(agent_of(rec), &crate::store::computer_id());
        if let Some(pr) = self.ws.page_mut(&rec.project).and_then(|p| p.project.as_mut())
            && pr.end_step(&rec.step, &holder, issue)
        {
            self.dirty_since.get_or_insert_with(Instant::now);
        }
    }

    /// A page under "Rapports des IA › <project> › <day>" (`project` None: the daily
    /// summaries' folder): each project's reports on their own.
    pub(super) fn report_page(&mut self, project: Option<&str>, day: &str, title: &str, md: &str) -> String {
        let day_page = self.ws.report_day(project, day);
        let mut page = crate::store::page_from_markdown(md, title);
        page.title = title.to_string();
        page.icon = "📋".into();
        page.parent = Some(day_page);
        let id = page.id.clone();
        self.ws.pages.push(page);
        id
    }

    /// The day's overall report, at the chosen hour, as soon as a session ended that day
    /// (and for yesterday, if Orchestra was closed at that hour). Sessions still running
    /// don't hold it back: with sessions that never stop, it never came.
    pub(super) fn daily_tick(&mut self) {
        if self
            .orch
            .daily_checked
            .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
        {
            return;
        }
        self.orch.daily_checked = Some(Instant::now());
        let o = &self.ws.settings.orch;
        let due = auto::daily_due(crate::model::now_ms(), o.report_hour, &o.last_daily, &self.ws.runs);
        for (day, runs) in due {
            let mut projects: Vec<&str> = runs.iter().map(|r| r.project.as_str()).collect();
            projects.sort();
            projects.dedup();
            let refs: Vec<&crate::model::RunRecord> = runs.iter().collect();
            let md = auto::daily_report(&day, &refs);
            let page = self.report_page(
                None,
                &day,
                crate::tr!("Bilan de la journée", "Daily summary"),
                &md,
            );
            let (p, n) = (projects.len(), runs.len());
            let text = crate::trf!(
                "{p} projet{}, {n} session{}",
                "{p} project{}, {n} session{}",
                crate::i18n::plural(p),
                crate::i18n::plural(n)
            );
            self.ws.notify(
                "rapport",
                "",
                &crate::trf!("Bilan de la journée · {day}", "Daily summary · {day}"),
                &text,
                Some(page),
            );
            self.ws.settings.orch.last_daily = day;
            self.detect_changes();
        }
    }

    // ------------------------------------------------------------------ the page

    /// Stop the autonomous sessions' programs (and everything they started), before quitting
    /// or replacing the app.
    /// Autonomous sessions still working, and copies being merged or discarded.
    pub(in crate::app) fn runs_working(&self) -> usize {
        self.orch.runs.iter().filter(|r| !r.term.exited()).count() + self.orch.copy_jobs.len()
    }

    pub(in crate::app) fn stop_runs(&mut self) {
        for r in self.orch.runs.iter_mut() {
            r.term.stop();
        }
        // A copy being merged or discarded: let git finish (cut short, it could leave the
        // project's folder half merged), within a minute, and record how it went.
        let until = Instant::now() + Duration::from_secs(60);
        while !self.orch.copy_jobs.is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
            self.copy_jobs_tick();
        }
    }

    /// What the links under a session's copy show: being dealt with, or the second click
    /// awaited.
    pub(super) fn copy_states(&self) -> Vec<(String, &'static str)> {
        let mut v: Vec<(String, &'static str)> = self
            .orch
            .copy_jobs
            .iter()
            .map(|j| (j.id.clone(), if j.merge { "merging" } else { "discarding" }))
            .collect();
        v.extend(self.orch.confirm.clone());
        v
    }

    /// The merges and discards finished in the background: their outcome recorded.
    pub(super) fn copy_jobs_tick(&mut self) {
        let mut finished = vec![];
        let mut reviews = vec![];
        self.orch.copy_jobs.retain(|j| match j.done.try_recv() {
            Ok(res) => {
                if let Some(rv) = j.review.lock().ok().and_then(|mut g| g.take()) {
                    reviews.push((j.id.clone(), rv));
                }
                finished.push((
                    j.id.clone(),
                    if j.park { 2 } else { j.merge as u8 },
                    j.title.clone(),
                    res,
                ));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                finished.push((
                    j.id.clone(),
                    if j.park { 2 } else { j.merge as u8 },
                    j.title.clone(),
                    Err(crate::tr!("interrompu", "interrupted").into()),
                ));
                false
            }
        });
        // The reviewer's opinion goes at the end of the session's report.
        for (id, rv) in reviews {
            let page = self
                .ws
                .runs
                .iter()
                .find(|x| x.id == id)
                .and_then(|x| x.report.clone());
            if let Some(page) = page {
                let extra = crate::store::page_from_markdown(
                    &crate::review::report_section(&rv),
                    "",
                );
                if let Some(p) = self.ws.pages.iter_mut().find(|p| p.id == page) {
                    p.blocks.extend(extra.blocks);
                }
            }
        }
        for (id, merge, title, res) in finished {
            match res {
                Ok(m) => {
                    if let Some(x) = self.ws.runs.iter_mut().find(|x| x.id == id) {
                        x.closed = match merge {
                            2 => crate::tr!("gardé sur dev", "kept on dev").into(),
                            1 => crate::tr!("fusionné", "merged").into(),
                            _ => crate::tr!("abandonné", "discarded").into(),
                        };
                    }
                    self.dirty_since.get_or_insert_with(Instant::now);
                    self.detect_changes();
                    self.info(format!("{title} : {m}"));
                }
                Err(e) => self.fail(format!("{title} : {e}")),
            }
        }
    }

    /// A click on a finished session's copy: ask first, then merge or discard it on its
    /// own thread, the interface going on meanwhile.
    pub(super) fn run_action(&mut self, ctx: &egui::Context, id: &str, a: &'static str) {
        match a {
            "ask-merge" => self.orch.confirm = Some((id.to_string(), "merge")),
            "ask-discard" => self.orch.confirm = Some((id.to_string(), "discard")),
            "cancel" => self.orch.confirm = None,
            "open" => {
                if let Some(r) = self.ws.runs.iter().find(|r| r.id == id) {
                    let _ = crate::opener::open_path(std::path::Path::new(&r.worktree));
                }
            }
            "merge" | "discard" => {
                self.orch.confirm = None;
                let Some(rec) = self.ws.runs.iter().find(|r| r.id == id).cloned() else {
                    return;
                };
                let title = rec.title.clone();
                let op = if a == "merge" {
                    CopyOp::Merge
                } else {
                    CopyOp::Discard
                };
                // One git operation at a time per repository: a second would find it locked.
                if !self.start_copy_job(ctx, rec, op) {
                    self.fail(crate::trf!("{} : une autre copie de ce projet est en cours de traitement, réessayez dans un instant.", "{}: another copy of this project is being dealt with, try again in a moment.", title));
                }
            }
            _ => {}
        }
    }
}

/// How long the agent's prompt cache keeps a conversation (Claude Code on a subscription: one
/// hour). A pause longer than this would make the next turn write the whole history to the
/// cache again: a new conversation, starting from the mission, costs less.
pub(super) const CACHE_LIFE_MS: u64 = 60 * 60 * 1000;

/// Resuming after a pause of `paused_ms` goes on in a new conversation.
pub(super) fn fresh_after_pause(paused_ms: u64) -> bool {
    paused_ms > CACHE_LIFE_MS
}

impl LiveRun {
    /// Where the session stands, in a word or two.
    pub(super) fn state_text(&self) -> String {
        match self.phase {
            Phase::Starting(_) => crate::tr!("démarre", "starting").to_string(),
            Phase::Summary(_) => crate::tr!("compte rendu", "summary").to_string(),
            Phase::Closing(_) => crate::tr!("se ferme", "closing").to_string(),
            Phase::Waiting => crate::tr!("en pause", "paused").to_string(),
            _ => crate::trf!("tour {}", "turn {}", self.turns + 1),
        }
    }

    /// Give the AI its next prompt: typed into its terminal, or for an agent driven turn by
    /// turn in headless mode (Mistral Vibe), handed to the loop running its turns
    /// (`Agent::send_turn`), which starts the run at once (no Enter to press).
    pub(super) fn send(&mut self, text: &str) {
        if self.agent.headless() {
            if let Err(e) = self
                .agent
                .send_turn(std::path::Path::new(&self.worktree), text)
            {
                self.stop = Some(crate::trf!(
                    "consigne non transmise à {} : {e}",
                    "prompt not handed to {}: {e}",
                    self.agent.label()
                ));
            }
        } else {
            self.term.paste(text);
        }
    }

    /// Short sessions: once a step is done, the work goes on in a new conversation of the
    /// agent, started again from the mission (and get_project_state) rather than from a
    /// history that grows with each step. Claude Code clears its own (`/clear`); a headless
    /// agent's next run doesn't continue the last one. It is told it has no history and to
    /// start from get_project_state (`fresh_conversation_note`); `note` follows, then the
    /// mission.
    pub(super) fn new_conversation(&mut self, lead: &str, note: &str, now: Instant) {
        if self.agent.headless() {
            if let Err(e) = self
                .agent
                .new_conversation(std::path::Path::new(&self.worktree))
            {
                self.stop = Some(crate::trf!(
                    "nouvelle conversation impossible avec {} : {e}",
                    "no new conversation with {}: {e}",
                    self.agent.label()
                ));
            }
        } else {
            self.term.paste("/clear");
            self.term.write(b"\r");
        }
        self.reader.new_conversation();
        self.next = Some(format!("{lead}{note}{}", self.first_prompt));
        // As at the start: the prompt is typed once the agent is ready again.
        self.phase = Phase::Starting(now);
        self.screen_changed = now;
        self.last_line = crate::tr!(
            "Étape terminée : nouvelle conversation…",
            "Step done: new conversation…"
        )
        .into();
    }

    /// Put the session on hold until the person answers.
    pub(super) fn pause(&mut self, why: Wait) {
        self.waiting = Some(why);
        self.paused_wall = crate::model::wall_ms();
        self.phase = Phase::Waiting;
        self.paused_screen = self.screen_hash;
        self.last_line = why.label().into();
    }

    /// The person answered what the session waits for: in the terminal (the question is
    /// gone from a screen that moved since the pause), or in the inbox (`question_left`:
    /// one of its questions still has no answer).
    pub(super) fn answered(&self, screen: &str, idle: Duration, question_left: bool) -> bool {
        let moved = self.screen_hash != self.paused_screen && idle > Duration::from_millis(1500);
        match self.waiting {
            Some(Wait::Trust) => moved && auto::read_screen(screen) != Screen::Trust,
            Some(Wait::Permission) => moved && auto::read_screen(screen) != Screen::Permission,
            Some(Wait::Question) => !question_left,
            None => false,
        }
    }

    /// Leave the pause (« Reprendre »). A question still on the screen is refused, as
    /// without the pause.
    pub fn resume(&mut self) {
        let Some(why) = self.waiting.take() else {
            return;
        };
        let now = Instant::now();
        let still = auto::read_screen(&self.term.text());
        match why {
            Wait::Trust if still == Screen::Trust => {
                // Never accept for them; Claude Code closes and the session ends.
                self.term.write(b"\x1b");
                self.stop = Some(
                    crate::tr!(
                        "le dossier n'a pas été accepté dans Claude Code",
                        "the folder wasn't accepted in Claude Code"
                    )
                    .into(),
                );
                self.phase = Phase::Starting(now);
            }
            Wait::Trust => {
                self.phase = Phase::Starting(now);
                self.last_line =
                    crate::tr!("Démarrage de Claude Code…", "Starting Claude Code…").into();
            }
            Wait::Permission if still == Screen::Permission => {
                self.term.write(b"\x1b");
                self.denials += 1;
                self.last_line = crate::tr!(
                    "Commande refusée (non autorisée en autonome)",
                    "Command refused (not allowed when autonomous)"
                )
                .into();
                self.phase = Phase::Denied(now);
            }
            Wait::Permission => {
                self.phase = Phase::Working(now);
                self.last_line = turn_running(self.turns);
            }
            // Past the cache's life, the history would be paid again in full: a new
            // conversation, from get_project_state (where the answer now is).
            Wait::Question
                if fresh_after_pause(
                    crate::model::wall_ms().saturating_sub(self.paused_wall),
                ) =>
            {
                self.new_conversation(auto::fresh_conversation_note(true), "", now);
                self.last_line = crate::tr!(
                    "Longue pause : nouvelle conversation…",
                    "Long pause: new conversation…"
                )
                .into();
                return;
            }
            Wait::Question => {
                match &self.endless {
                    Some(c) => {
                        let p = auto::endless_prompt(!c.when_done);
                        self.send(p)
                    }
                    None => self.send(auto::continue_prompt()),
                }
                self.phase = Phase::Typed(now);
            }
        }
        self.screen_changed = now;
    }
}

/// Whether a session can start under the global limit: at least 1 % of the window is left.
pub(super) fn room_left(five: Option<f32>, global: f32) -> bool {
    five.is_none_or(|f| global - f >= 1.0)
}

/// A session's ceiling in absolute % of the 5-hour window: what the window held without it
/// (its figure `five` less the session's own use `pct`, 0 at its start) plus its budget,
/// never past the global limit. None while the window's figure is unknown.
pub(super) fn ceiling_from(five: Option<f32>, pct: f32, budget: f32, global: f32) -> Option<f32> {
    five.map(|f| ((f - pct).max(0.0) + budget).min(global).clamp(0.0, 100.0))
}

impl Figures {
    /// A new reading (`new`, None when it failed) laid over the last known figures: a window
    /// the reading misses keeps its last figure as long as its reset time is still ahead,
    /// so a failed or partial reading never passes for 0 % (nor lifts the ceilings). A window
    /// that has reset, or whose reset time is unknown, is forgotten. Readings are taken again
    /// every few seconds: the next one that succeeds replaces what was kept.
    pub(super) fn merged(self, new: Option<Figures>, now: u64) -> Figures {
        let new = new.unwrap_or_default();
        let keep = |pct: Option<f32>, reset: Option<u64>| match (pct, reset) {
            (Some(p), Some(r)) if r > now => (Some(p), Some(r)),
            _ => (None, None),
        };
        let (five, five_reset) = if new.five.is_some() {
            (new.five, new.five_reset)
        } else {
            keep(self.five, self.five_reset)
        };
        let (week, week_reset) = if new.week.is_some() {
            (new.week, new.week_reset)
        } else {
            keep(self.week, self.week_reset)
        };
        Figures { five, week, five_reset, week_reset }
    }
}

/// The only step the session `run` was sent on (« Lancer » on its line); empty: none.
pub(super) fn sent_on<'a>(records: &'a [crate::model::RunRecord], run: &str) -> &'a str {
    records
        .iter()
        .find(|x| x.id == run)
        .map_or("", |x| x.step.as_str())
}

/// The step a session is on: the one it was sent on, else the one it is doing (`holder`:
/// see `current_step`).
pub(super) fn session_step<'a>(
    sent_on: &str,
    steps: &'a [crate::project::Step],
    holder: &str,
) -> Option<&'a crate::project::Step> {
    if sent_on.is_empty() {
        current_step(steps, holder)
    } else {
        steps.iter().find(|s| s.id == sent_on)
    }
}

pub(super) fn turn_running(turns: u32) -> String {
    crate::trf!("Tour {} en cours…", "Turn {} running…", turns + 1)
}

/// The name under which a session of `agent` on computer `computer` reserves steps: what
/// `store::holder_id` gives its MCP server.
pub(super) fn holder_of(agent: Agent, computer: &str) -> String {
    match agent.holder() {
        "" => computer.to_string(),
        h => format!("{computer}#{h}"),
    }
}

/// Forget the planned restarts of a project's sessions; returns those that had one.
pub(super) fn cancel_resumes(
    runs: &mut [crate::model::RunRecord],
    project: &str,
) -> Vec<crate::model::RunRecord> {
    let mut left = vec![];
    for x in runs.iter_mut().filter(|x| x.project == project) {
        if x.resume_at > 0 {
            left.push(x.clone());
            x.resume_at = 0;
        }
    }
    left
}

/// The AI of a recorded session, named by its label.
pub(super) fn agent_of(rec: &crate::model::RunRecord) -> Agent {
    Agent::ALL
        .into_iter()
        .find(|a| a.label() == rec.tool)
        .unwrap_or(Agent::ClaudeCode)
}

/// What the end of a session on one step makes of that step, from why it stopped (`why`),
/// whether it starts again by itself (`resumes`) and whether a question it asked still
/// waits (`asked`: « done » is then not taken from its word). None: the step stays « doing ».
pub(super) fn step_issue(why: &str, resumes: bool, asked: bool) -> Option<StepIssue> {
    if let Some(rest) = why.strip_prefix(blocked_text()) {
        let said = rest.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
        return Some(StepIssue::Blocked(match said {
            "" => crate::tr!("l'IA n'a pas dit pourquoi", "the AI didn't say why").into(),
            said => said.to_string(),
        }));
    }
    if why == Stop::MissionDone.text() && !asked {
        Some(StepIssue::Finished)
    } else if resumes {
        None
    } else {
        Some(StepIssue::Interrupted)
    }
}

pub(super) fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// A session's work on the steps since it started.
#[derive(Debug, PartialEq)]
pub(super) struct StepsSince {
    /// Titles of the steps done since the start (as they read now).
    pub(super) done: Vec<String>,
    /// Steps an AI added since the start and not done yet. Those the person added in the
    /// meantime don't count: adding steps while a session runs never stops it.
    pub(super) added_open: usize,
}

/// Compared by step id, not by title: a step renamed during the session stays the same
/// step (it used to count as new, and as done twice), and two steps with one title stay
/// two. `before`: (step id, done) at the start.
pub(super) fn steps_since(before: &[(String, bool)], steps: &[crate::project::Step]) -> StepsSince {
    let done = |s: &crate::project::Step| s.status == crate::project::StepStatus::Done;
    let at_start =
        |s: &crate::project::Step| before.iter().find(|(id, _)| *id == s.id).map(|(_, d)| *d);
    StepsSince {
        done: steps
            .iter()
            .filter(|s| done(s) && at_start(s) != Some(true))
            .map(|s| s.title.clone())
            .collect(),
        added_open: steps
            .iter()
            .filter(|s| !done(s) && at_start(s).is_none() && !s.by_person())
            .count(),
    }
}

/// The step a session is doing: « doing » and reserved by this computer (`me`), or not
/// reserved at all; the most recently changed when there are several.
pub(super) fn current_step<'a>(
    steps: &'a [crate::project::Step],
    me: &str,
) -> Option<&'a crate::project::Step> {
    steps
        .iter()
        .filter(|s| {
            s.status == crate::project::StepStatus::Doing
                && (s.held_by.is_empty() || s.held_by == me)
        })
        .max_by_key(|s| s.updated)
}

/// Open steps the person added (typed, accepted, received) since the turn began: `at_turn`
/// holds the step ids then. Their titles, in the project's order.
pub(super) fn fresh_steps(at_turn: &[String], steps: &[crate::project::Step]) -> Vec<String> {
    steps
        .iter()
        .filter(|s| {
            s.status != crate::project::StepStatus::Done
                && s.by_person()
                && !at_turn.contains(&s.id)
        })
        .map(|s| s.title.clone())
        .collect()
}

/// Put before the next turn's prompt when the person added steps during the turn, so the
/// AI takes them without leaving its endless mode. Empty when there are none.
pub(super) fn new_steps_note(fresh: &[String]) -> String {
    if fresh.is_empty() {
        return String::new();
    }
    let list: Vec<String> = fresh
        .iter()
        .take(5)
        .map(|t| format!("« {} »", crate::richtext::one_line(t, 100)))
        .collect();
    crate::trf!(
        "La personne vient d'ajouter {} étape(s) : {}. Prends-les en compte. ",
        "The person just added {} step(s): {}. Take them into account. ",
        fresh.len(),
        list.join(", ")
    )
}

/// How long a failed reading of the subscription goes unsaid (readings are taken every 3 s,
/// and one can miss while Claude Code rewrites its files).
pub(super) const MEASURE_GRACE_MS: u64 = 20_000;

/// The alert (title, text) shown while Claude Code's 5-hour window can't be read: since when
/// (`since`), the figure kept from the last reading (`kept`), whether sessions are running.
/// None while it is read, during a short grace, or when nothing rests on it (no session
/// running and no figure kept: the tile already says « unknown for now »).
pub(super) fn measure_alert(
    since: Option<u64>,
    kept: Option<f32>,
    running: bool,
    now: u64,
) -> Option<(String, String)> {
    let since = since?;
    let gone = now.saturating_sub(since);
    if gone < MEASURE_GRACE_MS || (!running && kept.is_none()) {
        return None;
    }
    let title = crate::trf!(
        "Mesure de l'abonnement Claude Code indisponible depuis {}",
        "Claude Code subscription use unavailable for {}",
        crate::usage::left_text(gone)
    );
    let text = match kept {
        Some(k) => crate::trf!(
            "La dernière valeur connue ({k:.0} %) est gardée jusqu'à la réinitialisation de la fenêtre : elle n'est jamais prise pour 0 % et les plafonds continuent d'arrêter les IA, mais elle ne voit pas ce qui a été utilisé depuis. Une session jamais mesurée s'arrêtera à son deuxième tour. Orchestra réessaie toutes les 3 s ; vérifiez que la ligne d'état de Claude Code mène bien à Orchestra.",
            "The last known figure ({k:.0} %) is kept until the window resets: it is never taken for 0 % and the limits still stop the AIs, but it doesn't see what has been used since. A session never measured stops at its second turn. Orchestra tries again every 3 s; check that Claude Code's status line leads to Orchestra."
        ),
        None => crate::tr!(
            "Aucune valeur connue : aucun plafond ne peut être vérifié, donc les sessions qui n'ont pas été mesurées s'arrêteront à leur deuxième tour plutôt que de tourner sans limite. Orchestra réessaie toutes les 3 s ; vérifiez que Claude Code est à jour et que sa ligne d'état mène bien à Orchestra.",
            "No known figure: no limit can be checked, so sessions that weren't measured stop at their second turn rather than run without a limit. Orchestra tries again every 3 s; check that Claude Code is up to date and that its status line leads to Orchestra."
        )
        .into(),
    };
    Some((title, text))
}

/// What a session's turn ended on, to choose whether it stops.
pub(super) struct TurnEnd {
    /// A stop already asked (by the person, or a limit seen during the turn).
    pub(super) asked: Option<String>,
    pub(super) too_long: bool,
    /// Endless mode: the end condition reached, if any.
    pub(super) endless_end: Option<String>,
    pub(super) signal: Signal,
    /// What the AI says blocks it, with `Signal::Blocked` (`auto::block_reason`).
    pub(super) blocked_why: String,
    pub(super) endless: bool,
    /// The session's use of the 5-hour window, and its budget.
    pub(super) pct: f32,
    pub(super) limit: f32,
    /// Where it stops, in absolute % of the 5-hour window (`LiveRun::ceiling`).
    pub(super) ceiling: Option<f32>,
    /// The 5-hour window, and the global limit.
    pub(super) five: Option<f32>,
    pub(super) global: f32,
    /// The week, the share of it the AIs may use, and the most one turn took from it.
    pub(super) week: Option<f32>,
    pub(super) week_limit: f32,
    pub(super) week_turn: f32,
    pub(super) measured: bool,
    pub(super) turns: u32,
    pub(super) max_turns: u32,
    /// An API-mode session's estimated spend and its budget, in micro-dollars.
    pub(super) spent_micro: u64,
    pub(super) budget_micro: Option<u64>,
}

/// Why a session stops after a turn.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Stop {
    Asked(String),
    TooLong,
    EndlessEnd(String),
    MissionDone,
    /// With what the AI says blocks it (empty: it didn't say).
    Blocked(String),
    Budget(f32),
    /// An API-mode session spent its budget in dollars (micro-dollars).
    Dollars(u64),
    /// The session's own ceiling on the 5-hour window (absolute %), under the global limit.
    Ceiling(f32),
    GlobalLimit(f32),
    /// The week's reserve (in %) is near.
    WeekReserve(f32),
    /// Claude Code said the subscription's own limit is reached.
    SubscriptionLimit,
    Unmeasured,
    Turns(u32),
}

/// The quota a stop waits for, to start again once it resets.
#[derive(Debug, PartialEq)]
pub(super) enum Quota {
    None,
    FiveHours,
    Week,
}

impl Stop {
    pub(super) fn quota(&self) -> Quota {
        match self {
            Stop::GlobalLimit(_) => Quota::FiveHours,
            Stop::WeekReserve(_) => Quota::Week,
            _ => Quota::None,
        }
    }

    pub(super) fn text(&self) -> String {
        match self {
            Stop::Asked(s) | Stop::EndlessEnd(s) => s.clone(),
            Stop::TooLong => crate::tr!(
                "elle a mis trop de temps à répondre",
                "it took too long to answer"
            )
            .into(),
            Stop::MissionDone => crate::tr!("mission terminée", "mission complete").into(),
            Stop::Blocked(why) if why.is_empty() => blocked_text().into(),
            Stop::Blocked(why) => {
                crate::trf!("{} : {why}", "{}: {why}", blocked_text())
            }
            Stop::Budget(l) => crate::trf!(
                "ce projet a utilisé toute la part d'abonnement prévue pour lui ({l:.0} %)",
                "this project used all the share of the subscription set for it ({l:.0} %)"
            ),
            Stop::Dollars(b) => crate::trf!(
                "budget en dollars atteint ({})",
                "dollar budget reached ({})",
                crate::pricing::dollars(*b)
            ),
            Stop::Ceiling(c) => crate::trf!(
                "la session a atteint son maximum ({c:.0} % de la limite de 5 heures de l'abonnement)",
                "the session reached its maximum ({c:.0} % of the subscription's 5-hour limit)"
            ),
            Stop::GlobalLimit(g) => crate::trf!(
                "la limite d'utilisation réglée dans Orchestra est atteinte ({g:.0} % de l'abonnement)",
                "the usage limit set in Orchestra is reached ({g:.0} % of the subscription)"
            ),
            Stop::WeekReserve(r) => crate::trf!(
                "pause pour vous laisser au moins {r:.0} % de l'abonnement de la semaine",
                "paused to leave you at least {r:.0} % of the week's subscription"
            ),
            // Any agent's (Claude, Codex…): the session knows which by its card.
            Stop::SubscriptionLimit => crate::tr!(
                "limite de l'abonnement de l'IA atteinte",
                "the AI's subscription limit reached"
            )
            .into(),
            Stop::Unmeasured => crate::tr!(
                "impossible de savoir ce qu'elle consomme de l'abonnement, arrêt par prudence",
                "what it uses of the subscription can't be measured, stopped to be safe"
            )
            .into(),
            Stop::Turns(n) => crate::trf!(
                "nombre maximum d'échanges atteint ({n})",
                "maximum number of exchanges reached ({n})"
            ),
        }
    }
}

/// How every reason of a blocked session starts (`Stop::Blocked`).
pub(super) fn blocked_text() -> &'static str {
    crate::tr!("l'IA est bloquée", "the AI is blocked")
}

/// The title of the inbox message saying that the session `title` is paused.
pub(super) fn waiting_title(title: &str) -> String {
    crate::trf!(
        "{title} attend votre réponse",
        "{title} waits for your answer"
    )
}

/// Whether a session stops after a turn, and why: the first reason in this order. None:
/// it goes on.
pub(super) fn stop_reason(t: &TurnEnd) -> Option<Stop> {
    Some(if let Some(s) = &t.asked {
        Stop::Asked(s.clone())
    } else if t.too_long {
        Stop::TooLong
    } else if let Some(why) = &t.endless_end {
        Stop::EndlessEnd(why.clone())
    } else if t.signal == Signal::Done && !t.endless {
        // Endless mode has its own end conditions instead of [[FINI]].
        Stop::MissionDone
    } else if t.signal == Signal::Blocked {
        Stop::Blocked(t.blocked_why.clone())
    } else if let Some(b) = t.budget_micro
        && t.spent_micro >= b
    {
        Stop::Dollars(b)
    } else if let Some(c) = t.ceiling
        && c < t.global
        && t.five.is_some_and(|f| f >= c)
    {
        // At the global limit itself, the global limit's stop says it (and starts the
        // session again once the window resets).
        Stop::Ceiling(c)
    } else if t.pct >= t.limit {
        Stop::Budget(t.limit)
    } else if t.five.is_some_and(|f| f >= t.global) {
        Stop::GlobalLimit(t.global)
    } else if t
        .week
        .is_some_and(|w| crate::usage::near_reserve(w, t.week_limit, t.week_turn))
    {
        // Paused before the next turn bites into the reserve; starts again once the week resets.
        Stop::WeekReserve(100.0 - t.week_limit)
    } else if !t.measured && t.turns >= 2 {
        Stop::Unmeasured
    } else if !t.endless && t.turns >= t.max_turns {
        Stop::Turns(t.max_turns)
    } else {
        return None;
    })
}

impl Figures {
    pub(super) fn of(u: &crate::usage::Usage) -> Self {
        Figures {
            five: u.measured().map(|w| w.pct),
            week: u.week().map(|w| w.pct),
            five_reset: u.measured().and_then(|w| w.resets_at),
            week_reset: u.week().and_then(|w| w.resets_at),
        }
    }
}

/// When a session starts, on the computer's real clock: the readings it is measured by are
/// stamped by the status line, another process, on that clock too. Orchestra's own clock
/// (`model::now_ms`) can run ahead of it (it did by up to 38 minutes before its lead was
/// capped, and still does after a time seen from a computer whose clock is ahead): a start
/// stamped on it came after every reading of a short session, which was then never measured
/// and stopped at its second turn « impossible à mesurer ».
pub(super) fn session_start() -> u64 {
    crate::model::wall_ms()
}

/// A new reading of `agent`'s 5-hour window (taken at `at`): its rise since `last` is shared
/// between that agent's running sessions only, by the tokens each used since then; the
/// sessions of other agents, on other subscriptions, get nothing of it.
pub(super) fn apply_reading(
    runs: &mut [LiveRun],
    agent: Agent,
    last: &mut Option<(u64, f32, Option<u64>)>,
    at: u64,
    five: &crate::usage::Window,
) {
    let Some(rise) = next_rise(last, at, five) else { return };
    let sessions: Vec<(Agent, u64)> = runs
        .iter()
        .map(|r| (r.agent, r.reader.tokens.saturating_sub(r.tokens_counted)))
        .collect();
    for (r, s) in runs.iter_mut().zip(agent_shares(&sessions, agent, rise)) {
        let Some(s) = s else { continue };
        if at >= r.started_wall {
            r.measured = true;
        }
        r.pct += s;
        r.tokens_counted = r.reader.tokens;
    }
}

/// Each session's share of a rise of `agent`'s window (`sessions`: each one's agent and the
/// tokens it used since the last reading): None for the sessions of other agents.
pub(super) fn agent_shares(sessions: &[(Agent, u64)], agent: Agent, rise: f32) -> Vec<Option<f32>> {
    let deltas: Vec<u64> = sessions
        .iter()
        .filter(|(a, _)| *a == agent)
        .map(|(_, d)| *d)
        .collect();
    let mut shares = split_rise(rise, &deltas).into_iter();
    sessions
        .iter()
        .map(|(a, _)| if *a == agent { shares.next() } else { None })
        .collect()
}

/// The last `n` lines of a terminal screen that hold something.
pub(super) fn last_lines(screen: &str, n: usize) -> String {
    let lines: Vec<&str> = screen.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// The rise of the 5-hour window that a new reading (taken at `at`) brings since `last`,
/// which it then becomes; None for the reading already counted. A failed reading never gets
/// here and leaves `last` as it was: what was used meanwhile is counted whole by the next
/// reading that succeeds, so a measure that fails for a while loses nothing of a budget.
pub(super) fn next_rise(
    last: &mut Option<(u64, f32, Option<u64>)>,
    at: u64,
    five: &crate::usage::Window,
) -> Option<f32> {
    let prev = *last;
    if prev.is_some_and(|(a, _, _)| a == at) {
        return None;
    }
    *last = Some((at, five.pct, five.resets_at));
    Some(window_rise(prev.map(|(_, p, r)| (p, r)), five.pct, five.resets_at))
}

/// How much the 5-hour window rose since the last reading (`prev`: its figure and reset
/// time then). A new window (another reset time) counts from zero; the first reading
/// counts for nothing: what was used before it isn't anyone's.
pub(super) fn window_rise(prev: Option<(f32, Option<u64>)>, pct: f32, resets_at: Option<u64>) -> f32 {
    match (prev, resets_at) {
        (Some((p, Some(r))), Some(now)) if r.abs_diff(now) < 120_000 => (pct - p).max(0.0),
        (Some((p, None)), None) => (pct - p).max(0.0),
        (Some(_), _) => pct,
        (None, _) => 0.0,
    }
}

/// A rise of the 5-hour window shared between the running sessions, by the tokens each
/// used since the last reading (evenly when none is known). What each session shows
/// (`LiveRun::pct`) is the sum of its shares since it started: the window's own figure
/// at its start is never in it.
pub(super) fn split_rise(rise: f32, tokens: &[u64]) -> Vec<f32> {
    let total: u64 = tokens.iter().sum();
    let n = tokens.len().max(1) as f32;
    tokens
        .iter()
        .map(|&d| {
            if total > 0 {
                rise * d as f32 / total as f32
            } else {
                rise / n
            }
        })
        .collect()
}

/// What the end message of an autonomous session tells.
pub(super) struct SessionEnd<'a> {
    pub(super) project: &'a str,
    /// Why it stopped (`Stop::text` and the like): a few words, no final full stop.
    pub(super) why: &'a str,
    pub(super) steps: usize,
    /// Commits made, and changed files left without one.
    pub(super) saved: usize,
    pub(super) unsaved: usize,
    /// Share of the subscription used, in %; 0 when it isn't known.
    pub(super) pct: f32,
    /// In how long it starts again by itself, if it does.
    pub(super) resume_in: Option<String>,
}

/// The inbox message at the end of a session: its title and its text, in everyday words
/// (one sentence per line, the reason first; the report it opens has the technical detail).
pub(super) fn end_message(e: &SessionEnd) -> (String, String) {
    let done = e.why == Stop::MissionDone.text();
    let title = if done {
        crate::trf!("L'IA a terminé son travail sur {}", "The AI finished its work on {}", e.project)
    } else {
        crate::trf!("L'IA s'est arrêtée sur {}", "The AI stopped on {}", e.project)
    };
    let mut lines = vec![];
    if !done {
        let why = e.why.trim().trim_end_matches('.');
        lines.push(crate::trf!("Raison : {why}.", "Reason: {why}."));
    }
    let steps = match e.steps {
        0 => crate::tr!("Aucune étape terminée", "No step finished").to_string(),
        n => {
            let s = crate::i18n::plural(n);
            crate::trf!("{n} étape{s} terminée{s}", "{n} step{s} finished")
        }
    };
    let saved = match e.saved {
        0 => String::new(),
        n => {
            let s = crate::i18n::plural(n);
            crate::trf!(
                ", {n} modification{s} enregistrée{s}",
                ", {n} change{s} saved"
            )
        }
    };
    lines.push(format!("{steps}{saved}."));
    if e.unsaved > 0 {
        let n = e.unsaved;
        let s = crate::i18n::plural(n);
        lines.push(crate::trf!(
            "{n} fichier{s} modifié{s} mais pas encore enregistré{s}.",
            "{n} file{s} changed but not saved yet."
        ));
    }
    if e.pct >= 0.5 {
        lines.push(crate::trf!(
            "Part de votre abonnement utilisée : environ {:.0} %.",
            "Share of your subscription used: about {:.0} %.",
            e.pct
        ));
    }
    // Right after the reason: the card only shows the first lines (`TEXT_LINES`).
    if let Some(left) = &e.resume_in {
        let at = usize::from(!done);
        lines.insert(at, crate::trf!(
            "Elle reprendra toute seule dans {left}, quand votre abonnement le permettra à nouveau.",
            "It will start again by itself in {left}, once your subscription allows it again."
        ));
    }
    (title, lines.join("\n"))
}
