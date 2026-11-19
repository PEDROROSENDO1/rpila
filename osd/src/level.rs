//! The levels shown, read where they are: the volume and the microphone from
//! PipeWire's `wpctl get-volume` (its default sink and source), the
//! brightness from the backlight in `/sys/class/backlight` (`backlight.rs`).

use std::{path::Path, process::Command};

use crate::backlight;

/// What a level is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Volume,
    Mic,
    Brightness,
}

impl Kind {
    pub fn named(name: &str) -> Option<Kind> {
        match name {
            "volume" => Some(Kind::Volume),
            "mic" => Some(Kind::Mic),
            "brightness" => Some(Kind::Brightness),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Volume => "volume",
            Kind::Mic => "mic",
            Kind::Brightness => "brightness",
        }
    }
}

/// A level to show: in percent (the volume may go past 100), muted or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub kind: Kind,
    pub percent: u32,
    pub muted: bool,
}

impl Level {
    /// The level of a backlight.
    pub fn brightness(device: &backlight::Device) -> Level {
        Level {
            kind: Kind::Brightness,
            percent: device.percent(),
            muted: false,
        }
    }

    /// The level as a request to the run in the background: one line.
    pub fn encode(&self) -> String {
        let muted = if self.muted { " muted" } else { "" };
        format!("{} {}{muted}\n", self.kind.name(), self.percent)
    }

    pub fn decode(text: &str) -> Option<Level> {
        let mut words = text.split_whitespace();
        let kind = Kind::named(words.next()?)?;
        let percent = words.next()?.parse().ok()?;
        let muted = match words.next() {
            None => false,
            Some("muted") => true,
            Some(_) => return None,
        };
        Some(Level {
            kind,
            percent,
            muted,
        })
    }
}

/// The level of `kind` now, or why it can't be read.
pub fn read(kind: Kind) -> Result<Level, String> {
    match kind {
        Kind::Volume | Kind::Mic => {
            let node = match kind {
                Kind::Volume => "@DEFAULT_AUDIO_SINK@",
                _ => "@DEFAULT_AUDIO_SOURCE@",
            };
            let out = Command::new("wpctl")
                .args(["get-volume", node])
                .output()
                .map_err(|err| format!("can't run wpctl (package wireplumber): {err}"))?;
            let text = String::from_utf8_lossy(&out.stdout);
            let (percent, muted) =
                wpctl_volume(&text).ok_or_else(|| format!("wpctl said {:?}", text.trim()))?;
            Ok(Level {
                kind,
                percent,
                muted,
            })
        }
        Kind::Brightness => {
            let device = backlight::find(Path::new(backlight::CLASS))
                .ok_or_else(|| format!("no backlight in {}", backlight::CLASS))?;
            Ok(Level::brightness(&device))
        }
    }
}

/// `wpctl get-volume`'s answer, `Volume: 0.45` or `Volume: 0.45 [MUTED]`,
/// as a percentage and whether it's muted.
pub fn wpctl_volume(text: &str) -> Option<(u32, bool)> {
    let rest = text.trim().strip_prefix("Volume:")?.trim();
    let (number, muted) = match rest.strip_suffix("[MUTED]") {
        Some(number) => (number.trim(), true),
        None => (rest, false),
    };
    let volume: f64 = number.parse().ok()?;
    if !volume.is_finite() || volume < 0.0 {
        return None;
    }
    Some(((volume * 100.0).round() as u32, muted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpctl_volumes() {
        assert_eq!(wpctl_volume("Volume: 0.45\n"), Some((45, false)));
        assert_eq!(wpctl_volume("Volume: 0.40 [MUTED]\n"), Some((40, true)));
        assert_eq!(wpctl_volume("Volume: 1.25"), Some((125, false)));
        assert_eq!(wpctl_volume("Volume: 0.005"), Some((1, false)));
        for wrong in [
            "",
            "Volume:",
            "Volume: loud",
            "Volume: -1",
            "Gain: 0.4",
            "Volume: NaN",
        ] {
            assert_eq!(wpctl_volume(wrong), None, "{wrong:?}");
        }
    }

    #[test]
    fn levels_go_to_the_background_run_and_back() {
        let level = Level {
            kind: Kind::Volume,
            percent: 45,
            muted: true,
        };
        assert_eq!(level.encode(), "volume 45 muted\n");
        assert_eq!(Level::decode(&level.encode()), Some(level));
        let bright = Level {
            kind: Kind::Brightness,
            percent: 100,
            muted: false,
        };
        assert_eq!(Level::decode(&bright.encode()), Some(bright));
        for wrong in ["", "volume", "loudness 4", "mic x", "mic 4 quiet"] {
            assert_eq!(Level::decode(wrong), None, "{wrong:?}");
        }
    }
}
