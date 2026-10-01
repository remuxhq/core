# A relay

The engine already sends the live to every armed destination: one RTMP stream per
platform, all of them from your machine. A relay is optional. It is a server of yours that
takes one stream from the engine and sends it on to each platform, so your machine uploads
once whatever the number of platforms.

Run it on a host with upload to spare (a VPS, a cloud VM), not on the machine that runs
remux: a relay there still uploads one stream per platform from the same connection, and
saves nothing.

This page is the contract. Build one in whatever you like, or configure an RTMP server
that already exists.

## What the engine sends

One RTMP publish, as FLV, to the URL and key of a `custom` destination:

| | |
|---|---|
| Video | H.264, 1920x1080, 30 fps, 6 Mbps constant, a keyframe every 2 s, no B-frames |
| Audio | AAC, 160 kbps, 48 kHz, stereo |
| Address | `rtmp://<host>:1935/<app>`, the key appended as the last path segment |

## What a relay does

1. Accepts an RTMP publish on port 1935, and only with the key you chose: an RTMP server
   that takes any key takes anybody's video.
2. For each platform, republishes the same stream **without re-encoding** (`-c copy`, or
   the server's own push), so the picture is the engine's and the host needs no GPU:
   - Twitch: `rtmp://live.twitch.tv/app/<twitch key>`
   - YouTube: `rtmp://a.rtmp.youtube.com/live2/<youtube key>`
   - Any other RTMP ingest: its URL and key.
3. Keeps the platforms' keys on the host, in its environment or a file only it reads,
   never in a repository and never on a command line others on the host can list.
4. Stops a platform's copy when the publish ends, and starts it again when it comes back.

The host needs 6.2 Mbps of upload per platform, and about that much download.

## On remux's side

```sh
remux destination add custom relay --url rtmp://<host>:1935/<app> --key -   # the relay's key, at its prompt
remux destination disarm <twitch id>      # the relay sends to Twitch now: never both
remux destination disarm <youtube id>
remux destination list                    # only the relay armed
remux plan
```

The engine picks the armed destinations when a live starts, so a change during a live
takes effect at the next `remux stop` and `remux live`. A platform that receives the same
key from the relay and from the engine drops one of the two.

## Ready-made

Any RTMP server that can push a stream on does it. Two that need no code:

- **mediamtx**: a path that runs one `ffmpeg -c copy` per platform when the publish is
  ready. `byo/mediamtx.yml` is that config, written for loopback; on a host, listen on the
  public address and require the publish key.
- **nginx with the RTMP module**: one `push` line per platform in the application block.
