# Architecture

DDD, hexagonal-lite, in Rust. `engine/domain/` (crate `remuxd-domain`) decides and sees
no framework and no socket; a motor is the machine behind the domain's ports
(`engine/motor-obs/` is libobs; another motor is another crate on the same ports);
`engine/remuxd/` is the daemon: `boot` (the daemon as a function, given a motor), the
socket, the wire; `engine/cli/` is a face, and its words are its own; `engine/wire/` is the HTTP the daemon and the CLI
share.

## The engine's contexts

`engine/domain/src/engine/` is one module per context, each owning its verbs and the
port it drives; `mod.rs` holds the state, the dispatcher and what crosses contexts
(the panic button, the tick, the status):

- `picture`: the active scene's layers (captures) and elements (text, timers) in one
  back-to-front order, the scenes and their switch, the filters (WGSL), the
  preview lease. Port: `Picture`.
- `sound`: the microphone, the gate, the denoiser, the music and its rotation, the
  clips, the faders, the speakers, and the audio layers (a microphone, one
  application, what the computer plays), each the active scene's. Port: `Sound`.
- `air`: going live and recording, the two levers on the same picture; the live written
  down when it ends. Port: `Air`.
- `app`: what the engine asks on a face's behalf (arm, retitle, announce, the chat, the
  categories). Port: `Watching`, implemented by `destinations::Local` with no account
  and by `remuxd::wire::App` with one.

`Pipeline` is the three media ports together plus the grants, and one motor implements
each port in its own `impl`. A verb that needs two contexts belongs in `mod.rs`, like
the panic button.

## Beside the engine

The domain's modules sit under the context they serve, so a building block is found
where its verbs are:

- `picture/`: scene, layers, scenes, sources, camera, preview, timer, and `shader`,
  the port filters are compiled through (`ShaderCompiler`) and the WGSL contract.
- `sound/`: audio_layers, music, clips, and `mixer/`: the `Filter` contract, the gate's
  side of it (`GateFilter`, `MakeGate`, its settings and levels), and the meters.

Filters are ports like the others: the domain owns the contract and never runs a
filter. The adapters (`remux-shader` over naga, `remux-mixer` with the gate) depend on
the domain, and the daemon's `main.rs` injects them into the motor. Nothing else knows
an adapter, which `make remuxd.seam` checks for the domain and the CLI.
- `air/`: destinations (the file, `~/.config/remux/destinations.json`, and `Local`,
  the `Watching` over it), plan, recording, history, journal.
- `app/`: wire (`docs/wire.md`), chat (the feed every face reads), session, login.
- At the top, the host: `config`, `os` (what is in effect, and the one table of what
  differs per OS), `log`, `socket`, `daemon`, `bug`, `wait`, `health`, `remembered`;
  and `protocol`, the language between faces and engine.

## Rules

- **Shared code never names an operating system.** The domain, the daemon, the wire
  and the CLI read one table, `remuxd_domain::os::OS` (where state, recordings and OBS
  live; how the engine runs as a service, as argv lists the shell executes). The libobs
  motor reads its own, `motor_obs::platform::TABLE` (plugins, source ids, encoders,
  paths). A new OS is a column in each table and a runner in the release matrix, never
  an `if` elsewhere; `make remuxd.seam` fails on `target_os` outside the two files
  (`cfg(unix)` around std's file modes is std's shim, allowed).
- Decisions live in pure modules next to the adapter that acts on them. A thread or an
  HTTP call is transport; if it starts deciding, split it.
- Stream keys and tokens are per-row data in the destinations file, never in code,
  config or logs; the shell writes them, the engine reads them.
- The CLI's default prose is a contract (its unit tests); `--json` is the shape.
