//! "Synchroniser avec les IA": ask the coding agents installed on this machine (Claude Code,
//! Codex) to plug themselves into the dashboard's MCP server and write the state of play of
//! each of their projects.
//!
//! For every chosen folder, one agent runs headless *in that folder*, read-only (it may read
//! files and run `git log` / `git status`, nothing else), and talks to the dashboard only
//! through the MCP tools (get_project_state, create_project / update_project, add_steps).
//! Runs go one after the other, so two agents never write the workspace at the same time.
//! It uses the person's own Claude / Codex subscription, as if they had typed the request.

use crate::sessions::Tool;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// API keys found in the environment make Claude Code / Codex bill API credits instead of
/// the person's subscription ("Credit balance is too low"): agents launched from the
/// dashboard never get them, so they use the account the person logged in with.
pub const API_KEY_VARS: [&str; 3] = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "OPENAI_API_KEY"];

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
/// Starting the shell takes about a second: the answer is kept a few minutes (git alone asks
/// for it a dozen times to read one repository).
pub fn shell_path() -> String {
    static CACHE: Mutex<Option<(Instant, String)>> = Mutex::new(None);
    if let Some((at, p)) = CACHE.lock().unwrap().as_ref() {
        if at.elapsed() < Duration::from_secs(300) {
            return p.clone();
        }
    }
    let p = read_shell_path();
    *CACHE.lock().unwrap() = Some((Instant::now(), p.clone()));
    p
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
        let mut cmd = Command::new(&shell);
        cmd.args(["-lic", "printf '__PATH__%s\\n' \"$PATH\""]).stdin(Stdio::null()).stderr(Stdio::null());
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
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("__PATH__").map(String::from)))
            .unwrap_or_default();
        parts.extend(std::env::split_paths(&from_shell).filter(|p| !p.as_os_str().is_empty()));
    }
    let home = dirs::home_dir().unwrap_or_default();
    let mut extras = vec![home.join(".local/bin"), home.join(".claude/local"), home.join(".npm-global/bin")];
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
    std::env::join_paths(parts).map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// A program on this PATH (with `.exe` / `.cmd` on Windows).
pub fn which(name: &str, path: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    std::env::split_paths(path).find_map(|d| exts.iter().map(|e| d.join(format!("{name}{e}"))).find(|p| p.is_file()))
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
    crate::trf!("Tu es connecté au tableau de bord « orchestra » par MCP. Le projet est dans le dossier « {folder} ».\n\
         1. Appelle get_project_state avec folder = « {folder} » pour voir s'il existe déjà.\n\
         2. Examine rapidement le projet SANS RIEN MODIFIER : README, structure des dossiers, `git log --oneline -20`, `git status`.\n\
         3. S'il n'existe pas, appelle create_project (title : nom court et lisible du projet, folder : « {folder} ») ; sinon update_project.\n\
            Donne status, et l'état des lieux avec les listes done, in_progress et blockers : 3 à 6 éléments chacune au plus, une phrase courte par élément, sans Markdown.\n\
         Ne modifie aucun fichier, ne lance aucune autre commande. Termine en répondant seulement « ok ».", "You are connected to the « orchestra » dashboard through MCP. The project is in the folder « {folder} ».\n\
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
        Tool::Codex => vec!["exec".into(), "--sandbox".into(), "read-only".into(), "--skip-git-repo-check".into(), prompt.into()],
    }
}

fn last_line(s: &str) -> Option<String> {
    s.lines().rev().map(str::trim).find(|l| !l.is_empty()).map(|l| crate::richtext::one_line(l, 160))
}

/// Run one agent in one folder; stops it after `PER_PROJECT` or when `cancel` is set.
/// Run an agent headless in `folder` with these arguments. Returns its final answer and a
/// short report ("terminé en 42 s"), or an error people can act on.
fn run_agent(agent: &Agent, folder: &str, args: Vec<String>, path: &str, cancel: &AtomicBool, limit: Duration) -> Result<(String, String), String> {
    let dir = crate::project::normalize_folder(folder);
    if !dir.is_dir() {
        return Err(crate::tr!("dossier introuvable", "folder not found").into());
    }
    let mut cmd = Command::new(&agent.bin);
    for k in API_KEY_VARS {
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
    let out_t = read(child.stdout.take().map(|x| Box::new(x) as Box<dyn std::io::Read + Send>));
    let err_t = read(child.stderr.take().map(|x| Box::new(x) as Box<dyn std::io::Read + Send>));
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        if cancel.load(Ordering::Relaxed) || start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if cancel.load(Ordering::Relaxed) { crate::tr!("arrêté", "stopped").into() } else { crate::tr!("trop long, arrêté", "too long, stopped").into() });
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
    if agent.tool == Tool::ClaudeCode {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(out.trim()) {
            let result = v.get("result").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if v.get("is_error").and_then(|x| x.as_bool()) == Some(true) {
                return Err(crate::richtext::one_line(if result.is_empty() { "erreur" } else { &result }, 160));
            }
            let turns = v.get("num_turns").and_then(|x| x.as_u64()).unwrap_or(0);
            return Ok((result, crate::trf!("terminé en {} s, {turns} échanges", "done in {} s, {turns} exchanges", start.elapsed().as_secs())));
        }
    }
    if status.success() {
        Ok((out.trim().to_string(), crate::trf!("terminé en {} s", "done in {} s", start.elapsed().as_secs())))
    } else {
        Err(last_line(&err).or_else(|| last_line(&out)).unwrap_or_else(|| format!("code {}", status.code().unwrap_or(-1))))
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
    ["oauth", "authentication_error", "failed to authenticate", "not logged in", "please run /login", "invalid api key"].iter().any(|k| low.contains(k))
}

/// Synchronise one folder (see `prompt`).
fn run_one(agent: &Agent, folder: &str, path: &str, cancel: &AtomicBool) -> Result<String, String> {
    run_agent(agent, folder, agent_args(agent.tool, &prompt(folder)), path, cancel, PER_PROJECT).map(|(_, report)| report)
}

/// What the agent is asked for the "Regard extérieur" card.
pub fn review_prompt(folder: &str) -> String {
    crate::trf!("Donne un regard extérieur sur le projet du dossier « {folder} ». Lis-le rapidement SANS RIEN MODIFIER \
         (README, structure, `git log --oneline -15`) ; si l'outil get_project_state du serveur orchestra est disponible, \
         appelle-le avec folder = « {folder} » pour connaître l'état des lieux. Si tu peux chercher sur le web, regarde \
         quels projets ou produits similaires existent ou sont apparus récemment.\n\
         Réponds UNIQUEMENT avec ce Markdown, en français, condensé (900 caractères au plus en tout, pas de tableau, pas d'introduction ni de conclusion) :\n\
         ### Avis\n- 2 ou 3 puces : forces et faiblesses, sans complaisance\n\
         ### Projets similaires\n- 2 ou 3 puces : nom — en quoi il ressemble ou diffère (écris « rien de notable » si tu n'en vois pas)\n\
         ### Suggestions\n- 3 ou 4 puces : fonctionnalités ou actions concrètes pour la suite, la plus utile d'abord", "Give an outside view of the project in the folder « {folder} ». Read it quickly WITHOUT CHANGING ANYTHING \
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

pub fn fill_prompt(folder: &str) -> String {
    crate::trf!("Remplis la fiche du projet du dossier « {folder} » dans un tableau de bord. Lis-le rapidement SANS RIEN MODIFIER \
         (README, structure des dossiers, `git log --oneline -20`, `git status`). Si tu peux chercher sur le web, regarde quels \
         projets ou produits similaires existent.\n\
         Réponds UNIQUEMENT avec un objet JSON, en français, sans bloc de code :\n\
         {{\"status\": \"idea|active|paused|done\", \"done\": [\"…\"], \"in_progress\": [\"…\"], \"blockers\": [\"…\"], \"notes\": \"…\", \"review\": \"…\"}}\n\
         - done / in_progress / blockers : l'état des lieux, 0 à 6 éléments chacun, une phrase courte par élément, sans Markdown ;\n\
         - notes : une ou deux phrases sur ce qu'est le projet (peut être vide) ;\n\
         - review : un regard extérieur en Markdown, 900 caractères au plus, avec exactement ces sections : « ### Avis » (2 ou 3 puces, forces et faiblesses, sans complaisance), \
         « ### Projets similaires » (2 ou 3 puces « nom — en quoi il ressemble ou diffère », ou « rien de notable »), « ### Suggestions » (3 ou 4 puces, la plus utile d'abord).\n\
         Ne propose pas de liste de tâches.", "Fill in the card of the project in the folder « {folder} » in a dashboard. Read it quickly WITHOUT CHANGING ANYTHING \
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
    let v: serde_json::Value = serde_json::from_str(&answer[a..=b]).map_err(|_| crate::tr!("réponse de l'IA illisible", "unreadable AI answer").to_string())?;
    let list = |k: &str| -> Vec<String> {
        v.get(k).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).take(8).collect()).unwrap_or_default()
    };
    let text = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
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
pub fn ask_fill(folder: &str, prefer: Option<Tool>) -> Result<(Fill, Tool), String> {
    let (answer, tool) = ask(folder, fill_prompt(folder), "Read,Glob,Grep,Bash(git log:*),Bash(git status:*),Bash(ls:*),WebSearch,WebFetch", 30, prefer)?;
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
                (d > 0).then(|| l[d..].strip_prefix(". ").or_else(|| l[d..].strip_prefix(") "))).flatten()
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

/// Next tasks for a project, proposed by an AI that reads it (read-only). `existing`: the
/// steps already there, not to repeat.
pub fn ask_tasks(folder: &str, existing: &[String], prefer: Option<Tool>) -> Result<Vec<String>, String> {
    let have = if existing.is_empty() { crate::tr!("(aucune)", "(none)").to_string() } else { existing.iter().map(|e| format!("- {e}")).collect::<Vec<_>>().join("\n") };
    // These steps are what the AIs work on when they run on their own (Orchestration page):
    // each must be a real piece of work, precise enough to be done and checked without a human.
    let prompt = crate::trf!("Tu prépares le travail d'une IA qui va avancer SEULE sur le projet du dossier « {folder} ». \
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
         Exemple : « - Sécurité : échapper le nom de dossier passé à la commande git (src/git.rs) ».", "You are preparing the work of an AI that will move the project in the folder « {folder} » forward ON ITS OWN. \
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
         Example: « - Security: escape the folder name passed to the git command (src/git.rs) ».");
    let (answer, _) = ask(folder, prompt, "Read,Glob,Grep,Bash(git log:*),Bash(git status:*),Bash(ls:*)", 40, prefer)?;
    let tasks = parse_tasks(&answer);
    if tasks.is_empty() {
        return Err(crate::tr!("l'IA n'a proposé aucune tâche", "the AI proposed no task").into());
    }
    Ok(tasks)
}

/// The command line for one question to an agent. Images: Codex takes them with `--image`;
/// Claude Code reads them itself, from their folders added with `--add-dir`.
fn ask_args(tool: Tool, prompt: String, claude_tools: &str, max_turns: u32, images: &[String]) -> Vec<String> {
    match tool {
        Tool::ClaudeCode => {
            let mut a: Vec<String> = vec!["-p".into(), prompt, "--output-format".into(), "json".into(), "--max-turns".into(), max_turns.to_string(), "--allowedTools".into(), claude_tools.into()];
            let mut dirs: Vec<String> = vec![];
            for i in images {
                if let Some(d) = Path::new(i).parent().map(|d| d.display().to_string()).filter(|d| !d.is_empty()) {
                    if !dirs.contains(&d) {
                        dirs.push(d);
                    }
                }
            }
            for d in dirs {
                a.push("--add-dir".into());
                a.push(d);
            }
            a
        }
        Tool::Codex => {
            let mut a: Vec<String> = vec!["exec".into(), "--sandbox".into(), "read-only".into(), "--skip-git-repo-check".into()];
            for i in images {
                a.push(format!("--image={i}"));
            }
            a.push("--".into());
            a.push(prompt);
            a
        }
    }
}

/// The image files a person may join to a text (by their extension).
pub fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp"))
}

/// A pasted text (notes, an email, a list…) and joined images (screenshots…) turned into
/// tasks by an AI. It reads nothing else.
pub fn text_to_tasks(folder: &str, text: &str, images: &[String], prefer: Option<Tool>) -> Result<Vec<String>, String> {
    let text = if text.trim().is_empty() { crate::tr!("(pas de texte : seulement les images)", "(no text: only the images)").to_string() } else { text.to_string() };
    let tools_rule = if images.is_empty() {
        crate::tr!("N'utilise aucun outil.", "Don't use any tool.").to_string()
    } else {
        let list = images.iter().map(|i| format!("- {i}")).collect::<Vec<_>>().join("\n");
        crate::trf!("Des images sont jointes (captures d'écran, photos…) : regarde-les (outil Read sur chaque chemin) et tiens-en compte autant que du texte. N'utilise aucun autre outil.\nImages :\n{list}", "Images are joined (screenshots, photos…): look at them (Read tool on each path) and take them into account as much as the text. Don't use any other tool.\nImages:\n{list}")
    };
    let prompt = crate::trf!("Transforme ce texte en tâches claires et actionnables pour un projet : regroupe ce qui va ensemble, retire les doublons \
         et le bavardage, garde l'ordre logique. Une tâche = une action courte à l'infinitif (tu peux la préfixer d'un domaine court suivi de « : »).\n\
         Réponds UNIQUEMENT par la liste, une tâche par ligne commençant par « - ». {tools_rule}\n\nTexte :\n{text}", "Turn this text into clear, actionable tasks for a project: group what belongs together, drop duplicates \
         and chatter, keep a logical order. One task = one short action (you may prefix it with a short area followed by « : »).\n\
         Answer ONLY with the list, one task per line starting with « - ». {tools_rule}\n\nText:\n{text}");
    let dir = if !folder.trim().is_empty() && crate::project::normalize_folder(folder).is_dir() { folder.to_string() } else { dirs::home_dir().unwrap_or_default().display().to_string() };
    let (answer, _) = ask_with_images(&dir, prompt, "Read", 3 + images.len() as u32, prefer, images)?;
    let tasks = parse_tasks(&answer);
    if tasks.is_empty() {
        return Err(crate::tr!("l'IA n'a trouvé aucune tâche dans ce texte", "the AI found no task in this text").into());
    }
    Ok(tasks)
}

/// Ask an agent one question in `folder` and return its answer. Claude Code may use only
/// `claude_tools`; Codex runs read-only. `prefer`: the agent to use when installed.
pub fn ask(folder: &str, prompt: String, claude_tools: &str, max_turns: u32, prefer: Option<Tool>) -> Result<(String, Tool), String> {
    ask_with_images(folder, prompt, claude_tools, max_turns, prefer, &[])
}

/// Like `ask`, with images the agent may look at: Codex gets them with `--image`, Claude Code
/// reads them (its `Read` tool must be allowed) from folders added with `--add-dir`.
pub fn ask_with_images(folder: &str, prompt: String, claude_tools: &str, max_turns: u32, prefer: Option<Tool>, images: &[String]) -> Result<(String, Tool), String> {
    let path = shell_path();
    let agents = detect(&path);
    let agent = agents
        .iter()
        .find(|a| Some(a.tool) == prefer)
        .or_else(|| agents.first())
        .ok_or(crate::tr!("Ni Claude Code ni Codex n'ont été trouvés sur cette machine.", "Neither Claude Code nor Codex was found on this computer."))?;
    let prompt_copy = prompt.clone();
    let args = ask_args(agent.tool, prompt, claude_tools, max_turns, images);
    let res = run_agent(agent, folder, args, &path, &AtomicBool::new(false), PER_PROJECT);
    // Claude Code can't renew its sign-in headless: Codex answers instead, when installed.
    if matches!(&res, Err(e) if e == AUTH_ERROR) && prefer != Some(Tool::Codex) {
        if agents.iter().any(|a| a.tool == Tool::Codex) {
            if let Ok(r) = ask_with_images(folder, prompt_copy, claude_tools, max_turns, Some(Tool::Codex), images) {
                return Ok(r);
            }
        }
    }
    let (answer, _) = res?;
    if answer.trim().is_empty() {
        return Err(crate::tr!("l'IA n'a rien répondu", "the AI answered nothing").into());
    }
    Ok((answer, agent.tool))
}

/// Ask an agent for a short outside view of a project (read-only, web search allowed).
/// `prefer`: the agent that worked on the project last, when it is installed.
pub fn ask_review(folder: &str, prefer: Option<Tool>) -> Result<(String, Tool), String> {
    ask(folder, review_prompt(folder), "Read,Glob,Grep,Bash(git log:*),Bash(ls:*),WebSearch,WebFetch,mcp__orchestra__get_project_state", 30, prefer)
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
            jobs: Arc::new(Mutex::new(wanted.iter().map(|(f, t)| Job { folder: f.clone(), tool: *t, state: JobState::Waiting }).collect())),
            notes: Arc::default(),
            cancel: Arc::default(),
            finished: Arc::default(),
        };
        let r = run.clone();
        std::thread::spawn(move || {
            let note = |m: String| {
                r.notes.lock().unwrap().push(m);
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
                            note(crate::trf!("{} est maintenant branché au tableau de bord.", "{} is now plugged into the dashboard.", a.tool.label()));
                        }
                        ready.push(a.clone());
                    }
                    Err(e) => note(crate::trf!("{} n'a pas pu être branché : {e}", "{} couldn't be plugged in: {e}", a.tool.label())),
                }
            }
            let n = r.jobs.lock().unwrap().len();
            for i in 0..n {
                if r.cancel.load(Ordering::Relaxed) {
                    break;
                }
                let (folder, tool) = {
                    let j = &r.jobs.lock().unwrap()[i];
                    (j.folder.clone(), j.tool)
                };
                let agent = ready.iter().find(|a| a.tool == tool).or_else(|| ready.first()).cloned();
                let result = match &agent {
                    None => Err(crate::tr!("aucune IA disponible", "no AI available").to_string()),
                    Some(a) => {
                        {
                            let mut jobs = r.jobs.lock().unwrap();
                            jobs[i].state = JobState::Running;
                            jobs[i].tool = a.tool;
                        }
                        on_change();
                        run_one(a, &folder, &path, &r.cancel)
                    }
                };
                r.jobs.lock().unwrap()[i].state = match result {
                    Ok(m) => JobState::Done(m),
                    Err(e) => JobState::Failed(e),
                };
                on_change();
            }
            // Folders never reached (stopped): mark them.
            for j in r.jobs.lock().unwrap().iter_mut() {
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
        (self.jobs.lock().unwrap().clone(), self.notes.lock().unwrap().clone(), self.finished.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_reach_the_agents() {
        let imgs = vec!["/tmp/x/a.png".to_string(), "/tmp/x/b.jpg".to_string()];
        let c = ask_args(Tool::ClaudeCode, "p".into(), "Read", 5, &imgs);
        // Claude reads them itself: their folder is added once.
        assert_eq!(c.iter().filter(|a| *a == "--add-dir").count(), 1);
        assert!(c.windows(2).any(|w| w[0] == "--add-dir" && w[1] == "/tmp/x"));
        let x = ask_args(Tool::Codex, "p".into(), "Read", 5, &imgs);
        assert!(x.contains(&"--image=/tmp/x/a.png".to_string()) && x.contains(&"--image=/tmp/x/b.jpg".to_string()));
        assert_eq!(x.last().map(String::as_str), Some("p"));
        // Without images, nothing changes.
        assert!(!ask_args(Tool::ClaudeCode, "p".into(), "Read", 5, &[]).contains(&"--add-dir".to_string()));
        assert!(is_image(Path::new("/a/B.PNG")) && is_image(Path::new("c.webp")) && !is_image(Path::new("notes.txt")) && !is_image(Path::new("dir")));
    }

    #[test]
    fn fill_and_tasks_are_parsed() {
        let f = parse_fill("Voici :\n```json\n{\"status\":\"active\",\"done\":[\"A\",\"\"],\"in_progress\":[\"B\"],\"blockers\":[],\"notes\":\"N\",\"review\":\"### Avis\\n- x\"}\n```").unwrap();
        assert_eq!(f.status, Some(crate::project::ProjectStatus::Active));
        assert_eq!((f.done.len(), f.doing.len(), f.notes.as_str()), (1, 1, "N"));
        assert!(f.review.starts_with("### Avis"));
        assert!(parse_fill("pas de json").is_err());
        let t = parse_tasks("Voici :\n- UI : refaire la page\n* Écrire les tests\n2. Publier\n- [ ] Coller\n- UI : refaire la page\ntexte");
        assert_eq!(t, vec!["UI : refaire la page", "Écrire les tests", "Publier", "Coller"]);
    }

    #[test]
    fn repo_root_finds_git() {
        let base = std::env::temp_dir().join(format!("tia-sync-{}", std::process::id()));
        let deep = base.join("proj/src/app");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(base.join("proj/.git")).unwrap();
        assert_eq!(repo_root(&deep.display().to_string()), base.join("proj").display().to_string());
        let other = base.join("loose");
        std::fs::create_dir_all(&other).unwrap();
        assert_eq!(repo_root(&other.display().to_string()), other.display().to_string());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn prompt_and_args_stay_read_only() {
        let p = prompt("/x/y");
        assert!(p.contains("get_project_state") && p.contains("SANS RIEN MODIFIER"));
        let a = agent_args(Tool::ClaudeCode, &p).join(" ");
        assert!(a.contains("--allowedTools") && !a.contains("Edit") && !a.contains("Write") && !a.contains("skip-permissions"));
        assert!(agent_args(Tool::Codex, &p).join(" ").contains("read-only"));
    }

    #[test]
    fn fake_agent_runs_in_the_folder() {
        // A stand-in "claude" that prints its working folder as Claude's JSON result.
        let base = std::env::temp_dir().join(format!("tia-fake-{}", std::process::id()));
        std::fs::create_dir_all(base.join("bin")).unwrap();
        std::fs::create_dir_all(base.join("proj")).unwrap();
        let bin = base.join("bin/claude");
        std::fs::write(&bin, "#!/bin/sh\necho '{\"is_error\":false,\"num_turns\":3,\"result\":\"ok\"}'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = format!("{}:/usr/bin:/bin", base.join("bin").display());
        let agents = detect(&path);
        assert_eq!(agents[0].tool, Tool::ClaudeCode);
        let r = run_one(&agents[0], &base.join("proj").display().to_string(), &path, &AtomicBool::new(false));
        assert!(r.unwrap().contains("3 échanges"));
        assert_eq!(run_one(&agents[0], "/nope/nope", &path, &AtomicBool::new(false)), Err("dossier introuvable".into()));
        let _ = std::fs::remove_dir_all(base);
    }
}
