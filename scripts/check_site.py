#!/usr/bin/env python3
"""Checks the built site/ for the SEO and accessibility basics. Exits 1 if anything fails.

  - every internal link, src, srcset, poster and asset resolves to a file in site/ (or a _redirects rule)
  - one <h1> per page, and headings never skip a level going down (h2 -> h4)
  - every <img> has an alt attribute
  - title, description, canonical, og:* and twitter:* tags are present; og:image points at an existing 1200x630 card
  - JSON-LD parses and every node has an @type; the sitemap lists every indexable page with a lastmod
  - 404.html is noindex and not in the sitemap
  - blog: every published post in site-src/blog is built with BlogPosting and breadcrumb JSON-LD, article meta, a 15-70
    character title and 50-160 character description, is linked from blog/index.html and listed in the sitemap and in
    blog/feed.xml (which must parse as RSS); draft posts are nowhere

Run `python3 scripts/check_site.py` after build_site.py (`--drafts` after `build_site.py --drafts`, to check drafts too).
"""
import datetime
import email.utils
import html.parser
import json
import pathlib
import re
import struct
import sys
import urllib.parse
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "site"
SITE_URL = "https://kwant.si"
REQUIRED_META = [
    "description", "og:type", "og:title", "og:description", "og:url", "og:image", "og:image:alt", "og:image:width",
    "og:image:height", "og:locale", "twitter:card", "twitter:title", "twitter:description", "twitter:image", "twitter:image:alt",
]


class Page(html.parser.HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.refs, self.headings, self.imgs_no_alt, self.meta, self.ids = [], [], 0, {}, set()
        self.canonical, self.title, self.jsonld, self._in = None, "", [], None

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if "id" in a:
            self.ids.add(a["id"])
        if re.fullmatch(r"h[1-6]", tag):
            self.headings.append(int(tag[1]))
        if tag == "img" and "alt" not in a:
            self.imgs_no_alt += 1
        if tag == "meta":
            key = a.get("property") or a.get("name")
            if key:
                self.meta[key] = a.get("content", "")
        if tag == "link" and a.get("rel") == "canonical":
            self.canonical = a.get("href")
        for attr in ("href", "src", "poster"):
            if a.get(attr) and not (tag == "link" and a.get("rel") == "canonical"):
                self.refs.append(a[attr])
        if a.get("srcset"):
            self.refs += [part.split()[0] for part in a["srcset"].split(",")]
        if tag == "title":
            self._in = "title"
        if tag == "script" and a.get("type") == "application/ld+json":
            self._in = "ld"
            self.jsonld.append("")

    def handle_endtag(self, tag):
        self._in = None

    def handle_data(self, data):
        if self._in == "title":
            self.title += data
        elif self._in == "ld":
            self.jsonld[-1] += data


def redirects():
    path = OUT / "_redirects"
    return {line.split()[0] for line in path.read_text().splitlines() if line.strip()} if path.exists() else set()


def resolve(page_path, ref):
    """The file a local reference points at, or None for external and fragment-only links."""
    url = urllib.parse.urlsplit(ref)
    if url.scheme in ("http", "https"):
        if not ref.startswith(SITE_URL):
            return None
        url = urllib.parse.urlsplit(ref[len(SITE_URL):] or "/")
    elif url.scheme:  # mailto:, data:
        return None
    if not url.path:
        return None
    base = OUT if url.path.startswith("/") else page_path.parent
    target = (base / url.path.lstrip("/")).resolve()
    return target, url.path


def exists(target, raw):
    if raw in redirects() or raw.rstrip("/") in redirects():
        return True
    if target.is_dir():
        return (target / "index.html").exists()
    return target.exists() or target.with_suffix(".html").exists()


def png_size(path):
    with open(path, "rb") as f:
        head = f.read(24)
    return struct.unpack(">II", head[16:24])


def iso_date(value):
    try:
        datetime.date.fromisoformat(value)
        return True
    except (TypeError, ValueError):
        return False


def check_blog(errors, sitemap_urls, drafts):
    """The blog: each published post has its page, BlogPosting and breadcrumb JSON-LD, article meta, a sensible title and
    description, and is linked from the index, listed in the sitemap and in the feed; drafts appear nowhere."""
    src = ROOT / "site-src/blog"
    sources = sorted(src.glob("*.md")) if src.exists() else []
    draft = lambda f: re.search(r"^draft:\s*yes\s*$", f.read_text().split("\n---\n", 1)[0], re.M)
    published = [f.stem for f in sources if drafts or not draft(f)]
    for f in sources:
        if draft(f) and not drafts:
            rel = f"blog/{f.stem}.html"
            if (OUT / rel).exists():
                errors.append(f"{rel}: draft post was built")
            if f"{SITE_URL}/blog/{f.stem}" in sitemap_urls:
                errors.append(f"{rel}: draft post is in sitemap.xml")
            feed = OUT / "blog/feed.xml"
            if feed.exists() and f"{SITE_URL}/blog/{f.stem}<" in feed.read_text():
                errors.append(f"{rel}: draft post is in the feed")
    built = sorted(p.stem for p in (OUT / "blog").glob("*.html") if p.name != "index.html") if (OUT / "blog").exists() else []
    for extra in sorted(set(built) - set(published)):
        errors.append(f"blog/{extra}.html: no published source in site-src/blog")
    if not published:
        for name in ("blog/index.html", "blog/feed.xml"):
            if (OUT / name).exists():
                errors.append(f"{name}: exists with no published posts")
        return
    index = OUT / "blog/index.html"
    if not index.exists():
        errors.append("blog/index.html: missing, but there are published posts")
        return
    index_html = index.read_text()
    for slug in published:
        rel = f"blog/{slug}.html"
        err = lambda msg: errors.append(f"{rel}: {msg}")
        page = OUT / rel
        if not page.exists():
            err("published post was not built")
            continue
        p = Page()
        p.feed(page.read_text())
        title, desc = p.title.strip(), p.meta.get("description", "")
        if not 15 <= len(title) <= 70:
            err(f"<title> is {len(title)} characters (keep it 15-70 so search results show all of it)")
        if not 50 <= len(desc) <= 160:
            err(f"description is {len(desc)} characters (keep it 50-160)")
        if p.meta.get("og:type") != "article":
            err("og:type is not article")
        for key in ("article:published_time", "article:modified_time"):
            if not iso_date(p.meta.get(key)):
                err(f"missing or bad {key}")
        nodes = [n for block in p.jsonld for n in json.loads(block).get("@graph", [])]
        post = next((n for n in nodes if n.get("@type") == "BlogPosting"), None)
        if not post:
            err("no BlogPosting JSON-LD")
        else:
            for key in ("headline", "description", "url", "image", "datePublished", "dateModified", "author", "publisher"):
                if not post.get(key):
                    err(f"BlogPosting has no {key}")
            if not (iso_date(post.get("datePublished")) and iso_date(post.get("dateModified"))):
                err("BlogPosting dates are not YYYY-MM-DD")
            elif post["dateModified"] < post["datePublished"]:
                err("BlogPosting is modified before it was published")
            if post.get("url") != p.canonical:
                err("BlogPosting url differs from canonical")
            if len(post.get("headline", "")) > 110:
                err("BlogPosting headline is over 110 characters")
        crumbs = next((n for n in nodes if n.get("@type") == "BreadcrumbList"), None)
        if not crumbs or [i.get("name") for i in crumbs.get("itemListElement", [])][:2] != ["kwant", "Blog"]:
            err("no kwant > Blog breadcrumb JSON-LD")
        if f'href="/blog/{slug}"' not in index_html:
            err("not linked from blog/index.html")
    feed_path = OUT / "blog/feed.xml"
    if not feed_path.exists():
        errors.append("blog/feed.xml: missing, but there are published posts")
        return
    try:
        channel = ET.parse(feed_path).getroot().find("channel")
    except ET.ParseError as e:
        errors.append(f"blog/feed.xml: does not parse: {e}")
        return
    if channel is None or not all(channel.findtext(k) for k in ("title", "link", "description")):
        errors.append("blog/feed.xml: channel needs title, link and description")
        return
    links = []
    for item in channel.findall("item"):
        for key in ("title", "link", "guid", "pubDate", "description"):
            if not item.findtext(key):
                errors.append(f"blog/feed.xml: item without {key}")
        try:
            email.utils.parsedate_to_datetime(item.findtext("pubDate") or "")
        except (TypeError, ValueError):
            errors.append(f"blog/feed.xml: bad pubDate {item.findtext('pubDate')}")
        link = item.findtext("link") or ""
        links.append(link)
        if link not in sitemap_urls:
            errors.append(f"blog/feed.xml: {link} is not in sitemap.xml")
    for slug in published:
        if f"{SITE_URL}/blog/{slug}" not in links:
            errors.append(f"blog/feed.xml: blog/{slug} is missing")


def check_release_line(errors):
    """The hero's release line must match CHANGELOG.md, so a release can't leave kwant.si with an old count."""
    import build_site

    want = build_site.release_line()
    home = (OUT / "index.html").read_text()
    if want not in home:
        found = re.search(r'<p class="proof">.*?</p>', home)
        errors.append(
            "index.html: release line does not match CHANGELOG.md (run python3 scripts/build_site.py): "
            + (found[0] if found else "missing")
        )


def main():
    drafts = "--drafts" in sys.argv[1:]
    errors = []
    pages = sorted(OUT.rglob("*.html"))
    check_release_line(errors)
    sitemap = (OUT / "sitemap.xml").read_text()
    sitemap_urls = dict(re.findall(r"<loc>([^<]+)</loc><lastmod>(\d{4}-\d\d-\d\d)</lastmod>", sitemap))
    for path in pages:
        rel = path.relative_to(OUT).as_posix()
        p = Page()
        p.feed(path.read_text())
        err = lambda msg: errors.append(f"{rel}: {msg}")
        for name in sorted(set(re.findall(r"\{\{([A-Za-z_:]+)", path.read_text()))):
            err(f"unfilled placeholder {{{{{name}}}}} (set it in OPERATOR in scripts/build_site.py)")
        if p.headings.count(1) != 1:
            err(f"{p.headings.count(1)} h1 elements")
        for prev, cur in zip(p.headings, p.headings[1:]):
            if cur > prev + 1:
                err(f"heading skips from h{prev} to h{cur}")
                break
        if p.imgs_no_alt:
            err(f"{p.imgs_no_alt} img without alt")
        if not p.title.strip():
            err("empty <title>")
        for key in REQUIRED_META:
            if not p.meta.get(key):
                err(f"missing meta {key}")
        if not p.canonical:
            err("missing canonical")
        if p.meta.get("og:url") != p.canonical:
            err("og:url differs from canonical")
        image = p.meta.get("og:image", "")
        card = OUT / urllib.parse.urlsplit(image).path.lstrip("/")
        if not card.exists():
            err(f"og:image {image} not found")
        elif card.suffix == ".png" and png_size(card) != (1200, 630):
            err(f"og:image is {png_size(card)}, not 1200x630")
        elif card.stat().st_size > 150_000:
            err(f"og:image is {card.stat().st_size} bytes")
        for block in p.jsonld:
            try:
                data = json.loads(block)
            except json.JSONDecodeError as e:
                err(f"JSON-LD does not parse: {e}")
                continue
            for node in data.get("@graph", [data]):
                if "@type" not in node:
                    err("JSON-LD node without @type")
        noindex = "noindex" in p.meta.get("robots", "")
        if rel == "404.html":
            if not noindex:
                err("404 page is indexable")
            if p.canonical in sitemap_urls:
                err("404 page is in the sitemap")
        else:
            if noindex:
                err("indexable page has noindex")
            if not p.jsonld:
                err("no JSON-LD")
            if p.canonical not in sitemap_urls:
                err("not in sitemap.xml (with a lastmod)")
        for ref in p.refs:
            if ref.startswith("#"):
                if ref[1:] and ref[1:] not in p.ids:
                    err(f"anchor {ref} has no target")
                continue
            hit = resolve(path, ref.split("?")[0].split("#")[0] if not ref.startswith("http") else ref)
            if hit and not exists(*hit):
                err(f"broken link {ref}")
    for url in sitemap_urls:
        target = resolve(OUT / "index.html", url)
        if target and not exists(*target):
            errors.append(f"sitemap.xml: {url} has no page")
    check_blog(errors, sitemap_urls, drafts)
    robots = (OUT / "robots.txt").read_text()
    if f"Sitemap: {SITE_URL}/sitemap.xml" not in robots or "Disallow: /\n" in robots:
        errors.append("robots.txt: missing sitemap line or blocks the site")
    print(f"checked {len(pages)} pages, {len(sitemap_urls)} sitemap URLs")
    for e in errors:
        print("FAIL", e)
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
