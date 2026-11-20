//! Pila's default wallpaper, which swaybg draws (`tool_configs.rs` hands it
//! the file): `background`, plain, with a small mark in the bottom-right
//! corner, three outlined rectangles piled like the windows of Pila's icon,
//! in greys a third and two thirds of the way to `hover` and `hover` in
//! front, with its top edge in the accent (`snap-preview`). Square, flat and
//! in the user's `[colors]`: drawn again when they change.
//!
//! It's a PNG, which every gdk-pixbuf (swaybg's image loader) reads: built in
//! up to 2.42, through glycin's loaders since, where SVG would need a loader
//! package older ones may lack. 4K, so that swaybg only ever shrinks it. swaybg shows it
//! whole (`-m fit`) with `background` around it where the monitor's shape
//! isn't 16:9, so the mark is never cropped.

use crate::config::Colors;

/// The image's size: 4K, 16:9.
pub const SIZE: (u32, u32) = (3840, 2160);
/// The design is laid out on a 1920x1080 grid, drawn at this many pixels a
/// unit.
const UNIT: u32 = SIZE.0 / 1920;
/// The three rectangles, back to front, as (x, y, width, height) on the grid
/// (the icon's, shifted by 25 and 28), and their outline's width.
const RECTS: [(u32, u32, u32, u32); 3] = [
    (1750, 900, 100, 80),
    (1725, 928, 100, 80),
    (1700, 956, 100, 80),
];
const STROKE: u32 = 2;
/// The accent's height on the front rectangle's top edge.
const ACCENT: u32 = 4;

/// A premultiplied color as opaque 8-bit RGB (the accent, `snap-preview`, is
/// translucent in Pila; drawn opaque, as the tools draw it).
pub(crate) fn rgb8([r, g, b, a]: [f32; 4]) -> [u8; 3] {
    let channel = |c: f32| {
        let c = if a > 0.0 { c / a } else { c };
        (c.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    [channel(r), channel(g), channel(b)]
}

/// `from` moved `step` of the way to `to`.
fn mix(from: [u8; 3], to: [u8; 3], step: f32) -> [u8; 3] {
    let channel =
        |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * step).round() as u8;
    [
        channel(from[0], to[0]),
        channel(from[1], to[1]),
        channel(from[2], to[2]),
    ]
}

/// The wallpaper's colors: the background, each rectangle's outline (back to
/// front) and the accent.
fn palette(colors: &Colors) -> ([u8; 3], [[u8; 3]; 3], [u8; 3]) {
    let background = rgb8(colors.background);
    let [r, g, b] = colors.hover;
    let front = rgb8([r, g, b, 1.0]);
    let outlines = [
        mix(background, front, 1.0 / 3.0),
        mix(background, front, 2.0 / 3.0),
        front,
    ];
    (background, outlines, rgb8(colors.snap_preview))
}

/// The wallpaper in `colors`, as RGB rows of `SIZE`.
pub fn pixels(colors: &Colors) -> Vec<u8> {
    let (width, height) = SIZE;
    let (background, outlines, accent) = palette(colors);
    let mut pixels = background.repeat((width * height) as usize);
    let mut fill = |(x, y, w, h): (u32, u32, u32, u32), color: [u8; 3]| {
        let (x, y, w, h) = (x * UNIT, y * UNIT, w * UNIT, h * UNIT);
        for row in y..(y + h).min(height) {
            let start = (row * width + x) as usize * 3;
            let end = (row * width + (x + w).min(width)) as usize * 3;
            for pixel in pixels[start..end].as_chunks_mut::<3>().0 {
                *pixel = color;
            }
        }
    };
    for (&(x, y, w, h), &color) in RECTS.iter().zip(&outlines) {
        // Its four edges, inside its bounds.
        fill((x, y, w, STROKE), color);
        fill((x, y + h - STROKE, w, STROKE), color);
        fill((x, y, STROKE, h), color);
        fill((x + w - STROKE, y, STROKE, h), color);
    }
    let (x, y, w, _) = RECTS[2];
    fill((x, y, w, ACCENT), accent);
    pixels
}

/// The wallpaper in `colors`, encoded as a PNG. Mostly one color: each row is
/// stored as its difference to the one above (`Up`), which compresses to
/// almost nothing, quickly.
pub fn png(colors: &Colors) -> Result<Vec<u8>, png::EncodingError> {
    let (width, height) = SIZE;
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_filter(png::Filter::Up);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&pixels(colors))?;
    writer.finish()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(toml: &str) -> Colors {
        crate::config::Config::from_toml(toml, &|_| None)
            .unwrap()
            .colors
    }

    fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
        let i = (y * SIZE.0 + x) as usize * 3;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    }

    /// The defaults: the background, plain but for the mark in the
    /// bottom-right corner: the outlines a third, two thirds and all the way
    /// to `hover`, the accent on the front one's top edge.
    #[test]
    fn the_default_wallpaper_is_in_the_default_colors() {
        let pixels = pixels(&colors(""));
        let unit = |v: u32| v * UNIT;
        let background = [0x1f, 0x21, 0x26];
        assert_eq!(at(&pixels, 0, 0), background);
        assert_eq!(at(&pixels, unit(960), unit(540)), background);
        assert_eq!(at(&pixels, SIZE.0 - 1, SIZE.1 - 1), background);
        // Inside the front rectangle, below the middle one: background.
        assert_eq!(at(&pixels, unit(1750), unit(1020)), background);
        // The left edges, back to front.
        assert_eq!(at(&pixels, unit(1750), unit(950)), [0x2a, 0x2c, 0x32]);
        assert_eq!(at(&pixels, unit(1725), unit(940)), [0x34, 0x37, 0x3d]);
        assert_eq!(at(&pixels, unit(1700), unit(1000)), [0x3f, 0x42, 0x49]);
        // The accent, opaque.
        assert_eq!(at(&pixels, unit(1710), unit(957)), [0x44, 0x77, 0xcc]);
        // Just under it, the front one's inside.
        assert_eq!(at(&pixels, unit(1710), unit(956 + ACCENT) + 1), background);
    }

    /// The mark stays inside the picture, with room around it.
    #[test]
    fn the_mark_is_in_the_corner_with_a_margin() {
        let right = RECTS.iter().map(|(x, _, w, _)| x + w).max().unwrap();
        let bottom = RECTS.iter().map(|(_, y, _, h)| y + h).max().unwrap();
        assert!(
            1920 - right >= 40 && 1080 - bottom >= 40,
            "{right}, {bottom}"
        );
        assert!(RECTS.iter().all(|(x, y, ..)| *x > 1600 && *y > 850));
    }

    /// The user's `[colors]` draw it.
    #[test]
    fn the_wallpaper_follows_the_users_colors() {
        let pixels = pixels(&colors(
            "[colors]\nbackground = \"#000000\"\nhover = \"#ffffff\"\nsnap-preview = \"#ff000080\"\n",
        ));
        let unit = |v: u32| v * UNIT;
        assert_eq!(at(&pixels, 0, 0), [0, 0, 0]);
        assert_eq!(at(&pixels, unit(1750), unit(950)), [0x55, 0x55, 0x55]);
        assert_eq!(at(&pixels, unit(1700), unit(1000)), [0xff, 0xff, 0xff]);
        assert_eq!(at(&pixels, unit(1710), unit(957)), [0xff, 0, 0]);
    }

    /// A PNG of `SIZE` that decodes to the same pixels, and small.
    #[test]
    fn the_png_decodes_to_the_wallpaper() {
        let colors = colors("");
        let file = png(&colors).unwrap();
        assert!(file.len() < 200_000, "{} bytes", file.len());
        let mut reader = png::Decoder::new(std::io::Cursor::new(&file))
            .read_info()
            .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!((info.width, info.height), SIZE);
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!(decoded, pixels(&colors));
    }
}
