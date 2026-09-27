//! The sound: the microphone, the music, the mixer and the speakers.

use super::*;

/// The sound's half of the media path: the microphone, the music, the mixer
/// and the speakers. See [`crate::engine::Pipeline`] for why it is a port.
pub trait Sound: Send {
    /// Open a microphone by id, or close the one that is open.
    fn mic(&mut self, device: Option<&str>) -> Result<(), String>;
    /// Play a track, or stop. The engine decides *which* track; this makes the
    /// sound. Stopping is `None` rather than a separate verb because there is
    /// only ever one music track playing and it is either that one or none.
    fn play(&mut self, track: Option<&Track>) -> Result<(), String>;
    /// Where the faders are, and whether the music and the screen's sound are
    /// in the mix that leaves. Sent whenever one moves, rather than read by
    /// the mixer, so that a fader nobody touched costs nothing.
    fn levels(
        &mut self,
        mic: f64,
        music: f64,
        duck_db: f64,
        muted: bool,
        music_to_stream: bool,
        screen_sound: bool,
    ) -> Result<(), String>;
    /// Where the gate's thresholds are now. The whole set, not the patch: the
    /// engine is what holds them, so the machine is told the answer rather
    /// than being asked to work it out.
    fn gate(&mut self, params: GateParams);
    /// Take the room out of the microphone before the gate, or stop.
    fn denoise(&mut self, _on: bool) {}
    /// Hear these applications alone in the screen's sound, or, empty, the
    /// whole screen again.
    fn hear(&mut self, _apps: &[String]) -> Result<(), String> {
        Ok(())
    }
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
        }
    }

    pub(super) fn mute(&mut self, on: bool) -> Reply {
        self.status.muted = on;
        self.sound()
    }

    pub(super) fn volume(&mut self, level: f64) -> Reply {
        // Up to 200%, which is what a fader offers. Clamping at unity
        // here makes a quiet microphone unraisable.
        self.status.faders.mic = level.clamp(0.0, crate::music::MAX_GAIN);
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

    /// Whether what the computer plays reaches the audience. Its own switch, off
    /// by default: the screen is chosen, what happens to be sounding on it
    /// is not, until somebody says so.
    /// `remux play clap`: the file the name means, once, over everything.
    pub(super) fn play_clip(&mut self, name: &str) -> Reply {
        let root = crate::clips::root();
        let Some(path) = crate::clips::find(name, &root, |p| p.is_file()) else {
            return Reply::Error {
                message: format!(
                    "no clip called {name}: not a file, and not in {} ({})",
                    root.display(),
                    match crate::clips::list(&root).as_slice() {
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

    pub(super) fn screen_sound(&mut self, on: bool) -> Reply {
        self.status.screen_sound = on;
        self.sound()
    }

    /// `remux hear Spotify`: that application's sound alone, by the name
    /// the system lists it under, and the screen's sound switched on with
    /// it, since nobody names an app they do not want heard.
    pub(super) fn hear(&mut self, apps: Vec<String>) -> Reply {
        let names = if apps.is_empty() {
            Vec::new()
        } else {
            let running = match self.sources.available() {
                Ok(available) => available.apps,
                Err(why) => return Reply::Error { message: why },
            };
            let mut names = Vec::new();
            for asked in &apps {
                let wanted = asked.to_lowercase();
                match running
                    .iter()
                    .find(|name| name.to_lowercase() == wanted)
                    .or_else(|| {
                        running
                            .iter()
                            .find(|name| name.to_lowercase().contains(&wanted))
                    }) {
                    Some(name) => names.push(name.clone()),
                    None => {
                        return Reply::Error {
                            message: format!(
                                "{asked} is not running; there is {}",
                                list_of(running.iter().cloned())
                            ),
                        }
                    }
                }
            }
            names
        };
        if let Err(message) = self.pipeline.hear(&names) {
            return Reply::Error { message };
        }
        self.status.hearing_apps = names;
        if !self.status.hearing_apps.is_empty() {
            self.status.screen_sound = true;
        }
        self.sound()
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
                    None => Reply::Error {
                        message: "there is no music in the folder".into(),
                    },
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
        let patch = crate::gate::parse_gate_params(&patch);
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
        match self.pipeline.levels(
            self.status.faders.mic,
            self.status.faders.music,
            self.status.faders.duck_db,
            self.status.muted,
            self.status.music_to_stream,
            self.status.screen_sound,
        ) {
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
