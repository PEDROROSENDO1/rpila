//! A configuration of Pila's own for the standard tools its defaults start
//! (waybar, fuzzel, mako) and swaylock, written at startup where the user has
//! none, so that a first session looks like one desktop: Pila's default
//! `[colors]`, square corners. waybar shows the apps' own icons, and symbols
//! for the volume, battery and clock where Font Awesome is installed (text
//! where it isn't: a missing glyph would show as a box). Never over
//! anything of the user's: a tool counts as configured when any file it reads
//! is there, and each file is created with `create_new`, a link (even a
//! broken one) included. A tool installed later gets its files at the next
//! start; a file deleted is written again. `tool-configs = false` turns it off.

use std::path::{Path, PathBuf};

use tracing::{info, warn};

/// A tool Pila writes a configuration for.
struct Tool {
    /// The program, looked up in `PATH`.
    program: &'static str,
    /// Files the tool reads, relative to the config directory, or to the home
    /// directory when they start with `~/`: any of them there means the user
    /// has a configuration of their own.
    existing: &'static [&'static str],
    /// What Pila writes, relative to the config directory.
    files: &'static [(&'static str, &'static str)],
    /// What it writes instead where an icon font is installed
    /// (`icon_font_installed`); empty for a tool with no such variant.
    with_icons: &'static [(&'static str, &'static str)],
}

const WAYBAR_STYLE: (&str, &str) = (
    "waybar/style.css",
    include_str!("../resources/tools/waybar/style.css"),
);

const TOOLS: &[Tool] = &[
    Tool {
        program: "waybar",
        existing: &["waybar/config", "waybar/config.jsonc"],
        files: &[
            (
                "waybar/config.jsonc",
                include_str!("../resources/tools/waybar/config.jsonc"),
            ),
            WAYBAR_STYLE,
        ],
        with_icons: &[
            (
                "waybar/config.jsonc",
                include_str!("../resources/tools/waybar/config-icons.jsonc"),
            ),
            WAYBAR_STYLE,
        ],
    },
    Tool {
        program: "fuzzel",
        existing: &["fuzzel/fuzzel.ini"],
        files: &[(
            "fuzzel/fuzzel.ini",
            include_str!("../resources/tools/fuzzel/fuzzel.ini"),
        )],
        with_icons: &[],
    },
    Tool {
        program: "mako",
        existing: &["mako/config", "~/.mako/config"],
        files: &[(
            "mako/config",
            include_str!("../resources/tools/mako/config"),
        )],
        with_icons: &[],
    },
    Tool {
        program: "swaylock",
        existing: &["swaylock/config", "~/.swaylock/config"],
        files: &[(
            "swaylock/config",
            include_str!("../resources/tools/swaylock/config"),
        )],
        with_icons: &[],
    },
];

/// Write the configuration of each installed tool (`installed`) that has none
/// under `config_home` (or `home`, for the tools that also look there), the
/// variant with icons where `icon_font` says there's an icon font (asked only
/// when such a variant is about to be written). Returns the files written; a
/// file that can't be written is logged, never an error (it's startup: Pila
/// runs without it).
pub fn write_missing(
    config_home: &Path,
    home: Option<&Path>,
    installed: impl Fn(&str) -> bool,
    icon_font: impl Fn() -> bool,
) -> Vec<PathBuf> {
    let mut written = Vec::new();
    for tool in TOOLS {
        if !installed(tool.program) {
            continue;
        }
        let configured = tool.existing.iter().any(|path| {
            let path = match path.strip_prefix("~/") {
                Some(rest) => match home {
                    Some(home) => home.join(rest),
                    None => return false,
                },
                None => config_home.join(path),
            };
            // A broken link is the user's too.
            path.symlink_metadata().is_ok()
        });
        if configured {
            continue;
        }
        let files = if !tool.with_icons.is_empty() && icon_font() {
            tool.with_icons
        } else {
            tool.files
        };
        for (relative, contents) in files {
            let path = config_home.join(relative);
            match crate::config::create_new_file(&path, contents) {
                Ok(true) => written.push(path),
                Ok(false) => {}
                Err(err) => {
                    warn!(path = %path.display(), %err, "can't write a tool config");
                }
            }
        }
    }
    written
}

/// `write_missing` for the user's directories and the tools in `PATH`,
/// logging each file written.
pub fn write_missing_for_user() {
    let Some(config_home) = crate::config::config_home() else {
        return;
    };
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let font_dirs = font_dirs(home.as_deref());
    let icon_font = || icon_font_installed(&font_dirs);
    for path in write_missing(
        &config_home,
        home.as_deref(),
        crate::config::in_path,
        icon_font,
    ) {
        info!(path = %path.display(), "created a tool config");
    }
}

/// Where fontconfig looks for fonts by default: the user's data directory and
/// `~/.fonts`, then each of `XDG_DATA_DIRS`' `fonts` (and NixOS's `X11/fonts`).
fn font_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    let mut dirs = Vec::new();
    match var("XDG_DATA_HOME") {
        Some(data) => dirs.push(PathBuf::from(data).join("fonts")),
        None => dirs.extend(home.map(|home| home.join(".local/share/fonts"))),
    }
    dirs.extend(home.map(|home| home.join(".fonts")));
    let data_dirs = var("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for dir in std::env::split_paths(&data_dirs) {
        dirs.push(dir.join("fonts"));
        dirs.push(dir.join("X11/fonts"));
    }
    dirs
}

/// Whether Font Awesome is installed: a font file under `dirs` with
/// "awesome" in its path (Arch's `Font Awesome 7 Free-Solid-900.otf`,
/// Fedora's `fontawesome-6-free-fonts/`, Debian's `font-awesome/`). Its
/// symbols waybar's config uses are at the same code points in Font Awesome
/// 4 (Debian, Ubuntu) and 5 to 7.
pub(crate) fn icon_font_installed(dirs: &[PathBuf]) -> bool {
    fn walk(dir: &Path, depth: u32) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            // Followed: font directories are often links (NixOS).
            if path.is_dir() {
                return depth > 0 && walk(&path, depth - 1);
            }
            let name = path.to_string_lossy().to_lowercase();
            name.contains("awesome") && (name.ends_with(".otf") || name.ends_with(".ttf"))
        })
    }
    dirs.iter().any(|dir| walk(dir, 4))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The colors of `[colors]`' defaults in `config.example.toml`.
    fn palette() -> Vec<String> {
        let example: toml::Table = toml::from_str(crate::config::EXAMPLE).unwrap();
        example["colors"]
            .as_table()
            .unwrap()
            .values()
            .map(|v| v.as_str().unwrap()[1..7].to_lowercase())
            .collect()
    }

    /// Every `#rrggbb` (CSS, mako) or `rrggbb[aa]` value (fuzzel, swaylock)
    /// in `text`.
    fn colors_in(text: &str) -> Vec<String> {
        let mut found = Vec::new();
        for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
            for word in line.split(|c: char| !c.is_ascii_alphanumeric()) {
                let hex = word.len() == 6 || word.len() == 8;
                if hex && word.chars().all(|c| c.is_ascii_hexdigit()) {
                    found.push(word[..6].to_lowercase());
                }
            }
        }
        found
    }

    #[test]
    fn every_color_is_one_of_pilas() {
        let palette = palette();
        for tool in TOOLS {
            for (path, text) in tool.files.iter().chain(tool.with_icons) {
                let colors = colors_in(text);
                assert!(
                    !colors.is_empty() || path.ends_with(".jsonc"),
                    "{path} has no colors"
                );
                for color in colors {
                    // Transparent and black, for what swaylock must not draw.
                    let none = color == "000000";
                    assert!(
                        none || palette.contains(&color),
                        "{path}: {color} isn't a [colors] default ({palette:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn waybars_configs_are_json_and_alike() {
        let text: serde_json::Value =
            serde_json::from_str(include_str!("../resources/tools/waybar/config.jsonc")).unwrap();
        let icons: serde_json::Value =
            serde_json::from_str(include_str!("../resources/tools/waybar/config-icons.jsonc"))
                .unwrap();
        for config in [&text, &icons] {
            // Pila's workspaces, through the standard protocol, and the apps'
            // own icons in the window list.
            assert_eq!(config["modules-left"][0], "ext/workspaces");
            assert_eq!(config["wlr/taskbar"]["format"], "{icon} {title}");
            assert_eq!(config["modules-right"], text["modules-right"]);
        }
        // Text alone without the font; with it, symbols only from the code
        // points Font Awesome 4 shares with 5 to 7.
        let shared = ['\u{f017}', '\u{f026}', '\u{f027}', '\u{f028}', '\u{f1e6}']
            .into_iter()
            .chain('\u{f240}'..='\u{f244}');
        let shared: Vec<char> = shared.collect();
        let private = |c: &char| ('\u{e000}'..='\u{f8ff}').contains(c);
        let raw = |v: &serde_json::Value| v.to_string();
        assert!(!raw(&text).chars().any(|c| private(&c)));
        let used: Vec<char> = raw(&icons).chars().filter(private).collect();
        assert!(!used.is_empty());
        for c in used {
            assert!(
                shared.contains(&c),
                "U+{:04X} isn't in Font Awesome 4",
                c as u32
            );
        }
    }

    #[test]
    fn the_icon_font_is_found_by_its_files_name() {
        let dir = std::env::temp_dir().join(format!("pila-icon-font-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let fonts = dir.join("fonts");
        std::fs::create_dir_all(fonts.join("TTF")).unwrap();
        std::fs::write(fonts.join("TTF/DejaVuSans.ttf"), "").unwrap();
        assert!(!icon_font_installed(std::slice::from_ref(&fonts)));
        for path in [
            "OTF/Font Awesome 7 Free-Solid-900.otf",
            "fontawesome-6-free-fonts/Font Awesome 6 Free-Solid-900.otf",
            "truetype/font-awesome/fontawesome-webfont.ttf",
        ] {
            let _ = std::fs::remove_dir_all(&fonts);
            let file = fonts.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, "").unwrap();
            assert!(icon_font_installed(std::slice::from_ref(&fonts)), "{path}");
        }
        // Not a font: its licence.
        let _ = std::fs::remove_dir_all(&fonts);
        std::fs::create_dir_all(fonts.join("font-awesome")).unwrap();
        std::fs::write(fonts.join("font-awesome/LICENSE.txt"), "").unwrap();
        assert!(!icon_font_installed(std::slice::from_ref(&fonts)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_link_counts_as_the_users() {
        let dir = std::env::temp_dir().join(format!("pila-tool-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("fuzzel")).unwrap();
        std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("fuzzel/fuzzel.ini")).unwrap();
        let written = write_missing(&dir, None, |p| p == "fuzzel", || false);
        assert_eq!(written, Vec::<PathBuf>::new());
        assert!(!dir.join("nowhere").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
