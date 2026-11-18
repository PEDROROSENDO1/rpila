//! Icons drawn as text, from an icon font the user has installed: a Nerd
//! Font (any of them, `Symbols Nerd Font` included) or Font Awesome. Looked
//! for once, on a thread at startup (`find`): without one the bar shows words
//! and numbers alone, never a missing glyph's box.
//!
//! The battery is one icon. A Nerd Font has Material Design's batteries,
//! by tens, each also with a bolt inside for charging; Font Awesome has
//! batteries by quarters only (at the same code points in Nerd Fonts), so
//! with it a charging battery is drawn in the accent color instead.

use iced::{
    font::{Family, Style, Weight},
    Font,
};

use crate::battery::{Battery, State};

/// Font Awesome's battery levels, empty to full.
const QUARTERS: [char; 5] = ['\u{f244}', '\u{f243}', '\u{f242}', '\u{f241}', '\u{f240}'];

/// Material Design's batteries in Nerd Fonts, 0% to 100% by tens: as
/// they are (`md-battery_outline`, `md-battery_10`.., `md-battery`), and
/// charging (`md-battery_charging_outline`, `md-battery_charging_10`..).
const TENS: [char; 11] = [
    '\u{f008e}',
    '\u{f007a}',
    '\u{f007b}',
    '\u{f007c}',
    '\u{f007d}',
    '\u{f007e}',
    '\u{f007f}',
    '\u{f0080}',
    '\u{f0081}',
    '\u{f0082}',
    '\u{f0079}',
];
const TENS_CHARGING: [char; 11] = [
    '\u{f089f}',
    '\u{f089c}',
    '\u{f0086}',
    '\u{f0087}',
    '\u{f0088}',
    '\u{f089d}',
    '\u{f0089}',
    '\u{f089e}',
    '\u{f008a}',
    '\u{f008b}',
    '\u{f0085}',
];

/// The icon font found, and whether it has Material Design's batteries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconFont {
    pub font: Font,
    pub tens: bool,
}

/// An icon to draw, and whether in the accent color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    pub glyph: char,
    pub accent: bool,
}

/// The battery's one icon: its level, with a bolt inside while charging; in
/// a font with quarters only, its level, in the accent color while
/// charging. Plugged in and not charging, its level alone.
pub fn battery(battery: &Battery, tens: bool) -> Icon {
    let charging = battery.state == State::Charging;
    let percent = usize::from(battery.percent.min(100));
    if tens {
        let level = (percent + 5) / 10;
        let glyph = match charging {
            true => TENS_CHARGING[level],
            false => TENS[level],
        };
        return Icon {
            glyph,
            accent: false,
        };
    }
    let level = match percent {
        88.. => 4,
        63.. => 3,
        38.. => 2,
        13.. => 1,
        _ => 0,
    };
    Icon {
        glyph: QUARTERS[level],
        accent: charging,
    }
}

/// A font face the system has: its family, weight and whether it's italic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
}

/// The icon font installed, if there's one with the batteries by quarters
/// (and whether it has them by tens too): read from the system's fonts as
/// iced reads them (fontdb).
pub fn find() -> Option<IconFont> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let faces: Vec<(Face, fontdb::ID)> = db
        .faces()
        .filter_map(|face| {
            let family = face.families.first()?.0.clone();
            let face_info = Face {
                family,
                weight: face.weight.0,
                italic: face.style != fontdb::Style::Normal,
            };
            Some((face_info, face.id))
        })
        .collect();
    // Whether a face has the quarters, and whether the tens too.
    let has = |id: fontdb::ID| {
        db.with_face_data(id, |data, index| {
            let face = ttf_parser::Face::parse(data, index).ok()?;
            let all = |glyphs: &[char]| glyphs.iter().all(|&c| face.glyph_index(c).is_some());
            Some((all(&QUARTERS), all(&TENS) && all(&TENS_CHARGING)))
        })
        .flatten()
        .unwrap_or((false, false))
    };
    let icon_font = |face: &Face, id| {
        let (quarters, tens) = has(id);
        quarters.then(|| IconFont {
            font: font(face),
            tens,
        })
    };
    let chosen = pick(faces.iter().map(|(face, _)| face))?;
    let id = faces.iter().find(|(face, _)| face == chosen)?.1;
    icon_font(chosen, id).or_else(|| {
        // The likeliest one lacks them (an old Font Awesome): any that has.
        faces
            .iter()
            .filter(|(face, _)| is_icon_family(&face.family) && !face.italic)
            .find_map(|(face, id)| icon_font(face, *id))
    })
}

/// The family names icon fonts go by: Font Awesome 4 (Debian's and
/// Ubuntu's) is `FontAwesome`, later ones `Font Awesome 6 Free` and so on.
fn is_icon_family(family: &str) -> bool {
    family.contains("Nerd Font") || family.replace(' ', "").starts_with("FontAwesome")
}

/// The likeliest icon font among `faces`: Nerd Fonts' symbols alone first
/// (made for this), then any Nerd Font, then Font Awesome's solid style
/// (its free icons are in its heaviest face); never an italic.
fn pick<'a>(faces: impl Iterator<Item = &'a Face>) -> Option<&'a Face> {
    faces
        .filter(|face| is_icon_family(&face.family) && !face.italic)
        .min_by_key(|face| {
            let rank = if face.family.starts_with("Symbols Nerd Font") {
                0
            } else if face.family.contains("Nerd Font") {
                1
            } else {
                2
            };
            // Regular for a Nerd Font; for Font Awesome, the heaviest.
            let weight = match rank {
                2 => u16::MAX - face.weight,
                _ => face.weight.abs_diff(400),
            };
            (rank, face.family.contains("Mono"), weight)
        })
}

/// An iced font for a face: its family by name, its weight.
fn font(face: &Face) -> Font {
    // iced names a family by a 'static str; there's one, found once.
    let family: &'static str = Box::leak(face.family.clone().into_boxed_str());
    Font {
        family: Family::Name(family),
        weight: match face.weight {
            0..=149 => Weight::Thin,
            150..=249 => Weight::ExtraLight,
            250..=349 => Weight::Light,
            350..=449 => Weight::Normal,
            450..=549 => Weight::Medium,
            550..=649 => Weight::Semibold,
            650..=749 => Weight::Bold,
            750..=849 => Weight::ExtraBold,
            _ => Weight::Black,
        },
        style: Style::Normal,
        ..Font::DEFAULT
    }
}

/// The icon font, from a thread of its own: one answer, `None` if there's
/// none (or the thread couldn't start).
pub fn found() -> impl iced::futures::Future<Output = Option<IconFont>> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("pila-bar-fonts".into())
        .spawn(move || {
            let _ = sender.send(find());
        });
    if let Err(err) = spawned {
        eprintln!("pila-bar: no icons: {err}");
    }
    async move { receiver.await.ok().flatten() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(family: &str, weight: u16) -> Face {
        Face {
            family: family.into(),
            weight,
            italic: false,
        }
    }

    #[test]
    fn one_battery_icon() {
        let at = |percent, state, tens| battery(&Battery { percent, state }, tens);
        let plain = |glyph| Icon {
            glyph,
            accent: false,
        };
        // By tens, a bolt inside while charging.
        assert_eq!(at(100, State::Discharging, true), plain(TENS[10]));
        assert_eq!(at(64, State::Discharging, true), plain(TENS[6]));
        assert_eq!(at(65, State::Discharging, true), plain(TENS[7]));
        assert_eq!(at(4, State::Discharging, true), plain(TENS[0]));
        assert_eq!(at(42, State::Charging, true), plain(TENS_CHARGING[4]));
        assert_eq!(at(100, State::Plugged, true), plain(TENS[10]));
        // By quarters, in the accent color while charging.
        assert_eq!(at(88, State::Discharging, false), plain(QUARTERS[4]));
        assert_eq!(at(87, State::Discharging, false), plain(QUARTERS[3]));
        assert_eq!(at(50, State::Discharging, false), plain(QUARTERS[2]));
        assert_eq!(at(20, State::Discharging, false), plain(QUARTERS[1]));
        assert_eq!(at(5, State::Discharging, false), plain(QUARTERS[0]));
        assert_eq!(
            at(50, State::Charging, false),
            Icon {
                glyph: QUARTERS[2],
                accent: true
            }
        );
        assert_eq!(at(100, State::Plugged, false), plain(QUARTERS[4]));
    }

    #[test]
    fn the_likeliest_icon_font() {
        let faces = [
            face("DejaVu Sans", 400),
            face("Font Awesome 7 Free", 400),
            face("Font Awesome 7 Free", 900),
            face("JetBrainsMono Nerd Font", 700),
            face("JetBrainsMono Nerd Font", 400),
            face("JetBrainsMono Nerd Font Mono", 400),
        ];
        let pick_of = |faces: &[Face]| pick(faces.iter()).cloned();
        assert_eq!(pick_of(&faces), Some(face("JetBrainsMono Nerd Font", 400)));
        let mut with_symbols = faces.to_vec();
        with_symbols.push(face("Symbols Nerd Font Mono", 400));
        with_symbols.push(face("Symbols Nerd Font", 400));
        assert_eq!(pick_of(&with_symbols), Some(face("Symbols Nerd Font", 400)));
        // Font Awesome alone: its solid face.
        assert_eq!(pick_of(&faces[..3]), Some(face("Font Awesome 7 Free", 900)));
        assert_eq!(pick_of(&faces[..1]), None);
        // Font Awesome 4, one face.
        assert_eq!(
            pick_of(&[face("DejaVu Sans", 400), face("FontAwesome", 400)]),
            Some(face("FontAwesome", 400))
        );
        let italic = Face {
            italic: true,
            ..face("Symbols Nerd Font", 400)
        };
        assert_eq!(pick_of(&[italic]), None);
    }
}
