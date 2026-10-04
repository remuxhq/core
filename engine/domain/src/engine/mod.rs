//! What the engine decides, with nothing plugged in.
//!
//! Every command a client sends lands here and every reply comes from here,
//! and none of it touches a socket, a display or a device. That is deliberate:
//! it is the difference between a daemon whose behaviour is covered by
//! `cargo test` and one you can only find out about by running it. The
//! transport is [`crate::server`], and it is thin enough to read in one go.

use crate::picture::scenes::{Element, ElementContent, Scene};
use crate::picture::sources::{pick, pick_device, DisplayId, Screen, Window, WindowId};
use crate::protocol::{
    Command, Destination, Devices, Flowing, Found, Framed, Grant, Hearing, Mixing, Named, Reply,
    Status,
};
use crate::sound::mixer::gate::GateParams;
use crate::sound::music::{fader_db, Playlist, Rotation, Track};
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
        _old: &crate::picture::layers::Layer,
        new: &crate::picture::layers::Layer,
    ) -> Result<(u32, u32), picture::LayerSwapError> {
        self.layer_add(new)
            .map_err(|reason| picture::LayerSwapError {
                reason,
                restored: true,
            })
    }
    fn scene_transition(
        &mut self,
        _: &[crate::picture::layers::Layer],
        to: &[crate::picture::layers::Layer],
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
    fn layer_add(&mut self, layer: &crate::picture::layers::Layer) -> Result<(u32, u32), String> {
        // No-capture mode accepts a scene layout, but cannot produce frames.
        Ok(match layer.source.kind {
            crate::picture::layers::Kind::Screen => (1920, 1080),
            crate::picture::layers::Kind::Camera => (1280, 720),
            crate::picture::layers::Kind::Window => (853, 479),
            crate::picture::layers::Kind::Image => (640, 480),
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
    /// The microphones alone. A motor that can list them without listing
    /// cameras says so: listing a camera opens it, and the one on the air
    /// stutters.
    fn mics(&self) -> Result<Vec<Named>, String> {
        self.available().map(|available| available.mics)
    }
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
/// Whether a command only reads. A read cannot move what the events follow,
/// so no snapshot is taken after it: a panel's meter asks twelve times a
/// second. One misfiled here costs a quarter of a second, not an event: the
/// tick compares with what was seen last.
fn only_reads(command: &Command) -> bool {
    matches!(
        command,
        Command::Status
            | Command::Levels
            | Command::Sources
            | Command::Plan
            | Command::Shot { .. }
            | Command::LayerShot { .. }
            | Command::Grants
            | Command::Chat { .. }
            | Command::Events { .. }
    )
}

/// Whether a command changes a destination's row: armed, rehearsed,
/// retitled, recategorized. With an account the row changes when the server
/// says so, on the wire; with none, here, in the file.
fn moves_a_row(command: &Command) -> bool {
    matches!(
        command,
        Command::Arm { .. }
            | Command::Sandbox { .. }
            | Command::Retitle { .. }
            | Command::Categorize { .. }
    )
}

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
    chat: std::sync::Arc<std::sync::Mutex<crate::app::chat::Feed>>,
    /// What changed, for every face that follows it. Shared with the
    /// daemon's socket, which hands it out without taking the engine's lock:
    /// a face that reads slowly must never hold up a command.
    events: std::sync::Arc<std::sync::Mutex<crate::app::events::Events>>,
    /// What the events saw last. `None` until the engine first does
    /// anything, since a motor and an app are given after construction.
    seen: Option<crate::app::events::Snapshot>,
    /// The cameras shown, watched for a count that stops.
    stalls: crate::picture::stall::Stalls,
    /// Where the picture goes when somebody presses Go live.
    ///
    /// It carries a credential, so the engine takes it from whoever started it
    /// and never learns it from a client: a stream key is per-user data and a
    /// panel has no business holding one. `None` is an engine that can capture,
    /// mix and record but cannot go live, which is a real configuration and not
    /// a broken one.
    destination: Option<String>,
    /// What has happened, for the window that shows it. See
    /// [`crate::air::journal`]: the engine narrates itself, so "why is nothing
    /// going out" has an answer that is not a debugger.
    journal: crate::air::journal::Journal,
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
    /// The live in progress, sampled off the muxer. See [`crate::air::history`].
    live: Option<crate::air::history::Sampler>,
    /// Which motor is behind the ports, for the status.
    motor: String,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// How many deleted scenes the trash keeps.
const TRASH: usize = 10;

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
            events: Default::default(),
            seen: None,
            stalls: Default::default(),
            pipeline: Box::new(NoPipeline),
            library: Box::new(NoLibrary),
            playing: None,
            destination: None,
            journal: crate::air::journal::Journal::default(),
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

    /// The chat feed the daemon's wire fills. See [`crate::app::chat`].
    pub fn with_chat(
        mut self,
        chat: std::sync::Arc<std::sync::Mutex<crate::app::chat::Feed>>,
    ) -> Self {
        self.chat = chat;
        self
    }

    /// The events the daemon's socket hands out. See [`crate::app::events`].
    pub fn with_events(
        mut self,
        events: std::sync::Arc<std::sync::Mutex<crate::app::events::Events>>,
    ) -> Self {
        self.events = events;
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
            trash: self.status.trash.clone(),
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
        self.status.trash = setup.trash.clone();
        let _ = self.handle(Command::Gate {
            patch: serde_json::to_value(setup.gate).unwrap_or_default(),
        });
        let _ = self.sound();
        let mut scenes = if setup.scenes.is_empty() {
            let mut defaults = crate::picture::scenes::defaults();
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
                crate::picture::layers::Kind::Screen => {
                    self.display_now(&saved.source)
                        .map(|display| Command::LayerScreen {
                            id: saved.id.clone(),
                            display,
                        })
                }
                crate::picture::layers::Kind::Camera => Some(Command::LayerCamera {
                    id: saved.id.clone(),
                    device: saved.source.name.clone(),
                }),
                crate::picture::layers::Kind::Window => Some(Command::LayerWindow {
                    id: saved.id.clone(),
                    query: saved.source.name.clone(),
                }),
                crate::picture::layers::Kind::Image => Some(Command::LayerImage {
                    id: saved.id.clone(),
                    path: saved.source.handle.clone(),
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
                if !saved.duck.by_kind() {
                    let _ = self.handle(Command::AudioLayerDuck {
                        id: saved.id.clone(),
                        duck: saved.duck,
                    });
                }
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

    /// The number a saved display has now: found by the monitor's own
    /// identity when the layer kept one, and `None` when that monitor is not
    /// connected, rather than whichever display took its number since. A layer
    /// saved before identities, or on a platform without them, keeps its number.
    fn display_now(&self, source: &crate::picture::layers::Source) -> Option<u32> {
        let Some(stable) = &source.stable else {
            return source.handle.parse().ok();
        };
        let found = self
            .sources
            .available()
            .ok()?
            .screens
            .into_iter()
            .find(|screen| screen.stable.as_ref() == Some(stable))
            .map(|screen| screen.id.0);
        if found.is_none() {
            crate::log::note(&format!(
                "{} is not connected: its layer is not put back",
                source.name
            ));
        }
        found
    }

    /// The displays of these layers under their numbers of now, so what the
    /// status says a face can type again.
    fn renumber(&self, layers: &mut [crate::picture::layers::Layer]) {
        for layer in layers
            .iter_mut()
            .filter(|l| l.source.kind == crate::picture::layers::Kind::Screen)
        {
            if let Some(display) = self.display_now(&layer.source) {
                layer.source.handle = display.to_string();
            }
        }
    }

    fn current_scenes(&self) -> Vec<crate::picture::scenes::Scene> {
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

    /// Whether a new scene may be called this.
    fn a_new_scene_name(&self, name: &str) -> Result<(), String> {
        if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
            return Err("scene name must be 1–80 printable characters".into());
        }
        if self.status.scenes.iter().any(|scene| scene.name == name) {
            return Err(format!("scene {name:?} already exists"));
        }
        Ok(())
    }

    /// An empty scene, switched to so its layers can be added: the layer verbs
    /// edit the active scene. The switch is the ordinary one, so the captures
    /// the empty scene does not use are closed, and one that refuses leaves
    /// no scene behind.
    fn scene_create(&mut self, name: String) -> Reply {
        if let Err(message) = self.a_new_scene_name(&name) {
            return Reply::Error { message };
        }
        self.status.scenes.push(crate::picture::scenes::Scene {
            name: name.clone(),
            layers: vec![],
            elements: vec![],
            order: vec![],
            shader: None,
        });
        let switched = self.scene_switch(name.clone());
        if matches!(switched, Reply::Error { .. }) {
            self.status.scenes.retain(|scene| scene.name != name);
        }
        switched
    }

    /// The active scene under another name, made active: its captures are the
    /// ones running, so nothing on the air moves.
    fn scene_duplicate(&mut self, name: String) -> Reply {
        if let Err(message) = self.a_new_scene_name(&name) {
            return Reply::Error { message };
        }
        let saved = self.remembered();
        self.status.shader = saved
            .scenes
            .iter()
            .find(|scene| scene.name == self.status.active_scene)
            .and_then(|scene| scene.shader.clone());
        self.status.scenes = saved.scenes;
        self.status.layers = saved.layers;
        self.status.scenes.push(crate::picture::scenes::Scene {
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
        let gone = self.current_scenes().remove(index);
        self.status.scenes.remove(index);
        self.status.trash.insert(0, gone);
        self.status.trash.truncate(TRASH);
        if self.status.staged.as_deref() == Some(name.as_str()) {
            self.unstage();
        }
        Reply::Status(Box::new(self.reported()))
    }

    /// Draws the scene off the air. The scene on the air stages nothing: it is
    /// already what is seen.
    fn scene_stage(&mut self, name: String) -> Reply {
        if name == self.status.active_scene {
            self.unstage();
            return Reply::Status(Box::new(self.reported()));
        }
        let Some(scene) = self
            .current_scenes()
            .into_iter()
            .find(|scene| scene.name == name)
        else {
            return Reply::Error {
                message: format!("no scene {name:?}"),
            };
        };
        if let Err(message) = self.pipeline.stage(Some(&scene)) {
            return Reply::Error { message };
        }
        self.status.staged = Some(name);
        Reply::Status(Box::new(self.reported()))
    }

    /// The staged scene on the air by the ordinary switch, and the one that was
    /// out staged in its place.
    fn scene_take(&mut self) -> Reply {
        let Some(next) = self.status.staged.clone() else {
            return Reply::Error {
                message: "nothing staged: scene stage <name> first".into(),
            };
        };
        let out = self.status.active_scene.clone();
        let switched = self.scene_switch(next);
        if matches!(switched, Reply::Error { .. }) {
            return switched;
        }
        self.status.staged = None;
        match self.scene_stage(out) {
            Reply::Error { message } => {
                crate::log::note(&format!(
                    "took the staged scene, but staging the one that was out failed: {message}"
                ));
                self.unstage();
                Reply::Status(Box::new(self.reported()))
            }
            staged => staged,
        }
    }

    /// A new scene, empty or a copy of `from`, staged: nothing on the air
    /// moves, and one the pipeline will not stage is not kept.
    fn scene_draft(&mut self, name: String, from: Option<String>) -> Reply {
        if let Err(message) = self.a_new_scene_name(&name) {
            return Reply::Error { message };
        }
        let scene = match from {
            None => crate::picture::scenes::Scene {
                name: name.clone(),
                layers: vec![],
                elements: vec![],
                order: vec![],
                shader: None,
            },
            Some(from) => {
                let Some(mut copy) = self.current_scenes().into_iter().find(|s| s.name == from)
                else {
                    return Reply::Error {
                        message: format!("no scene {from:?}"),
                    };
                };
                copy.name = name.clone();
                copy
            }
        };
        self.status.scenes.push(scene);
        let staged = self.scene_stage(name.clone());
        if matches!(staged, Reply::Error { .. }) {
            self.status.scenes.retain(|scene| scene.name != name);
        }
        staged
    }

    /// A layer verb run on the staged scene: the air's state is put aside,
    /// the staged scene stands in its place on the domain's side and on the
    /// pipeline's, the verb runs as it always does, and the air comes back.
    /// Nothing is saved in between: the daemon writes the setup after a
    /// command, never during one.
    fn staged_verb(&mut self, command: Command) -> Reply {
        let Some(name) = self.status.staged.clone() else {
            return Reply::Error {
                message: "nothing staged: scene stage <name> first".into(),
            };
        };
        let a_layer_verb = matches!(
            command,
            Command::LayerCamera { .. }
                | Command::LayerScreen { .. }
                | Command::LayerWindow { .. }
                | Command::LayerImage { .. }
                | Command::LayerReplaceScreen { .. }
                | Command::LayerReplaceWindow { .. }
                | Command::LayerReplaceCamera { .. }
                | Command::LayerReplaceImage { .. }
                | Command::LayerVisible { .. }
                | Command::LayerRemove { .. }
                | Command::LayerMove { .. }
                | Command::LayerTransform { .. }
                | Command::LayerCrop { .. }
                | Command::LayerShape { .. }
                | Command::LayerMirror { .. }
                | Command::LayerPosition { .. }
                | Command::LayerShader { .. }
                | Command::Shader { .. }
                | Command::SceneElementAdd { .. }
                | Command::SceneElementSet { .. }
                | Command::SceneElementRemove { .. }
        );
        if !a_layer_verb {
            return Reply::Error {
                message: "the preview takes layer verbs alone".into(),
            };
        }
        self.status.scenes = self.current_scenes();
        let Some(staged) = self.status.scenes.iter().find(|s| s.name == name).cloned() else {
            self.unstage();
            return Reply::Error {
                message: format!("the staged scene {name:?} is gone"),
            };
        };
        let air_layers = std::mem::replace(&mut self.status.layers, staged.layers);
        let air_name = std::mem::replace(&mut self.status.active_scene, name.clone());
        let air_shader = std::mem::replace(&mut self.status.shader, staged.shader);
        let air_clock = std::mem::take(&mut self.counting);
        self.pipeline.edit_staged(true);
        let reply = self.handle(command);
        self.status.scenes = self.current_scenes();
        self.pipeline.edit_staged(false);
        self.status.layers = air_layers;
        self.status.active_scene = air_name;
        self.status.shader = air_shader;
        self.counting = air_clock;
        self.status.staged = Some(name);
        match reply {
            Reply::Error { message } => Reply::Error { message },
            _ => Reply::Status(Box::new(self.reported())),
        }
    }

    /// A scene out of the trash, beside the others, under a name not taken.
    fn scene_restore(&mut self, name: String) -> Reply {
        let Some(at) = self.status.trash.iter().position(|s| s.name == name) else {
            return Reply::Error {
                message: format!("no scene {name:?} in the trash"),
            };
        };
        let scene = self.status.trash.remove(at);
        self.scene_add(scene, "restored")
    }

    /// A scene beside the others, under its name, or `name (why)`, `name (why
    /// 2)`… when that is taken. Nothing on the air moves.
    fn scene_add(&mut self, mut scene: Scene, why: &str) -> Reply {
        let name = scene.name.clone();
        let taken = |n: &str| self.status.scenes.iter().any(|s| s.name == n);
        if taken(&name) {
            let mut again = format!("{name} ({why})");
            let mut n = 2;
            while taken(&again) {
                again = format!("{name} ({why} {n})");
                n += 1;
            }
            scene.name = again;
        }
        self.status.scenes.push(scene);
        Reply::Status(Box::new(self.reported()))
    }

    fn unstage(&mut self) {
        let _ = self.pipeline.stage(None);
        self.status.staged = None;
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
                .map(crate::picture::scenes::CaptureKey::of)
        });
        let new_sound = sound_source.and_then(|key| {
            next.iter()
                .find(|layer| {
                    layer.source.kind == crate::picture::layers::Kind::Screen
                        && crate::picture::scenes::CaptureKey::of(layer) == key
                })
                .map(|layer| layer.id.clone())
        });
        self.status.scenes = previous_scenes;
        let mut next = next;
        self.renumber(&mut next);
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
        self.seen();
        self.ticked();
        let cameras: Vec<(String, u64)> = self
            .status
            .layers
            .iter()
            .filter(|layer| {
                layer.visible && layer.source.kind == crate::picture::layers::Kind::Camera
            })
            .map(|layer| {
                (
                    layer.id.clone(),
                    self.pipeline.layer_flowing(&layer.id).captured,
                )
            })
            .collect();
        let stalled = self.stalls.observe(cameras);
        self.keep(stalled);
        self.notice();
    }

    /// The tick's own work, before the events are taken; it returns early,
    /// and an early return must not skip them.
    fn ticked(&mut self) {
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
            self.journal.note(now(), notice.clone());
            self.keep([crate::app::events::Event::Notice { text: notice }]);
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
        if let Some(line) = crate::air::journal::said(&Command::NextTrack, &reply) {
            self.journal.note(now(), line);
        }
    }

    /// Run a command and note what it did.
    ///
    /// The journal is written here rather than at each decision because there
    /// is exactly one funnel and a line written in forty places is a line
    /// missing from the forty-first. What is worth saying is
    /// [`crate::air::journal::said`], which is pure.
    pub fn handle(&mut self, command: Command) -> Reply {
        self.seen();
        let reads = only_reads(&command);
        // The rows, around the few verbs that change one. Only those: with no
        // account they are a file on disk, read whole.
        let rows = moves_a_row(&command).then(|| self.watching.destinations());
        let reply = self.decide(command.clone());
        if let Some(line) = crate::air::journal::said(&command, &reply) {
            self.journal.note(now(), line);
        }
        if let Some(refused) = crate::app::events::refused(&command, &reply) {
            self.keep([refused]);
        }
        if let Some(before) = rows {
            let after = self.watching.destinations();
            self.keep(crate::app::events::rows_between(&before, &after));
        }
        if !reads {
            self.notice();
        }
        reply
    }

    /// What the events follow, as it is now. Built from the engine's own
    /// state and not from [`Self::reported`], which asks the pipeline a
    /// dozen things.
    fn snapshot(&self) -> crate::app::events::Snapshot {
        use crate::app::events::LayerSeen;
        let heard = self.pipeline.hearing();
        crate::app::events::Snapshot {
            on_air: self.status.on_air,
            recording: self.status.recording,
            active_scene: self.status.active_scene.clone(),
            muted: self.status.muted,
            music: self.status.music.clone(),
            app: self.watching.reachable(),
            sending: self.pipeline.publishing().into_iter().collect(),
            troubles: self.pipeline.troubles().into_iter().collect(),
            scenes: self
                .status
                .scenes
                .iter()
                .map(|scene| scene.name.clone())
                .collect(),
            layers: self
                .status
                .layers
                .iter()
                .map(LayerSeen::of_layer)
                .chain(self.active_elements().iter().map(LayerSeen::of_element))
                .collect(),
            filter: self.status.shader.clone(),
            timers_done: self
                .counting
                .iter()
                .filter(|(_, deadline)| **deadline <= Instant::now())
                .map(|(id, _)| id.clone())
                .collect(),
            mic_complaint: heard.complaint,
            screen_complaint: heard.screen_complaint,
            app_complaint: heard.app_complaint,
            starved: heard.starved,
            dropped: heard.dropped,
            faders: self.status.faders,
            gate: self.status.gate,
            monitoring: self.status.monitoring,
            music_to_stream: self.status.music_to_stream,
            screen_sound: self.status.screen_sound,
            screen_sound_layer: self.status.screen_sound_layer.clone(),
            denoise: self.status.denoise,
            hearing_apps: self.status.hearing_apps.clone(),
            app_audio: self.status.app_audio.clone(),
            app_audio_volume: self.status.app_audio_volume,
            mirrored: self.status.mirrored,
            viewers: self.watching.viewers(),
        }
    }

    /// The events after `since`. See [`Command::Events`].
    fn events(&self, since: u64) -> Reply {
        let crate::app::events::Since { gap, events } = match self.events.lock() {
            Ok(events) => events.since(since),
            Err(poisoned) => poisoned.into_inner().since(since),
        };
        Reply::Events { gap, events }
    }

    /// The first snapshot, before the first thing the engine ever does, so
    /// that thing is an event like any other. Once.
    fn seen(&mut self) {
        if self.seen.is_none() {
            self.seen = Some(self.snapshot());
        }
    }

    /// Keep what changed since the last snapshot as events.
    ///
    /// Compared with what was seen last rather than with a snapshot taken
    /// just before the command: a read takes none, and whatever changed
    /// under one, or with no command at all, is still an event at the next
    /// tick instead of lost between two snapshots that both have it.
    fn notice(&mut self) {
        let now_seen = self.snapshot();
        let Some(before) = self.seen.replace(now_seen.clone()) else {
            return;
        };
        let changes = crate::app::events::between(&before, &now_seen);
        if changes.is_empty() {
            return;
        }
        self.keep(changes);
    }

    /// Keep these events, now, in the order given.
    fn keep(&self, changes: impl IntoIterator<Item = crate::app::events::Event>) {
        let at = now();
        let mut events = match self.events.lock() {
            Ok(events) => events,
            Err(poisoned) => poisoned.into_inner(),
        };
        for event in changes {
            events.push(at, event);
        }
    }

    /// One verb, one hand: every arm is a call into the context that owns the
    /// verb (`picture`, `sound`, `air`, `app`), and nothing is decided here.
    fn decide(&mut self, command: Command) -> Reply {
        match command {
            Command::Status => Reply::Status(Box::new(self.reported())),
            Command::SceneCreate { name } => self.scene_create(name),
            Command::SceneDuplicate { name } => self.scene_duplicate(name),
            Command::SceneSwitch { name } => {
                let switched = self.scene_switch(name.clone());
                // The scene in the preview went on the air by the switch: it
                // is staged no more.
                if !matches!(switched, Reply::Error { .. })
                    && self.status.staged.as_deref() == Some(name.as_str())
                {
                    self.unstage();
                }
                switched
            }
            Command::SceneDelete { name } => self.scene_delete(name),
            Command::SceneStage { name } => self.scene_stage(name),
            Command::SceneTake => self.scene_take(),
            Command::SceneDraft { name, from } => self.scene_draft(name, from),
            Command::SceneRestore { name } => self.scene_restore(name),
            Command::SceneAdd { scene } => self.scene_add(*scene, "imported"),
            Command::Staged { command } => self.staged_verb(*command),
            Command::AudioLayerAdd { id, source } => self.audio_layer_add(id, source),
            Command::AudioLayerRemove { id } => self.audio_layer_remove(id),
            Command::AudioLayerVolume { id, volume } => self.audio_layer_volume(id, volume),
            Command::AudioLayerMute { id, on } => self.audio_layer_mute(id, on),
            Command::AudioLayerDuck { id, duck } => self.audio_layer_duck(id, duck),
            Command::Watching { on } => self.watch(on),
            Command::Present => self.present(),
            Command::Levels => self.levels(),
            Command::Sources => self.sources(),
            Command::Mics => match self.sources.mics() {
                Ok(mics) => Reply::Sources(Devices {
                    mics,
                    ..Devices::default()
                }),
                Err(message) => Reply::Error { message },
            },
            Command::Genres => Reply::Sources(Devices {
                genres: self.genres(),
                ..Devices::default()
            }),
            Command::Quit => self.quit(),
            // the daemon acts on it beside the engine; here it is a fact
            Command::Rewire => Reply::Ok,
            Command::GoLive => self.go_live(),
            Command::Plan => Reply::Plan(crate::air::plan::Plan::of(&self.reported())),
            Command::Live { plan } => {
                if crate::air::plan::Plan::of(&self.reported()).fingerprint != plan {
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
            Command::LayerImage { id, path } => self.layer_image(id, path),
            Command::LayerReplaceImage { id, path } => self.layer_replace_image(id, path),
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
            Command::Events { since, .. } => self.events(since),
            Command::Hide { seq } => self.hide(seq),
            Command::Delete { seq } => self.delete_chat(seq),
            Command::Say { body, channel } => self.say(&body, channel),
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

    /// The music folder's genres, by id and readable name.
    fn genres(&self) -> Vec<Named> {
        self.library
            .playlists()
            .into_iter()
            .map(|playlist| Named {
                id: playlist.name,
                name: playlist.title,
            })
            .collect()
    }

    pub(super) fn sources(&mut self) -> Reply {
        match self.sources.available() {
            Ok(available) => {
                let mut devices: Devices = available.into();
                devices.genres = self.genres();
                Reply::Sources(devices)
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
        crate::sound::music::music_fader_db(self.status.faders.music)
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
mod fake;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::*;
    use crate::picture::layers::Kind;
    use crate::picture::scenes::Scene;

    // Setting up is the part nobody wants to do twice.
    #[test]
    fn what_was_chosen_comes_back_after_a_restart() {
        let (mut engine, played, _) = machine_with_music();
        engine.handle(Command::Camera {
            device: Some("HP".into()),
        });
        engine.handle(Command::Mic {
            device: Some("HyperX".into()),
        });
        engine.handle(Command::Mirror { on: true });
        engine.handle(Command::CameraShape {
            shape: crate::picture::scene::CameraShape::Rectangle,
        });
        engine.handle(Command::CameraPosition {
            at: Some(crate::picture::scene::CameraPosition { x: 360, y: 180 }),
        });
        engine.handle(Command::Volume { level: 1.5 });
        engine.handle(Command::Genre { name: "edm".into() });
        engine.handle(Command::SceneElementAdd {
            element: Element {
                id: "title".into(),
                x: 200,
                y: 200,
                width: 600,
                height: 100,
                visible: true,
                shader: None,
                content: ElementContent::Text {
                    text: "back at nine".into(),
                },
            },
        });
        let setup = engine.remembered();
        // Never state: an engine that came up publishing because it was
        // publishing when the machine slept goes live in an empty room.
        let written = crate::remembered::write(&setup).expect("written");
        assert!(!written.contains("on_air"), "{written}");
        assert!(!written.contains("recording"), "{written}");

        let (mut next, next_played, _) = machine_with_music();
        next.restore(&crate::remembered::read(&written));
        let now = next.status();
        assert_eq!(now.mic.as_deref(), Some("HyperX DuoCast"));
        assert!(now.mirrored);
        assert_eq!(
            now.layers[0].shape,
            Some(crate::picture::scene::CameraShape::Rectangle)
        );
        assert_eq!(
            (now.layers[0].transform.x, now.layers[0].transform.y),
            (360, 180)
        );
        assert_eq!(now.faders.mic, 1.5);
        assert_eq!(
            now.scenes[0].elements[0].content,
            ElementContent::Text {
                text: "back at nine".into()
            }
        );
        assert!(!now.on_air);
        // The genre came back as the shelf, not as music: nothing plays until
        // somebody turns it on, and then it is that shelf. An engine that came
        // up playing was the music that "kept playing on its own with the app
        // closed": every engine started for a smoke played the last genre
        // through the speakers of a room nobody was in.
        assert!(
            now.music.is_none(),
            "restoring a genre started it: {:?}",
            now.music
        );
        assert!(next_played.lock().expect("played").is_empty());
        next.handle(Command::Music { on: true });
        let track = next_played
            .lock()
            .expect("played")
            .first()
            .cloned()
            .flatten();
        assert!(
            track.as_deref().is_some_and(|t| t.contains("edm")),
            "music on resumes the remembered shelf, played {track:?}"
        );
        let _ = played;
    }

    // A display is put back by what the monitor is, not by the number it had:
    // the VG2791R saved as display 2 is display 3 now, and a monitor that is not
    // here is left out rather than swapped for the one that took its number.
    #[test]
    fn a_saved_display_comes_back_as_the_same_monitor_whatever_its_number() {
        let (mut engine, _, _) = machine_with_music();
        let desk = |id: &str, handle: &str, stable: &str| crate::picture::layers::Layer {
            id: id.into(),
            source: crate::picture::layers::Source {
                kind: crate::picture::layers::Kind::Screen,
                handle: handle.into(),
                name: "a monitor".into(),
                width: 1920,
                height: 1080,
                stable: Some(stable.into()),
            },
            transform: crate::picture::layers::Transform::native((1920, 1080)),
            visible: true,
            crop: None,
            shape: None,
            mirrored: false,
            shader: None,
        };
        engine.restore(&crate::remembered::Remembered {
            layers: vec![desk("desk", "2", "5C1E09B4"), desk("gone", "1", "0FF1CE00")],
            ..Default::default()
        });
        let layers = &engine.status().layers;
        assert_eq!(
            layers.len(),
            1,
            "the absent monitor is left out: {layers:?}"
        );
        assert_eq!(
            (layers[0].id.as_str(), layers[0].source.handle.as_str()),
            ("desk", "3")
        );
        assert_eq!(layers[0].source.name, "VG2791R");
        assert_eq!(layers[0].source.stable.as_deref(), Some("5C1E09B4"));
        assert_eq!(
            engine.remembered().layers[0].source.stable.as_deref(),
            Some("5C1E09B4"),
            "and it is kept by its identity again"
        );
    }

    // Losing a webcam is not a reason to lose the gate settings somebody spent
    // an evening on.
    #[test]
    fn a_device_that_is_gone_does_not_take_the_rest_of_the_setup_with_it() {
        let (mut engine, _, _) = machine_with_music();
        let setup = crate::remembered::Remembered {
            layers: vec![crate::picture::layers::Layer {
                id: "gone".into(),
                source: crate::picture::layers::Source {
                    kind: crate::picture::layers::Kind::Camera,
                    handle: "missing".into(),
                    name: "a camera nobody has".into(),
                    width: 1280,
                    height: 720,
                    stable: None,
                },
                transform: crate::picture::layers::Transform::native((1280, 720)),
                visible: true,
                crop: None,
                shape: Some(crate::picture::scene::CameraShape::Rectangle),
                mirrored: false,
                shader: None,
            }],
            mic: Some("HyperX".into()),
            mirrored: true,
            ..Default::default()
        };
        engine.restore(&setup);
        assert!(engine.status().layers.is_empty(), "it must not invent one");
        assert_eq!(engine.status().mic.as_deref(), Some("HyperX DuoCast"));
        assert!(engine.status().mirrored);
    }

    #[test]
    fn restart_restores_named_window_layers_without_legacy_fields() {
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
        engine.handle(Command::LayerWindow {
            id: "notes".into(),
            query: "tmux".into(),
        });
        let saved = crate::remembered::write(&engine.remembered()).unwrap();
        assert!(saved.contains("Ghostty — tmux a"));
        assert!(!saved.contains("camera_position"));
        assert!(!saved.contains("\"screen\""));
        let mut next =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
        next.restore(&crate::remembered::read(&saved));
        assert_eq!(next.status().layers.len(), 1);
        assert_eq!(next.status().layers[0].id, "notes");
        assert_eq!(
            next.status().layers[0].source.kind,
            crate::picture::layers::Kind::Window
        );
    }

    #[test]
    fn the_status_carries_what_the_engine_has_done() {
        let mut engine = Engine::new();
        let _ = engine.handle(Command::Mute { on: true });
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status");
        };
        // The stamp is the clock; what is asserted is the sentence and that
        // asking for the status did not itself become a line.
        assert_eq!(status.log.len(), 1);
        assert!(status.log[0].ends_with("mic muted"), "got {:?}", status.log);
    }

    #[test]
    fn sources_come_from_whatever_is_plugged_in() {
        let mut engine = Engine::with_sources(Box::new(ThisMachine));
        let Reply::Sources(devices) = engine.handle(Command::Sources) else {
            panic!("sources answers with sources")
        };
        assert_eq!(devices.screens.len(), 2);
        assert_eq!(devices.screens[1].name, "VG2791R");
        // A window is shown as who owns it and then what it says, because a
        // browser window is titled after the page and neither half alone
        // tells you which one it is.
        assert_eq!(devices.windows[1].name, "Brave Browser — remux");
    }

    // Asking what can be captured moves nothing the events follow: no
    // snapshot after it.
    #[test]
    fn asking_what_can_be_captured_only_reads() {
        assert!(only_reads(&Command::Sources));
    }

    // An empty list would read as "you have no monitors", which sends a person
    // looking at their cables instead of at System Settings.
    #[test]
    fn a_refused_grant_comes_back_as_a_sentence_not_an_empty_list() {
        let mut engine = Engine::with_sources(Box::new(Refused));
        assert_eq!(
            engine.handle(Command::Sources),
            Reply::Error {
                message: "macOS refused: screen recording".into()
            }
        );
    }

    #[test]
    fn an_engine_with_nothing_plugged_in_says_so_truthfully() {
        let mut engine = engine();
        assert_eq!(
            engine.handle(Command::Sources),
            Reply::Sources(Devices::default())
        );
    }

    #[test]
    fn a_fresh_engine_is_off_air_and_not_recording() {
        let mut engine = engine();
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status answers with a status")
        };
        assert!(!status.on_air);
        assert!(!status.recording);
        assert_eq!(status.viewers, None);
    }

    #[test]
    fn fresh_engine_has_only_a_default_scene() {
        let engine = engine();
        assert_eq!(engine.status().scenes.len(), 1);
        assert_eq!(engine.status().active_scene, "default");
    }

    // The panic button had a bug worth keeping a test for: it hid the picture
    // and left the microphone open, which is the worst of both.
    #[test]
    fn hiding_everything_mutes_you_too() {
        let (mut engine, _) = publishing_engine(None);
        engine.handle(Command::Screen { display: 1 });
        engine.handle(Command::GoLive);
        engine.handle(Command::HideEverything);
        assert!(
            engine.status().muted,
            "hide everything means hide your voice too"
        );
        assert_eq!(engine.status().active_scene, "default");
        assert!(engine.status().layers.is_empty());
        assert!(
            engine.status().on_air,
            "it is a break, not the end of the live"
        );
    }

    #[test]
    fn every_verb_in_the_protocol_is_one_this_build_answers() {
        // There used to be a `Reply::Unsupported` and a test naming whichever
        // verb was still missing; it was `next-track`, then `arm`, and now
        // there is nothing to name. A reply nothing can produce is a reply
        // somebody will write a reader for, so it went with the last gap.
        //
        // What is asserted instead is the property that made it safe to
        // delete: every command decodes into something `handle` answers, which
        // the compiler proves by the match being exhaustive, and this proves
        // by asking for one of each shape.
        let (mut engine, _) = publishing_engine(None);
        for command in [
            Command::Status,
            Command::Sources,
            Command::Grants,
            Command::Chat {
                since: 0,
                follow: false,
            },
            Command::Mute { on: true },
            Command::Arm {
                adapter: 1,
                on: true,
            },
        ] {
            let reply = engine.handle(command.clone());
            assert!(
                !matches!(reply, Reply::Error { message } if message.contains("cannot")),
                "{command:?} came back as unanswerable"
            );
        }
    }

    #[test]
    fn quitting_is_a_decision_the_engine_records_and_does_not_act_on() {
        let mut engine = engine();
        assert!(!engine.quitting());
        assert_eq!(engine.handle(Command::Quit), Reply::Ok);
        assert!(
            engine.quitting(),
            "the transport reads this and stops accepting"
        );
    }

    #[test]
    fn the_camera_and_the_microphone_are_separate_lists() {
        let mut engine = machine();
        // "HyperX" is a microphone, so asking the camera list for it must miss
        // rather than quietly find the mic.
        let Reply::Error { message } = engine.handle(Command::Camera {
            device: Some("hyperx".into()),
        }) else {
            panic!("a camera named after a microphone is not a camera")
        };
        assert!(message.contains("MacBook Pro Camera"), "{message}");
        assert!(engine.status().layers.is_empty());

        engine.handle(Command::Mic {
            device: Some("hyperx".into()),
        });
        assert_eq!(engine.status().mic, Some("HyperX DuoCast".into()));
    }

    #[test]
    fn deleted_starter_scene_is_not_recreated_on_restore() {
        let mut engine = Engine::new();
        engine.handle(Command::SceneCreate {
            name: "Custom".into(),
        });
        engine.handle(Command::SceneDelete {
            name: "default".into(),
        });
        let saved =
            crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
        let mut next = Engine::new();
        next.restore(&saved);
        assert_eq!(next.status().scenes.len(), 1);
        assert_eq!(next.status().active_scene, "Custom");
    }

    #[test]
    fn old_generated_presets_are_ignored_without_losing_custom_scenes() {
        let saved = crate::remembered::read(
            r#"{"active_scene":"BRB","scenes":[{"name":"default","layers":[]},{"name":"BRB","layers":[],"graphic":{"kind":"back-in-a-moment","text":"back"}},{"name":"My scene","layers":[],"elements":[{"id":"note","kind":"text","text":"Hi","x":2,"y":3,"width":100,"height":80}]}]}"#,
        );
        let mut engine = Engine::new();
        engine.restore(&saved);
        assert_eq!(engine.status().active_scene, "default");
        assert_eq!(engine.status().scenes.len(), 2);
        assert_eq!(engine.status().scenes[1].elements[0].id, "note");
        assert_eq!(engine.status().scenes[1].ordered_ids(), ["note"]);
    }

    #[test]
    fn scene_switch_prepares_then_commits_and_closes_only_unshared_captures() {
        let fake = Wrote::default();
        let events = fake.scene_events.clone();
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        let before = vec![
            layer("camera", Kind::Camera, "cam", 0),
            layer("old", Kind::Screen, "1", 0),
        ];
        let after = vec![
            layer("closeup", Kind::Camera, "cam", 900),
            layer("new", Kind::Window, "2", 10),
        ];
        engine.status.layers = before.clone();
        engine.status.scenes.push(Scene {
            name: "next".into(),
            layers: after.clone(),
            elements: vec![],
            order: vec![],
            shader: None,
        });
        engine.status.on_air = true;
        engine.status.recording = true;
        assert!(matches!(
            engine.handle(Command::SceneSwitch {
                name: "next".into()
            }),
            Reply::Status(_)
        ));
        assert!(engine.status.on_air);
        assert!(engine.status.recording);
        assert_eq!(engine.status.layers, after);
        assert_eq!(engine.status.scenes[0].layers, before);
        assert_eq!(
            *events.lock().unwrap(),
            ["prepare Window:2", "commit layout", "close Screen:1"]
        );
        assert!(matches!(
            engine.handle(Command::SceneSwitch {
                name: "default".into()
            }),
            Reply::Status(_)
        ));
        assert_eq!(engine.status.layers, before);
    }

    /// The air on "default" with one screen, and "next" beside it with a camera.
    fn with_next(fake: Wrote) -> Engine {
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        engine.status.layers = vec![layer("desk", Kind::Screen, "1", 0)];
        engine.status.scenes.push(Scene {
            name: "next".into(),
            layers: vec![layer("keys", Kind::Camera, "cam", 0)],
            elements: vec![],
            order: vec![],
            shader: None,
        });
        engine
    }

    #[test]
    fn staging_draws_a_scene_off_the_air_and_leaves_the_air_alone() {
        let fake = Wrote::default();
        let events = fake.scene_events.clone();
        let mut engine = with_next(fake);
        engine.status.on_air = true;
        let staged = engine.handle(Command::SceneStage {
            name: "next".into(),
        });
        assert!(matches!(staged, Reply::Status(_)), "{staged:?}");
        assert_eq!(engine.status.staged.as_deref(), Some("next"));
        assert_eq!(
            engine.status.active_scene, "default",
            "the air did not move"
        );
        assert_eq!(engine.status.layers[0].id, "desk");
        assert_eq!(*events.lock().unwrap(), ["stage next"]);
    }

    #[test]
    fn take_puts_the_staged_scene_on_the_air_and_stages_the_one_that_was_out() {
        let fake = Wrote::default();
        let events = fake.scene_events.clone();
        let mut engine = with_next(fake);
        engine.handle(Command::SceneStage {
            name: "next".into(),
        });
        let taken = engine.handle(Command::SceneTake);
        assert!(matches!(taken, Reply::Status(_)), "{taken:?}");
        assert_eq!(engine.status.active_scene, "next");
        assert_eq!(engine.status.layers[0].id, "keys");
        assert_eq!(
            engine.status.staged.as_deref(),
            Some("default"),
            "a second take goes back"
        );
        assert_eq!(
            events.lock().unwrap().last().map(String::as_str),
            Some("stage default")
        );
    }

    #[test]
    fn take_with_nothing_staged_says_so_and_moves_nothing() {
        let mut engine = with_next(Wrote::default());
        let Reply::Error { message } = engine.handle(Command::SceneTake) else {
            panic!("a take with nothing staged is an error")
        };
        assert!(message.contains("scene stage"), "{message}");
        assert_eq!(engine.status.active_scene, "default");
    }

    #[test]
    fn a_layer_verb_on_the_preview_edits_the_staged_scene_and_leaves_the_air_alone() {
        let fake = Wrote::default();
        let events = fake.scene_events.clone();
        let mut engine = with_next(fake);
        engine.handle(Command::SceneStage {
            name: "next".into(),
        });
        let reply = engine.handle(Command::Staged {
            command: Box::new(Command::LayerVisible {
                id: "keys".into(),
                on: false,
            }),
        });
        assert!(matches!(reply, Reply::Status(_)), "{reply:?}");
        assert_eq!(
            engine.status.active_scene, "default",
            "the air did not move"
        );
        assert_eq!(engine.status.staged.as_deref(), Some("next"));
        assert!(
            engine.status.layers[0].visible,
            "the air's desk is as it was"
        );
        let next = engine
            .status
            .scenes
            .iter()
            .find(|s| s.name == "next")
            .unwrap();
        assert!(!next.layers[0].visible, "the staged keys are hidden");
        let told = events.lock().unwrap().clone();
        assert_eq!(told.first().map(String::as_str), Some("stage next"));
        assert!(
            told.contains(&"edit staged".to_string()) && told.contains(&"edit air".to_string()),
            "{told:?}"
        );
    }

    #[test]
    fn a_layer_added_on_the_preview_is_the_staged_scene_s_alone() {
        let mut engine = with_next(Wrote::default());
        engine.handle(Command::SceneStage {
            name: "next".into(),
        });
        let logo =
            std::env::temp_dir().join(format!("remux-staged-logo-{}.png", std::process::id()));
        std::fs::write(&logo, b"\x89PNG").expect("a picture file");
        let reply = engine.handle(Command::Staged {
            command: Box::new(Command::LayerImage {
                id: "logo".into(),
                path: logo.to_string_lossy().into_owned(),
            }),
        });
        assert!(matches!(reply, Reply::Status(_)), "{reply:?}");
        assert_eq!(engine.status.layers.len(), 1, "the air has its desk alone");
        let next = engine
            .status
            .scenes
            .iter()
            .find(|s| s.name == "next")
            .unwrap();
        assert_eq!(
            next.layers
                .iter()
                .map(|l| l.id.as_str())
                .collect::<Vec<_>>(),
            ["keys", "logo"]
        );
    }

    #[test]
    fn the_preview_takes_layer_verbs_alone_and_needs_a_scene_staged() {
        let mut engine = with_next(Wrote::default());
        let Reply::Error { message } = engine.handle(Command::Staged {
            command: Box::new(Command::LayerVisible {
                id: "keys".into(),
                on: false,
            }),
        }) else {
            panic!("nothing staged is an error")
        };
        assert!(message.contains("nothing staged"), "{message}");
        engine.handle(Command::SceneStage {
            name: "next".into(),
        });
        let Reply::Error { message } = engine.handle(Command::Staged {
            command: Box::new(Command::Stop),
        }) else {
            panic!("stop is not a layer verb")
        };
        assert!(message.contains("layer"), "{message}");
    }

    #[test]
    fn a_draft_is_a_new_scene_in_the_preview_and_the_air_does_not_move() {
        let fake = Wrote::default();
        let events = fake.scene_events.clone();
        let mut engine = with_next(fake);
        let reply = engine.handle(Command::SceneDraft {
            name: "empty".into(),
            from: None,
        });
        assert!(matches!(reply, Reply::Status(_)), "{reply:?}");
        assert_eq!(engine.status.active_scene, "default");
        assert_eq!(engine.status.staged.as_deref(), Some("empty"));
        assert!(engine
            .status
            .scenes
            .iter()
            .any(|s| s.name == "empty" && s.layers.is_empty()));
        assert_eq!(*events.lock().unwrap(), ["stage empty"]);
    }

    #[test]
    fn a_draft_from_a_scene_copies_it_into_the_preview() {
        let mut engine = with_next(Wrote::default());
        engine.handle(Command::SceneDraft {
            name: "copy".into(),
            from: Some("next".into()),
        });
        let copy = engine
            .status
            .scenes
            .iter()
            .find(|s| s.name == "copy")
            .expect("the copy");
        assert_eq!(copy.layers[0].id, "keys");
        assert_eq!(engine.status.staged.as_deref(), Some("copy"));
        assert_eq!(engine.status.active_scene, "default");
        assert!(
            matches!(
                engine.handle(Command::SceneDraft {
                    name: "copy".into(),
                    from: None
                }),
                Reply::Error { .. }
            ),
            "a name already taken"
        );
        assert!(
            matches!(
                engine.handle(Command::SceneDraft {
                    name: "other".into(),
                    from: Some("nope".into())
                }),
                Reply::Error { .. }
            ),
            "a scene that is not there"
        );
    }

    #[test]
    fn a_scene_from_a_bench_is_added_beside_the_others_under_a_free_name() {
        let mut engine = with_next(Wrote::default());
        let mut theirs = engine
            .status
            .scenes
            .iter()
            .find(|s| s.name == "next")
            .cloned()
            .expect("next");
        theirs.layers[0].id = "theirs".into();
        let added = engine.handle(Command::SceneAdd {
            scene: Box::new(theirs),
        });
        assert!(matches!(added, Reply::Status(_)), "{added:?}");
        let names: Vec<_> = engine
            .status
            .scenes
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(names.contains(&"next (imported)"), "{names:?}");
        let ours = engine
            .status
            .scenes
            .iter()
            .find(|s| s.name == "next")
            .expect("kept");
        assert_eq!(
            ours.layers[0].id, "keys",
            "the scene that was there is untouched"
        );
        assert_eq!(
            engine.status.active_scene, "default",
            "nothing on the air moves"
        );
    }

    #[test]
    fn a_deleted_scene_goes_to_the_trash_whole_and_comes_back_by_name() {
        let mut engine = with_next(Wrote::default());
        engine.handle(Command::SceneDelete {
            name: "next".into(),
        });
        assert!(!engine.status.scenes.iter().any(|s| s.name == "next"));
        assert_eq!(engine.status.trash.len(), 1);
        assert_eq!(
            engine.status.trash[0].layers[0].id, "keys",
            "whole, its layers kept"
        );
        let back = engine.handle(Command::SceneRestore {
            name: "next".into(),
        });
        assert!(matches!(back, Reply::Status(_)), "{back:?}");
        let next = engine
            .status
            .scenes
            .iter()
            .find(|s| s.name == "next")
            .expect("back");
        assert_eq!(next.layers[0].id, "keys");
        assert!(engine.status.trash.is_empty());
        assert_eq!(
            engine.status.active_scene, "default",
            "restoring puts nothing on the air"
        );
    }

    #[test]
    fn a_scene_restored_under_a_name_taken_since_gets_another() {
        let mut engine = with_next(Wrote::default());
        engine.handle(Command::SceneDelete {
            name: "next".into(),
        });
        engine.handle(Command::SceneDraft {
            name: "next".into(),
            from: None,
        });
        engine.handle(Command::SceneRestore {
            name: "next".into(),
        });
        let names: Vec<_> = engine
            .status
            .scenes
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            names.contains(&"next") && names.contains(&"next (restored)"),
            "{names:?}"
        );
    }

    #[test]
    fn the_trash_keeps_the_last_ten_and_restoring_what_is_not_there_says_so() {
        let mut engine = with_next(Wrote::default());
        for n in 0..12 {
            let name = format!("s{n}");
            engine.handle(Command::SceneDraft {
                name: name.clone(),
                from: None,
            });
            engine.handle(Command::SceneStage {
                name: "default".into(),
            });
            engine.handle(Command::SceneDelete { name });
        }
        assert_eq!(engine.status.trash.len(), 10);
        assert_eq!(engine.status.trash[0].name, "s11", "the latest first");
        assert!(matches!(
            engine.handle(Command::SceneRestore { name: "s0".into() }),
            Reply::Error { .. }
        ));
    }

    #[test]
    fn the_trash_is_remembered_across_a_restart() {
        let mut engine = with_next(Wrote::default());
        engine.handle(Command::SceneDelete {
            name: "next".into(),
        });
        let saved =
            crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
        let mut again = Engine::new().with_pipeline(Box::new(Wrote::default()));
        again.restore(&saved);
        assert_eq!(again.status.trash.len(), 1);
        assert_eq!(again.status.trash[0].name, "next");
    }

    #[test]
    fn staging_the_scene_on_the_air_stages_nothing() {
        let fake = Wrote::default();
        let events = fake.scene_events.clone();
        let mut engine = with_next(fake);
        engine.handle(Command::SceneStage {
            name: "next".into(),
        });
        engine.handle(Command::SceneStage {
            name: "default".into(),
        });
        assert_eq!(engine.status.staged, None);
        assert_eq!(*events.lock().unwrap(), ["stage next", "unstage"]);
    }

    #[test]
    fn a_stage_the_pipeline_refuses_leaves_nothing_staged() {
        let mut engine = with_next(Wrote {
            refuse: Some("stage:camera busy".into()),
            ..Default::default()
        });
        let Reply::Error { message } = engine.handle(Command::SceneStage {
            name: "next".into(),
        }) else {
            panic!("a refused stage is an error")
        };
        assert!(message.contains("camera busy"), "{message}");
        assert_eq!(engine.status.staged, None);
    }

    #[test]
    fn staging_a_scene_that_is_not_there_is_an_error() {
        let mut engine = with_next(Wrote::default());
        assert!(matches!(
            engine.handle(Command::SceneStage {
                name: "nope".into()
            }),
            Reply::Error { .. }
        ));
    }

    #[test]
    fn deleting_the_staged_scene_unstages_it() {
        let fake = Wrote::default();
        let events = fake.scene_events.clone();
        let mut engine = with_next(fake);
        engine.handle(Command::SceneStage {
            name: "next".into(),
        });
        engine.handle(Command::SceneDelete {
            name: "next".into(),
        });
        assert_eq!(engine.status.staged, None);
        assert_eq!(
            events.lock().unwrap().last().map(String::as_str),
            Some("unstage")
        );
    }

    #[test]
    fn failed_preparation_preserves_active_layout_and_live() {
        let fake = Wrote {
            refuse: Some("capture unavailable".into()),
            ..Default::default()
        };
        let events = fake.scene_events.clone();
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        let old = layer("face", Kind::Camera, "cam", 0);
        engine.status.layers = vec![old.clone()];
        engine.status.on_air = true;
        engine.status.scenes.push(Scene {
            name: "other".into(),
            layers: vec![layer("new", Kind::Camera, "other-cam", 10)],
            elements: vec![],
            order: vec![],
            shader: None,
        });
        assert!(matches!(
            engine.handle(Command::SceneSwitch {
                name: "other".into()
            }),
            Reply::Error { .. }
        ));
        assert_eq!(engine.status.active_scene, "default");
        assert_eq!(engine.status.layers, [old]);
        assert!(engine.status.on_air);
        assert_eq!(
            *events.lock().unwrap(),
            ["prepare Camera:other-cam", "rollback prepared"]
        );
    }

    #[test]
    fn second_capture_failure_rolls_back_prepared_source_before_touching_live() {
        let fake = Wrote {
            refuse: Some("scene:missing".into()),
            ..Default::default()
        };
        let events = fake.scene_events.clone();
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        let previous = layer("face", Kind::Camera, "camera", 0);
        engine.status.layers = vec![previous.clone()];
        engine.status.on_air = true;
        engine.status.scenes.push(Scene {
            name: "next".into(),
            layers: vec![
                layer("screen", Kind::Screen, "1", 0),
                layer("bad", Kind::Window, "missing", 0),
            ],
            elements: vec![],
            order: vec![],
            shader: None,
        });
        assert!(matches!(
            engine.handle(Command::SceneSwitch {
                name: "next".into()
            }),
            Reply::Error { .. }
        ));
        assert_eq!(engine.status.active_scene, "default");
        assert_eq!(engine.status.layers, [previous]);
        assert!(engine.status.on_air);
        assert_eq!(
            *events.lock().unwrap(),
            [
                "prepare Screen:1",
                "prepare Window:missing",
                "rollback 1",
                "rollback prepared"
            ]
        );
    }

    #[test]
    fn legacy_layers_restore_as_default_and_named_scenes_round_trip() {
        let old = layer("desktop", Kind::Screen, "1", 17);
        let legacy = crate::remembered::read(&serde_json::json!({"layers": [old]}).to_string());
        let mut engine = Engine::with_sources(Box::new(ThisMachine));
        engine.restore(&legacy);
        assert_eq!(engine.status.active_scene, "default");
        assert_eq!(engine.remembered().scenes[0].layers, engine.status.layers);

        let mut setup = engine.remembered();
        setup.scenes.push(Scene {
            name: "camera".into(),
            layers: vec![layer("second", Kind::Screen, "3", 88)],
            elements: vec![],
            order: vec![],
            shader: None,
        });
        setup.active_scene = "camera".into();
        let saved = crate::remembered::read(&crate::remembered::write(&setup).unwrap());
        let mut restored = Engine::with_sources(Box::new(ThisMachine));
        restored.restore(&saved);
        assert_eq!(restored.status.active_scene, "camera");
        assert_eq!(restored.status.layers[0].id, "second");
        assert_eq!(restored.remembered().scenes[0].layers[0].transform.x, 17);
    }

    #[test]
    fn invalid_layer_assignment_and_scene_shader_roll_back_without_stopping_live() {
        let fake = Wrote {
            refuse: Some("shader:invalid".into()),
            ..Default::default()
        };
        let events = fake.scene_events.clone();
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        let old = layer("old", Kind::Camera, "cam", 0);
        engine.status.layers = vec![old.clone()];
        engine.status.on_air = true;
        let mut next = layer("new", Kind::Screen, "display", 0);
        next.shader = Some("bad.wgsl".into());
        engine.status.scenes.push(Scene {
            name: "next".into(),
            layers: vec![next],
            elements: vec![],
            order: vec![],
            shader: None,
        });
        assert!(matches!(
            engine.handle(Command::SceneSwitch {
                name: "next".into()
            }),
            Reply::Error { .. }
        ));
        assert_eq!(engine.status.layers, [old]);
        assert!(engine.status.on_air);
        assert_eq!(engine.status.active_scene, "default");
        assert_eq!(
            *events.lock().unwrap(),
            [
                "prepare Screen:display",
                "rollback display",
                "rollback prepared"
            ]
        );
        assert!(matches!(
            engine.handle(Command::LayerShader {
                id: "old".into(),
                path: Some("bad.wgsl".into())
            }),
            Reply::Error { .. }
        ));
        assert_eq!(engine.status.layers[0].shader, None);
        engine.status.scenes.push(Scene {
            name: "global".into(),
            layers: vec![],
            elements: vec![],
            order: vec![],
            shader: Some("bad.wgsl".into()),
        });
        assert!(matches!(
            engine.handle(Command::SceneSwitch {
                name: "global".into()
            }),
            Reply::Error { .. }
        ));
        assert_eq!(engine.status.active_scene, "default");
    }

    #[test]
    fn runtime_failure_hides_only_the_affected_shader_in_status_and_persistence() {
        let fake = Wrote {
            refuse: Some("runtime:layer".into()),
            ..Default::default()
        };
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        let mut disabled = layer("disabled", Kind::Camera, "cam", 0);
        disabled.shader = Some("failed.wgsl".into());
        let mut healthy = layer("healthy", Kind::Screen, "1", 0);
        healthy.shader = Some("good.wgsl".into());
        engine.status.layers = vec![disabled, healthy];
        engine.status.shader = Some("scene.wgsl".into());
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status")
        };
        assert_eq!(status.layers[0].shader, None);
        assert_eq!(status.layers[1].shader.as_deref(), Some("good.wgsl"));
        assert_eq!(status.shader.as_deref(), Some("scene.wgsl"));
        assert_eq!(engine.remembered().layers[0].shader, None);
        assert_eq!(
            engine.remembered().layers[1].shader.as_deref(),
            Some("good.wgsl")
        );

        let fake = Wrote {
            refuse: Some("runtime:global".into()),
            ..Default::default()
        };
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        engine.status.shader = Some("failed.wgsl".into());
        engine.status.layers = vec![layer("healthy", Kind::Camera, "cam", 0)];
        engine.status.layers[0].shader = Some("good.wgsl".into());
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status")
        };
        assert_eq!(status.shader, None);
        assert_eq!(status.layers[0].shader.as_deref(), Some("good.wgsl"));
        assert_eq!(engine.remembered().scenes[0].shader, None);
    }

    #[test]
    fn restore_skips_bad_shader_paths_without_losing_sources_or_other_scenes() {
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
        let mut saved_layer = layer("desktop", Kind::Screen, "1", 0);
        saved_layer.shader = Some("bad.wgsl".into());
        let setup = crate::remembered::Remembered {
            scenes: vec![
                Scene {
                    name: "default".into(),
                    layers: vec![saved_layer],
                    elements: vec![],
                    order: vec![],
                    shader: Some("bad.wgsl".into()),
                },
                Scene {
                    name: "later".into(),
                    layers: vec![],
                    elements: vec![],
                    order: vec![],
                    shader: Some("good.wgsl".into()),
                },
            ],
            ..Default::default()
        };
        engine.restore(&setup);
        assert_eq!(engine.status.layers.len(), 1);
        assert_eq!(engine.status.layers[0].shader, None);
        assert_eq!(engine.status.shader, None);
        assert_eq!(
            engine.remembered().scenes[1].shader.as_deref(),
            Some("good.wgsl")
        );
    }

    #[test]
    fn clone_edit_delete_and_retain_inactive_layouts() {
        let mut engine = Engine::new();
        assert!(matches!(
            engine.handle(Command::SceneDuplicate {
                name: "second".into()
            }),
            Reply::Status(_)
        ));
        assert!(matches!(
            engine.handle(Command::SceneDelete {
                name: "second".into()
            }),
            Reply::Error { .. }
        ));
        assert!(matches!(
            engine.handle(Command::SceneSwitch {
                name: "default".into()
            }),
            Reply::Status(_)
        ));
        assert!(matches!(
            engine.handle(Command::SceneDelete {
                name: "second".into()
            }),
            Reply::Status(_)
        ));
        assert_eq!(engine.remembered().scenes.len(), 1);
    }

    #[test]
    fn a_created_scene_is_empty_and_switched_to_and_a_duplicate_is_the_active_one() {
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
        engine.handle(Command::LayerCamera {
            id: "face".into(),
            device: "MacBook Pro Camera".into(),
        });
        engine.handle(Command::SceneElementAdd {
            element: Element {
                id: "title".into(),
                x: 10,
                y: 10,
                width: 300,
                height: 90,
                visible: true,
                shader: None,
                content: ElementContent::Text { text: "Hi".into() },
            },
        });
        let Reply::Status(copied) = engine.handle(Command::SceneDuplicate {
            name: "copy".into(),
        }) else {
            panic!("a duplicate answers with the status")
        };
        assert_eq!(copied.active_scene, "copy");
        assert_eq!(copied.layers.len(), 1, "the capture carries on");
        let copy = copied.scenes.iter().find(|s| s.name == "copy").unwrap();
        assert_eq!(copy.ordered_ids(), ["face", "title"]);

        let Reply::Status(empty) = engine.handle(Command::SceneCreate {
            name: "blank".into(),
        }) else {
            panic!("a create answers with the status")
        };
        assert_eq!(empty.active_scene, "blank");
        assert!(empty.layers.is_empty(), "nothing from the scene before");
        let blank = empty.scenes.iter().find(|s| s.name == "blank").unwrap();
        assert!(blank.elements.is_empty() && blank.shader.is_none());
        let copy = empty.scenes.iter().find(|s| s.name == "copy").unwrap();
        assert_eq!(copy.layers.len(), 1, "the scene left keeps its layers");

        for taken in ["blank", "copy", "default"] {
            assert!(matches!(
                engine.handle(Command::SceneCreate { name: taken.into() }),
                Reply::Error { .. }
            ));
            assert!(matches!(
                engine.handle(Command::SceneDuplicate { name: taken.into() }),
                Reply::Error { .. }
            ));
        }
        assert_eq!(engine.status().active_scene, "blank");
    }

    // A plan is confirmed by its fingerprint: the live goes on what was printed
    // and refused on anything else, so an agent cannot confirm a plan it did not
    // see and a person cannot go on a screen that changed under the prompt.
    #[test]
    fn a_live_confirmed_on_a_stale_plan_is_refused() {
        let (mut engine, published) = publishing_engine(None);
        engine.handle(Command::Screen { display: 1 });
        let Reply::Plan(plan) = engine.handle(Command::Plan) else {
            panic!("plan answers with a plan")
        };
        assert!(matches!(
            engine.handle(Command::Live {
                plan: plan.fingerprint.wrapping_add(1)
            }),
            Reply::Error { .. }
        ));
        assert!(published.lock().expect("published").is_empty());
        engine.handle(Command::Mute { on: true });
        assert!(
            matches!(
                engine.handle(Command::Live {
                    plan: plan.fingerprint
                }),
                Reply::Error { .. }
            ),
            "muting after the plan is a change a person confirms"
        );
        let Reply::Plan(fresh) = engine.handle(Command::Plan) else {
            panic!()
        };
        assert_eq!(
            engine.handle(Command::Live {
                plan: fresh.fingerprint
            }),
            Reply::Ok
        );
        assert!(engine.status().on_air);
    }

    /// A machine whose full listing fails, as one that would have to open a
    /// camera to list it, and whose microphones are read alone.
    struct MicsAlone;
    impl Sources for MicsAlone {
        fn available(&self) -> Result<Available, String> {
            Err("listing everything opens a camera".into())
        }
        fn mics(&self) -> Result<Vec<Named>, String> {
            Ok(vec![Named {
                id: "c9bf".into(),
                name: "HyperX DuoCast".into(),
            }])
        }
    }

    #[test]
    fn the_microphones_come_alone_without_a_camera_being_opened() {
        let mut engine = Engine::with_sources(Box::new(MicsAlone));
        let Reply::Sources(devices) = engine.handle(Command::Mics) else {
            panic!("mics answers as sources do, with the microphones alone")
        };
        assert_eq!(
            devices
                .mics
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            ["HyperX DuoCast"]
        );
        assert!(devices.cameras.is_empty());
    }

    #[test]
    fn the_microphones_of_a_machine_that_lists_them_with_the_rest_are_those() {
        let mut engine = Engine::with_sources(Box::new(ThisMachine));
        let Reply::Sources(devices) = engine.handle(Command::Mics) else {
            panic!("mics answers")
        };
        assert_eq!(devices.mics.len(), 2);
        assert!(devices.cameras.is_empty());
    }

    #[test]
    fn the_genres_come_alone_without_a_device_being_asked() {
        // Refused would fail any listing of devices: genres never ask for one.
        let mut engine =
            Engine::with_sources(Box::new(Refused)).with_library(Box::new(ThreeGenres));
        let Reply::Sources(devices) = engine.handle(Command::Genres) else {
            panic!("genres answers as sources do, with the genres alone")
        };
        assert_eq!(
            devices
                .genres
                .iter()
                .map(|g| g.id.as_str())
                .collect::<Vec<_>>(),
            vec!["lofi", "edm", "synthwave"]
        );
        assert!(
            devices.cameras.is_empty() && devices.mics.is_empty() && devices.screens.is_empty()
        );
    }

    #[test]
    fn the_music_a_panel_can_offer_comes_back_with_the_sources() {
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_library(Box::new(ThreeGenres));
        let Reply::Sources(devices) = engine.handle(Command::Sources) else {
            panic!("sources answers with sources")
        };
        assert_eq!(
            devices
                .genres
                .iter()
                .map(|g| g.id.as_str())
                .collect::<Vec<_>>(),
            vec!["lofi", "edm", "synthwave"],
            "a panel that can play music has to be able to offer some"
        );
        assert_eq!(
            devices.genres[0].name, "Lofi",
            "the name is the readable one"
        );
    }

    #[test]
    fn a_panel_can_ask_what_the_machine_allows() {
        let (mut engine, _) = publishing_engine(None);
        assert_eq!(
            engine.handle(Command::Grants),
            Reply::Grants {
                screen: Grant::Granted,
                camera: Grant::Granted,
                microphone: Grant::NotAsked,
            },
            "all three, always: the one that is wrong is what somebody is looking for"
        );
    }

    #[test]
    fn an_engine_that_captures_nothing_has_been_allowed_nothing() {
        let mut engine = engine();
        let Reply::Grants { screen, .. } = engine.handle(Command::Grants) else {
            panic!("grants answers with grants")
        };
        assert_eq!(screen, Grant::Refused, "true rather than convenient");
    }

    #[test]
    fn silence_is_the_floor_and_never_full_scale() {
        // Zero dBFS is the loudest sound there is, so a derived default is the
        // worst possible reading for "nothing is connected". A panel drew a
        // full red meter for a microphone that was not open.
        let floor = crate::sound::mixer::levels::Meter::FLOOR_DB;
        assert_eq!(Hearing::default().level_db, floor);
        assert_eq!(Mixing::default().level_db, floor);
        assert_eq!(Mixing::default().music_db, floor);
        assert_eq!(Mixing::default().monitor_db, floor);

        let mut engine = engine();
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status answers with a status")
        };
        assert_eq!(status.hearing.level_db, floor, "nothing is plugged in");
        assert_eq!(status.mixing.music_db, floor);
    }

    /// What the pipeline was last told a running timer had left.
    type Counting = std::sync::Arc<std::sync::Mutex<Option<Duration>>>;

    fn machine_watching_elements() -> (Engine, Shown, Counting) {
        let shown: Shown = Default::default();
        let counting: Counting = Default::default();
        let pipeline = Wrote {
            told: Default::default(),
            cameras: Default::default(),
            mics: Default::default(),
            played: Default::default(),
            levels: Default::default(),
            shown: shown.clone(),
            scene_events: Default::default(),
            counting: counting.clone(),
            published: Default::default(),
            recorded: Default::default(),
            mirrored: Default::default(),
            gated: Default::default(),
            heard_here: Default::default(),
            speaker_calls: Default::default(),
            previewed: Default::default(),
            ran_out: Default::default(),
            refuse: None,
            ducked: Default::default(),
        };
        (
            Engine::with_sources(Box::new(ThisMachine))
                .with_pipeline(Box::new(pipeline))
                .with_destination(Some(DESTINATION.into())),
            shown,
            counting,
        )
    }

    #[test]
    fn panic_clears_scene_and_stops_media() {
        let (mut engine, shown, _) = machine_watching_elements();
        engine.handle(Command::Screen { display: 3 });
        engine.handle(Command::HideEverything);
        assert_eq!(engine.status().active_scene, "default");
        assert!(elements_shown(&shown).last().is_some());
        assert!(engine.status().layers.is_empty());
        assert!(engine.status().muted);
    }

    // A bed that plays one file and then goes quiet is worse than no bed at
    // all: the panel still names the track, so nobody notices until somebody
    // watching says the stream went silent.
    // The panic button is pressed when somebody walks into the room. What it
    // has to guarantee is that nothing of yours reaches anybody, and half a
    // guarantee is not one.
    #[test]
    fn the_panic_button_leaves_nothing_running() {
        let (mut engine, played, _) = machine_with_music();
        engine.handle(Command::Genre { name: "edm".into() });
        engine.handle(Command::Mic {
            device: Some("HyperX".into()),
        });
        engine.handle(Command::Camera {
            device: Some("MacBook".into()),
        });
        engine.handle(Command::Monitor { on: true });

        engine.handle(Command::HideEverything);

        let now = engine.status();
        assert_eq!(now.active_scene, "default");
        assert!(now.layers.is_empty(), "the sources are off");
        assert!(now.muted, "the microphone is muted");
        assert_eq!(
            now.mic, None,
            "and closed: a muted microphone is still an open device"
        );
        assert_eq!(now.music, None, "the music stops");
        assert!(!now.monitoring, "the speakers stop");
        assert!(
            !now.music_to_stream,
            "and the bed stays off the stream until somebody says otherwise"
        );
        // It reached the pipeline and not only the status: a `None` at the end
        // of what was played is the mixer being told to stop.
        assert_eq!(played.lock().expect("played").last(), Some(&None));
    }

    // The engine that captures nothing is honest about it: nothing to hear, and
    // nothing to write, even with a folder to write into.
    #[test]
    fn an_engine_that_captures_nothing_refuses_the_speakers_and_the_file() {
        let mut engine = Engine::new().with_recordings(Some("/tmp/films".into()));
        assert_eq!(
            engine.status().record_dir.as_deref(),
            None,
            "read, never kept"
        );
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status answers with a status")
        };
        assert_eq!(status.record_dir.as_deref(), Some("/tmp/films"));
        let reply = engine.handle(Command::RecordStart);
        assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
        assert!(!engine.status().recording);
        let reply = engine.handle(Command::Monitor { on: true });
        assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
        assert!(!engine.status().monitoring);
    }

    #[test]
    fn an_engine_no_panel_has_spoken_to_runs_until_it_is_told() {
        let mut engine = engine();
        for _ in 0..(PANEL_LEASE_TICKS * 5) {
            engine.tick();
        }
        assert!(!engine.quitting(), "nothing told it to stop");
    }

    #[test]
    fn a_panel_that_stops_saying_it_is_here_stops_the_engine() {
        let mut engine = engine();
        assert_eq!(engine.handle(Command::Present), Reply::Ok);
        for _ in 0..(PANEL_LEASE_TICKS - 1) {
            engine.tick();
        }
        assert!(!engine.quitting(), "still within the lease");
        engine.tick();
        assert!(engine.quitting(), "the lease ran out");
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status answers with a status")
        };
        assert!(
            status
                .log
                .iter()
                .any(|line| line.contains("the panel went away")),
            "and the log says why: {:?}",
            status.log
        );
    }

    #[test]
    fn a_panel_that_keeps_saying_it_is_here_keeps_the_engine() {
        let mut engine = engine();
        for _ in 0..10 {
            engine.handle(Command::Present);
            for _ in 0..(PANEL_LEASE_TICKS - 5) {
                engine.tick();
            }
        }
        assert!(!engine.quitting());
    }

    #[test]
    fn saying_so_writes_nothing_in_the_log() {
        let mut engine = engine();
        engine.handle(Command::Present);
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status answers with a status")
        };
        assert!(
            status.log.is_empty(),
            "once a second would push everything else off: {:?}",
            status.log
        );
    }

    struct Replugged(std::sync::Arc<std::sync::atomic::AtomicU64>);

    impl Sources for Replugged {
        fn available(&self) -> Result<Available, String> {
            ThisMachine.available()
        }
        fn generation(&self) -> u64 {
            self.0.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    #[test]
    fn the_status_says_when_the_device_list_moved_so_a_face_reads_it_again() {
        let plugged = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let mut engine = Engine::with_sources(Box::new(Replugged(plugged.clone())));
        let Reply::Status(before) = engine.handle(Command::Status) else {
            panic!("status answers with a status")
        };
        assert_eq!(before.devices_generation, 0);
        plugged.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Reply::Status(after) = engine.handle(Command::Status) else {
            panic!("status answers with a status")
        };
        assert_eq!(after.devices_generation, 1, "a headset came back");
    }

    #[test]
    fn an_engine_with_nothing_plugged_in_never_moves_the_list() {
        let mut engine = engine();
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status answers with a status")
        };
        assert_eq!(status.devices_generation, 0);
    }

    fn followed() -> std::sync::Arc<std::sync::Mutex<crate::app::events::Events>> {
        Default::default()
    }

    fn said(
        events: &std::sync::Arc<std::sync::Mutex<crate::app::events::Events>>,
    ) -> Vec<crate::app::events::Event> {
        let events = events.lock().expect("events");
        events
            .since(0)
            .events
            .into_iter()
            .map(|e| e.event)
            .collect()
    }

    #[test]
    fn a_command_that_changed_what_the_feed_follows_is_an_event() {
        use crate::app::events::Event;
        let events = followed();
        let (engine, _) = publishing_engine(None);
        let mut engine = engine.with_events(std::sync::Arc::clone(&events));
        engine.handle(Command::Screen { display: 1 });
        let added = Event::LayerAdded {
            id: "source-1".into(),
            kind: "screen".into(),
        };
        assert_eq!(said(&events), vec![added.clone()], "a capture is a layer");
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        assert_eq!(engine.handle(Command::Stop), Reply::Ok);
        assert_eq!(
            said(&events),
            vec![added, Event::LiveStarted, Event::LiveEnded]
        );
    }

    #[test]
    fn what_changes_with_nobody_asking_is_an_event_on_the_tick() {
        use crate::app::events::Event;
        let events = followed();
        let (engine, published) = publishing_engine(None);
        let mut engine = engine.with_events(std::sync::Arc::clone(&events));
        engine.handle(Command::Screen { display: 1 });
        let from = events.lock().expect("events").last();
        let since = |events: &std::sync::Arc<std::sync::Mutex<crate::app::events::Events>>| {
            let events = events.lock().expect("events");
            events
                .since(from)
                .events
                .into_iter()
                .map(|e| e.event)
                .collect::<Vec<_>>()
        };
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        engine.tick();
        assert_eq!(since(&events), vec![Event::LiveStarted]);
        // The relay hung up.
        published.lock().expect("published").push(None);
        engine.tick();
        assert_eq!(since(&events), vec![Event::LiveStarted, Event::LiveEnded]);
    }

    #[test]
    fn an_event_carries_the_moment_it_happened() {
        let events = followed();
        let (engine, _) = publishing_engine(None);
        let mut engine = engine.with_events(std::sync::Arc::clone(&events));
        engine.handle(Command::Screen { display: 1 });
        let before = now();
        engine.handle(Command::GoLive);
        let events = events.lock().expect("events").since(0).events;
        let live = events.last().expect("the live");
        assert!(live.at >= before && live.at <= now());
    }

    #[test]
    fn a_face_asks_for_the_events_after_the_last_it_saw() {
        use crate::app::events::Event;
        let (engine, _) = publishing_engine(None);
        let mut engine = engine.with_events(followed());
        engine.handle(Command::Screen { display: 1 });
        engine.handle(Command::GoLive);
        engine.handle(Command::Stop);
        // 1 the screen added, 2 the live started, 3 the live ended.
        let Reply::Events { gap, events } = engine.handle(Command::Events {
            since: 2,
            follow: false,
        }) else {
            panic!("events answers with events")
        };
        assert_eq!(gap, None);
        assert_eq!(
            events.iter().map(|e| (e.seq, &e.event)).collect::<Vec<_>>(),
            [(3, &Event::LiveEnded)]
        );
    }

    /// An app whose reachability the test turns, and that counts how often
    /// the engine asked: the one question a snapshot puts to a port.
    #[derive(Clone, Default)]
    struct Counted {
        up: std::sync::Arc<std::sync::atomic::AtomicBool>,
        asked: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        rows: std::sync::Arc<std::sync::Mutex<Vec<Destination>>>,
        said: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Watching for Counted {
        fn reachable(&self) -> bool {
            use std::sync::atomic::Ordering;
            self.asked.fetch_add(1, Ordering::SeqCst);
            self.up.load(Ordering::SeqCst)
        }
        fn destinations(&self) -> Vec<Destination> {
            self.rows.lock().expect("rows").clone()
        }
        fn viewers(&self) -> Option<u32> {
            None
        }
        fn arm(&self, adapter: i64, on: bool) -> Result<(), String> {
            for row in self.rows.lock().expect("rows").iter_mut() {
                if row.id == adapter {
                    row.armed = on;
                }
            }
            Ok(())
        }
        fn notices(&mut self) -> Vec<String> {
            std::mem::take(&mut *self.said.lock().expect("said"))
        }
        fn sandbox(&self, _: i64, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn retitle(&self, _: i64, _: Option<&str>, _: Option<&str>) -> Result<(), String> {
            Ok(())
        }
        fn announce(&self, _: i64) -> Result<(), String> {
            Ok(())
        }
        fn disconnect(&self, _: i64) -> Result<(), String> {
            Ok(())
        }
        fn categorize(&self, _: i64, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn search_categories(&self, _: i64, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    // A panel's meter asks twelve times a second; a snapshot on either side
    // of each would be two dozen looks a second at what cannot have moved,
    // and a field that costs something to look at (a file, the motor) would
    // cost it that many times.
    #[test]
    fn a_read_looks_at_nothing_the_events_follow() {
        use std::sync::atomic::Ordering;
        let app = Counted::default();
        let mut engine = engine()
            .with_app(Box::new(app.clone()))
            .with_events(followed());
        engine.tick();
        app.asked.store(0, Ordering::SeqCst);
        for _ in 0..12 {
            engine.handle(Command::Levels);
            engine.handle(Command::Events {
                since: 0,
                follow: false,
            });
        }
        assert_eq!(app.asked.load(Ordering::SeqCst), 0);
    }

    // Whatever changed with no command to say so, or under one that was
    // taken for a read, is an event on the next tick, a quarter of a second
    // at most: compared with what was seen last, never lost.
    #[test]
    fn what_changed_under_a_read_is_an_event_on_the_next_tick() {
        use crate::app::events::Event;
        use std::sync::atomic::Ordering;
        let events = followed();
        let app = Counted::default();
        app.up.store(true, Ordering::SeqCst);
        let mut engine = engine()
            .with_app(Box::new(app.clone()))
            .with_events(std::sync::Arc::clone(&events));
        engine.tick();
        app.up.store(false, Ordering::SeqCst);
        engine.handle(Command::Levels);
        assert_eq!(said(&events), vec![]);
        engine.tick();
        assert_eq!(said(&events), vec![Event::AppReachable { on: false }]);
    }

    #[test]
    fn the_first_command_the_engine_ever_runs_is_an_event_too() {
        use crate::app::events::Event;
        let events = followed();
        let mut engine = engine().with_events(std::sync::Arc::clone(&events));
        engine.handle(Command::Mute { on: true });
        assert_eq!(said(&events), vec![Event::Muted { on: true }]);
    }

    fn a_room_with(
        body: &str,
    ) -> (
        std::sync::Arc<std::sync::Mutex<crate::app::chat::Feed>>,
        u64,
    ) {
        let feed: std::sync::Arc<std::sync::Mutex<crate::app::chat::Feed>> = Default::default();
        let seq = feed.lock().expect("feed").push(crate::app::wire::Line {
            id: "m1".into(),
            platform: "twitch".into(),
            channel: "kartths".into(),
            from: "ana".into(),
            body: body.into(),
        });
        (feed, seq)
    }

    #[test]
    fn a_line_hidden_or_deleted_is_taken_down_on_every_face_that_follows() {
        use crate::app::events::Event;
        for take in [|seq| Command::Hide { seq }, |seq| Command::Delete { seq }] {
            let events = followed();
            let (feed, seq) = a_room_with("spam");
            let mut engine = engine()
                .with_chat(feed)
                .with_events(std::sync::Arc::clone(&events));
            assert_eq!(engine.handle(take(seq)), Reply::Ok);
            assert_eq!(said(&events), vec![Event::ChatHidden { line: seq }]);
        }
    }

    #[test]
    fn a_line_the_engine_no_longer_has_is_not_taken_down() {
        use crate::app::events::Event;
        let events = followed();
        let (feed, seq) = a_room_with("spam");
        let mut engine = engine()
            .with_chat(feed)
            .with_events(std::sync::Arc::clone(&events));
        let Reply::Error { message } = engine.handle(Command::Delete { seq: seq + 40 }) else {
            panic!("no such line, so no")
        };
        assert_eq!(
            said(&events),
            vec![Event::Refused {
                verb: "delete".into(),
                message
            }],
            "the refusal, and no line taken down"
        );
    }

    #[test]
    fn a_refused_command_is_an_event_saying_why() {
        use crate::app::events::Event;
        let events = followed();
        let mut engine = engine().with_events(std::sync::Arc::clone(&events));
        let Reply::Error { message } = engine.handle(Command::GoLive) else {
            panic!("nothing to send, so no")
        };
        assert_eq!(
            said(&events),
            vec![Event::Refused {
                verb: "go-live".into(),
                message
            }]
        );
    }

    #[test]
    fn what_a_server_said_out_loud_is_an_event_on_the_tick() {
        use crate::app::events::Event;
        let events = followed();
        let app = Counted::default();
        let mut engine = engine()
            .with_app(Box::new(app.clone()))
            .with_events(std::sync::Arc::clone(&events));
        app.said
            .lock()
            .expect("said")
            .push("Twitch refused the title".into());
        engine.tick();
        assert_eq!(
            said(&events),
            vec![Event::Notice {
                text: "Twitch refused the title".into()
            }]
        );
    }

    #[test]
    fn arming_a_destination_is_an_event() {
        use crate::app::events::Event;
        let events = followed();
        let app = Counted::default();
        app.rows.lock().expect("rows").push(Destination {
            id: 2,
            name: "twitch".into(),
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
        });
        let mut engine = engine()
            .with_app(Box::new(app.clone()))
            .with_events(std::sync::Arc::clone(&events));
        assert_eq!(
            engine.handle(Command::Arm {
                adapter: 2,
                on: true
            }),
            Reply::Ok
        );
        assert_eq!(
            said(&events),
            vec![Event::DestinationArmed { id: 2, on: true }]
        );
    }

    #[test]
    fn a_timer_reaching_zero_is_an_event() {
        use crate::app::events::Event;
        use crate::picture::scenes::{Element, ElementContent};
        let events = followed();
        let mut engine = engine().with_events(std::sync::Arc::clone(&events));
        engine.handle(Command::SceneElementAdd {
            element: Element {
                id: "clock".into(),
                x: 600,
                y: 400,
                width: 700,
                height: 180,
                visible: true,
                shader: None,
                content: ElementContent::Timer { seconds: 0 },
            },
        });
        engine.handle(Command::SceneTimerStart { id: "clock".into() });
        engine.tick();
        assert!(
            said(&events).contains(&Event::TimerFinished { id: "clock".into() }),
            "got {:?}",
            said(&events)
        );
    }

    #[test]
    fn a_camera_whose_frames_stop_is_an_event_on_the_ticks() {
        use crate::app::events::Event;
        let events = followed();
        // The fake camera's count never moves past three.
        let (engine, _) = publishing_engine(None);
        let mut engine = engine.with_events(std::sync::Arc::clone(&events));
        engine.handle(Command::Camera {
            device: Some("HP".into()),
        });
        let id = engine.status().layers[0].id.clone();
        for _ in 0..=crate::picture::stall::Stalls::FLAT_TICKS {
            engine.tick();
        }
        assert!(
            said(&events).contains(&Event::LayerStalled { id }),
            "got {:?}",
            said(&events)
        );
    }

    #[test]
    fn a_fader_moved_is_an_event_on_the_detail_ring() {
        use crate::app::events::Event;
        let events = followed();
        let mut engine = engine().with_events(std::sync::Arc::clone(&events));
        engine.handle(Command::Volume { level: 0.5 });
        let faders = engine.status().faders;
        assert_eq!(
            said(&events),
            vec![Event::Faders {
                mic: 0.5,
                music: faders.music,
                duck_db: faders.duck_db
            }]
        );
    }
}
