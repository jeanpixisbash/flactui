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
# A prebuilt binary is only trusted if it is a real ELF executable for this
# machine — a stale, corrupt or wrong-arch file must not get installed.
looks_like_elf() {
  [ -f "$1" ] || return 1
  [ "$(head -c 4 "$1" 2>/dev/null | od -An -tx1 | tr -d ' \n')" = "7f454c46" ] || return 1
  # arch check when `file` is available (x86-64 / aarch64 naming differs)
  if command -v file >/dev/null 2>&1 && command -v uname >/dev/null 2>&1; then
    case "$(uname -m)" in
      x86_64)  file -b "$1" | grep -q "x86-64" ;;
      aarch64) file -b "$1" | grep -q "aarch64" ;;
      *) true ;; # unknown host arch: ELF magic is all we can sanity-check
    esac
  else
    true
  fi
}

PREBUILT="$(dirname "$0")/flactui"
BUILT="$(dirname "$0")/target/release/flactui"
if [ -x "$PREBUILT" ] && ! looks_like_elf "$PREBUILT"; then
  echo "⚠  '$PREBUILT' is not a valid ELF binary for this machine — ignoring it."
  PREBUILT="/nonexistent"
fi
if [ ! -x "$PREBUILT" ] && [ ! -x "$BUILT" ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "No valid prebuilt binary and no cargo. Install Rust (https://rustup.rs) or download"
    echo "a release binary from GitHub Releases and put it next to this installer."
    exit 1
  fi
  echo "Building flactui (release)..."
  (cd "$(dirname "$0")" && cargo build --release)
  BIN="$BUILT"
else
  BIN="$( [ -x "$PREBUILT" ] && echo "$PREBUILT" || echo "$BUILT" )"
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
