"""Tests for the performance budget's comparison logic (they need no kwnt binary and run in the `site` job's unittest step).

- over its limit fails, at its limit passes, under passes;
- a missing browser is skipped and never fails the run;
- a measure with no limit in perf_budget.json is reported, not failed;
- limits are the baseline x 1.5 (memory x 1.3), rounded up;
- the shipped perf_budget.json names only known measures, and every limit is above its baseline;
- the CI step exists in the fresh-machine job and is not a required check yet.
"""
import json
import re
import sys
import unittest
from unittest import mock
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import perf_budget as pb  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
LIMITS = {"cold_start_s": 2.0, "first_reply_s": 1.0, "idle_rss_mb": 150, "after_reply_rss_mb": 160, "browser_launch_s": 6.0}


def status(rows, key):
    return next(r["status"] for r in rows if r["id"] == key)


class Judge(unittest.TestCase):
    def test_under_the_limit_passes(self):
        rows = pb.judge({"cold_start_s": 1.2}, LIMITS)
        self.assertEqual(status(rows, "cold_start_s"), "PASS")
        self.assertEqual(pb.exit_code(rows), 0)

    def test_at_the_limit_passes(self):
        rows = pb.judge({"cold_start_s": 2.0, "idle_rss_mb": 150}, LIMITS)
        self.assertEqual(status(rows, "cold_start_s"), "PASS")
        self.assertEqual(status(rows, "idle_rss_mb"), "PASS")
        self.assertEqual(pb.exit_code(rows), 0)

    def test_over_the_limit_fails_and_exits_1(self):
        rows = pb.judge({"cold_start_s": 2.01, "first_reply_s": 0.2}, LIMITS)
        self.assertEqual(status(rows, "cold_start_s"), "FAIL")
        self.assertEqual(status(rows, "first_reply_s"), "PASS")
        self.assertEqual(pb.exit_code(rows), 1)

    def test_one_failing_measure_fails_the_run_even_when_the_rest_pass(self):
        medians = {k: v / 2 for k, v in LIMITS.items()}
        medians["after_reply_rss_mb"] = 161
        self.assertEqual(pb.exit_code(pb.judge(medians, LIMITS)), 1)

    def test_a_missing_browser_is_skipped_not_failed(self):
        medians = {k: v / 2 for k, v in LIMITS.items() if k != "browser_launch_s"}
        rows = pb.judge(medians, LIMITS)
        self.assertEqual(status(rows, "browser_launch_s"), "SKIP")
        self.assertEqual(pb.exit_code(rows), 0)

    def test_a_measure_without_a_limit_is_reported_not_failed(self):
        rows = pb.judge({"cold_start_s": 99.0}, {})
        self.assertEqual(status(rows, "cold_start_s"), "NO LIMIT")
        self.assertEqual(pb.exit_code(rows), 0)

    def test_the_table_has_one_line_per_measure_and_the_result(self):
        text = pb.table(pb.judge({"cold_start_s": 3.0}, LIMITS))
        self.assertEqual(len(text.splitlines()), 1 + len(pb.MEASURES))
        self.assertIn("FAIL", text)
        self.assertIn("SKIP", text)


class Numbers(unittest.TestCase):
    def test_median_ignores_missing_values(self):
        self.assertEqual(pb.median([3, None, 1, 2]), 2)
        self.assertIsNone(pb.median([None]))
        self.assertIsNone(pb.median([]))

    def test_round_up_never_rounds_down(self):
        self.assertEqual(pb.round_up(0.31, 1), 0.4)
        self.assertEqual(pb.round_up(0.3, 1), 0.3)
        self.assertEqual(pb.round_up(100.2, 0), 101)
        self.assertEqual(pb.round_up(0.1 * 3, 1), 0.3)  # 0.30000000000000004 must not become 0.4

    def test_limits_are_the_baseline_times_1_5_and_memory_times_1_3_rounded_up(self):
        limits = pb.compute_limits({"cold_start_s": 1.1, "first_reply_s": 0.2, "idle_rss_mb": 90.0, "browser_launch_s": 2.5})
        self.assertEqual(limits["cold_start_s"], 1.7)  # 1.65
        self.assertEqual(limits["first_reply_s"], 0.3)
        self.assertEqual(limits["idle_rss_mb"], 117)  # 90 x 1.3 = 117, not 118
        self.assertEqual(limits["browser_launch_s"], 3.8)  # 3.75


DOWN = {"error": "The bot browser could not start kwant's Chrome on this computer: Chrome started but did not open its debugging port in time.", "is_running": False}


class ChromeThatWillNotStart(unittest.TestCase):
    """A runner where Chrome cannot start is SKIP with the reason, never FAIL; a Chrome that starts is still held to its limit."""

    def test_the_browser_row_is_skip_with_the_reason_and_the_run_passes(self):
        medians = {k: v / 2 for k, v in LIMITS.items() if k != "browser_launch_s"}
        rows = pb.judge(medians, LIMITS, {"browser_launch_s": "Chrome did not start"})
        row = next(r for r in rows if r["id"] == "browser_launch_s")
        self.assertEqual(row["status"], "SKIP")
        self.assertEqual(row["reason"], "Chrome did not start")
        self.assertEqual(pb.exit_code(rows), 0)

    def test_the_reason_is_in_the_printed_table_and_the_job_summary(self):
        rows = pb.judge({}, LIMITS, {"browser_launch_s": "Chrome did not start"})
        self.assertIn("Chrome did not start", pb.table(rows))
        self.assertIn("Chrome did not start", pb.summary_markdown(rows))
        self.assertIn("| SKIP |", pb.summary_markdown(rows))

    def test_a_slow_browser_that_did_start_still_fails(self):
        medians = {k: v / 2 for k, v in LIMITS.items()}
        medians["browser_launch_s"] = LIMITS["browser_launch_s"] + 0.1
        rows = pb.judge(medians, LIMITS, {"browser_launch_s": "ignored: it was measured"})
        self.assertEqual(status(rows, "browser_launch_s"), "FAIL")
        self.assertIsNone(next(r for r in rows if r["id"] == "browser_launch_s")["reason"])
        self.assertEqual(pb.exit_code(rows), 1)

    def test_a_503_from_kwnt_about_chrome_that_lasts_past_the_wait_is_chrome_did_not_start(self):
        with mock.patch.object(pb, "request", return_value=(503, DOWN)) as req, mock.patch.object(pb.time, "sleep"):
            with self.assertRaises(pb.ChromeDidNotStart) as ctx:
                pb.open_browser_page(1234, "kid", 0)
        self.assertIn("debugging port", str(ctx.exception))
        self.assertEqual(req.call_count, 1)  # the wait was already over after the first answer

    def test_it_asks_again_inside_the_wait_and_times_the_whole_launch(self):
        answers = [(503, DOWN), (200, {"status": "navigated"})]
        with mock.patch.object(pb, "request", side_effect=answers), mock.patch.object(pb.time, "sleep"):
            seconds = pb.open_browser_page(1234, "kid", 60)
        self.assertGreaterEqual(seconds, 0)

    def test_any_other_failure_is_a_real_failure_not_a_skip(self):
        for answer in [(500, {"error": "boom"}), (503, {"error": "Sandbox browser is unavailable"}), (200, {"status": "failed"}), (404, "nope")]:
            with mock.patch.object(pb, "request", return_value=answer):
                with self.assertRaises(RuntimeError, msg=str(answer)):
                    pb.open_browser_page(1234, "kid", 60)


class ShippedFiles(unittest.TestCase):
    def test_perf_budget_json_names_known_measures_and_sits_above_its_baseline(self):
        data = json.loads((ROOT / "scripts" / "perf_budget.json").read_text())
        for key, limit in data["limits"].items():
            self.assertIn(key, pb.MEASURES)
            if key in data.get("baseline", {}):
                self.assertGreaterEqual(limit, data["baseline"][key], key)

    def test_the_ci_step_is_in_the_fresh_machine_job_and_not_required_yet(self):
        ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
        job = re.search(r"\n  fresh-machine:\n(.*?)\n  \w[\w-]*:\n", ci, re.S)
        self.assertTrue(job, "no fresh-machine job in ci.yml")
        self.assertIn("name: Performance budget", job.group(1))
        self.assertIn("scripts/perf_budget.py", job.group(1))
        self.assertNotIn("Performance budget", (ROOT / "scripts" / "release_required_checks.txt").read_text())


class BarePathHasWhatChromesLauncherNeeds(unittest.TestCase):
    def test_readlink_and_touch_are_on_the_bare_path(self):
        # /usr/bin/google-chrome is a shell script: on the CI runner it died with "readlink: command not found" and
        # "touch: command not found" under the run's bare PATH, and Chrome never opened its debugging port
        for tool in ("readlink", "touch"):
            self.assertIn(tool, pb.SYSTEM_TOOLS)


if __name__ == "__main__":
    unittest.main()
