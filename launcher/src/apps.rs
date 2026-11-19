//! The installed apps: their desktop entries (the `.desktop` files in each
//! `applications` directory of the XDG data directories), and their order
//! for a search.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

/// An app the launcher lists: what its desktop entry says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// The desktop file ID (`org.gnome.Nautilus.desktop`), unique.
    pub id: String,
    /// `Name`, in the user's language if the entry has it.
    pub name: String,
    /// `GenericName` and `Keywords`, also searched, after the name.
    pub generic_name: String,
    pub keywords: Vec<String>,
    /// `Exec`, field codes and all.
    pub exec: String,
    pub icon: Option<String>,
    /// `Terminal=true`: run in a terminal.
    pub terminal: bool,
    /// `Path`: the directory to start it in.
    pub path: Option<String>,
    /// `StartupWMClass`: the app id (or X11 class) its windows have, where it
    /// isn't the entry's ID. For pila-switcher, which finds a window's app by
    /// it; the launcher has no use for it.
    #[allow(dead_code)]
    pub wm_class: Option<String>,
}

/// What a scan needs to know about the session.
pub struct Session<'a> {
    /// `XDG_CURRENT_DESKTOP`'s names, for `OnlyShowIn` and `NotShowIn`.
    pub desktops: &'a [String],
    /// The `Name[...]` keys to try, best first (`locale_keys`).
    pub locales: &'a [String],
    /// Whether a `TryExec` program exists.
    pub found: &'a dyn Fn(&str) -> bool,
}

/// The XDG data directories, most important first: `$XDG_DATA_HOME`
/// (default `~/.local/share`), then each of `$XDG_DATA_DIRS` (default
/// `/usr/local/share:/usr/share`).
pub fn data_dirs(
    data_home: Option<&str>,
    home: Option<&str>,
    data_dirs: Option<&str>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match data_home.filter(|d| !d.is_empty()) {
        Some(dir) => dirs.push(PathBuf::from(dir)),
        None => {
            if let Some(home) = home.filter(|h| !h.is_empty()) {
                dirs.push(Path::new(home).join(".local/share"));
            }
        }
    }
    let system = data_dirs
        .filter(|d| !d.is_empty())
        .unwrap_or("/usr/local/share:/usr/share");
    dirs.extend(
        system
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    dirs
}

/// The localized keys to look for, best first, from a POSIX locale
/// (`LC_ALL`, `LC_MESSAGES` or `LANG`): `es_AR.UTF-8@euro` gives
/// `es_AR@euro`, `es_AR`, `es@euro`, `es` (the desktop entry spec's order).
pub fn locale_keys(locale: Option<&str>) -> Vec<String> {
    let Some(locale) = locale.filter(|l| !l.is_empty() && *l != "C" && *l != "POSIX") else {
        return Vec::new();
    };
    let (rest, modifier) = match locale.split_once('@') {
        Some((rest, modifier)) => (rest, Some(modifier)),
        None => (locale, None),
    };
    let rest = rest.split('.').next().unwrap_or(rest);
    let (lang, country) = match rest.split_once('_') {
        Some((lang, country)) => (lang, Some(country)),
        None => (rest, None),
    };
    let mut keys = Vec::new();
    if let (Some(country), Some(modifier)) = (country, modifier) {
        keys.push(format!("{lang}_{country}@{modifier}"));
    }
    if let Some(country) = country {
        keys.push(format!("{lang}_{country}"));
    }
    if let Some(modifier) = modifier {
        keys.push(format!("{lang}@{modifier}"));
    }
    keys.push(lang.to_string());
    keys
}

/// Every app in `dirs`' `applications` directories that should be listed.
/// The first entry with a given ID wins (`dirs` is most important first),
/// even one that isn't listed: that's how a user hides a system app.
pub fn scan(dirs: &[PathBuf], session: &Session) -> Vec<App> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in dirs {
        let mut files = Vec::new();
        collect(&dir.join("applications"), "", &mut files, 0);
        // Directory order isn't stable; IDs are.
        files.sort();
        for (id, path) in files {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(app) = parse(&id, &text, session) {
                apps.push(app);
            }
        }
    }
    apps.sort_by_cached_key(|app| (app.name.to_lowercase(), app.id.clone()));
    apps
}

/// The `.desktop` files under `dir`, with their IDs: a file in a
/// subdirectory `sub/name.desktop` has the ID `sub-name.desktop`.
fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>, depth: usize) {
    // Deep enough for any real tree, and no loop through symlinks.
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if path.is_dir() {
            collect(&path, &format!("{prefix}{name}-"), out, depth + 1);
        } else if name.ends_with(".desktop") {
            out.push((format!("{prefix}{name}"), path));
        }
    }
}

/// The app a desktop entry describes, if it should be listed: an
/// `Application` with an `Exec`, not `NoDisplay` or `Hidden`, shown in
/// this desktop, and whose `TryExec` program exists.
pub fn parse(id: &str, text: &str, session: &Session) -> Option<App> {
    let mut in_entry = false;
    let mut name: Option<(usize, String)> = None;
    let mut generic_name: Option<(usize, String)> = None;
    let mut keywords: Option<(usize, String)> = None;
    let mut kind = None;
    let mut exec = None;
    let mut icon = None;
    let mut terminal = false;
    let mut path = None;
    let mut hidden = false;
    let mut only_show_in = None;
    let mut not_show_in = None;
    let mut try_exec = None;
    let mut wm_class = None;
    // How good a localized key is: the unlocalized one is the worst.
    let rank = |locale: Option<&str>| -> Option<usize> {
        match locale {
            None => Some(session.locales.len()),
            Some(locale) => session.locales.iter().position(|l| l == locale),
        }
    };
    let keep = |slot: &mut Option<(usize, String)>, rank: Option<usize>, value: &str| {
        if let Some(rank) = rank {
            if slot.as_ref().is_none_or(|(best, _)| rank < *best) {
                *slot = Some((rank, value.to_string()));
            }
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let (key, locale) = match key.split_once('[') {
            Some((key, locale)) => (key, locale.strip_suffix(']')),
            None => (key, None),
        };
        match (key, locale) {
            ("Name", _) => keep(&mut name, rank(locale), &unescape(value)),
            ("GenericName", _) => keep(&mut generic_name, rank(locale), &unescape(value)),
            ("Keywords", _) => keep(&mut keywords, rank(locale), value),
            ("Type", None) => kind = Some(value.to_string()),
            ("Exec", None) => exec = Some(unescape(value)),
            ("Icon", None) => icon = Some(unescape(value)).filter(|i| !i.is_empty()),
            ("Terminal", None) => terminal = value == "true",
            ("Path", None) => path = Some(unescape(value)).filter(|p| !p.is_empty()),
            ("NoDisplay" | "Hidden", None) => hidden |= value == "true",
            ("OnlyShowIn", None) => only_show_in = Some(list(value)),
            ("NotShowIn", None) => not_show_in = Some(list(value)),
            ("TryExec", None) => try_exec = Some(unescape(value)),
            ("StartupWMClass", None) => wm_class = Some(unescape(value)).filter(|c| !c.is_empty()),
            _ => {}
        }
    }
    if kind.as_deref() != Some("Application") || hidden {
        return None;
    }
    let shown_here = |names: &[String]| {
        names
            .iter()
            .any(|n| session.desktops.iter().any(|d| d.eq_ignore_ascii_case(n)))
    };
    if only_show_in.is_some_and(|names| !shown_here(&names)) {
        return None;
    }
    if not_show_in.is_some_and(|names| shown_here(&names)) {
        return None;
    }
    if try_exec.is_some_and(|program| !program.is_empty() && !(session.found)(&program)) {
        return None;
    }
    let exec = exec.filter(|e| !e.trim().is_empty())?;
    let (_, name) = name.filter(|(_, n)| !n.is_empty())?;
    Some(App {
        id: id.to_string(),
        name,
        generic_name: generic_name.map(|(_, g)| g).unwrap_or_default(),
        keywords: keywords.map(|(_, k)| list(&k)).unwrap_or_default(),
        exec,
        icon,
        terminal,
        path,
        wm_class,
    })
}

/// A string value with the spec's escapes (`\s`, `\n`, `\t`, `\r`, `\\`)
/// read.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            // `\;` in a list, and anything else: kept as written.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// A `;`-separated list (`OnlyShowIn`, `Keywords`), escapes read.
fn list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut item = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(';') => item.push(';'),
                Some(other) => {
                    item.push('\\');
                    item.push(other);
                }
                None => item.push('\\'),
            },
            ';' => items.push(std::mem::take(&mut item)),
            _ => item.push(c),
        }
    }
    items.push(item);
    items
        .into_iter()
        .map(|i| unescape(i.trim()))
        .filter(|i| !i.is_empty())
        .collect()
}

/// The apps that match `query`, best first, as indices into `apps`
/// (sorted by name): those whose name starts with it, then those with a
/// word in the name that does, then those with it anywhere in the name,
/// then in the generic name, keywords or command. Case doesn't matter; an
/// empty query lists them all.
pub fn rank<'a>(apps: impl IntoIterator<Item = &'a App>, query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let apps = apps.into_iter();
    if query.is_empty() {
        return (0..apps.count()).collect();
    }
    let mut matches: Vec<(u8, usize)> = apps
        .enumerate()
        .filter_map(|(i, app)| score(app, &query).map(|s| (s, i)))
        .collect();
    // Stable: same score keeps the order by name.
    matches.sort_by_key(|&(score, _)| score);
    matches.into_iter().map(|(_, i)| i).collect()
}

fn score(app: &App, query: &str) -> Option<u8> {
    let name = app.name.to_lowercase();
    if name.starts_with(query) {
        return Some(0);
    }
    if name
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(query))
    {
        return Some(1);
    }
    if name.contains(query) {
        return Some(2);
    }
    let others = std::iter::once(&app.generic_name)
        .chain(&app.keywords)
        .chain(std::iter::once(&app.exec));
    for other in others {
        if other.to_lowercase().contains(query) {
            return Some(3);
        }
    }
    None
}

/// Whether `program` (a `TryExec`) is an executable file: a path as it is,
/// a name looked up in `path` (`$PATH`).
pub fn found_in(program: &str, path: Option<&str>) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &Path| {
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return executable(Path::new(program));
    }
    path.unwrap_or("")
        .split(':')
        .filter(|d| !d.is_empty())
        .any(|dir| executable(&Path::new(dir).join(program)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session<'a>(desktops: &'a [String], locales: &'a [String]) -> Session<'a> {
        Session {
            desktops,
            locales,
            found: &|program| program != "missing",
        }
    }

    fn app(text: &str) -> Option<App> {
        parse(
            "a.desktop",
            text,
            &session(&["pila".to_string()], &locale_keys(Some("es_AR.UTF-8"))),
        )
    }

    #[test]
    fn a_plain_entry() {
        let app = app("[Desktop Entry]\n\
             Type=Application\n\
             Name=Files\n\
             GenericName=File Manager\n\
             Keywords=folder;explorer;\n\
             Exec=nautilus --new-window %U\n\
             Icon=org.gnome.Nautilus\n\
             Path=/tmp\n\
             StartupWMClass=nautilus\n\
             [Desktop Action new]\n\
             Name=Other\n\
             Exec=other\n")
        .unwrap();
        assert_eq!(app.name, "Files");
        assert_eq!(app.generic_name, "File Manager");
        assert_eq!(app.keywords, ["folder", "explorer"]);
        assert_eq!(app.exec, "nautilus --new-window %U");
        assert_eq!(app.icon.as_deref(), Some("org.gnome.Nautilus"));
        assert_eq!(app.path.as_deref(), Some("/tmp"));
        assert_eq!(app.wm_class.as_deref(), Some("nautilus"));
        assert!(!app.terminal);
    }

    #[test]
    fn the_name_in_the_users_language() {
        let text = "[Desktop Entry]\nType=Application\nExec=x\n\
                    Name=Files\nName[es]=Archivos\nName[es_AR]=Archivos AR\nName[de]=Dateien\n";
        assert_eq!(app(text).unwrap().name, "Archivos AR");
        let text = "[Desktop Entry]\nType=Application\nExec=x\nName[es]=Archivos\nName=Files\n";
        assert_eq!(app(text).unwrap().name, "Archivos");
        let text = "[Desktop Entry]\nType=Application\nExec=x\nName[de]=Dateien\nName=Files\n";
        assert_eq!(app(text).unwrap().name, "Files");
        assert_eq!(
            locale_keys(Some("sr_RS.UTF-8@latin")),
            ["sr_RS@latin", "sr_RS", "sr@latin", "sr"]
        );
        assert_eq!(locale_keys(Some("C")), Vec::<String>::new());
        assert_eq!(locale_keys(None), Vec::<String>::new());
    }

    #[test]
    fn entries_that_arent_listed() {
        let base = "[Desktop Entry]\nType=Application\nName=A\nExec=a\n";
        assert!(app(base).is_some());
        assert!(app(&format!("{base}NoDisplay=true\n")).is_none());
        assert!(app(&format!("{base}Hidden=true\n")).is_none());
        assert!(app(&format!("{base}OnlyShowIn=GNOME;KDE;\n")).is_none());
        assert!(app(&format!("{base}OnlyShowIn=GNOME;Pila;\n")).is_some());
        assert!(app(&format!("{base}NotShowIn=pila;\n")).is_none());
        assert!(app(&format!("{base}NotShowIn=GNOME;\n")).is_some());
        assert!(app(&format!("{base}TryExec=missing\n")).is_none());
        assert!(app(&format!("{base}TryExec=present\n")).is_some());
        assert!(app("[Desktop Entry]\nType=Link\nName=A\nExec=a\n").is_none());
        assert!(app("[Desktop Entry]\nType=Application\nName=A\n").is_none());
        assert!(app("[Desktop Entry]\nType=Application\nExec=a\n").is_none());
        // Keys of another group don't count.
        assert!(app("[Other]\nType=Application\nName=A\nExec=a\n").is_none());
    }

    #[test]
    fn escapes() {
        let app = app("[Desktop Entry]\nType=Application\nName=A\\sB\n\
             Exec=sh -c \"echo a\\\\nb\"\nKeywords=x\\;y;z\nTerminal=true\n")
        .unwrap();
        assert_eq!(app.name, "A B");
        assert_eq!(app.exec, "sh -c \"echo a\\nb\"");
        assert_eq!(app.keywords, ["x;y", "z"]);
        assert!(app.terminal);
    }

    #[test]
    fn the_data_directories() {
        assert_eq!(
            data_dirs(None, Some("/home/u"), None),
            [
                PathBuf::from("/home/u/.local/share"),
                "/usr/local/share".into(),
                "/usr/share".into()
            ]
        );
        assert_eq!(
            data_dirs(Some("/d"), Some("/home/u"), Some("/a::/b")),
            [PathBuf::from("/d"), "/a".into(), "/b".into()]
        );
    }

    #[test]
    fn a_scan_takes_the_first_id_and_names_subdirectories() {
        let root = std::env::temp_dir().join(format!("pila-launcher-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let entry =
            |name: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\n");
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("home/applications/a.desktop", &entry("Home A"));
        write(
            "home/applications/hidden.desktop",
            "[Desktop Entry]\nType=Application\nName=H\nExec=x\nHidden=true\n",
        );
        write("sys/applications/a.desktop", &entry("System A"));
        write("sys/applications/hidden.desktop", &entry("System H"));
        write("sys/applications/kde/b.desktop", &entry("B"));
        write("sys/applications/c.txt", &entry("Not an entry"));
        let dirs = [root.join("home"), root.join("sys")];
        let apps = scan(&dirs, &session(&[], &[]));
        let found: Vec<_> = apps
            .iter()
            .map(|a| (a.id.as_str(), a.name.as_str()))
            .collect();
        assert_eq!(found, [("kde-b.desktop", "B"), ("a.desktop", "Home A")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ranking() {
        let app = |name: &str, generic: &str, exec: &str| App {
            id: format!("{name}.desktop"),
            name: name.into(),
            generic_name: generic.into(),
            keywords: vec!["shell".into()],
            exec: exec.into(),
            icon: None,
            terminal: false,
            path: None,
            wm_class: None,
        };
        let apps = [
            app("Firefox", "Web Browser", "firefox"),
            app("GNOME Web", "Web Browser", "epiphany"),
            app("kitty", "Terminal", "kitty"),
            app("Webcam", "", "webcam"),
            app("Zweb", "", "zweb"),
        ];
        let names = |query: &str| -> Vec<&str> {
            rank(&apps, query)
                .into_iter()
                .map(|i| apps[i].name.as_str())
                .collect()
        };
        assert_eq!(names("web"), ["Webcam", "GNOME Web", "Zweb", "Firefox"]);
        assert_eq!(names("WEB"), names("web"));
        assert_eq!(names("term"), ["kitty"]);
        assert_eq!(names("epiph"), ["GNOME Web"]);
        assert_eq!(names("shell").len(), 5);
        assert_eq!(names("nothing"), Vec::<&str>::new());
        assert_eq!(
            names(" "),
            ["Firefox", "GNOME Web", "kitty", "Webcam", "Zweb"]
        );
    }

    #[test]
    fn try_exec_lookup() {
        assert!(found_in("sh", Some("/nowhere:/bin:/usr/bin")));
        assert!(!found_in("sh", Some("/nowhere")));
        assert!(!found_in("sh", None));
        assert!(found_in("/bin/sh", None));
        assert!(!found_in("/etc/hostname-not-there", None));
    }
}
