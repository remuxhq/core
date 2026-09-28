---
name: release
description: Bump remux's version and write the release notes for people, from the diff since the last release. Use when asked to release, cut a version, bump the version, tag a release, or write release notes.
---

A release is a version bump, its notes, and a tag. The tag is what publishes:
`release.yml` builds every target and refuses a tag without
`docs/releases/<version>.md` or whose version disagrees with `engine/cli/Cargo.toml`.

## 1. What changed since the last release

```sh
last=$(gh release view --json tagName --jq .tagName)   # e.g. v0.1.1
git fetch -q origin
git log --oneline "$last"..origin/main
git diff --stat "$last"..origin/main
```

Read the merged PRs' descriptions (`gh pr list --state merged --search "merged:>$(gh release view --json publishedAt --jq .publishedAt)"`)
and the diff of anything a person runs: the CLI's words and help, `install*.sh`,
the engine's behaviour. Nothing changed for a person: say so and stop.

## 2. The version

Patch for fixes and changes a person does not have to learn. Minor for a new verb,
a changed word, or a new behaviour a person notices (0.x: minor is also the
breaking one). Unsure: ask, with patch as the default.

## 3. The bump

Every crate moves together, whatever version it was at:

```sh
new=0.1.2
for f in engine/*/Cargo.toml; do sed -i '' "3s/^version = \".*\"/version = \"$new\"/" "$f"; done
sed -i '' "s/instead of the latest ([0-9.]*)/instead of the latest ($new)/" install.sh
(cd engine && cargo update -w --offline -q && cargo update -w --offline -q --manifest-path motor-obs/Cargo.toml)
grep -n '^version = ' engine/*/Cargo.toml          # all $new
```

## 4. The notes: `docs/releases/<version>.md`

For a person who uses remux, not for whoever wrote the code. The shape of
`docs/releases/0.1.1.md`:

```
remux 0.1.2.

- <what a person can do now, or what works now>: <why, in one clause>.
- ...
```

- One bullet per change a person notices, most important first. Five is plenty.
- Say what it does, not how: "the gate's keys boost and floor act now", not "hosted
  under the Filter contract".
- A new build prerequisite goes in a last bullet, "For developers: ...".
- Every bullet is checked against the diff before it is written. Nothing is claimed
  that the code does not do.
- No commit list, no PR numbers, no em dashes, no promotional words.

## 5. The gate, the PR, the tag

```sh
git switch -c release/$new
make remuxd.check && make security
git add engine/*/Cargo.toml engine/Cargo.lock engine/motor-obs/Cargo.lock install.sh docs/releases/$new.md
git commit -m "remux $new"
git push -u origin release/$new && gh pr create --title "remux $new" --body-file docs/releases/$new.md
gh pr checks --watch
```

After the PR is merged, and only when the person says to publish (a tag is public
and cannot be taken back quietly):

```sh
git switch main && git pull --ff-only
git tag -a v$new -m "remux $new" && git push origin v$new
gh run watch "$(gh run list -w release -L 1 --json databaseId --jq '.[0].databaseId')" --exit-status
gh release view v$new --json assets --jq '.assets[].name'   # a tarball and a .sha256 per target
```
