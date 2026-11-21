#!/usr/bin/env bash
# Changes things in the test Home Assistant, to see the plugin's tiles follow.
#
#   flip.sh door open|closed        the front door
#   flip.sh window open|closed      the kitchen window
#   flip.sh leak wet|dry            the kitchen leak sensor
#   flip.sh smoke on|off            the hall smoke alarm
#   flip.sh battery 9               the hall sensor's battery, in percent
#   flip.sh lamp on|off             power to the lamp that does not answer
#   flip.sh person ana home|away    where someone is
#   flip.sh state light.bed_light   print one thing's state
#
# INSTANCE and PORT name a second server started with up.sh.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(dirname "$(dirname "$(dirname "$HERE")")")
INSTANCE=${INSTANCE:-}
BED=$ROOT/workdir/ha-testbed${INSTANCE:+/beds/$INSTANCE}
URL=$(cat "$BED/scheme" 2>/dev/null || echo http)://localhost:${PORT:-8123}
auth="Authorization: Bearer $(cat "$BED/tokens/admin")"

call() { curl -kfsS -o /dev/null -H "$auth" -H 'Content-Type: application/json' -d "$2" "$URL/api/services/$1"; }
helper() { call "input_boolean/turn_$2" "{\"entity_id\": \"input_boolean.$1\"}"; }
on_off() { case $1 in open|wet|on) echo on ;; closed|dry|off) echo off ;; *) echo "not a state: $1" >&2; exit 1 ;; esac; }

case ${1:-} in
  door) helper front_door_open "$(on_off "$2")" ;;
  window) helper kitchen_window_open "$(on_off "$2")" ;;
  leak) helper kitchen_leak "$(on_off "$2")" ;;
  smoke) helper hall_smoke "$(on_off "$2")" ;;
  lamp) helper broken_lamp_power "$(on_off "$2")" ;;
  battery) call input_number/set_value "{\"entity_id\": \"input_number.hall_sensor_battery\", \"value\": $2}" ;;
  person)
    case $3 in home) state=home ;; away) state=not_home ;; *) state=$3 ;; esac
    name=$(curl -kfsS -H "$auth" "$URL/api/states/person.$2" | jq -r .attributes.friendly_name)
    curl -kfsS -o /dev/null -H "$auth" -H 'Content-Type: application/json' -d "{\"state\": \"$state\", \"attributes\": {\"friendly_name\": \"$name\"}}" "$URL/api/states/person.$2" ;;
  state) curl -kfsS -H "$auth" "$URL/api/states/$2" | jq -c '{state, attributes: (.attributes | del(.access_token, .entity_picture))}' ;;
  *) sed -n '2,13p' "$0" | cut -c3-; exit 1 ;;
esac
