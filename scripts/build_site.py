#!/usr/bin/env python3
"""Builds the static kwant website in site/ from the page sources in site-src/.

The output is committed, so serving or deploying site/ needs no build step.
Run `python3 scripts/build_site.py` after editing anything in site-src/.

Each page source starts with a header block of `key: value` lines, then `---`, then HTML.
Keys: title, description, path (output path under site/), nav (docs nav id, docs pages only),
active (header link to mark current on non-docs pages), faq (yes: FAQPage JSON-LD from the page's
<section class="faq">, each <h3> question and the <p> after it), and card, card_title, card_art for the
social card (see scripts/build_og.py).
Shortcodes in the HTML:
  {{root}}                                relative path back to site/
  {{shot:NAME|ALT|SIZES}}                 a zoomable responsive screenshot (desktop-* or phone-*)
  {{video:NAME|LABEL}}                    a muted looping demo clip with its poster
  {{install:ID}}                          the install command box with macOS/Linux and Docker tabs
  {{CONTACT_EMAIL}}                       the contact address, from OPERATOR below (marked placeholder while empty)
  {{avatar:ROLE}}                         a kwant app avatar (the same CRT-framed creature the app draws)

Every local asset URL in the pages and in site.css gets a `?v=<content hash>` query, so a deploy never pairs new HTML
with a browser's cached old CSS, JS, fonts or images.

Pages also get their social tags (Open Graph, Twitter), JSON-LD structured data and a sitemap with lastmod dates.
The build also writes llms.txt (https://llmstxt.org): a short summary of kwant for AI assistants, listing every page with
its title and description, so new pages show up there without extra work.
Each page's og:image is its own card from scripts/build_og.py (site/assets/og/), so run that first when titles or cards change.
lastmod and the docs' dateModified come from the last git commit that changed the page body (today if it's uncommitted).

It also writes the favicons (favicon.svg, favicon-32.png, apple-touch-icon.png, favicon.ico) from LOGO_RECTS,
so the tab icon is always the header logo.
"""
import datetime
import hashlib
import html
import json
import pathlib
import posixpath
import re
import struct
import subprocess
import sys
import zlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "site-src"
OUT = ROOT / "site"
SITE_URL = "https://kwant.si"
GITHUB = "https://github.com/alexsergivan/kwant"
# How to reach whoever runs kwant.si, for the privacy policy and terms ({{CONTACT_EMAIL}} in the page sources).
# An empty value leaves the marked placeholder in the page, and check_site.py fails until it is filled in.
OPERATOR = {
    "CONTACT_EMAIL": "contact@kwant.si",
}

DOC_GROUPS = [
    ("Get started", [("quickstart", "Quickstart"), ("install", "Install & run")]),
    ("Using kwant", [
        ("spaces", "Spaces"),
        ("bots-and-teams", "Bots & teams"),
        ("accounts", "Accounts & profiles"),
        ("models", "Models & routing"),
        ("approvals", "Approvals & passwords"),
        ("flight-school", "Flight School"),
    ]),
    ("Works on its own", [
        ("mission-board", "Mission board"),
        ("routines", "Routines & bot actions"),
        ("memory-skills", "Memory, skills & search"),
        ("loose-ends", "Loose ends & letters"),
        ("parallel-universe", "Parallel Universe"),
    ]),
    ("Connect", [("mcp", "MCP tools"), ("browser", "Browser")]),
]
DOCS = [d for _, items in DOC_GROUPS for d in items]

# The mark is the kwant app's antenna bot, the same 8-bit creature the bots use as avatars.
LOGO_RECTS = [
    (10, 4, 1, 1, "#1d1147"), (10, 1, 2, 2, "#ddd5ff"), (4, 5, 8, 1, "#fff"), (3, 6, 10, 1, "#fff"), (2, 7, 12, 7, "#fff"),
    (3, 14, 10, 1, "#fff"), (4, 15, 8, 1, "#ddd5ff"), (13, 7, 1, 7, "#ddd5ff"), (5, 8, 2, 3, "#1d1147"), (9, 8, 2, 3, "#1d1147"),
    (3, 11, 2, 1, "#b8a9fb"), (11, 11, 2, 1, "#b8a9fb"), (6, 12, 1, 1, "#1d1147"), (9, 12, 1, 1, "#1d1147"), (7, 13, 2, 1, "#1d1147"),
    (4, 16, 2, 1, "#ddd5ff"), (10, 16, 2, 1, "#ddd5ff"),
]
LOGO = (
    '<svg viewBox="0 0 16 17" width="38" height="38" shape-rendering="crispEdges" aria-hidden="true">'
    + "".join(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{c}"/>' for x, y, w, h, c in LOGO_RECTS)
    + "</svg>"
)
GH_ICON = (
    '<svg viewBox="0 0 16 16" width="17" height="17" fill="currentColor" aria-hidden="true"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 '
    '5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52'
    '-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59'
    '.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82'
    '.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 '
    '.21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>'
)


# ---- App avatars, ported from frontend/src/components/ModernAvatar.tsx (framed, 26-grid) ----
AV_FRAME, AV_PANEL, AV_SCAN, AV_INK, AV_BLUSH = "#4a3f8a", "#15112e", "#0d0a1f", "#1b1033", "#ff6b81"
AV_PALETTES = {
    "yellow": ("#fdc934", "#d98a1f", "#fff0a0", "#5ce1ff"),
    "green": ("#3ccf6e", "#1f8f4c", "#a8f5bf", "#c6ff6b"),
    "cyan": ("#5ce1ff", "#2a92c9", "#d2f7ff", "#ff6b81"),
    "violet": ("#a98bff", "#6d4fd6", "#e1d6ff", "#ffd23f"),
    "blue": ("#6d9bff", "#3c5fcf", "#cddcff", "#ff7eb6"),
    "coral": ("#ff8a7a", "#cf5446", "#ffd6cf", "#6d4fd6"),
    "red": ("#ff6b6b", "#c43c4c", "#ffc9c9", "#5ce1ff"),
    "mint": ("#9be564", "#5e9e33", "#e2ffc7", "#ffe066"),
}
AV_ROLES = {
    "brand": ("yellow", "antenna"), "user": ("blue", "ears"), "architect": ("violet", "horns"),
    "tech_lead": ("cyan", "visor"), "devops": ("green", "spikes"), "designer": ("coral", "beret"),
    "qa": ("red", "goggles"), "research": ("mint", "bulb"),
}


def avatar_rects(role):
    pal, acc = AV_ROLES[role]
    body, shade, light, accent = AV_PALETTES[pal]
    head = {
        "antenna": [(15, 6, 1, 1, AV_INK), (15, 3, 2, 2, accent), (15, 3, 1, 1, "#ffffff")],
        "spikes": [(10, 5, 1, 2, body), (13, 4, 1, 3, body), (16, 5, 1, 2, body), (6, 10, 1, 1, shade), (6, 13, 1, 1, shade),
                   (19, 10, 1, 1, shade), (19, 13, 1, 1, shade), (13, 4, 1, 1, light)],
        "ears": [(8, 5, 2, 3, body), (16, 5, 2, 3, shade), (8, 6, 1, 1, AV_BLUSH), (17, 6, 1, 1, AV_BLUSH)],
        "horns": [(8, 5, 1, 2, accent), (9, 6, 1, 1, accent), (17, 5, 1, 2, accent), (16, 6, 1, 1, accent)],
        "visor": [(9, 5, 8, 1, accent), (8, 6, 1, 3, accent), (17, 6, 1, 3, accent), (6, 10, 2, 3, accent), (18, 10, 2, 3, accent)],
        "beret": [(9, 5, 8, 2, accent), (8, 6, 1, 1, accent), (13, 4, 1, 1, accent), (10, 5, 2, 1, "#9a7cff")],
        "bulb": [(12, 2, 2, 1, accent), (11, 3, 4, 2, accent), (12, 3, 1, 1, "#ffffff"), (12, 5, 2, 1, "#8a8aa8"), (12, 6, 2, 1, AV_INK)],
    }.get(acc, [])
    face = {
        "goggles": [(7, 8, 12, 1, AV_INK), (9, 7, 3, 2, accent), (14, 7, 3, 2, accent), (9, 7, 1, 1, "#ffffff"), (14, 7, 1, 1, "#ffffff")],
    }.get(acc, [])
    return [(7, 20, 12, 1, "#261d4a")] + head + [
        (9, 7, 8, 1, body), (8, 8, 10, 1, body), (7, 9, 12, 7, body), (8, 16, 10, 1, body), (9, 17, 8, 1, shade),
        (18, 9, 1, 7, shade), (17, 16, 1, 1, shade), (7, 15, 1, 1, shade), (8, 16, 1, 1, shade), (10, 8, 2, 1, light), (9, 9, 1, 1, light),
        (10, 10, 2, 3, AV_INK), (14, 10, 2, 3, AV_INK), (10, 10, 1, 1, "#f5f2ff"), (14, 10, 1, 1, "#f5f2ff"),
        (8, 13, 2, 1, AV_BLUSH), (16, 13, 2, 1, AV_BLUSH), (11, 14, 1, 1, AV_INK), (14, 14, 1, 1, AV_INK), (12, 15, 2, 1, AV_INK),
        (9, 18, 2, 1, AV_INK), (15, 18, 2, 1, AV_INK),
    ] + face


AV_FRAME_FILL = [(6, 2, 14, 22), (4, 3, 18, 20), (3, 4, 20, 18), (2, 6, 22, 14)]


def avatar(role):
    r = lambda x, y, w, h, c: f'<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{c}"/>'
    parts = [r(x - 1, y - 1, w + 2, h + 2, AV_FRAME) for x, y, w, h in AV_FRAME_FILL]
    parts += [r(x, y, w, h, AV_PANEL) for x, y, w, h in AV_FRAME_FILL]
    parts += [r(3.5, 4.5 + i, 19, 0.25, AV_SCAN) for i in range(16)]
    parts += [r(*a) for a in avatar_rects(role)]
    return f'<svg class="av" viewBox="0 0 26 26" width="34" height="34" shape-rendering="crispEdges" aria-hidden="true">{"".join(parts)}</svg>'


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
        f'poster="{base}-poster.webp" width="1280" height="800" aria-label="{lbl}">'
        f'<source src="{base}.mp4" type="video/mp4"></video>'
    )


INSTALL = [
    ("src", "macOS · Linux", "git clone https://github.com/alexsergivan/kwant && cd kwant && make run"),
    ("docker", "Docker", "git clone https://github.com/alexsergivan/kwant && cd kwant && docker compose up -d --build"),
]


NOTAB = ' tabindex="-1"'


def install(uid):
    tabs = "".join(
        f'<button type="button" role="tab" id="{uid}-t-{k}" aria-controls="{uid}-p-{k}" '
        f'aria-selected="{"true" if i == 0 else "false"}"{"" if i == 0 else NOTAB}>{html.escape(label)}</button>'
        for i, (k, label, _) in enumerate(INSTALL)
    )
    panels = "".join(
        f'<div class="cmd" role="tabpanel" id="{uid}-p-{k}" aria-labelledby="{uid}-t-{k}"{"" if i == 0 else " hidden"}>'
        f'<span class="p" aria-hidden="true">$</span><code>{html.escape(cmd)}</code>'
        f'<button type="button" class="copy-cmd" aria-label="Copy the {html.escape(label)} install command">copy</button></div>'
        for i, (k, label, cmd) in enumerate(INSTALL)
    )
    return (
        f'<div class="install" data-tabs><div class="install-top"><span class="install-k" aria-hidden="true">Install</span>'
        f'<div class="tabs" role="tablist" aria-label="Install with">{tabs}</div></div>{panels}</div>'
    )


def operator(key):
    value = OPERATOR[key].strip()
    if not value:
        return f'<span class="placeholder">{{{{{key}}}}}</span>'
    if key == "CONTACT_EMAIL":
        return f'<a href="mailto:{html.escape(value, quote=True)}">{html.escape(value)}</a>'
    return html.escape(value)


def expand(body, root):
    body = body.replace("{{root}}", root)
    body = re.sub(r"\{\{(" + "|".join(OPERATOR) + r")\}\}", lambda m: operator(m[1]), body)
    body = re.sub(r"\{\{install:([a-z0-9-]+)\}\}", lambda m: install(m[1]), body)
    body = re.sub(r"\{\{avatar:([a-z_]+)\}\}", lambda m: avatar(m[1]), body)
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
      <a href="{root}features.html"{cur("features")}>Features</a>
      <a href="{root}docs/quickstart.html"{cur("docs")}>Docs</a>
      <a class="btn btn-soft" href="{GITHUB}" rel="noopener">{GH_ICON}<span>GitHub</span></a>
    </nav>
  </div>
</header>"""


def footer(root):
    home = root or "./"
    return f"""<footer class="site-footer">
  <div class="wrap">
    <div class="foot-brand">
      <a class="brand" href="{home}" aria-label="kwant home">{LOGO}<span>kwant</span></a>
      <p>A self-hosted team of AI bots in one binary. Free and open source under AGPL-3.0.</p>
      <a class="foot-badge" href="https://www.opensourcealternatives.to/" rel="noopener"><img src="{root}assets/badges/opensourcealternatives.svg" alt="As seen on Open Source Alternatives" width="270" height="20" loading="lazy"></a>
    </div>
    <nav aria-label="Product">
      <h2>Product</h2>
      <a href="{root}features.html">Features</a>
      <a href="{root}docs/quickstart.html">Quickstart</a>
      <a href="{root}docs/install.html">Install &amp; run</a>
      <a href="{root}self-hosted-ai-agents.html">Self-hosted AI agents</a>
      <a href="{root}docs/mcp.html">MCP tools</a>
    </nav>
    <nav aria-label="Using kwant">
      <h2>Using kwant</h2>
      <a href="{root}docs/spaces.html">Spaces</a>
      <a href="{root}docs/accounts.html">Accounts</a>
      <a href="{root}docs/approvals.html">Approvals</a>
      <a href="{root}docs/routines.html">Routines</a>
    </nav>
    <nav aria-label="Project">
      <h2>Project</h2>
      <a href="{GITHUB}" rel="noopener">GitHub</a>
      <a href="{GITHUB}/issues" rel="noopener">Issues</a>
      <a href="{GITHUB}/blob/main/SECURITY.md" rel="noopener">Security</a>
    </nav>
    <nav aria-label="Legal">
      <h2>Legal</h2>
      <a href="{root}privacy.html">Privacy</a>
      <a href="{root}terms.html">Terms of use</a>
      <a href="{root}terms.html#license">License</a>
    </nav>
    <p class="foot-legal">kwant is not affiliated with or endorsed by Anthropic, OpenAI, xAI, Google or Apple. Their names and products are trademarks of their owners.</p>
  </div>
</footer>
<script src="{root}assets/js/site.js" defer></script>
<script src="{root}assets/js/pixel.js" defer></script>"""


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


def git_dates(src):
    """(first published, last modified) as YYYY-MM-DD from git. Modified is the last commit that changed the page body, so
    header-only edits (a card line) don't look like new content. Today when the body has uncommitted changes or git isn't there."""
    today = datetime.date.today().isoformat()
    git = lambda *a: subprocess.run(["git", *a], cwd=ROOT, capture_output=True, text=True, check=True).stdout
    body = lambda text: text.split("\n---\n", 1)[-1]
    try:
        log = git("log", "--follow", "--name-only", "--format=@%H %cs", "--", str(src.relative_to(ROOT))).split("@")[1:]
    except (OSError, subprocess.CalledProcessError):
        return today, today
    commits = [(head.split()[0], head.split()[1], head.split()[2]) for head in (entry.strip() for entry in log) if len(head.split()) >= 3]
    if not commits:
        return today, today
    current, modified = body(src.read_text()), today
    for sha, date, path in commits:
        if body(git("show", f"{sha}:{path}")) != current:
            break
        modified = date
    return commits[-1][1], modified


def page_url(path):
    return SITE_URL + "/" + path.replace("index.html", "").replace(".html", "")


def og_image(path):
    """The page's own social card, with a content hash so link previews refresh when it changes. 404 uses the home card."""
    name = "home" if path in ("index.html", "404.html") else path.removesuffix(".html").replace("/", "-")
    card = OUT / f"assets/og/{name}.png"
    if not card.exists():
        return f"{SITE_URL}/assets/og.png"
    return f"{SITE_URL}/assets/og/{name}.png?v={file_hash(card)}"


ORG_ID, SITE_ID, APP_ID = f"{SITE_URL}/#org", f"{SITE_URL}/#website", f"{SITE_URL}/#app"
LICENSES = [f"{GITHUB}/blob/main/LICENSE"]


def faq_items(body):
    """(question, answer) pairs from the page's <section class="faq">: each <h3> and the <p> right after it, as plain text,
    so the FAQPage JSON-LD always says exactly what the page shows."""
    section = re.search(r'<section class="faq"[^>]*>(.*?)</section>', body, re.S)
    if not section:
        sys.exit("faq: yes, but the page has no <section class=\"faq\">")
    text = lambda h: html.unescape(re.sub(r"<[^>]+>", "", h)).strip()
    return [(text(q), text(a)) for q, a in re.findall(r"<h3[^>]*>(.*?)</h3>\s*<p>(.*?)</p>", section[1], re.S)]


def structured_data(meta, url, title, image, dates, body=""):
    """JSON-LD for the page: the app, site and project on the home page, an article with breadcrumbs on docs pages."""
    path = meta["path"]
    org = {"@type": "Organization", "@id": ORG_ID, "name": "kwant", "alternateName": "kwnt", "url": SITE_URL + "/",
           "logo": f"{SITE_URL}/apple-touch-icon.png", "sameAs": [GITHUB]}
    crumbs = lambda items: {"@type": "BreadcrumbList", "itemListElement": [
        {"@type": "ListItem", "position": i + 1, "name": n, "item": u} for i, (n, u) in enumerate(items)]}
    if path == "index.html":
        graph = [
            org,
            {"@type": "WebSite", "@id": SITE_ID, "name": "kwant", "url": SITE_URL + "/", "inLanguage": "en", "publisher": {"@id": ORG_ID}},
            {"@type": "SoftwareApplication", "@id": APP_ID, "name": "kwant", "alternateName": "kwnt", "url": SITE_URL + "/", "description": meta["description"],
             "applicationCategory": "DeveloperApplication", "operatingSystem": "macOS, Linux", "image": image,
             "screenshot": f"{SITE_URL}/assets/media/desktop-team-discussion-1440.webp",
             "offers": {"@type": "Offer", "price": "0", "priceCurrency": "USD"},
             "license": LICENSES, "isAccessibleForFree": True, "downloadUrl": GITHUB, "sameAs": [GITHUB],
             "author": {"@id": ORG_ID}, "publisher": {"@id": ORG_ID}},
            {"@type": "SoftwareSourceCode", "@id": f"{SITE_URL}/#code", "name": "kwant", "codeRepository": GITHUB,
             "programmingLanguage": ["Rust", "TypeScript"], "license": LICENSES, "targetProduct": {"@id": APP_ID}},
        ]
    elif meta.get("nav"):
        label = dict(DOCS)[meta["nav"]]
        graph = [
            {"@type": "TechArticle", "@id": url + "#article", "headline": title, "description": meta["description"], "url": url,
             "mainEntityOfPage": url, "image": image, "inLanguage": "en", "datePublished": dates[0], "dateModified": dates[1],
             "author": {"@id": ORG_ID}, "publisher": {"@id": ORG_ID}, "about": {"@id": APP_ID}, "isPartOf": {"@id": SITE_ID}},
            crumbs([("kwant", SITE_URL + "/"), ("Docs", f"{SITE_URL}/docs/quickstart"), (label, url)]),
            org,
        ]
    elif path == "404.html":
        return ""
    else:
        graph = [
            {"@type": "WebPage", "@id": url, "name": title, "description": meta["description"], "url": url, "image": image,
             "inLanguage": "en", "dateModified": dates[1], "about": {"@id": APP_ID}, "isPartOf": {"@id": SITE_ID}},
            crumbs([("kwant", SITE_URL + "/"), (title.split(":")[0], url)]),
        ]
    if meta.get("faq") == "yes":
        graph.append({"@type": "FAQPage", "@id": url + "#faq", "url": url, "mainEntity": [
            {"@type": "Question", "name": q, "acceptedAnswer": {"@type": "Answer", "text": a}} for q, a in faq_items(body)]})
    data = json.dumps({"@context": "https://schema.org", "@graph": graph}, ensure_ascii=False, separators=(",", ":"))
    # JSON-LD is a data block, not a script, so the CSP's script-src doesn't apply to it. It goes at the end of <body>
    # so the head and the first screen of text still fit in the first TCP round trip.
    return f'<script type="application/ld+json">{data.replace("</", "<" + chr(92) + "/")}</script>\n'


def page(meta, body, root, dates):
    title = meta["title"]
    full_title = title if title.startswith("kwant") else f"{title} · kwant"
    desc = html.escape(meta["description"], quote=True)
    is404 = meta["path"] == "404.html"
    robots = '<meta name="robots" content="noindex">\n' if is404 else ""
    url = page_url(meta["path"])
    image = og_image(meta["path"])
    card_name = "kwant" if is404 else meta.get("card_title") or title.split(":")[0]
    img_alt = html.escape(f"{card_name}: {meta.get('card', meta['description'])}", quote=True)
    ftitle = html.escape(full_title, quote=True)
    article = (
        f'<meta property="article:published_time" content="{dates[0]}">\n<meta property="article:modified_time" content="{dates[1]}">\n'
        if meta.get("nav") else ""
    )
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(full_title)}</title>
<meta name="description" content="{desc}">
<meta name="theme-color" content="#f4f1ff">
<meta name="color-scheme" content="light">
{robots}<link rel="canonical" href="{url}">
<meta property="og:type" content="{"article" if meta.get("nav") else "website"}">
<meta property="og:site_name" content="kwant">
<meta property="og:locale" content="en_US">
<meta property="og:title" content="{ftitle}">
<meta property="og:description" content="{desc}">
<meta property="og:url" content="{url}">
<meta property="og:image" content="{image}">
<meta property="og:image:type" content="image/png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="{img_alt}">
{article}<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="{ftitle}">
<meta name="twitter:description" content="{desc}">
<meta name="twitter:image" content="{image}">
<meta name="twitter:image:alt" content="{img_alt}">
<link rel="icon" href="{root}favicon.svg" type="image/svg+xml">
<link rel="icon" href="{root}favicon-32.png" sizes="32x32" type="image/png">
<link rel="apple-touch-icon" href="{root}apple-touch-icon.png">
<link rel="preload" href="{root}assets/fonts/pixelify-sans.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="{root}assets/fonts/inter-var.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="{root}assets/css/site.css">
</head>
<body class="page-{meta.get("active", "docs" if meta.get("nav") else "home") if not is404 else "404"}">
{header(root, "docs" if meta.get("nav") else meta.get("active", "home"))}
<main id="main">
{body}
</main>
{footer(root)}
{structured_data(meta, url, title, image, dates, body)}</body>
</html>
"""


# ---- Cache busting ----
_hashes = {}


def file_hash(path):
    if path not in _hashes:
        _hashes[path] = hashlib.sha256(path.read_bytes()).hexdigest()[:10]
    return _hashes[path]


def version_css():
    """Adds ?v=<hash> to every url() in site.css, so a changed font or image gets a new URL. Re-running is a no-op."""
    css = OUT / "assets/css/site.css"
    text = css.read_text()
    new = re.sub(
        r'url\("(\.\./[^"?#]+)(?:\?v=[0-9a-f]+)?"\)',
        lambda m: f'url("{m[1]}?v={file_hash((css.parent / m[1]).resolve())}")',
        text,
    )
    if new != text:
        css.write_text(new)


ASSET_URL = re.compile(
    r'(?<=["\s,])((?:\.\./)*|/)?((?:assets/[\w./-]+\.(?:css|js|woff2|png|webp|jpg|svg|mp4))|favicon\.svg|favicon-32\.png|apple-touch-icon\.png)(?=["\s,])'
)


def version_urls(page_html):
    """Appends ?v=<hash> to the page's relative asset URLs (src, href, srcset, poster). Absolute URLs such as og:image stay as they are."""
    return ASSET_URL.sub(lambda m: f"{m[1] or ''}{m[2]}?v={file_hash(OUT / m[2])}", page_html)


# ---- Favicons, drawn from LOGO_RECTS ----
# The logo's pixels span x 2..13 and y 1..16, so a 16x16 box from y=1 holds it exactly: crisp at 16, 32 and 48 px.
FAV_BG = "#4024a8"


def favicon_svg():
    rects = "".join(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{c}"/>' for x, y, w, h, c in LOGO_RECTS)
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 1 16 16" shape-rendering="crispEdges">'
        f'<rect x="0" y="1" width="16" height="16" rx="3" fill="{FAV_BG}"/>{rects}</svg>\n'
    )


def hexrgb(c):
    c = c.lstrip("#")
    if len(c) == 3:
        c = "".join(ch * 2 for ch in c)
    return tuple(int(c[i:i + 2], 16) for i in (0, 2, 4))


def favicon_pixels(size, scale, corner):
    """RGBA rows: the violet square (corners cut by `corner` px) with the logo scaled by an integer `scale`."""
    bg = hexrgb(FAV_BG) + (255,)
    px = [[bg] * size for _ in range(size)]
    for y in range(size):
        for x in range(size):
            dx = max(corner - x, x - (size - 1 - corner), 0)
            dy = max(corner - y, y - (size - 1 - corner), 0)
            if corner and dx and dy and dx * dx + dy * dy > corner * corner:
                px[y][x] = (0, 0, 0, 0)
    ox, oy = (size - 12 * scale) // 2 - 2 * scale, (size - 16 * scale) // 2 - scale
    for x, y, w, h, c in LOGO_RECTS:
        col = hexrgb(c) + (255,)
        for yy in range(oy + y * scale, oy + (y + h) * scale):
            for xx in range(ox + x * scale, ox + (x + w) * scale):
                px[yy][xx] = col
    return px


def png_bytes(px):
    h, w = len(px), len(px[0])
    raw = b"".join(b"\x00" + bytes(v for p in row for v in p) for row in px)
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def ico_bytes(pngs):
    head = struct.pack("<HHH", 0, 1, len(pngs))
    offset = 6 + 16 * len(pngs)
    entries = data = b""
    for size, png in pngs:
        entries += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(png), offset)
        offset += len(png)
        data += png
    return head + entries + data


def write_favicons():
    (OUT / "favicon.svg").write_text(favicon_svg())
    p16, p32 = png_bytes(favicon_pixels(16, 1, 0)), png_bytes(favicon_pixels(32, 2, 4))
    (OUT / "favicon-32.png").write_bytes(p32)
    (OUT / "favicon.ico").write_bytes(ico_bytes([(16, p16), (32, p32)]))
    # iOS rounds the corners itself, so the touch icon is a full square with room around the logo.
    (OUT / "apple-touch-icon.png").write_bytes(png_bytes(favicon_pixels(180, 8, 0)))


LLMS_INTRO = """# kwant

> kwant is an open-source, self-hosted team of AI agents in one Rust binary (`kwnt`). Bots share chats, hand work to each
> other with @mentions and get it done on their own, on a schedule or while you're away. It runs on a laptop, a VPS or
> Docker, with the Claude, Codex or Grok CLI logins you already have, API keys, local Ollama or any OpenAI-compatible server.

Key facts:

- License: AGPL-3.0, with commercial licenses available (contact@kwant.si).
- Source: https://github.com/alexsergivan/kwant
- Mac app: https://github.com/alexsergivan/kwant/releases/latest/download/kwant_universal.dmg
- The program you install and run is called `kwnt` (`kwnt serve`).
- No telemetry: kwant sends nothing to its developers.
- Optional Mission board: a task counts as Verified only when a check passes, and the bot that handed out work reviews
  the result. No token limits by default; hand-off caps (4 per bot, 16 per request, with Continue) stop loops.
- Imports bots from Claude Code subagents, SKILL.md files, character cards, JSON exports and Grok Bot exports.
- Not to be confused with the Kwant quantum-transport Python package (kwant-project.org) or other products named Kwant; this kwant is an AI agent app.
"""


def llms_txt(pages):
    """llms.txt: the intro above, then every page as a markdown link with its description, docs in nav order."""
    line = lambda path: f"- [{pages[path]['title']}]({page_url(path)}): {pages[path]['description']}"
    docs = [f"docs/{d}.html" for d, _ in DOCS if f"docs/{d}.html" in pages]
    alternatives = sorted(p for p in pages if p.startswith("alternatives/"))
    main = sorted((p for p in pages if p not in docs + alternatives and p not in ("privacy.html", "terms.html", "404.html")), key=lambda p: p != "index.html")
    legal = [p for p in ("privacy.html", "terms.html") if p in pages]
    out = [LLMS_INTRO, "## Site", *map(line, main), "", "## Docs", *map(line, docs), ""]
    if alternatives:
        out += ["## Alternatives", *map(line, alternatives), ""]
    out += ["## Optional", *map(line, legal)]
    return "\n".join(out) + "\n"


def clean_links(page_html, path):
    """Points every internal page link at its clean, root-relative URL (/docs/install, not ../docs/install.html), the form
    Pages serves without a 308 redirect and the sitemap and canonicals use."""
    def clean(m):
        target, frag = m[2], m[3] or ""
        if not target.startswith("/"):
            target = posixpath.normpath(posixpath.join(posixpath.dirname("/" + path), target))
        target = re.sub(r"(^|/)index\.html$", r"\1", target).removesuffix(".html")
        return f'{m[1]}{target}{frag}"'
    return re.sub(r'(<a\b[^>]*?\bhref=")((?![a-z]+:|#)[^"#?]*\.html)(#[^"]*)?"', clean, page_html)


def main():
    write_favicons()
    version_css()
    written, lastmod, metas = [], {}, {}
    for src in sorted(SRC.glob("*.html")):
        meta, body = parse(src)
        dates = git_dates(src)
        lastmod[meta["path"]] = dates[1]
        metas[meta["path"]] = meta
        out = OUT / meta["path"]
        # The 404 page is served for any missing path, so it needs root-absolute links.
        root = "/" if meta["path"] == "404.html" else "../" * meta["path"].count("/")
        body = expand(body, root)
        if meta.get("nav"):
            body = docs_shell(root, meta["nav"], body)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(clean_links(version_urls(page(meta, body, root, dates)), meta["path"]))
        written.append(meta["path"])
    urls = [p for p in written if p != "404.html"]
    sitemap = ['<?xml version="1.0" encoding="UTF-8"?>', '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">']
    for p in urls:
        sitemap.append(f"  <url><loc>{page_url(p)}</loc><lastmod>{lastmod[p]}</lastmod></url>")
    sitemap.append("</urlset>")
    (OUT / "sitemap.xml").write_text("\n".join(sitemap) + "\n")
    (OUT / "llms.txt").write_text(llms_txt(metas))
    (OUT / "robots.txt").write_text(f"User-agent: *\nAllow: /\n\nSitemap: {SITE_URL}/sitemap.xml\n")
    print("wrote", ", ".join(written), file=sys.stderr)


if __name__ == "__main__":
    main()
