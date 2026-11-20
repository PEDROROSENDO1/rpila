//! The configuration Pila writes at startup for the tools its defaults start
//! (`tool_configs.rs`): for those installed, where the user has none, never
//! over a file of theirs.

mod common;

use common::{Options, TestPila};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const WAYBAR: &str = include_str!("../resources/tools/waybar/config.jsonc");
const WAYBAR_ICONS: &str = include_str!("../resources/tools/waybar/config-icons.jsonc");
const WAYBAR_STYLE: &str = include_str!("../resources/tools/waybar/style.css");
const FUZZEL: &str = include_str!("../resources/tools/fuzzel/fuzzel.ini");
const MAKO: &str = include_str!("../resources/tools/mako/config");
const SWAYLOCK: &str = include_str!("../resources/tools/swaylock/config");

/// A directory of its own for a test: `bin` with stand-ins for `tools` (the
/// whole `PATH`, so nothing else counts as installed), `config` and `home`
/// for `XDG_CONFIG_HOME` and `HOME`, and `data` and `system` for
/// `XDG_DATA_HOME` and `XDG_DATA_DIRS` (no font of the system's counts).
fn sandbox(name: &str, tools: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pila-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["bin", "config", "home", "data", "system"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    for tool in tools {
        let path = dir.join("bin").join(tool);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
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

fn read(dir: &Path, relative: &str) -> Option<String> {
    std::fs::read_to_string(dir.join("config").join(relative)).ok()
}

#[test]
fn missing_tool_configs_are_written_for_installed_tools() {
    let dir = sandbox("all", &["waybar", "fuzzel", "mako", "swaylock"]);
    let mut pila = start(&dir, "");
    pila.wait_log("created a tool config");
    assert_eq!(read(&dir, "waybar/config.jsonc").as_deref(), Some(WAYBAR));
    assert_eq!(
        read(&dir, "waybar/style.css").as_deref(),
        Some(WAYBAR_STYLE)
    );
    assert_eq!(read(&dir, "fuzzel/fuzzel.ini").as_deref(), Some(FUZZEL));
    assert_eq!(read(&dir, "mako/config").as_deref(), Some(MAKO));
    assert_eq!(read(&dir, "swaylock/config").as_deref(), Some(SWAYLOCK));
    drop(pila);
    // Deleted, it's written again at the next start.
    std::fs::remove_file(dir.join("config/fuzzel/fuzzel.ini")).unwrap();
    let _pila = start(&dir, "");
    assert_eq!(read(&dir, "fuzzel/fuzzel.ini").as_deref(), Some(FUZZEL));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_existing_tool_config_is_left_alone() {
    let dir = sandbox("own", &["waybar", "fuzzel", "mako", "swaylock"]);
    let config = dir.join("config");
    // waybar's older name, a fuzzel.ini of one's own, swaylock's and mako's
    // in the home directory.
    std::fs::create_dir_all(config.join("waybar")).unwrap();
    std::fs::write(config.join("waybar/config"), "{}").unwrap();
    std::fs::create_dir_all(config.join("fuzzel")).unwrap();
    std::fs::write(config.join("fuzzel/fuzzel.ini"), "# mine\n").unwrap();
    for own in [".swaylock", ".mako"] {
        std::fs::create_dir_all(dir.join("home").join(own)).unwrap();
        std::fs::write(dir.join("home").join(own).join("config"), "").unwrap();
    }
    let mut pila = start(&dir, "");
    pila.settle();
    assert_eq!(read(&dir, "waybar/config.jsonc"), None);
    assert_eq!(read(&dir, "waybar/style.css"), None);
    assert_eq!(read(&dir, "fuzzel/fuzzel.ini").as_deref(), Some("# mine\n"));
    assert_eq!(read(&dir, "swaylock/config"), None);
    assert_eq!(read(&dir, "mako/config"), None);
    assert!(!pila.log().contains("created a tool config"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tool_that_isnt_installed_gets_no_config() {
    let dir = sandbox("one", &["fuzzel"]);
    let mut pila = start(&dir, "");
    pila.wait_log("created a tool config");
    assert_eq!(read(&dir, "fuzzel/fuzzel.ini").as_deref(), Some(FUZZEL));
    for absent in ["waybar", "mako", "swaylock"] {
        assert!(!dir.join("config").join(absent).exists(), "{absent}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tool_configs_false_writes_nothing() {
    let dir = sandbox("off", &["waybar", "fuzzel", "mako", "swaylock"]);
    let mut pila = start(&dir, "tool-configs = false\n");
    pila.settle();
    for tool in ["waybar", "fuzzel", "mako", "swaylock"] {
        assert!(!dir.join("config").join(tool).exists(), "{tool}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// With Font Awesome installed (here a file named as Arch's, in the system's
/// font directory), waybar gets the configuration with symbols; the style is
/// the same.
#[test]
fn waybar_gets_symbols_where_font_awesome_is_installed() {
    let dir = sandbox("icons", &["waybar"]);
    let fonts = dir.join("system/fonts/OTF");
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::write(fonts.join("Font Awesome 7 Free-Solid-900.otf"), "").unwrap();
    let mut pila = start(&dir, "");
    pila.wait_log("created a tool config");
    assert_eq!(
        read(&dir, "waybar/config.jsonc").as_deref(),
        Some(WAYBAR_ICONS)
    );
    assert_eq!(
        read(&dir, "waybar/style.css").as_deref(),
        Some(WAYBAR_STYLE)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
