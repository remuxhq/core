//! Where the engine's socket lives.
//!
//! Every face and the daemon agree on one address, and the decision is here
//! rather than in the daemon so the CLI does not have to link the daemon to
//! know it.

use std::path::PathBuf;

/// The socket path: `REMUXD_SOCKET` when set, else the person's own place
/// for such things (macOS: the app's support folder; Linux: the state
/// folder), because a daemon has no business writing to a shared temp path.
pub fn default_path() -> PathBuf {
    std::env::var_os("REMUXD_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| (crate::os::OS.state_dir)(&crate::os::home()).join("remuxd.sock"))
}

#[cfg(test)]
mod tests {
    use super::default_path;

    // nextest runs each test in its own process, so the environment is ours.
    #[test]
    fn the_environment_names_the_socket_when_it_says_so() {
        std::env::set_var("REMUXD_SOCKET", "/tmp/somewhere.sock");
        assert_eq!(default_path().to_str(), Some("/tmp/somewhere.sock"));
    }

    #[test]
    fn otherwise_the_socket_lives_in_the_persons_own_place() {
        std::env::remove_var("REMUXD_SOCKET");
        std::env::set_var("HOME", "/Users/somebody");
        let expected =
            (crate::os::OS.state_dir)(std::path::Path::new("/Users/somebody")).join("remuxd.sock");
        assert_eq!(default_path(), expected);
        assert!(default_path().starts_with("/Users/somebody"));
    }
}
