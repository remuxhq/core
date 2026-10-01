# Releasing remux

A release is a pull request, and only an owner cuts one: the people named in
`.github/CODEOWNERS`. The owner names the version, the `release` skill walks the
steps below and opens the pull request from a `release/<version>` branch, and the
merge publishes it.

A release pull request runs more than any other. Every pull request runs `lint`,
`unit` (macOS and Linux) and `sec`, none of which needs OBS. A `release/` branch also
runs `integration`: the libobs motor, its clippy and its tests, and the daemon the
socket tests spawn, on macOS against the OBS the release builds with and on Linux
against the distribution's. `release guard` refuses it when the author is not an
owner, the version does not move, or the notes are missing. The other owner approves,
and the merge is the release.

The merge touches `engine/cli/Cargo.toml`, which runs `release.yml`: it checks the
version and the notes, builds the tarball for every target (macOS on Apple silicon,
Linux x86_64 and aarch64) from the commit that set the version, attests each one,
tags that commit as `v<version>` and publishes the release with each tarball, its
`.sha256` and the notes as its body. A version already released is left alone.
`install.sh` and the site install the latest release, which GitHub defines as the
newest one that is neither a draft nor a pre-release.

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

### A pre-release

For people to try a version before it is the one everybody gets, the version carries a
pre-release part, as semver orders them: `0.3.0-beta.1` while it still changes,
`0.3.0-rc.1` when it is meant to be the release. It goes through the same pull
request, the same `integration` and the same notes, and `release.yml` publishes it as
a pre-release that never becomes the latest: `install.sh` and the site keep the stable
one. Whoever tries it asks for it by name:

```sh
curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install.sh | REMUX_VERSION=0.3.0-rc.1 sh
```

The stable `0.3.0` that follows is a release of its own, with its own notes.

## 3. The bump

Every crate in `engine/` moves to the new version together, whatever version it was
at, and so does the example in `install.sh`:

```sh
new=0.1.2
for f in engine/*/Cargo.toml; do sed -i '' "3s/^version = \".*\"/version = \"$new\"/" "$f"; done
sed -i '' "s/instead of the latest ([0-9.]*)/instead of the latest ($new)/" install.sh   # a stable version only
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

By an owner, from a `release/` branch: anything else fails `release guard`.

```sh
git switch -c release/$new
make remuxd.check && make security
git add engine/*/Cargo.toml engine/Cargo.lock engine/motor-obs/Cargo.lock install.sh docs/releases/$new.md
git commit -m "remux $new"
git push -u origin release/$new
gh pr create --title "remux $new" --body-file docs/releases/$new.md
gh pr checks --watch     # lint, unit, sec, integration (macos, linux), release guard
```

The other owner reads the notes against the diff and approves.

## 6. Publishing

Merging the pull request publishes it. Then watch the run:

```sh
git fetch -q origin main; sha=$(git rev-parse origin/main)   # the merge
until run=$(gh run list -w release.yml -c "$sha" --json databaseId --jq '.[0].databaseId') && [ -n "$run" ]; do sleep 10; done
gh run watch "$run" --exit-status
gh release view v$new --json assets,isPrerelease --jq '.isPrerelease, .assets[].name'   # a tarball and a .sha256 per target
gh attestation verify remux-$new-aarch64-apple-darwin.tar.gz -R remuxhq/core   # after gh release download v$new
```

A run that failed (a runner, the network) runs again with
`gh workflow run release.yml --ref main`, by an owner, which publishes the version on
main unless it is already released. The workflow makes the tag; nobody tags by hand. `make
release` builds this machine's tarball into `dist/` and `make release.install`
installs it, for trying a release before publishing it.
