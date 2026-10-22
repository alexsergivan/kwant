#!/usr/bin/env python3
"""Builds the static kwant website in site/ from the page sources in site-src/.

The output is committed, so serving or deploying site/ needs no build step.
Run `python3 scripts/build_site.py` after editing anything in site-src/.

Each page source starts with a header block of `key: value` lines, then `---`, then HTML.
Keys: title, description, path (output path under site/), nav (docs nav id, docs pages only),
active (header link to mark current on non-docs pages).
Shortcodes in the HTML:
  {{root}}                                relative path back to site/
  {{shot:NAME|ALT|SIZES}}                 a zoomable responsive screenshot (desktop-* or phone-*)
  {{video:NAME|LABEL}}                    a muted looping demo clip with its poster
"""
import html
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "site-src"
OUT = ROOT / "site"
SITE_URL = "https://kwant.si"
GITHUB = "https://github.com/alexsergivan/kwant"

DOC_GROUPS = [
    ("Get started", [("quickstart", "Quickstart"), ("install", "Install & run")]),
    ("Using kwant", [
        ("spaces", "Spaces"),
        ("bots-and-teams", "Bots & teams"),
        ("accounts", "Accounts & profiles"),
        ("models", "Models & routing"),
        ("approvals", "Approvals & passwords"),
    ]),
    ("Works on its own", [
        ("routines", "Routines & bot actions"),
        ("memory-skills", "Memory, skills & search"),
        ("loose-ends", "Loose ends & letters"),
        ("parallel-universe", "Parallel Universe"),
    ]),
    ("Connect", [("mcp", "MCP tools"), ("browser", "Browser")]),
]
DOCS = [d for _, items in DOC_GROUPS for d in items]

LOGO = (
    '<svg viewBox="0 0 32 32" aria-hidden="true">'
    '<circle cx="16" cy="16" r="14" fill="#11151c" stroke="#3a4558" stroke-width="1.5"/>'
    '<path d="M16 5a11 11 0 0 1 10.4 7.4" fill="none" stroke="#5ec4b6" stroke-width="3" stroke-linecap="round"/>'
    '<path d="M24.6 22.6A11 11 0 0 1 16 27" fill="none" stroke="#ef7d62" stroke-width="3" stroke-linecap="round"/>'
    '<path d="M7.4 21A11 11 0 0 1 5.4 13" fill="none" stroke="#f0b44c" stroke-width="3" stroke-linecap="round"/>'
    '<path d="M16 16L11.5 10" stroke="#ece5d6" stroke-width="2" stroke-linecap="round"/>'
    '<circle cx="16" cy="16" r="2.4" fill="#f0b44c"/></svg>'
)
GH_ICON = (
    '<svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 '
    '5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52'
    '-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59'
    '.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82'
    '.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 '
    '.21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>'
)


def parse(path):
    text = path.read_text()
    head, body = text.split("\n---\n", 1)
    meta = {}
    for line in head.strip().splitlines():
        k, v = line.split(":", 1)
        meta[k.strip()] = v.strip()
    return meta, body


def shot(root, name, alt, sizes):
    phone = name.startswith("phone-")
    small, big, w, h = (390, 780, 390, 844) if phone else (1440, 2880, 1440, 900)
    base = f"{root}assets/media/{name}"
    sizes = sizes or "(max-width: 1024px) 100vw, 680px"
    a = html.escape(alt, quote=True)
    return (
        f'<a class="zoom" href="{base}-{big}.webp" data-caption="{a}">'
        f'<img src="{base}-{small}.webp" srcset="{base}-{small}.webp {small}w, {base}-{big}.webp {big}w" '
        f'sizes="{sizes}" width="{w}" height="{h}" loading="lazy" decoding="async" alt="{a}"></a>'
    )


def video(root, name, label, eager=False):
    base = f"{root}assets/media/{name}"
    lbl = html.escape(label, quote=True)
    return (
        f'<video data-autoplay controls muted playsinline loop preload="{"metadata" if eager else "none"}" '
        f'poster="{base}-poster.jpg" width="1280" height="800" aria-label="{lbl}">'
        f'<source src="{base}.mp4" type="video/mp4"></video>'
    )


def expand(body, root):
    body = body.replace("{{root}}", root)
    body = re.sub(r"\{\{shot:([^|}]+)\|([^|}]+)(?:\|([^}]*))?\}\}", lambda m: shot(root, m[1], m[2], m[3]), body)
    body = re.sub(
        r"\{\{video(!?):([^|}]+)\|([^}]+)\}\}", lambda m: video(root, m[2], m[3], eager=m[1] == "!"), body
    )
    return body


def header(root, active):
    home = root or "./"
    def cur(name):
        return ' aria-current="page"' if active == name else ""

    return f"""<a class="skip" href="#main">Skip to content</a>
<header class="site-header">
  <div class="wrap">
    <a class="brand" href="{home}" aria-label="kwant home">{LOGO}<span>kwant</span></a>
    <nav class="nav" aria-label="Main">
      <a class="hide-sm" href="{root}features.html"{cur("features")}>Features</a>
      <a class="hide-sm" href="{home}#how">How it works</a>
      <a class="hide-sm" href="{home}#faq">FAQ</a>
      <a href="{root}docs/quickstart.html"{cur("docs")}>Docs</a>
      <a class="btn btn-ghost" href="{GITHUB}" rel="noopener">{GH_ICON}<span>GitHub</span></a>
    </nav>
  </div>
</header>"""


def footer(root):
    home = root or "./"
    return f"""<footer class="site-footer">
  <div class="wrap">
    <div>
      <a class="brand" href="{home}" aria-label="kwant home">{LOGO}<span>kwant</span></a>
      <p class="foot-log"><span>07:15</span> End of shift.</p>
      <p>A self-hosted team of AI bots in one binary. Screens and clips on this site come from a real kwant build running sample data.</p>
    </div>
    <nav aria-label="Footer">
      <a href="{root}docs/quickstart.html">Quickstart</a>
      <a href="{root}docs/install.html">Install &amp; run</a>
      <a href="{root}features.html">All features</a>
      <a href="{root}docs/spaces.html">Spaces</a>
      <a href="{root}docs/accounts.html">Accounts</a>
      <a href="{root}docs/approvals.html">Approvals</a>
      <a href="{GITHUB}" rel="noopener">GitHub</a>
    </nav>
  </div>
</footer>
<script src="{root}assets/js/site.js" defer></script>"""


def docs_shell(root, current, body):
    here = ' aria-current="page"'
    items = "\n".join(
        f'    <div class="docs-group"><h3>{html.escape(group)}</h3><ul>'
        + "".join(
            f'<li><a href="{root}docs/{slug}.html"{here if slug == current else ""}>{html.escape(label)}</a></li>'
            for slug, label in links
        )
        + "</ul></div>"
        for group, links in DOC_GROUPS
    )
    idx = [s for s, _ in DOCS].index(current)
    prev_link = next_link = ""
    if idx > 0:
        s, l = DOCS[idx - 1]
        prev_link = f'<a class="prev" href="{root}docs/{s}.html"><small>Previous</small>{html.escape(l)}</a>'
    if idx < len(DOCS) - 1:
        s, l = DOCS[idx + 1]
        next_link = f'<a class="next" href="{root}docs/{s}.html"><small>Next</small>{html.escape(l)}</a>'
    return f"""<div class="wrap docs">
  <nav class="docs-nav" aria-label="Docs">
    <h2>Docs</h2>
{items}
  </nav>
  <article class="prose">
{body}
    <nav class="pager" aria-label="Docs pages">{prev_link}{next_link}</nav>
  </article>
</div>"""


def page(meta, body, root):
    title = meta["title"]
    full_title = title if title.startswith("kwant") else f"{title} · kwant"
    desc = html.escape(meta["description"], quote=True)
    robots = '<meta name="robots" content="noindex">\n' if meta["path"] == "404.html" else ""
    url = SITE_URL + "/" + meta["path"].replace("index.html", "").replace(".html", "")
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(full_title)}</title>
<meta name="description" content="{desc}">
<meta name="theme-color" content="#0b0e13">
<meta name="color-scheme" content="dark">
{robots}<link rel="canonical" href="{url}">
<meta property="og:type" content="website">
<meta property="og:site_name" content="kwant">
<meta property="og:title" content="{html.escape(full_title, quote=True)}">
<meta property="og:description" content="{desc}">
<meta property="og:url" content="{url}">
<meta property="og:image" content="{SITE_URL}/assets/og.png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="kwant: your team of AI bots works the night shift">
<meta name="twitter:card" content="summary_large_image">
<link rel="icon" href="{root}favicon.svg" type="image/svg+xml">
<link rel="icon" href="{root}favicon-32.png" sizes="32x32" type="image/png">
<link rel="apple-touch-icon" href="{root}apple-touch-icon.png">
<link rel="preload" href="{root}assets/fonts/fraunces-var.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="{root}assets/fonts/instrument-sans-var.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="{root}assets/css/site.css">
</head>
<body>
{header(root, "docs" if meta.get("nav") else meta.get("active", "home"))}
<main id="main">
{body}
</main>
{footer(root)}
</body>
</html>
"""


def main():
    written = []
    for src in sorted(SRC.glob("*.html")):
        meta, body = parse(src)
        out = OUT / meta["path"]
        # The 404 page is served for any missing path, so it needs root-absolute links.
        root = "/" if meta["path"] == "404.html" else "../" * meta["path"].count("/")
        body = expand(body, root)
        if meta.get("nav"):
            body = docs_shell(root, meta["nav"], body)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(page(meta, body, root))
        written.append(meta["path"])
    urls = [p for p in written if p != "404.html"]
    sitemap = ['<?xml version="1.0" encoding="UTF-8"?>', '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">']
    for p in urls:
        sitemap.append(f"  <url><loc>{SITE_URL}/{p.replace('index.html', '').replace('.html', '')}</loc></url>")
    sitemap.append("</urlset>")
    (OUT / "sitemap.xml").write_text("\n".join(sitemap) + "\n")
    (OUT / "robots.txt").write_text(f"User-agent: *\nAllow: /\n\nSitemap: {SITE_URL}/sitemap.xml\n")
    print("wrote", ", ".join(written), file=sys.stderr)


if __name__ == "__main__":
    main()
