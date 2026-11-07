"""The release gate's README and What's new checks: README.md's "New in X.Y" section must name the newest release,
and frontend/src/whatsnew/<version>.json must exist and keep the sheet's limits."""
import json
import tempfile
import unittest
from pathlib import Path

import site_release_gate as gate

CHANGELOG = "# Changelog\n\n## Unreleased\n\n## [Mac 0.7.2] - 2026-11-06\n\n### Added\n- Things.\n\n" \
            "## [Mac 0.7.1] - 2026-11-06\n"


NAV = "export const SPACE_SETTINGS_TABS = ['general', 'accounts', 'bots', 'teams', 'connections'] as const;\n"


def whatsnew(**over):
    entry = {
        "version": "0.7.2",
        "date": "2026-11-06",
        "highlights": [
            {"id": "teams", "title": "Ready-made teams", "text": "One step.", "image": None, "open": "settings/teams"},
            {"id": "jira", "title": "Jira", "text": "Tickets.", "image": None, "open": None},
        ],
        "fixes": ["A fix."],
    }
    entry.update(over)
    return entry


def readme(section):
    return "# kwant\n\nIntro.\n\n---\n\n" + section + "\n---\n\n## Why kwant\n\n- 0.7.2 is mentioned here too.\n"


class ReadmeGap(unittest.TestCase):
    def test_section_naming_the_release_passes(self):
        self.assertIsNone(gate.readme_gap("0.7.2", readme("## New in 0.7\n\n- **Repos** (0.7.2). Text.\n")))

    def test_covers_comment_passes(self):
        text = readme("## New in 0.7\n\n- **Dev mode** (0.7.0).\n<!-- covers: 0.7.2 -->\n")
        self.assertIsNone(gate.readme_gap("0.7.2", text))

    def test_old_minor_section_is_a_gap(self):
        gap = gate.readme_gap("0.7.2", readme("## New in 0.6\n\n- **Routing** (0.6.4).\n"))
        self.assertIn("no '## New in 0.7' section", gap)

    def test_section_missing_the_patch_is_a_gap(self):
        gap = gate.readme_gap("0.7.2", readme("## New in 0.7\n\n- **Dev mode** (0.7.0, 0.7.1).\n"))
        self.assertIn("doesn't name 0.7.2", gap)

    def test_version_outside_the_section_does_not_count(self):
        # "Why kwant" below the rule mentions 0.7.2; only the New in section counts.
        self.assertIn("doesn't name", gate.readme_gap("0.7.2", readme("## New in 0.7\n\n- **A** (0.7.0).\n")))

    def test_longer_version_does_not_count(self):
        self.assertIn("doesn't name", gate.readme_gap("0.7.2", readme("## New in 0.7\n\n- **A** (0.7.20).\n")))
        self.assertIn("doesn't name", gate.readme_gap("0.7.2", readme("## New in 0.7\n\n- **A** (10.7.2).\n")))


class Gaps(unittest.TestCase):
    def tree(self, readme_text, wn=None):
        root = Path(self.tmp.name)
        (root / "frontend" / "src" / "whatsnew").mkdir(parents=True)
        (root / "frontend" / "src" / "nav.ts").write_text(NAV)
        if wn is not False:
            (root / "frontend" / "src" / "whatsnew" / "0.7.2.json").write_text(json.dumps(wn or whatsnew()))
        (root / "site-src").mkdir()
        (root / "site" / "assets" / "media").mkdir(parents=True)
        (root / "CHANGELOG.md").write_text(CHANGELOG)
        (root / "site-src" / "features.html").write_text('<section id="new-072" class="fgroup"></section>')
        (root / "site-src" / "index.html").write_text('<a href="features.html#new-072">New</a>')
        if readme_text is not None:
            (root / "README.md").write_text(readme_text)
        return root

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

    def test_current_site_and_readme_have_no_gaps(self):
        self.assertEqual(gate.gaps(self.tree(readme("## New in 0.7\n\n- **A** (0.7.2).\n"))), [])

    def test_stale_readme_is_the_only_gap(self):
        found = gate.gaps(self.tree(readme("## New in 0.6\n\n- **A** (0.6.4).\n")))
        self.assertEqual(len(found), 1)
        self.assertTrue(found[0].startswith("README.md: no '## New in 0.7' section"))

    def test_missing_readme_is_a_gap(self):
        self.assertEqual(gate.gaps(self.tree(None)), ["README.md is missing"])


class WhatsNew(unittest.TestCase):
    """Check 6: a release fails without frontend/src/whatsnew/<version>.json, or with one that breaks the limits."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        Gaps.tree(self, readme("## New in 0.7\n\n- **A** (0.7.2).\n"), wn=False)

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, entry):
        (self.root / "frontend" / "src" / "whatsnew" / "0.7.2.json").write_text(json.dumps(entry))

    def test_missing_file_fails_the_release(self):
        found = gate.gaps(self.root)
        self.assertEqual(len(found), 1)
        self.assertTrue(found[0].startswith("frontend/src/whatsnew/0.7.2.json is missing"))

    def test_good_file_passes(self):
        self.write(whatsnew())
        self.assertEqual(gate.gaps(self.root), [])

    def test_wrong_version_and_bad_json(self):
        self.write(whatsnew(version="0.7.1"))
        self.assertIn("version is '0.7.1'", gate.whatsnew_gaps("0.7.2", self.root)[0])
        (self.root / "frontend" / "src" / "whatsnew" / "0.7.2.json").write_text("{nope")
        self.assertIn("isn't valid JSON", gate.whatsnew_gaps("0.7.2", self.root)[0])

    def test_limits(self):
        h = whatsnew()["highlights"][0]
        cases = [
            (whatsnew(highlights=[h]), "1 highlights, needs 2-6"),
            (whatsnew(highlights=[dict(h, id=f"h{i}") for i in range(7)]), "7 highlights, needs 2-6"),
            (whatsnew(highlights=[dict(h, title="x" * 61), dict(h, id="b")]), "title is 61 characters, max 60"),
            (whatsnew(highlights=[dict(h, text="x" * 201), dict(h, id="b")]), "text is 201 characters, max 200"),
            (whatsnew(highlights=[h, h]), "is used twice"),
            (whatsnew(fixes=["x"] * 16), "16 fixes, max 15"),
            (whatsnew(fixes=["x" * 141]), "fixes[0] is 141 characters, max 140"),
            (whatsnew(highlights=[dict(h, open="settings/nope"), dict(h, id="b")]), "isn't an in-app place"),
            (whatsnew(highlights=[dict(h, image="pic.png"), dict(h, id="b")]), "image must be null or a .webp"),
            (whatsnew(highlights=[dict(h, image="pic.webp"), dict(h, id="b")]), "whatsnew/0.7.2/pic.webp is missing"),
        ]
        for entry, want in cases:
            with self.subTest(want=want):
                self.write(entry)
                found = gate.whatsnew_gaps("0.7.2", self.root)
                self.assertEqual(len(found), 1, found)
                self.assertIn(want, found[0])

    def test_limits_count_characters_not_bytes(self):
        h = whatsnew()["highlights"][0]
        self.write(whatsnew(highlights=[dict(h, title="é" * 60), dict(h, id="b")]))
        self.assertEqual(gate.whatsnew_gaps("0.7.2", self.root), [])

    def test_image_over_80_kb_fails(self):
        pics = self.root / "frontend" / "public" / "whatsnew" / "0.7.2"
        pics.mkdir(parents=True)
        (pics / "small.webp").write_bytes(b"x" * 80 * 1024)
        (pics / "big.webp").write_bytes(b"x" * (80 * 1024 + 1))
        h = whatsnew()["highlights"][0]
        self.write(whatsnew(highlights=[dict(h, image="small.webp"), dict(h, id="b", image="big.webp")]))
        found = gate.whatsnew_gaps("0.7.2", self.root)
        self.assertEqual(len(found), 1, found)
        self.assertIn("big.webp is 80.0 KB, max 80 KB", found[0])

    def test_every_open_in_the_repo_file_is_an_app_place(self):
        # The real files: the gate reads the real nav.ts, so a renamed tab fails here first.
        for f in sorted((gate.ROOT / "frontend" / "src" / "whatsnew").glob("*.json")):
            with self.subTest(file=f.name):
                self.assertEqual(gate.whatsnew_gaps(f.stem, gate.ROOT), [])


if __name__ == "__main__":
    unittest.main()
