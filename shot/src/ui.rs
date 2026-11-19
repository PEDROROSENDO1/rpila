//! Picking what to take: each monitor covered by a layer surface (an
//! Overlay anchored to every edge, over bars too, taking the keyboard)
//! showing its picture as it was when pila-shot started, dimmed. For a
//! region, a drag on one of them selects a rectangle, shown undimmed in a
//! square accent-colored frame; for the monitor to share, a click picks the
//! one under the pointer, which is shown undimmed. Escape or a right click
//! cancels.
//!
//! An iced_layershell daemon that opens its surfaces at once and exits when
//! something is picked or cancelled (`Outcome`, read by `main.rs` after).

use std::sync::{Arc, Mutex};

use iced::{
    keyboard, mouse,
    widget::{
        canvas::{self, Action, Frame, Geometry, Stroke},
        image, stack,
    },
    window, Color, ContentFit, Element, Length, Point, Rectangle, Renderer, Size, Subscription,
    Task, Theme,
};
use iced_layershell::{
    reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption},
    settings::{LayerShellSettings, Settings, StartMode},
    to_layer_message,
};

use crate::{
    capture::Shot,
    pila_ipc,
    style::{self, Style},
};

/// The layer surfaces' namespace, for `[[layer-rule]]`s.
pub const NAMESPACE: &str = "pila-shot";
const CLIENT: &str = "pila-shot";

/// How dark the parts not picked are.
const DIM: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.45);
/// The selection's frame, and the picked monitor's.
const FRAME: f32 = 2.0;
/// A drag shorter than this either way is a click, not a selection.
const LEAST: f32 = 3.0;

/// What's being picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Region,
    Output,
}

/// What was picked, by the index of its monitor in the shots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    /// A region, as fractions `(x, y, width, height)` of its monitor.
    Region(usize, (f32, f32, f32, f32)),
    Output(usize),
    Cancelled,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Picked(Outcome),
    Key(keyboard::Event),
    Style(Style),
}

struct Picker {
    mode: Mode,
    /// Each monitor's picture and the surface showing it.
    shown: Vec<(image::Handle, Option<window::Id>)>,
    outcome: Arc<Mutex<Option<Outcome>>>,
    style: Style,
}

/// Show `shots` and let the user pick; `Cancelled` if anything goes wrong.
pub fn run(mode: Mode, shots: &[Shot]) -> Outcome {
    let outcome = Arc::new(Mutex::new(None));
    let handles: Vec<image::Handle> = shots
        .iter()
        .map(|shot| image::Handle::from_rgba(shot.width, shot.height, shot.rgba.clone()))
        .collect();
    let outputs: Vec<String> = shots.iter().map(|shot| shot.output.clone()).collect();
    let boot = {
        let outcome = outcome.clone();
        move || {
            let mut picker = Picker {
                mode,
                shown: handles.iter().map(|h| (h.clone(), None)).collect(),
                outcome: outcome.clone(),
                style: Style::default(),
            };
            let mut tasks = vec![Task::run(pila_ipc::styles(CLIENT), Message::Style)];
            for (index, output) in outputs.iter().enumerate() {
                let (id, open) = Message::layershell_open(NewLayerShellSettings {
                    // Anchored to every edge: the whole monitor.
                    size: Some((0, 0)),
                    layer: Layer::Overlay,
                    anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
                    // Over the bars' exclusive zones too.
                    exclusive_zone: Some(-1),
                    margin: None,
                    // For Escape.
                    keyboard_interactivity: KeyboardInteractivity::Exclusive,
                    output_option: OutputOption::OutputName(output.clone()),
                    events_transparent: false,
                    namespace: Some(NAMESPACE.to_string()),
                });
                picker.shown[index].1 = Some(id);
                tasks.push(open);
            }
            (picker, Task::batch(tasks))
        }
    };
    let result = iced_layershell::daemon(boot, NAMESPACE, Picker::update, Picker::view)
        .subscription(Picker::subscription)
        .theme(|picker: &Picker, _| Some(picker.style.theme()))
        .style(|_: &Picker, theme: &Theme| iced::theme::Style {
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
    if let Err(err) = result {
        eprintln!("{CLIENT}: {err}");
    }
    let picked = outcome.lock().ok().and_then(|o| *o);
    picked.unwrap_or(Outcome::Cancelled)
}

impl Picker {
    fn subscription(&self) -> Subscription<Message> {
        keyboard::listen().map(Message::Key)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Picked(outcome) => self.end(outcome),
            Message::Key(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => self.end(Outcome::Cancelled),
            Message::Key(_) => Task::none(),
            Message::Style(style) => {
                self.style = style;
                Task::none()
            }
            // The layer shell's own messages are handled before `update`.
            _ => Task::none(),
        }
    }

    /// The first outcome counts: the surfaces go, and so does the daemon.
    fn end(&mut self, outcome: Outcome) -> Task<Message> {
        if let Ok(mut kept) = self.outcome.lock() {
            kept.get_or_insert(outcome);
        }
        let mut tasks: Vec<Task<Message>> = self
            .shown
            .iter_mut()
            .filter_map(|(_, id)| id.take())
            .map(|id| Task::done(Message::RemoveWindow(id)))
            .collect();
        tasks.push(iced::exit());
        Task::batch(tasks)
    }

    fn view(&self, window: window::Id) -> Element<'_, Message> {
        let Some(index) = self.shown.iter().position(|(_, id)| *id == Some(window)) else {
            return iced::widget::Space::new().into();
        };
        let picture = image(self.shown[index].0.clone())
            .width(Length::Fill)
            .height(Length::Fill)
            .content_fit(ContentFit::Fill);
        let selection = canvas::Canvas::new(Selection {
            index,
            mode: self.mode,
            accent: self.style.accent,
        })
        .width(Length::Fill)
        .height(Length::Fill);
        stack![picture, selection].into()
    }
}

/// The selection over one monitor's picture.
struct Selection {
    index: usize,
    mode: Mode,
    accent: Color,
}

#[derive(Default)]
struct Drag {
    start: Option<Point>,
    end: Option<Point>,
    /// The pointer is over this monitor.
    over: bool,
}

impl canvas::Program<Message> for Selection {
    type State = Drag;

    fn update(
        &self,
        drag: &mut Drag,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        let canvas::Event::Mouse(event) = event else {
            return None;
        };
        // Where the pointer is on this surface, kept inside it.
        let here = cursor.position().map(|p| {
            Point::new(
                (p.x - bounds.x).clamp(0.0, bounds.width),
                (p.y - bounds.y).clamp(0.0, bounds.height),
            )
        });
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Right) => {
                Some(Action::publish(Message::Picked(Outcome::Cancelled)).and_capture())
            }
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                cursor.position_in(bounds)?;
                match self.mode {
                    Mode::Output => Some(
                        Action::publish(Message::Picked(Outcome::Output(self.index))).and_capture(),
                    ),
                    Mode::Region => {
                        drag.start = here;
                        drag.end = here;
                        Some(Action::request_redraw().and_capture())
                    }
                }
            }
            mouse::Event::CursorMoved { .. }
            | mouse::Event::CursorEntered
            | mouse::Event::CursorLeft => {
                let over = cursor.position_in(bounds).is_some();
                let moved = drag.start.is_some() && here.is_some() && here != drag.end;
                if moved {
                    drag.end = here;
                }
                let changed = moved || over != drag.over;
                drag.over = over;
                changed.then(Action::request_redraw)
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => {
                let (start, end) = (drag.start.take()?, drag.end.take().or(here)?);
                let rect = between(start, end);
                if rect.width < LEAST || rect.height < LEAST {
                    return Some(Action::request_redraw());
                }
                let fractions = (
                    rect.x / bounds.width,
                    rect.y / bounds.height,
                    rect.width / bounds.width,
                    rect.height / bounds.height,
                );
                Some(Action::publish(Message::Picked(Outcome::Region(
                    self.index, fractions,
                ))))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        drag: &Drag,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let whole = Rectangle::new(Point::ORIGIN, bounds.size());
        let stroke = Stroke::default().with_color(self.accent).with_width(FRAME);
        match self.mode {
            Mode::Region => match (drag.start, drag.end) {
                (Some(start), Some(end)) => {
                    let rect = between(start, end);
                    for part in around(whole, rect) {
                        frame.fill_rectangle(part.position(), part.size(), DIM);
                    }
                    frame.stroke_rectangle(rect.position(), rect.size(), stroke);
                }
                _ => frame.fill_rectangle(Point::ORIGIN, whole.size(), DIM),
            },
            Mode::Output if drag.over => {
                let inset = FRAME / 2.0;
                frame.stroke_rectangle(
                    Point::new(inset, inset),
                    Size::new(whole.width - FRAME, whole.height - FRAME),
                    stroke.with_width(2.0 * FRAME),
                );
            }
            Mode::Output => frame.fill_rectangle(Point::ORIGIN, whole.size(), DIM),
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _drag: &Drag,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match self.mode {
            Mode::Region => mouse::Interaction::Crosshair,
            Mode::Output if cursor.is_over(bounds) => mouse::Interaction::Pointer,
            Mode::Output => mouse::Interaction::default(),
        }
    }
}

/// The rectangle with `a` and `b` at opposite corners.
pub fn between(a: Point, b: Point) -> Rectangle {
    Rectangle::new(
        Point::new(a.x.min(b.x), a.y.min(b.y)),
        Size::new((a.x - b.x).abs(), (a.y - b.y).abs()),
    )
}

/// The parts of `whole` outside `inner` (inside it): above, below, left and
/// right of it.
pub fn around(whole: Rectangle, inner: Rectangle) -> [Rectangle; 4] {
    let bottom = inner.y + inner.height;
    let right = inner.x + inner.width;
    [
        Rectangle::new(Point::ORIGIN, Size::new(whole.width, inner.y)),
        Rectangle::new(
            Point::new(0.0, bottom),
            Size::new(whole.width, whole.height - bottom),
        ),
        Rectangle::new(Point::new(0.0, inner.y), Size::new(inner.x, inner.height)),
        Rectangle::new(
            Point::new(right, inner.y),
            Size::new(whole.width - right, inner.height),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_either_way_is_the_same_rectangle() {
        let r = between(Point::new(30.0, 40.0), Point::new(10.0, 15.0));
        assert_eq!(
            r,
            Rectangle::new(Point::new(10.0, 15.0), Size::new(20.0, 25.0))
        );
    }

    #[test]
    fn the_dimmed_parts_cover_all_but_the_selection() {
        let whole = Rectangle::new(Point::ORIGIN, Size::new(100.0, 50.0));
        let inner = Rectangle::new(Point::new(10.0, 20.0), Size::new(30.0, 5.0));
        let parts = around(whole, inner);
        let area: f32 = parts.iter().map(|p| p.width * p.height).sum();
        assert_eq!(
            area + inner.width * inner.height,
            whole.width * whole.height
        );
        assert!(parts.iter().all(|p| p
            .intersection(&inner)
            .is_none_or(|i| i.width * i.height == 0.0)));
    }
}
