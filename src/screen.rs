//! Screenshots for the AI (MCP tool `take_screenshot`): the whole screen, or the window of
//! one app (Orchestra itself or any other), so that an AI can check what it changed. The
//! system's own tools take the picture; on macOS the app that runs the AI (the terminal,
//! Orchestra…) needs the « Screen Recording » permission.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

/// Pictures larger than this are given by path only, not inline.
const INLINE_MAX: u64 = 3_500_000;

pub struct Shot {
    pub path: PathBuf,
    /// What was captured, said in one sentence (and any fallback taken).
    pub note: String,
}

impl Shot {
    /// The picture as base64, when small enough to send inline.
    pub fn inline(&self) -> Option<(String, &'static str)> {
        let len = std::fs::metadata(&self.path).ok()?.len();
        if len == 0 || len > INLINE_MAX {
            return None;
        }
        let bytes = std::fs::read(&self.path).ok()?;
        Some((base64(&bytes), mime(&self.path)))
    }
}

fn mime(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "jpg" | "jpeg" => "image/jpeg",
        _ => "image/png",
    }
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// The app's bundle identifier (packaging/Info.plist): macOS files its permissions under it.
pub const BUNDLE_ID: &str = "app.orchestra.desktop";

/// Whether this app may record the screen. On macOS, the AIs that Orchestra starts (the
/// autonomous sessions, in its own hidden terminal) take their screenshots under its name:
/// without the « Screen Recording » permission, `take_screenshot` gets no picture. When
/// missing, the system is asked once per launch: it shows its dialog and lists Orchestra in
/// System Settings › Privacy & Security, where the person switches it on.
///
/// `granted_version` is the last version that had the permission (kept in the settings).
/// When an earlier version had it and this one hasn't, the update left a stale entry: still
/// ticked in System Settings but made for the old build's signature (ad hoc: tied to its
/// exact binary, see docs/permissions-macos.md), so it blocks this one without a new
/// dialog. That entry, and only Orchestra's, is reset with `tccutil` before asking again.
pub fn ensure_permission(granted_version: &mut String) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static ASKED: AtomicBool = AtomicBool::new(false);
        let current = crate::update::VERSION;
        // SAFETY: plain CoreGraphics calls with no arguments (macOS 10.15 and later).
        if unsafe { CGPreflightScreenCaptureAccess() } {
            *granted_version = current.to_string();
            return true;
        }
        if ASKED.swap(true, Ordering::SeqCst) {
            return false;
        }
        if stale_after_update(granted_version, current) && in_app_bundle() {
            // Only this app's entry: `tccutil reset ScreenCapture <bundle id>`.
            let _ = run("tccutil", &["reset", "ScreenCapture", BUNDLE_ID]);
            granted_version.clear();
        }
        unsafe { CGRequestScreenCaptureAccess() }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = granted_version;
        true
    }
}

/// The permission (now missing) was there for an earlier version: the entry left by the
/// update is stale.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn stale_after_update(granted_version: &str, current: &str) -> bool {
    !granted_version.is_empty() && granted_version != current
}

/// Running as Orchestra.app (not `cargo run`): only then is the system's entry ours.
#[cfg(target_os = "macos")]
fn in_app_bundle() -> bool {
    std::env::current_exe().map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/")).unwrap_or(false)
}

pub fn shots_dir() -> PathBuf {
    std::env::temp_dir().join("orchestra-screenshots")
}

/// How long a screenshot is kept: long enough for the AI that asked to read it again.
const KEEP_FOR: Duration = Duration::from_secs(24 * 3600);

/// Delete the pictures of `dir` older than `max_age` (by modification time). Only .png and
/// .jpg files: nothing else in the folder is touched. Returns how many went.
fn prune_old(dir: &Path, max_age: Duration, now: SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for e in entries.flatten() {
        let path = e.path();
        let picture = matches!(path.extension().and_then(|x| x.to_str()), Some("png" | "jpg" | "jpeg"));
        let Ok(meta) = e.metadata() else { continue };
        if !picture || !meta.is_file() {
            continue;
        }
        let old = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > max_age);
        if old && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Take a screenshot: of `app`'s front window when given (the app comes to the front), else
/// of the whole main screen.
pub fn take(app: Option<&str>) -> Result<Shot, String> {
    let dir = shots_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    prune_old(&dir, KEEP_FOR, SystemTime::now());
    let app = app.map(str::trim).filter(|a| !a.is_empty());
    let shot = capture(&dir, app)?;
    if std::fs::metadata(&shot.path).map(|m| m.len()).unwrap_or(0) == 0 {
        let _ = std::fs::remove_file(&shot.path);
        return Err(no_picture());
    }
    Ok(shot)
}

fn no_picture() -> String {
    if cfg!(target_os = "macos") {
        "No picture was taken. Give the « Screen Recording » permission (System Settings › Privacy & Security) to the app that runs the AI (Terminal, iTerm, Orchestra…), then try again.".into()
    } else {
        "No picture was taken: no screenshot tool worked on this system.".into()
    }
}

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd).args(args).stdin(Stdio::null()).output().map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!("{cmd}: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn new_file(dir: &Path, ext: &str) -> PathBuf {
    dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()))
}

/// "x,y,w,h" as four whole numbers.
pub fn parse_bounds(s: &str) -> Option<(i64, i64, i64, i64)> {
    let v: Vec<i64> = s.split(',').map(|p| p.trim().parse::<f64>().ok().map(|f| f.round() as i64)).collect::<Option<Vec<_>>>()?;
    match v[..] {
        [x, y, w, h] if w > 0 && h > 0 => Some((x, y, w, h)),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn capture(dir: &Path, app: Option<&str>) -> Result<Shot, String> {
    let path = new_file(dir, "png");
    let p = path.display().to_string();
    let mut note = "the whole main screen".to_string();
    let mut region = None;
    if let Some(app) = app {
        // Bring the app forward, then read its front window's place (needs Accessibility for
        // System Events; without it, the whole screen with the app in front).
        let script = "on run argv\n\
            set n to item 1 of argv\n\
            tell application n to activate\n\
            delay 0.6\n\
            tell application \"System Events\"\n\
            set ps to (every process whose name is n)\n\
            if ps is {} then set ps to (every process whose frontmost is true)\n\
            tell item 1 of ps\n\
            set {x, y} to position of front window\n\
            set {w, h} to size of front window\n\
            end tell\n\
            end tell\n\
            return (x as text) & \",\" & (y as text) & \",\" & (w as text) & \",\" & (h as text)\n\
            end run";
        match run("osascript", &["-e", script, app]) {
            Ok(b) => match parse_bounds(&b) {
                Some((x, y, w, h)) => {
                    region = Some(format!("{x},{y},{w},{h}"));
                    note = format!("the front window of « {app} »");
                }
                None => note = format!("the whole screen, with « {app} » in front (its window couldn't be found)"),
            },
            Err(e) if e.contains("-1728") || e.contains("-10814") || e.contains("Can’t get application") || e.contains("Can't get application") => {
                return Err(format!("No app named « {app} » was found on this Mac."));
            }
            Err(_) => note = format!("the whole screen, with « {app} » in front (give System Events the Accessibility permission to capture only its window)"),
        }
    }
    let mut args = vec!["-x".to_string()];
    if let Some(r) = &region {
        args.push("-R".into());
        args.push(r.clone());
    }
    args.push(p.clone());
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    run("screencapture", &a).map_err(|e| format!("{e}. {}", no_picture()))?;
    // Retina pictures are large: keep at most 1600 pixels on the long side.
    let _ = run("sips", &["-Z", "1600", &p]);
    Ok(Shot { path, note })
}

#[cfg(target_os = "windows")]
fn capture(dir: &Path, app: Option<&str>) -> Result<Shot, String> {
    let path = new_file(dir, "png");
    let script = "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
        $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; \
        $i = New-Object System.Drawing.Bitmap $b.Width, $b.Height; \
        $g = [System.Drawing.Graphics]::FromImage($i); \
        $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size); \
        $i.Save($args[0], [System.Drawing.Imaging.ImageFormat]::Png)";
    let p = path.display().to_string();
    run("powershell", &["-NoProfile", "-Command", script, &p])?;
    let note = match app {
        Some(a) => format!("the whole screen (capturing only « {a} » isn't supported on Windows yet)"),
        None => "the whole main screen".into(),
    };
    Ok(Shot { path, note })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn capture(dir: &Path, app: Option<&str>) -> Result<Shot, String> {
    let path = new_file(dir, "png");
    let p = path.display().to_string();
    // One app's window, with X11 tools.
    if let Some(app) = app {
        if let Ok(ids) = run("xdotool", &["search", "--onlyvisible", "--name", app]) {
            if let Some(id) = ids.lines().next().filter(|l| !l.trim().is_empty()) {
                let _ = run("xdotool", &["windowactivate", "--sync", id]);
                if run("import", &["-window", id, &p]).is_ok() {
                    return Ok(Shot { path, note: format!("the window of « {app} »") });
                }
            }
        }
    }
    let tries: [(&str, Vec<&str>); 4] = [("grim", vec![&p]), ("gnome-screenshot", vec!["-f", &p]), ("scrot", vec!["-o", &p]), ("import", vec!["-window", "root", &p])];
    for (cmd, args) in tries {
        if run(cmd, &args).is_ok() {
            let note = match app {
                Some(a) => format!("the whole screen (the window of « {a} » couldn't be found)"),
                None => "the whole screen".into(),
            };
            return Ok(Shot { path, note });
        }
    }
    Err(no_picture())
}

/// Standard base64 (with padding).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case reported: the permission given to 0.6.4, lost after the update to 0.6.5.
    #[test]
    fn a_stale_entry_is_one_left_by_an_earlier_version() {
        assert!(stale_after_update("0.6.4", "0.6.5"));
        // Never granted (first install), or granted to this very version (the person
        // switched it off): nothing to reset, the system asks as usual.
        assert!(!stale_after_update("", "0.6.5"));
        assert!(!stale_after_update("0.6.5", "0.6.5"));
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn old_screenshots_are_deleted_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("orchestra-shots-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(dir.join("sous-dossier.png")).unwrap();
        let now = SystemTime::now();
        let make = |name: &str, age_h: u64| {
            let p = dir.join(name);
            let f = std::fs::File::create(&p).unwrap();
            f.set_modified(now - Duration::from_secs(age_h * 3600)).unwrap();
            p
        };
        let old_png = make("ancienne.png", 25);
        let old_jpg = make("ancienne.jpg", 48);
        let fresh = make("recente.png", 23);
        let other = make("notes.txt", 100);
        assert_eq!(prune_old(&dir, KEEP_FOR, now), 2);
        assert!(!old_png.exists() && !old_jpg.exists());
        assert!(fresh.exists(), "a picture of the last 24 h stays");
        assert!(other.exists(), "only pictures are deleted");
        assert!(dir.join("sous-dossier.png").is_dir(), "folders are left alone");
        // A missing folder is not an error.
        assert_eq!(prune_old(&dir.join("absent"), KEEP_FOR, now), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn window_bounds_from_applescript() {
        assert_eq!(parse_bounds("10, 25, 800, 600"), Some((10, 25, 800, 600)));
        assert_eq!(parse_bounds("-1440,0,1440,900"), Some((-1440, 0, 1440, 900)));
        assert_eq!(parse_bounds("1,2,0,4"), None);
        assert_eq!(parse_bounds("1,2,3"), None);
        assert_eq!(parse_bounds("a,b,c,d"), None);
    }
}
