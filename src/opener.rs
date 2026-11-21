//! Opening a web page or a folder with the system (`open`, `xdg-open`, `explorer`), in one
//! place. What is opened may come from outside (a GitHub answer, a repository's remote, a
//! shared page): `open` would just as well start an application, run a script or follow a
//! `file:` or custom-scheme link, so only https pages and existing folders or files named
//! in full go through.

use std::path::Path;
use std::process::Command;

fn opener() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    }
}

/// A web page in the browser: an https address only, with a host, and nothing that a
/// shell or the opener could read as something else.
pub fn open_url(url: &str) -> Result<(), String> {
    if !url_ok(url) {
        return Err(crate::trf!(
            "adresse refusée : {url}",
            "address refused: {url}"
        ));
    }
    crate::proc::command(opener())
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn url_ok(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty()
        && !host.starts_with('-')
        && !host.contains('@')
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// A folder (or a file) in the Finder / file manager: a full path that exists.
pub fn open_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || !path.exists() {
        return Err(crate::trf!(
            "introuvable : {}",
            "not found: {}",
            path.display()
        ));
    }
    crate::proc::command(opener())
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The system's "choose a folder" dialog (Finder, Explorer, GTK/KDE), opened on `start` when
/// it is a folder. Blocks until the person picks a folder or cancels (`None`): call it off
/// the interface thread.
pub fn pick_folder(prompt: &str, start: Option<&Path>) -> Option<std::path::PathBuf> {
    let start = start
        .filter(|p| p.is_absolute() && p.is_dir())
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let out = pick_folder_cmd(prompt, &start)?;
    let s = String::from_utf8_lossy(&out)
        .trim_end_matches(['\r', '\n'])
        .to_string();
    // Finder answers "/Users/me/Code/x/": keep the folder without its last slash.
    let t = s.trim_end_matches('/');
    let p = std::path::PathBuf::from(if t.is_empty() { "/" } else { t });
    (p.is_absolute() && p.is_dir()).then_some(p)
}

fn run_out(cmd: &mut Command) -> Option<Vec<u8>> {
    let o = cmd
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    (o.status.success() && !o.stdout.is_empty()).then_some(o.stdout)
}

#[cfg(target_os = "macos")]
fn pick_folder_cmd(prompt: &str, start: &str) -> Option<Vec<u8>> {
    // The prompt and the start folder go in as arguments, never inside the script.
    let script = "on run argv\n\
        set p to item 1 of argv\n\
        set s to item 2 of argv\n\
        activate\n\
        if s is \"\" then\n\
        return POSIX path of (choose folder with prompt p)\n\
        else\n\
        return POSIX path of (choose folder with prompt p default location (POSIX file s))\n\
        end if\n\
        end run";
    run_out(crate::proc::command("osascript").args(["-e", script, prompt, start]))
}

#[cfg(windows)]
fn pick_folder_cmd(prompt: &str, start: &str) -> Option<Vec<u8>> {
    let script = "Add-Type -AssemblyName System.Windows.Forms; \
        $d = New-Object System.Windows.Forms.FolderBrowserDialog; \
        $d.Description = $env:ORCHESTRA_PICK_PROMPT; \
        if ($env:ORCHESTRA_PICK_START) { $d.SelectedPath = $env:ORCHESTRA_PICK_START }; \
        if ($d.ShowDialog() -eq 'OK') { [Console]::OutputEncoding = [Text.Encoding]::UTF8; Write-Output $d.SelectedPath }";
    run_out(
        crate::proc::command("powershell")
            .args(["-NoProfile", "-STA", "-Command", script])
            .env("ORCHESTRA_PICK_PROMPT", prompt)
            .env("ORCHESTRA_PICK_START", start),
    )
}

#[cfg(not(any(target_os = "macos", windows)))]
fn pick_folder_cmd(prompt: &str, start: &str) -> Option<Vec<u8>> {
    let mut zen = crate::proc::command("zenity");
    zen.args(["--file-selection", "--directory", "--title", prompt]);
    if !start.is_empty() {
        zen.arg(format!("--filename={start}/"));
    }
    // zenity there (even if cancelled) settles it; only a missing zenity falls back to kdialog.
    if let Ok(o) = zen
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    {
        return (o.status.success() && !o.stdout.is_empty()).then_some(o.stdout);
    }
    let home = dirs::home_dir()
        .map(|h| h.display().to_string())
        .unwrap_or_else(|| "/".into());
    run_out(crate::proc::command("kdialog").args([
        "--getexistingdirectory",
        if start.is_empty() { &home } else { start },
        "--title",
        prompt,
    ]))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_https_pages() {
        for ok in [
            "https://github.com/Bwwwah/orchestra",
            "https://cli.github.com",
            "https://github.com/a/b/actions/runs/1?x=1#y",
        ] {
            assert!(super::url_ok(ok), "{ok}");
        }
        for bad in [
            "http://example.com",
            "file:///Applications/Calculator.app",
            "javascript:alert(1)",
            "x-apple.systempreferences:com.apple.preference",
            "https://",
            "https:///etc/passwd",
            "https://-a",
            "https://user@evil.example/",
            "https://a.com/ b",
            "https://a.com/\nb",
            "/Applications/Calculator.app",
            "-a Calculator",
        ] {
            assert!(!super::url_ok(bad), "{bad}");
        }
    }

    #[test]
    fn only_existing_full_paths() {
        assert!(super::open_path(std::path::Path::new("relatif/dossier")).is_err());
        assert!(super::open_path(std::path::Path::new("/nulle/part/ici-42")).is_err());
    }
}
