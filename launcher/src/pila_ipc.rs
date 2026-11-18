//! What pila-launcher and pila-bar ask Pila itself, on its IPC socket
//! (`docs/IPC.md`; `PILA_SOCKET`, which Pila gives the programs it starts):
//! its `[colors]`, followed as the user saves new ones. Elsewhere (another
//! compositor, no `PILA_SOCKET`) or from a Pila too old to say, nothing
//! comes and the clients keep the defaults.
//!
//! pila-bar includes this file (`#[path]`): one source for both.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
};

use serde_json::Value;

use crate::style::Style;

/// Pila's colors as a `Style`, first as they are and then each time they
/// change, from a thread of its own; it ends with Pila.
pub fn styles(client: &'static str) -> impl iced::futures::Stream<Item = Style> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    if let Some(path) = std::env::var_os("PILA_SOCKET").filter(|p| !p.is_empty()) {
        let spawned = std::thread::Builder::new()
            .name(format!("{client}-colors"))
            .spawn(move || {
                let Ok(mut stream) = UnixStream::connect(&path) else {
                    eprintln!("{client}: can't reach Pila: its colors aren't followed");
                    return;
                };
                let request = "{\"request\": \"subscribe\", \"events\": [\"colors\"]}\n";
                if stream.write_all(request.as_bytes()).is_err() {
                    return;
                }
                for style in follow(BufReader::new(stream)) {
                    if sender.unbounded_send(style).is_err() {
                        return;
                    }
                }
            });
        if let Err(err) = spawned {
            eprintln!("{client}: Pila's colors aren't followed: {err}");
        }
    }
    receiver
}

/// The styles of the `colors` events on a subscribed connection, after
/// Pila's answer; none if it refused (an older Pila has no colors).
fn follow(reader: impl BufRead) -> impl Iterator<Item = Style> {
    let mut lines = reader
        .lines()
        .map_while(Result::ok)
        .map(|line| serde_json::from_str::<Value>(&line).unwrap_or_default());
    let accepted = lines.next().is_some_and(|answer| answer["ok"] == true);
    // Refused: nothing more read.
    accepted
        .then_some(lines)
        .into_iter()
        .flatten()
        .filter(|event| event["event"] == "colors")
        .map(|event| Style::from_pila(&event["colors"]))
}

#[cfg(test)]
mod pila_ipc_tests {
    use super::*;
    use iced::Color;

    #[test]
    fn colors_events_become_styles() {
        let lines = concat!(
            "{\"ok\": true, \"protocol\": 1}\n",
            "{\"event\": \"colors\", \"colors\": {\"bar\": \"#102030\"}}\n",
            "{\"event\": \"windows\", \"windows\": []}\n",
            "{\"event\": \"colors\", \"colors\": {\"bar\": \"#405060\"}}\n",
        );
        let surfaces: Vec<Color> = follow(lines.as_bytes()).map(|s| s.surface).collect();
        assert_eq!(
            surfaces,
            [
                Color::from_rgb8(0x10, 0x20, 0x30),
                Color::from_rgb8(0x40, 0x50, 0x60)
            ]
        );
    }

    #[test]
    fn an_older_pila_gives_none() {
        let lines = concat!(
            "{\"ok\": false, \"error\": \"no such events \\\"colors\\\"\"}\n",
            "{\"event\": \"colors\", \"colors\": {\"bar\": \"#102030\"}}\n",
        );
        assert_eq!(follow(lines.as_bytes()).count(), 0);
    }
}
