//! Something every face may follow, and the bell that wakes a follower when
//! it changed: the chat's feed, the engine's events.
//!
//! A follower looks and falls asleep under the followed thing's own lock, so
//! a change landing between its look and its sleep cannot slip past it: the
//! change needs the lock, which the follower lets go of only by sleeping.
//! A bell with a lock of its own had that gap, and a chat line that fell in
//! it waited out the follower's patience, a second, before anybody saw it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

pub struct Bell<T> {
    /// The followed thing, shared with whoever changes it.
    pub held: Arc<Mutex<T>>,
    rung: Condvar,
    /// How many followers are asleep on the bell right now. Counted under
    /// the lock, so one seen here has let go of it only by sleeping.
    asleep: AtomicUsize,
}

impl<T> Bell<T> {
    pub fn new(held: Arc<Mutex<T>>) -> Self {
        Self {
            held,
            rung: Condvar::new(),
            asleep: AtomicUsize::new(0),
        }
    }

    /// Wake every follower. Ring after the change, with or without the lock.
    pub fn ring(&self) {
        self.rung.notify_all();
    }

    /// What `look` finds, waiting up to `patience` for it when it finds
    /// nothing yet; `None` when it still finds nothing.
    pub fn after<R>(&self, patience: Duration, look: impl Fn(&T) -> Option<R>) -> Option<R> {
        let held = self.lock();
        if let Some(found) = look(&held) {
            return Some(found);
        }
        self.asleep.fetch_add(1, Ordering::SeqCst);
        let (held, _) = match self.rung.wait_timeout(held, patience) {
            Ok(woken) => woken,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.asleep.fetch_sub(1, Ordering::SeqCst);
        look(&held)
    }

    /// How many followers are asleep on the bell.
    pub fn asleep(&self) -> usize {
        self.asleep.load(Ordering::SeqCst)
    }

    /// The followed thing, even if a thread panicked holding it: one
    /// command's panic must not take it from every face.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        match self.held.lock() {
            Ok(held) => held,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn some(n: &u32) -> Option<u32> {
        (*n > 0).then_some(*n)
    }

    // The patience is ten seconds and the budget five, so a follower that
    // only woke on its patience fails here every time, not now and then.
    #[test]
    fn a_follower_is_woken_by_the_bell_not_by_its_patience() {
        let bell = Arc::new(Bell::new(Arc::new(Mutex::new(0))));
        let waiting = Arc::clone(&bell);
        let began = Instant::now();
        let follower = std::thread::spawn(move || waiting.after(Duration::from_secs(10), some));
        // Until it is asleep: changed before, it would see the change without
        // ever waiting, and the bell would go untested.
        while bell.asleep() == 0 {
            std::thread::yield_now();
        }
        *bell.lock() = 1;
        bell.ring();
        assert_eq!(follower.join().expect("the follower returns"), Some(1));
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "woken by its patience, after {:?}",
            began.elapsed()
        );
    }

    #[test]
    fn a_follower_with_something_to_find_does_not_wait() {
        let bell = Bell::new(Arc::new(Mutex::new(1)));
        let began = Instant::now();
        assert_eq!(bell.after(Duration::from_secs(10), some), Some(1));
        assert!(began.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_follower_that_finds_nothing_in_its_patience_says_so() {
        let bell = Bell::new(Arc::new(Mutex::new(0)));
        assert_eq!(bell.after(Duration::from_millis(1), some), None);
        assert_eq!(bell.asleep(), 0);
    }
}
