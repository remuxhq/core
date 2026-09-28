# Releasing remux

A release is a pull request. A person decides to cut one and names the version, the
`release` skill walks the steps below, and merging the pull request publishes it:
`release.yml` runs after every green `ci` on main, and publishes the version
`engine/cli/Cargo.toml` says unless it is already released. A red main publishes
nothing.

A release is three things: every crate at the new version, the notes for people in
`docs/releases/<version>.md`, and the published release. On that run,
`release.yml` checks the version, builds the tarball for every target (macOS on Apple silicon,
Linux x86_64 and aarch64), then tags the commit it built as `v<version>` and publishes
the release with each tarball, its `.sha256`, and the notes as its body.
`install.sh` installs the latest release by default.

The workflow refuses a version whose `docs/releases/<version>.md` is missing or
empty, and leaves one already released alone.
That refusal is the gate: the notes are the record of why a release is what it is.

## 1. What changed since the last release

```sh
last=$(gh release view --json tagName --jq .tagName)   # the latest, e.g. v0.1.1
git fetch -q origin
git log --oneline "$last"..origin/main
git diff --stat "$last"..origin/main
```

Read the merged pull requests and the diff of anything a person meets: the CLI's
words and help, `install*.sh`, what the engine does on the air. A release with
nothing changed for a person is still a release when the person asks for one; its
notes say what changed inside.

## 2. The version

The person who asks for the release names it. When they name none, ask:

- **Patch** (0.1.1 to 0.1.2): fixes, and changes a person does not have to learn.
- **Minor** (0.1 to 0.2): a new command, a changed word, a behaviour a person
  notices. While remux is 0.x, a breaking change is a minor too.

Versions only go up, and a version on main is fixed once merged, published or not:
its number and its notes are never renamed or rewritten. What comes after it is the
next version, with notes of its own.

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

`docs/releases/<version>.md`, one file per version, for the people who use remux:

```
remux 0.1.2.

- <what a person can do now, or what works now>: <why, in one clause>.
- ...
- For developers: <a new build prerequisite, when there is one>.
```

When nothing changed for a person, the notes say what changed in the architecture,
one bullet per change, and end with "Nothing changes for a person using remux."

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

Merging the pull request publishes it once `ci` is green on main. Then watch the run:

```sh
git fetch -q origin main; sha=$(git rev-parse origin/main)   # the merge
# The run starts once ci on that commit is green, minutes later.
until run=$(gh run list -w release.yml -c "$sha" --json databaseId --jq '.[0].databaseId') && [ -n "$run" ]; do sleep 30; done
gh run watch "$run" --exit-status
gh release view v$new --json assets --jq '.assets[].name'   # a tarball and a .sha256 per target
```

A run that failed (a runner, the network) runs again with
`gh workflow run release.yml --ref main`, which publishes the version on main unless
it is already released. The workflow makes the tag; nobody tags by hand. `make
release` builds this machine's tarball into `dist/` and `make release.install`
installs it, for trying a release before publishing it.
