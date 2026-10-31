"""Tests for the blog in build_site.py: the Markdown subset, the comparison table, drafts, reading time, related posts
and the feed. Run with `python3 -m unittest discover -s scripts -p 'test_*.py'`."""
import pathlib
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET

sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import build_site as site  # noqa: E402


def post(name, date, tags="", draft=False, body="Some words here."):
    head = f"title: Post {name} title\ndescription: A description for post {name}.\ndate: {date}\ntags: {tags}\n"
    return head + ("draft: yes\n" if draft else "") + "---\n" + body + "\n"


class Markdown(unittest.TestCase):
    def test_blocks(self):
        out = site.markdown("## Why it *matters*\n\nA [link](https://example.com) and `<code>`.\n\n- one\n- two\n\n1. a\n2. b\n\n> quoted\n\n```\n<b>x</b>\n```")
        self.assertIn('<h2 id="why-it-matters">Why it <em>matters</em></h2>', out)
        self.assertIn('<a href="https://example.com" rel="noopener">link</a>', out)
        self.assertIn("<code>&lt;code&gt;</code>", out)
        self.assertIn("<ul><li>one</li><li>two</li></ul>", out)
        self.assertIn("<ol><li>a</li><li>b</li></ol>", out)
        self.assertIn("<blockquote><p>quoted</p></blockquote>", out)
        self.assertIn("<pre><code>&lt;b&gt;x&lt;/b&gt;</code></pre>", out)

    def test_text_is_escaped(self):
        self.assertIn("&lt;script&gt;", site.md_inline("a <script> tag"))

    def test_safe_links_render(self):
        cases = {
            "https://github.com/x": '<a href="https://github.com/x" rel="noopener">t</a>',
            "HTTP://example.com/": '<a href="HTTP://example.com/" rel="noopener">t</a>',
            "https://kwant.si/features": '<a href="https://kwant.si/features">t</a>',
            "mailto:hi@kwant.si": '<a href="mailto:hi@kwant.si">t</a>',
            "/docs/install": '<a href="/docs/install">t</a>',
            "#setup": '<a href="#setup">t</a>',
            "{{root}}docs/quickstart.html": '<a href="{{root}}docs/quickstart.html">t</a>',
            "https://a.example/?x=1&y=2": '<a href="https://a.example/?x=1&amp;y=2" rel="noopener">t</a>',
        }
        for href, want in cases.items():
            with self.subTest(href=href):
                self.assertEqual(site.md_inline(f"[t]({href})"), want)

    def test_unsafe_links_stay_text(self):
        for href in ("javascript:alert(5)", "JavaScript:alert(1)", "data:text/html,hi", "vbscript:msgbox",
                     "//evil.example/x", "/\\evil.example", "{{root}}javascript:x", "file:///etc/passwd",
                     "docs/install.html", "https:/evil", "&#106;avascript:alert(1)", "java&#x09;script:x"):
            with self.subTest(href=href):
                out = site.md_inline(f"[t]({href})")
                self.assertNotIn("<a", out)
                self.assertTrue(out.startswith("[t]("), out)

    def test_link_url_with_parentheses(self):
        self.assertEqual(
            site.md_inline("see [Foo](https://en.wikipedia.org/wiki/Foo_(bar)) now"),
            'see <a href="https://en.wikipedia.org/wiki/Foo_(bar)" rel="noopener">Foo</a> now',
        )
        self.assertEqual(
            site.md_inline("([docs](/docs/install))"),
            '(<a href="/docs/install">docs</a>)',
        )

    def test_h1_is_refused(self):
        with self.assertRaises(SystemExit):
            site.markdown("# Title")

    def test_compare_labels_every_cell(self):
        out = site.markdown("```compare\ncaption: Test\n| Feature | kwant | Other |\n| --- | --- | --- |\n| Self-hosted | yes | partial (paid) |\n| Price | Free | $5 |\n```")
        self.assertIn("<caption>Test</caption>", out)
        self.assertIn('<th scope="row">Self-hosted</th>', out)
        self.assertEqual(out.count('data-label="kwant"'), 2)
        self.assertEqual(out.count('data-label="Other"'), 2)
        self.assertIn('aria-hidden="true">✓</span> Yes', out)
        self.assertIn('Partly <span class="cmp-note">(paid)</span>', out)

    def test_reading_time(self):
        self.assertEqual(site.reading_minutes("word " * 10), 1)
        self.assertEqual(site.reading_minutes("word " * (site.WORDS_PER_MINUTE * 2 + 1)), 3)


class Posts(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.old = site.BLOG_SRC
        site.BLOG_SRC = pathlib.Path(self.dir.name)
        files = {"a.md": post("a", "2026-01-01", "x, y"), "b.md": post("b", "2026-02-01", "y"),
                 "c.md": post("c", "2026-03-01", "z"), "d.md": post("d", "2026-04-01", "x, y", draft=True)}
        for name, text in files.items():
            (site.BLOG_SRC / name).write_text(text)

    def tearDown(self):
        site.BLOG_SRC = self.old
        self.dir.cleanup()

    def test_drafts_left_out(self):
        self.assertEqual([m["slug"] for m, _ in site.blog_posts()], ["c", "b", "a"])
        self.assertEqual([m["slug"] for m, _ in site.blog_posts(drafts=True)], ["d", "c", "b", "a"])

    def test_related_prefers_shared_tags(self):
        posts = site.blog_posts()
        a = next(m for m, _ in posts if m["slug"] == "a")
        self.assertEqual([m["slug"] for m in site.related(a, posts)], ["b", "c"])

    def test_feed_parses_and_is_stable(self):
        posts = site.blog_posts()
        feed = site.blog_feed(posts)
        self.assertEqual(feed, site.blog_feed(site.blog_posts()))
        items = ET.fromstring(feed).find("channel").findall("item")
        self.assertEqual([i.findtext("link") for i in items], [f"{site.SITE_URL}/blog/{s}" for s in "cba"])
        self.assertTrue(all(i.findtext("pubDate") and i.findtext("guid") for i in items))

    def test_missing_title_fails(self):
        (site.BLOG_SRC / "e.md").write_text("description: x\ndate: 2026-01-01\n---\nbody\n")
        with self.assertRaises(SystemExit):
            site.blog_posts()


if __name__ == "__main__":
    unittest.main()
