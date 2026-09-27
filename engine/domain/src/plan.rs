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
    /// What is behind the picture: a screen, a window, or nothing.
    pub picture: String,
    pub camera: Option<String>,
    pub mirrored: bool,
    pub mic: Option<String>,
    pub muted: bool,
    pub music: Option<String>,
    pub music_to_stream: bool,
    pub screen_sound: bool,
    pub card: Option<String>,
    pub destinations: Vec<Planned>,
    /// What stops the live before it starts. Empty means it would go.
    pub blockers: Vec<String>,
    pub fingerprint: u64,
}

impl Plan {
    pub fn of(status: &Status) -> Self {
        let destinations: Vec<Planned> = status.destinations.iter().map(planned).collect();
        let mut blockers = Vec::new();
        if status.flowing.frames == 0 {
            blockers.push("there is no picture to send yet".into());
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
            picture: status
                .screen
                .clone()
                .unwrap_or_else(|| "nothing shared".into()),
            camera: status.camera.clone(),
            mirrored: status.mirrored,
            mic: status.mic.clone(),
            muted: status.muted,
            music: status.music.clone(),
            music_to_stream: status.music_to_stream,
            screen_sound: status.screen_sound,
            card: status.card.map(|card| format!("{card:?}")),
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
            &self.card,
            &self.destinations,
            &self.blockers,
        )
            .hash(&mut hasher);
        hasher.finish()
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
            screen: Some("VG2791R".into()),
            flowing: Flowing {
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
        assert_eq!(plan.picture, "VG2791R");
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
    fn the_fingerprint_moves_with_what_a_person_confirms_and_with_nothing_else() {
        let a = Plan::of(&ready());
        let mut retitled = ready();
        retitled.destinations[0].title = Some("something else".into());
        assert_ne!(a.fingerprint, Plan::of(&retitled).fingerprint);
        let mut louder = ready();
        louder.hearing.level_db = -3.0;
        louder.viewers = Some(9);
        louder.flowing.frames = 3_000;
        assert_eq!(a.fingerprint, Plan::of(&louder).fingerprint);
    }
}
