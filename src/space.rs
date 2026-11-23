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

// The AI rule and server spaces aren't wired yet (docs/collaboration-serveur.md, step 1):
// remove once they are.
#![allow(dead_code)]

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
    /// A copy of a team server's space (step 2 of the plan).
    Server { url: String },
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

fn list_path() -> PathBuf {
    crate::store::data_dir().join("espaces.json")
}

/// The list of spaces; just "Mon espace" when there is none yet or it can't be read.
pub fn load() -> Spaces {
    std::fs::read_to_string(list_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Spaces>(&s).ok())
        .unwrap_or_default()
        .tidy()
}

pub fn save(spaces: &Spaces) -> std::io::Result<()> {
    std::fs::create_dir_all(crate::store::data_dir())?;
    let text = serde_json::to_string_pretty(&spaces.clone().tidy()).unwrap_or_default();
    crate::store::write_atomic(&list_path(), text.as_bytes())
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
                url: "https://o.example".into(),
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
