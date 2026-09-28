<p align="center"><img src="docs/icon.svg" width="64" alt="remux"></p>

# remux

[Remux](https://remux.live) is a live streaming tool based on [libOBS](https://github.com/obsproject/obs-studio) that provides a set of building blocks for streaming. It's developer-centric, ready for AI-assisted workflows, free and open-source.

This free **GPL-3.0-only core** version comes with a CLI ready for humans and agents, including a variety of commands to control the scene, camera and audio. Remux is also extensible by design: it brings all the wiring for your own RTMP relay and an interface for ingesting chat.

All these fundamental pieces make Remux a composable tool for live streaming with support for multiple destinations and chat ingestion.

## Quick start

Needs OBS installed. The engine is powered by libobs.

```
curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install.sh | sh
remux destination add youtube yt --key-file ~/yt-streaming.key

remux devices                             # the ids of screens, cameras and mics
remux scene layer add screen desk 1
remux scene layer add camera face FaceTime
remux audio mic Razer

remux destination sandbox 1 on            # sandbox mode, according to the destination
remux plan                                # preview where it goes before going live
remux live                                # on air
remux stop
```

## Components

```
remux            engine/cli        the CLI, for humans and agents
  │ unix socket
  ▼
remuxd           engine/remuxd     daemon, socket and chat wire
  ├─ domain      engine/domain     core business domain, including ports to the motors
  ├─ motor-obs   engine/motor-obs  libobs behind the ports, one ffmpeg per destination
  ├─ shader      engine/shader     WGSL filters to OBS effects
  ├─ mixer       engine/mixer      mic gate
  └─ wire        engine/wire       HTTP for login, shared with the CLI
```

`AGENTS.md` says how to work on it. `make help` lists the targets.

## Stack

* Rust 1.98.0
* OBS 32 (macOS), 30+ (Linux)

## Development setup

Remux is under active development, and environment steps will likely change.

For macOS 15+, Apple silicon:

```
xcode-select --install          # clang, which reads OBS's headers, and codesign
brew install simde              # OBS's headers include it on ARM
make obs.fetch                  # the OBS the engine links, pinned, in engine/target/obs
```

Linux, X11 (Ubuntu 24.04+):

```
sudo apt-get install obs-studio libobs-dev clang
sudo apt-get install xvfb       # only without a display: the socket tests start a daemon
```

Both:

```
curl https://sh.rustup.rs -sSf | sh   # engine/rust-toolchain.toml pins the version
make setup                            # nextest, llvm-cov, scanners
```

## What it runs on

| | Engine | Notes |
|---|---|---|
| macOS 15+, Apple silicon | OBS 32 from obsproject.com or Homebrew | Tested as of September 28, 2026, with OBS 32.1.2. Screen Recording is asked once, per install path |
| Linux, X11 (Ubuntu 24.04+) | OBS 30+ from the distribution | Limited support, needs further testing; proven in an Ubuntu 24.04 VM. Wayland: not yet |
| Windows | | Not yet |

## License

GPL-3.0 (`LICENSE`). Contributions under `CLA.md`.
