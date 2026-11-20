//! Telling the user when a command Pila ran needs a program that isn't
//! installed: a binding's (`brightnessctl -q set 5%+`), the
//! terminal, the launcher or `autostart`'s. The shell says "command not
//! found" only to Pila's log; a key that silently does nothing is the first
//! thing a new user meets.
//!
//! A spawned command (`Pila::spawn`) that exits with status 127, the shell's
//! for a command it can't find, has the programs its command line names
//! (`programs`) looked up in the `PATH` it ran with (`missing`); each one
//! that isn't there gets a notification, once a session
//! (`Pila::notify_missing_programs`). A program that exits 127 itself, with
//! everything installed, names none and notifies nothing.

use std::{os::unix::fs::PermissionsExt, path::Path};

use crate::Pila;

/// The shell's exit status for a command it can't find.
pub const NOT_FOUND: i32 = 127;

/// Words that aren't programs run by the shell: builtins and keywords that
/// run nothing of their own (`exec`, `command`, `env`, `nohup` and the like
/// run the next word, which `programs` takes instead).
const NOT_PROGRAMS: &[&str] = &[
    "!", ":", ".", "[", "[[", "break", "case", "cd", "continue", "do", "done", "echo", "elif",
    "else", "esac", "eval", "exit", "export", "false", "fi", "for", "if", "in", "local", "printf",
    "pwd", "read", "return", "set", "shift", "source", "test", "then", "trap", "true", "type",
    "ulimit", "umask", "unset", "until", "wait", "while", "{", "}",
];

/// Words that run the word after them (past their options and, for `env`,
/// assignments).
const RUN_NEXT: &[&str] = &["exec", "command", "env", "nohup", "setsid", "time"];

/// The programs a shell command line runs, as written: the first word of each
/// of its commands (split at `&&`, `||`, `;`, `|`, `&`, newlines and
/// parentheses outside quotes), past variable assignments and words that run
/// the next one. Words the shell would expand (`$...`, backquotes) are left
/// out: what they name isn't known before. A rough reading of `sh`, enough
/// for what bindings and `autostart` hold.
pub fn programs(command: &str) -> Vec<String> {
    let mut found = Vec::new();
    for words in commands(command) {
        let mut words = words.into_iter().peekable();
        while let Some(word) = words.next() {
            let assignment = word.split_once('=').is_some_and(|(name, _)| {
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
            if assignment {
                continue;
            }
            if RUN_NEXT.contains(&word.as_str()) {
                // Its options (and `env`'s assignments, above) come first,
                // `env`'s `-u NAME` and `-C DIR` with their argument.
                while let Some(option) = words.next_if(|w| w.starts_with('-')) {
                    let with_argument = ["-u", "--unset", "-C", "--chdir"];
                    if word == "env" && with_argument.contains(&option.as_str()) {
                        words.next();
                    }
                }
                continue;
            }
            let expanded = word.contains('$') || word.contains('`');
            if !expanded && !NOT_PROGRAMS.contains(&word.as_str()) && !found.contains(&word) {
                found.push(word);
            }
            break;
        }
    }
    found
}

/// A command line as its commands, each as its words with the quotes taken
/// off (a quoted operator is part of a word).
fn commands(command: &str) -> Vec<Vec<String>> {
    let mut commands = vec![Vec::new()];
    let mut word = String::new();
    // A word that was quoted ("") still counts as one.
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    let end_word = |commands: &mut Vec<Vec<String>>, word: &mut String, in_word: &mut bool| {
        if *in_word {
            if let Some(current) = commands.last_mut() {
                current.push(std::mem::take(word));
            }
            *in_word = false;
        }
    };
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    in_word = true;
                }
            }
            (None, '&' | '|' | ';' | '\n' | '(' | ')') => {
                end_word(&mut commands, &mut word, &mut in_word);
                // `&&`, `||`: one operator.
                if matches!(c, '&' | '|') && chars.peek() == Some(&c) {
                    chars.next();
                }
                commands.push(Vec::new());
            }
            (None, c) if c.is_whitespace() => end_word(&mut commands, &mut word, &mut in_word),
            (None, '#') if !in_word => {
                // A comment, to the end of the line.
                while chars.peek().is_some_and(|&c| c != '\n') {
                    chars.next();
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    end_word(&mut commands, &mut word, &mut in_word);
    commands.retain(|words| !words.is_empty());
    commands
}

/// Of the programs `command` runs (`programs`), those that aren't installed:
/// a name found in no directory of `path`, or a path that isn't an
/// executable file. `None` for `path` is the shell's default.
pub fn missing(command: &str, path: Option<&str>) -> Vec<String> {
    let executable = |file: &Path| {
        file.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    let path = path.unwrap_or("/usr/local/bin:/usr/bin:/bin");
    programs(command)
        .into_iter()
        .filter(|program| {
            if program.contains('/') {
                let file = match program.strip_prefix("~/") {
                    Some(rest) => match std::env::var_os("HOME") {
                        Some(home) => Path::new(&home).join(rest),
                        None => return false,
                    },
                    None => program.into(),
                };
                return !executable(&file);
            }
            !path
                .split(':')
                .filter(|dir| !dir.is_empty())
                .any(|dir| executable(&Path::new(dir).join(program)))
        })
        .collect()
}

/// The longest command a notification quotes whole.
const QUOTED: usize = 60;

/// `command` as a notification quotes it: whole if short, else its start and
/// an ellipsis.
fn shortened(command: &str) -> String {
    let one_line = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= QUOTED {
        return one_line;
    }
    let start: String = one_line.chars().take(QUOTED - 1).collect();
    format!("{}\u{2026}", start.trim_end())
}

/// The package a program comes in, where its name isn't the program's and
/// is the same on most distributions: for the default bindings' programs.
fn package_of(program: &str) -> Option<&'static str> {
    match program {
        "wpctl" => Some("wireplumber"),
        "wl-copy" | "wl-paste" => Some("wl-clipboard"),
        _ => None,
    }
}

impl Pila {
    /// A command Pila spawned exited with `NOT_FOUND`: a notification for
    /// each program it needs that isn't installed, the first time this
    /// session (a held key's repeats, or the same binding again, don't
    /// notify again).
    pub(crate) fn notify_missing_programs(&mut self, command: &str) {
        let path = self.spawn_path();
        for program in missing(command, path.as_deref()) {
            if !self.missing_programs.insert(program.clone()) {
                continue;
            }
            tracing::warn!(
                program,
                command,
                "a command needs a program that isn't installed"
            );
            let install = match package_of(&program) {
                Some(package) => format!("Install it (package {package})"),
                None => "Install it".to_string(),
            };
            // The advice first: notification daemons show a few lines.
            self.notify_error(
                &format!("{program} isn't installed"),
                &format!(
                    "{install}, or change the command that needs it in the configuration: \
                     \u{201c}{}\u{201d}",
                    shortened(command)
                ),
            );
        }
    }

    /// `autostart` as Pila runs it: without the example's own commands
    /// (`config::example_autostart`, also in a file made from it) whose
    /// programs aren't installed, which only the log mentions. Those are
    /// the defaults, not the user's choice: the tools of a full desktop
    /// (waybar, mako, swayidle) that a package such as `pila-desktop` brings,
    /// and that a user may leave out. A command of the user's own still runs,
    /// and its missing program is still notified (`notify_missing_programs`).
    pub fn autostart_installed(&self) -> Vec<String> {
        let defaults = crate::config::example_autostart();
        let path = self.spawn_path();
        self.config
            .autostart
            .iter()
            .filter(|command| {
                if !defaults.contains(command) {
                    return true;
                }
                let missing = missing(command, path.as_deref());
                if missing.is_empty() {
                    return true;
                }
                tracing::info!(
                    command = command.as_str(),
                    ?missing,
                    "not starting the example's autostart command: not installed"
                );
                false
            })
            .cloned()
            .collect()
    }

    /// The `PATH` the programs Pila starts get: `[environment]`'s, else
    /// Pila's own.
    fn spawn_path(&self) -> Option<String> {
        match self
            .config
            .environment
            .iter()
            .rev()
            .find(|(name, _)| name == "PATH")
        {
            Some((_, value)) => value.clone(),
            None => std::env::var("PATH").ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn the_programs_a_command_line_runs() {
        let cases: &[(&str, &[&str])] = &[
            (
                "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%- && notify-send v",
                &["wpctl", "notify-send"],
            ),
            ("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle", &["wpctl"]),
            ("foot", &["foot"]),
            ("FOO=1 BAR=2 exec foot -e htop", &["foot"]),
            ("env -u X LANG=C kitty", &["kitty"]),
            ("grim -g \"$(slurp)\" - | wl-copy", &["grim", "wl-copy"]),
            (
                "mkdir -p ~/Pictures && grim -o \"$PILA_OUTPUT\" ~/Pictures/s.png",
                &["mkdir", "grim"],
            ),
            ("sh -c 'a && b'; true", &["sh"]),
            ("echo hi > $XDG_RUNTIME_DIR/out", &[]),
            ("$TERMINAL -e top", &[]),
            ("(swaybg -c 333355 &) ; mako", &["swaybg", "mako"]),
            ("a || a || b", &["a", "b"]),
            ("\"my app\" --flag", &["my app"]),
            ("swaylock # lock it", &["swaylock"]),
            ("", &[]),
        ];
        for (command, expected) in cases {
            assert_eq!(programs(command), words(expected), "{command:?}");
        }
    }

    #[test]
    fn the_example_configs_commands_parse_to_programs() {
        // Every `spawn` of the example names programs this reading finds.
        let example = include_str!("../config.example.toml");
        for line in example.lines().filter(|l| !l.starts_with('#')) {
            let Some(at) = line.find("\"spawn ") else {
                continue;
            };
            let command = &line[at + 7..];
            assert!(!programs(command).is_empty(), "{line}");
        }
    }

    #[test]
    fn long_commands_are_shortened() {
        assert_eq!(shortened("foot  -e\n htop"), "foot -e htop");
        let long = "a ".repeat(40);
        let short = shortened(&long);
        assert_eq!(short.chars().count(), QUOTED);
        assert!(short.ends_with("a\u{2026}"), "{short}");
    }

    #[test]
    fn missing_programs_are_those_on_no_path() {
        let dir = std::env::temp_dir().join(format!("pila-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("tool");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let not_executable = dir.join("data");
        std::fs::write(&not_executable, "").unwrap();
        let path = format!("/nonexistent:{}", dir.display());
        assert_eq!(
            missing("tool && surely-not-installed x | tool", Some(&path)),
            words(&["surely-not-installed"])
        );
        assert_eq!(missing("data", Some(&path)), words(&["data"]));
        let full = format!("{} && {}", tool.display(), not_executable.display());
        assert_eq!(
            missing(&full, Some("")),
            vec![not_executable.display().to_string()]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
