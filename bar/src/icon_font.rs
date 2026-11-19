//! Finding the icon font the user has installed, for icons drawn as text: a
//! Nerd Font (any of them, `Symbols Nerd Font` first) or Font Awesome, and
//! whether it has the glyphs a client needs. Read from the system's fonts as
//! iced reads them (fontdb, ttf-parser: the versions iced's text brings);
//! slow, so on a thread.
//!
//! pila-bar's file; pila-osd includes it (`#[path]`).

use iced::{
    font::{Family, Style, Weight},
    Font,
};

/// The likeliest icon font that `probe` accepts, with what it said of it
/// (the glyphs it found): the likeliest face if it does, else any icon
/// font's that does (the likeliest may be an old Font Awesome without them).
pub fn find<T>(probe: impl Fn(&ttf_parser::Face) -> Option<T>) -> Option<(Font, T)> {
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
    let probed = |face: &Face, id: fontdb::ID| {
        db.with_face_data(id, |data, index| {
            probe(&ttf_parser::Face::parse(data, index).ok()?)
        })
        .flatten()
        .map(|found| (font(face), found))
    };
    let chosen = pick(faces.iter().map(|(face, _)| face))?;
    let id = faces.iter().find(|(face, _)| face == chosen)?.1;
    probed(chosen, id).or_else(|| {
        faces
            .iter()
            .filter(|(face, _)| is_icon_family(&face.family) && !face.italic)
            .find_map(|(face, id)| probed(face, *id))
    })
}

/// A font face the system has: its family, weight and whether it's italic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
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

#[cfg(test)]
mod icon_font_tests {
    use super::*;

    fn face(family: &str, weight: u16) -> Face {
        Face {
            family: family.into(),
            weight,
            italic: false,
        }
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
