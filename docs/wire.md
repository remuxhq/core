# The wire

Everything the engine says to a server, and hears back, goes down one
WebSocket as JSON objects, one per text frame, keyed by what they are. The
engine knows this contract and no platform. The web serves it for an account
at `/wire/websocket?token=<socket token>&vsn=1.0.0`; anybody can serve the
chat half for themselves, in any language, with what follows.

## A chat bridge: the contract

A chat bridge reads a platform's chat however it likes and hands it to the
engine. That is the whole of it; the rest of this page is the account's wire.

1. **It is the server.** It listens for WebSocket connections (`ws://` on this
   machine, `wss://` elsewhere); the engine is the client, and connects to the
   URL `remux chat url ws://127.0.0.1:9999` keeps. The engine reconnects three
   seconds after a drop, so a bridge that restarts loses nothing but the gap.
2. **Down, one JSON object per text frame, to every connected client.** For
   each message in a chat, a `line`:

   | field | type | what |
   |---|---|---|
   | `id` | string | the platform's own message id: what a delete names |
   | `platform` | string | `twitch`, `youtube`, or any name |
   | `channel` | string | which chat it was said in, the same for every line of one chat |
   | `from` | string | who said it, as the platform shows the name |
   | `body` | string | what was said, as written |

   ```json
   {"line":{"id":"m1","platform":"twitch","channel":"somechannel","from":"ana","body":"hi"}}
   ```

   Optionally, an `event` for what happened beyond a line (a sub, a raid, a
   ban: the table below), and a `notice` to say what went wrong:
   `{"notice":{"about":"twitch","text":"the channel does not exist","fine":false}}`.
3. **Up, it may receive** `{"say":{"body":"…","channel":"…"}}` (post it; no
   channel means every chat it reads) and `{"delete":{"id":"…","channel":"…"}}`
   (take it down). Acting on either needs the platform's token; a bridge
   without one ignores them, or answers with a `notice`. Anything else it
   does not know, it ignores.
4. **Text from strangers.** A bridge passes chat on as data. It never runs it,
   and never puts it in a command, a query or a log format string.

Try it: `remux chat url ws://127.0.0.1:9999`, then `remux chat read -f` shows
the lines and `remux events -f` the events. `remux chat url -` forgets it.

Reading a platform, for a first bridge:

- **Twitch**, with no token: IRC over TLS at `irc.chat.twitch.tv:6697`,
  `NICK justinfan<any number>`, `JOIN #<channel>`, answer `PING` with `PONG`;
  each `PRIVMSG #<channel> :<body>` is a line, the sender in the `:<nick>!`
  prefix and the id in the `id=` tag (`CAP REQ :twitch.tv/tags` first).
- **YouTube**, with a Data API key: `videos?part=liveStreamingDetails&id=<video>`
  gives the `activeLiveChatId`; poll `liveChatMessages` with it, waiting the
  `pollingIntervalMillis` each answer names. The broadcast has to exist first.

Two to read beside this page: `engine/remuxd/examples/wire.rs`, the smallest,
a line a second (`cargo run -p remuxd --example wire`); `byo/bridge.py`,
Twitch and YouTube in Python.

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
