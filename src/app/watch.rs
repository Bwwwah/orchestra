//! "VEILLE · CVE" card of a code project: the known vulnerabilities of its dependencies,
//! checked with each ecosystem's audit tool (`crate::audit`) when the person clicks. The
//! last result of each folder is kept while Orchestra runs.

use super::projects::{BLUE, GREEN, ORANGE};
use super::*;
use crate::audit::{EcoReport, Finding, Severity};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Default)]
struct Entry {
    running: bool,
    /// The last check: when (ms) and what it found.
    last: Option<(u64, Vec<EcoReport>)>,
}

type Book = Arc<Mutex<HashMap<String, Entry>>>;

fn book() -> &'static Book {
    static B: OnceLock<Book> = OnceLock::new();
    B.get_or_init(Default::default)
}

/// « Corriger dans une branche fix » clicked: the project and the important findings, taken
/// by the orchestration (`take_fix_request`), which adds the steps and starts the session.
#[derive(Clone, Default)]
pub(super) struct FixRequest {
    pub project: String,
    pub findings: Vec<Finding>,
}

fn fix_key() -> egui::Id {
    egui::Id::unique("cve-fix-request")
}

/// Whether a fix session was asked for and not taken yet.
pub(super) fn fix_pending(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<FixRequest>(fix_key()).is_some())
}

pub(super) fn take_fix_request(ctx: &egui::Context) -> Option<FixRequest> {
    ctx.data_mut(|d| d.remove_temp::<FixRequest>(fix_key()))
}

/// The findings worth a fix session right away: high or critical.
pub(super) fn important(findings: &[&Finding]) -> Vec<Finding> {
    findings
        .iter()
        .filter(|f| f.severity.important())
        .map(|f| (*f).clone())
        .collect()
}

/// One step per vulnerable package, « Thème : action » like the AI's steps.
pub(super) fn fix_steps(findings: &[Finding]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for f in findings {
        let to = f
            .fix
            .as_deref()
            .map(|x| crate::trf!(" ({x})", " ({x})"))
            .unwrap_or_default();
        let s = crate::trf!(
            "Veille CVE : corriger {} dans {}{to}",
            "CVE watch: fix {} in {}{to}",
            f.id,
            f.package
        );
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// The fix session's mission.
pub(super) fn fix_mission(findings: &[Finding]) -> String {
    let list: Vec<String> = findings
        .iter()
        .map(|f| {
            let fix = f.fix.as_deref().map(|x| format!(" → {x}")).unwrap_or_default();
            format!("{} ({} {}{fix})", f.id, f.package, f.version)
        })
        .collect();
    crate::trf!(
        "Corrige les failles de sécurité importantes trouvées dans les dépendances : {}. Mets à jour chaque dépendance vers une version corrigée (en changeant le moins possible), puis vérifie que le projet compile et que ses tests passent. Passe les étapes « Veille CVE » une par une.",
        "Fix the important security vulnerabilities found in the dependencies: {}. Update each dependency to a fixed version (changing as little as possible), then check that the project builds and its tests pass. Go through the « CVE watch » steps one by one.",
        list.join(", ")
    )
}

/// The automatic watch checks each watched project once a day.
pub(super) const DAY_MS: u64 = 24 * 3600 * 1000;

/// Automatic watch: whether a new check is due, given the last check taken into account
/// (`seen`) and the last one made since Orchestra opened (`last`), both in ms.
pub(super) fn check_due(seen: u64, last: Option<u64>, now: u64) -> bool {
    let latest = last.map_or(seen, |l| l.max(seen));
    now.saturating_sub(latest) >= DAY_MS
}

/// The last check of `folder` since Orchestra opened, when none is running: (when, findings
/// important enough for a fix session).
pub(super) fn last_result(folder: &str) -> Option<(u64, Vec<Finding>)> {
    let b = book().lock_or_recover();
    let e = b.get(folder)?;
    if e.running {
        return None;
    }
    let (at, reports) = e.last.as_ref()?;
    Some((*at, important(&all_findings(reports))))
}

/// Whether a check of `folder` is running.
pub(super) fn checking(folder: &str) -> bool {
    book().lock_or_recover().get(folder).is_some_and(|e| e.running)
}

/// Start a check of `folder` (the automatic watch).
pub(super) fn start_check(folder: &str, ctx: &egui::Context) {
    start(folder, ctx);
}

/// The important findings that no step of the project names yet (whatever its status): a
/// fix already started, done or abandoned isn't started again by the automatic watch.
pub(super) fn untreated(findings: &[Finding], steps: &[String]) -> Vec<Finding> {
    findings
        .iter()
        .filter(|f| {
            fix_steps(std::slice::from_ref(*f))
                .iter()
                .any(|s| !steps.contains(s))
        })
        .cloned()
        .collect()
}

/// Ask the orchestration for a fix session (button or automatic watch).
pub(super) fn request_fix(ctx: &egui::Context, req: FixRequest) {
    ctx.data_mut(|d| d.insert_temp(fix_key(), req));
}

const RED: Color32 = Color32::from_rgb(224, 62, 62);
/// Findings listed before « … et N autres ».
const SHOWN: usize = 6;

fn severity_color(s: Severity, t: &Tone) -> Color32 {
    match s {
        Severity::Critical | Severity::High => RED,
        Severity::Moderate => ORANGE,
        Severity::Low => BLUE,
        Severity::Unknown => t.tertiary,
    }
}

/// Start a check of `folder` in the background (nothing when one runs already).
fn start(folder: &str, ctx: &egui::Context) {
    {
        let mut b = book().lock_or_recover();
        let e = b.entry(folder.to_string()).or_default();
        if e.running {
            return;
        }
        e.running = true;
    }
    let (folder, ctx) = (folder.to_string(), ctx.clone());
    std::thread::spawn(move || {
        let reports = crate::audit::check(std::path::Path::new(&folder));
        let mut b = book().lock_or_recover();
        let e = b.entry(folder).or_default();
        e.running = false;
        e.last = Some((crate::model::now_ms(), reports));
        ctx.request_repaint();
    });
}

/// Every finding of a check, the worst first.
fn all_findings(reports: &[EcoReport]) -> Vec<&Finding> {
    let mut v: Vec<&Finding> = reports
        .iter()
        .filter_map(|r| r.result.as_ref().ok())
        .flatten()
        .collect();
    v.sort_by_key(|f| std::cmp::Reverse(f.severity));
    v
}

/// The card's header line: "2 importantes · 5 au total", "aucune faille connue", "".
fn summary(reports: &[EcoReport]) -> String {
    let f = all_findings(reports);
    let checked = reports.iter().any(|r| r.result.is_ok());
    if f.is_empty() {
        return if checked {
            crate::tr!("aucune faille connue", "no known vulnerability").into()
        } else {
            String::new()
        };
    }
    let important = f.iter().filter(|x| x.severity.important()).count();
    let (n, en) = (f.len(), crate::i18n::en());
    if important > 0 {
        if en {
            format!("{important} important · {n} in all")
        } else {
            let s = crate::i18n::plural(important);
            format!("{important} importante{s} · {n} au total")
        }
    } else if en {
        format!("{n} vulnerabilit{}", if n == 1 { "y" } else { "ies" })
    } else {
        format!("{n} faille{}", crate::i18n::plural(n))
    }
}

pub(super) fn watch_card(
    ui: &mut egui::Ui,
    project: &str,
    folder: &str,
    auto: &mut bool,
    t: &Tone,
    extra: f32,
) -> f32 {
    let ctx = ui.ctx().clone();
    let ecos = if folder.is_empty() {
        vec![]
    } else {
        crate::audit::ecosystems(std::path::Path::new(folder))
    };
    let (running, last) = {
        let b = book().lock_or_recover();
        match b.get(folder) {
            Some(e) => (e.running, e.last.clone()),
            None => (false, None),
        }
    };
    let head = last
        .as_ref()
        .map(|(_, r)| summary(r))
        .unwrap_or_default();
    super::projects::card(ui, t, extra, |ui| {
        super::projects::card_title(ui, crate::tr!("VEILLE · CVE", "WATCH · CVE"), &head, t);
        if ecos.is_empty() {
            ui.label(
                RichText::new(crate::tr!(
                    "Aucune dépendance suivie ici : la veille lit Cargo.lock (Rust) et package-lock.json (npm) à la racine du dossier.",
                    "No dependency followed here: the watch reads Cargo.lock (Rust) and package-lock.json (npm) at the folder's root."
                ))
                .size(12.5)
                .color(t.tertiary),
            );
            return;
        }
        match &last {
            None if !running => {
                ui.label(
                    RichText::new(crate::trf!(
                        "Dépendances suivies : {}. Pas encore vérifiées depuis l'ouverture d'Orchestra.",
                        "Dependencies followed: {}. Not checked yet since Orchestra opened.",
                        ecos.iter().map(|e| e.label()).collect::<Vec<_>>().join(", ")
                    ))
                    .size(12.5)
                    .color(t.tertiary),
                );
            }
            None => {}
            Some((_, reports)) => {
                for r in reports {
                    if let Err(e) = &r.result {
                        ui.label(
                            RichText::new(format!("{} — {e}", r.eco.label()))
                                .size(12.5)
                                .color(ORANGE),
                        );
                    }
                }
                let found = all_findings(reports);
                if found.is_empty() && reports.iter().any(|r| r.result.is_ok()) {
                    ui.label(
                        RichText::new(crate::tr!(
                            "✓ Aucune faille connue dans les dépendances.",
                            "✓ No known vulnerability in the dependencies."
                        ))
                        .size(13.0)
                        .color(GREEN),
                    );
                }
                for f in found.iter().take(SHOWN) {
                    finding_row(ui, f, t);
                }
                if found.len() > SHOWN {
                    ui.label(
                        RichText::new(crate::trf!(
                            "… et {} autres",
                            "… and {} more",
                            found.len() - SHOWN
                        ))
                        .size(12.0)
                        .color(t.tertiary),
                    );
                }
                let urgent = important(&found);
                if !urgent.is_empty() {
                    ui.add_space(4.0);
                    let r = ui.add(egui::Button::new(
                        RichText::new(crate::tr!(
                            "Corriger dans une branche fix",
                            "Fix in a fix branch"
                        ))
                        .size(13.0)
                        .color(RED),
                    ));
                    if r
                        .on_hover_text(crate::trf!(
                            "Ajoute une étape « Veille CVE » par faille importante ({}), puis lance une session autonome sur une branche à part (orchestra/auto-fix-…) qui met à jour les dépendances et vérifie le projet. Rien n'arrive sur main sans votre fusion.",
                            "Adds a « CVE watch » step per important vulnerability ({}), then starts an autonomous session on a branch of its own (orchestra/auto-fix-…) that updates the dependencies and checks the project. Nothing reaches main without your merge.",
                            urgent.len()
                        ))
                        .clicked()
                    {
                        let req = FixRequest { project: project.to_string(), findings: urgent };
                        request_fix(ui.ctx(), req);
                    }
                }
            }
        }
        super::projects::card_rule(ui, t);
        ui.horizontal(|ui| {
            if running {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(
                    RichText::new(crate::tr!("Vérification…", "Checking…"))
                        .size(12.5)
                        .color(t.tertiary),
                );
            } else {
                let text = if last.is_some() {
                    crate::tr!("Vérifier à nouveau", "Check again")
                } else {
                    crate::tr!("Vérifier maintenant", "Check now")
                };
                if super::projects::action_link(ui, text)
                    .on_hover_text(crate::tr!(
                        "Lance l'outil d'audit de chaque écosystème (cargo audit, npm audit) dans le dossier du projet. Il consulte les bases de failles connues (RustSec, registre npm) : la liste des dépendances peut leur être envoyée.",
                        "Runs each ecosystem's audit tool (cargo audit, npm audit) in the project's folder. It looks up the known vulnerability databases (RustSec, npm registry): the list of dependencies may be sent to them."
                    ))
                    .clicked()
                {
                    start(folder, &ctx);
                }
            }
            if let Some((at, _)) = &last {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(super::chrome::relative_time(*at))
                            .size(12.0)
                            .color(t.tertiary),
                    );
                });
            }
        });
        let color = if *auto { RED } else { t.tertiary };
        ui.checkbox(
            auto,
            RichText::new(crate::tr!(
                "Veille automatique (une fois par jour)",
                "Automatic watch (once a day)"
            ))
            .size(12.5)
            .color(color),
        )
        .on_hover_text(crate::tr!(
            "Tant qu'Orchestra est ouvert, vérifie les dépendances une fois par jour. Dès qu'une faille importante (élevée ou critique) pas encore traitée apparaît, ajoute ses étapes « Veille CVE » et lance seule une session de correction sur une branche à part (orchestra/auto-fix-…). Rien n'arrive sur main sans votre fusion.",
            "While Orchestra is open, checks the dependencies once a day. As soon as an important vulnerability (high or critical) not handled yet shows up, adds its « CVE watch » steps and starts a fix session by itself on a branch of its own (orchestra/auto-fix-…). Nothing reaches main without your merge."
        ));
    })
}

/// "● élevée  CVE-2024-1234  foo 1.2.0 — Out-of-bounds read", the id opening the advisory.
fn finding_row(ui: &mut egui::Ui, f: &Finding, t: &Tone) {
    let color = severity_color(f.severity, t);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(format!("● {}", f.severity.label())).size(12.0).color(color));
        let id = ui.add(
            egui::Label::new(RichText::new(&f.id).size(12.5).color(BLUE))
                .sense(egui::Sense::click()),
        );
        if let Some(u) = &f.url
            && id.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(u).clicked()
            && let Err(e) = crate::opener::open_url(u)
        {
            eprintln!("orchestra: {e}");
        }
        ui.label(
            RichText::new(format!("{} {}", f.package, f.version).trim().to_string())
                .size(12.5)
                .color(t.text),
        );
    });
    let mut sub = f.title.clone();
    if let Some(fix) = &f.fix {
        let corr = crate::trf!("corrigé : {fix}", "fixed: {fix}");
        sub = if sub.is_empty() { corr } else { format!("{sub} · {corr}") };
    }
    if !sub.is_empty() {
        ui.label(RichText::new(sub).size(12.0).color(t.tertiary));
    }
    ui.add_space(3.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::Eco;

    fn finding(id: &str, s: Severity) -> Finding {
        Finding {
            id: id.into(),
            package: "p".into(),
            version: "1".into(),
            title: String::new(),
            severity: s,
            fix: None,
            url: None,
        }
    }

    /// The fix session gets one step per important vulnerability, and a mission naming them.
    #[test]
    fn a_fix_session_is_built_from_the_important_findings() {
        let mut a = finding("CVE-2024-1", Severity::Critical);
        a.package = "foo".into();
        a.fix = Some(">=1.2.3".into());
        let low = finding("RUSTSEC-9", Severity::Low);
        let mut b = finding("GHSA-x", Severity::High);
        b.package = "bar".into();
        let urgent = important(&[&a, &low, &b, &a]);
        assert_eq!(urgent.len(), 3);
        let steps = fix_steps(&urgent);
        // Never twice the same step; never the minor one.
        assert_eq!(steps.len(), 2);
        if !crate::i18n::en() {
            assert_eq!(steps[0], "Veille CVE : corriger CVE-2024-1 dans foo (>=1.2.3)");
            assert_eq!(steps[1], "Veille CVE : corriger GHSA-x dans bar");
        }
        let m = fix_mission(&urgent);
        assert!(m.contains("CVE-2024-1 (foo 1 → >=1.2.3)"), "{m}");
        assert!(!m.contains("RUSTSEC-9"));
    }

    /// The automatic watch: once a day, counted from the latest check (taken into account
    /// or not), and never twice for a vulnerability a step already names.
    #[test]
    fn the_automatic_watch_checks_daily_and_fixes_once() {
        let now = 10 * DAY_MS;
        assert!(check_due(0, None, now));
        assert!(!check_due(now - 1000, None, now));
        assert!(check_due(now - DAY_MS, None, now));
        // A check made by hand an hour ago counts too.
        assert!(!check_due(now - 3 * DAY_MS, Some(now - 3_600_000), now));

        let mut a = finding("CVE-2024-1", Severity::Critical);
        a.package = "foo".into();
        let mut b = finding("GHSA-x", Severity::High);
        b.package = "bar".into();
        let both = vec![a.clone(), b.clone()];
        assert_eq!(untreated(&both, &[]).len(), 2);
        let known = fix_steps(std::slice::from_ref(&a));
        let left = untreated(&both, &known);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "GHSA-x");
        assert!(untreated(&both, &fix_steps(&both)).is_empty());
    }

    #[test]
    fn the_header_counts_the_important_findings() {
        let ok = |v| EcoReport { eco: Eco::Cargo, result: Ok(v) };
        let err = EcoReport { eco: Eco::Npm, result: Err("npm n'est pas installé".into()) };
        assert_eq!(summary(&[]), "");
        assert_eq!(summary(std::slice::from_ref(&err)), "");
        if !crate::i18n::en() {
            assert_eq!(summary(&[ok(vec![])]), "aucune faille connue");
            let r = [ok(vec![finding("a", Severity::Low)]), err.clone()];
            assert_eq!(summary(&r), "1 faille");
            let r = [
                ok(vec![finding("a", Severity::Critical), finding("b", Severity::Moderate)]),
                EcoReport { eco: Eco::Npm, result: Ok(vec![finding("c", Severity::High)]) },
            ];
            assert_eq!(summary(&r), "2 importantes · 3 au total");
            // The worst first, whichever ecosystem they come from.
            let ids: Vec<&str> = all_findings(&r).iter().map(|f| f.id.as_str()).collect();
            assert_eq!(ids, ["a", "c", "b"]);
        }
    }
}
