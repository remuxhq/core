//! The wire: what the engine and whoever serves it say to each other, over
//! one WebSocket, one JSON object per text frame, keyed by what it is.
//!
//! Down, from the server: `{"line":{...}}`, `{"event":{...}}`, `{"history":[...]}`,
//! `{"destinations":[...]}`, `{"viewers":{...}}`, `{"categories":{...}}`,
//! `{"notice":{...}}`. Up, from the engine: `{"open":"control"}`,
//! `{"arm":{...}}`, `{"retitle":{...}}`, `{"delete":{...}}`, `{"say":{...}}` and the rest of
//! [`Up`]. The web serves it for an account (`/wire`); a chat source of
//! one's own serves the `line` half and nothing else. The engine never knows
//! a platform; it knows this. `docs/wire.md` is the contract.
//!
//! Reading is lenient on purpose: a frame the engine does not know is
//! dropped, a row with less in it than expected is read with defaults, and
//! nothing here trusts a server to be well-behaved.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::protocol::{Category, ChatLine, Destination, Found};

/// One line as the wire spells it: the platform's own message id, the
/// destination it came from, who said it and what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Line {
    pub id: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub channel: String,
    pub from: String,
    pub body: String,
}

impl From<Line> for ChatLine {
    fn from(line: Line) -> Self {
        ChatLine {
            seq: 0,
            from: line.from,
            body: line.body,
            platform: line.platform,
            id: line.id,
            channel: line.channel,
        }
    }
}

/// Something that happened in a platform's chat beyond a line: who, where, what
/// they wrote with it (a resub's message, a Super Chat's comment), and `what`,
/// flat beside the rest and keyed by `type`. `id` is the platform's own; the
/// line that carries a tip's message has the same one. `badges` say who `from`
/// is in words every platform shares (broadcaster, moderator, vip, member,
/// verified, first); `reply` is the id of the message it answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Happening {
    #[serde(flatten)]
    pub what: What,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub badges: Vec<String>,
    #[serde(default)]
    pub reply: String,
}

/// What happened, in words every platform shares, and a platform's own kind for
/// the rest (`custom`, named `<platform>.<what>`), so a bridge says something new
/// without this engine learning it first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum What {
    /// A chat message: the wire carries those as lines, so one never comes
    /// down as an event.
    Chat,
    /// A subscription or a membership, new or renewed.
    Sub {
        #[serde(default)]
        months: u32,
        #[serde(default)]
        tier: String,
    },
    /// Subscriptions or memberships given; `to` is empty for a community gift.
    Gift {
        #[serde(default)]
        count: u32,
        #[serde(default)]
        tier: String,
        #[serde(default)]
        to: String,
    },
    /// Money, or bits: `amount` as the platform shows it, `micros` in
    /// millionths of `currency`, to add up and rank.
    Tip {
        #[serde(default)]
        amount: String,
        #[serde(default)]
        currency: String,
        #[serde(default)]
        micros: u64,
    },
    Raid {
        #[serde(default)]
        viewers: u32,
    },
    Follow,
    /// A message taken down on its platform: `target` is its id.
    Deleted {
        #[serde(default)]
        target: String,
    },
    /// A viewer banned (`seconds` 0) or timed out.
    Banned {
        #[serde(default)]
        user: String,
        #[serde(default)]
        seconds: u32,
    },
    /// The whole chat cleared.
    Cleared,
    Custom {
        #[serde(default)]
        name: String,
        #[serde(default)]
        fields: BTreeMap<String, String>,
    },
}

/// Which message to take out of the platform's chat, on which destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Delete {
    pub id: String,
    pub channel: String,
}

/// A line the operator says in the platform's chat. `channel` is a line's
/// own (`Line::channel`), the chat it goes to; none is every chat the server
/// reads. The platform hands it back down as a `line` like anybody's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Say {
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

/// One destination on the wire, up: by the id the server gave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Adapter {
    pub adapter: i64,
}

/// What the engine says up the wire. One JSON object each, keyed by the
/// verb; the server answers with what changed (`destinations`, a `notice`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Up {
    /// `control` or `chat`: which half of the wire to start, once each.
    Open(String),
    Arm {
        adapter: i64,
        on: bool,
    },
    Sandbox {
        adapter: i64,
        on: bool,
    },
    Retitle {
        adapter: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    Announce(Adapter),
    Disconnect(Adapter),
    Categorize {
        adapter: i64,
        id: String,
        name: String,
    },
    Search {
        adapter: i64,
        query: String,
    },
    Delete(Delete),
    Say(Say),
    /// Every twenty-five seconds; a server closes a wire that says nothing.
    Heartbeat(Value),
}

impl Up {
    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("a frame serialises")
    }
}

/// One WebSocket the engine keeps, and the halves it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    /// The web's, for an account: the control half, and the chat half
    /// unless a chat wire of one's own carries it.
    Account { chat: bool },
    /// A chat wire of one's own (`remux chat url`): lines down, deletes and
    /// says up.
    Own,
}

impl Wire {
    /// What it says once it is open.
    pub fn opens(self) -> Vec<Up> {
        match self {
            Wire::Account { chat } => {
                let mut halves = vec![Up::Open("control".into())];
                if chat {
                    halves.push(Up::Open("chat".into()));
                }
                halves
            }
            Wire::Own => Vec::new(),
        }
    }

    /// Whether the account's verbs go up it and its destinations come down.
    pub fn control(self) -> bool {
        matches!(self, Wire::Account { .. })
    }

    /// Whether the chat comes down it and deletes and says go up.
    pub fn chat(self) -> bool {
        matches!(self, Wire::Account { chat: true } | Wire::Own)
    }

    /// Whether a frame is heard: the chat from the wire that carries it,
    /// the account's state from the account's. A notice from either.
    pub fn carries(self, down: &Down) -> bool {
        match down {
            Down::Line(_) | Down::Event(_) | Down::History(_) => self.chat(),
            Down::Notice(_) => true,
            _ => self.control(),
        }
    }
}

/// The wires to keep. The account keeps its destinations whoever serves the
/// chat; a chat wire of one's own takes the chat half and nothing else.
pub fn wires(account: bool, own_chat: bool) -> Vec<Wire> {
    let mut wires = Vec::new();
    if account {
        wires.push(Wire::Account { chat: !own_chat });
    }
    if own_chat {
        wires.push(Wire::Own);
    }
    wires
}

/// The web's wire for an account: the web's own address, the socket's token
/// on the query. `wss` under `https`, because a token in the clear is a
/// session anybody on the network can take.
pub fn url(base: &str, token: &str) -> String {
    let ws = base
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    format!("{ws}/wire/websocket?token={token}&vsn=1.0.0")
}

/// What the wire says down, as the engine keeps it.
#[derive(Debug, Clone, PartialEq)]
pub enum Down {
    Line(ChatLine),
    /// A sub, a gift, a tip, a raid, a moderator's hand: anything but a line.
    Event(Happening),
    /// Everything said before this engine opened, oldest first.
    History(Vec<ChatLine>),
    /// The whole list, replacing what was known.
    Destinations(Vec<Destination>),
    /// `{total, answered, peak}`.
    Viewers(Watchers),
    /// What a platform offers for what a face typed.
    Categories(Found),
    /// A refusal or a confirmation, worded for the log.
    Notice(String),
}

impl Down {
    /// One text frame, read; `None` for anything the engine does not know
    /// or cannot read, which is dropped rather than argued with.
    pub fn read(text: &str) -> Option<Down> {
        let value: Value = serde_json::from_str(text).ok()?;
        let object = value.as_object()?;
        let (key, payload) = object.iter().next()?;
        match key.as_str() {
            "line" => Some(Down::Line(line(payload)?)),
            "event" => Some(Down::Event(happening(payload)?)),
            "history" => Some(Down::History(history(payload)?)),
            "destinations" => Some(Down::Destinations(rows(payload)?)),
            "viewers" => Some(Down::Viewers(serde_json::from_value(payload.clone()).ok()?)),
            "categories" => Some(Down::Categories(found(payload))),
            "notice" => Some(Down::Notice(notice(payload))),
            // a verb the server refused: worded for the log like a notice
            "error" => Some(Down::Notice(format!("! {}", refusal(payload)))),
            _ => None,
        }
    }
}

/// `{"reason": "..."}`, a bare string, or whatever it was.
fn refusal(payload: &Value) -> String {
    payload
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| payload.as_str().map(str::to_string))
        .unwrap_or_else(|| payload.to_string())
}

fn line(payload: &Value) -> Option<ChatLine> {
    let said: Line = serde_json::from_value(payload.clone()).ok()?;
    if said.body.is_empty() {
        return None;
    }
    Some(said.into())
}

/// An event, unless it is one this engine cannot read or a chat message, which
/// is a line's to carry.
fn happening(payload: &Value) -> Option<Happening> {
    let happened: Happening = serde_json::from_value(payload.clone()).ok()?;
    (happened.what != What::Chat).then_some(happened)
}

/// The last `CHAT_LINES` of them, oldest first.
fn history(payload: &Value) -> Option<Vec<ChatLine>> {
    let list = payload.as_array()?;
    let mut kept: Vec<ChatLine> = list
        .iter()
        .rev()
        .filter_map(line)
        .take(crate::protocol::CHAT_LINES)
        .collect();
    kept.reverse();
    Some(kept)
}

/// How many are watching, and whether anybody answered.
///
/// `answered` is the whole point: a destination that could not be asked is
/// left out of the sum, never counted as zero, so "nobody is watching" and
/// "nobody told us" stay different all the way out to a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Watchers {
    pub answered: bool,
    /// Only meaningful while `answered`; the app sends it either way.
    pub total: Option<u64>,
    /// The most at once since the live began. Zero is "none yet".
    pub peak: u64,
}

/// `{items: [...]}`, and a row that cannot be read is left out rather than
/// taking the list down with it.
fn rows(payload: &Value) -> Option<Vec<Destination>> {
    let list = payload.as_array()?;
    Some(list.iter().filter_map(destination).collect())
}

/// A destination as the app sends it. Every field defaults, because the app
/// leaves out what it has nothing to say about, and `id` is the one thing a
/// row is nothing without.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Row {
    id: Option<i64>,
    name: String,
    platform: String,
    status: String,
    armed: bool,
    sandbox: bool,
    connected: bool,
    account: Option<String>,
    category: Option<String>,
    category_id: Option<String>,
    title: Option<String>,
    description: Option<String>,
    /// Read as a `u64` and narrowed, so a count too big for a face is
    /// "nobody told us" rather than a row nobody sees. Absent and null are
    /// the same answer and neither becomes a zero.
    viewers: Option<u64>,
    viewers_peak: Option<u64>,
    /// `{reason, since}` on the wire; the reason is what a face shows.
    trouble: Option<Trouble>,
    channel: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Trouble {
    reason: Option<String>,
}

fn destination(value: &Value) -> Option<Destination> {
    let row: Row = serde_json::from_value(value.clone()).ok()?;
    Some(Destination {
        id: row.id?,
        name: row.name,
        platform: row.platform,
        status: row.status,
        armed: row.armed,
        sandbox: row.sandbox,
        connected: row.connected,
        account: row.account,
        category: row.category,
        category_id: row.category_id,
        viewers: row.viewers.and_then(|n| u32::try_from(n).ok()),
        viewers_peak: row.viewers_peak.and_then(|n| u32::try_from(n).ok()),
        trouble: row.trouble.and_then(|t| t.reason),
        title: row.title,
        description: row.description,
        channel: row.channel,
    })
}

/// `{id, query, items: [{id, name}]}`, kept whole so a face can tell which
/// question this answers and show it only to its own.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Search {
    id: i64,
    query: String,
    items: Vec<Value>,
}

fn found(payload: &Value) -> Found {
    let search: Search = serde_json::from_value(payload.clone()).unwrap_or_default();
    Found {
        adapter: search.id,
        query: search.query,
        items: search
            .items
            .iter()
            .filter_map(|one| serde_json::from_value::<Category>(one.clone()).ok())
            .collect(),
    }
}

/// A refusal the app says out loud, because the engine only pushes and never
/// reads a reply.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Note {
    about: String,
    text: String,
    fine: bool,
}

/// How a notice goes into the log: `! tico: the token was refused` for a
/// refusal, `tico: told the title: remux live` for something that went as
/// asked. The mark is what a face reads to colour it, and a success used to
/// arrive marked like an alarm or not at all.
fn notice(payload: &Value) -> String {
    let note: Note = serde_json::from_value(payload.clone()).unwrap_or_default();
    let words = if note.about.is_empty() {
        note.text
    } else {
        format!("{}: {}", note.about, note.text)
    };
    if note.fine {
        words
    } else {
        format!("! {words}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn read(key: &str, payload: Value) -> Option<Down> {
        Down::read(&json!({ key: payload }).to_string())
    }

    #[test]
    fn a_chat_wire_of_ones_own_takes_the_chat_and_leaves_the_account_its_destinations() {
        assert_eq!(
            wires(true, true),
            vec![Wire::Account { chat: false }, Wire::Own]
        );
        assert_eq!(wires(true, false), vec![Wire::Account { chat: true }]);
        assert_eq!(wires(false, true), vec![Wire::Own]);
        assert_eq!(wires(false, false), vec![]);
    }

    #[test]
    fn a_wire_opens_the_halves_it_carries() {
        let open = |half: &str| Up::Open(half.into());
        assert_eq!(
            Wire::Account { chat: true }.opens(),
            vec![open("control"), open("chat")]
        );
        assert_eq!(Wire::Account { chat: false }.opens(), vec![open("control")]);
        assert_eq!(Wire::Own.opens(), vec![]);
        assert!(Wire::Account { chat: false }.control() && !Wire::Account { chat: false }.chat());
        assert!(Wire::Own.chat() && !Wire::Own.control());
    }

    #[test]
    fn a_wire_is_heard_only_on_the_halves_it_carries() {
        let line = read("line", json!({"id": "m1", "from": "ana", "body": "hi"})).unwrap();
        let rows = read("destinations", json!([])).unwrap();
        let notice = Down::Notice("told the title".into());
        assert!(Wire::Own.carries(&line));
        assert!(
            !Wire::Own.carries(&rows),
            "a bridge never names the destinations"
        );
        assert!(
            !Wire::Account { chat: false }.carries(&line),
            "the chat is the bridge's"
        );
        assert!(Wire::Account { chat: false }.carries(&rows));
        assert!(Wire::Account { chat: true }.carries(&line));
        assert!(Wire::Own.carries(&notice) && Wire::Account { chat: false }.carries(&notice));
    }

    #[test]
    fn the_accounts_wire_is_secure_under_https() {
        assert_eq!(
            url("https://remux.live", "t"),
            "wss://remux.live/wire/websocket?token=t&vsn=1.0.0"
        );
        assert_eq!(
            url("http://localhost:4700", "t"),
            "ws://localhost:4700/wire/websocket?token=t&vsn=1.0.0"
        );
    }

    #[test]
    fn a_refusal_lands_in_the_log_as_an_alarm() {
        assert_eq!(
            read("error", json!({"reason": "no plan on this account"})),
            Some(Down::Notice("! no plan on this account".into()))
        );
        assert_eq!(
            read("error", json!("the channel closed")),
            Some(Down::Notice("! the channel closed".into()))
        );
    }

    #[test]
    fn an_up_frame_is_one_object_keyed_by_the_verb() {
        assert_eq!(Up::Open("control".into()).encode(), r#"{"open":"control"}"#);
        assert_eq!(
            Up::Arm {
                adapter: 2,
                on: true
            }
            .encode(),
            r#"{"arm":{"adapter":2,"on":true}}"#
        );
        assert_eq!(
            Up::Retitle {
                adapter: 2,
                title: Some("t".into()),
                description: None
            }
            .encode(),
            r#"{"retitle":{"adapter":2,"title":"t"}}"#
        );
        assert_eq!(
            Up::Delete(Delete {
                id: "m1".into(),
                channel: "main".into()
            })
            .encode(),
            r#"{"delete":{"id":"m1","channel":"main"}}"#
        );
        assert_eq!(
            Up::Say(Say {
                body: "oi".into(),
                channel: Some("main".into())
            })
            .encode(),
            r#"{"say":{"body":"oi","channel":"main"}}"#
        );
        assert_eq!(
            Up::Say(Say {
                body: "oi".into(),
                channel: None
            })
            .encode(),
            r#"{"say":{"body":"oi"}}"#,
            "no channel is every chat the server reads"
        );
    }

    #[test]
    fn a_line_and_a_history_are_read_and_a_line_with_no_words_is_not() {
        let Some(Down::Line(line)) = Down::read(
            r#"{"line":{"id":"m1","platform":"twitch","channel":"main","from":"ana","body":"hi"}}"#,
        ) else {
            panic!("no line")
        };
        assert_eq!(
            (line.from.as_str(), line.body.as_str(), line.seq),
            ("ana", "hi", 0)
        );
        assert_eq!(
            read("line", json!({"id": "m1", "from": "a", "body": ""})),
            None
        );
        let many: Vec<Value> = (0..crate::protocol::CHAT_LINES + 10)
            .map(|n| json!({"id": n.to_string(), "from": "a", "body": format!("line {n}")}))
            .collect();
        let Some(Down::History(lines)) = read("history", json!(many)) else {
            panic!("no history")
        };
        assert_eq!(lines.len(), crate::protocol::CHAT_LINES);
        assert_eq!(lines[0].body, "line 10", "the oldest kept, first");
        assert_eq!(Down::read("not json"), None);
        assert_eq!(Down::read(r#"{"a":1,"b":2}"#), None, "one key, or nothing");
    }

    #[test]
    fn an_event_this_engine_does_not_know_changes_nothing() {
        assert_eq!(read("opened", json!("control")), None);
        assert_eq!(read("", json!({})), None);
    }

    #[test]
    fn the_destinations_arrive_under_the_key_the_channel_uses() {
        let said = read(
            "destinations",
            json!([{"id": 1, "name": "tico", "platform": "twitch",
                             "status": "live", "armed": true}]),
        );
        let Some(Down::Destinations(rows)) = said else {
            panic!("no destinations: {said:?}")
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "tico");
        assert!(rows[0].armed);
        assert!(!rows[0].sandbox, "what the app left out is its default");
    }

    #[test]
    fn a_payload_without_the_list_says_nothing_rather_than_emptying_it() {
        // The last list known stands: a panel with the app halfway through a
        // deploy keeps its rows.
        assert_eq!(read("destinations", json!({})), None);
        assert_eq!(read("destinations", json!("soon")), None);
    }

    #[test]
    fn a_row_the_app_malformed_is_left_out_and_the_rest_arrive() {
        let said = read(
            "destinations",
            json!([
                {"name": "no id here"},
                {"id": "seven", "name": "an id that is not a number"},
                {"id": 2, "name": "teco", "platform": "youtube", "status": "off",
                 "armed": false, "armed_at": "a field this engine has never heard of"}
            ]),
        );
        let Some(Down::Destinations(rows)) = said else {
            panic!("no destinations")
        };
        assert_eq!(rows.len(), 1, "only the row that is one");
        assert_eq!(rows[0].id, 2);
    }

    // The first public live: the YouTube quota gone two hours in, every call
    // refused, the viewer count quiet and the chat stopped, and no face said
    // why. The app puts what the platform last refused on the row; the engine
    // carries it to every face as it is.
    #[test]
    fn a_destination_carries_what_its_platform_refuses_and_nothing_when_it_does_not() {
        let troubled = json!([{
            "id": 6, "name": "Youtube", "platform": "youtube", "status": "live",
            "armed": true, "sandbox": false, "connected": true,
            "trouble": {"reason": "youtube said 403 quotaExceeded",
                        "since": "2026-09-06T22:31:00Z"}
        }]);
        let Some(Down::Destinations(rows)) = read("destinations", troubled) else {
            panic!("no destinations")
        };
        assert_eq!(
            rows[0].trouble.as_deref(),
            Some("youtube said 403 quotaExceeded")
        );

        let fine = json!([{
            "id": 6, "name": "Youtube", "platform": "youtube", "status": "live",
            "armed": true, "trouble": null
        }]);
        let Some(Down::Destinations(rows)) = read("destinations", fine) else {
            panic!("no destinations")
        };
        assert_eq!(rows[0].trouble, None);
    }

    #[test]
    fn a_count_nobody_answered_is_not_a_zero() {
        let said = read(
            "viewers",
            json!({"total": 0, "answered": false, "peak": 12}),
        );
        assert_eq!(
            said,
            Some(Down::Viewers(Watchers {
                answered: false,
                total: Some(0),
                peak: 12
            }))
        );
        let said = read("viewers", json!({"total": 6, "answered": true}));
        assert_eq!(
            said,
            Some(Down::Viewers(Watchers {
                answered: true,
                total: Some(6),
                peak: 0
            }))
        );
    }

    #[test]
    fn a_viewers_payload_that_is_not_one_says_nothing() {
        assert_eq!(read("viewers", json!({"answered": "yes"})), None);
        assert_eq!(read("viewers", json!("six")), None);
    }

    #[test]
    fn a_destination_that_answered_a_count_too_big_for_a_face_answered_nothing() {
        let said = read(
            "destinations",
            json!([{"id": 1, "name": "tico", "viewers": 5_000_000_000u64}]),
        );
        let Some(Down::Destinations(rows)) = said else {
            panic!("no destinations")
        };
        assert_eq!(rows[0].viewers, None);
    }

    #[test]
    fn a_search_is_kept_whole_so_a_face_knows_which_question_it_answers() {
        let Some(Down::Categories(found)) = read(
            "categories",
            json!({"id": 3, "query": "soft", "items": [
                {"id": "509670", "name": "Software and Game Development"},
                {"name": "an offer with no id"}
            ]}),
        ) else {
            panic!("no categories")
        };
        assert_eq!(found.adapter, 3);
        assert_eq!(found.query, "soft");
        assert_eq!(found.items.len(), 1);
        assert_eq!(found.items[0].id, "509670");
    }

    #[test]
    fn a_search_that_found_nothing_is_still_an_answer() {
        // Empty and "no answer yet" are different: a face showing the last
        // list for a word that matched nothing is a face lying quietly.
        let Some(Down::Categories(found)) = read("categories", json!({"id": 3, "query": "zzz"}))
        else {
            panic!("no categories")
        };
        assert!(found.items.is_empty());
        assert_eq!(found.query, "zzz");
    }

    // Update on the panel did nothing visible: the app told the platform and
    // the only trace was a log line marked like an alarm, or none.
    #[test]
    fn a_notice_is_marked_as_an_alarm_unless_the_app_says_it_went_fine() {
        assert_eq!(
            read(
                "notice",
                json!({"about": "tico", "text": "the token was refused"})
            ),
            Some(Down::Notice("! tico: the token was refused".into()))
        );
        assert_eq!(
            read(
                "notice",
                json!({"about": "tico", "text": "told the title: remux live", "fine": true})
            ),
            Some(Down::Notice("tico: told the title: remux live".into()))
        );
        assert_eq!(
            read("notice", json!({"text": "nobody is on air"})),
            Some(Down::Notice("! nobody is on air".into()))
        );
        assert_eq!(
            read("notice", json!("not an object")),
            Some(Down::Notice("! ".into())),
            "a notice nobody can read is still an alarm"
        );
    }

    // A bridge says more than lines: subs, gifts, tips, raids, and what a
    // moderator took down. Each comes as one event, flat, keyed by its type.
    #[test]
    fn an_event_is_read_by_its_type_with_its_own_fields() {
        let Some(Down::Event(sub)) = Down::read(
            r#"{"event":{"type":"sub","id":"u1","platform":"twitch","channel":"kartths","from":"Ana","body":"six months!","badges":["member"],"reply":"","months":6,"tier":"1000"}}"#,
        ) else {
            panic!("no event")
        };
        assert_eq!(
            sub.what,
            What::Sub {
                months: 6,
                tier: "1000".into()
            }
        );
        assert_eq!(
            (sub.from.as_str(), sub.body.as_str()),
            ("Ana", "six months!")
        );
        assert_eq!(sub.badges, vec!["member".to_string()]);
        let Some(Down::Event(tip)) = read(
            "event",
            json!({"type": "tip", "id": "s1", "from": "Bob", "body": "gg",
                   "amount": "$5.00", "currency": "USD", "micros": 5_000_000}),
        ) else {
            panic!("no tip")
        };
        assert_eq!(
            tip.what,
            What::Tip {
                amount: "$5.00".into(),
                currency: "USD".into(),
                micros: 5_000_000
            }
        );
        let Some(Down::Event(own)) = read(
            "event",
            json!({"type": "custom", "id": "w1", "name": "twitch.viewermilestone",
                   "fields": {"category": "watch-streak", "value": "5"}}),
        ) else {
            panic!("no custom")
        };
        let What::Custom { name, fields } = own.what else {
            panic!("not custom")
        };
        assert_eq!(name, "twitch.viewermilestone");
        assert_eq!(fields.get("value").map(String::as_str), Some("5"));
    }

    #[test]
    fn an_event_with_less_in_it_is_read_with_defaults() {
        let Some(Down::Event(cleared)) = read("event", json!({"type": "cleared"})) else {
            panic!("no event")
        };
        assert_eq!(cleared.what, What::Cleared);
        assert!(cleared.id.is_empty() && cleared.badges.is_empty());
        let Some(Down::Event(banned)) = read("event", json!({"type": "banned", "user": "troll"}))
        else {
            panic!("no ban")
        };
        assert_eq!(
            banned.what,
            What::Banned {
                user: "troll".into(),
                seconds: 0
            },
            "no seconds is for good"
        );
    }

    #[test]
    fn an_event_this_engine_cannot_read_is_dropped_and_chat_is_a_line() {
        assert_eq!(read("event", json!({"type": "hype-train"})), None);
        assert_eq!(read("event", json!({"id": "no type"})), None);
        assert_eq!(read("event", json!("sub")), None);
        assert_eq!(
            read(
                "event",
                json!({"type": "chat", "id": "m1", "from": "a", "body": "hi"})
            ),
            None,
            "a chat message comes down as a line"
        );
    }

    #[test]
    fn an_event_is_heard_on_the_wire_that_carries_the_chat() {
        let event = read("event", json!({"type": "raid", "viewers": 42})).unwrap();
        assert!(Wire::Own.carries(&event));
        assert!(Wire::Account { chat: true }.carries(&event));
        assert!(!Wire::Account { chat: false }.carries(&event));
    }
}
