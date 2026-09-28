---
name: release
description: Bump remux's version and write the release notes for people, from the diff since the last release. Use when asked to release, cut a version, bump the version, tag a release, or write release notes.
---

`docs/release.md` is the process: read it and follow it, step by step. It says how
to find what changed since the last release, which version to pick, how every
crate moves together, the shape of the notes, and the tag.

What is an agent's alone:

- Read the diff before writing a bullet, and write none the diff does not show.
- The version is a question for the person when the diff leaves it unclear; patch is
  the default.
- Stop after the pull request is green. Publishing is dispatching `release.yml`
  (step 6), done only when the person says so in this conversation, after the pull
  request is merged. Never tag by hand.
- Report the release's assets once the workflow is green: a tarball and a `.sha256`
  per target.
