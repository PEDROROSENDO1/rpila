//! The wallpaper's surfaces: one per monitor while there's a picture, each a
//! Background layer surface anchored to every edge (under the bars' zones
//! too), taking no keyboard, showing the picture as `[wallpaper] fit` says on
//! `[colors] background`.
//!
//! An iced_layershell daemon: no surface while there's no image; the outputs
//! come from `screens.rs`, the image and its fit from Pila (`wallpaper`
//! events), each new one loaded on a thread (`picture::load`) before it's
//! shown, so the old one stays until then.

use std::process::ExitCode;

use iced::{
    widget::{container, image},
    window, Color, Element, Length, Subscription, Task, Theme,
};
use iced_layershell::{
    reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption},
    settings::{LayerShellSettings, Settings, StartMode},
    to_layer_message,
};
use serde_json::Value;
use wayland_client::{protocol::wl_output::WlOutput, Connection};

use crate::{
    outputs::Change,
    picture::{self, Placement, Wanted},
    pila_ipc, screens,
    style::{self, Style},
};

/// The layer surfaces' namespace, for `[[layer-rule]]`s.
pub const NAMESPACE: &str = "pila-wallpaper";
const CLIENT: &str = "pila-wallpaper";

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    /// An output to cover, or one gone.
    Output(Change<WlOutput>),
    /// An event of Pila's IPC: the wallpaper or the colors.
    Pila(Value),
    /// The picture of the wallpaper wanted, or why there's none.
    Loaded(Wanted, Result<image::Handle, String>),
    Closed(window::Id),
}

struct Wallpaper {
    outputs: Vec<WlOutput>,
    /// Each output's surface, while there's a picture.
    surfaces: Vec<(WlOutput, window::Id)>,
    /// What Pila asks for last; `None` for no image.
    wanted: Option<Wanted>,
    /// The picture shown and how it fits.
    shown: Option<(image::Handle, Placement)>,
    /// `[colors] background`, around a picture that doesn't fill a monitor.
    background: Color,
}

pub fn run() -> ExitCode {
    // One connection for iced_layershell and the outputs' thread: the
    // outputs that thread binds are ones the layer shell can use.
    let connection = match Connection::connect_to_env() {
        Ok(connection) => connection,
        Err(err) => {
            eprintln!("{CLIENT}: can't connect to Wayland: {err}");
            return ExitCode::FAILURE;
        }
    };
    let outputs_connection = connection.clone();
    let boot = move || {
        let wallpaper = Wallpaper {
            outputs: Vec::new(),
            surfaces: Vec::new(),
            wanted: None,
            shown: None,
            background: Style::default().edge,
        };
        let tasks = [
            Task::run(
                screens::changes(outputs_connection.clone()),
                Message::Output,
            ),
            Task::run(
                pila_ipc::events(CLIENT, &["wallpaper", "colors"]),
                Message::Pila,
            ),
        ];
        (wallpaper, Task::batch(tasks))
    };
    let result = iced_layershell::daemon(boot, NAMESPACE, Wallpaper::update, Wallpaper::view)
        .subscription(Wallpaper::subscription)
        .style(|wallpaper: &Wallpaper, _: &Theme| iced::theme::Style {
            // Opaque: the software renderer's buffers have no alpha.
            background_color: wallpaper.background,
            text_color: Color::WHITE,
        })
        .settings(Settings {
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background,
                ..Default::default()
            },
            default_font: style::FONT,
            with_connection: Some(connection.into()),
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

impl Wallpaper {
    fn subscription(&self) -> Subscription<Message> {
        window::close_events().map(Message::Closed)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Output(Change::Open(output)) => {
                self.outputs.push(output.clone());
                if self.shown.is_some() {
                    return self.open(output);
                }
                Task::none()
            }
            Message::Output(Change::Close(output)) => {
                self.outputs.retain(|o| o != &output);
                match self.surfaces.iter().position(|(o, _)| o == &output) {
                    Some(index) => {
                        let (_, id) = self.surfaces.remove(index);
                        Task::done(Message::RemoveWindow(id))
                    }
                    None => Task::none(),
                }
            }
            Message::Pila(event) if event["event"] == "colors" => {
                self.background = Style::from_pila(&event["colors"]).edge;
                Task::none()
            }
            Message::Pila(event) if event["event"] == "wallpaper" => {
                let wanted = Wanted::from_pila(&event["wallpaper"]);
                if wanted == self.wanted {
                    return Task::none();
                }
                self.wanted = wanted.clone();
                match wanted {
                    None => {
                        self.shown = None;
                        self.close_all()
                    }
                    Some(wanted) => Task::perform(picture::load(wanted.clone()), move |loaded| {
                        Message::Loaded(wanted.clone(), loaded)
                    }),
                }
            }
            Message::Pila(_) => Task::none(),
            // A picture no longer wanted (another came since) is dropped.
            Message::Loaded(wanted, _) if self.wanted.as_ref() != Some(&wanted) => Task::none(),
            Message::Loaded(wanted, Ok(handle)) => {
                let first = self.shown.is_none();
                self.shown = Some((handle, wanted.fit));
                if first {
                    let outputs = self.outputs.clone();
                    return Task::batch(outputs.into_iter().map(|o| self.open(o)));
                }
                Task::none()
            }
            Message::Loaded(_, Err(err)) => {
                eprintln!("{CLIENT}: {err}");
                self.shown = None;
                self.close_all()
            }
            Message::Closed(id) => {
                self.surfaces.retain(|(_, w)| *w != id);
                Task::none()
            }
            // The layer shell's own messages are handled before `update`.
            _ => Task::none(),
        }
    }

    /// A surface covering `output`, under everything.
    fn open(&mut self, output: WlOutput) -> Task<Message> {
        if self.surfaces.iter().any(|(o, _)| o == &output) {
            return Task::none();
        }
        let (id, open) = Message::layershell_open(NewLayerShellSettings {
            // The whole output.
            size: Some((0, 0)),
            layer: Layer::Background,
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            // Under the bars' exclusive zones too.
            exclusive_zone: Some(-1),
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::None,
            output_option: OutputOption::Output(output.clone()),
            events_transparent: false,
            namespace: Some(NAMESPACE.to_string()),
        });
        self.surfaces.push((output, id));
        open
    }

    fn close_all(&mut self) -> Task<Message> {
        Task::batch(
            self.surfaces
                .drain(..)
                .map(|(_, id)| Task::done(Message::RemoveWindow(id))),
        )
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let background = self.background;
        let picture: Element<'_, Message> = match &self.shown {
            Some((handle, fit)) => image(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(fit.content_fit())
                .into(),
            None => iced::widget::Space::new().into(),
        };
        container(picture)
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill)
            .style(move |_| container::Style::default().background(background))
            .into()
    }
}
