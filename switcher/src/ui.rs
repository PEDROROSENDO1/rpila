//! The switcher's window: the windows of the switch in its order, as tiles
//! in rows of `COLUMNS`, each a still of the window (`previews.rs`) over its
//! app's icon and its title, the selected one in the accent color, as a layer
//! surface (an Overlay the compositor centers, on the monitor the pointer is
//! on, taking no keyboard: the keys stay Pila's).
//!
//! An iced_layershell daemon: no surface while no switch goes on, a new one
//! for each switch still going on `DELAY` after it started (so a quick
//! Alt+Tab, there and back, shows nothing), removed when it ends. The icons
//! and the previews are found on threads at each switch's start; a window
//! whose preview isn't there (yet) shows its icon large in its place.

use std::{collections::HashMap, path::PathBuf, process::ExitCode, sync::Arc, time::Duration};

use iced::{
    mouse,
    widget::{column, container, image, mouse_area, row, svg, text, Space},
    window, Alignment, Border, ContentFit, Element, Length, Subscription, Task, Theme,
};
use iced_layershell::{
    reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption},
    settings::{LayerShellSettings, Settings, StartMode},
    to_layer_message,
};
use serde_json::Value;

use crate::{
    apps, icons,
    icons::IconFile,
    matching,
    model::{self, Change, Model, Row},
    pila_ipc, previews,
    style::{self, Style},
};

/// The layer surface's namespace, for `[[layer-rule]]`s.
pub const NAMESPACE: &str = "pila-switcher";

/// How long a switch goes on before it's shown.
const DELAY: Duration = Duration::from_millis(150);
/// Tiles in a row, and rows shown at once (the selected one's kept in view).
const COLUMNS: usize = 5;
const ROWS: usize = 3;
/// A tile: its padding around a preview, the preview, and the title under it.
const TILE_PADDING: f32 = 8.0;
const PREVIEW: (f32, f32) = (184.0, 115.0);
const CAPTION: f32 = 20.0;
const CAPTION_GAP: f32 = 6.0;
const TILE_WIDTH: f32 = PREVIEW.0 + 2.0 * TILE_PADDING;
const TILE_HEIGHT: f32 = PREVIEW.1 + CAPTION_GAP + CAPTION + 2.0 * TILE_PADDING;
/// Between tiles, and around them all.
const SPACING: f32 = 4.0;
const PADDING: f32 = 12.0;
const ICON_SIZE: f32 = 16.0;
/// A window's icon where its preview isn't.
const BIG_ICON_SIZE: f32 = 64.0;

const CLIENT: &str = "pila-switcher";

#[derive(Debug, Clone)]
pub enum Icon {
    Image(image::Handle),
    Svg(svg::Handle),
}

/// What a window's app shows: its icon and its name, from its desktop entry.
#[derive(Debug, Clone, Default)]
pub struct AppLook {
    icon: Option<Icon>,
    name: Option<String>,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    /// An event of Pila's IPC.
    Pila(Value),
    /// The delay of switch number `.0` is over.
    Due(u64),
    /// The icons and names of the windows' apps, by app id.
    Apps(Arc<HashMap<String, AppLook>>),
    /// A window's preview, for switch number `.0`.
    Preview(u64, String, image::Handle),
    /// A click on a window, by its id.
    Pick(String),
    Closed(window::Id),
}

struct Switcher {
    model: Model,
    window: Option<window::Id>,
    /// The size the surface was given.
    shown_size: (u32, u32),
    apps: Arc<HashMap<String, AppLook>>,
    /// This switch's previews, by window id.
    previews: HashMap<String, image::Handle>,
    style: Style,
}

pub fn run() -> ExitCode {
    let boot = || {
        let switcher = Switcher {
            model: Model::default(),
            window: None,
            shown_size: (0, 0),
            apps: Arc::new(HashMap::new()),
            previews: HashMap::new(),
            style: Style::default(),
        };
        let events = pila_ipc::events(CLIENT, &["colors", "windows", "switch"]);
        (switcher, Task::run(events, Message::Pila))
    };
    let result = iced_layershell::daemon(boot, NAMESPACE, Switcher::update, Switcher::view)
        .subscription(Switcher::subscription)
        .theme(|switcher: &Switcher, _| Some(switcher.style.theme()))
        .style(|_: &Switcher, theme: &Theme| iced::theme::Style {
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

impl Switcher {
    fn subscription(&self) -> Subscription<Message> {
        window::close_events().map(Message::Closed)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pila(event) => {
                if event["event"] == "colors" {
                    self.style = Style::from_pila(&event["colors"]);
                    return Task::none();
                }
                match self.model.apply(&event) {
                    Change::Started(started) => self.start(started),
                    Change::Updated => self.resize(),
                    Change::Ended => {
                        self.previews.clear();
                        self.hide()
                    }
                    Change::Nothing => Task::none(),
                }
            }
            Message::Due(started) if self.model.is_due(started) && self.window.is_none() => {
                self.show()
            }
            Message::Due(_) => Task::none(),
            Message::Apps(apps) => {
                self.apps = apps;
                Task::none()
            }
            Message::Preview(started, id, handle) => {
                if self.model.is_due(started) {
                    self.previews.insert(id, handle);
                }
                Task::none()
            }
            Message::Pick(id) => {
                // Pila ends the switch on it, and the surface goes with it.
                pila_ipc::focus(CLIENT, id);
                Task::none()
            }
            Message::Closed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                }
                Task::none()
            }
            // The layer shell's own messages are handled before `update`.
            _ => Task::none(),
        }
    }

    /// A switch started: its delay, and the icons and previews looked for
    /// meanwhile (the selected window's preview first).
    fn start(&mut self, started: u64) -> Task<Message> {
        self.previews.clear();
        let rows = self.model.rows();
        let mut ids: Vec<String> = rows
            .iter()
            .filter(|r| r.selected)
            .map(|r| r.id.clone())
            .collect();
        ids.extend(rows.iter().filter(|r| !r.selected).map(|r| r.id.clone()));
        Task::batch([
            Task::perform(after(DELAY), move |()| Message::Due(started)),
            Task::perform(find_apps(self.model.app_ids()), |apps| {
                Message::Apps(Arc::new(apps))
            }),
            Task::run(previews::capture(ids), move |(id, handle)| {
                Message::Preview(started, id, handle)
            }),
        ])
    }

    fn show(&mut self) -> Task<Message> {
        self.shown_size = size(self.model.rows().len());
        let (id, open) = Message::layershell_open(NewLayerShellSettings {
            size: Some(self.shown_size),
            layer: Layer::Overlay,
            // No edge: the compositor centers it.
            anchor: Anchor::empty(),
            exclusive_zone: None,
            margin: None,
            // The keys are Pila's: it does the switching.
            keyboard_interactivity: KeyboardInteractivity::None,
            // The compositor's choice: Pila's is the monitor the pointer is on,
            // as the switch's `output`.
            output_option: OutputOption::Active,
            events_transparent: false,
            namespace: Some(NAMESPACE.to_string()),
        });
        self.window = Some(id);
        open
    }

    /// A window of the switch went away: the surface fits the tiles left.
    fn resize(&mut self) -> Task<Message> {
        let Some(id) = self.window else {
            return Task::none();
        };
        let new = size(self.model.rows().len());
        if new == self.shown_size {
            return Task::none();
        }
        self.shown_size = new;
        Task::done(Message::SizeChange { id, size: new })
    }

    fn hide(&mut self) -> Task<Message> {
        match self.window.take() {
            Some(id) => Task::done(Message::RemoveWindow(id)),
            None => Task::none(),
        }
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let st = self.style;
        let tiles = self.model.rows();
        let selected = tiles.iter().position(|r| r.selected);
        let row_count = tiles.len().div_ceil(COLUMNS);
        let first = model::first_shown(row_count, ROWS, selected.map(|s| s / COLUMNS));
        let mut grid = column![].spacing(SPACING);
        let mut tiles = tiles
            .into_iter()
            .skip(first * COLUMNS)
            .take(ROWS * COLUMNS)
            .peekable();
        while tiles.peek().is_some() {
            let mut line = row![].spacing(SPACING);
            for tile in tiles.by_ref().take(COLUMNS) {
                line = line.push(self.tile(tile));
            }
            grid = grid.push(line);
        }
        container(grid.padding(PADDING))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(st.surface_dim.into()),
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

    fn tile(&self, tile: Row) -> Element<'_, Message> {
        let st = self.style;
        let selected = tile.selected;
        let app = self.apps.get(&tile.window.app_id);
        let icon = |size: f32| -> Element<'_, Message> {
            match app.and_then(|a| a.icon.as_ref()) {
                Some(Icon::Image(handle)) => image(handle.clone()).width(size).height(size).into(),
                Some(Icon::Svg(handle)) => svg(handle.clone()).width(size).height(size).into(),
                None => Space::new().width(size).height(size).into(),
            }
        };
        let preview: Element<'_, Message> = match self.previews.get(&tile.id) {
            Some(handle) => image(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(ContentFit::Contain)
                .into(),
            None => icon(BIG_ICON_SIZE),
        };
        let preview = container(preview)
            .width(PREVIEW.0)
            .height(PREVIEW.1)
            .center_x(PREVIEW.0)
            .center_y(PREVIEW.1);
        // A window with no title is named by its app.
        let title = if tile.window.title.is_empty() {
            app.and_then(|a| a.name.clone())
                .unwrap_or_else(|| tile.window.app_id.clone())
        } else {
            tile.window.title.clone()
        };
        let caption = row![
            icon(ICON_SIZE),
            container(text(title).wrapping(text::Wrapping::None))
                .width(Length::Fill)
                .clip(true),
        ]
        .spacing(6)
        .height(CAPTION)
        .align_y(Alignment::Center);
        let item = container(column![preview, caption].spacing(CAPTION_GAP))
            .padding(TILE_PADDING)
            .width(TILE_WIDTH)
            .height(TILE_HEIGHT)
            .style(move |_| {
                if selected {
                    // As the bar's active workspace, square.
                    container::Style {
                        background: Some(st.accent.into()),
                        text_color: Some(st.text_bright),
                        ..Default::default()
                    }
                } else {
                    container::Style {
                        text_color: Some(st.text),
                        ..Default::default()
                    }
                }
            });
        mouse_area(item)
            .on_press(Message::Pick(tile.id))
            .interaction(mouse::Interaction::Pointer)
            .into()
    }
}

/// The surface's size for `tiles` tiles: rows of `COLUMNS`, at most `ROWS`.
fn size(tiles: usize) -> (u32, u32) {
    let columns = tiles.clamp(1, COLUMNS);
    let rows = tiles.div_ceil(columns).clamp(1, ROWS);
    let across = |n: usize, tile: f32| 2.0 * PADDING + n as f32 * tile + (n - 1) as f32 * SPACING;
    (
        across(columns, TILE_WIDTH).ceil() as u32,
        across(rows, TILE_HEIGHT).ceil() as u32,
    )
}

/// Ready after `delay`, waited on a thread (iced here has no timers).
async fn after(delay: Duration) {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name(format!("{CLIENT}-delay"))
        .spawn(move || {
            std::thread::sleep(delay);
            let _ = sender.send(());
        });
    if spawned.is_ok() {
        let _ = receiver.await;
    }
}

/// The icons and names of the apps of `app_ids`, found off the UI thread.
async fn find_apps(app_ids: Vec<String>) -> HashMap<String, AppLook> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name(format!("{CLIENT}-icons"))
        .spawn(move || {
            let _ = sender.send(find_apps_now(&app_ids));
        });
    match spawned {
        Ok(_) => receiver.await.unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

fn find_apps_now(app_ids: &[String]) -> HashMap<String, AppLook> {
    let var = |name| std::env::var(name).ok();
    let dirs: Vec<PathBuf> = apps::data_dirs(
        var("XDG_DATA_HOME").as_deref(),
        var("HOME").as_deref(),
        var("XDG_DATA_DIRS").as_deref(),
    );
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
    // Its windows are open: every program is taken as there.
    let session = apps::Session {
        desktops: &desktops,
        locales: &locales,
        found: &|_| true,
    };
    let entries = apps::scan(&dirs, &session);
    app_ids
        .iter()
        .map(|app_id| {
            let entry = matching::app_for(&entries, app_id);
            // The entry's icon, else one named as the app id.
            let icon_name = entry
                .and_then(|app| app.icon.clone())
                .unwrap_or_else(|| app_id.clone());
            let icon = icons::find(&icon_name, &dirs).map(|file| match file {
                IconFile::Png(path) => Icon::Image(image::Handle::from_path(path)),
                IconFile::Svg(path) => Icon::Svg(svg::Handle::from_path(path)),
            });
            let name = entry.map(|app| app.name.clone());
            (app_id.clone(), AppLook { icon, name })
        })
        .collect()
}
