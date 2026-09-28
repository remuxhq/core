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

use crate::gate::GateParams;
use crate::protocol::Faders;

/// The setup, as it was last left.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Remembered {
    /// Ordered scene sources and their independent layouts. Missing devices are skipped.
    #[serde(default)]
    pub layers: Vec<crate::layers::Layer>,
    #[serde(default)]
    pub scenes: Vec<crate::scenes::Scene>,
    #[serde(default = "crate::protocol::default_scene_name")]
    pub active_scene: String,
    #[serde(default)]
    pub audio_layers: Vec<crate::audio_layers::Layer>,
    #[serde(default)]
    pub mic: Option<String>,
    #[serde(default)]
    pub mirrored: bool,
    /// The genre, not the track: coming back to the same song is worse than
    /// coming back to the same shelf.
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub faders: Faders,
    #[serde(default)]
    pub gate: GateParams,
    #[serde(default)]
    pub monitoring: bool,
    #[serde(default)]
    pub denoise: bool,
}

impl Default for Remembered {
    fn default() -> Self {
        Self {
            layers: Vec::new(),
            scenes: Vec::new(),
            active_scene: crate::protocol::default_scene_name(),
            audio_layers: Vec::new(),
            mic: None,
            mirrored: false,
            genre: None,
            faders: Faders::default(),
            gate: GateParams::default(),
            monitoring: false,
            denoise: false,
        }
    }
}

/// What a saved file that will not parse should become.
///
/// An empty setup, never a refusal. A preferences file somebody edited by hand,
/// or one written by an older build, must not be the reason an engine will not
/// start: the worst it can cost is choosing a microphone once.
#[must_use]
pub fn read(said: &str) -> Remembered {
    let mut value: serde_json::Value = serde_json::from_str(said).unwrap_or_default();
    older_picture(&mut value);
    if let Some(scenes) = value.get_mut("scenes").and_then(|s| s.as_array_mut()) {
        scenes.retain(|scene| {
            let generated = matches!(
                scene
                    .get("graphic")
                    .and_then(|g| g.get("kind"))
                    .and_then(|k| k.as_str()),
                Some("starting-soon" | "back-in-a-moment" | "nothing-shared")
            );
            // Drop untouched generated presets; retain a scene with an actual
            // user layout, ignoring its obsolete graphic field during decode.
            !generated
                || scene
                    .get("layers")
                    .and_then(|v| v.as_array())
                    .is_some_and(|v| !v.is_empty())
                || scene
                    .get("elements")
                    .and_then(|v| v.as_array())
                    .is_some_and(|v| !v.is_empty())
                || scene.get("shader").is_some_and(|v| !v.is_null())
        });
    }
    serde_json::from_value(value).unwrap_or_default()
}

/// A file from before layers: one display and one camera, the camera in the
/// bottom-right corner at a quarter of the width, which is where the old
/// layout put it by default. Scenes kept then named a source and a layout,
/// not layers; they would come back as empty scenes, so they are dropped.
fn older_picture(value: &mut serde_json::Value) {
    let Some(file) = value.as_object_mut() else {
        return;
    };
    if let Some(scenes) = file.get_mut("scenes").and_then(|s| s.as_array_mut()) {
        scenes.retain(|scene| scene.get("shown").is_none());
    }
    if file.contains_key("layers") {
        return;
    }
    let mut layers = Vec::new();
    let layer = |id: &str, kind: &str, handle: String, transform: [i64; 4]| {
        serde_json::json!({
            "id": id,
            "source": { "kind": kind, "handle": handle, "name": handle, "width": 0, "height": 0 },
            "transform": {
                "x": transform[0], "y": transform[1],
                "width": transform[2], "height": transform[3], "degrees": 0
            },
        })
    };
    if let Some(display) = file.get("screen").and_then(serde_json::Value::as_u64) {
        layers.push(layer(
            "screen-1",
            "screen",
            display.to_string(),
            [0, 0, 1920, 1080],
        ));
    }
    if let Some(camera) = file.get("camera").and_then(serde_json::Value::as_str) {
        layers.push(layer(
            "camera-1",
            "camera",
            camera.to_string(),
            [1416, 786, 480, 270],
        ));
    }
    file.insert("layers".into(), serde_json::Value::Array(layers));
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
            layers: Vec::new(),
            scenes: Vec::new(),
            active_scene: crate::protocol::default_scene_name(),
            audio_layers: Vec::new(),
            mic: Some("HyperX DuoCast".into()),
            mirrored: true,
            denoise: false,
            genre: Some("lofi".into()),
            faders: Faders {
                mic: 1.5,
                music: 0.3,
                duck_db: -24.0,
            },
            gate: GateParams::default(),
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
        assert!(back.layers.is_empty());
        assert!(back.scenes.is_empty());
    }

    #[test]
    fn a_file_from_before_layers_keeps_its_display_and_camera() {
        let older = r#"{"screen":2,"camera":"FaceTime HD","layout":{"corner":"top-left"},
            "scenes":[{"name":"code","shown":{"kind":"screen","display":2},"mirrored":false}]}"#;
        let back = read(older);
        let kinds: Vec<_> = back
            .layers
            .iter()
            .map(|l| (l.id.as_str(), l.source.kind, l.source.handle.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("screen-1", crate::layers::Kind::Screen, "2"),
                ("camera-1", crate::layers::Kind::Camera, "FaceTime HD")
            ]
        );
        assert_eq!(back.layers[1].transform.x, 1416);
        assert!(
            back.scenes.is_empty(),
            "an old scene is not an empty new one"
        );
    }
}
