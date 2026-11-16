//! Input methods: text-input-v3 for apps, input-method-v2 for the input
//! method (fcitx5 and the like), its keyboard grab, the keys it hands back
//! and its candidate window (`input_method.rs`).

mod common;

use common::client::Ev;
use common::protocols::{keymap_of, virtual_keyboard};
use common::*;
use serde_json::Value;

/// The candidate window's color, and the pixel it shows as.
const IME_COLOR: [u8; 4] = [255, 40, 200, 90];
const IME_RGBA: [u8; 4] = [40, 200, 90, 255];
const LOCK_COLOR: [u8; 4] = [255, 10, 20, 30];

/// A window "a" focused, with an enabled text input whose text cursor is at
/// `cursor`; an input method that has seen it activate.
fn typing_in_a(
    cursor: (i32, i32, i32, i32),
) -> (
    TestPila,
    Client,
    wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::ZwpTextInputV3,
    Client,
    wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::ZwpInputMethodV2,
) {
    let mut pila = TestPila::start();
    let app = pila.client();
    app.window("a").size(400, 300).create();
    pila.wait_focus("a");
    let ime = pila.client();
    let input_method = ime.input_method();
    let text_input = app.text_input();
    app.wait_logged("text_input", "Enter");
    app.enable_text_input(&text_input, cursor);
    ime.wait_logged("input_method", "Activate");
    (pila, app, text_input, ime, input_method)
}

fn count(client: &Client, tag: &str, text: &str) -> usize {
    client
        .logged(tag)
        .iter()
        .filter(|e| e.contains(text))
        .count()
}

fn popups(state: &Value) -> Vec<Value> {
    state["input_method"]["popups"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn saw_key(client: &Client, key: u32, pressed: bool) -> bool {
    client.events().contains(&Ev::Key { key, pressed })
}

/// evdev keycodes.
const KEY_Q: u32 = 16;
const KEY_N: u32 = 49;

#[test]
fn an_app_gets_the_text_the_input_method_commits() {
    let (mut pila, app, _text_input, ime, input_method) = typing_in_a((10, 10, 2, 20));
    let state = pila.state();
    assert_eq!(state["input_method"]["present"], true);
    assert_eq!(state["input_method"]["text_input_focus"], "a");
    ime.commit_text(&input_method, "你好");
    app.wait_logged("text_input", "你好");
}

#[test]
fn the_input_method_follows_the_keyboard_focus() {
    let (mut pila, app, _text_input, ime, _input_method) = typing_in_a((10, 10, 2, 20));
    let other = pila.client();
    other.window("b").create();
    pila.wait_focus("b");
    ime.wait_logged("input_method", "Deactivate");
    app.wait_logged("text_input", "Leave");
    let text_input = other.text_input();
    other.wait_logged("text_input", "Enter");
    other.enable_text_input(&text_input, (0, 0, 2, 20));
    ime.wait("activated again", |s| {
        s.log
            .iter()
            .filter(|l| l.tag == "input_method" && l.event.starts_with("Activate"))
            .count()
            == 2
    });
    pila.wait("b has it", |s| s["input_method"]["text_input_focus"] == "b");
}

#[test]
fn an_input_method_started_later_finds_the_focused_text_field() {
    let mut pila = TestPila::start();
    let app = pila.client();
    app.window("a").create();
    pila.wait_focus("a");
    let text_input = app.text_input();
    app.roundtrip();
    assert!(
        app.logged("text_input").is_empty(),
        "no enter without an input method"
    );
    let ime = pila.client();
    let _input_method = ime.input_method();
    app.wait_logged("text_input", "Enter");
    app.enable_text_input(&text_input, (0, 0, 2, 20));
    ime.wait_logged("input_method", "Activate");
}

#[test]
fn a_second_input_method_is_unavailable() {
    let (pila, _app, _text_input, _ime, _input_method) = typing_in_a((10, 10, 2, 20));
    let second = pila.client();
    let _other = second.input_method();
    second.wait_logged("input_method", "Unavailable");
}

#[test]
fn the_input_method_gets_the_keys_and_hands_back_those_it_doesnt_use() {
    let (mut pila, app, _text_input, ime, input_method) = typing_in_a((10, 10, 2, 20));
    let _grab = ime.grab_keyboard(&input_method);
    pila.wait("the input method's grab", |s| {
        s["keyboard_grab"] == "input_method"
    });
    app.clear_events();
    // Held down while the input method hands it back, as fcitx5 does with a
    // key it doesn't use: the window gets it once, and its release.
    pila.key_press("q");
    ime.wait("the press", |s| {
        s.log
            .iter()
            .any(|l| l.tag == "ime_grab" && l.event.contains("key: 16"))
    });
    assert!(
        !saw_key(&app, KEY_Q, true),
        "the window got the key past the input method"
    );
    let keyboard = virtual_keyboard(&ime, &keymap_of(&app));
    keyboard.key(1, KEY_Q, 1);
    ime.flush();
    app.wait("q from the input method", |s| {
        s.events.contains(&Ev::Key {
            key: KEY_Q,
            pressed: true,
        })
    });
    pila.key_release("q");
    ime.wait("the release", |s| {
        s.log
            .iter()
            .filter(|l| l.tag == "ime_grab" && l.event.contains("key: 16"))
            .count()
            == 2
    });
    keyboard.key(2, KEY_Q, 0);
    ime.flush();
    app.wait("its release", |s| {
        s.events.contains(&Ev::Key {
            key: KEY_Q,
            pressed: false,
        })
    });
    app.roundtrip();
    let presses = app
        .events()
        .iter()
        .filter(|e| {
            **e == Ev::Key {
                key: KEY_Q,
                pressed: true,
            }
        })
        .count();
    assert_eq!(presses, 1);
    // Nothing went round again to the input method.
    assert_eq!(count(&ime, "ime_grab", "key: 16"), 2);
}

#[test]
fn bindings_run_before_the_input_method_sees_the_keys() {
    let (mut pila, _app, _text_input, ime, input_method) = typing_in_a((10, 10, 2, 20));
    let _grab = ime.grab_keyboard(&input_method);
    pila.wait("the input method's grab", |s| {
        s["keyboard_grab"] == "input_method"
    });
    pila.combo("Super+n");
    pila.wait("minimized", |s| {
        window_in(s, "a").unwrap()["minimized"] == true
    });
    ime.roundtrip();
    assert_eq!(count(&ime, "ime_grab", &format!("key: {KEY_N},")), 0);
}

#[test]
fn the_candidate_window_goes_below_the_text_cursor() {
    let (mut pila, _app, _text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    let _popup = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    let state = pila.settle();
    let popup = &popups(&state)[0];
    assert_eq!(popup["parent"], "a");
    assert_eq!(
        popup["geometry"],
        serde_json::json!([x + 20, y + 30 + 20, 200, 50])
    );
    assert_eq!(pila.pixel((x + 30) as f64, (y + 60) as f64), IME_RGBA);
}

#[test]
fn the_candidate_window_stays_on_the_output() {
    let (mut pila, app, text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    let _popup = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    let state = pila.state();
    let (ow, oh) = (
        state["outputs"][0]["geometry"]["w"].as_i64().unwrap(),
        state["outputs"][0]["geometry"]["h"].as_i64().unwrap(),
    );
    // Near the bottom edge: above the cursor. (Once it's drawn where it
    // is: a frame on its way would show the move by itself.)
    pila.settle();
    let cursor_y = (oh - y - 30) as i32;
    app.move_text_cursor(&text_input, (20, cursor_y, 2, 20));
    let state = pila.wait("above", |s| {
        popups(s)
            .first()
            .is_some_and(|p| p["geometry"][1] == y + cursor_y as i64 - 50)
    });
    assert_eq!(popups(&state)[0]["geometry"][0], x + 20);
    // Moved with no commit of its own: redrawn there.
    pila.settle();
    let above = (x + 30, y + cursor_y as i64 - 40);
    assert_eq!(pila.pixel(above.0 as f64, above.1 as f64), IME_RGBA);
    // Near the right edge: slid left.
    let cursor_x = (ow - x - 50) as i32;
    app.move_text_cursor(&text_input, (cursor_x, 30, 2, 20));
    pila.wait("slid left", |s| {
        popups(s)
            .first()
            .is_some_and(|p| p["geometry"] == serde_json::json!([ow - 200, y + 50, 200, 50]))
    });
}

#[test]
fn the_candidate_window_goes_with_the_text_field() {
    let (mut pila, app, text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    let _popup = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    pila.settle();
    assert_eq!(pila.pixel((x + 30) as f64, (y + 60) as f64), IME_RGBA);
    app.disable_text_input(&text_input);
    ime.wait_logged("input_method", "Deactivate");
    pila.wait("no candidate window", |s| popups(s).is_empty());
    pila.settle();
    assert_ne!(pila.pixel((x + 30) as f64, (y + 60) as f64), IME_RGBA);
}

#[test]
fn a_click_on_the_candidate_window_goes_to_the_input_method() {
    let (mut pila, _app, _text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    let (surf, _popup) = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    pila.click_at((x + 30) as f64, (y + 60) as f64);
    ime.wait("the click", |s| {
        s.events
            .iter()
            .any(|e| matches!(e, Ev::Button { pressed: true, .. }))
    });
    let state = pila.settle();
    assert_eq!(state["focused"], "a");
    assert_eq!(state["input_method"]["text_input_focus"], "a");
    assert_eq!(count(&ime, "input_method", "Deactivate"), 0);
    let _ = surf;
}

#[test]
fn a_menu_opens_while_the_input_method_holds_the_keyboard() {
    let (mut pila, app, _text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let _grab = ime.grab_keyboard(&input_method);
    pila.wait("the input method's grab", |s| {
        s["keyboard_grab"] == "input_method"
    });
    let window = common::client::Window(0);
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    pila.move_to((x + 20) as f64, (y + 20) as f64);
    pila.press("left");
    app.wait("the press", |s| {
        s.events
            .iter()
            .any(|e| matches!(e, Ev::Button { pressed: true, .. }))
    });
    let (menu, _) = app.popup(
        "menu",
        Ok(window),
        &common::protocols::Placement::default(),
        true,
        [255, 0, 0, 255],
    );
    app.wait_surface_configured(menu);
    pila.release("left");
    let state = pila.wait("the menu's pointer grab", |s| {
        s["pointer"]["grab"] == "popup"
    });
    // The keyboard stays with the input method, and on the window.
    assert_eq!(state["keyboard_grab"], "input_method");
    assert_eq!(state["focused"], "a");
    // Its keys reach the window through the input method.
    let keyboard = virtual_keyboard(&ime, &keymap_of(&app));
    keyboard.key(1, KEY_Q, 1);
    keyboard.key(2, KEY_Q, 0);
    ime.flush();
    app.wait("the key", |s| {
        s.events.contains(&Ev::Key {
            key: KEY_Q,
            pressed: false,
        })
    });
    // A click outside closes it; the input method keeps the keyboard.
    pila.click_at(5.0, 795.0);
    app.wait_surface_closed(menu);
    let state = pila.wait("no menu grab", |s| s["pointer"]["grab"] != "popup");
    assert_eq!(state["keyboard_grab"], "input_method");
    assert_eq!(state["focused"], "a");
}

#[test]
fn a_menu_binding_doesnt_end_the_input_methods_grab() {
    let (mut pila, _app, _text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let _grab = ime.grab_keyboard(&input_method);
    pila.wait("the input method's grab", |s| {
        s["keyboard_grab"] == "input_method"
    });
    // Focusing another window dismisses menus (`dismiss_popup_grab`): not
    // the input method's grab.
    let other = pila.client();
    other.window("b").create();
    pila.wait_focus("b");
    let state = pila.settle();
    assert_eq!(state["keyboard_grab"], "input_method");
}

#[test]
fn the_input_method_sees_nothing_typed_on_the_lock_screen() {
    let (mut pila, _app, _text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let _grab = ime.grab_keyboard(&input_method);
    pila.wait("the input method's grab", |s| {
        s["keyboard_grab"] == "input_method"
    });
    let _popup = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    let locker = pila.client();
    let lock = locker.lock_session();
    let surf = locker.lock_surface(&lock, "HEADLESS-1", LOCK_COLOR);
    locker.wait_surface_configured(surf);
    locker.wait_logged("lock", "Locked");
    pila.wait("locked", |s| {
        s["locked"] == true && s["keyboard_focus"] == "lock"
    });
    // The lock screen's own text field doesn't bring the input method.
    let text_input = locker.text_input();
    locker.roundtrip();
    locker.enable_text_input(&text_input, (10, 10, 2, 20));
    let before = count(&ime, "ime_grab", "Key");
    pila.key("q");
    locker.wait("q on the lock surface", |s| {
        s.events.contains(&Ev::Key {
            key: KEY_Q,
            pressed: false,
        })
    });
    ime.roundtrip();
    assert_eq!(
        count(&ime, "ime_grab", "Key"),
        before,
        "the input method saw a key"
    );
    assert_eq!(count(&ime, "input_method", "Activate"), 1);
    assert!(!locker.has_logged("text_input", "Enter"));
    let state = pila.state();
    assert_eq!(state["input_method"]["text_input_focus"], Value::Null);
    assert_eq!(pila.pixel(1.0, 1.0), [10, 20, 30, 255]);
}

#[test]
fn a_launchers_candidate_window_goes_below_its_text_cursor() {
    use common::protocols::LayerSpec;
    use wayland_protocols_wlr::layer_shell::v1::client::{
        zwlr_layer_shell_v1::Layer, zwlr_layer_surface_v1::KeyboardInteractivity,
    };
    let mut pila = TestPila::start();
    let launcher = pila.client();
    let (surf, _layer) = launcher.layer_surface(
        "launcher",
        &LayerSpec {
            layer: Layer::Overlay,
            keyboard: KeyboardInteractivity::Exclusive,
            ..LayerSpec::default()
        },
        [255, 0, 0, 255],
    );
    launcher.wait_surface_configured(surf);
    pila.wait("the launcher has the keyboard", |s| {
        s["keyboard_focus"] == "layer:launcher"
    });
    let ime = pila.client();
    let input_method = ime.input_method();
    let text_input = launcher.text_input();
    launcher.wait_logged("text_input", "Enter");
    launcher.enable_text_input(&text_input, (10, 10, 2, 20));
    ime.wait_logged("input_method", "Activate");
    let _popup = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    let state = pila.settle();
    let layer = state["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["namespace"] == "launcher")
        .unwrap();
    let (x, y, ..) = rect(&layer["geometry"]);
    let popup = &popups(&state)[0];
    assert_eq!(popup["parent"], "launcher");
    assert_eq!(
        popup["geometry"],
        serde_json::json!([x + 10, y + 30, 200, 50])
    );
    assert_eq!(pila.pixel((x + 20) as f64, (y + 40) as f64), IME_RGBA);
}

#[test]
fn a_candidate_window_is_placed_in_a_client_decorated_windows_surface() {
    use common::client::Decorations;
    let mut pila = TestPila::start();
    let app = pila.client();
    // GTK's: a shadow 20 px wide around the window's geometry, so the text
    // cursor's coordinates, the surface's, are 20 px off the geometry's.
    app.window("a")
        .size(400, 300)
        .decorations(Decorations::Client)
        .csd_shadow(20)
        .create();
    pila.wait_focus("a");
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    let ime = pila.client();
    let input_method = ime.input_method();
    let text_input = app.text_input();
    app.wait_logged("text_input", "Enter");
    app.enable_text_input(&text_input, (30, 40, 2, 20));
    ime.wait_logged("input_method", "Activate");
    let _popup = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    let state = pila.settle();
    assert_eq!(
        popups(&state)[0]["geometry"],
        serde_json::json!([x - 20 + 30, y - 20 + 40 + 20, 200, 50])
    );
    assert_eq!(pila.pixel((x + 20) as f64, (y + 50) as f64), IME_RGBA);
}

#[test]
fn an_input_method_grabbing_in_an_open_menu_leaves_no_grab_behind() {
    // A popover with a text field: the menu holds the keyboard when the
    // input method takes it, then lets it go.
    let mut pila = TestPila::start();
    let app = pila.client();
    let window = app.window("a").size(400, 300).create();
    pila.wait_focus("a");
    let ime = pila.client();
    let input_method = ime.input_method();
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    pila.move_to((x + 20) as f64, (y + 20) as f64);
    pila.press("left");
    app.wait("the press", |s| {
        s.events
            .iter()
            .any(|e| matches!(e, Ev::Button { pressed: true, .. }))
    });
    let (menu, _) = app.popup(
        "menu",
        Ok(window),
        &common::protocols::Placement::default(),
        true,
        [255, 0, 0, 255],
    );
    app.wait_surface_configured(menu);
    pila.release("left");
    pila.wait("the menu's grab", |s| s["keyboard_grab"] == "popup");
    let grab = ime.grab_keyboard(&input_method);
    pila.wait("the input method's grab", |s| {
        s["keyboard_grab"] == "input_method"
    });
    grab.release();
    ime.flush();
    pila.wait("no keyboard grab", |s| s["keyboard_grab"].is_null());
    pila.click_at(5.0, 795.0);
    app.wait_surface_closed(menu);
    let state = pila.wait("no grab", |s| {
        s["pointer"]["grabbed"] == false && s["keyboard_grabbed"] == false
    });
    assert_eq!(state["focused"], "a");
    pila.key("q");
    app.wait("keys reach the window", |s| {
        s.events.contains(&Ev::Key {
            key: KEY_Q,
            pressed: false,
        })
    });
}

#[test]
fn a_candidate_window_growing_as_one_types_stays_on_the_output() {
    let (mut pila, app, text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    let (surf, _popup) = ime.ime_popup(&input_method, IME_COLOR, (100, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    let ow = pila.state()["outputs"][0]["geometry"]["w"]
        .as_i64()
        .unwrap();
    let cursor_x = (ow - x - 250) as i32;
    app.move_text_cursor(&text_input, (cursor_x, 30, 2, 20));
    pila.wait("at the cursor", |s| {
        popups(s)
            .first()
            .is_some_and(|p| p["geometry"][0] == ow - 250)
    });
    // More candidates: wider than the room left of the edge.
    ime.repaint(surf, (400, 50));
    pila.wait("slid left", |s| {
        popups(s)
            .first()
            .is_some_and(|p| p["geometry"] == serde_json::json!([ow - 400, y + 50, 400, 50]))
    });
    pila.settle();
    assert_eq!(pila.pixel((ow - 10) as f64, (y + 60) as f64), IME_RGBA);
}

#[test]
fn the_candidate_window_shows_again_in_the_next_text_field() {
    let (mut pila, app, text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    let _popup = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    app.disable_text_input(&text_input);
    pila.wait("gone", |s| popups(s).is_empty());
    pila.settle();
    assert_ne!(pila.pixel((x + 30) as f64, (y + 60) as f64), IME_RGBA);
    // The same place again: Smithay shows the same popup surface anew.
    app.enable_text_input(&text_input, (20, 30, 2, 20));
    pila.wait("back", |s| !popups(s).is_empty());
    pila.settle();
    assert_eq!(pila.pixel((x + 30) as f64, (y + 60) as f64), IME_RGBA);
}

#[test]
fn a_panels_menu_opened_while_typing_closes_when_the_focus_moves() {
    use common::protocols::LayerSpec;
    use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;
    let (mut pila, app, _text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    // A tooltip of the window being typed in: no menu, and not the panel's.
    let (tip, _) = app.popup(
        "tip",
        Ok(common::client::Window(0)),
        &common::protocols::Placement::default(),
        false,
        [255, 0, 0, 255],
    );
    app.wait_surface_configured(tip);
    let _grab = ime.grab_keyboard(&input_method);
    pila.wait("the input method's grab", |s| {
        s["keyboard_grab"] == "input_method"
    });
    let panel = pila.client();
    let (bar, _layer) = panel.layer_surface(
        "panel",
        &LayerSpec {
            anchor: Anchor::Top | Anchor::Left | Anchor::Right,
            size: (0, 30),
            exclusive_zone: 30,
            ..LayerSpec::default()
        },
        [255, 0, 0, 255],
    );
    panel.wait_surface_configured(bar);
    // Its buffer reaches Pila before the pointer moves there.
    panel.roundtrip();
    pila.settle();
    pila.move_to(20.0, 10.0);
    panel.wait("the pointer on the panel", |s| {
        s.pointer_name.as_deref() == Some("panel")
    });
    pila.press("left");
    panel.wait("the press", |s| {
        s.events
            .iter()
            .any(|e| matches!(e, Ev::Button { pressed: true, .. }))
    });
    let (menu, _) = panel.popup(
        "menu",
        Err(bar),
        &common::protocols::Placement::default(),
        true,
        [255, 0, 255, 0],
    );
    panel.wait_surface_configured(menu);
    pila.release("left");
    pila.wait("the menu's pointer grab", |s| {
        s["pointer"]["grab"] == "popup"
    });
    // Focusing another window closes it, though the keyboard isn't on it.
    let other = pila.client();
    other.window("b").create();
    pila.wait_focus("b");
    panel.wait_surface_closed(menu);
    app.roundtrip();
    assert!(
        !app.surface_data(tip, |s| s.closed),
        "the tooltip was closed"
    );
}

#[test]
fn a_candidate_window_gets_its_outputs_scale_before_its_first_picture() {
    let mut pila = TestPila::with_config("[[output]]\nname = \"HEADLESS-1\"\nscale = 2\n");
    let app = pila.client();
    app.window("a").create();
    pila.wait_focus("a");
    let ime = pila.client();
    let input_method = ime.input_method();
    let text_input = app.text_input();
    app.wait_logged("text_input", "Enter");
    app.enable_text_input(&text_input, (10, 10, 2, 20));
    ime.wait_logged("input_method", "Activate");
    // No frame to send it after: the monitor is off.
    let off = std::process::Command::new(env!("CARGO_BIN_EXE_pila"))
        .args(["msg", "action", "power-off-monitors"])
        .env_remove("PILA_SOCKET")
        .env("WAYLAND_DISPLAY", &pila.display)
        .env("XDG_RUNTIME_DIR", &pila.dir)
        .status()
        .expect("run pila msg");
    assert!(off.success());
    pila.wait("off", |s| s["outputs"][0]["power"] == false);
    let (surf, _popup) = ime.ime_popup_unpainted(&input_method, IME_COLOR, (200, 50));
    ime.roundtrip();
    assert_eq!(ime.surface_data(surf, |s| s.preferred_scale), Some(2));
}

#[test]
fn closing_a_window_with_its_candidate_window_open_is_fine() {
    let (mut pila, app, text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let other = pila.client();
    other.window("b").create();
    pila.wait_focus("b");
    pila.combo("Super+Tab");
    pila.wait_focus("a");
    app.wait("the text field entered again", |s| {
        s.log
            .iter()
            .filter(|l| l.tag == "text_input" && l.event.starts_with("Enter"))
            .count()
            == 2
    });
    app.enable_text_input(&text_input, (20, 30, 2, 20));
    ime.wait("activated again", |s| {
        s.log
            .iter()
            .filter(|l| l.tag == "input_method" && l.event.starts_with("Activate"))
            .count()
            == 2
    });
    let (surf, _popup) = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    // The window goes with the candidate window on it; the input method
    // draws once more before it hears of it.
    app.destroy(common::client::Window(0));
    ime.repaint(surf, (220, 50));
    pila.wait_focus("b");
    pila.wait("no candidate window", |s| popups(s).is_empty());
    ime.repaint(surf, (240, 50));
    ime.roundtrip();
    let text_input = other.text_input();
    other.wait_logged("text_input", "Enter");
    other.enable_text_input(&text_input, (10, 10, 2, 20));
    let state = pila.wait("the candidate window on b", |s| !popups(s).is_empty());
    assert_eq!(popups(&state)[0]["parent"], "b");
    assert!(pila.alive());
}

#[test]
fn a_text_field_in_a_popup_gets_its_candidate_window_below_its_cursor() {
    let mut pila = TestPila::start();
    let app = pila.client();
    let window = app.window("a").size(400, 300).create();
    pila.wait_focus("a");
    let ime = pila.client();
    let input_method = ime.input_method();
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    pila.move_to((x + 20) as f64, (y + 20) as f64);
    pila.press("left");
    app.wait("the press", |s| {
        s.events
            .iter()
            .any(|e| matches!(e, Ev::Button { pressed: true, .. }))
    });
    // A popover: at (300, 10) of the window, with a shadow 5 px wide around
    // its geometry, as GTK's.
    let (menu, _) = app.popup(
        "menu",
        Ok(window),
        &common::protocols::Placement {
            size: (200, 150),
            anchor_rect: (300, 10, 1, 1),
            ..common::protocols::Placement::default()
        },
        true,
        [255, 0, 0, 255],
    );
    app.wait_surface_configured(menu);
    pila.release("left");
    pila.wait("the popup has the keyboard", |s| {
        s["keyboard_focus"] == "popup:a"
    });
    let xdg_surface = app
        .lock()
        .popup_xdg_surfaces
        .iter()
        .find(|(i, _)| *i == menu.0)
        .map(|(_, x)| x.clone())
        .unwrap();
    xdg_surface.set_window_geometry(5, 5, 190, 140);
    app.repaint(menu, (200, 150));
    let text_input = app.text_input();
    app.wait_logged("text_input", "Enter");
    app.enable_text_input(&text_input, (20, 30, 2, 20));
    ime.wait_logged("input_method", "Activate");
    let (candidates, _popup) = ime.ime_popup(&input_method, IME_COLOR, (100, 40));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    let state = pila.settle();
    // The popup's surface starts 5 px up and left of its place.
    let (sx, sy) = (x + 300 - 5, y + 10 - 5);
    assert_eq!(
        popups(&state)[0]["geometry"],
        serde_json::json!([sx + 20, sy + 30 + 20, 100, 40])
    );
    assert_eq!(pila.pixel((sx + 30) as f64, (sy + 60) as f64), IME_RGBA);
    // Wider than the room right of the text cursor: on the output, from
    // where the popup is on it.
    let ow = state["outputs"][0]["geometry"]["w"].as_i64().unwrap();
    assert!(sx + 20 + 600 > ow);
    ime.repaint(candidates, (600, 40));
    pila.wait("slid left", |s| {
        popups(s)
            .first()
            .is_some_and(|p| p["geometry"] == serde_json::json!([ow - 600, sy + 50, 600, 40]))
    });
    pila.settle();
    // It goes when the text field does.
    app.disable_text_input(&text_input);
    ime.wait_logged("input_method", "Deactivate");
    pila.wait("no candidate window", |s| popups(s).is_empty());
    pila.settle();
    assert_ne!(pila.pixel((sx + 30) as f64, (sy + 60) as f64), IME_RGBA);
}

#[test]
fn a_click_on_the_candidate_window_of_a_text_field_in_a_popover_leaves_it_open() {
    let mut pila = TestPila::start();
    let app = pila.client();
    let window = app.window("a").size(400, 300).create();
    pila.wait_focus("a");
    let ime = pila.client();
    let input_method = ime.input_method();
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    pila.move_to((x + 20) as f64, (y + 20) as f64);
    pila.press("left");
    app.wait("the press", |s| {
        s.events
            .iter()
            .any(|e| matches!(e, Ev::Button { pressed: true, .. }))
    });
    // A popover with a text field, as a GTK search entry in a menu.
    let (menu, _) = app.popup(
        "menu",
        Ok(window),
        &common::protocols::Placement {
            size: (200, 150),
            anchor_rect: (100, 10, 1, 1),
            ..common::protocols::Placement::default()
        },
        true,
        [255, 0, 0, 255],
    );
    app.wait_surface_configured(menu);
    pila.release("left");
    pila.wait("the popover's grab", |s| s["pointer"]["grab"] == "popup");
    let text_input = app.text_input();
    app.wait_logged("text_input", "Enter");
    app.enable_text_input(&text_input, (20, 30, 2, 20));
    ime.wait_logged("input_method", "Activate");
    let (_candidates, _popup) = ime.ime_popup(&input_method, IME_COLOR, (100, 40));
    let state = pila.wait("the candidate window", |s| !popups(s).is_empty());
    let geometry = &popups(&state)[0]["geometry"];
    let (px, py) = (geometry[0].as_i64().unwrap(), geometry[1].as_i64().unwrap());
    // The pointer over a candidate is the input method's (hover), and a click
    // there picks it: the popover stays, its text field keeps the input method.
    pila.move_to((px + 10) as f64, (py + 10) as f64);
    ime.wait("the pointer on the candidates", |s| {
        s.events
            .iter()
            .any(|e| matches!(e, Ev::PointerEnter { .. }))
    });
    assert_eq!(pila.state()["pointer"]["grab"], "popup");
    let deactivated = count(&ime, "input_method", "Deactivate");
    pila.click_at((px + 10) as f64, (py + 10) as f64);
    ime.wait("the click", |s| {
        s.events
            .iter()
            .any(|e| matches!(e, Ev::Button { pressed: true, .. }))
    });
    let state = pila.settle();
    assert_eq!(state["pointer"]["grab"], "popup", "the popover closed");
    assert_eq!(count(&ime, "input_method", "Deactivate"), deactivated);
    let _ = text_input;
}

#[test]
fn the_candidate_window_stays_clear_of_a_panel() {
    use common::protocols::LayerSpec;
    use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;
    let (mut pila, app, text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let panel = pila.client();
    let (bar, _layer) = panel.layer_surface(
        "panel",
        &LayerSpec {
            anchor: Anchor::Bottom | Anchor::Left | Anchor::Right,
            size: (0, 100),
            exclusive_zone: 100,
            ..LayerSpec::default()
        },
        [255, 0, 0, 255],
    );
    panel.wait_surface_configured(bar);
    panel.roundtrip();
    pila.settle();
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    let _popup = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    let oh = pila.state()["outputs"][0]["geometry"]["h"]
        .as_i64()
        .unwrap();
    // Room below on the output, but the panel is there, above the windows.
    let cursor_y = (oh - 100 - 60 - y) as i32;
    app.move_text_cursor(&text_input, (20, cursor_y, 2, 20));
    pila.wait("above the text cursor", |s| {
        popups(s).first().is_some_and(|p| {
            p["geometry"] == serde_json::json!([x + 20, y + cursor_y as i64 - 50, 200, 50])
        })
    });
}

#[test]
fn a_window_on_two_outputs_gets_its_candidate_window_where_the_text_is() {
    let mut pila = TestPila::start_with(Options {
        outputs: Some("A:1280x800,B:1280x800".into()),
        ..Options::default()
    });
    let app = pila.client();
    app.window("a").size(400, 300).create();
    pila.wait_focus("a");
    // Mostly on A, its right end on B.
    let (x, y, w, _) = rect(&pila.window("a")["geometry"]);
    let (tx, ty) = (x + 100, y - 10);
    pila.move_to(tx as f64, ty as f64);
    pila.press("left");
    pila.move_to((tx + 300) as f64, ty as f64);
    pila.move_to((1280 - 300 + 100) as f64, ty as f64);
    pila.release("left");
    let state = pila.wait("a across the outputs", |s| {
        window_in(s, "a").is_some_and(|a| a["geometry"]["x"] == 1280 - 300)
    });
    let (x, y, ..) = rect(&window_in(&state, "a").unwrap()["geometry"]);
    assert_eq!((x, w), (980, 400));
    let ime = pila.client();
    let input_method = ime.input_method();
    let text_input = app.text_input();
    app.wait_logged("text_input", "Enter");
    // The text cursor on B, 60 px in: a candidate window 200 px wide fits
    // there, and doesn't on A.
    app.enable_text_input(&text_input, (360, 30, 2, 20));
    ime.wait_logged("input_method", "Activate");
    let _popup = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    let state = pila.settle();
    assert_eq!(
        popups(&state)[0]["geometry"],
        serde_json::json!([x + 360, y + 50, 200, 50])
    );
}

#[test]
fn a_candidate_window_with_no_text_field_redraws_nothing() {
    let (mut pila, app, text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let (surf, _popup) = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    app.disable_text_input(&text_input);
    ime.wait_logged("input_method", "Deactivate");
    pila.wait("no candidate window", |s| popups(s).is_empty());
    let before = pila.settle()["outputs"][0].clone();
    // The input method draws it again, with no text field to show it at.
    ime.repaint(surf, (220, 50));
    ime.roundtrip();
    pila.stays("no redraw", std::time::Duration::from_millis(200), |s| {
        s["outputs"][0]["frames"] == before["frames"] && s["outputs"][0]["idle"] == before["idle"]
    });
}

#[test]
fn keys_released_for_the_lock_screen_dont_reach_the_input_method() {
    let (mut pila, _app, _text_input, ime, input_method) = typing_in_a((20, 30, 2, 20));
    let _grab = ime.grab_keyboard(&input_method);
    pila.wait("the input method's grab", |s| {
        s["keyboard_grab"] == "input_method"
    });
    let locker = pila.client();
    let lock = locker.lock_session();
    let surf = locker.lock_surface(&lock, "HEADLESS-1", LOCK_COLOR);
    locker.wait_surface_configured(surf);
    locker.wait_logged("lock", "Locked");
    pila.wait("locked", |s| {
        s["locked"] == true && s["keyboard_focus"] == "lock"
    });
    let before = count(&ime, "ime_grab", "Key");
    pila.key_press("q");
    locker.wait("q pressed on the lock surface", |s| {
        s.events.contains(&Ev::Key {
            key: KEY_Q,
            pressed: true,
        })
    });
    // The nested window losing the keyboard: every held key is released.
    pila.request(serde_json::json!({ "cmd": "release_keys" }));
    locker.wait("q released there", |s| {
        s.events.contains(&Ev::Key {
            key: KEY_Q,
            pressed: false,
        })
    });
    ime.roundtrip();
    assert_eq!(count(&ime, "ime_grab", "Key"), before);
}

/// GTK's text cursor is a line: its rectangle has no width. (Smithay dropped
/// such rectangles; Pila's fork of it keeps them, `Cargo.toml`.)
#[test]
fn a_text_cursor_with_no_width_places_the_candidate_window() {
    let (mut pila, _app, _text_input, ime, input_method) = typing_in_a((20, 30, 0, 20));
    let (x, y, ..) = rect(&pila.window("a")["geometry"]);
    let _popup = ime.ime_popup(&input_method, IME_COLOR, (200, 50));
    pila.wait("the candidate window", |s| !popups(s).is_empty());
    let state = pila.settle();
    assert_eq!(
        popups(&state)[0]["geometry"],
        serde_json::json!([x + 20, y + 30 + 20, 200, 50])
    );
}
