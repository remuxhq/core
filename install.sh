#!/bin/sh
# Install remux (the CLI and the engine) from a release, on macOS or Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install.sh | sh
#
# What it does, and says before doing: downloads the release for this
# machine, checks its sha256, unpacks it under ~/.local/share/remux, links
# `remux` into ~/.local/bin, puts that folder on your PATH (a marked block in
# your shell's rc file), and starts the engine as a service (launchd, or
# systemd --user). Nothing needs sudo, except a missing dependency it offers
# to install with your package manager. Safe to run again: the same version
# is reinstalled in place, a newer one takes over.
#
#   REMUX_VERSION           a version instead of the latest (0.2.1)
#   REMUX_RELEASE_URL       where the tarballs are (file:///…/dist for a local `make release`)
#   REMUX_HOME              where versions live (~/.local/share/remux)
#   REMUX_YES=1  or -y      do not ask
#   REMUX_NO_MODIFY_PATH=1  leave the rc file alone
#   OBS_APP                 macOS: the OBS the engine links (/Applications/OBS.app)
set -eu

REPO="remuxhq/core"
HOME_DIR="${REMUX_HOME:-$HOME/.local/share/remux}"
BIN_DIR="${REMUX_BIN:-$HOME/.local/bin}"
YES="${REMUX_YES:-}"
for arg in "$@"; do case "$arg" in -y|--yes) YES=1 ;; esac; done

say() { printf '%s\n' "$*"; }
die() { printf 'install: %s\n' "$*" >&2; exit 1; }
ask() {
  # `curl | sh` has no stdin of its own: the question goes to the terminal.
  [ -n "$YES" ] && return 0
  ( : < /dev/tty ) 2>/dev/null || die "no terminal to ask on; run with -y (or REMUX_YES=1) to accept"
  printf '%s [Y/n] ' "$1" > /dev/tty; read -r answer < /dev/tty || answer=""
  case "$answer" in n|N|no|NO) return 1 ;; *) return 0 ;; esac
}

# libobs wherever this distribution keeps it: multiarch (Debian, Ubuntu), lib64 (Fedora), lib (Arch).
have_libobs() { ls /usr/lib/*/libobs.so.0 /usr/lib64/libobs.so.0 /usr/lib/libobs.so.0 >/dev/null 2>&1 || { command -v ldconfig >/dev/null && ldconfig -p 2>/dev/null | grep -q 'libobs\.so\.0'; }; }
OS=$(uname -s); ARCH=$(uname -m)
case "$OS-$ARCH" in
  Darwin-arm64) TARGET="aarch64-apple-darwin" ;;
  Linux-x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
  Darwin-x86_64) die "no build for Intel Macs yet (Apple silicon, Linux x86_64 and aarch64 today)" ;;
  *) die "no build for $OS on $ARCH yet (macOS Apple silicon, Linux x86_64 and aarch64 today)" ;;
esac
command -v curl >/dev/null || die "curl is needed"

VERSION="${REMUX_VERSION:-}"
if [ -z "$VERSION" ]; then
  VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -1)
  [ -n "$VERSION" ] || die "could not read the latest version from GitHub; set REMUX_VERSION"
fi
URL="${REMUX_RELEASE_URL:-https://github.com/$REPO/releases/download/v$VERSION}"
NAME="remux-$VERSION-$TARGET"

# ---- the plan, said before anything changes --------------------------------
say "remux $VERSION for $TARGET"
say ""
say "  into      $HOME_DIR/$VERSION, and $BIN_DIR/remux"
say "  PATH      $BIN_DIR, a marked block in your shell's rc file (REMUX_NO_MODIFY_PATH=1 leaves it)"
OBS_VERSION=""
if [ "$OS" = "Darwin" ]; then
  OBS="${OBS_APP:-/Applications/OBS.app}"
  OBS_VERSION=$(defaults read "$OBS/Contents/Info.plist" CFBundleShortVersionString 2>/dev/null || true)
  case "$OBS_VERSION" in
    32.*) say "  needs     OBS 32 (the engine is libobs): found $OBS_VERSION at $OBS" ;;
    "")   say "  needs     OBS 32 (the engine is libobs): not at $OBS, will run: brew install --cask obs" ;;
    *)    say "  needs     OBS 32 (the engine is libobs): $OBS is $OBS_VERSION, will run: brew upgrade --cask obs" ;;
  esac
  say "  service   a launchd agent, com.remux.remuxd (remux daemon start|stop|status|log)"
else
  if have_libobs; then
    say "  needs     OBS 30+ (the engine is libobs): found $(dpkg-query -W -f='${Version}' obs-studio 2>/dev/null || echo libobs)"
  else
    say "  needs     OBS 30+ (the engine is libobs): not found, will run: sudo apt-get install obs-studio"
  fi
  say "  service   a systemd --user unit, remuxd (remux daemon start|stop|status|log)"
fi
say ""
ask "install?" || { say "nothing done"; exit 0; }

# ---- the dependency --------------------------------------------------------
if [ "$OS" = "Darwin" ]; then
  case "$OBS_VERSION" in
    32.*) ;;
    "") command -v brew >/dev/null || die "no Homebrew: install OBS 32 from obsproject.com, then run this again"
        brew install --cask obs ;;
    *)  command -v brew >/dev/null || die "install OBS 32 from obsproject.com, then run this again"
        brew upgrade --cask obs ;;
  esac
  OBS_VERSION=$(defaults read "$OBS/Contents/Info.plist" CFBundleShortVersionString 2>/dev/null || true)
  case "$OBS_VERSION" in 32.*) ;; *) die "OBS 32 is still not at $OBS (OBS_APP=… names another)" ;; esac
elif ! have_libobs; then
  command -v apt-get >/dev/null || die "install OBS Studio with your package manager (dnf install obs-studio, pacman -S obs-studio), then run this again"
  sudo apt-get install -y obs-studio
  have_libobs || die "OBS Studio is installed but no libobs.so.0 was found; remux bug"
fi

# ---- the release, checked --------------------------------------------------
TMP=$(mktemp -d "${TMPDIR:-/tmp}/remux-install.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
curl -fsSL -o "$TMP/$NAME.tar.gz" "$URL/$NAME.tar.gz" || die "no $NAME.tar.gz at $URL"
curl -fsSL -o "$TMP/$NAME.tar.gz.sha256" "$URL/$NAME.tar.gz.sha256" || die "no $NAME.tar.gz.sha256 at $URL"
if command -v shasum >/dev/null; then SUM="shasum -a 256"; else SUM="sha256sum"; fi
(cd "$TMP" && $SUM -c "$NAME.tar.gz.sha256" >/dev/null) || die "$NAME.tar.gz does not match its sha256"
say "sha256 verified"
tar -xzf "$TMP/$NAME.tar.gz" -C "$TMP"
[ -x "$TMP/$NAME/bin/remux" ] && [ -x "$TMP/$NAME/bin/remuxd" ] || die "the tarball has no bin/remux and bin/remuxd"

# ---- in place ---------------------------------------------------------------
mkdir -p "$HOME_DIR" "$BIN_DIR"
rm -rf "$HOME_DIR/$VERSION.tmp"
mv "$TMP/$NAME" "$HOME_DIR/$VERSION.tmp"
if [ "$OS" = "Darwin" ]; then
  # The engine links libobs through `Frameworks` beside `bin/`.
  ln -sfhn "$OBS/Contents/Frameworks" "$HOME_DIR/$VERSION.tmp/Frameworks"
  xattr -dr com.apple.quarantine "$HOME_DIR/$VERSION.tmp" 2>/dev/null || true
fi
rm -rf "$HOME_DIR/$VERSION"
mv "$HOME_DIR/$VERSION.tmp" "$HOME_DIR/$VERSION"
rm -f "$HOME_DIR/current" && ln -s "$VERSION" "$HOME_DIR/current"
ln -sfn "$HOME_DIR/current/bin/remux" "$BIN_DIR/remux"
mkdir -p "$HOME/.config/remux" && chmod 700 "$HOME/.config/remux"
say "installed $HOME_DIR/$VERSION"

# ---- the PATH, once ---------------------------------------------------------
if [ -z "${REMUX_NO_MODIFY_PATH:-}" ]; then
  case "$(basename "${SHELL:-sh}")" in
    zsh)  rc="$HOME/.zshrc"; line="export PATH=\"$BIN_DIR:\$PATH\"" ;;
    bash) rc="$HOME/.bashrc"; [ "$OS" = "Darwin" ] && rc="$HOME/.bash_profile"; line="export PATH=\"$BIN_DIR:\$PATH\"" ;;
    fish) rc="$HOME/.config/fish/config.fish"; line="fish_add_path $BIN_DIR" ;;
    *)    rc="$HOME/.profile"; line="export PATH=\"$BIN_DIR:\$PATH\"" ;;
  esac
  if ! { [ -f "$rc" ] && grep -q "# >>> remux >>>" "$rc"; }; then
    mkdir -p "$(dirname "$rc")"
    printf '\n# >>> remux >>>\n%s\n# <<< remux <<<\n' "$line" >> "$rc"
    say "PATH: $BIN_DIR added to $rc (a new shell picks it up; this one: $line)"
  fi
fi

# ---- the engine -------------------------------------------------------------
REMUX="$HOME_DIR/current/bin/remux"
status=$("$REMUX" daemon status 2>/dev/null || true)
case "$status" in
  *"ON AIR"*) say "the engine is on air: the new version takes over at the next remux daemon restart" ;;
  *running*)  "$REMUX" daemon restart ;;
  *)          "$REMUX" daemon start ;;
esac
say ""
say "next:  remux health        what stands in the way of a live, one line each"
say "       remux guide         the verbs, from a script or by hand"
say "       a relay and a chat of your own: install-relay.sh, install-chat.sh (docs/byo.md)"
