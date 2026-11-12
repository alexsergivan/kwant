#!/usr/bin/env python3
"""P3: the performance budget. Measures kwnt's own speed and memory and fails when a median goes over its limit.

Every run starts the built binary the way scripts/fresh_machine_test.py does (`kwnt serve --no-laya`, `env -i`,
an empty temp HOME and data folder, a free port) with a fake `claude` CLI on PATH that answers at once, so what is
timed is kwnt, not a model or the network. Per run it takes:

  cold_start_s      spawn until GET /api/health answers 200
  first_reply_s     create a bot, send it one message, and time from the send until the first reply text
                    (a TextDelta) arrives on /api/events, the stream the app itself listens to
  idle_rss_mb       kwnt's VmRSS (/proc/<pid>/status) --idle-seconds after startup
  after_reply_rss_mb  the same, --idle-seconds after that first reply
  browser_launch_s  a bot opens its browser (local Chrome, launched cold) and loads a page; only when a Chrome or
                    Chromium is found, otherwise `SKIP browser: no browser` (a missing browser never fails). When a
                    Chrome is found but does not come up (kwnt reports it could not start Chrome, and it still cannot
                    within --browser-wait seconds, default 60), the row is `SKIP` with the reason and the run goes on
                    (a runner where Chrome cannot start is not a performance regression, so the browser is dropped for
                    the remaining runs). A Chrome that does start and is slower than its limit still FAILS.

Memory is read in the first --memory-runs runs only (each costs 2 x --idle-seconds of waiting); the other measures
are taken in all --runs. The median of the runs is compared with scripts/perf_budget.json. A median over its limit
FAILS (exit 1); a median at its limit passes.

Usage:
  python3 scripts/perf_budget.py --kwnt target/debug/kwnt [--browser /usr/bin/chromium] [--out perf-results.json]
  python3 scripts/perf_budget.py --kwnt ... --update-baseline     # rewrite perf_budget.json from this run (on purpose)
  python3 scripts/perf_budget.py --kwnt ... --limits /tmp/tiny.json   # try other limits, e.g. to see a FAIL

Limits: baseline x 1.5 for times, x 1.3 for memory, rounded up (see compute_limits). Linux only (reads /proc).
Needs no packages: standard library only.
"""
import argparse
import datetime
import http.client
import json
import math
import os
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

LIMITS_FILE = Path(__file__).with_name("perf_budget.json")
FORBIDDEN_PORTS = {8799, 8800, 8810, 8852, 8931}
SYSTEM_TOOLS = ["sh", "bash", "cat", "mkdir", "chmod", "sleep", "which", "env", "uname", "dirname", "rm", "cp", "mv", "ls"]

CHROME_DOWN_MARKER = "could not start kwant's Chrome"  # what kwnt's 503 says when Chrome never opens its debugging port


class ChromeDidNotStart(Exception):
    """kwnt's Chrome did not come up on this machine within the wait. Not a performance result: the row is skipped."""


# measure id -> (what it is, unit, factor from baseline to limit, decimals the limit is rounded up to)
MEASURES = {
    "cold_start_s": ("cold start (spawn to /api/health 200)", "s", 1.5, 1),
    "first_reply_s": ("first reply (send to first text, fake claude)", "s", 1.5, 1),
    "idle_rss_mb": ("idle memory, 30 s after start", "MB", 1.3, 0),
    "after_reply_rss_mb": ("idle memory, 30 s after first reply", "MB", 1.3, 0),
    "browser_launch_s": ("bot browser launch (cold Chrome to page open)", "s", 1.5, 1),
}

# What the fake claude prints: signed in, then a reply streamed at once. Same idea as FAKE_CLAUDE in
# tests/turn_recovery_test.rs.
FAKE_CLAUDE = """#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
  --version) echo "2.0.0 (Claude Code)"; exit 0 ;;
esac
echo '{"type":"system","subtype":"init"}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello from the fake claude."}}}'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Hello from the fake claude."}]}}'
echo '{"type":"result","is_error":false,"num_turns":1,"result":"Hello from the fake claude."}'
"""


# ---- The comparison logic (unit-tested in test_perf_budget.py; needs no binary) -------------------------------------
def median(values):
    """The median of the numbers, or None when there are none."""
    values = [v for v in values if v is not None]
    return statistics.median(values) if values else None


def round_up(value, decimals):
    """Rounds up (never down): 0.31 -> 0.4 at 1 decimal, 100.2 -> 101 at 0. Guards float noise (0.3 stays 0.3)."""
    scale = 10 ** decimals
    return math.ceil(round(value * scale, 6)) / scale if decimals else float(math.ceil(round(value, 6)))


def compute_limits(baseline):
    """A limit per measure from its baseline: x1.5 for times, x1.3 for memory, rounded up."""
    return {k: round_up(v * MEASURES[k][2], MEASURES[k][3]) for k, v in baseline.items() if k in MEASURES and v is not None}


def judge(medians, limits, skipped=None):
    """One row per measure: {id, median, limit, status, reason}. Status is PASS (at or under the limit), FAIL (over),
    SKIP (not measured, e.g. no browser, or Chrome would not start; `skipped` maps a measure id to why) or NO LIMIT
    (measured, but perf_budget.json has no limit for it: not a failure). `reason` is set on a SKIP row only."""
    rows = []
    skipped = skipped or {}
    for key in MEASURES:
        med, limit = medians.get(key), limits.get(key)
        reason = None
        if med is None:
            status = "SKIP"
            reason = skipped.get(key)
        elif limit is None:
            status = "NO LIMIT"
        else:
            status = "PASS" if med <= limit else "FAIL"
        rows.append({"id": key, "median": med, "limit": limit, "status": status, "reason": reason})
    return rows


def table(rows):
    """The small text table printed at the end."""
    lines = [f"{'measure':<46} {'median':>9} {'limit':>9}  result"]
    for r in rows:
        what, unit = MEASURES[r["id"]][0], MEASURES[r["id"]][1]
        med = "-" if r["median"] is None else f"{r['median']:.2f} {unit}"
        lim = "-" if r["limit"] is None else f"{r['limit']:g} {unit}"
        lines.append(f"{what:<46} {med:>9} {lim:>9}  {r['status']}")
        if r.get("reason"):
            lines.append(f"    {r['status']} {r['id']}: {r['reason']}")
    return "\n".join(lines)


def summary_markdown(rows):
    """The same table as Markdown, for the GitHub job summary ($GITHUB_STEP_SUMMARY); SKIP rows carry their reason."""
    out = ["### Performance budget", "", "| measure | median | limit | result |", "|---|---|---|---|"]
    for r in rows:
        what, unit = MEASURES[r["id"]][0], MEASURES[r["id"]][1]
        med = "-" if r["median"] is None else f"{r['median']:.2f} {unit}"
        lim = "-" if r["limit"] is None else f"{r['limit']:g} {unit}"
        out.append(f"| {what} | {med} | {lim} | {r['status']} |")
    out += [f"\n**{r['status']} {r['id']}**: {r['reason']}" for r in rows if r.get("reason")]
    return "\n".join(out) + "\n"


def exit_code(rows):
    """1 when any median is over its limit, else 0."""
    return 1 if any(r["status"] == "FAIL" for r in rows) else 0


# ---- Running kwnt ---------------------------------------------------------------------------------------------------
def free_port():
    while True:
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
        s.close()
        if port not in FORBIDDEN_PORTS:
            return port


def request(port, method, path, body=None, timeout=30):
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    try:
        conn.request(method, path, json.dumps(body) if body is not None else None, {"Content-Type": "application/json"})
        resp = conn.getresponse()
        raw = resp.read()
        try:
            return resp.status, json.loads(raw)
        except ValueError:
            return resp.status, raw.decode(errors="replace")
    finally:
        conn.close()


def find_browser(explicit):
    for cand in [explicit, os.environ.get("CHROMIUM"), "google-chrome", "google-chrome-stable", "chromium", "chromium-browser"]:
        if cand and (shutil.which(cand) or Path(cand).is_file()):
            return shutil.which(cand) or cand
    return None


def make_machine(root: Path):
    """An empty home and data folder, a bare PATH (a few system tools) and the fake claude."""
    home, binp, data = root / "home", root / "bin", root / "data"
    for d in (home, binp, data):
        d.mkdir(parents=True)
    for tool in SYSTEM_TOOLS:
        src = shutil.which(tool)
        if src:
            (binp / tool).symlink_to(src)
    fake = binp / "claude"
    fake.write_text(FAKE_CLAUDE)
    fake.chmod(0o755)
    return home, binp, data


def rss_mb(pid):
    """kwnt's resident memory in MB (VmRSS), or None when /proc has no such process."""
    try:
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) / 1024
    except OSError:
        return None
    return None


class Events:
    """Listens to /api/events (the stream the app uses) and notes when the first reply text of a bot arrives."""

    def __init__(self, port):
        self.port, self.kwant, self.first_text, self.error = port, None, None, None
        self.ready = threading.Event()
        self.seen = threading.Event()
        self.conn = None
        threading.Thread(target=self._run, daemon=True).start()

    def _run(self):
        try:
            self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=120)
            self.conn.request("GET", "/api/events")
            resp = self.conn.getresponse()
            self.ready.set()
            while True:
                line = resp.readline()
                if not line:
                    return
                if not line.startswith(b"data:"):
                    continue
                try:
                    event = json.loads(line[5:])
                except ValueError:
                    continue
                if event.get("type") == "TextDelta" and event.get("kwant_id") == self.kwant and not self.first_text:
                    self.first_text = time.perf_counter()
                    self.seen.set()
        except Exception as e:  # noqa: BLE001  (the run reports it)
            self.error = repr(e)
            self.ready.set()

    def close(self):
        if self.conn:
            self.conn.close()


def wait_health(port, seconds):
    end = time.perf_counter() + seconds
    while time.perf_counter() < end:
        try:
            if request(port, "GET", "/api/health", timeout=2)[0] == 200:
                return time.perf_counter()
        except OSError:
            pass
        time.sleep(0.01)
    raise TimeoutError(f"/api/health did not answer in {seconds}s")


def open_browser_page(port, kid, wait):
    """Time from asking the bot to open a page to the page being open (cold Chrome). kwnt answers 503 ("could not start
    kwant's Chrome ...") when Chrome never opens its debugging port; on that the call is repeated until `wait` seconds
    have passed since the first ask, and if Chrome is still down raises ChromeDidNotStart with kwnt's reason. Any other
    failure (another status, a page that did not open) is a real failure and raises RuntimeError."""
    t_nav = time.perf_counter()
    reason = None
    while True:
        code, page = request(port, "POST", f"/api/sandbox/{kid}/navigate", {"url": f"http://127.0.0.1:{port}/api/health"}, timeout=90)
        if code == 200 and isinstance(page, dict) and page.get("status") == "navigated":
            return time.perf_counter() - t_nav
        error = page.get("error", "") if isinstance(page, dict) else str(page)
        if code == 503 and CHROME_DOWN_MARKER in error:
            reason = error
            if time.perf_counter() - t_nav >= wait:
                raise ChromeDidNotStart(f"{reason} (no debugging port after {time.perf_counter() - t_nav:.0f} s)")
            time.sleep(1)
            continue
        raise RuntimeError(f"the bot browser did not open a page: {code} {str(page)[:200]}")


def one_run(kwnt, browser, idle_seconds, take_memory, log, browser_wait=60):
    """One fresh kwnt: {measure id: value}. Raises on a failure of kwnt itself (not of a limit). When the bot browser's
    Chrome never comes up, the other measures are kept and the result carries "browser_skip": why (the caller then
    drops the browser from the remaining runs)."""
    root = Path(tempfile.mkdtemp(prefix="perf-budget-"))
    home, binp, data = make_machine(root)
    port = free_port()
    env = {"HOME": str(home), "PATH": str(binp), "KWNT_DATA_DIR": str(data), "KWNT_HOST": "127.0.0.1", "KWNT_PORT": str(port)}
    if browser:
        env["KWNT_COMPUTER_MODE"] = "local"
        env["KWNT_CHROME_PATH"] = browser
    out = {}
    t0 = time.perf_counter()
    proc = subprocess.Popen([str(kwnt), "serve", "--no-laya"], env=env, stdout=open(root / "kwnt.log", "w"),
                            stderr=subprocess.STDOUT, cwd=str(root), start_new_session=True)
    events = None
    try:
        out["cold_start_s"] = wait_health(port, 60) - t0
        t_up = time.perf_counter()

        code, bot = request(port, "POST", "/api/kwants", {"name": "Clio", "title": "Tester", "persona_prompt": "Test persona",
                                                            "avatar_sprite": "s", "driver_type": "api", "preferred_model": "claude"})
        if code not in (200, 201) or not isinstance(bot, dict) or "id" not in bot:
            raise RuntimeError(f"creating the bot failed: {code} {bot}")
        kid = bot["id"]
        events = Events(port)
        events.kwant = kid
        if not events.ready.wait(10) or events.error:
            raise RuntimeError(f"could not listen to /api/events: {events.error}")
        t_send = time.perf_counter()
        code, sent = request(port, "POST", "/api/chat/send", {"kwant_id": kid, "prompt": "hi", "sender_id": "user"})
        if code != 200:
            raise RuntimeError(f"sending the message failed: {code} {sent}")
        if not events.seen.wait(30):
            raise RuntimeError("no reply text arrived in 30 s")
        out["first_reply_s"] = events.first_text - t_send

        if take_memory:
            # 30 s after startup, then 30 s after the reply (measured from when each happened)
            time.sleep(max(0.0, t_up + idle_seconds - time.perf_counter()))
            out["idle_rss_mb"] = rss_mb(proc.pid)
            time.sleep(max(0.0, events.first_text + idle_seconds - time.perf_counter()))
            out["after_reply_rss_mb"] = rss_mb(proc.pid)

        if browser:
            try:
                out["browser_launch_s"] = open_browser_page(port, kid, browser_wait)
            except ChromeDidNotStart as e:
                out["browser_skip"] = str(e)
        return out
    except Exception:
        log(f"---- kwnt log (last 20 lines) ----\n" + "\n".join((root / "kwnt.log").read_text(errors="replace").splitlines()[-20:]))
        raise
    finally:
        try:
            os.killpg(proc.pid, signal.SIGTERM)
            proc.wait(10)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        if events:
            events.close()  # after kwnt stopped: closing a stream that is still open waits for it
        if browser:
            # kwnt's Chrome keeps running when kwnt stops: end everything that still names this run's folder
            subprocess.run(["pkill", "-f", "--", str(root)], check=False)
        shutil.rmtree(root, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kwnt", required=True, help="path to the kwnt binary")
    ap.add_argument("--browser", help="Chrome or Chromium for the browser measure (default: found on PATH; none = skipped)")
    ap.add_argument("--runs", type=int, default=5, help="runs per measure (default 5)")
    ap.add_argument("--memory-runs", type=int, default=3, help="runs that also read memory (default 3; each waits 2 x idle-seconds)")
    ap.add_argument("--idle-seconds", type=float, default=30, help="how long kwnt idles before memory is read (default 30)")
    ap.add_argument("--browser-wait", type=float, default=60, help="how long to wait for Chrome's debugging port before the browser row is skipped (default 60)")
    ap.add_argument("--limits", default=str(LIMITS_FILE), help="the limits file (default scripts/perf_budget.json)")
    ap.add_argument("--out", default="perf-results.json", help="where the results are written")
    ap.add_argument("--update-baseline", action="store_true", help="rewrite the limits file from this run (on purpose, not in CI)")
    args = ap.parse_args()

    kwnt = Path(args.kwnt).resolve()
    if not kwnt.is_file():
        sys.exit(f"no kwnt binary at {kwnt}")
    if not sys.platform.startswith("linux"):
        sys.exit("perf_budget.py reads /proc, so it runs on Linux only")
    limits_path = Path(args.limits)
    limits = json.loads(limits_path.read_text()).get("limits", {}) if limits_path.is_file() else {}
    browser = find_browser(args.browser)
    skipped = {}
    if not browser:
        skipped["browser_launch_s"] = "no browser"
        print("SKIP browser: no browser", flush=True)
    found_browser = browser

    runs = []
    for i in range(args.runs):
        print(f"run {i + 1}/{args.runs} ...", flush=True)
        runs.append(one_run(kwnt, browser, args.idle_seconds, i < args.memory_runs, lambda m: print(m, flush=True), args.browser_wait))
        print("  " + "  ".join(f"{k}={v:.3f}" for k, v in runs[-1].items() if isinstance(v, float)), flush=True)
        if runs[-1].get("browser_skip"):
            skipped["browser_launch_s"] = runs[-1]["browser_skip"]
            print(f"SKIP browser: {skipped['browser_launch_s']}", flush=True)
            browser = None  # Chrome cannot start here: the remaining runs do not try it again

    medians = {k: median([r.get(k) for r in runs]) for k in MEASURES}
    rows = judge(medians, limits, skipped)
    print("\n" + table(rows))

    results = {
        "when": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "kwnt": str(kwnt), "runs": args.runs, "memory_runs": args.memory_runs, "idle_seconds": args.idle_seconds,
        "browser": found_browser, "browser_skip": skipped.get("browser_launch_s"), "measures": rows, "samples": {k: [r.get(k) for r in runs] for k in MEASURES},
    }
    Path(args.out).write_text(json.dumps(results, indent=2) + "\n")
    print(f"\nwrote {args.out}")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(summary_markdown(rows))

    if args.update_baseline:
        baseline = {k: round(v, 3) for k, v in medians.items() if v is not None}
        limits_path.write_text(json.dumps({
            "note": "Limits are the baseline x 1.5 (memory x 1.3), rounded up. Rewrite with `perf_budget.py --update-baseline` on purpose.",
            "baseline": baseline,
            "baseline_from": {"date": results["when"], "runs": args.runs, "kwnt": kwnt.name},
            "limits": compute_limits(baseline),
        }, indent=2) + "\n")
        print(f"rewrote {limits_path}")
        return 0
    code = exit_code(rows)
    print("performance budget: " + ("FAILED, a median is over its limit" if code else "ok"))
    return code


if __name__ == "__main__":
    sys.exit(main())
