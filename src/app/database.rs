//! Database views view tabs, a flat toolbar, a table with grey
//! headers, thin grid lines, coloured tags and a "+ Nouveau" row.

use super::*;

/// Tag colours for select options, cycling default/brown/orange/yellow/green/
/// blue/purple/pink/red.
fn tag_colors(i: usize, dark: bool) -> (Color32, Color32) {
    const LIGHT: [([u8; 3], [u8; 3]); 9] = [
        ([227, 226, 224], [50, 48, 44]),
        ([238, 224, 218], [68, 42, 30]),
        ([250, 222, 201], [73, 41, 14]),
        ([253, 236, 200], [64, 44, 27]),
        ([219, 237, 219], [28, 56, 41]),
        ([211, 229, 239], [24, 51, 71]),
        ([232, 222, 238], [65, 36, 84]),
        ([245, 224, 233], [76, 35, 55]),
        ([255, 226, 221], [93, 23, 21]),
    ];
    const DARK: [[u8; 3]; 9] = [
        [55, 55, 55],
        [68, 50, 40],
        [92, 59, 35],
        [86, 67, 40],
        [36, 61, 48],
        [20, 58, 78],
        [60, 45, 73],
        [78, 44, 60],
        [82, 46, 42],
    ];
    if dark {
        (rgb(DARK[i % 9]), Color32::from_rgb(240, 239, 237))
    } else {
        let (bg, fg) = LIGHT[i % 9];
        (rgb(bg), rgb(fg))
    }
}

pub(super) fn tag(ui: &mut egui::Ui, text: &str, index: usize) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let (bg, fg) = tag_colors(index, dark);
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(14.0), fg);
    let size = galley.size() + egui::vec2(12.0, 4.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    ui.painter().rect_filled(rect, 3.0, bg);
    ui.painter().galley(rect.min + egui::vec2(6.0, 2.0), galley, fg);
    resp
}

fn kind_glyph(k: ColumnKind) -> &'static str {
    match k {
        ColumnKind::Text => "Aa",
        ColumnKind::Number => "#",
        ColumnKind::Select => "◉",
        ColumnKind::Checkbox => "☑",
        ColumnKind::Date => "◷",
    }
}

/// A flat text button in secondary grey with a hover tint, like the database toolbar's.
fn flat_button(ui: &mut egui::Ui, text: impl Into<String>, t: &Tone) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text.into()).size(14.0).color(t.secondary)).frame_when_inactive(false))
}

pub(super) fn database_view(ui: &mut egui::Ui, page_id: &str, db: &mut Database, new_col: &mut (String, ColumnKind), actions: &mut Vec<Action>) {
    let t = tone(ui.visuals().dark_mode);
    // View tabs on the left (the active one underlined), tools on the right.
    let bar = ui.horizontal(|ui| {
        for (v, label) in [(DbView::Table, crate::tr!("▦  Tableau", "▦  Table")), (DbView::Board, crate::tr!("▥  Kanban", "▥  Kanban")), (DbView::List, crate::tr!("☰  Liste", "☰  List"))] {
            let active = db.view == v;
            let color = if active { t.text } else { t.secondary };
            let font = if active { semibold(14.0) } else { FontId::proportional(14.0) };
            let r = ui.add(egui::Button::new(RichText::new(label).font(font).color(color)).frame_when_inactive(false));
            if active {
                ui.painter().hline(r.rect.x_range(), r.rect.bottom() + 5.0, egui::Stroke::new(2.0, t.text));
            }
            if r.clicked() {
                db.view = v;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let add = flat_button(ui, crate::tr!("+ Propriété", "+ Property"), &t);
            egui::Popup::menu(&add).show(|ui| {
                ui.set_min_width(220.0);
                ui.add(egui::TextEdit::singleline(&mut new_col.0).hint_text(crate::tr!("Nom de la propriété", "Property name")));
                for k in ColumnKind::ALL {
                    ui.radio_value(&mut new_col.1, k, format!("{}  {}", kind_glyph(k), k.label()));
                }
                if ui.button(crate::tr!("Ajouter", "Add")).clicked() {
                    let name = if new_col.0.trim().is_empty() { crate::tr!("Propriété", "Property").to_string() } else { new_col.0.trim().to_string() };
                    let options = if new_col.1 == ColumnKind::Select { vec!["Option 1".into(), crate::tr!("Option 2", "Option 2").into()] } else { vec![] };
                    db.columns.push(Column { id: new_id(), name, kind: new_col.1, options });
                    new_col.0.clear();
                    ui.close();
                }
            });
            let sort_label = match &db.sort {
                Some((c, asc)) => format!(
                    "{} {}",
                    if *asc { "↑" } else { "↓" },
                    db.columns.iter().find(|x| &x.id == c).map(|x| x.name.as_str()).unwrap_or("?")
                ),
                None => crate::tr!("Trier", "Sort").into(),
            };
            let sort_btn = flat_button(ui, sort_label, &t);
            egui::Popup::menu(&sort_btn).show(|ui| {
                ui.set_min_width(200.0);
                if ui.button(crate::tr!("Aucun tri", "No sorting")).clicked() {
                    db.sort = None;
                    ui.close();
                }
                for c in db.columns.clone() {
                    if ui.button(format!("↑  {}", c.name)).clicked() {
                        db.sort = Some((c.id.clone(), true));
                        ui.close();
                    }
                    if ui.button(format!("↓  {}", c.name)).clicked() {
                        db.sort = Some((c.id.clone(), false));
                        ui.close();
                    }
                }
            });
            ui.add(
                egui::TextEdit::singleline(&mut db.filter)
                    .hint_text(RichText::new(crate::tr!("Filtrer…", "Filter…")).color(t.tertiary))
                    .desired_width(130.0)
                    .frame(egui::Frame::new().fill(Color32::TRANSPARENT).inner_margin(egui::Margin::symmetric(6, 4))),
            );
        });
    });
    let line_y = bar.response.rect.bottom() + 6.0;
    ui.painter().hline(bar.response.rect.x_range(), line_y, egui::Stroke::new(1.0, t.border));
    ui.add_space(12.0);
    match db.view {
        DbView::Table => table_view(ui, page_id, db, actions),
        DbView::Board => board_view(ui, page_id, db, actions),
        DbView::List => list_view(ui, page_id, db, actions),
    }
}

fn cell_editor(ui: &mut egui::Ui, col: &Column, value: &mut String, width: f32) {
    let t = tone(ui.visuals().dark_mode);
    match col.kind {
        ColumnKind::Checkbox => {
            let mut v = value == "true";
            todo_box(ui, &mut v, &t);
            *value = v.to_string();
        }
        ColumnKind::Select => {
            let resp = if value.is_empty() {
                let w = width.min(80.0);
                let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 22.0), egui::Sense::click());
                if resp.hovered() {
                    ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, crate::tr!("Choisir…", "Choose…"), FontId::proportional(14.0), t.tertiary);
                }
                resp
            } else {
                let idx = col.options.iter().position(|o| o == value).unwrap_or(0);
                tag(ui, value, idx)
            };
            egui::Popup::menu(&resp).show(|ui| {
                ui.set_min_width(180.0);
                ui.label(RichText::new(crate::tr!("Choisissez une option", "Choose an option")).small().color(t.secondary));
                for (i, o) in col.options.iter().enumerate() {
                    if tag(ui, o, i).clicked() {
                        *value = o.clone();
                        ui.close();
                    }
                }
                if !value.is_empty() && ui.button(crate::tr!("Effacer", "Clear")).clicked() {
                    value.clear();
                    ui.close();
                }
            });
        }
        ColumnKind::Date | ColumnKind::Number | ColumnKind::Text => {
            let ok = col.kind != ColumnKind::Number || value.trim().is_empty() || value.trim().replace(',', ".").parse::<f64>().is_ok();
            let hint = "";
            let mut te = egui::TextEdit::singleline(value)
                .hint_text(RichText::new(hint).color(t.tertiary))
                .frame(egui::Frame::NONE)
                .font(FontId::proportional(14.0))
                .text_color(if ok { t.text } else { Color32::from_rgb(205, 60, 58) })
                .desired_width(width);
            if col.kind == ColumnKind::Number {
                te = te.align(egui::Align2::RIGHT_CENTER);
            }
            ui.add(te);
        }
    }
}

fn table_view(ui: &mut egui::Ui, page_id: &str, db: &mut Database, actions: &mut Vec<Action>) {
    let t = tone(ui.visuals().dark_mode);
    let mut remove_col: Option<usize> = None;
    let mut remove_row: Option<String> = None;
    let rows = db.visible_rows();
    // Column widths: the title column wider.
    let widths: Vec<f32> = db
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| if i == 0 { 280.0 } else if c.kind == ColumnKind::Checkbox { 100.0 } else { 180.0 })
        .collect();
    let total: f32 = widths.iter().sum();
    let row_h = 34.0;
    let stroke = egui::Stroke::new(1.0, t.border);
    egui::ScrollArea::horizontal().id_salt(("dbh", page_id)).show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        let origin = ui.cursor().min;
        // Header.
        ui.horizontal(|ui| {
            for (ci, c) in db.columns.iter_mut().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(widths[ci], row_h), egui::Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, t.hover);
                }
                ui.painter().with_clip_rect(rect.shrink(4.0)).text(
                    rect.left_center() + egui::vec2(8.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{}  {}", kind_glyph(c.kind), c.name),
                    FontId::proportional(14.0),
                    t.secondary,
                );
                egui::Popup::menu(&resp).show(|ui| {
                    ui.set_min_width(220.0);
                    ui.text_edit_singleline(&mut c.name);
                    ui.label(RichText::new(crate::tr!("Type", "Type")).small().color(t.secondary));
                    for k in ColumnKind::ALL {
                        ui.radio_value(&mut c.kind, k, format!("{}  {}", kind_glyph(k), k.label()));
                    }
                    if c.kind == ColumnKind::Select {
                        ui.label(RichText::new(crate::tr!("Options (séparées par des virgules)", "Options (comma separated)")).small().color(t.secondary));
                        let mut opts = c.options.join(", ");
                        if ui.text_edit_singleline(&mut opts).changed() {
                            c.options = opts.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                        }
                    }
                    if ci > 0 && ui.button(crate::tr!("Retirer la propriété", "Remove the property")).clicked() {
                        remove_col = Some(ci);
                        ui.close();
                    }
                });
            }
        });
        // Rows.
        for &ri in &rows {
            let row_id = db.rows[ri].id.clone();
            let row = ui.horizontal(|ui| {
                for (ci, &w) in widths.iter().enumerate().take(db.columns.len()) {
                    let col = db.columns[ci].clone();
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, row_h), egui::Sense::hover());
                    let hovered = ui.rect_contains_pointer(rect);
                    let value = db.rows[ri].cells.entry(col.id.clone()).or_default();
                    let inner = rect.shrink2(egui::vec2(8.0, 6.0));
                    let edit_w = if ci == 0 { inner.width() - 64.0 } else { inner.width() };
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(inner)
                            .layout(egui::Layout::left_to_right(egui::Align::Center))
                            .id_salt((&row_id, &col.id)),
                    );
                    if ci == 0 {
                        child.add(
                            egui::TextEdit::singleline(value)
                                .hint_text(RichText::new(crate::tr!("Sans titre", "Untitled")).color(t.tertiary))
                                .frame(egui::Frame::NONE)
                                .font(semibold(14.0))
                                .text_color(t.text)
                                .desired_width(edit_w),
                        );
                    } else {
                        cell_editor(&mut child, &col, value, edit_w);
                    }
                    // "OUVRIR" appears in the title cell on hover, 
                    if ci == 0 && hovered {
                        let b = egui::Rect::from_min_size(egui::pos2(rect.right() - 62.0, rect.top() + 6.0), egui::vec2(54.0, 22.0));
                        let r = ui.interact(b, ui.scope_id().with(("open", &row_id)), egui::Sense::click());
                        ui.painter().rect_filled(b, 4.0, ui.visuals().panel_fill);
                        ui.painter().rect_stroke(b, 4.0, stroke, egui::StrokeKind::Inside);
                        ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, crate::tr!("OUVRIR", "OPEN"), semibold(11.0), t.secondary);
                        if r.clicked() {
                            actions.push(Action::OpenRow(page_id.to_string(), row_id.clone()));
                        }
                    }
                }
            });
            let rr = row.response.rect;
            ui.painter().hline(rr.x_range(), rr.top(), stroke);
            row.response.interact(egui::Sense::click()).context_menu(|ui| {
                if ui.button(crate::tr!("Ouvrir comme page", "Open as page")).clicked() {
                    actions.push(Action::OpenRow(page_id.to_string(), row_id.clone()));
                    ui.close();
                }
                if ui.button(crate::tr!("Retirer la ligne", "Remove the row")).clicked() {
                    remove_row = Some(row_id.clone());
                    ui.close();
                }
            });
        }
        // "+ Nouveau" row.
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(total, row_h), egui::Sense::click());
        ui.painter().hline(rect.x_range(), rect.top(), stroke);
        if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, t.hover);
        }
        ui.painter().text(rect.left_center() + egui::vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, crate::tr!("+  Nouveau", "+  New"), FontId::proportional(14.0), t.tertiary);
        if resp.clicked() {
            db.add_row("");
        }
        ui.painter().hline(rect.x_range(), rect.bottom(), stroke);
        // Lines between columns.
        let mut x = origin.x;
        for w in &widths[..widths.len().saturating_sub(1)] {
            x += w;
            ui.painter().vline(x, egui::Rangef::new(origin.y + row_h, rect.top()), stroke);
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new(crate::trf!("NOMBRE  {}", "NUMBER  {}", rows.len())).size(11.0).color(t.tertiary));
    if let Some(ci) = remove_col {
        let id = db.columns.remove(ci).id;
        for r in db.rows.iter_mut() {
            r.cells.remove(&id);
        }
        if db.sort.as_ref().map(|s| s.0 == id).unwrap_or(false) {
            db.sort = None;
        }
    }
    if let Some(r) = remove_row {
        db.rows.retain(|x| x.id != r);
    }
}
