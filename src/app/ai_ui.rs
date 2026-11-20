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
    pub tracking: bool,
}

impl Ais {
    /// True once the check is done and no AI is plugged in: the round AI button shows then.
    pub fn none_plugged(&self) -> bool {
        self.states.lock().unwrap().as_ref().is_some_and(|s| !s.iter().any(|x| x.plugged))
    }

    pub fn plugged(&self) -> Vec<Ai> {
        self.states.lock().unwrap().as_ref().map(|s| s.iter().filter(|x| x.plugged).map(|x| x.ai).collect()).unwrap_or_default()
    }

    /// Buttons for "Nouvelle session": the plugged agents that run in a terminal, then the shell.
    pub fn launchers(&self) -> Vec<(&'static str, Option<&'static str>, Color32)> {
        let mut out: Vec<(&'static str, Option<&'static str>, Color32)> = self
            .plugged()
            .into_iter()
            .filter_map(|a| {
                let (r, g, b) = a.accent();
                a.launch().map(|c| (a.label(), Some(c), Color32::from_rgb(r, g, b)))
            })
            .collect();
        out.push(("Shell", None, Color32::GRAY));
        out
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
            *report.lock().unwrap() = None;
        }
        std::thread::spawn(move || {
            if plug {
                let exe = std::env::current_exe().unwrap_or_default();
                let r = crate::setup::plug_all(&exe, &off);
                *report.lock().unwrap() = Some(r);
            }
            *states.lock().unwrap() = Some(crate::setup::states());
            ctx.request_repaint();
        });
    }

    /// Plug (or unplug) one AI, then check them all again.
    pub(super) fn set_ai(&mut self, ctx: &egui::Context, ai: Ai, on: bool) {
        if on {
            self.ws.settings.ai_off.retain(|k| k != ai.key());
        } else if !self.ws.settings.ai_off.iter().any(|k| k == ai.key()) {
            self.ws.settings.ai_off.push(ai.key().to_string());
        }
        self.dirty_since.get_or_insert_with(Instant::now);
        let (states, busy, msg) = (self.ais.states.clone(), self.ais.busy.clone(), self.ais.done_msg.clone());
        *busy.lock().unwrap() = Some(ai.key().to_string());
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let text = if on {
                let exe = std::env::current_exe().unwrap_or_default();
                match crate::setup::plug(ai, &exe) {
                    crate::setup::Outcome::Failed(e) => Err(crate::trf!("{} : branchement impossible ({e}).", "{}: couldn't plug it in ({e}).", ai.label())),
                    _ if ai == Ai::ClaudeDesktop => Ok(crate::trf!("{} est branchée : redémarrez l'app Claude pour qu'elle le voie.", "{} is plugged in: restart the Claude app so it sees it.", ai.label())),
                    _ => Ok(crate::trf!("{} est branché au tableau de bord.", "{} is plugged into the dashboard.", ai.label())),
                }
            } else {
                match crate::setup::unplug(ai) {
                    Ok(()) => Ok(crate::trf!("{} est débranché.", "{} is unplugged.", ai.label())),
                    Err(e) => Err(crate::trf!("{} : impossible de débrancher ({e}).", "{}: couldn't unplug it ({e}).", ai.label())),
                }
            };
            *states.lock().unwrap() = Some(crate::setup::states());
            *busy.lock().unwrap() = None;
            *msg.lock().unwrap() = Some(text);
            ctx.request_repaint();
        });
    }

    /// Once per frame: messages of finished actions; usage read again every 20 s.
    pub(super) fn ais_tick(&mut self) {
        let done = self.ais.done_msg.lock().unwrap().take();
        if let Some(m) = done {
            self.notice(super::notices::Notice::from_result(m));
        }
        if self.ais.usage_read.is_none_or(|t| t.elapsed().as_secs() >= 20) {
            self.ais.usage_read = Some(Instant::now());
            self.ais.usage = crate::usage::read();
            self.ais.tracking = crate::usage::tracking_on();
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

/// "repart dans 2 h 14": the time left before the 5-hour window resets, for the header of
/// the subscription tile. None when that time isn't known or has passed.
fn five_reset_in(u: &crate::usage::Usage, now: u64) -> Option<String> {
    let left = crate::usage::until(u.five()?.resets_at?, now)?;
    Some(crate::trf!("repart dans {}", "resets in {}", crate::usage::left_text(left)))
}

#[cfg(test)]
mod reset_tests {
    use crate::usage::{Usage, Window};

    #[test]
    fn header_shows_time_left_before_the_five_hour_reset() {
        let now = 1_000_000_000u64;
        let win = |label: &str, resets_at: Option<u64>| Window { label: label.into(), pct: 40.0, resets_at };
        let u = Usage { windows: vec![win("Session (5 h)", Some(now + (2 * 60 + 14) * 60_000)), win("Semaine", Some(now + 3 * 86_400_000))], at: now };
        let s = super::five_reset_in(&u, now).unwrap();
        assert!(s.ends_with("2 h 14"), "{s}");
        // Passed, unknown, or no 5-hour window: nothing (the reading's age is shown instead).
        assert!(super::five_reset_in(&u, now + 3 * 3_600_000).is_none());
        let u = Usage { windows: vec![win("Session (5 h)", None)], at: now };
        assert!(super::five_reset_in(&u, now).is_none());
        let u = Usage { windows: vec![win("Semaine", Some(now + 1))], at: now };
        assert!(super::five_reset_in(&u, now).is_none());
    }
}

impl NotesApp {
    /// Bottom of the sidebar: the Claude subscription at a glance; details on hover.
    pub(super) fn usage_widget(&mut self, ui: &mut egui::Ui, t: &Tone) {
        let plugged = self.ais.plugged().contains(&Ai::ClaudeCode);
        let usage = self.ais.usage.clone();
        if usage.is_none() && !plugged && !self.ais.tracking {
            return;
        }
        ui.add_space(6.0);
        let w = ui.available_width();
        let shown: Vec<crate::usage::Window> = usage.as_ref().map(|u| u.windows.iter().take(2).cloned().collect()).unwrap_or_default();
        let h = if shown.is_empty() { 34.0 } else { 26.0 + 20.0 * shown.len() as f32 };
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
        let hov = ui.ctx().animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.15);
        let p = ui.painter();
        p.rect_filled(rect, 10.0, t.hover.gamma_multiply(0.4 + 0.6 * hov));
        p.rect_stroke(rect, 10.0, egui::Stroke::new(1.0, t.border.lerp_to_gamma(CLAUDE, 0.35 * hov)), egui::StrokeKind::Inside);
        // Header: a small mark, "Claude", and the time since the last reading.
        let top = rect.top() + 14.0;
        p.circle_filled(egui::pos2(rect.left() + 16.0, top), 4.0, CLAUDE);
        p.text(egui::pos2(rect.left() + 28.0, top), egui::Align2::LEFT_CENTER, crate::tr!("Abonnement Claude", "Claude subscription"), semibold(12.5), t.text);
        if shown.is_empty() {
            let (text, color) = if self.ais.tracking { (crate::tr!("en attente…", "waiting…"), t.tertiary) } else { (crate::tr!("Afficher", "Show"), super::projects::BLUE) };
            p.text(egui::pos2(rect.right() - 12.0, top), egui::Align2::RIGHT_CENTER, text, FontId::proportional(12.0), color);
        } else if let Some(u) = &usage {
            // The time left before the 5-hour window starts again; the reading's age when
            // that time isn't known (or has passed: the next reading brings the new one).
            let head = five_reset_in(u, crate::model::now_ms()).unwrap_or_else(|| super::chrome::relative_time(u.at));
            p.text(egui::pos2(rect.right() - 12.0, top), egui::Align2::RIGHT_CENTER, head, FontId::proportional(11.0), t.tertiary);
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(30));
        }
        for (i, win) in shown.iter().enumerate() {
            let y = rect.top() + 34.0 + 20.0 * i as f32;
            let short = if win.label.starts_with("Session") { "5 h".to_string() } else { win.label.replace("Semaine", crate::tr!("Sem.", "Wk")) };
            p.text(egui::pos2(rect.left() + 12.0, y), egui::Align2::LEFT_CENTER, short, FontId::proportional(11.5), t.secondary);
            let pct_x = rect.right() - 12.0;
            let color = level_color(win.pct);
            p.text(egui::pos2(pct_x, y), egui::Align2::RIGHT_CENTER, format!("{:.0} %", win.pct), semibold(11.5), color);
            let bar = egui::Rect::from_min_max(egui::pos2(rect.left() + 58.0, y - 2.5), egui::pos2(pct_x - 40.0, y + 2.5));
            let frac = ui.ctx().animate_value_with_time(resp.id.with(("bar", i)), win.pct / 100.0, 0.8);
            p.rect_filled(bar, 2.5, t.fill);
            p.rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac, bar.height())), 2.5, color);
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

/// "Abonnement Claude  Session (5 h) ▓▓░ 23 % · 15:40   Semaine ▓░░ 41 % · lun." in one line.
pub(super) fn usage_strip(ui: &mut egui::Ui, u: &crate::usage::Usage, t: &Tone) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(crate::tr!("Abonnement Claude", "Claude subscription")).size(12.5).color(t.tertiary));
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
                ui.label(RichText::new(crate::usage::shown(&w.label)).size(12.5).color(t.secondary));
                let (bar, _) = ui.allocate_exact_size(egui::vec2(56.0, 6.0), egui::Sense::hover());
                ui.painter().rect_filled(bar, 3.0, t.fill);
                ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * w.pct / 100.0, 6.0)), 3.0, color);
                ui.label(RichText::new(format!("{:.0} %", w.pct)).size(12.5).color(color));
            });
            let reset = w.resets_at.map(|r| crate::trf!("Repart à zéro {}", "Resets {}", when(r))).unwrap_or_default();
            resp.response.on_hover_text(crate::trf!("{reset}\nChiffres de Claude Code, relevés {}", "{reset}\nClaude Code's figures, read {}", super::chrome::relative_time(u.at)));
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
        let out = std::process::Command::new("date").arg("+%z").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
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
