//! The session with the web, after `remux login`: where it is and the token,
//! in one file only the owner reads. The shell writes it; the engine reads it
//! at boot and is the account's engine from then on. No file, no account: the
//! destinations file is the whole of it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::destinations::{keep, Key};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// The web, `https://...`, no trailing slash.
    pub base: String,
    pub token: Key,
}

/// Where the file is: `REMUX_SESSION`, else `~/.config/remux/session.json`.
pub fn path() -> PathBuf {
    if let Some(said) = std::env::var_os("REMUX_SESSION") {
        return PathBuf::from(said);
    }
    crate::os::config_dir().join("session.json")
}

/// The web `remux login` talks to unless told otherwise: the config's.
pub fn default_base() -> String {
    crate::config::web_url()
}

pub fn read(path: &Path) -> Option<Session> {
    let said = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&said).ok()
}

/// Written whole, 0600, renamed into place.
pub fn write(path: &Path, session: &Session) -> Result<(), String> {
    let said = serde_json::to_string_pretty(session).map_err(|e| e.to_string())?;
    keep(path, &said)
}

/// `remux logout`: whether there was a session to forget.
pub fn forget(path: &Path) -> bool {
    std::fs::remove_file(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_kept_0600_read_back_and_forgotten() {
        let dir = std::env::temp_dir().join(format!("remux-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("session.json");
        let session = Session {
            base: "http://localhost:4700".into(),
            token: Key("abc".into()),
        };
        assert_eq!(read(&path), None);
        write(&path, &session).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(read(&path), Some(session.clone()));
        assert_eq!(
            format!("{session:?}"),
            "Session { base: \"http://localhost:4700\", token: **** }"
        );
        assert!(forget(&path));
        assert!(!forget(&path));
    }
}
