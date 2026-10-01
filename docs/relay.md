# Building a relay

The engine already sends the live to every armed destination, one RTMP stream per
platform. A relay is optional: a server of yours that takes one stream from the engine
and sends it on to each platform. Where it runs is yours to pick. On a host with upload to
spare, it is one upload from your machine whatever the number of platforms; on the same
machine as remux it saves no upload, but keeps the platforms and their keys out of remux.

This page is the contract and a path to build one, in any language, testing each step
before the next.

## What the engine sends

One RTMP publish, as FLV, to the URL and key of a `custom` destination:

| | |
|---|---|
| Video | H.264, 1920x1080, 30 fps, 6 Mbps constant, a keyframe every 2 s, no B-frames |
| Audio | AAC, 160 kbps, 48 kHz, stereo |
| Address | `rtmp://<host>:1935/<app>/<key>`: the URL you give remux, then the key |

## The contract

1. Accept an RTMP publish on port 1935, and only with the key you chose. An RTMP server
   that takes any key takes anybody's video.
2. For each platform, republish the same stream **without re-encoding**, so the picture is
   the engine's and the relay needs no GPU:
   - Twitch: `rtmp://live.twitch.tv/app/<twitch key>`
   - YouTube: `rtmp://a.rtmp.youtube.com/live2/<youtube key>`
   - Any other RTMP ingest: its URL and its key.
3. Start the copies when the publish starts, stop them when it ends, and start them again
   when it comes back.
4. Keep the platforms' keys where only the relay reads them (its environment, a file of
   mode 0600), never in a repository, and never on a command line other users can list.

Upload: 6.2 Mbps per platform; download: about 6.2 Mbps.

## Build it, step by step

Test with a stand-in for remux and a file in place of a platform. **Never test on a
platform's real ingest.** The stand-in is ffmpeg, sending what remux sends:

```sh
ffmpeg -re -f lavfi -i testsrc2=size=1920x1080:rate=30 -f lavfi -i sine=frequency=440:sample_rate=48000 \
  -c:v libx264 -b:v 6000k -g 60 -bf 0 -c:a aac -b:a 160k -ar 48000 \
  -f flv rtmp://127.0.0.1:1935/live/<key>
```

**1. Accept the publish.** Three ways, from the least code to the most:

- An RTMP server that exists, configured (next section).
- An RTMP library in your language, which hands you the publish and its packets.
- The protocol by hand (the last section of this page).

Test: the stand-in connects and keeps sending; a wrong key is refused.

**2. Copy without re-encoding.** For each platform, one copy. With ffmpeg as the copier,
either pulling from your server (`ffmpeg -i <the stream on your server> -c copy -f flv <platform>`)
or fed the stream on its stdin (`ffmpeg -f flv -i pipe:0 -c copy -f flv <platform>`).

Test: send each copy to a file instead (`-f flv /tmp/twitch.flv`), then
`ffprobe /tmp/twitch.flv` says h264 1920x1080 30 fps and aac 48000 Hz stereo.

**3. Follow the publish.** Stop the stand-in: the copies stop. Start it again: they start.

**4. remux, still to files.** Add the relay to remux, and send a real live through it while
the relay still writes files:

```sh
remux destination add custom relay --url rtmp://<host>:1935/live --key -   # the relay's key, at its prompt
remux destination disarm <twitch id>        # never the relay and the platform direct at once
remux destination disarm <youtube id>
remux destination list                      # only the relay armed
remux plan
remux live
```

**5. The platforms.** Point the copies at the real ingests, with their keys, and rehearse
first: Twitch's bandwidth test (`?bandwidthtest=true` after the key) shows nothing to
anyone; on YouTube, a private broadcast.

The engine picks the armed destinations when a live starts: a change to them during a
live takes effect at the next `remux stop` and `remux live`. A platform that gets one key
from two sources, the relay and the engine direct, drops one of them.

## With a server that exists

- **mediamtx**: a path whose `runOnReady` starts one `ffmpeg -c copy` per platform.
  `byo/mediamtx.yml` is that config, for loopback; on a host, set `rtmpAddress` to the
  public address and require the publish key (its `authInternalUsers`).
- **nginx with the RTMP module**: an `application` with one `push <platform>/<key>;` per
  platform, and `on_publish` to check the key.

## The protocol by hand

For a relay in a language with no RTMP library. All integers are big-endian unless said.

**Handshake.** The client sends C0 (one byte, 3) and C1 (1536 bytes: a 4-byte time, 4
zero bytes, 1528 random). Answer S0 (3), S1 (1536 bytes of your own) and S2 (C1, echoed).
Read C2 (1536 bytes, an echo of S1) and ignore it.

**Chunks.** Every message arrives cut in chunks of the chunk size: 128 bytes until the
client sends a Set Chunk Size (message type 1, a 4-byte size), which ffmpeg does at once.
A chunk starts with a basic header: 2 bits of format, then a chunk stream id in 6 bits (0
means one more byte, the id minus 64; 1 means two more, little-endian). Then the message
header, by format: 0 is 11 bytes (timestamp 3, length 3, type 1, message stream id 4,
little-endian); 1 is 7 (timestamp delta, length, type); 2 is 3 (timestamp delta); 3 is
nothing, the same as the last chunk on that id. A timestamp of `0xFFFFFF` means 4 more
bytes of it. Keep the last header per chunk stream id, and join chunks until a message's
length is read.

**Commands** (message type 20, AMF0 values one after another). In the order a publisher
sends them:

| It sends | You answer |
|---|---|
| `connect` (transaction 1, an object with `app`) | Window Acknowledgement Size (type 5), Set Peer Bandwidth (type 6), then `_result` (1, an object, an object with `level: "status"` and `code: "NetConnection.Connect.Success"`) |
| `releaseStream`, `FCPublish` | nothing is needed |
| `createStream` (transaction n) | `_result` (n, null, 1): the message stream id is 1 |
| `publish` (0, null, the key, `"live"`) | if the key is not yours, close; else `onStatus` (0, null, `level: "status"`, `code: "NetStream.Publish.Start"`) on message stream 1 |
| `FCUnpublish`, `deleteStream` | the publish ended: stop the copies |

AMF0: a number is `0x00` and an 8-byte double; a boolean `0x01` and a byte; a string
`0x02`, a 2-byte length and the bytes; null `0x05`; an object `0x03`, then pairs of a key
(2-byte length and bytes) and a value, ended by `0x00 0x00 0x09`. Each window of bytes
received, send an Acknowledgement (type 3, the 4-byte count).

**Media.** After the publish: type 18 is data (`@setDataFrame`, `onMetaData`), 8 is audio,
9 is video. To feed a copier, write the FLV header once (`FLV`, `0x01`, `0x05`, then 4
bytes of 9 and 4 zero bytes), then each message as a tag: its type (1 byte), its length (3),
its timestamp (3, then its high byte), 3 zero bytes, the data, and the tag's whole size (4).
A copy that starts late needs the first video message (the H.264 decoder configuration),
the first audio message (the AAC configuration) and the metadata: keep them, and write
them to every new copy before anything else.
