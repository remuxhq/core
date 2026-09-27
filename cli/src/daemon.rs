//! `remux daemon start|stop|restart|status|log|path`: the engine as a
//! service of the session. Which service manager, and the words for it, is
//! `remuxd_domain::os::OS.service`; this runs them. Every verb is safe to
//! repeat; `stop` and `restart` refuse on air.

use std::path::{Path, PathBuf};
use std::process::{Command as Shell, Stdio};
use std::time::{Duration, Instant};

use remuxd_domain::daemon::{self, Loaded, Verb};
use remuxd_domain::os::OS;
use remuxd_domain::protocol::{Command, Reply};

/// The engine: `REMUXD_BIN`, else `remuxd` beside this binary. The link on
/// the PATH is followed, the folders are not: macOS ties a Screen Recording
/// grant to the path, and `~/.local/share/remux/current/bin/remuxd` is the
/// same path after an upgrade where `…/0.2.0/bin/remuxd` is a stranger.
fn binary() -> Result<PathBuf, String> {
    if let Some(said) = std::env::var_os("REMUXD_BIN") {
        return Ok(PathBuf::from(said));
    }
    // As called, not as the kernel resolves it: Linux's /proc/self/exe
    // walks through `current`, and the point is to keep that link.
    let called = std::env::args().next().unwrap_or_default();
    let mut me = if called.contains('/') {
        PathBuf::from(&called)
    } else {
        std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .map(|dir| dir.join(&called))
                    .find(|p| p.is_file())
            })
            .or_else(|| std::env::current_exe().ok())
            .ok_or("cannot tell where this binary is")?
    };
    while let Ok(to) = std::fs::read_link(&me) {
        me = if to.is_absolute() {
            to
        } else {
            me.parent().map(|dir| dir.join(to)).unwrap_or_default()
        };
    }
    me.parent()
        .map(|dir| dir.join("remuxd"))
        .filter(|p| p.is_file())
        .ok_or_else(|| "remuxd is not beside this binary; set REMUXD_BIN".to_string())
}

fn uid() -> u32 {
    Shell::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|out| String::from_utf8_lossy(&out.stdout).trim().parse().ok())
        .unwrap_or(501)
}

/// One command from the table: its stdout on success, its stderr otherwise.
fn run(words: &[String]) -> Result<String, String> {
    let (program, args) = words.split_first().ok_or("an empty command")?;
    let out = Shell::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn run_all(commands: Vec<Vec<String>>) -> Result<(), String> {
    for words in commands {
        run(&words)?;
    }
    Ok(())
}

fn loaded() -> Loaded {
    (OS.service.loaded)(run(&(OS.service.show)(uid())).ok().as_deref())
}

fn until(what: &str, budget: Duration, mut done: impl FnMut() -> bool) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < budget {
        if done() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("{what} did not happen in {} s", budget.as_secs()))
}

fn on_air(socket: &Path, ask: &dyn Fn(&Path, &Command) -> Result<Reply, String>) -> bool {
    matches!(ask(socket, &Command::Status), Ok(Reply::Status(status)) if status.on_air)
}

fn start(
    socket: &Path,
    ask: &dyn Fn(&Path, &Command) -> Result<Reply, String>,
) -> Result<String, String> {
    let binary = binary()?;
    let support = socket.parent().map(Path::to_path_buf).unwrap_or_default();
    std::fs::create_dir_all(&support).map_err(|e| format!("{}: {e}", support.display()))?;
    let file = daemon::service_file();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let wanted = (OS.service.text)(
        &binary,
        &support,
        &std::env::var("PATH").unwrap_or_default(),
        &daemon::output_path(),
    );
    let changed = std::fs::read_to_string(&file).ok().as_deref() != Some(wanted.as_str());
    if changed {
        if loaded() != Loaded::Not {
            let _ = run_all((OS.service.unload)(uid()));
        }
        std::fs::write(&file, &wanted).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    match loaded() {
        Loaded::Running { pid } if !changed => {
            return Ok(format!(
                "remuxd is running (pid {pid}); remux daemon restart to start it over"
            ));
        }
        Loaded::Not => run_all((OS.service.load)(uid(), &file))?,
        _ => run_all((OS.service.kick)(uid()))?,
    }
    until("the engine answering", Duration::from_secs(15), || {
        ask(socket, &Command::Status).is_ok()
    })
    .map_err(|why| format!("{why}; remux daemon log"))?;
    let pid = match loaded() {
        Loaded::Running { pid } => pid.to_string(),
        _ => "?".into(),
    };
    Ok(format!("remuxd started (pid {pid}), {}", socket.display()))
}

fn stop(
    socket: &Path,
    force: bool,
    ask: &dyn Fn(&Path, &Command) -> Result<Reply, String>,
) -> Result<String, String> {
    if !force && on_air(socket, ask) {
        return Err("on air: remux stop first, or remux daemon stop --force".into());
    }
    if loaded() == Loaded::Not {
        return Ok(format!("remuxd is not running under {}", OS.service.name));
    }
    run_all((OS.service.unload)(uid()))?;
    until("the engine going away", Duration::from_secs(10), || {
        ask(socket, &Command::Status).is_err()
    })?;
    Ok("remuxd stopped".into())
}

fn status(socket: &Path, ask: &dyn Fn(&Path, &Command) -> Result<Reply, String>) -> String {
    let agent = match loaded() {
        Loaded::Not => "not loaded (remux daemon start)".to_string(),
        Loaded::Idle => "loaded, not running (remux daemon start, or remux daemon log)".to_string(),
        Loaded::Running { pid } => format!("running under {} (pid {pid})", OS.service.name),
    };
    let engine = match ask(socket, &Command::Status) {
        Ok(Reply::Status(status)) if status.on_air => "answering, ON AIR".to_string(),
        Ok(_) => "answering, off air".to_string(),
        Err(_) => "not answering".to_string(),
    };
    format!(
        "service: {agent}\nengine:  {engine} on {}",
        socket.display()
    )
}

fn log(follow: bool) -> ! {
    let files = [remuxd_domain::log::path(), daemon::output_path()];
    let mut tail = Shell::new("tail");
    tail.arg("-n").arg("40");
    if follow {
        tail.arg("-F");
    }
    for file in &files {
        if file.is_file() {
            tail.arg(file);
        }
    }
    if !files.iter().any(|f| f.is_file()) {
        eprintln!("no log yet at {}", files[0].display());
        std::process::exit(1);
    }
    let status = tail.stdin(Stdio::null()).status();
    std::process::exit(status.map(|s| s.code().unwrap_or(1)).unwrap_or(1));
}

fn paths(socket: &Path) -> String {
    format!(
        "engine  {}\nservice {}\nsocket  {}\nlog     {}\noutput  {}\nconfig  {}",
        binary().map_or_else(|why| why, |p| p.display().to_string()),
        daemon::service_file().display(),
        socket.display(),
        remuxd_domain::log::path().display(),
        daemon::output_path().display(),
        remuxd_domain::config::path().display(),
    )
}

/// `remux daemon <verb>`, then exit with what happened.
pub fn run_verb(
    verb: &Verb,
    socket: &Path,
    ask: &dyn Fn(&Path, &Command) -> Result<Reply, String>,
) -> ! {
    let outcome = match verb {
        Verb::Start => start(socket, ask),
        Verb::Stop { force } => stop(socket, *force, ask),
        Verb::Restart { force } => stop(socket, *force, ask)
            .and_then(|said| start(socket, ask).map(|then| format!("{said}\n{then}"))),
        Verb::Status => Ok(status(socket, ask)),
        Verb::Log { follow } => log(*follow),
        Verb::Path => Ok(paths(socket)),
    };
    match outcome {
        Ok(said) => {
            println!("{said}");
            std::process::exit(0);
        }
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(1);
        }
    }
}
