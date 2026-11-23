//! Team server: a space shared by its members, on a server someone hosts (`orchestra equipe`).
//! See docs/collaboration-serveur.md, step 2.
//!
//! * **Who may connect**: members are known by the public half of their key (identity.rs).
//!   The server sends a fresh challenge; the app answers with its key and a signature of it.
//!   A key the server doesn't know comes in only with an unused invitation (one use each),
//!   which makes it a member. No password, no shared code. A member taken off the space is
//!   cut off within seconds, and refused after.
//! * **Roles**: administrator (the first person in), member, reader. A reader's changes are
//!   not taken.
//! * **Wire**: WebSocket, one JSON message per frame. After the greeting, the messages are
//!   those of live collaboration (`live::LiveMsg`), so the window merges, sends and shows
//!   presence exactly as in live mode, plus `Seq` marks (below).
//! * **Storage**: SQLite (`espace.sqlite`), one row per page. A page received is merged into
//!   the stored one block by block (`collab::merge_page`), then passed on to the others.
//! * **Picking up after a break**: every change stored gets the next number (`seq`). The app
//!   keeps the last number it has and gives it when it connects: it receives only what came
//!   after. Each message it sends is acknowledged with a number, so on reconnecting it sends
//!   only what the server hasn't acknowledged.
//! * **Encryption**: the server speaks plain WebSocket; on the Internet, put it behind an
//!   HTTPS proxy (Caddy, Traefik…) and join with a `wss://` link (TLS, certificates checked
//!   against the usual authorities, plus a company's own: `ORCHESTRA_CA=<file.pem>`). `ws://`
//!   is for a private network (Tailscale, WireGuard…).
//! * **Load**: one thread per connection, with a small stack; a few thousand connections at
//!   once are fine. Changes are stored one at a time (one SQLite connection).

use crate::live::{self, LiveMsg, Status};
use crate::model::{ChatMsg, Page};
use crate::space::AiRule;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

pub const DEFAULT_PORT: u16 = 7879;
/// Connections the server keeps at once.
const MAX_CONNECTIONS: usize = 4096;
/// Stack of each connection's thread: what it does is shallow.
const THREAD_STACK: usize = 256 * 1024;
/// Largest message accepted (a whole workspace sent at once).
const MAX_MESSAGE: usize = 32 * 1024 * 1024;
/// How long a reader waits before looking at what is to be sent.
const TICK: Duration = Duration::from_millis(100);
/// How often the server lightens old documents, and how long a block must be left untouched.
const COMPACT_EVERY: Duration = Duration::from_secs(3600);
pub const COMPACT_IDLE_MS: u64 = 7 * 24 * 3600 * 1000;
/// How often a connected member's access is checked again (taken off the space meanwhile?).
const RECHECK: Duration = Duration::from_secs(5);

/// A member's role in the space.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    #[default]
    Member,
    /// Sees everything, changes nothing.
    Reader,
}

impl Role {
    pub fn key(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
            Role::Reader => "reader",
        }
    }
    pub fn from_key(k: &str) -> Option<Role> {
        match k.trim() {
            "admin" | "administrateur" => Some(Role::Admin),
            "member" | "membre" => Some(Role::Member),
            "reader" | "lecteur" => Some(Role::Reader),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Role::Admin => crate::tr!("administrateur", "administrator"),
            Role::Member => crate::tr!("membre", "member"),
            Role::Reader => crate::tr!("lecteur", "reader"),
        }
    }
}

/// The greeting, and the server's marks after it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Greeting {
    /// Server → app: sign this.
    Challenge { space: String, nonce: String },
    /// App → server: who I am, the signature, an invitation for a first visit, and the last
    /// change number I have.
    Auth {
        key: String,
        name: String,
        sig: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        invite: String,
        #[serde(default)]
        since: u64,
    },
    /// Server → app: welcome, with what the space is and the member's role.
    Welcome {
        space: SpaceInfo,
        name: String,
        #[serde(default)]
        role: Role,
        /// The AIs the space provides to this member, with their key.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ais: Vec<crate::space::SpaceAi>,
    },
    /// Server → app: not (or no longer) welcome.
    Refused { reason: String },
    /// Server → app: the answer to an administrator's request.
    AdminReply { view: AdminView },
    /// Server → app: everything up to change `n` has been sent to you. `ack`: and the
    /// message you sent last has been taken.
    Seq {
        n: u64,
        #[serde(default)]
        ack: bool,
    },
}

/// What an administrator asks from the app (`LiveMsg::Admin`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AdminOp {
    List,
    Invite {
        role: Role,
    },
    Revoke {
        key: String,
    },
    Restore {
        key: String,
    },
    SetRole {
        key: String,
        role: Role,
    },
    AddAi {
        ai: crate::space::SpaceAi,
    },
    /// The company's key of an AI, for one member (their public key) or everyone (`*`).
    SetKey {
        ai: String,
        member: String,
        key: String,
    },
    RemoveAi {
        name: String,
    },
    Quota {
        ai: String,
        per_day: u64,
    },
    Rename {
        name: String,
    },
    Rule {
        rule: AiRule,
        allowed: Vec<String>,
    },
}

/// What an administrator sees of the space: never a key.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct AdminView {
    pub name: String,
    pub rule: AiRule,
    pub allowed: Vec<String>,
    pub members: Vec<MemberView>,
    pub ais: Vec<AiView>,
    /// Calls of today through the relay: (member's name, AI, calls).
    pub today: Vec<(String, String, u64)>,
    /// A new invitation's token (`Invite`): the app makes the link.
    #[serde(default)]
    pub invite: String,
    /// Why the request was refused, if it was.
    #[serde(default)]
    pub error: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemberView {
    pub key: String,
    pub name: String,
    pub role: Role,
    pub revoked: bool,
    /// The administrator looking.
    pub me: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AiView {
    pub name: String,
    pub tool: String,
    pub base_url: String,
    pub model: String,
    pub quota: u64,
    /// How many keys it has (members' own and everyone's).
    pub keys: usize,
}

/// What a server says about its space.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct SpaceInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub ai_rule: AiRule,
    #[serde(default)]
    pub allowed_ai: Vec<String>,
}

/// What is signed to connect: tied to the space, so a signature made for one server is worth
/// nothing on another.
pub fn to_sign(space: &str, nonce: &str) -> Vec<u8> {
    format!("orchestra-equipe:{space}:{nonce}").into_bytes()
}

/// An invitation link: the server's address and a one-time token.
/// `ws://192.168.1.20:7879/?invitation=…` → ("ws://192.168.1.20:7879", "…").
pub fn parse_invitation(link: &str) -> Option<(String, String)> {
    let link = link.trim();
    let (base, query) = link.split_once('?')?;
    let token = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("invitation="))?
        .trim();
    let base = base.trim_end_matches('/');
    let ok_scheme = base.starts_with("ws://") || base.starts_with("wss://");
    if !ok_scheme || token.len() < 16 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some((base.to_string(), token.to_string()))
}

// ------------------------------------------------------------------ the server's storage

/// A member, as `orchestra equipe membres` lists them.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub key: String,
    pub name: String,
    pub role: Role,
    pub joined: u64,
    pub revoked: bool,
}

/// The space on the server's disk.
pub struct Db(Connection);

impl Db {
    pub fn open(dir: &std::path::Path) -> rusqlite::Result<Db> {
        let _ = std::fs::create_dir_all(dir);
        let c = Connection::open(dir.join("espace.sqlite"))?;
        c.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS members (key TEXT PRIMARY KEY, name TEXT NOT NULL,
                 role TEXT NOT NULL, joined INTEGER NOT NULL, revoked INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS invites (token TEXT PRIMARY KEY, role TEXT NOT NULL,
                 created INTEGER NOT NULL, used_by TEXT);
             CREATE TABLE IF NOT EXISTS pages (id TEXT PRIMARY KEY, json TEXT NOT NULL,
                 updated INTEGER NOT NULL, seq INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS deleted (id TEXT PRIMARY KEY, at INTEGER NOT NULL,
                 seq INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS chat (id TEXT PRIMARY KEY, json TEXT NOT NULL,
                 seq INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS space_ais (name TEXT PRIMARY KEY, tool TEXT NOT NULL,
                 base_url TEXT NOT NULL, model TEXT NOT NULL DEFAULT '',
                 wire_api TEXT NOT NULL DEFAULT '');
             CREATE TABLE IF NOT EXISTS ai_keys (ai TEXT NOT NULL, member TEXT NOT NULL,
                 key TEXT NOT NULL, PRIMARY KEY (ai, member));
             CREATE TABLE IF NOT EXISTS proxy_tokens (token TEXT PRIMARY KEY,
                 member TEXT NOT NULL UNIQUE);
             CREATE TABLE IF NOT EXISTS usage (member TEXT NOT NULL, ai TEXT NOT NULL,
                 day TEXT NOT NULL, n INTEGER NOT NULL, PRIMARY KEY (member, ai, day));",
        )?;
        let _ = c.execute(
            "ALTER TABLE space_ais ADD COLUMN quota INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Spaces made before change numbers: add the column (0: sent to everyone once).
        for t in ["pages", "deleted", "chat"] {
            let _ = c.execute(
                &format!("ALTER TABLE {t} ADD COLUMN seq INTEGER NOT NULL DEFAULT 0"),
                [],
            );
        }
        let db = Db(c);
        if db.meta("id").is_none() {
            db.set_meta("id", &crate::model::new_id())?;
            db.set_meta("name", crate::tr!("Équipe", "Team"))?;
        }
        Ok(db)
    }

    fn meta(&self, k: &str) -> Option<String> {
        self.0
            .query_row("SELECT v FROM meta WHERE k = ?1", [k], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_meta(&self, k: &str, v: &str) -> rusqlite::Result<()> {
        self.0.execute(
            "INSERT INTO meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = ?2",
            params![k, v],
        )?;
        Ok(())
    }

    /// The number of the last change stored.
    pub fn seq(&self) -> u64 {
        self.meta("seq").and_then(|s| s.parse().ok()).unwrap_or(0)
    }

    fn next_seq(&self) -> u64 {
        let n = self.seq() + 1;
        let _ = self.set_meta("seq", &n.to_string());
        n
    }

    pub fn info(&self) -> SpaceInfo {
        SpaceInfo {
            id: self.meta("id").unwrap_or_default(),
            name: self.meta("name").unwrap_or_default(),
            ai_rule: self
                .meta("ai_rule")
                .and_then(|r| serde_json::from_str(&format!("\"{r}\"")).ok())
                .unwrap_or_default(),
            allowed_ai: self
                .meta("allowed_ai")
                .and_then(|a| serde_json::from_str(&a).ok())
                .unwrap_or_default(),
        }
    }

    pub fn set_ai_rule(&self, rule: AiRule, allowed: &[String]) -> rusqlite::Result<()> {
        let r = serde_json::to_string(&rule).unwrap_or_default();
        self.set_meta("ai_rule", r.trim_matches('"'))?;
        self.set_meta(
            "allowed_ai",
            &serde_json::to_string(allowed).unwrap_or_default(),
        )
    }

    pub fn members(&self) -> usize {
        self.0
            .query_row("SELECT COUNT(*) FROM members WHERE revoked = 0", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize
    }

    /// Everyone who ever came in, revoked ones included.
    pub fn member_list(&self) -> Vec<Member> {
        let Ok(mut st) = self
            .0
            .prepare("SELECT key, name, role, joined, revoked FROM members ORDER BY joined")
        else {
            return vec![];
        };
        st.query_map([], |r| {
            Ok(Member {
                key: r.get(0)?,
                name: r.get(1)?,
                role: Role::from_key(&r.get::<_, String>(2)?).unwrap_or_default(),
                joined: r.get::<_, i64>(3)? as u64,
                revoked: r.get::<_, i64>(4)? != 0,
            })
        })
        .map(|it| it.flatten().collect())
        .unwrap_or_default()
    }

    /// The member `who` names: their whole key, the start of it (8 characters at least), or
    /// their name when only one member has it.
    pub fn find_member(&self, who: &str) -> Result<Member, String> {
        let who = who.trim();
        let all = self.member_list();
        let by_key: Vec<&Member> = all
            .iter()
            .filter(|m| who.len() >= 8 && m.key.starts_with(who))
            .collect();
        let found: Vec<&Member> = if by_key.is_empty() {
            all.iter().filter(|m| m.name == who).collect()
        } else {
            by_key
        };
        match found.as_slice() {
            [m] => Ok((*m).clone()),
            [] => Err(crate::trf!("aucun membre « {who} »", "no member « {who} »")),
            _ => Err(crate::trf!(
                "plusieurs membres « {who} » : donnez le début de la clé (orchestra equipe membres)",
                "several members « {who} »: give the start of the key (orchestra equipe members)"
            )),
        }
    }

    /// Take `key` off the space (cut off within seconds if connected), or let it back in.
    pub fn set_revoked(&self, key: &str, revoked: bool) -> rusqlite::Result<usize> {
        self.0.execute(
            "UPDATE members SET revoked = ?2 WHERE key = ?1",
            params![key, revoked as i64],
        )
    }

    pub fn set_role(&self, key: &str, role: Role) -> rusqlite::Result<usize> {
        self.0.execute(
            "UPDATE members SET role = ?2 WHERE key = ?1",
            params![key, role.key()],
        )
    }

    /// Add (or change) an AI the space provides. `tool`: `claude-code` or `codex`.
    pub fn set_space_ai(&self, a: &crate::space::SpaceAi) -> rusqlite::Result<()> {
        self.0.execute(
            "INSERT INTO space_ais (name, tool, base_url, model, wire_api) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(name) DO UPDATE SET tool = ?2, base_url = ?3, model = ?4, wire_api = ?5",
            params![a.name, a.tool, a.base_url, a.model, a.wire_api],
        )?;
        Ok(())
    }

    pub fn remove_space_ai(&self, name: &str) -> rusqlite::Result<usize> {
        self.0
            .execute("DELETE FROM ai_keys WHERE ai = ?1", [name])?;
        self.0
            .execute("DELETE FROM space_ais WHERE name = ?1", [name])
    }

    /// The AIs the space provides (without keys), and for each how many keys it has.
    pub fn space_ais(&self) -> Vec<(crate::space::SpaceAi, usize)> {
        let Ok(mut st) = self.0.prepare(
            "SELECT name, tool, base_url, model, wire_api,
                    (SELECT COUNT(*) FROM ai_keys k WHERE k.ai = a.name)
             FROM space_ais a ORDER BY name",
        ) else {
            return vec![];
        };
        st.query_map([], |r| {
            Ok((
                crate::space::SpaceAi {
                    name: r.get(0)?,
                    tool: r.get(1)?,
                    base_url: r.get(2)?,
                    model: r.get(3)?,
                    wire_api: r.get(4)?,
                    key: String::new(),
                },
                r.get::<_, i64>(5)? as usize,
            ))
        })
        .map(|it| it.flatten().collect())
        .unwrap_or_default()
    }

    /// The key of AI `ai` for member `member` (their public key), or for everyone (`*`).
    pub fn set_ai_key(&self, ai: &str, member: &str, key: &str) -> rusqlite::Result<usize> {
        self.0.execute(
            "INSERT INTO ai_keys (ai, member, key) VALUES (?1, ?2, ?3)
             ON CONFLICT(ai, member) DO UPDATE SET key = ?3",
            params![ai, member, key],
        )
    }

    /// The company's key of AI `ai` for `member`: their own, else the one for everyone.
    fn upstream_key(&self, ai: &str, member: &str) -> Option<String> {
        self.0
            .query_row(
                "SELECT key FROM ai_keys WHERE ai = ?1 AND member IN (?2, '*')
                 ORDER BY member = '*' LIMIT 1",
                params![ai, member],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Member `member`'s own token for the AIs of the space (made the first time).
    pub fn proxy_token(&self, member: &str) -> String {
        let known: Option<String> = self
            .0
            .query_row(
                "SELECT token FROM proxy_tokens WHERE member = ?1",
                [member],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        if let Some(t) = known {
            return t;
        }
        let t = format!("orch-{}", crate::identity::random_hex(24));
        let _ = self.0.execute(
            "INSERT INTO proxy_tokens (token, member) VALUES (?1, ?2)",
            params![t, member],
        );
        t
    }

    /// The space's AIs as member `member` gets them: the address of this server's relay
    /// (`/ia/<name>`, completed by the app with the server's own) and their token, never the
    /// company's key. Only those with a key for them (theirs or everyone's). A reader gets
    /// none (they change nothing).
    pub fn ais_for(&self, member: &str, role: Role) -> Vec<crate::space::SpaceAi> {
        if role == Role::Reader {
            return vec![];
        }
        let token = self.proxy_token(member);
        self.space_ais()
            .into_iter()
            .filter(|(a, _)| self.upstream_key(&a.name, member).is_some())
            .map(|(mut a, _)| {
                a.base_url = format!("/ia/{}", crate::proxy::slug(&a.name));
                a.key = token.clone();
                a
            })
            .collect()
    }

    /// Who may call AI `slug` through the relay with `token`: the AI and the company's key,
    /// the call counted; or why not (unknown token, member taken off, reader, no key, daily
    /// quota reached).
    pub fn proxy_grant(
        &self,
        token: &str,
        slug: &str,
    ) -> Result<(crate::space::SpaceAi, String), String> {
        let member: String = self
            .0
            .query_row(
                "SELECT member FROM proxy_tokens WHERE token = ?1",
                [token],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
            .ok_or_else(|| crate::tr!("jeton inconnu", "unknown token").to_string())?;
        let (_, role) = self.member(&member).ok_or_else(|| {
            crate::tr!(
                "votre accès à cet espace a été retiré",
                "your access to this space was removed"
            )
            .to_string()
        })?;
        if role == Role::Reader {
            return Err(crate::tr!(
                "les lecteurs n'utilisent pas les IA de l'espace",
                "readers don't use the space's AIs"
            )
            .into());
        }
        let (ai, _) = self
            .space_ais()
            .into_iter()
            .find(|(a, _)| crate::proxy::slug(&a.name) == slug)
            .ok_or_else(|| crate::tr!("IA inconnue", "unknown AI").to_string())?;
        let key = self.upstream_key(&ai.name, &member).ok_or_else(|| {
            crate::tr!(
                "pas de clé pour vous sur cette IA",
                "no key for you on this AI"
            )
            .to_string()
        })?;
        let day = today();
        let used: i64 = self
            .0
            .query_row(
                "SELECT n FROM usage WHERE member = ?1 AND ai = ?2 AND day = ?3",
                params![member, ai.name, day],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
            .unwrap_or(0);
        let quota: i64 = self
            .0
            .query_row(
                "SELECT quota FROM space_ais WHERE name = ?1",
                [&ai.name],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if quota > 0 && used >= quota {
            return Err(crate::trf!(
                "quota du jour atteint ({quota} appels)",
                "daily quota reached ({quota} calls)"
            ));
        }
        let _ = self.0.execute(
            "INSERT INTO usage (member, ai, day, n) VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(member, ai, day) DO UPDATE SET n = n + 1",
            params![member, ai.name, day],
        );
        Ok((ai, key))
    }

    /// Calls per day of an AI of the space to stop at (0: no limit), per member.
    pub fn set_quota(&self, ai: &str, per_day: u64) -> rusqlite::Result<usize> {
        self.0.execute(
            "UPDATE space_ais SET quota = ?2 WHERE name = ?1",
            params![ai, per_day as i64],
        )
    }

    /// Calls made through the relay: (day, member's name, AI, calls), latest first.
    pub fn usage(&self, days: u32) -> Vec<(String, String, String, u64)> {
        let since = day_of(crate::model::now_ms().saturating_sub(days as u64 * 86_400_000));
        self.0
            .prepare(
                "SELECT u.day, COALESCE(m.name, substr(u.member, 1, 12)), u.ai, u.n
                 FROM usage u LEFT JOIN members m ON m.key = u.member
                 WHERE u.day >= ?1 ORDER BY u.day DESC, u.n DESC",
            )
            .and_then(|mut st| {
                st.query_map([since], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? as u64))
                })
                .map(|it| it.flatten().collect())
            })
            .unwrap_or_default()
    }

    /// A new one-time invitation for `role` (the very first person to come in becomes the
    /// administrator, whichever invitation they use: see `redeem`).
    pub fn invite(&self, role: Role) -> rusqlite::Result<String> {
        let token = crate::identity::random_hex(16);
        self.0.execute(
            "INSERT INTO invites (token, role, created) VALUES (?1, ?2, ?3)",
            params![token, role.key(), crate::model::now_ms() as i64],
        )?;
        Ok(token)
    }

    /// The name and role a known, not revoked key goes by.
    fn member(&self, key: &str) -> Option<(String, Role)> {
        self.0
            .query_row(
                "SELECT name, role FROM members WHERE key = ?1 AND revoked = 0",
                [key],
                |r| Ok((r.get(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .ok()
            .flatten()
            .map(|(n, r)| (n, Role::from_key(&r).unwrap_or_default()))
    }

    fn revoked(&self, key: &str) -> bool {
        self.0
            .query_row("SELECT revoked FROM members WHERE key = ?1", [key], |r| {
                r.get::<_, i64>(0)
            })
            .optional()
            .ok()
            .flatten()
            .is_some_and(|r| r != 0)
    }

    /// Spend `token` to make `key` a member. None when it doesn't exist or was used.
    fn redeem(&self, token: &str, key: &str, name: &str) -> Option<Role> {
        let role: String = self
            .0
            .query_row(
                "SELECT role FROM invites WHERE token = ?1 AND used_by IS NULL",
                [token],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()?;
        let role = if self.members() == 0 {
            Role::Admin
        } else {
            Role::from_key(&role).unwrap_or_default()
        };
        let used = self
            .0
            .execute(
                "UPDATE invites SET used_by = ?2 WHERE token = ?1 AND used_by IS NULL",
                params![token, key],
            )
            .unwrap_or(0);
        (used == 1
            && self
                .0
                .execute(
                    "INSERT INTO members (key, name, role, joined) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(key) DO NOTHING",
                    params![key, name, role.key(), crate::model::now_ms() as i64],
                )
                .is_ok())
        .then_some(role)
    }

    fn page(&self, id: &str) -> Option<Page> {
        let json: Option<String> = self
            .0
            .query_row("SELECT json FROM pages WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .ok()
            .flatten();
        json.and_then(|j| serde_json::from_str(&j).ok())
    }

    /// What changed after change `since` (everything with 0), for whoever connects.
    pub fn since(&self, since: u64) -> (Vec<Page>, BTreeMap<String, u64>, Vec<ChatMsg>) {
        let rows = |sql: &str| -> Vec<String> {
            let Ok(mut st) = self.0.prepare(sql) else {
                return vec![];
            };
            st.query_map([since as i64], |r| r.get::<_, String>(0))
                .map(|it| it.flatten().collect())
                .unwrap_or_default()
        };
        let pages = rows("SELECT json FROM pages WHERE seq > ?1 OR ?1 = 0")
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let chat = rows("SELECT json FROM chat WHERE seq > ?1 OR ?1 = 0")
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let mut deleted = BTreeMap::new();
        if let Ok(mut st) = self
            .0
            .prepare("SELECT id, at FROM deleted WHERE seq > ?1 OR ?1 = 0")
            && let Ok(it) = st.query_map([since as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
        {
            for (id, at) in it.flatten() {
                deleted.insert(id, at as u64);
            }
        }
        (pages, deleted, chat)
    }

    /// Everything.
    #[cfg(test)]
    pub fn everything(&self) -> (Vec<Page>, BTreeMap<String, u64>, Vec<ChatMsg>) {
        self.since(0)
    }

    /// Merge pages and deletions a member sent. Returns the pages as they now stand, and the
    /// deletions that are news, to pass on; each change stored gets the next number.
    pub fn absorb(
        &self,
        pages: Vec<Page>,
        deleted: BTreeMap<String, u64>,
    ) -> (Vec<Page>, BTreeMap<String, u64>) {
        let mut out_deleted = BTreeMap::new();
        for (id, at) in deleted {
            let known: Option<i64> = self
                .0
                .query_row("SELECT at FROM deleted WHERE id = ?1", [&id], |r| r.get(0))
                .optional()
                .ok()
                .flatten();
            if known.is_some_and(|k| k as u64 >= at) {
                continue;
            }
            let seq = self.next_seq();
            let _ = self.0.execute(
                "INSERT INTO deleted (id, at, seq) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET at = MAX(at, ?2), seq = ?3",
                params![id, at as i64, seq as i64],
            );
            // A page deleted for good, unless edited again after.
            let _ = self.0.execute(
                "DELETE FROM pages WHERE id = ?1 AND updated <= ?2",
                params![id, at as i64],
            );
            out_deleted.insert(id, at);
        }
        let mut out = vec![];
        for p in pages {
            let gone: Option<i64> = self
                .0
                .query_row("SELECT at FROM deleted WHERE id = ?1", [&p.id], |r| {
                    r.get(0)
                })
                .optional()
                .ok()
                .flatten();
            if gone.is_some_and(|at| p.updated <= at as u64) {
                continue;
            }
            // Every block kept here has its document (crdt.rs): a page from an app that
            // didn't give them (an older version) gets them on arrival.
            let mut p = p;
            for b in p.blocks.iter_mut().filter(|b| b.crdt.is_none()) {
                crate::crdt::reconcile(b);
            }
            if p.order_crdt.is_none() {
                crate::crdt::reconcile_order(&mut p);
            }
            let merged = match self.page(&p.id) {
                Some(mut ours) => {
                    if !crate::collab::merge_page(&mut ours, &p) {
                        continue; // nothing new
                    }
                    ours
                }
                None => p,
            };
            let seq = self.next_seq();
            let _ = self.0.execute(
                "INSERT INTO pages (id, json, updated, seq) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET json = ?2, updated = ?3, seq = ?4",
                params![
                    merged.id,
                    serde_json::to_string(&merged).unwrap_or_default(),
                    merged.updated as i64,
                    seq as i64
                ],
            );
            out.push(merged);
        }
        (out, out_deleted)
    }

    /// Carry out `op` for the administrator `me` (their public key) and say how the space now
    /// stands. An administrator can't take themselves off, nor change their own role (the
    /// space would risk having none).
    pub fn admin(&self, op: AdminOp, me: &str) -> AdminView {
        let own = |k: &str| {
            (k == me).then(|| {
                crate::tr!(
                    "pas sur vous-même : demandez à un autre administrateur",
                    "not on yourself: ask another administrator"
                )
                .to_string()
            })
        };
        let mut invite = String::new();
        let done: Result<(), String> = match op {
            AdminOp::List => Ok(()),
            AdminOp::Invite { role } => self
                .invite(role)
                .map(|t| invite = t)
                .map_err(|e| e.to_string()),
            AdminOp::Revoke { key } => match own(&key) {
                Some(e) => Err(e),
                None => self
                    .set_revoked(&key, true)
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
            },
            AdminOp::Restore { key } => self
                .set_revoked(&key, false)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            AdminOp::SetRole { key, role } => match own(&key) {
                Some(e) => Err(e),
                None => self
                    .set_role(&key, role)
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
            },
            AdminOp::AddAi { ai } => {
                if ai.name.trim().is_empty()
                    || !matches!(ai.tool.as_str(), "claude-code" | "codex")
                    || crate::proxy::upstream(&ai.base_url).is_none()
                {
                    Err(crate::tr!(
                        "IA incomplète : un nom, Claude Code ou Codex, une adresse http(s)://",
                        "incomplete AI: a name, Claude Code or Codex, an http(s):// address"
                    )
                    .into())
                } else {
                    let ai = crate::space::SpaceAi {
                        key: String::new(),
                        ..ai
                    };
                    self.set_space_ai(&ai).map_err(|e| e.to_string())
                }
            }
            AdminOp::SetKey { ai, member, key } => {
                if key.trim().is_empty() {
                    Err(crate::tr!("clé vide", "empty key").into())
                } else {
                    self.set_ai_key(&ai, &member, key.trim())
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                }
            }
            AdminOp::RemoveAi { name } => self
                .remove_space_ai(&name)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            AdminOp::Quota { ai, per_day } => self
                .set_quota(&ai, per_day)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            AdminOp::Rename { name } => {
                if name.trim().is_empty() {
                    Err(crate::tr!("nom vide", "empty name").into())
                } else {
                    self.set_meta("name", name.trim())
                        .map_err(|e| e.to_string())
                }
            }
            AdminOp::Rule { rule, allowed } => {
                self.set_ai_rule(rule, &allowed).map_err(|e| e.to_string())
            }
        };
        let info = self.info();
        let today = today();
        AdminView {
            name: info.name,
            rule: info.ai_rule,
            allowed: info.allowed_ai,
            members: self
                .member_list()
                .into_iter()
                .map(|m| MemberView {
                    me: m.key == me,
                    key: m.key,
                    name: m.name,
                    role: m.role,
                    revoked: m.revoked,
                })
                .collect(),
            ais: self
                .space_ais()
                .into_iter()
                .map(|(a, keys)| AiView {
                    quota: self
                        .0
                        .query_row(
                            "SELECT quota FROM space_ais WHERE name = ?1",
                            [&a.name],
                            |r| r.get::<_, i64>(0),
                        )
                        .unwrap_or(0) as u64,
                    name: a.name,
                    tool: a.tool,
                    base_url: a.base_url,
                    model: a.model,
                    keys,
                })
                .collect(),
            today: self
                .usage(1)
                .into_iter()
                .filter(|u| u.0 == today)
                .map(|(_, who, ai, n)| (who, ai, n))
                .collect(),
            invite,
            error: done.err().unwrap_or_default(),
        }
    }

    /// Lighten the documents of blocks nobody touched for `idle_ms` (crdt::compact). Each page
    /// lightened gets a new change number: the members take the lighter copy the next time.
    /// Returns how many pages were lightened.
    pub fn compact_idle(&self, idle_ms: u64) -> usize {
        let cutoff = crate::model::now_ms().saturating_sub(idle_ms);
        let rows: Vec<String> = self
            .0
            .prepare("SELECT json FROM pages WHERE updated < ?1")
            .and_then(|mut st| {
                st.query_map([cutoff as i64], |r| r.get::<_, String>(0))
                    .map(|it| it.flatten().collect())
            })
            .unwrap_or_default();
        let mut n = 0;
        for j in rows {
            let Ok(mut p) = serde_json::from_str::<Page>(&j) else {
                continue;
            };
            let mut lighter = false;
            for b in p.blocks.iter_mut().filter(|b| b.updated < cutoff) {
                lighter |= crate::crdt::compact(b);
            }
            if lighter {
                let seq = self.next_seq();
                let _ = self.0.execute(
                    "UPDATE pages SET json = ?2, seq = ?3 WHERE id = ?1",
                    params![
                        p.id,
                        serde_json::to_string(&p).unwrap_or_default(),
                        seq as i64
                    ],
                );
                n += 1;
            }
        }
        n
    }

    /// New discussion messages (never edited): those not stored yet.
    pub fn absorb_chat(&self, msgs: Vec<ChatMsg>) -> Vec<ChatMsg> {
        msgs.into_iter()
            .filter(|m| {
                if m.id.is_empty() {
                    return false;
                }
                let known = self
                    .0
                    .query_row("SELECT 1 FROM chat WHERE id = ?1", [&m.id], |_| Ok(()))
                    .optional()
                    .ok()
                    .flatten()
                    .is_some();
                if known {
                    return false;
                }
                let seq = self.next_seq();
                self.0
                    .execute(
                        "INSERT INTO chat (id, json, seq) VALUES (?1, ?2, ?3)",
                        params![
                            m.id,
                            serde_json::to_string(m).unwrap_or_default(),
                            seq as i64
                        ],
                    )
                    .unwrap_or(0)
                    == 1
            })
            .collect()
    }
}

/// A day as « 2026-11-23 » (UTC), for counting calls.
fn day_of(ms: u64) -> String {
    let days = (ms / 86_400_000) as i64;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

fn today() -> String {
    day_of(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    )
}

/// Check a member's greeting; returns their name and role, or why not.
fn admit(db: &Db, space: &str, nonce: &str, auth: &Greeting) -> Result<(String, Role), String> {
    let Greeting::Auth {
        key,
        name,
        sig,
        invite,
        ..
    } = auth
    else {
        return Err(crate::tr!("salutation attendue", "greeting expected").into());
    };
    if !crate::identity::verify(key, &to_sign(space, nonce), sig) {
        return Err(crate::tr!("signature invalide", "invalid signature").into());
    }
    if db.revoked(key) {
        return Err(crate::tr!(
            "votre accès à cet espace a été retiré",
            "your access to this space was removed"
        )
        .into());
    }
    if let Some(m) = db.member(key) {
        return Ok(m);
    }
    let name = name.trim().chars().take(60).collect::<String>();
    let name = if name.is_empty() {
        crate::tr!("Membre", "Member").to_string()
    } else {
        name
    };
    if !invite.is_empty()
        && let Some(role) = db.redeem(invite, key, &name)
    {
        return Ok((name, role));
    }
    Err(crate::tr!(
        "invitation inconnue ou déjà utilisée",
        "unknown or already used invitation"
    )
    .into())
}

// ------------------------------------------------------------------ the server

struct Conn {
    id: u64,
    out: Sender<String>,
}

type Conns = Arc<Mutex<Vec<Conn>>>;

/// Queue `texts` for every connection but `from`, in order.
fn pass_on(conns: &Conns, from: u64, texts: &[String]) {
    if let Ok(list) = conns.lock() {
        for c in list.iter().filter(|c| c.id != from) {
            for t in texts {
                let _ = c.out.send(t.clone());
            }
        }
    }
}

fn json<T: Serialize>(msg: &T) -> String {
    serde_json::to_string(msg).unwrap_or_default()
}

/// Is this a read that only timed out (nothing came during `TICK`)?
fn idle(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

fn ws_config() -> tungstenite::protocol::WebSocketConfig {
    tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
}

/// Serve the space kept in `dir` on `port`, until the process stops.
pub fn serve(dir: &std::path::Path, port: u16) -> Result<(), String> {
    let db = Arc::new(Mutex::new(Db::open(dir).map_err(|e| e.to_string())?));
    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| e.to_string())?;
    let conns: Conns = Arc::new(Mutex::new(vec![]));
    // Every hour, the documents of blocks untouched for a week are lightened.
    {
        let db = db.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(COMPACT_EVERY);
                if let Ok(d) = db.lock() {
                    let n = d.compact_idle(COMPACT_IDLE_MS);
                    if n > 0 {
                        eprintln!("  {n} page(s) allégée(s)");
                    }
                }
            }
        });
    }
    let open = Arc::new(AtomicUsize::new(0));
    let next = AtomicU64::new(1);
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        // Counted from the first byte (a connection still greeting holds a thread too).
        if open.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
            continue;
        }
        open.fetch_add(1, Ordering::SeqCst);
        let id = next.fetch_add(1, Ordering::SeqCst);
        let (db, conns, open2) = (db.clone(), conns.clone(), open.clone());
        let spawned = std::thread::Builder::new()
            .stack_size(THREAD_STACK)
            .spawn(move || {
                // WebSocket (the space) or plain HTTP (the relay of the space's AIs).
                match read_head(stream) {
                    Ok((stream, head, raw, rest)) if head.is_websocket() => {
                        let mut pre = raw;
                        pre.extend_from_slice(&rest);
                        let replay = Replay {
                            pre,
                            pos: 0,
                            inner: stream,
                        };
                        if let Err(e) = serve_one(id, replay, &db, &conns) {
                            eprintln!("orchestra: connexion {id} : {e}");
                        }
                    }
                    Ok((stream, head, _, rest)) => {
                        crate::proxy::serve_http(stream, head, rest, &db)
                    }
                    Err(_) => {}
                }
                if let Ok(mut l) = conns.lock() {
                    l.retain(|c| c.id != id);
                }
                open2.fetch_sub(1, Ordering::SeqCst);
            });
        if spawned.is_err() {
            open.fetch_sub(1, Ordering::SeqCst);
        }
    }
    Ok(())
}

/// A connection whose first bytes were already read (to tell WebSocket from HTTP): they are
/// read again first.
struct Replay {
    pre: Vec<u8>,
    pos: usize,
    inner: TcpStream,
}

impl Read for Replay {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos < self.pre.len() {
            let n = buf.len().min(self.pre.len() - self.pos);
            buf[..n].copy_from_slice(&self.pre[self.pos..self.pos + n]);
            self.pos += n;
            return Ok(n);
        }
        self.inner.read(buf)
    }
}

impl Write for Replay {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// The request's head (up to the empty line, 64 Kio at most): parsed, as read, and what came
/// after it.
type HeadRead = (TcpStream, crate::proxy::Head, Vec<u8>, Vec<u8>);

fn read_head(mut stream: TcpStream) -> Result<HeadRead, String> {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = crate::proxy::parse_head(&buf[..end + 4]).ok_or("not HTTP")?;
            let rest = buf[end + 4..].to_vec();
            buf.truncate(end + 4);
            return Ok((stream, head, buf, rest));
        }
        if buf.len() > 64 * 1024 {
            return Err("head too long".into());
        }
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("closed".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn serve_one(id: u64, stream: Replay, db: &Arc<Mutex<Db>>, conns: &Conns) -> Result<(), String> {
    let mut ws =
        tungstenite::accept_with_config(stream, Some(ws_config())).map_err(|e| e.to_string())?;
    let info = db.lock().map_err(|e| e.to_string())?.info();
    let nonce = crate::identity::random_hex(32);
    send_json(
        &mut ws,
        &Greeting::Challenge {
            space: info.id.clone(),
            nonce: nonce.clone(),
        },
    )?;
    let auth: Greeting = read_json(&mut ws)?;
    let since = match &auth {
        Greeting::Auth { since, .. } => *since,
        _ => 0,
    };
    let key = match &auth {
        Greeting::Auth { key, .. } => key.clone(),
        _ => String::new(),
    };
    let admitted = {
        let d = db.lock().map_err(|e| e.to_string())?;
        admit(&d, &info.id, &nonce, &auth)
    };
    let (name, role) = match admitted {
        Ok(m) => m,
        Err(reason) => {
            let _ = send_json(
                &mut ws,
                &Greeting::Refused {
                    reason: reason.clone(),
                },
            );
            let _ = ws.close(None);
            return Err(reason);
        }
    };
    eprintln!("+ {name} ({})", role.label());
    let ais = db.lock().map_err(|e| e.to_string())?.ais_for(&key, role);
    send_json(
        &mut ws,
        &Greeting::Welcome {
            space: info,
            name: name.clone(),
            role,
            ais,
        },
    )?;
    // What came after the last change they have; then the others' news as it comes. Queued
    // under the lock: nothing stored meanwhile can be numbered before and sent after.
    let (tx, rx) = channel::<String>();
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        let (pages, deleted, chat) = d.since(since);
        eprintln!(
            "  {name} : {} page(s), {} suppression(s), {} message(s) après le changement {since}",
            pages.len(),
            deleted.len(),
            chat.len()
        );
        let _ = tx.send(json(&LiveMsg::Pages { pages, deleted }));
        let _ = tx.send(json(&LiveMsg::Chat { msgs: chat }));
        let _ = tx.send(json(&Greeting::Seq {
            n: d.seq(),
            ack: false,
        }));
        if let Ok(mut l) = conns.lock() {
            l.push(Conn {
                id,
                out: tx.clone(),
            });
        }
    }
    let _ = ws.get_mut().inner.set_read_timeout(Some(TICK));
    let mut checked = Instant::now();
    let result = pump(
        &mut ws,
        &rx,
        |text| {
            let Ok(msg) = serde_json::from_str::<LiveMsg>(text) else {
                return;
            };
            match msg {
                LiveMsg::Pages { pages, deleted } => {
                    let Ok(d) = db.lock() else { return };
                    if role != Role::Reader {
                        let (pages, deleted) = d.absorb(pages, deleted);
                        if !pages.is_empty() || !deleted.is_empty() {
                            let mark = json(&Greeting::Seq {
                                n: d.seq(),
                                ack: false,
                            });
                            pass_on(conns, id, &[json(&LiveMsg::Pages { pages, deleted }), mark]);
                        }
                    }
                    let _ = tx.send(json(&Greeting::Seq {
                        n: d.seq(),
                        ack: true,
                    }));
                }
                LiveMsg::Chat { msgs } => {
                    let Ok(d) = db.lock() else { return };
                    if role != Role::Reader {
                        let msgs = d.absorb_chat(msgs);
                        if !msgs.is_empty() {
                            let mark = json(&Greeting::Seq {
                                n: d.seq(),
                                ack: false,
                            });
                            pass_on(conns, id, &[json(&LiveMsg::Chat { msgs }), mark]);
                        }
                    }
                    let _ = tx.send(json(&Greeting::Seq {
                        n: d.seq(),
                        ack: true,
                    }));
                }
                // Only an administrator (checked again: the role may have changed since).
                LiveMsg::Admin { req } => {
                    let Ok(d) = db.lock() else { return };
                    let view = match (d.member(&key), serde_json::from_value::<AdminOp>(req)) {
                        (Some((_, Role::Admin)), Ok(op)) => d.admin(op, &key),
                        (Some((_, Role::Admin)), Err(e)) => AdminView {
                            error: e.to_string(),
                            ..Default::default()
                        },
                        _ => AdminView {
                            error: crate::tr!(
                                "réservé aux administrateurs de l'espace",
                                "for the space's administrators only"
                            )
                            .into(),
                            ..Default::default()
                        },
                    };
                    let _ = tx.send(json(&Greeting::AdminReply { view }));
                }
                // Under the name the server knows, never the one a message claims.
                LiveMsg::Presence {
                    page, project, ai, ..
                } => pass_on(
                    conns,
                    id,
                    &[json(&LiveMsg::Presence {
                        name: name.clone(),
                        page,
                        project,
                        ai,
                    })],
                ),
                _ => {}
            }
        },
        || {
            // Taken off the space meanwhile: cut off.
            if checked.elapsed() < RECHECK {
                return None;
            }
            checked = Instant::now();
            let gone = db.lock().map(|d| d.member(&key).is_none()).unwrap_or(false);
            gone.then(|| {
                json(&Greeting::Refused {
                    reason: crate::tr!(
                        "votre accès à cet espace a été retiré",
                        "your access to this space was removed"
                    )
                    .into(),
                })
            })
        },
    );
    eprintln!("- {name}");
    pass_on(conns, id, &[json(&LiveMsg::Bye { name })]);
    result
}

fn send_json<S: Read + Write, T: Serialize>(ws: &mut WebSocket<S>, v: &T) -> Result<(), String> {
    ws.send(Message::text(json(v))).map_err(|e| e.to_string())
}

/// The next text message, as `T` (blocking, within the socket's read timeout).
fn read_json<S: Read + Write, T: for<'a> Deserialize<'a>>(
    ws: &mut WebSocket<S>,
) -> Result<T, String> {
    loop {
        match ws.read().map_err(|e| e.to_string())? {
            Message::Text(t) => {
                return serde_json::from_str(t.as_str()).map_err(|e| e.to_string());
            }
            Message::Close(_) => {
                return Err(crate::tr!("connexion fermée", "connection closed").into());
            }
            _ => {}
        }
    }
}

/// Until the connection ends: send what is queued in `rx`, hand each text received to `on`;
/// `cut` may end it with a last message.
fn pump<S: Read + Write>(
    ws: &mut WebSocket<S>,
    rx: &Receiver<String>,
    mut on: impl FnMut(&str),
    mut cut: impl FnMut() -> Option<String>,
) -> Result<(), String> {
    loop {
        while let Ok(text) = rx.try_recv() {
            ws.send(Message::text(text)).map_err(|e| e.to_string())?;
        }
        if let Some(last) = cut() {
            let _ = ws.send(Message::text(last));
            let _ = ws.close(None);
            return Ok(());
        }
        match ws.read() {
            Ok(Message::Text(t)) => on(t.as_str()),
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(e) if idle(&e) => {}
            Err(tungstenite::Error::ConnectionClosed) => return Ok(()),
            Err(e) => return Err(e.to_string()),
        }
    }
}

// ------------------------------------------------------------------ the app's side

/// The connection under the WebSocket: plain (`ws://`) or TLS (`wss://`).
enum Net {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Net {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Net::Plain(s) => s.read(buf),
            Net::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Net {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Net::Plain(s) => s.write(buf),
            Net::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Net::Plain(s) => s.flush(),
            Net::Tls(s) => s.flush(),
        }
    }
}

/// The TLS settings: the usual certificate authorities (those of the Mozilla list).
pub fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            // A company's own certificate authority (PEM file), for a server whose
            // certificate it signed.
            if let Ok(path) = std::env::var("ORCHESTRA_CA")
                && !path.trim().is_empty()
            {
                use rustls::pki_types::pem::PemObject;
                match rustls::pki_types::CertificateDer::pem_file_iter(path.trim()) {
                    Ok(certs) => {
                        for c in certs.flatten() {
                            let _ = roots.add(c);
                        }
                    }
                    Err(e) => eprintln!("orchestra: ORCHESTRA_CA illisible : {e}"),
                }
            }
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            Arc::new(
                rustls::ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .expect("TLS versions")
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

/// `ws://host:port/…` or `wss://host/…` → (host, port, TLS or not).
fn endpoint(url: &str) -> Option<(String, u16, bool)> {
    let (rest, tls) = if let Some(r) = url.strip_prefix("wss://") {
        (r, true)
    } else {
        (url.strip_prefix("ws://")?, false)
    };
    let hostport = rest.split('/').next()?.trim();
    if hostport.is_empty() {
        return None;
    }
    let default = if tls { 443 } else { DEFAULT_PORT };
    // An IPv6 address in brackets: [::1]:7879.
    if let Some(r) = hostport.strip_prefix('[') {
        let (h, after) = r.split_once(']')?;
        let port = after
            .strip_prefix(':')
            .map(|p| p.parse().ok())
            .unwrap_or(Some(default))?;
        return Some((h.to_string(), port, tls));
    }
    match hostport.rsplit_once(':') {
        Some((h, p)) => Some((h.to_string(), p.parse().ok()?, tls)),
        None => Some((hostport.to_string(), default, tls)),
    }
}

/// Connect to the team server of space `space_id` (its local id) at `url`, as this computer's
/// identity, with `invite` on a first visit, asking for what came after change `since`.
/// Retries every 3 s after losing the connection; stops for good when the server refuses.
/// What the server says of its space (name, AI rule, role) goes into the list of spaces.
pub fn connect(
    space_id: &str,
    url: &str,
    invite: &str,
    name: &str,
    since: u64,
    wake: live::Wake,
) -> live::Live {
    let (tx, inbox) = channel::<LiveMsg>();
    let stop = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(Status {
        address: url.to_string(),
        ..Default::default()
    }));
    let outbox: Arc<Mutex<Option<Sender<String>>>> = Arc::new(Mutex::new(None));
    {
        let (stop, status, outbox) = (stop.clone(), status.clone(), outbox.clone());
        let (space_id, url, name) = (space_id.to_string(), url.to_string(), name.to_string());
        let mut invite = invite.to_string();
        let mut since = since;
        let mut last: Option<String> = None;
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let mut s = Session {
                    url: &url,
                    invite: &mut invite,
                    name: &name,
                    space_id: &space_id,
                    since: &mut since,
                    tx: &tx,
                    outbox: &outbox,
                    status: &status,
                    stop: &stop,
                    wake: &wake,
                };
                if let Err((reason, refused)) = s.run() {
                    if refused {
                        stop.store(true, Ordering::SeqCst);
                    }
                    // Said once: not again at each attempt while the server stays out of reach.
                    let was_in = status.lock().map(|st| st.connected).unwrap_or(false);
                    let news = was_in || refused || last.as_deref() != Some(reason.as_str());
                    if let Ok(mut st) = status.lock() {
                        st.connected = false;
                        st.error = Some(reason.clone());
                    }
                    if news {
                        let _ = tx.send(LiveMsg::Disconnected {
                            reason: reason.clone(),
                        });
                        wake();
                    }
                    last = Some(reason);
                }
                *outbox.lock().unwrap_or_else(|e| e.into_inner()) = None;
                for _ in 0..30 {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
    }
    let out_box = outbox.clone();
    let out: Arc<dyn Fn(&LiveMsg) + Send + Sync> = Arc::new(move |msg: &LiveMsg| {
        if let Some(s) = out_box.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = s.send(json(msg));
        }
    });
    live::Live::from_parts(inbox, out, status, stop)
}

/// One connection, from the greeting to its end.
struct Session<'a> {
    url: &'a str,
    invite: &'a mut String,
    name: &'a str,
    space_id: &'a str,
    since: &'a mut u64,
    tx: &'a Sender<LiveMsg>,
    outbox: &'a Arc<Mutex<Option<Sender<String>>>>,
    status: &'a Arc<Mutex<Status>>,
    stop: &'a Arc<AtomicBool>,
    wake: &'a live::Wake,
}

impl Session<'_> {
    /// Err((why, refused by the server: no new attempt)).
    fn run(&mut self) -> Result<(), (String, bool)> {
        let net = |e: String| {
            (
                crate::trf!("connexion impossible : {e}", "connection failed: {e}"),
                false,
            )
        };
        let (host, port, tls) = endpoint(self.url).ok_or_else(|| {
            (
                crate::tr!("adresse invalide", "invalid address").to_string(),
                true,
            )
        })?;
        // Each address the name gives, in turn (« localhost »: IPv6 first, the server may
        // listen on IPv4 only).
        let addrs: Vec<_> = std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), port))
            .map_err(|e| net(e.to_string()))?
            .collect();
        let mut last = crate::tr!("adresse introuvable", "address not found").to_string();
        let mut tcp = None;
        for a in &addrs {
            match TcpStream::connect_timeout(a, Duration::from_secs(5)) {
                Ok(s) => {
                    tcp = Some(s);
                    break;
                }
                Err(e) => last = e.to_string(),
            }
        }
        let tcp = tcp.ok_or_else(|| net(last))?;
        let _ = tcp.set_nodelay(true);
        let _ = tcp.set_read_timeout(Some(Duration::from_secs(10)));
        let _ = tcp.set_write_timeout(Some(Duration::from_secs(10)));
        // Kept to change the read timeout once greeted (the WebSocket owns the stream).
        let raw = tcp.try_clone().map_err(|e| net(e.to_string()))?;
        let stream = if tls {
            let server = rustls::pki_types::ServerName::try_from(host.clone())
                .map_err(|e| net(e.to_string()))?;
            let conn = rustls::ClientConnection::new(tls_config(), server)
                .map_err(|e| net(e.to_string()))?;
            Net::Tls(Box::new(rustls::StreamOwned::new(conn, tcp)))
        } else {
            Net::Plain(tcp)
        };
        let (mut ws, _) =
            tungstenite::client::client_with_config(self.url, stream, Some(ws_config()))
                .map_err(|e| {
                    let e = e.to_string();
                    // A certificate the app doesn't trust: trying again won't change it.
                    if e.contains("certificate") {
                        (
                            crate::trf!(
                                "certificat du serveur non reconnu ({e}). Si votre entreprise a sa propre autorité de certification, indiquez son fichier avec ORCHESTRA_CA.",
                                "server certificate not trusted ({e}). If your company has its own certificate authority, give its file with ORCHESTRA_CA."
                            ),
                            true,
                        )
                    } else {
                        net(e)
                    }
                })?;
        let key = crate::identity::key().map_err(|e| (e, true))?;
        let (server_space, nonce) = match read_json::<_, Greeting>(&mut ws).map_err(net)? {
            Greeting::Challenge { space, nonce } => (space, nonce),
            _ => {
                return Err(net(
                    crate::tr!("serveur inattendu", "unexpected server").into()
                ));
            }
        };
        send_json(
            &mut ws,
            &Greeting::Auth {
                key: crate::identity::public_hex(&key),
                name: self.name.to_string(),
                sig: crate::identity::sign_hex(&key, &to_sign(&server_space, &nonce)),
                invite: self.invite.clone(),
                since: *self.since,
            },
        )
        .map_err(net)?;
        let (info, role, ais) = match read_json::<_, Greeting>(&mut ws).map_err(net)? {
            Greeting::Welcome {
                space, role, ais, ..
            } => (space, role, ais),
            Greeting::Refused { reason } => return Err((reason, true)),
            _ => {
                return Err(net(
                    crate::tr!("serveur inattendu", "unexpected server").into()
                ));
            }
        };
        // In: the invitation is spent; later visits go by the key alone.
        self.invite.clear();
        remember(self.space_id, &info, role, ais);
        let (otx, orx) = channel::<String>();
        *self.outbox.lock().unwrap_or_else(|e| e.into_inner()) = Some(otx);
        if let Ok(mut st) = self.status.lock() {
            st.connected = true;
            st.error = None;
        }
        let _ = self.tx.send(LiveMsg::Connected);
        (self.wake)();
        let _ = raw.set_read_timeout(Some(TICK));
        let mut refused: Option<String> = None;
        let mut stopped = false;
        let (tx, wake, stop) = (self.tx, self.wake, self.stop);
        let since = &mut *self.since;
        let r = pump(
            &mut ws,
            &orx,
            |text| {
                if let Ok(msg) = serde_json::from_str::<LiveMsg>(text) {
                    let _ = tx.send(msg);
                    wake();
                    return;
                }
                match serde_json::from_str::<Greeting>(text) {
                    Ok(Greeting::Seq { n, ack }) => {
                        *since = (*since).max(n);
                        let _ = tx.send(LiveMsg::Synced { seq: n, ack });
                        wake();
                    }
                    Ok(Greeting::Refused { reason }) => refused = Some(reason),
                    Ok(Greeting::AdminReply { view }) => {
                        let _ = tx.send(LiveMsg::AdminReply {
                            reply: serde_json::to_value(view).unwrap_or_default(),
                        });
                        wake();
                    }
                    _ => {}
                }
            },
            || {
                if stop.load(Ordering::SeqCst) {
                    stopped = true;
                    return Some(json(&LiveMsg::Bye {
                        name: String::new(),
                    }));
                }
                None
            },
        );
        if let Some(reason) = refused {
            return Err((reason, true));
        }
        if stopped {
            return Ok(());
        }
        Err((
            r.err()
                .unwrap_or_else(|| crate::tr!("connexion perdue", "connection lost").into()),
            false,
        ))
    }
}

/// The team server's address for plain HTTP (its relay of the space's AIs):
/// `wss://equipe.example/x` → `https://equipe.example/x`, `ws://…` → `http://…`.
pub fn http_base(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    if let Some(r) = u.strip_prefix("wss://") {
        format!("https://{r}")
    } else if let Some(r) = u.strip_prefix("ws://") {
        format!("http://{r}")
    } else {
        u.to_string()
    }
}

/// What the server says of its space goes into the list: its name, its AI rule, the role
/// given here. The invitation is spent.
fn remember(space_id: &str, info: &SpaceInfo, role: Role, ais: Vec<crate::space::SpaceAi>) {
    let mut list = crate::space::load();
    if let Some(s) = list.spaces.iter_mut().find(|s| s.id == space_id) {
        if !info.name.trim().is_empty() {
            s.name = info.name.clone();
        }
        s.ai_rule = info.ai_rule;
        s.allowed_ai = info.allowed_ai.clone();
        s.role = role.key().to_string();
        // Replaced each time: an AI or a key the server took back is gone here too. Their
        // address is this server's relay, completed with the server's own address.
        let base = http_base(&s.server().map(|(u, _)| u.to_string()).unwrap_or_default());
        s.ais = ais
            .into_iter()
            .map(|mut a| {
                if a.base_url.starts_with('/') {
                    a.base_url = format!("{base}{}", a.base_url);
                }
                a
            })
            .collect();
        if let crate::space::SpaceKind::Server { invite, .. } = &mut s.kind {
            invite.clear();
        }
        let _ = crate::space::save(&list);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invitation_links_and_addresses() {
        let t = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            parse_invitation(&format!("ws://192.168.1.20:7879/?invitation={t}")),
            Some(("ws://192.168.1.20:7879".into(), t.into()))
        );
        assert_eq!(
            parse_invitation(&format!("  wss://equipe.example/?invitation={t} ")),
            Some(("wss://equipe.example".into(), t.into()))
        );
        assert_eq!(parse_invitation("ws://x/?invitation=court"), None);
        assert_eq!(parse_invitation(&format!("http://x/?invitation={t}")), None);
        assert_eq!(parse_invitation("ws://x/"), None);
        assert_eq!(
            endpoint("ws://machine/"),
            Some(("machine".into(), DEFAULT_PORT, false))
        );
        assert_eq!(
            endpoint("ws://10.0.0.2:9000"),
            Some(("10.0.0.2".into(), 9000, false))
        );
        assert_eq!(
            endpoint("wss://equipe.example/orchestra"),
            Some(("equipe.example".into(), 443, true))
        );
        assert_eq!(
            endpoint("wss://equipe.example:8443"),
            Some(("equipe.example".into(), 8443, true))
        );
        assert_eq!(
            endpoint("ws://[::1]:7000/"),
            Some(("::1".into(), 7000, false))
        );
        assert_eq!(endpoint("http://x"), None);
    }

    fn db() -> Db {
        let dir = std::env::temp_dir().join(format!("orchestra-equipe-{}", crate::model::new_id()));
        Db::open(&dir).unwrap()
    }

    fn auth(k: &ed25519_dalek::SigningKey, space: &str, nonce: &str, invite: &str) -> Greeting {
        Greeting::Auth {
            key: crate::identity::public_hex(k),
            name: "Ana".into(),
            sig: crate::identity::sign_hex(k, &to_sign(space, nonce)),
            invite: invite.into(),
            since: 0,
        }
    }

    #[test]
    fn only_members_or_invited_keys_come_in_and_an_invitation_serves_once() {
        let d = db();
        let space = d.info().id;
        let ana = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let bob = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        // Unknown, no invitation.
        assert!(admit(&d, &space, "n1", &auth(&ana, &space, "n1", "")).is_err());
        let inv = d.invite(Role::Member).unwrap();
        // A signature for another challenge, or another space, is worth nothing.
        assert!(admit(&d, &space, "n2", &auth(&ana, &space, "n1", &inv)).is_err());
        assert!(admit(&d, &space, "n2", &auth(&ana, "autre", "n2", &inv)).is_err());
        // The first one in is the administrator.
        assert_eq!(
            admit(&d, &space, "n3", &auth(&ana, &space, "n3", &inv)),
            Ok(("Ana".into(), Role::Admin))
        );
        // Spent: Bob can't use it; Ana comes back without it.
        assert!(admit(&d, &space, "n4", &auth(&bob, &space, "n4", &inv)).is_err());
        assert!(admit(&d, &space, "n5", &auth(&ana, &space, "n5", "")).is_ok());
        // A reader's invitation makes a reader.
        let read = d.invite(Role::Reader).unwrap();
        assert_eq!(
            admit(&d, &space, "n6", &auth(&bob, &space, "n6", &read)).map(|m| m.1),
            Ok(Role::Reader)
        );
        // Taken off: refused.
        let ana_key = crate::identity::public_hex(&ana);
        d.set_revoked(&ana_key, true).unwrap();
        assert!(admit(&d, &space, "n7", &auth(&ana, &space, "n7", "")).is_err());
        assert_eq!(d.members(), 1);
    }

    #[test]
    fn members_are_found_by_name_or_key() {
        let d = db();
        let space = d.info().id;
        let ana = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let inv = d.invite(Role::Member).unwrap();
        admit(&d, &space, "n", &auth(&ana, &space, "n", &inv)).unwrap();
        let key = crate::identity::public_hex(&ana);
        assert_eq!(d.find_member("Ana").unwrap().key, key);
        assert_eq!(d.find_member(&key[..10]).unwrap().name, "Ana");
        assert!(d.find_member("Bob").is_err());
        assert!(
            d.find_member(&key[..4]).is_err(),
            "too short a start of key"
        );
        d.set_role(&key, Role::Reader).unwrap();
        assert_eq!(d.find_member("Ana").unwrap().role, Role::Reader);
    }

    #[test]
    fn the_server_merges_pages_block_by_block_and_keeps_deletions() {
        let d = db();
        let mut p = Page::new("Plan", None);
        p.blocks = vec![crate::model::Block::new(
            crate::model::BlockKind::Paragraph,
            "a",
        )];
        p.updated = 10;
        let (out, _) = d.absorb(vec![p.clone()], BTreeMap::new());
        assert_eq!(out.len(), 1);
        // The same again: nothing new to pass on.
        assert!(d.absorb(vec![p.clone()], BTreeMap::new()).0.is_empty());
        // Another block added elsewhere: kept with the first.
        let mut q = p.clone();
        q.blocks.push(crate::model::Block::new(
            crate::model::BlockKind::Paragraph,
            "b",
        ));
        q.updated = 20;
        let (out, _) = d.absorb(vec![q], BTreeMap::new());
        assert_eq!(out[0].blocks.len(), 2);
        // Deleted for good after: gone, and an older copy doesn't bring it back.
        let mut del = BTreeMap::new();
        del.insert(p.id.clone(), 30);
        let (_, gone) = d.absorb(vec![], del.clone());
        assert_eq!(gone, del);
        assert!(d.everything().0.is_empty());
        assert!(d.absorb(vec![p], BTreeMap::new()).0.is_empty());
    }

    #[test]
    fn members_get_a_token_for_the_relay_never_the_company_key() {
        let d = db();
        let space = d.info().id;
        let ana = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let bob = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let i1 = d.invite(Role::Member).unwrap();
        admit(&d, &space, "n", &auth(&ana, &space, "n", &i1)).unwrap();
        let i2 = d.invite(Role::Member).unwrap();
        admit(&d, &space, "m", &auth(&bob, &space, "m", &i2)).unwrap();
        let (ka, kb) = (
            crate::identity::public_hex(&ana),
            crate::identity::public_hex(&bob),
        );
        d.set_space_ai(&crate::space::SpaceAi {
            name: "Modèle maison".into(),
            tool: "codex".into(),
            base_url: "https://ia.example/v1".into(),
            ..Default::default()
        })
        .unwrap();
        // No key: nobody gets it.
        assert!(d.ais_for(&ka, Role::Member).is_empty());
        d.set_ai_key("Modèle maison", "*", "cle-commune").unwrap();
        d.set_ai_key("Modèle maison", &ka, "cle-ana").unwrap();
        let got = d.ais_for(&ka, Role::Member);
        assert_eq!(got[0].base_url, "/ia/mod-le-maison");
        assert!(
            got[0].key.starts_with("orch-"),
            "a token, not the company key"
        );
        assert_eq!(got[0].key, d.proxy_token(&ka), "the same token each time");
        assert!(d.ais_for(&kb, Role::Reader).is_empty());
        // Through the relay: each one's company key.
        assert_eq!(
            d.proxy_grant(&got[0].key, "mod-le-maison").unwrap().1,
            "cle-ana"
        );
        let tb = d.proxy_token(&kb);
        assert_eq!(
            d.proxy_grant(&tb, "mod-le-maison").unwrap().1,
            "cle-commune"
        );
        assert!(d.proxy_grant("orch-faux", "mod-le-maison").is_err());
        assert!(d.proxy_grant(&tb, "autre").is_err());
        // Daily quota: Bob made 1 call; at 2, one more passes, then no.
        d.set_quota("Modèle maison", 2).unwrap();
        assert!(d.proxy_grant(&tb, "mod-le-maison").is_ok());
        assert!(d.proxy_grant(&tb, "mod-le-maison").is_err());
        assert_eq!(d.usage(1).iter().map(|u| u.3).sum::<u64>(), 3);
        // Taken off the space: cut off at once.
        d.set_revoked(&ka, true).unwrap();
        assert!(d.proxy_grant(&got[0].key, "mod-le-maison").is_err());
        assert_eq!(d.space_ais().len(), 1);
        d.remove_space_ai("Modèle maison").unwrap();
        assert!(d.ais_for(&kb, Role::Member).is_empty());
        assert_eq!(
            http_base("wss://equipe.example/x/"),
            "https://equipe.example/x"
        );
        assert_eq!(http_base("ws://10.0.0.2:7879"), "http://10.0.0.2:7879");
    }

    #[test]
    fn an_administrator_manages_others_never_themselves_and_sees_no_key() {
        let d = db();
        let space = d.info().id;
        let ana = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let bob = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let i1 = d.invite(Role::Member).unwrap();
        admit(&d, &space, "n", &auth(&ana, &space, "n", &i1)).unwrap();
        let i2 = d.invite(Role::Member).unwrap();
        admit(&d, &space, "m", &auth(&bob, &space, "m", &i2)).unwrap();
        let (ka, kb) = (
            crate::identity::public_hex(&ana),
            crate::identity::public_hex(&bob),
        );
        let v = d.admin(AdminOp::Revoke { key: ka.clone() }, &ka);
        assert!(!v.error.is_empty(), "not on yourself");
        assert!(
            !d.admin(
                AdminOp::SetRole {
                    key: ka.clone(),
                    role: Role::Reader
                },
                &ka
            )
            .error
            .is_empty()
        );
        let v = d.admin(
            AdminOp::SetRole {
                key: kb.clone(),
                role: Role::Reader,
            },
            &ka,
        );
        assert!(v.error.is_empty());
        assert_eq!(
            v.members.iter().find(|m| m.key == kb).unwrap().role,
            Role::Reader
        );
        assert!(v.members.iter().find(|m| m.key == ka).unwrap().me);
        let v = d.admin(AdminOp::Invite { role: Role::Reader }, &ka);
        assert_eq!(v.invite.len(), 32);
        let v = d.admin(
            AdminOp::AddAi {
                ai: crate::space::SpaceAi {
                    name: "Maison".into(),
                    tool: "codex".into(),
                    base_url: "https://ia.example/v1".into(),
                    key: "ne-doit-pas-rester".into(),
                    ..Default::default()
                },
            },
            &ka,
        );
        assert!(v.error.is_empty());
        let v = d.admin(
            AdminOp::SetKey {
                ai: "Maison".into(),
                member: "*".into(),
                key: "cle-secrete".into(),
            },
            &ka,
        );
        let shown = serde_json::to_string(&v).unwrap();
        assert!(!shown.contains("cle-secrete") && !shown.contains("ne-doit-pas-rester"));
        assert_eq!(v.ais[0].keys, 1);
        assert!(
            !d.admin(
                AdminOp::AddAi {
                    ai: Default::default()
                },
                &ka
            )
            .error
            .is_empty()
        );
        let v = d.admin(AdminOp::Revoke { key: kb.clone() }, &ka);
        assert!(v.members.iter().find(|m| m.key == kb).unwrap().revoked);
    }

    #[test]
    fn a_member_coming_back_gets_only_what_came_after() {
        let d = db();
        let page = |t: &str, at: u64| {
            let mut p = Page::new(t, None);
            p.updated = at;
            p
        };
        d.absorb(vec![page("Un", 1), page("Deux", 2)], BTreeMap::new());
        let seen = d.seq();
        assert_eq!(seen, 2);
        d.absorb(vec![page("Trois", 3)], BTreeMap::new());
        let msg = ChatMsg {
            id: "m1".into(),
            project: "p".into(),
            author: "Ana".into(),
            text: "salut".into(),
            at: 4,
        };
        assert_eq!(d.absorb_chat(vec![msg.clone()]).len(), 1);
        assert!(
            d.absorb_chat(vec![msg]).is_empty(),
            "a message is stored once"
        );
        let (pages, _, chat) = d.since(seen);
        assert_eq!(
            pages.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(),
            ["Trois"]
        );
        assert_eq!(chat.len(), 1);
        assert_eq!(d.since(d.seq()).0.len(), 0);
        assert_eq!(d.since(0).0.len(), 3);
    }
}
