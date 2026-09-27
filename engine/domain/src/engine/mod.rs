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
    Card, Command, Destination, Devices, Flowing, Found, Framed, Grant, Hearing, Mixing, Named,
    Reply, Status,
};
use crate::sources::{pick, pick_device, DisplayId, Screen, Window, WindowId};
use std::time::Duration;

mod air;
mod app;
mod picture;
mod sound;

pub use air::Air;
pub use picture::Picture;
pub use sound::Sound;

/// How long the countdown runs when nobody says otherwise. Three minutes is
/// what a person needs to sit down, and long enough that somebody arriving
/// early sees a number that is worth waiting out.
pub const DEFAULT_COUNTDOWN_SECONDS: u32 = 180;

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
    /// Black, with "No content shared" written on it. Not an error: it is what
    /// the switch on the panel does, and the live has to keep flowing through
    /// it or the viewer sees a frozen frame instead of a deliberate blank.
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
pub struct NoPipeline {
    behind: Behind,
}

impl Picture for NoPipeline {
    fn capture(&mut self, behind: Behind) -> Result<(), String> {
        self.behind = behind;
        Ok(())
    }
    fn camera(&mut self, _device: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    fn show(
        &mut self,
        _card: Card,
        _line: &str,
        _counting: Option<Duration>,
    ) -> Result<(), String> {
        Ok(())
    }
    fn stop(&mut self) {
        self.behind = Behind::Nothing;
    }
    fn flowing(&self) -> Flowing {
        Flowing::default()
    }
    fn mirror(&mut self, _on: bool) {}
    fn shot(&mut self, _of: Framed) -> Option<(Vec<u8>, u32, u32)> {
        None
    }
    fn camera_flowing(&self) -> Flowing {
        Flowing::default()
    }
}

impl Sound for NoPipeline {
    fn mic(&mut self, _device: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    fn play(&mut self, _track: Option<&Track>) -> Result<(), String> {
        Ok(())
    }
    fn levels(&mut self, _: f64, _: f64, _: f64, _: bool, _: bool, _: bool) -> Result<(), String> {
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
    /// Whether the engine's own "No content shared" is up.
    ///
    /// Its own field because the status deliberately does not carry it: the
    /// status says what the *operator* chose, and reading it back to decide
    /// whether to take this down means the two requirements contradict each
    /// other. They did, and a test found it.
    blank_up: bool,
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
    /// Which display was chosen, so a restart can choose it again. See
    /// [`crate::remembered`].
    chosen_display: Option<u32>,
    /// The words that found the window behind the picture, when one is:
    /// what a scene keeps, since a window's id is new every time it opens.
    chosen_window: Option<String>,
    scenes: Vec<crate::scenes::Scene>,
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
            blank_up: false,
            quitting: false,
            sources,
            watching: Box::new(NoApp),
            chat: Default::default(),
            pipeline: Box::new(NoPipeline::default()),
            library: Box::new(NoLibrary),
            playing: None,
            destination: None,
            journal: crate::journal::Journal::default(),
            chosen_display: None,
            chosen_window: None,
            scenes: Vec::new(),
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
        crate::remembered::Remembered {
            screen: self.chosen_display,
            camera: self.status.camera.clone(),
            mic: self.status.mic.clone(),
            mirrored: self.status.mirrored,
            layout: self.status.layout,
            scenes: self.scenes.clone(),
            genre: self.playing.as_ref().map(|(genre, _)| genre.clone()),
            faders: self.status.faders,
            gate: self.status.gate,
            words: self.status.words.clone(),
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
        self.status.words = setup.words.clone();
        self.status.faders = setup.faders;
        let _ = self.handle(Command::Gate {
            patch: serde_json::to_value(setup.gate).unwrap_or_default(),
        });
        let _ = self.sound();
        if let Some(display) = setup.screen {
            let _ = self.handle(Command::Screen { display });
        }
        if let Some(camera) = &setup.camera {
            let _ = self.handle(Command::Camera {
                device: Some(camera.clone()),
            });
        }
        if let Some(mic) = &setup.mic {
            let _ = self.handle(Command::Mic {
                device: Some(mic.clone()),
            });
        }
        if setup.mirrored {
            let _ = self.handle(Command::Mirror { on: true });
        }
        self.scenes = setup.scenes.clone();
        self.status.scenes = self.scenes.iter().map(|s| s.name.clone()).collect();
        if setup.layout != crate::scene::Layout::default() {
            let _ = self.handle(Command::Layout {
                patch: crate::scene::LayoutPatch {
                    mode: Some(setup.layout.mode),
                    corner: Some(setup.layout.corner),
                    share: Some(setup.layout.share),
                    margin: Some(setup.layout.margin),
                    shape: Some(setup.layout.shape),
                    filter: Some(setup.layout.filter),
                },
            });
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
            Command::Screen { display } => self.choose_screen(display),
            Command::Window { query } => self.choose_window(query),
            Command::Camera { device } => self.choose_camera(device),
            Command::Mic { device } => self.choose_mic(device),
            Command::Share { on } => self.share(on),
            Command::Genre { name } => self.pick_genre(&name),
            Command::NextTrack => self.next_track(),
            Command::Music { on } => self.music(on),
            Command::Card { which } => self.show(which),
            Command::Countdown { seconds } => self.countdown(seconds),
            Command::CardText { which, text } => self.card_text(which, text),
            Command::HideEverything => self.hide_everything(),
            Command::Gate { patch } => self.gate(patch),
            Command::Denoise { on } => self.denoise(on),
            Command::Hear { apps } => self.hear(apps),
            Command::Clip { name } => self.play_clip(&name),
            Command::Shot { of } => self.shot(of),
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
            Command::Layout { patch } => self.layout(patch),
            Command::SceneSave { name } => self.scene_save(name),
            Command::SceneSwitch { name } => self.scene_switch(&name),
            Command::SceneForget { name } => self.scene_forget(&name),
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
        status.flowing = self.pipeline.flowing();
        status.camera_flowing = self.pipeline.camera_flowing();
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
        // The card first, so there is never a frame between the screen going
        // and something being there to show.
        let line = self.status.words.line(Card::BackInAMoment).to_string();
        if let Err(why) = self.pipeline.show(Card::BackInAMoment, &line, None) {
            return Reply::Error { message: why };
        }
        self.status.card = Some(Card::BackInAMoment);
        self.blank_up = false;

        if let Err(why) = self.pipeline.camera(None) {
            return Reply::Error { message: why };
        }
        self.status.camera = None;

        if let Err(why) = self.pipeline.capture(Behind::Nothing) {
            return Reply::Error { message: why };
        }
        self.status.screen = None;

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
