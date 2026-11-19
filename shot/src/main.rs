//! `pila-shot`: Pila's screenshots, with nothing else installed. A monitor
//! (`output`) or a region picked on one (`region`, `ui.rs`) as Pila shows
//! it (`capture.rs`: ext-image-copy-capture), to a PNG file or the clipboard
//! (`save.rs`, `clipboard.rs`: ext-data-control); and `pick-output`, the
//! monitor to share, for xdg-desktop-portal-wlr's chooser
//! (`resources/xdg/xdg-desktop-portal-wlr/pila`).
//!
//! The clipboard is served by a process of its own (`serve-clipboard`,
//! started detached with the PNG on its stdin): the first one exits once the
//! picture is offered, the second when something else takes the clipboard.

mod capture;
mod clipboard;
#[allow(dead_code)] // pila-bar's: the local time is all that's used here.
#[path = "../../bar/src/clock.rs"]
mod clock;
#[path = "../../launcher/src/pila_ipc.rs"]
mod pila_ipc;
mod save;
#[path = "../../switcher/src/shm.rs"]
mod shm;
#[path = "../../launcher/src/style.rs"]
mod style;
mod ui;

use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, ExitCode, Stdio},
};

use capture::Shot;

const USAGE: &str = "\
Usage: pila-shot output [NAME] [--file PATH | --clipboard]
       pila-shot region [--file PATH | --clipboard]
       pila-shot pick-output
       pila-shot [--help | --version]

Screenshots in Pila, without the cursor.

  output       a whole monitor: NAME, else the one Pila runs this on
               ($PILA_OUTPUT: the pointer's), else the first; by default to
               a file in the pictures folder (XDG_PICTURES_DIR, else
               ~/Pictures), named by the time, whose path it prints
  region       a rectangle dragged on a monitor (Escape or a right click
               cancels); by default to the clipboard, as image/png
  pick-output  click a monitor: prints \"Monitor: NAME\", for
               xdg-desktop-portal-wlr's chooser (screen sharing)

  --file PATH  write the PNG to PATH (- for stdout)
  --clipboard  put it on the clipboard (served until something else is
               copied)

Exits 1 if cancelled or if the picture can't be taken.
";

/// Where a picture goes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Destination {
    Pictures,
    File(PathBuf),
    Stdout,
    Clipboard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Request {
    Output(Option<String>, Destination),
    Region(Destination),
    PickOutput,
    ServeClipboard,
    Help,
    Version,
}

fn parse(args: &[String]) -> Result<Request, String> {
    let mut args = args.iter().map(String::as_str);
    let command = args
        .next()
        .ok_or("what to take: output, region or pick-output")?;
    let mut destination = None;
    let mut rest = Vec::new();
    while let Some(arg) = args.next() {
        match arg {
            "--file" => {
                let path = args.next().ok_or("--file needs a path")?;
                destination = Some(match path {
                    "-" => Destination::Stdout,
                    path => Destination::File(PathBuf::from(path)),
                });
            }
            "--clipboard" => destination = Some(Destination::Clipboard),
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option {other:?}"))
            }
            other => rest.push(other.to_string()),
        }
    }
    let no_more = |rest: &[String]| match rest.first() {
        Some(extra) => Err(format!("unexpected {extra:?}")),
        None => Ok(()),
    };
    match command {
        "-h" | "--help" => Ok(Request::Help),
        "-V" | "--version" => Ok(Request::Version),
        "output" => {
            if rest.len() > 1 {
                no_more(&rest[1..])?;
            }
            Ok(Request::Output(
                rest.into_iter().next(),
                destination.unwrap_or(Destination::Pictures),
            ))
        }
        "region" => {
            no_more(&rest)?;
            Ok(Request::Region(
                destination.unwrap_or(Destination::Clipboard),
            ))
        }
        "pick-output" => {
            no_more(&rest)?;
            match destination {
                Some(_) => Err("pick-output takes no --file or --clipboard".into()),
                None => Ok(Request::PickOutput),
            }
        }
        "serve-clipboard" => Ok(Request::ServeClipboard),
        other => Err(format!("unknown command {other:?}")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse(&args) {
        Ok(command) => command,
        Err(err) => {
            eprintln!("pila-shot: {err}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = match command {
        Request::Help => {
            print!("{USAGE}");
            Ok(())
        }
        Request::Version => {
            println!("pila-shot {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Request::Output(name, destination) => output(name, &destination),
        Request::Region(destination) => region(&destination),
        Request::PickOutput => pick_output(),
        Request::ServeClipboard => serve_clipboard(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("pila-shot: {err}");
            ExitCode::FAILURE
        }
    }
}

fn output(name: Option<String>, destination: &Destination) -> Result<(), String> {
    let name = name.or_else(|| std::env::var("PILA_OUTPUT").ok().filter(|n| !n.is_empty()));
    let only: Vec<String> = name.into_iter().collect();
    let shot = capture::outputs(&only)?.swap_remove(0);
    deliver(&shot, destination)
}

fn region(destination: &Destination) -> Result<(), String> {
    let shots = capture::outputs(&[])?;
    match ui::run(ui::Mode::Region, &shots) {
        ui::Outcome::Region(index, fractions) => {
            let shot = &shots[index];
            let rect = save::rect_of(fractions, shot.width, shot.height);
            deliver(&save::crop(shot, rect), destination)
        }
        _ => Err("cancelled".into()),
    }
}

fn pick_output() -> Result<(), String> {
    let shots = capture::outputs(&[])?;
    match ui::run(ui::Mode::Output, &shots) {
        ui::Outcome::Output(index) => {
            println!("Monitor: {}", shots[index].output);
            Ok(())
        }
        _ => Err("cancelled".into()),
    }
}

/// `shot` to `destination`, as PNG.
fn deliver(shot: &Shot, destination: &Destination) -> Result<(), String> {
    let png = save::png(shot)?;
    match destination {
        Destination::Pictures => {
            let dir = save::pictures_dir().ok_or("HOME isn't set: no pictures folder")?;
            let path = dir.join(save::file_name(&clock::now("%Y%m%d-%H%M%S")));
            save::write(&path, &png)?;
            println!("{}", path.display());
            Ok(())
        }
        Destination::File(path) => {
            save::write(path, &png)?;
            println!("{}", path.display());
            Ok(())
        }
        Destination::Stdout => std::io::stdout()
            .write_all(&png)
            .map_err(|err| format!("can't write the PNG: {err}")),
        Destination::Clipboard => to_clipboard(&png),
    }
}

/// Hand `png` to a `serve-clipboard` process of its own, detached (a session
/// of its own: a terminal closing doesn't end it), and wait until it's
/// offered.
fn to_clipboard(png: &[u8]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| format!("can't find pila-shot: {err}"))?;
    let mut command = Command::new(exe);
    command
        .arg("serve-clipboard")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("can't serve the clipboard: {err}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("no pipe to the clipboard's process")?;
    stdin
        .write_all(png)
        .map_err(|err| format!("can't hand the picture over: {err}"))?;
    drop(stdin);
    let stdout = child
        .stdout
        .take()
        .ok_or("no pipe from the clipboard's process")?;
    let mut line = String::new();
    let _ = BufReader::new(stdout).read_line(&mut line);
    if line.trim() == "offered" {
        return Ok(());
    }
    // It failed and said why on stderr.
    let _ = child.wait();
    Err("the picture isn't on the clipboard".into())
}

/// The clipboard's process: the PNG from stdin, served until replaced;
/// "offered" on stdout once it's on the clipboard, which ends the first
/// process.
fn serve_clipboard() -> Result<(), String> {
    let mut png = Vec::new();
    std::io::stdin()
        .read_to_end(&mut png)
        .map_err(|err| format!("can't read the picture: {err}"))?;
    clipboard::serve(png, || {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "offered");
        let _ = out.flush();
        // Nothing more goes there: the first process stops waiting.
        // SAFETY: closing our own stdout and stdin, no longer used.
        unsafe {
            libc::close(1);
            libc::close(0);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &str) -> Result<Request, String> {
        let args: Vec<String> = args.split_whitespace().map(String::from).collect();
        parse(&args)
    }

    #[test]
    fn commands_and_their_destinations() {
        assert_eq!(
            parsed("output"),
            Ok(Request::Output(None, Destination::Pictures))
        );
        assert_eq!(
            parsed("output DP-1 --clipboard"),
            Ok(Request::Output(Some("DP-1".into()), Destination::Clipboard))
        );
        assert_eq!(
            parsed("output --file /tmp/a.png"),
            Ok(Request::Output(
                None,
                Destination::File("/tmp/a.png".into())
            ))
        );
        assert_eq!(
            parsed("region"),
            Ok(Request::Region(Destination::Clipboard))
        );
        assert_eq!(
            parsed("region --file -"),
            Ok(Request::Region(Destination::Stdout))
        );
        assert_eq!(parsed("pick-output"), Ok(Request::PickOutput));
        assert_eq!(parsed("--version"), Ok(Request::Version));
    }

    #[test]
    fn bad_arguments_say_why() {
        assert!(parsed("").is_err());
        assert!(parsed("window").unwrap_err().contains("window"));
        assert!(parsed("region DP-1").unwrap_err().contains("DP-1"));
        assert!(parsed("output A B").unwrap_err().contains('B'));
        assert!(parsed("output --file").unwrap_err().contains("--file"));
        assert!(parsed("pick-output --clipboard").is_err());
        assert!(parsed("region --bogus").unwrap_err().contains("--bogus"));
    }
}
