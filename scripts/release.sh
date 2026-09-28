#!/bin/sh
# Build the release tarball of remux for the machine this runs on: the CLI
# and the engine on the libobs motor (the GPL build), the docs, the BYO
# files, the scripts, and the tarball's sha256 beside it. The version is
# engine/cli/Cargo.toml's; the release workflow, dispatched by hand, runs this
# on every target and publishes them under the tag of the same number.
#
#   sh scripts/release.sh [dist dir]      -> dist/remux-<version>-<target>.tar.gz
#
# One target per machine; CI runs it on every runner of the matrix (macOS
# Apple silicon, Linux x86_64 and aarch64), a laptop runs it for
# its own target, a VM the same way. What each target needs:
#   macOS   the OBS.app the engine links, fetched and pinned by `make obs.fetch`
#           (OBS_APP names another); signed with SIGN_ID when that identity is
#           in the keychain, ad hoc otherwise (then macOS asks for Screen
#           Recording again after each upgrade)
#   Linux   libobs from the distribution (sudo apt-get install obs-studio libobs-dev)
set -eu
cd "$(dirname "$0")/.."
DIST="${1:-$PWD/dist}"
have_libobs() { ls /usr/lib/*/libobs.so.0 /usr/lib64/libobs.so.0 /usr/lib/libobs.so.0 >/dev/null 2>&1 || { command -v ldconfig >/dev/null && ldconfig -p 2>/dev/null | grep -q 'libobs\.so\.0'; }; }
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' engine/cli/Cargo.toml | head -1)
OS=$(uname -s); ARCH=$(uname -m)
case "$OS-$ARCH" in
  Darwin-arm64)  TARGET="aarch64-apple-darwin" ;;
  Linux-x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
  *) echo "release: no target for $OS on $ARCH" >&2; exit 1 ;;
esac
RELEASE="remux-$VERSION-$TARGET"
STAGE="$DIST/$RELEASE"
say() { printf '%s\n' "$*"; }

if [ "$OS" = "Darwin" ]; then
  OBS_APP="${OBS_APP:-$PWD/engine/target/obs/OBS.app}"
  [ -d "$OBS_APP" ] || { echo "release: no OBS at $OBS_APP: make obs.fetch" >&2; exit 1; }
  export OBS_APP
  rm -f engine/target/release/obs-ffmpeg-mux engine/target/Frameworks
else
  have_libobs || { echo "release: no libobs: sudo apt-get install obs-studio libobs-dev" >&2; exit 1; }
  # The bindings link the distribution's libobs from its folder, not through
  # pkg-config, which would ask for the OBS their headers are from.
  if [ -z "${LIBOBS_PATH:-}" ]; then
    LIBOBS_PATH=$(dirname "$(ls /usr/lib/*/libobs.so /usr/lib64/libobs.so /usr/lib/libobs.so 2>/dev/null | head -1)")
  fi
  [ -e "$LIBOBS_PATH/libobs.so" ] || { echo "release: no libobs.so in $LIBOBS_PATH: sudo apt-get install libobs-dev" >&2; exit 1; }
  export LIBOBS_PATH
fi

(cd engine && cargo build --locked --release -p remuxd -p remux --no-default-features --features remuxd/obs)

if [ "$OS" = "Darwin" ]; then
  SIGN_ID="${SIGN_ID:-remux dev}"
  if security find-identity -v -p codesigning 2>/dev/null | grep -q "$SIGN_ID"; then sign="$SIGN_ID"; else sign="-"; say "release: no identity '$SIGN_ID' in the keychain: signing ad hoc"; fi
  codesign --force --sign "$sign" --identifier com.remux.engine engine/target/release/remuxd
  codesign --force --sign "$sign" --identifier com.remux.cli engine/target/release/remux
fi

rm -rf "$STAGE" && mkdir -p "$STAGE/bin" "$STAGE/docs" "$STAGE/byo"
cp engine/target/release/remux engine/target/release/remuxd "$STAGE/bin/"
cp uninstall.sh install-relay.sh install-chat.sh README.md LICENSE "$STAGE/"
cp docs/wire.md docs/relay.md docs/byo.md docs/login.md "$STAGE/docs/"
cp byo/mediamtx.yml byo/bridge.py "$STAGE/byo/"
if command -v shasum >/dev/null; then SUM="shasum -a 256"; else SUM="sha256sum"; fi
(cd "$DIST" && rm -f "$RELEASE.tar.gz" && tar -czf "$RELEASE.tar.gz" "$RELEASE" && rm -rf "$RELEASE" \
  && $SUM "$RELEASE.tar.gz" > "$RELEASE.tar.gz.sha256" && cat "$RELEASE.tar.gz.sha256")
