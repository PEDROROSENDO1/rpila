//! A picture's way out: cropped to the region picked, encoded as PNG, and
//! written where the user keeps pictures (`XDG_PICTURES_DIR`, else
//! `~/Pictures`) under the time it was taken, or to a file named.

use std::path::{Path, PathBuf};

use crate::capture::Shot;

/// A region of a picture, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The part of `shot` in `rect` (kept inside it), as a picture of its own.
pub fn crop(shot: &Shot, rect: Rect) -> Shot {
    let x = rect.x.min(shot.width.saturating_sub(1));
    let y = rect.y.min(shot.height.saturating_sub(1));
    let width = rect.width.clamp(1, shot.width - x);
    let height = rect.height.clamp(1, shot.height - y);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for row in y..y + height {
        let start = ((row * shot.width + x) * 4) as usize;
        rgba.extend_from_slice(&shot.rgba[start..start + (width * 4) as usize]);
    }
    Shot {
        output: shot.output.clone(),
        width,
        height,
        rgba,
    }
}

/// The region a selection of the fractions `(x, y, width, height)` of a
/// monitor's surface (0 to 1, as the surface is shown) is of its picture
/// of `width` x `height` pixels: the surface may be smaller than the
/// picture (a scale above 1), never other than it in shape.
pub fn rect_of(fractions: (f32, f32, f32, f32), width: u32, height: u32) -> Rect {
    let (fx, fy, fw, fh) = fractions;
    let at =
        |fraction: f32, of: u32| ((fraction.clamp(0.0, 1.0) * of as f32).round() as u32).min(of);
    let (x0, y0) = (at(fx, width), at(fy, height));
    let (x1, y1) = (at(fx + fw, width), at(fy + fh, height));
    Rect {
        x: x0,
        y: y0,
        width: (x1 - x0).max(1),
        height: (y1 - y0).max(1),
    }
}

/// `shot` as a PNG file's bytes, without alpha (a monitor has none).
pub fn png(shot: &Shot) -> Result<Vec<u8>, String> {
    let rgb: Vec<u8> = shot
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    let image = image::RgbImage::from_raw(shot.width, shot.height, rgb)
        .ok_or("the picture's size doesn't match its pixels")?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|err| format!("can't encode the PNG: {err}"))?;
    Ok(bytes.into_inner())
}

/// Where pictures go: `XDG_PICTURES_DIR` from `user-dirs.dirs` (in
/// `XDG_CONFIG_HOME`, else `~/.config`), else `~/Pictures`.
pub fn pictures_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    let home = PathBuf::from(home);
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|c| !c.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let named = std::fs::read_to_string(config.join("user-dirs.dirs"))
        .ok()
        .and_then(|text| user_dirs_pictures(&text, &home));
    Some(named.unwrap_or_else(|| home.join("Pictures")))
}

/// `XDG_PICTURES_DIR` in a `user-dirs.dirs` file's text, `$HOME` expanded
/// (the one form the file has, `"$HOME/..."` or an absolute path).
pub fn user_dirs_pictures(text: &str, home: &Path) -> Option<PathBuf> {
    let value = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("XDG_PICTURES_DIR=")
            .map(|v| v.trim().trim_matches('"'))
    })?;
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(value),
    };
    // `"$HOME/"` alone means the home directory: no pictures folder.
    (path.is_absolute() && path != home).then_some(path)
}

/// A new picture's file name, from the local time:
/// `screenshot-20261119-143005.png`.
pub fn file_name(time: &str) -> String {
    format!("screenshot-{time}.png")
}

/// Write `bytes` to `path`, making its directory if need be.
pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("can't make {}: {err}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|err| format!("can't write {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(width: u32, height: u32) -> Shot {
        let rgba = (0..width * height)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 0, 255])
            .collect();
        Shot {
            output: "OUT-1".into(),
            width,
            height,
            rgba,
        }
    }

    #[test]
    fn a_crop_is_the_rows_of_the_region() {
        let full = shot(4, 3);
        let part = crop(
            &full,
            Rect {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            },
        );
        assert_eq!((part.width, part.height), (2, 2));
        let firsts: Vec<u8> = part.rgba.chunks(4).map(|p| p[0]).collect();
        assert_eq!(firsts, [5, 6, 9, 10]);
        // Past the edge: what's inside.
        let edge = crop(
            &full,
            Rect {
                x: 3,
                y: 2,
                width: 10,
                height: 10,
            },
        );
        assert_eq!((edge.width, edge.height), (1, 1));
    }

    #[test]
    fn a_selection_of_the_surface_is_a_region_of_the_picture() {
        // Scale 2: the surface is half the picture, the fractions the same.
        let rect = rect_of((0.25, 0.5, 0.5, 0.25), 3840, 2160);
        assert_eq!(
            rect,
            Rect {
                x: 960,
                y: 1080,
                width: 1920,
                height: 540
            }
        );
        // Dragged past the edge.
        let rect = rect_of((0.9, 0.9, 0.5, 0.5), 100, 100);
        assert_eq!(
            rect,
            Rect {
                x: 90,
                y: 90,
                width: 10,
                height: 10
            }
        );
        // Never empty.
        assert_eq!(rect_of((0.5, 0.5, 0.0, 0.0), 100, 100).width, 1);
    }

    #[test]
    fn a_png_has_the_pictures_size() {
        let bytes = png(&shot(5, 3)).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (5, 3));
        assert_eq!(decoded.color(), image::ColorType::Rgb8);
    }

    #[test]
    fn pictures_go_where_user_dirs_says() {
        let home = Path::new("/home/someone");
        let text =
            "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_PICTURES_DIR=\"$HOME/Imágenes\"\n";
        assert_eq!(
            user_dirs_pictures(text, home),
            Some(PathBuf::from("/home/someone/Imágenes"))
        );
        assert_eq!(
            user_dirs_pictures("XDG_PICTURES_DIR=\"/data/pics\"", home),
            Some(PathBuf::from("/data/pics"))
        );
        assert_eq!(
            user_dirs_pictures("XDG_PICTURES_DIR=\"$HOME/\"", home),
            None
        );
        assert_eq!(
            user_dirs_pictures("XDG_MUSIC_DIR=\"$HOME/Music\"", home),
            None
        );
    }

    #[test]
    fn a_file_is_named_by_its_time() {
        assert_eq!(
            file_name("20261119-143005"),
            "screenshot-20261119-143005.png"
        );
    }
}
