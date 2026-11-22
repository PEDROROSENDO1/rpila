//! Mod+Tab window switching and minimizing.

mod common;

use common::*;
use std::time::Duration;

/// Windows `names` opened in order (the last one focused).
fn windows(names: &[&str]) -> (TestPila, Client, Vec<common::client::Window>) {
    let mut pila = TestPila::start();
    let client = pila.client();
    let mut handles = Vec::new();
    for name in names {
        handles.push(client.window(name).create());
        pila.wait_focus(name);
    }
    (pila, client, handles)
}

fn history(state: &Value) -> Vec<String> {
    state["focus_history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn mod_tab_switches_to_the_previous_window_and_ends_on_release() {
    let (mut pila, _client, _) = windows(&["a", "b"]);
    pila.key_press("Super");
    pila.key("Tab");
    let state = pila.wait_focus("a");
    assert!(state["switch"].is_object(), "a switch in progress");
    // The history waits for the switch to end.
    assert_eq!(history(&state), vec!["b", "a"]);
    assert_eq!(stacking(&state), vec!["b", "a"]);
    pila.key_release("Super");
    let state = pila.wait("the switch to end", |s| s["switch"].is_null());
    assert_eq!(history(&state), vec!["a", "b"]);
    assert_eq!(state["focused"], "a");
}

#[test]
fn alt_tab_switches_windows_too_whatever_the_mod_key() {
    // Mod is Super here: Alt+Tab is a default of its own, as on other desktops.
    let (mut pila, _client, _) = windows(&["a", "b", "c"]);
    pila.key_press("Alt");
    pila.key("Tab");
    pila.wait_focus("b");
    pila.key("Tab");
    pila.wait_focus("a");
    pila.key_release("Alt");
    let state = pila.wait("the switch to end", |s| s["switch"].is_null());
    assert_eq!(history(&state), vec!["a", "c", "b"]);
    pila.key_press("Alt");
    pila.key_press("Shift");
    pila.key("Tab");
    pila.wait_focus("b");
    pila.key_release("Shift");
    pila.key_release("Alt");
    pila.wait("the switch to end", |s| s["switch"].is_null());
}

#[test]
fn alt_tab_switches_with_mod_set_to_alt() {
    // Mod+Tab and Alt+Tab are then the same combo, bound twice.
    let mut pila = TestPila::with_config("mod = \"alt\"\n");
    let client = pila.client();
    for name in ["a", "b"] {
        client.window(name).create();
        pila.wait_focus(name);
    }
    pila.combo("Alt+Tab");
    pila.wait_focus("a");
    pila.wait("the switch to end", |s| s["switch"].is_null());
    pila.combo("Alt+Shift+Tab");
    pila.wait_focus("b");
}

#[test]
fn alt_tab_set_to_none_goes_to_the_app() {
    const KEY_TAB: u32 = 15;
    let mut pila =
        TestPila::with_config("[bindings]\n\"Alt+Tab\" = \"none\"\n\"Alt+Shift+Tab\" = \"none\"\n");
    let client = pila.client();
    for name in ["a", "b"] {
        client.window(name).create();
        pila.wait_focus(name);
    }
    pila.combo("Alt+Tab");
    pila.combo("Alt+Shift+Tab");
    client.wait("both Tabs to reach the app", |s| {
        s.events
            .iter()
            .filter(|e| {
                **e == common::client::Ev::Key {
                    key: KEY_TAB,
                    pressed: false,
                }
            })
            .count()
            == 2
    });
    let state = pila.settle();
    assert_eq!(state["focused"], "b");
    assert!(state["switch"].is_null());
    // Mod+Tab still switches.
    pila.combo("Super+Tab");
    pila.wait_focus("a");
}

#[test]
fn a_window_closing_during_mod_tab_doesnt_upset_the_order() {
    let (mut pila, client, handles) = windows(&["a", "b", "c", "d"]);
    pila.key_press("Super");
    pila.key("Tab");
    pila.wait_focus("c");
    pila.key("Tab");
    pila.wait_focus("b");
    // One already passed goes away: the next step is still the one after b.
    client.destroy(handles[3]);
    client.flush();
    pila.wait("d is gone", |s| window_in(s, "d").is_none());
    pila.key("Tab");
    pila.wait_focus("a");
    pila.key_release("Super");
}

#[test]
fn holding_mod_and_tapping_tab_goes_further_back() {
    let (mut pila, _client, _) = windows(&["a", "b", "c"]);
    pila.key_press("Super");
    pila.key("Tab");
    pila.wait_focus("b");
    pila.key("Tab");
    pila.wait_focus("a");
    pila.key("Tab");
    // Wraps around to the start.
    pila.wait_focus("c");
    pila.key("Tab");
    pila.wait_focus("b");
    pila.key_release("Super");
    let state = pila.wait("the switch to end", |s| s["switch"].is_null());
    assert_eq!(history(&state), vec!["b", "c", "a"]);
}

#[test]
fn mod_shift_tab_goes_the_other_way() {
    let (mut pila, _client, _) = windows(&["a", "b", "c"]);
    pila.key_press("Super");
    pila.key_press("Shift");
    pila.key("Tab");
    pila.wait_focus("a");
    pila.key_release("Shift");
    pila.key_release("Super");
    let state = pila.wait("the switch to end", |s| s["switch"].is_null());
    assert_eq!(history(&state), vec!["a", "c", "b"]);
}

#[test]
fn mod_j_switches_too() {
    let (mut pila, _client, _) = windows(&["a", "b"]);
    pila.combo("Super+j");
    let state = pila.wait_focus("a");
    assert!(state["switch"].is_null());
    assert_eq!(history(&state), vec!["a", "b"]);
}

#[test]
fn a_click_ends_the_switch() {
    let (mut pila, _client, _) = windows(&["a", "b"]);
    pila.key_press("Super");
    pila.key("Tab");
    pila.wait_focus("a");
    pila.click("middle");
    let state = pila.wait("the switch to end", |s| s["switch"].is_null());
    assert_eq!(history(&state)[0], "a");
    pila.key_release("Super");
}

#[test]
fn with_one_window_mod_tab_does_nothing() {
    let (mut pila, _client, _) = windows(&["a"]);
    pila.key_press("Super");
    pila.key("Tab");
    let state = pila.settle();
    assert!(state["switch"].is_null());
    assert_eq!(state["focused"], "a");
    pila.key_release("Super");
}

#[test]
fn minimizing_focuses_the_most_recent_window_left() {
    let (mut pila, _client, _) = windows(&["a", "b", "c"]);
    // History c, b, a: minimizing c focuses b.
    pila.combo("Super+n");
    let state = pila.wait_focus("b");
    let c = window_in(&state, "c").unwrap();
    assert_eq!(c["minimized"], true);
    assert_eq!(c["mapped"], false);
    assert_eq!(c["stack"], Value::Null);
}

#[test]
fn mod_tab_shows_a_minimized_window_while_selected_and_keeps_it_when_ending_there() {
    let (mut pila, _client, _) = windows(&["a", "b"]);
    let before = rect(&pila.window("b")["geometry"]);
    pila.combo("Super+n");
    pila.wait_focus("a");
    pila.key_press("Super");
    pila.key("Tab");
    let state = pila.wait_focus("b");
    let b = window_in(&state, "b").unwrap();
    assert_eq!(b["minimized"], false);
    assert_eq!(rect(&b["geometry"]), before);
    // Moving on hides it again.
    pila.key("Tab");
    let state = pila.wait_focus("a");
    assert_eq!(window_in(&state, "b").unwrap()["minimized"], true);
    // Back to it, and the switch ends there: it stays.
    pila.key("Tab");
    pila.wait_focus("b");
    pila.key_release("Super");
    let state = pila.wait("the switch to end", |s| s["switch"].is_null());
    let b = window_in(&state, "b").unwrap();
    assert_eq!(b["minimized"], false);
    assert_eq!(b["mapped"], true);
}

#[test]
fn a_lone_minimized_window_comes_back_with_mod_tab() {
    let (mut pila, _client, _) = windows(&["a"]);
    pila.combo("Super+n");
    pila.wait("minimized", |s| s["focused"].is_null());
    pila.combo("Super+Tab");
    let state = pila.wait_focus("a");
    assert_eq!(window_in(&state, "a").unwrap()["minimized"], false);
}

#[test]
fn a_client_can_minimize_itself() {
    let (mut pila, client, handles) = windows(&["a", "b"]);
    client.request(handles[1], |t| t.set_minimized());
    let state = pila.wait_focus("a");
    assert_eq!(window_in(&state, "b").unwrap()["minimized"], true);
}

#[test]
fn a_minimized_window_whose_client_dies_is_forgotten() {
    let mut pila = TestPila::start();
    let other = pila.client();
    other.window("other").create();
    pila.wait_focus("other");
    let mut client = pila.client();
    client.window("a").create();
    pila.wait_focus("a");
    pila.combo("Super+n");
    pila.wait_focus("other");
    client.disconnect();
    pila.wait("a gone", |s| window_in(s, "a").is_none());
    pila.combo("Super+Tab");
    let state = pila.settle();
    assert_eq!(state["focused"], "other");
}

// --- The switcher Pila draws (`switcher_view.rs`) ----------------------------

const ACCENT: [u8; 4] = [68, 119, 204, 255];
/// `[colors] bar-inactive`, the panel's.
const PANEL: [u8; 4] = [38, 39, 43, 255];

/// Windows of their own colors (ARGB), opened in order, the last focused.
fn colored(windows: &[(&str, [u8; 4])]) -> (TestPila, Client, Vec<common::client::Window>) {
    let mut pila = TestPila::start();
    let client = pila.client();
    let mut handles = Vec::new();
    for (name, color) in windows {
        handles.push(client.window(name).color(*color).create());
        pila.wait_focus(name);
    }
    (pila, client, handles)
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() <= 3)
}

/// The tile of `app_id` in the switcher shown: its rect (x, y, w, h).
fn tile(state: &Value, app_id: &str) -> (f64, f64, f64, f64) {
    let tile = state["switcher_view"]["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["app_id"] == app_id)
        .unwrap_or_else(|| panic!("no tile of {app_id}: {}", state["switcher_view"]));
    let r = &tile["rect"];
    let f = |k: &str| r[k].as_f64().unwrap();
    (f("x"), f("y"), f("w"), f("h"))
}

/// The middle of a tile's preview.
fn preview_center(state: &Value, app_id: &str) -> (f64, f64) {
    let (x, y, ..) = tile(state, app_id);
    // Its padding, then half the preview (184 x 115).
    (x + 8.0 + 92.0, y + 8.0 + 57.0)
}

const RED: [u8; 4] = [255, 200, 30, 30];
const GREEN: [u8; 4] = [255, 30, 200, 30];
const BLUE: [u8; 4] = [255, 30, 30, 200];

#[test]
fn a_quick_mod_tab_shows_no_switcher() {
    let (mut pila, _client, _) = colored(&[("a", RED), ("b", GREEN)]);
    pila.key_press("Super");
    pila.key("Tab");
    pila.key_release("Super");
    pila.wait("the switch to end", |s| s["switch"].is_null());
    pila.stays("no switcher", Duration::from_millis(400), |s| {
        s["switcher_view"]["shown"] == false
    });
}

#[test]
fn a_held_mod_tab_shows_live_previews_centered_the_selected_one_in_the_accent_color() {
    let (mut pila, client, handles) = colored(&[("a", RED), ("b", GREEN), ("c", BLUE)]);
    client.subsurface(
        &client.surface(handles[0]),
        "mark",
        (100, 100),
        (100, 50),
        GREEN,
    );
    pila.settle();
    pila.key_press("Super");
    pila.key("Tab");
    let state = pila.wait("the switcher", |s| s["switcher_view"]["shown"] == true);
    let view = &state["switcher_view"];
    let apps: Vec<&str> = view["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["app_id"].as_str().unwrap())
        .collect();
    assert_eq!(apps, ["c", "b", "a"], "most recent first");
    // In the middle of the output.
    let output = &state["outputs"][0];
    let r = &view["rect"];
    let middle = |k: &str, size: &str| r[k].as_i64().unwrap() * 2 + r[size].as_i64().unwrap();
    let g = &output["geometry"];
    let center = |k: &str, size: &str| g[k].as_i64().unwrap() * 2 + g[size].as_i64().unwrap();
    assert!(
        (middle("x", "w") - center("x", "w")).abs() <= 1
            && (middle("y", "h") - center("y", "h")).abs() <= 1,
        "{view} on {output}"
    );
    // b is selected: its tile in the accent color around its preview.
    let (x, y, ..) = tile(&state, "b");
    pila.wait_pixel_is(x + 2.0, y + 2.0, "the accent color", |px| near(px, ACCENT));
    for (app, color) in [("a", [200, 30, 30, 255]), ("b", [30, 200, 30, 255])] {
        let (x, y) = preview_center(&state, app);
        pila.wait_pixel_is(x, y, &format!("{app}'s preview"), |px| near(px, color));
    }
    // Shrunk to fit as a whole, not cropped: a 400 x 300 window is 153 x
    // 115 in its 184 x 115 preview (0.383), the panel's color on either
    // side; its green mark (a 100 x 50 subsurface at 100, 100) is 38 x 19
    // at 38, 38 in it, with red around.
    let (x, y, ..) = tile(&state, "a");
    let px = pila.pixel(x + 8.0 + 5.0, y + 8.0 + 57.0);
    assert!(near(px, PANEL), "left of a's preview: {px:?}");
    let (left, top) = (x + 8.0 + 15.5, y + 8.0);
    pila.wait_pixel_is(left + 57.0, top + 47.0, "a's green mark, shrunk", |px| {
        near(px, [30, 200, 30, 255])
    });
    let px = pila.pixel(left + 100.0, top + 70.0);
    assert!(near(px, [200, 30, 30, 255]), "beside the mark, red: {px:?}");
    // Live: a drawing a new picture shows in its tile.
    client.with_window(handles[0], |w| w.color = BLUE);
    client.draw(handles[0], None);
    let (x, y) = preview_center(&state, "a");
    pila.wait_pixel_is(x, y, "a's new picture", |px| near(px, [30, 30, 200, 255]));
    pila.key_release("Super");
    pila.wait("the switcher gone", |s| {
        s["switcher_view"]["shown"] == false
    });
}

#[test]
fn a_click_on_a_tile_ends_the_switch_there_and_reaches_no_app() {
    let (mut pila, client, _) = colored(&[("a", RED), ("b", GREEN), ("c", BLUE)]);
    pila.key_press("Alt");
    pila.key("Tab");
    let state = pila.wait("the switcher", |s| s["switcher_view"]["shown"] == true);
    // The panel covers whatever is under it: no client gets the pointer.
    let (x, y) = preview_center(&state, "a");
    pila.move_to(x, y);
    pila.wait("no pointer focus on the panel", |s| {
        s["pointer"]["focus"].is_null()
    });
    client.roundtrip();
    let buttons = client.buttons().len();
    pila.click("left");
    let state = pila.wait("the switch to end on a", |s| {
        s["switch"].is_null() && s["focused"] == "a"
    });
    assert_eq!(state["switcher_view"]["shown"], false);
    pila.key_release("Alt");
    client.roundtrip();
    assert_eq!(client.buttons().len(), buttons, "{:?}", client.buttons());
}

#[test]
fn a_minimized_window_has_its_preview_too() {
    let (mut pila, _client, _) = colored(&[("b", GREEN), ("a", RED)]);
    pila.combo("Super+n");
    pila.wait("a minimized", |s| {
        window_in(s, "a").unwrap()["minimized"] == true
    });
    pila.key_press("Super");
    pila.key("Tab");
    let state = pila.wait("the switcher", |s| s["switcher_view"]["shown"] == true);
    let (x, y) = preview_center(&state, "a");
    pila.wait_pixel_is(x, y, "a's preview", |px| near(px, [200, 30, 30, 255]));
    pila.key_release("Super");
}

#[test]
fn a_preview_of_a_window_on_another_output_is_live_too() {
    // a on output B; the switcher on A, where the pointer is: a's commits
    // redraw B, and its tile on A only through the switcher.
    let mut pila = TestPila::start_with(Options {
        outputs: Some("A:1280x800,B:1280x800".into()),
        ..Options::default()
    });
    let client = pila.client();
    let a = client.window("a").color(RED).create();
    pila.wait_focus("a");
    pila.combo("Super+Shift+o");
    pila.wait("a on B", |s| window_in(s, "a").unwrap()["output"] == "B");
    pila.move_to(640.0, 400.0);
    client.window("b").color(GREEN).create();
    pila.wait_focus("b");
    pila.key_press("Super");
    pila.key("Tab");
    let state = pila.wait("the switcher", |s| s["switcher_view"]["shown"] == true);
    assert_eq!(state["switcher_view"]["output"], "A");
    let (x, y) = preview_center(&state, "a");
    pila.wait_pixel_is(x, y, "a's preview", |px| near(px, [200, 30, 30, 255]));
    // Back on b, so a stays on B while it draws.
    pila.key("Tab");
    pila.wait_focus("b");
    // Nothing else pending on A: only the switcher can redraw it now.
    pila.settle();
    client.with_window(a, |w| w.color = BLUE);
    client.draw(a, None);
    pila.wait_pixel_is(x, y, "a's new picture", |px| near(px, [30, 30, 200, 255]));
    pila.key_release("Super");
}

/// The switcher's panel has the windows' corners: by default the top two
/// rounded, as GTK's; square with no radius. Its border (`[colors] hover`)
/// shows at a corner that's square and not at one that's rounded.
#[test]
fn the_switcher_has_the_windows_corners() {
    const BORDER: [u8; 4] = [0x3f, 0x42, 0x49, 255];
    for (config, top_cut) in [("corner-radius = 8\n", true), ("", false)] {
        let mut pila = TestPila::with_config(config);
        let client = pila.client();
        for (name, color) in [("a", RED), ("b", GREEN)] {
            client.window(name).size(200, 150).color(color).create();
            pila.wait_focus(name);
        }
        pila.settle();
        pila.key_press("Super");
        pila.key("Tab");
        let state = pila.wait("the switcher", |s| s["switcher_view"]["shown"] == true);
        let r = &state["switcher_view"]["rect"];
        let f = |k: &str| r[k].as_f64().unwrap();
        let (x, y, h) = (f("x"), f("y"), f("h"));
        // The bottom corner stays square: its border is there.
        pila.wait_pixel(x - 1.0, y + h, BORDER);
        let top = pila.pixel(x - 1.0, y - 1.0);
        assert_eq!(top != BORDER, top_cut, "{config:?}: top corner {top:?}");
        pila.key_release("Super");
    }
}
