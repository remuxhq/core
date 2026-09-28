# Testing

Two layers, each with a job. Before writing code, name which layer proves the
change.

| Layer | Runs | Proves | Cost |
|---|---|---|---|
| Unit: `cargo test -p remuxd-domain` | `make remuxd.check`, `make remuxd.test F=name` mid-loop | every decision, the parsers, the engine through fake ports | under a second |
| Security: `make security` | before a PR; the sec lane in CI | secrets, advisories, deny, over tracked files only | ~20 s |

## Never against production

A test live goes to a file (`REMUXD_RTMP`) or a platform's sandbox. A read-only probe is the check of a reader.

## Flakiness: zero tolerance

- Wait for effects, never for time. A check that passes on retry is a bug.
- A guard needs a negative control: a check that has only ever passed could be
  asserting nothing (the seam check, the manifest check).
- A unique name is not a timestamp: use a counter or the pid.
- A restart is a test case: state the engine holds on a face's behalf is a lease.
- A "not built yet" assertion is a thing you will move: name the verb, expect to
  repoint it, never soften it.

## Measuring

A glitch somebody saw is measured before it is chased, and the recording is the
reproduction. Pictures: crop the region, then `signalstats` per frame, bucketed per
ten seconds; ask the engine, not the file, for the source's rate. Sound: the meter
off the socket (`levels -f`), the ring's starvation count, an A/B under load, and
the operator's own voice, never `say`.
