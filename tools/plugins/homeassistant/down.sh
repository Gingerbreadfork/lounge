#!/usr/bin/env bash
# Stops the test Home Assistant and removes its container. Its rooms and tokens are kept for the
# next up.sh unless --wipe is given, which removes what it kept under workdir/ha-testbed.
# INSTANCE names a second server started the same way.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(dirname "$(dirname "$(dirname "$HERE")")")
INSTANCE=${INSTANCE:-}
BED=$ROOT/workdir/ha-testbed${INSTANCE:+/beds/$INSTANCE}
NAME=lounge-ha${INSTANCE:+-$INSTANCE}

podman rm -f -t 20 "$NAME" >/dev/null 2>&1 || true
if [ "${1:-}" = "--wipe" ]; then
  # Home Assistant writes its files as the container's own user, so they are removed from inside one.
  if [ -n "$INSTANCE" ]; then
    podman unshare rm -rf "$BED"
  else
    podman unshare rm -rf "$BED/config" "$BED/state" "$BED/tokens" "$BED/scheme"
  fi
  # The network goes with the last server on it.
  podman network rm lounge-test >/dev/null 2>&1 || true
  echo "Removed $NAME and everything it kept"
else
  echo "Removed $NAME; its files are still in $BED"
fi
