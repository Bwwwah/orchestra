//! Keeps the computer awake while autonomous sessions run, so that they go on with the
//! screen off or (on mains power, on macOS) the lid closed. A switched-off computer runs
//! nothing: this only stops it from going to sleep.
//!
//! The system's own tool holds the wake lock (`caffeinate` on macOS, `systemd-inhibit` on
//! Linux) and is told to follow Orchestra's process, so the lock goes away with Orchestra
//! even if it crashes.

use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

/// The least time between two starts of the tool: one that fails or stops at once is not
/// started again on every frame (60 processes a second).
const RETRY_AFTER: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Awake {
    child: Option<Child>,
    last_start: Option<Instant>,
}

impl Awake {
    /// Holds the wake lock while `on`, lets it go otherwise. Cheap to call every frame.
    pub fn set(&mut self, on: bool) {
        if let Some(c) = self.child.as_mut() {
            // The tool may have stopped on its own: start it again, a minute later at most.
            if !matches!(c.try_wait(), Ok(None)) {
                self.child = None;
            }
        }
        match (on, self.child.is_some()) {
            (true, false) if may_start(self.last_start, Instant::now()) => {
                self.last_start = Some(Instant::now());
                self.child = spawn();
            }
            (false, true) => self.release(),
            _ => {}
        }
    }

    pub fn held(&self) -> bool {
        self.child.is_some()
    }

    fn release(&mut self) {
        // Let go on purpose: the next start needn't wait (only a tool that failed does).
        self.last_start = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        self.release();
    }
}

/// The command that holds the lock while process `pid` lives, if this system has one.
pub fn command(pid: u32) -> Option<Vec<String>> {
    if cfg!(target_os = "macos") {
        // -i: no idle sleep; -s: no system sleep on mains power (lid closed too);
        // -m: the disk keeps spinning; -w: until Orchestra ends. The screen may still sleep.
        Some(
            ["caffeinate", "-i", "-s", "-m", "-w"]
                .iter()
                .map(|s| s.to_string())
                .chain([pid.to_string()])
                .collect(),
        )
    } else if cfg!(target_os = "linux") {
        let script = format!("while kill -0 {pid} 2>/dev/null; do sleep 30; done");
        Some(
            [
                "systemd-inhibit",
                "--what=sleep:idle:handle-lid-switch",
                "--who=Orchestra",
                "--why=Sessions IA autonomes en cours",
                "sh",
                "-c",
            ]
            .iter()
            .map(|s| s.to_string())
            .chain([script])
            .collect(),
        )
    } else {
        None
    }
}

fn may_start(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.duration_since(t) >= RETRY_AFTER)
}

fn spawn() -> Option<Child> {
    let argv = command(std::process::id())?;
    crate::proc::command(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_follows_the_process() {
        if let Some(argv) = super::command(4242) {
            assert!(argv.iter().any(|a| a.contains("4242")));
            #[cfg(target_os = "macos")]
            assert_eq!(argv[..5], ["caffeinate", "-i", "-s", "-m", "-w"]);
        }
    }

    /// A tool that fails or stops at once is started again a minute later, not every frame.
    #[test]
    fn a_failing_tool_is_not_started_in_a_loop() {
        use std::time::{Duration, Instant};
        let t = Instant::now();
        assert!(super::may_start(None, t));
        assert!(!super::may_start(Some(t), t + Duration::from_millis(16)));
        assert!(!super::may_start(Some(t), t + Duration::from_secs(59)));
        assert!(super::may_start(Some(t), t + Duration::from_secs(60)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn lock_is_taken_and_given_back() {
        let mut a = super::Awake::default();
        a.set(true);
        assert!(a.held());
        a.set(true);
        assert!(a.held());
        a.set(false);
        assert!(!a.held());
        // Taken again at once after letting go on purpose.
        a.set(true);
        assert!(a.held());
        a.set(false);
    }
}
