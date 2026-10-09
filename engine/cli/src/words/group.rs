//! Human-sized command groups. Expand them to the existing wire vocabulary;
//! no group name is sent over the socket and old one-word commands still work.

pub(super) struct Action {
    pub name: &'static str,
    pub target: &'static str,
    pub prefix: &'static [&'static str],
    pub args: &'static str,
    pub summary: &'static str,
}

pub(super) struct Group {
    pub name: &'static str,
    pub summary: &'static str,
    pub actions: &'static [Action],
}

const AUDIO: &[Action] = &[
    Action {
        name: "layer",
        target: "audio-layer",
        prefix: &[],
        args: "add mic <id> <device> | add app <id> <name> | add system <id> | volume <id> <percent> | mute <id> on|off | duck <id> on|off|auto | remove <id>",
        summary: "Manage the active scene's audio captures by ID; an app or screen ducks under the voice unless told off.",
    },
    Action {
        name: "mic",
        target: "mic",
        prefix: &[],
        args: "[name|off]",
        summary: "Select or close the microphone.",
    },
    Action {
        name: "mute",
        target: "mute",
        prefix: &[],
        args: "[on|off]",
        summary: "Mute the microphone.",
    },
    Action {
        name: "vol",
        target: "vol",
        prefix: &[],
        args: "<percent>",
        summary: "Set microphone volume.",
    },
    Action {
        name: "gate",
        target: "gate",
        prefix: &[],
        args: "[reset | opens|highs|closed|keys <dB> | hold_ms|attack_ms|hf_attack_ms <ms>]",
        summary: "Show or tune the microphone gate.",
    },
    Action {
        name: "duck",
        target: "duck",
        prefix: &[],
        args: "<dB>",
        summary: "Set how far music dips under speech.",
    },
    Action {
        name: "monitor",
        target: "monitor",
        prefix: &[],
        args: "[on|off]",
        summary: "Hear the mix through the speakers.",
    },
    Action {
        name: "levels",
        target: "levels",
        prefix: &[],
        args: "[-f]",
        summary: "Read the sound levels.",
    },
    Action {
        name: "denoise",
        target: "denoise",
        prefix: &[],
        args: "[on|off]",
        summary: "Take the room out of the microphone before the gate.",
    },
    Action {
        name: "clip",
        target: "play",
        prefix: &[],
        args: "<clip|file>",
        summary: "Play one clip once over the mix.",
    },
    Action {
        name: "clips",
        target: "clips",
        prefix: &[],
        args: "",
        summary: "List the clips `audio clip` can play.",
    },
];

const MUSIC: &[Action] = &[
    Action {
        name: "play",
        target: "music",
        prefix: &["on"],
        args: "",
        summary: "Start playing music.",
    },
    Action {
        name: "off",
        target: "music",
        prefix: &["off"],
        args: "",
        summary: "Stop playing music.",
    },
    Action {
        name: "genre",
        target: "music",
        prefix: &[],
        args: "<name>",
        summary: "Choose a music genre.",
    },
    Action {
        name: "next",
        target: "next",
        prefix: &[],
        args: "",
        summary: "Skip to the next track.",
    },
    Action {
        name: "vol",
        target: "mvol",
        prefix: &[],
        args: "<percent>",
        summary: "Set music volume.",
    },
    Action {
        name: "stream",
        target: "stream-music",
        prefix: &[],
        args: "[on|off]",
        summary: "Include music in the live mix.",
    },
];

const DESTINATION: &[Action] = &[
    Action {
        name: "list",
        target: "destinations",
        prefix: &[],
        args: "",
        summary: "List the destinations with their IDs.",
    },
    Action {
        name: "add",
        target: "destination",
        prefix: &["add"],
        args: "twitch|youtube|custom <name> [--url <rtmp>] [--key -|--key-file <file>]",
        summary: "Keep a destination; its key is read from stdin or a file.",
    },
    Action {
        name: "rm",
        target: "destination",
        prefix: &["rm"],
        args: "<id|name>",
        summary: "Forget a destination.",
    },
    Action {
        name: "arm",
        target: "arm",
        prefix: &[],
        args: "<id>",
        summary: "Include a destination in the next live.",
    },
    Action {
        name: "disarm",
        target: "disarm",
        prefix: &[],
        args: "<id>",
        summary: "Leave a destination out of the next live.",
    },
    Action {
        name: "title",
        target: "title",
        prefix: &[],
        args: "<id> <words>",
        summary: "Change its title.",
    },
    Action {
        name: "describe",
        target: "describe",
        prefix: &[],
        args: "<id> <words>",
        summary: "Change its description.",
    },
    Action {
        name: "announce",
        target: "announce",
        prefix: &[],
        args: "<id>",
        summary: "Send its current details to the platform.",
    },
    Action {
        name: "category",
        target: "category",
        prefix: &[],
        args: "<id> <category id> <name>",
        summary: "Set its category.",
    },
    Action {
        name: "categories",
        target: "categories",
        prefix: &[],
        args: "<id> [query]",
        summary: "Search the platform's categories.",
    },
    Action {
        name: "sandbox",
        target: "sandbox",
        prefix: &[],
        args: "<id> [on|off]",
        summary: "Rehearse without a public audience.",
    },
    Action {
        name: "disconnect",
        target: "disconnect",
        prefix: &[],
        args: "<id>",
        summary: "Disconnect its platform account.",
    },
];

const CHAT: &[Action] = &[
    Action {
        name: "read",
        target: "chat",
        prefix: &[],
        args: "[-f|--follow|follow]",
        summary: "Read chat; follow for new lines.",
    },
    Action {
        name: "url",
        target: "chat",
        prefix: &["--url"],
        args: "<ws://…|wss://…|->",
        summary: "Read the chat from a wire of your own; - forgets it.",
    },
    Action {
        name: "hide",
        target: "hide",
        prefix: &[],
        args: "<line number>",
        summary: "Hide a line on remux's faces.",
    },
    Action {
        name: "delete",
        target: "delete",
        prefix: &[],
        args: "<line number>",
        summary: "Delete a line on its platform.",
    },
    Action {
        name: "say",
        target: "say",
        prefix: &[],
        args: "[--to <chat>] <words>",
        summary: "Say a line in the platform's chat.",
    },
];

const SCENE: &[Action] = &[
    Action {
        name: "create",
        target: "scene-create",
        prefix: &[],
        args: "<name>",
        summary: "Make a new, empty scene and switch to it.",
    },
    Action {
        name: "duplicate",
        target: "scene-duplicate",
        prefix: &[],
        args: "<name>",
        summary: "Copy the active scene, its layers and elements, and switch to the copy.",
    },
    Action {
        name: "switch",
        target: "scene-switch",
        prefix: &[],
        args: "<name>",
        summary: "Switch scene without stopping the live.",
    },
    Action {
        name: "list",
        target: "scenes",
        prefix: &[],
        args: "",
        summary: "List scenes and the active scene.",
    },
    Action {
        name: "status",
        target: "status",
        prefix: &[],
        args: "",
        summary: "Show the current scene and live status.",
    },
    Action {
        name: "layer",
        target: "layer",
        prefix: &[],
        args: "add|set screen|camera|window|image <id> <source> | add|set text|timer <id> <x> <y> <width> <height> <words|seconds> | hide|show|remove|shot <id> | move <id> <index> | transform <id> <x> <y> <width> <height> <degrees> | filter <id> <file.wgsl|off> | crop <id> <x> <y> <width> <height>|off | shape <id> circle|rectangle | mirror <id> on|off | position <id> <x> <y>|default",
        summary: "Compose captures, images, text and timers in one back-to-front order.",
    },
    Action {
        name: "filter",
        target: "shader",
        prefix: &[],
        args: "<file.wgsl|off>",
        summary: "Apply a WGSL filter to the composed scene.",
    },
    Action {
        name: "shot",
        target: "shot",
        prefix: &[],
        args: "[--out <file.jpg|->]",
        summary: "Read the composed scene preview.",
    },
    Action {
        name: "timer",
        target: "scene-timer",
        prefix: &[],
        args: "start|stop <id>",
        summary: "Start or stop a timer in the active scene.",
    },
    Action {
        name: "delete",
        target: "scene-delete",
        prefix: &[],
        args: "<name>",
        summary: "Delete an inactive scene.",
    },
];

pub(super) const GROUPS: &[Group] = &[
    Group {
        name: "scene",
        summary: "Named, ordered picture layouts",
        actions: SCENE,
    },
    Group {
        name: "audio",
        summary: "Microphone and sound mix",
        actions: AUDIO,
    },
    Group {
        name: "music",
        summary: "Playback and music mix",
        actions: MUSIC,
    },
    Group {
        name: "destination",
        summary: "Platforms and broadcast details",
        actions: DESTINATION,
    },
    Group {
        name: "chat",
        summary: "Read and moderate chat",
        actions: CHAT,
    },
];

/// The commands outside every group: the live's levers, what the engine is
/// and what this shell keeps by itself.
pub(super) const TOP: &[&str] = &[
    "status", "sources", "grants", "levels", "plan", "live", "stop", "record", "cut", "quit",
    "health", "wait", "events", "history", "log", "login", "logout", "config", "daemon", "bug",
    "schema",
];

pub(super) fn find(name: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|group| group.name == name)
}

pub(super) fn usage(group: &Group) -> String {
    let mut text = format!(
        "Usage: remux {} <command> [arguments] [--json]\n\n{}:\n",
        group.name, group.summary
    );
    for action in group.actions {
        text.push_str(&format!("  {:<18} {}\n", action.name, action.summary));
    }
    text.push_str(&format!(
        "\nUse `remux help {} <command>` for details.",
        group.name
    ));
    text
}

/// The public shell vocabulary. Flat words exist only inside the translator;
/// they cannot be entered on the command line any more.
pub fn normalize(words: &[String]) -> Result<Vec<String>, String> {
    let Some(first) = words.first() else {
        return Err(super::usage());
    };
    let Some(group) = find(first) else {
        return if TOP.contains(&first.as_str()) {
            Ok(words.to_vec())
        } else {
            Err(format!("{first} is not a command.\n{}", super::usage()))
        };
    };
    let Some(next) = words.get(1) else {
        return Err(usage(group));
    };
    if let Some(action) = group.actions.iter().find(|action| action.name == next) {
        let rest = &words[2..];
        if ((group.name == "music" && matches!(action.name, "play" | "off" | "next"))
            || (group.name == "audio" && action.name == "clips"))
            && !rest.is_empty()
        {
            return Err(format!(
                "remux {} {} takes no arguments",
                group.name, action.name
            ));
        }
        if group.name == "music" && action.name == "genre" && rest.is_empty() {
            return Err("music genre needs a name".into());
        }
        if group.name == "chat"
            && action.name == "read"
            && !(rest.is_empty()
                || matches!(rest, [one] if matches!(one.as_str(), "-f" | "--follow" | "follow")))
        {
            return Err("chat read takes -f, --follow or follow".into());
        }
        let mut expanded = vec![action.target.to_string()];
        expanded.extend(action.prefix.iter().map(|word| (*word).to_string()));
        expanded.extend_from_slice(&words[2..]);
        return Ok(expanded);
    }
    Err(format!(
        "{next} is not a {} command.\n{}",
        group.name,
        usage(group)
    ))
}
