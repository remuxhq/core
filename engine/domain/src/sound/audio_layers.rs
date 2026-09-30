//! Independent audio captures. These are not visual scene layers: each has
//! its own capture, gain, mute and stable ID, and many of each kind may coexist.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[schemars(rename = "AudioSourceKind")]
pub enum Kind {
    Mic,
    App,
    Screen,
}

impl Kind {
    /// Whether a sound of this kind steps back under the voice, as the music
    /// does. An application's or a screen's sound is what plays while one
    /// talks over it: a video, a game, a call. A microphone is somebody
    /// talking, and a voice ducked under another voice is a voice lost.
    pub fn ducks(self) -> bool {
        match self {
            Kind::Mic => false,
            Kind::App | Kind::Screen => true,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<u32>,
}
impl Source {
    pub fn mic(device: String) -> Self {
        Self {
            kind: Kind::Mic,
            device: Some(device),
            name: None,
            display: None,
        }
    }
    pub fn app(name: String) -> Self {
        Self {
            kind: Kind::App,
            device: None,
            name: Some(name),
            display: None,
        }
    }
    pub fn screen(display: u32) -> Self {
        Self {
            kind: Kind::Screen,
            device: None,
            name: None,
            display: Some(display),
        }
    }
    fn valid(&self) -> bool {
        match self.kind {
            Kind::Mic => {
                self.device.as_ref().is_some_and(|s| !s.trim().is_empty())
                    && self.name.is_none()
                    && self.display.is_none()
            }
            Kind::App => {
                self.name.as_ref().is_some_and(|s| !s.trim().is_empty())
                    && self.device.is_none()
                    && self.display.is_none()
            }
            Kind::Screen => self.display.is_some() && self.device.is_none() && self.name.is_none(),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn what_plays_under_the_voice_ducks_and_another_voice_does_not() {
        assert!(Kind::App.ducks());
        assert!(Kind::Screen.ducks());
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
        let mut mismatched = Source::screen(1);
        mismatched.device = Some("wrong".into());
        assert!(Layer::new("a".into(), mismatched).is_err());
    }
}
