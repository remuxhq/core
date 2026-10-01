#!/usr/bin/env python3
"""A chat bridge of your own: Twitch and YouTube chat, on the engine's wire.

    set -a; . ~/.config/remux/platforms.env; set +a      # YOUTUBE_API_KEY
    python3 byo/bridge.py [--twitch <channel>] [--youtube <video id>] [--port 9999]
    remux chat url ws://127.0.0.1:9999

What is not on the command line is read from ~/.config/remux/config.toml
(REMUX_CONFIG names another), the `[byo]` table: `twitch`, `youtube`.

Serves the `line` half of docs/wire.md on a WebSocket: one JSON object per
frame, {"line": {id, platform, channel, from, body}}, to every connected
engine. Twitch is read over IRC, anonymously, no token; YouTube is polled
through the Data API with an API key (the video must be live, public or
unlisted). A `delete` coming up the wire is printed: taking a message down
on the platform needs a moderator's token, which this bridge does not hold.

A `say` coming up the wire (`remux chat say`) is posted as the broadcaster,
when byo.env holds what that takes; the line comes back down with the rest
of the chat. What goes wrong is told to the engine as a `notice`.

    TWITCH_CHAT_TOKEN   a user token with the chat:edit scope
    TWITCH_CHAT_NICK    whose token it is (default: the channel)
    YOUTUBE_CLIENT_ID, YOUTUBE_CLIENT_SECRET, YOUTUBE_REFRESH_TOKEN
                        OAuth for liveChatMessages.insert (youtube.force-ssl)

Standard library only, Python 3.9 or newer. Restart-safe: each source
reconnects on its own.
"""

import argparse
import base64
import hashlib
import json
import os
import queue
import socket
import ssl
import struct
import sys
import threading
import time
import urllib.parse
import urllib.request


def config():
    """The `[byo]` table of the engine's config, or nothing. Read by hand:
    the table is two quoted strings, and Python's toml reader is 3.11+ while
    a Mac's own python3 is 3.9."""
    path = os.environ.get("REMUX_CONFIG") or os.path.expanduser("~/.config/remux/config.toml")
    kept, table = {}, None
    try:
        with open(path, encoding="utf-8") as file:
            for raw in file:
                line = raw.split("#", 1)[0].strip()
                if line.startswith("["):
                    table = line.strip("[]").strip()
                elif table == "byo" and "=" in line:
                    key, value = (part.strip() for part in line.split("=", 1))
                    kept[key] = value.strip("\"'")
    except OSError:
        pass
    return kept

# ---- the wire: a WebSocket server, text frames only ----------------------

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
clients = []
clients_lock = threading.Lock()


def log(text):
    print(time.strftime("%H:%M:%S"), text, file=sys.stderr, flush=True)


def handshake(conn):
    request = b""
    while b"\r\n\r\n" not in request:
        chunk = conn.recv(4096)
        if not chunk:
            return False
        request += chunk
    key = None
    for line in request.decode(errors="replace").split("\r\n"):
        if line.lower().startswith("sec-websocket-key:"):
            key = line.split(":", 1)[1].strip()
    if not key:
        return False
    accept = base64.b64encode(hashlib.sha1((key + GUID).encode()).digest()).decode()
    conn.sendall(
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n"
        b"Connection: Upgrade\r\nSec-WebSocket-Accept: " + accept.encode() + b"\r\n\r\n"
    )
    return True


def frame(text):
    payload = text.encode()
    head = bytearray([0x81])
    n = len(payload)
    if n < 126:
        head.append(n)
    elif n < 65536:
        head += struct.pack("!BH", 126, n)
    else:
        head += struct.pack("!BQ", 127, n)
    return bytes(head) + payload


def read_frame(conn):
    """One frame from the engine: (opcode, text) or None when it hung up."""
    head = conn.recv(2)
    if len(head) < 2:
        return None
    opcode = head[0] & 0x0F
    masked = head[1] & 0x80
    n = head[1] & 0x7F
    if n == 126:
        n = struct.unpack("!H", conn.recv(2))[0]
    elif n == 127:
        n = struct.unpack("!Q", conn.recv(8))[0]
    mask = conn.recv(4) if masked else b""
    data = b""
    while len(data) < n:
        chunk = conn.recv(n - len(data))
        if not chunk:
            return None
        data += chunk
    if masked:
        data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
    return opcode, data


def serve_client(conn, who):
    try:
        if not handshake(conn):
            return
        with clients_lock:
            clients.append(conn)
        log(f"wire: engine connected from {who}")
        while True:
            got = read_frame(conn)
            if got is None:
                return
            opcode, data = got
            if opcode == 0x8:
                return
            if opcode == 0x9:
                conn.sendall(b"\x8a" + bytes([len(data)]) + data)
            elif opcode == 0x1:
                heard(data.decode(errors="replace"))
    except OSError:
        pass
    finally:
        with clients_lock:
            if conn in clients:
                clients.remove(conn)
        conn.close()
        log(f"wire: engine from {who} gone")


def send(down):
    """One frame to every connected engine."""
    data = frame(json.dumps(down, ensure_ascii=False))
    with clients_lock:
        for conn in list(clients):
            try:
                conn.sendall(data)
            except OSError:
                clients.remove(conn)


def broadcast(lines):
    while True:
        send({"line": lines.get()})


def tell(about, text, fine=False):
    """A notice to the engine, which lands in its log and in `remux events`."""
    log(f"{about}: {text}")
    send({"notice": {"about": about, "text": text, "fine": fine}})


# What the engine asks, up the wire. Filled by main: a channel's name (a
# Twitch channel, a YouTube video id) to the queue of the thread that posts
# there.
voices = {}


def heard(text):
    try:
        up = json.loads(text)
    except ValueError:
        return
    if not isinstance(up, dict):
        return
    if "delete" in up:
        log(f"wire: delete asked, not done here: {text}")
    said = up.get("say")
    if not isinstance(said, dict):
        return
    body = str(said.get("body") or "").strip()
    # The engine refuses these too; a newline here would be a second IRC
    # command on Twitch, so the bridge does not trust anybody to have checked.
    if not body or any(ord(c) < 32 or ord(c) == 127 for c in body):
        tell("say", "refused a line that was empty or not one line")
        return
    channel = said.get("channel")
    if channel:
        voice = voices.get(str(channel).lower().lstrip("#"))
        if voice is None:
            tell("say", f"no chat named {channel} here; these: {', '.join(voices) or 'none'}")
            return
        voice.put(body)
    elif not voices:
        tell("say", "nothing to say it on: --twitch and/or --youtube")
    else:
        for voice in voices.values():
            voice.put(body)


def serve(port, lines):
    threading.Thread(target=broadcast, args=(lines,), daemon=True).start()
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", port))
    server.listen(8)
    log(f"wire: ws://127.0.0.1:{port}  (remux chat url ws://127.0.0.1:{port})")
    while True:
        conn, addr = server.accept()
        threading.Thread(target=serve_client, args=(conn, f"{addr[0]}:{addr[1]}"), daemon=True).start()


# ---- Twitch: IRC, anonymous -------------------------------------------------


def irc_tags(prefix):
    tags = {}
    for part in prefix.split(";"):
        if "=" in part:
            key, value = part.split("=", 1)
            tags[key] = value.replace("\\s", " ").replace("\\:", ";")
    return tags


def twitch(channel, lines):
    channel = channel.lower().lstrip("#")
    while True:
        try:
            raw = socket.create_connection(("irc.chat.twitch.tv", 6697), timeout=360)
            conn = ssl.create_default_context().wrap_socket(raw, server_hostname="irc.chat.twitch.tv")
            conn.sendall(b"CAP REQ :twitch.tv/tags\r\n")
            conn.sendall(f"NICK justinfan{os.getpid() % 90000 + 10000}\r\n".encode())
            conn.sendall(f"JOIN #{channel}\r\n".encode())
            log(f"twitch: reading #{channel}")
            buffer = b""
            while True:
                chunk = conn.recv(4096)
                if not chunk:
                    raise OSError("twitch closed the connection")
                buffer += chunk
                while b"\r\n" in buffer:
                    message, buffer = buffer.split(b"\r\n", 1)
                    text = message.decode(errors="replace")
                    if text.startswith("PING"):
                        conn.sendall(b"PONG :tmi.twitch.tv\r\n")
                        continue
                    tags = {}
                    if text.startswith("@"):
                        prefix, text = text[1:].split(" ", 1)
                        tags = irc_tags(prefix)
                    if " PRIVMSG #" not in text:
                        continue
                    head, body = text.split(" :", 1) if " :" in text else (text, "")
                    nick = head.split("!", 1)[0].lstrip(":")
                    lines.put(
                        {
                            "id": tags.get("id") or f"twitch-{time.time_ns()}",
                            "platform": "twitch",
                            "channel": channel,
                            "from": tags.get("display-name") or nick,
                            "body": body,
                        }
                    )
        except (OSError, ssl.SSLError) as why:
            log(f"twitch: {why}; again in 5 s")
            time.sleep(5)


def twitch_say(channel, nick, token, says):
    """Post what the engine says in #channel, as `nick`, over IRC. A connection
    of its own, kept, answering PINGs: the reader stays anonymous and hears
    the line come back like anybody's."""
    channel = channel.lower().lstrip("#")
    token = token if token.startswith("oauth:") else f"oauth:{token}"
    state = {"conn": None}
    ready = threading.Event()
    lock = threading.Lock()

    def keep():
        while True:
            try:
                raw = socket.create_connection(("irc.chat.twitch.tv", 6697), timeout=360)
                conn = ssl.create_default_context().wrap_socket(raw, server_hostname="irc.chat.twitch.tv")
                conn.sendall(f"PASS {token}\r\nNICK {nick.lower()}\r\nJOIN #{channel}\r\n".encode())
                state["conn"] = conn
                buffer = b""
                while True:
                    chunk = conn.recv(4096)
                    if not chunk:
                        raise OSError("twitch closed the connection")
                    buffer += chunk
                    while b"\r\n" in buffer:
                        message, buffer = buffer.split(b"\r\n", 1)
                        text = message.decode(errors="replace")
                        if text.startswith("PING"):
                            with lock:
                                conn.sendall(b"PONG :tmi.twitch.tv\r\n")
                        elif f" JOIN #{channel}" in text:
                            ready.set()
                        elif " NOTICE " in text:
                            # A refusal: the token, the rate, a ban, slow mode.
                            tell("twitch", text.split(" :", 1)[-1])
                            if "authentication failed" in text or "Improperly formatted auth" in text:
                                raise OSError("the token was refused")
            except (OSError, ssl.SSLError) as why:
                ready.clear()
                state["conn"] = None
                log(f"twitch: saying: {why}; again in 15 s")
                time.sleep(15)

    threading.Thread(target=keep, daemon=True).start()
    while True:
        body = says.get()
        if not ready.wait(10) or state["conn"] is None:
            tell("twitch", f"not connected to #{channel} to say it; dropped")
            continue
        try:
            with lock:
                state["conn"].sendall(f"PRIVMSG #{channel} :{body}\r\n".encode())
            log(f"twitch: said in #{channel}")
        except OSError as why:
            tell("twitch", f"could not say it: {why}")


# ---- YouTube: the live chat, polled with an API key -------------------------

YOUTUBE = "https://www.googleapis.com/youtube/v3"


def youtube_get(path, key, **params):
    params["key"] = key
    with urllib.request.urlopen(f"{YOUTUBE}/{path}?{urllib.parse.urlencode(params)}", timeout=15) as answer:
        return json.load(answer)


# The live chat's id, found by the reader, for whoever says something there.
youtube_chat = {}


def youtube(video, key, lines):
    page = None
    while True:
        try:
            details = youtube_get("videos", key, part="liveStreamingDetails", id=video)
            items = details.get("items") or []
            chat_id = items[0].get("liveStreamingDetails", {}).get("activeLiveChatId") if items else None
            youtube_chat[video] = chat_id
            if not chat_id:
                log(f"youtube: {video} has no live chat yet (is it live?); again in 15 s")
                time.sleep(15)
                continue
            log(f"youtube: reading the chat of {video}")
            while True:
                params = {"liveChatId": chat_id, "part": "snippet,authorDetails", "maxResults": 200}
                if page:
                    params["pageToken"] = page
                answer = youtube_get("liveChat/messages", key, **params)
                if page is not None:
                    for item in answer.get("items", []):
                        snippet = item.get("snippet", {})
                        body = snippet.get("displayMessage")
                        if not body:
                            continue
                        lines.put(
                            {
                                "id": item["id"],
                                "platform": "youtube",
                                "channel": video,
                                "from": item.get("authorDetails", {}).get("displayName", "?"),
                                "body": body,
                            }
                        )
                page = answer.get("nextPageToken") or page or ""
                time.sleep(max(answer.get("pollingIntervalMillis", 5000), 3000) / 1000)
        except urllib.error.HTTPError as why:
            body = why.read().decode(errors="replace")[:200]
            if why.code in (403, 404):
                log(f"youtube: {why.code} {body}; the chat ended or the key is refused; again in 30 s")
                page = None
                time.sleep(30)
            else:
                log(f"youtube: {why.code}; again in 10 s")
                time.sleep(10)
        except OSError as why:
            log(f"youtube: {why}; again in 10 s")
            time.sleep(10)


def mute(about, needs, says):
    """A chat this bridge reads and cannot write: says so for every line."""
    while True:
        says.get()
        tell(about, f"reading only: saying needs {needs} in byo.env")


def youtube_say(video, client_id, secret, refresh, says):
    """Post what the engine says in the live chat of `video`, as the channel
    that owns the refresh token. An API key cannot write: this is OAuth, an
    access token minted from the refresh token and kept until it expires.
    Neither token is ever printed."""
    access = {"token": None, "until": 0.0}

    def token():
        if access["token"] and time.time() < access["until"]:
            return access["token"]
        form = urllib.parse.urlencode(
            {"client_id": client_id, "client_secret": secret,
             "refresh_token": refresh, "grant_type": "refresh_token"}
        ).encode()
        with urllib.request.urlopen("https://oauth2.googleapis.com/token", data=form, timeout=15) as answer:
            got = json.load(answer)
        access["token"] = got["access_token"]
        access["until"] = time.time() + int(got.get("expires_in", 3600)) - 60
        return access["token"]

    while True:
        body = says.get()
        chat_id = youtube_chat.get(video)
        if not chat_id:
            tell("youtube", f"{video} has no live chat to say it in (is it live?); dropped")
            continue
        message = {"snippet": {"liveChatId": chat_id, "type": "textMessageEvent",
                               "textMessageDetails": {"messageText": body}}}
        try:
            request = urllib.request.Request(
                f"{YOUTUBE}/liveChat/messages?part=snippet",
                data=json.dumps(message).encode(),
                headers={"Authorization": f"Bearer {token()}", "Content-Type": "application/json"},
            )
            with urllib.request.urlopen(request, timeout=15):
                pass
            log(f"youtube: said in {video}")
        except urllib.error.HTTPError as why:
            if why.code == 401:
                access["token"] = None
            reason = why.read().decode(errors="replace")
            try:
                reason = json.loads(reason)["error"]["message"]
            except (ValueError, KeyError, TypeError):
                reason = reason[:200]
            tell("youtube", f"could not say it: {why.code} {reason}")
        except (OSError, KeyError, ValueError) as why:
            tell("youtube", f"could not say it: {why}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    kept = config()
    parser.add_argument("--twitch", metavar="CHANNEL", default=kept.get("twitch"), help="a Twitch channel to read")
    parser.add_argument(
        "--youtube", metavar="VIDEO_ID", default=kept.get("youtube"),
        help="a live YouTube video to read (YOUTUBE_API_KEY in the environment)",
    )
    parser.add_argument("--port", type=int, default=9999)
    args = parser.parse_args()
    if not args.twitch and not args.youtube:
        parser.error("nothing to read: --twitch and/or --youtube, or [byo] in ~/.config/remux/config.toml")
    lines = queue.Queue()
    if args.twitch:
        threading.Thread(target=twitch, args=(args.twitch, lines), daemon=True).start()
        channel = args.twitch.lower().lstrip("#")
        token = os.environ.get("TWITCH_CHAT_TOKEN")
        voices[channel] = says = queue.Queue()
        if token:
            nick = os.environ.get("TWITCH_CHAT_NICK") or channel
            threading.Thread(target=twitch_say, args=(channel, nick, token, says), daemon=True).start()
        else:
            threading.Thread(target=mute, args=("twitch", "TWITCH_CHAT_TOKEN", says), daemon=True).start()
    if args.youtube:
        key = os.environ.get("YOUTUBE_API_KEY")
        if not key:
            parser.error("--youtube needs YOUTUBE_API_KEY in the environment, never on the command line")
        threading.Thread(target=youtube, args=(args.youtube, key, lines), daemon=True).start()
        oauth = [os.environ.get(name) for name in ("YOUTUBE_CLIENT_ID", "YOUTUBE_CLIENT_SECRET", "YOUTUBE_REFRESH_TOKEN")]
        voices[args.youtube.lower()] = says = queue.Queue()
        if all(oauth):
            threading.Thread(target=youtube_say, args=(args.youtube, *oauth, says), daemon=True).start()
        else:
            threading.Thread(
                target=mute, args=("youtube", "YOUTUBE_CLIENT_ID, _SECRET and _REFRESH_TOKEN", says), daemon=True
            ).start()
    try:
        serve(args.port, lines)
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
