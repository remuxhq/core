---
name: release
description: Bump remux's version and write the release notes for people, from the diff since the last release. Use when asked to release, cut a version, bump the version, tag a release, or write release notes.
---

`docs/release.md` is the process: read it and follow it, step by step. It says how
to find what changed since the last release, which version to pick, how every
crate moves together, the shape of the notes, and the tag.

What is an agent's alone:

- Read the diff before writing a bullet, and write none the diff does not show.
- The version is the person's: the one they name, or a question when they name none.
- Stop after the pull request is green. Merging it publishes (step 6); the merge is
  the person's. Never tag by hand, never dispatch to publish a new version.
- Once it is merged, watch `release.yml` and report the release's assets: a tarball
  and a `.sha256` per target.
