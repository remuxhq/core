//! The air: the stream and the file, the two levers on the same picture.

use super::*;

/// Where the picture and the sound go: the stream and the file. See
/// [`crate::engine::Pipeline`] for why it is a port.
pub trait Air: Send {
    /// Send the picture and the sound somewhere. The url carries the
    /// credential, which is why it is passed on every call and never held
    /// here: a key belongs to the client that knows the user, not to a
    /// decision about whether to go live.
    ///
    /// It returns a `Result` and the engine believes it. A live that could not
    /// start must not report itself on air: that is the same rule the studio
    /// has for a destination that cannot be told its title, one layer down.
    fn publish(&mut self, url: &str) -> Result<(), String>;
    /// Take the stream down. Not a `Result`: stopping cannot fail in a way
    /// anybody could act on, and a stop that refuses is worse than one that
    /// half worked.
    fn unpublish(&mut self);
    /// Send it to one door among several, named by the destination's id, so
    /// each can fail and be read on its own. The default is the one door.
    fn publish_to(&mut self, _id: i64, url: &str) -> Result<(), String> {
        self.publish(url)
    }
    /// Which doors are being sent to right now.
    fn publishing(&self) -> Vec<i64> {
        Vec::new()
    }
    /// What a door's ffmpeg last complained about, by destination id.
    fn troubles(&self) -> Vec<(i64, String)> {
        Vec::new()
    }
    /// Start writing the picture to a file in this folder, and say which file.
    ///
    /// The folder is the decision; the name is not, because naming a file
    /// after the moment it started needs a clock and this crate has none. The
    /// adapter has one, so it names the file and says what it chose.
    fn record(&mut self, into: &str) -> Result<String, String>;
    /// Stop writing. Not a `Result`, for the same reason as `unpublish`.
    fn stop_recording(&mut self);
    /// What is leaving, measured at the muxer rather than derived from these
    /// counters: see [`crate::protocol::Outgoing`]. All zeroes off air.
    fn outgoing(&self) -> crate::protocol::Outgoing {
        crate::protocol::Outgoing::default()
    }
    /// Whether the stream started by `publish` is still going. Asked on every
    /// tick while on air: a relay that hung up ends the stream on the machine,
    /// and the flag follows the stream (see [`Engine::go_live`]), so it has
    /// to be read, not remembered. An adapter that cannot tell says yes.
    fn still_publishing(&mut self) -> bool {
        true
    }
}

impl Engine {
    /// Going live and stopping close the same picture, so they are the
    /// only pair that touches `on_air`.
    ///
    /// The flag follows the stream and never leads it. An engine that
    /// reported itself on air because a button was pressed would keep
    /// reporting it after the publisher died, and every panel reading
    /// this would agree with it.
    pub(super) fn go_live(&mut self) -> Reply {
        // The operator's destination when the engine was given one, else
        // where the app says the scene goes: a signed-in engine knows the relay
        // and carries its own token, so nothing has to be typed or read out
        // of a database to start it.
        // The operator's destination when the engine was given one; else
        // every door the app hands out: the relay's one, or each armed
        // destination's own.
        let outlets = match self.destination.clone() {
            Some(url) => vec![Outlet { id: 0, url }],
            None => self.watching.outlets(),
        };
        if outlets.is_empty() {
            return Reply::Error {
                message: "this engine has nowhere to send a live: no destination is armed".into(),
            };
        }
        // A live needs a picture. Without one the pipeline would go
        // and wait for a keyframe that cannot come, holding the engine
        // for every other client while it did: the socket tests found
        // that as a six second stall. The studio has the same rule and
        // never offers Go live while the scene is down.
        if self.pipeline.flowing().frames == 0 {
            return Reply::Error {
                message: "there is no picture to send yet".into(),
            };
        }
        // And a picture of nothing is black on the air: the plan's blocker,
        // kept here too for a face that goes live without a plan.
        if crate::air::plan::empty(&self.reported()) {
            return Reply::Error {
                message: crate::air::plan::EMPTY.into(),
            };
        }
        // The platforms first, all of them or none: a live carrying
        // yesterday's title is worse than one that did not start.
        if let Err(message) = self.watching.announce_armed() {
            return Reply::Error { message };
        }
        // All of them or none: a door that refuses takes the ones already
        // open down with it, the same rule as announcing.
        for outlet in &outlets {
            if let Err(message) = self.pipeline.publish_to(outlet.id, &outlet.url) {
                self.pipeline.unpublish();
                self.watching.withdraw();
                return Reply::Error {
                    message: match self
                        .watching
                        .destinations()
                        .iter()
                        .find(|d| d.id == outlet.id)
                    {
                        Some(d) => format!("{}: {message}", d.name),
                        None => message,
                    },
                };
            }
        }
        self.status.on_air = true;
        self.status.on_air_since = Some(now());
        let names = self.watching.destinations();
        self.live = Some(crate::air::history::Sampler::start(
            now(),
            outlets
                .iter()
                .filter_map(|o| names.iter().find(|d| d.id == o.id))
                .map(|d| d.name.clone())
                .collect(),
        ));
        Reply::Ok
    }

    /// What is going out, asked of the muxer every two seconds while on air.
    pub(super) fn sample_the_live(&mut self) {
        let at = now();
        if let Some(live) = self.live.as_mut().filter(|l| l.due(at)) {
            live.sample(at, self.pipeline.outgoing());
        }
    }

    /// The live ended, however it ended: written down, with the platforms'
    /// peak, where `remux history` reads.
    fn write_down_the_live(&mut self) {
        let Some(live) = self.live.take() else {
            return;
        };
        let record = live.finish(now(), self.watching.viewers_peak());
        if let Some(path) = &self.history {
            if let Err(why) = crate::air::history::append(path, &record) {
                self.journal.note(now(), format!("! {why}"));
            }
        }
    }

    /// The stream ended without anybody asking: the relay went away, or the
    /// publisher died. Off air, as `stop_live` would leave it, and said in
    /// the log. It kept saying "on air" with the relay stopped underneath it,
    /// eight seconds and counting, because nothing ever read the stream back.
    pub(super) fn air_lapsed(&mut self) {
        if self.status.on_air && !self.pipeline.still_publishing() {
            self.pipeline.unpublish();
            self.watching.off_air();
            self.status.on_air = false;
            self.status.on_air_since = None;
            self.write_down_the_live();
            self.journal
                .note(now(), "! the stream ended on its own; off air");
        }
    }

    pub(super) fn stop_live(&mut self) -> Reply {
        self.pipeline.unpublish();
        self.watching.off_air();
        self.write_down_the_live();
        self.status.on_air = false;
        self.status.on_air_since = None;
        Reply::Ok
    }

    /// Record and Go live are two levers on the same picture: recording
    /// needs no destination and no relay, so it never consults on_air.
    pub(super) fn record_start(&mut self) -> Reply {
        let Some(folder) = self.recordings.clone() else {
            return Reply::Error {
                message: "this engine has nowhere to write a recording".into(),
            };
        };
        match self.pipeline.record(&folder) {
            // The flag follows the file, the same way `on_air` follows
            // the stream: a panel showing a running clock over a
            // recording that never started is worse than a button that
            // refused.
            Ok(_) => {
                self.status.recording = true;
                self.status.recording_since = Some(now());
                Reply::Ok
            }
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn record_stop(&mut self) -> Reply {
        self.pipeline.stop_recording();
        self.status.recording = false;
        self.status.recording_since = None;
        Reply::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::*;

    #[test]
    fn an_engine_that_captures_nothing_cannot_go_live() {
        let mut engine = engine();
        let reply = engine.handle(Command::GoLive);
        assert!(
            matches!(reply, Reply::Error { .. }),
            "nothing is plugged in, so there is nothing to send, got {reply:?}"
        );
        assert!(!engine.state().on_air);
        // Stopping what never started is not an error: a panel that lost track
        // should be able to say stop and be believed.
        assert_eq!(engine.handle(Command::Stop), Reply::Ok);
        assert!(!engine.state().on_air);
    }

    // Both motors draw an empty scene at the full rate, so frames cannot keep
    // black off the air: going live refuses what the plan says would stop it.
    #[test]
    fn an_empty_scene_cannot_go_live_though_it_has_frames() {
        let (mut engine, published) = publishing_engine(None);
        engine.handle(Command::Screen { display: 1 });
        let id = engine.state().layers[0].id.clone();
        engine.handle(Command::LayerVisible { id, on: false });
        assert_eq!(
            engine.handle(Command::GoLive),
            Reply::Error {
                message: "the scene is empty: nothing would be shared".into()
            }
        );
        assert!(!engine.state().on_air);
        assert!(published.lock().expect("published").is_empty());
    }

    #[test]
    fn recording_needs_no_destination_so_it_does_not_touch_the_air() {
        let (mut engine, _) = publishing_engine(None);
        engine.set_recordings(Some("/tmp/films".into()));
        engine.handle(Command::RecordStart);
        assert!(engine.state().recording);
        assert!(!engine.state().on_air, "recording is not going live");
        engine.handle(Command::RecordStop);
        assert!(!engine.state().recording);
    }

    #[test]
    fn recording_survives_going_live_and_coming_back_off() {
        let (mut engine, _) = publishing_engine(None);
        engine.set_recordings(Some("/tmp/films".into()));
        engine.handle(Command::RecordStart);
        engine.handle(Command::GoLive);
        engine.handle(Command::Stop);
        assert!(
            engine.state().recording,
            "stopping the live must not stop the file"
        );
    }

    // Straight to every armed destination, one door each, all of them or none:
    // the engine hands the file's outlets to the pipeline, and a door that
    // refuses takes the open ones down with it.
    #[test]
    fn going_live_sends_to_every_armed_destination_in_the_file() {
        use crate::air::destinations::{add, write, Local};
        let path =
            std::env::temp_dir().join(format!("remuxd-test-dest-{}.json", std::process::id()));
        let mut kept = Vec::new();
        add(
            &mut kept,
            "yt",
            "youtube",
            "rtmp://a.rtmp.youtube.com/live2",
            "yt-key",
        )
        .unwrap();
        add(
            &mut kept,
            "tw",
            "twitch",
            "rtmp://live.twitch.tv/app",
            "tw-key",
        )
        .unwrap();
        write(&path, &kept).unwrap();
        let (engine, published) = publishing_engine(None);
        let mut engine = engine
            .with_destination(None)
            .with_app(Box::new(Local::new(path.clone())));
        engine.handle(Command::Screen { display: 1 });
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        assert_eq!(
            *published.lock().expect("published"),
            vec![
                Some("rtmp://a.rtmp.youtube.com/live2/yt-key".to_string()),
                Some("rtmp://live.twitch.tv/app/tw-key".to_string())
            ]
        );
        let _ = std::fs::remove_file(path);
    }

    // A live that ended is one line on the history file, whichever way it ended,
    // naming where it went.
    #[test]
    fn a_live_that_ended_is_written_down() {
        let path =
            std::env::temp_dir().join(format!("remuxd-test-history-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let (engine, published) = publishing_engine(None);
        let mut engine = engine.with_history(Some(path.clone()));
        engine.handle(Command::Screen { display: 1 });
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        engine.tick();
        assert_eq!(engine.handle(Command::Stop), Reply::Ok);
        let kept = crate::air::history::read(&path);
        assert_eq!(kept.len(), 1);
        assert!(kept[0].ended >= kept[0].started);
        // ended on its own: the relay hung up
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        published.lock().expect("published").push(None);
        engine.tick();
        assert!(!engine.state().on_air);
        assert_eq!(crate::air::history::read(&path).len(), 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn going_live_sends_the_picture_to_the_destination() {
        let (mut engine, published) = publishing_engine(None);
        engine.handle(Command::Screen { display: 1 });
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        assert!(engine.state().on_air);
        assert!(
            engine.state().on_air_since.is_some(),
            "the clock starts here, on the engine's own time, so every face \
                 shows the same running time for the same live"
        );
        assert_eq!(
            *published.lock().expect("published"),
            vec![Some(DESTINATION.to_string())],
            "go live must reach the pipeline, not just flip a flag"
        );
        engine.handle(Command::Stop);
        assert_eq!(engine.state().on_air_since, None, "and stops with the air");
    }

    #[test]
    fn an_engine_with_nowhere_to_go_does_not_claim_the_air() {
        let (mut engine, published) = publishing_engine(None);
        engine.set_destination(None);
        let reply = engine.handle(Command::GoLive);
        assert!(
            matches!(reply, Reply::Error { .. }),
            "with no destination going live is an error, got {reply:?}"
        );
        assert!(!engine.state().on_air);
        assert!(published.lock().expect("published").is_empty());
    }

    // The flag follows the stream: a relay that hangs up takes "on air" with it,
    // on the next tick, and the log says so. It used to say on air for as long
    // as nobody pressed Stop.
    #[test]
    fn a_stream_that_ends_on_its_own_takes_on_air_with_it() {
        let (mut engine, published) = publishing_engine(None);
        engine.handle(Command::Screen { display: 1 });
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        engine.tick();
        assert!(
            engine.state().on_air,
            "a live that is going stays on air through a tick"
        );
        published.lock().expect("published").push(None);
        engine.tick();
        assert!(
            !engine.state().on_air,
            "the stream ended and the flag did not follow"
        );
        assert!(engine.state().on_air_since.is_none());
        let said = engine.journal.lines().join("\n");
        assert!(
            said.contains("ended on its own"),
            "the log says nothing about it: {said}"
        );
    }

    #[test]
    fn a_publisher_that_refuses_leaves_the_engine_off_air() {
        let (mut engine, _) = publishing_engine(Some("ffmpeg is not here".into()));
        let reply = engine.handle(Command::GoLive);
        assert!(
            matches!(reply, Reply::Error { .. }),
            "a live that could not start must say so, got {reply:?}"
        );
        assert!(
            !engine.state().on_air,
            "an engine that failed to publish must not report itself on air"
        );
    }

    #[test]
    fn stopping_takes_the_stream_down() {
        let (mut engine, published) = publishing_engine(None);
        engine.handle(Command::Screen { display: 1 });
        engine.handle(Command::GoLive);
        assert_eq!(engine.handle(Command::Stop), Reply::Ok);
        assert!(!engine.state().on_air);
        assert_eq!(
            *published.lock().expect("published"),
            vec![Some(DESTINATION.to_string()), None],
            "stop must take the stream down, not just flip a flag"
        );
    }

    #[test]
    fn recording_writes_a_file_and_says_so() {
        let (mut engine, _) = publishing_engine(None);
        engine.set_recordings(Some("/tmp/films".into()));
        assert_eq!(engine.handle(Command::RecordStart), Reply::Ok);
        assert!(engine.state().recording);
        assert!(!engine.state().on_air, "recording is not going live");
        assert_eq!(engine.handle(Command::RecordStop), Reply::Ok);
        assert!(!engine.state().recording);
    }

    #[test]
    fn an_engine_with_nowhere_to_write_does_not_claim_to_be_recording() {
        let (mut engine, _) = publishing_engine(None);
        engine.set_recordings(None);
        let reply = engine.handle(Command::RecordStart);
        assert!(
            matches!(reply, Reply::Error { .. }),
            "with no folder recording is an error, got {reply:?}"
        );
        assert!(!engine.state().recording);
    }

    #[test]
    fn a_recorder_that_refuses_leaves_the_clock_stopped() {
        let (mut engine, _) = publishing_engine(Some("the disk is full".into()));
        engine.set_recordings(Some("/tmp/films".into()));
        let reply = engine.handle(Command::RecordStart);
        assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
        assert!(
            !engine.state().recording,
            "a clock ticking over a file that was never opened is a lie"
        );
    }

    #[test]
    fn going_live_with_no_picture_is_refused_at_once() {
        let (mut engine, published) = publishing_engine(None);
        // `Wrote` reports nothing flowing until something is captured, which
        // is what a cold engine looks like.
        let reply = engine.handle(Command::GoLive);
        assert!(
            matches!(reply, Reply::Error { .. }),
            "there is nothing to send, got {reply:?}"
        );
        assert!(!engine.state().on_air);
        assert!(
            published.lock().expect("published").is_empty(),
            "it must not even reach the pipeline"
        );
    }

    // A destination handed over after the fact is the destination: the refusal
    // moves from "nowhere to send" to "no picture yet".
    #[test]
    fn a_destination_set_later_is_the_one_go_live_uses() {
        let mut engine = engine();
        let Reply::Error { message } = engine.handle(Command::GoLive) else {
            panic!("nowhere to send is an error")
        };
        assert!(message.contains("nowhere"), "{message}");
        engine.set_destination(Some("rtmp://hub/scene".into()));
        let Reply::Error { message } = engine.handle(Command::GoLive) else {
            panic!("no picture is an error")
        };
        assert!(message.contains("no picture"), "{message}");
    }

    // The clock the engine stamps with is the real one: a live that started
    // reports a moment in this decade, not zero.
    #[test]
    fn the_engine_stamps_with_a_real_clock() {
        let (mut engine, _) = publishing_engine(None);
        engine.handle(Command::Screen { display: 1 });
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        let since = engine.state().on_air_since.expect("on air since");
        assert!(
            since > 1_700_000_000,
            "{since} is not a moment on the real clock"
        );
    }

    /// An app that names the relay, the way a signed-in one does.
    struct NamesTheRelay;

    impl Watching for NamesTheRelay {
        fn reachable(&self) -> bool {
            true
        }
        fn destinations(&self) -> Vec<Destination> {
            Vec::new()
        }
        fn viewers(&self) -> Option<u32> {
            None
        }
        fn arm(&self, _: i64, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn sandbox(&self, _: i64, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn retitle(&self, _: i64, _: Option<&str>, _: Option<&str>) -> Result<(), String> {
            Ok(())
        }
        fn announce(&self, _: i64) -> Result<(), String> {
            Ok(())
        }
        fn disconnect(&self, _: i64) -> Result<(), String> {
            Ok(())
        }
        fn categorize(&self, _: i64, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn search_categories(&self, _: i64, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn scene(&self) -> Option<String> {
            Some("rtmp://hub:1935/scene?user=remux&pass=token".into())
        }
    }

    // An engine started with no destination and signed in to an app goes live
    // where the app says the scene goes: nobody read a key out of a database
    // to start it.
    #[test]
    fn an_engine_with_no_destination_of_its_own_goes_live_where_the_app_says() {
        let published: Published = Default::default();
        let pipeline = Wrote {
            published: published.clone(),
            ..Default::default()
        };
        let mut engine = Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_app(Box::new(NamesTheRelay));
        engine.handle(Command::Screen { display: 1 });
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        assert_eq!(
            *published.lock().expect("published"),
            vec![Some(
                "rtmp://hub:1935/scene?user=remux&pass=token".to_string()
            )]
        );
    }

    // And one started with a destination keeps it: what the operator said wins
    // over what the app would have said.
    #[test]
    fn a_destination_of_its_own_outranks_the_app_s() {
        let (mut engine, published) = publishing_engine(None);
        engine = engine.with_app(Box::new(NamesTheRelay));
        engine.handle(Command::Screen { display: 1 });
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        assert_eq!(
            *published.lock().expect("published"),
            vec![Some(DESTINATION.to_string())]
        );
    }
}
