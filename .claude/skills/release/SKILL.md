---
name: release
description: Bump remux's version and write the release notes for people, from the diff since the last release. Use when asked to release, cut a version, bump the version, tag a release, or write release notes.
---

`docs/release.md` is the process: read it and follow it, step by step. It says how
to find what changed since the last release, which version to pick, how every
crate moves together, the shape of the notes, and the tag.

What is an agent's alone:

- Only an owner cuts a release (`.github/CODEOWNERS`). When the person asking is not
  one, stop and say so; `release guard` would refuse the pull request anyway.
- Read the diff before writing a bullet, and write none the diff does not show.
- The version is the person's: the one they name, or a question when they name none.
  A pre-release (`0.3.0-rc.1`) when they want people to try it first; it leaves the
  `install.sh` example alone.
- The branch is `release/<version>`; any other name fails `release guard`.
- Stop after the pull request is green, `integration` included. The approval is the
  other owner's and the merge is the person's: merging publishes (step 6). Never tag by
  hand, never dispatch to publish a new version.
- Once it is merged, watch `release.yml` and report the release's assets (a tarball
  and a `.sha256` per target), whether it is a pre-release, and the attestation check.
