# The login contract

`remux login --url https://server` signs a terminal into a server, by a code
typed on a page, and keeps a token in `~/.config/remux/session.json` (0600).
Three HTTP calls, JSON both ways. This is the CLI's side of our web; without
an account there is no login, and none is needed (`docs/relay.md`,
`docs/wire.md`).

## 1. `POST /api/device`

No body. Answers `200`:

```json
{"device_code":"<secret the terminal keeps>",
 "user_code":"BWA384FB",
 "verification_url":"https://server/device",
 "expires_in":600,
 "interval":3}
```

The terminal prints `open <verification_url> and type the code <user_code>`
and polls.

## 2. `POST /api/token`

Body `{"device_code":"..."}`. Every `interval` seconds until `expires_in`:

- `428 {"error":"authorization_pending"}`: nobody typed it yet;
- `200 {"token":"<bearer, url-safe base64>"}`: typed, by a signed-in person;
  the code is spent;
- `410 {"error":"expired"}`, or any other status with `{"error":"..."}`:
  the terminal stops and says why.

## 3. `GET /api/session`

`Authorization: Bearer <token>`. Answers `200`:

```json
{"email":"who@example.com",
 "socket_token":"<what the wire's URL carries>",
 "rtmp":"rtmp://relay:1935/scene?user=remux&pass=<publish token>"}
```

`rtmp` is where the engine publishes for this account (the relay's door,
with a credential the relay checks). `socket_token` goes on the wire's URL
(`docs/wire.md`). Without a valid token: `401`.

The engine calls this at start and at every reconnect of the wire; a session
the server no longer knows makes `remux destination list` say so. `remux logout`
removes the file.
