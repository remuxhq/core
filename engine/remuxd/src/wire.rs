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
use crate::events::Followed;
use remuxd_domain::app::events::Event;
use remuxd_domain::protocol::ChatLine;

/// What the wire has said, readable without waiting, and what waits to be
/// said. Shared between the socket's thread, the engine and the daemon's
/// socket (a follower wakes on the feed's bell).
pub struct Shared {
    /// The chat, rung whenever it changed, for a follower to wake on.
    pub feed: Bell<Feed>,
    /// The events, where a line off the wire is said too.
    events: Arc<Followed>,
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
    pub fn new(feed: Arc<Mutex<Feed>>, events: Arc<Followed>) -> Arc<Self> {
        Arc::new(Self {
            feed: Bell::new(feed),
            events,
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
                let said = {
                    let mut feed = self.feed.lock();
                    let seq = feed.push_line(line.clone());
                    Event::said(&ChatLine { seq, ..line })
                };
                self.ring();
                self.events.tell([said]);
            }
            // Said as it came, and what a moderator took down on the platform
            // (a message, a viewer's lines, the chat) leaves every face here.
            Down::Event(happened) => {
                let taken = self.feed.lock().take_down(&happened);
                self.ring();
                let hidden = taken.into_iter().map(|line| Event::ChatHidden { line });
                self.events
                    .tell(std::iter::once(Event::ChatEvent { happened }).chain(hidden));
            }
            // Not events: what was said before this engine opened comes
            // again on every connect, and a bridge that dropped and came back
            // would say it all twice to every face that follows.
            Down::History(lines) => {
                let mut feed = self.feed.lock();
                for line in lines {
                    feed.push_line(line);
                }
                self.ring();
            }
            Down::Destinations(rows) => {
                let moved = {
                    let mut kept = self.destinations.lock().expect("destinations");
                    let moved = remuxd_domain::app::events::rows_between(&kept, &rows);
                    *kept = rows;
                    moved
                };
                if !moved.is_empty() {
                    self.events.tell(moved);
                }
            }
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
        // The account's verbs go up the control half, the deletes and says up the
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Followed;
    use remuxd_domain::app::events::Event;
    use remuxd_domain::app::wire::{Happening, What};
    use remuxd_domain::protocol::ChatLine;
    use std::time::Instant;

    fn said(id: &str, body: &str) -> ChatLine {
        ChatLine {
            seq: 0,
            from: "ana".into(),
            body: body.into(),
            platform: "twitch".into(),
            id: id.into(),
            channel: "kartths".into(),
        }
    }

    fn shared() -> (Arc<Shared>, Arc<Followed>) {
        let followed = Followed::starting_at(1);
        let shared = Shared::new(
            Arc::new(Mutex::new(Feed::starting_at(1))),
            Arc::clone(&followed),
        );
        (shared, followed)
    }

    #[test]
    fn a_line_off_the_wire_is_an_event_with_the_chats_number() {
        let (shared, followed) = shared();
        shared.fold(Down::Line(said("m1", "oi")));
        let held = followed.after(0, Duration::ZERO).events;
        assert_eq!(held.len(), 1);
        assert_eq!(
            held[0].event,
            Event::Chat {
                line: 1,
                platform: "twitch".into(),
                channel: "kartths".into(),
                from: "ana".into(),
                body: "oi".into(),
                id: "m1".into(),
            }
        );
    }

    fn happened(what: What) -> Happening {
        Happening {
            what,
            id: "e1".into(),
            platform: "twitch".into(),
            channel: "kartths".into(),
            from: "ana".into(),
            body: String::new(),
            badges: Vec::new(),
            reply: String::new(),
        }
    }

    #[test]
    fn an_event_off_the_wire_is_said_as_it_came() {
        let (shared, followed) = shared();
        let raid = happened(What::Raid { viewers: 42 });
        shared.fold(Down::Event(raid.clone()));
        let held = followed.after(0, Duration::ZERO).events;
        assert_eq!(
            held.iter().map(|n| n.event.clone()).collect::<Vec<_>>(),
            vec![Event::ChatEvent { happened: raid }]
        );
    }

    // A moderator deleted a message on Twitch and remux kept showing it.
    #[test]
    fn what_a_moderator_took_down_is_said_and_leaves_every_face() {
        let (shared, followed) = shared();
        shared.fold(Down::Line(said("m1", "spam")));
        shared.fold(Down::Line(said("m2", "fine")));
        let deleted = happened(What::Deleted {
            target: "m1".into(),
        });
        shared.fold(Down::Event(deleted.clone()));
        let held = followed.after(0, Duration::ZERO).events;
        assert_eq!(
            held.iter()
                .skip(2)
                .map(|n| n.event.clone())
                .collect::<Vec<_>>(),
            vec![
                Event::ChatEvent { happened: deleted },
                Event::ChatHidden { line: 1 }
            ]
        );
        assert_eq!(
            shared
                .feed
                .lock()
                .since(0)
                .iter()
                .map(|l| l.id.clone())
                .collect::<Vec<_>>(),
            vec!["m2".to_string()]
        );
    }

    // What was said before the engine opened comes again on every connect:
    // as events, a bridge that dropped and came back would say it all twice.
    #[test]
    fn the_history_a_wire_opens_with_is_not_an_event() {
        let (shared, followed) = shared();
        shared.fold(Down::History(vec![said("m0", "earlier")]));
        assert_eq!(shared.feed.lock().since(0).len(), 1, "the chat has it");
        assert_eq!(followed.after(0, Duration::ZERO).events, vec![]);
    }

    #[test]
    fn a_face_following_the_events_is_woken_by_a_line() {
        let (shared, followed) = shared();
        let waiting = Arc::clone(&followed);
        let began = Instant::now();
        let follower = std::thread::spawn(move || waiting.after(0, Duration::from_secs(10)).events);
        while followed.asleep() == 0 {
            std::thread::yield_now();
        }
        shared.fold(Down::Line(said("m1", "oi")));
        assert_eq!(follower.join().expect("the follower returns").len(), 1);
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "woken by its patience, after {:?}",
            began.elapsed()
        );
    }

    fn row(id: i64, armed: bool) -> Destination {
        Destination {
            id,
            name: "twitch".into(),
            platform: "twitch".into(),
            status: "off".into(),
            armed,
            sandbox: false,
            connected: true,
            account: None,
            category: None,
            category_id: None,
            viewers: None,
            viewers_peak: None,
            trouble: None,
            title: None,
            description: None,
            channel: None,
        }
    }

    // With an account the server owns the rows: armed from the site, from
    // another machine, or by this engine asking, the change is the list it
    // sends down, and only the rows that moved are said.
    #[test]
    fn a_row_the_server_changed_is_an_event_and_the_first_list_is_not() {
        let (shared, followed) = shared();
        shared.fold(Down::Destinations(vec![row(2, false), row(6, true)]));
        assert_eq!(followed.after(0, Duration::ZERO).events, vec![]);
        shared.fold(Down::Destinations(vec![row(2, true), row(6, true)]));
        let held = followed.after(0, Duration::ZERO).events;
        assert_eq!(
            held.iter().map(|e| &e.event).collect::<Vec<_>>(),
            [&Event::DestinationArmed { id: 2, on: true }]
        );
    }
}
