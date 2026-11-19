//! The media keys: `pila-osd play-pause|next|previous|stop` tells a media
//! player over MPRIS, the session bus's standard for them, so no
//! playerctl, then shows what it did: playing or paused, the next or the
//! previous track, stopped, with the track's title and artist (`Media`);
//! "no media player" when there's none. The one that's playing gets it,
//! else a paused one, else any; ties go by bus name.
//!
//! Players change state after they answer: the state shown is read again
//! until it changes (`settle`, up to `SETTLE`), else play-pause shows the
//! opposite of what was, and the next or previous track the player's name
//! (its new title not known yet).

use std::{collections::HashMap, time::Duration};

use zbus::{blocking::Connection, zvariant::OwnedValue};

/// MPRIS players' bus names start with this.
const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// How long a player has to answer: a hung one doesn't hang the key.
const TIMEOUT: Duration = Duration::from_secs(2);
/// How long a player has to show the change before the popup does.
const SETTLE: Duration = Duration::from_millis(400);
const POLL: Duration = Duration::from_millis(25);
/// The longest title handed to the popup (which shortens it further to fit).
const TITLE_CHARS: usize = 200;

/// What a media key asks of the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    Stop,
}

impl Command {
    pub fn named(name: &str) -> Option<Command> {
        match name {
            "play-pause" => Some(Command::PlayPause),
            "next" => Some(Command::Next),
            "previous" => Some(Command::Previous),
            "stop" => Some(Command::Stop),
            _ => None,
        }
    }

    /// Its method of MPRIS's player interface.
    fn method(self) -> &'static str {
        match self {
            Command::PlayPause => "PlayPause",
            Command::Next => "Next",
            Command::Previous => "Previous",
            Command::Stop => "Stop",
        }
    }
}

/// What the popup shows of a media key: its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Playing,
    Paused,
    Stopped,
    Next,
    Previous,
    NoPlayer,
}

const STATES: [(State, &str); 6] = [
    (State::Playing, "playing"),
    (State::Paused, "paused"),
    (State::Stopped, "stopped"),
    (State::Next, "next"),
    (State::Previous, "previous"),
    (State::NoPlayer, "none"),
];

impl State {
    fn name(self) -> &'static str {
        STATES
            .iter()
            .find(|(s, _)| *s == self)
            .map_or("none", |(_, n)| n)
    }

    fn named(name: &str) -> Option<State> {
        STATES.iter().find(|(_, n)| *n == name).map(|(s, _)| *s)
    }

    /// The word for it, without an icon font.
    pub fn word(self) -> &'static str {
        match self {
            State::Playing => "Playing",
            State::Paused => "Paused",
            State::Stopped => "Stopped",
            State::Next => "Next",
            State::Previous => "Previous",
            State::NoPlayer => "Media",
        }
    }
}

/// A media key's popup: what it did and to what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub state: State,
    /// "Title — Artist", the title alone, or the player's name; for no
    /// player, "No media player".
    pub title: String,
}

impl Media {
    /// The popup as a request to the run in the background: one line.
    pub fn encode(&self) -> String {
        format!("media {} {}\n", self.state.name(), self.title)
    }

    pub fn decode(text: &str) -> Option<Media> {
        let rest = text.strip_prefix("media ")?.trim_end_matches('\n');
        let (state, title) = rest.split_once(' ').unwrap_or((rest, ""));
        Some(Media {
            state: State::named(state)?,
            title: clean(title),
        })
    }
}

/// Tell the player on the session bus; what to show of it.
pub fn run(command: Command) -> Result<Media, String> {
    let bus = zbus::blocking::connection::Builder::session()
        .and_then(|builder| builder.method_timeout(TIMEOUT).build())
        .map_err(|err| format!("no D-Bus session bus: {err}"))?;
    control(&bus, command)
}

/// `run`, on `bus`.
pub fn control(bus: &Connection, command: Command) -> Result<Media, String> {
    let dbus = zbus::blocking::fdo::DBusProxy::new(bus).map_err(|err| err.to_string())?;
    let names = dbus.list_names().map_err(|err| err.to_string())?;
    let mut players: Vec<(u8, String, Track)> = names
        .into_iter()
        .map(|name| name.to_string())
        .filter(|name| name.starts_with(PREFIX))
        .map(|name| {
            let track = Track::read(bus, &name);
            (track.rank(), name, track)
        })
        .collect();
    players.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let Some((_, name, before)) = players.into_iter().next() else {
        return Ok(Media {
            state: State::NoPlayer,
            title: "No media player".into(),
        });
    };
    bus.call_method(
        Some(name.as_str()),
        PATH,
        Some(PLAYER),
        command.method(),
        &(),
    )
    .map_err(|err| format!("{name}: {err}"))?;
    let after = settle(bus, &name, &before, command);
    Ok(shown(&name, command, &before, &after))
}

/// What the popup shows, from the player's state before the command and
/// after it (as far as it changed in time).
fn shown(name: &str, command: Command, before: &Track, after: &Track) -> Media {
    let changed_track = after.id != before.id;
    let state = match command {
        Command::PlayPause => match after.status.as_deref() {
            Some(status) if after.status != before.status => state_of(status),
            // Not changed in time: the opposite of what it was.
            _ if before.status.as_deref() == Some("Playing") => State::Paused,
            _ => State::Playing,
        },
        Command::Next => State::Next,
        Command::Previous => State::Previous,
        Command::Stop => State::Stopped,
    };
    let title = match command {
        // The old track's title would be wrong.
        Command::Next | Command::Previous if !changed_track => player_name(name),
        _ if after.title.is_empty() => player_name(name),
        _ => after.title.clone(),
    };
    Media {
        state,
        title: clean(&title),
    }
}

fn state_of(status: &str) -> State {
    match status {
        "Playing" => State::Playing,
        "Paused" => State::Paused,
        _ => State::Stopped,
    }
}

/// Read the player again until the command's change shows, for `SETTLE`
/// at most; as it is then.
fn settle(bus: &Connection, name: &str, before: &Track, command: Command) -> Track {
    let started = std::time::Instant::now();
    loop {
        let now = Track::read(bus, name);
        let done = match command {
            Command::PlayPause | Command::Stop => now.status != before.status,
            Command::Next | Command::Previous => now.id != before.id,
        };
        if done || started.elapsed() >= SETTLE {
            return now;
        }
        std::thread::sleep(POLL);
    }
}

/// A player's state: its status and the track it's on.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Track {
    status: Option<String>,
    /// What tells tracks apart: `mpris:trackid`, else the title shown.
    id: Option<String>,
    /// "Title — Artist", the title alone, or empty.
    title: String,
}

impl Track {
    fn read(bus: &Connection, name: &str) -> Track {
        let get = |property: &str| -> Option<OwnedValue> {
            bus.call_method(
                Some(name),
                PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(PLAYER, property),
            )
            .ok()?
            .body()
            .deserialize::<OwnedValue>()
            .ok()
        };
        let status = get("PlaybackStatus").and_then(|v| String::try_from(v).ok());
        let metadata: HashMap<String, OwnedValue> = get("Metadata")
            .and_then(|v| HashMap::try_from(v).ok())
            .unwrap_or_default();
        let text = |key: &str| -> Option<String> {
            let value = metadata.get(key)?.try_clone().ok()?;
            String::try_from(value.clone()).ok().or_else(|| {
                // `mpris:trackid` is an object path.
                zbus::zvariant::OwnedObjectPath::try_from(value)
                    .ok()
                    .map(|p| p.to_string())
            })
        };
        let artists: Vec<String> = metadata
            .get("xesam:artist")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| Vec::<String>::try_from(v).ok())
            .unwrap_or_default();
        let title = text("xesam:title").unwrap_or_default();
        let artists = artists.join(", ");
        let title = match (title.is_empty(), artists.is_empty()) {
            (false, false) => format!("{title} — {artists}"),
            (false, true) => title,
            _ => String::new(),
        };
        let id = text("mpris:trackid").or_else(|| (!title.is_empty()).then(|| title.clone()));
        Track { status, id, title }
    }

    /// Which players go first: the one playing, then a paused one, then any
    /// (stopped, or not saying).
    fn rank(&self) -> u8 {
        match self.status.as_deref() {
            Some("Playing") => 0,
            Some("Paused") => 1,
            _ => 2,
        }
    }
}

/// The player's name from its bus name: `org.mpris.MediaPlayer2.firefox.instance_1_2`
/// is "Firefox".
fn player_name(bus_name: &str) -> String {
    let short = bus_name.strip_prefix(PREFIX).unwrap_or(bus_name);
    let short = short.split('.').next().unwrap_or(short);
    let mut chars = short.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A title as one line, not too long for the socket.
fn clean(title: &str) -> String {
    let one_line: String = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(TITLE_CHARS)
        .collect();
    one_line.trim().to_string()
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Stdio},
        sync::{Arc, Mutex},
    };

    use zbus::zvariant::{ObjectPath, Value};

    use super::*;

    /// A session bus of the test's own, gone when dropped (as the bar's
    /// tray tests'). `None` where there's no dbus-daemon.
    struct Bus {
        daemon: Child,
        address: String,
    }

    impl Bus {
        fn start() -> Option<Bus> {
            let mut daemon = std::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|err| eprintln!("skipped: no dbus-daemon ({err})"))
                .ok()?;
            let mut address = String::new();
            BufReader::new(daemon.stdout.take()?)
                .read_line(&mut address)
                .ok()?;
            Some(Bus {
                daemon,
                address: address.trim().to_string(),
            })
        }

        fn connect(&self) -> zbus::blocking::connection::Builder<'static> {
            zbus::blocking::connection::Builder::address(self.address.as_str())
                .unwrap()
                .method_timeout(TIMEOUT)
        }

        /// A player on the bus as `name`, in `status`, on track 1; what it
        /// was told goes to `told`.
        fn player(&self, name: &str, status: &str, told: &Arc<Mutex<Vec<String>>>) -> Connection {
            self.serve(FakePlayer::new(name, status, told))
        }

        fn serve(&self, player: FakePlayer) -> Connection {
            self.connect()
                .name(format!("{PREFIX}{}", player.name))
                .unwrap()
                .serve_at(PATH, player)
                .unwrap()
                .build()
                .unwrap()
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }

    /// A media player's MPRIS player interface, the part the keys use: it
    /// plays or pauses, goes to the next or previous track (numbered), or
    /// does nothing at all if `stuck`.
    struct FakePlayer {
        name: String,
        status: Mutex<String>,
        track: Mutex<i32>,
        stuck: bool,
        told: Arc<Mutex<Vec<String>>>,
    }

    impl FakePlayer {
        fn new(name: &str, status: &str, told: &Arc<Mutex<Vec<String>>>) -> FakePlayer {
            FakePlayer {
                name: name.to_string(),
                status: Mutex::new(status.to_string()),
                track: Mutex::new(1),
                stuck: false,
                told: told.clone(),
            }
        }

        fn tell(&self, what: &str) {
            self.told
                .lock()
                .unwrap()
                .push(format!("{} {what}", self.name));
        }

        fn set_status(&self, status: &str) {
            if !self.stuck {
                *self.status.lock().unwrap() = status.to_string();
            }
        }

        fn skip(&self, by: i32) {
            if !self.stuck {
                *self.track.lock().unwrap() += by;
            }
        }
    }

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl FakePlayer {
        fn play_pause(&self) {
            self.tell("PlayPause");
            let playing = *self.status.lock().unwrap() == "Playing";
            self.set_status(if playing { "Paused" } else { "Playing" });
        }

        fn next(&self) {
            self.tell("Next");
            self.skip(1);
        }

        fn previous(&self) {
            self.tell("Previous");
            self.skip(-1);
        }

        fn stop(&self) {
            self.tell("Stop");
            self.set_status("Stopped");
        }

        #[zbus(property)]
        fn playback_status(&self) -> String {
            self.status.lock().unwrap().clone()
        }

        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, OwnedValue> {
            let track = *self.track.lock().unwrap();
            let path = format!("/org/test/track/{track}");
            let value = |v: Value| OwnedValue::try_from(v).unwrap();
            HashMap::from([
                (
                    "mpris:trackid".to_string(),
                    value(Value::from(ObjectPath::try_from(path).unwrap())),
                ),
                (
                    "xesam:title".to_string(),
                    value(Value::from(format!("Song {track}"))),
                ),
                (
                    "xesam:artist".to_string(),
                    value(Value::from(vec!["Band".to_string(), "Guest".to_string()])),
                ),
            ])
        }
    }

    fn media(state: State, title: &str) -> Media {
        Media {
            state,
            title: title.into(),
        }
    }

    #[test]
    fn commands_by_name() {
        assert_eq!(Command::named("play-pause"), Some(Command::PlayPause));
        assert_eq!(Command::named("previous"), Some(Command::Previous));
        assert_eq!(Command::named("volume"), None);
    }

    #[test]
    fn popups_go_to_the_background_run_and_back() {
        for (state, _) in STATES {
            let shown = media(state, "Song — Band, Guest");
            assert_eq!(Media::decode(&shown.encode()), Some(shown));
        }
        assert_eq!(
            Media::decode("media paused\n"),
            Some(media(State::Paused, ""))
        );
        for wrong in ["", "media", "media loud x", "volume 4"] {
            assert_eq!(Media::decode(wrong), None, "{wrong:?}");
        }
        // One line, however the player wrote it.
        assert_eq!(clean(" a\nb\t"), "a b");
        assert_eq!(clean(&"x".repeat(500)).len(), TITLE_CHARS);
    }

    #[test]
    fn player_names() {
        assert_eq!(
            player_name("org.mpris.MediaPlayer2.firefox.instance_1_23"),
            "Firefox"
        );
        assert_eq!(player_name("org.mpris.MediaPlayer2.mpv"), "Mpv");
    }

    #[test]
    fn the_player_that_is_playing_is_told_and_shown() {
        let Some(bus) = Bus::start() else { return };
        let keys = bus.connect().build().unwrap();
        let told = Arc::new(Mutex::new(Vec::new()));
        // No player: said so, and no error.
        assert_eq!(
            control(&keys, Command::PlayPause),
            Ok(media(State::NoPlayer, "No media player"))
        );

        let _stopped = bus.player("aaa", "Stopped", &told);
        let _paused = bus.player("bbb", "Paused", &told);
        assert_eq!(
            control(&keys, Command::PlayPause),
            Ok(media(State::Playing, "Song 1 — Band, Guest"))
        );
        let _playing = bus.player("ccc", "Stopped", &told);
        assert_eq!(
            control(&keys, Command::Next),
            Ok(media(State::Next, "Song 2 — Band, Guest"))
        );
        assert_eq!(
            control(&keys, Command::Previous),
            Ok(media(State::Previous, "Song 1 — Band, Guest"))
        );
        assert_eq!(
            control(&keys, Command::PlayPause),
            Ok(media(State::Paused, "Song 1 — Band, Guest"))
        );
        assert_eq!(
            control(&keys, Command::Stop),
            Ok(media(State::Stopped, "Song 1 — Band, Guest"))
        );
        assert_eq!(
            *told.lock().unwrap(),
            [
                "bbb PlayPause",
                "bbb Next",
                "bbb Previous",
                "bbb PlayPause",
                "bbb Stop"
            ]
        );
    }

    #[test]
    fn any_player_when_none_says_it_plays() {
        let Some(bus) = Bus::start() else { return };
        let keys = bus.connect().build().unwrap();
        let told = Arc::new(Mutex::new(Vec::new()));
        let _b = bus.player("bbb", "Stopped", &told);
        let _a = bus.player("aaa", "Stopped", &told);
        control(&keys, Command::PlayPause).unwrap();
        assert_eq!(*told.lock().unwrap(), ["aaa PlayPause"]);
    }

    /// A player that's slow to change (or never does): play-pause shows the
    /// opposite of what was, the next track the player's name rather than
    /// the old title.
    #[test]
    fn a_player_slow_to_change_is_shown_what_was_asked() {
        let Some(bus) = Bus::start() else { return };
        let keys = bus.connect().build().unwrap();
        let told = Arc::new(Mutex::new(Vec::new()));
        let mut player = FakePlayer::new("slow", "Playing", &told);
        player.stuck = true;
        let _slow = bus.serve(player);
        assert_eq!(
            control(&keys, Command::PlayPause),
            Ok(media(State::Paused, "Song 1 — Band, Guest"))
        );
        assert_eq!(
            control(&keys, Command::Next),
            Ok(media(State::Next, "Slow"))
        );
    }
}
