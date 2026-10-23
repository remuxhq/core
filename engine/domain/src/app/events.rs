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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
    /// One destination's stream is out, on its own door.
    DestinationLive {
        id: i64,
    },
    /// One destination's stream ended: `why` is what its ffmpeg said when it
    /// fell over, `None` when somebody stopped it. The live may go on
    /// elsewhere; `live-ended` is when the last one ends.
    DestinationEnded {
        id: i64,
        why: Option<String>,
    },
    /// Armed for the next live, or left out of it.
    DestinationArmed {
        id: i64,
        on: bool,
    },
    /// A rehearsal with no audience, or the real thing again.
    DestinationSandbox {
        id: i64,
        on: bool,
    },
    DestinationRetitled {
        id: i64,
        title: Option<String>,
        description: Option<String>,
    },
    DestinationCategorized {
        id: i64,
        category: Option<String>,
    },
    /// A command the engine said no to: `verb` as the wire names it
    /// (`go-live`), `message` as the reply said it.
    Refused {
        verb: String,
        message: String,
    },
    /// A scene saved under a new name, made or duplicated.
    SceneCreated {
        name: String,
    },
    SceneDeleted {
        name: String,
    },
    /// A layer put in the active scene: `kind` is `camera`, `window`,
    /// `screen`, `image`, `text` or `timer`.
    LayerAdded {
        id: String,
        kind: String,
    },
    LayerRemoved {
        id: String,
    },
    /// Shown, or hidden with its capture kept open.
    LayerVisible {
        id: String,
        on: bool,
    },
    /// A WGSL filter put on a layer, or on the whole scene when `layer` is
    /// `None`; `file` is `None` when it was taken off.
    FilterSet {
        layer: Option<String>,
        file: Option<String>,
    },
    /// A timer in the active scene reached 00:00. The engine never switches
    /// scenes for it; this is where a face that wants to, does.
    TimerFinished {
        id: String,
    },
    /// A camera that stopped handing over frames, shown, for three seconds.
    LayerStalled {
        id: String,
    },
    /// A camera that stalled handing over frames again.
    LayerFlowing {
        id: String,
    },
    /// A sound put in the active scene: `source` as a person reads it,
    /// `app Discord`, `mic USB`, `system`.
    AudioLayerAdded {
        id: String,
        source: String,
    },
    AudioLayerRemoved {
        id: String,
    },
    AudioLayerMuted {
        id: String,
        on: bool,
    },
    /// Its fader, 0 to 2.
    AudioLayerVolume {
        id: String,
        volume: f64,
    },
    /// Whether it steps back under the voice now.
    AudioLayerDucked {
        id: String,
        on: bool,
    },
    /// A capture of sound saying what is wrong with it, a format it does not
    /// read, or `None` once it is over it.
    SoundComplaint {
        source: Heard,
        complaint: Option<String>,
    },
    /// The voice had a hole (`starved`: the microphone late) or a crackle
    /// (`dropped`: samples thrown away), as totals since the microphone
    /// opened. On the detail ring: a bad device says it often.
    AudioGlitch {
        starved: u64,
        dropped: u64,
    },
    Faders {
        mic: f64,
        music: f64,
        duck_db: f64,
    },
    Gate {
        params: crate::sound::mixer::gate::GateParams,
    },
    /// Hearing your own mix on the speakers.
    Monitoring {
        on: bool,
    },
    /// The music in the mix that leaves.
    MusicToStream {
        on: bool,
    },
    Denoise {
        on: bool,
    },
    /// The self-view flipped.
    Mirrored {
        on: bool,
    },
    /// How many are watching, when the platforms said.
    Viewers {
        total: Option<u32>,
    },
    /// What a server said out loud: a platform that refused a title.
    Notice {
        text: String,
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
    /// Something happened in a platform's chat beyond a line (`docs/wire.md`):
    /// a sub, a gift, a tip, a raid, a moderator's hand, or the platform's own
    /// kind. Its `type` and fields are the wire's. Text from strangers, like a
    /// line's.
    ChatEvent {
        #[serde(flatten)]
        happened: crate::app::wire::Happening,
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

    /// Which ring keeps it.
    fn ring(&self) -> Ring {
        match self {
            Self::Chat { .. } | Self::ChatHidden { .. } | Self::ChatEvent { .. } => Ring::Chat,
            Self::AudioGlitch { .. }
            | Self::Faders { .. }
            | Self::Gate { .. }
            | Self::Monitoring { .. }
            | Self::MusicToStream { .. }
            | Self::Denoise { .. }
            | Self::AudioLayerVolume { .. }
            | Self::AudioLayerDucked { .. }
            | Self::Mirrored { .. }
            | Self::Viewers { .. } => Ring::Detail,
            _ => Ring::State,
        }
    }
}

/// A capture of sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Heard {
    Mic,
}

/// The three rings, so that what is said often never pushes out what is
/// said once: a busy chat, a fader dragged, the viewers counted.
enum Ring {
    State,
    Chat,
    Detail,
}

/// An event as the feed keeps it: numbered from one in the order it
/// happened, so a face that asks for what came after the last one it saw
/// misses nothing, and stamped in seconds past the epoch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Since {
    pub gap: Option<Gap>,
    pub events: Vec<Numbered>,
}

/// The last of the events, for every face, oldest first.
///
/// Three rings on one count: the engine's own, the chat's and the detail's
/// (faders, switches, viewers, glitches). A busy room says a thousand things
/// in an hour, and in one ring they pushed the live ending out before a face
/// that was a little behind came back for it.
#[derive(Debug, Clone, Default)]
pub struct Events {
    kept: VecDeque<Numbered>,
    chat: VecDeque<Numbered>,
    detail: VecDeque<Numbered>,
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
    /// How many of the detail's: a fader dragged says dozens a second.
    pub const DETAIL_KEPT: usize = 200;

    /// Numbers start at `from` and only rise: an engine restarted under a
    /// face that remembers the last number it saw hands over bigger ones,
    /// and the face is told it missed the restart. See `app::chat::Feed`.
    #[must_use]
    pub fn starting_at(from: u64) -> Self {
        let last = from.max(1) - 1;
        Self {
            kept: VecDeque::new(),
            chat: VecDeque::new(),
            detail: VecDeque::new(),
            last,
            lost: last,
        }
    }

    /// Keep an event, at this many seconds past the epoch; its number.
    pub fn push(&mut self, at: i64, event: Event) -> u64 {
        let (ring, most) = match event.ring() {
            Ring::State => (&mut self.kept, Self::KEPT),
            Ring::Chat => (&mut self.chat, Self::CHAT_KEPT),
            Ring::Detail => (&mut self.detail, Self::DETAIL_KEPT),
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
            .chain(&self.detail)
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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub on_air: bool,
    pub recording: bool,
    pub active_scene: String,
    pub muted: bool,
    pub music: Option<String>,
    pub app: bool,
    /// The destinations whose stream is out right now.
    pub sending: std::collections::BTreeSet<i64>,
    /// What each door's ffmpeg last complained about.
    pub troubles: std::collections::BTreeMap<i64, String>,
    /// The scenes' names.
    pub scenes: std::collections::BTreeSet<String>,
    /// The active scene's layers, captures and generated alike.
    pub layers: std::collections::BTreeMap<String, LayerSeen>,
    /// The active scene's sounds.
    pub audio_layers: std::collections::BTreeMap<String, AudioLayerSeen>,
    /// The active scene's filter.
    pub filter: Option<String>,
    /// The timers of the active scene at 00:00.
    pub timers_done: std::collections::BTreeSet<String>,
    pub mic_complaint: Option<String>,
    /// The voice's holes and crackles, counted since the microphone opened.
    pub starved: u64,
    pub dropped: u64,
    pub faders: crate::protocol::Faders,
    pub gate: crate::sound::mixer::gate::GateParams,
    pub monitoring: bool,
    pub music_to_stream: bool,
    pub denoise: bool,
    pub mirrored: bool,
    pub viewers: Option<u32>,
}

/// A sound, as far as the events follow it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioLayerSeen {
    pub source: String,
    pub muted: bool,
    pub volume: f64,
    pub ducks: bool,
}

impl AudioLayerSeen {
    #[must_use]
    pub fn of(layer: &crate::sound::audio_layers::Layer) -> (String, Self) {
        (
            layer.id.clone(),
            Self {
                source: layer.source.said(),
                muted: layer.muted,
                volume: layer.volume,
                ducks: layer.ducks(),
            },
        )
    }
}

/// A layer, as far as the events follow it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LayerSeen {
    pub kind: String,
    pub visible: bool,
    pub filter: Option<String>,
}

impl LayerSeen {
    /// A capture: camera, window, screen or image.
    #[must_use]
    pub fn of_layer(layer: &crate::picture::layers::Layer) -> (String, Self) {
        let kind = serde_json::to_value(layer.source.kind)
            .ok()
            .and_then(|kind| kind.as_str().map(str::to_string))
            .unwrap_or_default();
        (
            layer.id.clone(),
            Self {
                kind,
                visible: layer.visible,
                filter: layer.shader.clone(),
            },
        )
    }

    /// A generated layer: text or timer.
    #[must_use]
    pub fn of_element(element: &crate::picture::scenes::Element) -> (String, Self) {
        let kind = match element.content {
            crate::picture::scenes::ElementContent::Text { .. } => "text",
            crate::picture::scenes::ElementContent::Timer { .. } => "timer",
        };
        (
            element.id.clone(),
            Self {
                kind: kind.into(),
                visible: element.visible,
                filter: element.shader.clone(),
            },
        )
    }
}

impl From<&Status> for Snapshot {
    /// What a face sees, as the events follow it. The voice's glitches and the
    /// timers at zero are the engine's to count; a status starts them at none.
    fn from(status: &Status) -> Self {
        let viewers: Vec<u32> = status
            .destinations
            .iter()
            .filter_map(|row| row.viewers)
            .collect();
        Self {
            on_air: status.on_air,
            recording: status.recording,
            active_scene: status.scene.name.clone(),
            muted: status.muted,
            music: status.music.clone(),
            app: status.app_reachable,
            sending: status
                .destinations
                .iter()
                .filter(|row| row.status == "live")
                .map(|row| row.id)
                .collect(),
            troubles: status
                .destinations
                .iter()
                .filter_map(|row| Some((row.id, row.trouble.clone()?)))
                .collect(),
            scenes: status.scenes.iter().cloned().collect(),
            layers: status
                .scene
                .layers
                .iter()
                .map(LayerSeen::of_layer)
                .chain(status.scene.elements.iter().map(LayerSeen::of_element))
                .collect(),
            filter: status.scene.shader.clone(),
            audio_layers: status
                .scene
                .audio_layers
                .iter()
                .map(AudioLayerSeen::of)
                .collect(),
            timers_done: std::collections::BTreeSet::new(),
            mic_complaint: status.mic_complaint.clone(),
            starved: 0,
            dropped: 0,
            faders: status.faders,
            gate: status.gate,
            monitoring: status.monitoring,
            music_to_stream: status.music_to_stream,
            denoise: status.denoise,
            mirrored: status.mirrored,
            viewers: (!viewers.is_empty()).then(|| viewers.iter().sum()),
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
    for &id in after.sending.difference(&before.sending) {
        events.push(Event::DestinationLive { id });
    }
    for &id in before.sending.difference(&after.sending) {
        events.push(Event::DestinationEnded {
            id,
            why: after.troubles.get(&id).cloned(),
        });
    }
    let switched = before.active_scene != after.active_scene;
    if switched {
        events.push(Event::SceneSwitched {
            name: after.active_scene.clone(),
        });
    }
    for name in after.scenes.difference(&before.scenes) {
        events.push(Event::SceneCreated { name: name.clone() });
    }
    for name in before.scenes.difference(&after.scenes) {
        events.push(Event::SceneDeleted { name: name.clone() });
    }
    // Within one scene only: a switch moves every layer, filter and sound,
    // and the switch is the one thing that happened.
    if !switched {
        picture_between(before, after, &mut events);
        audio_layers_between(before, after, &mut events);
    }
    for id in after.timers_done.difference(&before.timers_done) {
        events.push(Event::TimerFinished { id: id.clone() });
    }
    if before.muted != after.muted {
        events.push(Event::Muted { on: after.muted });
    }
    if before.music != after.music {
        events.push(Event::TrackChanged {
            title: after.music.clone(),
        });
    }
    sound_between(before, after, &mut events);
    if before.app != after.app {
        events.push(Event::AppReachable { on: after.app });
    }
    if before.viewers != after.viewers {
        events.push(Event::Viewers {
            total: after.viewers,
        });
    }
    events
}

/// The captures' complaints, the voice's glitches, the switches and faders.
fn sound_between(before: &Snapshot, after: &Snapshot, events: &mut Vec<Event>) {
    for (source, was, now) in [(Heard::Mic, &before.mic_complaint, &after.mic_complaint)] {
        if was != now {
            events.push(Event::SoundComplaint {
                source,
                complaint: now.clone(),
            });
        }
    }
    // Rising only: a microphone opened again counts from zero.
    if after.starved > before.starved || after.dropped > before.dropped {
        events.push(Event::AudioGlitch {
            starved: after.starved,
            dropped: after.dropped,
        });
    }
    if before.faders != after.faders {
        events.push(Event::Faders {
            mic: after.faders.mic,
            music: after.faders.music,
            duck_db: after.faders.duck_db,
        });
    }
    if before.gate != after.gate {
        events.push(Event::Gate { params: after.gate });
    }
    if before.monitoring != after.monitoring {
        events.push(Event::Monitoring {
            on: after.monitoring,
        });
    }
    if before.music_to_stream != after.music_to_stream {
        events.push(Event::MusicToStream {
            on: after.music_to_stream,
        });
    }
    if before.denoise != after.denoise {
        events.push(Event::Denoise { on: after.denoise });
    }
    if before.mirrored != after.mirrored {
        events.push(Event::Mirrored { on: after.mirrored });
    }
}

/// The active scene's sounds, from `before` to `after`.
fn audio_layers_between(before: &Snapshot, after: &Snapshot, events: &mut Vec<Event>) {
    for id in before.audio_layers.keys() {
        if !after.audio_layers.contains_key(id) {
            events.push(Event::AudioLayerRemoved { id: id.clone() });
        }
    }
    for (id, now) in &after.audio_layers {
        let Some(was) = before.audio_layers.get(id) else {
            events.push(Event::AudioLayerAdded {
                id: id.clone(),
                source: now.source.clone(),
            });
            continue;
        };
        if was.source != now.source {
            events.push(Event::AudioLayerRemoved { id: id.clone() });
            events.push(Event::AudioLayerAdded {
                id: id.clone(),
                source: now.source.clone(),
            });
            continue;
        }
        if was.muted != now.muted {
            events.push(Event::AudioLayerMuted {
                id: id.clone(),
                on: now.muted,
            });
        }
        if was.volume != now.volume {
            events.push(Event::AudioLayerVolume {
                id: id.clone(),
                volume: now.volume,
            });
        }
        if was.ducks != now.ducks {
            events.push(Event::AudioLayerDucked {
                id: id.clone(),
                on: now.ducks,
            });
        }
    }
}

/// The active scene's layers and filters, from `before` to `after`.
fn picture_between(before: &Snapshot, after: &Snapshot, events: &mut Vec<Event>) {
    for id in before.layers.keys() {
        if !after.layers.contains_key(id) {
            events.push(Event::LayerRemoved { id: id.clone() });
        }
    }
    for (id, now) in &after.layers {
        if !before.layers.contains_key(id) {
            events.push(Event::LayerAdded {
                id: id.clone(),
                kind: now.kind.clone(),
            });
        }
    }
    for (id, now) in &after.layers {
        let Some(was) = before.layers.get(id) else {
            continue;
        };
        if was.visible != now.visible {
            events.push(Event::LayerVisible {
                id: id.clone(),
                on: now.visible,
            });
        }
        if was.filter != now.filter {
            events.push(Event::FilterSet {
                layer: Some(id.clone()),
                file: now.filter.clone(),
            });
        }
    }
    if before.filter != after.filter {
        events.push(Event::FilterSet {
            layer: None,
            file: after.filter.clone(),
        });
    }
}

/// What changed in the rows both lists hold: armed, rehearsed, retitled,
/// recategorized. A row only one of them has says nothing: the first list a
/// wire hands over is every row at once, and armed rows in it were armed
/// before anybody was following.
#[must_use]
pub fn rows_between(
    before: &[crate::protocol::Destination],
    after: &[crate::protocol::Destination],
) -> Vec<Event> {
    let mut events = Vec::new();
    for now in after {
        let Some(was) = before.iter().find(|row| row.id == now.id) else {
            continue;
        };
        let id = now.id;
        if was.armed != now.armed {
            events.push(Event::DestinationArmed { id, on: now.armed });
        }
        if was.sandbox != now.sandbox {
            events.push(Event::DestinationSandbox {
                id,
                on: now.sandbox,
            });
        }
        if (&was.title, &was.description) != (&now.title, &now.description) {
            events.push(Event::DestinationRetitled {
                id,
                title: now.title.clone(),
                description: now.description.clone(),
            });
        }
        if was.category != now.category {
            events.push(Event::DestinationCategorized {
                id,
                category: now.category.clone(),
            });
        }
    }
    events
}

/// The refusal in a reply, if the command was one somebody meant to change
/// something with: the journal's own test (`air::journal::said`), which
/// leaves out a read that failed.
#[must_use]
pub fn refused(
    command: &crate::protocol::Command,
    reply: &crate::protocol::Reply,
) -> Option<Event> {
    let crate::protocol::Reply::Error { message } = reply else {
        return None;
    };
    crate::air::journal::said(command, reply)?;
    let verb = serde_json::to_value(command)
        .ok()
        .and_then(|told| told.get("cmd")?.as_str().map(str::to_string))
        .unwrap_or_default();
    Some(Event::Refused {
        verb,
        message: message.clone(),
    })
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

    // A sound added, muted, turned down, told not to duck and taken away,
    // each said once.
    #[test]
    fn an_audio_layer_and_its_changes_are_events() {
        use crate::sound::audio_layers::{Duck, Layer, Source};
        let call = Layer::new("call".into(), Source::app("Discord".into())).unwrap();
        let mut with = status();
        with.scene.audio_layers = vec![call.clone()];
        assert_eq!(
            changed(&status(), &with),
            vec![Event::AudioLayerAdded {
                id: "call".into(),
                source: "app Discord".into()
            }]
        );
        let mut changed_one = with.clone();
        changed_one.scene.audio_layers[0].muted = true;
        changed_one.scene.audio_layers[0].volume = 0.5;
        changed_one.scene.audio_layers[0].duck = Duck::Off;
        assert_eq!(
            changed(&with, &changed_one),
            vec![
                Event::AudioLayerMuted {
                    id: "call".into(),
                    on: true
                },
                Event::AudioLayerVolume {
                    id: "call".into(),
                    volume: 0.5
                },
                Event::AudioLayerDucked {
                    id: "call".into(),
                    on: false
                },
            ]
        );
        assert_eq!(
            changed(&with, &status()),
            vec![Event::AudioLayerRemoved { id: "call".into() }]
        );
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
        let mut after = status();
        after.scene.name = "break".into();
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
            app_reachable: true,
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
        // The frames count thirty times a second: a feed of them would be the
        // only thing anybody saw in it.
        let mut after = Status {
            version: "9.9.9".into(),
            motor: "obs 99".into(),
            ..status()
        };
        after.picture.frames = 900;
        assert_eq!(changed(&status(), &after), vec![]);
    }

    #[test]
    fn several_changes_at_once_come_out_air_first() {
        let mut after = Status {
            on_air: true,
            recording: true,
            ..status()
        };
        after.scene.name = "code".into();
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

    // What a bridge says beyond a line reaches a face as one flat event, the
    // wire's own type and fields beside its number, and reads back the same.
    #[test]
    fn a_chat_event_reads_as_one_flat_object_with_the_wires_type() {
        let happened = crate::app::wire::Happening {
            what: crate::app::wire::What::Sub {
                months: 6,
                tier: "1000".into(),
            },
            id: "u1".into(),
            platform: "twitch".into(),
            channel: "kartths".into(),
            from: "Ana".into(),
            body: "six months!".into(),
            badges: vec!["member".into()],
            reply: String::new(),
        };
        let numbered = Numbered {
            seq: 11,
            at: 1,
            event: Event::ChatEvent { happened },
        };
        let said = serde_json::to_string(&numbered).unwrap();
        assert_eq!(
            said,
            r#"{"seq":11,"at":1,"event":"chat-event","type":"sub","months":6,"tier":"1000","id":"u1","platform":"twitch","channel":"kartths","from":"Ana","body":"six months!","badges":["member"],"reply":""}"#
        );
        assert_eq!(serde_json::from_str::<Numbered>(&said).unwrap(), numbered);
        assert!(matches!(numbered.event.ring(), Ring::Chat));
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

    fn row(id: i64) -> crate::protocol::Destination {
        crate::protocol::Destination {
            id,
            name: format!("row {id}"),
            platform: "twitch".into(),
            status: "off".into(),
            armed: false,
            sandbox: false,
            connected: true,
            account: None,
            category: None,
            category_id: None,
            viewers: None,
            viewers_peak: None,
            trouble: None,
            title: None,
            description: None,
            channel: None,
        }
    }

    fn sending(rows: Vec<crate::protocol::Destination>) -> Status {
        Status {
            destinations: rows,
            ..status()
        }
    }

    #[test]
    fn each_destination_going_out_and_ending_is_an_event() {
        let live = |id| crate::protocol::Destination {
            status: "live".into(),
            ..row(id)
        };
        let both = sending(vec![live(2), live(6)]);
        assert_eq!(
            changed(&sending(vec![row(2), row(6)]), &both),
            vec![
                Event::DestinationLive { id: 2 },
                Event::DestinationLive { id: 6 }
            ]
        );
        // YouTube fell over with the Twitch still going: what its ffmpeg said.
        let dropped = sending(vec![
            live(2),
            crate::protocol::Destination {
                trouble: Some("connection reset".into()),
                ..row(6)
            },
        ]);
        assert_eq!(
            changed(&both, &dropped),
            vec![Event::DestinationEnded {
                id: 6,
                why: Some("connection reset".into())
            }]
        );
        // A stop is an end with nothing wrong.
        assert_eq!(
            changed(&dropped, &sending(vec![row(2), row(6)])),
            vec![Event::DestinationEnded { id: 2, why: None }]
        );
    }

    #[test]
    fn a_row_armed_rehearsed_retitled_or_recategorized_is_an_event() {
        let before = vec![row(2), row(6)];
        let after = vec![
            crate::protocol::Destination {
                armed: true,
                sandbox: true,
                ..row(2)
            },
            crate::protocol::Destination {
                title: Some("Rust at midnight".into()),
                category: Some("Software and Game Development".into()),
                ..row(6)
            },
        ];
        assert_eq!(
            rows_between(&before, &after),
            vec![
                Event::DestinationArmed { id: 2, on: true },
                Event::DestinationSandbox { id: 2, on: true },
                Event::DestinationRetitled {
                    id: 6,
                    title: Some("Rust at midnight".into()),
                    description: None
                },
                Event::DestinationCategorized {
                    id: 6,
                    category: Some("Software and Game Development".into())
                },
            ]
        );
    }

    #[test]
    fn a_row_that_arrived_or_left_says_nothing_of_what_it_holds() {
        // The first list a wire hands over is every row at once: armed rows
        // in it were armed before anybody was following.
        let armed = crate::protocol::Destination {
            armed: true,
            ..row(2)
        };
        assert_eq!(rows_between(&[], std::slice::from_ref(&armed)), vec![]);
        assert_eq!(rows_between(&[armed], &[]), vec![]);
    }

    #[test]
    fn a_refusal_names_the_verb_and_says_why() {
        use crate::protocol::{Command, Reply};
        let no = Reply::Error {
            message: "the scene is empty".into(),
        };
        assert_eq!(
            refused(&Command::GoLive, &no),
            Some(Event::Refused {
                verb: "go-live".into(),
                message: "the scene is empty".into()
            })
        );
        assert_eq!(refused(&Command::GoLive, &Reply::Ok), None);
        // A read that failed refused nobody anything.
        assert_eq!(refused(&Command::Levels, &no), None);
    }

    fn seen(kind: &str, visible: bool, filter: Option<&str>) -> LayerSeen {
        LayerSeen {
            kind: kind.into(),
            visible,
            filter: filter.map(str::to_string),
        }
    }

    fn with_layers(scene: &str, layers: &[(&str, LayerSeen)]) -> Snapshot {
        Snapshot {
            active_scene: scene.into(),
            layers: layers
                .iter()
                .map(|(id, seen)| ((*id).to_string(), seen.clone()))
                .collect(),
            ..Snapshot::default()
        }
    }

    #[test]
    fn a_layer_added_removed_shown_or_filtered_in_one_scene_is_an_event() {
        let before = with_layers(
            "code",
            &[
                ("face", seen("camera", true, None)),
                ("logo", seen("image", true, None)),
            ],
        );
        let after = with_layers(
            "code",
            &[
                ("face", seen("camera", false, Some("/tmp/warm.wgsl"))),
                ("title", seen("text", true, None)),
            ],
        );
        assert_eq!(
            between(&before, &after),
            vec![
                Event::LayerRemoved { id: "logo".into() },
                Event::LayerAdded {
                    id: "title".into(),
                    kind: "text".into()
                },
                Event::LayerVisible {
                    id: "face".into(),
                    on: false
                },
                Event::FilterSet {
                    layer: Some("face".into()),
                    file: Some("/tmp/warm.wgsl".into())
                },
            ]
        );
    }

    #[test]
    fn a_scene_switch_is_the_switch_not_every_layer_it_moved() {
        let before = with_layers("code", &[("face", seen("camera", true, None))]);
        let after = Snapshot {
            filter: Some("/tmp/crt.wgsl".into()),
            ..with_layers("break", &[("card", seen("image", true, None))])
        };
        assert_eq!(
            between(&before, &after),
            vec![Event::SceneSwitched {
                name: "break".into()
            }]
        );
    }

    #[test]
    fn a_scene_made_or_deleted_and_the_scene_filter_are_events() {
        let scenes = |names: &[&str]| Snapshot {
            active_scene: "code".into(),
            scenes: names.iter().map(|n| (*n).to_string()).collect(),
            ..Snapshot::default()
        };
        assert_eq!(
            between(&scenes(&["code"]), &scenes(&["code", "break"])),
            vec![Event::SceneCreated {
                name: "break".into()
            }]
        );
        assert_eq!(
            between(&scenes(&["code", "break"]), &scenes(&["code"])),
            vec![Event::SceneDeleted {
                name: "break".into()
            }]
        );
        let filtered = Snapshot {
            filter: Some("/tmp/crt.wgsl".into()),
            ..scenes(&["code"])
        };
        assert_eq!(
            between(&scenes(&["code"]), &filtered),
            vec![Event::FilterSet {
                layer: None,
                file: Some("/tmp/crt.wgsl".into())
            }]
        );
    }

    #[test]
    fn a_timer_reaching_zero_is_an_event_once() {
        let done = |ids: &[&str]| Snapshot {
            timers_done: ids.iter().map(|n| (*n).to_string()).collect(),
            ..Snapshot::default()
        };
        assert_eq!(
            between(&done(&[]), &done(&["clock"])),
            vec![Event::TimerFinished { id: "clock".into() }]
        );
        assert_eq!(between(&done(&["clock"]), &done(&["clock"])), vec![]);
        // Restarted or stopped: nothing to say until it reaches zero again.
        assert_eq!(between(&done(&["clock"]), &done(&[])), vec![]);
    }

    #[test]
    fn a_capture_complaining_and_getting_over_it_is_an_event() {
        let complaining = Status {
            mic_complaint: Some("the microphone speaks 8-bit".into()),
            ..status()
        };
        assert_eq!(
            changed(&status(), &complaining),
            vec![Event::SoundComplaint {
                source: Heard::Mic,
                complaint: Some("the microphone speaks 8-bit".into())
            }]
        );
        assert_eq!(
            changed(&complaining, &status()),
            vec![Event::SoundComplaint {
                source: Heard::Mic,
                complaint: None
            }]
        );
    }

    #[test]
    fn a_hole_or_a_crackle_in_the_voice_is_an_event_with_the_totals() {
        let heard = |starved, dropped| Snapshot {
            starved,
            dropped,
            ..Snapshot::default()
        };
        assert_eq!(
            between(&heard(3, 0), &heard(5, 0)),
            vec![Event::AudioGlitch {
                starved: 5,
                dropped: 0
            }]
        );
        assert_eq!(between(&heard(5, 0), &heard(5, 0)), vec![]);
        // A new microphone counts from zero: nothing went wrong.
        assert_eq!(between(&heard(5, 2), &heard(0, 0)), vec![]);
    }

    #[test]
    fn the_sound_switches_faders_and_viewers_are_events() {
        let before = status();
        let mut after = status();
        after.faders.mic = 0.8;
        after.gate.full = 0.2;
        after.monitoring = true;
        after.music_to_stream = !before.music_to_stream;
        after.denoise = true;
        after.mirrored = true;
        after.destinations = vec![crate::protocol::Destination {
            viewers: Some(12),
            ..Default::default()
        }];
        let said = changed(&before, &after);
        assert_eq!(
            said,
            vec![
                Event::Faders {
                    mic: 0.8,
                    music: before.faders.music,
                    duck_db: before.faders.duck_db
                },
                Event::Gate { params: after.gate },
                Event::Monitoring { on: true },
                Event::MusicToStream {
                    on: after.music_to_stream
                },
                Event::Denoise { on: true },
                Event::Mirrored { on: true },
                Event::Viewers { total: Some(12) },
            ]
        );
    }

    #[test]
    fn what_changes_all_the_time_never_pushes_the_live_out() {
        let mut events = Events::default();
        events.push(1, Event::LiveStarted);
        for n in 0..Events::DETAIL_KEPT as u32 + 50 {
            events.push(2, Event::Viewers { total: Some(n) });
        }
        let held = events.since(0).events;
        assert_eq!(held.first().map(|e| &e.event), Some(&Event::LiveStarted));
        assert_eq!(held.len(), 1 + Events::DETAIL_KEPT);
    }
}
