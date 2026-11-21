//! Images for the AI: a picture copied to the clipboard (a screenshot…) saved as a PNG file,
//! so that its path can be given to Claude Code or Codex. egui only pastes text, so the
//! system's own tools read the clipboard.
//!
//! What the clipboard may hold, and what is done with it:
//! - PNG data (most screenshots): written as is;
//! - another kind of image (TIFF from Preview, a picture copied from a browser, PDF, HEIC…):
//!   converted to PNG (macOS) or saved through the system's bitmap (Windows);
//! - an image file copied in the Finder / Explorer: that file itself is joined.
//!
//! A file is only kept when it really is a PNG with a size (signature and IHDR checked), so
//! that an empty or broken file never reaches the AI.

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Where pasted images are kept (the system's temporary folder).
pub fn images_dir() -> PathBuf {
    std::env::temp_dir().join("orchestra-images")
}

/// What the system's tool found in the clipboard.
#[derive(Debug, PartialEq, Eq)]
enum Grab {
    /// The image was written to the file asked for.
    Saved,
    /// An image file was copied: its path.
    File(PathBuf),
    /// No image.
    Nothing,
}

/// The tool's answer on its standard output: « ok », « file:<path> » or anything else.
fn grab_from(stdout: &str) -> Grab {
    let s = stdout.trim();
    if s == "ok" {
        Grab::Saved
    } else if let Some(p) = s.strip_prefix("file:") {
        Grab::File(PathBuf::from(p.trim()))
    } else {
        Grab::Nothing
    }
}

/// A PNG file with a picture in it: the 8-byte signature, then an IHDR chunk whose width and
/// height aren't zero.
fn is_real_png(bytes: &[u8]) -> bool {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[..8] != SIG || &bytes[12..16] != b"IHDR" {
        return false;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    w > 0 && h > 0
}

fn no_image() -> String {
    crate::tr!(
        "le presse-papiers ne contient pas d'image",
        "the clipboard holds no image"
    )
    .into()
}

/// Save the clipboard's image as a new PNG file (or give the copied image file) and return
/// its path.
pub fn paste_image() -> Result<PathBuf, String> {
    let dir = images_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    let grab = read_clipboard(&out);
    match grab {
        Grab::File(p) if p.is_file() && crate::sync::is_image(&p) => {
            let _ = std::fs::remove_file(&out);
            Ok(p)
        }
        Grab::Saved if std::fs::read(&out).is_ok_and(|b| is_real_png(&b)) => Ok(out),
        _ => {
            let _ = std::fs::remove_file(&out);
            Err(no_image())
        }
    }
}

fn run(cmd: &mut std::process::Command) -> Grab {
    match cmd.stdin(Stdio::null()).stderr(Stdio::null()).output() {
        Ok(o) if o.status.success() => grab_from(&String::from_utf8_lossy(&o.stdout)),
        _ => Grab::Nothing,
    }
}

#[cfg(target_os = "macos")]
fn read_clipboard(out: &Path) -> Grab {
    // AppKit through JavaScript for Automation: the pasteboard's PNG data, else any image
    // it holds converted to PNG; a copied image file is given back by its path (an NSImage
    // made from it would be the file's icon). The path goes in as an argument.
    let script = r#"ObjC.import('AppKit');
function run(argv) {
  const pb = $.NSPasteboard.generalPasteboard;
  const urls = pb.readObjectsForClassesOptions($([$.NSURL]), $({}));
  if (urls && !urls.isNil() && urls.count > 0) {
    const u = urls.objectAtIndex(0);
    if (u.isFileURL) return 'file:' + u.path.js;
  }
  let data = pb.dataForType('public.png');
  if (!data || data.isNil() || data.length == 0) {
    const img = $.NSImage.alloc.initWithPasteboard(pb);
    if (!img || img.isNil()) return 'none';
    const rep = $.NSBitmapImageRep.imageRepWithData(img.TIFFRepresentation);
    if (!rep || rep.isNil()) return 'none';
    data = rep.representationUsingTypeProperties(4, $({}));
  }
  if (!data || data.isNil() || data.length == 0) return 'none';
  return data.writeToFileAtomically(argv[0], true) ? 'ok' : 'none';
}"#;
    match run(crate::proc::command("osascript")
        .args(["-l", "JavaScript", "-e", script])
        .arg(out))
    {
        Grab::Nothing => {
            // The former way, kept as a fallback: AppleScript writes the clipboard's PNG data.
            let script = "on run argv\n\
                set f to open for access (POSIX file (item 1 of argv)) with write permission\n\
                try\n\
                write (the clipboard as «class PNGf») to f\n\
                end try\n\
                close access f\n\
                end run";
            let ok = crate::proc::command("osascript")
                .arg("-e")
                .arg(script)
                .arg(out)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if ok { Grab::Saved } else { Grab::Nothing }
        }
        g => g,
    }
}

#[cfg(target_os = "windows")]
fn read_clipboard(out: &Path) -> Grab {
    // The path goes in through the environment: with -Command, PowerShell joins every
    // following argument into the script itself, so `$args[0]` was always empty and nothing
    // was saved. The « PNG » format first (browsers, the Snipping Tool: it keeps
    // transparency), then the system's bitmap; a copied image file is given back by its path.
    let script = "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; \
        [Console]::OutputEncoding = [Text.Encoding]::UTF8; \
        $out = $env:ORCHESTRA_CLIP_OUT; \
        $files = [System.Windows.Forms.Clipboard]::GetFileDropList(); \
        if ($files -and $files.Count -gt 0) { Write-Output ('file:' + $files[0]); exit 0 }; \
        $d = [System.Windows.Forms.Clipboard]::GetDataObject(); \
        if ($d -and $d.GetDataPresent('PNG')) { $s = $d.GetData('PNG'); $o = [System.IO.File]::Create($out); $s.CopyTo($o); $o.Close(); Write-Output 'ok'; exit 0 }; \
        $i = [System.Windows.Forms.Clipboard]::GetImage(); \
        if ($i) { $i.Save($out, [System.Drawing.Imaging.ImageFormat]::Png); Write-Output 'ok'; exit 0 }; \
        Write-Output 'none'";
    run(crate::proc::command("powershell")
        .args(["-NoProfile", "-STA", "-Command", script])
        .env("ORCHESTRA_CLIP_OUT", out))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_clipboard(out: &Path) -> Grab {
    // Wayland first, then X11.
    let tries: [(&str, &[&str]); 2] = [
        ("wl-paste", &["--type", "image/png"]),
        (
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"],
        ),
    ];
    for (cmd, args) in tries {
        let Ok(file) = std::fs::File::create(out) else {
            return Grab::Nothing;
        };
        if crate::proc::command(cmd)
            .args(args)
            .stdout(file)
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return Grab::Saved;
        }
    }
    Grab::Nothing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tools_answer_is_read() {
        assert_eq!(grab_from("ok\n"), Grab::Saved);
        assert_eq!(
            grab_from("file:/Users/ana/Desktop/capture.png\n"),
            Grab::File(PathBuf::from("/Users/ana/Desktop/capture.png"))
        );
        assert_eq!(grab_from("none"), Grab::Nothing);
        assert_eq!(grab_from(""), Grab::Nothing);
    }

    /// The case reported: « empty PNGs ». Only a PNG with a picture in it is kept.
    #[test]
    fn only_a_real_png_is_kept() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert!(is_real_png(&png));
        assert!(!is_real_png(&[]));
        assert!(!is_real_png(&png[..20]));
        let mut empty = png.clone();
        empty[16..20].copy_from_slice(&0u32.to_be_bytes());
        assert!(!is_real_png(&empty), "a 0×480 picture is no picture");
        let mut tiff = png.clone();
        tiff[..4].copy_from_slice(b"II*\0");
        assert!(!is_real_png(&tiff));
    }
}
