#!/usr/bin/env python3
"""Sets up the test Home Assistant. Run by up.sh inside the container, once.

It finishes the first-run steps, makes an administrator, a plain user and one who may only read,
with a long-lived token each, and gives the demo devices rooms, a label and a hidden device. The
last line it prints is JSON with the sign-ins and the tokens; everything else goes to stderr.
"""
import asyncio
import json
import os
import secrets
import shutil
import sys

import aiohttp

BASE = os.environ.get("LOUNGE_SCHEME", "http") + "://127.0.0.1:8123"
CLIENT = BASE + "/"

# Room name to what goes in it. A name ending in "*" places the entity's whole device there; so
# does any entity that is its device's main feature, since Home Assistant gives those no room of
# their own.
ROOMS = {
    "Living Room": [
        "light.ceiling_lights",
        "light.living_room_rgbww_lights",
        "cover.living_room_window",
        "climate.heatpump",
        "fan.living_room_fan",
        "media_player.living_room_speaker",
        "switch.decorative_lights",
        "scene.movie_time",
    ],
    "Kitchen": [
        "light.kitchen_lights",
        "cover.kitchen_window",
        "media_player.kitchen_radio",
        "switch.ac",
        "binary_sensor.kitchen_window",
        "binary_sensor.kitchen_leak",
        "vacuum.demo_vacuum_0_ground_floor",
    ],
    "Bedroom": [
        "light.bed_light",
        "light.broken_lamp",
        "climate.hvac",
        "media_player.bedroom_tv",
        "fan.ceiling_fan",
        "humidifier.bedroom_humidifier",
        "scene.bedtime_reading",
    ],
    "Büro & Café": [
        "light.office_rgbw_lights",
        "climate.ecobee",
        "cover.hall_window",
    ],
    "Hall": [
        "light.entrance_color_white_lights",
        "lock.back_door",
        "binary_sensor.front_door",
        "binary_sensor.hall_smoke",
        "sensor.hall_sensor_battery",
        "alarm_control_panel.house_alarm",
        "camera.porch",
    ],
    "Garage": [
        "cover.garage_door",
        "lock.cellar_door",
    ],
    "Attic": [
        "sensor.outside_temperature*",
    ],
}

LABEL = "TV"
# What carries the label: an entity, a whole device and a whole room.
LABELLED_ENTITIES = ["light.bed_light"]
LABELLED_DEVICES = ["cover.living_room_window"]
LABELLED_ROOMS = ["Büro & Café"]
HIDDEN = ["fan.preset_only_limited_fan"]


def note(text):
    print(text, file=sys.stderr, flush=True)


class Socket:
    """Home Assistant's WebSocket API, one command at a time."""

    def __init__(self, ws):
        self.ws = ws
        self.id = 0

    @classmethod
    async def open(cls, session, token):
        ws = await session.ws_connect(BASE + "/api/websocket", max_msg_size=0)
        await ws.receive_json()
        await ws.send_json({"type": "auth", "access_token": token})
        hello = await ws.receive_json()
        if hello.get("type") != "auth_ok":
            raise SystemExit(f"websocket sign-in failed: {hello}")
        return cls(ws)

    async def call(self, kind, **data):
        self.id += 1
        await self.ws.send_json({"id": self.id, "type": kind, **data})
        while True:
            answer = await self.ws.receive_json()
            if answer.get("id") == self.id and answer.get("type") == "result":
                if not answer.get("success"):
                    raise RuntimeError(f"{kind}: {answer.get('error')}")
                return answer.get("result")

    async def close(self):
        await self.ws.close()


async def token_for(session, code):
    async with session.post(BASE + "/auth/token", data={"grant_type": "authorization_code", "code": code, "client_id": CLIENT}) as r:
        r.raise_for_status()
        return (await r.json())["access_token"]


async def onboard(session, username, password):
    for _ in range(120):
        try:
            async with session.get(BASE + "/api/onboarding") as r:
                if r.status == 200:
                    steps = await r.json()
                    break
        except aiohttp.ClientError:
            pass
        await asyncio.sleep(1)
    else:
        raise SystemExit("Home Assistant never offered its first-run steps")
    if any(s["step"] == "user" and s["done"] for s in steps):
        raise SystemExit("This Home Assistant is already set up. Run down.sh --wipe, then up.sh.")
    body = {"client_id": CLIENT, "name": "Lounge Admin", "username": username, "password": password, "language": "en"}
    async with session.post(BASE + "/api/onboarding/users", json=body) as r:
        r.raise_for_status()
        access = await token_for(session, (await r.json())["auth_code"])
    bearer = {"Authorization": f"Bearer {access}"}
    for step, body in [("core_config", {}), ("analytics", {}), ("integration", {"client_id": CLIENT, "redirect_uri": CLIENT})]:
        async with session.post(f"{BASE}/api/onboarding/{step}", json=body, headers=bearer) as r:
            if r.status not in (200, 403):
                note(f"onboarding step {step} answered {r.status}")
    for _ in range(180):
        async with session.get(BASE + "/api/config", headers=bearer) as r:
            if r.status == 200 and (await r.json()).get("state") == "RUNNING":
                return access
        await asyncio.sleep(1)
    raise SystemExit("Home Assistant did not finish starting")


async def sign_in(session, username, password):
    """A short-lived token for a user, through the same steps the sign-in page takes."""
    flow = {"client_id": CLIENT, "handler": ["homeassistant", None], "redirect_uri": CLIENT}
    async with session.post(BASE + "/auth/login_flow", json=flow) as r:
        r.raise_for_status()
        flow_id = (await r.json())["flow_id"]
    async with session.post(f"{BASE}/auth/login_flow/{flow_id}", json={"username": username, "password": password, "client_id": CLIENT}) as r:
        r.raise_for_status()
        return await token_for(session, (await r.json())["result"])


async def long_lived(session, access, name):
    ws = await Socket.open(session, access)
    try:
        return await ws.call("auth/long_lived_access_token", client_name=name, lifespan=3650)
    finally:
        await ws.close()


async def furnish(ws, many):
    """Rooms, the label, the hidden device, and rooms for the extra devices of a large home."""
    entities = {e["entity_id"]: e for e in await ws.call("config/entity_registry/list")}
    # The first-run steps make a few rooms of their own, which are used as they are.
    areas = {a["name"]: a["area_id"] for a in await ws.call("config/area_registry/list")}
    for name in ROOMS:
        if name not in areas:
            areas[name] = (await ws.call("config/area_registry/create", name=name))["area_id"]
    label = (await ws.call("config/label_registry/create", name=LABEL))["label_id"]

    async def attempt(kind, **data):
        try:
            await ws.call(kind, **data)
            return True
        except RuntimeError as e:
            note(str(e))
            return False

    async def place(entity_id, area_id, whole_device=False):
        if entity_id not in entities:
            note(f"no {entity_id} in this Home Assistant")
            return
        device = entities[entity_id].get("device_id")
        if whole_device and device:
            await attempt("config/device_registry/update", device_id=device, area_id=area_id)
            return
        try:
            await ws.call("config/entity_registry/update", entity_id=entity_id, area_id=area_id)
        except RuntimeError:
            # An entity that is its device's main feature takes its room from the device.
            if device:
                await attempt("config/device_registry/update", device_id=device, area_id=area_id)

    for name, members in ROOMS.items():
        for member in members:
            await place(member.rstrip("*"), areas[name], member.endswith("*"))
    for entity_id in LABELLED_ENTITIES:
        await attempt("config/entity_registry/update", entity_id=entity_id, labels=[label])
    for entity_id in LABELLED_DEVICES:
        if device := entities.get(entity_id, {}).get("device_id"):
            await attempt("config/device_registry/update", device_id=device, labels=[label])
    for name in LABELLED_ROOMS:
        await attempt("config/area_registry/update", area_id=areas[name], labels=[label])
    for entity_id in HIDDEN:
        await attempt("config/entity_registry/update", entity_id=entity_id, hidden_by="user")
    # The camera is named after its file.
    await attempt("config/entity_registry/update", entity_id="camera.porch", name="Porch")

    bulk = sorted(e for e in entities if "bulk_" in e)
    if many and bulk:
        rooms = [(await ws.call("config/area_registry/create", name=f"Room {n}"))["area_id"] for n in range(1, 41)]
        for n, entity_id in enumerate(bulk):
            await place(entity_id, rooms[n % len(rooms)])
        note(f"{len(bulk)} extra devices in {len(rooms)} rooms")
    note(f"{len(areas)} rooms, label {label}")


async def add_camera(session, access):
    """A camera that can be given a room, showing one of the demo's own pictures."""
    shutil.copy("/usr/src/homeassistant/homeassistant/components/demo/demo_0.jpg", "/config/porch.jpg")
    bearer = {"Authorization": f"Bearer {access}"}
    async with session.post(BASE + "/api/config/config_entries/flow", json={"handler": "local_file"}, headers=bearer) as r:
        if r.status >= 300:
            note(f"starting a camera answered {r.status}")
            return
        flow_id = (await r.json())["flow_id"]
    async with session.post(f"{BASE}/api/config/config_entries/flow/{flow_id}", json={"file_path": "/config/porch.jpg"}, headers=bearer) as r:
        answer = await r.json()
        if answer.get("type") != "create_entry":
            note(f"adding a camera answered {r.status}: {answer}")


async def add_phone(session, access):
    """A phone with a nearly flat battery, which the TV should not report."""
    bearer = {"Authorization": f"Bearer {access}"}
    phone = {
        "device_id": "lounge-test-phone", "app_id": "tv.lounge.testbed", "app_name": "Lounge test bed", "app_version": "1.0",
        "device_name": "Ana's Phone", "manufacturer": "Lounge", "model": "Test", "os_name": "Android", "os_version": "16", "supports_encryption": False,
    }
    async with session.post(BASE + "/api/mobile_app/registrations", json=phone, headers=bearer) as r:
        if r.status >= 300:
            note(f"registering a phone answered {r.status}")
            return
        webhook = (await r.json())["webhook_id"]
    sensor = {"type": "register_sensor", "data": {"type": "sensor", "unique_id": "battery_level", "name": "Battery Level", "state": 7, "device_class": "battery", "unit_of_measurement": "%"}}
    async with session.post(f"{BASE}/api/webhook/{webhook}", json=sensor) as r:
        if r.status >= 300:
            note(f"registering the phone's battery answered {r.status}")


async def main():
    many = int(os.environ.get("LOUNGE_MANY", "0") or 0)
    made = {"username": "admin", "password": secrets.token_urlsafe(12), "user_username": "guest", "user_password": secrets.token_urlsafe(12)}
    made |= {"reader_username": "reader", "reader_password": secrets.token_urlsafe(12)}
    # The server's own certificate, when it has one, is not one anybody else signed.
    async with aiohttp.ClientSession(connector=aiohttp.TCPConnector(ssl=False)) as session:
        access = await onboard(session, made["username"], made["password"])
        made["admin_token"] = await long_lived(session, access, "Lounge test bed")
        # A plain user may control the home but not list its rooms. A reader may do neither.
        for who, name, group in [("user", "Guest", "system-users"), ("reader", "Reader", "system-read-only")]:
            ws = await Socket.open(session, access)
            try:
                user = (await ws.call("config/auth/create", name=name, group_ids=[group], local_only=False))["user"]
                await ws.call("config/auth_provider/homeassistant/create", user_id=user["id"], username=made[f"{who}_username"], password=made[f"{who}_password"])
            finally:
                await ws.close()
            signed_in = await sign_in(session, made[f"{who}_username"], made[f"{who}_password"])
            made[f"{who}_token"] = await long_lived(session, signed_in, "Lounge test bed")
        # The tokens are worth having even when a room or a label could not be made.
        try:
            await add_camera(session, access)
            await add_phone(session, access)
            ws = await Socket.open(session, access)
            await furnish(ws, many)
            await ws.close()
        except Exception as e:
            note(f"setting up the rooms stopped early: {e}")
    print(json.dumps(made))


asyncio.run(main())
