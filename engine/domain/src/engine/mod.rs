//! What the engine decides, with nothing plugged in.
//!
//! Every command a client sends lands here and every reply comes from here,
//! and none of it touches a socket, a display or a device. That is deliberate:
//! it is the difference between a daemon whose behaviour is covered by
//! `cargo test` and one you can only find out about by running it. The
//! transport is [`crate::server`], and it is thin enough to read in one go.

use crate::gate::GateParams;
use crate::music::{fader_db, Playlist, Rotation, Track};
use crate::protocol::{
    Command, Destination, Devices, Flowing, Found, Framed, Grant, Hearing, Mixing, Named, Reply,
    Status,
};
use crate::scenes::{Element, ElementContent};
use crate::sources::{pick, pick_device, DisplayId, Screen, Window, WindowId};
use std::time::{Duration, Instant};

mod air;
mod app;
mod audio_layers;
mod picture;
mod sound;

pub use air::Air;
pub use picture::{LayerSwapError, Picture};
pub use sound::{Sound, SoundLevels};

/// How many ticks a face's "watching" is good for, at four ticks a second.
/// A face renews once a second while it draws, so three seconds is two missed
/// renewals before the preview stops; a face that crashed stops costing the
/// engine after three seconds instead of for ever.
pub const WATCH_LEASE_TICKS: u32 = 12;

/// How many ticks a panel's "present" is good for: five seconds at four ticks
/// a second. A panel says it once a second, so this is four missed renewals,
/// which is a panel that is gone and not one that stuttered. See
/// [`Command::Present`].
pub const PANEL_LEASE_TICKS: u32 = 20;

/// Everything this machine could capture right now, in the domain's own terms.
///
/// Structured rather than pre-formatted on purpose. A window's title and the
/// application that owns it are two facts, and `pick` needs both; turning them
/// into the one line a picker shows is a decision, and decisions live here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Available {
    pub screens: Vec<Screen>,
    pub windows: Vec<Window>,
    pub cameras: Vec<Named>,
    pub mics: Vec<Named>,
    /// The running applications, by name: what `hear` can pick sound from.
    pub apps: Vec<String>,
}

/// What is behind the picture right now.
///
/// One tag, not a screen id beside a window id where both could be set and
/// disagree. There is one picture, and this is what is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Behind {
    /// No physical capture selected. The compositor still produces frames.
    #[default]
    Nothing,
    Screen(DisplayId),
    Window(WindowId),
}

/// The media path: capture now, encode and publish later.
///
/// The engine decides what should be happening and this makes it happen. Same
/// reason as [`Sources`]: everything on the other side of it needs a display
/// and a grant, and everything on this side is a decision that `cargo test`
/// can drive with a fake.
pub trait Pipeline: Picture + Sound + Air + Send {
    /// What macOS is letting this process do, right now rather than when it
    /// started: a grant can be taken away from System Settings while a live is
    /// running, and the first anyone knows is a black picture.
    fn grants(&self) -> (Grant, Grant, Grant);
}

/// A pipeline that captures nothing, for an engine that has not been given
/// one. It is honest rather than a stub: `remuxd --no-capture` would run this.
#[derive(Debug, Default)]
pub struct NoPipeline;

impl Picture for NoPipeline {
    fn layer_replace(
        &mut self,
        _old: &crate::layers::Layer,
        new: &crate::layers::Layer,
    ) -> Result<(u32, u32), picture::LayerSwapError> {
        self.layer_add(new)
            .map_err(|reason| picture::LayerSwapError {
                reason,
                restored: true,
            })
    }
    fn scene_transition(
        &mut self,
        _: &[crate::layers::Layer],
        to: &[crate::layers::Layer],
        elements: &[Element],
        shader: Option<&str>,
    ) -> Result<(), String> {
        if shader.is_some()
            || to.iter().any(|layer| layer.shader.is_some())
            || elements.iter().any(|element| element.shader.is_some())
        {
            Err("this engine has no compositor".into())
        } else {
            Ok(())
        }
    }
    fn layer_add(&mut self, layer: &crate::layers::Layer) -> Result<(u32, u32), String> {
        // No-capture mode accepts a scene layout, but cannot produce frames.
        Ok(match layer.source.kind {
            crate::layers::Kind::Screen => (1920, 1080),
            crate::layers::Kind::Camera => (1280, 720),
            crate::layers::Kind::Window => (853, 479),
        })
    }
    fn show(
        &mut self,
        _elements: &[Element],
        _timers: &[(String, Duration)],
        _order: &[String],
    ) -> Result<(), String> {
        Ok(())
    }
    fn stop(&mut self) {}
    fn flowing(&self) -> Flowing {
        Flowing::default()
    }
    fn mirror(&mut self, _on: bool) {}
    fn shader(&mut self, path: Option<&str>) -> Result<(), String> {
        if path.is_none() {
            Ok(())
        } else {
            Err("this engine has no compositor".into())
        }
    }
    fn shot(&mut self, _of: Framed) -> Option<(Vec<u8>, u32, u32)> {
        None
    }
}

impl Sound for NoPipeline {
    fn mic(&mut self, _device: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    fn app_audio(&mut self, _app: Option<&str>) -> Result<Option<String>, String> {
        Err("this engine has no application audio capture".into())
    }
    fn play(&mut self, _track: Option<&Track>) -> Result<(), String> {
        Ok(())
    }
    fn levels(&mut self, _: SoundLevels) -> Result<(), String> {
        Ok(())
    }
    fn monitor(&mut self, _on: bool) -> Result<(), String> {
        Err("this engine has no sound to hear".into())
    }
    fn speakers(&self) -> Option<String> {
        None
    }
    fn gate(&mut self, _params: GateParams) {}
    fn hearing(&self) -> Hearing {
        Hearing::default()
    }
    fn mixing(&self) -> Mixing {
        Mixing::default()
    }
}

impl Air for NoPipeline {
    fn publish(&mut self, _url: &str) -> Result<(), String> {
        Err("this engine captures nothing, so it has nothing to publish".into())
    }
    fn unpublish(&mut self) {}
    fn record(&mut self, _into: &str) -> Result<String, String> {
        Err("this engine captures nothing, so it has nothing to record".into())
    }
    fn stop_recording(&mut self) {}
}

impl Pipeline for NoPipeline {
    fn grants(&self) -> (Grant, Grant, Grant) {
        // An engine that captures nothing has been allowed nothing, which is
        // true rather than convenient.
        (Grant::Refused, Grant::Refused, Grant::Refused)
    }
}

/// The app, as far as the engine is concerned.
///
/// Everything behind it belongs to the web: what a destination
/// is, whether it is armed, who is watching, what the chat said. The engine
/// holds none of it and repeats all of it, which is why this is a port and not
/// a field: an engine with no app is a real configuration, and it captures,
/// records and publishes perfectly well.
/// Where a live goes: one destination's door, with its key on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outlet {
    pub id: i64,
    pub url: String,
}

pub trait Watching: Send {
    /// Whether the app is reachable right now. Reported beside the lists,
    /// because an empty list from a server that is down and an empty list from
    /// a person with no destinations look identical and are not.
    fn reachable(&self) -> bool;
    /// Which app this is, for a window that says so. `None` when there is no
    /// app at all.
    fn server(&self) -> Option<String> {
        None
    }
    fn destinations(&self) -> Vec<Destination>;
    /// Summed across the destinations that answered, and `None` when none did.
    fn viewers(&self) -> Option<u32>;
    /// Where the app publishes the scene, with this session's own token, for
    /// an engine that was started with no destination of its own. See
    /// the relay's door, when one is kept.
    fn scene(&self) -> Option<String> {
        None
    }
    /// Where Go live sends the picture: through the relay, one door for the
    /// scene; or straight to every armed destination, one door each.
    fn outlets(&self) -> Vec<Outlet> {
        self.scene()
            .map(|url| vec![Outlet { id: 0, url }])
            .unwrap_or_default()
    }
    /// Ask the app to arm or disarm one. It owns the column; this asks.
    fn arm(&self, adapter: i64, on: bool) -> Result<(), String>;
    /// Ask the app to put one in the sandbox, or take it out. Same rule.
    fn sandbox(&self, adapter: i64, on: bool) -> Result<(), String>;
    /// Ask the app to call the live something on one destination. Same rule
    /// as `arm`: the app's columns, and `None` leaves a field alone.
    fn retitle(
        &self,
        adapter: i64,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<(), String>;
    /// Ask the app to tell one destination's platform the title now.
    fn announce(&self, adapter: i64) -> Result<(), String>;
    /// Tell every armed destination's platform, or none, before a frame goes
    /// out; an `Err` is the refusal, and nothing was left told.
    fn announce_armed(&self) -> Result<(), String> {
        Ok(())
    }
    /// Take back what `announce_armed` told: the live did not start after all.
    fn withdraw(&self) {}
    /// The live ended, as it should: what was told stays told.
    fn off_air(&self) {}
    /// Ask the app to forget one destination's platform account.
    fn disconnect(&self, adapter: i64) -> Result<(), String>;
    /// Ask the app to file one destination's live under a category.
    fn categorize(&self, adapter: i64, id: &str, name: &str) -> Result<(), String>;
    /// Ask the app where one destination's live can be filed, for what was
    /// typed; the answer comes back on `found`.
    fn search_categories(&self, adapter: i64, query: &str) -> Result<(), String>;
    /// The last search's answer, if the app has said.
    fn found(&self) -> Option<Found> {
        None
    }
    /// The most watching at once this live, if the app has said.
    fn viewers_peak(&self) -> Option<u32> {
        None
    }
    /// What the app said out loud since the last time anybody asked: a
    /// platform that refused a title, by name, marked `! ` as the log marks
    /// an alarm; what went as asked, unmarked. Drained, so each is read
    /// once; the engine puts them in its log as they are, where every face
    /// reads.
    fn notices(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// An engine with no app behind it. Not a stub: `remuxd` with no credentials
/// in its environment runs exactly this, and it is the ordinary case for
/// somebody recording rather than streaming.
#[derive(Debug, Default)]
pub struct NoApp;

impl Watching for NoApp {
    fn reachable(&self) -> bool {
        false
    }
    fn destinations(&self) -> Vec<Destination> {
        Vec::new()
    }
    fn viewers(&self) -> Option<u32> {
        None
    }
    fn sandbox(&self, _adapter: i64, _on: bool) -> Result<(), String> {
        Err("not signed in: nowhere to ask".into())
    }
    fn arm(&self, _adapter: i64, _on: bool) -> Result<(), String> {
        Err("this engine is not signed in to an app".into())
    }
    fn retitle(&self, _: i64, _: Option<&str>, _: Option<&str>) -> Result<(), String> {
        Err("this engine is not signed in to an app".into())
    }
    fn disconnect(&self, _adapter: i64) -> Result<(), String> {
        Err("not signed in: nowhere to ask".into())
    }
    fn categorize(&self, _adapter: i64, _id: &str, _name: &str) -> Result<(), String> {
        Err("not signed in: nowhere to ask".into())
    }
    fn search_categories(&self, _adapter: i64, _query: &str) -> Result<(), String> {
        Err("not signed in: nowhere to ask".into())
    }
    fn announce(&self, _adapter: i64) -> Result<(), String> {
        Err("this engine is not signed in to an app".into())
    }
}

/// The music on disk, as far as the engine is concerned.
///
/// Its own port and not part of [`Sources`], because a folder of files and a
/// list of cameras fail for entirely different reasons and at entirely
/// different moments: a camera is unplugged mid-live, a music folder is
/// wrong once, at the start, and then never again.
pub trait Library: Send {
    fn playlists(&self) -> Vec<Playlist>;
}

/// No music. What the engine runs as before a folder is configured, and a
/// truthful answer rather than a stub.
#[derive(Debug, Default)]
pub struct NoLibrary;

impl Library for NoLibrary {
    fn playlists(&self) -> Vec<Playlist> {
        Vec::new()
    }
}

/// What can be captured, as far as the engine is concerned.
///
/// The engine never names an Apple framework. It asks through this, so
/// `cargo test` drives it with a fake and the one implementation that talks to
/// ScreenCaptureKit stays in `macos`, where it cannot be tested and does not
/// need to be.
pub trait Sources: Send {
    fn available(&self) -> Result<Available, String>;
    /// A number that moves when a device is plugged in or pulled out. A face
    /// reads the list when it changes and never otherwise: enumerating
    /// cameras wakes them, and a list read only when empty missed the headset
    /// that reconnected after the window opened.
    fn generation(&self) -> u64 {
        0
    }
}

/// The engine with nothing plugged into it. Not a stub for tests: it is what
/// the daemon runs as before anything has been chosen, and answering "nothing
/// yet" is a truthful answer.
#[derive(Debug, Default)]
pub struct NoSources;

impl Sources for NoSources {
    fn available(&self) -> Result<Available, String> {
        Ok(Available::default())
    }
}

/// The one line a picker shows for a window: who owns it, then what it says.
/// Both halves matter, because a browser window is titled after the page.
fn window_label(window: &Window) -> String {
    format!("{} — {}", window.app, window.title)
}

impl From<Available> for Devices {
    fn from(available: Available) -> Self {
        Devices {
            screens: available
                .screens
                .iter()
                .map(|screen| Named {
                    id: screen.id.0.to_string(),
                    name: screen.name.clone(),
                })
                .collect(),
            windows: available
                .windows
                .iter()
                .map(|window| Named {
                    id: window.id.0.to_string(),
                    name: window_label(window),
                })
                .collect(),
            cameras: available.cameras,
            mics: available.mics,
            apps: available
                .apps
                .iter()
                .map(|name| Named {
                    id: name.clone(),
                    name: name.clone(),
                })
                .collect(),
            // Filled by the engine, which is what holds the library: this
            // conversion only knows about the machine.
            genres: Vec::new(),
        }
    }
}

/// Seconds past the epoch.
///
/// The one place this crate reads a clock. It is `std`, not a dependency, and
/// what it feeds is a timestamp on a log line: everything that decides
/// anything takes its time as an argument and stays testable.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

/// The engine's whole state. Small on purpose: anything that grows a decision
/// of its own gets a pure module beside it, the way `gate` and `music` are.
pub struct Engine {
    status: Status,
    /// Running deadlines are transient and scoped to the active scene's element IDs.
    counting: std::collections::HashMap<String, Instant>,
    quitting: bool,
    sources: Box<dyn Sources>,
    pipeline: Box<dyn Pipeline>,
    library: Box<dyn Library>,
    /// The genre being played, and its order. The order is state and not a
    /// setting: it remembers what it has played so it can refuse to repeat,
    /// which is the whole point of it.
    playing: Option<(String, Rotation)>,
    watching: Box<dyn Watching>,
    /// The chat, off whatever wire the daemon opened. Shared with that wire's
    /// thread, which pushes lines in and takes deletes out.
    chat: std::sync::Arc<std::sync::Mutex<crate::chat::Feed>>,
    /// Where the picture goes when somebody presses Go live.
    ///
    /// It carries a credential, so the engine takes it from whoever started it
    /// and never learns it from a client: a stream key is per-user data and a
    /// panel has no business holding one. `None` is an engine that can capture,
    /// mix and record but cannot go live, which is a real configuration and not
    /// a broken one.
    destination: Option<String>,
    /// What has happened, for the window that shows it. See
    /// [`crate::journal`]: the engine narrates itself, so "why is nothing
    /// going out" has an answer that is not a debugger.
    journal: crate::journal::Journal,
    /// How many windows are drawing the preview. See [`Command::Watching`].
    /// Ticks left on the last face's "watching". A lease, never a count: a
    /// count lived in this engine's memory, and an engine restarted under a
    /// live panel had nobody in it and never published a frame again.
    watch_lease: u32,
    /// Ticks left on the last panel's "present", and zero for an engine no
    /// panel has spoken to. See [`Command::Present`]: when this runs out the
    /// engine stops itself, so a panel and its engine leave together.
    panel_lease: u32,
    /// The folder a recording is written into. Its own setting and not derived
    /// from the destination: recording needs no relay and no destination at all,
    /// which is the whole point of it being a second lever.
    recordings: Option<String>,
    /// Where lives are written down when they end; `None` keeps no record.
    history: Option<std::path::PathBuf>,
    /// The live in progress, sampled off the muxer. See [`crate::history`].
    live: Option<crate::history::Sampler>,
    /// Which motor is behind the ports, for the status.
    motor: String,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self::with_sources(Box::new(NoSources))
    }

    pub fn with_sources(sources: Box<dyn Sources>) -> Self {
        Self {
            status: Status::default(),
            counting: Default::default(),
            quitting: false,
            sources,
            watching: Box::new(NoApp),
            chat: Default::default(),
            pipeline: Box::new(NoPipeline),
            library: Box::new(NoLibrary),
            playing: None,
            destination: None,
            journal: crate::journal::Journal::default(),
            watch_lease: 0,
            panel_lease: 0,
            recordings: None,
            history: None,
            live: None,
            motor: "none".into(),
        }
    }

    /// Give the engine a music folder.
    #[must_use]
    pub fn with_library(mut self, library: Box<dyn Library>) -> Self {
        self.library = library;
        self
    }

    /// Give the engine a media path. Separate from the constructor because
    /// most of what the engine decides has nothing to do with one, and a test
    /// that does not care should not have to say so.
    #[must_use]
    /// Where Go live sends the picture. See [`Engine::destination`].
    pub fn with_destination(mut self, destination: Option<String>) -> Self {
        self.destination = destination;
        self
    }

    /// Point a running engine somewhere else. Off air only in practice: the
    /// caller sets this at boot, and changing it mid-live would not move a
    /// stream that is already flowing.
    pub fn set_destination(&mut self, destination: Option<String>) {
        self.destination = destination;
    }

    /// Point a running engine at another folder. See [`Engine::recordings`].
    pub fn set_recordings(&mut self, folder: Option<String>) {
        self.recordings = folder;
    }

    /// Where recordings are written. See [`Engine::recordings`].
    pub fn with_recordings(mut self, folder: Option<String>) -> Self {
        self.recordings = folder;
        self
    }

    /// The motor's name, as the status says it.
    pub fn with_motor(mut self, motor: String) -> Self {
        self.motor = motor;
        self
    }

    /// The file lives are written down in when they end.
    pub fn with_history(mut self, path: Option<std::path::PathBuf>) -> Self {
        self.history = path;
        self
    }

    /// The app this engine repeats. See [`Watching`].
    pub fn with_app(mut self, watching: Box<dyn Watching>) -> Self {
        self.watching = watching;
        self
    }

    /// The chat feed the daemon's wire fills. See [`crate::chat`].
    pub fn with_chat(mut self, chat: std::sync::Arc<std::sync::Mutex<crate::chat::Feed>>) -> Self {
        self.chat = chat;
        self
    }

    pub fn with_pipeline(mut self, pipeline: Box<dyn Pipeline>) -> Self {
        self.pipeline = pipeline;
        self
    }

    /// What is actually coming out of the capture right now, measured.
    pub fn flowing(&self) -> Flowing {
        self.pipeline.flowing()
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// True once a client has asked the engine to quit, so the transport knows
    /// to stop accepting. The engine never exits the process itself: deciding
    /// and dying are different jobs.
    pub fn quitting(&self) -> bool {
        self.quitting
    }

    /// The setup, to write down. See [`crate::remembered`].
    ///
    /// Choices only, never state: nothing here can bring an engine up on air.
    #[must_use]
    pub fn remembered(&self) -> crate::remembered::Remembered {
        let mut layers = self.status.layers.clone();
        for layer in &mut layers {
            if !self.pipeline.layer_shader_active(&layer.id) {
                layer.shader = None;
            }
        }
        let shader = self
            .pipeline
            .shader_active()
            .then(|| self.status.shader.clone())
            .flatten();
        let mut scenes = self.current_scenes();
        if let Some(active) = scenes
            .iter_mut()
            .find(|scene| scene.name == self.status.active_scene)
        {
            active.layers = layers.clone();
            for element in &mut active.elements {
                if !self.pipeline.layer_shader_active(&element.id) {
                    element.shader = None;
                }
            }
            active.normalize_order();
            active.shader = shader.clone();
        }
        crate::remembered::Remembered {
            layers,
            scenes,
            active_scene: self.status.active_scene.clone(),
            audio_layers: self.status.audio_layers.clone(),
            mic: self.status.mic.clone(),
            mirrored: self.status.mirrored,
            genre: self.playing.as_ref().map(|(genre, _)| genre.clone()),
            faders: self.status.faders,
            gate: self.status.gate,
            monitoring: self.status.monitoring,
            denoise: self.status.denoise,
        }
    }

    /// Put a saved setup back, as commands.
    ///
    /// Replayed rather than assigned, and that is the point: the capture has to
    /// actually start, the gate has to actually be retuned, and a camera that
    /// has been unplugged since must fail here the same way it would fail if
    /// somebody chose it now. Whatever is gone is skipped and the rest still
    /// comes back, because losing a webcam is not a reason to lose the gate
    /// settings somebody spent an evening on.
    pub fn restore(&mut self, setup: &crate::remembered::Remembered) {
        self.status.faders = setup.faders;
        let _ = self.handle(Command::Gate {
            patch: serde_json::to_value(setup.gate).unwrap_or_default(),
        });
        let _ = self.sound();
        let mut scenes = if setup.scenes.is_empty() {
            let mut defaults = crate::scenes::defaults();
            defaults[0].layers = setup.layers.clone();
            defaults[0].normalize_order();
            defaults
        } else {
            // A saved collection is authoritative: if the operator deleted
            // the starter scene, do not silently recreate it on restart.
            setup.scenes.clone()
        };
        for scene in &mut scenes {
            scene.normalize_order();
        }
        let active = if scenes.iter().any(|scene| scene.name == setup.active_scene) {
            setup.active_scene.clone()
        } else {
            scenes[0].name.clone()
        };
        let saved_order = scenes
            .iter()
            .find(|s| s.name == active)
            .map(|s| s.ordered_ids())
            .unwrap_or_default();
        let saved_layers = scenes
            .iter()
            .find(|scene| scene.name == active)
            .expect("active scene")
            .layers
            .clone();
        self.status.scenes = scenes;
        self.status.active_scene = active;
        // Replay capture selections against empty slots, not saved IDs that
        // have not been opened yet. The saved stacking order is restored below.
        if let Some(scene) = self
            .status
            .scenes
            .iter_mut()
            .find(|s| s.name == self.status.active_scene)
        {
            scene.layers.clear();
            scene.normalize_order();
        }
        self.render_scene();
        for saved in &saved_layers {
            let reply = match saved.source.kind {
                crate::layers::Kind::Screen => {
                    saved
                        .source
                        .handle
                        .parse()
                        .ok()
                        .map(|display| Command::LayerScreen {
                            id: saved.id.clone(),
                            display,
                        })
                }
                crate::layers::Kind::Camera => Some(Command::LayerCamera {
                    id: saved.id.clone(),
                    device: saved.source.name.clone(),
                }),
                crate::layers::Kind::Window => Some(Command::LayerWindow {
                    id: saved.id.clone(),
                    query: saved.source.name.clone(),
                }),
            };
            if let Some(command) = reply {
                if matches!(self.handle(command), Reply::Status(_)) {
                    let _ = self.handle(Command::LayerTransform {
                        id: saved.id.clone(),
                        transform: saved.transform,
                    });
                    if let Some(crop) = saved.crop {
                        let _ = self.handle(Command::LayerCrop {
                            id: saved.id.clone(),
                            crop: Some(crop),
                        });
                    }
                    if let Some(shape) = saved.shape {
                        let _ = self.handle(Command::LayerShape {
                            id: saved.id.clone(),
                            shape,
                        });
                    }
                    if saved.mirrored {
                        let _ = self.handle(Command::LayerMirror {
                            id: saved.id.clone(),
                            on: true,
                        });
                    }
                    if let Some(path) = &saved.shader {
                        let _ = self.handle(Command::LayerShader {
                            id: saved.id.clone(),
                            path: Some(path.clone()),
                        });
                    }
                    if !saved.visible {
                        let _ = self.handle(Command::LayerVisible {
                            id: saved.id.clone(),
                            on: false,
                        });
                    }
                }
            }
        }
        if let Some(scene) = self
            .status
            .scenes
            .iter_mut()
            .find(|s| s.name == self.status.active_scene)
        {
            scene.order = saved_order;
            scene.normalize_order();
        }
        self.render_scene();
        let saved_element_shaders = self
            .status
            .scenes
            .iter()
            .find(|s| s.name == self.status.active_scene)
            .map(|s| s.elements.clone())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| e.shader.map(|path| (e.id, path)))
            .collect::<Vec<_>>();
        if let Some(scene) = self
            .status
            .scenes
            .iter_mut()
            .find(|s| s.name == self.status.active_scene)
        {
            for element in &mut scene.elements {
                element.shader = None;
            }
        }
        for (id, path) in saved_element_shaders {
            let _ = self.handle(Command::LayerShader {
                id,
                path: Some(path),
            });
        }
        let selected_shader = self
            .status
            .scenes
            .iter()
            .find(|scene| scene.name == self.status.active_scene)
            .and_then(|scene| scene.shader.clone());
        if let Some(path) = selected_shader {
            let _ = self.handle(Command::Shader { path: Some(path) });
        }
        for saved in &setup.audio_layers {
            if matches!(
                self.handle(Command::AudioLayerAdd {
                    id: saved.id.clone(),
                    source: saved.source.clone(),
                }),
                Reply::Status(_)
            ) {
                let _ = self.handle(Command::AudioLayerVolume {
                    id: saved.id.clone(),
                    volume: saved.volume,
                });
                let _ = self.handle(Command::AudioLayerMute {
                    id: saved.id.clone(),
                    on: saved.muted,
                });
            }
        }
        if let Some(mic) = &setup.mic {
            let _ = self.handle(Command::Mic {
                device: Some(mic.clone()),
            });
        }
        if setup.mirrored {
            let _ = self.handle(Command::Mirror { on: true });
        }
        if setup.monitoring {
            let _ = self.handle(Command::Monitor { on: true });
        }
        if setup.denoise {
            let _ = self.handle(Command::Denoise { on: true });
        }
        // The music last, and only the shelf: the genre is remembered so
        // `music on` resumes it, and nothing plays until somebody says so.
        // It used to replay `Genre`, which plays and opens the speakers, and
        // every engine that started (a smoke's, a restart's) was music in a
        // room with nobody in it.
        if let Some(genre) = &setup.genre {
            self.shelve(genre);
        }
    }

    fn current_scenes(&self) -> Vec<crate::scenes::Scene> {
        let mut scenes = self.status.scenes.clone();
        if let Some(active) = scenes
            .iter_mut()
            .find(|scene| scene.name == self.status.active_scene)
        {
            active.layers = self.status.layers.clone();
            active.normalize_order();
            active.shader = self.status.shader.clone();
        }
        scenes
    }

    fn scene_create(&mut self, name: String) -> Reply {
        if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
            return Reply::Error {
                message: "scene name must be 1–80 printable characters".into(),
            };
        }
        if self.status.scenes.iter().any(|scene| scene.name == name) {
            return Reply::Error {
                message: format!("scene {name:?} already exists"),
            };
        }
        let saved = self.remembered();
        self.status.shader = saved
            .scenes
            .iter()
            .find(|scene| scene.name == self.status.active_scene)
            .and_then(|scene| scene.shader.clone());
        self.status.scenes = saved.scenes;
        self.status.layers = saved.layers;
        self.status.scenes.push(crate::scenes::Scene {
            name: name.clone(),
            layers: self.status.layers.clone(),
            elements: self
                .status
                .scenes
                .iter()
                .find(|s| s.name == self.status.active_scene)
                .map(|s| s.elements.clone())
                .unwrap_or_default(),
            order: self
                .status
                .scenes
                .iter()
                .find(|s| s.name == self.status.active_scene)
                .map(|s| s.ordered_ids())
                .unwrap_or_default(),
            shader: self.status.shader.clone(),
        });
        self.status.active_scene = name;
        self.counting.clear();
        self.render_scene();
        Reply::Status(Box::new(self.reported()))
    }

    fn scene_delete(&mut self, name: String) -> Reply {
        if name == self.status.active_scene {
            return Reply::Error {
                message: "cannot delete the active scene".into(),
            };
        }
        let Some(index) = self
            .status
            .scenes
            .iter()
            .position(|scene| scene.name == name)
        else {
            return Reply::Error {
                message: format!("no scene {name:?}"),
            };
        };
        self.status.scenes.remove(index);
        Reply::Status(Box::new(self.reported()))
    }

    fn scene_switch(&mut self, name: String) -> Reply {
        if name == self.status.active_scene {
            return Reply::Status(Box::new(self.reported()));
        }
        let Some(target) = self.status.scenes.iter().find(|scene| scene.name == name) else {
            return Reply::Error {
                message: format!("no scene {name:?}"),
            };
        };
        let previous = self.status.clone();
        let previous_clock = self.counting.clone();
        let next = target.layers.clone();
        let next_shader = target.shader.clone();
        let next_elements = target.elements.clone();
        let next_order = target.ordered_ids();
        // Snapshot the outgoing scene *before* the pipeline swaps its active
        // flags to the incoming programs (a runtime-disabled shader stays off).
        let previous_scenes = self.remembered().scenes;
        if !next_elements.is_empty() {
            if let Err(message) = self.pipeline.show(&next_elements, &[], &next_order) {
                return Reply::Error { message };
            }
        }
        if let Err(message) = self.pipeline.scene_transition(
            &self.status.layers,
            &next,
            &next_elements,
            next_shader.as_deref(),
        ) {
            self.render_scene();
            return Reply::Error { message };
        }
        // Preserve screen sound only when its physical display remains in the new scene.
        let sound_source = self.status.screen_sound_layer.as_ref().and_then(|id| {
            self.status
                .layers
                .iter()
                .find(|layer| &layer.id == id)
                .map(crate::scenes::CaptureKey::of)
        });
        let new_sound = sound_source.and_then(|key| {
            next.iter()
                .find(|layer| {
                    layer.source.kind == crate::layers::Kind::Screen
                        && crate::scenes::CaptureKey::of(layer) == key
                })
                .map(|layer| layer.id.clone())
        });
        self.status.scenes = previous_scenes;
        self.status.layers = next;
        self.status.shader = next_shader;
        self.status.active_scene = name;
        self.counting.clear();
        self.pipeline.layers_changed(&self.status.layers);
        if self.status.screen_sound_layer != new_sound {
            if self.pipeline.screen_audio(new_sound.as_deref()).is_err() {
                let _ = self.pipeline.screen_audio(None);
                self.status.screen_sound = false;
                self.status.screen_sound_layer = None;
            } else {
                self.status.screen_sound_layer = new_sound;
                if self.status.screen_sound_layer.is_none() {
                    self.status.screen_sound = false;
                }
            }
        }
        // A scene can hide/show the same sound-supplying display without
        // changing its layer ID; update the mixer gate in that case too.
        let _ = self.sound();
        let drawn = self.pipeline.show(&next_elements, &[], &next_order);
        if let Err(message) = drawn {
            // A renderer refusal cannot silently commit a scene whose picture
            // still shows the outgoing scene. The macOS renderer never refuses
            // this choice, but a failing pipeline must have an honest status.
            let _ = self.pipeline.scene_transition(
                &self.status.layers,
                &previous.layers,
                &previous
                    .scenes
                    .iter()
                    .find(|scene| scene.name == previous.active_scene)
                    .map(|scene| scene.elements.clone())
                    .unwrap_or_default(),
                previous.shader.as_deref(),
            );
            self.status = previous;
            self.counting = previous_clock;
            self.pipeline.layers_changed(&self.status.layers);
            let _ = self
                .pipeline
                .screen_audio(self.status.screen_sound_layer.as_deref());
            self.render_scene();
            return Reply::Error { message };
        }
        Reply::Status(Box::new(self.reported()))
    }

    /// A moment passing, with nobody asking for anything.
    ///
    /// The engine is otherwise driven entirely by commands, which is the right
    /// shape for everything a person does. One thing is not a person: a track
    /// ending. Without this the music plays one file and then goes quiet with
    /// the old name still on the panel, which is exactly the failure a bed is
    /// there to prevent. So the transport calls this on a timer, and the only
    /// thing it does is start the next track.
    ///
    /// It answers nothing. Whatever it changed is on the next status, which
    /// every face is already reading.
    pub fn tick(&mut self) {
        // The watching lease, counted down here because this is the one
        // thing that runs without a face asking. See `Command::Watching`.
        if self.watch_lease > 0 {
            self.watch_lease -= 1;
            if self.watch_lease == 0 {
                self.pipeline.previewing(false);
            }
        }
        // The panel's lease. Running out is the panel having gone, by
        // whatever door, and the engine leaves with it: a daemon nobody is
        // looking at holding a camera with its light on is the one thing a
        // person notices about a process that outlived its window.
        if self.panel_lease > 0 {
            self.panel_lease -= 1;
            if self.panel_lease == 0 {
                self.quitting = true;
                self.journal.note(now(), "the panel went away; stopping");
            }
        }
        // What the app said out loud, into the log: a platform that refused
        // a title names itself here, where the sheet and the info window read.
        for notice in self.watching.notices() {
            self.journal.note(now(), notice);
        }
        self.air_lapsed();
        self.sample_the_live();
        if !self.pipeline.music_ended() {
            return;
        }
        // Only if music is still meant to be playing. `playing` is the genre
        // and its rotation and survives the switch on purpose, so that turning
        // music back on resumes the same genre; what says whether anything
        // should be sounding is the track name, which the switch clears. A
        // track that ran out after somebody pressed stop is a track that ran
        // out, not a reason to start another one.
        if self.status.music.is_none() {
            return;
        }
        let reply = self.advance();
        if let Some(line) = crate::journal::said(&Command::NextTrack, &reply) {
            self.journal.note(now(), line);
        }
    }

    /// Run a command and note what it did.
    ///
    /// The journal is written here rather than at each decision because there
    /// is exactly one funnel and a line written in forty places is a line
    /// missing from the forty-first. What is worth saying is
    /// [`crate::journal::said`], which is pure.
    pub fn handle(&mut self, command: Command) -> Reply {
        let reply = self.decide(command.clone());
        if let Some(line) = crate::journal::said(&command, &reply) {
            self.journal.note(now(), line);
        }
        reply
    }

    /// One verb, one hand: every arm is a call into the context that owns the
    /// verb (`picture`, `sound`, `air`, `app`), and nothing is decided here.
    fn decide(&mut self, command: Command) -> Reply {
        match command {
            Command::Status => Reply::Status(Box::new(self.reported())),
            Command::SceneCreate { name } => self.scene_create(name),
            Command::SceneSwitch { name } => self.scene_switch(name),
            Command::SceneDelete { name } => self.scene_delete(name),
            Command::AudioLayerAdd { id, source } => self.audio_layer_add(id, source),
            Command::AudioLayerRemove { id } => self.audio_layer_remove(id),
            Command::AudioLayerVolume { id, volume } => self.audio_layer_volume(id, volume),
            Command::AudioLayerMute { id, on } => self.audio_layer_mute(id, on),
            Command::Watching { on } => self.watch(on),
            Command::Present => self.present(),
            Command::Levels => self.levels(),
            Command::Devices => self.devices(),
            Command::Quit => self.quit(),
            // the daemon acts on it beside the engine; here it is a fact
            Command::Rewire => Reply::Ok,
            Command::GoLive => self.go_live(),
            Command::Plan => Reply::Plan(crate::plan::Plan::of(&self.reported())),
            Command::Live { plan } => {
                if crate::plan::Plan::of(&self.reported()).fingerprint != plan {
                    Reply::Error {
                        message: "the plan changed since it was printed; run `remux plan` again"
                            .into(),
                    }
                } else {
                    self.go_live()
                }
            }
            Command::Stop => self.stop_live(),
            Command::RecordStart => self.record_start(),
            Command::RecordStop => self.record_stop(),
            Command::Mute { on } => self.mute(on),
            Command::Volume { level } => self.volume(level),
            Command::MusicVolume { level } => self.music_volume(level),
            Command::Duck { db } => self.duck(db),
            Command::Monitor { on } => self.monitor(on),
            Command::StreamMusic { on } => self.stream_music(on),
            Command::ScreenSound { on } => self.screen_sound(on),
            Command::LayerScreenSound { id, on } => self.layer_screen_sound(id, on),
            Command::AppAudio { app } => self.app_audio(app),
            Command::AppAudioVolume { level } => self.app_audio_volume(level),
            Command::Screen { display } => self.choose_screen(display),
            Command::Window { query } => self.choose_window(query),
            Command::Camera { device } => self.choose_camera(device),
            Command::CameraPosition { at } => self.camera_position(at),
            Command::CameraShape { shape } => self.camera_shape(shape),
            Command::LayerCamera { id, device } => self.layer_camera(id, device),
            Command::LayerScreen { id, display } => self.layer_screen(id, display),
            Command::LayerWindow { id, query } => self.layer_window(id, query),
            Command::LayerReplaceScreen { id, display } => self.layer_replace_screen(id, display),
            Command::LayerReplaceWindow { id, query } => self.layer_replace_window(id, query),
            Command::LayerReplaceCamera { id, device } => self.layer_replace_camera(id, device),
            Command::LayerVisible { id, on } => self.layer_visible(id, on),
            Command::LayerRemove { id } => self.layer_remove(id),
            Command::LayerMove { id, index } => self.layer_move(id, index),
            Command::LayerTransform { id, transform } => self.layer_transform(id, transform),
            Command::LayerCrop { id, crop } => self.layer_crop(id, crop),
            Command::LayerShape { id, shape } => self.layer_shape(id, shape),
            Command::LayerMirror { id, on } => self.layer_mirror(id, on),
            Command::LayerPosition { id, at } => self.layer_position(id, at),
            Command::Shader { path } => self.shader(path),
            Command::LayerShader { id, path } => self.layer_shader(id, path),
            Command::Mic { device } => self.choose_mic(device),
            Command::Share { on } => self.share(on),
            Command::Genre { name } => self.pick_genre(&name),
            Command::NextTrack => self.next_track(),
            Command::Music { on } => self.music(on),
            Command::SceneElementAdd { element } => self.scene_element_add(element),
            Command::SceneElementSet { element } => self.scene_element_set(element),
            Command::SceneElementRemove { id } => self.scene_element_remove(id),
            Command::SceneTimerStart { id } => self.scene_timer(id, true),
            Command::SceneTimerStop { id } => self.scene_timer(id, false),
            Command::HideEverything => self.hide_everything(),
            Command::Gate { patch } => self.gate(patch),
            Command::Denoise { on } => self.denoise(on),
            Command::Hear { apps } => self.hear(apps),
            Command::Clip { name } => self.play_clip(&name),
            Command::Shot { of } => self.shot(of),
            Command::LayerShot { id } => self.layer_shot(id),
            Command::Arm { adapter, on } => self.arm(adapter, on),
            Command::Sandbox { adapter, on } => self.sandbox(adapter, on),
            Command::Retitle {
                adapter,
                title,
                description,
            } => self.retitle(adapter, title, description),
            Command::Announce { adapter } => self.announce(adapter),
            Command::Disconnect { adapter } => self.disconnect(adapter),
            Command::Chat { since, .. } => self.chat(since),
            Command::Hide { seq } => self.hide(seq),
            Command::Delete { seq } => self.delete_chat(seq),
            Command::Categorize { adapter, id, name } => self.categorize(adapter, &id, &name),
            Command::Categories { adapter, query } => self.search_categories(adapter, &query),
            Command::Grants => self.grants(),
            Command::Mirror { on } => self.mirror(on),
        }
    }

    /// The status as a client should see it: the flags the engine holds, plus
    /// what the capture is actually doing right now. Read rather than cached,
    /// because a cached frame count is exactly the lie this field exists to
    /// stop telling.
    fn reported(&self) -> Status {
        let mut status = self.status.clone();
        status.version = env!("CARGO_PKG_VERSION").into();
        status.motor = self.motor.clone();
        status.scenes = self.current_scenes();
        status.scene_flowing = self.pipeline.flowing();
        status.layer_flowing = status
            .layers
            .iter()
            .map(|layer| (layer.id.clone(), self.pipeline.layer_flowing(&layer.id)))
            .collect();
        if !self.pipeline.shader_active() {
            status.shader = None;
        }
        for layer in &mut status.layers {
            if !self.pipeline.layer_shader_active(&layer.id) {
                layer.shader = None;
            }
        }
        if let Some(active) = status
            .scenes
            .iter_mut()
            .find(|scene| scene.name == status.active_scene)
        {
            active.layers = status.layers.clone();
            for element in &mut active.elements {
                if !self.pipeline.layer_shader_active(&element.id) {
                    element.shader = None;
                }
            }
            active.shader = status.shader.clone();
        }
        status.hearing = self.pipeline.hearing();
        status.mixing = self.pipeline.mixing();
        status.speakers = self.pipeline.speakers();
        // Read from the app rather than kept, every time. The engine is not a
        // second home for the web's columns, and a cached list of
        // destinations is a list that is wrong the moment somebody arms one
        // from another face.
        status.destinations = self.watching.destinations();
        // What each door is doing, from the pipeline that holds it: on air
        // through it, or what its ffmpeg last said.
        let sending = self.pipeline.publishing();
        let troubles = self.pipeline.troubles();
        for row in &mut status.destinations {
            if sending.contains(&row.id) {
                row.status = "live".into();
            }
            if let Some((_, why)) = troubles.iter().find(|(id, _)| *id == row.id) {
                row.trouble = Some(why.clone());
            }
        }
        status.categories = self.watching.found();
        status.viewers_peak = self.watching.viewers_peak();
        status.app = self.watching.reachable();
        status.viewers = self.watching.viewers();
        status.outgoing = self.pipeline.outgoing();
        status.preview = self.pipeline.preview();
        status.record_dir = self.recordings.clone();
        status.devices_generation = self.sources.generation();
        status.server = self.watching.server();
        status.log = self.journal.lines();
        status
    }

    /// Everything off.
    ///
    /// Not six commands in a row from a client: it is one button because it is
    /// pressed in the moment somebody walks into the room, and six round trips
    /// is six chances for one of them to be the one that does not arrive.
    /// Muting used to be missing, which is the worst of both: the picture
    /// hidden and the microphone still open.
    ///
    /// **The music used to keep playing**, on the grounds that a live which
    /// goes silent looks broken while one with a bed under it looks like a
    /// break. That is a fine argument for a card and a bad one for this: half a
    /// guarantee is not one, and somebody pressing this is not thinking about
    /// how the stream reads.
    fn hide_everything(&mut self) -> Reply {
        self.counting.clear();

        // An arbitrary shader may obscure even the emergency scene. The panic
        // button must restore an unfiltered picture before it takes sources off.
        if let Err(why) = self.pipeline.shader(None) {
            return Reply::Error { message: why };
        }
        self.status.shader = None;

        for layer in &self.status.layers {
            self.pipeline.layer_remove(&layer.id);
        }
        self.status.layers.clear();
        if let Some(scene) = self
            .status
            .scenes
            .iter_mut()
            .find(|s| s.name == self.status.active_scene)
        {
            scene.layers.clear();
            scene.elements.clear();
            scene.order.clear();
            scene.shader = None;
        }
        self.render_scene();
        for layer in &self.status.audio_layers {
            self.pipeline.audio_layer_remove(&layer.id);
        }
        self.status.audio_layers.clear();
        self.pipeline.layers_changed(&[]);

        // Everything off, and that includes the sound. The music used to keep
        // playing here on the grounds that a live which goes silent looks
        // broken while one with a bed under it looks like a break. That is a
        // fine argument for a card and a bad one for a panic button: this is
        // pressed when somebody walks into the room, and what it has to
        // guarantee is that nothing of yours reaches anybody. Half a guarantee
        // is not one.
        if let Err(why) = self.pipeline.play(None) {
            return Reply::Error { message: why };
        }
        self.status.music = None;

        // The speakers too. Monitoring left on is the operator's own room
        // playing to them while they deal with whatever made them press this.
        if let Err(why) = self.pipeline.monitor(false) {
            return Reply::Error { message: why };
        }
        self.status.monitoring = false;

        // The microphone closed, not only muted. Muted is a gain of zero on
        // an open device, which is a promise made in software; a device that
        // is not open cannot hear anything. Both, because the mute is what
        // the panel shows and the close is what the guarantee rests on.
        if let Err(why) = self.pipeline.mic(None) {
            return Reply::Error { message: why };
        }
        self.status.mic = None;
        self.status.muted = true;
        // The bed off the stream too, so music turned back on afterwards
        // plays to the room and not to the audience until somebody says so.
        self.status.music_to_stream = false;
        // And the screen's sound: the button means nothing of this room
        // reaches the audience until somebody says so again.
        self.status.screen_sound = false;
        self.status.screen_sound_layer = None;
        if let Err(why) = self.pipeline.screen_audio(None) {
            return Reply::Error { message: why };
        }
        if let Err(why) = self.pipeline.app_audio(None) {
            return Reply::Error { message: why };
        }
        self.status.app_audio = None;
        // Muting is part of the button and has to reach the sound, not only
        // the status. This was missing once and it is the worst half to miss.
        self.sound()
    }

    pub(super) fn devices(&mut self) -> Reply {
        match self.sources.available() {
            Ok(available) => {
                let mut devices: Devices = available.into();
                devices.genres = self
                    .library
                    .playlists()
                    .into_iter()
                    .map(|playlist| Named {
                        id: playlist.name,
                        name: playlist.title,
                    })
                    .collect();
                Reply::Devices(devices)
            }
            // The overwhelmingly likely reason is the screen recording
            // grant, which macOS ties to a code signature, so it comes
            // back as a sentence a person can act on rather than an empty
            // list that looks like "you have no monitors".
            Err(why) => Reply::Error { message: why },
        }
    }

    /// A panel is here. Renews the lease that stops the engine when the
    /// panel goes; see [`Command::Present`].
    pub(super) fn present(&mut self) -> Reply {
        self.panel_lease = PANEL_LEASE_TICKS;
        Reply::Ok
    }

    pub(super) fn quit(&mut self) -> Reply {
        self.quitting = true;
        Reply::Ok
    }

    pub(super) fn grants(&mut self) -> Reply {
        let (screen, camera, microphone) = self.pipeline.grants();
        Reply::Grants {
            screen,
            camera,
            microphone,
        }
    }

    /// Where the microphone fader is, in dB, for a panel that speaks in dB
    /// rather than in percentages that mean nothing.
    pub fn mic_db(&self) -> f64 {
        fader_db(self.status.faders.mic)
    }

    pub fn music_db(&self) -> f64 {
        fader_db(self.status.faders.music)
    }

    pub fn duck_db(&self) -> f64 {
        self.status.faders.duck_db
    }

    pub fn monitoring(&self) -> bool {
        self.status.monitoring
    }
}

/// A human list: "a, b and c". A machine would take the array; a person about
/// to go live is reading a sentence.
fn list_of(names: impl Iterator<Item = String>) -> String {
    let names: Vec<String> = names.collect();
    match names.split_last() {
        None => "nothing".into(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests;
