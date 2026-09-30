---
name: remux
description: Drive a live (screen, camera, mic, music, Twitch and YouTube) through the remux CLI. Use when asked to set up, start, watch or stop a live from this machine.
---

Run `remux guide` first: it is the whole contract, eight steps. Every verb takes
`--json`; `remux schema` prints the shapes; exit code 1 is a refusal with the reason
on stderr, 2 wrong words. Never go live for real: `remux destination sandbox <id> on` before
arming a real platform (with no account it reaches Twitch alone; elsewhere rehearse on a
private broadcast), and `remux plan` before `remux live --confirm <fingerprint>`.
A key is never on a command line (`--key -`, `--key-file`).
