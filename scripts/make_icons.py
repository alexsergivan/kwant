#!/usr/bin/env python3
"""Draws every app icon from the kwant.si logo, so the Mac app, the menu bar, the web app and the site all match.

The logo is LOGO_RECTS on the FAV_BG violet in scripts/build_site.py, the same pixels as site/favicon.svg.
Logo pixels are scaled by whole numbers only (no resampling), so they stay crisp at every size.

Writes:
  apps/mac/icons/   icon.icns (16-1024, with @2x), icon.png, source-1024.png, 32x32.png, 128x128.png,
                    128x128@2x.png, tray-template.png (black silhouette for the menu bar, 22pt @2x)
  frontend/public/  icon.svg, icons/icon-192/512.png, icons/icon-maskable-192/512.png, and
                    icons/apple-touch-icon.png (a copy of site/apple-touch-icon.png)

Needs Pillow (pip install pillow). Run from anywhere: python3 scripts/make_icons.py
"""

import io
import pathlib
import struct
import sys

from PIL import Image, ImageDraw, ImageFilter

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
from build_site import FAV_BG, LOGO_RECTS, favicon_svg, hexrgb  # noqa: E402

MAC = ROOT / "apps/mac/icons"
WEB = ROOT / "frontend/public"

# The logo's pixels span x 2..13 and y 1..16 of its grid.
LOGO_X, LOGO_Y, LOGO_W, LOGO_H = 2, 1, 12, 16
# Cut out of the tray silhouette so the face still reads at 22 px.
FACE = {"#1d1147"}
SS = 4  # supersampling for the smooth tile edge and shadow


def logo(scale, silhouette=False):
    """The logo alone on transparent, `scale` px per logo pixel."""
    im = Image.new("RGBA", (LOGO_W * scale, LOGO_H * scale), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    for x, y, w, h, c in LOGO_RECTS:
        if silhouette:
            # The antenna stem is face-coloured but sits outside the body, so it stays solid.
            col = (0, 0, 0, 0) if c in FACE and (x, y) != (10, 4) else (0, 0, 0, 255)
        else:
            col = hexrgb(c) + (255,)
        x0, y0 = (x - LOGO_X) * scale, (y - LOGO_Y) * scale
        d.rectangle([x0, y0, x0 + w * scale - 1, y0 + h * scale - 1], fill=col)
    return im


def tile(size, side, radius, shadow=False, offset=(0, 0)):
    """A violet rounded square of `side` px centred on a `size` canvas, antialiased, with an optional soft drop shadow."""
    big = size * SS
    left = (big - side * SS) / 2 + offset[0] * SS
    top = (big - side * SS) / 2 + offset[1] * SS
    box = [left, top, left + side * SS - 1, top + side * SS - 1]
    im = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    if shadow:
        # Apple's grid: a soft shadow 10/1024 below the tile, about 28% black.
        sh = Image.new("RGBA", (big, big), (0, 0, 0, 0))
        dy = round(big * 10 / 1024)
        ImageDraw.Draw(sh).rounded_rectangle([box[0], box[1] + dy, box[2], box[3] + dy], radius * SS, fill=(0, 0, 0, 72))
        im = Image.alpha_composite(im, sh.filter(ImageFilter.GaussianBlur(big * 10 / 1024)))
    ImageDraw.Draw(im).rounded_rectangle(box, radius * SS, fill=hexrgb(FAV_BG) + (255,))
    return im.resize((size, size), Image.LANCZOS)


def mac_icon(size):
    """The Mac app icon at `size` px, on Apple's 1024 grid: an 824 px tile with 100 px margins and a soft shadow.

    16 and 32 px are the favicon itself, full bleed, because a 13 px tile can't hold the 16-pixel logo crisply.
    From 64 px up the logo has the padding of site/apple-touch-icon.png.
    """
    if size <= 32:
        scale = size // 16
        im = tile(size, size, 3 * scale)
        im.alpha_composite(logo(scale), ((size - LOGO_W * scale) // 2, (size - LOGO_H * scale) // 2))
        return im
    side = size * 824 / 1024
    # Same breathing room as the site's large touch icon: 8 px per logo pixel on 180.
    scale = max(1, int(side * 8 / 180))
    # Snap the tile's centre to whole pixels so the logo sits on the same grid as the tile.
    left = round((size - side) / 2)
    im = tile(size, side, side * 0.2237, shadow=True, offset=(left - (size - side) / 2, 0))
    lw, lh = LOGO_W * scale, LOGO_H * scale
    im.alpha_composite(logo(scale), (left + round((side - lw) / 2), round((size - lh) / 2)))
    return im


def png(im):
    buf = io.BytesIO()
    im.save(buf, "PNG", optimize=True)
    return buf.getvalue()


# icns entry types holding PNG data, and their pixel sizes. ic11-ic14 and ic10 are the @2x variants.
ICNS_TYPES = [
    (b"icp4", 16), (b"ic11", 32), (b"icp5", 32), (b"ic12", 64), (b"ic07", 128),
    (b"ic13", 256), (b"ic08", 256), (b"ic14", 512), (b"ic09", 512), (b"ic10", 1024),
]


def icns(images):
    body = b"".join(t + struct.pack(">I", 8 + len(images[s])) + images[s] for t, s in ICNS_TYPES)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def maskable(size, scale):
    """Full-bleed violet with the logo inside the maskable safe zone (a circle of 40% radius)."""
    im = Image.new("RGBA", (size, size), hexrgb(FAV_BG) + (255,))
    lw, lh = LOGO_W * scale, LOGO_H * scale
    assert (lw * lw + lh * lh) ** 0.5 / 2 <= 0.4 * size, f"logo leaves the safe zone at {size}"
    im.alpha_composite(logo(scale), ((size - lw) // 2, (size - lh) // 2))
    return im


def web_icon(size, scale):
    """The favicon's rounded tile, full bleed, with the logo at `scale` and the same padding ratio as the site touch icon."""
    im = tile(size, size, size * 3 / 16)
    lw, lh = LOGO_W * scale, LOGO_H * scale
    im.alpha_composite(logo(scale), ((size - lw) // 2, (size - lh) // 2))
    return im


def tray():
    """Menu-bar template: black silhouette with the face cut out, 2 px per logo pixel on a 44 px (22pt @2x) canvas."""
    im = Image.new("RGBA", (44, 44), (0, 0, 0, 0))
    im.alpha_composite(logo(2, silhouette=True), ((44 - 24) // 2, (44 - 32) // 2))
    return im


def main():
    sizes = {s: png(mac_icon(s)) for s in (16, 32, 64, 128, 256, 512, 1024)}
    (MAC / "icon.icns").write_bytes(icns(sizes))
    (MAC / "source-1024.png").write_bytes(sizes[1024])
    (MAC / "icon.png").write_bytes(sizes[512])
    (MAC / "128x128@2x.png").write_bytes(sizes[256])
    (MAC / "128x128.png").write_bytes(sizes[128])
    (MAC / "32x32.png").write_bytes(sizes[32])
    tray().save(MAC / "tray-template.png", optimize=True)

    (WEB / "icon.svg").write_text(favicon_svg())
    web_icon(192, 9).save(WEB / "icons/icon-192.png", optimize=True)
    web_icon(512, 24).save(WEB / "icons/icon-512.png", optimize=True)
    maskable(192, 7).save(WEB / "icons/icon-maskable-192.png", optimize=True)
    maskable(512, 20).save(WEB / "icons/icon-maskable-512.png", optimize=True)
    # iOS rounds the corners itself: the site's full-square touch icon is the one to use.
    (WEB / "icons/apple-touch-icon.png").write_bytes((ROOT / "site/apple-touch-icon.png").read_bytes())


if __name__ == "__main__":
    main()
