//! `pila-wallpaper`: the image of Pila's `[wallpaper]` behind everything, on
//! every monitor (`ui.rs`), followed as the user saves the config: Pila sends
//! it over its IPC (`wallpaper` events, `docs/IPC.md`), with the colors for
//! what an image that doesn't fill a monitor leaves around it. With no image
//! set it shows nothing, and `[colors] background` shows.
//!
//! Started with the session (`autostart`), it needs Pila's `PILA_SOCKET`. Any
//! other wallpaper program can take its place there.

#[allow(dead_code)] // pila-bar's: every output is wanted here.
#[path = "../../bar/src/outputs.rs"]
mod outputs;
mod picture;
#[path = "../../launcher/src/pila_ipc.rs"]
mod pila_ipc;
mod screens;
#[path = "../../launcher/src/style.rs"]
mod style;
mod ui;

use std::process::ExitCode;

const USAGE: &str = "\
Usage: pila-wallpaper [--help | --version]

Shows the image of [wallpaper] in Pila's config behind everything, on every
monitor, and follows it as the config is saved. Started with the session
(autostart in Pila's config); it needs Pila's PILA_SOCKET.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {}
        Some("-h" | "--help") => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("-V" | "--version") => {
            println!("pila-wallpaper {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("pila-wallpaper: unknown argument {other:?}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    }
    if std::env::var_os("PILA_SOCKET").is_none_or(|p| p.is_empty()) {
        eprintln!("pila-wallpaper: PILA_SOCKET isn't set: it runs in a Pila session");
        return ExitCode::FAILURE;
    }
    ui::run()
}
