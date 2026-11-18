//! The command line, the messages between runs, where they meet, and the
//! commands that start an app.

use std::path::PathBuf;

/// What `pila-launcher` was asked to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// The terminal for apps with `Terminal=true` (else `pick_terminal`'s).
    pub terminal: Option<String>,
}

pub const USAGE: &str = "\
Usage: pila-launcher [--terminal COMMAND]

Pila's app launcher: the installed apps, searched as you type. Return or a
click starts the one picked; Escape closes it. The first run stays in the
background; running it again shows or hides that one. Pila opens it on
Mod+D and tapping Mod with `launcher = \"pila-launcher\"`, which the config
it writes on the first run sets.

Options:
  --terminal COMMAND  Run apps that ask for a terminal (Terminal=true in
                      their .desktop file) in COMMAND, as COMMAND -e ...
                      (default: $TERMINAL, else the first installed of
                      kitty, foot, alacritty, ghostty, wezterm, konsole,
                      gnome-terminal and xterm, as Pila picks its own)
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

/// Terminals tried in order with no `--terminal` and no `$TERMINAL`: the
/// list Pila tries for its own (`config::TERMINALS`), kitty first.
pub const TERMINALS: &[&str] = &[
    "kitty",
    "foot",
    "alacritty",
    "ghostty",
    "wezterm",
    "konsole",
    "gnome-terminal",
    "xterm",
];

/// The terminal for apps with `Terminal=true`: `given` (`--terminal`), else
/// `from_env` (`$TERMINAL`), else the first of `TERMINALS` that
/// `installed` finds; `None` if there's none.
pub fn pick_terminal(
    given: Option<&str>,
    from_env: Option<&str>,
    installed: impl Fn(&str) -> bool,
) -> Option<String> {
    given
        .or(from_env)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .or_else(|| {
            TERMINALS
                .iter()
                .find(|t| installed(t))
                .map(|t| t.to_string())
        })
}

/// Whether `program` is an executable file in a `PATH` directory.
pub fn in_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        std::fs::metadata(dir.join(program))
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    })
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
    fn kitty_is_the_default_terminal() {
        let all = |_: &str| true;
        assert_eq!(pick_terminal(None, None, all).as_deref(), Some("kitty"));
        // Where kitty isn't, the next installed, as Pila does.
        let no_kitty = |t: &str| t != "kitty";
        assert_eq!(pick_terminal(None, None, no_kitty).as_deref(), Some("foot"));
        assert_eq!(pick_terminal(None, None, |_| false), None);
        // --terminal, then $TERMINAL, come first.
        assert_eq!(
            pick_terminal(Some("foot"), Some("xterm"), all).as_deref(),
            Some("foot")
        );
        assert_eq!(
            pick_terminal(None, Some("xterm"), all).as_deref(),
            Some("xterm")
        );
        assert_eq!(pick_terminal(None, Some(""), all).as_deref(), Some("kitty"));
        // The same list as Pila's.
        assert_eq!(TERMINALS[0], "kitty");
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
