#!/usr/bin/env bash
# Drives Lounge on a device with remote key events and captures screenshots after each step.
# Usage: tools/drive.sh <adb serial> <out dir> [install apk]
set -u
SERIAL=${1:?adb serial}; OUT=${2:?out dir}; APK=${3:-}
A="adb -s $SERIAL"; mkdir -p "$OUT"
PKG=${PKG:-tv.lounge.launcher.debug}; ACT=tv.lounge.launcher.MainActivity
snap() { sleep "${2:-1.2}"; $A exec-out screencap -p > "$OUT/$1.png"; echo "shot $1"; }
key() { for k in "$@"; do $A shell input keyevent "$k"; sleep 0.45; done; }

$A shell dumpsys power | grep -q 'mWakefulness=Awake' || $A shell input keyevent KEYCODE_WAKEUP
[ -n "$APK" ] && $A install -r -t "$APK" | tail -1
$A logcat -c
$A shell am force-stop $PKG; sleep 1
$A shell am start -W -n $PKG/$ACT | grep -E 'TotalTime'
snap 01-home 7
key KEYCODE_DPAD_RIGHT;                 snap 02-stage-right
key KEYCODE_DPAD_RIGHT KEYCODE_DPAD_RIGHT; snap 03-stage-right3
key KEYCODE_DPAD_DOWN;                  snap 04-rows-collapsed
key KEYCODE_DPAD_UP;                    snap 04b-band-raised
key KEYCODE_DPAD_LEFT KEYCODE_DPAD_LEFT; snap 04c-stage-left2
key KEYCODE_DPAD_DOWN;                  snap 04d-rows-again
key KEYCODE_DPAD_RIGHT KEYCODE_DPAD_RIGHT; snap 05-rows-right2
key KEYCODE_DPAD_DOWN;                  snap 06-rows-down
key KEYCODE_MENU;                       snap 07-tile-menu
key KEYCODE_BACK;                       snap 08-menu-closed 0.8
key KEYCODE_BACK;                       snap 09-back-to-stage
key KEYCODE_DPAD_UP;                    snap 10-playbill 0.8
key KEYCODE_DPAD_UP;                    snap 11-topbar 0.8
# Right saturates at Settings whether or not the Guide button is showing.
key KEYCODE_DPAD_RIGHT KEYCODE_DPAD_RIGHT KEYCODE_DPAD_RIGHT KEYCODE_DPAD_CENTER; snap 12-settings
key KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN; snap 13-settings-home-row 0.8
key KEYCODE_DPAD_CENTER;                snap 14-home-role 1.0
key KEYCODE_BACK KEYCODE_BACK;          snap 15-settings-closed 0.8
key KEYCODE_DPAD_LEFT KEYCODE_DPAD_CENTER; snap 16-search 1.5
$A shell input text "ne"; snap 17-search-typed 1.5
key KEYCODE_DPAD_DOWN;                  snap 18-search-result 1.0
key KEYCODE_BACK KEYCODE_BACK;          snap 19-search-closed 1.0
key KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN KEYCODE_DPAD_DOWN; snap 20-rows-deep 1.0
key KEYCODE_BACK;                       snap 21-back-home 1.0
echo "=== logcat issues"; $A logcat -d | grep -E 'FATAL|AndroidRuntime|E Lounge|W Lounge' | head -12
$A logcat -d | grep -A12 'FATAL EXCEPTION' | head -20
echo "=== memory"; $A shell dumpsys meminfo $PKG | grep -E 'TOTAL PSS' | head -1
echo "=== frame stats"; $A shell dumpsys gfxinfo $PKG | grep -E 'Total frames|Janky frames|90th|95th|99th' | head -5
