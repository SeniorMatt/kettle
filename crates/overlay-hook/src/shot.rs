//! Screenshots: the renderer reads the game's frame back (before the overlay draws into it) and hands the
//! pixels here; a thread encodes a PNG into the folder Kettle named (`~/Pictures/Kettle/<game>/`) and tells
//! Kettle where it went. Nothing here may slow or stop the game, so all of it happens off the render thread.

use crate::ipc;
use sr_overlay_proto::FromHook;
use std::path::{Path, PathBuf};

/// The folder for a game's screenshots when Kettle did not name one: `$KETTLE_OVERLAY_SHOTDIR` (tests), else
/// `~/Pictures/Kettle/<game>`.
pub fn default_dir(game: &str) -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("KETTLE_OVERLAY_SHOTDIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join("Pictures/Kettle").join(folder_name(game)))
}

/// A game's name as one folder name: no slashes, dots at the start or control characters.
pub fn folder_name(game: &str) -> String {
    let s: String =
        game.chars()
            .map(|c| {
                if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
    let s = s.trim().trim_start_matches('.').trim();
    if s.is_empty() { "Games".into() } else { s.chars().take(80).collect() }
}

/// `<time>.png`: sortable, to the second (a second shot in the same second gets a number).
pub fn file_name(game: &str, stamp: &str, n: u32) -> String {
    let base = folder_name(game);
    if n == 0 { format!("{base}_{stamp}.png") } else { format!("{base}_{stamp}_{n}.png") }
}

/// Flip rows (GL reads from the bottom) and make every pixel opaque.
pub fn finish_gl(rgba: &mut [u8], w: usize, h: usize) {
    for y in 0..h / 2 {
        let (a, b) = rgba.split_at_mut((h - 1 - y) * w * 4);
        a[y * w * 4..(y + 1) * w * 4].swap_with_slice(&mut b[..w * 4]);
    }
    for p in rgba.as_chunks_mut::<4>().0 {
        p[3] = 255;
    }
}

pub fn encode_png(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    if rgba.len() != (w as usize) * (h as usize) * 4 || w == 0 || h == 0 {
        return None;
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut wr = enc.write_header().ok()?;
        wr.write_image_data(rgba).ok()?;
    }
    Some(out)
}

fn stamp() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return t.to_string();
        }
        let mut buf = [0u8; 32];
        let n = libc::strftime(buf.as_mut_ptr().cast(), buf.len(), c"%Y%m%d_%H%M%S".as_ptr(), &tm);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

/// A free file name in `dir`.
fn free_path(dir: &Path, game: &str) -> PathBuf {
    let stamp = stamp();
    (0..1000)
        .map(|n| dir.join(file_name(game, &stamp, n)))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(file_name(game, &stamp, 1000)))
}

/// Save a frame (RGBA, top row first). Returns at once; a thread does the work.
pub fn save(rgba: Vec<u8>, w: u32, h: u32) {
    let (dir, game) = crate::state::shot_target();
    let dir = if dir.is_empty() { default_dir(&game) } else { Some(PathBuf::from(dir)) };
    let Some(dir) = dir else { return };
    let _ = std::thread::Builder::new().name("kettle-shot".into()).spawn(move || {
        let _ = std::panic::catch_unwind(|| {
            let Some(png) = encode_png(&rgba, w, h) else { return };
            if std::fs::create_dir_all(&dir).is_err() {
                return;
            }
            let path = free_path(&dir, &game);
            if std::fs::write(&path, png).is_ok() {
                let p = path.display().to_string();
                ipc::log(&format!("screenshot {p}"));
                ipc::send(&FromHook::Screenshot { path: p });
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(folder_name("Half-Life 2"), "Half-Life 2");
        assert_eq!(folder_name("../../etc"), "_.._etc");
        assert_eq!(folder_name(""), "Games");
        assert_eq!(folder_name("  "), "Games");
        assert_eq!(folder_name("a/b\\c:d"), "a_b_c_d");
        assert!(!folder_name("..hidden").starts_with('.'));
        assert_eq!(folder_name(&"x".repeat(200)).len(), 80);
    }

    #[test]
    fn names_do_not_collide() {
        assert_eq!(file_name("G", "20240101_101010", 0), "G_20240101_101010.png");
        assert_eq!(file_name("G", "20240101_101010", 2), "G_20240101_101010_2.png");
    }

    #[test]
    fn gl_frames_are_flipped_and_opaque() {
        // 1 pixel wide, 3 rows: bottom row first as GL gives it.
        let mut px = vec![1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0];
        finish_gl(&mut px, 1, 3);
        assert_eq!(px, vec![3, 0, 0, 255, 2, 0, 0, 255, 1, 0, 0, 255]);
    }

    #[test]
    fn png_has_the_right_size_and_rejects_bad_buffers() {
        let png = encode_png(&[255; 4 * 6], 3, 2).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert!(encode_png(&[0; 5], 3, 2).is_none());
        assert!(encode_png(&[], 0, 0).is_none());
    }
}
