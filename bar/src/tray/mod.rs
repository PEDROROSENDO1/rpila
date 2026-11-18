//! The tray: StatusNotifierItems (the icons of fcitx5, nm-applet, Steam,
//! Discord, ...) over the D-Bus session bus, with zbus's blocking API on a
//! thread of its own, so nothing waits on the bar's.
//!
//! The bar is the StatusNotifierWatcher (the registry items announce
//! themselves to) when no one else is, as on Pila, and a host of its own in
//! any case: it follows the watcher's items and their signals, reads each
//! one's properties on a thread (a hung app holds only its own), and sends
//! the bar every change as the whole list (`model::Model`). A click calls
//! the item, on a thread too.

#[cfg(test)]
mod bus_tests;
pub mod icons;
pub mod menu;
mod model;

use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

use iced::widget::{image, svg};
use zbus::{
    blocking::{Connection, MessageIterator},
    fdo::{RequestNameFlags, RequestNameReply},
    message::{Header, Type as MessageType},
    object_server::SignalEmitter,
    zvariant::{OwnedValue, Value},
    MatchRule,
};

use model::{item_key, pick_pixmap, rgba, split_key, Model, Props, Status};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";

/// `GetLayout`'s answer: a revision, and the root's `(ia{sv}av)`.
type Layout = (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>));
/// How long an item may take to answer.
const TIMEOUT: Duration = Duration::from_secs(3);

/// An item's icon, ready to draw.
#[derive(Debug, Clone)]
pub enum Icon {
    Image(image::Handle),
    Svg(svg::Handle),
    /// A `-symbolic` icon: one color, drawn in the text's.
    Symbolic(svg::Handle),
}

pub type Item = model::Item<Icon>;

/// What the tray's thread sends the bar.
#[derive(Debug, Clone)]
pub enum Update {
    /// Items can be clicked.
    Tray(Tray),
    /// The items to show, all of them.
    Items(Vec<Item>),
}

/// What a click on an item asks of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// The left button: what the app does when its icon is clicked.
    Activate,
    /// The middle button.
    SecondaryActivate,
    /// The right button, for an item with no menu Pila can show.
    ContextMenu,
    /// The wheel, in notches (positive: down).
    Scroll(i32),
}

/// Clicks items, on the tray's connection.
#[derive(Debug, Clone)]
pub struct Tray {
    connection: Connection,
}

impl Tray {
    /// Its menu is about to show: the app may update it (`AboutToShow`).
    pub fn menu_opened(&self, key: &str, menu: &str) {
        // Optional for the app: its answer, or none, changes nothing here.
        self.call_menu(key, menu, |connection, service, menu| {
            let _ =
                connection.call_method(Some(service), menu, Some(MENU), "AboutToShow", &(0i32,));
        });
    }

    /// An entry of its menu was chosen.
    pub fn menu_clicked(&self, key: &str, menu: &str, id: i32) {
        self.call_menu(key, menu, move |connection, service, menu| {
            let body = (id, "clicked", Value::I32(0), 0u32);
            if let Err(err) =
                connection.call_method(Some(service), menu, Some(MENU), "Event", &body)
            {
                eprintln!("pila-bar: the tray menu {service}{menu} didn't take a click: {err}");
            }
        });
    }

    /// `call` on a thread, with the item's bus name and its menu's path.
    fn call_menu(
        &self,
        key: &str,
        menu: &str,
        call: impl FnOnce(&Connection, &str, &str) + Send + 'static,
    ) {
        let service = split_key(key).0.to_string();
        let menu = menu.to_string();
        let connection = self.connection.clone();
        let spawned = std::thread::Builder::new()
            .name("pila-bar-tray-menu".into())
            .spawn(move || call(&connection, &service, &menu));
        if let Err(err) = spawned {
            eprintln!("pila-bar: can't reach a tray menu: {err}");
        }
    }

    pub fn click(&self, key: &str, click: Click) {
        let (service, path) = split_key(key);
        let (service, path) = (service.to_string(), path.to_string());
        let connection = self.connection.clone();
        // At the pointer, which a Wayland client doesn't know: items take
        // 0, 0 as "wherever".
        let spawned = std::thread::Builder::new()
            .name("pila-bar-tray-click".into())
            .spawn(move || {
                let call = |method: &str, body: &(i32, i32)| {
                    connection.call_method(
                        Some(service.as_str()),
                        path.as_str(),
                        Some(ITEM),
                        method,
                        body,
                    )
                };
                let result = match click {
                    Click::Activate => call("Activate", &(0, 0)),
                    Click::SecondaryActivate => call("SecondaryActivate", &(0, 0)),
                    Click::ContextMenu => call("ContextMenu", &(0, 0)),
                    Click::Scroll(delta) => connection.call_method(
                        Some(service.as_str()),
                        path.as_str(),
                        Some(ITEM),
                        "Scroll",
                        &(delta, "vertical"),
                    ),
                };
                if let Err(err) = result {
                    eprintln!("pila-bar: the tray item {service}{path} didn't take a click: {err}");
                }
            });
        if let Err(err) = spawned {
            eprintln!("pila-bar: can't click a tray item: {err}");
        }
    }
}

/// The tray's updates, from a thread with a session bus connection of its
/// own, icons found for about `icon_size` pixels. Nothing without a bus.
pub fn updates(icon_size: u32) -> impl iced::futures::Stream<Item = Update> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("pila-bar-tray".into())
        .spawn(move || {
            let connection = match zbus::blocking::connection::Builder::session()
                .map(|b| b.method_timeout(TIMEOUT))
                .and_then(|b| b.build())
            {
                Ok(connection) => connection,
                Err(err) => {
                    eprintln!("pila-bar: no tray, no D-Bus session bus: {err}");
                    return;
                }
            };
            let places = icons::Places::of_session();
            let send = |update| sender.unbounded_send(update).is_ok();
            if let Err(err) = follow(&connection, &places, icon_size, send) {
                eprintln!("pila-bar: the tray stopped: {err}");
            }
        });
    if let Err(err) = spawned {
        eprintln!("pila-bar: no tray: {err}");
    }
    receiver
}

/// What the tray's thread hears.
enum Event {
    Registered(String),
    Unregistered(String),
    /// An item's signal (a new icon, status, title...): read it again.
    Changed {
        sender: String,
        path: String,
    },
    /// An item's properties, and the unique name of the connection that
    /// has it.
    Read {
        key: String,
        owner: String,
        props: Props<Icon>,
    },
    /// A bus name gone: its items go too.
    Gone(String),
    /// A menu's signal (its layout or entries changed): read it again.
    MenuChanged {
        sender: String,
        path: String,
    },
    /// An item's menu, read.
    Layout {
        key: String,
        layout: model::Menu,
    },
}

/// Be the watcher if there's none, register as a host, and send the items
/// until `send` says no one listens.
fn follow(
    connection: &Connection,
    places: &icons::Places,
    icon_size: u32,
    mut send: impl FnMut(Update) -> bool,
) -> zbus::Result<()> {
    let (events, heard) = mpsc::channel();
    // Listening first, so that nothing registered meanwhile is missed.
    listen(connection, watcher_signals()?, events.clone(), |message| {
        let header = message.header();
        let key: String = message.body().deserialize().ok()?;
        match header.member()?.as_str() {
            "StatusNotifierItemRegistered" => Some(Event::Registered(key)),
            "StatusNotifierItemUnregistered" => Some(Event::Unregistered(key)),
            _ => None,
        }
    })?;
    listen(connection, item_signals()?, events.clone(), |message| {
        let header = message.header();
        Some(Event::Changed {
            sender: header.sender()?.to_string(),
            path: header.path()?.to_string(),
        })
    })?;
    listen(connection, menu_signals()?, events.clone(), |message| {
        let header = message.header();
        Some(Event::MenuChanged {
            sender: header.sender()?.to_string(),
            path: header.path()?.to_string(),
        })
    })?;
    listen(connection, names_gone()?, events.clone(), |message| {
        let (name, _, new): (String, String, String) = message.body().deserialize().ok()?;
        new.is_empty().then_some(Event::Gone(name))
    })?;

    let registered = Arc::new(Mutex::new(Vec::new()));
    connection.object_server().at(
        WATCHER_PATH,
        Watcher {
            items: registered.clone(),
        },
    )?;
    let ours = connection.request_name_with_flags(WATCHER, RequestNameFlags::DoNotQueue.into())?
        == RequestNameReply::PrimaryOwner;
    let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    connection.request_name(host.as_str())?;
    connection.call_method(
        Some(WATCHER),
        WATCHER_PATH,
        Some(WATCHER),
        "RegisterStatusNotifierHost",
        &(host.as_str(),),
    )?;
    if !send(Update::Tray(Tray {
        connection: connection.clone(),
    })) {
        return Ok(());
    }
    let reply = connection.call_method(
        Some(WATCHER),
        WATCHER_PATH,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(WATCHER, "RegisteredStatusNotifierItems"),
    )?;
    let value: OwnedValue = reply.body().deserialize()?;
    for key in strings(&value) {
        let _ = events.send(Event::Registered(key));
    }

    let mut model = Model::default();
    let mut owners: HashMap<String, String> = HashMap::new();
    for event in heard {
        match event {
            Event::Registered(key) => {
                if model.registered(&key) {
                    read(connection, &key, places, icon_size, &events);
                }
                continue;
            }
            Event::Unregistered(key) => {
                model.unregistered(&key);
                owners.remove(&key);
            }
            Event::Changed { sender, path } => {
                for key in model.keys() {
                    if owners.get(key) == Some(&sender) && split_key(key).1 == path {
                        read(connection, key, places, icon_size, &events);
                    }
                }
                continue;
            }
            Event::Read { key, owner, props } => {
                owners.insert(key.clone(), owner);
                let had_menu = model.menu(&key).map(str::to_string);
                model.read(&key, props);
                // Read ahead of a click; again only for a new menu.
                if let Some(menu) = model.menu(&key).filter(|m| had_menu.as_deref() != Some(*m)) {
                    read_layout(connection, &key, menu, &events);
                }
            }
            Event::MenuChanged { sender, path } => {
                for key in model.keys() {
                    if owners.get(key) == Some(&sender) && model.menu(key) == Some(path.as_str()) {
                        read_layout(connection, key, &path, &events);
                    }
                }
                continue;
            }
            Event::Layout { key, layout } => model.read_layout(&key, layout),
            Event::Gone(name) => {
                let gone: Vec<String> = model
                    .keys()
                    .filter(|key| split_key(key).0 == name || owners.get(*key) == Some(&name))
                    .map(str::to_string)
                    .collect();
                if ours {
                    unregister(connection, &registered, &name);
                }
                if gone.is_empty() {
                    continue;
                }
                for key in &gone {
                    model.unregistered(key);
                    owners.remove(key);
                }
            }
        }
        if !send(Update::Items(model.items())) {
            return Ok(());
        }
    }
    Ok(())
}

/// The watcher's part when a bus name goes: its items leave the list, and
/// hosts hear of it.
fn unregister(connection: &Connection, registered: &Mutex<Vec<String>>, name: &str) {
    let gone: Vec<String> = {
        let Ok(mut items) = registered.lock() else {
            return;
        };
        let (gone, kept) = items.drain(..).partition(|key| split_key(key).0 == name);
        *items = kept;
        gone
    };
    for key in gone {
        let _ = connection.emit_signal(
            None::<&str>,
            WATCHER_PATH,
            WATCHER,
            "StatusNotifierItemUnregistered",
            &(key.as_str(),),
        );
    }
}

/// Forward the messages `rule` matches, as `event` makes them, from a
/// thread of their own.
fn listen(
    connection: &Connection,
    rule: MatchRule<'static>,
    events: mpsc::Sender<Event>,
    event: impl Fn(&zbus::Message) -> Option<Event> + Send + 'static,
) -> zbus::Result<()> {
    let messages = MessageIterator::for_match_rule(rule, connection, Some(256))?;
    std::thread::Builder::new()
        .name("pila-bar-tray-signals".into())
        .spawn(move || {
            for message in messages.flatten() {
                if let Some(event) = event(&message) {
                    if events.send(event).is_err() {
                        return;
                    }
                }
            }
        })
        .map_err(|err| zbus::Error::Failure(err.to_string()))?;
    Ok(())
}

fn watcher_signals() -> zbus::Result<MatchRule<'static>> {
    Ok(MatchRule::builder()
        .msg_type(MessageType::Signal)
        .interface(WATCHER)?
        .path(WATCHER_PATH)?
        .build())
}

fn item_signals() -> zbus::Result<MatchRule<'static>> {
    Ok(MatchRule::builder()
        .msg_type(MessageType::Signal)
        .interface(ITEM)?
        .build())
}

fn menu_signals() -> zbus::Result<MatchRule<'static>> {
    Ok(MatchRule::builder()
        .msg_type(MessageType::Signal)
        .interface(MENU)?
        .build())
}

/// Read an item's menu on a thread: `Event::Layout` when done.
fn read_layout(connection: &Connection, key: &str, menu: &str, events: &mpsc::Sender<Event>) {
    let (connection, key, menu, events) = (
        connection.clone(),
        key.to_string(),
        menu.to_string(),
        events.clone(),
    );
    let spawned = std::thread::Builder::new()
        .name("pila-bar-tray-menu".into())
        .spawn(move || {
            let service = split_key(&key).0;
            let layout = connection
                .call_method(
                    Some(service),
                    menu.as_str(),
                    Some(MENU),
                    "GetLayout",
                    &(0i32, -1i32, Vec::<&str>::new()),
                )
                .and_then(|reply| reply.body().deserialize::<Layout>());
            match layout {
                Ok((_, (_, _, children))) => {
                    let layout = Arc::new(menu::entries(children.iter().map(|c| &**c)));
                    let _ = events.send(Event::Layout { key, layout });
                }
                Err(err) => eprintln!("pila-bar: can't read the menu of {key}: {err}"),
            }
        });
    if let Err(err) = spawned {
        eprintln!("pila-bar: can't read a tray menu: {err}");
    }
}

fn names_gone() -> zbus::Result<MatchRule<'static>> {
    Ok(MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .build())
}

/// Read an item's properties on a thread, and its icons: `Event::Read`
/// when done; nothing if it doesn't answer.
fn read(
    connection: &Connection,
    key: &str,
    places: &icons::Places,
    icon_size: u32,
    events: &mpsc::Sender<Event>,
) {
    let (connection, key, places, events) = (
        connection.clone(),
        key.to_string(),
        places.clone(),
        events.clone(),
    );
    let spawned = std::thread::Builder::new()
        .name("pila-bar-tray-read".into())
        .spawn(move || {
            let (service, path) = split_key(&key);
            let owner = connection
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "GetNameOwner",
                    &(service,),
                )
                .and_then(|reply| reply.body().deserialize::<String>());
            let all = connection
                .call_method(
                    Some(service),
                    path,
                    Some("org.freedesktop.DBus.Properties"),
                    "GetAll",
                    &(ITEM,),
                )
                .and_then(|reply| reply.body().deserialize::<HashMap<String, OwnedValue>>());
            match (owner, all) {
                (Ok(owner), Ok(all)) => {
                    let props = props(&all, &places, icon_size);
                    let _ = events.send(Event::Read { key, owner, props });
                }
                (Err(err), _) | (_, Err(err)) => {
                    eprintln!("pila-bar: can't read the tray item {key}: {err}");
                }
            }
        });
    if let Err(err) = spawned {
        eprintln!("pila-bar: can't read a tray item: {err}");
    }
}

/// An item's properties as the bar takes them, its icons found.
fn props(all: &HashMap<String, OwnedValue>, places: &icons::Places, size: u32) -> Props<Icon> {
    let text = |name: &str| all.get(name).and_then(|v| string(v)).unwrap_or_default();
    let theme_path = text("IconThemePath");
    let icon = |name: &str, pixmap: &str| {
        named_icon(&text(name), &theme_path, size, places)
            .or_else(|| all.get(pixmap).and_then(|v| pixmap_icon(v, size)))
    };
    let menu = all
        .get("Menu")
        .and_then(|v| match &**v {
            Value::ObjectPath(path) => Some(path.to_string()),
            _ => None,
        })
        .filter(|path| path != "/" && path != "/NO_DBUSMENU");
    let title = Some(text("Title"))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| text("Id"));
    Props {
        title,
        status: Status::parse(&text("Status")),
        icon: icon("IconName", "IconPixmap"),
        attention_icon: icon("AttentionIconName", "AttentionIconPixmap"),
        menu,
        item_is_menu: matches!(all.get("ItemIsMenu").map(|v| &**v), Some(Value::Bool(true))),
    }
}

fn string(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.to_string()),
        _ => None,
    }
}

fn strings(value: &Value) -> Vec<String> {
    match value {
        Value::Array(array) => array.iter().filter_map(string).collect(),
        // Get's answer may come as a variant in a variant.
        Value::Value(inner) => strings(inner),
        _ => Vec::new(),
    }
}

fn named_icon(name: &str, theme_path: &str, size: u32, places: &icons::Places) -> Option<Icon> {
    if name.is_empty() {
        return None;
    }
    Some(match icons::find(name, theme_path, size, places)? {
        icons::IconFile::Png(path) => Icon::Image(image::Handle::from_path(path)),
        icons::IconFile::Svg(path) => {
            let symbolic = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.ends_with("-symbolic"));
            match symbolic {
                true => Icon::Symbolic(svg::Handle::from_path(path)),
                false => Icon::Svg(svg::Handle::from_path(path)),
            }
        }
    })
}

/// The pixmap (`a(iiay)`) closest to `size`, as an image.
fn pixmap_icon(value: &Value, size: u32) -> Option<Icon> {
    let Value::Array(array) = value else {
        return None;
    };
    let pixmaps: Vec<(i32, i32, Vec<u8>)> = array
        .iter()
        .filter_map(|item| {
            let Value::Structure(fields) = item else {
                return None;
            };
            match fields.fields() {
                [Value::I32(w), Value::I32(h), Value::Array(bytes)] => {
                    let bytes = bytes
                        .iter()
                        .map(|b| match b {
                            Value::U8(b) => Some(*b),
                            _ => None,
                        })
                        .collect::<Option<Vec<u8>>>()?;
                    Some((*w, *h, bytes))
                }
                _ => None,
            }
        })
        .collect();
    let sizes: Vec<(i32, i32)> = pixmaps.iter().map(|(w, h, _)| (*w, *h)).collect();
    let (w, h, argb) = &pixmaps[pick_pixmap(&sizes, size as i32)?];
    let pixels = rgba(*w, *h, argb)?;
    Some(Icon::Image(image::Handle::from_rgba(
        *w as u32, *h as u32, pixels,
    )))
}

/// The StatusNotifierWatcher, when the bar is: the items registered, by key.
struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let sender = header.sender().map(|s| s.as_str()).unwrap_or_default();
        let key = item_key(service, sender);
        let new = match self.items.lock() {
            Ok(mut items) if !items.contains(&key) => {
                items.push(key.clone());
                true
            }
            _ => false,
        };
        if new {
            let _ = Self::status_notifier_item_registered(&emitter, &key).await;
        }
    }

    async fn register_status_notifier_host(
        &self,
        _service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let _ = Self::status_notifier_host_registered(&emitter).await;
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items
            .lock()
            .map(|items| items.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
