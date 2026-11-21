#!/usr/bin/env bash
# Runs the Home Assistant plugin against the test server from up.sh, as the TV would. lgp-tool
# refuses this machine's own addresses, as the TV refuses its own, so it runs in a container on
# the server's network and reaches it by name.
#
#   run.sh                                     a refresh
#   run.sh --resolve toggle:light.bed_light    a press
#   run.sh --sheet room:living_room            a sheet
#   run.sh --search lamp
#   run.sh --artwork                           a refresh that also downloads the cameras' pictures
#
# AS=user uses the plain user's token, AS=reader that of one who may only read, AS=wrong one Home
# Assistant refuses, and AS=none no token.
# STATE names the file under workdir/ha-testbed/state that holds the plugin's store.
# LABEL and SCRIPTS fill those settings, SERVER names another address, and INSTANCE a second
# server started with up.sh.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(dirname "$(dirname "$(dirname "$HERE")")")
INSTANCE=${INSTANCE:-}
BED=$ROOT/workdir/ha-testbed${INSTANCE:+/beds/$INSTANCE}
NAME=lounge-ha${INSTANCE:+-$INSTANCE}
TARGET=$ROOT/plugins/target
WASM=wasm32-unknown-unknown/release/lounge_plugin_homeassistant.wasm
[ -f "$TARGET/release/lgp-tool" ] || { echo "Build lgp-tool first: cd plugins && cargo build --release -p lgp-tool" >&2; exit 1; }
[ -f "$TARGET/$WASM" ] || { echo "Build the plugin first: cd plugins && cargo build --release --target wasm32-unknown-unknown -p lounge-plugin-homeassistant" >&2; exit 1; }

settings=(--set "server=${SERVER:-$(cat "$BED/scheme" 2>/dev/null || echo http)://$NAME:8123}")
case ${AS:-admin} in
  wrong) settings+=(--secret "token=not.a.token") ;;
  none) ;;
  *)
    [ -s "$BED/tokens/${AS:-admin}" ] || { echo "No token in $BED/tokens/${AS:-admin}; run up.sh first" >&2; exit 1; }
    settings+=(--secret "token=$(cat "$BED/tokens/${AS:-admin}")") ;;
esac
[ -z "${LABEL:-}" ] || settings+=(--set "label=$LABEL")
[ -z "${SCRIPTS:-}" ] || settings+=(--set "scripts=$SCRIPTS")

mkdir -p "$BED/state"
# The tool is built against this machine's C library, which Fedora's small image shares. Labels
# are left alone so the build folder is not relabelled for one container.
exec podman run --rm --network lounge-test --security-opt label=disable \
  -v "$TARGET:/t:ro" -v "$BED/state:/s" \
  registry.fedoraproject.org/fedora-minimal:44 \
  /t/release/lgp-tool run "/t/$WASM" "${settings[@]}" --state "/s/${STATE:-kv}.json" "$@"
