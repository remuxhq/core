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
    let grant = |name: &str, grant: Grant| match grant {
        Grant::Refused => Some(format!(
            "{name}: refused; System Settings, Privacy & Security, {name} for remux"
        )),
        // Screen recording is the one macOS never grants from the dialog: it
        // lists the asker in System Settings, the person turns it on there,
        // and the process has to start again to see it.
        Grant::NotAsked if name == "screen recording" => Some(format!(
            "{name}: not granted yet; remux screen <id> asks once, then System Settings, \
             Privacy & Security, Screen Recording, remuxd on, then remux daemon restart"
        )),
        Grant::NotAsked => Some(format!("{name}: not granted yet; the first use asks")),
        Grant::Granted => None,
    };
    trouble.extend(grant("screen recording", grants.screen));
    trouble.extend(grant("microphone", grants.microphone));
    if status.camera.is_some() {
        trouble.extend(grant("camera", grants.camera));
    }
    if status.screen.is_none() {
        trouble.push("no picture: remux screen <id> or remux window <words>".into());
    } else if status.flowing.frames == 0 {
        trouble.push("the capture is not delivering frames".into());
    }
    if let (Some(mic), Some(why)) = (&status.mic, &status.hearing.complaint) {
        trouble.push(format!("mic {mic}: {why}"));
    }
    if !status.destinations.iter().any(|d| d.armed && d.connected) {
        trouble.push("no destination armed and connected: remux destinations".into());
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
mod tests {
    use super::*;
    use crate::protocol::Destination;

    fn ready() -> (Status, Grants) {
        let mut status = Status {
            version: "0.1.0".into(),
            screen: Some("VG2791R".into()),
            destinations: vec![Destination {
                id: 1,
                name: "tw".into(),
                armed: true,
                connected: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        status.flowing.frames = 30;
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
        status.screen = None;
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
        status.camera = Some("FaceTime".into());
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
