#!/usr/bin/env python3
"""O9: the new-computer test. Starts kwnt as a brand-new user and checks the first minutes.

The "new computer" is made like this:
  * HOME and the data folder are empty temp folders.
  * kwnt runs with `env -i` (nothing from the runner leaks in) and a bare PATH: a folder of symlinks to the few
    system tools an installer needs (sh, bash, cat, ...), plus a stand-in `curl`. No claude, codex, grok, node,
    npm, brew, apt, docker.
  * The one-click install runs for real (kwnt's own `/api/setup/install`), but the download is a stand-in: the
    fake curl serves a local install.sh for https://claude.ai/install.sh and REFUSES every other URL. The real
    download is never made. The installer drops a fake `claude` into ~/.local/bin that is signed out and prints a
    sign-in link when asked to log in.

Checks (each prints PASS or FAIL):
  1. the machine is really new: no AI tool is installed, and the Setup list offers to install Claude;
  2. the Get started card shows (a headless Chromium opens the page, scripts/fresh_machine_page.mjs) at "Choose your AI",
     with a Claude tile that says neither "Installed" nor "Signed in";
  3. the one-click install starts, finishes, ran only the stand-in download, and kwnt then sees Claude (signed out);
     on a fresh page load the Claude tile says "Installed · sign in next";
  4. the sign-in starts: kwnt asks for a link and returns an allowed claude.ai/claude.com link in the
     waiting-for-code state (the sign-in is then cancelled).

Usage:
  python3 scripts/fresh_machine_test.py --kwnt target/debug/kwnt [--browser /usr/bin/chromium] [--keep]
Exit 0 when every check passes. Needs frontend/dist embedded in the kwnt binary (the CI job builds it first), node, and playwright-core
(PW_CORE=/path/to/playwright-core/index.mjs, or `npm i playwright-core` somewhere node finds it).
What it cannot tell you: that claude.ai's real install.sh still works (the stand-in replaces it), that Laya installs
(`serve --no-laya`: 420 MB), or what a person sees in the Tauri window (it loads the same page this test opens).

The page checks look for Iris's "Get started" card (O2, 0.7.6; GetStarted.tsx), set in CARD below.
"""
import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

# ---- What the checks look for in the first screen: the Get started card (GetStarted.tsx). ----
CARD = {
    # CSS selector of the first-run card (GetStarted.tsx's <section data-get-started>)
    "selector": "[data-get-started]",
    # its first step's title, shown while no AI is chosen yet
    "step_label": "Choose your AI",
    # the Claude tile inside step 1 (one <button data-ai-tile=...> per AI)
    "tile": '[data-ai-tile="claude"]',
    # what a tile must not say on a new machine (Claude isn't installed, nobody is signed in)
    "not_yet": ("Installed", "Signed in"),
    # what the Claude tile says once Claude Code is installed but signed out
    "installed_label": "Installed · sign in next",
}
# ------------------------------------------------------------------------------------------------------------

FORBIDDEN_PORTS = {8799, 8800, 8810, 8852, 8931}
# system tools the stand-in installer and kwnt's own checks use, linked into the bare PATH
SYSTEM_TOOLS = ["sh", "bash", "cat", "mkdir", "chmod", "sleep", "which", "env", "uname", "dirname", "rm", "cp", "mv", "ls"]
INSTALL_URL = "https://claude.ai/install.sh"
ALLOWED_LOGIN_HOSTS = ("claude.ai", "claude.com")

results = []


def check(name, ok, detail=""):
    results.append((name, ok))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  ({detail})" if detail and not ok else ""), flush=True)
    return ok


def free_port():
    while True:
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
        s.close()
        if port not in FORBIDDEN_PORTS:
            return port


def http(base, method, path, body=None, timeout=20):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(base + path, data=data, method=method, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw[:1] in (b"{", b"[") else raw.decode(errors="replace"))
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw)
        except ValueError:
            return e.code, raw.decode(errors="replace")


def wait_for(fn, what, seconds):
    end = time.time() + seconds
    last = None
    while time.time() < end:
        try:
            last = fn()
        except Exception as e:  # noqa: BLE001  (server still starting)
            last = e
        if last and not isinstance(last, Exception):
            return last
        time.sleep(0.5)
    raise TimeoutError(f"timed out after {seconds}s: {what} (last: {last})")


def make_machine(root: Path):
    """The empty home, the bare PATH and the stand-in download."""
    home, binp, standin, data = root / "home", root / "bin", root / "standin", root / "data"
    for d in (home, binp, standin, data):
        d.mkdir(parents=True)
    for tool in SYSTEM_TOOLS:
        src = shutil.which(tool)
        if src:
            (binp / tool).symlink_to(src)
    curl_log = root / "curl.log"
    (standin / "install.sh").write_text(
        """#!/bin/bash
set -eu
echo "stand-in Claude Code installer (fresh-machine test, not the real download)"
mkdir -p "$HOME/.local/bin"
cat > "$HOME/.local/bin/claude" <<'CLAUDE'
#!/bin/sh
# stand-in Claude Code: signed out; "auth login" prints a link and waits for a code
case "$1" in
  --version) echo "2.0.0 (Claude Code)" ;;
  auth)
    case "$2" in
      status) echo '{"loggedIn":false}' ;;
      login)
        echo "Opening your browser to sign in..."
        echo "If it didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&client_id=fresh-machine-test"
        sleep 60 ;;
      *) exit 0 ;;
    esac ;;
  *) exit 0 ;;
esac
CLAUDE
chmod 755 "$HOME/.local/bin/claude"
echo "installed claude into $HOME/.local/bin"
"""
    )
    # The stand-in curl: the installer URL gets the local file, anything else is refused (and logged).
    (binp / "curl").write_text(
        f"""#!/bin/sh
for a in "$@"; do url="$a"; done
echo "$url" >> "{curl_log}"
if [ "$url" = "{INSTALL_URL}" ]; then cat "{standin}/install.sh"; exit 0; fi
echo "fresh-machine test: refused to download $url" >&2
exit 22
"""
    )
    (binp / "curl").chmod(0o755)
    return home, binp, data, curl_log


def first_card(browser, url, expect=None):
    """{"found": bool, "text": str, "tile": bool, "tileText": str}: the Get started card and its Claude tile,
    read after the page's engine list arrived."""
    helper = Path(__file__).with_name("fresh_machine_page.mjs")
    env = {"HOME": tempfile.gettempdir(), "PATH": os.environ.get("PATH", "/usr/bin:/bin"), "CHROMIUM": browser}
    if os.environ.get("PW_CORE"):
        env["PW_CORE"] = os.environ["PW_CORE"]
    out = subprocess.run(["node", str(helper), url, CARD["selector"], CARD["tile"], *([expect] if expect else [])],
                         capture_output=True, text=True, timeout=120, env=env)
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError):
        return {"found": False, "text": "", "error": (out.stderr or out.stdout)[-300:]}


def find_browser(explicit):
    for cand in [explicit, os.environ.get("CHROMIUM"), "google-chrome", "google-chrome-stable", "chromium", "chromium-browser"]:
        if cand and (shutil.which(cand) or Path(cand).is_file()):
            return shutil.which(cand) or cand
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kwnt", required=True, help="path to the kwnt binary (with the web UI embedded)")
    ap.add_argument("--browser", help="Chromium or Chrome (default: CHROMIUM, google-chrome, chromium)")
    ap.add_argument("--port", type=int, help="port to serve on (default: a free one)")
    ap.add_argument("--keep", action="store_true", help="keep the temp folder and print its path")
    args = ap.parse_args()

    kwnt = Path(args.kwnt).resolve()
    if not kwnt.is_file():
        sys.exit(f"no kwnt binary at {kwnt}")
    browser = find_browser(args.browser)
    if not browser:
        sys.exit("no Chromium or Chrome found (use --browser or CHROMIUM)")
    port = args.port or free_port()
    if port in FORBIDDEN_PORTS:
        sys.exit(f"port {port} is reserved")

    root = Path(tempfile.mkdtemp(prefix="fresh-machine-"))
    home, binp, data, curl_log = make_machine(root)
    log = root / "kwnt.log"
    base = f"http://127.0.0.1:{port}"
    # `env -i`: only these variables exist for kwnt
    env = {"HOME": str(home), "PATH": str(binp), "KWNT_DATA_DIR": str(data), "KWNT_HOST": "127.0.0.1", "KWNT_PORT": str(port)}
    proc = subprocess.Popen([str(kwnt), "serve", "--no-laya"], env=env, stdout=open(log, "w"), stderr=subprocess.STDOUT, cwd=str(root))
    try:
        try:
            wait_for(lambda: http(base, "GET", "/api/health")[0] == 200, "kwnt to answer /api/health", 60)
        except TimeoutError as e:
            check("kwnt starts on an empty HOME", False, str(e))
            return finish(root, log, proc, args.keep)
        check("kwnt starts on an empty HOME", True)

        # 1. A really new machine
        code, status = http(base, "GET", "/api/setup/status")
        items = {i["id"]: i for i in status["items"]} if code == 200 and isinstance(status, dict) else {}
        none_installed = bool(items) and all(items[k].get("state") != "ready" for k in ("claude", "codex", "grok") if k in items)
        check("new machine: no AI tool is installed", none_installed, f"setup status {code}: {json.dumps(items.get('claude'))[:200]}")
        check("new machine: Setup offers to install Claude Code by itself", items.get("claude", {}).get("can_install") is True,
              json.dumps(items.get("claude"))[:300])

        # 2. First screen: the Get started card
        card = first_card(browser, base + "/")
        check(f"first run: the Get started card shows ({CARD['selector']})", card["found"], card.get("error", ""))
        check(f"first run: the card is at \"{CARD['step_label']}\"", CARD["step_label"] in card["text"], card["text"][:200])
        check(f"first run: the Claude tile is there ({CARD['tile']})", card.get("tile") is True, card["text"][:200])
        check("first run: the Claude tile says neither \"Installed\" nor \"Signed in\"",
              card.get("tile") is True and not any(w in card["tileText"] for w in CARD["not_yet"]), card.get("tileText", ""))

        # 3. One-click install against the stand-in download
        code, started = http(base, "POST", "/api/setup/install", {"item": "claude"})
        ok_start = code == 202 and isinstance(started, dict) and started.get("job_id")
        check("install: the one-click install starts (202 with a job)", bool(ok_start), f"{code} {started}")
        if ok_start:
            job_id, lines, after, job = started["job_id"], [], 0, {}
            try:
                def poll():
                    nonlocal after, job
                    c, j = http(base, "GET", f"/api/setup/jobs/{job_id}?after={after}")
                    if c != 200:
                        return None
                    lines.extend(j.get("lines", []))
                    after, job = j["next"], j
                    return j["state"] in ("done", "failed")
                wait_for(poll, "the install job to finish", 120)
            except TimeoutError as e:
                check("install: it finishes", False, str(e))
            else:
                check("install: it finishes without an error", job.get("state") == "done", f"{job.get('state')}: {job.get('error')} | {' / '.join(lines[-4:])}")
            requested = curl_log.read_text().split() if curl_log.exists() else []
            check("install: only the stand-in download ran (no other URL was asked for)", requested == [INSTALL_URL], f"asked for: {requested}")
            claude_bin = home / ".local" / "bin" / "claude"
            check("install: Claude Code landed in ~/.local/bin", claude_bin.is_file() and os.access(claude_bin, os.X_OK))
            code, engines = http(base, "GET", "/api/engines")
            eng = {e.get("id"): e for e in engines} if code == 200 and isinstance(engines, list) else \
                {e.get("id"): e for e in (engines.get("engines", []) if isinstance(engines, dict) else [])}
            claude_state = (eng.get("claude") or {}).get("status")
            check("install: kwnt sees Claude Code at once, signed out", claude_state == "signed_out", f"engine claude = {json.dumps(eng.get('claude'))[:200]}")
            card = first_card(browser, base + "/", CARD["installed_label"])
            check(f"install: on a fresh page load the Claude tile says \"{CARD['installed_label']}\"",
                  CARD["installed_label"] in card.get("tileText", ""), card.get("tileText") or card["text"][:200])

        # 4. The sign-in starts
        code, sess = http(base, "POST", "/api/engines/claude/login")
        sid = sess.get("session_id") if isinstance(sess, dict) else None
        check("sign-in: it starts (a session comes back)", code == 200 and bool(sid), f"{code} {sess}")
        if sid:
            def link():
                c, s = http(base, "GET", f"/api/engines/claude/login/{sid}")
                return s if c == 200 and s.get("state") == "waiting_for_code" and s.get("url") else None
            try:
                s = wait_for(link, "the sign-in link", 30)
                host = re.match(r"https://([^/:]+)/", s["url"])
                check("sign-in: kwnt shows the sign-in link and waits for the code",
                      bool(host) and host.group(1) in ALLOWED_LOGIN_HOSTS, s["url"])
            except TimeoutError as e:
                check("sign-in: kwnt shows the sign-in link and waits for the code", False, str(e))
            http(base, "DELETE", f"/api/engines/claude/login/{sid}")
    finally:
        pass
    return finish(root, log, proc, args.keep)


def finish(root, log, proc, keep):
    proc.terminate()
    try:
        proc.wait(10)
    except subprocess.TimeoutExpired:
        proc.kill()
    failed = [n for n, ok in results if not ok]
    if failed and log.exists():
        print("---- kwnt log (last 40 lines) ----")
        print("\n".join(log.read_text(errors="replace").splitlines()[-40:]).replace(str(root), "<tmp>"))
    print(f"\nfresh-machine: {len(results) - len(failed)}/{len(results)} checks passed")
    if keep:
        print(f"kept {root}")
    else:
        shutil.rmtree(root, ignore_errors=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
