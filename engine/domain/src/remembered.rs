//! What survives a restart.
//!
//! An engine that forgets which microphone you use is an engine you set up
//! every single time you sit down, and setting up is the part nobody wants to
//! do twice. OBS remembers, and this (`prefs.json`) is the same list: what is in the picture, what it sounds like, and
//! where the gate is tuned to your room.
//!
//! **Choices, never state.** Whether it is on air is not here, and neither is
//! whether it is recording: an engine that came up publishing because it was
//! publishing when the machine slept is an engine that goes live in an empty
//! room. What is here is what a person chose and would choose again.
//!
//! Nor is anything a credential. The destination and the recording folder come
//! from the environment the daemon was started with; a file a client could
//! reach has no business holding a stream key.

use serde::{Deserialize, Serialize};

use crate::card::Words;
use crate::gate::GateParams;
use crate::protocol::Faders;

/// The setup, as it was last left.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Remembered {
    /// The display id, not its place in a list: the capturer and the display
    /// list order the same hardware differently, and an index would come back
    /// pointing at the other monitor.
    #[serde(default)]
    pub screen: Option<u32>,
    /// By name, because a camera's id is not stable across reboots on macOS
    /// while what is written on it is.
    #[serde(default)]
    pub camera: Option<String>,
    #[serde(default)]
    pub mic: Option<String>,
    #[serde(default)]
    pub mirrored: bool,
    #[serde(default)]
    pub layout: crate::scene::Layout,
    #[serde(default)]
    pub scenes: Vec<crate::scenes::Scene>,
    /// The genre, not the track: coming back to the same song is worse than
    /// coming back to the same shelf.
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub faders: Faders,
    #[serde(default)]
    pub gate: GateParams,
    #[serde(default)]
    pub words: Words,
    #[serde(default)]
    pub monitoring: bool,
    #[serde(default)]
    pub denoise: bool,
}

/// What a saved file that will not parse should become.
///
/// An empty setup, never a refusal. A preferences file somebody edited by hand,
/// or one written by an older build, must not be the reason an engine will not
/// start: the worst it can cost is choosing a microphone once.
#[must_use]
pub fn read(said: &str) -> Remembered {
    serde_json::from_str(said).unwrap_or_default()
}

/// The setup as a line of JSON, or nothing when it cannot be written, which is
/// not a failure worth stopping anything for.
#[must_use]
pub fn write(remembered: &Remembered) -> Option<String> {
    serde_json::to_string_pretty(remembered).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_written_is_what_comes_back() {
        let setup = Remembered {
            screen: Some(3),
            camera: Some("MacBook Pro Camera".into()),
            mic: Some("HyperX DuoCast".into()),
            mirrored: true,
            layout: crate::scene::Layout::default(),
            scenes: Vec::new(),
            denoise: false,
            genre: Some("lofi".into()),
            faders: Faders {
                mic: 1.5,
                music: 0.3,
                duck_db: -24.0,
            },
            gate: GateParams::default(),
            words: Words::default(),
            monitoring: false,
        };
        assert_eq!(read(&write(&setup).expect("written")), setup);
    }

    #[test]
    fn a_file_that_will_not_parse_costs_a_setup_and_not_a_start() {
        assert_eq!(read("{ this is not json"), Remembered::default());
        assert_eq!(read(""), Remembered::default());
    }

    // A file written by an older build has fields this one does not know and is
    // missing fields this one has. Neither may throw the rest away.
    #[test]
    fn an_older_file_keeps_what_it_does_have() {
        let older = r#"{"mic":"HyperX DuoCast","something_removed":7}"#;
        let back = read(older);
        assert_eq!(back.mic.as_deref(), Some("HyperX DuoCast"));
        assert_eq!(back.camera, None);
    }
}
