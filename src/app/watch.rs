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

pub(super) fn watch_card(ui: &mut egui::Ui, folder: &str, t: &Tone, extra: f32) -> f32 {
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
