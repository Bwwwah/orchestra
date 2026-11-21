//! Updates from the project's GitHub repository: the newest release (the one "Publier une
//! version" creates, with the apps that the repository's workflow builds) is compared with
//! this build's version; when newer, its file for this system is downloaded, put in place of
//! the running app, and the app restarts.
//!
//! The repository is the one the app was built from (ORCHESTRA_UPDATE_REPO at build time,
//! which the GitHub workflow sets), or the one chosen in the settings. A private repository
//! works through `gh` when signed in (each friend needs access to it); a public one works
//! with no account at all.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The repository built into the app (owner/name), if any.
pub fn built_in_repo() -> Option<&'static str> {
    option_env!("ORCHESTRA_UPDATE_REPO").filter(|r| r.contains('/'))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub tag: String,
    pub notes: String,
    /// (file name, download url)
    pub assets: Vec<(String, String)>,
}

impl Release {
    /// This system's file.
    pub fn asset(&self) -> Option<&(String, String)> {
        let suffix = if cfg!(target_os = "macos") {
            "-macOS.zip"
        } else if cfg!(windows) {
            "-Windows.zip"
        } else {
            "-Linux.tar.gz"
        };
        self.assets.iter().find(|(n, _)| n.ends_with(suffix))
    }
}

fn run(bin: &str, args: &[&str], dir: Option<&Path>) -> Result<String, String> {
    let p = crate::sync::shell_path();
    let exe = crate::sync::which(bin, &p).ok_or_else(|| format!("« {bin} » introuvable"))?;
    let mut c = crate::proc::command(exe);
    c.args(args)
        .env("PATH", &p)
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null());
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let o = c.output().map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&o.stderr)
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("échec")
            .trim()
            .chars()
            .take(200)
            .collect())
    }
}

fn gh_ready() -> bool {
    run("gh", &["auth", "status"], None).is_ok()
}

fn parse(v: &Value) -> Option<Release> {
    let tag = v["tag_name"].as_str()?.to_string();
    let assets = v["assets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some((
                        x["name"].as_str()?.to_string(),
                        x["browser_download_url"].as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Release {
        tag,
        notes: v["body"].as_str().unwrap_or("").trim().to_string(),
        assets,
    })
}

/// The newest release of `repo` (owner/name).
pub fn latest(repo: &str) -> Result<Release, String> {
    let repo = repo
        .trim()
        .trim_start_matches("https://github.com/")
        .trim_end_matches('/')
        .trim_end_matches(".git");
    if repo.split('/').count() != 2 {
        return Err(crate::trf!(
            "« {repo} » : attendu propriétaire/dépôt",
            "« {repo} »: expected owner/repository"
        ));
    }
    let path = format!("repos/{repo}/releases/latest");
    let text = if gh_ready() {
        run("gh", &["api", &path], None)?
    } else {
        run("curl", &["-fsSL", "-H", "Accept: application/vnd.github+json", &format!("https://api.github.com/{path}")], None)
            .map_err(|e| crate::trf!("dépôt introuvable ou privé ({e}) ; pour un dépôt privé, connectez « gh »", "repository not found or private ({e}); for a private repository, sign in with « gh »"))?
    };
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    parse(&v)
        .ok_or_else(|| crate::tr!("aucune version publiée", "no version published").to_string())
}

/// `tag` (v1.2.3) is newer than `current` (1.2.0).
pub fn newer(tag: &str, current: &str) -> bool {
    let n = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|x| x.parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (n(tag), n(current));
    (0..3)
        .map(|i| {
            (
                a.get(i).copied().unwrap_or(0),
                b.get(i).copied().unwrap_or(0),
            )
        })
        .find(|(x, y)| x != y)
        .is_some_and(|(x, y)| x > y)
}

/// What gets replaced: the .app bundle on macOS, the program elsewhere. None when running
/// from the source (cargo), which updates with git pull + cargo build instead.
pub fn target() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    if exe.components().any(|c| c.as_os_str() == "target")
        && exe.ancestors().any(|a| a.join("Cargo.toml").is_file())
    {
        return None;
    }
    if cfg!(target_os = "macos") {
        let bundle = exe.parent()?.parent()?.parent()?.to_path_buf();
        return bundle
            .extension()
            .is_some_and(|e| e == "app")
            .then_some(bundle);
    }
    Some(exe)
}

/// Download, unpack and put the new version in place. Returns what to start afterwards.
pub fn install(repo: &str, rel: &Release) -> Result<PathBuf, String> {
    let target = target().ok_or(crate::tr!(
        "Orchestra tourne depuis son code source : mettez-le à jour avec git pull puis cargo build.",
        "Orchestra runs from its source code: update it with git pull then cargo build."
    ))?;
    install_to(&target, repo, rel)
}

fn install_to(target: &Path, repo: &str, rel: &Release) -> Result<PathBuf, String> {
    let target = target.to_path_buf();
    // Installed by a package (/usr/bin…) or in a folder we can't write: say so first.
    let probe = target.with_file_name(".orchestra-ecriture");
    if std::fs::write(&probe, b"").is_err() {
        return Err(crate::trf!(
            "l'app est installée dans un dossier protégé ({}) : téléchargez la nouvelle version depuis la page GitHub du projet (.deb, installateur…)",
            "the app is installed in a protected folder ({}): download the new version from the project's GitHub page (.deb, installer…)",
            target
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ));
    }
    let _ = std::fs::remove_file(&probe);
    let (name, url) = rel
        .asset()
        .ok_or(crate::tr!(
            "cette version n'a pas de fichier pour ce système",
            "this version has no file for this system"
        ))?
        .clone();
    // The tag and the file name come from the network and become paths and command
    // arguments: only plain names are accepted.
    if !safe_name(&rel.tag) || !safe_name(&name) {
        return Err(crate::tr!(
            "nom de version ou de fichier invalide : mise à jour refusée",
            "invalid version or file name: update refused"
        )
        .to_string());
    }
    let tmp = std::env::temp_dir().join(format!("orchestra-maj-{}", rel.tag));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let file = fetch(repo, rel, &name, &url, &tmp)?;
    // Integrity: the release publishes "<file>.sha256"; without it, or if it differs, nothing
    // is installed.
    let sum_name = format!("{name}.sha256");
    let (_, sum_url) = rel.assets.iter().find(|(n, _)| *n == sum_name).cloned().ok_or(crate::tr!(
        "cette version ne publie pas d'empreinte SHA-256 : mise à jour refusée, téléchargez-la depuis la page GitHub du projet",
        "this version publishes no SHA-256 checksum: update refused, download it from the project's GitHub page"
    ))?;
    let sum_file = fetch(repo, rel, &sum_name, &sum_url, &tmp)?;
    let expected = std::fs::read_to_string(&sum_file)
        .ok()
        .and_then(|t| expected_sha256(&t))
        .ok_or(crate::tr!(
            "empreinte SHA-256 illisible",
            "unreadable SHA-256 checksum"
        ))?;
    let got = sha256_file(&file)?;
    if got != expected {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(crate::tr!(
            "le fichier téléchargé ne correspond pas à son empreinte SHA-256 : mise à jour refusée",
            "the downloaded file doesn't match its SHA-256 checksum: update refused"
        )
        .to_string());
    }
    // Unpack.
    let out = tmp.join("x");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let (f, o) = (file.display().to_string(), out.display().to_string());
    if name.ends_with(".tar.gz") {
        run("tar", &["-xzf", &f, "-C", &o], None)?;
    } else if cfg!(target_os = "macos") {
        run("ditto", &["-x", "-k", &f, &o], None)?;
    } else if cfg!(windows) {
        let (f, o) = (ps_quote(&f), ps_quote(&o));
        run(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                &format!("Expand-Archive -Force -LiteralPath {f} -DestinationPath {o}"),
            ],
            None,
        )?;
    } else {
        run("unzip", &["-q", "-o", &f, "-d", &o], None)?;
    }
    let new = find(&out, |p| {
        let n = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if cfg!(target_os = "macos") {
            n.ends_with(".app") && p.is_dir()
        } else if cfg!(windows) {
            n.eq_ignore_ascii_case("orchestra.exe")
        } else {
            n == "orchestra" && p.is_file()
        }
    })
    .ok_or(crate::tr!(
        "le fichier téléchargé ne contient pas l'app",
        "the downloaded file doesn't contain the app"
    ))?;
    // Signature: once the installed app is signed with a Developer ID, the new one must be
    // validly signed by the same Apple team (docs/permissions-macos.md).
    if cfg!(target_os = "macos") {
        let current = signing_team(&target);
        let got = if current.is_some() {
            verified_team(&new)
        } else {
            None
        };
        if !signature_allows(current.as_deref(), got.as_deref()) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(crate::tr!(
                "la nouvelle version n'est pas signée par le même développeur Apple que l'app installée : mise à jour refusée",
                "the new version isn't signed by the same Apple developer as the installed app: update refused"
            )
            .to_string());
        }
    }
    // Swap: the old one is renamed first (a running program can be renamed everywhere).
    let old = target.with_file_name(format!(
        "{}.ancienne",
        target.file_name().unwrap().to_string_lossy()
    ));
    if old.exists() {
        let _ = if old.is_dir() {
            std::fs::remove_dir_all(&old)
        } else {
            std::fs::remove_file(&old)
        };
    }
    std::fs::rename(&target, &old).map_err(|e| {
        crate::trf!(
            "impossible de remplacer l'app ({e})",
            "couldn't replace the app ({e})"
        )
    })?;
    let placed = if cfg!(target_os = "macos") {
        run(
            "ditto",
            &[&new.display().to_string(), &target.display().to_string()],
            None,
        )
        .map(|_| ())
    } else {
        std::fs::copy(&new, &target)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    if let Err(e) = placed {
        let _ = std::fs::rename(&old, &target);
        return Err(crate::trf!(
            "impossible d'installer la nouvelle version ({e}) ; l'ancienne est remise",
            "couldn't install the new version ({e}); the old one is back"
        ));
    }
    #[cfg(unix)]
    if !cfg!(target_os = "macos") {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
        // RTK, shipped next to the program in the Linux archive (macOS: inside the .app,
        // replaced with it): placed beside it too, best effort (the sessions run without it).
        if let (Some(src), Some(dir)) = (new.parent().map(|d| d.join("rtk")), target.parent())
            && src.is_file()
        {
            let dst = dir.join("rtk");
            let tmp_dst = dir.join("rtk.nouveau");
            if std::fs::copy(&src, &tmp_dst).is_ok() {
                let _ = std::fs::set_permissions(&tmp_dst, std::fs::Permissions::from_mode(0o755));
                let _ = std::fs::rename(&tmp_dst, &dst);
            }
        }
    }
    if cfg!(target_os = "macos") {
        let _ = run(
            "xattr",
            &["-dr", "com.apple.quarantine", &target.display().to_string()],
            None,
        );
    }
    let _ = if old.is_dir() {
        std::fs::remove_dir_all(&old)
    } else {
        std::fs::remove_file(&old)
    };
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(target)
}

/// Download one file of the release into `dir`: gh for a private repository, curl otherwise.
fn fetch(repo: &str, rel: &Release, name: &str, url: &str, dir: &Path) -> Result<PathBuf, String> {
    let file = dir.join(name);
    let got = if gh_ready() {
        run(
            "gh",
            &[
                "release",
                "download",
                &rel.tag,
                "-R",
                repo.trim(),
                "-p",
                name,
                "-D",
                &dir.display().to_string(),
                "--clobber",
            ],
            None,
        )
        .map(|_| ())
    } else {
        Err(String::new())
    };
    if got.is_err() || !file.is_file() {
        run(
            "curl",
            &["-fsSL", "-o", &file.display().to_string(), "--url", url],
            None,
        )
        .map_err(|e| crate::trf!("téléchargement impossible : {e}", "download failed: {e}"))?;
    }
    Ok(file)
}

/// A release tag or file name that is safe as a path component and a command argument:
/// [A-Za-z0-9._-] only, not starting with "-" or ".", at most 128 characters.
fn safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with(['-', '.'])
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A PowerShell single-quoted literal ('' stands for a quote inside).
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The checksum in a "<hex>  <file>" line (sha256sum format), lowercase.
fn expected_sha256(text: &str) -> Option<String> {
    let h = text.split_whitespace().next()?.to_ascii_lowercase();
    (h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).then_some(h)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The Apple team that signed `app` (codesign's TeamIdentifier), None when ad hoc or unsigned.
fn signing_team(app: &Path) -> Option<String> {
    let o = crate::proc::command("/usr/bin/codesign")
        .arg("-dv")
        .arg(app)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    // codesign -d writes its details on stderr.
    team_id(&String::from_utf8_lossy(&o.stderr))
}

/// The signing team of `app`, only when its signature is valid for the whole bundle.
fn verified_team(app: &Path) -> Option<String> {
    let ok = crate::proc::command("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .stdin(Stdio::null())
        .output()
        .ok()?
        .status
        .success();
    if ok { signing_team(app) } else { None }
}

/// "TeamIdentifier=ABCDE12345" in codesign -dv's output; None for "not set" or missing.
fn team_id(text: &str) -> Option<String> {
    let v = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("TeamIdentifier="))?
        .trim();
    (!v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| v.to_string())
}

/// An installed app without a team (ad hoc) accepts any new version, so the first one signed
/// with a Developer ID can come in; after that, only the same team.
fn signature_allows(current: Option<&str>, new: Option<&str>) -> bool {
    match current {
        None => true,
        Some(c) => new == Some(c),
    }
}

fn find(dir: &Path, ok: impl Fn(&Path) -> bool + Copy) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if ok(&p) {
            return Some(p);
        }
        if p.is_dir()
            && p.extension().is_none_or(|x| x != "app")
            && let Some(f) = find(&p, ok)
        {
            return Some(f);
        }
    }
    None
}

/// Start the new version (then the caller quits).
pub fn restart(target: &Path) {
    if cfg!(target_os = "macos") {
        let _ = crate::proc::command("open").arg("-n").arg(target).spawn();
    } else {
        let _ = crate::proc::command(target).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn installs_over_the_program() {
        let base = std::env::temp_dir().join(format!("orch-upd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("pack/orchestra")).unwrap();
        std::fs::write(
            base.join("pack/orchestra/orchestra"),
            "#!/bin/sh\necho nouvelle\n",
        )
        .unwrap();
        let tar = base.join("Orchestra-v9.0.0-Linux.tar.gz");
        assert!(
            crate::proc::command("tar")
                .args([
                    "-czf",
                    &tar.display().to_string(),
                    "-C",
                    &base.join("pack").display().to_string(),
                    "orchestra"
                ])
                .status()
                .unwrap()
                .success()
        );
        let app = base.join("installed/orchestra");
        std::fs::create_dir_all(app.parent().unwrap()).unwrap();
        std::fs::write(&app, "ancienne").unwrap();
        let sum = base.join("Orchestra-v9.0.0-Linux.tar.gz.sha256");
        std::fs::write(
            &sum,
            format!(
                "{}  Orchestra-v9.0.0-Linux.tar.gz\n",
                sha256_file(&tar).unwrap()
            ),
        )
        .unwrap();
        let mut rel = Release {
            tag: "v9.0.0".into(),
            notes: String::new(),
            assets: vec![(
                "Orchestra-v9.0.0-Linux.tar.gz".into(),
                format!("file://{}", tar.display()),
            )],
        };
        // No checksum published: refused.
        assert!(install_to(&app, "moi/orchestra", &rel).is_err());
        assert_eq!(std::fs::read_to_string(&app).unwrap(), "ancienne");
        // Wrong checksum: refused.
        let bad = base.join("bad.sha256");
        std::fs::write(&bad, format!("{}  x\n", "0".repeat(64))).unwrap();
        rel.assets.push((
            "Orchestra-v9.0.0-Linux.tar.gz.sha256".into(),
            format!("file://{}", bad.display()),
        ));
        assert!(install_to(&app, "moi/orchestra", &rel).is_err());
        assert_eq!(std::fs::read_to_string(&app).unwrap(), "ancienne");
        rel.assets[1].1 = format!("file://{}", sum.display());
        let got = install_to(&app, "moi/orchestra", &rel).unwrap();
        assert_eq!(got, app);
        assert!(std::fs::read_to_string(&app).unwrap().contains("nouvelle"));
        assert!(!app.with_file_name("orchestra.ancienne").exists());
        let out = crate::proc::command(&app).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "nouvelle");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn names_are_sanitized() {
        assert!(safe_name("v0.6.0"));
        assert!(safe_name("Orchestra-v0.6.0-Linux.tar.gz"));
        assert!(safe_name("Orchestra-v0.6.0-macOS.zip.sha256"));
        for bad in [
            "", "..", "../x", "a/b", "a\\b", "-x", ".hidden", "v1'; rm", "v1 x", "v1$(x)", "é",
        ] {
            assert!(!safe_name(bad), "{bad}");
        }
        assert!(!safe_name(&"a".repeat(129)));
        assert_eq!(ps_quote("C:\\a'b"), "'C:\\a''b'");
        let rel = Release {
            tag: "../../x".into(),
            notes: String::new(),
            assets: vec![],
        };
        let dir = std::env::temp_dir().join(format!("orch-upd-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut r = rel.clone();
        r.assets.push(("Orchestra-x-macOS.zip".into(), "u".into()));
        r.assets
            .push(("Orchestra-x-Linux.tar.gz".into(), "u".into()));
        r.assets
            .push(("Orchestra-x-Windows.zip".into(), "u".into()));
        let e = install_to(&dir.join("orchestra"), "moi/orchestra", &r).unwrap_err();
        assert!(e.contains("invalid") || e.contains("invalide"), "{e}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn signing_team_rules() {
        let out = "Executable=/A/Orchestra.app/Contents/MacOS/orchestra\nIdentifier=app.orchestra.desktop\nTeamIdentifier=ABCDE12345\nSealed Resources version=2\n";
        assert_eq!(team_id(out).as_deref(), Some("ABCDE12345"));
        assert_eq!(
            team_id("Identifier=app.orchestra.desktop\nTeamIdentifier=not set\n"),
            None
        );
        assert_eq!(team_id("Signature=adhoc\n"), None);
        assert_eq!(team_id("TeamIdentifier=\n"), None);
        // Ad hoc installed: anything goes, so the first Developer ID version can come in.
        assert!(signature_allows(None, None));
        assert!(signature_allows(None, Some("ABCDE12345")));
        // Developer ID installed: only the same team, validly signed.
        assert!(signature_allows(Some("ABCDE12345"), Some("ABCDE12345")));
        assert!(!signature_allows(Some("ABCDE12345"), Some("ZZZZZ99999")));
        assert!(!signature_allows(Some("ABCDE12345"), None));
        // A folder that isn't a signed app has no team.
        assert_eq!(signing_team(&std::env::temp_dir()), None);
    }

    #[test]
    fn checksums() {
        let h = "AB".repeat(32);
        assert_eq!(
            expected_sha256(&format!("{h}  Orchestra-v1-macOS.zip\n")),
            Some("ab".repeat(32))
        );
        assert_eq!(expected_sha256("abc  x"), None);
        assert_eq!(expected_sha256(&format!("{}  x", "g".repeat(64))), None);
        assert_eq!(expected_sha256(""), None);
        let f = std::env::temp_dir().join(format!("orch-sha-{}", std::process::id()));
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(
            sha256_file(&f).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(f);
    }

    #[test]
    fn versions_and_assets() {
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(newer("v1.0.0", "0.9.9"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("v0.1.0", "0.2.0"));
        let v = serde_json::json!({"tag_name": "v0.2.0", "body": "- x", "assets": [
            {"name": "Orchestra-v0.2.0-macOS.zip", "browser_download_url": "https://a/m"},
            {"name": "Orchestra-v0.2.0-Linux.tar.gz", "browser_download_url": "https://a/l"},
            {"name": "Orchestra-v0.2.0-Windows.zip", "browser_download_url": "https://a/w"}]});
        let r = parse(&v).unwrap();
        assert_eq!(r.tag, "v0.2.0");
        assert!(r.asset().is_some());
    }
}
