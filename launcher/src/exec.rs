//! The command line, the messages between runs, where they meet, and the
//! commands that start an app.

use std::path::PathBuf;

/// What `pila-launcher` was asked to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// The terminal for apps with `Terminal=true` (else `$TERMINAL`).
    pub terminal: Option<String>,
}

pub const USAGE: &str = "\
Usage: pila-launcher [--terminal COMMAND]

Pila's app launcher: the installed apps, searched as you type. Return or a
click starts the one picked; Escape closes it. The first run stays in the
background; running it again shows or hides that one. In Pila, set
`launcher = \"pila-launcher\"` for Mod+D and tapping Mod.

Options:
  --terminal COMMAND  Run apps that ask for a terminal (Terminal=true in
                      their .desktop file) in COMMAND, as COMMAND -e ...
                      (default: $TERMINAL)
  -h, --help          Show this help
  -V, --version       Show the version
";

/// What to do instead of launching: print something, or a usage error.
#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    Help,
    Version,
    Error(String),
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, Exit> {
    let mut parsed = Args::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(Exit::Help),
            "-V" | "--version" => return Err(Exit::Version),
            "--terminal" => match args.next() {
                Some(terminal) => parsed.terminal = Some(terminal).filter(|t| !t.is_empty()),
                None => return Err(Exit::Error("--terminal needs a command".into())),
            },
            other => match other.strip_prefix("--terminal=") {
                Some(terminal) => {
                    parsed.terminal = Some(terminal.to_string()).filter(|t| !t.is_empty())
                }
                None => return Err(Exit::Error(format!("unknown argument {other:?}"))),
            },
        }
    }
    Ok(parsed)
}

/// A request from another run to the one in the background: show or hide,
/// and the terminal to use from now on.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Request {
    pub terminal: Option<String>,
    pub toggle: bool,
}

impl Request {
    pub fn encode(&self) -> String {
        let mut out = String::new();
        if let Some(terminal) = &self.terminal {
            // One line: a terminal command has no newlines worth keeping.
            out.push_str("terminal ");
            out.push_str(&terminal.replace('\n', " "));
            out.push('\n');
        }
        if self.toggle {
            out.push_str("toggle\n");
        }
        out
    }

    /// Unknown lines are skipped (a newer `pila-launcher` talking to an
    /// older one in the background).
    pub fn decode(text: &str) -> Self {
        let mut request = Request::default();
        for line in text.lines() {
            if line == "toggle" {
                request.toggle = true;
            } else if let Some(terminal) = line.strip_prefix("terminal ") {
                request.terminal = Some(terminal.to_string()).filter(|t| !t.is_empty());
            }
        }
        request
    }
}

/// Where the run in the background listens: one per Wayland display, in
/// the runtime directory. `None` without `XDG_RUNTIME_DIR`.
pub fn socket_path(runtime_dir: Option<&str>, wayland_display: Option<&str>) -> Option<PathBuf> {
    let runtime_dir = runtime_dir.filter(|d| !d.is_empty())?;
    // A display may be a path (`/run/user/1000/wayland-1`): its name.
    let display = wayland_display
        .and_then(|d| d.rsplit('/').next())
        .filter(|d| !d.is_empty())
        .unwrap_or("wayland-0");
    let display: String = display
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(PathBuf::from(runtime_dir).join(format!("pila-launcher-{display}.sock")))
}

/// An `Exec` line to run with no files or URLs: its field codes (`%f`,
/// `%U`, `%i`, ...) removed and `%%` made `%`. The quoting the spec allows
/// is the shell's, so the rest goes to `/bin/sh -c` as it is.
pub fn clean_exec(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('%') => {
                out.push('%');
                chars.next();
            }
            Some(code) if "fFuUdDnNickvm".contains(*code) => {
                chars.next();
            }
            _ => out.push('%'),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The shell command running `exec` (an `Exec` line) in `terminal`.
pub fn in_terminal(terminal: &str, exec: &str) -> String {
    // foot and kitty ignore `-e`; xterm, alacritty and most others need it.
    format!("{terminal} -e {}", clean_exec(exec))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, Exit> {
        parse_args(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn arguments() {
        assert_eq!(args(&[]), Ok(Args::default()));
        assert_eq!(
            args(&["--terminal", "foot"]).unwrap().terminal.as_deref(),
            Some("foot")
        );
        assert_eq!(
            args(&["--terminal=kitty -1"]).unwrap().terminal.as_deref(),
            Some("kitty -1")
        );
        assert_eq!(args(&["--terminal", ""]).unwrap().terminal, None);
        assert_eq!(args(&["-h"]), Err(Exit::Help));
        assert_eq!(args(&["--version"]), Err(Exit::Version));
        assert!(matches!(args(&["--terminal"]), Err(Exit::Error(_))));
        assert!(matches!(args(&["--quit"]), Err(Exit::Error(_))));
    }

    #[test]
    fn requests_go_through_the_socket_and_back() {
        let request = Request {
            terminal: Some("foot\n--server".into()),
            toggle: true,
        };
        assert_eq!(request.encode(), "terminal foot --server\ntoggle\n");
        assert_eq!(
            Request::decode(&request.encode()),
            Request {
                terminal: Some("foot --server".into()),
                toggle: true,
            }
        );
        assert_eq!(
            Request::decode("hello\ntoggle\n"),
            Request {
                terminal: None,
                toggle: true,
            }
        );
        assert_eq!(Request::decode(""), Request::default());
    }

    #[test]
    fn one_socket_per_display() {
        let path = |dir, display| socket_path(dir, display).map(|p| p.display().to_string());
        assert_eq!(
            path(Some("/run/user/1000"), Some("wayland-1")).as_deref(),
            Some("/run/user/1000/pila-launcher-wayland-1.sock")
        );
        assert_eq!(
            path(Some("/run/user/1000"), Some("/tmp/x/wayland-2")).as_deref(),
            Some("/run/user/1000/pila-launcher-wayland-2.sock")
        );
        assert_eq!(
            path(Some("/r"), None).as_deref(),
            Some("/r/pila-launcher-wayland-0.sock")
        );
        assert_eq!(
            path(Some("/r"), Some("a b")).as_deref(),
            Some("/r/pila-launcher-a_b.sock")
        );
        assert_eq!(path(None, Some("wayland-1")), None);
        assert_eq!(path(Some(""), Some("wayland-1")), None);
    }

    #[test]
    fn exec_field_codes_go_and_quoting_stays() {
        assert_eq!(clean_exec("app %f"), "app");
        assert_eq!(clean_exec("app --x %U --y"), "app --x --y");
        assert_eq!(clean_exec("app %i %c %k"), "app");
        assert_eq!(clean_exec("printf 100%%"), "printf 100%");
        assert_eq!(
            clean_exec(r#"sh -c "echo \"a b\"""#),
            r#"sh -c "echo \"a b\"""#
        );
        assert_eq!(in_terminal("foot", "htop %F"), "foot -e htop");
    }
}
