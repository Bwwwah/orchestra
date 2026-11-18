//! "Chercher ou demander": a palette in the middle of the window, over a veil that dims the
//! page. Pages first; when nothing matches (or on demand), the question goes to the AI
//! plugged in, which can read the workspace through the dashboard's MCP tools.

use super::*;
use super::icons::Icon;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Ask {
    pub question: String,
    /// Ok((answer, by)) or Err(message), once done.
    pub result: Arc<Mutex<Option<Result<(String, String), String>>>>,
    pub answer: Option<Result<(String, String), String>>,
    pub running: bool,
    /// Where the question was asked from (the project's folder, or home).
    pub folder: String,
    /// The palette should close (the question went to the terminal).
    pub close: bool,
}

const WIDTH: f32 = 660.0;

fn ask_prompt(q: &str, folder: &str) -> String {
    crate::trf!("Question posée depuis Orchestra (le tableau de bord de projets de l'utilisateur) : « {q} »\n\
         Pour y répondre, tu peux consulter son espace avec les outils MCP du serveur orchestra \
         (search_pages, read_page, list_projects, get_project_state…) et lire les fichiers du dossier « {folder} ». \
         Ne modifie rien. Réponds en français, directement et brièvement, en Markdown simple (pas de tableau large).", "Question asked from Orchestra (the user's project dashboard): « {q} »\n\
         To answer, you may look at their workspace with the orchestra MCP server's tools \
         (search_pages, read_page, list_projects, get_project_state…) and read the files of the folder « {folder} ». \
         Don't change anything. Answer in English, directly and briefly, in simple Markdown (no wide table)."
    )
}

impl NotesApp {
    pub(super) fn search_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        // Collect a finished answer.
        if self.search_ask.running {
            if let Some(r) = self.search_ask.result.lock().unwrap().take() {
                self.search_ask.answer = Some(r);
                self.search_ask.running = false;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            }
        }
        let shown = ctx.animate_bool_with_time(egui::Id::unique("search-shown"), self.search_open, 0.16);
        if shown <= 0.0 {
            return;
        }
        let t = tone(ctx.global_style().visuals.dark_mode);
        let screen = ctx.viewport_rect();
        // The veil: dims the whole window; a click on it closes.
        let veil_layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("search-veil-paint"));
        let alpha = if ctx.global_style().visuals.dark_mode { 160.0 } else { 100.0 };
        ctx.layer_painter(veil_layer).with_clip_rect(screen).rect_filled(screen, 0.0, Color32::from_black_alpha((alpha * shown) as u8));
        let veil = egui::Area::new(egui::Id::unique("search-veil")).order(egui::Order::Foreground).fixed_pos(screen.min).constrain(false).show(ctx, |ui| {
            ui.allocate_exact_size(screen.size(), egui::Sense::click()).1
        });
        let mut close = veil.inner.clicked() || ctx.input(|i| i.key_pressed(Key::Escape));
        if !self.search_open {
            return;
        }
        let results = self.ws.search(&self.search_query);
        let n = results.len().min(40);
        let query = self.search_query.trim().to_string();
        let plugged = self.ais.plugged();
        let ai_name = if plugged.contains(&crate::setup::Ai::ClaudeCode) {
            Some("Claude Code")
        } else if plugged.contains(&crate::setup::Ai::Codex) {
            Some("Codex")
        } else {
            None
        };
        // Keyboard: arrows move, Enter opens (or asks when the AI row is selected).
        let rows = n + usize::from(!query.is_empty());
        let (up, down, enter) = ctx.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Enter)));
        if down && rows > 0 {
            self.search_sel = (self.search_sel + 1).min(rows - 1);
        }
        if up {
            self.search_sel = self.search_sel.saturating_sub(1);
        }
        if self.search_sel >= rows.max(1) {
            self.search_sel = 0;
        }
        let mut ask_now = false;
        let top = screen.center().y - 230.0 + 12.0 * (1.0 - shown);
        egui::Area::new(egui::Id::unique("search-panel"))
            .order(egui::Order::Tooltip)
            .fixed_pos(egui::pos2(screen.center().x - WIDTH / 2.0, top.max(screen.top() + 40.0)))
            .show(ctx, |ui| {
                ui.multiply_opacity(shown);
                egui::Frame::new()
                    .fill(t.card)
                    .stroke(egui::Stroke::new(1.0, t.border))
                    .corner_radius(16.0)
                    .shadow(egui::epaint::Shadow { offset: [0, 18], blur: 48, spread: 0, color: Color32::from_black_alpha(110) })
                    .inner_margin(egui::Margin::symmetric(0, 6))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        // The field.
                        ui.horizontal(|ui| {
                            ui.add_space(18.0);
                            let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 44.0), egui::Sense::hover());
                            icons::paint(ui.painter(), egui::Rect::from_center_size(r.center(), egui::vec2(18.0, 18.0)), Icon::Search, t.secondary);
                            ui.add_space(8.0);
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut self.search_query)
                                    .hint_text(crate::tr!("Chercher une page, ou poser une question à l'IA…", "Search a page, or ask the AI a question…"))
                                    .font(FontId::proportional(18.0))
                                    .frame(egui::Frame::NONE)
                                    .desired_width(WIDTH - 70.0),
                            );
                            if self.search_focus {
                                edit.request_focus();
                                self.search_focus = false;
                            }
                            if edit.changed() {
                                self.search_sel = 0;
                                self.search_ask.answer = None;
                            }
                        });
                        let sep = ui.allocate_exact_size(egui::vec2(WIDTH, 1.0), egui::Sense::hover()).0;
                        ui.painter().rect_filled(sep, 0.0, t.border);
                        ui.add_space(6.0);
                        // An answer from the AI takes the place of the results.
                        if self.search_ask.running || self.search_ask.answer.is_some() {
                            answer_view(ui, &mut self.search_ask, &t);
                            return;
                        }
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            if n == 0 && query.is_empty() {
                                hint(ui, crate::tr!("Tapez pour chercher dans vos pages.", "Type to search your pages."), &t);
                            } else if n == 0 {
                                hint(ui, crate::tr!("Aucune page ne correspond.", "No page matches."), &t);
                            }
                            for (i, (id, snip)) in results.iter().take(n).enumerate() {
                                let Some(p) = self.ws.page(id) else { continue };
                                let resp = result_row(ui, i == self.search_sel, page_icon(p), &p.display_title(), snip, &t);
                                if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                                    self.search_sel = i;
                                }
                                if resp.clicked() || (enter && i == self.search_sel) {
                                    actions.push(Action::Open(id.clone()));
                                    close = true;
                                }
                            }
                        });
                        // Ask the AI: always offered once something is typed; first when nothing matched.
                        if !query.is_empty() {
                            ui.add_space(4.0);
                            let sel = self.search_sel == n;
                            let label = match ai_name {
                                Some(name) => crate::trf!("Demander à {name} : « {} »", "Ask {name}: « {} »", crate::richtext::one_line(&query, 60)),
                                None => crate::tr!("Branchez une IA (Réglages › IA) pour lui poser la question", "Plug in an AI (Settings › AI) to ask it the question").into(),
                            };
                            let resp = ai_row(ui, sel || n == 0, ai_name.is_some(), &label, &t);
                            if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                                self.search_sel = n;
                            }
                            if ai_name.is_some() && (resp.clicked() || (enter && (sel || n == 0))) {
                                ask_now = true;
                            }
                        }
                        // Footer.
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(18.0);
                            ui.label(RichText::new(crate::tr!("↑ ↓ pour choisir · Entrée pour ouvrir · Échap pour fermer", "↑ ↓ to choose · Enter to open · Esc to close")).size(11.5).color(t.tertiary));
                        });
                        ui.add_space(4.0);
                    });
            });
        if ask_now {
            let folder = self
                .current
                .as_ref()
                .and_then(|c| self.ws.project_of(c))
                .and_then(|p| p.project.as_ref())
                .map(|x| x.folder.trim().to_string())
                .filter(|f| !f.is_empty() && crate::project::normalize_folder(f).is_dir())
                .unwrap_or_else(|| dirs::home_dir().map(|h| h.display().to_string()).unwrap_or_default());
            let prefer = if ai_name == Some("Codex") { Some(crate::sessions::Tool::Codex) } else { Some(crate::sessions::Tool::ClaudeCode) };
            self.search_ask.question = query.clone();
            self.search_ask.folder = folder.clone();
            self.search_ask.running = true;
            self.search_ask.answer = None;
            let (slot, c) = (self.search_ask.result.clone(), ctx.clone());
            std::thread::spawn(move || {
                let r = crate::sync::ask(&folder, ask_prompt(&query, &folder), "mcp__orchestra,Read,Glob,Grep,WebSearch,WebFetch", 20, prefer)
                    .map(|(a, tool)| (a, tool.label().to_string()));
                *slot.lock().unwrap() = Some(r);
                c.request_repaint();
            });
        }
        if std::mem::take(&mut self.search_ask.close) {
            self.search_ask.answer = None;
            close = true;
        }
        if close {
            self.search_open = false;
        }
    }
}

fn hint(ui: &mut egui::Ui, text: &str, t: &Tone) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.label(RichText::new(text).size(13.5).color(t.tertiary));
    });
    ui.add_space(8.0);
}

fn result_row(ui: &mut egui::Ui, selected: bool, icon: &str, title: &str, snip: &str, t: &Tone) -> egui::Response {
    let h = if snip.is_empty() { 36.0 } else { 52.0 };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(WIDTH, h), egui::Sense::click());
    let a = ui.ctx().animate_bool_with_time(resp.id.with("sel"), selected, 0.12);
    let r = rect.shrink2(egui::vec2(8.0, 2.0));
    if a > 0.0 {
        ui.painter().rect_filled(r, 9.0, t.hover.gamma_multiply(a));
    }
    let x = r.left() + 14.0 + 3.0 * a;
    let y = if snip.is_empty() { r.center().y } else { r.top() + 15.0 };
    ui.painter().text(egui::pos2(x, y), egui::Align2::LEFT_CENTER, icon, FontId::proportional(15.0), t.text);
    let mut job = egui::text::LayoutJob::single_section(title.to_string(), egui::TextFormat::simple(FontId::proportional(14.5), t.text));
    job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() - 60.0);
    let g = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().galley(egui::pos2(x + 26.0, y - g.size().y / 2.0), g, t.text);
    if !snip.is_empty() {
        let mut job = egui::text::LayoutJob::single_section(snip.to_string(), egui::TextFormat::simple(FontId::proportional(12.5), t.tertiary));
        job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() - 60.0);
        let g = ui.fonts_mut(|f| f.layout_job(job));
        ui.painter().galley(egui::pos2(x + 26.0, r.top() + 29.0), g, t.tertiary);
    }
    if a > 0.5 {
        ui.painter().text(egui::pos2(r.right() - 14.0, r.center().y), egui::Align2::RIGHT_CENTER, "↵", FontId::proportional(13.0), t.tertiary.gamma_multiply(a));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn ai_row(ui: &mut egui::Ui, selected: bool, enabled: bool, label: &str, t: &Tone) -> egui::Response {
    let accent = Color32::from_rgb(144, 101, 176);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(WIDTH, 42.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let a = ui.ctx().animate_bool_with_time(resp.id.with("sel"), selected && enabled, 0.14);
    let r = rect.shrink2(egui::vec2(8.0, 2.0));
    ui.painter().rect_filled(r, 10.0, accent.gamma_multiply(0.08 + 0.14 * a));
    if a > 0.0 {
        ui.painter().rect_stroke(r, 10.0, egui::Stroke::new(1.0, accent.gamma_multiply(0.6 * a)), egui::StrokeKind::Inside);
    }
    let x = r.left() + 14.0 + 3.0 * a;
    let col = if enabled { accent.lerp_to_gamma(t.text, 0.35) } else { t.tertiary };
    icons::paint(ui.painter(), egui::Rect::from_center_size(egui::pos2(x + 8.0, r.center().y), egui::vec2(17.0, 17.0)), Icon::Sparkle, col);
    let mut job = egui::text::LayoutJob::single_section(label.to_string(), egui::TextFormat::simple(FontId::proportional(14.0), col));
    job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() - 70.0);
    let g = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().galley(egui::pos2(x + 26.0, r.center().y - g.size().y / 2.0), g, col);
    if enabled && a > 0.5 {
        ui.painter().text(egui::pos2(r.right() - 14.0, r.center().y), egui::Align2::RIGHT_CENTER, "↵", FontId::proportional(13.0), col.gamma_multiply(a));
    }
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

fn answer_view(ui: &mut egui::Ui, ask: &mut Ask, t: &Tone) {
    egui::Frame::new().inner_margin(egui::Margin { left: 22, right: 22, top: 6, bottom: 10 }).show(ui, |ui| {
        ui.set_width(WIDTH - 44.0);
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(RichText::new(format!("« {} »", ask.question)).size(13.0).color(t.tertiary));
        match &ask.answer {
            None => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(RichText::new(crate::tr!("L'IA lit vos pages et vos projets…", "The AI is reading your pages and projects…")).color(t.secondary));
                });
            }
            Some(Err(e)) if e == crate::sync::AUTH_ERROR => {
                ui.add(egui::Label::new(RichText::new(crate::sync::auth_error_text()).color(Color32::from_rgb(217, 115, 13))).wrap());
                ui.add_space(4.0);
                let (mut login, mut in_term) = (false, false);
                ui.horizontal(|ui| {
                    login = super::projects::action_link(ui, crate::tr!("Se reconnecter à Claude", "Sign in to Claude again")).clicked();
                    in_term = super::projects::action_link(ui, crate::tr!("Poser la question dans le terminal", "Ask the question in the terminal")).clicked();
                });
                if login {
                    super::term_ui::request_with_input(ui.ctx(), crate::tr!("Connexion · Claude Code", "Sign in · Claude Code"), &ask.folder, Some("claude".into()), Some("/login".into()));
                    ask.close = true;
                }
                if in_term {
                    // Interactive Claude Code renews its sign-in by itself.
                    let cmd = format!("claude {}", crate::sessions::shell_quote(&ask_prompt(&ask.question, &ask.folder)));
                    super::term_ui::request(ui.ctx(), crate::tr!("Question · Claude Code", "Question · Claude Code"), &ask.folder, Some(cmd));
                    ask.close = true;
                }
            }
            Some(Err(e)) => {
                ui.label(RichText::new(crate::trf!("Pas de réponse : {e}", "No answer: {e}")).color(Color32::from_rgb(224, 62, 62)));
            }
            Some(Ok((text, by))) => {
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    super::rich::markdown(ui, text, 14.0, t);
                });
                ui.add_space(4.0);
                let mut copy = false;
                let mut again = false;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(crate::trf!("Réponse de {by}", "Answer from {by}")).size(12.0).color(t.tertiary));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        again = super::projects::action_link(ui, crate::tr!("Nouvelle question", "New question")).clicked();
                        copy = super::projects::action_link(ui, crate::tr!("Copier", "Copy")).clicked();
                    });
                });
                if copy {
                    ui.ctx().copy_text(text.clone());
                }
                if again {
                    ask.answer = None;
                }
            }
        }
    });
}
