//! A real terminal inside the app: a shell in a pseudo-terminal (portable-pty), its output
//! decoded into a screen grid (vt100), so full-screen programs such as Claude Code or Codex
//! run in it as they would in Terminal.app. The window draws the grid (app/term_ui.rs) and
//! sends the keys typed into it.

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::LockOrRecover;
use std::sync::{Arc, Mutex, OnceLock};

/// The person's shell PATH, looked up once (see `sync::shell_path`).
pub fn path() -> &'static str {
    static P: OnceLock<String> = OnceLock::new();
    P.get_or_init(crate::sync::shell_path)
}

pub struct Term {
    pub title: String,
    pub cwd: String,
    /// What the shell was asked to run first (None: a plain shell).
    pub cmd: Option<String>,
    /// Mouse selection, in screen cells: (row, col) where it started and where it ends.
    pub selection: Option<((u16, u16), (u16, u16))>,
    /// Text to type once the program had time to start (sent from the side panel).
    pub pending: Option<(std::time::Instant, String)>,
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    exited: Arc<AtomicBool>,
    size: (u16, u16),
}

fn nz(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n.max(1)).unwrap()
}

impl Term {
    /// A login shell in `cwd`; with `cmd`, the shell runs it first, then stays open.
    pub fn spawn(title: &str, cwd: &str, cmd: Option<&str>, on_output: impl Fn() + Send + 'static) -> Result<Term, String> {
        Self::spawn_with(title, cwd, cmd, true, on_output)
    }

    /// The shell runs `cmd` and ends with it: `exited()` says when the program has quit
    /// (sessions driven by Orchestra, which must notice it).
    pub fn spawn_once(title: &str, cwd: &str, cmd: &str, on_output: impl Fn() + Send + 'static) -> Result<Term, String> {
        Self::spawn_with(title, cwd, Some(cmd), false, on_output)
    }

    fn spawn_with(title: &str, cwd: &str, cmd: Option<&str>, keep_shell: bool, on_output: impl Fn() + Send + 'static) -> Result<Term, String> {
        let (rows, cols) = (24u16, 80u16);
        let pair = native_pty_system()
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| e.to_string())?;
        #[cfg(windows)]
        let c = {
            // PowerShell; with a command, it runs it then stays open.
            let mut c = CommandBuilder::new("powershell.exe");
            c.arg("-NoLogo");
            if let Some(cmd) = cmd {
                if keep_shell {
                    c.arg("-NoExit");
                }
                c.args(["-Command", cmd]);
            }
            c
        };
        #[cfg(not(windows))]
        let c = {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
            let mut c = CommandBuilder::new(&shell);
            match cmd {
                Some(cmd) if keep_shell => {
                    c.args(["-l", "-c", &format!("{cmd}; exec {} -l", crate::sessions::shell_quote(&shell))]);
                }
                Some(cmd) => {
                    c.args(["-l", "-c", cmd]);
                }
                None => c.arg("-l"),
            }
            c
        };
        let mut c = c;
        let dir = crate::project::normalize_folder(cwd);
        if dir.is_dir() {
            c.cwd(&dir);
        } else if let Some(h) = dirs::home_dir() {
            c.cwd(h);
        }
        c.env("TERM", "xterm-256color");
        c.env("COLORTERM", "truecolor");
        c.env("PATH", path());
        c.env("TERM_PROGRAM", "orchestra");
        if cmd.is_some() {
            // The agents use the account the person logged in with, not stray API keys.
            for k in crate::sync::API_KEY_VARS {
                c.env_remove(k);
            }
        }
        let child = pair.slave.spawn_command(c).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(nz(rows), nz(cols), 5000)));
        let exited = Arc::new(AtomicBool::new(false));
        let (p2, e2) = (parser.clone(), exited.clone());
        std::thread::Builder::new()
            .name("pty".into())
            .spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            p2.lock_or_recover().process(&buf[..n]);
                            on_output();
                        }
                    }
                }
                e2.store(true, Ordering::Relaxed);
                on_output();
            })
            .map_err(|e| e.to_string())?;
        Ok(Term {
            title: title.to_string(),
            cwd: cwd.to_string(),
            cmd: cmd.map(String::from),
            selection: None,
            pending: None,
            parser,
            writer,
            master: pair.master,
            child,
            exited,
            size: (rows, cols),
        })
    }

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<String> {
        let ((r0, c0), (r1, c1)) = ordered(self.selection?);
        if (r0, c0) == (r1, c1) {
            return None;
        }
        Some(self.with_screen(|s| s.contents_between(r0, c0, r1, c1 + 1)))
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.selection = None;
        // Typing brings the view back to the live screen.
        self.parser.lock_or_recover().screen_mut().set_scrollback(0);
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// Paste text, wrapped in bracketed-paste markers when the program asked for them.
    pub fn paste(&mut self, text: &str) {
        let bracketed = self.parser.lock_or_recover().screen().bracketed_paste();
        let text = text.replace("\r\n", "\n").replace('\n', "\r");
        if bracketed {
            self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            self.write(text.as_bytes());
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(10));
        if (rows, cols) == self.size {
            return;
        }
        self.size = (rows, cols);
        self.parser.lock_or_recover().set_size(nz(rows), nz(cols));
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
    }

    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        f(self.parser.lock_or_recover().screen())
    }

    /// The mouse wheel over cell (row, col), `lines` > 0 going up. A program that tracks
    /// the mouse (Claude Code does) gets the wheel and scrolls itself; one on the alternate
    /// screen without it gets arrow keys, like in most terminals; otherwise our history moves.
    pub fn wheel(&mut self, lines: i32, row: u16, col: u16) {
        let (mode, enc, alt) = self.with_screen(|s| (s.mouse_protocol_mode(), s.mouse_protocol_encoding(), s.alternate_screen()));
        match wheel_bytes(mode != vt100::MouseProtocolMode::None, enc == vt100::MouseProtocolEncoding::Sgr, alt, lines, row, col) {
            Some(out) => {
                let _ = self.writer.write_all(&out);
                let _ = self.writer.flush();
            }
            None => self.scroll(lines),
        }
    }

    /// Move through the history: positive goes back in time.
    pub fn scroll(&mut self, rows: i32) {
        self.selection = None;
        let mut p = self.parser.lock_or_recover();
        let cur = p.screen().scrollback() as i32;
        p.screen_mut().set_scrollback((cur + rows).max(0) as usize);
    }

    pub fn exited(&self) -> bool {
        self.exited.load(Ordering::Relaxed)
    }

    /// The whole screen as text (for "copy").
    pub fn text(&self) -> String {
        self.with_screen(|s| s.contents())
    }
}

/// A selection from its start to its end, whichever way it was dragged.
pub fn ordered(sel: ((u16, u16), (u16, u16))) -> ((u16, u16), (u16, u16)) {
    if sel.0 <= sel.1 { sel } else { (sel.1, sel.0) }
}

/// What to run to bring a saved terminal back: agents resume their conversation
/// (`claude --resume <id>` keeps its id; a plain `claude` continues the folder's latest one).
pub fn restore_cmd(cmd: Option<&str>) -> Option<String> {
    let c = cmd?.trim();
    let first = c.split_whitespace().next().unwrap_or("");
    Some(match first {
        "claude" if c.contains("--resume") || c.contains("--continue") => c.to_string(),
        "claude" => "claude --continue".to_string(),
        "codex" if c.starts_with("codex resume") => c.to_string(),
        "codex" => "codex resume --last".to_string(),
        _ => c.to_string(),
    })
}

impl Term {
    /// Stop the shell and everything it started (claude, node, a build…), not only the
    /// shell: otherwise they keep running after Orchestra, and on Windows they hold its
    /// files, which then can't be replaced or uninstalled.
    pub fn stop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(None)) {
            return;
        }
        if let Some(pid) = self.child.process_id() {
            let _ = crate::proc::command(tree_kill_program()).args(tree_kill_args(pid)).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
        }
        let _ = self.child.kill();
    }
}

/// Windows: `taskkill /F /T` ends the process and its whole tree. Elsewhere the shell leads
/// its own session in the pseudo-terminal (portable-pty calls setsid), so its process group
/// holds what it ran: `kill -TERM -- -<pid>` reaches all of it.
fn tree_kill_program() -> &'static str {
    if cfg!(windows) { "taskkill" } else { "kill" }
}

fn tree_kill_args(pid: u32) -> Vec<String> {
    if cfg!(windows) {
        vec!["/F".into(), "/T".into(), "/PID".into(), pid.to_string()]
    } else {
        vec!["-TERM".into(), "--".into(), format!("-{pid}")]
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod stop_tests {
    #[test]
    fn the_whole_tree_is_named() {
        let a = super::tree_kill_args(4242);
        if cfg!(windows) {
            assert_eq!(a, ["/F", "/T", "/PID", "4242"]);
        } else {
            assert_eq!(a, ["-TERM", "--", "-4242"]);
        }
    }

    /// A shell that started a long command: stopping the terminal ends that command too.
    #[cfg(unix)]
    #[test]
    fn stopping_ends_what_the_shell_started() {
        let dir = std::env::temp_dir().join(format!("orchestra-stop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("pid");
        let cmd = format!("sleep 60 & echo $! > '{}'; wait", pidfile.display());
        let mut t = super::Term::spawn_once("test", &dir.display().to_string(), &cmd, || {}).unwrap();
        let mut pid = String::new();
        for _ in 0..100 {
            pid = std::fs::read_to_string(&pidfile).unwrap_or_default().trim().to_string();
            if !pid.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(!pid.is_empty(), "the command didn't start");
        let alive = |p: &str| std::process::Command::new("kill").args(["-0", p]).status().is_ok_and(|s| s.success());
        assert!(alive(&pid));
        t.stop();
        let mut gone = false;
        for _ in 0..40 {
            if !alive(&pid) {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(gone, "sleep {pid} still runs after stop()");
    }
}

/// What the wheel sends to the program: mouse events when it tracks the mouse (SGR or
/// classic encoding), arrow keys on the alternate screen; None: move our own history.
pub fn wheel_bytes(tracking: bool, sgr: bool, alt: bool, lines: i32, row: u16, col: u16) -> Option<Vec<u8>> {
    let n = lines.unsigned_abs().min(10);
    let mut out = Vec::new();
    if tracking {
        let button = if lines > 0 { 64u8 } else { 65u8 };
        let (x, y) = (col as u32 + 1, row as u32 + 1);
        for _ in 0..n {
            if sgr {
                out.extend_from_slice(format!("\x1b[<{button};{x};{y}M").as_bytes());
            } else {
                out.extend_from_slice(b"\x1b[M");
                out.push(32 + button);
                out.push((32 + x).min(255) as u8);
                out.push((32 + y).min(255) as u8);
            }
        }
        Some(out)
    } else if alt {
        for _ in 0..n {
            out.extend_from_slice(if lines > 0 { b"\x1b[A" } else { b"\x1b[B" });
        }
        Some(out)
    } else {
        None
    }
}

/// The bytes a key sends, as xterm does. `app_cursor`: the program asked for the
/// "application" arrow keys (`ESC O A` instead of `ESC [ A`).
pub fn key_bytes(key: egui_keys::Key, ctrl: bool, alt: bool, shift: bool, app_cursor: bool) -> Option<Vec<u8>> {
    use egui_keys::Key as K;
    let arrow = |c: char| {
        let mods = 1 + (shift as u8) + 2 * (alt as u8) + 4 * (ctrl as u8);
        if mods > 1 {
            format!("\x1b[1;{mods}{c}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    let b: Vec<u8> = match key {
        K::Enter => {
            if alt || shift {
                // Newline inside the prompt (Claude Code: Option+Entrée).
                b"\x1b\r".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        K::Tab => if shift { b"\x1b[Z".to_vec() } else { b"\t".to_vec() },
        K::Backspace => if alt { b"\x1b\x7f".to_vec() } else { b"\x7f".to_vec() },
        K::Escape => b"\x1b".to_vec(),
        K::ArrowUp => arrow('A'),
        K::ArrowDown => arrow('B'),
        K::ArrowRight => arrow('C'),
        K::ArrowLeft => arrow('D'),
        K::Home => b"\x1b[H".to_vec(),
        K::End => b"\x1b[F".to_vec(),
        K::PageUp => b"\x1b[5~".to_vec(),
        K::PageDown => b"\x1b[6~".to_vec(),
        K::Delete => b"\x1b[3~".to_vec(),
        K::Insert => b"\x1b[2~".to_vec(),
        other if ctrl => {
            // Ctrl+letter → 0x01..0x1a.
            let name = other.name();
            let c = name.chars().next()?;
            if name.len() == 1 && c.is_ascii_alphabetic() {
                vec![(c.to_ascii_uppercase() as u8) - b'A' + 1]
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(b)
}

/// Re-exported so the key table above can be tested without a window.
pub mod egui_keys {
    pub use eframe::egui::Key;
}

/// A colour of the 256-colour palette, as RGB.
pub fn palette(i: u8, dark: bool) -> (u8, u8, u8) {
    const BASE_DARK: [(u8, u8, u8); 16] = [
        (40, 40, 40), (229, 83, 75), (110, 190, 112), (230, 190, 90), (88, 150, 230), (190, 120, 220), (80, 190, 200), (210, 210, 210),
        (110, 110, 110), (255, 120, 110), (140, 220, 140), (250, 215, 120), (120, 175, 255), (215, 150, 240), (110, 215, 225), (245, 245, 245),
    ];
    const BASE_LIGHT: [(u8, u8, u8); 16] = [
        (30, 30, 30), (200, 40, 40), (40, 140, 60), (170, 120, 0), (30, 90, 200), (150, 60, 180), (20, 140, 150), (190, 190, 190),
        (100, 100, 100), (230, 70, 60), (60, 170, 80), (200, 150, 20), (60, 120, 230), (180, 90, 210), (40, 170, 180), (250, 250, 250),
    ];
    match i {
        0..=15 => if dark { BASE_DARK[i as usize] } else { BASE_LIGHT[i as usize] },
        16..=231 => {
            let n = i - 16;
            let v = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
            (v(n / 36), v((n / 6) % 6), v(n % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            (g, g, g)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_keys::Key;

    #[test]
    fn keys() {
        assert_eq!(key_bytes(Key::Enter, false, false, false, false), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(Key::Tab, false, false, true, false), Some(b"\x1b[Z".to_vec()));
        assert_eq!(key_bytes(Key::ArrowUp, false, false, false, false), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes(Key::ArrowUp, false, false, false, true), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_bytes(Key::ArrowLeft, false, true, false, false), Some(b"\x1b[1;3D".to_vec()));
        assert_eq!(key_bytes(Key::C, true, false, false, false), Some(vec![3]));
        assert_eq!(key_bytes(Key::C, false, false, false, false), None);
    }

    #[test]
    fn restore() {
        assert_eq!(restore_cmd(None), None);
        assert_eq!(restore_cmd(Some("claude")).as_deref(), Some("claude --continue"));
        assert_eq!(restore_cmd(Some("claude --resume abc")).as_deref(), Some("claude --resume abc"));
        assert_eq!(restore_cmd(Some("codex")).as_deref(), Some("codex resume --last"));
        assert_eq!(restore_cmd(Some("codex resume x")).as_deref(), Some("codex resume x"));
        assert_eq!(ordered(((3, 1), (1, 5))), ((1, 5), (3, 1)));
    }

    #[test]
    fn colours() {
        assert_eq!(palette(16, true), (0, 0, 0));
        assert_eq!(palette(231, true), (255, 255, 255));
        assert_eq!(palette(232, true), (8, 8, 8));
    }

    #[test]
    fn shell_runs_in_a_pty() {
        let dir = std::env::temp_dir();
        let mut t = Term::spawn("test", &dir.display().to_string(), Some("printf 'bonjour %s' tableau; exit 0"), || {}).unwrap();
        t.resize(10, 40);
        let start = std::time::Instant::now();
        while !t.text().contains("bonjour tableau") && start.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(t.text().contains("bonjour tableau"), "écran : {:?}", t.text());
        t.write(b"exit\r");
    }
}

#[cfg(all(test, unix))]
mod wheel_tests {
    use super::*;

    fn wait_for(t: &Term, needle: &str) -> bool {
        for _ in 0..60 {
            if t.text().contains(needle) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    }

    #[test]
    fn wheel_goes_to_a_program_that_tracks_the_mouse() {
        assert_eq!(wheel_bytes(true, true, false, 2, 2, 3).unwrap(), b"\x1b[<64;4;3M\x1b[<64;4;3M".to_vec());
        assert_eq!(wheel_bytes(true, false, false, -1, 0, 0).unwrap(), vec![0x1b, b'[', b'M', 32 + 65, 33, 33]);
        assert_eq!(wheel_bytes(false, false, true, -2, 0, 0).unwrap(), b"\x1b[B\x1b[B".to_vec());
        assert!(wheel_bytes(false, false, false, 3, 0, 0).is_none());
    }

    #[test]
    fn wheel_moves_history_otherwise() {
        let dir = std::env::temp_dir().display().to_string();
        let mut t = Term::spawn("t", &dir, Some("seq 1 200"), || {}).unwrap();
        assert!(wait_for(&t, "200"));
        t.wheel(5, 0, 0);
        assert_eq!(t.with_screen(|s| s.scrollback()), 5);
    }
}
