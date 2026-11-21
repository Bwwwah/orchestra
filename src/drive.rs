//! Developer aid: with `ORCHESTRA_DRIVE=<folder>` the window takes clicks, keys and text
//! from a file and answers with a picture of itself, so an AI can use the app the way a
//! person would. The events go straight into egui's input: no Accessibility or Screen
//! Recording permission is needed, and nothing outside the window can be touched.
//!
//! Write the commands, one per line, to `<folder>/in` (write to another name, then rename,
//! so the app never reads half a file). The app takes the file, plays the commands in
//! order and, once the last picture is saved, writes `<folder>/out`:
//! `ok ppp=<pixels per point> size=<w>x<h>` then one line per refused command.
//!
//! Commands (positions in points: a picture's pixels divided by `ppp`):
//! - `click X Y`, `dclick X Y` (double), `rclick X Y` (secondary), `move X Y`
//! - `drag X1 Y1 X2 Y2`
//! - `scroll DX DY` (at the pointer; DY < 0 shows what is further down)
//! - `type some text` (the rest of the line, as typed)
//! - `key Enter`, `key cmd+K`, `key shift+Tab` (modifiers: cmd, ctrl, alt, shift)
//! - `wait MS`
//! - `shot [file.png]` (default `<folder>/shot.png`); one is added at the end when the
//!   commands don't end with one.

use eframe::egui;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// What happens on one frame.
#[derive(Debug, Clone, PartialEq)]
enum Step {
    Events(Vec<egui::Event>),
    Wait(Duration),
    Shot(PathBuf),
}

pub struct Drive {
    dir: PathBuf,
    steps: VecDeque<Step>,
    pointer: egui::Pos2,
    /// Until when a `wait` holds the next steps.
    until: Option<Instant>,
    /// A picture asked for and not yet saved: the next steps wait for it.
    shooting: Option<Arc<AtomicBool>>,
    /// Commands refused while reading the file, reported in `out`.
    refused: Vec<String>,
    /// A batch is being played (its `out` is written when it ends).
    busy: bool,
    last_poll: Instant,
    ppp: f32,
    size: egui::Vec2,
}

impl Drive {
    pub fn from_env() -> Option<Drive> {
        let dir = std::env::var("ORCHESTRA_DRIVE")
            .ok()
            .filter(|d| !d.trim().is_empty())?;
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Drive::new(dir))
    }

    fn new(dir: PathBuf) -> Drive {
        Drive {
            dir,
            steps: VecDeque::new(),
            pointer: egui::pos2(0.0, 0.0),
            until: None,
            shooting: None,
            refused: vec![],
            busy: false,
            last_poll: Instant::now() - Duration::from_secs(1),
            ppp: 1.0,
            size: egui::Vec2::ZERO,
        }
    }

    /// Called before each frame: hands this frame's events to egui.
    pub fn feed(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.ppp = ctx.pixels_per_point();
        if let Some(r) = raw.screen_rect {
            self.size = r.size();
        }
        if self.busy {
            // The window acts as if it had the focus, even behind other windows.
            raw.focused = true;
        }
        if let Some(t) = self.until {
            if Instant::now() < t {
                return;
            }
            self.until = None;
        }
        if let Some(done) = &self.shooting {
            if !done.load(Ordering::SeqCst) {
                return;
            }
            self.shooting = None;
        }
        match self.steps.pop_front() {
            Some(Step::Events(ev)) => {
                for e in &ev {
                    if let egui::Event::PointerMoved(p) = e {
                        self.pointer = *p;
                    }
                }
                raw.events.extend(ev);
            }
            Some(Step::Wait(d)) => self.until = Some(Instant::now() + d),
            Some(Step::Shot(path)) => {
                let done = Arc::new(AtomicBool::new(false));
                self.shooting = Some(done.clone());
                let repaint = ctx.clone();
                ctx.request_screenshot(move |img| {
                    if let Err(e) = crate::shot::write_png(
                        &path.to_string_lossy(),
                        img.size[0],
                        img.size[1],
                        &img.pixels,
                    ) {
                        eprintln!("orchestra: picture not saved: {e}");
                    }
                    done.store(true, Ordering::SeqCst);
                    repaint.request_repaint();
                });
            }
            None => {
                if self.busy {
                    self.busy = false;
                    self.answer();
                }
            }
        }
    }

    /// Called after each frame: looks for new commands and keeps frames coming.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if !self.busy && self.last_poll.elapsed() > Duration::from_millis(150) {
            self.last_poll = Instant::now();
            self.take_file();
        }
        let soon = if self.busy { 16 } else { 150 };
        ctx.request_repaint_after(Duration::from_millis(soon));
    }

    fn take_file(&mut self) {
        let inbox = self.dir.join("in");
        let Ok(text) = std::fs::read_to_string(&inbox) else {
            return;
        };
        let _ = std::fs::remove_file(&inbox);
        let _ = std::fs::remove_file(self.dir.join("out"));
        let default_shot = self.dir.join("shot.png");
        let (steps, refused) = parse(&text, self.pointer, &default_shot);
        self.steps = steps.into();
        self.refused = refused;
        self.busy = true;
    }

    fn answer(&mut self) {
        let mut out = format!(
            "ok ppp={} size={}x{}\n",
            self.ppp,
            self.size.x.round(),
            self.size.y.round()
        );
        for r in self.refused.drain(..) {
            out.push_str(&format!("refusé : {r}\n"));
        }
        let tmp = self.dir.join("out.tmp");
        if std::fs::write(&tmp, out).is_ok() {
            let _ = std::fs::rename(tmp, self.dir.join("out"));
        }
    }
}

fn button(
    pos: egui::Pos2,
    b: egui::PointerButton,
    pressed: bool,
    m: egui::Modifiers,
) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: b,
        pressed,
        modifiers: m,
    }
}

fn click_steps(pos: egui::Pos2, b: egui::PointerButton, times: usize) -> Vec<Step> {
    let none = egui::Modifiers::NONE;
    let mut s = vec![Step::Events(vec![egui::Event::PointerMoved(pos)])];
    for _ in 0..times {
        s.push(Step::Events(vec![button(pos, b, true, none)]));
        s.push(Step::Events(vec![button(pos, b, false, none)]));
    }
    // A frame with no event, so the click's effect is drawn before what comes next.
    s.push(Step::Events(vec![]));
    s
}

fn key_steps(spec: &str) -> Option<Vec<Step>> {
    let mut m = egui::Modifiers::NONE;
    let mut parts: Vec<&str> = spec.split('+').map(str::trim).collect();
    let name = parts.pop()?;
    for p in parts {
        match p.to_lowercase().as_str() {
            "cmd" | "command" => {
                m.mac_cmd = cfg!(target_os = "macos");
                m.ctrl |= !cfg!(target_os = "macos");
                m.command = true;
            }
            "ctrl" => m.ctrl = true,
            "alt" | "option" => m.alt = true,
            "shift" => m.shift = true,
            _ => return None,
        }
    }
    let key = egui::Key::from_name(name)
        .or_else(|| egui::Key::from_name(&name.to_uppercase()))
        .or_else(|| {
            let mut c = name.chars();
            let first = c.next()?.to_uppercase().collect::<String>();
            egui::Key::from_name(&(first + &c.as_str().to_lowercase()))
        })?;
    let ev = |pressed| egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: m,
    };
    let mut press = vec![ev(true)];
    // Shortcuts that edit text come as their own events, as the window would send them.
    if m.command && !m.shift && !m.alt {
        match key {
            egui::Key::C => press.push(egui::Event::Copy),
            egui::Key::X => press.push(egui::Event::Cut),
            _ => {}
        }
    }
    Some(vec![
        Step::Events(press),
        Step::Events(vec![ev(false)]),
        Step::Events(vec![]),
    ])
}

fn nums(args: &str) -> Option<Vec<f32>> {
    args.split_whitespace().map(|a| a.parse().ok()).collect()
}

/// The steps of a batch of commands, and the lines refused.
fn parse(
    text: &str,
    mut pointer: egui::Pos2,
    default_shot: &std::path::Path,
) -> (Vec<Step>, Vec<String>) {
    let mut steps = vec![];
    let mut refused = vec![];
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (cmd, args) = line
            .trim_start()
            .split_once(' ')
            .unwrap_or((line.trim(), ""));
        let n = nums(args);
        let got: Option<Vec<Step>> = match (cmd, n.as_deref()) {
            ("click", Some([x, y])) => Some(click_steps(
                egui::pos2(*x, *y),
                egui::PointerButton::Primary,
                1,
            )),
            ("dclick", Some([x, y])) => Some(click_steps(
                egui::pos2(*x, *y),
                egui::PointerButton::Primary,
                2,
            )),
            ("rclick", Some([x, y])) => Some(click_steps(
                egui::pos2(*x, *y),
                egui::PointerButton::Secondary,
                1,
            )),
            ("move", Some([x, y])) => Some(vec![
                Step::Events(vec![egui::Event::PointerMoved(egui::pos2(*x, *y))]),
                Step::Events(vec![]),
            ]),
            ("drag", Some([x1, y1, x2, y2])) => {
                let (a, b) = (egui::pos2(*x1, *y1), egui::pos2(*x2, *y2));
                let none = egui::Modifiers::NONE;
                let p = egui::PointerButton::Primary;
                let mut s = vec![
                    Step::Events(vec![egui::Event::PointerMoved(a)]),
                    Step::Events(vec![button(a, p, true, none)]),
                ];
                for i in 1..=8 {
                    s.push(Step::Events(vec![egui::Event::PointerMoved(
                        a.lerp(b, i as f32 / 8.0),
                    )]));
                }
                s.push(Step::Events(vec![button(b, p, false, none)]));
                s.push(Step::Events(vec![]));
                Some(s)
            }
            ("scroll", Some([dx, dy])) => Some(vec![
                Step::Events(vec![egui::Event::PointerMoved(pointer)]),
                Step::Events(vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(*dx, *dy),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: egui::Modifiers::NONE,
                }]),
                Step::Events(vec![]),
            ]),
            ("type", _) if !args.is_empty() => Some(vec![
                Step::Events(vec![egui::Event::Text(args.to_string())]),
                Step::Events(vec![]),
            ]),
            ("key", _) => key_steps(args.trim()),
            ("wait", Some([ms])) if *ms >= 0.0 => {
                Some(vec![Step::Wait(Duration::from_millis(*ms as u64))])
            }
            ("shot", _) => {
                let a = args.trim();
                let path = if a.is_empty() {
                    default_shot.to_path_buf()
                } else {
                    PathBuf::from(a)
                };
                Some(vec![Step::Events(vec![]), Step::Shot(path)])
            }
            _ => None,
        };
        match got {
            Some(s) => {
                for st in &s {
                    if let Step::Events(ev) = st {
                        for e in ev {
                            if let egui::Event::PointerMoved(p) = e {
                                pointer = *p;
                            }
                        }
                    }
                }
                steps.extend(s);
            }
            None => refused.push(line.to_string()),
        }
    }
    if !matches!(steps.last(), Some(Step::Shot(_))) {
        // Let what the last command started settle a little before the picture.
        steps.push(Step::Wait(Duration::from_millis(300)));
        steps.push(Step::Events(vec![]));
        steps.push(Step::Shot(default_shot.to_path_buf()));
    }
    (steps, refused)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(steps: &[Step]) -> Vec<egui::Event> {
        steps
            .iter()
            .filter_map(|s| match s {
                Step::Events(e) => Some(e.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn a_batch_becomes_events_and_ends_with_a_picture() {
        let shot = std::path::Path::new("/x/shot.png");
        let (steps, refused) = parse(
            "click 10 20\n# note\n\ntype héllo à tous\nkey cmd+K\nscroll 0 -120\nvoler 1 2\nclick 1\n",
            egui::Pos2::ZERO,
            shot,
        );
        assert_eq!(refused, vec!["voler 1 2", "click 1"]);
        let ev = events(&steps);
        assert!(ev.contains(&egui::Event::PointerMoved(egui::pos2(10.0, 20.0))));
        assert!(ev.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: true, pos, .. } if *pos == egui::pos2(10.0, 20.0))));
        assert!(ev.contains(&egui::Event::Text("héllo à tous".into())));
        assert!(ev.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::K, pressed: true, modifiers, .. } if modifiers.command)));
        // The wheel turns where the last click left the pointer.
        let wheel_at = steps.iter().position(|s| matches!(s, Step::Events(e) if e.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })))).unwrap();
        assert_eq!(
            steps[wheel_at - 1],
            Step::Events(vec![egui::Event::PointerMoved(egui::pos2(10.0, 20.0))])
        );
        assert_eq!(steps.last(), Some(&Step::Shot(shot.to_path_buf())));
    }

    #[test]
    fn a_batch_ending_with_a_picture_gets_no_other() {
        let (steps, _) = parse(
            "shot /y/a.png",
            egui::Pos2::ZERO,
            std::path::Path::new("/x/s.png"),
        );
        let shots: Vec<_> = steps
            .iter()
            .filter(|s| matches!(s, Step::Shot(_)))
            .collect();
        assert_eq!(shots, vec![&Step::Shot(PathBuf::from("/y/a.png"))]);
    }

    #[test]
    fn unknown_keys_and_modifiers_are_refused() {
        assert!(key_steps("Enter").is_some());
        assert!(key_steps("shift+Tab").is_some());
        assert!(key_steps("escape").is_some());
        assert!(key_steps("hyper+K").is_none());
        assert!(key_steps("NoSuchKey").is_none());
    }

    #[test]
    fn the_events_reach_the_window_one_frame_at_a_time() {
        let dir = std::env::temp_dir().join(format!("orchestra-drive-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("in"), "click 5 6\nwait 0\n").unwrap();
        let mut d = Drive::new(dir.clone());
        let ctx = egui::Context::default();
        d.tick(&ctx);
        assert!(d.busy && !dir.join("in").exists());
        let mut seen = vec![];
        for _ in 0..10 {
            let mut raw = egui::RawInput::default();
            d.feed(&ctx, &mut raw);
            assert!(raw.focused);
            seen.extend(raw.events);
            if matches!(d.steps.front(), Some(Step::Shot(_))) {
                break;
            }
        }
        assert_eq!(seen[0], egui::Event::PointerMoved(egui::pos2(5.0, 6.0)));
        assert!(matches!(
            seen[1],
            egui::Event::PointerButton { pressed: true, .. }
        ));
        assert!(matches!(
            seen[2],
            egui::Event::PointerButton { pressed: false, .. }
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
}
