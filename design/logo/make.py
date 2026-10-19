#!/usr/bin/env python3
"""Renders every size of the Lounge mark from the two source pictures in this folder.

sofa.png           the plush coral sofa on transparency
sofa-gradient.png  the same sofa on its indigo-to-orange gradient; only the gradient is used,
                   refitted as a smooth surface so it can be drawn at any aspect without the sofa

Writes the adaptive launcher icon layers, the Android TV banner, the top-bar mark, the
companion web icons and design/icon-512.png. Usage: python3 design/logo/make.py
"""
import os

import numpy as np
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
APP_RES = os.path.join(ROOT, "android/app/src/main/res")
DESIGN_RES = os.path.join(ROOT, "android/core/design/src/main/res")
COMPANION = os.path.join(ROOT, "companion/public")
FONT = os.path.join(ROOT, "design/fonts/BricolageGrotesque.ttf")
LIMELIGHT = (0xF6, 0xEF, 0xE7)

DENSITIES = {"mdpi": 1.0, "hdpi": 1.5, "xhdpi": 2.0, "xxhdpi": 3.0, "xxxhdpi": 4.0}


def load_sofa():
    """The sofa cropped to its pixels, premultiplied so resizing never bleeds the black under its edges."""
    img = np.array(Image.open(os.path.join(HERE, "sofa.png")).convert("RGBA")).astype(np.float64)
    alpha = img[..., 3]
    rows = np.where((alpha > 8).any(axis=1))[0]
    cols = np.where((alpha > 8).any(axis=0))[0]
    img = img[rows.min():rows.max() + 1, cols.min():cols.max() + 1]
    img[..., 3][img[..., 3] < 3] = 0
    pre = img.copy()
    pre[..., :3] *= pre[..., 3:4] / 255
    return pre


def sofa_at(pre, width):
    """The premultiplied sofa resized to `width` px, returned straight (un-premultiplied) RGBA."""
    h = round(pre.shape[0] * width / pre.shape[1])
    im = Image.fromarray(np.clip(pre, 0, 255).astype(np.uint8), "RGBA").resize((width, h), Image.LANCZOS)
    arr = np.array(im).astype(np.float64)
    a = arr[..., 3:4]
    arr[..., :3] = np.where(a > 0, arr[..., :3] * 255 / np.maximum(a, 1), 0)
    return Image.fromarray(np.clip(arr, 0, 255).astype(np.uint8), "RGBA")


def fit_gradient():
    """A cubic surface per channel fitted to the gradient picture outside the sofa."""
    g = np.array(Image.open(os.path.join(HERE, "sofa-gradient.png")).convert("RGB")).astype(np.float64)
    h, w, _ = g.shape
    ys, xs = np.mgrid[0:h, 0:w]
    outside = ~((xs > w * 0.09) & (xs < w * 0.91) & (ys > h * 0.26) & (ys < h * 0.76))
    sel = outside & (((ys % 4) == 0) & ((xs % 4) == 0))
    u = xs[sel] / w
    v = ys[sel] / h
    basis = lambda u, v: np.stack([np.ones_like(u), u, v, u * u, u * v, v * v, u ** 3, u * u * v, u * v * v, v ** 3], axis=-1)
    coeffs = [np.linalg.lstsq(basis(u, v), g[..., c][sel], rcond=None)[0] for c in range(3)]

    def render(width, height, v0=0.0, v1=1.0):
        vv, uu = np.mgrid[0:height, 0:width]
        uu = (uu + 0.5) / width
        vv = v0 + (v1 - v0) * (vv + 0.5) / height
        b = basis(uu, vv)
        out = np.stack([b @ coeffs[c] for c in range(3)], axis=-1)
        return Image.fromarray(np.clip(out, 0, 255).astype(np.uint8), "RGB")

    return render


def save(img, *path):
    full = os.path.join(*path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    img.save(full, optimize=True)
    return full


def paste_centered(canvas, sofa, cx, cy):
    canvas.alpha_composite(sofa, (round(cx - sofa.width / 2), round(cy - sofa.height / 2)))


def launcher_icon(pre, gradient):
    """Adaptive icon layers on the 108 dp canvas; the sofa fills 58 dp, inside the 66 dp safe zone."""
    for name, scale in DENSITIES.items():
        px = round(108 * scale)
        save(gradient(px, px), APP_RES, f"mipmap-{name}", "ic_launcher_background.png")
        fg = Image.new("RGBA", (px, px), (0, 0, 0, 0))
        sofa = sofa_at(pre, round(58 * scale))
        paste_centered(fg, sofa, px / 2, px / 2)
        save(fg, APP_RES, f"mipmap-{name}", "ic_launcher_foreground.png")
        mono = Image.new("RGBA", (px, px), (255, 255, 255, 0))
        mono.putalpha(fg.getchannel("A"))
        save(mono, APP_RES, f"mipmap-{name}", "ic_launcher_monochrome.png")


def wordmark(text, px_size, color):
    font = ImageFont.truetype(FONT, px_size)
    font.set_variation_by_axes([96, 700, 100])
    left, top, right, bottom = font.getbbox(text)
    img = Image.new("RGBA", (right - left, bottom - top), (0, 0, 0, 0))
    ImageDraw.Draw(img).text((-left, -top), text, font=font, fill=color + (255,))
    return img


def banner(pre, gradient):
    """320 × 180 dp: the sofa on the left, "Lounge" set in Bricolage Grotesque to its right."""
    for name in ("xhdpi", "xxhdpi", "xxxhdpi"):
        scale = DENSITIES[name]
        w, h = round(320 * scale), round(180 * scale)
        img = gradient(w, h, 0.2, 0.8).convert("RGBA")
        sofa = sofa_at(pre, round(128 * scale))
        paste_centered(img, sofa, 22 * scale + sofa.width / 2, h / 2)
        word = wordmark("Lounge", round(40 * scale), LIMELIGHT)
        x = round(164 * scale)
        img.alpha_composite(word, (x, round(h / 2 - word.height / 2)))
        save(img.convert("RGB"), APP_RES, f"drawable-{name}", "banner.png")


def top_bar_mark(pre):
    """The sofa alone, 42 dp wide, for the top bar."""
    for name in ("xhdpi", "xxhdpi", "xxxhdpi"):
        save(sofa_at(pre, round(42 * DENSITIES[name])), DESIGN_RES, f"drawable-{name}", "mark_sofa.png")


def web_icons(pre, gradient):
    """Full-bleed maskable icons: the sofa inside the central 80 %."""
    for size, path in ((512, (COMPANION, "icon-512.png")), (192, (COMPANION, "icon-192.png")), (180, (COMPANION, "apple-touch-icon.png")), (512, (ROOT, "design", "icon-512.png"))):
        img = gradient(size, size).convert("RGBA")
        paste_centered(img, sofa_at(pre, round(size * 0.68)), size / 2, size / 2)
        save(img.convert("RGB"), *path)


def main():
    pre = load_sofa()
    gradient = fit_gradient()
    launcher_icon(pre, gradient)
    banner(pre, gradient)
    top_bar_mark(pre)
    web_icons(pre, gradient)
    save(sofa_at(pre, 264), COMPANION, "mark.png")
    save(sofa_at(pre, 480), ROOT, "docs", "logo.png")
    print("done")


if __name__ == "__main__":
    main()
