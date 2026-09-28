# remux

Live media for one person, from a shell, on macOS and Linux. The engine (`engine/`,
Rust) drives a screen, a camera, a microphone and a music bed into one 1080p30 scene,
records it, and sends it by RTMP, one ffmpeg per armed destination kept on this
machine. The `remux` CLI (`cli/`) is the face, for humans (tmux) and for agents. A
relay and a chat bridge of one's own live in `byo/`; anybody may serve the chat wire
(`docs/wire.md`) or a relay (`docs/relay.md`).

## Hard rules

These are absolute. Nothing else in this repository uses absolute language.

1. **Never go live for real from a test.** A test live goes to a lab door (`make
   smoke.lab.up`) or a platform's sandbox; the smokes refuse a real armed destination
   otherwise (`lab_only`).
2. **Never touch the machine under a live.** No gates, no restarts, no builds, no
   launching or signalling anything named `remux`. Read-only until the live ends.
3. **Secrets never leave `~/.config/remux/`.** `destinations.json` (keys),
   `session.json` (a token), `byo.env`: 0600, written by the shell, read by the
   engine, never on argv, never over the socket, never in code, config, fixtures,
   logs, commits or chat. Name one by its first four characters if you must.
4. **Shared code names no operating system.** Two tables do: `engine/domain/src/os.rs`
   (paths, the service) and `engine/motor-obs/src/platform.rs` (libobs per OS);
   `make remuxd.seam` fails on anything else.

## How to work

Act by default. Stop only for the rules above, a destructive action, or a decision
the operator has kept. A failing test first, on production code; the smallest fix;
the fast tests after every edit (`make remuxd.test F=name`, sub-second); the one smoke
that proves the unit, by name, when the unit closes (`make remuxd.smoke S=name`); the
whole suite and the gate never mid-loop. Hardware-only checks are the operator's when
you have no machine: report them as pending and finish. Small commits, present tense,
plain subjects, no authorship trailers, no session links. Code and commit messages are
the documentation; a measurement behind a decision goes in the comment beside it.

## Layout

- `engine/domain/`: every decision, serde only. Contexts under `src/engine/`: picture,
  sound, air, app; ports `Picture`, `Sound`, `Air`, `Sources`, `Watching`.
  `destinations` (the file; `Local` is the `Watching` with no account), `session` and
  `login` (`remux login`), `wire` (what a server and the engine say to each other) and
  `chat` (the feed), `config`, `os`, `history`, `plan`, `layers`, `scenes`, `audio_layers`,
  `music`, `gate`.
- `engine/remuxd/`: the daemon: `boot` (the daemon as a function, given a motor), the
  socket, `wire.rs` (the one WebSocket to whoever serves the wire); `smoke/` the parity
  checks, `cli.py` and the lab.
- `engine/motor-obs/`: the libobs motor; `platform.rs` is the table per OS, `picture.rs`
  the layers and elements on one scene, `effect.rs` the two sources it adds to libobs
  (an operator's WGSL filter as an OBS effect, an element's box of text); `engine/filter/`
  is everything about filters: the WGSL contract and its OBS effect.
- `engine/wire/`: the HTTP the daemon and the CLI share (login, session).
- `cli/`: `remux`. The words are `domain/src/cli.rs`; its prose is a contract.
- `byo/`, `docs/`, `scripts/`, `install*.sh`, `uninstall.sh`.

## Commands

- `make remuxd.check`: the gate (seam, fmt, clippy, nextest, domain coverage ≥ 90%).
  `make remuxd.test F=name` mid-loop. `make remuxd.cli` round-trips every verb (~12 s).
  `make remuxd.smoke S=name` one parity check on the machine (`free` and `byo` against
  `make smoke.lab.up`, Docker). `make security` the scanners.
- macOS: `make obs.fetch` once (the OBS the engine links, pinned), `make remuxd.identity`
  once (the signing certificate). Linux: `apt-get install obs-studio libobs-dev`.
- `make remuxd.start` / `remuxd.run`: the engine here. `make release`,
  `release.install`, `release.uninstall`: the tarball for this machine and the install
  a person gets, from `dist/`; `make release.publish` the GitHub release of the version.

## Conventions

Code, comments, tests, docs in English. A feature lands with tests and is verified on
the machine before it counts as done. Wire changes keep old clients working (contract
tests assert exact bytes). Contributions are under `CLA.md`.
