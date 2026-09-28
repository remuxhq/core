//! What Go live would do, said before it is done.
//!
//! A shell has no sheet to read before the button: a `remux live` that went
//! at once would put a wrong screen or a stale title on the air before anyone
//! saw it. The plan is every fact a person confirms, read off the status, with
//! a fingerprint: `live --confirm <fingerprint>` goes only if nothing moved
//! since the plan was printed, so an agent cannot confirm a plan it did not see.

use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::protocol::{Destination, Status};

/// One destination as the live would find it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Planned {
    pub id: i64,
    pub name: String,
    pub platform: String,
    pub armed: bool,
    pub sandbox: bool,
    pub connected: bool,
    pub title: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    /// Nothing when this destination would be told and fed; else why not.
    pub why_not: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Plan {
    pub on_air: bool,
    pub recording: bool,
    /// The scene that goes out, by name.
    pub scene: String,
    /// What is in it, back to front: each layer as `id (kind name)`, or
    /// "nothing shared" when the scene is empty.
    pub picture: String,
    pub camera: Option<String>,
    pub mirrored: bool,
    pub mic: Option<String>,
    pub muted: bool,
    pub music: Option<String>,
    pub music_to_stream: bool,
    pub screen_sound: bool,
    pub destinations: Vec<Planned>,
    /// What stops the live before it starts. Empty means it would go.
    pub blockers: Vec<String>,
    pub fingerprint: u64,
}

impl Plan {
    pub fn of(status: &Status) -> Self {
        let destinations: Vec<Planned> = status.destinations.iter().map(planned).collect();
        let picture = picture(status);
        let mut blockers = Vec::new();
        if status.scene_flowing.frames == 0 {
            blockers.push("there is no picture to send yet".into());
        } else if picture == NOTHING {
            blockers.push(EMPTY.into());
        }
        if !destinations.iter().any(|d| d.armed) {
            blockers.push("no destination is armed".into());
        }
        for d in destinations.iter().filter(|d| d.armed) {
            if let Some(why) = &d.why_not {
                blockers.push(format!("{}: {why}", d.name));
            }
        }
        let mut plan = Self {
            on_air: status.on_air,
            recording: status.recording,
            scene: status.active_scene.clone(),
            picture,
            camera: status
                .layers
                .iter()
                .find(|l| l.visible && l.source.kind == crate::layers::Kind::Camera)
                .map(|l| l.source.name.clone()),
            mirrored: status.mirrored,
            mic: status.mic.clone(),
            muted: status.muted,
            music: status.music.clone(),
            music_to_stream: status.music_to_stream,
            screen_sound: status.screen_sound,
            destinations,
            blockers,
            fingerprint: 0,
        };
        plan.fingerprint = plan.fingerprint();
        plan
    }

    /// The same facts, the same number: anything a person would confirm is
    /// in it, and nothing that moves on its own (levels, clocks, viewers).
    fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (
            &self.picture,
            &self.camera,
            self.mirrored,
            &self.mic,
            self.muted,
            &self.music,
            self.music_to_stream,
            self.screen_sound,
            &self.scene,
            &self.destinations,
            &self.blockers,
        )
            .hash(&mut hasher);
        hasher.finish()
    }
}

/// What the picture reads when the scene has nothing visible in it.
const NOTHING: &str = "nothing shared";

/// Why a scene with nothing visible does not go live.
pub const EMPTY: &str = "the scene is empty: nothing would be shared";

/// Whether the active scene has nothing visible in it: both motors draw it
/// at the full rate, so its frames cannot say so.
pub fn empty(status: &Status) -> bool {
    picture(status) == NOTHING
}

/// The active scene's layers and elements, back to front, the way a person
/// reads them before confirming.
fn picture(status: &Status) -> String {
    let scene = status.scenes.iter().find(|s| s.name == status.active_scene);
    // The status's layers are the active scene's, whether or not the scene
    // in the list has caught up with them yet.
    let mut ordered = scene
        .cloned()
        .unwrap_or_else(|| crate::scenes::defaults().remove(0));
    ordered.layers = status.layers.clone();
    let shown: Vec<String> = ordered
        .ordered_ids()
        .into_iter()
        .filter_map(|id| {
            if let Some(layer) = status.layers.iter().find(|l| l.id == id) {
                return layer.visible.then(|| {
                    let kind = match layer.source.kind {
                        crate::layers::Kind::Screen => "screen",
                        crate::layers::Kind::Window => "window",
                        crate::layers::Kind::Camera => "camera",
                    };
                    format!("{id} ({kind} {})", layer.source.name)
                });
            }
            let element = scene?.elements.iter().find(|e| e.id == id)?;
            element.visible.then(|| match &element.content {
                crate::scenes::ElementContent::Text { text } => format!("{id} (text {text})"),
                crate::scenes::ElementContent::Timer { seconds } => {
                    format!("{id} (timer {seconds} s)")
                }
            })
        })
        .collect();
    if shown.is_empty() {
        NOTHING.into()
    } else {
        shown.join(", ")
    }
}

fn planned(d: &Destination) -> Planned {
    Planned {
        id: d.id,
        name: d.name.clone(),
        platform: d.platform.clone(),
        armed: d.armed,
        sandbox: d.sandbox,
        connected: d.connected,
        title: d.title.clone(),
        description: d.description.clone(),
        category: d.category.clone(),
        why_not: (!d.connected).then(|| "not connected".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Flowing;

    fn twitch(armed: bool, connected: bool) -> Destination {
        Destination {
            id: 2,
            name: "twitch".into(),
            platform: "twitch".into(),
            status: "off".into(),
            armed,
            sandbox: true,
            connected,
            account: None,
            category: None,
            category_id: None,
            viewers: None,
            viewers_peak: None,
            trouble: None,
            title: Some("remux".into()),
            description: None,
            channel: None,
        }
    }

    fn ready() -> Status {
        Status {
            layers: vec![crate::health::tests::screen_layer("VG2791R")],
            scene_flowing: Flowing {
                frames: 30,
                ..Flowing::default()
            },
            destinations: vec![twitch(true, true)],
            ..Status::default()
        }
    }

    #[test]
    fn a_plan_that_would_go_has_no_blockers_and_names_what_it_sends() {
        let plan = Plan::of(&ready());
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        assert_eq!(plan.picture, "desk (screen VG2791R)");
        assert_eq!(plan.scene, "default");
        assert_eq!(plan.destinations[0].title.as_deref(), Some("remux"));
    }

    #[test]
    fn what_would_stop_the_live_is_said_before_it() {
        let idle = Plan::of(&Status::default());
        assert_eq!(
            idle.blockers,
            vec!["there is no picture to send yet", "no destination is armed"]
        );
        let unconnected = Plan::of(&Status {
            destinations: vec![twitch(true, false)],
            ..ready()
        });
        assert_eq!(unconnected.blockers, vec!["twitch: not connected"]);
    }

    #[test]
    fn an_empty_scene_is_a_blocker_though_it_has_frames() {
        // Both motors draw an empty scene at the full rate, so the frames
        // cannot say whether anything is in it; the plan says so instead.
        let mut empty = ready();
        empty.layers[0].visible = false;
        assert_eq!(
            Plan::of(&empty).blockers,
            vec!["the scene is empty: nothing would be shared"]
        );
    }

    #[test]
    fn the_fingerprint_moves_with_what_a_person_confirms_and_with_nothing_else() {
        let a = Plan::of(&ready());
        let mut retitled = ready();
        retitled.destinations[0].title = Some("something else".into());
        assert_ne!(a.fingerprint, Plan::of(&retitled).fingerprint);
        let mut louder = ready();
        louder.hearing.level_db = -3.0;
        louder.viewers = Some(9);
        louder.scene_flowing.frames = 3_000;
        assert_eq!(a.fingerprint, Plan::of(&louder).fingerprint);
    }
}
