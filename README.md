# remux

Live media for one person, from a shell. The engine captures a screen or a window, a
camera and a microphone, composes one 1080p30 scene, mixes the sound, records it and
sends it by RTMP, one stream per destination. `remux` is the face: five groups of
commands (scene, audio, music, destination, chat), prose for a person, `--json` for
anything else. macOS on Apple silicon, Linux on X11.

```
curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install.sh | sh
echo scene | remux destination add custom relay --url rtmp://127.0.0.1:1935 --key -
remux scene layer add screen desk 1 && remux scene layer add camera face FaceTime
remux audio mic Razer
remux plan                               # where it goes, and a fingerprint
remux live                               # the plan, a yes, then on air
remux stop && remux history
```

`install.sh` says what it will do and asks: the release for this machine, checked by
sha256, under `~/.local/share/remux`; `remux` on the PATH; the engine as a service of
your session (`remux daemon status|log|restart`). To run it, OBS is the one
dependency: the engine is libobs. `install-relay.sh` and `install-chat.sh` add a relay and a chat
bridge of your own; `uninstall.sh` takes everything out. [remux.live](https://remux.live).

## The tree

- `engine/`: `remuxd`, the daemon on a unix socket; `domain/` decides, `motor-obs/` is
  the machine (libobs), `wire/` the HTTP the daemon and the CLI share; `shader/` and
  `mixer/` are the picture's and the sound's filters, adapters the daemon injects into
  the motor.
- `engine/cli/`: `remux`, the words (`words.rs`: what was typed, and the prose back), the socket and the service.
- `byo/`: a relay (mediamtx, one ffmpeg per platform) and a chat bridge (Twitch IRC,
  YouTube) of your own; `docs/byo.md` is the walk-through, `docs/wire.md` the chat
  contract, `docs/relay.md` the relay's.
- `scripts/release.sh` builds the tarball for the machine it runs on; the release
  workflow runs it per target.

`AGENTS.md` says how to work on it. `make help` lists the targets.

## Building it

What a developer installs before `make remuxd.check` passes. Nothing here is needed
to run a release: those are build and test tools, and `install.sh` only asks for OBS.

macOS 15+, Apple silicon:

```
xcode-select --install          # clang, which reads OBS's headers, and codesign
curl https://sh.rustup.rs -sSf | sh   # engine/rust-toolchain.toml pins the version
brew install simde              # OBS's headers include it on ARM
make obs.fetch                  # the OBS the engine links, pinned, in engine/target/obs
```

Linux, X11 (Ubuntu 24.04+):

```
sudo apt-get install obs-studio libobs-dev clang
curl https://sh.rustup.rs -sSf | sh
sudo apt-get install xvfb       # only without a display: the socket tests start a daemon
```

Both, for the gate and the scanners:

```
cargo install cargo-nextest cargo-llvm-cov --locked
rustup component add llvm-tools-preview
make security.tools             # gitleaks, cargo-audit, cargo-deny
```

Then `make remuxd.check` (the gate) and `make security` (the scanners). To run the
engine you built on macOS, `make remuxd.identity` once first: macOS ties the Screen
Recording grant to the code signature, and signed ad hoc every rebuild is a stranger
whose capture comes back empty while the toggle stays on. A certificate of your own
keeps the grant across rebuilds; `make remuxd.run` is the engine in the foreground.

## What it runs on

| | Engine | Notes |
|---|---|---|
| macOS 15+, Apple silicon | OBS 32 from obsproject.com or Homebrew | Screen Recording is asked once, per install path |
| Linux, X11 (Ubuntu 24.04+) | OBS 30+ from the distribution | proven in an Ubuntu 24.04 VM; Wayland: not yet |
| Windows | | not yet |

## Licence

GPL-3.0 (`LICENSE`). Contributions under `CLA.md`.
