@AGENTS.md

## Where things are found

| From | To | Set in | Default |
|---|---|---|---|
| CLI | engine | `REMUXD_SOCKET` | `~/Library/Application Support/remux/remuxd.sock` (Linux: `~/.local/state/remux/`) |
| engine | destinations and their keys (no account) | `REMUX_DESTINATIONS` | `~/.config/remux/destinations.json` |
| engine, CLI | the account (`remux login`) | `REMUX_SESSION`; the web itself `REMUX_WEB` | `~/.config/remux/session.json`; `https://remux.live` |
| engine | disk | `REMUXD_RECORD_DIR`, `REMUX_MUSIC_DIR`, `REMUX_CLIPS_DIR`, `REMUXD_PREFS`, `REMUX_HISTORY` | `~/Movies/remux` (Linux `~/Videos/remux`), `~/Music/remux`, `~/Music/remux/clips`, beside the socket, `~/.config/remux/history.jsonl` |
| engine, CLI | everything above that is a folder or a door, once | `REMUX_CONFIG` | `~/.config/remux/config.toml` (`remux config` says what is in effect; env > file > default) |
| engine | a chat wire of one's own | `REMUX_CHAT_URL`, or `[chat] url` (`remux chat --url`) | none |
| engine | OBS (libobs) | `OBS_APP`, or `[daemon] obs_app` | `/Applications/OBS.app`; Linux the prefix, `/usr` |
| CLI | the engine as a service | `remux daemon` | launchd `~/Library/LaunchAgents/com.remux.remuxd.plist`; Linux `~/.config/systemd/user/remuxd.service` |

`REMUXD_RTMP` sends a live to one URL instead of the destinations (the smokes, a file).

## Architecture and discipline

`.claude/rules/architecture.md`, `typing.md`, `testing.md`, `security.md`.
