//! What changed in the engine, as a feed every face may follow: the live
//! started, the scene switched, the track changed, somebody said something.
//!
//! The engine's own state is the difference between two snapshots
//! ([`between`]), taken where every change passes (`Engine::handle` and
//! `Engine::tick`), so no verb has to remember to say it and nothing a verb
//! forgets goes unsaid. A [`Snapshot`] holds the few fields the feed follows,
//! which keeps anything secret out by construction. The chat is said where it
//! arrives, the wire, and where a line is taken off, a hide or a delete.
//! Pure; the socket that follows it is `remuxd::server`.

use std::collections::VecDeque;

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
    /// Somebody said something. `line` is the chat's own number, what
    /// `remux chat hide` and `remux chat delete` take. Text from strangers:
    /// a face strips it before a terminal and never runs it.
    Chat {
        line: u64,
        platform: String,
        channel: String,
        from: String,
        body: String,
        id: String,
    },
    /// A line of chat taken off every face, hidden here or deleted on its
    /// platform: a face that shows the chat takes it down too.
    ChatHidden {
        line: u64,
    },
}

impl Event {
    /// A line of chat as the feed kept it, numbered.
    #[must_use]
    pub fn said(line: &crate::protocol::ChatLine) -> Self {
        Self::Chat {
            line: line.seq,
            platform: line.platform.clone(),
            channel: line.channel.clone(),
            from: line.from.clone(),
            body: line.body.clone(),
            id: line.id.clone(),
        }
    }

    /// Whether it belongs to the chat, which has a ring of its own.
    fn is_chat(&self) -> bool {
        matches!(self, Self::Chat { .. } | Self::ChatHidden { .. })
    }
}

/// An event as the feed keeps it: numbered from one in the order it
/// happened, so a face that asks for what came after the last one it saw
/// misses nothing, and stamped in seconds past the epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Numbered {
    pub seq: u64,
    pub at: i64,
    #[serde(flatten)]
    pub event: Event,
}

/// Events a face asked for and can no longer have: they fell out of a ring
/// before it came back for them. Some of `from` to `to`, both included, are
/// gone (the other ring may still hold the rest, and hands them over), and a
/// status is what makes the face whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Gap {
    pub from: u64,
    pub to: u64,
}

/// What came after a number: the events still held, and the gap before
/// them when some were not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Since {
    pub gap: Option<Gap>,
    pub events: Vec<Numbered>,
}

/// The last of the events, for every face, oldest first.
///
/// Two rings on one count: the engine's own and the chat's. A busy room says
/// a thousand things in an hour, and in one ring they pushed the live ending
/// out before a face that was a little behind came back for it.
#[derive(Debug, Clone, Default)]
pub struct Events {
    kept: VecDeque<Numbered>,
    chat: VecDeque<Numbered>,
    last: u64,
    /// The highest number no longer held, by either ring; below the first
    /// number of an engine that started after another, every one.
    lost: u64,
}

impl Events {
    /// How many of the engine's own survive, as many as the journal: a face
    /// that follows is woken on every one and never far behind, and one that
    /// asks once in a while is told what it missed.
    pub const KEPT: usize = 200;
    /// How many of the chat's, as many as the chat holds.
    pub const CHAT_KEPT: usize = crate::protocol::CHAT_LINES;

    /// Numbers start at `from` and only rise: an engine restarted under a
    /// face that remembers the last number it saw hands over bigger ones,
    /// and the face is told it missed the restart. See `app::chat::Feed`.
    #[must_use]
    pub fn starting_at(from: u64) -> Self {
        let last = from.max(1) - 1;
        Self {
            kept: VecDeque::new(),
            chat: VecDeque::new(),
            last,
            lost: last,
        }
    }

    /// Keep an event, at this many seconds past the epoch; its number.
    pub fn push(&mut self, at: i64, event: Event) -> u64 {
        let (ring, most) = if event.is_chat() {
            (&mut self.chat, Self::CHAT_KEPT)
        } else {
            (&mut self.kept, Self::KEPT)
        };
        if ring.len() == most {
            if let Some(gone) = ring.pop_front() {
                self.lost = self.lost.max(gone.seq);
            }
        }
        self.last += 1;
        ring.push_back(Numbered {
            seq: self.last,
            at,
            event,
        });
        self.last
    }

    /// Everything after `seq`, zero for all that is held.
    #[must_use]
    pub fn since(&self, seq: u64) -> Since {
        if seq >= self.last {
            return Since::default();
        }
        let gap = (seq > 0 && seq < self.lost).then_some(Gap {
            from: seq + 1,
            to: self.lost,
        });
        let mut events: Vec<Numbered> = self
            .kept
            .iter()
            .chain(&self.chat)
            .filter(|kept| kept.seq > seq)
            .cloned()
            .collect();
        events.sort_by_key(|kept| kept.seq);
        Since { gap, events }
    }

    /// The number of the newest event, zero before the first.
    #[must_use]
    pub fn last(&self) -> u64 {
        self.last
    }
}

/// The part of the engine's state the feed follows, and nothing else: what
/// [`between`] compares. Its own type rather than a whole [`Status`] because
/// the engine takes one after every command that changes something and on
/// every tick, and a status is the pipeline asked a dozen things.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub on_air: bool,
    pub recording: bool,
    pub active_scene: String,
    pub muted: bool,
    pub music: Option<String>,
    pub app: bool,
}

impl From<&Status> for Snapshot {
    fn from(status: &Status) -> Self {
        Self {
            on_air: status.on_air,
            recording: status.recording,
            active_scene: status.active_scene.clone(),
            muted: status.muted,
            music: status.music.clone(),
            app: status.app,
        }
    }
}

/// What changed from `before` to `after`, in the order a person reads it:
/// the air first, then the picture, then the sound, then the app.
#[must_use]
pub fn between(before: &Snapshot, after: &Snapshot) -> Vec<Event> {
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

    fn changed(before: &Status, after: &Status) -> Vec<Event> {
        between(&before.into(), &after.into())
    }

    #[test]
    fn nothing_changed_says_nothing() {
        assert_eq!(changed(&status(), &status()), vec![]);
    }

    #[test]
    fn the_live_starting_and_ending_are_events() {
        let off = status();
        let on = Status {
            on_air: true,
            ..status()
        };
        assert_eq!(changed(&off, &on), vec![Event::LiveStarted]);
        assert_eq!(changed(&on, &off), vec![Event::LiveEnded]);
    }

    #[test]
    fn the_recording_starting_and_stopping_are_events() {
        let off = status();
        let on = Status {
            recording: true,
            ..status()
        };
        assert_eq!(changed(&off, &on), vec![Event::RecordStarted]);
        assert_eq!(changed(&on, &off), vec![Event::RecordStopped]);
    }

    #[test]
    fn a_scene_switch_names_the_scene_switched_to() {
        let after = Status {
            active_scene: "break".into(),
            ..status()
        };
        assert_eq!(
            changed(&status(), &after),
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
        assert_eq!(changed(&status(), &muted), vec![Event::Muted { on: true }]);
        assert_eq!(changed(&muted, &status()), vec![Event::Muted { on: false }]);
    }

    #[test]
    fn a_track_changing_or_the_music_stopping_is_an_event() {
        let playing = |title: &str| Status {
            music: Some(title.into()),
            ..status()
        };
        assert_eq!(
            changed(&playing("one"), &playing("two")),
            vec![Event::TrackChanged {
                title: Some("two".into())
            }]
        );
        assert_eq!(
            changed(&playing("two"), &status()),
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
            changed(&status(), &up),
            vec![Event::AppReachable { on: true }]
        );
        assert_eq!(
            changed(&up, &status()),
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
        assert_eq!(changed(&status(), &after), vec![]);
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
            changed(&status(), &after),
            vec![
                Event::LiveStarted,
                Event::RecordStarted,
                Event::SceneSwitched {
                    name: "code".into()
                },
            ]
        );
    }

    fn switched(name: &str) -> Event {
        Event::SceneSwitched { name: name.into() }
    }

    #[test]
    fn events_are_numbered_from_one_in_the_order_they_happened() {
        let mut events = Events::default();
        assert_eq!(events.last(), 0);
        assert_eq!(events.push(10, Event::LiveStarted), 1);
        assert_eq!(events.push(11, switched("code")), 2);
        assert_eq!(
            events.since(0),
            Since {
                gap: None,
                events: vec![
                    Numbered {
                        seq: 1,
                        at: 10,
                        event: Event::LiveStarted
                    },
                    Numbered {
                        seq: 2,
                        at: 11,
                        event: switched("code")
                    },
                ],
            }
        );
        assert_eq!(events.last(), 2);
    }

    #[test]
    fn a_face_gets_only_what_came_after_the_last_it_saw() {
        let mut events = Events::default();
        events.push(10, Event::LiveStarted);
        events.push(11, switched("code"));
        let after = events.since(1);
        assert_eq!(after.gap, None);
        assert_eq!(after.events.iter().map(|e| e.seq).collect::<Vec<_>>(), [2]);
        assert_eq!(events.since(2), Since::default());
    }

    #[test]
    fn a_face_that_fell_behind_the_ring_is_told_what_it_missed() {
        let mut events = Events::default();
        for at in 0..Events::KEPT as i64 + 5 {
            events.push(at, Event::LiveStarted);
        }
        // Held: 6 to 205. A face that saw 2 missed 3, 4 and 5.
        let after = events.since(2);
        assert_eq!(after.gap, Some(Gap { from: 3, to: 5 }));
        assert_eq!(after.events.first().map(|e| e.seq), Some(6));
        assert_eq!(after.events.len(), Events::KEPT);
        // Asking for everything held is not a gap: nothing was promised.
        assert_eq!(events.since(0).gap, None);
        // The one just before the oldest held missed nothing.
        assert_eq!(events.since(5).gap, None);
    }

    #[test]
    fn a_number_from_the_future_is_nothing_yet() {
        // A face that outlived an engine restart asks for a number this one
        // never gave out; it gets nothing rather than everything twice.
        let mut events = Events::default();
        events.push(10, Event::LiveStarted);
        assert_eq!(events.since(40), Since::default());
    }

    #[test]
    fn an_event_reads_as_one_flat_object() {
        let line = serde_json::to_string(&Numbered {
            seq: 3,
            at: 1_700_000_000,
            event: switched("code"),
        })
        .unwrap();
        assert_eq!(
            line,
            r#"{"seq":3,"at":1700000000,"event":"scene-switched","name":"code"}"#
        );
        let live = serde_json::to_string(&Numbered {
            seq: 1,
            at: 1,
            event: Event::LiveStarted,
        })
        .unwrap();
        assert_eq!(live, r#"{"seq":1,"at":1,"event":"live-started"}"#);
    }

    #[test]
    fn a_face_that_outlived_the_engine_is_told_it_missed_the_restart() {
        // The daemon starts the numbers at the moment it started, as the
        // chat does, so the number a face kept from the engine before is
        // below every one this engine gives out.
        let mut before = Events::default();
        before.push(10, Event::LiveStarted);
        let kept = before.last();

        let mut after = Events::starting_at(1_000);
        assert_eq!(after.push(20, Event::LiveEnded), 1_000);
        let told = after.since(kept);
        assert_eq!(told.gap, Some(Gap { from: 2, to: 999 }));
        assert_eq!(told.events.len(), 1);
    }

    fn chat(n: u64) -> Event {
        Event::Chat {
            line: n,
            platform: "twitch".into(),
            channel: "kartths".into(),
            from: "ana".into(),
            body: format!("line {n}"),
            id: format!("m{n}"),
        }
    }

    #[test]
    fn a_chat_line_reads_as_one_flat_object_with_its_own_number() {
        let line = serde_json::to_string(&Numbered {
            seq: 9,
            at: 1,
            event: Event::Chat {
                line: 7,
                platform: "twitch".into(),
                channel: "kartths".into(),
                from: "ana".into(),
                body: "oi".into(),
                id: "m1".into(),
            },
        })
        .unwrap();
        assert_eq!(
            line,
            r#"{"seq":9,"at":1,"event":"chat","line":7,"platform":"twitch","channel":"kartths","from":"ana","body":"oi","id":"m1"}"#
        );
        let hidden = serde_json::to_string(&Numbered {
            seq: 10,
            at: 1,
            event: Event::ChatHidden { line: 7 },
        })
        .unwrap();
        assert_eq!(
            hidden,
            r#"{"seq":10,"at":1,"event":"chat-hidden","line":7}"#
        );
    }

    #[test]
    fn a_line_of_chat_is_its_own_line_whatever_a_stranger_typed() {
        let said = crate::protocol::ChatLine {
            seq: 7,
            from: "ana".into(),
            body: "oi".into(),
            platform: "twitch".into(),
            id: "m1".into(),
            channel: "kartths".into(),
        };
        assert_eq!(
            Event::said(&said),
            Event::Chat {
                line: 7,
                platform: "twitch".into(),
                channel: "kartths".into(),
                from: "ana".into(),
                body: "oi".into(),
                id: "m1".into(),
            }
        );
    }

    #[test]
    fn a_busy_chat_never_pushes_the_live_out() {
        let mut events = Events::default();
        events.push(1, Event::LiveStarted);
        for n in 0..Events::CHAT_KEPT as u64 + 50 {
            events.push(2, chat(n));
        }
        let held = events.since(0).events;
        assert_eq!(held.first().map(|e| &e.event), Some(&Event::LiveStarted));
        assert_eq!(held.len(), 1 + Events::CHAT_KEPT);
        assert!(
            held.windows(2).all(|pair| pair[0].seq < pair[1].seq),
            "one order, by number, whichever ring an event sits in"
        );
    }

    #[test]
    fn the_two_rings_come_out_as_one_order() {
        let mut events = Events::default();
        events.push(1, Event::LiveStarted);
        events.push(2, chat(1));
        events.push(3, switched("code"));
        events.push(4, Event::ChatHidden { line: 1 });
        let order: Vec<u64> = events.since(1).events.iter().map(|e| e.seq).collect();
        assert_eq!(order, [2, 3, 4]);
    }

    #[test]
    fn a_face_behind_the_chat_ring_is_told_though_the_live_is_still_held() {
        let mut events = Events::default();
        for n in 0..Events::CHAT_KEPT as u64 + 5 {
            events.push(1, chat(n));
        }
        events.push(2, Event::LiveEnded);
        // Chat 1 to 5 fell out; a face that saw 2 missed 3 to 5.
        let after = events.since(2);
        assert_eq!(after.gap, Some(Gap { from: 3, to: 5 }));
        assert_eq!(after.events.first().map(|e| e.seq), Some(6));
        assert_eq!(events.since(5).gap, None);
    }
}
