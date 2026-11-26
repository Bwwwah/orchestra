//! The question asked on the first use, before the tutorial: which AI Orchestra works with.
//! Claude Code (the default: installed and signed in from here if needed), a model running on
//! this computer (Ollama), or a service asking for an API key (Mistral or another
//! OpenAI-compatible server). The last two set `Settings::text_model`; everything can be
//! changed later in Settings › AI.

use super::settings::{Look, dropdown, field, look};
use super::*;
use crate::claude_account::Account;
use crate::model::TextModel;
use crate::setup::{Ai, AiState, Install};
use crate::sync::openai::{OpenAi, Service};

/// The model offered for Mistral, which the person may change.
const MISTRAL_MODEL: &str = "mistral-small-latest";

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub(super) enum Pick {
    #[default]
    ClaudeCode,
    Local,
    ApiKey,
}

/// What the window holds while the person makes up their mind.
#[derive(Clone, Debug)]
pub(super) struct Choice {
    pick: Pick,
    local_model: String,
    /// The API key way: Mistral, or any other compatible server.
    service: Service,
    mistral_model: String,
    other_model: String,
    other_url: String,
    /// Typed here, saved in the Keychain on confirmation only.
    key: String,
}

impl Default for Choice {
    fn default() -> Self {
        Choice {
            pick: Pick::default(),
            local_model: String::new(),
            service: Service::Mistral,
            mistral_model: MISTRAL_MODEL.into(),
            other_model: String::new(),
            other_url: String::new(),
            key: String::new(),
        }
    }
}

impl Choice {
    /// The setting this choice gives (the default one: Claude Code or Codex do the work).
    fn text_model(&self) -> TextModel {
        let (service, url, model) = match self.pick {
            Pick::ClaudeCode => return TextModel::default(),
            Pick::Local => (Service::Ollama, "", &self.local_model),
            Pick::ApiKey if self.service == Service::Mistral => {
                (Service::Mistral, "", &self.mistral_model)
            }
            Pick::ApiKey => (self.service, self.other_url.as_str(), &self.other_model),
        };
        TextModel {
            service: service.id().into(),
            url: url.trim().into(),
            model: model.trim().into(),
        }
    }

    /// What keeps this choice from being confirmed; `saved` is the key already in the
    /// Keychain for its service ("": none).
    fn check(&self, saved: &str) -> Result<(), String> {
        let key = if self.key.trim().is_empty() {
            saved
        } else {
            &self.key
        };
        let key = if self.pick == Pick::ApiKey { key } else { "" };
        if self.pick == Pick::ApiKey
            && self.service == Service::Other
            && self.other_url.trim().is_empty()
        {
            return Err(crate::tr!("Il manque l'adresse du serveur.", "The server's address is missing.").into());
        }
        OpenAi::check_choice(&self.text_model(), key)
    }
}

/// What choosing Claude Code leads to, from where it stands on this computer.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Claude {
    /// Still being looked for.
    Checking,
    /// Not there: this command installs it.
    Install(String),
    /// There, with nobody signed in.
    SignIn,
    /// Nothing left to do here (signed in, or it doesn't tell, or only a page to download
    /// it from: Settings › AI has that button).
    Ready,
}

pub(super) fn claude_next(states: Option<&[AiState]>) -> Claude {
    let Some(states) = states else {
        return Claude::Checking;
    };
    match states.iter().find(|s| s.ai == Ai::ClaudeCode) {
        Some(s) if !s.installed => match &s.install {
            Some(Install::Run(cmd, _)) => Claude::Install(cmd.clone()),
            _ => Claude::Ready,
        },
        Some(s) if s.account == Some(Account::SignedOut) => Claude::SignIn,
        _ => Claude::Ready,
    }
}

/// One of the three ways, as a card that is picked by clicking it.
fn card(ui: &mut egui::Ui, picked: bool, title: &str, text: &str, t: &Tone) -> bool {
    let frame = egui::Frame::new()
        .fill(if picked { t.selected } else { Color32::TRANSPARENT })
        .stroke(egui::Stroke::new(
            if picked { 2.0 } else { 1.0 },
            if picked { super::style::BLUE } else { t.border },
        ))
        .corner_radius(8.0)
        // The text stays in place when the border grows.
        .inner_margin(if picked { 11 } else { 12 });
    let r = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(title).font(semibold(14.0)).color(t.text)).selectable(false));
            ui.add(egui::Label::new(RichText::new(text).size(13.0).color(t.secondary)).wrap().selectable(false));
        })
        .response;
    r.interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// The person's answer to the question.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Answer {
    Confirm,
    Later,
}

/// The question's window; `saved` is the API key already in the Keychain for the service
/// picked ("": none).
fn window(ctx: &egui::Context, c: &mut Choice, claude: &Claude, saved: &str) -> Option<Answer> {
    let dark = ctx.global_style().visuals.dark_mode;
    let (t, l) = (tone(dark), look(dark));
    let screen = ctx.content_rect();
    let width = (screen.width() - 64.0).clamp(240.0, 520.0);
    let mut answer = None;
    egui::Modal::new(egui::Id::unique("first-run"))
        .backdrop_color(Color32::from_black_alpha(112))
        .frame(
            egui::Frame::new()
                .fill(l.window)
                .stroke(egui::Stroke::new(1.0, l.border))
                .corner_radius(12.0)
                .inner_margin(24),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 8.0;
            // The buttons stay under the scrolled part, in sight on a small window.
            egui::ScrollArea::vertical()
                .max_height((screen.height() - 150.0).max(120.0))
                .show(ui, |ui| question(ui, c, claude, saved, &t, &l));
            let ready = c.check(saved).is_ok()
                && !(c.pick == Pick::ClaudeCode && *claude == Claude::Checking);
            let label = match (c.pick, claude) {
                (Pick::ClaudeCode, Claude::Install(_)) => {
                    crate::tr!("Installer Claude Code", "Install Claude Code")
                }
                (Pick::ClaudeCode, Claude::SignIn) => crate::tr!("Se connecter", "Sign in"),
                _ => crate::tr!("Continuer", "Continue"),
            };
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if super::style::quiet(ui, crate::tr!("Plus tard", "Later")).clicked() {
                    answer = Some(Answer::Later);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(ready, |ui| {
                        if super::style::primary(ui, label).clicked() {
                            answer = Some(Answer::Confirm);
                        }
                    });
                });
            });
        });
    answer
}

/// The three ways and what the one picked needs.
fn question(ui: &mut egui::Ui, c: &mut Choice, claude: &Claude, saved: &str, t: &Tone, l: &Look) {
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.label(
        RichText::new(crate::tr!("Avec quelle IA travailler ?", "Which AI to work with?"))
            .font(bold(22.0))
            .color(t.text),
    );
    ui.label(
        RichText::new(crate::tr!(
            "Vous pourrez changer à tout moment dans Paramètres › IA.",
            "You can change at any time in Settings › AI."
        ))
        .size(14.0)
        .color(t.secondary),
    );
    for (pick, title, text) in [
        (
            Pick::ClaudeCode,
            "Claude Code",
            crate::tr!(
                "Avec votre compte Claude. Les IA lisent vos dossiers et peuvent avancer seules sur les étapes.",
                "With your Claude account. The AIs read your folders and can work on the steps on their own."
            ),
        ),
        (
            Pick::Local,
            crate::tr!("Modèle local", "Local model"),
            crate::tr!(
                "Avec Ollama, un modèle tourne sur cet ordinateur. Il remplit les projets, suggère des étapes et discute des projets sans code.",
                "With Ollama, a model runs on this computer. It fills projects in, suggests steps and chats about projects without code."
            ),
        ),
        (
            Pick::ApiKey,
            crate::tr!("Clé API", "API key"),
            crate::tr!(
                "Mistral ou un autre serveur compatible OpenAI, pour les mêmes tâches qu'un modèle local. Le service reçoit ce que vous lui demandez de lire et peut facturer l'usage.",
                "Mistral or another OpenAI-compatible server, for the same tasks as a local model. The service receives what you ask it to read and may bill the use."
            ),
        ),
    ] {
        if card(ui, c.pick == pick, title, text, t) {
            c.pick = pick;
        }
    }
    ui.add_space(4.0);
    let note = |ui: &mut egui::Ui, text: &str| {
        ui.add(egui::Label::new(RichText::new(text).size(13.0).color(t.secondary)).wrap());
    };
    let input = |ui: &mut egui::Ui, name: &str, value: &mut String, hint: &str, secret: bool| {
        ui.label(RichText::new(name).size(13.0).color(t.text));
        ui.add(field(value, hint, f32::INFINITY, t, l).password(secret));
    };
    match c.pick {
        Pick::ClaudeCode => note(
            ui,
            match claude {
                Claude::Checking => crate::tr!(
                    "Recherche de Claude Code sur cet ordinateur…",
                    "Looking for Claude Code on this computer…"
                ),
                Claude::Install(_) => crate::tr!(
                    "Claude Code n'est pas installé. Son installation se lance dans le terminal d'Orchestra, puis la connexion à votre compte s'ouvre dans votre navigateur.",
                    "Claude Code isn't installed. Its install runs in Orchestra's terminal, then signing in to your account opens in your browser."
                ),
                Claude::SignIn => crate::tr!(
                    "Claude Code est installé, mais aucun compte n'est connecté. La connexion se fait dans votre navigateur ; Orchestra ne voit ni mot de passe ni jeton.",
                    "Claude Code is installed, but no account is signed in. You sign in in your browser; Orchestra sees no password or token."
                ),
                Claude::Ready => crate::tr!(
                    "L'état de Claude Code et de votre compte est affiché dans Paramètres › IA.",
                    "Claude Code and your account are shown in Settings › AI."
                ),
            },
        ),
        Pick::Local => {
            input(
                ui,
                crate::tr!(
                    "Nom du modèle, tel que « ollama list » l'affiche",
                    "Model name, as « ollama list » shows it"
                ),
                &mut c.local_model,
                "qwen3",
                false,
            );
            note(
                ui,
                crate::tr!(
                    "Ollama doit être installé et lancé. Les sessions autonomes demandent toujours Claude Code ou Codex.",
                    "Ollama has to be installed and running. Autonomous sessions still need Claude Code or Codex."
                ),
            );
        }
        Pick::ApiKey => {
            let current = usize::from(c.service == Service::Other);
            let services = [
                "Mistral",
                crate::tr!("Autre serveur compatible OpenAI", "Other OpenAI-compatible server"),
            ];
            if let Some(i) = dropdown(ui, "first-run-service", &services, current, t, l)
                && i != current
            {
                c.service = [Service::Mistral, Service::Other][i];
                // A key typed for one service never goes to the other.
                c.key.clear();
            }
            if c.service == Service::Mistral {
                input(ui, crate::tr!("Modèle", "Model"), &mut c.mistral_model, MISTRAL_MODEL, false);
            } else {
                input(
                    ui,
                    crate::tr!(
                        "Adresse, ce qui précède « /chat/completions »",
                        "Address, what comes before « /chat/completions »"
                    ),
                    &mut c.other_url,
                    "https://…/v1",
                    false,
                );
                input(ui, crate::tr!("Modèle", "Model"), &mut c.other_model, "", false);
            }
            let hint = if saved.is_empty() {
                ""
            } else {
                crate::tr!("une clé est déjà enregistrée", "a key is already saved")
            };
            input(ui, crate::tr!("Clé API", "API key"), &mut c.key, hint, true);
            note(
                ui,
                crate::tr!(
                    "La clé est gardée dans le trousseau, jamais dans vos données. Les sessions autonomes demandent toujours Claude Code ou Codex.",
                    "The key is kept in the Keychain, never in your data. Autonomous sessions still need Claude Code or Codex."
                ),
            );
        }
    }
    // Why « Continue » is greyed out, said once something is typed, not on the empty form.
    let typed = match c.pick {
        Pick::ClaudeCode => false,
        Pick::Local => !c.local_model.trim().is_empty(),
        Pick::ApiKey => !c.key.trim().is_empty(),
    };
    if typed && let Err(why) = c.check(saved) {
        ui.add(egui::Label::new(RichText::new(why).size(13.0).color(super::style::ORANGE)).wrap());
    }
}

impl NotesApp {
    /// Draw the question, if it is waiting for an answer.
    pub(super) fn first_run_ui(&mut self, ctx: &egui::Context) {
        let Some(mut c) = self.first_run.take() else {
            // The tutorial waits for the install or the sign-in under way in the terminal,
            // which its dimming would cover.
            if self.tour_waits && self.ais.installing.is_none() && self.ais.signing_in.is_none() {
                self.tour_waits = false;
                self.start_tour();
            }
            return;
        };
        let claude = claude_next(self.ais.states.lock_or_recover().as_deref());
        let key_var = match c.pick {
            Pick::ApiKey => c.service.key_var(),
            _ => None,
        };
        let saved = key_var.map(crate::secret::api_key).unwrap_or_default();
        let answer = window(ctx, &mut c, &claude, &saved);
        if answer == Some(Answer::Later) {
            self.close_first_run();
        } else if answer == Some(Answer::Confirm) {
            // The service may have changed in this very frame.
            let key_var = match c.pick {
                Pick::ApiKey => c.service.key_var(),
                _ => None,
            };
            if let Some(var) = key_var
                && !c.key.trim().is_empty()
                && let Err(e) = crate::secret::set_api_key(var, &c.key)
            {
                self.fail(crate::trf!(
                    "Clé API non enregistrée dans le trousseau : {e}",
                    "The API key couldn't be changed in the Keychain: {e}"
                ));
                self.first_run = Some(c);
                return;
            }
            self.ws.settings.text_model = c.text_model();
            if c.pick == Pick::ClaudeCode {
                self.start_claude(ctx, claude);
            }
            self.close_first_run();
        } else {
            self.first_run = Some(c);
        }
    }

    /// Run in the terminal what Claude Code still needs, and watch it get done.
    fn start_claude(&mut self, ctx: &egui::Context, next: Claude) {
        let (title, cmd) = match next {
            Claude::Install(cmd) => {
                self.watch_install(Ai::ClaudeCode);
                (crate::tr!("Installation · Claude Code", "Installing · Claude Code"), cmd)
            }
            Claude::SignIn => {
                self.watch_sign_in();
                (
                    crate::tr!("Connexion · Claude Code", "Sign in · Claude Code"),
                    crate::claude_account::LOGIN_COMMAND.to_string(),
                )
            }
            Claude::Checking | Claude::Ready => return,
        };
        let home = dirs::home_dir()
            .map(|h| h.display().to_string())
            .unwrap_or_default();
        super::term_ui::request(ctx, title, &home, Some(cmd));
        ctx.request_repaint();
    }

    /// The question is answered (or put off): never asked again, the tutorial comes next.
    fn close_first_run(&mut self) {
        self.first_run = None;
        self.ws.settings.ai_choice_pending = false;
        self.dirty_since.get_or_insert_with(Instant::now);
        self.tour_waits = true;
        self.ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn with(pick: Pick) -> Choice {
        Choice {
            pick,
            ..Default::default()
        }
    }

    #[test]
    fn claude_code_keeps_the_usual_setting() {
        let c = with(Pick::ClaudeCode);
        assert!(c.text_model().is_unset());
        assert!(c.check("").is_ok());
    }

    #[test]
    fn a_local_model_needs_its_name() {
        let mut c = with(Pick::Local);
        assert!(c.check("").is_err());
        c.local_model = " qwen3 ".into();
        assert!(c.check("").is_ok());
        let m = c.text_model();
        assert_eq!((m.service.as_str(), m.url.as_str(), m.model.as_str()), ("ollama", "", "qwen3"));
    }

    #[test]
    fn mistral_needs_a_key_typed_or_saved() {
        let mut c = with(Pick::ApiKey);
        assert!(c.check("").is_err());
        assert!(c.check("saved").is_ok());
        c.key = "typed".into();
        assert!(c.check("").is_ok());
        assert_eq!(c.text_model().model, MISTRAL_MODEL);
        assert_eq!(c.text_model().url, "");
    }

    #[test]
    fn another_server_needs_an_address_and_never_gets_the_key_in_clear() {
        let mut c = with(Pick::ApiKey);
        c.service = Service::Other;
        c.other_model = "m".into();
        assert!(c.check("").is_err(), "no address");
        c.other_url = "http://example.com/v1".into();
        assert!(c.check("").is_ok(), "no key to protect");
        c.key = "secret".into();
        assert!(c.check("").is_err(), "a key over plain http");
        c.other_url = "https://example.com/v1".into();
        assert!(c.check("").is_ok());
        assert_eq!(c.text_model().service, "openai-compatible");
    }

    /// The window alone, in a window of `size`: what is picked, and the last answer.
    fn harness(size: [f32; 2]) -> Harness<'static, (Choice, Option<Answer>)> {
        // Drawn only once the app's fonts are in.
        let ready = std::rc::Rc::new(std::cell::Cell::new(false));
        let fonts = ready.clone();
        let mut h = Harness::builder().with_size(size).build_ui_state(
            move |ui, (c, answer): &mut (Choice, Option<Answer>)| {
                if fonts.get()
                    && let Some(a) = window(ui.ctx(), c, &Claude::Ready, "")
                {
                    *answer = Some(a);
                }
            },
            (Choice::default(), None),
        );
        super::super::setup_fonts(&h.ctx);
        h.run_steps(2);
        ready.set(true);
        h.run_steps(4);
        h
    }

    fn click(h: &mut Harness<'static, (Choice, Option<Answer>)>, label: &str) {
        h.get_by_label(label).click();
        h.run_steps(3);
    }

    #[test]
    fn a_card_is_picked_by_clicking_it() {
        let mut h = harness([1000.0, 800.0]);
        click(&mut h, crate::tr!("Modèle local", "Local model"));
        assert_eq!(h.state().0.pick, Pick::Local);
        click(&mut h, crate::tr!("Clé API", "API key"));
        assert_eq!(h.state().0.pick, Pick::ApiKey);
        click(&mut h, "Claude Code");
        assert_eq!(h.state().0.pick, Pick::ClaudeCode);
        click(&mut h, crate::tr!("Continuer", "Continue"));
        assert_eq!(h.state().1, Some(Answer::Confirm));
    }

    /// On the smallest window, the buttons stay in sight under the scrolled question; an
    /// unfinished choice can't be confirmed, only put off.
    #[test]
    fn the_buttons_stay_in_sight_on_a_small_window() {
        let size = [original::MIN_WINDOW_WIDTH, original::MIN_WINDOW_HEIGHT];
        let mut h = harness(size);
        h.state_mut().0.pick = Pick::ApiKey;
        h.state_mut().0.service = Service::Other;
        h.run_steps(4);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size.into());
        for label in [crate::tr!("Continuer", "Continue"), crate::tr!("Plus tard", "Later")] {
            let r = h.get_by_label(label).rect();
            assert!(screen.contains_rect(r), "{label}: {r:?}");
        }
        click(&mut h, crate::tr!("Continuer", "Continue"));
        assert_eq!(h.state().1, None, "nothing chosen yet");
        click(&mut h, crate::tr!("Plus tard", "Later"));
        assert_eq!(h.state().1, Some(Answer::Later));
    }

    #[test]
    fn claude_code_is_installed_or_signed_in_as_needed() {
        let state = |installed, install, account| AiState {
            ai: Ai::ClaudeCode,
            installed,
            plugged: false,
            install,
            account,
        };
        assert_eq!(claude_next(None), Claude::Checking);
        let run = Some(Install::Run("get claude".into(), ""));
        assert_eq!(
            claude_next(Some(&[state(false, run, None)])),
            Claude::Install("get claude".into())
        );
        let out = state(true, None, Some(Account::SignedOut));
        assert_eq!(claude_next(Some(&[out])), Claude::SignIn);
        // An older Claude Code that doesn't tell its account: nothing to start.
        assert_eq!(claude_next(Some(&[state(true, None, None)])), Claude::Ready);
    }
}
