//! The popup: an icon (or the level's name, without an icon font), a square
//! level bar and the percentage; or for a media key, its icon and the
//! track's title (shortened to fit, `fit`). At the bottom of the monitor, as a layer
//! surface (an Overlay anchored to the bottom edge, which the compositor
//! centers along it, on the monitor it picks: Pila's is the pointer's)
//! that takes neither the keyboard nor the pointer.
//!
//! An iced_layershell daemon: no surface while hidden. A popup shows it (a
//! new surface if none) and keeps it `SHOWN` from then; the popups of
//! later runs come in through a stream from the socket's thread.

use std::{os::unix::net::UnixListener, process::ExitCode, sync::Mutex, time::Duration};

use iced::{
    widget::{container, progress_bar, row, text, text::Wrapping},
    window, Alignment, Border, Element, Length, Subscription, Task, Theme,
};
use iced_layershell::{
    reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption},
    settings::{LayerShellSettings, Settings, StartMode},
    to_layer_message,
};

use crate::{
    glyphs::{self, IconFont},
    level::{Kind, Level},
    media::Media,
    pila_ipc,
    style::{self, Style},
    Popup,
};

/// The layer surface's namespace, for `[[layer-rule]]`s.
pub const NAMESPACE: &str = "pila-osd";

/// How long a popup stays shown after the last one came.
const SHOWN: Duration = Duration::from_millis(1200);
const WIDTH: u32 = 320;
const HEIGHT: u32 = 52;
/// Above the bottom edge (and any bar there: Pila keeps it clear of
/// exclusive zones).
const MARGIN: i32 = 96;
const ICON_SIZE: f32 = 22.0;
/// How much of a title fits beside the icon, in narrow characters (a wide
/// one, CJK's, counts two): 14 px text in what's left of `WIDTH`.
const TITLE_FITS: usize = 34;

const CLIENT: &str = "pila-osd";

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    /// A popup to show, the first or from a later run.
    Show(Popup),
    /// The time to hide after the popup numbered `.0` is up.
    Expired(u64),
    Fonts(Option<IconFont>),
    Closed(window::Id),
    /// Pila's colors, new or changed.
    Style(Style),
}

struct Osd {
    window: Option<window::Id>,
    popup: Popup,
    /// Counts the popups shown, so that only the last one's time hides it.
    shown: u64,
    icons: Option<IconFont>,
    style: Style,
}

pub fn run(listener: UnixListener, first: Popup) -> ExitCode {
    // The boot function may run only once, but must be `Fn`.
    let listener = Mutex::new(Some(listener));
    let boot = move || {
        let mut osd = Osd {
            window: None,
            popup: first.clone(),
            shown: 0,
            icons: None,
            style: Style::default(),
        };
        let show = osd.show(first.clone());
        let popups = match listener.lock().ok().and_then(|mut l| l.take()) {
            Some(listener) => Task::run(popups(listener), Message::Show),
            None => Task::none(),
        };
        let fonts = Task::perform(glyphs::found(), Message::Fonts);
        let styles = Task::run(pila_ipc::styles(CLIENT), Message::Style);
        (osd, Task::batch([show, popups, fonts, styles]))
    };
    let result = iced_layershell::daemon(boot, NAMESPACE, Osd::update, Osd::view)
        .subscription(Osd::subscription)
        .theme(|osd: &Osd, _| Some(osd.style.theme()))
        .style(|_: &Osd, theme: &Theme| iced::theme::Style {
            // Opaque: the software renderer's buffers have no alpha.
            background_color: theme.palette().background,
            text_color: theme.palette().text,
        })
        .settings(Settings {
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background,
                ..Default::default()
            },
            default_font: style::FONT,
            default_text_size: style::TEXT_SIZE.into(),
            ..Default::default()
        })
        .run();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{CLIENT}: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The popups of later runs, read on a thread of their own.
fn popups(listener: UnixListener) -> impl iced::futures::Stream<Item = Popup> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("pila-osd-socket".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Some(popup) = crate::read_request(stream) {
                    if sender.unbounded_send(popup).is_err() {
                        break;
                    }
                }
            }
        });
    if let Err(err) = spawned {
        eprintln!("{CLIENT}: can't listen for later runs: {err}");
    }
    receiver
}

impl Osd {
    fn subscription(&self) -> Subscription<Message> {
        window::close_events().map(Message::Closed)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Show(popup) => self.show(popup),
            Message::Expired(shown) if shown == self.shown => match self.window.take() {
                Some(id) => Task::done(Message::RemoveWindow(id)),
                None => Task::none(),
            },
            Message::Expired(_) => Task::none(),
            Message::Fonts(icons) => {
                self.icons = icons;
                Task::none()
            }
            Message::Closed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                }
                Task::none()
            }
            Message::Style(style) => {
                self.style = style;
                Task::none()
            }
            // The layer shell's own messages are handled before `update`.
            _ => Task::none(),
        }
    }

    /// Show `popup`: on the surface there is, or a new one; hidden `SHOWN`
    /// after, unless another comes first.
    fn show(&mut self, popup: Popup) -> Task<Message> {
        self.popup = popup;
        self.shown += 1;
        let shown = self.shown;
        let expire = Task::perform(after(SHOWN), move |()| Message::Expired(shown));
        if self.window.is_some() {
            return expire;
        }
        let (id, open) = Message::layershell_open(NewLayerShellSettings {
            size: Some((WIDTH, HEIGHT)),
            layer: Layer::Overlay,
            // The bottom edge alone: the compositor centers it along it.
            anchor: Anchor::Bottom,
            exclusive_zone: None,
            margin: Some((0, 0, MARGIN, 0)),
            keyboard_interactivity: KeyboardInteractivity::None,
            // The compositor's choice: Pila's is the monitor the pointer is on.
            output_option: OutputOption::Active,
            // Clicks go through it.
            events_transparent: true,
            namespace: Some(NAMESPACE.to_string()),
        });
        self.window = Some(id);
        Task::batch([open, expire])
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let (line, dim) = match &self.popup {
            Popup::Level(level) => (self.level_line(*level), level.muted),
            Popup::Media(media) => (self.media_line(media), false),
        };
        let st = self.style;
        container(line)
            .padding([0, 16])
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(Alignment::Center)
            .style(move |_| container::Style {
                background: Some(st.surface_dim.into()),
                text_color: Some(if dim { st.text_dim } else { st.text }),
                border: Border {
                    color: st.hover,
                    width: 1.0,
                    // Square: transparent corners would show black.
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    /// A media key's: its icon (or what it did, in a word) and the title.
    fn media_line(&self, media: &Media) -> Element<'_, Message> {
        let icon: Element<'_, Message> = match self.icons {
            Some(icons) => text(glyphs::media_glyph(media.state, icons.material))
                .font(icons.font)
                .size(ICON_SIZE)
                .width(ICON_SIZE + 6.0)
                .into(),
            None => text(media.state.word()).font(style::FONT_STRONG).into(),
        };
        row![
            icon,
            text(fit(&media.title, TITLE_FITS))
                .wrapping(Wrapping::None)
                .width(Length::Fill),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    }

    /// A level's: its icon (or name), the bar and the percentage.
    fn level_line(&self, level: Level) -> Element<'_, Message> {
        let st = self.style;
        let dim = level.muted;
        let icon: Element<'_, Message> = match self.icons {
            Some(icons) => text(glyphs::glyph(&level, icons.material))
                .font(icons.font)
                .size(ICON_SIZE)
                .width(ICON_SIZE + 6.0)
                .into(),
            None => text(match level.kind {
                Kind::Volume => "Volume",
                Kind::Mic => "Microphone",
                Kind::Brightness => "Brightness",
            })
            .into(),
        };
        let fill = if dim { st.text_dim } else { st.accent };
        let bar = progress_bar(0.0..=100.0, level.percent.min(100) as f32)
            .length(Length::Fill)
            .girth(8.0)
            .style(move |_| progress_bar::Style {
                background: st.hover.into(),
                bar: fill.into(),
                border: Border::default(),
            });
        let amount = if dim {
            "muted".to_string()
        } else {
            format!("{}%", level.percent)
        };
        row![
            icon,
            bar,
            text(amount)
                .font(style::FONT_STRONG)
                .width(52.0)
                .align_x(Alignment::End),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    }
}

/// `title` cut to `fits` narrow characters (a wide one counts two), with an
/// ellipsis where it's cut.
fn fit(title: &str, fits: usize) -> String {
    let width = |c: char| if is_wide(c) { 2 } else { 1 };
    if title.chars().map(width).sum::<usize>() <= fits {
        return title.to_string();
    }
    let mut used = 0;
    let mut cut: String = title
        .chars()
        .take_while(|&c| {
            used += width(c);
            used < fits
        })
        .collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// East Asian wide characters, which take two columns: Hangul, CJK, kana,
/// full-width forms.
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD)
}

/// Ready after `delay`, waited on a thread (iced here has no timers).
async fn after(delay: Duration) {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name(format!("{CLIENT}-timer"))
        .spawn(move || {
            std::thread::sleep(delay);
            let _ = sender.send(());
        });
    if spawned.is_ok() {
        let _ = receiver.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_fit() {
        assert_eq!(fit("Song — Band", 34), "Song — Band");
        assert_eq!(fit("abcdefghij", 10), "abcdefghij");
        assert_eq!(fit("abcdefghijk", 10), "abcdefghi…");
        // Not "abc …".
        assert_eq!(fit("abc defghijk", 5), "abc…");
        assert_eq!(fit("日本語の歌のタイトル", 10), "日本語の…");
    }
}
