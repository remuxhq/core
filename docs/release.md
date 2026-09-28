# Releasing remux

A release is orchestrated, not automated. A person decides to cut one, the
`release` skill walks the steps below, and nothing publishes until that person says
so. There is one way to publish: dispatching `release.yml` by hand. A push, a tag or
a pull request never starts it.

A release is three things: every crate at the new version, the notes for people in
`docs/releases/<version>.md`, and the published release. Dispatched with a version,
`release.yml` checks it, builds the tarball for every target (macOS on Apple silicon,
Linux x86_64 and aarch64), then tags the commit it built as `v<version>` and publishes
the release with each tarball, its `.sha256`, and the notes as its body.
`install.sh` installs the latest release by default.

The workflow refuses a version whose `docs/releases/<version>.md` is missing or
empty, that disagrees with `engine/cli/Cargo.toml`, or that is already released.
That refusal is the gate: the notes are the record of why a release is what it is.

## 1. What changed since the last release

```sh
last=$(gh release view --json tagName --jq .tagName)   # the latest, e.g. v0.1.1
git fetch -q origin
git log --oneline "$last"..origin/main
git diff --stat "$last"..origin/main
```

Read the merged pull requests and the diff of anything a person meets: the CLI's
words and help, `install*.sh`, what the engine does on the air. If nothing changed
for a person, there is nothing to release.

## 2. The version

- **Patch** (0.1.1 to 0.1.2): fixes, and changes a person does not have to learn.
- **Minor** (0.1 to 0.2): a new command, a changed word, a behaviour a person
  notices. While remux is 0.x, a breaking change is a minor too.

When unsure, patch.

## 3. The bump

Every crate in `engine/` moves to the new version together, whatever version it was
at, and so does the example in `install.sh`:

```sh
new=0.1.2
for f in engine/*/Cargo.toml; do sed -i '' "3s/^version = \".*\"/version = \"$new\"/" "$f"; done
sed -i '' "s/instead of the latest ([0-9.]*)/instead of the latest ($new)/" install.sh
(cd engine && cargo update -w --offline -q && cargo update -w --offline -q --manifest-path motor-obs/Cargo.toml)
grep -n '^version = ' engine/*/Cargo.toml        # every line says $new
```

`version` is the third line of every manifest; a manifest that moves it breaks the
loop above, and the `grep` says so. `motor-obs` is a workspace of its own, hence
its own lockfile.

## 4. The notes

`docs/releases/<version>.md`, for a person who uses remux, not for whoever wrote
the code:

```
remux 0.1.2.

- <what a person can do now, or what works now>: <why, in one clause>.
- ...
- For developers: <a new build prerequisite, when there is one>.
```

- One bullet per change a person notices, the most important first. Five is plenty.
- What it does, not how: "the gate's keys boost and floor act now", not "hosted under
  the Filter contract".
- Every bullet is checked against the diff. Nothing is claimed that the code does not do.
- No commit list, no pull request numbers, no em dashes, no promotional words.

## 5. The pull request

```sh
git switch -c release/$new
make remuxd.check && make security
git add engine/*/Cargo.toml engine/Cargo.lock engine/motor-obs/Cargo.lock install.sh docs/releases/$new.md
git commit -m "remux $new"
git push -u origin release/$new
gh pr create --title "remux $new" --body-file docs/releases/$new.md
gh pr checks --watch
```

## 6. Publishing

After the pull request is merged, and only when the person says to publish:

```sh
gh workflow run release.yml --ref main -f version=$new
sleep 5; run=$(gh run list -w release.yml -L 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$run" --exit-status
gh release view v$new --json assets --jq '.assets[].name'   # a tarball and a .sha256 per target
```

The workflow makes the tag; nobody tags by hand. `make release` builds this
machine's tarball into `dist/` and `make release.install` installs it, for trying a
release before publishing it.
