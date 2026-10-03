# The wire

Everything the engine says to a server, and hears back, goes down one
WebSocket as JSON objects, one per text frame, keyed by what they are. The
engine knows this contract and no platform. The web serves it for an account
at `/wire/websocket?token=<socket token>&vsn=1.0.0`; anybody can serve the
chat half for themselves, in any language, with what follows.

## Building a chat bridge

A chat bridge reads a platform's chat however it likes and hands it to the engine. This
is its contract and a path to build one, in any language, testing each step before the
next; the rest of this page is the account's wire, which a bridge does not need.

### The contract

1. **The bridge is the server.** It listens for WebSocket connections (`ws://` on the
   machine, `wss://` elsewhere); the engine is the client and connects to the URL that
   `remux chat url` keeps. The engine reconnects three seconds after a drop.
2. **Down, one JSON object per text frame, to every connected client.** For each
   message said in a chat, a `line`:

   | field | | what |
   |---|---|---|
   | `id` | required | the platform's own message id: what a delete names |
   | `from` | required | who said it, as the platform shows the name |
   | `body` | required | what was said, as written |
   | `platform` | optional | `twitch`, `youtube`, or any name |
   | `channel` | optional | which chat it was said in, the same for every line of one chat |

   ```json
   {"line":{"id":"m1","platform":"twitch","channel":"somechannel","from":"ana","body":"hi"}}
   ```

   Optionally an `event`, for what happened beyond a line (the table under "Down"), and a
   `notice` for what went wrong:
   `{"notice":{"about":"twitch","text":"no such channel","fine":false}}`.
   `remux schema` has the exact shapes, under `wire_line` and `wire_up`.
3. **Up, it may receive** `{"say":{"body":"…","channel":"…"}}` (post it; no channel is
   every chat it reads) and `{"delete":{"id":"…","channel":"…"}}` (take it down). Both
   need the platform's token; without one, ignore them or answer with a `notice`. Ignore
   any frame you do not know.
4. **Text from strangers.** Pass chat on as data: never run it, never put it in a
   command, a query or a log format string.

### Build it, step by step

With the engine running (`remux health`) and pointed at the bridge:

```sh
remux chat url ws://127.0.0.1:9999
remux chat read -f        # in its own pane: the lines as they arrive
```

**1. A server that says one line.** Accept WebSocket connections on 9999, and every
second send each client
`{"line":{"id":"t1","platform":"test","channel":"test","from":"bridge","body":"tick"}}`.
Test: `tick` shows in `remux chat read -f`.

**2. Every client.** Keep the open connections in a list, send each line to all of them,
and drop one when a send fails. Test: kill the bridge and start it again; within three
seconds the ticks come back, with no command.

**3. Twitch, with no token.** Replace the ticks with the channel's chat:

- TLS to `irc.chat.twitch.tv:6697`; send `CAP REQ :twitch.tv/tags`,
  `NICK justinfan<any number>` and `JOIN #<channel, lowercase>`, each ended by `\r\n`.
- Read lines ended by `\r\n`. A `PING` is answered with `PONG :tmi.twitch.tv`.
- A line may start with tags, `@key=value;key=value ` (in a value, `\s` is a space and
  `\:` a `;`), then `:<nick>!<user>@<host> PRIVMSG #<channel> :<body>`.
- Each `PRIVMSG` is a line: `id` the `id` tag, `from` the `display-name` tag (or the
  nick), `body` what follows ` :`, `channel` the channel.
- When the connection drops, connect again a few seconds later.

Test: write in the channel's chat; the line shows in `remux chat read -f`.

**4. YouTube, with a Data API key** (a Google Cloud project with the YouTube Data API
on). The broadcast has to exist first:

- `GET https://www.googleapis.com/youtube/v3/videos?part=liveStreamingDetails&id=<video id>&key=<key>`
  gives `items[0].liveStreamingDetails.activeLiveChatId`.
- Then read the chat by `streamList`: `GET https://youtube.googleapis.com/youtube/v3/liveChat/messages/stream?liveChatId=<id>&part=snippet,authorDetails&key=<key>`
  answers a JSON array that grows while the connection is open, one page each time YouTube
  has messages (`items[].id`, `authorDetails.displayName`, `snippet.displayMessage`), each
  page with a `nextPageToken`. The first page is the chat from before you came. YouTube ends
  the connection after about ten seconds: open it again at once with `pageToken` set to the
  last `nextPageToken`, and nothing is read twice. An `offlineAt` says the broadcast ended.
  The same method is gRPC at `youtube.googleapis.com:443`
  (`youtube.api.v3.V3DataLiveChatMessageService/StreamList`), with the key in the
  `x-goog-api-key` metadata.
- Do not poll `liveChat/messages` instead. Every call spends the key's daily quota (10,000
  units, reset at midnight Pacific time), and a bridge polling at the `pollingIntervalMillis`
  YouTube asks for can run it out within hours: `403 quotaExceeded`, and no YouTube chat until
  the next day.

**5. Events, optionally.** On Twitch, from the same connection: a `USERNOTICE` whose
`msg-id` tag is `sub` or `resub` is a `sub`, `subgift` a `gift`, `raid` a `raid`; a
`CLEARMSG` is `deleted` (the `target-msg-id` tag); a `CLEARCHAT` naming a user is
`banned` (`ban-duration` in seconds, none for good), and one naming nobody is `cleared`.
Test: `remux events -f` shows them as `chat-event`.

**6. Say and delete, optionally.** Posting needs a token of the account that posts: on
Twitch, a second IRC connection with `PASS oauth:<token>` and `NICK <its login>`, then
`PRIVMSG #<channel> :<body>`. Deleting needs a moderator's token and the platform's API.
Test: `remux chat say hello` shows `hello` come back as a line.

### When nothing shows

| Symptom | Look at |
|---|---|
| `remux chat read -f` says `no chat wire` | `remux chat url` was not set: `remux config` shows `chat.url` |
| Nothing arrives, the bridge sees no client | `remux log -f`: the engine writes `wire: …` with the reason when it cannot connect |
| The client connects, no line shows | a frame the engine does not know is dropped without a word: compare yours with `remux schema` (`wire_line`); `id`, `from` and `body` are required |
| Lines show twice | two bridges, or the same chat read twice |

### Two to read beside this page

`engine/remuxd/examples/wire.rs` is step 1 and 2 in Rust (`cargo run -p remuxd --example
wire`); `byo/bridge.py` is steps 3 and 4 in Python.

## Down, server to engine

```json
{"line":{"id":"m1","platform":"twitch","channel":"main","from":"ana","body":"hi"}}
{"event":{"type":"sub","id":"u1","platform":"twitch","channel":"main","from":"ana","body":"six months!","badges":["member"],"reply":"","months":6,"tier":"1000"}}
{"history":[{"id":"m0","from":"bob","body":"earlier"}]}
{"destinations":[{"id":6,"name":"Youtube","platform":"youtube","status":"off","armed":true,"sandbox":true,"connected":true,"title":"teste"}]}
{"viewers":{"total":12,"answered":true,"peak":40}}
{"categories":{"id":6,"query":"soft","items":[{"id":"509670","name":"Software and Game Development"}]}}
{"notice":{"about":"Youtube","text":"told the title","fine":true}}
{"opened":"control"}
{"error":{"reason":"..."}}
```

`id` on a line is the platform's own message id, what a delete names;
`channel` is the destination's name. A frame the engine does not know is
dropped; a row with less in it is read with defaults. The shapes are in
`remux schema` under `wire_up` and `wire_line`.

An `event` is what happened in a chat beyond a line, flat, keyed by `type`,
with its own fields beside `id`, `platform`, `channel`, `from` and `body` (what
was written with it), `badges` (broadcaster, moderator, vip, member, verified,
first) and `reply` (the id it answers):

| type | fields |
|---|---|
| `sub` | `months`, `tier` |
| `gift` | `count`, `tier`, `to` (empty for a community gift) |
| `tip` | `amount` as shown, `currency`, `micros` (millionths of it; bits count as a currency) |
| `raid` | `viewers` |
| `follow` | |
| `deleted` | `target`, the id of the message taken down |
| `banned` | `user`, `seconds` (0 is for good) |
| `cleared` | |
| `custom` | `name` (`<platform>.<what>`), `fields` (strings) |

A chat message is a `line`, never an event; a tip's message comes as a line
and as its event, with one id. The engine says each as `chat-event` in `remux
events`, and `deleted`, `banned` and `cleared` take what they name off every
face. An event of a type it does not know is dropped.

## Up, engine to server

```json
{"open":"control"}                 once; then {"open":"chat"}, unless a bridge has the chat
{"arm":{"adapter":6,"on":true}}
{"sandbox":{"adapter":6,"on":true}}
{"retitle":{"adapter":6,"title":"...","description":"..."}}   only the fields given
{"announce":{"adapter":6}}
{"disconnect":{"adapter":6}}
{"categorize":{"adapter":6,"id":"509670","name":"Software and Game Development"}}
{"search":{"adapter":6,"query":"soft"}}
{"delete":{"id":"m1","channel":"main"}}
{"say":{"body":"valeu!","channel":"main"}}      no channel: every chat the server reads
{"heartbeat":{}}                   every 25 s
```

A server answers a verb with what changed (`destinations`, a `notice`), not
with a reply; an engine reads state, never acknowledgements. A `say` is
answered by the platform: the line comes back down as a `line` like
anybody's, and the engine keeps no copy of its own. `channel` is a line's.

## Where the engine looks

The chat and the destinations are looked up apart.

- The chat: `[chat] url` in `~/.config/remux/config.toml`, kept by `remux chat
  url ws://…` (0600, the URL may carry a token; `REMUX_CHAT_URL` overrides
  it): the `line` half alone, nothing is sent up but deletes and says. `remux chat
  url -` forgets it. Else the session's wire, its `chat` half. Else nothing:
  `remux chat` says `no chat wire`.
- The destinations: with a session (`remux login`), the account's, over the
  web's wire, its `control` half, whoever serves the chat. Else this machine's
  file.

With both, the engine keeps two sockets: the web's opens `control` alone and
the chat is the bridge's.

The engine reconnects on its own, three seconds after a drop. `remux chat
url` tells a running engine to open the new wire at once, a live untouched;
`remux daemon restart` after `remux login` or `remux logout`.

Without an account, a bridge's engine takes its destinations from its own
file, and the control half of the wire is not used.
