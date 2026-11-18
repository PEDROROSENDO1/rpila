//! The command line.

/// What `pila-bar` was asked for.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    /// On the bottom edge instead of the top.
    pub bottom: bool,
    /// The clock's strftime format; empty for no clock.
    pub clock: String,
    /// The bar's height in logical pixels, its exclusive zone.
    pub height: u32,
    /// The outputs (by name) to show the bar on; empty for all of them.
    pub outputs: Vec<String>,
    /// Show the battery, where there's one.
    pub battery: bool,
}

pub const DEFAULT_CLOCK: &str = "%a %e %b  %H:%M";
/// The height when `--height` doesn't say, what the text size is made for.
pub const DEFAULT_HEIGHT: u32 = 28;
/// What `--height` takes.
pub const HEIGHTS: std::ops::RangeInclusive<u32> = 16..=96;

impl Default for Args {
    fn default() -> Self {
        Args {
            bottom: false,
            clock: DEFAULT_CLOCK.to_string(),
            height: DEFAULT_HEIGHT,
            outputs: Vec::new(),
            battery: true,
        }
    }
}

pub const USAGE: &str = "\
Usage: pila-bar [--bottom] [--clock FORMAT] [--height PIXELS] [--output NAME]...
               [--no-battery]

Pila's bar, on every monitor: the workspaces (a click or the wheel switches),
the battery (where there's one) and a clock. It's Pila's default `autostart`. It works in any compositor with
the layer shell and ext-workspace.

Options:
  --bottom          On the bottom edge of the screen (default: the top)
  --clock FORMAT    The clock as strftime(3) writes it, in the language of
                    LC_TIME; '' leaves it out (default: '%a %e %b  %H:%M')
  --height PIXELS   The bar's height, 16 to 96; the text grows with it
                    (default: 28)
  --output NAME     Only on this monitor (as `pila msg outputs` names it);
                    once per monitor for several (default: all of them)
  --no-battery      Leave the battery out
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
        // `--option=value` as `--option value`.
        let (option, inline) = match arg.split_once('=') {
            Some((option, value)) if option.starts_with("--") => (option, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = |what: &str| {
            inline
                .clone()
                .or_else(|| args.next())
                .ok_or_else(|| Exit::Error(format!("{option} needs {what}")))
        };
        match option {
            "-h" | "--help" => return Err(Exit::Help),
            "-V" | "--version" => return Err(Exit::Version),
            "--bottom" if inline.is_none() => parsed.bottom = true,
            "--no-battery" if inline.is_none() => parsed.battery = false,
            "--clock" => parsed.clock = value("a format")?,
            "--height" => {
                let text = value("a height")?;
                parsed.height = text
                    .parse()
                    .ok()
                    .filter(|h| HEIGHTS.contains(h))
                    .ok_or_else(|| {
                        Exit::Error(format!(
                            "--height takes {} to {} pixels, not {text:?}",
                            HEIGHTS.start(),
                            HEIGHTS.end()
                        ))
                    })?;
            }
            "--output" => match value("a monitor's name")? {
                name if name.is_empty() => {
                    return Err(Exit::Error("--output needs a monitor's name".into()))
                }
                name => parsed.outputs.push(name),
            },
            _ => return Err(Exit::Error(format!("unknown argument {arg:?}"))),
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
        assert!(args(&[]).unwrap().battery);
        assert!(!args(&["--no-battery"]).unwrap().battery);
        assert_eq!(args(&["--clock", "%H:%M"]).unwrap().clock, "%H:%M");
        assert_eq!(args(&["--clock=%T"]).unwrap().clock, "%T");
        assert_eq!(args(&["--clock", ""]).unwrap().clock, "");
        // A format may hold '='.
        assert_eq!(args(&["--clock=a=b"]).unwrap().clock, "a=b");
        assert_eq!(args(&["-h"]), Err(Exit::Help));
        assert_eq!(args(&["--version"]), Err(Exit::Version));
        assert!(matches!(args(&["--clock"]), Err(Exit::Error(_))));
        assert!(matches!(args(&["--top"]), Err(Exit::Error(_))));
        assert!(matches!(args(&["--bottom=yes"]), Err(Exit::Error(_))));
    }

    #[test]
    fn height() {
        assert_eq!(args(&["--height", "36"]).unwrap().height, 36);
        assert_eq!(args(&["--height=16"]).unwrap().height, 16);
        assert_eq!(args(&["--height", "96"]).unwrap().height, 96);
        for wrong in ["15", "97", "-1", "big", ""] {
            assert!(
                matches!(args(&["--height", wrong]), Err(Exit::Error(_))),
                "{wrong:?}"
            );
        }
        assert!(matches!(args(&["--height"]), Err(Exit::Error(_))));
    }

    #[test]
    fn outputs() {
        assert_eq!(
            args(&["--output", "DP-1", "--output=eDP-1"])
                .unwrap()
                .outputs,
            ["DP-1", "eDP-1"]
        );
        assert!(matches!(args(&["--output"]), Err(Exit::Error(_))));
        assert!(matches!(args(&["--output", ""]), Err(Exit::Error(_))));
    }
}
