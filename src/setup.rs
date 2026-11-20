//! At start, plug every AI found on this machine into the dashboard's MCP server, so they
//! can all read and write the projects without any manual setup:
//!
//! - Claude Code and Codex, through their own `mcp add` command;
//! - Gemini CLI and the Claude desktop app, by adding one entry to their JSON settings
//!   (a `.bak` copy of the file is kept the first time).
//!
//! An entry that already points to this program is left alone; one that points to an older
//! copy (moved or rebuilt elsewhere) is updated. Nothing else in those files is touched.

use crate::sessions::Tool;
use crate::sync::{Agent, detect, shell_path};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const NAME: &str = "orchestra";
/// Names the server had before (the app was "Tableau IA"): removed when found.
const OLD_NAMES: [&str; 1] = ["tableau-ia"];

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    AlreadyThere,
    Added,
    Updated,
    Failed(String),
}

impl Outcome {
    pub fn changed(&self) -> bool {
        matches!(self, Outcome::Added | Outcome::Updated)
    }
}

/// (AI name, what happened), for every AI found.
pub type Report = Vec<(String, Outcome)>;

fn cli(bin: &Path, args: &[&str], path: &str) -> Result<String, String> {
    let mut c = Command::new(bin);
    c.args(args).env("PATH", path).stdin(Stdio::null());
    for k in crate::sync::API_KEY_VARS {
        c.env_remove(k);
    }
    let o = c.output().map_err(|e| e.to_string())?;
    let out = String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr);
    if o.status.success() { Ok(out) } else { Err(out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("échec").trim().to_string()) }
}

/// Claude Code or Codex: register `exe mcp` under NAME, or fix the path if it moved.
pub fn plug_cli(agent: &Agent, exe: &Path, path: &str) -> Outcome {
    let exe_s = exe.display().to_string();
    for old in OLD_NAMES {
        let rm: Vec<&str> = match agent.tool {
            Tool::ClaudeCode => vec!["mcp", "remove", old, "-s", "user"],
            Tool::Codex => vec!["mcp", "remove", old],
        };
        let _ = cli(&agent.bin, &rm, path);
    }
    let existing = match agent.tool {
        Tool::ClaudeCode => cli(&agent.bin, &["mcp", "get", NAME], path).ok(),
        Tool::Codex => cli(&agent.bin, &["mcp", "list"], path).ok().filter(|o| o.contains(NAME)),
    };
    if let Some(out) = &existing {
        if out.contains(&exe_s) {
            return Outcome::AlreadyThere;
        }
        let rm: Vec<&str> = match agent.tool {
            Tool::ClaudeCode => vec!["mcp", "remove", NAME, "-s", "user"],
            Tool::Codex => vec!["mcp", "remove", NAME],
        };
        let _ = cli(&agent.bin, &rm, path);
    }
    let add: Vec<&str> = match agent.tool {
        Tool::ClaudeCode => vec!["mcp", "add", "--scope", "user", NAME, "--", &exe_s, "mcp"],
        Tool::Codex => vec!["mcp", "add", NAME, "--", &exe_s, "mcp"],
    };
    match cli(&agent.bin, &add, path) {
        Ok(_) if existing.is_some() => Outcome::Updated,
        Ok(_) => Outcome::Added,
        Err(e) => Outcome::Failed(e),
    }
}

/// Apps configured by a JSON file with an `mcpServers` object (Gemini CLI, Claude desktop).
pub fn plug_json(file: &Path, exe: &Path) -> Outcome {
    let entry = json!({ "command": exe.display().to_string(), "args": ["mcp"] });
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let mut v: Value = if text.trim().is_empty() {
        json!({})
    } else {
        match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => return Outcome::Failed(crate::trf!("{} illisible ({e}), laissé tel quel", "{} unreadable ({e}), left as is", file.display())),
        }
    };
    let Some(obj) = v.as_object_mut() else { return Outcome::Failed(crate::tr!("format inattendu, laissé tel quel", "unexpected format, left as is").into()) };
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    let Some(servers) = servers.as_object_mut() else { return Outcome::Failed(crate::tr!("« mcpServers » inattendu, laissé tel quel", "unexpected « mcpServers », left as is").into()) };
    let mut renamed = false;
    for old in OLD_NAMES {
        renamed |= servers.remove(old).is_some();
    }
    let before = servers.get(NAME).cloned();
    if !renamed && before.as_ref().and_then(|b| b.get("command")) == entry.get("command") {
        return Outcome::AlreadyThere;
    }
    servers.insert(NAME.into(), entry);
    if !text.is_empty() {
        let bak = file.with_extension("json.bak");
        if !bak.exists() {
            let _ = std::fs::write(&bak, &text);
        }
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(file, serde_json::to_string_pretty(&v).unwrap_or_default() + "\n") {
        Ok(()) if before.is_some() => Outcome::Updated,
        Ok(()) => Outcome::Added,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

// ------------------------------------------------------------------ Mistral Vibe (TOML)

/// Mistral Vibe's settings: `$VIBE_HOME/config.toml`, by default `~/.vibe/config.toml`. Its
/// MCP servers are `[[mcp_servers]]` tables (name, transport, command, args).
fn vibe_config() -> PathBuf {
    let home = std::env::var_os("VIBE_HOME").map(PathBuf::from).unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".vibe"));
    home.join("config.toml")
}

/// The file cut into its tables: each piece starts at a `[…]` header (the first one may
/// have none: the top-level keys).
fn toml_tables(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![String::new()];
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with('[') && !out.last().is_some_and(|t| t.is_empty()) {
            out.push(String::new());
        }
        out.last_mut().unwrap().push_str(line);
    }
    out
}

/// One of our `[[mcp_servers]]` tables (under our name or an old one).
fn toml_is_ours(table: &str) -> bool {
    let mut lines = table.lines();
    if lines.next().map(|h| h.split('#').next().unwrap_or("").trim().replace(' ', "")) != Some("[[mcp_servers]]".into()) {
        return false;
    }
    lines.any(|l| {
        let l = l.split('#').next().unwrap_or("").replace(' ', "");
        std::iter::once(NAME).chain(OLD_NAMES).any(|n| l == format!("name=\"{n}\"") || l == format!("name='{n}'"))
    })
}

fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn toml_entry(exe: &Path) -> String {
    format!("[[mcp_servers]]\nname = \"{NAME}\"\ntransport = \"stdio\"\ncommand = {}\nargs = [\"mcp\"]\n", toml_string(&exe.display().to_string()))
}

/// Mistral Vibe: put our `[[mcp_servers]]` table in its TOML settings, or fix the path if
/// it moved. Everything else in the file stays as written (comments included); a `.bak`
/// copy is kept the first time.
pub fn plug_toml(file: &Path, exe: &Path) -> Outcome {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let tables = toml_tables(&text);
    let ours: Vec<&String> = tables.iter().filter(|t| toml_is_ours(t)).collect();
    let entry = toml_entry(exe);
    if ours.len() == 1 && ours[0].trim_end() == entry.trim_end() {
        return Outcome::AlreadyThere;
    }
    let mut kept: String = tables.iter().filter(|t| !toml_is_ours(t)).map(String::as_str).collect();
    if !kept.is_empty() && !kept.ends_with("\n\n") {
        kept.push_str(if kept.ends_with('\n') { "\n" } else { "\n\n" });
    }
    kept.push_str(&entry);
    if !text.is_empty() {
        let bak = file.with_extension("toml.bak");
        if !bak.exists() {
            let _ = std::fs::write(&bak, &text);
        }
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(file, kept) {
        Ok(()) if !ours.is_empty() => Outcome::Updated,
        Ok(()) => Outcome::Added,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

fn toml_has(file: &Path) -> bool {
    std::fs::read_to_string(file).is_ok_and(|t| toml_tables(&t).iter().any(|x| toml_is_ours(x)))
}

fn unplug_toml(file: &Path) -> Result<(), String> {
    let Ok(text) = std::fs::read_to_string(file) else { return Ok(()) };
    let tables = toml_tables(&text);
    if !tables.iter().any(|t| toml_is_ours(t)) {
        return Ok(());
    }
    let kept: String = tables.iter().filter(|t| !toml_is_ours(t)).map(String::as_str).collect();
    std::fs::write(file, kept.trim_end().to_string() + "\n").map_err(|e| e.to_string())
}

fn gemini_settings() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".gemini").join("settings.json")
}

fn cursor_settings() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".cursor").join("mcp.json")
}

fn claude_desktop_config() -> Option<PathBuf> {
    // macOS: ~/Library/Application Support/Claude ; Windows: %APPDATA%\Claude ; Linux: ~/.config/Claude
    let dir = dirs::config_dir()?.join("Claude");
    dir.is_dir().then(|| dir.join("claude_desktop_config.json"))
}

/// The AIs the dashboard knows how to plug in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ai {
    ClaudeCode,
    Codex,
    Gemini,
    ClaudeDesktop,
    Cursor,
    /// Mistral Vibe, Mistral's coding agent in the terminal (`vibe`).
    Vibe,
}

impl Ai {
    pub const ALL: [Ai; 6] = [Ai::ClaudeCode, Ai::Codex, Ai::Gemini, Ai::Vibe, Ai::ClaudeDesktop, Ai::Cursor];

    /// Stable name, kept in the settings.
    pub fn key(self) -> &'static str {
        match self {
            Ai::ClaudeCode => "claude-code",
            Ai::Codex => "codex",
            Ai::Gemini => "gemini",
            Ai::ClaudeDesktop => "claude-desktop",
            Ai::Cursor => "cursor",
            Ai::Vibe => "vibe",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Ai::ClaudeCode => "Claude Code",
            Ai::Codex => "Codex",
            Ai::Gemini => "Gemini CLI",
            Ai::ClaudeDesktop => crate::tr!("Claude (app de bureau)", "Claude (desktop app)"),
            Ai::Cursor => "Cursor",
            Ai::Vibe => "Mistral Vibe",
        }
    }
    pub fn what(self) -> &'static str {
        match self {
            Ai::ClaudeCode => crate::tr!("Agent de code d'Anthropic, sur votre abonnement Claude", "Anthropic's coding agent, on your Claude subscription"),
            Ai::Codex => crate::tr!("Agent de code d'OpenAI, sur votre abonnement ChatGPT", "OpenAI's coding agent, on your ChatGPT subscription"),
            Ai::Gemini => crate::tr!("Agent de code de Google, sur votre compte Google", "Google's coding agent, on your Google account"),
            Ai::ClaudeDesktop => crate::tr!("L'app Claude : ses conversations lisent et écrivent vos projets", "The Claude app: its conversations read and write your projects"),
            Ai::Cursor => crate::tr!("L'éditeur Cursor : son agent lit et écrit vos projets", "The Cursor editor: its agent reads and writes your projects"),
            Ai::Vibe => crate::tr!("Agent de code de Mistral, sur votre compte Mistral", "Mistral's coding agent, on your Mistral account"),
        }
    }
    /// Command to start it in a terminal, for the agents that run in one.
    pub fn launch(self) -> Option<&'static str> {
        match self {
            Ai::ClaudeCode => Some("claude"),
            Ai::Codex => Some("codex"),
            Ai::Gemini => Some("gemini"),
            Ai::Vibe => Some("vibe"),
            _ => None,
        }
    }
    /// What to type in a terminal to sign in with one's account.
    pub fn login(self) -> Option<(&'static str, &'static str)> {
        match self {
            Ai::ClaudeCode => Some(("claude", "/login")),
            Ai::Codex => Some(("codex login", "")),
            Ai::Gemini => Some(("gemini", "")),
            // Its first start asks for the Mistral key.
            Ai::Vibe => Some(("vibe", "")),
            _ => None,
        }
    }
    /// The official install command (run in the built-in terminal, in view).
    pub fn install(self) -> Option<&'static str> {
        match self {
            Ai::ClaudeCode => Some("npm install -g @anthropic-ai/claude-code"),
            Ai::Codex => Some("npm install -g @openai/codex"),
            Ai::Gemini => Some("npm install -g @google/gemini-cli"),
            // Mistral's Python package, through uv.
            Ai::Vibe => Some("uv tool install mistral-vibe"),
            _ => None,
        }
    }
    pub fn download_page(self) -> Option<&'static str> {
        match self {
            Ai::ClaudeDesktop => Some("https://claude.ai/download"),
            Ai::Cursor => Some("https://cursor.com/download"),
            _ => None,
        }
    }
    pub fn accent(self) -> (u8, u8, u8) {
        match self {
            Ai::ClaudeCode | Ai::ClaudeDesktop => (204, 120, 92),
            Ai::Codex => (110, 150, 230),
            Ai::Gemini => (66, 133, 244),
            Ai::Cursor => (150, 150, 150),
            Ai::Vibe => (250, 82, 15),
        }
    }
    /// For the AIs set up in a TOML file (Mistral Vibe).
    fn toml_file(self) -> Option<PathBuf> {
        match self {
            Ai::Vibe => Some(vibe_config()),
            _ => None,
        }
    }
    fn agent(self, path: &str) -> Option<Agent> {
        let tool = match self {
            Ai::ClaudeCode => Tool::ClaudeCode,
            Ai::Codex => Tool::Codex,
            _ => return None,
        };
        detect(path).into_iter().find(|a| a.tool == tool)
    }
    fn json_file(self) -> Option<PathBuf> {
        match self {
            Ai::Gemini => Some(gemini_settings()),
            Ai::ClaudeDesktop => claude_desktop_config(),
            Ai::Cursor => Some(cursor_settings()),
            _ => None,
        }
    }
}

/// Where an AI stands on this machine.
#[derive(Clone, Debug, PartialEq)]
pub struct AiState {
    pub ai: Ai,
    pub installed: bool,
    /// Knows the dashboard's MCP server.
    pub plugged: bool,
}

fn installed(ai: Ai, path: &str) -> bool {
    match ai {
        Ai::ClaudeCode | Ai::Codex => ai.agent(path).is_some(),
        Ai::Gemini => crate::sync::which("gemini", path).is_some(),
        Ai::ClaudeDesktop => claude_desktop_config().is_some(),
        Ai::Cursor => cursor_settings().parent().is_some_and(Path::is_dir),
        Ai::Vibe => crate::sync::which("vibe", path).is_some(),
    }
}

fn json_has(file: &Path) -> bool {
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).is_some_and(|v| v["mcpServers"].get(NAME).is_some())
}

fn plugged(ai: Ai, path: &str) -> bool {
    if let Some(a) = ai.agent(path) {
        return match a.tool {
            Tool::ClaudeCode => cli(&a.bin, &["mcp", "get", NAME], path).is_ok(),
            Tool::Codex => cli(&a.bin, &["mcp", "list"], path).is_ok_and(|o| o.contains(NAME)),
        };
    }
    if let Some(f) = ai.toml_file() {
        return toml_has(&f);
    }
    ai.json_file().is_some_and(|f| json_has(&f))
}

/// The state of every known AI (runs the CLIs: call it off the UI thread).
pub fn states() -> Vec<AiState> {
    let path = shell_path();
    Ai::ALL
        .into_iter()
        .map(|ai| {
            let installed = installed(ai, &path);
            AiState { ai, installed, plugged: installed && plugged(ai, &path) }
        })
        .collect()
}

/// Plug one AI into the dashboard.
pub fn plug(ai: Ai, exe: &Path) -> Outcome {
    let path = shell_path();
    if let Some(a) = ai.agent(&path) {
        return plug_cli(&a, exe, &path);
    }
    if let Some(f) = ai.toml_file() {
        return if installed(ai, &path) { plug_toml(&f, exe) } else { Outcome::Failed(crate::trf!("{} n'est pas installé", "{} isn't installed", ai.label())) };
    }
    match ai.json_file() {
        Some(f) if installed(ai, &path) => plug_json(&f, exe),
        _ => Outcome::Failed(crate::trf!("{} n'est pas installé", "{} isn't installed", ai.label())),
    }
}

/// Unplug one AI: remove the dashboard's entry, nothing else.
pub fn unplug(ai: Ai) -> Result<(), String> {
    let path = shell_path();
    if let Some(a) = ai.agent(&path) {
        let rm: Vec<&str> = match a.tool {
            Tool::ClaudeCode => vec!["mcp", "remove", NAME, "-s", "user"],
            Tool::Codex => vec!["mcp", "remove", NAME],
        };
        return cli(&a.bin, &rm, &path).map(|_| ());
    }
    if let Some(f) = ai.toml_file() {
        return unplug_toml(&f);
    }
    let Some(f) = ai.json_file() else { return Ok(()) };
    let Ok(text) = std::fs::read_to_string(&f) else { return Ok(()) };
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("{} illisible ({e})", f.display()))?;
    if v["mcpServers"].as_object_mut().and_then(|o| o.remove(NAME)).is_some() {
        std::fs::write(&f, serde_json::to_string_pretty(&v).unwrap_or_default() + "\n").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Plug every AI found, except those the person unplugged (`off`, by key).
/// Slow-ish (runs the CLIs): call it off the UI thread.
pub fn plug_all(exe: &Path, off: &[String]) -> Report {
    let path = shell_path();
    let mut out: Report = vec![];
    for ai in Ai::ALL {
        if off.iter().any(|k| k == ai.key()) || !installed(ai, &path) {
            continue;
        }
        let o = match (ai.agent(&path), ai.toml_file()) {
            (Some(a), _) => plug_cli(&a, exe, &path),
            (None, Some(f)) => plug_toml(&f, exe),
            (None, None) => match ai.json_file() {
                Some(f) => plug_json(&f, exe),
                None => continue,
            },
        };
        out.push((ai.label().to_string(), o));
    }
    out
}

/// One line for the status bar, only when something changed or failed.
pub fn summary(r: &Report) -> Option<String> {
    let changed: Vec<&str> = r.iter().filter(|(_, o)| o.changed()).map(|(n, _)| n.as_str()).collect();
    let failed: Vec<String> = r.iter().filter_map(|(n, o)| if let Outcome::Failed(e) = o { Some(format!("{n} ({e})")) } else { None }).collect();
    let mut parts = vec![];
    if !changed.is_empty() {
        let mut s = crate::trf!("Branché au tableau de bord : {}", "Plugged into the dashboard: {}", changed.join(", "));
        if changed.iter().any(|n| n.starts_with("Claude (app")) {
            s.push_str(crate::tr!(" — redémarrez l'app Claude pour qu'elle le voie", " — restart the Claude app so it sees it"));
        }
        parts.push(s);
    }
    if !failed.is_empty() {
        parts.push(crate::trf!("Échec du branchement : {}", "Plugging in failed: {}", failed.join(", ")));
    }
    (!parts.is_empty()).then(|| parts.join(". ") + ".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_settings_keep_everything_else() {
        let dir = std::env::temp_dir().join(format!("tia-setup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("settings.json");
        std::fs::write(&f, r#"{"theme":"dark","mcpServers":{"other":{"command":"x"}}}"#).unwrap();
        let exe = Path::new("/apps/orchestra");
        assert_eq!(plug_json(&f, exe), Outcome::Added);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"][NAME]["command"], "/apps/orchestra");
        assert!(dir.join("settings.json.bak").exists());
        assert_eq!(plug_json(&f, exe), Outcome::AlreadyThere);
        assert_eq!(plug_json(&f, Path::new("/moved/orchestra")), Outcome::Updated);
        std::fs::write(&f, "{ broken").unwrap();
        assert!(matches!(plug_json(&f, exe), Outcome::Failed(_)));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "{ broken");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Mistral Vibe's TOML: our table added, put right when the program moved, removed on
    /// unplugging; the person's own settings, comments and other servers never touched.
    #[test]
    fn vibe_toml_keeps_everything_else() {
        let dir = std::env::temp_dir().join(format!("orch-vibe-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("config.toml");
        let mine = "# mes réglages\nactive_model = \"devstral\"\n\n[[mcp_servers]]\nname = \"fetch\"\ntransport = \"stdio\"\ncommand = \"uvx\"\nargs = [\"mcp-server-fetch\"]\n\n[tools.bash]\npermission = \"ask\"\n";
        std::fs::write(&f, mine).unwrap();
        let exe = Path::new("/Applications/Orchestra.app/Contents/MacOS/orchestra");
        assert_eq!(plug_toml(&f, exe), Outcome::Added);
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.starts_with(mine), "the file as it was, then our table:\n{text}");
        assert!(text.contains("name = \"orchestra\"") && text.contains("command = \"/Applications/Orchestra.app/Contents/MacOS/orchestra\"") && text.contains("args = [\"mcp\"]"));
        assert!(toml_has(&f));
        assert!(dir.join("config.toml.bak").exists());
        assert_eq!(plug_toml(&f, exe), Outcome::AlreadyThere);
        // Moved: one table, the new path.
        assert_eq!(plug_toml(&f, Path::new("/moved/orchestra")), Outcome::Updated);
        let text = std::fs::read_to_string(&f).unwrap();
        assert_eq!(text.matches("name = \"orchestra\"").count(), 1);
        assert!(text.contains("/moved/orchestra") && !text.contains("/Applications/Orchestra.app"));
        assert!(text.contains("name = \"fetch\"") && text.contains("[tools.bash]") && text.contains("# mes réglages"));
        // Unplugged: back to the person's settings only.
        unplug_toml(&f).unwrap();
        assert!(!toml_has(&f));
        assert_eq!(std::fs::read_to_string(&f).unwrap().trim_end(), mine.trim_end());
        // No file yet: one is made. A path with quotes stays valid TOML.
        let new = dir.join("neuf").join("config.toml");
        assert_eq!(plug_toml(&new, Path::new("/a \"b\"/orchestra")), Outcome::Added);
        assert!(std::fs::read_to_string(&new).unwrap().contains(r#"command = "/a \"b\"/orchestra""#));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fake_claude_cli() {
        // A stand-in "claude" that knows the server under an old path.
        let base = std::env::temp_dir().join(format!("tia-setup-cli-{}", std::process::id()));
        std::fs::create_dir_all(base.join("bin")).unwrap();
        let log = base.join("log");
        let bin = base.join("bin/claude");
        std::fs::write(
            &bin,
            format!("#!/bin/sh\necho \"$@\" >> {}\nif [ \"$2\" = get ]; then echo 'Command: /old/orchestra'; fi\nexit 0\n", log.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let agent = Agent { tool: Tool::ClaudeCode, bin };
        assert_eq!(plug_cli(&agent, Path::new("/new/orchestra"), "/usr/bin:/bin"), Outcome::Updated);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(calls.contains("mcp remove orchestra -s user"));
        assert!(calls.contains("mcp add --scope user orchestra -- /new/orchestra mcp"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn summary_only_when_useful() {
        assert_eq!(summary(&vec![("Claude Code".into(), Outcome::AlreadyThere)]), None);
        let s = summary(&vec![("Codex".into(), Outcome::Added), ("Gemini CLI".into(), Outcome::Failed("x".into()))]).unwrap();
        assert!(s.contains("Codex") && s.contains("Gemini CLI (x)"));
    }
}
