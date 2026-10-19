#!/usr/bin/env bash
# Builds the companion web app into companion/dist, which lounge-companion embeds at compile
# time. Gradle runs this automatically (buildCompanionSpa) when node_modules is present.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(dirname "$(dirname "$HERE")")
cd "$ROOT/companion"
[ -d node_modules ] || pnpm install
pnpm build
