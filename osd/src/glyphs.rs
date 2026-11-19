//! Which built-in icon (`symbols.rs`, pila-bar's) the popup shows: the
//! volume by thirds, a muted speaker or microphone, the sun by thirds; play,
//! pause, skip, stop for the media keys, or a note crossed out when there's
//! no player.

use crate::{
    level::{Kind, Level},
    media::State,
    symbols::Symbol,
};

/// The icon for `level`.
pub fn level(level: &Level) -> Symbol {
    let third = match level.percent {
        0..=33 => 0,
        34..=66 => 1,
        _ => 2,
    };
    match level.kind {
        Kind::Volume if level.muted || level.percent == 0 => Symbol::VolumeOff,
        Kind::Volume => [Symbol::VolumeLow, Symbol::VolumeMedium, Symbol::VolumeHigh][third],
        Kind::Mic if level.muted => Symbol::MicOff,
        Kind::Mic => Symbol::Mic,
        Kind::Brightness => [
            Symbol::BrightnessLow,
            Symbol::BrightnessMedium,
            Symbol::BrightnessHigh,
        ][third],
    }
}

/// The icon for what a media key did.
pub fn media(state: State) -> Symbol {
    match state {
        State::Playing => Symbol::Play,
        State::Paused => Symbol::Pause,
        State::Stopped => Symbol::Stop,
        State::Next => Symbol::Next,
        State::Previous => Symbol::Previous,
        State::NoPlayer => Symbol::NoPlayer,
    }
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
        let g = |kind, percent, muted| level(&at(kind, percent, muted));
        assert_eq!(g(Kind::Volume, 80, false), Symbol::VolumeHigh);
        assert_eq!(g(Kind::Volume, 50, false), Symbol::VolumeMedium);
        assert_eq!(g(Kind::Volume, 10, false), Symbol::VolumeLow);
        assert_eq!(g(Kind::Volume, 150, false), Symbol::VolumeHigh);
        assert_eq!(g(Kind::Volume, 80, true), Symbol::VolumeOff);
        assert_eq!(g(Kind::Volume, 0, false), Symbol::VolumeOff);
        assert_eq!(g(Kind::Mic, 20, true), Symbol::MicOff);
        assert_eq!(g(Kind::Mic, 20, false), Symbol::Mic);
        assert_eq!(g(Kind::Brightness, 100, false), Symbol::BrightnessHigh);
        assert_eq!(g(Kind::Brightness, 5, false), Symbol::BrightnessLow);
    }

    #[test]
    fn media_icons_follow_what_was_done() {
        assert_eq!(media(State::Playing), Symbol::Play);
        assert_eq!(media(State::Paused), Symbol::Pause);
        assert_eq!(media(State::NoPlayer), Symbol::NoPlayer);
        assert_eq!(media(State::Previous), Symbol::Previous);
    }
}
