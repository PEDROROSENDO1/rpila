//! The shortcuts cheatsheet (`shortcuts_view.rs`): shown on the first run
//! (the welcome screen) and by `show-shortcuts` (Mod+?); hidden by any key
//! but a modifier, which goes nowhere else unless it's a binding, or by a
//! click, which goes nowhere else on the panel; never while locked.

mod common;

use common::*;

const KEY_ESC: u32 = 1;
const KEY_A: u32 = 30;
const BTN_LEFT: u32 = 0x110;

fn shown(state: &serde_json::Value) -> bool {
    state["shortcuts_view"]["shown"] == true
}

/// A window "a" that has the keyboard, under a cheatsheet shown by Mod+?.
fn shown_over_a_window(config: &str) -> (TestPila, Client) {
    let mut pila = TestPila::with_config(config);
    let client = pila.client();
    client.window("a").size(1280, 800).create();
    pila.wait_focus("a");
    pila.settle();
    client.roundtrip();
    pila.combo("Super+Shift+slash");
    pila.wait("shown", shown);
    (pila, client)
}

#[test]
fn the_first_run_welcomes_with_the_shortcuts() {
    let mut pila = TestPila::start_with(Options {
        default_config_path: true,
        ..Options::default()
    });
    pila.wait_log("created the config from the example");
    let state = pila.wait("welcome", shown);
    let view = &state["shortcuts_view"];
    assert_eq!(view["heading"], "Welcome to Pila");
    assert_eq!(view["output"], "HEADLESS-1");
    // Centered on the output, 1280x800.
    let (x, y, w, h) = rect(&view["rect"]);
    assert_eq!((x, 1280 - x - w), (x, x));
    assert!((y - (800 - y - h)).abs() <= 1, "{view}");
    let rows = view["rows"].as_array().unwrap();
    assert!(
        rows.contains(&serde_json::json!(["Super+Return", "Open a terminal"])),
        "{view}"
    );
    assert!(
        rows.contains(&serde_json::json!([
            "Super+Shift+/",
            "Show these shortcuts"
        ])),
        "{view}"
    );
    assert!(
        view["notes"][0].as_str().unwrap().starts_with("Settings: "),
        "{view}"
    );

    // Drawn, with its text: the heading's band isn't the desktop, and the
    // heading's line has more than the band's color in it.
    let desktop = pila.pixel(5.0, 5.0);
    let band_y = (y + 16 + 12) as f64;
    pila.wait_pixel_is((x + w - 5) as f64, band_y, "the heading's band", |p| {
        p != desktop
    });
    let band = pila.pixel((x + w - 5) as f64, band_y);
    let text = (x + 16..x + 160).any(|tx| pila.pixel(tx as f64, band_y) != band);
    assert!(text, "no heading text drawn");

    // Escape hides it, and what was under it shows again.
    pila.key("Escape");
    pila.wait("hidden", |s| !shown(s));
    pila.wait_pixel((x + w - 5) as f64, band_y, desktop);
}

#[test]
fn a_config_of_ones_own_starts_without_it_and_mod_question_mark_toggles_it() {
    let mut pila = TestPila::start_with(Options {
        default_config_path: true,
        config: "# mine\n".into(),
        ..Options::default()
    });
    pila.wait_log("watching the config for changes");
    pila.stays("not shown", std::time::Duration::from_millis(300), |s| {
        !shown(s)
    });
    pila.combo("Super+Shift+slash");
    let state = pila.wait("shown", shown);
    assert_eq!(state["shortcuts_view"]["heading"], "Shortcuts");
    // Only the welcome names the settings file.
    assert_eq!(
        state["shortcuts_view"]["notes"].as_array().unwrap().len(),
        1
    );
    pila.combo("Super+Shift+slash");
    pila.wait("hidden", |s| !shown(s));
}

#[test]
fn a_key_that_isnt_a_binding_hides_it_and_reaches_no_window() {
    let (mut pila, client) = shown_over_a_window("");
    // A modifier alone doesn't hide it.
    pila.key("Shift_L");
    pila.stays("shown", std::time::Duration::from_millis(200), shown);
    pila.key("Escape");
    pila.wait("hidden", |s| !shown(s));
    pila.settle();
    client.roundtrip();
    assert!(
        !client.keys().iter().any(|(k, _)| *k == KEY_ESC),
        "{:?}",
        client.keys()
    );
    // Hidden, keys reach the window again.
    pila.key("a");
    client.wait("the key", |s| {
        s.events.iter().any(|e| {
            *e == common::client::Ev::Key {
                key: KEY_A,
                pressed: false,
            }
        })
    });
}

#[test]
fn a_binding_hides_it_and_still_runs() {
    let (mut pila, _client) = shown_over_a_window(
        "[bindings]\n\"Mod+Return\" = \"spawn echo ran > \\\"$XDG_RUNTIME_DIR/out\\\"\"\n",
    );
    pila.combo("Super+Return");
    pila.wait("hidden", |s| !shown(s));
    assert_eq!(pila.wait_file("out"), "ran\n");
}

#[test]
fn the_rows_follow_the_bindings() {
    let (mut pila, _client) = shown_over_a_window(
        "[bindings]\n\"Mod+Return\" = \"none\"\n\"Mod+Shift+F12\" = \"quit\"\n",
    );
    let state = pila.state();
    let rows = state["shortcuts_view"]["rows"].as_array().unwrap().clone();
    assert!(!rows.iter().any(|r| r[1] == "Open a terminal"), "{rows:?}");
    // Named keys (F12) before letters (E).
    assert!(
        rows.contains(&serde_json::json!([
            "Super+Shift+F12, Super+Shift+E",
            "Quit Pila"
        ])),
        "{rows:?}"
    );
}

#[test]
fn a_click_on_it_hides_it_and_reaches_no_window() {
    let (mut pila, client) = shown_over_a_window("");
    let state = pila.state();
    let (x, y, ..) = rect(&state["shortcuts_view"]["rect"]);
    pila.click_at((x + 20) as f64, (y + 20) as f64);
    pila.wait("hidden", |s| !shown(s));
    pila.settle();
    client.roundtrip();
    assert!(client.buttons().is_empty(), "{:?}", client.buttons());
}

#[test]
fn a_click_beside_it_hides_it_and_reaches_the_window() {
    let (mut pila, client) = shown_over_a_window("");
    // Left of the panel, below the title bar.
    pila.click_at(100.0, 400.0);
    pila.wait("hidden", |s| !shown(s));
    client.wait("the click", |s| {
        s.events.iter().any(|e| {
            matches!(e, common::client::Ev::Button { button, pressed: false, .. } if *button == BTN_LEFT)
        })
    });
}

#[test]
fn locking_hides_it_and_it_doesnt_show_while_locked() {
    let (mut pila, _client) = shown_over_a_window("");
    let locker = pila.client();
    let lock = locker.lock_session();
    locker.lock_surface(&lock, "HEADLESS-1", [255, 10, 20, 30]);
    locker.wait_logged("lock", "Locked");
    pila.wait("locked and hidden", |s| s["locked"] == true && !shown(s));
    pila.combo("Super+Shift+slash");
    pila.stays("not shown", std::time::Duration::from_millis(300), |s| {
        !shown(s)
    });
}
