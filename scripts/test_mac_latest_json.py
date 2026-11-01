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

    def test_empty_notes_are_refused(self):
        changelog = "## Unreleased\n\n## [Mac 0.2.0] - 2026-11-01\n\n### Added\n\n## [Mac 0.1.2]\n- Old.\n"
        with self.assertRaises(ValueError):
            m.release_notes(changelog, "0.2.0")
        with self.assertRaises(ValueError):
            m.release_notes("## [Mac 0.2.0]\n\n## [Mac 0.1.2]\n- Old.\n", "0.2.0")
        self.assertEqual(m.release_notes(changelog, "0.1.2"), "- Old.")

    def test_check_notes_cli(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "CHANGELOG.md")
            with open(path, "w", encoding="utf-8") as f:
                f.write("## [Mac 0.2.0]\n### Fixed\n\n## [Mac 0.1.2]\n- Old.\n")
            with contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(m.main(["--check-notes", "--version", "0.2.0", "--changelog", path]), 1)
            self.assertIn("empty", err.getvalue())
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(m.main(["--check-notes", "--version", "0.3.0", "--changelog", path]), 1)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(m.main(["--check-notes", "--version", "0.1.2", "--changelog", path]), 0)
            self.assertEqual(out.getvalue(), "", "a check writes no feed")

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


# What scripts/mac_latest_json.py wrote at main 72a8ac5 (Mac 0.6.0) for the inputs in MacBytes.
# Installed Macs parse this file; adding Linux must not change a byte of it.
MAC_ONLY_GOLDEN = """{
  "version": "0.1.2",
  "notes": "### Added\\n- **In-app updates.** \u201cquotes\u201d and \u00fcn\u00efcode.",
  "pub_date": "2026-10-27T10:00:00Z",
  "platforms": {
    "darwin-aarch64": {
      "signature": "c2lnbmF0dXJl",
      "url": "https://github.com/alexsergivan/kwant/releases/download/mac-v0.1.2/kwant_universal.app.tar.gz"
    },
    "darwin-x86_64": {
      "signature": "c2lnbmF0dXJl",
      "url": "https://github.com/alexsergivan/kwant/releases/download/mac-v0.1.2/kwant_universal.app.tar.gz"
    }
  }
}
"""

DL = "https://github.com/alexsergivan/kwant/releases/download/mac-v0.1.2/"


class MacBytes(unittest.TestCase):
    """The Linux entries are extra keys after the Mac ones; the Mac part of the file stays as it was."""

    def feed(self, *linux):
        with tempfile.TemporaryDirectory() as d:
            cl, sig = os.path.join(d, "CHANGELOG.md"), os.path.join(d, "a.sig")
            with open(cl, "w", encoding="utf-8") as f:
                f.write("## [Mac 0.1.2] - 2026-10-27\n\n### Added\n- **In-app updates.** \u201cquotes\u201d and \u00fcn\u00efcode.\n")
            with open(sig, "w", encoding="utf-8") as f:
                f.write("c2lnbmF0dXJl\n")
            args = ["--version", "0.1.2", "--signature", sig, "--url", DL + "kwant_universal.app.tar.gz",
                    "--changelog", cl, "--pub-date", "2026-10-27T10:00:00Z"]
            for key, name in linux:
                path = os.path.join(d, name + ".sig")
                with open(path, "w", encoding="utf-8") as f:
                    f.write(f"sig-of-{name}\n")
                args += ["--linux", key, path, DL + name]
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                code = m.main(args)
            self.assertEqual(code, 0, err.getvalue())
            return out.getvalue()

    def test_mac_only_feed_is_byte_identical_to_main(self):
        self.assertEqual(self.feed(), MAC_ONLY_GOLDEN)

    def test_adding_linux_leaves_every_darwin_entry_byte_identical(self):
        both = self.feed(("linux-x86_64-appimage", "kwant_x86_64.AppImage"), ("linux-x86_64-deb", "kwant_amd64.deb"))
        doc = json.loads(both)
        self.assertEqual(list(doc["platforms"]),
                         ["darwin-aarch64", "darwin-x86_64", "linux-x86_64-appimage", "linux-x86_64-deb"])
        self.assertEqual(doc["platforms"]["linux-x86_64-appimage"],
                         {"signature": "sig-of-kwant_x86_64.AppImage", "url": DL + "kwant_x86_64.AppImage"})
        self.assertEqual(doc["platforms"]["linux-x86_64-deb"],
                         {"signature": "sig-of-kwant_amd64.deb", "url": DL + "kwant_amd64.deb"})
        # drop the Linux keys and the file is, byte for byte, the Mac-only one
        for key in m.LINUX_PLATFORMS:
            del doc["platforms"][key]
        self.assertEqual(json.dumps(doc, indent=2, ensure_ascii=False) + "\n", MAC_ONLY_GOLDEN)
        # and the darwin lines are a verbatim prefix of the file a Mac downloads
        cut = MAC_ONLY_GOLDEN.rindex('    }\n  }\n}\n') + len("    }")
        self.assertTrue(both.startswith(MAC_ONLY_GOLDEN[:cut] + ",\n"), both)

    def test_no_bare_linux_key_and_no_unknown_key(self):
        for key in ("linux-x86_64", "linux-aarch64-appimage", "darwin-aarch64", "windows-x86_64-nsis"):
            with self.subTest(key=key), self.assertRaises(ValueError):
                m.latest_json("0.1.2", "s", URL, "n", "2026-10-27T10:00:00Z", {key: ("s", URL)})

    def test_bad_linux_entries_are_refused(self):
        for sig, url in [("  \n", URL), ("s", "http://example.com/kwant.deb")]:
            with self.subTest(sig=sig, url=url), self.assertRaises(ValueError):
                m.latest_json("0.1.2", "s", URL, "n", "2026-10-27T10:00:00Z", {"linux-x86_64-deb": (sig, url)})

    def test_a_platform_given_twice_fails(self):
        with tempfile.TemporaryDirectory() as d:
            cl, sig = os.path.join(d, "CHANGELOG.md"), os.path.join(d, "a.sig")
            open(cl, "w").write(CHANGELOG)
            open(sig, "w").write("s")
            err = io.StringIO()
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
                code = m.main(["--version", "0.1.2", "--signature", sig, "--url", URL, "--changelog", cl,
                               "--linux", "linux-x86_64-deb", sig, URL, "--linux", "linux-x86_64-deb", sig, URL])
            self.assertEqual(code, 1)
            self.assertIn("twice", err.getvalue())


if __name__ == "__main__":
    unittest.main()
