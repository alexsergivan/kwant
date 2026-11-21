// Typing harness runner (dev only; perf/stream/README.md, 33j): opens the harness chat of --n messages, focuses
// the composer and types --text one key every --gap ms, the way a person types. For each key it times keydown
// until after the next frame (the time the letter takes to show), and counts React commits.
// Usage: node perf/stream/typing.mjs --n 100 --results 10 [--width 1440|390] [--motion reduce|no-preference] [--label after]
import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const arg = (k, d) => {
  const i = process.argv.indexOf(`--${k}`);
  return i > 0 ? process.argv[i + 1] : d;
};
const N = Number(arg('n', 100));
const RESULTS = Number(arg('results', 10));
const WIDTH = Number(arg('width', 1440));
const GAP = Number(arg('gap', 60));
const MOTION = arg('motion', 'reduce');
const LABEL = arg('label', '');
const TEXT = arg('text', 'Can you compare March totals with last year and chart it?'); // 57 keys, as in Mira's run
const { chromium } = await import(process.env.PLAYWRIGHT_CORE ?? '/home/maus/work/iris-pw/node_modules/playwright-core/index.mjs');
const exe = process.env.CHROMIUM ?? '/home/maus/.cache/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-linux-arm64/chrome-headless-shell';

const dist = path.resolve(arg('dist', path.join(path.dirname(fileURLToPath(import.meta.url)), 'dist')));
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2', '.svg': 'image/svg+xml', '.png': 'image/png' };
const server = http.createServer((req, res) => {
  const p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  const file = path.join(dist, p === '/' ? 'index.html' : p);
  if (!file.startsWith(dist) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) return res.writeHead(404).end();
  res.writeHead(200, { 'Content-Type': types[path.extname(file)] ?? 'application/octet-stream' }).end(fs.readFileSync(file));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

const browser = await chromium.launch({ executablePath: exe });
const phone = WIDTH < 500;
const ctx = await browser.newContext({ viewport: { width: WIDTH, height: phone ? 844 : 900 }, deviceScaleFactor: phone ? 2 : 1, isMobile: phone, hasTouch: phone, colorScheme: 'dark', reducedMotion: MOTION });
const page = await ctx.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(String(e)));
await page.addInitScript(() => {
  const m = { commits: 0, on: false, keys: [] };
  window.__m = m;
  window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
    supportsFiber: true,
    renderers: new Map(),
    inject: () => 1,
    onCommitFiberRoot: () => m.on && m.commits++,
    onCommitFiberUnmount: () => {},
    onPostCommitFiberRoot: () => {},
    checkDCE: () => {},
  };
  // keydown until after the next frame: the letter is painted by then
  addEventListener(
    'keydown',
    (e) => {
      if (!m.on) return;
      const t0 = e.timeStamp;
      requestAnimationFrame(() => setTimeout(() => m.keys.push(performance.now() - t0), 0));
    },
    true,
  );
});
await page.goto(`${base}/?n=${N}&results=${RESULTS}`);
await page.waitForFunction(() => window.__harness?.ready(), null, { timeout: 120000 });
await page.waitForTimeout(2000);
const shape = await page.evaluate(() => ({
  rows: document.querySelectorAll('[data-message-id]').length,
  smart: document.querySelectorAll('[data-message-id] svg, [data-message-id] table').length,
  nodes: document.getElementsByTagName('*').length,
}));
const ta = page.locator('textarea[data-composer]');
await ta.click();
await page.waitForTimeout(500);
await page.evaluate(() => Object.assign(window.__m, { on: true, commits: 0, keys: [] }));
await ta.pressSequentially(TEXT, { delay: GAP });
await page.waitForTimeout(500);
const out = await page.evaluate(() => {
  const m = window.__m;
  m.on = false;
  return { keys: m.keys, commits: m.commits, value: document.querySelector('textarea[data-composer]').value };
});
const k = out.keys.slice().sort((a, b) => a - b);
const pct = (p) => k[Math.min(k.length - 1, Math.floor((k.length * p) / 100))] ?? 0;
console.log(
  JSON.stringify({
    label: LABEL,
    messages: N,
    width: WIDTH,
    motion: MOTION,
    ...shape,
    keys: k.length,
    p50: +pct(50).toFixed(1),
    p95: +pct(95).toFixed(1),
    max: +(k[k.length - 1] ?? 0).toFixed(1),
    commits: out.commits,
    typedAll: out.value === TEXT,
    load: os.loadavg().map((x) => +x.toFixed(2)),
    errors,
  }),
);
await browser.close();
server.close();
