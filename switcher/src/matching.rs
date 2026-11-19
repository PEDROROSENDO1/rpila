//! Which app a window belongs to, by its app id (Wayland) or class (X11):
//! for its icon.

use crate::apps::App;

/// The desktop entry of the app whose windows have `app_id`: the entry of
/// that ID (`org.gnome.Nautilus` for `org.gnome.Nautilus.desktop`), else one
/// that says so (`StartupWMClass`), else one whose ID's last part is it
/// (`nautilus`, an older app id). Case doesn't matter past the first.
pub fn app_for<'a>(apps: &'a [App], app_id: &str) -> Option<&'a App> {
    if app_id.is_empty() {
        return None;
    }
    let id = |app: &'a App| app.id.strip_suffix(".desktop").unwrap_or(&app.id);
    apps.iter()
        .find(|app| id(app) == app_id)
        .or_else(|| apps.iter().find(|app| id(app).eq_ignore_ascii_case(app_id)))
        .or_else(|| {
            apps.iter().find(|app| {
                app.wm_class
                    .as_deref()
                    .is_some_and(|class| class.eq_ignore_ascii_case(app_id))
            })
        })
        .or_else(|| {
            apps.iter().find(|app| {
                id(app)
                    .rsplit('.')
                    .next()
                    .is_some_and(|last| last.eq_ignore_ascii_case(app_id))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, wm_class: Option<&str>) -> App {
        App {
            id: format!("{id}.desktop"),
            name: id.into(),
            generic_name: String::new(),
            keywords: Vec::new(),
            exec: id.into(),
            icon: Some(id.into()),
            terminal: false,
            path: None,
            wm_class: wm_class.map(str::to_string),
        }
    }

    #[test]
    fn a_window_finds_its_app() {
        let apps = [
            app("org.gnome.Nautilus", None),
            app("foot", None),
            app("code", Some("Code")),
            app("org.kde.Kate", None),
            app("Kate", None),
        ];
        let found = |app_id| app_for(&apps, app_id).map(|a| a.id.as_str());
        assert_eq!(
            found("org.gnome.Nautilus"),
            Some("org.gnome.Nautilus.desktop")
        );
        assert_eq!(found("foot"), Some("foot.desktop"));
        assert_eq!(found("Foot"), Some("foot.desktop"));
        // An X11 or Electron class, by StartupWMClass.
        assert_eq!(found("code"), Some("code.desktop"));
        assert_eq!(found("Code"), Some("code.desktop"));
        // The last part of a reverse-DNS ID, after any exact match.
        assert_eq!(found("nautilus"), Some("org.gnome.Nautilus.desktop"));
        assert_eq!(found("Kate"), Some("Kate.desktop"));
        assert_eq!(found("xterm"), None);
        assert_eq!(found(""), None);
    }
}
