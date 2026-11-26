//! The tutorial: the screen dimmed except the area being presented, with a bubble next to it
//! (text, Précédent / Suivant / Passer, counter « 2/5 »). The areas are named: the screens
//! record where they drew them (`mark`) and each stop of the tour points at one by name; a stop
//! whose area isn't on screen is skipped over, so the tour never shows an empty frame.

use super::*;

/// Where a stop is shown: the tour opens that screen before pointing at the area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Place {
    /// The tutorial's example project (created if needed).
    Project,
    View(View),
}

/// One stop: the area it presents, the screen it is on, and what is said about it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stop {
    pub target: &'static str,
    pub place: Place,
    pub title: &'static str,
    pub text: &'static str,
}

/// The tour in progress.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Tour {
    pub stops: Vec<Stop>,
    pub at: usize,
    /// The stop whose screen was last opened, and on which pass: its area gets a few frames
    /// to appear before the stop is skipped.
    shown: Option<(usize, u64)>,
}

impl Tour {
    pub fn new(stops: Vec<Stop>) -> Tour {
        Tour {
            stops,
            at: 0,
            shown: None,
        }
    }

    pub fn current(&self) -> Option<&Stop> {
        self.stops.get(self.at)
    }

    /// « 2/5 ».
    pub fn counter(&self) -> String {
        format!("{}/{}", self.at + 1, self.stops.len())
    }

    pub fn is_last(&self) -> bool {
        self.at + 1 >= self.stops.len()
    }

    /// Next stop; false (and no move) on the last one.
    pub fn next(&mut self) -> bool {
        if self.is_last() {
            return false;
        }
        self.at += 1;
        true
    }

    pub fn previous(&mut self) -> bool {
        if self.at == 0 {
            return false;
        }
        self.at -= 1;
        true
    }
}

/// Frames a stop's area gets to appear after its screen opens, before the stop is skipped.
const ARRIVAL_PASSES: u64 = 6;

/// The guided tour: one walk through the app, from a project to the dashboard, the inbox
/// and the subscriptions, each stop on its own screen.
pub(super) fn tour_stops() -> Vec<Stop> {
    vec![
        Stop {
            target: "side:projects",
            place: Place::Project,
            title: crate::tr!("Vos projets", "Your projects"),
            text: crate::tr!(
                "Chaque projet que suivent vos IA. Un point vert : une IA y travaille ; orange : une question attend votre réponse.",
                "Every project your AIs follow. A green dot: an AI is working on it; orange: a question waits for your answer."
            ),
        },
        Stop {
            target: "project:actions",
            place: Place::Project,
            title: crate::tr!("Lancer une IA", "Start an AI"),
            text: crate::tr!(
                "Une IA avance seule sur les étapes ouvertes, dans une copie à part du dossier. « Réglages… » : l'IA (ou ✨ Automatique), le modèle, le plafond, une consigne.",
                "An AI works alone on the open steps, in a separate copy of the folder. « Settings… »: the AI (or ✨ Automatic), the model, the ceiling, a mission."
            ),
        },
        Stop {
            target: "project:tabs",
            place: Place::Project,
            title: crate::tr!("Les onglets", "The tabs"),
            text: crate::tr!(
                "La vue d'ensemble pour l'essentiel ; les versions, la veille des failles, les pages et le regard extérieur à un clic.",
                "The overview for what matters; versions, the vulnerability watch, the pages and the outside view a click away."
            ),
        },
        Stop {
            target: "card:summary",
            place: Place::Project,
            title: crate::tr!("État des lieux", "Current state"),
            text: crate::tr!(
                "Ce qui est fait, en cours, bloqué : tenu à jour par les IA à chaque session.",
                "What is done, in progress, blocked: kept up to date by the AIs at each session."
            ),
        },
        Stop {
            target: "card:steps",
            place: Place::Project,
            title: crate::tr!("Étapes", "Steps"),
            text: crate::tr!(
                "Ce qu'il reste à faire, dans l'ordre. Les IA les prennent une à une. Dépliez une étape pour la confier à une IA précise.",
                "What is left to do, in order. The AIs take them one by one. Unfold a step to entrust it to a given AI."
            ),
        },
        Stop {
            target: "card:sessions",
            place: Place::Project,
            title: crate::tr!("Sessions", "Sessions"),
            text: crate::tr!(
                "Les sessions des IA sur ce projet, en cours ou passées ; reprenez-en une, ou ouvrez-en une nouvelle dans le terminal.",
                "The AIs' sessions on this project, running or past; resume one, or open a new one in the terminal."
            ),
        },
        Stop {
            target: "orch:launch",
            place: Place::View(View::Orchestra),
            title: crate::tr!("Orchestration", "Orchestration"),
            text: crate::tr!(
                "Le tableau de bord de tous les projets : ce qui tourne, et le bouton « Lancer » de chacun.",
                "The dashboard of every project: what runs, and each one's « Start » button."
            ),
        },
        Stop {
            target: "orch:limits",
            place: Place::View(View::Orchestra),
            title: crate::tr!("Réglages et limites", "Settings and limits"),
            text: crate::tr!(
                "Les réglages communs des sessions. « Limites de chaque IA » ouvre Paramètres › IA : où ses sessions s'arrêtent et la réserve qu'elle vous laisse.",
                "The sessions' common settings. « Each AI's limits » opens Settings › AI: where its sessions stop and the reserve it leaves you."
            ),
        },
        Stop {
            target: "inbox:filters",
            place: Place::View(View::Inbox),
            title: crate::tr!("Boîte de réception", "Inbox"),
            text: crate::tr!(
                "Les questions des IA, les fins de session et les rapports. Répondez ici : une session en pause reprend.",
                "The AIs' questions, session ends and reports. Answer here: a paused session goes on."
            ),
        },
        Stop {
            target: "side:usage",
            place: Place::View(View::Inbox),
            title: crate::tr!("Vos abonnements", "Your subscriptions"),
            text: crate::tr!(
                "Chaque IA branchée, sa fenêtre de 5 h et sa semaine, d'un coup d'œil. Le détail au survol.",
                "Each AI plugged in, its 5-hour window and its week, at a glance. The details on hover."
            ),
        },
    ]
}

fn mark_key(name: &str) -> egui::Id {
    egui::Id::unique(("tutorial-area", name))
}

/// The mark a tour is pointing at now, if any (a page shows the part that holds it).
pub fn wanted(ctx: &egui::Context) -> Option<String> {
    ctx.data(|d| d.get_temp(want_key()))
}

fn want_key() -> egui::Id {
    egui::Id::unique("tutorial-want")
}

fn scrolled_key() -> egui::Id {
    egui::Id::unique("tutorial-scrolled")
}

/// Record where `name` was drawn this frame. If the tour is presenting it and it isn't fully
/// visible in the scroll area drawing it, scroll it into view (once per stop).
pub fn mark(ui: &egui::Ui, name: &str, rect: egui::Rect) {
    let ctx = ui.ctx();
    // Read the pass number first: calling `ctx` inside `data_mut` locks the context twice and
    // freezes the app.
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(mark_key(name), (rect, pass)));
    let want: Option<String> = ctx.data(|d| d.get_temp(want_key()));
    if want.as_deref() != Some(name) {
        return;
    }
    let done: Option<String> = ctx.data(|d| d.get_temp(scrolled_key()));
    if done.as_deref() == Some(name) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(scrolled_key(), name.to_string()));
    if !ui.clip_rect().contains_rect(rect) {
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
    }
}

/// Where `name` was drawn, if it was on the last frames.
pub fn area(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
    let (rect, pass): (egui::Rect, u64) = ctx.data(|d| d.get_temp(mark_key(name)))?;
    (ctx.cumulative_pass_nr().saturating_sub(pass) <= 2).then_some(rect)
}

/// Where to put a bubble of `size` next to `target` inside `screen`: under it if it fits, else
/// above, else beside; always kept inside the screen.
pub fn bubble_pos(target: egui::Rect, size: egui::Vec2, screen: egui::Rect) -> egui::Pos2 {
    let gap = 12.0;
    let mut p = if target.bottom() + gap + size.y <= screen.bottom() {
        egui::pos2(target.left(), target.bottom() + gap)
    } else if target.top() - gap - size.y >= screen.top() {
        egui::pos2(target.left(), target.top() - gap - size.y)
    } else if target.right() + gap + size.x <= screen.right() {
        egui::pos2(target.right() + gap, target.top())
    } else {
        egui::pos2(target.left() - gap - size.x, target.top())
    };
    p.x = p.x.clamp(screen.left() + 8.0, (screen.right() - size.x - 8.0).max(screen.left() + 8.0));
    p.y = p.y.clamp(screen.top() + 8.0, (screen.bottom() - size.y - 8.0).max(screen.top() + 8.0));
    p
}

impl NotesApp {
    /// Begin the guided tour, from the tutorial's example project.
    pub(super) fn start_tour(&mut self) {
        self.tour = Some(Tour::new(tour_stops()));
    }

    /// Open the screen of the tour's current stop.
    fn open_place(&mut self, place: Place) {
        match place {
            Place::Project => {
                let id = self.ws.ensure_tutorial_project();
                if self.current.as_deref() != Some(id.as_str()) || self.view != View::Page {
                    self.navigate(id);
                }
                self.view = View::Page;
            }
            Place::View(v) => self.view = v,
        }
    }

    /// Draw the tour in progress, if any.
    pub(super) fn tutorial_ui(&mut self, ctx: &egui::Context) {
        let pass = ctx.cumulative_pass_nr();
        loop {
            let Some(tour) = self.tour.as_mut() else { return };
            let Some(stop) = tour.current().cloned() else { break };
            // Arriving at a stop: open its screen, and give its area a few frames.
            if tour.shown.map(|(i, _)| i) != Some(tour.at) {
                tour.shown = Some((tour.at, pass));
                self.open_place(stop.place);
                ctx.request_repaint();
                break;
            }
            let since = tour.shown.map_or(0, |(_, p)| pass.saturating_sub(p));
            // Its area never came (a card the project doesn't have): on to the next stop.
            if area(ctx, stop.target).is_none() && since > ARRIVAL_PASSES {
                if !tour.next() {
                    tour.at = tour.stops.len();
                }
                continue;
            }
            break;
        }
        let Some(tour) = self.tour.as_mut() else { return };
        let Some(stop) = tour.current().cloned() else {
            self.tour = None;
            ctx.data_mut(|d| {
                d.remove::<String>(want_key());
                d.remove::<String>(scrolled_key());
            });
            return;
        };
        // Tell the screens which area is presented, so the one drawing it can scroll to it.
        ctx.data_mut(|d| {
            if d.get_temp::<String>(want_key()).as_deref() != Some(stop.target) {
                d.remove::<String>(scrolled_key());
                d.insert_temp(want_key(), stop.target.to_string());
            }
        });
        let Some(target) = area(ctx, stop.target) else {
            // Its screen is opening: wait for the area.
            ctx.request_repaint();
            return;
        };
        let t = tone(ctx.global_style().visuals.dark_mode);
        let screen = ctx.content_rect();
        let hole = target.expand(6.0);
        // The dimming: four bands around the presented area, which also catch the clicks.
        let bands = [
            egui::Rect::from_min_max(screen.min, egui::pos2(screen.right(), hole.top())),
            egui::Rect::from_min_max(egui::pos2(screen.left(), hole.bottom()), screen.max),
            egui::Rect::from_min_max(
                egui::pos2(screen.left(), hole.top()),
                egui::pos2(hole.left(), hole.bottom()),
            ),
            egui::Rect::from_min_max(
                egui::pos2(hole.right(), hole.top()),
                egui::pos2(screen.right(), hole.bottom()),
            ),
        ];
        egui::Area::new(egui::Id::unique("tutorial-dim"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                for b in bands {
                    if b.is_positive() {
                        ui.painter().rect_filled(b, 0.0, egui::Color32::from_black_alpha(150));
                        ui.allocate_rect(b, egui::Sense::click_and_drag());
                    }
                }
                ui.painter().rect_stroke(
                    hole,
                    8.0,
                    egui::Stroke::new(2.0, t.accent_text),
                    egui::StrokeKind::Outside,
                );
            });
        let (mut go_next, mut go_prev, mut stop_tour) = (false, false, false);
        let (counter, last, first) = (tour.counter(), tour.is_last(), tour.at == 0);
        let size = egui::vec2(340.0, 170.0);
        let pos = bubble_pos(target, size, screen);
        egui::Area::new(egui::Id::unique("tutorial-bubble"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(t.card)
                    .stroke(egui::Stroke::new(1.0, t.border))
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::symmetric(16, 14))
                    .show(ui, |ui| {
                        ui.set_width(size.x - 32.0);
                        ui.label(egui::RichText::new(stop.title).font(semibold(15.0)));
                        ui.add_space(4.0);
                        ui.label(stop.text);
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&counter).weak());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                let label = if last {
                                    crate::tr!("Terminer", "Finish")
                                } else {
                                    crate::tr!("Suivant", "Next")
                                };
                                go_next = super::style::primary(ui, label).clicked();
                                if !first {
                                    go_prev = super::style::secondary(ui, crate::tr!("Précédent", "Back")).clicked();
                                }
                                if !last {
                                    stop_tour = super::style::quiet(ui, crate::tr!("Passer", "Skip")).clicked();
                                }
                            });
                        });
                    });
            });
        if stop_tour || (go_next && last) {
            self.tour = None;
        } else if go_next {
            tour.next();
        } else if go_prev {
            tour.previous();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three() -> Tour {
        Tour::new(tour_stops().into_iter().take(3).collect())
    }

    #[test]
    fn the_tour_moves_between_its_stops() {
        let mut t = three();
        assert_eq!(t.counter(), "1/3");
        assert!(!t.previous());
        assert!(t.next() && t.next());
        assert_eq!(t.counter(), "3/3");
        assert!(t.is_last());
        assert!(!t.next(), "no move past the last stop");
        assert!(t.previous());
        assert_eq!(t.counter(), "2/3");
    }

    /// Each stop presents an area of its own; the walk starts on the project, and each
    /// screen's stops come together (the tour doesn't go back and forth between screens).
    #[test]
    fn the_tour_walks_each_screen_once() {
        let s = tour_stops();
        let mut names: Vec<_> = s.iter().map(|s| s.target).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), s.len());
        assert_eq!(s[0].place, Place::Project);
        let mut seen: Vec<Place> = vec![];
        for st in &s {
            if seen.last() != Some(&st.place) {
                assert!(!seen.contains(&st.place), "{:?} comes back", st.place);
                seen.push(st.place);
            }
        }
    }

    /// The marks the tour points at in a project's cards each show their tab.
    #[test]
    fn the_cards_the_tour_presents_are_on_the_overview() {
        for st in tour_stops().iter().filter(|s| s.target.starts_with("card:")) {
            assert_eq!(
                super::super::projects::tab_of_mark(st.target),
                Some(super::super::projects::ProjectTab::Overview),
                "{}",
                st.target
            );
        }
    }

    #[test]
    fn the_bubble_stays_on_screen_next_to_its_area() {
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 700.0));
        let size = egui::vec2(320.0, 150.0);
        // Room below: under the area.
        let a = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(300.0, 100.0));
        assert_eq!(bubble_pos(a, size, screen), egui::pos2(100.0, 212.0));
        // No room below: above.
        let a = egui::Rect::from_min_size(egui::pos2(100.0, 500.0), egui::vec2(300.0, 150.0));
        assert_eq!(bubble_pos(a, size, screen).y, 500.0 - 12.0 - 150.0);
        // Against the right edge: pulled back in.
        let a = egui::Rect::from_min_size(egui::pos2(900.0, 100.0), egui::vec2(90.0, 50.0));
        let p = bubble_pos(a, size, screen);
        assert!(p.x + size.x <= screen.right());
        // Taller than anything around: still inside.
        let a = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(980.0, 680.0));
        assert!(screen.contains(bubble_pos(a, size, screen)));
    }
}
