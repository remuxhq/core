//! The engine's memory: what it holds between commands.

use crate::protocol::Faders;

/// What the engine holds: the switches it was given and every scene in full.
/// Its own memory, never on the wire; a face reads the [`Status`] made of it
/// with what the motor and the app say now.
///
/// [`Status`]: crate::protocol::Status
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub on_air: bool,
    pub recording: bool,
    /// When the recording started, seconds past the epoch, while one is
    /// running. The clock is here rather than in each face because three faces
    /// starting their own the moment they connect would each show a different
    /// running time for the same file, and a panel opened mid-recording would
    /// show it starting from zero.
    pub recording_since: Option<i64>,
    /// When the live started, seconds past the epoch, while one is running.
    /// The same reason as `recording_since`: one clock, the engine's.
    pub on_air_since: Option<i64>,
    /// Scene layers, back to front. Their choices and layout survive a restart.
    pub layers: Vec<crate::picture::layers::Layer>,
    /// Named layouts; the active entry reflects the current layers.
    pub scenes: Vec<crate::picture::scenes::Scene>,
    pub active_scene: String,
    /// Independent audio captures, not ordered visual scene layers.
    pub audio_layers: Vec<crate::sound::audio_layers::Layer>,
    /// The selected scene shader, if any. Never its source.
    pub shader: Option<String>,
    pub mic: Option<String>,
    pub muted: bool,
    /// Whether the self-view is flipped. On the status rather than assumed,
    /// because it is a switch on the panel and a panel that cannot read its
    /// own switches back draws them wrong after anything else changes them.
    pub mirrored: bool,
    pub music: Option<String>,
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
    /// Whether you are hearing your own mix. On the status like every other
    /// switch, because a panel that kept its own copy would disagree with the
    /// terminal about whether the speakers are live, which is the one switch
    /// where that matters.
    pub monitoring: bool,
    /// Whether the music is in the mix that leaves. On unless somebody said
    /// otherwise: a bed nobody meant to keep off the air is the worse
    /// surprise. Hearing it is the other switch, `monitoring`, and the two are
    /// independent on purpose.
    pub music_to_stream: bool,
    /// Whether the microphone goes through the denoiser before the gate.
    pub denoise: bool,
}

impl Default for State {
    fn default() -> Self {
        // Written out rather than derived for one field: the music is in the
        // mix that leaves unless somebody said otherwise, and a derived
        // default said `false`. A face that keeps its own defaults
        // reads them from here, so a window drawn before the first status agrees.
        Self {
            on_air: false,
            recording: false,
            denoise: false,
            recording_since: None,
            on_air_since: None,
            layers: Vec::new(),
            scenes: crate::picture::scenes::defaults(),
            active_scene: crate::protocol::default_scene_name(),
            audio_layers: Vec::new(),
            shader: None,
            mic: None,
            muted: false,
            mirrored: false,
            music: None,
            faders: Faders::default(),
            gate: crate::sound::mixer::gate::GateParams::default(),
            monitoring: false,
            music_to_stream: true,
        }
    }
}
