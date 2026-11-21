//! How much of the Claude subscription is used: the 5-hour session window and the weekly
//! windows, as Claude Code itself reports them. No token is read and no request is made:
//!
//! - Claude Code hands every status line command a JSON with `rate_limits` (five_hour,
//!   seven_day, each with `used_percentage` and `resets_at`). `orchestra statusline` is such a
//!   command: it keeps the figures in the data folder, then shows the person's own status line
//!   (kept aside when ours was installed) or a short one of its own.
//! - Claude Code also caches the figures in `~/.claude.json` (`cachedUsageUtilization`),
//!   read as a fallback, whatever its exact shape.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub label: String,
    /// 0 to 100.
    pub pct: f32,
    /// When the window starts again (ms since epoch).
    pub resets_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Usage {
    pub windows: Vec<Window>,
    /// When Claude Code last reported it (ms).
    pub at: u64,
}

fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"))
}

pub(crate) fn claude_json() -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) => PathBuf::from(d).join(".claude.json"),
        None => dirs::home_dir().unwrap_or_default().join(".claude.json"),
    }
}

fn captured_file() -> PathBuf {
    crate::store::data_dir().join("claude-usage.json")
}

fn previous_file() -> PathBuf {
    crate::store::data_dir().join("statusline-precedente.txt")
}

/// A window's label in the interface's language (labels are kept in French inside).
pub fn shown(label: &str) -> String {
    if !crate::i18n::en() {
        return label.to_string();
    }
    label
        .replace("Semaine", "Week")
        .replace("Dépenses", "Spending")
}

fn label_for(key: &str) -> String {
    match key {
        "five_hour" => "Session (5 h)".into(),
        "seven_day" => "Semaine".into(),
        "seven_day_opus" => "Semaine (Opus)".into(),
        "seven_day_sonnet" => "Semaine (Sonnet)".into(),
        "spend_limit" => "Dépenses".into(),
        k => k.replace('_', " "),
    }
}

/// Seconds or milliseconds since epoch, or an ISO date, into ms.
fn time_ms(v: &Value) -> Option<u64> {
    if let Some(n) = v.as_f64() {
        return Some(if n > 1e12 {
            n as u64
        } else {
            (n * 1000.0) as u64
        });
    }
    v.as_str().and_then(crate::sessions::parse_iso_ms)
}

/// The status line's `rate_limits` object.
fn from_rate_limits(v: &Value) -> Vec<Window> {
    let Some(o) = v.as_object() else {
        return vec![];
    };
    let mut out: Vec<Window> = o
        .iter()
        .filter_map(|(k, w)| {
            let pct = w.get("used_percentage")?.as_f64()? as f32;
            Some(Window {
                label: label_for(k),
                pct: pct.clamp(0.0, 100.0),
                resets_at: w.get("resets_at").and_then(time_ms),
            })
        })
        .collect();
    out.sort_by_key(|w| order(&w.label));
    out
}

fn order(label: &str) -> u8 {
    if label.starts_with("Session") {
        0
    } else if label == "Semaine" {
        1
    } else {
        2
    }
}

/// Anything under `cachedUsageUtilization` that has a `utilization`: named by its key, or by
/// its `display_name` / `name` when it sits in a list.
fn from_cache(v: &Value) -> Vec<Window> {
    fn walk(v: &Value, key: &str, out: &mut Vec<(String, f64, Option<u64>)>) {
        match v {
            Value::Object(o) => {
                if let Some(u) = o.get("utilization").and_then(Value::as_f64) {
                    let name = o
                        .get("display_name")
                        .or_else(|| o.get("name"))
                        .and_then(Value::as_str)
                        .map(|n| format!("Semaine ({n})"))
                        .unwrap_or_else(|| label_for(key));
                    out.push((name, u, o.get("resets_at").and_then(time_ms)));
                    return;
                }
                for (k, x) in o {
                    walk(x, k, out);
                }
            }
            Value::Array(a) => {
                for x in a {
                    walk(x, key, out);
                }
            }
            _ => {}
        }
    }
    let mut found = vec![];
    walk(v, "", &mut found);
    // Fractions (0..1) or percentages, all the same way.
    let fraction = found.iter().all(|(_, u, _)| *u <= 1.0);
    let mut out: Vec<Window> = vec![];
    for (label, u, r) in found {
        if label.is_empty() || out.iter().any(|w| w.label == label) {
            continue;
        }
        let pct = if fraction { u * 100.0 } else { u } as f32;
        out.push(Window {
            label,
            pct: pct.clamp(0.0, 100.0),
            resets_at: r,
        });
    }
    out.sort_by_key(|w| order(&w.label));
    out
}

fn mtime_ms(p: &std::path::Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The latest figures, from whichever source is freshest. Windows that already started
/// again are dropped (their figure is stale).
pub fn read() -> Option<Usage> {
    let now = crate::model::now_ms();
    let mut best: Option<Usage> = None;
    if let Some(v) = std::fs::read_to_string(captured_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        let w = from_rate_limits(&v["rate_limits"]);
        if !w.is_empty() {
            best = Some(Usage {
                windows: w,
                at: v["at"].as_u64().unwrap_or(0),
            });
        }
    }
    let cj = claude_json();
    if let Some(v) = std::fs::read_to_string(&cj)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        let c = &v["cachedUsageUtilization"];
        let w = from_cache(c);
        if !w.is_empty() {
            let stamped = ["fetchedAt", "timestamp", "updatedAt", "lastUpdated"]
                .iter()
                .find_map(|k| c.get(*k).and_then(time_ms));
            // Without its own time, the cache only stands in when nothing else is there:
            // ~/.claude.json is rewritten all the time for other things, so its date would
            // pass old figures for newer than the status line's, and switching between the
            // two would count rises that never happened (and use up a session's budget).
            let newer = match stamped {
                Some(at) => best.as_ref().is_none_or(|b| at > b.at),
                None => best.is_none(),
            };
            if newer {
                let at = stamped.unwrap_or_else(|| mtime_ms(&cj));
                best = Some(Usage { windows: w, at });
            }
        }
    }
    let mut u = best?;
    u.windows.retain(|w| w.resets_at.is_none_or(|r| r > now));
    (!u.windows.is_empty()).then_some(u)
}

// ------------------------------------------------------------------ the other agents
//
// What each agent tells about its limits, on this machine, without a request:
//
// - Codex (ChatGPT subscription): every `token_count` event of its session files carries
//   `rate_limits`, a `primary` window (5 h) and a `secondary` one (week), each with
//   `used_percent`, `window_minutes` and `resets_at` (seconds since epoch; older versions
//   wrote `resets_in_seconds`). Read here, the same windows as Claude Code's.
// - Gemini CLI: its free or paid quota is a number of requests per day, counted by Google
//   and never written locally (`/stats` only shows the tokens of the session). Nothing to read.
// - Mistral Vibe: billed by use on an API key, with no subscription window; its session files
//   hold tokens and cost, not a limit. Nothing to read.

/// The agents whose use of the subscription Orchestra can read (the ceiling of a session
/// rests on it).
pub fn reports_usage(agent: crate::auto::Agent) -> bool {
    use crate::auto::Agent;
    matches!(agent, Agent::ClaudeCode | Agent::Codex)
}

/// Why an agent's quota can't be shown, in a few words (None when Orchestra reads it).
pub fn unreadable_reason(agent: crate::auto::Agent) -> Option<&'static str> {
    use crate::auto::Agent;
    match agent {
        Agent::ClaudeCode | Agent::Codex => None,
        Agent::Gemini => Some(crate::tr!(
            "Quota compté par Google en requêtes par jour, jamais écrit sur cet ordinateur.",
            "Quota counted by Google in requests per day, never written on this computer."
        )),
        Agent::Vibe => Some(crate::tr!(
            "Facturé à l'usage sur une clé d'API Mistral : pas de fenêtre d'abonnement à suivre.",
            "Billed by use on a Mistral API key: no subscription window to follow."
        )),
    }
}

/// The latest figures of an agent; None for those that tell none (see above).
pub fn read_for(agent: crate::auto::Agent) -> Option<Usage> {
    use crate::auto::Agent;
    match agent {
        Agent::ClaudeCode => read(),
        Agent::Codex => read_codex(),
        Agent::Gemini | Agent::Vibe => None,
    }
}

/// Codex's figures, from its most recent session files (whatever their folder: the limits are
/// those of the account).
fn read_codex() -> Option<Usage> {
    let now = crate::model::now_ms();
    // A week back: the weekly window is still meaningful, older files are not.
    let since = std::time::SystemTime::now() - std::time::Duration::from_secs(7 * 24 * 3600);
    let dir = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex"))
        .join("sessions");
    for f in crate::auto::session_files(&dir, 4, "jsonl", since).into_iter().take(5) {
        let Ok(mut file) = std::fs::File::open(&f) else { continue };
        // The last events are enough: the end of the file only (records may be long).
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let start = len.saturating_sub(512 * 1024);
        let mut buf = Vec::new();
        use std::io::Seek;
        if file.seek(std::io::SeekFrom::Start(start)).is_err()
            || file.take(len - start).read_to_end(&mut buf).is_err()
        {
            continue;
        }
        let text = String::from_utf8_lossy(&buf);
        if let Some(mut u) = codex_usage(&text, mtime_ms(&f)) {
            u.windows.retain(|w| w.resets_at.is_none_or(|r| r > now));
            if !u.windows.is_empty() {
                return Some(u);
            }
        }
    }
    None
}

/// The last `rate_limits` of a Codex session file's text. `file_ms`: when the file last
/// changed, for records without their own time.
fn codex_usage(text: &str, file_ms: u64) -> Option<Usage> {
    for line in text.lines().rev() {
        if !line.contains("rate_limits") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let rl = &v["payload"]["rate_limits"];
        if !rl.is_object() {
            continue;
        }
        let at = v.get("timestamp").and_then(time_ms).unwrap_or(file_ms);
        let mut windows: Vec<Window> = ["primary", "secondary"]
            .iter()
            .filter_map(|k| {
                let w = &rl[*k];
                let pct = w["used_percent"].as_f64()? as f32;
                let minutes = w["window_minutes"].as_u64();
                let label = match (minutes, *k) {
                    (Some(300), _) | (None, "primary") => label_for("five_hour"),
                    (Some(10080), _) | (None, _) => label_for("seven_day"),
                    (Some(m), _) if m % 1440 == 0 => format!("{} j", m / 1440),
                    (Some(m), _) if m % 60 == 0 => format!("{} h", m / 60),
                    (Some(m), _) => format!("{m} min"),
                };
                let resets_at = w.get("resets_at").and_then(time_ms).or_else(|| {
                    w["resets_in_seconds"].as_u64().map(|s| at + s * 1000)
                });
                Some(Window { label, pct: pct.clamp(0.0, 100.0), resets_at })
            })
            .collect();
        if windows.is_empty() {
            continue;
        }
        windows.sort_by_key(|w| order(&w.label));
        return Some(Usage { windows, at });
    }
    None
}

impl Usage {
    /// The 5-hour window.
    pub fn five(&self) -> Option<&Window> {
        self.windows.iter().find(|w| w.label.starts_with("Session"))
    }

    /// The weekly window (all models).
    pub fn week(&self) -> Option<&Window> {
        self.windows.iter().find(|w| w.label == "Semaine")
    }
}

/// Time left before `resets_at` (ms); None once it has passed.
pub fn until(resets_at: u64, now: u64) -> Option<u64> {
    (resets_at > now).then(|| resets_at - now)
}

/// "2 h 14", "3 j 5 h", "12 min", "< 1 min": a time left, short.
pub fn left_text(ms: u64) -> String {
    let min = ms / 60_000;
    let (d, h, m) = (min / (24 * 60), min / 60 % 24, min % 60);
    if d > 0 {
        if crate::i18n::en() {
            format!("{d} d {h} h")
        } else {
            format!("{d} j {h} h")
        }
    } else if h > 0 {
        format!("{h} h {m:02}")
    } else if m > 0 {
        format!("{m} min")
    } else {
        "< 1 min".into()
    }
}

// ---------------------------------------------------------------- last 24 hours
//
// Claude Code only gives the current 5-hour window and the week. Orchestra keeps the
// readings of the last day, to add up the 5-hour windows of the last 24 hours.

const HOUR: u64 = 3_600_000;
const DAY: u64 = 24 * HOUR;
/// Two `resets_at` this close are the same window (Claude Code rounds them its own way).
const SAME_RESET: u64 = 120_000;

/// One reading of the 5-hour and weekly windows.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Sample {
    pub at: u64,
    pub five: Option<f32>,
    pub five_reset: Option<u64>,
    pub week: Option<f32>,
    pub week_reset: Option<u64>,
}

fn history_file() -> PathBuf {
    crate::store::data_dir().join("claude-usage-history.json")
}

impl Usage {
    pub fn sample(&self) -> Sample {
        Sample {
            at: self.at,
            five: self.five().map(|w| w.pct),
            five_reset: self.five().and_then(|w| w.resets_at),
            week: self.week().map(|w| w.pct),
            week_reset: self.week().and_then(|w| w.resets_at),
        }
    }
}

fn near(a: Option<u64>, b: Option<u64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.abs_diff(b) < SAME_RESET,
        (None, None) => true,
        _ => false,
    }
}

/// Add a reading when it says something new, and forget what is too old to matter (more
/// than 30 hours, keeping one earlier reading as a starting point). True when `h` changed.
pub fn push_sample(h: &mut Vec<Sample>, s: Sample) -> bool {
    if s.five.is_none() && s.week.is_none() {
        return false;
    }
    if let Some(l) = h.last()
        && (s.at <= l.at
            || (l.five == s.five
                && l.week == s.week
                && near(l.five_reset, s.five_reset)
                && near(l.week_reset, s.week_reset)))
    {
        return false;
    }
    let cut = s.at.saturating_sub(30 * HOUR);
    h.push(s);
    let first_kept = h.iter().position(|x| x.at >= cut).unwrap_or(h.len());
    if first_kept > 1 {
        h.drain(..first_kept - 1);
    }
    true
}

/// The readings kept so far, oldest first.
pub fn history() -> Vec<Sample> {
    std::fs::read_to_string(history_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Keep this reading in the history; returns the history with it.
pub fn record(u: &Usage) -> Vec<Sample> {
    let mut h = history();
    if push_sample(&mut h, u.sample()) {
        let _ = std::fs::create_dir_all(crate::store::data_dir());
        // Several Claude Code sessions run this at once: each through a temporary file of its own.
        let _ = crate::store::write_atomic(
            &history_file(),
            serde_json::to_string(&h).unwrap_or_default().as_bytes(),
        );
    }
    h
}

/// The 5-hour windows of the last 24 hours, added up (each one counted at its highest
/// reading). A window is counted when it was still running at some point of the last 24
/// hours. None without any reading of the 5-hour window.
pub fn day_total(h: &[Sample], now: u64) -> Option<f32> {
    let from = now.saturating_sub(DAY);
    // The window being read: highest figure, last figure, its reset, last reading.
    struct Win {
        max: f32,
        last: f32,
        reset: Option<u64>,
        at: u64,
    }
    let counts = |w: &Win| w.at >= from || w.reset.is_some_and(|r| r > from);
    let mut total = 0.0;
    let mut cur: Option<Win> = None;
    for s in h.iter().filter(|s| s.at <= now) {
        let Some(p) = s.five else { continue };
        let same = cur.as_ref().is_some_and(|w| match (w.reset, s.five_reset) {
            (Some(a), Some(b)) => a.abs_diff(b) < SAME_RESET,
            // Without a reset time: same window while the figure doesn't drop and less than 5 h went by.
            _ => p + 0.5 >= w.last && s.at - w.at < 5 * HOUR,
        });
        match cur.as_mut() {
            Some(w) if same => {
                w.max = w.max.max(p);
                w.last = p;
                w.at = s.at;
                w.reset = w.reset.or(s.five_reset);
            }
            _ => {
                if let Some(w) = cur.take().filter(counts) {
                    total += w.max;
                }
                cur = Some(Win {
                    max: p,
                    last: p,
                    reset: s.five_reset,
                    at: s.at,
                });
            }
        }
    }
    let w = cur?;
    if counts(&w) {
        total += w.max;
    }
    Some(total)
}

/// Without reset times, a drop of the weekly figure bigger than this (points) is a new week.
const WEEK_DROP: f32 = 5.0;

/// How much of the week went in the last 24 hours (percentage points of the week), and
/// over how long the readings go back (24 h at most). None without any weekly reading.
pub fn day_week_used(h: &[Sample], now: u64) -> Option<(f32, u64)> {
    let from = now.saturating_sub(DAY);
    let readings: Vec<&Sample> = h
        .iter()
        .filter(|s| s.week.is_some() && s.at <= now)
        .collect();
    // Starting point: the last reading before the 24 hours, or else the first one.
    let start = readings.iter().rposition(|s| s.at <= from).unwrap_or(0);
    let first = readings.get(start)?;
    // `high`: the highest figure of the week being read. Only what goes past it counts: the
    // two sources (status line, ~/.claude.json) take turns and one may be a little late,
    // so the figure dips and climbs back; counting each climb counted the same use twice.
    let (mut high, mut reset) = (first.week?, first.week_reset);
    let mut used = 0.0;
    for s in &readings[start + 1..] {
        let p = s.week?;
        let same = match (reset, s.week_reset) {
            (Some(a), Some(b)) => a.abs_diff(b) < SAME_RESET,
            // A reset time missing on one side: a new week drops the figure to a few
            // points; a dip of a point or two is a late reading of the same week.
            _ => p + WEEK_DROP >= high,
        };
        if same {
            used += (p - high).max(0.0);
            high = high.max(p);
        } else {
            // A new week: its figure so far is all new use.
            used += p;
            high = p;
        }
        reset = s.week_reset.or(if same { reset } else { None });
    }
    Some((used, now - first.at.max(from)))
}

/// The day's share of the subscription: what went from the week in the last 24 hours,
/// against a seventh of the week (100 % = on pace to use exactly the whole week).
pub fn daily_pct(week_used: f32) -> f32 {
    week_used * 7.0
}

/// One day's share of the week (in points of the week): a seventh.
pub const DAY_SHARE: f32 = 100.0 / 7.0;

/// The line under the week's bar: the points of the week used over the last 24 hours (or
/// since the first reading, `span`), next to a day's share. Before, it read « 105 % de la
/// part du jour », a percentage of a percentage hard to read; the figures themselves are
/// shown now. The flag: faster than a day's share, at which pace the week runs out early
/// (over a shorter span, compared with that span's share of a day).
pub fn day_pace_line(used: f32, span: u64) -> (String, bool) {
    let full = span >= 23 * HOUR;
    let share = if full {
        DAY_SHARE
    } else {
        DAY_SHARE * span as f32 / DAY as f32
    };
    let fast = used > share + 0.05;
    let when = if full {
        crate::tr!("Ces 24 h", "These 24 h").to_string()
    } else {
        crate::trf!("Depuis {}", "Last {}", left_text(span))
    };
    let pace = if fast {
        crate::tr!(
            " · plus vite que la part d'un jour",
            " · faster than a day's share"
        )
    } else {
        ""
    };
    let (used, day) = (crate::i18n::dec1(used), crate::i18n::dec1(DAY_SHARE));
    (
        crate::trf!(
            "{when} : {used} % de la semaine utilisés (part d'un jour : {day} %){pace}",
            "{when}: {used} % of the week used (a day's share: {day} %){pace}"
        ),
        fast,
    )
}

/// The reserve is near: one more turn, as big as the biggest so far (at least half a
/// point of the week), would reach `limit` (100 − reserve).
pub fn near_reserve(week: f32, limit: f32, turn: f32) -> bool {
    week + turn.max(0.5) >= limit
}

// ---------------------------------------------------------------- status line

fn settings_file() -> PathBuf {
    claude_dir().join("settings.json")
}

/// The status line command: this program with `statusline`, and the data folder when it isn't
/// the default one (else the figures would be kept where this Orchestra never reads them).
fn our_command(exe: &std::path::Path) -> String {
    let env = std::env::var("ORCHESTRA_DIR")
        .ok()
        .filter(|d| !d.trim().is_empty())
        .map(|d| format!("ORCHESTRA_DIR={} ", crate::sessions::shell_quote(&d)))
        .unwrap_or_default();
    format!(
        "{env}{} statusline",
        crate::sessions::shell_quote(&exe.display().to_string())
    )
}

/// The words of a command as `shell_quote` writes them: plain words, or quoted with `'…'`
/// (a quote inside written `'\''`).
fn command_words(cmd: &str) -> Vec<String> {
    let mut words = vec![];
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                quoted = !quoted;
                any = true;
            }
            '\\' if !quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// What a status line command of ours runs: its program and its data folder (None: the
/// default one). None when it isn't ours.
fn command_target(cmd: &str) -> Option<(PathBuf, Option<String>)> {
    let mut w = command_words(cmd);
    if w.last().map(String::as_str) != Some("statusline") {
        return None;
    }
    w.pop();
    let dir = match w.first().and_then(|x| x.strip_prefix("ORCHESTRA_DIR=")) {
        Some(d) => {
            let d = d.to_string();
            w.remove(0);
            Some(d)
        }
        None => None,
    };
    (w.len() == 1).then(|| (PathBuf::from(&w[0]), dir))
}

/// Our status line is in place AND works: its program is still there (an Orchestra moved,
/// updated or built in a copy since deleted leaves a command that fails, and then no figure
/// ever comes: the sessions stop as unmeasured), and it keeps the figures in this Orchestra's
/// data folder.
pub fn tracking_works() -> bool {
    let here = std::env::var("ORCHESTRA_DIR").ok().filter(|d| !d.trim().is_empty());
    command_target(&status_command()).is_some_and(|(exe, dir)| exe.is_file() && dir == here)
}

/// Our status line is in place but its program is gone: nothing is measured any more. Put
/// right by itself (not when it is another Orchestra's that works: they would take turns).
pub fn tracking_broken() -> bool {
    command_target(&status_command()).is_some_and(|(exe, _)| !exe.is_file())
}

/// Claude Code's status line command, as its settings hold it.
fn status_command() -> String {
    std::fs::read_to_string(settings_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["statusLine"]["command"].as_str().map(String::from))
        .unwrap_or_default()
}

/// Our status line is the one Claude Code runs.
pub fn tracking_on() -> bool {
    std::fs::read_to_string(settings_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| {
            v["statusLine"]["command"]
                .as_str()
                .map(|c| c.ends_with(" statusline"))
        })
        .unwrap_or(false)
}

/// Make `orchestra statusline` Claude Code's status line. The previous one keeps running
/// (ours calls it), and comes back with `stop_tracking`.
pub fn start_tracking(exe: &std::path::Path) -> Result<(), String> {
    let f = settings_file();
    let text = std::fs::read_to_string(&f).unwrap_or_default();
    let mut v: Value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text)
            .map_err(|e| format!("{} illisible ({e}), laissé tel quel", f.display()))?
    };
    let obj = v.as_object_mut().ok_or("format inattendu")?;
    let mine = our_command(exe);
    if let Some(prev) = obj.get("statusLine").and_then(|s| s["command"].as_str())
        && !prev.ends_with(" statusline")
    {
        std::fs::create_dir_all(crate::store::data_dir()).map_err(|e| e.to_string())?;
        std::fs::write(previous_file(), prev).map_err(|e| e.to_string())?;
    }
    obj.insert(
        "statusLine".into(),
        json!({ "type": "command", "command": mine, "padding": 0 }),
    );
    if !text.is_empty() {
        let bak = f.with_extension("json.bak");
        if !bak.exists() {
            let _ = std::fs::write(&bak, &text);
        }
    }
    std::fs::create_dir_all(claude_dir()).map_err(|e| e.to_string())?;
    std::fs::write(
        &f,
        serde_json::to_string_pretty(&v).unwrap_or_default() + "\n",
    )
    .map_err(|e| e.to_string())
}

/// Put the person's own status line back (or none). Only while Claude Code's status line is
/// still ours: one the person set since then stays.
pub fn stop_tracking() -> Result<(), String> {
    let f = settings_file();
    let Ok(text) = std::fs::read_to_string(&f) else {
        return Ok(());
    };
    let mut v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let prev = std::fs::read_to_string(previous_file()).ok();
    if !restore_status_line(&mut v, prev.as_deref())? {
        return Ok(());
    }
    let _ = std::fs::remove_file(previous_file());
    std::fs::write(
        &f,
        serde_json::to_string_pretty(&v).unwrap_or_default() + "\n",
    )
    .map_err(|e| e.to_string())
}

/// In Claude Code's settings `v`: ours replaced by the person's previous status line, or
/// removed when there was none. False (and `v` untouched) when the status line isn't ours.
fn restore_status_line(v: &mut Value, prev: Option<&str>) -> Result<bool, String> {
    if !v["statusLine"]["command"]
        .as_str()
        .is_some_and(|c| c.ends_with(" statusline"))
    {
        return Ok(false);
    }
    let obj = v.as_object_mut().ok_or("format inattendu")?;
    match prev.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => {
            obj.insert(
                "statusLine".into(),
                json!({ "type": "command", "command": p }),
            );
        }
        None => {
            obj.remove("statusLine");
        }
    }
    Ok(true)
}

#[test]
fn a_status_line_of_ours_is_read_back() {
    // As written by our_command: quoted program, optional data folder.
    assert_eq!(
        command_target("'/Applications/Orchestra.app/Contents/MacOS/orchestra' statusline"),
        Some((PathBuf::from("/Applications/Orchestra.app/Contents/MacOS/orchestra"), None))
    );
    assert_eq!(
        command_target("ORCHESTRA_DIR='/tmp/my data' /usr/local/bin/orchestra statusline"),
        Some((PathBuf::from("/usr/local/bin/orchestra"), Some("/tmp/my data".into())))
    );
    // A quote inside a quoted path.
    let exe = PathBuf::from("/Users/a/l'app/orchestra");
    let cmd = format!("{} statusline", crate::sessions::shell_quote(&exe.display().to_string()));
    assert_eq!(command_target(&cmd), Some((exe, None)));
    // Not ours.
    assert_eq!(command_target("~/bin/ma-ligne.sh"), None);
    assert_eq!(command_target("statusline"), None);
    assert_eq!(command_target("a b statusline"), None);
    assert_eq!(command_target(""), None);
    // What our_command writes reads back to the same program.
    let me = std::env::current_exe().unwrap();
    assert_eq!(command_target(&our_command(&me)).map(|(p, _)| p), Some(me));
}

#[test]
fn only_our_status_line_is_given_back() {
    let ours = || json!({ "model": "x", "statusLine": { "type": "command", "command": "'/Applications/Orchestra.app/Contents/MacOS/orchestra' statusline" } });
    // Ours, with the person's previous one: it comes back.
    let mut v = ours();
    assert!(restore_status_line(&mut v, Some("~/bin/ma-ligne.sh\n")).unwrap());
    assert_eq!(v["statusLine"]["command"], "~/bin/ma-ligne.sh");
    assert_eq!(v["model"], "x");
    // Ours, none before: removed.
    let mut v = ours();
    assert!(restore_status_line(&mut v, None).unwrap());
    assert!(v.get("statusLine").is_none());
    // One the person set since: left as it is.
    let mut v = json!({ "statusLine": { "type": "command", "command": "~/bin/autre.sh" } });
    assert!(!restore_status_line(&mut v, Some("~/bin/ma-ligne.sh")).unwrap());
    assert_eq!(v["statusLine"]["command"], "~/bin/autre.sh");
}

/// `orchestra statusline`: called by Claude Code with its JSON on stdin.
pub fn statusline_main() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let v: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    if v.get("rate_limits").is_some_and(|r| r.is_object()) {
        let out = json!({ "rate_limits": v["rate_limits"], "at": crate::model::now_ms() });
        let _ = std::fs::create_dir_all(crate::store::data_dir());
        let _ = crate::store::write_atomic(&captured_file(), out.to_string().as_bytes());
        // The 24-hour figures need every reading, Orchestra open or not.
        let windows = from_rate_limits(&v["rate_limits"]);
        if !windows.is_empty() {
            record(&Usage {
                windows,
                at: crate::model::now_ms(),
            });
        }
    }
    // The person's own status line, unchanged.
    if let Ok(prev) = std::fs::read_to_string(previous_file())
        && !prev.trim().is_empty()
    {
        let shell = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };
        if let Ok(mut child) = crate::proc::command(shell.0)
            .args([shell.1, prev.trim()])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::inherit())
            .spawn()
        {
            if let Some(mut i) = child.stdin.take() {
                let _ = i.write_all(input.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
    // Ours: model, then the windows.
    let mut parts = vec![];
    if let Some(m) = v["model"]["display_name"].as_str() {
        parts.push(m.to_string());
    }
    for w in from_rate_limits(&v["rate_limits"]) {
        let short = if w.label.starts_with("Session") {
            "5 h".to_string()
        } else {
            w.label.to_lowercase()
        };
        parts.push(format!("{short} {:.0} %", w.pct));
    }
    println!("{}", parts.join(" · "));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limits_and_cache() {
        let w = from_rate_limits(
            &json!({"seven_day": {"used_percentage": 41.2, "resets_at": 1738857600}, "five_hour": {"used_percentage": 23.5, "resets_at": 1738425600}}),
        );
        assert_eq!(w[0].label, "Session (5 h)");
        assert_eq!(w[0].resets_at, Some(1738425600000));
        assert_eq!(w[1].pct, 41.2);
        let c = from_cache(
            &json!({"utilization": {"five_hour": {"utilization": 0.3, "resets_at": "2026-11-18T15:00:00Z"}, "limits": [{"display_name": "Fable", "utilization": 0.57}]}}),
        );
        assert_eq!(c.len(), 2);
        assert!((c[0].pct - 30.0).abs() < 0.01);
        assert_eq!(c[1].label, "Semaine (Fable)");
        let p = from_cache(&json!({"five_hour": {"utilization": 23.0}}));
        assert_eq!(p[0].pct, 23.0);
    }

    /// The sessions' figures (their share, the global limit, the 5-hour tile) all come from
    /// `five()`: Claude Code's `five_hour` window, from either source, never the week nor a
    /// per-model limit, whatever the order they arrive in.
    #[test]
    fn the_sessions_count_on_the_five_hour_window() {
        let rl = from_rate_limits(&json!({
            "seven_day_opus": {"used_percentage": 12.0},
            "seven_day": {"used_percentage": 41.0},
            "five_hour": {"used_percentage": 75.0, "resets_at": 1738425600}
        }));
        let cache = from_cache(&json!({"utilization": {
            "seven_day": {"utilization": 0.41},
            "five_hour": {"utilization": 0.75, "resets_at": "2026-11-18T15:00:00Z"},
            "limits": [{"display_name": "Opus", "utilization": 0.12}]
        }}));
        for windows in [rl, cache] {
            let u = Usage { windows, at: 0 };
            assert!((u.five().unwrap().pct - 75.0).abs() < 0.01);
            assert!(u.five().unwrap().resets_at.is_some());
            assert!((u.week().unwrap().pct - 41.0).abs() < 0.01);
            // The 5-hour window comes first: the sidebar tile shows it on its first line.
            assert_eq!(u.windows[0].label, "Session (5 h)");
        }
    }

    #[test]
    fn time_left_before_reset() {
        assert_eq!(until(1_000, 2_000), None);
        assert_eq!(until(5_000, 2_000), Some(3_000));
        assert_eq!(left_text(30_000), "< 1 min");
        assert_eq!(left_text(12 * 60_000), "12 min");
        assert_eq!(left_text((2 * 60 + 4) * 60_000), "2 h 04");
        assert!(left_text((3 * 24 * 60 + 5 * 60) * 60_000).starts_with('3'));
        let u = Usage {
            windows: from_rate_limits(
                &json!({"seven_day": {"used_percentage": 41.0}, "five_hour": {"used_percentage": 23.0}}),
            ),
            at: 0,
        };
        assert_eq!(u.five().unwrap().pct, 23.0);
        assert_eq!(u.week().unwrap().pct, 41.0);
    }

    fn s(at_h: u64, five: f32, reset_h: Option<u64>) -> Sample {
        Sample {
            at: at_h * HOUR,
            five: Some(five),
            five_reset: reset_h.map(|r| r * HOUR),
            week: None,
            week_reset: None,
        }
    }

    #[test]
    fn history_kept_short_and_without_repeats() {
        let mut h = vec![];
        assert!(push_sample(&mut h, s(1, 10.0, Some(5))));
        assert!(!push_sample(&mut h, s(2, 10.0, Some(5))), "same figures");
        assert!(
            !push_sample(
                &mut h,
                Sample {
                    at: 3 * HOUR,
                    ..Default::default()
                }
            ),
            "nothing in it"
        );
        assert!(push_sample(&mut h, s(3, 20.0, Some(5))));
        for i in 4..40 {
            push_sample(&mut h, s(i, i as f32, Some(i + 4)));
        }
        // 30 hours kept, plus one reading before as a starting point.
        assert!(h[0].at < 9 * HOUR && h[1].at >= 9 * HOUR);
    }

    #[test]
    fn five_hour_windows_added_over_a_day() {
        let now = 40 * HOUR;
        // Window ended 30 h ago (too old), one that ended 18 h after its start before now - 24 h
        // but was still running then, two inside the day.
        let h = vec![
            s(8, 50.0, Some(10)),
            s(14, 30.0, Some(17)),
            s(16, 40.0, Some(17)),
            s(25, 20.0, Some(29)),
            s(28, 35.0, Some(29)),
            s(36, 10.0, Some(40)),
        ];
        assert_eq!(day_total(&h, now), Some(40.0 + 35.0 + 10.0));
        // Without reset times: a drop starts a new window.
        let h = vec![
            s(30, 10.0, None),
            s(31, 25.0, None),
            s(33, 5.0, None),
            s(34, 15.0, None),
        ];
        assert_eq!(day_total(&h, now), Some(25.0 + 15.0));
        assert_eq!(day_total(&[], now), None);
    }

    #[test]
    fn codex_limits_are_read_from_its_session() {
        use crate::auto::Agent;
        let text = concat!(
            r#"{"timestamp":"2026-11-21T10:00:00.000Z","type":"session_meta","payload":{"cwd":"/x"}}"#, "\n",
            r#"{"timestamp":"2026-11-21T10:01:00.000Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":10.0,"window_minutes":300,"resets_at":1795000000},"secondary":{"used_percent":3.0,"window_minutes":10080,"resets_at":1795500000}}}}"#, "\n",
            r#"{"timestamp":"2026-11-21T10:02:00.000Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"secondary":{"used_percent":4.5,"window_minutes":10080,"resets_in_seconds":3600},"primary":{"used_percent":12.5,"window_minutes":300,"resets_in_seconds":60}}}}"#, "\n",
            r#"{"timestamp":"2026-11-21T10:03:00.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"rate_limits ?"}]}}"#, "\n",
        );
        let u = codex_usage(text, 0).unwrap();
        // The last figures, in the same windows and order as Claude Code's.
        let at = crate::sessions::parse_iso_ms("2026-11-21T10:02:00.000Z").unwrap();
        assert_eq!(u.at, at);
        assert_eq!(u.five().unwrap().pct, 12.5);
        assert_eq!(u.five().unwrap().resets_at, Some(at + 60_000));
        assert_eq!(u.week().unwrap().pct, 4.5);
        assert_eq!(u.windows[0].label, "Session (5 h)");
        // An older record with times in seconds since epoch.
        let first = text.lines().nth(1).unwrap();
        let u = codex_usage(first, 0).unwrap();
        assert_eq!(u.five().unwrap().resets_at, Some(1_795_000_000_000));
        // Nothing to read: no figures, never a made-up 0 %.
        assert!(codex_usage("", 0).is_none());
        assert!(codex_usage(r#"{"payload":{"rate_limits":null}}"#, 0).is_none());
        assert!(codex_usage(r#"{"payload":{"rate_limits":{"primary":{}}}}"#, 0).is_none());
        assert!(reports_usage(Agent::ClaudeCode) && reports_usage(Agent::Codex));
        assert!(!reports_usage(Agent::Gemini) && !reports_usage(Agent::Vibe));
        assert!(read_for(Agent::Gemini).is_none() && read_for(Agent::Vibe).is_none());
        // Every agent either has its figures read or says why not, never both.
        for a in Agent::ALL {
            assert_eq!(unreadable_reason(a).is_none(), reports_usage(a), "{a:?}");
        }
    }

    #[test]
    fn week_used_over_a_day() {
        let w = |at_h: u64, pct: f32, reset_h: u64| Sample {
            at: at_h * HOUR,
            week: Some(pct),
            week_reset: Some(reset_h * HOUR),
            ..Default::default()
        };
        let now = 40 * HOUR;
        // 30 % before the day, 34 % then 36 %: 6 points over a full day.
        let (used, span) = day_week_used(
            &[
                w(10, 28.0, 100),
                w(15, 30.0, 100),
                w(20, 34.0, 100),
                w(39, 36.0, 100),
            ],
            now,
        )
        .unwrap();
        assert!((used - 6.0).abs() < 0.01);
        assert_eq!(span, 24 * HOUR);
        assert!((daily_pct(used) - 42.0).abs() < 0.01);
        // The week starts again: what came before the reset, then the new figure.
        let (used, span) =
            day_week_used(&[w(30, 90.0, 32), w(31, 95.0, 32), w(35, 3.0, 200)], now).unwrap();
        assert!((used - 8.0).abs() < 0.01);
        assert_eq!(span, 10 * HOUR);
        assert_eq!(day_week_used(&[], now), None);
        // Two sources taking turns (the status line, live, and ~/.claude.json, a little
        // late): the figure goes 40, 39.5, 40.2, 39.8, 41. Only the week's real rise, 1
        // point, counts; the dips and the climbs back over them don't.
        let noisy = [
            w(20, 40.0, 100),
            w(21, 39.5, 100),
            w(22, 40.2, 100),
            w(23, 39.8, 100),
            w(24, 41.0, 100),
        ];
        let (used, _) = day_week_used(&noisy, now).unwrap();
        assert!((used - 1.0).abs() < 0.01, "{used}");
        // The late source without a reset time: a dip of a point or two is still the same
        // week, not a new one whose whole figure would count.
        let bare = |at_h: u64, pct: f32| Sample {
            at: at_h * HOUR,
            week: Some(pct),
            ..Default::default()
        };
        let (used, _) =
            day_week_used(&[w(20, 40.0, 100), bare(21, 38.5), w(22, 41.0, 100)], now).unwrap();
        assert!((used - 1.0).abs() < 0.01, "{used}");
        // Without any reset time, a real new week (a big drop) still counts from zero.
        let (used, _) =
            day_week_used(&[bare(30, 90.0), bare(31, 95.0), bare(35, 3.0)], now).unwrap();
        assert!((used - 8.0).abs() < 0.01, "{used}");
    }

    /// The figures themselves, not « 105 % de la part du jour »: 15 points of the week in
    /// 24 h is a little faster than a day's share (14.3); over 3 h, 2 points is fast too.
    #[test]
    fn day_pace_reads_in_points_of_the_week() {
        let (line, fast) = day_pace_line(15.0, 24 * HOUR);
        // French by default in tests: decimal commas.
        assert!(
            line.contains("15,0 %") && line.contains("14,3 %") && !line.contains("105"),
            "{line}"
        );
        assert!(fast);
        assert!(!day_pace_line(10.0, 24 * HOUR).1);
        assert!(
            !day_pace_line(14.28, 24 * HOUR).1,
            "a day's share exactly is not too fast"
        );
        // Over 3 hours, a day's share is about 1.8 points.
        assert!(day_pace_line(2.0, 3 * HOUR).1);
        assert!(!day_pace_line(1.0, 3 * HOUR).1);
    }

    #[test]
    fn pause_before_the_reserve() {
        assert!(!near_reserve(80.0, 95.0, 3.0));
        assert!(
            near_reserve(92.5, 95.0, 3.0),
            "the next turn would bite into it"
        );
        assert!(
            near_reserve(94.6, 95.0, 0.0),
            "half a point of margin at least"
        );
        assert!(near_reserve(96.0, 95.0, 0.0));
    }
}
