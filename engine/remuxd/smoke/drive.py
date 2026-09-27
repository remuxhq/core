#!/usr/bin/env python3
"""Drive a real remuxd over its real socket.

The harness for every parity item. It starts the daemon, waits for the
readiness line it prints once the socket is bound and accepting, and then
speaks the protocol. It never sleeps: waiting on a duration is waiting on a
guess, and a guess that is usually long enough is a flake in disguise.

    ./engine/remuxd/smoke/drive.py '{"cmd":"devices"}' '{"cmd":"status"}'
    ./engine/remuxd/smoke/drive.py --binary engine/target/release/remuxd '{"cmd":"status"}'
"""

import argparse
import json
import os
import socket
import subprocess
import sys
import tempfile
import time


class Daemon:
    #: How many daemons this process has started. The pid alone is not a unique
    #: name: a check that wants a second engine beside the shared one draws the
    #: same path and the second daemon correctly refuses to steal a socket
    #: somebody is listening on. A counter cannot collide.
    started = 0

    def __init__(self, binary="engine/target/debug/remuxd", **env):
        Daemon.started += 1
        self.socket_path = os.path.join(
            tempfile.gettempdir(), f"smoke{os.getpid()}-{Daemon.started}.sock"
        )
        # Its own preferences file, beside its own socket. Without this every
        # check restores whatever the person running them last set up, which on
        # a real machine means opening their camera before it will answer.
        # And its own files for everything else the engine keeps, so a smoke
        # never writes a live into the person's history.
        own = {
            "REMUXD_SOCKET": self.socket_path,
            "REMUXD_PREFS": f"{self.socket_path}.prefs.json",
            "REMUX_DESTINATIONS": f"{self.socket_path}.destinations.json",
            "REMUX_HISTORY": f"{self.socket_path}.history.jsonl",
            "REMUX_SESSION": f"{self.socket_path}.session.json",
            "REMUX_CONFIG": f"{self.socket_path}.config.toml",
            # The music the smokes play is the repository's own folder (gitignored),
            # never the person's ~/Music/remux.
            "REMUX_MUSIC_DIR": os.environ.get("REMUX_MUSIC_DIR", "music"),
        }
        self.env = env = {**os.environ, **own, **env}
        self.proc = subprocess.Popen(
            [binary], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
        )
        # The readiness protocol. This blocks for exactly as long as binding
        # takes and not a millisecond of anyone's guess.
        # libobs talks on stdout before the daemon does; the announcement is
        # the first line that is the daemon's own.
        line = self.proc.stdout.readline()
        while line and not line.startswith("listening "):
            line = self.proc.stdout.readline()
        if not line.startswith("listening "):
            # It died before it could speak, and its own complaint is the
            # useful thing here, not the empty line we were handed.
            self.proc.kill()
            raise SystemExit(
                f"remuxd did not announce itself (got {line!r}); it said: "
                f"{self.proc.stderr.readline().strip()}"
            )
        self.conn = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.conn.settimeout(10)
        self.conn.connect(self.socket_path)
        self.io = self.conn.makefile("rw")

    def ask(self, command):
        """One command, one reply."""
        self.io.write(json.dumps(command) + "\n")
        self.io.flush()
        return json.loads(self.io.readline())

    def until(self, condition, what, budget=8.0, every=0.05):
        """Poll the engine until something is true, or give up saying what.

        This is not "wait for a duration": the loop asks the daemon and stops
        the moment the answer changes, so a fast machine is fast. The pause
        between asks is there because a frame takes tens of milliseconds to
        exist and hammering a mutex thousands of times proves nothing about
        whether one arrived. The budget is generous on purpose (eight seconds
        against a capture that should start in well under one) so that it can
        only fire on something genuinely stuck.
        """
        deadline = time.monotonic() + budget
        last = None
        while time.monotonic() < deadline:
            last = condition()
            if last:
                return last
            time.sleep(every)
        raise TimeoutError(f"{what} did not happen in {budget}s (last saw {last!r})")

    def close(self):
        try:
            self.ask({"cmd": "quit"})
            self.proc.wait(timeout=5)
        except Exception:
            self.proc.kill()
        finally:
            if os.path.exists(self.socket_path):
                os.unlink(self.socket_path)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="engine/target/debug/remuxd")
    parser.add_argument("commands", nargs="*")
    args = parser.parse_args()

    with Daemon(args.binary) as daemon:
        for raw in args.commands:
            began = time.monotonic()
            reply = daemon.ask(json.loads(raw))
            took = (time.monotonic() - began) * 1000
            print(f"{raw}  ->  {json.dumps(reply)[:400]}   [{took:.0f}ms]")


if __name__ == "__main__":
    main()
