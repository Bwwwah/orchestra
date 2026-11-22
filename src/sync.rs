//! "Synchroniser avec les IA": ask the coding agents installed on this machine (Claude Code,
//! Codex) to plug themselves into the dashboard's MCP server and write the state of play of
//! each of their projects.
//!
//! For every chosen folder, one agent runs headless *in that folder*, read-only (it may read
//! files and run `git log` / `git status`, nothing else), and talks to the dashboard only
//! through the MCP tools (get_project_state, create_project / update_project, add_steps).
//! Runs go one after the other, so two agents never write the workspace at the same time.
//! It uses the person's own Claude / Codex subscription, as if they had typed the request.

use crate::LockOrRecover;
use crate::sessions::Tool;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// API keys found in the environment make Claude Code / Codex bill API credits instead of
/// the person's subscription ("Credit balance is too low"): agents launched from the
/// dashboard never get them, so they use the account the person logged in with.
pub const API_KEY_VARS: [&str; 3] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "OPENAI_API_KEY",
];

/// What a Claude Code session puts in the environment of the programs it runs. Orchestra
/// started from one (`cargo run` in a Claude Code terminal) would hand them down: a Claude
/// Code started with them takes itself for that session's child and saves no transcript,
/// so an autonomous session never sees its turns end. Never passed on; the person's own
/// settings (CLAUDE_CONFIG_DIR, CLAUDE_CODE_USE_BEDROCK…) are.
pub const HOST_SESSION_VARS: [&str; 12] = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_CHILD_NOTES_DIR",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "AI_AGENT",
];

/// Longest a single project may take before the agent is stopped.
const PER_PROJECT: Duration = Duration::from_secs(8 * 60);

#[derive(Clone, Debug)]
pub struct Agent {
    pub tool: Tool,
    pub bin: PathBuf,
}

/// The PATH of the person's own shell. Apps opened from the Finder get a bare PATH, where
/// `claude` (often installed with npm / nvm) and `node` are missing.
///
/// Starting the shell takes about a second and git alone asks for the PATH a dozen times to
/// read one repository: the shell is started once per process. Threads asking at the same
/// time wait for that one answer rather than each starting a shell. A program installed
/// later in a folder already on the PATH is still found (`which` looks at the folders).
pub fn shell_path() -> String {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(read_shell_path).clone()
}

#[cfg(all(test, unix))]
#[test]
fn shell_path_is_read_once_and_shared() {
    let start = Instant::now();
    let a = shell_path();
    let first = start.elapsed();
    let t = Instant::now();
    let others: Vec<String> = (0..8)
        .map(|_| std::thread::spawn(shell_path))
        .map(|h| h.join().unwrap())
        .collect();
    assert!(
        others.iter().all(|p| *p == a),
        "every caller sees the same PATH"
    );
    // The first call may start a shell; the next ones must not.
    assert!(
        t.elapsed() < Duration::from_millis(50),
        "later calls took {:?} (first {first:?})",
        t.elapsed()
    );
    assert!(!a.is_empty());
}

#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> i32;
}

fn read_shell_path() -> String {
    let mut parts: Vec<PathBuf> = vec![];
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let mut cmd = crate::proc::command(&shell);
        cmd.args(["-lic", "printf '__PATH__%s\\n' \"$PATH\""])
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        // An interactive shell takes over its terminal (job control). When Orchestra runs as
        // the MCP server of a Claude Code session, that terminal is Claude Code's, which
        // then loses it and quits in the middle of its turn. In a session of its own, the
        // shell has no terminal to take.
        // SAFETY: setsid is async-signal-safe and touches no memory of the parent.
        unsafe {
            cmd.pre_exec(|| {
                setsid();
                Ok(())
            });
        }
        let out = cmd.output();
        let from_shell = out
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("__PATH__").map(String::from))
            })
            .unwrap_or_default();
        parts.extend(std::env::split_paths(&from_shell).filter(|p| !p.as_os_str().is_empty()));
    }
    let home = dirs::home_dir().unwrap_or_default();
    let mut extras = vec![
        home.join(".local/bin"),
        home.join(".claude/local"),
        home.join(".npm-global/bin"),
    ];
    if cfg!(windows) {
        if let Some(d) = dirs::data_dir() {
            extras.push(d.join("npm"));
        }
    } else {
        extras.push(PathBuf::from("/opt/homebrew/bin"));
        extras.push(PathBuf::from("/usr/local/bin"));
    }
    for e in extras {
        if !parts.contains(&e) {
            parts.push(e);
        }
    }
    for p in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if !p.as_os_str().is_empty() && !parts.contains(&p) {
            parts.push(p);
        }
    }
    std::env::join_paths(parts)
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// A program on this PATH (with `.exe` / `.cmd` on Windows).
pub fn which(name: &str, path: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    std::env::split_paths(path).find_map(|d| {
        exts.iter()
            .map(|e| d.join(format!("{name}{e}")))
            .find(|p| p.is_file())
    })
}

/// Claude Code and Codex, when installed.
pub fn detect(path: &str) -> Vec<Agent> {
    [(Tool::ClaudeCode, "claude"), (Tool::Codex, "codex")]
        .into_iter()
        .filter_map(|(tool, name)| which(name, path).map(|bin| Agent { tool, bin }))
        .collect()
}

/// Make sure the agent knows the dashboard's MCP server (added once, for every folder).
pub fn ensure_mcp(agent: &Agent, exe: &Path, path: &str) -> Result<&'static str, String> {
    use crate::setup::Outcome;
    match crate::setup::plug_cli(agent, exe, path) {
        Outcome::AlreadyThere => Ok("déjà branché"),
        Outcome::Added | Outcome::Updated => Ok("branché"),
        Outcome::Failed(e) => Err(e),
    }
}

/// What the agent is asked, in the project's folder.
pub fn prompt(folder: &str) -> String {
    crate::trf!(
        "Tu es connecté au tableau de bord « orchestra » par MCP. Le projet est dans le dossier « {folder} ».\n\
         1. Appelle get_project_state avec folder = « {folder} » pour voir s'il existe déjà.\n\
         2. Examine rapidement le projet SANS RIEN MODIFIER : README, structure des dossiers, `git log --oneline -20`, `git status`.\n\
         3. S'il n'existe pas, appelle create_project (title : nom court et lisible du projet, folder : « {folder} ») ; sinon update_project.\n\
            Donne status, et l'état des lieux avec les listes done, in_progress et blockers : 3 à 6 éléments chacune au plus, une phrase courte par élément, sans Markdown.\n\
         Ne modifie aucun fichier, ne lance aucune autre commande. Termine en répondant seulement « ok ».",
        "You are connected to the « orchestra » dashboard through MCP. The project is in the folder « {folder} ».\n\
         1. Call get_project_state with folder = « {folder} » to see if it already exists.\n\
         2. Look quickly at the project WITHOUT CHANGING ANYTHING: README, folder structure, `git log --oneline -20`, `git status`.\n\
         3. If it doesn't exist, call create_project (title: short readable name of the project, folder: « {folder} »); otherwise update_project.\n\
            Give the status, and the current state with the done, in_progress and blockers lists: 3 to 6 items each at most, one short sentence per item, no Markdown. Write in English.\n\
         Don't change any file, don't run any other command. End by answering only « ok »."
    )
}

fn agent_args(tool: Tool, prompt: &str) -> Vec<String> {
    match tool {
        Tool::ClaudeCode => vec![
            "-p".into(),
            prompt.into(),
            "--output-format".into(),
            "json".into(),
            "--max-turns".into(),
            "40".into(),
            // Read-only: reading tools, two git commands, and the dashboard's MCP tools.
            "--allowedTools".into(),
            "mcp__orchestra,Read,Glob,Grep,Bash(git log:*),Bash(git status:*),Bash(ls:*)".into(),
            // The steps are the person's: the AI proposes them only when asked.
            "--disallowedTools".into(),
            "mcp__orchestra__add_steps".into(),
        ],
        Tool::Codex => vec![
            "exec".into(),
            "--sandbox".into(),
            "read-only".into(),
            "--skip-git-repo-check".into(),
            prompt.into(),
        ],
    }
}

fn last_line(s: &str) -> Option<String> {
    s.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| crate::richtext::one_line(l, 160))
}

/// Run one agent in one folder; stops it after `PER_PROJECT` or when `cancel` is set.
/// Run an agent headless in `folder` with these arguments. Returns its final answer and a
/// short report ("terminé en 42 s"), or an error people can act on.
fn run_agent(
    agent: &Agent,
    folder: &str,
    args: Vec<String>,
    path: &str,
    cancel: &AtomicBool,
    limit: Duration,
) -> Result<(String, String, Option<String>), String> {
    let dir = crate::project::normalize_folder(folder);
    if !dir.is_dir() {
        return Err(crate::tr!("dossier introuvable", "folder not found").into());
    }
    let mut cmd = crate::proc::command(&agent.bin);
    for k in API_KEY_VARS.iter().chain(&HOST_SESSION_VARS) {
        cmd.env_remove(k);
    }
    let mut child = cmd
        .args(args)
        .current_dir(&dir)
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    // Read the pipes in threads so a chatty agent can't block on a full pipe.
    let read = |r: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut r) = r {
                let _ = r.read_to_string(&mut s);
            }
            s
        })
    };
    let out_t = read(
        child
            .stdout
            .take()
            .map(|x| Box::new(x) as Box<dyn std::io::Read + Send>),
    );
    let err_t = read(
        child
            .stderr
            .take()
            .map(|x| Box::new(x) as Box<dyn std::io::Read + Send>),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        if cancel.load(Ordering::Relaxed) || start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if cancel.load(Ordering::Relaxed) {
                crate::tr!("arrêté", "stopped").into()
            } else {
                crate::tr!("trop long, arrêté", "too long, stopped").into()
            });
        }
        std::thread::sleep(Duration::from_millis(300));
    };
    let out = out_t.join().unwrap_or_default();
    let err = err_t.join().unwrap_or_default();
    let low = format!("{out}{err}").to_lowercase();
    if agent.tool == Tool::ClaudeCode && is_auth_error(&low) {
        return Err(AUTH_ERROR.into());
    }
    if low.contains("credit balance is too low") {
        return Err(crate::tr!("Claude Code est connecté avec une clé API sans crédit, pas à votre abonnement : lancez `claude`, puis `/login` et choisissez votre compte Claude (Pro / Max).", "Claude Code is signed in with an API key without credit, not your subscription: run `claude`, then `/login` and choose your Claude account (Pro / Max).").into());
    }
    if agent.tool == Tool::ClaudeCode
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(out.trim())
    {
        let result = v
            .get("result")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if v.get("is_error").and_then(|x| x.as_bool()) == Some(true) {
            return Err(crate::richtext::one_line(
                if result.is_empty() { "erreur" } else { &result },
                160,
            ));
        }
        let turns = v.get("num_turns").and_then(|x| x.as_u64()).unwrap_or(0);
        let session = v
            .get("session_id")
            .and_then(|x| x.as_str())
            .filter(|s| is_session_id(s))
            .map(String::from);
        return Ok((
            result,
            crate::trf!(
                "terminé en {} s, {turns} échanges",
                "done in {} s, {turns} exchanges",
                start.elapsed().as_secs()
            ),
            session,
        ));
    }
    if status.success() {
        let session = if agent.tool == Tool::Codex {
            codex_session_id(&err).or_else(|| codex_session_id(&out))
        } else {
            None
        };
        Ok((
            out.trim().to_string(),
            crate::trf!("terminé en {} s", "done in {} s", start.elapsed().as_secs()),
            session,
        ))
    } else {
        Err(last_line(&err)
            .or_else(|| last_line(&out))
            .unwrap_or_else(|| format!("code {}", status.code().unwrap_or(-1))))
    }
}

/// Claude Code run headless (`claude -p`) sometimes can't renew its sign-in, while the
/// interactive Claude Code renews it fine (a known Claude Code issue). Errors starting with
/// this text offer the two ways out: sign in again, or ask in the terminal.
pub const AUTH_ERROR: &str = "Claude Code n'a pas pu renouveler sa connexion en mode automatique (souci connu de Claude Code). Reconnectez-vous, ou posez la question dans le terminal intégré.";

/// The message to show for AUTH_ERROR, in the interface's language.
pub fn auth_error_text() -> &'static str {
    crate::tr!(
        "Claude Code n'a pas pu renouveler sa connexion en mode automatique (souci connu de Claude Code). Reconnectez-vous, ou posez la question dans le terminal intégré.",
        "Claude Code couldn't renew its sign-in in automatic mode (a known Claude Code issue). Sign in again, or ask the question in the built-in terminal."
    )
}

fn is_auth_error(low: &str) -> bool {
    [
        "oauth",
        "authentication_error",
        "failed to authenticate",
        "not logged in",
        "please run /login",
        "invalid api key",
    ]
    .iter()
    .any(|k| low.contains(k))
}

/// Synchronise one folder (see `prompt`).
fn run_one(agent: &Agent, folder: &str, path: &str, cancel: &AtomicBool) -> Result<String, String> {
    run_agent(
        agent,
        folder,
        agent_args(agent.tool, &prompt(folder)),
        path,
        cancel,
        PER_PROJECT,
    )
    .map(|(_, report, _)| report)
}

/// A conversation id an agent gave back, safe to put on a command line: letters, digits
/// and dashes only (Claude Code and Codex use UUIDs).
fn is_session_id(s: &str) -> bool {
    (8..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The conversation id `codex exec` prints in its header (« session id: … »).
fn codex_session_id(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let l = l.trim();
        let rest = l
            .strip_prefix("session id:")
            .or_else(|| l.strip_prefix("session_id:"))?;
        let id = rest.trim();
        is_session_id(id).then(|| id.to_string())
    })
}

/// The command that reopens an agent's conversation in a terminal, to carry it on.
pub fn resume_command(tool: Tool, session: &str) -> Option<String> {
    if !is_session_id(session) {
        return None;
    }
    Some(match tool {
        Tool::ClaudeCode => format!("claude --resume {session}"),
        Tool::Codex => format!("codex resume {session}"),
    })
}

/// What the agent is asked for the "Regard extérieur" card. `code`: a code project
/// (similar products, features); otherwise documents — school work, paperwork or anything
/// else — where useful resources and next actions replace similar projects.
pub fn review_prompt(folder: &str, code: bool) -> String {
    if !code {
        return crate::trf!(
            "Donne un regard extérieur sur le projet du dossier « {folder} » : un projet sans code (travail scolaire, \
             démarche administrative ou autre). Parcours rapidement ses documents SANS RIEN MODIFIER ; si l'outil \
             get_project_state du serveur orchestra est disponible, appelle-le avec folder = « {folder} » pour connaître \
             l'état des lieux. Si tu peux chercher sur le web, cherche les ressources fiables qui aideraient (sites \
             officiels, guides, modèles, échéances).\n\
             Réponds UNIQUEMENT avec ce Markdown, en français, avec des mots simples et sans jargon technique, condensé \
             (900 caractères au plus en tout, pas de tableau, pas d'introduction ni de conclusion) :\n\
             ### Avis\n- 2 ou 3 puces : ce qui est bien avancé et ce qui manque ou risque de coincer, sans complaisance\n\
             ### Ressources utiles\n- 2 ou 3 puces : nom — à quoi elle sert ici (écris « rien de notable » si tu n'en vois pas)\n\
             ### Suggestions\n- 3 ou 4 puces : prochaines actions concrètes, la plus utile ou la plus urgente d'abord",
            "Give an outside view of the project in the folder « {folder} »: a project without code (school work, \
             paperwork or anything else). Look quickly through its documents WITHOUT CHANGING ANYTHING; if the orchestra \
             server's get_project_state tool is available, call it with folder = « {folder} » to know the current state. \
             If you can search the web, look for reliable resources that would help (official sites, guides, templates, \
             deadlines).\n\
             Answer ONLY with this Markdown, in English, in plain words without technical jargon, condensed (900 \
             characters at most in all, no table, no introduction or conclusion):\n\
             ### Opinion\n- 2 or 3 bullets: what is well under way and what is missing or may get stuck, frankly\n\
             ### Useful resources\n- 2 or 3 bullets: name — what it is for here (write « nothing notable » if you see none)\n\
             ### Suggestions\n- 3 or 4 bullets: concrete next actions, the most useful or urgent first"
        );
    }
    crate::trf!(
        "Donne un regard extérieur sur le projet du dossier « {folder} ». Lis-le rapidement SANS RIEN MODIFIER \
         (README, structure, `git log --oneline -15`) ; si l'outil get_project_state du serveur orchestra est disponible, \
         appelle-le avec folder = « {folder} » pour connaître l'état des lieux. Si tu peux chercher sur le web, regarde \
         quels projets ou produits similaires existent ou sont apparus récemment.\n\
         Réponds UNIQUEMENT avec ce Markdown, en français, condensé (900 caractères au plus en tout, pas de tableau, pas d'introduction ni de conclusion) :\n\
         ### Avis\n- 2 ou 3 puces : forces et faiblesses, sans complaisance\n\
         ### Projets similaires\n- 2 ou 3 puces : nom — en quoi il ressemble ou diffère (écris « rien de notable » si tu n'en vois pas)\n\
         ### Suggestions\n- 3 ou 4 puces : fonctionnalités ou actions concrètes pour la suite, la plus utile d'abord",
        "Give an outside view of the project in the folder « {folder} ». Read it quickly WITHOUT CHANGING ANYTHING \
         (README, structure, `git log --oneline -15`); if the orchestra server's get_project_state tool is available, \
         call it with folder = « {folder} » to know the current state. If you can search the web, look at \
         which similar projects or products exist or appeared recently.\n\
         Answer ONLY with this Markdown, in English, condensed (900 characters at most in all, no table, no introduction or conclusion):\n\
         ### Opinion\n- 2 or 3 bullets: strengths and weaknesses, frankly\n\
         ### Similar projects\n- 2 or 3 bullets: name — how it is alike or different (write « nothing notable » if you see none)\n\
         ### Suggestions\n- 3 or 4 bullets: concrete features or actions for what's next, the most useful first"
    )
}

/// What an AI writes into a project's cards (all but the steps).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fill {
    pub status: Option<crate::project::ProjectStatus>,
    pub done: Vec<String>,
    pub doing: Vec<String>,
    pub blockers: Vec<String>,
    pub notes: String,
    /// "Regard extérieur" Markdown.
    pub review: String,
}

/// `code`: see `review_prompt`; a project without code gets a plain-words review with
/// useful resources in place of similar projects.
pub fn fill_prompt(folder: &str, code: bool) -> String {
    let base = fill_prompt_code(folder);
    if code {
        return base;
    }
    base + crate::tr!(
        "\nCe projet n'est pas du code (travail scolaire, démarche administrative ou autre) : parcours ses documents plutôt \
         que l'historique git, écris avec des mots simples sans jargon technique, et dans review remplace « ### Projets similaires » \
         par « ### Ressources utiles » (2 ou 3 puces « nom — à quoi elle sert ici » : sites officiels, guides, modèles), \
         les suggestions étant les prochaines actions concrètes.",
        "\nThis project isn't code (school work, paperwork or anything else): look through its documents rather than the git \
         history, write in plain words without technical jargon, and in review replace « ### Similar projects » with \
         « ### Useful resources » (2 or 3 bullets « name — what it is for here »: official sites, guides, templates), \
         the suggestions being the concrete next actions."
    )
}

fn fill_prompt_code(folder: &str) -> String {
    crate::trf!(
        "Remplis la fiche du projet du dossier « {folder} » dans un tableau de bord. Lis-le rapidement SANS RIEN MODIFIER \
         (README, structure des dossiers, `git log --oneline -20`, `git status`). Si tu peux chercher sur le web, regarde quels \
         projets ou produits similaires existent.\n\
         Réponds UNIQUEMENT avec un objet JSON, en français, sans bloc de code :\n\
         {{\"status\": \"idea|active|paused|done\", \"done\": [\"…\"], \"in_progress\": [\"…\"], \"blockers\": [\"…\"], \"notes\": \"…\", \"review\": \"…\"}}\n\
         - done / in_progress / blockers : l'état des lieux, 0 à 6 éléments chacun, une phrase courte par élément, sans Markdown ;\n\
         - notes : une ou deux phrases sur ce qu'est le projet (peut être vide) ;\n\
         - review : un regard extérieur en Markdown, 900 caractères au plus, avec exactement ces sections : « ### Avis » (2 ou 3 puces, forces et faiblesses, sans complaisance), \
         « ### Projets similaires » (2 ou 3 puces « nom — en quoi il ressemble ou diffère », ou « rien de notable »), « ### Suggestions » (3 ou 4 puces, la plus utile d'abord).\n\
         Ne propose pas de liste de tâches.",
        "Fill in the card of the project in the folder « {folder} » in a dashboard. Read it quickly WITHOUT CHANGING ANYTHING \
         (README, folder structure, `git log --oneline -20`, `git status`). If you can search the web, look at which \
         similar projects or products exist.\n\
         Answer ONLY with a JSON object, in English, no code block:\n\
         {{\"status\": \"idea|active|paused|done\", \"done\": [\"…\"], \"in_progress\": [\"…\"], \"blockers\": [\"…\"], \"notes\": \"…\", \"review\": \"…\"}}\n\
         - done / in_progress / blockers: the current state, 0 to 6 items each, one short sentence per item, no Markdown;\n\
         - notes: one or two sentences on what the project is (may be empty);\n\
         - review: an outside view in Markdown, 900 characters at most, with exactly these sections: « ### Opinion » (2 or 3 bullets, strengths and weaknesses, frankly), \
         « ### Similar projects » (2 or 3 bullets « name — how it is alike or different », or « nothing notable »), « ### Suggestions » (3 or 4 bullets, the most useful first).\n\
         Don't propose a task list."
    )
}

/// The JSON object in an answer (a code block or words around it are tolerated).
pub fn parse_fill(answer: &str) -> Result<Fill, String> {
    let (Some(a), Some(b)) = (answer.find('{'), answer.rfind('}')) else {
        return Err(crate::tr!("réponse de l'IA illisible", "unreadable AI answer").into());
    };
    let v: serde_json::Value = serde_json::from_str(&answer[a..=b])
        .map_err(|_| crate::tr!("réponse de l'IA illisible", "unreadable AI answer").to_string())?;
    let list = |k: &str| -> Vec<String> {
        v.get(k)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .take(8)
                    .collect()
            })
            .unwrap_or_default()
    };
    let text = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    Ok(Fill {
        status: crate::project::ProjectStatus::from_name(&text("status")),
        done: list("done"),
        doing: list("in_progress"),
        blockers: list("blockers"),
        notes: text("notes"),
        review: text("review"),
    })
}

/// Ask an agent to fill in a project's cards (read-only, web search allowed).
/// `code`: the project kind forced by the person (None: detected).
pub fn ask_fill(
    folder: &str,
    code: Option<bool>,
    prefer: Option<Tool>,
) -> Result<(Fill, Tool), String> {
    let (answer, tool) = ask(
        folder,
        fill_prompt(folder, crate::project::code_kind(folder, code)),
        "Read,Glob,Grep,Bash(git log:*),Bash(git status:*),Bash(ls:*),WebSearch,WebFetch",
        30,
        prefer,
    )?;
    Ok((parse_fill(&answer)?, tool))
}

/// Task titles from an answer: one per "- " / "* " / "1. " line.
pub fn parse_tasks(answer: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for l in answer.lines() {
        let l = l.trim();
        let rest = l
            .strip_prefix("- [ ] ")
            .or_else(|| l.strip_prefix("- "))
            .or_else(|| l.strip_prefix("* "))
            .or_else(|| l.strip_prefix("• "))
            .or_else(|| {
                let d = l.find(|c: char| !c.is_ascii_digit())?;
                (d > 0)
                    .then(|| {
                        l[d..]
                            .strip_prefix(". ")
                            .or_else(|| l[d..].strip_prefix(") "))
                    })
                    .flatten()
            });
        if let Some(t) = rest {
            let t = t.trim().trim_matches('*').trim();
            if !t.is_empty() && !out.iter().any(|o| o == t) {
                out.push(crate::richtext::one_line(t, crate::project::STEP_MAX));
            }
        }
    }
    out.truncate(25);
    out
}

/// The steps of an AI's answer that follow the AI step format (`project::ai_step`), tidied;
/// the others are dropped rather than proposed.
pub fn ai_steps_only(steps: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for s in steps {
        if let Ok(s) = crate::project::ai_step(&s)
            && !out.contains(&s)
        {
            out.push(s);
        }
    }
    out
}

/// Next tasks for a project, proposed by an AI that reads it (read-only). `existing`: the
/// steps already there, not to repeat.
pub fn ask_tasks(
    folder: &str,
    existing: &[String],
    prefer: Option<Tool>,
) -> Result<Vec<String>, String> {
    let have = if existing.is_empty() {
        crate::tr!("(aucune)", "(none)").to_string()
    } else {
        existing
            .iter()
            .map(|e| format!("- {e}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    // These steps are what the AIs work on when they run on their own (Orchestration page):
    // each must be a real piece of work, precise enough to be done and checked without a human.
    let prompt = crate::trf!(
        "Tu prépares le travail d'une IA qui va avancer SEULE sur le projet du dossier « {folder} ». \
         Lis-le SANS RIEN MODIFIER, et lis vraiment le code, pas seulement le README : structure, manifestes \
         (Cargo.toml, package.json…), les fichiers source principaux, les tests, `git log --oneline -15`, `git status`.\n\
         Propose ensuite 4 à 8 étapes, réparties entre ces trois domaines :\n\
         - Fonctionnalité : une fonction utile qui manque ou qui est inachevée, vue dans le code ;\n\
         - Technique : un bug probable, une fragilité, un manque de tests sur du code critique, une lenteur, du code dupliqué ;\n\
         - Sécurité : secrets en clair, entrées non validées, commandes shell construites à partir d'entrées, permissions, dépendances à risque.\n\
         Chaque étape doit pouvoir être faite et vérifiée par une IA seule en une session : précise, concrète, \
         elle nomme le fichier, le module ou la fonction concernés. \
         Interdit : les étapes vagues (« améliorer l'UX », « optimiser le code »), la doc ou le README, les tâches de routine \
         (commit, push, publier une version), et ce qui demande une décision ou un accès humain. \
         Ne répète pas les étapes déjà là :\n{have}\n\n\
         Réponds UNIQUEMENT par la liste, la plus importante d'abord, une étape par ligne au format \
         « - Domaine : action à l'infinitif (fichier ou module) », 140 caractères au plus. \
         Exemple : « - Sécurité : échapper le nom de dossier passé à la commande git (src/git.rs) ».",
        "You are preparing the work of an AI that will move the project in the folder « {folder} » forward ON ITS OWN. \
         Read it WITHOUT CHANGING ANYTHING, and really read the code, not just the README: structure, manifests \
         (Cargo.toml, package.json…), the main source files, the tests, `git log --oneline -15`, `git status`.\n\
         Then propose 4 to 8 steps, spread across these three areas:\n\
         - Feature: a useful function that is missing or unfinished, seen in the code;\n\
         - Technical: a likely bug, a fragile spot, missing tests on critical code, something slow, duplicated code;\n\
         - Security: secrets in clear, unchecked input, shell commands built from input, permissions, risky dependencies.\n\
         Each step must be doable and checkable by an AI alone in one session: precise, concrete, \
         naming the file, module or function concerned. \
         Not allowed: vague steps (« improve the UX », « optimise the code »), docs or README, routine chores \
         (commit, push, publish a release), and anything needing a human decision or access. \
         Don't repeat the steps already there:\n{have}\n\n\
         Answer ONLY with the list, the most important first, one step per line as \
         « - Area: action (file or module) », 140 characters at most. \
         Example: « - Security: escape the folder name passed to the git command (src/git.rs) »."
    );
    let (answer, _) = ask(
        folder,
        prompt,
        "Read,Glob,Grep,Bash(git log:*),Bash(git status:*),Bash(ls:*)",
        40,
        prefer,
    )?;
    let tasks = ai_steps_only(parse_tasks(&answer));
    if tasks.is_empty() {
        return Err(crate::tr!(
            "l'IA n'a proposé aucune étape au format « Thème : action »",
            "the AI proposed no step written « Theme: action »"
        )
        .into());
    }
    Ok(tasks)
}

/// How much thinking a question needs, to spend no more of the subscription than it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    /// Putting given material into shape (a text into tasks, a diff into a commit message,
    /// commits into release notes): a light model (Haiku…) does it as well, for less.
    Light,
    /// Reading and judging a project (filling it in, proposing steps, an outside view, a
    /// search): the person's own default model.
    Full,
    /// Answering another AI's question from the whole project, where a wrong answer sends a
    /// session the wrong way: the cheapest high-end model (`catalog::strong`), high effort.
    Strong,
}

/// The command line for one question to an agent. Images: Codex takes them with `--image`;
/// Claude Code reads them itself, from their folders added with `--add-dir`. A light
/// or strong question runs on `model` (chosen by the catalog, see `weight_model`),
/// with low (light) or high (strong) reasoning effort on Codex; without one the agent keeps
/// its default.
fn ask_args(
    tool: Tool,
    prompt: String,
    claude_tools: &str,
    max_turns: u32,
    images: &[String],
    weight: Weight,
    model: Option<&str>,
) -> Vec<String> {
    let model = model
        .filter(|_| weight != Weight::Full)
        .and_then(crate::auto::model_arg);
    match tool {
        Tool::ClaudeCode => {
            let mut a: Vec<String> = vec![
                "-p".into(),
                prompt,
                "--output-format".into(),
                "json".into(),
                "--max-turns".into(),
                max_turns.to_string(),
                "--allowedTools".into(),
                claude_tools.into(),
            ];
            if let Some(m) = model {
                a.push("--model".into());
                a.push(m.into());
            }
            let mut dirs: Vec<String> = vec![];
            for i in images {
                if let Some(d) = Path::new(i)
                    .parent()
                    .map(|d| d.display().to_string())
                    .filter(|d| !d.is_empty())
                    && !dirs.contains(&d)
                {
                    dirs.push(d);
                }
            }
            for d in dirs {
                a.push("--add-dir".into());
                a.push(d);
            }
            a
        }
        Tool::Codex => {
            let mut a: Vec<String> = vec![
                "exec".into(),
                "--sandbox".into(),
                "read-only".into(),
                "--skip-git-repo-check".into(),
            ];
            let effort = match weight {
                Weight::Light => Some("low"),
                Weight::Strong => Some("high"),
                Weight::Full => None,
            };
            if let Some(e) = effort {
                a.push("-c".into());
                a.push(format!("model_reasoning_effort=\"{e}\""));
            }
            if let Some(m) = model {
                a.push("--model".into());
                a.push(m.into());
            }
            for i in images {
                a.push(format!("--image={i}"));
            }
            a.push("--".into());
            a.push(prompt);
            a
        }
    }
}

/// The model a question of `weight` runs on, chosen by the catalog: a light one the cheapest
/// light model `tool` can be started with (Claude Code: Haiku; Codex: a mini model of its own
/// list), a strong one the cheapest high-end model (Claude Code: Opus). None: it keeps its
/// default.
fn weight_model(tool: Tool, weight: Weight) -> Option<String> {
    let agent = match tool {
        Tool::ClaudeCode => crate::auto::Agent::ClaudeCode,
        Tool::Codex => crate::auto::Agent::Codex,
    };
    let models = crate::catalog::available(agent);
    match weight {
        Weight::Light => crate::catalog::light(&models),
        Weight::Strong => crate::catalog::strong(&models),
        Weight::Full => None,
    }
    .map(|m| m.id.clone())
}

/// The image files a person may join to a text (by their extension).
pub fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp"
        )
    })
}

/// A pasted text (notes, an email, a list…) and joined images (screenshots…) turned into
/// tasks by an AI. It reads nothing else.
pub fn text_to_tasks(
    folder: &str,
    text: &str,
    images: &[String],
    prefer: Option<Tool>,
) -> Result<Vec<String>, String> {
    let text = if text.trim().is_empty() {
        crate::tr!(
            "(pas de texte : seulement les images)",
            "(no text: only the images)"
        )
        .to_string()
    } else {
        text.to_string()
    };
    let tools_rule = if images.is_empty() {
        crate::tr!("N'utilise aucun outil.", "Don't use any tool.").to_string()
    } else {
        let list = images
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        crate::trf!(
            "Des images sont jointes (captures d'écran, photos…) : regarde-les (outil Read sur chaque chemin) et tiens-en compte autant que du texte. N'utilise aucun autre outil.\nImages :\n{list}",
            "Images are joined (screenshots, photos…): look at them (Read tool on each path) and take them into account as much as the text. Don't use any other tool.\nImages:\n{list}"
        )
    };
    let prompt = crate::trf!(
        "Transforme ce texte en tâches claires et actionnables pour un projet : regroupe ce qui va ensemble, retire les doublons \
         et le bavardage, garde l'ordre logique. Chaque tâche s'écrit OBLIGATOIREMENT « Thème : action à l'infinitif » : \
         un thème court (1 à 4 mots), deux-points, puis UNE seule action (pas de « ; », pas de « puis », pas de seconde phrase ; \
         deux actions = deux tâches). Exemple : « Inscription : envoyer le dossier au secrétariat ».\n\
         Réponds UNIQUEMENT par la liste, une tâche par ligne commençant par « - ». {tools_rule}\n\nTexte :\n{text}",
        "Turn this text into clear, actionable tasks for a project: group what belongs together, drop duplicates \
         and chatter, keep a logical order. Each task MUST be written « Theme: action »: a short theme (1 to 4 words), \
         a colon, then ONE action, verb first (no « ; », no « then », no second sentence; two actions = two tasks). \
         Example: « Enrolment: send the file to the office ».\n\
         Answer ONLY with the list, one task per line starting with « - ». {tools_rule}\n\nText:\n{text}"
    );
    let dir = if !folder.trim().is_empty() && crate::project::normalize_folder(folder).is_dir() {
        folder.to_string()
    } else {
        dirs::home_dir().unwrap_or_default().display().to_string()
    };
    let (answer, _) = ask_with_images(
        &dir,
        prompt,
        "Read",
        3 + images.len() as u32,
        prefer,
        images,
        Weight::Light,
    )?;
    let tasks = ai_steps_only(parse_tasks(&answer));
    if tasks.is_empty() {
        return Err(crate::tr!(
            "l'IA n'a trouvé aucune tâche dans ce texte (au format « Thème : action »)",
            "the AI found no task in this text (written « Theme: action »)"
        )
        .into());
    }
    Ok(tasks)
}

/// Ask an agent one question in `folder` and return its answer. Claude Code may use only
/// `claude_tools`; Codex runs read-only. `prefer`: the agent to use when installed.
pub fn ask(
    folder: &str,
    prompt: String,
    claude_tools: &str,
    max_turns: u32,
    prefer: Option<Tool>,
) -> Result<(String, Tool), String> {
    ask_with_images(
        folder,
        prompt,
        claude_tools,
        max_turns,
        prefer,
        &[],
        Weight::Full,
    )
}

/// Like `ask`, for a question that only puts given material into shape (see `Weight::Light`).
pub fn ask_light(
    folder: &str,
    prompt: String,
    claude_tools: &str,
    max_turns: u32,
    prefer: Option<Tool>,
) -> Result<(String, Tool), String> {
    ask_with_images(
        folder,
        prompt,
        claude_tools,
        max_turns,
        prefer,
        &[],
        Weight::Light,
    )
}

/// Like `ask`, with images the agent may look at: Codex gets them with `--image`, Claude Code
/// reads them (its `Read` tool must be allowed) from folders added with `--add-dir`.
pub fn ask_with_images(
    folder: &str,
    prompt: String,
    claude_tools: &str,
    max_turns: u32,
    prefer: Option<Tool>,
    images: &[String],
    weight: Weight,
) -> Result<(String, Tool), String> {
    ask_full(folder, prompt, claude_tools, max_turns, prefer, images, weight)
        .map(|(answer, tool, _)| (answer, tool))
}

/// Like `ask_with_images`, with the id of the agent's conversation when it gave one.
fn ask_full(
    folder: &str,
    prompt: String,
    claude_tools: &str,
    max_turns: u32,
    prefer: Option<Tool>,
    images: &[String],
    weight: Weight,
) -> Result<(String, Tool, Option<String>), String> {
    let path = shell_path();
    let agents = detect(&path);
    let agent = agents
        .iter()
        .find(|a| Some(a.tool) == prefer)
        .or_else(|| agents.first())
        .ok_or(crate::tr!(
            "Ni Claude Code ni Codex n'ont été trouvés sur cette machine.",
            "Neither Claude Code nor Codex was found on this computer."
        ))?;
    let prompt_copy = prompt.clone();
    let light = weight_model(agent.tool, weight);
    let args = ask_args(
        agent.tool,
        prompt,
        claude_tools,
        max_turns,
        images,
        weight,
        light.as_deref(),
    );
    let res = run_agent(
        agent,
        folder,
        args,
        &path,
        &AtomicBool::new(false),
        PER_PROJECT,
    );
    // Claude Code can't renew its sign-in headless: Codex answers instead, when installed.
    if matches!(&res, Err(e) if e == AUTH_ERROR)
        && prefer != Some(Tool::Codex)
        && agents.iter().any(|a| a.tool == Tool::Codex)
        && let Ok(r) = ask_full(
            folder,
            prompt_copy,
            claude_tools,
            max_turns,
            Some(Tool::Codex),
            images,
            weight,
        )
    {
        return Ok(r);
    }
    let (answer, _, session) = res?;
    if answer.trim().is_empty() {
        return Err(crate::tr!("l'IA n'a rien répondu", "the AI answered nothing").into());
    }
    Ok((answer, agent.tool, session))
}

/// The longest quick question sent to an agent (characters).
pub const QUESTION_MAX: usize = 1000;

/// The prompt of a quick question about a project: a short, direct answer, read-only.
/// `code`: a code project (feature ideas, code); otherwise a project of documents —
/// school work, paperwork or anything else — where the answer stays about its content.
fn question_prompt(folder: &str, question: &str, code: bool) -> String {
    let mut p = question_prompt_base(folder, question, code);
    // The person found « about half a day for a person » beside the point: the work is
    // done by the AIs, so no estimate of time unless asked for.
    p.push_str(crate::tr!(
        "\n\nNe donne aucune estimation de durée (heures, jours, « une demi-journée »…), ni pour une \
         personne ni pour une IA, sauf si la question la demande.",
        "\n\nGive no estimate of time (hours, days, « half a day »…), neither for a person nor for an AI, \
         unless the question asks for one."
    ));
    p.push_str(crate::tr!(
        "\n\nSi la question demande d'ajouter, de créer ou de proposer des étapes au projet, termine ta \
         réponse par un bloc de code ```etapes avec une étape par ligne, au format « Thème : action à \
         l'infinitif », une seule action par étape, 10 au plus ; elles seront proposées dans l'encadré \
         Étapes. Sinon, pas de bloc.",
        "\n\nIf the question asks to add, create or propose steps for the project, end your answer with a \
         ```steps code block, one step per line, written « Theme: action », one action per step, 10 at \
         most; they will be proposed in the Steps box. Otherwise, no block."
    ));
    p
}

/// The most steps one answer may propose.
pub const ANSWER_STEPS_MAX: usize = 10;

/// An answer split into the text to show and the steps it proposes: the lines of its last
/// ```etapes (or ```étapes, ```steps) block, cleaned to one short line each, at most
/// `ANSWER_STEPS_MAX`; the block itself is taken out of the text.
pub fn split_answer_steps(answer: &str) -> (String, Vec<String>) {
    let lines: Vec<&str> = answer.lines().collect();
    let open = lines.iter().rposition(|l| {
        let tag = l.trim().strip_prefix("```").unwrap_or("").trim().to_lowercase();
        matches!(tag.as_str(), "etapes" | "étapes" | "steps")
    });
    let Some(open) = open else {
        return (answer.trim().to_string(), vec![]);
    };
    let close = lines[open + 1..]
        .iter()
        .position(|l| l.trim().starts_with("```"))
        .map(|i| open + 1 + i)
        .unwrap_or(lines.len());
    let mut steps: Vec<String> = vec![];
    for l in &lines[open + 1..close] {
        // A list mark (« - », « * », « • », « 3. », « 3) ») before the step, not the step's own digits.
        let s = l.trim().trim_start_matches(['-', '*', '•']).trim_start();
        let digits = s.len() - s.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let s = match s[digits..].chars().next() {
            Some('.' | ')') if digits > 0 => &s[digits + 1..],
            _ => s,
        }
        .trim();
        // Only steps in the AI step format are proposed.
        let Ok(s) = crate::project::ai_step(&crate::richtext::one_line(s, 200)) else {
            continue;
        };
        if !steps.contains(&s) && steps.len() < ANSWER_STEPS_MAX {
            steps.push(s);
        }
    }
    let mut text: Vec<&str> = lines[..open].to_vec();
    if close < lines.len() {
        text.extend(&lines[close + 1..]);
    }
    (text.join("\n").trim().to_string(), steps)
}

fn question_prompt_base(folder: &str, question: &str, code: bool) -> String {
    let q: String = question.trim().chars().take(QUESTION_MAX).collect();
    if !code {
        return crate::trf!(
            "Question rapide sur le projet du dossier « {folder} » (un projet sans code : travail scolaire, \
             démarche administrative ou autre). Ne modifie RIEN. Lis les documents du dossier seulement si c'est \
             utile pour répondre juste. Réponds dans la langue de la question, directement, en 150 mots au plus, \
             sans préambule, avec des mots simples et sans jargon technique : un avis franc, et si c'est une \
             démarche ou une idée, dis si elle vaut le coup, ce qu'il y aurait à faire et par quoi commencer.\
             \n\nQuestion :\n{q}",
            "Quick question about the project in the folder « {folder} » (a project without code: school work, \
             paperwork or anything else). Change NOTHING. Read the folder's documents only if it helps to answer \
             right. Answer in the question's language, directly, in 150 words at most, no preamble, in plain \
             words without technical jargon: a frank opinion, and if it's a task or an idea, say whether it's \
             worth it, what there would be to do and where to start.\n\nQuestion:\n{q}"
        );
    }
    crate::trf!(
        "Question rapide sur le projet du dossier « {folder} ». Ne modifie RIEN. Regarde le code ou l'historique \
         seulement si c'est utile pour répondre juste. Réponds dans la langue de la question, directement, \
         en 150 mots au plus, sans préambule : un avis franc, et si c'est une idée de fonctionnalité, \
         dis si elle vaut le coup, l'ampleur du changement (petit, moyen, gros : ce qu'il toucherait) et où \
         elle irait dans le code.\n\nQuestion :\n{q}",
        "Quick question about the project in the folder « {folder} ». Change NOTHING. Look at the code or history \
         only if it helps to answer right. Answer in the question's language, directly, in 150 words at most, \
         no preamble: a frank opinion, and if it's a feature idea, say whether it's worth it, how big the change \
         is (small, medium, large: what it would touch) and where it would go in the code.\n\nQuestion:\n{q}"
    )
}

/// An agent's conversation that can be carried on: its id, the folder it ran in (where its
/// session is filed) and the command that reopens it (`claude --resume …`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resume {
    pub session: String,
    pub cwd: String,
    pub cmd: String,
}

/// The answer to a quick question, and how to carry the conversation on.
#[derive(Clone, Debug)]
pub struct QuestionReply {
    pub answer: String,
    pub tool: Tool,
    /// When the agent gave its conversation's id.
    pub resume: Option<Resume>,
}

/// A quick question about a project, answered in a few lines by an agent (read-only).
pub fn ask_question(
    folder: &str,
    question: &str,
    code: Option<bool>,
    prefer: Option<Tool>,
) -> Result<QuestionReply, String> {
    if question.trim().is_empty() {
        return Err(crate::tr!("question vide", "empty question").into());
    }
    let dir = if !folder.trim().is_empty() && crate::project::normalize_folder(folder).is_dir() {
        crate::project::normalize_folder(folder)
            .display()
            .to_string()
    } else {
        dirs::home_dir().unwrap_or_default().display().to_string()
    };
    let code = crate::project::code_kind(&dir, code);
    let (answer, tool, session) = ask_full(
        &dir,
        question_prompt(&dir, question, code),
        "Read,Glob,Grep,Bash(git log:*),Bash(ls:*)",
        12,
        prefer,
        &[],
        Weight::Full,
    )?;
    Ok(QuestionReply {
        answer: answer.trim().to_string(),
        tool,
        resume: session.and_then(|s| {
            resume_command(tool, &s).map(|cmd| Resume {
                session: s,
                cwd: dir,
                cmd,
            })
        }),
    })
}

/// The arguments that send a reply on a Claude Code conversation, headless and read-only
/// (`claude -p <reply> --resume <id>`) or on a Codex one (`codex exec resume <id> <reply>`).
/// None for an id unsafe on a command line.
fn followup_args(tool: Tool, session: &str, reply: &str) -> Option<Vec<String>> {
    if !is_session_id(session) {
        return None;
    }
    match tool {
        Tool::ClaudeCode => {
            let r: String = reply.trim().chars().take(QUESTION_MAX).collect();
            Some(vec![
                "-p".into(),
                r,
                "--resume".into(),
                session.into(),
                "--output-format".into(),
                "json".into(),
                "--max-turns".into(),
                "12".into(),
                "--allowedTools".into(),
                "Read,Glob,Grep,Bash(git log:*),Bash(ls:*)".into(),
            ])
        }
        // `codex exec resume <id> <reply>`: the read-only sandbox is set before the
        // subcommand, so it applies whatever `resume` accepts.
        Tool::Codex => {
            let r: String = reply.trim().chars().take(QUESTION_MAX).collect();
            Some(vec![
                "exec".into(),
                "--sandbox".into(),
                "read-only".into(),
                "--skip-git-repo-check".into(),
                "resume".into(),
                session.into(),
                "--".into(),
                r,
            ])
        }
    }
}

/// A reply to a quick question's AI, sent on the same conversation (`claude -p --resume`),
/// run in the folder where that conversation is filed. The new answer keeps the same
/// conversation to carry on.
pub fn ask_followup(resume: &Resume, tool: Tool, reply: &str) -> Result<QuestionReply, String> {
    if reply.trim().is_empty() {
        return Err(crate::tr!("réponse vide", "empty reply").into());
    }
    let args = followup_args(tool, &resume.session, reply).ok_or(crate::tr!(
        "Cette IA ne peut pas encore poursuivre la conversation ici.",
        "This AI can't carry the conversation on here yet."
    ))?;
    let path = shell_path();
    let agent = detect(&path)
        .into_iter()
        .find(|a| a.tool == tool)
        .ok_or(crate::tr!(
            "L'IA de cette conversation n'est plus installée.",
            "The AI of this conversation is no longer installed."
        ))?;
    let (answer, _, session) = run_agent(
        &agent,
        &resume.cwd,
        args,
        &path,
        &AtomicBool::new(false),
        PER_PROJECT,
    )?;
    if answer.trim().is_empty() {
        return Err(crate::tr!("l'IA n'a rien répondu", "the AI answered nothing").into());
    }
    let session = session.unwrap_or_else(|| resume.session.clone());
    Ok(QuestionReply {
        answer: answer.trim().to_string(),
        tool,
        resume: resume_command(tool, &session).map(|cmd| Resume {
            session,
            cwd: resume.cwd.clone(),
            cmd,
        }),
    })
}

/// Ask an agent for a short outside view of a project (read-only, web search allowed).
/// `prefer`: the agent that worked on the project last, when it is installed.
pub fn ask_review(
    folder: &str,
    code: Option<bool>,
    prefer: Option<Tool>,
) -> Result<(String, Tool), String> {
    ask(
        folder,
        review_prompt(folder, crate::project::code_kind(folder, code)),
        "Read,Glob,Grep,Bash(git log:*),Bash(ls:*),WebSearch,WebFetch,mcp__orchestra__get_project_state",
        30,
        prefer,
    )
}

/// Where a folder's repository starts (the nearest folder holding `.git`), else itself.
pub fn repo_root(dir: &str) -> String {
    let p = Path::new(dir);
    let home = dirs::home_dir().unwrap_or_default();
    let mut cur = Some(p);
    while let Some(d) = cur {
        if d == home || d.parent().is_none() {
            break;
        }
        if d.join(".git").exists() {
            return d.display().to_string();
        }
        cur = d.parent();
    }
    dir.to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Waiting,
    Running,
    Done(String),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Job {
    pub folder: String,
    pub tool: Tool,
    pub state: JobState,
}

/// A synchronisation in progress, shared with the window.
#[derive(Clone)]
pub struct SyncRun {
    pub jobs: Arc<Mutex<Vec<Job>>>,
    /// Problems before the projects (agent missing, MCP not added…).
    pub notes: Arc<Mutex<Vec<String>>>,
    pub cancel: Arc<AtomicBool>,
    pub finished: Arc<AtomicBool>,
}

impl SyncRun {
    /// `wanted`: (folder, preferred agent). Folders whose agent isn't installed fall back
    /// to whichever agent is.
    pub fn start(wanted: Vec<(String, Tool)>, on_change: impl Fn() + Send + 'static) -> Self {
        let run = SyncRun {
            jobs: Arc::new(Mutex::new(
                wanted
                    .iter()
                    .map(|(f, t)| Job {
                        folder: f.clone(),
                        tool: *t,
                        state: JobState::Waiting,
                    })
                    .collect(),
            )),
            notes: Arc::default(),
            cancel: Arc::default(),
            finished: Arc::default(),
        };
        let r = run.clone();
        std::thread::spawn(move || {
            let note = |m: String| {
                r.notes.lock_or_recover().push(m);
                on_change();
            };
            let path = shell_path();
            let agents = detect(&path);
            if agents.is_empty() {
                note(crate::tr!("Ni Claude Code ni Codex n'ont été trouvés sur cette machine (commandes « claude » et « codex »).", "Neither Claude Code nor Codex was found on this computer (« claude » and « codex » commands).").into());
            }
            let exe = std::env::current_exe().unwrap_or_default();
            let mut ready: Vec<Agent> = vec![];
            for a in &agents {
                match ensure_mcp(a, &exe, &path) {
                    Ok(m) => {
                        if m == "branché" {
                            note(crate::trf!(
                                "{} est maintenant branché au tableau de bord.",
                                "{} is now plugged into the dashboard.",
                                a.tool.label()
                            ));
                        }
                        ready.push(a.clone());
                    }
                    Err(e) => note(crate::trf!(
                        "{} n'a pas pu être branché : {e}",
                        "{} couldn't be plugged in: {e}",
                        a.tool.label()
                    )),
                }
            }
            let n = r.jobs.lock_or_recover().len();
            for i in 0..n {
                if r.cancel.load(Ordering::Relaxed) {
                    break;
                }
                let (folder, tool) = {
                    let j = &r.jobs.lock_or_recover()[i];
                    (j.folder.clone(), j.tool)
                };
                let agent = ready
                    .iter()
                    .find(|a| a.tool == tool)
                    .or_else(|| ready.first())
                    .cloned();
                let result = match &agent {
                    None => Err(crate::tr!("aucune IA disponible", "no AI available").to_string()),
                    Some(a) => {
                        {
                            let mut jobs = r.jobs.lock_or_recover();
                            jobs[i].state = JobState::Running;
                            jobs[i].tool = a.tool;
                        }
                        on_change();
                        run_one(a, &folder, &path, &r.cancel)
                    }
                };
                r.jobs.lock_or_recover()[i].state = match result {
                    Ok(m) => JobState::Done(m),
                    Err(e) => JobState::Failed(e),
                };
                on_change();
            }
            // Folders never reached (stopped): mark them.
            for j in r.jobs.lock_or_recover().iter_mut() {
                if j.state == JobState::Waiting {
                    j.state = JobState::Failed(crate::tr!("non lancé", "not started").into());
                }
            }
            r.finished.store(true, Ordering::Relaxed);
            on_change();
        });
        run
    }

    pub fn snapshot(&self) -> (Vec<Job>, Vec<String>, bool) {
        (
            self.jobs.lock_or_recover().clone(),
            self.notes.lock_or_recover().clone(),
            self.finished.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_reach_the_agents() {
        let imgs = vec!["/tmp/x/a.png".to_string(), "/tmp/x/b.jpg".to_string()];
        let c = ask_args(
            Tool::ClaudeCode,
            "p".into(),
            "Read",
            5,
            &imgs,
            Weight::Full,
            None,
        );
        // Claude reads them itself: their folder is added once.
        assert_eq!(c.iter().filter(|a| *a == "--add-dir").count(), 1);
        assert!(
            c.windows(2)
                .any(|w| w[0] == "--add-dir" && w[1] == "/tmp/x")
        );
        let x = ask_args(
            Tool::Codex,
            "p".into(),
            "Read",
            5,
            &imgs,
            Weight::Full,
            None,
        );
        assert!(
            x.contains(&"--image=/tmp/x/a.png".to_string())
                && x.contains(&"--image=/tmp/x/b.jpg".to_string())
        );
        assert_eq!(x.last().map(String::as_str), Some("p"));
        // Without images, nothing changes.
        assert!(
            !ask_args(
                Tool::ClaudeCode,
                "p".into(),
                "Read",
                5,
                &[],
                Weight::Full,
                None
            )
            .contains(&"--add-dir".to_string())
        );
        assert!(
            is_image(Path::new("/a/B.PNG"))
                && is_image(Path::new("c.webp"))
                && !is_image(Path::new("notes.txt"))
                && !is_image(Path::new("dir"))
        );
    }

    #[test]
    fn light_questions_use_a_lighter_model() {
        // Commit messages, versions and tasks: Claude Code's light model, Haiku.
        let haiku = crate::catalog::light(&crate::catalog::builtin(crate::auto::Agent::ClaudeCode))
            .map(|m| m.id.clone());
        assert_eq!(haiku.as_deref(), Some("haiku"));
        let c = ask_args(
            Tool::ClaudeCode,
            "p".into(),
            "Read",
            3,
            &[],
            Weight::Light,
            haiku.as_deref(),
        );
        assert!(c.windows(2).any(|w| w[0] == "--model" && w[1] == "haiku"));
        let x = ask_args(
            Tool::Codex,
            "p".into(),
            "Read",
            3,
            &[],
            Weight::Light,
            Some("gpt-5.2-mini"),
        );
        assert!(
            x.windows(2)
                .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=\"low\"")
        );
        assert!(
            x.windows(2)
                .any(|w| w[0] == "--model" && w[1] == "gpt-5.2-mini")
        );
        assert_eq!(
            x.last().map(String::as_str),
            Some("p"),
            "the prompt stays last, after « -- »"
        );
        // Codex without a light model of its own: its default, with low effort only.
        let x = ask_args(Tool::Codex, "p".into(), "Read", 3, &[], Weight::Light, None);
        assert!(!x.contains(&"--model".to_string()) && x.contains(&"-c".to_string()));
        // A name that could be another option is left out.
        let c = ask_args(
            Tool::ClaudeCode,
            "p".into(),
            "Read",
            3,
            &[],
            Weight::Light,
            Some("--x"),
        );
        assert!(!c.contains(&"--model".to_string()));
        // A full question keeps the person's own default model.
        assert!(
            !ask_args(
                Tool::ClaudeCode,
                "p".into(),
                "Read",
                3,
                &[],
                Weight::Full,
                Some("haiku")
            )
            .contains(&"--model".to_string())
        );
        let x = ask_args(
            Tool::Codex,
            "p".into(),
            "Read",
            3,
            &[],
            Weight::Full,
            Some("gpt-5.2-mini"),
        );
        assert!(!x.contains(&"-c".to_string()) && !x.contains(&"--model".to_string()));
    }

    #[test]
    fn strong_questions_use_the_catalogs_high_end_model() {
        assert_eq!(
            weight_model(Tool::ClaudeCode, Weight::Strong).as_deref(),
            Some("opus")
        );
        assert_eq!(weight_model(Tool::ClaudeCode, Weight::Full), None);
        let c = ask_args(Tool::ClaudeCode, "p".into(), "Read", 3, &[], Weight::Strong, Some("opus"));
        assert!(c.windows(2).any(|w| w[0] == "--model" && w[1] == "opus"));
        let x = ask_args(Tool::Codex, "p".into(), "Read", 3, &[], Weight::Strong, None);
        assert!(
            x.windows(2)
                .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=\"high\"")
        );
    }

    #[test]
    fn review_follows_the_project_kind() {
        for code in [true, false] {
            let (r, f) = (review_prompt("/tmp/x", code), fill_prompt("/tmp/x", code));
            assert!(r.contains("/tmp/x") && f.contains("/tmp/x"));
            assert!(r.contains("RIEN") || r.contains("NOTHING"));
            assert!(r.contains("Suggestions"));
        }
        let (r, f) = (review_prompt("/tmp/x", false), fill_prompt("/tmp/x", false));
        assert!(r.contains("Ressources utiles") || r.contains("Useful resources"));
        assert!(
            !r.contains("git log"),
            "no git history for a project of documents"
        );
        assert!(f.contains("Ressources utiles") || f.contains("Useful resources"));
        assert!(review_prompt("/tmp/x", true).contains("git log"));
        assert!(
            !fill_prompt("/tmp/x", true).contains("Ressources utiles")
                && !fill_prompt("/tmp/x", true).contains("Useful resources")
        );
    }

    #[test]
    fn an_answer_can_propose_steps() {
        let a = "Oui, ça vaut le coup.\n\n```etapes\n- Export : ajouter le bouton PDF\n2. Tests : vérifier l'export\n2FA : activer la double authentification\n- Export : ajouter le bouton PDF\n\n```\nÀ vous de voir.";
        let (text, steps) = split_answer_steps(a);
        assert_eq!(text, "Oui, ça vaut le coup.\n\nÀ vous de voir.");
        assert_eq!(
            steps,
            [
                "Export : ajouter le bouton PDF",
                "Tests : vérifier l'export",
                "2FA : activer la double authentification"
            ]
        );
        // No block: the answer as it is, no step.
        assert_eq!(split_answer_steps("  Non.  "), ("Non.".to_string(), vec![]));
        // Another code block stays in the text; a block left open still counts.
        let (text, steps) =
            split_answer_steps("```rust\nfn x() {}\n```\n```steps\nDocs: write it\nWrite it then test it");
        assert_eq!(text, "```rust\nfn x() {}\n```");
        // Tidied to « Theme : action »; a line out of the format is dropped.
        assert_eq!(steps, ["Docs : write it"]);
        assert_eq!(
            ai_steps_only(vec!["a: faire x".into(), "Faire y".into(), "A : faire x".into()]),
            ["A : faire x"]
        );
        // Bounded.
        let many = format!("```étapes\n{}```", (0..30).map(|i| format!("T : faire {i}\n")).collect::<String>());
        assert_eq!(split_answer_steps(&many).1.len(), ANSWER_STEPS_MAX);
        assert!(question_prompt("/tmp/x", "?", true).contains("```"));
    }

    #[test]
    fn quick_questions_are_bounded_and_read_only() {
        let long = "é".repeat(QUESTION_MAX + 500);
        for code in [true, false] {
            let p = question_prompt("/tmp/x", &long, code);
            assert!(p.contains("/tmp/x"));
            assert_eq!(
                p.matches('é').count() - question_prompt("/tmp/x", "", code).matches('é').count(),
                QUESTION_MAX,
                "the question is cut at QUESTION_MAX"
            );
            assert!(p.contains("RIEN") || p.contains("NOTHING"));
            // No time estimate asked for, and one forbidden unless the question wants it.
            assert!(!p.contains("coûterait") && !p.contains("demanderait") && !p.contains("would cost"));
            assert!(p.contains("estimation de durée") || p.contains("estimate of time"));
        }
        // A project without code: no talk of code, its documents instead.
        let p = question_prompt("/tmp/x", "?", false);
        assert!(!p.contains("code »") && !p.contains("dans le code") && !p.contains("in the code"));
        assert!(p.contains("documents"));
        assert!(question_prompt("/tmp/x", "?", true).contains("code"));
        assert!(
            ask_question("/tmp/x", "   ", None, None).is_err(),
            "an empty question never reaches an agent"
        );
    }

    /// A quick question's conversation can be carried on: the agent's id is read from its
    /// output, and only a plain id ever reaches the command line.
    #[test]
    fn a_reply_resumes_the_same_claude_conversation() {
        let id = "0199a2b3-c4d5-7e6f-8a9b-0c1d2e3f4a5b";
        let a = followup_args(Tool::ClaudeCode, id, "  et pour le reste ?  ").unwrap();
        assert_eq!(a[0], "-p");
        assert_eq!(a[1], "et pour le reste ?");
        let i = a.iter().position(|x| x == "--resume").unwrap();
        assert_eq!(a[i + 1], id);
        assert!(a.iter().any(|x| x == "json"));
        assert!(!a.iter().any(|x| x.contains("Edit") || x.contains("Write")));
        assert!(followup_args(Tool::ClaudeCode, "abc;ls", "x").is_none());
        assert!(ask_followup(&Resume::default(), Tool::ClaudeCode, "  ").is_err());
        // Codex: same conversation through `exec resume`, still read-only.
        let c = followup_args(Tool::Codex, id, " et ensuite ? ").unwrap();
        let i = c.iter().position(|x| x == "resume").unwrap();
        assert_eq!(c[i + 1], id);
        assert_eq!(c.last().map(String::as_str), Some("et ensuite ?"));
        assert!(c.iter().position(|x| x == "read-only").unwrap() < i);
        assert!(followup_args(Tool::Codex, "abc;ls", "x").is_none());
    }

    #[test]
    fn a_quick_question_conversation_can_be_resumed() {
        let id = "0199a2b3-c4d5-7e6f-8a9b-0c1d2e3f4a5b";
        let header = format!("OpenAI Codex v0.50\n--------\nworkdir: /tmp\nmodel: gpt-5\nsession id: {id}\n--------\n");
        assert_eq!(codex_session_id(&header).as_deref(), Some(id));
        assert_eq!(codex_session_id("pas d'en-tête"), None);
        assert_eq!(codex_session_id("session id: x; rm -rf ~"), None);
        assert_eq!(
            resume_command(Tool::ClaudeCode, id).as_deref(),
            Some(format!("claude --resume {id}").as_str())
        );
        assert_eq!(
            resume_command(Tool::Codex, id).as_deref(),
            Some(format!("codex resume {id}").as_str())
        );
        for bad in ["", "abc", "a b c d e f g h", "$(touch x)abcdef", "abcdefgh;ls"] {
            assert_eq!(resume_command(Tool::ClaudeCode, bad), None, "{bad:?}");
        }
        // What reopens a saved terminal keeps the same conversation.
        let cmd = resume_command(Tool::ClaudeCode, id).unwrap();
        assert_eq!(crate::terminal::restore_cmd(Some(&cmd)), Some(cmd.clone()));
        let cmd = resume_command(Tool::Codex, id).unwrap();
        assert_eq!(crate::terminal::restore_cmd(Some(&cmd)), Some(cmd));
    }

    #[test]
    fn fill_and_tasks_are_parsed() {
        let f = parse_fill("Voici :\n```json\n{\"status\":\"active\",\"done\":[\"A\",\"\"],\"in_progress\":[\"B\"],\"blockers\":[],\"notes\":\"N\",\"review\":\"### Avis\\n- x\"}\n```").unwrap();
        assert_eq!(f.status, Some(crate::project::ProjectStatus::Active));
        assert_eq!((f.done.len(), f.doing.len(), f.notes.as_str()), (1, 1, "N"));
        assert!(f.review.starts_with("### Avis"));
        assert!(parse_fill("pas de json").is_err());
        let t = parse_tasks(
            "Voici :\n- UI : refaire la page\n* Écrire les tests\n2. Publier\n- [ ] Coller\n- UI : refaire la page\ntexte",
        );
        assert_eq!(
            t,
            vec![
                "UI : refaire la page",
                "Écrire les tests",
                "Publier",
                "Coller"
            ]
        );
    }

    #[test]
    fn repo_root_finds_git() {
        let base = std::env::temp_dir().join(format!("tia-sync-{}", std::process::id()));
        let deep = base.join("proj/src/app");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(base.join("proj/.git")).unwrap();
        assert_eq!(
            repo_root(&deep.display().to_string()),
            base.join("proj").display().to_string()
        );
        let other = base.join("loose");
        std::fs::create_dir_all(&other).unwrap();
        assert_eq!(
            repo_root(&other.display().to_string()),
            other.display().to_string()
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn prompt_and_args_stay_read_only() {
        let p = prompt("/x/y");
        assert!(p.contains("get_project_state") && p.contains("SANS RIEN MODIFIER"));
        let a = agent_args(Tool::ClaudeCode, &p).join(" ");
        assert!(
            a.contains("--allowedTools")
                && !a.contains("Edit")
                && !a.contains("Write")
                && !a.contains("skip-permissions")
        );
        assert!(agent_args(Tool::Codex, &p).join(" ").contains("read-only"));
    }

    #[test]
    fn fake_agent_runs_in_the_folder() {
        // A stand-in "claude" that prints its working folder as Claude's JSON result.
        let base = std::env::temp_dir().join(format!("tia-fake-{}", std::process::id()));
        std::fs::create_dir_all(base.join("bin")).unwrap();
        std::fs::create_dir_all(base.join("proj")).unwrap();
        let bin = base.join("bin/claude");
        std::fs::write(
            &bin,
            "#!/bin/sh\necho '{\"is_error\":false,\"num_turns\":3,\"result\":\"ok\"}'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = format!("{}:/usr/bin:/bin", base.join("bin").display());
        let agents = detect(&path);
        assert_eq!(agents[0].tool, Tool::ClaudeCode);
        let r = run_one(
            &agents[0],
            &base.join("proj").display().to_string(),
            &path,
            &AtomicBool::new(false),
        );
        assert!(r.unwrap().contains("3 échanges"));
        assert_eq!(
            run_one(&agents[0], "/nope/nope", &path, &AtomicBool::new(false)),
            Err("dossier introuvable".into())
        );
        let _ = std::fs::remove_dir_all(base);
    }
}
