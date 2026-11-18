//! The bar: a layer surface on every monitor (Top, along the top or bottom
//! edge, with an exclusive zone so windows keep clear of it), showing the
//! workspaces, the focused window's title and the clock.
//!
//! An iced_layershell daemon in `AllScreens` mode: it opens a surface on
//! each output there is and on each one that comes, and drops a surface
//! whose output goes. Every surface shows the same: Pila's workspaces span
//! all its outputs. The desktop (`desktop.rs`) and the clock (`clock.rs`)
//! come in as streams from threads of their own.

use std::process::ExitCode;

use iced::{
    event, mouse,
    widget::{container, mouse_area, row, text, Row},
    window, Alignment, Element, Event, Length, Subscription, Task, Theme,
};
use iced_layershell::{
    reexport::{Anchor, KeyboardInteractivity, Layer},
    settings::{LayerShellSettings, Settings, StartMode},
    to_layer_message,
};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1::ExtWorkspaceHandleV1;

use crate::{
    args::Args,
    clock,
    desktop::{self, Desktop, Switcher, Update},
};

/// The layer surfaces' namespace, for `[[layer-rule]]`s.
pub const NAMESPACE: &str = "pila-bar";

/// The bar's height, its exclusive zone.
pub const HEIGHT: u32 = 28;
/// A workspace's button, for names of one or two characters (longer ones
/// get what they need).
pub const BUTTON_WIDTH: f32 = 28.0;
const TEXT_SIZE: f32 = 13.0;
/// How far a touchpad scrolls (in pixels) for one workspace.
const PIXELS_PER_STEP: f32 = 40.0;

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Update(Update),
    Clock(String),
    /// A click on a workspace (its place in the bar).
    Activate(usize),
    Wheel(mouse::ScrollDelta),
}

struct Bar {
    desktop: Desktop<ExtWorkspaceHandleV1>,
    switcher: Option<Switcher>,
    clock: String,
    /// A touchpad's scroll not yet a workspace's worth.
    scrolled: f32,
}

pub fn run(args: Args) -> ExitCode {
    let format = args.clock.clone();
    let boot = move || {
        let bar = Bar {
            desktop: Desktop::default(),
            switcher: None,
            clock: String::new(),
            scrolled: 0.0,
        };
        let mut tasks = vec![Task::run(desktop::updates(), Message::Update)];
        if !format.is_empty() {
            tasks.push(Task::run(clock::ticks(format.clone()), Message::Clock));
        }
        (bar, Task::batch(tasks))
    };
    let edge = if args.bottom {
        Anchor::Bottom
    } else {
        Anchor::Top
    };
    let result = iced_layershell::daemon(boot, NAMESPACE, Bar::update, Bar::view)
        .subscription(Bar::subscription)
        .theme(|_: &Bar, _| Some(Theme::Dark))
        .style(|_: &Bar, theme: &Theme| iced::theme::Style {
            // Opaque: the software renderer's buffers have no alpha.
            background_color: theme.palette().background,
            text_color: theme.palette().text,
        })
        .settings(Settings {
            layer_settings: LayerShellSettings {
                anchor: edge | Anchor::Left | Anchor::Right,
                layer: Layer::Top,
                exclusive_zone: HEIGHT as i32,
                // The width of the output.
                size: Some((0, HEIGHT)),
                keyboard_interactivity: KeyboardInteractivity::None,
                start_mode: StartMode::AllScreens,
                ..Default::default()
            },
            default_text_size: TEXT_SIZE.into(),
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
            Message::Clock(clock) => self.clock = clock,
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
                .map(|(index, w)| button(index, &w.name, w.active, w.urgent)),
        );
        let title = container(
            text(self.desktop.title.as_deref().unwrap_or_default()).wrapping(text::Wrapping::None),
        )
        .padding([0, 12])
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::Center)
        .clip(true);
        let clock = container(text(&self.clock).wrapping(text::Wrapping::None))
            .padding([0, 12])
            .height(Length::Fill)
            .align_y(Alignment::Center);
        container(row![workspaces, title, clock].height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(theme.extended_palette().background.base.color.into()),
                ..Default::default()
            })
            .into()
    }
}

/// A workspace's button: the active one in the accent color, an urgent one
/// in the danger color.
fn button<'a>(index: usize, name: &'a str, active: bool, urgent: bool) -> Element<'a, Message> {
    let label = container(text(name).wrapping(text::Wrapping::None))
        .height(Length::Fill)
        .align_y(Alignment::Center);
    let label = if name.chars().count() <= 2 {
        label.width(BUTTON_WIDTH).align_x(Alignment::Center)
    } else {
        label.padding([0, 8])
    };
    let label = label.style(move |theme: &Theme| {
        let palette = theme.extended_palette();
        let pair = if active {
            Some(palette.primary.base)
        } else if urgent {
            Some(palette.danger.base)
        } else {
            None
        };
        match pair {
            Some(pair) => container::Style {
                background: Some(pair.color.into()),
                text_color: Some(pair.text),
                ..Default::default()
            },
            None => container::Style::default(),
        }
    });
    mouse_area(label)
        .on_press(Message::Activate(index))
        .interaction(mouse::Interaction::Pointer)
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
