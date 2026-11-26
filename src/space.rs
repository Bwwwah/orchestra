//! Spaces: "Mon espace" (local, private) and, later, one space per server joined.
//!
//! Each space has its own folder, so nothing of one space can reach another: "Mon espace" keeps
//! the data folder itself (files of older versions stay where they are), the others live in
//! `espaces/<id>/`. The list of spaces is kept in `espaces.json` in the data folder; what is
//! personal (AI usage, worktrees, the computer's id…) stays in the data folder, for all spaces.
//!
//! Which space a process works in is chosen by that process (`store::use_space`), never read
//! from the list: a program that chooses nothing (an MCP server, an autonomous session) works
//! in "Mon espace", never in a shared space by default. See docs/collaboration-serveur.md.

use crate::project::ProjectMember;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The id of "Mon espace".
pub const LOCAL: &str = "local";

/// Where a space's pages come from.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SpaceKind {
    /// On this computer only.
    #[default]
    Local,
    /// A copy of a team server's space (team.rs): its address, and the invitation until it
    /// is spent on the first visit.
    Server {
        url: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        invite: String,
    },
}

/// Which AIs may work on a space's projects. Set by the space's administrator (by hand for
/// now, by the server later). Orchestra keeps to it; it can't stop someone from running an
/// AI on the same folder outside Orchestra.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiRule {
    /// The person's own AIs, and the space's own if it offers some.
    #[default]
    Free,
    /// Only the AIs the space provides.
    SpaceOnly,
    /// No AI on this space's projects.
    NoAi,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Space {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: SpaceKind,
    #[serde(default)]
    pub ai_rule: AiRule,
    /// With `AiRule::Free`: the person's own AIs allowed here, by key (`Ai::key`); empty means
    /// all of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_ai: Vec<String>,
    /// A team space: the role its server gives this person (`team::Role::key`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// The AIs the space provides to this person (from its server), with their key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ais: Vec<SpaceAi>,
}

/// An AI a space provides: one of the coding agents (Claude Code, Codex) running on this
/// computer, its calls to the model going to the company's own (vLLM, Ollama, LiteLLM,
/// Bedrock…) with a key given to this person. Orchestra hands out the settings; the company's
/// gateway does the rest (and is where a key is revoked for good).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct SpaceAi {
    pub name: String,
    /// The agent: `claude-code` or `codex` (`Ai::key`).
    pub tool: String,
    /// The model's address: Anthropic Messages format for Claude Code, OpenAI for Codex.
    pub base_url: String,
    /// The model to ask for (empty: the gateway's default).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Codex only: `responses` (default) or `chat`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wire_api: String,
    /// This person's key (only in the app's copy; the server keeps one per member).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
}

/// How the id of an AI's membership starts: a person's is their key, in hexadecimal, so the
/// two never meet.
pub const AI_MEMBER_PREFIX: &str = "ia:";

/// What it takes to run an agent on a space's AI: environment variables (the key goes there,
/// never on a command line) and options placed right after the program's name.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SpaceAiRun {
    pub env: Vec<(String, String)>,
    pub args: Vec<String>,
    /// Options that must come after everything else on the command line (Codex: the session
    /// token for Orchestra's MCP server, after a command that sets its servers).
    pub late_args: Vec<String>,
}

/// The environment variable naming a session Orchestra started on a space's AI.
pub const SESSION_VAR: &str = "ORCHESTRA_SESSION";
/// How long such a session's token is good for.
const SESSION_LIFE_MS: u64 = 48 * 3600 * 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionGrant {
    space: String,
    at: u64,
}

fn grants_path() -> PathBuf {
    crate::store::data_dir().join(".sessions-espace.json")
}

static GRANTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A new token for a session Orchestra starts on space `space`'s own AI. Orchestra's MCP
/// server, in a space that allows only its own AIs, serves only sessions with one: an agent
/// started by hand (on the person's subscription) has none.
pub fn grant_session(space: &str) -> String {
    let _g = GRANTS.lock().unwrap_or_else(|e| e.into_inner());
    let token = crate::identity::random_hex(16);
    let now = crate::model::now_ms();
    let mut all: std::collections::BTreeMap<String, SessionGrant> =
        std::fs::read_to_string(grants_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
    all.retain(|_, g| now.saturating_sub(g.at) < SESSION_LIFE_MS);
    all.insert(
        token.clone(),
        SessionGrant {
            space: space.to_string(),
            at: now,
        },
    );
    let _ = crate::store::write_atomic(
        &grants_path(),
        serde_json::to_string(&all).unwrap_or_default().as_bytes(),
    );
    token
}

/// Was `token` given by Orchestra to a session on space `space`'s own AI, not long ago?
pub fn granted(token: &str, space: &str) -> bool {
    if token.trim().is_empty() {
        return false;
    }
    let all: std::collections::BTreeMap<String, SessionGrant> =
        std::fs::read_to_string(grants_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
    all.get(token.trim()).is_some_and(|g| {
        g.space == space && crate::model::now_ms().saturating_sub(g.at) < SESSION_LIFE_MS
    })
}

/// The environment variable holding the key for Codex (`env_key` of its provider).
pub const CODEX_KEY_VAR: &str = "ORCHESTRA_ESPACE_CLE";

impl SpaceAi {
    /// It as a member of the space's projects, one work can be given to. The server knows its
    /// AIs by name: the id is the same on every computer.
    pub fn member(&self) -> ProjectMember {
        ProjectMember {
            id: format!("{AI_MEMBER_PREFIX}{}", self.name),
            name: crate::richtext::one_line(&self.name, crate::project::RECEIVED_AUTHOR_MAX),
            role: crate::team::Role::Member,
            ai: self.tool.clone(),
        }
    }

    /// How to run its agent on it; None for an agent Orchestra can't point elsewhere.
    pub fn run(&self) -> Option<SpaceAiRun> {
        if self.key.is_empty() || self.base_url.trim().is_empty() {
            return None;
        }
        let s = |v: &str| serde_json::to_string(v).unwrap_or_default();
        match self.tool.as_str() {
            "claude-code" => {
                let mut env = vec![
                    (
                        "ANTHROPIC_BASE_URL".to_string(),
                        self.base_url.trim().to_string(),
                    ),
                    ("ANTHROPIC_AUTH_TOKEN".to_string(), self.key.clone()),
                ];
                if !self.model.trim().is_empty() {
                    env.push(("ANTHROPIC_MODEL".into(), self.model.trim().to_string()));
                }
                Some(SpaceAiRun {
                    env,
                    args: vec![],
                    late_args: vec![],
                })
            }
            "codex" => {
                let wire = if self.wire_api.trim().is_empty() {
                    "responses"
                } else {
                    self.wire_api.trim()
                };
                let mut args = vec![
                    "-c".to_string(),
                    "model_provider=\"orchestra_espace\"".to_string(),
                    "-c".to_string(),
                    format!(
                        "model_providers.orchestra_espace={{name={},base_url={},env_key={},wire_api={}}}",
                        s(&self.name),
                        s(self.base_url.trim()),
                        s(CODEX_KEY_VAR),
                        s(wire)
                    ),
                ];
                if !self.model.trim().is_empty() {
                    args.push("-m".into());
                    args.push(self.model.trim().to_string());
                }
                Some(SpaceAiRun {
                    env: vec![(CODEX_KEY_VAR.to_string(), self.key.clone())],
                    args,
                    late_args: vec![],
                })
            }
            _ => None,
        }
    }
}

/// `args` without a choice of model (`-m x`, `--model x`, `--model=x`): a space's AI has its
/// own, and the gateway may not know the person's usual ones.
pub fn without_model(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if a == "-m" || a == "--model" {
            skip = true;
            continue;
        }
        if a.starts_with("--model=") {
            continue;
        }
        out.push(a);
    }
    out
}

impl Space {
    fn local() -> Self {
        Space {
            id: LOCAL.into(),
            name: crate::tr!("Mon espace", "My workspace").into(),
            kind: SpaceKind::Local,
            ai_rule: AiRule::Free,
            allowed_ai: vec![],
            role: String::new(),
            ais: vec![],
        }
    }

    pub fn is_local(&self) -> bool {
        self.id == LOCAL
    }

    /// The team server it is a copy of: (address, invitation not spent yet).
    pub fn server(&self) -> Option<(&str, &str)> {
        match &self.kind {
            SpaceKind::Server { url, invite } => Some((url.as_str(), invite.as_str())),
            SpaceKind::Local => None,
        }
    }

    /// The AI this space provides for agent `tool` (`Ai::key`), ready to run.
    pub fn space_ai(&self, tool: &str) -> Option<&SpaceAi> {
        if self.ai_rule == AiRule::NoAi {
            return None;
        }
        self.ais
            .iter()
            .find(|a| a.tool == tool && a.run().is_some())
    }

    /// The space's AIs as members: those this person can run here (none with « no AI »).
    pub fn ai_members(&self) -> Vec<ProjectMember> {
        if self.ai_rule == AiRule::NoAi {
            return vec![];
        }
        self.ais
            .iter()
            .filter(|a| a.run().is_some())
            .map(SpaceAi::member)
            .collect()
    }

    /// May agent `tool` work here at all: on the space's AI if it provides one for it (that
    /// one first), else as the person's own?
    pub fn may_run(&self, tool: &str) -> bool {
        self.space_ai(tool).is_some() || self.allows_own_ai(tool)
    }

    /// May the person's own AI `key` (`Ai::key`) work on this space's projects?
    /// "Mon espace" allows them all, whatever its file says.
    pub fn allows_own_ai(&self, key: &str) -> bool {
        if self.is_local() {
            return true;
        }
        match self.ai_rule {
            AiRule::Free => self.allowed_ai.is_empty() || self.allowed_ai.iter().any(|a| a == key),
            AiRule::SpaceOnly | AiRule::NoAi => false,
        }
    }
}

/// The list of spaces, "Mon espace" always first.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Spaces {
    /// The space the window shows (only the window reads it).
    #[serde(default = "local_id")]
    pub active: String,
    #[serde(default)]
    pub spaces: Vec<Space>,
}

fn local_id() -> String {
    LOCAL.into()
}

impl Default for Spaces {
    fn default() -> Self {
        Spaces {
            active: local_id(),
            spaces: vec![Space::local()],
        }
    }
}

impl Spaces {
    /// "Mon espace" first and exactly once, ids unique, `active` naming a known space.
    fn tidy(mut self) -> Self {
        let mut seen = std::collections::HashSet::new();
        self.spaces
            .retain(|s| !s.id.trim().is_empty() && seen.insert(s.id.clone()));
        match self.spaces.iter().position(|s| s.is_local()) {
            Some(0) => {}
            Some(i) => {
                let l = self.spaces.remove(i);
                self.spaces.insert(0, l);
            }
            None => self.spaces.insert(0, Space::local()),
        }
        // "Mon espace" is always on this computer and open to the person's AIs.
        self.spaces[0].kind = SpaceKind::Local;
        self.spaces[0].ai_rule = AiRule::Free;
        self.spaces[0].allowed_ai.clear();
        if !self.spaces.iter().any(|s| s.id == self.active) {
            self.active = local_id();
        }
        self
    }

    pub fn get(&self, id: &str) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }

    /// Add a space; returns its id.
    pub fn add(&mut self, name: &str, kind: SpaceKind) -> String {
        let id = crate::model::new_id();
        self.spaces.push(Space {
            id: id.clone(),
            name: name.trim().to_string(),
            kind,
            ai_rule: AiRule::Free,
            allowed_ai: vec![],
            role: String::new(),
            ais: vec![],
        });
        id
    }

    /// Take a space off the list ("Mon espace" can't be). Its folder is left on disk.
    pub fn remove(&mut self, id: &str) -> bool {
        if id == LOCAL {
            return false;
        }
        let before = self.spaces.len();
        self.spaces.retain(|s| s.id != id);
        if self.active == id {
            self.active = local_id();
        }
        self.spaces.len() != before
    }
}

/// The space this process works in (see `store::use_space`); "Mon espace" if it is gone
/// from the list.
pub fn current() -> Space {
    let id = crate::store::space_id();
    load().get(&id).cloned().unwrap_or_else(Space::local)
}

/// Why the person's own AI `key` (`Ai::key`) may not work here, in the space this process
/// works in; None when it may.
pub fn refusal(key: &str, label: &str) -> Option<String> {
    let s = current();
    if s.may_run(key) {
        return None;
    }
    let name = s.name.clone();
    Some(match s.ai_rule {
        AiRule::NoAi => crate::trf!(
            "l'espace « {name} » n'autorise aucune IA",
            "the space « {name} » allows no AI"
        ),
        AiRule::SpaceOnly => crate::trf!(
            "l'espace « {name} » n'autorise que ses propres IA, et n'en fournit pas pour {label}",
            "the space « {name} » only allows its own AIs, and provides none for {label}"
        ),
        AiRule::Free => crate::trf!(
            "l'espace « {name} » n'autorise pas {label}",
            "the space « {name} » doesn't allow {label}"
        ),
    })
}

/// The space holding the project of `folder`: the one whose project folder is the closest
/// to it; "Mon espace" when no space has one, or when it has one as close as the others.
pub fn space_for_folder(folder: &std::path::Path) -> String {
    let mut best: (usize, String) = (0, LOCAL.to_string());
    for s in load().spaces {
        let path = crate::store::workspace_path_in(&s.id);
        if !path.exists() {
            continue;
        }
        let ws = crate::store::load_path(&path);
        let depth = ws
            .project_for_path(folder)
            .and_then(|p| p.project.as_ref())
            .map(|x| {
                crate::project::normalize_folder(&x.folder)
                    .components()
                    .count()
            });
        // Strictly closer only: on a tie, the first one (« Mon espace ») is kept.
        if let Some(n) = depth
            && n > best.0
        {
            best = (n, s.id.clone());
        }
    }
    best.1
}

/// The key (`Ai::key`) of the AI behind an MCP client, from the name it gives when it
/// connects (`clientInfo.name`) or the name of an autonomous session (ORCHESTRA_HOLDER).
pub fn ai_key_of_client(name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    if n.contains("claude-code") || n.contains("claude code") {
        Some("claude-code")
    } else if n.contains("codex") {
        Some("codex")
    } else if n.contains("vibe") || n.contains("mistral") {
        Some("vibe")
    } else if n.contains("antigravity") || n.contains("gemini") || n == "agy" {
        Some("antigravity")
    } else if n.contains("cursor") {
        Some("cursor")
    } else if n.contains("claude") {
        Some("claude-desktop")
    } else {
        None
    }
}

/// How to run agent `tool` on the AI the space this process works in provides for it, if
/// it provides one.
/// A session token comes with it (`grant_session`): in the environment (Claude Code hands its
/// environment to its MCP servers) and, for Codex, in its MCP server's settings.
pub fn ai_run(tool: &str) -> Option<SpaceAiRun> {
    let sp = current();
    let mut run = sp.space_ai(tool).and_then(|a| a.run())?;
    let token = grant_session(&sp.id);
    if tool == "codex" {
        run.late_args = vec![
            "-c".into(),
            format!(
                "mcp_servers.{}.env.{SESSION_VAR}={}",
                crate::setup::NAME,
                serde_json::to_string(&token).unwrap_or_default()
            ),
        ];
    }
    run.env.push((SESSION_VAR.to_string(), token));
    Some(run)
}

fn list_path() -> PathBuf {
    crate::store::data_dir().join("espaces.json")
}

/// The list as last read or written, with the file's time then: the window asks for it at
/// every frame, the file is read again only when it changed.
static CACHE: std::sync::Mutex<Option<(Option<std::time::SystemTime>, Spaces)>> =
    std::sync::Mutex::new(None);

/// The list of spaces; just "Mon espace" when there is none yet or it can't be read.
pub fn load() -> Spaces {
    let path = list_path();
    let m = crate::store::modified(&path);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, s)) = cache.as_ref()
        && *at == m
        && m.is_some()
    {
        return s.clone();
    }
    let s = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Spaces>(&s).ok())
        .unwrap_or_default()
        .tidy();
    *cache = Some((m, s.clone()));
    s
}

pub fn save(spaces: &Spaces) -> std::io::Result<()> {
    std::fs::create_dir_all(crate::store::data_dir())?;
    let tidy = spaces.clone().tidy();
    let text = serde_json::to_string_pretty(&tidy).unwrap_or_default();
    let path = list_path();
    crate::store::write_atomic(&path, text.as_bytes())?;
    // Written here: the next read takes it as is (the file's time may not have moved).
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((crate::store::modified(&path), tidy));
    Ok(())
}

/// The folder of space `id`: the data folder for "Mon espace", `espaces/<id>/` for the others.
/// An id that isn't a plain name (`..`, a slash…) can never point outside `espaces/`.
pub fn dir_of(id: &str) -> PathBuf {
    let root = crate::store::data_dir();
    if id == LOCAL {
        return root;
    }
    root.join("espaces").join(crate::store::safe_name(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mon_espace_is_always_first_local_and_open_to_own_ais() {
        let s: Spaces = serde_json::from_str(
            r#"{"active":"gone","spaces":[
                {"id":"a","name":"Boîte","kind":{"type":"server","url":"https://x"},"ai_rule":"no_ai"},
                {"id":"a","name":"Doublon"},
                {"id":"local","name":"Moi","kind":{"type":"server","url":"https://y"},"ai_rule":"no_ai"}
            ]}"#,
        )
        .unwrap();
        let s = s.tidy();
        assert_eq!(s.spaces.len(), 2);
        assert!(s.spaces[0].is_local());
        assert_eq!(s.spaces[0].name, "Moi");
        assert_eq!(s.spaces[0].kind, SpaceKind::Local);
        assert!(s.spaces[0].allows_own_ai("claude"));
        assert_eq!(s.active, LOCAL);
        assert_eq!(s.spaces[1].name, "Boîte");
    }

    #[test]
    fn missing_list_gives_mon_espace_alone() {
        let s = Spaces::default().tidy();
        assert_eq!(s.spaces.len(), 1);
        assert_eq!(s.active, LOCAL);
    }

    #[test]
    fn ai_rule_of_a_shared_space() {
        let mut s = Spaces::default();
        let id = s.add(
            "Boîte",
            SpaceKind::Server {
                url: "ws://o.example".into(),
                invite: String::new(),
            },
        );
        let sp = |s: &Spaces| s.get(&id).unwrap().clone();
        assert!(sp(&s).allows_own_ai("codex"));
        s.spaces[1].allowed_ai = vec!["claude".into()];
        assert!(sp(&s).allows_own_ai("claude"));
        assert!(!sp(&s).allows_own_ai("codex"));
        s.spaces[1].ai_rule = AiRule::SpaceOnly;
        assert!(!sp(&s).allows_own_ai("claude"));
        s.spaces[1].ai_rule = AiRule::NoAi;
        assert!(!sp(&s).allows_own_ai("claude"));
    }

    #[test]
    fn spaces_have_their_own_folders() {
        assert_eq!(dir_of(LOCAL), crate::store::data_dir());
        let a = dir_of("abc");
        assert!(a.starts_with(crate::store::data_dir().join("espaces")));
        assert_ne!(a, dir_of("abd"));
        for evil in ["..", "../..", "a/../../b", "/etc"] {
            assert!(
                dir_of(evil).starts_with(crate::store::data_dir().join("espaces")),
                "{evil}"
            );
            assert_ne!(
                dir_of(evil),
                crate::store::data_dir().join("espaces"),
                "{evil}"
            );
        }
    }

    #[test]
    fn a_process_that_chooses_nothing_works_in_mon_espace() {
        // Nothing in the tests calls `store::use_space`: the MCP server's case.
        assert_eq!(crate::store::space_id(), LOCAL);
        assert_eq!(
            crate::store::workspace_path(),
            crate::store::data_dir().join("workspace.json")
        );
    }

    #[test]
    fn a_space_ai_runs_its_agent_on_the_company_model_with_the_key_out_of_the_command_line() {
        let claude = SpaceAi {
            name: "Maison".into(),
            tool: "claude-code".into(),
            base_url: "https://passerelle.example".into(),
            model: "claude-maison".into(),
            key: "sk-ana".into(),
            ..Default::default()
        };
        let r = claude.run().unwrap();
        assert!(r.args.is_empty());
        assert!(r.env.contains(&(
            "ANTHROPIC_BASE_URL".into(),
            "https://passerelle.example".into()
        )));
        assert!(
            r.env
                .contains(&("ANTHROPIC_AUTH_TOKEN".into(), "sk-ana".into()))
        );
        assert!(
            r.env
                .contains(&("ANTHROPIC_MODEL".into(), "claude-maison".into()))
        );
        let codex = SpaceAi {
            name: "Maison \"v2\"".into(),
            tool: "codex".into(),
            base_url: "https://ia.example/v1".into(),
            model: "qwen".into(),
            key: "sk-ana".into(),
            ..Default::default()
        };
        let r = codex.run().unwrap();
        assert_eq!(
            r.env,
            vec![(CODEX_KEY_VAR.to_string(), "sk-ana".to_string())]
        );
        assert!(
            r.args.iter().all(|a| !a.contains("sk-ana")),
            "the key never on the command line"
        );
        assert_eq!(r.args[1], "model_provider=\"orchestra_espace\"");
        assert_eq!(
            r.args[3],
            r#"model_providers.orchestra_espace={name="Maison \"v2\"",base_url="https://ia.example/v1",env_key="ORCHESTRA_ESPACE_CLE",wire_api="responses"}"#
        );
        assert_eq!(&r.args[4..], ["-m", "qwen"]);
        // Without a key, or for an agent that can't be pointed elsewhere: nothing.
        assert!(
            SpaceAi {
                key: String::new(),
                ..codex.clone()
            }
            .run()
            .is_none()
        );
        assert!(
            SpaceAi {
                tool: "vibe".into(),
                ..codex.clone()
            }
            .run()
            .is_none()
        );
        // The space's AI first; with « no AI », not even that.
        let mut s = Spaces::default();
        s.add("Boîte", SpaceKind::Local);
        s.spaces[1].ai_rule = AiRule::SpaceOnly;
        s.spaces[1].ais = vec![codex];
        assert!(s.spaces[1].may_run("codex"));
        assert!(!s.spaces[1].may_run("claude-code"));
        s.spaces[1].ai_rule = AiRule::NoAi;
        assert!(!s.spaces[1].may_run("codex"));
        assert_eq!(
            without_model(vec![
                "exec".into(),
                "-m".into(),
                "o3".into(),
                "--model=x".into(),
                "--model".into(),
                "y".into(),
                "fin".into()
            ]),
            vec!["exec".to_string(), "fin".to_string()]
        );
    }

    #[test]
    fn the_ais_of_a_space_are_members_with_the_same_id_for_everyone() {
        let ai = SpaceAi {
            name: "Maison".into(),
            tool: "codex".into(),
            base_url: "https://ia.example/v1".into(),
            key: "sk-ana".into(),
            ..Default::default()
        };
        let mut s = Spaces::default();
        s.add("Boîte", SpaceKind::Local);
        // Without a key, this person can't run it: not offered.
        s.spaces[1].ais = vec![
            ai.clone(),
            SpaceAi {
                name: "Sans clé".into(),
                key: String::new(),
                ..ai.clone()
            },
        ];
        let members = s.spaces[1].ai_members();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].id, "ia:Maison");
        assert_eq!(members[0].name, "Maison");
        assert_eq!(members[0].ai, "codex");
        // Someone else's token gives the same member, which carries no secret.
        let bobs = SpaceAi {
            key: "sk-bob".into(),
            ..ai
        };
        assert_eq!(bobs.member(), members[0]);
        assert!(!serde_json::to_string(&members).unwrap().contains("sk-"));
        s.spaces[1].ai_rule = AiRule::NoAi;
        assert!(s.spaces[1].ai_members().is_empty());
    }

    #[test]
    fn mcp_clients_are_told_apart() {
        assert_eq!(ai_key_of_client("claude-code"), Some("claude-code"));
        assert_eq!(ai_key_of_client("codex-mcp-client"), Some("codex"));
        assert_eq!(ai_key_of_client("codex"), Some("codex"));
        assert_eq!(ai_key_of_client("vibe"), Some("vibe"));
        assert_eq!(
            ai_key_of_client("gemini-cli-mcp-client"),
            Some("antigravity")
        );
        assert_eq!(ai_key_of_client("claude-ai"), Some("claude-desktop"));
        assert_eq!(ai_key_of_client("cursor-vscode"), Some("cursor"));
        assert_eq!(ai_key_of_client("un-inconnu"), None);
    }

    #[test]
    fn an_unknown_ai_is_allowed_only_where_every_ai_is() {
        let mut s = Spaces::default();
        s.add("Boîte", SpaceKind::Local);
        assert!(s.spaces[1].allows_own_ai("inconnue"));
        s.spaces[1].allowed_ai = vec!["codex".into()];
        assert!(!s.spaces[1].allows_own_ai("inconnue"));
    }

    #[test]
    fn mon_espace_cannot_be_removed() {
        let mut s = Spaces::default();
        let id = s.add("Perso", SpaceKind::Local);
        s.active = id.clone();
        assert!(!s.remove(LOCAL));
        assert!(s.remove(&id));
        assert_eq!(s.active, LOCAL);
        assert_eq!(s.spaces.len(), 1);
    }
}
