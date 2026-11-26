//! The « Membres » card of a project in a team space: its people, then the space's AIs.
//! Members belong to the space, the same in all its projects: they are managed in the
//! space's administration window (spaces_ui.rs), which this card opens for an
//! administrator.

use super::projects::{card, card_title};
use super::*;
use crate::project::ProjectMember;

/// What the card needs to know about the team space shown, beyond the members themselves.
#[derive(Clone, Debug, Default)]
pub(super) struct TeamCx {
    /// The person's own member id (the public half of their key).
    pub me: String,
    /// The person administers the space.
    pub admin: bool,
    /// The space's server is reached now (it alone can change the members).
    pub connected: bool,
}

/// What the card needs in the team space this process shows.
pub(super) fn team_cx(ctx: &egui::Context, connected: bool) -> TeamCx {
    // The key is a file: read once, not at each frame.
    let id = egui::Id::unique("my-member-id");
    let me = ctx.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| {
        let me = crate::identity::key()
            .map(|k| crate::identity::public_hex(&k))
            .unwrap_or_default();
        ctx.data_mut(|d| d.insert_temp(id, me.clone()));
        me
    });
    TeamCx {
        me,
        admin: crate::team::Role::from_key(&crate::space::current().role)
            == Some(crate::team::Role::Admin),
        connected,
    }
}

/// The name of the agent an AI member runs (its key when Orchestra doesn't know it).
fn agent_label(key: &str) -> &str {
    crate::setup::Ai::ALL
        .into_iter()
        .find(|a| a.key() == key)
        .map_or(key, |a| a.label())
}

fn member_row(ui: &mut egui::Ui, m: &ProjectMember, me: bool, detail: &str, t: &Tone) {
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        ui.spacing_mut().item_spacing.x = 10.0;
        super::side::face(ui, &m.name);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(detail).size(12.5).color(t.tertiary));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let name = if me {
                    crate::trf!("{} (vous)", "{} (you)", m.name)
                } else {
                    m.name.clone()
                };
                ui.add(egui::Label::new(RichText::new(name).color(t.text)).truncate());
            });
        });
    });
}

/// Draws the card; true when the person asks to manage the members.
pub(super) fn members_card(
    ui: &mut egui::Ui,
    members: &[ProjectMember],
    team: &TeamCx,
    t: &Tone,
    extra: f32,
) -> (f32, bool) {
    let mut manage = false;
    let h = card(ui, t, extra, |ui| {
        card_title(
            ui,
            crate::tr!("MEMBRES", "MEMBERS"),
            &members.len().to_string(),
            t,
        );
        let (ais, people): (Vec<_>, Vec<_>) = members.iter().partition(|m| !m.ai.is_empty());
        for m in &people {
            member_row(ui, m, m.id == team.me, m.role.label(), t);
        }
        if !ais.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new(crate::tr!("IA de l'espace", "The space's AIs"))
                    .font(semibold(12.5))
                    .color(t.secondary),
            );
            for m in &ais {
                member_row(ui, m, false, agent_label(&m.ai), t);
            }
        }
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                RichText::new(if team.admin {
                    crate::tr!(
                        "Les membres sont ceux de l'espace : les mêmes dans tous ses projets.",
                        "Members are the space's: the same in all its projects."
                    )
                } else {
                    crate::tr!(
                        "Les membres sont ceux de l'espace, les mêmes dans tous ses projets. Un administrateur de l'espace les gère.",
                        "Members are the space's, the same in all its projects. An administrator of the space manages them."
                    )
                })
                .size(12.0)
                .color(t.tertiary),
            )
            .wrap(),
        );
        if team.admin {
            ui.add_space(6.0);
            let r = ui
                .add_enabled_ui(team.connected, |ui| {
                    super::style::secondary(ui, crate::tr!("Gérer les membres…", "Manage members…"))
                })
                .inner
                .on_hover_text(crate::tr!(
                    "Invitations, rôles, retraits et IA de l'espace",
                    "Invitations, roles, removals and the space's AIs"
                ))
                .on_disabled_hover_text(crate::tr!(
                    "Pas relié au serveur de l'espace.",
                    "Not connected to the space's server."
                ));
            manage = r.clicked();
        }
    });
    (h, manage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    #[derive(Default)]
    struct S {
        team: TeamCx,
        asked: bool,
        /// Drawn only once the app's fonts are in (the card's labels need them).
        draw: bool,
    }

    fn member(id: &str, name: &str, role: crate::team::Role, ai: &str) -> ProjectMember {
        ProjectMember {
            id: id.into(),
            name: name.into(),
            role,
            ai: ai.into(),
        }
    }

    fn harness(team: TeamCx) -> Harness<'static, S> {
        let t = tone(false);
        let members = vec![
            member("a", "Ana", crate::team::Role::Admin, ""),
            member("b", "Bob", crate::team::Role::Reader, ""),
            member("ia:Maison", "Maison", crate::team::Role::Member, "codex"),
        ];
        let mut h = Harness::builder().with_size([500.0, 500.0]).build_ui_state(
            move |ui, s: &mut S| {
                if s.draw {
                    s.asked |= members_card(ui, &members, &s.team, &t, 0.0).1;
                }
            },
            S {
                team,
                ..Default::default()
            },
        );
        setup_fonts(&h.ctx);
        h.run();
        h.state_mut().draw = true;
        h.run();
        h
    }

    #[test]
    fn the_card_lists_people_then_the_spaces_ais() {
        let h = harness(TeamCx {
            me: "b".into(),
            ..Default::default()
        });
        h.get_by_label("Ana");
        h.get_by_label("Bob (vous)");
        h.get_by_label("Maison");
        h.get_by_label("Codex");
        h.get_by_label("IA de l'espace");
        // Not an administrator: no way to manage them from here.
        assert!(h.query_by_label("Gérer les membres…").is_none());
    }

    #[test]
    fn an_administrator_opens_the_management_from_the_card() {
        let mut h = harness(TeamCx {
            me: "a".into(),
            admin: true,
            connected: true,
        });
        h.get_by_label("Gérer les membres…").click();
        h.run();
        assert!(h.state().asked);
    }
}
