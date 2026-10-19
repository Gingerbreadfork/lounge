"""Cuts the static font instances the design uses from the variable Bricolage Grotesque and
Figtree files (OFL). Run from the repo root with a venv that has fonttools:
    python tools/fonts/instance.py path/to/bricolage_grotesque.ttf path/to/figtree.ttf
Writes into android/core/design/src/main/res/font/."""
import os
import sys
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

OUT = os.path.join(os.path.dirname(__file__), "..", "..", "android", "core", "design", "src", "main", "res", "font")
STYLES = {
    "bricolage_display_xl": (0, dict(wght=700, opsz=96, wdth=100)),
    "bricolage_display_l": (0, dict(wght=650, opsz=48, wdth=100)),
    "bricolage_headline": (0, dict(wght=600, opsz=34, wdth=100)),
    "bricolage_clock": (0, dict(wght=500, opsz=40, wdth=100)),
    "figtree_semibold": (1, dict(wght=600)),
    "figtree_medium": (1, dict(wght=500)),
    "figtree_regular": (1, dict(wght=400)),
}

def main(sources):
    for name, (src_index, axes) in STYLES.items():
        font = TTFont(sources[src_index])
        static = instancer.instantiateVariableFont(font, axes, inplace=False, updateFontNames=False)
        out = os.path.join(OUT, name + ".ttf")
        static.save(out)
        print(f"{out}: {os.path.getsize(out)} bytes")

if __name__ == "__main__":
    main(sys.argv[1:3])
