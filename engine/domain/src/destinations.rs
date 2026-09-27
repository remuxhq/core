//! Destinations kept on this machine: where a live goes, with the key, in
//! one file only the owner can read.
//!
//! `~/.config/remux/destinations.json`, mode 0600, written whole and renamed
//! into place. The engine reads it on every status, so a shell that edited it
//! is believed at once; the key never crosses the socket, because the shell
//! writes the file itself. A row is what the status shows of it, less the key.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::protocol::Destination;

/// A stream key. Serialised as it is, printed as stars.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Key(pub String);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("****")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Kept {
    pub id: i64,
    pub name: String,
    /// `twitch`, `youtube` or `custom`.
    pub platform: String,
    /// The ingest, without the key.
    pub url: String,
    pub key: Key,
    #[serde(default)]
    pub armed: bool,
    #[serde(default)]
    pub sandbox: bool,
}

/// Where the file is: `REMUX_DESTINATIONS`, else `~/.config/remux/destinations.json`.
pub fn path() -> PathBuf {
    if let Some(said) = std::env::var_os("REMUX_DESTINATIONS") {
        return PathBuf::from(said);
    }
    crate::os::config_dir().join("destinations.json")
}

/// The ingest a platform takes by default, so a person types a key and nothing else.
pub fn ingest_of(platform: &str) -> Option<&'static str> {
    match platform {
        "twitch" => Some("rtmp://live.twitch.tv/app"),
        "youtube" => Some("rtmp://a.rtmp.youtube.com/live2"),
        _ => None,
    }
}

pub fn read(path: &Path) -> Vec<Kept> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|said| serde_json::from_str(&said).ok())
        .unwrap_or_default()
}

/// Written whole, renamed into place, readable by the owner alone.
pub fn write(path: &Path, kept: &[Kept]) -> Result<(), String> {
    let said = serde_json::to_string_pretty(kept).map_err(|e| e.to_string())?;
    keep(path, &said)
}

/// A file only the owner reads: written whole, 0600, renamed into place.
pub fn keep(path: &Path, said: &str) -> Result<(), String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|e| format!("cannot make {}: {e}", folder.display()))?;
    }
    let meanwhile = path.with_extension("json.new");
    std::fs::write(&meanwhile, said)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&meanwhile, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("cannot protect {}: {e}", path.display()))?;
    }
    std::fs::rename(&meanwhile, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// A new destination with the next id; the name must be new.
pub fn add(
    kept: &mut Vec<Kept>,
    name: &str,
    platform: &str,
    url: &str,
    key: &str,
) -> Result<i64, String> {
    if kept.iter().any(|k| k.name == name) {
        return Err(format!("there is already a destination called {name}"));
    }
    let id = kept.iter().map(|k| k.id).max().unwrap_or(0) + 1;
    kept.push(Kept {
        id,
        name: name.to_string(),
        platform: platform.to_string(),
        url: url.trim_end_matches('/').to_string(),
        key: Key(key.trim().to_string()),
        armed: true,
        ..Kept::default()
    });
    Ok(id)
}

/// Forget one, by id or by name; whether there was one.
pub fn remove(kept: &mut Vec<Kept>, which: &str) -> bool {
    let before = kept.len();
    kept.retain(|k| k.name != which && k.id.to_string() != which);
    kept.len() != before
}

/// Where the stream goes: the ingest with the key on it, and Twitch's
/// bandwidth-test door for a sandbox, so a rehearsal never reaches the channel.
pub fn outlet(kept: &Kept) -> String {
    let door = if kept.platform == "twitch" && kept.sandbox {
        "?bandwidthtest=true"
    } else {
        ""
    };
    format!("{}/{}{door}", kept.url, kept.key.0)
}

/// What a face sees of it: everything but the key.
pub fn row(kept: &Kept) -> Destination {
    Destination {
        id: kept.id,
        name: kept.name.clone(),
        platform: kept.platform.clone(),
        status: "off".into(),
        armed: kept.armed,
        sandbox: kept.sandbox,
        connected: !kept.key.0.is_empty() && !kept.url.is_empty(),
        account: None,
        category: None,
        category_id: None,
        viewers: None,
        viewers_peak: None,
        trouble: None,
        title: None,
        description: None,
        channel: None,
    }
}

/// The destinations file as the `Watching` of a machine with no account:
/// rows, armed or not, one door each. Every ask writes the file; every read
/// reads it, so a shell that edited it is believed at once. What needs a
/// platform's API (a title, the chat, the viewers) needs an account, and
/// says so.
pub struct Local {
    pub path: PathBuf,
}

const NEEDS_AN_ACCOUNT: &str = "needs an account: remux login";

impl Local {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn change(&self, id: i64, change: impl FnOnce(&mut Kept)) -> Result<(), String> {
        let mut kept = read(&self.path);
        let one = kept
            .iter_mut()
            .find(|k| k.id == id)
            .ok_or_else(|| format!("no destination {id}"))?;
        change(one);
        write(&self.path, &kept)
    }
}

impl crate::engine::Watching for Local {
    fn reachable(&self) -> bool {
        true
    }
    fn server(&self) -> Option<String> {
        Some(self.path.display().to_string())
    }
    fn destinations(&self) -> Vec<Destination> {
        read(&self.path).iter().map(row).collect()
    }
    fn viewers(&self) -> Option<u32> {
        None
    }
    fn outlets(&self) -> Vec<crate::engine::Outlet> {
        read(&self.path)
            .iter()
            .filter(|k| k.armed && !k.key.0.is_empty())
            .map(|k| crate::engine::Outlet {
                id: k.id,
                url: outlet(k),
            })
            .collect()
    }
    fn arm(&self, adapter: i64, on: bool) -> Result<(), String> {
        self.change(adapter, |k| k.armed = on)
    }
    fn sandbox(&self, adapter: i64, on: bool) -> Result<(), String> {
        self.change(adapter, |k| k.sandbox = on)
    }
    fn retitle(&self, _: i64, _: Option<&str>, _: Option<&str>) -> Result<(), String> {
        Err(NEEDS_AN_ACCOUNT.into())
    }
    fn announce(&self, _adapter: i64) -> Result<(), String> {
        Err(NEEDS_AN_ACCOUNT.into())
    }
    fn disconnect(&self, _adapter: i64) -> Result<(), String> {
        Err(NEEDS_AN_ACCOUNT.into())
    }
    fn categorize(&self, _adapter: i64, _id: &str, _name: &str) -> Result<(), String> {
        Err(NEEDS_AN_ACCOUNT.into())
    }
    fn search_categories(&self, _adapter: i64, _query: &str) -> Result<(), String> {
        Err(NEEDS_AN_ACCOUNT.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("remux-destinations-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("destinations.json")
    }

    #[test]
    fn a_destination_is_kept_in_a_file_only_the_owner_reads_and_read_back_whole() {
        let path = scratch("kept");
        let mut kept = Vec::new();
        let id = add(
            &mut kept,
            "twitch",
            "twitch",
            "rtmp://live.twitch.tv/app/",
            "live_abc",
        )
        .unwrap();
        assert_eq!(id, 1);
        assert!(
            add(&mut kept, "twitch", "custom", "rtmp://x", "k").is_err(),
            "one name once"
        );
        write(&path, &kept).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(read(&path), kept);
        assert_eq!(format!("{:?}", kept[0].key), "****", "a key never prints");
        assert!(remove(&mut kept, "1"));
        assert!(kept.is_empty());
    }

    #[test]
    fn the_outlet_carries_the_key_and_twitch_s_sandbox_door() {
        let mut kept = Vec::new();
        add(
            &mut kept,
            "t",
            "twitch",
            "rtmp://live.twitch.tv/app",
            "live_abc",
        )
        .unwrap();
        assert_eq!(outlet(&kept[0]), "rtmp://live.twitch.tv/app/live_abc");
        kept[0].sandbox = true;
        assert_eq!(
            outlet(&kept[0]),
            "rtmp://live.twitch.tv/app/live_abc?bandwidthtest=true"
        );
        assert_eq!(row(&kept[0]).platform, "twitch");
    }

    #[test]
    fn the_local_app_arms_and_hands_out_outlets_off_the_file_and_refuses_the_rest() {
        use crate::engine::Watching;
        let path = scratch("local");
        let mut kept = Vec::new();
        add(
            &mut kept,
            "yt",
            "youtube",
            "rtmp://a.rtmp.youtube.com/live2",
            "xyz",
        )
        .unwrap();
        add(
            &mut kept,
            "tw",
            "twitch",
            "rtmp://live.twitch.tv/app",
            "abc",
        )
        .unwrap();
        write(&path, &kept).unwrap();
        let local = Local::new(path);
        assert_eq!(local.outlets().len(), 2);
        local.arm(2, false).unwrap();
        assert_eq!(local.outlets().len(), 1);
        assert!(!local.destinations()[1].armed);
        assert!(local.arm(9, true).is_err());
        assert_eq!(
            local.retitle(1, Some("hello"), None).unwrap_err(),
            "needs an account: remux login"
        );
        assert!(local.announce(1).is_err());
    }
}
