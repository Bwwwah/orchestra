//! The AIs on this machine as the window sees them: which are installed and plugged into the
//! dashboard (checked in the background), the buttons that plug or unplug them, and the use
//! of the Claude subscription.

use super::*;
use crate::setup::{Ai, AiState};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Ais {
    /// None while the first check runs.
    pub states: Arc<Mutex<Option<Vec<AiState>>>>,
    /// An AI being plugged or unplugged (key), and the message to show when it's done.
    pub busy: Arc<Mutex<Option<String>>>,
    pub done_msg: Arc<Mutex<Option<Result<String, String>>>>,
    pub usage: Option<crate::usage::Usage>,
    pub usage_read: Option<Instant>,
    /// The other plugged AIs (not Claude Code) and their figures when Orchestra can read
    /// them (Codex, Vibe's monthly budget); None for those that tell none (Gemini) or before
    /// a first reading. Read with `usage`.
    pub others: Vec<(crate::auto::Agent, Option<crate::usage::Usage>)>,
    /// Mistral Vibe's plan and tokens, when it is among `others`.
    pub vibe: Option<crate::usage::VibeUse>,
    pub tracking: bool,
    /// What RTK saved over 7 days for each AI, computed in the background (see `rtk_tick`).
    pub rtk_week: Arc<Mutex<Vec<(crate::auto::Agent, crate::rtk_stats::WeekSaving)>>>,
    pub rtk_read: Option<Instant>,
    /// An install started from the settings (built-in terminal): the AI, when it started,
    /// and the last check of whether it is there yet. Plugged in as soon as it is.
    pub installing: Option<(Ai, Instant, Instant)>,
}

impl Ais {
    /// True once the check is done and no AI is plugged in: the round AI button shows then.
    pub fn none_plugged(&self) -> bool {
        self.states
            .lock_or_recover()
            .as_ref()
            .is_some_and(|s| !s.iter().any(|x| x.plugged))
    }

    pub fn plugged(&self) -> Vec<Ai> {
        self.states
            .lock_or_recover()
            .as_ref()
            .map(|s| s.iter().filter(|x| x.plugged).map(|x| x.ai).collect())
            .unwrap_or_default()
    }

    /// The AIs of « Nouvelle session » (« Sessions IA » card): see `launchers_from`.
    pub fn launchers(&self) -> Vec<(&'static str, Option<&'static str>, Color32)> {
        launchers_from(self.states.lock_or_recover().as_deref().unwrap_or(&[]))
    }
}

/// The plugged AIs whose quota is shown besides Claude's subscription: the agents other than
/// Claude Code, in the setup list's order (never an app such as Cursor or Claude Desktop).
pub fn other_quota_agents(plugged: &[Ai]) -> Vec<crate::auto::Agent> {
    use crate::auto::Agent;
    Agent::ALL
        .into_iter()
        .filter(|a| *a != Agent::ClaudeCode && plugged.contains(&a.ai()))
        .collect()
}

#[cfg(test)]
mod other_quota_tests {
    use super::{Ai, other_quota_agents};
    use crate::auto::Agent;

    #[test]
    fn only_the_other_plugged_agents_get_a_quota_tile() {
        let plugged = [Ai::Vibe, Ai::ClaudeCode, Ai::Cursor, Ai::Codex];
        assert_eq!(other_quota_agents(&plugged), [Agent::Codex, Agent::Vibe]);
        assert!(other_quota_agents(&[Ai::ClaudeCode, Ai::ClaudeDesktop]).is_empty());
        assert!(other_quota_agents(&[]).is_empty());
    }
}

/// The AIs of the setup list installed on this machine that open in a terminal, in the order
/// of that list (plugged into the dashboard or not: one can work without Orchestra's MCP),
/// then the shell. While the first check runs (`states` empty), only the shell.
pub fn launchers_from(states: &[AiState]) -> Vec<(&'static str, Option<&'static str>, Color32)> {
    let mut out: Vec<(&'static str, Option<&'static str>, Color32)> = Ai::ALL
        .into_iter()
        .filter(|a| states.iter().any(|s| s.ai == *a && s.installed))
        .filter_map(|a| {
            let (r, g, b) = a.accent();
            a.launch()
                .map(|c| (a.label(), Some(c), Color32::from_rgb(r, g, b)))
        })
        .collect();
    out.push(("Shell", None, Color32::GRAY));
    out
}

#[cfg(test)]
mod launcher_tests {
    use super::{Ai, AiState, launchers_from};

    #[test]
    fn only_the_installed_ais_that_open_in_a_terminal_are_offered() {
        let st = |ai, installed, plugged| AiState { ai, installed, plugged, install: None };
        let states = [
            st(Ai::Vibe, true, false),
            st(Ai::Gemini, false, false),
            st(Ai::ClaudeCode, true, true),
            st(Ai::Cursor, true, true),
            st(Ai::Codex, true, false),
        ];
        let labels: Vec<&str> = launchers_from(&states).iter().map(|l| l.0).collect();
        // The setup list's order; installed even if not plugged; never an app (Cursor) nor
        // an AI that isn't there (Gemini); the shell last.
        assert_eq!(labels, ["Claude Code", "Codex", "Mistral Vibe", "Shell"]);
        let l = launchers_from(&states);
        assert_eq!(l[0].1, Some("claude"));
        assert_eq!(l.last().unwrap().1, None);
        // Before the first check: the shell only.
        assert_eq!(launchers_from(&[]).len(), 1);
    }
}

impl NotesApp {
    /// Check every AI again, in the background (and plug them first when `plug`).
    pub(super) fn refresh_ais(&mut self, ctx: &egui::Context, plug: bool) {
        let states = self.ais.states.clone();
        let report = self.mcp_report.clone();
        let off = self.ws.settings.ai_off.clone();
        let ctx = ctx.clone();
        self.mcp_report_shown = !plug;
        if plug {
            *report.lock_or_recover() = None;
        }
        std::thread::spawn(move || {
            if plug {
                let exe = std::env::current_exe().unwrap_or_default();
                let r = crate::setup::plug_all(&exe, &off);
                *report.lock_or_recover() = Some(r);
            }
            *states.lock_or_recover() = Some(crate::setup::states());
            ctx.request_repaint();
        });
    }

    /// Start installing `ai` (its command runs in the built-in terminal): from now on the
    /// AIs are checked again every few seconds, and `ai` is plugged in once it is there.
    pub(super) fn watch_install(&mut self, ai: Ai) {
        let now = Instant::now();
        self.ais.installing = Some((ai, now, now));
    }

    /// While an install runs: check every few seconds whether the AI is there; plug it in
    /// as soon as it is. Gives up after a while (install cancelled or failed).
    pub(super) fn install_tick(&mut self, ctx: &egui::Context) {
        use std::time::Duration;
        const EVERY: Duration = Duration::from_secs(5);
        const GIVE_UP: Duration = Duration::from_secs(20 * 60);
        let Some((ai, started, last)) = self.ais.installing else {
            return;
        };
        let there = self
            .ais
            .states
            .lock_or_recover()
            .as_ref()
            .and_then(|s| s.iter().find(|x| x.ai == ai).map(|x| x.installed));
        if there == Some(true) {
            self.ais.installing = None;
            self.set_ai(ctx, ai, true);
            self.info(crate::trf!(
                "{} est installée et branchée à Orchestra.",
                "{} is installed and plugged into Orchestra.",
                ai.label()
            ));
            return;
        }
        if started.elapsed() > GIVE_UP {
            self.ais.installing = None;
            return;
        }
        // Once the first check is in (`there` known), one every few seconds.
        if last.elapsed() >= EVERY && there.is_some() {
            self.ais.installing = Some((ai, started, Instant::now()));
            self.refresh_ais(ctx, false);
        }
        ctx.request_repaint_after(EVERY);
    }

    /// Plug (or unplug) one AI, then check them all again.
    pub(super) fn set_ai(&mut self, ctx: &egui::Context, ai: Ai, on: bool) {
        if on {
            self.ws.settings.ai_off.retain(|k| k != ai.key());
        } else if !self.ws.settings.ai_off.iter().any(|k| k == ai.key()) {
            self.ws.settings.ai_off.push(ai.key().to_string());
        }
        self.dirty_since.get_or_insert_with(Instant::now);
        let (states, busy, msg) = (
            self.ais.states.clone(),
            self.ais.busy.clone(),
            self.ais.done_msg.clone(),
        );
        *busy.lock_or_recover() = Some(ai.key().to_string());
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let text = if on {
                let exe = std::env::current_exe().unwrap_or_default();
                match crate::setup::plug(ai, &exe) {
                    crate::setup::Outcome::Failed(e) => Err(crate::trf!(
                        "{} : branchement impossible ({e}).",
                        "{}: couldn't plug it in ({e}).",
                        ai.label()
                    )),
                    _ if ai == Ai::ClaudeDesktop => Ok(crate::trf!(
                        "{} est branchée : redémarrez l'app Claude pour qu'elle le voie.",
                        "{} is plugged in: restart the Claude app so it sees it.",
                        ai.label()
                    )),
                    _ => Ok(crate::trf!(
                        "{} est branché au tableau de bord.",
                        "{} is plugged into the dashboard.",
                        ai.label()
                    )),
                }
            } else {
                match crate::setup::unplug(ai) {
                    Ok(()) => Ok(crate::trf!(
                        "{} est débranché.",
                        "{} is unplugged.",
                        ai.label()
                    )),
                    Err(e) => Err(crate::trf!(
                        "{} : impossible de débrancher ({e}).",
                        "{}: couldn't unplug it ({e}).",
                        ai.label()
                    )),
                }
            };
            *states.lock_or_recover() = Some(crate::setup::states());
            *busy.lock_or_recover() = None;
            *msg.lock_or_recover() = Some(text);
            ctx.request_repaint();
        });
    }

    /// Once per frame: messages of finished actions; usage read again every 20 s.
    pub(super) fn ais_tick(&mut self) {
        let done = self.ais.done_msg.lock_or_recover().take();
        if let Some(m) = done {
            self.notice(super::notices::Notice::from_result(m));
        }
        if self
            .ais
            .usage_read
            .is_none_or(|t| t.elapsed().as_secs() >= 20)
        {
            self.ais.usage_read = Some(Instant::now());
            self.ais.usage = crate::usage::read();
            self.ais.others = other_quota_agents(&self.ais.plugged())
                .into_iter()
                .map(|a| (a, crate::usage::read_for(a)))
                .collect();
            self.ais.vibe = self
                .ais
                .others
                .iter()
                .any(|(a, _)| *a == crate::auto::Agent::Vibe)
                .then(crate::usage::read_vibe)
                .flatten();
            self.ais.tracking = crate::usage::tracking_on();
            // Our status line left pointing to an Orchestra that is gone (moved, updated, or
            // built in a deleted copy): it measures nothing, so the sessions would stop as
            // unmeasured. Put this one back in its place.
            if self.ais.tracking
                && crate::usage::tracking_broken()
                && let Err(e) =
                    crate::usage::start_tracking(&std::env::current_exe().unwrap_or_default())
            {
                eprintln!("orchestra: status line not repaired: {e}");
            }
        }
    }

    pub(super) fn set_tracking(&mut self, on: bool) {
        let r = if on {
            crate::usage::start_tracking(&std::env::current_exe().unwrap_or_default())
        } else {
            crate::usage::stop_tracking()
        };
        let n = match r {
            Ok(()) if on => super::notices::Notice::Info(crate::tr!("Suivi activé : les chiffres arrivent dès que Claude Code répond (dans le terminal intégré ou ailleurs).", "Tracking on: the figures arrive as soon as Claude Code answers (in the built-in terminal or elsewhere).").into()),
            Ok(()) => super::notices::Notice::Info(crate::tr!("Suivi désactivé, votre ligne d'état d'origine est revenue.", "Tracking off, your original status line is back.").into()),
            Err(e) => super::notices::Notice::Error(crate::trf!("Impossible de changer la ligne d'état de Claude Code : {e}", "Couldn't change Claude Code's status line: {e}")),
        };
        self.notice(n);
        self.ais.usage_read = None;
    }
}

fn level_color(pct: f32) -> Color32 {
    if pct >= 90.0 {
        Color32::from_rgb(224, 62, 62)
    } else if pct >= 70.0 {
        Color32::from_rgb(217, 115, 13)
    } else {
        Color32::from_rgb(68, 131, 97)
    }
}

const CLAUDE: Color32 = Color32::from_rgb(204, 120, 92);

/// "repart dans 2 h 14": the time left before the 5-hour window (or a budget's month) resets, for the header of
/// the subscription tile. None when that time isn't known or has passed.
fn five_reset_in(u: &crate::usage::Usage, now: u64) -> Option<String> {
    let left = crate::usage::until(u.measured()?.resets_at?, now)?;
    Some(crate::trf!(
        "repart dans {}",
        "resets in {}",
        crate::usage::left_text(left)
    ))
}

#[cfg(test)]
mod reset_tests {
    use crate::usage::{Usage, Window};

    /// The Mistral tile says on itself why it has no percentage, in one line that fits the
    /// narrowest sidebar (220 px, less its margins and the text's indent).
    #[test]
    fn the_mistral_tile_says_why_it_has_no_percentage_in_one_line() {
        let ctx = eframe::egui::Context::default();
        super::super::setup_fonts(&ctx);
        let waiting = crate::usage::VibeUse::default();
        let failed = crate::usage::VibeUse {
            budget_problem: Some("Mistral a répondu 500.".into()),
            ..Default::default()
        };
        assert_ne!(super::vibe_no_pct_note(&waiting), super::vibe_no_pct_note(&failed));
        for v in [&waiting, &failed] {
            let mut width = 0.0;
            for _ in 0..2 {
                ctx.begin_pass(Default::default());
                width = ctx.fonts_mut(|f| {
                    f.layout_no_wrap(
                        super::vibe_no_pct_note(v).to_string(),
                        eframe::egui::FontId::proportional(10.5),
                        eframe::egui::Color32::WHITE,
                    )
                    .size()
                    .x
                });
                ctx.end_pass().textures_delta.clear();
            }
            assert!(width > 0.0 && width <= 220.0 - 20.0 - 24.0, "{width}");
        }
        // Its monthly budget is read like Claude's and Codex's windows, never "not readable".
        assert!(crate::usage::unreadable_reason(crate::auto::Agent::Vibe).is_none());
    }

    #[test]
    fn header_shows_time_left_before_the_month_of_a_budget_resets() {
        let now = 1_000_000_000u64;
        let u = Usage {
            windows: vec![Window {
                label: crate::usage::MONTH.into(),
                pct: 12.0,
                resets_at: Some(now + 3 * 86_400_000 + 5 * 3_600_000),
            }],
            at: now,
        };
        let s = super::five_reset_in(&u, now).unwrap();
        assert!(s.ends_with("3 j 5 h") || s.ends_with("3 d 5 h"), "{s}");
    }

    #[test]
    fn header_shows_time_left_before_the_five_hour_reset() {
        let now = 1_000_000_000u64;
        let win = |label: &str, resets_at: Option<u64>| Window {
            label: label.into(),
            pct: 40.0,
            resets_at,
        };
        let u = Usage {
            windows: vec![
                win("Session (5 h)", Some(now + (2 * 60 + 14) * 60_000)),
                win("Semaine", Some(now + 3 * 86_400_000)),
            ],
            at: now,
        };
        let s = super::five_reset_in(&u, now).unwrap();
        assert!(s.ends_with("2 h 14"), "{s}");
        // Passed, unknown, or no 5-hour window: nothing (the reading's age is shown instead).
        assert!(super::five_reset_in(&u, now + 3 * 3_600_000).is_none());
        let u = Usage {
            windows: vec![win("Session (5 h)", None)],
            at: now,
        };
        assert!(super::five_reset_in(&u, now).is_none());
        let u = Usage {
            windows: vec![win("Semaine", Some(now + 1))],
            at: now,
        };
        assert!(super::five_reset_in(&u, now).is_none());
    }
}

impl NotesApp {
    /// Bottom of the sidebar: the Claude subscription, then the quota of every other plugged
    /// AI (Codex, Gemini, Mistral Vibe…), each in its own tile; details on hover.
    pub(super) fn usage_widget(&mut self, ui: &mut egui::Ui, t: &Tone) {
        self.rtk_tick(ui.ctx());
        self.claude_usage_tile(ui, t);
        for (agent, usage) in self.ais.others.clone() {
            let vibe = self.ais.vibe.as_ref().filter(|_| agent == crate::auto::Agent::Vibe);
            let rtk = self.rtk_line_of(agent);
            if other_usage_tile(ui, agent, usage.as_ref(), vibe, rtk, t).clicked() {
                self.open_settings(SettingsTab::Ai);
            }
        }
    }

    /// Every minute, in the background: what RTK saved this week for each AI.
    fn rtk_tick(&mut self, ctx: &egui::Context) {
        if self.ais.rtk_read.is_some_and(|t| t.elapsed().as_secs() < 60) {
            return;
        }
        self.ais.rtk_read = Some(Instant::now());
        let sessions = self.agents.all();
        let out = self.ais.rtk_week.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = crate::rtk_stats::week_savings_all(&sessions, crate::model::now_ms());
            *out.lock_or_recover() = r;
            ctx.request_repaint();
        });
    }

    /// The tile line « ≈ X % de consommation économisée grâce à RTK… » of an AI, if it has one.
    fn rtk_line_of(&self, agent: crate::auto::Agent) -> Option<String> {
        let all = self.ais.rtk_week.lock_or_recover();
        let (_, w) = all.iter().find(|(a, _)| *a == agent)?;
        crate::rtk_stats::saving_line(w)
    }

    /// The Claude subscription at a glance.
    fn claude_usage_tile(&mut self, ui: &mut egui::Ui, t: &Tone) {
        let plugged = self.ais.plugged().contains(&Ai::ClaudeCode);
        let usage = self.ais.usage.clone();
        if usage.is_none() && !plugged && !self.ais.tracking {
            return;
        }
        ui.add_space(6.0);
        let w = ui.available_width();
        let shown: Vec<crate::usage::Window> = usage
            .as_ref()
            .map(|u| u.windows.iter().take(2).cloned().collect())
            .unwrap_or_default();
        let h = if shown.is_empty() {
            34.0
        } else {
            26.0 + 20.0 * shown.len() as f32
        };
        let rtk = self.rtk_line_of(crate::auto::Agent::ClaudeCode);
        let note = rtk_note(ui, rtk, w, t);
        let h = h + note.as_ref().map_or(0.0, |g| g.size().y + 6.0);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
        let hov = ui
            .ctx()
            .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.15);
        let p = ui.painter();
        p.rect_filled(rect, 10.0, t.hover.gamma_multiply(0.4 + 0.6 * hov));
        p.rect_stroke(
            rect,
            10.0,
            egui::Stroke::new(1.0, t.border.lerp_to_gamma(CLAUDE, 0.35 * hov)),
            egui::StrokeKind::Inside,
        );
        draw_rtk_note(p, rect, note);
        // Header: a small mark, "Claude", and the time since the last reading.
        let top = rect.top() + 14.0;
        p.circle_filled(egui::pos2(rect.left() + 16.0, top), 4.0, CLAUDE);
        p.text(
            egui::pos2(rect.left() + 28.0, top),
            egui::Align2::LEFT_CENTER,
            crate::auto::Agent::ClaudeCode.short_name(),
            semibold(12.5),
            t.text,
        );
        if shown.is_empty() {
            let (text, color) = if self.ais.tracking {
                (crate::tr!("en attente…", "waiting…"), t.tertiary)
            } else {
                (crate::tr!("Afficher", "Show"), super::projects::BLUE)
            };
            p.text(
                egui::pos2(rect.right() - 12.0, top),
                egui::Align2::RIGHT_CENTER,
                text,
                FontId::proportional(12.0),
                color,
            );
        } else if let Some(u) = &usage {
            // The time left before the 5-hour window starts again; the reading's age when
            // that time isn't known (or has passed: the next reading brings the new one).
            let head = five_reset_in(u, crate::model::now_ms())
                .unwrap_or_else(|| super::chrome::relative_time(u.at));
            p.text(
                egui::pos2(rect.right() - 12.0, top),
                egui::Align2::RIGHT_CENTER,
                head,
                FontId::proportional(11.0),
                t.tertiary,
            );
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(30));
        }
        for (i, win) in shown.iter().enumerate() {
            let y = rect.top() + 34.0 + 20.0 * i as f32;
            let short = if win.label.starts_with("Session") {
                "5 h".to_string()
            } else {
                win.label.replace("Semaine", crate::tr!("Sem.", "Wk"))
            };
            p.text(
                egui::pos2(rect.left() + 12.0, y),
                egui::Align2::LEFT_CENTER,
                short,
                FontId::proportional(11.5),
                t.secondary,
            );
            let pct_x = rect.right() - 12.0;
            let color = level_color(win.pct);
            p.text(
                egui::pos2(pct_x, y),
                egui::Align2::RIGHT_CENTER,
                format!("{:.0} %", win.pct),
                semibold(11.5),
                color,
            );
            let bar = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 58.0, y - 2.5),
                egui::pos2(pct_x - 40.0, y + 2.5),
            );
            let frac =
                ui.ctx()
                    .animate_value_with_time(resp.id.with(("bar", i)), win.pct / 100.0, 0.8);
            p.rect_filled(bar, 2.5, t.fill);
            p.rect_filled(
                egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac, bar.height())),
                2.5,
                color,
            );
        }
        let resp = resp.on_hover_ui(|ui| {
            ui.set_min_width(240.0);
            match &usage {
                None if self.ais.tracking => {
                    ui.label(crate::tr!("Les chiffres arrivent à la prochaine réponse de Claude Code.", "The figures arrive with Claude Code's next answer."));
                }
                None => {
                    ui.label(crate::tr!("Cliquer pour afficher ici l'utilisation de votre abonnement Claude.", "Click to show the use of your Claude subscription here."));
                    ui.label(RichText::new(crate::tr!("Orchestra devient la ligne d'état de Claude Code ; la vôtre continue de s'afficher. Aucun jeton n'est lu.", "Orchestra becomes Claude Code's status line; yours keeps showing. No token is read.")).size(12.0).color(t.tertiary));
                }
                Some(u) => {
                    ui.label(RichText::new(crate::tr!("Abonnement Claude", "Claude subscription")).font(semibold(14.0)));
                    ui.add_space(4.0);
                    for win in &u.windows {
                        let color = level_color(win.pct);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(crate::usage::shown(&win.label)).size(13.0));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(format!("{:.0} %", win.pct)).font(semibold(13.0)).color(color));
                            });
                        });
                        let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), egui::Sense::hover());
                        ui.painter().rect_filled(bar, 3.0, t.fill);
                        ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * win.pct / 100.0, 6.0)), 3.0, color);
                        let left = 100.0 - win.pct;
                        let reset = win.resets_at.map(|r| crate::trf!(" · repart à zéro {}", " · resets {}", when(r))).unwrap_or_default();
                        ui.label(RichText::new(format!("{left:.0} % restant{reset}")).size(12.0).color(t.tertiary));
                        ui.add_space(6.0);
                    }
                    ui.label(RichText::new(crate::trf!("Relevé par Claude Code {}", "Read from Claude Code {}", super::chrome::relative_time(u.at))).size(11.5).color(t.tertiary));
                }
            }
        });
        if resp.clicked() {
            if usage.is_none() && !self.ais.tracking {
                self.set_tracking(true);
            } else {
                self.open_settings(SettingsTab::Ai);
            }
        }
    }
}

/// The RTK line of a tile, laid out to fit the tile's width (it wraps).
fn rtk_note(
    ui: &egui::Ui,
    line: Option<String>,
    width: f32,
    t: &Tone,
) -> Option<std::sync::Arc<egui::Galley>> {
    let line = line?;
    Some(ui.painter().layout(
        line,
        FontId::proportional(10.5),
        t.tertiary,
        (width - 24.0).max(40.0),
    ))
}

/// Draw the RTK line at the bottom of a tile.
fn draw_rtk_note(p: &egui::Painter, rect: egui::Rect, note: Option<std::sync::Arc<egui::Galley>>) {
    if let Some(g) = note {
        let pos = egui::pos2(rect.left() + 12.0, rect.bottom() - g.size().y - 6.0);
        p.galley(pos, g, Color32::PLACEHOLDER);
    }
}

/// "Abonnement Codex", "Quota Gemini": the title of an agent's tile.
pub(super) fn quota_title(agent: crate::auto::Agent) -> String {
    if crate::usage::reports_usage(agent) {
        crate::trf!("Abonnement {}", "{} subscription", agent.label())
    } else {
        crate::trf!("Quota {}", "{} quota", agent.label())
    }
}

/// Mistral Vibe's plan and tokens, in the tile's hover and the settings.
pub(super) fn vibe_details(ui: &mut egui::Ui, v: &crate::usage::VibeUse, t: &Tone) {
    if let Some(plan) = &v.plan {
        ui.label(RichText::new(crate::trf!("Forfait : {}", "Plan: {}", plan)).size(13.0));
    }
    ui.label(
        RichText::new(crate::trf!(
            "Jetons utilisés : {} sur 24 h, {} sur 7 jours",
            "Tokens used: {} over 24 h, {} over 7 days",
            crate::usage::tokens_short(v.tokens_day),
            crate::usage::tokens_short(v.tokens_week)
        ))
        .size(13.0),
    );
    if v.at > 0 {
        ui.label(
            RichText::new(crate::trf!(
                "Dernière session {}",
                "Last session {}",
                super::chrome::relative_time(v.at)
            ))
            .size(11.5)
            .color(t.tertiary),
        );
    }
    ui.add_space(2.0);
}

/// The Mistral tile's last line while its monthly budget isn't shown: why (the hover says more).
fn vibe_no_pct_note(v: &crate::usage::VibeUse) -> &'static str {
    if v.budget_problem.is_some() {
        crate::tr!("pas de % : budget non lu (survol)", "no %: budget not read (hover)")
    } else {
        crate::tr!("% du budget : demandé à Mistral…", "budget %: asking Mistral…")
    }
}

/// Where a reading of the monthly budget comes from and how old it is, or why it failed.
pub(super) fn vibe_budget_note(ui: &mut egui::Ui, v: &crate::usage::VibeUse, t: &Tone) {
    let text = match &v.budget_problem {
        Some(why) => crate::trf!(
            "Budget mensuel de Vibe non lu : {}",
            "Vibe's monthly budget not read: {}",
            why
        ),
        None => crate::tr!(
            "Le budget mensuel de Vibe est demandé à Mistral avec la connexion de Vibe, comme Vibe le fait pour le % en bas de son écran.",
            "Vibe's monthly budget is asked to Mistral with Vibe's sign-in, as Vibe does for the % at the bottom of its screen."
        )
        .to_string(),
    };
    ui.label(RichText::new(text).size(12.0).color(t.secondary));
}

/// A plugged AI other than Claude Code, in the sidebar: its windows (5 h, week) when Orchestra
/// reads them, "en attente…" until a first reading, or "non lisible" with the reason on hover.
fn other_usage_tile(
    ui: &mut egui::Ui,
    agent: crate::auto::Agent,
    usage: Option<&crate::usage::Usage>,
    vibe: Option<&crate::usage::VibeUse>,
    rtk: Option<String>,
    t: &Tone,
) -> egui::Response {
    let (r, g, b) = agent.ai().accent();
    let accent = Color32::from_rgb(r, g, b);
    ui.add_space(6.0);
    let w = ui.available_width();
    let shown: Vec<crate::usage::Window> = usage
        .map(|u| u.windows.iter().take(2).cloned().collect())
        .unwrap_or_default();
    let h = if shown.is_empty() && vibe.is_some() {
        62.0
    } else if shown.is_empty() {
        28.0
    } else {
        26.0 + 20.0 * shown.len() as f32
    };
    let note = rtk_note(ui, rtk, w, t);
    let h = h + note.as_ref().map_or(0.0, |g| g.size().y + 6.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
    let hov = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.15);
    let p = ui.painter();
    p.rect_filled(rect, 10.0, t.hover.gamma_multiply(0.4 + 0.6 * hov));
    p.rect_stroke(
        rect,
        10.0,
        egui::Stroke::new(1.0, t.border.lerp_to_gamma(accent, 0.35 * hov)),
        egui::StrokeKind::Inside,
    );
    draw_rtk_note(p, rect, note);
    let top = rect.top() + 14.0;
    p.circle_filled(egui::pos2(rect.left() + 16.0, top), 4.0, accent);
    p.text(
        egui::pos2(rect.left() + 28.0, top),
        egui::Align2::LEFT_CENTER,
        agent.short_name(),
        semibold(12.5),
        t.text,
    );
    let head = match (usage, vibe) {
        (Some(u), _) => five_reset_in(u, crate::model::now_ms())
            .unwrap_or_else(|| super::chrome::relative_time(u.at)),
        (None, Some(v)) => v.plan.clone().unwrap_or_default(),
        (None, None) if crate::usage::reports_usage(agent) => crate::tr!("en attente…", "waiting…").into(),
        (None, None) => crate::tr!("non lisible", "not readable").into(),
    };
    if let (true, Some(v)) = (shown.is_empty(), vibe) {
        // No share of a limit to show: the tokens used, over a day and a week.
        p.text(
            egui::pos2(rect.left() + 12.0, rect.top() + 33.0),
            egui::Align2::LEFT_CENTER,
            crate::trf!(
                "24 h : {} jetons · 7 j : {}",
                "24 h: {} tokens · 7 d: {}",
                crate::usage::tokens_short(v.tokens_day),
                crate::usage::tokens_short(v.tokens_week)
            ),
            FontId::proportional(11.5),
            t.secondary,
        );
        // Why there is no percentage like Claude's, said on the tile, not only on hover.
        p.text(
            egui::pos2(rect.left() + 12.0, rect.top() + 49.0),
            egui::Align2::LEFT_CENTER,
            vibe_no_pct_note(v),
            FontId::proportional(10.5),
            t.tertiary,
        );
    }
    p.text(
        egui::pos2(rect.right() - 12.0, top),
        egui::Align2::RIGHT_CENTER,
        head,
        FontId::proportional(11.0),
        t.tertiary,
    );
    for (i, win) in shown.iter().enumerate() {
        let y = rect.top() + 34.0 + 20.0 * i as f32;
        let short = if win.label.starts_with("Session") {
            "5 h".to_string()
        } else if win.label == crate::usage::MONTH {
            crate::tr!("Mois", "Month").to_string()
        } else {
            win.label.replace("Semaine", crate::tr!("Sem.", "Wk"))
        };
        p.text(
            egui::pos2(rect.left() + 12.0, y),
            egui::Align2::LEFT_CENTER,
            short,
            FontId::proportional(11.5),
            t.secondary,
        );
        let pct_x = rect.right() - 12.0;
        let color = level_color(win.pct);
        p.text(
            egui::pos2(pct_x, y),
            egui::Align2::RIGHT_CENTER,
            format!("{:.0} %", win.pct),
            semibold(11.5),
            color,
        );
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 58.0, y - 2.5),
            egui::pos2(pct_x - 40.0, y + 2.5),
        );
        p.rect_filled(bar, 2.5, t.fill);
        p.rect_filled(
            egui::Rect::from_min_size(
                bar.min,
                egui::vec2(bar.width() * win.pct / 100.0, bar.height()),
            ),
            2.5,
            color,
        );
    }
    resp.on_hover_ui(|ui| {
        ui.set_min_width(240.0);
        ui.label(RichText::new(quota_title(agent)).font(semibold(14.0)));
        ui.add_space(4.0);
        match (usage, crate::usage::unreadable_reason(agent)) {
            (_, Some(why)) => {
                if let Some(v) = vibe {
                    vibe_details(ui, v, t);
                }
                ui.label(RichText::new(why).size(12.5).color(t.secondary));
            }
            (None, None) if vibe.is_some() => {
                if let Some(v) = vibe {
                    vibe_details(ui, v, t);
                    vibe_budget_note(ui, v, t);
                }
            }
            (None, None) => {
                ui.label(RichText::new(crate::trf!(
                    "Les chiffres arrivent à la prochaine réponse de {} (ils sont lus dans ses fichiers de session).",
                    "The figures arrive with {}'s next answer (read from its session files).",
                    agent.label()
                )).size(12.5).color(t.secondary));
            }
            (Some(u), None) => {
                for win in &u.windows {
                    let reset = win
                        .resets_at
                        .map(|r| crate::trf!(" · repart à zéro {}", " · resets {}", when(r)))
                        .unwrap_or_default();
                    ui.label(
                        RichText::new(format!(
                            "{} : {:.0} %{reset}",
                            crate::usage::shown(&win.label),
                            win.pct
                        ))
                        .size(13.0),
                    );
                }
                let from = if agent == crate::auto::Agent::Vibe {
                    "Mistral".to_string()
                } else {
                    agent.label().to_string()
                };
                ui.label(
                    RichText::new(crate::trf!(
                        "Relevé par {} {}",
                        "Read from {} {}",
                        from,
                        super::chrome::relative_time(u.at)
                    ))
                    .size(11.5)
                    .color(t.tertiary),
                );
                if let Some(v) = vibe {
                    ui.add_space(4.0);
                    vibe_details(ui, v, t);
                }
            }
        }
    })
}

/// "Abonnement Claude  Session (5 h) ▓▓░ 23 % · 15:40   Semaine ▓░░ 41 % · lun." in one line.
/// `title`: whose subscription it is ("Abonnement Claude", "Abonnement Codex").
pub(super) fn usage_strip(ui: &mut egui::Ui, title: &str, u: &crate::usage::Usage, t: &Tone) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(title).size(12.5).color(t.tertiary));
        for w in &u.windows {
            let color = if w.pct >= 90.0 {
                Color32::from_rgb(224, 62, 62)
            } else if w.pct >= 70.0 {
                Color32::from_rgb(217, 115, 13)
            } else {
                Color32::from_rgb(68, 131, 97)
            };
            let resp = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(
                    RichText::new(crate::usage::shown(&w.label))
                        .size(12.5)
                        .color(t.secondary),
                );
                let (bar, _) = ui.allocate_exact_size(egui::vec2(56.0, 6.0), egui::Sense::hover());
                ui.painter().rect_filled(bar, 3.0, t.fill);
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        bar.min,
                        egui::vec2(bar.width() * w.pct / 100.0, 6.0),
                    ),
                    3.0,
                    color,
                );
                ui.label(
                    RichText::new(format!("{:.0} %", w.pct))
                        .size(12.5)
                        .color(color),
                );
            });
            let reset = w
                .resets_at
                .map(|r| crate::trf!("Repart à zéro {}", "Resets {}", when(r)))
                .unwrap_or_default();
            resp.response.on_hover_text(crate::trf!(
                "{reset}\n{title} : chiffres relevés {}",
                "{reset}\n{title}: figures read {}",
                super::chrome::relative_time(u.at)
            ));
        }
    });
}

/// "à 15:40", "demain à 09:00", "le 24/11 à 18:00" (local time).
fn when(ms: u64) -> String {
    let now = crate::model::now_ms();
    let off = local_offset_ms();
    let day = |x: u64| (x as i64 + off).div_euclid(86_400_000);
    let mins = ((ms as i64 + off).rem_euclid(86_400_000)) / 60_000;
    let hm = format!("{:02}:{:02}", mins / 60, mins % 60);
    match day(ms) - day(now) {
        0 => crate::trf!("à {hm}", "at {hm}"),
        1 => crate::trf!("demain à {hm}", "tomorrow at {hm}"),
        _ => {
            let (_, m, d) = civil((ms as i64 + off).div_euclid(86_400_000));
            crate::trf!("le {d:02}/{m:02} à {hm}", "on {m:02}/{d:02} at {hm}")
        }
    }
}

/// Today's local date, "20 novembre 2026" / "November 20, 2026" (the "/" menu's "Date du jour").
pub(super) fn today_label() -> String {
    date_label((crate::model::now_ms() as i64 + local_offset_ms()).div_euclid(86_400_000))
}

fn date_label(days: i64) -> String {
    let (y, m, d) = civil(days);
    let i = (m as usize).clamp(1, 12) - 1;
    const FR: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];
    const EN: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let (fr, en) = (FR[i], EN[i]);
    crate::trf!("{d} {fr} {y}", "{en} {d}, {y}")
}

#[test]
fn date_label_writes_the_day() {
    assert_eq!(civil(20_412), (2025, 11, 20));
    let s = date_label(20_412);
    assert!(s == "20 novembre 2025" || s == "November 20, 2025", "{s}");
}

/// Days since 1970 → (year, month, day).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The local time zone's offset, from `date +%z` (cached).
fn local_offset_ms() -> i64 {
    static OFF: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| {
        let out = std::process::Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let b = out.as_bytes();
        if b.len() == 5 && (b[0] == b'+' || b[0] == b'-') {
            let h: i64 = out[1..3].parse().unwrap_or(0);
            let m: i64 = out[3..5].parse().unwrap_or(0);
            let v = (h * 60 + m) * 60_000;
            if b[0] == b'-' { -v } else { v }
        } else {
            0
        }
    })
}
