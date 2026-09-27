# Bring your own: a relay and a chat, no account

The CLI and the engine work with no account at all. What an account adds
(our relay, our chat bridge) you can run yourself, on this machine, with
three scripts. Each says what it will do and asks before doing it; each is
safe to run again. macOS on Apple silicon (15+), and Linux (X11).

```
curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install.sh | sh
curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install-relay.sh | sh
curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install-chat.sh | sh
```

`install.sh` checks the release's sha256, puts the versions in
`~/.local/share/remux`, `remux` on `~/.local/bin` (and on your PATH, a marked
block in your shell's rc file), and starts the engine as a service of your
session (launchd, or `systemd --user`): `remux daemon status`, `remux daemon
log -f`, `remux daemon restart`. OBS is the one dependency, because the
engine is libobs: OBS 32 on macOS, OBS 30+ from the distribution on Linux;
the script offers to install it. The uninstall is
`~/.local/share/remux/current/uninstall.sh` (`--purge` takes the config too).

## 1. The relay: one stream out, one ffmpeg per platform

`install-relay.sh` installs mediamtx and ffmpeg, writes
`~/.config/remux/byo.env` (0600) for your keys, and puts `remux-relay` on
the PATH. `byo/mediamtx.yml` is mediamtx on loopback: the engine publishes to
`rtmp://127.0.0.1:1935/scene`, and when the scene is there mediamtx runs one
`ffmpeg -c copy` per platform whose key is set. The keys live in that file
of yours, never on a command line you type:

```
$EDITOR ~/.config/remux/byo.env      # TWITCH_KEY=…  YOUTUBE_KEY=…  YOUTUBE_API_KEY=…
remux-relay                          # keep it running, its own pane
```

Then the relay is a destination like any other, the path as its key:

```
echo scene | remux destination add custom relay --url rtmp://127.0.0.1:1935 --key -
```

The keys do reach ffmpeg's command line, where `ps` on this machine can see
them; this machine is yours. Any RTMP server elsewhere works the same way
(`docs/relay.md`); so does one ffmpeg per destination with no relay at all,
which is what the engine does when the destinations are the platforms
themselves.

## 2. The chat: Twitch and YouTube, on the wire

`install-chat.sh` puts `remux-chat` on the PATH: `byo/bridge.py`, which reads
Twitch over IRC (anonymously) and YouTube's live chat through the Data API
(`YOUTUBE_API_KEY` in `byo.env`, from a Google Cloud project with the YouTube
Data API on), and serves both on the engine's wire (`docs/wire.md`). Python's
standard library only, 3.9 or newer. `remux-chat` points the engine at the
bridge itself (`remux chat --url ws://127.0.0.1:9999`).

```
remux-chat --twitch <channel> --youtube <video id>        # its own pane
remux chat -f
```

The channel and the video id can live in `~/.config/remux/config.toml`
instead, under `[byo]` (`twitch = "…"`, `youtube = "…"`); the bridge reads
them when its command line says nothing. The YouTube video id is the one in
the live's URL: YouTube Studio → Go live → Stream → the share link; a
broadcast has to exist there before the chat can be read, because the
platform hands out the chat per broadcast (with an account, the web's API
does this for you). Twitch needs nothing but the channel's name. `remux chat --url -` goes back to no chat (or the
account's). A `delete` from `remux delete <seq>` reaches the bridge and is
printed, not done: taking a message down on a platform needs a moderator's
token, which the bridge does not hold.

## 3. The live

```
remux health                       # what stands in the way, one line each
remux screen 1; remux camera "FaceTime"; remux mic "Razer"
remux shot --out /tmp/scene.jpg    # the picture, as it would go out
remux plan                         # where it goes, and a fingerprint
remux live                         # the plan, a yes, then on air
remux chat -f                      # both platforms, one stream of lines
remux stop; remux history
```

On macOS, Screen Recording and Microphone are granted to `remuxd` once: the
first `remux screen` makes macOS list `remuxd` in System Settings → Privacy &
Security → Screen Recording, you turn it on, `remux daemon restart`; the
microphone asks in a dialog. `remux health` says which is missing. Linux
asks for nothing. The engine writes `~/Movies/remux/<date>.mp4`
(`~/Videos/remux` on Linux) of what went out.

## When something is wrong

`remux daemon log -f` is the engine talking. `remux bug` prints a report
(versions, what stands in the way, the config, the last log lines; keys and
tokens redacted) to paste into an issue; `remux bug --open` opens GitHub's
bug form with it filled in, and you press submit. An issue without that
report is closed by a bot until it has one.

## Where things are

`remux config` prints what is in effect and where each value came from: the
environment, then `~/.config/remux/config.toml`, then the defaults. Music in
`~/Music/remux`, clips in `~/Music/remux/clips`, recordings in
`~/Movies/remux` (`~/Videos/remux` on Linux); the destinations file, the config and `byo.env` in
`~/.config/remux`, 0600.
