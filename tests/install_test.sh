#!/usr/bin/env bash
# install.sh contract tests — run in a sandboxed HOME, no cargo needed (faked).
set -u
cd "$(dirname "$0")/.." || exit 1
PASS=0; FAIL=0
check() { # name expected(condition: 0 ok) command...
  local name="$1"; shift
  if "$@" >/dev/null 2>&1; then PASS=$((PASS+1)); echo "  ok   $name"
  else FAIL=$((FAIL+1)); echo "  FAIL $name"; fi
}
fresh_env() { # sets up sandbox; echoes sandbox dir
  local d; d=$(mktemp -d /tmp/flactui-install-test.XXXXXX)
  mkdir -p "$d/home/.local" "$d/bin"
  cp install.sh "$d/install.sh"
  echo "$d"
}

echo "== install.sh =="

# 1. valid ELF prebuilt next to the installer wins (no cargo involved)
d=$(fresh_env)
cp /bin/true "$d/flactui"
HOME="$d/home" PATH="/usr/bin:/bin" bash "$d/install.sh" >/dev/null 2>&1
check "valid prebuilt installed as-is" cmp -s /bin/true "$d/home/.local/bin/flactui"
rm -rf "$d"

# 2. bogus (non-ELF) prebuilt is REJECTED; falls back to cargo build
d=$(fresh_env)
printf '#!/bin/sh\necho bogus\n' > "$d/flactui"; chmod +x "$d/flactui"
mkdir -p "$d/bin"
printf '#!/bin/sh\nmkdir -p target/release\ncp /bin/true target/release/flactui\n' > "$d/bin/cargo"
chmod +x "$d/bin/cargo"
HOME="$d/home" PATH="$d/bin:/usr/bin:/bin" bash "$d/install.sh" >/dev/null 2>&1
check "bogus prebuilt rejected, cargo build used" cmp -s /bin/true "$d/home/.local/bin/flactui"
rm -rf "$d"

# 3. bogus prebuilt + no cargo = clear error, nothing installed
d=$(fresh_env)
printf '#!/bin/sh\necho bogus\n' > "$d/flactui"; chmod +x "$d/flactui"
HOME="$d/home" PATH="/usr/bin:/bin" bash "$d/install.sh" >/dev/null 2>&1
check "bogus prebuilt + no cargo errors out" test ! -e "$d/home/.local/bin/flactui"
rm -rf "$d"

echo
echo "passed: $PASS  failed: $FAIL"
[ "$FAIL" -eq 0 ]
