//! The active scene's sounds beside the microphone: one capture per ID.
//! Never report a source that failed to open.
use super::*;
use crate::sound::audio_layers::{transition, Duck, Kind, Layer, Source, Transition};

/// Names no layer can have (an ID has no colon), so a capture opened or kept
/// across a switch never meets one of the scene's own on its way.
fn switching(at: usize) -> String {
    format!("{at}:switching")
}
fn kept(at: usize) -> String {
    format!("{at}:kept")
}

impl Engine {
    /// The next scene's sounds the active one does not hear, opened muted
    /// before anything on the air changes. One that will not open lets the
    /// others go and refuses the switch, as a picture's capture does.
    pub(super) fn audio_prepare(&mut self, to: &[Layer]) -> Result<Transition, String> {
        let plan = transition(&self.status.audio_layers, to);
        for (at, layer) in plan.open.iter().enumerate() {
            let mut quiet = layer.clone();
            quiet.id = switching(at);
            quiet.muted = true;
            if let Err(why) = self.pipeline.audio_layer_add(&quiet) {
                for back in (0..at).rev() {
                    self.pipeline.audio_layer_remove(&switching(back));
                }
                return Err(format!("audio layer {}: {why}", layer.id));
            }
        }
        Ok(plan)
    }

    /// The scene's sounds' meters, in its order; one the motor has nothing
    /// on reads as silent with nothing handed over.
    pub(super) fn audio_layers_heard(&self) -> Vec<crate::protocol::AudioLayerHeard> {
        let mut heard = self.pipeline.audio_layers_heard();
        self.status
            .audio_layers
            .iter()
            .map(|layer| {
                heard
                    .iter()
                    .position(|h| h.id == layer.id)
                    .map(|at| heard.swap_remove(at))
                    .unwrap_or_else(|| crate::protocol::AudioLayerHeard::silent(layer.id.clone()))
            })
            .collect()
    }

    /// A switch that did not happen: what was prepared is let go.
    pub(super) fn audio_abandon(&mut self, plan: &Transition) {
        for at in (0..plan.open.len()).rev() {
            self.pipeline.audio_layer_remove(&switching(at));
        }
    }

    /// The switch happened: the old scene's other sounds close, the kept ones
    /// take the new scene's IDs and levels, the prepared ones are heard.
    pub(super) fn audio_commit(&mut self, plan: Transition, to: Vec<Layer>) {
        for id in &plan.close {
            self.pipeline.audio_layer_remove(id);
        }
        let renamed: Vec<_> = plan
            .keep
            .iter()
            .filter(|(old, new)| *old != new.id)
            .collect();
        for (at, (old, _)) in renamed.iter().enumerate() {
            self.pipeline.audio_layer_rename(old, &kept(at));
        }
        for (at, (_, new)) in renamed.iter().enumerate() {
            self.pipeline.audio_layer_rename(&kept(at), &new.id);
        }
        for (at, layer) in plan.open.iter().enumerate() {
            self.pipeline.audio_layer_rename(&switching(at), &layer.id);
            self.pipeline
                .audio_layer_levels(&layer.id, layer.volume, layer.muted);
        }
        for (old, layer) in &plan.keep {
            let Some(was) = self.status.audio_layers.iter().find(|l| &l.id == old) else {
                continue;
            };
            if was.volume != layer.volume || was.muted != layer.muted {
                self.pipeline
                    .audio_layer_levels(&layer.id, layer.volume, layer.muted);
            }
            if was.ducks() != layer.ducks() {
                self.pipeline.audio_layer_duck(&layer.id, layer.ducks());
            }
        }
        self.status.audio_layers = to;
    }

    pub(super) fn audio_layer_add(&mut self, id: String, source: Source) -> Reply {
        let source = match self.a_microphone_here(source) {
            Ok(source) => source,
            Err(message) => return Reply::Error { message },
        };
        let layer = match Layer::new(id, source) {
            Ok(layer) => layer,
            Err(message) => return Reply::Error { message },
        };
        if self.status.audio_layers.len() >= 16 {
            return Reply::Error {
                message: "at most 16 audio layers can be open".into(),
            };
        }
        if self.status.audio_layers.iter().any(|l| l.id == layer.id) {
            return Reply::Error {
                message: format!("audio layer {:?} already exists", layer.id),
            };
        }
        if let Err(message) = self.pipeline.audio_layer_add(&layer) {
            return Reply::Error { message };
        }
        self.status.audio_layers.push(layer);
        Reply::Status(Box::new(self.reported()))
    }

    /// A microphone by a device this machine has, kept by its id, as the main
    /// microphone is chosen; the other kinds as they were said.
    fn a_microphone_here(&self, source: Source) -> Result<Source, String> {
        let Some(query) = source
            .device
            .as_deref()
            .filter(|_| source.kind == Kind::Mic)
        else {
            return Ok(source);
        };
        let available = self.sources.available()?;
        pick_device(query, &available.mics)
            .map(|mic| Source::mic(mic.id.clone()))
            .ok_or_else(|| {
                format!(
                    "no microphone matches {query:?}. There is {}",
                    list_of(available.mics.iter().map(|m| m.name.clone()))
                )
            })
    }

    pub(super) fn audio_layer_remove(&mut self, id: String) -> Reply {
        let Some(index) = self.status.audio_layers.iter().position(|l| l.id == id) else {
            if self.forget_kept(|kept| kept.sounds.retain(|(_, sound)| sound.id != id)) {
                return Reply::Status(Box::new(self.reported()));
            }
            return Reply::Error {
                message: format!("no audio layer {id:?}"),
            };
        };
        self.pipeline.audio_layer_remove(&id);
        self.status.audio_layers.remove(index);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn audio_layer_volume(&mut self, id: String, volume: f64) -> Reply {
        if !volume.is_finite() || !(0.0..=2.0).contains(&volume) {
            return Reply::Error {
                message: "audio layer volume must be between 0 and 2".into(),
            };
        }
        let Some(layer) = self.status.audio_layers.iter_mut().find(|l| l.id == id) else {
            return Reply::Error {
                message: format!("no audio layer {id:?}"),
            };
        };
        layer.volume = volume;
        self.pipeline.audio_layer_levels(&id, volume, layer.muted);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn audio_layer_mute(&mut self, id: String, on: bool) -> Reply {
        let Some(layer) = self.status.audio_layers.iter_mut().find(|l| l.id == id) else {
            return Reply::Error {
                message: format!("no audio layer {id:?}"),
            };
        };
        layer.muted = on;
        self.pipeline.audio_layer_levels(&id, layer.volume, on);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn audio_layer_duck(&mut self, id: String, duck: Duck) -> Reply {
        let Some(layer) = self.status.audio_layers.iter_mut().find(|l| l.id == id) else {
            return Reply::Error {
                message: format!("no audio layer {id:?}"),
            };
        };
        layer.duck = duck;
        self.pipeline.audio_layer_duck(&id, layer.ducks());
        Reply::Status(Box::new(self.reported()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::*;

    #[test]
    fn independent_audio_layers_are_addressed_by_id_and_remembered() {
        use crate::sound::audio_layers::Source;
        let mut engine = engine();
        for id in ["browser", "editor"] {
            assert!(matches!(
                engine.handle(Command::AudioLayerAdd {
                    id: id.into(),
                    source: Source::app("Safari".into()),
                }),
                Reply::Status(_)
            ));
        }
        assert_eq!(engine.state().audio_layers.len(), 2);
        assert!(matches!(
            engine.handle(Command::AudioLayerVolume {
                id: "editor".into(),
                volume: 0.25
            }),
            Reply::Status(_)
        ));
        assert!(matches!(
            engine.handle(Command::AudioLayerMute {
                id: "browser".into(),
                on: true
            }),
            Reply::Status(_)
        ));
        assert_eq!(engine.state().audio_layers[0].volume, 1.0);
        assert!(engine.state().audio_layers[0].muted);
        assert_eq!(engine.state().audio_layers[1].volume, 0.25);
        assert!(!engine.state().audio_layers[1].muted);
        assert!(matches!(
            engine.handle(Command::AudioLayerAdd {
                id: "browser".into(),
                source: Source::mic("mic".into()),
            }),
            Reply::Error { .. }
        ));
        assert!(matches!(
            engine.handle(Command::AudioLayerVolume {
                id: "editor".into(),
                volume: f64::NAN
            }),
            Reply::Error { .. }
        ));
        let setup = engine.remembered();
        let mut restored = Engine::new();
        restored.restore(&setup);
        assert_eq!(restored.state().audio_layers, engine.state().audio_layers);
        assert!(matches!(
            engine.handle(Command::AudioLayerRemove {
                id: "browser".into()
            }),
            Reply::Status(_)
        ));
        assert_eq!(engine.state().audio_layers.len(), 1);
    }

    // A call captured as an app ducked under the voice like a video, and the
    // guest went quiet every time the operator spoke.
    #[test]
    fn one_audio_layer_is_told_whether_it_ducks_and_it_is_remembered() {
        let fake = Wrote::default();
        let ducked = fake.ducked.clone();
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        engine.handle(Command::AudioLayerAdd {
            id: "call".into(),
            source: Source::app("Discord".into()),
        });
        assert!(
            engine.state().audio_layers[0].ducks(),
            "an app ducks by its kind"
        );
        assert!(matches!(
            engine.handle(Command::AudioLayerDuck {
                id: "call".into(),
                duck: Duck::Off,
            }),
            Reply::Status(_)
        ));
        assert!(!engine.state().audio_layers[0].ducks());
        assert_eq!(*ducked.lock().unwrap(), [("call".to_string(), false)]);
        assert!(matches!(
            engine.handle(Command::AudioLayerDuck {
                id: "nobody".into(),
                duck: Duck::Off,
            }),
            Reply::Error { .. }
        ));

        let fake = Wrote::default();
        let told = fake.ducked.clone();
        let mut restored = Engine::new().with_pipeline(Box::new(fake));
        restored.restore(&engine.remembered());
        assert_eq!(restored.state().audio_layers[0].duck, Duck::Off);
        assert_eq!(*told.lock().unwrap(), [("call".to_string(), false)]);
    }

    fn add(engine: &mut Engine, id: &str, app: &str) {
        assert!(matches!(
            engine.handle(Command::AudioLayerAdd {
                id: id.into(),
                source: Source::app(app.into()),
            }),
            Reply::Status(_)
        ));
    }

    fn switch(engine: &mut Engine, name: &str) -> Reply {
        engine.handle(Command::SceneSwitch { name: name.into() })
    }

    // The music of a game kept playing over the talking head after the scene
    // switched to it: an audio layer belonged to no scene.
    #[test]
    fn an_audio_layer_belongs_to_its_scene_and_a_switch_keeps_what_both_hear() {
        let fake = Wrote::default();
        let heard = fake.heard.clone();
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        add(&mut engine, "call", "Discord");
        add(&mut engine, "game", "Steam");
        assert!(matches!(
            engine.handle(Command::SceneCreate {
                name: "talk".into()
            }),
            Reply::Status(_)
        ));
        assert!(
            engine.state().audio_layers.is_empty(),
            "a new scene is quiet"
        );
        add(&mut engine, "guest", "discord");
        heard.lock().unwrap().clear();

        assert!(matches!(switch(&mut engine, "default"), Reply::Status(_)));
        assert_eq!(
            engine
                .state()
                .audio_layers
                .iter()
                .map(|l| l.id.as_str())
                .collect::<Vec<_>>(),
            ["call", "game"]
        );
        let said = heard.lock().unwrap().clone();
        assert_eq!(said[0], "open 0:switching Steam muted", "{said:?}");
        assert!(!said.iter().any(|s| s.starts_with("close")), "{said:?}");
        assert!(
            said.contains(&"rename guest 0:kept".to_string())
                && said.contains(&"rename 0:kept call".to_string())
                && said.contains(&"rename 0:switching game".to_string()),
            "Discord stays open under the scene's own ID: {said:?}"
        );
        assert_eq!(said.last().unwrap(), "level game 1");

        heard.lock().unwrap().clear();
        assert!(matches!(switch(&mut engine, "talk"), Reply::Status(_)));
        let said = heard.lock().unwrap().clone();
        assert!(said.contains(&"close game".to_string()), "{said:?}");
        assert!(!said.iter().any(|s| s.starts_with("open")), "{said:?}");
        let talk = &engine.state().audio_layers;
        assert_eq!(talk.len(), 1);
        assert_eq!(talk[0].id, "guest");
        let default = engine
            .state()
            .scenes
            .iter()
            .find(|s| s.name == "default")
            .unwrap();
        assert_eq!(
            default.audio_layers.len(),
            2,
            "the other scene keeps its own"
        );
    }

    #[test]
    fn a_kept_capture_takes_the_new_scenes_level_and_duck() {
        let fake = Wrote::default();
        let heard = fake.heard.clone();
        let ducked = fake.ducked.clone();
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        add(&mut engine, "call", "Discord");
        engine.handle(Command::SceneDuplicate {
            name: "quiet".into(),
        });
        engine.handle(Command::AudioLayerVolume {
            id: "call".into(),
            volume: 0.5,
        });
        engine.handle(Command::AudioLayerDuck {
            id: "call".into(),
            duck: Duck::Off,
        });
        heard.lock().unwrap().clear();
        ducked.lock().unwrap().clear();
        assert!(matches!(switch(&mut engine, "default"), Reply::Status(_)));
        assert_eq!(*heard.lock().unwrap(), ["level call 1"]);
        assert_eq!(*ducked.lock().unwrap(), [("call".to_string(), true)]);
        assert_eq!(engine.state().audio_layers[0].volume, 1.0);
    }

    #[test]
    fn a_sound_that_will_not_open_refuses_the_switch_and_the_old_scene_plays_on() {
        let fake = Wrote::default();
        let heard = fake.heard.clone();
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        add(&mut engine, "call", "Discord");
        let mut scenes = engine.remembered().scenes;
        scenes.push(crate::picture::scenes::Scene {
            name: "games".into(),
            layers: vec![],
            elements: vec![],
            order: vec![],
            shader: None,
            audio_layers: vec![
                Layer::new("music".into(), Source::app("Spotify".into())).unwrap(),
                Layer::new("game".into(), Source::app("Steam".into())).unwrap(),
            ],
        });
        engine.status.scenes = scenes;
        engine.pipeline = Box::new(Wrote {
            refuse: Some("audio:Steam".into()),
            heard: heard.clone(),
            ..Default::default()
        });
        heard.lock().unwrap().clear();
        assert!(matches!(switch(&mut engine, "games"), Reply::Error { .. }));
        assert_eq!(engine.state().active_scene, "default");
        assert_eq!(engine.state().audio_layers.len(), 1);
        assert_eq!(
            *heard.lock().unwrap(),
            ["open 0:switching Spotify muted", "close 0:switching"],
            "what was prepared is let go, and Discord was never touched"
        );
    }

    #[test]
    fn each_scene_remembers_its_own_sound() {
        let mut engine = engine();
        add(&mut engine, "call", "Discord");
        engine.handle(Command::SceneCreate {
            name: "talk".into(),
        });
        add(&mut engine, "video", "Safari");
        let setup = engine.remembered();
        assert_eq!(
            setup.audio_layers.len(),
            1,
            "the active scene's, for an older engine"
        );

        let mut restored = Engine::new();
        restored.restore(&setup);
        assert_eq!(restored.state().active_scene, "talk");
        assert_eq!(restored.state().audio_layers[0].id, "video");
        assert!(matches!(switch(&mut restored, "default"), Reply::Status(_)));
        assert_eq!(restored.state().audio_layers[0].id, "call");
    }

    // Saved before a scene had a sound: every scene heard every audio layer,
    // and an upgrade changes nothing until somebody does.
    #[test]
    fn audio_layers_saved_before_scenes_had_them_are_in_every_scene() {
        let mut setup = crate::remembered::Remembered::default();
        for name in ["default", "talk"] {
            let mut scene = crate::picture::scenes::defaults().remove(0);
            scene.name = name.into();
            setup.scenes.push(scene);
        }
        setup.audio_layers =
            vec![Layer::new("call".into(), Source::app("Discord".into())).unwrap()];
        let mut restored = Engine::new();
        restored.restore(&setup);
        assert_eq!(restored.state().audio_layers.len(), 1);
        assert!(matches!(switch(&mut restored, "talk"), Reply::Status(_)));
        assert_eq!(restored.state().audio_layers.len(), 1);
    }

    // A microphone layer on "MacBook" was reported open and was silence: the
    // motor took the word for a device id that no device has.
    #[test]
    fn a_microphone_layer_is_a_microphone_this_machine_has() {
        let mut engine = machine();
        assert!(matches!(
            engine.handle(Command::AudioLayerAdd {
                id: "room".into(),
                source: Source::mic("MacBook".into()),
            }),
            Reply::Status(_)
        ));
        assert_eq!(
            engine.state().audio_layers[0].source,
            Source::mic("BuiltInMic".into()),
            "kept by its id, as the main microphone is"
        );
        let Reply::Error { message } = engine.handle(Command::AudioLayerAdd {
            id: "ghost".into(),
            source: Source::mic("AirPods".into()),
        }) else {
            panic!("a microphone nobody has was added");
        };
        assert!(message.contains("no microphone matches"), "{message}");
        assert_eq!(engine.state().audio_layers.len(), 1);
    }

    // A meter per sound, in the scene's order, so a face can draw one under
    // each fader and a person can see the one that is silent.
    #[test]
    fn levels_meter_each_sound_of_the_scene() {
        let fake = Wrote::default();
        let heard = fake.audio_heard.clone();
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        add(&mut engine, "music", "Spotify");
        add(&mut engine, "call", "Discord");
        heard.lock().unwrap().insert("call".into(), 9600);
        let Reply::Levels { audio_layers, .. } = engine.handle(Command::Levels) else {
            panic!("levels");
        };
        assert_eq!(
            audio_layers
                .iter()
                .map(|l| (l.id.as_str(), l.samples))
                .collect::<Vec<_>>(),
            [("music", 0), ("call", 9600)]
        );
    }
}
