//! What differs between the operating systems libobs runs on: where OBS is,
//! which plugins to load, and the ids of the sources and encoders each OS
//! provides. One table per OS, chosen at compile time; the pipeline reads
//! the table and never an OS name.

/// Where OBS is: `OBS_APP`, else the OS's usual place (`/Applications/OBS.app`;
/// on Linux the install prefix, `/usr`).
pub fn app() -> String {
    std::env::var("OBS_APP").unwrap_or_else(|_| DEFAULT_APP.into())
}

pub struct Table {
    /// The plugins a motor needs and nothing more: the frontend's want Qt.
    pub modules: &'static [&'static str],
    /// Plugins that may be missing on a given machine; a refusal is logged,
    /// never a reason not to boot (Linux: pipewire, only a Wayland session's).
    pub optional_modules: &'static [&'static str],
    /// The graphics module `obs_reset_video` loads, as a name or a path.
    pub graphics: fn(&str) -> String,
    /// libobs's own effects and shaders.
    pub data: fn(&str) -> String,
    /// A plugin's binary and its data folder.
    pub module: fn(&str, &str) -> (String, String),
    pub screen: Screen,
    pub camera: Camera,
    pub mic: Mic,
    pub system_sound: SystemSound,
    pub video_encoder: &'static str,
    pub audio_encoder: &'static str,
    /// Whether this OS grants the screen, the camera and the microphone
    /// one by one (macOS does; Linux has nothing to ask).
    pub grants: bool,
    /// What the recording helper needs beside the engine before libobs
    /// spawns it, if anything.
    pub helper: fn() -> Result<(), String>,
    /// The face the cards are set in, one this OS has.
    pub card_font: &'static str,
    /// Its weight, by the name the face gives it: Medium on macOS, what the
    /// native motor sets its text in.
    pub card_style: &'static str,
}

/// The source that shows a display or a window, and how it is asked.
pub struct Screen {
    pub source: &'static str,
    /// The list property naming the displays, and the settings key a chosen
    /// display goes in.
    pub displays: &'static str,
    pub display_key: &'static str,
    /// A `type` settings key, for a source that does both (macOS: 0 display,
    /// 1 window); none where display and window are two sources.
    pub kind_key: Option<&'static str>,
    /// The source for a window when it is not the same one.
    pub window_source: &'static str,
    pub windows: &'static str,
    pub window_key: &'static str,
    /// The list property naming the apps, or none where the OS has no such list.
    pub apps: Option<&'static str>,
    /// A portal picks the screen or window in a dialog of the desktop's,
    /// so there is no list to offer: one entry stands for "what the portal
    /// picks", and the settings key below carries the token that restores
    /// the pick on the next boot without asking again.
    pub portal: bool,
    pub token_key: Option<&'static str>,
    /// Whether a display's value in the list is the monitor's own identity
    /// (a UUID, the same after a reboot or a replug) rather than its
    /// position: what a saved layer is opened by, where it is.
    pub stable_displays: bool,
}

pub struct Camera {
    pub source: &'static str,
    pub devices: &'static str,
    pub device_key: &'static str,
    /// A preset the source takes for 720p, where it has presets.
    pub preset: Option<(&'static str, &'static str)>,
    /// Whether asking the source for its devices is safe right now: OBS 30's
    /// v4l2 plugin corrupts the heap when it is probed with no /dev/video*.
    pub probe: fn() -> bool,
}

pub struct Mic {
    pub source: &'static str,
    pub devices: &'static str,
    pub device_key: &'static str,
}

/// What the computer plays (or one application of it).
pub struct SystemSound {
    pub source: &'static str,
    /// Whether it can follow one application (macOS: `type` 1 + `application`).
    pub per_app: bool,
}

#[cfg(target_os = "macos")]
pub const DEFAULT_APP: &str = "/Applications/OBS.app";
/// ScreenCaptureKit names a display by its UUID.
#[cfg(target_os = "macos")]
const STABLE_DISPLAYS: bool = true;
#[cfg(target_os = "macos")]
pub const TABLE: Table = Table {
    optional_modules: &[],
    modules: &[
        "image-source",
        "text-freetype2",
        "rtmp-services",
        "obs-filters",
        "mac-capture",
        "mac-avcapture",
        "obs-ffmpeg",
        "obs-outputs",
        "mac-videotoolbox",
        "coreaudio-encoder",
        "obs-x264",
    ],
    graphics: |app| format!("{app}/Contents/Frameworks/libobs-opengl.dylib"),
    data: |app| format!("{app}/Contents/Frameworks/libobs.framework/Resources/"),
    module: |app, name| {
        (
            format!("{app}/Contents/PlugIns/{name}.plugin/Contents/MacOS/{name}"),
            format!("{app}/Contents/PlugIns/{name}.plugin/Contents/Resources"),
        )
    },
    screen: Screen {
        source: "screen_capture",
        displays: "display_uuid",
        display_key: "display_uuid",
        kind_key: Some("type"),
        window_source: "screen_capture",
        windows: "window",
        window_key: "window",
        apps: Some("application"),
        portal: false,
        token_key: None,
        stable_displays: STABLE_DISPLAYS,
    },
    camera: Camera {
        source: "macos-avcapture",
        devices: "device",
        device_key: "device",
        preset: Some(("preset", "AVCaptureSessionPreset1280x720")),
        probe: || true,
    },
    mic: Mic {
        source: "coreaudio_input_capture",
        devices: "device_id",
        device_key: "device_id",
    },
    system_sound: SystemSound {
        source: "sck_audio_capture",
        per_app: true,
    },
    video_encoder: "com.apple.videotoolbox.videoencoder.ave.avc",
    audio_encoder: "CoreAudio_AAC",
    grants: true,
    helper: helper_beside_us,
    card_font: "Helvetica Neue",
    card_style: "Medium",
};

/// The screen source of this session: on macOS there is one.
#[cfg(target_os = "macos")]
pub fn screen() -> &'static Screen {
    &TABLE.screen
}

/// The recording helper (`obs-ffmpeg-mux`) is a process libobs spawns from
/// beside the running executable, and its libraries sit in `../Frameworks`
/// from there: two links next to `remuxd` point both at OBS.app. A bundle
/// ships them instead.
#[cfg(target_os = "macos")]
fn helper_beside_us() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no folder")?;
    let app = app();
    for (link, to) in [
        (
            dir.join("obs-ffmpeg-mux"),
            format!("{app}/Contents/MacOS/obs-ffmpeg-mux"),
        ),
        (
            dir.join("../Frameworks"),
            format!("{app}/Contents/Frameworks"),
        ),
    ] {
        link_beside(&link, &to)?;
    }
    Ok(())
}

/// One link, made once: a link already there is the link, whoever made it.
/// Two daemons starting at once (the socket tests, under nextest) both saw
/// no link and the second died on `File exists`; a link left dangling by an
/// OBS.app that moved read as absent for the same reason.
#[cfg(target_os = "macos")]
fn link_beside(link: &std::path::Path, to: &str) -> Result<(), String> {
    match std::os::unix::fs::symlink(to, link) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(format!("cannot link {}: {e}", link.display())),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::link_beside;

    #[test]
    fn a_link_already_there_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("remux-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("obs-ffmpeg-mux");
        // Made by the other daemon a moment ago, or left behind by an OBS.app
        // that moved: either way `exists()` says no and `symlink` says yes.
        std::os::unix::fs::symlink("/nowhere/obs-ffmpeg-mux", &link).unwrap();
        assert_eq!(link_beside(&link, "/nowhere/obs-ffmpeg-mux"), Ok(()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(target_os = "linux")]
pub const DEFAULT_APP: &str = "/usr";
/// X11 names a screen by its number: a position, not an identity.
#[cfg(target_os = "linux")]
const STABLE_DISPLAYS: bool = false;
#[cfg(target_os = "linux")]
pub const TABLE: Table = Table {
    // The portal's plugin needs a PipeWire this machine may not have.
    optional_modules: &["linux-pipewire"],
    modules: &[
        "image-source",
        "text-freetype2",
        "rtmp-services",
        "obs-filters",
        "linux-capture",
        "linux-v4l2",
        "linux-pulseaudio",
        "obs-ffmpeg",
        "obs-outputs",
        "obs-x264",
    ],
    // The versioned name, whole: libobs appends `.so` to a bare name and the
    // package ships no `libobs-opengl.so` link. Whichever soname is there.
    graphics: |app| {
        let dir = crate::platform::lib_dir(app);
        std::fs::read_dir(&dir)
            .ok()
            .and_then(|entries| {
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.starts_with("libobs-opengl.so."))
                    .min()
            })
            .map_or_else(
                || format!("{dir}/libobs-opengl.so.1"),
                |n| format!("{dir}/{n}"),
            )
    },
    data: |app| format!("{app}/share/obs/libobs/"),
    module: |app, name| {
        (
            format!("{}/obs-plugins/{name}.so", crate::platform::lib_dir(app)),
            format!("{app}/share/obs/obs-plugins/{name}"),
        )
    },
    // X11: xshm for a display, xcomposite for a window. A Wayland session
    // is `WAYLAND_SCREEN`, chosen by `screen()` at boot.
    screen: Screen {
        source: "xshm_input",
        displays: "screen",
        display_key: "screen",
        kind_key: None,
        window_source: "xcomposite_input",
        windows: "capture_window",
        window_key: "capture_window",
        apps: None,
        portal: false,
        token_key: None,
        stable_displays: STABLE_DISPLAYS,
    },
    camera: Camera {
        source: "v4l2_input",
        devices: "device_id",
        device_key: "device_id",
        preset: None,
        probe: || {
            std::fs::read_dir("/dev")
                .map(|dir| {
                    dir.flatten()
                        .any(|entry| entry.file_name().to_string_lossy().starts_with("video"))
                })
                .unwrap_or(false)
        },
    },
    mic: Mic {
        source: "pulse_input_capture",
        devices: "device_id",
        device_key: "device_id",
    },
    system_sound: SystemSound {
        source: "pulse_output_capture",
        per_app: false,
    },
    video_encoder: "obs_x264",
    audio_encoder: "ffmpeg_aac",
    grants: false,
    // The distribution installs the helper where libobs looks.
    helper: || Ok(()),
    card_font: "DejaVu Sans",
    // DejaVu has no Medium; Book is its regular weight.
    card_style: "Book",
};

/// Wayland: the desktop's portal (xdg-desktop-portal and its backend) picks
/// a screen or a window in its own dialog and hands the frames over
/// PipeWire; `restore_token` brings the same pick back without the dialog.
#[cfg(target_os = "linux")]
pub const WAYLAND_SCREEN: Screen = Screen {
    source: "pipewire-screen-capture-source",
    displays: "",
    display_key: "",
    kind_key: None,
    window_source: "pipewire-screen-capture-source",
    windows: "",
    window_key: "",
    apps: None,
    portal: true,
    token_key: Some("restore_token"),
    stable_displays: false,
};

/// The screen source of this session: the portal's under Wayland, X11's
/// otherwise. `XDG_SESSION_TYPE` is what the session manager says; a
/// `WAYLAND_DISPLAY` with no verdict counts too.
#[cfg(target_os = "linux")]
pub fn screen() -> &'static Screen {
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let wayland = session == "wayland"
        || (session.is_empty() && std::env::var_os("WAYLAND_DISPLAY").is_some());
    if wayland {
        &WAYLAND_SCREEN
    } else {
        &TABLE.screen
    }
}

/// Where the portal's restore token is kept: beside the socket.
pub fn portal_token_path() -> std::path::PathBuf {
    remuxd_domain::socket::default_path().with_file_name("portal.token")
}

/// Where this distribution keeps libobs: the multiarch folder (Debian,
/// Ubuntu), `lib64` (Fedora, openSUSE) or `lib` (Arch), whichever has it.
#[cfg(target_os = "linux")]
pub fn lib_dir(prefix: &str) -> String {
    let triplet = if cfg!(target_arch = "aarch64") {
        "aarch64-linux-gnu"
    } else {
        "x86_64-linux-gnu"
    };
    [
        format!("{prefix}/lib/{triplet}"),
        format!("{prefix}/lib64"),
        format!("{prefix}/lib"),
    ]
    .into_iter()
    .find(|dir| std::path::Path::new(&format!("{dir}/libobs.so.0")).exists())
    .unwrap_or_else(|| format!("{prefix}/lib/{triplet}"))
}
