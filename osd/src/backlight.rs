//! The screen's backlight: found in `/sys/class/backlight` (`find`), read
//! for the level shown (`level::read`) and changed by `pila-osd brightness
//! +5%` (`change`): written there where the user may, else through
//! logind's (or elogind's) `SetBrightness`, which needs no permissions of
//! its own, so no brightnessctl.

use std::path::Path;

/// Where the kernel lists the backlights.
pub const CLASS: &str = "/sys/class/backlight";

/// A backlight: its name in the class and its brightness, raw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub now: u64,
    pub max: u64,
}

impl Device {
    /// The brightness in percent, rounded.
    pub fn percent(&self) -> u32 {
        ((self.now * 100 + self.max / 2) / self.max) as u32
    }
}

/// The backlight in `class`: the firmware's first, then the platform's,
/// then a raw one (the kernel's order of preference), the first by name of
/// those; one that can't be read is passed over.
pub fn find(class: &Path) -> Option<Device> {
    let mut devices: Vec<(u8, String)> = std::fs::read_dir(class)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let kind = std::fs::read_to_string(entry.path().join("type")).unwrap_or_default();
            let rank = match kind.trim() {
                "firmware" => 0,
                "platform" => 1,
                _ => 2,
            };
            Some((rank, name))
        })
        .collect();
    devices.sort();
    devices.into_iter().find_map(|(_, name)| {
        let read = |file: &str| -> Option<u64> {
            std::fs::read_to_string(class.join(&name).join(file))
                .ok()?
                .trim()
                .parse()
                .ok()
        };
        let max = read("max_brightness").filter(|&max| max > 0)?;
        let now = read("brightness")?.min(max);
        Some(Device { name, now, max })
    })
}

/// A change asked for: `+5%` or `-5%`, of the maximum.
pub fn parse_step(text: &str) -> Option<i32> {
    let number = text.strip_suffix('%')?;
    let (sign, digits) = match number.as_bytes().first()? {
        b'+' => (1, &number[1..]),
        b'-' => (-1, &number[1..]),
        _ => return None,
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let percent: i32 = digits.parse().ok()?;
    (percent <= 100).then_some(sign * percent)
}

/// The brightness `step` percent of the maximum away from `device`'s
/// (a step of at least one), within the range; never down to off (1 at
/// least, as `brightnessctl -n`), though one that's off stays off.
pub fn stepped(device: &Device, step: i32) -> u64 {
    let amount = ((device.max * u64::from(step.unsigned_abs()) + 50) / 100).max(1);
    if step >= 0 {
        (device.now + amount).min(device.max)
    } else {
        device.now.saturating_sub(amount).max(device.now.min(1))
    }
}

/// Change the backlight in `class` by `step` percent; the device as left.
/// Written to the class where the user may (a udev rule, a group), else
/// asked of logind's session (`set_by_logind`).
pub fn change(class: &Path, step: i32) -> Result<Device, String> {
    let device = find(class).ok_or_else(|| format!("no backlight in {}", class.display()))?;
    let value = stepped(&device, step);
    let file = class.join(&device.name).join("brightness");
    if let Err(write_err) = std::fs::write(&file, format!("{value}\n")) {
        set_by_logind(&device.name, value).map_err(|bus_err| {
            format!(
                "can't set the brightness: {} ({write_err}), nor through logind ({bus_err})",
                file.display()
            )
        })?;
    }
    Ok(Device {
        now: value,
        ..device
    })
}

/// `SetBrightness` of the caller's session (logind's `auto`: the session
/// it runs in, else the user's graphical one), on the system bus.
fn set_by_logind(name: &str, value: u64) -> Result<(), String> {
    let value = u32::try_from(value).map_err(|_| format!("brightness {value} too large"))?;
    let bus = zbus::blocking::Connection::system().map_err(|err| err.to_string())?;
    bus.call_method(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1/session/auto",
        Some("org.freedesktop.login1.Session"),
        "SetBrightness",
        &("backlight", name, value),
    )
    .map(drop)
    .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backlight class of the test's own, in a temporary directory.
    struct Class(std::path::PathBuf);

    impl Class {
        fn new(test: &str) -> Class {
            let dir = std::env::temp_dir()
                .join(format!("pila-osd-backlight-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Class(dir)
        }

        fn device(&self, name: &str, kind: &str, now: u64, max: u64) {
            let dir = self.0.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("type"), format!("{kind}\n")).unwrap();
            std::fs::write(dir.join("brightness"), format!("{now}\n")).unwrap();
            std::fs::write(dir.join("max_brightness"), format!("{max}\n")).unwrap();
        }

        fn brightness(&self, name: &str) -> String {
            std::fs::read_to_string(self.0.join(name).join("brightness")).unwrap()
        }
    }

    impl Drop for Class {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_firmware_backlight_first() {
        let class = Class::new("order");
        assert_eq!(find(&class.0), None);
        class.device("acpi_video0", "raw", 1, 10);
        assert_eq!(find(&class.0).map(|d| d.percent()), Some(10));
        class.device("amdgpu_bl1", "firmware", 128, 255);
        let device = find(&class.0).unwrap();
        assert_eq!((device.name.as_str(), device.percent()), ("amdgpu_bl1", 50));
        // A broken one is passed over.
        class.device("aaa", "firmware", 1, 0);
        assert_eq!(find(&class.0).unwrap().name, "amdgpu_bl1");
    }

    #[test]
    fn steps() {
        for (text, step) in [("+5%", Some(5)), ("-10%", Some(-10)), ("+100%", Some(100))] {
            assert_eq!(parse_step(text), step, "{text:?}");
        }
        for wrong in ["5%", "+5", "+%", "+-5%", "+101%", "+5.5%", "", "%", "up"] {
            assert_eq!(parse_step(wrong), None, "{wrong:?}");
        }
        let device = |now, max| Device {
            name: "bl".into(),
            now,
            max,
        };
        assert_eq!(stepped(&device(128, 255), 5), 141);
        assert_eq!(stepped(&device(128, 255), -5), 115);
        assert_eq!(stepped(&device(250, 255), 5), 255);
        // Never quite off, but one that's off stays off.
        assert_eq!(stepped(&device(5, 255), -5), 1);
        assert_eq!(stepped(&device(1, 255), -5), 1);
        assert_eq!(stepped(&device(0, 255), -5), 0);
        // A step of at least one on a backlight with few levels.
        assert_eq!(stepped(&device(3, 7), 5), 4);
        assert_eq!(stepped(&device(3, 7), -5), 2);
    }

    #[test]
    fn written_where_the_user_may() {
        let class = Class::new("write");
        class.device("amdgpu_bl1", "firmware", 128, 255);
        let device = change(&class.0, 5).unwrap();
        assert_eq!((device.now, device.percent()), (141, 55));
        assert_eq!(class.brightness("amdgpu_bl1"), "141\n");
        change(&class.0, -10).unwrap();
        assert_eq!(class.brightness("amdgpu_bl1"), "115\n");
    }

    #[test]
    fn no_backlight_is_an_error() {
        let class = Class::new("none");
        let err = change(&class.0, 5).unwrap_err();
        assert!(err.contains("no backlight"), "{err}");
    }
}
