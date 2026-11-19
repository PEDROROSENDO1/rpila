//! What the switcher knows, from Pila's IPC events (`docs/IPC.md`), and
//! nothing of Wayland or iced, so it's unit tested: the windows (their app
//! ids and titles, by id), the switch in progress and whether it's been on
//! long enough to show.

use std::collections::HashMap;

use serde_json::Value;

/// A window as the switcher shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Window {
    pub app_id: String,
    pub title: String,
    pub minimized: bool,
}

/// A row of the list: a window of the switch, in its order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub window: Window,
    pub selected: bool,
}

/// What an event changed, for the UI to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// A switch started: show it once it's still on after the delay, if
    /// it's still switch number `.0` then (`Model::is_due`).
    Started(u64),
    /// The switch went on (a step, a window gone) or a window changed.
    Updated,
    /// No switch any more.
    Ended,
    /// Nothing the switcher shows.
    Nothing,
}

#[derive(Debug, Default)]
pub struct Model {
    windows: HashMap<String, Window>,
    /// The switch's windows, most recent first, and the selected one's id.
    switch: Option<(Vec<String>, Option<String>)>,
    /// Counts switches, so that a delay ending finds out whether its switch
    /// is still the one going on.
    started: u64,
}

impl Model {
    /// Apply an event of Pila's: `windows` or `switch` (others are ignored).
    pub fn apply(&mut self, event: &Value) -> Change {
        match event["event"].as_str() {
            Some("windows") => {
                self.windows = event["windows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|w| Some((w["id"].as_str()?.to_string(), window(w))))
                    .collect();
                if self.switch.is_some() {
                    Change::Updated
                } else {
                    Change::Nothing
                }
            }
            Some("switch") => {
                let switch = &event["switch"];
                if !switch.is_object() {
                    return match self.switch.take() {
                        Some(_) => Change::Ended,
                        None => Change::Nothing,
                    };
                }
                let ids = switch["windows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id.as_str().map(str::to_string))
                    .collect();
                let selected = switch["selected"].as_str().map(str::to_string);
                let started = self.switch.is_none();
                self.switch = Some((ids, selected));
                if started {
                    self.started += 1;
                    Change::Started(self.started)
                } else {
                    Change::Updated
                }
            }
            _ => Change::Nothing,
        }
    }

    /// Whether switch number `started` is still going on, to be shown.
    pub fn is_due(&self, started: u64) -> bool {
        self.switch.is_some() && self.started == started
    }

    /// The rows to show: the switch's windows in its order. One the
    /// windows events haven't told of yet is shown with no title.
    pub fn rows(&self) -> Vec<Row> {
        let Some((ids, selected)) = &self.switch else {
            return Vec::new();
        };
        ids.iter()
            .map(|id| Row {
                id: id.clone(),
                window: self.windows.get(id).cloned().unwrap_or_default(),
                selected: selected.as_ref() == Some(id),
            })
            .collect()
    }

    /// The app ids of every window, for their icons.
    pub fn app_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.windows.values().map(|w| w.app_id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    }
}

fn window(fields: &Value) -> Window {
    let text = |name: &str| plain(fields[name].as_str().unwrap_or_default());
    Window {
        app_id: text("app_id"),
        title: text("title"),
        minimized: fields["minimized"] == true,
    }
}

/// Titles come from any program: no control characters drawn.
fn plain(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// The first row to show of `len`, `max` at a time, so that `selected` is
/// in view: from the top while it fits, else with it last.
pub fn first_shown(len: usize, max: usize, selected: Option<usize>) -> usize {
    let selected = selected.unwrap_or(0);
    if max == 0 || selected < max {
        return 0;
    }
    (selected + 1 - max).min(len.saturating_sub(max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn windows() -> Value {
        json!({ "event": "windows", "windows": [
            { "id": "1", "app_id": "foot", "title": "a shell", "minimized": false },
            { "id": "2", "app_id": "org.gnome.Nautilus", "title": "Home\u{1b}[2J", "minimized": true },
            { "id": "3", "app_id": "foot", "title": "", "minimized": false },
        ]})
    }

    fn switch(ids: &[&str], selected: &str) -> Value {
        json!({ "event": "switch", "switch": {
            "windows": ids, "selected": selected, "output": "eDP-1",
        }})
    }

    #[test]
    fn a_switch_starts_steps_and_ends() {
        let mut model = Model::default();
        assert_eq!(model.apply(&windows()), Change::Nothing);
        assert_eq!(
            model.apply(&json!({ "event": "switch", "switch": null })),
            Change::Nothing
        );
        assert_eq!(
            model.apply(&switch(&["3", "1", "2"], "1")),
            Change::Started(1)
        );
        assert!(model.is_due(1));
        let rows = model.rows();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["3", "1", "2"]);
        let selected: Vec<bool> = rows.iter().map(|r| r.selected).collect();
        assert_eq!(selected, [false, true, false]);
        assert_eq!(rows[1].window.title, "a shell");
        // Control characters left out; minimized marked.
        assert_eq!(rows[2].window.title, "Home[2J");
        assert!(rows[2].window.minimized);

        assert_eq!(model.apply(&switch(&["3", "1", "2"], "2")), Change::Updated);
        assert!(model.rows()[2].selected);
        assert_eq!(
            model.apply(&json!({ "event": "switch", "switch": null })),
            Change::Ended
        );
        assert!(model.rows().is_empty());
        assert!(!model.is_due(1));
    }

    #[test]
    fn a_delay_from_an_earlier_switch_is_stale() {
        let mut model = Model::default();
        model.apply(&switch(&["1", "2"], "2"));
        model.apply(&json!({ "event": "switch", "switch": null }));
        assert_eq!(model.apply(&switch(&["2", "1"], "1")), Change::Started(2));
        assert!(!model.is_due(1));
        assert!(model.is_due(2));
    }

    #[test]
    fn a_window_not_told_of_yet_has_no_title() {
        let mut model = Model::default();
        model.apply(&windows());
        model.apply(&switch(&["9", "1"], "9"));
        assert_eq!(model.rows()[0].window, Window::default());
        // Windows changing during a switch redraw it.
        assert_eq!(model.apply(&windows()), Change::Updated);
        assert_eq!(model.app_ids(), ["foot", "org.gnome.Nautilus"]);
    }

    #[test]
    fn the_selected_row_stays_in_view() {
        assert_eq!(first_shown(5, 8, Some(4)), 0);
        assert_eq!(first_shown(20, 8, Some(7)), 0);
        assert_eq!(first_shown(20, 8, Some(8)), 1);
        assert_eq!(first_shown(20, 8, Some(19)), 12);
        assert_eq!(first_shown(20, 8, None), 0);
    }
}
