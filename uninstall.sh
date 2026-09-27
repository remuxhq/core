#!/bin/sh
# Remove remux from this machine: the daemon, its service, every version,
# the link on the PATH, the engine's socket and logs. Safe to run again.
#
#   sh ~/.local/share/remux/current/uninstall.sh [--purge] [--force]
#
#   --purge  also ~/.config/remux (destinations, keys, tokens, config.toml)
#   --force  even on air (a live ends)
#
# Recordings (~/Movies/remux, ~/Videos/remux on Linux) and music (~/Music/remux) are yours; they stay.
set -u

HOME_DIR="${REMUX_HOME:-$HOME/.local/share/remux}"
BIN_DIR="${REMUX_BIN:-$HOME/.local/bin}"
LABEL="com.remux.remuxd"
PURGE=""; FORCE=""
for arg in "$@"; do
  case "$arg" in
    --purge) PURGE=1 ;;
    --force) FORCE="--force" ;;
    *) printf 'uninstall: unknown %s\n' "$arg" >&2; exit 2 ;;
  esac
done

say() { printf '%s\n' "$*"; }

REMUX="$HOME_DIR/current/bin/remux"
if [ -x "$REMUX" ]; then
  "$REMUX" daemon stop $FORCE || { say "uninstall: the engine is on air; remux stop first, or --force"; exit 1; }
elif [ "$(uname -s)" = "Darwin" ]; then
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
else
  systemctl --user disable --now remuxd 2>/dev/null || true
fi

if [ "$(uname -s)" = "Darwin" ]; then
  rm -f "$HOME/Library/LaunchAgents/$LABEL.plist"
  rm -rf "$HOME/Library/Application Support/remux"
else
  rm -f "$HOME/.config/systemd/user/remuxd.service"
  systemctl --user daemon-reload 2>/dev/null || true
  rm -rf "$HOME/.local/state/remux"
fi
case "$(readlink "$BIN_DIR/remux" 2>/dev/null)" in
  "$HOME_DIR"/*) rm -f "$BIN_DIR/remux" ;;
esac
rm -rf "$HOME_DIR"
# The PATH block install.sh appended, out of every rc file it could have used.
for rc in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.profile" "$HOME/.config/fish/config.fish"; do
  if [ -f "$rc" ] && grep -q "# >>> remux >>>" "$rc"; then
    # Through a temp file: an rc file is often a link into a dotfiles repo.
    sed '/# >>> remux >>>/,/# <<< remux <<</d' "$rc" > "$rc.remux-tmp" && cat "$rc.remux-tmp" > "$rc" && rm -f "$rc.remux-tmp"
  fi
done
if [ -n "$PURGE" ]; then
  rm -rf "$HOME/.config/remux"
  say "removed remux and ~/.config/remux"
  # macOS keeps the engine's grants by its path, and tccutil cannot reset a
  # path (only a bundle id): to start over as a machine that never saw remux,
  # the rows are removed by hand.
  if [ "$(uname -s)" = "Darwin" ]; then
    say "to forget its permissions too: System Settings, Privacy & Security, Screen Recording and Microphone, remove remuxd (-)"
  fi
else
  say "removed remux; ~/.config/remux kept (--purge removes it)"
fi
