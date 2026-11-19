//! A session bus of the test's own (a private dbus-daemon) with a
//! notification daemon on it that records what Pila sends, as mako would
//! show it: give Pila the bus with `Options::env`
//! (`DBUS_SESSION_BUS_ADDRESS`, `Bus::address`).

use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Instant,
};

use zbus::{blocking::Connection, zvariant::OwnedValue};

use super::TIMEOUT;

/// A notification as the daemon got it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub expire_timeout: i32,
}

/// A private session bus, gone when dropped.
pub struct Bus {
    daemon: Child,
    pub address: String,
}

impl Bus {
    pub fn start() -> Bus {
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon (package dbus)");
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Bus {
            daemon,
            address: address.trim().to_string(),
        }
    }

    /// The bus for Pila: `Options::env`'s entry.
    pub fn env(&self) -> (String, String) {
        ("DBUS_SESSION_BUS_ADDRESS".into(), self.address.clone())
    }

    /// A notification daemon on this bus, recording, until it's dropped.
    pub fn notification_daemon(&self) -> Daemon {
        let shown = Arc::new(Mutex::new(Vec::new()));
        let connection = zbus::blocking::connection::Builder::address(self.address.as_str())
            .unwrap()
            .name("org.freedesktop.Notifications")
            .unwrap()
            .serve_at(
                "/org/freedesktop/Notifications",
                FakeDaemon {
                    shown: shown.clone(),
                },
            )
            .unwrap()
            .build()
            .unwrap();
        Daemon {
            _connection: connection,
            shown,
        }
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

/// A running notification daemon of the test's.
pub struct Daemon {
    _connection: Connection,
    shown: Arc<Mutex<Vec<Shown>>>,
}

impl Daemon {
    /// The notifications shown so far.
    pub fn shown(&self) -> Vec<Shown> {
        self.shown.lock().unwrap().clone()
    }

    /// Wait for the `count`th notification; all of them so far.
    pub fn wait_for(&self, count: usize) -> Vec<Shown> {
        let started = Instant::now();
        loop {
            let shown = self.shown();
            if shown.len() >= count {
                return shown;
            }
            assert!(
                started.elapsed() < TIMEOUT,
                "{} notifications, not {count}: {shown:#?}",
                shown.len()
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

struct FakeDaemon {
    shown: Arc<Mutex<Vec<Shown>>>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl FakeDaemon {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        _replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        _hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let mut shown = self.shown.lock().unwrap();
        shown.push(Shown {
            app_name,
            summary,
            body,
            expire_timeout,
        });
        shown.len() as u32
    }
}
