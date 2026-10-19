#!/usr/bin/env bash
# Key-only traversal with per-phase frame timing from `dumpsys gfxinfo <pkg> framestats`.
# Usage: tools/framestats.sh <adb serial> [package] [skip-keys]
set -u
SERIAL=${1:?adb serial}; PKG=${2:-tv.lounge.launcher}; SKIP=${3:-}
A="adb -s $SERIAL"
OUT=${FRAMESTATS_OUT:-/tmp/framestats.txt}
$A shell dumpsys gfxinfo "$PKG" reset >/dev/null
if [ -z "$SKIP" ]; then
  for k in RIGHT RIGHT RIGHT RIGHT LEFT LEFT DOWN RIGHT RIGHT RIGHT RIGHT DOWN RIGHT RIGHT UP UP LEFT LEFT; do
    $A shell input keyevent KEYCODE_DPAD_$k; sleep 0.4
  done
  $A shell input keyevent KEYCODE_BACK; sleep 1.2
fi
$A shell dumpsys gfxinfo "$PKG" framestats > "$OUT"
grep -E 'Total frames|Janky frames:|50th|90th|95th|99th' "$OUT" | head -6
# Column names come from the header line, so this works across HWUI versions.
gawk -F, '
/^Flags,/ { for (i = 1; i <= NF; i++) col[$i] = i; next }
/^[0-9]+,/ && (length(col) > 0) && ($col["Flags"] == 0) {
  n++
  iv = $col["IntendedVsync"]
  late[n]  = ($col["HandleInputStart"] - iv) / 1e6
  ui[n]    = ($col["SyncQueued"] - $col["HandleInputStart"]) / 1e6
  anim[n]  = ($col["PerformTraversalsStart"] - $col["AnimationStart"]) / 1e6
  lay[n]   = ($col["DrawStart"] - $col["PerformTraversalsStart"]) / 1e6
  rec[n]   = ($col["SyncQueued"] - $col["DrawStart"]) / 1e6
  issue[n] = ($col["SwapBuffers"] - $col["IssueDrawCommandsStart"]) / 1e6
  gpu[n]   = ($col["FrameCompleted"] - $col["SwapBuffers"]) / 1e6
  tot[n]   = ($col["FrameCompleted"] - iv) / 1e6
  dl       = ($col["FrameDeadline"] - iv) / 1e6
  if (tot[n] > dl) missed++
}
function stats(name, arr, n,   i, s, sorted) {
  s = 0; for (i = 1; i <= n; i++) { s += arr[i]; sorted[i] = arr[i] }
  asort(sorted)
  printf "  %-22s avg %6.1f  p50 %6.1f  p90 %6.1f  max %6.1f ms\n", name, s / n, sorted[int(n * 0.5) + 1], sorted[int(n * 0.9)], sorted[n]
}
END {
  if (n == 0) { print "no framestats rows"; exit }
  printf "%d frames, %d past the deadline\n", n, missed
  stats("vsync->main thread", late, n)
  stats("main: animation", anim, n)
  stats("main: measure/layout", lay, n)
  stats("main: draw record", rec, n)
  stats("main thread total", ui, n)
  stats("render: issue GL", issue, n)
  stats("gpu: swap->complete", gpu, n)
  stats("vsync->complete", tot, n)
}' "$OUT"
