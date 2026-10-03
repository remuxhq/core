//! Companions: the operator's own programs that run beside the engine, which
//! a face starts, stops and watches without knowing what any of them is. To
//! remux a companion is a name, an argv and a state.
//!
//! The list is a file of the operator's own, holding no secret:
//!
//! ```toml
//! [[companion]]
//! name     = "first"
//! run      = ["./first", "--flag"]       # argv, never a shell
//! cwd      = "~/somewhere"
//! env_file = "~/.config/remux/first.env" # optional, 0600, read when it starts
//! ```
//!
//! `REMUX_COMPANIONS` names it, else `[companions] file` in `config.toml`.
//! The engine never reads it: the faces run companions, and the socket keeps
//! having no verb that executes anything.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One companion: what it is called, the argv that runs it, the folder it
/// runs in, and the file its environment is read from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Companion {
    pub name: String,
    pub run: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env_file: Option<String>,
    /// Whether it reads words on its standard input: `remux companion send`
    /// writes a line there.
    #[serde(default)]
    pub input: bool,
}

#[derive(Deserialize)]
struct List {
    #[serde(default)]
    companion: Vec<Companion>,
}

/// The list, every companion checked: a name a file can be called (it names
/// the pid and the log), something to run, and no name twice.
pub fn parse(text: &str) -> Result<Vec<Companion>, String> {
    let list: List = toml::from_str(text).map_err(|why| format!("the companions file: {why}"))?;
    let mut seen = Vec::new();
    for companion in &list.companion {
        let name = &companion.name;
        let fits = !name.is_empty()
            && name.len() <= 32
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !fits {
            return Err(format!(
                "companion {name:?}: a name is letters, digits, - and _, up to 32"
            ));
        }
        if companion.run.is_empty() || companion.run[0].is_empty() {
            return Err(format!("companion {name}: run names nothing to run"));
        }
        if seen.contains(&name) {
            return Err(format!("companion {name} is named twice"));
        }
        seen.push(name);
    }
    Ok(list.companion)
}

/// `KEY=value` lines, blank lines and `#` comments skipped. A line that is
/// not one is named by its number alone: its value may be a secret.
pub fn environment(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut read = Vec::new();
    for (at, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let key = line.split_once('=').map(|(key, value)| (key.trim(), value));
        let named = key.is_some_and(|(key, _)| {
            key.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        match key {
            Some((key, value)) if named => read.push((key.to_string(), value.to_string())),
            _ => return Err(format!("line {} is not KEY=value", at + 1)),
        }
    }
    Ok(read)
}

/// Whether a file's mode keeps it to its owner: an environment file holds
/// keys, and one the group or the world can read is not read.
pub fn private(mode: u32) -> bool {
    mode & 0o077 == 0
}

/// A started companion: its pid, which is also its process group, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub pid: u32,
    pub since: i64,
}

impl Record {
    pub fn write(&self) -> String {
        format!("{} {}\n", self.pid, self.since)
    }

    pub fn read(text: &str) -> Option<Self> {
        let mut words = text.split_whitespace();
        Some(Self {
            pid: words.next()?.parse().ok()?,
            since: words.next()?.parse().ok()?,
        })
    }
}

/// What a face says of a companion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Started, and its process lives.
    Up(Record),
    /// Started, and its process is gone without being stopped.
    Fell(Record),
    /// Not started, or stopped.
    Down,
}

impl State {
    pub fn of(record: Option<Record>, alive: bool) -> Self {
        match (record, alive) {
            (Some(record), true) => Self::Up(record),
            (Some(record), false) => Self::Fell(record),
            (None, _) => Self::Down,
        }
    }
}

/// The companions file: `REMUX_COMPANIONS`, else `[companions] file`, else
/// none.
pub fn path() -> Option<PathBuf> {
    std::env::var("REMUX_COMPANIONS")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| crate::config::read(&crate::config::path()).companions.file)
        .map(|file| crate::config::expand(&file))
}

/// Where each companion's pid and log are kept: beside the socket.
pub fn dir() -> PathBuf {
    let socket = crate::socket::default_path();
    socket
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("companions")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = r#"
[[companion]]
name = "first"
run = ["./first", "--flag"]
cwd = "~/somewhere"
env_file = "~/.config/remux/first.env"

[[companion]]
name = "second"
run = ["./second"]
"#;

    #[test]
    fn a_list_reads_each_companion_as_a_name_an_argv_a_folder_and_an_environment() {
        let list = parse(TWO).expect("two companions");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "first");
        assert_eq!(list[0].run, ["./first", "--flag"]);
        assert_eq!(list[0].cwd.as_deref(), Some("~/somewhere"));
        assert_eq!(
            list[0].env_file.as_deref(),
            Some("~/.config/remux/first.env")
        );
        assert_eq!(list[1].cwd, None);
        assert_eq!(list[1].env_file, None);
    }

    #[test]
    fn a_companion_with_nothing_to_run_is_refused_by_name() {
        let refused = parse("[[companion]]\nname = \"idle\"\nrun = []\n").unwrap_err();
        assert!(
            refused.contains("idle") && refused.contains("run"),
            "{refused}"
        );
    }

    #[test]
    fn a_name_is_letters_digits_dashes_and_underscores() {
        assert!(parse("[[companion]]\nname = \"a b\"\nrun = [\"x\"]\n").is_err());
        assert!(parse("[[companion]]\nname = \"../x\"\nrun = [\"x\"]\n").is_err());
        assert!(parse("[[companion]]\nname = \"\"\nrun = [\"x\"]\n").is_err());
        assert!(parse("[[companion]]\nname = \"chat-2_b\"\nrun = [\"x\"]\n").is_ok());
    }

    #[test]
    fn two_companions_of_one_name_are_refused() {
        let twice = "[[companion]]\nname = \"a\"\nrun = [\"x\"]\n[[companion]]\nname = \"a\"\nrun = [\"y\"]\n";
        assert!(parse(twice).unwrap_err().contains("twice"));
    }

    #[test]
    fn a_list_that_does_not_parse_says_so() {
        assert!(parse("[[companion]\n").is_err());
    }

    #[test]
    fn an_environment_file_is_key_value_lines_and_a_bad_line_is_named_by_number_alone() {
        let read =
            environment("# a comment\n\nFIRST_KEY=one=two\nOTHER=\n").expect("an environment");
        assert_eq!(
            read,
            [
                ("FIRST_KEY".to_string(), "one=two".to_string()),
                ("OTHER".to_string(), String::new())
            ]
        );
        let bad = environment("GOOD=1\nnot a line secret-value\n").unwrap_err();
        assert!(bad.contains("line 2"), "{bad}");
        assert!(
            !bad.contains("secret-value"),
            "the value never shows: {bad}"
        );
    }

    #[test]
    fn an_environment_file_others_can_read_is_not_read() {
        assert!(private(0o100600));
        assert!(private(0o100400));
        assert!(!private(0o100644));
        assert!(!private(0o100640));
    }

    #[test]
    fn a_record_is_the_pid_and_when_it_started() {
        let record = Record {
            pid: 4242,
            since: 1_790_000_000,
        };
        assert_eq!(Record::read(&record.write()), Some(record));
        assert_eq!(Record::read("garbage"), None);
    }

    #[test]
    fn a_companion_is_up_while_its_process_lives_fell_when_it_died_and_down_with_no_record() {
        let record = Record {
            pid: 4242,
            since: 1_790_000_000,
        };
        assert_eq!(State::of(Some(record), true), State::Up(record));
        assert_eq!(State::of(Some(record), false), State::Fell(record));
        assert_eq!(State::of(None, false), State::Down);
    }
}
