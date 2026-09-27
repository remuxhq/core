#!/bin/sh
# Install a stream relay of your own, on this machine: mediamtx takes the one
# stream the engine sends and one ffmpeg per platform sends it on.
#
#   curl -fsSL https://raw.githubusercontent.com/remuxhq/core/main/install-relay.sh | sh
#
# What it does, and says before doing: installs mediamtx and ffmpeg with
# your package manager, writes ~/.config/remux/byo.env (0600) for your stream
# keys if it is not there, and puts `remux-relay` in ~/.local/bin, which runs
# the relay in the foreground with those keys. Safe to run again. Needs
# remux installed first (install.sh).
#
#   REMUX_YES=1 or -y     do not ask
set -eu

HOME_DIR="${REMUX_HOME:-$HOME/.local/share/remux}"
BIN_DIR="${REMUX_BIN:-$HOME/.local/bin}"
ENV_FILE="$HOME/.config/remux/byo.env"
YES="${REMUX_YES:-}"
for arg in "$@"; do case "$arg" in -y|--yes) YES=1 ;; esac; done

say() { printf '%s\n' "$*"; }
die() { printf 'install-relay: %s\n' "$*" >&2; exit 1; }
ask() {
  [ -n "$YES" ] && return 0
  [ -r /dev/tty ] || die "no terminal to ask on; run with -y to accept"
  printf '%s [Y/n] ' "$1" > /dev/tty; read -r answer < /dev/tty || answer=""
  case "$answer" in n|N|no|NO) return 1 ;; *) return 0 ;; esac
}

[ -f "$HOME_DIR/current/byo/mediamtx.yml" ] || die "remux is not installed here (install.sh first)"
OS=$(uname -s)
missing=""
command -v mediamtx >/dev/null || missing="$missing mediamtx"
command -v ffmpeg >/dev/null || missing="$missing ffmpeg"

say "a relay of your own: mediamtx on rtmp://127.0.0.1:1935/scene, one ffmpeg per platform"
say ""
if [ -n "$missing" ]; then
  if [ "$OS" = "Darwin" ]; then
    say "  needs    $missing, will run: brew install$missing"
  else
    say "  needs    $missing, will run: sudo apt-get install ffmpeg; mediamtx from its GitHub release into $BIN_DIR"
  fi
else
  say "  needs     mediamtx and ffmpeg: found"
fi
say "  keys      $ENV_FILE (0600): TWITCH_KEY, YOUTUBE_KEY, the ones you fill in"
say "  runs as   remux-relay (foreground, your own pane; Ctrl-C stops it)"
say ""
ask "install?" || { say "nothing done"; exit 0; }

if [ -n "$missing" ]; then
  if [ "$OS" = "Darwin" ]; then
    command -v brew >/dev/null || die "no Homebrew: install$missing yourself, then run this again"
    # shellcheck disable=SC2086
    brew install $missing
  else
    case "$missing" in *ffmpeg*)
      command -v apt-get >/dev/null || die "install ffmpeg with your package manager, then run this again"
      sudo apt-get install -y ffmpeg ;;
    esac
    case "$missing" in *mediamtx*)
      arch=$(uname -m); case "$arch" in x86_64) arch=amd64 ;; aarch64) arch=arm64 ;; esac
      tag=$(curl -fsSL https://api.github.com/repos/bluenviron/mediamtx/releases/latest | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
      [ -n "$tag" ] || die "could not read mediamtx's latest version"
      tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
      curl -fsSL "https://github.com/bluenviron/mediamtx/releases/download/$tag/mediamtx_${tag}_linux_${arch}.tar.gz" | tar -xzf - -C "$tmp"
      mkdir -p "$BIN_DIR" && install -m 755 "$tmp/mediamtx" "$BIN_DIR/mediamtx"
      say "mediamtx $tag in $BIN_DIR" ;;
    esac
  fi
fi

mkdir -p "$HOME/.config/remux" && chmod 700 "$HOME/.config/remux"
if [ ! -f "$ENV_FILE" ]; then
  umask 077
  cat > "$ENV_FILE" <<'EOF'
# Your stream keys, read by remux-relay and remux-chat. Never share this file.
# Twitch: dashboard.twitch.tv -> Settings -> Stream -> Primary Stream key
TWITCH_KEY=
# YouTube: studio.youtube.com -> Go live -> Stream -> Stream key
YOUTUBE_KEY=
# YouTube Data API key (the chat): console.cloud.google.com -> APIs -> YouTube Data API v3 -> Credentials
YOUTUBE_API_KEY=
EOF
  say "wrote $ENV_FILE: put your keys in it (\${EDITOR:-vi} $ENV_FILE)"
fi

mkdir -p "$BIN_DIR"
cat > "$BIN_DIR/remux-relay" <<EOF
#!/bin/sh
# The relay of your own, in the foreground: mediamtx with the keys in byo.env.
set -eu
ENV_FILE="\${REMUX_BYO_ENV:-$ENV_FILE}"
[ -f "\$ENV_FILE" ] || { echo "remux-relay: no \$ENV_FILE (install-relay.sh writes it)" >&2; exit 1; }
set -a; . "\$ENV_FILE"; set +a
if [ -z "\${TWITCH_KEY:-}" ] && [ -z "\${YOUTUBE_KEY:-}" ]; then
  echo "remux-relay: no TWITCH_KEY and no YOUTUBE_KEY in \$ENV_FILE; nothing to send to" >&2; exit 1
fi
echo "remux-relay: rtmp://127.0.0.1:1935/scene ->\${TWITCH_KEY:+ twitch}\${YOUTUBE_KEY:+ youtube}  (remux destination add custom relay --url rtmp://127.0.0.1:1935 --key - <<< scene)"
exec mediamtx "$HOME_DIR/current/byo/mediamtx.yml"
EOF
chmod 755 "$BIN_DIR/remux-relay"
say "installed $BIN_DIR/remux-relay"
say ""
say "next:  \${EDITOR:-vi} $ENV_FILE                       your keys"
say "       remux-relay                                    in its own pane"
say "       echo scene | remux destination add custom relay --url rtmp://127.0.0.1:1935 --key -"
