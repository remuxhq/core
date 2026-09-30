//! What somebody typed, as a command the engine answers.
//!
//! A decision and not a transport, so it is here and it is tested by
//! `cargo test`: the binary that uses it opens a socket, writes a line and
//! prints what comes back, and that is all it does.
//!
//! The words are the ones the engine this replaces already answered to, because
//! they are in people's fingers and in their shell history.

mod group;
mod help;
pub use group::normalize;
pub use help::{guide, help, usage};

/// A global output option, removed before interpreting positional arguments.
/// It can precede the command or follow any of its arguments.
pub fn output_mode(words: &[String]) -> (bool, Vec<String>) {
    let asked = |word: &String| matches!(word.as_str(), "--json" | "-j");
    (
        words.iter().any(asked),
        words.iter().filter(|word| !asked(word)).cloned().collect(),
    )
}

/// The same reply shape the socket uses, one complete JSON object per line.
pub fn render_json(reply: &Reply) -> String {
    remuxd_domain::protocol::encode(reply)
        .trim_end_matches('\n')
        .to_string()
}

use remuxd_domain::protocol::{Command, Devices, Framed, Grant, Named, Reply, Status};

/// Read a command out of the words after the program's own name.
///
/// The error is what a person reads when they get it wrong, so it says what
/// was expected rather than that something was invalid.
/// Whether the words ask to keep reading the chat (`chat read -f`,
/// `chat read --follow`, `chat read follow`), and the words with that taken out. Following is the
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

/// What `chat read -f` prints for messages that arrived after some were already
/// on the screen: the rule first. The rule goes between messages, and the
/// message before these is the last one printed, so without it every batch
/// that landed a second apart ran into the one before.
pub fn render_more(reply: &Reply, ink: Ink) -> String {
    format!("{}\n{}", ink.rule(), render_with(reply, ink))
}

pub fn parse(words: &[String]) -> Result<Command, String> {
    // An explicit genre is never a switch: even a genre named "off" is a
    // genre. Keep this distinction before expanding grouped shell words.
    if matches!(words.first().map(String::as_str), Some("music"))
        && matches!(words.get(1).map(String::as_str), Some("genre"))
    {
        let _ = normalize(words)?;
        return Ok(Command::Genre {
            name: words[2..].join(" "),
        });
    }
    let words = normalize(words)?;
    parse_wire_words(&words)
}

// The wire vocabulary stays stable; only the shell grammar above changes.
fn parse_wire_words(words: &[String]) -> Result<Command, String> {
    let (verb, rest) = words.split_first().ok_or_else(usage)?;
    let joined = rest.join(" ");
    match verb.as_str() {
        "status" => {
            if rest.is_empty() {
                Ok(Command::Status)
            } else {
                Err("status takes no arguments".into())
            }
        }
        "scene-create" | "scene-duplicate" | "scene-switch" | "scene-delete" => match rest {
            [name] if !name.is_empty() => Ok(match verb.as_str() {
                "scene-create" => Command::SceneCreate { name: name.clone() },
                "scene-duplicate" => Command::SceneDuplicate { name: name.clone() },
                "scene-switch" => Command::SceneSwitch { name: name.clone() },
                _ => Command::SceneDelete { name: name.clone() },
            }),
            _ => Err("scene command needs exactly one name (quote names with spaces)".into()),
        },
        "audio-layer" => parse_audio_layer(rest),
        "levels" => Ok(Command::Levels),
        // The composed scene preview.
        "shot" if rest.is_empty() => Ok(Command::Shot { of: Framed::Scene }),
        "shot" => Err("scene shot takes no arguments; use scene layer shot <id>".into()),
        "grants" => Ok(Command::Grants),
        "chat" => Ok(Command::Chat {
            since: 0,
            follow: false,
        }),
        // `remux chat read -f`: the same, then again every second for what is new,
        // the way `tail -f` reads a file. The flag is the shell's (`follow`),
        // the command on the wire is the same one.
        // `remux chat hide 42`: that line of chat, off every face.
        "hide" => {
            let seq = rest
                .first()
                .ok_or("hide needs the line's number")?
                .parse()
                .map_err(|_| "a line's number is a number".to_string())?;
            Ok(Command::Hide { seq })
        }
        "sources" => Ok(Command::Devices),
        // `remux plan` says what live would do; `remux live --confirm <plan>`
        // does it only if that is still true. `remux live` alone asks a
        // person at a terminal, or is refused where there is nobody to ask.
        "plan" => Ok(Command::Plan),
        // What the shell draws out of the status by itself.
        "scenes" | "destinations" | "log" | "health" => Ok(Command::Status),
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
                .ok_or("screen needs a display id, which `remux sources` lists")?;
            display
                .parse()
                .map(|display| Command::Screen { display })
                .map_err(|_| format!("{display} is not a display id; `remux sources` lists them"))
        }
        "window" => {
            if joined.is_empty() {
                return Err(
                    "window needs part of a title, as in `remux scene layer add window editor ghostty`".into(),
                );
            }
            Ok(Command::Window { query: joined })
        }
        "camera" => Ok(Command::Camera {
            device: off_or(&joined),
        }),
        "layer" => parse_layer(rest),
        "shader" => match rest {
            [path] if path == "off" => Ok(Command::Shader { path: None }),
            [path] if !path.is_empty() => Ok(Command::Shader {
                path: Some(path.clone()),
            }),
            _ => Err("filter takes one .wgsl file or `off`".into()),
        },
        "camera-shape" => match rest {
            [shape] if shape == "circle" => Ok(Command::CameraShape {
                shape: remuxd_domain::picture::scene::CameraShape::Circle,
            }),
            [shape] if shape == "rectangle" => Ok(Command::CameraShape {
                shape: remuxd_domain::picture::scene::CameraShape::Rectangle,
            }),
            _ => Err("camera-shape takes exactly `circle` or `rectangle`".into()),
        },
        "camera-position" => {
            if rest.len() == 1 && rest[0] == "default" {
                return Ok(Command::CameraPosition { at: None });
            }
            if rest.len() != 2 {
                return Err(
                    "camera-position takes x and y in 1920x1080 pixels, or `default`".into(),
                );
            }
            let (width, height) = remuxd_domain::picture::scene::CAMERA_OUTPUT;
            let point = |word: &str, limit: u32| -> Result<u32, String> {
                let value = word
                    .parse::<u32>()
                    .map_err(|_| format!("{word} is not a non-negative pixel coordinate"))?;
                (value < limit)
                    .then_some(value)
                    .ok_or_else(|| format!("{word} must be less than {limit}"))
            };
            Ok(Command::CameraPosition {
                at: Some(remuxd_domain::picture::scene::CameraPosition {
                    x: point(&rest[0], width)?,
                    y: point(&rest[1], height)?,
                }),
            })
        }
        "mic" => Ok(Command::Mic {
            device: off_or(&joined),
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
        "app-audio" => Ok(Command::AppAudio {
            app: off_or(&joined),
        }),
        "app-audio-volume" => Ok(Command::AppAudioVolume {
            level: percentage(&joined, "audio app-volume")?,
        }),
        "music" => match joined.as_str() {
            "" | "on" | "off" => Ok(Command::Music {
                on: on_or(&joined)?,
            }),
            // `remux music genre jazz` is what a person means, and it is a genre.
            name => Ok(Command::Genre { name: name.into() }),
        },
        "next" | "skip" => Ok(Command::NextTrack),
        // `remux play clap`: once, over everything. `remux clips` lists them.
        "play" if joined.is_empty() => Err("play takes a clip's name or a file".into()),
        "play" => Ok(Command::Clip { name: joined }),

        "vol" => Ok(Command::Volume {
            level: percentage(&joined, "audio vol")?,
        }),
        "mvol" => Ok(Command::MusicVolume {
            level: percentage(&joined, "music vol")?,
        }),
        "duck" => {
            let db: f64 = joined
                .parse()
                .map_err(|_| "duck takes decibels, as in `remux audio duck 18`".to_string())?;
            // Said as a positive number and meant as a step downward, which is
            // how the panel labels it and how anybody says it out loud.
            Ok(Command::Duck { db: -db.abs() })
        }

        "scene-element" => parse_scene_element(rest),
        "scene-timer" => match rest {
            [action, id] if !id.is_empty() => match action.as_str() {
                "start" => Ok(Command::SceneTimerStart { id: id.clone() }),
                "stop" => Ok(Command::SceneTimerStop { id: id.clone() }),
                _ => Err("scene timer takes start|stop <id>".into()),
            },
            _ => Err("scene timer takes start|stop <id>".into()),
        },
        "cut" => Ok(Command::HideEverything),

        "record" => match joined.as_str() {
            "" | "start" => Ok(Command::RecordStart),
            "stop" => Ok(Command::RecordStop),
            other => Err(format!("record takes start or stop, not {other}")),
        },

        // One threshold at a time, because that is how a person tunes a gate:
        // `remux audio gate full 0.2`. The names are the ones the panel's sliders
        // carry and the ones in `gate::GateParams`.
        // `remux audio gate reset`: the seven defaults. `remux audio gate opens -30`: the
        // panel's words, in dB; `remux gate full 0.03`: the wire's, as they are.
        "gate" if joined == "reset" => Ok(Command::Gate {
            patch: serde_json::to_value(remuxd_domain::sound::mixer::gate::GateParams::default())
                .map_err(|e| e.to_string())?,
        }),
        "gate" => {
            let (name, value) = (rest.first(), rest.get(1));
            let (Some(name), Some(value)) = (name, value) else {
                return Err(
                    "gate takes a threshold and a number, as in `remux audio gate opens -30`.\n\
                     in dB: opens, highs, closed, keys; in ms: hold_ms, attack_ms, hf_attack_ms;\n\
                     as the wire has them: hf, full, floor, keys_boost; or `gate reset`"
                        .into(),
                );
            };
            let number: f64 = value
                .parse()
                .map_err(|_| format!("{value} is not a number"))?;
            let (name, number) = match name.as_str() {
                "opens" => (
                    "full",
                    remuxd_domain::sound::mixer::levels::amplitude(number),
                ),
                "highs" => ("hf", remuxd_domain::sound::mixer::levels::amplitude(number)),
                "closed" => (
                    "floor",
                    remuxd_domain::sound::mixer::levels::amplitude(number),
                ),
                "keys" => (
                    "keys_boost",
                    remuxd_domain::sound::mixer::levels::amplitude(number),
                ),
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

        // `remux destination title 2 Rust at midnight`: the id, then the words.
        "title" | "describe" => {
            let adapter: i64 = rest
                .first()
                .ok_or(format!("{verb} needs a destination id, then the words"))?
                .parse()
                .map_err(|_| "a destination id is a number".to_string())?;
            let words = rest[1..].join(" ");
            if words.is_empty() {
                return Err(format!(
                    "{verb} needs the words, as in `remux destination {verb} 2 Rust at midnight`"
                ));
            }
            Ok(Command::Retitle {
                adapter,
                title: (verb == "title").then_some(words.clone()),
                description: (verb == "describe").then_some(words),
            })
        }
        // `remux destination announce 2`: tell that destination's platform the title now.
        "announce" => {
            let adapter = rest
                .first()
                .ok_or("announce needs a destination id")?
                .parse()
                .map_err(|_| "a destination id is a number".to_string())?;
            Ok(Command::Announce { adapter })
        }
        // `remux chat delete 42`: that line of chat, out of the platform's chat for everybody.
        "delete" => {
            let seq = rest
                .first()
                .ok_or("delete needs the line's number")?
                .parse()
                .map_err(|_| "a line's number is a number".to_string())?;
            Ok(Command::Delete { seq })
        }
        // `remux destination category 2 509670 Science & Technology`: file that destination's live.
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
        // `remux destination categories 2 science`: where that destination's live can be filed.
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
        // `remux destination sandbox 2` / `remux destination sandbox 2 off`: a rehearsal nobody is told about.
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

fn parse_scene_element(words: &[String]) -> Result<Command, String> {
    use remuxd_domain::picture::scenes::{Element, ElementContent};
    if let [action, id] = words {
        if action == "remove" && !id.is_empty() {
            return Ok(Command::SceneElementRemove { id: id.clone() });
        }
    }
    let [action, kind, id, x, y, width, height, rest @ ..] = words else {
        return Err("scene layer takes add|set text|timer <id> <x> <y> <width> <height> <words|seconds>, or remove <id>".into());
    };
    let number = |v: &str| {
        v.parse::<u32>()
            .map_err(|_| format!("{v} is not a non-negative number"))
    };
    let content = match kind.as_str() {
        "text" if !rest.is_empty() => ElementContent::Text {
            text: rest.join(" "),
        },
        "timer" if rest.len() == 1 => ElementContent::Timer {
            seconds: number(&rest[0])?,
        },
        _ => return Err("element content takes text <words> or timer <seconds>".into()),
    };
    let element = Element {
        id: id.clone(),
        x: x.parse().map_err(|_| "x must be an integer")?,
        y: y.parse().map_err(|_| "y must be an integer")?,
        width: number(width)?,
        height: number(height)?,
        visible: true,
        shader: None,
        content,
    };
    if !element.valid() {
        return Err("element viewport must fit inside 1920x1080 and ID must be printable".into());
    }
    match action.as_str() {
        "add" => Ok(Command::SceneElementAdd { element }),
        "set" => Ok(Command::SceneElementSet { element }),
        _ => Err("scene layer takes add|set|remove".into()),
    }
}

fn parse_audio_layer(words: &[String]) -> Result<Command, String> {
    use remuxd_domain::sound::audio_layers::Source;
    let usage = "audio layer: add mic|app|screen <id> <device|name|display-id>, volume <id> <percent>, mute <id> on|off, duck <id> on|off|auto, remove <id>";
    match words {
        [add, kind, id, source @ ..] if add == "add" && !source.is_empty() => {
            let said = source.join(" ");
            let source = match kind.as_str() {
                "mic" => Source::mic(said),
                "app" => Source::app(said),
                "screen" if source.len() == 1 => Source::screen(
                    said.parse()
                        .map_err(|_| "screen needs a display id from `remux sources`")?,
                ),
                _ => return Err(usage.into()),
            };
            remuxd_domain::sound::audio_layers::Layer::new(id.clone(), source.clone())?;
            Ok(Command::AudioLayerAdd {
                id: id.clone(),
                source,
            })
        }
        [remove, id] if remove == "remove" => Ok(Command::AudioLayerRemove { id: id.clone() }),
        [volume, id, percent] if volume == "volume" => Ok(Command::AudioLayerVolume {
            id: id.clone(),
            volume: percentage(percent, "audio layer volume")?,
        }),
        [mute, id, on] if mute == "mute" => Ok(Command::AudioLayerMute {
            id: id.clone(),
            on: on_or(on)?,
        }),
        // Auto is the kind's: an app or a screen ducks, a microphone does not.
        [duck, id, said] if duck == "duck" => Ok(Command::AudioLayerDuck {
            id: id.clone(),
            duck: match said.as_str() {
                "auto" => remuxd_domain::sound::audio_layers::Duck::ByKind,
                said if on_or(said)? => remuxd_domain::sound::audio_layers::Duck::On,
                _ => remuxd_domain::sound::audio_layers::Duck::Off,
            },
        }),
        _ => Err(usage.into()),
    }
}

fn parse_layer(words: &[String]) -> Result<Command, String> {
    let id = |value: &str| -> Result<String, String> {
        if !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            Ok(value.into())
        } else {
            Err("layer id must contain only letters, digits, - or _".into())
        }
    };
    if matches!(words, [action, kind, ..] if matches!(action.as_str(), "add" | "set") && matches!(kind.as_str(), "text" | "timer"))
    {
        return parse_scene_element(words);
    }
    match words {
        [mirror, name, on] if mirror == "mirror" => Ok(Command::LayerMirror { id: id(name)?, on: on_or(on)? }),
        [sound, name, setting @ ..] if sound == "screen-sound" && setting.len() <= 1 => Ok(Command::LayerScreenSound {
            id: id(name)?, on: on_or(&setting.join(" "))?,
        }),
        [set, kind, name, display] if set == "set" && kind == "screen" => Ok(Command::LayerReplaceScreen {
            id: id(name)?,
            display: display.parse().map_err(|_| format!("{display} is not a display id; `remux sources` lists them"))?,
        }),
        [set, kind, name, query @ ..] if set == "set" && !query.is_empty() => {
            let id = id(name)?;
            let query = query.join(" ");
            match kind.as_str() {
                "camera" => Ok(Command::LayerReplaceCamera { id, device: query }),
                "window" => Ok(Command::LayerReplaceWindow { id, query }),
                _ => Err("layer set expects screen, camera or window".into()),
            }
        }
        [add, kind, name, display] if add == "add" && kind == "screen" => Ok(Command::LayerScreen {
            id: id(name)?,
            display: display.parse().map_err(|_| format!("{display} is not a display id; `remux sources` lists them"))?,
        }),
        [add, kind, name, query @ ..] if add == "add" && !query.is_empty() => {
            let id = id(name)?;
            let query = query.join(" ");
            match kind.as_str() {
                "camera" => Ok(Command::LayerCamera { id, device: query }),
                "window" => Ok(Command::LayerWindow { id, query }),
                _ => Err("layer add expects screen, camera or window".into()),
            }
        }
        [crop, name, off] if crop == "crop" && off == "off" => Ok(Command::LayerCrop { id: id(name)?, crop: None }),
        [crop, name, x, y, width, height] if crop == "crop" => {
            let number = |word: &str| word.parse::<u32>().map_err(|_| format!("{word:?} is not a non-negative source pixel"));
            Ok(Command::LayerCrop {
                id: id(name)?,
                crop: Some(remuxd_domain::picture::layers::Crop { x: number(x)?, y: number(y)?, width: number(width)?, height: number(height)? }),
            })
        }
        [shape, name, value] if shape == "shape" => Ok(Command::LayerShape {
            id: id(name)?,
            shape: match value.as_str() {
                "circle" => remuxd_domain::picture::scene::CameraShape::Circle,
                "rectangle" => remuxd_domain::picture::scene::CameraShape::Rectangle,
                _ => return Err("layer shape takes circle or rectangle".into()),
            },
        }),
        [position, name, value] if position == "position" && value == "default" => Ok(Command::LayerPosition { id: id(name)?, at: None }),
        [position, name, x, y] if position == "position" => {
            let (wide, tall) = remuxd_domain::picture::scene::CAMERA_OUTPUT;
            let point = |word: &str, limit: u32| -> Result<u32, String> {
                let value = word.parse::<u32>().map_err(|_| format!("{word} is not a non-negative pixel coordinate"))?;
                (value < limit).then_some(value).ok_or_else(|| format!("{word} must be less than {limit}"))
            };
            Ok(Command::LayerPosition { id: id(name)?, at: Some(remuxd_domain::picture::scene::CameraPosition { x: point(x, wide)?, y: point(y, tall)? }) })
        }
        [filter, name, path] if filter == "filter" => Ok(Command::LayerShader {
            id: id(name)?, path: (path != "off").then(|| path.clone()),
        }),
        [shot, name] if shot == "shot" => Ok(Command::LayerShot { id: id(name)? }),
        [hide, name] if hide == "hide" => Ok(Command::LayerVisible { id: id(name)?, on: false }),
        [show, name] if show == "show" => Ok(Command::LayerVisible { id: id(name)?, on: true }),
        [remove, name] if remove == "remove" => Ok(Command::LayerRemove { id: id(name)? }),
        [move_, name, index] if move_ == "move" => Ok(Command::LayerMove {
            id: id(name)?,
            index: index.parse().map_err(|_| "layer index must be a non-negative number")?,
        }),
        [change, name, x, y, width, height, degrees] if change == "transform" => {
            let number = |s: &str| s.parse().map_err(|_| format!("{s:?} is not a whole number"));
            let transform = remuxd_domain::picture::layers::Transform {
                x: number(x)?, y: number(y)?, width: width.parse().map_err(|_| "width must be a positive whole number")?, height: height.parse().map_err(|_| "height must be a positive whole number")?, degrees: number(degrees)?,
            };
            transform.validate()?;
            Ok(Command::LayerTransform { id: id(name)?, transform })
        }
        _ => Err("layer: add|set screen|camera|window <id> <display-id|name>, filter <id> <file.wgsl|off>, screen-sound <id> [on|off], hide|show <id>, shot <id>, crop <id> <x> <y> <width> <height>|off, shape <id> circle|rectangle, position <id> <x> <y>|default, remove <id>, move <id> <index>, or transform <id> <x> <y> <width> <height> <degrees>".into()),
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
/// typing `remux audio mute` means.
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
            remuxd_domain::sound::mixer::levels::decibels(hearing.gate_levels.full),
            remuxd_domain::sound::mixer::levels::decibels(hearing.gate_levels.hf),
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
fn render_plan(plan: &remuxd_domain::air::plan::Plan) -> String {
    let mut lines = Vec::new();
    if plan.on_air {
        lines.push("already on air".to_string());
    }
    lines.push(format!("scene     {}", plan.scene));
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

pub fn render_scene_list(status: &Status) -> String {
    status
        .scenes
        .iter()
        .map(|scene| {
            format!(
                "{}{} ({} layers)",
                if scene.name == status.active_scene {
                    "* "
                } else {
                    "  "
                },
                scene.name,
                scene.ordered_ids().len()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
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
    if let Some(shader) = &status.shader {
        said.push(format!("filter {shader}"));
    }
    // Said only when it is on: it is the exception, and the one an
    // operator wants to be reminded of before a call comes in.
    if status.layers.is_empty() {
        said.push("no layers".into());
    }
    if let Some(scene) = status
        .scenes
        .iter()
        .find(|scene| scene.name == status.active_scene)
    {
        for (index, element) in scene.elements.iter().enumerate() {
            let content = match &element.content {
                remuxd_domain::picture::scenes::ElementContent::Text { text } => {
                    format!("text {:?}", plain(text))
                }
                remuxd_domain::picture::scenes::ElementContent::Timer { seconds } => {
                    format!("timer {seconds}s")
                }
            };
            said.push(format!(
                "element {} at {},{} {}x{} (order {index}): {content}",
                element.id, element.x, element.y, element.width, element.height
            ));
        }
    }
    for (index, layer) in status.layers.iter().enumerate() {
        said.push(format!(
            "layer {}: {:?} {} ({}x{} source) at {},{} {}x{} rotated {}° (order {index}){}{}{}{}",
            layer.id,
            layer.source.kind,
            plain(&layer.source.name),
            layer.source.width,
            layer.source.height,
            layer.transform.x,
            layer.transform.y,
            layer.transform.width,
            layer.transform.height,
            layer.transform.degrees,
            layer.crop.map_or(String::new(), |crop| format!(
                " crop {},{} {}x{}",
                crop.x, crop.y, crop.width, crop.height
            )),
            layer
                .shape
                .map_or(String::new(), |shape| format!(" {shape:?}")),
            if layer.visible { "" } else { " hidden" },
            layer
                .shader
                .as_ref()
                .map_or(String::new(), |path| format!(" filter {path}"))
        ));
    }
    for layer in &status.audio_layers {
        let source = match layer.source.kind {
            remuxd_domain::sound::audio_layers::Kind::Mic => {
                layer.source.device.as_deref().unwrap_or("?")
            }
            remuxd_domain::sound::audio_layers::Kind::App => {
                layer.source.name.as_deref().unwrap_or("?")
            }
            remuxd_domain::sound::audio_layers::Kind::Screen => "display",
        };
        said.push(format!(
            "audio layer {}: {:?} {} ({}%){}{}",
            layer.id,
            layer.source.kind,
            plain(source),
            (layer.volume * 100.0).round(),
            if layer.muted { " muted" } else { "" },
            match layer.duck {
                remuxd_domain::sound::audio_layers::Duck::ByKind => "",
                remuxd_domain::sound::audio_layers::Duck::On => " ducked",
                remuxd_domain::sound::audio_layers::Duck::Off => " not ducked",
            }
        ));
    }
    if status.screen_sound {
        let audible = status.screen_sound_layer.as_deref().is_some_and(|id| {
            status
                .layers
                .iter()
                .any(|layer| layer.id == id && layer.visible)
        });
        said.push(
            if audible {
                "screen sound out"
            } else {
                "screen sound paused (hidden)"
            }
            .into(),
        );
    }
    if let Some(app) = &status.app_audio {
        said.push(format!(
            "app audio {app} ({}%)",
            (status.app_audio_volume * 100.0).round()
        ));
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
        // `remux audio mute` and read back a line with no word for it in would
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
        "scene {} ({} saved)",
        status.active_scene,
        status.scenes.len()
    ));
    said.push(format!(
        "{}x{} at {} frames",
        status.scene_flowing.width, status.scene_flowing.height, status.scene_flowing.frames
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
    // The windows last and only counted: there are dozens, and `remux scene layer add window`
    // takes part of a title rather than an id, so the list is not the way in.
    if !devices.windows.is_empty() {
        lines.push(format!(
            "windows: {} of them, named by part of a title",
            devices.windows.len()
        ));
    }
    lines.join("\n")
}

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
    fn scene_cli_parses_names_and_shows_list() {
        let words = |items: &[&str]| {
            items
                .iter()
                .map(|item| (*item).into())
                .collect::<Vec<String>>()
        };
        assert_eq!(
            super::parse(&words(&["scene", "create", "Close Up"])),
            Ok(remuxd_domain::protocol::Command::SceneCreate {
                name: "Close Up".into()
            })
        );
        assert_eq!(
            super::parse(&words(&["scene", "duplicate", "Close Up"])),
            Ok(remuxd_domain::protocol::Command::SceneDuplicate {
                name: "Close Up".into()
            })
        );
        assert_eq!(
            super::parse(&words(&["scene", "switch", "Close Up"])),
            Ok(remuxd_domain::protocol::Command::SceneSwitch {
                name: "Close Up".into()
            })
        );
        assert_eq!(
            super::parse(&words(&["scene", "delete", "Close Up"])),
            Ok(remuxd_domain::protocol::Command::SceneDelete {
                name: "Close Up".into()
            })
        );
        assert_eq!(
            super::parse(&words(&["scene", "list"])),
            Ok(remuxd_domain::protocol::Command::Status)
        );
        assert!(super::parse(&words(&["scene", "create"])).is_err());
        assert!(
            super::render_scene_list(&remuxd_domain::protocol::Status::default())
                .contains("* default")
        );
    }

    #[test]
    fn a_layer_filter_takes_a_path_or_off() {
        let words = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        assert_eq!(
            super::parse(&words("scene layer filter face effect.wgsl")),
            Ok(remuxd_domain::protocol::Command::LayerShader {
                id: "face".into(),
                path: Some("effect.wgsl".into())
            })
        );
        assert_eq!(
            super::parse(&words("scene layer filter face off")),
            Ok(remuxd_domain::protocol::Command::LayerShader {
                id: "face".into(),
                path: None
            })
        );
        assert!(super::parse(&words("scene layer filter face")).is_err());
    }

    #[test]
    fn layer_commands_are_strict_and_round_trip() {
        let words = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        assert_eq!(
            super::parse(&words("scene layer add window editor Ghostty Window")),
            Ok(remuxd_domain::protocol::Command::LayerWindow {
                id: "editor".into(),
                query: "Ghostty Window".into()
            })
        );
        assert_eq!(
            super::parse(&words("scene layer add screen desktop 3")),
            Ok(remuxd_domain::protocol::Command::LayerScreen {
                id: "desktop".into(),
                display: 3
            })
        );
        assert_eq!(
            super::parse(&words("scene layer set window desktop Emacs Notes")),
            Ok(remuxd_domain::protocol::Command::LayerReplaceWindow {
                id: "desktop".into(),
                query: "Emacs Notes".into(),
            })
        );
        assert_eq!(
            super::parse(&words("scene layer set screen desktop 3")),
            Ok(remuxd_domain::protocol::Command::LayerReplaceScreen {
                id: "desktop".into(),
                display: 3
            })
        );
        assert_eq!(
            super::parse(&words("scene layer set camera host FaceTime")),
            Ok(remuxd_domain::protocol::Command::LayerReplaceCamera {
                id: "host".into(),
                device: "FaceTime".into()
            })
        );
        assert!(super::parse(&words("scene layer set screen desktop nope")).is_err());
        assert_eq!(
            super::parse(&words("scene layer hide desktop")),
            Ok(remuxd_domain::protocol::Command::LayerVisible {
                id: "desktop".into(),
                on: false
            })
        );
        assert_eq!(
            super::parse(&words("scene layer show desktop")),
            Ok(remuxd_domain::protocol::Command::LayerVisible {
                id: "desktop".into(),
                on: true
            })
        );
        assert!(super::parse(&words("scene layer add screen desktop three")).is_err());
        assert!(super::parse(&words("scene layer add screen desktop 3 extra")).is_err());
        assert_eq!(
            super::parse(&words("scene layer shape face circle")),
            Ok(remuxd_domain::protocol::Command::LayerShape {
                id: "face".into(),
                shape: remuxd_domain::picture::scene::CameraShape::Circle
            })
        );
        assert_eq!(
            super::parse(&words("scene layer position face 300 200")),
            Ok(remuxd_domain::protocol::Command::LayerPosition {
                id: "face".into(),
                at: Some(remuxd_domain::picture::scene::CameraPosition { x: 300, y: 200 })
            })
        );
        assert_eq!(
            super::parse(&words("scene layer position face default")),
            Ok(remuxd_domain::protocol::Command::LayerPosition {
                id: "face".into(),
                at: None
            })
        );
        assert!(super::parse(&words("scene layer shape face oval")).is_err());
        assert!(super::parse(&words("scene layer position face 1920 0")).is_err());
        assert_eq!(
            super::parse(&words("scene layer move editor 0")),
            Ok(remuxd_domain::protocol::Command::LayerMove {
                id: "editor".into(),
                index: 0
            })
        );
        assert_eq!(
            super::parse(&words("scene layer crop editor 10 20 300 200")),
            Ok(remuxd_domain::protocol::Command::LayerCrop {
                id: "editor".into(),
                crop: Some(remuxd_domain::picture::layers::Crop {
                    x: 10,
                    y: 20,
                    width: 300,
                    height: 200
                })
            })
        );
        assert_eq!(
            super::parse(&words("scene layer crop editor off")),
            Ok(remuxd_domain::protocol::Command::LayerCrop {
                id: "editor".into(),
                crop: None
            })
        );
        assert!(super::parse(&words("scene layer crop editor -1 0 100 100")).is_err());
        assert!(super::parse(&words("scene layer transform editor 10 20 0 270 90")).is_err());
        assert!(super::parse(&words("scene layer add camera bad/id Cam")).is_err());
    }
    #[test]
    fn the_chat_reads_one_message_at_a_time_with_a_rule_between() {
        use remuxd_domain::protocol::{ChatLine, Reply};
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
        use remuxd_domain::protocol::{ChatLine, Reply};
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
        use remuxd_domain::protocol::{ChatLine, Reply};
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
        for words in ["chat read -f", "chat read --follow", "chat read follow"] {
            assert_eq!(follow(&normalize(&w(words)).unwrap()), (true, w("chat")));
        }
        assert_eq!(
            follow(&normalize(&w("chat read")).unwrap()),
            (false, w("chat"))
        );
        assert_eq!(
            follow(&w("status -f")),
            (false, w("status -f")),
            "only the chat follows"
        );
        assert!(matches!(
            parse(&w("chat read")),
            Ok(Command::Chat {
                since: 0,
                follow: false
            })
        ));
    }

    use super::*;

    fn said(line: &str) -> Result<Command, String> {
        let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        parse_wire_words(&words)
    }

    fn typed(line: &str) -> Result<Command, String> {
        let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        parse(&words)
    }

    #[test]
    fn audio_layers_have_grouped_commands_and_distinct_ids() {
        use remuxd_domain::sound::audio_layers::Source;
        assert_eq!(
            typed("audio layer add app chat Safari"),
            Ok(Command::AudioLayerAdd {
                id: "chat".into(),
                source: Source::app("Safari".into()),
            })
        );
        assert_eq!(
            typed("audio layer add screen desktop 3"),
            Ok(Command::AudioLayerAdd {
                id: "desktop".into(),
                source: Source::screen(3),
            })
        );
        assert_eq!(
            typed("audio layer volume chat 80"),
            Ok(Command::AudioLayerVolume {
                id: "chat".into(),
                volume: 0.8
            })
        );
        assert_eq!(
            typed("audio layer mute chat on"),
            Ok(Command::AudioLayerMute {
                id: "chat".into(),
                on: true
            })
        );
        assert_eq!(
            typed("audio layer remove chat"),
            Ok(Command::AudioLayerRemove { id: "chat".into() })
        );
        use remuxd_domain::sound::audio_layers::Duck;
        for (said, duck) in [("off", Duck::Off), ("on", Duck::On), ("auto", Duck::ByKind)] {
            assert_eq!(
                typed(&format!("audio layer duck chat {said}")),
                Ok(Command::AudioLayerDuck {
                    id: "chat".into(),
                    duck
                })
            );
        }
        assert!(typed("audio layer duck chat maybe").is_err());
        assert!(typed("audio layer add screen desktop not-an-id").is_err());
    }

    #[test]
    fn json_option_is_global_and_replies_keep_the_wire_shape() {
        let words = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        for text in [
            "--json music genre off",
            "music --json genre off",
            "music genre off --json",
        ] {
            let (json, clean) = output_mode(&words(text));
            assert!(json);
            assert_eq!(parse(&clean), Ok(Command::Genre { name: "off".into() }));
        }
        let (json, clean) = output_mode(&words("chat read --follow --json"));
        assert!(json);
        assert_eq!(follow(&normalize(&clean).unwrap()), (true, words("chat")));
        assert_eq!(output_mode(&words("status")), (false, words("status")));
        assert_eq!(output_mode(&words("status -j")), (true, words("status")));
        for reply in [
            Reply::Ok,
            Reply::Error {
                message: "refused".into(),
            },
            Reply::Status(Box::default()),
            Reply::Devices(Devices::default()),
            Reply::Chat {
                reachable: false,
                lines: vec![],
            },
            Reply::Shot {
                jpeg: "aGVsbG8=".into(),
                width: 2,
                height: 3,
            },
        ] {
            let encoded = render_json(&reply);
            assert_eq!(remuxd_domain::protocol::decode_reply(&encoded), Ok(reply));
            assert!(!encoded.contains('\n'));
        }
    }

    #[test]
    fn grouped_commands_translate_to_the_same_wire_commands() {
        for (grouped, old) in [
            ("audio mic USB Microphone", "mic USB Microphone"),
            ("audio mute off", "mute off"),
            ("audio vol 80", "vol 80"),
            ("audio gate full 0.2", "gate full 0.2"),
            ("audio duck -18", "duck -18"),
            ("audio monitor", "monitor"),
            ("audio screen-sound on", "screen-sound on"),
            ("audio app Safari", "app-audio Safari"),
            ("audio app off", "app-audio off"),
            ("audio app-volume 65", "app-audio-volume 65"),
            ("audio levels", "levels"),
            ("music play", "music on"),
            ("music off", "music off"),
            ("music genre lofi", "music lofi"),
            ("music next", "next"),
            ("music vol 30", "mvol 30"),
            ("music stream off", "stream-music off"),
            ("scene shot", "shot"),
            ("destination arm 2", "arm 2"),
            ("destination disarm 2", "disarm 2"),
            (
                "destination title 2 Rust at midnight",
                "title 2 Rust at midnight",
            ),
            (
                "destination describe 2 the native engine",
                "describe 2 the native engine",
            ),
            ("destination announce 2", "announce 2"),
            (
                "destination category 2 509670 Science & Technology",
                "category 2 509670 Science & Technology",
            ),
            ("destination categories 2 science", "categories 2 science"),
            ("destination sandbox 2 off", "sandbox 2 off"),
            ("destination disconnect 2", "disconnect 2"),
            ("chat read", "chat"),
            ("chat hide 42", "hide 42"),
            ("chat delete 42", "delete 42"),
        ] {
            assert_eq!(typed(grouped), said(old), "{grouped}");
        }
        let words = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        let expanded = normalize(&words("chat read --follow")).unwrap();
        assert_eq!(follow(&expanded), (true, words("chat")));
        assert!(normalize(&words("chat -f")).is_err());
        assert!(normalize(&words("music lofi")).is_err());
        assert!(normalize(&words("destination nope"))
            .unwrap_err()
            .contains("destination"));
        assert!(normalize(&words("audio")).is_err());
        assert!(normalize(&words("music play extra")).is_err());
        assert!(normalize(&words("music genre")).is_err());
        assert!(normalize(&words("chat read nonsense")).is_err());
        assert_eq!(
            typed("music genre off"),
            Ok(Command::Genre { name: "off".into() })
        );
    }

    #[test]
    fn flat_commands_and_old_aliases_are_not_public_commands() {
        for words in [
            "arm 2",
            "disarm 2",
            "title 2 words",
            "hide 42",
            "delete 42",
            "screen 3",
            "camera off",
            "shot",
            "mute",
            "gate full 0.2",
            "vol 80",
            "mvol 30",
            "next",
            "stream-music off",
            "sandbox 2",
            "music",
            "music lofi",
            "chat",
            "chat -f",
            "present",
            "watching",
            "meters",
            "devices",
            "permissions",
            "preview",
            "skip",
            "go-live",
            "hide-everything",
            "update 2",
        ] {
            let error = typed(words).expect_err(words);
            assert!(
                error.contains("not a command") || error.contains("Usage: remux"),
                "{words}: {error}"
            );
        }
        assert_eq!(typed("status"), Ok(Command::Status));
        assert_eq!(typed("live"), Ok(Command::GoLive));
        assert_eq!(
            typed("chat read"),
            Ok(Command::Chat {
                since: 0,
                follow: false
            })
        );
    }

    #[test]
    fn screen_sound_from_a_hidden_layer_reads_as_paused() {
        let status = Status {
            screen_sound: true,
            screen_sound_layer: Some("desk".into()),
            ..Status::default()
        };
        let said = render(&Reply::Status(Box::new(status)));
        assert!(said.contains("screen sound paused (hidden)"), "{said}");
    }

    #[test]
    fn a_status_is_one_line_a_person_can_read() {
        let status = Status {
            on_air: true,
            mic: Some("HyperX DuoCast".into()),
            muted: true,
            scene_flowing: remuxd_domain::protocol::Flowing {
                width: 1920,
                height: 1080,
                frames: 900,
                ..Default::default()
            },
            ..Default::default()
        };
        let said = render(&Reply::Status(Box::new(status)));
        assert!(said.starts_with("on air, "), "{said}");
        assert!(said.contains("mic HyperX DuoCast (muted)"), "{said}");
        assert!(said.contains("1920x1080 at 900 frames"), "{said}");
        assert!(!said.contains('\n'), "one line: {said}");

        // The chosen microphone is not delivering: the line says so, because
        // its name alone read as a working one the night it had left.
        let unplugged = remuxd_domain::protocol::Status {
            mic: Some("Razer".into()),
            hearing: remuxd_domain::protocol::Hearing {
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
        assert!(said.contains("no layers"), "{said}");
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
            apps: vec![],
            genres: vec![Named {
                id: "lofi".into(),
                name: "Lofi".into(),
            }],
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
        assert!(missing.contains("remux audio gate opens -30"), "{missing}");
    }

    // The panel's heartbeat: one `present` from a shell armed the lease and
    // the engine quit five seconds later, live included.
    #[test]
    fn present_is_the_panel_s_word_and_not_a_shell_s() {
        assert!(said("present").is_err());
    }

    #[test]
    fn a_shot_is_of_the_scene_unless_it_says_otherwise() {
        assert_eq!(typed("scene shot"), Ok(Command::Shot { of: Framed::Scene }));
        assert!(typed("scene shot camera").is_err());
    }

    #[test]
    fn the_short_ones_are_themselves() {
        assert_eq!(said("status"), Ok(Command::Status));
        assert_eq!(typed("sources"), Ok(Command::Devices));
        assert_eq!(said("live"), Ok(Command::GoLive));
        assert_eq!(said("stop"), Ok(Command::Stop));
        assert_eq!(said("cut"), Ok(Command::HideEverything));
        assert_eq!(said("levels"), Ok(Command::Levels));
        assert_eq!(typed("audio levels"), Ok(Command::Levels));
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
            complaint.contains("remux sources"),
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
        assert_eq!(
            typed("destination announce 2"),
            Ok(Command::Announce { adapter: 2 })
        );
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
        assert_eq!(
            said("app-audio Brave Browser"),
            Ok(Command::AppAudio {
                app: Some("Brave Browser".into())
            })
        );
        assert_eq!(said("app-audio off"), Ok(Command::AppAudio { app: None }));
    }

    #[test]
    fn scene_filter_is_one_file_or_off() {
        assert_eq!(
            typed("scene filter /tmp/invert.wgsl"),
            Ok(Command::Shader {
                path: Some("/tmp/invert.wgsl".into())
            })
        );
        assert_eq!(
            typed("scene filter off"),
            Ok(Command::Shader { path: None })
        );
        for bad in [
            "scene filter",
            "scene filter a b",
            "scene shader off",
            "scene layer shader face off",
        ] {
            assert!(typed(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn camera_shape_requires_one_of_two_explicit_shapes() {
        use remuxd_domain::picture::scene::CameraShape;
        assert_eq!(
            typed("scene layer shape face circle"),
            Ok(Command::LayerShape {
                id: "face".into(),
                shape: CameraShape::Circle
            })
        );
        assert_eq!(
            typed("scene layer shape face rectangle"),
            Ok(Command::LayerShape {
                id: "face".into(),
                shape: CameraShape::Rectangle
            })
        );
        for bad in [
            "scene layer shape",
            "scene layer shape face oval",
            "scene layer shape face circle rectangle",
        ] {
            assert!(typed(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn camera_position_takes_two_scene_pixels_or_default() {
        assert_eq!(
            typed("scene layer mirror face on"),
            Ok(Command::LayerMirror {
                id: "face".into(),
                on: true
            })
        );
        assert!(typed("video camera-position 300 200").is_err());
        assert!(typed("video screen 1").is_err());
        assert!(typed("video layer add camera face c920").is_err());
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
    fn scene_elements_and_timers_have_scene_verbs() {
        assert_eq!(
            typed("scene timer start clock"),
            Ok(Command::SceneTimerStart { id: "clock".into() })
        );
        assert_eq!(
            typed("scene timer stop clock"),
            Ok(Command::SceneTimerStop { id: "clock".into() })
        );
        assert_eq!(
            typed("scene layer remove title"),
            Ok(Command::LayerRemove { id: "title".into() })
        );
        assert_eq!(
            typed("scene layer add text title 20 30 500 90 Hello world"),
            Ok(Command::SceneElementAdd {
                element: remuxd_domain::picture::scenes::Element {
                    id: "title".into(),
                    x: 20,
                    y: 30,
                    width: 500,
                    height: 90,
                    visible: true,
                    shader: None,
                    content: remuxd_domain::picture::scenes::ElementContent::Text {
                        text: "Hello world".into()
                    }
                }
            })
        );
        assert!(typed("scene countdown 5").is_err());
        assert!(typed("scene text BRB back soon").is_err());
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
    fn standalone_help_is_local_and_explains_destination_ids() {
        for word in ["help", "-h", "--help"] {
            let text = help(&[word.into()])
                .expect("a standalone help request")
                .expect("known help");
            assert!(text.contains("Usage: remux"), "{text}");
            assert!(text.contains("arm"), "{text}");
            assert!(text.contains("remux destination list"), "{text}");
        }
        assert!(help(&[]).is_none());
        assert!(help(&["help".into(), "extra".into()]).unwrap().is_err());
        assert!(help(&["status".into()]).is_none());
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
            complaint.contains("scene"),
            "and then says which groups it can do"
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
    /// `scene list`: the scenes, the active one marked.
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
        until: remuxd_domain::wait::Until,
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
    Daemon(remuxd_domain::daemon::Verb),
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
    // A genre is never a switch, even one named "off": it goes as itself.
    if matches!(words.first().map(String::as_str), Some("music"))
        && matches!(words.get(1).map(String::as_str), Some("genre"))
    {
        return Ok(Ask {
            command: Some(parse(&words)?),
            view: View::Reply,
            format,
            follow: false,
        });
    }
    // The shell's grouped words, as the flat ones the engine answers.
    let words = normalize(&words)?;
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
                let mut url =
                    remuxd_domain::air::destinations::ingest_of(&platform).map(String::from);
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
            (None, _) => remuxd_domain::app::session::default_base(),
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
            view: View::Daemon(remuxd_domain::daemon::Verb::parse(&words[1..])?),
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
        let until = remuxd_domain::wait::Until::parse(
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
    // `scene shot --out f.jpg`, `scene layer shot <id> --out f.jpg`: the
    // picture written to a file by the shell.
    let a_shot = match words.as_slice() {
        [first, ..] if first == "shot" => true,
        [first, second, ..] => first == "layer" && second == "shot",
        _ => false,
    };
    if a_shot {
        if let Some(at) = words.iter().position(|w| w == "--out") {
            let file = words
                .get(at + 1)
                .cloned()
                .ok_or("--out takes a file name")?;
            let mut rest = words.clone();
            rest.drain(at..at + 2);
            return Ok(Ask {
                command: Some(parse_wire_words(&rest)?),
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
        Some("scenes") => (Command::Status, View::Scenes),
        Some("health") => (Command::Status, View::Health),
        Some("destinations") | Some("dests") => (Command::Status, View::Destinations),
        Some("log") => (Command::Status, View::Log),
        Some("categories") => match parse_wire_words(&words)? {
            Command::Categories { adapter, query } => (
                Command::Categories {
                    adapter,
                    query: query.clone(),
                },
                View::Categories { adapter, query },
            ),
            other => (other, View::Reply),
        },
        _ => (parse_wire_words(&words)?, View::Reply),
    };
    Ok(Ask {
        command: Some(command),
        view,
        format,
        follow,
    })
}

/// What the shell answers by itself, when there is nothing to ask.
pub fn local(view: &View, format: Format) -> String {
    match view {
        View::Clips => {
            let root = remuxd_domain::sound::clips::root();
            let names = remuxd_domain::sound::clips::list(&root);
            if names.is_empty() {
                format!("no clips in {} (remux config)", root.display())
            } else {
                names.join("\n")
            }
        }
        View::Guide => help::GUIDE.to_string(),
        View::Config => remuxd_domain::config::describe(),
        View::History => {
            let all = remuxd_domain::air::history::read(&remuxd_domain::air::history::path());
            match format {
                Format::Json => json(&all),
                Format::Prose if all.is_empty() => "no lives on record yet".into(),
                Format::Prose => all
                    .iter()
                    .map(|b| {
                        format!(
                            "{}  {:>8}  {:<24} {}{}",
                            remuxd_domain::air::journal::clock_of(b.started),
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
            "wire_up": schemars::schema_for!(remuxd_domain::app::wire::Up),
            "wire_line": schemars::schema_for!(remuxd_domain::app::wire::Line),
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
        (Format::Json, View::Scenes, Reply::Status(status)) => json(&serde_json::json!({
            "scenes": status.scenes,
            "active_scene": status.active_scene
        })),
        (Format::Prose, View::Scenes, Reply::Status(status)) => render_scene_list(status),
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
fn render_destinations(rows: &[remuxd_domain::protocol::Destination]) -> String {
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

fn render_gate(g: &remuxd_domain::sound::mixer::gate::GateParams) -> String {
    use remuxd_domain::sound::mixer::levels::decibels;
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
    use remuxd_domain::sound::mixer::levels::decibels;
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
    lines.push(format!("scene {}", status.active_scene));
    if status.layers.is_empty() {
        lines.push("no layers".into());
    }
    for layer in &status.layers {
        let t = layer.transform;
        lines.push(format!(
            "layer {} {} at {},{} {}x{}{}{}{}",
            layer.id,
            layer.source.name,
            t.x,
            t.y,
            t.width,
            t.height,
            if t.degrees == 0 {
                String::new()
            } else {
                format!(" {}°", t.degrees)
            },
            if layer.visible { "" } else { ", hidden" },
            if layer.mirrored { ", mirrored" } else { "" }
        ));
    }
    if status.screen_sound {
        lines.push(match status.hearing_apps.as_slice() {
            [] => "screen sound out".into(),
            apps => format!("screen sound out: {}", apps.join(", ")),
        });
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
        remuxd_domain::sound::music::music_fader_db(status.faders.music),
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
        status.scene_flowing.width,
        status.scene_flowing.height,
        status.scene_flowing.frames
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
    use remuxd_domain::protocol::Destination;

    fn w(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn json_is_the_shell_s_flag_and_never_reaches_the_engine() {
        let ask = read(&w("status --json")).unwrap();
        assert_eq!(ask.command, Some(Command::Status));
        assert_eq!(ask.format, Format::Json);
        assert_eq!(
            read(&w("-j audio mute")).unwrap().command,
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
                base: remuxd_domain::app::session::default_base()
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
            read(&w("chat url ws://localhost:9999")).unwrap().view,
            View::ChatKeep("ws://localhost:9999".into())
        );
        assert!(read(&w("chat url")).is_err());
        assert_eq!(read(&w("config")).unwrap().view, View::Config);
        assert_eq!(
            read(&w("bug --open")).unwrap().view,
            View::Bug { open: true }
        );
        assert_eq!(
            read(&w("daemon stop --force")).unwrap().view,
            View::Daemon(remuxd_domain::daemon::Verb::Stop { force: true })
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
        let layer = read(&w("scene layer shot desk --out /tmp/y.jpg")).unwrap();
        assert_eq!(layer.view, View::ShotTo("/tmp/y.jpg".into()));
        assert_eq!(
            layer.command,
            Some(Command::LayerShot { id: "desk".into() })
        );
        let shot = read(&w("scene shot --out /tmp/x.jpg")).unwrap();
        assert_eq!(shot.view, View::ShotTo("/tmp/x.jpg".into()));
        assert_eq!(shot.command, Some(Command::Shot { of: Framed::Scene }));
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
        assert_eq!(
            parse(&w("audio denoise")),
            Ok(Command::Denoise { on: true })
        );
        assert_eq!(
            parse(&w("audio clip clap")),
            Ok(Command::Clip {
                name: "clap".into()
            })
        );
        assert_eq!(
            parse(&w("audio hear Spotify, Brave")),
            Ok(Command::Hear {
                apps: vec!["Spotify".into(), "Brave".into()]
            })
        );
        assert_eq!(
            parse(&w("audio hear off")),
            Ok(Command::Hear { apps: vec![] })
        );
        assert!(parse(&w("audio hear")).is_err());
        assert!(parse(&w("audio clip")).is_err());
        assert_eq!(read(&w("audio clips")).unwrap().view, View::Clips);
        assert!(parse(&w("live --confirm")).is_err());
        let asks = read(&w("live")).unwrap();
        assert_eq!(
            (asks.command, asks.view),
            (Some(Command::Plan), View::Confirm)
        );
        let plan = remuxd_domain::air::plan::Plan::of(&Status::default());
        let shown = render(&Reply::Plan(plan.clone()));
        assert!(shown.contains("! no destination is armed"), "{shown}");
        assert!(
            shown.ends_with(&format!("plan {}", plan.fingerprint)),
            "{shown}"
        );
    }

    #[test]
    fn the_scene_list_marks_the_active_scene() {
        assert_eq!(read(&w("scene list")).unwrap().view, View::Scenes);
        let scene = |name: &str| remuxd_domain::picture::scenes::Scene {
            name: name.into(),
            layers: vec![],
            order: vec![],
            elements: vec![],
            shader: None,
        };
        let status = Status {
            scenes: vec![scene("code"), scene("talk")],
            active_scene: "talk".into(),
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
            "  code (0 layers)\n* talk (0 layers)"
        );
    }

    #[test]
    fn the_gate_speaks_the_panel_s_words_in_db_and_resets_whole() {
        assert_eq!(read(&w("audio gate")).unwrap().view, View::Gate);
        let Ok(Command::Gate { patch }) = parse(&w("audio gate opens -20")) else {
            panic!()
        };
        assert!(
            (patch["full"].as_f64().unwrap() - 0.1).abs() < 1e-9,
            "{patch}"
        );
        let Ok(Command::Gate { patch }) = parse(&w("audio gate reset")) else {
            panic!()
        };
        assert_eq!(patch["hold_ms"], 450.0);
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
        assert_eq!(
            read(&w("destination list")).unwrap().view,
            View::Destinations
        );
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
