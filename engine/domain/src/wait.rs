//! `remux wait <until> [--for <seconds>]`: the shell blocks until the engine
//! says so, for a script that goes live and then does the next thing.

use crate::protocol::Status;

/// What is waited for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Until {
    OnAir,
    OffAir,
    /// The capture delivering a second of frames.
    Picture,
    Recording,
    NotRecording,
    /// A destination's row saying `live`, by id or name.
    Live(String),
}

impl Until {
    pub fn parse(word: &str, rest: Option<&str>) -> Result<Self, String> {
        Ok(match (word, rest) {
            ("on-air", _) => Until::OnAir,
            ("off-air", _) => Until::OffAir,
            ("picture", _) => Until::Picture,
            ("recording", _) => Until::Recording,
            ("not-recording", _) => Until::NotRecording,
            ("live", Some(which)) => Until::Live(which.to_string()),
            _ => return Err(
                "wait takes on-air, off-air, picture, recording, not-recording or live <id|name>"
                    .into(),
            ),
        })
    }

    pub fn met(&self, status: &Status) -> bool {
        match self {
            Until::OnAir => status.on_air,
            Until::OffAir => !status.on_air,
            Until::Picture => status.picture.frames >= 30,
            Until::Recording => status.recording,
            Until::NotRecording => !status.recording,
            Until::Live(which) => status
                .destinations
                .iter()
                .any(|d| (d.name == *which || d.id.to_string() == *which) && d.status == "live"),
        }
    }

    /// What is still being waited for, for the timeout's line.
    pub fn words(&self) -> String {
        match self {
            Until::OnAir => "on air".into(),
            Until::OffAir => "off air".into(),
            Until::Picture => "a picture".into(),
            Until::Recording => "recording".into(),
            Until::NotRecording => "the recording to stop".into(),
            Until::Live(which) => format!("{which} to be live"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Destination;

    #[test]
    fn each_condition_reads_the_status() {
        let mut status = Status::default();
        assert!(Until::OffAir.met(&status));
        assert!(!Until::OnAir.met(&status));
        assert!(!Until::Picture.met(&status));
        status.picture.frames = 30;
        status.on_air = true;
        status.destinations = vec![Destination {
            id: 2,
            name: "tw".into(),
            status: "live".into(),
            ..Default::default()
        }];
        assert!(Until::Picture.met(&status));
        assert!(Until::OnAir.met(&status));
        assert!(Until::Live("tw".into()).met(&status));
        assert!(Until::Live("2".into()).met(&status));
        assert!(!Until::Live("yt".into()).met(&status));
        assert_eq!(
            Until::parse("live", Some("tw")),
            Ok(Until::Live("tw".into()))
        );
        assert!(Until::parse("live", None).is_err());
        assert!(Until::parse("sideways", None).is_err());
        assert_eq!(Until::Live("tw".into()).words(), "tw to be live");
    }
}
