#!/usr/bin/env python3
"""Draws the social preview images (Open Graph / Twitter cards) for the kwant website.

One 1200x630 PNG per page in site-src/, written to site/assets/og/<slug>.png, where <slug> is the page's output path
without .html and with / turned into - (index.html -> home.png, docs/quickstart.html -> docs-quickstart.png).
build_site.py points each page's og:image and twitter:image at its own card, so run this first, then build_site.py:

    python3 scripts/build_og.py && python3 scripts/build_site.py

Needs Pillow (`pip install pillow`); everything else (fonts, logo, avatars, the town still) comes from the repo.
Each card uses three optional header keys in the page source:
  card_title: the big title (falls back to the page title up to its first colon), so a search-led
    <title> can change without changing the card
  card: a short line under the title (falls back to the description's first sentence)
  card_art: what sits on the CRT screen: scene (the pixel town), team (every role's avatar) or avatar:ROLE
"""
import pathlib
import sys

from PIL import Image, ImageDraw, ImageFont

sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import build_site as site  # noqa: E402

W, H = 1200, 630
FONTS = site.OUT / "assets/fonts"
INK, MUTED, VIOLET, LINE = "#1b1033", "#4d4566", "#4024a8", "#cfc6f0"
BG_TOP, BG_BOTTOM = (247, 245, 255), (236, 230, 255)
SCREEN = (652, 92, 1136, 538)  # the CRT, frame included
BEZEL = 56


def font(name, size, weight):
    f = ImageFont.truetype(str(FONTS / f"{name}.woff2"), size)
    f.set_variation_by_axes([weight])
    return f


def card_title(meta):
    """The card's big title: the page's short name, not its search-led <title>."""
    return meta.get("card_title") or meta["title"].split(":")[0]


def slug(path):
    return "home" if path == "index.html" else path.removesuffix(".html").replace("/", "-")


def wrap(draw, text, fnt, width):
    lines, line = [], ""
    for word in text.split():
        trial = f"{line} {word}".strip()
        if line and draw.textlength(trial, font=fnt) > width:
            lines.append(line)
            line = word
        else:
            line = trial
    return lines + [line] if line else lines


def stepped(draw, box, fill, u):
    """A rectangle with the site's stepped pixel corners (the --steps clip-path), u pixels per step."""
    x0, y0, x1, y1 = box
    draw.rectangle((x0 + 4 * u, y0, x1 - 4 * u, y1), fill=fill)
    draw.rectangle((x0 + 2 * u, y0 + u, x1 - 2 * u, y1 - u), fill=fill)
    draw.rectangle((x0 + u, y0 + 2 * u, x1 - u, y1 - 2 * u), fill=fill)
    draw.rectangle((x0, y0 + 4 * u, x1, y1 - 4 * u), fill=fill)


def rects(draw, items, ox, oy, scale):
    for x, y, w, h, c in items:
        draw.rectangle((ox + x * scale, oy + y * scale, ox + (x + w) * scale - 1, oy + (y + h) * scale - 1), fill=c)


def background():
    img = Image.new("RGB", (W, H))
    draw = ImageDraw.Draw(img)
    for y in range(H):
        t = y / (H - 1)
        draw.line([(0, y), (W, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(BG_TOP, BG_BOTTOM)))
    return img


def screen_art(img, art, area):
    """Fills the CRT panel `area` with the town still, the whole team or one avatar, pixel-crisp."""
    x0, y0, x1, y1 = area
    w, h = x1 - x0, y1 - y0
    draw = ImageDraw.Draw(img)
    if art == "scene":
        still = Image.open(site.OUT / "assets/media/scene-still.png").convert("RGB")
        # The still is dithered pixel art, so a fractional nearest-neighbour scale to the panel's width still reads crisp.
        scale = min(w / still.width, h / still.height)
        still = still.resize((round(still.width * scale), round(still.height * scale)), Image.NEAREST)
        img.paste(still, (x0 + (w - still.width) // 2, y0 + (h - still.height) // 2))
        return
    roles = list(site.AV_ROLES) if art == "team" else [art.split(":", 1)[1]]
    cols = 4 if len(roles) > 1 else 1
    rows = -(-len(roles) // cols)
    scale = min(w // (cols * 26), h // (rows * 26), 11)
    cw, ch = 26 * scale, 26 * scale
    gx, gy = x0 + (w - cols * cw) // 2, y0 + (h - rows * ch) // 2
    for i, role in enumerate(roles):
        rects(draw, site.avatar_rects(role), gx + (i % cols) * cw, gy + (i // cols) * ch, scale)


def scanlines(img, area):
    x0, y0, x1, y1 = area
    overlay = Image.new("RGBA", (x1 - x0, y1 - y0), (0, 0, 0, 0))
    od = ImageDraw.Draw(overlay)
    for y in range(0, y1 - y0, 4):
        od.line([(0, y), (x1 - x0, y)], fill=(13, 10, 31, 70))
    img.paste(overlay, (x0, y0), overlay)


def card(meta):
    path = meta["path"]
    img = background()
    draw = ImageDraw.Draw(img)

    # Logo and wordmark, as in the site header
    draw.rounded_rectangle((64, 52, 124, 112), radius=12, fill=site.FAV_BG)
    rects(draw, site.LOGO_RECTS, 64 + 30 - 8 * 3, 52 + 30 - 9 * 3, 3)
    draw.text((140, 82), "kwant", font=font("pixelify-sans", 44, 600), fill=INK, anchor="lm")

    left, width = 64, 548
    y = 168
    group = next((g for g, items in site.DOC_GROUPS for s, _ in items if meta.get("nav") == s), None)
    kicker = f"Docs · {group}" if group else ("Every feature" if meta.get("active") == "features" else "")
    if kicker:
        draw.text((left, y), kicker.upper(), font=font("inter-var", 21, 700), fill=VIOLET)
        y += 44

    title = card_title(meta)
    sub = meta.get("card") or meta["description"].split(". ")[0].rstrip(".") + "."
    sf = font("inter-var", 29, 400)
    sub_lines = wrap(draw, sub, sf, width - 20)[:3]
    # The largest title that still leaves room for the subtitle above the chip at y=540
    for size in (84, 76, 68, 60, 54):
        tf = font("pixelify-sans", size, 600)
        lines = wrap(draw, title, tf, width)
        if y + len(lines) * round(size * 1.08) + 22 + len(sub_lines) * 41 <= 516:
            break
    for line in lines:
        draw.text((left, y), line, font=tf, fill=INK)
        y += round(size * 1.08)
    y += 22
    for line in sub_lines:
        draw.text((left, y), line, font=sf, fill=MUTED)
        y += 41

    # The brand chip, bottom left
    cf = font("inter-var", 22, 600)
    chip = "Source-available · self-hosted · one binary"
    cw = draw.textlength(chip, font=cf)
    draw.rounded_rectangle((left, 540, left + cw + 80, 590), radius=25, fill="#ffffff", outline=LINE, width=2)
    draw.rectangle((left + 22, 558, left + 34, 570), fill="#3ccf6e")
    draw.text((left + 50, 565), chip, font=cf, fill="#3a3252", anchor="lm")

    # The CRT: stepped frame, dark panel with the art and scanlines, bezel with an LED and the page address
    sx0, sy0, sx1, sy1 = SCREEN
    stepped(draw, (sx0, sy0, sx1, sy1), site.AV_FRAME, 5)
    panel = (sx0 + 18, sy0 + 18, sx1 - 18, sy1 - BEZEL)
    draw.rectangle(panel, fill=site.AV_PANEL)
    screen_art(img, meta.get("card_art", "scene"), panel)
    scanlines(img, panel)
    by = sy1 - BEZEL // 2 + 2
    draw.rectangle((sx0 + 22, by - 6, sx0 + 34, by + 6), fill="#3ccf6e")
    address = "kwant.si/" + path.replace("index.html", "").removesuffix(".html")
    draw.text((sx0 + 48, by), address, font=font("inter-var", 21, 600), fill="#e1d6ff", anchor="lm")
    return img


def main():
    out = site.OUT / "assets/og"
    out.mkdir(parents=True, exist_ok=True)
    written = []
    for src in sorted(site.SRC.glob("*.html")):
        meta, _ = site.parse(src)
        if meta["path"] == "404.html":
            continue
        name = slug(meta["path"])
        card(meta).save(out / f"{name}.png", optimize=True)
        written.append(name)
    print("wrote og:", ", ".join(written), file=sys.stderr)


if __name__ == "__main__":
    main()
