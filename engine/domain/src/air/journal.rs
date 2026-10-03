//! What just happened, in the order it happened.
//!
//! The engine narrates itself into a ring, and the info window reads it. It is
//! the one surface that answers "why is nothing going out" without a debugger:
//! a capture that was interrupted, a card that went up, a recording that
//! refused to start, all in the order they landed and with the time on them.
//!
//! Bounded on purpose. A live runs for hours and the last two hundred lines are
//! the ones anybody reads; keeping all of them is a slow leak that only shows
//! up on the longest session, which is the worst night to find it.
//!
//! Formatting happens on the way out rather than on the way in, so what is held
//! is the fact and the moment, and the shape of a line stays a decision this
//! module can change.

use std::collections::VecDeque;

use crate::protocol::{Command, Reply};
use crate::sound::audio_layers::Duck;

/// What a command that just ran should say in the journal.
///
/// Driven by the reply and not only by the command, because "go live" and "go
/// live, refused" are the two lines somebody is actually looking for and the
/// command alone cannot tell them apart. A command that changes nothing worth
/// remembering says nothing: a status poll every second would otherwise be the
/// only thing in here.
#[must_use]
pub fn said(command: &Command, reply: &Reply) -> Option<String> {
    // A verb on the preview says what the verb says, of the preview.
    if let Command::Staged { command } = command {
        return said(command, reply).map(|line| format!("preview: {line}"));
    }
    if let Reply::Error { message } = reply {
        return match command {
            Command::Status
            | Command::Watching { .. }
            | Command::Present
            | Command::Levels
            | Command::Sources
            | Command::Genres
            | Command::Mics
            | Command::Shot { .. }
            | Command::LayerShot { .. }
            | Command::Chat { .. }
            | Command::Events { .. }
            | Command::Plan
            | Command::Hide { .. }
            | Command::Categories { .. }
            | Command::Rewire => None,
            _ => Some(format!("! {}: {message}", verb(command))),
        };
    }
    let on = |on: &bool| if *on { "on" } else { "off" };
    let named = |device: &Option<String>| device.clone().unwrap_or_else(|| "none".into());
    Some(match command {
        Command::SceneCreate { name } => format!("scene {name} created"),
        Command::SceneDuplicate { name } => format!("scene {name} duplicated from the active one"),
        Command::SceneSwitch { name } => format!("scene switched to {name}"),
        Command::SceneDelete { name } => format!("scene {name} deleted"),
        Command::SceneStage { name } => format!("scene {name} staged"),
        Command::SceneTake => "staged scene taken to the air".into(),
        Command::SceneDraft { name, .. } => format!("scene {name} drafted in the preview"),
        Command::SceneRestore { name } => format!("scene {name} restored from the trash"),
        Command::SceneAdd { scene } => format!("scene {} added", scene.name),
        Command::Staged { .. } => return None,
        Command::AudioLayerAdd { id, .. } => format!("audio layer {id} added"),
        Command::AudioLayerRemove { id } => format!("audio layer {id} removed"),
        Command::AudioLayerMute { id, on } => {
            format!("audio layer {id} {}", if *on { "muted" } else { "open" })
        }
        Command::AudioLayerDuck { id, duck } => format!(
            "audio layer {id} {}",
            match duck {
                Duck::ByKind => "ducks by its kind",
                Duck::On => "ducks under the voice",
                Duck::Off => "stays level with the voice",
            }
        ),
        Command::GoLive | Command::Live { .. } => "\u{25b6} on air".into(),
        Command::Stop => "\u{25a0} off air".into(),
        Command::RecordStart => "\u{23fa} recording".into(),
        Command::RecordStop => "\u{23f9} recording stopped".into(),
        Command::Screen { display } => format!("screen: display {display}"),
        Command::Window { query } => format!("window: {query}"),
        Command::Camera { device } => format!("camera: {}", named(device)),
        Command::CameraPosition { at: Some(at) } => format!("camera at {},{}", at.x, at.y),
        Command::CameraPosition { at: None } => "camera position: default".into(),
        Command::CameraShape { shape } => format!("camera shape: {shape:?}"),
        Command::LayerCamera { id, .. }
        | Command::LayerWindow { id, .. }
        | Command::LayerScreen { id, .. }
        | Command::LayerImage { id, .. } => {
            format!("layer {id} added")
        }
        Command::LayerReplaceCamera { id, .. }
        | Command::LayerReplaceWindow { id, .. }
        | Command::LayerReplaceScreen { id, .. }
        | Command::LayerReplaceImage { id, .. } => format!("layer {id} source changed"),
        Command::LayerVisible { id, on } => {
            format!("layer {id} {}", if *on { "shown" } else { "hidden" })
        }
        Command::LayerRemove { id } => format!("layer {id} removed"),
        Command::LayerMove { id, index } => format!("layer {id} moved to {index}"),
        Command::LayerShape { id, shape } => format!("layer {id} shape: {shape:?}"),
        Command::LayerMirror { id, on } => format!("layer {id} mirror: {on}"),
        Command::LayerPosition { id, at } => match at {
            Some(at) => format!("layer {id} at {},{}", at.x, at.y),
            None => format!("layer {id} position: default"),
        },
        Command::LayerTransform { id, .. } => format!("layer {id} transformed"),
        Command::LayerCrop { id, crop } => format!(
            "layer {id} crop {}",
            if crop.is_some() { "set" } else { "off" }
        ),
        Command::Shader { path } => format!("shader: {}", named(path)),
        Command::LayerShader { id, path } => format!("layer {id} shader: {}", named(path)),
        Command::Hide { seq } => format!("chat: line {seq} hidden"),
        Command::Delete { seq } => format!("chat: line {seq} deleted on the platform"),
        // Not the words: they come back down as a line, and the journal is
        // what the engine did, not a second copy of the chat.
        Command::Say { channel, .. } => match channel {
            Some(channel) => format!("chat: said on {channel}"),
            None => "chat: said".into(),
        },
        Command::Mic { device } => format!("mic: {}", named(device)),
        Command::Mirror { on: flipped } => format!("mirror {}", on(flipped)),
        Command::Share { on: shared } => format!("screen {}", on(shared)),
        Command::Mute { on: muted } => format!("mic {}", if *muted { "muted" } else { "open" }),
        Command::Monitor { on: hearing } => format!("monitoring {}", on(hearing)),
        Command::Denoise { on: cleaned } => format!("denoise {}", on(cleaned)),
        Command::Hear { apps } if apps.is_empty() => "hearing the whole screen".into(),
        Command::Hear { apps } => format!("hearing {}", apps.join(", ")),
        Command::Clip { name } => format!("clip: {name}"),
        Command::StreamMusic { on: sent } => format!("music to the stream {}", on(sent)),
        Command::ScreenSound { on: sent } => {
            format!("the screen's sound to the stream {}", on(sent))
        }
        Command::LayerScreenSound { id, on: sent } => {
            format!("display layer {id} sound {}", on(sent))
        }
        Command::AppAudio { app } => format!("app audio: {}", named(app)),
        Command::Music { on: playing } => format!("music {}", on(playing)),
        Command::Genre { name } => format!("music: {name}"),
        Command::NextTrack => "music: next".into(),
        Command::SceneElementAdd { .. } => "scene element added".into(),
        Command::SceneElementSet { .. } => "scene element updated".into(),
        Command::SceneElementRemove { .. } => "scene element removed".into(),
        Command::SceneTimerStart { .. } => "scene timer started".into(),
        Command::SceneTimerStop { .. } => "scene timer stopped".into(),
        Command::HideEverything => "\u{2716} everything hidden".into(),
        Command::Retitle { adapter, title, .. } => match title {
            Some(title) => format!("destination {adapter} titled {title:?}"),
            None => format!("destination {adapter} described"),
        },
        Command::Announce { adapter } => format!("destination {adapter} told its title"),
        Command::Disconnect { adapter } => format!("destination {adapter} disconnected"),
        Command::Categorize { adapter, name, .. } => {
            format!("destination {adapter} filed under {name}")
        }
        Command::Sandbox { adapter, on } => format!(
            "destination {adapter} {}",
            if *on {
                "in the sandbox: nobody is told"
            } else {
                "out of the sandbox"
            }
        ),
        Command::Arm { adapter, on: armed } => {
            format!(
                "destination {adapter} {}",
                if *armed { "armed" } else { "disarmed" }
            )
        }
        // Sliders, polls and reads. A fader moved thirty times in a minute
        // would push everything worth reading off the top.
        Command::Status
        | Command::Watching { .. }
        | Command::Present
        | Command::Plan
        | Command::Levels
        | Command::Sources
        | Command::Genres
        | Command::Mics
        | Command::AudioLayerVolume { .. }
        | Command::Volume { .. }
        | Command::MusicVolume { .. }
        | Command::AppAudioVolume { .. }
        | Command::Duck { .. }
        | Command::Gate { .. }
        | Command::Shot { .. }
        | Command::LayerShot { .. }
        | Command::Grants
        | Command::Chat { .. }
        | Command::Events { .. }
        | Command::Categories { .. }
        | Command::Rewire
        | Command::Quit => return None,
    })
}

/// The name a failure is reported under, which is the word somebody typed.
fn verb(command: &Command) -> &'static str {
    match command {
        Command::SceneCreate { .. }
        | Command::SceneDuplicate { .. }
        | Command::SceneSwitch { .. }
        | Command::SceneDelete { .. }
        | Command::SceneStage { .. }
        | Command::SceneTake
        | Command::SceneDraft { .. }
        | Command::SceneRestore { .. }
        | Command::SceneAdd { .. }
        | Command::Staged { .. } => "scene",
        Command::AudioLayerAdd { .. }
        | Command::AudioLayerRemove { .. }
        | Command::AudioLayerVolume { .. }
        | Command::AudioLayerMute { .. }
        | Command::AudioLayerDuck { .. } => "audio layer",
        Command::GoLive | Command::Live { .. } => "go live",
        Command::Stop => "stop",
        Command::RecordStart | Command::RecordStop => "recording",
        Command::Screen { .. } => "screen",
        Command::Window { .. } => "window",
        Command::Camera { .. } => "camera",
        Command::CameraPosition { .. } => "camera-position",
        Command::CameraShape { .. } => "camera-shape",
        Command::LayerCamera { .. }
        | Command::LayerScreen { .. }
        | Command::LayerWindow { .. }
        | Command::LayerReplaceScreen { .. }
        | Command::LayerReplaceWindow { .. }
        | Command::LayerReplaceCamera { .. }
        | Command::LayerImage { .. }
        | Command::LayerReplaceImage { .. }
        | Command::LayerVisible { .. }
        | Command::LayerRemove { .. }
        | Command::LayerMove { .. }
        | Command::LayerTransform { .. }
        | Command::LayerCrop { .. }
        | Command::LayerShape { .. }
        | Command::LayerMirror { .. }
        | Command::LayerPosition { .. } => "layer",
        Command::Shader { .. } => "shader",
        Command::LayerShader { .. } => "layer-shader",
        Command::Mic { .. } => "mic",
        Command::Mirror { .. } => "mirror",
        Command::Share { .. } => "share",
        Command::Mute { .. } => "mute",
        Command::Monitor { .. } => "monitor",
        Command::Denoise { .. } => "denoise",
        Command::Hear { .. } => "hear",
        Command::Clip { .. } => "play",
        Command::StreamMusic { .. } => "stream-music",
        Command::ScreenSound { .. } | Command::LayerScreenSound { .. } => "screen-sound",
        Command::AppAudio { .. } => "app-audio",
        Command::AppAudioVolume { .. } => "app-audio-volume",
        Command::Music { .. } | Command::Genre { .. } | Command::NextTrack => "music",
        Command::Volume { .. } | Command::MusicVolume { .. } | Command::Duck { .. } => "volume",
        Command::Gate { .. } => "gate",
        Command::SceneElementAdd { .. }
        | Command::SceneElementSet { .. }
        | Command::SceneElementRemove { .. }
        | Command::SceneTimerStart { .. }
        | Command::SceneTimerStop { .. } => "scene",
        Command::HideEverything => "hide everything",
        Command::Arm { .. } => "arm",
        Command::Sandbox { .. } => "sandbox",
        Command::Disconnect { .. } => "disconnect",
        Command::Categorize { .. } => "category",
        Command::Categories { .. } => "categories",
        Command::Retitle { .. } => "title",
        Command::Announce { .. } => "announce",
        Command::Grants => "permissions",
        Command::Quit => "quit",
        Command::Rewire => "rewire",
        Command::Status
        | Command::Watching { .. }
        | Command::Levels
        | Command::Sources
        | Command::Genres
        | Command::Mics
        | Command::Shot { .. }
        | Command::Plan
        | Command::LayerShot { .. }
        | Command::Chat { .. }
        | Command::Events { .. } => "read",
        Command::Hide { .. } => "hide",
        Command::Delete { .. } => "delete",
        Command::Say { .. } => "say",
        Command::Present => "present",
    }
}

/// A ring of what the engine has done, newest last.
#[derive(Debug, Clone, Default)]
pub struct Journal {
    kept: VecDeque<(i64, String)>,
}

impl Journal {
    /// How many lines survive. Two hundred is about a screen and a half of
    /// scrollback, which is as far back as anybody looks while something is
    /// wrong.
    pub const KEPT: usize = 200;

    /// Note a line, at this many seconds past the epoch.
    pub fn note(&mut self, at: i64, line: impl Into<String>) {
        if self.kept.len() == Self::KEPT {
            let _ = self.kept.pop_front();
        }
        self.kept.push_back((at, line.into()));
    }

    /// The lines, stamped, oldest first.
    ///
    /// UTC, like a recording's name and for the same reason: local time makes
    /// one hour of the year appear twice, and a log where the clock goes
    /// backwards is a log nobody trusts again.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.kept
            .iter()
            .map(|(at, line)| format!("{} {line}", clock(*at)))
            .collect()
    }

    /// Whether anything has been noted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }
}

/// `HH:MM:SS` of a moment past the epoch, UTC.
///
/// Public because the daemon's own log stamps its lines the same way, and two
/// clocks that print the same thing differently is two clocks to reconcile
/// when somebody is comparing a crash against what the engine was doing.
#[must_use]
pub fn clock_of(unix_seconds: i64) -> String {
    clock(unix_seconds)
}

fn clock(unix_seconds: i64) -> String {
    let rest = unix_seconds.rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Framed;

    #[test]
    fn a_line_carries_the_time_it_happened() {
        let mut journal = Journal::default();
        journal.note(1_756_000_000, "screen: Built-in Retina Display");
        assert_eq!(
            journal.lines(),
            vec!["01:46:40 screen: Built-in Retina Display"]
        );
    }

    #[test]
    fn the_order_is_the_order_things_happened() {
        let mut journal = Journal::default();
        journal.note(0, "one");
        journal.note(1, "two");
        assert_eq!(journal.lines(), vec!["00:00:00 one", "00:00:01 two"]);
    }

    #[test]
    fn the_oldest_line_is_the_one_that_goes() {
        let mut journal = Journal::default();
        for n in 0..Journal::KEPT + 5 {
            journal.note(0, format!("line {n}"));
        }
        let lines = journal.lines();
        assert_eq!(lines.len(), Journal::KEPT);
        assert_eq!(lines.first().map(String::as_str), Some("00:00:00 line 5"));
        assert_eq!(lines.last().map(String::as_str), Some("00:00:00 line 204"));
    }

    #[test]
    fn midnight_is_the_start_of_a_day_and_not_a_negative_hour() {
        // A moment before the epoch still reads as a time of day. `%` alone
        // would give -1 here and print `-1:00:00`.
        assert_eq!(clock(-1), "23:59:59");
    }

    #[test]
    fn going_live_is_worth_a_line_and_a_status_poll_is_not() {
        assert_eq!(
            said(&Command::GoLive, &Reply::Ok).as_deref(),
            Some("\u{25b6} on air")
        );
        assert_eq!(said(&Command::Status, &Reply::Ok), None);
    }

    #[test]
    fn a_refusal_says_which_verb_refused_and_why() {
        let refused = Reply::Error {
            message: "no destination".into(),
        };
        assert_eq!(
            said(&Command::GoLive, &refused).as_deref(),
            Some("! go live: no destination")
        );
    }

    #[test]
    fn a_read_that_fails_is_still_not_a_line() {
        // Otherwise a panel polling `shot` once a second against an engine
        // with no capture fills the whole ring with one sentence.
        let refused = Reply::Error {
            message: "nothing to shoot".into(),
        };
        assert_eq!(
            said(
                &Command::Shot {
                    of: Framed::default()
                },
                &refused
            ),
            None
        );
    }

    #[test]
    fn a_device_turned_off_says_so_rather_than_saying_nothing() {
        assert_eq!(
            said(&Command::Camera { device: None }, &Reply::Ok).as_deref(),
            Some("camera: none")
        );
    }

    #[test]
    fn every_verb_names_itself_when_it_refuses() {
        // A failure that says "! read: ..." for something somebody typed is a
        // failure they cannot act on. This walks the ones a person presses.
        let refused = Reply::Error {
            message: "no".into(),
        };
        let cases: [(Command, &str); 8] = [
            (Command::Stop, "! stop: no"),
            (Command::Screen { display: 1 }, "! screen: no"),
            (
                Command::Window {
                    query: "Safari".into(),
                },
                "! window: no",
            ),
            (Command::RecordStart, "! recording: no"),
            (Command::Mirror { on: true }, "! mirror: no"),
            (Command::Monitor { on: true }, "! monitor: no"),
            (Command::NextTrack, "! music: no"),
            (Command::Grants, "! permissions: no"),
        ];
        for (command, expected) in cases {
            assert_eq!(said(&command, &refused).as_deref(), Some(expected));
        }
    }

    #[test]
    fn the_switches_say_which_way_they_went() {
        let cases: [(Command, &str); 6] = [
            (Command::Share { on: false }, "screen off"),
            (Command::Mute { on: false }, "mic open"),
            (Command::Music { on: true }, "music on"),
            (
                Command::SceneTimerStart { id: "clock".into() },
                "scene timer started",
            ),
            (
                Command::SceneTimerStop { id: "clock".into() },
                "scene timer stopped",
            ),
            (
                Command::Arm {
                    adapter: 7,
                    on: false,
                },
                "destination 7 disarmed",
            ),
        ];
        for (command, expected) in cases {
            assert_eq!(said(&command, &Reply::Ok).as_deref(), Some(expected));
        }
    }

    #[test]
    fn the_card_says_which_one_and_taking_it_down_says_that() {
        assert_eq!(
            said(&Command::SceneSwitch { name: "BRB".into() }, &Reply::Ok).as_deref(),
            Some("scene switched to BRB")
        );
    }

    #[test]
    fn nothing_noted_is_nothing_to_show() {
        assert!(Journal::default().is_empty());
        assert!(Journal::default().lines().is_empty());
    }
}
