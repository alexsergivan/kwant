"""Tests for check_test_skips.py: python3 -m unittest discover -s scripts -p 'test_*.py'"""
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_test_skips  # noqa: E402


class CheckTestSkips(unittest.TestCase):
    def tree(self, files):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        root = Path(d.name)
        for rel in ("tests", "src"):
            (root / rel).mkdir()
        for rel, body in files.items():
            p = root / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(textwrap.dedent(body))
        return root

    def problems(self, files):
        return check_test_skips.check(self.tree(files))

    def test_a_skip_printed_in_a_test_fails_on_its_line(self):
        found = self.problems({"tests/a_test.rs": """\
            #[test]
            fn t() {
                if chrome().is_none() {
                    eprintln!("skipped: no Chrome or Chromium installed");
                    return;
                }
            }
            """})
        self.assertEqual(len(found), 1, found)
        self.assertTrue(found[0].startswith("tests/a_test.rs:4: a test prints a skip itself"), found)

    def test_other_spellings_and_a_split_macro_are_caught(self):
        found = self.problems({"tests/b_test.rs": """\
            fn a() { println!("git isn't installed; skipped"); }
            fn b() { eprintln!("KWNT_X is not set: Skipping the live test"); }
            fn c() {
                eprintln!(
                    "window part skipped: no Xvfb"
                );
            }
            fn d() { eprint!("no ffmpeg here: skipped\\n"); }
            """})
        self.assertEqual([p.split(": ")[0] for p in found],
                         ["tests/b_test.rs:1", "tests/b_test.rs:2", "tests/b_test.rs:4", "tests/b_test.rs:8"], found)

    def test_the_helper_and_other_prints_pass(self):
        found = self.problems({
            "tests/support/skip.rs": 'pub fn skip_or_fail() { eprintln!("skipped: {what}"); std::env::var("CI"); }\n',
            "tests/c_test.rs": """\
                fn t() {
                    if chrome().is_none() {
                        skip::skip_or_fail("Chrome or Chromium", skip::CHROME_FIX);
                        return;
                    }
                    eprintln!("took {ms} ms");
                }
                """,
        })
        self.assertEqual(found, [])

    def test_in_src_only_the_test_module_counts(self):
        found = self.problems({"src/setup.rs": """\
            fn install() {
                println!("  skipped: no terminal to ask in");
            }
            #[cfg(test)]
            mod tests {
                fn t() { eprintln!("skipped: no Chrome"); }
            }
            """})
        self.assertEqual([p.split(": ")[0] for p in found], ["src/setup.rs:6"], found)

    def test_reading_ci_directly_fails(self):
        found = self.problems({"tests/d_test.rs": """\
            fn t() {
                assert!(std::env::var_os("CI").is_none());
                assert!(std::env::var("KWNT_AGENT_TEST_REQUIRE").is_err());
            }
            """})
        self.assertEqual([p.split(": ")[0] for p in found], ["tests/d_test.rs:2", "tests/d_test.rs:3"], found)
        self.assertIn("skip::strict()", found[0])

    def test_an_ignored_test_must_be_listed(self):
        files = {"tests/e_test.rs": """\
            #[tokio::test]
            #[ignore = "needs the network"]
            async fn a_live_page_works() {}
            #[test]
            #[ignore]
            fn listed_one() {}
            fn f() { s.replace("#[test]\\n", "#[test]\\n#[ignore]\\n"); }
            """,
                 "tests/NOT_IN_CI.md": "| `listed_one` | why | cmd |\n"}
        found = self.problems(files)
        self.assertEqual(len(found), 1, found)
        self.assertTrue(found[0].startswith("tests/e_test.rs:2: #[ignore]d test `a_live_page_works` is not listed"), found)

    def test_this_repo_passes(self):
        root = Path(__file__).resolve().parent.parent
        self.assertEqual(check_test_skips.check(root), [])


if __name__ == "__main__":
    unittest.main()
