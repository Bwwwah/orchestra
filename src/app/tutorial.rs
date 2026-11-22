//! The tutorial: the screen dimmed except the area being presented, with a bubble next to it
//! (text, Précédent / Suivant / Passer, counter « 2/5 »). The areas are named: the screens
//! record where they drew them (`mark`) and each stop of the tour points at one by name; a stop
//! whose area isn't on screen is skipped over, so the tour never shows an empty frame.

use super::*;

/// One stop: the area it presents and what is said about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub target: &'static str,
    pub title: &'static str,
    pub text: &'static str,
}

/// The tour in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Tour {
    pub stops: Vec<Stop>,
    pub at: usize,
}

impl Tour {
    pub fn new(stops: Vec<Stop>) -> Tour {
        Tour { stops, at: 0 }
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

/// The stops presenting the cards of a project's page.
pub fn project_cards_stops() -> Vec<Stop> {
    vec![
        Stop {
            target: "card:summary",
            title: "Résumé",
            text: "L'état des lieux du projet : ce qui est fait, en cours, bloqué.",
        },
        Stop {
            target: "card:steps",
            title: "Étapes",
            text: "Ce qu'il reste à faire. Les IA les prennent une à une et les cochent.",
        },
        Stop {
            target: "card:questions",
            title: "Questions",
            text: "Quand une IA a besoin d'une décision, sa question arrive ici.",
        },
        Stop {
            target: "card:sessions",
            title: "Sessions IA",
            text: "Les sessions lancées sur ce projet, en cours ou terminées.",
        },
        Stop {
            target: "card:pages",
            title: "Pages",
            text: "Les notes et documents rattachés au projet.",
        },
        Stop {
            target: "card:versions",
            title: "Versions",
            text: "L'état git du projet et la préparation de la prochaine version.",
        },
    ]
}

fn mark_key(name: &str) -> egui::Id {
    egui::Id::unique(("tutorial-area", name))
}

/// Record where `name` was drawn this frame.
pub fn mark(ctx: &egui::Context, name: &str, rect: egui::Rect) {
    // Read the pass number first: calling `ctx` inside `data_mut` locks the context twice and
    // freezes the app.
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(mark_key(name), (rect, pass)));
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
    /// Begin a tour.
    pub(super) fn start_tour(&mut self, stops: Vec<Stop>) {
        self.tour = Some(Tour::new(stops));
    }

    /// Open the tutorial's project and begin the tour of its page's cards.
    pub(super) fn start_project_cards_tour(&mut self) {
        let id = self.ws.ensure_tutorial_project();
        self.navigate(id);
        self.start_tour(project_cards_stops());
    }

    /// Draw the tour in progress, if any.
    pub(super) fn tutorial_ui(&mut self, ctx: &egui::Context) {
        let Some(tour) = self.tour.as_mut() else { return };
        // Skip the stops whose area isn't on screen.
        while tour.current().is_some_and(|s| area(ctx, s.target).is_none()) {
            if !tour.next() {
                break;
            }
        }
        let Some(stop) = tour.current().cloned() else {
            self.tour = None;
            return;
        };
        let Some(target) = area(ctx, stop.target) else {
            // Nothing of the tour is on screen (another page): wait for it.
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
        let size = egui::vec2(320.0, 150.0);
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
                                let label = if last { "Terminer" } else { "Suivant" };
                                go_next = ui.button(label).clicked();
                                if !first {
                                    go_prev = ui.button("Précédent").clicked();
                                }
                                if !last {
                                    stop_tour = ui.button("Passer").clicked();
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
        Tour::new(project_cards_stops().into_iter().take(3).collect())
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

    #[test]
    fn the_cards_stops_have_distinct_areas() {
        let s = project_cards_stops();
        let mut names: Vec<_> = s.iter().map(|s| s.target).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), s.len());
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
