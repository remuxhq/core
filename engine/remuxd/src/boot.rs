//! The daemon, as a function: bind the socket, start the motor, wire the
//! engine, tick, serve, park. A `main` gives it the motor and what the main
//! thread does while the daemon runs; everything else is here, once, for
//! every motor.

use std::io::Write;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use remuxd_domain::engine::{Engine, Pipeline, Sources};

/// The machine behind the ports, started.
pub struct Motor {
    /// What the status says: `obs 30.2.3`.
    pub name: String,
    pub sources: Box<dyn Sources>,
    pub pipeline: Box<dyn Pipeline>,
}

/// Where Go live sends the picture, from the environment and never from a
/// client: `REMUXD_RTMP` overrides the destinations, which is how a test
/// send a live to one door or a file.
fn destination() -> Option<String> {
    std::env::var("REMUXD_RTMP").ok().filter(|s| !s.is_empty())
}

/// Where recordings are written: the config's, or the OS's films folder.
fn recordings() -> Option<String> {
    Some(remuxd_domain::config::record_dir().display().to_string())
}

/// The wires and who is watching. With a session (`remux login`) the web's
/// rows are what a face sees and its verbs go up the web's wire; without
/// one, the destinations file on this machine. A chat source of one's own
/// (`remux chat --url`) takes the chat and nothing else. The feed starts
/// numbering at the clock so a restart never hands a face a number below one
/// it has.
/// Milliseconds past the epoch, where the chat's and the events' numbers
/// start: an engine restarted under a face hands over bigger ones.
fn started() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(1)
}

fn wire() -> (
    Arc<crate::wire::Shared>,
    Box<dyn remuxd_domain::engine::Watching>,
) {
    use crate::wire::{keep, App, Shared, Source};
    use remuxd_domain::air::destinations;
    use remuxd_domain::app::{chat, session};
    let shared = Shared::new(Arc::new(Mutex::new(chat::Feed::starting_at(started()))));
    let watching: Box<dyn remuxd_domain::engine::Watching> = match session::read(&session::path()) {
        Some(session) => Box::new(App::new(session.base, Arc::clone(&shared))),
        None => Box::new(destinations::Local::new(destinations::path())),
    };
    for source in Source::from_files() {
        keep(source, Arc::clone(&shared));
    }
    (shared, watching)
}

/// Run the daemon until it quits. `start` makes the motor, after the socket
/// is bound (a second daemon must fail before any capture starts) and
/// before anything is served. `park` is what the main thread does meanwhile:
/// it gets the quit signal and returns when the daemon is to end; a motor
/// that needs the main thread runs its queue there. Exits the process on a
/// socket or motor failure, with the reason on stderr.
pub fn boot(start: impl FnOnce() -> Result<Motor, String>, park: impl FnOnce(mpsc::Receiver<()>)) {
    remuxd_domain::log::start();
    let path = crate::server::default_socket_path();
    let listener = match crate::server::bind(&path) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("remuxd: cannot listen on {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    let motor = match start() {
        Ok(motor) => motor,
        Err(why) => {
            eprintln!("remuxd: the motor did not start: {why}");
            let _ = std::fs::remove_file(&path);
            std::process::exit(1);
        }
    };
    let (wire, watching) = wire();
    let followed = crate::events::Followed::starting_at(started());
    let engine = Arc::new(Mutex::new(
        Engine::with_sources(motor.sources)
            .with_pipeline(motor.pipeline)
            .with_motor(motor.name)
            .with_library(Box::new(crate::library::Folder::default()))
            .with_destination(destination())
            .with_recordings(recordings())
            .with_history(Some(remuxd_domain::air::history::path()))
            .with_app(watching)
            .with_chat(Arc::clone(&wire.feed))
            .with_events(Arc::clone(&followed.events)),
    ));

    // A heartbeat, for the one thing no client asks for: a track ending. Four
    // times a second is fast enough that the gap between one track and the
    // next is inaudible and slow enough to cost nothing. See `Engine::tick`.
    let (quit, quitted) = mpsc::channel();
    let beating = Arc::clone(&engine);
    let quit_from_the_tick = quit.clone();
    let rung_by_the_tick = Arc::clone(&followed);
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(250));
        if let Ok(mut engine) = beating.lock() {
            let before = rung_by_the_tick.last();
            engine.tick();
            rung_by_the_tick.ring_after(before);
            // The tick is a way out too: a face's lease running out sets
            // `quitting` with no client on the line to notice it.
            if engine.quitting() {
                remuxd_domain::log::note("quitting: the lease ran out");
                let _ = quit_from_the_tick.send(());
                return;
            }
        }
    });

    let announcing = path.clone();
    std::thread::spawn(move || {
        // What was chosen last time, put back before anybody is told the
        // daemon is up. Replayed as commands, so the capture actually starts
        // and a device that has since been unplugged fails here rather than
        // being believed. On this thread and not the main one, because a
        // motor may need the main thread for its own questions.
        match engine.lock() {
            Ok(mut engine) => engine.restore(&crate::prefs::read()),
            Err(poisoned) => poisoned.into_inner().restore(&crate::prefs::read()),
        }
        // Announced only once the socket is bound and the setup is back, so
        // anything waiting on this line can connect straight away. The tests
        // and the CLI both read it.
        println!("listening {}", announcing.display());
        let _ = std::io::stdout().flush();
        crate::server::serve(listener, engine, wire, followed, quit);
    });
    park(quitted);
    let _ = std::fs::remove_file(&path);
}

/// The park of a motor with nothing to do on the main thread: wait for the quit.
pub fn wait(quitted: mpsc::Receiver<()>) {
    let _ = quitted.recv();
}
