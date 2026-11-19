//! The popup's icon, drawn as text from the icon font the user has
//! installed (`icon_font.rs`, as pila-bar's): Material Design's in a Nerd
//! Font (the volume by thirds, a muted speaker or microphone, the sun by
//! thirds; play, pause, skip, stop for the media keys), else Font Awesome's
//! fewer. Without one, the popup names what it shows in words.

use iced::Font;

use crate::{icon_font, level::Kind, level::Level, media::State};

/// Material Design's, in Nerd Fonts: `md-volume_low`, `_medium`, `_high`,
/// `_off`; `md-microphone`, `_off`; `md-brightness_5`, `_6`, `_7`.
const MATERIAL_VOLUME: [char; 3] = ['\u{f057f}', '\u{f0580}', '\u{f057e}'];
const MATERIAL_VOLUME_OFF: char = '\u{f0581}';
const MATERIAL_MIC: char = '\u{f036c}';
const MATERIAL_MIC_OFF: char = '\u{f036d}';
const MATERIAL_BRIGHTNESS: [char; 3] = ['\u{f00de}', '\u{f00df}', '\u{f00e0}'];
/// Font Awesome's (at the same code points in Nerd Fonts): `volume-off`,
/// `-down`, `-up`; `microphone`, `-slash`; `sun`.
const AWESOME_VOLUME: [char; 3] = ['\u{f026}', '\u{f027}', '\u{f028}'];
const AWESOME_MIC: char = '\u{f130}';
const AWESOME_MIC_OFF: char = '\u{f131}';
const AWESOME_SUN: char = '\u{f185}';
/// The media keys': Material Design's `md-play`, `md-pause`, `md-stop`,
/// `md-skip_next`, `md-skip_previous`, `md-music_off` (no player); Font
/// Awesome's `play`, `pause`, `stop`, `step-forward`, `step-backward`,
/// `music`, in that order.
const MATERIAL_MEDIA: [char; 6] = [
    '\u{f040a}',
    '\u{f03e4}',
    '\u{f04db}',
    '\u{f04ad}',
    '\u{f04ae}',
    '\u{f075b}',
];
const AWESOME_MEDIA: [char; 6] = [
    '\u{f04b}', '\u{f04c}', '\u{f04d}', '\u{f051}', '\u{f048}', '\u{f001}',
];

/// The icon font found, and whether it has Material Design's icons.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconFont {
    pub font: Font,
    pub material: bool,
}

/// The icon for `level`.
pub fn glyph(level: &Level, material: bool) -> char {
    let third = match level.percent {
        0..=33 => 0,
        34..=66 => 1,
        _ => 2,
    };
    match (level.kind, material) {
        (Kind::Volume, true) if level.muted || level.percent == 0 => MATERIAL_VOLUME_OFF,
        (Kind::Volume, true) => MATERIAL_VOLUME[third],
        (Kind::Volume, false) if level.muted || level.percent == 0 => AWESOME_VOLUME[0],
        (Kind::Volume, false) => AWESOME_VOLUME[if third == 2 { 2 } else { 1 }],
        (Kind::Mic, true) if level.muted => MATERIAL_MIC_OFF,
        (Kind::Mic, true) => MATERIAL_MIC,
        (Kind::Mic, false) if level.muted => AWESOME_MIC_OFF,
        (Kind::Mic, false) => AWESOME_MIC,
        (Kind::Brightness, true) => MATERIAL_BRIGHTNESS[third],
        (Kind::Brightness, false) => AWESOME_SUN,
    }
}

/// The icon for what a media key did.
pub fn media_glyph(state: State, material: bool) -> char {
    let icons = if material {
        MATERIAL_MEDIA
    } else {
        AWESOME_MEDIA
    };
    icons[match state {
        State::Playing => 0,
        State::Paused => 1,
        State::Stopped => 2,
        State::Next => 3,
        State::Previous => 4,
        State::NoPlayer => 5,
    }]
}

/// The installed icon font with every icon above: Material Design's if it
/// has them all, else Font Awesome's.
pub fn find() -> Option<IconFont> {
    let (font, material) = icon_font::find(|face| {
        let all = |glyphs: &[char]| glyphs.iter().all(|&c| face.glyph_index(c).is_some());
        let mut material = MATERIAL_VOLUME.to_vec();
        material.extend([MATERIAL_VOLUME_OFF, MATERIAL_MIC, MATERIAL_MIC_OFF]);
        material.extend(MATERIAL_BRIGHTNESS);
        material.extend(MATERIAL_MEDIA);
        let mut awesome = AWESOME_VOLUME.to_vec();
        awesome.extend([AWESOME_MIC, AWESOME_MIC_OFF, AWESOME_SUN]);
        awesome.extend(AWESOME_MEDIA);
        if all(&material) {
            Some(true)
        } else {
            all(&awesome).then_some(false)
        }
    })?;
    Some(IconFont { font, material })
}

/// The icon font, from a thread of its own: one answer, `None` if there's
/// none (or the thread couldn't start).
pub fn found() -> impl iced::futures::Future<Output = Option<IconFont>> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("pila-osd-fonts".into())
        .spawn(move || {
            let _ = sender.send(find());
        });
    if let Err(err) = spawned {
        eprintln!("pila-osd: no icons: {err}");
    }
    async move { receiver.await.ok().flatten() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(kind: Kind, percent: u32, muted: bool) -> Level {
        Level {
            kind,
            percent,
            muted,
        }
    }

    #[test]
    fn icons_follow_the_level() {
        let g = |kind, percent, muted, material| glyph(&at(kind, percent, muted), material);
        assert_eq!(g(Kind::Volume, 80, false, true), MATERIAL_VOLUME[2]);
        assert_eq!(g(Kind::Volume, 50, false, true), MATERIAL_VOLUME[1]);
        assert_eq!(g(Kind::Volume, 10, false, true), MATERIAL_VOLUME[0]);
        assert_eq!(g(Kind::Volume, 80, true, true), MATERIAL_VOLUME_OFF);
        assert_eq!(g(Kind::Volume, 0, false, true), MATERIAL_VOLUME_OFF);
        assert_eq!(g(Kind::Volume, 150, false, false), AWESOME_VOLUME[2]);
        assert_eq!(g(Kind::Volume, 20, false, false), AWESOME_VOLUME[1]);
        assert_eq!(g(Kind::Volume, 20, true, false), AWESOME_VOLUME[0]);
        assert_eq!(g(Kind::Mic, 20, true, true), MATERIAL_MIC_OFF);
        assert_eq!(g(Kind::Mic, 20, false, false), AWESOME_MIC);
        assert_eq!(
            g(Kind::Brightness, 100, false, true),
            MATERIAL_BRIGHTNESS[2]
        );
        assert_eq!(g(Kind::Brightness, 5, false, false), AWESOME_SUN);
    }

    #[test]
    fn media_icons_follow_what_was_done() {
        assert_eq!(media_glyph(State::Playing, true), '\u{f040a}');
        assert_eq!(media_glyph(State::Paused, false), '\u{f04c}');
        assert_eq!(media_glyph(State::NoPlayer, true), '\u{f075b}');
        assert_eq!(media_glyph(State::Previous, false), '\u{f048}');
    }
}
