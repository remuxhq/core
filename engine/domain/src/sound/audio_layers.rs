//! Independent audio captures. These are not visual scene layers: each has
//! its own capture, gain, mute and stable ID, and many of each kind may coexist.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[schemars(rename = "AudioSourceKind")]
pub enum Kind {
    Mic,
    App,
    /// What the computer plays, every application at once: the whole
    /// desktop's sound, never one display's, since no display has a sound.
    System,
}

impl Kind {
    /// Whether a sound of this kind steps back under the voice, as the music
    /// does. An application's or the system's sound is what plays while one
    /// talks over it: a video, a game, a call. A microphone is somebody
    /// talking, and a voice ducked under another voice is a voice lost.
    pub fn ducks(self) -> bool {
        match self {
            Kind::Mic => false,
            Kind::App | Kind::System => true,
        }
    }
}

/// One device per layer. The optional fields keep the protocol plain JSON and
/// Codable in Swift; `Layer::new` rejects mismatched or missing fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(rename = "AudioSource")]
pub struct Source {
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}
impl Source {
    pub fn mic(device: String) -> Self {
        Self {
            kind: Kind::Mic,
            device: Some(device),
            name: None,
        }
    }
    pub fn app(name: String) -> Self {
        Self {
            kind: Kind::App,
            device: None,
            name: Some(name),
        }
    }
    pub fn system() -> Self {
        Self {
            kind: Kind::System,
            device: None,
            name: None,
        }
    }
    fn valid(&self) -> bool {
        match self.kind {
            Kind::Mic => {
                self.device.as_ref().is_some_and(|s| !s.trim().is_empty()) && self.name.is_none()
            }
            Kind::App => {
                self.name.as_ref().is_some_and(|s| !s.trim().is_empty()) && self.device.is_none()
            }
            Kind::System => self.device.is_none() && self.name.is_none(),
        }
    }

    /// What it hears, for a person: `mic USB`, `app Discord`, `system`.
    pub fn said(&self) -> String {
        match self.kind {
            Kind::Mic => format!("mic {}", self.device.as_deref().unwrap_or_default()),
            Kind::App => format!("app {}", self.name.as_deref().unwrap_or_default()),
            Kind::System => "system".into(),
        }
    }
}

/// Whether one layer steps back under the voice: as its kind says, or as
/// the operator said. A call captured as an app is a voice to keep level
/// with one's own; a game captured as a screen is not.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[schemars(rename = "AudioLayerDuck")]
pub enum Duck {
    #[default]
    ByKind,
    On,
    Off,
}

impl Duck {
    pub fn by_kind(&self) -> bool {
        *self == Duck::ByKind
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(rename = "AudioLayer")]
pub struct Layer {
    pub id: String,
    pub source: Source,
    /// Linear gain, 0..=2.0. Mute is separate so a previous level survives.
    pub volume: f64,
    pub muted: bool,
    /// Left out while it is the kind's, so a layer reads as it did before
    /// a layer could be told.
    #[serde(default, skip_serializing_if = "Duck::by_kind")]
    pub duck: Duck,
}

impl Layer {
    pub fn new(id: String, source: Source) -> Result<Self, String> {
        if id.is_empty()
            || id.len() > 64
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("audio layer ID must be 1..64 ASCII letters, digits, - or _".into());
        }
        if !source.valid() {
            return Err("audio layer needs exactly one source of its declared kind".into());
        }
        Ok(Self {
            id,
            source,
            volume: 1.0,
            muted: false,
            duck: Duck::ByKind,
        })
    }

    /// Whether this layer steps back under the voice now.
    pub fn ducks(&self) -> bool {
        match self.duck {
            Duck::ByKind => self.source.kind.ducks(),
            Duck::On => true,
            Duck::Off => false,
        }
    }
}

impl Source {
    /// Whether two layers hear the same capture, whatever they are called:
    /// one microphone, one application (by name, as the motor finds it), or
    /// the system's sound.
    pub fn same_capture(&self, other: &Source) -> bool {
        self.kind == other.kind
            && self.device == other.device
            && match (&self.name, &other.name) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (a, b) => a == b,
            }
    }
}

/// What a scene switch does to the audio layers: the captures both scenes
/// hear stay open (under the new scene's ID and levels), the new scene's
/// others open, the old scene's others close. As the pictures' captures do.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Transition {
    /// The old layer's ID, and what it becomes.
    pub keep: Vec<(String, Layer)>,
    pub open: Vec<Layer>,
    /// The old layers' IDs.
    pub close: Vec<String>,
}

/// One old layer to each new one at most: a layer under the same ID and
/// capture first, then any with the capture, so two layers on one
/// application in both scenes stay two.
pub fn transition(from: &[Layer], to: &[Layer]) -> Transition {
    let mut taken = vec![false; from.len()];
    let mut kept: Vec<Option<usize>> = vec![None; to.len()];
    for (next, layer) in to.iter().enumerate() {
        kept[next] = from
            .iter()
            .position(|old| old.id == layer.id && old.source.same_capture(&layer.source));
        if let Some(at) = kept[next] {
            taken[at] = true;
        }
    }
    for (next, layer) in to.iter().enumerate() {
        if kept[next].is_none() {
            kept[next] = (0..from.len())
                .find(|&at| !taken[at] && from[at].source.same_capture(&layer.source));
            if let Some(at) = kept[next] {
                taken[at] = true;
            }
        }
    }
    let mut plan = Transition::default();
    for (layer, old) in to.iter().zip(kept) {
        match old {
            Some(at) => plan.keep.push((from[at].id.clone(), layer.clone())),
            None => plan.open.push(layer.clone()),
        }
    }
    plan.close = from
        .iter()
        .zip(taken)
        .filter(|(_, taken)| !taken)
        .map(|(old, _)| old.id.clone())
        .collect();
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str) -> Layer {
        Layer::new(id.into(), Source::app(name.into())).unwrap()
    }

    #[test]
    fn a_switch_keeps_the_captures_both_scenes_hear_whatever_they_are_called() {
        let from = [app("call", "Discord"), app("video", "Safari")];
        let to = [app("guest", "discord"), app("game", "Steam")];
        let plan = transition(&from, &to);
        assert_eq!(plan.keep, [("call".to_string(), to[0].clone())]);
        assert_eq!(plan.open, [to[1].clone()]);
        assert_eq!(plan.close, ["video"]);
    }

    #[test]
    fn a_switch_pairs_each_capture_once_and_its_own_id_first() {
        let from = [app("a", "Safari"), app("b", "Safari")];
        let to = [app("b", "Safari"), app("c", "Safari"), app("d", "Safari")];
        let plan = transition(&from, &to);
        assert_eq!(
            plan.keep,
            [
                ("b".to_string(), to[0].clone()),
                ("a".to_string(), to[1].clone())
            ]
        );
        assert_eq!(plan.open, [to[2].clone()]);
        assert!(plan.close.is_empty());
    }

    #[test]
    fn one_id_on_another_capture_is_a_close_and_an_open() {
        let plan = transition(&[app("call", "Discord")], &[app("call", "Zoom")]);
        assert!(plan.keep.is_empty());
        assert_eq!(plan.open, [app("call", "Zoom")]);
        assert_eq!(plan.close, ["call"]);
        let mic = Layer::new("guest".into(), Source::mic("USB".into())).unwrap();
        let system = Layer::new("guest".into(), Source::system()).unwrap();
        assert!(!mic.source.same_capture(&system.source));
        assert!(system.source.same_capture(&Source::system()));
    }

    #[test]
    fn what_plays_under_the_voice_ducks_and_another_voice_does_not() {
        assert!(Kind::App.ducks());
        assert!(Kind::System.ducks());
        assert!(!Kind::Mic.ducks(), "a second microphone is a voice");
    }
    #[test]
    fn a_layer_ducks_by_its_kind_until_told() {
        let mut call = Layer::new("call".into(), Source::app("Discord".into())).unwrap();
        assert!(call.ducks());
        call.duck = Duck::Off;
        assert!(!call.ducks(), "a call kept level with the voice");
        let mut guest = Layer::new("guest".into(), Source::mic("USB".into())).unwrap();
        assert!(!guest.ducks());
        guest.duck = Duck::On;
        assert!(guest.ducks());
    }
    #[test]
    fn a_layer_left_to_its_kind_reads_as_it_did_before() {
        let layer = Layer::new("call".into(), Source::app("Discord".into())).unwrap();
        let said = serde_json::to_string(&layer).unwrap();
        assert_eq!(
            said,
            r#"{"id":"call","source":{"kind":"app","name":"Discord"},"volume":1.0,"muted":false}"#
        );
        assert_eq!(serde_json::from_str::<Layer>(&said).unwrap(), layer);
        let mut told = layer;
        told.duck = Duck::Off;
        let said = serde_json::to_string(&told).unwrap();
        assert!(said.ends_with(r#""muted":false,"duck":"off"}"#), "{said}");
        assert_eq!(serde_json::from_str::<Layer>(&said).unwrap(), told);
    }
    #[test]
    fn layer_ids_and_sources_are_safe() {
        assert!(Layer::new("voice_2".into(), Source::mic("a".into())).is_ok());
        assert!(Layer::new("../bad".into(), Source::mic("a".into())).is_err());
        assert!(Layer::new("a".into(), Source::app(" ".into())).is_err());
        let mut mismatched = Source::system();
        mismatched.device = Some("wrong".into());
        assert!(Layer::new("a".into(), mismatched).is_err());
    }
}
