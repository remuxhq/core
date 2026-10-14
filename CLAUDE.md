@AGENTS.md

## Where things are found

| From | To | Set in | Default |
|---|---|---|---|
| CLI | engine | `REMUXD_SOCKET` | `~/Library/Application Support/remux/remuxd.sock` (Linux: `~/.local/state/remux/`) |
| engine | destinations and their keys (no account) | `REMUX_DESTINATIONS` | `~/.config/remux/destinations.json` |
| engine, CLI | the account (`remux login`) | `REMUX_SESSION`; the web itself `REMUX_WEB` | `~/.config/remux/session.json`; `https://remux.live` |
| engine | disk | `REMUXD_RECORD_DIR`, `REMUX_MUSIC_DIR`, `REMUX_CLIPS_DIR`, `REMUXD_PREFS`, `REMUX_HISTORY` | `~/Movies/remux` (Linux `~/Videos/remux`), `~/Music/remux`, `~/Music/remux/clips`, beside the socket, `~/.config/remux/history.jsonl` |
| engine, CLI | everything above that is a folder or a door, once | `REMUX_CONFIG` | `~/.config/remux/config.toml` (`remux config` says what is in effect; env > file > default) |
| engine | a chat wire of one's own | `REMUX_CHAT_URL`, or `[chat] url` (`remux chat url`) | none |
| engine | OBS (libobs) | `OBS_APP`, or `[daemon] obs_app` | `/Applications/OBS.app`; Linux the prefix, `/usr` |
| CLI | the engine as a service | `remux daemon` | launchd `~/Library/LaunchAgents/com.remux.remuxd.plist`; Linux `~/.config/systemd/user/remuxd.service` |

`REMUXD_RTMP` sends a live to one URL instead of the destinations (a test live, a file).

## Architecture and discipline

`.claude/rules/architecture.md`, `typing.md`, `testing.md`, `security.md`.

## Commits

Every subject follows Conventional Commits 1.0.0 (conventionalcommits.org):
`<type>(<scope>): <description>`, the scope optional.

- The type: `feat` (a new capability), `fix` (a bug), `docs`, `refactor`, `perf`,
  `test`, `build`, `ci`, `chore`, `style`, `revert`.
- The scope, when it helps: one noun for the part touched, `engine`, `motor`, `cli`,
  `tui`, `wire`, `deps`, `release`.
- The description starts in lowercase, in the imperative present, with no full stop:
  `fix: correct the music fader's floor`, `feat(tui): zoom the focused panel`.
- A change that breaks a face or the wire takes `!` after the type or scope, and a
  `BREAKING CHANGE:` footer saying what breaks.
- A bump that fixes an advisory is its own commit, named after it:
  `fix(deps): rustls 0.23.46, for RUSTSEC-2026-0346`.
- A pull request's title is its squashed commit, so it follows the same rule.
