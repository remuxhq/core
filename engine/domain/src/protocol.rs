//! The contract between the engine and everything that drives it.
//!
//! A graphical face and the `remux` CLI speak this. Neither shares a language with the daemon and that is the
//! point, so the wire format is newline-delimited JSON over a unix socket:
//! anything can speak it, and `nc -U` can debug it at three in the morning.
//!
//! The whole command surface is declared here, because a wire format is the
//! expensive thing to change once three clients read it. Every command is
//! answered; what the engine cannot carry out comes back as [`Reply::Error`]
//! with a sentence, not a crash.

use serde::{Deserialize, Serialize};

/// How many lines of chat the engine keeps, and the app beside it. A face
/// that keeps its own copy caps it at the same number.
pub const CHAT_LINES: usize = 1000;

/// What a client asks the engine to do.
///
/// A CLI scene layer command is one hop from
/// `{"cmd":"screen","display":1}` and a bug report can be pasted
/// straight into `nc`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Command {
    // ---- what is going on -------------------------------------------------
    Status,
    /// A new scene with nothing in it, switched to: its layers are added
    /// there. On the air the picture is empty until they are.
    SceneCreate {
        name: String,
    },
    /// A copy of the active scene (its layers, elements, order and filter),
    /// made active: the picture that goes out does not change.
    SceneDuplicate {
        name: String,
    },
    SceneSwitch {
        name: String,
    },
    SceneDelete {
        name: String,
    },
    AudioLayerAdd {
        id: String,
        source: crate::sound::audio_layers::Source,
    },
    AudioLayerRemove {
        id: String,
    },
    AudioLayerVolume {
        id: String,
        volume: f64,
    },
    AudioLayerMute {
        id: String,
        on: bool,
    },
    /// Whether one audio layer steps back under the voice.
    AudioLayerDuck {
        id: String,
        duck: crate::sound::audio_layers::Duck,
    },
    /// Whether a window is drawing the preview.
    ///
    /// The engine renders the preview into shared memory on a clock, and that
    /// costs about two points of a core whether or not anybody has the region
    /// mapped. A panel says so when a window that draws opens and says so again
    /// when the last one closes, exactly the way it already does for the
    /// meters, and an engine nobody is looking at costs what it did before
    /// there was a preview at all.
    ///
    /// Counted, not a flag: two windows can be drawing and the first to close
    /// must not stop the second.
    Watching {
        on: bool,
    },
    /// A panel saying it is here, once a second while it runs.
    ///
    /// The engine and the panel are tied by this and by nothing about who
    /// started whom: an engine that has heard it and then stops hearing it
    /// for five seconds stops itself, camera and microphone released, the
    /// way the panel's own Quit would have. A panel closed by any door, or
    /// one that crashed, leaves no daemon holding a camera with its light on.
    /// An engine that never heard it runs until told: the CLI's engine.
    Present,
    /// Only the meters, so a panel can draw them at the rate they move.
    ///
    /// Its own verb rather than the whole status because a meter wants twelve
    /// answers a second and the status is a wide read: it clones the
    /// destination list, asks the app for the viewer count and formats two
    /// hundred log lines. A meter on a once-a-second feed is what made the
    /// window this replaces feel dead, and asking twelve times a second for
    /// everything is how you make the engine feel it.
    Levels,
    /// Everything capturable right now: screens, windows, cameras, mics,
    /// apps, and the music's genres.
    Sources,

    // ---- the two levers ---------------------------------------------------
    GoLive,
    /// What Go live would do, with a fingerprint to confirm it by.
    Plan,
    /// Go live only if the plan still has this fingerprint: nothing moved
    /// since it was printed and confirmed.
    Live {
        plan: u64,
    },
    Stop,
    RecordStart,
    RecordStop,

    // ---- what is in the picture -------------------------------------------
    /// A screen by its display id, never by its position in a list: the
    /// capturer and the display list order the same hardware differently.
    Screen {
        display: u32,
    },
    /// A window by part of its title, the way the CLI has always taken it.
    Window {
        query: String,
    },
    Camera {
        device: Option<String>,
    },
    /// Add a camera layer under a caller-chosen ID.
    LayerCamera {
        id: String,
        device: String,
    },
    /// Add a display by its stable display id, with no reserved layer id.
    LayerScreen {
        id: String,
        display: u32,
    },
    /// Add one window, matched by title at selection time.
    LayerWindow {
        id: String,
        query: String,
    },
    /// Add a picture file, by its absolute path.
    LayerImage {
        id: String,
        path: String,
    },
    /// Replace the source of this layer without changing its ID or order.
    LayerReplaceScreen {
        id: String,
        display: u32,
    },
    LayerReplaceWindow {
        id: String,
        query: String,
    },
    LayerReplaceCamera {
        id: String,
        device: String,
    },
    LayerReplaceImage {
        id: String,
        path: String,
    },
    /// Hide/show the layer's video without closing its capture.
    LayerVisible {
        id: String,
        on: bool,
    },
    LayerRemove {
        id: String,
    },
    /// Zero is the bottommost layer.
    LayerMove {
        id: String,
        index: usize,
    },
    LayerTransform {
        id: String,
        transform: crate::picture::layers::Transform,
    },
    /// Cut a source in its native pixels; null restores the entire source.
    LayerCrop {
        id: String,
        crop: Option<crate::picture::layers::Crop>,
    },
    /// A camera's mask, independent of every other camera in the scene.
    LayerShape {
        id: String,
        shape: crate::picture::scene::CameraShape,
    },
    LayerMirror {
        id: String,
        on: bool,
    },
    /// Move a camera viewport without changing its size or rotation.
    /// `None` restores the layer's original native position (0,0).
    LayerPosition {
        id: String,
        at: Option<crate::picture::scene::CameraPosition>,
    },
    /// Top-left pixel of the camera viewport on the 1920x1080 scene.
    /// `None` restores the default corner.
    CameraPosition {
        at: Option<crate::picture::scene::CameraPosition>,
    },
    /// Choose the composited camera's crop without changing its device or position.
    CameraShape {
        shape: crate::picture::scene::CameraShape,
    },
    Mirror {
        on: bool,
    },
    /// Load a WGSL filter over the entire scene, or clear it.
    Shader {
        path: Option<String>,
    },
    /// Apply a WGSL filter to a layer's own pixels before composition.
    LayerShader {
        id: String,
        path: Option<String>,
    },
    /// Remove the single selected screen/window from the current scene.
    Share {
        on: bool,
    },

    // ---- what it sounds like ----------------------------------------------
    Mic {
        device: Option<String>,
    },
    Mute {
        on: bool,
    },
    /// Microphone fader, 0 to 1.
    Volume {
        level: f64,
    },
    /// Music fader, 0 to 1.
    MusicVolume {
        level: f64,
    },
    /// How far the music steps under the voice, in dB below unity.
    Duck {
        db: f64,
    },
    /// Hearing your own mix. The mic never joins it: it would come back
    /// through itself.
    Monitor {
        on: bool,
    },
    /// Whether the music reaches the audience. Hearing it is not optional
    /// (starting the music opens the speakers); sending it out is.
    StreamMusic {
        on: bool,
    },
    /// Whether the screen's own sound (what the computer is playing) joins the
    /// mix that leaves. Off unless asked: a call, a notification or a video
    /// nobody meant to share is the worse surprise.
    ScreenSound {
        on: bool,
    },
    /// Select one display layer as the only source of system audio.
    LayerScreenSound {
        id: String,
        on: bool,
    },
    /// Capture one running application's sound, independently of screen sound.
    /// None closes the dedicated capture. Names must match a running app.
    AppAudio {
        app: Option<String>,
    },
    /// Dedicated application audio fader, 0 to 1.
    AppAudioVolume {
        level: f64,
    },
    Music {
        on: bool,
    },
    Genre {
        name: String,
    },
    NextTrack,
    /// One sound once, over the mix: a file, or a name in the clips folder.
    Clip {
        name: String,
    },
    /// Whose sound the screen's sound is: these applications' alone, or
    /// everything the computer plays when the list is empty.
    Hear {
        apps: Vec<String>,
    },
    /// The room out of the microphone (RNNoise), or as it comes.
    Denoise {
        on: bool,
    },
    Gate {
        patch: serde_json::Value,
    },

    // ---- scene content ----------------------------------------------------
    SceneElementAdd {
        element: crate::picture::scenes::Element,
    },
    SceneElementSet {
        element: crate::picture::scenes::Element,
    },
    SceneElementRemove {
        id: String,
    },
    SceneTimerStart {
        id: String,
    },
    SceneTimerStop {
        id: String,
    },
    /// Everything off but the music: the panic button.
    HideEverything,

    // ---- where it goes ----------------------------------------------------
    /// What a destination is told the live is called. The app's column, like
    /// `armed`: asked of the app and read back on the row, never kept here.
    /// `None` leaves that field as it is.
    Retitle {
        adapter: i64,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
    Arm {
        adapter: i64,
        on: bool,
    },
    /// Put one destination in the sandbox, or take it out: nobody is told
    /// while it is in. The app's column, asked the way `arm` is.
    Sandbox {
        adapter: i64,
        on: bool,
    },
    /// Forget one destination's platform account: the tokens, who it was, and
    /// what connecting filled in. The destination stays, disconnected.
    Disconnect {
        adapter: i64,
    },
    /// Tell one destination's platform the title now, without waiting for
    /// the next Go live: OBS's "Update". A refusal comes back as a notice.
    Announce {
        adapter: i64,
    },

    /// A picture a panel can draw, small.
    ///
    /// Of the whole scene by default, or of one source: a floating self-view
    /// wants the camera alone, cropped and flipped the way the scene shows it,
    /// and not the composed frame with itself in the corner.
    Shot {
        #[serde(default)]
        of: Framed,
    },
    /// A source-only preview of precisely this layer, independent of scene order.
    LayerShot {
        id: String,
    },
    /// What the OS is letting this engine do.
    Grants,
    /// The last of the chat, as the engine has it off its wire
    /// (`remuxd_domain::app::chat`): what was said after the line numbered
    /// `since`, zero for everything. With `follow`, the daemon answers once
    /// and then keeps the connection, pushing a `chat` reply for every line
    /// after it, until the client hangs up.
    Chat {
        #[serde(default)]
        since: u64,
        #[serde(default)]
        follow: bool,
    },
    /// What changed in the engine (`remuxd_domain::app::events`): the events
    /// after the one numbered `since`, zero for all that is held. With
    /// `follow`, the daemon answers once and then keeps the connection,
    /// pushing an `events` reply whenever more happen, until the client hangs
    /// up.
    Events {
        #[serde(default)]
        since: u64,
        #[serde(default)]
        follow: bool,
    },
    /// Take one line of chat off every face for the rest of the run: a
    /// spammer's, before the overlay on the stream shows it any longer. The
    /// app keeps the conversation as it was said; this is the engine's own
    /// list of what not to hand out.
    Hide {
        seq: u64,
    },
    /// File one destination's live under a category: Twitch's game, YouTube's
    /// video category. The app's column, asked the way the title is; told to
    /// the platform at go live beside the title.
    Categorize {
        adapter: i64,
        id: String,
        name: String,
    },
    /// Ask where one destination's live can be filed, for what was typed. The
    /// app answers with a push and the answer lands on the status
    /// (`Status::categories`), so every face reads the same list.
    Categories {
        adapter: i64,
        query: String,
    },
    /// Take one line of chat out of the platform's chat, for everybody who is
    /// watching there, as the broadcaster moderating their own room; and off
    /// every face here at once, like `Hide`. The app does the asking; the
    /// engine finds the line by its number and hands over the message's id.
    Delete {
        seq: u64,
    },
    /// The config changed where the chat comes from (`remux chat --url`):
    /// the daemon drops its wire and opens what the config says now. A live
    /// is untouched.
    Rewire,
    Quit,
}

/// What the engine answers. One reply per command, always.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "reply", rename_all = "kebab-case")]
pub enum Reply {
    /// Just the two meters. See [`Command::Levels`].
    Levels {
        hearing: Hearing,
        mixing: Mixing,
    },
    Ok,
    /// Boxed, and only because of its size: a `Reply` that carried a `Status`
    /// inline would make every other answer, including `Ok`, three hundred
    /// bytes wide. It serialises exactly the same, so the wire does not know.
    Status(Box<Status>),
    Plan(crate::air::plan::Plan),
    Sources(Devices),
    Error {
        message: String,
    },
    /// One frame of what is going out, as JPEG in base64.
    ///
    /// Base64 because this wire is newline-delimited JSON and stays that way:
    /// a second, binary channel would be a second thing to get right in three
    /// clients for a picture that is forty kilobytes once a second. The cost
    /// is a third more bytes, which is nothing beside what it is a picture of.
    Shot {
        jpeg: String,
        width: u32,
        height: u32,
    },
    /// The chat, and whether the wire it comes down is up. The second is not
    /// decoration: an empty list from a source that is down and an empty
    /// list from a quiet room look identical and are not.
    Chat {
        reachable: bool,
        lines: Vec<ChatLine>,
    },
    /// What changed, oldest first, and what a face asked for and can no
    /// longer have, which a status makes whole. See [`Command::Events`].
    Events {
        gap: Option<crate::app::events::Gap>,
        events: Vec<crate::app::events::Numbered>,
    },
    /// The three permissions, as the OS has them right now.
    ///
    /// Asked of the engine and not of whoever is asking, because the grants
    /// belong to the process that opens the devices: a panel that checked its
    /// own would report on a process that captures nothing, and be wrong in
    /// the most confusing possible way.
    Grants {
        screen: Grant,
        camera: Grant,
        microphone: Grant,
    },
}

/// What a `Shot` is a picture of.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Framed {
    /// Everything, as it is going out: captures and visual elements.
    #[default]
    Scene,
    /// The camera alone, in the shape and the flip its slot gives it. A
    /// self-view is what a person checks their own face in, and a person
    /// checking their face in a mirror expects a mirror.
    Camera,
    /// The screen alone, whatever is on air.
    ///
    /// Its own picture: the scene preview may have visual elements
    /// while the operator checks a screen independently.
    Screen,
}

/// What the OS says about one permission (an OS with nothing to ask answers granted).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Grant {
    /// Never asked. Opening one will put a dialog on the screen. What a
    /// face assumes before the engine has answered, for the same reason.
    #[default]
    NotAsked,
    Granted,
    /// Refused, or forbidden by policy. Opening will fail and no dialog will
    /// appear: the only cure is System Settings, which is why a panel that
    /// says "denied" has to say where to go as well.
    Refused,
}

/// What is actually coming out of the capture, measured rather than assumed.
///
/// A client showing "on air" from a flag alone will keep showing it after the
/// capture has silently stopped. A frame count that stops going up is the only
/// thing that tells the truth about that.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct Flowing {
    /// What the capture handed over. This rises only when the picture changes,
    /// because that is all ScreenCaptureKit ever reports: measured on a Mac
    /// with two displays, the busy one gave 373 frames in five seconds and the
    /// idle one gave exactly one. A still screen is not a broken capture.
    pub captured: u64,
    /// What the engine produced. This rises at the output rate whatever the
    /// screen is doing, because a live that stops sending frames is
    /// indistinguishable from a live that broke, and a repeated frame costs
    /// almost nothing in H264: there is no difference to encode.
    ///
    /// It does not restart when the operator changes monitors, switches scenes
    /// or hides everything. The capture restarts; the stream does not.
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    /// The rate the device is held at, when the engine could hold it: a
    /// camera with a mode at the picture's size runs it at the picture's
    /// rate and never its own fastest (`remuxd_domain::camera`). None for
    /// the screen, and for a camera whose modes offered nothing to hold,
    /// which keeps whatever the preset chose.
    #[serde(default)]
    pub held: Option<u32>,
}

/// What the microphone is doing.
///
/// The level is measured *before* the gate, which is the only useful place for
/// it: a meter reading the gated signal sits at the floor and tells nobody
/// where to put the threshold. The gate's own state is reported beside it, so
/// a panel can draw both the sound arriving and the decision made about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Hearing {
    /// Samples the system has handed over. A microphone reports silence, so
    /// this rises whether anybody speaks or not: it stopping is the signal.
    pub samples: u64,
    pub level_db: f64,
    /// The peak, held and decaying, for the marker that sits on the bar.
    ///
    /// Two numbers and not one, because they are two things a person reads at
    /// once: where the sound is now and where it has just been. Reporting only
    /// the held one, which this used to, makes the *bar* move like a peak
    /// meter and a person setting a gate cannot see the signal at all.
    pub peak_db: f64,
    pub gate_open: bool,
    /// What each of the gate's two detectors is hearing, as amplitudes.
    ///
    /// On the wire because the panel draws them: a threshold with nothing to
    /// compare it against is a number somebody guesses at, and the whole point
    /// of the two thin bars beside the sliders is seeing the room and the line
    /// at the same time.
    pub gate_levels: crate::sound::mixer::gate::GateLevels,
    /// What the gate is doing to the level right now, in dB.
    pub gain_db: f64,
    /// Blocks the mixer made with less voice than a block holds, because the
    /// microphone had not delivered it yet. Each is a hole of up to ten
    /// milliseconds in the voice, which is a syllable's end gone; a device
    /// that arrives in bursts, a Bluetooth headset, is what makes them.
    #[serde(default)]
    pub starved: u64,
    /// Frames of voice waiting in the ring for the mixer. Held near forty
    /// milliseconds by the motor's drift compensation, whatever clock the device runs on;
    /// a number that climbs is a device the mixer is not keeping up with.
    #[serde(default)]
    pub buffered: u64,
    /// Samples the ring threw away because it was full. Zero, or the voice
    /// is crackling: this is the number that says so.
    #[serde(default)]
    pub dropped: u64,
    /// Samples of the screen's own sound the capture has handed over, sent
    /// or not. Zero with the screen shared is a capture that delivers no
    /// sound, which is a fact about the system and not about the switch.
    #[serde(default)]
    pub screen_samples: u64,
    /// Why the screen's sound is not being read, when it is not: the format
    /// the capture delivered, in a sentence, the way `complaint` is for the
    /// microphone.
    #[serde(default)]
    pub screen_complaint: Option<String>,
    /// Samples and latest capture failure from the dedicated app stream.
    #[serde(default)]
    pub app_samples: u64,
    #[serde(default)]
    pub app_complaint: Option<String>,
    /// Set when a device speaks a format this does not understand. A meter
    /// sitting at zero with no explanation sends a person looking at a cable.
    pub complaint: Option<String>,
}

impl Default for Hearing {
    fn default() -> Self {
        // Silence is the floor, not zero. Zero dBFS is the loudest sound there
        // is, and a panel drawing a meter from a derived default painted it
        // full red for a microphone that was not even open.
        Self {
            samples: 0,
            level_db: crate::sound::mixer::levels::Meter::FLOOR_DB,
            peak_db: crate::sound::mixer::levels::Meter::FLOOR_DB,
            gate_open: false,
            gate_levels: crate::sound::mixer::gate::GateLevels::default(),
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

/// The one stream of sound leaving the engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Mixing {
    /// Samples mixed. It rises whether anything is making noise or not: this
    /// is a clock, and it stopping is the signal.
    pub frames: u64,
    /// Everything together, in dBFS.
    pub level_db: f64,
    /// Its held peak, for the marker. See [`Hearing::peak_db`].
    pub peak_db: f64,
    /// The music as you hear it: after its fader and after any duck, whether
    /// or not it is in the mix that leaves. The panel's Level row.
    pub music_db: f64,
    pub music_peak_db: f64,
    /// The music in the mix that leaves: the same as `music_db` while it is
    /// sent, the floor while it is kept off the air. The panel's Stream row,
    /// and the two walk together, which is what a person setting a bed by ear
    /// expects: what you hear is what the live hears.
    #[serde(default = "floor")]
    pub music_out_db: f64,
    #[serde(default = "floor")]
    pub music_out_peak_db: f64,
    /// Dedicated app bus after its fader, in dBFS.
    #[serde(default = "floor")]
    pub app_db: f64,
    pub playing: bool,
    /// How far the duck has the bed stepped back right now, in dB at or
    /// below zero: zero while nobody is talking, easing toward the depth
    /// while somebody is. The one number that says the duck is acting.
    #[serde(default)]
    pub ducked_db: f64,
    /// What is going to the speakers, in dBFS, or the floor when nothing is.
    ///
    /// Its own reading and not the mix's, because the two are deliberately
    /// different: the microphone is in one and never in the other. A person
    /// checking that this is what they think it is can watch this stay at the
    /// floor while they talk, which is the property, and watch it move with
    /// the music, which is the point.
    pub monitor_db: f64,
}

impl Default for Mixing {
    fn default() -> Self {
        // Same reason as `Hearing`: nothing playing is the floor.
        Self {
            frames: 0,
            level_db: crate::sound::mixer::levels::Meter::FLOOR_DB,
            peak_db: crate::sound::mixer::levels::Meter::FLOOR_DB,
            music_db: crate::sound::mixer::levels::Meter::FLOOR_DB,
            music_peak_db: crate::sound::mixer::levels::Meter::FLOOR_DB,
            music_out_db: crate::sound::mixer::levels::Meter::FLOOR_DB,
            music_out_peak_db: crate::sound::mixer::levels::Meter::FLOOR_DB,
            app_db: crate::sound::mixer::levels::Meter::FLOOR_DB,
            playing: false,
            ducked_db: 0.0,
            monitor_db: crate::sound::mixer::levels::Meter::FLOOR_DB,
        }
    }
}

/// What is actually leaving, measured at the muxer.
///
/// Zero everywhere when nothing is going out, which is what a panel draws as a
/// dash rather than as a number. There is no packet loss on purpose: this path
/// is RTMP over TCP, where a lost packet is a retransmitted one, and the
/// browser studio only has that figure because WebRTC's `getStats` reports it.
/// A zero here would read as "measured, and fine".
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
pub struct Outgoing {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub video_kbps: u32,
    pub audio_kbps: u32,
}

impl Outgoing {
    /// The most pictures a second worth reporting.
    ///
    /// A stretch too short to measure divides by almost nothing, and the
    /// number that comes out is thousands of frames a second on a panel that
    /// has no way to know better.
    pub const MOST_FPS: u32 = 240;

    /// What left in the last stretch, from the writer's counters.
    ///
    /// The counters are reset on each tick rather than averaged over the
    /// whole stream: a bitrate that only ever goes down for an hour is a
    /// number nobody can use. Keeps the size the caller already knows.
    pub fn counted(
        &self,
        pictures: u32,
        video_bytes: usize,
        audio_bytes: usize,
        over_seconds: f64,
    ) -> Outgoing {
        Outgoing {
            width: self.width,
            height: self.height,
            fps: per_second(f64::from(pictures), over_seconds)
                .round()
                .min(f64::from(Self::MOST_FPS)) as u32,
            video_kbps: kbps(video_bytes, over_seconds),
            audio_kbps: kbps(audio_bytes, over_seconds),
        }
    }
}

/// Kilobits a second, from bytes over a stretch.
pub fn kbps(bytes: usize, over_seconds: f64) -> u32 {
    (per_second(bytes as f64 * 8.0, over_seconds) / 1000.0).round() as u32
}

/// Anything a second, and nothing at all over no time: a stretch of zero is a
/// tick that has not happened rather than an infinite rate.
fn per_second(what: f64, over_seconds: f64) -> f64 {
    if over_seconds <= 0.0 {
        return 0.0;
    }
    what / over_seconds
}

/// A serde default for a switch that starts on.
fn yes() -> bool {
    true
}

pub fn default_scene_name() -> String {
    "default".into()
}

fn floor() -> f64 {
    crate::sound::mixer::levels::Meter::FLOOR_DB
}

fn yes_volume() -> f64 {
    1.0
}

/// Everything a client needs to draw the state of the engine in one line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Status {
    pub on_air: bool,
    pub recording: bool,
    /// When the recording started, seconds past the epoch, while one is
    /// running. The clock is here rather than in each face because three faces
    /// starting their own the moment they connect would each show a different
    /// running time for the same file, and a panel opened mid-recording would
    /// show it starting from zero.
    #[serde(default)]
    pub recording_since: Option<i64>,
    /// When the live started, seconds past the epoch, while one is running.
    /// The same reason as `recording_since`: one clock, the engine's.
    #[serde(default)]
    pub on_air_since: Option<i64>,
    /// Scene layers, back to front. Their choices and layout survive a restart.
    #[serde(default)]
    pub layers: Vec<crate::picture::layers::Layer>,
    /// Named layouts; the active entry reflects the current layers.
    #[serde(default)]
    pub scenes: Vec<crate::picture::scenes::Scene>,
    #[serde(default = "default_scene_name")]
    pub active_scene: String,
    /// Independent audio captures, not ordered visual scene layers.
    #[serde(default)]
    pub audio_layers: Vec<crate::sound::audio_layers::Layer>,
    /// The selected scene shader, if any. Never its source.
    #[serde(default)]
    pub shader: Option<String>,
    pub mic: Option<String>,
    pub muted: bool,
    /// Whether the self-view is flipped. On the status rather than assumed,
    /// because it is a switch on the panel and a panel that cannot read its
    /// own switches back draws them wrong after anything else changes them.
    pub mirrored: bool,
    pub music: Option<String>,
    /// The scene's frames, drawn at the full rate whether or not anything is
    /// in it; the plan, not the count, keeps an empty scene off the air.
    pub scene_flowing: Flowing,
    /// Capture measurements keyed by layer ID; no arbitrary first source.
    #[serde(default)]
    pub layer_flowing: std::collections::BTreeMap<String, Flowing>,
    pub hearing: Hearing,
    pub mixing: Mixing,
    /// Summed across the destinations that answered. A destination that could
    /// not be asked is left out, never counted as zero.
    pub viewers: Option<u32>,
    /// The most watching at once since the live began, summed like `viewers`.
    /// The platforms give the number now and never the peak; the app keeps
    /// it while a broadcast is open. `None` off air or before anyone answered.
    #[serde(default)]
    pub viewers_peak: Option<u32>,
    /// Where the faders are, so a panel can draw them where they are.
    ///
    /// Without this a panel has to keep its own copy, and then two faces
    /// looking at one engine disagree about the volume within a minute: the
    /// one that did not move it goes on drawing the old position forever. Same
    /// reason `muted` and `mirrored` are here.
    pub faders: Faders,
    /// Where the gate's thresholds are. On the status for the same reason as
    /// everything else here: a panel with five sliders on it has to be able to
    /// draw them where they are, and the engine is what knows.
    pub gate: crate::sound::mixer::gate::GateParams,
    /// Whether the app is reachable. See [`Reply::Chat`] for why it travels
    /// beside the list rather than being inferred from it.
    pub app: bool,
    /// The engine's own version, so a face and a script know who answers.
    #[serde(default)]
    pub version: String,
    /// Which motor is behind the ports, by name and version: `obs 30.2.3`.
    #[serde(default)]
    pub motor: String,
    /// Whether you are hearing your own mix. On the status like every other
    /// switch, because a panel that kept its own copy would disagree with the
    /// terminal about whether the speakers are live, which is the one switch
    /// where that matters.
    pub monitoring: bool,
    /// Where the bed is playing while `monitoring` is on: the system's output
    /// device by name, so a person with a headset and a pair of speakers
    /// knows which one is live without opening System Settings. `None` while
    /// the speakers are closed.
    #[serde(default)]
    pub speakers: Option<String>,
    /// Whether the music is in the mix that leaves. On unless somebody said
    /// otherwise: a bed nobody meant to keep off the air is the worse
    /// surprise. Hearing it is the other switch, `monitoring`, and the two are
    /// independent on purpose.
    #[serde(default = "yes")]
    pub music_to_stream: bool,
    /// Whether screen sound was requested. Off by default; while its display
    /// layer is hidden, the mixer gates it out and resumes it on show.
    #[serde(default)]
    pub screen_sound: bool,
    /// Whether the microphone goes through the denoiser before the gate.
    #[serde(default)]
    pub denoise: bool,
    /// The applications whose sound is heard; empty is the whole screen's.
    #[serde(default)]
    pub hearing_apps: Vec<String>,
    /// The display layer supplying system audio; never more than one.
    #[serde(default)]
    pub screen_sound_layer: Option<String>,
    /// Dedicated application capture, independent of the screen's sound.
    #[serde(default)]
    pub app_audio: Option<String>,
    /// Its fader. Kept when the source is off.
    #[serde(default = "yes_volume")]
    pub app_audio_volume: f64,
    /// Where recordings are written, as the engine was started with. `None`
    /// is an engine that can capture, mix and publish but cannot record, which
    /// is a real configuration and not a broken one.
    #[serde(default)]
    pub record_dir: Option<String>,
    /// Which app this engine repeats, so a settings window can say which one
    /// rather than making somebody read a process listing to find out.
    #[serde(default)]
    pub server: Option<String>,
    /// Where a panel can read the preview without asking for it. `None` on an
    /// engine that could not make the region, and a client that sees `None`
    /// falls back to [`Command::Shot`]. See [`crate::picture::preview`].
    #[serde(default)]
    pub preview: Option<crate::picture::preview::Preview>,
    /// What is leaving, measured where it leaves. See [`Outgoing`].
    #[serde(default)]
    pub outgoing: Outgoing,
    /// What the engine has done, newest last, stamped. See
    /// [`crate::air::journal`]. Empty on a fresh engine and never long: two hundred
    /// lines is as far back as anybody reads while something is wrong.
    #[serde(default)]
    pub log: Vec<String>,
    /// What the app says the destinations are. Repeated, never held: `armed`
    /// is the web's column and this engine is not its second home.
    pub destinations: Vec<Destination>,
    /// The last answer to [`Command::Categories`]: which destination, what was
    /// typed, what the platform offers. `None` until somebody asks.
    #[serde(default)]
    pub categories: Option<Found>,
    /// Moves when a device is plugged in or pulled out. A face keeps the one
    /// it last read the device list at and asks again when this differs;
    /// see [`crate::engine::Sources::generation`].
    #[serde(default)]
    pub devices_generation: u64,
}

impl Default for Status {
    fn default() -> Self {
        // Written out rather than derived for one field: the music is in the
        // mix that leaves unless somebody said otherwise, and a derived
        // default said `false`. A face that keeps its own defaults
        // reads them from here, so a window drawn before the first status agrees.
        Self {
            on_air: false,
            recording: false,
            denoise: false,
            version: String::new(),
            motor: String::new(),
            hearing_apps: Vec::new(),
            categories: None,
            viewers_peak: None,
            recording_since: None,
            on_air_since: None,
            layers: Vec::new(),
            scenes: crate::picture::scenes::defaults(),
            active_scene: default_scene_name(),
            audio_layers: Vec::new(),
            shader: None,
            mic: None,
            muted: false,
            mirrored: false,
            music: None,
            scene_flowing: Flowing::default(),
            layer_flowing: std::collections::BTreeMap::new(),
            hearing: Hearing::default(),
            mixing: Mixing::default(),
            viewers: None,
            faders: Faders::default(),
            gate: crate::sound::mixer::gate::GateParams::default(),
            app: false,
            monitoring: false,
            speakers: None,
            music_to_stream: true,
            screen_sound: false,
            screen_sound_layer: None,
            app_audio: None,
            app_audio_volume: 1.0,
            record_dir: None,
            server: None,
            preview: None,
            outgoing: Outgoing::default(),
            log: Vec::new(),
            destinations: Vec::new(),
            devices_generation: 0,
        }
    }
}

/// A destination as the app reports it. Its own copy of nothing: the engine
/// holds no destinations, it repeats what the app said, and `armed` is the
/// web's column.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Destination {
    pub id: i64,
    pub name: String,
    pub platform: String,
    /// `off`, `starting` or `live`, as the relay reports it.
    pub status: String,
    pub armed: bool,
    /// A rehearsal with the real pipes and no audience: Twitch takes the
    /// stream as a bandwidth test (channel offline, the Inspector shows it),
    /// YouTube's broadcast is unlisted. The app's column, like `armed`.
    #[serde(default)]
    pub sandbox: bool,
    /// Whether there is somewhere to push: the ingest URL and the key are
    /// both there, from the account or by hand. Without them going live is
    /// refused, so a face says "not connected" instead of "ready".
    #[serde(default)]
    pub connected: bool,
    /// The platform account it is connected as, when it is: the name a face
    /// shows beside "connected", and what Disconnect forgets. `None` by hand
    /// or not connected.
    #[serde(default)]
    pub account: Option<String>,
    /// What the platform files the live under: Twitch's game, YouTube's video
    /// category. The name a face shows and the id the API takes; the app's
    /// columns, carried on the row like the title.
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub category_id: Option<String>,
    /// How many are watching this one, when the platform answered. `None` is
    /// "nobody told us" and is drawn as the status instead of as a zero: Twitch
    /// lists nothing until it has recognised the stream as live, and YouTube
    /// omits the count entirely when the channel hides it.
    #[serde(default)]
    pub viewers: Option<u32>,
    /// This destination's most at once since the live began. See `viewers`.
    #[serde(default)]
    pub viewers_peak: Option<u32>,
    /// What this destination's platform last refused, as the app worded it
    /// ("youtube said 403 quotaExceeded"), until a call to it succeeds. The
    /// first public live ran the YouTube quota out two hours in, the chat
    /// stopped and the viewer count went quiet, and no face said why.
    #[serde(default)]
    pub trouble: Option<String>,
    /// What this destination is told the live is called, and the line under
    /// it. The app's columns, carried on the row so a panel can show them
    /// without a second place to ask.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Where a viewer would watch it, when the app said. The link a panel
    /// offers next to the row.
    #[serde(default)]
    pub channel: Option<String>,
}

/// One place a live can be filed under: the id the platform takes and the
/// name a person reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Category {
    pub id: String,
    pub name: String,
}

/// The answer to a category search, whole, so a face can tell which question
/// it answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Found {
    pub adapter: i64,
    pub query: String,
    pub items: Vec<Category>,
}

/// One line of chat, from whichever platform said it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChatLine {
    /// Where this line sits in the order, minted by the engine and rising.
    /// It is what a face asks `since` with, and what keeps a row the same row
    /// when the one before it drops off the ring: identity by position
    /// shifted the whole list under whoever was reading.
    #[serde(default)]
    pub seq: u64,
    pub from: String,
    pub body: String,
    pub platform: String,
    /// The message's id on its platform, which is what deleting it there
    /// takes. The rehearsal's lines have ids of their own and nobody to tell.
    #[serde(default)]
    pub id: String,
    /// The destination it came from, by name: which account gets to delete it.
    #[serde(default)]
    pub channel: String,
}

/// The three things a hand can be on, as positions rather than as sound.
///
/// `mic` and `music` are fader travel, 0 to 1, except that the microphone goes
/// to 2: the panel's slider is marked 0 to 200% and means it. `duck_db` is how
/// far the music steps under a voice and is negative, because it is a step
/// down.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Faders {
    pub mic: f64,
    pub music: f64,
    pub duck_db: f64,
}

impl Default for Faders {
    fn default() -> Self {
        // The same places the engine starts them, so a panel drawn before the
        // first status is drawn right rather than at zero.
        Self {
            mic: 1.0,
            music: 0.85,
            duck_db: crate::sound::music::DUCK_DEFAULT_DB,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct Devices {
    pub screens: Vec<Named>,
    pub windows: Vec<Named>,
    pub cameras: Vec<Named>,
    pub mics: Vec<Named>,
    /// The running applications: what `hear` and `audio app` pick sound from.
    #[serde(default)]
    pub apps: Vec<Named>,
    /// The music folders, by the name a person picks them with.
    ///
    /// Here rather than in a reply of its own because this is the "what can I
    /// choose" question and a panel asks it once: a folder of genres and a
    /// list of cameras are both things on the machine that the operator picks
    /// between. Without it a panel could play music and could not offer any.
    pub genres: Vec<Named>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Named {
    pub id: String,
    pub name: String,
}

/// One line of the wire. Newline-delimited JSON, so a partial write is a
/// partial line and never a half-parsed command.
pub fn encode<T: Serialize>(message: &T) -> String {
    let mut line = serde_json::to_string(message)
        .unwrap_or_else(|e| format!(r#"{{"reply":"error","message":"could not encode: {e}"}}"#));
    line.push('\n');
    line
}

/// A line from a client, which is untrusted input like any other boundary.
pub fn decode(line: &str) -> Result<Command, String> {
    serde_json::from_str(line).map_err(|e| e.to_string())
}
/// Read what the engine said. The other direction of [`decode`], for the
/// clients: the daemon never needs this and the CLI, the terminal and the app
/// all do.
pub fn decode_reply(line: &str) -> Result<Reply, String> {
    serde_json::from_str(line.trim()).map_err(|e| format!("not a reply: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_left_in_the_last_second_is_counted_over_that_second_alone() {
        let size = Outgoing {
            width: 1920,
            height: 1080,
            ..Outgoing::default()
        };
        // 30 pictures and 750 kB of them in a second: 6 Mbps, the encoder's
        // target.
        let counted = size.counted(30, 750_000, 16_000, 1.0);
        assert_eq!(counted.fps, 30);
        assert_eq!(counted.video_kbps, 6000);
        assert_eq!(counted.audio_kbps, 128);
        assert_eq!(
            (counted.width, counted.height),
            (1920, 1080),
            "the size is not something a tick measures"
        );
    }

    #[test]
    fn a_tick_that_ran_long_is_counted_over_how_long_it_actually_ran() {
        let counted = Outgoing::default().counted(60, 1_500_000, 32_000, 2.0);
        assert_eq!(counted.fps, 30);
        assert_eq!(counted.video_kbps, 6000);
    }

    #[test]
    fn a_stretch_of_no_time_reads_as_nothing_rather_than_as_thousands() {
        let counted = Outgoing::default().counted(3, 90_000, 2_000, 0.0);
        assert_eq!(counted.fps, 0);
        assert_eq!(counted.video_kbps, 0);
        assert_eq!(counted.audio_kbps, 0);
    }

    #[test]
    fn a_burst_is_capped_at_a_rate_a_face_can_believe() {
        // A hundredth of a second between ticks: without the cap this reads
        // as three hundred frames a second on the panel.
        let counted = Outgoing::default().counted(3, 0, 0, 0.01);
        assert_eq!(counted.fps, Outgoing::MOST_FPS);
    }

    // These are contract tests, not round-trip tests. Three clients read this
    // wire format and only one of them is written in this language, so the
    // exact bytes matter and a rename that serde would happily carry across a
    // round trip has to fail here instead.
    // What can be captured is asked as `sources`; the old `devices` is gone
    // from the wire, both ways.
    #[test]
    fn what_can_be_captured_is_asked_as_sources_and_devices_is_gone() {
        assert_eq!(encode(&Command::Sources), "{\"cmd\":\"sources\"}\n");
        assert_eq!(decode("{\"cmd\":\"sources\"}"), Ok(Command::Sources));
        assert!(decode("{\"cmd\":\"devices\"}").is_err());
        let lists =
            "\"screens\":[],\"windows\":[],\"cameras\":[],\"mics\":[],\"apps\":[],\"genres\":[]";
        assert_eq!(
            encode(&Reply::Sources(Devices::default())),
            format!("{{\"reply\":\"sources\",{lists}}}\n")
        );
        assert!(decode_reply(&format!("{{\"reply\":\"devices\",{lists}}}")).is_err());
    }

    #[test]
    fn the_verbs_go_out_on_the_wire_under_the_names_the_cli_already_uses() {
        assert_eq!(encode(&Command::GoLive), "{\"cmd\":\"go-live\"}\n");
        assert_eq!(encode(&Command::Status), "{\"cmd\":\"status\"}\n");
        assert_eq!(
            encode(&Command::AppAudio {
                app: Some("Safari".into())
            }),
            "{\"cmd\":\"app-audio\",\"app\":\"Safari\"}\n"
        );
        assert_eq!(
            encode(&Command::Screen { display: 7 }),
            "{\"cmd\":\"screen\",\"display\":7}\n"
        );
        assert_eq!(
            encode(&Command::Window {
                query: "tmux".into()
            }),
            "{\"cmd\":\"window\",\"query\":\"tmux\"}\n"
        );
        assert_eq!(
            encode(&Command::LayerImage {
                id: "logo".into(),
                path: "/tmp/logo.png".into()
            }),
            "{\"cmd\":\"layer-image\",\"id\":\"logo\",\"path\":\"/tmp/logo.png\"}\n"
        );
        assert_eq!(
            encode(&Command::CameraPosition {
                at: Some(crate::picture::scene::CameraPosition { x: 300, y: 200 })
            }),
            "{\"cmd\":\"camera-position\",\"at\":{\"x\":300,\"y\":200}}\n"
        );
        assert_eq!(
            encode(&Command::CameraShape {
                shape: crate::picture::scene::CameraShape::Rectangle
            }),
            "{\"cmd\":\"camera-shape\",\"shape\":\"rectangle\"}\n"
        );
        assert_eq!(
            encode(&Command::SceneSwitch {
                name: "Studio".into()
            }),
            "{\"cmd\":\"scene-switch\",\"name\":\"Studio\"}\n"
        );
    }

    #[test]
    fn a_command_from_a_client_is_parsed_or_refused_with_a_reason() {
        assert_eq!(decode("{\"cmd\":\"stop\"}"), Ok(Command::Stop));
        assert_eq!(
            decode("{\"cmd\":\"volume\",\"level\":0.5}"),
            Ok(Command::Volume { level: 0.5 })
        );
        assert!(decode("{\"cmd\":\"fly\"}").is_err());
        assert!(decode("not json at all").is_err());
        // A known verb with the wrong shape is refused, not defaulted.
        assert!(decode("{\"cmd\":\"screen\"}").is_err());
    }

    #[test]
    fn every_line_ends_in_a_newline_so_a_short_write_is_never_half_a_command() {
        for line in [encode(&Command::Quit), encode(&Reply::Ok)] {
            assert!(line.ends_with('\n'));
            assert_eq!(line.matches('\n').count(), 1, "no newline inside a line");
        }
    }

    #[test]
    fn an_absent_device_is_absent_rather_than_an_empty_name() {
        assert_eq!(
            encode(&Command::Camera { device: None }),
            "{\"cmd\":\"camera\",\"device\":null}\n"
        );
        assert_eq!(
            decode("{\"cmd\":\"camera\",\"device\":null}"),
            Ok(Command::Camera { device: None })
        );
    }

    #[test]
    fn independent_audio_layers_are_labeled_in_status_json() {
        let mut status = Status::default();
        status.audio_layers.push(
            crate::sound::audio_layers::Layer::new(
                "call".into(),
                crate::sound::audio_layers::Source::app("Zoom".into()),
            )
            .unwrap(),
        );
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(
            json["audio_layers"][0]["source"],
            serde_json::json!({"kind":"app", "name":"Zoom"})
        );
        assert_eq!(json["audio_layers"][0]["id"], "call");
        assert_eq!(serde_json::from_value::<Status>(json).unwrap(), status);
    }

    #[test]
    fn status_carries_nothing_it_does_not_know() {
        let status = Status::default();
        assert_eq!(status.viewers, None, "nobody asked is not nobody watching");
        assert!(!status.on_air);
        let json = serde_json::to_value(&status).expect("status encodes");
        assert_eq!(json["viewers"], serde_json::Value::Null);
        assert_eq!(json["layers"], serde_json::json!([]));
        assert_eq!(json["audio_layers"], serde_json::json!([]));
        assert_eq!(json["layer_flowing"], serde_json::json!({}));
        assert!(json.get("camera_shape").is_none());
        assert!(json.get("screen").is_none());
    }

    #[test]
    fn the_events_are_asked_for_and_answered_in_these_bytes() {
        use crate::app::events::{Event, Gap, Numbered};
        assert_eq!(
            encode(&Command::Events {
                since: 4,
                follow: true
            }),
            "{\"cmd\":\"events\",\"since\":4,\"follow\":true}\n"
        );
        assert_eq!(
            decode("{\"cmd\":\"events\"}"),
            Ok(Command::Events {
                since: 0,
                follow: false
            }),
            "a face that says nothing more asks for everything, once"
        );
        assert_eq!(
            encode(&Reply::Events {
                gap: None,
                events: vec![Numbered {
                    seq: 5,
                    at: 1_700_000_000,
                    event: Event::SceneSwitched {
                        name: "code".into()
                    },
                }],
            }),
            "{\"reply\":\"events\",\"gap\":null,\"events\":[{\"seq\":5,\"at\":1700000000,\"event\":\"scene-switched\",\"name\":\"code\"}]}\n"
        );
        assert_eq!(
            encode(&Reply::Events {
                gap: Some(Gap { from: 3, to: 4 }),
                events: vec![],
            }),
            "{\"reply\":\"events\",\"gap\":{\"from\":3,\"to\":4},\"events\":[]}\n"
        );
    }
}
