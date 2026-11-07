"""The release gate's README check: README.md's "New in X.Y" section must name the newest release."""
import tempfile
import unittest
from pathlib import Path

import site_release_gate as gate

CHANGELOG = "# Changelog\n\n## Unreleased\n\n## [Mac 0.7.2] - 2026-11-06\n\n### Added\n- Things.\n\n" \
            "## [Mac 0.7.1] - 2026-11-06\n"


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
    def tree(self, readme_text):
        root = Path(self.tmp.name)
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


if __name__ == "__main__":
    unittest.main()
