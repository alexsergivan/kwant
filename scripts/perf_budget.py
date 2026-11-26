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
  after_search_rss_mb the same, --idle-seconds after Find a tool's first search has synced kwnt's copy of the MCP
                    Registry. The registry is a fake one this script serves on 127.0.0.1 (FAKE_REGISTRY_ENTRIES
                    one-click entries, the real registry's size on Nov 26, 2026), so no network is used and every run
                    holds the same copy. perf-idle-memory A2: a full copy used to cost ~150 MB until a restart.
  browser_launch_s  a bot opens its browser (local Chrome, launched cold) and loads a page; only when a Chrome or
                    Chromium is found, otherwise `SKIP browser: no browser` (a missing browser never fails). When a
                    Chrome is found but does not come up (kwnt reports it could not start Chrome, and it still cannot
                    within --browser-wait seconds, default 60), the row is `SKIP` with the reason and the run goes on
                    (a runner where Chrome cannot start is not a performance regression, so the browser is dropped for
                    the remaining runs). A Chrome that does start and is slower than its limit still FAILS.

Memory is read in the first --memory-runs runs only (each costs 2 x --idle-seconds of waiting); the other measures
are taken in all --runs. The median of the runs is compared with scripts/perf_budget.json. A median over its limit
FAILS; a median at its limit passes. Only a FAIL in a *blocking* measure (perf_budget.json "blocking") exits 1 and turns
CI red; a measure that is not blocking (browser_launch_s for now) shows WARN instead and exits 0.

Usage:
  python3 scripts/perf_budget.py --kwnt target/debug/kwnt [--browser /usr/bin/chromium] [--out perf-results.json]
  python3 scripts/perf_budget.py --kwnt ... --update-baseline     # rewrite perf_budget.json from this run (on purpose)
  python3 scripts/perf_budget.py --kwnt ... --limits /tmp/tiny.json   # try other limits, e.g. to see a FAIL
  python3 scripts/perf_budget.py --judge perf-results.json            # judge a saved results file again (no kwnt run),
                                                                      # e.g. a doctored one, to prove a FAIL exits 1
  python3 scripts/perf_budget.py --profile release --kwnt target/release/kwnt   # the release build, against the
                                                                      # "release" limits in perf_budget.json
  python3 scripts/perf_budget.py --set-release-limits a.json b.json c.json      # release limits from 3 CI runs
  git diff --name-only BASE HEAD | python3 scripts/perf_budget.py --needs-release-run   # prints true or false

Profiles (perf-release-budget, 0.9.56): CI measures two builds. `debug` (the fresh-machine job's target/debug/kwnt) is
judged against the top-level "limits" and "blocking" of perf_budget.json; `release` (the perf-release job's
`cargo build --release`, what people run) against the "release" object's own "limits" and "blocking". A release run
whose limits file has no "release" limits stops with exit 2 instead of passing unjudged. --judge uses the profile the
saved file was measured with unless --profile says otherwise.

Limits: debug, baseline x 1.5 for times, x 1.3 for memory, rounded up (see compute_limits); release, the median of 3
CI runs' medians + 15%, rounded up to 0.01 s / 1 MB (see release_limits). Linux only (reads /proc).
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
# readlink and touch (and the rest of the second row) are what Google Chrome's /usr/bin/google-chrome launcher script calls:
# without them it died with `readlink: command not found` (exit 127) on the CI runner, so Chrome never opened its port.
SYSTEM_TOOLS = ["sh", "bash", "cat", "mkdir", "chmod", "sleep", "which", "env", "uname", "dirname", "rm", "cp", "mv", "ls",
                "readlink", "touch", "basename", "grep", "sed", "tr", "head", "id"]

CHROME_DOWN_MARKER = "could not start kwant's Chrome"  # what kwnt's 503 says when Chrome never opens its debugging port


class ChromeDidNotStart(Exception):
    """kwnt's Chrome did not come up on this machine within the wait. Not a performance result: the row is skipped."""


# measure id -> (what it is, unit, factor from baseline to limit, decimals the limit is rounded up to)
MEASURES = {
    "cold_start_s": ("cold start (spawn to /api/health 200)", "s", 1.5, 1),
    "first_reply_s": ("first reply (send to first text, fake claude)", "s", 1.5, 1),
    "idle_rss_mb": ("idle memory, 30 s after start", "MB", 1.3, 0),
    "after_reply_rss_mb": ("idle memory, 30 s after first reply", "MB", 1.3, 0),
    "after_search_rss_mb": ("memory, 30 s after Find a tool synced (75k)", "MB", 1.3, 0),
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


# ---- A fake MCP Registry for after_search_rss_mb --------------------------------------------------------------------
FAKE_REGISTRY_ENTRIES = 75_000  # the real registry listed 74,866 latest entries on Nov 26, 2026
FAKE_REGISTRY_PAGE = 500        # bigger pages than the real 100 (kwnt follows nextCursor), so a sync takes seconds
WORDS = ("stripe notion github slack linear jira gmail drive sheets calendar postgres redis docker figma zoom "
         "weather maps search browser files memory notes tasks crm sales email chat pdf image audio video code "
         "deploy cloud vault billing invoice ledger trading crypto news sports travel recipes fitness health").split()


def fake_registry_entry(i):
    """One registry answer item, varied like the real ones: 2 in 3 remote (streamable HTTP), 1 in 3 an npm package
    with a secret and a plain env value. Deterministic, so every run syncs the same copy."""
    a, b, c = WORDS[i % len(WORDS)], WORDS[(i * 7 + 3) % len(WORDS)], WORDS[(i * 13 + 5) % len(WORDS)]
    owner = f"{a}{b}{i % 9973}"
    name = f"io.github.{owner}/{c}-{a}-mcp-{i}" if i % 2 else f"com.{owner}/{b}-{c}-{i}"
    server = {
        "name": name,
        "title": f"{a.title()} {b.title()} {c} tools {i}",
        "description": f"Connects {a} and {b} to your assistant: read, search and update {c} items, with typed "
                       f"results and paging. Fake entry {i} for kwnt's performance budget.",
        "version": f"1.{i % 50}.{i % 7}",
        "repository": {"url": f"https://github.com/{owner}/{c}-mcp", "source": "github"},
        "websiteUrl": f"https://{owner}.example.com/{c}",
    }
    if i % 3:
        server["remotes"] = [{"type": "streamable-http", "url": f"https://mcp.{owner}.example.com/{c}/mcp"}]
    else:
        server["packages"] = [{
            "registryType": "npm", "registryBaseUrl": "https://registry.npmjs.org", "identifier": f"@{owner}/{c}-mcp",
            "version": f"1.{i % 50}.{i % 7}", "runtimeHint": "npx", "transport": {"type": "stdio"},
            "environmentVariables": [
                {"name": f"{c.upper()}_API_KEY", "description": f"Your {c} API key, from the {a} settings page.",
                 "isRequired": True, "isSecret": True},
                {"name": f"{c.upper()}_REGION", "description": "Which region to use.", "isRequired": False},
            ],
        }]
    meta = {"io.modelcontextprotocol.registry/official": {
        "status": "active", "publishedAt": "2026-10-01T00:00:00Z", "updatedAt": "2026-10-01T00:00:00Z", "isLatest": True}}
    return {"server": server, "_meta": meta}


class FakeRegistry:
    """Serves GET /v0.1/servers like the MCP Registry: pages by cursor, `search=` by name, and nothing changed for
    `updated_since`. Pages are made once, up front, and shared by every run."""

    def __init__(self, entries=FAKE_REGISTRY_ENTRIES, page=FAKE_REGISTRY_PAGE):
        import http.server
        from urllib.parse import parse_qs, urlparse
        items = [fake_registry_entry(i) for i in range(entries)]
        pages = []
        for start in range(0, entries, page):
            nxt = str(start + page) if start + page < entries else None
            meta = {"count": len(items[start:start + page])} | ({"nextCursor": nxt} if nxt else {})
            pages.append(json.dumps({"servers": items[start:start + page], "metadata": meta}).encode())
        names = [(it["server"]["name"].lower(), it) for it in items]
        empty = json.dumps({"servers": [], "metadata": {"count": 0}}).encode()

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                url = urlparse(self.path)
                q = {k: v[0] for k, v in parse_qs(url.query).items()}
                if url.path.rstrip("/") != "/v0.1/servers":
                    body, code = b'{"error":"not found"}', 404
                elif "search" in q:
                    hits = [it for n, it in names if q["search"].lower() in n][:30]
                    body, code = json.dumps({"servers": hits, "metadata": {"count": len(hits)}}).encode(), 200
                elif "updated_since" in q:
                    body, code = empty, 200
                else:
                    start = int(q.get("cursor") or 0)
                    body, code = pages[start // page] if start // page < len(pages) else empty, 200
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()


def directory_lines(log_text):
    """kwnt's "MCP directory" log lines (sync, memory), without terminal colours: the allocator's numbers for the
    after-search memory go into perf-results.json next to the RSS readings."""
    import re
    plain = re.sub(r"\x1b\[[0-9;]*m", "", log_text)
    return [line.strip() for line in plain.splitlines() if "MCP directory" in line][-6:]


def sync_directory(port, seconds=240):
    """Find a tool's first search, then the same search until it answers from kwnt's synced copy. Returns when it does."""
    path = "/api/mcp/registry/search?q=stripe"
    t0 = time.perf_counter()
    while True:
        code, answer = request(port, "GET", path)
        if code != 200 or not isinstance(answer, dict):
            raise RuntimeError(f"Find a tool's search failed: {code} {str(answer)[:200]}")
        if answer.get("source") == "copy":
            if not answer.get("entries"):
                raise RuntimeError("Find a tool's synced copy found nothing for 'stripe'")
            return time.perf_counter() - t0
        if time.perf_counter() - t0 > seconds:
            raise RuntimeError(f"Find a tool's copy did not sync within {seconds} s")
        time.sleep(1)


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


PROFILES = ("debug", "release")
DEFAULT_RELEASE_BLOCKING = {"cold_start_s", "first_reply_s", "idle_rss_mb", "after_reply_rss_mb", "after_search_rss_mb"}
RELEASE_HEADROOM = 1.15                      # release limits: the median of 3 CI runs + 15% (Ada's brief, Nov 26)
RELEASE_DECIMALS = {"s": 2, "MB": 0}         # rounded up to 0.01 s and to whole MB
# A pull request runs the perf-release job only when it touches one of these (a release build adds ~10+ minutes).
# Pushes to main always run it. Prefixes end in "/" (a folder) or match the start of the path.
RELEASE_RUN_PATHS = ("src/", "Cargo.toml", "Cargo.lock", "build.rs", "scripts/perf_budget.", ".github/workflows/ci.yml")


def section_of(limits_file_data, profile):
    """The part of perf_budget.json a profile is judged against: the top level for debug, the "release" object for
    release. A release section without limits raises ValueError: a release run must never pass unjudged."""
    if profile not in PROFILES:
        raise ValueError(f"unknown profile {profile!r} (one of {', '.join(PROFILES)})")
    if profile == "debug":
        return limits_file_data
    section = limits_file_data.get(profile)
    if not isinstance(section, dict) or not section.get("limits"):
        raise ValueError(f'the limits file has no "{profile}" limits')
    return section


def release_limits(run_medians):
    """Release limits from several CI runs: for each measure, the median of the runs' medians x RELEASE_HEADROOM,
    rounded up (times to 0.01 s, memory to 1 MB). A measure no run has is left out. Returns (baseline, limits)."""
    baseline, limits = {}, {}
    for key, (_, unit, _, _) in MEASURES.items():
        med = median([m.get(key) for m in run_medians])
        if med is None:
            continue
        baseline[key] = round(med, 3)
        limits[key] = round_up(med * RELEASE_HEADROOM, RELEASE_DECIMALS[unit])
    return baseline, limits


def medians_of_results(results):
    """The per-measure medians of one saved perf-results.json (its raw samples, so a re-judge never trusts old rows)."""
    samples = results.get("samples", {})
    return {k: median(samples.get(k) or []) for k in MEASURES}


def needs_release_run(paths):
    """True when one of the changed paths can change the release build's numbers (RELEASE_RUN_PATHS)."""
    return any(p.strip().startswith(RELEASE_RUN_PATHS) for p in paths if p.strip())


def blocking_of(limits_file_data):
    """The measures whose FAIL blocks CI: perf_budget.json's "blocking" list. A file without the key blocks on every
    measure (the old behaviour), so an older limits file is never silently weaker. Unknown ids are ignored."""
    listed = limits_file_data.get("blocking")
    return set(MEASURES) if listed is None else {k for k in listed if k in MEASURES}


def judge(medians, limits, skipped=None, blocking=None):
    """One row per measure: {id, median, limit, status, reason, blocking}. Status is PASS (at or under the limit), FAIL
    (over, in a blocking measure), WARN (over, in a measure that does not block), SKIP (not measured, e.g. no browser,
    or Chrome would not start; `skipped` maps a measure id to why) or NO LIMIT (measured, but perf_budget.json has no
    limit for it: not a failure). `reason` is set on a SKIP row only. `blocking` defaults to every measure."""
    rows = []
    skipped = skipped or {}
    blocking = set(MEASURES) if blocking is None else set(blocking)
    for key in MEASURES:
        med, limit = medians.get(key), limits.get(key)
        reason = None
        if med is None:
            status = "SKIP"
            reason = skipped.get(key)
        elif limit is None:
            status = "NO LIMIT"
        else:
            status = "PASS" if med <= limit else ("FAIL" if key in blocking else "WARN")
        rows.append({"id": key, "median": med, "limit": limit, "status": status, "reason": reason, "blocking": key in blocking})
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


def summary_markdown(rows, profile="debug"):
    """The same table as Markdown, for the GitHub job summary ($GITHUB_STEP_SUMMARY); SKIP rows carry their reason."""
    title = "### Performance budget" + ("" if profile == "debug" else f" ({profile} build)")
    out = [title, "", "| measure | median | limit | result |", "|---|---|---|---|"]
    for r in rows:
        what, unit = MEASURES[r["id"]][0], MEASURES[r["id"]][1]
        med = "-" if r["median"] is None else f"{r['median']:.2f} {unit}"
        lim = "-" if r["limit"] is None else f"{r['limit']:g} {unit}"
        out.append(f"| {what} | {med} | {lim} | {r['status']} |")
    out += [f"\n**{r['status']} {r['id']}**: {r['reason']}" for r in rows if r.get("reason")]
    return "\n".join(out) + "\n"


def exit_code(rows):
    """1 when a blocking measure's median is over its limit (a FAIL row), else 0. WARN rows never fail the run."""
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


def rss_split(status_text):
    """{"anon_mb", "file_mb"} from a /proc/<pid>/status text: the heap and other private memory (RssAnon) and the
    mapped files, mostly kwnt's own executable (RssFile). Not judged: it tells a heap regression from a bigger binary
    (perf-idle-memory). A field the kernel doesn't show is None."""
    out = {"anon_mb": None, "file_mb": None}
    for line in status_text.splitlines():
        for field, key in (("RssAnon:", "anon_mb"), ("RssFile:", "file_mb")):
            if line.startswith(field):
                out[key] = round(int(line.split()[1]) / 1024, 1)
    return out


def rss_split_of(pid):
    """rss_split for a running process; None when /proc has no such process."""
    try:
        return rss_split(Path(f"/proc/{pid}/status").read_text())
    except OSError:
        return None


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


def _read(path):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return "n/a"


def probe_chrome(browser, extra=(), seconds=20, env=None):
    """Starts `browser` the way kwnt's local Chrome starts (same flags, a fresh profile) but with its stderr kept, and
    says whether it opened a debugging port. Returns (opened, exit code or None, stderr tail). Never raises."""
    root = Path(tempfile.mkdtemp(prefix="perf-chrome-probe-"))
    err = root / "stderr.txt"
    proc = None
    try:
        args = [browser, f"--user-data-dir={root / 'p'}", "--remote-debugging-port=0", "--remote-debugging-address=127.0.0.1",
                "--no-first-run", "--no-default-browser-check", "--headless=new", "--window-size=1280,800", *extra, "about:blank"]
        proc = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=open(err, "w"), start_new_session=True, env=env)
        opened, code = False, None
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if (root / "p" / "DevToolsActivePort").exists():
                opened = True
                break
            code = proc.poll()
            if code is not None:
                break
            time.sleep(0.25)
        tail = "\n".join(err.read_text(errors="replace").splitlines()[-12:])
        return opened, code, tail
    except Exception as e:  # a diagnosis must never turn a SKIP into a crash
        return False, None, f"probe failed: {e}"
    finally:
        if proc:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass


def diagnose_chrome(browser):
    """Why Chrome may not start here, for the log: the OS, the AppArmor switch for unprivileged user namespaces, and
    Chrome's own stderr with kwnt's flags and again with --no-sandbox (kwnt throws Chrome's stderr away)."""
    lines = [f"os: {next((l.split('=', 1)[1].strip(chr(34)) for l in _read('/etc/os-release').splitlines() if l.startswith('PRETTY_NAME=')), 'n/a')}",
             f"kernel.apparmor_restrict_unprivileged_userns = {_read('/proc/sys/kernel/apparmor_restrict_unprivileged_userns')}",
             f"kernel.unprivileged_userns_clone = {_read('/proc/sys/kernel/unprivileged_userns_clone')}",
             f"user.max_user_namespaces = {_read('/proc/sys/user/max_user_namespaces')}",
             f"chrome: {browser}"]
    try:
        lines.append("version: " + subprocess.run([browser, "--version"], capture_output=True, text=True, timeout=20).stdout.strip())
    except Exception as e:
        lines.append(f"version: failed ({e})")
    machine = Path(tempfile.mkdtemp(prefix="perf-chrome-env-"))
    home, binp, _ = make_machine(machine)
    bare = {"HOME": str(home), "PATH": str(binp)}  # exactly what kwnt (and so the Chrome it starts) ran with
    lines.append(f"bare PATH holds: {' '.join(sorted(p.name for p in binp.iterdir()))}")
    for label, extra, env in (("kwnt's flags, the runner's own environment", (), None),
                              ("kwnt's flags, the bare environment of the run (HOME and PATH only)", (), bare),
                              ("kwnt's flags plus --no-sandbox, the bare environment", ("--no-sandbox",), bare)):
        opened, code, tail = probe_chrome(browser, extra, env=env)
        lines.append(f"{label}: " + ("opened its debugging port" if opened else f"NO debugging port (exit {code})"))
        if tail and not opened:
            lines.append("  stderr: " + tail.replace("\n", "\n          "))
    return "\n".join(lines)


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


def one_run(kwnt, browser, idle_seconds, take_memory, log, browser_wait=60, registry_url=None):
    """One fresh kwnt: {measure id: value}. Raises on a failure of kwnt itself (not of a limit). When the bot browser's
    Chrome never comes up, the other measures are kept and the result carries "browser_skip": why (the caller then
    drops the browser from the remaining runs)."""
    root = Path(tempfile.mkdtemp(prefix="perf-budget-"))
    home, binp, data = make_machine(root)
    port = free_port()
    env = {"HOME": str(home), "PATH": str(binp), "KWNT_DATA_DIR": str(data), "KWNT_HOST": "127.0.0.1", "KWNT_PORT": str(port)}
    if registry_url:
        env["KWNT_MCP_REGISTRY_URL"] = registry_url
        # Only the MCP directory's own lines: its memory line says where the after-search memory is
        env["RUST_LOG"] = "error,kwnt::mcp::directory=info"
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
            out["idle_rss_split"] = rss_split_of(proc.pid)
            time.sleep(max(0.0, events.first_text + idle_seconds - time.perf_counter()))
            out["after_reply_rss_mb"] = rss_mb(proc.pid)
            out["after_reply_rss_split"] = rss_split_of(proc.pid)
            if registry_url:
                # Find a tool's first search syncs the whole (fake) registry; read memory 30 s after the copy is in
                out["directory_sync_s"] = sync_directory(port)
                time.sleep(idle_seconds)
                out["after_search_rss_mb"] = rss_mb(proc.pid)
                out["after_search_rss_split"] = rss_split_of(proc.pid)
                out["directory_log"] = directory_lines((root / "kwnt.log").read_text(errors="replace"))

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
    ap.add_argument("--kwnt", help="path to the kwnt binary (required unless --judge)")
    ap.add_argument("--judge", metavar="RESULTS_JSON", help="judge a saved perf-results.json against the limits again, without running kwnt")
    ap.add_argument("--browser", help="Chrome or Chromium for the browser measure (default: found on PATH; none = skipped)")
    ap.add_argument("--runs", type=int, default=5, help="runs per measure (default 5)")
    ap.add_argument("--memory-runs", type=int, default=3, help="runs that also read memory (default 3; each waits 2 x idle-seconds)")
    ap.add_argument("--idle-seconds", type=float, default=30, help="how long kwnt idles before memory is read (default 30)")
    ap.add_argument("--browser-wait", type=float, default=60, help="how long to wait for Chrome's debugging port before the browser row is skipped (default 60)")
    ap.add_argument("--limits", default=str(LIMITS_FILE), help="the limits file (default scripts/perf_budget.json)")
    ap.add_argument("--out", default="perf-results.json", help="where the results are written")
    ap.add_argument("--no-registry", action="store_true", help="skip after_search_rss_mb (no fake MCP Registry)")
    ap.add_argument("--update-baseline", action="store_true", help="rewrite the limits file from this run (on purpose, not in CI; debug only)")
    ap.add_argument("--profile", choices=PROFILES, help="which build this is, so which limits apply (default debug; --judge: the saved file's)")
    ap.add_argument("--set-release-limits", nargs="+", metavar="RESULTS_JSON", help="write the \"release\" limits from saved CI runs (median + 15%%)")
    ap.add_argument("--needs-release-run", action="store_true", help="read changed paths on stdin, print true when the perf-release job should run")
    args = ap.parse_args()
    if args.needs_release_run:
        print("true" if needs_release_run(sys.stdin.read().splitlines()) else "false")
        return 0
    limits_path = Path(args.limits)
    limits_data = json.loads(limits_path.read_text()) if limits_path.is_file() else {}
    if args.set_release_limits:
        return set_release_limits(limits_path, limits_data, [Path(f) for f in args.set_release_limits])
    if args.judge:
        saved = json.loads(Path(args.judge).read_text())
        profile = args.profile or saved.get("profile") or "debug"
        return judge_saved(Path(args.judge), saved, profile, limits_data)
    profile = args.profile or "debug"
    if args.update_baseline and profile != "debug":
        ap.error("--update-baseline rewrites the debug limits only; use --set-release-limits for release")
    try:
        section = section_of(limits_data, profile)
    except ValueError as e:
        print(f"performance budget: {e} ({limits_path})", flush=True)
        return 2
    limits, blocking = section.get("limits", {}), blocking_of(section)
    if not args.kwnt:
        ap.error("--kwnt is required (or --judge a saved results file)")

    kwnt = Path(args.kwnt).resolve()
    if not kwnt.is_file():
        sys.exit(f"no kwnt binary at {kwnt}")
    if not sys.platform.startswith("linux"):
        sys.exit("perf_budget.py reads /proc, so it runs on Linux only")
    browser = find_browser(args.browser)
    skipped = {}
    diagnosis = None
    if not browser:
        skipped["browser_launch_s"] = "no browser"
        print("SKIP browser: no browser", flush=True)
    found_browser = browser
    registry = None
    if args.memory_runs > 0 and not args.no_registry:
        print(f"fake MCP Registry: {FAKE_REGISTRY_ENTRIES} entries ...", flush=True)
        registry = FakeRegistry()
    else:
        skipped["after_search_rss_mb"] = "no fake registry (--no-registry)"

    runs = []
    for i in range(args.runs):
        print(f"run {i + 1}/{args.runs} ...", flush=True)
        runs.append(one_run(kwnt, browser, args.idle_seconds, i < args.memory_runs, lambda m: print(m, flush=True), args.browser_wait,
                            registry.url if registry else None))
        print("  " + "  ".join(f"{k}={v:.3f}" for k, v in runs[-1].items() if isinstance(v, float)), flush=True)
        if runs[-1].get("browser_skip"):
            skipped["browser_launch_s"] = runs[-1]["browser_skip"]
            print(f"SKIP browser: {skipped['browser_launch_s']}", flush=True)
            diagnosis = diagnose_chrome(browser)
            print("---- why Chrome did not start (diagnosis) ----\n" + diagnosis, flush=True)
            browser = None  # Chrome cannot start here: the remaining runs do not try it again

    if registry:
        registry.close()
    medians = {k: median([r.get(k) for r in runs]) for k in MEASURES}
    rows = judge(medians, limits, skipped, blocking)
    print(f"\n{profile} build\n" + table(rows))

    results = {
        "when": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "profile": profile, "kwnt": str(kwnt), "runs": args.runs, "memory_runs": args.memory_runs, "idle_seconds": args.idle_seconds,
        "browser": found_browser, "browser_skip": skipped.get("browser_launch_s"), "chrome_diagnosis": diagnosis, "measures": rows, "samples": {k: [r.get(k) for r in runs] for k in MEASURES},
        "memory_split": {k: [r.get(f"{k}_rss_split") for r in runs[:args.memory_runs]] for k in ("idle", "after_reply", "after_search")},
        "directory_sync_s": [r.get("directory_sync_s") for r in runs[:args.memory_runs]],
        "directory_log": [r.get("directory_log") for r in runs[:args.memory_runs]],
    }
    Path(args.out).write_text(json.dumps(results, indent=2) + "\n")
    print(f"\nwrote {args.out}")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(summary_markdown(rows, profile))
            if diagnosis:
                f.write("\n<details><summary>Why Chrome did not start</summary>\n\n```\n" + diagnosis + "\n```\n</details>\n")

    if args.update_baseline:
        baseline = {k: round(v, 3) for k, v in medians.items() if v is not None}
        limits_path.write_text(json.dumps({
            "note": "Limits are the baseline x 1.5 (memory x 1.3), rounded up. Rewrite with `perf_budget.py --update-baseline` on purpose.",
            "baseline": baseline,
            "baseline_from": {"date": results["when"], "runs": args.runs, "kwnt": kwnt.name},
            "limits": compute_limits(baseline),
            "blocking": sorted(blocking),
        }, indent=2) + "\n")
        print(f"rewrote {limits_path}")
        return 0
    return report_exit(rows)


def report_exit(rows):
    code = exit_code(rows)
    warned = [r["id"] for r in rows if r["status"] == "WARN"]
    print("performance budget: " + ("FAILED, a blocking median is over its limit" if code else "ok")
          + (f" (warn only, not blocking: {', '.join(warned)})" if warned else ""))
    return code


def judge_saved(path, saved, profile, limits_data):
    """--judge: the medians of a saved results file's samples, judged against today's limits for `profile`. Runs nothing."""
    try:
        section = section_of(limits_data, profile)
    except ValueError as e:
        print(f"performance budget: {e}", flush=True)
        return 2
    medians = medians_of_results(saved)
    skipped = {"browser_launch_s": saved["browser_skip"]} if saved.get("browser_skip") else {}
    rows = judge(medians, section.get("limits", {}), skipped, blocking_of(section))
    print(f"judging {path} as the {profile} build (no kwnt run)\n" + table(rows))
    return report_exit(rows)


def set_release_limits(limits_path, limits_data, result_files):
    """--set-release-limits: the "release" limits from saved CI runs of the release build. Keeps the section's note
    and blocking list; every other part of the file is left as it is."""
    saved = [json.loads(f.read_text()) for f in result_files]
    wrong = [str(f) for f, r in zip(result_files, saved) if r.get("profile") != "release"]
    if wrong:
        print(f"not release-build results (profile != release): {', '.join(wrong)}", flush=True)
        return 2
    baseline, limits = release_limits([medians_of_results(r) for r in saved])
    section = dict(limits_data.get("release") or {})
    section.setdefault("blocking", sorted(DEFAULT_RELEASE_BLOCKING))
    section["baseline"] = baseline
    section["baseline_from"] = {"files": [f.name for f in result_files], "when": [r.get("when") for r in saved],
                                "rule": f"median of the runs' medians x {RELEASE_HEADROOM}, rounded up"}
    section["limits"] = limits
    limits_data["release"] = section
    limits_path.write_text(json.dumps(limits_data, indent=2) + "\n")
    print(f"rewrote the release limits in {limits_path}: {json.dumps(limits)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
