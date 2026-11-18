//! The look pila-launcher and pila-bar share: Pila's own colors as an iced
//! theme, and the few colors and fonts the theme doesn't name. On Pila they
//! follow the user's `[colors]` (`pila_ipc::styles`); elsewhere, or until
//! Pila says, they're the defaults of `[colors]` (`Style::default`).
//!
//! pila-bar includes this file (`#[path]`): one source for both.

#![allow(dead_code)] // Each client uses part of it.

use iced::{color, theme::Palette, Color, Font, Theme};
use serde_json::Value;

/// The colors both clients draw with, each named after the `[colors]`
/// setting it comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The theme's background (`bar`, the focused title bar).
    pub surface: Color,
    /// Behind the bar and the launcher (`bar-inactive`).
    pub surface_dim: Color,
    /// Lines and edges (`background`, behind everything).
    pub edge: Color,
    /// Under the pointer (`hover`), and the launcher's border.
    pub hover: Color,
    /// Text (`icon`, a focused title).
    pub text: Color,
    /// What reads second: subtitles, placeholders, idle workspaces
    /// (`icon-inactive`).
    pub text_dim: Color,
    /// Text that stands out (`hover-icon`).
    pub text_bright: Color,
    /// The active workspace and the selected app (`snap-preview`, opaque).
    pub accent: Color,
    /// Asking for attention (`bar-urgent`).
    pub urgent: Color,
    /// Danger (`close-hover`).
    pub danger: Color,
}

impl Default for Style {
    /// The defaults of `[colors]` in Pila's `config.example.toml`.
    fn default() -> Self {
        Style {
            surface: color!(0x2f3136),
            surface_dim: color!(0x26272b),
            edge: color!(0x1f2126),
            hover: color!(0x3f4249),
            text: color!(0xdfe1e5),
            text_dim: color!(0x80838a),
            text_bright: color!(0xf2f3f5),
            accent: color!(0x4477cc),
            urgent: color!(0x8a5712),
            danger: color!(0xc42b1c),
        }
    }
}

impl Style {
    /// The colors of a `colors` answer or event of Pila's IPC (`docs/IPC.md`):
    /// each one it gives, opaque (the clients' surfaces have no alpha); the
    /// default for any it leaves out or that doesn't read.
    pub fn from_pila(colors: &Value) -> Style {
        let mut style = Style::default();
        let fields = [
            ("bar", &mut style.surface),
            ("bar_inactive", &mut style.surface_dim),
            ("background", &mut style.edge),
            ("hover", &mut style.hover),
            ("icon", &mut style.text),
            ("icon_inactive", &mut style.text_dim),
            ("hover_icon", &mut style.text_bright),
            ("snap_preview", &mut style.accent),
            ("bar_urgent", &mut style.urgent),
            ("close_hover", &mut style.danger),
        ];
        for (name, field) in fields {
            if let Some(color) = colors[name].as_str().and_then(parse_hex) {
                *field = color;
            }
        }
        style
    }

    pub fn theme(&self) -> Theme {
        Theme::custom(
            "Pila",
            Palette {
                background: self.surface,
                text: self.text,
                primary: self.accent,
                success: color!(0x3f9c5a),
                warning: self.urgent,
                danger: self.danger,
            },
        )
    }
}

/// `#rrggbb` or `#rrggbbaa`, as opaque: the alpha is left out.
fn parse_hex(text: &str) -> Option<Color> {
    let digits = text.strip_prefix('#')?;
    if !digits.is_ascii() || !(digits.len() == 6 || digits.len() == 8) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some(Color::from_rgb8(channel(0)?, channel(2)?, channel(4)?))
}

/// The one font both use, at one size: the font of Pila's title bars
/// (`text::FONT_SIZE` in Pila, which a test keeps equal to `TEXT_SIZE`).
/// Regular weight, but for what pila-bar puts first (`FONT_STRONG`).
pub const FONT: Font = Font::DEFAULT;
pub const TEXT_SIZE: f32 = 14.0;
/// pila-bar's active workspace and clock: bold, which nearly every family
/// has (iced's text takes a weight the family has, else another family's:
/// semibold would mean another font where the user's has none).
pub const FONT_STRONG: Font = Font {
    weight: iced::font::Weight::Bold,
    ..FONT
};

#[cfg(test)]
mod style_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn colors_come_from_pila() {
        let style = Style::from_pila(&json!({
            "bar": "#102030",
            "snap_preview": "#4477cc4d",
            "hover": "not a color",
            "icon": "#12345",
        }));
        assert_eq!(style.surface, Color::from_rgb8(0x10, 0x20, 0x30));
        // Opaque: the alpha is left out.
        assert_eq!(style.accent, Color::from_rgb8(0x44, 0x77, 0xcc));
        // Those it doesn't give or that don't read: the defaults.
        let default = Style::default();
        assert_eq!(style.hover, default.hover);
        assert_eq!(style.text, default.text);
        assert_eq!(style.edge, default.edge);
        assert_eq!(Style::from_pila(&json!(null)), default);
    }

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex("#ffffff"), Some(Color::WHITE));
        assert_eq!(parse_hex("#00000080"), Some(Color::BLACK));
        for wrong in ["ffffff", "#fff", "#gggggg", "#ffffffff00", "#ffé123", ""] {
            assert_eq!(parse_hex(wrong), None, "{wrong:?}");
        }
    }
}
