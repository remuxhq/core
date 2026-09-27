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
        self.live = Some(crate::history::Sampler::start(
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
            if let Err(why) = crate::history::append(path, &record) {
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
