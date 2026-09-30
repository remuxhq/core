//! The chat as the engine holds it, off the wire (`crate::app::wire`): the ring
//! every face reads. Where a chat of one's own comes from is
//! `config::chat_url`. Pure; the socket is `remuxd::wire`.

use std::collections::{BTreeSet, VecDeque};

use crate::app::wire::{Delete, Line, Say, Up};
use crate::protocol::{ChatLine, CHAT_LINES};

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

    /// A line the operator says, up the wire to the platform's chat. No copy
    /// is kept: the platform hands it back down like anybody's line. Refused
    /// with no wire up, since the queue would send it minutes out of its
    /// moment, and with a control character in it: a newline is a second
    /// line, and on Twitch's IRC a second command.
    pub fn say(&mut self, body: &str, channel: Option<String>) -> Result<(), String> {
        if !self.reachable {
            return Err("no chat wire to say it on".into());
        }
        let body = body.trim();
        if body.is_empty() {
            return Err("say needs the words".into());
        }
        if body.chars().any(char::is_control) {
            return Err("a line of chat is one line, with no control characters".into());
        }
        self.outgoing.push_back(Up::Say(Say {
            body: body.into(),
            channel: channel.filter(|channel| !channel.is_empty()),
        }));
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

    // What the operator says goes up the wire and nowhere else: the platform
    // hands it back down as a line like anybody's, so the feed keeps no copy
    // of its own to show twice.
    #[test]
    fn a_line_said_goes_up_the_wire_once_and_comes_back_only_from_the_platform() {
        let mut feed = Feed {
            reachable: true,
            ..Feed::default()
        };
        feed.say("hello chat", None).unwrap();
        feed.say("oi", Some("main".into())).unwrap();
        assert!(feed.since(0).is_empty(), "no copy here");
        assert_eq!(
            feed.take_outgoing(),
            vec![
                Up::Say(Say {
                    body: "hello chat".into(),
                    channel: None
                }),
                Up::Say(Say {
                    body: "oi".into(),
                    channel: Some("main".into())
                }),
            ]
        );
    }

    // A line said to nobody would wait in the queue and go up minutes later,
    // out of its moment; one with a newline in it is two lines on a platform
    // that reads lines (Twitch's IRC), the second one a command.
    #[test]
    fn a_line_is_refused_with_no_wire_no_words_or_a_break_in_it() {
        let mut feed = Feed::default();
        assert!(feed.say("hi", None).unwrap_err().contains("no chat wire"));
        feed.reachable = true;
        assert!(feed.say("   ", None).is_err());
        assert!(feed.say("hi\r\nPRIVMSG #x :pwned", None).is_err());
        assert!(feed.say("hi\u{7}", None).is_err());
        assert!(feed.take_outgoing().is_empty());
        feed.say("  trimmed  ", Some("".into())).unwrap();
        assert_eq!(
            feed.take_outgoing(),
            vec![Up::Say(Say {
                body: "trimmed".into(),
                channel: None
            })],
            "an empty channel is every chat"
        );
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
