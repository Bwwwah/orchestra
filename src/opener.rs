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
        return Err(crate::trf!("adresse refusée : {url}", "address refused: {url}"));
    }
    Command::new(opener()).arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn url_ok(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !host.starts_with('-') && !host.contains('@') && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// A folder (or a file) in the Finder / file manager: a full path that exists.
pub fn open_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || !path.exists() {
        return Err(crate::trf!("introuvable : {}", "not found: {}", path.display()));
    }
    Command::new(opener()).arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_https_pages() {
        for ok in ["https://github.com/Bwwwah/orchestra", "https://cli.github.com", "https://github.com/a/b/actions/runs/1?x=1#y"] {
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
