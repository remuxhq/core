//! The app: what the engine asks the web on a face's behalf, and the chat off its wire.

use super::*;

impl Engine {
    /// The app owns the column; this asks, and says so when it cannot.
    pub(super) fn arm(&mut self, adapter: i64, on: bool) -> Reply {
        match self.watching.arm(adapter, on) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn disconnect(&mut self, adapter: i64) -> Reply {
        match self.watching.disconnect(adapter) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn categorize(&mut self, adapter: i64, id: &str, name: &str) -> Reply {
        match self.watching.categorize(adapter, id, name) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn search_categories(&mut self, adapter: i64, query: &str) -> Reply {
        match self.watching.search_categories(adapter, query) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn sandbox(&mut self, adapter: i64, on: bool) -> Reply {
        match self.watching.sandbox(adapter, on) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn retitle(
        &mut self,
        adapter: i64,
        title: Option<String>,
        description: Option<String>,
    ) -> Reply {
        match self
            .watching
            .retitle(adapter, title.as_deref(), description.as_deref())
        {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn announce(&mut self, adapter: i64) -> Reply {
        match self.watching.announce(adapter) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }

    /// The chat is the feed's, off whichever wire the daemon opened; the
    /// engine only reads it and takes lines off it.
    pub(super) fn chat(&mut self, since: u64) -> Reply {
        let feed = self.chat.lock().expect("chat");
        Reply::Chat {
            reachable: feed.reachable,
            lines: feed.since(since),
        }
    }

    pub(super) fn hide(&mut self, seq: u64) -> Reply {
        self.chat.lock().expect("chat").hide(seq);
        Reply::Ok
    }

    /// Off every face here at once, and a delete down the wire for the
    /// platform. A line the engine no longer has cannot be deleted from here.
    pub(super) fn delete_chat(&mut self, seq: u64) -> Reply {
        match self.chat.lock().expect("chat").delete(seq) {
            Ok(()) => Reply::Ok,
            Err(message) => Reply::Error { message },
        }
    }
}
