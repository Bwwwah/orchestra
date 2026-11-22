//! The app's visual vocabulary in one place: the few colours that carry a meaning, the text
//! sizes, and the three kinds of button. Screens take them from here instead of writing
//! their own RGB values and button styles, so the same thing looks the same everywhere.
//!
//! Colours: one accent (blue) for the action that matters on a screen and for links; green
//! for what runs or is done; orange for what needs a look; red for errors and dangers. The
//! greys come from the theme (`Tone`).
//!
//! Buttons, by weight:
//! - `primary`: filled accent; one per screen or card at most, the action it is about.
//! - `secondary`: framed, neutral; the other actions.
//! - `quiet`: text only, grey; actions one rarely needs (clear, mark as read…).

use super::*;

/// The accent: the screen's main action, links, a selected choice.
pub const BLUE: Color32 = Color32::from_rgb(35, 131, 226);
/// Running, done, healthy.
pub const GREEN: Color32 = Color32::from_rgb(68, 131, 97);
/// Needs a look: a warning, a question waiting.
pub const ORANGE: Color32 = Color32::from_rgb(217, 115, 13);
/// Errors, dangers, destructive actions.
pub const RED: Color32 = Color32::from_rgb(224, 62, 62);
/// AI-made content (state of play, suggestions).
pub const PURPLE: Color32 = Color32::from_rgb(144, 101, 176);
/// Claude's own colour (its sessions, its tile).
pub const CORAL: Color32 = Color32::from_rgb(204, 120, 92);
/// Highlights that are neither good nor bad news.
pub const PINK: Color32 = Color32::from_rgb(212, 76, 125);

/// Text sizes: page titles, card or section titles, body, small print.
pub const TITLE: f32 = 30.0;
pub const HEADING: f32 = 16.0;
pub const BODY: f32 = 14.0;
pub const SMALL: f32 = 12.5;

/// Height of a regular button.
pub const BUTTON_H: f32 = 30.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Primary,
    Secondary,
    Quiet,
}

/// The screen's main action: filled accent, white text.
pub fn primary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    button(ui, Kind::Primary, text)
}

/// Any other action: a neutral frame.
pub fn secondary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    button(ui, Kind::Secondary, text)
}

/// A rarely needed action: grey text, a soft background under the pointer.
pub fn quiet(ui: &mut egui::Ui, text: &str) -> egui::Response {
    button(ui, Kind::Quiet, text)
}

/// One button of the given kind; greyed out when the `ui` is disabled.
pub fn button(ui: &mut egui::Ui, kind: Kind, text: &str) -> egui::Response {
    let t = tone(ui.visuals().dark_mode);
    let enabled = ui.is_enabled();
    let font = match kind {
        Kind::Primary => semibold(13.5),
        _ => FontId::proportional(13.5),
    };
    let g = ui.painter().layout_no_wrap(text.to_string(), font, t.text);
    let pad = if kind == Kind::Quiet { 10.0 } else { 14.0 };
    let size = egui::vec2(g.size().x + 2.0 * pad, BUTTON_H);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let a = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hov"), resp.hovered() && enabled, 0.14);
    let down = resp.is_pointer_button_down_on() && enabled;
    let r = if down { rect.shrink(1.0) } else { rect };
    let p = ui.painter();
    let (fill, stroke, color) = match (kind, enabled) {
        (Kind::Primary, true) => (
            BLUE.lerp_to_gamma(Color32::WHITE, 0.12 * a),
            egui::Stroke::NONE,
            Color32::WHITE,
        ),
        (Kind::Primary, false) => (t.fill, egui::Stroke::NONE, t.tertiary),
        (Kind::Secondary, en) => (
            t.hover.gamma_multiply(0.6 + 0.8 * a),
            egui::Stroke::new(1.0, t.border.lerp_to_gamma(t.tertiary, 0.5 * a)),
            if en { t.text } else { t.tertiary },
        ),
        (Kind::Quiet, en) => (
            t.hover.gamma_multiply(a),
            egui::Stroke::NONE,
            if en {
                t.secondary.lerp_to_gamma(t.text, a)
            } else {
                t.tertiary
            },
        ),
    };
    p.rect_filled(r, 8.0, fill);
    if stroke.width > 0.0 {
        p.rect_stroke(r, 8.0, stroke, egui::StrokeKind::Inside);
    }
    p.galley_with_override_text_color(r.center() - g.size() / 2.0, g, color);
    // Screen readers (and the tests) find it by its text.
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text));
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}
