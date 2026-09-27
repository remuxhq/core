//! What the engine said, kept where somebody can read it.
//!
//! A daemon that dies with no file, no console and nothing to look at is a
//! shrug. A daemon that a person starts from a
//! makefile and then forgets about has to leave a trail, or every crash is a
//! shrug.
//!
//! Everything it writes to stderr goes here as well as to the terminal, and a
//! panic writes its reason before the process goes. The file is truncated at
//! each start rather than appended: what anybody wants after a crash is the run
//! that crashed, not a year of them, and a log that grows without bound is its
//! own outage.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// `remuxd.log` beside the socket, in the OS's state folder.
#[must_use]
pub fn path() -> PathBuf {
    if let Some(said) = std::env::var_os("REMUXD_LOG") {
        return PathBuf::from(said);
    }
    crate::socket::default_path().with_file_name("remuxd.log")
}

static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Start writing, and say where. Failing to open it is not a reason to refuse
/// to run: the engine works fine without a log, it is just harder to ask
/// afterwards what happened.
pub fn start() {
    let path = path();
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    // The previous run is kept beside this one as `remuxd.log.1`: the panel
    // starts the next engine the second one leaves, and the line saying why
    // it left ("quitting: ...") was truncated away before anybody read it.
    let _ = std::fs::rename(&path, path.with_extension("log.1"));
    match std::fs::File::create(&path) {
        Ok(file) => {
            *FILE.lock().expect("log") = Some(file);
            note(&format!("remuxd {} starting", env!("CARGO_PKG_VERSION")));
        }
        Err(why) => eprintln!("remuxd: no log at {}: {why}", path.display()),
    }

    // A panic is the one thing that has to reach the file, because it is the
    // one thing nobody is watching the terminal for.
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        note(&format!("panic: {panic}"));
        before(panic);
    }));
}

/// One line, to the terminal and to the file.
///
/// Stamped, because the first question after a crash is what else was
/// happening at the time.
pub fn note(line: &str) {
    let stamped = format!(
        "{} {line}",
        crate::journal::clock_of(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs() as i64)
        )
    );
    eprintln!("{stamped}");
    if let Ok(mut file) = FILE.lock() {
        if let Some(file) = file.as_mut() {
            let _ = writeln!(file, "{stamped}");
            let _ = file.flush();
        }
    }
}

/// The last few lines, for a panel with nothing else to show.
#[must_use]
pub fn tail(lines: usize) -> Vec<String> {
    let said = std::fs::read_to_string(path()).unwrap_or_default();
    said.lines()
        .rev()
        .take(lines)
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}
