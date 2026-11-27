"""Tests for the performance budget's comparison logic (they need no kwnt binary and run in the `site` job's unittest step).

- over its limit fails, at its limit passes, under passes;
- a missing browser is skipped and never fails the run;
- a measure with no limit in perf_budget.json is reported, not failed;
- limits are the baseline x 1.5 (memory x 1.3), rounded up;
- the shipped perf_budget.json names only known measures, and every limit is above its baseline;
- the CI step exists in the fresh-machine job, has no continue-on-error, and only "blocking" measures exit 1;
- --judge re-judges a saved results file without a kwnt (how a doctored file proves a FAIL turns the step red);
- the release profile (perf-release job) is judged against perf_budget.json's own "release" limits, never unjudged;
  its limits are the median of 3 CI runs + 15%, rounded up; the job runs on PRs that touch the release build only.
"""
import json
import os
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


class MemorySplit(unittest.TestCase):
    STATUS = "Name:\tkwnt\nVmRSS:\t   91124 kB\nRssAnon:\t   28324 kB\nRssFile:\t   62800 kB\nRssShmem:\t       0 kB\n"

    def test_heap_and_mapped_files_are_read_apart(self):
        self.assertEqual(pb.rss_split(self.STATUS), {"anon_mb": 27.7, "file_mb": 61.3})

    def test_a_kernel_without_the_fields_gives_none_not_an_error(self):
        self.assertEqual(pb.rss_split("Name:\tkwnt\nVmRSS:\t 1 kB\n"), {"anon_mb": None, "file_mb": None})

    def test_a_process_that_is_gone_gives_none(self):
        self.assertIsNone(pb.rss_split_of(2 ** 22 + 12345))

    def test_this_process_has_both(self):
        split = pb.rss_split_of(os.getpid())
        self.assertGreater(split["anon_mb"], 0)
        self.assertGreater(split["file_mb"], 0)


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

    def test_the_ci_step_is_in_the_fresh_machine_job(self):
        ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
        job = re.search(r"\n  fresh-machine:\n(.*?)\n  \w[\w-]*:\n", ci, re.S)
        self.assertTrue(job, "no fresh-machine job in ci.yml")
        self.assertIn("name: Performance budget", job.group(1))
        self.assertIn("scripts/perf_budget.py", job.group(1))
        self.assertNotIn("Performance budget", (ROOT / "scripts" / "release_required_checks.txt").read_text())

    def test_the_step_blocks_now_there_is_no_continue_on_error(self):
        # Nov 26 (#1265): with continue-on-error idle memory went 109 -> 166 MB and nobody saw it. The step must fail the job.
        ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
        step = re.search(r"\n      - name: Performance budget\n(.*?)(?=\n      - name:|\Z)", ci, re.S)
        self.assertTrue(step, "no Performance budget step in ci.yml")
        self.assertNotIn("continue-on-error", step.group(1))

    def test_memory_cold_start_and_first_reply_block_and_the_browser_only_warns(self):
        data = json.loads((ROOT / "scripts" / "perf_budget.json").read_text())
        self.assertEqual(pb.blocking_of(data), {"cold_start_s", "first_reply_s", "idle_rss_mb", "after_reply_rss_mb", "after_search_rss_mb"})
        # A2: a synced Find a tool copy may cost about 20 MB over idle, not the ~150 MB it did
        self.assertLessEqual(data["limits"]["after_search_rss_mb"], data["limits"]["idle_rss_mb"] + 20)
        self.assertLessEqual(data["limits"]["idle_rss_mb"], 132)
        self.assertLessEqual(data["limits"]["after_reply_rss_mb"], 132)


class Blocking(unittest.TestCase):
    """Only a FAIL in a blocking measure exits 1; over the limit elsewhere is WARN."""

    def test_over_the_limit_in_a_measure_that_does_not_block_warns_and_exits_0(self):
        medians = {k: v / 2 for k, v in LIMITS.items()}
        medians["browser_launch_s"] = LIMITS["browser_launch_s"] + 1
        rows = pb.judge(medians, LIMITS, blocking={"cold_start_s", "idle_rss_mb"})
        self.assertEqual(status(rows, "browser_launch_s"), "WARN")
        self.assertEqual(pb.exit_code(rows), 0)
        self.assertIn("WARN", pb.table(rows))
        self.assertIn("| WARN |", pb.summary_markdown(rows))

    def test_over_the_limit_in_a_blocking_measure_fails_and_exits_1(self):
        medians = {k: v / 2 for k, v in LIMITS.items()}
        medians["idle_rss_mb"] = LIMITS["idle_rss_mb"] + 0.01
        rows = pb.judge(medians, LIMITS, blocking={"idle_rss_mb"})
        self.assertEqual(status(rows, "idle_rss_mb"), "FAIL")
        self.assertTrue(next(r for r in rows if r["id"] == "idle_rss_mb")["blocking"])
        self.assertEqual(pb.exit_code(rows), 1)

    def test_a_limits_file_without_the_list_blocks_on_everything(self):
        self.assertEqual(pb.blocking_of({"limits": {}}), set(pb.MEASURES))
        self.assertEqual(pb.blocking_of({"blocking": ["idle_rss_mb", "nonsense"]}), {"idle_rss_mb"})
        self.assertEqual(pb.blocking_of({"blocking": []}), set())


class JudgeASavedFile(unittest.TestCase):
    """--judge: the proof that a value over the limit turns the step red, without building or running kwnt."""

    def run_main(self, results, limits_data):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            res, lim = Path(d) / "perf-results.json", Path(d) / "limits.json"
            res.write_text(json.dumps(results))
            lim.write_text(json.dumps(limits_data))
            with mock.patch.object(sys, "argv", ["perf_budget.py", "--judge", str(res), "--limits", str(lim)]), \
                    mock.patch("builtins.print"):
                return pb.main()

    def samples(self, idle):
        return {"samples": {"cold_start_s": [0.5] * 5, "first_reply_s": [0.05] * 5, "idle_rss_mb": idle,
                            "after_reply_rss_mb": [128.0, 128.1, 127.9], "after_search_rss_mb": [140.0, 141.0, 139.0],
                            "browser_launch_s": [9.3] * 5}}

    def test_the_shipped_limits_pass_the_pr_numbers_and_fail_a_doctored_idle(self):
        shipped = json.loads((ROOT / "scripts" / "perf_budget.json").read_text())
        self.assertEqual(self.run_main(self.samples([129.2, 127.9, 127.4]), shipped), 0)  # browser 9.3 s only WARNs
        self.assertEqual(self.run_main(self.samples([166.3, 166.2, 166.4]), shipped), 1)

    def test_a_skipped_browser_in_the_saved_file_stays_skipped(self):
        results = self.samples([100.0] * 3)
        results["samples"]["browser_launch_s"] = [None] * 5
        results["browser_skip"] = "Chrome did not start"
        self.assertEqual(self.run_main(results, {"limits": LIMITS}), 0)


RELEASE_LIMITS = {"cold_start_s": 0.4, "first_reply_s": 0.06, "idle_rss_mb": 80.0, "after_reply_rss_mb": 80.0,
                  "after_search_rss_mb": 100.0, "browser_launch_s": 2.4}


def release_file(limits=RELEASE_LIMITS, blocking=None):
    """A limits file with the debug limits at the top and a "release" object, as perf_budget.json has."""
    return {"limits": LIMITS, "blocking": ["idle_rss_mb"],
            "release": {"limits": dict(limits), "blocking": sorted(blocking or pb.DEFAULT_RELEASE_BLOCKING)}}


def release_results(idle, profile="release"):
    return {"profile": profile, "when": "2026-11-26T18:00:00+00:00",
            "samples": {"cold_start_s": [0.3] * 5, "first_reply_s": [0.05] * 5, "idle_rss_mb": idle,
                        "after_reply_rss_mb": [70.0, 71.0, 69.0], "after_search_rss_mb": [90.0, 91.0, 89.0],
                        "browser_launch_s": [9.3] * 5}}


class ReleaseProfile(unittest.TestCase):
    """perf-release-budget: the release build has its own limits; a release run is never judged by the debug ones."""

    def run_main(self, argv, limits_data, files=None):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            lim = Path(d) / "limits.json"
            lim.write_text(json.dumps(limits_data))
            paths = []
            for i, data in enumerate(files or []):
                f = Path(d) / f"perf-results-{i}.json"
                f.write_text(json.dumps(data))
                paths.append(str(f))
            args = [a.format(*paths) for a in argv]
            with mock.patch.object(sys, "argv", ["perf_budget.py", *args, "--limits", str(lim)]), mock.patch("builtins.print"):
                code = pb.main()
            return code, json.loads(lim.read_text())

    def test_debug_is_the_top_level_and_release_its_own_object(self):
        data = release_file()
        self.assertIs(pb.section_of(data, "debug"), data)
        self.assertEqual(pb.section_of(data, "release")["limits"], RELEASE_LIMITS)
        self.assertEqual(pb.blocking_of(pb.section_of(data, "release")), pb.DEFAULT_RELEASE_BLOCKING)

    def test_a_file_without_release_limits_is_an_error_not_a_pass(self):
        for data in ({"limits": LIMITS}, {"limits": LIMITS, "release": {}}, {"limits": LIMITS, "release": {"limits": {}}}):
            with self.assertRaises(ValueError):
                pb.section_of(data, "release")
        with self.assertRaises(ValueError):
            pb.section_of(release_file(), "profiling")
        # a release run (and a release --judge) with such a file stops with exit 2 before it measures anything
        self.assertEqual(self.run_main(["--profile", "release", "--kwnt", "/nonexistent/kwnt"], {"limits": LIMITS})[0], 2)
        self.assertEqual(self.run_main(["--judge", "{0}"], {"limits": LIMITS}, [release_results([60.0] * 3)])[0], 2)

    def test_judge_over_a_release_limit_exits_1_and_at_it_exits_0(self):
        self.assertEqual(self.run_main(["--judge", "{0}"], release_file(), [release_results([79.0, 80.0, 80.5])])[0], 0)
        self.assertEqual(self.run_main(["--judge", "{0}"], release_file(), [release_results([80.0, 81.0, 80.5])])[0], 1)

    def test_a_saved_release_file_is_judged_by_the_release_limits_even_when_it_passes_the_debug_ones(self):
        # idle 100 MB is under the debug 150 but over the release 80: a release file must fail
        self.assertEqual(self.run_main(["--judge", "{0}"], release_file(), [release_results([100.0] * 3)])[0], 1)
        # --profile overrides what the file says; a file without a profile is debug (files from before this PR)
        self.assertEqual(self.run_main(["--judge", "{0}", "--profile", "debug"], release_file(), [release_results([100.0] * 3)])[0], 0)
        old = release_results([100.0] * 3)
        del old["profile"]
        self.assertEqual(self.run_main(["--judge", "{0}"], release_file(), [old])[0], 0)

    def test_the_browser_only_warns_in_release_too(self):
        # browser 9.3 s is over 2.4 in every file above, yet they exit 0: it is not in the release blocking list
        self.assertNotIn("browser_launch_s", pb.DEFAULT_RELEASE_BLOCKING)

    def test_limits_are_the_median_of_the_runs_plus_15_percent_rounded_up(self):
        runs = [{"idle_rss_mb": 60.2, "cold_start_s": 0.31, "first_reply_s": 0.05},
                {"idle_rss_mb": 64.9, "cold_start_s": 0.29, "first_reply_s": 0.04},
                {"idle_rss_mb": 62.0, "cold_start_s": 0.30, "first_reply_s": None}]
        baseline, limits = pb.release_limits(runs)
        self.assertEqual(baseline["idle_rss_mb"], 62.0)
        self.assertEqual(limits["idle_rss_mb"], 72.0)        # 62.0 x 1.15 = 71.3 -> 72
        self.assertEqual(limits["cold_start_s"], 0.35)      # 0.30 x 1.15 = 0.345 -> 0.35
        self.assertEqual(limits["first_reply_s"], 0.06)     # median of 0.05 and 0.04 = 0.045 x 1.15 = 0.05175 -> 0.06
        self.assertNotIn("after_search_rss_mb", limits)      # no run measured it: no limit is made up
        for key, limit in limits.items():
            self.assertGreaterEqual(limit, baseline[key] * pb.RELEASE_HEADROOM - 1e-9, key)

    def test_set_release_limits_writes_only_the_release_object(self):
        data = release_file()
        data["release"]["note"] = "kept"
        files = [release_results([60.0, 61.0, 62.0]), release_results([64.0] * 3), release_results([63.0] * 3)]
        code, out = self.run_main(["--set-release-limits", "{0}", "{1}", "{2}"], data, files)
        self.assertEqual(code, 0)
        self.assertEqual(out["limits"], LIMITS)              # the debug limits are untouched
        self.assertEqual(out["blocking"], ["idle_rss_mb"])
        self.assertEqual(out["release"]["note"], "kept")
        self.assertEqual(out["release"]["baseline"]["idle_rss_mb"], 63.0)
        self.assertEqual(out["release"]["limits"]["idle_rss_mb"], 73.0)   # 63 x 1.15 = 72.45 -> 73
        self.assertEqual(out["release"]["limits"]["after_reply_rss_mb"], 81.0)  # 70 x 1.15 = 80.5 -> 81
        self.assertEqual(len(out["release"]["baseline_from"]["files"]), 3)

    def test_set_release_limits_refuses_debug_results(self):
        files = [release_results([60.0] * 3), release_results([60.0] * 3, profile="debug"), release_results([60.0] * 3)]
        code, out = self.run_main(["--set-release-limits", "{0}", "{1}", "{2}"], release_file(), files)
        self.assertEqual(code, 2)
        self.assertEqual(out["release"]["limits"], RELEASE_LIMITS)

    def test_update_baseline_is_debug_only(self):
        with self.assertRaises(SystemExit):
            with mock.patch("sys.stderr"):
                self.run_main(["--profile", "release", "--kwnt", "/x", "--update-baseline"], release_file())


class ReleaseRunPaths(unittest.TestCase):
    """The perf-release job runs on a PR only when it can change the release build's numbers."""

    def test_rust_cargo_and_the_budget_itself_run_it(self):
        for path in ("src/mcp/directory.rs", "Cargo.toml", "Cargo.lock", "build.rs", "scripts/perf_budget.py",
                     "scripts/perf_budget.json", ".github/workflows/ci.yml"):
            self.assertTrue(pb.needs_release_run([path]), path)

    def test_docs_frontend_and_other_workflows_skip_it(self):
        for path in ("docs/mcp-api.md", "frontend/src/App.tsx", "frontend/src/main.rs", "apps/mac/Cargo.toml",
                     ".github/workflows/mac.yml", "scripts/fresh_machine_test.py", "srcx/a.rs", "README.md"):
            self.assertFalse(pb.needs_release_run([path]), path)
        self.assertFalse(pb.needs_release_run([]))
        self.assertFalse(pb.needs_release_run(["", "  "]))

    def test_one_matching_path_among_many_is_enough(self):
        self.assertTrue(pb.needs_release_run(["docs/a.md", "frontend/x.ts", "src/main.rs"]))

    def test_the_cli_reads_paths_on_stdin_and_prints_true_or_false(self):
        import io
        for stdin, want in (("docs/a.md\nsrc/lib.rs\n", "true"), ("docs/a.md\nfrontend/x.ts\n", "false"), ("", "false")):
            with mock.patch.object(sys, "argv", ["perf_budget.py", "--needs-release-run"]), \
                    mock.patch("sys.stdin", io.StringIO(stdin)), mock.patch("builtins.print") as printed:
                self.assertEqual(pb.main(), 0)
            printed.assert_called_once_with(want)


class ShippedReleaseJob(unittest.TestCase):
    def ci_job(self):
        ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
        job = re.search(r"\n  perf-release:\n(.*?)(?=\n  \w[\w-]*:\n|\Z)", ci, re.S)
        self.assertTrue(job, "no perf-release job in ci.yml")
        return job.group(1)

    def test_the_job_builds_release_and_judges_it_as_release(self):
        job = self.ci_job()
        self.assertIn("cargo build --release --locked --bin kwnt", job)
        self.assertIn("scripts/perf_budget.py --profile release --kwnt target/release/kwnt", job)
        self.assertIn("--needs-release-run", job)
        self.assertNotIn("continue-on-error", job)  # blocking from the start

    def test_the_release_limits_block_the_five_measures_and_are_no_looser_than_debug(self):
        data = json.loads((ROOT / "scripts" / "perf_budget.json").read_text())
        release = pb.section_of(data, "release")
        self.assertEqual(pb.blocking_of(release), pb.DEFAULT_RELEASE_BLOCKING)
        for key in pb.DEFAULT_RELEASE_BLOCKING:
            self.assertIn(key, release["limits"], key)
            self.assertLessEqual(release["limits"][key], data["limits"][key], key)
        for key, limit in release["limits"].items():
            self.assertIn(key, pb.MEASURES)
            if key in release.get("baseline", {}):
                self.assertGreaterEqual(limit, release["baseline"][key], key)


class FakeMcpRegistry(unittest.TestCase):
    """after_search_rss_mb syncs kwnt's copy from this fake registry: it pages like the real one and searches by name."""

    @classmethod
    def setUpClass(cls):
        cls.reg = pb.FakeRegistry(entries=1200, page=500)

    @classmethod
    def tearDownClass(cls):
        cls.reg.close()

    def get(self, query):
        import urllib.request
        with urllib.request.urlopen(f"{self.reg.url}/v0.1/servers?{query}") as r:
            return json.loads(r.read())

    def test_pages_follow_next_cursor_through_every_entry(self):
        names, cursor, pages = [], None, 0
        while True:
            page = self.get("version=latest&limit=100" + (f"&cursor={cursor}" if cursor else ""))
            names += [s["server"]["name"] for s in page["servers"]]
            pages += 1
            cursor = page["metadata"].get("nextCursor")
            if not cursor:
                break
        self.assertEqual((pages, len(names), len(set(names))), (3, 1200, 1200))

    def test_search_and_updated_since(self):
        hits = self.get("search=stripe&version=latest&limit=30")["servers"]
        self.assertTrue(hits)
        self.assertTrue(all("stripe" in h["server"]["name"] for h in hits))
        self.assertEqual(self.get("version=latest&updated_since=2026-11-26T00:00:00Z")["servers"], [])

    def test_entries_are_one_click_remote_or_npm_and_deterministic(self):
        kinds = {("remotes" in pb.fake_registry_entry(i)["server"], "packages" in pb.fake_registry_entry(i)["server"]) for i in range(6)}
        self.assertEqual(kinds, {(True, False), (False, True)})
        self.assertEqual(pb.fake_registry_entry(42), pb.fake_registry_entry(42))
        self.assertTrue(all("{" not in r["url"] for i in range(30) for r in pb.fake_registry_entry(i)["server"].get("remotes", [])))
        self.assertLess(len(json.dumps({"servers": [pb.fake_registry_entry(i) for i in range(pb.FAKE_REGISTRY_PAGE)]})), 2 * 1024 * 1024,
                        "a page fits kwnt's 2 MB answer limit")


class DirectoryLogLines(unittest.TestCase):
    def test_only_the_directory_lines_without_colours(self):
        log = ("\x1b[2m2026-11-26T15:00:00Z\x1b[0m \x1b[32m INFO\x1b[0m kwnt::mcp::directory: MCP directory synced listed=75000\n"
               "some other line\n"
               " INFO kwnt::mcp::directory: MCP directory memory (before the sync -> after it -> after giving freed memory back)"
               " key_text_mb=5.61 in_use_mb=20.1 -> 60.3 -> 28.4\n")
        lines = pb.directory_lines(log)
        self.assertEqual(len(lines), 2)
        self.assertTrue(lines[0].startswith("2026-11-26T15:00:00Z  INFO"), lines[0])
        self.assertIn("key_text_mb=5.61", lines[1])
        self.assertNotIn("\x1b", "".join(lines))
        self.assertEqual(pb.directory_lines(""), [])


class BarePathHasWhatChromesLauncherNeeds(unittest.TestCase):
    def test_readlink_and_touch_are_on_the_bare_path(self):
        # /usr/bin/google-chrome is a shell script: on the CI runner it died with "readlink: command not found" and
        # "touch: command not found" under the run's bare PATH, and Chrome never opened its debugging port
        for tool in ("readlink", "touch"):
            self.assertIn(tool, pb.SYSTEM_TOOLS)


if __name__ == "__main__":
    unittest.main()
