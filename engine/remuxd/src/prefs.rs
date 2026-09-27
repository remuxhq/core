//! Where the setup is kept between runs.
//!
//! The file itself, and nothing about what goes in it: what is worth
//! remembering is [`remuxd_domain::remembered`], which is pure and tested. This
//! is the disk.
//!
//! Beside the socket, in the app's own support folder, because a daemon that
//! ships inside a signed bundle has no business writing to a shared temporary
//! path. Written whole and atomically: a preferences file half-written by a
//! machine that lost power is a setup nobody can get back, and the cure is one
//! rename.

use std::path::PathBuf;

use remuxd_domain::remembered::{self, Remembered};

/// `prefs.json` beside the socket, in the OS's state folder, or wherever the socket
/// was pointed.
#[must_use]
pub fn path() -> PathBuf {
    if let Some(said) = std::env::var_os("REMUXD_PREFS") {
        return PathBuf::from(said);
    }
    crate::server::default_socket_path().with_file_name("prefs.json")
}

/// The setup as last left, or an empty one.
///
/// A file that is missing, unreadable or nonsense all mean the same thing here
/// and none of them stops a start: the worst any of it costs is choosing a
/// microphone once.
#[must_use]
pub fn read() -> Remembered {
    std::fs::read_to_string(path())
        .ok()
        .map(|said| remembered::read(&said))
        .unwrap_or_default()
}

/// Write it down, and say nothing if that fails.
///
/// A full disk is not a reason to interrupt a live. The next command writes it
/// again, and there is one every time anybody touches anything.
pub fn write(setup: &Remembered) {
    let Some(said) = remembered::write(setup) else {
        return;
    };
    let path = path();
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    // Whole, then renamed. A reader that arrives mid-write sees the old file or
    // the new one, never half of either.
    let meanwhile = path.with_extension("json.new");
    if std::fs::write(&meanwhile, said).is_ok() {
        let _ = std::fs::rename(&meanwhile, &path);
    }
}
