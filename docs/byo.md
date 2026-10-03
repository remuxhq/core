# Bring your own relay and chat

Remux **core** works without an account. A relay and a chat bridge are yours to build or pick: `docs/relay.md` and `docs/wire.md` are their contracts, and this page walks through the ones in `byo/`. Each script asks before doing anything and is safe to run again.

```
curl -fsSL https://github.com/remuxhq/core/releases/latest/download/install.sh | sh
curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install-chat.sh | sh   # optional, the Python bridge
```

`install.sh` installs `remux` and starts the engine as a service. Needs OBS installed; the script offers to install it. To remove everything, run `~/.local/share/remux/current/uninstall.sh` (`--purge` also removes the config).

## Relay

The engine already sends to every armed destination, one stream each, and that is enough for most lives. A relay is optional, and it belongs on a host with upload to spare, not on this machine: here it would still upload one stream per platform. `docs/relay.md` is what a relay does, what the engine sends it, and the commands that switch to it.

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

The bridge takes only the chat. With an account (`remux login`), the destinations, the title and the rest stay the account's. `remux chat url -` turns the bridge off, back to the account's chat if you have one. `remux chat delete` only reaches the bridge: deleting on the platform needs a moderator token.

`remux chat say valeu!` posts a line as you, on every chat the bridge reads; `--to <channel>` picks one (a line's channel, as `remux events` shows it). It comes back in `remux chat read` like anybody's. The bridge needs your tokens in `byo.env`, and says what is missing otherwise:

```
TWITCH_CHAT_TOKEN=…        # a user token with the chat:edit scope
TWITCH_CHAT_NICK=…         # whose token it is, if not the channel's
YOUTUBE_CLIENT_ID=…        # OAuth with the youtube.force-ssl scope;
YOUTUBE_CLIENT_SECRET=…    # the refresh token is the channel's that
YOUTUBE_REFRESH_TOKEN=…    # owns the live
```

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

`remux daemon log -f` shows the engine's log. `remux bug --open <what went wrong>` opens a GitHub issue titled with those words and a report filled in, keys redacted. `remux config` shows every setting and where it came from.
