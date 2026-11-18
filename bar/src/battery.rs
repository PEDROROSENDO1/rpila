//! The battery: the system's batteries in `/sys/class/power_supply` (not a
//! mouse's or a headset's, whose `scope` is `Device`), read on a thread of
//! its own every few seconds, which wakes the bar only when what it shows
//! changes. No battery, no thread.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

/// Where the kernel lists power supplies.
pub const SYSFS: &str = "/sys/class/power_supply";
/// How often the batteries are read.
const EVERY: Duration = Duration::from_secs(5);
/// At or below this, discharging, the battery asks for attention.
pub const LOW: u8 = 15;

/// What the bar shows of the batteries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    /// How full, 0 to 100 (all batteries together).
    pub percent: u8,
    pub state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Charging,
    Discharging,
    /// Plugged in and not charging: full, or held below full.
    Plugged,
}

impl Battery {
    /// Low and running down.
    pub fn low(&self) -> bool {
        self.state == State::Discharging && self.percent <= LOW
    }

    /// The text the bar shows: the percentage, an arrow up while charging.
    pub fn text(&self) -> String {
        match self.state {
            State::Charging => format!("\u{2191}{}%", self.percent),
            _ => format!("{}%", self.percent),
        }
    }
}

/// One battery as sysfs has it.
#[derive(Debug, Default)]
struct Supply {
    capacity: Option<u8>,
    /// What it holds now and when full, in its own unit (µWh or µAh).
    now: Option<u64>,
    full: Option<u64>,
    status: String,
}

fn read_text(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name))
        .ok()
        .map(|s| s.trim().to_string())
}

fn read_number<T: std::str::FromStr>(dir: &Path, name: &str) -> Option<T> {
    read_text(dir, name)?.parse().ok()
}

/// The system's batteries under `root` (a mouse's left out), by path.
fn batteries(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|dir| read_text(dir, "type").as_deref() == Some("Battery"))
        .filter(|dir| read_text(dir, "scope").as_deref() != Some("Device"))
        // A slot with no battery in it.
        .filter(|dir| read_text(dir, "present").as_deref() != Some("0"))
        .collect();
    found.sort();
    found
}

fn supply(dir: &Path) -> Supply {
    let energy = read_number(dir, "energy_now").zip(read_number(dir, "energy_full"));
    let (now, full) = energy
        .or_else(|| read_number(dir, "charge_now").zip(read_number(dir, "charge_full")))
        .unzip();
    Supply {
        capacity: read_number(dir, "capacity"),
        now,
        full,
        status: read_text(dir, "status").unwrap_or_default(),
    }
}

/// The batteries together: how full (by what they hold, where each says;
/// else their capacities' average) and charging if any is, discharging if
/// any is, else plugged. None with no battery.
fn combine(supplies: &[Supply]) -> Option<Battery> {
    if supplies.is_empty() {
        return None;
    }
    let held: Option<(u64, u64)> = supplies
        .iter()
        .map(|s| s.now.zip(s.full))
        .try_fold((0, 0), |(now, full), s| {
            s.map(|(n, f)| (now + n.min(f), full + f))
        });
    let percent = match held {
        Some((now, full)) if full > 0 => (now * 100 + full / 2) / full,
        _ => {
            let capacities: Vec<u64> = supplies
                .iter()
                .filter_map(|s| s.capacity.map(u64::from))
                .collect();
            if capacities.is_empty() {
                return None;
            }
            capacities.iter().sum::<u64>() / capacities.len() as u64
        }
    };
    let any = |status: &str| supplies.iter().any(|s| s.status == status);
    let state = if any("Charging") {
        State::Charging
    } else if any("Discharging") {
        State::Discharging
    } else {
        State::Plugged
    };
    Some(Battery {
        percent: percent.min(100) as u8,
        state,
    })
}

/// The batteries under `root` now.
pub fn read(root: &Path) -> Option<Battery> {
    let supplies: Vec<Supply> = batteries(root).iter().map(|dir| supply(dir)).collect();
    combine(&supplies)
}

/// The battery each time what the bar shows of it changes, from a thread of
/// its own; nothing (and no thread) on a machine without one.
pub fn updates() -> impl iced::futures::Stream<Item = Battery> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let root = PathBuf::from(SYSFS);
    if let Some(first) = read(&root) {
        let _ = sender.unbounded_send(first);
        let spawned = std::thread::Builder::new()
            .name("pila-bar-battery".into())
            .spawn(move || {
                let mut shown = first;
                loop {
                    std::thread::sleep(EVERY);
                    // Gone (unplugged from a dock): what it said last stays.
                    let Some(battery) = read(&root) else { continue };
                    if battery != shown {
                        if sender.unbounded_send(battery).is_err() {
                            break;
                        }
                        shown = battery;
                    }
                }
            });
        if let Err(err) = spawned {
            eprintln!("pila-bar: no battery: {err}");
        }
    }
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A power_supply directory of its own, removed when dropped.
    struct Sysfs(PathBuf);

    impl Sysfs {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("pila-bar-battery-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Sysfs(root)
        }

        fn supply(&self, name: &str, files: &[(&str, &str)]) {
            let dir = self.0.join(name);
            fs::create_dir_all(&dir).unwrap();
            for (file, text) in files {
                fs::write(dir.join(file), format!("{text}\n")).unwrap();
            }
        }
    }

    impl Drop for Sysfs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_laptop_battery_not_a_mouse() {
        let sysfs = Sysfs::new("laptop");
        sysfs.supply("ACAD", &[("type", "Mains"), ("online", "0")]);
        sysfs.supply(
            "BAT0",
            &[
                ("type", "Battery"),
                ("status", "Discharging"),
                ("capacity", "41"),
                ("charge_now", "2000000"),
                ("charge_full", "4800000"),
            ],
        );
        sysfs.supply(
            "hidpp_battery_0",
            &[
                ("type", "Battery"),
                ("scope", "Device"),
                ("status", "Charging"),
                ("capacity", "5"),
            ],
        );
        let battery = read(&sysfs.0).unwrap();
        assert_eq!(battery.percent, 42, "by what it holds: 2.0 of 4.8");
        assert_eq!(battery.state, State::Discharging);
        assert_eq!(battery.text(), "42%");
        assert!(!battery.low());
    }

    #[test]
    fn two_batteries_by_what_they_hold() {
        let sysfs = Sysfs::new("two");
        for (name, now, full, status) in [
            ("BAT0", "10000000", "20000000", "Discharging"),
            ("BAT1", "30000000", "60000000", "Charging"),
        ] {
            sysfs.supply(
                name,
                &[
                    ("type", "Battery"),
                    ("status", status),
                    ("energy_now", now),
                    ("energy_full", full),
                ],
            );
        }
        // An empty bay.
        sysfs.supply("BAT2", &[("type", "Battery"), ("present", "0")]);
        let battery = read(&sysfs.0).unwrap();
        assert_eq!(battery.percent, 50);
        assert_eq!(battery.state, State::Charging);
        assert_eq!(battery.text(), "\u{2191}50%");
    }

    #[test]
    fn capacity_where_nothing_else_says() {
        let sysfs = Sysfs::new("capacity");
        sysfs.supply(
            "BAT0",
            &[
                ("type", "Battery"),
                ("status", "Discharging"),
                ("capacity", "12"),
            ],
        );
        let battery = read(&sysfs.0).unwrap();
        assert_eq!(
            battery,
            Battery {
                percent: 12,
                state: State::Discharging
            }
        );
        assert!(battery.low());
        sysfs.supply("BAT0", &[("status", "Not charging"), ("capacity", "100")]);
        let battery = read(&sysfs.0).unwrap();
        assert_eq!(
            battery,
            Battery {
                percent: 100,
                state: State::Plugged
            }
        );
        assert!(!battery.low());
    }

    #[test]
    fn no_battery_nothing_shown() {
        let sysfs = Sysfs::new("desktop");
        sysfs.supply("ACAD", &[("type", "Mains")]);
        assert_eq!(read(&sysfs.0), None);
        assert_eq!(read(&sysfs.0.join("missing")), None);
        // A battery that says nothing of its charge.
        sysfs.supply("BAT0", &[("type", "Battery"), ("status", "Unknown")]);
        assert_eq!(read(&sysfs.0), None);
    }
}
