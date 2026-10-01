# Your own relay

The engine sends one RTMP stream to each destination. A relay is a destination that takes that stream and sends it on to the platforms, so only one stream leaves your machine.

```
echo scene | remux destination add custom relay --url rtmp://127.0.0.1:1935 --key -
```

The key goes in on stdin or with `--key-file`, never on the command line.

## The one in `byo/`

`byo/mediamtx.yml` runs mediamtx on this machine, taking RTMP on port 1935. When the stream arrives, it runs one `ffmpeg -c copy` for each platform whose key is in `~/.config/remux/byo.env` (`TWITCH_KEY`, `YOUTUBE_KEY`). `install-relay.sh` sets it up; `docs/byo.md` walks through it.

Any other RTMP server works the same way, here or elsewhere: give the engine its URL and key.
