//! What the picture says when it is not showing the screen.
//!
//! Three of them, and they are not decoration. A live that opens on a blank
//! screen looks broken; one that opens on "Starting soon" with music under it
//! looks like a live that has not started. The difference is the whole reason
//! these exist, and it is why the recording carries them too: the recording is
//! the record of the broadcast, never something the audience did not see.

use serde::{Deserialize, Serialize};

/// Which card is up, if any.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Card {
    /// No card. The screen and the camera are the picture.
    #[default]
    Live,
    StartingSoon,
    BackInAMoment,
    /// Not a card the operator chooses: it is what is drawn when nothing is
    /// behind the picture, so a viewer can tell a deliberate blank from a
    /// stream that died. The text is fixed for the same reason.
    NothingShared,
}

pub const STARTING_SOON: &str = "Starting soon";
pub const BACK_IN_A_MOMENT: &str = "Back in a moment";
/// Fixed, and deliberately not customisable: this is the engine telling the
/// truth about itself, not the operator addressing an audience.
pub const NOTHING_SHARED: &str = "No content shared";

/// What the operator can change about the cards.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Words {
    pub starting: String,
    pub back: String,
}

impl Default for Words {
    fn default() -> Self {
        Self {
            starting: STARTING_SOON.into(),
            back: BACK_IN_A_MOMENT.into(),
        }
    }
}

impl Words {
    /// The line a card shows. Blank text falls back rather than showing an
    /// empty card: somebody who cleared the field wants the default back, not
    /// a black rectangle in front of an audience.
    pub fn line(&self, card: Card) -> &str {
        let chosen = match card {
            Card::Live => return "",
            Card::StartingSoon => self.starting.trim(),
            Card::BackInAMoment => self.back.trim(),
            Card::NothingShared => return NOTHING_SHARED,
        };
        if chosen.is_empty() {
            match card {
                Card::StartingSoon => STARTING_SOON,
                _ => BACK_IN_A_MOMENT,
            }
        } else {
            chosen
        }
    }
}

/// The card's background, as red, green and blue from 0 to 1.
///
/// Very dark rather than black. Pure black on a stream reads as signal loss,
/// and an encoder given a perfectly flat frame produces a suspiciously tiny
/// one; a hint of colour keeps the picture looking like a picture.
pub const BACKGROUND: (f64, f64, f64) = (0.055, 0.06, 0.075);

/// How large the line is, as a share of the picture's height. A card is read
/// from a phone across a room, not from a desk.
pub const TEXT_HEIGHT_SHARE: f64 = 0.11;

/// How large "No content shared" is, as a share of the height.
///
/// Half the size of a card's own words, because it is the engine saying
/// something about itself rather than the operator addressing an audience.
pub const NOTHING_SHARED_HEIGHT_SHARE: f64 = 0.05;

/// The amber a card glows, which is the studio's accent. The countdown is
/// written in it too.
pub const GLOW: (f64, f64, f64) = (0.949, 0.710, 0.267);

/// How large the countdown is, as a share of the height.
///
/// Larger than the words above it, and in the accent, because after the first
/// read the words are furniture and the number is the only thing anybody is
/// still looking at.
pub const CLOCK_HEIGHT_SHARE: f64 = 0.157;

/// Where the words sit when a clock is under them, and where the clock sits,
/// as a share of the height measured from the middle. Positive is up.
pub const WORDS_ABOVE_CLOCK: f64 = 0.055;
pub const CLOCK_BELOW_MIDDLE: f64 = 0.12;

/// How far across the picture the glow reaches, as a share of the width.
pub const GLOW_REACH: f64 = 0.55;

/// A breath, from 0 to 1, at this many milliseconds.
///
/// A card sits on somebody's stream for ten minutes at a time. Perfectly still,
/// it reads as a frozen picture, and a viewer who thinks the stream has died
/// leaves. Something has to move, and it has to be slow enough that nobody
/// looks at it: the period is about fourteen seconds, which is a breath.
#[must_use]
pub fn breath(millis: f64) -> f64 {
    0.5 + 0.5 * (millis / 2200.0).sin()
}

/// How strong a card's glow is at that moment. Never off, never much.
#[must_use]
pub fn glow_alpha(millis: f64) -> f64 {
    0.05 + 0.05 * breath(millis)
}

/// How visible "No content shared" is at that moment. Fainter than a card's
/// words on purpose: it is not addressed to anybody.
#[must_use]
pub fn nothing_shared_alpha(millis: f64) -> f64 {
    0.35 + 0.25 * breath(millis)
}

/// The line a card shows while a countdown is running.
///
/// The words and the clock, not the clock alone: "Starting soon" with 2:14
/// under it tells somebody arriving what they are waiting for, where a bare
/// 2:14 is a number on a dark screen. They are one line and not two so that
/// the compositor has nothing to lay out.
pub fn counting_line(words: &str, remaining_seconds: i64) -> String {
    format!("{words}  {}", countdown(remaining_seconds))
}

/// The countdown, as a person reads a clock.
///
/// Minutes and seconds, and never negative: a countdown that has run out shows
/// zero and stays there. The operator is the one who starts the live, so
/// reaching zero is a cue and not an event.
pub fn countdown(remaining_seconds: i64) -> String {
    let remaining = remaining_seconds.max(0);
    format!("{}:{:02}", remaining / 60, remaining % 60)
}

/// How many seconds are left, given when the countdown was started, how long
/// it was set for, and what time it is now. All in seconds since anything, as
/// long as it is the same anything.
pub fn remaining(started_at: f64, length: f64, now: f64) -> i64 {
    (length - (now - started_at)).ceil() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_breath_stays_between_nothing_and_everything() {
        for millis in (0..40_000).step_by(137) {
            let it = breath(f64::from(millis));
            assert!((0.0..=1.0).contains(&it), "{it} at {millis}");
        }
    }

    #[test]
    fn the_glow_is_always_there_and_never_loud() {
        for millis in (0..40_000).step_by(311) {
            let it = glow_alpha(f64::from(millis));
            assert!(it >= 0.05, "a card that stops glowing looks frozen: {it}");
            assert!(it <= 0.10, "a glow anybody notices is a distraction: {it}");
        }
    }

    #[test]
    fn the_breath_actually_moves() {
        // A constant would satisfy the bounds above and defeat the point.
        let over: Vec<f64> = (0..14)
            .map(|second| glow_alpha(f64::from(second) * 1000.0))
            .collect();
        let low = over.iter().copied().fold(f64::MAX, f64::min);
        let high = over.iter().copied().fold(f64::MIN, f64::max);
        assert!(high - low > 0.04, "it barely moved: {low} to {high}");
    }

    #[test]
    fn a_card_says_what_the_operator_wrote() {
        let words = Words {
            starting: "Chegando já".into(),
            back: "Já volto".into(),
        };
        assert_eq!(words.line(Card::StartingSoon), "Chegando já");
        assert_eq!(words.line(Card::BackInAMoment), "Já volto");
    }

    // Somebody who cleared the field wants the default back, not a black
    // rectangle in front of an audience.
    #[test]
    fn an_empty_line_falls_back_instead_of_showing_nothing() {
        let words = Words {
            starting: "   ".into(),
            back: String::new(),
        };
        assert_eq!(words.line(Card::StartingSoon), STARTING_SOON);
        assert_eq!(words.line(Card::BackInAMoment), BACK_IN_A_MOMENT);
    }

    // This one is the engine telling the truth about itself, not the operator
    // addressing an audience, so it is not theirs to change.
    #[test]
    fn nothing_shared_says_the_same_thing_whatever_the_operator_wrote() {
        let words = Words {
            starting: "anything".into(),
            back: "anything".into(),
        };
        assert_eq!(words.line(Card::NothingShared), NOTHING_SHARED);
    }

    #[test]
    fn the_live_picture_has_no_line() {
        assert_eq!(Words::default().line(Card::Live), "");
    }

    #[test]
    fn the_countdown_reads_like_a_clock() {
        assert_eq!(countdown(180), "3:00");
        assert_eq!(countdown(9), "0:09");
        assert_eq!(countdown(61), "1:01");
        assert_eq!(countdown(600), "10:00");
    }

    // A countdown that has run out sits at zero. The operator starts the live,
    // so reaching zero is a cue and not an event.
    #[test]
    fn a_countdown_that_ran_out_sits_at_zero() {
        assert_eq!(countdown(0), "0:00");
        assert_eq!(countdown(-5), "0:00");
        assert_eq!(countdown(-600), "0:00");
    }

    #[test]
    fn what_is_left_counts_down_in_real_time() {
        // three minutes, started at t=100
        assert_eq!(remaining(100.0, 180.0, 100.0), 180);
        assert_eq!(remaining(100.0, 180.0, 130.0), 150);
        assert_eq!(remaining(100.0, 180.0, 280.0), 0);
        assert_eq!(
            remaining(100.0, 180.0, 400.0),
            -120,
            "past zero is the caller's to floor"
        );
    }

    // Rounding up, not down: a countdown showing 0:00 for the last whole
    // second before it ends looks stuck.
    #[test]
    fn a_fraction_of_a_second_still_counts_as_a_second() {
        assert_eq!(remaining(0.0, 10.0, 9.5), 1);
        assert_eq!(remaining(0.0, 10.0, 9.99), 1);
        assert_eq!(remaining(0.0, 10.0, 10.0), 0);
    }

    #[test]
    fn the_background_is_dark_but_never_flat_black() {
        let (r, g, b) = BACKGROUND;
        for channel in [r, g, b] {
            assert!(channel > 0.0, "flat black reads as signal loss");
            assert!(channel < 0.15, "a card is not a light background");
        }
    }

    #[test]
    fn the_line_carries_the_words_and_the_clock_together() {
        assert_eq!(counting_line("Starting soon", 134), "Starting soon  2:14");
        assert_eq!(counting_line("Chegando já", 5), "Chegando já  0:05");
    }

    // A bare number on a dark screen tells somebody arriving nothing about
    // what they are waiting for.
    #[test]
    fn the_clock_never_appears_without_its_words() {
        assert!(counting_line("Starting soon", 0).starts_with("Starting soon"));
    }
}
