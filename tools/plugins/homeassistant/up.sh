#!/usr/bin/env bash
# Runs a Home Assistant with demo devices for testing the Home Assistant plugin, in podman on a
# network of its own, and sets it up: an administrator, a plain user and one who may only read,
# rooms, a label, a hidden device. Everything it keeps is under workdir/ha-testbed. Run it again
# to start a stopped server.
#
#   up.sh             the demo home
#   up.sh --many 400  the demo home and 400 more devices spread over 40 rooms, for sizes and timings
#   up.sh --tls       the demo home over https, with a certificate it signs itself
#
# INSTANCE=big PORT=8124 up.sh --many 3000 runs a second server beside the first, as lounge-ha-big,
# with its files under workdir/ha-testbed/beds/big. The other scripts take INSTANCE and PORT too.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(dirname "$(dirname "$(dirname "$HERE")")")
INSTANCE=${INSTANCE:-}
BED=$ROOT/workdir/ha-testbed${INSTANCE:+/beds/$INSTANCE}
IMAGE=ghcr.io/home-assistant/home-assistant:2026.11.2
NAME=lounge-ha${INSTANCE:+-$INSTANCE}
NETWORK=lounge-test
PORT=${PORT:-8123}
MANY=0
TLS=
while [ $# -gt 0 ]; do
  case $1 in
    --many) MANY=$2; shift 2 ;;
    --tls) TLS=1; shift ;;
    *) echo "usage: up.sh [--many N] [--tls]" >&2; exit 1 ;;
  esac
done

podman network exists "$NETWORK" || podman network create "$NETWORK" >/dev/null

if podman container exists "$NAME"; then
  podman start "$NAME" >/dev/null
else
  mkdir -p "$BED/config" "$BED/state"
  mkdir -p -m 700 "$BED/tokens"
  if [ ! -f "$BED/config/configuration.yaml" ]; then
    cat >"$BED/config/configuration.yaml" <<'YAML'
# Written by tools/plugins/homeassistant/up.sh.
default_config:
demo:

homeassistant:
  name: Lounge test home
  latitude: 52.52
  longitude: 13.41
  unit_system: metric
  time_zone: Europe/Berlin
  country: DE
  allowlist_external_dirs:
    - /config

person:
  - name: Ana
    id: ana
  - name: Ben
    id: ben
  - name: Dee
    id: dee

input_boolean:
  front_door_open:
    name: Front door open
  kitchen_window_open:
    name: Kitchen window open
  kitchen_leak:
    name: Kitchen leak
  hall_smoke:
    name: Hall smoke
  guest_mode:
    name: Guest mode
  back_door_locked:
    name: Back door locked
    initial: true
  cellar_door_locked:
    name: Cellar door locked
    initial: true
  broken_lamp_power:
    name: Broken lamp power
  humidifier_plug:
    name: Humidifier plug

input_select:
  house_alarm_state:
    name: House alarm state
    options: [disarmed, armed_home, armed_away, armed_night]
    initial: disarmed

input_number:
  hall_sensor_battery:
    name: Hall sensor battery level
    min: 0
    max: 100
    step: 1
    initial: 80

template: !include templates.yaml

# The demo's speakers, locks and cameras have no place in the registry, so they cannot be given a
# room. These stand in front of them, and the setup script puts them in rooms.
media_player:
  - platform: universal
    name: Living Room Speaker
    unique_id: lounge_living_room_speaker
    children:
      - media_player.living_room
  - platform: universal
    name: Kitchen Radio
    unique_id: lounge_kitchen_radio
    children:
      - media_player.kitchen
  - platform: universal
    name: Bedroom TV
    unique_id: lounge_bedroom_tv
    device_class: tv
    children:
      - media_player.lounge_room

generic_hygrostat:
  - name: Bedroom Humidifier
    unique_id: lounge_bedroom_humidifier
    humidifier: input_boolean.humidifier_plug
    target_sensor: sensor.outside_humidity

light:
  - platform: group
    name: Downstairs Lights
    unique_id: lounge_downstairs_lights
    entities:
      - light.kitchen_lights
      - light.ceiling_lights

scene:
  - id: lounge_movie_time
    name: Movie time
    entities:
      light.ceiling_lights:
        state: "on"
        brightness: 40
      light.kitchen_lights: "off"
  - id: lounge_good_night
    name: Good night
    entities:
      light.ceiling_lights: "off"
      light.kitchen_lights: "off"
      light.bed_light:
        state: "on"
        brightness: 25
  - id: lounge_bedtime_reading
    name: Bedtime reading
    entities:
      light.bed_light:
        state: "on"
        brightness: 120

script:
  welcome_home:
    alias: Welcome home
    sequence:
      - action: light.turn_on
        target:
          entity_id: light.kitchen_lights
  slow_fade:
    alias: Slow fade
    sequence:
      - delay: "00:00:20"
      - action: light.turn_off
        target:
          entity_id: light.kitchen_lights
YAML
    {
      cat <<'YAML'
# Doors, a leak, smoke and a battery that tools/plugins/homeassistant/flip.sh can change.
- binary_sensor:
    - name: Front Door
      unique_id: lounge_front_door
      device_class: door
      state: "{{ is_state('input_boolean.front_door_open', 'on') }}"
    - name: Kitchen Window
      unique_id: lounge_kitchen_window
      device_class: window
      state: "{{ is_state('input_boolean.kitchen_window_open', 'on') }}"
    - name: Kitchen Leak
      unique_id: lounge_kitchen_leak
      device_class: moisture
      state: "{{ is_state('input_boolean.kitchen_leak', 'on') }}"
    - name: Hall Smoke
      unique_id: lounge_hall_smoke
      device_class: smoke
      state: "{{ is_state('input_boolean.hall_smoke', 'on') }}"
- sensor:
    - name: Hall Sensor Battery
      unique_id: lounge_hall_sensor_battery
      device_class: battery
      unit_of_measurement: "%"
      state: "{{ states('input_number.hall_sensor_battery') | int(0) }}"
# A lock that opens without a code and one that needs four digits.
- lock:
    - name: Back Door
      unique_id: lounge_back_door
      state: "{{ is_state('input_boolean.back_door_locked', 'on') }}"
      lock:
        - action: input_boolean.turn_on
          target:
            entity_id: input_boolean.back_door_locked
      unlock:
        - action: input_boolean.turn_off
          target:
            entity_id: input_boolean.back_door_locked
    - name: Cellar Door
      unique_id: lounge_cellar_door
      code_format: "{{ '\\d{4}' }}"
      state: "{{ is_state('input_boolean.cellar_door_locked', 'on') }}"
      lock:
        - action: input_boolean.turn_on
          target:
            entity_id: input_boolean.cellar_door_locked
      unlock:
        - action: input_boolean.turn_off
          target:
            entity_id: input_boolean.cellar_door_locked
# An alarm that asks for no code, unlike the demo's.
- alarm_control_panel:
    - name: House Alarm
      unique_id: lounge_house_alarm
      state: "{{ states('input_select.house_alarm_state') }}"
      code_arm_required: false
      code_format: no_code
      arm_home:
        - action: input_select.select_option
          target:
            entity_id: input_select.house_alarm_state
          data:
            option: armed_home
      arm_away:
        - action: input_select.select_option
          target:
            entity_id: input_select.house_alarm_state
          data:
            option: armed_away
      arm_night:
        - action: input_select.select_option
          target:
            entity_id: input_select.house_alarm_state
          data:
            option: armed_night
      disarm:
        - action: input_select.select_option
          target:
            entity_id: input_select.house_alarm_state
          data:
            option: disarmed
# A lamp that does not answer until its power is switched on.
- light:
    - name: Broken Lamp
      unique_id: lounge_broken_lamp
      availability: "{{ is_state('input_boolean.broken_lamp_power', 'on') }}"
      turn_on: []
      turn_off: []
YAML
      if [ "$MANY" -gt 0 ]; then
        echo "- light:"
        for n in $(seq 1 $((MANY / 2))); do
          printf '    - name: Bulk Light %d\n      unique_id: lounge_bulk_light_%d\n      turn_on: []\n      turn_off: []\n      set_level: []\n' "$n" "$n"
        done
        echo "- switch:"
        for n in $(seq 1 $((MANY / 4))); do
          printf '    - name: Bulk Switch %d\n      unique_id: lounge_bulk_switch_%d\n      turn_on: []\n      turn_off: []\n' "$n" "$n"
        done
        echo "- sensor:"
        for n in $(seq 1 $((MANY / 4))); do
          printf '    - name: Bulk Temperature %d\n      unique_id: lounge_bulk_temperature_%d\n      device_class: temperature\n      unit_of_measurement: "°C"\n      state: "{{ 18 + %d %% 7 }}"\n' "$n" "$n" "$n"
        done
      fi
    } >"$BED/config/templates.yaml"
  fi
  if [ -n "$TLS" ] && [ ! -f "$BED/config/tls.crt" ]; then
    # The image has no openssl, so its Python makes the key and the certificate.
    podman run --rm -e NAME="$NAME" -v "$BED/config:/config:Z" "$IMAGE" python3 -c '
import datetime, os
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID
key = ec.generate_private_key(ec.SECP256R1())
name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, os.environ["NAME"])])
now = datetime.datetime.now(datetime.timezone.utc)
names = x509.SubjectAlternativeName([x509.DNSName(os.environ["NAME"]), x509.DNSName("localhost")])
cert = x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(key.public_key()).serial_number(x509.random_serial_number())
cert = cert.not_valid_before(now).not_valid_after(now + datetime.timedelta(days=3650)).add_extension(names, critical=False).sign(key, hashes.SHA256())
open("/config/tls.key", "wb").write(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
open("/config/tls.crt", "wb").write(cert.public_bytes(serialization.Encoding.PEM))
'
    # Home Assistant takes these from the file only the first time it starts, then keeps its own copy.
    printf '\nhttp:\n  ssl_certificate: /config/tls.crt\n  ssl_key: /config/tls.key\n' >>"$BED/config/configuration.yaml"
  fi
  podman run -d --name "$NAME" --network "$NETWORK" -p "$PORT:8123" -e TZ=Europe/Berlin -v "$BED/config:/config:Z" "$IMAGE" >/dev/null
fi
# The other scripts read from here whether the server speaks https.
SCHEME=http; [ ! -f "$BED/config/tls.crt" ] || SCHEME=https
echo "$SCHEME" >"$BED/scheme"

printf 'Waiting for Home Assistant'
for _ in $(seq 1 300); do
  if [ "$(curl -ks -o /dev/null -w '%{http_code}' "$SCHEME://localhost:$PORT/manifest.json" || true)" = 200 ]; then break; fi
  printf '.'; sleep 2
done
echo

if [ ! -s "$BED/tokens/admin" ]; then
  podman cp "$HERE/setup.py" "$NAME:/tmp/setup.py"
  # The script prints one line of JSON with the sign-ins it made and the tokens.
  made=$(podman exec -e LOUNGE_MANY="$MANY" -e LOUNGE_SCHEME="$SCHEME" "$NAME" python3 /tmp/setup.py)
  (
    umask 077
    jq -r .admin_token <<<"$made" >"$BED/tokens/admin"
    jq -r .user_token <<<"$made" >"$BED/tokens/user"
    jq -r .reader_token <<<"$made" >"$BED/tokens/reader"
    jq 'del(.admin_token, .user_token, .reader_token)' <<<"$made" >"$BED/tokens/login.json"
  )
fi

echo "Home Assistant is at $SCHEME://localhost:$PORT, and from the TV at $SCHEME://$(hostname -I | awk '{print $1}'):$PORT"
echo "Tokens and the sign-in are in $BED/tokens"
