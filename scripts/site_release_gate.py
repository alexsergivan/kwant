#!/usr/bin/env python3
"""kwant.si and the GitHub README must show the newest release before that release counts as done.

Reads the newest `## [Mac X.Y.Z]` heading in CHANGELOG.md and checks site-src/ and README.md:

1. features.html has a "New in" section for it: `id="new-<id>"`, where <id> is the version without dots or a
   trailing .0 (0.7.0 -> new-07, 0.7.1 -> new-071, 0.6.11 -> new-0611), or any section whose `data-covers` lists
   the version (a small fix release folded into a bigger section).
2. index.html's "Added in" strip links to that section.
3. Every guide the CHANGELOG entry names (`docs/NAME.md`) has a page, site-src/docs-NAME.html, or an entry in
   GUIDE_PAGES saying where the site covers it.
4. Every screenshot, video and media file that site-src names exists in site/assets/media.
5. README.md has a `## New in X.Y` section for the release's minor version, and that section names the release
   (0.7.2 as "(0.7.2)" on a bullet, say). A small fix release with no bullet of its own can be named in an HTML
   comment in the section: `<!-- covers: 0.7.3 -->`.

Usage: python3 scripts/site_release_gate.py [--warn]
Exit 0 when kwant.si and the README are current, else 1 with one line per gap. --warn prints the gaps and exits 0:
check_site.py uses it, because the release PR bumps CHANGELOG.md before the site PR lands. The release script uses
the strict form, so a release isn't done until the site and the README are.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "site-src"
MEDIA = ROOT / "site" / "assets" / "media"

# Guides that have no page of their own on kwant.si: name -> the page that covers them.
GUIDE_PAGES = {
    "payments": "flight-school.html#payments",
    "saved-passwords": "approvals.html",
}

HEAD = re.compile(r"^## \[Mac (\d+\.\d+\.\d+)\] - \d{4}-\d{2}-\d{2}\s*$", re.M)
README_NEW = re.compile(r"^## New in (\d+)\.(\d+)\s*$", re.M)


def section_id(version):
    """0.7.0 -> 07, 0.7.1 -> 071, 0.6.11 -> 0611, 1.0.0 -> 10."""
    major, minor, patch = version.split(".")
    return f"{major}{minor}" + ("" if patch == "0" else patch)


def newest(changelog):
    """(version, entry text) of the first release heading."""
    m = HEAD.search(changelog)
    if not m:
        raise SystemExit("CHANGELOG.md has no '## [Mac X.Y.Z] - YYYY-MM-DD' release headings")
    nxt = re.search(r"^## \[", changelog[m.end():], re.M)
    return m[1], changelog[m.end(): m.end() + nxt.start()] if nxt else changelog[m.end():]


def readme_gap(version, readme):
    """None when README.md's "## New in X.Y" section names `version`, else the gap. The section ends at the next
    `## ` heading or `---` rule."""
    major, minor, _ = version.split(".")
    for m in README_NEW.finditer(readme):
        if (m[1], m[2]) != (major, minor):
            continue
        end = re.search(r"^(?:## |---\s*$)", readme[m.end():], re.M)
        body = readme[m.end(): m.end() + end.start()] if end else readme[m.end():]
        if re.search(rf"(?<![0-9.]){re.escape(version)}(?![0-9])", body):
            return None
        return (f"README.md: the 'New in {major}.{minor}' section doesn't name {version} (add a bullet tagged "
                f"({version}), or <!-- covers: {version} --> for a fix release)")
    return (f"README.md: no '## New in {major}.{minor}' section for {version} (replace the old New in section "
            f"with one for {major}.{minor}, its bullets tagged by version)")


def media_files(name):
    if name.startswith("phone-"):
        return [f"{name}-390.webp", f"{name}-780.webp"]
    return [f"{name}-1440.webp", f"{name}-2880.webp"]


def gaps(root=ROOT):
    src, media = root / "site-src", root / "site" / "assets" / "media"
    version, entry = newest((root / "CHANGELOG.md").read_text())
    out = []
    features = (src / "features.html").read_text()
    sid = None
    for m in re.finditer(r"<section\b([^>]*)>", features):
        attrs = m[1]
        sec = re.search(r'\bid="(new-[0-9]+)"', attrs)
        covers = re.search(r'\bdata-covers="([^"]*)"', attrs)
        if sec and (sec[1] == f"new-{section_id(version)}" or (covers and version in covers[1].split())):
            sid = sec[1]
            break
    if not sid:
        out.append(f"features.html: no 'New in {version}' section (id=\"new-{section_id(version)}\", "
                   f"or data-covers=\"{version}\" on another New in section)")
    elif f'features.html#{sid}"' not in (src / "index.html").read_text():
        out.append(f"index.html: the Added in strip doesn't link to features.html#{sid}")
    for guide in sorted(set(re.findall(r"docs/([a-z0-9-]+)\.md", entry))):
        if not (src / f"docs-{guide}.html").exists() and guide not in GUIDE_PAGES:
            out.append(f"docs/{guide}.md is named in the {version} entry but kwant.si has no docs-{guide}.html "
                       f"(write one, or add it to GUIDE_PAGES in scripts/site_release_gate.py)")
    missing = {}
    for page in sorted(src.rglob("*.html")):
        text = page.read_text()
        need = []
        for name in re.findall(r"\{\{shot:([^|}]+)\|", text):
            need += media_files(name)
        for name in re.findall(r"\{\{video:([^|}]+)\|", text):
            need += [f"{name}.mp4", f"{name}-poster.webp"]
        need += re.findall(r'assets/media/([A-Za-z0-9._-]+\.(?:webp|png|jpg|mp4|gif|svg))', text)
        for f in set(need):
            if not (media / f).exists():
                missing.setdefault(f, []).append(page.name)
    for f, pages in sorted(missing.items()):
        out.append(f"site/assets/media/{f} is missing (used in {', '.join(pages)})")
    readme = root / "README.md"
    gap = readme_gap(version, readme.read_text()) if readme.exists() else "README.md is missing"
    if gap:
        out.append(gap)
    return out


def main():
    warn = "--warn" in sys.argv[1:]
    version, _ = newest((ROOT / "CHANGELOG.md").read_text())
    found = gaps()
    for g in found:
        print(("WARN " if warn else "GAP ") + g)
    if not found:
        print(f"kwant.si and README.md cover {version}")
    sys.exit(0 if warn or not found else 1)


if __name__ == "__main__":
    main()
