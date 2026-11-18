//! The tray against a real bus: a private dbus-daemon, the tray as watcher
//! and host, and items of the test's own, served with zbus as apps serve
//! theirs. Skipped where there's no dbus-daemon.

use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU32, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

use zbus::{
    blocking::Connection,
    object_server::SignalEmitter,
    zvariant::{Array, Dict, OwnedObjectPath, Signature, StructureBuilder},
};

use super::*;

/// A session bus of the test's own, gone when dropped.
struct Bus {
    daemon: Child,
    address: String,
}

impl Bus {
    fn start() -> Option<Bus> {
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| eprintln!("skipped: no dbus-daemon ({err})"))
            .ok()?;
        let mut address = String::new();
        BufReader::new(daemon.stdout.take()?)
            .read_line(&mut address)
            .ok()?;
        Some(Bus {
            daemon,
            address: address.trim().to_string(),
        })
    }

    fn connect(&self) -> zbus::blocking::connection::Builder<'static> {
        zbus::blocking::connection::Builder::address(self.address.as_str())
            .unwrap()
            .method_timeout(TIMEOUT)
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

/// An app's tray item: a title, a status the test changes, a 2x2 red
/// pixmap, and how often it was activated.
struct FakeItem {
    status: Arc<Mutex<String>>,
    activated: Arc<AtomicU32>,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl FakeItem {
    fn activate(&self, _x: i32, _y: i32) {
        self.activated.fetch_add(1, Ordering::SeqCst);
    }

    #[zbus(property)]
    fn id(&self) -> String {
        "fake".into()
    }

    #[zbus(property)]
    fn title(&self) -> String {
        "Fake".into()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        self.status.lock().unwrap().clone()
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![(2, 2, [0xff, 0xff, 0, 0].repeat(4))]
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(MENU_PATH).unwrap()
    }

    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
}

const MENU_PATH: &str = "/MenuBar";

/// An app's menu: "Open" (1) and a submenu "More" (2) with "Quit" (3);
/// the entries clicked.
struct FakeMenu {
    clicked: Arc<Mutex<Vec<i32>>>,
}

fn layout(id: i32, label: &str, children: Vec<Value<'static>>) -> Value<'static> {
    let mut props = Dict::new(&Signature::Str, &Signature::Variant);
    props
        .append(
            Value::from("label"),
            Value::Value(Box::new(Value::from(label.to_string()))),
        )
        .unwrap();
    let mut array = Array::new(&Signature::Variant);
    for child in children {
        array.append(Value::Value(Box::new(child))).unwrap();
    }
    Value::Structure(
        StructureBuilder::new()
            .add_field(id)
            .append_field(Value::Dict(props))
            .append_field(Value::Array(array))
            .build()
            .unwrap(),
    )
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl FakeMenu {
    fn get_layout(&self, _parent: i32, _depth: i32, _properties: Vec<String>) -> Layout {
        let more = layout(2, "_More", vec![layout(3, "Quit", vec![])]);
        let children = [layout(1, "Open", vec![]), more]
            .into_iter()
            .map(|v| OwnedValue::try_from(v).unwrap())
            .collect();
        (1, (0, HashMap::new(), children))
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn event(&self, id: i32, event: String, _data: OwnedValue, _timestamp: u32) {
        if event == "clicked" {
            self.clicked.lock().unwrap().push(id);
        }
    }
}

/// The tray following `bus`, and what it sends.
fn tray(bus: &Bus) -> (Tray, mpsc::Receiver<Update>) {
    let connection = bus.connect().build().unwrap();
    let places = icons::Places {
        data_dirs: Vec::new(),
        themes: Vec::new(),
    };
    let (sender, updates) = mpsc::channel();
    std::thread::spawn(move || {
        let send = |update| sender.send(update).is_ok();
        follow(&connection, &places, 32, send).unwrap();
    });
    match updates.recv_timeout(Duration::from_secs(10)) {
        Ok(Update::Tray(tray)) => (tray, updates),
        other => panic!("no tray: {other:?}"),
    }
}

/// The next items `what` holds for.
#[track_caller]
fn wait_items(
    updates: &mpsc::Receiver<Update>,
    what: &str,
    f: impl Fn(&[Item]) -> bool,
) -> Vec<Item> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match updates.recv_timeout(left) {
            Ok(Update::Items(items)) if f(&items) => return items,
            Ok(_) => {}
            Err(_) => panic!("never {what}"),
        }
    }
}

fn register(connection: &Connection, argument: &str) {
    connection
        .call_method(
            Some(WATCHER),
            WATCHER_PATH,
            Some(WATCHER),
            "RegisterStatusNotifierItem",
            &(argument,),
        )
        .unwrap();
}

#[test]
fn items_come_change_take_clicks_and_go() {
    let Some(bus) = Bus::start() else { return };
    let (tray, updates) = tray(&bus);

    let status = Arc::new(Mutex::new("Active".to_string()));
    let activated = Arc::new(AtomicU32::new(0));
    let item = FakeItem {
        status: status.clone(),
        activated: activated.clone(),
    };
    let name = "org.kde.StatusNotifierItem-1-1";
    let clicked = Arc::new(Mutex::new(Vec::new()));
    let app = bus
        .connect()
        .serve_at("/StatusNotifierItem", item)
        .unwrap()
        .serve_at(
            MENU_PATH,
            FakeMenu {
                clicked: clicked.clone(),
            },
        )
        .unwrap()
        .name(name)
        .unwrap()
        .build()
        .unwrap();
    register(&app, name);
    let items = wait_items(&updates, "the item shown", |items| items.len() == 1);
    assert_eq!(items[0].key, name);
    assert_eq!(items[0].title, "Fake");
    assert!(
        matches!(items[0].icon, Some(Icon::Image(_))),
        "its pixmap: {:?}",
        items[0].icon
    );

    // Other hosts (waybar) see it in the watcher's list.
    let reply = app
        .call_method(
            Some(WATCHER),
            WATCHER_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(WATCHER, "RegisteredStatusNotifierItems"),
        )
        .unwrap();
    let listed: OwnedValue = reply.body().deserialize().unwrap();
    assert_eq!(strings(&listed), [name]);

    // Its menu, read ahead of a click.
    let items = wait_items(&updates, "the menu read", |items| {
        items.first().is_some_and(|i| i.layout.is_some())
    });
    assert_eq!(items[0].menu.as_deref(), Some(MENU_PATH));
    let layout = items[0].layout.clone().unwrap();
    let labels: Vec<&str> = layout.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["Open", "More"]);
    assert_eq!(layout[1].children[0].label, "Quit");
    tray.menu_clicked(name, MENU_PATH, 3);
    let deadline = Instant::now() + Duration::from_secs(10);
    while clicked.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "the menu never heard the click");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(*clicked.lock().unwrap(), [3]);

    tray.click(name, Click::Activate);
    let deadline = Instant::now() + Duration::from_secs(10);
    while activated.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline, "never activated");
        std::thread::sleep(Duration::from_millis(10));
    }

    // Passive: hidden, until it's active again.
    *status.lock().unwrap() = "Passive".into();
    app.emit_signal(
        None::<&str>,
        "/StatusNotifierItem",
        ITEM,
        "NewStatus",
        &("Passive",),
    )
    .unwrap();
    wait_items(&updates, "the passive item hidden", |items| {
        items.is_empty()
    });
    *status.lock().unwrap() = "Active".into();
    app.emit_signal(
        None::<&str>,
        "/StatusNotifierItem",
        ITEM,
        "NewStatus",
        &("Active",),
    )
    .unwrap();
    wait_items(&updates, "the item back", |items| items.len() == 1);

    // An item registered by its path (libappindicator's way), from a
    // connection with no name of its own.
    let path = "/org/ayatana/NotificationItem/other";
    let other = bus
        .connect()
        .serve_at(
            path,
            FakeItem {
                status: Arc::new(Mutex::new("Active".into())),
                activated: Arc::new(AtomicU32::new(0)),
            },
        )
        .unwrap()
        .build()
        .unwrap();
    register(&other, path);
    let unique = other.unique_name().unwrap().to_string();
    let items = wait_items(&updates, "both shown", |items| items.len() == 2);
    assert_eq!(items[1].key, format!("{unique}{path}"));

    // Apps that quit take their items along.
    drop(other);
    let items = wait_items(&updates, "the second one gone", |items| items.len() == 1);
    assert_eq!(items[0].key, name);
    drop(app);
    wait_items(&updates, "none left", |items| items.is_empty());
}
