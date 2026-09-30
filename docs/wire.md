# The wire

Everything the engine says to a server, and hears back, goes down one
WebSocket as JSON objects, one per text frame, keyed by what they are. The
engine knows this contract and no platform. The web serves it for an account
at `/wire/websocket?token=<socket token>&vsn=1.0.0`; anybody can serve the
chat half for themselves.

## Down, server to engine

```json
{"line":{"id":"m1","platform":"twitch","channel":"main","from":"ana","body":"hi"}}
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
{"heartbeat":{}}                   every 25 s
```

A server answers a verb with what changed (`destinations`, a `notice`), not
with a reply; an engine reads state, never acknowledgements.

## Where the engine looks

The chat and the destinations are looked up apart.

- The chat: `[chat] url` in `~/.config/remux/config.toml`, kept by `remux chat
  --url ws://…` (0600, the URL may carry a token; `REMUX_CHAT_URL` overrides
  it): the `line` half alone, nothing is sent up but deletes. `remux chat
  --url -` forgets it. Else the session's wire, its `chat` half. Else nothing:
  `remux chat` says `no chat wire`.
- The destinations: with a session (`remux login`), the account's, over the
  web's wire, its `control` half, whoever serves the chat. Else this machine's
  file.

With both, the engine keeps two sockets: the web's opens `control` alone and
the chat is the bridge's.

The engine reconnects on its own, three seconds after a drop. `remux chat
--url` tells a running engine to open the new wire at once, a live untouched;
`remux daemon restart` after `remux login` or `remux logout`.

## A chat bridge of your own

Read the platform's chat however you like and write one `line` object per
message to every connected client; act on `delete` or ignore it. That is the
whole of it: without an account the destinations are the engine's own file,
and the control half is not used. The smallest bridge is
`cargo run -p remuxd --example wire`.
