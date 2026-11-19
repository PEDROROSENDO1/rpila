//! What Pila's own clients ask Pila itself, on its IPC socket (`docs/IPC.md`;
//! `PILA_SOCKET`, which Pila gives the programs it starts): its `[colors]`,
//! followed as the user saves new ones. Elsewhere (another compositor, no
//! `PILA_SOCKET`) or from a Pila too old to say, nothing comes and the
//! clients keep the defaults.
//!
//! pila-bar, pila-osd and pila-shot include this file (`#[path]`): one source
//! for all.

#![allow(dead_code)] // Each client uses part of it.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
};

use serde_json::Value;

use crate::style::Style;

/// Pila's colors as a `Style`, first as they are and then each time they
/// change, from a thread of its own; it ends with Pila.
pub fn styles(client: &'static str) -> impl iced::futures::Stream<Item = Style> {
    use iced::futures::StreamExt;
    events(client, &["colors"]).map(|event| Style::from_pila(&event["colors"]))
}

/// The events of `kinds`, first as things are and then each change, from a
/// thread of its own; none if Pila refuses (an older one, without them), and
/// they end with Pila.
pub fn events(
    client: &'static str,
    kinds: &'static [&'static str],
) -> impl iced::futures::Stream<Item = Value> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    if let Some(path) = std::env::var_os("PILA_SOCKET").filter(|p| !p.is_empty()) {
        let spawned = std::thread::Builder::new()
            .name(format!("{client}-ipc"))
            .spawn(move || {
                let Ok(mut stream) = UnixStream::connect(&path) else {
                    eprintln!(
                        "{client}: can't reach Pila: its {} aren't followed",
                        kinds.join(", ")
                    );
                    return;
                };
                let request = serde_json::json!({ "request": "subscribe", "events": kinds });
                if stream.write_all(format!("{request}\n").as_bytes()).is_err() {
                    return;
                }
                for event in follow(BufReader::new(stream), kinds) {
                    if sender.unbounded_send(event).is_err() {
                        return;
                    }
                }
            });
        if let Err(err) = spawned {
            eprintln!(
                "{client}: Pila's {} aren't followed: {err}",
                kinds.join(", ")
            );
        }
    }
    receiver
}

/// The events of `kinds` on a subscribed connection, after Pila's answer;
/// none if it refused.
fn follow<'a>(reader: impl BufRead + 'a, kinds: &'a [&str]) -> impl Iterator<Item = Value> + 'a {
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
        .filter(move |event| kinds.iter().any(|kind| event["event"] == *kind))
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
        let surfaces: Vec<Color> = follow(lines.as_bytes(), &["colors"])
            .map(|event| Style::from_pila(&event["colors"]).surface)
            .collect();
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
        assert_eq!(follow(lines.as_bytes(), &["colors"]).count(), 0);
    }
}
