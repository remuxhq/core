//! The wire's client: one WebSocket to whatever serves `remuxd_domain::app::wire`,
//! frames down into the feed and the account's state, frames up out of the
//! outbox, reconnecting on its own. The words are the domain's; this is the
//! socket and the thread.
//!
//! With an account it is the web's wire, both halves; with a chat source of
//! one's own it is the `line` half alone, and nothing is asked of the
//! server. With both, two sockets: the account keeps the control half and the
//! chat is the bridge's. Each hears only the frames of the halves it carries.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use remuxd_domain::app::chat::Feed;
use remuxd_domain::app::session::Session;
use remuxd_domain::app::wire::{wires, Adapter, Down, Up, Wire};
use remuxd_domain::protocol::{Destination, Found};
use serde_json::json;

use crate::bell::Bell;

/// What the wire has said, readable without waiting, and what waits to be
/// said. Shared between the socket's thread, the engine and the daemon's
/// socket (a follower wakes on the feed's bell).
pub struct Shared {
    /// The chat, rung whenever it changed, for a follower to wake on.
    pub feed: Bell<Feed>,
    /// Whether the wire carrying the control half is up right now.
    pub connected: AtomicBool,
    pub destinations: Mutex<Vec<Destination>>,
    pub viewers: AtomicU64,
    pub viewers_peak: AtomicU64,
    /// Set when the count is known, which is not the same as zero.
    pub viewers_known: AtomicBool,
    pub categories: Mutex<Option<Found>>,
    /// The relay's RTMP door for this account, with its publish token, as
    /// `/api/session` last gave it.
    pub scene: Mutex<Option<String>>,
    /// What the server said out loud and nobody has read yet.
    pub notices: Mutex<Vec<String>>,
    /// The control frames waiting to go up.
    pub outgoing: Mutex<VecDeque<Up>>,
    /// Bumped by `rewire`; a socket thread from an older generation stops.
    pub generation: AtomicU64,
}

impl Shared {
    pub fn new(feed: Arc<Mutex<Feed>>) -> Arc<Self> {
        Arc::new(Self {
            feed: Bell::new(feed),
            connected: AtomicBool::new(false),
            destinations: Mutex::new(Vec::new()),
            viewers: AtomicU64::new(0),
            viewers_peak: AtomicU64::new(0),
            viewers_known: AtomicBool::new(false),
            categories: Mutex::new(None),
            scene: Mutex::new(None),
            notices: Mutex::new(Vec::new()),
            outgoing: Mutex::new(VecDeque::new()),
            generation: AtomicU64::new(0),
        })
    }

    fn ring(&self) {
        self.feed.ring();
    }

    fn fold(&self, down: Down) {
        match down {
            Down::Line(line) => {
                self.feed.lock().push_line(line);
                self.ring();
            }
            Down::History(lines) => {
                let mut feed = self.feed.lock();
                for line in lines {
                    feed.push_line(line);
                }
                self.ring();
            }
            Down::Destinations(rows) => *self.destinations.lock().expect("destinations") = rows,
            Down::Viewers(watchers) => {
                if let (true, Some(total)) = (watchers.answered, watchers.total) {
                    self.viewers.store(total, Ordering::Relaxed);
                }
                self.viewers_known
                    .store(watchers.answered, Ordering::Relaxed);
                self.viewers_peak.store(watchers.peak, Ordering::Relaxed);
            }
            Down::Categories(found) => *self.categories.lock().expect("categories") = Some(found),
            Down::Notice(words) => self.notices.lock().expect("notices").push(words),
        }
    }

    fn up(&self, frame: Up) -> Result<(), String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err("the wire is not up".into());
        }
        self.outgoing.lock().expect("outgoing").push_back(frame);
        Ok(())
    }
}

/// Where a wire is: a URL the shell kept (`remux chat --url`, the `line`
/// half), or the web's for an account, asked for at every connect because
/// the token on it is the session's.
enum Door {
    Fixed(String),
    Account(Session),
}

/// One wire to keep: where it is and the halves it carries.
pub struct Source {
    door: Door,
    wire: Wire,
}

impl Source {
    /// What the files say now: the account's wire for its control, and the
    /// chat from a wire of one's own if the config names one, else from the
    /// account's.
    pub fn from_files() -> Vec<Source> {
        let own = remuxd_domain::config::chat_url();
        let mut session = remuxd_domain::app::session::read(&remuxd_domain::app::session::path());
        wires(session.is_some(), own.is_some())
            .into_iter()
            .filter_map(|wire| {
                let door = match wire {
                    Wire::Account { .. } => Door::Account(session.take()?),
                    Wire::Own => Door::Fixed(own.clone()?),
                };
                Some(Source { door, wire })
            })
            .collect()
    }

    /// Where to connect. For an account, `/api/session` answers with the
    /// socket's token and the relay's door, and the door is kept for the
    /// engine to publish to.
    fn url(&self, shared: &Shared) -> Result<String, String> {
        match &self.door {
            Door::Fixed(url) => Ok(url.clone()),
            Door::Account(session) => {
                let answer = remux_wire::get_json(
                    &format!("{}/api/session", session.base),
                    &session.token.0,
                )?;
                let text = |key: &str| {
                    answer
                        .body
                        .get(key)
                        .and_then(|t| t.as_str())
                        .map(str::to_string)
                        .ok_or_else(|| format!("the web said {} and no {key}", answer.status))
                };
                *shared.scene.lock().expect("scene") = Some(text("rtmp")?);
                Ok(remuxd_domain::app::wire::url(
                    &session.base,
                    &text("socket_token")?,
                ))
            }
        }
    }

    /// Mark this wire's halves up or down: the control's `connected`, the
    /// chat's `reachable`.
    fn up(&self, shared: &Shared, up: bool) {
        if self.wire.control() {
            shared.connected.store(up, Ordering::Relaxed);
        }
        if self.wire.chat() {
            shared.feed.lock().reachable = up;
        }
        shared.ring();
    }
}

/// Keep the wire up until the daemon ends or `rewire` moves on. Answers at
/// once; `connected` says whether it is up.
pub fn keep(source: Source, shared: Arc<Shared>) {
    let generation = shared.generation.load(Ordering::Relaxed);
    std::thread::spawn(move || {
        while shared.generation.load(Ordering::Relaxed) == generation {
            if let Err(why) = stay(&source, &shared, generation) {
                remuxd_domain::log::note(&format!("wire: {why}"));
            }
            // A drop, unless `rewire` moved on: then the next wire owns the
            // flags and this one leaves them alone.
            if shared.generation.load(Ordering::Relaxed) != generation {
                return;
            }
            source.up(&shared, false);
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

/// `remux chat --url` changed the config: drop the wires and open what the
/// files say now, the live untouched.
pub fn rewire(shared: &Arc<Shared>) {
    shared.generation.fetch_add(1, Ordering::Relaxed);
    shared.connected.store(false, Ordering::Relaxed);
    shared.feed.lock().reachable = false;
    shared.ring();
    let sources = Source::from_files();
    if sources.is_empty() {
        remuxd_domain::log::note("wire: none now; remux chat --url, or remux login");
    }
    for source in sources {
        remuxd_domain::log::note("wire: opening again, as the config says now");
        keep(source, Arc::clone(shared));
    }
}

fn stay(source: &Source, shared: &Shared, generation: u64) -> Result<(), String> {
    let url = source.url(shared)?;
    let (mut socket, _) =
        tungstenite::connect(&url).map_err(|e| format!("the wire would not open: {e}"))?;
    match socket.get_ref() {
        tungstenite::stream::MaybeTlsStream::Plain(stream) => stream.set_nonblocking(true).ok(),
        tungstenite::stream::MaybeTlsStream::Rustls(tls) => {
            tls.get_ref().set_nonblocking(true).ok()
        }
        _ => None,
    };
    for open in source.wire.opens() {
        socket
            .send(tungstenite::Message::Text(open.encode()))
            .map_err(|e| format!("opening: {e}"))?;
    }
    source.up(shared, true);

    let mut beat = std::time::Instant::now();
    loop {
        if shared.generation.load(Ordering::Relaxed) != generation {
            return Ok(());
        }
        // The account's verbs go up the control half, the deletes up the
        // chat's: with two wires, each takes only its own.
        let mut waiting: Vec<Up> = Vec::new();
        if source.wire.control() {
            waiting.extend(shared.outgoing.lock().expect("outgoing").drain(..));
        }
        if source.wire.chat() {
            waiting.extend(shared.feed.lock().take_outgoing());
        }
        // A server closes a wire that says nothing for a minute.
        if beat.elapsed() > Duration::from_secs(25) {
            beat = std::time::Instant::now();
            waiting.push(Up::Heartbeat(json!({})));
        }
        for frame in waiting {
            socket
                .send(tungstenite::Message::Text(frame.encode()))
                .map_err(|e| format!("sending: {e}"))?;
        }
        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                if let Some(down) = Down::read(&text).filter(|d| source.wire.carries(d)) {
                    shared.fold(down);
                }
            }
            Ok(tungstenite::Message::Ping(payload)) => {
                let _ = socket.send(tungstenite::Message::Pong(payload));
            }
            Ok(tungstenite::Message::Close(_)) => return Err("the server closed the wire".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("the wire: {e}")),
        }
    }
}

/// The `Watching` of an engine with an account: the web's rows, and the
/// verbs sent up the wire. The web owns every column; this asks.
pub struct App {
    base: String,
    shared: Arc<Shared>,
}

impl App {
    pub fn new(base: String, shared: Arc<Shared>) -> Self {
        Self { base, shared }
    }
}

impl remuxd_domain::engine::Watching for App {
    fn server(&self) -> Option<String> {
        Some(self.base.clone())
    }
    fn reachable(&self) -> bool {
        self.shared.connected.load(Ordering::Relaxed)
    }
    fn destinations(&self) -> Vec<Destination> {
        self.shared
            .destinations
            .lock()
            .expect("destinations")
            .clone()
    }
    /// The relay's door, with this session's token: one stream, the relay
    /// fans out to every armed destination.
    fn scene(&self) -> Option<String> {
        self.shared.scene.lock().expect("scene").clone()
    }
    fn viewers(&self) -> Option<u32> {
        self.shared
            .viewers_known
            .load(Ordering::Relaxed)
            .then(|| self.shared.viewers.load(Ordering::Relaxed) as u32)
    }
    fn viewers_peak(&self) -> Option<u32> {
        match self.shared.viewers_peak.load(Ordering::Relaxed) {
            0 => None,
            n => Some(n as u32),
        }
    }
    fn arm(&self, adapter: i64, on: bool) -> Result<(), String> {
        self.shared.up(Up::Arm { adapter, on })
    }
    fn sandbox(&self, adapter: i64, on: bool) -> Result<(), String> {
        self.shared.up(Up::Sandbox { adapter, on })
    }
    fn retitle(
        &self,
        adapter: i64,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<(), String> {
        self.shared.up(Up::Retitle {
            adapter,
            title: title.map(str::to_string),
            description: description.map(str::to_string),
        })
    }
    fn announce(&self, adapter: i64) -> Result<(), String> {
        self.shared.up(Up::Announce(Adapter { adapter }))
    }
    fn disconnect(&self, adapter: i64) -> Result<(), String> {
        self.shared.up(Up::Disconnect(Adapter { adapter }))
    }
    fn categorize(&self, adapter: i64, id: &str, name: &str) -> Result<(), String> {
        self.shared.up(Up::Categorize {
            adapter,
            id: id.to_string(),
            name: name.to_string(),
        })
    }
    fn search_categories(&self, adapter: i64, query: &str) -> Result<(), String> {
        self.shared.up(Up::Search {
            adapter,
            query: query.to_string(),
        })
    }
    fn found(&self) -> Option<Found> {
        self.shared.categories.lock().expect("categories").clone()
    }
    fn notices(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.shared.notices.lock().expect("notices"))
    }
}
