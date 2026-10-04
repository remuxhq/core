//! A bench: a setup as one JSON, to hand to another machine (`remux bench`).
//!
//! The scenes with their layers, elements and filters; the audio layers, the
//! gate and the faders; the companions list. Never a key, a token, the
//! destinations, the chat's URL or the session: nothing here reads them. The
//! files a scene names (pictures, filters) travel beside the JSON, under
//! `files/`, and the CLI packs and unpacks them; this module decides which
//! files, where they land, and which layers this machine cannot open.

use serde::{Deserialize, Serialize};

use crate::companions::Companion;
use crate::picture::layers::Kind;
use crate::picture::scenes::Scene;
use crate::protocol::{Devices, Faders};
use crate::sound::audio_layers::Layer as AudioLayer;
use crate::sound::mixer::gate::GateParams;

/// The bench format's version.
pub const VERSION: u32 = 1;

/// What a bench was made on: a bench made on one system says so, and another
/// can still read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requires {
    pub os: String,
    pub motor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bench {
    pub bench: u32,
    pub requires: Requires,
    pub scenes: Vec<Scene>,
    #[serde(default)]
    pub audio_layers: Vec<AudioLayer>,
    pub gate: GateParams,
    pub faders: Faders,
    #[serde(default)]
    pub companions: Vec<Companion>,
}

/// Every file the scenes name, once each, in the order they are met: an
/// image layer's picture and every filter.
pub fn files(scenes: &[Scene]) -> Vec<String> {
    let mut named = Vec::new();
    for scene in scenes {
        let pictures = scene
            .layers
            .iter()
            .filter(|l| l.source.kind == Kind::Image)
            .map(|l| &l.source.handle);
        let filters = scene
            .layers
            .iter()
            .filter_map(|l| l.shader.as_ref())
            .chain(scene.elements.iter().filter_map(|e| e.shader.as_ref()))
            .chain(scene.shader.as_ref());
        for path in pictures.chain(filters) {
            if !named.contains(path) {
                named.push(path.clone());
            }
        }
    }
    named
}

/// The name a file travels under: `files/`, its place in [`files`], and its
/// own name, so that two `logo.png` from two folders stay two.
pub fn packed(at: usize, path: &str) -> String {
    let base = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    format!("files/{at}-{base}")
}

/// The scenes with every path they name moved by `to`: to the packed names on
/// export, to where they were unpacked on import.
pub fn relocate(scenes: &mut [Scene], to: impl Fn(&str) -> String) {
    for scene in scenes {
        for layer in &mut scene.layers {
            if layer.source.kind == Kind::Image {
                layer.source.handle = to(&layer.source.handle);
            }
            if let Some(shader) = &mut layer.shader {
                *shader = to(shader);
            }
        }
        for element in &mut scene.elements {
            if let Some(shader) = &mut element.shader {
                *shader = to(shader);
            }
        }
        if let Some(shader) = &mut scene.shader {
            *shader = to(shader);
        }
    }
}

/// The layers this machine has nothing for, hidden, by id: a camera, a
/// display or a window the bench names that is not here. The operator picks
/// theirs. A picture travels with the bench, so it is always here.
pub fn hide_missing(scene: &mut Scene, here: &Devices) -> Vec<String> {
    let has = |list: &[crate::protocol::Named], handle: &str, name: &str| {
        list.iter().any(|d| d.id == handle || d.name == name)
    };
    let mut hidden = Vec::new();
    for layer in &mut scene.layers {
        let source = &layer.source;
        let present = match source.kind {
            Kind::Image => true,
            Kind::Camera => has(&here.cameras, &source.handle, &source.name),
            Kind::Screen => has(&here.screens, &source.handle, &source.name),
            Kind::Window => has(&here.windows, &source.handle, &source.name),
        };
        if !present {
            layer.visible = false;
            hidden.push(layer.id.clone());
        }
    }
    hidden
}

/// What differs between where a bench was made and here, said plainly; none
/// when it was made on this kind of machine.
pub fn mismatch(made: &Requires, here: &Requires) -> Option<String> {
    let os = made.os != here.os;
    let motor = motor_name(&made.motor) != motor_name(&here.motor);
    (os || motor).then(|| {
        format!(
            "made on {} with {}, this is {} with {}",
            made.os, made.motor, here.os, here.motor
        )
    })
}

// The motor's name without its version: `obs 30.2.3` is `obs`. A newer libobs
// reads the same scenes.
fn motor_name(motor: &str) -> &str {
    motor.split_whitespace().next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picture::layers::{Layer, Source, Transform};
    use crate::protocol::Named;

    fn layer(id: &str, kind: Kind, handle: &str, name: &str) -> Layer {
        Layer {
            id: id.into(),
            source: Source {
                kind,
                handle: handle.into(),
                name: name.into(),
                width: 1920,
                height: 1080,
                stable: None,
            },
            transform: Transform::default(),
            visible: true,
            crop: None,
            shape: None,
            mirrored: false,
            shader: None,
        }
    }

    fn scene(layers: Vec<Layer>) -> Scene {
        Scene {
            name: "Starting soon".into(),
            layers,
            order: vec![],
            elements: vec![],
            shader: None,
        }
    }

    #[test]
    fn the_files_are_the_pictures_and_the_filters_each_once() {
        let mut bg = layer("bg", Kind::Image, "/a/bg.png", "bg.png");
        bg.shader = Some("/s/wave.wgsl".into());
        let mut first = scene(vec![bg, layer("cam", Kind::Camera, "0x1", "Cam")]);
        first.shader = Some("/s/wave.wgsl".into());
        let second = scene(vec![layer("logo", Kind::Image, "/b/logo.png", "logo.png")]);
        assert_eq!(
            files(&[first, second]),
            ["/a/bg.png", "/s/wave.wgsl", "/b/logo.png"]
        );
    }

    #[test]
    fn a_packed_file_keeps_its_name_behind_its_place() {
        assert_eq!(packed(0, "/a/logo.png"), "files/0-logo.png");
        assert_eq!(packed(1, "/b/logo.png"), "files/1-logo.png");
    }

    #[test]
    fn relocating_moves_every_path_and_leaves_devices_alone() {
        let mut bg = layer("bg", Kind::Image, "/a/bg.png", "bg.png");
        bg.shader = Some("/s/wave.wgsl".into());
        let mut scenes = [scene(vec![bg, layer("cam", Kind::Camera, "0x1", "Cam")])];
        scenes[0].shader = Some("/s/wave.wgsl".into());
        relocate(&mut scenes, |p| format!("/here{p}"));
        let [moved] = &scenes;
        assert_eq!(moved.layers[0].source.handle, "/here/a/bg.png");
        assert_eq!(moved.layers[0].shader.as_deref(), Some("/here/s/wave.wgsl"));
        assert_eq!(moved.shader.as_deref(), Some("/here/s/wave.wgsl"));
        assert_eq!(
            moved.layers[1].source.handle, "0x1",
            "a device is not a file"
        );
    }

    #[test]
    fn a_device_not_here_hides_its_layer_and_is_named() {
        let here = Devices {
            cameras: vec![Named {
                id: "0x9".into(),
                name: "Cam".into(),
            }],
            ..Devices::default()
        };
        let mut scene = scene(vec![
            layer("cam", Kind::Camera, "0x1", "Cam"),
            layer("desk", Kind::Screen, "1", "Big monitor"),
            layer("bg", Kind::Image, "/x/bg.png", "bg.png"),
        ]);
        assert_eq!(hide_missing(&mut scene, &here), ["desk"]);
        assert!(scene.layers[0].visible, "the same camera by its name");
        assert!(!scene.layers[1].visible);
        assert!(scene.layers[2].visible, "a picture travels with the bench");
    }

    #[test]
    fn another_system_is_said_and_another_version_of_the_motor_is_not() {
        let made = Requires {
            os: "macos".into(),
            motor: "obs 30.2.3".into(),
        };
        let newer = Requires {
            os: "macos".into(),
            motor: "obs 31.0.0".into(),
        };
        let linux = Requires {
            os: "linux".into(),
            motor: "obs 30.2.3".into(),
        };
        assert_eq!(mismatch(&made, &newer), None);
        assert_eq!(
            mismatch(&made, &linux).as_deref(),
            Some("made on macos with obs 30.2.3, this is linux with obs 30.2.3")
        );
    }
}
