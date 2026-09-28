# A relay of your own

The engine sends one RTMP stream wherever you point it. A relay is anything that
takes that stream and sends it on: a destination like any other, from the engine's
side.

```
echo scene | remux destination add custom relay --url rtmp://127.0.0.1:1935 --key -
```

The key goes in on stdin (or `--key-file`), never on the command line. One ffmpeg from
your machine to the relay; the relay fans out.

## The one in `byo/`

`byo/mediamtx.yml` is mediamtx on loopback: RTMP in on 1935, RTSP for its own egress
on 8554, everything else off. When the `scene` path is up it runs one `ffmpeg -c copy`
per platform whose key is set in the environment (`TWITCH_KEY`, `YOUTUBE_KEY`, from
`~/.config/remux/byo.env`). `install-relay.sh` installs mediamtx and ffmpeg and puts
`remux-relay` on the PATH, which runs it with those keys; `docs/byo.md` is the
walk-through.

Any other RTMP server works the same way, on this machine or elsewhere: give the
engine its URL and its key.
