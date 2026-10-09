//! The app: what the engine asks the web on a face's behalf, and the chat off its wire.

use super::*;

impl Engine {
    /// The app owns the column; this asks, and says so when it cannot.
    pub(super) fn arm(&mut self, adapter: i64, on: bool) -> Reply {
        match self.watching.arm(adapter, on) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn disconnect(&mut self, adapter: i64) -> Reply {
        match self.watching.disconnect(adapter) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn categorize(&mut self, adapter: i64, id: &str, name: &str) -> Reply {
        match self.watching.categorize(adapter, id, name) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn search_categories(&mut self, adapter: i64, query: &str) -> Reply {
        match self.watching.search_categories(adapter, query) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn sandbox(&mut self, adapter: i64, on: bool) -> Reply {
        match self.watching.sandbox(adapter, on) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn retitle(
        &mut self,
        adapter: i64,
        title: Option<String>,
        description: Option<String>,
    ) -> Reply {
        match self
            .watching
            .retitle(adapter, title.as_deref(), description.as_deref())
        {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn announce(&mut self, adapter: i64) -> Reply {
        match self.watching.announce(adapter) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    /// The chat is the feed's, off whichever wire the daemon opened; the
    /// engine only reads it and takes lines off it.
    pub(super) fn chat(&mut self, since: u64) -> Reply {
        let feed = self.chat.lock().expect("chat");
        Reply::Chat {
            reachable: feed.reachable,
            lines: feed.since(since),
        }
    }

    /// Off every face, and off every face that follows the events too.
    pub(super) fn hide(&mut self, seq: u64) -> Reply {
        self.chat.lock().expect("chat").hide(seq);
        self.keep([crate::app::events::Event::ChatHidden { line: seq }]);
        Reply::Ok
    }

    /// Off every face here at once, and a delete down the wire for the
    /// platform. A line the engine no longer has cannot be deleted from here.
    pub(super) fn delete_chat(&mut self, seq: u64) -> Reply {
        let deleted = self.chat.lock().expect("chat").delete(seq);
        match deleted {
            Ok(()) => {
                self.keep([crate::app::events::Event::ChatHidden { line: seq }]);
                Reply::Ok
            }
            Err(message) => Reply::Error { message },
        }
    }

    /// Up the wire to the platform's chat; the line comes back down from
    /// there, so nothing is kept or told here.
    pub(super) fn say(&mut self, body: &str, channel: Option<String>) -> Reply {
        match self.chat.lock().expect("chat").say(body, channel) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::*;
    use crate::protocol::Category;

    /// An app with two destinations and something said in the chat.
    struct AnApp;

    impl Watching for AnApp {
        fn reachable(&self) -> bool {
            true
        }
        fn destinations(&self) -> Vec<Destination> {
            vec![
                Destination {
                    id: 1,
                    name: "tico".into(),
                    platform: "twitch".into(),
                    status: "live".into(),
                    armed: true,
                    sandbox: false,
                    connected: true,
                    account: None,
                    category: None,
                    category_id: None,
                    viewers_peak: None,
                    trouble: None,
                    viewers: Some(42),
                    channel: Some("http://localhost:8880/tico/".into()),
                    title: Some("Rust at midnight".into()),
                    description: None,
                },
                Destination {
                    id: 2,
                    name: "teco".into(),
                    platform: "youtube".into(),
                    status: "off".into(),
                    armed: false,
                    sandbox: false,
                    connected: true,
                    account: None,
                    category: None,
                    category_id: None,
                    viewers_peak: None,
                    trouble: None,
                    // Off air and nobody asked: the two are different and only
                    // one of them is a zero.
                    viewers: None,
                    channel: None,
                    title: None,
                    description: None,
                },
            ]
        }
        fn viewers(&self) -> Option<u32> {
            Some(12)
        }
        fn arm(&self, _adapter: i64, _on: bool) -> Result<(), String> {
            Ok(())
        }
        fn sandbox(&self, _adapter: i64, _on: bool) -> Result<(), String> {
            Ok(())
        }
        fn retitle(&self, _: i64, _: Option<&str>, _: Option<&str>) -> Result<(), String> {
            Ok(())
        }
        fn announce(&self, _adapter: i64) -> Result<(), String> {
            Ok(())
        }
        fn disconnect(&self, _adapter: i64) -> Result<(), String> {
            Ok(())
        }
        fn categorize(&self, _adapter: i64, _id: &str, _name: &str) -> Result<(), String> {
            Ok(())
        }
        fn search_categories(&self, _adapter: i64, _query: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn the_engine_repeats_the_app_rather_than_holding_a_copy() {
        let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(AnApp));
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status answers with a status")
        };
        assert!(status.app_reachable, "and says the app is reachable");
        assert_eq!(status.destinations.len(), 2);
        assert_eq!(status.destinations[0].name, "tico");
        assert_eq!(
            status.destinations[0].title.as_deref(),
            Some("Rust at midnight"),
            "what the live is called rides on the row, so every face can show it"
        );
    }

    /// What an app was asked to call a live: the destination, the title, the
    /// line under it.
    type Asked = std::sync::Arc<std::sync::Mutex<Vec<(i64, Option<String>, Option<String>)>>>;

    /// An app that writes down what it was asked to call a live.
    struct Retitled(Asked);

    impl Watching for Retitled {
        fn categorize(&self, adapter: i64, id: &str, name: &str) -> Result<(), String> {
            self.0.lock().expect("retitled").push((
                adapter,
                Some(format!("filed {id} {name}")),
                None,
            ));
            Ok(())
        }
        fn search_categories(&self, adapter: i64, query: &str) -> Result<(), String> {
            self.0.lock().expect("retitled").push((
                adapter,
                Some(format!("searched {query}")),
                None,
            ));
            Ok(())
        }
        fn found(&self) -> Option<Found> {
            Some(Found {
                adapter: 2,
                query: "sci".into(),
                items: vec![Category {
                    id: "509670".into(),
                    name: "Science & Technology".into(),
                }],
            })
        }
        fn reachable(&self) -> bool {
            true
        }
        fn destinations(&self) -> Vec<Destination> {
            Vec::new()
        }
        fn viewers(&self) -> Option<u32> {
            None
        }
        fn arm(&self, _adapter: i64, _on: bool) -> Result<(), String> {
            Ok(())
        }
        fn retitle(
            &self,
            adapter: i64,
            title: Option<&str>,
            description: Option<&str>,
        ) -> Result<(), String> {
            self.0.lock().expect("retitled").push((
                adapter,
                title.map(str::to_string),
                description.map(str::to_string),
            ));
            Ok(())
        }
        fn sandbox(&self, adapter: i64, on: bool) -> Result<(), String> {
            // Written down as a title, like the announcement, so one list serves.
            self.0.lock().expect("retitled").push((
                adapter,
                Some(if on { "sandbox on" } else { "sandbox off" }.into()),
                None,
            ));
            Ok(())
        }
        fn disconnect(&self, adapter: i64) -> Result<(), String> {
            self.0
                .lock()
                .expect("retitled")
                .push((adapter, Some("disconnected".into()), None));
            Ok(())
        }
        fn announce(&self, adapter: i64) -> Result<(), String> {
            // Written down as a title of "announced", so one list serves.
            self.0
                .lock()
                .expect("retitled")
                .push((adapter, Some("announced".into()), None));
            Ok(())
        }
        fn notices(&mut self) -> Vec<String> {
            vec!["! tico: the token was refused".into()]
        }
    }

    // OBS has "Update": tell the platform now, not at the next Go live. The
    // engine only pushes, so the platform's refusal comes back as a notice
    // and lands in the log with the destination's name.
    #[test]
    fn an_announcement_is_asked_of_the_app_and_its_refusal_reaches_the_log() {
        let asked: Asked = Default::default();
        let mut told =
            Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(Retitled(asked.clone())));
        assert_eq!(told.handle(Command::Announce { adapter: 2 }), Reply::Ok);
        assert_eq!(
            asked.lock().expect("retitled").last(),
            Some(&(2, Some("announced".to_string()), None))
        );
        told.tick();
        let Reply::Log { lines: log } = told.handle(Command::Log) else {
            panic!("log answers with the journal")
        };
        assert!(
            log.iter()
                .any(|line| line.contains("! tico: the token was refused")),
            "the notice is in the log: {log:?}"
        );
    }

    // The title is the app's column, like `armed`: the engine asks and
    // repeats, and never keeps a copy that could disagree with the row.
    #[test]
    fn disconnecting_is_asked_of_the_app_and_kept_nowhere() {
        let asked: Asked = Default::default();
        let mut told =
            Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(Retitled(asked.clone())));
        assert_eq!(told.handle(Command::Disconnect { adapter: 2 }), Reply::Ok);
        assert_eq!(
            asked.lock().expect("retitled").last(),
            Some(&(2, Some("disconnected".to_string()), None))
        );
    }

    // The category is the app's column, like the title: the engine asks, keeps
    // nothing, and the search's answer rides on the status for every face.
    #[test]
    fn a_category_is_asked_of_the_app_and_the_search_answers_on_the_status() {
        let asked: Asked = Default::default();
        let mut told =
            Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(Retitled(asked.clone())));
        assert_eq!(
            told.handle(Command::Categorize {
                adapter: 2,
                id: "509670".into(),
                name: "Science & Technology".into()
            }),
            Reply::Ok
        );
        assert_eq!(
            asked.lock().expect("retitled").last(),
            Some(&(
                2,
                Some("filed 509670 Science & Technology".to_string()),
                None
            ))
        );
        assert_eq!(
            told.handle(Command::Categories {
                adapter: 2,
                query: "sci".into()
            }),
            Reply::Ok
        );
        let Reply::Categories { found } = told.handle(Command::CategoriesFound) else {
            panic!("categories-found answers with what was found")
        };
        let found = found.expect("the app's answer is kept for every face");
        assert_eq!(found.query, "sci");
        assert_eq!(found.items[0].name, "Science & Technology");
    }

    // The sandbox is the app's column, like armed: the engine asks and keeps
    // nothing, and every face reads it back on the rows.
    #[test]
    fn the_sandbox_is_asked_of_the_app_and_never_kept_here() {
        let asked: Asked = Default::default();
        let mut told =
            Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(Retitled(asked.clone())));
        assert_eq!(
            told.handle(Command::Sandbox {
                adapter: 2,
                on: true
            }),
            Reply::Ok
        );
        assert_eq!(
            asked.lock().expect("retitled").last(),
            Some(&(2, Some("sandbox on".to_string()), None))
        );
        // and an engine with no app says so
        let mut alone = Engine::with_sources(Box::new(ThisMachine));
        assert!(matches!(
            alone.handle(Command::Sandbox {
                adapter: 2,
                on: false
            }),
            Reply::Error { .. }
        ));
    }

    #[test]
    fn a_title_is_asked_of_the_app_and_never_kept_here() {
        let asked: Asked = Default::default();
        let mut told =
            Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(Retitled(asked.clone())));
        let reply = told.handle(Command::Retitle {
            adapter: 2,
            title: Some("Rust at midnight".into()),
            description: None,
        });
        assert_eq!(reply, Reply::Ok);
        assert_eq!(
            *asked.lock().expect("retitled"),
            vec![(2, Some("Rust at midnight".to_string()), None)]
        );
        // And with nobody to ask, it says so rather than pretending.
        let reply = engine().handle(Command::Retitle {
            adapter: 2,
            title: Some("x".into()),
            description: None,
        });
        assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
    }

    #[test]
    fn an_engine_with_no_app_is_a_real_configuration_and_not_a_broken_one() {
        let mut engine = engine();
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status answers with a status")
        };
        assert!(!status.app_reachable);
        assert!(status.destinations.is_empty());
        // And it says so rather than pretending it armed something.
        assert!(matches!(
            engine.handle(Command::Arm {
                adapter: 1,
                on: true
            }),
            Reply::Error { .. }
        ));
    }

    /// An engine whose wire has said three things, numbered from seven.
    fn told_engine() -> Engine {
        use crate::app::chat::Feed;
        use crate::app::wire::Line;
        let mut feed = Feed::starting_at(7);
        for (id, body) in [("m7", "hello"), ("m8", "gg"), ("m9", "first!")] {
            feed.push(Line {
                id: id.into(),
                platform: "twitch".into(),
                channel: "tw".into(),
                from: "ana".into(),
                body: body.into(),
            });
        }
        feed.reachable = true;
        Engine::with_sources(Box::new(ThisMachine))
            .with_chat(std::sync::Arc::new(std::sync::Mutex::new(feed)))
    }

    #[test]
    fn the_chat_says_whether_its_wire_is_up() {
        let mut told = told_engine();
        let Reply::Chat { reachable, lines } = told.handle(Command::Chat {
            since: 0,
            follow: false,
        }) else {
            panic!("chat answers with chat")
        };
        assert!(reachable);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].body, "hello");

        // A quiet room and a wire that is down look identical in the list
        // and must not look identical in the answer.
        let Reply::Chat { reachable, lines } = engine().handle(Command::Chat {
            since: 0,
            follow: false,
        }) else {
            panic!("chat answers with chat")
        };
        assert!(!reachable);
        assert!(lines.is_empty());
    }

    // A face asks once a second, and a thousand lines a second for nothing
    // new is the cost that made the cap forty. So it asks for what came after
    // the last line it has, by sequence, and gets only that.
    #[test]
    fn the_chat_hands_over_only_what_came_after_since() {
        let mut told = told_engine();
        let Reply::Chat { lines, .. } = told.handle(Command::Chat {
            since: 8,
            follow: false,
        }) else {
            panic!("chat answers with chat")
        };
        assert_eq!(
            lines.iter().map(|l| l.seq).collect::<Vec<_>>(),
            vec![9],
            "only what is newer than 8"
        );
    }

    // A spammer's line comes off every face, the overlay on the stream included,
    // and stays off: the app keeps the conversation and would hand it back.
    #[test]
    fn a_hidden_line_is_gone_from_every_face_for_the_rest_of_the_run() {
        let mut told = told_engine();
        assert!(matches!(told.handle(Command::Hide { seq: 8 }), Reply::Ok));
        let Reply::Chat { lines, .. } = told.handle(Command::Chat {
            since: 0,
            follow: false,
        }) else {
            panic!("chat answers with chat")
        };
        assert_eq!(
            lines.iter().map(|l| l.seq).collect::<Vec<_>>(),
            vec![7, 9],
            "the hidden line is out, the rest stay in order"
        );
    }

    // Deleting is hiding plus a delete down the wire: the line leaves every face
    // here at once and the source is told which message, on which destination.
    #[test]
    fn a_deleted_line_goes_down_the_wire_by_its_id_and_is_hidden_here() {
        let mut told = told_engine();
        assert_eq!(told.handle(Command::Delete { seq: 8 }), Reply::Ok);
        let Reply::Chat { lines, .. } = told.handle(Command::Chat {
            since: 0,
            follow: false,
        }) else {
            panic!("chat answers with chat")
        };
        assert_eq!(lines.iter().map(|l| l.seq).collect::<Vec<_>>(), vec![7, 9]);
        assert!(
            matches!(
                told.handle(Command::Delete { seq: 99 }),
                Reply::Error { .. }
            ),
            "a line the engine no longer has cannot be deleted from here"
        );
    }

    // A face says a line: it goes up the wire to that chat, the engine keeps
    // no copy (the platform hands it back), and a refusal is a sentence.
    #[test]
    fn a_line_said_goes_up_the_wire_to_the_chat_named() {
        let mut told = told_engine();
        assert_eq!(
            told.handle(Command::Say {
                body: "valeu!".into(),
                channel: Some("tw".into()),
            }),
            Reply::Ok
        );
        let Reply::Chat { lines, .. } = told.handle(Command::Chat {
            since: 0,
            follow: false,
        }) else {
            panic!("chat answers with chat")
        };
        assert_eq!(lines.len(), 3, "no copy here: {lines:?}");
        assert_eq!(
            told.chat.lock().expect("chat").take_outgoing(),
            vec![crate::app::wire::Up::Say(crate::app::wire::Say {
                body: "valeu!".into(),
                channel: Some("tw".into()),
            })]
        );
        assert!(matches!(
            told.handle(Command::Say {
                body: "a\nb".into(),
                channel: None,
            }),
            Reply::Error { .. }
        ));
    }

    // Rewording a card re-shows it only when it is the one up: rewording the
    // other one must not put anything back on the picture.
    // An engine with no app behind it has no server to name and nothing to
    // announce, and its log carries no notice from anybody.
    #[test]
    fn an_engine_with_no_app_has_no_server_no_notices_and_nothing_to_announce() {
        let mut engine = engine();
        engine.tick();
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status answers with a status")
        };
        assert_eq!(status.destinations_from, None);
        let Reply::Log { lines } = engine.handle(Command::Log) else {
            panic!("log answers with the journal")
        };
        assert!(
            lines.iter().all(|line| !line.contains("! ")),
            "no notice from nobody: {lines:?}"
        );
        let reply = engine.handle(Command::Announce { adapter: 1 });
        assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
    }
}
