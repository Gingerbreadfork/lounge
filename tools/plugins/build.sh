#!/usr/bin/env bash
# Builds the first-party plugins for wasm32, bundles and signs them, and drops the .lgp files into
# the app's assets. Needs the wasm32-unknown-unknown target. The signing key lives at
# ~/.config/lounge/publisher.key and is generated on first use.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(dirname "$(dirname "$HERE")")
KEY=${LOUNGE_PUBLISHER_KEY:-$HOME/.config/lounge/publisher.key}
OUT=$ROOT/android/app/src/main/assets/plugins
export PATH="$HOME/.cargo/bin:$PATH"

cd "$ROOT/plugins"
cargo build -q --release -p lgp-tool
TOOL=target/release/lgp-tool
if [ ! -f "$KEY" ]; then
  mkdir -p "$(dirname "$KEY")"
  "$TOOL" keygen "$KEY"
fi
SHIPPED="iptv jellyfin emby plex youtube archive immich hdhomerun tvheadend channels nextpvr art trakt kodi twitch"
# shellcheck disable=SC2086
cargo build -q --release --target wasm32-unknown-unknown $(for n in $SHIPPED sample; do printf -- '-p lounge-plugin-%s ' "$n"; done)
mkdir -p "$OUT"
for name in $SHIPPED; do
  "$TOOL" bundle "target/wasm32-unknown-unknown/release/lounge_plugin_$name.wasm" "$KEY" "$OUT/$name.lgp"
done
# The sample library is not shipped in the APK; install it from the phone when you want it.
"$TOOL" bundle target/wasm32-unknown-unknown/release/lounge_plugin_sample.wasm "$KEY" target/sample.lgp
ls -la "$OUT" target/sample.lgp
