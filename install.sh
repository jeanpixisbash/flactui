#!/usr/bin/env bash
# flactui installer — user-level, no sudo.
set -euo pipefail

DEST="${HOME}/.local/bin"

echo "flactui installer"
echo

echo "Checking dependencies:"
MISS=0
for dep in mpv; do
  if command -v "$dep" >/dev/null 2>&1; then
    echo "  ✓ $dep"
  else
    echo "  ✗ missing: $dep"
    MISS=1
  fi
done
# cargo only needed when there is no prebuilt binary next to the installer
if command -v cargo >/dev/null 2>&1; then
  echo "  ✓ cargo"
else
  echo "  • cargo not found (only needed to build; get a release binary otherwise)"
fi
# KDE Connect daemon check (kdeconnectd) — informational
if pgrep -x kdeconnectd >/dev/null 2>&1; then
  echo "  ✓ KDE Connect daemon running"
else
  echo "  • KDE Connect daemon not running (start KDE Connect and pair your phone)"
fi

echo
if [ ! -x "$(dirname "$0")/flactui" ] && [ ! -x "target/release/flactui" ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "No prebuilt binary and no cargo. Install Rust (https://rustup.rs) or download"
    echo "a release binary from GitHub Releases and put it next to this installer."
    exit 1
  fi
  echo "Building flactui (release)..."
  cargo build --release
  BIN="target/release/flactui"
else
  BIN="$( [ -x "$(dirname "$0")/flactui" ] && echo "$(dirname "$0")/flactui" || echo "target/release/flactui" )"
fi

mkdir -p "$DEST"
install -m 0755 "$BIN" "$DEST/flactui"
echo
echo "Installed: $DEST/flactui"

case ":$PATH:" in
  *":$DEST:"*) ;;
  *) echo
     echo "NOTE: $DEST is not in your PATH. Add it:"
     echo "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc && source ~/.bashrc"
     ;;
esac

echo
echo "Next steps:"
echo "  1. Pair your phone with KDE Connect (app on the phone + this computer)."
echo "  2. Run:  flactui"
echo
echo "Enjoy!"
