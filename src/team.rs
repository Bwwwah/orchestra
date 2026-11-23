//! Team server: a space shared by its members, on a server someone hosts (`orchestra equipe`).
//! See docs/collaboration-serveur.md, step 2.
//!
//! * **Who may connect**: members are known by the public half of their key (identity.rs).
//!   The server sends a fresh challenge; the app answers with its key and a signature of it.
//!   A key the server doesn't know comes in only with an unused invitation (one use each),
//!   which makes it a member. No password, no shared code.
//! * **Wire**: WebSocket, one JSON message per frame. After the greeting, the messages are
//!   those of live collaboration (`live::LiveMsg`), so the window merges, sends and shows
//!   presence exactly as in live mode.
//! * **Storage**: SQLite (`espace.sqlite`), one row per page. A page received is merged into
//!   the stored one block by block (`collab::merge_page`), then passed on to the others.
//! * **Encryption**: none of its own. On the Internet, put the server behind an HTTPS proxy
//!   (Caddy, Traefik…) and join with `wss://` (coming next: the app speaks `ws://` only for
//!   now, so keep to a private network: same network, Tailscale, WireGuard…).

use crate::live::{self, LiveMsg, Status};
use crate::model::{ChatMsg, Page};
use crate::space::AiRule;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tungstenite::{Message, WebSocket};

pub const DEFAULT_PORT: u16 = 7879;
/// Connections the server keeps at once.
const MAX_CONNECTIONS: usize = 256;
/// Largest message accepted (a whole workspace sent at once, with its pictures' links).
const MAX_MESSAGE: usize = 32 * 1024 * 1024;
/// How long a reader waits before looking at what is to be sent.
const TICK: Duration = Duration::from_millis(100);

/// The greeting, before the messages of live collaboration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Greeting {
    /// Server → app: sign this.
    Challenge {
        space: String,
        nonce: String,
    },
    /// App → server: who I am, the signature, and an invitation for a first visit.
    Auth {
        key: String,
        name: String,
        sig: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        invite: String,
    },
    /// Server → app: welcome, with what the space is.
    Welcome {
        space: SpaceInfo,
        name: String,
    },
    Refused {
        reason: String,
    },
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
                 updated INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS deleted (id TEXT PRIMARY KEY, at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS chat (id TEXT PRIMARY KEY, json TEXT NOT NULL);",
        )?;
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

    /// A new one-time invitation (the first person to come in becomes the administrator,
    /// whichever invitation they use: see `redeem`).
    pub fn invite(&self) -> rusqlite::Result<String> {
        let token = crate::identity::random_hex(16);
        let role = "member";
        self.0.execute(
            "INSERT INTO invites (token, role, created) VALUES (?1, ?2, ?3)",
            params![token, role, crate::model::now_ms() as i64],
        )?;
        Ok(token)
    }

    /// The name a known, not revoked key goes by.
    fn member(&self, key: &str) -> Option<String> {
        self.0
            .query_row(
                "SELECT name FROM members WHERE key = ?1 AND revoked = 0",
                [key],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
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

    /// Spend `token` to make `key` a member. False when it doesn't exist or was used.
    fn redeem(&self, token: &str, key: &str, name: &str) -> bool {
        let role: Option<String> = self
            .0
            .query_row(
                "SELECT role FROM invites WHERE token = ?1 AND used_by IS NULL",
                [token],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        let Some(role) = role else { return false };
        let role = if self.members() == 0 {
            "admin".to_string()
        } else {
            role
        };
        let used = self
            .0
            .execute(
                "UPDATE invites SET used_by = ?2 WHERE token = ?1 AND used_by IS NULL",
                params![token, key],
            )
            .unwrap_or(0);
        used == 1
            && self
                .0
                .execute(
                    "INSERT INTO members (key, name, role, joined) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(key) DO NOTHING",
                    params![key, name, role, crate::model::now_ms() as i64],
                )
                .is_ok()
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

    /// Everything, for whoever connects.
    pub fn everything(&self) -> (Vec<Page>, BTreeMap<String, u64>, Vec<ChatMsg>) {
        let rows = |sql: &str| -> Vec<String> {
            let Ok(mut st) = self.0.prepare(sql) else {
                return vec![];
            };
            st.query_map([], |r| r.get::<_, String>(0))
                .map(|it| it.flatten().collect())
                .unwrap_or_default()
        };
        let pages = rows("SELECT json FROM pages")
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let chat = rows("SELECT json FROM chat")
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let mut deleted = BTreeMap::new();
        if let Ok(mut st) = self.0.prepare("SELECT id, at FROM deleted")
            && let Ok(it) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        {
            for (id, at) in it.flatten() {
                deleted.insert(id, at as u64);
            }
        }
        (pages, deleted, chat)
    }

    /// Merge pages and deletions a member sent. Returns the pages as they now stand, and the
    /// deletions that are news, to pass on.
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
            let _ = self.0.execute(
                "INSERT INTO deleted (id, at) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET at = MAX(at, ?2)",
                params![id, at as i64],
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
            let merged = match self.page(&p.id) {
                Some(mut ours) => {
                    if !crate::collab::merge_page(&mut ours, &p) {
                        continue; // nothing new
                    }
                    ours
                }
                None => p,
            };
            let _ = self.0.execute(
                "INSERT INTO pages (id, json, updated) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET json = ?2, updated = ?3",
                params![
                    merged.id,
                    serde_json::to_string(&merged).unwrap_or_default(),
                    merged.updated as i64
                ],
            );
            out.push(merged);
        }
        (out, out_deleted)
    }

    /// New discussion messages (never edited): those not stored yet.
    pub fn absorb_chat(&self, msgs: Vec<ChatMsg>) -> Vec<ChatMsg> {
        msgs.into_iter()
            .filter(|m| {
                !m.id.is_empty()
                    && self
                        .0
                        .execute(
                            "INSERT INTO chat (id, json) VALUES (?1, ?2) ON CONFLICT(id) DO NOTHING",
                            params![m.id, serde_json::to_string(m).unwrap_or_default()],
                        )
                        .unwrap_or(0)
                        == 1
            })
            .collect()
    }
}

/// Check a member's greeting; returns their name, or why not.
fn admit(db: &Db, space: &str, nonce: &str, auth: &Greeting) -> Result<String, String> {
    let Greeting::Auth {
        key,
        name,
        sig,
        invite,
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
    if let Some(n) = db.member(key) {
        return Ok(n);
    }
    let name = name.trim().chars().take(60).collect::<String>();
    let name = if name.is_empty() {
        crate::tr!("Membre", "Member").to_string()
    } else {
        name
    };
    if !invite.is_empty() && db.redeem(invite, key, &name) {
        return Ok(name);
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

fn pass_on(conns: &Conns, from: u64, text: &str) {
    if let Ok(list) = conns.lock() {
        for c in list.iter().filter(|c| c.id != from) {
            let _ = c.out.send(text.to_string());
        }
    }
}

fn json(msg: &LiveMsg) -> String {
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
    let next = AtomicU64::new(1);
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if conns.lock().map(|c| c.len()).unwrap_or(0) >= MAX_CONNECTIONS {
            continue;
        }
        let id = next.fetch_add(1, Ordering::SeqCst);
        let (db, conns) = (db.clone(), conns.clone());
        std::thread::spawn(move || {
            if let Err(e) = serve_one(id, stream, &db, &conns) {
                eprintln!("orchestra: connexion {id} : {e}");
            }
            if let Ok(mut l) = conns.lock() {
                l.retain(|c| c.id != id);
            }
        });
    }
    Ok(())
}

fn serve_one(id: u64, stream: TcpStream, db: &Arc<Mutex<Db>>, conns: &Conns) -> Result<(), String> {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
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
    let admitted = {
        let d = db.lock().map_err(|e| e.to_string())?;
        admit(&d, &info.id, &nonce, &auth)
    };
    let name = match admitted {
        Ok(n) => n,
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
    eprintln!("+ {name}");
    send_json(
        &mut ws,
        &Greeting::Welcome {
            space: info,
            name: name.clone(),
        },
    )?;
    // Everything the space holds, then the others' news as it comes.
    let (pages, deleted, chat) = db.lock().map_err(|e| e.to_string())?.everything();
    ws.send(Message::text(json(&LiveMsg::Pages { pages, deleted })))
        .map_err(|e| e.to_string())?;
    ws.send(Message::text(json(&LiveMsg::Chat { msgs: chat })))
        .map_err(|e| e.to_string())?;
    let (tx, rx) = channel::<String>();
    if let Ok(mut l) = conns.lock() {
        l.push(Conn { id, out: tx });
    }
    let _ = ws.get_mut().set_read_timeout(Some(TICK));
    let result = pump(&mut ws, &rx, |text| {
        let Ok(msg) = serde_json::from_str::<LiveMsg>(text) else {
            return;
        };
        match msg {
            LiveMsg::Pages { pages, deleted } => {
                let Ok(d) = db.lock() else { return };
                let (pages, deleted) = d.absorb(pages, deleted);
                drop(d);
                if !pages.is_empty() || !deleted.is_empty() {
                    pass_on(conns, id, &json(&LiveMsg::Pages { pages, deleted }));
                }
            }
            LiveMsg::Chat { msgs } => {
                let Ok(d) = db.lock() else { return };
                let msgs = d.absorb_chat(msgs);
                drop(d);
                if !msgs.is_empty() {
                    pass_on(conns, id, &json(&LiveMsg::Chat { msgs }));
                }
            }
            // Under the name the server knows, never the one a message claims.
            LiveMsg::Presence {
                page, project, ai, ..
            } => pass_on(
                conns,
                id,
                &json(&LiveMsg::Presence {
                    name: name.clone(),
                    page,
                    project,
                    ai,
                }),
            ),
            _ => {}
        }
    });
    eprintln!("- {name}");
    pass_on(conns, id, &json(&LiveMsg::Bye { name }));
    result
}

fn send_json<T: Serialize>(ws: &mut WebSocket<TcpStream>, v: &T) -> Result<(), String> {
    ws.send(Message::text(serde_json::to_string(v).unwrap_or_default()))
        .map_err(|e| e.to_string())
}

/// The next text message, as `T` (blocking, within the socket's read timeout).
fn read_json<T: for<'a> Deserialize<'a>>(ws: &mut WebSocket<TcpStream>) -> Result<T, String> {
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

/// Until the connection ends: send what is queued in `rx`, hand each text received to `on`.
fn pump(
    ws: &mut WebSocket<TcpStream>,
    rx: &Receiver<String>,
    mut on: impl FnMut(&str),
) -> Result<(), String> {
    loop {
        while let Ok(text) = rx.try_recv() {
            ws.send(Message::text(text)).map_err(|e| e.to_string())?;
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

/// `ws://host:port[/path]` → ("host:port", the URL).
fn host_port(url: &str) -> Option<String> {
    let rest = url.strip_prefix("ws://")?;
    let host = rest.split('/').next()?.trim();
    if host.is_empty() {
        return None;
    }
    Some(
        if host
            .rsplit_once(':')
            .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
        {
            host.to_string()
        } else {
            format!("{host}:{DEFAULT_PORT}")
        },
    )
}

/// Connect to the team server of space `space_id` (its local id) at `url`, as this computer's
/// identity, with `invite` on a first visit. Retries every 3 s after losing the connection;
/// stops for good when the server refuses. What the server says of its space (name, AI rule)
/// goes into the list of spaces.
pub fn connect(
    space_id: &str,
    url: &str,
    invite: &str,
    name: &str,
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
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let refused = match session(
                    &url, &invite, &name, &space_id, &tx, &outbox, &status, &stop, &wake,
                ) {
                    Ok(()) => false,
                    Err((reason, refused)) => {
                        if refused {
                            stop.store(true, Ordering::SeqCst);
                        }
                        if let Ok(mut st) = status.lock() {
                            st.connected = false;
                            st.error = Some(reason.clone());
                        }
                        let _ = tx.send(LiveMsg::Disconnected { reason });
                        wake();
                        refused
                    }
                };
                *outbox.lock().unwrap_or_else(|e| e.into_inner()) = None;
                if refused {
                    stop.store(true, Ordering::SeqCst);
                    wake();
                    break;
                }
                // Once in, the invitation is spent: later visits go by the key alone.
                if status.lock().map(|s| s.peers > 0).unwrap_or(false) {
                    invite.clear();
                }
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

/// One connection, from the greeting to its end. Err((why, refused by the server)).
#[allow(clippy::too_many_arguments)]
fn session(
    url: &str,
    invite: &str,
    name: &str,
    space_id: &str,
    tx: &Sender<LiveMsg>,
    outbox: &Arc<Mutex<Option<Sender<String>>>>,
    status: &Arc<Mutex<Status>>,
    stop: &Arc<AtomicBool>,
    wake: &live::Wake,
) -> Result<(), (String, bool)> {
    let net = |e: String| {
        (
            crate::trf!("connexion impossible : {e}", "connection failed: {e}"),
            false,
        )
    };
    if url.starts_with("wss://") {
        return Err((
            crate::tr!(
                "les adresses wss:// (HTTPS) arrivent bientôt ; utilisez ws:// sur un réseau privé",
                "wss:// (HTTPS) addresses are coming soon; use ws:// on a private network"
            )
            .into(),
            true,
        ));
    }
    let target = host_port(url).ok_or_else(|| {
        (
            crate::tr!("adresse invalide", "invalid address").to_string(),
            true,
        )
    })?;
    let addr = std::net::ToSocketAddrs::to_socket_addrs(&target)
        .map_err(|e| net(e.to_string()))?
        .next()
        .ok_or_else(|| net(crate::tr!("adresse introuvable", "address not found").into()))?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| net(e.to_string()))?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let (mut ws, _) = tungstenite::client::client_with_config(url, stream, Some(ws_config()))
        .map_err(|e| net(e.to_string()))?;
    let key = crate::identity::key().map_err(|e| (e, true))?;
    let (server_space, nonce) = match read_json::<Greeting>(&mut ws).map_err(net)? {
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
            name: name.to_string(),
            sig: crate::identity::sign_hex(&key, &to_sign(&server_space, &nonce)),
            invite: invite.to_string(),
        },
    )
    .map_err(net)?;
    let info = match read_json::<Greeting>(&mut ws).map_err(net)? {
        Greeting::Welcome { space, .. } => space,
        Greeting::Refused { reason } => return Err((reason, true)),
        _ => {
            return Err(net(
                crate::tr!("serveur inattendu", "unexpected server").into()
            ));
        }
    };
    remember(space_id, &info);
    let (otx, orx) = channel::<String>();
    *outbox.lock().unwrap_or_else(|e| e.into_inner()) = Some(otx);
    if let Ok(mut st) = status.lock() {
        st.connected = true;
        st.error = None;
        st.peers = 1; // in: the invitation is spent
    }
    let _ = tx.send(LiveMsg::Connected);
    wake();
    let _ = ws.get_mut().set_read_timeout(Some(TICK));
    let mut stopped = false;
    let r = pump_until(&mut ws, &orx, stop, &mut stopped, |text| {
        if let Ok(msg) = serde_json::from_str::<LiveMsg>(text) {
            let _ = tx.send(msg);
            wake();
        }
    });
    if stopped {
        let _ = ws.close(None);
        return Ok(());
    }
    Err((
        r.err()
            .unwrap_or_else(|| crate::tr!("connexion perdue", "connection lost").into()),
        false,
    ))
}

/// `pump`, also ending when `stop` is set.
fn pump_until(
    ws: &mut WebSocket<TcpStream>,
    rx: &Receiver<String>,
    stop: &AtomicBool,
    stopped: &mut bool,
    mut on: impl FnMut(&str),
) -> Result<(), String> {
    loop {
        if stop.load(Ordering::SeqCst) {
            *stopped = true;
            return Ok(());
        }
        while let Ok(text) = rx.try_recv() {
            ws.send(Message::text(text)).map_err(|e| e.to_string())?;
        }
        match ws.read() {
            Ok(Message::Text(t)) => on(t.as_str()),
            Ok(Message::Close(_)) => {
                return Err(crate::tr!(
                    "le serveur a fermé la connexion",
                    "the server closed the connection"
                )
                .into());
            }
            Ok(_) => {}
            Err(e) if idle(&e) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// What the server says of its space goes into the list: its name, its AI rule. The
/// invitation is spent.
fn remember(space_id: &str, info: &SpaceInfo) {
    let mut list = crate::space::load();
    if let Some(s) = list.spaces.iter_mut().find(|s| s.id == space_id) {
        if !info.name.trim().is_empty() {
            s.name = info.name.clone();
        }
        s.ai_rule = info.ai_rule;
        s.allowed_ai = info.allowed_ai.clone();
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
    fn invitation_links() {
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
            host_port("ws://machine/"),
            Some(format!("machine:{DEFAULT_PORT}"))
        );
        assert_eq!(
            host_port("ws://10.0.0.2:9000"),
            Some("10.0.0.2:9000".into())
        );
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
        let inv = d.invite().unwrap();
        // A signature for another challenge, or another space, is worth nothing.
        assert!(admit(&d, &space, "n2", &auth(&ana, &space, "n1", &inv)).is_err());
        assert!(admit(&d, &space, "n2", &auth(&ana, "autre", "n2", &inv)).is_err());
        assert_eq!(
            admit(&d, &space, "n3", &auth(&ana, &space, "n3", &inv)),
            Ok("Ana".into())
        );
        // Spent: Bob can't use it; Ana comes back without it.
        assert!(admit(&d, &space, "n4", &auth(&bob, &space, "n4", &inv)).is_err());
        assert_eq!(
            admit(&d, &space, "n5", &auth(&ana, &space, "n5", "")),
            Ok("Ana".into())
        );
        // Revoked.
        d.0.execute("UPDATE members SET revoked = 1", []).unwrap();
        assert!(admit(&d, &space, "n6", &auth(&ana, &space, "n6", "")).is_err());
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
}
