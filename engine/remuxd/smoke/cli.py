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
EFFECT = "$PWD/engine/motor-obs/examples/invert.wgsl"
SAYS = [
    ("status", "off air"),
    # the same, every line of it, and for a program
    ("status -v", "opens at -20 dB"),
    ("status --json", '"reply":"status"'),
    ("help", "Usage: remux"),
    ("help scene layer", "Usage: remux scene layer"),
    ("destination list", "no destinations"),
    ("log", None),
    ("config", "config"),
    ("bug", "remux 0.1.0"),
    ("daemon path", "service "),
    ("daemon status", "answering"),
    ("audio gate", "opens at "),
    ("audio gate opens -30", None),
    ("audio gate reset", None),
    ("schema", '"command"'),
    ("plan", "plan "),
    # The picture: generated layers need no device, so they round-trip anywhere.
    ("scene list", "* default"),
    ("scene layer add text title 200 200 1000 160 Welcome", 'text "Welcome"'),
    ("scene layer add timer clock 700 450 520 160 180", "timer 180s"),
    ("scene layer set text title 200 200 1000 160 Hello", 'text "Hello"'),
    ("scene timer start clock", None),
    ("scene layer shot title", "bytes of jpeg"),
    (f"scene layer filter title {EFFECT}", None),
    (f"scene filter {EFFECT}", "filter "),
    ("scene shot", "bytes of jpeg"),
    ("scene layer move clock 0", None),
    ("scene status --json", '"order":["clock","title"]'),
    ("scene layer hide title", None),
    ("scene layer show title", None),
    ("scene duplicate copy", None),
    ("scene list", "* copy (2 layers)"),
    ("scene create talk", None),
    ("scene list", "* talk (0 layers)"),
    ("scene switch default", None),
    ("scene delete talk", None),
    ("scene delete copy", None),
    ("scene filter off", None),
    ("scene layer filter title off", None),
    ("scene timer stop clock", None),
    ("scene layer remove clock", None),
    ("scene layer remove title", "no layers"),
    ("audio denoise", None),
    ("audio denoise off", None),
    ("audio clips", None),
    ("audio hear off", None),
    ("destination add custom vps --url rtmp://localhost/live --key-file $REMUX_CLI_KEYFILE", "1 vps"),
    ("destination list", "vps"),
    ("plan", "vps"),
    ("destination rm vps", "forgotten"),
    ("guide", "remux CLI guide"),
    ("wait off-air --for 1", "off air"),
    ("history", "no lives on record"),
    ("plan --json", '"fingerprint"'),
    # the meters on their own, which is what a panel asks twelve times a second
    ("levels", "mic "),
    ("audio levels", "mic "),
    ("devices", "screens:"),
    ("grants", "screen"),
    ("chat read", "no chat wire"),
    ("chat hide 42", None),
    ("audio mute", "muted"),
    ("audio mute off", None),
    ("audio vol 150", None),
    ("music vol 40", None),
    ("audio duck 18", None),
    ("audio monitor off", None),
    ("audio monitor", None),
    ("audio monitor off", None),
    ("music stream off", None),
    ("music stream", None),
    ("audio screen-sound off", None),
    ("music off", None),
    ("audio gate full 0.2", None),
    ("cut", None),
    ("audio mic off", None),
    ("record start", None),
    ("record stop", None),
]

#: Verbs that are meant to be refused, and the exit code that says which kind
#: of refusal it is: 2 for "you typed something that is not a verb", 1 for "the
#: engine understood and said no". A CLI that answers both with the same code
#: cannot be used in a script.
REFUSES = [
    ("fly", 2, "not a command"),
    # The flat words and the picture's old verbs are gone from the shell.
    ("arm 2", 2, "not a command"),
    ("screen 1", 2, "not a command"),
    ("layout tl", 2, "not a command"),
    ("card starting", 2, "not a command"),
    ("audio gate loudness 3", 2, "keys_boost"),
    ("audio gate nonsense", 2, "remux audio gate opens -30"),
    ("audio clip", 2, "a clip's name"),
    ("audio hear no-such-app-anywhere", 1, "not running"),
    ("audio hear", 2, "app names"),
    ("audio screen-sound", 1, "no display layer"),
    ("scene switch nope", 1, "no scene"),
    ("scene delete default", 1, "cannot delete the active scene"),
    ("scene filter /no-such-remux-filter.wgsl", 1, "cannot open filter"),
    ("scene layer add text late 1900 0 100 100 Hi", 2, "must fit"),
    ("scene layer transform nobody 0 0 10 10 0", 1, "no layer"),
    ("scene layer add window w no-such-window-anywhere", 1, "no window matches"),
    ("destination add twitch main --key live_abc", 2, "never typed"),
    ("destination rm nobody", 1, "no destination"),
    ("login --url", 2, "login takes"),
    ("logout", 0, "not signed in"),
    ("health", 1, "no picture"),
    ("wait on-air --for 1", 1, "still waiting"),
    ("wait sideways", 2, "wait takes"),
    ("audio clip no-such-clip", 1, "no clip called"),
    ("destination title 2", 2, "needs the words"),
    ("destination describe two words", 2, "a number"),
    # Understood, and refused for want of an app to ask.
    ("destination title 2 Rust at midnight", 1, "needs an account: remux login"),
    ("destination announce 2", 1, "needs an account: remux login"),
    ("destination announce", 2, "destination id"),
    ("destination describe 2 the engine, live", 1, "needs an account: remux login"),
    ("live --yes", 1, "nowhere to send"),
    ("live --confirm 1", 1, "the plan changed"),
    ("live --confirm", 2, "the number the plan printed"),
    # nothing armed and no picture: the plan says so and the shell stops there
    ("live", 1, "no destination is armed"),
    # The engine understood these perfectly and said no, which is the answer.
    ("music next", 1, "no genre is playing"),
    ("help nope", 2, "not a command"),
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
