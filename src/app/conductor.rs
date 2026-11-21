//! The conductor: moves the autonomous sessions along while the window isn't drawn.
//!
//! The sessions are driven from the window's frames (`orch_tick`). macOS draws no frame
//! for a window it doesn't show (screen off or locked, window in the Dock, on another
//! desktop), so a session left alone at night stopped with the screen: its turn over, the
//! next one never sent. A thread of its own does that work while no frame comes, on the
//! same app, behind the same lock: the window, the session's details and its terminal stay
//! as they are, and pick up where the conductor left off.

use crate::LockOrRecover;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

/// Without a frame for this long, the window is taken for not drawn. It asks for one every
/// 500 ms while it shows.
pub const STALE: Duration = Duration::from_millis(1500);
/// How often the conductor looks.
pub const EVERY: Duration = Duration::from_millis(500);

/// Whether the conductor steps in: the last frame is older than `stale`.
pub fn due(last_frame: Instant, now: Instant, stale: Duration) -> bool {
    now.saturating_duration_since(last_frame) > stale
}

/// The app, shared by the window and the conductor.
pub struct Shared<T> {
    app: Mutex<T>,
    last_frame: Mutex<Instant>,
    stop: AtomicBool,
}

impl<T: Send + 'static> Shared<T> {
    pub fn new(app: T) -> Arc<Self> {
        Arc::new(Shared {
            app: Mutex::new(app),
            last_frame: Mutex::new(Instant::now()),
            stop: AtomicBool::new(false),
        })
    }

    /// The app for a frame of the window: the conductor stands back meanwhile.
    pub fn frame(&self) -> MutexGuard<'_, T> {
        *self.last_frame.lock_or_recover() = Instant::now();
        self.app.lock_or_recover()
    }

    /// The app outside a frame (closing).
    pub fn app(&self) -> MutexGuard<'_, T> {
        self.app.lock_or_recover()
    }

    /// The conductor stops at its next look.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Start the conductor: every `every`, while no frame came for `stale`, `tick` runs on
    /// the app. A frame in progress holds the app: the conductor skips that look.
    pub fn spawn(
        self: &Arc<Self>,
        every: Duration,
        stale: Duration,
        mut tick: impl FnMut(&mut T) + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        let me = self.clone();
        std::thread::Builder::new()
            .name("chef d'orchestre".into())
            .spawn(move || {
                while !me.stop.load(Ordering::Relaxed) {
                    std::thread::sleep(every);
                    if me.stop.load(Ordering::Relaxed)
                        || !due(*me.last_frame.lock_or_recover(), Instant::now(), stale)
                    {
                        continue;
                    }
                    match me.app.try_lock() {
                        Ok(mut app) => tick(&mut app),
                        // A frame that panicked: the app is still there to drive.
                        Err(TryLockError::Poisoned(e)) => tick(&mut e.into_inner()),
                        Err(TryLockError::WouldBlock) => {}
                    }
                }
            })
            .expect("fil du chef d'orchestre")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn steps_in_only_once_the_last_frame_is_old() {
        let t0 = Instant::now();
        assert!(!due(t0, t0 + 1000 * MS, STALE));
        assert!(due(t0, t0 + 1600 * MS, STALE));
        // A frame stamped after the look (clocks read in another order) isn't old.
        assert!(!due(t0 + 10 * MS, t0, STALE));
    }

    /// While frames come, the conductor does nothing; once they stop (screen off), it moves
    /// the app along by itself; frames back, it stands back again; asked to, it stops.
    #[test]
    fn drives_the_app_while_no_frame_comes() {
        let shared = Shared::new(0u32);
        let handle = shared.spawn(5 * MS, 60 * MS, |n| *n += 1);
        let drawing = |for_ms: u32| {
            let end = Instant::now() + for_ms * MS;
            while Instant::now() < end {
                drop(shared.frame());
                std::thread::sleep(5 * MS);
            }
        };
        drawing(200);
        assert_eq!(*shared.app(), 0, "frames come: the conductor stays out");
        std::thread::sleep(300 * MS);
        let alone = *shared.app();
        assert!(alone > 0, "no frame: the conductor drives");
        drawing(200);
        // At most a look that had started before the frames came back.
        assert!(
            *shared.app() <= alone + 1,
            "frames back: the conductor stands back"
        );
        shared.stop();
        handle.join().unwrap();
    }

    /// A frame that panicked while holding the app: the conductor still drives it.
    #[test]
    fn drives_on_after_a_frame_panicked() {
        let shared = Shared::new(0u32);
        let s = shared.clone();
        let _ = std::thread::spawn(move || {
            let _g = s.frame();
            panic!("image en échec (attendu par le test)");
        })
        .join();
        let handle = shared.spawn(5 * MS, 20 * MS, |n| *n += 1);
        std::thread::sleep(150 * MS);
        assert!(*shared.app() > 0);
        shared.stop();
        handle.join().unwrap();
    }
}
