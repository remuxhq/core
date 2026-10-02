//! The sound: the microphone, the music, the mixer and the speakers.

use super::*;

#[derive(Debug, Clone, Copy)]
pub struct SoundLevels {
    pub mic: f64,
    pub music: f64,
    pub duck_db: f64,
    pub muted: bool,
    pub music_to_stream: bool,
}

/// The sound's half of the media path: the microphone, the music, the mixer
/// and the speakers. See [`crate::engine::Pipeline`] for why it is a port.
pub trait Sound: Send {
    fn audio_layer_add(
        &mut self,
        _layer: &crate::sound::audio_layers::Layer,
    ) -> Result<(), String> {
        Ok(())
    }
    fn audio_layer_remove(&mut self, _id: &str) {}
    /// The audio layer `from` is called `to` now: its capture stays open.
    fn audio_layer_rename(&mut self, _from: &str, _to: &str) {}
    fn audio_layer_levels(&mut self, _id: &str, _volume: f64, _muted: bool) {}
    /// Whether one audio layer steps back under the voice now.
    fn audio_layer_duck(&mut self, _id: &str, _ducks: bool) {}
    /// The meters of the audio layers the motor has open, in any order.
    fn audio_layers_heard(&self) -> Vec<crate::protocol::AudioLayerHeard> {
        Vec::new()
    }
    /// Open a microphone by id, or close the one that is open.
    fn mic(&mut self, device: Option<&str>) -> Result<(), String>;

    /// Play a track, or stop. The engine decides *which* track; this makes the
    /// sound. Stopping is `None` rather than a separate verb because there is
    /// only ever one music track playing and it is either that one or none.
    fn play(&mut self, track: Option<&Track>) -> Result<(), String>;
    /// Where the faders are, and whether the music is in the mix that
    /// leaves. Sent whenever one moves, rather than read by
    /// the mixer, so that a fader nobody touched costs nothing.
    fn levels(&mut self, levels: SoundLevels) -> Result<(), String>;
    /// Where the gate's thresholds are now. The whole set, not the patch: the
    /// engine is what holds them, so the machine is told the answer rather
    /// than being asked to work it out.
    fn gate(&mut self, params: GateParams);
    /// Take the room out of the microphone before the gate, or stop.
    fn denoise(&mut self, _on: bool) {}
    /// Play one file once over the mix, at its own level.
    fn clip(&mut self, _path: &std::path::Path) -> Result<(), String> {
        Err("this engine plays no clips".into())
    }
    /// Send the mix to the speakers, or stop.
    ///
    /// **The microphone is never in it.** On speakers it would come back
    /// through itself, and what a person is checking when they turn this on is
    /// the music: whether the bed is too loud and whether it steps back far
    /// enough when they talk. Their own voice they can hear already.
    fn monitor(&mut self, on: bool) -> Result<(), String>;
    /// What the speakers are, while they are open: the output device the
    /// bed is playing through, by its name, or `None` when they are closed
    /// or the machine will not say. Informative only; the engine never
    /// chooses an output, it plays where the system does.
    fn speakers(&self) -> Option<String>;
    fn hearing(&self) -> Hearing;
    fn mixing(&self) -> Mixing;
    /// Whether the track that was playing has run out since this was last
    /// asked. True exactly once per track, so asking twice does not skip one.
    ///
    /// The pipeline reports it rather than deciding it, because *which* track
    /// comes next is a decision (the rotation refuses what it played recently)
    /// and decisions do not live in an adapter.
    fn music_ended(&mut self) -> bool {
        false
    }
}

impl Engine {
    /// Deliberately narrow: the two meters and nothing else, because
    /// this is asked twelve times a second and the status is not.
    pub(super) fn levels(&mut self) -> Reply {
        Reply::Levels {
            hearing: self.pipeline.hearing(),
            mixing: self.pipeline.mixing(),
            audio_layers: self.audio_layers_heard(),
            layer_flowing: self
                .status
                .layers
                .iter()
                .map(|layer| (layer.id.clone(), self.pipeline.layer_flowing(&layer.id)))
                .collect(),
        }
    }

    pub(super) fn mute(&mut self, on: bool) -> Reply {
        self.status.muted = on;
        self.sound()
    }

    pub(super) fn volume(&mut self, level: f64) -> Reply {
        // Up to 200%, which is what a fader offers. Clamping at unity
        // here makes a quiet microphone unraisable.
        self.status.faders.mic = level.clamp(0.0, crate::sound::music::MAX_GAIN);
        self.sound()
    }

    pub(super) fn music_volume(&mut self, level: f64) -> Reply {
        self.status.faders.music = level.clamp(0.0, 1.0);
        self.sound()
    }

    pub(super) fn duck(&mut self, db: f64) -> Reply {
        // Downward only: a duck that raises the music over the voice
        // is not a duck, and the slider only ever goes one way.
        self.status.faders.duck_db = db.clamp(-30.0, 0.0);
        self.sound()
    }

    /// Hearing your own mix. It answers with a status rather than
    /// `ok`, because it is a switch that makes a noise in the room and
    /// every face has to agree about whether it is on.
    pub(super) fn monitor(&mut self, on: bool) -> Reply {
        match self.pipeline.monitor(on) {
            Ok(()) => {
                self.status.monitoring = on;
                Reply::Status(Box::new(self.reported()))
            }
            Err(message) => Reply::Error { message },
        }
    }

    /// Whether the bed reaches the audience. Its own switch, and not
    /// the same one as the speakers: hearing the music is what a person
    /// turned it on for, sending it out is the part that is optional.
    pub(super) fn stream_music(&mut self, on: bool) -> Reply {
        self.status.music_to_stream = on;
        self.sound()
    }

    /// `remux play clap`: the file the name means, once, over everything.
    pub(super) fn play_clip(&mut self, name: &str) -> Reply {
        let root = crate::sound::clips::root();
        let Some(path) = crate::sound::clips::find(name, &root, |p| p.is_file()) else {
            return Reply::Error {
                message: format!(
                    "no clip called {name}: not a file, and not in {} ({})",
                    root.display(),
                    match crate::sound::clips::list(&root).as_slice() {
                        [] => "which is empty".to_string(),
                        names => names.join(", "),
                    }
                ),
            };
        };
        match self.pipeline.clip(&path) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn denoise(&mut self, on: bool) -> Reply {
        self.status.denoise = on;
        self.pipeline.denoise(on);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn choose_mic(&mut self, device: Option<String>) -> Reply {
        match device {
            None => match self.pipeline.mic(None) {
                Ok(()) => {
                    self.status.mic = None;
                    Reply::Status(Box::new(self.reported()))
                }
                Err(why) => Reply::Error { message: why },
            },
            Some(query) => self.open_mic(query),
        }
    }

    pub(super) fn next_track(&mut self) -> Reply {
        match self.playing.is_some() {
            true => self.advance(),
            false => Reply::Error {
                message: "no genre is playing".into(),
            },
        }
    }

    pub(super) fn music(&mut self, on: bool) -> Reply {
        if on {
            match self.playing.as_ref().map(|(genre, _)| genre.clone()) {
                Some(genre) => self.pick_genre(&genre),
                // Turning music on without having chosen a genre picks
                // the first one rather than refusing. The panel's
                // switch is a switch, not a question.
                None => match self.library.playlists().first() {
                    Some(first) => {
                        let name = first.name.clone();
                        self.pick_genre(&name)
                    }
                    None => no_music(),
                },
            }
        } else {
            match self.pipeline.play(None) {
                Ok(()) => {
                    self.status.music = None;
                    // The music's switch is what makes it heard, so
                    // off closes the speakers it opened. An output
                    // that will not close is not worth refusing over.
                    if self.status.monitoring && self.pipeline.monitor(false).is_ok() {
                        self.status.monitoring = false;
                    }
                    Reply::Status(Box::new(self.reported()))
                }
                Err(why) => Reply::Error { message: why },
            }
        }
    }

    /// The gate, tuned while it runs. The patch is what a panel sends
    /// because a panel moves one slider at a time; what comes back is
    /// the whole set, so every other face redraws from one answer.
    pub(super) fn gate(&mut self, patch: serde_json::Value) -> Reply {
        let patch = crate::sound::mixer::gate::parse_gate_params(&patch);
        let mut params = self.status.gate;
        if let Some(v) = patch.hf {
            params.hf = v;
        }
        if let Some(v) = patch.full {
            params.full = v;
        }
        if let Some(v) = patch.floor {
            params.floor = v;
        }
        if let Some(v) = patch.hold_ms {
            params.hold_ms = v;
        }
        if let Some(v) = patch.attack_ms {
            params.attack_ms = v;
        }
        if let Some(v) = patch.hf_attack_ms {
            params.hf_attack_ms = v;
        }
        if let Some(v) = patch.keys_boost {
            params.keys_boost = v;
        }
        self.status.gate = params;
        self.pipeline.gate(params);
        Reply::Status(Box::new(self.reported()))
    }
    /// Push the faders down to the mixer and answer with the new state.
    ///
    /// Pushed rather than pulled: a fader nobody touched should cost nothing,
    /// and the mixer runs a hundred times a second.
    pub(super) fn sound(&mut self) -> Reply {
        match self.pipeline.levels(SoundLevels {
            mic: self.status.faders.mic,
            music: self.status.faders.music,
            duck_db: self.status.faders.duck_db,
            muted: self.status.muted,
            music_to_stream: self.status.music_to_stream,
        }) {
            Ok(()) => Reply::Status(Box::new(self.reported())),
            Err(why) => Reply::Error { message: why },
        }
    }

    /// Put a genre on the shelf without playing it: what a restart brings
    /// back. `music on` resumes it; nothing else starts music by itself,
    /// because an engine that came up playing was music through the
    /// speakers of a room nobody was in.
    pub(super) fn shelve(&mut self, name: &str) {
        let playlists = self.library.playlists();
        if let Some(playlist) = playlists.iter().find(|p| p.name == name) {
            self.playing = Some((
                playlist.name.clone(),
                Rotation::new(playlist.tracks.clone()),
            ));
        }
    }

    /// Start a genre from the beginning of its rotation.
    pub(super) fn pick_genre(&mut self, name: &str) -> Reply {
        let playlists = self.library.playlists();
        if playlists.is_empty() {
            return no_music();
        }
        let Some(playlist) = playlists.iter().find(|p| p.name == name) else {
            return Reply::Error {
                message: format!(
                    "no genre called {name:?}. There is {}",
                    list_of(playlists.iter().map(|p| p.name.clone()))
                ),
            };
        };
        self.playing = Some((
            playlist.name.clone(),
            Rotation::new(playlist.tracks.clone()),
        ));
        let reply = self.advance();
        // Choosing music is choosing to hear it. The speakers open here and
        // not on every track, so a person who turned them off mid-set is not
        // argued with at the next song; and an output that will not open is
        // no reason for the music not to start, so the answer is not checked.
        if matches!(reply, Reply::Status(_)) && !self.status.monitoring {
            if let Ok(()) = self.pipeline.monitor(true) {
                self.status.monitoring = true;
            }
        }
        reply
    }

    /// Move to the next track and say what is playing.
    ///
    /// The rotation refuses to repeat what it played recently, which is what
    /// makes eight hours on twenty-five tracks bearable. Skipping goes through
    /// here too, so a skip counts as having played it.
    pub(super) fn advance(&mut self) -> Reply {
        let Some((_, rotation)) = self.playing.as_mut() else {
            return Reply::Error {
                message: "no genre is playing".into(),
            };
        };
        let Some(track) = rotation.next() else {
            return Reply::Error {
                message: "that genre has no tracks in it".into(),
            };
        };
        match self.pipeline.play(Some(&track)) {
            Ok(()) => {
                self.status.music = Some(match &track.artist {
                    Some(artist) => format!("{artist} — {}", track.title),
                    None => track.title.clone(),
                });
                Reply::Status(Box::new(self.reported()))
            }
            Err(why) => Reply::Error { message: why },
        }
    }

    /// Find the microphone somebody meant and actually open it.
    ///
    /// Near enough to [`Self::open_camera`] to be tempting, and left alone on
    /// purpose. There was a shared helper here, and it stopped working the day
    /// both of them had to actually *open* the device rather than remember a
    /// name: a camera and a microphone fail differently and their lists are
    /// not interchangeable. Two of a thing is not yet a pattern.
    pub(super) fn open_mic(&mut self, query: String) -> Reply {
        let available = match self.sources.available() {
            Ok(available) => available,
            Err(why) => return Reply::Error { message: why },
        };
        let Some(mic) = pick_device(&query, &available.mics) else {
            return Reply::Error {
                message: format!(
                    "no microphone matches {query:?}. There is {}",
                    list_of(available.mics.iter().map(|m| m.name.clone()))
                ),
            };
        };
        match self.pipeline.mic(Some(&mic.id)) {
            Ok(()) => {
                self.status.mic = Some(mic.name.clone());
                Reply::Status(Box::new(self.reported()))
            }
            Err(why) => Reply::Error { message: why },
        }
    }
}

/// Nothing in the music folder, which is every machine on its first day: no
/// track ships with remux, since the ones free for a stream are not ours to
/// redistribute. So the refusal says where tracks go and where to find some.
fn no_music() -> Reply {
    Reply::Error {
        message: format!(
            "no music yet: put tracks in {}, one folder per genre \
             (free for streams: https://www.streambeats.com)",
            crate::config::music_dir().join("lofi").display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::*;

    #[test]
    fn a_fader_outside_its_travel_is_brought_back_into_it() {
        let mut engine = engine();
        engine.handle(Command::Volume { level: 3.0 });
        assert!(
            (engine.mic_db() - 6.0206).abs() < 0.001,
            "it stops at the 200% the panel offers, which is +6 dB, not at unity"
        );
        engine.handle(Command::Volume { level: -1.0 });
        assert_eq!(engine.mic_db(), f64::NEG_INFINITY);
    }

    #[test]
    fn the_duck_only_ever_goes_downward() {
        let mut engine = engine();
        engine.handle(Command::Duck { db: 12.0 });
        assert_eq!(
            engine.duck_db(),
            0.0,
            "a duck that raises the music is not a duck"
        );
        engine.handle(Command::Duck { db: -99.0 });
        assert_eq!(engine.duck_db(), -30.0);
    }

    #[test]
    fn asking_for_a_device_that_is_not_plugged_in_says_what_is() {
        let mut engine = machine();
        let Reply::Error { message } = engine.handle(Command::Mic {
            device: Some("Rode NT-USB".into()),
        }) else {
            panic!("an absent device is an error")
        };
        assert!(message.contains("Rode NT-USB"), "{message}");
        assert!(
            message.contains("HyperX DuoCast and MacBook Pro Microphone"),
            "it should say what there is: {message}"
        );
        assert_eq!(engine.state().mic, None, "and change nothing");
    }

    #[test]
    fn a_clip_nobody_has_is_refused_by_name_and_the_folder_is_named() {
        let mut one = engine();
        let Reply::Error { message } = one.handle(Command::Clip {
            name: "no-such-clip".into(),
        }) else {
            panic!("a missing clip is an error")
        };
        assert!(
            message.contains("no-such-clip") && message.contains("clips"),
            "{message}"
        );
    }

    #[test]
    fn a_panel_can_read_the_faders_back_where_it_put_them() {
        let (mut engine, _) = publishing_engine(None);
        engine.handle(Command::Volume { level: 1.5 });
        engine.handle(Command::MusicVolume { level: 0.3 });
        engine.handle(Command::Duck { db: -24.0 });
        let faders = engine.state().faders;
        assert!((faders.mic - 1.5).abs() < 1e-9, "the microphone past unity");
        assert!((faders.music - 0.3).abs() < 1e-9);
        assert!((faders.duck_db + 24.0).abs() < 1e-9);
    }

    #[test]
    fn the_faders_start_where_the_engine_starts_them() {
        let faders = engine().state().faders;
        assert!((faders.mic - 1.0).abs() < 1e-9, "unity, not silence");
        assert_eq!(faders, crate::protocol::Faders::default());
    }

    #[test]
    fn the_gate_is_tuned_a_slider_at_a_time_and_answers_with_the_whole_set() {
        let gated: std::sync::Arc<std::sync::Mutex<Option<GateParams>>> = Default::default();
        let pipeline = Wrote {
            told: Default::default(),
            cameras: Default::default(),
            mics: Default::default(),
            played: Default::default(),
            levels: Default::default(),
            shown: Default::default(),
            counting: Default::default(),
            published: Default::default(),
            recorded: Default::default(),
            mirrored: Default::default(),
            gated: gated.clone(),
            heard_here: Default::default(),
            speaker_calls: Default::default(),
            previewed: Default::default(),
            ran_out: Default::default(),
            refuse: None,
            scene_events: Default::default(),
            ducked: Default::default(),
            heard: Default::default(),
            audio_heard: Default::default(),
        };
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));

        let before = engine.state().gate;
        let Reply::Status(after) = engine.handle(Command::Gate {
            patch: serde_json::json!({ "full": 0.2 }),
        }) else {
            panic!("the gate answers with a status, so every face redraws from one answer")
        };
        assert!(
            (after.gate.full - 0.2).abs() < 1e-9,
            "the slider that moved"
        );
        assert!(
            (after.gate.hf - before.hf).abs() < 1e-9,
            "and only that one: a patch is not a replacement"
        );
        assert_eq!(
            gated.lock().expect("gated").map(|p| p.full),
            Some(0.2),
            "the microphone has to be told, not just the status"
        );
    }

    #[test]
    fn hearing_your_own_mix_is_a_switch_every_face_can_read() {
        let (mut engine, _) = publishing_engine(None);
        assert!(!engine.state().monitoring, "the speakers start quiet");
        let Reply::Status(after) = engine.handle(Command::Monitor { on: true }) else {
            panic!("it answers with a status: this switch makes a noise in a room")
        };
        assert!(after.monitoring);
        engine.handle(Command::Monitor { on: false });
        assert!(!engine.state().monitoring);
    }

    // "I had to be able to hear the music even without streaming it. Sending
    // it to the stream is the optional part, not hearing it. If it is on, I
    // always hear it." So starting the music opens the speakers, and whether
    // the bed reaches the audience is its own switch, on unless somebody says
    // otherwise. The speakers stay a switch of their own because on speakers
    // the microphone picks the music up and it goes out twice.
    #[test]
    fn hearing_the_music_is_not_optional_and_sending_it_out_is() {
        let levels: Faders = Default::default();
        let heard_here: std::sync::Arc<std::sync::Mutex<bool>> = Default::default();
        let pipeline = Wrote {
            levels: levels.clone(),
            heard_here: heard_here.clone(),
            ..Default::default()
        };
        let mut engine = Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_library(Box::new(ThreeGenres));
        assert!(
            engine.state().music_to_stream,
            "the bed reaches the audience unless somebody says otherwise"
        );
        assert!(
            !engine.state().monitoring,
            "quiet until there is something to hear"
        );

        engine.handle(Command::Genre { name: "edm".into() });
        assert!(
            engine.state().monitoring,
            "starting the music opens the speakers"
        );
        assert!(
            *heard_here.lock().expect("heard here"),
            "and the pipeline was told"
        );

        let Reply::Status(after) = engine.handle(Command::StreamMusic { on: false }) else {
            panic!("a switch every face reads answers with a status")
        };
        assert!(!after.music_to_stream);
        assert_eq!(
            levels.lock().expect("levels").map(|l| l.4),
            Some(false),
            "the mixer was told to keep the bed out of the mix"
        );
        assert!(
            after.monitoring,
            "and the speakers are untouched: they are the other switch"
        );
    }

    // The music's own switch is what makes it heard: on opens the speakers
    // (proven above), so off closes them, or the switch says one thing and
    // the room says another.
    // A person with a headset and a pair of speakers wants to know which one
    // the bed is on, without opening System Settings; the status names it while
    // the speakers are open and says nothing while they are closed.
    #[test]
    fn the_status_names_the_speakers_while_they_are_open() {
        let (mut engine, _, _) = machine_with_music();
        assert_eq!(engine.reported().speakers, None);
        engine.handle(Command::Genre { name: "edm".into() });
        assert_eq!(engine.reported().speakers.as_deref(), Some("Desk speakers"));
        engine.handle(Command::Monitor { on: false });
        assert_eq!(engine.reported().speakers, None);
    }

    #[test]
    fn turning_the_music_off_closes_the_speakers() {
        let (mut engine, _, _) = machine_with_music();
        engine.handle(Command::Genre { name: "edm".into() });
        assert!(engine.state().monitoring);
        engine.handle(Command::Music { on: false });
        assert!(
            !engine.state().monitoring,
            "the speakers close with the music"
        );
    }

    #[test]
    fn an_engine_that_cannot_open_an_output_says_so_and_stays_quiet() {
        let (mut engine, _) = publishing_engine(Some("no output device".into()));
        let reply = engine.handle(Command::Monitor { on: true });
        assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
        assert!(
            !engine.state().monitoring,
            "a switch that says it is on while nothing is playing is the worst \
                 possible answer for this one"
        );
    }

    // Nobody picks a genre in order to not hear it, so choosing one starts it.
    #[test]
    fn choosing_a_genre_starts_playing_it() {
        let (mut engine, played, _) = machine_with_music();
        let Reply::Status(status) = engine.handle(Command::Genre {
            name: "lofi".into(),
        }) else {
            panic!("choosing a genre answers a status")
        };
        assert!(
            status
                .music
                .as_deref()
                .unwrap_or_default()
                .starts_with("StreamBeats — lofi"),
            "the status says what is playing, got {:?}",
            status.music
        );
        assert_eq!(played.lock().expect("played").len(), 1);
    }

    #[test]
    fn asking_for_a_genre_that_is_not_there_says_what_is() {
        let (mut engine, _, _) = machine_with_music();
        let Reply::Error { message } = engine.handle(Command::Genre {
            name: "drum-and-bass".into(),
        }) else {
            panic!("an absent genre is an error")
        };
        assert!(message.contains("lofi, edm and synthwave"), "{message}");
    }

    // Eight hours on twenty-five tracks is only bearable because the rotation
    // refuses to repeat what it played recently.
    #[test]
    fn the_next_track_is_never_the_one_just_played() {
        let (mut engine, played, _) = machine_with_music();
        engine.handle(Command::Genre { name: "edm".into() });
        for _ in 0..5 {
            engine.handle(Command::NextTrack);
        }
        let played = played.lock().expect("played").clone();
        assert_eq!(played.len(), 6);
        for pair in played.windows(2) {
            assert_ne!(
                pair[0], pair[1],
                "a track came back immediately: {played:?}"
            );
        }
    }

    #[test]
    fn a_track_that_runs_out_starts_the_next_one() {
        let (mut engine, played, ran_out) = machine_with_music();
        engine.handle(Command::Genre { name: "edm".into() });
        assert_eq!(played.lock().expect("played").len(), 1);

        *ran_out.lock().expect("ran out") = true;
        engine.tick();
        let played = played.lock().expect("played").clone();
        assert_eq!(played.len(), 2, "the next track never started: {played:?}");
        assert_ne!(played[0], played[1], "it played the same file again");
    }

    #[test]
    fn a_tick_with_nothing_finished_starts_nothing() {
        let (mut engine, played, _) = machine_with_music();
        engine.handle(Command::Genre { name: "edm".into() });
        for _ in 0..10 {
            engine.tick();
        }
        assert_eq!(played.lock().expect("played").len(), 1);
    }

    // Stopping the music and a track running out arrive in either order, and
    // the one that arrives second must not restart what was just stopped.
    #[test]
    fn a_track_running_out_after_stop_stays_stopped() {
        let (mut engine, played, ran_out) = machine_with_music();
        engine.handle(Command::Genre { name: "edm".into() });
        engine.handle(Command::Music { on: false });
        let before = played.lock().expect("played").len();

        *ran_out.lock().expect("ran out") = true;
        engine.tick();
        assert_eq!(played.lock().expect("played").len(), before);
    }

    #[test]
    fn skipping_with_nothing_playing_says_so_rather_than_guessing() {
        let (mut engine, _, _) = machine_with_music();
        assert!(matches!(
            engine.handle(Command::NextTrack),
            Reply::Error { .. }
        ));
    }

    // The panel's music control is a switch, not a question. Turning it on
    // without a genre chosen picks one rather than refusing.
    #[test]
    fn turning_music_on_with_no_genre_chosen_picks_one() {
        let (mut engine, played, _) = machine_with_music();
        engine.handle(Command::Music { on: true });
        assert!(engine.state().music.is_some());
        assert_eq!(played.lock().expect("played").len(), 1);
    }

    #[test]
    fn turning_music_off_stops_it_and_says_nothing_is_playing() {
        let (mut engine, played, _) = machine_with_music();
        engine.handle(Command::Genre {
            name: "lofi".into(),
        });
        engine.handle(Command::Music { on: false });
        assert_eq!(engine.state().music, None);
        assert_eq!(
            played.lock().expect("played").last(),
            Some(&None),
            "stopping is telling the pipeline, not only forgetting"
        );
    }

    #[test]
    fn an_engine_with_no_music_folder_says_so_rather_than_pretending() {
        let mut engine = Engine::new();
        assert!(matches!(
            engine.handle(Command::Music { on: true }),
            Reply::Error { .. }
        ));
    }

    fn speaking_engine() -> (Engine, std::sync::Arc<std::sync::Mutex<Vec<bool>>>) {
        let speaker_calls: std::sync::Arc<std::sync::Mutex<Vec<bool>>> = Default::default();
        let pipeline = Wrote {
            speaker_calls: speaker_calls.clone(),
            ..Default::default()
        };
        (
            Engine::with_sources(Box::new(ThisMachine))
                .with_pipeline(Box::new(pipeline))
                .with_library(Box::new(ThreeGenres)),
            speaker_calls,
        )
    }

    // Off closes the speakers the music opened; with the speakers already closed
    // there is nothing to close, and telling them so anyway is a call an output
    // that refuses would turn into an error for a switch that did nothing.
    #[test]
    fn turning_the_music_off_with_the_speakers_already_closed_leaves_them_alone() {
        let (mut engine, calls) = speaking_engine();
        engine.handle(Command::Genre { name: "edm".into() });
        engine.handle(Command::Monitor { on: false });
        assert_eq!(*calls.lock().expect("calls"), vec![true, false]);
        engine.handle(Command::Music { on: false });
        assert_eq!(
            *calls.lock().expect("calls"),
            vec![true, false],
            "the speakers were not told again"
        );
    }

    // The speakers open with the first genre and not with every one, so a person
    // who turned them off mid-set is not argued with at the next shelf either.
    #[test]
    fn picking_another_genre_while_already_hearing_it_does_not_open_the_speakers_again() {
        let (mut engine, calls) = speaking_engine();
        engine.handle(Command::Genre {
            name: "lofi".into(),
        });
        engine.handle(Command::Genre { name: "edm".into() });
        assert_eq!(*calls.lock().expect("calls"), vec![true]);
    }

    // A genre that could not start opens nothing: the speakers follow the music,
    // not the attempt.
    #[test]
    fn a_genre_that_refuses_to_play_opens_no_speakers() {
        let calls: std::sync::Arc<std::sync::Mutex<Vec<bool>>> = Default::default();
        let pipeline = Wrote {
            speaker_calls: calls.clone(),
            refuse: Some("no output".into()),
            ..Default::default()
        };
        let mut engine = Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_library(Box::new(ThreeGenres));
        let reply = engine.handle(Command::Genre {
            name: "lofi".into(),
        });
        assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
        assert!(calls.lock().expect("calls").is_empty());
    }

    // A track never ends on an engine that plays nothing, so the tick leaves the
    // music where it is.
    #[test]
    fn a_track_never_ends_where_nothing_plays() {
        let mut engine = Engine::new().with_library(Box::new(ThreeGenres));
        engine.handle(Command::Genre {
            name: "lofi".into(),
        });
        let playing = engine.state().music.clone();
        assert!(playing.is_some());
        engine.tick();
        engine.tick();
        assert_eq!(
            engine.state().music,
            playing,
            "the tick did not skip a track"
        );
    }

    // The readings a panel asks for in dB come off the faders, and the speakers
    // read off the status.
    #[test]
    fn the_faders_read_in_db_and_the_speakers_read_off_the_status() {
        let (mut engine, _) = speaking_engine();
        assert_eq!(engine.music_db(), crate::sound::music::fader_db(0.85));
        assert_eq!(engine.mic_db(), crate::sound::music::fader_db(1.0));
        assert_eq!(engine.duck_db(), crate::sound::music::DUCK_DEFAULT_DB);
        assert!(!engine.monitoring());
        engine.handle(Command::MusicVolume { level: 0.25 });
        assert_eq!(engine.music_db(), crate::sound::music::fader_db(0.25));
        engine.handle(Command::Genre {
            name: "lofi".into(),
        });
        assert!(engine.monitoring());
    }

    // Choosing a genre opens the speakers. Closing them again must leave the
    // bed exactly where it was for the audience: still playing, still sent.
    #[test]
    fn the_speakers_close_while_the_bed_keeps_going_out() {
        let (mut engine, calls) = speaking_engine();
        engine.handle(Command::Genre {
            name: "lofi".into(),
        });
        assert!(engine.state().monitoring);
        let Reply::Status(after) = engine.handle(Command::Monitor { on: false }) else {
            panic!("a switch every face reads answers with a status")
        };
        assert!(!after.monitoring, "the speakers are closed");
        assert!(after.music.is_some(), "the bed is still playing");
        assert!(after.music_to_stream, "and still goes out");
        assert_eq!(*calls.lock().expect("calls"), vec![true, false]);
    }

    #[test]
    fn no_music_says_where_it_goes_and_where_to_find_some() {
        let folder = crate::config::music_dir().display().to_string();
        for command in [
            Command::Music { on: true },
            Command::Genre {
                name: "lofi".into(),
            },
        ] {
            let Reply::Error { message } = engine().handle(command) else {
                panic!("no music is an error")
            };
            assert!(
                message.contains(&folder) && message.contains("https://www.streambeats.com"),
                "{message}"
            );
        }
    }
}
