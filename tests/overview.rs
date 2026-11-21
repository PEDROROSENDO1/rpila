//! The overview (Mod+W): the workspace's windows spread out, picked with a
//! click or the arrows and Enter.

mod common;

use common::*;
use std::time::Duration;

/// Windows `names` opened in order (the last one focused).
fn windows(pila: &mut TestPila, names: &[&str]) -> (Client, Vec<common::client::Window>) {
    let client = pila.client();
    let mut handles = Vec::new();
    for name in names {
        handles.push(client.window(name).create());
        pila.wait_focus(name);
    }
    (client, handles)
}

/// `pila msg <args>` as a program in this session would run it.
fn msg(pila: &TestPila, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_pila"))
        .arg("msg")
        .args(args)
        .env_remove("PILA_SOCKET")
        .env("WAYLAND_DISPLAY", &pila.display)
        .env("XDG_RUNTIME_DIR", &pila.dir)
        .output()
        .expect("run pila msg")
}

/// `pila msg action <words>`, which must succeed.
#[track_caller]
fn action(pila: &TestPila, words: &[&str]) {
    let mut all = vec!["action"];
    all.extend_from_slice(words);
    let out = msg(pila, &all);
    assert!(out.status.success(), "{words:?}: {:?}", out);
}

fn open(pila: &mut TestPila) -> Value {
    pila.combo("Super+w");
    pila.wait("the overview", |s| s["overview"]["open"] == true)
}

/// The place of `app_id` in the overview: (x, y, w, h).
fn slot(state: &Value, app_id: &str) -> (i64, i64, i64, i64) {
    let slot = slots(state)
        .into_iter()
        .find(|s| s["app_id"] == app_id)
        .unwrap_or_else(|| panic!("no place for {app_id}: {}", state["overview"]));
    rect(&slot["rect"])
}

fn slots(state: &Value) -> Vec<Value> {
    state["overview"]["slots"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn center((x, y, w, h): (i64, i64, i64, i64)) -> (f64, f64) {
    ((x + w / 2) as f64, (y + h / 2) as f64)
}

/// The app got no key but Super (a modifier reaches it, as ever).
fn no_key_but_super(client: &Client) -> bool {
    const KEY_LEFTMETA: u32 = 125;
    client.keys().iter().all(|(key, _)| *key == KEY_LEFTMETA)
}

fn overlap(a: (i64, i64, i64, i64), b: (i64, i64, i64, i64)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

#[test]
fn mod_w_spreads_the_windows_out_and_escape_closes_it() {
    let mut pila = TestPila::start();
    let (client, _) = windows(&mut pila, &["a", "b", "c"]);
    let state = open(&mut pila);
    let output = rect(&state["outputs"][0]["geometry"]);
    let places: Vec<_> = ["a", "b", "c"].iter().map(|a| slot(&state, a)).collect();
    for (i, place) in places.iter().enumerate() {
        assert!(
            place.0 >= output.0
                && place.1 >= output.1
                && place.0 + place.2 <= output.0 + output.2
                && place.1 + place.3 <= output.1 + output.3,
            "{place:?} out of {output:?}"
        );
        for other in &places[i + 1..] {
            assert!(!overlap(*place, *other), "{place:?} over {other:?}");
        }
    }
    assert_eq!(state["overview"]["selected"], "c", "the focused one");
    // No client gets the pointer while it's open.
    let (x, y) = center(slot(&state, "a"));
    pila.move_to(x, y);
    pila.wait("no pointer focus", |s| s["pointer"]["focus"].is_null());
    pila.key("Escape");
    let state = pila.wait("closed", |s| s["overview"]["open"] == false);
    assert_eq!(state["focused"], "c", "the focus as it was");
    client.roundtrip();
    assert!(
        client.keys().iter().all(|k| k.0 != 1),
        "Escape reached no app: {:?}",
        client.keys()
    );
}

#[test]
fn a_click_on_a_window_picks_it_and_reaches_no_app() {
    let mut pila = TestPila::start();
    let (client, _) = windows(&mut pila, &["a", "b", "c"]);
    let state = open(&mut pila);
    let (x, y) = center(slot(&state, "a"));
    pila.move_to(x, y);
    pila.wait("a selected under the pointer", |s| {
        s["overview"]["selected"] == "a"
    });
    client.roundtrip();
    let buttons = client.buttons().len();
    pila.click("left");
    let state = pila.wait("closed on a", |s| {
        s["overview"]["open"] == false && s["focused"] == "a"
    });
    assert_eq!(stacking(&state).last().unwrap(), "a", "raised");
    client.roundtrip();
    assert_eq!(client.buttons().len(), buttons, "{:?}", client.buttons());
}

#[test]
fn a_click_beside_the_windows_closes_it() {
    let mut pila = TestPila::start();
    let (client, _) = windows(&mut pila, &["a", "b"]);
    open(&mut pila);
    pila.move_to(2.0, 400.0);
    client.roundtrip();
    let buttons = client.buttons().len();
    pila.click("left");
    let state = pila.wait("closed", |s| s["overview"]["open"] == false);
    assert_eq!(state["focused"], "b");
    client.roundtrip();
    assert_eq!(client.buttons().len(), buttons);
}

#[test]
fn arrows_and_enter_pick_a_window() {
    let mut pila = TestPila::start();
    let (client, _) = windows(&mut pila, &["a", "b", "c"]);
    let state = open(&mut pila);
    // From c, the arrow toward a's place.
    let (cx, _) = center(slot(&state, "c"));
    let (ax, _) = center(slot(&state, "a"));
    let toward_a = if ax < cx { "Left" } else { "Right" };
    // Each step leaves c; the one that reaches a is the last.
    for _ in 0..3 {
        if pila.state()["overview"]["selected"] == "a" {
            break;
        }
        pila.key(toward_a);
    }
    pila.wait("a selected", |s| s["overview"]["selected"] == "a");
    pila.key("Return");
    pila.wait("closed on a", |s| {
        s["overview"]["open"] == false && s["focused"] == "a"
    });
    client.roundtrip();
    assert!(no_key_but_super(&client), "{:?}", client.keys());
}

#[test]
fn a_key_that_isnt_a_binding_reaches_no_window_and_another_binding_closes_it_and_runs() {
    let mut pila = TestPila::start();
    let (client, _) = windows(&mut pila, &["a", "b"]);
    open(&mut pila);
    pila.key("x");
    pila.settle();
    client.roundtrip();
    assert!(no_key_but_super(&client), "{:?}", client.keys());
    assert_eq!(pila.state()["overview"]["open"], true);
    // Mod+M maximizes the focused window, once the overview is closed.
    pila.combo("Super+m");
    let state = pila.wait("closed and b maximized", |s| {
        s["overview"]["open"] == false && window_in(s, "b").unwrap()["maximized"] == true
    });
    assert_eq!(state["focused"], "b");
}

#[test]
fn mod_w_again_closes_it_and_pila_msg_toggles_it() {
    let mut pila = TestPila::start();
    let _client = windows(&mut pila, &["a"]);
    open(&mut pila);
    pila.combo("Super+w");
    pila.wait("closed", |s| s["overview"]["open"] == false);
    action(&pila, &["overview", "open"]);
    pila.wait("open", |s| s["overview"]["open"] == true);
    action(&pila, &["overview"]);
    pila.wait("closed", |s| s["overview"]["open"] == false);
}

#[test]
fn each_output_shows_its_own_windows() {
    let mut pila = TestPila::start_with(Options {
        outputs: Some("A:1280x800,B:1024x768".into()),
        ..Options::default()
    });
    let (_client, _) = windows(&mut pila, &["a", "b"]);
    // b to the second output.
    pila.combo("Super+Shift+o");
    pila.wait("b on B", |s| window_in(s, "b").unwrap()["output"] == "B");
    let state = open(&mut pila);
    let output_of = |app: &str| {
        slots(&state)
            .into_iter()
            .find(|s| s["app_id"] == app)
            .unwrap()["output"]
            .clone()
    };
    assert_eq!(output_of("a"), "A");
    assert_eq!(output_of("b"), "B");
    let b = slot(&state, "b");
    assert!(b.0 >= 1280, "on B: {b:?}");
}

#[test]
fn a_minimized_window_has_a_place_and_picking_it_brings_it_back() {
    let mut pila = TestPila::start();
    let _client = windows(&mut pila, &["a", "b"]);
    pila.combo("Super+n");
    pila.wait("b minimized", |s| {
        window_in(s, "b").unwrap()["minimized"] == true
    });
    let state = open(&mut pila);
    let (x, y) = center(slot(&state, "b"));
    pila.click_at(x, y);
    let state = pila.wait("b back and focused", |s| {
        s["overview"]["open"] == false && s["focused"] == "b"
    });
    assert_eq!(window_in(&state, "b").unwrap()["minimized"], false);
}

#[test]
fn a_window_opening_or_closing_meanwhile_takes_or_leaves_its_place() {
    let mut pila = TestPila::start();
    let (client, handles) = windows(&mut pila, &["a", "b"]);
    open(&mut pila);
    client.window("c").create();
    let state = pila.wait("c's place", |s| slots(s).iter().any(|s| s["app_id"] == "c"));
    assert_eq!(state["overview"]["open"], true);
    client.destroy(handles[0]);
    let state = pila.wait("a's place gone", |s| {
        !slots(s).iter().any(|s| s["app_id"] == "a")
    });
    let places: Vec<_> = slots(&state).iter().map(|s| rect(&s["rect"])).collect();
    assert_eq!(places.len(), 2);
    assert!(!overlap(places[0], places[1]));
}

#[test]
fn the_windows_are_live_in_it() {
    let mut pila = TestPila::start();
    let client = pila.client();
    let a = client.window("a").color([255, 200, 30, 30]).create();
    pila.wait_focus("a");
    let state = open(&mut pila);
    let (x, y) = center(slot(&state, "a"));
    pila.wait_pixel_is(x, y, "a's picture", |px| px[0] > 150 && px[2] < 80);
    client.with_window(a, |w| w.color = [255, 30, 30, 200]);
    client.draw(a, None);
    pila.wait_pixel_is(x, y, "a's new picture", |px| px[2] > 150 && px[0] < 80);
}

#[test]
fn going_to_another_workspace_shows_its_windows_in_it() {
    let mut pila = TestPila::start();
    let client = pila.client();
    client.window("a").create();
    pila.wait_focus("a");
    pila.combo("Super+2");
    pila.wait("workspace 2", |s| s["workspace"] == 2);
    client.window("b").create();
    pila.wait_focus("b");
    open(&mut pila);
    pila.combo("Super+1");
    let state = pila.wait("a's place on workspace 1", |s| {
        s["workspace"] == 1 && slots(s).iter().any(|s| s["app_id"] == "a")
    });
    assert_eq!(state["overview"]["open"], true);
    assert!(!slots(&state).iter().any(|s| s["app_id"] == "b"));
}

#[test]
fn locking_closes_it_and_it_doesnt_open_while_locked() {
    let mut pila = TestPila::start();
    let _client = windows(&mut pila, &["a"]);
    open(&mut pila);
    let locker = pila.client();
    let lock = locker.lock_session();
    locker.lock_surface(&lock, "HEADLESS-1", [255, 0, 0, 0]);
    pila.wait("locked, overview closed", |s| {
        s["locked"] == true && s["overview"]["open"] == false
    });
    // `pila msg` refuses it while locked.
    let out = msg(&pila, &["action", "overview", "open"]);
    assert!(!out.status.success());
    pila.combo("Super+w");
    pila.stays("closed while locked", Duration::from_millis(300), |s| {
        s["overview"]["open"] == false
    });
}
