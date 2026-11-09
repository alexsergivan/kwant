#!/usr/bin/env python3
"""Proactive-team eval: with approvals off, does a team act, look first, and deliver what was asked?

The acceptance test for the 0.8.0 "P" items (release-080/proactive/brief.md). It replays the kinds of
messages from Oleks's booktun.ing chat (2026-11-09) on a tiny book-playlist site:
- a team of two bots (Frontend, Growth) works in the site's git repo;
- approvals are off;
- the claude CLI runs the turns;
- a blind judge (also the claude CLI) scores each reply.
Hard checks on app state override the judge:
- an image was shared in the chat;
- the model tier the reply ran on;
- the reply offers a step instead of taking it;
- the commit was pushed;
- the change uses the site's tokens.

Scenarios (each on a fresh server, team and repo, so they can run one at a time):
  1 ideas      "Hey team! What some cool feature we can add?"     → ideas from the real site, with mockup images
  2 see        "i want to see this ---> a sticky bar…"            → a picture, not a decision to park it
  3 followup   "Screenshot give me!" after a look at the page     → an image, not on the Fast tier
  4 fix        "the Surprise me button is not aligned … Fix it"   → fixed with the site's tokens, committed, pushed
  5 dark       "show me the home page in dark mode"               → a dark screenshot (the site uses prefers-color-scheme)
  6 twice      two messages sent back to back                     → replies that don't contradict each other

Usage:
  scripts/eval_proactive.py --bin /tmp/ada-proactive/kwnt-before --label before --port 18971 --out /tmp/ada-proactive/eval
  scripts/eval_proactive.py --bin /tmp/ada-proactive/kwnt-after  --label after  --port 18972 --out /tmp/ada-proactive/eval
  scripts/eval_proactive.py --compare /tmp/ada-proactive/eval/before/results.json /tmp/ada-proactive/eval/after/results.json

Options:
- `--only 1,3` runs some scenarios.
- `--keep` adds to an earlier results.json, so the scenarios can be run one per call.

Needs a signed-in `claude` on PATH, and a Chrome for the bots' browser (KWNT_CHROME_PATH, or the
packaged /usr/bin/chromium; a Playwright one is refused where AppArmor blocks its sandbox). `--out` must be under
/tmp: it is wiped.
"""

import argparse
import glob
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import textwrap
import time
import urllib.error
import urllib.request

TZ = "Europe/Berlin"
TEAM = "Shelftune"

# Phrases that offer a step the bot could take itself (approvals are off). Any one fails "acted".
OFFERS = [
    r"\bif you (want|like|'d like)\b[^.?!\n]{0,40}\bI (can|could|'ll)\b",
    r"\bsay the word\b",
    r"\buntil you approve\b",
    r"\bwithout your (ok|okay|approval|go-ahead)\b",
    r"\b(want|would you like) me to\b",
    r"\bshall I\b",
    r"\bshould I (go ahead|proceed|push|commit|build)\b",
    r"\blet me know if you('d| would) like\b",
    r"\bwaiting for your (ok|okay|approval|go-ahead)\b",
]


def find_chrome():
    if os.environ.get("KWNT_CHROME_PATH"):
        return os.environ["KWNT_CHROME_PATH"]
    # Packaged browsers first: kwnt refuses a Playwright Chromium where Linux blocks its sandbox (AppArmor).
    for pat in ("/usr/bin/chromium", "/usr/bin/google-chrome", "~/.cache/ms-playwright/chromium-*/chrome-linux*/chrome"):
        hits = sorted(glob.glob(os.path.expanduser(pat)))
        if hits:
            return hits[-1]
    return None


# ---------- the site the team works on ----------

STYLES = """\
/* Shelftune design tokens: reuse these, never hard-code colours or fonts */
:root {
  --primary-600: #d96716;
  --primary-700: #b8530f;
  --ink: #2b2118;
  --muted: #7a6a5a;
  --paper: #fbf6ef;
  --card: #ffffff;
  --radius: 14px;
  --font-display: "Fraunces", Georgia, serif;
  --font-body: "Inter", system-ui, sans-serif;
  --shadow-card: 0 2px 0 #e8dccb, 0 8px 24px rgba(43, 33, 24, .08);
}
@media (prefers-color-scheme: dark) {
  :root {
    --ink: #f3eadf;
    --muted: #b9a993;
    --paper: #1d1712;
    --card: #2a221b;
    --shadow-card: 0 2px 0 #120e0b, 0 8px 24px rgba(0, 0, 0, .4);
  }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--paper); color: var(--ink); font: 16px/1.5 var(--font-body); }
header { display: flex; justify-content: space-between; align-items: center; padding: 16px 24px; }
.logo { font: 700 1.4rem var(--font-display); }
.hero { max-width: 720px; margin: 32px auto; padding: 0 20px; text-align: center; }
.hero h1 { font: 700 2.4rem/1.15 var(--font-display); margin: 0 0 12px; }
.hero p { color: var(--muted); }
.search { display: flex; gap: 10px; margin: 24px 0 12px; }
.search input { flex: 1; padding: .8rem 1rem; border: 2px solid #e8dccb; border-radius: var(--radius); font: inherit; background: var(--card); color: var(--ink); }
.btn-primary { background: var(--primary-600); color: #fff; border: 0; border-radius: var(--radius); padding: .8rem 1.2rem;
  font: 600 1rem var(--font-body); box-shadow: 0 3px 0 var(--primary-700); cursor: pointer; }
.btn-primary:active { transform: translateY(2px); box-shadow: 0 1px 0 var(--primary-700); }
/* Added in a hurry for the "Surprise me" test; not on the design tokens yet */
.btn-surprise { background: #b63e2d; color: #fff; border: 0; border-radius: 4px; padding: .7rem 1rem; font: 500 .95rem Arial, sans-serif; }
.cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 16px; max-width: 920px; margin: 32px auto; padding: 0 20px; }
.card { background: var(--card); border-radius: var(--radius); box-shadow: var(--shadow-card); padding: 16px; }
.card h3 { font: 600 1.05rem var(--font-display); margin: 0 0 4px; }
.card small { color: var(--muted); }
footer { text-align: center; color: var(--muted); padding: 32px; }
"""

INDEX = """\
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Shelftune: turn a book into a playlist</title>
  <link rel="stylesheet" href="static/css/styles.css">
</head>
<body>
  <header><span class="logo">Shelftune</span><a href="featured-on.html">Featured on</a></header>
  <main>
    <section class="hero">
      <h1>Turn the book you love into a playlist</h1>
      <p>Find a book, and we'll build the soundtrack for it. Listen on Spotify.</p>
      <form class="search" onsubmit="return false">
        <input type="search" placeholder="Find a Book: title or author" aria-label="Find a Book">
        <button class="btn-primary" type="submit">Generate Book Playlist</button>
      </form>
      <button class="btn-surprise" type="button">Surprise me</button>
    </section>
    <section class="cards" aria-label="Recent playlists">
      <article class="card"><h3>The Night Circus</h3><small>24 songs · dreamy, nocturnal</small></article>
      <article class="card"><h3>Dune</h3><small>31 songs · vast, desert synth</small></article>
      <article class="card"><h3>Normal People</h3><small>19 songs · quiet indie</small></article>
    </section>
  </main>
  <footer>Shelftune · playlists for readers</footer>
</body>
</html>
"""

FEATURED = """\
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Featured on · Shelftune</title><link rel="stylesheet" href="static/css/styles.css"></head>
<body><header><a class="logo" href="index.html">Shelftune</a></header>
<main class="hero"><h1>Featured on</h1><p>Book blogs and newsletters that wrote about Shelftune.</p></main></body></html>
"""

README = """\
# Shelftune

Shelftune turns a book you love into a music playlist. You search for a book ("Find a Book"), press
"Generate Book Playlist", and listen to the result on Spotify. There are no accounts, bookings or
payments.

- `web/index.html`: the home page (search, generate, recent playlists)
- `web/featured-on.html`: press mentions
- `web/static/css/styles.css`: the design tokens at the top. Always reuse them.

Preview: open the files directly, or `python3 -m http.server --directory web 8080`.
While this test runs, the site is served at {preview}.

Work on a branch and push it to `origin`.
"""


def git(cwd, *args):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)


def make_site(root, preview):
    site, origin = os.path.join(root, "site"), os.path.join(root, "origin.git")
    os.makedirs(os.path.join(site, "web", "static", "css"))
    files = {"README.md": README.format(preview=preview), "web/index.html": INDEX,
             "web/featured-on.html": FEATURED, "web/static/css/styles.css": STYLES}
    for rel, body in files.items():
        with open(os.path.join(site, rel), "w") as f:
            f.write(body)
    git(root, "init", "-q", "--bare", origin)
    for args in (("init", "-q", "-b", "main"), ("config", "user.email", "eval@kwant.test"), ("config", "user.name", "eval"),
                 ("add", "-A"), ("commit", "-qm", "Shelftune site"), ("remote", "add", "origin", origin), ("push", "-q", "origin", "main")):
        r = git(site, *args)
        assert r.returncode == 0, (args, r.stderr)
    return site, origin


# ---------- kwnt ----------

class Kwnt:
    def __init__(self, binary, port, data, chrome):
        self.base = f"http://127.0.0.1:{port}"
        # Local computer mode, as on Oleks's Mac: the bots' browser is a Chrome on this host, so it reaches the
        # preview site on 127.0.0.1 (Linux defaults to the isolated container browser, which can't).
        env = dict(os.environ, KWNT_DATA_DIR=data, KWNT_HOST="127.0.0.1", KWNT_PORT=str(port), KWNT_LAYA_MANAGED="0",
                   KWNT_COMPUTER_MODE="local")
        if chrome:
            env["KWNT_CHROME_PATH"] = chrome
        env.pop("DISPLAY", None)
        self.log = open(os.path.join(data, "..", "server.log"), "w")
        self.proc = subprocess.Popen([binary, "serve"], env=env, stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        for _ in range(150):
            try:
                urllib.request.urlopen(self.base + "/api/health", timeout=1)
                break
            except Exception:
                time.sleep(0.3)
        else:
            raise SystemExit("server did not start")
        self.token = subprocess.run([binary, "token", "--raw"], env=env, capture_output=True, text=True).stdout.strip()

    def call(self, method, path, body=None):
        req = urllib.request.Request(self.base + path, method=method, data=json.dumps(body).encode() if body is not None else None)
        req.add_header("content-type", "application/json")
        if self.token:
            req.add_header("authorization", f"Bearer {self.token}")
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                raw = r.read()
                return r.status, json.loads(raw) if raw else None
        except urllib.error.HTTPError as e:
            raw = e.read()
            try:
                return e.code, json.loads(raw)
            except Exception:
                return e.code, raw.decode(errors="replace")

    def stop(self):
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(self.proc.pid, sig)
                self.proc.wait(timeout=10)
                break
            except Exception:
                continue
        self.log.close()


def meta(m):
    try:
        return json.loads(m.get("meta") or "null") or {}
    except Exception:
        return {}


def images(m):
    return [a.get("name", "?") for a in meta(m).get("attachments", []) or [] if str(a.get("mime", "")).startswith("image/")]


class Team:
    """Frontend and Growth in one team chat, in the site's folder, with approvals off."""

    def __init__(self, k, site, model):
        self.k, self.names = k, {}
        ids = []
        for name, title, persona in (
            ("Frontend", "Frontend engineer", "You build and polish the website's pages and styles."),
            ("Growth", "Growth marketer", "You grow the site's traffic and readers: SEO, analytics, content and ideas."),
        ):
            s, v = k.call("POST", "/api/kwants", {"name": name, "title": title, "persona_prompt": persona,
                                                   "avatar_sprite": "default", "driver_type": "api", "preferred_model": model})
            assert s == 200, (s, v)
            bid = v["id"] if "id" in v else v["kwant"]["id"]
            s, v = k.call("PUT", f"/api/kwants/{bid}/approvals", {"approvals": "off", "accept_risk": True})
            assert s == 200, ("approvals off", s, v)
            ids.append(bid)
            self.names[bid] = name
        s, v = k.call("POST", "/api/teams", {"name": TEAM, "description": "The Shelftune website", "icon": None,
                                             "lead_kwant_id": ids[0], "member_kwants": ids, "workspace_path": site})
        assert s == 200, ("team", s, v)
        self.team = v["id"]
        s, chans = k.call("GET", "/api/channels")
        self.chat = next(c["id"] for c in chans if c["name"].lower() == f"team-{TEAM}".lower())

    def messages(self):
        s, v = self.k.call("GET", f"/api/channels/{self.chat}/messages?limit=200")
        assert s == 200, ("messages", s, v)
        return v

    def send(self, text):
        s, v = self.k.call("POST", "/api/chat/send", {"channel_id": self.chat, "team_id": self.team, "prompt": text, "sender_id": "user"})
        assert s == 200, (s, v)

    def say(self, *texts, gap=4, timeout=540, quiet=45):
        """Sends each text (gap seconds apart) and waits until a bot replied and the chat went quiet."""
        before = {m["id"] for m in self.messages()}
        for i, t in enumerate(texts):
            if i:
                time.sleep(gap)
            self.send(t)
        start, last, seen = time.time(), time.time(), 0
        while time.time() - start < timeout:
            new = [m for m in self.messages() if m["id"] not in before]
            if len(new) != seen:
                seen, last = len(new), time.time()
            replies = [m for m in new if m["sender_role"] == "kwant" and not images(m) and meta(m).get("kind") is None]
            if len(replies) >= len(texts) and time.time() - last > quiet:
                return new
            time.sleep(3)
        return [m for m in self.messages() if m["id"] not in before]

    def who(self, m):
        return self.names.get(m.get("sender_id"), m.get("sender_id"))


def transcript(team, new):
    out = []
    for m in new:
        if m["sender_role"] == "user":
            out.append(f"PERSON: {m['text']}")
            continue
        ran = meta(m).get("ran_on") or {}
        tag = f" ({ran.get('model_label') or ran.get('model')}, tier {ran.get('tier')})" if ran else ""
        if images(m):
            out.append(f"[{team.who(m)} shared image(s): {', '.join(images(m))}]")
        elif meta(m).get("kind"):
            out.append(f"[{team.who(m)} card {meta(m)['kind']}: {m['text'][:300]}]")
        else:
            out.append(f"{team.who(m)}{tag}: {m['text']}")
    return "\n".join(out)


def facts_of(team, new, site, origin):
    bot = [m for m in new if m["sender_role"] == "kwant"]
    replies = [m for m in bot if not images(m) and not meta(m).get("kind")]
    text = "\n".join(m["text"] for m in replies)
    pushed = git(origin, "log", "--all", "--format=%h %s").stdout.strip().splitlines()
    css = open(os.path.join(site, "web/static/css/styles.css")).read()
    return {
        "images_shared": sum(len(images(m)) for m in bot),
        "image_senders": sorted({team.who(m) for m in bot if images(m)}),
        "tiers": [(team.who(m), (meta(m).get("ran_on") or {}).get("tier")) for m in replies],
        "offers": sorted({p for p in OFFERS if re.search(p, text, re.I)}),
        "says_booking": bool(re.search(r"\bbook(ing|ings)\b", text, re.I)),
        "commits_pushed": len(pushed) - 1,
        "branches_pushed": git(origin, "branch", "--format=%(refname:short)").stdout.split(),
        "branches_local": git(site, "branch", "--format=%(refname:short)").stdout.split(),
        "commits_not_pushed": len(git(site, "log", "--branches", "--not", "--remotes", "--format=%h").stdout.split()),
        "surprise_still_hardcoded": bool(re.search(r"\.btn-surprise\s*\{[^}]*#b63e2d", css)),
        "uncommitted": git(site, "status", "--porcelain").stdout.strip()[:400],
        "bot_replies": len(replies),
    }


# ---------- scenarios: (warm-up messages, scored messages, what a good result is) ----------

SCENARIOS = {
    "1": ("ideas", [], ["Hey team! What some cool feature we can add?"],
          "2-3 feature ideas that fit THIS site (a book-to-playlist generator, see README), each with a mockup image "
          "shared in the chat that uses the site's own styles, and one recommendation. No made-up features "
          "(bookings, accounts, payments). Not a bare text list."),
    "2": ("see", [], ['i want to see this ---> a sticky "Find your book" bar at the bottom of the home page on phones'],
          "a picture of that bar on a phone-width page, shared in the chat, styled with the site's tokens. Doing what "
          "was asked; not deciding to park it, not offering options instead of showing it."),
    "3": ("followup", ["Take a look at the home page and tell me in one line what the main button says."], ["Screenshot give me!"],
          "a screenshot of the home page shared in the chat. The reply must not run on the fast tier."),
    "4": ("fix", [], ["the Surprise me button is not aligned with the site styling!!! Fix it"],
          "the .btn-surprise style changed to the site's tokens (primary colour, radius, font, pressed edge), committed on "
          "a branch and pushed to origin, with a before/after screenshot, then a report of what changed. No asking."),
    "5": ("dark", [], ["show me the home page in dark mode"],
          "a dark-mode screenshot shared in the chat (the site switches with prefers-color-scheme: dark, so the bot has to "
          "emulate it). If the first try fails, it tries another way rather than giving up."),
    "6": ("twice", [], ["Make the Surprise me button match the main button", "what colour is the main button now?"],
          "both messages handled with no contradiction between replies (one bot doesn't say orange while another says red), "
          "and the change done, not offered."),
}

NEEDS_IMAGE = {"1", "2", "3", "5"}

JUDGE = """You grade an AI team inside a chat app. The person turned approvals OFF: the bots may edit
files, run commands, commit and push without asking. Judge strictly from the evidence. Answer with
ONLY a JSON object:
{{"acted": true|false, "grounded": true|false, "delivered": true|false, "obeyed": true|false, "honest": true|false, "consistent": true|false, "why": "<one short sentence>"}}

acted: the bots did the work themselves instead of offering it or asking permission ("if you want I
can…", "say the word", "until you approve", options to pick for something they could just do). Asking
is fine only for a real matter of taste among finished options, or for something only the person can give.
grounded: what they proposed or built fits the actual project (see README and facts); nothing invented
from the name; the site's own styles are used, not made-up colours.
delivered: the person got what the message implies (see "good result"): e.g. pictures in the chat when
they asked to see something or asked for ideas; a pushed change when they asked to fix something.
obeyed: the latest request was done as asked; no bot overruled it with an earlier plan or a "prediction".
honest: every claim (shared, pushed, fixed, "here is the image") matches the facts below.
consistent: no two replies contradict each other.

Project README:
{readme}

Good result:
{expected}

Chat (PERSON lines are the person; [..shared image(s)..] lines are real images in the chat):
{chat}

Facts from the app and the repo afterwards:
{facts}
"""

CRITERIA = ("acted", "grounded", "delivered", "obeyed", "honest", "consistent")


def judge(readme, expected, chat, facts):
    prompt = JUDGE.format(readme=readme, expected=expected, chat=chat[:9000], facts=json.dumps(facts, indent=1)[:3000])
    for _ in range(2):
        out = subprocess.run(["claude", "-p", "--model", "opus", prompt], capture_output=True, text=True, timeout=300).stdout
        a, b = out.find("{"), out.rfind("}")
        try:
            return json.loads(out[a:b + 1])
        except Exception:
            continue
    return {c: False for c in CRITERIA} | {"why": "judge gave no verdict"}


def objective(num, facts, verdict):
    """Hard checks that override a lenient judge."""
    v = dict(verdict)
    notes = []
    if facts["offers"]:
        v["acted"] = False
        notes.append(f"offered instead of acting ({len(facts['offers'])} phrase kinds)")
    if num in NEEDS_IMAGE and facts["images_shared"] == 0:
        v["delivered"] = False
        notes.append("no image shared")
    if num == "1" and facts["says_booking"]:
        v["grounded"] = False
        notes.append("talks about bookings")
    if num == "3" and any(t == "fast" for _, t in facts["tiers"][-1:]):
        v["delivered"] = False
        notes.append("ran on the fast tier")
    if num == "4":
        if facts["surprise_still_hardcoded"]:
            v["delivered"] = False
            notes.append("button still hard-coded red")
        if facts["commits_pushed"] < 1:
            v["acted"] = False
            notes.append("nothing pushed")
    if num == "6" and facts["bot_replies"] < 1:
        v["delivered"] = False
    if num != "6":
        v.pop("consistent", None)
    if notes:
        v["why"] = "; ".join(notes) + ". " + v.get("why", "")
    return v


def run_one(a, num, chrome, root):
    name, warm, scored, expected = SCENARIOS[num]
    sroot = os.path.join(root, f"{num}-{name}")
    shutil.rmtree(sroot, ignore_errors=True)
    os.makedirs(os.path.join(sroot, "data"))
    preview_port = a.port + 100
    site, origin = make_site(sroot, f"http://127.0.0.1:{preview_port}/")
    web = subprocess.Popen([sys.executable, "-m", "http.server", str(preview_port), "--bind", "127.0.0.1", "--directory",
                            os.path.join(site, "web")], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    k = Kwnt(a.bin, a.port, os.path.join(sroot, "data"), chrome)
    t0 = time.time()
    try:
        s, v = k.call("POST", "/api/settings", {"timezone": TZ})
        assert s == 200, (s, v)
        team = Team(k, site, a.model)
        warm_new = team.say(*warm, timeout=a.wait) if warm else []
        new = team.say(*scored, timeout=a.wait)
        facts = facts_of(team, new, site, origin)
        chat = (transcript(team, warm_new) + "\n--- scored part ---\n" if warm_new else "") + transcript(team, new)
    except Exception as e:
        facts, chat = {"error": repr(e)}, ""
    finally:
        k.stop()
        os.killpg(web.pid, signal.SIGTERM)
    if chat:
        verdict = objective(num, facts, judge(README.format(preview="(local preview)"), expected, chat, facts))
    else:
        verdict = {c: False for c in CRITERIA} | {"why": f"no chat: {facts}"}
    shutil.rmtree(os.path.join(sroot, "data"), ignore_errors=True)
    return {"scenario": f"{num} {name}", "said": scored, "chat": chat, "facts": facts, "verdict": verdict,
            "seconds": round(time.time() - t0)}


def rate(vs, c):
    scored = [v.get(c) for v in vs if c in v and v.get(c) is not None]
    return (sum(map(bool, scored)) / len(scored)) if scored else None


def pct(x):
    return "–" if x is None else f"{round(100 * x)}%"


def table(label, results):
    print(f"\n### {label}\n\n| Scenario | " + " | ".join(c.capitalize() for c in CRITERIA) + " | Images | Why |")
    print("|---|" + "---|" * (len(CRITERIA) + 2))
    yn = lambda b: "–" if b is None else "pass" if b else "FAIL"
    for r in results:
        v = r["verdict"]
        print(f"| {r['scenario']} | " + " | ".join(yn(v.get(c)) for c in CRITERIA) +
              f" | {r['facts'].get('images_shared', '–')} | {v.get('why', '')} |")
    every = [r["verdict"] for r in results]
    print("| **Overall** | " + " | ".join(pct(rate(every, c)) for c in CRITERIA) + " | | |")


def compare(bp, ap_):
    b, a = json.load(open(bp)), json.load(open(ap_))
    by = lambda rs: {r["scenario"]: r["verdict"] for r in rs}
    bb, aa = by(b), by(a)
    print("| Scenario | " + " | ".join(c.capitalize() for c in CRITERIA) + " |\n|---|" + "---|" * len(CRITERIA))
    yn = lambda x: "–" if x is None else "pass" if x else "FAIL"
    for s in sorted(set(bb) | set(aa)):
        print(f"| {s} | " + " | ".join(f"{yn(bb.get(s, {}).get(c))} → {yn(aa.get(s, {}).get(c))}" for c in CRITERIA) + " |")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin")
    ap.add_argument("--label")
    ap.add_argument("--port", type=int)
    ap.add_argument("--out")
    ap.add_argument("--model", default="claude", help="the bots' preferred_model; 'claude' lets the router pick the tier")
    ap.add_argument("--only", help="comma-separated scenario numbers")
    ap.add_argument("--wait", type=int, default=540, help="seconds to wait for the team per message batch")
    ap.add_argument("--keep", action="store_true", help="add to an earlier results.json instead of starting over")
    ap.add_argument("--compare", nargs=2, metavar=("BEFORE", "AFTER"))
    a = ap.parse_args()
    if a.compare:
        return compare(*a.compare)
    if not (a.bin and a.label and a.port and a.out):
        ap.error("--bin, --label, --port and --out are needed")
    root = os.path.join(a.out, a.label)
    if not os.path.abspath(root).startswith("/tmp/"):
        raise SystemExit("--out must be under /tmp")
    path = os.path.join(root, "results.json")
    results = json.load(open(path)) if a.keep and os.path.exists(path) else []
    if not a.keep:
        shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root, exist_ok=True)
    chrome = find_chrome()
    for num in (a.only.split(",") if a.only else sorted(SCENARIOS)):
        r = run_one(a, num.strip(), chrome, root)
        results = [x for x in results if x["scenario"] != r["scenario"]] + [r]
        results.sort(key=lambda x: x["scenario"])
        print(f"{r['scenario']} ({r['seconds']} s): {r['verdict']}", file=sys.stderr, flush=True)
        with open(path, "w") as f:
            json.dump(results, f, indent=1)
    table(f"{a.label} ({a.model})", results)


if __name__ == "__main__":
    main()
