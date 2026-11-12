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


if __name__ == "__main__":
    unittest.main()
