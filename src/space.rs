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
}

impl Space {
    fn local() -> Self {
        Space {
            id: LOCAL.into(),
            name: crate::tr!("Mon espace", "My workspace").into(),
            kind: SpaceKind::Local,
            ai_rule: AiRule::Free,
            allowed_ai: vec![],
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
    if s.allows_own_ai(key) {
        return None;
    }
    let name = s.name.clone();
    Some(match s.ai_rule {
        AiRule::NoAi => crate::trf!(
            "l'espace « {name} » n'autorise aucune IA",
            "the space « {name} » allows no AI"
        ),
        AiRule::SpaceOnly => crate::trf!(
            "l'espace « {name} » n'autorise que ses propres IA, pas {label}",
            "the space « {name} » only allows its own AIs, not {label}"
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
