//! The look pila-launcher and pila-bar share: Pila's own colors (the
//! defaults of `[colors]` in Pila's config, which its title bars use) as an
//! iced theme, and the few colors and fonts the theme doesn't name.
//!
//! pila-bar includes this file (`#[path]`): one source for both.

#![allow(dead_code)] // Each client uses part of it.

use iced::{color, theme::Palette, Color, Font, Theme};

/// The theme's background (Pila's `bar`, the focused title bar).
pub const SURFACE: Color = color!(0x2f3136);
/// Behind the bar and the launcher (Pila's `bar-inactive`).
pub const SURFACE_DIM: Color = color!(0x26272b);
/// Lines and edges (Pila's `background`, behind everything).
pub const EDGE: Color = color!(0x1f2126);
/// Under the pointer (Pila's `hover`), and the launcher's border.
pub const HOVER: Color = color!(0x3f4249);
pub const TEXT: Color = color!(0xdfe1e5);
/// What reads second: subtitles, placeholders, idle workspaces (Pila's
/// `icon-inactive`).
pub const TEXT_DIM: Color = color!(0x80838a);
pub const TEXT_BRIGHT: Color = color!(0xf2f3f5);
/// The active workspace and the selected app (the blue of Pila's snap
/// preview).
pub const ACCENT: Color = color!(0x4477cc);
/// Asking for attention (Pila's `bar-urgent`).
pub const URGENT: Color = color!(0x8a5712);

/// The one font both use, at one size, regular weight everywhere (the
/// active and selected stand out by color): the font of Pila's title bars
/// (`text::FONT_SIZE` in Pila, which a test keeps equal to `TEXT_SIZE`).
pub const FONT: Font = Font::DEFAULT;
pub const TEXT_SIZE: f32 = 14.0;

pub fn theme() -> Theme {
    Theme::custom(
        "Pila",
        Palette {
            background: SURFACE,
            text: TEXT,
            primary: ACCENT,
            success: color!(0x3f9c5a),
            warning: URGENT,
            danger: color!(0xc42b1c),
        },
    )
}
