//! `pila-launcher`: an app launcher for Pila (or any compositor with the
//! layer shell). A layer surface (an Overlay that takes the keyboard)
//! listing the installed apps, searched as you type (`ui.rs`, iced).
//!
//! The first run stays in the background once shown, listening on a socket
//! in the runtime directory (`exec::socket_path`, one per Wayland display);
//! a later run sends it a `Request` (show or hide, and the terminal) and
//! exits, so after the first time it opens at once.

mod apps;
mod exec;
mod icons;
mod ui;

use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::Path,
    process::ExitCode,
    time::Duration,
};

use exec::{Exit, Request};

fn main() -> ExitCode {
    let args = match exec::parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(Exit::Help) => {
            print!("{}", exec::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(Exit::Version) => {
            println!("pila-launcher {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(Exit::Error(err)) => {
            eprintln!("pila-launcher: {err}\n\n{}", exec::USAGE);
            return ExitCode::from(2);
        }
    };
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok();
    let display = std::env::var("WAYLAND_DISPLAY").ok();
    let Some(path) = exec::socket_path(runtime_dir.as_deref(), display.as_deref()) else {
        eprintln!("pila-launcher: XDG_RUNTIME_DIR isn't set");
        return ExitCode::FAILURE;
    };
    let request = Request {
        terminal: args.terminal,
        toggle: true,
    };
    // Twice: two runs at once may both find no one listening, and one of
    // them binds first.
    for _ in 0..2 {
        if send(&path, &request) {
            return ExitCode::SUCCESS;
        }
        // No one listening: a socket left by a run that died, if any.
        let _ = std::fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => return serve(listener, &path, request),
            Err(err) => eprintln!("pila-launcher: can't listen on {}: {err}", path.display()),
        }
    }
    ExitCode::FAILURE
}

/// Give `request` to the run listening on `path`, if one is.
fn send(path: &Path, request: &Request) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    stream.write_all(request.encode().as_bytes()).is_ok()
}

/// Be the run in the background: show the launcher (`first` asks for it)
/// and take the requests of later runs.
fn serve(listener: UnixListener, path: &Path, first: Request) -> ExitCode {
    // Pila's per-command output of the first run means nothing for the apps
    // started later on.
    // SAFETY: no other thread runs yet.
    unsafe { std::env::remove_var("PILA_OUTPUT") };
    let status = ui::run(listener, first);
    let _ = std::fs::remove_file(path);
    status
}

/// A request, sent all at once by `send` (a stuck sender is given up on).
fn read_request(mut stream: UnixStream) -> Option<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    let mut text = String::new();
    (&mut stream)
        .take(64 * 1024)
        .read_to_string(&mut text)
        .ok()?;
    Some(Request::decode(&text))
}
