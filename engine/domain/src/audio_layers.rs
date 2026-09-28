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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(rename = "AudioLayer")]
pub struct Layer {
    pub id: String,
    pub source: Source,
    /// Linear gain, 0..=2.0. Mute is separate so a previous level survives.
    pub volume: f64,
    pub muted: bool,
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
