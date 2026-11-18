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
    widget::{
        button as button_widget, column, container, image, mouse_area, row, stack, svg, text, Row,
        Space,
    },
    window, Alignment, Border, Color, Element, Event, Length, Subscription, Task, Theme,
};
use iced_layershell::{
    actions::IcedNewPopupSettings,
    reexport::{
        Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption, PopupAnchor,
        PopupGravity,
    },
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
    glyphs,
    outputs::Change,
    pila_ipc,
    style::{self, Style},
    tray::{self, menu, Click, Tray},
};

/// The layer surfaces' namespace, for `[[layer-rule]]`s.
pub const NAMESPACE: &str = "pila-bar";
/// The clock's tooltip's.
pub const TOOLTIP_NAMESPACE: &str = "pila-bar-tooltip";

/// The line along the edge that faces the windows.
const EDGE: f32 = 1.0;
/// How far a touchpad scrolls (in pixels) for one workspace.
const PIXELS_PER_STEP: f32 = 40.0;
/// A tray menu's rows, separators and border.
const MENU_ROW: f32 = 28.0;
const MENU_SEPARATOR: f32 = 9.0;
const MENU_BORDER: f32 = 1.0;
/// About as wide as a character of the menu's text, for its size.
const MENU_CHAR: f32 = 8.5;

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Update(Update),
    Clock(String),
    /// The clock's tooltip's text.
    Date(String),
    /// The pointer entered (true) or left the clock.
    ClockHover(bool),
    /// The pointer has been on the clock long enough for its tooltip (the
    /// hover it was asked for).
    TooltipDue(u64),
    Battery(Battery),
    /// The icon font installed, if any.
    IconFont(Option<glyphs::IconFont>),
    /// Pila's colors, new or changed.
    Style(Style),
    Tray(tray::Update),
    /// A click on a tray item (its key).
    TrayClick(String, Click),
    /// The pointer moved on a surface (its x there).
    Pointer(window::Id, f32),
    Closed(window::Id),
    /// An entry of the open tray menu (its id), or back from a submenu.
    MenuEntry(i32),
    MenuBack,
    /// A click on a workspace (its place in the bar).
    Activate(usize),
    Wheel(mouse::ScrollDelta),
}

struct Bar {
    desktop: Desktop<ExtWorkspaceHandleV1>,
    switcher: Option<Switcher>,
    clock: String,
    /// The clock's tooltip; empty for none.
    date: String,
    /// The pointer's latest entering or leaving the clock, counted: a
    /// tooltip asked for during an older one is stale.
    hover: u64,
    hovered: bool,
    /// The tooltip's surface, while it shows.
    tooltip: Option<window::Id>,
    /// None with no battery, or `--no-battery`.
    battery: Option<Battery>,
    /// An icon font (`glyphs.rs`): icons where there'd be words.
    icon_font: Option<glyphs::IconFont>,
    /// A touchpad's scroll not yet a workspace's worth.
    scrolled: f32,
    /// Along the bottom edge: the line facing the windows goes on top.
    bottom: bool,
    /// The bar's height, and a workspace's square block.
    height: u32,
    /// The surfaces open, by output.
    surfaces: Vec<(WlOutput, window::Id)>,
    style: Style,
    tray: Option<Tray>,
    tray_items: Vec<tray::Item>,
    /// Where the pointer was last on the bar: a menu opens there.
    pointer: Option<(window::Id, f32)>,
    menu: Option<OpenMenu>,
}

/// A tray item's menu, open in a popup of the bar's.
struct OpenMenu {
    window: window::Id,
    /// The item's key and its menu's path.
    key: String,
    path: String,
    /// The submenus shown, by id, from the top: the last one's entries are.
    submenus: Vec<i32>,
}

/// The text size for a bar `height` high: `style::TEXT_SIZE` at the default
/// height, and as much bigger or smaller as the bar is.
pub fn text_size(height: u32) -> f32 {
    (style::TEXT_SIZE * height as f32 / args::DEFAULT_HEIGHT as f32).round()
}

/// A tray icon's size in a bar `height` high.
fn tray_icon_size(height: u32) -> f32 {
    (height as f32 * 0.6).round()
}

/// The pixels a size of `pixels` at the default height takes in a bar
/// `height` high.
fn scaled(pixels: f32, height: u32) -> f32 {
    (pixels * height as f32 / args::DEFAULT_HEIGHT as f32).round()
}

/// The air around a workspace's block, which the bar's full height would
/// otherwise fill.
fn inset(height: u32) -> f32 {
    scaled(4.0, height)
}

/// Each side of the battery and the clock, and between the tray's icons and
/// what's next to them: the same air between every two on the right.
fn module_padding(height: u32) -> f32 {
    scaled(12.0, height)
}

/// Between an icon of the icon font and its text.
fn icon_gap(height: u32) -> f32 {
    scaled(6.0, height)
}

/// An icon of the icon font: a little bigger than the text, whose digits
/// fill less of their line than a glyph of the icon font does.
fn glyph_size(height: u32) -> f32 {
    (text_size(height) * 1.15).round()
}

/// The widest `clock` can be with other digits: each one a `0`, the widest
/// in proportional fonts, so the clock keeps its place as the time changes.
fn clock_placeholder(clock: &str) -> String {
    clock
        .chars()
        .map(|c| if c.is_ascii_digit() { '0' } else { c })
        .collect()
}

/// `shown`, as wide as `widest`: drawn over an invisible `widest`, from its
/// start.
fn steady<'a>(
    shown: iced::widget::Text<'a>,
    widest: iced::widget::Text<'a>,
) -> Element<'a, Message> {
    let line = |t: iced::widget::Text<'a>| t.wrapping(text::Wrapping::None).line_height(LINE);
    stack![line(widest).color(Color::TRANSPARENT), line(shown)].into()
}

/// The text and icons' line: as high as their size, centered in the bar,
/// rather than a taller line box each font places its glyphs in its own way.
const LINE: text::LineHeight = text::LineHeight::Relative(1.0);

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
        date: date_format,
        height,
        outputs,
        battery: show_battery,
        tray: show_tray,
    } = args;
    let desktop_connection = connection.clone();
    let boot = move || {
        let bar = Bar {
            desktop: Desktop::default(),
            switcher: None,
            clock: String::new(),
            date: String::new(),
            hover: 0,
            hovered: false,
            tooltip: None,
            battery: None,
            icon_font: None,
            scrolled: 0.0,
            bottom,
            height,
            surfaces: Vec::new(),
            style: Style::default(),
            tray: None,
            tray_items: Vec::new(),
            pointer: None,
            menu: None,
        };
        let updates = desktop::updates(desktop_connection.clone(), outputs.clone());
        let mut tasks = vec![
            Task::run(updates, Message::Update),
            Task::run(pila_ipc::styles("pila-bar"), Message::Style),
        ];
        if !format.is_empty() {
            tasks.push(Task::run(clock::ticks(format.clone()), Message::Clock));
            if !date_format.is_empty() {
                tasks.push(Task::run(clock::ticks(date_format.clone()), Message::Date));
            }
        }
        if show_tray {
            // Sharp at scale 2.
            let pixels = 2 * tray_icon_size(height) as u32;
            tasks.push(Task::run(tray::updates(pixels), Message::Tray));
        }
        if show_battery {
            tasks.push(Task::run(battery::updates(), Message::Battery));
        }
        if show_battery || !format.is_empty() {
            tasks.push(Task::perform(glyphs::found(), Message::IconFont));
        }
        (bar, Task::batch(tasks))
    };
    let result = iced_layershell::daemon(boot, NAMESPACE, Bar::update, Bar::view)
        .subscription(Bar::subscription)
        .theme(|bar: &Bar, _| Some(bar.style.theme()))
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
        Subscription::batch([
            event::listen_with(wheel),
            window::close_events().map(Message::Closed),
        ])
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
            Message::Date(date) => self.date = date,
            Message::ClockHover(entered) => {
                self.hover += 1;
                self.hovered = entered;
                if entered && self.menu.is_none() && !self.date.is_empty() {
                    let hover = self.hover;
                    return Task::perform(after(TOOLTIP_DELAY), move |()| {
                        Message::TooltipDue(hover)
                    });
                }
                return self.close_tooltip();
            }
            Message::TooltipDue(hover) => {
                if hover == self.hover && self.hovered && self.menu.is_none() {
                    return self.open_tooltip();
                }
            }
            Message::Battery(battery) => self.battery = Some(battery),
            Message::IconFont(font) => self.icon_font = font,
            Message::Style(style) => self.style = style,
            Message::Tray(tray::Update::Tray(tray)) => self.tray = Some(tray),
            Message::Tray(tray::Update::Items(items)) => self.tray_items = items,
            Message::TrayClick(key, click) => return self.tray_click(key, click),
            Message::Pointer(window, x) => {
                let popup = self.menu.as_ref().is_some_and(|m| m.window == window)
                    || self.tooltip == Some(window);
                if !popup {
                    self.pointer = Some((window, x));
                }
            }
            Message::Closed(window) => {
                if self.menu.as_ref().is_some_and(|m| m.window == window) {
                    self.menu = None;
                }
                if self.tooltip == Some(window) {
                    self.tooltip = None;
                }
            }
            Message::MenuEntry(id) => return self.menu_entry(id),
            Message::MenuBack => {
                if let Some(open) = &mut self.menu {
                    open.submenus.pop();
                }
            }
            Message::Activate(index) => self.activate(index),
            // Not while a menu is open: the wheel may be over it.
            Message::Wheel(_) if self.menu.is_some() => {}
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

    /// A click on a tray item: its menu, if it has one read and the click
    /// asks for it, else the item's own. A click while a menu is open
    /// closes it (a second click on the same item does only that).
    /// The clock's tooltip: a layer surface of its own below the bar (the
    /// compositor keeps it clear of the bar's exclusive zone), on the
    /// output the pointer is on, against its right edge, under the clock,
    /// which is the bar's last block; it takes no input. Not a popup:
    /// iced_layershell asks a grab for every popup it makes, and a tooltip
    /// mustn't hold the pointer (or, denied the grab, be dismissed).
    fn open_tooltip(&mut self) -> Task<Message> {
        let Some((bar, _)) = self.pointer else {
            return Task::none();
        };
        let Some(output) = self
            .surfaces
            .iter()
            .find(|(_, w)| *w == bar)
            .map(|(o, _)| o.clone())
        else {
            return Task::none();
        };
        let closed = self.close_tooltip();
        let (width, height) = tooltip_size(&self.date, self.height);
        let edge = if self.bottom {
            Anchor::Bottom
        } else {
            Anchor::Top
        };
        let (window, open) = Message::layershell_open(NewLayerShellSettings {
            size: Some((width, height)),
            layer: Layer::Top,
            anchor: edge | Anchor::Right,
            // Placed clear of the bar's exclusive zone.
            exclusive_zone: Some(0),
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::None,
            output_option: OutputOption::Output(output),
            events_transparent: true,
            namespace: Some(TOOLTIP_NAMESPACE.to_string()),
        });
        self.tooltip = Some(window);
        Task::batch([closed, open])
    }

    fn close_tooltip(&mut self) -> Task<Message> {
        match self.tooltip.take() {
            Some(window) => Task::done(Message::RemoveWindow(window)),
            None => Task::none(),
        }
    }

    fn tray_click(&mut self, key: String, click: Click) -> Task<Message> {
        let was_open = self.menu.as_ref().map(|m| m.key.clone());
        let closed = Task::batch([self.close_menu(), self.close_tooltip()]);
        let Some(tray) = &self.tray else {
            return closed;
        };
        if click != Click::ContextMenu {
            tray.click(&key, click);
            return closed;
        }
        if was_open.as_deref() == Some(key.as_str()) {
            return closed;
        }
        let item = self.tray_items.iter().find(|i| i.key == key);
        let menu = item.and_then(|i| Some((i.menu.clone()?, i.layout.clone()?)));
        let (Some((path, layout)), Some((parent, x))) = (menu, self.pointer) else {
            tray.click(&key, click);
            return closed;
        };
        if layout.is_empty() {
            tray.click(&key, click);
            return closed;
        }
        tray.menu_opened(&key, &path);
        let size = menu_size(&layout);
        let edge = (x.round() as i32, 0, 1, self.height as i32);
        let (anchor, gravity) = match self.bottom {
            true => (PopupAnchor::TopLeft, PopupGravity::TopRight),
            false => (PopupAnchor::BottomLeft, PopupGravity::BottomRight),
        };
        let settings = IcedNewPopupSettings::new(parent, size, edge)
            .anchor(anchor)
            .gravity(gravity);
        let (window, open) = Message::popup_open(settings);
        self.menu = Some(OpenMenu {
            window,
            key,
            path,
            submenus: Vec::new(),
        });
        Task::batch([closed, open])
    }

    /// Close the open menu, if there is one.
    fn close_menu(&mut self) -> Task<Message> {
        match self.menu.take() {
            Some(open) => Task::done(Message::RemoveWindow(open.window)),
            None => Task::none(),
        }
    }

    /// The entries the open menu shows now, with its item.
    fn menu_level(&self) -> Option<(&OpenMenu, &[menu::Entry])> {
        let open = self.menu.as_ref()?;
        let item = self.tray_items.iter().find(|i| i.key == open.key)?;
        let mut level: &[menu::Entry] = item.layout.as_deref()?;
        for id in &open.submenus {
            level = &level.iter().find(|e| e.id == *id)?.children;
        }
        Some((open, level))
    }

    /// An entry chosen: into its submenu, or its action and the menu closes.
    fn menu_entry(&mut self, id: i32) -> Task<Message> {
        let Some((open, level)) = self.menu_level() else {
            return self.close_menu();
        };
        let Some(entry) = level.iter().find(|e| e.id == id) else {
            return Task::none();
        };
        if !entry.children.is_empty() {
            if let Some(open) = &mut self.menu {
                open.submenus.push(id);
            }
            return Task::none();
        }
        if let Some(tray) = &self.tray {
            tray.menu_clicked(&open.key, &open.path, id);
        }
        self.close_menu()
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

    fn view(&self, window: window::Id) -> Element<'_, Message> {
        if let Some((open, level)) = self.menu_level().filter(|(m, _)| m.window == window) {
            return menu_view(level, !open.submenus.is_empty(), self.style);
        }
        if self.tooltip == Some(window) {
            return tooltip_view(&self.date, self.style);
        }
        let st = self.style;
        let workspaces = Row::with_children(
            self.desktop
                .workspaces
                .iter()
                .enumerate()
                .map(|(index, w)| button(index, &w.name, w.active, w.urgent, self.height, st)),
        )
        .height(Length::Fill);
        let left = container(workspaces)
            .width(Length::Fill)
            .height(Length::Fill)
            .clip(true);
        let height = self.height;
        let padding = module_padding(height);
        let gap = icon_gap(height);
        // An icon of the icon font, at its size, on the text's line.
        let glyph = |icons: glyphs::IconFont, glyph: char| {
            text(glyph.to_string())
                .font(icons.font)
                .size(glyph_size(height))
                .line_height(LINE)
        };
        // As wide as the time can be: the time changing doesn't move it.
        let time = steady(
            text(&self.clock)
                .font(style::FONT_STRONG)
                .color(st.text_bright),
            text(clock_placeholder(&self.clock)).font(style::FONT_STRONG),
        );
        // A clock before it, from the icon font, if it has one.
        let clock_icon = self
            .icon_font
            .and_then(|icons| Some(glyph(icons, icons.clock?).color(st.text)));
        let clock = container(
            mouse_area(match clock_icon {
                Some(icon) => {
                    Element::from(row![icon, time].spacing(gap).align_y(Alignment::Center))
                }
                None => time,
            })
            .on_enter(Message::ClockHover(true))
            .on_exit(Message::ClockHover(false)),
        )
        .padding([0.0, padding])
        .height(Length::Fill)
        .align_x(Alignment::End)
        .align_y(Alignment::Center)
        .clip(true);
        let battery = self.battery.map(|battery| {
            let low = battery.low();
            let color = if low { st.text_bright } else { st.text };
            // With an icon font, its icon and the percentage; else the
            // percentage, with an arrow up while charging.
            let shown: Element<'_, Message> = match self.icon_font {
                Some(icons) => row![
                    {
                        let icon = glyphs::battery(&battery, icons.tens);
                        let icon_color = if icon.accent && !low {
                            st.accent
                        } else {
                            color
                        };
                        glyph(icons, icon.glyph).color(icon_color)
                    },
                    text(format!("{}%", battery.percent))
                        .line_height(LINE)
                        .color(color),
                ]
                .spacing(gap)
                .align_y(Alignment::Center)
                .into(),
                None => text(battery.text())
                    .wrapping(text::Wrapping::None)
                    .line_height(LINE)
                    .color(color)
                    .into(),
            };
            container(shown)
                .padding([0.0, padding])
                .height(Length::Fill)
                .align_y(Alignment::Center)
                .style(move |_| match low {
                    true => container::Style::default().background(st.urgent),
                    false => container::Style::default(),
                })
        });
        // The workspaces on the left; the battery and the clock on the right.
        let mut bar = row![left].height(Length::Fill);
        let icon_size = tray_icon_size(height);
        if !self.tray_items.is_empty() {
            // The icons' square blocks already leave some air around them:
            // as much again as the rest of the right side has, not more.
            let air = (height as f32 - icon_size) / 2.0;
            let icons = Row::with_children(
                self.tray_items
                    .iter()
                    .map(|item| tray_item(item, height, icon_size, st)),
            );
            bar = bar.push(
                container(icons)
                    .height(Length::Fill)
                    .padding([0.0, (padding - air).max(0.0)]),
            );
        }
        if let Some(battery) = battery {
            bar = bar.push(battery);
        }
        // No clock (`--clock ''`): no room kept for it.
        if !self.clock.is_empty() {
            bar = bar.push(clock);
        }
        let edge = container(Space::new())
            .width(Length::Fill)
            .height(EDGE)
            .style(move |_| container::Style::default().background(st.edge));
        let content = if self.bottom {
            column![edge, bar]
        } else {
            column![bar, edge]
        };
        let bar = container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style::default().background(st.surface_dim));
        // Where the pointer is on this surface, for a tray menu to open
        // there: told as it enters too (with no motion before a click, as
        // a pointer warped onto the bar), which a tray item doesn't capture.
        mouse_area(bar)
            .on_move(move |position| Message::Pointer(window, position.x))
            .into()
    }
}

/// A workspace's button: a square block inset in a cell as high as the bar
/// (as wide as it too, for a name of one or two characters), the active one
/// in the accent color and its name semibold, an urgent one in the
/// attention color, the others plain until hovered. The air around the
/// block is the workspace's too.
fn button<'a>(
    index: usize,
    name: &'a str,
    active: bool,
    urgent: bool,
    height: u32,
    st: Style,
) -> Element<'a, Message> {
    let font = if active {
        style::FONT_STRONG
    } else {
        style::FONT
    };
    // As wide as when active: one becoming active moves none.
    let label = steady(text(name).font(font), text(name).font(style::FONT_STRONG));
    let block = button_widget(
        container(label)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
    )
    .on_press(Message::Activate(index))
    .height(Length::Fill)
    .style(move |_, status| {
        let hovered = matches!(
            status,
            button_widget::Status::Hovered | button_widget::Status::Pressed
        );
        let (background, text_color) = if active {
            (Some(st.accent), st.text_bright)
        } else if urgent {
            (Some(st.urgent), st.text_bright)
        } else if hovered {
            (Some(st.hover), st.text_bright)
        } else {
            (None, st.text_dim)
        };
        button_widget::Style {
            background: background.map(Into::into),
            text_color,
            // Square corners.
            border: Border::default(),
            ..Default::default()
        }
    });
    let short = name.chars().count() <= 2;
    let block = match short {
        true => block.width(Length::Fill).padding(0),
        false => block.padding([0.0, scaled(8.0, height)]),
    };
    let cell = container(block).height(Length::Fill).padding(inset(height));
    let cell = match short {
        true => cell.width(height as f32),
        false => cell,
    };
    mouse_area(cell)
        .on_press(Message::Activate(index))
        .interaction(mouse::Interaction::Pointer)
        .into()
}

/// A tray item: its icon in a square block as high as the bar, standing
/// out while it asks for attention. The left button activates it (or asks
/// for its menu, if that's all it has), the middle one is its second
/// action, the right one its menu, and the wheel is its own.
fn tray_item<'a>(
    item: &tray::Item,
    height: u32,
    icon_size: f32,
    st: Style,
) -> Element<'a, Message> {
    let icon: Element<'a, Message> = match &item.icon {
        Some(tray::Icon::Image(handle)) => image(handle.clone())
            .width(icon_size)
            .height(icon_size)
            .into(),
        Some(tray::Icon::Svg(handle)) => svg(handle.clone())
            .width(icon_size)
            .height(icon_size)
            .into(),
        Some(tray::Icon::Symbolic(handle)) => svg(handle.clone())
            .width(icon_size)
            .height(icon_size)
            .style(move |_, _| svg::Style {
                color: Some(st.text),
            })
            .into(),
        // No icon found: its title's first letter.
        None => text(item.title.chars().next().unwrap_or('?').to_string())
            .color(st.text)
            .into(),
    };
    let attention = item.needs_attention;
    let block = container(icon)
        .width(height as f32)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_| match attention {
            true => container::Style::default().background(st.urgent),
            false => container::Style::default(),
        });
    let key = item.key.clone();
    let left = match item.item_is_menu {
        true => Click::ContextMenu,
        false => Click::Activate,
    };
    let scroll_key = key.clone();
    mouse_area(block)
        .on_press(Message::TrayClick(key.clone(), left))
        .on_middle_press(Message::TrayClick(key.clone(), Click::SecondaryActivate))
        .on_right_press(Message::TrayClick(key, Click::ContextMenu))
        .on_scroll(move |delta| {
            let notches = match delta {
                mouse::ScrollDelta::Lines { y, .. } => -y,
                mouse::ScrollDelta::Pixels { y, .. } => -y / PIXELS_PER_STEP,
            };
            Message::TrayClick(scroll_key.clone(), Click::Scroll(notches.round() as i32))
        })
        .interaction(mouse::Interaction::Pointer)
        .into()
}

/// How long the pointer stays on the clock before its tooltip shows.
const TOOLTIP_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// Done after `delay`, from a thread of its own: no async runtime's timer.
fn after(delay: std::time::Duration) -> impl iced::futures::Future<Output = ()> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("pila-bar-delay".into())
        .spawn(move || {
            std::thread::sleep(delay);
            let _ = sender.send(());
        });
    if let Err(err) = spawned {
        eprintln!("pila-bar: no tooltip: {err}");
    }
    async move {
        let _ = receiver.await;
    }
}

/// A tooltip's size for `text` in a bar `height` high: a row as high as
/// the bar, about as wide as the text.
fn tooltip_size(text: &str, height: u32) -> (u32, u32) {
    // At least as wide as a character, for the text's size (proportional
    // fonts' widest run near 0.7 of it): a little room to spare, the text
    // centered, rather than a date cut short.
    let char_width = text_size(height) * 0.7;
    let width = text.chars().count() as f32 * char_width + 2.0 * 12.0 + 2.0 * MENU_BORDER;
    (width.round() as u32, height + 2 * MENU_BORDER as u32)
}

/// The clock's tooltip: its text in a square box, as a menu is.
fn tooltip_view<'a>(date: &str, st: Style) -> Element<'a, Message> {
    container(
        text(date.to_string())
            .wrapping(text::Wrapping::None)
            .color(st.text),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding([0, 12])
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .style(move |_| container::Style {
        background: Some(st.surface_dim.into()),
        border: Border {
            color: st.hover,
            width: MENU_BORDER,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// A popup's size for a menu with these entries: the tallest of its
/// levels, as wide as its longest label.
fn menu_size(entries: &[menu::Entry]) -> (u32, u32) {
    let (rows, separators, widest) = menu::extent(entries);
    // A toggle's mark, the label, a submenu's arrow.
    let width = (widest as f32 * MENU_CHAR + 3.0 * 24.0).clamp(160.0, 480.0);
    let height = rows as f32 * MENU_ROW + separators as f32 * MENU_SEPARATOR + 2.0 * MENU_BORDER;
    (width.round() as u32, height.round() as u32)
}

/// A tray menu's level: a row to go back from a submenu, then its entries
/// (a check or radio mark, the label, an arrow for a submenu), square, as
/// the rest of the bar.
fn menu_view<'a>(entries: &[menu::Entry], back: bool, st: Style) -> Element<'a, Message> {
    let row_style = move |enabled: bool| {
        move |_: &Theme, status: button_widget::Status| {
            let hovered = enabled
                && matches!(
                    status,
                    button_widget::Status::Hovered | button_widget::Status::Pressed
                );
            button_widget::Style {
                background: hovered.then_some(st.accent.into()),
                text_color: match (enabled, hovered) {
                    (false, _) => st.text_dim,
                    (true, true) => st.text_bright,
                    (true, false) => st.text,
                },
                border: Border::default(),
                ..Default::default()
            }
        }
    };
    let line = |mark: &str, label: String, arrow: &str| {
        row![
            text(mark.to_string()).width(16),
            text(label)
                .wrapping(text::Wrapping::None)
                .width(Length::Fill),
            text(arrow.to_string()),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
    };
    let mut list = column![];
    if back {
        list = list.push(
            button_widget(line("\u{2039}", String::new(), ""))
                .on_press(Message::MenuBack)
                .width(Length::Fill)
                .height(MENU_ROW)
                .padding([0, 8])
                .style(row_style(true)),
        );
    }
    for entry in entries {
        if entry.separator {
            list = list.push(
                container(
                    container(Space::new())
                        .width(Length::Fill)
                        .height(1)
                        .style(move |_| container::Style::default().background(st.hover)),
                )
                .height(MENU_SEPARATOR)
                .align_y(Alignment::Center),
            );
            continue;
        }
        let mark = match entry.toggle {
            menu::Toggle::Check(true) => "\u{2713}",
            menu::Toggle::Radio(true) => "\u{2022}",
            _ => "",
        };
        let arrow = if entry.children.is_empty() {
            ""
        } else {
            "\u{203a}"
        };
        let mut button = button_widget(line(mark, entry.label.clone(), arrow))
            .width(Length::Fill)
            .height(MENU_ROW)
            .padding([0, 8])
            .style(row_style(entry.enabled));
        if entry.enabled {
            button = button.on_press(Message::MenuEntry(entry.id));
        }
        list = list.push(button);
    }
    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(MENU_BORDER)
        .style(move |_| container::Style {
            background: Some(st.surface_dim.into()),
            border: Border {
                color: st.hover,
                width: MENU_BORDER,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// The wheel over the bar, unless a tray item took it.
fn wheel(event: Event, status: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::WheelScrolled { delta }) if status == event::Status::Ignored => {
            Some(Message::Wheel(delta))
        }
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
    fn the_clock_keeps_its_widest() {
        assert_eq!(clock_placeholder("17:21"), "00:00");
        assert_eq!(
            clock_placeholder("Wed 9 Nov, 1:05 pm"),
            "Wed 0 Nov, 0:00 pm"
        );
        assert_eq!(clock_placeholder(""), "");
    }

    #[test]
    fn sizes_follow_the_height() {
        let default = args::DEFAULT_HEIGHT;
        assert_eq!(inset(default), 4.0);
        assert_eq!(module_padding(default), 12.0);
        assert_eq!(icon_gap(default), 6.0);
        assert_eq!(glyph_size(default), 16.0);
        assert_eq!(inset(default * 2), 8.0);
        assert_eq!(module_padding(default * 2), 24.0);
        // The block stays square in a cell as wide as the bar is high.
        assert!(inset(40) > inset(default));
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
