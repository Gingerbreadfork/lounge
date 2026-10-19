#!/usr/bin/env bash
# Sends keys one at a time and reports which node holds focus after each, via uiautomator.
# Usage: tools/focusprobe.sh <adb serial> KEY [KEY...]   (KEY like DOWN, UP, BACK, CENTER, MENU)
set -u
SERIAL=${1:?adb serial}; shift
A="adb -s $SERIAL"
focused() {
  $A exec-out uiautomator dump /dev/tty 2>/dev/null \
    | grep -o '<node[^>]*focused="true"[^>]*>' \
    | sed -E 's/.* text="([^"]*)".* class="([^"]*)".* content-desc="([^"]*)".* bounds="([^"]*)".*/    text="\1" desc="\3" \4/' \
    | head -3
  [ "${PIPESTATUS[1]}" = 0 ] || echo "    (nothing focused)"
}
echo "start:"; focused
for k in "$@"; do
  case $k in
    BACK|MENU|HOME) code=KEYCODE_$k ;;
    CENTER) code=KEYCODE_DPAD_CENTER ;;
    *) code=KEYCODE_DPAD_$k ;;
  esac
  $A shell input keyevent $code; sleep "${PROBE_DELAY:-1.4}"
  echo "$k:"; focused
done
