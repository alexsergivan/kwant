"""Tests for mac_latest_json.py: python3 -m unittest discover -s scripts -p 'test_*.py'"""

import contextlib
import io
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mac_latest_json as m  # noqa: E402

CHANGELOG = """# Changelog

## Unreleased

### Added
- Not released yet.

## [Mac 0.1.2] - 2026-10-27

### Added
- **In-app updates.** kwant tells you when a new version is out.

### Fixed
- Something.

## [Mac 0.1.1] - 2026-10-26

### Added
- Older things.

## [Mac 0.1.10] - 2026-12-01

- Not 0.1.1.
"""

URL = "https://github.com/alexsergivan/kwant/releases/download/mac-v0.1.2/kwant_universal.app.tar.gz"


class ChangelogSection(unittest.TestCase):
    def test_section_up_to_the_next_heading(self):
        notes = m.changelog_section(CHANGELOG, "0.1.2")
        self.assertTrue(notes.startswith("### Added\n- **In-app updates.**"))
        self.assertTrue(notes.endswith("- Something."))
        self.assertNotIn("Older things", notes)
        self.assertNotIn("Not released yet", notes)

    def test_exact_version_only(self):
        self.assertEqual(m.changelog_section(CHANGELOG, "0.1.1"), "### Added\n- Older things.")
        self.assertEqual(m.changelog_section(CHANGELOG, "0.1.10"), "- Not 0.1.1.")

    def test_missing_section_fails(self):
        with self.assertRaises(ValueError):
            m.changelog_section(CHANGELOG, "0.1.3")
        with self.assertRaises(ValueError):
            m.changelog_section(CHANGELOG, "0.1")


class LatestJson(unittest.TestCase):
    def test_shape(self):
        doc = m.latest_json("0.1.2", "c2lnbmF0dXJl\n", URL, "### Added\n- x", "2026-10-27T10:00:00Z")
        self.assertEqual(doc["version"], "0.1.2")
        self.assertEqual(doc["notes"], "### Added\n- x")
        self.assertEqual(doc["pub_date"], "2026-10-27T10:00:00Z")
        self.assertEqual(set(doc["platforms"]), {"darwin-aarch64", "darwin-x86_64"})
        for p in doc["platforms"].values():
            self.assertEqual(p, {"signature": "c2lnbmF0dXJl", "url": URL})

    def test_refuses_bad_input(self):
        for version, sig, url, notes in [
            ("0.2.0-rc.1", "s", URL, "n"),
            ("v0.1.2", "s", URL, "n"),
            ("0.1", "s", URL, "n"),
            ("0.1.2", "  \n", URL, "n"),
            ("0.1.2", "s", "http://example.com/a.tar.gz", "n"),
            ("0.1.2", "s", URL, ""),
        ]:
            with self.subTest(version=version, sig=sig, url=url, notes=notes):
                with self.assertRaises(ValueError):
                    m.latest_json(version, sig, url, notes, "2026-10-27T10:00:00Z")


class Cli(unittest.TestCase):
    def run_main(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = m.main(list(args))
        return code, out.getvalue(), err.getvalue()

    def test_writes_the_feed(self):
        with tempfile.TemporaryDirectory() as d:
            cl, sig = os.path.join(d, "CHANGELOG.md"), os.path.join(d, "a.sig")
            open(cl, "w").write(CHANGELOG)
            open(sig, "w").write("c2lnbmF0dXJl")
            code, out, err = self.run_main("--version", "0.1.2", "--signature", sig, "--url", URL, "--changelog", cl)
            self.assertEqual(code, 0, err)
            doc = json.loads(out)
            self.assertEqual(doc["version"], "0.1.2")
            self.assertRegex(doc["pub_date"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
            self.assertIn("In-app updates", doc["notes"])

            code, out, err = self.run_main("--version", "0.1.3", "--signature", sig, "--url", URL, "--changelog", cl)
            self.assertEqual(code, 1)
            self.assertEqual(out, "")
            self.assertIn("no '## [Mac 0.1.3]' section", err)

    def test_real_changelog_has_released_sections(self):
        root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        text = open(os.path.join(root, "CHANGELOG.md"), encoding="utf-8").read()
        self.assertTrue(m.changelog_section(text, "0.1.1"))


if __name__ == "__main__":
    unittest.main()
