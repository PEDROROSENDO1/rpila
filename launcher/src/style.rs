//! The look pila-launcher and pila-bar share: Pila's own colors (the
//! defaults of `[colors]` in Pila's config, which its title bars use) as an
//! iced theme, and the few colors and fonts the theme doesn't name.
//!
//! pila-bar includes this file (`#[path]`): one source for both.

#![allow(dead_code)] // Each client uses part of it.

use iced::{color, font::Weight, theme::Palette, Color, Font, Theme};

/// Behind the launcher's list (Pila's `bar`, the focused title bar).
pub const SURFACE: Color = color!(0x2f3136);
/// The bar's background (Pila's `bar-inactive`).
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
/// The active and selected (the blue of Pila's snap preview).
pub const ACCENT: Color = color!(0x4477cc);
/// The selected row: the accent, faded into `SURFACE`.
pub const ACCENT_TINT: Color = color!(0x354a70);
/// Asking for attention (Pila's `bar-urgent`).
pub const URGENT: Color = color!(0x8a5712);

/// Bold, not semibold: the common sans fonts (DejaVu Sans among them) have
/// no semibold, and the font lookup then falls back to another family.
pub const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};

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
