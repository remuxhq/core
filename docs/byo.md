# Bring your own relay and chat

Remux **core** works without an account. You can run your own relay and chat bridge on this machine, with three scripts. Each one asks before doing anything and is safe to run again.

```
curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install.sh | sh
curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install-relay.sh | sh
curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install-chat.sh | sh
```

`install.sh` installs `remux` and starts the engine as a service. Needs OBS installed; the script offers to install it. To remove everything, run `~/.local/share/remux/current/uninstall.sh` (`--purge` also removes the config).

## Relay

The engine already sends to every armed destination, one stream each. The relay is optional: one stream leaves your machine, and the relay sends it on to each platform, which spares your upload. `install-relay.sh` installs mediamtx and ffmpeg, and writes `~/.config/remux/byo.env` (0600) for your keys.

```
$EDITOR ~/.config/remux/byo.env      # TWITCH_KEY=…  YOUTUBE_KEY=…
remux-relay                          # keep it running in its own pane
echo scene | remux destination add custom relay --url rtmp://127.0.0.1:1935 --key -
```

The keys reach ffmpeg's command line, visible to `ps` on this machine. Any other RTMP server works the same way (`docs/relay.md`).

## Chat

`install-chat.sh` installs `remux-chat`, a bridge that reads Twitch and YouTube chat and points the engine at it. YouTube needs `YOUTUBE_API_KEY` in `byo.env`, from a Google Cloud project with the YouTube Data API enabled. Twitch needs only the channel name.

```
remux-chat --twitch <channel> --youtube <video id>    # keep it running in its own pane
remux chat read -f
```

The YouTube video id is in the live's share link, so the broadcast must exist in YouTube Studio first. You can also set both in `~/.config/remux/config.toml`:

```
[byo]
twitch = "<channel>"
youtube = "<video id>"
```

`remux chat --url -` turns the chat off. `remux chat delete` only reaches the bridge: deleting on the platform needs a moderator token.

## Going live

```
remux health                         # what stands in the way
remux scene layer add screen desk 1
remux scene layer add camera face FaceTime
remux audio mic Razer
remux plan
remux live
remux stop
```

On macOS, turn on `remuxd` in System Settings → Privacy & Security → Screen Recording, then `remux daemon restart`. The microphone asks in a dialog. Linux asks for nothing. Recordings go to `~/Movies/remux` (`~/Videos/remux` on Linux).

## When something is wrong

`remux daemon log -f` shows the engine's log. `remux bug --open` opens a GitHub issue with a report filled in, keys redacted. `remux config` shows every setting and where it came from.
