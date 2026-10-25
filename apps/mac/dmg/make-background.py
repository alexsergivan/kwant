"""Draws the kwant DMG window background: background.png (660x400) and background@2x.png.

Run with: python3 apps/mac/dmg/make-background.py   (needs Pillow)
The window is 660x400 with kwant.app at (180, 170) and Applications at (480, 170),
matching bundle.macOS.dmg in tauri.conf.json. Plain art only, no text.
"""
import math
import os

from PIL import Image, ImageDraw, ImageFilter

W, H = 660, 400
APP_X, APPS_X, ICON_Y = 180, 480, 170
SS = 4  # supersampling for the 2x image

# Light on purpose: over a background picture Finder draws icon labels black in light and dark
# mode alike, so a dark background makes "kwant" and "Applications" unreadable.
TOP = (246, 244, 251)
BOTTOM = (231, 228, 240)
ARROW = (112, 104, 150, 255)


def draw(scale):
    s = scale * SS
    w, h = W * s, H * s
    # vertical gradient
    grad = Image.new("RGB", (1, h))
    for y in range(h):
        t = y / (h - 1)
        grad.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)))
    img = grad.resize((w, h)).convert("RGBA")

    # soft glow behind each icon slot
    glow = Image.new("RGBA", (w, h), (255, 255, 255, 0))
    g = ImageDraw.Draw(glow)
    r = 92 * s
    for cx in (APP_X, APPS_X):
        g.ellipse((cx * s - r, ICON_Y * s - r, cx * s + r, ICON_Y * s + r), fill=(255, 255, 255, 150))
    glow = glow.filter(ImageFilter.GaussianBlur(40 * s))
    img = Image.alpha_composite(img, glow)

    # arrow: a shallow arc from the app slot to the Applications slot
    d = ImageDraw.Draw(img)
    x0, x1 = (APP_X + 90) * s, (APPS_X - 92) * s
    y = (ICON_Y - 4) * s
    sag = 14 * s
    width = round(4.5 * s)
    pts = []
    for i in range(61):
        t = i / 60
        pts.append((x0 + (x1 - x0) * t, y - sag * math.sin(math.pi * t)))
    d.line(pts, fill=ARROW, width=width, joint="curve")
    for px, py in (pts[0],):
        d.ellipse((px - width / 2, py - width / 2, px + width / 2, py + width / 2), fill=ARROW)
    # head, aligned with the arc's end tangent
    (ax, ay), (bx, by) = pts[-3], pts[-1]
    ang = math.atan2(by - ay, bx - ax)
    L, spread = 20 * s, math.radians(34)
    for side in (-1, 1):
        hx = bx - L * math.cos(ang + side * spread)
        hy = by - L * math.sin(ang + side * spread)
        d.line([(hx, hy), (bx, by)], fill=ARROW, width=width)
        d.ellipse((hx - width / 2, hy - width / 2, hx + width / 2, hy + width / 2), fill=ARROW)
    d.ellipse((bx - width / 2, by - width / 2, bx + width / 2, by + width / 2), fill=ARROW)

    return img.resize((W * scale, H * scale), Image.LANCZOS).convert("RGB")


here = os.path.dirname(os.path.abspath(__file__))
draw(1).save(os.path.join(here, "background.png"), dpi=(72, 72), optimize=True)
draw(2).save(os.path.join(here, "background@2x.png"), dpi=(144, 144), optimize=True)
