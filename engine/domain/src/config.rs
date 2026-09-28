//! `~/.config/remux/config.toml`: what the engine and the shell are told once
//! and read every time. A verb writes it (`remux chat --url`), a person edits
//! it, the environment overrides it (a test, a one-off run). 0600,
//! because a chat wire's URL may carry a token.
//!
//! ```toml
//! [chat]
//! url = "ws://127.0.0.1:9999"
//! [web]
//! url = "https://remux.live"
//! [daemon]
//! motor = "obs"
//! record_dir = "~/Movies/remux"        # ~/Videos/remux on Linux
//! music_dir = "~/Music/remux"
//! clips_dir = "~/Music/remux/clips"
//! obs_app = "/Applications/OBS.app"    # the prefix on Linux, /usr
//! [byo]
//! twitch = "somebody"          # the channel byo/bridge.py reads
//! youtube = "dQw4w9WgXcQ"      # the live video it reads
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub chat: Chat,
    pub web: Web,
    pub daemon: Daemon,
    pub byo: Byo,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Chat {
    /// A chat wire of one's own (`docs/wire.md`); absent, the account's, or none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Web {
    /// The web `remux login` talks to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Daemon {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clips_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obs_app: Option<String>,
}

/// What `byo/bridge.py` reads when it is not told on its command line: the
/// engine never looks at these, the file is simply the one place for them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Byo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twitch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub youtube: Option<String>,
}

pub const DEFAULT_WEB: &str = "https://remux.live";
/// Where OBS is: the app on macOS, the install prefix on Linux.
pub const DEFAULT_OBS_APP: &str = crate::os::OS.obs;
/// Where recordings go: the OS's own folder for films.
pub const RECORDINGS: &str = crate::os::OS.recordings;

/// `REMUX_CONFIG`, else `~/.config/remux/config.toml`.
pub fn path() -> PathBuf {
    if let Some(said) = std::env::var_os("REMUX_CONFIG") {
        return PathBuf::from(said);
    }
    crate::os::config_dir().join("config.toml")
}

use crate::os::home;

/// `~/x` as the person's own folder.
pub fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if path == "~" => home(),
        None => PathBuf::from(path),
    }
}

/// The file, or the defaults when there is none or it does not parse: an
/// unreadable config is a config to fix, not a reason to refuse to start.
pub fn read(path: &Path) -> Config {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|said| toml::from_str(&said).ok())
        .unwrap_or_default()
}

/// Written whole, 0600, renamed into place.
pub fn write(path: &Path, config: &Config) -> Result<(), String> {
    let said = toml::to_string_pretty(config).map_err(|e| e.to_string())?;
    crate::destinations::keep(path, &said)
}

/// Change one thing and keep the rest.
pub fn edit(path: &Path, change: impl FnOnce(&mut Config)) -> Result<(), String> {
    let mut config = read(path);
    change(&mut config);
    write(path, &config)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

// What is in effect: the environment, else the file, else the default.

pub fn chat_url() -> Option<String> {
    env("REMUX_CHAT_URL").or_else(|| read(&path()).chat.url)
}

pub fn web_url() -> String {
    env("REMUX_WEB")
        .or_else(|| read(&path()).web.url)
        .unwrap_or_else(|| DEFAULT_WEB.into())
        .trim_end_matches('/')
        .to_string()
}

pub fn motor() -> Option<String> {
    env("REMUX_MOTOR").or_else(|| read(&path()).daemon.motor)
}

pub fn record_dir() -> PathBuf {
    env("REMUXD_RECORD_DIR")
        .or_else(|| read(&path()).daemon.record_dir)
        .map(|p| expand(&p))
        .unwrap_or_else(|| home().join(RECORDINGS))
}

pub fn music_dir() -> PathBuf {
    env("REMUX_MUSIC_DIR")
        .or_else(|| read(&path()).daemon.music_dir)
        .map(|p| expand(&p))
        .unwrap_or_else(|| home().join("Music/remux"))
}

pub fn clips_dir() -> PathBuf {
    env("REMUX_CLIPS_DIR")
        .or_else(|| read(&path()).daemon.clips_dir)
        .map(|p| expand(&p))
        .unwrap_or_else(|| home().join("Music/remux/clips"))
}

pub fn obs_app() -> String {
    env("OBS_APP")
        .or_else(|| read(&path()).daemon.obs_app)
        .unwrap_or_else(|| DEFAULT_OBS_APP.into())
}

/// `remux config`: what is in effect, and where it came from.
pub fn describe() -> String {
    let file = path();
    let kept = read(&file);
    let from = |var: &str, in_file: bool| {
        if env(var).is_some() {
            format!("  ({var})")
        } else if in_file {
            "  (config)".into()
        } else {
            "  (default)".into()
        }
    };
    format!(
        "config     {}{}\n\
         chat.url   {}{}\n\
         web.url    {}{}\n\
         motor      {}{}\n\
         record_dir {}{}\n\
         music_dir  {}{}\n\
         clips_dir  {}{}\n\
         obs_app    {}{}\n\
         byo.twitch  {}  (config, read by byo/bridge.py)\n\
         byo.youtube {}  (config, read by byo/bridge.py)",
        file.display(),
        if file.exists() {
            ""
        } else {
            "  (not there yet)"
        },
        chat_url().unwrap_or_else(|| "none".into()),
        from("REMUX_CHAT_URL", kept.chat.url.is_some()),
        web_url(),
        from("REMUX_WEB", kept.web.url.is_some()),
        motor().unwrap_or_else(|| "the build's".into()),
        from("REMUX_MOTOR", kept.daemon.motor.is_some()),
        record_dir().display(),
        from("REMUXD_RECORD_DIR", kept.daemon.record_dir.is_some()),
        music_dir().display(),
        from("REMUX_MUSIC_DIR", kept.daemon.music_dir.is_some()),
        clips_dir().display(),
        from("REMUX_CLIPS_DIR", kept.daemon.clips_dir.is_some()),
        obs_app(),
        from("OBS_APP", kept.daemon.obs_app.is_some()),
        kept.byo.twitch.unwrap_or_else(|| "none".into()),
        kept.byo.youtube.unwrap_or_else(|| "none".into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_is_kept_0600_read_back_whole_and_edited_one_key_at_a_time() {
        let dir = std::env::temp_dir().join(format!("remux-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        assert_eq!(read(&path), Config::default(), "no file is the defaults");
        edit(&path, |c| c.chat.url = Some("ws://localhost:9999".into())).unwrap();
        edit(&path, |c| c.daemon.music_dir = Some("~/Music/remux".into())).unwrap();
        edit(&path, |c| c.byo.twitch = Some("somebody".into())).unwrap();
        let kept = read(&path);
        assert_eq!(kept.byo.twitch.as_deref(), Some("somebody"));
        assert_eq!(kept.chat.url.as_deref(), Some("ws://localhost:9999"));
        assert_eq!(kept.daemon.music_dir.as_deref(), Some("~/Music/remux"));
        assert_eq!(kept.web.url, None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::write(&path, "not = [toml").unwrap();
        assert_eq!(read(&path), Config::default(), "junk is the defaults");
    }

    // nextest runs each test in its own process, so the environment is ours.
    #[test]
    fn what_is_in_effect_is_the_environment_then_the_file_then_the_default() {
        let dir = std::env::temp_dir().join(format!("remux-config-eff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("config.toml");
        let home_before = std::env::var_os("HOME").unwrap_or_default();
        std::env::set_var("REMUX_CONFIG", &file);
        std::env::set_var("HOME", "/Users/somebody");
        for var in [
            "REMUX_CHAT_URL",
            "REMUX_WEB",
            "REMUX_MOTOR",
            "REMUXD_RECORD_DIR",
            "REMUX_MUSIC_DIR",
            "REMUX_CLIPS_DIR",
            "OBS_APP",
        ] {
            std::env::remove_var(var);
        }
        assert_eq!(path(), file);
        assert_eq!(chat_url(), None);
        assert_eq!(web_url(), DEFAULT_WEB);
        assert_eq!(motor(), None);
        assert_eq!(
            record_dir(),
            PathBuf::from("/Users/somebody").join(RECORDINGS)
        );
        assert_eq!(music_dir(), PathBuf::from("/Users/somebody/Music/remux"));
        assert_eq!(
            clips_dir(),
            PathBuf::from("/Users/somebody/Music/remux/clips")
        );
        assert_eq!(obs_app(), DEFAULT_OBS_APP);
        assert!(describe().contains("not there yet"));

        edit(&file, |c| {
            c.chat.url = Some("ws://localhost:9999".into());
            c.web.url = Some("http://localhost:4700/".into());
            c.daemon.motor = Some("obs".into());
            c.daemon.record_dir = Some("~/Films".into());
            c.daemon.music_dir = Some("/music".into());
            c.daemon.clips_dir = Some("~/clips".into());
            c.daemon.obs_app = Some("/Apps/OBS.app".into());
        })
        .unwrap();
        assert_eq!(chat_url().as_deref(), Some("ws://localhost:9999"));
        assert_eq!(web_url(), "http://localhost:4700", "no trailing slash");
        assert_eq!(motor().as_deref(), Some("obs"));
        assert_eq!(record_dir(), PathBuf::from("/Users/somebody/Films"));
        assert_eq!(music_dir(), PathBuf::from("/music"));
        assert_eq!(clips_dir(), PathBuf::from("/Users/somebody/clips"));
        assert_eq!(obs_app(), "/Apps/OBS.app");
        assert!(describe().contains("(config)"));

        std::env::set_var("REMUX_CHAT_URL", "ws://elsewhere:1");
        std::env::set_var("REMUX_WEB", "https://web/");
        std::env::set_var("REMUX_MOTOR", "other");
        std::env::set_var("REMUXD_RECORD_DIR", "/r");
        std::env::set_var("REMUX_MUSIC_DIR", "/m");
        std::env::set_var("REMUX_CLIPS_DIR", "/c");
        std::env::set_var("OBS_APP", "/o");
        assert_eq!(chat_url().as_deref(), Some("ws://elsewhere:1"));
        assert_eq!(web_url(), "https://web");
        assert_eq!(motor().as_deref(), Some("other"));
        assert_eq!(record_dir(), PathBuf::from("/r"));
        assert_eq!(music_dir(), PathBuf::from("/m"));
        assert_eq!(clips_dir(), PathBuf::from("/c"));
        assert_eq!(obs_app(), "/o");
        let said = describe();
        assert!(said.contains("(REMUX_CHAT_URL)") && said.contains("(OBS_APP)"));
        // cargo test (llvm-cov) runs every test in one process: leave the
        // environment as it was found.
        for var in [
            "REMUX_CONFIG",
            "REMUX_CHAT_URL",
            "REMUX_WEB",
            "REMUX_MOTOR",
            "REMUXD_RECORD_DIR",
            "REMUX_MUSIC_DIR",
            "REMUX_CLIPS_DIR",
            "OBS_APP",
        ] {
            std::env::remove_var(var);
        }
        std::env::set_var("HOME", home_before);
    }

    #[test]
    fn a_tilde_is_the_persons_folder() {
        assert_eq!(expand("~/Music"), home().join("Music"));
        assert_eq!(expand("/tmp/x"), PathBuf::from("/tmp/x"));
    }
}
