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
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"))
}

fn claude_json() -> PathBuf {
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
    label.replace("Semaine", "Week").replace("Dépenses", "Spending")
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
        return Some(if n > 1e12 { n as u64 } else { (n * 1000.0) as u64 });
    }
    v.as_str().and_then(crate::sessions::parse_iso_ms)
}

/// The status line's `rate_limits` object.
fn from_rate_limits(v: &Value) -> Vec<Window> {
    let Some(o) = v.as_object() else { return vec![] };
    let mut out: Vec<Window> = o
        .iter()
        .filter_map(|(k, w)| {
            let pct = w.get("used_percentage")?.as_f64()? as f32;
            Some(Window { label: label_for(k), pct: pct.clamp(0.0, 100.0), resets_at: w.get("resets_at").and_then(time_ms) })
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
                    let name = o.get("display_name").or_else(|| o.get("name")).and_then(Value::as_str).map(|n| format!("Semaine ({n})")).unwrap_or_else(|| label_for(key));
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
        out.push(Window { label, pct: pct.clamp(0.0, 100.0), resets_at: r });
    }
    out.sort_by_key(|w| order(&w.label));
    out
}

fn mtime_ms(p: &std::path::Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The latest figures, from whichever source is freshest. Windows that already started
/// again are dropped (their figure is stale).
pub fn read() -> Option<Usage> {
    let now = crate::model::now_ms();
    let mut best: Option<Usage> = None;
    if let Some(v) = std::fs::read_to_string(captured_file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        let w = from_rate_limits(&v["rate_limits"]);
        if !w.is_empty() {
            best = Some(Usage { windows: w, at: v["at"].as_u64().unwrap_or(0) });
        }
    }
    let cj = claude_json();
    if let Some(v) = std::fs::read_to_string(&cj).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        let c = &v["cachedUsageUtilization"];
        let w = from_cache(c);
        if !w.is_empty() {
            let at = ["fetchedAt", "timestamp", "updatedAt", "lastUpdated"].iter().find_map(|k| c.get(*k).and_then(time_ms)).unwrap_or_else(|| mtime_ms(&cj));
            if best.as_ref().is_none_or(|b| at > b.at) {
                best = Some(Usage { windows: w, at });
            }
        }
    }
    let mut u = best?;
    u.windows.retain(|w| w.resets_at.is_none_or(|r| r > now));
    (!u.windows.is_empty()).then_some(u)
}

// ---------------------------------------------------------------- status line

fn settings_file() -> PathBuf {
    claude_dir().join("settings.json")
}

fn our_command(exe: &std::path::Path) -> String {
    format!("{} statusline", crate::sessions::shell_quote(&exe.display().to_string()))
}

/// Our status line is the one Claude Code runs.
pub fn tracking_on() -> bool {
    std::fs::read_to_string(settings_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["statusLine"]["command"].as_str().map(|c| c.ends_with(" statusline")))
        .unwrap_or(false)
}

/// Make `orchestra statusline` Claude Code's status line. The previous one keeps running
/// (ours calls it), and comes back with `stop_tracking`.
pub fn start_tracking(exe: &std::path::Path) -> Result<(), String> {
    let f = settings_file();
    let text = std::fs::read_to_string(&f).unwrap_or_default();
    let mut v: Value = if text.trim().is_empty() { json!({}) } else { serde_json::from_str(&text).map_err(|e| format!("{} illisible ({e}), laissé tel quel", f.display()))? };
    let obj = v.as_object_mut().ok_or("format inattendu")?;
    let mine = our_command(exe);
    if let Some(prev) = obj.get("statusLine").and_then(|s| s["command"].as_str()) {
        if !prev.ends_with(" statusline") {
            std::fs::create_dir_all(crate::store::data_dir()).map_err(|e| e.to_string())?;
            std::fs::write(previous_file(), prev).map_err(|e| e.to_string())?;
        }
    }
    obj.insert("statusLine".into(), json!({ "type": "command", "command": mine, "padding": 0 }));
    if !text.is_empty() {
        let bak = f.with_extension("json.bak");
        if !bak.exists() {
            let _ = std::fs::write(&bak, &text);
        }
    }
    std::fs::create_dir_all(claude_dir()).map_err(|e| e.to_string())?;
    std::fs::write(&f, serde_json::to_string_pretty(&v).unwrap_or_default() + "\n").map_err(|e| e.to_string())
}

/// Put the person's own status line back (or none).
pub fn stop_tracking() -> Result<(), String> {
    let f = settings_file();
    let Ok(text) = std::fs::read_to_string(&f) else { return Ok(()) };
    let mut v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let obj = v.as_object_mut().ok_or("format inattendu")?;
    match std::fs::read_to_string(previous_file()) {
        Ok(prev) if !prev.trim().is_empty() => {
            obj.insert("statusLine".into(), json!({ "type": "command", "command": prev.trim() }));
        }
        _ => {
            obj.remove("statusLine");
        }
    }
    let _ = std::fs::remove_file(previous_file());
    std::fs::write(&f, serde_json::to_string_pretty(&v).unwrap_or_default() + "\n").map_err(|e| e.to_string())
}

/// `orchestra statusline`: called by Claude Code with its JSON on stdin.
pub fn statusline_main() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let v: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    if v.get("rate_limits").is_some_and(|r| r.is_object()) {
        let out = json!({ "rate_limits": v["rate_limits"], "at": crate::model::now_ms() });
        let _ = std::fs::create_dir_all(crate::store::data_dir());
        let tmp = captured_file().with_extension("json.tmp");
        if std::fs::write(&tmp, out.to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, captured_file());
        }
    }
    // The person's own status line, unchanged.
    if let Ok(prev) = std::fs::read_to_string(previous_file()) {
        if !prev.trim().is_empty() {
            let shell = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
            if let Ok(mut child) = std::process::Command::new(shell.0)
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
    }
    // Ours: model, then the windows.
    let mut parts = vec![];
    if let Some(m) = v["model"]["display_name"].as_str() {
        parts.push(m.to_string());
    }
    for w in from_rate_limits(&v["rate_limits"]) {
        let short = if w.label.starts_with("Session") { "5 h".to_string() } else { w.label.to_lowercase() };
        parts.push(format!("{short} {:.0} %", w.pct));
    }
    println!("{}", parts.join(" · "));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limits_and_cache() {
        let w = from_rate_limits(&json!({"seven_day": {"used_percentage": 41.2, "resets_at": 1738857600}, "five_hour": {"used_percentage": 23.5, "resets_at": 1738425600}}));
        assert_eq!(w[0].label, "Session (5 h)");
        assert_eq!(w[0].resets_at, Some(1738425600000));
        assert_eq!(w[1].pct, 41.2);
        let c = from_cache(&json!({"utilization": {"five_hour": {"utilization": 0.3, "resets_at": "2026-11-18T15:00:00Z"}, "limits": [{"display_name": "Fable", "utilization": 0.57}]}}));
        assert_eq!(c.len(), 2);
        assert!((c[0].pct - 30.0).abs() < 0.01);
        assert_eq!(c[1].label, "Semaine (Fable)");
        let p = from_cache(&json!({"five_hour": {"utilization": 23.0}}));
        assert_eq!(p[0].pct, 23.0);
    }
}
