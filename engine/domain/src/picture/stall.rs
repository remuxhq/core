//! A camera that stopped handing over frames, told from its count.
//!
//! Cameras alone: a screen or a window hands over a frame only when the
//! picture changes (`protocol::Flowing::captured`), so a still one is a
//! quiet one and not a broken one. A camera delivers at its rate whatever it
//! sees, a covered lens included, so a count that stops is a camera that
//! stopped. Pure; the engine feeds it the counts on its tick.

use std::collections::BTreeMap;

use crate::app::events::Event;

/// The cameras being watched, by layer id.
#[derive(Debug, Default)]
pub struct Stalls {
    seen: BTreeMap<String, Watched>,
}

#[derive(Debug)]
struct Watched {
    captured: u64,
    flat: u32,
    stalled: bool,
}

impl Stalls {
    /// Ticks with no new frame before a camera is stalled: three seconds at
    /// the tick's four a second. A camera runs at thirty frames a second, so
    /// three seconds without one is ninety missing, not jitter. Chosen, not
    /// yet measured against a camera unplugged under a live.
    pub const FLAT_TICKS: u32 = 12;

    /// One tick's counts, for the cameras shown now: what changed. A camera
    /// no longer in `counts` is forgotten, hidden or removed, and says
    /// nothing.
    pub fn observe(&mut self, counts: impl IntoIterator<Item = (String, u64)>) -> Vec<Event> {
        let mut events = Vec::new();
        let mut now = BTreeMap::new();
        for (id, captured) in counts {
            // The first tick of a camera only sees where its count is.
            let Some(mut watched) = self.seen.remove(&id) else {
                now.insert(
                    id,
                    Watched {
                        captured,
                        flat: 0,
                        stalled: false,
                    },
                );
                continue;
            };
            if captured != watched.captured {
                watched.captured = captured;
                watched.flat = 0;
                if watched.stalled {
                    watched.stalled = false;
                    events.push(Event::LayerFlowing { id: id.clone() });
                }
            } else {
                watched.flat += 1;
                if !watched.stalled && watched.flat >= Self::FLAT_TICKS {
                    watched.stalled = true;
                    events.push(Event::LayerStalled { id: id.clone() });
                }
            }
            now.insert(id, watched);
        }
        self.seen = now;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(stalls: &mut Stalls, captured: u64) -> Vec<Event> {
        stalls.observe([("face".to_string(), captured)])
    }

    #[test]
    fn a_camera_whose_count_stops_is_stalled_once_and_flowing_when_it_moves() {
        let mut stalls = Stalls::default();
        let mut said = Vec::new();
        for _ in 0..Stalls::FLAT_TICKS {
            said.extend(tick(&mut stalls, 90));
        }
        assert_eq!(said, vec![], "the first tick only sees it");
        assert_eq!(
            tick(&mut stalls, 90),
            vec![Event::LayerStalled { id: "face".into() }]
        );
        assert_eq!(tick(&mut stalls, 90), vec![], "said once");
        assert_eq!(
            tick(&mut stalls, 91),
            vec![Event::LayerFlowing { id: "face".into() }]
        );
    }

    #[test]
    fn a_camera_that_keeps_counting_says_nothing() {
        let mut stalls = Stalls::default();
        for n in 0..100 {
            assert_eq!(tick(&mut stalls, n), vec![]);
        }
    }

    #[test]
    fn a_camera_that_never_delivered_is_stalled_too() {
        let mut stalls = Stalls::default();
        let said: Vec<Event> = (0..=Stalls::FLAT_TICKS)
            .flat_map(|_| tick(&mut stalls, 0))
            .collect();
        assert_eq!(said, vec![Event::LayerStalled { id: "face".into() }]);
    }

    #[test]
    fn a_camera_hidden_and_shown_again_starts_over() {
        let mut stalls = Stalls::default();
        for _ in 0..Stalls::FLAT_TICKS - 1 {
            tick(&mut stalls, 5);
        }
        assert_eq!(stalls.observe(Vec::<(String, u64)>::new()), vec![]);
        for _ in 0..Stalls::FLAT_TICKS {
            assert_eq!(tick(&mut stalls, 5), vec![], "counted from its return");
        }
    }
}
