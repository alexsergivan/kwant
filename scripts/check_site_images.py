#!/usr/bin/env python3
"""Checks that no <img> on the built site renders stretched or squashed.

Serves site/ on a local port, loads every page in headless Chromium at desktop,
tablet and phone widths, and compares each image's rendered aspect ratio with its
real one. Images that crop on purpose (object-fit other than fill) are skipped.

    python3 scripts/check_site_images.py            # uses chromium / google-chrome on PATH
    CHROME=/path/to/chrome python3 scripts/check_site_images.py

Exits 1 and lists every distorted image if any are found.
"""
import functools
import http.server
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"
WIDTHS = [1440, 1280, 900, 390]
TOLERANCE = 0.02  # 2 % difference between rendered and real aspect ratio

HARNESS = """<!doctype html><meta charset="utf-8"><body><script>
const pages = %(pages)s, widths = %(widths)s, results = [];
async function measure(page, width) {
  const f = document.createElement('iframe');
  f.style.cssText = `width:${width}px;height:900px;border:0`;
  document.body.append(f);
  await new Promise(r => { f.onload = r; f.src = page; });
  const d = f.contentDocument;
  const imgs = [...d.querySelectorAll('img')];
  imgs.forEach(i => { i.loading = 'eager'; });
  await Promise.all(imgs.map(i => i.decode().catch(() => {})));
  for (const i of imgs) {
    const r = i.getBoundingClientRect(), fit = f.contentWindow.getComputedStyle(i).objectFit;
    if (!r.width || !r.height || !i.naturalWidth || fit !== 'fill') continue;
    const shown = r.height / r.width, real = i.naturalHeight / i.naturalWidth;
    if (Math.abs(shown / real - 1) > %(tol)s)
      results.push({page, width, src: i.getAttribute('src'), rendered: [Math.round(r.width), Math.round(r.height)],
                    natural: [i.naturalWidth, i.naturalHeight], stretch: +(shown / real).toFixed(2)});
  }
  f.remove();
}
(async () => {
  let n = 0;
  for (const p of pages) for (const w of widths) { await measure(p, w); n++; }
  await fetch('/__result', {method: 'POST', body: JSON.stringify({checked: n, bad: results})});
})();
</script>"""


def find_chrome():
    for c in [os.environ.get("CHROME"), "chromium", "chromium-browser", "google-chrome", "google-chrome-stable"]:
        if c and shutil.which(c):
            return shutil.which(c)
    sys.exit("check_site_images: no Chromium found; set CHROME=/path/to/chrome")


def main():
    pages = sorted(str(p.relative_to(SITE)) for p in SITE.rglob("*.html"))
    harness = HARNESS % {"pages": json.dumps(pages), "widths": json.dumps(WIDTHS), "tol": TOLERANCE}

    result, done = {}, threading.Event()

    class Handler(http.server.SimpleHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/__check_images.html":
                body = harness.encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                super().do_GET()

        def do_POST(self):
            result.update(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
            self.send_response(204)
            self.end_headers()
            done.set()

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=str(SITE)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{server.server_address[1]}/__check_images.html"
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as profile:
        browser = subprocess.Popen(
            [find_chrome(), "--headless", "--disable-gpu", "--no-sandbox", f"--user-data-dir={profile}", url],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        finished = done.wait(300)
        browser.terminate()
        browser.wait()
    server.shutdown()
    if not finished:
        sys.exit("check_site_images: the browser didn't finish measuring within 5 minutes")
    data = result
    for b in data["bad"]:
        print(f"{b['page']} @ {b['width']}px: {b['src']} shows {b['rendered'][0]}x{b['rendered'][1]}, "
              f"real ratio {b['natural'][0]}:{b['natural'][1]} (height stretched x{b['stretch']})")
    print(f"check_site_images: {len(pages)} pages x {len(WIDTHS)} widths, {len(data['bad'])} distorted images")
    return 1 if data["bad"] else 0


if __name__ == "__main__":
    sys.exit(main())
