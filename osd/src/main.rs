//! `pila-osd`: Pila's on-screen display. `pila-osd volume|mic|brightness`
//! reads that level (`level.rs`) and shows it for a moment at the bottom of
//! the monitor (`ui.rs`, iced): run by the bindings of the volume and
//! brightness keys right after they change it (`config.example.toml`).
//! `pila-osd brightness +5%` changes the backlight first (`backlight.rs`),
//! and `pila-osd play-pause|next|previous|stop` tells the media player and
//! shows what it did, with the track (`media.rs`): the keys need no other
//! program but wpctl. Either is a `Popup`.
//!
//! The first run stays in the background once shown, listening on a socket
//! in the runtime directory (`socket_path`, one per Wayland display); a
//! later run hands it the popup and exits, so a held key's every repeat
//! shows at once.

mod backlight;
mod glyphs;
#[path = "../../bar/src/icon_font.rs"]
mod icon_font;
mod level;
mod media;
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
use media::Media;

/// What the popup shows: a level, or what a media key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    Level(Level),
    Media(Media),
}

impl Popup {
    /// The popup as a request to the run in the background: one line.
    fn encode(&self) -> String {
        match self {
            Popup::Level(level) => level.encode(),
            Popup::Media(media) => media.encode(),
        }
    }

    fn decode(text: &str) -> Option<Popup> {
        Media::decode(text)
            .map(Popup::Media)
            .or_else(|| Level::decode(text).map(Popup::Level))
    }
}

const USAGE: &str = "\
Usage: pila-osd volume|mic|brightness
       pila-osd brightness +N%|-N%
       pila-osd play-pause|next|previous|stop
       pila-osd --help | --version

Shows the volume, the microphone's volume or the screen's brightness for a
moment at the bottom of the monitor: the default bindings of the volume and
brightness keys run it after changing them. The volume and the microphone
are read with wpctl (PipeWire), the brightness from /sys/class/backlight.

brightness +N% or -N% changes the brightness by N% of its maximum first,
never quite to off: written to /sys/class/backlight where you may, else
through logind (or elogind).

play-pause, next, previous and stop tell the media player that's playing
(else a paused one) over MPRIS, and show what it did with the track's title
and artist.
";

/// What a run is asked to do.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    Help,
    Version,
    Show(Kind),
    /// Change the brightness by this many percent, then show it.
    ChangeBrightness(i32),
    Media(media::Command),
}

fn parse(args: &[String]) -> Result<Request, String> {
    match args {
        [flag] if flag == "-h" || flag == "--help" => Ok(Request::Help),
        [flag] if flag == "-V" || flag == "--version" => Ok(Request::Version),
        [name] => Kind::named(name)
            .map(Request::Show)
            .or_else(|| media::Command::named(name).map(Request::Media))
            .ok_or_else(|| format!("unknown level {name:?}")),
        [name, step] if name == "brightness" => backlight::parse_step(step)
            .map(Request::ChangeBrightness)
            .ok_or_else(|| format!("not a change of brightness: {step:?}")),
        _ => Err("one level to show".into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let read = match parse(&args) {
        Err(err) => return usage_error(&err),
        Ok(Request::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Request::Version) => {
            println!("pila-osd {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(Request::Media(command)) => media::run(command).map(Popup::Media),
        Ok(Request::Show(kind)) => level::read(kind).map(Popup::Level),
        Ok(Request::ChangeBrightness(step)) => backlight::change(Path::new(backlight::CLASS), step)
            .map(|d| Popup::Level(Level::brightness(&d))),
    };
    let popup = match read {
        Ok(popup) => popup,
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
        if send(&path, &popup) {
            return ExitCode::SUCCESS;
        }
        // No one listening: a socket left by a run that died, if any.
        let _ = std::fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => {
                let status = ui::run(listener, popup);
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

/// Give `popup` to the run listening on `path`, if one is.
fn send(path: &Path, popup: &Popup) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    stream.write_all(popup.encode().as_bytes()).is_ok()
}

/// A popup from a later run, sent all at once (a stuck sender is given up on).
fn read_request(stream: UnixStream) -> Option<Popup> {
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    let mut text = String::new();
    stream.take(1024).read_to_string(&mut text).ok()?;
    Popup::decode(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests() {
        let parsed = |args: &[&str]| parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert_eq!(parsed(&["volume"]), Ok(Request::Show(Kind::Volume)));
        assert_eq!(parsed(&["brightness"]), Ok(Request::Show(Kind::Brightness)));
        assert_eq!(
            parsed(&["brightness", "-5%"]),
            Ok(Request::ChangeBrightness(-5))
        );
        assert_eq!(
            parsed(&["play-pause"]),
            Ok(Request::Media(media::Command::PlayPause))
        );
        assert_eq!(parsed(&["--version"]), Ok(Request::Version));
        for wrong in [
            &["loudness"][..],
            &["volume", "+5%"],
            &["brightness", "5%"],
            &[],
            &["mic", "mic", "mic"],
        ] {
            assert!(parsed(wrong).is_err(), "{wrong:?}");
        }
    }

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
