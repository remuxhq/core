//! The events, shared between the engine that keeps them and every face that
//! follows them (`remuxd_domain::app::events`), and the bell that wakes a
//! follower when there are more.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use remuxd_domain::app::events::{Events, Since};

pub struct Followed {
    /// Handed to the engine, which keeps an event for every change.
    pub events: Arc<Mutex<Events>>,
    /// Rung when the engine kept one. Waited on with the events' own lock, so
    /// an event kept between a follower's look and its sleep still wakes it.
    rung: Condvar,
    /// How many followers are asleep on the bell right now. Counted under
    /// the events' lock, so one seen here has let go of it only by sleeping.
    asleep: AtomicUsize,
}

impl Followed {
    /// Numbered from `from`, the moment the engine started, as the chat is.
    pub fn starting_at(from: u64) -> Arc<Self> {
        Arc::new(Self {
            events: Arc::new(Mutex::new(Events::starting_at(from))),
            rung: Condvar::new(),
            asleep: AtomicUsize::new(0),
        })
    }

    /// The number of the newest event.
    pub fn last(&self) -> u64 {
        self.held().last()
    }

    /// Wake every follower if anything was kept since `before`, a number
    /// from [`Self::last`]. Only then: the tick runs four times a second and
    /// a follower woken for nothing is a follower on a clock.
    pub fn ring_after(&self, before: u64) {
        if self.last() != before {
            self.rung.notify_all();
        }
    }

    /// What came after `since`, waiting up to `patience` for it when there
    /// is nothing yet.
    pub fn after(&self, since: u64, patience: Duration) -> Since {
        let events = self.held();
        let now = events.since(since);
        if now.gap.is_some() || !now.events.is_empty() {
            return now;
        }
        self.asleep.fetch_add(1, Ordering::SeqCst);
        let (events, _) = match self.rung.wait_timeout(events, patience) {
            Ok(woken) => woken,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.asleep.fetch_sub(1, Ordering::SeqCst);
        events.since(since)
    }

    /// How many followers are asleep on the bell.
    pub fn asleep(&self) -> usize {
        self.asleep.load(Ordering::SeqCst)
    }

    /// The ring, even if a thread panicked holding it: one command's panic
    /// must not take the feed from every face.
    fn held(&self) -> MutexGuard<'_, Events> {
        match self.events.lock() {
            Ok(events) => events,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remuxd_domain::app::events::Event;
    use std::time::Instant;

    // The patience is ten seconds and the budget five, so a follower that
    // only woke on its patience fails here every time, not now and then.
    #[test]
    fn a_follower_is_woken_by_the_event_not_by_its_patience() {
        let followed = Followed::starting_at(1);
        let waiting = Arc::clone(&followed);
        let began = Instant::now();
        let follower = std::thread::spawn(move || waiting.after(0, Duration::from_secs(10)).events);
        // Until it is asleep: pushed before, it would see the event without
        // ever waiting, and the bell would go untested.
        while followed.asleep() == 0 {
            std::thread::yield_now();
        }
        let before = followed.last();
        followed
            .events
            .lock()
            .expect("events")
            .push(0, Event::LiveStarted);
        followed.ring_after(before);
        let heard = follower.join().expect("the follower returns");
        assert_eq!(heard.len(), 1);
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
            .events
            .lock()
            .expect("events")
            .push(0, Event::LiveStarted);
        let began = Instant::now();
        assert_eq!(followed.after(0, Duration::from_secs(10)).events.len(), 1);
        assert!(began.elapsed() < Duration::from_secs(5));
    }
}
