//! What somebody typed, as a command the engine answers.
//!
//! A decision and not a transport, so it is here and it is tested by
//! `cargo test`: the binary that uses it opens a socket, writes a line and
//! prints what comes back, and that is all it does.
//!
//! The words are the ones the engine this replaces already answered to, because
//! they are in people's fingers and in their shell history.

use crate::card::Card;
use crate::protocol::{Command, Devices, Framed, Grant, Named, Reply, Status};

/// Read a command out of the words after the program's own name.
///
/// The error is what a person reads when they get it wrong, so it says what
/// was expected rather than that something was invalid.
/// Whether the words ask to keep reading the chat (`chat -f`, `chat --follow`,
/// `chat follow`), and the words with that taken out. Following is the
/// shell's job, not the engine's: the command on the wire is `chat`, with
/// `since` moving.
pub fn follow(words: &[String]) -> (bool, Vec<String>) {
    let follows = |w: &String| matches!(w.as_str(), "-f" | "--follow" | "follow");
    if matches!(
        words.first().map(String::as_str),
        Some("chat") | Some("levels") | Some("meters")
    ) && words[1..].iter().any(follows)
    {
        (
            true,
            words.iter().filter(|w| !follows(w)).cloned().collect(),
        )
    } else {
        (false, words.to_vec())
    }
}

/// What separates two messages of the chat on a screen.
pub const CHAT_RULE: &str = "────────────────────────────────────────";

/// How the chat is dressed: in colour for a person at a terminal, plain for
/// a pipe, a file, a test, or a terminal that asked for none (`NO_COLOR`).
/// The colours are this shell's own, put around a viewer's words after
/// those have been made plain; nothing a viewer typed becomes a sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Plain,
    Colour,
}

impl Ink {
    /// Colour when the output is a terminal and nobody said otherwise.
    pub fn for_terminal(is_terminal: bool, no_color: bool) -> Self {
        if is_terminal && !no_color {
            Self::Colour
        } else {
            Self::Plain
        }
    }

    fn wrap(self, code: &str, text: &str) -> String {
        match self {
            Self::Plain => text.to_string(),
            Self::Colour => format!("\x1b[{code}m{text}\x1b[0m"),
        }
    }

    /// The platform, in its own colour, so a glance says where a line came
    /// from: Twitch's purple, YouTube's red, anything else in cyan.
    fn platform(self, name: &str) -> String {
        let code = match name {
            "twitch" => "1;38;5;141",
            "youtube" => "1;38;5;203",
            _ => "1;36",
        };
        self.wrap(code, name)
    }

    fn from(self, name: &str) -> String {
        self.wrap("1", name)
    }

    fn rule(self) -> String {
        self.wrap("2", CHAT_RULE)
    }
}

/// `render`, with the chat dressed as asked. Every other reply reads the same
/// in colour and out of it.
pub fn render_with(reply: &Reply, ink: Ink) -> String {
    match reply {
        // "Not signed in" rather than an empty screen: a room can be quiet
        // and a server can be down and they look the same until one says so.
        Reply::Chat { reachable, .. } if !reachable => {
            "no chat wire: remux login, or remux chat --url ws://...".into()
        }
        Reply::Chat { lines, .. } if lines.is_empty() => "nobody has said anything".into(),
        // One message, two lines: where it came from and who, then what was
        // said, indented; a rule between messages, since a name that runs
        // long pushed the words off their column and the messages ran into
        // each other on the screen. Oldest first, the way a person reads it.
        Reply::Chat { lines, .. } => lines
            .iter()
            .map(|line| {
                format!(
                    "#{} {}  {}\n  {}",
                    line.seq,
                    ink.platform(&line.platform),
                    ink.from(&plain(&line.from)),
                    plain(&line.body)
                )
            })
            .collect::<Vec<_>>()
            .join(&format!("\n{}\n", ink.rule())),
        other => render(other),
    }
}

/// What `chat -f` prints for messages that arrived after some were already
/// on the screen: the rule first. The rule goes between messages, and the
/// message before these is the last one printed, so without it every batch
/// that landed a second apart ran into the one before.
pub fn render_more(reply: &Reply, ink: Ink) -> String {
    format!("{}\n{}", ink.rule(), render_with(reply, ink))
}

pub fn parse(words: &[String]) -> Result<Command, String> {
    let (verb, rest) = words.split_first().ok_or_else(usage)?;
    let joined = rest.join(" ");
    match verb.as_str() {
        "status" => Ok(Command::Status),
        "levels" | "meters" => Ok(Command::Levels),
        "watching" => Ok(Command::Watching {
            on: !matches!(joined.as_str(), "off" | "false" | "0"),
        }),
        // `remux shot` is the scene, `remux shot camera` is the self-view.
        "shot" => Ok(Command::Shot {
            of: match joined.as_str() {
                "" | "scene" => Framed::Scene,
                "camera" | "cam" => Framed::Camera,
                "screen" => Framed::Screen,
                other => {
                    return Err(format!(
                        "a shot is of the scene, the camera or the screen, not {other}"
                    ))
                }
            },
        }),
        "grants" | "permissions" => Ok(Command::Grants),
        "chat" => Ok(Command::Chat {
            since: 0,
            follow: false,
        }),
        // `remux chat -f`: the same, then again every second for what is new,
        // the way `tail -f` reads a file. The flag is the shell's (`follow`),
        // the command on the wire is the same one.
        // `remux hide 42`: that line of chat, off every face.
        "hide" => {
            let seq = rest
                .first()
                .ok_or("hide needs the line's number")?
                .parse()
                .map_err(|_| "a line's number is a number".to_string())?;
            Ok(Command::Hide { seq })
        }
        "devices" | "sources" => Ok(Command::Devices),
        // `remux plan` says what live would do; `remux live --confirm <plan>`
        // does it only if that is still true. `remux live` alone asks a
        // person at a terminal, or is refused where there is nobody to ask.
        "plan" => Ok(Command::Plan),
        "live" | "go-live" => match (rest.first().map(String::as_str), rest.get(1)) {
            (Some("--confirm"), Some(plan)) => Ok(Command::Live {
                plan: plan
                    .parse()
                    .map_err(|_| "--confirm takes the number the plan printed".to_string())?,
            }),
            (Some("--confirm"), None) => Err("--confirm takes the number the plan printed".into()),
            (Some("--yes"), _) | (None, _) => Ok(Command::GoLive),
            (Some(other), _) => Err(format!("live takes --confirm <plan> or --yes, not {other}")),
        },
        "stop" => Ok(Command::Stop),
        "quit" => Ok(Command::Quit),

        "screen" => {
            let display = rest
                .first()
                .ok_or("screen needs a display id, which `remux devices` lists")?;
            display
                .parse()
                .map(|display| Command::Screen { display })
                .map_err(|_| format!("{display} is not a display id; `remux devices` lists them"))
        }
        "window" => {
            if joined.is_empty() {
                return Err("window needs part of a title, as in `remux window ghostty`".into());
            }
            Ok(Command::Window { query: joined })
        }
        "camera" => Ok(Command::Camera {
            device: off_or(&joined),
        }),
        "mic" => Ok(Command::Mic {
            device: off_or(&joined),
        }),
        // `remux scene save code`, `remux scene code`, `remux scene rm code`.
        "scene" => match (rest.first().map(String::as_str), rest.get(1)) {
            (Some("save") | Some("keep"), Some(name)) => {
                Ok(Command::SceneSave { name: name.clone() })
            }
            (Some("rm") | Some("forget"), Some(name)) => {
                Ok(Command::SceneForget { name: name.clone() })
            }
            (Some("save") | Some("keep") | Some("rm") | Some("forget"), None) => {
                Err("scene save|rm takes the scene's name".into())
            }
            (Some(name), None) => Ok(Command::SceneSwitch {
                name: name.to_string(),
            }),
            _ => Err("scene takes a name to switch to, or save|rm <name>".into()),
        },
        // `remux layout top-left 50% circle`, in any order, any subset.
        "layout" => Ok(Command::Layout {
            patch: layout_patch(rest)?,
        }),
        // `remux hear Spotify, Brave`: those apps' sound alone; `hear off`,
        // the whole screen's again.
        "hear" => Ok(Command::Hear {
            apps: match joined.as_str() {
                "" => return Err("hear takes app names, or off".into()),
                "off" | "all" | "screen" => Vec::new(),
                names => names
                    .split(',')
                    .map(|n| n.trim().to_string())
                    .filter(|n| !n.is_empty())
                    .collect(),
            },
        }),
        "denoise" => Ok(Command::Denoise {
            on: on_or(&joined)?,
        }),
        "mirror" => Ok(Command::Mirror {
            on: on_or(&joined)?,
        }),
        "share" => Ok(Command::Share {
            on: on_or(&joined)?,
        }),
        "mute" => Ok(Command::Mute {
            on: on_or(&joined)?,
        }),
        "monitor" => Ok(Command::Monitor {
            on: on_or(&joined)?,
        }),
        "stream-music" => Ok(Command::StreamMusic {
            on: on_or(&joined)?,
        }),
        "screen-sound" => Ok(Command::ScreenSound {
            on: on_or(&joined)?,
        }),
        "music" => match joined.as_str() {
            "" | "on" | "off" => Ok(Command::Music {
                on: on_or(&joined)?,
            }),
            // `remux music jazz` is what a person means, and it is a genre.
            name => Ok(Command::Genre { name: name.into() }),
        },
        "next" | "skip" => Ok(Command::NextTrack),
        // `remux play clap`: once, over everything. `remux clips` lists them.
        "play" if joined.is_empty() => Err("play takes a clip's name or a file".into()),
        "play" => Ok(Command::Clip { name: joined }),

        "vol" | "volume" => Ok(Command::Volume {
            level: percentage(&joined, "volume")?,
        }),
        "mvol" | "music-volume" => Ok(Command::MusicVolume {
            level: percentage(&joined, "music-volume")?,
        }),
        "duck" => {
            let db: f64 = joined
                .parse()
                .map_err(|_| "duck takes decibels, as in `remux duck 18`".to_string())?;
            // Said as a positive number and meant as a step downward, which is
            // how the panel labels it and how anybody says it out loud.
            Ok(Command::Duck { db: -db.abs() })
        }

        "card" => match joined.as_str() {
            "" | "live" => Ok(Command::Card { which: Card::Live }),
            "starting" | "starting-soon" => Ok(Command::Card {
                which: Card::StartingSoon,
            }),
            "brb" | "back" | "back-in-a-moment" => Ok(Command::Card {
                which: Card::BackInAMoment,
            }),
            other => Err(format!(
                "{other} is not a card; there is live, starting and brb"
            )),
        },
        // `remux words starting Back in five`: what the card says.
        "words" | "card-text" => {
            let which = match rest.first().map(String::as_str) {
                Some("starting") => Card::StartingSoon,
                Some("brb") | Some("back") => Card::BackInAMoment,
                _ => return Err("words takes starting or brb, then the words".into()),
            };
            let text = rest[1..].join(" ");
            if text.is_empty() {
                return Err("words takes starting or brb, then the words".into());
            }
            Ok(Command::CardText { which, text })
        }
        "countdown" => Ok(Command::Countdown {
            seconds: match joined.as_str() {
                "" => None,
                minutes => Some(
                    minutes
                        .parse::<u32>()
                        .map_err(|_| "countdown takes minutes, as in `remux countdown 5`")?
                        * 60,
                ),
            },
        }),
        "panic" | "cut" | "hide-everything" => Ok(Command::HideEverything),

        "record" => match joined.as_str() {
            "" | "start" => Ok(Command::RecordStart),
            "stop" => Ok(Command::RecordStop),
            other => Err(format!("record takes start or stop, not {other}")),
        },

        // One threshold at a time, because that is how a person tunes a gate:
        // `remux gate full 0.2`. The names are the ones the panel's sliders
        // carry and the ones in `gate::GateParams`.
        // `remux gate reset`: the seven defaults. `remux gate opens -30`: the
        // panel's words, in dB; `remux gate full 0.03`: the wire's, as they are.
        "gate" if joined == "reset" => Ok(Command::Gate {
            patch: serde_json::to_value(crate::gate::GateParams::default())
                .map_err(|e| e.to_string())?,
        }),
        "gate" => {
            let (name, value) = (rest.first(), rest.get(1));
            let (Some(name), Some(value)) = (name, value) else {
                return Err(
                    "gate takes a threshold and a number, as in `remux gate opens -30`.\n\
                     in dB: opens, highs, closed, keys; in ms: hold_ms, attack_ms, hf_attack_ms;\n\
                     as the wire has them: hf, full, floor, keys_boost; or `gate reset`"
                        .into(),
                );
            };
            let number: f64 = value
                .parse()
                .map_err(|_| format!("{value} is not a number"))?;
            let (name, number) = match name.as_str() {
                "opens" => ("full", crate::levels::amplitude(number)),
                "highs" => ("hf", crate::levels::amplitude(number)),
                "closed" => ("floor", crate::levels::amplitude(number)),
                "keys" => ("keys_boost", crate::levels::amplitude(number)),
                other => (other, number),
            };
            let name = &name.to_string();
            const KNOWN: [&str; 7] = [
                "hf",
                "full",
                "floor",
                "hold_ms",
                "attack_ms",
                "hf_attack_ms",
                "keys_boost",
            ];
            if !KNOWN.contains(&name.as_str()) {
                return Err(format!(
                    "{name} is not a threshold; they are {}",
                    KNOWN.join(", ")
                ));
            }
            Ok(Command::Gate {
                patch: serde_json::json!({ name.as_str(): number }),
            })
        }

        // `remux title 2 Rust at midnight`: the id, then the words.
        "title" | "describe" => {
            let adapter: i64 = rest
                .first()
                .ok_or(format!("{verb} needs a destination id, then the words"))?
                .parse()
                .map_err(|_| "a destination id is a number".to_string())?;
            let words = rest[1..].join(" ");
            if words.is_empty() {
                return Err(format!(
                    "{verb} needs the words, as in `remux {verb} 2 Rust at midnight`"
                ));
            }
            Ok(Command::Retitle {
                adapter,
                title: (verb == "title").then_some(words.clone()),
                description: (verb == "describe").then_some(words),
            })
        }
        // `remux announce 2`: tell that destination's platform the title now.
        "announce" | "update" => {
            let adapter = rest
                .first()
                .ok_or("announce needs a destination id")?
                .parse()
                .map_err(|_| "a destination id is a number".to_string())?;
            Ok(Command::Announce { adapter })
        }
        // `remux delete 42`: that line of chat, out of the platform's chat for everybody.
        "delete" => {
            let seq = rest
                .first()
                .ok_or("delete needs the line's number")?
                .parse()
                .map_err(|_| "a line's number is a number".to_string())?;
            Ok(Command::Delete { seq })
        }
        // `remux category 2 509670 Science & Technology`: file that destination's live.
        "category" => {
            let adapter = rest
                .first()
                .ok_or("category needs a destination id")?
                .parse()
                .map_err(|_| "a destination id is a number".to_string())?;
            let id = rest
                .get(1)
                .ok_or("category needs the category's id")?
                .clone();
            let name = rest[2..].join(" ");
            if name.is_empty() {
                return Err("category needs the category's name".into());
            }
            Ok(Command::Categorize { adapter, id, name })
        }
        // `remux categories 2 science`: where that destination's live can be filed.
        "categories" => {
            let adapter = rest
                .first()
                .ok_or("categories needs a destination id")?
                .parse()
                .map_err(|_| "a destination id is a number".to_string())?;
            Ok(Command::Categories {
                adapter,
                query: rest[1..].join(" "),
            })
        }
        // `remux disconnect 2`: forget that destination's platform account.
        "disconnect" => {
            let adapter = rest
                .first()
                .ok_or("disconnect needs a destination id")?
                .parse()
                .map_err(|_| "a destination id is a number".to_string())?;
            Ok(Command::Disconnect { adapter })
        }
        // `remux sandbox 2` / `remux sandbox 2 off`: a rehearsal nobody is told about.
        "sandbox" => {
            let adapter = rest
                .first()
                .ok_or("sandbox needs a destination id")?
                .parse()
                .map_err(|_| "a destination id is a number".to_string())?;
            let on = match rest.get(1).map(String::as_str) {
                None | Some("on") => true,
                Some("off") => false,
                Some(other) => return Err(format!("{other}: on or off")),
            };
            Ok(Command::Sandbox { adapter, on })
        }
        "arm" | "disarm" => {
            let adapter = rest
                .first()
                .ok_or("arm needs a destination id")?
                .parse()
                .map_err(|_| "a destination id is a number".to_string())?;
            Ok(Command::Arm {
                adapter,
                on: verb == "arm",
            })
        }

        other => Err(format!(
            "{other} is not a thing this engine does.\n{}",
            usage()
        )),
    }
}

/// A device name, or nothing at all, which is how a camera is closed.
fn off_or(said: &str) -> Option<String> {
    match said {
        "" | "off" | "none" => None,
        name => Some(name.to_string()),
    }
}

/// A switch. Saying nothing means turning it on, because that is what a person
/// typing `remux mute` means.
fn on_or(said: &str) -> Result<bool, String> {
    match said {
        "" | "on" | "true" | "yes" => Ok(true),
        "off" | "false" | "no" => Ok(false),
        other => Err(format!("{other} is not on or off")),
    }
}

/// A fader, said the way the panel labels it: a percentage, not a fraction.
fn percentage(said: &str, what: &str) -> Result<f64, String> {
    let cleaned = said.trim_end_matches('%');
    cleaned
        .parse::<f64>()
        .map(|number| number / 100.0)
        .map_err(|_| format!("{what} takes a percentage, as in `remux {what} 80`"))
}

/// What the engine said, for a person rather than for a program.
///
/// One line for anything that fits on one, because the common use of this is
/// inside a shell prompt or a keybinding and not at a terminal being read.
pub fn render(reply: &Reply) -> String {
    match reply {
        Reply::Ok => "ok".into(),
        Reply::Error { message } => format!("no: {message}"),
        Reply::Status(status) => render_status(status),
        Reply::Plan(plan) => render_plan(plan),
        Reply::Devices(devices) => render_devices(devices),
        // dB, because that is what the meters are marked in and what a person
        // reading this in a terminal is comparing against them.
        // The panel's meters on one line: bar and held peak for the mic, the
        // gate's lamp with the gain it applies, what its two detectors hear,
        // then the mix and the bed with how far the duck has it.
        Reply::Levels { hearing, mixing } => format!(
            "mic {:.1} dB (peak {:.1}) gate {}{}, voice {:.0} highs {:.0}, mix {:.1} dB, music {:.1} dB{}",
            hearing.level_db,
            hearing.peak_db,
            if hearing.gate_open { "open" } else { "closed" },
            if hearing.gate_open {
                format!(" {:+.1} dB", hearing.gain_db)
            } else {
                String::new()
            },
            crate::levels::decibels(hearing.gate_levels.full),
            crate::levels::decibels(hearing.gate_levels.hf),
            mixing.level_db,
            mixing.music_db,
            if mixing.ducked_db < -0.5 {
                format!(" (ducking {:.0})", mixing.ducked_db)
            } else {
                String::new()
            }
        ),
        // Every one of them, always, including the ones that are fine: a
        // person reading this is looking for the one that is not, and a list
        // with the good ones left out makes them count.
        Reply::Grants {
            screen,
            camera,
            microphone,
        } => format!(
            "screen {}, camera {}, microphone {}",
            grant(screen),
            grant(camera),
            grant(microphone)
        ),
        // The chat, plain: what a pipe or a test reads. See `render_with`.
        chat @ Reply::Chat { .. } => render_with(chat, Ink::Plain),
        // A shell prints what it can read. The bytes are for a panel; here
        // the useful thing is that a picture exists and how big it is.
        Reply::Shot {
            width,
            height,
            jpeg,
        } => {
            format!("{width}x{height}, {} bytes of jpeg", jpeg.len() * 3 / 4)
        }
    }
}

fn grant(grant: &Grant) -> &'static str {
    match grant {
        Grant::Granted => "ok",
        Grant::NotAsked => "not asked",
        Grant::Refused => "refused (System Settings > Privacy)",
    }
}

/// The plan the way a person confirms it: what leaves, where to, and what
/// would stop it, with the fingerprint `live --confirm` takes on the last line.
fn render_plan(plan: &crate::plan::Plan) -> String {
    let mut lines = Vec::new();
    if plan.on_air {
        lines.push("already on air".to_string());
    }
    lines.push(format!("picture   {}", plan.picture));
    lines.push(format!(
        "camera    {}{}",
        plan.camera.as_deref().unwrap_or("off"),
        if plan.mirrored { ", mirrored" } else { "" }
    ));
    lines.push(format!(
        "mic       {}{}",
        plan.mic.as_deref().unwrap_or("off"),
        if plan.muted { ", muted" } else { "" }
    ));
    lines.push(format!(
        "music     {}{}",
        plan.music.as_deref().unwrap_or("off"),
        if plan.music.is_some() && !plan.music_to_stream {
            ", not sent"
        } else {
            ""
        }
    ));
    lines.push(format!(
        "screen sound {}",
        if plan.screen_sound { "sent" } else { "off" }
    ));
    if let Some(card) = &plan.card {
        lines.push(format!("card      {card}"));
    }
    if plan.recording {
        lines.push("recording".into());
    }
    lines.push("destinations".into());
    for d in &plan.destinations {
        lines.push(format!(
            "  {} {:<3} {:<12} {:<8} {}{}{}",
            if d.armed { "\u{25cf}" } else { "\u{25cb}" },
            d.id,
            d.name,
            d.platform,
            if d.sandbox { "sandbox " } else { "" },
            d.title.as_deref().unwrap_or("(no title)"),
            match &d.why_not {
                Some(why) => format!("  ! {why}"),
                None => String::new(),
            }
        ));
        if let Some(category) = &d.category {
            lines.push(format!("        {category}"));
        }
    }
    for blocker in &plan.blockers {
        lines.push(format!("! {blocker}"));
    }
    lines.push(format!("plan {}", plan.fingerprint));
    lines.join("\n")
}

fn render_status(status: &Status) -> String {
    let mut said = vec![if status.on_air {
        "on air".to_string()
    } else {
        "off air".to_string()
    }];
    if status.recording {
        said.push("recording".into());
    }
    if let Some(card) = &status.card {
        said.push(format!("card {card:?}"));
    }
    said.push(match &status.screen {
        Some(screen) => format!("screen {screen}"),
        None => "no screen".into(),
    });
    if let Some(camera) = &status.camera {
        said.push(format!("camera {camera}"));
    }
    // Said only when it is on: it is the exception, and the one an
    // operator wants to be reminded of before a call comes in.
    if status.screen_sound {
        said.push("screen sound out".into());
    }
    match (&status.mic, status.muted) {
        (Some(mic), true) => said.push(format!("mic {mic} (muted)")),
        // A microphone that is chosen and not delivering says why on the same
        // line: the name alone read as a working one the night it had left.
        (Some(mic), false) => said.push(match &status.hearing.complaint {
            Some(why) => format!("mic {mic} ({why})"),
            None => format!("mic {mic}"),
        }),
        // Muted with nothing open still has to say so: somebody who typed
        // `remux mute` and read back a line with no word for it in would
        // reasonably type it again.
        (None, true) => said.push("muted".into()),
        (None, false) => {}
    }
    if let Some(music) = &status.music {
        said.push(format!("music {music}"));
    }
    if let Some(viewers) = status.viewers {
        said.push(format!("{viewers} watching"));
    }
    // What is actually coming out, which is the only part that can disagree
    // with everything above it.
    said.push(format!(
        "{}x{} at {} frames",
        status.flowing.width, status.flowing.height, status.flowing.frames
    ));
    said.join(", ")
}

fn render_devices(devices: &Devices) -> String {
    let mut lines = Vec::new();
    let mut list = |what: &str, named: &[Named]| {
        if named.is_empty() {
            return;
        }
        lines.push(format!("{what}:"));
        for one in named {
            lines.push(format!("  {:<38} {}", one.name, one.id));
        }
    };
    list("screens", &devices.screens);
    list("cameras", &devices.cameras);
    list("mics", &devices.mics);
    list("apps", &devices.apps);
    list("music", &devices.genres);
    // The windows last and only counted: there are dozens, and `remux window`
    // takes part of a title rather than an id, so the list is not the way in.
    if !devices.windows.is_empty() {
        lines.push(format!(
            "windows: {} of them, named by part of a title",
            devices.windows.len()
        ));
    }
    lines.join("\n")
}

pub fn usage() -> String {
    "what it can do (add --json for a program):\n  \
     status [-v], destinations, log [-f], levels [-f], devices, shot [camera|screen], grants\n  \
     chat [-f], hide <n>, delete <n>\n  \
     screen <id>, window <part of a title>, camera <name|off>, mic <name|off>\n  \
     layout [tl|tr|bl|br] [<n>%] [rect|square|circle] [plain|sepia|mono|noir] [overlay|columns|bounce]\n  \
     scene save <name>|<name>|rm <name>, scenes\n  \
     mirror|share|mute|monitor|stream-music|screen-sound|denoise [on|off], hear <apps|off>\n  \
     music [on|off|<genre>], next, vol <%>, mvol <%>, duck <dB>, play <clip|file>, clips\n  \
     card [live|starting|brb], countdown [minutes], panic (everything off, sound included), shot --out <file.jpg>\n  \
     gate [reset|opens|highs|closed|keys <dB>|hold_ms|attack_ms|hf_attack_ms <ms>]\n  \
     words starting|brb <words>, schema\n  \
     plan, live [--confirm <plan>|--yes], stop, record [start|stop]\n  \
     destination add twitch|youtube|custom <name> [--url <rtmp>] [--key -|--key-file <f>]\n  \
     destination rm <id|name>\n  \
     history: every live on record, newest first (--json for the numbers)\n  \
     health, wait on-air|off-air|picture|recording|not-recording|live <id|name> [--for <s>], guide\n  \
     login [--url <web>] (a code typed on the web, once), logout\n  \
     chat [-f], chat --url ws://host:port (a chat wire of your own; - forgets it)\n  \
     config: what is in effect (~/.config/remux/config.toml), daemon start|stop|restart|status|log|path\n  \
     bug [--open]: a report for an issue, keys redacted; --open fills GitHub's form for you to submit\n  \
     arm|disarm <id>, sandbox <id> [on|off]\n  \
     title <id> <words>, describe <id> <words>, announce <id>, disconnect <id>,\n  \
     category <id> <category id> <name>, categories <id> <words>, quit"
        .to_string()
}

/// How to drive the engine from a script. `remux guide`.
pub const GUIDE: &str = "\
remux, from a script

  Every verb answers prose for a person and, with --json, one JSON value for a
  program; `remux schema` prints the shapes. Exit codes: 0 done, 1 the engine
  refused or is not there (the reason on stderr), 2 the words were wrong.

  1. remux health           what stands in the way of a live, one line each
  2. remux destinations     the rows: remux destination add custom main --url <rtmp> --key -,
                            or remux login and the account's destinations (the web's)
  3. remux screen <id>, remux camera <name>, remux mic <name>, remux title <id> <words>
  4. remux plan --json      what go live would do, and a fingerprint
  5. remux live --confirm <fingerprint>   on air only if nothing moved since the plan
     (a person types `remux live` and answers y; --yes is for a person too)
  6. remux wait on-air --for 20           then remux wait live main
  7. remux chat -f --json (pushed, one JSON line each), remux log -f --json, remux levels -f --json
  8. remux stop, remux history

  With an account (remux login), the live goes to the web's relay, one stream out of
  this machine, and the chat comes down the web's wire; without one, one ffmpeg per
  destination, here, and the chat from a wire of your own (remux chat --url ws://...,
  taken up at once, no restart; one JSON object per frame:
  {\"line\":{id,platform,channel,from,body}} in, {\"delete\":{id,channel}} out; docs/wire.md).
  `remux config` is what is in effect and where it came from: the environment, then
  ~/.config/remux/config.toml, then the defaults. `remux daemon start` runs the engine
  as a service of your session; `remux daemon status|log -f|stop` when something is off.
  Something wrong: `remux bug` prints a report (versions, health, config, the last log
  lines, keys redacted) to paste into an issue; `remux bug --open` opens GitHub's form
  with it filled in. A person submits it, never a script.
  A key is never typed on a command line: --key - reads stdin, --key-file a file.
  A test live is a sandbox live: remux sandbox <id> on before arming a real platform.
  Everything the engine knows is on `remux status --json`, once.
";

/// A stranger's words as text and nothing else, before the line reaches a
/// terminal, where an escape sequence writes the clipboard or rewrites the
/// screen (#8). An escape goes with its whole sequence, so no payload is
/// left standing as words nobody typed: a CSI to its final byte, an OSC to
/// its BEL or ST, any other to the character after it. Every other control
/// but the tab is dropped, the 8-bit C1 controls with them.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(next) = chars.next() {
                        match next {
                            '\x07' => break,
                            '\x1b' => {
                                chars.next();
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_chat_reads_one_message_at_a_time_with_a_rule_between() {
        use crate::protocol::{ChatLine, Reply};
        let line = |seq, platform: &str, from: &str, body: &str| ChatLine {
            seq,
            from: from.into(),
            body: body.into(),
            platform: platform.into(),
            id: String::new(),
            channel: String::new(),
        };
        let reply = Reply::Chat {
            reachable: true,
            lines: vec![
                line(1, "twitch", "GuiSai", "qq eh reemux"),
                line(2, "youtube", "@patrioticdome", "vc é dev?"),
            ],
        };
        let shown = render(&reply);
        assert_eq!(
            shown,
            format!(
                "#1 twitch  GuiSai\n  qq eh reemux\n{CHAT_RULE}\n#2 youtube  @patrioticdome\n  vc é dev?"
            )
        );
        assert_eq!(
            shown.matches(CHAT_RULE).count(),
            1,
            "between messages, not after the last"
        );
        let later = Reply::Chat {
            reachable: true,
            lines: vec![line(3, "twitch", "GuiSai", "e agora?")],
        };
        assert_eq!(
            render_more(&later, Ink::Plain),
            format!("{CHAT_RULE}\n#3 twitch  GuiSai\n  e agora?"),
            "what follows is set apart from what was already on the screen"
        );
    }

    // At a terminal the platform wears its colour and the name is bold; the
    // words are left as they are. A pipe and a test see none of it, and a
    // terminal that set NO_COLOR is a pipe.
    #[test]
    fn the_chat_is_coloured_for_a_terminal_and_plain_for_everything_else() {
        use crate::protocol::{ChatLine, Reply};
        let reply = Reply::Chat {
            reachable: true,
            lines: vec![
                ChatLine {
                    seq: 1,
                    from: "GuiSai".into(),
                    body: "qq eh reemux".into(),
                    platform: "twitch".into(),
                    id: String::new(),
                    channel: String::new(),
                },
                ChatLine {
                    seq: 2,
                    from: "\x1b[2Aadmin".into(),
                    body: "vc é dev?".into(),
                    platform: "youtube".into(),
                    id: String::new(),
                    channel: String::new(),
                },
            ],
        };
        let shown = render_with(&reply, Ink::Colour);
        assert_eq!(
            shown,
            format!(
                "#1 \x1b[1;38;5;141mtwitch\x1b[0m  \x1b[1mGuiSai\x1b[0m\n  qq eh reemux\n\
                 \x1b[2m{CHAT_RULE}\x1b[0m\n\
                 #2 \x1b[1;38;5;203myoutube\x1b[0m  \x1b[1madmin\x1b[0m\n  vc é dev?"
            ),
            "the viewer's own escape is gone before the shell's goes around the name"
        );
        assert_eq!(render_with(&reply, Ink::Plain), render(&reply));
        assert_eq!(Ink::for_terminal(true, false), Ink::Colour);
        assert_eq!(Ink::for_terminal(false, false), Ink::Plain, "a pipe");
        assert_eq!(Ink::for_terminal(true, true), Ink::Plain, "NO_COLOR");
        assert_eq!(
            render_more(&reply, Ink::Colour).lines().next(),
            Some(format!("\x1b[2m{CHAT_RULE}\x1b[0m").as_str())
        );
    }

    // A viewer's words reach the terminal as text and nothing else: an OSC 52
    // writes the operator's clipboard, a cursor move rewrites an earlier line
    // into a message nobody sent (#8). A tab is the one control that stays.
    #[test]
    fn the_chat_carries_no_control_sequence_to_the_terminal() {
        use crate::protocol::{ChatLine, Reply};
        let reply = Reply::Chat {
            reachable: true,
            lines: vec![ChatLine {
                seq: 1,
                from: "\x1b[2Aadmin".into(),
                body: "hi\x1b]52;c;aGVsbG8=\x07 there\tfriend\r\n".into(),
                platform: "twitch".into(),
                id: String::new(),
                channel: String::new(),
            }],
        };
        let shown = render(&reply);
        assert_eq!(shown, "#1 twitch  admin\n  hi there\tfriend");
    }

    #[test]
    fn chat_can_be_followed_and_the_flag_never_reaches_the_engine() {
        let w = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(follow(&w("chat -f")), (true, w("chat")));
        assert_eq!(follow(&w("chat --follow")), (true, w("chat")));
        assert_eq!(follow(&w("chat")), (false, w("chat")));
        assert_eq!(
            follow(&w("status -f")),
            (false, w("status -f")),
            "only the chat follows"
        );
        assert!(matches!(
            parse(&w("chat")),
            Ok(Command::Chat {
                since: 0,
                follow: false
            })
        ));
    }

    use super::*;

    fn said(line: &str) -> Result<Command, String> {
        let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        parse(&words)
    }

    #[test]
    fn a_status_is_one_line_a_person_can_read() {
        let status = Status {
            on_air: true,
            screen: Some("VG2791R".into()),
            mic: Some("HyperX DuoCast".into()),
            muted: true,
            flowing: crate::protocol::Flowing {
                width: 1920,
                height: 1080,
                frames: 900,
                ..Default::default()
            },
            ..Default::default()
        };
        let said = render(&Reply::Status(Box::new(status)));
        assert!(said.starts_with("on air, "), "{said}");
        assert!(said.contains("screen VG2791R"), "{said}");
        assert!(said.contains("mic HyperX DuoCast (muted)"), "{said}");
        assert!(said.contains("1920x1080 at 900 frames"), "{said}");
        assert!(!said.contains('\n'), "one line: {said}");

        // The chosen microphone is not delivering: the line says so, because
        // its name alone read as a working one the night it had left.
        let unplugged = crate::protocol::Status {
            mic: Some("Razer".into()),
            hearing: crate::protocol::Hearing {
                complaint: Some("unplugged".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let said = render(&Reply::Status(Box::new(unplugged)));
        assert!(said.contains("mic Razer (unplugged)"), "{said}");
    }

    #[test]
    fn muting_says_so_even_with_no_microphone_open() {
        let status = Status {
            muted: true,
            ..Default::default()
        };
        assert!(render(&Reply::Status(Box::new(status))).contains("muted"));
    }

    #[test]
    fn an_engine_with_nothing_plugged_in_says_that_rather_than_nothing() {
        let said = render(&Reply::Status(Box::default()));
        assert!(said.contains("off air"), "{said}");
        assert!(said.contains("no screen"), "{said}");
    }

    #[test]
    fn a_refusal_reads_as_a_refusal() {
        assert_eq!(
            render(&Reply::Error {
                message: "there is no picture to send yet".into()
            }),
            "no: there is no picture to send yet"
        );
    }

    #[test]
    fn the_device_list_puts_the_name_first_because_that_is_what_you_read() {
        let devices = Devices {
            screens: vec![Named {
                id: "3".into(),
                name: "VG2791R".into(),
            }],
            windows: vec![
                Named {
                    id: "1".into(),
                    name: "a".into()
                };
                40
            ],
            cameras: vec![],
            mics: vec![],
            genres: vec![Named {
                id: "lofi".into(),
                name: "Lofi".into(),
            }],
            apps: vec![],
        };
        let said = render(&Reply::Devices(devices));
        assert!(said.starts_with("screens:"), "{said}");
        assert!(said.contains("VG2791R"), "{said}");
        assert!(
            said.contains("40 of them"),
            "the windows are counted, not listed: {said}"
        );
        assert!(!said.contains("cameras:"), "an empty list is not a heading");
        assert!(
            said.contains("Lofi"),
            "the music a person can choose: {said}"
        );
    }

    #[test]
    fn the_gate_is_tuned_one_threshold_at_a_time() {
        let Ok(Command::Gate { patch }) = said("gate full 0.2") else {
            panic!("gate takes a name and a number")
        };
        assert_eq!(patch, serde_json::json!({ "full": 0.2 }));
    }

    #[test]
    fn a_threshold_that_is_not_one_says_which_are() {
        let complaint = said("gate loudness 3").expect_err("there is no such threshold");
        assert!(
            complaint.contains("keys_boost"),
            "it lists them: {complaint}"
        );
        let missing = said("gate").expect_err("it needs both");
        assert!(missing.contains("remux gate opens -30"), "{missing}");
    }

    // The panel's heartbeat: one `present` from a shell armed the lease and
    // the engine quit five seconds later, live included.
    #[test]
    fn present_is_the_panel_s_word_and_not_a_shell_s() {
        assert!(said("present").is_err());
    }

    #[test]
    fn a_shot_is_of_the_scene_unless_it_says_otherwise() {
        assert_eq!(said("shot"), Ok(Command::Shot { of: Framed::Scene }));
        assert_eq!(
            said("shot camera"),
            Ok(Command::Shot { of: Framed::Camera })
        );
        assert!(said("shot elbow").is_err());
    }

    #[test]
    fn the_short_ones_are_themselves() {
        assert_eq!(said("status"), Ok(Command::Status));
        assert_eq!(said("devices"), Ok(Command::Devices));
        assert_eq!(said("live"), Ok(Command::GoLive));
        assert_eq!(said("stop"), Ok(Command::Stop));
        assert_eq!(said("cut"), Ok(Command::HideEverything));
        assert_eq!(said("panic"), Ok(Command::HideEverything));
        assert_eq!(said("levels"), Ok(Command::Levels));
        assert_eq!(said("meters"), Ok(Command::Levels));
    }

    #[test]
    fn a_window_is_named_by_however_much_of_its_title_you_remember() {
        assert_eq!(
            said("window tmux a"),
            Ok(Command::Window {
                query: "tmux a".into()
            }),
            "the words after the verb are one title, not several arguments"
        );
    }

    #[test]
    fn a_screen_is_a_display_id_and_says_so_when_it_is_not() {
        assert_eq!(said("screen 3"), Ok(Command::Screen { display: 3 }));
        let complaint = said("screen VG2791R").expect_err("a name is not an id");
        assert!(
            complaint.contains("devices"),
            "it says where to look: {complaint}"
        );
    }

    #[test]
    fn a_title_and_a_description_are_words_after_a_destination_id() {
        assert_eq!(
            said("title 2 Rust at midnight"),
            Ok(Command::Retitle {
                adapter: 2,
                title: Some("Rust at midnight".into()),
                description: None,
            })
        );
        assert_eq!(
            said("describe 2 the engine, live"),
            Ok(Command::Retitle {
                adapter: 2,
                title: None,
                description: Some("the engine, live".into()),
            })
        );
        assert!(said("title 2").is_err(), "a title needs words");
        assert_eq!(said("announce 2"), Ok(Command::Announce { adapter: 2 }));
        assert_eq!(said("hide 42"), Ok(Command::Hide { seq: 42 }));
        assert_eq!(
            said("category 2 509670 Science & Technology"),
            Ok(Command::Categorize {
                adapter: 2,
                id: "509670".into(),
                name: "Science & Technology".into()
            })
        );
        assert_eq!(
            said("categories 2 sci fi"),
            Ok(Command::Categories {
                adapter: 2,
                query: "sci fi".into()
            })
        );
        assert!(said("category 2 509670").is_err(), "a category has a name");
        assert_eq!(said("delete 42"), Ok(Command::Delete { seq: 42 }));
        assert_eq!(said("disconnect 2"), Ok(Command::Disconnect { adapter: 2 }));
        assert_eq!(
            said("sandbox 2"),
            Ok(Command::Sandbox {
                adapter: 2,
                on: true
            })
        );
        assert_eq!(
            said("sandbox 2 off"),
            Ok(Command::Sandbox {
                adapter: 2,
                on: false
            })
        );
        assert!(said("sandbox 2 maybe").is_err());
        assert!(said("hide").unwrap_err().contains("number"));
        assert_eq!(said("update 2"), Ok(Command::Announce { adapter: 2 }));
        assert!(said("title two words").is_err(), "and an id first");
    }

    #[test]
    fn a_switch_said_alone_turns_on() {
        assert_eq!(said("mute"), Ok(Command::Mute { on: true }));
        assert_eq!(said("mute off"), Ok(Command::Mute { on: false }));
        assert_eq!(said("monitor no"), Ok(Command::Monitor { on: false }));
        assert_eq!(
            said("stream-music off"),
            Ok(Command::StreamMusic { on: false })
        );
        assert_eq!(said("screen-sound"), Ok(Command::ScreenSound { on: true }));
    }

    #[test]
    fn a_camera_is_closed_by_saying_off() {
        assert_eq!(
            said("camera off"),
            Ok(Command::Camera { device: None }),
            "and not by leaving the argument out and hoping"
        );
        assert_eq!(
            said("camera MacBook Pro Camera"),
            Ok(Command::Camera {
                device: Some("MacBook Pro Camera".into())
            })
        );
    }

    #[test]
    fn a_fader_is_said_as_the_panel_labels_it() {
        assert_eq!(said("vol 80"), Ok(Command::Volume { level: 0.8 }));
        assert_eq!(said("vol 80%"), Ok(Command::Volume { level: 0.8 }));
        assert_eq!(
            said("vol 150"),
            Ok(Command::Volume { level: 1.5 }),
            "the slider goes past unity and so does this"
        );
        assert_eq!(said("mvol 25"), Ok(Command::MusicVolume { level: 0.25 }));
    }

    #[test]
    fn the_duck_is_said_upward_and_meant_downward() {
        assert_eq!(said("duck 18"), Ok(Command::Duck { db: -18.0 }));
        assert_eq!(
            said("duck -18"),
            Ok(Command::Duck { db: -18.0 }),
            "however it is written, it is a step down"
        );
    }

    #[test]
    fn music_takes_a_genre_because_that_is_what_a_person_means() {
        assert_eq!(said("music"), Ok(Command::Music { on: true }));
        assert_eq!(said("music off"), Ok(Command::Music { on: false }));
        assert_eq!(
            said("music lofi"),
            Ok(Command::Genre {
                name: "lofi".into()
            })
        );
    }

    #[test]
    fn the_countdown_is_said_in_minutes_and_carried_in_seconds() {
        assert_eq!(
            said("countdown 5"),
            Ok(Command::Countdown { seconds: Some(300) })
        );
        assert_eq!(said("countdown"), Ok(Command::Countdown { seconds: None }));
    }

    #[test]
    fn the_cards_answer_to_what_people_call_them() {
        assert_eq!(
            said("card brb").expect("a card"),
            Command::Card {
                which: Card::BackInAMoment
            }
        );
        assert_eq!(
            said("card starting").expect("a card"),
            Command::Card {
                which: Card::StartingSoon
            }
        );
        assert_eq!(
            said("card").expect("a card"),
            Command::Card { which: Card::Live }
        );
        assert!(said("card purple").is_err());
    }

    #[test]
    fn arming_and_disarming_are_the_same_verb_with_the_switch_the_other_way() {
        assert_eq!(
            said("arm 2"),
            Ok(Command::Arm {
                adapter: 2,
                on: true
            })
        );
        assert_eq!(
            said("disarm 2"),
            Ok(Command::Arm {
                adapter: 2,
                on: false
            })
        );
    }

    #[test]
    fn nothing_at_all_asks_what_it_can_do() {
        let complaint = parse(&[]).expect_err("nothing is not a command");
        assert!(complaint.contains("status"), "it lists them: {complaint}");
    }

    #[test]
    fn a_word_it_does_not_know_says_so_and_then_lists_them() {
        let complaint = said("fly").expect_err("it cannot fly");
        assert!(complaint.starts_with("fly is not"), "{complaint}");
        assert!(
            complaint.contains("countdown"),
            "and then says what it can do"
        );
    }
}

/// How the answer is printed: prose for a person, JSON for a program. `--json`
/// anywhere in the words asks for the second; it never reaches the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Prose,
    Json,
}

/// What of the answer a verb wants shown. Most verbs show the reply as it is;
/// a few read one part of the status the one-line render leaves out.
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Reply,
    /// `shot --out f.jpg`: the picture written to a file.
    ShotTo(String),
    /// `gate` with nothing after it: the thresholds, in dB.
    Gate,
    /// `layout` with nothing after it: where the camera sits.
    Layout,
    /// `scenes`: the kept scenes, the current one marked.
    Scenes,
    /// `schema`: the wire's JSON Schema, answered here without an engine.
    Schema,
    /// `clips`: what `play` can play, read off the folder here.
    Clips,
    /// `destination add <platform> <name> [--url u] [--key -|--key-file f]`:
    /// written to the destinations file by the shell, never sent to the
    /// engine, so a key never crosses the socket. The key is read by the
    /// binary from stdin or a file, never taken off the command line.
    DestinationAdd {
        platform: String,
        name: String,
        url: String,
        key_from: KeyFrom,
    },
    /// `destination rm <id|name>`.
    DestinationRemove(String),
    /// `history`: every live on record, newest first, off the file here.
    History,
    /// `health`: could this engine go live now, and what stands in the way.
    /// The shell asks the grants too and joins the two.
    Health,
    /// `wait <until> [--for <seconds>]`: the shell polls until it is so.
    Wait {
        until: crate::wait::Until,
        for_secs: u64,
    },
    /// `guide`: how to drive this from a script, in prose, off the shell.
    Guide,
    /// `login [--url <web>]`: the device code sign-in to the web, the
    /// token kept in the session file by the shell.
    Login {
        base: String,
    },
    /// `logout`: the session file forgotten.
    Logout,
    /// `chat --url <ws>`: where the engine reads its chat from, kept in the
    /// config by the shell and taken up by the engine at once; `chat --url -`
    /// forgets it (the account's wire again, or none).
    ChatKeep(String),
    /// `config`: what is in effect and where each value came from.
    Config,
    /// `bug [--open]`: a report for an issue, gathered here; `--open` lands
    /// on GitHub's form with it filled in, for a person to submit.
    Bug {
        open: bool,
    },
    /// `daemon start|stop|restart|status|log|path`: the engine as a service of the session.
    Daemon(crate::daemon::Verb),
    /// `live` with nothing after it: the plan, then a person's yes.
    Confirm,
    /// `status -v`: every line of it.
    Verbose,
    /// `destinations`: the rows with their ids.
    Destinations,
    /// `log`: the engine's journal, newest last.
    Log,
    /// `categories <id> <words>`: the answer lands on the status later.
    Categories {
        adapter: i64,
        query: String,
    },
}

/// Where a stream key is read from: never the command line, which `ps` lists.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyFrom {
    Stdin,
    File(String),
    /// A destination with no key yet: connected later.
    Nowhere,
}

/// The words, read: what to send, what to show of the answer, how, and
/// whether to keep asking.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    /// Nothing to send when the shell answers by itself (`schema`).
    pub command: Option<Command>,
    pub view: View,
    pub format: Format,
    pub follow: bool,
}

pub fn read(words: &[String]) -> Result<Ask, String> {
    let json = words.iter().any(|w| matches!(w.as_str(), "--json" | "-j"));
    let words: Vec<String> = words
        .iter()
        .filter(|w| !matches!(w.as_str(), "--json" | "-j"))
        .cloned()
        .collect();
    let format = if json { Format::Json } else { Format::Prose };
    let (follow, words) = follow(&words);
    let verbose = words.len() == 2 && words[0] == "status" && words[1] == "-v";
    let (follow, words) = if words.first().map(String::as_str) == Some("log") {
        let follows = words[1..]
            .iter()
            .any(|w| matches!(w.as_str(), "-f" | "--follow"));
        (follows, vec!["log".to_string()])
    } else {
        (follow, words)
    };
    if words.first().map(String::as_str) == Some("destination") {
        let view = match words.get(1).map(String::as_str) {
            Some("add") => {
                let platform = words.get(2).cloned().ok_or(
                    "destination add takes a platform (twitch, youtube, custom) and a name",
                )?;
                let name = words
                    .get(3)
                    .cloned()
                    .ok_or("destination add takes a platform and a name")?;
                let mut url = crate::destinations::ingest_of(&platform).map(String::from);
                let mut key_from = KeyFrom::Nowhere;
                let mut at = 4;
                while at < words.len() {
                    match (words[at].as_str(), words.get(at + 1)) {
                        ("--url", Some(u)) => url = Some(u.clone()),
                        ("--key", Some(dash)) if dash == "-" => key_from = KeyFrom::Stdin,
                        ("--key", Some(_)) => {
                            return Err("a key is never typed on the command line: --key - reads it from stdin, --key-file from a file".into())
                        }
                        ("--key-file", Some(file)) => key_from = KeyFrom::File(file.clone()),
                        (other, _) => return Err(format!("{other}: destination add takes --url <rtmp url>, --key - or --key-file <file>")),
                    }
                    at += 2;
                }
                let url = url.ok_or(format!("{platform} has no default ingest; say --url"))?;
                View::DestinationAdd {
                    platform,
                    name,
                    url,
                    key_from,
                }
            }
            Some("rm") | Some("remove") => View::DestinationRemove(
                words
                    .get(2)
                    .cloned()
                    .ok_or("destination rm takes an id or a name")?,
            ),
            _ => return Err("destination takes add <platform> <name> or rm <id|name>".into()),
        };
        return Ok(Ask {
            command: None,
            view,
            format,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("login") {
        let base = match (words.get(1).map(String::as_str), words.get(2)) {
            (None, _) => crate::session::default_base(),
            (Some("--url"), Some(base)) => base.trim_end_matches('/').to_string(),
            _ => return Err("login takes nothing or --url <web>".into()),
        };
        return Ok(Ask {
            command: None,
            view: View::Login { base },
            format,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("chat")
        && words.get(1).map(String::as_str) == Some("--url")
    {
        let url = words
            .get(2)
            .cloned()
            .ok_or("chat --url takes a ws:// or wss:// address, or - to forget it")?;
        return Ok(Ask {
            command: None,
            view: View::ChatKeep(url),
            format,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("logout") {
        return Ok(Ask {
            command: None,
            view: View::Logout,
            format,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("guide") {
        return Ok(Ask {
            command: None,
            view: View::Guide,
            format,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("daemon") {
        return Ok(Ask {
            command: None,
            view: View::Daemon(crate::daemon::Verb::parse(&words[1..])?),
            format,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("bug") {
        return Ok(Ask {
            command: None,
            view: View::Bug {
                open: words.iter().any(|w| w == "--open"),
            },
            format,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("config") {
        return Ok(Ask {
            command: None,
            view: View::Config,
            format,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("wait") {
        let until = crate::wait::Until::parse(
            words.get(1).map_or("", String::as_str),
            words.get(2).map(String::as_str),
        )?;
        let mut for_secs = 30;
        if let Some(at) = words.iter().position(|w| w == "--for") {
            for_secs = words
                .get(at + 1)
                .and_then(|s| s.parse().ok())
                .ok_or("--for takes seconds")?;
        }
        return Ok(Ask {
            command: Some(Command::Status),
            view: View::Wait { until, for_secs },
            format,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("history") {
        return Ok(Ask {
            command: None,
            view: View::History,
            format,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("clips") {
        return Ok(Ask {
            command: None,
            view: View::Clips,
            format,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("schema") {
        return Ok(Ask {
            command: None,
            view: View::Schema,
            format: Format::Json,
            follow: false,
        });
    }
    if words.first().map(String::as_str) == Some("shot") {
        if let Some(at) = words.iter().position(|w| w == "--out") {
            let file = words
                .get(at + 1)
                .cloned()
                .ok_or("--out takes a file name")?;
            let mut rest = words.clone();
            rest.drain(at..at + 2);
            return Ok(Ask {
                command: Some(parse(&rest)?),
                view: View::ShotTo(file),
                format,
                follow,
            });
        }
    }
    let (command, view) = match words.first().map(String::as_str) {
        _ if verbose => (Command::Status, View::Verbose),
        Some("live") | Some("go-live") if words.len() == 1 => (Command::Plan, View::Confirm),
        Some("gate") if words.len() == 1 => (Command::Status, View::Gate),
        Some("layout") if words.len() == 1 => (Command::Status, View::Layout),
        Some("layout") => (parse(&words)?, View::Layout),
        Some("scenes") => (Command::Status, View::Scenes),
        Some("health") => (Command::Status, View::Health),
        Some("scene") => (parse(&words)?, View::Scenes),
        Some("destinations") | Some("dests") => (Command::Status, View::Destinations),
        Some("log") => (Command::Status, View::Log),
        Some("categories") => match parse(&words)? {
            Command::Categories { adapter, query } => (
                Command::Categories {
                    adapter,
                    query: query.clone(),
                },
                View::Categories { adapter, query },
            ),
            other => (other, View::Reply),
        },
        _ => (parse(&words)?, View::Reply),
    };
    Ok(Ask {
        command: Some(command),
        view,
        format,
        follow,
    })
}

/// The camera's layout out of words: a corner, a width in percent, a shape;
/// any of them, in any order.
pub fn layout_patch(words: &[String]) -> Result<crate::scene::LayoutPatch, String> {
    use crate::scene::{Corner, Filter, LayoutPatch, Mode, Shape};
    let mut patch = LayoutPatch::default();
    for word in words {
        match word.as_str() {
            "tl" | "top-left" => patch.corner = Some(Corner::TopLeft),
            "tr" | "top-right" => patch.corner = Some(Corner::TopRight),
            "bl" | "bottom-left" => patch.corner = Some(Corner::BottomLeft),
            "br" | "bottom-right" => patch.corner = Some(Corner::BottomRight),
            "rect" | "rectangle" => patch.shape = Some(Shape::Rectangle),
            "square" => patch.shape = Some(Shape::Square),
            "circle" | "round" => patch.shape = Some(Shape::Circle),
            "plain" => patch.filter = Some(Filter::Plain),
            "sepia" => patch.filter = Some(Filter::Sepia),
            "mono" | "bw" => patch.filter = Some(Filter::Mono),
            "noir" => patch.filter = Some(Filter::Noir),
            "overlay" | "corner" => patch.mode = Some(Mode::Overlay),
            "columns" | "column" | "side" => patch.mode = Some(Mode::Columns),
            "bounce" | "dvd" => patch.mode = Some(Mode::Bounce),
            other => {
                let percent = other
                    .strip_suffix('%')
                    .and_then(|n| n.parse::<f64>().ok())
                    .ok_or_else(|| {
                        format!(
                            "{other} is not a corner (tl, tr, bl, br), a width (25%), a shape (rect, square, circle), a look (plain, sepia, mono, noir) or a mode (overlay, columns, bounce)"
                        )
                    })?;
                patch.share = Some(percent / 100.0);
            }
        }
    }
    if patch == LayoutPatch::default() {
        return Err("layout takes a corner (tl, tr, bl, br), a width (25%), a shape (rect, square, circle) or a look (plain, sepia, mono, noir)".into());
    }
    Ok(patch)
}

/// A patch, said back: what the journal writes.
pub fn layout_words(patch: &crate::scene::LayoutPatch) -> String {
    let mut said = Vec::new();
    if let Some(mode) = patch.mode {
        said.push(format!("{mode:?}").to_lowercase());
    }
    if let Some(corner) = patch.corner {
        said.push(format!("{corner:?}").to_lowercase());
    }
    if let Some(share) = patch.share {
        said.push(format!("{:.0}%", share * 100.0));
    }
    if let Some(shape) = patch.shape {
        said.push(format!("{shape:?}").to_lowercase());
    }
    if let Some(filter) = patch.filter {
        said.push(format!("{filter:?}").to_lowercase());
    }
    said.join(" ")
}

/// What the shell answers by itself, when there is nothing to ask.
pub fn local(view: &View, format: Format) -> String {
    match view {
        View::Clips => {
            let root = crate::clips::root();
            let names = crate::clips::list(&root);
            if names.is_empty() {
                format!("no clips in {} (remux config)", root.display())
            } else {
                names.join("\n")
            }
        }
        View::Guide => GUIDE.to_string(),
        View::Config => crate::config::describe(),
        View::History => {
            let all = crate::history::read(&crate::history::path());
            match format {
                Format::Json => json(&all),
                Format::Prose if all.is_empty() => "no lives on record yet".into(),
                Format::Prose => all
                    .iter()
                    .map(|b| {
                        format!(
                            "{}  {:>8}  {:<24} {}{}",
                            crate::journal::clock_of(b.started),
                            elapsed(Some(b.started), b.ended),
                            b.destinations.join(", "),
                            if b.samples == 0 {
                                String::new()
                            } else {
                                format!(
                                    "{} {} fps {} kbps  ",
                                    b.resolution, b.fps_avg, b.video_kbps_avg
                                )
                            },
                            b.peak_viewers
                                .map(|n| format!("{n} watching at most"))
                                .unwrap_or_default()
                        )
                        .trim_end()
                        .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            }
        }
        View::Schema => json(&serde_json::json!({
            "command": schemars::schema_for!(Command),
            "reply": schemars::schema_for!(Reply),
            "wire_up": schemars::schema_for!(crate::wire::Up),
            "wire_line": schemars::schema_for!(crate::wire::Line),
        })),
        _ => String::new(),
    }
}

/// The picture out of a shot, as the bytes of a JPEG.
pub fn jpeg_bytes(reply: &Reply) -> Option<Vec<u8>> {
    use base64::Engine;
    match reply {
        Reply::Shot { jpeg, .. } => base64::engine::general_purpose::STANDARD.decode(jpeg).ok(),
        _ => None,
    }
}

/// The answer, shown the way the words asked. `now` is seconds past the
/// epoch, for the clocks.
pub fn show(reply: &Reply, view: &View, format: Format, ink: Ink, now: i64) -> String {
    match (format, view, reply) {
        (Format::Json, View::Destinations, Reply::Status(status)) => json(&status.destinations),
        (Format::Json, View::Log, Reply::Status(status)) => json(&status.log),
        (Format::Json, View::Categories { .. }, Reply::Status(status)) => json(&status.categories),
        (Format::Json, View::Gate, Reply::Status(status)) => json(&status.gate),
        (Format::Json, View::Layout, Reply::Status(status)) => json(&status.layout),
        (Format::Json, View::Scenes, Reply::Status(status)) => {
            json(&serde_json::json!({ "scenes": status.scenes, "scene": status.scene }))
        }
        (Format::Prose, View::Scenes, Reply::Status(status)) => render_scenes(status),
        (Format::Prose, View::Layout, Reply::Status(status)) => render_layout(&status.layout),
        (Format::Prose, View::Gate, Reply::Status(status)) => render_gate(&status.gate),
        (Format::Json, _, reply) => json(reply),
        (Format::Prose, View::Destinations, Reply::Status(status)) => {
            render_destinations(&status.destinations)
        }
        (Format::Prose, View::Log, Reply::Status(status)) => status.log.join("\n"),
        (Format::Prose, View::Verbose, Reply::Status(status)) => render_verbose(status, now),
        (Format::Prose, View::Categories { .. }, Reply::Status(status)) => match &status.categories
        {
            Some(found) if !found.items.is_empty() => found
                .items
                .iter()
                .map(|c| format!("{:<12} {}", c.id, c.name))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => "nothing found".into(),
        },
        (Format::Prose, _, reply) => render_with(reply, ink),
    }
}

/// One line of text as a JSON string, for a stream of lines.
pub fn json_line(line: &str) -> String {
    json(&line)
}

fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

/// Every destination on one row with its id first, because the id is what
/// `arm`, `title`, `sandbox` and `category` take and no other read gave it.
fn render_destinations(rows: &[crate::protocol::Destination]) -> String {
    if rows.is_empty() {
        return "no destinations".into();
    }
    let mut lines = vec![format!(
        "{:<4} {:<14} {:<8} {:<8} {:<6} {:<8} {:<16} {}",
        "id", "name", "platform", "status", "armed", "sandbox", "account", "title"
    )];
    for row in rows {
        lines.push(format!(
            "{:<4} {:<14} {:<8} {:<8} {:<6} {:<8} {:<16} {}",
            row.id,
            row.name,
            row.platform,
            row.status,
            if row.armed { "yes" } else { "no" },
            if row.sandbox { "yes" } else { "no" },
            row.account
                .as_deref()
                .unwrap_or(if row.connected { "" } else { "not connected" }),
            row.title.as_deref().unwrap_or(""),
        ));
        let mut more = Vec::new();
        if let Some(category) = &row.category {
            more.push(format!("category {category}"));
        }
        if let Some(viewers) = row.viewers {
            more.push(format!("{viewers} watching"));
        }
        if let Some(trouble) = &row.trouble {
            more.push(format!("! {trouble}"));
        }
        if !more.is_empty() {
            lines.push(format!("     {}", more.join(", ")));
        }
    }
    lines.join("\n")
}

fn render_layout(l: &crate::scene::Layout) -> String {
    format!(
        "camera {}, {:.0}% wide, {}{}{}",
        format!("{:?}", l.corner).to_lowercase(),
        l.share * 100.0,
        format!("{:?}", l.shape).to_lowercase(),
        match l.filter {
            crate::scene::Filter::Plain => String::new(),
            look => format!(", {}", format!("{look:?}").to_lowercase()),
        },
        match l.mode {
            crate::scene::Mode::Overlay => String::new(),
            mode => format!(", {}", format!("{mode:?}").to_lowercase()),
        }
    )
}

fn render_scenes(status: &Status) -> String {
    if status.scenes.is_empty() {
        return "no scenes kept; remux scene save <name> keeps the setup of now".into();
    }
    status
        .scenes
        .iter()
        .map(|name| {
            format!(
                "{} {name}",
                if status.scene.as_deref() == Some(name) {
                    "*"
                } else {
                    " "
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_gate(g: &crate::gate::GateParams) -> String {
    use crate::levels::decibels;
    format!(
        "opens at {:.0} dB, highs at {:.0} dB, closed {:.0} dB, hold {:.0} ms, attack {:.0} ms, keys boost {:+.0} dB",
        decibels(g.full),
        decibels(g.hf),
        decibels(g.floor),
        g.hold_ms,
        g.attack_ms,
        decibels(g.keys_boost)
    )
}

/// `h:mm:ss` since a moment, or nothing when there is none.
pub fn elapsed(since: Option<i64>, now: i64) -> String {
    match since {
        Some(since) => {
            let s = (now - since).max(0);
            format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
        }
        None => String::new(),
    }
}

/// The whole status, one fact a line, in the units the sliders are marked in.
fn render_verbose(status: &Status, now: i64) -> String {
    use crate::levels::decibels;
    let mut lines = Vec::new();
    lines.push(match status.on_air_since {
        Some(since) => format!("on air {}", elapsed(Some(since), now)),
        None if status.on_air => "on air".into(),
        None => "off air".into(),
    });
    if status.recording {
        lines.push(format!(
            "recording {}",
            elapsed(status.recording_since, now)
        ));
    }
    if let Some(card) = &status.card {
        lines.push(format!("card {card:?}"));
    }
    lines.push(match &status.screen {
        Some(screen) => format!("screen {screen}"),
        None => "no screen".into(),
    });
    if status.screen_sound {
        lines.push(match status.hearing_apps.as_slice() {
            [] => "screen sound out".into(),
            apps => format!("screen sound out: {}", apps.join(", ")),
        });
    }
    if let Some(camera) = &status.camera {
        lines.push(format!(
            "camera {camera}{}, {}",
            if status.mirrored { ", mirrored" } else { "" },
            render_layout(&status.layout)
        ));
    }
    match &status.mic {
        Some(mic) => lines.push(format!(
            "mic {mic}{}{}{}",
            if status.muted { ", muted" } else { "" },
            if status.denoise { ", denoised" } else { "" },
            match &status.hearing.complaint {
                Some(why) => format!(" ({why})"),
                None => String::new(),
            }
        )),
        None => lines.push("no mic".into()),
    }
    let h = &status.hearing;
    lines.push(format!(
        "  level {:.1} dB, peak {:.1} dB, gate {} at {:+.1} dB, voice {:.1} dB, highs {:.1} dB",
        h.level_db,
        h.peak_db,
        if h.gate_open { "open" } else { "closed" },
        h.gain_db,
        decibels(h.gate_levels.full),
        decibels(h.gate_levels.hf)
    ));
    lines.push(format!("  {}", render_gate(&status.gate)));
    if h.starved > 0 || h.dropped > 0 {
        lines.push(format!(
            "  ring {} frames, starved {} blocks, dropped {} samples",
            h.buffered, h.starved, h.dropped
        ));
    }
    lines.push(format!(
        "volume {:.0}%, music {:.0} dB, duck {:.0} dB{}",
        status.faders.mic * 100.0,
        crate::music::music_fader_db(status.faders.music),
        status.faders.duck_db,
        if status.mixing.ducked_db < -0.5 {
            format!(" (ducking {:.0} dB)", status.mixing.ducked_db)
        } else {
            String::new()
        }
    ));
    lines.push(format!(
        "music {}{}{}",
        status.music.as_deref().unwrap_or("off"),
        if status.music_to_stream {
            ""
        } else {
            ", not sent"
        },
        match (status.monitoring, &status.speakers) {
            (true, Some(speakers)) => format!(", on {speakers}"),
            (true, None) => ", on the speakers".into(),
            (false, _) => String::new(),
        }
    ));
    if let Some(viewers) = status.viewers {
        lines.push(format!(
            "{viewers} watching{}",
            match status.viewers_peak {
                Some(peak) => format!(", peak {peak}"),
                None => String::new(),
            }
        ));
    }
    let o = &status.outgoing;
    lines.push(format!(
        "out {}x{} at {} fps, {} kbps video, {} kbps audio; {}x{} at {} frames in",
        o.width,
        o.height,
        o.fps,
        o.video_kbps,
        o.audio_kbps,
        status.flowing.width,
        status.flowing.height,
        status.flowing.frames
    ));
    lines.push(format!(
        "app {}{}",
        if status.app {
            "reachable"
        } else {
            "not signed in"
        },
        match &status.server {
            Some(server) => format!(" at {server}"),
            None => String::new(),
        }
    ));
    if let Some(dir) = &status.record_dir {
        lines.push(format!("recordings in {dir}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod reading {
    use super::*;
    use crate::protocol::Destination;

    fn w(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn json_is_the_shell_s_flag_and_never_reaches_the_engine() {
        let ask = read(&w("status --json")).unwrap();
        assert_eq!(ask.command, Some(Command::Status));
        assert_eq!(ask.format, Format::Json);
        assert_eq!(
            read(&w("-j mute")).unwrap().command,
            Some(Command::Mute { on: true })
        );
    }

    #[test]
    fn a_destination_is_added_with_its_key_off_the_command_line() {
        let ask = read(&w("destination add twitch main --key -")).unwrap();
        assert_eq!(
            ask.view,
            View::DestinationAdd {
                platform: "twitch".into(),
                name: "main".into(),
                url: "rtmp://live.twitch.tv/app".into(),
                key_from: KeyFrom::Stdin,
            }
        );
        assert!(
            read(&w("destination add twitch main --key live_abc")).is_err(),
            "never on argv"
        );
        assert!(
            read(&w("destination add custom vps")).is_err(),
            "custom needs --url"
        );
        let vps = read(&w(
            "destination add custom vps --url rtmp://host/live --key-file k.txt",
        ))
        .unwrap();
        assert!(matches!(
            vps.view,
            View::DestinationAdd {
                key_from: KeyFrom::File(_),
                ..
            }
        ));
        assert_eq!(
            read(&w("destination rm 2")).unwrap().view,
            View::DestinationRemove("2".into())
        );
        assert_eq!(
            read(&w("login")).unwrap().view,
            View::Login {
                base: crate::session::default_base()
            }
        );
        assert_eq!(
            read(&w("login --url http://localhost:4700/")).unwrap().view,
            View::Login {
                base: "http://localhost:4700".into()
            }
        );
        assert_eq!(read(&w("logout")).unwrap().view, View::Logout);
        assert_eq!(
            read(&w("chat --url ws://localhost:9999")).unwrap().view,
            View::ChatKeep("ws://localhost:9999".into())
        );
        assert!(read(&w("chat --url")).is_err());
        assert_eq!(read(&w("config")).unwrap().view, View::Config);
        assert_eq!(
            read(&w("bug --open")).unwrap().view,
            View::Bug { open: true }
        );
        assert_eq!(
            read(&w("daemon stop --force")).unwrap().view,
            View::Daemon(crate::daemon::Verb::Stop { force: true })
        );
        assert!(read(&w("daemon")).is_err());
    }

    #[test]
    fn the_shell_answers_schema_and_writes_a_shot_by_itself() {
        assert!(
            read(&w("preview")).is_err(),
            "no preview verb: remux shot is the picture"
        );
        let ask = read(&w("schema")).unwrap();
        assert_eq!((ask.command, &ask.view), (None, &View::Schema));
        assert!(local(&ask.view, Format::Prose).contains("\"command\""));
        let shot = read(&w("shot camera --out /tmp/x.jpg")).unwrap();
        assert_eq!(shot.view, View::ShotTo("/tmp/x.jpg".into()));
        assert_eq!(shot.command, Some(Command::Shot { of: Framed::Camera }));
        assert_eq!(
            jpeg_bytes(&Reply::Shot {
                jpeg: "AAEC".into(),
                width: 1,
                height: 1
            }),
            Some(vec![0, 1, 2])
        );
    }

    #[test]
    fn live_goes_on_a_confirmed_plan_or_asks() {
        assert_eq!(parse(&w("plan")), Ok(Command::Plan));
        assert_eq!(
            parse(&w("live --confirm 42")),
            Ok(Command::Live { plan: 42 })
        );
        assert_eq!(parse(&w("live --yes")), Ok(Command::GoLive));
        assert_eq!(parse(&w("denoise")), Ok(Command::Denoise { on: true }));
        assert_eq!(
            parse(&w("play clap")),
            Ok(Command::Clip {
                name: "clap".into()
            })
        );
        assert_eq!(
            parse(&w("hear Spotify, Brave")),
            Ok(Command::Hear {
                apps: vec!["Spotify".into(), "Brave".into()]
            })
        );
        assert_eq!(parse(&w("hear off")), Ok(Command::Hear { apps: vec![] }));
        assert!(parse(&w("hear")).is_err());
        assert!(parse(&w("play")).is_err());
        assert_eq!(read(&w("clips")).unwrap().view, View::Clips);
        assert!(parse(&w("live --confirm")).is_err());
        let asks = read(&w("live")).unwrap();
        assert_eq!(
            (asks.command, asks.view),
            (Some(Command::Plan), View::Confirm)
        );
        let plan = crate::plan::Plan::of(&Status::default());
        let shown = render(&Reply::Plan(plan.clone()));
        assert!(shown.contains("! no destination is armed"), "{shown}");
        assert!(
            shown.ends_with(&format!("plan {}", plan.fingerprint)),
            "{shown}"
        );
    }

    #[test]
    fn the_layout_is_said_in_any_order_and_read_back() {
        use crate::scene::{Corner, Shape};
        let Ok(Command::Layout { patch }) = parse(&w("layout circle 50% tl")) else {
            panic!()
        };
        assert_eq!(
            (patch.corner, patch.share, patch.shape),
            (Some(Corner::TopLeft), Some(0.5), Some(Shape::Circle))
        );
        assert!(parse(&w("layout sideways")).is_err());
        assert!(parse(&w("layout")).is_err(), "nothing to change");
        assert_eq!(read(&w("layout")).unwrap().view, View::Layout);
        assert_eq!(
            show(
                &Reply::Status(Box::default()),
                &View::Layout,
                Format::Prose,
                Ink::Plain,
                0
            ),
            "camera bottomright, 25% wide, rectangle"
        );
        assert_eq!(layout_words(&patch), "topleft 50% circle");
        let Ok(Command::Layout { patch }) = parse(&w("layout sepia")) else {
            panic!()
        };
        assert_eq!(patch.filter, Some(crate::scene::Filter::Sepia));
    }

    #[test]
    fn scenes_are_kept_switched_and_forgotten_by_name() {
        assert_eq!(
            parse(&w("scene save code")),
            Ok(Command::SceneSave {
                name: "code".into()
            })
        );
        assert_eq!(
            parse(&w("scene code")),
            Ok(Command::SceneSwitch {
                name: "code".into()
            })
        );
        assert_eq!(
            parse(&w("scene rm code")),
            Ok(Command::SceneForget {
                name: "code".into()
            })
        );
        assert!(parse(&w("scene")).is_err());
        assert_eq!(read(&w("scenes")).unwrap().view, View::Scenes);
        let status = Status {
            scenes: vec!["code".into(), "talk".into()],
            scene: Some("talk".into()),
            ..Status::default()
        };
        assert_eq!(
            show(
                &Reply::Status(Box::new(status)),
                &View::Scenes,
                Format::Prose,
                Ink::Plain,
                0
            ),
            "  code\n* talk"
        );
    }

    #[test]
    fn the_gate_speaks_the_panel_s_words_in_db_and_resets_whole() {
        assert_eq!(read(&w("gate")).unwrap().view, View::Gate);
        let Ok(Command::Gate { patch }) = parse(&w("gate opens -20")) else {
            panic!()
        };
        assert!(
            (patch["full"].as_f64().unwrap() - 0.1).abs() < 1e-9,
            "{patch}"
        );
        let Ok(Command::Gate { patch }) = parse(&w("gate reset")) else {
            panic!()
        };
        assert_eq!(patch["hold_ms"], 450.0);
        assert_eq!(
            parse(&w("words brb Back in five")),
            Ok(Command::CardText {
                which: Card::BackInAMoment,
                text: "Back in five".into()
            })
        );
        let shown = show(
            &Reply::Status(Box::default()),
            &View::Gate,
            Format::Prose,
            Ink::Plain,
            0,
        );
        assert!(shown.starts_with("opens at -20 dB"), "{shown}");
    }

    #[test]
    fn destinations_log_and_verbose_read_the_status() {
        assert_eq!(read(&w("destinations")).unwrap().view, View::Destinations);
        assert_eq!(read(&w("status -v")).unwrap().view, View::Verbose);
        let log = read(&w("log -f")).unwrap();
        assert_eq!(
            (log.command, log.view, log.follow),
            (Some(Command::Status), View::Log, true)
        );
    }

    fn twitch() -> Destination {
        Destination {
            id: 2,
            name: "twitch".into(),
            platform: "twitch".into(),
            status: "off".into(),
            armed: true,
            sandbox: false,
            connected: true,
            account: Some("someone".into()),
            category: Some("Science & Technology".into()),
            category_id: Some("509670".into()),
            viewers: Some(3),
            viewers_peak: None,
            trouble: Some("twitch said 429".into()),
            title: Some("remux live".into()),
            description: None,
            channel: None,
        }
    }

    #[test]
    fn destinations_show_the_id_first_and_the_trouble_under_the_row() {
        let status = Status {
            destinations: vec![twitch()],
            ..Status::default()
        };
        let shown = show(
            &Reply::Status(Box::new(status)),
            &View::Destinations,
            Format::Prose,
            Ink::Plain,
            0,
        );
        assert!(
            shown.lines().nth(1).unwrap().starts_with("2    twitch"),
            "{shown}"
        );
        assert!(
            shown.contains("remux live") && shown.contains("! twitch said 429"),
            "{shown}"
        );
        assert_eq!(
            show(
                &Reply::Status(Box::default()),
                &View::Destinations,
                Format::Prose,
                Ink::Plain,
                0
            ),
            "no destinations"
        );
    }

    #[test]
    fn json_of_a_view_is_that_part_of_the_status() {
        let status = Status {
            destinations: vec![twitch()],
            log: vec!["one".into()],
            ..Status::default()
        };
        let reply = Reply::Status(Box::new(status));
        assert!(
            show(&reply, &View::Destinations, Format::Json, Ink::Plain, 0)
                .starts_with("[{\"id\":2")
        );
        assert_eq!(
            show(&reply, &View::Log, Format::Json, Ink::Plain, 0),
            "[\"one\"]"
        );
        assert!(show(&Reply::Ok, &View::Reply, Format::Json, Ink::Plain, 0)
            .contains("\"reply\":\"ok\""));
    }

    #[test]
    fn the_verbose_status_has_the_clocks_and_the_gate_in_db() {
        let status = Status {
            on_air: true,
            on_air_since: Some(1_000),
            ..Status::default()
        };
        let shown = show(
            &Reply::Status(Box::new(status)),
            &View::Verbose,
            Format::Prose,
            Ink::Plain,
            4_661,
        );
        assert!(shown.starts_with("on air 1:01:01\n"), "{shown}");
        assert!(
            shown.contains("opens at -20 dB, highs at -55 dB, closed -40 dB, hold 450 ms"),
            "{shown}"
        );
        assert_eq!(elapsed(None, 5), "");
    }
}
