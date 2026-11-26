//! The data folder chosen in the settings, without any environment variable.
//!
//! The choice is a one-line file next to the usual data folder, which every Orchestra program
//! (window, MCP servers, status line) reads when it starts. A new choice is first only a
//! request: the window carries it out at its next launch, before anything is loaded, so that
//! nothing written meanwhile is left behind. Nothing is ever deleted from the folder left.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Names the folder in use, when it isn't the usual one.
const CHOSEN: &str = "orchestra-dossier";
/// Names the folder asked for, until the window's next launch.
const WANTED: &str = "orchestra-dossier.demande";
/// Marks a folder being filled: a copy that failed halfway may be made again.
const PARTIAL: &str = ".orchestra-copie-en-cours";

/// What stays behind: copies of autonomous sessions are tied to their path by git (two folders
/// would share one index), and RTK's folder is made again when needed.
const LEFT_BEHIND: [&str; 2] = ["worktrees", "rtk-home"];

/// What moving to a folder does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// The folder is empty (or to be made): today's data is copied into it.
    Copy,
    /// The folder already holds Orchestra data: it is used as it is, nothing is copied.
    UseExisting,
}

/// Where the two files above live.
pub fn base_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn read_path(file: &Path) -> Option<PathBuf> {
    let s = fs::read_to_string(file).ok()?;
    let p = PathBuf::from(s.trim_end_matches(['\r', '\n']));
    p.is_absolute().then_some(p)
}

/// The folder the person chose, whether or not it can be reached today.
pub fn chosen_in(base: &Path) -> Option<PathBuf> {
    read_path(&base.join(CHOSEN))
}

/// The folder asked for, not used yet.
pub fn wanted_in(base: &Path) -> Option<PathBuf> {
    read_path(&base.join(WANTED))
}

/// The folder to use: the chosen one when it is there, else the usual one (an unplugged disk:
/// the settings say so).
pub fn folder_in(base: &Path, usual: &Path) -> PathBuf {
    chosen_in(base)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| usual.to_path_buf())
}

/// What the person typed: spaces around removed, `~` standing for their home.
pub fn expand(typed: &str) -> PathBuf {
    let typed = typed.trim();
    if let Some(home) = dirs::home_dir() {
        if typed == "~" {
            return home;
        }
        if let Some(rest) = typed.strip_prefix("~/") {
            return home.join(rest);
        }
    }
    PathBuf::from(typed)
}

/// `p` with its links resolved, as far as it exists.
fn real(p: &Path) -> PathBuf {
    if let Ok(r) = fs::canonicalize(p) {
        return r;
    }
    match (p.parent(), p.file_name()) {
        (Some(parent), Some(name)) => real(parent).join(name),
        _ => p.to_path_buf(),
    }
}

/// Whether Orchestra can go from folder `current` to folder `target`, and how.
pub fn check(current: &Path, target: &Path, usual: &Path) -> Result<Plan, String> {
    let dots = target.components().any(|c| c.as_os_str() == "..");
    if !target.is_absolute() || dots {
        return Err(crate::tr!(
            "indiquez le chemin complet du dossier",
            "give the folder's full path"
        )
        .to_string());
    }
    let (from, to) = (real(current), real(target));
    if from == to {
        return Err(crate::tr!(
            "c'est déjà le dossier utilisé",
            "this folder is already in use"
        )
        .to_string());
    }
    if to.starts_with(&from) || from.starts_with(&to) {
        return Err(crate::tr!(
            "le nouveau dossier et l'actuel ne doivent pas être l'un dans l'autre",
            "the new folder and the current one can't be inside each other"
        )
        .to_string());
    }
    if !target.exists() {
        return Ok(Plan::Copy);
    }
    if !target.is_dir() {
        return Err(crate::tr!(
            "ce chemin désigne un fichier, pas un dossier",
            "this path is a file, not a folder"
        )
        .to_string());
    }
    if target.join(PARTIAL).is_file() {
        return Ok(Plan::Copy);
    }
    if target.join("workspace.json").is_file() {
        return Ok(Plan::UseExisting);
    }
    // The usual folder is Orchestra's own, even without a workspace yet.
    if to == real(usual) {
        return Ok(Plan::Copy);
    }
    let mut entries = fs::read_dir(target).map_err(|e| e.to_string())?;
    if entries.any(|e| e.map_or(true, |e| e.file_name() != ".DS_Store")) {
        return Err(crate::tr!(
            "choisissez un dossier vide, ou un dossier de données d'Orchestra",
            "choose an empty folder, or an Orchestra data folder"
        )
        .to_string());
    }
    Ok(Plan::Copy)
}

/// Ask for `target` at the next launch.
pub fn request_in(base: &Path, current: &Path, target: &Path, usual: &Path) -> Result<(), String> {
    check(current, target, usual)?;
    let text = target.to_str().filter(|s| !s.contains(['\r', '\n']));
    let Some(text) = text else {
        return Err(crate::tr!(
            "ce nom de dossier ne peut pas être enregistré",
            "this folder name can't be saved"
        )
        .to_string());
    };
    fs::create_dir_all(base)
        .and_then(|()| crate::store::write_atomic(&base.join(WANTED), text.as_bytes()))
        .map_err(|e| e.to_string())
}

/// Forget the folder asked for.
pub fn cancel_in(base: &Path) {
    let _ = fs::remove_file(base.join(WANTED));
}

/// Forget the chosen folder: the usual one again at the next launch, nothing moved.
pub fn forget_in(base: &Path) {
    let _ = fs::remove_file(base.join(CHOSEN));
}

/// Carry out the request, if any: the folder now in use, or why it stays the same. A request
/// is tried once only.
pub fn apply_in(base: &Path, usual: &Path) -> Option<Result<PathBuf, String>> {
    let target = wanted_in(base)?;
    cancel_in(base);
    let current = folder_in(base, usual);
    Some(switch(base, &current, &target, usual).map(|()| target))
}

/// Why the last request wasn't carried out at this launch.
static FAILURE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The window's launch, before anything is read: carry out the request, if any.
pub fn apply_at_launch() {
    // `ORCHESTRA_DIR` imposes its folder: a request waits for a launch without it.
    if crate::store::forced_data_dir().is_some() {
        return;
    }
    if let Some(Err(e)) = apply_in(&base_dir(), &crate::store::usual_data_dir()) {
        eprintln!("orchestra: data folder not changed ({e})");
        let _ = FAILURE.set(e);
    }
}

pub fn launch_failure() -> Option<&'static str> {
    FAILURE.get().map(String::as_str)
}

fn switch(base: &Path, current: &Path, target: &Path, usual: &Path) -> Result<(), String> {
    if check(current, target, usual)? == Plan::Copy {
        copy_data(current, target).map_err(|e| e.to_string())?;
    }
    let file = base.join(CHOSEN);
    if real(target) == real(usual) {
        match fs::remove_file(file) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    } else {
        let text = target.to_string_lossy();
        crate::store::write_atomic(&file, text.as_bytes()).map_err(|e| e.to_string())
    }
}

fn copy_data(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    // The data is private: only its owner reads the new folder, like the files in it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Some disks know no permissions: that doesn't stop the copy.
        let _ = fs::set_permissions(to, fs::Permissions::from_mode(0o700));
    }
    fs::write(to.join(PARTIAL), b"")?;
    if from.is_dir() {
        copy_folder(from, to, true)?;
        // Last: a folder holds data only once everything else is there.
        let workspace = from.join("workspace.json");
        if workspace.is_file() {
            fs::copy(workspace, to.join("workspace.json"))?;
        }
    }
    fs::remove_file(to.join(PARTIAL))
}

fn copy_folder(from: &Path, to: &Path, top: bool) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let skipped = top
            && (name == "workspace.json" || name == PARTIAL || LEFT_BEHIND.iter().any(|x| name == *x));
        if skipped || Path::new(&name).extension().is_some_and(|x| x == "lock") {
            continue;
        }
        // Links and sockets aren't data: only folders and plain files go.
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_folder(&entry.path(), &to.join(&name), false)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), to.join(&name))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder with the usual data folder in it.
    fn setup() -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("orchestra-data-{}", crate::model::new_id()));
        let usual = base.join("orchestra");
        for d in ["espaces/a", "backups", "worktrees/copie", "rtk-home"] {
            fs::create_dir_all(usual.join(d)).unwrap();
        }
        for f in [
            "workspace.json",
            "workspace.json.lock",
            ".computer-id",
            "espaces/a/workspace.json",
            "backups/b.txt",
            "worktrees/copie/c.txt",
            "rtk-home/r.txt",
        ] {
            fs::write(usual.join(f), f).unwrap();
        }
        (base, usual)
    }

    #[test]
    fn nothing_moves_before_the_next_launch() {
        let (base, usual) = setup();
        let target = base.join("ailleurs");
        request_in(&base, &usual, &target, &usual).unwrap();
        assert_eq!(folder_in(&base, &usual), usual);
        assert_eq!(wanted_in(&base), Some(target.clone()));
        assert!(!target.exists());
        cancel_in(&base);
        assert!(apply_in(&base, &usual).is_none());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn data_is_copied_and_the_old_folder_kept() {
        let (base, usual) = setup();
        let target = base.join("mon dossier");
        request_in(&base, &usual, &target, &usual).unwrap();
        assert_eq!(apply_in(&base, &usual), Some(Ok(target.clone())));
        assert_eq!(folder_in(&base, &usual), target);
        for f in [
            "workspace.json",
            ".computer-id",
            "espaces/a/workspace.json",
            "backups/b.txt",
        ] {
            assert_eq!(fs::read_to_string(target.join(f)).unwrap(), f);
            assert!(usual.join(f).exists(), "{f} stays");
        }
        for f in ["workspace.json.lock", "worktrees", "rtk-home", PARTIAL] {
            assert!(!target.join(f).exists(), "{f} isn't copied");
        }
        assert!(wanted_in(&base).is_none());

        // Back to the usual folder: it has data, used as it is.
        fs::write(target.join("workspace.json"), "newer").unwrap();
        assert_eq!(check(&target, &usual, &usual), Ok(Plan::UseExisting));
        request_in(&base, &target, &usual, &usual).unwrap();
        assert_eq!(apply_in(&base, &usual), Some(Ok(usual.clone())));
        assert!(chosen_in(&base).is_none());
        let kept = fs::read_to_string(usual.join("workspace.json")).unwrap();
        assert_eq!(kept, "workspace.json");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn refused_folders() {
        let (base, usual) = setup();
        let full = base.join("documents");
        fs::create_dir_all(&full).unwrap();
        fs::write(full.join("lettre.txt"), "x").unwrap();
        for bad in [
            PathBuf::from("relatif"),
            usual.clone(),
            usual.join("dedans"),
            base.clone(),
            full.clone(),
            full.join("lettre.txt"),
            base.join("ailleurs/../orchestra/dedans"),
        ] {
            assert!(check(&usual, &bad, &usual).is_err(), "{}", bad.display());
        }
        let empty = base.join("vide");
        fs::create_dir_all(&empty).unwrap();
        fs::write(empty.join(".DS_Store"), "").unwrap();
        assert_eq!(check(&usual, &empty, &usual), Ok(Plan::Copy));
        // A copy that stopped halfway can be made again.
        fs::write(full.join(PARTIAL), "").unwrap();
        assert_eq!(check(&usual, &full, &usual), Ok(Plan::Copy));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn a_request_that_fails_changes_nothing() {
        let (base, usual) = setup();
        let target = base.join("plus-tard");
        request_in(&base, &usual, &target, &usual).unwrap();
        // Someone filled the folder in the meantime.
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("autre.txt"), "x").unwrap();
        assert!(matches!(apply_in(&base, &usual), Some(Err(_))));
        assert_eq!(folder_in(&base, &usual), usual);
        assert!(apply_in(&base, &usual).is_none(), "tried once only");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn a_missing_chosen_folder_falls_back() {
        let (base, usual) = setup();
        let gone = base.join("disque-debranche");
        fs::write(base.join(CHOSEN), gone.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(folder_in(&base, &usual), usual);
        assert_eq!(chosen_in(&base), Some(gone));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn home_is_expanded() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand(" ~/Notes "), home.join("Notes"));
        assert_eq!(expand("~"), home);
        assert_eq!(expand("/tmp/x"), PathBuf::from("/tmp/x"));
    }
}
