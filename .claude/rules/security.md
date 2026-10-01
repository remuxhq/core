# Security

What never goes anywhere, and what every change has to keep true.

## Secrets

- A stream key, a token, an API key: never in code, config, a fixture, a log line, a
  commit message, a comment, a chat message, a screenshot. Redact to the first four
  characters when one must be named.
- They live in `~/.config/remux/` (0600, written whole and renamed into place by
  `destinations::keep`): `destinations.json`, `session.json`, `config.toml`, `byo.env`.
  The shell writes them (`destination add --key -`, `--key-file`, `remux login`,
  `chat url`); a key is never on argv, which `ps` lists, and never crosses the socket.
- `Key` prints stars. A `Debug` of a row is safe; a `.0` reaching a log is a finding.
- Files the engine writes beside the socket (`prefs.json`, the log) hold no secret.
  `remux bug` redacts before it prints.

## Doors

- The engine's socket answers any local process today. Verbs that start capture,
  publish or quit are a face's.
- Anything that executes a string takes argv from validated fields, never a shell from
  interpolated text. A stream key on a command line is visible to `ps` on that host:
  `byo/mediamtx.yml` puts them on ffmpeg's, on the person's own machine, and says so.
- The engine is signed with hardened runtime on macOS; a build without the identity
  says so rather than pretending.

## Text from strangers

Chat, titles, names: control characters stripped before a terminal (`remux chat read -f`),
never interpolated into a command, a query or a log format string.

## Process

- Never run a scanner over the whole tree: `make security` scans the history and the
  tracked files only, each tool under a timeout.
- Never touch the machine under a live.
- A CVE is cited only when a fetched page names the id, the range and the fix. A
  tolerated advisory is a line in `engine/deny.toml` with its reason. A bump that fixes
  an advisory is its own commit, named after it.
