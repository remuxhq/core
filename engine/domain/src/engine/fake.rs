//! The doubles the engine's tests share: sources, a pipeline that writes
//! down what it was told, a library, and engines built on them.

use super::*;
use crate::picture::layers::{Kind, Layer, Source, Transform};
use crate::picture::sources::{DisplayId, WindowId};

pub(super) fn engine() -> Engine {
    Engine::new()
}

/// A machine with two monitors and three windows: enough shape for every
/// decision the engine makes about sources. The built-in is display 1 and
/// the monitor is display 3, an ordering a real machine produces and the
/// reason position is never the handle.
pub(super) struct ThisMachine;

impl Sources for ThisMachine {
    fn available(&self) -> Result<Available, String> {
        Ok(Available {
            screens: vec![
                Screen {
                    id: DisplayId(1),
                    name: "Built-in Retina Display".into(),
                    stable: Some("37D8832A".into()),
                },
                Screen {
                    id: DisplayId(3),
                    name: "VG2791R".into(),
                    stable: Some("5C1E09B4".into()),
                },
            ],
            windows: vec![
                Window {
                    id: WindowId(10),
                    title: "tmux a".into(),
                    app: "Ghostty".into(),
                },
                Window {
                    id: WindowId(11),
                    title: "remux".into(),
                    app: "Brave Browser".into(),
                },
                Window {
                    id: WindowId(12),
                    title: "notes".into(),
                    app: "TextEdit".into(),
                },
            ],
            cameras: vec![
                Named {
                    id: "6C707041".into(),
                    name: "MacBook Pro Camera".into(),
                },
                Named {
                    id: "0x111000".into(),
                    name: "HP 430/435 FHD Webcam".into(),
                },
            ],
            mics: vec![
                Named {
                    id: "c9bf1690".into(),
                    name: "HyperX DuoCast".into(),
                },
                Named {
                    id: "BuiltInMic".into(),
                    name: "MacBook Pro Microphone".into(),
                },
            ],
            apps: vec!["Spotify".into(), "Brave Browser".into()],
        })
    }
}

pub(super) struct Refused;

impl Sources for Refused {
    fn available(&self) -> Result<Available, String> {
        Err("macOS refused: screen recording".into())
    }
}

pub(super) fn machine() -> Engine {
    Engine::with_sources(Box::new(ThisMachine))
}

pub(super) fn layer(id: &str, kind: Kind, handle: &str, x: i32) -> Layer {
    Layer {
        id: id.into(),
        source: Source {
            kind,
            handle: handle.into(),
            name: id.into(),
            width: 640,
            height: 480,
            stable: None,
        },
        transform: Transform {
            x,
            ..Transform::native((640, 480))
        },
        visible: true,
        crop: None,
        shape: None,
        shader: None,
        mirrored: false,
    }
}

pub(super) type Published = std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>;

/// A pipeline that writes down what it was told, so a test can ask whether
/// the capture was actually pointed somewhere rather than only whether the
/// status changed. The two disagreeing is the bug worth catching.
#[derive(Default)]
pub(super) struct Wrote {
    pub(super) told: std::sync::Arc<std::sync::Mutex<Vec<Behind>>>,
    pub(super) cameras: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    pub(super) mics: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    pub(super) played: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    pub(super) levels: Faders,
    pub(super) shown: Shown,
    pub(super) counting: std::sync::Arc<std::sync::Mutex<Option<Duration>>>,
    /// Every destination it was told to publish to, and a `None` for every
    /// time it was told to stop, in order.
    pub(super) published: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    /// Every file it was told to write, and a `None` for every stop.
    pub(super) recorded: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    pub(super) mirrored: std::sync::Arc<std::sync::Mutex<bool>>,
    pub(super) gated: std::sync::Arc<std::sync::Mutex<Option<GateParams>>>,
    pub(super) heard_here: std::sync::Arc<std::sync::Mutex<bool>>,
    /// Every time the speakers were told anything, on or off, in order. What
    /// "left alone" means is this not growing.
    pub(super) speaker_calls: std::sync::Arc<std::sync::Mutex<Vec<bool>>>,
    /// The last thing it was told about the preview: on, off, or never.
    pub(super) previewed: std::sync::Arc<std::sync::Mutex<Option<bool>>>,
    /// Set by a test to say the track that was playing has run out.
    pub(super) ran_out: std::sync::Arc<std::sync::Mutex<bool>>,
    pub(super) refuse: Option<String>,
    pub(super) scene_events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// Every audio layer told whether it ducks, in order.
    pub(super) ducked: std::sync::Arc<std::sync::Mutex<Vec<(String, bool)>>>,
    /// Every audio capture opened, renamed and closed, in order.
    pub(super) heard: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl Picture for Wrote {
    fn scene_transition(
        &mut self,
        from: &[crate::picture::layers::Layer],
        to: &[crate::picture::layers::Layer],
        elements: &[Element],
        shader: Option<&str>,
    ) -> Result<(), String> {
        let plan = crate::picture::scenes::transition(from, to);
        let mut events = self.scene_events.lock().unwrap();
        let mut prepared = Vec::new();
        for key in &plan.open {
            events.push(format!("prepare {:?}:{}", key.kind, key.handle));
            if let Some(reason) = &self.refuse {
                if (!reason.starts_with("scene:") && !reason.starts_with("shader:"))
                    || reason == &format!("scene:{}", key.handle)
                {
                    for source in prepared.iter().rev() {
                        events.push(format!("rollback {source}"));
                    }
                    events.push("rollback prepared".into());
                    return Err(reason.clone());
                }
            }
            prepared.push(key.handle.clone());
        }
        if let Some(reason) = &self.refuse {
            if reason.starts_with("shader:")
                && (shader == Some("bad.wgsl")
                    || to.iter().any(|l| l.shader.as_deref() == Some("bad.wgsl"))
                    || elements
                        .iter()
                        .any(|e| e.shader.as_deref() == Some("bad.wgsl")))
            {
                for source in prepared.iter().rev() {
                    events.push(format!("rollback {source}"));
                }
                events.push("rollback prepared".into());
                return Err(reason.clone());
            }
        }
        events.push("commit layout".into());
        for key in &plan.close {
            events.push(format!("close {:?}:{}", key.kind, key.handle));
        }
        Ok(())
    }
    fn show(
        &mut self,
        elements: &[Element],
        timers: &[(String, Duration)],
        _order: &[String],
    ) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            if !why.starts_with("shader:") {
                return Err(why.clone());
            }
        }
        self.shown
            .lock()
            .expect("shown")
            .push((elements.to_vec(), timers.to_vec()));
        *self.counting.lock().expect("counting") = timers.first().map(|(_, left)| *left);
        Ok(())
    }
    fn mirror(&mut self, on: bool) {
        *self.mirrored.lock().expect("mirrored") = on;
    }
    fn layer_remove(&mut self, _id: &str) {
        self.told.lock().unwrap().push(Behind::Nothing);
    }
    fn layer_replace(
        &mut self,
        old: &crate::picture::layers::Layer,
        new: &crate::picture::layers::Layer,
    ) -> Result<(u32, u32), crate::engine::LayerSwapError> {
        if let Some(reason) = &self.refuse {
            return Err(crate::engine::LayerSwapError {
                reason: reason.clone(),
                restored: reason != "rollback lost",
            });
        }
        if old.source.kind == new.source.kind && old.source.handle == new.source.handle {
            return Ok((old.source.width, old.source.height));
        }
        self.layer_remove(&old.id);
        self.layer_add(new)
            .map_err(|reason| crate::engine::LayerSwapError {
                reason,
                restored: false,
            })
    }
    fn layer_add(&mut self, layer: &crate::picture::layers::Layer) -> Result<(u32, u32), String> {
        self.refuse.clone().map_or_else(
            || {
                match layer.source.kind {
                    crate::picture::layers::Kind::Screen => self.told.lock().unwrap().push(
                        Behind::Screen(crate::picture::sources::DisplayId(
                            layer.source.handle.parse().unwrap(),
                        )),
                    ),
                    crate::picture::layers::Kind::Window => {
                        self.told.lock().unwrap().push(Behind::Window(
                            crate::picture::sources::WindowId(layer.source.handle.parse().unwrap()),
                        ))
                    }
                    crate::picture::layers::Kind::Camera => self
                        .cameras
                        .lock()
                        .unwrap()
                        .push(Some(layer.source.handle.clone())),
                    crate::picture::layers::Kind::Image => {}
                }
                Ok(match layer.source.kind {
                    crate::picture::layers::Kind::Camera => (1280, 720),
                    crate::picture::layers::Kind::Window => (853, 479),
                    crate::picture::layers::Kind::Screen => (1920, 1080),
                    crate::picture::layers::Kind::Image => (640, 480),
                })
            },
            Err,
        )
    }
    fn shader_active(&self) -> bool {
        self.refuse.as_deref() != Some("runtime:global")
    }
    fn layer_shader_active(&self, id: &str) -> bool {
        self.refuse.as_deref() != Some("runtime:layer") || id != "disabled"
    }
    fn element_shader(&mut self, _element: &Element, path: Option<&str>) -> Result<(), String> {
        if path == Some("bad.wgsl") {
            Err("invalid shader".into())
        } else {
            Ok(())
        }
    }
    fn layer_shader(
        &mut self,
        _layer: &crate::picture::layers::Layer,
        path: Option<&str>,
    ) -> Result<(), String> {
        if path == Some("bad.wgsl") {
            Err("invalid shader".into())
        } else {
            Ok(())
        }
    }
    fn shader(&mut self, path: Option<&str>) -> Result<(), String> {
        if path == Some("bad.wgsl") {
            return Err("invalid shader".into());
        }
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        let _ = path;
        Ok(())
    }
    fn previewing(&mut self, on: bool) {
        *self.previewed.lock().expect("previewed") = Some(on);
    }
    fn layer_shot(&mut self, _id: &str) -> Option<(Vec<u8>, u32, u32)> {
        Some((vec![0xff, 0xd8, 0xff], 480, 270))
    }
    fn shot(&mut self, of: Framed) -> Option<(Vec<u8>, u32, u32)> {
        // Three bytes that are not a JPEG: what this proves is the path,
        // and a fake that produced a real one would be proving libjpeg.
        // The two shapes differ so a caller asking for the wrong one shows
        // up as the wrong size rather than as nothing at all.
        match of {
            Framed::Scene => Some((vec![0xff, 0xd8, 0xff], 480, 270)),
            Framed::Camera => Some((vec![0xff, 0xd8, 0xff], 480, 360)),
            Framed::Screen => Some((vec![0xff, 0xd8, 0xff], 480, 300)),
        }
    }
    fn stop(&mut self) {}
    fn layer_flowing(&self, id: &str) -> Flowing {
        if id.is_empty() {
            return Flowing::default();
        }
        Flowing {
            captured: 3,
            frames: 3,
            width: 1280,
            height: 720,
            held: None,
        }
    }
    fn flowing(&self) -> Flowing {
        // Nothing is flowing until something has been pointed at, which is
        // what a cold engine looks like and what makes "there is no
        // picture to send yet" a thing a test can reach.
        if self.told.lock().expect("told").is_empty() {
            return Flowing::default();
        }
        Flowing {
            captured: 7,
            frames: 42,
            width: 1920,
            height: 1080,
            held: None,
        }
    }
}

impl Sound for Wrote {
    fn audio_layer_add(&mut self, layer: &crate::sound::audio_layers::Layer) -> Result<(), String> {
        let said = layer
            .source
            .name
            .clone()
            .or_else(|| layer.source.device.clone())
            .or_else(|| layer.source.display.map(|d| d.to_string()))
            .unwrap_or_default();
        if self.refuse.as_deref() == Some(&format!("audio:{said}")) {
            return Err(format!("no running application called {said}"));
        }
        self.heard.lock().expect("heard").push(format!(
            "open {} {said}{}",
            layer.id,
            if layer.muted { " muted" } else { "" }
        ));
        Ok(())
    }
    fn audio_layer_rename(&mut self, from: &str, to: &str) {
        self.heard
            .lock()
            .expect("heard")
            .push(format!("rename {from} {to}"));
    }
    fn audio_layer_remove(&mut self, id: &str) {
        self.heard
            .lock()
            .expect("heard")
            .push(format!("close {id}"));
    }
    fn audio_layer_levels(&mut self, id: &str, volume: f64, muted: bool) {
        self.heard.lock().expect("heard").push(format!(
            "level {id} {volume}{}",
            if muted { " muted" } else { "" }
        ));
    }
    fn audio_layer_duck(&mut self, id: &str, ducks: bool) {
        self.ducked
            .lock()
            .expect("ducked")
            .push((id.to_string(), ducks));
    }
    fn app_audio(&mut self, app: Option<&str>) -> Result<Option<String>, String> {
        Ok(app.map(str::to_string))
    }
    fn music_ended(&mut self) -> bool {
        // Exactly once, like the real one: asking twice must not skip a
        // track, which is the bug this shape exists to make impossible.
        std::mem::replace(&mut *self.ran_out.lock().expect("ran out"), false)
    }
    fn mic(&mut self, device: Option<&str>) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        self.mics
            .lock()
            .expect("mics")
            .push(device.map(str::to_string));
        Ok(())
    }
    fn play(&mut self, track: Option<&Track>) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        self.played
            .lock()
            .expect("played")
            .push(track.map(|t| t.title.clone()));
        Ok(())
    }
    fn levels(&mut self, levels: SoundLevels) -> Result<(), String> {
        *self.levels.lock().expect("levels") = Some((
            levels.mic,
            levels.music,
            levels.duck_db,
            levels.muted,
            levels.music_to_stream,
            levels.screen_sound,
        ));
        Ok(())
    }
    fn monitor(&mut self, on: bool) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        self.speaker_calls.lock().expect("speaker calls").push(on);
        *self.heard_here.lock().expect("heard here") = on;
        Ok(())
    }
    fn speakers(&self) -> Option<String> {
        self.heard_here
            .lock()
            .expect("heard here")
            .then(|| "Desk speakers".to_string())
    }
    fn gate(&mut self, params: GateParams) {
        *self.gated.lock().expect("gated") = Some(params);
    }
    fn mixing(&self) -> Mixing {
        Mixing {
            frames: 480,
            level_db: -18.0,
            peak_db: -14.0,
            music_db: -30.0,
            music_peak_db: -26.0,
            music_out_db: -20.0,
            music_out_peak_db: -18.0,
            app_db: -60.0,
            ducked_db: 0.0,
            playing: true,
            monitor_db: -60.0,
        }
    }
    fn hearing(&self) -> Hearing {
        Hearing {
            samples: 4800,
            level_db: -21.0,
            peak_db: -17.0,
            gate_levels: crate::sound::mixer::gate::GateLevels {
                full: 0.05,
                hf: 0.001,
            },
            gate_open: true,
            gain_db: 0.0,
            starved: 0,
            buffered: 0,
            dropped: 0,
            screen_samples: 0,
            screen_complaint: None,
            app_samples: 0,
            app_complaint: None,
            complaint: None,
        }
    }
}

impl Air for Wrote {
    fn publish(&mut self, url: &str) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        self.published
            .lock()
            .expect("published")
            .push(Some(url.to_string()));
        Ok(())
    }
    fn unpublish(&mut self) {
        self.published.lock().expect("published").push(None);
    }
    /// Up while the last thing on the record is a stream that started; a
    /// test ends one from outside by pushing a `None`, the way a relay that
    /// hung up would.
    fn still_publishing(&mut self) -> bool {
        matches!(
            self.published.lock().expect("published").last(),
            Some(Some(_))
        )
    }
    fn record(&mut self, into: &str) -> Result<String, String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        let name = format!("{into}/remux-whenever.mp4");
        self.recorded
            .lock()
            .expect("recorded")
            .push(Some(name.clone()));
        Ok(name)
    }
    fn stop_recording(&mut self) {
        self.recorded.lock().expect("recorded").push(None);
    }
}

impl Pipeline for Wrote {
    fn grants(&self) -> (Grant, Grant, Grant) {
        (Grant::Granted, Grant::Granted, Grant::NotAsked)
    }
}

pub(super) fn publishing_engine(refuse: Option<String>) -> (Engine, Published) {
    let published: Published = Default::default();
    let pipeline = Wrote {
        told: Default::default(),
        cameras: Default::default(),
        mics: Default::default(),
        played: Default::default(),
        levels: Default::default(),
        shown: Default::default(),
        counting: Default::default(),
        published: published.clone(),
        recorded: Default::default(),
        mirrored: Default::default(),
        gated: Default::default(),
        heard_here: Default::default(),
        speaker_calls: Default::default(),
        previewed: Default::default(),
        ran_out: Default::default(),
        refuse,
        scene_events: Default::default(),
        ducked: Default::default(),
        heard: Default::default(),
    };
    (
        Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_destination(Some(DESTINATION.into())),
        published,
    )
}

pub(super) const DESTINATION: &str = "rtmp://hub/scene?user=remux&pass=not-a-real-key";

/// Every card the engine put up, in order, as the fake pipeline saw them.
pub(super) type Shown =
    std::sync::Arc<std::sync::Mutex<Vec<(Vec<Element>, Vec<(String, Duration)>)>>>;

/// Where the faders were last put: microphone, music, duck, muted.
pub(super) type Faders =
    std::sync::Arc<std::sync::Mutex<Option<(f64, f64, f64, bool, bool, bool)>>>;

/// Each `show` the pipeline was given: the elements and the running timers.
pub(super) type Showings = Vec<(Vec<Element>, Vec<(String, Duration)>)>;

pub(super) fn elements_shown(log: &Shown) -> Showings {
    log.lock().expect("shown").clone()
}

/// A music folder: three genres,
/// tracks named the way stream-safe libraries name them.
pub(super) struct ThreeGenres;

impl Library for ThreeGenres {
    fn playlists(&self) -> Vec<Playlist> {
        ["lofi", "edm", "synthwave"]
            .iter()
            .map(|name| Playlist {
                name: (*name).to_string(),
                title: crate::sound::music::genre_title(name),
                tracks: (0..6)
                    .map(|n| Track {
                        title: format!("{name} {n}"),
                        artist: Some("StreamBeats".into()),
                        url: format!("/music/{name}/{n}.mp3"),
                    })
                    .collect(),
            })
            .collect()
    }
}

/// Every file the pipeline was told to play, and a `None` for every stop.
pub(super) type Played = std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>;
/// The switch a test flips to say the track that was playing has run out.
pub(super) type RanOut = std::sync::Arc<std::sync::Mutex<bool>>;

/// An engine with three genres, what it played, and that switch.
pub(super) fn machine_with_music() -> (Engine, Played, RanOut) {
    let played: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>> = Default::default();
    let ran_out: std::sync::Arc<std::sync::Mutex<bool>> = Default::default();
    let pipeline = Wrote {
        told: Default::default(),
        cameras: Default::default(),
        mics: Default::default(),
        played: played.clone(),
        levels: Default::default(),
        shown: Default::default(),
        counting: Default::default(),
        published: Default::default(),
        recorded: Default::default(),
        mirrored: Default::default(),
        gated: Default::default(),
        heard_here: Default::default(),
        speaker_calls: Default::default(),
        previewed: Default::default(),
        ran_out: ran_out.clone(),
        refuse: None,
        scene_events: Default::default(),
        ducked: Default::default(),
        heard: Default::default(),
    };
    (
        Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_library(Box::new(ThreeGenres)),
        played,
        ran_out,
    )
}
