//! Live reading of the coding agents' own session files — no API, no network:
//!
//! - Claude Code: `~/.claude/projects/<folder>/<session-id>.jsonl` (or `$CLAUDE_CONFIG_DIR`)
//! - Codex:       `~/.codex/sessions/YYYY/MM/DD/rollout-…-<id>.jsonl` (or `$CODEX_HOME`)
//!
//! Each file is one JSON object per line, appended while the agent works. The `Scanner`
//! remembers how far it read each file and only parses what was added since, so it can
//! run every couple of seconds. Each session carries its working folder (`cwd`), which is
//! how it is matched to a project (`Workspace::project_for_path`).
//!
//! The formats aren't documented and change between versions, so every field is optional
//! and unknown lines are skipped: a new version can lose a detail, never break the app.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    ClaudeCode,
    Codex,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::ClaudeCode => "claude-code",
            Tool::Codex => "codex",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Tool::ClaudeCode => "Claude Code",
            Tool::Codex => "Codex",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tokens {
    /// Input + output + cache writes: what counts against limits.
    pub billed: u64,
    /// Cache reads (cheap, shown apart).
    pub cached: u64,
}

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub tool: Tool,
    pub id: String,
    pub cwd: String,
    /// The agent's own summary if it wrote one, else the first thing the user asked.
    pub title: String,
    /// The last thing the agent said (one line), to see where it stopped.
    pub last_reply: String,
    pub model: String,
    pub git_branch: String,
    pub started: u64,
    pub last_activity: u64,
    pub user_messages: u32,
    pub agent_messages: u32,
    pub tokens: Tokens,
    pub path: PathBuf,
}

impl SessionInfo {
    fn new(tool: Tool, path: &Path) -> Self {
        let id = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        // Codex files are named rollout-<date>-<uuid>: keep the uuid.
        let id = if tool == Tool::Codex && id.len() >= 36 { id[id.len() - 36..].to_string() } else { id };
        SessionInfo {
            tool,
            id,
            cwd: String::new(),
            title: String::new(),
            last_reply: String::new(),
            model: String::new(),
            git_branch: String::new(),
            started: 0,
            last_activity: 0,
            user_messages: 0,
            agent_messages: 0,
            tokens: Tokens::default(),
            path: path.to_path_buf(),
        }
    }

    /// Something was written in the last 2 minutes: the agent is probably at work.
    pub fn is_live(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_activity) < 120_000
    }

    /// The id comes from a file name and ends up in a shell command: only [A-Za-z0-9-],
    /// not starting with "-".
    pub fn has_safe_id(&self) -> bool {
        !self.id.is_empty() && !self.id.starts_with('-') && self.id.len() <= 128 && self.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }

    /// The agent command that reopens this session (without the cd), None for an unsafe id.
    pub fn resume_args(&self) -> Option<String> {
        if !self.has_safe_id() {
            return None;
        }
        Some(match self.tool {
            Tool::ClaudeCode => format!("claude --resume {}", self.id),
            Tool::Codex => format!("codex resume {}", self.id),
        })
    }

    /// The shell command that reopens this session where it stopped, None for an unsafe id.
    pub fn resume_command(&self) -> Option<String> {
        let cd = if self.cwd.is_empty() { String::new() } else { format!("cd {} && ", shell_quote(&self.cwd)) };
        self.resume_args().map(|a| format!("{cd}{a}"))
    }

    pub fn display_title(&self) -> String {
        if self.title.is_empty() { format!("Session {}", &self.id[..self.id.len().min(8)]) } else { self.title.clone() }
    }
}

pub fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-~".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Where each agent keeps its sessions on this machine.
pub fn default_roots() -> Vec<(Tool, PathBuf)> {
    let home = dirs::home_dir().unwrap_or_default();
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".claude"));
    let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
    vec![(Tool::ClaudeCode, claude.join("projects")), (Tool::Codex, codex.join("sessions"))]
}

struct FileState {
    offset: u64,
    /// The end of the last read when it stopped mid-line.
    carry: Vec<u8>,
    info: SessionInfo,
    /// Claude repeats an assistant message once per content block: count tokens once.
    seen_messages: HashSet<String>,
}

pub struct Scanner {
    roots: Vec<(Tool, PathBuf)>,
    files: HashMap<PathBuf, FileState>,
    /// Ignore sessions untouched for longer than this.
    pub max_age: Duration,
    /// Only read sessions whose folder is this one or inside it (skips the others cheaply).
    only: Option<PathBuf>,
}

impl Scanner {
    pub fn new(roots: Vec<(Tool, PathBuf)>) -> Self {
        Scanner { roots, files: HashMap::new(), max_age: Duration::from_secs(60 * 60 * 24 * 60), only: None }
    }

    /// Read what changed since the last call. Returns true when something did.
    pub fn scan(&mut self) -> bool {
        let mut changed = false;
        let mut present = HashSet::new();
        let oldest = SystemTime::now().checked_sub(self.max_age).unwrap_or(UNIX_EPOCH);
        for (tool, root) in self.roots.clone() {
            for (path, len, mtime) in list_session_files(tool, &root) {
                if mtime < oldest || !self.may_match(tool, &root, &path) {
                    continue;
                }
                present.insert(path.clone());
                changed |= self.read_file(tool, &path, len, mtime);
            }
        }
        let before = self.files.len();
        self.files.retain(|p, _| present.contains(p));
        changed || before != self.files.len()
    }

    fn read_file(&mut self, tool: Tool, path: &Path, len: u64, mtime: SystemTime) -> bool {
        let st = self.files.entry(path.to_path_buf()).or_insert_with(|| FileState {
            offset: 0,
            carry: vec![],
            info: SessionInfo::new(tool, path),
            seen_messages: HashSet::new(),
        });
        if len == st.offset {
            return false;
        }
        if len < st.offset {
            // Rewritten from scratch: start over.
            *st = FileState { offset: 0, carry: vec![], info: SessionInfo::new(tool, path), seen_messages: HashSet::new() };
        }
        let Ok(mut f) = File::open(path) else { return false };
        if f.seek(SeekFrom::Start(st.offset)).is_err() {
            return false;
        }
        let mut buf = std::mem::take(&mut st.carry);
        if f.take(len - st.offset).read_to_end(&mut buf).is_err() {
            return false;
        }
        st.offset = len;
        let complete = match buf.iter().rposition(|b| *b == b'\n') {
            Some(i) => {
                st.carry = buf[i + 1..].to_vec();
                &buf[..=i]
            }
            None => {
                st.carry = buf;
                return false;
            }
        };
        for line in complete.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_slice::<Value>(line) {
                match tool {
                    Tool::ClaudeCode => claude_line(&mut st.info, &mut st.seen_messages, &v),
                    Tool::Codex => codex_line(&mut st.info, &v),
                }
            }
        }
        let mt = mtime.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        if st.info.last_activity == 0 {
            st.info.last_activity = mt;
        }
        if st.info.started == 0 {
            st.info.started = st.info.last_activity;
        }
        true
    }

    /// With `only` set: Claude folder names encode the path (every non-alphanumeric
    /// character becomes `-`), so a name that doesn't start like the folder is skipped;
    /// a Codex file is skipped when its first line names another folder.
    fn may_match(&self, tool: Tool, root: &Path, path: &Path) -> bool {
        let Some(only) = &self.only else { return true };
        if self.files.contains_key(path) {
            return true;
        }
        match tool {
            Tool::ClaudeCode => {
                let enc: String = only.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
                let dir = path.parent().and_then(|p| p.strip_prefix(root).ok()).map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                dir.starts_with(&enc)
            }
            Tool::Codex => {
                let mut first = String::new();
                if let Ok(f) = File::open(path) {
                    let mut r = std::io::BufReader::new(f.take(256 * 1024));
                    let _ = std::io::BufRead::read_line(&mut r, &mut first);
                }
                match serde_json::from_str::<Value>(&first).ok().and_then(|v| str_at(&v, &["payload", "cwd"]).map(String::from)) {
                    Some(cwd) => Path::new(&cwd).starts_with(only),
                    None => true,
                }
            }
        }
    }

    /// Sessions that have a working folder and at least one message, most recent first.
    pub fn sessions(&self) -> Vec<SessionInfo> {
        let mut v: Vec<SessionInfo> = self
            .files
            .values()
            .map(|f| f.info.clone())
            .filter(|s| !s.cwd.is_empty() && s.user_messages + s.agent_messages > 0)
            .collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.last_activity));
        v
    }
}

/// `*.jsonl` files of one agent: Claude keeps one folder per project (sub-agent files
/// inside session folders are skipped); Codex sorts by date, a few levels deep.
fn list_session_files(tool: Tool, root: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut out = vec![];
    let depth = match tool {
        Tool::ClaudeCode => 1,
        Tool::Codex => 3,
    };
    walk(root, depth, &mut out);
    out
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<(PathBuf, u64, SystemTime)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        let p = e.path();
        if md.is_dir() {
            if depth > 0 {
                walk(&p, depth - 1, out);
            }
        } else if depth == 0 && p.extension().is_some_and(|x| x == "jsonl") {
            out.push((p, md.len(), md.modified().unwrap_or(UNIX_EPOCH)));
        }
    }
}

// ---------------------------------------------------------------- line formats

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for k in path {
        cur = cur.get(k)?;
    }
    cur.as_str()
}

fn u64_at(v: &Value, path: &[&str]) -> u64 {
    let mut cur = v;
    for k in path {
        match cur.get(k) {
            Some(x) => cur = x,
            None => return 0,
        }
    }
    cur.as_u64().unwrap_or(0)
}

/// Text of a message `content`: a string, or a list of `{type: text|input_text|output_text, text}`.
fn content_text(c: &Value) -> String {
    match c {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| matches!(p.get("type").and_then(Value::as_str), Some("text" | "input_text" | "output_text")))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Text the user really typed (not tool results, slash-command echoes or injected context).
fn is_human_text(t: &str) -> bool {
    let t = t.trim_start();
    !t.is_empty()
        && !t.starts_with('<')
        && !t.starts_with("Caveat:")
        && !t.starts_with("[Request interrupted")
        && !t.starts_with("This session is being continued")
}

fn short(t: &str, max: usize) -> String {
    crate::richtext::one_line(t, max)
}

fn touch(info: &mut SessionInfo, ts: Option<&str>) {
    if let Some(ms) = ts.and_then(parse_iso_ms) {
        if info.started == 0 || ms < info.started {
            info.started = ms;
        }
        info.last_activity = info.last_activity.max(ms);
    }
}

fn claude_line(info: &mut SessionInfo, seen: &mut HashSet<String>, v: &Value) {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
    if kind == "summary" {
        if let Some(s) = v.get("summary").and_then(Value::as_str) {
            info.title = short(s, 120);
        }
        return;
    }
    if !matches!(kind, "user" | "assistant") {
        return;
    }
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return;
    }
    touch(info, v.get("timestamp").and_then(Value::as_str));
    if let Some(c) = v.get("cwd").and_then(Value::as_str) {
        info.cwd = c.to_string();
    }
    if let Some(id) = v.get("sessionId").and_then(Value::as_str) {
        info.id = id.to_string();
    }
    if let Some(b) = v.get("gitBranch").and_then(Value::as_str).filter(|b| !b.is_empty()) {
        info.git_branch = b.to_string();
    }
    let msg = v.get("message").unwrap_or(&Value::Null);
    let text = content_text(msg.get("content").unwrap_or(&Value::Null));
    if kind == "user" {
        if v.get("isMeta").and_then(Value::as_bool) == Some(true) || !is_human_text(&text) {
            return;
        }
        info.user_messages += 1;
        if info.title.is_empty() {
            info.title = short(&text, 120);
        }
    } else {
        let mid = msg.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let first_time = mid.is_empty() || seen.insert(mid);
        if first_time {
            info.agent_messages += 1;
            let u = msg.get("usage").unwrap_or(&Value::Null);
            info.tokens.billed += u64_at(u, &["input_tokens"]) + u64_at(u, &["output_tokens"]) + u64_at(u, &["cache_creation_input_tokens"]);
            info.tokens.cached += u64_at(u, &["cache_read_input_tokens"]);
        }
        if let Some(m) = msg.get("model").and_then(Value::as_str).filter(|m| !m.starts_with('<')) {
            info.model = m.to_string();
        }
        if !text.trim().is_empty() {
            info.last_reply = short(&text, 200);
        }
    }
}

fn codex_line(info: &mut SessionInfo, v: &Value) {
    let ts = v.get("timestamp").and_then(Value::as_str);
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
    let p = v.get("payload").unwrap_or(v);
    match kind {
        "session_meta" => {
            if let Some(id) = str_at(p, &["id"]) {
                info.id = id.to_string();
            }
            if let Some(c) = str_at(p, &["cwd"]) {
                info.cwd = c.to_string();
            }
            if let Some(b) = str_at(p, &["git", "branch"]) {
                info.git_branch = b.to_string();
            }
            touch(info, str_at(p, &["timestamp"]).or(ts));
        }
        "turn_context" => {
            if let Some(c) = str_at(p, &["cwd"]) {
                info.cwd = c.to_string();
            }
            if let Some(m) = str_at(p, &["model"]) {
                info.model = m.to_string();
            }
        }
        "response_item" | "" => {
            // Older files have the item itself on the line, without a wrapper.
            if p.get("type").and_then(Value::as_str) != Some("message") {
                return;
            }
            touch(info, ts);
            let text = content_text(p.get("content").unwrap_or(&Value::Null));
            match p.get("role").and_then(Value::as_str) {
                Some("user") if is_human_text(&text) => {
                    info.user_messages += 1;
                    if info.title.is_empty() {
                        info.title = short(&text, 120);
                    }
                }
                Some("assistant") => {
                    info.agent_messages += 1;
                    if !text.trim().is_empty() {
                        info.last_reply = short(&text, 200);
                    }
                }
                _ => {}
            }
        }
        "event_msg" => {
            touch(info, ts);
            if str_at(p, &["type"]) == Some("token_count") {
                // Running totals for the whole session.
                let t = p.get("info").and_then(|i| i.get("total_token_usage")).unwrap_or(&Value::Null);
                let cached = u64_at(t, &["cached_input_tokens"]);
                let input = u64_at(t, &["input_tokens"]);
                let billed = input.saturating_sub(cached) + u64_at(t, &["output_tokens"]);
                if billed + cached > 0 {
                    info.tokens = Tokens { billed, cached };
                }
            }
        }
        _ => {}
    }
}

/// `2026-11-18T07:15:02.123Z` (or with an offset) → milliseconds since 1970.
pub fn parse_iso_ms(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[13] != b':' {
        return None;
    }
    let n = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    let mut rest = &s[19..];
    let mut ms = 0i64;
    if let Some(r) = rest.strip_prefix('.') {
        let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
        rest = &r[digits.len()..];
        let d3: String = digits.chars().chain("000".chars()).take(3).collect();
        ms = d3.parse().unwrap_or(0);
    }
    let offset_min = match rest.chars().next() {
        Some('+') | Some('-') if rest.len() >= 6 => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            sign * (rest[1..3].parse::<i64>().ok()? * 60 + rest[4..6].parse::<i64>().ok()?)
        }
        _ => 0,
    };
    // Days from civil (Howard Hinnant).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + h * 3600 + mi * 60 + se - offset_min * 60;
    (secs >= 0).then(|| (secs * 1000 + ms) as u64)
}

// ---------------------------------------------------------------- transcript

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Who {
    User,
    Agent,
    Tool,
}

/// One line of a conversation, for the side panel.
#[derive(Clone, Debug)]
pub struct Turn {
    pub who: Who,
    pub text: String,
    #[allow(dead_code)]
    pub at: u64,
}

/// The end of a session, as a conversation: what was asked, what the agent answered, the
/// tools it used (one line each). Reads only the last `tail_bytes` of the file.
pub fn transcript(tool: Tool, path: &Path, tail_bytes: u64, max: usize) -> Vec<Turn> {
    let Ok(mut f) = File::open(path) else { return vec![] };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(tail_bytes);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return vec![];
    }
    let mut buf = vec![];
    if f.read_to_end(&mut buf).is_err() {
        return vec![];
    }
    let mut lines = buf.split(|b| *b == b'\n');
    if start > 0 {
        lines.next(); // a cut line
    }
    let mut out: Vec<Turn> = vec![];
    let mut seen = HashSet::new();
    for line in lines {
        let Ok(v) = serde_json::from_slice::<Value>(line) else { continue };
        let at = v.get("timestamp").and_then(Value::as_str).and_then(parse_iso_ms).unwrap_or(0);
        let push = |out: &mut Vec<Turn>, who: Who, text: String| {
            let text = text.trim().to_string();
            if !text.is_empty() {
                out.push(Turn { who, text, at });
            }
        };
        match tool {
            Tool::ClaudeCode => {
                let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
                if v.get("isSidechain").and_then(Value::as_bool) == Some(true) || !matches!(kind, "user" | "assistant") {
                    continue;
                }
                let msg = v.get("message").unwrap_or(&Value::Null);
                if kind == "user" {
                    let t = content_text(msg.get("content").unwrap_or(&Value::Null));
                    if v.get("isMeta").and_then(Value::as_bool) != Some(true) && is_human_text(&t) {
                        push(&mut out, Who::User, t);
                    }
                    continue;
                }
                // Assistant: one line per content block (repeated lines share the message id).
                if let Some(id) = msg.get("id").and_then(Value::as_str) {
                    let key = format!("{id}:{}", serde_json::to_string(msg.get("content").unwrap_or(&Value::Null)).unwrap_or_default().len());
                    if !seen.insert(key) {
                        continue;
                    }
                }
                for part in msg.get("content").and_then(Value::as_array).cloned().unwrap_or_default() {
                    match part.get("type").and_then(Value::as_str) {
                        Some("text") => push(&mut out, Who::Agent, part.get("text").and_then(Value::as_str).unwrap_or("").to_string()),
                        Some("tool_use") => {
                            let name = part.get("name").and_then(Value::as_str).unwrap_or("outil");
                            let input = part.get("input").unwrap_or(&Value::Null);
                            let arg = ["file_path", "command", "pattern", "path", "url", "description"]
                                .iter()
                                .find_map(|k| input.get(k).and_then(Value::as_str))
                                .unwrap_or("");
                            push(&mut out, Who::Tool, format!("{name} {}", short(arg, 90)));
                        }
                        _ => {}
                    }
                }
            }
            Tool::Codex => {
                let p = v.get("payload").unwrap_or(&v);
                match (v.get("type").and_then(Value::as_str), p.get("type").and_then(Value::as_str)) {
                    (_, Some("message")) => {
                        let t = content_text(p.get("content").unwrap_or(&Value::Null));
                        match p.get("role").and_then(Value::as_str) {
                            Some("user") if is_human_text(&t) => push(&mut out, Who::User, t),
                            Some("assistant") => push(&mut out, Who::Agent, t),
                            _ => {}
                        }
                    }
                    (_, Some("function_call")) | (_, Some("local_shell_call")) => {
                        let name = p.get("name").and_then(Value::as_str).unwrap_or("shell");
                        let args = p.get("arguments").and_then(Value::as_str).unwrap_or("");
                        push(&mut out, Who::Tool, format!("{name} {}", short(args, 90)));
                    }
                    _ => {}
                }
            }
        }
    }
    let n = out.len();
    out.split_off(n.saturating_sub(max))
}

// ---------------------------------------------------------------- background watcher

/// Scans every 2 seconds in a background thread; the window reads the latest list.
#[derive(Clone)]
pub struct Watcher {
    latest: Arc<Mutex<Arc<Vec<SessionInfo>>>>,
}

impl Watcher {
    pub fn start(on_change: impl Fn() + Send + 'static) -> Self {
        let latest: Arc<Mutex<Arc<Vec<SessionInfo>>>> = Arc::default();
        let out = latest.clone();
        std::thread::Builder::new()
            .name("sessions".into())
            .spawn(move || {
                let mut sc = Scanner::new(default_roots());
                loop {
                    if sc.scan() {
                        *out.lock().unwrap() = Arc::new(sc.sessions());
                        on_change();
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            })
            .ok();
        Watcher { latest }
    }

    pub fn sessions(&self) -> Arc<Vec<SessionInfo>> {
        self.latest.lock().unwrap().clone()
    }
}

/// Sessions whose folder is `folder` or inside it (one-shot scan, for the MCP server).
pub fn sessions_in(folder: &Path) -> Vec<SessionInfo> {
    let f = crate::project::normalize_folder(&folder.to_string_lossy());
    let mut sc = Scanner::new(default_roots());
    sc.only = Some(f.clone());
    sc.scan();
    let found: Vec<SessionInfo> = sc.sessions().into_iter().filter(|s| Path::new(&s.cwd).starts_with(&f)).collect();
    if !found.is_empty() {
        return found;
    }
    // Nothing with the shortcut (a folder-name encoding we don't know?): read everything.
    let mut sc = Scanner::new(default_roots());
    sc.scan();
    sc.sessions().into_iter().filter(|s| Path::new(&s.cwd).starts_with(&f)).collect()
}

/// "12 k", "1,4 M"
pub fn human_tokens(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{} k", n / 1000),
        _ => format!("{:.1} M", n as f64 / 1e6).replace('.', ","),
    }
}

/// Open a terminal window running `cmd` (macOS Terminal, or a Linux terminal emulator).
pub fn open_in_terminal(cmd: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let esc = cmd.replace('\\', "\\\\").replace('"', "\\\"");
        std::process::Command::new("osascript")
            .args(["-e", &format!("tell application \"Terminal\" to do script \"{esc}\""), "-e", "tell application \"Terminal\" to activate"])
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd").args(["/C", "start", "powershell", "-NoExit", "-Command", cmd]).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let keep = format!("{cmd}; exec $SHELL");
        for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
            let r = if term == "gnome-terminal" {
                std::process::Command::new(term).args(["--", "bash", "-lc", &keep]).spawn()
            } else {
                std::process::Command::new(term).args(["-e", "bash", "-lc", &keep]).spawn()
            };
            if r.is_ok() {
                return Ok(());
            }
        }
        Err(crate::tr!("aucun terminal trouvé", "no terminal found").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tia-sessions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_ms("2026-11-18T07:15:02.123Z"), Some(1_794_986_102_123));
        assert_eq!(parse_iso_ms("2026-11-18T08:15:02.123+01:00"), Some(1_794_986_102_123));
        assert_eq!(parse_iso_ms("nope"), None);
    }

    #[test]
    fn claude_session_read_incrementally() {
        let root = tmp("claude");
        let dir = root.join("-home-me-Code-app");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("abc-123.jsonl");
        let mut f = File::create(&file).unwrap();
        writeln!(f, r#"{{"type":"user","sessionId":"abc-123","cwd":"/home/me/Code/app","timestamp":"2026-11-18T07:00:00Z","message":{{"role":"user","content":"<command-name>/clear</command-name>"}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"user","sessionId":"abc-123","cwd":"/home/me/Code/app","gitBranch":"main","timestamp":"2026-11-18T07:00:01Z","message":{{"role":"user","content":"Ajoute la **vue projets**"}}}}"#).unwrap();
        // The same assistant message twice (one line per content block): tokens counted once.
        for _ in 0..2 {
            writeln!(f, r#"{{"type":"assistant","sessionId":"abc-123","cwd":"/home/me/Code/app","timestamp":"2026-11-18T07:00:05Z","message":{{"id":"m1","model":"claude-opus-5-5","role":"assistant","content":[{{"type":"text","text":"C'est fait."}}],"usage":{{"input_tokens":10,"output_tokens":20,"cache_creation_input_tokens":5,"cache_read_input_tokens":1000}}}}}}"#).unwrap();
        }
        writeln!(f, r#"{{"type":"user","timestamp":"2026-11-18T07:00:06Z","message":{{"role":"user","content":[{{"type":"tool_result","content":"ok"}}]}}}}"#).unwrap();
        write!(f, r#"{{"type":"assistant","timestamp":"2026-11-18T07:00:09Z","message":{{"id":"m2""#).unwrap(); // half a line
        drop(f);

        let mut sc = Scanner::new(vec![(Tool::ClaudeCode, root.clone())]);
        assert!(sc.scan());
        let s = &sc.sessions()[0];
        assert_eq!(s.id, "abc-123");
        assert_eq!(s.cwd, "/home/me/Code/app");
        assert_eq!(s.title, "Ajoute la vue projets");
        assert_eq!(s.git_branch, "main");
        assert_eq!(s.user_messages, 1);
        assert_eq!(s.agent_messages, 1);
        assert_eq!(s.tokens, Tokens { billed: 35, cached: 1000 });
        assert_eq!(s.model, "claude-opus-5-5");
        assert_eq!(s.last_reply, "C'est fait.");
        assert_eq!(s.resume_command().as_deref(), Some("cd /home/me/Code/app && claude --resume abc-123"));
        // An id with shell characters or an option is never put in a command.
        for bad in ["abc;rm -rf ~", "$(x)", "a b", "-x", "a'b", "a/b", ""] {
            let mut t = s.clone();
            t.id = bad.into();
            assert!(t.resume_command().is_none() && t.resume_args().is_none(), "{bad}");
        }

        // Nothing new: nothing to do. Then the half line is finished: it is read.
        assert!(!sc.scan());
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, r#","role":"assistant","content":[{{"type":"text","text":"Terminé"}}],"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"summary","summary":"Vue projets du tableau de bord"}}"#).unwrap();
        drop(f);
        assert!(sc.scan());
        let s = &sc.sessions()[0];
        assert_eq!(s.agent_messages, 2);
        assert_eq!(s.last_reply, "Terminé");
        assert_eq!(s.title, "Vue projets du tableau de bord");
        assert_eq!(s.last_activity, parse_iso_ms("2026-11-18T07:00:09Z").unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_session() {
        let root = tmp("codex");
        let dir = root.join("2026/11/18");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rollout-2026-11-18T07-00-00-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b.jsonl");
        let mut f = File::create(&file).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-11-18T07:00:00Z","type":"session_meta","payload":{{"id":"0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b","cwd":"/home/me/Code/app","git":{{"branch":"dev"}}}}}}"#).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-11-18T07:00:01Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"<environment_context>x</environment_context>"}}]}}}}"#).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-11-18T07:00:02Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"Corrige les tests"}}]}}}}"#).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-11-18T07:00:03Z","type":"turn_context","payload":{{"cwd":"/home/me/Code/app","model":"gpt-5-codex"}}}}"#).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-11-18T07:00:09Z","type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"Tests corrigés."}}]}}}}"#).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-11-18T07:00:10Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":5000,"cached_input_tokens":4000,"output_tokens":300}}}}}}}}"#).unwrap();
        drop(f);
        let mut sc = Scanner::new(vec![(Tool::Codex, root.clone())]);
        sc.scan();
        let s = &sc.sessions()[0];
        assert_eq!(s.id, "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b");
        assert_eq!(s.title, "Corrige les tests");
        assert_eq!(s.git_branch, "dev");
        assert_eq!(s.model, "gpt-5-codex");
        assert_eq!((s.user_messages, s.agent_messages), (1, 1));
        assert_eq!(s.tokens, Tokens { billed: 1300, cached: 4000 });
        assert_eq!(s.resume_command().as_deref(), Some("cd /home/me/Code/app && codex resume 0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn folder_filter_skips_other_projects() {
        let root = tmp("filter");
        for (dir, cwd) in [("-home-me-Code-app", "/home/me/Code/app"), ("-home-me-Code-other", "/home/me/Code/other")] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            let line = format!(r#"{{"type":"user","sessionId":"{dir}","cwd":"{cwd}","timestamp":"2026-11-18T07:00:00Z","message":{{"content":"hi"}}}}"#);
            std::fs::write(root.join(dir).join("s.jsonl"), line + "\n").unwrap();
        }
        let mut sc = Scanner::new(vec![(Tool::ClaudeCode, root.clone())]);
        sc.only = Some(PathBuf::from("/home/me/Code/app"));
        sc.scan();
        assert_eq!(sc.files.len(), 1);
        assert_eq!(sc.sessions()[0].cwd, "/home/me/Code/app");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn transcript_reads_the_end() {
        let root = tmp("transcript");
        let f = root.join("s.jsonl");
        let mut w = File::create(&f).unwrap();
        writeln!(w, r#"{{"type":"user","timestamp":"2026-11-18T07:00:00Z","message":{{"content":"Corrige le bug"}}}}"#).unwrap();
        writeln!(w, r#"{{"type":"assistant","message":{{"id":"m1","content":[{{"type":"text","text":"Je regarde."}},{{"type":"tool_use","name":"Read","input":{{"file_path":"src/a.rs"}}}}]}}}}"#).unwrap();
        writeln!(w, r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","content":"..."}}]}}}}"#).unwrap();
        writeln!(w, r#"{{"type":"assistant","message":{{"id":"m2","content":[{{"type":"text","text":"Corrigé."}}]}}}}"#).unwrap();
        drop(w);
        let t = transcript(Tool::ClaudeCode, &f, 1 << 20, 50);
        let shown: Vec<(Who, &str)> = t.iter().map(|x| (x.who, x.text.as_str())).collect();
        assert_eq!(shown, vec![(Who::User, "Corrige le bug"), (Who::Agent, "Je regarde."), (Who::Tool, "Read src/a.rs"), (Who::Agent, "Corrigé.")]);
        assert_eq!(transcript(Tool::ClaudeCode, &f, 1 << 20, 2).len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn quoting_and_numbers() {
        assert_eq!(shell_quote("/a/b c/d'e"), "'/a/b c/d'\\''e'");
        assert_eq!(human_tokens(950), "950");
        assert_eq!(human_tokens(12_400), "12 k");
        assert_eq!(human_tokens(1_400_000), "1,4 M");
    }
}
