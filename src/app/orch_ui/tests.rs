/// An AI's block in Settings › AI offers the way back to the common limits only once it
/// has limits of its own, and taking it forgets them.
#[test]
fn an_ais_own_limits_can_be_dropped_for_the_common_ones() {
    use egui_kittest::kittest::Queryable;
    let t = super::super::tone(false);
    // Drawn only once the app's fonts are in.
    let mut h = egui_kittest::Harness::builder()
        .with_size([700.0, 300.0])
        .build_ui_state(
            move |ui, s: &mut (bool, crate::model::OrchSettings)| {
                if s.0 {
                    super::agent_limits(
                        ui,
                        crate::auto::Agent::ClaudeCode,
                        super::Figures::default(),
                        &mut s.1,
                        &t,
                    );
                }
            },
            Default::default(),
        );
    super::super::setup_fonts(&h.ctx);
    h.run();
    h.state_mut().0 = true;
    h.run();
    assert!(h.query_by_label("↺ Valeurs communes").is_none());
    h.state_mut()
        .1
        .set_five_limit(crate::auto::Agent::ClaudeCode, 30.0);
    h.run();
    h.get_by_label("↺ Valeurs communes").click();
    h.run();
    assert!(h.state().1.per_agent.is_empty());
}

/// A project's launch settings start from the choices kept in the project, else from the
/// settings; and what it keeps back is what it started from.
#[test]
fn a_draft_starts_from_the_kept_choices() {
    use super::{Draft, default_prefs};
    use crate::model::EndConditions;
    use crate::project::{LaunchPrefs, Project};
    // Never set: the default ceiling and the settings' commits, not endless, 4 h when
    // switched on.
    let d = Draft::new(None, true);
    assert_eq!(
        (d.ceiling, d.commits, d.endless),
        (None, true, false)
    );
    assert_eq!(d.end.hours, Some(4.0));
    assert_eq!(d.prefs(), default_prefs(true));
    let fresh = Project::default();
    assert_eq!(Draft::new(Some(&fresh), false).ceiling, None);
    // Set before: 95 %, no commits, endless for 6 h or 12 turns.
    let kept = LaunchPrefs {
        commits: false,
        endless: true,
        end: EndConditions {
            hours: Some(6.0),
            turns: Some(12),
            ..Default::default()
        },
        agent: None, model: String::new(),
        saving: crate::model::Saving::Economical,
        auto_agent: false,
    };
    let p = Project {
        session_ceiling: Some(95.0),
        launch: Some(kept.clone()),
        ..Default::default()
    };
    let d = Draft::new(Some(&p), true);
    assert_eq!((d.ceiling, d.commits, d.endless), (Some(95.0), false, true));
    assert!(d.mission.is_empty());
    assert_eq!(d.prefs(), kept);
    // Only the ceiling kept: the rest from the settings. The old relative budget isn't
    // taken for a ceiling (20 % of budget isn't « stop at 20 % »).
    let p = Project {
        session_ceiling: Some(50.0),
        session_limit: Some(20.0),
        ..Default::default()
    };
    let d = Draft::new(Some(&p), true);
    assert_eq!((d.ceiling, d.commits, d.endless), (Some(50.0), true, false));
    let old = Project { session_limit: Some(20.0), ..Default::default() };
    assert_eq!(Draft::new(Some(&old), true).ceiling, None);
}

/// Only an absolute ceiling is asked at the start: the one chosen, else the window's
/// figure plus the settings' default share; always above the window, never past the
/// global limit.
#[test]
fn a_session_starts_with_an_absolute_ceiling() {
    use super::{ceiling_choice, ceiling_floor, room_left};
    // 37 % used, « stop at 50 % » chosen: 50.
    assert_eq!(ceiling_choice(Some(50.0), Some(37.0), 20.0, 90.0), 50.0);
    // Nothing chosen: 37 + 20 = 57; window unknown: 20.
    assert_eq!(ceiling_choice(None, Some(37.0), 20.0, 90.0), 57.0);
    assert_eq!(ceiling_choice(None, None, 20.0, 90.0), 20.0);
    // Never past the global limit…
    assert_eq!(ceiling_choice(Some(99.0), Some(37.0), 20.0, 90.0), 90.0);
    assert_eq!(ceiling_choice(None, Some(80.0), 20.0, 90.0), 90.0);
    // …and always above the window (a ceiling already reached would stop it at once).
    assert_eq!(ceiling_floor(Some(37.4), 90.0), 38.0);
    assert_eq!(ceiling_choice(Some(30.0), Some(37.4), 20.0, 90.0), 38.0);
    assert_eq!(ceiling_floor(None, 90.0), 1.0);
    // No start once the window is at the global limit (less than 1 % left).
    assert!(room_left(Some(89.0), 90.0));
    assert!(!room_left(Some(89.5), 90.0));
    assert!(room_left(None, 90.0));
}

/// The AI of a project's sessions: Claude Code by default, kept in the project once
/// another is chosen; only those wired for working alone can be picked.
#[test]
fn a_session_starts_with_the_ai_chosen() {
    use super::{Agent, Draft, agent_unavailable};
    let d = Draft::new(None, true);
    assert_eq!(d.agent, Agent::ClaudeCode);
    // Claude Code, the default, isn't written in the project.
    assert_eq!(d.prefs().agent, None);
    let mut d2 = d.clone();
    d2.agent = Agent::Codex;
    let kept = crate::project::Project { launch: Some(d2.prefs()), ..Default::default() };
    assert_eq!(Draft::new(Some(&kept), true).agent, Agent::Codex);
    // Written and read back by name.
    let json = serde_json::to_string(&d2.prefs()).unwrap();
    assert!(json.contains("\"codex\""), "{json}");
    // Claude Code can be picked; an AI not wired yet says why.
    assert!(agent_unavailable(Agent::ClaudeCode).is_none());
    for a in Agent::ALL {
        assert_eq!(agent_unavailable(a).is_none(), a.ready(), "{a:?}");
    }
    // Gemini: the reason names what's missing, the % behind « arrêt à ». Mistral Vibe
    // gives it (its monthly budget) and its turns are driven: it can be picked.
    let why = agent_unavailable(Agent::Antigravity).unwrap();
    assert!(why.contains("arrêt à") || why.contains("stop at"), "{why}");
    assert_eq!(agent_unavailable(Agent::Vibe).is_none(), cfg!(unix));
    // Only the plugged AIs are listed, in the usual order, plus the one already chosen.
    use super::agent_choices;
    let plugged = [Agent::Vibe, Agent::ClaudeCode];
    assert_eq!(agent_choices(&plugged, Agent::ClaudeCode), [Agent::ClaudeCode, Agent::Vibe]);
    assert_eq!(
        agent_choices(&plugged, Agent::Codex),
        [Agent::ClaudeCode, Agent::Codex, Agent::Vibe]
    );
    assert_eq!(agent_choices(&[], Agent::ClaudeCode), [Agent::ClaudeCode]);
}

/// A project's ceiling and share are worked out on the subscription of the AI chosen for
/// it: its window, and the ceilings of the sessions running with that same AI only.
#[test]
fn percentages_are_worked_out_on_the_ai_chosen() {
    use super::{Agent, Draft, ceiling_choice, draft_ceiling, running_ceiling_of};
    let runs = [(Agent::ClaudeCode, Some(50.0)), (Agent::Codex, Some(30.0)), (Agent::Codex, None)];
    assert_eq!(running_ceiling_of(&runs, Agent::ClaudeCode), Some(50.0));
    assert_eq!(running_ceiling_of(&runs, Agent::Codex), Some(30.0));
    assert_eq!(running_ceiling_of(&runs, Agent::Antigravity), None);
    // Claude's window at 45 %, Codex's at 10 %: a Codex project starts from Codex's own
    // running ceiling (30 %) and counts from Codex's window, not Claude's.
    let mut d = Draft::new(None, true);
    d.agent = Agent::Codex;
    let (five, running) = (Some(10.0), running_ceiling_of(&runs, d.agent));
    let c = ceiling_choice(draft_ceiling(&d, running), five, 20.0, 90.0);
    assert_eq!(c, 30.0);
    // The same project on Claude Code: Claude's figures.
    d.agent = Agent::ClaudeCode;
    let (five, running) = (Some(45.0), running_ceiling_of(&runs, d.agent));
    assert_eq!(ceiling_choice(draft_ceiling(&d, running), five, 20.0, 90.0), 50.0);
}

#[test]
fn the_next_session_may_stop_higher() {
    use super::{Draft, Stop, ceiling_choice, draft_ceiling, parallel_note, stop_reason};
    let mut d = Draft::new(None, true);
    (d.ceiling, d.touched) = (Some(60.0), true);
    let c = ceiling_choice(draft_ceiling(&d, Some(50.0)), Some(42.0), 20.0, 90.0);
    assert_eq!(c, 60.0);
    // Nothing shares the global limit out any more: 60 beside 50 is fine.
    // The window at 55 %: the first stops at its 50 %, the second goes on to 60 %.
    let at = |ceiling: f32, five: f32| {
        let mut t = calm();
        (t.ceiling, t.five, t.pct, t.limit, t.global) = (Some(ceiling), Some(five), 5.0, 90.0, 90.0);
        stop_reason(&t)
    };
    assert_eq!(at(50.0, 55.0), Some(Stop::Ceiling(50.0)));
    assert_eq!(at(60.0, 55.0), None);
    assert_eq!(at(60.0, 60.0), Some(Stop::Ceiling(60.0)));
    // What the tooltip says: the offer, then the outcome once raised.
    assert!(parallel_note(None, 60.0, true).is_empty());
    assert!(parallel_note(Some(50.0), 50.0, false).contains("60 %"));
    let n = parallel_note(Some(50.0), 60.0, true);
    assert!(n.contains("60 %") && n.contains("50 %"), "{n}");
}

#[test]
fn a_parallel_session_starts_from_the_running_ceiling() {
    use super::{Draft, ceiling_choice, draft_ceiling, running_ceiling};
    assert_eq!(running_ceiling([]), None);
    assert_eq!(running_ceiling([None, Some(50.0)]), Some(50.0));
    assert_eq!(running_ceiling([Some(45.0), Some(50.0), None]), Some(50.0));
    let kept = crate::project::Project { session_ceiling: Some(70.0), ..Default::default() };
    let mut d = Draft::new(Some(&kept), true);
    // Nothing running: the one kept.
    assert_eq!(draft_ceiling(&d, None), Some(70.0));
    // 50 % running, the window at 42 %: 50 for this one too.
    assert_eq!(draft_ceiling(&d, Some(50.0)), Some(50.0));
    assert_eq!(ceiling_choice(draft_ceiling(&d, Some(50.0)), Some(42.0), 20.0, 90.0), 50.0);
    // Nothing kept either: still the running one, not 42 + 20.
    let fresh = Draft::new(None, true);
    assert_eq!(ceiling_choice(draft_ceiling(&fresh, Some(50.0)), Some(42.0), 20.0, 90.0), 50.0);
    // Changed in the launch settings (to 60 %): the person's choice wins.
    (d.ceiling, d.touched) = (Some(60.0), true);
    assert_eq!(draft_ceiling(&d, Some(50.0)), Some(60.0));
    // The running ceiling already reached by the window: raised just above it.
    assert_eq!(ceiling_choice(draft_ceiling(&fresh, Some(50.0)), Some(50.2), 20.0, 90.0), 51.0);
}

#[test]
fn only_endless_on_main_is_warned() {
    assert!(super::endless_on_main(true, true));
    assert!(!super::endless_on_main(true, false));
    assert!(!super::endless_on_main(false, true));
    assert!(!super::endless_main_warning().is_empty());
}

#[test]
fn a_project_without_code_never_merges_into_main() {
    let mut pr = crate::project::Project {
        commit_to_main: true,
        code: Some(true),
        ..Default::default()
    };
    assert!(super::on_main(&pr));
    // « sur main » ticked before the project was set as without code: it counts for nothing.
    pr.code = Some(false);
    assert!(!super::on_main(&pr));
    pr.code = Some(true);
    pr.commit_to_main = false;
    assert!(!super::on_main(&pr));
}

/// The Orchestration page shows the waiting questions first, then the recent answers;
/// never other messages nor old answers.
#[test]
fn orchestration_shows_waiting_questions_then_recent_answers() {
    use crate::model::InboxMsg;
    let day = 86_400_000;
    let now = 100 * day;
    let msg = |kind: &str, at: u64, answer: &str, answered_at: u64| InboxMsg {
        kind: kind.into(),
        at,
        answer: answer.into(),
        answered_at,
        ..Default::default()
    };
    let inbox = vec![
        msg("question", now - 30 * day, "", 0), // 0: old, still waiting
        msg("question", now - day, "oui", now - day), // 1: answered yesterday
        msg("fin", now, "", 0),                 // 2: not a question
        msg("question", now - 10 * day, "non", now - 9 * day), // 3: answered long ago
        msg("question", now - 2 * day, "", 0),  // 4: waiting, newer
        msg("question", now - 5 * day, "plus tard", now - 60_000), // 5: answered a minute ago
    ];
    assert_eq!(
        super::orch_questions(&inbox, now, super::ORCH_ANSWERED_MS),
        [4, 0, 5, 1]
    );
    // The project page keeps the answers whatever their age.
    assert_eq!(
        super::orch_questions(&inbox, now, u64::MAX),
        [4, 0, 5, 1, 3]
    );
    assert!(super::orch_questions(&[], now, super::ORCH_ANSWERED_MS).is_empty());
    let many: Vec<InboxMsg> = (0..9).map(|i| msg("question", now - i, "", 0)).collect();
    assert_eq!(
        super::orch_questions(&many, now, super::ORCH_ANSWERED_MS).len(),
        super::ORCH_QUESTIONS
    );
}

/// An end-of-session message finds its session by the report page, and only once that
/// session is over and nothing else deals with it.
#[test]
fn an_end_message_finds_its_finished_session() {
    use crate::model::{InboxMsg, RunRecord};
    let run = |id: &str, report: &str, outcome: &str| RunRecord {
        id: id.into(),
        report: Some(report.into()),
        outcome: outcome.into(),
        ..Default::default()
    };
    let runs = vec![
        run("a", "page-a", "terminée"),
        run("b", "page-b", ""),
        run("c", "page-c", "terminée"),
    ];
    let msg = |kind: &str, page: Option<&str>| InboxMsg {
        kind: kind.into(),
        page: page.map(Into::into),
        ..Default::default()
    };
    let idle = |_: &str| false;
    assert_eq!(
        super::run_of_message(&msg("fin", Some("page-a")), &runs, idle).map(|r| r.id.as_str()),
        Some("a")
    );
    // Still running, unknown page, no page, another kind of message.
    assert!(super::run_of_message(&msg("fin", Some("page-b")), &runs, idle).is_none());
    assert!(super::run_of_message(&msg("fin", Some("autre")), &runs, idle).is_none());
    assert!(super::run_of_message(&msg("fin", None), &runs, idle).is_none());
    assert!(super::run_of_message(&msg("fin", Some("")), &runs, idle).is_none());
    assert!(super::run_of_message(&msg("rapport", Some("page-a")), &runs, idle).is_none());
    // Started again, or waiting to be merged into main by itself.
    assert!(
        super::run_of_message(&msg("fin", Some("page-c")), &runs, |id| id == "c").is_none()
    );
}

/// Listed on the Orchestration page: only finished work still waiting in its copy whose
/// end message (and so its links) left the inbox.
#[test]
fn copies_left_without_their_end_message_are_listed() {
    use crate::model::{InboxMsg, RunRecord};
    let run = |id: &str, outcome: &str, closed: &str| RunRecord {
        id: id.into(),
        worktree: format!("/w/{id}"),
        report: Some(format!("page-{id}")),
        outcome: outcome.into(),
        closed: closed.into(),
        ..Default::default()
    };
    let runs = vec![
        run("orpheline", "terminée", ""),
        run("avec-message", "terminée", ""),
        run("fusionnee", "terminée", "fusionné"),
        run("en-cours", "", ""),
        run("copie-partie", "terminée", ""),
        run("vers-main", "terminée", ""),
    ];
    let inbox = vec![InboxMsg {
        kind: "fin".into(),
        page: Some("page-avec-message".into()),
        ..Default::default()
    }];
    let exists = |w: &str| w != "/w/copie-partie";
    assert_eq!(
        super::orphan_copies(&runs, &inbox, |id| id == "vers-main", exists),
        [0]
    );
    assert_eq!(
        super::orphan_copies(&runs, &[], |_| false, exists),
        [0, 1, 5]
    );
}

#[test]
fn inbox_messages_group_by_local_day() {
    let h = 3_600_000u64;
    let day = 24 * h;
    // Noon UTC on some day, local time UTC+1.
    let now = 20_000 * day + 12 * h;
    let g = |at: u64| super::day_group(at, now, h as i64);
    assert_eq!(g(now - h), g(now));
    // 22:30 UTC the day before is 23:30 local: yesterday; 23:30 UTC is 00:30 local: today.
    assert_eq!(g(now - 12 * h - 90 * 60_000), g(now - day));
    assert_eq!(g(now - 12 * h + 30 * 60_000), g(now));
    assert_ne!(g(now - day), g(now));
    assert_eq!(g(now - 3 * day), g(now - 6 * day));
    assert_ne!(g(now - 3 * day), g(now - 7 * day));
    // A clock a little behind: still today.
    assert_eq!(g(now + h), g(now));
}

use eframe::egui;

/// Runs `add` in a column of `width` px, as `orchestra_view` sets it (`ui.set_width`),
/// without a window. Two passes: egui settles sizes on the second.
fn column(width: f32, mut add: impl FnMut(&mut egui::Ui)) {
    let ctx = egui::Context::default();
    for _ in 0..2 {
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            ..Default::default()
        });
        egui::Area::new(egui::Id::unique("column"))
            .fixed_pos(egui::Pos2::ZERO)
            .show(&ctx, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(width);
                    add(ui);
                });
            });
        // No screen to send the textures to: drop them on purpose.
        ctx.end_pass().textures_delta.clear();
    }
}

/// A card as `projects::card` lays it out (full width, inner margin); its width.
fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) -> f32 {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(18, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        })
        .response
        .rect
        .width()
}

/// The running session's card: its inner frame and the row of its budget.
fn running(ui: &mut egui::Ui) {
    let t = super::tone(false);
    egui::Frame::new()
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            super::share_row(ui, "run", 0.0, 20.0, Some(90.0), Some(75.0), 90.0, &t);
        });
}

/// A card drawn before a session starts, the running session's card, then a card
/// after it; the widths of the three.
fn card_widths(width: f32) -> [f32; 3] {
    let mut out = [0.0; 3];
    column(width, |ui| {
        out[0] = card(ui, |ui| {
            ui.label("Cette semaine");
        });
        out[1] = card(ui, running);
        out[2] = card(ui, |ui| {
            ui.label("Réglages et limites");
        });
    });
    out
}

/// The mechanism behind the gap: in an egui column, something wider than the column
/// widens it for everything that follows, never for what came before. The column has
/// no grid nor fixed widths of its own (the cards fill `ui.available_width()`).
#[test]
fn a_row_wider_than_the_column_widens_what_follows() {
    let (mut before, mut after) = (0.0, 0.0);
    column(400.0, |ui| {
        before = card(ui, |ui| {
            ui.label("avant");
        });
        ui.allocate_exact_size(egui::vec2(460.0, 10.0), egui::Sense::hover());
        after = card(ui, |ui| {
            ui.label("après");
        });
    });
    assert_eq!(before, 400.0);
    assert_eq!(after, 460.0);
}

/// The cause was the budget row of a running session: its bar took all the width but
/// 150 px, kept for « 0.0 % utilisés sur un budget de 20 % », which needs more, so the
/// row came out wider than its card. Now it fills its card exactly, side by side when
/// there is room, the text under the bar when not, with large figures too.
#[test]
fn the_budget_row_fits_its_card() {
    for width in [140.0, 240.0, 300.0, 480.0, 1000.0] {
        for (pct, limit, window) in [
            (0.0, 20.0, None),
            (123.4, 100.0, None),
            (5.0, 90.0, Some(80.0)),
            (12.3, 100.0, Some(100.0)),
        ] {
            let mut row = 0.0;
            column(width, |ui| {
                ui.scope(|ui| {
                    let t = super::tone(false);
                    super::share_row(ui, "run", pct, limit, None, window, 90.0, &t);
                    row = ui.min_rect().width();
                });
            });
            assert!(
                row <= width + 0.5,
                "à {width} px ({pct} / {limit}) : rangée de {row} px"
            );
        }
    }
}

/// The gap once seen when a session started: the cards from the running one down came
/// out 57.5 px wider than the ones above.
#[test]
fn cards_keep_one_width_once_a_session_runs() {
    let mut uneven = vec![];
    for width in [320.0, 480.0, 620.0, 1000.0] {
        let [above, running, below] = card_widths(width);
        if (running - above).abs() >= 0.5 || (below - above).abs() >= 0.5 {
            uneven.push(format!(
                "à {width} px : au-dessus {above}, en cours {running}, en dessous {below}"
            ));
        }
    }
    assert!(uneven.is_empty(), "{}", uneven.join(" ; "));
}

/// With nothing running, the page leads to the projects a session could work on: those
/// with a folder and a step not done.
#[test]
fn the_quiet_page_lists_projects_with_open_steps() {
    let mut ws = crate::model::Workspace::default();
    let ready = ws.create_project("Prêt", "~/Code/pret");
    let finished = ws.create_project("Fini", "~/Code/fini");
    let homeless = ws.create_project("Sans dossier", "");
    let mut done = String::new();
    for id in [&ready, &finished, &homeless] {
        let pr = ws.page_mut(id).and_then(|p| p.project.as_mut()).unwrap();
        pr.add_step("Relire", "moi");
        if *id == finished {
            done = pr.steps[0].id.clone();
            pr.set_step_status(&done, crate::project::StepStatus::Done);
        }
    }
    assert!(!done.is_empty());
    let list = super::projects_with_open_steps(&ws);
    assert_eq!(list, [(ready, "Prêt".to_string(), 1)]);
}

/// A project without code (absent from `to_main`) gets neither « commits » nor « sur main »:
/// its controls come out narrower than a code project's.
#[test]
fn a_project_without_code_shows_no_git_switches() {
    let width_of = |code: bool| {
        let mut w = 0.0;
        column(2000.0, |ui| {
            let t = super::tone(false);
            let lk = super::super::settings::look(false);
            let id = "p".to_string();
            let mut d = super::Draft {
                mission: String::new(),
                ceiling: Some(50.0),
                touched: false,
                commits: true,
                endless: false,
                end: Default::default(),
                agent: super::Agent::ClaudeCode, auto: false, model: String::new(),
                saving: Default::default(),
            };
            let mut to_main: super::HashMap<String, bool> = super::HashMap::new();
            if code {
                to_main.insert(id.clone(), false);
            }
            let r = ui.horizontal(|ui| {
                super::launch_options(
                    ui,
                    &id,
                    &mut d,
                    false,
                    &[super::Agent::ClaudeCode],
                    &mut to_main,
                    &lk,
                    &t,
                );
            });
            w = r.response.rect.width();
        });
        w
    };
    let (code, nocode) = (width_of(true), width_of(false));
    // The switch, « commits », the box and « sur main »: well over 100 px.
    assert!(code - nocode > 100.0, "code {code} px, sans code {nocode} px");
}

/// « IA : Automatique » takes the cheapest model able to do the step among the AIs with
/// room; on equal cost the one with the most room; none with room: nothing picked.
#[test]
fn the_automatic_ai_is_the_cheapest_with_room() {
    use super::{Agent, AutoCandidate, pick_auto};
    use crate::catalog::{ModelInfo, Tier};
    let m = |id: &str, tier: Tier, cost: u32| ModelInfo {
        id: id.into(),
        label: id.into(),
        needs_higher_plan: false,
        tier,
        cost_cents: cost,
    };
    let c = |agent, room: f32, week_full, models| AutoCandidate { agent, room, week_full, models };
    let claude = || vec![m("haiku", Tier::Light, 1), m("sonnet", Tier::Mid, 5), m("opus", Tier::High, 20)];
    let codex = || vec![m("mini", Tier::Mid, 3), m("gpt", Tier::High, 15)];
    // A medium step: Codex's mini is the cheapest able to do it.
    let (a, why) = pick_auto(vec![c(Agent::ClaudeCode, 30.0, false, claude()), c(Agent::Codex, 30.0, false, codex())], Tier::Mid).unwrap();
    assert_eq!(a, Agent::Codex);
    assert!(why.contains("mini"), "{why}");
    // Codex out of room: Claude's sonnet.
    let (a, _) = pick_auto(vec![c(Agent::ClaudeCode, 30.0, false, claude()), c(Agent::Codex, 0.5, false, codex())], Tier::Mid).unwrap();
    assert_eq!(a, Agent::ClaudeCode);
    // Its week's reserve near: left out too.
    let (a, _) = pick_auto(vec![c(Agent::ClaudeCode, 30.0, false, claude()), c(Agent::Codex, 50.0, true, codex())], Tier::Mid).unwrap();
    assert_eq!(a, Agent::ClaudeCode);
    // Same cost on both: the one with the most room.
    let same = || vec![m("x", Tier::Mid, 5)];
    let (a, _) = pick_auto(vec![c(Agent::ClaudeCode, 10.0, false, same()), c(Agent::Codex, 40.0, false, same())], Tier::Mid).unwrap();
    assert_eq!(a, Agent::Codex);
    // No model list anywhere: the most room.
    let (a, _) = pick_auto(vec![c(Agent::Vibe, 40.0, false, vec![]), c(Agent::Codex, 10.0, false, vec![])], Tier::Mid).unwrap();
    assert_eq!(a, Agent::Vibe);
    // Nobody with room.
    assert!(pick_auto(vec![c(Agent::ClaudeCode, 0.0, false, claude())], Tier::Mid).is_none());
}

/// The case reported: the 5-hour window already at 75 % when a session starts with a
/// budget of 90 %; it then rises to 80 %. The session's card reads « 5 % utilisés sur
/// un budget de 90 % »: its own use since it started, not the window's 80 %.
#[test]
fn a_session_shows_its_own_use_not_the_window() {
    let reset = Some(1_000_000u64);
    // First reading after the start: the 75 % already there are nobody's.
    let mut pct = 0.0;
    let mut prev: Option<(f32, Option<u64>)> = None;
    for (window, tokens) in [(75.0, 0u64), (78.0, 3_000), (80.0, 2_000)] {
        pct += super::split_rise(super::window_rise(prev, window, reset), &[tokens])[0];
        prev = Some((window, reset));
    }
    assert!((pct - 5.0).abs() < 0.01, "{pct}");
    // Two sessions: the rise shared by their tokens.
    let s = super::split_rise(4.0, &[3_000, 1_000]);
    assert!((s[0] - 3.0).abs() < 0.01 && (s[1] - 1.0).abs() < 0.01);
    // A new window: counted from zero; an unknown reset time on both sides: the difference.
    assert_eq!(
        super::window_rise(Some((75.0, reset)), 2.0, Some(1_000_000 + 5 * 3_600_000)),
        2.0
    );
    assert_eq!(super::window_rise(Some((10.0, None)), 12.0, None), 2.0);
}

/// The bar of a running session is the 5-hour window itself. The case reported: 75 %
/// used before it, 5 % since, a budget of 90 % and a global limit of 90 %: the window
/// reads 80 %, and it stops at the global limit (90 %), well before its budget.
#[test]
fn a_session_bar_is_the_five_hour_window() {
    use super::{WindowBar, window_bar};
    assert_eq!(
        window_bar(80.0, 5.0, 90.0, None, 90.0),
        WindowBar {
            before: 75.0,
            window: 80.0,
            stop: 90.0
        }
    );
    // A small budget stops it first.
    assert_eq!(
        window_bar(80.0, 5.0, 10.0, None, 90.0),
        WindowBar {
            before: 75.0,
            window: 80.0,
            stop: 85.0
        }
    );
    // Its share can't start below zero, nor anything go past 100.
    assert_eq!(
        window_bar(3.0, 5.0, 20.0, None, 100.0),
        WindowBar {
            before: 0.0,
            window: 3.0,
            stop: 20.0
        }
    );
    assert_eq!(window_bar(120.0, 5.0, 50.0, None, 200.0).stop, 100.0);
    // Its own ceiling (absolute): 37 % at its start and a budget of 13 %: the tick at
    // 50 %, even when others made the window rise (40 %, 3 % of it its own).
    assert_eq!(window_bar(40.0, 3.0, 13.0, Some(50.0), 90.0).stop, 50.0);
}

/// The ceiling of a session is absolute: what the window held at its start plus its
/// budget (37 % + 13 % → 50 %), never past the global limit; it stops the session when
/// the window gets there, whoever used it.
#[test]
fn a_session_stops_at_its_absolute_ceiling() {
    use super::{Stop, ceiling_from, stop_reason};
    assert_eq!(ceiling_from(Some(37.0), 0.0, 13.0, 90.0), Some(50.0));
    assert_eq!(ceiling_from(Some(85.0), 0.0, 13.0, 90.0), Some(90.0));
    assert_eq!(ceiling_from(None, 0.0, 13.0, 90.0), None);
    // Figure known only later: what it used meanwhile is its own, not « before ».
    assert_eq!(ceiling_from(Some(41.0), 4.0, 13.0, 90.0), Some(50.0));
    let mut t = calm();
    (t.ceiling, t.pct, t.limit, t.global) = (Some(50.0), 5.0, 13.0, 90.0);
    t.five = Some(49.0);
    assert_eq!(stop_reason(&t), None);
    // Another session pushed the window to 50 %: this one stops too, under its budget.
    t.five = Some(50.0);
    assert_eq!(stop_reason(&t), Some(Stop::Ceiling(50.0)));
    assert!(Stop::Ceiling(50.0).text().contains("50 %"));
    // A ceiling at the global limit: the global limit's stop (restarted after the reset).
    (t.ceiling, t.global, t.five) = (Some(90.0), 90.0, Some(90.0));
    assert_eq!(stop_reason(&t), Some(Stop::GlobalLimit(90.0)));
    // Unknown window: its budget still stops it.
    (t.ceiling, t.five, t.pct) = (None, None, 13.0);
    assert_eq!(stop_reason(&t), Some(Stop::Budget(13.0)));
}

#[test]
fn a_failed_reading_keeps_the_last_figures_until_their_reset() {
    use super::{Figures, Stop, stop_reason};
    let now = 1_000_000;
    let last = Figures {
        five: Some(58.0),
        week: Some(40.0),
        five_reset: Some(now + 3_600_000),
        week_reset: Some(now + 86_400_000),
    };
    // The reading fails: the last figures stay (never 0 %), and the ceiling still stops.
    let kept = last.merged(None, now);
    assert_eq!(kept, last);
    let mut t = calm();
    (t.five, t.week, t.global) = (kept.five, kept.week, 50.0);
    assert_eq!(stop_reason(&t), Some(Stop::GlobalLimit(50.0)));
    // A partial reading: what it brings replaces, what it misses is kept.
    let part = Figures { five: Some(62.0), five_reset: Some(now + 3_000_000), ..Default::default() };
    let m = last.merged(Some(part), now);
    assert_eq!((m.five, m.five_reset), (Some(62.0), Some(now + 3_000_000)));
    assert_eq!((m.week, m.week_reset), (last.week, last.week_reset));
    // Once a window has reset, its old figure is forgotten, not kept.
    let later = now + 4_000_000;
    let m = last.merged(None, later);
    assert_eq!((m.five, m.five_reset), (None, None));
    assert_eq!(m.week, Some(40.0));
    // Without a reset time, nothing tells when it goes stale: forgotten.
    let no_reset = Figures { five: Some(30.0), ..Default::default() };
    assert_eq!(no_reset.merged(None, now), Figures::default());
    // The next reading that succeeds wins, even lower (the window started again).
    let fresh = Figures { five: Some(2.0), five_reset: Some(now + 18_000_000), ..Default::default() };
    assert_eq!(last.merged(Some(fresh), now).five, Some(2.0));
}

/// The three sessions stopped « impossible à mesurer » (20 and 21 November): Orchestra's
/// clock was 8 to 38 minutes ahead, their start was stamped on it, and every reading of
/// the status line (real time) came before it. The start is on the real clock now, so a
/// reading taken after it counts even while Orchestra's clock is ahead.
#[test]
fn a_session_is_measured_while_the_clock_runs_ahead() {
    use super::session_start;
    // On the real clock, whatever Orchestra's own clock says (pushing it ahead here would
    // disturb the other tests, which share it): between two readings of the real time.
    let before = crate::model::wall_ms();
    let start = session_start();
    let after = crate::model::wall_ms();
    assert!(before <= start && start <= after, "{before} {start} {after}");
    // The status line stamps its readings on the same clock: one taken after the start
    // is never before it, so the session is measured.
    assert!(crate::model::wall_ms() >= start);
}

/// A session measured, then the readings fail for a while, then come back: as `usage_tick`
/// goes (figures merged with the last known ones, the rise counted from the last reading
/// that succeeded, the stop decided on both).
#[test]
fn a_failed_measure_neither_stops_the_session_nor_lets_it_pass_the_ceiling() {
    use super::{Figures, Stop, next_rise, stop_reason};
    let reset = 10_000_000;
    let win = |pct: f32| crate::usage::Window { label: "5h".into(), pct, resets_at: Some(reset) };
    let fig = |pct: f32| Figures { five: Some(pct), five_reset: Some(reset), ..Default::default() };
    let mut last = None;
    let mut shown = Figures::default();
    let mut pct = 0.0; // the session's own use (it is alone: every rise is its own)
    let mut take = |at: u64, reading: Option<f32>, shown: &mut Figures, pct: &mut f32| {
        *shown = shown.merged(reading.map(fig), at);
        if let Some(p) = reading
            && let Some(rise) = next_rise(&mut last, at, &win(p))
        {
            *pct += rise;
        }
    };
    let turn = |shown: &Figures, pct: f32, turns: u32| {
        let mut t = calm();
        (t.five, t.week, t.pct, t.limit, t.global, t.turns) = (shown.five, None, pct, 20.0, 60.0, turns);
        stop_reason(&t)
    };
    // Started at 30 %, measured up to 38 %.
    take(1_000, Some(30.0), &mut shown, &mut pct);
    take(2_000, Some(38.0), &mut shown, &mut pct);
    assert!((pct - 8.0).abs() < 0.01);
    // The readings fail over several turns: the session goes on, on the last known figure.
    for (i, at) in (3_000..60_000).step_by(3_000).enumerate() {
        take(at, None, &mut shown, &mut pct);
        assert_eq!(shown.five, Some(38.0), "never 0 %");
        assert_eq!(turn(&shown, pct, 3 + i as u32), None, "a failed measure doesn't stop it");
    }
    // Meanwhile the window went to 52 %: the next reading counts all of it, so the
    // budget (20 % of the window) stops the session, nothing lost while it failed.
    take(61_000, Some(52.0), &mut shown, &mut pct);
    assert!((pct - 22.0).abs() < 0.01, "{pct}");
    assert_eq!(turn(&shown, pct, 30), Some(Stop::Budget(20.0)));
    // And the global limit holds on the kept figure alone, readings failing.
    let mut t = calm();
    let kept = fig(61.0).merged(None, 70_000);
    (t.five, t.pct) = (kept.five, 1.0);
    assert_eq!(stop_reason(&t), Some(Stop::GlobalLimit(60.0)));
}

#[test]
fn an_unavailable_measure_is_told_clearly() {
    use super::{MEASURE_GRACE_MS, measure_alert};
    let since = 1_000_000;
    let later = since + MEASURE_GRACE_MS;
    // Read fine, or only just missed: nothing said.
    assert!(measure_alert(None, Some(40.0), true, later).is_none());
    assert!(measure_alert(Some(since), Some(40.0), true, later - 1).is_none());
    // Nothing rests on it (no session, no figure kept): the tile says « unknown » already.
    assert!(measure_alert(Some(since), None, false, later).is_none());
    // A figure kept: said, with that figure, never as 0 %.
    let (title, text) = measure_alert(Some(since), Some(42.4), false, later).unwrap();
    assert!(title.contains("indisponible") || title.contains("unavailable"), "{title}");
    // It names the AI whose figures are missing: another AI's are on their own.
    assert!(title.contains("Claude Code"), "{title}");
    assert!(text.contains("(42 %)"), "{text}");
    // Sessions running with no figure at all: they stop rather than run without a limit.
    let (_, text) = measure_alert(Some(since), None, true, later + 120_000).unwrap();
    assert!(text.contains("deuxième tour") || text.contains("second turn"), "{text}");
}

#[test]
fn each_session_is_stopped_on_its_own_agents_subscription() {
    use super::{Agent, Figures, Stop, agent_shares, stop_reason};
    // A rise of Codex's window goes to the Codex sessions only, by their tokens.
    let sessions = [(Agent::ClaudeCode, 500), (Agent::Codex, 300), (Agent::Codex, 100)];
    let s = agent_shares(&sessions, Agent::Codex, 8.0);
    assert_eq!(s[0], None);
    assert!((s[1].unwrap() - 6.0).abs() < 0.01 && (s[2].unwrap() - 2.0).abs() < 0.01);
    let s = agent_shares(&sessions, Agent::ClaudeCode, 4.0);
    assert!((s[0].unwrap() - 4.0).abs() < 0.01 && s[1].is_none() && s[2].is_none());
    // Claude Code's window full, Codex's not: the Codex session goes on.
    let claude = Figures { five: Some(80.0), ..Default::default() };
    let codex = Figures { five: Some(10.0), week: Some(5.0), ..Default::default() };
    let end = |f: Figures| {
        let mut t = calm();
        (t.five, t.week) = (f.five, f.week);
        stop_reason(&t)
    };
    assert_eq!(end(claude), Some(Stop::GlobalLimit(60.0)));
    assert_eq!(end(codex), None);
    // An agent that tells no figures: never measured, so it stops at its second turn.
    let mut t = calm();
    (t.five, t.week, t.measured, t.turns) = (None, None, false, 2);
    assert_eq!(stop_reason(&t), Some(Stop::Unmeasured));
    // And none is ever started alone without its figures.
    for a in Agent::ALL {
        assert!(!a.ready() || crate::usage::reports_usage(a), "{a:?}");
    }
    // Their limit messages end the session on the spot.
    for m in [
        "You've hit your usage limit. Try again in 2 hours.",
        "[API Error: Quota exceeded for quota metric 'Gemini 2.5 Pro Requests']",
        "RESOURCE_EXHAUSTED",
        "Error: Rate limit exceeded (429)",
    ] {
        assert!(crate::auto::is_limit_message(m), "{m}");
    }
}

/// A turn that ends with nothing to stop on: under every limit, measured, few turns.
fn calm() -> super::TurnEnd {
    super::TurnEnd {
        asked: None,
        too_long: false,
        endless_end: None,
        signal: super::Signal::Continue,
        blocked_why: String::new(),
        endless: false,
        pct: 2.0,
        limit: 20.0,
        ceiling: None,
        five: Some(30.0),
        global: 60.0,
        week: Some(40.0),
        week_limit: 95.0,
        week_turn: 1.0,
        measured: true,
        turns: 3,
        max_turns: 30,
        spent_micro: 0,
        budget_micro: None,
    }
}

/// An API-mode session stops once its estimated spend reaches its dollar budget, and one
/// with no budget (subscription) never stops on dollars.
#[test]
fn a_session_stops_at_its_dollar_budget() {
    use super::{Stop, stop_reason};
    let mut t = calm();
    t.spent_micro = 9_000_000;
    assert_eq!(stop_reason(&t), None);
    t.budget_micro = Some(5_000_000);
    assert_eq!(stop_reason(&t), Some(Stop::Dollars(5_000_000)));
    t.spent_micro = 4_990_000;
    assert_eq!(stop_reason(&t), None);
    let s = crate::model::OrchSettings::default();
    assert_eq!(s.budget_micro(false), None);
    assert_eq!(s.budget_micro(true), Some(5_000_000));
}

#[test]
fn a_session_goes_on_or_stops_for_the_first_reason() {
    use super::{Quota, Signal, Stop, TurnEnd, stop_reason};
    assert_eq!(stop_reason(&calm()), None);
    let with = |f: &dyn Fn(&mut TurnEnd)| {
        let mut t = calm();
        f(&mut t);
        stop_reason(&t)
    };
    assert_eq!(
        with(&|t| t.asked = Some("arrêtée à la main".into())),
        Some(Stop::Asked("arrêtée à la main".into()))
    );
    assert_eq!(with(&|t| t.too_long = true), Some(Stop::TooLong));
    assert_eq!(with(&|t| t.signal = Signal::Done), Some(Stop::MissionDone));
    assert_eq!(
        with(&|t| {
            t.signal = Signal::Blocked;
            t.blocked_why = "il manque la clé".into();
        }),
        Some(Stop::Blocked("il manque la clé".into()))
    );
    assert!(
        Stop::Blocked("il manque la clé".into())
            .text()
            .ends_with("il manque la clé")
    );
    assert_eq!(with(&|t| t.pct = 20.0), Some(Stop::Budget(20.0)));
    assert_eq!(
        with(&|t| t.five = Some(60.0)),
        Some(Stop::GlobalLimit(60.0))
    );
    assert_eq!(with(&|t| t.week = Some(94.5)), Some(Stop::WeekReserve(5.0)));
    assert_eq!(with(&|t| t.measured = false), Some(Stop::Unmeasured));
    assert_eq!(with(&|t| t.turns = 30), Some(Stop::Turns(30)));
    // Unmeasured only from the second turn; unknown figures stop nothing.
    assert_eq!(
        with(&|t| {
            t.measured = false;
            t.turns = 1
        }),
        None
    );
    assert_eq!(
        with(&|t| {
            t.five = None;
            t.week = None
        }),
        None
    );
    // Endless: neither [[FINI]] nor the turn limit, its own end conditions instead.
    assert_eq!(
        with(&|t| {
            t.endless = true;
            t.signal = Signal::Done;
            t.turns = 500
        }),
        None
    );
    assert_eq!(
        with(&|t| {
            t.endless = true;
            t.endless_end = Some("4 h écoulées".into())
        }),
        Some(Stop::EndlessEnd("4 h écoulées".into()))
    );
    // The order: a stop asked comes before all, the budget before the global limit…
    assert_eq!(
        with(&|t| {
            t.asked = Some("x".into());
            t.too_long = true;
            t.pct = 99.0
        }),
        Some(Stop::Asked("x".into()))
    );
    assert_eq!(
        with(&|t| {
            t.pct = 20.0;
            t.five = Some(90.0)
        }),
        Some(Stop::Budget(20.0))
    );
    assert_eq!(
        with(&|t| {
            t.five = Some(90.0);
            t.week = Some(99.0)
        }),
        Some(Stop::GlobalLimit(60.0))
    );
    // Only the quotas start again by themselves, each when its own window resets.
    assert_eq!(Stop::GlobalLimit(60.0).quota(), Quota::FiveHours);
    assert_eq!(Stop::WeekReserve(5.0).quota(), Quota::Week);
    assert_eq!(Stop::Budget(20.0).quota(), Quota::None);
    assert_eq!(Stop::MissionDone.quota(), Quota::None);
    assert!(Stop::GlobalLimit(60.0).text().contains("60"));
}

/// Steps counted by id: renamed, twins and new ones each count once, and right.
#[test]
fn a_pause_past_the_cache_life_starts_a_new_conversation() {
    use super::{CACHE_LIFE_MS, fresh_after_pause};
    // A few minutes, or just under the hour: the cache still holds the history.
    assert!(!fresh_after_pause(5 * 60 * 1000));
    assert!(!fresh_after_pause(CACHE_LIFE_MS));
    // Past it (a night, a Mac asleep): a new conversation.
    assert!(fresh_after_pause(CACHE_LIFE_MS + 1));
    assert!(fresh_after_pause(8 * 60 * 60 * 1000));
}

#[test]
fn steps_since_the_start_go_by_id() {
    use crate::project::{Project, StepStatus};
    let mut p = Project::default();
    let a = p.add_step("Écrire les tests", "moi");
    let b = p.add_step("Doublon", "moi");
    let c = p.add_step("Doublon", "moi");
    let d = p.add_step("Déjà faite", "moi");
    p.set_step_status(&d, StepStatus::Done);
    let before: Vec<(String, bool)> = p
        .steps
        .iter()
        .map(|s| (s.id.clone(), s.status == StepStatus::Done))
        .collect();
    assert_eq!(
        super::steps_since(&before, &p.steps),
        super::StepsSince {
            done: vec![],
            added_open: 0
        }
    );
    // During the session: A renamed then done, one of the twins done, a new step added
    // and another added then done.
    p.steps.iter_mut().find(|s| s.id == a).unwrap().title =
        "Écrire les tests (unitaires)".into();
    p.set_step_status(&a, StepStatus::Done);
    p.set_step_status(&c, StepStatus::Done);
    p.add_step("Nouvelle étape", "claude-code");
    let e = p.add_step("Nouvelle et faite", "claude-code");
    p.set_step_status(&e, StepStatus::Done);
    let s = super::steps_since(&before, &p.steps);
    assert_eq!(
        s.done,
        vec![
            "Écrire les tests (unitaires)".to_string(),
            "Doublon".into(),
            "Nouvelle et faite".into()
        ]
    );
    // Only the new open step: the renamed one is not new, the twin still open is not new.
    assert_eq!(s.added_open, 1);
    assert!(
        p.steps
            .iter()
            .any(|x| x.id == b && x.status == StepStatus::Todo)
    );
    // The person adds many steps meanwhile (typed, or AI proposals accepted, or received
    // from a teammate): they never count as the AI piling up steps, so the endless
    // session goes on.
    for i in 0..10 {
        p.add_step(&format!("À moi {i}"), crate::project::SOURCE_PERSON);
        p.add_step(&format!("Proposée {i}"), crate::project::SOURCE_ACCEPTED);
    }
    let r = p.add_step("Reçue", "autre");
    p.steps.iter_mut().find(|x| x.id == r).unwrap().received = true;
    let s = super::steps_since(&before, &p.steps);
    assert_eq!(s.added_open, 1);
    assert!(!crate::auto::too_many_self_steps(s.added_open));
    // An AI adding through MCP (default source « ia ») still counts.
    for i in 0..7 {
        p.add_step(&format!("Ajoutée par l'IA {i}"), "ia");
    }
    assert!(crate::auto::too_many_self_steps(
        super::steps_since(&before, &p.steps).added_open
    ));
}

/// The step shown as « busy with »: doing, ours or unreserved, the latest changed.
#[test]
fn current_step_is_the_doing_one_of_this_computer() {
    use crate::project::{Project, StepStatus};
    let mut p = Project::default();
    let a = p.add_step("A faire", "moi");
    assert!(super::current_step(&p.steps, "ici").is_none());
    let b = p.add_step("Chez un autre", "moi");
    p.set_step_status(&b, StepStatus::Doing);
    p.steps.iter_mut().find(|s| s.id == b).unwrap().held_by = "ailleurs".into();
    assert!(
        super::current_step(&p.steps, "ici").is_none(),
        "reserved by another computer"
    );
    p.set_step_status(&a, StepStatus::Doing);
    p.steps.iter_mut().find(|s| s.id == a).unwrap().held_by = "ici".into();
    let title = |p: &Project| super::current_step(&p.steps, "ici").map(|s| s.title.clone());
    assert_eq!(title(&p).as_deref(), Some("A faire"));
    // Sent on one step, that one is shown whatever the others are; gone, none.
    let on = |id: &str| super::session_step(id, &p.steps, "ici").map(|s| s.id.clone());
    assert_eq!(on(&b), Some(b.clone()));
    assert_eq!(on(""), Some(a.clone()));
    assert_eq!(on("disparue"), None);
    let c = p.add_step("Plus récente", "moi");
    p.set_step_status(&c, StepStatus::Doing);
    p.steps.iter_mut().find(|s| s.id == c).unwrap().updated = u64::MAX;
    assert_eq!(title(&p).as_deref(), Some("Plus récente"));
    p.set_step_status(&c, StepStatus::Done);
    p.set_step_status(&a, StepStatus::Done);
    assert!(super::current_step(&p.steps, "ici").is_none());
}

/// Steps the person adds during a turn reach the AI on the next one, even after a [[FINI]].
#[test]
fn steps_added_during_a_turn_are_sent_on() {
    use crate::project::{Project, StepStatus};
    let mut p = Project::default();
    p.add_step("Ancienne", crate::project::SOURCE_PERSON);
    let at_turn: Vec<String> = p.steps.iter().map(|s| s.id.clone()).collect();
    assert!(super::fresh_steps(&at_turn, &p.steps).is_empty());
    assert_eq!(super::new_steps_note(&[]), "");
    // During the turn: the person adds two, one gets done already; the AI adds its own.
    p.add_step("Nouvelle à faire", crate::project::SOURCE_PERSON);
    let d = p.add_step("Nouvelle déjà faite", crate::project::SOURCE_PERSON);
    p.set_step_status(&d, StepStatus::Done);
    p.add_step("Proposée et acceptée", crate::project::SOURCE_ACCEPTED);
    p.add_step("De l'IA", "claude-code");
    let fresh = super::fresh_steps(&at_turn, &p.steps);
    assert_eq!(
        fresh,
        vec![
            "Nouvelle à faire".to_string(),
            "Proposée et acceptée".into()
        ]
    );
    let note = super::new_steps_note(&fresh);
    assert!(note.contains("Nouvelle à faire") && note.contains('2'));
    // The next turn starts from what is there now: nothing new any more.
    let at_turn: Vec<String> = p.steps.iter().map(|s| s.id.clone()).collect();
    assert!(super::fresh_steps(&at_turn, &p.steps).is_empty());
}

#[test]
fn the_responder_analysis_is_split_from_the_question() {
    let (q, a) = super::split_analysis("Quoi faire ?\n\n— Analyse du répondeur :\nOption C.");
    assert_eq!((q, a), ("Quoi faire ?", Some("Option C.")));
    assert_eq!(super::split_analysis("Sans analyse"), ("Sans analyse", None));
}

#[test]
fn a_step_session_says_what_becomes_of_its_step() {
    use super::{Stop, StepIssue, step_issue};
    let done = Stop::MissionDone.text();
    assert_eq!(step_issue(&done, false, false), Some(StepIssue::Finished));
    // A question still waits: not finished on its word.
    assert_eq!(step_issue(&done, false, true), Some(StepIssue::Interrupted));
    let why = Stop::Blocked("il manque la clé".into()).text();
    assert_eq!(
        step_issue(&why, false, true),
        Some(StepIssue::Blocked("il manque la clé".into()))
    );
    let silent = step_issue(&Stop::Blocked(String::new()).text(), false, false);
    assert!(matches!(silent, Some(StepIssue::Blocked(w)) if !w.is_empty()));
    let limit = Stop::GlobalLimit(60.0).text();
    assert_eq!(step_issue(&limit, true, false), None, "it starts again");
    assert_eq!(step_issue(&limit, false, false), Some(StepIssue::Interrupted));
}

#[test]
fn a_blocked_ai_comes_with_a_button_while_it_is_of_use() {
    use super::{Unblock, Wait, inbox_unblock, waiting_title};
    use crate::model::{InboxMsg, RunRecord};
    let run = |id: &str, started, outcome: &str| RunRecord {
        id: id.into(),
        project: "p".into(),
        started,
        outcome: outcome.into(),
        ..Default::default()
    };
    let msg = |kind: &str, at, title: String| InboxMsg {
        kind: kind.into(),
        project: "p".into(),
        at,
        title,
        ..Default::default()
    };
    // Paused: only the newest message of the pause, and only while it lasts.
    let runs = vec![run("r1", 1, "")];
    let inbox = vec![
        msg("info", 2, waiting_title("Projet")),
        msg("info", 3, waiting_title("Projet")),
    ];
    let paused = vec![("r1".to_string(), "Projet".to_string(), Wait::Permission)];
    assert_eq!(inbox_unblock(&inbox[0], &inbox, &runs, None, &paused), None);
    assert_eq!(
        inbox_unblock(&inbox[1], &inbox, &runs, None, &paused),
        Some(Unblock::Terminal("r1".into()))
    );
    assert_eq!(inbox_unblock(&inbox[1], &inbox, &runs, None, &[]), None);
    assert_eq!(Unblock::of_wait(Wait::Question, "r1"), Unblock::Answer);
    // Ended blocked: the question to answer if one waits, else the project's steps.
    let why = super::Stop::Blocked("il manque la clé".into()).text();
    let mut runs = vec![run("r1", 1, &why)];
    let end = msg("fin", 4, "fin".into());
    let of = |inbox: &[InboxMsg], runs: &[RunRecord]| {
        inbox_unblock(&end, inbox, runs, Some(&runs[0]), &[])
    };
    assert_eq!(of(&[], &runs), Some(Unblock::Steps("p".into())));
    let asked = [msg("question", 2, "?".into())];
    assert_eq!(of(&asked, &runs), Some(Unblock::Answer));
    // A later session on the project: the block is old news.
    runs.push(run("r2", 5, ""));
    assert_eq!(of(&asked, &runs), None);
    // Stopped for another reason: no button.
    let runs = vec![run("r1", 1, &super::Stop::MissionDone.text())];
    assert_eq!(of(&asked, &runs), None);
}

#[test]
fn end_of_session_message_reads_in_everyday_words() {
    let end = |why: &str, steps, saved, unsaved, pct, resume_in: Option<&str>| {
        super::end_message(&super::SessionEnd {
            project: "Site",
            why,
            steps,
            saved,
            unsaved,
            pct,
            resume_in: resume_in.map(String::from),
        })
    };
    // The mission done: said by the title, no reason to repeat.
    let done = super::Stop::MissionDone.text();
    assert_eq!(
        end(&done, 2, 1, 0, 12.4, None),
        (
            "L'IA a terminé son travail sur Site".to_string(),
            "2 étapes terminées, 1 modification enregistrée.\nPart de votre abonnement utilisée : environ 12 %.".to_string()
        )
    );
    // Stopped otherwise: the reason first; an unknown share isn't shown as 0 %.
    let (title, text) = end("l'IA est bloquée.", 0, 0, 3, 0.0, Some("2 h"));
    assert_eq!(title, "L'IA s'est arrêtée sur Site");
    assert_eq!(
        text,
        "Raison : l'IA est bloquée.\nElle reprendra toute seule dans 2 h, quand votre abonnement le permettra à nouveau.\nAucune étape terminée.\n3 fichiers modifiés mais pas encore enregistrés."
    );
}

#[test]
fn inbox_text_drops_markdown_marks() {
    let s = "### Fait\n- **une** chose\n\n\n### Reste\n- rien\n\n";
    assert_eq!(super::plain_text(s), "Fait\n- une chose\n\nReste\n- rien");
}
