#!/bin/sh
# Install a chat bridge of your own: Twitch and YouTube chat, read here and
# served to the engine on its wire (docs/wire.md).
#
#   curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install-chat.sh | sh
#
# What it does, and says before doing: checks for python3 (3.9 or newer, the
# standard library only), puts `remux-chat` in ~/.local/bin, which runs the
# bridge in the foreground, and points the engine at it (remux chat url).
# The channel and the video go in ~/.config/remux/config.toml under [byo], or
# on remux-chat's command line. Safe to run again. Needs remux installed
# first (install.sh); the YouTube Data API key goes in byo.env
# (install-relay.sh writes it, or write it yourself).
#
#   REMUX_YES=1 or -y     do not ask
set -eu

HOME_DIR="${REMUX_HOME:-$HOME/.local/share/remux}"
BIN_DIR="${REMUX_BIN:-$HOME/.local/bin}"
ENV_FILE="$HOME/.config/remux/byo.env"
YES="${REMUX_YES:-}"
for arg in "$@"; do case "$arg" in -y|--yes) YES=1 ;; esac; done

say() { printf '%s\n' "$*"; }
die() { printf 'install-chat: %s\n' "$*" >&2; exit 1; }
ask() {
  [ -n "$YES" ] && return 0
  [ -r /dev/tty ] || die "no terminal to ask on; run with -y to accept"
  printf '%s [Y/n] ' "$1" > /dev/tty; read -r answer < /dev/tty || answer=""
  case "$answer" in n|N|no|NO) return 1 ;; *) return 0 ;; esac
}

[ -f "$HOME_DIR/current/byo/bridge.py" ] || die "remux is not installed here (install.sh first)"
PY=$(command -v python3 || true)
say "a chat bridge of your own: Twitch (IRC, anonymous) and YouTube (Data API) on ws://127.0.0.1:9999"
say ""
if [ -n "$PY" ]; then
  say "  needs     python3: found $($PY --version 2>&1)"
else
  if [ "$(uname -s)" = "Darwin" ]; then say "  needs     python3: not found, will run: xcode-select --install (Apple's, no Homebrew needed)"
  else say "  needs     python3: not found, will run: sudo apt-get install python3 (or your package manager)"; fi
fi
say "  reads     [byo] twitch and youtube in ~/.config/remux/config.toml, YOUTUBE_API_KEY in $ENV_FILE"
say "  runs as   remux-chat (foreground, your own pane; Ctrl-C stops it)"
say ""
ask "install?" || { say "nothing done"; exit 0; }

if [ -z "$PY" ]; then
  if [ "$(uname -s)" = "Darwin" ]; then xcode-select --install; die "run this again once the command line tools are in"
  else command -v apt-get >/dev/null || die "install python3 with your package manager, then run this again"; sudo apt-get install -y python3; fi
fi
$PY -c 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)' || die "python3 is older than 3.9"

mkdir -p "$BIN_DIR"
cat > "$BIN_DIR/remux-chat" <<EOF
#!/bin/sh
# The chat bridge of your own, in the foreground; the engine is pointed at it.
set -eu
ENV_FILE="\${REMUX_BYO_ENV:-$ENV_FILE}"
if [ -f "\$ENV_FILE" ]; then set -a; . "\$ENV_FILE"; set +a; fi
"$HOME_DIR/current/bin/remux" chat --url ws://127.0.0.1:9999 >/dev/null 2>&1 || true
exec python3 "$HOME_DIR/current/byo/bridge.py" "\$@"
EOF
chmod 755 "$BIN_DIR/remux-chat"
say "installed $BIN_DIR/remux-chat"
say ""
say "next:  remux-chat --twitch <channel> --youtube <video id>     in its own pane"
say "       (or [byo] twitch = \"…\", youtube = \"…\" in ~/.config/remux/config.toml, then just remux-chat)"
say "       remux chat read -f                                     the lines, as they come"
