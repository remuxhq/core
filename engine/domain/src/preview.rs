//! The frame a panel draws, in memory both processes can see.
//!
//! The panel used to ask for a JPEG once per frame: encode, base64, JSON,
//! parse, decode, and a picture that arrived late and sometimes not at all.
//! Here the engine renders the preview straight into shared memory and the
//! panel maps the same memory read-only. Nothing is encoded and nothing is
//! copied down a socket; what crosses the socket is the name of the region and
//! its shape, once.
//!
//! **Why not `IOSurface`.** That was the first answer and it does not work:
//! macOS no longer lets one process find another's surface by id.
//! A spike proved it, with a negative control beside it. Sending the surface as a mach port needs
//! XPC or a launchd service, and this daemon is neither.
//!
//! **What it costs in privacy.** A named region is reachable by any process of
//! the same user. So is a window on their screen, and this is a picture of
//! their screen; it carries no credential and nothing that is not already on
//! the display. It is created `0600` so it is that user and nobody else.
//!
//! This module is the layout and the rules, with no operating system in it, so
//! the arithmetic that both sides depend on is tested rather than trusted.

use serde::{Deserialize, Serialize};

/// How many frames the ring holds.
///
/// Three: one the writer is filling, one the reader may be drawing, and one
/// spare so those two never meet. Two would let a reader that stalls for a
/// single frame be overwritten mid-draw.
pub const SLOTS: u32 = 3;

/// How many pictures the region carries: the scene, the self-view, the screen.
/// They are separate because each is a different picture, not a crop of
/// another: a card up makes the scene the card while the other two go on
/// showing what they are pointed at.
pub const RINGS: usize = 3;

/// What a panel is told, once, so it can map the region and read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Preview {
    /// The POSIX shared memory name, leading slash and all.
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Bytes per row. Not `width * 4`: a renderer is allowed to want its rows
    /// aligned, and a reader that assumes otherwise draws a picture that
    /// shears.
    pub stride: u32,
    pub slots: u32,
}

impl Preview {
    /// Where a slot of the scene's ring begins, counted from the start.
    ///
    /// **This arithmetic does not change.** The self-view's ring was added
    /// after it, not woven into it, so the scene keeps the same bytes, the same offsets and the same sequence
    /// than it had. `the_scene_keeps_its_bytes` is the test that says so.
    #[must_use]
    pub fn offset(&self, slot: u32) -> usize {
        HEADER + (slot as usize) * self.frame()
    }

    /// Where a slot of the self-view's ring begins. After the scene's, always.
    #[must_use]
    pub fn camera_offset(&self, slot: u32) -> usize {
        self.ring_offset(1, slot)
    }

    /// And the screen's, after the self-view's.
    #[must_use]
    pub fn screen_offset(&self, slot: u32) -> usize {
        self.ring_offset(2, slot)
    }

    fn ring_offset(&self, ring: usize, slot: u32) -> usize {
        HEADER + (ring * self.slots as usize + slot as usize) * self.frame()
    }

    /// One frame's bytes.
    #[must_use]
    pub fn frame(&self) -> usize {
        (self.stride as usize) * (self.height as usize)
    }

    /// The whole region: the scene's ring, then the self-view's, then the
    /// screen's.
    #[must_use]
    pub fn size(&self) -> usize {
        HEADER + RINGS * (self.slots as usize) * self.frame()
    }

    /// Which slot a sequence number lands in.
    #[must_use]
    pub fn slot(&self, sequence: u64) -> u32 {
        if self.slots == 0 {
            return 0;
        }
        (sequence % u64::from(self.slots)) as u32
    }
}

/// The bytes before the first frame: a magic number, then one sequence per
/// plane.
///
/// Sixty-four so the sequences sit alone in a cache line and a reader spinning
/// on one never shares it with pixels being written.
pub const HEADER: usize = 64;

/// `RMXP`, so a region left over from something else is not mistaken for one of
/// ours and drawn as noise.
pub const MAGIC: u32 = 0x524d_5850;

/// Where the scene's newest sequence number lives, in bytes from the start.
pub const SEQUENCE_AT: usize = 8;

/// And the self-view's, eight bytes after it, in space the header already had.
///
/// A self-view travelling as a JPEG over the socket costs, measured, 141 ms a frame and 125 kilobytes of base64, which is a slideshow rather than a
/// picture of somebody's face. Here it is written into the same mapping as the
/// scene, on the same clock, and read the same way.
pub const CAMERA_SEQUENCE_AT: usize = 16;

/// And the screen's, after that. The screen card is not a crop of the scene:
/// while a card is up the scene *is* the card, and the card that says "screen"
/// still has to show the screen.
pub const SCREEN_SEQUENCE_AT: usize = 24;

/// Both sequences have to fit in the header, before the first pixel. Checked
/// while compiling rather than while running: it is arithmetic over constants,
/// and a test would only ever prove what the compiler already knows.
const _: () = assert!(SCREEN_SEQUENCE_AT + 8 <= HEADER);

/// The slot the writer fills next, given how many frames it has published.
///
/// The frame it is about to fill will be published as `published + 1`, and a
/// reader that sees that number opens [`Preview::slot`] of it, so the two
/// have to agree on this one line. They did not, for a day and a half: the
/// writer filled `published % slots`, which is the slot a reader of
/// `published` was drawing from, so every frame was two behind and, whenever
/// the two overlapped, torn across the middle while [`intact`] said it was
/// whole.
#[must_use]
pub fn fills(published: u64, slots: u32) -> u32 {
    if slots == 0 {
        return 0;
    }
    ((published + 1) % u64::from(slots)) as u32
}

/// Whether a frame read at this sequence is still intact.
///
/// The writer publishes a sequence after filling a slot, so a reader takes the
/// sequence, reads that slot, and asks this. The frame survived if the writer
/// has not come back round to that slot in the meantime. It is back on it
/// one publish *before* the lap shows in the count: having published
/// `read_at + slots - 1`, it is filling `read_at + slots`, which is the
/// reader's slot again ([`fills`]). So the count may move `slots - 2` and
/// no further; with three slots, one frame of movement is fine and two is
/// the writer under the reader.
///
/// A reader that loses this race simply draws the next frame instead. There is
/// no lock and there is nothing to block: the writer is a live pipeline and
/// must never wait for a window.
#[must_use]
pub fn intact(read_at: u64, now: u64, slots: u32) -> bool {
    now.saturating_sub(read_at) + 1 < u64::from(slots)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> Preview {
        Preview {
            name: "/remux.preview".into(),
            width: 960,
            height: 540,
            stride: 960 * 4,
            slots: SLOTS,
        }
    }

    // The writer and the reader, driven together: what the writer fills is
    // what a reader of the next published number opens, and never what a
    // reader of the current one is still drawing.
    #[test]
    fn the_reader_opens_the_slot_the_writer_just_filled_and_never_the_one_it_fills_next() {
        let it = shape();
        for published in 0..12u64 {
            let filling = fills(published, it.slots);
            assert_eq!(
                filling,
                it.slot(published + 1),
                "a reader of {} must open what was filled for it",
                published + 1
            );
            assert_ne!(
                filling,
                it.slot(published),
                "a reader still drawing {published} must not be under the writer"
            );
        }
    }

    #[test]
    fn the_slots_do_not_overlap_and_the_region_holds_them_all() {
        let it = shape();
        assert_eq!(it.frame(), 960 * 540 * 4);
        assert_eq!(it.offset(0), HEADER);
        assert_eq!(it.offset(1), HEADER + it.frame());
        assert_eq!(it.size(), HEADER + 9 * it.frame(), "three rings of three");
        assert_eq!(it.offset(it.slots - 1) + it.frame(), it.camera_offset(0));
    }

    // The scene's arithmetic is pinned here so that adding anything beside it cannot move it by a byte.
    #[test]
    fn the_scene_keeps_its_bytes() {
        let it = shape();
        assert_eq!(it.offset(0), HEADER);
        assert_eq!(it.offset(1), HEADER + it.frame());
        assert_eq!(it.offset(2), HEADER + 2 * it.frame());
        assert_eq!(SEQUENCE_AT, 8);
    }

    // Each ring is added *after* the ones before it, never woven in, so the
    // scene's picture keeps its bytes whatever is put beside it. This walks
    // every slot of every ring and says they are one unbroken run.
    #[test]
    fn the_rings_are_laid_end_to_end_and_never_on_each_other() {
        let it = shape();
        let mut expected = HEADER;
        let rings = [
            Preview::offset as fn(&Preview, u32) -> usize,
            Preview::camera_offset,
            Preview::screen_offset,
        ];
        for ring in rings {
            for slot in 0..it.slots {
                assert_eq!(ring(&it, slot), expected, "a ring moved");
                expected += it.frame();
            }
        }
        assert_eq!(expected, it.size(), "the region ends where the rings do");
        let says = [SEQUENCE_AT, CAMERA_SEQUENCE_AT, SCREEN_SEQUENCE_AT];
        assert_eq!(
            says.iter().collect::<std::collections::HashSet<_>>().len(),
            says.len(),
            "two rings share a sequence"
        );
    }

    #[test]
    fn a_padded_stride_is_obeyed_rather_than_assumed() {
        // A renderer that wants 64-byte rows gives a stride wider than the
        // pixels. A reader that computed `width * 4` would shear every frame.
        let padded = Preview {
            stride: 960 * 4 + 128,
            ..shape()
        };
        assert_eq!(padded.frame(), (960 * 4 + 128) * 540);
        assert_ne!(padded.frame(), shape().frame());
    }

    #[test]
    fn sequences_go_round_the_ring() {
        let it = shape();
        assert_eq!(it.slot(0), 0);
        assert_eq!(it.slot(1), 1);
        assert_eq!(it.slot(2), 2);
        assert_eq!(it.slot(3), 0);
        assert_eq!(it.slot(u64::MAX), ((u64::MAX % 3) as u32));
    }

    #[test]
    fn a_frame_survives_the_writer_moving_on_once_but_not_coming_back_round() {
        assert!(intact(10, 10, SLOTS), "nothing moved");
        assert!(intact(10, 11, SLOTS), "one frame on is a different slot");
        assert!(
            !intact(10, 12, SLOTS),
            "two on: 12 is published, so 13 is being filled, and 13 is our slot"
        );
        assert!(!intact(10, 13, SLOTS), "a full lap is the same slot again");
        assert!(!intact(10, 99, SLOTS));
    }

    // The two rules against each other: a reader is told it lost exactly
    // when the writer, at any count since the read, filled the slot it read.
    #[test]
    fn intact_is_false_exactly_when_the_writer_has_filled_the_slot_that_was_read() {
        let it = shape();
        for read_at in 1..8u64 {
            for now in read_at..read_at + 6 {
                let lost = (read_at..=now).any(|count| fills(count, it.slots) == it.slot(read_at));
                assert_eq!(intact(read_at, now, it.slots), !lost, "{read_at} at {now}");
            }
        }
    }

    #[test]
    fn a_reader_that_starts_ahead_of_the_writer_is_not_told_it_lost() {
        // Can only happen if the two disagree about the clock, and the answer
        // is to draw rather than to spin.
        assert!(intact(20, 10, SLOTS));
    }

    #[test]
    fn no_slots_is_not_a_division_by_zero() {
        let none = Preview {
            slots: 0,
            ..shape()
        };
        assert_eq!(none.slot(7), 0);
    }
}
