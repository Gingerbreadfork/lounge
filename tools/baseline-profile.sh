#!/usr/bin/env bash
# Captures a baseline profile from the non-minified `profile` build on a device (API 33+).
# Builds the APK, exercises the launcher with tools/drive.sh, dumps ART's profile for the
# process and writes it to app/src/main/baseline-prof.txt for the release build to compile in.
# Usage: tools/baseline-profile.sh <adb serial>
set -eu
SERIAL=${1:?adb serial}
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(dirname "$HERE")
A="adb -s $SERIAL"; PKG=tv.lounge.launcher.profile
OUT=${BASELINE_OUT:-/tmp/lounge-baseline}; mkdir -p "$OUT"

(cd "$ROOT/android" && ./gradlew :app:assembleProfile --console=plain -q)
$A install -r -t "$ROOT/android/app/build/outputs/apk/profile/app-profile.apk" | tail -1
# Start from an empty profile so only this session's behaviour is recorded.
$A shell cmd package clear-profiles "$PKG" >/dev/null 2>&1 || true

# Two full passes: the second one covers the warm paths the first one JIT-compiled. ART saves
# the profile a few seconds after start and then periodically, so the last pass is left running.
for pass in 1 2; do
  PKG=$PKG "$HERE/drive.sh" "$SERIAL" "$OUT/drive-$pass" >/dev/null 2>&1
done
sleep 45

$A shell cmd package dump-profiles --dump-classes-and-methods "$PKG" >/dev/null
$A shell cat "/data/misc/profman/$PKG-primary.prof.txt" > "$OUT/raw.txt"
# Keep rules for code that ships in the APK; drop anything the runtime attributed elsewhere.
grep -E '^(HS?P?L|S?P?L|L)(tv/lounge|androidx|kotlin|kotlinx|coil3|com/sun/jna|dagger|javax/inject|okio)' "$OUT/raw.txt" \
  | sort -u > "$ROOT/android/app/src/main/baseline-prof.txt"
echo "rules: $(wc -l < "$ROOT/android/app/src/main/baseline-prof.txt") (raw $(wc -l < "$OUT/raw.txt"))"
grep -E '^HSPLtv/lounge' "$ROOT/android/app/src/main/baseline-prof.txt" | head -3
