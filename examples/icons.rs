//! Makes the app's icons from the logo: `cargo run --example icons`.
//!
//! Source: assets/logo-source.png, a square image (any size, ≥ 1024 px is best).
//! Writes packaging/icon-1024.png, the master that packaging/build-macos.sh turns into
//! AppIcon.icns: the macOS shape, a rounded square of 824 px with a transparent margin of
//! 100 px all around (Apple's icon grid), so it sits in the Dock like the other apps.

use image::{Rgba, RgbaImage, imageops::FilterType};

const CANVAS: u32 = 1024;
const SQUARE: u32 = 824;
const RADIUS: f32 = 185.0;

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

/// The logo cut into the macOS shape on a transparent canvas.
pub fn macos_master(logo: &RgbaImage) -> RgbaImage {
    let side = logo.width().min(logo.height());
    let crop = image::imageops::crop_imm(
        logo,
        (logo.width() - side) / 2,
        (logo.height() - side) / 2,
        side,
        side,
    )
    .to_image();
    let square = image::imageops::resize(&crop, SQUARE, SQUARE, FilterType::Lanczos3);
    let mut out = RgbaImage::from_pixel(CANVAS, CANVAS, Rgba([0, 0, 0, 0]));
    let off = (CANVAS - SQUARE) / 2;
    for (x, y, p) in square.enumerate_pixels() {
        let a = coverage(x, y, SQUARE as f32, RADIUS);
        if a > 0.0 {
            let mut q = *p;
            q[3] = (q[3] as f32 * a).round() as u8;
            out.put_pixel(x + off, y + off, q);
        }
    }
    out
}

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = root.join("assets/logo-source.png");
    let logo = image::open(&src)
        .unwrap_or_else(|e| panic!("{}: {e}", src.display()))
        .to_rgba8();
    let master = macos_master(&logo);
    let out = root.join("packaging/icon-1024.png");
    master.save(&out).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    println!("{} ({CANVAS}×{CANVAS})", out.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_master_has_the_macos_shape() {
        let logo = RgbaImage::from_pixel(300, 200, Rgba([120, 60, 220, 255]));
        let m = macos_master(&logo);
        assert_eq!(m.dimensions(), (CANVAS, CANVAS));
        // Transparent margin and corners, opaque middle and edge middles.
        assert_eq!(m.get_pixel(50, 50)[3], 0);
        assert_eq!(m.get_pixel(101, 101)[3], 0, "rounded corner");
        assert_eq!(m.get_pixel(512, 512)[3], 255);
        assert_eq!(m.get_pixel(101, 512)[3], 255);
        assert_eq!(m.get_pixel(1000, 512)[3], 0);
    }
}
