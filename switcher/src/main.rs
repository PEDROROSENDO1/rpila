//! `pila-switcher`: the windows of a Mod+Tab (or Alt+Tab) in Pila, shown
//! while it's held as previews, the one it's on highlighted. Pila does the switching (the
//! keys, the order, the end when they're let go) and tells it over its IPC
//! (`switch` events, `docs/IPC.md`); this only shows it (`ui.rs`, iced), with
//! a still of each window (`previews.rs`), and a click on a window ends the
//! switch there.
//!
//! A daemon with no surface while no switch goes on: started with the
//! session (`autostart`), it needs Pila's `PILA_SOCKET`.

#[allow(dead_code)] // The launcher's: the entries and their icons are used here.
#[path = "../../launcher/src/apps.rs"]
mod apps;
#[allow(dead_code)]
#[path = "../../launcher/src/icons.rs"]
mod icons;
mod matching;
mod model;
#[path = "../../launcher/src/pila_ipc.rs"]
mod pila_ipc;
mod previews;
#[path = "../../launcher/src/style.rs"]
mod style;
mod ui;

use std::process::ExitCode;

const USAGE: &str = "\
Usage: pila-switcher [--help | --version]

Shows the windows of a Mod+Tab or Alt+Tab in Pila while it's held, the one it
is on highlighted; a click on one switches to it. Started with the session
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
            println!("pila-switcher {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("pila-switcher: unknown argument {other:?}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    }
    if std::env::var_os("PILA_SOCKET").is_none_or(|p| p.is_empty()) {
        eprintln!("pila-switcher: PILA_SOCKET isn't set: it runs in a Pila session");
        return ExitCode::FAILURE;
    }
    ui::run()
}
