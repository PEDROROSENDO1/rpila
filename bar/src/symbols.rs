//! Pila's own icons, built into the programs that draw them (pila-bar's
//! battery and clock, pila-osd's popup, which takes this file by
//! `#[path]`): Material Design Icons' SVG files in `bar/icons/` (Apache
//! License 2.0, `bar/icons/README.md`), drawn in the color of the text
//! beside them. Nothing to install, and nothing looked for at startup.

use std::sync::LazyLock;

use iced::{widget::svg, Color};

macro_rules! symbols {
    ($($name:ident = $file:literal,)*) => {
        /// One of the icons.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Symbol {
            $($name,)*
        }

        impl Symbol {
            /// Every icon, in the order they're declared.
            pub const ALL: &[Symbol] = &[$(Symbol::$name,)*];

            /// Its SVG file, as built in.
            pub fn svg(self) -> &'static [u8] {
                match self {
                    $(Symbol::$name => include_bytes!(concat!("../icons/", $file, ".svg")),)*
                }
            }
        }
    };
}

symbols! {
    Battery0 = "battery-outline",
    Battery10 = "battery-10",
    Battery20 = "battery-20",
    Battery30 = "battery-30",
    Battery40 = "battery-40",
    Battery50 = "battery-50",
    Battery60 = "battery-60",
    Battery70 = "battery-70",
    Battery80 = "battery-80",
    Battery90 = "battery-90",
    Battery100 = "battery",
    Charging0 = "battery-charging-outline",
    Charging10 = "battery-charging-10",
    Charging20 = "battery-charging-20",
    Charging30 = "battery-charging-30",
    Charging40 = "battery-charging-40",
    Charging50 = "battery-charging-50",
    Charging60 = "battery-charging-60",
    Charging70 = "battery-charging-70",
    Charging80 = "battery-charging-80",
    Charging90 = "battery-charging-90",
    Charging100 = "battery-charging-100",
    Clock = "clock-outline",
    VolumeLow = "volume-low",
    VolumeMedium = "volume-medium",
    VolumeHigh = "volume-high",
    VolumeOff = "volume-off",
    Mic = "microphone",
    MicOff = "microphone-off",
    BrightnessLow = "brightness-5",
    BrightnessMedium = "brightness-6",
    BrightnessHigh = "brightness-7",
    Play = "play",
    Pause = "pause",
    Stop = "stop",
    Next = "skip-next",
    Previous = "skip-previous",
    NoPlayer = "music-off",
}

/// The handles, made once: iced keeps an icon's raster by its handle.
static HANDLES: LazyLock<Vec<svg::Handle>> = LazyLock::new(|| {
    Symbol::ALL
        .iter()
        .map(|symbol| svg::Handle::from_memory(symbol.svg()))
        .collect()
});

impl Symbol {
    /// The icon, `size` pixels square, all in `color`.
    pub fn view<'a>(self, size: f32, color: Color) -> svg::Svg<'a> {
        svg(HANDLES[self as usize].clone())
            .width(size)
            .height(size)
            .style(move |_, _| svg::Style { color: Some(color) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_square_svg_of_one_shape() {
        for (index, &symbol) in Symbol::ALL.iter().enumerate() {
            assert_eq!(symbol as usize, index, "{symbol:?} out of order");
            let text = std::str::from_utf8(symbol.svg()).unwrap();
            assert!(text.starts_with("<svg "), "{symbol:?}: {text}");
            assert!(
                text.contains(r#"viewBox="0 0 24 24""#),
                "{symbol:?}: {text}"
            );
            assert_eq!(text.matches("<path ").count(), 1, "{symbol:?}: {text}");
        }
    }
}
