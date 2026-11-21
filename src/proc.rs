//! Starting other programs without a console window on Windows.
//!
//! Orchestra is a windowed program there (`windows_subsystem = "windows"`): it has no console
//! of its own, so Windows opens a new console window for every console program it starts
//! (git, gh, powershell, cmd, claude, where…). Several of them run every few seconds (git
//! status, the AIs' checks, the usage readings): windows kept popping up and closing in a
//! loop. `command` starts them with CREATE_NO_WINDOW: no window, the pipes still work.
//! Elsewhere it is plain `Command::new`.
//!
//! A terminal the person must see (« open in a terminal ») keeps `Command::new` on purpose.

use std::ffi::OsStr;
use std::process::Command;

/// CREATE_NO_WINDOW (winbase.h): the console program gets no console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `Command::new(program)`, with no console window on Windows.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

#[cfg(test)]
mod tests {
    #[test]
    fn runs_like_command_new() {
        let prog = if cfg!(windows) { "cmd" } else { "sh" };
        let arg = if cfg!(windows) { "/C" } else { "-c" };
        let out = super::command(prog)
            .args([arg, "echo orchestra"])
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("orchestra"));
    }
}
