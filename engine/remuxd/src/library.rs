//! The music on disk.
//!
//! A folder, not a library. One directory, one folder per genre, the files
//! inside are the playlist. Drop tracks in a folder and they are there; there
//! is nothing to import, no metadata to fill in and no database. That opinion
//! is the engine's, reading the directory itself rather than asking a server
//! for it: the daemon is on the same
//! machine as the files, and a music folder that stops working because the
//! control plane is down would be an absurd way to lose a live.
//!
//! What counts as a track and what it is called is [`remuxd_domain::sound::music`],
//! which is pure. This is the filesystem and nothing else.

use std::path::{Path, PathBuf};

use remuxd_domain::engine::Library;
use remuxd_domain::sound::music::{genre_title, track_from, Playlist, Track};

/// The folder, behind the engine's port.
///
/// Read on every ask rather than cached. A folder is something a person drops
/// a file into while the engine is running, and the whole opinion of this
/// feature is that dropping a file in is all there is to it.
pub struct Folder(pub PathBuf);

impl Default for Folder {
    fn default() -> Self {
        Self(root())
    }
}

impl Library for Folder {
    fn playlists(&self) -> Vec<Playlist> {
        playlists(&self.0)
    }
}

/// Where the music is: the config's, or `~/Music/remux`.
pub fn root() -> PathBuf {
    remuxd_domain::config::music_dir()
}

/// Every genre folder with at least one track in it, sorted by name.
///
/// A folder that cannot be read is not an error worth stopping for: it means
/// no music, and a live without a bed under it is a live, where a live that
/// refused to start because of a missing directory is not.
pub fn playlists(root: &Path) -> Vec<Playlist> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| playlist(root, &name))
        .filter(|playlist| !playlist.tracks.is_empty())
        .collect()
}

/// One genre, with its tracks in the order the files sort.
///
/// Sorted rather than in whatever order the filesystem hands them over,
/// because the order is what [`remuxd_domain::sound::music::Rotation`] shuffles, and
/// a shuffle of an unstable order is unreproducible when something goes wrong.
pub fn playlist(root: &Path, name: &str) -> Playlist {
    let folder = root.join(name);
    let mut files: Vec<String> = std::fs::read_dir(&folder)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    files.sort();

    let tracks: Vec<Track> = files
        .iter()
        .filter_map(|file| {
            // The url is the path on disk. The engine reads the file itself,
            // where the browser had to be handed a URL to fetch.
            track_from(file, folder.join(file).to_string_lossy().into_owned())
        })
        .collect();

    Playlist {
        name: name.to_string(),
        title: genre_title(name),
        tracks,
    }
}
