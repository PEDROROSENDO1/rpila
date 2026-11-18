//! The nested backend (winit), in a headless Pila as its host.

mod common;

use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use common::TestPila;

/// How long the nested Pila may take to start, and to quit.
const TIMEOUT: Duration = Duration::from_secs(20);

#[test]
fn a_nested_pila_quits_when_its_host_goes() {
    let host = TestPila::start();
    let socket = host.dir.join(&host.display);
    // Apart from the host's directory, which goes with it.
    let dir = std::env::temp_dir().join(format!("pila-nested-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.toml");
    std::fs::write(&config, "autostart = []\n").unwrap();
    let log_path = dir.join("pila.log");
    let mut nested = Command::new(env!("CARGO_BIN_EXE_pila"))
        .args(["--backend", "winit", "--config"])
        .arg(&config)
        .env("WAYLAND_DISPLAY", &socket)
        .env_remove("DISPLAY")
        .env("XDG_RUNTIME_DIR", &dir)
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("RUST_LOG", "info")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log_path).unwrap())
        .spawn()
        .expect("start the nested pila");
    let log = || std::fs::read_to_string(&log_path).unwrap_or_default();

    // Up once its output exists: a window on the host, drawn with EGL.
    let started = Instant::now();
    while !log().contains("Creating new Output name=\"winit\"") {
        if let Ok(Some(status)) = nested.try_wait() {
            // No EGL for a window on the host here (no GPU, no software
            // rendering through wl_shm): nothing to test.
            eprintln!(
                "skipped: the nested pila couldn't start ({status}):\n{}",
                log()
            );
            return;
        }
        assert!(
            started.elapsed() < TIMEOUT,
            "the nested pila didn't start:\n{}",
            log()
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // The host goes (its session ended, it crashed): the nested Pila has no
    // window left and quits, with an error.
    drop(host);
    let started = Instant::now();
    let status = loop {
        if let Ok(Some(status)) = nested.try_wait() {
            break status;
        }
        if started.elapsed() > TIMEOUT {
            let _ = nested.kill();
            let _ = nested.wait();
            let text = log();
            panic!(
                "the nested pila runs on with its host gone ({} lines of \"Io error\"):\n{}",
                text.matches("Io error").count(),
                text.lines()
                    .filter(|l| !l.contains("Io error"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let text = log();
    assert!(
        !status.success(),
        "it quit as if asked to: {status}\n{text}"
    );
    assert!(text.contains("lost the host"), "{text}");
    assert!(!text.contains("panicked"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
