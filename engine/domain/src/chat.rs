//! The chat as the engine holds it, off the wire (`crate::wire`): the ring
//! every face reads. Where a chat of one's own comes from is
//! `config::chat_url`. Pure; the socket is `remuxd::wire`.

use std::collections::{BTreeSet, VecDeque};

use crate::protocol::{ChatLine, CHAT_LINES};
use crate::wire::{Delete, Line, Up};

/// The last of the chat, as the engine holds it for every face: numbered as
/// it arrived, bounded, with what a face took off (`hide`) and what waits to
/// be sent back down the wire (a delete).
#[derive(Debug)]
pub struct Feed {
    lines: VecDeque<ChatLine>,
    hidden: BTreeSet<u64>,
    next_seq: u64,
    outgoing: VecDeque<Up>,
    /// Whether the wire is up. Reported beside the lines: an empty list from
    /// a source that is down and one from a quiet room look identical.
    pub reachable: bool,
}

impl Feed {
    /// Numbers start at `from` and only rise: an engine restarted under a
    /// face that remembers the last number it saw hands over bigger ones.
    pub fn starting_at(from: u64) -> Self {
        Self {
            lines: VecDeque::new(),
            hidden: BTreeSet::new(),
            next_seq: from.max(1),
            outgoing: VecDeque::new(),
            reachable: false,
        }
    }

    /// A line arrived: numbered, kept, the oldest dropped past the cap.
    pub fn push(&mut self, line: Line) -> u64 {
        self.push_line(ChatLine::from(line))
    }

    /// The same, for a line already read off the wire.
    pub fn push_line(&mut self, mut kept: ChatLine) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        kept.seq = seq;
        if self.lines.len() >= CHAT_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(kept);
        seq
    }

    /// What came after `since`, less what a face took off.
    pub fn since(&self, since: u64) -> Vec<ChatLine> {
        self.lines
            .iter()
            .filter(|line| line.seq > since && !self.hidden.contains(&line.seq))
            .cloned()
            .collect()
    }

    /// The newest number handed out, for a follower to ask from.
    pub fn last_seq(&self) -> u64 {
        self.lines.back().map_or(0, |line| line.seq)
    }

    pub fn hide(&mut self, seq: u64) {
        self.hidden.insert(seq);
    }

    /// Off every face here, and a delete on the wire for the platform. A line
    /// the engine no longer holds cannot be deleted from here.
    pub fn delete(&mut self, seq: u64) -> Result<(), String> {
        let line = self
            .lines
            .iter()
            .find(|line| line.seq == seq)
            .ok_or_else(|| format!("no line {seq} in the chat"))?;
        self.outgoing.push_back(Up::Delete(Delete {
            id: line.id.clone(),
            channel: line.channel.clone(),
        }));
        self.hidden.insert(seq);
        Ok(())
    }

    /// What waits to go down the wire; the socket takes it.
    pub fn take_outgoing(&mut self) -> Vec<Up> {
        self.outgoing.drain(..).collect()
    }
}

impl Default for Feed {
    fn default() -> Self {
        Self::starting_at(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, body: &str) -> Line {
        Line {
            id: id.into(),
            platform: "twitch".into(),
            channel: "main".into(),
            from: "ana".into(),
            body: body.into(),
        }
    }

    #[test]
    fn the_feed_numbers_lines_hands_over_what_is_new_and_hides_what_a_face_took_off() {
        let mut feed = Feed::starting_at(7);
        assert_eq!(feed.push(line("m1", "hello")), 7);
        assert_eq!(feed.push(line("m2", "gg")), 8);
        assert_eq!(feed.push(line("m3", "first!")), 9);
        assert_eq!(
            feed.since(8).iter().map(|l| l.seq).collect::<Vec<_>>(),
            vec![9]
        );
        feed.hide(8);
        assert_eq!(
            feed.since(0).iter().map(|l| l.seq).collect::<Vec<_>>(),
            vec![7, 9]
        );
        assert_eq!(feed.last_seq(), 9);
    }

    #[test]
    fn a_delete_leaves_every_face_and_goes_down_the_wire_by_the_platforms_id() {
        let mut feed = Feed::default();
        feed.push(line("m1", "spam"));
        feed.delete(1).unwrap();
        assert!(feed.since(0).is_empty());
        assert_eq!(
            feed.take_outgoing(),
            vec![Up::Delete(Delete {
                id: "m1".into(),
                channel: "main".into()
            })]
        );
        assert!(feed.take_outgoing().is_empty(), "taken once");
        assert!(feed.delete(99).is_err());
    }

    #[test]
    fn the_ring_is_bounded() {
        let mut feed = Feed::default();
        for n in 0..(CHAT_LINES + 5) {
            feed.push(line(&n.to_string(), "x"));
        }
        assert_eq!(feed.since(0).len(), CHAT_LINES);
        assert_eq!(feed.since(0)[0].seq, 6);
    }
}
