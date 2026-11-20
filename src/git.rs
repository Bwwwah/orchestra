//! Versions of a project, with the person's own `git` and GitHub's official `gh` tool
//! (signed in once with `gh auth login`): the dashboard never handles a token.
//!
//! Three steps, each started by a click:
//! - **Enregistrer**: `git add -A` then `git commit` (the AI may propose the message);
//! - **Envoyer**: `git push` (sets the upstream the first time);
//! - **Publier une version**: an annotated tag `vX.Y.Z`, pushed, then a GitHub release with
//!   the notes when `gh` is there. The repository's workflow builds the apps from the tag.

use crate::sessions::Tool;
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    /// ms since epoch.
    pub at: u64,
    pub author: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GitState {
    pub root: String,
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// `git status --porcelain` lines (two status letters, a space, the path).
    pub changes: Vec<String>,
    pub commits: Vec<Commit>,
    pub last_tag: Option<String>,
    /// Commits since the last tag (all of them when there is none).
    pub since_tag: u32,
    pub remote: Option<String>,
    /// GitHub page of the repository, when the remote is on GitHub.
    pub github: Option<String>,
    /// `gh` is installed and signed in.
    pub gh: bool,
    /// Local branches, then those only on the remote (without "origin/").
    pub branches: Vec<String>,
    pub remote_branches: Vec<String>,
    /// Full hash of HEAD (to find its GitHub Actions runs).
    pub head: String,
}

fn path() -> String {
    crate::sync::shell_path()
}

fn run_in(dir: &Path, bin: &str, args: &[&str]) -> Result<String, String> {
    let p = path();
    let exe = crate::sync::which(bin, &p).unwrap_or_else(|| PathBuf::from(bin));
    let o = crate::proc::command(exe)
        .args(args)
        .current_dir(dir)
        .env("PATH", &p)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{bin} : {e}"))?;
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    if o.status.success() {
        Ok(out)
    } else {
        let err = String::from_utf8_lossy(&o.stderr).to_string() + &out;
        Err(err.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("hint:")).take(3).collect::<Vec<_>>().join(" ").chars().take(300).collect())
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run_in(dir, "git", args)
}

/// `https://github.com/owner/repo` from an ssh or https remote.
pub fn github_page(remote: &str) -> Option<String> {
    let r = remote.trim().trim_end_matches(".git");
    let rest = r.strip_prefix("git@github.com:").or_else(|| r.strip_prefix("https://github.com/")).or_else(|| r.strip_prefix("ssh://git@github.com/"))?;
    (rest.split('/').count() == 2).then(|| format!("https://github.com/{rest}"))
}

/// The state of the repository holding `folder`. Err when it isn't one.
pub fn state(folder: &str) -> Result<GitState, String> {
    let dir = crate::project::normalize_folder(folder);
    if !dir.is_dir() {
        return Err(crate::tr!("dossier introuvable", "folder not found").into());
    }
    let root = git(&dir, &["rev-parse", "--show-toplevel"]).map_err(|_| crate::tr!("pas encore un dépôt git", "not a git repository yet").to_string())?.trim().to_string();
    let rd = PathBuf::from(&root);
    let mut st = GitState { root: root.clone(), ..Default::default() };
    let status = git(&rd, &["status", "--porcelain=v1", "-b"])?;
    for (i, l) in status.lines().enumerate() {
        if i == 0 && l.starts_with("## ") {
            parse_branch_line(&l[3..], &mut st);
        } else if !l.trim().is_empty() {
            st.changes.push(l.to_string());
        }
    }
    if let Ok(log) = git(&rd, &["log", "-8", "--format=%h%x1f%s%x1f%ct%x1f%an"]) {
        for l in log.lines() {
            let f: Vec<&str> = l.split('\u{1f}').collect();
            if f.len() == 4 {
                st.commits.push(Commit { hash: f[0].into(), subject: f[1].into(), at: f[2].parse::<u64>().unwrap_or(0) * 1000, author: f[3].into() });
            }
        }
    }
    st.last_tag = git(&rd, &["describe", "--tags", "--abbrev=0"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let range = st.last_tag.as_ref().map(|t| format!("{t}..HEAD")).unwrap_or_else(|| "HEAD".into());
    st.since_tag = git(&rd, &["rev-list", "--count", &range]).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    st.head = git(&rd, &["rev-parse", "HEAD"]).map(|h| h.trim().to_string()).unwrap_or_default();
    st.remote = git(&rd, &["remote", "get-url", "origin"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    st.github = st.remote.as_deref().and_then(github_page);
    st.gh = run_in(&rd, "gh", &["auth", "status"]).is_ok();
    if let Ok(out) = git(&rd, &["branch", "--format=%(refname:short)"]) {
        st.branches = out.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('(')).map(String::from).collect();
    }
    if let Ok(out) = git(&rd, &["branch", "-r", "--format=%(refname:short)"]) {
        for l in out.lines().map(str::trim) {
            let Some((_, name)) = l.split_once('/') else { continue };
            if name != "HEAD" && !name.is_empty() && !st.branches.iter().any(|b| b == name) && !st.remote_branches.iter().any(|b| b == name) {
                st.remote_branches.push(name.to_string());
            }
        }
    }
    Ok(st)
}

/// "main...origin/main [ahead 2, behind 1]", "main", "No commits yet on main".
fn parse_branch_line(s: &str, st: &mut GitState) {
    let s = s.trim();
    if let Some(b) = s.strip_prefix("No commits yet on ").or_else(|| s.strip_prefix("Initial commit on ")) {
        st.branch = b.to_string();
        return;
    }
    let (head, info) = match s.find(" [") {
        Some(i) => (&s[..i], Some(s[i + 2..].trim_end_matches(']'))),
        None => (s, None),
    };
    match head.split_once("...") {
        Some((b, up)) => {
            st.branch = b.into();
            st.upstream = Some(up.into());
        }
        None => st.branch = head.into(),
    }
    if let Some(info) = info {
        for part in info.split(", ") {
            if let Some(n) = part.strip_prefix("ahead ") {
                st.ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                st.behind = n.parse().unwrap_or(0);
            }
        }
    }
}

/// Turn the folder into a repository (first commit of everything).
pub fn init(folder: &str) -> Result<String, String> {
    let dir = crate::project::normalize_folder(folder);
    git(&dir, &["init", "-b", "main"])?;
    git(&dir, &["add", "-A"])?;
    git(&dir, &["commit", "-m", crate::tr!("Premier enregistrement", "First commit")])?;
    Ok(crate::tr!("Dépôt git créé, premier enregistrement fait.", "Git repository created, first commit done.").into())
}

/// `gh` (GitHub's official tool): (installed, signed in).
pub fn gh_info() -> (bool, bool) {
    let p = path();
    if crate::sync::which("gh", &p).is_none() {
        return (false, false);
    }
    (true, run_in(&std::env::temp_dir(), "gh", &["auth", "status"]).is_ok())
}

/// A GitHub repository for the project, then push (needs `gh`, signed in). When the folder
/// isn't a git repository yet, it becomes one first (everything in a first commit).
pub fn create_github(folder: &str, name: &str, private: bool) -> Result<String, String> {
    // Checked before anything is done to the folder. The name goes to `gh` as an argument:
    // one starting with "-" would be read as an option.
    let name = name.trim();
    if !valid_repo_name(name) {
        return Err(crate::tr!(
            "nom de dépôt invalide : lettres, chiffres, « . », « _ » et « - » seulement, sans « - » au début",
            "invalid repository name: letters, digits, « . », « _ » and « - » only, not starting with « - »"
        )
        .into());
    }
    let dir = crate::project::normalize_folder(folder);
    let root = match git(&dir, &["rev-parse", "--show-toplevel"]) {
        Ok(r) => PathBuf::from(r.trim()),
        Err(_) => {
            init(folder)?;
            dir.clone()
        }
    };
    if git(&root, &["rev-parse", "HEAD"]).is_err() {
        git(&root, &["add", "-A"])?;
        git(&root, &["commit", "-m", crate::tr!("Premier enregistrement", "First commit")])?;
    }
    let vis = if private { "--private" } else { "--public" };
    run_in(&root, "gh", &["repo", "create", name, vis, "--source", ".", "--remote", "origin", "--push"])?;
    Ok(crate::trf!("Dépôt GitHub {} « {name} » créé et envoyé.", "{} GitHub repository « {name} » created and pushed.", if private { crate::tr!("privé", "Private") } else { "public" }))
}

/// A name GitHub accepts for a new repository and `gh` can't take for an option:
/// [A-Za-z0-9._-], at most 100 characters, not starting with "-", not "." or "..".
pub fn valid_repo_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('-')
        && name != "."
        && name != ".."
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A repository name from a folder or title: "Bind — app mobile" → "bind-app-mobile".
pub fn repo_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        let c = match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c if c.is_ascii_alphanumeric() || c == '.' || c == '_' => c,
            _ => '-',
        };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    out.trim_matches('-').to_string()
}

pub fn commit(root: &str, message: &str) -> Result<String, String> {
    let rd = PathBuf::from(root);
    let msg = message.trim();
    if msg.is_empty() {
        return Err(crate::tr!("message vide", "empty message").into());
    }
    git(&rd, &["add", "-A"])?;
    git(&rd, &["commit", "-m", msg])?;
    let h = git(&rd, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    Ok(crate::trf!("Enregistré ({}).", "Saved ({}).", h.trim()))
}

pub fn push(root: &str, has_upstream: bool) -> Result<String, String> {
    let rd = PathBuf::from(root);
    if has_upstream {
        git(&rd, &["push"])?;
    } else {
        git(&rd, &["push", "-u", "origin", "HEAD"])?;
    }
    Ok(crate::tr!("Envoyé sur le dépôt distant.", "Pushed to the remote repository.").into())
}

/// Work on another branch (`git switch`; a branch only on the remote is fetched and
/// followed). Uncommitted changes come along when git can carry them, otherwise it refuses.
pub fn switch(root: &str, branch: &str, create: bool) -> Result<String, String> {
    let rd = PathBuf::from(root);
    let b = branch.trim();
    if !valid_branch(b) {
        return Err(crate::trf!("« {b} » n'est pas un nom de branche possible", "« {b} » isn't a valid branch name"));
    }
    if create {
        git(&rd, &["switch", "-c", b])?;
        Ok(crate::trf!("Nouvelle branche « {b} », vous y êtes.", "New branch « {b} », you're on it."))
    } else {
        if let Some(at) = worktree_of(&rd, b) {
            return Err(crate::trf!(
                "« {b} » est la copie d'une session autonome, ouverte dans {at}. Pour la récupérer, fusionnez-la depuis l'historique d'Orchestration ; pour la consulter, ouvrez ce dossier ou la branche sur GitHub",
                "« {b} » is an autonomous session's copy, open in {at}. To bring it in, merge it from the Orchestration history; to look at it, open that folder or the branch on GitHub"
            ));
        }
        git(&rd, &["switch", b])?;
        Ok(crate::trf!("Vous travaillez maintenant sur « {b} ».", "You're now working on « {b} »."))
    }
}

/// The folder of another worktree that has `branch` checked out (git refuses to switch to it).
fn worktree_of(rd: &Path, branch: &str) -> Option<String> {
    let list = git(rd, &["worktree", "list", "--porcelain"]).ok()?;
    let mut path = None;
    for l in list.lines() {
        if let Some(p) = l.strip_prefix("worktree ") {
            path = Some(p.to_string());
        } else if l.strip_prefix("branch refs/heads/") == Some(branch) {
            return path.filter(|p| Path::new(p) != rd);
        }
    }
    None
}

pub fn valid_branch(b: &str) -> bool {
    !b.is_empty()
        && !b.starts_with('-')
        && !b.starts_with('/')
        && !b.ends_with('/')
        && !b.ends_with(".lock")
        && !b.contains("..")
        && !b.chars().any(|c| c.is_whitespace() || "~^:?*[\\".contains(c) || c.is_control())
}

/// Bring the remote's new commits. Behind only: fast-forward. Diverged (commits on both
/// sides): replay ours on top of theirs; on a conflict, undo and say so. Unsaved changes
/// are set aside and put back (autostash).
pub fn pull(root: &str, diverged: bool) -> Result<String, String> {
    let rd = PathBuf::from(root);
    if diverged {
        if let Err(e) = git(&rd, &["pull", "--rebase", "--autostash"]) {
            let _ = git(&rd, &["rebase", "--abort"]);
            return Err(crate::trf!("conflit entre vos commits et ceux de GitHub, rien n'a changé ({e}). Réglez-le dans le terminal (git pull --rebase).", "conflict between your commits and GitHub's, nothing changed ({e}). Sort it out in the terminal (git pull --rebase)."));
        }
        return Ok(crate::tr!("Récupéré : vos commits sont replacés après ceux de GitHub. Envoyez-les quand vous voulez.", "Pulled: your commits now come after GitHub's. Push them when you want.").into());
    }
    git(&rd, &["pull", "--ff-only", "--autostash"])?;
    Ok(crate::tr!("À jour avec le dépôt distant.", "Up to date with the remote repository.").into())
}

/// Ask the remote what changed (`git fetch`), then say where the folder stands.
pub fn check(root: &str) -> Result<String, String> {
    let rd = PathBuf::from(root);
    git(&rd, &["fetch", "--prune", "--tags", "origin"])?;
    let st = state(root)?;
    Ok(describe(&st))
}

/// One sentence: saved or not, and ahead / behind / diverged against the remote.
pub fn describe(st: &GitState) -> String {
    let local = if st.changes.is_empty() { String::new() } else { crate::trf!(" ({} fichier(s) modifié(s) en local)", " ({} file(s) changed locally)", st.changes.len()) };
    let remote = match (st.upstream.is_some(), st.ahead, st.behind) {
        (false, _, _) => crate::tr!("Branche pas encore envoyée sur le dépôt distant", "Branch not pushed to the remote yet").to_string(),
        (true, 0, 0) => crate::tr!("À jour avec le dépôt distant", "Up to date with the remote").to_string(),
        (true, a, 0) => crate::trf!("{a} commit(s) à envoyer", "{a} commit(s) to push"),
        (true, 0, b) => crate::trf!("{b} nouveauté(s) sur le dépôt distant à récupérer", "{b} new commit(s) on the remote to pull"),
        (true, a, b) => crate::trf!("Divergé : {a} commit(s) ici et {b} sur le dépôt distant", "Diverged: {a} commit(s) here and {b} on the remote"),
    };
    format!("{remote}{local}.")
}

/// A GitHub Actions run (from `gh run list`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub workflow: String,
    pub title: String,
    /// The branch, or the tag for a release build.
    pub branch: String,
    pub sha: String,
    /// queued, in_progress, completed…
    pub status: String,
    /// success, failure, cancelled… (when completed)
    pub conclusion: String,
    pub url: String,
    /// ms since epoch.
    pub at: u64,
}

impl Run {
    pub fn running(&self) -> bool {
        self.status != "completed"
    }
}

/// The repository's latest workflow runs (needs `gh`, signed in).
pub fn runs(root: &str) -> Result<Vec<Run>, String> {
    let out = run_in(&PathBuf::from(root), "gh", &["run", "list", "--limit", "15", "--json", "workflowName,displayTitle,headBranch,headSha,status,conclusion,url,createdAt"])?;
    Ok(parse_runs(&out))
}

fn parse_runs(json: &str) -> Vec<Run> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let s = |r: &serde_json::Value, k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|r| Run {
                    workflow: s(r, "workflowName"),
                    title: s(r, "displayTitle"),
                    branch: s(r, "headBranch"),
                    sha: s(r, "headSha"),
                    status: s(r, "status"),
                    conclusion: s(r, "conclusion"),
                    url: s(r, "url"),
                    at: iso_ms(&s(r, "createdAt")),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The runs worth showing: for the current commit, the latest one per branch or tag (a
/// push to main and the release tag on the same commit give two), else the latest run of
/// the branch. Two at most.
pub fn runs_to_show(all: &[Run], st: &GitState) -> Vec<Run> {
    let mut out: Vec<Run> = vec![];
    for r in all.iter().filter(|r| !st.head.is_empty() && r.sha == st.head) {
        if !out.iter().any(|o| o.branch == r.branch) {
            out.push(r.clone());
        }
    }
    if out.is_empty() {
        if let Some(r) = all.iter().find(|r| r.branch == st.branch) {
            out.push(r.clone());
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.at));
    out.truncate(2);
    out
}

/// "2026-11-19T08:30:05Z" → ms since epoch (0 when unreadable).
pub fn iso_ms(s: &str) -> u64 {
    let n = |a: usize, b: usize| s.get(a..b).and_then(|x| x.parse::<i64>().ok());
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(se)) = (n(0, 4), n(5, 7), n(8, 10), n(11, 13), n(14, 16), n(17, 19)) else { return 0 };
    // Days from civil (Howard Hinnant).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    ((days * 86400 + h * 3600 + mi * 60 + se).max(0) as u64) * 1000
}

/// Tag, push the tag, then the GitHub release with its notes (when `gh` is there).
pub fn release(root: &str, version: &str, notes: &str, gh: bool) -> Result<String, String> {
    let rd = PathBuf::from(root);
    let v = version.trim();
    if !valid_version(v) {
        return Err(crate::trf!("« {v} » n'est pas un numéro de version (ex. v1.2.0)", "« {v} » isn't a version number (e.g. v1.2.0)"));
    }
    git(&rd, &["tag", "-a", v, "-m", &format!("{v}\n\n{}", notes.trim())])?;
    git(&rd, &["push", "origin", v])?;
    if gh {
        // If the build workflow made the release first, only update its notes.
        if run_in(&rd, "gh", &["release", "view", v]).is_ok() {
            run_in(&rd, "gh", &["release", "edit", v, "--notes", notes.trim()])?;
        } else {
            run_in(&rd, "gh", &["release", "create", v, "--title", v, "--notes", notes.trim(), "--verify-tag"])?;
        }
        return Ok(crate::trf!("Version {v} publiée sur GitHub.", "Version {v} published on GitHub."));
    }
    Ok(crate::trf!("Version {v} étiquetée et envoyée (installez « gh » pour créer la page de version).", "Version {v} tagged and pushed (install « gh » to also create the release page)."))
}

pub fn valid_version(v: &str) -> bool {
    let n = v.strip_prefix('v').unwrap_or(v);
    let parts: Vec<&str> = n.split('-').next().unwrap_or("").split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// (patch, minor, major) after `last` ("v0.3.1" → v0.3.2, v0.4.0, v1.0.0).
pub fn next_versions(last: Option<&str>) -> [String; 3] {
    let n: Vec<u64> = last
        .map(|l| l.trim_start_matches('v').split('-').next().unwrap_or("").split('.').map(|x| x.parse().unwrap_or(0)).collect())
        .unwrap_or_default();
    if n.len() < 3 {
        return ["v0.1.0".into(), "v0.1.0".into(), "v1.0.0".into()];
    }
    [format!("v{}.{}.{}", n[0], n[1], n[2] + 1), format!("v{}.{}.0", n[0], n[1] + 1), format!("v{}.0.0", n[0] + 1)]
}

// ------------------------------------------------------------------- the AI's help

fn diff_for_prompt(rd: &Path) -> String {
    let stat = git(rd, &["diff", "HEAD", "--stat"]).or_else(|_| git(rd, &["diff", "--cached", "--stat"])).unwrap_or_default();
    let untracked = git(rd, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    let mut diff = git(rd, &["diff", "HEAD"]).unwrap_or_default();
    if diff.len() > 14_000 {
        let mut cut = 14_000;
        while !diff.is_char_boundary(cut) {
            cut -= 1;
        }
        diff.truncate(cut);
        diff.push_str(crate::tr!("\n[… diff coupé]", "\n[… diff cut]"));
    }
    let untracked: Vec<&str> = untracked.lines().take(40).collect();
    crate::trf!("Résumé :\n{stat}\nNouveaux fichiers :\n{}\n\nDiff :\n{diff}", "Summary:\n{stat}\nNew files:\n{}\n\nDiff:\n{diff}", untracked.join("\n"))
}

/// A commit message for the current changes, in the style of the recent ones.
pub fn suggest_commit(st: &GitState, prefer: Option<Tool>) -> Result<String, String> {
    let rd = PathBuf::from(&st.root);
    let recent: Vec<String> = st.commits.iter().take(6).map(|c| format!("- {}", c.subject)).collect();
    let prompt = crate::trf!("Écris le message de commit de ces changements.\n\
         Format : une première ligne courte (72 caractères au plus), à l'impératif, qui dit ce qui change ; \
         puis, si utile, une ligne vide et 1 à 4 puces « - » courtes. Écris dans la langue des commits récents \
         (français s'il n'y en a pas). Réponds UNIQUEMENT avec le message, sans guillemets ni bloc de code.\n\n\
         Commits récents :\n{}\n\n{}", "Write the commit message for these changes.\n\
         Format: a short first line (72 characters at most), in the imperative, saying what changes; \
         then, if useful, a blank line and 1 to 4 short « - » bullets. Write in the language of the recent commits \
         (English if there are none). Answer ONLY with the message, no quotes, no code block.\n\n\
         Recent commits:\n{}\n\n{}",
        if recent.is_empty() { crate::tr!("(aucun)", "(none)").into() } else { recent.join("\n") },
        diff_for_prompt(&rd)
    );
    // Putting a diff into words: a lighter model is enough.
    let (answer, _) = crate::sync::ask_light(&st.root, prompt, "Bash(git diff:*),Bash(git status:*),Read", 4, prefer)?;
    Ok(clean_answer(&answer))
}

/// (version, notes) for the next release, from the commits since the last tag.
pub fn suggest_release(st: &GitState, prefer: Option<Tool>) -> Result<(String, String), String> {
    let rd = PathBuf::from(&st.root);
    let range = st.last_tag.as_ref().map(|t| format!("{t}..HEAD")).unwrap_or_else(|| "HEAD".into());
    let log = git(&rd, &["log", &range, "--format=- %s", "-60"]).unwrap_or_default();
    let [patch, minor, major] = next_versions(st.last_tag.as_deref());
    let prompt = crate::trf!("Prépare la prochaine version du projet. Dernière version : {}.\n\
         Choisis le numéro : {patch} pour des corrections seulement, {minor} pour de nouvelles fonctionnalités, \
         {major} pour des changements qui cassent l'existant.\n\
         Réponds UNIQUEMENT ainsi : la première ligne « VERSION: vX.Y.Z », puis les notes de version en Markdown \
         français, courtes : 3 à 8 puces regroupées sous « ### Nouveautés », « ### Corrections » (seulement les \
         sections utiles), en langage clair pour un utilisateur.\n\nCommits depuis la dernière version :\n{log}", "Prepare the project's next version. Last version: {}.\n\
         Choose the number: {patch} for fixes only, {minor} for new features, \
         {major} for changes that break what exists.\n\
         Answer ONLY like this: the first line « VERSION: vX.Y.Z », then the release notes in English \
         Markdown, short: 3 to 8 bullets grouped under « ### New », « ### Fixes » (only the useful \
         sections), in plain language for a user.\n\nCommits since the last version:\n{log}",
        st.last_tag.as_deref().unwrap_or("aucune")
    );
    let (answer, _) = crate::sync::ask_light(&st.root, prompt, "Bash(git log:*),Bash(git diff:*),Read", 4, prefer)?;
    let answer = clean_answer(&answer);
    let mut version = minor.clone();
    let mut notes = vec![];
    for l in answer.lines() {
        if let Some(v) = l.trim().strip_prefix("VERSION:") {
            let v = v.trim().trim_matches('`');
            let v = if v.starts_with('v') { v.to_string() } else { format!("v{v}") };
            if valid_version(&v) {
                version = v;
            }
        } else {
            notes.push(l);
        }
    }
    Ok((version, notes.join("\n").trim().to_string()))
}

fn clean_answer(a: &str) -> String {
    let t = a.trim();
    let t = t.strip_prefix("```").map(|r| r.split_once('\n').map(|x| x.1).unwrap_or(r)).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t);
    t.trim().trim_matches('"').trim().to_string()
}

/// A few lines about the repository for the AIs (MCP `get_project_state`).
pub fn for_ai(folder: &str) -> Option<String> {
    let st = state(folder).ok()?;
    let mut s = crate::trf!("Branche {}", "Branch {}", st.branch);
    if let Some(u) = &st.upstream {
        s.push_str(&crate::trf!(" (suit {u}, {} en avance, {} en retard)", " (tracks {u}, {} ahead, {} behind)", st.ahead, st.behind));
    }
    s.push_str(&crate::trf!(" · {} fichier(s) modifié(s) non enregistré(s)", " · {} unsaved changed file(s)", st.changes.len()));
    if let Some(t) = &st.last_tag {
        s.push_str(&crate::trf!(" · dernière version {t}, {} commit(s) depuis", " · last version {t}, {} commit(s) since", st.since_tag));
    }
    if let Some(g) = &st.github {
        s.push_str(&format!(" · {g}"));
    }
    for c in st.commits.iter().take(5) {
        s.push_str(&format!("\n- {} {}", c.hash, c.subject));
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_line() {
        let mut st = GitState::default();
        parse_branch_line("main...origin/main [ahead 2, behind 1]", &mut st);
        assert_eq!((st.branch.as_str(), st.upstream.as_deref(), st.ahead, st.behind), ("main", Some("origin/main"), 2, 1));
        let mut st = GitState::default();
        parse_branch_line("No commits yet on main", &mut st);
        assert_eq!(st.branch, "main");
    }

    #[test]
    fn repository_names() {
        for ok in ["orchestra", "Bind-app_mobile.v2", "a", ".github", &"x".repeat(100)] {
            assert!(valid_repo_name(ok), "{ok}");
        }
        for bad in ["", "-h", "--public", "a b", "owner/name", ".", "..", "café", "a;rm", "$(x)", &"x".repeat(101)] {
            assert!(!valid_repo_name(bad), "{bad}");
        }
        // The names the app proposes pass.
        assert!(valid_repo_name(&repo_name("Bind — app mobile")));
        // Refused before the folder is touched (no git init, no commit).
        let dir = std::env::temp_dir().join(format!("orch-ghname-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(create_github(dir.to_str().unwrap(), "--help", true).is_err());
        assert!(!dir.join(".git").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn versions() {
        assert_eq!(next_versions(Some("v0.3.1")), ["v0.3.2", "v0.4.0", "v1.0.0"]);
        assert_eq!(next_versions(None)[0], "v0.1.0");
        assert!(valid_version("v1.2.3") && valid_version("1.0.0-beta") && !valid_version("v1.2"));
        assert_eq!(github_page("git@github.com:bwwwah/orchestra.git").as_deref(), Some("https://github.com/bwwwah/orchestra"));
        assert_eq!(repo_name("Bind — app mobile"), "bind-app-mobile");
    }

    #[test]
    fn runs_and_dates() {
        assert_eq!(iso_ms("1970-01-02T00:00:01Z"), 86_401_000);
        assert_eq!(iso_ms("2026-11-19T00:00:00Z"), 1_795_046_400_000);
        let json = r#"[{"workflowName":"Release","displayTitle":"v0.3.0","headBranch":"v0.3.0","headSha":"abc","status":"in_progress","conclusion":"","url":"u1","createdAt":"2026-11-19T09:00:00Z"},
            {"workflowName":"Release","displayTitle":"Ajoute x","headBranch":"main","headSha":"abc","status":"completed","conclusion":"success","url":"u2","createdAt":"2026-11-19T08:59:00Z"},
            {"workflowName":"Release","displayTitle":"Ancien","headBranch":"main","headSha":"old","status":"completed","conclusion":"failure","url":"u3","createdAt":"2026-11-18T08:00:00Z"}]"#;
        let all = parse_runs(json);
        assert_eq!(all.len(), 3);
        let st = GitState { head: "abc".into(), branch: "main".into(), ..Default::default() };
        let shown = runs_to_show(&all, &st);
        assert_eq!(shown.iter().map(|r| r.url.as_str()).collect::<Vec<_>>(), ["u1", "u2"]);
        assert!(shown[0].running());
        let st = GitState { head: "new".into(), branch: "main".into(), ..Default::default() };
        assert_eq!(runs_to_show(&all, &st)[0].url, "u2");
        let st = GitState { upstream: Some("origin/main".into()), ahead: 1, behind: 2, ..Default::default() };
        assert!(describe(&st).contains('1'));
    }

    #[test]
    fn real_repo() {
        let dir = std::env::temp_dir().join(format!("orch-git-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let d = dir.display().to_string();
        assert!(state(&d).is_err());
        let _ = crate::proc::command("git").args(["config", "--global", "user.email"]).output();
        if init(&d).is_err() {
            // No git identity on this machine: nothing more to check.
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        std::fs::write(dir.join("b.txt"), "y").unwrap();
        let st = state(&d).unwrap();
        assert_eq!(st.branch, "main");
        assert_eq!(st.changes.len(), 1);
        assert_eq!(st.commits.len(), 1);
        assert_eq!(st.branches, vec!["main".to_string()]);
        commit(&st.root, "Ajoute b").unwrap();
        switch(&st.root, "essai", true).unwrap();
        assert_eq!(state(&d).unwrap().branch, "essai");
        switch(&st.root, "main", false).unwrap();
        assert!(switch(&st.root, "a b", true).is_err());
        let st = state(&d).unwrap();
        assert!(st.changes.is_empty());
        assert_eq!(st.since_tag, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
