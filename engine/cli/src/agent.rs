//! The TUI's agent panel: the person's own `claude` (Claude Code), run headless
//! on their login, with remux's guide and no tool but the `remux` command. remux
//! holds no credential and calls no model: without `claude` on the PATH the panel
//! says so and does nothing.
//!
//! What it reads is `claude -p --output-format stream-json`, one JSON object a
//! line, as Claude Code 2.1 writes it (measured): text arrives in
//! `stream_event` deltas, a command in full in an `assistant` message, its
//! output in a `user` message, and the turn ends with a `result` that names the
//! session to resume.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;

use serde_json::Value;

/// What the panel hears from a running request.
#[derive(Debug, Clone, PartialEq)]
pub enum Heard {
    /// What it is doing before it says anything: starting, thinking, running a
    /// command. Seconds of each pass with nothing written (measured: 15 s of a
    /// person's start-up hook, then as much thinking).
    Doing(&'static str),
    /// A piece of the answer, as it is written.
    Text(String),
    /// A command it runs.
    Ran(String),
    /// The first line of what a command answered, or why it was refused.
    Output { line: String, refused: bool },
    /// The turn is over: the session to go on with, and what it cost.
    Done { session: String, cost: f64 },
    /// It could not run, or ended badly.
    Failed(String),
}

/// The verbs that change what the audience sees or stop the engine: the
/// person's alone, as the CLI and the TUI give them.
const DENIED: [&str; 8] = [
    "live",
    "stop",
    "cut",
    "quit",
    "daemon",
    "login",
    "logout",
    "destination add",
];

/// The rules on top of the guide.
const RULES: &str = "You are the agent inside remux's TUI, a small panel on a \
person's screen while they run a live. Act through the remux CLI alone. Going live, \
stopping, the panic cut, quitting or restarting the engine, signing in and adding a \
destination are the person's: never run them, say which key or command does it. \
Build and change scenes off the air with --staged and say that t in the TUI takes \
them to the air. The person's own programs beside the engine (windows, cameras with \
effects, bots) are companions: remux companion list names each, its state and the \
words it takes; remux companion send <name> <words> gives it words and says back what \
it answered; remux companion log <name> is what it said. Answer in a few short lines, \
in the person's language.";

/// The argv of one request: its words, the session it continues, the rules and
/// the guide, the `remux` command allowed and the person's verbs refused.
pub fn args(prompt: &str, session: Option<&str>, guide: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        prompt,
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-mode",
        "dontAsk",
        "--allowedTools",
        "Bash(remux:*)",
        "--settings",
        r#"{"disableAllHooks":true}"#,
        "--strict-mcp-config",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    args.push("--disallowedTools".into());
    for verb in DENIED {
        args.push(format!("Bash(remux {verb}:*)"));
        args.push(format!("Bash(remux --json {verb}:*)"));
    }
    args.push("--append-system-prompt".into());
    args.push(format!("{RULES}\n\n{guide}"));
    if let Some(session) = session {
        args.push("--resume".into());
        args.push(session.into());
    }
    args
}

/// What one line of the stream says, if anything the panel shows.
pub fn heard(line: &str) -> Vec<Heard> {
    let Ok(event) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    let blocks = |event: &Value| -> Vec<Value> {
        event["message"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    match event["type"].as_str().unwrap_or("") {
        "system" => match event["subtype"].as_str().unwrap_or("") {
            "hook_started" => vec![Heard::Doing("starting")],
            "init" => vec![Heard::Doing("thinking")],
            _ => vec![],
        },
        "stream_event" if event["event"]["type"] == "content_block_start" => {
            match event["event"]["content_block"]["type"]
                .as_str()
                .unwrap_or("")
            {
                "thinking" => vec![Heard::Doing("thinking")],
                "tool_use" => vec![Heard::Doing("running a command")],
                _ => vec![],
            }
        }
        "stream_event" => match &event["event"]["delta"] {
            delta if delta["type"] == "text_delta" => delta["text"]
                .as_str()
                .map(|text| vec![Heard::Text(text.into())])
                .unwrap_or_default(),
            _ => vec![],
        },
        "assistant" => blocks(&event)
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .filter_map(|b| b["input"]["command"].as_str().map(|c| Heard::Ran(c.into())))
            .collect(),
        "user" => blocks(&event)
            .iter()
            .filter(|b| b["type"] == "tool_result")
            .map(|b| {
                let text = match &b["content"] {
                    Value::String(text) => text.clone(),
                    Value::Array(parts) => parts
                        .iter()
                        .filter_map(|p| p["text"].as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                    _ => String::new(),
                };
                // The beginning is enough to follow it: a command's JSON is one
                // long line.
                let first = text.lines().next().unwrap_or("");
                let line = match first.chars().count() > 80 {
                    true => format!("{}…", first.chars().take(80).collect::<String>()),
                    false => first.to_string(),
                };
                Heard::Output {
                    line,
                    refused: b["is_error"] == true,
                }
            })
            .collect(),
        "result" if event["subtype"] == "success" => vec![Heard::Done {
            session: event["session_id"].as_str().unwrap_or("").into(),
            cost: event["total_cost_usd"].as_f64().unwrap_or(0.0),
        }],
        "result" => vec![Heard::Failed(format!(
            "the agent stopped: {}",
            event["subtype"].as_str().unwrap_or("an error")
        ))],
        _ => vec![],
    }
}

/// Run one request; what it says goes to `out` as it comes. The child is the
/// caller's, to stop.
pub fn ask(prompt: &str, session: Option<&str>, out: Sender<Heard>) -> Result<Child, String> {
    let mut child = Command::new("claude")
        .args(args(prompt, session, crate::words::help::GUIDE))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|why| match why.kind() {
            std::io::ErrorKind::NotFound => {
                "no claude here: this panel runs your own Claude Code (claude on the PATH)".into()
            }
            _ => format!("claude: {why}"),
        })?;
    let stdout = child.stdout.take().ok_or("claude: no output")?;
    std::thread::spawn(move || {
        let mut ended = false;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            for one in heard(&line) {
                ended |= matches!(one, Heard::Done { .. } | Heard::Failed(_));
                if out.send(one).is_err() {
                    return;
                }
            }
        }
        if !ended {
            let _ = out.send(Heard::Failed("the agent stopped".into()));
        }
    });
    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lines as Claude Code 2.1.288 wrote them, trimmed of what the panel ignores.
    const TEXT: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"No, `"}},"session_id":"s1"}"#;
    const RAN: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"remux status","description":"Show remux status"}}]},"session_id":"s1"}"#;
    const OUTPUT: &str = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t1","type":"tool_result","content":"off air, layer bg: Image\nsecond line"}]},"session_id":"s1"}"#;
    const REFUSED: &str = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t2","type":"tool_result","content":"Permission to use Bash with command remux status has been denied.","is_error":true}]}}"#;
    const DONE: &str = r#"{"type":"result","subtype":"success","session_id":"s1","total_cost_usd":0.2061,"result":"No."}"#;
    const PARTIAL_TOOL: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}}"#;

    #[test]
    fn each_line_is_heard_as_the_panel_shows_it() {
        assert_eq!(heard(TEXT), [Heard::Text("No, `".into())]);
        assert_eq!(heard(RAN), [Heard::Ran("remux status".into())]);
        assert_eq!(
            heard(OUTPUT),
            [Heard::Output {
                line: "off air, layer bg: Image".into(),
                refused: false
            }]
        );
        assert!(matches!(
            heard(REFUSED).as_slice(),
            [Heard::Output { refused: true, .. }]
        ));
        assert_eq!(
            heard(DONE),
            [Heard::Done {
                session: "s1".into(),
                cost: 0.2061
            }]
        );
        assert!(
            heard(PARTIAL_TOOL).is_empty(),
            "a command is shown once, whole"
        );
        assert!(heard("not json").is_empty());
    }

    #[test]
    fn what_it_is_doing_is_heard_before_it_says_anything() {
        let hook = r#"{"type":"system","subtype":"hook_started"}"#;
        let init = r#"{"type":"system","subtype":"init","session_id":"s1"}"#;
        let thinking = r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}}"#;
        let tool = r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t","name":"Bash","input":{}}}}"#;
        assert_eq!(heard(hook), [Heard::Doing("starting")]);
        assert_eq!(heard(init), [Heard::Doing("thinking")]);
        assert_eq!(heard(thinking), [Heard::Doing("thinking")]);
        assert_eq!(heard(tool), [Heard::Doing("running a command")]);
    }

    #[test]
    fn a_long_answer_from_a_command_is_its_beginning() {
        let long = format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","content":"{}"}}]}}}}"#,
            "x".repeat(500)
        );
        let heard = heard(&long);
        let [Heard::Output { line, .. }] = heard.as_slice() else {
            panic!("an output: {heard:?}")
        };
        assert_eq!(
            line.chars().count(),
            81,
            "eighty characters and an ellipsis"
        );
        assert!(line.ends_with('…'));
    }

    #[test]
    fn only_remux_is_allowed_and_the_persons_verbs_are_refused() {
        let args = args("make a scene", Some("s1"), "the guide");
        let after = |flag: &str| args[args.iter().position(|a| a == flag).expect(flag) + 1].clone();
        assert_eq!(after("--allowedTools"), "Bash(remux:*)");
        // The person's hooks and MCP servers cost seconds a request (measured: 7 s
        // to ready with them, 1 s without) and nothing here needs them.
        assert_eq!(after("--settings"), r#"{"disableAllHooks":true}"#);
        assert!(args.contains(&"--strict-mcp-config".to_string()));
        assert_eq!(after("--permission-mode"), "dontAsk");
        for verb in ["live", "stop", "cut", "quit", "daemon"] {
            assert!(args.contains(&format!("Bash(remux {verb}:*)")), "{verb}");
            assert!(
                args.contains(&format!("Bash(remux --json {verb}:*)")),
                "{verb}"
            );
        }
        assert!(after("--append-system-prompt").ends_with("the guide"));
        assert_eq!(after("--resume"), "s1");
        assert_eq!(after("-p"), "make a scene");
    }
}
