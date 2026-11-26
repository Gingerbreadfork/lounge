#!/usr/bin/env python3
"""Fails when a method in an APK needs more than 256 registers, which a device may refuse to verify.

It lists every method within reach of that, largest first. The app's build runs it on each APK
it assembles, and it can be run on one by hand:

    tools/dex_registers.py android/app/build/outputs/apk/debug/app-debug.apk
"""
import glob
import os
import subprocess
import sys
import tempfile
import zipfile

LIMIT = 256
NEAR = 240


def dexdump():
    sdk = os.environ.get("ANDROID_HOME") or os.path.expanduser("~/Android/Sdk")
    tools = sorted(glob.glob(os.path.join(sdk, "build-tools", "*", "dexdump")))
    return tools[-1] if tools else None


def methods(apk, tool):
    with tempfile.TemporaryDirectory() as tmp, zipfile.ZipFile(apk) as z:
        for name in z.namelist():
            if not (name.startswith("classes") and name.endswith(".dex")):
                continue
            out = subprocess.run([tool, z.extract(name, tmp)], capture_output=True).stdout.decode("utf-8", "replace")
            owner = method = ""
            for line in out.splitlines():
                words = line.split()
                if line.startswith("  Class descriptor"):
                    owner = words[-1].strip("'")
                elif len(words) == 3 and words[:2] == ["name", ":"]:
                    method = words[2].strip("'")
                elif len(words) == 3 and words[:2] == ["registers", ":"]:
                    yield int(words[2]), owner, method


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    # A release build renames its classes, so every method counts and not only Lounge's own by name.
    tool = dexdump()
    if tool is None:
        print("dex registers not checked: no dexdump in the Android SDK's build-tools")
        return
    large = sorted((m for m in methods(sys.argv[1], tool) if m[0] >= NEAR), reverse=True)
    for registers, owner, method in large:
        print(f"{registers:4d}  {owner[1:-1].replace('/', '.')}.{method}")
    over = [m for m in large if m[0] > LIMIT]
    if over:
        sys.exit(f"{len(over)} over {LIMIT} registers: move part of each into a function of its own")
    print(f"nothing over {LIMIT}" + (f"; {len(large)} within {LIMIT - NEAR} of it" if large else ""))


if __name__ == "__main__":
    main()
