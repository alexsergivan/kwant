#!/usr/bin/env python3
"""Regenerates src/mcp/composio_popular.json: Composio's most popular apps (slug, name, logo,
rank), read from the public https://composio.dev/toolkits page. No key is needed.

kwant shows this list in Find a tool before Composio is added, and with a Composio Connect
(`ck_`) key, which can't list Composio's catalogue. Run it at release:

    python3 scripts/composio_popular_apps.py            # fetch and write
    python3 scripts/composio_popular_apps.py page.html  # from a saved copy of the page
"""

import json
import re
import sys
import urllib.request
from pathlib import Path

URL = "https://composio.dev/toolkits"
OUT = Path(__file__).resolve().parent.parent / "src" / "mcp" / "composio_popular.json"
SLUG = re.compile(r"^[a-z0-9_-]{1,64}$")
MAX_APPS = 120


def page_text(argv):
    if len(argv) > 1:
        return Path(argv[1]).read_text(encoding="utf-8", errors="replace")
    req = urllib.request.Request(URL, headers={"User-Agent": "Mozilla/5.0 (kwant release script)"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return resp.read().decode("utf-8", errors="replace")


def flight_data(html):
    """The page's Next.js data: every `self.__next_f.push([1,"…"])` string, decoded and joined."""
    parts = []
    for m in re.finditer(r'self\.__next_f\.push\(\[1,("(?:[^"\\]|\\.)*")\]\)', html):
        parts.append(json.loads(m.group(1)))
    return "".join(parts)


def toolkits(data):
    start = data.find('"toolkits":[')
    if start < 0:
        raise SystemExit("no toolkits list on the page: has composio.dev/toolkits changed?")
    items, _ = json.JSONDecoder().raw_decode(data, start + len('"toolkits":'))
    apps, seen = [], set()
    for t in sorted(items, key=lambda t: t.get("rank", 1 << 30)):
        slug, name, logo = t.get("slug"), t.get("name"), t.get("logo")
        if not isinstance(slug, str) or not SLUG.match(slug) or slug in seen or not isinstance(name, str):
            continue
        seen.add(slug)
        app = {"slug": slug, "name": name.strip()[:80], "rank": len(apps)}
        if isinstance(logo, str) and logo.startswith("https://") and len(logo) <= 300:
            app["logo"] = logo
        apps.append(app)
        if len(apps) == MAX_APPS:
            break
    if len(apps) < 50:
        raise SystemExit(f"only {len(apps)} apps found; not writing")
    return apps


def main(argv):
    apps = toolkits(flight_data(page_text(argv)))
    lines = ",\n".join(json.dumps(a, ensure_ascii=False, separators=(",", ":")) for a in apps)
    OUT.write_text("[\n" + lines + "\n]\n", encoding="utf-8")
    print(f"wrote {len(apps)} apps to {OUT}")


if __name__ == "__main__":
    main(sys.argv)
