#!/usr/bin/env python3
"""Every verb the CLI offers, round-tripped against a real daemon.

The grammar is unit tested in `remuxd_domain::cli` and so is the rendering;
what cannot be tested there is that the words reach an engine and that it
answers them. So this is deliberately shallow and wide: one of everything,
asserting only that it was understood and answered, because what each verb
*does* is what the parity suite next door is for.

    make remuxd.cli
"""

import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from drive import Daemon  # noqa: E402

REMUX = "engine/target/release/remux"
REMUXD = os.environ.get("REMUXD_BIN", "engine/target/release/remuxd")

#: The verb, and what a person should be able to read in the answer. `None`
#: where the answer is a status line that says nothing specific back.
SAYS = [
    ("status", "off air"),
    # the same, every line of it, and for a program
    ("status -v", "opens at -20 dB"),
    ("status --json", '"reply":"status"'),
    ("destinations", "no destinations"),
    ("log", None),
    ("config", "config"),
    ("bug", "remux 0.1.1"),
    ("daemon path", "service "),
    ("daemon status", "answering"),
    ("gate", "opens at "),
    ("gate opens -30", None),
    ("gate reset", None),
    ("words brb Back in five", None),
    ("schema", '"command"'),
    ("plan", "plan "),
    ("layout", "camera bottomright, 25% wide"),
    ("layout tl 50% circle", "camera topleft, 50% wide, circle"),
    ("layout br 25% rect", None),
    ("layout sepia", "sepia"),
    ("layout columns", "columns"),
    ("layout bounce", "bounce"),
    ("layout overlay", None),
    ("layout plain", None),
    ("scenes", "no scenes kept"),
    ("scene save code", "* code"),
    ("scene code", "* code"),
    ("scene rm code", None),
    ("denoise", None),
    ("denoise off", None),
    ("clips", None),
    ("hear off", None),
    ("destination add custom vps --url rtmp://localhost/live --key-file $REMUX_CLI_KEYFILE", "1 vps"),
    ("destinations", "vps"),
    ("plan", "vps"),
    ("destination rm vps", "forgotten"),
    ("guide", "remux, from a script"),
    ("panic", None),
    ("wait off-air --for 1", "off air"),
    ("history", "no lives on record"),
    ("plan --json", '"fingerprint"'),
    # the meters on their own, which is what a panel asks twelve times a second
    ("levels", "mic "),
    # a window says it is drawing the preview, so the engine starts making one
    ("watching", None),
    ("watching off", None),
    ("devices", "screens:"),
    ("shot", "bytes of jpeg"),
    ("grants", "screen"),
    ("chat", "no chat wire"),
    ("hide 42", None),
    ("mute", "muted"),
    ("mute off", None),
    ("vol 150", None),
    ("mvol 40", None),
    ("duck 18", None),
    ("monitor off", None),
    ("monitor", None),
    ("monitor off", None),
    ("stream-music off", None),
    ("stream-music", None),
    ("screen-sound", None),
    ("screen-sound off", None),
    ("music off", None),
    ("mirror", None),
    ("gate full 0.2", None),
    ("share off", None),
    ("card starting", "StartingSoon"),
    ("countdown 5", "StartingSoon"),
    ("card live", None),
    ("cut", "BackInAMoment"),
    ("camera off", None),
    ("mic off", None),
    ("record start", None),
    ("record stop", None),
]

#: Verbs that are meant to be refused, and the exit code that says which kind
#: of refusal it is: 2 for "you typed something that is not a verb", 1 for "the
#: engine understood and said no". A CLI that answers both with the same code
#: cannot be used in a script.
REFUSES = [
    ("fly", 2, "not a thing"),
    ("screen VG2791R", 2, "display id"),
    ("card purple", 2, "not a card"),
    ("gate loudness 3", 2, "keys_boost"),
    ("gate nonsense", 2, "remux gate opens -30"),
    ("layout sideways", 2, "not a corner"),
    ("play", 2, "a clip's name"),
    ("scene nope", 1, "no scene called"),
    ("hear no-such-app-anywhere", 1, "not running"),
    ("hear", 2, "app names"),
    ("destination add twitch main --key live_abc", 2, "never typed"),
    ("destination rm nobody", 1, "no destination"),
    ("login --url", 2, "login takes"),
    ("logout", 0, "not signed in"),
    ("health", 1, "no picture"),
    ("wait on-air --for 1", 1, "still waiting"),
    ("wait sideways", 2, "wait takes"),
    ("scene", 2, "a name"),
    ("play no-such-clip", 1, "no clip called"),
    ("title 2", 2, "needs the words"),
    ("describe two words", 2, "a number"),
    # Understood, and refused for want of an app to ask.
    ("title 2 Rust at midnight", 1, "needs an account: remux login"),
    ("announce 2", 1, "needs an account: remux login"),
    ("announce", 2, "destination id"),
    ("describe 2 the engine, live", 1, "needs an account: remux login"),
    ("live --yes", 1, "nowhere to send"),
    ("live --confirm 1", 1, "the plan changed"),
    ("live --confirm", 2, "the number the plan printed"),
    # nothing armed and no picture: the plan says so and the shell stops there
    ("live", 1, "no destination is armed"),
    # The engine understood these perfectly and said no, which is the answer.
    ("window no-such-window-anywhere", 1, "no window matches"),
    ("next", 1, "no genre is playing"),
]


def run(daemon, line):
    return subprocess.run(
        [REMUX, *[os.path.expandvars(w) for w in line.split()]],
        env=dict(os.environ, REMUXD_SOCKET=daemon.socket_path),
        capture_output=True,
        text=True,
    )


def main():
    failures = 0
    # The destinations file of this run alone, never the operator's.
    kept = os.path.join(tempfile.gettempdir(), f"remux-cli-{os.getpid()}-destinations.json")
    os.environ["REMUX_DESTINATIONS"] = kept
    os.environ["REMUX_SESSION"] = os.path.join(tempfile.gettempdir(), f"remux-cli-{os.getpid()}-session.json")
    os.environ["REMUX_HISTORY"] = os.path.join(tempfile.gettempdir(), f"remux-cli-{os.getpid()}-history.jsonl")
    keyfile = os.path.join(tempfile.gettempdir(), f"remux-cli-{os.getpid()}-key")
    with open(keyfile, "w") as f:
        f.write("not-a-real-key\n")
    os.environ["REMUX_CLI_KEYFILE"] = keyfile
    with Daemon(REMUXD, REMUX_DESTINATIONS=kept) as daemon:
        for line, expected in SAYS:
            got = run(daemon, line)
            said = (got.stdout + got.stderr).strip()
            if got.returncode != 0:
                print(f"  FAIL  remux {line:<28} exited {got.returncode}: {said[:70]}")
                failures += 1
            elif expected and expected not in said:
                print(f"  FAIL  remux {line:<28} said {said[:60]!r}, wanted {expected!r}")
                failures += 1
            else:
                print(f"  ok    remux {line:<28} {(said.splitlines() or [""])[0][:60]}")

        for line, code, expected in REFUSES:
            got = run(daemon, line)
            said = (got.stdout + got.stderr).strip()
            if got.returncode != code or expected not in said:
                print(
                    f"  FAIL  remux {line:<28} exited {got.returncode} "
                    f"(wanted {code}) saying {said[:50]!r}"
                )
                failures += 1
            else:
                print(f"  ok    remux {line:<28} refused, {code}: {said.splitlines()[0][:44]}")

    total = len(SAYS) + len(REFUSES)
    print()
    print(f"{total - failures}/{total} verbs round-tripped")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
