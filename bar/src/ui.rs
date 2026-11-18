//! The bar: a layer surface on every monitor, or those `--output` names (Top,
//! along the top or bottom edge, with an exclusive zone so windows keep
//! clear of it), showing the workspaces and the clock.
//!
//! An iced_layershell daemon that starts with no surface: the desktop thread
//! (`desktop.rs`, on the same Wayland connection) says which outputs get one
//! (`outputs.rs`), and the bar opens a surface on each and drops it when its
//! output goes. Every surface shows the same: Pila's workspaces span all its
//! outputs. The desktop and the clock (`clock.rs`) come in as streams from
//! threads of their own.

use std::process::ExitCode;

use iced::{
    event, mouse,
    widget::{button as button_widget, column, container, row, text, Row, Space},
    window, Alignment, Border, Element, Event, Length, Subscription, Task, Theme,
};
use iced_layershell::{
    reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption},
    settings::{LayerShellSettings, Settings, StartMode},
    to_layer_message,
};
use wayland_client::{protocol::wl_output::WlOutput, Connection};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1::ExtWorkspaceHandleV1;

use crate::{
    args::{self, Args},
    battery::{self, Battery},
    clock,
    desktop::{self, Desktop, Switcher, Update},
    outputs::Change,
    style,
};

/// The layer surfaces' namespace, for `[[layer-rule]]`s.
pub const NAMESPACE: &str = "pila-bar";

/// The line along the edge that faces the windows.
const EDGE: f32 = 1.0;
/// How far a touchpad scrolls (in pixels) for one workspace.
const PIXELS_PER_STEP: f32 = 40.0;

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Update(Update),
    Clock(String),
    Battery(Battery),
    /// A click on a workspace (its place in the bar).
    Activate(usize),
    Wheel(mouse::ScrollDelta),
}

struct Bar {
    desktop: Desktop<ExtWorkspaceHandleV1>,
    switcher: Option<Switcher>,
    clock: String,
    /// None with no battery, or `--no-battery`.
    battery: Option<Battery>,
    /// A touchpad's scroll not yet a workspace's worth.
    scrolled: f32,
    /// Along the bottom edge: the line facing the windows goes on top.
    bottom: bool,
    /// The bar's height, and a workspace's square block.
    height: u32,
    /// The surfaces open, by output.
    surfaces: Vec<(WlOutput, window::Id)>,
}

/// The text size for a bar `height` high: `style::TEXT_SIZE` at the default
/// height, and as much bigger or smaller as the bar is.
pub fn text_size(height: u32) -> f32 {
    (style::TEXT_SIZE * height as f32 / args::DEFAULT_HEIGHT as f32).round()
}

pub fn run(args: Args) -> ExitCode {
    // One connection for iced_layershell and the desktop thread: the
    // outputs that thread binds are ones the layer shell can use.
    let connection = match Connection::connect_to_env() {
        Ok(connection) => connection,
        Err(err) => {
            eprintln!("pila-bar: can't connect to Wayland: {err}");
            return ExitCode::FAILURE;
        }
    };
    let Args {
        bottom,
        clock: format,
        height,
        outputs,
        battery: show_battery,
    } = args;
    let desktop_connection = connection.clone();
    let boot = move || {
        let bar = Bar {
            desktop: Desktop::default(),
            switcher: None,
            clock: String::new(),
            battery: None,
            scrolled: 0.0,
            bottom,
            height,
            surfaces: Vec::new(),
        };
        let updates = desktop::updates(desktop_connection.clone(), outputs.clone());
        let mut tasks = vec![Task::run(updates, Message::Update)];
        if !format.is_empty() {
            tasks.push(Task::run(clock::ticks(format.clone()), Message::Clock));
        }
        if show_battery {
            tasks.push(Task::run(battery::updates(), Message::Battery));
        }
        (bar, Task::batch(tasks))
    };
    let result = iced_layershell::daemon(boot, NAMESPACE, Bar::update, Bar::view)
        .subscription(Bar::subscription)
        .theme(|_: &Bar, _| Some(style::theme()))
        .style(|_: &Bar, theme: &Theme| iced::theme::Style {
            // Opaque: the software renderer's buffers have no alpha.
            background_color: theme.palette().background,
            text_color: theme.palette().text,
        })
        .settings(Settings {
            layer_settings: LayerShellSettings {
                // No surface until the desktop thread names an output.
                start_mode: StartMode::Background,
                ..Default::default()
            },
            default_font: style::FONT,
            default_text_size: text_size(height).into(),
            with_connection: Some(connection.into()),
            ..Default::default()
        })
        .run();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("pila-bar: {err}");
            ExitCode::FAILURE
        }
    }
}

impl Bar {
    fn subscription(&self) -> Subscription<Message> {
        event::listen_with(wheel)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Update(Update::Switcher(switcher)) => self.switcher = Some(switcher),
            Message::Update(Update::Desktop(desktop)) => self.desktop = desktop,
            Message::Update(Update::Output(Change::Open(output))) => return self.open(output),
            Message::Update(Update::Output(Change::Close(output))) => {
                if let Some(index) = self.surfaces.iter().position(|(o, _)| o == &output) {
                    let (_, id) = self.surfaces.remove(index);
                    return Task::done(Message::RemoveWindow(id));
                }
            }
            Message::Clock(clock) => self.clock = clock,
            Message::Battery(battery) => self.battery = Some(battery),
            Message::Activate(index) => self.activate(index),
            Message::Wheel(delta) => {
                let (steps, rest) = wheel_steps(self.scrolled, delta);
                self.scrolled = rest;
                if steps != 0 {
                    let active = self.desktop.workspaces.iter().position(|w| w.active);
                    let last = self.desktop.workspaces.len().saturating_sub(1);
                    if let Some(active) = active {
                        let next = active.saturating_add_signed(steps).min(last);
                        if next != active {
                            self.activate(next);
                        }
                    }
                }
            }
            // The layer shell's own messages are handled before `update`.
            _ => {}
        }
        Task::none()
    }

    /// A surface on `output`, along its edge.
    fn open(&mut self, output: WlOutput) -> Task<Message> {
        let edge = if self.bottom {
            Anchor::Bottom
        } else {
            Anchor::Top
        };
        let (id, open) = Message::layershell_open(NewLayerShellSettings {
            // The width of the output.
            size: Some((0, self.height)),
            layer: Layer::Top,
            anchor: edge | Anchor::Left | Anchor::Right,
            exclusive_zone: Some(self.height as i32),
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::None,
            output_option: OutputOption::Output(output.clone()),
            events_transparent: false,
            namespace: Some(NAMESPACE.to_string()),
        });
        self.surfaces.push((output, id));
        open
    }

    fn activate(&self, index: usize) {
        if let (Some(switcher), Some(workspace)) =
            (&self.switcher, self.desktop.workspaces.get(index))
        {
            switcher.activate(&workspace.key);
        }
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let workspaces = Row::with_children(
            self.desktop
                .workspaces
                .iter()
                .enumerate()
                .map(|(index, w)| button(index, &w.name, w.active, w.urgent, self.height)),
        )
        .height(Length::Fill);
        let left = container(workspaces)
            .width(Length::Fill)
            .height(Length::Fill)
            .clip(true);
        let clock = container(
            text(&self.clock)
                .wrapping(text::Wrapping::None)
                .color(style::TEXT_BRIGHT),
        )
        .padding([0, 12])
        .height(Length::Fill)
        .align_x(Alignment::End)
        .align_y(Alignment::Center)
        .clip(true);
        let battery = self.battery.map(|battery| {
            let low = battery.low();
            container(
                text(battery.text())
                    .wrapping(text::Wrapping::None)
                    .color(if low { style::TEXT_BRIGHT } else { style::TEXT }),
            )
            .padding([0, 10])
            .height(Length::Fill)
            .align_y(Alignment::Center)
            .style(move |_| match low {
                true => container::Style::default().background(style::URGENT),
                false => container::Style::default(),
            })
        });
        // The workspaces on the left; the battery and the clock on the right.
        let mut bar = row![left].height(Length::Fill);
        if let Some(battery) = battery {
            bar = bar.push(battery);
        }
        let bar = bar.push(clock);
        let edge = container(Space::new())
            .width(Length::Fill)
            .height(EDGE)
            .style(|_| container::Style::default().background(style::EDGE));
        let content = if self.bottom {
            column![edge, bar]
        } else {
            column![bar, edge]
        };
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style::default().background(style::SURFACE_DIM))
            .into()
    }
}

/// A workspace's button, a square block: the active one in the accent
/// color, an urgent one in the attention color, the others plain until
/// hovered.
fn button<'a>(
    index: usize,
    name: &'a str,
    active: bool,
    urgent: bool,
    height: u32,
) -> Element<'a, Message> {
    let label = text(name).wrapping(text::Wrapping::None);
    let block = button_widget(
        container(label)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
    )
    .on_press(Message::Activate(index))
    .height(Length::Fill);
    let block = if name.chars().count() <= 2 {
        block.width(height as f32).padding(0)
    } else {
        block.padding([0, 10])
    };
    block
        .style(move |_, status| {
            let hovered = matches!(
                status,
                button_widget::Status::Hovered | button_widget::Status::Pressed
            );
            let (background, text_color) = if active {
                (Some(style::ACCENT), style::TEXT_BRIGHT)
            } else if urgent {
                (Some(style::URGENT), style::TEXT_BRIGHT)
            } else if hovered {
                (Some(style::HOVER), style::TEXT_BRIGHT)
            } else {
                (None, style::TEXT_DIM)
            };
            button_widget::Style {
                background: background.map(Into::into),
                text_color,
                // Square corners.
                border: Border::default(),
                ..Default::default()
            }
        })
        .into()
}

fn wheel(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::WheelScrolled { delta }) => Some(Message::Wheel(delta)),
        _ => None,
    }
}

/// How many workspaces a scroll moves (positive: to the next ones), and
/// what's left of a touchpad's to add to the next: a wheel's notch is one, a
/// touchpad's `PIXELS_PER_STEP` pixels.
fn wheel_steps(scrolled: f32, delta: mouse::ScrollDelta) -> (isize, f32) {
    match delta {
        mouse::ScrollDelta::Lines { x, y } => {
            // Down or right: the next workspace.
            let by = if y != 0.0 { -y } else { x };
            (by.signum() as isize * (by != 0.0) as isize, 0.0)
        }
        mouse::ScrollDelta::Pixels { x, y } => {
            let by = if y != 0.0 { -y } else { x };
            let total = scrolled + by;
            let steps = (total / PIXELS_PER_STEP).trunc();
            (steps as isize, total - steps * PIXELS_PER_STEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mouse::ScrollDelta::{Lines, Pixels};

    #[test]
    fn a_notch_is_a_workspace() {
        assert_eq!(wheel_steps(0.0, Lines { x: 0.0, y: -1.0 }), (1, 0.0));
        assert_eq!(wheel_steps(0.0, Lines { x: 0.0, y: 3.0 }), (-1, 0.0));
        assert_eq!(wheel_steps(0.0, Lines { x: 1.0, y: 0.0 }), (1, 0.0));
        assert_eq!(wheel_steps(0.0, Lines { x: 0.0, y: 0.0 }), (0, 0.0));
        // A notch drops what a touchpad left.
        assert_eq!(wheel_steps(30.0, Lines { x: 0.0, y: -1.0 }), (1, 0.0));
    }

    #[test]
    fn a_touchpad_adds_up() {
        let (steps, rest) = wheel_steps(0.0, Pixels { x: 0.0, y: -30.0 });
        assert_eq!((steps, rest), (0, 30.0));
        let (steps, rest) = wheel_steps(rest, Pixels { x: 0.0, y: -30.0 });
        assert_eq!((steps, rest), (1, 20.0));
        assert_eq!(wheel_steps(0.0, Pixels { x: 0.0, y: 100.0 }), (-2, -20.0));
        // Back the other way undoes what was left.
        assert_eq!(wheel_steps(20.0, Pixels { x: 0.0, y: 20.0 }), (0, 0.0));
    }
}
