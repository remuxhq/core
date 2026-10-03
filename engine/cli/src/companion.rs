//! `remux companion`: the operator's own programs beside the engine, started,
//! stopped and watched here, by the shell. The list and its rules are the
//! domain's (`remuxd_domain::companions`); this is the processes.

use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use remuxd_domain::companions::State;
use remuxd_domain::companions::{self, Companion, Record};
use remuxd_domain::config::expand;

/// What `remux companion` was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    List,
    Start(String),
    Stop(String),
    Log(String),
    /// A line of words to a companion's standard input.
    Send(String, String),
}

impl Verb {
    pub fn parse(words: &[String]) -> Result<Self, String> {
        match words {
            [] => Ok(Self::List),
            [list] if list == "list" => Ok(Self::List),
            [verb, name] if verb == "start" => Ok(Self::Start(name.clone())),
            [verb, name] if verb == "stop" => Ok(Self::Stop(name.clone())),
            [verb, name] if verb == "log" => Ok(Self::Log(name.clone())),
            [verb, name, words @ ..] if verb == "send" && !words.is_empty() => {
                Ok(Self::Send(name.clone(), words.join(" ")))
            }
            _ => Err("companion: list, start|stop|log <name>, or send <name> <words>".into()),
        }
    }
}

/// Runs the verb and says what came of it; a refusal exits non-zero.
pub fn run(verb: &Verb) -> Result<String, String> {
    match verb {
        Verb::List => Ok(states()?
            .into_iter()
            .map(|(name, state)| match state {
                State::Up(record) => format!("{name:<16} up    pid {}", record.pid),
                State::Fell(_) => format!("{name:<16} fell  (remux companion log {name})"),
                State::Down => format!("{name:<16} down"),
            })
            .collect::<Vec<_>>()
            .join("\n")),
        Verb::Start(name) => start(name),
        Verb::Stop(name) => stop(name),
        Verb::Log(name) => log(name, 40),
        Verb::Send(name, words) => send(name, words),
    }
}

/// The companions file, read and checked.
pub fn listed() -> Result<Vec<Companion>, String> {
    let path = companions::path().ok_or(
        "no companions file: name one with REMUX_COMPANIONS or `[companions] file` in config.toml",
    )?;
    let text = std::fs::read_to_string(&path)
        .map_err(|why| format!("the companions file {}: {why}", path.display()))?;
    companions::parse(&text)
}

fn named(name: &str) -> Result<Companion, String> {
    listed()?
        .into_iter()
        .find(|companion| companion.name == name)
        .ok_or_else(|| format!("no companion {name} in the companions file"))
}

/// The pipe a companion that takes input reads as its standard input.
fn input_file(name: &str) -> PathBuf {
    companions::dir().join(format!("{name}.in"))
}

fn pid_file(name: &str) -> PathBuf {
    companions::dir().join(format!("{name}.pid"))
}

/// Where a companion's output goes, appended.
pub fn log_file(name: &str) -> PathBuf {
    companions::dir().join(format!("{name}.log"))
}

fn record(name: &str) -> Option<Record> {
    std::fs::read_to_string(pid_file(name))
        .ok()
        .and_then(|text| Record::read(&text))
}

/// Whether a process lives: signal zero asks without sending.
fn alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 checks the pid and delivers nothing.
    let answered = unsafe { libc::kill(pid, 0) } == 0;
    answered || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn state_of(name: &str) -> State {
    let record = record(name);
    State::of(record, record.is_some_and(|r| alive(r.pid)))
}

/// One companion's state.
#[cfg(test)]
fn state(name: &str) -> Result<State, String> {
    named(name)?;
    Ok(state_of(name))
}

/// Every companion and its state, in the file's order.
pub fn states() -> Result<Vec<(String, State)>, String> {
    Ok(listed()?
        .into_iter()
        .map(|companion| {
            let state = state_of(&companion.name);
            (companion.name, state)
        })
        .collect())
}

/// Starts a companion in a process group of its own, its output appended to
/// its log beside the socket, its environment read from its file (0600, or
/// it does not start). A companion already up is left alone.
pub fn start(name: &str) -> Result<String, String> {
    let companion = named(name)?;
    if let State::Up(_) = state_of(name) {
        return Ok(format!("{name} already up"));
    }
    let environment = match &companion.env_file {
        Some(file) => {
            let path = expand(file);
            let mode = std::fs::metadata(&path)
                .map_err(|why| format!("{name}: its env file {}: {why}", path.display()))?
                .permissions()
                .mode();
            if !companions::private(mode) {
                return Err(format!(
                    "{name}: its env file {} can be read by others: chmod 0600 it",
                    path.display()
                ));
            }
            let text = std::fs::read_to_string(&path)
                .map_err(|why| format!("{name}: its env file {}: {why}", path.display()))?;
            companions::environment(&text).map_err(|why| format!("{name}: its env file: {why}"))?
        }
        None => Vec::new(),
    };
    let cwd = companion.cwd.as_deref().map(expand);
    // A relative program is the folder's: `./thing` runs the one beside it.
    let program = match &cwd {
        Some(cwd) if companion.run[0].contains('/') && !companion.run[0].starts_with('/') => {
            cwd.join(&companion.run[0])
        }
        _ => PathBuf::from(&companion.run[0]),
    };
    let dir = companions::dir();
    std::fs::create_dir_all(&dir).map_err(|why| format!("{}: {why}", dir.display()))?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log_file(name))
        .map_err(|why| format!("{name}: its log: {why}"))?;
    // A companion that takes input reads a pipe of its own, opened for reading
    // and writing so that it never sees its end: words come when they are sent.
    let input = if companion.input {
        let pipe = input_file(name);
        let _ = std::fs::remove_file(&pipe);
        let path = std::ffi::CString::new(pipe.to_string_lossy().as_bytes())
            .map_err(|_| format!("{name}: its input's path"))?;
        // SAFETY: a path made above, a mode of the owner's alone.
        if unsafe { libc::mkfifo(path.as_ptr(), 0o600) } != 0 {
            return Err(format!(
                "{name}: its input: {}",
                std::io::Error::last_os_error()
            ));
        }
        let pipe = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe)
            .map_err(|why| format!("{name}: its input: {why}"))?;
        Stdio::from(pipe)
    } else {
        Stdio::null()
    };
    let mut command = Command::new(&program);
    command
        .args(&companion.run[1..])
        .envs(environment)
        .stdin(input)
        .stdout(log.try_clone().map_err(|why| why.to_string())?)
        .stderr(log)
        .process_group(0);
    if let Some(cwd) = &cwd {
        command.current_dir(cwd);
    }
    let mut child = command
        .spawn()
        .map_err(|why| format!("{name}: {} would not start: {why}", program.display()))?;
    let record = Record {
        pid: child.id(),
        since: now(),
    };
    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(pid_file(name))
        .and_then(|mut file| std::io::Write::write_all(&mut file, record.write().as_bytes()))
        .map_err(|why| format!("{name}: its pid: {why}"))?;
    // Waited on while this process lives, so one that dies is gone rather
    // than a zombie that answers as alive.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(format!("{name} started (pid {})", record.pid))
}

/// Stops a companion's whole process group: asked first, then made to, three
/// seconds on. One that fell is cleared.
pub fn stop(name: &str) -> Result<String, String> {
    named(name)?;
    let Some(record) = record(name) else {
        return Ok(format!("{name} is not running"));
    };
    let said = if alive(record.pid) {
        signal(record.pid, libc::SIGTERM);
        let until = Instant::now() + Duration::from_secs(3);
        while alive(record.pid) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        if alive(record.pid) {
            signal(record.pid, libc::SIGKILL);
        }
        format!("{name} stopped")
    } else {
        format!("{name} had fallen; cleared")
    };
    let _ = std::fs::remove_file(pid_file(name));
    Ok(said)
}

/// The group the companion leads, else the companion alone.
fn signal(pid: u32, signal: i32) {
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };
    // SAFETY: a signal to a pid this shell started and recorded.
    unsafe {
        if libc::kill(-pid, signal) != 0 {
            libc::kill(pid, signal);
        }
    }
}

/// A line of words to a companion that takes input, as it would be typed on
/// its standard input; control characters are taken out.
pub fn send(name: &str, words: &str) -> Result<String, String> {
    let companion = named(name)?;
    if !companion.input {
        return Err(format!(
            "{name} takes no words: set input = true for it in the companions file"
        ));
    }
    if !matches!(state_of(name), State::Up(_)) {
        return Err(format!("{name} is not running"));
    }
    let line: String = words.chars().filter(|c| !c.is_control()).collect();
    let mut pipe = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(input_file(name))
        .map_err(|why| format!("{name}: its input: {why}"))?;
    std::io::Write::write_all(&mut pipe, format!("{line}\n").as_bytes())
        .map_err(|why| format!("{name}: its input: {why}"))?;
    Ok(format!("{name}: {line}"))
}

/// The last lines of a companion's log, made plain for a terminal.
pub fn log(name: &str, lines: usize) -> Result<String, String> {
    named(name)?;
    let text = std::fs::read_to_string(log_file(name)).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    Ok(all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|line| crate::words::plain(line))
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A companions file in a folder of its own, and the socket beside it, so
    /// the pids and logs land there too. nextest runs each test in its own
    /// process: the environment is this test's.
    fn place(list: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("remux-companion-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        std::fs::write(dir.join("companions.toml"), list).expect("the list");
        std::env::set_var("REMUX_COMPANIONS", dir.join("companions.toml"));
        std::env::set_var("REMUXD_SOCKET", dir.join("remuxd.sock"));
        dir
    }

    #[test]
    fn the_verbs_are_list_start_stop_and_log_each_by_name() {
        let w = |words: &[&str]| words.iter().map(|w| (*w).to_string()).collect::<Vec<_>>();
        assert_eq!(Verb::parse(&w(&[])), Ok(Verb::List));
        assert_eq!(Verb::parse(&w(&["list"])), Ok(Verb::List));
        assert_eq!(
            Verb::parse(&w(&["start", "a"])),
            Ok(Verb::Start("a".into()))
        );
        assert_eq!(Verb::parse(&w(&["stop", "a"])), Ok(Verb::Stop("a".into())));
        assert_eq!(Verb::parse(&w(&["log", "a"])), Ok(Verb::Log("a".into())));
        assert_eq!(
            Verb::parse(&w(&["send", "a", "size", "480"])),
            Ok(Verb::Send("a".into(), "size 480".into()))
        );
        assert!(Verb::parse(&w(&["start"])).is_err());
        assert!(Verb::parse(&w(&["launch", "a"])).is_err());
    }

    #[test]
    fn words_sent_to_a_companion_that_takes_input_reach_its_standard_input() {
        place("[[companion]]\nname = \"echo\"\nrun = [\"cat\"]\ninput = true\n");
        start("echo").expect("it starts");
        send("echo", "dvd").expect("it takes the words");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !std::fs::read_to_string(log_file("echo"))
            .unwrap_or_default()
            .contains("dvd")
        {
            assert!(
                std::time::Instant::now() < until,
                "the words never reached it"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        stop("echo").expect("it stops");
    }

    #[test]
    fn a_companion_without_input_or_down_takes_no_words() {
        place("[[companion]]\nname = \"deaf\"\nrun = [\"sleep\", \"30\"]\n[[companion]]\nname = \"idle\"\nrun = [\"cat\"]\ninput = true\n");
        start("deaf").expect("it starts");
        assert!(send("deaf", "x").unwrap_err().contains("input = true"));
        stop("deaf").expect("it stops");
        assert!(send("idle", "x").unwrap_err().contains("not running"));
    }

    const SLEEPER: &str = "[[companion]]\nname = \"sleeper\"\nrun = [\"sleep\", \"30\"]\n";

    #[test]
    fn a_companion_starts_once_runs_beside_and_stops() {
        place(SLEEPER);
        assert!(matches!(state("sleeper"), Ok(State::Down)));
        let started = start("sleeper").expect("it starts");
        assert!(started.contains("started"), "{started}");
        assert!(matches!(state("sleeper"), Ok(State::Up(_))));
        let again = start("sleeper").expect("a second start is no error");
        assert!(again.contains("already up"), "{again}");
        stop("sleeper").expect("it stops");
        assert!(matches!(state("sleeper"), Ok(State::Down)));
    }

    #[test]
    fn a_companion_that_dies_by_itself_is_said_to_have_fallen() {
        place("[[companion]]\nname = \"brief\"\nrun = [\"true\"]\n");
        start("brief").expect("it starts");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !matches!(state("brief"), Ok(State::Fell(_))) {
            assert!(std::time::Instant::now() < until, "it never fell");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        stop("brief").expect("stopping a fallen one clears it");
        assert!(matches!(state("brief"), Ok(State::Down)));
    }

    #[test]
    fn an_environment_file_others_can_read_is_refused_and_nothing_starts() {
        let dir = place(
            "[[companion]]\nname = \"keyed\"\nrun = [\"sleep\", \"30\"]\nenv_file = \"ENV\"\n",
        );
        let env = dir.join("keyed.env");
        std::fs::write(&env, "KEY=value\n").expect("an env file");
        std::fs::set_permissions(&env, std::os::unix::fs::PermissionsExt::from_mode(0o644))
            .expect("its mode");
        let list = std::fs::read_to_string(dir.join("companions.toml"))
            .expect("the list")
            .replace("ENV", env.to_str().expect("a path"));
        std::fs::write(dir.join("companions.toml"), list).expect("the list");
        let refused = start("keyed").unwrap_err();
        assert!(refused.contains("0600"), "{refused}");
        assert!(matches!(state("keyed"), Ok(State::Down)));
    }

    #[test]
    fn an_unknown_name_and_a_missing_list_say_so() {
        place(SLEEPER);
        assert!(start("nobody").unwrap_err().contains("no companion nobody"));
        std::env::remove_var("REMUX_COMPANIONS");
        std::env::set_var("REMUX_CONFIG", "/nonexistent/config.toml");
        assert!(listed().unwrap_err().contains("companions file"));
    }
}
