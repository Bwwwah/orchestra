//! The models each AI able to work alone can be started with: the base for choosing, step by
//! step, the cheapest model able to do the work (later steps class them and pass the choice).
//!
//! Where the AI keeps a list of its own on this computer, that list wins, since it follows its
//! account and its version:
//! - Mistral Vibe: `$VIBE_HOME/model_catalog_cache.json` (the catalog of the account, fetched
//!   by Vibe itself; one entry per account, the newest is read), plus the `[[models]]` the
//!   person added to `config.toml`;
//! - Codex: `$CODEX_HOME/models_cache.json` (the models of the account, fetched by Codex).
//!
//! Claude Code and Gemini CLI keep none: their names are the ones their `--model` option takes,
//! written here (`builtin`). The same goes for Codex and Vibe while their list isn't there yet.

// Read by the next steps (choosing a model per step); until then only the tests use it.
#![allow(dead_code)]

use crate::auto::Agent;
use serde_json::Value;
use std::path::PathBuf;

/// One model an AI can be started with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    /// The name the AI takes: its `--model` option (Claude Code, Codex, Gemini CLI) or
    /// `active_model` of its settings (Mistral Vibe).
    pub id: String,
    /// The name shown to the person.
    pub label: String,
    /// The account's plan doesn't include it (Vibe's catalog says so): listed, never chosen
    /// on its own.
    pub needs_higher_plan: bool,
    /// Light, intermediate or high-end: what the model can be trusted with.
    pub tier: Tier,
    /// Rough public price, in US cents per million output tokens: only meant to compare models,
    /// across AIs too, never shown as a bill.
    pub cost_cents: u32,
}

/// What a model can be trusted with, from the cheapest to the most capable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Short, well-framed work: commit messages, versions, splitting a text into tasks.
    Light,
    /// Ordinary steps: a bounded change in a few files.
    Mid,
    /// Hard steps: design, wide changes, tricky bugs.
    High,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::Light => "léger",
            Tier::Mid => "intermédiaire",
            Tier::High => "haut de gamme",
        }
    }
}

/// The tier and rough cost of a model, from the words of its name and label (`gemini-2.5-flash`,
/// `GPT-5.2 Codex`…). Prices are the public ones per million output tokens, rounded: enough to
/// tell a cheap model from a dear one. A name nobody knows yet counts as intermediate.
pub fn classify(id: &str, label: &str) -> (Tier, u32) {
    let text = format!("{id} {label}").to_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &str| words.contains(&w);
    // Claude
    if has("opus") {
        return (Tier::High, 2500);
    }
    if has("sonnet") {
        return (Tier::Mid, 1500);
    }
    if has("haiku") {
        return (Tier::Light, 500);
    }
    // Gemini (word by word, so « gemini » isn't read as « mini »)
    if has("gemini") {
        return if has("lite") {
            (Tier::Light, 40)
        } else if has("flash") {
            (Tier::Mid, 250)
        } else {
            (Tier::High, 1000)
        };
    }
    // Light words shared by several families: gpt-5-mini, codex-mini, devstral-small…
    if has("nano") || has("tiny") {
        return (Tier::Light, 40);
    }
    if has("mini") {
        return (Tier::Light, 200);
    }
    if has("small") || has("ministral") {
        return (Tier::Light, 30);
    }
    // OpenAI
    if has("gpt") || has("codex") || has("o3") || has("o4") {
        return (Tier::High, 1000);
    }
    // Mistral
    if has("large") {
        return (Tier::High, 600);
    }
    if has("codestral") {
        return (Tier::Mid, 90);
    }
    if has("medium") || has("devstral") || has("magistral") || has("mistral") {
        return (Tier::Mid, 200);
    }
    (Tier::Mid, 500)
}

impl ModelInfo {
    fn new(id: &str, label: &str) -> ModelInfo {
        let (tier, cost_cents) = classify(id, label);
        ModelInfo {
            id: id.to_string(),
            label: label.to_string(),
            needs_higher_plan: false,
            tier,
            cost_cents,
        }
    }
}

/// The names written in Orchestra, when the AI keeps no list on this computer. Claude Code's
/// aliases follow the latest model of each family.
pub fn builtin(agent: Agent) -> Vec<ModelInfo> {
    let list: &[(&str, &str)] = match agent {
        Agent::ClaudeCode => &[
            ("opus", "Claude Opus"),
            ("sonnet", "Claude Sonnet"),
            ("haiku", "Claude Haiku"),
        ],
        Agent::Codex => &[("gpt-5-codex", "GPT-5 Codex"), ("gpt-5", "GPT-5")],
        Agent::Gemini => &[
            ("gemini-2.5-pro", "Gemini 2.5 Pro"),
            ("gemini-2.5-flash", "Gemini 2.5 Flash"),
        ],
        // Vibe's own default, the one of its settings when the person changed nothing.
        Agent::Vibe => &[("mistral-medium-latest", "Mistral Medium")],
    };
    list.iter()
        .map(|(id, label)| ModelInfo::new(id, label))
        .collect()
}

/// The models `agent` can be started with on this computer: its own list when it keeps one,
/// else the names written in Orchestra.
pub fn available(agent: Agent) -> Vec<ModelInfo> {
    let read = |p: PathBuf| std::fs::read_to_string(p).unwrap_or_default();
    let found = match agent {
        Agent::Codex => codex_models(&read(codex_home().join("models_cache.json"))),
        Agent::Vibe => {
            let dir = crate::setup::vibe_config()
                .parent()
                .map(PathBuf::from)
                .unwrap_or_default();
            let mut list = vibe_catalog(&read(dir.join("model_catalog_cache.json")));
            for m in vibe_config_models(&read(crate::setup::vibe_config())) {
                if !list.iter().any(|x| x.id == m.id) {
                    list.push(m);
                }
            }
            list
        }
        Agent::ClaudeCode | Agent::Gemini => Vec::new(),
    };
    if found.is_empty() {
        builtin(agent)
    } else {
        found
    }
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex"))
}

/// Codex's `models_cache.json`: `{"models": [{"slug", "display_name", "visibility"…}]}`; the
/// models it hides from its own picker (`visibility: "hide"`) are left out.
fn codex_models(text: &str) -> Vec<ModelInfo> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut out: Vec<ModelInfo> = Vec::new();
    for m in v["models"].as_array().into_iter().flatten() {
        let Some(id) = m["slug"].as_str().or(m["id"].as_str()).filter(|s| plain(s)) else {
            continue;
        };
        if m["visibility"].as_str() == Some("hide") || out.iter().any(|x| x.id == id) {
            continue;
        }
        let label = m["display_name"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(id);
        out.push(ModelInfo::new(id, label));
    }
    out
}

/// Vibe's `model_catalog_cache.json`: one entry per account, `{"stored_at_timestamp",
/// "payload": {"models": [{"id", "label", "requires"…}]}}`; the newest entry is read. A model
/// with `requires` (`{"kind": "higher_plan"}`) isn't in the account's plan.
fn vibe_catalog(text: &str) -> Vec<ModelInfo> {
    let Ok(Value::Object(entries)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(newest) = entries
        .values()
        .filter(|e| e["payload"]["models"].is_array())
        .max_by_key(|e| e["stored_at_timestamp"].as_u64().unwrap_or(0))
    else {
        return Vec::new();
    };
    let mut out: Vec<ModelInfo> = Vec::new();
    for m in newest["payload"]["models"].as_array().into_iter().flatten() {
        let Some(id) = m["id"].as_str().filter(|s| plain(s)) else {
            continue;
        };
        if out.iter().any(|x| x.id == id) {
            continue;
        }
        let label = m["label"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(id);
        let mut info = ModelInfo::new(id, label);
        info.needs_higher_plan = !m["requires"].is_null();
        out.push(info);
    }
    out
}

/// The `[[models]]` of Vibe's `config.toml`, by their `alias` (the name `active_model` takes),
/// else their `name`.
fn vibe_config_models(text: &str) -> Vec<ModelInfo> {
    let mut out: Vec<ModelInfo> = Vec::new();
    for table in crate::setup::toml_tables(text) {
        let mut lines = table.lines();
        if lines.next().map(str::trim) != Some("[[models]]") {
            continue;
        }
        let (mut name, mut alias) = (None, None);
        for line in lines {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim().trim_matches('"').to_string();
            match k.trim() {
                "name" => name = Some(v),
                "alias" => alias = Some(v),
                _ => {}
            }
        }
        if let Some(id) = alias.or(name).filter(|s| plain(s))
            && !out.iter().any(|x| x.id == id)
        {
            out.push(ModelInfo::new(&id, &id));
        }
    }
    out
}

/// How hard a step looks before its session starts: the tier of model it needs, and why
/// (shown to the person with the choice).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difficulty {
    pub tier: Tier,
    /// Short French reason: the word that decided, or « étape ordinaire ».
    pub reason: String,
}

/// Word stems of hard work: design, wide changes, tricky bugs, security, concurrency.
const HARD: &[&str] = &[
    "architectur",
    "concevoir",
    "conception",
    "refonte",
    "refactor",
    "réécri",
    "migr",
    "sécurit",
    "securit",
    "cve",
    "vulnérab",
    "chiffr",
    "crypt",
    "synchronis",
    "concurren",
    "parallél",
    "thread",
    "race",
    "intermitten",
    "plantage",
    "crash",
    "fuite",
    "leak",
    "deadlock",
    "performance",
    "optimis",
    "algorithm",
    "protocole",
    "design",
];

/// Word stems of short, well-framed work: commit messages, versions, wording, docs.
const EASY: &[&str] = &[
    "commit",
    "version",
    "changelog",
    "renomm",
    "rename",
    "typo",
    "faute",
    "orthograph",
    "tradu",
    "translat",
    "libellé",
    "wording",
    "readme",
    "documentation",
    "doc",
    "commentaire",
    "infobulle",
    "tooltip",
    "icône",
    "couleur",
];

/// Estimate how hard a session's work is, before it starts, from the text it is given: the
/// title of the step it will take (and the mission when the person wrote one). A word of hard
/// work makes it high-end, a word of short work light (unless the text is long: several
/// things to do), anything else intermediate. A hard word wins over a light one.
pub fn estimate(text: &str) -> Difficulty {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let found = |stems: &[&str]| {
        words
            .iter()
            .find(|w| stems.iter().any(|s| w.starts_with(s)))
            .map(|w| w.to_string())
    };
    if let Some(w) = found(HARD) {
        return Difficulty {
            tier: Tier::High,
            reason: format!("travail délicat (« {w} »)"),
        };
    }
    // Long text: several things, or a detailed request: never trusted to a light model.
    let long = words.len() > 30;
    if let Some(w) = found(EASY).filter(|_| !long) {
        return Difficulty {
            tier: Tier::Light,
            reason: format!("travail court (« {w} »)"),
        };
    }
    Difficulty {
        tier: Tier::Mid,
        reason: if long {
            "demande longue".into()
        } else {
            "étape ordinaire".into()
        },
    }
}

/// The difficulty of the session about to start on `pr`: a fix of the CVE watch is high-end;
/// else the mission the person wrote, if any, and the step the AI will take first (the one in
/// progress, else the first to do, skipping those another computer's AI holds). Without
/// either, intermediate.
pub fn estimate_session(
    pr: &crate::project::Project,
    mission: &str,
    fix: bool,
    me: &str,
    now: u64,
) -> Difficulty {
    if fix {
        return Difficulty {
            tier: Tier::High,
            reason: "correctif de sécurité".into(),
        };
    }
    let step = pr
        .next_step_for(me, now)
        .map(|s| s.title.as_str())
        .unwrap_or("");
    let text = format!("{} {step}", mission.trim());
    if text.trim().is_empty() {
        return Difficulty {
            tier: Tier::Mid,
            reason: "aucune étape ouverte".into(),
        };
    }
    estimate(&text)
}

/// A name safe on a command line and in a TOML string: no space, quote or shell sign.
fn plain(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:/@".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ai_has_models_and_the_written_ones_match_the_session_card() {
        for a in Agent::ALL {
            let b = builtin(a);
            assert!(!b.is_empty(), "{a:?}");
            assert!(b.iter().all(|m| plain(&m.id) && !m.label.is_empty()));
            // The « Sessions IA » card offers the same names (Vibe: none, it has no --model).
            if a != Agent::Vibe {
                let ids: Vec<&str> = b.iter().map(|m| m.id.as_str()).collect();
                assert_eq!(ids, a.ai().models(), "{a:?}");
            }
        }
    }

    #[test]
    fn codex_reads_its_cache_and_leaves_out_hidden_models() {
        let text = r#"{"fetched_at":"2026-11-20T10:00:00Z","models":[
            {"slug":"gpt-5.2-codex","display_name":"GPT-5.2 Codex","visibility":"list"},
            {"slug":"gpt-5.2-mini","visibility":"list"},
            {"slug":"internal-x","display_name":"X","visibility":"hide"},
            {"slug":"bad name; rm","display_name":"Y"},
            {"slug":"gpt-5.2-codex","display_name":"twice"}]}"#;
        let m = codex_models(text);
        assert_eq!(
            m,
            vec![
                ModelInfo::new("gpt-5.2-codex", "GPT-5.2 Codex"),
                ModelInfo::new("gpt-5.2-mini", "gpt-5.2-mini")
            ]
        );
        assert!(codex_models("").is_empty() && codex_models("{\"models\":3}").is_empty());
    }

    #[test]
    fn vibe_reads_the_newest_catalog_and_the_plan() {
        let text = r#"{
            "old":{"stored_at_timestamp":100,"payload":{"models":[{"id":"devstral","label":"Devstral"}]}},
            "new":{"stored_at_timestamp":200,"payload":{"models":[
                {"id":"mistral-large-4-0","label":"Mistral Large 4","requires":{"kind":"higher_plan"}},
                {"id":"mistral-medium-latest","label":"Mistral Medium 3.5","requires":null}]}}}"#;
        let m = vibe_catalog(text);
        assert_eq!(m.len(), 2);
        assert_eq!(
            (m[0].id.as_str(), m[0].needs_higher_plan),
            ("mistral-large-4-0", true)
        );
        assert_eq!(
            (m[1].label.as_str(), m[1].needs_higher_plan),
            ("Mistral Medium 3.5", false)
        );
        assert!(vibe_catalog("[]").is_empty());
    }

    #[test]
    fn vibe_adds_the_models_of_its_settings() {
        let text = "active_model = \"local\"\n\n[[models]]\nname = \"devstral-small-2\"\nprovider = \"llamacpp\"\nalias = \"local\"\n\n[[models]]\nname = \"codestral\"\nprovider = \"mistral\"\n\n[[mcp_servers]]\nname = \"fetch\"\n";
        let ids: Vec<String> = vibe_config_models(text).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["local", "codestral"]);
    }

    #[test]
    fn models_are_classed_by_tier_and_cost() {
        let c = |id: &str| classify(id, id);
        assert_eq!(c("opus").0, Tier::High);
        assert_eq!(c("sonnet").0, Tier::Mid);
        assert_eq!(c("haiku").0, Tier::Light);
        assert_eq!(c("gemini-2.5-pro").0, Tier::High);
        assert_eq!(c("gemini-2.5-flash").0, Tier::Mid);
        assert_eq!(c("gemini-2.5-flash-lite").0, Tier::Light);
        assert_eq!(c("gpt-5-codex").0, Tier::High);
        assert_eq!(c("gpt-5.1-codex-mini").0, Tier::Light);
        assert_eq!(c("mistral-large-4-0").0, Tier::High);
        assert_eq!(c("mistral-medium-latest").0, Tier::Mid);
        assert_eq!(c("devstral-small-2").0, Tier::Light);
        assert_eq!(c("something-new"), (Tier::Mid, 500));
        // The label counts too: a Vibe alias says nothing, its label does.
        assert_eq!(classify("local", "Devstral Small").0, Tier::Light);
        // Within a family, the higher tier costs more.
        assert!(c("opus").1 > c("sonnet").1 && c("sonnet").1 > c("haiku").1);
        assert!(c("gemini-2.5-pro").1 > c("gemini-2.5-flash").1);
        // Every written model is classed.
        for a in Agent::ALL {
            for m in builtin(a) {
                assert!(m.cost_cents > 0, "{}", m.id);
            }
        }
        assert_eq!(Tier::High.label(), "haut de gamme");
        assert!(Tier::Light < Tier::Mid && Tier::Mid < Tier::High);
    }

    #[test]
    fn a_step_is_estimated_from_its_words() {
        let t = |s: &str| estimate(s).tier;
        assert_eq!(t("Sync : corriger une fuite de mémoire"), Tier::High);
        assert_eq!(t("Refonte de la synchronisation des pages"), Tier::High);
        assert_eq!(
            t("Barre latérale : le % disparaît de façon intermittente"),
            Tier::High
        );
        assert_eq!(t("Écrire le message de commit"), Tier::Light);
        assert_eq!(t("Préparer la prochaine version"), Tier::Light);
        assert_eq!(t("Ajouter une infobulle sur le bouton"), Tier::Light);
        assert_eq!(
            t("Paramètres : ajouter un bouton pour relancer le tuto"),
            Tier::Mid
        );
        // A hard word wins over a light one.
        assert_eq!(t("Corriger la faute du plantage au démarrage"), Tier::High);
        // A long request is never light.
        let long = format!(
            "Mettre à jour la documentation {}",
            "et encore ceci ".repeat(12)
        );
        assert_eq!(estimate(&long).tier, Tier::Mid);
        assert_eq!(estimate(&long).reason, "demande longue");
        // Accents and case don't matter; the reason names the word.
        let d = estimate("SÉCURITÉ : vérifier les droits");
        assert_eq!(d.tier, Tier::High);
        assert!(d.reason.contains("sécurité"), "{}", d.reason);
        // Stems only match the start of a word: « race » inside « tracer » doesn't count,
        // nor does « comment » (how) mean a code comment.
        assert_eq!(t("Tracer la courbe"), Tier::Mid);
        assert_eq!(t("Comment activer la veille ?"), Tier::Mid);
    }

    #[test]
    fn a_session_is_estimated_from_its_next_step() {
        let mut pr = crate::project::Project::new("/tmp/x");
        assert_eq!(estimate_session(&pr, "", false, "", 0).tier, Tier::Mid);
        let a = pr.add_step("Corriger la typo du titre", "moi");
        pr.add_step("Refonte du moteur de synchronisation", "moi");
        assert_eq!(estimate_session(&pr, "", false, "", 0).tier, Tier::Light);
        // The first step done: the next one decides.
        pr.set_step_status(&a, crate::project::StepStatus::Done);
        assert_eq!(estimate_session(&pr, "", false, "", 0).tier, Tier::High);
        // A fix of the CVE watch is always high-end.
        let mut easy = crate::project::Project::new("/tmp/y");
        easy.add_step("Corriger la typo", "moi");
        let d = estimate_session(&easy, "", true, "", 0);
        assert_eq!(d.tier, Tier::High);
        // The person's mission counts too.
        assert_eq!(
            estimate_session(&easy, "Migrer la base", false, "", 0).tier,
            Tier::High
        );
    }
}
