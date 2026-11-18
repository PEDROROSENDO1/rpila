//! The launcher's window: a search field over the installed apps, as a layer
//! surface (an Overlay the compositor centers, taking the keyboard).
//!
//! An iced_layershell daemon: no surface while hidden, a new one each time
//! it's shown. Requests from later runs (`main.rs`) come in through a
//! stream from the socket's thread; the apps are read again on a thread each
//! time it's shown, so a newly installed one is there.

use std::{
    os::unix::net::UnixListener,
    path::PathBuf,
    process::{Command, ExitCode, Stdio},
    sync::{Arc, Mutex},
};

use iced::{
    event, keyboard, mouse,
    widget::{self, column, container, image, mouse_area, row, svg, text, text_input, Space},
    window, Alignment, Border, Element, Event, Length, Subscription, Task, Theme,
};
use iced_layershell::{
    reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption},
    settings::{LayerShellSettings, Settings, StartMode},
    to_layer_message,
};

use crate::{
    apps::{self, App},
    exec::{self, Request},
    icons::{self, IconFile},
};

/// The layer surface's namespace, for `[[layer-rule]]`s.
pub const NAMESPACE: &str = "pila-launcher";

const WIDTH: f32 = 480.0;
const ROWS: usize = 8;
const ROW_HEIGHT: f32 = 36.0;
const ICON_SIZE: f32 = 24.0;
const PADDING: f32 = 12.0;
const SPACING: f32 = 8.0;
const INPUT_HEIGHT: f32 = 40.0;
const HEIGHT: f32 = 2.0 * PADDING + INPUT_HEIGHT + SPACING + ROWS as f32 * ROW_HEIGHT;

const QUERY: widget::Id = widget::Id::new("query");

/// An app and its icon, ready to draw.
#[derive(Debug, Clone)]
pub struct Entry {
    app: App,
    icon: Option<Icon>,
}

#[derive(Debug, Clone)]
enum Icon {
    Image(image::Handle),
    Svg(svg::Handle),
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    /// From a later run: show or hide.
    Request(Request),
    Scanned(Arc<Vec<Entry>>),
    Opened(window::Id),
    Closed(window::Id),
    Query(String),
    /// Return in the search field.
    Submit,
    /// A click on a row (its place in the list shown).
    Launch(usize),
    /// Arrows, Tab, Page Up/Down: move the selection by this many rows.
    Move(isize),
    /// The wheel: scroll by this many rows.
    Scroll(isize),
    Hide,
}

struct Launcher {
    /// For `Terminal=true` apps; the latest run's wins.
    terminal: Option<String>,
    window: Option<window::Id>,
    entries: Arc<Vec<Entry>>,
    query: String,
    /// What the query matches, best first, as indices into `entries`.
    shown: Vec<usize>,
    /// The selected row and the first one in view, in `shown`.
    selected: usize,
    first: usize,
}

pub fn run(listener: UnixListener, first: Request) -> ExitCode {
    // The boot function may run only once, but must be `Fn`.
    let listener = Mutex::new(Some(listener));
    let boot = move || {
        let mut launcher = Launcher {
            terminal: None,
            window: None,
            entries: Arc::new(Vec::new()),
            query: String::new(),
            shown: Vec::new(),
            selected: 0,
            first: 0,
        };
        let show = launcher.request(first.clone());
        let requests = match listener.lock().ok().and_then(|mut l| l.take()) {
            Some(listener) => Task::run(requests(listener), Message::Request),
            None => Task::none(),
        };
        (launcher, Task::batch([show, requests]))
    };
    let result = iced_layershell::daemon(boot, NAMESPACE, Launcher::update, Launcher::view)
        .subscription(Launcher::subscription)
        .theme(|_: &Launcher, _| Some(Theme::Dark))
        .style(|_: &Launcher, theme: &Theme| iced::theme::Style {
            // Opaque: the software renderer's buffers have no alpha.
            background_color: theme.palette().background,
            text_color: theme.palette().text,
        })
        .settings(Settings {
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background,
                ..Default::default()
            },
            ..Default::default()
        })
        .run();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("pila-launcher: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The requests of later runs, read on a thread of their own.
fn requests(listener: UnixListener) -> impl iced::futures::Stream<Item = Request> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("pila-launcher-socket".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Some(request) = crate::read_request(stream) {
                    if sender.unbounded_send(request).is_err() {
                        break;
                    }
                }
            }
        });
    if let Err(err) = spawned {
        eprintln!("pila-launcher: can't listen for later runs: {err}");
    }
    receiver
}

impl Launcher {
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            event::listen_with(key_or_wheel),
            window::open_events().map(Message::Opened),
            window::close_events().map(Message::Closed),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Request(request) => self.request(request),
            Message::Scanned(entries) => {
                self.entries = entries;
                self.search();
                Task::none()
            }
            Message::Opened(id) if self.window == Some(id) => widget::operation::focus(QUERY),
            Message::Opened(_) => Task::none(),
            Message::Closed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                }
                Task::none()
            }
            Message::Query(query) => {
                self.query = query;
                self.search();
                Task::none()
            }
            Message::Submit => self.launch(self.selected),
            Message::Launch(row) => self.launch(row),
            Message::Move(by) => {
                if !self.shown.is_empty() {
                    let last = self.shown.len() - 1;
                    self.selected = self.selected.saturating_add_signed(by).min(last);
                    if self.selected < self.first {
                        self.first = self.selected;
                    } else if self.selected >= self.first + ROWS {
                        self.first = self.selected + 1 - ROWS;
                    }
                }
                Task::none()
            }
            Message::Scroll(by) => {
                let max_first = self.shown.len().saturating_sub(ROWS);
                self.first = self.first.saturating_add_signed(by).min(max_first);
                self.selected = self.selected.clamp(self.first, self.first + ROWS - 1);
                Task::none()
            }
            Message::Hide => self.hide(),
            // The layer shell's own messages are handled before `update`.
            _ => Task::none(),
        }
    }

    fn request(&mut self, request: Request) -> Task<Message> {
        if request.terminal.is_some() {
            self.terminal = request.terminal;
        }
        if !request.toggle {
            return Task::none();
        }
        if self.window.is_some() {
            return self.hide();
        }
        self.query.clear();
        self.search();
        let (id, open) = Message::layershell_open(NewLayerShellSettings {
            size: Some((WIDTH as u32, HEIGHT as u32)),
            layer: Layer::Overlay,
            // No edge: the compositor centers it.
            anchor: Anchor::empty(),
            exclusive_zone: None,
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            output_option: OutputOption::Active,
            events_transparent: false,
            namespace: Some(NAMESPACE.to_string()),
        });
        self.window = Some(id);
        let scan = Task::perform(scan(), |entries| Message::Scanned(Arc::new(entries)));
        Task::batch([open, scan])
    }

    fn hide(&mut self) -> Task<Message> {
        match self.window.take() {
            Some(id) => Task::done(Message::RemoveWindow(id)),
            None => Task::none(),
        }
    }

    fn search(&mut self) {
        self.shown = apps::rank(self.entries.iter().map(|e| &e.app), &self.query);
        self.selected = 0;
        self.first = 0;
    }

    fn launch(&mut self, row: usize) -> Task<Message> {
        if let Some(entry) = self.shown.get(row).and_then(|&i| self.entries.get(i)) {
            start(&entry.app, self.terminal.as_deref());
        }
        self.hide()
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let input = text_input("Search apps", &self.query)
            .id(QUERY)
            .on_input(Message::Query)
            .on_submit(Message::Submit)
            .size(18)
            .padding(8);
        let mut rows = column![].spacing(0);
        let end = (self.first + ROWS).min(self.shown.len());
        for row_index in self.first..end {
            let entry = &self.entries[self.shown[row_index]];
            rows = rows.push(self.row(entry, row_index));
        }
        if self.shown.is_empty() && !self.entries.is_empty() {
            rows = rows.push(
                container(text("No app matches").size(15))
                    .padding([8, 12])
                    .style(|theme: &Theme| container::Style {
                        text_color: Some(theme.extended_palette().background.strong.text),
                        ..Default::default()
                    }),
            );
        }
        let content = column![input, rows.height(Length::Fill)]
            .spacing(SPACING)
            .padding(PADDING);
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style {
                    background: Some(palette.background.base.color.into()),
                    border: Border {
                        color: palette.background.strong.color,
                        width: 1.0,
                        // Square: transparent corners would show black.
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                }
            })
            .into()
    }

    fn row<'a>(&self, entry: &'a Entry, row_index: usize) -> Element<'a, Message> {
        let icon: Element<'a, Message> = match &entry.icon {
            Some(Icon::Image(handle)) => image(handle.clone())
                .width(ICON_SIZE)
                .height(ICON_SIZE)
                .into(),
            Some(Icon::Svg(handle)) => svg(handle.clone())
                .width(ICON_SIZE)
                .height(ICON_SIZE)
                .into(),
            None => Space::new().width(ICON_SIZE).height(ICON_SIZE).into(),
        };
        let selected = row_index == self.selected;
        let line = row![icon, text(&entry.app.name).size(15)]
            .spacing(12)
            .align_y(Alignment::Center);
        let item = container(line)
            .padding([0, 10])
            .width(Length::Fill)
            .height(ROW_HEIGHT)
            .align_y(Alignment::Center)
            .style(move |theme: &Theme| {
                let palette = theme.extended_palette();
                if selected {
                    container::Style {
                        background: Some(palette.primary.weak.color.into()),
                        text_color: Some(palette.primary.weak.text),
                        border: Border {
                            radius: 8.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                } else {
                    container::Style::default()
                }
            });
        mouse_area(item)
            .on_press(Message::Launch(row_index))
            .interaction(mouse::Interaction::Pointer)
            .into()
    }
}

/// The keys the list takes (the search field takes the rest) and the wheel.
fn key_or_wheel(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    use keyboard::key::Named;
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(named),
            modifiers,
            ..
        }) => match named {
            Named::Escape => Some(Message::Hide),
            Named::ArrowDown => Some(Message::Move(1)),
            Named::ArrowUp => Some(Message::Move(-1)),
            Named::Tab if modifiers.shift() => Some(Message::Move(-1)),
            Named::Tab => Some(Message::Move(1)),
            Named::PageDown => Some(Message::Move(ROWS as isize)),
            Named::PageUp => Some(Message::Move(-(ROWS as isize))),
            _ => None,
        },
        Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
            let y = match delta {
                mouse::ScrollDelta::Lines { y, .. } => y,
                mouse::ScrollDelta::Pixels { y, .. } => y / ROW_HEIGHT,
            };
            let rows = -y.round() as isize;
            (rows != 0).then_some(Message::Scroll(rows))
        }
        _ => None,
    }
}

/// The apps and their icons, read off the UI thread.
async fn scan() -> Vec<Entry> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("pila-launcher-scan".into())
        .spawn(move || {
            let _ = sender.send(scan_now());
        });
    if spawned.is_err() {
        return scan_now();
    }
    receiver.await.unwrap_or_default()
}

fn scan_now() -> Vec<Entry> {
    let var = |name| std::env::var(name).ok();
    let dirs = data_dirs();
    let desktops: Vec<String> = var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .collect();
    let locale = var("LC_ALL")
        .filter(|l| !l.is_empty())
        .or_else(|| var("LC_MESSAGES").filter(|l| !l.is_empty()))
        .or_else(|| var("LANG"));
    let locales = apps::locale_keys(locale.as_deref());
    let path = var("PATH");
    let found = |program: &str| apps::found_in(program, path.as_deref());
    let session = apps::Session {
        desktops: &desktops,
        locales: &locales,
        found: &found,
    };
    apps::scan(&dirs, &session)
        .into_iter()
        .map(|app| {
            let icon = app
                .icon
                .as_deref()
                .and_then(|icon| icons::find(icon, &dirs))
                .map(|file| match file {
                    IconFile::Png(path) => Icon::Image(image::Handle::from_path(path)),
                    IconFile::Svg(path) => Icon::Svg(svg::Handle::from_path(path)),
                });
            Entry { app, icon }
        })
        .collect()
}

fn data_dirs() -> Vec<PathBuf> {
    let var = |name| std::env::var(name).ok();
    apps::data_dirs(
        var("XDG_DATA_HOME").as_deref(),
        var("HOME").as_deref(),
        var("XDG_DATA_DIRS").as_deref(),
    )
}

/// Start `app` as Pila starts a command (`/bin/sh -c`, its output where the
/// launcher's goes), reaped on a thread when it ends.
fn start(app: &App, terminal: Option<&str>) {
    let command = if app.terminal {
        let from_env = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty());
        match terminal.map(str::to_string).or(from_env) {
            Some(terminal) => exec::in_terminal(&terminal, &app.exec),
            None => {
                eprintln!(
                    "pila-launcher: {} runs in a terminal: pass --terminal or set TERMINAL",
                    app.name
                );
                return;
            }
        }
    } else {
        exec::clean_exec(&app.exec)
    };
    let mut process = Command::new("/bin/sh");
    process.arg("-c").arg(&command).stdin(Stdio::null());
    let dir = app
        .path
        .clone()
        .or_else(|| std::env::var("HOME").ok())
        .filter(|d| std::path::Path::new(d).is_dir());
    if let Some(dir) = dir {
        process.current_dir(dir);
    }
    match process.spawn() {
        Ok(mut child) => {
            let reaped = std::thread::Builder::new()
                .name("pila-launcher-child".into())
                .spawn(move || child.wait());
            if let Err(err) = reaped {
                eprintln!("pila-launcher: can't wait for {}: {err}", app.name);
            }
        }
        Err(err) => eprintln!("pila-launcher: can't start {}: {err}", app.name),
    }
}
