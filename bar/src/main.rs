//! `pila-bar`: a bar for Pila (or any compositor with the layer shell and
//! ext-workspace). A layer surface on every monitor showing the workspaces,
//! the battery and a clock (`ui.rs`, iced).

mod args;
mod battery;
mod clock;
mod desktop;
mod outputs;
#[path = "../../launcher/src/style.rs"]
mod style;
mod ui;

use std::process::ExitCode;

use args::Exit;

fn main() -> ExitCode {
    let args = match args::parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(Exit::Help) => {
            print!("{}", args::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(Exit::Version) => {
            println!("pila-bar {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(Exit::Error(err)) => {
            eprintln!("pila-bar: {err}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    // The clock in the user's language (LC_TIME, or LC_ALL / LANG); the rest
    // of the C library stays in "C".
    // SAFETY: no other thread runs yet, and the argument is a C string.
    unsafe { libc::setlocale(libc::LC_TIME, c"".as_ptr()) };
    ui::run(args)
}
