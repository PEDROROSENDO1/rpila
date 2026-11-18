//! Icons drawn as text, from an icon font the user has installed: a Nerd
//! Font (any of them, `Symbols Nerd Font` included) or Font Awesome, which
//! put the same icons at the same code points. Looked for once, on a thread
//! at startup (`find`): without one the bar shows words and numbers alone,
//! never a missing glyph's box.

use iced::{
    font::{Family, Style, Weight},
    Font,
};

use crate::battery::{Battery, State};

/// Font Awesome's battery levels, full to empty, its bolt and its plug.
const BATTERY_FULL: char = '\u{f240}';
const BATTERY_THREE_QUARTERS: char = '\u{f241}';
const BATTERY_HALF: char = '\u{f242}';
const BATTERY_QUARTER: char = '\u{f243}';
const BATTERY_EMPTY: char = '\u{f244}';
const BOLT: char = '\u{f0e7}';
const PLUG: char = '\u{f1e6}';

/// What a font must have to be used: every icon the bar draws.
const NEEDED: [char; 7] = [
    BATTERY_FULL,
    BATTERY_THREE_QUARTERS,
    BATTERY_HALF,
    BATTERY_QUARTER,
    BATTERY_EMPTY,
    BOLT,
    PLUG,
];

/// The battery's icon: its level in quarters, after a bolt while charging
/// or a plug while plugged in and not charging.
pub fn battery(battery: &Battery) -> String {
    let level = match battery.percent {
        88.. => BATTERY_FULL,
        63.. => BATTERY_THREE_QUARTERS,
        38.. => BATTERY_HALF,
        13.. => BATTERY_QUARTER,
        _ => BATTERY_EMPTY,
    };
    match battery.state {
        State::Charging => format!("{BOLT} {level}"),
        State::Plugged => format!("{PLUG} {level}"),
        State::Discharging => level.to_string(),
    }
}

/// A font face the system has: its family, weight and whether it's italic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
}

/// The icon font installed, if there's one with every icon the bar draws:
/// read from the system's fonts as iced reads them (fontdb).
pub fn find() -> Option<Font> {
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
    let has_icons = |id: fontdb::ID| {
        db.with_face_data(id, |data, index| {
            let face = ttf_parser::Face::parse(data, index).ok()?;
            Some(NEEDED.iter().all(|&c| face.glyph_index(c).is_some()))
        })
        .flatten()
        .unwrap_or(false)
    };
    let chosen = pick(faces.iter().map(|(face, _)| face))?;
    let id = faces.iter().find(|(face, _)| face == chosen)?.1;
    has_icons(id).then(|| font(chosen)).or_else(|| {
        // The likeliest one lacks them (an old Font Awesome): any that has.
        faces
            .iter()
            .filter(|(face, _)| is_icon_family(&face.family) && !face.italic)
            .find(|(_, id)| has_icons(*id))
            .map(|(face, _)| font(face))
    })
}

/// The family names icon fonts go by.
fn is_icon_family(family: &str) -> bool {
    family.contains("Nerd Font") || family.starts_with("Font Awesome")
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
pub fn found() -> impl iced::futures::Future<Output = Option<Font>> {
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
    fn the_battery_by_quarters() {
        let at = |percent, state| battery(&Battery { percent, state });
        assert_eq!(at(100, State::Discharging), BATTERY_FULL.to_string());
        assert_eq!(at(88, State::Discharging), BATTERY_FULL.to_string());
        assert_eq!(
            at(87, State::Discharging),
            BATTERY_THREE_QUARTERS.to_string()
        );
        assert_eq!(at(50, State::Discharging), BATTERY_HALF.to_string());
        assert_eq!(at(20, State::Discharging), BATTERY_QUARTER.to_string());
        assert_eq!(at(5, State::Discharging), BATTERY_EMPTY.to_string());
        assert_eq!(at(50, State::Charging), format!("{BOLT} {BATTERY_HALF}"));
        assert_eq!(at(100, State::Plugged), format!("{PLUG} {BATTERY_FULL}"));
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
        let italic = Face {
            italic: true,
            ..face("Symbols Nerd Font", 400)
        };
        assert_eq!(pick_of(&[italic]), None);
    }
}
