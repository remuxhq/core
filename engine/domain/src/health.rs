//! `remux health`: whether this engine could go live now, and what stands
//! in the way, one line each. The first question an agent asks, and the one
//! a person asks when the picture is black.

use serde::{Deserialize, Serialize};

use crate::protocol::{Grant, Status};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Health {
    pub version: String,
    pub motor: String,
    /// Nothing stands in the way of a live.
    pub ok: bool,
    /// What does, in the order to fix it.
    pub trouble: Vec<String>,
}

/// The grants as the engine answered them.
pub struct Grants {
    pub screen: Grant,
    pub camera: Grant,
    pub microphone: Grant,
}

pub fn of(status: &Status, grants: &Grants) -> Health {
    let mut trouble = Vec::new();
    let grant = |name: &str, grant: Grant| {
        match grant {
        Grant::Refused => Some(format!(
            "{name}: refused; System Settings, Privacy & Security, {name} for remux"
        )),
        // Screen recording is granted in System Settings alone: an engine not
        // granted is shown no display, so nothing it does makes macOS ask.
        // The grant is tied to the build's signature, so a new build is a new
        // app there, under the old one's name: the old entry has to go.
        Grant::NotAsked if name == "screen recording" => Some(format!(
            "{name}: not granted to this build; System Settings, Privacy & Security, Screen Recording: \
             remove remuxd if it is listed, add it again with +, turn it on, then remux daemon restart"
        )),
        Grant::NotAsked => Some(format!("{name}: not granted yet; the first use asks")),
        Grant::Granted => None,
    }
    };
    trouble.extend(grant("screen recording", grants.screen));
    trouble.extend(grant("microphone", grants.microphone));
    if status
        .layers
        .iter()
        .any(|l| l.source.kind == crate::picture::layers::Kind::Camera)
    {
        trouble.extend(grant("camera", grants.camera));
    }
    if !status.layers.iter().any(|l| l.visible) {
        trouble.push(
            "no picture: remux scene layer add screen <id> <display> or window <id> <words>".into(),
        );
    } else if status.scene_flowing.frames == 0 {
        trouble.push("the capture is not delivering frames".into());
    }
    if let (Some(mic), Some(why)) = (&status.mic, &status.hearing.complaint) {
        trouble.push(format!("mic {mic}: {why}"));
    }
    if !status.destinations.iter().any(|d| d.armed && d.connected) {
        trouble.push("no destination armed and connected: remux destination list".into());
    }
    for row in status.destinations.iter().filter(|d| d.armed) {
        if let Some(why) = &row.trouble {
            trouble.push(format!("{}: {why}", row.name));
        }
    }
    Health {
        version: status.version.clone(),
        motor: status.motor.clone(),
        ok: trouble.is_empty(),
        trouble,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::protocol::Destination;

    pub(crate) fn screen_layer(name: &str) -> crate::picture::layers::Layer {
        crate::picture::layers::Layer {
            id: "desk".into(),
            source: crate::picture::layers::Source {
                kind: crate::picture::layers::Kind::Screen,
                handle: "1".into(),
                name: name.into(),
                width: 1920,
                height: 1080,
                stable: None,
            },
            transform: crate::picture::layers::Transform::native((1920, 1080)),
            visible: true,
            crop: None,
            shape: None,
            mirrored: false,
            shader: None,
        }
    }

    fn ready() -> (Status, Grants) {
        let mut status = Status {
            version: "0.1.0".into(),
            layers: vec![screen_layer("VG2791R")],
            destinations: vec![Destination {
                id: 1,
                name: "tw".into(),
                armed: true,
                connected: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        status.scene_flowing.frames = 30;
        (
            status,
            Grants {
                screen: Grant::Granted,
                camera: Grant::NotAsked,
                microphone: Grant::Granted,
            },
        )
    }

    #[test]
    fn a_screen_not_granted_is_sent_to_the_settings_not_told_to_wait_for_a_dialog() {
        let (status, mut grants) = ready();
        grants.screen = Grant::NotAsked;
        let said = of(&status, &grants).trouble.join("\n");
        // macOS lists no display to an engine it has not granted, so nothing ever
        // asks: the settings are the way, and a new build is a new app there.
        assert!(
            said.contains("System Settings, Privacy & Security, Screen Recording"),
            "{said}"
        );
        assert!(said.contains("remove remuxd"), "{said}");
        assert!(said.contains("remux daemon restart"), "{said}");
        assert!(!said.contains("asks once"), "{said}");
    }

    #[test]
    fn a_ready_engine_is_ok_and_every_lack_is_a_line() {
        let (status, grants) = ready();
        assert_eq!(
            of(&status, &grants),
            Health {
                version: "0.1.0".into(),
                motor: String::new(),
                ok: true,
                trouble: vec![]
            }
        );
        let mut status = status;
        status.layers.clear();
        status.destinations[0].armed = false;
        let mut grants = grants;
        grants.screen = Grant::Refused;
        let said = of(&status, &grants);
        assert!(!said.ok);
        assert_eq!(said.trouble.len(), 3);
        assert!(said.trouble[0].contains("System Settings"));
        assert!(said.trouble[1].starts_with("no picture"));
        assert!(said.trouble[2].starts_with("no destination"));
    }

    #[test]
    fn a_camera_chosen_needs_its_grant_and_a_refusal_is_on_its_row() {
        let (mut status, grants) = ready();
        let mut face = screen_layer("FaceTime");
        face.id = "face".into();
        face.source.kind = crate::picture::layers::Kind::Camera;
        status.layers.push(face);
        status.destinations[0].trouble = Some("youtube said 403".into());
        let said = of(&status, &grants);
        assert_eq!(
            said.trouble,
            vec![
                "camera: not granted yet; the first use asks",
                "tw: youtube said 403"
            ]
        );
    }
}
