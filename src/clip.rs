//! Images for the AI: a picture copied to the clipboard (a screenshot…) saved as a PNG file,
//! so that its path can be given to Claude Code or Codex. egui only pastes text, so the
//! system's own tools read the clipboard.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Where pasted images are kept (the system's temporary folder).
pub fn images_dir() -> PathBuf {
    std::env::temp_dir().join("orchestra-images")
}

/// Save the clipboard's image as a new PNG file and return its path.
pub fn paste_image() -> Result<PathBuf, String> {
    let dir = images_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    let ok = save_clipboard(&out);
    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    if ok && size > 0 {
        Ok(out)
    } else {
        let _ = std::fs::remove_file(&out);
        Err(crate::tr!("le presse-papiers ne contient pas d'image", "the clipboard holds no image").into())
    }
}

#[cfg(target_os = "macos")]
fn save_clipboard(out: &std::path::Path) -> bool {
    // AppleScript writes the clipboard's PNG data to the file; the path goes in as an argument.
    let script = "on run argv\n\
        set f to open for access (POSIX file (item 1 of argv)) with write permission\n\
        try\n\
        write (the clipboard as «class PNGf») to f\n\
        end try\n\
        close access f\n\
        end run";
    Command::new("osascript").arg("-e").arg(script).arg(out).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

#[cfg(target_os = "windows")]
fn save_clipboard(out: &std::path::Path) -> bool {
    let script = "Add-Type -AssemblyName System.Windows.Forms; $i = [System.Windows.Forms.Clipboard]::GetImage(); if ($i) { $i.Save($args[0], [System.Drawing.Imaging.ImageFormat]::Png) } else { exit 1 }";
    Command::new("powershell").args(["-NoProfile", "-STA", "-Command", script]).arg(out).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn save_clipboard(out: &std::path::Path) -> bool {
    // Wayland first, then X11.
    let tries: [(&str, &[&str]); 2] = [("wl-paste", &["--type", "image/png"]), ("xclip", &["-selection", "clipboard", "-t", "image/png", "-o"])];
    for (cmd, args) in tries {
        let Ok(file) = std::fs::File::create(out) else { return false };
        if Command::new(cmd).args(args).stdout(file).stderr(Stdio::null()).status().is_ok_and(|s| s.success()) {
            return true;
        }
    }
    false
}
