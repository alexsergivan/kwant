"""Tests for the hero's release line, which build_site.py reads from CHANGELOG.md."""
import datetime
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import build_site as site  # noqa: E402

LOG = """# Changelog

## Unreleased

### Added
- Something.

## [Mac 0.2.0] - 2026-11-02
## [Mac 0.10.0] - 2026-11-02
## [Mac 0.1.0-test.1] - 2026-10-20
## [Mac 0.1.0] - 2026-10-25
### [Mac 9.9.9] - 2026-12-01
"""


class ReleaseLine(unittest.TestCase):
    def test_counts_only_mac_release_headings(self):
        rel = site.releases(LOG)
        self.assertEqual([v for v, _ in rel], ["0.10.0", "0.2.0", "0.1.0"])
        self.assertEqual(rel[-1][1], datetime.date(2026, 10, 25))

    def test_line_names_latest_count_and_first_date(self):
        line = site.release_line(LOG)
        self.assertIn("0.10.0 released Nov 2", line)
        self.assertIn("3 releases since Oct 25", line)
        self.assertIn("AGPL-3.0", line)

    def test_single_release(self):
        self.assertIn("First release", site.release_line("## [Mac 1.0.0] - 2027-01-05\n"))

    def test_no_releases_fails_the_build(self):
        with self.assertRaises(SystemExit):
            site.releases("# Changelog\n")

    def test_real_changelog_parses(self):
        self.assertGreaterEqual(len(site.releases()), 10)


if __name__ == "__main__":
    unittest.main()
