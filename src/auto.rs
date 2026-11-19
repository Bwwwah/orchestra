//! Autonomous work: an AI works alone on a project until its limit, in a copy of the
//! repository of its own (a git worktree on a branch `orchestra/auto-…`), so several
//! projects can go on in parallel and nothing touches the person's working copy.
//!
//! The AI is the person's own Claude Code, run *interactively* in a hidden terminal: that
//! mode renews its sign-in by itself, and reports the use of the subscription to the status
//! line (usage.rs) after each answer, which is how the limits are checked. Orchestra types
//! the instructions, reads the session file to know when a turn is over, and starts the next
//! one until the work is done, a limit is reached or it is stopped.
//!
//! Only Claude Code for now; what depends on the agent is in `Agent` (command, session
//! files), so Codex can be added there.

use serde_json::Value;
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
}

impl Agent {
    pub fn label(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "Claude Code",
        }
    }

    /// The command started in the hidden terminal.
    pub fn command(self, allow_commits: bool, extra: &str) -> String {
        match self {
            Agent::ClaudeCode => {
                let mut allowed: Vec<String> = [
                    "Read", "Edit", "Write", "MultiEdit", "Glob", "Grep", "TodoWrite", "mcp__orchestra",
                    "Bash(ls:*)", "Bash(cat:*)", "Bash(pwd)", "Bash(git status:*)", "Bash(git diff:*)", "Bash(git log:*)", "Bash(git show:*)",
                    "Bash(cargo:*)", "Bash(npm:*)", "Bash(pnpm:*)", "Bash(yarn:*)", "Bash(node:*)", "Bash(python:*)", "Bash(python3:*)",
                    "Bash(pytest:*)", "Bash(make:*)", "Bash(go:*)",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect();
                if allow_commits {
                    allowed.push("Bash(git add:*)".into());
                    allowed.push("Bash(git commit:*)".into());
                }
                for x in extra.split(',').map(str::trim).filter(|x| !x.is_empty()) {
                    allowed.push(x.to_string());
                }
                let denied = "Bash(git push:*),Bash(git reset:*),Bash(git checkout:*),Bash(git switch:*),Bash(rm -rf:*),Bash(sudo:*),Bash(curl:*),Bash(wget:*)";
                format!(
                    "claude --permission-mode acceptEdits --allowedTools {} --disallowedTools {}",
                    crate::sessions::shell_quote(&allowed.join(",")),
                    crate::sessions::shell_quote(denied)
                )
            }
        }
    }

    /// Where its session files for `cwd` are written.
    fn session_dir(self, cwd: &Path) -> PathBuf {
        match self {
            Agent::ClaudeCode => {
                let root = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"));
                let enc: String = cwd.display().to_string().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
                root.join("projects").join(enc)
            }
        }
    }
}

// ------------------------------------------------------------------ the copy of the repository

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let p = crate::sync::shell_path();
    let o = Command::new(crate::sync::which("git", &p).unwrap_or_else(|| "git".into()))
        .args(args)
        .current_dir(dir)
        .env("PATH", &p)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    } else {
        let e = String::from_utf8_lossy(&o.stderr).to_string();
        Err(e.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("hint:")).take(2).collect::<Vec<_>>().join(" "))
    }
}

pub struct Copy {
    pub root: String,
    pub worktree: String,
    pub branch: String,
    pub base: String,
}

fn slug(s: &str) -> String {
    let mut out: String = s
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c if c.is_ascii_alphanumeric() => c,
            _ => '-',
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').chars().take(30).collect::<String>().trim_matches('-').to_string()
}

/// A new worktree of the project's repository, on a branch of its own, from HEAD.
pub fn make_copy(folder: &str, title: &str) -> Result<Copy, String> {
    let dir = crate::project::normalize_folder(folder);
    let root = git(&dir, &["rev-parse", "--show-toplevel"]).map_err(|_| crate::tr!("le projet n'est pas un dépôt git : créez-le d'abord (encadré Versions)", "the project isn't a git repository: create it first (Versions card)").to_string())?.trim().to_string();
    let rd = PathBuf::from(&root);
    let base = git(&rd, &["rev-parse", "HEAD"]).map_err(|_| crate::tr!("le dépôt n'a encore aucun commit : faites un premier enregistrement", "the repository has no commit yet: make a first commit").to_string())?.trim().to_string();
    let stamp = local_stamp(crate::model::now_ms());
    let name = format!("{}-{stamp}", if slug(title).is_empty() { "projet".to_string() } else { slug(title) });
    let branch = format!("orchestra/auto-{name}");
    let wt = crate::store::data_dir().join("worktrees").join(&name);
    std::fs::create_dir_all(wt.parent().unwrap()).map_err(|e| e.to_string())?;
    git(&rd, &["worktree", "add", "-b", &branch, &wt.display().to_string(), "HEAD"])?;
    Ok(Copy { root, worktree: wt.display().to_string(), branch, base })
}

/// Send the session's branch to the remote (`origin`), so its work shows on GitHub as it
/// goes. Nothing to do without a remote or without new commits. Network: call it off the
/// interface's thread.
pub fn push_copy(worktree: &str) -> Result<bool, String> {
    let wt = PathBuf::from(worktree);
    if !wt.is_dir() || git(&wt, &["remote", "get-url", "origin"]).is_err() {
        return Ok(false);
    }
    let branch = git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        return Ok(false);
    }
    let local = git(&wt, &["rev-parse", "HEAD"])?.trim().to_string();
    let remote = git(&wt, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]).unwrap_or_default().trim().to_string();
    if local == remote {
        return Ok(false);
    }
    git(&wt, &["push", "-u", "origin", &format!("HEAD:refs/heads/{branch}")])?;
    Ok(true)
}

/// The same, in the background.
pub fn push_copy_later(worktree: String) {
    std::thread::spawn(move || {
        if let Err(e) = push_copy(&worktree) {
            eprintln!("orchestra: push of {worktree} failed: {e}");
        }
    });
}

/// The branch on the remote too, once merged or thrown away (in the background).
fn delete_remote_later(root: String, branch: String) {
    std::thread::spawn(move || {
        let rd = PathBuf::from(&root);
        if git(&rd, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]).is_ok() {
            let _ = git(&rd, &["push", "origin", "--delete", &branch]);
        }
    });
}

/// Bring the session's work into the branch checked out in the project's folder.
pub fn merge(r: &crate::model::RunRecord) -> Result<String, String> {
    let wt = PathBuf::from(&r.worktree);
    if wt.is_dir() && !git(&wt, &["status", "--porcelain"]).unwrap_or_default().trim().is_empty() {
        git(&wt, &["add", "-A"])?;
        git(&wt, &["commit", "-m", &crate::trf!("Travail autonome Orchestra · {}", "Orchestra autonomous work · {}", r.title)])?;
    }
    let rd = PathBuf::from(&r.root);
    let into = git(&rd, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default().trim().to_string();
    if let Err(e) = git(&rd, &["merge", "--no-ff", &r.branch, "-m", &crate::trf!("Intègre le travail autonome sur {}", "Brings in the autonomous work on {}", r.title)]) {
        let _ = git(&rd, &["merge", "--abort"]);
        return Err(crate::trf!("fusion impossible ({e}) ; rien n'a changé dans votre dossier", "merge failed ({e}); nothing changed in your folder"));
    }
    let _ = git(&rd, &["worktree", "remove", "--force", &r.worktree]);
    let _ = git(&rd, &["branch", "-d", &r.branch]);
    delete_remote_later(r.root.clone(), r.branch.clone());
    Ok(crate::trf!("Travail fusionné dans « {into} ».", "Work merged into « {into} »."))
}

/// Throw the session's copy and branch away.
pub fn discard(r: &crate::model::RunRecord) -> Result<String, String> {
    let rd = PathBuf::from(&r.root);
    let _ = git(&rd, &["worktree", "remove", "--force", &r.worktree]);
    let _ = git(&rd, &["worktree", "prune"]);
    git(&rd, &["branch", "-D", &r.branch])?;
    delete_remote_later(r.root.clone(), r.branch.clone());
    Ok(crate::tr!("Copie et branche supprimées.", "Copy and branch deleted.").into())
}

/// Commits of the session, and the files changed but not committed.
pub fn work_done(r: &crate::model::RunRecord, base: &str) -> (Vec<String>, usize) {
    let wt = PathBuf::from(&r.worktree);
    let commits = git(&wt, &["log", "--format=%h %s", &format!("{base}..HEAD")]).unwrap_or_default().lines().map(String::from).collect();
    let dirty = git(&wt, &["status", "--porcelain"]).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).count();
    (commits, dirty)
}

// ------------------------------------------------------------------ reading the session

/// What the session file says so far.
#[derive(Default)]
pub struct Reader {
    path: Option<PathBuf>,
    offset: u64,
    carry: String,
    seen: HashSet<String>,
    /// Billed tokens (input, output, cache writes).
    pub tokens: u64,
    /// The last record is an answer that ended its turn.
    pub turn_over: bool,
    /// Text of the last answer.
    pub last_text: String,
    /// When the file last grew.
    pub last_growth: Option<std::time::Instant>,
    pub records: usize,
}

impl Reader {
    /// Read what was added; `since`: session files older than this are not ours.
    pub fn poll(&mut self, agent: Agent, cwd: &Path, since: std::time::SystemTime) {
        if self.path.is_none() {
            let dir = agent.session_dir(cwd);
            let newest = std::fs::read_dir(&dir)
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .filter(|(m, _)| *m >= since)
                .max_by_key(|(m, _)| *m);
            self.path = newest.map(|(_, p)| p);
        }
        let Some(path) = &self.path else { return };
        let Ok(mut f) = std::fs::File::open(path) else { return };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len <= self.offset {
            return;
        }
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut buf = String::new();
        if f.read_to_string(&mut buf).is_err() {
            return;
        }
        self.offset = len;
        self.last_growth = Some(std::time::Instant::now());
        let text = std::mem::take(&mut self.carry) + &buf;
        let mut lines: Vec<&str> = text.split('\n').collect();
        self.carry = lines.pop().unwrap_or("").to_string();
        for l in lines {
            let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
            self.records += 1;
            match v["type"].as_str() {
                Some("assistant") => {
                    let m = &v["message"];
                    let id = m["id"].as_str().unwrap_or("").to_string();
                    if !id.is_empty() && self.seen.insert(id) {
                        let u = &m["usage"];
                        self.tokens += ["input_tokens", "output_tokens", "cache_creation_input_tokens"].iter().map(|k| u[k].as_u64().unwrap_or(0)).sum::<u64>();
                    }
                    let text: String = m["content"].as_array().map(|a| a.iter().filter(|c| c["type"] == "text").filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                    if !text.trim().is_empty() {
                        self.last_text = text;
                    }
                    self.turn_over = m["stop_reason"].as_str() == Some("end_turn");
                }
                Some("user") => {
                    // A tool result or a new message: the turn goes on.
                    self.turn_over = false;
                }
                _ => {}
            }
        }
    }
}

// ------------------------------------------------------------------ what the AI is told

pub fn mission_prompt(title: &str, project_id: &str, branch: &str, mission: &str, commits: bool) -> String {
    let commit_rule = if commits {
        crate::tr!("Après chaque étape terminée, fais un commit clair (git add -A puis git commit -m « … »). Ne fais jamais de git push.", "After each finished step, make a clear commit (git add -A then git commit -m « … »). Never git push.")
    } else {
        crate::tr!("Ne fais aucun commit ni git push : laisse les modifications non enregistrées, la personne les relira.", "Make no commit and no git push: leave the changes unsaved, the person will review them.")
    };
    crate::trf!("Tu travailles en autonomie pour Orchestra sur le projet « {title} » (id {project_id}). \
         Ce dossier est une copie dédiée du dépôt, sur la branche {branch} : travaille uniquement ici, ne change pas de branche.\n\
         1. Appelle get_project_state avec project = « {project_id} » pour l'état des lieux et les étapes.\n\
         2. Mission : {mission}\n\
         3. Prends les étapes ouvertes une par une : passe-la « doing » avec update_step, fais le travail, vérifie-le \
         (compilation, tests) quand c'est possible, puis passe-la « done ».\n\
         4. {commit_rule}\n\
         5. Si une décision humaine est nécessaire, appelle ask_human (project = « {project_id} ») avec une question courte, puis continue sur autre chose.\n\
         6. Ne modifie rien hors de ce dossier ; aucune commande destructrice.\n\
         Quand tu t'arrêtes pour ce tour, termine ta réponse par une seule de ces lignes : \
         [[CONTINUER]] s'il reste du travail possible, [[FINI]] si tout ce qui pouvait être fait l'est, [[BLOQUÉ]] si tu ne peux plus avancer.", "You are working autonomously for Orchestra on the project « {title} » (id {project_id}). \
         This folder is a dedicated copy of the repository, on the branch {branch}: work only here, don't switch branches.\n\
         1. Call get_project_state with project = « {project_id} » for the current state and the steps.\n\
         2. Mission: {mission}\n\
         3. Take the open steps one by one: set it to « doing » with update_step, do the work, check it \
         (build, tests) when possible, then set it to « done ».\n\
         4. {commit_rule}\n\
         5. If a human decision is needed, call ask_human (project = « {project_id} ») with a short question, then go on with something else.\n\
         6. Change nothing outside this folder; no destructive command.\n\
         When you stop for this turn, end your answer with exactly one of these lines: \
         [[CONTINUER]] if there is work left you can do, [[FINI]] if everything that could be done is done, [[BLOQUÉ]] if you can't go further."
    )
}

pub fn continue_prompt() -> &'static str {
    crate::tr!(
        "Continue la mission avec les mêmes règles (étape suivante). Termine par [[CONTINUER]], [[FINI]] ou [[BLOQUÉ]].",
        "Go on with the mission under the same rules (next step). End with [[CONTINUER]], [[FINI]] or [[BLOQUÉ]]."
    )
}

pub fn denied_prompt() -> &'static str {
    crate::tr!(
        "Cette commande n'est pas autorisée en mode autonome. Continue sans elle ; si elle est indispensable, appelle ask_human pour la demander, puis passe à autre chose. Termine par [[CONTINUER]], [[FINI]] ou [[BLOQUÉ]].",
        "This command isn't allowed in autonomous mode. Go on without it; if it is essential, call ask_human to request it, then move on to something else. End with [[CONTINUER]], [[FINI]] or [[BLOQUÉ]]."
    )
}

pub fn summary_prompt(project_id: &str, reason: &str) -> String {
    crate::trf!("La session de travail autonome s'arrête : {reason}. Ne commence rien de nouveau.\n\
         Mets à jour l'état des lieux avec update_project (project = « {project_id} »), puis réponds UNIQUEMENT \
         avec ce Markdown, en français, court :\n### Fait\n- …\n### Reste à faire\n- …\n### Blocages\n- … (ou « aucun »)", "The autonomous work session is stopping: {reason}. Don't start anything new.\n\
         Update the current state with update_project (project = « {project_id} »), then answer ONLY \
         with this Markdown, in English, short:\n### Done\n- …\n### Left to do\n- …\n### Blockers\n- … (or « none »)"
    )
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Signal {
    Continue,
    Done,
    Blocked,
}

/// The last marker of an answer.
pub fn signal(text: &str) -> Signal {
    let t = text.to_uppercase();
    let at = |k: &str| t.rfind(k);
    match [(at("[[FINI]]"), Signal::Done), (at("[[BLOQUÉ]]").or(at("[[BLOQUE]]")), Signal::Blocked), (at("[[CONTINUER]]"), Signal::Continue)]
        .into_iter()
        .filter_map(|(p, s)| p.map(|p| (p, s)))
        .max_by_key(|(p, _)| *p)
    {
        Some((_, s)) => s,
        None => Signal::Continue,
    }
}

pub fn strip_markers(text: &str) -> String {
    text.replace("[[CONTINUER]]", "").replace("[[FINI]]", "").replace("[[BLOQUÉ]]", "").replace("[[BLOQUE]]", "").trim().to_string()
}

/// What the screen of the hidden terminal shows that needs an answer.
#[derive(Debug, PartialEq)]
pub enum Screen {
    Trust,
    Permission,
    Other,
}

pub fn read_screen(text: &str) -> Screen {
    let t = text.to_lowercase();
    if t.contains("do you trust") || (t.contains("trust") && t.contains("folder") && t.contains("proceed")) {
        Screen::Trust
    } else if t.contains("do you want to proceed") || t.contains("do you want to make this edit") || t.contains("do you want to create") || t.contains("do you want to allow") {
        Screen::Permission
    } else {
        Screen::Other
    }
}

// ------------------------------------------------------------------ dates and reports

/// The local time zone's offset (`date +%z`, read once).
pub fn local_offset_ms() -> i64 {
    static OFF: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| {
        let out = Command::new("date").arg("+%z").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        let b = out.as_bytes();
        if b.len() == 5 && (b[0] == b'+' || b[0] == b'-') {
            let v = (out[1..3].parse::<i64>().unwrap_or(0) * 60 + out[3..5].parse::<i64>().unwrap_or(0)) * 60_000;
            if b[0] == b'-' { -v } else { v }
        } else {
            0
        }
    })
}

fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "2026-11-18" in local time.
pub fn local_day(ms: u64) -> String {
    let (y, m, d) = civil((ms as i64 + local_offset_ms()).div_euclid(86_400_000));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Local hour (0..23).
pub fn local_hour(ms: u64) -> u8 {
    ((ms as i64 + local_offset_ms()).rem_euclid(86_400_000) / 3_600_000) as u8
}

/// "20261118-1730" in local time.
fn local_stamp(ms: u64) -> String {
    let l = ms as i64 + local_offset_ms();
    let (y, m, d) = civil(l.div_euclid(86_400_000));
    let secs = l.rem_euclid(86_400_000) / 1000;
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

pub fn hm(ms: u64) -> String {
    let mins = (ms as i64 + local_offset_ms()).rem_euclid(86_400_000) / 60_000;
    format!("{:02}:{:02}", mins / 60, mins % 60)
}

/// The report of one work session, in Markdown.
pub fn session_report(r: &crate::model::RunRecord, commits: &[String], dirty: usize, summary: &str) -> String {
    let mins = r.ended.saturating_sub(r.started) / 60_000;
    let mut s = crate::trf!("# Rapport · {}\n\n", "# Report · {}\n\n", r.title);
    s.push_str(&crate::trf!("- **IA** : {} · {} tour(s) · {} → {} ({mins} min)\n- **Consommation** : ≈ {:.0} % de la session de 5 h · {} tokens\n- **Arrêt** : {}\n- **Branche** : `{}`\n", "- **AI**: {} · {} turn(s) · {} → {} ({mins} min)\n- **Usage**: ≈ {:.0} % of the 5-hour session · {} tokens\n- **Stopped**: {}\n- **Branch**: `{}`\n",
        r.tool,
        r.turns,
        hm(r.started),
        hm(r.ended),
        r.pct,
        crate::sessions::human_tokens(r.tokens),
        r.outcome,
        r.branch
    ));
    s.push_str(crate::tr!("\n## Étapes terminées\n\n", "\n## Steps finished\n\n"));
    if r.steps_done.is_empty() {
        s.push_str(crate::tr!("- aucune\n", "- none\n"));
    }
    for x in &r.steps_done {
        s.push_str(&format!("- {x}\n"));
    }
    s.push_str(crate::tr!("\n## Commits\n\n", "\n## Commits\n\n"));
    if commits.is_empty() {
        s.push_str(crate::tr!("- aucun\n", "- none\n"));
    }
    for c in commits.iter().take(30) {
        s.push_str(&format!("- `{c}`\n"));
    }
    if dirty > 0 {
        s.push_str(&crate::trf!("\n{dirty} fichier(s) modifié(s) non enregistré(s) dans la copie.\n", "\n{dirty} unsaved changed file(s) in the copy.\n"));
    }
    let summary = strip_markers(summary);
    if !summary.is_empty() {
        s.push_str(crate::tr!("\n## Compte rendu de l'IA\n\n", "\n## The AI's summary\n\n"));
        s.push_str(summary.trim());
        s.push('\n');
    }
    s.push_str(crate::tr!("\n## Et maintenant\n\nSur la page Orchestration : **Fusionner** pour intégrer ce travail dans votre branche, ou **Abandonner** pour le jeter.\n", "\n## What now\n\nOn the Orchestration page: **Merge** to bring this work into your branch, or **Discard** to throw it away.\n"));
    s
}

/// The day's overall report, from the sessions of the day.
pub fn daily_report(day: &str, runs: &[&crate::model::RunRecord]) -> String {
    let total: f32 = runs.iter().map(|r| r.pct).sum();
    let tokens: u64 = runs.iter().map(|r| r.tokens).sum();
    let steps: usize = runs.iter().map(|r| r.steps_done.len()).sum();
    let mut projects: Vec<&str> = runs.iter().map(|r| r.title.as_str()).collect();
    projects.sort();
    projects.dedup();
    let mut s = crate::trf!("# Bilan de la journée · {day}\n\n", "# Daily summary · {day}\n\n");
    s.push_str(&crate::trf!("- **{} projet(s)**, {} session(s) de travail, {steps} étape(s) terminée(s)\n- **Consommation** : ≈ {total:.0} % de sessions de 5 h · {} tokens\n\n", "- **{} project(s)**, {} work session(s), {steps} step(s) finished\n- **Usage**: ≈ {total:.0} % of 5-hour sessions · {} tokens\n\n",
        projects.len(),
        runs.len(),
        crate::sessions::human_tokens(tokens)
    ));
    for p in projects {
        s.push_str(&format!("## {p}\n\n"));
        for r in runs.iter().filter(|r| r.title == p) {
            s.push_str(&crate::trf!("- {} → {} · {} · ≈ {:.0} % · {} étape(s)\n", "- {} → {} · {} · ≈ {:.0} % · {} step(s)\n", hm(r.started), hm(r.ended), r.outcome, r.pct, r.steps_done.len()));
            for x in &r.steps_done {
                s.push_str(&format!("  - {x}\n"));
            }
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signals_and_screen() {
        assert_eq!(signal("fait.\n[[FINI]]"), Signal::Done);
        assert_eq!(signal("[[CONTINUER]] puis… [[BLOQUÉ]]"), Signal::Blocked);
        assert_eq!(signal("rien"), Signal::Continue);
        assert_eq!(read_screen("Do you trust the files in this folder?\n❯ 1. Yes, proceed"), Screen::Trust);
        assert_eq!(read_screen("Bash command\n rm x\nDo you want to proceed?"), Screen::Permission);
        assert_eq!(slug("Démo — Été 2026!"), "demo-ete-2026");
        assert!(Agent::ClaudeCode.command(false, "").contains("--permission-mode acceptEdits"));
        assert!(!Agent::ClaudeCode.command(false, "").contains("git commit"));
        assert!(Agent::ClaudeCode.command(true, "Bash(just:*)").contains("Bash(just:*)"));
    }

    #[test]
    fn copy_merge_discard() {
        let repo = std::env::temp_dir().join(format!("orch-auto-repo-{}", std::process::id()));
        std::fs::create_dir_all(&repo).unwrap();
        let ok = |d: &Path, a: &[&str]| git(d, a).is_ok();
        if !ok(&repo, &["init", "-b", "main"]) {
            return;
        }
        let _ = git(&repo, &["config", "user.email", "t@t"]);
        let _ = git(&repo, &["config", "user.name", "T"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        assert!(ok(&repo, &["add", "-A"]) && ok(&repo, &["commit", "-m", "a"]));
        let c = make_copy(&repo.display().to_string(), "Essai").unwrap();
        let wt = PathBuf::from(&c.worktree);
        std::fs::write(wt.join("b.txt"), "b").unwrap();
        let rec = crate::model::RunRecord { title: "Essai".into(), root: c.root.clone(), worktree: c.worktree.clone(), branch: c.branch.clone(), ..Default::default() };
        let (commits, dirty) = work_done(&rec, &c.base);
        assert_eq!((commits.len(), dirty), (0, 1));
        merge(&rec).unwrap();
        assert!(repo.join("b.txt").exists());
        assert!(!wt.exists());
        let c2 = make_copy(&repo.display().to_string(), "Essai deux").unwrap();
        let rec2 = crate::model::RunRecord { root: c2.root.clone(), worktree: c2.worktree.clone(), branch: c2.branch.clone(), ..Default::default() };
        discard(&rec2).unwrap();
        assert!(!git(&repo, &["branch"]).unwrap().contains("orchestra/"));
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn reader_counts_and_ends() {
        let dir = std::env::temp_dir().join(format!("orch-auto-{}", std::process::id()));
        let cwd = dir.join("wt");
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir.join("claude")) };
        let sd = Agent::ClaudeCode.session_dir(&cwd);
        std::fs::create_dir_all(&sd).unwrap();
        let since = std::time::SystemTime::now() - std::time::Duration::from_secs(5);
        let f = sd.join("s.jsonl");
        std::fs::write(&f, concat!(
            "{\"type\":\"user\",\"message\":{\"content\":\"go\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"stop_reason\":\"tool_use\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5},\"content\":[]}}\n",
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\"}]}}\n",
            "{\"type\":\"assistant\",\"message\":{\"id\":\"m2\",\"stop_reason\":\"end_turn\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2},\"content\":[{\"type\":\"text\",\"text\":\"ok [[FINI]]\"}]}}\n"
        ))
        .unwrap();
        let mut r = Reader::default();
        r.poll(Agent::ClaudeCode, &cwd, since);
        unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") };
        assert!(r.turn_over);
        assert_eq!(r.tokens, 20);
        assert_eq!(signal(&r.last_text), Signal::Done);
        let _ = std::fs::remove_dir_all(dir);
    }
}
