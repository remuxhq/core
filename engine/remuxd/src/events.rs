//! The events, shared between the engine that keeps them and every face that
//! follows them (`remuxd_domain::app::events`), on a [`Bell`].

use std::sync::{Arc, Mutex};
use std::time::Duration;

use remuxd_domain::app::events::{Events, Since};

use crate::bell::Bell;

pub struct Followed {
    /// The events, handed to the engine, which keeps one for every change,
    /// and the bell that wakes a follower when it did.
    bell: Bell<Events>,
}

impl Followed {
    /// Numbered from `from`, the moment the engine started, as the chat is.
    pub fn starting_at(from: u64) -> Arc<Self> {
        Arc::new(Self {
            bell: Bell::new(Arc::new(Mutex::new(Events::starting_at(from)))),
        })
    }

    /// The events, for the engine to keep them in.
    pub fn events(&self) -> Arc<Mutex<Events>> {
        Arc::clone(&self.bell.held)
    }

    /// The number of the newest event.
    pub fn last(&self) -> u64 {
        self.bell.lock().last()
    }

    /// Wake every follower if anything was kept since `before`, a number
    /// from [`Self::last`]. Only then: the tick runs four times a second and
    /// a follower woken for nothing is a follower on a clock.
    pub fn ring_after(&self, before: u64) {
        if self.last() != before {
            self.bell.ring();
        }
    }

    /// What came after `since`, waiting up to `patience` for it when there
    /// is nothing yet.
    pub fn after(&self, since: u64, patience: Duration) -> Since {
        self.bell
            .after(patience, |events| {
                let now = events.since(since);
                (now.gap.is_some() || !now.events.is_empty()).then_some(now)
            })
            .unwrap_or_default()
    }

    /// How many followers are asleep on the bell.
    #[cfg(test)]
    fn asleep(&self) -> usize {
        self.bell.asleep()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remuxd_domain::app::events::Event;
    use std::time::Instant;

    // The bell itself is proved in `bell`; this is the events on it, rung
    // only when the engine kept one.
    #[test]
    fn a_follower_is_woken_when_an_event_was_kept() {
        let followed = Followed::starting_at(1);
        let waiting = Arc::clone(&followed);
        let began = Instant::now();
        let follower = std::thread::spawn(move || waiting.after(0, Duration::from_secs(10)).events);
        while followed.asleep() == 0 {
            std::thread::yield_now();
        }
        let before = followed.last();
        followed
            .events()
            .lock()
            .expect("events")
            .push(0, Event::LiveStarted);
        followed.ring_after(before);
        assert_eq!(follower.join().expect("the follower returns").len(), 1);
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "woken by its patience, after {:?}",
            began.elapsed()
        );
    }

    #[test]
    fn a_follower_behind_is_answered_without_waiting() {
        let followed = Followed::starting_at(1);
        followed
            .events()
            .lock()
            .expect("events")
            .push(0, Event::LiveStarted);
        let began = Instant::now();
        assert_eq!(followed.after(0, Duration::from_secs(10)).events.len(), 1);
        assert!(began.elapsed() < Duration::from_secs(5));
    }
}
