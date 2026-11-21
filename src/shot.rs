//! Developer aid: with `ORCHESTRA_SHOT=/path/out.png` the window saves a picture of
//! itself after `ORCHESTRA_SHOT_FRAME` frames (default 8) and closes. Works without the
//! system's screen-recording permission because the app reads its own pixels.

use eframe::egui;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Shot {
    path: String,
    at_frame: u64,
    frame: u64,
    requested: bool,
    done: Arc<AtomicBool>,
    /// ORCHESTRA_SHOT_SECS: wait this long too.
    after: Option<(std::time::Instant, f32)>,
}

impl Shot {
    pub fn from_env() -> Option<Shot> {
        let path = std::env::var("ORCHESTRA_SHOT")
            .ok()
            .filter(|p| p.ends_with(".png"))?;
        let at_frame = std::env::var("ORCHESTRA_SHOT_FRAME")
            .ok()
            .and_then(|f| f.parse().ok())
            .unwrap_or(8);
        let after = std::env::var("ORCHESTRA_SHOT_SECS")
            .ok()
            .and_then(|f| f.parse().ok())
            .map(|s| (std::time::Instant::now(), s));
        Some(Shot {
            path,
            at_frame,
            frame: 0,
            requested: false,
            done: Arc::new(AtomicBool::new(false)),
            after,
        })
    }

    /// Call once per frame.
    pub fn tick(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        // ORCHESTRA_SHOT_RESIZE=WxH: the window changes size on frame 30 (to test resizing).
        if self.frame == 30
            && let Some((w, h)) = std::env::var("ORCHESTRA_SHOT_RESIZE").ok().and_then(|v| {
                v.split_once('x')
                    .and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?)))
            })
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
        }
        if self.done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.requested
            && self.frame >= self.at_frame
            && self
                .after
                .is_none_or(|(t, s)| t.elapsed().as_secs_f32() >= s)
        {
            self.requested = true;
            let path = self.path.clone();
            let done = self.done.clone();
            let repaint = ctx.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                egui::ScreenshotCallback::new(move |img| {
                    if let Err(e) = write_png(&path, img.size[0], img.size[1], &img.pixels) {
                        eprintln!("orchestra: picture not saved: {e}");
                    }
                    done.store(true, Ordering::SeqCst);
                    repaint.request_repaint();
                }),
            ));
        }
        ctx.request_repaint();
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// A plain RGBA PNG with uncompressed deflate blocks (no dependency needed).
fn write_png(path: &str, w: usize, h: usize, pixels: &[egui::Color32]) -> std::io::Result<()> {
    let mut raw = Vec::with_capacity(h * (w * 4 + 1));
    for y in 0..h {
        raw.push(0u8);
        for p in &pixels[y * w..(y + 1) * w] {
            let [r, g, b, a] = p.to_srgba_unmultiplied();
            raw.extend_from_slice(&[r, g, b, a]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(c) = chunks.next() {
        z.push(if chunks.peek().is_none() { 1 } else { 0 });
        let len = c.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(c);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = vec![];
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    std::fs::File::create(path)?.write_all(&out)
}
