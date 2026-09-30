//! What changed in the engine, as a feed every face may follow: the live
//! started, the scene switched, the track changed.
//!
//! An event is the difference between two statuses ([`between`]), taken where
//! every change passes (`Engine::handle` and `Engine::tick`), so no verb has
//! to remember to say it and nothing a verb forgets goes unsaid. Only the
//! fields named here are compared, which keeps anything secret out by
//! construction. Pure; the socket that follows it is `remuxd::server`.

use serde::{Deserialize, Serialize};

use crate::protocol::Status;

/// One thing that changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    LiveStarted,
    LiveEnded,
    RecordStarted,
    RecordStopped,
    SceneSwitched {
        name: String,
    },
    Muted {
        on: bool,
    },
    /// The music bed's track, `None` when the music stopped.
    TrackChanged {
        title: Option<String>,
    },
    /// Whether the app is reachable: the chat, the destinations with an
    /// account, the viewers.
    AppReachable {
        on: bool,
    },
}

/// What changed from `before` to `after`, in the order a person reads it:
/// the air first, then the picture, then the sound, then the app.
#[must_use]
pub fn between(before: &Status, after: &Status) -> Vec<Event> {
    let mut events = Vec::new();
    if before.on_air != after.on_air {
        events.push(if after.on_air {
            Event::LiveStarted
        } else {
            Event::LiveEnded
        });
    }
    if before.recording != after.recording {
        events.push(if after.recording {
            Event::RecordStarted
        } else {
            Event::RecordStopped
        });
    }
    if before.active_scene != after.active_scene {
        events.push(Event::SceneSwitched {
            name: after.active_scene.clone(),
        });
    }
    if before.muted != after.muted {
        events.push(Event::Muted { on: after.muted });
    }
    if before.music != after.music {
        events.push(Event::TrackChanged {
            title: after.music.clone(),
        });
    }
    if before.app != after.app {
        events.push(Event::AppReachable { on: after.app });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status::default()
    }

    #[test]
    fn nothing_changed_says_nothing() {
        assert_eq!(between(&status(), &status()), vec![]);
    }

    #[test]
    fn the_live_starting_and_ending_are_events() {
        let off = status();
        let on = Status {
            on_air: true,
            ..status()
        };
        assert_eq!(between(&off, &on), vec![Event::LiveStarted]);
        assert_eq!(between(&on, &off), vec![Event::LiveEnded]);
    }

    #[test]
    fn the_recording_starting_and_stopping_are_events() {
        let off = status();
        let on = Status {
            recording: true,
            ..status()
        };
        assert_eq!(between(&off, &on), vec![Event::RecordStarted]);
        assert_eq!(between(&on, &off), vec![Event::RecordStopped]);
    }

    #[test]
    fn a_scene_switch_names_the_scene_switched_to() {
        let after = Status {
            active_scene: "break".into(),
            ..status()
        };
        assert_eq!(
            between(&status(), &after),
            vec![Event::SceneSwitched {
                name: "break".into()
            }]
        );
    }

    #[test]
    fn muting_and_opening_the_microphone_are_events() {
        let muted = Status {
            muted: true,
            ..status()
        };
        assert_eq!(between(&status(), &muted), vec![Event::Muted { on: true }]);
        assert_eq!(between(&muted, &status()), vec![Event::Muted { on: false }]);
    }

    #[test]
    fn a_track_changing_or_the_music_stopping_is_an_event() {
        let playing = |title: &str| Status {
            music: Some(title.into()),
            ..status()
        };
        assert_eq!(
            between(&playing("one"), &playing("two")),
            vec![Event::TrackChanged {
                title: Some("two".into())
            }]
        );
        assert_eq!(
            between(&playing("two"), &status()),
            vec![Event::TrackChanged { title: None }]
        );
    }

    #[test]
    fn the_app_coming_and_going_is_an_event() {
        let up = Status {
            app: true,
            ..status()
        };
        assert_eq!(
            between(&status(), &up),
            vec![Event::AppReachable { on: true }]
        );
        assert_eq!(
            between(&up, &status()),
            vec![Event::AppReachable { on: false }]
        );
    }

    #[test]
    fn a_field_nobody_follows_changing_says_nothing() {
        // The meters and the viewers move all the time; a feed of them would
        // be the only thing anybody saw in it.
        let after = Status {
            viewers: Some(12),
            mirrored: true,
            ..status()
        };
        assert_eq!(between(&status(), &after), vec![]);
    }

    #[test]
    fn several_changes_at_once_come_out_air_first() {
        let after = Status {
            on_air: true,
            recording: true,
            active_scene: "code".into(),
            ..status()
        };
        assert_eq!(
            between(&status(), &after),
            vec![
                Event::LiveStarted,
                Event::RecordStarted,
                Event::SceneSwitched {
                    name: "code".into()
                },
            ]
        );
    }
}
