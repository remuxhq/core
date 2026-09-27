//! The transport. It moves lines; it decides nothing.
//!
//! Every decision belongs to [`crate::engine::Engine`], which is why this file
//! is short enough to read in one sitting and why almost none of the daemon's
//! behaviour depends on a socket being up to be tested.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

use remuxd_domain::engine::Engine;
use remuxd_domain::protocol::{decode, encode, Command, Reply};

use crate::wire::Shared;

/// Where the socket lives when nobody says otherwise. Inside the app's own
/// support directory, because a daemon that ships inside a signed bundle has
/// no business writing to a shared temp path.
pub fn default_socket_path() -> PathBuf {
    remuxd_domain::socket::default_path()
}

/// Bind the socket, replacing a stale one from a process that is gone.
///
/// A unix socket file outlives the process that made it, so a crash leaves a
/// path that looks bound and refuses every connection. Connecting to it first
/// is the only way to tell "somebody is listening" from "somebody died here":
/// if the connect succeeds, an engine really is running and this one should
/// not steal its address.
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                format!("an engine is already listening on {}", path.display()),
            ));
        }
        std::fs::remove_file(path)?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    UnixListener::bind(path)
}

/// Accept clients until the process is told to stop, handing each one its own
/// thread. `quit` fires when a client asks the engine to quit; the caller
/// decides what to do about it, because deciding and dying are different jobs.
///
/// The threads are deliberately not joined. Joining a client's thread means
/// waiting for that client to hang up, which serialises the whole daemon
/// behind whoever connected first: the panel stays open for a whole live, so
/// the CLI would never be served. The socket tests catch this by connecting
/// twice.
pub fn serve(
    listener: UnixListener,
    engine: Arc<Mutex<Engine>>,
    chat: Arc<Shared>,
    quit: mpsc::Sender<()>,
) {
    for incoming in listener.incoming() {
        let Ok(stream) = incoming else { continue };
        let engine = Arc::clone(&engine);
        let chat = Arc::clone(&chat);
        let quit = quit.clone();
        std::thread::spawn(move || {
            if converse(stream, &engine, chat) {
                let _ = quit.send(());
            }
        });
    }
}

/// `chat --follow`: after the one reply, every line that arrives is pushed
/// as a reply of its own, until the client hangs up. Woken by the wire's
/// bell, never by a clock.
fn follow(out: &mut UnixStream, chat: &Shared, mut since: u64) {
    loop {
        let (reachable, lines) = {
            let feed = chat.feed.lock().expect("feed");
            (feed.reachable, feed.since(since))
        };
        if let Some(last) = lines.last() {
            since = last.seq;
            let reply = Reply::Chat { reachable, lines };
            if out.write_all(encode(&reply).as_bytes()).is_err() {
                return;
            }
        }
        let held = chat.bell.lock().expect("bell");
        // A timeout only so a client that hung up is noticed within a second
        // even when the room is quiet.
        let (_held, _) = chat
            .changed
            .wait_timeout(held, std::time::Duration::from_secs(1))
            .expect("bell");
        if out.write_all(b"").is_err() {
            return;
        }
    }
}

/// One client, until it hangs up. Returns whether it asked the engine to quit.
fn converse(stream: UnixStream, engine: &Mutex<Engine>, chat: Arc<Shared>) -> bool {
    let Ok(mut out) = stream.try_clone() else {
        return false;
    };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { return false };
        if line.trim().is_empty() {
            continue;
        }
        // Whether this command could have changed the setup. Reads never do,
        // and there are twelve of those a second from any panel with a meter
        // on it; writing the file for each would be twelve writes a second for
        // nothing.
        let worth_keeping = decode(&line).map(|command| changes_the_setup(&command)) == Ok(true);
        let reply = match decode(&line) {
            Ok(command) => {
                let mut engine = match engine.lock() {
                    Ok(engine) => engine,
                    // A panic in one command must not wedge the daemon for
                    // every client after it.
                    Err(poisoned) => poisoned.into_inner(),
                };
                let reply = engine.handle(command);
                if engine.quitting() {
                    remuxd_domain::log::note("quitting: a client asked");
                    let _ = out.write_all(encode(&reply).as_bytes());
                    return true;
                }
                reply
            }
            Err(message) => Reply::Error { message },
        };
        if out.write_all(encode(&reply).as_bytes()).is_err() {
            return false;
        }
        if let Ok(Command::Rewire) = decode(&line) {
            crate::wire::rewire(&chat);
        }
        if let Ok(Command::Chat { follow: true, .. }) = decode(&line) {
            let since = match &reply {
                Reply::Chat { lines, .. } => lines.last().map_or(0, |l| l.seq),
                _ => 0,
            };
            follow(&mut out, &chat, since);
            return false;
        }
        if worth_keeping {
            let setup = match engine.lock() {
                Ok(engine) => engine.remembered(),
                Err(poisoned) => poisoned.into_inner().remembered(),
            };
            crate::prefs::write(&setup);
        }
    }
    false
}

/// Whether a command could have changed the setup that is written down.
///
/// Reads never do, and there are twelve of those a second from any panel with a
/// meter on it. Going live and recording never do either: what is kept is
/// choices, never state, so that an engine which was publishing when the
/// machine slept does not come up publishing into an empty room.
fn changes_the_setup(command: &Command) -> bool {
    !matches!(
        command,
        Command::Status
            | Command::Levels
            | Command::Devices
            | Command::Shot { .. }
            | Command::Grants
            | Command::Chat { .. }
            | Command::Rewire
            | Command::Quit
            | Command::GoLive
            | Command::Live { .. }
            | Command::Stop
            | Command::RecordStart
            | Command::RecordStop
            | Command::Arm { .. }
            | Command::Retitle { .. }
            | Command::Announce { .. }
            | Command::Card { .. }
            | Command::Countdown { .. }
            | Command::Share { .. }
            | Command::Mute { .. }
            | Command::HideEverything
            | Command::NextTrack
    )
}
