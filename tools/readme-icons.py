"""Writes the app's own icons from Icons.kt as SVGs for the README, in the lamp's coral.

    python3 tools/readme-icons.py play apps guide tv theme phone frame

Each named icon goes to docs/icons/<name>.svg. resume.svg and plugin.svg there have no icon in
the app and are drawn to the same grid: 28 units, 2.2 strokes with round ends.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICONS = os.path.join(ROOT, "android/core/design/src/main/kotlin/tv/lounge/design/Icons.kt")
OUT = os.path.join(ROOT, "docs/icons")
COLOUR = "#FFA86B"
COMMANDS = {"moveTo": "M", "lineTo": "L", "curveTo": "C", "close": "Z"}


def number(s):
    return s.strip().rstrip("f")


def path_data(builder):
    parts = []
    for fn, args in re.findall(r"(moveTo|lineTo|arcTo|curveTo|close)\(([^)]*)\)", builder):
        a = [number(x) for x in args.split(",")] if args.strip() else []
        if fn == "arcTo":
            rx, ry, rotation, large, sweep, x, y = a
            parts.append(f"A{rx} {ry} {rotation} {int(large == 'true')} {int(sweep == 'true')} {x} {y}")
        else:
            parts.append(COMMANDS[fn] + " ".join(a))
    return " ".join(parts)


def main(names):
    source = open(ICONS).read()
    icons = {m.group(1): m.group(2) for m in re.finditer(r'icon\("(\w+)"\) \{(.*?)\n        \}\n    \}', source, re.S)}
    for name in names:
        paths = []
        for kind, builder in re.findall(r"\b(stroke|fill) \{(.*?)\}", icons[name], re.S):
            d = path_data(builder)
            if kind == "stroke":
                paths.append(f'<path d="{d}" fill="none" stroke="{COLOUR}" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>')
            else:
                paths.append(f'<path d="{d}" fill="{COLOUR}"/>')
        svg = f'<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 28 28">{"".join(paths)}</svg>\n'
        with open(os.path.join(OUT, f"{name}.svg"), "w") as f:
            f.write(svg)


if __name__ == "__main__":
    main(sys.argv[1:])
