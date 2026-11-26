// Living avatars runner (dev only; ui_budget "life" case). Serves perf/stream/dist, opens life.html in headless
// Chromium with motion allowed, lets 30 avatars live for --seconds (default 20) and prints:
//   lifeCommits     React commits while they live (frames are DOM attributes, so this must be 0)
//   lifeTimers      the most timers the shared clock held at once (sampled every 50 ms; must be <= 1)
//   lifeP95FrameMs  the 95th percentile gap between animation frames, in ms
//   lifeEvents      how many frame changes the avatars showed (> 0, or life never ran)
//   lifeMaxMoving   the most avatars mid-frame at once (the clock caps it at 3; moments aside)
// Usage: node perf/stream/life.mjs [--seconds 20]. Needs PLAYWRIGHT_CORE and CHROMIUM, like run.mjs.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const arg = (k, d) => {
  const i = process.argv.indexOf(`--${k}`);
  return i > 0 ? process.argv[i + 1] : d;
};
const SECONDS = Number(arg('seconds', 20));
const SHOT = arg('shot', '');
const { chromium } = await import(process.env.PLAYWRIGHT_CORE ?? '/home/maus/work/iris-ft-shots/node_modules/playwright-core/index.mjs');
const exe = process.env.CHROMIUM ?? '/home/maus/.cache/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-linux-arm64/chrome-headless-shell';

const dist = path.join(path.dirname(fileURLToPath(import.meta.url)), 'dist');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2', '.svg': 'image/svg+xml' };
const server = http.createServer((req, res) => {
  const p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  const file = path.join(dist, p);
  if (!file.startsWith(dist) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) return res.writeHead(404).end();
  res.writeHead(200, { 'Content-Type': types[path.extname(file)] ?? 'application/octet-stream' }).end(fs.readFileSync(file));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

const browser = await chromium.launch({ executablePath: exe });
const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: 'dark', reducedMotion: 'no-preference' });
const page = await ctx.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(String(e)));
await page.addInitScript(() => {
  const m = { commits: 0, on: false, frames: [], last: 0, events: 0, maxTimers: 0, maxMoving: 0 };
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
  const loop = (t) => {
    if (m.on && m.last) m.frames.push(t - m.last);
    m.last = t;
    requestAnimationFrame(loop);
  };
  requestAnimationFrame(loop);
});
await page.goto(`${base}/life.html`);
await page.waitForFunction(() => window.__life?.ready(), null, { timeout: 60000 });
await page.waitForTimeout(1000);
await page.evaluate(() => {
  const m = window.__m;
  Object.assign(m, { on: true, commits: 0, frames: [], last: 0, events: 0 });
  new MutationObserver((rs) => (m.events += rs.length)).observe(document.body, { subtree: true, attributes: true, attributeFilter: ['data-life', 'data-hop'] });
  setInterval(() => {
    const s = window.__life.stats();
    m.maxTimers = Math.max(m.maxTimers, s.timers);
    m.maxMoving = Math.max(m.maxMoving, document.querySelectorAll('[data-life]').length);
  }, 50);
});
// Hover a few sidebar rows on the way, like a person reading the list.
for (let s = 0; s < SECONDS; s += 5) {
  await page.mouse.move(100, 30 + ((s * 37) % 600));
  await page.waitForTimeout(Math.min(5, SECONDS - s) * 1000);
}
if (SHOT) await page.screenshot({ path: SHOT });
const out = await page.evaluate(() => {
  const m = window.__m;
  m.on = false;
  const f = m.frames.slice().sort((a, b) => a - b);
  const p95 = f[Math.min(f.length - 1, Math.floor(f.length * 0.95))] ?? 0;
  return { commits: m.commits, p95, frames: f.length, events: m.events, maxTimers: m.maxTimers, maxMoving: m.maxMoving, registered: window.__life.stats().registered };
});
console.log(
  JSON.stringify({
    seconds: SECONDS,
    lifeCommits: out.commits,
    lifeTimers: out.maxTimers,
    lifeP95FrameMs: Math.round(out.p95 * 10) / 10,
    lifeEvents: out.events,
    lifeMaxMoving: out.maxMoving,
    registered: out.registered,
    frames: out.frames,
    errors,
  }),
);
await browser.close();
server.close();
