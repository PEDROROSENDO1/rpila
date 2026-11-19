//! `pila-osd`: Pila's on-screen display. `pila-osd volume|mic|brightness`
//! reads that level (`level.rs`) and shows it for a moment at the bottom of
//! the monitor (`ui.rs`, iced): run by the bindings of the volume and
//! brightness keys right after they change it (`config.example.toml`).
//!
//! The first run stays in the background once shown, listening on a socket
//! in the runtime directory (`socket_path`, one per Wayland display); a
//! later run hands it the level and exits, so a held key's every repeat
//! shows at once.

mod glyphs;
#[path = "../../bar/src/icon_font.rs"]
mod icon_font;
mod level;
#[path = "../../launcher/src/pila_ipc.rs"]
mod pila_ipc;
#[path = "../../launcher/src/style.rs"]
mod style;
mod ui;

use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use level::{Kind, Level};

const USAGE: &str = "\
Usage: pila-osd volume|mic|brightness
       pila-osd --help | --version

Shows the volume, the microphone's volume or the screen's brightness for a
moment at the bottom of the monitor: the default bindings of the volume and
brightness keys run it after changing them. The volume and the microphone
are read with wpctl (PipeWire), the brightness from /sys/class/backlight.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = match args.as_slice() {
        [flag] if flag == "-h" || flag == "--help" => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        [flag] if flag == "-V" || flag == "--version" => {
            println!("pila-osd {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        [name] => match Kind::named(name) {
            Some(kind) => kind,
            None => return usage_error(&format!("unknown level {name:?}")),
        },
        _ => return usage_error("one level to show"),
    };
    let level = match level::read(kind) {
        Ok(level) => level,
        Err(err) => {
            eprintln!("pila-osd: {err}");
            return ExitCode::FAILURE;
        }
    };
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok();
    let display = std::env::var("WAYLAND_DISPLAY").ok();
    let Some(path) = socket_path(runtime_dir.as_deref(), display.as_deref()) else {
        eprintln!("pila-osd: XDG_RUNTIME_DIR isn't set");
        return ExitCode::FAILURE;
    };
    // Twice: two runs at once may both find no one listening, and one of
    // them binds first.
    for _ in 0..2 {
        if send(&path, &level) {
            return ExitCode::SUCCESS;
        }
        // No one listening: a socket left by a run that died, if any.
        let _ = std::fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => {
                let status = ui::run(listener, level);
                let _ = std::fs::remove_file(&path);
                return status;
            }
            Err(err) => eprintln!("pila-osd: can't listen on {}: {err}", path.display()),
        }
    }
    ExitCode::FAILURE
}

fn usage_error(err: &str) -> ExitCode {
    eprintln!("pila-osd: {err}\n\n{USAGE}");
    ExitCode::from(2)
}

/// The socket of the run in the background: one per Wayland display, in
/// the runtime directory.
fn socket_path(runtime_dir: Option<&str>, display: Option<&str>) -> Option<PathBuf> {
    let runtime_dir = runtime_dir.filter(|d| !d.is_empty())?;
    // WAYLAND_DISPLAY may be a path: its last part names it.
    let display = display
        .filter(|d| !d.is_empty())
        .and_then(|d| Path::new(d).file_name()?.to_str().map(str::to_string))
        .unwrap_or_else(|| "wayland-0".to_string());
    Some(Path::new(runtime_dir).join(format!("pila-osd-{display}.sock")))
}

/// Give `level` to the run listening on `path`, if one is.
fn send(path: &Path, level: &Level) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    stream.write_all(level.encode().as_bytes()).is_ok()
}

/// A level from a later run, sent all at once (a stuck sender is given up on).
fn read_request(stream: UnixStream) -> Option<Level> {
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    let mut text = String::new();
    stream.take(1024).read_to_string(&mut text).ok()?;
    Level::decode(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_socket_per_display() {
        assert_eq!(
            socket_path(Some("/run/user/1000"), Some("wayland-1")),
            Some(PathBuf::from("/run/user/1000/pila-osd-wayland-1.sock"))
        );
        assert_eq!(
            socket_path(Some("/run/user/1000"), Some("/tmp/sock/wayland-9")),
            Some(PathBuf::from("/run/user/1000/pila-osd-wayland-9.sock"))
        );
        assert_eq!(
            socket_path(Some("/run/user/1000"), None),
            Some(PathBuf::from("/run/user/1000/pila-osd-wayland-0.sock"))
        );
        assert_eq!(socket_path(None, Some("wayland-1")), None);
    }
}
