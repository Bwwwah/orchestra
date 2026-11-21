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
}

impl ModelInfo {
    fn new(id: &str, label: &str) -> ModelInfo {
        ModelInfo {
            id: id.to_string(),
            label: label.to_string(),
            needs_higher_plan: false,
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
}
