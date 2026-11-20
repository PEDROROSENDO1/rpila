//! Pila's configuration for the tools its defaults start (`tool_configs.rs`):
//! the example's own commands (waybar, mako, the swayidle line's swaylock, the
//! launcher fuzzel) get it by their command line, from files in the runtime
//! directory, where the user has no configuration of their own for the tool.
//! Nothing is written where the tools look for the user's.

mod common;

use common::{Options, TestPila};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const WAYBAR: &str = include_str!("../resources/tools/waybar/config.jsonc");
const WAYBAR_ICONS: &str = include_str!("../resources/tools/waybar/config-icons.jsonc");
const WAYBAR_STYLE: &str = include_str!("../resources/tools/waybar/style.css");
const FUZZEL: &str = include_str!("../resources/tools/fuzzel/fuzzel.ini");
const MAKO: &str = include_str!("../resources/tools/mako/config");
const SWAYLOCK: &str = include_str!("../resources/tools/swaylock/config");

const TOOLS: &[&str] = &["waybar", "fuzzel", "mako", "swayidle", "swaylock"];

/// A directory of its own for a test: `bin` with stand-ins for `TOOLS` (the
/// whole `PATH`, so nothing else counts as installed) that write their
/// command line to `$XDG_RUNTIME_DIR/ran`, `config` and `home` for
/// `XDG_CONFIG_HOME` and `HOME`, and `data` and `system` for `XDG_DATA_HOME`
/// and `XDG_DATA_DIRS` (no font of the system's counts).
fn sandbox(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pila-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["bin", "config", "home", "data", "system"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    for tool in TOOLS {
        let path = dir.join("bin").join(tool);
        std::fs::write(
            &path,
            "#!/bin/sh\nprintf '%s\\n' \"${0##*/}${*:+ $*}\" >> \"$XDG_RUNTIME_DIR/ran\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

/// The example's `autostart`, as a line of TOML.
fn example_autostart() -> String {
    let commands = pila::config::Config::from_toml("", &|_| None)
        .unwrap()
        .autostart;
    let quoted: Vec<String> = commands.iter().map(|c| format!("{c:?}")).collect();
    format!("autostart = [{}]\n", quoted.join(", "))
}

fn start(dir: &Path, config: &str) -> TestPila {
    TestPila::start_with(Options {
        config: config.into(),
        env: vec![
            ("PATH".into(), dir.join("bin").display().to_string()),
            (
                "XDG_CONFIG_HOME".into(),
                dir.join("config").display().to_string(),
            ),
            ("HOME".into(), dir.join("home").display().to_string()),
            (
                "XDG_DATA_HOME".into(),
                dir.join("data").display().to_string(),
            ),
            (
                "XDG_DATA_DIRS".into(),
                dir.join("system").display().to_string(),
            ),
        ],
        ..Options::default()
    })
}

/// The command lines the stand-ins ran, once there are `count`, sorted.
#[track_caller]
fn ran(pila: &TestPila, count: usize) -> Vec<String> {
    let path = pila.dir.join("ran");
    let started = Instant::now();
    loop {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        if lines.len() >= count {
            lines.sort();
            return lines;
        }
        if started.elapsed() > Duration::from_secs(10) {
            panic!("{lines:?}, not {count} commands:\n{}", pila.log_tail());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The example's commands, with the launcher (Mod+D) after autostart's.
fn run_the_defaults(dir: &Path, config: &str) -> (TestPila, Vec<String>) {
    let mut pila = start(dir, &format!("{}{config}", example_autostart()));
    ran(&pila, 3);
    pila.combo("Super+d");
    let lines = ran(&pila, 4);
    (pila, lines)
}

#[test]
fn the_defaults_get_pilas_configuration_by_their_command_line() {
    let dir = sandbox("flags");
    let (pila, lines) = run_the_defaults(&dir, "");
    let tools = pila.dir.join("pila-tools");
    let t = tools.display();
    assert_eq!(
        lines,
        [
            format!("fuzzel --config={t}/fuzzel/fuzzel.ini"),
            format!("mako -c {t}/mako/config"),
            format!(
                "swayidle -w timeout 300 swaylock -C {t}/swaylock/config -f timeout 600 \
                 pila msg action power-off-monitors before-sleep swaylock -C {t}/swaylock/config -f"
            ),
            format!("waybar -c {t}/waybar/config.jsonc -s {t}/waybar/style.css"),
        ]
    );
    let read = |relative: &str| std::fs::read_to_string(tools.join(relative)).unwrap();
    assert_eq!(read("waybar/config.jsonc"), WAYBAR);
    assert_eq!(read("waybar/style.css"), WAYBAR_STYLE);
    assert_eq!(read("fuzzel/fuzzel.ini"), FUZZEL);
    assert_eq!(read("mako/config"), MAKO);
    assert_eq!(read("swaylock/config"), SWAYLOCK);
    // Nothing where the tools look for the user's.
    for tool in ["waybar", "fuzzel", "mako", "swaylock"] {
        assert!(!dir.join("config").join(tool).exists(), "{tool}");
    }
    for own in [".mako", ".swaylock"] {
        assert!(!dir.join("home").join(own).exists(), "{own}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A configuration of the user's own for a tool, wherever that tool reads
/// one (an older name, the home directory, a broken link too), starts it as
/// written.
#[test]
fn a_tools_own_configuration_wins() {
    let dir = sandbox("own");
    let config = dir.join("config");
    std::fs::create_dir_all(config.join("waybar")).unwrap();
    std::fs::write(config.join("waybar/config"), "{}").unwrap();
    std::fs::create_dir_all(config.join("fuzzel")).unwrap();
    std::os::unix::fs::symlink(dir.join("nowhere"), config.join("fuzzel/fuzzel.ini")).unwrap();
    for own in [".swaylock", ".mako"] {
        std::fs::create_dir_all(dir.join("home").join(own)).unwrap();
        std::fs::write(dir.join("home").join(own).join("config"), "").unwrap();
    }
    let (_pila, lines) = run_the_defaults(&dir, "");
    assert_eq!(
        lines,
        [
            "fuzzel",
            "mako",
            "swayidle -w timeout 300 swaylock -f timeout 600 \
             pila msg action power-off-monitors before-sleep swaylock -f",
            "waybar",
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tool_configs_false_starts_them_as_written() {
    let dir = sandbox("off");
    let (pila, lines) = run_the_defaults(&dir, "tool-configs = false\n");
    assert_eq!(lines[0], "fuzzel");
    assert_eq!(lines[1], "mako");
    assert_eq!(lines[3], "waybar");
    assert!(!pila.dir.join("pila-tools").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Only the example's own commands: one written differently is the user's
/// choice, and runs as written.
#[test]
fn a_command_of_ones_own_runs_as_written() {
    let dir = sandbox("mine");
    let mut pila = start(
        &dir,
        "autostart = [\"waybar -l debug\"]\nlauncher = \"fuzzel --width 50\"\n",
    );
    ran(&pila, 1);
    pila.combo("Super+d");
    assert_eq!(ran(&pila, 2), ["fuzzel --width 50", "waybar -l debug"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// With Font Awesome installed (here a file named as Arch's, in the system's
/// font directory), waybar gets the configuration with symbols; the style is
/// the same.
#[test]
fn waybar_gets_symbols_where_font_awesome_is_installed() {
    let dir = sandbox("icons");
    let fonts = dir.join("system/fonts/OTF");
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::write(fonts.join("Font Awesome 7 Free-Solid-900.otf"), "").unwrap();
    let pila = start(&dir, &example_autostart());
    ran(&pila, 3);
    let tools = pila.dir.join("pila-tools");
    assert_eq!(
        std::fs::read_to_string(tools.join("waybar/config.jsonc")).unwrap(),
        WAYBAR_ICONS
    );
    assert_eq!(
        std::fs::read_to_string(tools.join("waybar/style.css")).unwrap(),
        WAYBAR_STYLE
    );
    let _ = std::fs::remove_dir_all(&dir);
}
