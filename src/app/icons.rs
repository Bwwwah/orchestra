//! Thin line icons drawn with the painter, in a simple style
//! (outlined, 1.4 px strokes, rounded). Each draws inside `rect` in `color`.

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // a few kept for later screens
pub enum Icon {
    Sidebar,
    Compose,
    Search,
    Home,
    Sparkle,
    People,
    Library,
    Database,
    Board,
    Calendar,
    Plus,
    Sliders,
    Help,
    Trash,
    Lock,
    Link,
    Star,
    StarFilled,
    More,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Page,
    Close,
    Gear,
    Download,
    TextT,
    Heading(u8),
    TodoList,
    BulletList,
    NumberList,
    ToggleList,
    Quote,
    Callout,
    Code,
    Divider,
}

fn stroke(c: Color32) -> Stroke {
    Stroke::new(1.4, c)
}

fn poly(p: &egui::Painter, pts: Vec<Pos2>, closed: bool, s: Stroke) {
    if closed {
        p.add(Shape::closed_line(pts, s));
    } else {
        p.add(Shape::line(pts, s));
    }
}

pub fn paint(p: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = stroke(color);
    let c = rect.center();
    // Work in a 20x20 box centred on the rect.
    let k = rect.width().min(rect.height()) / 20.0;
    let at = |x: f32, y: f32| pos2(c.x + (x - 10.0) * k, c.y + (y - 10.0) * k);
    match icon {
        Icon::Sidebar => {
            p.rect_stroke(Rect::from_min_max(at(3.0, 4.0), at(17.0, 16.0)), 2.5 * k, s, egui::StrokeKind::Middle);
            p.line_segment([at(8.0, 4.0), at(8.0, 16.0)], s);
        }
        Icon::Compose => {
            poly(p, vec![at(10.0, 4.0), at(5.0, 4.0), at(4.0, 5.0), at(4.0, 16.0), at(5.0, 17.0), at(15.0, 17.0), at(16.0, 16.0), at(16.0, 11.0)], false, s);
            p.line_segment([at(9.0, 12.0), at(16.0, 4.5)], s);
            p.line_segment([at(9.0, 12.0), at(8.5, 13.0)], s);
        }
        Icon::Search => {
            p.circle_stroke(at(9.0, 9.0), 4.8 * k, s);
            p.line_segment([at(12.6, 12.6), at(16.0, 16.0)], s);
        }
        Icon::Home => {
            let pts = vec![at(10.0, 3.5), at(17.0, 9.5), at(15.5, 9.5), at(15.5, 16.5), at(4.5, 16.5), at(4.5, 9.5), at(3.0, 9.5)];
            p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Sparkle => {
            let star = |cx: f32, cy: f32, r: f32| {
                vec![at(cx, cy - r), at(cx + r * 0.28, cy - r * 0.28), at(cx + r, cy), at(cx + r * 0.28, cy + r * 0.28), at(cx, cy + r), at(cx - r * 0.28, cy + r * 0.28), at(cx - r, cy), at(cx - r * 0.28, cy - r * 0.28)]
            };
            poly(p, star(9.0, 10.5, 6.5), true, s);
            poly(p, star(15.5, 4.5, 2.5), true, s);
        }
        Icon::People => {
            p.circle_stroke(at(8.0, 7.0), 2.8 * k, s);
            poly(p, vec![at(3.0, 16.5), at(3.5, 13.5), at(5.5, 11.5), at(10.5, 11.5), at(12.5, 13.5), at(13.0, 16.5)], false, s);
            p.circle_stroke(at(14.0, 7.5), 2.2 * k, s);
            poly(p, vec![at(14.0, 11.5), at(16.0, 12.0), at(17.0, 14.0), at(17.0, 16.0)], false, s);
        }
        Icon::Library => {
            p.rect_stroke(Rect::from_min_max(at(3.5, 4.0), at(6.5, 16.5)), 0.5 * k, s, egui::StrokeKind::Middle);
            p.rect_stroke(Rect::from_min_max(at(7.5, 4.0), at(10.5, 16.5)), 0.5 * k, s, egui::StrokeKind::Middle);
            poly(p, vec![at(12.0, 5.0), at(14.5, 4.2), at(17.5, 15.8), at(15.0, 16.6)], true, s);
        }
        Icon::Database => {
            p.rect_stroke(Rect::from_min_max(at(3.5, 4.5), at(16.5, 15.5)), 2.0 * k, s, egui::StrokeKind::Middle);
            p.line_segment([at(3.5, 8.5), at(16.5, 8.5)], s);
            p.line_segment([at(8.0, 8.5), at(8.0, 15.5)], s);
        }
        Icon::Board => {
            // Three columns of cards of different heights.
            for (x, h) in [(3.5, 12.0), (8.5, 8.0), (13.5, 10.0)] {
                p.rect_stroke(Rect::from_min_max(at(x, 4.0), at(x + 3.0, 4.0 + h)), 1.0 * k, s, egui::StrokeKind::Middle);
            }
        }
        Icon::Calendar => {
            p.rect_stroke(Rect::from_min_max(at(3.5, 5.0), at(16.5, 16.5)), 2.0 * k, s, egui::StrokeKind::Middle);
            p.line_segment([at(3.5, 8.5), at(16.5, 8.5)], s);
            p.line_segment([at(7.0, 3.5), at(7.0, 6.0)], s);
            p.line_segment([at(13.0, 3.5), at(13.0, 6.0)], s);
            p.circle_filled(at(7.5, 12.5), 1.0 * k, color);
        }
        Icon::Plus => {
            p.line_segment([at(4.5, 10.0), at(15.5, 10.0)], s);
            p.line_segment([at(10.0, 4.5), at(10.0, 15.5)], s);
        }
        Icon::Sliders => {
            p.line_segment([at(3.5, 7.0), at(16.5, 7.0)], s);
            p.line_segment([at(3.5, 13.0), at(16.5, 13.0)], s);
            p.circle_filled(at(7.0, 7.0), 2.2 * k, color);
            p.circle_filled(at(13.0, 13.0), 2.2 * k, color);
        }
        Icon::Help => {
            p.circle_stroke(c, 7.0 * k, s);
            p.text(c + vec2(0.0, 0.5 * k), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(11.0 * k), color);
        }
        Icon::Trash => {
            p.line_segment([at(4.0, 6.0), at(16.0, 6.0)], s);
            poly(p, vec![at(8.0, 6.0), at(8.5, 4.0), at(11.5, 4.0), at(12.0, 6.0)], false, s);
            poly(p, vec![at(5.5, 6.0), at(6.5, 16.5), at(13.5, 16.5), at(14.5, 6.0)], false, s);
        }
        Icon::Lock => {
            p.rect_stroke(Rect::from_min_max(at(5.0, 9.0), at(15.0, 17.0)), 1.5 * k, s, egui::StrokeKind::Middle);
            poly(p, vec![at(7.0, 9.0), at(7.0, 6.5), at(8.0, 4.5), at(10.0, 3.8), at(12.0, 4.5), at(13.0, 6.5), at(13.0, 9.0)], false, s);
            p.circle_filled(at(10.0, 13.0), 1.0 * k, color);
        }
        Icon::Link => {
            // Two chain links: capsules along the diagonal, overlapping in the middle.
            let capsule = |cx: f32, cy: f32| {
                let (len, r) = (3.2f32, 2.6f32);
                let (ax, ay) = (std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2);
                let mut pts = vec![];
                for end in [1.0f32, -1.0] {
                    let (ex, ey) = (cx + ax * len * end, cy + ay * len * end);
                    let base = ay.atan2(ax) + if end > 0.0 { -std::f32::consts::FRAC_PI_2 } else { std::f32::consts::FRAC_PI_2 };
                    for k in 0..=8 {
                        let a = base + std::f32::consts::PI * (k as f32) / 8.0;
                        pts.push(at(ex + r * a.cos(), ey + r * a.sin()));
                    }
                }
                pts
            };
            poly(p, capsule(7.3, 12.7), true, s);
            poly(p, capsule(12.7, 7.3), true, s);
        }
        Icon::Star | Icon::StarFilled => {
            let mut pts = vec![];
            for i in 0..10 {
                let a = std::f32::consts::PI * (i as f32) / 5.0 - std::f32::consts::FRAC_PI_2;
                let r = if i % 2 == 0 { 7.0 } else { 3.0 };
                pts.push(at(10.0 + r * a.cos(), 10.5 + r * a.sin()));
            }
            if icon == Icon::StarFilled {
                // Fill as triangles from the centre (the star isn't convex).
                for i in 0..10 {
                    let tri = vec![at(10.0, 10.5), pts[i], pts[(i + 1) % 10]];
                    p.add(Shape::convex_polygon(tri, color, Stroke::NONE));
                }
            }
            poly(p, pts, true, s);
        }
        Icon::More => {
            for dx in [-5.0, 0.0, 5.0] {
                p.circle_filled(c + vec2(dx * k, 0.0), 1.4 * k, color);
            }
        }
        Icon::ChevronLeft => poly(p, vec![at(12.5, 4.5), at(7.0, 10.0), at(12.5, 15.5)], false, s),
        Icon::ChevronRight => poly(p, vec![at(7.5, 4.5), at(13.0, 10.0), at(7.5, 15.5)], false, s),
        Icon::ChevronDown => poly(p, vec![at(6.0, 8.0), at(10.0, 12.0), at(14.0, 8.0)], false, s),
        Icon::Page => {
            poly(p, vec![at(5.0, 3.5), at(12.0, 3.5), at(15.5, 7.0), at(15.5, 16.5), at(5.0, 16.5)], true, s);
            poly(p, vec![at(12.0, 3.5), at(12.0, 7.0), at(15.5, 7.0)], false, s);
        }
        Icon::Close => {
            p.line_segment([at(5.5, 5.5), at(14.5, 14.5)], s);
            p.line_segment([at(14.5, 5.5), at(5.5, 14.5)], s);
        }
        Icon::Download => {
            p.line_segment([at(10.0, 3.5), at(10.0, 13.0)], s);
            poly(p, vec![at(6.0, 9.5), at(10.0, 13.5), at(14.0, 9.5)], false, s);
            p.line_segment([at(4.0, 16.5), at(16.0, 16.5)], s);
        }
        Icon::TextT => {
            p.text(c, egui::Align2::CENTER_CENTER, "Aa", egui::FontId::proportional(12.0 * k), color);
        }
        Icon::Heading(n) => {
            p.text(c, egui::Align2::CENTER_CENTER, format!("H{n}"), egui::FontId::proportional(12.0 * k), color);
        }
        Icon::TodoList => {
            p.rect_stroke(Rect::from_min_max(at(4.0, 4.0), at(16.0, 16.0)), 3.0 * k, s, egui::StrokeKind::Middle);
            poly(p, vec![at(7.0, 10.0), at(9.3, 12.5), at(13.5, 7.5)], false, s);
        }
        Icon::BulletList => {
            for y in [5.5, 10.0, 14.5] {
                p.circle_filled(at(4.5, y), 1.2 * k, color);
                p.line_segment([at(8.0, y), at(16.5, y)], s);
            }
        }
        Icon::NumberList => {
            for (i, y) in [6.0f32, 14.0].into_iter().enumerate() {
                p.text(at(4.5, y), egui::Align2::CENTER_CENTER, format!("{}", i + 1), egui::FontId::proportional(7.0 * k), color);
                p.line_segment([at(8.0, y), at(16.5, y)], s);
            }
        }
        Icon::ToggleList => {
            for y in [6.0f32, 14.0] {
                p.add(Shape::convex_polygon(vec![at(3.5, y - 2.3), at(6.3, y), at(3.5, y + 2.3)], color, Stroke::NONE));
                p.line_segment([at(8.0, y), at(16.5, y)], s);
            }
        }
        Icon::Quote => {
            p.line_segment([at(4.5, 4.0), at(4.5, 16.0)], Stroke::new(2.0, color));
            for y in [6.0, 10.0, 14.0] {
                p.line_segment([at(8.0, y), at(16.5, y)], s);
            }
        }
        Icon::Callout => {
            p.rect_stroke(Rect::from_min_max(at(3.5, 4.5), at(16.5, 15.5)), 2.5 * k, s, egui::StrokeKind::Middle);
            p.circle_filled(at(7.0, 10.0), 1.4 * k, color);
            p.line_segment([at(10.0, 10.0), at(14.0, 10.0)], s);
        }
        Icon::Code => {
            poly(p, vec![at(7.0, 6.0), at(3.5, 10.0), at(7.0, 14.0)], false, s);
            poly(p, vec![at(13.0, 6.0), at(16.5, 10.0), at(13.0, 14.0)], false, s);
            p.line_segment([at(11.5, 5.0), at(8.5, 15.0)], s);
        }
        Icon::Divider => {
            p.line_segment([at(3.5, 10.0), at(16.5, 10.0)], s);
        }
        Icon::Gear => {
            p.circle_stroke(c, 2.5 * k, s);
            for i in 0..8 {
                let a = std::f32::consts::PI * (i as f32) / 4.0;
                p.line_segment([c + vec2(a.cos(), a.sin()) * 5.0 * k, c + vec2(a.cos(), a.sin()) * 7.0 * k], Stroke::new(2.0, color));
            }
            p.circle_stroke(c, 5.0 * k, s);
        }
    }
}

/// A square icon button with a rounded hover tint.
pub fn button(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32, hover: Color32, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, hover);
    }
    paint(ui.painter(), rect.shrink(size * 0.18), icon, color);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// The small "×" that deletes a step, a message, an image…: `rest` when the pointer is
/// elsewhere (transparent to hide it until its row is hovered), and `hover_fg` on a `hover_bg`
/// pad while the pointer is on it, so it never sinks into the background.
pub fn delete_cross(ui: &mut egui::Ui, size: f32, rest: Color32, hover_fg: Color32, hover_bg: Color32, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), egui::Sense::click());
    let color = if resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, hover_bg);
        hover_fg
    } else {
        rest
    };
    if color.a() > 0 {
        paint(ui.painter(), rect.shrink(size * 0.22), Icon::Close, color);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}
