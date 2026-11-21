//! Clients' shm buffers are copied to the GPU at their commits, not in the
//! frame that shows them (`upload.rs`), and only when a frame will show them:
//! `early_uploads` in the state counts those commits.

mod common;

use std::time::Duration;

use common::client::Window;
use common::*;

fn early_uploads(state: &Value) -> u64 {
    state["early_uploads"].as_u64().expect("early_uploads")
}

/// A window "a" shown and settled, its first frames drawn.
fn one_window() -> (TestPila, Client, Window) {
    let mut pila = TestPila::start();
    let client = pila.client();
    let window = client.window("a").size(400, 300).create();
    pila.wait_focus("a");
    client.roundtrip();
    pila.settle();
    (pila, client, window)
}

#[test]
fn a_shown_windows_new_buffer_is_uploaded_at_its_commit() {
    let (mut pila, client, window) = one_window();
    let before = early_uploads(&pila.settle());
    client.draw(window, None);
    client.roundtrip();
    pila.wait("the buffer uploaded at its commit", |s| {
        early_uploads(s) == before + 1
    });
}

#[test]
fn a_minimized_windows_buffer_isnt_uploaded_until_shown() {
    let (mut pila, client, window) = one_window();
    pila.combo("Super+n");
    pila.wait("minimized", |s| {
        window_in(s, "a").unwrap()["minimized"] == true
    });
    let before = early_uploads(&pila.settle());
    client.draw(window, None);
    client.roundtrip();
    pila.stays(
        "no upload for a window shown nowhere",
        Duration::from_millis(300),
        |s| early_uploads(s) == before,
    );
}

#[test]
fn a_window_on_another_workspace_isnt_uploaded() {
    let (mut pila, client, window) = one_window();
    pila.combo("Super+2");
    pila.wait("on workspace 2", |s| s["workspace"] == 2);
    let before = early_uploads(&pila.settle());
    client.draw(window, None);
    client.roundtrip();
    pila.stays(
        "no upload for a window on another workspace",
        Duration::from_millis(300),
        |s| early_uploads(s) == before,
    );
}

#[test]
fn commits_faster_than_frames_are_uploaded_once_between_frames() {
    let (mut pila, client, window) = one_window();
    let before = early_uploads(&pila.settle());
    client.draw_times(window, 3);
    client.roundtrip();
    let after = pila.settle();
    assert_eq!(
        early_uploads(&after),
        before + 1,
        "three commits with no frame between them were uploaded more than once"
    );
}

#[test]
fn a_window_behind_the_lock_screen_isnt_uploaded() {
    let (mut pila, apps, window) = one_window();
    let locker = pila.client();
    let lock = locker.lock_session();
    let surf = locker.lock_surface(&lock, "HEADLESS-1", [10, 20, 30, 255]);
    locker.wait_surface_configured(surf);
    locker.wait_logged("lock", "Locked");
    locker.roundtrip();
    pila.wait("locked", |s| {
        s["locked"] == true && s["lock_pending"] == false
    });
    let before = early_uploads(&pila.settle());
    apps.draw(window, None);
    apps.roundtrip();
    pila.stays(
        "no upload for a window behind the lock",
        Duration::from_millis(300),
        |s| early_uploads(s) == before,
    );
}
