//! One-shot sounds into the live: a clap, a jingle, a line of someone's
//! voice. A folder of files, played once over the mix at their own level,
//! never ducked and never on the speakers.

use std::path::{Path, PathBuf};

/// Where the clips are: `REMUX_CLIPS_DIR`, else `clips` beside where the
/// engine runs, the way the music has its folder.
pub fn root() -> PathBuf {
    crate::config::clips_dir()
}

const KINDS: [&str; 5] = ["wav", "mp3", "m4a", "aiff", "flac"];

/// The file a name means: a path that exists, as it is; else the name in
/// the folder, with or without one of the usual extensions.
pub fn find(name: &str, root: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let said = Path::new(name);
    if exists(said) {
        return Some(said.to_path_buf());
    }
    let mut tries = vec![root.join(name)];
    tries.extend(KINDS.iter().map(|kind| root.join(format!("{name}.{kind}"))));
    tries.into_iter().find(|path| exists(path))
}

/// Every clip in the folder, by the name `play` takes, sorted.
pub fn list(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| KINDS.contains(&e.to_ascii_lowercase().as_str()))
        })
        .filter_map(|path| path.file_stem()?.to_str().map(String::from))
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_finds_the_file_in_the_folder_with_or_without_its_extension() {
        let there = |p: &Path| p == Path::new("clips/clap.wav");
        assert_eq!(
            find("clap", Path::new("clips"), there),
            Some("clips/clap.wav".into())
        );
        assert_eq!(
            find("clap.wav", Path::new("clips"), there),
            Some("clips/clap.wav".into())
        );
        assert_eq!(find("boo", Path::new("clips"), there), None);
    }

    #[test]
    fn a_path_that_exists_is_taken_as_it_is() {
        let there = |p: &Path| p == Path::new("/tmp/hi.mp3");
        assert_eq!(
            find("/tmp/hi.mp3", Path::new("clips"), there),
            Some("/tmp/hi.mp3".into())
        );
    }
}
