//! Makes the app's icons from the logo: `cargo run --example icons`.
//!
//! Source: assets/logo-source.png, a square image (any size, ≥ 1024 px is best). Writes:
//! - packaging/icon-1024.png and assets/icon-macos.png: the macOS master, a rounded square of
//!   824 px with a transparent margin of 100 px all around (Apple's icon grid), so it sits in
//!   the Dock like the other apps; packaging/build-macos.sh turns it into the app's .icns;
//! - assets/AppIcon.icns: that master at every macOS size (16 to 1024 px, Retina included);
//! - assets/icon.png (1024), icon-512.png (Linux), icon-256.png (the window's icon): the
//!   rounded square edge to edge, as the other systems show icons without a margin;
//! - assets/icon.ico (Windows): the same, at 16, 24, 32, 48, 64, 128 and 256 px.
//!
//! The .ico and .icns files are written here (PNG images in their container), so no system
//! tool is needed and it works the same on every computer.

use image::{ImageFormat, Rgba, RgbaImage, imageops::FilterType};
use std::path::Path;

const CANVAS: u32 = 1024;
const SQUARE: u32 = 824;
/// Corner radius, as a share of the square's side (185 px at 824 px, as macOS draws it).
const RADIUS: f32 = 185.0 / 824.0;
/// Windows icon sizes.
const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
/// macOS icon types and their size in pixels (`ic11` is 16 pt at 2×, and so on).
const ICNS: [(&[u8; 4], u32); 10] = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic13", 256),
    (b"ic08", 256),
    (b"ic14", 512),
    (b"ic09", 512),
    (b"ic10", 1024),
];

/// How much of the pixel (x, y) (its centre) lies inside a rounded square of side `side` and
/// corner radius `r` starting at (0, 0): 1 inside, 0 outside, in between on the edge.
fn coverage(x: u32, y: u32, side: f32, r: f32) -> f32 {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    // Distance to the nearest corner centre, when in a corner's zone.
    let cx = px.clamp(r, side - r);
    let cy = py.clamp(r, side - r);
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    (r - d + 0.5).clamp(0.0, 1.0)
}

/// The logo (its centred square) at `side` px, cut into a rounded square.
fn rounded(logo: &RgbaImage, side: u32) -> RgbaImage {
    let s = logo.width().min(logo.height());
    let crop = image::imageops::crop_imm(logo, (logo.width() - s) / 2, (logo.height() - s) / 2, s, s)
        .to_image();
    let mut out = image::imageops::resize(&crop, side, side, FilterType::Lanczos3);
    let r = side as f32 * RADIUS;
    for (x, y, p) in out.enumerate_pixels_mut() {
        p[3] = (p[3] as f32 * coverage(x, y, side as f32, r)).round() as u8;
    }
    out
}

/// The logo cut into the macOS shape on a transparent canvas.
pub fn macos_master(logo: &RgbaImage) -> RgbaImage {
    let mut out = RgbaImage::from_pixel(CANVAS, CANVAS, Rgba([0, 0, 0, 0]));
    let off = (CANVAS - SQUARE) / 2;
    image::imageops::overlay(&mut out, &rounded(logo, SQUARE), off.into(), off.into());
    out
}

/// An image at `side` px, scaled down from a larger one.
fn scaled(img: &RgbaImage, side: u32) -> RgbaImage {
    if img.width() == side {
        img.clone()
    } else {
        image::imageops::resize(img, side, side, FilterType::Lanczos3)
    }
}

fn png_bytes(img: &RgbaImage) -> Vec<u8> {
    let mut v = std::io::Cursor::new(Vec::new());
    img.write_to(&mut v, ImageFormat::Png).expect("PNG encoding");
    v.into_inner()
}

/// A Windows .ico holding one PNG per size (Windows Vista and later read PNG entries).
pub fn ico(full: &RgbaImage, sizes: &[u32]) -> Vec<u8> {
    let pngs: Vec<Vec<u8>> = sizes.iter().map(|&s| png_bytes(&scaled(full, s))).collect();
    let mut out = vec![0, 0, 1, 0];
    out.extend((sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, png) in sizes.iter().zip(&pngs) {
        // 256 px is written as 0 (one byte).
        let b = if s >= 256 { 0 } else { s as u8 };
        out.extend([b, b, 0, 0]);
        out.extend(1u16.to_le_bytes()); // colour planes
        out.extend(32u16.to_le_bytes()); // bits per pixel
        out.extend((png.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in pngs {
        out.extend(png);
    }
    out
}

/// A macOS .icns holding one PNG per icon type (see `ICNS`).
pub fn icns(master: &RgbaImage) -> Vec<u8> {
    let mut body = Vec::new();
    for (kind, side) in ICNS {
        let png = png_bytes(&scaled(master, side));
        body.extend(kind);
        body.extend((png.len() as u32 + 8).to_be_bytes());
        body.extend(png);
    }
    let mut out = b"icns".to_vec();
    out.extend((body.len() as u32 + 8).to_be_bytes());
    out.extend(body);
    out
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    std::fs::write(&p, bytes).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    println!("{rel} ({} Kio)", bytes.len() / 1024);
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = root.join("assets/logo-source.png");
    let logo = image::open(&src)
        .unwrap_or_else(|e| panic!("{}: {e}", src.display()))
        .to_rgba8();
    let master = macos_master(&logo);
    let full = rounded(&logo, CANVAS);
    let master_png = png_bytes(&master);
    write(root, "packaging/icon-1024.png", &master_png);
    write(root, "assets/icon-macos.png", &master_png);
    write(root, "assets/AppIcon.icns", &icns(&master));
    write(root, "assets/icon.png", &png_bytes(&full));
    write(root, "assets/icon-512.png", &png_bytes(&scaled(&full, 512)));
    write(root, "assets/icon-256.png", &png_bytes(&scaled(&full, 256)));
    write(root, "assets/icon.ico", &ico(&full, &ICO_SIZES));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logo() -> RgbaImage {
        RgbaImage::from_pixel(300, 200, Rgba([120, 60, 220, 255]))
    }

    #[test]
    fn the_master_has_the_macos_shape() {
        let m = macos_master(&logo());
        assert_eq!(m.dimensions(), (CANVAS, CANVAS));
        // Transparent margin and corners, opaque middle and edge middles.
        assert_eq!(m.get_pixel(50, 50)[3], 0);
        assert_eq!(m.get_pixel(101, 101)[3], 0, "rounded corner");
        assert_eq!(m.get_pixel(512, 512)[3], 255);
        assert_eq!(m.get_pixel(101, 512)[3], 255);
        assert_eq!(m.get_pixel(1000, 512)[3], 0);
    }

    #[test]
    fn the_other_icons_go_edge_to_edge_with_round_corners() {
        let f = rounded(&logo(), 256);
        assert_eq!(f.dimensions(), (256, 256));
        assert_eq!(f.get_pixel(0, 128)[3], 255, "no margin");
        assert_eq!(f.get_pixel(0, 0)[3], 0, "rounded corner");
    }

    #[test]
    fn ico_and_icns_list_their_pngs() {
        let full = rounded(&logo(), 256);
        let i = ico(&full, &[16, 256]);
        assert_eq!(&i[..6], &[0, 0, 1, 0, 2, 0]);
        // First entry: 16 px; second: 256 px written as 0; each points at a PNG.
        assert_eq!((i[6], i[22]), (16, 0));
        for e in [6usize, 22] {
            let off = u32::from_le_bytes(i[e + 12..e + 16].try_into().unwrap()) as usize;
            assert_eq!(&i[off..off + 4], b"\x89PNG");
        }
        let n = icns(&macos_master(&logo()));
        assert_eq!(&n[..4], b"icns");
        assert_eq!(u32::from_be_bytes(n[4..8].try_into().unwrap()) as usize, n.len());
        // Walk the entries: every type of ICNS, each a PNG, the lengths adding up.
        let mut at = 8;
        let mut kinds = vec![];
        while at < n.len() {
            let len = u32::from_be_bytes(n[at + 4..at + 8].try_into().unwrap()) as usize;
            assert_eq!(&n[at + 8..at + 12], b"\x89PNG");
            kinds.push(n[at..at + 4].to_vec());
            at += len;
        }
        assert_eq!(at, n.len());
        assert_eq!(kinds, ICNS.iter().map(|(k, _)| k.to_vec()).collect::<Vec<_>>());
    }
}
