//! Where a tray item's named icon is. Unlike an app's (pila-launcher's
//! `icons::find`, hicolor's `apps`), a tray icon is a status (a network's
//! signal, an input method's language) that lives in any context of the
//! user's icon theme, or in a directory the item names (`IconThemePath`).

use std::{
    fs,
    path::{Path, PathBuf},
};

/// An icon file the bar can draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconFile {
    Png(PathBuf),
    Svg(PathBuf),
}

/// Where icons are looked for: the XDG data directories, most important
/// first, and the icon themes, the user's first and hicolor last.
#[derive(Debug, Clone)]
pub struct Places {
    pub data_dirs: Vec<PathBuf>,
    pub themes: Vec<String>,
}

impl Places {
    /// This session's: the XDG variables, and the GTK settings' icon theme.
    pub fn of_session() -> Places {
        let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
        let home = var("HOME");
        let mut data_dirs: Vec<PathBuf> = var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| Path::new(h).join(".local/share")))
            .into_iter()
            .collect();
        let system = var("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        data_dirs.extend(
            system
                .split(':')
                .filter(|d| !d.is_empty())
                .map(PathBuf::from),
        );
        let config = var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| Path::new(h).join(".config")));
        let user_theme = config.and_then(|config| {
            ["gtk-4.0", "gtk-3.0"].into_iter().find_map(|gtk| {
                let settings = fs::read_to_string(config.join(gtk).join("settings.ini")).ok()?;
                gtk_icon_theme(&settings)
            })
        });
        let themes = user_theme
            .into_iter()
            .chain(["breeze", "Adwaita", "hicolor"].map(String::from))
            .fold(Vec::new(), |mut themes, theme| {
                if !themes.contains(&theme) {
                    themes.push(theme);
                }
                themes
            });
        Places { data_dirs, themes }
    }
}

/// `gtk-icon-theme-name` in a GTK `settings.ini`.
fn gtk_icon_theme(settings: &str) -> Option<String> {
    settings.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "gtk-icon-theme-name")
            .then(|| value.trim().trim_matches('"').to_string())
            .filter(|v| !v.is_empty())
    })
}

/// The file for icon `name` (or a path), drawn at about `size` pixels:
/// first in `theme_path` (the item's own), then in each theme, then in
/// `pixmaps`.
pub fn find(name: &str, theme_path: &str, size: u32, places: &Places) -> Option<IconFile> {
    if name.starts_with('/') {
        let path = Path::new(name);
        return kind(path).filter(|_| path.is_file());
    }
    if name.is_empty() || name.contains('/') {
        return None;
    }
    if !theme_path.is_empty() {
        let root = Path::new(theme_path);
        if let Some(file) = in_dir(root, name).or_else(|| in_theme(root, name, size)) {
            return Some(file);
        }
        // A theme laid out inside it (`hicolor/22x22/apps`).
        if let Ok(themes) = fs::read_dir(root) {
            for theme in themes.flatten() {
                if let Some(file) = in_theme(&theme.path(), name, size) {
                    return Some(file);
                }
            }
        }
    }
    for theme in &places.themes {
        for dir in &places.data_dirs {
            if let Some(file) = in_theme(&dir.join("icons").join(theme), name, size) {
                return Some(file);
            }
        }
    }
    places
        .data_dirs
        .iter()
        .map(|dir| dir.join("pixmaps"))
        .find_map(|dir| in_dir(&dir, name))
}

/// `name` in a theme laid out as `<size>/<context>/` (or
/// `<context>/<size>/`, as breeze is): the best size for `size`.
fn in_theme(theme: &Path, name: &str, size: u32) -> Option<IconFile> {
    let mut best: Option<(u32, IconFile)> = None;
    for outer in subdirs(theme) {
        for inner in subdirs(&outer) {
            let Some(file) = in_dir(&inner, name) else {
                continue;
            };
            let dir_size = dir_size(&outer).or_else(|| dir_size(&inner));
            let score = score(dir_size, size);
            if best.as_ref().is_none_or(|(s, _)| score < *s) {
                best = Some((score, file));
            }
        }
    }
    best.map(|(_, file)| file)
}

/// A directory's icon size: `22x22`, `22x22@2`, `22` (breeze), or
/// `scalable` (None).
fn dir_size(dir: &Path) -> Option<u32> {
    let name = dir.file_name()?.to_str()?;
    let (size, scale) = match name.split_once('@') {
        Some((size, scale)) => (size, scale.parse().ok()?),
        None => (name, 1),
    };
    let side = size.split_once('x').map_or(size, |(w, _)| w);
    side.parse::<u32>().ok().map(|s| s * scale)
}

/// Lower is better: scalable first, then the smallest at least `size`, then
/// the biggest below it.
fn score(dir_size: Option<u32>, size: u32) -> u32 {
    match dir_size {
        None => 0,
        Some(s) if s >= size => 1 + (s - size),
        Some(s) => 10_000 + (size - s),
    }
}

fn subdirs(dir: &Path) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
}

fn in_dir(dir: &Path, name: &str) -> Option<IconFile> {
    ["svg", "png"].into_iter().find_map(|extension| {
        let path = dir.join(format!("{name}.{extension}"));
        if path.is_file() {
            kind(&path)
        } else {
            None
        }
    })
}

fn kind(path: &Path) -> Option<IconFile> {
    match path.extension()?.to_str()? {
        "png" => Some(IconFile::Png(path.to_path_buf())),
        "svg" => Some(IconFile::Svg(path.to_path_buf())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Root(PathBuf);

    impl Root {
        fn new(name: &str) -> Root {
            let root =
                std::env::temp_dir().join(format!("pila-bar-icons-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            Root(root)
        }

        fn touch(&self, path: &str) -> PathBuf {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"").unwrap();
            path
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn found_in_any_context_at_the_best_size() {
        let root = Root::new("contexts");
        root.touch("share/icons/hicolor/16x16/status/net.png");
        let big = root.touch("share/icons/hicolor/48x48/status/net.png");
        root.touch("share/icons/hicolor/22x22/apps/im.png");
        let themed = root.touch("share/icons/Papirus/24x24@2/panel/im.svg");
        let breeze = root.touch("share/icons/breeze/status/22/vol.svg");
        let pixmap = root.touch("share/pixmaps/old.png");
        let places = Places {
            data_dirs: vec![root.0.join("share")],
            themes: vec!["Papirus".into(), "breeze".into(), "hicolor".into()],
        };
        assert_eq!(find("net", "", 32, &places), Some(IconFile::Png(big)));
        assert_eq!(
            find("im", "", 32, &places),
            Some(IconFile::Svg(themed)),
            "the user's theme first"
        );
        assert_eq!(find("vol", "", 22, &places), Some(IconFile::Svg(breeze)));
        assert_eq!(find("old", "", 22, &places), Some(IconFile::Png(pixmap)));
        assert_eq!(find("none", "", 22, &places), None);
        assert_eq!(find("../net", "", 22, &places), None);
    }

    #[test]
    fn an_items_own_directory_first() {
        let root = Root::new("own");
        root.touch("share/icons/hicolor/22x22/apps/app.png");
        let flat = root.touch("item/app.png");
        let layout = root.touch("item2/hicolor/22x22/status/app-busy.svg");
        let places = Places {
            data_dirs: vec![root.0.join("share")],
            themes: vec!["hicolor".into()],
        };
        let item = root.0.join("item");
        let item2 = root.0.join("item2");
        assert_eq!(
            find("app", item.to_str().unwrap(), 22, &places),
            Some(IconFile::Png(flat))
        );
        assert_eq!(
            find("app-busy", item2.to_str().unwrap(), 22, &places),
            Some(IconFile::Svg(layout))
        );
    }

    #[test]
    fn the_gtk_settings_icon_theme() {
        let settings =
            "[Settings]\ngtk-theme-name=Adwaita-dark\ngtk-icon-theme-name = \"Papirus-Dark\"\n";
        assert_eq!(gtk_icon_theme(settings), Some("Papirus-Dark".into()));
        assert_eq!(gtk_icon_theme("[Settings]\n"), None);
        assert_eq!(dir_size(Path::new("22x22@2")), Some(44));
        assert_eq!(dir_size(Path::new("scalable")), None);
        assert_eq!(dir_size(Path::new("22")), Some(22));
    }
}
