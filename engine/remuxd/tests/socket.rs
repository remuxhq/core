//! The daemon, end to end: a real process, a real unix socket, real lines.
//!
//! Everything the engine decides is covered by `cargo test` without any of
//! this, so what is left to prove here is only what a socket adds: that a
//! client can reach it, that two clients see one engine, that a bad line does
//! not take the daemon down, and that it cleans up after itself.
//!
//! No test here sleeps. Waiting on a duration is waiting on a guess, and a
//! guess that is usually long enough is a flake wearing a disguise. The daemon
//! prints `listening <path>` once the socket is bound and accepting, so the
//! effect is there to wait for.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Daemon {
    child: Child,
    socket: PathBuf,
}

impl Daemon {
    fn start() -> Self {
        Self::start_on(unique_socket())
    }

    fn start_on(socket: PathBuf) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_remuxd"))
            .env("REMUXD_SOCKET", &socket)
            // Its own preferences file, and one that does not exist. Without
            // this every test restores whatever the person running them last
            // set up, which on a real machine means opening their camera.
            .env("REMUXD_PREFS", format!("{}.prefs.json", socket.display()))
            // Somewhere to send a live, because going live is now something
            // that reaches the pipeline and can be refused rather than a flag
            // being set. A file in the temp directory that nobody opens: what
            // these tests are about is the socket, not the stream.
            .env("REMUXD_RTMP", std::env::temp_dir().join("socket-tests.flv"))
            // Its own destinations file, and one that does not exist: never
            // the person's, whose rows would be armed at their platforms.
            .env(
                "REMUX_DESTINATIONS",
                format!("{}.destinations.json", socket.display()),
            )
            .env(
                "REMUX_HISTORY",
                format!("{}.history.jsonl", socket.display()),
            )
            // Never the person's session or chat wire either: a daemon under
            // test watches nothing but its own files.
            .env(
                "REMUX_SESSION",
                format!("{}.session.json", socket.display()),
            )
            .env("REMUX_CONFIG", format!("{}.config.toml", socket.display()))
            .env(
                "REMUXD_RECORD_DIR",
                std::env::temp_dir().join("socket-tests"),
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("remuxd starts");

        // The readiness protocol. Reading one line blocks until the daemon has
        // bound the socket, which is exactly as long as that takes and not a
        // millisecond of anyone's guess.
        // libobs talks on stdout before the daemon does; the announcement
        // is the first line that is the daemon's own.
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut lines = BufReader::new(stdout);
        let mut announced = String::new();
        for _ in 0..500 {
            announced.clear();
            let read = lines
                .read_line(&mut announced)
                .expect("the daemon announces itself");
            if read == 0 || announced.starts_with("listening ") {
                break;
            }
        }
        if !announced.starts_with("listening ") {
            // It died before it could speak. Its own complaint is the useful
            // thing, not the empty line we were handed.
            let _ = child.kill();
            let status = child.wait().map(|s| s.to_string()).unwrap_or_default();
            let mut why = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = std::io::Read::read_to_string(&mut stderr, &mut why);
            }
            panic!(
                "no readiness line (got {announced:?}); the daemon ({status}) said: {}",
                why.trim()
            );
        }

        Self { child, socket }
    }

    fn connect(&self) -> Client {
        Client::new(UnixStream::connect(&self.socket).expect("a client connects"))
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

struct Client {
    out: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    fn new(stream: UnixStream) -> Self {
        // A wedged daemon must fail its client, not hang it. Without these the
        // read blocks forever and the suite stops, holding the target lock; a
        // test that never ends is worse than one that fails, because nobody
        // sees it until they wonder why the terminal went quiet. Five seconds
        // is roughly twenty times the whole suite's runtime, so it can only
        // fire on something genuinely stuck.
        let budget = Some(Duration::from_secs(5));
        stream.set_read_timeout(budget).expect("a read budget");
        stream.set_write_timeout(budget).expect("a write budget");
        let reader = BufReader::new(stream.try_clone().expect("a second handle"));
        Self {
            out: stream,
            reader,
        }
    }

    /// Send one raw line and read the one reply it earns.
    fn ask(&mut self, line: &str) -> String {
        writeln!(self.out, "{line}").expect("the daemon takes the line");
        let mut reply = String::new();
        self.reader
            .read_line(&mut reply)
            .expect("the daemon answers");
        reply.trim_end().to_string()
    }
}

/// A socket of its own per test, so the suite can run in parallel the way
/// cargo runs it by default.
fn unique_socket() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let nth = NEXT.fetch_add(1, Ordering::Relaxed);
    // A unix socket path is capped near 104 bytes on macOS, so this stays short.
    std::env::temp_dir().join(format!("rd{}-{nth}.sock", std::process::id()))
}

#[test]
fn a_client_asks_for_the_status_and_gets_one() {
    let daemon = Daemon::start();
    let reply = daemon.connect().ask(r#"{"cmd":"status"}"#);
    assert!(reply.starts_with(r#"{"reply":"status""#), "got {reply}");
    assert!(reply.contains(r#""on_air":false"#), "got {reply}");
}

/// Going live reaches the machine, and says so when the machine says no.
///
/// This used to assert `ok`, back when going live set a flag. It now starts an
/// encoder and a publisher, and a daemon in a test has no display to encode,
/// so the honest answer is a refusal. The refusal is the assertion worth
/// having: it proves the command crossed the socket, reached the engine,
/// reached the pipeline and came back, which a flag being flipped never did.
/// It must also be *quick*: waiting for a keyframe that cannot come would hold
/// the engine's lock for every other client.
#[test]
fn going_live_with_nothing_to_send_is_refused_over_the_socket() {
    let daemon = Daemon::start();
    let mut client = daemon.connect();
    let began = std::time::Instant::now();
    let reply = client.ask(r#"{"cmd":"go-live"}"#);
    assert!(
        reply.starts_with(r#"{"reply":"error""#),
        "there is no picture on a test machine, got {reply}"
    );
    assert!(
        began.elapsed() < std::time::Duration::from_secs(2),
        "it must refuse rather than wait, took {:?}",
        began.elapsed()
    );
    assert!(client
        .ask(r#"{"cmd":"status"}"#)
        .contains(r#""on_air":false"#));
}

// Three clients drive this daemon and they are not the same program. Two of
// them disagreeing about the state of the live is the bug this guards.
#[test]
fn two_clients_are_looking_at_one_engine() {
    let daemon = Daemon::start();
    let mut first = daemon.connect();
    let mut second = daemon.connect();

    // The microphone rather than the live: it is state both clients must
    // agree about, and it needs no display to change.
    first.ask(r#"{"cmd":"mute","on":true}"#);
    assert!(second
        .ask(r#"{"cmd":"status"}"#)
        .contains(r#""muted":true"#));

    second.ask(r#"{"cmd":"mute","on":false}"#);
    assert!(first
        .ask(r#"{"cmd":"status"}"#)
        .contains(r#""muted":false"#));
}

#[test]
fn a_line_that_is_not_a_command_is_refused_and_the_daemon_lives_on() {
    let daemon = Daemon::start();
    let mut client = daemon.connect();
    assert!(client
        .ask("not json at all")
        .starts_with(r#"{"reply":"error""#));
    assert!(client
        .ask(r#"{"cmd":"fly"}"#)
        .starts_with(r#"{"reply":"error""#));
    // still serving. Something that works without a display, so that this
    // says "the daemon lives on" and not "this machine has a screen".
    let answer = client.ask(r#"{"cmd":"mute","on":true}"#);
    assert!(answer.contains(r#""muted":true"#), "got {answer}");
}

/// Every verb answers, and the one that used to be missing answers honestly.
///
/// This used to assert that some verb came back `unsupported`, and the comment
/// above it said it would be built one day and this would fail, loudly, and be
/// pointed at the next gap. It was `next-track` until the music landed, then
/// `arm` until the app did, and now there is no next gap: `Reply::Unsupported`
/// is gone because nothing could produce it, and a reply nothing can produce
/// is one somebody will write a reader for.
///
/// What is left is the property that made deleting it safe, and it is worth an
/// assertion of its own: a verb the engine cannot *carry out* still comes back
/// as a sentence rather than as a closed socket. `arm` on a daemon with no app
/// is exactly that, and it is the shape every "not right now" has to keep.
#[test]
fn a_verb_it_cannot_carry_out_answers_rather_than_dying() {
    let daemon = Daemon::start();
    let mut client = daemon.connect();
    let refused = client.ask(r#"{"cmd":"arm","adapter":1,"on":true}"#);
    assert!(
        refused.starts_with(r#"{"reply":"error""#),
        "there is no such destination, so it says so: {refused}"
    );
    assert!(
        refused.contains("no destination 1"),
        "and says which thing is missing: {refused}"
    );
    // Still serving afterwards, which is the half that matters.
    assert!(client
        .ask(r#"{"cmd":"status"}"#)
        .starts_with(r#"{"reply":"status""#));
}

#[test]
fn a_client_that_hangs_up_mid_conversation_does_not_take_the_daemon_with_it() {
    let daemon = Daemon::start();
    {
        let mut leaving = daemon.connect();
        // Something that leaves a mark and needs no display, so this test is
        // about the client walking away and not about the machine.
        leaving.ask(r#"{"cmd":"mute","on":true}"#);
    } // dropped without a goodbye
    assert!(daemon
        .connect()
        .ask(r#"{"cmd":"status"}"#)
        .contains(r#""muted":true"#));
}

#[test]
fn quit_answers_first_and_then_the_daemon_goes_away_taking_its_socket() {
    let mut daemon = Daemon::start();
    assert_eq!(
        daemon.connect().ask(r#"{"cmd":"quit"}"#),
        r#"{"reply":"ok"}"#
    );

    // Waiting on the process itself, not on a duration: this returns when the
    // daemon has actually exited.
    let status = daemon.child.wait().expect("the daemon exits");
    assert!(status.success(), "it should leave quietly, got {status}");
    assert!(
        !daemon.socket.exists(),
        "a quit engine leaves no socket behind"
    );
}

// A socket file outlives the process that made it, so a crash leaves a path
// that looks bound and refuses every connection. Refusing to start on it would
// mean a crash needs a manual cleanup before the app opens again.
#[test]
fn a_socket_left_behind_by_a_dead_engine_is_taken_over() {
    let socket = unique_socket();
    std::fs::write(&socket, b"the corpse of a previous run").expect("a stale file");
    assert!(socket.exists());

    let daemon = Daemon::start_on(socket);
    assert!(daemon
        .connect()
        .ask(r#"{"cmd":"status"}"#)
        .starts_with(r#"{"reply":"status""#));
}

// The other half of that rule: a socket somebody is actually listening on is
// not stale, and stealing it would leave two engines fighting over one live.
#[test]
fn a_second_engine_refuses_to_steal_a_socket_that_answers() {
    let daemon = Daemon::start();
    let second = Command::new(env!("CARGO_BIN_EXE_remuxd"))
        .env("REMUXD_SOCKET", &daemon.socket)
        .env(
            "REMUXD_PREFS",
            format!("{}.prefs.json", daemon.socket.display()),
        )
        .output()
        .expect("the second one runs");

    assert!(!second.status.success(), "it must not start");
    let complaint = String::from_utf8_lossy(&second.stderr);
    assert!(complaint.contains("already listening"), "got {complaint}");
    // and the first one is untouched
    assert!(daemon
        .connect()
        .ask(r#"{"cmd":"status"}"#)
        .starts_with(r#"{"reply":"status""#));
}
