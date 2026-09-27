//! Scenes: a setup with a name, kept and switched whole.
//!
//! What is behind the picture, where the camera sits, whether it is
//! mirrored, and which apps' sound is heard. `remux scene save code` keeps the
//! setup of now under that name; `remux scene code` puts it back. A window is
//! kept by the words that find it, never by its id, which the window server
//! hands out anew every time the app opens.

use serde::{Deserialize, Serialize};

use crate::scene::Layout;

/// What a scene puts behind the picture.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Shown {
    #[default]
    Nothing,
    Screen {
        display: u32,
    },
    Window {
        query: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Scene {
    pub name: String,
    pub shown: Shown,
    pub layout: Layout,
    pub mirrored: bool,
    /// The apps whose sound is heard, by name; empty is the whole screen's.
    #[serde(default)]
    pub hear: Vec<String>,
}

/// Keep a scene under its name: a second save with the same name replaces
/// the first, in place, so the list keeps the order a person made it in.
pub fn keep(scenes: &mut Vec<Scene>, scene: Scene) {
    match scenes.iter_mut().find(|s| s.name == scene.name) {
        Some(there) => *there = scene,
        None => scenes.push(scene),
    }
}

pub fn find<'a>(scenes: &'a [Scene], name: &str) -> Option<&'a Scene> {
    scenes.iter().find(|s| s.name == name)
}

/// Forget a scene by name; whether there was one.
pub fn forget(scenes: &mut Vec<Scene>, name: &str) -> bool {
    let before = scenes.len();
    scenes.retain(|s| s.name != name);
    scenes.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code() -> Scene {
        Scene {
            name: "code".into(),
            shown: Shown::Window {
                query: "Ghostty".into(),
            },
            layout: Layout::default(),
            mirrored: true,
            hear: vec![],
        }
    }

    #[test]
    fn a_scene_saved_again_replaces_itself_where_it_was() {
        let mut scenes = vec![
            code(),
            Scene {
                name: "talk".into(),
                ..code()
            },
        ];
        keep(
            &mut scenes,
            Scene {
                mirrored: false,
                ..code()
            },
        );
        assert_eq!(scenes.len(), 2);
        assert_eq!(scenes[0].name, "code");
        assert!(!scenes[0].mirrored);
        assert!(forget(&mut scenes, "code"));
        assert!(!forget(&mut scenes, "code"));
        assert!(find(&scenes, "talk").is_some());
    }

    #[test]
    fn a_scene_survives_the_file_it_is_kept_in() {
        let written = serde_json::to_string(&code()).unwrap();
        assert!(written.contains("\"kind\":\"window\""), "{written}");
        assert_eq!(serde_json::from_str::<Scene>(&written).unwrap(), code());
    }
}
