//! `remux daemon`: the engine as a service of the person's session, never
//! `remuxd &`. The words are here; which service manager and how is
//! `crate::os::OS.service`; the shell runs what that table says.

use std::path::PathBuf;

pub use crate::os::Loaded;

/// What `remux daemon <verb>` asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    Start,
    /// Refused on air unless forced: a stopped engine ends a live.
    Stop {
        force: bool,
    },
    Restart {
        force: bool,
    },
    Status,
    Log {
        follow: bool,
    },
    Path,
}

impl Verb {
    pub fn parse(words: &[String]) -> Result<Verb, String> {
        let force = words.iter().any(|w| w == "--force");
        let follow = words.iter().any(|w| w == "-f" || w == "--follow");
        match words.first().map(String::as_str) {
            Some("start") => Ok(Verb::Start),
            Some("stop") => Ok(Verb::Stop { force }),
            Some("restart") => Ok(Verb::Restart { force }),
            Some("status") => Ok(Verb::Status),
            Some("log") => Ok(Verb::Log { follow }),
            Some("path") => Ok(Verb::Path),
            _ => Err(
                "daemon takes start, stop [--force], restart [--force], status, log [-f] or path"
                    .into(),
            ),
        }
    }
}

/// The service file, under the person's home.
pub fn service_file() -> PathBuf {
    (crate::os::OS.service.file)(&crate::os::home())
}

/// Where the service manager writes what the engine says on stdout and
/// stderr (libobs talks there, and a panic does): beside the engine's own log.
pub fn output_path() -> PathBuf {
    crate::log::path().with_file_name("remuxd.out.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str) -> Vec<String> {
        text.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn the_words_are_six_verbs_and_two_flags() {
        assert_eq!(Verb::parse(&w("start")), Ok(Verb::Start));
        assert_eq!(Verb::parse(&w("stop")), Ok(Verb::Stop { force: false }));
        assert_eq!(
            Verb::parse(&w("restart --force")),
            Ok(Verb::Restart { force: true })
        );
        assert_eq!(Verb::parse(&w("log -f")), Ok(Verb::Log { follow: true }));
        assert_eq!(Verb::parse(&w("path")), Ok(Verb::Path));
        assert!(Verb::parse(&w("")).is_err());
        assert!(Verb::parse(&w("dance")).is_err());
    }

    // nextest runs each test in its own process, so the environment is ours.
    #[test]
    fn the_service_file_and_the_output_live_under_the_persons_home_and_beside_the_socket() {
        std::env::set_var("HOME", "/Users/somebody");
        std::env::remove_var("REMUXD_SOCKET");
        std::env::remove_var("REMUXD_LOG");
        assert!(service_file().starts_with("/Users/somebody"));
        assert_eq!(
            output_path(),
            crate::socket::default_path().with_file_name("remuxd.out.log")
        );
    }
}
