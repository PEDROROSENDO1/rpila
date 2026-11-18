//! The command line.

/// What `pila-bar` was asked for.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    /// On the bottom edge instead of the top.
    pub bottom: bool,
    /// The clock's strftime format; empty for no clock.
    pub clock: String,
}

pub const DEFAULT_CLOCK: &str = "%a %e %b  %H:%M";

impl Default for Args {
    fn default() -> Self {
        Args {
            bottom: false,
            clock: DEFAULT_CLOCK.to_string(),
        }
    }
}

pub const USAGE: &str = "\
Usage: pila-bar [--bottom] [--clock FORMAT]

Pila's bar, on every monitor: the workspaces (a click or the wheel switches),
the focused window's title and a clock. Start it with Pila, in `autostart`.
It works in any compositor with the layer shell, ext-workspace and
wlr-foreign-toplevel-management.

Options:
  --bottom          On the bottom edge of the screen (default: the top)
  --clock FORMAT    The clock as strftime(3) writes it, in the language of
                    LC_TIME; '' leaves it out (default: '%a %e %b  %H:%M')
  -h, --help        Show this help
  -V, --version     Show the version
";

/// What to do instead of showing the bar: print something, or a usage error.
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
            "--bottom" => parsed.bottom = true,
            "--clock" => match args.next() {
                Some(format) => parsed.clock = format,
                None => return Err(Exit::Error("--clock needs a format".into())),
            },
            other => match other.strip_prefix("--clock=") {
                Some(format) => parsed.clock = format.to_string(),
                None => return Err(Exit::Error(format!("unknown argument {other:?}"))),
            },
        }
    }
    Ok(parsed)
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
        assert!(args(&["--bottom"]).unwrap().bottom);
        assert_eq!(args(&["--clock", "%H:%M"]).unwrap().clock, "%H:%M");
        assert_eq!(args(&["--clock=%T"]).unwrap().clock, "%T");
        assert_eq!(args(&["--clock", ""]).unwrap().clock, "");
        assert_eq!(args(&["-h"]), Err(Exit::Help));
        assert_eq!(args(&["--version"]), Err(Exit::Version));
        assert!(matches!(args(&["--clock"]), Err(Exit::Error(_))));
        assert!(matches!(args(&["--top"]), Err(Exit::Error(_))));
    }
}
