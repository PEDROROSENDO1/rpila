//! Tearing (wp-tearing-control, `tearing.rs`): which output's frames may tear.
//! The headless backend can't flip asynchronously; it says what the DRM
//! backend would ask for (`tears` in the state, `Scene::tears`).

mod common;

use common::*;
use serde_json::Value;
use wayland_protocols::wp::tearing_control::v1::client::wp_tearing_control_v1::PresentationHint;

fn tears(state: &Value) -> bool {
    state["outputs"][0]["tears"] == true
}

/// A fullscreen window "a" whose app asks for its frames to tear.
fn fullscreen_game(pila: &mut TestPila, client: &Client) -> common::client::Window {
    let window = client.window("a").fullscreen().create();
    pila.wait("fullscreen", |s| {
        window_in(s, "a").is_some_and(|w| rect(&w["geometry"]) == (0, 0, 1280, 800))
    });
    let surface = client.surface(window);
    let control = client.tearing_control(&surface);
    control.set_presentation_hint(PresentationHint::Async);
    surface.commit();
    client.flush();
    pila.wait("its output tears", tears);
    window
}

#[test]
fn a_fullscreen_window_that_asks_tears_from_its_next_commit() {
    let mut pila = TestPila::start();
    let client = pila.client();
    let window = client.window("a").fullscreen().create();
    pila.wait("fullscreen", |s| {
        window_in(s, "a").is_some_and(|w| rect(&w["geometry"]) == (0, 0, 1280, 800))
    });
    assert!(!tears(&pila.settle()), "not asked");
    let surface = client.surface(window);
    let control = client.tearing_control(&surface);
    control.set_presentation_hint(PresentationHint::Async);
    client.flush();
    pila.stays(
        "asked, not committed",
        std::time::Duration::from_millis(200),
        |s| !tears(s),
    );
    surface.commit();
    client.flush();
    pila.wait("committed", tears);
    // Back to vsync, from the commit after the object goes.
    control.destroy();
    client.flush();
    pila.stays(
        "destroyed, not committed",
        std::time::Duration::from_millis(200),
        tears,
    );
    surface.commit();
    client.flush();
    pila.wait("vsync again", |s| !tears(s));
}

#[test]
fn only_a_fullscreen_window_in_front_tears() {
    let mut pila = TestPila::start();
    let client = pila.client();
    let window = fullscreen_game(&mut pila, &client);
    // Not fullscreen any more.
    client.with_window(window, |w| w.toplevel.as_ref().unwrap().unset_fullscreen());
    client.flush();
    pila.wait("windowed", |s| !tears(s));
    client.with_window(window, |w| {
        w.toplevel.as_ref().unwrap().set_fullscreen(None)
    });
    client.flush();
    pila.wait("fullscreen again", tears);
    // Another window in front of it.
    let other = pila.client();
    other.window("b").size(300, 200).create();
    pila.wait_focus("b");
    pila.wait("covered", |s| !tears(s));
}

#[test]
fn tearing_false_in_the_config_turns_it_off() {
    let mut pila = TestPila::with_config("tearing = false\n");
    let client = pila.client();
    let window = client.window("a").fullscreen().create();
    pila.wait("fullscreen", |s| {
        window_in(s, "a").is_some_and(|w| rect(&w["geometry"]) == (0, 0, 1280, 800))
    });
    let surface = client.surface(window);
    let control = client.tearing_control(&surface);
    control.set_presentation_hint(PresentationHint::Async);
    surface.commit();
    client.flush();
    client.roundtrip();
    pila.stays("off", std::time::Duration::from_millis(300), |s| !tears(s));
}

#[test]
fn nothing_tears_while_locked() {
    let mut pila = TestPila::start();
    let client = pila.client();
    fullscreen_game(&mut pila, &client);
    let locker = pila.client();
    let lock = locker.lock_session();
    locker.lock_surface(&lock, "HEADLESS-1", [255, 0, 0, 0]);
    pila.wait("locked", |s| s["locked"] == true && !tears(s));
}

#[test]
fn a_second_tearing_control_for_a_surface_is_a_protocol_error() {
    let mut pila = TestPila::start();
    let client = pila.client();
    let window = client.window("a").create();
    pila.wait_focus("a");
    let surface = client.surface(window);
    let _first = client.tearing_control(&surface);
    client.roundtrip();
    client.expect_error();
    let _second = client.tearing_control(&surface);
    client.flush();
    let error = client.wait_error();
    // tearing_control_exists
    assert!(
        error.contains("error 0 on object wp_tearing_control_manager_v1"),
        "{error}"
    );
}

#[test]
fn a_window_over_the_whole_output_that_isnt_fullscreen_doesnt_tear() {
    let mut pila = TestPila::with_config(
        "[[window-rule]]\napp-id = \"a\"\ntitle-bar = false\nsize = [1280, 800]\nposition = [0, 0]\n",
    );
    let client = pila.client();
    let window = client.window("a").create();
    pila.wait("over the whole output", |s| {
        window_in(s, "a").is_some_and(|w| rect(&w["geometry"]) == (0, 0, 1280, 800))
    });
    let surface = client.surface(window);
    let control = client.tearing_control(&surface);
    control.set_presentation_hint(PresentationHint::Async);
    surface.commit();
    client.flush();
    client.roundtrip();
    pila.stays(
        "not fullscreen",
        std::time::Duration::from_millis(300),
        |s| !tears(s),
    );
}
