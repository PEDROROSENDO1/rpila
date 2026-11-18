//! Where an app's icon is: its `Icon` key as a path, or a name looked up in
//! the `hicolor` theme (where apps install theirs) and in `pixmaps`.

use std::path::{Path, PathBuf};

/// An icon file the launcher can draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconFile {
    Png(PathBuf),
    Svg(PathBuf),
}

/// The sizes looked at, best first: drawn at 24 logical pixels, 48 is
/// sharp at scale 2 and larger ones scale down well.
const SIZES: [&str; 7] = [
    "48x48", "64x64", "96x96", "128x128", "256x256", "32x32", "scalable",
];

/// The file for an `Icon` value, if there's one the launcher can draw.
/// `data_dirs` are the XDG data directories, most important first.
pub fn find(icon: &str, data_dirs: &[PathBuf]) -> Option<IconFile> {
    if icon.starts_with('/') {
        return kind(Path::new(icon)).filter(|_| Path::new(icon).is_file());
    }
    // Some entries name the file (`app.png`): the spec says a name, so
    // look for the name without it.
    let name = icon
        .strip_suffix(".png")
        .or_else(|| icon.strip_suffix(".svg"))
        .unwrap_or(icon);
    if name.is_empty() || name.contains('/') {
        return None;
    }
    for size in SIZES {
        for dir in data_dirs {
            let dir = dir.join("icons/hicolor").join(size).join("apps");
            if let Some(file) = in_dir(&dir, name) {
                return Some(file);
            }
        }
    }
    data_dirs
        .iter()
        .map(|dir| dir.join("pixmaps"))
        .chain([PathBuf::from("/usr/share/pixmaps")])
        .find_map(|dir| in_dir(&dir, name))
}

fn in_dir(dir: &Path, name: &str) -> Option<IconFile> {
    ["png", "svg"].into_iter().find_map(|extension| {
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

    #[test]
    fn lookup() {
        let root = std::env::temp_dir().join(format!("pila-launcher-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let touch = |path: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"").unwrap();
        };
        touch("home/icons/hicolor/scalable/apps/a.svg");
        touch("sys/icons/hicolor/48x48/apps/a.png");
        touch("sys/icons/hicolor/256x256/apps/b.png");
        touch("home/icons/hicolor/scalable/apps/b.svg");
        touch("sys/pixmaps/c.png");
        touch("sys/pixmaps/d.xpm");
        touch("abs/e.svg");
        let dirs = [root.join("home"), root.join("sys")];
        let at = |p: &str| root.join(p);
        // A size beats a directory's importance.
        assert_eq!(
            find("a", &dirs),
            Some(IconFile::Png(at("sys/icons/hicolor/48x48/apps/a.png")))
        );
        assert_eq!(
            find("b", &dirs),
            Some(IconFile::Png(at("sys/icons/hicolor/256x256/apps/b.png")))
        );
        assert_eq!(
            find("c.png", &dirs),
            Some(IconFile::Png(at("sys/pixmaps/c.png")))
        );
        // XPM isn't drawn.
        assert_eq!(find("d", &dirs), None);
        let e = at("abs/e.svg");
        assert_eq!(find(e.to_str().unwrap(), &dirs), Some(IconFile::Svg(e)));
        assert_eq!(find(at("abs/none.svg").to_str().unwrap(), &dirs), None);
        assert_eq!(find("../a", &dirs), None);
        assert_eq!(find("", &dirs), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
