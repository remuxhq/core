//! The one place shared code knows which operating system it is on: where
//! a person's state, recordings and OBS live, and how the engine runs as a
//! service of their session. A table per OS, chosen at compile time; the
//! rest of the domain, the daemon and the CLI read the table and never an OS
//! name (`make remuxd.seam` fails if they do). What the *machine* does per
//! OS (capture, encoders, libobs) is a motor's table, not this one.

use std::path::{Path, PathBuf};

pub struct Os {
    pub name: &'static str,
    /// Where the socket, the logs and the engine's own files go.
    pub state_dir: fn(&Path) -> PathBuf,
    /// Recordings, under the person's home.
    pub recordings: &'static str,
    /// Where OBS is: the app on macOS, the install prefix on Linux.
    pub obs: &'static str,
    /// The command that opens a URL in the person's browser.
    pub open: &'static [&'static str],
    pub service: Service,
}

/// The engine as a service of the session. Every action is an argv list
/// the shell runs; the table decides the words, the shell only executes.
pub struct Service {
    pub name: &'static str,
    /// The service file (`plist`, `unit`), under the person's home.
    pub file: fn(&Path) -> PathBuf,
    /// The file's text: the engine, its folder, the PATH it was started
    /// from, where its output goes. Never a secret, never an override.
    pub text: fn(&Path, &Path, &str, &Path) -> String,
    /// Put the file in the session and start it (`uid` for launchd's domain).
    pub load: fn(u32, &Path) -> Vec<Vec<String>>,
    pub unload: fn(u32) -> Vec<Vec<String>>,
    /// Start a loaded service that is not running.
    pub kick: fn(u32) -> Vec<Vec<String>>,
    /// The command whose output `loaded` reads.
    pub show: fn(u32) -> Vec<String>,
    pub loaded: fn(Option<&str>) -> Loaded,
}

/// What the service manager says about the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    Not,
    /// Loaded, no process: it exited cleanly, or has not started yet.
    Idle,
    Running {
        pid: u32,
    },
}

/// The launchd label; the plist is named after it.
pub const LABEL: &str = "com.remux.remuxd";

/// The person's home, the one place the environment is asked for it.
pub fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// `~/.config/remux`: the destinations, the session, the config, the history.
pub fn config_dir() -> PathBuf {
    home().join(".config/remux")
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

// ---- macOS -----------------------------------------------------------------

#[cfg(target_os = "macos")]
pub const OS: Os = Os {
    name: "macos",
    state_dir: |home| home.join("Library/Application Support/remux"),
    recordings: "Movies/remux",
    obs: "/Applications/OBS.app",
    open: &["open"],
    service: Service {
        name: "launchd",
        file: |home| home.join(format!("Library/LaunchAgents/{LABEL}.plist")),
        text: plist,
        load: |uid, file| {
            vec![argv(&[
                "launchctl",
                "bootstrap",
                &format!("gui/{uid}"),
                &file.display().to_string(),
            ])]
        },
        unload: |uid| {
            vec![argv(&[
                "launchctl",
                "bootout",
                &format!("gui/{uid}/{LABEL}"),
            ])]
        },
        kick: |uid| {
            vec![argv(&[
                "launchctl",
                "kickstart",
                &format!("gui/{uid}/{LABEL}"),
            ])]
        },
        show: |uid| argv(&["launchctl", "print", &format!("gui/{uid}/{LABEL}")]),
        loaded: printed,
    },
};

/// The agent: the engine, its folder, the PATH it was started from (ffmpeg
/// is on it), back after a crash and only a crash, so `remux quit` stays
/// quit. Nothing in it is a secret; the engine reads those from its files.
pub fn plist(binary: &Path, working_dir: &Path, path_env: &str, output: &Path) -> String {
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{binary}</string>
	</array>
	<key>WorkingDirectory</key>
	<string>{dir}</string>
	<key>EnvironmentVariables</key>
	<dict>
		<key>PATH</key>
		<string>{path}</string>
	</dict>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>StandardOutPath</key>
	<string>{out}</string>
	<key>StandardErrorPath</key>
	<string>{out}</string>
</dict>
</plist>
"#,
        binary = escape(&binary.display().to_string()),
        dir = escape(&working_dir.display().to_string()),
        path = escape(path_env),
        out = escape(&output.display().to_string()),
    )
}

/// What `launchctl print` says, read for the two facts a person asks: is it
/// loaded, and with which pid.
pub fn printed(print: Option<&str>) -> Loaded {
    let Some(print) = print else {
        return Loaded::Not;
    };
    print
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("pid = "))
        .and_then(|pid| pid.trim().parse().ok())
        .map_or(Loaded::Idle, |pid| Loaded::Running { pid })
}

// ---- Linux -----------------------------------------------------------------

#[cfg(target_os = "linux")]
pub const OS: Os = Os {
    name: "linux",
    state_dir: |home| home.join(".local/state/remux"),
    recordings: "Videos/remux",
    obs: "/usr",
    open: &["xdg-open"],
    service: Service {
        name: "systemd --user",
        file: |home| home.join(".config/systemd/user/remuxd.service"),
        text: unit,
        load: |_, _| {
            vec![
                argv(&["systemctl", "--user", "daemon-reload"]),
                argv(&["systemctl", "--user", "enable", "--now", "remuxd"]),
            ]
        },
        unload: |_| vec![argv(&["systemctl", "--user", "disable", "--now", "remuxd"])],
        kick: |_| vec![argv(&["systemctl", "--user", "start", "remuxd"])],
        show: |_| {
            argv(&[
                "systemctl",
                "--user",
                "show",
                "remuxd",
                "-p",
                "LoadState,ActiveState,MainPID",
            ])
        },
        loaded: shown,
    },
};

/// The unit: the engine, its folder, the PATH and the display it was
/// started from (a session's `systemd --user` does not always carry
/// `DISPLAY`, and libobs opens the display at boot), its output in a file
/// beside the socket like launchd's, back after a crash and only a crash.
pub fn unit(binary: &Path, working_dir: &Path, path_env: &str, output: &Path) -> String {
    let display: String = [
        "DISPLAY",
        "XAUTHORITY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
    ]
    .iter()
    .filter_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| format!("Environment={name}={value}\n"))
    })
    .collect();
    format!(
        "[Unit]\nDescription=remux engine\n\n[Service]\nExecStart={}\nWorkingDirectory={}\n\
         Environment=PATH={}\n{display}StandardOutput=append:{out}\nStandardError=append:{out}\n\
         Restart=on-failure\nRestartSec=3\n\n[Install]\nWantedBy=default.target\n",
        binary.display(),
        working_dir.display(),
        path_env,
        out = output.display(),
    )
}

/// What `systemctl --user show remuxd` says: `LoadState=`, `ActiveState=`, `MainPID=`.
pub fn shown(show: Option<&str>) -> Loaded {
    let Some(show) = show else {
        return Loaded::Not;
    };
    let value = |key: &str| {
        show.lines()
            .find_map(|line| line.strip_prefix(key))
            .map(str::trim)
    };
    match (
        value("LoadState="),
        value("ActiveState="),
        value("MainPID="),
    ) {
        (Some("not-found"), _, _) | (None, _, _) => Loaded::Not,
        (_, Some("active"), Some(pid)) => pid
            .parse()
            .ok()
            .filter(|pid| *pid > 0)
            .map_or(Loaded::Idle, |pid| Loaded::Running { pid }),
        _ => Loaded::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plist_names_the_binary_its_folder_the_path_and_comes_back_only_after_a_crash() {
        let text = plist(
            Path::new("/Users/a/.local/share/remux/current/bin/remuxd"),
            Path::new("/Users/a/Library/Application Support/remux"),
            "/opt/homebrew/bin:/usr/bin",
            Path::new("/Users/a/Library/Application Support/remux/remuxd.out.log"),
        );
        assert!(text.contains("<string>com.remux.remuxd</string>"));
        assert!(text.contains("<string>/Users/a/.local/share/remux/current/bin/remuxd</string>"));
        assert!(text.contains("<string>/opt/homebrew/bin:/usr/bin</string>"));
        assert!(text.contains("<key>SuccessfulExit</key>\n\t\t<false/>"));
        assert!(
            !text.contains("REMUX_"),
            "no secret, no override rides in the plist"
        );
        assert!(plist(Path::new("/a&b"), Path::new("/"), "", Path::new("/")).contains("/a&amp;b"));
    }

    #[test]
    fn launchctl_print_is_read_for_the_pid() {
        assert_eq!(printed(None), Loaded::Not);
        assert_eq!(
            printed(Some("com.remux.remuxd = {\n\tstate = not running\n}")),
            Loaded::Idle
        );
        assert_eq!(
            printed(Some(
                "com.remux.remuxd = {\n\tstate = running\n\tpid = 4242\n}"
            )),
            Loaded::Running { pid: 4242 }
        );
    }

    #[test]
    fn the_unit_names_the_binary_its_folder_the_path_the_display_and_comes_back_only_after_a_crash()
    {
        std::env::set_var("DISPLAY", ":7");
        let text = unit(
            Path::new("/home/a/.local/share/remux/current/bin/remuxd"),
            Path::new("/home/a/.local/state/remux"),
            "/usr/bin:/bin",
            Path::new("/home/a/.local/state/remux/remuxd.out.log"),
        );
        assert!(text.contains("ExecStart=/home/a/.local/share/remux/current/bin/remuxd"));
        assert!(text.contains("Environment=PATH=/usr/bin:/bin"));
        assert!(text.contains("Environment=DISPLAY=:7"));
        assert!(text.contains("StandardOutput=append:/home/a/.local/state/remux/remuxd.out.log"));
        assert!(text.contains("Restart=on-failure"));
        assert!(!text.contains("REMUX_"));
    }

    #[test]
    fn systemctl_show_is_read_for_the_pid() {
        assert_eq!(shown(None), Loaded::Not);
        assert_eq!(
            shown(Some("LoadState=not-found\nActiveState=inactive\nMainPID=0")),
            Loaded::Not
        );
        assert_eq!(
            shown(Some("LoadState=loaded\nActiveState=inactive\nMainPID=0")),
            Loaded::Idle
        );
        assert_eq!(
            shown(Some("LoadState=loaded\nActiveState=active\nMainPID=4242")),
            Loaded::Running { pid: 4242 }
        );
    }

    #[test]
    fn this_os_has_a_table_and_every_action_is_words_for_the_shell() {
        let home = Path::new("/home/somebody");
        assert!(!OS.name.is_empty());
        assert!((OS.state_dir)(home).starts_with(home));
        assert!((OS.service.file)(home).starts_with(home));
        assert!(!(OS.service.load)(501, Path::new("/f")).is_empty());
        assert!(!(OS.service.unload)(501).is_empty());
        assert!(!(OS.service.kick)(501).is_empty());
        assert!(!(OS.service.show)(501).is_empty());
        assert!(!OS.open.is_empty());
    }
}
