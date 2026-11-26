//! « Versions de la page… » in a page's « … » menu: the versions kept for it on this computer
//! (history.rs), newest first, each one put back on the page with « Restaurer ».

use super::*;
use crate::history::{By, Content, Snapshot};

/// The window open on a page: its title, and its versions as they were when it opened.
pub(super) struct Open {
    pub page: String,
    title: String,
    list: Vec<Snapshot>,
}

impl Open {
    pub(super) fn of(ws: &Workspace, id: &str) -> Open {
        Open {
            page: id.to_string(),
            title: ws.page(id).map(|p| p.display_title()).unwrap_or_default(),
            list: crate::history::list(&crate::history::dir(), id),
        }
    }
}

impl NotesApp {
    pub(super) fn page_versions_window(&mut self, ctx: &egui::Context) {
        let Some(open) = self.page_versions.as_ref() else {
            return;
        };
        let id = open.page.clone();
        // Gone meanwhile (deleted, or moved to the trash): nothing to show.
        let Some(page) = self.ws.page(&id) else {
            self.page_versions = None;
            return;
        };
        let current = Content::of(page);
        let locked = self.ws.is_locked(&id);
        let dark = ctx.global_style().visuals.dark_mode;
        let t = tone(dark);
        let lk = super::settings::look(dark);
        let open = self.page_versions.as_ref().unwrap();
        let mut close = false;
        let mut restore: Option<usize> = None;
        let resp = egui::Modal::new(egui::Id::unique("page-versions"))
            .backdrop_color(Color32::from_black_alpha(100))
            .frame(
                egui::Frame::new()
                    .fill(lk.window)
                    .stroke(egui::Stroke::new(1.0, lk.border))
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::same(22)),
            )
            .show(ctx, |ui| {
                ui.set_width(480.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.label(
                    RichText::new(crate::tr!("Versions de la page", "Page versions"))
                        .font(semibold(super::style::HEADING + 2.0))
                        .color(t.text),
                );
                ui.label(RichText::new(&open.title).color(t.tertiary));
                if open.list.is_empty() {
                    ui.label(
                        RichText::new(crate::tr!(
                            "Aucune version gardée pour l'instant. Une version est gardée à chaque écriture d'une IA et toutes les 10 minutes d'édition.",
                            "No version kept yet. A version is kept at each write by an AI and every 10 minutes of editing."
                        ))
                        .color(t.tertiary),
                    );
                }
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (i, v) in open.list.iter().enumerate() {
                            let same = v.content.same(&current);
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    let name = if v.content.title.trim().is_empty() {
                                        crate::tr!("Sans titre", "Untitled").to_string()
                                    } else {
                                        v.content.title.clone()
                                    };
                                    ui.label(RichText::new(name).color(t.text));
                                    let by = match v.by {
                                        By::Ai => crate::tr!("après une écriture d'une IA", "after an AI's write"),
                                        By::Edit => crate::tr!("pendant la modification", "while editing"),
                                    };
                                    ui.label(
                                        RichText::new(format!(
                                            "{} · {by}",
                                            super::chrome::relative_time(v.at)
                                        ))
                                        .small()
                                        .color(t.tertiary),
                                    );
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if same {
                                        ui.label(
                                            RichText::new(crate::tr!("Version actuelle", "Current version"))
                                                .small()
                                                .color(t.tertiary),
                                        );
                                    } else if !locked
                                        && projects::action_link(ui, crate::tr!("Restaurer", "Restore")).clicked()
                                    {
                                        restore = Some(i);
                                    }
                                });
                            });
                            ui.separator();
                        }
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if projects::quiet_link(ui, crate::tr!("Fermer", "Close"), &t).clicked() {
                        close = true;
                    }
                });
            });
        if let Some(i) = restore {
            self.restore_page_version(&id, i);
        }
        if close || resp.should_close() || restore.is_some() {
            self.page_versions = None;
        }
    }

    /// Puts version `i` of the open list back on the page. What the page was just before is kept
    /// too, so that this can be undone the same way.
    fn restore_page_version(&mut self, id: &str, i: usize) {
        let Some(content) = self
            .page_versions
            .as_ref()
            .and_then(|o| o.list.get(i))
            .map(|s| s.content.clone())
        else {
            return;
        };
        let dir = crate::history::dir();
        let Some(before) = self.ws.page(id).cloned() else {
            return;
        };
        crate::history::record(&dir, &before, By::Edit);
        let now = crate::model::now_ms();
        let Some(p) = self.ws.page_mut(id) else {
            return;
        };
        content.apply_to(p, now);
        p.updated = now;
        let title = p.display_title();
        self.dirty_since.get_or_insert_with(Instant::now);
        self.ack(crate::trf!(
            "« {title} » remise dans cette version.",
            "« {title} » put back to this version."
        ));
    }
}
